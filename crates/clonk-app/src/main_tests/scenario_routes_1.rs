// Spliced into `mod tests` (src/main_tests.rs) via include!: a bare item
// sequence, not a child module, so test ids stay `tests::<fn>`.

    #[test]
    fn real_hazard_scenario_gui_sheet_overrides_apply_and_reach_running() {
        let user_data = tempdir().expect("isolated Hazard override user data");
        let (_paths_guard, paths) = exact_loader_test_paths(user_data.path(), None);
        configure_test_startup_participant(&paths, user_data.path());
        let audio_options = AudioOptions {
            sound_enabled: false,
            music_enabled: false,
            menu_music_enabled: false,
            menu_sound_enabled: false,
            ..AudioOptions::default()
        };
        let mut app = GameApp::new(
            320,
            200,
            audio_options,
            Some(&paths),
            RuntimeConfig {
                player_owner: 1,
                player_name: "Hazard GUI parity".to_string(),
                network: None,
                record_enabled: false,
            },
        )
        .expect("initialize Hazard override app");
        wait_for_menu(&mut app);
        let pristine_scroll = app
            .assets
            .startup_dialog_images
            .get("GUIScroll.png")
            .expect("pristine startup scroll sheet")
            .clone();
        let scenario =
            resolve_next_mission_scenario(&app.scenario_catalog, "Hazard.c4f/Tutorial.c4s")
                .expect("Hazard tutorial is present in the real scenario catalog");

        // The user repro: starting any Hazard map used to refuse during
        // loading with a GlobalGuiBootstrapResources boundary because the
        // folder's Graphics.c4g overrides GUICaption/GUIScroll/GUIProgress.
        // C++ instead applies those overrides (C4GraphicsResource::Init →
        // C4GUI::Resource::Load over the registered set).
        app.start_scenario(scenario).expect("start Hazard tutorial");
        wait_for_running_with_attempts(&mut app, 2_400);

        assert!(app.effective_global_gui_failures().is_empty());
        app.assets
            .require_classic_global_gui_bootstrap_resources(&HashMap::new())
            .expect("running Hazard keeps the global GUI bundle boundary-clean");
        for stem in ["GUICaption", "GUIScroll", "GUIProgress"] {
            let source = app
                .assets
                .active_gui_sheet_sources
                .get(stem)
                .unwrap_or_else(|| panic!("{stem} must be rebound while Hazard runs"));
            assert!(
                source.contains("Hazard.c4f") && source.contains("Graphics.c4g"),
                "{stem} must be won by the Hazard folder pack: {source}"
            );
        }
        let running_scroll = app
            .assets
            .startup_dialog_images
            .get("GUIScroll.png")
            .expect("running scroll sheet")
            .clone();
        assert_ne!(
            running_scroll.pixels(),
            pristine_scroll.pixels(),
            "the Hazard scroll sheet must replace the global surface"
        );
        assert!(
            app.assets.message_dialog_resources().is_some(),
            "running dialogs resolve from the rebound sheets"
        );

        app.return_to_menu();
        assert!(app.assets.active_gui_sheet_sources.is_empty());
        assert_eq!(
            app.assets
                .startup_dialog_images
                .get("GUIScroll.png")
                .expect("restored scroll sheet")
                .pixels()
                .as_ptr(),
            pristine_scroll.pixels().as_ptr(),
            "teardown must restore the pristine startup scroll sheet"
        );
    }

    #[test]
    fn real_alchemy_mouse_subcases_batch_1() {
        let prepared =
            PreparedRealInstalledScenario::new("Fantasy.c4f/Alchemy.c4s");
        let mut failures = Vec::new();
        run_real_alchemy_app_subcase(
            "right_click_positions_classic_context_magic_menu",
            &mut failures,
            || l068_real_alchemy_right_click_positions_classic_context_magic_menu(&prepared),
        );
        run_real_alchemy_app_subcase(
            "right_drag_frame_drops_all_selected_carryables",
            &mut failures,
            || real_alchemy_right_drag_frame_drops_all_selected_carryables(&prepared),
        );
        assert_no_real_alchemy_app_subcase_failures(failures);
    }

    #[test]
    fn real_alchemy_mouse_subcases_batch_2() {
        let prepared =
            PreparedRealInstalledScenario::new("Fantasy.c4f/Alchemy.c4s");
        let mut failures = Vec::new();
        run_real_alchemy_app_subcase(
            "control_right_drag_puts_carryable_into_hut",
            &mut failures,
            || real_alchemy_control_right_drag_puts_carryable_into_hut(&prepared),
        );
        run_real_alchemy_app_subcase(
            "right_drag_rectangle_replaces_crew_selection",
            &mut failures,
            || real_alchemy_right_drag_rectangle_replaces_crew_selection(&prepared),
        );
        run_real_alchemy_app_subcase(
            "left_double_click_gets_carryable_like_cpp_mouse_control",
            &mut failures,
            || real_alchemy_left_double_click_gets_carryable_like_cpp_mouse_control(&prepared),
        );
        assert_no_real_alchemy_app_subcase_failures(failures);
    }

    fn run_real_alchemy_app_subcase(
        name: &'static str,
        failures: &mut Vec<&'static str>,
        subcase: impl FnOnce(),
    ) {
        eprintln!("running Alchemy app subcase `{name}`");
        if std::panic::catch_unwind(std::panic::AssertUnwindSafe(subcase)).is_err() {
            eprintln!("Alchemy app subcase `{name}` failed; continuing batch");
            failures.push(name);
        }
    }

    fn assert_no_real_alchemy_app_subcase_failures(failures: Vec<&str>) {
        assert!(
            failures.is_empty(),
            "Alchemy app subcase(s) failed: {}",
            failures.join(", ")
        );
    }

    fn l068_real_alchemy_right_click_positions_classic_context_magic_menu(
        prepared: &PreparedRealInstalledScenario,
    ) {
        // C4MouseControl issues C4CMD_Context on right-up with the clicked
        // MCLK as Target2. The command installs classic style-1 context on
        // the selected mage; entering ContextMagic opens the shipped spell
        // menu (C4MouseControl.cpp:1230-1263; C4Command.cpp:1076-1090;
        // MagiClonk.c4d/Script.c:190-199).
        let mut app = prepared.instantiate("Alchemy mouse context parity", false);
        let owner = app.local_owner;
        let mage = app
            .engine
            .crew_cursor(owner)
            .expect("Alchemy starts with a selected mage");
        assert_eq!(
            app.engine
                .object_snapshot(mage)
                .expect("mage remains live")
                .definition_id,
            "MCLK"
        );
        assert_eq!(
            app.engine
                .object_snapshot(mage)
                .expect("mage remains live")
                .magic_energy,
            0,
            "Alchemy's NMGE rule leaves raw mana at zero, so C++ draws no HUD mana bar"
        );

        // Scenario join leaves crew inside the home base with the same
        // queued Exit command as C++ startup. Let that command finish before
        // exercising a world click: contained objects are deliberately not
        // mouse targets in C4Game::FindVisObject.
        for _ in 0..80 {
            if app
                .engine
                .object_snapshot(mage)
                .expect("mage remains live")
                .container
                .is_none()
            {
                break;
            }
            app.update().expect("execute startup Exit command");
        }
        assert!(
            app.engine
                .object_snapshot(mage)
                .expect("mage remains live")
                .container
                .is_none(),
            "Alchemy mage exits the home base before a world context click"
        );

        let mut frame = vec![0_u8; 320 * 200 * 4];
        app.render(&mut frame).expect("establish Alchemy viewport");
        let rendered_mage = app
            .snapshot
            .object(mage)
            .cloned()
            .expect("mage is present in app snapshot");
        assert_ne!(
            rendered_mage.ocf, 0,
            "live MCLK carries a targetable cached OCF"
        );
        let (screen_x, screen_y) = app
            .graphics
            .world_to_screen(
                owner,
                app.engine
                    .object_snapshot(mage)
                    .expect("mage snapshot")
                    .position,
            )
            .expect("mage is in the local viewport");
        app.handle_cursor_moved(PhysicalPosition::new(
            f64::from(screen_x),
            f64::from(screen_y),
        ))
        .expect("move pointer over mage");
        assert_eq!(
            app.graphics
                .object_at_point(&app.snapshot, owner, GuiPoint::new(screen_x, screen_y),),
            Some(mage),
            "C++ front-to-back object picking selects the topmost MCLK",
        );
        let pointer = app.ingame_pointer.expect("right-click retains viewport pointer");
        let projection = app
            .graphics
            .active_viewport_projections()
            .into_iter()
            .find(|viewport| viewport.owner == owner)
            .expect("Alchemy owner viewport projection");
        let (click_x, click_y) = ingame_pointer_viewport_pixel(pointer, projection);
        assert_ne!(click_x, 0, "fixture must enter C++'s free-alignment branch");
        assert_ne!(click_y, 0, "fixture must enter C++'s free-alignment branch");
        let click_location = Vector2::new(click_x, click_y);

        app.handle_right_mouse_button(ElementState::Pressed)
            .expect("right-down stores no command");
        assert!(app.engine.cursor_object_menu(owner).is_none());
        app.handle_right_mouse_button(ElementState::Released)
            .expect("right-up queues C4CMD_Context");
        app.update().expect("execute the context command");

        assert!(
            app.object_menu.is_none(),
            "mouse context must use the classic engine menu, not the app fallback"
        );
        let context = app
            .engine
            .cursor_object_menu(owner)
            .expect("right-up opens the mage context menu")
            .1
            .clone();
        assert_eq!(context.style, 1);
        assert!(!context.permanent);
        assert_eq!(
            context.location,
            Some(click_location),
            "the synchronized Context command keeps logical viewport-local Tx/Ty"
        );
        let magic_index = context
            .items
            .iter()
            .position(|item| item.command.contains("ContextMagic"))
            .unwrap_or_else(|| {
                panic!(
                    "MCLK context contains ContextMagic; action={:?}; items={:?}",
                    app.engine
                        .object_snapshot(mage)
                        .expect("mage remains live")
                        .action,
                    context.items
                )
            });

        let viewport = app.graphics.viewport_rect(owner).expect("Alchemy viewport");
        app.render(&mut frame)
            .expect("render the freely aligned context menu");
        let latched_screen = app
            .script_menu_presentations
            .get(&owner)
            .and_then(|state| state.location)
            .expect("free context location is latched after layout");
        let latched_local = Vector2::new(
            latched_screen.0.saturating_sub(viewport.x),
            latched_screen.1.saturating_sub(viewport.y),
        );
        assert!(
            latched_local.x <= click_location.x && latched_local.y <= click_location.y,
            "right/bottom edges may clamp the menu back into the viewport"
        );
        assert_eq!(
            app.ingame_menu_gfx
                .as_ref()
                .and_then(|gfx| gfx.menu_location),
            Some(latched_screen),
            "viewport-local coordinates are translated exactly once for drawing"
        );

        let mut moved_context = context.clone();
        let moved_x = latched_local.x.saturating_sub(4);
        assert_ne!(
            moved_x, latched_local.x,
            "fixture must leave room for relocation"
        );
        moved_context.location = Some(Vector2::new(moved_x, latched_local.y));
        app.engine
            .apply_object_update(
                mage,
                ObjectUpdate {
                    menu: Some(Some(moved_context.clone())),
                    ..ObjectUpdate::default()
                },
            )
            .expect("reopen the same context identity at another click");
        app.render(&mut frame)
            .expect("render the relocated context menu");
        assert_eq!(
            app.script_menu_presentations
                .get(&owner)
                .and_then(|state| state.location),
            Some((
                viewport.x.saturating_add(moved_x),
                viewport.y.saturating_add(latched_local.y),
            )),
            "a new click location invalidates the prior presentation latch"
        );

        let mut tall_context = moved_context;
        tall_context.location = Some(Vector2::new(
            viewport.width as i32 - 1,
            viewport.height as i32 - 1,
        ));
        tall_context.items.push(context.items[magic_index].clone());
        app.engine
            .apply_object_update(
                mage,
                ObjectUpdate {
                    menu: Some(Some(tall_context.clone())),
                    ..ObjectUpdate::default()
                },
            )
            .expect("install a taller edge-clamped context refill");
        app.render(&mut frame).expect("render the taller context");
        let edge_latched = app
            .script_menu_presentations
            .get(&owner)
            .and_then(|state| state.location)
            .expect("edge location clamps and latches");
        tall_context.items.pop();
        app.engine
            .apply_object_update(
                mage,
                ObjectUpdate {
                    menu: Some(Some(tall_context)),
                    ..ObjectUpdate::default()
                },
            )
            .expect("apply a shrinking context refill");
        app.render(&mut frame).expect("render the smaller context");
        assert_eq!(
            app.script_menu_presentations
                .get(&owner)
                .and_then(|state| state.location),
            Some(edge_latched),
            "C++ refills retain the first post-clamp rcBounds position"
        );

        app.engine
            .apply_object_update(
                mage,
                ObjectUpdate {
                    menu: Some(Some(context.clone())),
                    ..ObjectUpdate::default()
                },
            )
            .expect("restore the live context before selecting ContextMagic");

        app.dispatch_control_event(ControlEvent::RawPlayerControl {
            command: clonk_engine::COM_MENU_SELECT,
            data: i32::try_from(magic_index).expect("context index fits i32"),
        })
        .expect("select ContextMagic");
        app.dispatch_control_event(ControlEvent::RawPlayerControl {
            command: clonk_engine::COM_MENU_ENTER,
            data: 0,
        })
        .expect("enter ContextMagic");

        let spell_menu = app
            .engine
            .cursor_object_menu(owner)
            .expect("ContextMagic opens the shipped spell menu")
            .1;
        assert_eq!(
            spell_menu.extra,
            clonk_engine::ObjectMenuExtra::Components,
            "ALCO+NMGE uses C4MN_Extra_Components, never a mana footer"
        );
        let raise_gravity = spell_menu
            .items
            .iter()
            .find(|item| item.item_id == "MGUP")
            .expect("Alchemy's shipped Raise Gravity spell is player-accessible");
        assert_eq!(
            raise_gravity.components,
            [clonk_engine::ObjectMenuComponent {
                definition_id: "IROC".to_string(),
                count: 1,
            }],
            "Alchemy shows MGUP's ingredient recipe instead of mana"
        );
    }

    fn real_alchemy_right_drag_rectangle_replaces_crew_selection(
        prepared: &PreparedRealInstalledScenario,
    ) {
        // A right-down on ordinary landscape stores the down position. Once
        // motion exceeds C4MC_DragSensitivity, C4MouseControl enters
        // C4MC_Drag_Selecting; right-up sends CID_PlrSelect rather than a
        // context click (C4MouseControl.cpp:910-930,1009-1037,795-817,
        // 1160-1171). Exercise the actual app pointer/button path so the
        // platform event split cannot collapse the drag back into RightUp.
        let mut app = prepared.instantiate("Alchemy right drag parity", false);
        let owner = app.local_owner;
        let original = app
            .engine
            .crew_cursor(owner)
            .expect("Alchemy starts with a selected mage");
        advance_app_until(
            &mut app,
            "Alchemy MCLK finishes its startup Exit",
            160,
            |app| {
                app.engine.object_snapshot(original).is_some_and(|object| {
                    object.container.is_none() && object.command_stack.is_empty()
                })
            },
        );

        let mut frame = vec![0_u8; 320 * 200 * 4];
        app.snapshot = app.engine.snapshot();
        app.render(&mut frame).expect("establish Alchemy viewport");
        let (original_x, original_y) = app
            .graphics
            .world_to_screen(
                owner,
                app.engine
                    .object_snapshot(original)
                    .expect("original mage remains live")
                    .position,
            )
            .expect("original mage is visible");
        let target_pointer = (45..155)
            .step_by(10)
            .flat_map(|y| (45..275).step_by(10).map(move |x| (x, y)))
            .find_map(|(x, y)| {
                let point = GuiPoint::new(x as f32, y as f32);
                let start = GuiPoint::new(x as f32 - 24.0, y as f32 - 24.0);
                let pointer = app.graphics.viewport_point_at(point)?;
                let start_pointer = app.graphics.viewport_point_at(start)?;
                (pointer.owner == owner
                    && start_pointer.owner == owner
                    && (point.x - original_x).abs() > 50.0
                    && (point.y - original_y).abs() > 30.0
                    && app
                        .graphics
                        .object_at_point(&app.snapshot, owner, point)
                        .is_none()
                    && app
                        .graphics
                        .object_at_point(&app.snapshot, owner, start)
                        .is_none())
                .then_some(pointer)
            })
            .expect("Alchemy viewport has an empty drag target away from the original mage");
        let target_position = Vector2::new(
            target_pointer.world.x.round() as i32,
            target_pointer.world.y.round() as i32,
        );
        let replacement = app
            .engine
            .spawn_object(
                SpawnConfig::new("MCLK")
                    .with_position(target_position)
                    .with_owner(owner)
                    .with_crew_member(true),
            )
            .expect("spawn a second shipped mage");

        app.update()
            .expect("advance the spawned mage through its first OCF refresh");
        app.snapshot = app.engine.snapshot();
        app.render(&mut frame).expect("render the second mage");
        let target_position = app
            .engine
            .object_snapshot(replacement)
            .expect("second mage remains live")
            .position;
        let (target_x, target_y) = app
            .graphics
            .world_to_screen(owner, target_position)
            .expect("second mage is visible");
        let target = GuiPoint::new(target_x, target_y);
        let start = GuiPoint::new(target.x - 24.0, target.y - 24.0);
        assert_eq!(
            app.graphics.object_at_point(&app.snapshot, owner, target),
            Some(replacement),
            "right-up lands on the second mage, which would expose a collapsed context click"
        );
        assert_eq!(
            app.graphics.object_at_point(&app.snapshot, owner, start),
            None,
            "right-down begins on ordinary landscape"
        );

        app.handle_cursor_moved(PhysicalPosition::new(
            f64::from(start.x),
            f64::from(start.y),
        ))
        .expect("move to right-drag start");
        app.handle_right_mouse_button(ElementState::Pressed)
            .expect("physical right-down");
        app.handle_cursor_moved(PhysicalPosition::new(
            f64::from(target.x),
            f64::from(target.y),
        ))
        .expect("drag across the replacement mage");
        let drag = app
            .ingame_right_mouse_state
            .expect("crew selection drag remains live");
        assert_eq!(drag.motion.selection_kind, IngameDragSelectionKind::Crew);
        assert_eq!(
            app.ingame_selection_candidates(drag.motion),
            vec![replacement],
            "C4MouseControl's transient Selection contains the framed crew"
        );
        app.handle_right_mouse_button(ElementState::Released)
            .expect("physical right-up");

        assert_eq!(
            app.engine.selected_crew(owner),
            vec![replacement],
            "CID_PlrSelect replaces, rather than extends, the previous crew selection"
        );
        assert_eq!(app.engine.crew_cursor(owner), Some(replacement));
        assert!(
            app.engine.cursor_object_menu(owner).is_none(),
            "a completed selection drag must not fall through to C4CMD_Context"
        );
    }

    fn real_alchemy_right_drag_frame_drops_all_selected_carryables(
        prepared: &PreparedRealInstalledScenario,
    ) {
        // An object-only landscape frame remains in C4MouseControl::Selection
        // after right-up. Dragging either selected object then sends one Set
        // command followed by Append commands for the remaining objects
        // (C4MouseControl.cpp:626-645,795-817,909-968,1160-1201;
        // C4Player.cpp:1397-1450). Exercise the physical app events twice so
        // neither selection nor moving can collapse into a context click.
        let mut app = prepared.instantiate("Alchemy object drag parity", false);
        let owner = app.local_owner;
        let mage = app
            .engine
            .crew_cursor(owner)
            .expect("Alchemy starts with a selected mage");
        advance_app_until(
            &mut app,
            "Alchemy MCLK finishes its startup Exit",
            160,
            |app| {
                app.engine.object_snapshot(mage).is_some_and(|object| {
                    object.container.is_none() && object.command_stack.is_empty()
                })
            },
        );

        app.snapshot = app.engine.snapshot();
        app.refresh_focus();
        let mut frame = vec![0_u8; 320 * 200 * 4];
        app.render(&mut frame).expect("establish Alchemy viewport");
        let viewport = app
            .graphics
            .viewport_rect(owner)
            .expect("local Alchemy viewport");
        let (mage_x, mage_y) = app
            .graphics
            .world_to_screen(
                owner,
                app.engine
                    .object_snapshot(mage)
                    .expect("mage remains live")
                    .position,
            )
            .expect("mage is visible");
        let anchor = (50..150)
            .step_by(10)
            .flat_map(|y| (50..250).step_by(10).map(move |x| (x, y)))
            .find_map(|(x, y)| {
                let point = GuiPoint::new(x as f32, y as f32);
                let pointer = app.graphics.viewport_point_at(point)?;
                (pointer.owner == owner
                    && point.x >= viewport.x as f32 + 30.0
                    && point.x <= (viewport.x + viewport.width as i32) as f32 - 55.0
                    && point.y >= viewport.y as f32 + 30.0
                    && point.y <= (viewport.y + viewport.height as i32) as f32 - 30.0
                    && (point.x - mage_x).abs() > 70.0
                    && (point.y - mage_y).abs() > 35.0
                    && app
                        .graphics
                        .object_at_point(&app.snapshot, owner, point)
                        .is_none())
                .then_some(ingame_pointer_world_pixel(pointer))
            })
            .expect("Alchemy viewport has room for an object-only drag frame");
        let layer = app
            .engine
            .object_snapshot(mage)
            .expect("mage remains live")
            .layer;
        let spawn_bag = |app: &mut GameApp, position: Vector2| {
            let spawn = layer
                .map(|layer| {
                    SpawnConfig::new("ALC_")
                        .with_position(position)
                        .with_layer(layer)
                })
                .unwrap_or_else(|| SpawnConfig::new("ALC_").with_position(position));
            app.engine
                .spawn_object(spawn)
                .expect("spawn shipped carryable alchemy bag")
        };
        let first_bag = spawn_bag(&mut app, anchor);
        let second_bag = spawn_bag(&mut app, Vector2::new(anchor.x + 20, anchor.y));
        for bag in [first_bag, second_bag] {
            assert_ne!(
                app.engine
                    .object_snapshot(bag)
                    .expect("spawned bag remains live")
                    .ocf
                    & clonk_engine::ocf::CARRYABLE,
                0,
                "the regression target uses the shipped carryable definition"
            );
        }

        app.snapshot = app.engine.snapshot();
        app.render(&mut frame).expect("render both carryable bags");
        let (first_x, first_y) = app
            .graphics
            .world_to_screen(owner, anchor)
            .expect("first bag is visible");
        let (second_x, second_y) = app
            .graphics
            .world_to_screen(owner, Vector2::new(anchor.x + 20, anchor.y))
            .expect("second bag is visible");
        let frame_start = GuiPoint::new(first_x.min(second_x) - 24.0, first_y.min(second_y) - 24.0);
        let frame_end = GuiPoint::new(first_x.max(second_x) + 24.0, first_y.max(second_y) + 24.0);
        for point in [frame_start, frame_end] {
            assert!(
                app.graphics
                    .viewport_point_at(point)
                    .is_some_and(|pointer| pointer.owner == owner),
                "selection frame endpoint remains in the local viewport"
            );
            assert_eq!(
                app.graphics.object_at_point(&app.snapshot, owner, point),
                None,
                "selection begins and ends on landscape"
            );
        }

        app.handle_cursor_moved(PhysicalPosition::new(
            f64::from(frame_start.x),
            f64::from(frame_start.y),
        ))
        .expect("move to object-frame start");
        app.handle_right_mouse_button(ElementState::Pressed)
            .expect("physical frame right-down");
        app.handle_cursor_moved(PhysicalPosition::new(
            f64::from(frame_end.x),
            f64::from(frame_end.y),
        ))
        .expect("drag frame across both bags");
        let drag = app
            .ingame_right_mouse_state
            .expect("object selection drag remains live");
        assert_eq!(drag.motion.selection_kind, IngameDragSelectionKind::Objects);
        assert_eq!(
            app.ingame_selection_candidates(drag.motion),
            vec![second_bag, first_bag],
            "object marks retain C++ Game.Objects newest-first order"
        );
        app.handle_right_mouse_button(ElementState::Released)
            .expect("physical frame right-up retains object selection");
        assert!(
            app.engine
                .object_snapshot(mage)
                .expect("mage remains live")
                .command_stack
                .is_empty(),
            "object-frame selection is local and sends no player command"
        );

        let first_bag_point = (viewport.y..viewport.y + viewport.height as i32)
            .flat_map(|y| {
                (viewport.x..viewport.x + viewport.width as i32)
                    .map(move |x| GuiPoint::new(x as f32, y as f32))
            })
            .find(|point| {
                app.graphics.object_at_point(&app.snapshot, owner, *point) == Some(first_bag)
            })
            .expect("first selected bag has a visible C++ pick point");
        let drop_pointer = (viewport.y..viewport.y + viewport.height as i32)
            .flat_map(|y| {
                (viewport.x..viewport.x + viewport.width as i32)
                    .map(move |x| GuiPoint::new(x as f32, y as f32))
            })
            .find_map(|point| {
                let pointer = app.graphics.viewport_point_at(point)?;
                let world = ingame_pointer_world_pixel(pointer);
                let landscape = app.engine.landscape()?;
                let ground_y = (world.y..landscape.estimated_height())
                    .find(|y| landscape.is_solid_at(world.x, *y))?;
                (pointer.owner == owner
                    && (point.x - first_bag_point.x).abs() > 12.0
                    && !landscape.is_solid_at(world.x, world.y)
                    && (ground_y - world.y).abs() <= 5
                    && app
                        .graphics
                        .object_at_point(&app.snapshot, owner, point)
                        .is_none())
                .then_some((point, world))
            })
            .expect("visible landscape contains a C++ Drop cursor point");

        app.handle_cursor_moved(PhysicalPosition::new(
            f64::from(first_bag_point.x),
            f64::from(first_bag_point.y),
        ))
        .expect("move over one selected bag");
        app.handle_right_mouse_button(ElementState::Pressed)
            .expect("physical moving right-down");
        app.handle_cursor_moved(PhysicalPosition::new(
            f64::from(drop_pointer.0.x),
            f64::from(drop_pointer.0.y),
        ))
        .expect("drag selected bags to a Drop cursor point");
        app.handle_right_mouse_button(ElementState::Released)
            .expect("physical moving right-up sends object commands");

        let commands = app
            .engine
            .object_snapshot(mage)
            .expect("mage remains live")
            .command_stack
            .command_views();
        assert_eq!(commands.len(), 2, "both framed bags receive commands");
        assert!(commands.iter().all(|command| command.name == "Drop"));
        assert_eq!(
            commands
                .iter()
                .map(|command| command.target)
                .collect::<Vec<_>>(),
            vec![Some(second_bag), Some(first_bag)],
            "Game.Objects main-list order is preserved through Set then Append"
        );
        assert!(commands.iter().all(|command| {
            command.tx == Some(drop_pointer.1.x) && command.ty == Some(drop_pointer.1.y)
        }));
        assert!(app.engine.cursor_object_menu(owner).is_none());
    }

    fn real_alchemy_control_right_drag_puts_carryable_into_hut(
        prepared: &PreparedRealInstalledScenario,
    ) {
        // With Control held, C4MouseControl::DragMoving replaces the ordinary
        // Drop/Throw cursor with Put over an OCF_Container. Right-up sends a
        // C4CMD_Put whose Target is that container and whose Target2 is the
        // dragged object (C4MouseControl.cpp:833-850,1171-1201). Exercise the
        // physical pointer/modifier/button route with shipped ALC_/AHUT defs.
        let mut app = prepared.instantiate("Alchemy control-drag Put parity", false);
        let owner = app.local_owner;
        let mage = app
            .engine
            .crew_cursor(owner)
            .expect("Alchemy starts with a selected mage");
        advance_app_until(
            &mut app,
            "Alchemy MCLK finishes its startup Exit",
            160,
            |app| {
                app.engine.object_snapshot(mage).is_some_and(|object| {
                    object.container.is_none() && object.command_stack.is_empty()
                })
            },
        );

        let hut = app
            .engine
            .snapshot()
            .objects
            .iter()
            .find(|object| object.definition_id == "AHUT" && object.owner == owner)
            .map(|object| object.id)
            .expect("Alchemy starts with the player's shipped AHUT");
        assert_ne!(
            app.engine
                .object_snapshot(hut)
                .expect("AHUT remains live")
                .ocf
                & clonk_engine::ocf::CONTAINER,
            0,
            "AHUT is the C++ OCF_Container Put target"
        );

        app.snapshot = app.engine.snapshot();
        app.refresh_focus();
        let mut frame = vec![0_u8; 320 * 200 * 4];
        app.render(&mut frame).expect("establish Alchemy viewport");
        let viewport = app
            .graphics
            .viewport_rect(owner)
            .expect("local Alchemy viewport");
        let hut_point = (viewport.y..viewport.y + viewport.height as i32)
            .flat_map(|y| {
                (viewport.x..viewport.x + viewport.width as i32)
                    .map(move |x| GuiPoint::new(x as f32 + 0.5, y as f32 + 0.5))
            })
            .find(|point| app.graphics.object_at_point(&app.snapshot, owner, *point) == Some(hut))
            .expect("AHUT has a visible C++ pick point");
        let bag_pointer = (viewport.y..viewport.y + viewport.height as i32)
            .step_by(4)
            .flat_map(|y| {
                (viewport.x..viewport.x + viewport.width as i32)
                    .step_by(4)
                    .map(move |x| GuiPoint::new(x as f32 + 0.5, y as f32 + 0.5))
            })
            .find_map(|point| {
                let pointer = app.graphics.viewport_point_at(point)?;
                (pointer.owner == owner
                    && (point.x - hut_point.x).abs() > 24.0
                    && (point.y - hut_point.y).abs() > 12.0
                    && app
                        .graphics
                        .object_at_point(&app.snapshot, owner, point)
                        .is_none())
                .then_some(pointer)
            })
            .expect("Alchemy viewport has an empty bag spawn point away from AHUT");
        let bag_position = ingame_pointer_world_pixel(bag_pointer);
        let mut bag_spawn = SpawnConfig::new("ALC_").with_position(bag_position);
        if let Some(layer) = app
            .engine
            .object_snapshot(mage)
            .expect("mage remains live")
            .layer
        {
            bag_spawn = bag_spawn.with_layer(layer);
        }
        let bag = app
            .engine
            .spawn_object(bag_spawn)
            .expect("spawn the shipped carryable alchemy bag");

        app.snapshot = app.engine.snapshot();
        app.render(&mut frame).expect("render the dragged bag");
        let bag_point = (viewport.y..viewport.y + viewport.height as i32)
            .flat_map(|y| {
                (viewport.x..viewport.x + viewport.width as i32)
                    .map(move |x| GuiPoint::new(x as f32, y as f32))
            })
            .find(|point| app.graphics.object_at_point(&app.snapshot, owner, *point) == Some(bag))
            .expect("ALC_ has a visible C++ pick point");

        app.handle_modifiers_changed(ModifiersState::CONTROL)
            .expect("set Control modifier");
        app.handle_cursor_moved(PhysicalPosition::new(
            f64::from(bag_point.x),
            f64::from(bag_point.y),
        ))
        .expect("move over the shipped bag");
        app.handle_right_mouse_button(ElementState::Pressed)
            .expect("physical Control-right-down");
        app.handle_cursor_moved(PhysicalPosition::new(
            f64::from(hut_point.x),
            f64::from(hut_point.y),
        ))
        .expect("drag the bag over AHUT");
        app.handle_right_mouse_button(ElementState::Released)
            .expect("physical Control-right-up");
        app.handle_modifiers_changed(ModifiersState::empty())
            .expect("clear Control modifier");

        let commands = app
            .engine
            .object_snapshot(mage)
            .expect("mage remains live")
            .command_stack
            .command_views();
        assert_eq!(commands.len(), 1, "the drag emits exactly one Put");
        assert_eq!(commands[0].name, "Put");
        assert_eq!(commands[0].target, Some(hut));
        assert_eq!(commands[0].target2, Some(bag));
        assert_eq!(commands[0].tx, None);
        assert_eq!(commands[0].ty, None);
        assert!(app.engine.cursor_object_menu(owner).is_none());
    }

    fn real_alchemy_left_double_click_gets_carryable_like_cpp_mouse_control(
        prepared: &PreparedRealInstalledScenario,
    ) {
        // C4MouseControl's first ordinary left-up replaces the selected crew's
        // stack with MoveTo. A second left-down inside the platform's 400 ms
        // double-click window is delivered as LeftDouble instead: an Object
        // cursor replaces that command with C4CMD_Get and the following left-up
        // is ignored (C4FullScreen.cpp:327-350; C4MouseControl.cpp:817-830,
        // 982-1004,1101-1155).
        let mut app = prepared.instantiate("Alchemy mouse pickup parity", false);
        let owner = app.local_owner;
        let mage = app
            .engine
            .crew_cursor(owner)
            .expect("Alchemy starts with a selected mage");
        advance_app_until(
            &mut app,
            "Alchemy MCLK finishes its startup Exit",
            160,
            |app| {
                app.engine.object_snapshot(mage).is_some_and(|object| {
                    object.container.is_none() && object.command_stack.is_empty()
                })
            },
        );

        app.snapshot = app.engine.snapshot();
        app.refresh_focus();
        let mut frame = vec![0_u8; 320 * 200 * 4];
        app.render(&mut frame).expect("establish Alchemy viewport");
        let empty_pointer = (40..180)
            .step_by(20)
            .flat_map(|y| (20..300).step_by(20).map(move |x| (x, y)))
            .find_map(|(x, y)| {
                let point = GuiPoint::new(x as f32, y as f32);
                let pointer = app.graphics.viewport_point_at(point)?;
                (pointer.owner == owner
                    && app
                        .graphics
                        .object_at_point(&app.snapshot, owner, point)
                        .is_none())
                .then_some(pointer)
            })
            .expect("Alchemy viewport contains an empty world point");
        let bag_position = Vector2::new(
            empty_pointer.world.x.round() as i32,
            empty_pointer.world.y.round() as i32,
        );
        let mut bag_spawn = SpawnConfig::new("ALC_").with_position(bag_position);
        if let Some(layer) = app
            .engine
            .object_snapshot(mage)
            .expect("mage remains live")
            .layer
        {
            bag_spawn = bag_spawn.with_layer(layer);
        }
        let bag = app
            .engine
            .spawn_object(bag_spawn)
            .expect("spawn the shipped carryable alchemy bag");
        let bag_snapshot = app
            .engine
            .object_snapshot(bag)
            .expect("spawned bag remains live");
        assert_ne!(
            bag_snapshot.ocf & clonk_engine::ocf::CARRYABLE,
            0,
            "the regression target uses the shipped carryable definition"
        );

        // FindVisObject's OCF filter is part of the pick itself. A newer
        // foreground object with no primary mouse OCF must therefore be
        // skipped rather than blocking the carryable object behind it.
        let mut blocker =
            Definition::from_script("MBLK", "Mouse blocker", "#strict\n")
                .expect("blocker compiles");
        blocker.set_category(clonk_engine::CATEGORY_OBJECT);
        blocker.set_shape_rect(Some(clonk_engine::DefinitionRect::new(-3, -3, 6, 6)));
        app.engine
            .register_definition(blocker)
            .expect("register foreground blocker");
        let mut blocker_spawn = SpawnConfig::new("MBLK").with_position(bag_position);
        if let Some(layer) = bag_snapshot.layer {
            blocker_spawn = blocker_spawn.with_layer(layer);
        }
        let blocker = app
            .engine
            .spawn_object(blocker_spawn)
            .expect("spawn foreground non-primary blocker");

        app.snapshot = app.engine.snapshot();
        app.render(&mut frame).expect("establish Alchemy viewport");
        let viewport = app
            .graphics
            .viewport_rect(owner)
            .expect("local Alchemy viewport");
        let bag_point = (viewport.y..viewport.y + viewport.height as i32)
            .flat_map(|y| {
                (viewport.x..viewport.x + viewport.width as i32)
                    .map(move |x| GuiPoint::new(x as f32 + 0.5, y as f32 + 0.5))
            })
            .find(|point| {
                app.graphics.object_at_point(&app.snapshot, owner, *point) == Some(blocker)
                    && app.ingame_primary_mouse_target(owner, *point) == Some(bag)
            })
            .expect("the primary OCF pick sees the bag behind a foreground blocker");
        app.handle_cursor_moved(PhysicalPosition::new(
            f64::from(bag_point.x),
            f64::from(bag_point.y),
        ))
        .expect("move pointer over carryable bag");
        let click_world = ingame_pointer_world_pixel(
            app.ingame_pointer
                .expect("C++-quantized bag point maps into the local viewport"),
        );
        assert_eq!(
            app.graphics
                .object_at_point(&app.snapshot, owner, bag_point),
            Some(blocker),
            "the unfiltered foreground pick sees the newer blocker",
        );
        assert_eq!(
            app.ingame_primary_mouse_target(owner, bag_point),
            Some(bag),
            "the primary mouse OCF pick skips that blocker and resolves the carryable",
        );

        app.handle_mouse_button(ElementState::Pressed)
            .expect("first left-down");
        app.handle_mouse_button(ElementState::Released)
            .expect("first left-up");
        let first_click = app
            .engine
            .object_snapshot(mage)
            .expect("mage remains live after first click")
            .command_stack
            .command_views();
        assert_eq!(first_click.len(), 1);
        assert_eq!(first_click[0].name, "MoveTo");
        assert_eq!(first_click[0].target, None);
        assert_eq!(first_click[0].tx, Some(click_world.x));
        assert_eq!(first_click[0].ty, Some(click_world.y));

        app.handle_mouse_button(ElementState::Pressed)
            .expect("second left-down becomes LeftDouble");
        let double_click = app
            .engine
            .object_snapshot(mage)
            .expect("mage remains live after double click")
            .command_stack
            .command_views();
        assert_eq!(double_click.len(), 1);
        assert_eq!(double_click[0].name, "Get");
        assert_eq!(double_click[0].target, Some(bag));
        assert_eq!(double_click[0].tx, None);
        assert_eq!(double_click[0].ty, None);

        app.handle_mouse_button(ElementState::Released)
            .expect("post-double left-up is ignored");
        assert_eq!(
            app.engine
                .object_snapshot(mage)
                .expect("mage remains live after ignored release")
                .command_stack
                .command_views(),
            double_click,
            "the post-double release must not overwrite Get with MoveTo"
        );
    }

    #[test]
    fn real_tutorial06_elevator_rider_view_target_and_camera_stay_continuous() {
        // This is the short form of the real Tutorial06 app route below: use
        // its shipped CLNK/ELEV/ELEC definitions and the normal app snapshot
        // -> viewport -> renderer path, while opening only the test shaft so
        // the carriage can run for a small deterministic frame window.
        let mut app = real_tutorial_app(6, "Tutorial 6 elevator camera");
        let owner = app.local_owner;
        let rider = app
            .engine
            .crew_cursor(owner)
            .expect("Tutorial06 starts with a selected CLNK");
        advance_app_until(
            &mut app,
            "Tutorial06 selected CLNK completes its startup Exit",
            160,
            |app| {
                app.engine.object_snapshot(rider).is_some_and(|object| {
                    object.container.is_none() && object.action.name == "Walk"
                })
            },
        );

        app.engine
            .execute_shake_circle_operation(Vector2::new(332, 250), 180);
        let elevator = app
            .engine
            .spawn_object(
                SpawnConfig::new("ELEV")
                    .with_position(Vector2::new(332, 150))
                    .with_owner(owner),
            )
            .expect("spawn shipped Tutorial06 ELEV");
        let first = app.engine.snapshot();
        let elevator = first.object(elevator).expect("ELEV survives Initialize");
        let case_id = elevator
            .action
            .target
            .expect("real ELEV Initialize creates and targets ELEC");
        let case = first.object(case_id).expect("real ELEC exists");
        assert_eq!(case.definition_id, "ELEC");

        // CLNK's bottom vertex is y+9 and ELEC's shipped mask begins at
        // case y+11. Put the selected crew exactly on that platform and use
        // the real PUSH action target. C4SolidMask then carries it by every
        // case delta before its own movement pass (C4SolidMask.cpp:178-195,
        // 276-305), just as in the full physical-key route.
        let rider_offset = Vector2::new(0, 2);
        let rider_action = clonk_engine::ActionUpdate::default()
            .with_name("Push")
            .with_target(Some(case_id));
        app.engine
            .apply_object_update(
                rider,
                ObjectUpdate::new()
                    .with_position(Vector2::new(
                        case.position.x + rider_offset.x,
                        case.position.y + rider_offset.y,
                    ))
                    .with_velocity(Vector2::ZERO)
                    .with_command_direction(CommandDirection::Stop)
                    .with_action_update(rider_action),
            )
            .expect("attach selected CLNK to real ELEC");
        // Wait is the real ELEC FLOAT action. A downward comdir plus an
        // initial live velocity exercises ordinary fixed-point movement and
        // solid-mask rider restoration without invoking a test-only mover.
        app.engine
            .apply_object_update(
                case_id,
                ObjectUpdate::new()
                    .with_action("Wait")
                    .with_velocity(Vector2::new(0, 1))
                    .with_command_direction(CommandDirection::Down),
            )
            .expect("start real ELEC moving down the opened shaft");

        // The setup mutations above stand in for the object phase. C++
        // copies the selected ViewCursor position into ViewX/ViewY in the
        // later player phase (C4Player.cpp:200-209,1693-1713).
        app.engine
            .tick_player_systems()
            .expect("refresh rider view after fixture setup");

        app.focus_id = Some(rider);
        app.snapshot = app.engine.snapshot();
        app.refresh_focus();
        let initial_snapshot = app.snapshot.clone();
        let initial_inputs = collect_viewport_inputs(&initial_snapshot)
            .expect("real Tutorial06 player has an authoritative viewport");
        assert_eq!(initial_inputs.len(), 1);
        assert_eq!(
            initial_inputs[0]
                .focus
                .expect("player viewport focus")
                .id,
            rider
        );
        assert_eq!(
            initial_inputs[0].center,
            app.snapshot.object(rider).expect("initial rider").position,
            "C4Player::UpdateView follows the live ViewCursor position"
        );
        app.graphics
            .render_frame(&initial_snapshot, &initial_inputs);

        let initial_case = app
            .snapshot
            .object(case_id)
            .expect("initial moving ELEC")
            .position;
        let initial_rider = app
            .snapshot
            .object(rider)
            .expect("initial attached CLNK")
            .position;
        let initial_world_origin = app
            .graphics
            .world_to_screen(owner, Vector2::ZERO)
            .expect("initial viewport maps world origin")
            .1;
        let initial_rider_screen = app
            .graphics
            .world_to_screen(owner, initial_rider)
            .expect("initial viewport maps rider")
            .1;
        let mut samples = vec![(
            initial_case.y,
            initial_rider.y,
            initial_world_origin,
            initial_rider_screen,
        )];

        for frame in 1..=12 {
            app.update()
                .unwrap_or_else(|error| panic!("advance elevator frame {frame}: {error}"));
            let case = app
                .snapshot
                .object(case_id)
                .unwrap_or_else(|| panic!("ELEC survives frame {frame}"))
                .clone();
            let rider_now = app
                .snapshot
                .object(rider)
                .unwrap_or_else(|| panic!("CLNK survives frame {frame}"))
                .clone();
            assert_eq!(
                (rider_now.action.name.as_str(), rider_now.action.target),
                ("Push", Some(case_id)),
                "real PUSH attachment survives frame {frame}"
            );
            assert!(
                (rider_now.position.y - case.position.y - rider_offset.y).abs() <= 1,
                "rider and carriage cannot diverge on frame {frame}: rider={rider_now:?}, case={case:?}"
            );

            let render_snapshot = app.snapshot.clone();
            let inputs = collect_viewport_inputs(&render_snapshot)
                .expect("real Tutorial06 player keeps an authoritative viewport");
            assert_eq!(inputs.len(), 1, "one local viewport on frame {frame}");
            assert_eq!(
                inputs[0].focus.expect("player viewport focus").id,
                rider
            );
            assert_eq!(
                inputs[0].center, rider_now.position,
                "the app must present the rider's current frame position to C4Viewport on frame {frame}"
            );
            app.graphics.render_frame(&render_snapshot, &inputs);
            let world_origin = app
                .graphics
                .world_to_screen(owner, Vector2::ZERO)
                .unwrap_or_else(|| panic!("viewport maps world origin on frame {frame}"))
                .1;
            let rider_screen = app
                .graphics
                .world_to_screen(owner, rider_now.position)
                .unwrap_or_else(|| panic!("viewport maps rider on frame {frame}"))
                .1;
            samples.push((
                case.position.y,
                rider_now.position.y,
                world_origin,
                rider_screen,
            ));
        }

        assert!(
            samples.last().expect("final sample").0 > samples[0].0,
            "the real ELEC must move during the sample: {samples:?}"
        );
        for pair in samples.windows(2) {
            let [before, after] = pair else {
                unreachable!()
            };
            assert!(
                after.0 >= before.0 && after.1 >= before.1,
                "carriage/rider reversed between frames: {before:?} -> {after:?}"
            );
            assert!(
                after.2 <= before.2,
                "the fixed-point C4Viewport camera reversed between frames: {before:?} -> {after:?}"
            );
            assert!(
                after.3 >= before.3,
                "the rider jittered backwards on screen: {before:?} -> {after:?}"
            );
        }
    }

    #[test]
    fn overlay_text_helper_respects_custom_text() {
        assert!(overlay_text_needs_update("", "FRAME "));
        assert!(overlay_text_needs_update("FRAME 00005", "FRAME "));
        assert!(!overlay_text_needs_update("Inventory open", "FRAME "));

        assert!(overlay_text_needs_update("", "ENERGY "));
        assert!(overlay_text_needs_update(
            "ENERGY 100 DAMAGE 000 OWNER 1",
            "ENERGY "
        ));
        assert!(!overlay_text_needs_update("Paused", "ENERGY "));

        assert_eq!(
            c4_presentation_text(&clonk_script::c4_string_from_bytes(&[0xe9])),
            "\u{e9}"
        );

        let raw_name = clonk_script::c4_string_from_bytes(&[0xe9]);
        assert_eq!(player_join_board_line(&raw_name), "Player join: \u{e9}");
    }

    #[test]
    fn real_tutorial01_message_render_subcases_batch() {
        let prepared =
            PreparedRealInstalledScenario::new("Tutorial.c4f/Tutorial01.c4s");
        let mut failures = Vec::new();
        run_real_tutorial01_app_subcase(
            "renders_cpp_decorated_portrait_message",
            &mut failures,
            || real_tutorial01_renders_cpp_decorated_portrait_message(&prepared),
        );
        run_real_tutorial01_app_subcase(
            "scale_three_message_commits_native_pixels_after_filtered_base",
            &mut failures,
            || scale_three_tutorial_message_commits_native_pixels_after_filtered_base(&prepared),
        );
        assert_no_real_tutorial01_app_subcase_failures(failures);
    }

    fn real_tutorial01_renders_cpp_decorated_portrait_message(
        prepared: &PreparedRealInstalledScenario,
    ) {
        // TutorialMessage reaches C4GameMessage::Draw as a permanent
        // player-global message with DECO framing and an SCLK portrait
        // (Tutorial.c4f/System.c4g/Tutorial.c:22-31;
        // src/C4GameMessage.cpp:99-170).
        let _lock = env_lock().lock();
        let mut app = prepared.instantiate("Tutorial message parity", true);
        advance_app_until(&mut app, "Tutorial01 welcome message", 180, |app| {
            app_tutorial_message_contains(app, "Welcome to the world of Clonk.")
        });
        let message = app
            .snapshot
            .hud
            .messages
            .iter()
            .find(|message| {
                message
                    .lines
                    .iter()
                    .any(|line| line == "Welcome to the world of Clonk.")
            })
            .expect("shipped Tutorial01 welcome message")
            .clone();
        assert_eq!(message.kind, MessageKind::GlobalPlayer);
        assert_eq!(message.player, Some(app.local_owner));
        assert_eq!(message.target, None);
        assert_eq!(message.lines, ["Welcome to the world of Clonk."]);
        assert_eq!(message.offset, Vector2::new(50, 50));
        assert_eq!(message.color, 0xffff_ffff);
        assert_eq!(message.flags, 0x718);
        assert_eq!(message.width, Some(30));
        assert_eq!(message.decoration.as_deref(), Some("DECO"));
        assert_eq!(
            message.portrait.as_deref(),
            Some("Portrait:SCLK::0000ff::1")
        );

        let decoration = message
            .frame_decoration
            .as_ref()
            .expect("C4GameMessage snapshots DECO at creation");
        assert_eq!(decoration.source_definition, "DECO");
        assert_eq!(decoration.background_color, 0x8032_3232);
        assert_eq!(
            (
                decoration.border_top,
                decoration.border_left,
                decoration.border_right,
                decoration.border_bottom,
            ),
            (0, 0, 0, 0)
        );
        let facets = [
            decoration.top_left.as_ref(),
            decoration.top.as_ref(),
            decoration.top_right.as_ref(),
            decoration.right.as_ref(),
            decoration.bottom_right.as_ref(),
            decoration.bottom.as_ref(),
            decoration.bottom_left.as_ref(),
            decoration.left.as_ref(),
        ]
        .map(|facet| {
            let facet = facet.expect("Tutorial01 DECO contains all eight frame facets");
            (
                facet.x,
                facet.y,
                facet.width,
                facet.height,
                facet.target_x,
                facet.target_y,
            )
        });
        assert_eq!(
            facets,
            [
                (0, 0, 16, 16, -8, -7),
                (16, 0, 58, 12, 0, -7),
                (74, 0, 16, 16, -7, -7),
                (74, 16, 16, 58, -7, 0),
                (74, 74, 16, 16, -7, -8),
                (16, 76, 58, 16, 0, -6),
                (0, 74, 16, 16, -8, -8),
                (0, 16, 16, 58, -8, 0),
            ]
        );

        app.resize(1152, 644)
            .expect("resize to the reported logical surface");
        hold_message_board_for_frame_comparison(&mut app);
        let messages = std::mem::take(&mut app.snapshot.hud.messages);
        let mut warm = vec![0_u8; 1152 * 644 * 4];
        app.render(&mut warm)
            .expect("warm the message-free presentation state");
        let frame_gamma = app
            .graphics
            .active_gamma_ramp(&app.snapshot.environment.gamma);
        let mut baseline = vec![0_u8; 1152 * 644 * 4];
        app.render(&mut baseline)
            .expect("render the message-free Tutorial01 baseline");
        app.snapshot.hud.messages = messages;
        let mut rendered = vec![0_u8; 1152 * 644 * 4];
        app.render(&mut rendered)
            .expect("classic Tutorial01 C4GameMessage renders");

        let viewport = app
            .graphics
            .active_viewport_projections()
            .into_iter()
            .find(|viewport| viewport.owner == app.local_owner)
            .expect("local Tutorial01 viewport")
            .rect;
        assert_eq!(viewport, Rect::new(216, 56, 720, 560));
        let fonts = app
            .assets
            .clonk_fonts
            .as_deref()
            .expect("classic FontRegular");
        assert_eq!(
            fonts.text.measure("Welcome to the world of Clonk.", true),
            (194, 22)
        );

        let core_frame = Rect::new(576, 106, 278, 64);
        let deco_envelope = Rect::new(568, 99, 295, 81);
        let inside = |rect: Rect, x: i32, y: i32| {
            x >= rect.x
                && x < rect.x + rect.width as i32
                && y >= rect.y
                && y < rect.y + rect.height as i32
        };
        let changed = rendered
            .chunks_exact(4)
            .zip(baseline.chunks_exact(4))
            .enumerate()
            .filter_map(|(index, (actual, before))| (actual != before).then_some(index))
            .collect::<Vec<_>>();
        assert!(!changed.is_empty(), "the C4GameMessage contributes pixels");
        assert!(changed.iter().all(|index| {
            let x = (*index % 1152) as i32;
            let y = (*index / 1152) as i32;
            inside(viewport, x, y) && inside(deco_envelope, x, y)
        }));
        assert!(
            changed.iter().any(|index| {
                let x = (*index % 1152) as i32;
                let y = (*index / 1152) as i32;
                !inside(core_frame, x, y)
            }),
            "real DECO facets extend outside the core frame"
        );

        let pixel = |frame: &[u8], x: usize, y: usize| {
            let offset = (y * 1152 + x) * 4;
            Color::new(
                frame[offset],
                frame[offset + 1],
                frame[offset + 2],
                frame[offset + 3],
            )
        };
        assert_eq!(
            pixel(&rendered, 572, 100),
            clonk_frontend::gamma_encode_fragment(Color::opaque(126, 66, 23), &frame_gamma),
            "the opaque top-left DECO texel must draw outside the core frame"
        );

        let mut expected_gap = Surface::new(1, 1, clonk_graphics::PixelFormat::Rgba8888);
        expected_gap
            .set_pixel(0, 0, pixel(&baseline, 645, 130))
            .expect("seed the gap background");
        clonk_frontend::classic_gui::draw_engine_box(
            &mut expected_gap,
            0,
            0,
            0,
            0,
            0x8032_3232,
            Some(&frame_gamma),
        );
        assert_eq!(
            pixel(&rendered, 645, 130),
            expected_gap.get_pixel(0, 0).expect("blended gap pixel"),
            "the ten-pixel portrait/text gap contains only DECO background"
        );
    }

    fn scale_three_tutorial_message_commits_native_pixels_after_filtered_base(
        prepared: &PreparedRealInstalledScenario,
    ) {
        // FontRegular is rebuilt with Application.GetScale(), but its public
        // geometry remains in GUI units. Ordinary frame/portrait pixels pass
        // through GL_LINEAR first and native glyphs are then drawn into the
        // physical viewport (C4Fonts.cpp:158-173; StdFont.cpp:319-352,841-842;
        // C4Viewport.cpp:852-854).
        let _lock = env_lock().lock();
        let mut app = prepared.instantiate("Native tutorial message parity", true);
        advance_app_until(&mut app, "Tutorial01 welcome message", 180, |app| {
            app_tutorial_message_contains(app, "Welcome to the world of Clonk.")
        });
        app.configure_native_startup_fonts(3.0, false);
        assert!(app.can_defer_native_game_messages(3.0));

        let gamma = app
            .graphics
            .active_gamma_ramp(&app.snapshot.environment.gamma);
        let mut presenter = clonk_scaling::FramePresenter::new(3.0, 960, 598);
        let mut output = vec![0_u8; 960 * 598 * 4];
        let refreshed = presenter
            .present(&mut output, |frame| {
                app.render_for_presentation(frame, false, false, true)
            })
            .expect("render filtered base before Tutorial01 message");
        assert!(refreshed);
        let filtered_base = output.clone();

        app.render_native_game_messages(&mut output, presenter.presentation_geometry(), &gamma)
            .expect("render native Tutorial01 message text");
        assert_ne!(
            output, filtered_base,
            "the physical C4GameMessage pass must contribute message pixels"
        );

        // A 320x200 logical surface creates a nominal 960x600 lower-left GL
        // viewport in a 960x598 framebuffer, clipping two physical rows from
        // the top. Native message pixels must retain that offset and the
        // owning C4Viewport clip.
        let viewport = app
            .graphics
            .active_viewport_projections()
            .into_iter()
            .find(|viewport| viewport.owner == app.local_owner)
            .expect("local Tutorial01 viewport")
            .rect;
        let physical_viewport = Rect::new(
            viewport.x * 3,
            viewport.y * 3 - 2,
            viewport.width * 3,
            viewport.height * 3,
        );
        let changed = output
            .chunks_exact(4)
            .zip(filtered_base.chunks_exact(4))
            .enumerate()
            .filter_map(|(index, (native, base))| (native != base).then_some(index));
        let mut changed_count = 0;
        for index in changed {
            changed_count += 1;
            let point = Rect::new((index % 960) as i32, (index / 960) as i32, 1, 1);
            assert!(
                physical_viewport.intersection(point).is_some(),
                "native message pixel ({}, {}) escaped its viewport clip",
                point.x,
                point.y
            );
        }
        assert!(changed_count > 0);

        let solid = [17_u8, 29, 43, 255];
        let mut nominal = solid
            .into_iter()
            .cycle()
            .take(960 * 600 * 4)
            .collect::<Vec<_>>();
        let mut clipped = solid
            .into_iter()
            .cycle()
            .take(960 * 598 * 4)
            .collect::<Vec<_>>();
        let nominal_geometry =
            clonk_scaling::FramePresenter::new(3.0, 960, 600).presentation_geometry();
        let clipped_geometry =
            clonk_scaling::FramePresenter::new(3.0, 960, 598).presentation_geometry();
        app.render_native_game_messages(&mut nominal, nominal_geometry, &gamma)
            .expect("render nominal native-message probe");
        app.render_native_game_messages(&mut clipped, clipped_geometry, &gamma)
            .expect("render clipped native-message probe");
        for y in 0..598_usize {
            let clipped_row = &clipped[y * 960 * 4..(y + 1) * 960 * 4];
            let nominal_row = &nominal[(y + 2) * 960 * 4..(y + 3) * 960 * 4];
            assert_eq!(
                clipped_row,
                nominal_row,
                "the 598-row framebuffer must clip nominal physical row {}",
                y + 2
            );
        }
    }

    #[test]
    fn real_tutorial09_hud_names_subcases_batch() {
        let prepared =
            PreparedRealInstalledScenario::new("Tutorial.c4f/Tutorial09.c4s");
        let mut failures = Vec::new();
        run_real_tutorial09_app_subcase(
            "temporary_breath_physical_renders_the_cpp_hud_bar",
            &mut failures,
            || tutorial09_real_temporary_breath_physical_renders_the_cpp_hud_bar(&prepared),
        );
        run_real_tutorial09_app_subcase(
            "system_names_preserve_cpp_ready_conkit_route",
            &mut failures,
            || app_tutorial09_system_names_preserve_cpp_ready_conkit_route(&prepared),
        );
        assert_no_real_tutorial09_app_subcase_failures(failures);
    }

    fn run_real_tutorial09_app_subcase(
        name: &'static str,
        failures: &mut Vec<&'static str>,
        subcase: impl FnOnce(),
    ) {
        eprintln!("running Tutorial09 app subcase `{name}`");
        if std::panic::catch_unwind(std::panic::AssertUnwindSafe(subcase)).is_err() {
            eprintln!("Tutorial09 app subcase `{name}` failed; continuing batch");
            failures.push(name);
        }
    }

    fn assert_no_real_tutorial09_app_subcase_failures(failures: Vec<&str>) {
        assert!(
            failures.is_empty(),
            "Tutorial09 app subcase(s) failed: {}",
            failures.join(", ")
        );
    }

    fn tutorial09_real_temporary_breath_physical_renders_the_cpp_hud_bar(
        prepared: &PreparedRealInstalledScenario,
    ) {
        // Tutorial09 raises the ready CLNK's temporary Breath physical to
        // 250000 without rewriting its current 50000 breath
        // (Tutorial09.c4s/Script.c:18-23; C4Script.cpp:584-598;
        // C4Object.cpp:192-195). C4Viewport therefore draws the cyan breath
        // bar because 0 < Breath < GetPhysical()->Breath
        // (C4Viewport.cpp:920-943; C4Object.cpp:2728-2731).
        let mut app = prepared.instantiate("Breath HUD parity", false);
        wait_for_running(&mut app);
        app.update().expect("Tutorial09 first running frame");

        let clonk = app
            .snapshot
            .players
            .iter()
            .find(|player| player.id == app.local_owner)
            .and_then(|player| player.cursor)
            .expect("Tutorial09 local cursor CLNK");
        let object = app
            .snapshot
            .object(clonk)
            .expect("Tutorial09 cursor remains in the snapshot");
        let current_breath = object.breath;
        let capacity = app
            .engine
            .find_object_index(clonk)
            .map(|index| app.engine.object_physical(index).breath)
            .expect("Tutorial09 cursor has resolved physicals");
        assert_eq!(current_breath, 50_000, "CLNK keeps its birth breath");
        assert_eq!(capacity, 250_000, "Tutorial09 installs AquaClonk capacity");

        let overlays = {
            let game_app = &mut app.app;
            collect_player_overlays(
                &mut game_app.engine,
                &game_app.snapshot,
                Some(clonk),
                &game_app.bindings,
                &game_app.gamepad_bindings,
            )
        };
        let crew = overlays
            .iter()
            .find(|player| player.owner == app.local_owner)
            .and_then(|player| player.crew.iter().find(|crew| crew.object_id == clonk))
            .expect("Tutorial09 cursor reaches the HUD overlay");
        assert_eq!(crew.breath, 50_000);
        assert_eq!(crew.breath_capacity, 250_000);
        assert!(crew.breath != 0 && crew.breath < crew.breath_capacity);

        hold_message_board_for_frame_comparison(&mut app);

        // The stock EnergyBars.png is split into six 8px columns and three
        // 12px cap/tile rows (C4GraphicsResource.cpp:231-241). With portraits
        // enabled, an energy bar already occupying slot zero, and no magic,
        // the breath bar occupies x=5+(8+1), y=35+10+10, h=200-95. Its
        // filled pixels come from cyan columns 4/5 selected by bar_idx=2
        // (C4Facet.cpp:334-387).
        let hud = app.graphics.hud_graphics();
        let bars = hud.energy_bars.as_ref().expect("stock EnergyBars.png");
        assert_eq!((bars.width(), bars.height()), (48, 36));
        let mut surface = Surface::new(320, 200, PixelFormat::Rgba8888);
        clonk_frontend::hud::draw_level_bar(
            &mut surface,
            &hud,
            clonk_graphics::Rect::new(0, 0, 320, 200),
            clonk_frontend::hud::HudBarKind::Breath,
            1,
            crew.breath,
            crew.breath_capacity,
            true,
        );

        let painted = surface
            .pixels()
            .chunks_exact(4)
            .enumerate()
            .filter(|(_, pixel)| pixel[3] != 0)
            .map(|(index, pixel)| {
                (
                    (index % 320) as i32,
                    (index / 320) as i32,
                    [pixel[0], pixel[1], pixel[2]],
                )
            })
            .collect::<Vec<_>>();
        assert!(!painted.is_empty(), "real cyan breath asset draws pixels");
        assert!(painted
            .iter()
            .all(|(x, y, _)| (14..22).contains(x) && (55..160).contains(y)));
        assert!(
            painted.iter().any(|(_, y, [r, g, b])| *y >= 139
                && *g > r.saturating_add(20)
                && *b > r.saturating_add(20)),
            "the lower 20% uses the stock cyan filled breath column"
        );

        // Exercise the complete GameApp -> GraphicsOverlay -> render_frame
        // seam with the real scenario and graphics. Setting current breath to
        // capacity suppresses only C++'s `Breath < GetPhysical()->Breath`
        // predicate; restoring 50000 must add fragments exclusively inside
        // the compact second bar slot (C4Viewport.cpp:924-943).
        let mut frame = vec![0; app.graphics.surface().pixels().len()];
        app.snapshot
            .objects
            .iter_mut()
            .find(|object| object.id == clonk)
            .expect("Tutorial09 cursor remains mutable")
            .breath = capacity;
        app.render_running(&mut frame, false)
            .expect("render Tutorial09 with full breath");
        app.render_running(&mut frame, false)
            .expect("stabilize Tutorial09 full-breath frame");
        let without_breath = frame.clone();

        app.snapshot
            .objects
            .iter_mut()
            .find(|object| object.id == clonk)
            .expect("Tutorial09 cursor remains mutable")
            .breath = current_breath;
        app.render_running(&mut frame, false)
            .expect("render Tutorial09 with partial breath");
        let with_breath = frame.clone();

        app.snapshot
            .objects
            .iter_mut()
            .find(|object| object.id == clonk)
            .expect("Tutorial09 cursor remains mutable")
            .breath = capacity;
        app.render_running(&mut frame, false)
            .expect("render Tutorial09 with breath suppressed again");
        assert_eq!(
            frame, without_breath,
            "the stationary real frame is otherwise deterministic"
        );

        let viewport = app
            .graphics
            .viewport_rect(app.local_owner)
            .expect("Tutorial09 local viewport");
        let bar_x = viewport.x + 14;
        let bar_y = viewport.y + 55;
        let bar_height = viewport.height as i32 - 95;
        assert!(bar_height > 0, "C++ viewport height gate permits HUD bars");
        let fill_y = bar_y + bar_height - current_breath * bar_height / capacity;
        let changed = with_breath
            .chunks_exact(4)
            .zip(without_breath.chunks_exact(4))
            .enumerate()
            .filter(|(_, (with, without))| with != without)
            .map(|(index, (pixel, _))| {
                (
                    (index % 320) as i32,
                    (index / 320) as i32,
                    [pixel[0], pixel[1], pixel[2]],
                )
            })
            .collect::<Vec<_>>();
        assert!(
            !changed.is_empty(),
            "partial real Tutorial09 breath paints the HUD"
        );
        assert!(
            changed.iter().all(|(x, y, _)| {
                (bar_x..bar_x + 8).contains(x) && (bar_y..bar_y + bar_height).contains(y)
            }),
            "breath-only fragments stay inside the C++ bar rectangle: {changed:?}"
        );
        assert!(
            changed.iter().any(|(_, y, _)| *y < fill_y),
            "the empty breath source column paints above yBar"
        );
        assert!(
            changed.iter().any(|(_, y, [r, g, b])| {
                *y >= fill_y && *g > r.saturating_add(10) && *b > r.saturating_add(10)
            }),
            "the cyan filled source column paints at and below yBar"
        );
    }

    #[test]
    fn app_virtual_keyboard_routes_cpp_player_one_keys_without_arrow_aliases() {
        // C++ keyboard set one maps movement to S/Z/X/C and does not alias
        // the arrow keys (C4Config.cpp:624-635). Exercise those physical
        // keys through GameApp rather than injecting logical ControlEvents.
        let mut app = new_running_sandbox_app();
        let mut keyboard = AppVirtualKeyboard::new(&mut app);

        for (key, com) in [
            (VirtualKeyCode::KeyS, clonk_engine::COM_UP),
            (VirtualKeyCode::KeyZ, clonk_engine::COM_LEFT),
            (VirtualKeyCode::KeyX, clonk_engine::COM_DOWN),
            (VirtualKeyCode::KeyC, clonk_engine::COM_RIGHT),
        ] {
            keyboard.press(key).expect("physical key press");
            assert_ne!(
                keyboard.player_control().pressed_coms & (1 << com),
                0,
                "{key:?} must reach the matching C4Player::InCom bit"
            );
            keyboard.release(key).expect("physical key release");
            assert_eq!(
                keyboard.player_control().pressed_coms & (1 << com),
                0,
                "{key:?} release must clear the C4Player::InCom bit in either \
                 control style (clonk-rs key-up divergence)"
            );
        }

        let before_arrows = keyboard.player_control();
        for key in [
            VirtualKeyCode::ArrowUp,
            VirtualKeyCode::ArrowLeft,
            VirtualKeyCode::ArrowDown,
            VirtualKeyCode::ArrowRight,
        ] {
            keyboard.press(key).expect("unbound arrow press");
            keyboard.release(key).expect("unbound arrow release");
        }
        assert_eq!(keyboard.player_control(), before_arrows);
    }

    #[test]
    fn app_virtual_keyboard_completes_real_tutorial01_route() {
        // Drive Tutorial01 through the same physical keyboard-one boundary as
        // C++: A/S/D/Z/X/C are Throw/Up/Dig/Left/Down/Right
        // (C4Config.cpp:624-635). The complete real script requires FLAG
        // delivery through HUT2's context menu, buffered DigSingle plus live
        // DownLeft/Left steering to GOLD, and a physical return climb before
        // SCRG fulfills (Tutorial01/Script.c:61-182; C4Player.cpp:1213-1229,
        // 1490-1554; C4Object.cpp:3618-3628,3645-3651,3743-3754).
        let mut app = real_tutorial_app(1, "Tutorial 1 app virtual player");
        let clonk = app
            .engine
            .crew_cursor(app.local_owner)
            .expect("Tutorial01 selected CLNK");
        let hut = app_object_with_definition(&app, "HUT2").expect("Tutorial01 HUT2");

        advance_app_until(
            &mut app,
            "Tutorial01 CLNK lands in the valley",
            180,
            |app| {
                app.engine
                    .object_snapshot(clonk)
                    .is_some_and(|object| object.action.name == "Walk")
            },
        );
        advance_app_until(
            &mut app,
            "Tutorial01 creates FLAG and points left",
            500,
            |app| {
                app_object_with_definition(app, "FLAG").is_some()
                    && app_tutorial_message_contains(app, "hill to your left")
            },
        );
        let flag = app_object_with_definition(&app, "FLAG").expect("Tutorial01 FLAG");

        // Held Z supplies horizontal jump momentum. Each physical S tap is
        // separated by twelve app ticks, beyond C4DoubleClick's ten-tick
        // window, and its release must preserve the still-held Z bit.
        {
            let mut keyboard = AppVirtualKeyboard::new(&mut app);
            keyboard
                .press(VirtualKeyCode::KeyZ)
                .expect("physical Z toward FLAG");
        }
        for _ in 0..30 {
            let clonk_now = app
                .engine
                .object_snapshot(clonk)
                .expect("CLNK survives left-hill route");
            if app_clonk_carries(&app, clonk, "FLAG") || clonk_now.position.x <= 25 {
                break;
            }
            if clonk_now.action.name == "Walk" {
                let mut keyboard = AppVirtualKeyboard::new(&mut app);
                keyboard
                    .tap(VirtualKeyCode::KeyS)
                    .expect("physical S jumps toward FLAG");
                assert_ne!(
                    keyboard.player_control().pressed_coms & (1 << clonk_engine::COM_LEFT),
                    0,
                    "releasing S must preserve held Z/Left"
                );
            }
            for _ in 0..12 {
                app.update().expect("advance left-hill jump");
            }
        }
        {
            let mut keyboard = AppVirtualKeyboard::new(&mut app);
            keyboard
                .release(VirtualKeyCode::KeyZ)
                .expect("release physical Z at FLAG");
        }
        if !app_clonk_carries(&app, clonk, "FLAG") {
            advance_app_until(&mut app, "CLNK lands beside FLAG", 80, |app| {
                app.engine
                    .object_snapshot(clonk)
                    .is_some_and(|object| object.action.name == "Walk" && object.position.x <= 40)
            });
            {
                let mut keyboard = AppVirtualKeyboard::new(&mut app);
                keyboard
                    .press(VirtualKeyCode::KeyC)
                    .expect("physical C collects FLAG");
            }
            advance_app_until(&mut app, "CLNK naturally collects FLAG", 40, |app| {
                app_clonk_carries(app, clonk, "FLAG")
            });
            AppVirtualKeyboard::new(&mut app)
                .release(VirtualKeyCode::KeyC)
                .expect("release physical C after FLAG pickup");
        }
        assert_eq!(
            app.engine
                .object_snapshot(flag)
                .expect("collected FLAG")
                .container,
            Some(clonk)
        );
        assert!(
            app_cursor_inventory_contains(&mut app, clonk, "FLAG"),
            "the collected FLAG must reach the rendered cursor inventory"
        );
        app.snapshot.hud.messages.clear();
        let mut rendered = vec![0_u8; 320 * 200 * 4];
        app.render(&mut rendered)
            .expect("render Tutorial01 with FLAG inventory");

        advance_app_until(&mut app, "Tutorial01 points toward the cabin", 500, |app| {
            app_tutorial_message_contains(app, "cabin on the hill to your right")
        });
        {
            let mut keyboard = AppVirtualKeyboard::new(&mut app);
            keyboard
                .press(VirtualKeyCode::KeyC)
                .expect("physical C toward HUT2");
        }
        for _ in 0..90 {
            let clonk_now = app
                .engine
                .object_snapshot(clonk)
                .expect("CLNK survives cabin route");
            if clonk_now.position.x >= 558 {
                break;
            }
            if clonk_now.action.name == "Walk" {
                let mut keyboard = AppVirtualKeyboard::new(&mut app);
                keyboard
                    .tap(VirtualKeyCode::KeyS)
                    .expect("physical S jumps toward HUT2");
                assert_ne!(
                    keyboard.player_control().pressed_coms & (1 << clonk_engine::COM_RIGHT),
                    0,
                    "releasing S must preserve held C/Right"
                );
            }
            for _ in 0..12 {
                app.update().expect("advance cabin jump");
            }
        }
        AppVirtualKeyboard::new(&mut app)
            .release(VirtualKeyCode::KeyC)
            .expect("release physical C beside HUT2");
        advance_app_until(&mut app, "CLNK lands beside HUT2", 60, |app| {
            app.engine
                .object_snapshot(clonk)
                .is_some_and(|object| object.action.name == "Walk")
        });
        AppVirtualKeyboard::new(&mut app)
            .press(VirtualKeyCode::KeyZ)
            .expect("physical Z aligns with HUT2 entrance");
        advance_app_until(&mut app, "CLNK aligns with HUT2 entrance", 20, |app| {
            app.engine
                .object_snapshot(clonk)
                .is_some_and(|object| object.position.x <= 570)
        });
        {
            let mut keyboard = AppVirtualKeyboard::new(&mut app);
            keyboard
                .release(VirtualKeyCode::KeyZ)
                .expect("release physical Z at HUT2 entrance");
            keyboard
                .tap(VirtualKeyCode::KeyS)
                .expect("physical S enters HUT2");
        }
        advance_app_until(&mut app, "FLAG-carrying CLNK enters HUT2", 40, |app| {
            app.engine
                .object_snapshot(clonk)
                .is_some_and(|object| object.container == Some(hut))
        });

        // C++ inserts Put first while the contained CLNK carries FLAG
        // (C4ObjectMenu.cpp:335-359). Physical A becomes MenuEnter rather than
        // a world Throw while this cursor menu is active.
        advance_app_until(&mut app, "HUT2 context Put menu", 20, |app| {
            app.engine
                .cursor_object_menu(app.local_owner)
                .is_some_and(|(_, menu)| {
                    menu.selection == 0
                        && menu.items.first().is_some_and(|item| item.caption == "Put")
                })
        });
        AppVirtualKeyboard::new(&mut app)
            .tap(VirtualKeyCode::KeyA)
            .expect("physical A puts FLAG into HUT2");
        advance_app_until(&mut app, "FLAG enters HUT2", 80, |app| {
            app.engine
                .object_snapshot(flag)
                .is_some_and(|object| object.container == Some(hut))
        });
        advance_app_until(&mut app, "FLAG makes HUT2 the player base", 80, |app| {
            app.engine
                .object_snapshot(hut)
                .is_some_and(|object| object.base == app.local_owner)
        });
        advance_app_until(
            &mut app,
            "Tutorial01 Exit prompt and context row",
            450,
            |app| {
                app_tutorial_message_contains(app, "select 'Exit'")
                    && app
                        .engine
                        .cursor_object_menu(app.local_owner)
                        .is_some_and(|(_, menu)| {
                            menu.items.iter().any(|item| item.caption == "Exit")
                        })
            },
        );

        // Script148 highlights physical X/Down plus A. Move down through the
        // real context rows, including any Buy/Sell rows enabled by the base,
        // rather than selecting Exit by index or mutating menu state.
        let context_items = app
            .engine
            .cursor_object_menu(app.local_owner)
            .expect("HUT2 context with Exit")
            .1
            .items
            .len();
        for _ in 0..=context_items {
            let exit_selected = app
                .engine
                .cursor_object_menu(app.local_owner)
                .and_then(|(_, menu)| {
                    usize::try_from(menu.selection)
                        .ok()
                        .map(|index| (menu, index))
                })
                .and_then(|(menu, index)| menu.items.get(index))
                .is_some_and(|item| item.caption == "Exit");
            if exit_selected {
                break;
            }
            AppVirtualKeyboard::new(&mut app)
                .tap(VirtualKeyCode::KeyX)
                .expect("physical X navigates toward Exit");
        }
        assert!(
            app.engine
                .cursor_object_menu(app.local_owner)
                .and_then(|(_, menu)| usize::try_from(menu.selection)
                    .ok()
                    .map(|index| (menu, index)))
                .and_then(|(menu, index)| menu.items.get(index))
                .is_some_and(|item| item.caption == "Exit"),
            "physical X must select the real Exit row"
        );
        AppVirtualKeyboard::new(&mut app)
            .tap(VirtualKeyCode::KeyA)
            .expect("physical A activates Exit");
        advance_app_until(&mut app, "CLNK exits HUT2", 60, |app| {
            app.engine
                .object_snapshot(clonk)
                .is_some_and(|object| object.container.is_none())
        });

        advance_app_until(
            &mut app,
            "Tutorial01 creates GOLD and sends CLNK to the valley",
            120,
            |app| {
                app_object_with_definition(app, "GOLD").is_some()
                    && app_tutorial_message_contains(app, "back into the valley")
            },
        );
        let gold = app_object_with_definition(&app, "GOLD").expect("Tutorial01 GOLD");
        AppVirtualKeyboard::new(&mut app)
            .press(VirtualKeyCode::KeyZ)
            .expect("physical Z returns to the lesson valley");
        advance_app_until(
            &mut app,
            "CLNK reaches the digging lesson area",
            260,
            |app| {
                app.engine.object_snapshot(clonk).is_some_and(|object| {
                    (150..250).contains(&object.position.x)
                        && (250..350).contains(&object.position.y)
                })
            },
        );
        AppVirtualKeyboard::new(&mut app)
            .release(VirtualKeyCode::KeyZ)
            .expect("release physical Z in lesson valley");
        advance_app_until(&mut app, "Tutorial01 enables digging", 160, |app| {
            app_tutorial_message_contains(app, "start a digging process")
                && app
                    .engine
                    .object_snapshot(clonk)
                    .is_some_and(|object| object.temporary_physical.is_none())
        });

        // D is buffered until C4DoubleClick (10) expires. Do not press X/Z
        // early: C4Player::InCom would flush the pending DigSingle immediately
        // on a different press (C4Player.cpp:1522-1536).
        let dig_press_frame = app.engine.frame();
        AppVirtualKeyboard::new(&mut app)
            .tap(VirtualKeyCode::KeyD)
            .expect("physical D starts buffered DigSingle");
        advance_app_until(&mut app, "CLNK starts real Dig action", 30, |app| {
            app.engine
                .object_snapshot(clonk)
                .is_some_and(|object| object.action.name == "Dig")
        });
        assert!(
            app.engine.frame().saturating_sub(dig_press_frame) > 10,
            "physical D must wait through C4DoubleClick before DigSingle"
        );
        {
            let mut keyboard = AppVirtualKeyboard::new(&mut app);
            keyboard
                .press(VirtualKeyCode::KeyX)
                .expect("physical X steers Dig down");
            keyboard
                .press(VirtualKeyCode::KeyZ)
                .expect("physical Z adds leftward Dig steering");
            let control = keyboard.player_control();
            assert_ne!(control.pressed_coms & (1 << clonk_engine::COM_DOWN), 0);
            assert_ne!(control.pressed_coms & (1 << clonk_engine::COM_LEFT), 0);
            assert_eq!(
                keyboard
                    .engine()
                    .object_snapshot(clonk)
                    .expect("CLNK after X+Z")
                    .command_direction,
                CommandDirection::DownLeft
            );
        }
        advance_app_until(&mut app, "diagonal Dig reaches GOLD depth", 140, |app| {
            app_clonk_carries(app, clonk, "GOLD")
                || app
                    .engine
                    .object_snapshot(clonk)
                    .is_some_and(|object| object.position.y >= 320)
        });
        {
            let mut keyboard = AppVirtualKeyboard::new(&mut app);
            keyboard
                .release(VirtualKeyCode::KeyX)
                .expect("release physical X while Z remains held");
            let control = keyboard.player_control();
            assert_eq!(control.pressed_coms & (1 << clonk_engine::COM_DOWN), 0);
            assert_ne!(control.pressed_coms & (1 << clonk_engine::COM_LEFT), 0);
            let clonk_now = keyboard
                .engine()
                .object_snapshot(clonk)
                .expect("CLNK after partial Dig release");
            assert_eq!(clonk_now.action.name, "Dig");
            assert_eq!(clonk_now.command_direction, CommandDirection::Left);
        }
        advance_app_until(
            &mut app,
            "leftward Dig naturally collects GOLD",
            180,
            |app| app_clonk_carries(app, clonk, "GOLD"),
        );
        AppVirtualKeyboard::new(&mut app)
            .release(VirtualKeyCode::KeyZ)
            .expect("release physical Z after GOLD pickup");
        assert_eq!(
            app.engine
                .object_snapshot(gold)
                .expect("collected GOLD")
                .container,
            Some(clonk)
        );
        advance_app_until(
            &mut app,
            "CLNK stops digging after GOLD pickup",
            30,
            |app| {
                app.engine
                    .object_snapshot(clonk)
                    .is_some_and(|object| object.action.name == "Walk")
            },
        );
        assert!(
            app_cursor_inventory_contains(&mut app, clonk, "GOLD"),
            "the collected GOLD must reach the rendered cursor inventory"
        );
        // Typed C4GameMessage rejection has its own regression; isolate this
        // inventory-render assertion from that unported overlay.
        app.snapshot.hud.messages.clear();
        app.render(&mut rendered)
            .expect("render Tutorial01 with GOLD inventory");

        // Walk out of the excavated tunnel, then preserve held physical C
        // while reacting to the same Walk/Scale/Jump transitions as the
        // engine virtual route. Re-pressing C on entry to DFA_SCALE supplies
        // the edge C++ uses to let go or climb; an S tap on landing/flight
        // transitions jumps clear without assigning position or action
        // (C4Object.cpp:3618-3628,4823-4855).
        AppVirtualKeyboard::new(&mut app)
            .press(VirtualKeyCode::KeyC)
            .expect("physical C walks out of the GOLD tunnel");
        advance_app_until(
            &mut app,
            "GOLD-carrying CLNK exits the tunnel",
            180,
            |app| {
                app.engine
                    .object_snapshot(clonk)
                    .is_some_and(|object| object.position.x >= 215)
            },
        );
        AppVirtualKeyboard::new(&mut app)
            .release(VirtualKeyCode::KeyC)
            .expect("release physical C outside the GOLD tunnel");
        AppVirtualKeyboard::new(&mut app)
            .press(VirtualKeyCode::KeyC)
            .expect("physical C starts the return climb");
        let mut previous_action = String::new();
        for _ in 0..1_800 {
            let clonk_now = app
                .engine
                .object_snapshot(clonk)
                .expect("GOLD-carrying CLNK survives the return");
            if clonk_now.position.x >= 558 {
                break;
            }
            let action = clonk_now.action.name.clone();
            let entered_scale =
                action.starts_with("Scale") && !previous_action.starts_with("Scale");
            let left_scale_in_flight = action == "Jump" && previous_action.starts_with("Scale");
            let landed = action == "Walk" && previous_action != "Walk";
            if entered_scale {
                let mut keyboard = AppVirtualKeyboard::new(&mut app);
                keyboard
                    .release(VirtualKeyCode::KeyC)
                    .expect("release physical C on Scale transition");
                keyboard
                    .press(VirtualKeyCode::KeyC)
                    .expect("re-press physical C on Scale transition");
            } else if landed || left_scale_in_flight {
                let mut keyboard = AppVirtualKeyboard::new(&mut app);
                keyboard
                    .tap(VirtualKeyCode::KeyS)
                    .expect("physical S advances the return climb");
                assert_ne!(
                    keyboard.player_control().pressed_coms & (1 << clonk_engine::COM_RIGHT),
                    0,
                    "releasing S must preserve held C during the return climb"
                );
            }
            previous_action = action;
            app.update().expect("advance Tutorial01 return climb");
        }
        AppVirtualKeyboard::new(&mut app)
            .release(VirtualKeyCode::KeyC)
            .expect("release physical C on the cabin hill");
        assert!(
            app.engine
                .object_snapshot(clonk)
                .is_some_and(|object| object.position.x >= 558),
            "the GOLD-carrying CLNK must reach the cabin hill naturally"
        );
        advance_app_until(
            &mut app,
            "GOLD-carrying CLNK lands beside HUT2",
            60,
            |app| {
                app.engine
                    .object_snapshot(clonk)
                    .is_some_and(|object| object.action.name == "Walk")
            },
        );
        AppVirtualKeyboard::new(&mut app)
            .press(VirtualKeyCode::KeyZ)
            .expect("physical Z aligns GOLD-carrying CLNK with HUT2");
        advance_app_until(
            &mut app,
            "GOLD-carrying CLNK aligns with HUT2 entrance",
            60,
            |app| {
                app.engine
                    .object_snapshot(clonk)
                    .is_some_and(|object| object.position.x <= 570)
            },
        );
        {
            let mut keyboard = AppVirtualKeyboard::new(&mut app);
            keyboard
                .release(VirtualKeyCode::KeyZ)
                .expect("release physical Z at HUT2 entrance");
            keyboard
                .tap(VirtualKeyCode::KeyS)
                .expect("physical S enters HUT2 with GOLD");
        }
        advance_app_until(&mut app, "GOLD-carrying CLNK enters HUT2", 60, |app| {
            app.engine
                .object_snapshot(clonk)
                .is_some_and(|object| object.container == Some(hut))
        });

        advance_app_until(&mut app, "Tutorial01 selects Tutorial02", 240, |app| {
            app.engine.next_mission().path == r"Tutorial.c4f\Tutorial02.c4s"
        });
        advance_app_until(&mut app, "Tutorial01 reaches GameOver", 320, |app| {
            app.snapshot.game_over && app.game_over_dialog.is_some()
        });
        assert!(
            app.snapshot
                .round_results
                .fulfilled_goals
                .iter()
                .any(|goal| goal == "SCRG"),
            "Tutorial01 must fulfill its real SCRG before GameOver"
        );
        assert_eq!(
            app.engine.next_mission().path,
            r"Tutorial.c4f\Tutorial02.c4s"
        );
        // The typed C4GameMessage guard has a dedicated regression.
        app.snapshot.hud.messages.clear();
        app.render(&mut rendered)
            .expect("render Tutorial01 GameOver through GameApp");
    }

    #[test]
    #[cfg_attr(
        not(target_os = "macos"),
        ignore = "recording-host material order; required macOS CI job"
    )]
    fn app_virtual_keyboard_completes_real_tutorial02_route() {
        // The real window path maps keyboard-set-one X/X to Grab and S to Up.
        // While the Clonk pushes BALN, Jump'n'Run ControlUpdate follows held
        // S/X state and keeps DFA_PUSH attached to its moving solid mask; X/X
        // then falls through to UnGrab. Physical C/Z/D/A/S complete all three
        // LOAM bridges, recover FLAG and return it through HUT3's Put menu
        // (C4Object.cpp:3321-3338,3682-3724,4581-4652,5058-5114;
        // Tutorial02.c4s/Script.c:58-214).
        let mut app = real_tutorial_app(2, "Tutorial 2 virtual player");

        let clonk = app
            .engine
            .crew_cursor(app.local_owner)
            .expect("Tutorial02 selected CLNK");
        let balloon = app
            .engine
            .snapshot()
            .objects
            .into_iter()
            .find(|object| object.definition_id == "BALN")
            .expect("Tutorial02 BALN")
            .id;
        let hut = app_object_with_definition(&app, "HUT3").expect("Tutorial02 HUT3");
        let loam_menu_identification =
            serde_json::from_value(serde_json::json!({ "C4Id": "LMMS" }))
                .expect("LOAM menu identification deserializes");

        for _ in 0..160 {
            let clonk_ready = app
                .engine
                .object_snapshot(clonk)
                .is_some_and(|object| object.container.is_none() && object.action.name == "Walk");
            let balloon_ready = app
                .engine
                .object_snapshot(balloon)
                .is_some_and(|object| object.container.is_none());
            if clonk_ready && balloon_ready {
                break;
            }
            app.update().expect("advance Tutorial02 startup");
        }
        assert!(
            app.engine
                .object_snapshot(clonk)
                .is_some_and(|object| object.container.is_none() && object.action.name == "Walk"),
            "Tutorial02 CLNK exits the starting base through app frames"
        );

        {
            let mut keyboard = AppVirtualKeyboard::new(&mut app);
            keyboard.press(VirtualKeyCode::KeyX).expect("first physical X");
            keyboard
                .release(VirtualKeyCode::KeyX)
                .expect("release first physical X");
            keyboard
                .press(VirtualKeyCode::KeyX)
                .expect("second physical X");
        }
        for _ in 0..80 {
            if app.engine.object_snapshot(clonk).is_some_and(|object| {
                object.action.name == "Push" && object.action.target == Some(balloon)
            }) {
                break;
            }
            app.update().expect("advance physical Grab command");
        }
        let pushing = app.engine.object_snapshot(clonk).expect("CLNK after X/X");
        let balloon_before = app
            .engine
            .object_snapshot(balloon)
            .expect("BALN before lift");
        assert_eq!(
            (pushing.action.name.as_str(), pushing.action.target),
            ("Push", Some(balloon)),
            "physical X/X must grab BALN through GameApp"
        );
        let platform_delta_y = pushing.position.y - balloon_before.position.y;

        {
            let mut keyboard = AppVirtualKeyboard::new(&mut app);
            keyboard
                .release(VirtualKeyCode::KeyX)
                .expect("release second physical X");
            keyboard
                .press(VirtualKeyCode::KeyS)
                .expect("physical S while pushing BALN");
        }
        for lift_frame in 1..=20 {
            app.update()
                .expect("advance BALN lift through app scheduler");
            let clonk_now = app.engine.object_snapshot(clonk).expect("CLNK during lift");
            let balloon_now = app
                .engine
                .object_snapshot(balloon)
                .expect("BALN during lift");
            assert_eq!(
                (clonk_now.action.name.as_str(), clonk_now.action.target),
                ("Push", Some(balloon)),
                "DFA_PUSH must retain BALN on app lift frame {lift_frame}"
            );
            assert!(
                (clonk_now.position.y - balloon_now.position.y - platform_delta_y).abs() <= 1,
                "CLNK must remain on BALN's platform on app lift frame {lift_frame}; \
                 initial delta={platform_delta_y}, clonk={clonk_now:?}, balloon={balloon_now:?}"
            );
        }
        assert!(
            app.engine
                .object_snapshot(balloon)
                .expect("BALN after lift")
                .position
                .y
                < balloon_before.position.y,
            "physical S must lift BALN"
        );

        // The engine-only Tutorial02 replay deliberately joins a classic
        // player. This app fixture is the fresh-player Jump'n'Run default, so
        // release S (rather than a delayed X Single) supplies Stop through
        // BALN::ControlUpdate (C4Object.cpp:3327-3337;
        // Balloon.c4d/Script.c:60-78).
        for lift_frame in 21..=180 {
            if app
                .engine
                .object_snapshot(balloon)
                .is_some_and(|object| object.position.y <= 150)
            {
                break;
            }
            app.update().expect("advance BALN to flight corridor");
            let clonk_now = app
                .engine
                .object_snapshot(clonk)
                .expect("CLNK during remaining lift");
            let balloon_now = app
                .engine
                .object_snapshot(balloon)
                .expect("BALN during remaining lift");
            assert_eq!(
                (clonk_now.action.name.as_str(), clonk_now.action.target),
                ("Push", Some(balloon)),
                "DFA_PUSH must retain BALN on app lift frame {lift_frame}"
            );
            assert!(
                (clonk_now.position.y - balloon_now.position.y - platform_delta_y).abs() <= 1,
                "CLNK must remain on BALN's platform on app lift frame {lift_frame}"
            );
        }
        assert!(
            app.engine
                .object_snapshot(balloon)
                .is_some_and(|object| object.position.y <= 150),
            "held physical S must reach Tutorial02's flight corridor"
        );
        {
            let mut keyboard = AppVirtualKeyboard::new(&mut app);
            assert!(
                keyboard.player_control().control_style,
                "the isolated fresh player must use Jump'n'Run/AutoStop control"
            );
            keyboard
                .release(VirtualKeyCode::KeyS)
                .expect("release physical S in flight corridor");
            assert_eq!(
                keyboard
                    .engine()
                    .object_snapshot(balloon)
                    .expect("BALN after S release")
                    .command_direction,
                CommandDirection::Stop,
                "Jump'n'Run S release must stop vertical BALN control"
            );
        }

        // Stop intentionally retains BALN's wind-driven drift. Coast east
        // while continuously pinning the Push target and platform delta.
        for coast_frame in 1..=600 {
            if app
                .engine
                .object_snapshot(balloon)
                .is_some_and(|object| object.position.x >= 520)
            {
                break;
            }
            app.update().expect("coast BALN toward far island");
            let clonk_now = app
                .engine
                .object_snapshot(clonk)
                .expect("CLNK while coasting");
            let balloon_now = app
                .engine
                .object_snapshot(balloon)
                .expect("BALN while coasting");
            assert_eq!(
                (clonk_now.action.name.as_str(), clonk_now.action.target),
                ("Push", Some(balloon)),
                "DFA_PUSH must retain BALN on coast frame {coast_frame}"
            );
            assert!(
                (clonk_now.position.y - balloon_now.position.y - platform_delta_y).abs() <= 1,
                "CLNK must remain on BALN's platform on coast frame {coast_frame}; \
                 initial delta={platform_delta_y}, clonk={clonk_now:?}, balloon={balloon_now:?}"
            );
        }
        assert!(
            app.engine
                .object_snapshot(balloon)
                .is_some_and(|object| object.position.x >= 520),
            "stopped BALN must coast to the far-island longitude; balloon={:?}",
            app.engine.object_snapshot(balloon)
        );

        // In Jump'n'Run control, held physical X supplies Down immediately;
        // releasing X restores Stop. This intentionally does not use the
        // classic route's delayed DownSingle toggle.
        {
            let mut keyboard = AppVirtualKeyboard::new(&mut app);
            keyboard
                .press(VirtualKeyCode::KeyX)
                .expect("hold physical X to descend");
            assert_eq!(
                keyboard
                    .engine()
                    .object_snapshot(balloon)
                    .expect("BALN after X press")
                    .command_direction,
                CommandDirection::Down
            );
        }
        for descent_frame in 1..=240 {
            let in_gate = app.engine.object_snapshot(clonk).is_some_and(|object| {
                object.action.name == "Push"
                    && object.action.target == Some(balloon)
                    && (450..710).contains(&object.position.x)
                    && (250..320).contains(&object.position.y)
            });
            if in_gate {
                break;
            }
            app.update().expect("descend BALN toward far island");
            let clonk_now = app
                .engine
                .object_snapshot(clonk)
                .expect("CLNK while descending");
            let balloon_now = app
                .engine
                .object_snapshot(balloon)
                .expect("BALN while descending");
            assert_eq!(
                (clonk_now.action.name.as_str(), clonk_now.action.target),
                ("Push", Some(balloon)),
                "DFA_PUSH must retain BALN on descent frame {descent_frame}"
            );
            assert!(
                (clonk_now.position.y - balloon_now.position.y - platform_delta_y).abs() <= 1,
                "CLNK must remain on BALN's platform on descent frame {descent_frame}"
            );
        }
        assert!(
            app.engine.object_snapshot(clonk).is_some_and(|object| {
                object.action.name == "Push"
                    && object.action.target == Some(balloon)
                    && (450..710).contains(&object.position.x)
                    && (250..320).contains(&object.position.y)
            }),
            "held physical X must reach Tutorial02 Script3's far-island gate"
        );
        {
            let mut keyboard = AppVirtualKeyboard::new(&mut app);
            keyboard
                .release(VirtualKeyCode::KeyX)
                .expect("release physical X at far island");
            assert_eq!(
                keyboard
                    .engine()
                    .object_snapshot(balloon)
                    .expect("BALN after X release")
                    .command_direction,
                CommandDirection::Stop
            );
        }

        // Release does not clear C4Player::LastCom. Eleven app updates let the
        // prior X press leave C4DoubleClick's window before the instructed X/X;
        // otherwise the first new X could become the stale press's Double.
        for _ in 0..11 {
            app.update()
                .expect("wait out descent X double-click buffer");
            let clonk_now = app
                .engine
                .object_snapshot(clonk)
                .expect("CLNK while awaiting release prompt");
            let balloon_now = app
                .engine
                .object_snapshot(balloon)
                .expect("BALN while awaiting release prompt");
            assert_eq!(
                (clonk_now.action.name.as_str(), clonk_now.action.target),
                ("Push", Some(balloon))
            );
            assert!((clonk_now.position.y - balloon_now.position.y - platform_delta_y).abs() <= 1);
        }
        advance_app_until(&mut app, "Tutorial02 balloon-release prompt", 30, |app| {
            app_tutorial_message_contains(app, "Let go of the balloon")
        });
        {
            let mut keyboard = AppVirtualKeyboard::new(&mut app);
            keyboard
                .tap(VirtualKeyCode::KeyX)
                .expect("first physical X of ungrab double");
            keyboard
                .tap(VirtualKeyCode::KeyX)
                .expect("second physical X of ungrab double");
        }
        advance_app_until(&mut app, "CLNK lands on the far island", 100, |app| {
            app.engine.object_snapshot(clonk).is_some_and(|object| {
                object.action.name == "Walk"
                    && (450..710).contains(&object.position.x)
                    && (270..320).contains(&object.position.y)
            })
        });
        // The Jump'n'Run descent can land between the real material objects
        // instead of contacting one immediately like the classic route. Let
        // Script20 expose the actual next instruction, while still accepting
        // either natural FLAG/LOAM contact if the drift produced one.
        advance_app_until(
            &mut app,
            "Tutorial02 post-flight collectible prompt or contact",
            450,
            |app| {
                app_clonk_carries(app, clonk, "FLAG")
                    || app_clonk_carries(app, clonk, "LOAM")
                    || app_tutorial_message_contains(app, "Please drop the flag for now")
                    || app_tutorial_message_contains(app, "Pick up one of the loam chunks")
            },
        );

        // Contact may deterministically choose FLAG or one of four LOAM
        // objects. Script30 requires a real world Throw when FLAG wins; face
        // the island center with physical Z, then use physical A.
        if app_clonk_carries(&app, clonk, "FLAG") {
            advance_app_until(&mut app, "Tutorial02 temporary FLAG prompt", 450, |app| {
                app_tutorial_message_contains(app, "Please drop the flag for now")
            });
            AppVirtualKeyboard::new(&mut app)
                .press(VirtualKeyCode::KeyZ)
                .expect("physical Z faces island center");
            advance_app_until(
                &mut app,
                "CLNK faces left before throwing FLAG",
                30,
                |app| {
                    app.engine
                        .object_snapshot(clonk)
                        .is_some_and(|object| object.direction == Direction::Left)
                },
            );
            {
                let mut keyboard = AppVirtualKeyboard::new(&mut app);
                keyboard
                    .release(VirtualKeyCode::KeyZ)
                    .expect("release physical Z before FLAG throw");
                assert_eq!(
                    keyboard
                        .engine()
                        .object_snapshot(clonk)
                        .expect("CLNK before FLAG throw")
                        .direction,
                    Direction::Left
                );
                keyboard
                    .tap(VirtualKeyCode::KeyA)
                    .expect("physical world-A throws temporary FLAG");
            }
            advance_app_until(&mut app, "FLAG leaves CLNK inventory", 30, |app| {
                !app_clonk_carries(app, clonk, "FLAG")
            });
        }

        if !app_clonk_carries(&app, clonk, "LOAM") {
            advance_app_until(
                &mut app,
                "Tutorial02 LOAM pickup prompt or contact",
                450,
                |app| {
                    app_tutorial_message_contains(app, "Pick up one of the loam chunks")
                        || app_clonk_carries(app, clonk, "LOAM")
                },
            );
            if !app_clonk_carries(&app, clonk, "LOAM") {
                let direction_to_loam = |app: &GameApp| {
                    let clonk_x = app
                        .engine
                        .object_snapshot(clonk)
                        .expect("CLNK survives Tutorial02 landing")
                        .position
                        .x;
                    let loam_x = app
                        .engine
                        .snapshot()
                        .objects
                        .into_iter()
                        .filter(|object| object.definition_id == "LOAM")
                        .min_by_key(|object| (object.position.x - clonk_x).abs())
                        .expect("Tutorial02 keeps a loose LOAM chunk")
                        .position
                        .x;
                    if clonk_x < loam_x {
                        VirtualKeyCode::KeyC
                    } else {
                        VirtualKeyCode::KeyZ
                    }
                };
                let toward_first_object = direction_to_loam(&app);
                hold_app_key_until(
                    &mut app,
                    toward_first_object,
                    "CLNK naturally collects the first island object",
                    120,
                    |app| {
                        app_clonk_carries(app, clonk, "LOAM")
                            || app_clonk_carries(app, clonk, "FLAG")
                    },
                );
                if app_clonk_carries(&app, clonk, "FLAG") {
                    advance_app_until(&mut app, "Tutorial02 temporary FLAG prompt", 450, |app| {
                        app_tutorial_message_contains(app, "Please drop the flag for now")
                    });
                    AppVirtualKeyboard::new(&mut app)
                        .press(VirtualKeyCode::KeyC)
                        .expect("physical C faces away from the LOAM");
                    advance_app_until(
                        &mut app,
                        "CLNK faces right before throwing FLAG",
                        30,
                        |app| {
                            app.engine
                                .object_snapshot(clonk)
                                .is_some_and(|object| object.direction == Direction::Right)
                        },
                    );
                    {
                        let mut keyboard = AppVirtualKeyboard::new(&mut app);
                        keyboard
                            .release(VirtualKeyCode::KeyC)
                            .expect("release physical C before FLAG throw");
                        keyboard
                            .tap(VirtualKeyCode::KeyA)
                            .expect("physical world-A throws temporary FLAG away from LOAM");
                    }
                    advance_app_until(&mut app, "FLAG leaves CLNK inventory", 30, |app| {
                        !app_clonk_carries(app, clonk, "FLAG")
                    });
                }
                if !app_clonk_carries(&app, clonk, "LOAM") {
                    let toward_loam = direction_to_loam(&app);
                    hold_app_key_until(
                        &mut app,
                        toward_loam,
                        "CLNK naturally collects LOAM",
                        120,
                        |app| app_clonk_carries(app, clonk, "LOAM"),
                    );
                }
            }
        }
        assert!(app_clonk_carries(&app, clonk, "LOAM"));
        assert!(
            app_cursor_inventory_contains(&mut app, clonk, "LOAM"),
            "the collected LOAM must reach the cursor inventory presentation"
        );

        // Script40..42 moves the player to the left bridge position, observes
        // LMMS, and asks for its Diagonal left row. AutoStop Z release already
        // stops the CLNK, so no classic-only Down stop is injected here.
        advance_app_until(&mut app, "Tutorial02 move-left prompt", 240, |app| {
            app_tutorial_message_contains(app, "Now move to the very left edge")
        });
        AppVirtualKeyboard::new(&mut app)
            .press(VirtualKeyCode::KeyZ)
            .expect("physical Z walks to first bridge position");
        advance_app_until(&mut app, "Tutorial02 first bridge position", 120, |app| {
            app.engine.object_snapshot(clonk).is_some_and(|object| {
                object.action.name == "Walk" && (488..=490).contains(&object.position.x)
            })
        });
        AppVirtualKeyboard::new(&mut app)
            .release(VirtualKeyCode::KeyZ)
            .expect("release physical Z at first bridge position");
        advance_app_until(&mut app, "Tutorial02 double-Dig prompt", 180, |app| {
            app_tutorial_message_contains(app, "Press the 'dig' key twice quickly")
        });
        {
            let mut keyboard = AppVirtualKeyboard::new(&mut app);
            keyboard
                .tap(VirtualKeyCode::KeyD)
                .expect("first physical D for LOAM activation");
            keyboard
                .tap(VirtualKeyCode::KeyD)
                .expect("second physical D for LOAM activation");
        }
        advance_app_until(&mut app, "LOAM opens LMMS", 10, |app| {
            app.engine
                .cursor_object_menu(app.local_owner)
                .is_some_and(|(_, menu)| menu.identification == loam_menu_identification)
        });
        advance_app_until(&mut app, "Tutorial02 Diagonal left prompt", 180, |app| {
            app_tutorial_message_contains(app, "Select the option 'diagonal left'")
        });
        app.snapshot.hud.messages.clear();
        let mut rendered = vec![0_u8; 320 * 200 * 4];
        app.render(&mut rendered)
            .expect("render Tutorial02 LOAM construction menu");
        AppVirtualKeyboard::new(&mut app)
            .tap(VirtualKeyCode::KeyZ)
            .expect("physical Z selects Diagonal left");
        let selected = app
            .engine
            .cursor_object_menu(app.local_owner)
            .and_then(|(_, menu)| {
                usize::try_from(menu.selection)
                    .ok()
                    .map(|index| (menu, index))
            })
            .and_then(|(menu, index)| menu.items.get(index))
            .map(|item| item.caption.as_str());
        assert_eq!(selected, Some("Diagonal left"));
        let bridge_start = app
            .engine
            .object_snapshot(clonk)
            .expect("CLNK before first LOAM bridge")
            .position;
        AppVirtualKeyboard::new(&mut app)
            .tap(VirtualKeyCode::KeyA)
            .expect("physical A starts Diagonal left bridge");
        advance_app_until(&mut app, "CLNK starts first LOAM Bridge", 10, |app| {
            app.engine
                .object_snapshot(clonk)
                .is_some_and(|object| object.action.name == "Bridge")
        });
        assert_eq!(
            app.engine
                .object_snapshot(clonk)
                .expect("CLNK at first LOAM Bridge start")
                .position,
            bridge_start,
            "physical menu inputs must start Bridge without positioning the CLNK"
        );

        // C++ advances the moving UpLeft bridge first at Action.Time 6, then
        // moves sixteen (-1,-1) steps before returning to Walk
        // (C4Object.cpp:4581-4652,4755-4756).
        for _ in 0..6 {
            app.update().expect("advance first LOAM Bridge step");
        }
        let first_bridge_step = app
            .engine
            .object_snapshot(clonk)
            .expect("CLNK survives first LOAM Bridge step");
        assert_eq!(first_bridge_step.action.name, "Bridge");
        assert_eq!(first_bridge_step.action.time, 6);
        assert_eq!(
            first_bridge_step.action.data, 0x0064_0110,
            "LOAM must request C++'s moving, non-wall Earth bridge"
        );
        assert_eq!(
            first_bridge_step.position,
            Vector2::new(bridge_start.x - 1, bridge_start.y - 1)
        );
        advance_app_until(&mut app, "first UpLeft bridge completes", 114, |app| {
            app.engine
                .object_snapshot(clonk)
                .is_some_and(|object| object.action.name == "Walk")
        });
        let first_bridge_end = app
            .engine
            .object_snapshot(clonk)
            .expect("CLNK after first bridge")
            .position;
        assert_eq!(
            (
                first_bridge_end.x - bridge_start.x,
                first_bridge_end.y - bridge_start.y,
            ),
            (-16, -16)
        );
        advance_app_until(&mut app, "Tutorial02 three-bridge prompt", 180, |app| {
            app_tutorial_message_contains(app, "build three diagonal bridges")
        });

        // Cross back over bridge one for LOAM2, release C to stop, then return
        // with Z to its upper-left endpoint. Every fresh LMMS begins at row 7;
        // exactly one physical Z selects row 6, Diagonal left.
        AppVirtualKeyboard::new(&mut app)
            .press(VirtualKeyCode::KeyC)
            .expect("physical C crosses bridge one for LOAM2");
        advance_app_until(&mut app, "CLNK collects LOAM2", 220, |app| {
            app_clonk_carries(app, clonk, "LOAM")
        });
        AppVirtualKeyboard::new(&mut app)
            .release(VirtualKeyCode::KeyC)
            .expect("release physical C after LOAM2 pickup");
        AppVirtualKeyboard::new(&mut app)
            .press(VirtualKeyCode::KeyZ)
            .expect("physical Z returns to bridge-one endpoint");
        advance_app_until(
            &mut app,
            "CLNK returns to bridge-one endpoint",
            220,
            |app| {
                app.engine.object_snapshot(clonk).is_some_and(|object| {
                    object.container.is_none()
                        && object.action.name == "Walk"
                        && object.position.x <= first_bridge_end.x
                })
            },
        );
        AppVirtualKeyboard::new(&mut app)
            .release(VirtualKeyCode::KeyZ)
            .expect("release physical Z at bridge-one endpoint");
        {
            let mut keyboard = AppVirtualKeyboard::new(&mut app);
            keyboard
                .tap(VirtualKeyCode::KeyD)
                .expect("first physical D for LOAM2");
            keyboard
                .tap(VirtualKeyCode::KeyD)
                .expect("second physical D for LOAM2");
        }
        advance_app_until(&mut app, "LOAM2 opens LMMS", 20, |app| {
            app.engine
                .cursor_object_menu(app.local_owner)
                .is_some_and(|(_, menu)| menu.identification == loam_menu_identification)
        });
        assert_eq!(
            app.engine
                .cursor_object_menu(app.local_owner)
                .map(|(_, menu)| menu.selection),
            Some(7)
        );
        AppVirtualKeyboard::new(&mut app)
            .tap(VirtualKeyCode::KeyZ)
            .expect("physical Z selects LOAM2 Diagonal left");
        assert_eq!(
            app.engine
                .cursor_object_menu(app.local_owner)
                .and_then(|(_, menu)| {
                    usize::try_from(menu.selection)
                        .ok()
                        .and_then(|index| menu.items.get(index))
                })
                .map(|item| item.caption.as_str()),
            Some("Diagonal left")
        );
        let second_bridge_start = app
            .engine
            .object_snapshot(clonk)
            .expect("CLNK before second bridge")
            .position;
        assert!(
            (second_bridge_start.x - first_bridge_end.x).abs() <= 1
                && (second_bridge_start.y - first_bridge_end.y).abs() <= 1,
            "bridge two must continue bridge one; first_end={first_bridge_end:?}, second_start={second_bridge_start:?}"
        );
        AppVirtualKeyboard::new(&mut app)
            .tap(VirtualKeyCode::KeyA)
            .expect("physical A starts second bridge");
        advance_app_until(&mut app, "CLNK starts second Bridge", 10, |app| {
            app.engine
                .object_snapshot(clonk)
                .is_some_and(|object| object.action.name == "Bridge")
        });
        advance_app_until(&mut app, "second UpLeft bridge completes", 114, |app| {
            app.engine
                .object_snapshot(clonk)
                .is_some_and(|object| object.action.name == "Walk")
        });
        let second_bridge_end = app
            .engine
            .object_snapshot(clonk)
            .expect("CLNK after second bridge")
            .position;
        assert_eq!(
            (
                second_bridge_end.x - second_bridge_start.x,
                second_bridge_end.y - second_bridge_start.y,
            ),
            (-16, -16)
        );

        // Cross both spans for LOAM3. FLAG may be encountered first after the
        // earlier Script30 throw; face right with a physical C frame, throw it
        // using world A, finish Throw, then continue to adjacent LOAM.
        AppVirtualKeyboard::new(&mut app)
            .press(VirtualKeyCode::KeyC)
            .expect("physical C crosses two bridges for LOAM3");
        advance_app_until(&mut app, "CLNK reaches LOAM3 or FLAG", 260, |app| {
            app_clonk_carries(app, clonk, "LOAM") || app_clonk_carries(app, clonk, "FLAG")
        });
        AppVirtualKeyboard::new(&mut app)
            .release(VirtualKeyCode::KeyC)
            .expect("release physical C at far-island material");
        if app_clonk_carries(&app, clonk, "FLAG") {
            AppVirtualKeyboard::new(&mut app)
                .press(VirtualKeyCode::KeyC)
                .expect("physical C faces right before rethrowing FLAG");
            advance_app_until(
                &mut app,
                "CLNK faces right before rethrowing FLAG",
                30,
                |app| {
                    app.engine
                        .object_snapshot(clonk)
                        .is_some_and(|object| object.direction == Direction::Right)
                },
            );
            {
                let mut keyboard = AppVirtualKeyboard::new(&mut app);
                keyboard
                    .release(VirtualKeyCode::KeyC)
                    .expect("release physical C before rethrowing FLAG");
                assert_eq!(
                    keyboard
                        .engine()
                        .object_snapshot(clonk)
                        .expect("CLNK before rethrowing FLAG")
                        .direction,
                    Direction::Right
                );
                keyboard
                    .tap(VirtualKeyCode::KeyA)
                    .expect("physical world-A rethrows FLAG");
            }
            advance_app_until(&mut app, "recollected FLAG leaves CLNK", 30, |app| {
                !app_clonk_carries(app, clonk, "FLAG")
            });
            advance_app_until(&mut app, "CLNK finishes rethrowing FLAG", 30, |app| {
                app.engine
                    .object_snapshot(clonk)
                    .is_some_and(|object| object.action.name == "Walk")
            });
            AppVirtualKeyboard::new(&mut app)
                .press(VirtualKeyCode::KeyC)
                .expect("physical C continues to LOAM3");
            advance_app_until(&mut app, "CLNK collects LOAM3", 100, |app| {
                app_clonk_carries(app, clonk, "LOAM")
            });
            AppVirtualKeyboard::new(&mut app)
                .release(VirtualKeyCode::KeyC)
                .expect("release physical C after LOAM3 pickup");
        }
        assert!(app_clonk_carries(&app, clonk, "LOAM"));
        AppVirtualKeyboard::new(&mut app)
            .press(VirtualKeyCode::KeyZ)
            .expect("physical Z returns to bridge-two endpoint");
        advance_app_until(
            &mut app,
            "CLNK returns to bridge-two endpoint",
            260,
            |app| {
                app.engine.object_snapshot(clonk).is_some_and(|object| {
                    object.container.is_none()
                        && object.action.name == "Walk"
                        && object.position.x <= second_bridge_end.x
                })
            },
        );
        AppVirtualKeyboard::new(&mut app)
            .release(VirtualKeyCode::KeyZ)
            .expect("release physical Z at bridge-two endpoint");
        {
            let mut keyboard = AppVirtualKeyboard::new(&mut app);
            keyboard
                .tap(VirtualKeyCode::KeyD)
                .expect("first physical D for LOAM3");
            keyboard
                .tap(VirtualKeyCode::KeyD)
                .expect("second physical D for LOAM3");
        }
        advance_app_until(&mut app, "LOAM3 opens LMMS", 20, |app| {
            app.engine
                .cursor_object_menu(app.local_owner)
                .is_some_and(|(_, menu)| menu.identification == loam_menu_identification)
        });
        assert_eq!(
            app.engine
                .cursor_object_menu(app.local_owner)
                .map(|(_, menu)| menu.selection),
            Some(7)
        );
        AppVirtualKeyboard::new(&mut app)
            .tap(VirtualKeyCode::KeyZ)
            .expect("physical Z selects LOAM3 Diagonal left");
        assert_eq!(
            app.engine
                .cursor_object_menu(app.local_owner)
                .and_then(|(_, menu)| {
                    usize::try_from(menu.selection)
                        .ok()
                        .and_then(|index| menu.items.get(index))
                })
                .map(|item| item.caption.as_str()),
            Some("Diagonal left")
        );
        let third_bridge_start = app
            .engine
            .object_snapshot(clonk)
            .expect("CLNK before third bridge")
            .position;
        assert!(
            (third_bridge_start.x - second_bridge_end.x).abs() <= 1
                && (third_bridge_start.y - second_bridge_end.y).abs() <= 1,
            "bridge three must continue bridge two; second_end={second_bridge_end:?}, third_start={third_bridge_start:?}"
        );
        AppVirtualKeyboard::new(&mut app)
            .tap(VirtualKeyCode::KeyA)
            .expect("physical A starts third bridge");
        advance_app_until(&mut app, "CLNK starts third Bridge", 10, |app| {
            app.engine
                .object_snapshot(clonk)
                .is_some_and(|object| object.action.name == "Bridge")
        });
        advance_app_until(&mut app, "third UpLeft bridge completes", 114, |app| {
            app.engine
                .object_snapshot(clonk)
                .is_some_and(|object| object.action.name == "Walk")
        });
        let third_bridge_end = app
            .engine
            .object_snapshot(clonk)
            .expect("CLNK after third bridge")
            .position;
        assert_eq!(
            (
                third_bridge_end.x - third_bridge_start.x,
                third_bridge_end.y - third_bridge_start.y,
            ),
            (-16, -16)
        );
        let three_bridge_delta = (
            third_bridge_end.x - bridge_start.x,
            third_bridge_end.y - bridge_start.y,
        );
        assert!(
            (three_bridge_delta.0 + 48).abs() <= 2
                && (three_bridge_delta.1 + 48).abs() <= 2
                && (360..445).contains(&third_bridge_end.x)
                && (240..290).contains(&third_bridge_end.y),
            "three contiguous bridges must reach Script81; delta={three_bridge_delta:?}, end={third_bridge_end:?}"
        );
        advance_app_until(&mut app, "Tutorial02 close-enough prompt", 180, |app| {
            app_tutorial_message_contains(app, "close enough to jump")
        });

        // Walk back over all three bridges for FLAG. Four LOAM chunks exist
        // for three spans, so throw a spare left with world A before continuing
        // right to FLAG; inventory slot zero must then be FLAG.
        AppVirtualKeyboard::new(&mut app)
            .press(VirtualKeyCode::KeyC)
            .expect("physical C returns to far-island pickup");
        advance_app_until(&mut app, "CLNK reaches FLAG or spare LOAM", 420, |app| {
            app_clonk_carries(app, clonk, "FLAG") || app_clonk_carries(app, clonk, "LOAM")
        });
        AppVirtualKeyboard::new(&mut app)
            .release(VirtualKeyCode::KeyC)
            .expect("release physical C at final pickup");
        if app_clonk_carries(&app, clonk, "LOAM") {
            AppVirtualKeyboard::new(&mut app)
                .press(VirtualKeyCode::KeyZ)
                .expect("physical Z faces left before spare LOAM throw");
            advance_app_until(
                &mut app,
                "CLNK faces left before spare LOAM throw",
                30,
                |app| {
                    app.engine
                        .object_snapshot(clonk)
                        .is_some_and(|object| object.direction == Direction::Left)
                },
            );
            {
                let mut keyboard = AppVirtualKeyboard::new(&mut app);
                keyboard
                    .release(VirtualKeyCode::KeyZ)
                    .expect("release physical Z before spare LOAM throw");
                assert_eq!(
                    keyboard
                        .engine()
                        .object_snapshot(clonk)
                        .expect("CLNK before spare LOAM throw")
                        .direction,
                    Direction::Left
                );
                keyboard
                    .tap(VirtualKeyCode::KeyA)
                    .expect("physical world-A throws spare LOAM");
            }
            advance_app_until(&mut app, "spare LOAM leaves CLNK", 30, |app| {
                !app_clonk_carries(app, clonk, "LOAM")
            });
            advance_app_until(&mut app, "CLNK finishes throwing spare LOAM", 30, |app| {
                app.engine
                    .object_snapshot(clonk)
                    .is_some_and(|object| object.action.name == "Walk")
            });
        }
        if !app_clonk_carries(&app, clonk, "FLAG") {
            AppVirtualKeyboard::new(&mut app)
                .press(VirtualKeyCode::KeyC)
                .expect("physical C continues to FLAG");
            advance_app_until(&mut app, "CLNK collects FLAG", 180, |app| {
                app_clonk_carries(app, clonk, "FLAG")
            });
            AppVirtualKeyboard::new(&mut app)
                .release(VirtualKeyCode::KeyC)
                .expect("release physical C after FLAG pickup");
        }
        let flag = app
            .engine
            .object_snapshot(clonk)
            .and_then(|object| object.contents.first().copied())
            .expect("FLAG occupies CLNK inventory slot zero");
        assert_eq!(
            app.engine
                .object_snapshot(flag)
                .expect("carried FLAG")
                .definition_id,
            "FLAG"
        );

        // Keep physical Z held over all three bridges and both jumps home. S
        // release must preserve the held Left bit on each jump.
        AppVirtualKeyboard::new(&mut app)
            .press(VirtualKeyCode::KeyZ)
            .expect("physical Z starts FLAG return");
        advance_app_until(
            &mut app,
            "FLAG-carrying CLNK reaches bridge endpoint",
            420,
            |app| {
                app.engine.object_snapshot(clonk).is_some_and(|object| {
                    object.action.name == "Walk" && object.position.x <= third_bridge_end.x
                })
            },
        );
        let first_return_jump_frame = app.engine.frame();
        {
            let mut keyboard = AppVirtualKeyboard::new(&mut app);
            keyboard
                .tap(VirtualKeyCode::KeyS)
                .expect("physical S jumps to center island");
            assert_ne!(
                keyboard.player_control().pressed_coms & (1 << clonk_engine::COM_LEFT),
                0,
                "first S release must preserve held Z"
            );
        }
        advance_app_until(
            &mut app,
            "FLAG-carrying CLNK lands on center island",
            140,
            |app| {
                app.engine.object_snapshot(clonk).is_some_and(|object| {
                    object.action.name == "Walk" && (290..390).contains(&object.position.x)
                })
            },
        );
        advance_app_until(
            &mut app,
            "CLNK reaches center-island jump edge",
            120,
            |app| {
                app.engine
                    .object_snapshot(clonk)
                    .is_some_and(|object| object.action.name == "Walk" && object.position.x <= 310)
            },
        );
        // The Jump'n'Run center hop can finish inside C4DoubleClick's ten-tick
        // window. Keep Z held, but do not let the second physical S become an
        // ignored COM_Up_D and turn the intended jump into a walk-off fall.
        while app.engine.frame().saturating_sub(first_return_jump_frame) <= 10 {
            app.update()
                .expect("wait out first return S double-click buffer");
            assert_eq!(
                app.engine
                    .object_snapshot(clonk)
                    .expect("CLNK waits at center-island jump edge")
                    .action
                    .name,
                "Walk"
            );
        }
        let second_jump_start = app
            .engine
            .object_snapshot(clonk)
            .expect("CLNK at center-island jump edge")
            .position;
        {
            let mut keyboard = AppVirtualKeyboard::new(&mut app);
            keyboard
                .tap(VirtualKeyCode::KeyS)
                .expect("physical S jumps to home island");
            assert_ne!(
                keyboard.player_control().pressed_coms & (1 << clonk_engine::COM_LEFT),
                0,
                "second S release must preserve held Z"
            );
        }
        app.update().expect("execute second physical S jump");
        let launched = app
            .engine
            .object_snapshot(clonk)
            .expect("CLNK after second S executes");
        assert_eq!(launched.action.name, "Jump");
        assert!(
            launched.velocity.y < 0,
            "second physical S must launch upward; clonk={launched:?}"
        );
        for _ in 0..160 {
            if app
                .engine
                .object_snapshot(clonk)
                .is_some_and(|object| object.action.name == "Walk" && object.position.x <= 230)
            {
                break;
            }
            app.update().expect("advance second FLAG return jump");
        }
        let home_landing = app
            .engine
            .object_snapshot(clonk)
            .expect("FLAG-carrying CLNK after second return jump");
        assert!(
            home_landing.action.name == "Walk" && home_landing.position.x <= 230,
            "FLAG-carrying CLNK must land from {second_jump_start:?}; clonk={home_landing:?}"
        );
        let hut_position = app
            .engine
            .object_snapshot(hut)
            .expect("HUT3 survives Tutorial02")
            .position;
        advance_app_until(
            &mut app,
            "FLAG-carrying CLNK reaches HUT3 entrance",
            160,
            |app| {
                app.engine.object_snapshot(clonk).is_some_and(|object| {
                    object.action.name == "Walk"
                        && (hut_position.x + 2..hut_position.x + 19).contains(&object.position.x)
                        && (hut_position.y + 4..hut_position.y + 25).contains(&object.position.y)
                })
            },
        );
        AppVirtualKeyboard::new(&mut app)
            .release(VirtualKeyCode::KeyZ)
            .expect("release physical Z at HUT3 entrance");
        assert_eq!(
            app.engine
                .object_snapshot(hut)
                .expect("HUT3 before FLAG return")
                .base,
            -1,
            "HUT3 must not be a base while FlyBase FLAG is absent"
        );
        AppVirtualKeyboard::new(&mut app)
            .tap(VirtualKeyCode::KeyS)
            .expect("physical S enters HUT3");
        advance_app_until(&mut app, "FLAG-carrying CLNK enters HUT3", 80, |app| {
            app.engine
                .object_snapshot(clonk)
                .is_some_and(|object| object.container == Some(hut))
        });

        // AutoContextMenu inserts Put first for the contained FLAG. Physical A
        // is therefore MenuEnter/Put, not a direct contained Throw
        // (C4Player.cpp:1502-1513; C4ObjectMenu.cpp:335-359).
        advance_app_until(&mut app, "HUT3 auto-context Put row", 20, |app| {
            app.engine
                .cursor_object_menu(app.local_owner)
                .is_some_and(|(_, menu)| {
                    menu.selection == 0
                        && menu.items.first().is_some_and(|item| item.caption == "Put")
                })
        });
        advance_app_until(&mut app, "Tutorial02 FLAG Put prompt", 240, |app| {
            app_tutorial_message_contains(app, "Press 'throw' to put the flag")
        });
        AppVirtualKeyboard::new(&mut app)
            .tap(VirtualKeyCode::KeyA)
            .expect("physical A puts FLAG into HUT3");
        advance_app_until(&mut app, "FLAG enters HUT3", 80, |app| {
            app.engine
                .object_snapshot(flag)
                .is_some_and(|object| object.container == Some(hut))
        });
        advance_app_until(&mut app, "HUT3 restores the player base", 80, |app| {
            app.engine
                .object_snapshot(hut)
                .is_some_and(|object| object.base == app.local_owner)
        });
        advance_app_until(&mut app, "Tutorial02 selects Tutorial03", 180, |app| {
            app.engine.next_mission().path == r"Tutorial.c4f\Tutorial03.c4s"
        });
        advance_app_until(&mut app, "Tutorial02 reaches GameOver", 320, |app| {
            app.snapshot.game_over && app.game_over_dialog.is_some()
        });
        assert!(
            app.snapshot
                .round_results
                .fulfilled_goals
                .iter()
                .any(|goal| goal == "SCRG"),
            "Tutorial02 must fulfill SCRG before GameOver"
        );
        assert_eq!(
            app.engine.next_mission().path,
            r"Tutorial.c4f\Tutorial03.c4s"
        );
        // Typed C4GameMessage rejection has its own regression; isolate this
        // GameOver-render assertion from that unported overlay.
        app.snapshot.hud.messages.clear();
        app.render(&mut rendered)
            .expect("render Tutorial02 GameOver through GameApp");
    }

    #[test]
    fn app_virtual_keyboard_completes_real_tutorial03_route() {
        // Tutorial03 teaches the permanent building-menu sequence after the
        // Clonk enters HUT3: C4MN_Context=14 exposes Contents/Buy/Sell/Info/Exit
        // before the player selects Buy (Tutorial03.c4s/Script.c:106-145;
        // C4Object.cpp:1919-1980,3034-3048; C4ObjectMenu.cpp:361-427). Drive
        // C then S through GameApp's physical keyboard boundary so this also
        // covers the real key map and ObjectComUp entrance path.
        let mut app = real_tutorial_app(3, "Tutorial 3 app virtual player");
        assert!(
            !app.mouse_control,
            "Tutorial03 DisableMouse=1 must suppress player mouse control and the menu close X like C++ (C4Player.cpp:1907-1912; C4Menu.cpp:1270-1276)"
        );
        assert!(
            !app.option_flags(app.local_owner).mouse_shown,
            "DisableMouse must remove the in-game Options entry like C++ (C4MainMenu.cpp:563-571)"
        );

        let clonk = app
            .engine
            .crew_cursor(app.local_owner)
            .expect("Tutorial03 selected CLNK");
        let hut = app
            .engine
            .snapshot()
            .objects
            .into_iter()
            .find(|object| object.definition_id == "HUT3")
            .expect("Tutorial03 HUT3")
            .id;
        for _ in 0..360 {
            let ready = app
                .engine
                .object_snapshot(hut)
                .is_some_and(|object| object.base == app.local_owner)
                && app.engine.object_snapshot(clonk).is_some_and(|object| {
                    object.container.is_none() && object.action.name == "Walk"
                });
            if ready {
                break;
            }
            app.update().expect("advance Tutorial03 startup");
        }
        assert!(
            app.engine
                .object_snapshot(hut)
                .is_some_and(|object| { object.base == app.local_owner }),
            "Tutorial03 ready HUT3 must become the local player's base"
        );
        assert!(
            app.engine.object_snapshot(clonk).is_some_and(|object| {
                object.container.is_none() && object.action.name == "Walk"
            }),
            "Tutorial03 CLNK must exit the starting base through app frames"
        );

        {
            let mut keyboard = AppVirtualKeyboard::new(&mut app);
            keyboard
                .press(VirtualKeyCode::KeyC)
                .expect("physical C walks right");
        }
        for _ in 0..40 {
            let at_entrance = app
                .engine
                .object_snapshot(hut)
                .zip(app.engine.object_snapshot(clonk))
                .is_some_and(|(hut, clonk)| clonk.position.x >= hut.position.x + 2);
            if at_entrance {
                break;
            }
            app.update().expect("walk to HUT3 entrance");
        }
        {
            let mut keyboard = AppVirtualKeyboard::new(&mut app);
            keyboard
                .release(VirtualKeyCode::KeyC)
                .expect("release physical C");
            keyboard
                .press(VirtualKeyCode::KeyS)
                .expect("physical S enters HUT3");
            keyboard
                .release(VirtualKeyCode::KeyS)
                .expect("release physical S");
        }
        for _ in 0..40 {
            if app
                .engine
                .object_snapshot(clonk)
                .is_some_and(|object| object.container == Some(hut))
            {
                break;
            }
            app.update().expect("advance HUT3 entrance command");
        }
        assert_eq!(
            app.engine
                .object_snapshot(clonk)
                .expect("CLNK after physical S")
                .container,
            Some(hut),
            "physical C/S route must enter HUT3 through GameApp"
        );

        for _ in 0..20 {
            if app.engine.cursor_object_menu(app.local_owner).is_some() {
                break;
            }
            app.update().expect("wait for HUT3 auto-context menu");
        }
        let (_, menu) = app
            .engine
            .cursor_object_menu(app.local_owner)
            .expect("HUT3 exposes its app-visible auto-context menu");
        let context_identification = serde_json::from_value(serde_json::json!({ "Int": 14 }))
            .expect("integer menu identification deserializes");
        let buy_identification = serde_json::from_value(serde_json::json!({ "Int": 4 }))
            .expect("buy menu identification deserializes");
        let contents_identification = serde_json::from_value(serde_json::json!({ "Int": 18 }))
            .expect("contents menu identification deserializes");
        assert_eq!(menu.identification, context_identification);
        assert_eq!(
            menu.caption, "Cabin",
            "C4Def::Load must replace HUT3's DefCore fallback with Names.txt US localization (C4Def.cpp:635-639)"
        );
        assert_eq!(
            menu.items
                .iter()
                .map(|item| item.caption.as_str())
                .collect::<Vec<_>>(),
            vec!["Contents", "Buy", "Sell", "Info", "Exit"]
        );
        let mut rendered = vec![0_u8; 320 * 200 * 4];
        app.render(&mut rendered)
            .expect("render Tutorial03 context menu through the app");
        advance_app_until(&mut app, "Tutorial03 Buy-menu prompt", 240, |app| {
            app_tutorial_message_contains(app, "Select option 'Buy'")
        });

        // Physical X is the classic down control and physical A is Throw;
        // while a menu is open C4Player::InCom translates them to MenuDown
        // and MenuEnter (C4Player.cpp:1502-1513). This is the exact Tutorial03
        // input path from Context -> Buy, without mutating menu state.
        {
            let mut keyboard = AppVirtualKeyboard::new(&mut app);
            keyboard
                .press(VirtualKeyCode::KeyX)
                .expect("physical X selects Buy");
            keyboard
                .release(VirtualKeyCode::KeyX)
                .expect("release physical X");
            keyboard
                .press(VirtualKeyCode::KeyA)
                .expect("physical A enters Buy");
            keyboard
                .release(VirtualKeyCode::KeyA)
                .expect("release physical A");
        }
        for _ in 0..20 {
            let buy_menu_open = app
                .engine
                .cursor_object_menu(app.local_owner)
                .is_some_and(|(_, menu)| menu.identification == buy_identification);
            if buy_menu_open {
                break;
            }
            app.update().expect("advance physical Buy selection");
        }
        let (_, buy_menu) = app
            .engine
            .cursor_object_menu(app.local_owner)
            .expect("physical X/A opens Tutorial03 Buy menu");
        assert_eq!(buy_menu.identification, buy_identification);
        assert_eq!(
            buy_menu.title_symbol,
            clonk_engine::ObjectMenuSymbol::Buy {
                owner: app
                    .engine
                    .object_snapshot(hut)
                    .expect("Tutorial03 HUT3 remains active")
                    .owner,
            },
            "C4MN_Buy title uses the contained building owner (C4Object.cpp:1919-1928)"
        );
        assert_eq!(
            buy_menu.extra,
            clonk_engine::ObjectMenuExtra::Value,
            "C4MN_Buy exposes selected value in its footer"
        );
        assert_eq!(
            buy_menu
                .items
                .iter()
                .map(|item| (item.caption.as_str(), item.count, item.value))
                .collect::<Vec<_>>(),
            vec![("Buy Lorry", 1, Some(20))]
        );
        assert_eq!(
            buy_menu.items[0].info_caption,
            "Useful to transport large amounts of material. Holds up to 50 items.",
            "C4ObjectMenu::Refill passes each Buy definition's localized description to C4MenuItem (C4ObjectMenu.cpp:219-233)"
        );
        app.snapshot.hud.messages.clear();
        app.render(&mut rendered)
            .expect("render Tutorial03 Buy menu through the app");
        advance_app_until(&mut app, "Tutorial03 buy-LORY prompt", 240, |app| {
            app_tutorial_message_contains(app, "Buy a lorry")
        });

        // Buy the selected LORY with physical A/Throw. C++ leaves the
        // permanent Buy menu open and refills its C4IDList row at count zero
        // after C4Player::Buy consumes wealth and creates the object inside
        // the base (C4Command.cpp:2005-2035; C4ObjectMenu.cpp:124-129,207-237).
        {
            let mut keyboard = AppVirtualKeyboard::new(&mut app);
            keyboard.tap(VirtualKeyCode::KeyA).expect("buy selected LORY");
        }
        for _ in 0..20 {
            let bought = app
                .engine
                .snapshot()
                .objects
                .into_iter()
                .any(|object| object.definition_id == "LORY" && object.container == Some(hut));
            if bought
                && app
                    .engine
                    .player(app.local_owner)
                    .is_some_and(|player| player.wealth() == 5)
            {
                break;
            }
            app.update().expect("advance physical LORY purchase");
        }
        let lorry = app
            .engine
            .snapshot()
            .objects
            .into_iter()
            .find(|object| object.definition_id == "LORY")
            .expect("physical A buys Tutorial03 LORY")
            .id;
        assert_eq!(
            app.engine
                .object_snapshot(lorry)
                .expect("bought LORY")
                .container,
            Some(hut)
        );
        let player = app.engine.player(app.local_owner).expect("local player");
        assert_eq!(player.wealth(), 5);
        assert_eq!(player.home_base_material().get("LORY"), Some(&0));
        let (_, buy_menu) = app
            .engine
            .cursor_object_menu(app.local_owner)
            .expect("permanent Buy menu remains after purchase");
        assert_eq!(buy_menu.identification, buy_identification);
        assert_eq!(buy_menu.items[0].count, 0);
        advance_app_until(&mut app, "Tutorial03 close-Buy prompt", 240, |app| {
            app_tutorial_message_contains(app, "close the buy menu")
        });

        // D closes Buy back to auto-context; A activates its first Contents
        // row, then A activates LORY out of HUT3. These remain ordinary
        // physical controls translated by C4Player::InCom while a menu is
        // active (C4Player.cpp:1502-1513; C4ObjectMenu.cpp:279-326).
        {
            let mut keyboard = AppVirtualKeyboard::new(&mut app);
            keyboard.tap(VirtualKeyCode::KeyD).expect("close Buy menu");
        }
        for _ in 0..20 {
            if app
                .engine
                .cursor_object_menu(app.local_owner)
                .is_some_and(|(_, menu)| menu.identification == context_identification)
            {
                break;
            }
            app.update().expect("restore context after Buy");
        }
        advance_app_until(&mut app, "Tutorial03 Contents prompt", 240, |app| {
            app_tutorial_message_contains(app, "select 'Contents'")
        });
        {
            let mut keyboard = AppVirtualKeyboard::new(&mut app);
            keyboard.tap(VirtualKeyCode::KeyA).expect("open Contents");
        }
        for _ in 0..20 {
            if app
                .engine
                .cursor_object_menu(app.local_owner)
                .is_some_and(|(_, menu)| menu.identification == contents_identification)
            {
                break;
            }
            app.update().expect("open HUT3 Contents");
        }
        let (_, contents_menu) = app
            .engine
            .cursor_object_menu(app.local_owner)
            .expect("physical D/A opens Contents menu");
        assert_eq!(
            contents_menu
                .items
                .iter()
                .map(|item| (item.caption.as_str(), item.item_id.as_str()))
                .collect::<Vec<_>>(),
            vec![("Activate Lorry", "LORY")]
        );
        // Typed C4GameMessage rejection has its own regression; isolate this
        // Contents-render assertion from that unported overlay.
        app.snapshot.hud.messages.clear();
        app.render(&mut rendered)
            .expect("render Tutorial03 Contents menu through the app");
        advance_app_until(&mut app, "Tutorial03 activate-LORY prompt", 240, |app| {
            app_tutorial_message_contains(app, "Activate the lorry")
        });
        {
            let mut keyboard = AppVirtualKeyboard::new(&mut app);
            keyboard.tap(VirtualKeyCode::KeyA).expect("activate LORY");
        }
        for _ in 0..40 {
            if app
                .engine
                .object_snapshot(lorry)
                .is_some_and(|object| object.container.is_none())
            {
                break;
            }
            app.update().expect("exit LORY from HUT3");
        }
        assert!(
            app.engine
                .object_snapshot(lorry)
                .is_some_and(|object| object.container.is_none()),
            "Contents activation must exit LORY from HUT3"
        );
        advance_app_until(&mut app, "Tutorial03 leave-HUT3 prompt", 240, |app| {
            app_tutorial_message_contains(app, "exit the hut")
        });

        // Close Contents, then close the restored context menu. Its C++ close
        // command is Exit, so the tutorial-taught two physical D presses exit
        // the building without menu-selection shortcuts (C4Object.cpp:
        // 2044-2062; C4Menu.cpp:317-331; Tutorial03.c4s/Script.c:191-200).
        {
            let mut keyboard = AppVirtualKeyboard::new(&mut app);
            keyboard.tap(VirtualKeyCode::KeyD).expect("close Contents");
        }
        for _ in 0..20 {
            if app
                .engine
                .cursor_object_menu(app.local_owner)
                .is_some_and(|(_, menu)| menu.identification == context_identification)
            {
                break;
            }
            app.update().expect("restore context after Contents");
        }
        {
            let mut keyboard = AppVirtualKeyboard::new(&mut app);
            keyboard
                .tap(VirtualKeyCode::KeyD)
                .expect("close context through Exit command");
        }
        for _ in 0..40 {
            if app
                .engine
                .object_snapshot(clonk)
                .is_some_and(|object| object.container.is_none())
            {
                break;
            }
            app.update().expect("exit CLNK from HUT3");
        }
        assert!(
            app.engine
                .object_snapshot(clonk)
                .is_some_and(|object| object.container.is_none()),
            "physical D/D route must exit CLNK from HUT3"
        );

        // Once both objects are outside, Tutorial03 teaches the complete real
        // production route: LORY to SAWM, TRE2 through SAWM, ORE1 into LORY,
        // then LORY into FNDR. The engine replay uses the same fresh-player
        // Jump'n'Run/AutoContext preferences, so retain its physical bounds
        // exactly at GameApp::handle_key (Tutorial03.c4s/Script.c:204-284;
        // C4Object.cpp:3573-3740; C4ObjectCom.cpp:247-278).
        advance_app_until(
            &mut app,
            "Tutorial03 closes HUT3's cursor menu",
            20,
            |app| app.engine.cursor_object_menu(app.local_owner).is_none(),
        );
        assert!(
            app.engine.cursor_object_menu(app.local_owner).is_none(),
            "no engine cursor menu may intercept the first world X"
        );
        assert!(
            !app.menu_controls_active_for(app.local_owner),
            "no app menu may intercept the first world X"
        );
        let sawmill = app_object_with_definition(&app, "SAWM").expect("Tutorial03 SAWM");
        let foundry = app_object_with_definition(&app, "FNDR").expect("Tutorial03 FNDR");
        let tree = app
            .engine
            .snapshot()
            .objects
            .into_iter()
            .filter(|object| object.definition_id == "TRE2")
            .min_by_key(|object| (object.position.x - 167).abs())
            .expect("Tutorial03 first full TRE2 near x=167")
            .id;

        advance_app_until(&mut app, "Tutorial03 LORY grab prompt", 180, |app| {
            app_tutorial_message_contains(app, "once to grab the lorry")
        });
        AppVirtualKeyboard::new(&mut app)
            .tap(VirtualKeyCode::KeyX)
            .expect("physical X grabs LORY");
        advance_app_until(&mut app, "physical X grabs LORY", 40, |app| {
            app.engine.object_snapshot(clonk).is_some_and(|object| {
                object.action.name == "Push" && object.action.target == Some(lorry)
            })
        });
        AppVirtualKeyboard::new(&mut app)
            .press(VirtualKeyCode::KeyZ)
            .expect("physical Z pushes LORY left");
        advance_app_until(&mut app, "LORY reaches the sawmill chute", 240, |app| {
            app.engine.object_snapshot(lorry).is_some_and(|lorry| {
                (194..=218).contains(&lorry.position.x) && (257..=277).contains(&lorry.position.y)
            })
        });
        AppVirtualKeyboard::new(&mut app)
            .release(VirtualKeyCode::KeyZ)
            .expect("release physical Z at the sawmill chute");
        advance_app_until(&mut app, "Tutorial03 LORY release prompt", 180, |app| {
            app_tutorial_message_contains(app, "again to let go of the lorry")
        });
        AppVirtualKeyboard::new(&mut app)
            .tap(VirtualKeyCode::KeyX)
            .expect("physical X releases LORY");
        advance_app_until(&mut app, "physical X releases LORY", 40, |app| {
            app.engine
                .object_snapshot(clonk)
                .is_some_and(|object| object.action.name == "Walk")
        });

        advance_app_until(&mut app, "Tutorial03 first-tree prompt", 180, |app| {
            app_tutorial_message_contains(app, "first tree on the left")
        });
        AppVirtualKeyboard::new(&mut app)
            .press(VirtualKeyCode::KeyZ)
            .expect("physical Z walks to TRE2");
        advance_app_until(
            &mut app,
            "CLNK stands inside the first TRE2 shape",
            120,
            |app| {
                app.engine
                    .object_snapshot(tree)
                    .zip(app.engine.object_snapshot(clonk))
                    .is_some_and(|(tree, clonk)| {
                        (tree.position.x - 20..=tree.position.x + 20).contains(&clonk.position.x)
                            && (tree.position.y - 28..=tree.position.y + 28)
                                .contains(&clonk.position.y)
                    })
            },
        );
        AppVirtualKeyboard::new(&mut app)
            .release(VirtualKeyCode::KeyZ)
            .expect("release physical Z at TRE2");
        advance_app_until(&mut app, "Tutorial03 double-Dig prompt", 180, |app| {
            app_tutorial_message_contains(app, "twice quickly to start chopping")
        });

        // Two immediate physical D taps synthesize COM_Dig_D and must choose
        // Chop, not Script20's intentional too-slow Dig recovery branch
        // (C4Player.cpp:1522-1536; Tutorial03.c4s/Script.c:36-63).
        {
            let mut keyboard = AppVirtualKeyboard::new(&mut app);
            keyboard.tap(VirtualKeyCode::KeyD).expect("first physical D");
            keyboard.tap(VirtualKeyCode::KeyD).expect("second physical D");
        }
        advance_app_until(&mut app, "physical D/D starts Chop", 80, |app| {
            app.engine
                .object_snapshot(clonk)
                .is_some_and(|object| object.action.name == "Chop")
        });
        advance_app_until(&mut app, "TRE2 is chopped into a vehicle", 800, |app| {
            app.engine
                .object_snapshot(tree)
                .is_some_and(|object| object.category & clonk_engine::CATEGORY_VEHICLE != 0)
        });
        advance_app_until(&mut app, "Tutorial03 felled-tree grab prompt", 180, |app| {
            app_tutorial_message_contains(app, "grab the felled tree")
                && app
                    .engine
                    .object_snapshot(clonk)
                    .is_some_and(|object| object.action.name == "Walk")
        });
        AppVirtualKeyboard::new(&mut app)
            .tap(VirtualKeyCode::KeyX)
            .expect("physical X grabs felled TRE2");
        advance_app_until(&mut app, "physical X grabs felled TRE2", 80, |app| {
            app.engine.object_snapshot(clonk).is_some_and(|object| {
                object.action.name == "Push" && object.action.target == Some(tree)
            })
        });
        advance_app_until(&mut app, "Tutorial03 SAWM tree prompt", 180, |app| {
            app_tutorial_message_contains(app, "Push the tree over to the sawmill")
        });
        AppVirtualKeyboard::new(&mut app)
            .press(VirtualKeyCode::KeyC)
            .expect("physical C pushes TRE2 right");
        advance_app_until(&mut app, "TRE2 reaches the SAWM gate", 240, |app| {
            app.engine.object_snapshot(tree).is_some_and(|tree| {
                (239..=259).contains(&tree.position.x) && (254..=279).contains(&tree.position.y)
            })
        });
        AppVirtualKeyboard::new(&mut app)
            .release(VirtualKeyCode::KeyC)
            .expect("release physical C at SAWM");
        advance_app_until(&mut app, "Tutorial03 SAWM Up prompt", 180, |app| {
            app_tutorial_message_contains(app, "press 'up' to push it into the sawmill")
        });
        AppVirtualKeyboard::new(&mut app)
            .tap(VirtualKeyCode::KeyS)
            .expect("physical S pushes TRE2 into SAWM");
        advance_app_until(&mut app, "SAWM consumes TRE2", 240, |app| {
            app.engine.object_snapshot(tree).is_none()
        });
        advance_app_until(&mut app, "SAWM's five WOOD enter LORY", 600, |app| {
            app.engine
                .snapshot()
                .objects
                .into_iter()
                .filter(|object| object.definition_id == "WOOD" && object.container == Some(lorry))
                .count()
                >= 5
        });
        assert!(
            app.engine.object_snapshot(sawmill).is_some(),
            "SAWM must survive after consuming TRE2"
        );

        advance_app_until(&mut app, "Tutorial03 creates ORE1", 180, |app| {
            app_tutorial_message_contains(app, "dig out the chunk of ore")
                && app_object_with_definition(app, "ORE1").is_some()
        });
        let ore = app_object_with_definition(&app, "ORE1").expect("Tutorial03 ORE1");
        AppVirtualKeyboard::new(&mut app)
            .press(VirtualKeyCode::KeyC)
            .expect("physical C walks to ORE1");
        advance_app_until(&mut app, "CLNK reaches the ORE1 digging face", 600, |app| {
            app.engine
                .object_snapshot(clonk)
                .is_some_and(|object| object.position.x >= 480)
        });
        AppVirtualKeyboard::new(&mut app)
            .release(VirtualKeyCode::KeyC)
            .expect("release physical C at ORE1");

        // A single D is buffered to COM_Dig_S after C4DoubleClick. Wait for
        // Dig before pressing X+C so another physical command cannot flush the
        // pending single early (C4Player.cpp:1215-1229,1522-1531).
        AppVirtualKeyboard::new(&mut app)
            .tap(VirtualKeyCode::KeyD)
            .expect("physical D starts ORE1 dig");
        advance_app_until(&mut app, "CLNK starts digging toward ORE1", 30, |app| {
            app.engine
                .object_snapshot(clonk)
                .is_some_and(|object| object.action.name == "Dig")
        });
        {
            let mut keyboard = AppVirtualKeyboard::new(&mut app);
            keyboard
                .press(VirtualKeyCode::KeyX)
                .expect("physical X supplies Dig down");
            keyboard
                .press(VirtualKeyCode::KeyC)
                .expect("physical C supplies Dig right");
        }
        advance_app_until(&mut app, "real dig tunnel collects ORE1", 300, |app| {
            app.engine
                .object_snapshot(ore)
                .is_some_and(|object| object.container == Some(clonk))
        });
        {
            let mut keyboard = AppVirtualKeyboard::new(&mut app);
            keyboard
                .release(VirtualKeyCode::KeyX)
                .expect("release physical X after ORE1 pickup");
            keyboard
                .release(VirtualKeyCode::KeyC)
                .expect("release physical C after ORE1 pickup");
        }
        advance_app_until(&mut app, "ORE1-carrying CLNK finishes Dig", 80, |app| {
            app.engine
                .object_snapshot(clonk)
                .is_some_and(|object| object.action.name == "Walk")
        });

        advance_app_until(&mut app, "Tutorial03 ORE1 throw prompt", 180, |app| {
            app_tutorial_message_contains(app, "Throw the chunk of ore into the lorry")
        });
        AppVirtualKeyboard::new(&mut app)
            .press(VirtualKeyCode::KeyZ)
            .expect("physical Z returns to LORY");
        advance_app_until(&mut app, "CLNK reaches LORY's right side", 800, |app| {
            app.engine
                .object_snapshot(clonk)
                .zip(app.engine.object_snapshot(lorry))
                .is_some_and(|(clonk, lorry)| {
                    clonk.position.x >= lorry.position.x + 40
                        && clonk.position.x <= lorry.position.x + 42
                })
        });
        AppVirtualKeyboard::new(&mut app)
            .release(VirtualKeyCode::KeyZ)
            .expect("release physical Z beside LORY");
        assert!(
            app.engine.cursor_object_menu(app.local_owner).is_none(),
            "no engine cursor menu may intercept the world A throw"
        );
        assert!(
            !app.menu_controls_active_for(app.local_owner),
            "no app menu may intercept the world A throw"
        );
        AppVirtualKeyboard::new(&mut app)
            .tap(VirtualKeyCode::KeyA)
            .expect("physical A throws ORE1 into LORY");
        advance_app_until(&mut app, "ORE1 enters LORY", 180, |app| {
            app.engine
                .object_snapshot(ore)
                .is_some_and(|object| object.container == Some(lorry))
        });

        advance_app_until(&mut app, "Tutorial03 FNDR prompt", 240, |app| {
            app_tutorial_message_contains(
                app,
                "grab the lorry and push it into the gate of the foundry",
            )
        });
        advance_app_until(&mut app, "CLNK finishes the real Throw", 80, |app| {
            app.engine
                .object_snapshot(clonk)
                .is_some_and(|object| object.action.name == "Walk")
        });
        AppVirtualKeyboard::new(&mut app)
            .press(VirtualKeyCode::KeyZ)
            .expect("physical Z returns to LORY's grab area");
        advance_app_until(&mut app, "CLNK returns to LORY's grab area", 160, |app| {
            app.engine
                .object_snapshot(clonk)
                .zip(app.engine.object_snapshot(lorry))
                .is_some_and(|(clonk, lorry)| clonk.position.x <= lorry.position.x + 10)
        });
        AppVirtualKeyboard::new(&mut app)
            .release(VirtualKeyCode::KeyZ)
            .expect("release physical Z at LORY");
        AppVirtualKeyboard::new(&mut app)
            .tap(VirtualKeyCode::KeyX)
            .expect("physical X grabs loaded LORY");
        advance_app_until(&mut app, "CLNK grabs loaded LORY", 60, |app| {
            app.engine.object_snapshot(clonk).is_some_and(|object| {
                object.action.name == "Push" && object.action.target == Some(lorry)
            })
        });

        // S while pushing invokes ObjectComEnter on LORY. Its real Entrance
        // callback transfers ORE1 and WOOD into FNDR before metal production
        // (C4Object.cpp:3702-3710; Lorry.c4d/Script.c:82-91).
        AppVirtualKeyboard::new(&mut app)
            .press(VirtualKeyCode::KeyC)
            .expect("physical C pushes loaded LORY to FNDR");
        advance_app_until(&mut app, "loaded LORY reaches the FNDR gate", 400, |app| {
            app.engine.object_snapshot(lorry).is_some_and(|lorry| {
                (356..=376).contains(&lorry.position.x) && (253..=279).contains(&lorry.position.y)
            })
        });
        AppVirtualKeyboard::new(&mut app)
            .release(VirtualKeyCode::KeyC)
            .expect("release physical C at FNDR");
        AppVirtualKeyboard::new(&mut app)
            .tap(VirtualKeyCode::KeyS)
            .expect("physical S pushes LORY into FNDR");
        advance_app_until(&mut app, "loaded LORY enters FNDR", 120, |app| {
            app.engine
                .object_snapshot(lorry)
                .is_some_and(|object| object.container == Some(foundry))
        });
        advance_app_until(&mut app, "Tutorial03 explains FNDR", 240, |app| {
            app_tutorial_message_contains(app, "foundry processes ore and fuel into metal")
        });
        advance_app_until(&mut app, "FNDR produces METL", 600, |app| {
            app_object_with_definition(app, "METL").is_some()
        });
        advance_app_until(&mut app, "Tutorial03 explains METL", 240, |app| {
            app_tutorial_message_contains(app, "Metal can be used to build vehicles")
        });
        advance_app_until(&mut app, "Tutorial03 selects Tutorial04", 240, |app| {
            app.engine.next_mission().path == r"Tutorial.c4f\Tutorial04.c4s"
        });
        advance_app_until(&mut app, "Tutorial03 reaches GameOver", 320, |app| {
            app.snapshot.game_over && app.game_over_dialog.is_some()
        });
        assert!(
            app.snapshot
                .round_results
                .fulfilled_goals
                .iter()
                .any(|goal| goal == "SCRG"),
            "Tutorial03 must fulfill SCRG before GameOver"
        );
        assert_eq!(
            app.engine.next_mission().path,
            r"Tutorial.c4f\Tutorial04.c4s"
        );
        assert!(
            resolve_next_mission_scenario(
                &app.scenario_catalog,
                &app.engine.next_mission().path,
            )
            .is_some(),
            "the focused real-scenario catalog retains Tutorial04 navigation"
        );
        // The typed C4GameMessage guard has a dedicated regression.
        app.snapshot.hud.messages.clear();
        app.render(&mut rendered)
            .expect("render Tutorial03 GameOver through GameApp");
    }

    #[test]
    #[ignore = "over-constrained virtual-play driver; not a production parity oracle"]
    fn app_virtual_keyboard_completes_tutorial04_and_selects_tutorial05() {
        // Tutorial04 teaches the complete physical-key route from HUT2 and
        // CNKT through construction, elevator operation, mining, five GOLD
        // sales, SCRG fulfillment and Tutorial05 selection. Keep every state
        // transition behind GameApp::handle_key so this covers C++ key mapping,
        // menu conversion, DigDouble synthesis, movement and one-slot inventory
        // behavior at the actual app boundary
        // (Tutorial04.c4s/Script.c:40-234; C4Player.cpp:1490-1554;
        // C4ObjectMenu.cpp:279-435).
        let mut app = real_tutorial_app_with_roster(4, "Tutorial 4 app virtual player");
        assert!(
            !app.mouse_control,
            "Tutorial04 DisableMouse=1 must suppress player mouse control"
        );
        assert!(
            !app.option_flags(app.local_owner).mouse_shown,
            "Tutorial04 DisableMouse=1 must remove the in-game mouse Options entry"
        );

        let clonk = app
            .engine
            .crew_cursor(app.local_owner)
            .expect("Tutorial04 selected CLNK");
        let initial = app.engine.snapshot();
        let hut = initial
            .objects
            .iter()
            .find(|object| object.definition_id == "HUT2")
            .expect("Tutorial04 HUT2")
            .id;
        let conkit = initial
            .objects
            .iter()
            .find(|object| object.definition_id == "CNKT")
            .expect("Tutorial04 ready CNKT")
            .id;
        let context_identification = serde_json::from_value(serde_json::json!({ "Int": 14 }))
            .expect("context identification deserializes");
        let contents_identification = serde_json::from_value(serde_json::json!({ "Int": 18 }))
            .expect("contents identification deserializes");
        let construction_identification =
            serde_json::from_value(serde_json::json!({ "C4Id": "CXCN" }))
                .expect("construction identification deserializes");

        advance_app_until(&mut app, "Tutorial04 ready base and Clonk", 180, |app| {
            app.engine
                .object_snapshot(hut)
                .is_some_and(|object| object.base == app.local_owner)
                && app.engine.object_snapshot(clonk).is_some_and(|object| {
                    object.container.is_none() && object.action.name == "Walk"
                })
        });
        assert_eq!(
            app.engine
                .object_snapshot(hut)
                .expect("ready HUT2")
                .position,
            Vector2::new(586, 242),
            "seed-zero Tutorial04 must retain the HUT2 position used by its entrance lesson"
        );
        assert_eq!(
            app.engine
                .object_snapshot(conkit)
                .expect("ready CNKT")
                .container,
            Some(hut),
            "the real ready CNKT must begin inside HUT2"
        );
        advance_app_until(&mut app, "Tutorial04 enter-home-base prompt", 240, |app| {
            app_tutorial_message_contains(app, "Enter your home base")
        });

        // HUT2's seed-zero entrance is [568,584) x [250,267). Walk to its
        // center with physical Z, release it, then use physical S/Up so
        // ObjectComUp chooses Enter before Jump (Hut2 DefCore Entrance;
        // C4ObjectCom.cpp:335-350).
        {
            let mut keyboard = AppVirtualKeyboard::new(&mut app);
            keyboard
                .press(VirtualKeyCode::KeyZ)
                .expect("physical Z walks toward HUT2");
        }
        advance_app_until(&mut app, "CLNK aligned with HUT2 entrance", 30, |app| {
            app.engine.object_snapshot(clonk).is_some_and(|object| {
                object.action.name == "Walk"
                    && (574..578).contains(&object.position.x)
                    && (250..267).contains(&object.position.y)
            })
        });
        {
            let mut keyboard = AppVirtualKeyboard::new(&mut app);
            keyboard
                .release(VirtualKeyCode::KeyZ)
                .expect("release physical Z at HUT2");
            keyboard
                .tap(VirtualKeyCode::KeyS)
                .expect("physical S enters HUT2");
        }
        advance_app_until(&mut app, "CLNK entered HUT2", 50, |app| {
            app.engine
                .object_snapshot(clonk)
                .is_some_and(|object| object.container == Some(hut))
        });
        advance_app_until(&mut app, "Tutorial04 Contents prompt", 240, |app| {
            app_tutorial_message_contains(app, "select 'Contents'")
        });
        advance_app_until(&mut app, "HUT2 auto-context menu", 30, |app| {
            app.engine
                .cursor_object_menu(app.local_owner)
                .is_some_and(|(_, menu)| menu.identification == context_identification)
        });
        {
            let (_, menu) = app
                .engine
                .cursor_object_menu(app.local_owner)
                .expect("HUT2 context menu");
            assert_eq!(menu.selection, 0);
            assert_eq!(
                menu.items.first().map(|item| item.caption.as_str()),
                Some("Contents")
            );
        }

        // A is Throw/MenuEnter. The real context's first row opens Contents;
        // ready-material insertion keeps the later-created CNKT before FLAG.
        {
            let mut keyboard = AppVirtualKeyboard::new(&mut app);
            keyboard
                .tap(VirtualKeyCode::KeyA)
                .expect("physical A opens HUT2 Contents");
        }
        advance_app_until(&mut app, "HUT2 Contents menu", 20, |app| {
            app.engine
                .cursor_object_menu(app.local_owner)
                .is_some_and(|(_, menu)| menu.identification == contents_identification)
        });
        advance_app_until(
            &mut app,
            "Tutorial04 take-construction-kit prompt",
            240,
            |app| app_tutorial_message_contains(app, "Take the construction kit"),
        );
        {
            let (_, menu) = app
                .engine
                .cursor_object_menu(app.local_owner)
                .expect("HUT2 Contents remains open");
            assert_eq!(menu.selection, 0);
            assert_eq!(
                menu.items.first().map(|item| item.item_id.as_str()),
                Some("CNKT"),
                "physical A must target the real first Contents row"
            );
        }
        {
            let mut keyboard = AppVirtualKeyboard::new(&mut app);
            keyboard
                .tap(VirtualKeyCode::KeyA)
                .expect("physical A takes CNKT");
        }
        advance_app_until(&mut app, "CLNK carries CNKT", 60, |app| {
            app.engine
                .object_snapshot(conkit)
                .is_some_and(|object| object.container == Some(clonk))
        });
        advance_app_until(
            &mut app,
            "Tutorial04 close-menu-and-exit prompt",
            240,
            |app| app_tutorial_message_contains(app, "close the menu and exit"),
        );

        // Physical D closes Contents. AutoContextMenu returns on the next
        // player tick with the carried-CNKT Put row selected; physical S wraps
        // that first row to Exit and A activates it through ordinary menu
        // controls (C4Menu.cpp:433-480,1040-1069).
        {
            let mut keyboard = AppVirtualKeyboard::new(&mut app);
            keyboard
                .tap(VirtualKeyCode::KeyD)
                .expect("physical D closes Contents");
        }
        advance_app_until(&mut app, "HUT2 context restored", 30, |app| {
            app.engine
                .cursor_object_menu(app.local_owner)
                .is_some_and(|(_, menu)| menu.identification == context_identification)
        });
        {
            let (_, menu) = app
                .engine
                .cursor_object_menu(app.local_owner)
                .expect("HUT2 context restored around carried CNKT");
            assert_eq!(menu.selection, 0);
            assert_eq!(
                menu.items.first().map(|item| item.caption.as_str()),
                Some("Put")
            );
            assert_eq!(
                menu.items.last().map(|item| item.caption.as_str()),
                Some("Exit")
            );
        }
        {
            let mut keyboard = AppVirtualKeyboard::new(&mut app);
            keyboard
                .tap(VirtualKeyCode::KeyS)
                .expect("physical S wraps context selection to Exit");
            keyboard
                .tap(VirtualKeyCode::KeyA)
                .expect("physical A activates context Exit");
        }
        advance_app_until(&mut app, "CNKT-carrying CLNK exited HUT2", 60, |app| {
            app.engine
                .object_snapshot(clonk)
                .is_some_and(|object| object.container.is_none() && object.action.name == "Walk")
        });
        advance_app_until(&mut app, "Tutorial04 clear-area prompt", 240, |app| {
            app_tutorial_message_contains(app, "clear area to the left")
        });

        {
            let mut keyboard = AppVirtualKeyboard::new(&mut app);
            keyboard
                .press(VirtualKeyCode::KeyZ)
                .expect("physical Z walks to elevator site");
        }
        advance_app_until(&mut app, "CLNK reached elevator site", 120, |app| {
            app.engine.object_snapshot(clonk).is_some_and(|object| {
                object.action.name == "Walk" && (490..=510).contains(&object.position.x)
            })
        });
        {
            let mut keyboard = AppVirtualKeyboard::new(&mut app);
            keyboard
                .release(VirtualKeyCode::KeyZ)
                .expect("release physical Z at elevator site");
        }
        advance_app_until(&mut app, "Tutorial04 double-Dig prompt", 240, |app| {
            app_tutorial_message_contains(app, "twice quickly to open the construction menu")
        });
        assert!(
            app.engine
                .snapshot()
                .objects
                .iter()
                .all(|object| object.definition_id != "ELEV"),
            "Tutorial04 must not have an ELEV before CNKT activation"
        );

        // Two complete physical D edges inside C4DoubleClick's window become
        // COM_Dig_D. CNKT::Activate opens CXCN and fills its one known ELEV
        // row from GetPlrKnowledge without any menu or inventory mutation.
        {
            let mut keyboard = AppVirtualKeyboard::new(&mut app);
            keyboard
                .tap(VirtualKeyCode::KeyD)
                .expect("first physical D at elevator site");
            keyboard
                .tap(VirtualKeyCode::KeyD)
                .expect("second physical D at elevator site");
        }
        advance_app_until(&mut app, "CNKT CXCN menu", 20, |app| {
            app.engine
                .cursor_object_menu(app.local_owner)
                .is_some_and(|(_, menu)| menu.identification == construction_identification)
        });
        advance_app_until(&mut app, "Tutorial04 create-ELEV prompt", 240, |app| {
            app_tutorial_message_contains(app, "Create an elevator construction site")
        });
        {
            let (_, menu) = app
                .engine
                .cursor_object_menu(app.local_owner)
                .expect("physical D/D opens CNKT construction menu");
            assert_eq!(menu.identification, construction_identification);
            assert_eq!(menu.symbol_id, "CXCN");
            assert_eq!(menu.command_object, Some(conkit));
            assert_eq!(menu.extra, clonk_engine::ObjectMenuExtra::Components);
            assert_eq!(menu.selection, 0);
            assert_eq!(menu.items.len(), 1);
            assert_eq!(menu.items[0].item_id, "ELEV");
            assert_eq!(menu.items[0].caption, "Construction: Elevator");
            assert_eq!(
                menu.items[0].components,
                vec![
                    clonk_engine::ObjectMenuComponent {
                        definition_id: "WOOD".to_string(),
                        count: 4,
                    },
                    clonk_engine::ObjectMenuComponent {
                        definition_id: "METL".to_string(),
                        count: 2,
                    },
                ],
                "the app-visible CXCN row must retain ELEV's C++ component order"
            );
        }
        app.snapshot.hud.messages.clear();
        let mut rendered = vec![0_u8; 320 * 200 * 4];
        app.render(&mut rendered)
            .expect("render Tutorial04 CXCN through the app");

        {
            let mut keyboard = AppVirtualKeyboard::new(&mut app);
            keyboard
                .tap(VirtualKeyCode::KeyA)
                .expect("physical A creates ELEV construction");
        }
        advance_app_until(
            &mut app,
            "ELEV construction created and CNKT consumed",
            30,
            |app| {
                let elevator_exists = app
                    .engine
                    .snapshot()
                    .objects
                    .iter()
                    .any(|object| object.definition_id == "ELEV" && object.status.is_active());
                let conkit_removed = app
                    .engine
                    .object_snapshot(clonk)
                    .is_some_and(|object| !object.contents.contains(&conkit));
                elevator_exists && conkit_removed
            },
        );
        let elevator = app
            .engine
            .snapshot()
            .objects
            .into_iter()
            .find(|object| object.definition_id == "ELEV" && object.status.is_active())
            .expect("physical A creates active ELEV");
        assert_eq!(elevator.owner, app.local_owner);
        assert!((490..=510).contains(&elevator.position.x));
        assert!(
            (1..100_000).contains(&elevator.construction),
            "CNKT must create an incomplete construction site"
        );
        assert!(
            app.engine
                .object_snapshot(clonk)
                .is_some_and(|object| !object.contents.contains(&conkit)),
            "CreateConstructionSite consumes the real CNKT"
        );
        assert!(
            app.engine.cursor_object_menu(app.local_owner).is_none(),
            "removing CNKT closes the menu it owns"
        );

        // Down at an OCF_Construct object queues Build before Grab, and the
        // Build procedure advances construction until ELEV creates ELEC
        // (C4ObjectCom.cpp:573-588,690-697; C4Object.cpp:5010-5043;
        // Tutorial04.c4s/Script.c:119-137).
        advance_app_until(&mut app, "Tutorial04 build-ELEV prompt", 240, |app| {
            app_tutorial_message_contains(app, "press 'down' to start working")
        });
        AppVirtualKeyboard::new(&mut app)
            .tap(VirtualKeyCode::KeyX)
            .expect("physical X starts ELEV construction");
        advance_app_until(&mut app, "CLNK starts building ELEV", 30, |app| {
            app.engine
                .object_snapshot(clonk)
                .is_some_and(|object| object.action.name == "Build")
        });
        advance_app_until(&mut app, "ELEV finishes and creates ELEC", 720, |app| {
            app_object_with_definition(app, "ELEC").is_some()
                && app
                    .engine
                    .object_snapshot(elevator.id)
                    .is_some_and(|object| object.construction == 100_000)
        });
        let elevator_case =
            app_object_with_definition(&app, "ELEC").expect("completed ELEV creates ELEC");

        advance_app_until(&mut app, "Tutorial04 grab-ELEC prompt", 240, |app| {
            app_tutorial_message_contains(app, "Grab the elevator case")
        });
        AppVirtualKeyboard::new(&mut app)
            .tap(VirtualKeyCode::KeyX)
            .expect("physical X grabs ELEC");
        advance_app_until(&mut app, "CLNK grabs ELEC", 60, |app| {
            app.engine.object_snapshot(clonk).is_some_and(|object| {
                object.action.name == "Push" && object.action.target == Some(elevator_case)
            })
        });

        // ELEC maps held Dig to downward travel and held Up to upward travel;
        // Tutorial04 observes the real crew positions before changing prompts
        // (Tutorial04.c4s/Script.c:139-166).
        advance_app_until(&mut app, "Tutorial04 drill-shaft prompt", 240, |app| {
            app_tutorial_message_contains(app, "Hold down the 'dig' key")
        });
        hold_app_key_until(
            &mut app,
            VirtualKeyCode::KeyD,
            "ELEC drills CLNK to the shaft bottom",
            360,
            |app| {
                app.engine
                    .object_snapshot(clonk)
                    .is_some_and(|object| object.position.y >= 340)
            },
        );
        advance_app_until(&mut app, "Tutorial04 ride-up prompt", 240, |app| {
            app_tutorial_message_contains(app, "ride the elevator back up")
        });
        hold_app_key_until(
            &mut app,
            VirtualKeyCode::KeyS,
            "ELEC carries CLNK back to the surface",
            240,
            |app| {
                app.engine
                    .object_snapshot(clonk)
                    .is_some_and(|object| object.position.y <= 270)
            },
        );
        advance_app_until(&mut app, "Tutorial04 let-go prompt", 240, |app| {
            app_tutorial_message_contains(app, "Let go of the elevator case")
        });
        {
            let mut keyboard = AppVirtualKeyboard::new(&mut app);
            keyboard
                .tap(VirtualKeyCode::KeyX)
                .expect("first physical X to ungrab ELEC");
            keyboard
                .tap(VirtualKeyCode::KeyX)
                .expect("second physical X to ungrab ELEC");
        }
        advance_app_until(&mut app, "CLNK lets go of ELEC", 60, |app| {
            app.engine
                .object_snapshot(clonk)
                .is_some_and(|object| object.action.name != "Push")
        });
        advance_app_until(&mut app, "Tutorial04 spawns surface TFLN", 240, |app| {
            app_tutorial_message_contains(app, "Walk back to the cabin")
                && app_object_with_definition(app, "TFLN").is_some()
        });
        let first_flint = app_object_with_definition(&app, "TFLN")
            .expect("preserve Tutorial04's exact first TFLN identity");

        // The shaft lip alternates Jump and Scale. Re-emitting physical Right
        // on Scale and physical Up after landing follows the C++ transitions;
        // the exiting TFLN is collected naturally before its fuse expires
        // (C4Object.cpp:3618-3628,4284-4299,4823-4855;
        // Tutorial04.c4s/Script.c:167-179).
        AppVirtualKeyboard::new(&mut app)
            .press(VirtualKeyCode::KeyC)
            .expect("physical C exits the shaft toward TFLN");
        let mut previous_action = String::new();
        for _ in 0..60 {
            if app_clonk_carries(&app, clonk, "TFLN") {
                break;
            }
            let clonk_now = app
                .engine
                .object_snapshot(clonk)
                .expect("CLNK survives the shaft exit");
            let action = clonk_now.action.name;
            let entered_scale =
                action.starts_with("Scale") && !previous_action.starts_with("Scale");
            let left_scale_in_flight = action == "Jump" && previous_action.starts_with("Scale");
            let landed = action == "Walk" && previous_action != "Walk";
            if entered_scale {
                let mut keyboard = AppVirtualKeyboard::new(&mut app);
                keyboard
                    .release(VirtualKeyCode::KeyC)
                    .expect("release physical C on Scale");
                keyboard
                    .press(VirtualKeyCode::KeyC)
                    .expect("repress physical C on Scale");
            } else if (landed || left_scale_in_flight) && clonk_now.position.x < 550 {
                AppVirtualKeyboard::new(&mut app)
                    .tap(VirtualKeyCode::KeyS)
                    .expect("physical S jumps out of the shaft");
            }
            previous_action = action;
            app.update().expect("advance CLNK toward surface TFLN");
        }
        AppVirtualKeyboard::new(&mut app)
            .release(VirtualKeyCode::KeyC)
            .expect("release physical C after TFLN pickup");
        assert!(
            app_clonk_carries(&app, clonk, "TFLN"),
            "CLNK must naturally collect the real exiting TFLN"
        );

        hold_app_key_until(
            &mut app,
            VirtualKeyCode::KeyZ,
            "TFLN-carrying CLNK returns toward ELEC",
            120,
            |app| {
                app_tutorial_message_contains(app, "Ride back down into the mine")
                    || app
                        .engine
                        .object_snapshot(clonk)
                        .zip(app.engine.object_snapshot(elevator_case))
                        .is_some_and(|(clonk, elevator)| {
                            clonk.position.x <= elevator.position.x + 5
                        })
            },
        );
        advance_app_until(&mut app, "Tutorial04 TFLN ride-down prompt", 240, |app| {
            app_tutorial_message_contains(app, "Ride back down into the mine")
                && app_clonk_carries(app, clonk, "TFLN")
        });

        let (clonk_x, elevator_x) = app
            .engine
            .object_snapshot(clonk)
            .zip(app.engine.object_snapshot(elevator_case))
            .map(|(clonk, elevator)| (clonk.position.x, elevator.position.x))
            .expect("CLNK and ELEC survive the surface return");
        if clonk_x < elevator_x - 5 {
            hold_app_key_until(
                &mut app,
                VirtualKeyCode::KeyC,
                "CLNK aligns with ELEC from the left",
                120,
                |app| {
                    app.engine
                        .object_snapshot(clonk)
                        .zip(app.engine.object_snapshot(elevator_case))
                        .is_some_and(|(clonk, elevator)| {
                            (clonk.position.x - elevator.position.x).abs() <= 5
                        })
                },
            );
        } else if clonk_x > elevator_x + 5 {
            hold_app_key_until(
                &mut app,
                VirtualKeyCode::KeyZ,
                "CLNK aligns with ELEC from the right",
                120,
                |app| {
                    app.engine
                        .object_snapshot(clonk)
                        .zip(app.engine.object_snapshot(elevator_case))
                        .is_some_and(|(clonk, elevator)| {
                            (clonk.position.x - elevator.position.x).abs() <= 5
                        })
                },
            );
        }
        if let Some(clonk_now) = app.engine.object_snapshot(clonk) {
            if clonk_now.action.name.starts_with("Scale") {
                let away = if clonk_now.direction == Direction::Left {
                    VirtualKeyCode::KeyC
                } else {
                    VirtualKeyCode::KeyZ
                };
                AppVirtualKeyboard::new(&mut app)
                    .tap(away)
                    .expect("physical direction leaves Scale beside ELEC");
            }
        }
        advance_app_until(
            &mut app,
            "TFLN-carrying CLNK settles beside ELEC",
            120,
            |app| {
                app.engine
                    .object_snapshot(clonk)
                    .zip(app.engine.object_snapshot(elevator_case))
                    .is_some_and(|(clonk, elevator)| {
                        clonk.action.name == "Walk"
                            && (clonk.position.x - elevator.position.x).abs() <= 5
                    })
            },
        );
        AppVirtualKeyboard::new(&mut app)
            .tap(VirtualKeyCode::KeyX)
            .expect("physical X grabs ELEC with TFLN");
        advance_app_until(&mut app, "TFLN-carrying CLNK grabs ELEC", 60, |app| {
            app.engine.object_snapshot(clonk).is_some_and(|object| {
                object.action.name == "Push" && object.action.target == Some(elevator_case)
            })
        });
        hold_app_key_until(
            &mut app,
            VirtualKeyCode::KeyD,
            "ELEC carries TFLN-carrying CLNK underground",
            360,
            |app| {
                app.engine
                    .object_snapshot(clonk)
                    .is_some_and(|object| object.position.y >= 340)
            },
        );
        advance_app_until(&mut app, "Tutorial04 gold-tunnel prompt", 240, |app| {
            app_tutorial_message_contains(app, "Dig a tunnel all the way")
        });
        {
            let mut keyboard = AppVirtualKeyboard::new(&mut app);
            keyboard
                .tap(VirtualKeyCode::KeyX)
                .expect("first physical X to ungrab underground");
            keyboard
                .tap(VirtualKeyCode::KeyX)
                .expect("second physical X to ungrab underground");
        }
        advance_app_until(&mut app, "CLNK lets go underground", 60, |app| {
            app.engine
                .object_snapshot(clonk)
                .is_some_and(|object| object.action.name == "Walk")
        });

        // A physical Dig press followed by held Left+Down steers DFA_DIG into
        // Script175's 80x80 gold rectangle. Bottom contact redirects the first
        // diagonal to Left, so a fresh Down edge restores DownLeft exactly as
        // C++ does (Tutorial04.c4s/Script.c:180-207;
        // C4ObjectCom.cpp:353-362; C4Object.cpp:3573-3631,4354-4368).
        AppVirtualKeyboard::new(&mut app)
            .tap(VirtualKeyCode::KeyD)
            .expect("physical D starts the gold tunnel");
        advance_app_until(&mut app, "CLNK starts digging toward GOLD", 30, |app| {
            app.engine
                .object_snapshot(clonk)
                .is_some_and(|object| object.action.name == "Dig")
        });
        {
            let mut keyboard = AppVirtualKeyboard::new(&mut app);
            keyboard
                .press(VirtualKeyCode::KeyZ)
                .expect("physical Z steers Dig left");
            keyboard
                .press(VirtualKeyCode::KeyX)
                .expect("physical X steers Dig down");
        }
        for _ in 0..12 {
            app.update().expect("advance initial tunnel Dig");
        }
        {
            let mut keyboard = AppVirtualKeyboard::new(&mut app);
            keyboard
                .release(VirtualKeyCode::KeyX)
                .expect("release physical X to refresh diagonal Dig");
            keyboard
                .press(VirtualKeyCode::KeyX)
                .expect("repress physical X for diagonal Dig");
        }
        let mut reached_gold_face = false;
        for _ in 0..360 {
            let clonk_now = app
                .engine
                .object_snapshot(clonk)
                .expect("CLNK survives the real gold tunnel");
            if (clonk_now.action.name == "Dig" && clonk_now.position.x <= 432)
                || (clonk_now.action.name == "Walk"
                    && (357..437).contains(&clonk_now.position.x)
                    && (348..440).contains(&clonk_now.position.y))
            {
                reached_gold_face = true;
                break;
            }
            if clonk_now.command_direction == CommandDirection::Left {
                let mut keyboard = AppVirtualKeyboard::new(&mut app);
                keyboard
                    .release(VirtualKeyCode::KeyX)
                    .expect("release physical X after bottom redirect");
                keyboard
                    .press(VirtualKeyCode::KeyX)
                    .expect("repress physical X toward GOLD");
            }
            app.update().expect("advance real gold tunnel");
        }
        {
            let mut keyboard = AppVirtualKeyboard::new(&mut app);
            keyboard
                .release(VirtualKeyCode::KeyX)
                .expect("release physical X at the gold face");
            keyboard
                .release(VirtualKeyCode::KeyZ)
                .expect("release physical Z at the gold face");
        }
        assert!(
            reached_gold_face,
            "physical-key Dig must naturally stop at Tutorial04's solid-GOLD face; clonk={:?}",
            app.engine.object_snapshot(clonk)
        );
        advance_app_until(&mut app, "Tutorial04 blast-GOLD prompt", 120, |app| {
            app_tutorial_message_contains(app, "struck solid gold")
        });
        advance_app_until(&mut app, "CLNK stops Dig at the gold face", 40, |app| {
            app.engine
                .object_snapshot(clonk)
                .is_some_and(|object| object.action.name == "Walk")
        });

        let safe_x = app
            .engine
            .object_snapshot(clonk)
            .expect("CLNK survives the gold tunnel")
            .position
            .x
            + 24;
        hold_app_key_until(
            &mut app,
            VirtualKeyCode::KeyC,
            "TFLN-carrying CLNK reaches a safe throwing distance",
            80,
            |app| {
                app.engine
                    .object_snapshot(clonk)
                    .is_some_and(|object| object.position.x >= safe_x)
            },
        );
        AppVirtualKeyboard::new(&mut app)
            .tap(VirtualKeyCode::KeyZ)
            .expect("physical Z faces CLNK toward the gold vein");
        app.update()
            .expect("settle left-facing CLNK before TFLN throw");
        AppVirtualKeyboard::new(&mut app)
            .tap(VirtualKeyCode::KeyA)
            .expect("physical A throws TFLN toward GOLD");
        advance_app_until(&mut app, "TFLN leaves CLNK inventory", 30, |app| {
            !app_clonk_carries(app, clonk, "TFLN")
        });
        for _ in 0..180 {
            if app_object_with_definition(&app, "GOLD").is_some() {
                break;
            }
            app.update().expect("advance first TFLN toward GOLD blast");
        }
        assert!(
            app_object_with_definition(&app, "GOLD").is_some(),
            "first TFLN must free real GOLD objects"
        );
        assert!(
            app.engine.object_snapshot(first_flint).is_none(),
            "the exact first TFLN must detonate"
        );
        for _ in 0..100 {
            app.update().expect("settle the first real GOLD blast");
        }
        if app
            .engine
            .object_snapshot(clonk)
            .is_some_and(|object| object.action.name == "Hangle")
        {
            AppVirtualKeyboard::new(&mut app)
                .tap(VirtualKeyCode::KeyX)
                .expect("physical X drops CLNK from the tunnel ceiling");
            advance_app_until(&mut app, "CLNK drops into the GOLD pocket", 60, |app| {
                app.engine.object_snapshot(clonk).is_some_and(|object| {
                    object.action.name == "Walk" || object.action.name.starts_with("Scale")
                })
            });
        }
        app_collect_one_gold_around_blast_debris(&mut app, clonk);
        let sold_gold = app
            .engine
            .object_snapshot(clonk)
            .expect("GOLD-carrying CLNK survives collection")
            .contents
            .into_iter()
            .find(|&object_id| {
                app.engine
                    .object_snapshot(object_id)
                    .is_some_and(|object| object.definition_id == "GOLD")
            })
            .expect("CLNK carries the exact GOLD object to be sold");
        let wealth_before_sale = app
            .engine
            .player(app.local_owner)
            .expect("local player exists before sale")
            .wealth();

        AppVirtualKeyboard::new(&mut app)
            .press(VirtualKeyCode::KeyC)
            .expect("physical C returns GOLD-carrying CLNK to ELEC");
        let mut returned_to_elevator = false;
        let mut previous_action = String::new();
        for _ in 0..360 {
            let clonk_now = app
                .engine
                .object_snapshot(clonk)
                .expect("GOLD-carrying CLNK survives the tunnel return");
            returned_to_elevator =
                app.engine
                    .object_snapshot(elevator_case)
                    .is_some_and(|elevator| {
                        clonk_now.action.name == "Walk"
                            && (clonk_now.position.x - elevator.position.x).abs() <= 5
                    });
            if returned_to_elevator {
                break;
            }
            let action = clonk_now.action.name;
            if action.starts_with("Scale") && !previous_action.starts_with("Scale") {
                let mut keyboard = AppVirtualKeyboard::new(&mut app);
                keyboard
                    .release(VirtualKeyCode::KeyC)
                    .expect("release physical C on tunnel Scale");
                keyboard
                    .press(VirtualKeyCode::KeyC)
                    .expect("repress physical C to leave tunnel Scale");
            } else if action == "Hangle" && previous_action != "Hangle" {
                AppVirtualKeyboard::new(&mut app)
                    .tap(VirtualKeyCode::KeyX)
                    .expect("physical X drops from the tunnel ceiling");
            }
            previous_action = action;
            app.update().expect("advance GOLD-carrying CLNK to ELEC");
        }
        AppVirtualKeyboard::new(&mut app)
            .release(VirtualKeyCode::KeyC)
            .expect("release physical C beside ELEC");
        assert!(
            returned_to_elevator,
            "GOLD-carrying CLNK must return to ELEC; clonk={:?}, elevator={:?}",
            app.engine.object_snapshot(clonk),
            app.engine.object_snapshot(elevator_case)
        );
        AppVirtualKeyboard::new(&mut app)
            .tap(VirtualKeyCode::KeyX)
            .expect("physical X grabs ELEC with GOLD");
        advance_app_until(&mut app, "GOLD-carrying CLNK grabs ELEC", 60, |app| {
            app.engine.object_snapshot(clonk).is_some_and(|object| {
                object.action.name == "Push" && object.action.target == Some(elevator_case)
            })
        });
        hold_app_key_until(
            &mut app,
            VirtualKeyCode::KeyS,
            "ELEC raises GOLD-carrying CLNK to the surface",
            300,
            |app| {
                app.engine
                    .object_snapshot(clonk)
                    .is_some_and(|object| object.position.y <= 270)
            },
        );
        {
            let mut keyboard = AppVirtualKeyboard::new(&mut app);
            keyboard
                .tap(VirtualKeyCode::KeyX)
                .expect("first physical X to ungrab ELEC with GOLD");
            keyboard
                .tap(VirtualKeyCode::KeyX)
                .expect("second physical X to ungrab ELEC with GOLD");
        }
        advance_app_until(&mut app, "GOLD-carrying CLNK lets go of ELEC", 60, |app| {
            app.engine
                .object_snapshot(clonk)
                .is_some_and(|object| object.action.name != "Push")
        });

        // Cross the surface lip using only held Right and real Up jump edges,
        // then enter HUT2 through its actual entrance. BaseAutoSell removes the
        // GOLD and increments wealth by five (C4Object.cpp:3618-3628,
        // 4284-4299,4823-4855; Tutorial04.c4s/Script.c:214-234).
        AppVirtualKeyboard::new(&mut app)
            .press(VirtualKeyCode::KeyC)
            .expect("physical C climbs the shaft lip with GOLD");
        let mut previous_action = String::new();
        for _ in 0..240 {
            let clonk_now = app
                .engine
                .object_snapshot(clonk)
                .expect("GOLD-carrying CLNK survives the shaft climb");
            if clonk_now.position.x >= 558 {
                break;
            }
            let action = clonk_now.action.name;
            let entered_scale =
                action.starts_with("Scale") && !previous_action.starts_with("Scale");
            let left_scale_in_flight = action == "Jump" && previous_action.starts_with("Scale");
            let landed = action == "Walk" && previous_action != "Walk";
            if entered_scale {
                let mut keyboard = AppVirtualKeyboard::new(&mut app);
                keyboard
                    .release(VirtualKeyCode::KeyC)
                    .expect("release physical C on shaft Scale");
                keyboard
                    .press(VirtualKeyCode::KeyC)
                    .expect("repress physical C on shaft Scale");
            } else if landed || left_scale_in_flight {
                AppVirtualKeyboard::new(&mut app)
                    .tap(VirtualKeyCode::KeyS)
                    .expect("physical S jumps the shaft lip");
            }
            previous_action = action;
            app.update()
                .expect("advance GOLD-carrying CLNK over the shaft lip");
        }
        AppVirtualKeyboard::new(&mut app)
            .release(VirtualKeyCode::KeyC)
            .expect("release physical C on the cabin hill");
        assert!(
            app.engine
                .object_snapshot(clonk)
                .is_some_and(|object| object.position.x >= 558),
            "GOLD-carrying CLNK must reach the cabin hill"
        );
        advance_app_until(
            &mut app,
            "GOLD-carrying CLNK lands beside HUT2",
            80,
            |app| {
                app.engine
                    .object_snapshot(clonk)
                    .is_some_and(|object| object.action.name == "Walk")
            },
        );
        hold_app_key_until(
            &mut app,
            VirtualKeyCode::KeyZ,
            "GOLD-carrying CLNK aligns with HUT2's entrance",
            80,
            |app| {
                app.engine
                    .object_snapshot(clonk)
                    .is_some_and(|object| object.position.x <= 570)
            },
        );
        AppVirtualKeyboard::new(&mut app)
            .tap(VirtualKeyCode::KeyS)
            .expect("physical S enters HUT2 with GOLD");
        advance_app_until(&mut app, "GOLD-carrying CLNK enters HUT2", 60, |app| {
            app.engine
                .object_snapshot(clonk)
                .is_some_and(|object| object.container == Some(hut))
        });
        advance_app_until(&mut app, "HUT2 sells the first GOLD chunk", 80, |app| {
            app.engine
                .player(app.local_owner)
                .is_some_and(|player| player.wealth() == wealth_before_sale + 5)
                && app.engine.object_snapshot(sold_gold).is_none()
        });
        assert_eq!(
            app.engine
                .player(app.local_owner)
                .expect("local player survives sale")
                .wealth(),
            wealth_before_sale + 5,
            "GOLD value five is added exactly once (C4Object.cpp:970-997; C4Player.cpp:866-897; GOLD DefCore.txt:13,18)"
        );
        assert!(
            app.engine.object_snapshot(sold_gold).is_none(),
            "BaseAutoSell must remove the sold GOLD object"
        );
        assert!(
            !app_clonk_carries(&app, clonk, "GOLD"),
            "BaseAutoSell must remove the first GOLD from CLNK"
        );

        // Script200 creates three replacement TFLNs in HUT2 after the first
        // sale, then Script201/250 asks the contained Clonk to take one and
        // earn 25 gold points (Tutorial04.c4s/Script.c:214-231). Drive the
        // actual context/Contents menus: Down selects the TFLN row and
        // Special2 chooses Command2/EnterAll (C4Menu.cpp:433-440,498-523,
        // 1047-1054).
        advance_app_until(
            &mut app,
            "Tutorial04 creates three replacement TFLNs in HUT2",
            400,
            |app| {
                app_tutorial_message_contains(app, "more T-Flints")
                    && app_object_contents_count(app, hut, "TFLN") == 3
            },
        );
        advance_app_until(
            &mut app,
            "Tutorial04 replacement-flint Contents prompt",
            400,
            |app| app_tutorial_message_contains(app, "Select 'Contents'"),
        );
        advance_app_until(&mut app, "HUT2 replacement-flint context menu", 30, |app| {
            app.engine
                .cursor_object_menu(app.local_owner)
                .is_some_and(|(_, menu)| menu.identification == context_identification)
        });
        {
            let (_, menu) = app
                .engine
                .cursor_object_menu(app.local_owner)
                .expect("HUT2 replacement-flint context menu");
            assert_eq!(menu.selection, 0);
            assert_eq!(
                menu.items.first().map(|item| item.caption.as_str()),
                Some("Contents")
            );
        }
        AppVirtualKeyboard::new(&mut app)
            .tap(VirtualKeyCode::KeyA)
            .expect("physical A opens replacement-flint Contents");
        advance_app_until(&mut app, "HUT2 replacement-flint Contents", 30, |app| {
            app.engine
                .cursor_object_menu(app.local_owner)
                .is_some_and(|(_, menu)| menu.identification == contents_identification)
        });

        let contents_rows = app
            .engine
            .cursor_object_menu(app.local_owner)
            .expect("replacement-flint Contents menu")
            .1
            .items
            .len();
        for _ in 0..contents_rows {
            if app_selected_object_menu_item(&app).is_some_and(|item| item.item_id == "TFLN") {
                break;
            }
            AppVirtualKeyboard::new(&mut app)
                .tap(VirtualKeyCode::KeyX)
                .expect("physical X selects the next Contents row");
        }
        assert_eq!(
            app_selected_object_menu_item(&app).map(|item| item.item_id.as_str()),
            Some("TFLN"),
            "physical Down navigation must select the replacement TFLN row"
        );
        AppVirtualKeyboard::new(&mut app)
            .tap(VirtualKeyCode::KeyF)
            .expect("physical F takes all replacement TFLNs that fit");
        advance_app_until(
            &mut app,
            "C++ nonspecial capacity keeps one TFLN on CLNK and two in HUT2",
            120,
            |app| {
                app_object_contents_count(app, clonk, "TFLN") == 1
                    && app_object_contents_count(app, hut, "TFLN") == 2
            },
        );

        AppVirtualKeyboard::new(&mut app)
            .tap(VirtualKeyCode::KeyD)
            .expect("physical D closes replacement-flint Contents");
        advance_app_until(&mut app, "HUT2 context restored after TFLN", 30, |app| {
            app.engine
                .cursor_object_menu(app.local_owner)
                .is_some_and(|(_, menu)| menu.identification == context_identification)
        });
        {
            let (_, menu) = app
                .engine
                .cursor_object_menu(app.local_owner)
                .expect("HUT2 context restored around carried TFLN");
            assert_eq!(menu.selection, 0);
            assert_eq!(
                menu.items.first().map(|item| item.caption.as_str()),
                Some("Put")
            );
            assert_eq!(
                menu.items.last().map(|item| item.caption.as_str()),
                Some("Exit")
            );
        }
        {
            let mut keyboard = AppVirtualKeyboard::new(&mut app);
            keyboard
                .tap(VirtualKeyCode::KeyS)
                .expect("physical S wraps replacement-flint context to Exit");
            keyboard
                .tap(VirtualKeyCode::KeyA)
                .expect("physical A exits HUT2 with replacement TFLN");
        }
        advance_app_until(&mut app, "replacement-TFLN CLNK exits HUT2", 60, |app| {
            app.engine
                .object_snapshot(clonk)
                .is_some_and(|object| object.container.is_none())
        });
        advance_app_until(
            &mut app,
            "Tutorial04 states its 25-gold objective",
            640,
            |app| app_tutorial_message_contains(app, "Gain 25"),
        );
        assert_eq!(
            app.engine
                .player(app.local_owner)
                .expect("local player survives replacement-flint withdrawal")
                .wealth(),
            wealth_before_sale + 5,
            "withdrawing replacement TFLN must not change the 25-gold objective's wealth"
        );
        assert!(
            app_clonk_carries(&app, clonk, "TFLN"),
            "CLNK must exit with exactly one usable replacement TFLN"
        );
        let replacement_flint = app
            .engine
            .object_snapshot(clonk)
            .expect("replacement-TFLN CLNK survives HUT2 exit")
            .contents
            .into_iter()
            .find(|item| {
                app.engine
                    .object_snapshot(*item)
                    .is_some_and(|item| item.definition_id == "TFLN")
            })
            .expect("preserve the exact replacement TFLN withdrawn from HUT2");

        // Return over the real shaft lip, grab the same ELEC, and descend to
        // the first blast tunnel. C++ directs movement to the pushed ELEC and
        // turns Dig into its Down control; DownDouble releases it at the floor
        // (C4Player.cpp:1397-1443,1453-1553; C4Object.cpp:3321-3337,
        // 3520-3567; Tutorial04.c4s/Script.c:214-234).
        hold_app_key_until(
            &mut app,
            VirtualKeyCode::KeyZ,
            "replacement-TFLN CLNK returns to ELEC",
            180,
            |app| {
                app.engine
                    .object_snapshot(clonk)
                    .zip(app.engine.object_snapshot(elevator_case))
                    .is_some_and(|(clonk, elevator)| {
                        (clonk.position.x - elevator.position.x).abs() <= 5
                    })
            },
        );
        AppVirtualKeyboard::new(&mut app)
            .press(VirtualKeyCode::KeyC)
            .expect("physical C crosses the replacement-flint shaft lip");
        let mut previous_action = String::new();
        for _ in 0..120 {
            let clonk_now = app
                .engine
                .object_snapshot(clonk)
                .expect("replacement-TFLN CLNK survives the shaft lip");
            if clonk_now.action.name == "Walk" && clonk_now.position.x >= 505 {
                break;
            }
            let action = clonk_now.action.name;
            let entered_scale =
                action.starts_with("Scale") && !previous_action.starts_with("Scale");
            let left_scale_in_flight = action == "Jump" && previous_action.starts_with("Scale");
            let landed = action == "Walk" && previous_action != "Walk";
            if entered_scale {
                let mut keyboard = AppVirtualKeyboard::new(&mut app);
                keyboard
                    .release(VirtualKeyCode::KeyC)
                    .expect("release physical C on replacement-flint Scale");
                keyboard
                    .press(VirtualKeyCode::KeyC)
                    .expect("repress physical C on replacement-flint Scale");
            } else if landed || left_scale_in_flight {
                AppVirtualKeyboard::new(&mut app)
                    .tap(VirtualKeyCode::KeyS)
                    .expect("physical S jumps the replacement-flint shaft lip");
            }
            previous_action = action;
            app.update()
                .expect("advance replacement-TFLN CLNK across shaft lip");
        }
        AppVirtualKeyboard::new(&mut app)
            .release(VirtualKeyCode::KeyC)
            .expect("release physical C after replacement-flint shaft lip");
        hold_app_key_until(
            &mut app,
            VirtualKeyCode::KeyZ,
            "replacement-TFLN CLNK stands beside ELEC",
            80,
            |app| {
                app.engine
                    .object_snapshot(clonk)
                    .zip(app.engine.object_snapshot(elevator_case))
                    .is_some_and(|(clonk, elevator)| {
                        clonk.action.name == "Walk"
                            && (clonk.position.x - elevator.position.x).abs() <= 5
                    })
            },
        );
        AppVirtualKeyboard::new(&mut app)
            .tap(VirtualKeyCode::KeyX)
            .expect("physical X grabs ELEC with the replacement TFLN");
        advance_app_until(
            &mut app,
            "replacement-TFLN CLNK grabs exact ELEC",
            60,
            |app| {
                app.engine.object_snapshot(clonk).is_some_and(|object| {
                    object.action.name == "Push" && object.action.target == Some(elevator_case)
                })
            },
        );
        hold_app_key_until(
            &mut app,
            VirtualKeyCode::KeyD,
            "ELEC carries replacement-TFLN CLNK underground",
            360,
            |app| {
                app.engine
                    .object_snapshot(clonk)
                    .is_some_and(|object| object.position.y >= 340)
            },
        );
        {
            let mut keyboard = AppVirtualKeyboard::new(&mut app);
            keyboard
                .tap(VirtualKeyCode::KeyX)
                .expect("first physical X releases ELEC underground with TFLN");
            keyboard
                .tap(VirtualKeyCode::KeyX)
                .expect("second physical X releases ELEC underground with TFLN");
        }
        advance_app_until(
            &mut app,
            "replacement-TFLN CLNK lets go underground",
            60,
            |app| {
                app.engine
                    .object_snapshot(clonk)
                    .is_some_and(|object| object.action.name == "Walk")
            },
        );
        app_tutorial04_blast_next_gold_face(&mut app, clonk, replacement_flint, 414, 2);
        // The fixed corrected MapSeed face releases exactly two GOLD objects;
        // the helper pins that C++ output rather than accepting any growth.
        for _ in 0..120 {
            app.update()
                .expect("settle the replacement-TFLN blast pocket");
        }
        if app
            .engine
            .object_snapshot(clonk)
            .is_some_and(|object| object.action.name == "Hangle")
        {
            AppVirtualKeyboard::new(&mut app)
                .tap(VirtualKeyCode::KeyX)
                .expect("physical X drops CLNK into the second blast pocket");
            advance_app_until(&mut app, "CLNK drops into second blast pocket", 60, |app| {
                app.engine
                    .object_snapshot(clonk)
                    .is_some_and(|object| object.action.name == "Walk")
            });
        }

        app_collect_one_gold_around_blast_debris(&mut app, clonk);
        assert_eq!(
            app_object_contents_count(&app, clonk, "GOLD"),
            1,
            "C++ one-nonspecial-slot CLNK must carry exactly one GOLD per trip"
        );

        // Sell one chunk, then physically withdraw a second replacement TFLN.
        // Its fixed corrected-seed face releases exactly one more GOLD object;
        // the already exposed loose chunks supply the remaining sale trips.
        app_carry_tutorial04_gold_to_hut(&mut app, clonk, elevator_case, hut, 10);
        let additional_flint = app_take_tutorial04_flint_from_hut(&mut app, clonk, hut);
        app_return_tutorial04_from_hut_to_tunnel(&mut app, clonk, elevator_case, hut);
        app_tutorial04_blast_next_gold_face(&mut app, clonk, additional_flint, 402, 1);
        for _ in 0..120 {
            app.update()
                .expect("settle the additional-TFLN blast pocket");
        }
        if app
            .engine
            .object_snapshot(clonk)
            .is_some_and(|object| object.action.name == "Hangle")
        {
            AppVirtualKeyboard::new(&mut app)
                .tap(VirtualKeyCode::KeyX)
                .expect("physical X drops CLNK into the final blast pocket");
            advance_app_until(&mut app, "CLNK drops into final blast pocket", 60, |app| {
                app.engine
                    .object_snapshot(clonk)
                    .is_some_and(|object| object.action.name == "Walk")
            });
        }
        app_collect_one_gold_around_blast_debris(&mut app, clonk);
        assert_eq!(app_object_contents_count(&app, clonk, "GOLD"), 1);

        app_carry_tutorial04_gold_to_hut(&mut app, clonk, elevator_case, hut, 15);
        app_return_tutorial04_from_hut_to_tunnel(&mut app, clonk, elevator_case, hut);
        app_collect_one_gold_around_blast_debris(&mut app, clonk);
        assert_eq!(app_object_contents_count(&app, clonk, "GOLD"), 1);

        // The first three trips sold value-five chunks. The fixed face-414
        // and face-402 outputs leave enough exact loose GOLD for two more
        // physical ELEC/HUT2 round trips; each trip preserves the exact GOLD
        // identity until BaseAutoSell removes it. Script251 may fulfill SCRG
        // only after wealth reaches 25 and then selects Tutorial05
        // (Tutorial04.c4s/Script.c:227-234; C4Object.cpp:970-997;
        // C4Player.cpp:866-897).
        for sold_chunks in 4..=5 {
            let target_wealth = sold_chunks * 5;
            app_carry_tutorial04_gold_to_hut(&mut app, clonk, elevator_case, hut, target_wealth);
            if sold_chunks < 5 {
                app_return_tutorial04_from_hut_to_tunnel(&mut app, clonk, elevator_case, hut);
                app_collect_one_gold_around_blast_debris(&mut app, clonk);
                assert_eq!(
                    app_object_contents_count(&app, clonk, "GOLD"),
                    1,
                    "each return trip must collect exactly one real GOLD"
                );
            }
        }
        advance_app_until(&mut app, "Tutorial04 selects Tutorial05", 640, |app| {
            app.engine.next_mission().path == r"Tutorial.c4f\Tutorial05.c4s"
        });
        advance_app_until(
            &mut app,
            "Tutorial04 fulfilled goal reaches GameOver",
            320,
            |app| app.engine.snapshot().game_over,
        );
        assert!(
            app.engine
                .snapshot()
                .round_results
                .fulfilled_goals
                .iter()
                .any(|goal| goal == "SCRG"),
            "Tutorial04 must fulfill its exact SCRG before selecting Tutorial05"
        );
        assert_eq!(
            app.engine.next_mission().path,
            r"Tutorial.c4f\Tutorial05.c4s"
        );
    }
