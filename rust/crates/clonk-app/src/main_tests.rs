    use super::*;
    use flate2::read::ZlibDecoder;
    use clonk_audio::decode_audio;
    use clonk_engine::command::CommandData;
    use clonk_engine::{
        ActionState, CommandDirection, CommandStackSnapshot, DEFAULT_CATEGORY, Direction,
        EnvironmentFrame, FloatVector2, HudPlayerSnapshot, HudSnapshot, ObjectId, ObjectSnapshot,
        ObjectStatus, PlayerState, PlayerStatus, ScriptError, SimulationSnapshot, Vector2,
    };
    use clonk_graphics::clonk_font::{ClonkFont, ClonkFontRole, GlyphCell};
    use clonk_script::Value;
    use parking_lot::ReentrantMutex;
    use std::collections::HashMap;
    use std::env;
    use std::ffi::OsString;
    use std::fs;
    use std::io::{BufWriter, Read, Write};
    use std::path::Path;
    use std::sync::OnceLock;
    use std::thread;
    use std::time::Duration;

    fn tempdir() -> std::io::Result<tempfile::TempDir> {
        tempfile::Builder::new().prefix("lc-test-").tempdir()
    }

    fn install_record_test_definitions(root: &Path) {
        let definition = root.join("Objects.c4d").join("Record.c4d");
        fs::create_dir_all(&definition).expect("create record fixture definition");
        fs::write(
            definition.join("DefCore.txt"),
            b"[DefCore]\nid=RECD\nName=Record fixture\nCategory=1\n",
        )
        .expect("write record fixture definition core");
        write_test_definition_graphics(&definition);
    }

    fn write_test_definition_graphics(path: &Path) {
        image::RgbaImage::from_pixel(1, 1, image::Rgba([1, 2, 3, 255]))
            .save(path.join("Graphics.png"))
            .expect("write definition graphics");
    }

    fn render_mouse_test_app(app: &mut GameApp) {
        app.snapshot = app.engine.snapshot();
        let mut frame = vec![0_u8; 320 * 200 * 4];
        app.render(&mut frame).expect("render mouse test fixture");
    }

    fn mouse_test_object_point(app: &GameApp, owner: i32, object: ObjectId) -> GuiPoint {
        let viewport = app
            .graphics
            .viewport_rect(owner)
            .expect("mouse test has a local viewport");
        (viewport.y..viewport.y + viewport.height as i32)
            .flat_map(|y| {
                (viewport.x..viewport.x + viewport.width as i32)
                    .map(move |x| GuiPoint::new(x as f32 + 0.5, y as f32 + 0.5))
            })
            .find(|point| app.graphics.object_at_point(&app.snapshot, owner, *point) == Some(object))
            .expect("mouse test object has a visible pick point")
    }

    fn mouse_test_empty_point(
        app: &mut GameApp,
        owner: i32,
        start: GuiPoint,
        carry_command: Option<CommandId>,
    ) -> (GuiPoint, Vector2) {
        let viewport = app
            .graphics
            .viewport_rect(owner)
            .expect("mouse test has a local viewport");
        (viewport.y..viewport.y + viewport.height as i32)
            .flat_map(|y| {
                (viewport.x..viewport.x + viewport.width as i32)
                    .map(move |x| GuiPoint::new(x as f32 + 0.5, y as f32 + 0.5))
            })
            .find_map(|point| {
                let routed_point = GuiPoint::new(point.x.ceil(), point.y.ceil());
                let pointer = app.graphics.viewport_point_at(routed_point)?;
                let world = ingame_pointer_world_pixel(pointer);
                (pointer.owner == owner
                    && ((point.x - start.x).abs() >= 12.0
                        || (point.y - start.y).abs() >= 12.0)
                    && app
                        .ingame_viewport_region(owner, routed_point)
                        .is_none()
                    && app
                        .graphics
                        .object_at_point(&app.snapshot, owner, routed_point)
                        .is_none()
                    && carry_command.is_none_or(|expected| {
                        app.engine.mouse_drag_carryable_command(owner, world) == Some(expected)
                    }))
                .then_some((point, world))
            })
            .expect("mouse test has a matching empty release point")
    }

    fn physical_left_click_with_modifiers(
        app: &mut GameApp,
        point: GuiPoint,
        press_modifiers: ModifiersState,
        release_modifiers: ModifiersState,
    ) {
        // Distinct waypoint clicks are not a platform LeftDouble gesture.
        app.ingame_last_left_down = None;
        app.handle_modifiers_changed(press_modifiers)
            .expect("install left-down modifiers");
        app.handle_cursor_moved(PhysicalPosition::new(
            f64::from(point.x),
            f64::from(point.y),
        ))
        .expect("move to click point");
        app.handle_mouse_button(ElementState::Pressed)
            .expect("physical left-down");
        app.handle_modifiers_changed(release_modifiers)
            .expect("install left-up modifiers");
        app.handle_mouse_button(ElementState::Released)
            .expect("physical left-up");
    }

    fn install_mouse_network_capture(app: &mut GameApp) -> network::TestNetworkCommands {
        let (manager, _events, commands) = NetworkManager::test_stub_with_commands();
        app.network = Some(manager);
        commands
    }

    fn install_mouse_help_target(
        app: &mut GameApp,
        definition_id: &str,
        custom_name: &str,
        description: Option<&str>,
    ) -> (ObjectId, GuiPoint) {
        render_mouse_test_app(app);
        let owner = app.local_owner;
        let viewport = app.graphics.viewport_rect(owner).expect("sandbox viewport");
        let inset_x = 24_i32.min(viewport.width as i32 / 4);
        let inset_y = 24_i32.min(viewport.height as i32 / 4);
        let position = (viewport.y + inset_y..viewport.y + viewport.height as i32 - inset_y)
            .flat_map(|y| {
                (viewport.x + inset_x..viewport.x + viewport.width as i32 - inset_x)
                    .map(move |x| GuiPoint::new(x as f32 + 0.5, y as f32 + 0.5))
            })
            .find_map(|point| {
                let routed = GuiPoint::new(point.x.ceil(), point.y.ceil());
                let pointer = app.graphics.viewport_point_at(routed)?;
                (pointer.owner == owner
                    && app.ingame_viewport_region(owner, routed).is_none()
                    && app
                        .graphics
                        .object_at_point(&app.snapshot, owner, routed)
                        .is_none())
                .then_some(ingame_pointer_world_pixel(pointer))
            })
            .expect("help target has an empty interior spawn point");
        let mut definition =
            Definition::from_script(definition_id, "Definition name", "#strict\n")
                .expect("help target compiles");
        definition.set_category(clonk_engine::CATEGORY_OBJECT);
        definition.set_shape_rect(Some(clonk_engine::DefinitionRect::new(-4, -4, 8, 8)));
        definition.set_description(description.map(str::to_string));
        app.engine
            .register_definition(definition)
            .expect("register help target");
        let mut spawn = SpawnConfig::new(definition_id)
            .with_position(position)
            .with_custom_name(custom_name);
        if let Some(layer) = app
            .engine
            .crew_cursor(owner)
            .and_then(|cursor| app.engine.object_snapshot(cursor))
            .and_then(|cursor| cursor.layer)
        {
            spawn = spawn.with_layer(layer);
        }
        let target = app
            .engine
            .spawn_object(spawn)
            .expect("spawn help target");
        render_mouse_test_app(app);
        let point = mouse_test_object_point(app, owner, target);
        assert_eq!(
            app.ingame_primary_mouse_target(owner, point),
            None,
            "the normal interaction OCF mask excludes the help-only target"
        );
        assert_eq!(
            app.ingame_help_mouse_target(owner, point),
            Some(target),
            "Help widens the target mask to OCF_All"
        );
        (target, point)
    }

    fn inventory_region_fixture() -> (GameApp, i32, ObjectId, ObjectId, ObjectId, GuiPoint) {
        let mut app = new_running_sandbox_app();
        let owner = app.local_owner;
        let crew = app
            .engine
            .crew_cursor(owner)
            .expect("sandbox has a cursor crew member");
        let mut landscape = Landscape::flat(480, 180);
        landscape.set_world_height(200);
        app.engine.set_landscape(landscape);
        let mut item =
            Definition::from_script("MITM", "Mouse item", "#strict\n").expect("item compiles");
        item.set_category(clonk_engine::CATEGORY_OBJECT);
        item.set_collectible(true);
        app.engine.register_definition(item).expect("register item");
        let first = app
            .engine
            .spawn_object(SpawnConfig::new("MITM").with_container(crew))
            .expect("spawn first item");
        let second = app
            .engine
            .spawn_object(SpawnConfig::new("MITM").with_container(crew))
            .expect("spawn second item");

        app.snapshot = app.engine.snapshot();
        let mut frame = vec![0_u8; 320 * 200 * 4];
        app.render(&mut frame).expect("render inventory region");
        let viewport = app
            .graphics
            .viewport_rect(owner)
            .expect("local sandbox viewport");
        let region_point = GuiPoint::new(
            (viewport.x + clonk_frontend::hud::SYMBOL_BORDER + clonk_frontend::hud::SYMBOL_SIZE / 2)
                as f32,
            (viewport.y + viewport.height as i32
                - clonk_frontend::hud::SYMBOL_BORDER
                - clonk_frontend::hud::SYMBOL_SIZE / 2) as f32,
        );
        let region_target = app
            .ingame_inventory_region_target(owner, region_point)
            .expect("first inventory cell has a copied target");
        assert_eq!(
            region_target, second,
            "runtime same-ID cluster is newest-first"
        );
        (app, owner, crew, first, second, region_point)
    }

    fn command_region_point(app: &GameApp, command: u8) -> GuiPoint {
        let owner = app.local_owner;
        let cursor = app
            .snapshot
            .players
            .iter()
            .find(|player| player.id == owner)
            .and_then(|player| player.cursor)
            .expect("local cursor");
        let viewport = app
            .graphics
            .viewport_rect(owner)
            .expect("local viewport");
        let context = AppCommandContext {
            engine: &app.engine,
            bindings: &app.bindings,
            snapshot: &app.snapshot,
            resources: &app.startup_tooltip_resources,
        };
        let icons = draw_commands::build_cursor_commands(&app.snapshot, cursor, &context);
        let wanted = icons
            .iter()
            .position(|icon| icon.com == command)
            .unwrap_or_else(|| panic!("command bar has no COM {command}"));
        for y in viewport.y..viewport.y + viewport.height as i32 {
            for x in viewport.x..viewport.x + viewport.width as i32 {
                let point = GuiPoint::new(x as f32 + 0.5, y as f32 + 0.5);
                if clonk_frontend::hud::command_region_index(viewport, point, &icons) == Some(wanted) {
                    return point;
                }
            }
        }
        panic!("no visible region for COM {command}");
    }

    fn viewport_button_point(
        app: &GameApp,
        owner: i32,
        button: clonk_frontend::hud::ViewportButton,
    ) -> GuiPoint {
        let viewport = app
            .graphics
            .viewport_rect(owner)
            .expect("viewport button owner has a viewport");
        let rect = clonk_frontend::hud::viewport_button_rect(viewport, button);
        GuiPoint::new(
            rect.x as f32 + rect.width as f32 / 2.0,
            rect.y as f32 + rect.height as f32 / 2.0,
        )
    }

    fn command_bar_fixture(control_style: bool) -> (GameApp, i32, [(u8, GuiPoint); 4]) {
        let mut app = new_running_sandbox_app();
        let owner = app.local_owner;
        app.engine
            .player_mut(owner)
            .expect("local player")
            .control
            .control_style = control_style;
        let cursor = app.engine.crew_cursor(owner).expect("sandbox cursor");
        let container = Definition::from_script("MBAS", "Mouse base", "#strict\n")
            .expect("base definition compiles");
        app.engine
            .register_definition(container)
            .expect("register base");
        let container = app
            .engine
            .spawn_object(SpawnConfig::new("MBAS"))
            .expect("spawn base");
        app.engine
            .apply_object_update(container, ObjectUpdate::new().with_base(owner))
            .expect("mark player base");
        app.engine
            .apply_object_update(cursor, ObjectUpdate::new().with_container(container))
            .expect("contain cursor");
        let mut item = Definition::from_script("MCBI", "Command item", "#strict\n")
            .expect("item definition compiles");
        item.set_category(clonk_engine::CATEGORY_OBJECT);
        item.set_collectible(true);
        app.engine.register_definition(item).expect("register item");
        app.engine
            .spawn_object(SpawnConfig::new("MCBI").with_container(cursor))
            .expect("give cursor an item");
        app.engine.set_base_buy_enabled(true);
        app.engine.set_base_sell_enabled(true);
        app.snapshot = app.engine.snapshot();
        let mut frame = vec![0_u8; 320 * 200 * 4];
        app.render(&mut frame).expect("render command regions");

        let points = [5_u8, 4, 3, 6].map(|command| {
            let point = command_region_point(&app, command);
            assert_eq!(app.ingame_command_region_at(owner, point), Some(command));
            (command, point)
        });
        (app, owner, points)
    }

    /// Headless physical-key driver for app integration tests.
    ///
    /// This deliberately enters at the same `GameApp::handle_key` boundary
    /// as `WindowEvent::KeyboardInput` (main.rs:1691-1705). Going through
    /// `dispatch_control_event_for_owner`, `InputDispatcher`, or
    /// `Engine::player_in_com` would skip the configured keyboard mapping and
    /// C4Game::LocalControlKeyUp's classic/AutoStopControl release gate.
    struct AppVirtualKeyboard<'app> {
        app: &'app mut GameApp,
    }

    impl<'app> AppVirtualKeyboard<'app> {
        fn new(app: &'app mut GameApp) -> Self {
            Self { app }
        }

        fn press(&mut self, key: VirtualKeyCode) -> Result<(), EngineError> {
            self.app.handle_key(key, ElementState::Pressed)
        }

        fn release(&mut self, key: VirtualKeyCode) -> Result<(), EngineError> {
            self.app.handle_key(key, ElementState::Released)
        }

        fn tap(&mut self, key: VirtualKeyCode) -> Result<(), EngineError> {
            self.press(key)?;
            self.release(key)
        }

        fn engine(&self) -> &Engine {
            &self.app.engine
        }

        fn player_control(&self) -> clonk_engine::PlayerControlState {
            self.app
                .engine
                .snapshot()
                .players
                .into_iter()
                .find(|player| player.id == self.app.local_owner)
                .expect("virtual keyboard has a local player")
                .control
        }
    }

    struct RealTutorialApp {
        app: GameApp,
        _env_guard: EnvGuard,
        _user_data: tempfile::TempDir,
    }

    impl std::ops::Deref for RealTutorialApp {
        type Target = GameApp;

        fn deref(&self) -> &Self::Target {
            &self.app
        }
    }

    impl std::ops::DerefMut for RealTutorialApp {
        fn deref_mut(&mut self) -> &mut Self::Target {
            &mut self.app
        }
    }

    impl Drop for RealTutorialApp {
        fn drop(&mut self) {
            reset_cached_app_paths();
        }
    }

    fn real_installed_scenario_app(scenario_key: &str, player_name: &str) -> RealTutorialApp {
        real_installed_scenario_app_with_roster(scenario_key, player_name, false)
    }

    fn load_frontend_scenarios_for_test(
        paths: &AppPaths,
        scenario_key: &str,
    ) -> (FrontendScenario, Vec<FrontendScenario>) {
        fn restore_production_identifiers(entry: &mut FrontendScenario, parent: &Path) {
            entry.identifier = parent
                .join(&entry.identifier)
                .to_string_lossy()
                .replace('\\', "/");
            for child in &mut entry.children {
                restore_production_identifiers(child, parent);
            }
        }

        let relative = Path::new(scenario_key);
        let parent = relative.parent().unwrap_or_else(|| Path::new(""));
        let languages = startup_language_sequence(Some(paths));
        let language_packs = classic_language_packs(paths);
        for root in scenario_roots(paths) {
            let discovery_root = root.path.join(parent);
            if !discovery_root.exists() {
                continue;
            }
            let entries = resource_scenario::discover_with_languages_and_packs(
                &discovery_root,
                &languages,
                &language_packs,
            )
            .unwrap_or_else(|error| {
                panic!(
                    "discover focused real scenario {}: {error}",
                    discovery_root.display()
                )
            });
            let mut scenarios = entries
                .into_iter()
                .map(|entry| FrontendScenario::from_resource(entry, &root.label))
                .collect::<Vec<_>>();
            // Focused discovery starts at the requested scenario's immediate
            // parent. Restore install-root identifiers for that whole sibling
            // set so NextMission/restart navigation remains production-faithful
            // without scanning every scenario pack for every route test.
            for scenario in &mut scenarios {
                restore_production_identifiers(scenario, parent);
            }
            let catalog = build_scenario_catalog(&scenarios);
            let Some(scenario) = resolve_next_mission_scenario(&catalog, scenario_key) else {
                continue;
            };
            return (scenario, scenarios);
        }
        panic!("{scenario_key} is present in the real scenario catalog");
    }

    struct PreparedRealInstalledScenario {
        scenario_key: String,
        scenario: FrontendScenario,
        scenarios: Vec<FrontendScenario>,
        scenario_data: Scenario,
    }

    impl PreparedRealInstalledScenario {
        fn new(scenario_key: &str) -> Self {
            let repository = Path::new(env!("CARGO_MANIFEST_DIR"))
                .parent()
                .and_then(Path::parent)
                .and_then(Path::parent)
                .expect("repository root");
            let user_data = tempdir().expect("isolated prepared-scenario user data");
            let _env_guard = EnvGuard::set(&[
                ("LC_INSTALL_ROOT", Some(repository)),
                ("LC_USER_DATA_DIR", Some(user_data.path())),
                ("LC_LANGUAGE", Some(Path::new("US"))),
            ]);
            let paths = AppPaths::discover().expect("discover repository install");
            let (scenario, scenarios) =
                load_frontend_scenarios_for_test(&paths, scenario_key);
            let scenario_path = scenario
                .path
                .clone()
                .unwrap_or_else(|| panic!("{scenario_key} path"));
            let scenario_data = Scenario::load_from_path_with_languages(
                &scenario_path,
                &InstallDefinitionResolver::new(Some(Arc::new(paths.clone()))),
                &startup_language_sequence(Some(&paths)),
            )
            .unwrap_or_else(|error| panic!("load real {scenario_key}: {error}"));

            Self {
                scenario_key: scenario_key.to_string(),
                scenario,
                scenarios,
                scenario_data,
            }
        }

        fn instantiate(
            &self,
            player_name: &str,
            preexisting_clonk: bool,
        ) -> RealTutorialApp {
            let scenario_key = &self.scenario_key;
            let repository = Path::new(env!("CARGO_MANIFEST_DIR"))
                .parent()
                .and_then(Path::parent)
                .and_then(Path::parent)
                .expect("repository root");
            let user_data = tempdir().expect("isolated user data");
            let env_guard = EnvGuard::set(&[
                ("LC_INSTALL_ROOT", Some(repository)),
                ("LC_USER_DATA_DIR", Some(user_data.path())),
                // The virtual-player milestones intentionally assert the shipped
                // US tutorial copy. Isolate them from the developer machine's
                // locale just like the user-data/config roots.
                ("LC_LANGUAGE", Some(Path::new("US"))),
            ]);
            let paths = AppPaths::discover().expect("discover repository install");
            paths.ensure_user_dirs().expect("prepared user dirs");
            // The production launcher adapts an empty config before clonk-app.
            fs::write(
                paths.config_file(),
                b"[General]\nVersion=362\n[Graphics]\nShader=true\nDisableGamma=false\n",
            )
            .expect("materialize post-migration renderer config");
            let audio_options = AudioOptions {
                sound_enabled: false,
                music_enabled: false,
                menu_music_enabled: false,
                menu_sound_enabled: false,
                ..AudioOptions::default()
            };
            let mut app = GameApp::new_with_frontend_scenarios(
                320,
                200,
                audio_options,
                Some(&paths),
                RuntimeConfig {
                    player_owner: 1,
                    player_name: player_name.to_string(),
                    network: None,
                    record_enabled: false,
                },
                Some(self.scenarios.clone()),
            )
            .expect("initialise app");
            if preexisting_clonk {
                // These long physical-route fixtures exercise the ordinary C++
                // persistent-player path: GetIdle recruits the existing CLNK and
                // therefore does not create/name a new crew info. Fresh-crew
                // System-name RNG is pinned separately by Tutorial09 below.
                app.selected_player_file = Some(PlayerFile {
                    info_core: Default::default(),
                    name: player_name.to_string(),
                    score: 0,
                    rounds: 0,
                    rounds_won: 0,
                    rounds_lost: 0,
                    total_playing_time: 0,
                    pref_color: 0,
                    pref_color_dw: 0xff,
                    pref_color2_dw: 0,
                    pref_position: 0,
                    pref_control: 0,
                    pref_mouse: true,
                    pref_control_style: true,
                    pref_auto_context_menu: true,
                    crew: vec![clonk_engine::player_file::CrewInfo {
                        core: Default::default(),
                        id: "CLNK".to_string(),
                        name: "Clonk".to_string(),
                        death_message: String::new(),
                        rank: 0,
                        rank_name: "Clonk".to_string(),
                        experience: 0,
                        rounds: 0,
                        physical: clonk_engine::PhysicalInfo::default(),
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
                });
                // Their movement timings predate live Game.Parameters and were
                // recorded against the former engine-wide fair-crew default.
                // Pin that round option explicitly; normal-crew activation and
                // player-file physicals have dedicated L061 regressions.
                let mut options = app.scenario_game_options.values().clone();
                options.fair_crew = true;
                options.fair_crew_strength = 1_000;
                app.scenario_game_options =
                    GameOptionButtons::new(GameOptionContext::LocalSelector, options);
            }
            wait_for_menu(&mut app);

            app.activate_loaded_scenario(self.scenario.clone(), &self.scenario_data)
                .unwrap_or_else(|error| panic!("activate real {scenario_key}: {error}"));
            // Physical-route scenario tests start after the native event loop has
            // already delivered C4MouseControl's one-time centered move.
            app.ingame_mouse_init_centered = true;

            RealTutorialApp {
                app,
                _env_guard: env_guard,
                _user_data: user_data,
            }
        }
    }

    fn real_installed_scenario_app_with_roster(
        scenario_key: &str,
        player_name: &str,
        preexisting_clonk: bool,
    ) -> RealTutorialApp {
        PreparedRealInstalledScenario::new(scenario_key)
            .instantiate(player_name, preexisting_clonk)
    }

    /// Pump the app until asynchronous boot loading completes and the main menu
    /// is shown. A freshly constructed `GameApp` starts in `AppMode::Loading`
    /// while the boot/material-library thread runs; it transitions to
    /// `AppMode::Menu` only after `update()` polls the boot completion. Panics if
    /// it never settles, so a genuinely stuck boot still fails the test.
    fn wait_for_menu(app: &mut GameApp) {
        wait_for_menu_impl(app, true);
    }

    fn wait_for_menu_impl(app: &mut GameApp, dismiss_first_player_dialog: bool) {
        // Asset-less unit fixtures intentionally test isolated menu logic.
        // Production stays in Loading and reports the typed loader boundary;
        // only this test helper bypasses startup presentation explicitly.
        if app.app_paths.is_none() && app.loader_error.is_some() {
            app.boot_loading = None;
            app.mode = AppMode::Menu;
            app.startup_dialog_fade = None;
            return;
        }
        for _ in 0..480 {
            if matches!(app.mode, AppMode::Menu) {
                if dismiss_first_player_dialog
                    && app.startup_player_properties_dialog.as_ref().is_some_and(
                        |pending| matches!(
                            &pending.origin,
                            StartupPlayerPropertiesOrigin::MainMenuFirstPlayer
                        ),
                    )
                {
                    app.process_startup_player_properties_actions(vec![
                        clonk_frontend::startup_plrproperties::PlayerPropertiesAction::Cancel,
                    ]);
                }
                app.startup_dialog_fade = None;
                return;
            }
            app.update().expect("tick while waiting for boot to finish");
            thread::sleep(Duration::from_millis(2));
        }
        panic!("app did not reach menu mode in time");
    }

    fn wait_for_running_with_attempts(app: &mut GameApp, attempts: usize) {
        for _ in 0..attempts {
            if matches!(app.mode, AppMode::Running) {
                return;
            }
            app.update()
                .expect("tick while waiting for scenario to start");
            thread::sleep(Duration::from_millis(2));
        }
        panic!(
            "scenario did not enter running mode in time (mode={:?}, status={})",
            app.mode, app.status_text
        );
    }

    fn wait_for_running(app: &mut GameApp) {
        wait_for_running_with_attempts(app, 480);
    }

    fn network_catch_up_fixture(
        ready_count: u32,
        control_rate: i32,
    ) -> (GameApp, FrameSchedule, Duration) {
        let mut app = new_running_sandbox_app();
        let (manager, events) = NetworkManager::test_stub();
        let start_tick = 41_u32;
        app.network = Some(manager);
        app.network_control_clock = Some(NetworkControlClock::new(
            i32::try_from(start_tick).unwrap(),
            control_rate,
        ));
        app.engine.initialize_network_control_timing(
            clonk_engine::NetworkControlTiming::new(
                i32::try_from(start_tick).unwrap(),
                control_rate,
            )
            .expect("valid catch-up control timing"),
        );
        for offset in 0..ready_count {
            events
                .send(NetworkEvent::ReadyTick {
                    tick: start_tick + offset,
                    controls: Vec::new(),
                })
                .expect("queue ready control tick");
        }
        let schedule = frame_schedule_for_mode(
            app.mode,
            app.engine.game_tick_delay_ms(),
            app.engine.game_tick_delay_revision(),
            app.max_refresh_delay_ms,
        );
        let accumulator = schedule.simulation_interval;
        (app, schedule, accumulator)
    }

    fn assert_selected_player_horizontal_release(auto_stop: bool) {
        // C4Game takes Config.General.Participants as PlayerFilenames
        // (C4Game.cpp:362-366), and C4Player::InitControl copies the player
        // file's AutoStopControl preference (C4Player.cpp:2371-2380).
        let install = tempdir().expect("install root");
        install_global_gui_test_root(install.path(), None);
        let player_dir = install.path().join("build/Tyler.c4p");
        fs::create_dir_all(&player_dir).expect("create player file group");
        fs::write(
            player_dir.join("Player.txt"),
            format!(
                "[Player]\nName=Tyler\nScore=250\nTotalPlayingTime=1234\n\n[Preferences]\nControl=0\nAutoStopControl={}\n",
                i32::from(auto_stop)
            ),
        )
        .expect("write player core");
        let user_dir = install.path().join("user-data");
        let _guard = EnvGuard::set(&[
            ("LC_INSTALL_ROOT", Some(install.path())),
            ("LC_USER_DATA_DIR", Some(user_dir.as_path())),
        ]);
        let paths = AppPaths::discover().expect("discover app paths");
        fs::create_dir_all(paths.config_dir()).expect("create config directory");
        fs::write(
            paths.config_file(),
            "[General]\nParticipants=Tyler.c4p\nPlayerPath=\n",
        )
        .expect("write config");
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

        app.join_local_player().expect("join selected player");

        let player = app
            .engine
            .snapshot()
            .players
            .into_iter()
            .find(|player| player.id == app.local_owner)
            .expect("joined player");
        assert_eq!(
            player.control.control_style, auto_stop,
            "the selected player's AutoStopControl preference must reach C4Player"
        );
        assert_eq!(player.player_info_id, 1);
        assert_eq!(player.name, "Tyler");
        assert_eq!(player.score, 250);
        assert_eq!(player.total_playing_time, 1_234);

        let cursor = app
            .engine
            .crew_cursor(app.local_owner)
            .expect("crew cursor");
        app.mode = AppMode::Running;
        let mut keyboard = AppVirtualKeyboard::new(&mut app);
        for (key, held_direction) in [
            (VirtualKeyCode::C, CommandDirection::Right),
            (VirtualKeyCode::Z, CommandDirection::Left),
        ] {
            keyboard.press(key).expect("press movement key");
            assert_eq!(
                keyboard
                    .engine()
                    .object_snapshot(cursor)
                    .expect("cursor after press")
                    .command_direction,
                held_direction,
                "pressed horizontal key must steer"
            );

            keyboard.release(key).expect("release movement key");
            assert_eq!(
                keyboard
                    .engine()
                    .object_snapshot(cursor)
                    .expect("cursor after release")
                    .command_direction,
                if auto_stop {
                    CommandDirection::Stop
                } else {
                    held_direction
                },
                "C4Game::LocalControlKeyUp only synchronizes releases in AutoStopControl mode"
            );
            assert_eq!(
                keyboard.player_control().pressed_coms == 0,
                auto_stop,
                "classic key-up is not synchronized, so C4Player::PressedComs remains held"
            );
        }
    }

    fn write_preview_png(path: &Path, pixel: [u8; 4]) {
        let file = File::create(path).expect("create preview image");
        let writer = BufWriter::new(file);
        let mut encoder = Encoder::new(writer, 1, 1);
        encoder.set_color(ColorType::Rgba);
        encoder.set_depth(BitDepth::Eight);
        let mut writer = encoder.write_header().expect("write PNG header");
        writer
            .write_image_data(&pixel)
            .expect("write PNG pixel data");
    }

    fn write_preview_image(path: &Path, pixel: [u8; 4], format: image::ImageFormat) {
        image::save_buffer_with_format(path, &pixel, 1, 1, image::ColorType::Rgba8, format)
            .expect("write preview image");
    }

    fn install_global_gui_test_root(root: &Path, missing_sheet: Option<&str>) {
        let repository = Path::new(env!("CARGO_MANIFEST_DIR"))
            .parent()
            .and_then(Path::parent)
            .and_then(Path::parent)
            .expect("repository root");
        let system = root.join("planet/System.c4g");
        let graphics = root.join("planet/Graphics.c4g");
        fs::create_dir_all(&system).expect("fixture System.c4g");
        fs::create_dir_all(&graphics).expect("fixture Graphics.c4g");
        fs::copy(
            repository.join("planet/System.c4g/Endeavour.ttf"),
            system.join("Endeavour.ttf"),
        )
        .expect("copy fixture Endeavour font");
        for (_, canonical_name) in CLASSIC_GLOBAL_GUI_SHEETS {
            if missing_sheet == Some(canonical_name) {
                continue;
            }
            fs::copy(
                repository.join("planet/Graphics.c4g").join(canonical_name),
                graphics.join(canonical_name),
            )
                .unwrap_or_else(|error| panic!("copy fixture {canonical_name}: {error}"));
        }
        for cursor_name in [
            "CursorSmall.png",
            "CursorMedium.png",
            "CursorLarge.png",
            "CursorXLarge.png",
            "CursorXXLarge.png",
            "CursorXXXLarge.png",
            "CursorXXXXLarge.png",
            "CursorXXXXXLarge.png",
        ] {
            fs::copy(
                repository.join("planet/Graphics.c4g").join(cursor_name),
                graphics.join(cursor_name),
            )
            .unwrap_or_else(|error| panic!("copy fixture {cursor_name}: {error}"));
        }
    }

    fn install_global_gui_and_loader_test_root(root: &Path) {
        install_global_gui_test_root(root, None);
        let repository = Path::new(env!("CARGO_MANIFEST_DIR"))
            .parent()
            .and_then(Path::parent)
            .and_then(Path::parent)
            .expect("repository root");
        fs::copy(
            repository.join("planet/Graphics.c4g/LoaderGoldmine1.png"),
            root.join("planet/Graphics.c4g/LoaderGoldmine1.png"),
        )
        .expect("copy fixture startup loader");
    }

    fn packed_test_group(entries: &[(&str, bool, &[u8])]) -> Vec<u8> {
        const HEADER_SIZE: usize = 204;
        const ENTRY_SIZE: usize = 316;
        const GROUP_FILE_ID: &[u8] = b"RedWolf Design GrpFolder";

        fn put_i32(buffer: &mut [u8], offset: usize, value: i32) {
            buffer[offset..offset + 4].copy_from_slice(&value.to_le_bytes());
        }

        let mut header = [0_u8; HEADER_SIZE];
        header[..GROUP_FILE_ID.len()].copy_from_slice(GROUP_FILE_ID);
        put_i32(&mut header, 28, 1);
        put_i32(&mut header, 32, 2);
        put_i32(
            &mut header,
            36,
            i32::try_from(entries.len()).expect("packed entry count fits"),
        );
        for byte in &mut header {
            *byte ^= 237;
        }
        for chunk in header.chunks_exact_mut(3) {
            chunk.swap(0, 2);
        }

        let mut image = header.to_vec();
        let mut data_offset = 0_usize;
        for (name, child, data) in entries {
            let mut entry = [0_u8; ENTRY_SIZE];
            entry[..name.len()].copy_from_slice(name.as_bytes());
            put_i32(&mut entry, 264, i32::from(*child));
            put_i32(
                &mut entry,
                268,
                i32::try_from(data.len()).expect("packed entry size fits"),
            );
            put_i32(
                &mut entry,
                276,
                i32::try_from(data_offset).expect("packed offset fits"),
            );
            image.extend_from_slice(&entry);
            data_offset += data.len();
        }
        for (_, _, data) in entries {
            image.extend_from_slice(data);
        }
        image
    }

    fn packed_test_file_group(entries: &[(&str, bool, &[u8])]) -> Vec<u8> {
        let mut group = clonk_resources::MutableGroup::new("Fixture.bin");
        for (name, child, data) in entries {
            if *child {
                group
                    .add_packed_child_with_metadata(*name, data.to_vec(), 0, 0, false)
                    .expect("add packed child fixture");
            } else {
                group
                    .add_file(*name, data.to_vec())
                    .expect("add packed file fixture");
            }
        }
        group.pack().expect("compress packed fixture")
    }

    fn make_object(id: u64, definition: &str, position: Vector2) -> ObjectSnapshot {
        ObjectSnapshot {
            id: ObjectId::new(id),
            definition_id: definition.to_string(),
            custom_name: None,
            position,
            velocity: Vector2::new(0, 0),
            rotation: 0,
            energy: 100,
            need_energy: false,
            construction: clonk_engine::FULL_CON,
            damage: 0,
            magic_energy: 0,
            magic_capacity: 0,
            action: ActionState::default(),
            direction: Direction::default(),
            command_direction: CommandDirection::default(),
            action_procedure: None,
            effects: Vec::new(),
            vertices: Vec::new(),
            current_shape: None,
            current_fire_top: None,
            contact_density: 50,
            own_vertices: None,
            vertex_contacts: Vec::new(),
            solid_mask_override: None,
            container: None,
            layer: None,
            visibility: 0,
            blit_mode: 0,
            color: 0,
            color_modulation: 0,
            picture_rect: Default::default(),
            contents: Vec::new(),
            components: HashMap::new(),
            component_order: Vec::new(),
            status: ObjectStatus::Normal,
            owner: 1,
            controller: 1,
            category: DEFAULT_CATEGORY,
            crew_member: true,
            plr_view_range: 0,
            selected: false,
            alive: true,
            base_graphics: None,
            graphics_overlays: Vec::new(),
            draw_transform: None,
            command_queue: Vec::new(),
            command_stack: CommandStackSnapshot::default(),
            local_vars: HashMap::new(),
            in_liquid: false,
            mobile: false,
            ocf: 0,
            timer: 0,
            own_mass: 0,
            on_fire: false,
            fire_phase: 0,
            fire_caused_by: -1,
            info_physical: None,
            temporary_physical: None,
            physical_changes: Vec::new(),
            breath: 0,
            last_energy_loss_cause: -1,
            base: -1,
            fixed_position: None,
            fixed_velocity: None,
            rotation_velocity: None,
            fixed_rotation: None,
        }
    }

    fn make_snapshot(
        objects: Vec<ObjectSnapshot>,
        hud_players: Vec<HudPlayerSnapshot>,
    ) -> SimulationSnapshot {
        let mut known_crew_owners: Vec<i32> =
            hud_players.iter().map(|player| player.owner).collect();
        known_crew_owners.sort_unstable();
        known_crew_owners.dedup();

        SimulationSnapshot {
            frame: 0,
            game_time: 0,
            game_over: false,
            round_results: Default::default(),
            league_name: Vec::new(),
            player_info_league_progress_data: Default::default(),
            player_info_league_scores: Default::default(),
            physics: None,
            objects,
            render_order: Vec::new(),
            environment: EnvironmentFrame::default(),
            sky: None,
            weather_events: Vec::new(),
            global_effects: Vec::new(),
            script_globals: Default::default(),
            particles: Vec::new(),
            players: Vec::new(),
            fow_players: Default::default(),
            crew_selection: HashMap::new(),
            crew_roles: HashMap::new(),
            known_crew_owners,
            eliminated_crew_owners: Vec::new(),
            landscape: None,
            rng: clonk_engine::LcgRng::seed_from_u64(1),
            surfaces: Vec::new(),
            hud: HudSnapshot {
                players: hud_players,
                messages: Vec::new(),
                scoreboard: Default::default(),
                scoreboard_presentations: Vec::new(),
                local_players: Vec::new(),
            },
            controls: Vec::new(),
            network_packets: Vec::new(),
            definition_categories: HashMap::new(),
            definition_closed_containers: Default::default(),
            definition_lines: HashMap::new(),
            transfer_zones: Vec::new(),
            pathfinder_debug: Default::default(),
            menu_requests: Vec::new(),
            audio: Vec::new(),
        }
    }

    fn silent_pcm_wav(duration_ms: u32) -> Vec<u8> {
        const SAMPLE_RATE: u32 = 44_100;
        const CHANNELS: u16 = 1;
        const BITS_PER_SAMPLE: u16 = 16;
        let sample_count = u64::from(duration_ms) * u64::from(SAMPLE_RATE) / 1_000;
        let data_len = u32::try_from(sample_count * u64::from(BITS_PER_SAMPLE / 8))
            .expect("test WAV fits u32");
        let mut bytes = Vec::with_capacity(44 + data_len as usize);
        bytes.extend_from_slice(b"RIFF");
        bytes.extend_from_slice(&(36 + data_len).to_le_bytes());
        bytes.extend_from_slice(b"WAVEfmt ");
        bytes.extend_from_slice(&16_u32.to_le_bytes());
        bytes.extend_from_slice(&1_u16.to_le_bytes());
        bytes.extend_from_slice(&CHANNELS.to_le_bytes());
        bytes.extend_from_slice(&SAMPLE_RATE.to_le_bytes());
        let byte_rate = SAMPLE_RATE * u32::from(CHANNELS) * u32::from(BITS_PER_SAMPLE / 8);
        bytes.extend_from_slice(&byte_rate.to_le_bytes());
        let block_align = CHANNELS * (BITS_PER_SAMPLE / 8);
        bytes.extend_from_slice(&block_align.to_le_bytes());
        bytes.extend_from_slice(&BITS_PER_SAMPLE.to_le_bytes());
        bytes.extend_from_slice(b"data");
        bytes.extend_from_slice(&data_len.to_le_bytes());
        bytes.resize(44 + data_len as usize, 0);
        bytes
    }

    fn test_audio_context_with_sound(
        duration_ms: u32,
    ) -> (tempfile::TempDir, AudioContext, SimulationSnapshot) {
        let dir = tempdir().expect("tempdir");
        let scenario = dir.path().join("Audio.c4s");
        fs::create_dir_all(&scenario).expect("create scenario group");
        fs::write(scenario.join("Loop.wav"), silent_pcm_wav(duration_ms))
            .expect("write test sound");

        let mut audio = AudioContext::try_new(AudioOptions {
            max_channels: 1,
            ..AudioOptions::default()
        })
        .expect("audio context");
        audio.configure_scenario(Some(&scenario));
        (dir, audio, make_snapshot(Vec::new(), Vec::new()))
    }

    fn test_sound_command(looped: bool) -> AudioCommand {
        AudioCommand::PlaySound {
            name: "Loop".to_string(),
            target: None,
            volume: 100,
            looped,
            multiple: false,
            custom_falloff: None,
        }
    }

    struct EnvGuard {
        _lock: parking_lot::ReentrantMutexGuard<'static, ()>,
        saved: Vec<(String, Option<OsString>)>,
    }

    impl EnvGuard {
        fn set(vars: &[(&str, Option<&Path>)]) -> Self {
            let lock = env_lock().lock();
            super::reset_cached_app_paths();
            let mut saved = Vec::with_capacity(vars.len());
            for (key, value) in vars {
                let original = env::var_os(key);
                saved.push((key.to_string(), original));
                match value {
                    Some(path) => env::set_var(key, path.as_os_str()),
                    None => env::remove_var(key),
                }
            }
            Self { _lock: lock, saved }
        }
    }

    impl Drop for EnvGuard {
        fn drop(&mut self) {
            for (key, value) in self.saved.drain(..) {
                match value {
                    Some(val) => env::set_var(&key, val),
                    None => env::remove_var(&key),
                }
            }
            super::reset_cached_app_paths();
        }
    }

    pub(super) fn env_lock() -> &'static ReentrantMutex<()> {
        static LOCK: OnceLock<ReentrantMutex<()>> = OnceLock::new();
        LOCK.get_or_init(|| ReentrantMutex::new(()))
    }

    fn empty_test_audio_context() -> AudioContext {
        AudioContext::try_new(AudioOptions {
            sound_enabled: false,
            music_enabled: false,
            menu_music_enabled: false,
            menu_sound_enabled: false,
            ..AudioOptions::default()
        })
        .expect("deferred test audio context")
    }

    fn audio_viewport(
        index: usize,
        owner: i32,
        center: Vector2,
    ) -> ActiveViewportProjection {
        ActiveViewportProjection {
            index,
            owner,
            is_no_owner_viewport: false,
            rect: Rect::new(0, 0, 200, 100),
            content_rect: Rect::new(0, 0, 200, 100),
            target_x: center.x - 100,
            target_y: center.y - 50,
            logical_width: 200,
            logical_height: 100,
            content_origin_x: 0.0,
            content_origin_y: 0.0,
            zoom: 1.0,
        }
    }

    fn test_font() -> Arc<dyn TextFont> {
        Arc::new(BitmapFont::new())
    }

    fn sample_scenarios() -> Vec<FrontendScenario> {
        let child = FrontendScenario {
            identifier: "scenario_alpha".to_string(),
            title: "Alpha".to_string(),
            description: None,
            kind: ScenarioKind::Scenario,
            is_editable: true,
            is_playable: true,
            mission_access: None,
            path: None,
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

        let folder = FrontendScenario {
            identifier: "folder_missions".to_string(),
            title: "Missions".to_string(),
            description: Some("Mission pack".to_string()),
            kind: ScenarioKind::Folder,
            is_editable: false,
            is_playable: false,
            mission_access: None,
            path: None,
            source_paths: Vec::new(),
            root_label: None,
            preview: None,
            children: vec![child],
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

        vec![folder]
    }

    fn install_native_test_fonts(app: &mut GameApp, scale: f32) {
        app.graphics.set_runtime_sprite_filtering(scale, false);
        app.loader_render_config = Some(
            LoaderRenderConfig::new(scale, false)
                .expect("install scale-native test render configuration"),
        );
        app.loader_render_error = None;
        let font_path =
            Path::new(env!("CARGO_MANIFEST_DIR")).join("../../../planet/System.c4g/Endeavour.ttf");
        let bytes = fs::read(font_path).expect("read Endeavour.ttf");
        app.native_startup_fonts = Some(Arc::new(
            clonk_frontend::clonk_fonts::build_native_font_set(&bytes, scale)
                .expect("build scale-native test fonts"),
        ));
    }

    fn render_ordered_test_frame(
        app: &mut GameApp,
        scale: f32,
        physical_width: u32,
        physical_height: u32,
    ) -> (Vec<u8>, Vec<u8>, NativePresentationPlan) {
        assert!(app.can_present_ordered_native_text(scale));
        let mut presenter = clonk_scaling::FramePresenter::new(scale, physical_width, physical_height);
        let mut output = vec![0_u8; physical_width as usize * physical_height as usize * 4];
        assert!(presenter
            .present(&mut output, |logical| app
                .render_ordered_native_base(logical))
            .expect("render ordered logical base"));
        let base = output.clone();
        let plan = app
            .pending_native_presentation
            .as_ref()
            .expect("ordered renderer prepared a presentation plan")
            .clone();

        let mut chrome_only = base.clone();
        {
            let mut composer = presenter.ordered_composer(&mut chrome_only);
            for batch in &plan.batches {
                if let Some(layer) = batch.logical_layer.as_ref() {
                    composer.begin_layer().copy_from_slice(layer);
                    if let Some(clip) = batch.clip {
                        composer.composite_layer_with_clip(clip);
                    } else {
                        composer.composite_layer();
                    }
                }
            }
        }
        {
            let mut composer = presenter.ordered_composer(&mut output);
            app.replay_pending_native_presentation(&mut composer)
                .expect("replay ordered native text");
        }
        (chrome_only, output, plan)
    }

    fn assert_startup_error_log(app: &GameApp, expected_message: &str) {
        use clonk_frontend::message_dialog::{
            MessageDialogButton, MessageDialogButtons, MessageDialogIcon, MessageDialogSize,
        };

        assert_eq!(app.message_dialogs.len(), 1);
        let dialog = &app.message_dialogs[0];
        assert_eq!(dialog.state.caption(), "Error Log");
        assert_eq!(dialog.state.message(), expected_message);
        assert_eq!(dialog.state.buttons(), MessageDialogButtons::OK);
        assert_eq!(dialog.state.focused_button(), Some(MessageDialogButton::Ok));
        assert_eq!(dialog.state.icon(), MessageDialogIcon::ERROR);
        assert_eq!(dialog.state.size(), MessageDialogSize::Regular);
        assert!(matches!(dialog.continuation, MessageDialogContinuation::None));
        assert!(app.status_text.is_empty());
    }

    fn tutorial_frontend(repository: &Path) -> FrontendScenario {
        let mut frontend = FrontendScenario::fallback();
        frontend.identifier = "Tutorial.c4f/Tutorial01.c4s".to_string();
        frontend.title = "selector title must not own the lobby".to_string();
        frontend.path = Some(repository.join("content/Tutorial.c4f/Tutorial01.c4s"));
        frontend
    }

    fn prepare_tutorial_host_lobby(app: &GameApp, repository: &Path) -> StagedNetworkHostScenario {
        app.prepare_network_host_scenario(
            tutorial_frontend(repository),
            ScenarioDefinitionLoad::Seed {
                modules: vec!["Objects.c4d".to_string()],
                definition_root: None,
            },
        )
        .expect("pre-bind exact host scenario")
    }

    fn install_minimal_prepared_host_fixture(content: &Path) -> FrontendScenario {
        let scenario_path = content.join("Fixture.c4s");
        let definition_path = content.join("Defs.c4d/Good.c4d");
        fs::create_dir_all(&scenario_path).expect("minimal prepared-host scenario group");
        fs::create_dir_all(&definition_path).expect("minimal prepared-host definition");
        fs::create_dir_all(content.join("Material.c4g"))
            .expect("minimal prepared-host material group");
        fs::write(
            scenario_path.join("Scenario.txt"),
            "[Head]\nTitle=Fixture\nIcon=2\nMaxPlayer=1\nNoInitialize=1\n\n[Definitions]\nDefinition1=Defs.c4d\n\n[Player1]\nCrew=GOOD=1\n",
        )
        .expect("minimal prepared-host scenario core");
        fs::write(
            definition_path.join("DefCore.txt"),
            "[DefCore]\nid=GOOD\nName=Good\nCategory=0\nCrewMember=0\n",
        )
        .expect("minimal prepared-host definition core");
        fs::write(definition_path.join("Script.c"), "// fixture\n")
            .expect("minimal prepared-host definition script");
        write_test_definition_graphics(&definition_path);

        let mut frontend = FrontendScenario::fallback();
        frontend.identifier = "Fixture.c4s".to_string();
        frontend.title = "selector title must not own the lobby".to_string();
        frontend.path = Some(scenario_path);
        frontend
    }

    fn minimal_prepared_host_definition_load() -> ScenarioDefinitionLoad {
        ScenarioDefinitionLoad::Seed {
            modules: vec!["Defs.c4d".to_string()],
            definition_root: None,
        }
    }

    fn prepare_minimal_host_lobby(
        app: &GameApp,
        frontend: FrontendScenario,
    ) -> StagedNetworkHostScenario {
        app.prepare_network_host_scenario(frontend, minimal_prepared_host_definition_load())
            .expect("pre-bind minimal host scenario")
    }

    fn install_network_definition_pack(root: &Path, module: &str, id: &str) -> PathBuf {
        let path = root.join(module);
        fs::create_dir_all(&path).expect("create network definition pack");
        fs::write(
            path.join("DefCore.txt"),
            format!(
                "[DefCore]\nid={id}\nName={id}\nCategory=0\nCrewMember=0\n"
            ),
        )
        .expect("write network definition core");
        fs::write(path.join("Script.c"), "// network definition fixture\n")
            .expect("write network definition script");
        write_test_definition_graphics(&path);
        path
    }

    fn packed_network_definition(module: &str, id: &str) -> clonk_resources::MutableGroup {
        let mut definition = clonk_resources::MutableGroup::new(module);
        definition
            .add_file(
                "DefCore.txt",
                format!(
                    "[DefCore]\nid={id}\nName={id}\nCategory=0\nCrewMember=0\n"
                )
                .into_bytes(),
            )
            .expect("add packed network definition core");
        definition
            .add_file("Script.c", b"// packed network definition fixture\n".to_vec())
            .expect("add packed network definition script");
        definition
            .add_file(
                "Graphics.png",
                include_bytes!("../../../../content/Material.c4g/Snow.png").to_vec(),
            )
            .expect("add packed network definition graphics");
        definition
    }

    fn prepare_staged_network_host(
        app: &GameApp,
        staged: &StagedNetworkHostScenario,
    ) -> PreparedHostBootstrap {
        build_network_host_preparation(
            app,
            &staged.frontend,
            &staged.definition_load,
            &staged.effective_definition_modules,
            &staged.definition_resources,
            Some((&staged.definition_executable_path, &staged.definition_path)),
            Some((&staged.lobby.local_name, &staged.lobby.nick)),
        )
        .expect("build staged network host preparation")
        .prepare()
        .expect("prepare staged network host")
    }

    fn published_definition_wire_names(prepared: &PreparedHostBootstrap) -> Vec<Vec<u8>> {
        prepared
            .host_config()
            .initial_join_snapshot
            .as_ref()
            .expect("prepared JoinData")
            .parameters
            .game_resources
            .iter()
            .filter(|core| {
                core.resource_type == clonk_network::HostResourceType::Definitions as u8
            })
            .map(|core| core.filename.as_bytes().to_vec())
            .collect()
    }

    fn install_test_classic_host_lobby(app: &mut GameApp) {
        app.startup_view = StartupView::NetworkLobby;
        app.classic_host_lobby = Some(ClassicHostLobbyState {
            controller: ClassicGameLobby::new(
                LobbyRole::Host,
                "Probe",
                0,
                4,
                false,
                false,
                false,
                false,
                5,
                vec![LobbyRosterRow::Client(LobbyClientRow {
                    id: 0,
                    name: "Exact Host".to_string(),
                    nick: "Exact Host".to_string(),
                    color: [255, 255, 255, 255],
                    status: LobbyClientStatus::Host,
                    local: true,
                    connected: false,
                    resource_progress: None,
                    ping_ms: None,
                })],
            ),
            preload: LobbyPreloadState::new(false),
            pointer: None,
            last_roster_click: None,
            chat_history_index: -1,
            runtime_join_allowed: false,
            resource_rows: BTreeMap::new(),
            scenario_description: LobbyScenarioDescriptionState::default(),
        });
        app.scenario_game_options =
            GameOptionButtons::new(GameOptionContext::LobbyHost, GameOptionValues::default());
        app.sync_scenario_game_option_bounds();
    }

    fn install_test_classic_host_team_lobby(
        app: &mut GameApp,
    ) -> (
        clonk_engine::ControlPlayerInfoEntry,
        clonk_engine::ControlPlayerInfoEntry,
    ) {
        install_test_classic_host_lobby(app);
        let client = app
            .classic_host_lobby
            .as_ref()
            .expect("test lobby")
            .controller
            .rows()[0]
            .clone();
        let player = LobbyRosterRow::Player(clonk_frontend::game_lobby::LobbyPlayerRow {
            id: 7,
            client_id: 0,
            name: "Chooser".to_string(),
            color: [255, 255, 255, 255],
            icon: clonk_frontend::game_lobby::LobbyRosterIcon::Standard(7),
            joined_player_overlay: None,
            team: Some(clonk_frontend::game_lobby::LobbyTeamValue {
                id: 1,
                name: "Full current".to_string(),
                selectable: true,
            }),
            league_score: None,
            league_rank: None,
        });
        let controller = ClassicGameLobby::new(
            LobbyRole::Host,
            "Probe",
            1,
            4,
            true,
            false,
            true,
            false,
            5,
            vec![client, player],
        );
        app.classic_host_lobby
            .as_mut()
            .expect("test lobby")
            .controller = controller;
        let teams = vec![
            clonk_engine::TeamInfo::new(1, "Full current", 0x00f4_0000)
                .with_player_ids(vec![7])
                .with_max_players(1),
            clonk_engine::TeamInfo::new(2, "Open", 0x0000_c800),
            clonk_engine::TeamInfo::new(3, "Full other", 0x0020_20ff)
                .with_player_ids(vec![8])
                .with_max_players(1),
            clonk_engine::TeamInfo::new(4, "Malformed negative maximum", 0x00ff_ffff)
                .with_max_players(-1),
        ];
        app.network_team_assignment = Some(NetworkTeamAssignmentState::from_prepared_host(
            clonk_engine::InitialNetworkTeamMetadata {
                active: true,
                custom: true,
                allow_hostility_change: false,
                allow_team_switch: false,
                auto_generate_teams: false,
                last_team_id: 4,
                team_distribution: clonk_engine::InitialNetworkTeamDistribution::Free,
                team_colors: false,
                max_script_players: 0,
                script_player_names: LegacyCString::default(),
                random_team_count: 0,
                teams: teams.iter().map(initial_team_from_runtime).collect(),
            },
        ));
        assert!(
            app.engine.teams().is_empty(),
            "the host lobby must use its retained pregame C4TeamList"
        );
        let chooser = clonk_engine::ControlPlayerInfoEntry {
            name: LegacyCString::from_bytes(b"Chooser".to_vec()).expect("valid name"),
            id: 7,
            team: 1,
            color: 0x0012_3456,
            original_color: 0x0012_3456,
            ..clonk_engine::ControlPlayerInfoEntry::default()
        };
        let companion = clonk_engine::ControlPlayerInfoEntry {
            name: LegacyCString::from_bytes(b"Companion".to_vec()).expect("valid name"),
            id: 8,
            team: 3,
            color: 0x0065_4321,
            original_color: 0x0065_4321,
            ..clonk_engine::ControlPlayerInfoEntry::default()
        };
        app.control_player_infos.replace_snapshot(
            8,
            [clonk_engine::PlayerInfoControlData {
                client_id: 0,
                flags: clonk_engine::CLIENT_PLAYER_INFO_FLAG_INITIAL,
                players: vec![chooser.clone(), companion.clone()],
                by_client: 0,
            }],
        );
        (chooser, companion)
    }

    fn install_classic_host_network_stub(
        app: &mut GameApp,
    ) -> (mpsc::Sender<NetworkEvent>, network::TestNetworkCommands) {
        install_test_classic_host_lobby(app);
        app.network_mode = Some(NetworkMode::Host(HostSettings {
            bind_addr: SocketAddr::from(([127, 0, 0, 1], 11112)),
            player_name: "Exact Host".to_string(),
            prepared: None,
        }));
        let (manager, events, commands) = NetworkManager::test_stub_with_commands();
        app.network = Some(manager);
        (events, commands)
    }

    fn install_test_free_savegame_player_row(app: &mut GameApp, player_id: i32) {
        install_test_classic_host_lobby(app);
        let client = app
            .classic_host_lobby
            .as_ref()
            .expect("test lobby")
            .controller
            .rows()[0]
            .clone();
        app.classic_host_lobby
            .as_mut()
            .expect("test lobby")
            .controller
            .set_rows(vec![
                LobbyRosterRow::Header(LobbyHeaderRow {
                    kind: LobbyRosterHeader::UnassignedSavegamePlayers,
                    label: "Player assignment".to_string(),
                    icon: LobbyRosterIcon::Standard(12),
                    can_add_player: false,
                }),
                LobbyRosterRow::Player(LobbyPlayerRow {
                    id: player_id,
                    client_id: -1,
                    name: "Free restore".to_string(),
                    color: [0xff; 4],
                    icon: LobbyRosterIcon::Standard(7),
                    joined_player_overlay: None,
                    team: None,
                    league_score: None,
                    league_rank: None,
                }),
                client,
            ]);
    }

    fn l100_script_player_add_fixture(
        configured_names: &[u8],
        active_players: &[(&[u8], bool)],
        max_script_players: i32,
    ) -> (GameApp, network::TestNetworkCommands) {
        let mut app = new_state_only_menu_app(320, 200);
        app.control_player_infos.replace_snapshot(
            1,
            [clonk_engine::PlayerInfoControlData {
                client_id: 0,
                players: active_players
                    .iter()
                    .enumerate()
                    .map(|(index, (name, script_player))| {
                        clonk_engine::ControlPlayerInfoEntry {
                            id: i32::try_from(index + 1).unwrap(),
                            name: LegacyCString::from_bytes(name.to_vec()).unwrap(),
                            player_type: if *script_player {
                                clonk_engine::PLAYER_INFO_TYPE_SCRIPT
                            } else {
                                clonk_engine::PLAYER_INFO_TYPE_USER
                            },
                            ..Default::default()
                        }
                    })
                    .collect(),
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
                last_team_id: 0,
                team_distribution: clonk_engine::InitialNetworkTeamDistribution::Free,
                team_colors: false,
                max_script_players,
                script_player_names: LegacyCString::from_bytes(configured_names.to_vec()).unwrap(),
                random_team_count: 0,
                teams: Vec::new(),
            },
        ));
        let (manager, _events, commands) =
            NetworkManager::test_stub_with_commands_for_client_id(0);
        app.network = Some(manager);
        app.network_mode = Some(NetworkMode::Host(host_network_settings()));
        (app, commands)
    }

    fn new_menu_app(width: u32, height: u32) -> GameApp {
        let mut app = new_state_only_menu_app(width, height);
        install_synthetic_classic_test_assets(&mut app);
        app
    }

    fn new_real_menu_app(width: u32, height: u32) -> GameApp {
        let mut app = new_state_only_menu_app(width, height);
        install_classic_test_assets(&mut app);
        apply_test_post_migration_renderer_config(&mut app);
        app
    }

    fn apply_test_post_migration_renderer_config(app: &mut GameApp) {
        // These pathless fixtures bypass clonk-game::prepare_config. Model the
        // post-AdaptToCurrentVersion device state used by a normal launch.
        let renderer_config = app.graphics.advanced_renderer_config();
        app.graphics
            .set_advanced_renderer_config(clonk_frontend::AdvancedRendererConfig {
                shader: true,
                ..renderer_config
            });
    }

    fn new_state_only_menu_app(width: u32, height: u32) -> GameApp {
        new_menu_app_with_frontend_scenarios(width, height, Some(Vec::new()))
    }

    fn new_discovered_menu_app(width: u32, height: u32) -> GameApp {
        let mut app = new_menu_app_with_frontend_scenarios(width, height, None);
        install_classic_test_assets(&mut app);
        app
    }

    fn new_menu_app_with_frontend_scenarios(
        width: u32,
        height: u32,
        frontend_scenarios: Option<Vec<FrontendScenario>>,
    ) -> GameApp {
        let mut app = GameApp::new_with_frontend_scenarios(
            width,
            height,
            AudioOptions::default(),
            None,
            RuntimeConfig {
                player_owner: 1,
                player_name: "Player".to_string(),
                network: None,
                record_enabled: false,
            },
            frontend_scenarios,
        )
        .expect("initialise app");
        wait_for_menu(&mut app);
        app
    }

    fn default_exact_host_reference()
    -> (clonk_network::HostJoinSnapshot, clonk_network::HostGameReference) {
        let host_config = clonk_network::HostConfig::default();
        let snapshot = host_config
            .initial_join_snapshot
            .expect("default host JoinData");
        let parameters = snapshot.parameters.clone();
        let reference = clonk_network::HostGameReference::new(
            clonk_network::NetworkGameReference {
                title: "No title".to_string(),
                host_name: "Host".to_string(),
                host_nick: "Host".to_string(),
                state: "Lobby".to_string(),
                control_mode: host_config.initial_status.control_mode,
                join_allowed: true,
                max_players: parameters.max_players,
                ..Default::default()
            },
            clonk_network::HostGameReferenceMetadata::default(),
            parameters,
        )
        .expect("valid exact host reference");
        (snapshot, reference)
    }

    fn new_menu_app_with_paths(width: u32, height: u32, paths: &AppPaths) -> GameApp {
        let mut app = GameApp::new(
            width,
            height,
            AudioOptions::default(),
            Some(paths),
            RuntimeConfig {
                player_owner: 1,
                player_name: "Player".to_string(),
                network: None,
                record_enabled: false,
            },
        )
        .expect("initialise app with paths");
        wait_for_menu(&mut app);
        app
    }

    fn wait_for_scenario_selector_discovery(app: &mut GameApp) {
        let deadline = Instant::now() + Duration::from_secs(30);
        while app.scenario_selector_discovery.is_some() {
            app.poll_scenario_selector_discovery()
                .expect("poll scenario selector discovery");
            assert!(
                Instant::now() < deadline,
                "scenario selector discovery did not finish"
            );
            thread::yield_now();
        }
    }

    fn exact_loader_test_paths(
        user_data: &Path,
        content_dir: Option<&Path>,
    ) -> (EnvGuard, AppPaths) {
        let repository = Path::new(env!("CARGO_MANIFEST_DIR"))
            .parent()
            .and_then(Path::parent)
            .and_then(Path::parent)
            .expect("repository root");
        let guard = EnvGuard::set(&[
            ("LC_INSTALL_ROOT", Some(repository)),
            ("LC_CONTENT_DIR", content_dir),
            ("LC_USER_DATA_DIR", Some(user_data)),
        ]);
        let paths = AppPaths::discover().expect("discover exact loader test paths");
        paths.ensure_user_dirs().expect("loader test user dirs");
        persist_config_value(&paths, "General", "LanguageEx", "US")
            .expect("configure exact loader test language");
        persist_config_value(&paths, "Network", "LocalName", "Exact Host")
            .expect("configure exact loader test local name");
        (guard, paths)
    }

    fn configure_test_startup_participant(paths: &AppPaths, root: &Path) {
        let player = root.join("Exact.c4p");
        let mut group = clonk_resources::MutableGroup::new("Exact.c4p");
        group
            .add_file_with_metadata(
                "Player.txt",
                b"[Player]\nName=Exact Player\n\n[Preferences]\nControl=0\nMouse=1\nAutoStopControl=0\nColorDw=255\n"
                    .to_vec(),
                1,
                false,
            )
            .expect("add exact test player core");
        group
            .add_file_with_metadata(
                "BigIcon.png",
                encode_screenshot_png(1, 1, &[12, 34, 56, 255])
                    .expect("encode exact test player icon"),
                1,
                false,
            )
            .expect("add exact test player icon");
        fs::write(&player, group.pack().expect("pack exact test player"))
            .expect("write exact test player");
        let packed = Group::open(&player).expect("reopen exact test player group");
        PlayerFile::load(&packed).expect("parse exact test player core");
        persist_config_value(
            paths,
            "General",
            "PlayerPath",
            root.to_string_lossy().into_owned(),
        )
        .expect("configure exact test player path");
        persist_config_value(
            paths,
            "General",
            "Participants",
            player.to_string_lossy().into_owned(),
        )
        .expect("configure exact test participant");
        let configured = clonk_app::load_configured_client_players(paths)
            .expect("reload configured test participant");
        assert_eq!(
            configured.players().len(),
            1,
            "the packed participant must pass the same loader used by C4Game startup; config={}",
            String::from_utf8_lossy(
                &fs::read(paths.config_file()).expect("read configured test participant")
            )
        );
    }

    fn synthetic_classic_test_font(
        line_height: i32,
        role: ClonkFontRole,
        shadowed: bool,
    ) -> ClonkFont {
        let mut font = ClonkFont::new(line_height).with_role(role);
        if !shadowed {
            font.cell_height = line_height;
            font.h_space = 0;
        }
        let width = (line_height / 2).max(1);
        let opaque = GlyphCell {
            width,
            pixels: vec![
                Color::opaque(255, 0, 255);
                usize::try_from(width * font.cell_height).expect("synthetic glyph dimensions")
            ],
        };
        let transparent = GlyphCell {
            width,
            pixels: vec![
                Color::transparent();
                usize::try_from(width * font.cell_height).expect("synthetic glyph dimensions")
            ],
        };
        for ch in ' '..='~' {
            font.add_glyph(ch, if ch == 'A' { opaque.clone() } else { transparent.clone() });
        }
        font.add_glyph('\u{a6}', transparent.clone());
        font.set_missing_glyph(transparent);
        font
    }

    fn synthetic_classic_test_assets() -> FrontendAssets {
        let mut assets = FrontendAssets::load(None);
        let fonts = Arc::new(clonk_frontend::ClonkFontSet {
            title: synthetic_classic_test_font(34, ClonkFontRole::GuiTitle, true),
            caption: synthetic_classic_test_font(25, ClonkFontRole::GuiCaption, true),
            text: synthetic_classic_test_font(22, ClonkFontRole::GuiText, true),
            main_small: synthetic_classic_test_font(20, ClonkFontRole::GuiMainSmall, true),
            mini: synthetic_classic_test_font(18, ClonkFontRole::GuiMini, true),
        });
        let tooltip = Arc::new(synthetic_classic_test_font(
            22,
            ClonkFontRole::GuiTooltip,
            false,
        ));
        assets.clonk_fonts = Some(Arc::clone(&fonts));
        assets.startup_clonk_fonts = Some(fonts);
        assets.global_tooltip_font = Some(Arc::clone(&tooltip));
        assets.startup_global_tooltip_font = Some(tooltip);

        for (name, width, height) in [
            ("GUICaption.png", 192, 23),
            ("GUIButton.png", 128, 32),
            ("GUIButtonDown.png", 128, 32),
            ("GUIButtonHighlight.png", 16, 16),
            ("GUIIcons.png", 240, 360),
            ("GUIIcons2.png", 256, 320),
            ("GUIScroll.png", 32, 48),
            ("GUIContext.png", 32, 16),
            ("GUISubmenu.png", 8, 16),
            ("GUICheckbox.png", 128, 32),
            ("GUIBigArrows.png", 76, 40),
            ("GUIProgress.png", 32, 32),
            ("GUISpinBoxArrow.png", 13, 8),
        ] {
            assets.startup_dialog_images.insert(
                name.to_string(),
                ImageData::new(width, height, vec![0; width as usize * height as usize * 4]),
            );
        }
        assets.button_highlight = assets
            .startup_dialog_images
            .get("GUIButtonHighlight.png")
            .cloned();
        assets.game_over_button_highlight = assets
            .button_highlight
            .as_ref()
            .map(clonk_frontend::classic_gui::blacken_transparent_pixels);
        assets
    }

    fn install_synthetic_classic_test_assets(app: &mut GameApp) {
        static SYNTHETIC_CLASSIC_TEST_ASSETS: OnceLock<FrontendAssets> = OnceLock::new();
        let assets = Arc::new(
            SYNTHETIC_CLASSIC_TEST_ASSETS
                .get_or_init(synthetic_classic_test_assets)
                .clone(),
        );
        assets
            .require_classic_global_gui_bootstrap_resources(&HashMap::new())
            .expect("synthetic process-global GUI fixture is complete");
        apply_test_frontend_assets(app, assets);
    }

    fn install_classic_test_assets(app: &mut GameApp) {
        static CLASSIC_TEST_ASSETS: OnceLock<FrontendAssets> = OnceLock::new();
        let assets = Arc::new(
            CLASSIC_TEST_ASSETS
                .get_or_init(|| {
                    let repository = Path::new(env!("CARGO_MANIFEST_DIR"))
                        .parent()
                        .and_then(Path::parent)
                        .and_then(Path::parent)
                        .expect("repository root");
                    let _guard = EnvGuard::set(&[("LC_INSTALL_ROOT", Some(repository))]);
                    let paths = AppPaths::discover().expect("discover repository assets");
                    FrontendAssets::load(Some(&paths))
                })
                .clone(),
        );
        assets
            .require_classic_global_gui_bootstrap_resources(&HashMap::new())
            .expect("repository ships the complete process-global GUI bootstrap");
        assets
            .require_classic_startup_bootstrap_resources()
            .expect("repository ships the complete classic startup bootstrap");
        assets
            .require_classic_startup_main_resources()
            .expect("repository ships exact startup-main resources");
        assets
            .require_classic_ingame_menu_resources()
            .expect("repository ships exact in-game menu resources");

        apply_test_frontend_assets(app, assets);
    }

    fn apply_test_frontend_assets(app: &mut GameApp, assets: Arc<FrontendAssets>) {
        let mut main_menu = StartupMainMenu::new(assets.font_arc(), assets.button_textures());
        main_menu.set_highlight_texture(assets.button_highlight.clone());
        main_menu.set_clonk_fonts(assets.clonk_fonts.clone());
        main_menu.set_gamma_ramp(Some(Arc::new(clonk_graphics::GammaRamp::standard())));
        let surface = app.graphics.surface();
        main_menu.resize(surface.width() as f32, surface.height() as f32);
        app.main_menu_state.menu = main_menu;
        app.graphics.set_clonk_fonts(assets.clonk_fonts.clone());
        app.assets = assets;
        app.active_global_gui_failures.clear();
        app.menu_frame_cache = None;
        app.menu_backdrop_cache = StartupBackdropCache::default();
        app.mark_menu_dirty();
    }

    fn l018_cursor_atlas() -> Arc<CursorAtlas> {
        let cell = 4u32;
        let mut pixels = Vec::with_capacity((40 * cell * cell * 4) as usize);
        for _y in 0..cell {
            for x in 0..40 * cell {
                let phase = (x / cell) as u8;
                pixels.extend_from_slice(&[phase, phase.wrapping_add(40), 200, 255]);
            }
        }
        let mut entries = vec![None; 8];
        entries[7] = Some(ImageData::new(40 * cell, cell, pixels));
        Arc::new(CursorAtlas::new(entries))
    }

    fn install_l018_cursor_atlas(app: &mut GameApp) {
        assert!(
            app.active_game_graphics.is_none(),
            "focused cursor fixtures use the process atlas"
        );
        Arc::get_mut(&mut app.assets)
            .expect("focused fixture uniquely owns frontend assets")
            .cursor_atlas = l018_cursor_atlas();
        let (width, height) = {
            let surface = app.graphics.surface();
            (surface.width(), surface.height())
        };
        app.resize(width, height)
            .expect("rebuild graphics with focused cursor atlas");
    }

    fn new_classic_menu_app(width: u32, height: u32) -> GameApp {
        new_menu_app(width, height)
    }

    fn new_real_classic_menu_app(width: u32, height: u32) -> GameApp {
        new_real_menu_app(width, height)
    }

    fn assert_engine_parity_boundary(error: EngineError, expected: ClassicParityBoundary) {
        match error {
            EngineError::ClassicMenuParityBoundary { detail } => {
                assert_eq!(detail, expected.to_string())
            }
            other => panic!("unexpected engine error: {other}"),
        }
    }

    fn assert_startup_bootstrap_boundary(
        error: &anyhow::Error,
        expected_issues: Vec<ClassicStartupBootstrapIssue>,
    ) {
        let expected = ClassicParityBoundary::StartupBootstrapResources {
            issues: expected_issues,
        };
        assert_eq!(
            error.downcast_ref::<ClassicParityBoundary>(),
            Some(&expected)
        );
        assert!(
            error
                .to_string()
                .contains("refusing every startup root before cache or pixels"),
            "boundary must explain the all-root refusal: {error:#}"
        );
    }

    fn assert_global_gui_boundary(
        error: &anyhow::Error,
        expected_issues: Vec<ClassicGuiBootstrapIssue>,
    ) {
        let expected = ClassicParityBoundary::GlobalGuiBootstrapResources {
            issues: expected_issues,
        };
        assert_eq!(
            error.downcast_ref::<ClassicParityBoundary>(),
            Some(&expected)
        );
        assert!(
            error
                .to_string()
                .contains("before mutation, cache, or pixels"),
            "boundary must explain the process-global refusal: {error:#}"
        );
    }

    fn remove_global_gui_sheet(app: &mut GameApp, canonical_name: &str) -> ImageData {
        Arc::get_mut(&mut app.assets)
            .expect("frontend assets are app-owned")
            .startup_dialog_images
            .remove(canonical_name)
            .expect("global GUI fixture sheet")
    }

    fn activate_startup_network_chat(app: &mut GameApp) {
        let metrics = clonk_frontend::startup_netdlg::NetDlgFontMetrics {
            caption_back_extent: 51,
            text_ip_extent: 18,
            text_line_height: 22,
            caption_line_height: 25,
            title_line_height: 34,
        };
        let button =
            clonk_frontend::startup_netdlg::net_dlg_layout(640, 480, &metrics).btn_chat;
        let point = PhysicalPosition::new(
            f64::from(button.x + button.w / 2),
            f64::from(button.y + button.h / 2),
        );
        app.handle_cursor_moved(point).expect("hover Chat");
        app.handle_mouse_button(ElementState::Pressed)
            .expect("press Chat");
        app.handle_mouse_button(ElementState::Released)
            .expect("open the retained Chat page");
    }

    fn spawn_loopback_irc_server() -> (String, thread::JoinHandle<()>) {
        let listener = std::net::TcpListener::bind("127.0.0.1:0")
            .expect("bind loopback IRC listener");
        let address = listener
            .local_addr()
            .expect("read loopback IRC listener address")
            .to_string();
        let server = thread::spawn(move || {
            let (mut stream, _) = listener.accept().expect("accept loopback IRC client");
            let mut buffer = [0_u8; 512];
            while stream.read(&mut buffer).is_ok_and(|read| read != 0) {}
        });
        (address, server)
    }

    fn enter_unported_startup_subscreen(app: &mut GameApp, subscreen: ClassicStartupSubscreen) {
        match subscreen {
            ClassicStartupSubscreen::Options(target) => {
                app.open_options_menu();
                let sheets = [
                    clonk_frontend::startup_options_dlg::OptionsSheet::Graphics,
                    clonk_frontend::startup_options_dlg::OptionsSheet::Sound,
                    clonk_frontend::startup_options_dlg::OptionsSheet::Keyboard,
                    clonk_frontend::startup_options_dlg::OptionsSheet::Gamepad,
                    clonk_frontend::startup_options_dlg::OptionsSheet::Network,
                ];
                for sheet in sheets {
                    app.handle_key(VirtualKeyCode::Down, ElementState::Pressed)
                        .unwrap_or_else(|error| panic!("open Options {sheet:?}: {error}"));
                    app.handle_key(VirtualKeyCode::Down, ElementState::Released)
                        .expect("release options sheet key");
                    if sheet == target {
                        break;
                    }
                }
            }
        }
    }

    fn enter_about_licenses(app: &mut GameApp) {
        app.open_about_dialog();
        let surface = app.graphics.surface();
        let button = clonk_frontend::startup_about_dlg::about_layout(
            surface.width() as i32,
            surface.height() as i32,
        )
        .buttons[2];
        let point = PhysicalPosition::new(
            f64::from(button.x + button.w / 2),
            f64::from(button.y + button.h / 2),
        );
        app.handle_cursor_moved(point).expect("hover Licenses");
        app.handle_mouse_button(ElementState::Pressed)
            .expect("press Licenses");
        app.handle_mouse_button(ElementState::Released)
            .expect("open the classic Licenses page");
        assert_eq!(
            app.startup_about_dialog.as_ref().unwrap().current_page(),
            clonk_frontend::startup_about_dlg::AboutPage::Licenses
        );
    }

    #[derive(Clone, Copy)]
    enum RetainedStartupChild {
        Unported(ClassicStartupSubscreen),
        OptionsSound,
        AboutLicenses,
    }

    fn enter_retained_startup_child(app: &mut GameApp, child: RetainedStartupChild) {
        match child {
            RetainedStartupChild::Unported(subscreen) => {
                enter_unported_startup_subscreen(app, subscreen)
            }
            RetainedStartupChild::OptionsSound => enter_unported_startup_subscreen(
                app,
                ClassicStartupSubscreen::Options(
                    clonk_frontend::startup_options_dlg::OptionsSheet::Sound,
                ),
            ),
            RetainedStartupChild::AboutLicenses => enter_about_licenses(app),
        }
    }

    fn startup_player_properties_validation_app(
        user_data: &Path,
    ) -> (EnvGuard, AppPaths, PathBuf, GameApp) {
        let (guard, paths) = exact_loader_test_paths(user_data, None);
        let player_root = user_data.join("Players");
        fs::create_dir_all(&player_root).expect("create validation player root");
        persist_config_value(
            &paths,
            "General",
            "PlayerPath",
            player_root.to_string_lossy(),
        )
        .expect("configure validation player path");
        let mut app = new_classic_menu_app(640, 480);
        app.app_paths = Some(paths.clone());
        app.open_player_selection_dialog();
        app.open_new_startup_player_properties();
        (guard, paths, player_root, app)
    }

    fn write_map_png(path: &Path, width: u32, height: u32, pixel: [u8; 4]) {
        image::RgbaImage::from_pixel(width, height, image::Rgba(pixel))
            .save(path)
            .expect("write FolderMap image");
    }

    fn retained_test_presentation(app: &GameApp) -> GpuPresentation {
        GpuPresentation::identity(
            app.graphics.surface().width(),
            app.graphics.surface().height(),
        )
    }

    fn assert_retained_frame_has_commands(label: &str, frame: &RetainedGpuFrame) {
        assert!(!frame.layers.is_empty(), "{label} produced no GPU layers");
        assert!(
            frame
                .layers
                .iter()
                .any(|layer| !layer.scene.commands.is_empty()),
            "{label} produced no retained GPU commands"
        );
    }

    fn loader_origin_fixture_paths(root: &Path) -> (EnvGuard, AppPaths, PathBuf) {
        let planet = root.join("planet");
        fs::create_dir_all(&planet).expect("fixture planet directory");
        fs::write(planet.join("System.c4g"), b"stub").expect("fixture System group");
        let content = root.join("content");
        fs::create_dir_all(&content).expect("fixture content directory");
        let user_data = root.join("user-data");
        let guard = EnvGuard::set(&[
            ("LC_INSTALL_ROOT", Some(root)),
            ("LC_CONTENT_DIR", Some(content.as_path())),
            ("LC_USER_DATA_DIR", Some(user_data.as_path())),
        ]);
        let paths = AppPaths::discover().expect("fixture app paths");
        (guard, paths, content)
    }

    fn l038_running_browser_sandbox(selector_mode: ScenarioSelectorMode) -> GameApp {
        let scenarios = sample_scenarios();
        let menu = StartupMenu::new(build_menu_entries(&scenarios, false), test_font(), None)
            .expect("build L038 scenario menu");
        let mut app = new_real_menu_app(640, 480);
        app.menu_state = MenuState::new(menu, scenarios.clone());
        app.scenario_catalog = build_scenario_catalog(&scenarios);
        app.open_scenario_browser_with_mode(selector_mode);
        app.enter_scenario_folder("folder_missions");
        assert_eq!(app.menu_state.stack.len(), 2);
        if selector_mode == ScenarioSelectorMode::Local {
            app.handle_menu_input(|_| {
                vec![StartupMenuAction::StartScenario(
                    clonk_frontend::ScenarioSummary {
                        identifier: "scenario_alpha".to_string(),
                        title: "Alpha".to_string(),
                        kind: ScenarioKind::Scenario,
                    },
                )]
            })
            .expect("start L038 round through the scenario browser");
        } else {
            // A pathless fixture cannot enter the real prepared-host pipeline;
            // this branch isolates the startup-dialog memory across the
            // otherwise transient lobby view.
            app.start_sandbox_scenario(FrontendScenario::fallback())
                .expect("start hosted L038 state probe");
        }
        wait_for_running(&mut app);
        app
    }

    fn assert_l038_browser_return(app: &GameApp, selector_mode: ScenarioSelectorMode) {
        assert!(matches!(app.mode, AppMode::Menu));
        assert_eq!(app.startup_view, StartupView::ScenarioBrowser);
        assert_eq!(app.scenario_selector_mode, selector_mode);
        assert_eq!(
            app.last_startup_dialog,
            StartupDialog::ScenarioBrowser(selector_mode)
        );
        assert_eq!(app.startup_scenario_back_dialog, None);
        assert_eq!(
            app.menu_state.stack.len(),
            1,
            "native rebuilds the remembered scenario dialog at its root"
        );
        assert_eq!(app.scenario_label, app.menu_state.label_path());
    }

    fn attach_l040_network_dialog(app: &mut GameApp) {
        let metrics = clonk_frontend::startup_netdlg::NetDlgFontMetrics::from_fonts(
            app.assets
                .clonk_fonts
                .as_deref()
                .expect("classic startup fonts"),
        );
        let mut dialog = clonk_frontend::startup_netdlg::NetDlgController::new(
            clonk_frontend::startup_netdlg::NetDlgConfig {
                masterserver_signup: true,
                record: false,
            },
            metrics,
        );
        dialog.set_text_font(
            &app.assets
                .clonk_fonts
                .as_deref()
                .expect("classic startup fonts")
                .text,
        );
        dialog.resize(800, 600);
        app.startup_view = StartupView::NetworkGame;
        app.startup_network_dialog = Some(dialog);
        app.startup_game_search = None;
    }

    fn new_running_sandbox_app() -> GameApp {
        // Most state/input tests need the shipped CLNK definition but not the
        // much heavier shipped frontend bundle. Pixel/resource contracts use
        // the explicit classic constructor below.
        let paths = cached_app_paths().expect("discover sandbox crew definition");
        new_running_sandbox_app_with_definitions(SandboxDefinitionLoad::InstallCrew(
            paths.as_ref(),
        ))
    }

    fn finish_abort_dialog(
        app: &mut GameApp,
        result: clonk_frontend::message_dialog::MessageDialogResult,
    ) {
        assert!(app.message_dialogs.last().is_some_and(|dialog| matches!(
            dialog.continuation,
            MessageDialogContinuation::AbortGame { .. }
        )));
        app.finish_message_dialog(result)
            .expect("finish C4AbortGameDialog");
    }

    fn confirm_abort_dialog(app: &mut GameApp) {
        assert!(app.show_abort_dialog(app.local_owner));
        finish_abort_dialog(
            app,
            clonk_frontend::message_dialog::MessageDialogResult::Yes,
        );
    }

    fn new_classic_running_sandbox_app() -> GameApp {
        let paths = cached_app_paths().expect("discover sandbox crew definition");
        let mut app = new_running_sandbox_app_with_definitions_and_assets(
            SandboxDefinitionLoad::InstallCrew(paths.as_ref()),
            SandboxFixtureAssets::Classic,
        );
        apply_test_post_migration_renderer_config(&mut app);
        app
    }

    fn new_state_only_running_sandbox_app() -> GameApp {
        let paths = cached_app_paths().expect("discover sandbox crew definition");
        new_running_sandbox_app_with_definitions_and_assets(
            SandboxDefinitionLoad::InstallCrew(paths.as_ref()),
            SandboxFixtureAssets::StateOnly,
        )
    }

    fn new_state_only_lightweight_running_sandbox_app() -> GameApp {
        new_running_sandbox_app_with_definitions_and_assets(
            SandboxDefinitionLoad::None,
            SandboxFixtureAssets::StateOnly,
        )
    }

    fn install_synthetic_sandbox_crew_definition(app: &mut GameApp) {
        // Default PlayerStart uses one native CLNK. Tests which exercise a
        // later player activation therefore need that exact definition ID,
        // but not the shipped definition's scripts, graphics, or actions.
        let mut crew = Definition::from_script("CLNK", "Synthetic Clonk", "#strict\n")
            .expect("synthetic sandbox crew compiles");
        crew.set_crew_member(true);
        crew.set_category(clonk_engine::CATEGORY_OBJECT | clonk_engine::CATEGORY_LIVING);
        crew.set_physical(clonk_engine::PhysicalInfo {
            energy: 50_000,
            breath: 50_000,
            ..clonk_engine::PhysicalInfo::default()
        });
        app.engine
            .register_definition(crew)
            .expect("synthetic sandbox crew registers");
    }

    fn new_state_only_synthetic_crew_running_sandbox_app() -> GameApp {
        let mut app = new_state_only_lightweight_running_sandbox_app();
        install_synthetic_sandbox_crew_definition(&mut app);
        app
    }

    fn new_synthetic_running_sandbox_app() -> GameApp {
        let mut app = new_running_sandbox_app_with_definitions_and_assets(
            SandboxDefinitionLoad::None,
            SandboxFixtureAssets::Synthetic,
        );
        install_synthetic_sandbox_crew_definition(&mut app);
        app
    }

    fn new_classic_lightweight_running_sandbox_app() -> GameApp {
        new_running_sandbox_app_with_definitions_and_assets(
            SandboxDefinitionLoad::None,
            SandboxFixtureAssets::Classic,
        )
    }

    fn new_lightweight_running_sandbox_app() -> GameApp {
        new_running_sandbox_app_with_definitions(SandboxDefinitionLoad::None)
    }

    fn new_running_sandbox_app_with_definitions(
        definition_load: SandboxDefinitionLoad<'_>,
    ) -> GameApp {
        new_running_sandbox_app_with_definitions_and_assets(
            definition_load,
            SandboxFixtureAssets::Synthetic,
        )
    }

    enum SandboxFixtureAssets {
        StateOnly,
        Synthetic,
        Classic,
    }

    fn hold_message_board_for_frame_comparison(app: &mut GameApp) {
        // Pixel-composition tests compare consecutive presentations. Keep the
        // seeded join line in C4MessageBoard's stable single-line delay phase;
        // the L120 regressions exercise the animated transitions explicitly.
        app.message_board.back_scroll = 0;
        app.message_board.empty = false;
        app.message_board.fader = 0;
        app.message_board.delay = i32::MAX;
        app.message_board.speed = 1;
        app.message_board.screen_fader = -100;
    }

    fn new_running_sandbox_app_with_definitions_and_assets(
        definition_load: SandboxDefinitionLoad<'_>,
        fixture_assets: SandboxFixtureAssets,
    ) -> GameApp {
        // Silent audio: keeps these apps from initialising the global
        // sandbox-music OnceLock while env-guarded tests run in parallel.
        let audio_options = AudioOptions {
            sound_enabled: false,
            music_enabled: false,
            menu_music_enabled: false,
            menu_sound_enabled: false,
            ..AudioOptions::default()
        };
        let mut app = GameApp::new_with_frontend_scenarios(
            320,
            200,
            audio_options,
            None,
            RuntimeConfig {
                player_owner: 1,
                player_name: "Player".to_string(),
                network: None,
                record_enabled: false,
            },
            Some(Vec::new()),
        )
        .expect("initialise app");
        match fixture_assets {
            SandboxFixtureAssets::StateOnly => {}
            SandboxFixtureAssets::Synthetic => install_synthetic_classic_test_assets(&mut app),
            SandboxFixtureAssets::Classic => install_classic_test_assets(&mut app),
        }
        // Keep the app itself pathless while choosing exactly how much
        // definition data this fixture needs. The default uses the shipped
        // CLNK for native shape/action/portrait coverage; the explicit
        // lightweight variant is reserved for definition-independent tests.
        app.start_sandbox_scenario_with_definitions(
            FrontendScenario::fallback(),
            definition_load,
        )
        .expect("start sandbox scenario");
        wait_for_running(&mut app);
        // Most running-input tests begin after native's one-time centered
        // C4MouseControl move. Tests for that initialization explicitly clear
        // this latch before sending their first platform event.
        app.ingame_mouse_init_centered = true;
        // The lightweight fallback spawns its crew outside CreateInfoObject,
        // which normally installs C4FOW_Def_View_RangeX during player join.
        // Keep this ubiquitous fixture at the same native mouse/FoW invariant.
        if let Some(cursor) = app.engine.crew_cursor(app.local_owner) {
            let mut update = ObjectUpdate::new();
            update.plr_view_range = Some(500);
            app.engine
                .apply_object_update(cursor, update)
                .expect("install sandbox cursor view range");
            app.snapshot = app.engine.snapshot();
        }
        hold_message_board_for_frame_comparison(&mut app);
        app
    }

    fn set_test_scenario_head_flags(app: &mut GameApp, replay: i32, film: i32) {
        let mut state = app.engine.capture_state();
        let values = state
            .scenario_values
            .as_mut()
            .expect("captured state retains Game.C4S values");
        let mut encoded = serde_json::to_value(&*values).expect("serialize Game.C4S values");
        let head = encoded
            .get_mut("sections")
            .and_then(serde_json::Value::as_array_mut)
            .and_then(|sections| {
                sections.iter_mut().find(|section| {
                    section.get("name").and_then(serde_json::Value::as_str) == Some("Head")
                })
            })
            .expect("Game.C4S contains Head");
        let entries = head
            .get_mut("entries")
            .and_then(serde_json::Value::as_array_mut)
            .expect("Head contains compiler entries");
        for (name, value) in [("Replay", replay), ("Film", film)] {
            let entry = entries
                .iter_mut()
                .find(|entry| {
                    entry.get("name").and_then(serde_json::Value::as_str) == Some(name)
                })
                .unwrap_or_else(|| panic!("Head contains {name}"));
            entry["values"] = serde_json::json!([{ "Int": value }]);
        }
        *values = serde_json::from_value(encoded).expect("deserialize adjusted Game.C4S values");
        app.engine
            .restore_state(&state)
            .expect("restore adjusted Game.C4S values");
        app.engine.set_film_viewport_available(true);
        app.snapshot = app.engine.snapshot();
        assert_eq!(app.engine.replay(), replay != 0);
        assert_eq!(app.engine.film(), film != 0);
    }

    fn install_test_recording_template(app: &mut GameApp, output_path: PathBuf) {
        let mut group = MutableGroup::new("Recorded.c4s");
        group
            .add_file(
                "Scenario.txt",
                b"[Head]\nTitle=Recorded\nReplay=1\nIcon=29\n".to_vec(),
            )
            .expect("add replay scenario core");
        group
            .add_file("Sentinel.txt", b"preserved".to_vec())
            .expect("add copied scenario component");
        app.recording_template = Some(RecordingTemplate {
            group,
            output_path,
            initial_stream_chunk: Vec::new(),
            runtime_seed: None,
            description_title: b"Recorded".to_vec(),
            description_definition_modules: Vec::new(),
        });
    }

    fn recorded_right_control(player: i32) -> clonk_engine::ControlPacket {
        clonk_engine::ControlPacket::PlayerControl(clonk_engine::PlayerControlData {
            player,
            command: i32::from(clonk_engine::COM_RIGHT),
            data: 0,
            by_client: 0,
        })
    }

    fn install_running_network_stub(
        app: &mut GameApp,
        local_client_id: clonk_network::ClientId,
        start_tick: i32,
        control_rate: i32,
    ) -> (mpsc::Sender<NetworkEvent>, network::TestNetworkCommands) {
        let (manager, events, commands) =
            NetworkManager::test_stub_with_commands_for_client_id(local_client_id);
        app.network_mode = Some(if local_client_id == 0 {
            NetworkMode::Host(HostSettings {
                bind_addr: SocketAddr::from(([127, 0, 0, 1], 0)),
                player_name: "Host".to_string(),
                prepared: None,
            })
        } else {
            NetworkMode::Client(ClientSettings::new(
                SocketAddr::from(([127, 0, 0, 1], 11_112)),
                "Client",
            ))
        });
        app.network = Some(manager);
        app.network_control_clock = Some(NetworkControlClock::new(start_tick, control_rate));
        app.engine.initialize_network_control_timing(
            clonk_engine::NetworkControlTiming::new(start_tick, control_rate)
                .expect("valid runtime network timing"),
        );
        app.network_control_running = true;
        app.runtime_network_status_barrier = None;
        (events, commands)
    }

    fn queue_empty_ready_tick(app: &GameApp, events: &mpsc::Sender<NetworkEvent>) {
        events
            .send(NetworkEvent::ReadyTick {
                tick: app.expected_network_control_tick(),
                controls: Vec::new(),
            })
            .expect("queue complete empty control tick");
    }

    fn message_control(
        message_type: u8,
        player: i32,
        to_player: i32,
        message: &[u8],
        by_client: i32,
    ) -> MessageControlData {
        MessageControlData {
            message_type,
            player,
            to_player,
            message: clonk_engine::LegacyCString::from_bytes(message.to_vec())
                .expect("message fixture is NUL-free"),
            by_client,
        }
    }

    fn message_client(client_id: i32, nick: &[u8]) -> clonk_engine::ClientCoreControlData {
        clonk_engine::ClientCoreControlData {
            client_id,
            activated: true,
            observer: false,
            name: clonk_engine::LegacyCString::from_bytes(nick.to_vec())
                .expect("client name fixture is NUL-free"),
            nick: clonk_engine::LegacyCString::from_bytes(nick.to_vec())
                .expect("client nick fixture is NUL-free"),
            lobby_ready: false,
        }
    }

    fn message_board_logical_entries(app: &GameApp) -> Vec<String> {
        let mut entries: Vec<String> = Vec::new();
        for physical_line in &app.message_board.log_history {
            if let Some(continuation) = physical_line.strip_prefix("  ") {
                if let Some(entry) = entries.last_mut() {
                    entry.push(' ');
                    entry.push_str(continuation);
                } else {
                    entries.push(continuation.to_string());
                }
            } else {
                entries.push(physical_line.clone());
            }
        }
        entries
    }

    fn latest_message_board_logical_entry(app: &GameApp) -> Option<String> {
        message_board_logical_entries(app).pop()
    }

    fn install_message_fixture(app: &mut GameApp) {
        app.control_clients.replace_snapshot([
            message_client(0, b"Ali"),
            message_client(7, b"Remote"),
            message_client(8, b"Other"),
        ]);
        app.engine
            .register_player(
                PlayerConfig::new(7, "Sender").with_color(Some(RgbColor::new(0x12, 0x34, 0x56))),
            )
            .expect("message sender registers");
        app.engine
            .player_mut(7)
            .expect("message sender exists")
            .set_at_client(clonk_engine::PlayerAtClient::new(7));
        app.engine.set_local_players([app.local_owner]);
        let line_height = app.graphics.message_board_line_height();
        app.message_board.initialize(true, line_height);
        let _ = app.message_board.advance_frame(line_height, false);
    }

    fn add_secondary_local_player_for_mouse_option_test(app: &mut GameApp) -> i32 {
        let primary = app.local_owner;
        let secondary = app.engine.next_player_number();
        app.engine
            .register_player(PlayerConfig::new(secondary, "Secondary"))
            .expect("register secondary local player");
        app.engine.set_local_players([primary, secondary]);
        let assignment = app.local_controls.initialize(LocalControlInit {
            owner: secondary,
            preferred_set: 1,
            prefers_mouse: false,
            gamepads_enabled: true,
            replay: false,
            disable_mouse: false,
        });
        app.engine
            .set_player_runtime_control(secondary, assignment.runtime_control())
            .expect("project secondary local controls into the runtime player");
        secondary
    }

    #[test]
    fn client_lobby_preload_commits_async_and_pending_go_reuses_the_artifact() {
        let directory = tempdir().expect("client preload resource directory");
        let scenario_path = directory.path().join("Scenario.c4s");
        let dynamic_path = directory.path().join("Dynamic.c4s");
        let definitions_path = directory.path().join("Objects.c4d");
        let mut scenario_group = clonk_resources::MutableGroup::new("Scenario.c4s");
        scenario_group
            .add_file(
                "Scenario.txt",
                b"[Head]\nTitle=Preloaded client\nNetworkGame=1\nNoInitialize=1\n\n[Definitions]\nDefinition1=MissingLocal.c4d\n".to_vec(),
            )
            .expect("add client scenario core");
        fs::write(
            &scenario_path,
            scenario_group.pack().expect("pack client scenario"),
        )
        .expect("write client scenario");
        let mut dynamic_group = clonk_resources::MutableGroup::new("Dynamic.c4s");
        dynamic_group
            .add_file("Dynamic.txt", b"preloaded".to_vec())
            .expect("add dynamic marker");
        fs::write(
            &dynamic_path,
            dynamic_group.pack().expect("pack client dynamic data"),
        )
        .expect("write client dynamic data");
        let mut definitions = clonk_resources::MutableGroup::new("Objects.c4d");
        let mut definition = clonk_resources::MutableGroup::new("Host.c4d");
        definition
            .add_file(
                "DefCore.txt",
                b"[DefCore]\nid=HOST\nName=Host\nCategory=1\n".to_vec(),
            )
            .expect("add synchronized definition core");
        definition
            .add_file(
                "Graphics.png",
                include_bytes!("../../../../content/Material.c4g/Snow.png").to_vec(),
            )
            .expect("add synchronized definition graphics");
        definitions
            .add_child("Host.c4d", definition)
            .expect("add synchronized definition");
        fs::write(
            &definitions_path,
            definitions.pack().expect("pack synchronized definitions"),
        )
        .expect("write synchronized definitions");

        let mut app = new_menu_app(320, 200);
        let (manager, event_tx, mut commands) =
            NetworkManager::test_stub_with_commands_for_client_id(7);
        app.network = Some(manager);
        let mut settings =
            ClientSettings::new(SocketAddr::from(([127, 0, 0, 1], 11_112)), "Observer");
        settings.resource_directory = directory.path().to_path_buf();
        app.network_mode = Some(NetworkMode::Client(settings));
        app.network_lobby = Some(
            NetworkLobbyState::new(7, "Observer".to_string(), false)
                .with_preloading(false, LobbyLabels::default()),
        );
        app.startup_view = StartupView::NetworkLobby;
        let resource = |resource_type: clonk_network::HostResourceType, id, name: &[u8]| {
            clonk_engine::NetworkResourceCore {
                resource_type: resource_type as u8,
                id,
                loadable: true,
                filename: clonk_engine::LegacyCString::from_bytes(name.to_vec())
                    .expect("fixture filename is NUL-free"),
                ..Default::default()
            }
        };
        let host_config = clonk_network::HostConfig::default();
        let mut snapshot = host_config
            .initial_join_snapshot
            .expect("default host publishes JoinData");
        snapshot.parameters.random_seed = 41;
        snapshot.parameters.scenario =
            resource(clonk_network::HostResourceType::Scenario, 70, b"Scenario.c4s");
        snapshot.dynamic = resource(clonk_network::HostResourceType::Dynamic, 71, b"Dynamic.c4s");
        snapshot.parameters.game_resources = vec![resource(
            clonk_network::HostResourceType::Definitions,
            72,
            b"Objects.c4d",
        )];
        snapshot
            .parameters
            .clients
            .clients
            .push(clonk_engine::ClientCoreControlData {
                client_id: 7,
                name: clonk_engine::LegacyCString::from_bytes(b"Observer".to_vec()).unwrap(),
                observer: true,
                ..Default::default()
            });
        snapshot.parameters.clients.local_client_id = Some(7);
        let mut reference_status = host_config.initial_status;
        reference_status.target_tick = -1;
        let join_data = clonk_network::JoinDataEnvelope {
            client_id: 7,
            start_control_tick: 23,
            status: reference_status,
            dynamic: snapshot.dynamic.clone(),
            parameters: snapshot.parameters,
        };
        event_tx
            .send(NetworkEvent::JoinData(join_data.clone()))
            .expect("queue preload JoinData");
        app.process_network_events().expect("enter client lobby");
        commands.take_framed_status_acknowledgements();

        for (resource_id, core, path) in [
            (
                70,
                join_data.parameters.scenario.clone(),
                scenario_path,
            ),
            (71, join_data.dynamic.clone(), dynamic_path),
            (
                72,
                join_data.parameters.game_resources[0].clone(),
                definitions_path,
            ),
        ] {
            event_tx
                .send(NetworkEvent::ResourceComplete {
                    resource_id,
                    core,
                    path,
                    local: false,
                })
                .expect("complete preload resource");
        }
        app.process_network_events()
            .expect("make client preload eligible");
        assert!(app
            .network_lobby
            .as_ref()
            .is_some_and(|lobby| lobby.preload.eligible));

        app.request_lobby_preload();
        let go = clonk_network::NetworkStatus {
            state: clonk_network::NETWORK_STATE_GO,
            control_mode: 2,
            target_tick: 23,
        };
        event_tx
            .send(NetworkEvent::StatusRequested(go))
            .expect("queue GO during preload");
        app.process_network_events().expect("defer GO behind preload");
        assert_eq!(app.pending_client_start_status, Some(go));
        assert!(app.loading_state.is_none());
        let (removed_tx, removed_rx) = mpsc::channel();
        let (release_tx, release_rx) = mpsc::channel();
        let removal_observer = thread::spawn(move || {
            let (resource_id, completion) = commands.receive_resource_removal();
            removed_tx
                .send(resource_id)
                .expect("report preload removal");
            let _ = release_rx.recv_timeout(Duration::from_secs(10));
            completion.send(Ok(())).expect("complete preload removal");
            commands
        });

        let deadline = Instant::now() + Duration::from_secs(30);
        while !matches!(
            app.lobby_preload_task.as_ref().map(|task| &task.state),
            Some(LobbyPreloadTaskState::RemovingClientResource { .. })
        ) {
            app.poll_lobby_preload().expect("poll client preload worker");
            assert!(
                app.lobby_preload_task.is_some(),
                "client preload ended before asynchronous removal"
            );
            assert!(Instant::now() < deadline, "client preload did not commit");
            thread::yield_now();
        }
        let combined_path = directory.path().join("Combined7.c4s");
        assert_eq!(
            Group::open(&combined_path)
                .expect("open committed client scenario")
                .read_file("Dynamic.txt")
                .unwrap(),
            b"preloaded"
        );
        assert!(app.client_combined_preload_file.is_owned());
        assert!(app.lobby_preload_artifact.is_none());
        let (expected_hud, expected_textures, expected_render_info) = {
            let task = app.lobby_preload_task.as_ref().unwrap();
            let LobbyPreloadTaskState::RemovingClientResource { artifact, .. } = &task.state
            else {
                unreachable!("client removal is pending")
            };
            (
                Arc::clone(&artifact.game_graphics.hud_graphics),
                Arc::clone(&artifact.material_texture_images),
                Arc::clone(&artifact.material_render_info),
            )
        };
        assert_eq!(
            removed_rx
                .recv_timeout(Duration::from_secs(1))
                .expect("preload removal command"),
            71
        );
        app.poll_lobby_preload()
            .expect("resource removal remains asynchronous");
        assert!(app.lobby_preload_task.is_some());
        assert!(app.loading_state.is_none());

        release_tx.send(()).expect("release dynamic removal");
        let mut commands = removal_observer.join().expect("removal observer exits");
        while app.lobby_preload_task.is_some() {
            app.poll_lobby_preload()
                .expect("install completed client preload");
            assert!(Instant::now() < deadline, "client preload did not install");
            thread::yield_now();
        }
        assert!(app.loading_state.is_some(), "pending GO resumes immediately");
        assert!(app
            .lobby_preload_artifact
            .as_ref()
            .and_then(|artifact| artifact.client.as_ref())
            .is_some_and(|client| client.scenario.is_none()));

        app.poll_loading()
            .expect("activate the preloaded client scenario");
        assert!(Arc::ptr_eq(
            &expected_hud,
            &app.active_game_graphics
                .as_ref()
                .expect("active client graphics")
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
        assert_eq!(commands.take_framed_status_acknowledgements(), vec![(go, 0)]);

        app.clear_lobby_preload();
        assert!(!combined_path.exists());
    }

    #[test]
    fn l021_client_go_combines_scenario_once_and_defers_100_until_final_init() {
        // RetrieveScenario waits for Parameters.Scenario and ResDynamic, merges
        // them into Combined<client>.c4s, then waits for ordinary GameRes files.
        // It does not acknowledge GO until InitGame reaches FinalInit
        // (pristine 9ffa0a5d src/C4Network2.cpp:619-671;
        // src/C4Game.cpp:2526-2556,455-482).
        let directory = tempdir().expect("network resource directory");
        let scenario_path = directory.path().join("Scenario.c4s");
        let dynamic_path = directory.path().join("Dynamic.c4s");
        let game_resource_path = directory.path().join("Objects.c4d");
        let material_resource_path = directory.path().join("HostMaterials.c4g");
        let local_material_fallback_path = directory.path().join("Material.c4g");
        let mut scenario_group = clonk_resources::MutableGroup::new("Scenario.c4s");
        scenario_group
            .add_file(
                "Scenario.txt",
                b"[Head]\nTitle=Client start\nNetworkGame=1\nNoInitialize=1\n\n[Definitions]\nDefinition1=MissingLocal.c4d\n"
                    .to_vec(),
            )
            .expect("add scenario core");
        fs::write(
            &scenario_path,
            scenario_group.pack().expect("pack scenario"),
        )
        .expect("write scenario resource");
        let mut dynamic_group = clonk_resources::MutableGroup::new("Dynamic.c4s");
        dynamic_group
            .add_file("Dynamic.txt", b"merged".to_vec())
            .expect("add dynamic marker");
        fs::write(&dynamic_path, dynamic_group.pack().expect("pack dynamic"))
            .expect("write dynamic resource");
        let mut game_resource = clonk_resources::MutableGroup::new("Objects.c4d");
        let mut host_definition = clonk_resources::MutableGroup::new("Host.c4d");
        host_definition
            .add_file(
                "DefCore.txt",
                b"[DefCore]\nid=HOST\nName=Host\nCategory=1\n".to_vec(),
            )
            .expect("add host definition core");
        host_definition
            .add_file(
                "Graphics.png",
                include_bytes!("../../../../content/Material.c4g/Snow.png").to_vec(),
            )
            .expect("add host definition graphics");
        game_resource
            .add_child("Host.c4d", host_definition)
            .expect("add host definition");
        fs::write(
            &game_resource_path,
            game_resource.pack().expect("pack game resource"),
        )
        .expect("write game resource");
        let mut host_materials = clonk_resources::MutableGroup::new("HostMaterials.c4g");
        host_materials
            .add_file(
                "TexMap.txt",
                b"OverloadMaterials\nOverloadTextures\n1=NetworkOnly-HostTexture\n".to_vec(),
            )
            .expect("add host texture map");
        host_materials
            .add_file(
                "NetworkOnly.c4m",
                b"[Material]\nName=NetworkOnly\nColorX=11,12,13,14,15,16,17,18,19\nDensity=50\nTextureOverlay=HostTexture\n"
                    .to_vec(),
            )
            .expect("add host material");
        host_materials
            .add_file(
                "HostTexture.png",
                include_bytes!("../../../../content/Material.c4g/Snow.png").to_vec(),
            )
            .expect("add host texture");
        fs::write(
            &material_resource_path,
            host_materials.pack().expect("pack host materials"),
        )
        .expect("write host material resource");
        let mut local_material_fallback = clonk_resources::MutableGroup::new("Material.c4g");
        local_material_fallback
            .add_file("TexMap.txt", b"1=NetworkOnly-FallbackTexture\n".to_vec())
            .expect("add fallback texture map");
        local_material_fallback
            .add_file(
                "NetworkOnly.c4m",
                b"[Material]\nName=NetworkOnly\nColorX=201,202,203\nDensity=100\nTextureOverlay=FallbackTexture\n"
                    .to_vec(),
            )
            .expect("add conflicting fallback material");
        local_material_fallback
            .add_file(
                "FallbackOnly.c4m",
                b"[Material]\nName=FallbackOnly\nDensity=100\n".to_vec(),
            )
            .expect("add fallback-only material");
        local_material_fallback
            .add_file(
                "FallbackTexture.png",
                include_bytes!("../../../../content/Material.c4g/Snow.png").to_vec(),
            )
            .expect("add fallback texture");
        fs::write(
            &local_material_fallback_path,
            local_material_fallback
                .pack()
                .expect("pack fallback materials"),
        )
        .expect("write local material fallback");

        let mut app = new_menu_app(320, 200);
        app.loader_screen = Some(
            LoaderScreen::new(
                LoaderSelection::startup("Loader.png").expect("synthetic loader selection"),
                ImageData::new(1, 1, vec![0, 0, 0, 0xff]),
                app.assets
                    .loader_resources()
                    .expect("synthetic loader resources"),
                LoaderState::initial("Client start"),
            )
            .expect("synthetic client loader"),
        );
        let (manager, event_tx, mut commands) =
            NetworkManager::test_stub_with_commands_for_client_id(7);
        app.network = Some(manager);
        let mut settings =
            ClientSettings::new(SocketAddr::from(([127, 0, 0, 1], 11_112)), "Observer");
        settings.resource_directory = directory.path().to_path_buf();
        app.network_mode = Some(NetworkMode::Client(settings));
        let resource = |id, name: &[u8]| clonk_engine::NetworkResourceCore {
            id,
            loadable: true,
            filename: clonk_engine::LegacyCString::from_bytes(name.to_vec())
                .expect("fixture filename is NUL-free"),
            ..Default::default()
        };
        let host_config = clonk_network::HostConfig::default();
        let mut snapshot = host_config
            .initial_join_snapshot
            .expect("default host publishes JoinData");
        let network_random_seed = 7_i32;
        snapshot.parameters.random_seed = network_random_seed;
        snapshot.parameters.control_rate = 2;
        snapshot.parameters.scenario = resource(70, b"Scenario.c4s");
        snapshot.dynamic = resource(71, b"Dynamic.c4s");
        let mut definitions = resource(72, b"Objects.c4d");
        definitions.resource_type = clonk_network::HostResourceType::Definitions as u8;
        let mut materials = resource(73, b"HostMaterials.c4g");
        materials.resource_type = clonk_network::HostResourceType::Material as u8;
        snapshot.parameters.game_resources = vec![definitions, materials];
        let mut reference_status = host_config.initial_status;
        reference_status.target_tick = -1;
        let join_data = clonk_network::JoinDataEnvelope {
            client_id: 7,
            start_control_tick: 23,
            status: reference_status,
            dynamic: snapshot.dynamic.clone(),
            parameters: snapshot.parameters,
        };
        let go = clonk_network::NetworkStatus {
            state: clonk_network::NETWORK_STATE_GO,
            control_mode: 2,
            target_tick: 23,
        };
        event_tx
            .send(NetworkEvent::JoinData(join_data.clone()))
            .expect("queue JoinData");
        event_tx
            .send(NetworkEvent::StatusRequested(go))
            .expect("queue GO request");
        app.process_network_events().expect("apply start request");
        assert_eq!(
            app.blocking_resource_wait
                .as_ref()
                .map(|wait| (wait.scope, wait.resource_id)),
            Some((BlockingResourceScope::ClientStart, 70))
        );
        assert_eq!(
            app.message_dialogs
                .iter()
                .find(|dialog| matches!(
                    dialog.continuation,
                    MessageDialogContinuation::BlockingResourceWait { .. }
                ))
                .and_then(|dialog| dialog.state.progress()),
            Some(0)
        );

        let combined_path = directory.path().join("Combined7.c4s");
        assert!(!combined_path.exists());
        event_tx
            .send(NetworkEvent::ResourceComplete {
                resource_id: 70,
                core: join_data.parameters.scenario.clone(),
                path: scenario_path,
                local: false,
            })
            .expect("complete scenario");
        app.process_network_events().expect("wait for dynamic");
        assert!(!combined_path.exists());
        assert_eq!(
            app.blocking_resource_wait
                .as_ref()
                .map(|wait| wait.resource_id),
            Some(71)
        );
        assert_eq!(commands.take_player_info_updates().len(), 1);
        let (removed_tx, removed_rx) = mpsc::channel();
        let removal_observer = thread::spawn(move || {
            let (resource_id, completion) = commands.receive_resource_removal();
            completion.send(Ok(())).expect("complete dynamic removal");
            removed_tx
                .send(resource_id)
                .expect("report removed dynamic");
            commands
        });
        event_tx
            .send(NetworkEvent::ResourceComplete {
                resource_id: 71,
                core: join_data.dynamic.clone(),
                path: dynamic_path.clone(),
                local: false,
            })
            .expect("complete dynamic");
        app.process_network_events().expect("compose resources");
        assert_eq!(
            removed_rx
                .recv_timeout(Duration::from_secs(1))
                .expect("client did not retire its merged dynamic resource"),
            71
        );
        let mut commands = removal_observer.join().expect("removal observer exits");
        assert_eq!(
            app.blocking_resource_wait
                .as_ref()
                .map(|wait| wait.resource_id),
            Some(72)
        );

        let combined = Group::open(&combined_path).expect("open combined scenario");
        assert_eq!(combined.read_file("Dynamic.txt").unwrap(), b"merged");
        assert!(commands.take_status_acknowledgements().is_empty());

        event_tx
            .send(NetworkEvent::ResourceComplete {
                resource_id: 71,
                core: join_data.dynamic,
                path: dynamic_path,
                local: false,
            })
            .expect("repeat dynamic completion");
        event_tx
            .send(NetworkEvent::StatusRequested(go))
            .expect("repeat GO request");
        app.process_network_events().expect("ignore duplicate work");
        let combined_files = fs::read_dir(directory.path())
            .expect("read resource directory")
            .filter_map(Result::ok)
            .filter(|entry| entry.file_name().to_string_lossy().starts_with("Combined7"))
            .count();
        assert_eq!(combined_files, 1);

        event_tx
            .send(NetworkEvent::ResourceComplete {
                resource_id: 72,
                core: join_data.parameters.game_resources[0].clone(),
                path: game_resource_path,
                local: false,
            })
            .expect("complete ordinary game resource");
        event_tx
            .send(NetworkEvent::ResourceComplete {
                resource_id: 73,
                core: join_data.parameters.game_resources[1].clone(),
                path: material_resource_path,
                local: false,
            })
            .expect("complete authoritative material resource");
        app.process_network_events()
            .expect("begin client InitGame after all resources complete");
        assert!(app.blocking_resource_wait.is_none());
        assert!(!app.message_dialogs.iter().any(|dialog| matches!(
            dialog.continuation,
            MessageDialogContinuation::BlockingResourceWait { .. }
        )));
        assert!(matches!(app.mode, AppMode::Loading));
        app.poll_loading().expect("finish client InitGame phase");
        assert!(matches!(app.mode, AppMode::Loading));
        assert!(
            app.loader_screen
                .as_ref()
                .expect("client loader retained through GO wait")
                .state()
                .progress()
                < 100
        );
        assert!(app.network_start_wait.is_none());
        let client_wait = app
            .message_dialogs
            .iter()
            .find(|dialog| {
                matches!(
                    dialog.continuation,
                    MessageDialogContinuation::NetworkClientStartWait
                )
            })
            .expect("FinalInit presents the client start-wait message");
        assert_eq!(client_wait.state.message(), "Waiting for start...");
        assert_eq!(client_wait.state.caption(), "Network");
        assert_eq!(
            client_wait.state.buttons(),
            clonk_frontend::message_dialog::MessageDialogButtons::CANCEL
        );
        assert_eq!(client_wait.state.focused_button(), None);
        assert!(app.engine.snapshot().players.is_empty());
        assert!(app.engine.definition_ids().any(|id| id == "HOST"));
        // InitClient copies JoinData's start tick and control rate before
        // InitGame, so the engine-side SyncCheck clock must agree with the
        // network gate immediately after loading (src/C4Network2.cpp:1607-1609;
        // src/C4GameControl.cpp:61-68).
        assert_eq!(
            (
                app.engine.sync_check(7).control_tick,
                app.engine.control_rate
            ),
            (23, 2)
        );
        // C4Game opens the combined scenario's Material.c4g first and then the
        // host-ordered NRT_Material files. A client never re-resolves those
        // external files from its local installation, even when the last host
        // file requests both overload chains to continue (pristine 9ffa0a5d
        // src/C4Game.cpp:882-952; src/C4GameParameters.cpp:73-80,255-270).
        assert_eq!(
            app.material_render_info.get("networkonly"),
            Some(&clonk_frontend::MaterialRenderInfo::new(
                [11, 12, 13, 14, 15, 16, 17, 18, 19],
                [0; 6],
                Some("HostTexture".to_string()),
                0,
                50,
            )
            .with_placement(70)),
        );
        assert!(app.material_texture_images.contains_key("hosttexture"));
        assert!(!app.material_render_info.contains_key("fallbackonly"));
        assert!(!app.material_texture_images.contains_key("fallbacktexture"));
        assert_eq!(
            app.engine.random_seed(),
            u64::from(network_random_seed as u32),
            "JoinData remains authoritative over offline seed selection",
        );
        // C4Game::InitGameSecondPart fixes the synchronized RNG from
        // Parameters.RandomSeed before the landscape/weather initialization
        // draws (pristine 9ffa0a5d src/C4Game.cpp:2617-2632;
        // src/C4GameParameters.h:132). This fixture uses the stock C4SVal
        // ranges: Gravity, Season, YearSpeed, Climate, then Wind.
        let mut expected_rng =
            clonk_engine::LcgRng::seed_from_u64(u64::from(network_random_seed as u32));
        for range in [1, 101, 1, 21] {
            expected_rng.random(range);
        }
        let expected_wind = expected_rng.random(141) - 70;
        assert_eq!(
            app.engine.environment().wind,
            expected_wind,
            "client InitGame must use JoinData Parameters.RandomSeed before Weather.Init"
        );
        assert_eq!(
            commands.take_framed_status_acknowledgements(),
            vec![(go, 0)]
        );

        let host_commit = clonk_network::NetworkStatus {
            control_mode: 9,
            ..go
        };
        event_tx
            .send(NetworkEvent::StatusCommitted(host_commit))
            .expect("commit matching state and target");
        app.process_network_events()
            .expect("complete client Network.FinalInit");
        assert!(matches!(app.mode, AppMode::Running));
        assert_eq!(
            app.loader_screen
                .as_ref()
                .expect("client loader retained after final init")
                .state()
                .progress(),
            100
        );
        assert!(app.loading_state.is_none());
        assert!(app.network_control_running);
        assert!(app.message_dialogs.iter().all(|dialog| !matches!(
            dialog.continuation,
            MessageDialogContinuation::NetworkClientStartWait
        )));
    }

    fn set_control_test_team(
        id: i32,
        player_ids: Vec<i32>,
        max_players: i32,
    ) -> clonk_engine::InitialNetworkTeam {
        clonk_engine::InitialNetworkTeam {
            id,
            name: clonk_engine::LegacyCString::from_bytes(format!("Team {id}").into_bytes()).unwrap(),
            player_start_index: 0,
            player_ids,
            color: if id == 1 { 0x00f4_0000 } else { 0x0000_00f4 },
            icon_spec: clonk_engine::LegacyCString::default(),
            max_players,
        }
    }

    fn set_control_test_metadata(
        auto_generate_teams: bool,
        teams: Vec<clonk_engine::InitialNetworkTeam>,
    ) -> clonk_engine::InitialNetworkTeamMetadata {
        clonk_engine::InitialNetworkTeamMetadata {
            active: true,
            custom: false,
            allow_hostility_change: false,
            allow_team_switch: false,
            auto_generate_teams,
            last_team_id: teams.iter().map(|team| team.id).fold(0, i32::max),
            team_distribution: clonk_engine::InitialNetworkTeamDistribution::Free,
            team_colors: false,
            max_script_players: 0,
            script_player_names: clonk_engine::LegacyCString::default(),
            random_team_count: 0,
            teams,
        }
    }

    fn set_control_test_player(
        id: i32,
        team: i32,
        flags: u16,
    ) -> clonk_engine::ControlPlayerInfoEntry {
        clonk_engine::ControlPlayerInfoEntry {
            id,
            team,
            flags,
            name: clonk_engine::LegacyCString::from_bytes(format!("Player {id}").into_bytes())
                .unwrap(),
            ..Default::default()
        }
    }

    #[test]
    fn l052_ready_tick_local_join_opens_one_viewport_with_feedback() {
        // C4Control executes the complete list in packet order, so PlrInfo is
        // visible to the following JoinPlr; only then does C4Game advance the
        // simulation (src/C4Control.cpp:93-109; src/C4Game.cpp:797-805).
        let mut app = new_lightweight_running_sandbox_app();
        let (manager, event_tx) = NetworkManager::test_stub();
        app.network = Some(manager);
        app.engine.set_network_game(true);
        let tick = u32::try_from(app.engine.frame()).expect("test tick fits u32");
        let initial_frame = app.engine.frame();
        let info = clonk_engine::ControlPlayerInfoEntry {
            name: clonk_engine::LegacyCString::from_bytes(b"Network Tyler".to_vec())
                .expect("valid legacy name"),
            id: 7,
            color: 0x0011_2233,
            ..Default::default()
        };
        let join = clonk_engine::JoinPlayerControlData {
            filename: clonk_engine::LegacyCString::from_bytes(
                b"/definitely/missing/RemotePlayer.c4p".to_vec(),
            )
            .expect("valid legacy filename"),
            at_client: 0,
            info_id: 7,
            source: clonk_engine::JoinPlayerSource::Embedded(
                include_bytes!("../../clonk-engine/tests/fixtures/embedded_player.c4p").to_vec(),
            ),
            by_client: 1,
        };
        app.set_runtime_flash_message("Join clears me", RuntimeHelpCharset::Windows1252)
            .expect("install pre-viewport flash");
        assert!(app.runtime_flash_message.is_some());
        app.ui_sound_log.clear();
        event_tx
            .send(NetworkEvent::ReadyTick {
                tick,
                controls: vec![
                    NetworkControl::PlayerInfo(clonk_engine::PlayerInfoControlData {
                        client_id: 0,
                        players: vec![info],
                        by_client: 1,
                        ..Default::default()
                    }),
                    NetworkControl::JoinPlayer(join),
                ],
            })
            .expect("queue admission controls");

        app.update().expect("execute admission tick");

        assert_eq!(app.engine.frame(), initial_frame + 1);
        let joined = app
            .snapshot
            .players
            .iter()
            .find(|player| player.player_info_id == 7)
            .expect("embedded player joined before the simulation tick");
        assert_eq!(joined.name, "Network Tyler");
        assert_eq!((joined.score, joined.total_playing_time), (42, 99));
        // AtClient, independently of the remote ByClient source selection,
        // makes this user player local (src/C4Player.cpp:1871-1877).
        assert!(
            app.snapshot.hud.local_players.contains(&joined.id),
            "a user player targeted at the local client is locally controlled"
        );
        assert!(
            app.runtime_flash_message.is_none(),
            "owned C4Viewport::Init clears the process-global flash"
        );
        assert_eq!(
            app.ui_sound_log
                .iter()
                .filter(|sound| sound.as_str() == "CloseViewport")
                .count(),
            1,
            "one successful local join creates one non-silent viewport"
        );
    }

    #[test]
    fn l052_synchronized_remote_join_has_no_local_viewport_feedback() {
        // C4ControlJoinPlayer passes iAtClient through C4Game::JoinPlayer and
        // C4PlayerList::Join; C4Player::Init then stores it in AtClient
        // (pristine 9ffa0a5d src/C4Control.cpp:710-764;
        // src/C4Game.cpp:3505-3514; src/C4PlayerList.cpp:271-317;
        // src/C4Player.cpp:246-265).
        let mut app = new_lightweight_running_sandbox_app();
        let (manager, event_tx) = NetworkManager::test_stub();
        app.network = Some(manager);
        app.engine.set_network_game(true);
        let tick = u32::try_from(app.engine.frame()).expect("test tick fits u32");
        let info_id = 73;
        let at_client = 3;
        app.ui_sound_log.clear();
        app.control_clients.replace_snapshot([
            clonk_engine::ClientCoreControlData {
                client_id: 0,
                name: clonk_engine::LegacyCString::from_bytes(b"Host Client".to_vec())
                    .expect("valid host client name"),
                ..Default::default()
            },
            clonk_engine::ClientCoreControlData {
                client_id: at_client,
                name: clonk_engine::LegacyCString::from_bytes(b"Remote Andr\xe9".to_vec())
                    .expect("valid remote client name"),
                ..Default::default()
            },
        ]);
        event_tx
            .send(NetworkEvent::ReadyTick {
                tick,
                controls: vec![
                    NetworkControl::PlayerInfo(clonk_engine::PlayerInfoControlData {
                        client_id: at_client,
                        players: vec![clonk_engine::ControlPlayerInfoEntry {
                            name: clonk_engine::LegacyCString::from_bytes(b"Remote Ren\xe9".to_vec())
                                .expect("valid legacy name"),
                            id: info_id,
                            ..Default::default()
                        }],
                        by_client: 1,
                        ..Default::default()
                    }),
                    NetworkControl::JoinPlayer(clonk_engine::JoinPlayerControlData {
                        filename: clonk_engine::LegacyCString::from_bytes(
                            b"RemotePlayer.c4p".to_vec(),
                        )
                        .expect("valid legacy filename"),
                        at_client,
                        info_id,
                        source: clonk_engine::JoinPlayerSource::Embedded(
                            include_bytes!("../../clonk-engine/tests/fixtures/embedded_player.c4p")
                                .to_vec(),
                        ),
                        by_client: 1,
                    }),
                ],
            })
            .expect("queue remote admission controls");

        app.update().expect("execute remote admission tick");

        let joined = app
            .snapshot
            .players
            .iter()
            .find(|player| player.player_info_id == info_id)
            .expect("remote player joined");
        assert_eq!(
            app.engine
                .player(joined.id)
                .expect("runtime remote player")
                .at_client(),
            clonk_engine::PlayerAtClient::new(at_client)
        );
        let player = app.engine.player(joined.id).expect("runtime remote player");
        assert_eq!(
            clonk_script::c4_string_bytes(player.at_client_name()),
            b"Remote Andr\xe9"
        );
        assert_eq!(clonk_script::c4_string_bytes(player.name()), b"Remote Ren\xe9");
        assert!(
            app.ui_sound_log
                .iter()
                .all(|sound| sound != "CloseViewport"),
            "a remote player never creates a viewport on this process"
        );
    }

    #[test]
    fn synchronized_join_for_a_missing_client_is_ignored() {
        // C4ControlJoinPlayer resolves AtClient before joining and returns
        // immediately when that client has already disappeared
        // (C4Control.cpp:714-716).
        let mut app = new_state_only_running_sandbox_app();
        let (manager, _event_tx) = NetworkManager::test_stub();
        app.network = Some(manager);
        app.engine.set_network_game(true);
        let info_id = 74;
        app.control_player_infos
            .apply(clonk_engine::PlayerInfoControlData {
                client_id: 9,
                players: vec![clonk_engine::ControlPlayerInfoEntry {
                    name: clonk_engine::LegacyCString::from_bytes(b"Gone Owner".to_vec())
                        .expect("valid player name"),
                    id: info_id,
                    ..Default::default()
                }],
                by_client: 1,
                ..Default::default()
            });

        app.apply_join_player_control(clonk_engine::JoinPlayerControlData {
            filename: clonk_engine::LegacyCString::from_bytes(b"RemotePlayer.c4p".to_vec())
                .expect("valid legacy filename"),
            at_client: 9,
            info_id,
            source: clonk_engine::JoinPlayerSource::Embedded(
                include_bytes!("../../clonk-engine/tests/fixtures/embedded_player.c4p").to_vec(),
            ),
            by_client: 1,
        })
        .expect("missing-client join is ignored");

        assert!(
            app.engine
                .players()
                .all(|player| player.player_info_id() != info_id)
        );
    }

    fn two_item_script_menu(cursor: ObjectId) -> clonk_engine::ObjectMenuState {
        clonk_engine::ObjectMenuState {
            caption: "Choose".to_string(),
            symbol_id: "MENU".to_string(),
            title_symbol: clonk_engine::ObjectMenuSymbol::default(),
            identification: serde_json::from_value(serde_json::json!({ "C4Id": "MENU" }))
                .expect("menu identification deserializes"),
            style: 0,
            equal_item_height: false,
            permanent: false,
            location: None,
            runtime_id: 0,
            extra: clonk_engine::ObjectMenuExtra::default(),
            extra_data: 0,
            internal_refill_token: 0,
            selection: 0,
            user_menu: true,
            command_object: Some(cursor),
            scenario_callbacks: false,
            refill_object: None,
            refill_object_contents_count: 0,
            items: vec![
                clonk_engine::ObjectMenuItem {
                    caption: "First".to_string(),
                    info_caption: "First details".to_string(),
                    command: "0".to_string(),
                    command2: "0".to_string(),
                    count: 12_345_678,
                    item_id: "NONE".to_string(),
                    symbol: clonk_engine::ObjectMenuSymbol::default(),
                    image: clonk_engine::ObjectMenuImage::None,
                    presentation_definition_id: None,
                    picture_snapshot: None,
                    picture_object: None,
                    components: Vec::new(),
                    selectable: true,
                    value: None,
                    text_display_progress: -1,
                },
                clonk_engine::ObjectMenuItem {
                    caption: "Second".to_string(),
                    info_caption: "Second details".to_string(),
                    command: "0".to_string(),
                    command2: "0".to_string(),
                    count: 12_345_678,
                    item_id: "NONE".to_string(),
                    symbol: clonk_engine::ObjectMenuSymbol::default(),
                    image: clonk_engine::ObjectMenuImage::None,
                    presentation_definition_id: None,
                    picture_snapshot: None,
                    picture_object: None,
                    components: Vec::new(),
                    selectable: true,
                    value: None,
                    text_display_progress: -1,
                },
            ],
            columns: 5,
            lines: 0,
            text_progressing: false,
            decoration: None,
        }
    }

    fn long_script_menu(cursor: ObjectId, item_count: usize) -> clonk_engine::ObjectMenuState {
        let mut menu = two_item_script_menu(cursor);
        menu.columns = 1;
        let template = menu.items[0].clone();
        menu.items = (0..item_count)
            .map(|index| clonk_engine::ObjectMenuItem {
                caption: format!("Item {index}"),
                info_caption: format!("Details {index}"),
                ..template.clone()
            })
            .collect();
        menu
    }

    fn install_test_cursor_menu(
        app: &mut GameApp,
        cursor: ObjectId,
        menu: clonk_engine::ObjectMenuState,
    ) {
        app.engine
            .apply_object_update(
                cursor,
                ObjectUpdate {
                    menu: Some(Some(menu)),
                    ..ObjectUpdate::default()
                },
            )
            .expect("install cursor object menu");
    }

    fn construction_drag_fixture() -> (
        GameApp,
        i32,
        GuiPoint,
        GuiPoint,
        GuiPoint,
        Vector2,
        i32,
    ) {
        let mut app = new_classic_running_sandbox_app();
        let owner = app.local_owner;
        let cursor = app.engine.crew_cursor(owner).expect("sandbox cursor");
        app.engine
            .player_mut(owner)
            .expect("sandbox player")
            .set_fog_of_war(false);
        let mut landscape = Landscape::flat(480, 180);
        landscape.set_world_height(220);
        app.engine.set_landscape(landscape);

        let mut site = Definition::from_script("BLD1", "Build site", "#strict\n")
            .expect("constructable definition compiles");
        site.set_category(clonk_engine::CATEGORY_STRUCTURE);
        site.set_constructable(true);
        site.set_shape_rect(Some(clonk_engine::DefinitionRect::new(-4, -8, 8, 8)));
        app.engine
            .register_definition(site)
            .expect("register constructable definition");

        let mut menu = two_item_script_menu(cursor);
        menu.items[0].item_id = "BLD1".to_owned();
        install_test_cursor_menu(&mut app, cursor, menu);
        app.snapshot = app.engine.snapshot();
        let render_snapshot = app.snapshot.clone();
        let viewports = collect_viewport_inputs(&render_snapshot)
            .expect("construction fixture has a local viewport");
        app.graphics.render_frame(&render_snapshot, &viewports);

        let (width, height) = {
            let surface = app.graphics.surface();
            (surface.width() as i32, surface.height() as i32)
        };
        let menu_point = (0..height)
            .flat_map(|y| (0..width).map(move |x| GuiPoint::new(x as f32, y as f32)))
            .find(|point| {
                matches!(
                    app.script_menu_pointer_target(*point),
                    Ok(Some(EngineScriptMenuPointerTarget::Item(0)))
                )
            })
            .expect("construction menu first item has a pointer point");

        let mut valid = None;
        let mut invalid = None;
        let viewport = app.graphics.viewport_rect(owner).expect("local viewport");
        'points: for y in viewport.y..viewport.y + viewport.height as i32 {
            for x in viewport.x..viewport.x + viewport.width as i32 {
                let point = GuiPoint::new(x as f32, y as f32);
                if (point.x - menu_point.x)
                    .abs()
                    .max((point.y - menu_point.y).abs())
                    < MENU_DRAG_THRESHOLD
                    || app
                        .script_menu_pointer_target(point)
                        .expect("hit-test candidate site")
                        .is_some()
                {
                    continue;
                }
                let Some(pointer) = app.graphics.viewport_point_at(point) else {
                    continue;
                };
                if pointer.owner != owner {
                    continue;
                }
                let world = ingame_pointer_world_pixel(pointer);
                let placement_valid = app.engine.construction_site_visible(owner, world)
                    && app.engine.construction_site_valid("BLD1", world);
                if placement_valid && valid.is_none() {
                    valid = Some((point, world));
                } else if !placement_valid && invalid.is_none() {
                    invalid = Some(point);
                }
                if valid.is_some() && invalid.is_some() {
                    break 'points;
                }
            }
        }
        let (valid_point, valid_world) = valid.expect("flat landscape has a valid build site");
        let invalid_point = invalid.expect("viewport also has an unsupported build site");
        let raw_c4id = app
            .engine
            .object_menu_construction_drag(owner, 0)
            .expect("first row is a construction drag")
            .definition_c4id;
        (
            app,
            owner,
            menu_point,
            valid_point,
            invalid_point,
            valid_world,
            raw_c4id,
        )
    }

    fn begin_construction_drag(app: &mut GameApp, menu_point: GuiPoint, drop_point: GuiPoint) {
        app.handle_cursor_moved(PhysicalPosition::new(
            f64::from(menu_point.x),
            f64::from(menu_point.y),
        ))
        .expect("move over constructable menu row");
        app.handle_mouse_button(ElementState::Pressed)
        .expect("press constructable menu row");
        assert!(matches!(
            app.construction_menu_drag.as_ref(),
            Some(ConstructionMenuDrag::Candidate { .. })
        ));
        app.handle_cursor_moved(PhysicalPosition::new(
            f64::from(drop_point.x),
            f64::from(drop_point.y),
        ))
        .expect("cross construction-menu drag sensitivity");
        assert!(app.ingame_construction_drag_active());
    }

    fn new_game_over_keyboard_app() -> GameApp {
        let mut app = new_classic_running_sandbox_app();
        app.handle_game_over().expect("show game-over dialog");
        assert!(app.game_over_dialog.is_some());
        if let Some(audio) = app.audio.as_mut() {
            // Isolate game-over input from InitGameFinal's intentional
            // CloseViewport feedback instance.
            audio.active_channels.clear();
        }
        app.status_text.clear();
        app
    }

    #[derive(Debug, PartialEq, Eq)]
    struct RuntimeGlobalUiSnapshot {
        menu_render_version: u64,
        mode: AppMode,
        startup_view: StartupView,
        exit_requested: bool,
        status_text: String,
        message_dialogs: Vec<(String, String)>,
        game_over_open: bool,
        game_over_hovered_action: Option<GameOverAction>,
        game_over_focus: Option<GameOverFocus>,
        ingame_page: Option<ingame_menu::MenuPage>,
        object_menu_open: bool,
        engine_menu_style: Option<i32>,
        context_menu_open: bool,
        definition_selector_open: bool,
        game_option_input_open: bool,
        save_browser_open: bool,
        game_over_handled: bool,
        runtime_help_visible: bool,
        runtime_flash_message: Option<RuntimeFlashMessage>,
        runtime_client_list_open: bool,
        scoreboard_dialog: Option<ScoreboardPresentationRequest>,
        scoreboard: clonk_engine::ScoreboardState,
        scoreboard_initial_reconcile_pending: bool,
        scoreboard_close_pointer_capture: bool,
        pressed_engine_keys: HashSet<VirtualKeyCode>,
        message_dialog_consumed_keys: HashSet<VirtualKeyCode>,
    }

    fn runtime_global_ui_snapshot(app: &GameApp) -> RuntimeGlobalUiSnapshot {
        RuntimeGlobalUiSnapshot {
            menu_render_version: app.menu_render_version,
            mode: app.mode,
            startup_view: app.startup_view,
            exit_requested: app.exit_requested,
            status_text: app.status_text.clone(),
            message_dialogs: app
                .message_dialogs
                .iter()
                .map(|dialog| {
                    (
                        dialog.state.caption().to_string(),
                        dialog.state.message().to_string(),
                    )
                })
                .collect(),
            game_over_open: app.game_over_dialog.is_some(),
            game_over_hovered_action: app
                .game_over_dialog
                .as_ref()
                .and_then(GameOverState::hovered_action),
            game_over_focus: app
                .game_over_dialog
                .as_ref()
                .and_then(GameOverState::focused),
            ingame_page: app.ingame_menu.as_ref().map(IngameMenuState::page),
            object_menu_open: app.object_menu.is_some(),
            engine_menu_style: app
                .engine
                .cursor_object_menu(app.local_owner)
                .map(|(_, menu)| menu.style),
            context_menu_open: app.context_menu.is_some(),
            definition_selector_open: app.definition_selector.is_some(),
            game_option_input_open: app.game_option_input_dialog.is_some(),
            save_browser_open: app.save_browser.is_some(),
            game_over_handled: app.game_over_handled,
            runtime_help_visible: app.runtime_help_visible,
            runtime_flash_message: app.runtime_flash_message.clone(),
            runtime_client_list_open: app.runtime_client_list.is_some(),
            scoreboard_dialog: app.scoreboard_dialog.clone(),
            scoreboard: app.snapshot.hud.scoreboard.clone(),
            scoreboard_initial_reconcile_pending: app.scoreboard_initial_reconcile_pending,
            scoreboard_close_pointer_capture: app.scoreboard_close_pointer_capture,
            pressed_engine_keys: app.pressed_engine_keys.clone(),
            message_dialog_consumed_keys: app.message_dialog_consumed_keys.clone(),
        }
    }

    fn expect_runtime_global_boundary_unchanged(
        app: &mut GameApp,
        key: VirtualKeyCode,
        expected: ClassicParityBoundary,
    ) {
        let before = runtime_global_ui_snapshot(app);
        let expected = expected.to_string();
        let error = app
            .handle_key(key, ElementState::Pressed)
            .expect_err("unported runtime-global route must fail typed");
        let detail = match error {
            EngineError::ClassicMenuParityBoundary { detail } => detail,
            other => panic!("runtime-global route returned the wrong error: {other}"),
        };
        assert_eq!(detail, expected);
        assert_eq!(
            runtime_global_ui_snapshot(app),
            before,
            "runtime-global boundary must precede all app/UI mutation"
        );
    }

    fn new_scoreboard_test_app(script: &str) -> GameApp {
        configure_scoreboard_test_app(new_running_sandbox_app(), script)
    }

    fn new_classic_scoreboard_test_app(script: &str) -> GameApp {
        configure_scoreboard_test_app(new_classic_running_sandbox_app(), script)
    }

    fn configure_scoreboard_test_app(mut app: GameApp, script: &str) -> GameApp {
        app.reconcile_initial_scoreboard();
        app.status_text.clear();
        app.snapshot.hud.messages.clear();
        if !script.is_empty() {
            app.engine
                .install_scenario_script_with_convention("ScoreboardBoundary", script, true)
                .expect("install scoreboard fixture script");
            app.snapshot = app.engine.snapshot();
            app.snapshot.hud.messages.clear();
        }
        app
    }

    fn toggle_scoreboard(app: &mut GameApp, modifiers: ModifiersState) {
        app.handle_modifiers_changed(modifiers)
            .expect("set keyboard modifiers");
        app.handle_key(VirtualKeyCode::Tab, ElementState::Pressed)
            .expect("toggle the classic scoreboard");
        app.handle_key(VirtualKeyCode::Tab, ElementState::Released)
            .expect("release the scoreboard key");
    }

    fn route_primary_gamepad_to_local_owner(app: &mut GameApp) {
        let mut config = Config::new();
        config.set_in(
            Some("Gamepad0"),
            "Button9",
            input::legacy_gamepad_axis_key(0, 0, true)
                .expect("primary right-axis key")
                .to_string(),
        );
        app.gamepad_bindings = GamepadBindings::from_config(&config);
        app.local_controls.remove(app.local_owner);
        app.local_controls.initialize(LocalControlInit {
            owner: app.local_owner,
            preferred_set: GamepadSlot::new(0).control_set(),
            prefers_mouse: false,
            gamepads_enabled: true,
            replay: false,
            disable_mouse: false,
        });
    }

    fn host_network_settings() -> HostSettings {
        HostSettings {
            bind_addr: SocketAddr::from(([127, 0, 0, 1], 0)),
            player_name: "Host".to_string(),
            prepared: None,
        }
    }

    fn client_network_settings() -> ClientSettings {
        ClientSettings::new(SocketAddr::from(([127, 0, 0, 1], 11112)), "Client")
    }

    fn configure_runtime_network_role(app: &mut GameApp, role: RuntimeNetworkRole) {
        match role {
            RuntimeNetworkRole::Offline => {
                app.network = None;
                // Network absence is authoritative even if stale mode data
                // survives an interrupted teardown.
                app.network_mode = Some(NetworkMode::Client(client_network_settings()));
            }
            RuntimeNetworkRole::Host => {
                let (manager, _events) = NetworkManager::test_stub();
                app.network = Some(manager);
                app.network_mode = Some(NetworkMode::Host(host_network_settings()));
            }
            RuntimeNetworkRole::Client => {
                let (manager, _events) = NetworkManager::test_stub_for_client_id(3);
                app.network = Some(manager);
                app.network_mode = Some(NetworkMode::Client(client_network_settings()));
            }
            RuntimeNetworkRole::Ambiguous => {
                let (manager, _events) = NetworkManager::test_stub_for_client_id(3);
                app.network = Some(manager);
                app.network_mode = Some(NetworkMode::Host(host_network_settings()));
            }
        }
        assert_eq!(app.runtime_network_role(), role);
    }

    fn runtime_flash_text(app: &GameApp) -> Option<&str> {
        app.runtime_flash_message
            .as_ref()
            .map(|message| message.text.as_str())
    }

    #[test]
    fn runtime_f1_help_columns_match_cpp_rows_keys_and_us_labels() {
        let table = parse_runtime_help_language_table(
            include_bytes!(concat!(
                env!("CARGO_MANIFEST_DIR"),
                "/../../../planet/System.c4g/LanguageUS.txt"
            )),
            "LanguageUS.txt",
        )
        .expect("parse shipped language table");
        let columns = build_runtime_help_columns(&table).expect("build shipped help columns");
        let speed_up = if cfg!(target_os = "windows") {
            "Shift+Add"
        } else if cfg!(target_os = "linux") {
            "Shift+KP_Add"
        } else {
            "Shift+Keypad +"
        };

        assert_eq!(columns.left.lines().count(), 17);
        assert_eq!(columns.left.matches("<c ffff00>").count(), 11);
        assert_eq!(columns.right.lines().count(), 12);
        assert_eq!(columns.right.matches("<c ffff00>").count(), 6);
        assert_eq!(
            columns.left,
            "[Game Functions]\n\n<c ffff00>F1</c> - Help\n<c ffff00>F3</c> - Music\n<c ffff00>Ctrl+F3</c> - Sound\n<c ffff00>F4</c> - Network\n\n<c ffff00>F2/Return</c> - Send message\n<c ffff00>Shift+Up</c> - Scroll messages back\n<c ffff00>Shift+Down</c> - Scroll messages forward\n\n<c ffff00>Alt+C</c> - IRC-Chat\n\n<c ffff00>Tab</c> - Scoreboard (if available)\n\n<c ffff00>F9</c> - Screenshot\n<c ffff00>Ctrl+F9</c> - Screenshot (full game area)\n"
        );
        assert_eq!(
            columns.right,
            format!(
                "\n\n<c ffff00>{speed_up}</c> - Increase game speed\n<c ffff00></c> - Decrease game speed\n\n\n[Debug]\n\n<c ffff00>Ctrl+F5</c> - Debug mode\n<c ffff00>Ctrl+F6</c> - Entrance+Vertices\n<c ffff00>Ctrl+F7</c> - Actions/Commands/Pathfinder\n<c ffff00>Ctrl+F8</c> - SolidMasks\n"
            )
        );

        let duplicate = parse_runtime_help_language_table(
            b"IDS_CON_HELP=First\nIDS_CON_HELP=Second\n",
            "duplicate fixture",
        )
        .expect("parse duplicate fixture");
        assert_eq!(
            duplicate.get("IDS_CON_HELP").map(String::as_str),
            Some("First")
        );
    }

    #[test]
    fn default_rank_resource_names_decode_shipped_tables_and_preserve_segments() {
        let us = parse_runtime_language_table(
            include_bytes!(concat!(
                env!("CARGO_MANIFEST_DIR"),
                "/../../../planet/System.c4g/LanguageUS.txt"
            )),
            "LanguageUS.txt",
        )
        .expect("parse shipped US language table");
        let de = parse_runtime_language_table(
            include_bytes!(concat!(
                env!("CARGO_MANIFEST_DIR"),
                "/../../../planet/System.c4g/LanguageDE.txt"
            )),
            "LanguageDE.txt",
        )
        .expect("parse shipped German language table");
        assert_eq!(default_rank_resource_names(&us)[1], "Ensign");
        assert_eq!(default_rank_resource_names(&de)[1], "Fähnrich");

        let segments = RuntimeLanguageTable {
            charset: RuntimeHelpCharset::Utf8,
            entries: HashMap::from([(
                "IDS_GAME_DEFRANKS".to_string(),
                " Recruit|| Captain |".to_string(),
            )]),
        };
        assert_eq!(
            default_rank_resource_names(&segments),
            [" Recruit", "", " Captain ", ""]
        );
    }

    #[test]
    fn presentation_material_and_texture_overload_chains_stop_independently() {
        // InitMaterialTexture carries separate OverloadMaterials and
        // OverloadTextures booleans to the next NRT_Material source
        // (C4Game.cpp:914-977). The local group below admits its parent's
        // material but deliberately stops before Parent.png.
        let _env_lock = crate::tests::env_lock().lock();
        reset_cached_app_paths();
        let root = tempdir().expect("temp material chain");
        let family = root.path().join("Hazard.c4f");
        let scenario = family.join("Tutorial.c4s");
        let local = scenario.join("Material.c4g");
        let parent = family.join("Material.c4g");
        fs::create_dir_all(&local).expect("local material group");
        fs::create_dir_all(&parent).expect("parent material group");
        fs::write(
            local.join("TexMap.txt"),
            "OverloadMaterials\n1=Local-Local\n",
        )
        .expect("local texmap");
        fs::write(local.join("Local.c4m"), "[Material]\nName=Local\n").expect("local material");
        fs::write(
            local.join("Local.png"),
            include_bytes!("../../../../content/Material.c4g/Snow.png"),
        )
        .expect("local texture");
        fs::write(parent.join("TexMap.txt"), "1=Parent-Parent\n").expect("parent texmap");
        fs::write(parent.join("Parent.c4m"), "[Material]\nName=Parent\n").expect("parent material");
        fs::write(
            parent.join("Parent.png"),
            include_bytes!("../../../../content/Material.c4g/Snow.png"),
        )
        .expect("parent texture");

        let plain_ancestor_materials = root.path().join("Material.c4g");
        fs::create_dir_all(&plain_ancestor_materials).expect("plain ancestor material group");
        fs::write(
            plain_ancestor_materials.join("Wrong.c4m"),
            "[Material]\nName=Wrong\n",
        )
        .expect("plain ancestor material");
        let scenario_group = Group::open(&scenario).expect("scenario group");
        let external = InstallDefinitionResolver::new(None)
            .resolve_material_groups(&scenario_group)
            .expect("material parents resolve");
        assert_eq!(external.len(), 1);
        assert_eq!(external[0].root(), parent.as_path());

        let metadata = load_material_render_info(&scenario, None);
        let textures = load_scenario_material_textures(&scenario, None);
        assert!(metadata.contains_key("local"));
        assert!(metadata.contains_key("parent"));
        assert!(textures.contains_key("local"));
        assert!(
            !textures.contains_key("parent"),
            "no OverloadTextures means the parent texture is not admitted",
        );

        reset_cached_app_paths();
    }

    fn cleanup_quicksave_file() {
        let dir = resolve_save_directory();
        let path = dir.join(QUICK_SAVE_FILE);
        if path.exists() {
            let _ = std::fs::remove_file(&path);
        }
        let thumbnail = path.with_extension("png");
        if thumbnail.exists() {
            let _ = std::fs::remove_file(&thumbnail);
        }
    }

    // Area part files spliced into this same `tests` module: each part is a
    // bare item sequence (not a child module), so test ids stay `tests::<fn>`.
    include!("main_tests/scenario_routes.rs");
    include!("main_tests/game_over.rs");
    include!("main_tests/league.rs");
    include!("main_tests/lobby.rs");
    include!("main_tests/net_resources.rs");
    include!("main_tests/netplay.rs");
    include!("main_tests/saves.rs");
    include!("main_tests/scensel.rs");
    include!("main_tests/startup.rs");
    include!("main_tests/menus.rs");
    include!("main_tests/chat_messages.rs");
    include!("main_tests/audio.rs");
    include!("main_tests/rendering.rs");
    include!("main_tests/input.rs");
    include!("main_tests/runtime.rs");
