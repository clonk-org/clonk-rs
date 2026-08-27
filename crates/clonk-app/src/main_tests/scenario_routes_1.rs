// Spliced into `mod tests` (src/main_tests.rs) via include!: a bare item
// sequence, not a child module, so test ids stay `tests::<fn>`.

#[test]
fn real_hazard_scenario_gui_sheet_overrides_apply_and_reach_running() {
    let user_data = tempdir();
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
    .test_value();
    wait_for_menu(&mut app);
    let pristine_scroll = app
        .assets
        .startup_dialog_images
        .get("GUIScroll.png")
        .test_value()
        .clone();
    let scenario = resolve_next_mission_scenario(&app.scensel.catalog, "Hazard.c4f/Tutorial.c4s")
        .test_value();

    // The user repro: starting any Hazard map used to refuse during
    // loading with a GlobalGuiBootstrapResources boundary because the
    // folder's Graphics.c4g overrides GUICaption/GUIScroll/GUIProgress.
    // C++ instead applies those overrides (C4GraphicsResource::Init →
    // C4GUI::Resource::Load over the registered set).
    app.start_scenario(scenario).test_value();
    wait_for_running_with_attempts(&mut app, 4_800);

    main_assert!(app.effective_global_gui_failures().is_empty());
    app.assets
        .require_classic_global_gui_bootstrap_resources(&HashMap::new())
        .test_value();
    for stem in ["GUICaption", "GUIScroll", "GUIProgress"] {
        let source = app
            .assets
            .active_gui_sheet_sources
            .get(stem)
            .unwrap_or_else(|| panic!("{stem} must be rebound while Hazard runs"));
        main_assert!(source.contains("Hazard.c4f") && source.contains("Graphics.c4g"), "{stem} must be won by the Hazard folder pack: {source}");
    }
    let running_scroll = app
        .assets
        .startup_dialog_images
        .get("GUIScroll.png")
        .test_value()
        .clone();
    main_assert_ne!(running_scroll.pixels() => pristine_scroll.pixels(), "the Hazard scroll sheet must replace the global surface");
    main_assert!(app.assets.message_dialog_resources().is_some(), "running dialogs resolve from the rebound sheets");

    app.return_to_menu();
    main_assert!(app.assets.active_gui_sheet_sources.is_empty());
    main_assert_eq!(
        app.assets
            .startup_dialog_images
            .get("GUIScroll.png")
            .expect("restored scroll sheet")
            .pixels()
            .as_ptr() =>
        pristine_scroll.pixels().as_ptr(),
        "teardown must restore the pristine startup scroll sheet"
    );
}

#[test]
fn real_alchemy_mouse_subcases_batch_1() {
    let prepared = PreparedRealInstalledScenario::new("Fantasy.c4f/Alchemy.c4s");
    let mut failures = Vec::new();
    run_real_alchemy_app_subcase(
        "right_click_positions_classic_context_magic_menu",
        &mut failures,
        || real_alchemy_right_click_positions_classic_context_magic_menu(&prepared),
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
    let prepared = PreparedRealInstalledScenario::new("Fantasy.c4f/Alchemy.c4s");
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
    main_assert!(failures.is_empty(), "Alchemy app subcase(s) failed: {}", failures.join(", "));
}

fn real_alchemy_right_click_positions_classic_context_magic_menu(
    prepared: &PreparedRealInstalledScenario,
) {
    // C4MouseControl issues C4CMD_Context on right-up with the clicked
    // MCLK as Target2. The command installs classic style-1 context on
    // the selected mage; entering ContextMagic opens the shipped spell
    // menu (C4MouseControl.cpp:1230-1263; C4Command.cpp:1076-1090;
    // MagiClonk.c4d/Script.c:190-199).
    let mut app = prepared.instantiate("Alchemy mouse context parity", false);
    let owner = app.players.local_owner;
    let mage = app.engine.test_crew_cursor(owner);
    main_assert_eq!(app.engine.object_snapshot(mage).expect("mage remains live").definition_id => "MCLK");
    main_assert_eq!(
        app.engine
            .object_snapshot(mage)
            .expect("mage remains live")
            .magic_energy =>
        0,
        "Alchemy's NMGE rule leaves raw mana at zero, so C++ draws no HUD mana bar"
    );

    // Scenario join leaves crew inside the home base with the same
    // queued Exit command as C++ startup. Let that command finish before
    // exercising a world click: contained objects are deliberately not
    // mouse targets in C4Game::FindVisObject.
    for _ in 0..80 {
        if app.engine.test_object_snapshot(mage).container.is_none() {
            break;
        }
        app.test_update();
    }
    main_assert!(app.engine.object_snapshot(mage).expect("mage remains live").container.is_none(), "Alchemy mage exits the home base before a world context click");

    let mut frame = vec![0_u8; 320 * 200 * 4];
    app.test_render(&mut frame);
    let rendered_mage = app.snapshot.object(mage).cloned().test_value();
    main_assert_ne!(rendered_mage.ocf => 0, "live MCLK carries a targetable cached OCF");
    // Aim at a point the target search actually resolves to the mage rather
    // than at its raw origin. FindVisObject's point search expands a short
    // object's box upward by addtop() = max(18 - Shape.Hgt, 0)
    // (src/C4Game.cpp:1476-1477, src/C4Object.h:340), and the Alchemy bag
    // lying beside the mage is 10 tall (ALC_ DefCore Height=10, Offset=-5,-5),
    // so its expanded box reaches the mage's own origin and wins on render
    // order. That is C++ behaviour, not a picking bug; this subcase is about
    // the mage's context menu, so it needs a point that targets the mage.
    let mage_point = mouse_test_object_point(&app, owner, mage);
    let (screen_x, screen_y) = (mage_point.x, mage_point.y);
    app.test_cursor(PhysicalPosition::new(
        f64::from(screen_x),
        f64::from(screen_y),
    ));
    main_assert_eq!(
        app.graphics
            .object_at_point(&app.snapshot, owner, GuiPoint::new(screen_x, screen_y),) =>
        Some(mage),
        "C++ front-to-back object picking selects the topmost MCLK",
    );
    let pointer = app.live_input.ingame_pointer.test_value();
    let projection = app
        .graphics
        .active_viewport_projections()
        .into_iter()
        .find(|viewport| viewport.owner == owner)
        .test_value();
    let (click_x, click_y) = ingame_pointer_viewport_pixel(pointer, projection);
    main_assert_ne!(click_x => 0, "fixture must enter C++'s free-alignment branch");
    main_assert_ne!(click_y => 0, "fixture must enter C++'s free-alignment branch");
    let click_location = Vector2::new(click_x, click_y);

    app.test_right_button(ElementState::Pressed);
    main_assert!(app.engine.cursor_object_menu(owner).is_none());
    app.test_right_button(ElementState::Released);
    app.test_update();

    main_assert!(app.object_menu.is_none(), "mouse context must use the classic engine menu, not the app fallback");
    let context = app.engine.cursor_object_menu(owner).test_value().1.clone();
    main_assert_eq!(context.style => 1);
    main_assert!(!context.permanent);
    main_assert_eq!(context.location => Some(click_location), "the synchronized Context command keeps logical viewport-local Tx/Ty");
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

    let viewport = app.graphics.viewport_rect(owner).test_value();
    app.test_render(&mut frame);
    let latched_screen = app
        .script_menu_presentations
        .get(&owner)
        .and_then(|state| state.location)
        .test_value();
    let latched_local = Vector2::new(
        latched_screen.0.saturating_sub(viewport.x),
        latched_screen.1.saturating_sub(viewport.y),
    );
    main_assert!(latched_local.x <= click_location.x && latched_local.y <= click_location.y, "right/bottom edges may clamp the menu back into the viewport");
    main_assert_eq!(
        app.ingame_menu_gfx
            .as_ref()
            .and_then(|gfx| gfx.menu_location) =>
        Some(latched_screen),
        "viewport-local coordinates are translated exactly once for drawing"
    );

    let mut moved_context = context.clone();
    let moved_x = latched_local.x.saturating_sub(4);
    main_assert_ne!(moved_x => latched_local.x, "fixture must leave room for relocation");
    moved_context.location = Some(Vector2::new(moved_x, latched_local.y));
    app.engine
        .apply_object_update(
            mage,
            ObjectUpdate {
                menu: Some(Some(moved_context.clone())),
                ..ObjectUpdate::default()
            },
        )
        .test_value();
    app.test_render(&mut frame);
    main_assert_eq!(
        app.script_menu_presentations
            .get(&owner)
            .and_then(|state| state.location) =>
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
        .test_value();
    app.test_render(&mut frame);
    let edge_latched = app
        .script_menu_presentations
        .get(&owner)
        .and_then(|state| state.location)
        .test_value();
    tall_context.items.pop();
    app.engine
        .apply_object_update(
            mage,
            ObjectUpdate {
                menu: Some(Some(tall_context)),
                ..ObjectUpdate::default()
            },
        )
        .test_value();
    app.test_render(&mut frame);
    main_assert_eq!(
        app.script_menu_presentations
            .get(&owner)
            .and_then(|state| state.location) =>
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
        .test_value();

    app.dispatch_control_event(ControlEvent::RawPlayerControl {
        command: clonk_engine::COM_MENU_SELECT,
        data: i32::try_from(magic_index).test_value(),
    })
    .test_value();
    app.dispatch_control_event(ControlEvent::RawPlayerControl {
        command: clonk_engine::COM_MENU_ENTER,
        data: 0,
    })
    .test_value();

    let spell_menu = app.engine.cursor_object_menu(owner).test_value().1;
    main_assert_eq!(spell_menu.extra => clonk_engine::ObjectMenuExtra::Components, "ALCO+NMGE uses C4MN_Extra_Components, never a mana footer");
    let raise_gravity = spell_menu
        .items
        .iter()
        .find(|item| item.item_id == "MGUP")
        .test_value();
    main_assert_eq!(
        raise_gravity.components =>
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
    let owner = app.players.local_owner;
    let original = app.engine.test_crew_cursor(owner);
    advance_app_until(
        &mut app,
        "Alchemy MCLK finishes its startup Exit",
        160,
        |app| {
            app.engine
                .object_snapshot(original)
                .is_some_and(|object| object.container.is_none() && object.command_stack.is_empty())
        },
    );

    let mut frame = vec![0_u8; 320 * 200 * 4];
    app.snapshot = app.engine.snapshot();
    app.test_render(&mut frame);
    let (original_x, original_y) = app
        .graphics
        .world_to_screen(owner, app.engine.test_object_snapshot(original).position)
        .test_value();
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
        .test_value();
    let target_position = Vector2::new(
        target_pointer.world.x.round() as i32,
        target_pointer.world.y.round() as i32,
    );
    let replacement = app.engine.spawn_test_object(
        SpawnConfig::new("MCLK")
            .with_position(target_position)
            .with_owner(owner)
            .with_crew_member(true),
    );

    app.test_update();
    app.snapshot = app.engine.snapshot();
    app.test_render(&mut frame);
    let target_position = app.engine.test_object_snapshot(replacement).position;
    let (target_x, target_y) = app
        .graphics
        .world_to_screen(owner, target_position)
        .test_value();
    let target = GuiPoint::new(target_x, target_y);
    let start = GuiPoint::new(target.x - 24.0, target.y - 24.0);
    main_assert_eq!(
        app.graphics.object_at_point(&app.snapshot, owner, target) =>
        Some(replacement),
        "right-up lands on the second mage, which would expose a collapsed context click"
    );
    main_assert_eq!(app.graphics.object_at_point(&app.snapshot, owner, start) => None, "right-down begins on ordinary landscape");

    app.test_cursor(PhysicalPosition::new(
        f64::from(start.x),
        f64::from(start.y),
    ));
    app.test_right_button(ElementState::Pressed);
    app.test_cursor(PhysicalPosition::new(
        f64::from(target.x),
        f64::from(target.y),
    ));
    let drag = app.ingame_right_mouse_state.test_value();
    main_assert_eq!(drag.motion.selection_kind => IngameDragSelectionKind::Crew);
    main_assert_eq!(app.ingame_selection_candidates(drag.motion) => vec![replacement], "C4MouseControl's transient Selection contains the framed crew");
    app.test_right_button(ElementState::Released);

    main_assert_eq!(app.engine.selected_crew(owner) => vec![replacement], "CID_PlrSelect replaces, rather than extends, the previous crew selection");
    main_assert_eq!(app.engine.crew_cursor(owner) => Some(replacement));
    main_assert!(app.engine.cursor_object_menu(owner).is_none(), "a completed selection drag must not fall through to C4CMD_Context");
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
    let owner = app.players.local_owner;
    let mage = app.engine.test_crew_cursor(owner);
    advance_app_until(
        &mut app,
        "Alchemy MCLK finishes its startup Exit",
        160,
        |app| {
            app.engine
                .object_snapshot(mage)
                .is_some_and(|object| object.container.is_none() && object.command_stack.is_empty())
        },
    );

    app.snapshot = app.engine.snapshot();
    app.refresh_focus();
    let mut frame = vec![0_u8; 320 * 200 * 4];
    app.test_render(&mut frame);
    let viewport = app.graphics.viewport_rect(owner).test_value();
    let (mage_x, mage_y) = app
        .graphics
        .world_to_screen(owner, app.engine.test_object_snapshot(mage).position)
        .test_value();
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
        .test_value();
    let layer = app.engine.test_object_snapshot(mage).layer;
    let spawn_bag = |app: &mut GameApp, position: Vector2| {
        let spawn = layer
            .map(|layer| {
                SpawnConfig::new("ALC_")
                    .with_position(position)
                    .with_layer(layer)
            })
            .unwrap_or_else(|| SpawnConfig::new("ALC_").with_position(position));
        app.engine.spawn_test_object(spawn)
    };
    let first_bag = spawn_bag(&mut app, anchor);
    let second_bag = spawn_bag(&mut app, Vector2::new(anchor.x + 20, anchor.y));
    for bag in [first_bag, second_bag] {
        main_assert_ne!(
            app.engine
                .object_snapshot(bag)
                .expect("spawned bag remains live")
                .ocf
                & clonk_engine::ocf::CARRYABLE =>
            0,
            "the regression target uses the shipped carryable definition"
        );
    }

    app.snapshot = app.engine.snapshot();
    app.test_render(&mut frame);
    let (first_x, first_y) = app.graphics.world_to_screen(owner, anchor).test_value();
    let (second_x, second_y) = app
        .graphics
        .world_to_screen(owner, Vector2::new(anchor.x + 20, anchor.y))
        .test_value();
    let frame_start = GuiPoint::new(first_x.min(second_x) - 24.0, first_y.min(second_y) - 24.0);
    let frame_end = GuiPoint::new(first_x.max(second_x) + 24.0, first_y.max(second_y) + 24.0);
    for point in [frame_start, frame_end] {
        main_assert!(app.graphics.viewport_point_at(point).is_some_and(|pointer| pointer.owner == owner), "selection frame endpoint remains in the local viewport");
        main_assert_eq!(app.graphics.object_at_point(&app.snapshot, owner, point) => None, "selection begins and ends on landscape");
    }

    app.test_cursor(PhysicalPosition::new(
        f64::from(frame_start.x),
        f64::from(frame_start.y),
    ));
    app.test_right_button(ElementState::Pressed);
    app.test_cursor(PhysicalPosition::new(
        f64::from(frame_end.x),
        f64::from(frame_end.y),
    ));
    let drag = app.ingame_right_mouse_state.test_value();
    main_assert_eq!(drag.motion.selection_kind => IngameDragSelectionKind::Objects);
    main_assert_eq!(app.ingame_selection_candidates(drag.motion) => vec![second_bag, first_bag], "object marks retain C++ Game.Objects newest-first order");
    app.test_right_button(ElementState::Released);
    main_assert!(
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
        .find(|point| app.graphics.object_at_point(&app.snapshot, owner, *point) == Some(first_bag))
        .test_value();
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
        .test_value();

    app.test_cursor(PhysicalPosition::new(
        f64::from(first_bag_point.x),
        f64::from(first_bag_point.y),
    ));
    app.test_right_button(ElementState::Pressed);
    app.test_cursor(PhysicalPosition::new(
        f64::from(drop_pointer.0.x),
        f64::from(drop_pointer.0.y),
    ));
    app.test_right_button(ElementState::Released);

    let commands = app
        .engine
        .test_object_snapshot(mage)
        .command_stack
        .command_views();
    main_assert_eq!(commands.len() => 2, "both framed bags receive commands");
    main_assert!(commands.iter().all(|command| command.name == "Drop"));
    main_assert_eq!(
        commands
            .iter()
            .map(|command| command.target)
            .collect::<Vec<_>>() =>
        vec![Some(second_bag), Some(first_bag)],
        "Game.Objects main-list order is preserved through Set then Append"
    );
    main_assert!(commands.iter().all(|command| {command.tx == Some(drop_pointer.1.x) && command.ty == Some(drop_pointer.1.y)}));
    main_assert!(app.engine.cursor_object_menu(owner).is_none());
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
    let owner = app.players.local_owner;
    let mage = app.engine.test_crew_cursor(owner);
    advance_app_until(
        &mut app,
        "Alchemy MCLK finishes its startup Exit",
        160,
        |app| {
            app.engine
                .object_snapshot(mage)
                .is_some_and(|object| object.container.is_none() && object.command_stack.is_empty())
        },
    );

    let hut = app
        .engine
        .snapshot()
        .objects
        .iter()
        .find(|object| object.definition_id == "AHUT" && object.owner == owner)
        .map(|object| object.id)
        .test_value();
    main_assert_ne!(app.engine.object_snapshot(hut).expect("AHUT remains live").ocf & clonk_engine::ocf::CONTAINER => 0, "AHUT is the C++ OCF_Container Put target");

    app.snapshot = app.engine.snapshot();
    app.refresh_focus();
    let mut frame = vec![0_u8; 320 * 200 * 4];
    app.test_render(&mut frame);
    let viewport = app.graphics.viewport_rect(owner).test_value();
    let hut_point = (viewport.y..viewport.y + viewport.height as i32)
        .flat_map(|y| {
            (viewport.x..viewport.x + viewport.width as i32)
                .map(move |x| GuiPoint::new(x as f32 + 0.5, y as f32 + 0.5))
        })
        .find(|point| app.graphics.object_at_point(&app.snapshot, owner, *point) == Some(hut))
        .test_value();
    let mouse_inset = 24;
    let bag_pointer = (viewport.y + mouse_inset
        ..viewport.y + viewport.height as i32 - mouse_inset)
        .step_by(4)
        .flat_map(|y| {
            (viewport.x + mouse_inset..viewport.x + viewport.width as i32 - mouse_inset)
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
        .test_value();
    let bag_position = ingame_pointer_world_pixel(bag_pointer);
    let mut bag_spawn = SpawnConfig::new("ALC_").with_position(bag_position);
    if let Some(layer) = app.engine.test_object_snapshot(mage).layer {
        bag_spawn = bag_spawn.with_layer(layer);
    }
    let bag = app.engine.spawn_test_object(bag_spawn);

    app.snapshot = app.engine.snapshot();
    app.test_render(&mut frame);
    let bag_point = (viewport.y..viewport.y + viewport.height as i32)
        .flat_map(|y| {
            (viewport.x..viewport.x + viewport.width as i32)
                .map(move |x| GuiPoint::new(x as f32, y as f32))
        })
        .find(|point| {
            app.graphics.object_at_point(&app.snapshot, owner, *point) == Some(bag)
                && app.ingame_viewport_region(owner, *point).is_none()
        })
        .test_value();
    main_assert_ne!(app.snapshot.object(bag).test_value().ocf & clonk_engine::ocf::CARRYABLE => 0);
    main_assert_eq!(app.ingame_primary_mouse_target(owner, bag_point) => Some(bag));

    app.test_modifiers(ModifiersState::CONTROL);
    app.test_cursor(PhysicalPosition::new(
        f64::from(bag_point.x),
        f64::from(bag_point.y),
    ));
    // MouseControl's first Move after Init is viewport-centered; the next
    // Move performs the target refill consumed by RightDown
    // (C4MouseControl.cpp:216-239,259-315,1009-1023).
    app.test_cursor(PhysicalPosition::new(
        f64::from(bag_point.x),
        f64::from(bag_point.y),
    ));
    main_assert_eq!(app.live_input.ingame_mouse_target => Some(bag));
    app.test_right_button(ElementState::Pressed);
    app.test_cursor(PhysicalPosition::new(
        f64::from(hut_point.x),
        f64::from(hut_point.y),
    ));
    // The first Move crosses the drag threshold in DragNone; the next Move
    // runs DragMoving and refills the Put target before button-up
    // (C4MouseControl.cpp:893-980,742-770).
    app.test_cursor(PhysicalPosition::new(
        f64::from(hut_point.x),
        f64::from(hut_point.y),
    ));
    main_assert!(app.ingame_right_mouse_state.is_some_and(|state| state.motion.world_drag_started));
    main_assert_eq!(app.live_input.ingame_mouse_caption.cursor => IngameMouseCursorKind::Put);
    main_assert_eq!(app.live_input.ingame_mouse_target => Some(hut));
    app.test_right_button(ElementState::Released);
    app.test_modifiers(ModifiersState::empty());

    let commands = app
        .engine
        .test_object_snapshot(mage)
        .command_stack
        .command_views();
    main_assert_eq!(commands.len() => 1, "the drag emits exactly one Put");
    main_assert_eq!(commands[0].name => "Put");
    main_assert_eq!(commands[0].target => Some(hut));
    main_assert_eq!(commands[0].target2 => Some(bag));
    main_assert_eq!(commands[0].tx => None);
    main_assert_eq!(commands[0].ty => None);
    main_assert!(app.engine.cursor_object_menu(owner).is_none());
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
    let owner = app.players.local_owner;
    let mage = app.engine.test_crew_cursor(owner);
    advance_app_until(
        &mut app,
        "Alchemy MCLK finishes its startup Exit",
        160,
        |app| {
            app.engine
                .object_snapshot(mage)
                .is_some_and(|object| object.container.is_none() && object.command_stack.is_empty())
        },
    );

    app.snapshot = app.engine.snapshot();
    app.refresh_focus();
    let mut frame = vec![0_u8; 320 * 200 * 4];
    app.test_render(&mut frame);
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
        .test_value();
    let bag_position = Vector2::new(
        empty_pointer.world.x.round() as i32,
        empty_pointer.world.y.round() as i32,
    );
    let mut bag_spawn = SpawnConfig::new("ALC_").with_position(bag_position);
    if let Some(layer) = app.engine.test_object_snapshot(mage).layer {
        bag_spawn = bag_spawn.with_layer(layer);
    }
    let bag = app.engine.spawn_test_object(bag_spawn);
    let bag_snapshot = app.engine.test_object_snapshot(bag);
    main_assert_ne!(bag_snapshot.ocf & clonk_engine::ocf::CARRYABLE => 0, "the regression target uses the shipped carryable definition");

    // FindVisObject's OCF filter is part of the pick itself. A newer
    // foreground object with no primary mouse OCF must therefore be
    // skipped rather than blocking the carryable object behind it.
    let mut blocker = test_definition("MBLK", "Mouse blocker", "#strict\n");
    blocker.set_category(clonk_engine::CATEGORY_OBJECT);
    blocker.set_shape_rect(Some(clonk_engine::DefinitionRect::new(-3, -3, 6, 6)));
    app.engine.register_test_definition(blocker);
    let mut blocker_spawn = SpawnConfig::new("MBLK").with_position(bag_position);
    if let Some(layer) = bag_snapshot.layer {
        blocker_spawn = blocker_spawn.with_layer(layer);
    }
    let blocker = app.engine.spawn_test_object(blocker_spawn);

    app.snapshot = app.engine.snapshot();
    app.test_render(&mut frame);
    let viewport = app.graphics.viewport_rect(owner).test_value();
    let bag_point = (viewport.y..viewport.y + viewport.height as i32)
        .flat_map(|y| {
            (viewport.x..viewport.x + viewport.width as i32)
                .map(move |x| GuiPoint::new(x as f32 + 0.5, y as f32 + 0.5))
        })
        .find(|point| {
            app.graphics.object_at_point(&app.snapshot, owner, *point) == Some(blocker)
                && app.ingame_primary_mouse_target(owner, *point) == Some(bag)
        })
        .test_value();
    app.test_cursor(PhysicalPosition::new(
        f64::from(bag_point.x),
        f64::from(bag_point.y),
    ));
    let click_world = ingame_pointer_world_pixel(app.live_input.ingame_pointer.test_value());
    main_assert_eq!(app.graphics.object_at_point(&app.snapshot, owner, bag_point) => Some(blocker), "the unfiltered foreground pick sees the newer blocker",);
    main_assert_eq!(app.ingame_primary_mouse_target(owner, bag_point) => Some(bag), "the primary mouse OCF pick skips that blocker and resolves the carryable",);

    app.test_left_button(ElementState::Pressed);
    app.test_left_button(ElementState::Released);
    let first_click = app
        .engine
        .test_object_snapshot(mage)
        .command_stack
        .command_views();
    main_assert_eq!(first_click.len() => 1);
    main_assert_eq!(first_click[0].name => "MoveTo");
    main_assert_eq!(first_click[0].target => None);
    main_assert_eq!(first_click[0].tx => Some(click_world.x));
    main_assert_eq!(first_click[0].ty => Some(click_world.y));

    app.test_left_button(ElementState::Pressed);
    let double_click = app
        .engine
        .test_object_snapshot(mage)
        .command_stack
        .command_views();
    main_assert_eq!(double_click.len() => 1);
    main_assert_eq!(double_click[0].name => "Get");
    main_assert_eq!(double_click[0].target => Some(bag));
    main_assert_eq!(double_click[0].tx => None);
    main_assert_eq!(double_click[0].ty => None);

    app.test_left_button(ElementState::Released);
    main_assert_eq!(
        app.engine
            .object_snapshot(mage)
            .expect("mage remains live after ignored release")
            .command_stack
            .command_views() =>
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
    let owner = app.players.local_owner;
    let rider = app.engine.test_crew_cursor(owner);
    advance_app_until(
        &mut app,
        "Tutorial06 selected CLNK completes its startup Exit",
        160,
        |app| {
            app.engine
                .object_snapshot(rider)
                .is_some_and(|object| object.container.is_none() && object.action.name == "Walk")
        },
    );

    app.engine
        .execute_shake_circle_operation(Vector2::new(332, 250), 180);
    let elevator = app.engine.spawn_test_object(
        SpawnConfig::new("ELEV")
            .with_position(Vector2::new(332, 150))
            .with_owner(owner),
    );
    let first = app.engine.snapshot();
    let elevator = first.object(elevator).test_value();
    let case_id = elevator.action.target.test_value();
    let case = first.object(case_id).test_value();
    main_assert_eq!(case.definition_id => "ELEC");

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
        .test_value();
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
        .test_value();

    // The setup mutations above stand in for the object phase. C++
    // copies the selected ViewCursor position into ViewX/ViewY in the
    // later player phase (C4Player.cpp:200-209,1693-1713).
    app.engine.tick_player_systems().test_value();

    app.focus_id = Some(rider);
    app.snapshot = app.engine.snapshot();
    app.refresh_focus();
    let initial_snapshot = app.snapshot.clone();
    let initial_inputs = collect_viewport_inputs(&initial_snapshot).test_value();
    main_assert_eq!(initial_inputs.len() => 1);
    main_assert_eq!(initial_inputs[0].focus.expect("player viewport focus").id => rider);
    main_assert_eq!(
        initial_inputs[0].center =>
        app.snapshot.object(rider).expect("initial rider").position,
        "C4Player::UpdateView follows the live ViewCursor position"
    );
    app.graphics
        .render_frame(&initial_snapshot, &initial_inputs);

    let initial_case = app.snapshot.object(case_id).test_value().position;
    let initial_rider = app.snapshot.object(rider).test_value().position;
    let initial_world_origin = app
        .graphics
        .world_to_screen(owner, Vector2::ZERO)
        .test_value()
        .1;
    let initial_rider_screen = app
        .graphics
        .world_to_screen(owner, initial_rider)
        .test_value()
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
        main_assert_eq!((rider_now.action.name.as_str(), rider_now.action.target) => ("Push", Some(case_id)), "real PUSH attachment survives frame {frame}");
        main_assert!(
                (rider_now.position.y - case.position.y - rider_offset.y).abs() <= 1,
                "rider and carriage cannot diverge on frame {frame}: rider={rider_now:?}, case={case:?}"
            );

        let render_snapshot = app.snapshot.clone();
        let inputs = collect_viewport_inputs(&render_snapshot).test_value();
        main_assert_eq!(inputs.len() => 1, "one local viewport on frame {frame}");
        main_assert_eq!(inputs[0].focus.expect("player viewport focus").id => rider);
        main_assert_eq!(inputs[0].center => rider_now.position, "the app must present the rider's current frame position to C4Viewport on frame {frame}");
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

    main_assert!(samples.last().expect("final sample").0 > samples[0].0, "the real ELEC must move during the sample: {samples:?}");
    for pair in samples.windows(2) {
        let [before, after] = pair else {
            unreachable!()
        };
        main_assert!(after.0 >= before.0 && after.1 >= before.1, "carriage/rider reversed between frames: {before:?} -> {after:?}");
        main_assert!(after.2 <= before.2, "the fixed-point C4Viewport camera reversed between frames: {before:?} -> {after:?}");
        main_assert!(after.3 >= before.3, "the rider jittered backwards on screen: {before:?} -> {after:?}");
    }
}

#[test]
fn overlay_text_helper_respects_custom_text() {
    main_assert!(overlay_text_needs_update("", "FRAME "));
    main_assert!(overlay_text_needs_update("FRAME 00005", "FRAME "));
    main_assert!(!overlay_text_needs_update("Inventory open", "FRAME "));

    main_assert!(overlay_text_needs_update("", "ENERGY "));
    main_assert!(overlay_text_needs_update("ENERGY 100 DAMAGE 000 OWNER 1", "ENERGY "));
    main_assert!(!overlay_text_needs_update("Paused", "ENERGY "));

    main_assert_eq!(c4_presentation_text(&clonk_script::c4_string_from_bytes(&[0xe9])) => "\u{e9}");

    let raw_name = clonk_script::c4_string_from_bytes(&[0xe9]);
    main_assert_eq!(player_join_board_line(&raw_name) => "Player join: \u{e9}");
}

#[test]
fn real_tutorial01_message_render_subcases_batch() {
    let prepared = PreparedRealInstalledScenario::new("Tutorial.c4f/Tutorial01.c4s");
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
        .test_value()
        .clone();
    main_assert_eq!(message.kind => MessageKind::GlobalPlayer);
    main_assert_eq!(message.player => Some(app.players.local_owner));
    main_assert_eq!(message.target => None);
    main_assert_eq!(message.lines => ["Welcome to the world of Clonk."]);
    main_assert_eq!(message.offset => Vector2::new(50, 50));
    main_assert_eq!(message.color => 0xffff_ffff);
    main_assert_eq!(message.flags => 0x718);
    main_assert_eq!(message.width => Some(30));
    main_assert_eq!(message.decoration.as_deref() => Some("DECO"));
    main_assert_eq!(message.portrait.as_deref() => Some("Portrait:SCLK::0000ff::1"));

    let decoration = message.frame_decoration.test_ref();
    main_assert_eq!(decoration.source_definition => "DECO");
    main_assert_eq!(decoration.background_color => 0x8032_3232);
    main_assert_eq!((decoration.border_top, decoration.border_left, decoration.border_right, decoration.border_bottom,) => (0, 0, 0, 0));
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
        let facet = facet.test_value();
        (
            facet.x,
            facet.y,
            facet.width,
            facet.height,
            facet.target_x,
            facet.target_y,
        )
    });
    main_assert_eq!(
        facets =>
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

    app.resize(1152, 644).test_value();
    hold_message_board_for_frame_comparison(&mut app);
    let messages = std::mem::take(&mut app.snapshot.hud.messages);
    let mut warm = vec![0_u8; 1152 * 644 * 4];
    app.test_render(&mut warm);
    let frame_gamma = app
        .graphics
        .active_gamma_ramp(&app.snapshot.environment.gamma);
    let mut baseline = vec![0_u8; 1152 * 644 * 4];
    app.test_render(&mut baseline);
    app.snapshot.hud.messages = messages;
    let mut rendered = vec![0_u8; 1152 * 644 * 4];
    app.test_render(&mut rendered);

    let viewport = app
        .graphics
        .active_viewport_projections()
        .into_iter()
        .find(|viewport| viewport.owner == app.players.local_owner)
        .test_value()
        .rect;
    main_assert_eq!(viewport => Rect::new(216, 56, 720, 560));
    let fonts = app.assets.clonk_fonts.as_deref().test_value();
    main_assert_eq!(fonts.text.measure("Welcome to the world of Clonk.", true) => (194, 22));

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
    main_assert!(!changed.is_empty(), "the C4GameMessage contributes pixels");
    main_assert!(changed.iter().all(|index| {
        let x = (*index % 1152) as i32;
        let y = (*index / 1152) as i32;
        inside(viewport, x, y) && inside(deco_envelope, x, y)
    }));
    main_assert!(
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
    main_assert_eq!(
        pixel(&rendered, 572, 100) =>
        clonk_frontend::gamma_encode_fragment(Color::opaque(126, 66, 23), &frame_gamma),
        "the opaque top-left DECO texel must draw outside the core frame"
    );

    let mut expected_gap = Surface::new(1, 1, clonk_graphics::PixelFormat::Rgba8888);
    expected_gap
        .set_pixel(0, 0, pixel(&baseline, 645, 130))
        .test_value();
    clonk_frontend::classic_gui::draw_engine_box(
        &mut expected_gap,
        0,
        0,
        0,
        0,
        0x8032_3232,
        Some(&frame_gamma),
    );
    main_assert_eq!(
        pixel(&rendered, 645, 130) =>
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
    main_assert!(app.can_defer_native_game_messages(3.0));

    let gamma = app
        .graphics
        .active_gamma_ramp(&app.snapshot.environment.gamma);
    let mut presenter = clonk_scaling::FramePresenter::new(3.0, 960, 598);
    let mut output = vec![0_u8; 960 * 598 * 4];
    let refreshed = presenter
        .present(&mut output, |frame| {
            app.render_for_presentation(frame, false, false, true)
        })
        .test_value();
    main_assert!(refreshed);
    let filtered_base = output.clone();

    app.render_native_game_messages(&mut output, presenter.presentation_geometry(), &gamma)
        .test_value();
    main_assert_ne!(output => filtered_base, "the physical C4GameMessage pass must contribute message pixels");

    // A 320x200 logical surface creates a nominal 960x600 lower-left GL
    // viewport in a 960x598 framebuffer, clipping two physical rows from
    // the top. Native message pixels must retain that offset and the
    // owning C4Viewport clip.
    let viewport = app
        .graphics
        .active_viewport_projections()
        .into_iter()
        .find(|viewport| viewport.owner == app.players.local_owner)
        .test_value()
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
        main_assert!(physical_viewport.intersection(point).is_some(), "native message pixel ({}, {}) escaped its viewport clip", point.x, point.y);
    }
    main_assert!(changed_count > 0);

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
        .test_value();
    app.render_native_game_messages(&mut clipped, clipped_geometry, &gamma)
        .test_value();
    for y in 0..598_usize {
        let clipped_row = &clipped[y * 960 * 4..(y + 1) * 960 * 4];
        let nominal_row = &nominal[(y + 2) * 960 * 4..(y + 3) * 960 * 4];
        main_assert_eq!(clipped_row => nominal_row, "the 598-row framebuffer must clip nominal physical row {}", y + 2);
    }
}

#[test]
fn real_tutorial09_hud_names_subcases_batch() {
    let prepared = PreparedRealInstalledScenario::new("Tutorial.c4f/Tutorial09.c4s");
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
    main_assert!(failures.is_empty(), "Tutorial09 app subcase(s) failed: {}", failures.join(", "));
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
    app.test_update();

    let clonk = app
        .snapshot
        .players
        .iter()
        .find(|player| player.id == app.players.local_owner)
        .and_then(|player| player.cursor)
        .test_value();
    let object = app.snapshot.object(clonk).test_value();
    let current_breath = object.breath;
    let capacity = app
        .engine
        .find_object_index(clonk)
        .map(|index| app.engine.object_physical(index).breath)
        .test_value();
    main_assert_eq!(current_breath => 50_000, "CLNK keeps its birth breath");
    main_assert_eq!(capacity => 250_000, "Tutorial09 installs AquaClonk capacity");

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
        .find(|player| player.owner == app.players.local_owner)
        .and_then(|player| player.crew.iter().find(|crew| crew.object_id == clonk))
        .test_value();
    main_assert_eq!(crew.breath => 50_000);
    main_assert_eq!(crew.breath_capacity => 250_000);
    main_assert!(crew.breath != 0 && crew.breath < crew.breath_capacity);

    hold_message_board_for_frame_comparison(&mut app);

    // The stock EnergyBars.png is split into six 8px columns and three
    // 12px cap/tile rows (C4GraphicsResource.cpp:231-241). With portraits
    // enabled, an energy bar already occupying slot zero, and no magic,
    // the breath bar occupies x=5+(8+1), y=35+10+10, h=200-95. Its
    // filled pixels come from cyan columns 4/5 selected by bar_idx=2
    // (C4Facet.cpp:334-387).
    let hud = app.graphics.hud_graphics();
    let bars = hud.energy_bars.test_ref();
    main_assert_eq!((bars.width(), bars.height()) => (48, 36));
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
    main_assert!(!painted.is_empty(), "real cyan breath asset draws pixels");
    main_assert!(painted.iter().all(|(x, y, _)| (14..22).contains(x) && (55..160).contains(y)));
    main_assert!(
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
        .test_value()
        .breath = capacity;
    app.render_running(&mut frame, false).test_value();
    app.render_running(&mut frame, false).test_value();
    let without_breath = frame.clone();

    app.snapshot
        .objects
        .iter_mut()
        .find(|object| object.id == clonk)
        .test_value()
        .breath = current_breath;
    app.render_running(&mut frame, false).test_value();
    let with_breath = frame.clone();

    app.snapshot
        .objects
        .iter_mut()
        .find(|object| object.id == clonk)
        .test_value()
        .breath = capacity;
    app.render_running(&mut frame, false).test_value();
    main_assert_eq!(frame => without_breath, "the stationary real frame is otherwise deterministic");

    let viewport = app.graphics.viewport_rect(app.players.local_owner).test_value();
    let bar_x = viewport.x + 14;
    let bar_y = viewport.y + 55;
    let bar_height = viewport.height as i32 - 95;
    main_assert!(bar_height > 0, "C++ viewport height gate permits HUD bars");
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
    main_assert!(!changed.is_empty(), "partial real Tutorial09 breath paints the HUD");
    main_assert!(
        changed.iter().all(|(x, y, _)| {
            (bar_x..bar_x + 8).contains(x) && (bar_y..bar_y + bar_height).contains(y)
        }),
        "breath-only fragments stay inside the C++ bar rectangle: {changed:?}"
    );
    main_assert!(changed.iter().any(|(_, y, _)| *y < fill_y), "the empty breath source column paints above yBar");
    main_assert!(
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
        keyboard.press(key);
        main_assert_ne!(keyboard.player_control().pressed_coms & (1 << com) => 0, "{key:?} must reach the matching C4Player::InCom bit");
        keyboard.release(key);
        main_assert_eq!(
            keyboard.player_control().pressed_coms & (1 << com) =>
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
        keyboard.press(key);
        keyboard.release(key);
    }
    main_assert_eq!(keyboard.player_control() => before_arrows);
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
    let clonk = app.engine.test_crew_cursor(app.players.local_owner);
    let hut = app_object_with_definition(&app, "HUT2").test_value();

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
    let flag = app_object_with_definition(&app, "FLAG").test_value();

    // Held Z supplies horizontal jump momentum. Each physical S tap is
    // separated by twelve app ticks, beyond C4DoubleClick's ten-tick
    // window, and its release must preserve the still-held Z bit.
    {
        let mut keyboard = AppVirtualKeyboard::new(&mut app);
        keyboard.press(VirtualKeyCode::KeyZ);
    }
    for _ in 0..30 {
        let clonk_now = app.engine.test_object_snapshot(clonk);
        if app_clonk_carries(&app, clonk, "FLAG") || clonk_now.position.x <= 25 {
            break;
        }
        if clonk_now.action.name == "Walk" {
            let mut keyboard = AppVirtualKeyboard::new(&mut app);
            keyboard.tap(VirtualKeyCode::KeyS);
            main_assert_ne!(keyboard.player_control().pressed_coms & (1 << clonk_engine::COM_LEFT) => 0, "releasing S must preserve held Z/Left");
        }
        for _ in 0..12 {
            app.test_update();
        }
    }
    {
        let mut keyboard = AppVirtualKeyboard::new(&mut app);
        keyboard.release(VirtualKeyCode::KeyZ);
    }
    if !app_clonk_carries(&app, clonk, "FLAG") {
        advance_app_until(&mut app, "CLNK lands beside FLAG", 80, |app| {
            app.engine
                .object_snapshot(clonk)
                .is_some_and(|object| object.action.name == "Walk" && object.position.x <= 40)
        });
        {
            let mut keyboard = AppVirtualKeyboard::new(&mut app);
            keyboard.press(VirtualKeyCode::KeyC);
        }
        advance_app_until(&mut app, "CLNK naturally collects FLAG", 40, |app| {
            app_clonk_carries(app, clonk, "FLAG")
        });
        AppVirtualKeyboard::new(&mut app).release(VirtualKeyCode::KeyC);
    }
    main_assert_eq!(app.engine.object_snapshot(flag).expect("collected FLAG").container => Some(clonk));
    main_assert!(app_cursor_inventory_contains(&mut app, clonk, "FLAG"), "the collected FLAG must reach the rendered cursor inventory");
    app.snapshot.hud.messages.clear();
    let mut rendered = vec![0_u8; 320 * 200 * 4];
    app.test_render(&mut rendered);

    advance_app_until(&mut app, "Tutorial01 points toward the cabin", 500, |app| {
        app_tutorial_message_contains(app, "cabin on the hill to your right")
    });
    {
        let mut keyboard = AppVirtualKeyboard::new(&mut app);
        keyboard.press(VirtualKeyCode::KeyC);
    }
    for _ in 0..90 {
        let clonk_now = app.engine.test_object_snapshot(clonk);
        if clonk_now.position.x >= 558 {
            break;
        }
        if clonk_now.action.name == "Walk" {
            let mut keyboard = AppVirtualKeyboard::new(&mut app);
            keyboard.tap(VirtualKeyCode::KeyS);
            main_assert_ne!(keyboard.player_control().pressed_coms & (1 << clonk_engine::COM_RIGHT) => 0, "releasing S must preserve held C/Right");
        }
        for _ in 0..12 {
            app.test_update();
        }
    }
    AppVirtualKeyboard::new(&mut app).release(VirtualKeyCode::KeyC);
    advance_app_until(&mut app, "CLNK lands beside HUT2", 60, |app| {
        app.engine
            .object_snapshot(clonk)
            .is_some_and(|object| object.action.name == "Walk")
    });
    AppVirtualKeyboard::new(&mut app).press(VirtualKeyCode::KeyZ);
    advance_app_until(&mut app, "CLNK aligns with HUT2 entrance", 20, |app| {
        app.engine
            .object_snapshot(clonk)
            .is_some_and(|object| object.position.x <= 570)
    });
    {
        let mut keyboard = AppVirtualKeyboard::new(&mut app);
        keyboard.release(VirtualKeyCode::KeyZ);
        keyboard.tap(VirtualKeyCode::KeyS);
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
            .cursor_object_menu(app.players.local_owner)
            .is_some_and(|(_, menu)| {
                menu.selection == 0 && menu.items.first().is_some_and(|item| item.caption == "Put")
            })
    });
    AppVirtualKeyboard::new(&mut app).tap(VirtualKeyCode::KeyA);
    advance_app_until(&mut app, "FLAG enters HUT2", 80, |app| {
        app.engine
            .object_snapshot(flag)
            .is_some_and(|object| object.container == Some(hut))
    });
    advance_app_until(&mut app, "FLAG makes HUT2 the player base", 80, |app| {
        app.engine
            .object_snapshot(hut)
            .is_some_and(|object| object.base == app.players.local_owner)
    });
    advance_app_until(
        &mut app,
        "Tutorial01 Exit prompt and context row",
        450,
        |app| {
            app_tutorial_message_contains(app, "select 'Exit'")
                && app
                    .engine
                    .cursor_object_menu(app.players.local_owner)
                    .is_some_and(|(_, menu)| menu.items.iter().any(|item| item.caption == "Exit"))
        },
    );

    // Script148 highlights physical X/Down plus A. Move down through the
    // real context rows, including any Buy/Sell rows enabled by the base,
    // rather than selecting Exit by index or mutating menu state.
    let context_items = app
        .engine
        .cursor_object_menu(app.players.local_owner)
        .test_value()
        .1
        .items
        .len();
    for _ in 0..=context_items {
        let exit_selected = app
            .engine
            .cursor_object_menu(app.players.local_owner)
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
        AppVirtualKeyboard::new(&mut app).tap(VirtualKeyCode::KeyX);
    }
    main_assert!(
        app.engine
            .cursor_object_menu(app.players.local_owner)
            .and_then(|(_, menu)| usize::try_from(menu.selection)
                .ok()
                .map(|index| (menu, index)))
            .and_then(|(menu, index)| menu.items.get(index))
            .is_some_and(|item| item.caption == "Exit"),
        "physical X must select the real Exit row"
    );
    AppVirtualKeyboard::new(&mut app).tap(VirtualKeyCode::KeyA);
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
    let gold = app_object_with_definition(&app, "GOLD").test_value();
    AppVirtualKeyboard::new(&mut app).press(VirtualKeyCode::KeyZ);
    advance_app_until(
        &mut app,
        "CLNK reaches the digging lesson area",
        260,
        |app| {
            app.engine.object_snapshot(clonk).is_some_and(|object| {
                (150..250).contains(&object.position.x) && (250..350).contains(&object.position.y)
            })
        },
    );
    AppVirtualKeyboard::new(&mut app).release(VirtualKeyCode::KeyZ);
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
    AppVirtualKeyboard::new(&mut app).tap(VirtualKeyCode::KeyD);
    advance_app_until(&mut app, "CLNK starts real Dig action", 30, |app| {
        app.engine
            .object_snapshot(clonk)
            .is_some_and(|object| object.action.name == "Dig")
    });
    main_assert!(app.engine.frame().saturating_sub(dig_press_frame) > 10, "physical D must wait through C4DoubleClick before DigSingle");
    {
        let mut keyboard = AppVirtualKeyboard::new(&mut app);
        keyboard.press(VirtualKeyCode::KeyX);
        keyboard.press(VirtualKeyCode::KeyZ);
        let control = keyboard.player_control();
        main_assert_ne!(control.pressed_coms & (1 << clonk_engine::COM_DOWN) => 0);
        main_assert_ne!(control.pressed_coms & (1 << clonk_engine::COM_LEFT) => 0);
        main_assert_eq!(keyboard.engine().object_snapshot(clonk).expect("CLNK after X+Z").command_direction => CommandDirection::DownLeft);
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
        keyboard.release(VirtualKeyCode::KeyX);
        let control = keyboard.player_control();
        main_assert_eq!(control.pressed_coms & (1 << clonk_engine::COM_DOWN) => 0);
        main_assert_ne!(control.pressed_coms & (1 << clonk_engine::COM_LEFT) => 0);
        let clonk_now = keyboard.engine().test_object_snapshot(clonk);
        main_assert_eq!(clonk_now.action.name => "Dig");
        main_assert_eq!(clonk_now.command_direction => CommandDirection::Left);
    }
    advance_app_until(
        &mut app,
        "leftward Dig naturally collects GOLD",
        180,
        |app| app_clonk_carries(app, clonk, "GOLD"),
    );
    AppVirtualKeyboard::new(&mut app).release(VirtualKeyCode::KeyZ);
    main_assert_eq!(app.engine.object_snapshot(gold).expect("collected GOLD").container => Some(clonk));
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
    main_assert!(app_cursor_inventory_contains(&mut app, clonk, "GOLD"), "the collected GOLD must reach the rendered cursor inventory");
    // Typed C4GameMessage rejection has its own regression; isolate this
    // inventory-render assertion from that unported overlay.
    app.snapshot.hud.messages.clear();
    app.test_render(&mut rendered);

    // Walk out of the excavated tunnel, then preserve held physical C
    // while reacting to the same Walk/Scale/Jump transitions as the
    // engine virtual route. Re-pressing C on entry to DFA_SCALE supplies
    // the edge C++ uses to let go or climb; an S tap on landing/flight
    // transitions jumps clear without assigning position or action
    // (C4Object.cpp:3618-3628,4823-4855).
    AppVirtualKeyboard::new(&mut app).press(VirtualKeyCode::KeyC);
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
    AppVirtualKeyboard::new(&mut app).release(VirtualKeyCode::KeyC);
    AppVirtualKeyboard::new(&mut app).press(VirtualKeyCode::KeyC);
    let mut previous_action = String::new();
    for _ in 0..1_800 {
        let clonk_now = app.engine.test_object_snapshot(clonk);
        if clonk_now.position.x >= 558 {
            break;
        }
        let action = clonk_now.action.name.clone();
        let entered_scale = action.starts_with("Scale") && !previous_action.starts_with("Scale");
        let left_scale_in_flight = action == "Jump" && previous_action.starts_with("Scale");
        let landed = action == "Walk" && previous_action != "Walk";
        if entered_scale {
            let mut keyboard = AppVirtualKeyboard::new(&mut app);
            keyboard.release(VirtualKeyCode::KeyC);
            keyboard.press(VirtualKeyCode::KeyC);
        } else if landed || left_scale_in_flight {
            let mut keyboard = AppVirtualKeyboard::new(&mut app);
            keyboard.tap(VirtualKeyCode::KeyS);
            main_assert_ne!(keyboard.player_control().pressed_coms & (1 << clonk_engine::COM_RIGHT) => 0, "releasing S must preserve held C during the return climb");
        }
        previous_action = action;
        app.test_update();
    }
    AppVirtualKeyboard::new(&mut app).release(VirtualKeyCode::KeyC);
    main_assert!(app.engine.object_snapshot(clonk).is_some_and(|object| object.position.x >= 558), "the GOLD-carrying CLNK must reach the cabin hill naturally");
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
    AppVirtualKeyboard::new(&mut app).press(VirtualKeyCode::KeyZ);
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
        keyboard.release(VirtualKeyCode::KeyZ);
        keyboard.tap(VirtualKeyCode::KeyS);
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
    main_assert!(app.snapshot.round_results.fulfilled_goals.iter().any(|goal| goal == "SCRG"), "Tutorial01 must fulfill its real SCRG before GameOver");
    main_assert_eq!(
        app.engine.next_mission().path =>
        r"Tutorial.c4f\Tutorial02.c4s"
    );
    // The typed C4GameMessage guard has a dedicated regression.
    app.snapshot.hud.messages.clear();
    app.test_render(&mut rendered);
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

    let clonk = app.engine.test_crew_cursor(app.players.local_owner);
    let balloon = app
        .engine
        .snapshot()
        .objects
        .into_iter()
        .find(|object| object.definition_id == "BALN")
        .test_value()
        .id;
    let hut = app_object_with_definition(&app, "HUT3").test_value();
    let loam_menu_identification =
        serde_json::from_value(serde_json::json!({ "C4Id": "LMMS" })).test_value();

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
        app.test_update();
    }
    main_assert!(
        app.engine
            .object_snapshot(clonk)
            .is_some_and(|object| object.container.is_none() && object.action.name == "Walk"),
        "Tutorial02 CLNK exits the starting base through app frames"
    );

    {
        let mut keyboard = AppVirtualKeyboard::new(&mut app);
        keyboard.press(VirtualKeyCode::KeyX);
        keyboard.release(VirtualKeyCode::KeyX);
        keyboard.press(VirtualKeyCode::KeyX);
    }
    for _ in 0..80 {
        if app.engine.object_snapshot(clonk).is_some_and(|object| {
            object.action.name == "Push" && object.action.target == Some(balloon)
        }) {
            break;
        }
        app.test_update();
    }
    let pushing = app.engine.test_object_snapshot(clonk);
    let balloon_before = app.engine.test_object_snapshot(balloon);
    main_assert_eq!((pushing.action.name.as_str(), pushing.action.target) => ("Push", Some(balloon)), "physical X/X must grab BALN through GameApp");
    let platform_delta_y = pushing.position.y - balloon_before.position.y;

    {
        let mut keyboard = AppVirtualKeyboard::new(&mut app);
        keyboard.release(VirtualKeyCode::KeyX);
        keyboard.press(VirtualKeyCode::KeyS);
    }
    for lift_frame in 1..=20 {
        app.test_update();
        let clonk_now = app.engine.test_object_snapshot(clonk);
        let balloon_now = app.engine.test_object_snapshot(balloon);
        main_assert_eq!(
            (clonk_now.action.name.as_str(), clonk_now.action.target) =>
            ("Push", Some(balloon)),
            "DFA_PUSH must retain BALN on app lift frame {lift_frame}"
        );
        main_assert!(
            (clonk_now.position.y - balloon_now.position.y - platform_delta_y).abs() <= 1,
            "CLNK must remain on BALN's platform on app lift frame {lift_frame}; \
                 initial delta={platform_delta_y}, clonk={clonk_now:?}, balloon={balloon_now:?}"
        );
    }
    main_assert!(app.engine.object_snapshot(balloon).expect("BALN after lift").position.y < balloon_before.position.y, "physical S must lift BALN");

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
        app.test_update();
        let clonk_now = app.engine.test_object_snapshot(clonk);
        let balloon_now = app.engine.test_object_snapshot(balloon);
        main_assert_eq!(
            (clonk_now.action.name.as_str(), clonk_now.action.target) =>
            ("Push", Some(balloon)),
            "DFA_PUSH must retain BALN on app lift frame {lift_frame}"
        );
        main_assert!(
            (clonk_now.position.y - balloon_now.position.y - platform_delta_y).abs() <= 1,
            "CLNK must remain on BALN's platform on app lift frame {lift_frame}"
        );
    }
    main_assert!(app.engine.object_snapshot(balloon).is_some_and(|object| object.position.y <= 150), "held physical S must reach Tutorial02's flight corridor");
    {
        let mut keyboard = AppVirtualKeyboard::new(&mut app);
        main_assert!(keyboard.player_control().control_style, "the isolated fresh player must use Jump'n'Run/AutoStop control");
        keyboard.release(VirtualKeyCode::KeyS);
        main_assert_eq!(
            keyboard
                .engine()
                .object_snapshot(balloon)
                .expect("BALN after S release")
                .command_direction =>
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
        app.test_update();
        let clonk_now = app.engine.test_object_snapshot(clonk);
        let balloon_now = app.engine.test_object_snapshot(balloon);
        main_assert_eq!((clonk_now.action.name.as_str(), clonk_now.action.target) => ("Push", Some(balloon)), "DFA_PUSH must retain BALN on coast frame {coast_frame}");
        main_assert!(
            (clonk_now.position.y - balloon_now.position.y - platform_delta_y).abs() <= 1,
            "CLNK must remain on BALN's platform on coast frame {coast_frame}; \
                 initial delta={platform_delta_y}, clonk={clonk_now:?}, balloon={balloon_now:?}"
        );
    }
    main_assert!(
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
        keyboard.press(VirtualKeyCode::KeyX);
        main_assert_eq!(keyboard.engine().object_snapshot(balloon).expect("BALN after X press").command_direction => CommandDirection::Down);
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
        app.test_update();
        let clonk_now = app.engine.test_object_snapshot(clonk);
        let balloon_now = app.engine.test_object_snapshot(balloon);
        main_assert_eq!(
            (clonk_now.action.name.as_str(), clonk_now.action.target) =>
            ("Push", Some(balloon)),
            "DFA_PUSH must retain BALN on descent frame {descent_frame}"
        );
        main_assert!(
            (clonk_now.position.y - balloon_now.position.y - platform_delta_y).abs() <= 1,
            "CLNK must remain on BALN's platform on descent frame {descent_frame}"
        );
    }
    main_assert!(
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
        keyboard.release(VirtualKeyCode::KeyX);
        main_assert_eq!(keyboard.engine().object_snapshot(balloon).expect("BALN after X release").command_direction => CommandDirection::Stop);
    }

    // Release does not clear C4Player::LastCom. Eleven app updates let the
    // prior X press leave C4DoubleClick's window before the instructed X/X;
    // otherwise the first new X could become the stale press's Double.
    for _ in 0..11 {
        app.test_update();
        let clonk_now = app.engine.test_object_snapshot(clonk);
        let balloon_now = app.engine.test_object_snapshot(balloon);
        main_assert_eq!((clonk_now.action.name.as_str(), clonk_now.action.target) => ("Push", Some(balloon)));
        main_assert!((clonk_now.position.y - balloon_now.position.y - platform_delta_y).abs() <= 1);
    }
    advance_app_until(&mut app, "Tutorial02 balloon-release prompt", 30, |app| {
        app_tutorial_message_contains(app, "Let go of the balloon")
    });
    {
        let mut keyboard = AppVirtualKeyboard::new(&mut app);
        keyboard.tap(VirtualKeyCode::KeyX);
        keyboard.tap(VirtualKeyCode::KeyX);
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
        AppVirtualKeyboard::new(&mut app).press(VirtualKeyCode::KeyZ);
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
            keyboard.release(VirtualKeyCode::KeyZ);
            main_assert_eq!(keyboard.engine().object_snapshot(clonk).expect("CLNK before FLAG throw").direction => Direction::Left);
            keyboard.tap(VirtualKeyCode::KeyA);
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
                let clonk_x = app.engine.test_object_snapshot(clonk).position.x;
                let loam_x = app
                    .engine
                    .snapshot()
                    .objects
                    .into_iter()
                    .filter(|object| object.definition_id == "LOAM")
                    .min_by_key(|object| (object.position.x - clonk_x).abs())
                    .test_value()
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
                    app_clonk_carries(app, clonk, "LOAM") || app_clonk_carries(app, clonk, "FLAG")
                },
            );
            if app_clonk_carries(&app, clonk, "FLAG") {
                advance_app_until(&mut app, "Tutorial02 temporary FLAG prompt", 450, |app| {
                    app_tutorial_message_contains(app, "Please drop the flag for now")
                });
                AppVirtualKeyboard::new(&mut app).press(VirtualKeyCode::KeyC);
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
                    keyboard.release(VirtualKeyCode::KeyC);
                    keyboard.tap(VirtualKeyCode::KeyA);
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
    main_assert!(app_clonk_carries(&app, clonk, "LOAM"));
    main_assert!(app_cursor_inventory_contains(&mut app, clonk, "LOAM"), "the collected LOAM must reach the cursor inventory presentation");

    // Script40..42 moves the player to the left bridge position, observes
    // LMMS, and asks for its Diagonal left row. AutoStop Z release already
    // stops the CLNK, so no classic-only Down stop is injected here.
    advance_app_until(&mut app, "Tutorial02 move-left prompt", 240, |app| {
        app_tutorial_message_contains(app, "Now move to the very left edge")
    });
    AppVirtualKeyboard::new(&mut app).press(VirtualKeyCode::KeyZ);
    advance_app_until(&mut app, "Tutorial02 first bridge position", 120, |app| {
        app.engine.object_snapshot(clonk).is_some_and(|object| {
            object.action.name == "Walk" && (488..=490).contains(&object.position.x)
        })
    });
    AppVirtualKeyboard::new(&mut app).release(VirtualKeyCode::KeyZ);
    advance_app_until(&mut app, "Tutorial02 double-Dig prompt", 180, |app| {
        app_tutorial_message_contains(app, "Press the 'dig' key twice quickly")
    });
    {
        let mut keyboard = AppVirtualKeyboard::new(&mut app);
        keyboard.tap(VirtualKeyCode::KeyD);
        keyboard.tap(VirtualKeyCode::KeyD);
    }
    advance_app_until(&mut app, "LOAM opens LMMS", 10, |app| {
        app.engine
            .cursor_object_menu(app.players.local_owner)
            .is_some_and(|(_, menu)| menu.identification == loam_menu_identification)
    });
    advance_app_until(&mut app, "Tutorial02 Diagonal left prompt", 180, |app| {
        app_tutorial_message_contains(app, "Select the option 'diagonal left'")
    });
    app.snapshot.hud.messages.clear();
    let mut rendered = vec![0_u8; 320 * 200 * 4];
    app.test_render(&mut rendered);
    AppVirtualKeyboard::new(&mut app).tap(VirtualKeyCode::KeyZ);
    let selected = app
        .engine
        .cursor_object_menu(app.players.local_owner)
        .and_then(|(_, menu)| {
            usize::try_from(menu.selection)
                .ok()
                .map(|index| (menu, index))
        })
        .and_then(|(menu, index)| menu.items.get(index))
        .map(|item| item.caption.as_str());
    main_assert_eq!(selected => Some("Diagonal left"));
    let bridge_start = app.engine.test_object_snapshot(clonk).position;
    AppVirtualKeyboard::new(&mut app).tap(VirtualKeyCode::KeyA);
    advance_app_until(&mut app, "CLNK starts first LOAM Bridge", 10, |app| {
        app.engine
            .object_snapshot(clonk)
            .is_some_and(|object| object.action.name == "Bridge")
    });
    main_assert_eq!(
        app.engine
            .object_snapshot(clonk)
            .expect("CLNK at first LOAM Bridge start")
            .position =>
        bridge_start,
        "physical menu inputs must start Bridge without positioning the CLNK"
    );

    // C++ advances the moving UpLeft bridge first at Action.Time 6, then
    // moves sixteen (-1,-1) steps before returning to Walk
    // (C4Object.cpp:4581-4652,4755-4756).
    for _ in 0..6 {
        app.test_update();
    }
    let first_bridge_step = app.engine.test_object_snapshot(clonk);
    main_assert_eq!(first_bridge_step.action.name => "Bridge");
    main_assert_eq!(first_bridge_step.action.time => 6);
    main_assert_eq!(first_bridge_step.action.data => 0x0064_0110, "LOAM must request C++'s moving, non-wall Earth bridge");
    main_assert_eq!(first_bridge_step.position => Vector2::new(bridge_start.x - 1, bridge_start.y - 1));
    advance_app_until(&mut app, "first UpLeft bridge completes", 114, |app| {
        app.engine
            .object_snapshot(clonk)
            .is_some_and(|object| object.action.name == "Walk")
    });
    let first_bridge_end = app.engine.test_object_snapshot(clonk).position;
    main_assert_eq!((first_bridge_end.x - bridge_start.x, first_bridge_end.y - bridge_start.y,) => (-16, -16));
    advance_app_until(&mut app, "Tutorial02 three-bridge prompt", 180, |app| {
        app_tutorial_message_contains(app, "build three diagonal bridges")
    });

    // Cross back over bridge one for LOAM2, release C to stop, then return
    // with Z to its upper-left endpoint. Every fresh LMMS begins at row 7;
    // exactly one physical Z selects row 6, Diagonal left.
    AppVirtualKeyboard::new(&mut app).press(VirtualKeyCode::KeyC);
    advance_app_until(&mut app, "CLNK collects LOAM2", 220, |app| {
        app_clonk_carries(app, clonk, "LOAM")
    });
    AppVirtualKeyboard::new(&mut app).release(VirtualKeyCode::KeyC);
    AppVirtualKeyboard::new(&mut app).press(VirtualKeyCode::KeyZ);
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
    AppVirtualKeyboard::new(&mut app).release(VirtualKeyCode::KeyZ);
    {
        let mut keyboard = AppVirtualKeyboard::new(&mut app);
        keyboard.tap(VirtualKeyCode::KeyD);
        keyboard.tap(VirtualKeyCode::KeyD);
    }
    advance_app_until(&mut app, "LOAM2 opens LMMS", 20, |app| {
        app.engine
            .cursor_object_menu(app.players.local_owner)
            .is_some_and(|(_, menu)| menu.identification == loam_menu_identification)
    });
    main_assert_eq!(app.engine.cursor_object_menu(app.players.local_owner).map(|(_, menu)| menu.selection) => Some(7));
    AppVirtualKeyboard::new(&mut app).tap(VirtualKeyCode::KeyZ);
    main_assert_eq!(
        app.engine
            .cursor_object_menu(app.players.local_owner)
            .and_then(|(_, menu)| {
                usize::try_from(menu.selection)
                    .ok()
                    .and_then(|index| menu.items.get(index))
            })
            .map(|item| item.caption.as_str()) =>
        Some("Diagonal left")
    );
    let second_bridge_start = app.engine.test_object_snapshot(clonk).position;
    main_assert!(
            (second_bridge_start.x - first_bridge_end.x).abs() <= 1
                && (second_bridge_start.y - first_bridge_end.y).abs() <= 1,
            "bridge two must continue bridge one; first_end={first_bridge_end:?}, second_start={second_bridge_start:?}"
        );
    AppVirtualKeyboard::new(&mut app).tap(VirtualKeyCode::KeyA);
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
    let second_bridge_end = app.engine.test_object_snapshot(clonk).position;
    main_assert_eq!((second_bridge_end.x - second_bridge_start.x, second_bridge_end.y - second_bridge_start.y,) => (-16, -16));

    // Cross both spans for LOAM3. FLAG may be encountered first after the
    // earlier Script30 throw; face right with a physical C frame, throw it
    // using world A, finish Throw, then continue to adjacent LOAM.
    AppVirtualKeyboard::new(&mut app).press(VirtualKeyCode::KeyC);
    advance_app_until(&mut app, "CLNK reaches LOAM3 or FLAG", 260, |app| {
        app_clonk_carries(app, clonk, "LOAM") || app_clonk_carries(app, clonk, "FLAG")
    });
    AppVirtualKeyboard::new(&mut app).release(VirtualKeyCode::KeyC);
    if app_clonk_carries(&app, clonk, "FLAG") {
        AppVirtualKeyboard::new(&mut app).press(VirtualKeyCode::KeyC);
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
            keyboard.release(VirtualKeyCode::KeyC);
            main_assert_eq!(keyboard.engine().object_snapshot(clonk).expect("CLNK before rethrowing FLAG").direction => Direction::Right);
            keyboard.tap(VirtualKeyCode::KeyA);
        }
        advance_app_until(&mut app, "recollected FLAG leaves CLNK", 30, |app| {
            !app_clonk_carries(app, clonk, "FLAG")
        });
        advance_app_until(&mut app, "CLNK finishes rethrowing FLAG", 30, |app| {
            app.engine
                .object_snapshot(clonk)
                .is_some_and(|object| object.action.name == "Walk")
        });
        AppVirtualKeyboard::new(&mut app).press(VirtualKeyCode::KeyC);
        advance_app_until(&mut app, "CLNK collects LOAM3", 100, |app| {
            app_clonk_carries(app, clonk, "LOAM")
        });
        AppVirtualKeyboard::new(&mut app).release(VirtualKeyCode::KeyC);
    }
    main_assert!(app_clonk_carries(&app, clonk, "LOAM"));
    AppVirtualKeyboard::new(&mut app).press(VirtualKeyCode::KeyZ);
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
    AppVirtualKeyboard::new(&mut app).release(VirtualKeyCode::KeyZ);
    {
        let mut keyboard = AppVirtualKeyboard::new(&mut app);
        keyboard.tap(VirtualKeyCode::KeyD);
        keyboard.tap(VirtualKeyCode::KeyD);
    }
    advance_app_until(&mut app, "LOAM3 opens LMMS", 20, |app| {
        app.engine
            .cursor_object_menu(app.players.local_owner)
            .is_some_and(|(_, menu)| menu.identification == loam_menu_identification)
    });
    main_assert_eq!(app.engine.cursor_object_menu(app.players.local_owner).map(|(_, menu)| menu.selection) => Some(7));
    AppVirtualKeyboard::new(&mut app).tap(VirtualKeyCode::KeyZ);
    main_assert_eq!(
        app.engine
            .cursor_object_menu(app.players.local_owner)
            .and_then(|(_, menu)| {
                usize::try_from(menu.selection)
                    .ok()
                    .and_then(|index| menu.items.get(index))
            })
            .map(|item| item.caption.as_str()) =>
        Some("Diagonal left")
    );
    let third_bridge_start = app.engine.test_object_snapshot(clonk).position;
    main_assert!(
            (third_bridge_start.x - second_bridge_end.x).abs() <= 1
                && (third_bridge_start.y - second_bridge_end.y).abs() <= 1,
            "bridge three must continue bridge two; second_end={second_bridge_end:?}, third_start={third_bridge_start:?}"
        );
    AppVirtualKeyboard::new(&mut app).tap(VirtualKeyCode::KeyA);
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
    let third_bridge_end = app.engine.test_object_snapshot(clonk).position;
    main_assert_eq!((third_bridge_end.x - third_bridge_start.x, third_bridge_end.y - third_bridge_start.y,) => (-16, -16));
    let three_bridge_delta = (
        third_bridge_end.x - bridge_start.x,
        third_bridge_end.y - bridge_start.y,
    );
    main_assert!(
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
    AppVirtualKeyboard::new(&mut app).press(VirtualKeyCode::KeyC);
    advance_app_until(&mut app, "CLNK reaches FLAG or spare LOAM", 420, |app| {
        app_clonk_carries(app, clonk, "FLAG") || app_clonk_carries(app, clonk, "LOAM")
    });
    AppVirtualKeyboard::new(&mut app).release(VirtualKeyCode::KeyC);
    if app_clonk_carries(&app, clonk, "LOAM") {
        AppVirtualKeyboard::new(&mut app).press(VirtualKeyCode::KeyZ);
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
            keyboard.release(VirtualKeyCode::KeyZ);
            main_assert_eq!(keyboard.engine().object_snapshot(clonk).expect("CLNK before spare LOAM throw").direction => Direction::Left);
            keyboard.tap(VirtualKeyCode::KeyA);
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
        AppVirtualKeyboard::new(&mut app).press(VirtualKeyCode::KeyC);
        advance_app_until(&mut app, "CLNK collects FLAG", 180, |app| {
            app_clonk_carries(app, clonk, "FLAG")
        });
        AppVirtualKeyboard::new(&mut app).release(VirtualKeyCode::KeyC);
    }
    let flag = app
        .engine
        .object_snapshot(clonk)
        .and_then(|object| object.contents.first().copied())
        .test_value();
    main_assert_eq!(app.engine.object_snapshot(flag).expect("carried FLAG").definition_id => "FLAG");

    // Keep physical Z held over all three bridges and both jumps home. S
    // release must preserve the held Left bit on each jump.
    AppVirtualKeyboard::new(&mut app).press(VirtualKeyCode::KeyZ);
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
        keyboard.tap(VirtualKeyCode::KeyS);
        main_assert_ne!(keyboard.player_control().pressed_coms & (1 << clonk_engine::COM_LEFT) => 0, "first S release must preserve held Z");
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
        app.test_update();
        main_assert_eq!(app.engine.object_snapshot(clonk).expect("CLNK waits at center-island jump edge").action.name => "Walk");
    }
    let second_jump_start = app.engine.test_object_snapshot(clonk).position;
    {
        let mut keyboard = AppVirtualKeyboard::new(&mut app);
        keyboard.tap(VirtualKeyCode::KeyS);
        main_assert_ne!(keyboard.player_control().pressed_coms & (1 << clonk_engine::COM_LEFT) => 0, "second S release must preserve held Z");
    }
    app.test_update();
    let launched = app.engine.test_object_snapshot(clonk);
    main_assert_eq!(launched.action.name => "Jump");
    main_assert!(launched.velocity.y < 0, "second physical S must launch upward; clonk={launched:?}");
    for _ in 0..160 {
        if app
            .engine
            .object_snapshot(clonk)
            .is_some_and(|object| object.action.name == "Walk" && object.position.x <= 230)
        {
            break;
        }
        app.test_update();
    }
    let home_landing = app.engine.test_object_snapshot(clonk);
    main_assert!(
        home_landing.action.name == "Walk" && home_landing.position.x <= 230,
        "FLAG-carrying CLNK must land from {second_jump_start:?}; clonk={home_landing:?}"
    );
    let hut_position = app.engine.test_object_snapshot(hut).position;
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
    AppVirtualKeyboard::new(&mut app).release(VirtualKeyCode::KeyZ);
    main_assert_eq!(app.engine.object_snapshot(hut).expect("HUT3 before FLAG return").base => -1, "HUT3 must not be a base while FlyBase FLAG is absent");
    AppVirtualKeyboard::new(&mut app).tap(VirtualKeyCode::KeyS);
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
            .cursor_object_menu(app.players.local_owner)
            .is_some_and(|(_, menu)| {
                menu.selection == 0 && menu.items.first().is_some_and(|item| item.caption == "Put")
            })
    });
    advance_app_until(&mut app, "Tutorial02 FLAG Put prompt", 240, |app| {
        app_tutorial_message_contains(app, "Press 'throw' to put the flag")
    });
    AppVirtualKeyboard::new(&mut app).tap(VirtualKeyCode::KeyA);
    advance_app_until(&mut app, "FLAG enters HUT3", 80, |app| {
        app.engine
            .object_snapshot(flag)
            .is_some_and(|object| object.container == Some(hut))
    });
    advance_app_until(&mut app, "HUT3 restores the player base", 80, |app| {
        app.engine
            .object_snapshot(hut)
            .is_some_and(|object| object.base == app.players.local_owner)
    });
    advance_app_until(&mut app, "Tutorial02 selects Tutorial03", 180, |app| {
        app.engine.next_mission().path == r"Tutorial.c4f\Tutorial03.c4s"
    });
    advance_app_until(&mut app, "Tutorial02 reaches GameOver", 320, |app| {
        app.snapshot.game_over && app.game_over_dialog.is_some()
    });
    main_assert!(app.snapshot.round_results.fulfilled_goals.iter().any(|goal| goal == "SCRG"), "Tutorial02 must fulfill SCRG before GameOver");
    main_assert_eq!(
        app.engine.next_mission().path =>
        r"Tutorial.c4f\Tutorial03.c4s"
    );
    // Typed C4GameMessage rejection has its own regression; isolate this
    // GameOver-render assertion from that unported overlay.
    app.snapshot.hud.messages.clear();
    app.test_render(&mut rendered);
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
    main_assert!(
            !app.mouse_control,
            "Tutorial03 DisableMouse=1 must suppress player mouse control and the menu close X like C++ (C4Player.cpp:1907-1912; C4Menu.cpp:1270-1276)"
        );
    main_assert!(!app.option_flags(app.players.local_owner).mouse_shown, "DisableMouse must remove the in-game Options entry like C++ (C4MainMenu.cpp:563-571)");

    let clonk = app.engine.test_crew_cursor(app.players.local_owner);
    let hut = app
        .engine
        .snapshot()
        .objects
        .into_iter()
        .find(|object| object.definition_id == "HUT3")
        .test_value()
        .id;
    for _ in 0..360 {
        let ready =
            app.engine
                .object_snapshot(hut)
                .is_some_and(|object| object.base == app.players.local_owner)
                && app.engine.object_snapshot(clonk).is_some_and(|object| {
                    object.container.is_none() && object.action.name == "Walk"
                });
        if ready {
            break;
        }
        app.test_update();
    }
    main_assert!(
        app.engine
            .object_snapshot(hut)
            .is_some_and(|object| { object.base == app.players.local_owner }),
        "Tutorial03 ready HUT3 must become the local player's base"
    );
    main_assert!(
        app.engine
            .object_snapshot(clonk)
            .is_some_and(|object| { object.container.is_none() && object.action.name == "Walk" }),
        "Tutorial03 CLNK must exit the starting base through app frames"
    );

    {
        let mut keyboard = AppVirtualKeyboard::new(&mut app);
        keyboard.press(VirtualKeyCode::KeyC);
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
        app.test_update();
    }
    {
        let mut keyboard = AppVirtualKeyboard::new(&mut app);
        keyboard.release(VirtualKeyCode::KeyC);
        keyboard.press(VirtualKeyCode::KeyS);
        keyboard.release(VirtualKeyCode::KeyS);
    }
    for _ in 0..40 {
        if app
            .engine
            .object_snapshot(clonk)
            .is_some_and(|object| object.container == Some(hut))
        {
            break;
        }
        app.test_update();
    }
    main_assert_eq!(app.engine.object_snapshot(clonk).expect("CLNK after physical S").container => Some(hut), "physical C/S route must enter HUT3 through GameApp");

    for _ in 0..20 {
        if app.engine.cursor_object_menu(app.players.local_owner).is_some() {
            break;
        }
        app.test_update();
    }
    let (_, menu) = app.engine.cursor_object_menu(app.players.local_owner).test_value();
    let context_identification =
        serde_json::from_value(serde_json::json!({ "Int": 14 })).test_value();
    let buy_identification = serde_json::from_value(serde_json::json!({ "Int": 4 })).test_value();
    let contents_identification =
        serde_json::from_value(serde_json::json!({ "Int": 18 })).test_value();
    main_assert_eq!(menu.identification => context_identification);
    main_assert_eq!(menu.caption => "Cabin", "C4Def::Load must replace HUT3's DefCore fallback with Names.txt US localization (C4Def.cpp:635-639)");
    main_assert_eq!(menu.items.iter().map(|item| item.caption.as_str()).collect::<Vec<_>>() => vec!["Contents", "Buy", "Sell", "Info", "Exit"]);
    let mut rendered = vec![0_u8; 320 * 200 * 4];
    app.test_render(&mut rendered);
    advance_app_until(&mut app, "Tutorial03 Buy-menu prompt", 240, |app| {
        app_tutorial_message_contains(app, "Select option 'Buy'")
    });

    // Physical X is the classic down control and physical A is Throw;
    // while a menu is open C4Player::InCom translates them to MenuDown
    // and MenuEnter (C4Player.cpp:1502-1513). This is the exact Tutorial03
    // input path from Context -> Buy, without mutating menu state.
    {
        let mut keyboard = AppVirtualKeyboard::new(&mut app);
        keyboard.press(VirtualKeyCode::KeyX);
        keyboard.release(VirtualKeyCode::KeyX);
        keyboard.press(VirtualKeyCode::KeyA);
        keyboard.release(VirtualKeyCode::KeyA);
    }
    for _ in 0..20 {
        let buy_menu_open = app
            .engine
            .cursor_object_menu(app.players.local_owner)
            .is_some_and(|(_, menu)| menu.identification == buy_identification);
        if buy_menu_open {
            break;
        }
        app.test_update();
    }
    let (_, buy_menu) = app.engine.cursor_object_menu(app.players.local_owner).test_value();
    main_assert_eq!(buy_menu.identification => buy_identification);
    main_assert_eq!(
        buy_menu.title_symbol =>
        clonk_engine::ObjectMenuSymbol::Buy {
            owner: app
                .engine
                .object_snapshot(hut)
                .expect("Tutorial03 HUT3 remains active")
                .owner,
        },
        "C4MN_Buy title uses the contained building owner (C4Object.cpp:1919-1928)"
    );
    main_assert_eq!(buy_menu.extra => clonk_engine::ObjectMenuExtra::Value, "C4MN_Buy exposes selected value in its footer");
    main_assert_eq!(buy_menu.items.iter().map(|item| (item.caption.as_str(), item.count, item.value)).collect::<Vec<_>>() => vec![("Buy Lorry", 1, Some(20))]);
    main_assert_eq!(
            buy_menu.items[0].info_caption =>
            "Useful to transport large amounts of material. Holds up to 50 items.",
            "C4ObjectMenu::Refill passes each Buy definition's localized description to C4MenuItem (C4ObjectMenu.cpp:219-233)"
        );
    app.snapshot.hud.messages.clear();
    app.test_render(&mut rendered);
    advance_app_until(&mut app, "Tutorial03 buy-LORY prompt", 240, |app| {
        app_tutorial_message_contains(app, "Buy a lorry")
    });

    // Buy the selected LORY with physical A/Throw. C++ leaves the
    // permanent Buy menu open and refills its C4IDList row at count zero
    // after C4Player::Buy consumes wealth and creates the object inside
    // the base (C4Command.cpp:2005-2035; C4ObjectMenu.cpp:124-129,207-237).
    {
        let mut keyboard = AppVirtualKeyboard::new(&mut app);
        keyboard.tap(VirtualKeyCode::KeyA);
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
                .player(app.players.local_owner)
                .is_some_and(|player| player.wealth() == 5)
        {
            break;
        }
        app.test_update();
    }
    let lorry = app
        .engine
        .snapshot()
        .objects
        .into_iter()
        .find(|object| object.definition_id == "LORY")
        .test_value()
        .id;
    main_assert_eq!(app.engine.object_snapshot(lorry).expect("bought LORY").container => Some(hut));
    let player = app.engine.test_player(app.players.local_owner);
    main_assert_eq!(player.wealth() => 5);
    main_assert_eq!(player.home_base_material().get("LORY") => Some(&0));
    let (_, buy_menu) = app.engine.cursor_object_menu(app.players.local_owner).test_value();
    main_assert_eq!(buy_menu.identification => buy_identification);
    main_assert_eq!(buy_menu.items[0].count => 0);
    advance_app_until(&mut app, "Tutorial03 close-Buy prompt", 240, |app| {
        app_tutorial_message_contains(app, "close the buy menu")
    });

    // D closes Buy back to auto-context; A activates its first Contents
    // row, then A activates LORY out of HUT3. These remain ordinary
    // physical controls translated by C4Player::InCom while a menu is
    // active (C4Player.cpp:1502-1513; C4ObjectMenu.cpp:279-326).
    {
        let mut keyboard = AppVirtualKeyboard::new(&mut app);
        keyboard.tap(VirtualKeyCode::KeyD);
    }
    for _ in 0..20 {
        if app
            .engine
            .cursor_object_menu(app.players.local_owner)
            .is_some_and(|(_, menu)| menu.identification == context_identification)
        {
            break;
        }
        app.test_update();
    }
    advance_app_until(&mut app, "Tutorial03 Contents prompt", 240, |app| {
        app_tutorial_message_contains(app, "select 'Contents'")
    });
    {
        let mut keyboard = AppVirtualKeyboard::new(&mut app);
        keyboard.tap(VirtualKeyCode::KeyA);
    }
    for _ in 0..20 {
        if app
            .engine
            .cursor_object_menu(app.players.local_owner)
            .is_some_and(|(_, menu)| menu.identification == contents_identification)
        {
            break;
        }
        app.test_update();
    }
    let (_, contents_menu) = app.engine.cursor_object_menu(app.players.local_owner).test_value();
    main_assert_eq!(contents_menu.items.iter().map(|item| (item.caption.as_str(), item.item_id.as_str())).collect::<Vec<_>>() => vec![("Activate Lorry", "LORY")]);
    // Typed C4GameMessage rejection has its own regression; isolate this
    // Contents-render assertion from that unported overlay.
    app.snapshot.hud.messages.clear();
    app.test_render(&mut rendered);
    advance_app_until(&mut app, "Tutorial03 activate-LORY prompt", 240, |app| {
        app_tutorial_message_contains(app, "Activate the lorry")
    });
    {
        let mut keyboard = AppVirtualKeyboard::new(&mut app);
        keyboard.tap(VirtualKeyCode::KeyA);
    }
    for _ in 0..40 {
        if app
            .engine
            .object_snapshot(lorry)
            .is_some_and(|object| object.container.is_none())
        {
            break;
        }
        app.test_update();
    }
    main_assert!(app.engine.object_snapshot(lorry).is_some_and(|object| object.container.is_none()), "Contents activation must exit LORY from HUT3");
    advance_app_until(&mut app, "Tutorial03 leave-HUT3 prompt", 240, |app| {
        app_tutorial_message_contains(app, "exit the hut")
    });

    // Close Contents, then close the restored context menu. Its C++ close
    // command is Exit, so the tutorial-taught two physical D presses exit
    // the building without menu-selection shortcuts (C4Object.cpp:
    // 2044-2062; C4Menu.cpp:317-331; Tutorial03.c4s/Script.c:191-200).
    {
        let mut keyboard = AppVirtualKeyboard::new(&mut app);
        keyboard.tap(VirtualKeyCode::KeyD);
    }
    for _ in 0..20 {
        if app
            .engine
            .cursor_object_menu(app.players.local_owner)
            .is_some_and(|(_, menu)| menu.identification == context_identification)
        {
            break;
        }
        app.test_update();
    }
    {
        let mut keyboard = AppVirtualKeyboard::new(&mut app);
        keyboard.tap(VirtualKeyCode::KeyD);
    }
    for _ in 0..40 {
        if app
            .engine
            .object_snapshot(clonk)
            .is_some_and(|object| object.container.is_none())
        {
            break;
        }
        app.test_update();
    }
    main_assert!(app.engine.object_snapshot(clonk).is_some_and(|object| object.container.is_none()), "physical D/D route must exit CLNK from HUT3");

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
        |app| app.engine.cursor_object_menu(app.players.local_owner).is_none(),
    );
    main_assert!(app.engine.cursor_object_menu(app.players.local_owner).is_none(), "no engine cursor menu may intercept the first world X");
    main_assert!(!app.menu_controls_active_for(app.players.local_owner), "no app menu may intercept the first world X");
    let sawmill = app_object_with_definition(&app, "SAWM").test_value();
    let foundry = app_object_with_definition(&app, "FNDR").test_value();
    let tree = app
        .engine
        .snapshot()
        .objects
        .into_iter()
        .filter(|object| object.definition_id == "TRE2")
        .min_by_key(|object| (object.position.x - 167).abs())
        .test_value()
        .id;

    advance_app_until(&mut app, "Tutorial03 LORY grab prompt", 180, |app| {
        app_tutorial_message_contains(app, "once to grab the lorry")
    });
    AppVirtualKeyboard::new(&mut app).tap(VirtualKeyCode::KeyX);
    advance_app_until(&mut app, "physical X grabs LORY", 40, |app| {
        app.engine.object_snapshot(clonk).is_some_and(|object| {
            object.action.name == "Push" && object.action.target == Some(lorry)
        })
    });
    AppVirtualKeyboard::new(&mut app).press(VirtualKeyCode::KeyZ);
    advance_app_until(&mut app, "LORY reaches the sawmill chute", 240, |app| {
        app.engine.object_snapshot(lorry).is_some_and(|lorry| {
            (194..=218).contains(&lorry.position.x) && (257..=277).contains(&lorry.position.y)
        })
    });
    AppVirtualKeyboard::new(&mut app).release(VirtualKeyCode::KeyZ);
    advance_app_until(&mut app, "Tutorial03 LORY release prompt", 180, |app| {
        app_tutorial_message_contains(app, "again to let go of the lorry")
    });
    AppVirtualKeyboard::new(&mut app).tap(VirtualKeyCode::KeyX);
    advance_app_until(&mut app, "physical X releases LORY", 40, |app| {
        app.engine
            .object_snapshot(clonk)
            .is_some_and(|object| object.action.name == "Walk")
    });

    advance_app_until(&mut app, "Tutorial03 first-tree prompt", 180, |app| {
        app_tutorial_message_contains(app, "first tree on the left")
    });
    AppVirtualKeyboard::new(&mut app).press(VirtualKeyCode::KeyZ);
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
                        && (tree.position.y - 28..=tree.position.y + 28).contains(&clonk.position.y)
                })
        },
    );
    AppVirtualKeyboard::new(&mut app).release(VirtualKeyCode::KeyZ);
    advance_app_until(&mut app, "Tutorial03 double-Dig prompt", 180, |app| {
        app_tutorial_message_contains(app, "twice quickly to start chopping")
    });

    // Two immediate physical D taps synthesize COM_Dig_D and must choose
    // Chop, not Script20's intentional too-slow Dig recovery branch
    // (C4Player.cpp:1522-1536; Tutorial03.c4s/Script.c:36-63).
    {
        let mut keyboard = AppVirtualKeyboard::new(&mut app);
        keyboard.tap(VirtualKeyCode::KeyD);
        keyboard.tap(VirtualKeyCode::KeyD);
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
    AppVirtualKeyboard::new(&mut app).tap(VirtualKeyCode::KeyX);
    advance_app_until(&mut app, "physical X grabs felled TRE2", 80, |app| {
        app.engine.object_snapshot(clonk).is_some_and(|object| {
            object.action.name == "Push" && object.action.target == Some(tree)
        })
    });
    advance_app_until(&mut app, "Tutorial03 SAWM tree prompt", 180, |app| {
        app_tutorial_message_contains(app, "Push the tree over to the sawmill")
    });
    AppVirtualKeyboard::new(&mut app).press(VirtualKeyCode::KeyC);
    advance_app_until(&mut app, "TRE2 reaches the SAWM gate", 240, |app| {
        app.engine.object_snapshot(tree).is_some_and(|tree| {
            (239..=259).contains(&tree.position.x) && (254..=279).contains(&tree.position.y)
        })
    });
    AppVirtualKeyboard::new(&mut app).release(VirtualKeyCode::KeyC);
    advance_app_until(&mut app, "Tutorial03 SAWM Up prompt", 180, |app| {
        app_tutorial_message_contains(app, "press 'up' to push it into the sawmill")
    });
    AppVirtualKeyboard::new(&mut app).tap(VirtualKeyCode::KeyS);
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
    main_assert!(app.engine.object_snapshot(sawmill).is_some(), "SAWM must survive after consuming TRE2");

    advance_app_until(&mut app, "Tutorial03 creates ORE1", 180, |app| {
        app_tutorial_message_contains(app, "dig out the chunk of ore")
            && app_object_with_definition(app, "ORE1").is_some()
    });
    let ore = app_object_with_definition(&app, "ORE1").test_value();
    AppVirtualKeyboard::new(&mut app).press(VirtualKeyCode::KeyC);
    advance_app_until(&mut app, "CLNK reaches the ORE1 digging face", 600, |app| {
        app.engine
            .object_snapshot(clonk)
            .is_some_and(|object| object.position.x >= 480)
    });
    AppVirtualKeyboard::new(&mut app).release(VirtualKeyCode::KeyC);

    // A single D is buffered to COM_Dig_S after C4DoubleClick. Wait for
    // Dig before pressing X+C so another physical command cannot flush the
    // pending single early (C4Player.cpp:1215-1229,1522-1531).
    AppVirtualKeyboard::new(&mut app).tap(VirtualKeyCode::KeyD);
    advance_app_until(&mut app, "CLNK starts digging toward ORE1", 30, |app| {
        app.engine
            .object_snapshot(clonk)
            .is_some_and(|object| object.action.name == "Dig")
    });
    {
        let mut keyboard = AppVirtualKeyboard::new(&mut app);
        keyboard.press(VirtualKeyCode::KeyX);
        keyboard.press(VirtualKeyCode::KeyC);
    }
    advance_app_until(&mut app, "real dig tunnel collects ORE1", 300, |app| {
        app.engine
            .object_snapshot(ore)
            .is_some_and(|object| object.container == Some(clonk))
    });
    {
        let mut keyboard = AppVirtualKeyboard::new(&mut app);
        keyboard.release(VirtualKeyCode::KeyX);
        keyboard.release(VirtualKeyCode::KeyC);
    }
    advance_app_until(&mut app, "ORE1-carrying CLNK finishes Dig", 80, |app| {
        app.engine
            .object_snapshot(clonk)
            .is_some_and(|object| object.action.name == "Walk")
    });

    advance_app_until(&mut app, "Tutorial03 ORE1 throw prompt", 180, |app| {
        app_tutorial_message_contains(app, "Throw the chunk of ore into the lorry")
    });
    AppVirtualKeyboard::new(&mut app).press(VirtualKeyCode::KeyZ);
    advance_app_until(&mut app, "CLNK reaches LORY's right side", 800, |app| {
        app.engine
            .object_snapshot(clonk)
            .zip(app.engine.object_snapshot(lorry))
            .is_some_and(|(clonk, lorry)| {
                clonk.position.x >= lorry.position.x + 40
                    && clonk.position.x <= lorry.position.x + 42
            })
    });
    AppVirtualKeyboard::new(&mut app).release(VirtualKeyCode::KeyZ);
    main_assert!(app.engine.cursor_object_menu(app.players.local_owner).is_none(), "no engine cursor menu may intercept the world A throw");
    main_assert!(!app.menu_controls_active_for(app.players.local_owner), "no app menu may intercept the world A throw");
    AppVirtualKeyboard::new(&mut app).tap(VirtualKeyCode::KeyA);
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
    AppVirtualKeyboard::new(&mut app).press(VirtualKeyCode::KeyZ);
    advance_app_until(&mut app, "CLNK returns to LORY's grab area", 160, |app| {
        app.engine
            .object_snapshot(clonk)
            .zip(app.engine.object_snapshot(lorry))
            .is_some_and(|(clonk, lorry)| clonk.position.x <= lorry.position.x + 10)
    });
    AppVirtualKeyboard::new(&mut app).release(VirtualKeyCode::KeyZ);
    AppVirtualKeyboard::new(&mut app).tap(VirtualKeyCode::KeyX);
    advance_app_until(&mut app, "CLNK grabs loaded LORY", 60, |app| {
        app.engine.object_snapshot(clonk).is_some_and(|object| {
            object.action.name == "Push" && object.action.target == Some(lorry)
        })
    });

    // S while pushing invokes ObjectComEnter on LORY. Its real Entrance
    // callback transfers ORE1 and WOOD into FNDR before metal production
    // (C4Object.cpp:3702-3710; Lorry.c4d/Script.c:82-91).
    AppVirtualKeyboard::new(&mut app).press(VirtualKeyCode::KeyC);
    advance_app_until(&mut app, "loaded LORY reaches the FNDR gate", 400, |app| {
        app.engine.object_snapshot(lorry).is_some_and(|lorry| {
            (356..=376).contains(&lorry.position.x) && (253..=279).contains(&lorry.position.y)
        })
    });
    AppVirtualKeyboard::new(&mut app).release(VirtualKeyCode::KeyC);
    AppVirtualKeyboard::new(&mut app).tap(VirtualKeyCode::KeyS);
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
    main_assert!(app.snapshot.round_results.fulfilled_goals.iter().any(|goal| goal == "SCRG"), "Tutorial03 must fulfill SCRG before GameOver");
    main_assert_eq!(
        app.engine.next_mission().path =>
        r"Tutorial.c4f\Tutorial04.c4s"
    );
    main_assert!(
        resolve_next_mission_scenario(&app.scensel.catalog, &app.engine.next_mission().path,)
            .is_some(),
        "the focused real-scenario catalog retains Tutorial04 navigation"
    );
    // The typed C4GameMessage guard has a dedicated regression.
    app.snapshot.hud.messages.clear();
    app.test_render(&mut rendered);
}
