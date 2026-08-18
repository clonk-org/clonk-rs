// Spliced into `mod tests` (src/main_tests.rs) via include!: a bare item
// sequence, not a child module, so test ids stay `tests::<fn>`.

use clonk_frontend::startup_plrproperties::PlayerPropertiesController;
use clonk_frontend::startup_portraitsel::{
    PortraitFileEntry, PortraitLocation, PortraitSelController,
};

#[test]
fn mouse_drag_starts_only_after_cpp_five_pixel_sensitivity() {
    // DragNone uses `Abs(delta) > C4MC_DragSensitivity` with sensitivity
    // 5, so exactly five pixels remains a click and six starts dragging
    // (C4MouseControl.h:36; C4MouseControl.cpp:909-912).
    let pointer = |x: f32| ViewportPointer {
        owner: 1,
        world: FloatVector2::new(x, 20.0),
        screen: GuiPoint::new(x, 20.0),
    };
    let mut state = IngameMouseState::new(pointer(10.0), true);
    state.update(pointer(15.0));
    assert!(!state.moved, "five pixels is still below the strict > gate");
    state.update(pointer(16.0));
    assert!(state.moved, "six pixels enters C4MC_Drag_Moving");
}

#[test]
fn mouse_test_drop_geometry_fast_path_keeps_the_exact_cpp_ground_band() {
    // Drop admits air no more than five pixels above solid ground, while the
    // sixth pixel and solid ground itself do not qualify
    // (C4MouseControl.cpp:833-846).
    let landscape = Landscape::flat(8, 12);
    assert!(!mouse_test_drop_geometry_candidate(
        &landscape,
        Vector2::new(3, 6)
    ));
    assert!(mouse_test_drop_geometry_candidate(
        &landscape,
        Vector2::new(3, 7)
    ));
    assert!(mouse_test_drop_geometry_candidate(
        &landscape,
        Vector2::new(3, 11)
    ));
    assert!(!mouse_test_drop_geometry_candidate(
        &landscape,
        Vector2::new(3, 12)
    ));
}

fn move_cursor(app: &mut GameApp, point: GuiPoint, _expectation: &str) {
    app.test_cursor(PhysicalPosition::new(
        f64::from(point.x),
        f64::from(point.y),
    ));
}

fn move_cursor_to(app: &mut GameApp, position: PhysicalPosition<f64>, _expectation: &str) {
    app.test_cursor(position);
}

fn pressed_direction(slot: GamepadSlot, button: ControlButton) -> GamepadEvent {
    GamepadEvent::Direction {
        slot,
        button,
        state: ElementState::Pressed,
    }
}

fn process_primary_direction(app: &mut GameApp, button: ControlButton, _expectation: &str) {
    app.test_gamepad_events([pressed_direction(GamepadSlot::new(0), button)]);
}

fn pressed_gui_button(slot: GamepadSlot, class: GuiButtonClass) -> GamepadEvent {
    GamepadEvent::GuiButton {
        slot,
        class,
        state: ElementState::Pressed,
    }
}

fn pressed_gamepad_action(slot: GamepadSlot, action: GamepadActionType) -> GamepadEvent {
    GamepadEvent::Action {
        slot,
        action,
        state: ElementState::Pressed,
    }
}

fn pressed_gamepad_button(slot: GamepadSlot, button: LegacyGamepadButton) -> GamepadEvent {
    GamepadEvent::Button {
        slot,
        button,
        state: ElementState::Pressed,
    }
}

fn sourced_gamepad_event(gamepad: usize, cluster: u64, event: GamepadEvent) -> SourcedGamepadEvent {
    SourcedGamepadEvent {
        gamepad,
        cluster,
        event,
    }
}

fn open_player_properties<'a>(
    app: &'a mut GameApp,
    _expectation: &str,
) -> &'a mut PlayerPropertiesController {
    app.open_new_startup_player_properties();
    &mut app.startup_player_properties_dialog.test_mut().controller
}

fn open_portrait_selector<'a>(
    app: &'a mut GameApp,
    expectation: &str,
    locations: Vec<PortraitLocation>,
    entries: Vec<PortraitFileEntry>,
) -> &'a mut PlayerPropertiesController {
    let controller = open_player_properties(app, expectation);
    controller.open_portrait_selector(locations, 0, entries);
    controller
}

fn open_default_portrait_selector<'a>(
    app: &'a mut GameApp,
    expectation: &str,
) -> &'a mut PlayerPropertiesController {
    let controller = open_player_properties(app, expectation);
    open_default_portrait_selector_on(controller);
    controller
}

fn open_default_portrait_selector_on(controller: &mut PlayerPropertiesController) {
    controller.open_portrait_selector(
        vec![PortraitLocation::new("User", PathBuf::from("."))],
        0,
        Vec::new(),
    );
}

fn portrait_selector<'a>(app: &'a GameApp, _expectation: &str) -> &'a PortraitSelController {
    app.startup_player_properties_dialog
        .as_ref()
        .and_then(|pending| pending.controller.portrait_selector())
        .test_value()
}

fn physical_left_drag(app: &mut GameApp, start: GuiPoint, end: GuiPoint) {
    app.ingame_last_left_down = None;
    move_cursor(app, start, "move to left-drag start");
    app.test_left_button(ElementState::Pressed);
    move_cursor(app, end, "move physical left drag");
    app.test_left_button(ElementState::Released);
}

fn physical_right_click(app: &mut GameApp, point: GuiPoint) {
    move_cursor(app, point, "move to right-click point");
    app.test_right_button(ElementState::Pressed);
    app.test_right_button(ElementState::Released);
}

fn install_l067_context_stack(
    app: &mut GameApp,
    definitions_back_to_front: &[&str],
    selectable_windwing: bool,
) -> (Vec<ObjectId>, GuiPoint) {
    let owner = app.local_owner;
    let cursor = app
        .engine
        .crew_cursor(owner)
        .and_then(|cursor| app.engine.object_snapshot(cursor))
        .test_value();
    let position = Vector2::new(cursor.position.x - 60, cursor.position.y);
    let layer = cursor.layer;
    let mut registered = Vec::new();
    for definition_id in definitions_back_to_front {
        if registered.contains(definition_id) {
            continue;
        }
        let mut definition = test_definition(definition_id, definition_id, "#strict\n");
        definition.set_shape_rect(Some(clonk_engine::DefinitionRect::new(-5, -5, 10, 10)));
        match *definition_id {
            "WWNG" => {
                // The shipped WindWing is a rotate-only vehicle, so the
                // ordinary interaction mask misses it and right-up's
                // OCF_All fallback supplies the context target.
                definition.set_category(
                    clonk_engine::CATEGORY_VEHICLE
                        | if selectable_windwing {
                            clonk_engine::CATEGORY_MOUSE_SELECT
                        } else {
                            0
                        },
                );
                definition.set_rotateable(1);
                if selectable_windwing {
                    definition.set_grab(1);
                }
            }
            "M67C" => {
                // A closed container remains a context object without
                // OCF_Container masking the WWNG in the primary search.
                definition.set_category(clonk_engine::CATEGORY_STRUCTURE);
                definition.set_closed_container(1);
            }
            unexpected => panic!("unexpected definition {unexpected}"),
        }
        app.engine.register_test_definition(definition);
        registered.push(*definition_id);
    }

    let objects = definitions_back_to_front
        .iter()
        .map(|definition_id| {
            let spawn = layer
                .map(|layer| {
                    SpawnConfig::new(*definition_id)
                        .with_position(position)
                        .with_layer(layer)
                })
                .unwrap_or_else(|| SpawnConfig::new(*definition_id).with_position(position));
            app.engine.spawn_test_object(spawn)
        })
        .collect::<Vec<_>>();
    render_mouse_test_app(app);
    let front = *objects.last().test_value();
    let point = mouse_test_object_point(app, owner, front);
    assert_eq!(
        app.ingame_primary_mouse_target(owner, point),
        selectable_windwing.then_some(front)
    );
    assert_eq!(
        app.graphics.object_at_point(&app.snapshot, owner, point),
        Some(front)
    );
    assert_eq!(app.ingame_viewport_region(owner, point), None);
    (objects, point)
}

#[test]
fn right_click_wwng_targets_the_closed_container_behind_it() {
    let mut app = new_running_sandbox_app();
    let owner = app.local_owner;
    let (objects, point) = install_l067_context_stack(&mut app, &["M67C", "WWNG"], false);
    let [container, _windwing] = objects.as_slice() else {
        panic!("expected container and windwing, got {objects:?}");
    };
    let mut commands = install_mouse_network_capture(&mut app);

    physical_right_click(&mut app, point);

    let (direct, player_commands, selections) = commands.take_submitted_mouse_controls();
    assert!(direct.is_empty());
    assert!(selections.is_empty());
    let [(_, context)] = player_commands.as_slice() else {
        panic!("expected one Context command, got {player_commands:?}");
    };
    assert_eq!(context.player, owner);
    assert_eq!(context.command, CommandId::Context as i32);
    assert_eq!(context.target, 0);
    assert_eq!(context.target2, container.as_u64() as i32);
    assert_eq!(context.add_mode, 2);
}

#[test]
fn right_click_lone_wwng_falls_through_to_select_next() {
    let mut app = new_running_sandbox_app();
    let owner = app.local_owner;
    let (_objects, point) = install_l067_context_stack(&mut app, &["WWNG"], false);
    let expected_next = app
        .engine
        .player_mouse_select_next_object(owner)
        .test_value();
    let mut commands = install_mouse_network_capture(&mut app);

    physical_right_click(&mut app, point);

    let (direct, player_commands, selections) = commands.take_submitted_mouse_controls();
    assert!(direct.is_empty());
    assert!(
        player_commands.is_empty(),
        "a lone WWNG must not receive Context: {player_commands:?}"
    );
    let [(_, selection)] = selections.as_slice() else {
        panic!("expected one select-next packet, got {selections:?}");
    };
    assert_eq!(selection.player, owner);
    assert_eq!(selection.objects, vec![expected_next.as_u64() as i32]);
}

#[test]
fn right_click_excludes_only_the_front_wwng() {
    let mut app = new_running_sandbox_app();
    let owner = app.local_owner;
    let (objects, point) = install_l067_context_stack(&mut app, &["WWNG", "WWNG"], false);
    let [rear_windwing, _front_windwing] = objects.as_slice() else {
        panic!("expected two windwings, got {objects:?}");
    };
    let mut commands = install_mouse_network_capture(&mut app);

    physical_right_click(&mut app, point);

    let (direct, player_commands, selections) = commands.take_submitted_mouse_controls();
    assert!(direct.is_empty());
    assert!(selections.is_empty());
    let [(_, context)] = player_commands.as_slice() else {
        panic!("expected one Context command, got {player_commands:?}");
    };
    assert_eq!(context.player, owner);
    assert_eq!(context.command, CommandId::Context as i32);
    assert_eq!(context.target2, rear_windwing.as_u64() as i32);
}

#[test]
fn selectable_wwng_selects_before_falling_through_to_select_next() {
    let mut app = new_running_sandbox_app();
    let owner = app.local_owner;
    let (objects, point) = install_l067_context_stack(&mut app, &["WWNG"], true);
    let [windwing] = objects.as_slice() else {
        panic!("expected one selectable windwing, got {objects:?}");
    };
    let expected_next = app
        .engine
        .player_mouse_select_next_object(owner)
        .test_value();
    let mut commands = install_mouse_network_capture(&mut app);

    physical_right_click(&mut app, point);

    let (direct, player_commands, selections) = commands.take_submitted_mouse_controls();
    assert!(direct.is_empty());
    assert!(player_commands.is_empty());
    let [(_, selected), (_, cycled)] = selections.as_slice() else {
        panic!("expected Select followed by select-next, got {selections:?}");
    };
    assert_eq!(selected.objects, vec![windwing.as_u64() as i32]);
    assert_eq!(cycled.objects, vec![expected_next.as_u64() as i32]);
}

#[test]
fn help_click_describes_ocf_all_target_without_commands_or_drag() {
    let mut app = new_classic_running_sandbox_app();
    let owner = app.local_owner;
    let (target, point) =
        install_mouse_help_target(&mut app, "HLP1", "Named target", Some("Helpful details."));
    let (empty, _) = mouse_test_empty_point(&mut app, owner, point, None);
    let help = viewport_button_point(&app, owner, clonk_frontend::hud::ViewportButton::Help);
    let menu = viewport_button_point(&app, owner, clonk_frontend::hud::ViewportButton::PlayerMenu);
    assert_eq!(
        app.ingame_viewport_region(owner, help),
        Some(IngameViewportRegion::ViewportButton(
            clonk_frontend::hud::ViewportButton::Help,
        ))
    );
    assert_eq!(
        app.ingame_viewport_region(owner, menu),
        Some(IngameViewportRegion::ViewportButton(
            clonk_frontend::hud::ViewportButton::PlayerMenu,
        ))
    );
    let mut commands = install_mouse_network_capture(&mut app);

    physical_left_click_with_modifiers(
        &mut app,
        help,
        ModifiersState::empty(),
        ModifiersState::empty(),
    );
    assert!(
        app.ingame_mouse_help,
        "the HUD Help button enters Help mode"
    );
    assert_eq!(
        commands.take_submitted_mouse_controls(),
        (Vec::new(), Vec::new(), Vec::new()),
        "COM_Help remains process-local"
    );

    physical_left_click_with_modifiers(
        &mut app,
        menu,
        ModifiersState::empty(),
        ModifiersState::empty(),
    );
    assert!(app.ingame_mouse_help, "region clicks keep Help active");
    assert!(
        !app.ingame_menu_belongs_to(owner),
        "Help suppresses the PlayerMenu region's local side effect"
    );
    assert_eq!(
        commands.take_submitted_mouse_controls(),
        (Vec::new(), Vec::new(), Vec::new()),
        "Help suppresses synchronized region controls too"
    );

    physical_left_click_with_modifiers(
        &mut app,
        point,
        ModifiersState::empty(),
        ModifiersState::empty(),
    );
    assert!(app.ingame_mouse_help, "left-up keeps Help active");
    let expected = "Named target: Helpful details.";
    assert_eq!(
        app.ingame_mouse_help_caption,
        Some(IngameMouseHelpCaption {
            text: expected.to_string(),
            keep_moves: clonk_script::c4_string_bytes(expected).len() / 2,
        })
    );
    assert!(app.ingame_help_cursor_active());
    assert_eq!(
        commands.take_submitted_mouse_controls(),
        (Vec::new(), Vec::new(), Vec::new()),
        "a Help click never enters any synchronized mouse queue"
    );

    install_native_test_fonts(&mut app, 3.0);
    let (_, _, plan) = render_ordered_test_frame(&mut app, 3.0, 960, 600);
    let caption = plan
        .batches
        .iter()
        .flat_map(|batch| &batch.text)
        .find(|command| command.text == expected)
        .test_value();
    assert_eq!(
        caption.role,
        clonk_graphics::clonk_font::ClonkFontRole::GuiTooltip,
        "Help captions use the global tooltip font"
    );

    app.ingame_last_left_down = None;
    move_cursor(&mut app, point, "move back onto help target");
    app.test_left_button(ElementState::Pressed);
    move_cursor(&mut app, empty, "move beyond drag sensitivity in Help");
    let state = app.mouse_state.test_value();
    assert!(state.down_cursor_help);
    assert!(state.motion.moved);
    assert!(!state.motion.world_drag_started);
    assert!(!state.motion.region_drag_started);
    assert!(!state.motion.selection_frame);
    app.test_left_button(ElementState::Released);
    assert_eq!(
        commands.take_submitted_mouse_controls(),
        (Vec::new(), Vec::new(), Vec::new()),
        "crossing the drag threshold in Help still emits nothing"
    );
    assert_eq!(
        app.ingame_mouse_help_caption
            .as_ref()
            .map(|caption| caption.text.as_str()),
        Some(expected),
        "Help reports the object captured on left-down"
    );
    assert_eq!(
        app.engine.object_help_caption(target).as_deref(),
        Some(expected)
    );
}

#[test]
fn help_caption_uses_name_only_and_cpp_move_lifetime() {
    let mut app = new_running_sandbox_app();
    let raw_name = clonk_script::c4_string_from_bytes(b"Ren\xe9X");
    let (_target, point) = install_mouse_help_target(&mut app, "HLP2", &raw_name, None);
    app.ingame_mouse_help = true;
    physical_left_click_with_modifiers(
        &mut app,
        point,
        ModifiersState::empty(),
        ModifiersState::empty(),
    );

    let keep = clonk_script::c4_string_bytes(&raw_name).len() / 2;
    assert_ne!(keep, raw_name.len() / 2, "KeepCaption counts C4 bytes");
    assert_eq!(
        app.ingame_mouse_help_caption,
        Some(IngameMouseHelpCaption {
            text: raw_name,
            keep_moves: keep,
        })
    );
    for remaining in (0..keep).rev() {
        app.update_ingame_pointer(point).test_value();
        assert_eq!(
            app.ingame_mouse_help_caption
                .as_ref()
                .map(|caption| caption.keep_moves),
            Some(remaining),
            "caption survives the move that decrements KeepCaption to zero"
        );
    }
    app.update_ingame_pointer(point).test_value();
    assert!(app.ingame_mouse_help_caption.is_none());

    app.ingame_mouse_help_caption = Some(IngameMouseHelpCaption {
        text: "wheel".to_string(),
        keep_moves: 2,
    });
    app.test_mouse_wheel(MouseScrollDelta::LineDelta(0.0, 1.0), 1.0);
    assert_eq!(
        app.ingame_mouse_help_caption
            .as_ref()
            .map(|caption| caption.keep_moves),
        Some(1)
    );

    app.ingame_mouse_help_caption = Some(IngameMouseHelpCaption {
        text: "ignored up".to_string(),
        keep_moves: 2,
    });
    app.ingame_ignore_left_up = true;
    app.handle_ingame_mouse_button(ElementState::Released)
        .test_value();
    assert!(!app.ingame_ignore_left_up);
    assert_eq!(
        app.ingame_mouse_help_caption
            .as_ref()
            .map(|caption| caption.keep_moves),
        Some(1)
    );

    app.ingame_mouse_help_caption = Some(IngameMouseHelpCaption {
        text: "middle".to_string(),
        keep_moves: 2,
    });
    app.handle_other_mouse_button(ElementState::Pressed)
        .test_value();
    assert_eq!(
        app.ingame_mouse_help_caption
            .as_ref()
            .map(|caption| caption.keep_moves),
        Some(1)
    );
}

#[test]
fn shift_left_clicks_append_and_sample_release_modifiers() {
    let mut app = new_running_sandbox_app();
    let owner = app.local_owner;
    let cursor = app.engine.test_crew_cursor(owner);
    render_mouse_test_app(&mut app);
    let viewport = app.graphics.viewport_rect(owner).test_value();
    let mut clicks = Vec::new();
    'rows: for y in viewport.y..viewport.y + viewport.height as i32 {
        for x in viewport.x..viewport.x + viewport.width as i32 {
            let screen = GuiPoint::new(x as f32 + 0.5, y as f32 + 0.5);
            let routed = GuiPoint::new(screen.x.ceil(), screen.y.ceil());
            let Some(pointer) = app.graphics.viewport_point_at(routed) else {
                continue;
            };
            let world = ingame_pointer_world_pixel(pointer);
            if pointer.owner != owner
                || app.ingame_viewport_region(owner, routed).is_some()
                || app.ingame_primary_mouse_target(owner, routed).is_some()
                || app.ingame_pointer_fog_blocked(pointer)
                || app.engine.mouse_jump_zone(owner, world)
                || clicks
                    .iter()
                    .any(|(_, prior_world): &(GuiPoint, Vector2)| *prior_world == world)
            {
                continue;
            }
            clicks.push((screen, world));
            if clicks.len() == 3 {
                break 'rows;
            }
        }
    }
    let [(first_screen, first_world), (second_screen, second_world), (third_screen, third_world)] =
        clicks.as_slice()
    else {
        panic!("sandbox viewport needs three distinct MoveTo points: {clicks:?}");
    };

    // LeftUp is the triggering C4MouseControl::Move event. Shift pressed
    // after LeftDown must therefore append the first command.
    physical_left_click_with_modifiers(
        &mut app,
        *first_screen,
        ModifiersState::empty(),
        ModifiersState::SHIFT,
    );
    physical_left_click_with_modifiers(
        &mut app,
        *second_screen,
        ModifiersState::SHIFT,
        ModifiersState::SHIFT,
    );
    let appended = app
        .engine
        .test_object_snapshot(cursor)
        .command_stack
        .command_views();
    assert_eq!(
        appended
            .iter()
            .map(|command| command.name.as_str())
            .collect::<Vec<_>>(),
        ["MoveTo", "MoveTo"]
    );
    assert_eq!(
        appended
            .iter()
            .map(|command| (command.tx, command.ty))
            .collect::<Vec<_>>(),
        [
            (Some(first_world.x), Some(first_world.y)),
            (Some(second_world.x), Some(second_world.y)),
        ]
    );

    // Releasing Shift before LeftUp restores Set even though LeftDown saw
    // Shift, proving the modifier is sampled at the command event.
    physical_left_click_with_modifiers(
        &mut app,
        *third_screen,
        ModifiersState::SHIFT,
        ModifiersState::empty(),
    );
    let replaced = app
        .engine
        .test_object_snapshot(cursor)
        .command_stack
        .command_views();
    assert_eq!(replaced.len(), 1);
    assert_eq!(replaced[0].name, "MoveTo");
    assert_eq!(
        (replaced[0].tx, replaced[0].ty),
        (Some(third_world.x), Some(third_world.y))
    );
}

#[test]
fn shift_double_get_samples_the_second_press_event() {
    let mut app = new_running_sandbox_app();
    let owner = app.local_owner;
    let crew = app.engine.test_crew_cursor(owner);
    let crew_state = app.engine.test_object_snapshot(crew);
    let mut item = test_definition("M43G", "Shift Get target", "#strict\n");
    item.set_category(clonk_engine::CATEGORY_OBJECT);
    item.set_collectible(true);
    item.set_shape_rect(Some(clonk_engine::DefinitionRect::new(-4, -4, 8, 8)));
    app.engine.register_test_definition(item);
    let mut spawn = SpawnConfig::new("M43G").with_position(Vector2::new(
        crew_state.position.x - 60,
        crew_state.position.y,
    ));
    if let Some(layer) = crew_state.layer {
        spawn = spawn.with_layer(layer);
    }
    let target = app.engine.spawn_test_object(spawn);
    render_mouse_test_app(&mut app);
    let target_point = mouse_test_object_point(&app, owner, target);
    assert_eq!(
        app.ingame_primary_mouse_target(owner, target_point),
        Some(target)
    );
    let mut commands = install_mouse_network_capture(&mut app);

    app.ingame_last_left_down = None;
    app.test_modifiers(ModifiersState::empty());
    move_cursor(&mut app, target_point, "move over carryable target");
    app.test_left_button(ElementState::Pressed);
    app.test_left_button(ElementState::Released);

    app.test_modifiers(ModifiersState::SHIFT);
    app.test_left_button(ElementState::Pressed);
    app.test_modifiers(ModifiersState::empty());
    app.test_left_button(ElementState::Released);

    let (direct, player_commands, selections) = commands.take_submitted_mouse_controls();
    assert!(direct.is_empty());
    assert!(selections.is_empty());
    let [(_, first), (_, second)] = player_commands.as_slice() else {
        panic!("expected MoveTo then Get, got {player_commands:?}");
    };
    assert_eq!(
        (first.command, first.add_mode),
        (CommandId::MoveTo as i32, 1)
    );
    assert_eq!(
        (second.command, second.target, second.add_mode),
        (CommandId::Get as i32, target.as_u64() as i32, 1 | 4)
    );
}

fn configure_mouse_fog(
    app: &mut GameApp,
    range: i32,
) -> (i32, ObjectId, Vector2, Option<ObjectId>) {
    let owner = app.local_owner;
    let cursor = app.engine.test_crew_cursor(owner);
    // The fixture's own repeller is added below through the ordinary
    // SetPlrViewRange path, so the enabling edge's rebuild is a no-op here.
    let _ = app.engine.test_player_mut(owner).set_fog_of_war(true);
    let mut update = ObjectUpdate::new();
    update.plr_view_range = Some(range);
    app.engine.apply_object_update(cursor, update).test_value();
    let cursor = app.engine.test_object_snapshot(cursor);
    (owner, cursor.id, cursor.position, cursor.layer)
}

fn spawn_mouse_fog_target(
    app: &mut GameApp,
    id: &str,
    position: Vector2,
    layer: Option<ObjectId>,
    category: i32,
) -> ObjectId {
    let mut definition = test_definition(id, id, "#strict\n");
    definition.set_category(category);
    definition.set_collectible(true);
    definition.set_shape_rect(Some(clonk_engine::DefinitionRect::new(-3, -3, 6, 6)));
    app.engine.register_test_definition(definition);
    let spawn = layer
        .map(|layer| {
            SpawnConfig::new(id)
                .with_position(position)
                .with_layer(layer)
        })
        .unwrap_or_else(|| SpawnConfig::new(id).with_position(position));
    app.engine.spawn_test_object(spawn)
}

#[test]
fn mouse_fog_blocks_hidden_left_click_and_cycles_hidden_right_target() {
    let mut app = new_running_sandbox_app();
    let (owner, cursor, cursor_position, layer) = configure_mouse_fog(&mut app, 40);
    let target = spawn_mouse_fog_target(
        &mut app,
        "MFGH",
        Vector2::new(cursor_position.x + 100, cursor_position.y),
        layer,
        clonk_engine::CATEGORY_OBJECT | clonk_engine::CATEGORY_MOUSE_SELECT,
    );
    render_mouse_test_app(&mut app);

    let hidden_empty = [-100, -120, 120]
        .into_iter()
        .find_map(|offset| {
            let world = Vector2::new(cursor_position.x + offset, cursor_position.y);
            let (x, y) = app.graphics.world_to_screen(owner, world)?;
            let point = GuiPoint::new(x.ceil(), y.ceil());
            let pointer = app.graphics.viewport_point_at(point)?;
            (pointer.owner == owner
                && app.ingame_pointer_fog_blocked(pointer)
                && app
                    .graphics
                    .object_at_point(&app.snapshot, owner, point)
                    .is_none()
                && app.ingame_viewport_region(owner, point).is_none())
            .then_some(point)
        })
        .test_value();
    let target_point = mouse_test_object_point(&app, owner, target);
    let target_pointer = app
        .graphics
        .viewport_point_at(GuiPoint::new(target_point.x.ceil(), target_point.y.ceil()))
        .test_value();
    assert!(app.ingame_pointer_fog_blocked(target_pointer));
    assert_eq!(app.ingame_primary_mouse_target(owner, target_point), None);
    let expected_next = app
        .engine
        .player_mouse_select_next_object(owner)
        .test_value();
    assert_eq!(expected_next, cursor);
    let mut commands = install_mouse_network_capture(&mut app);

    app.ingame_last_left_down = None;
    move_cursor(&mut app, hidden_empty, "move to hidden empty point");
    app.test_left_button(ElementState::Pressed);
    app.test_left_button(ElementState::Released);

    move_cursor(&mut app, target_point, "move onto hidden target");
    app.test_right_button(ElementState::Pressed);
    app.test_right_button(ElementState::Released);

    let (direct, player_commands, selections) = commands.take_submitted_mouse_controls();
    assert!(direct.is_empty());
    assert!(
        player_commands.is_empty(),
        "fog must suppress both MoveTo and hidden Context: {player_commands:?}"
    );
    let [(_, selection)] = selections.as_slice() else {
        panic!("hidden right click must cycle exactly once, got {selections:?}");
    };
    assert_eq!(selection.player, owner);
    assert_eq!(selection.objects, vec![expected_next.as_u64() as i32]);
}

#[test]
fn mouse_fog_keeps_ignore_fow_target_clickable() {
    let mut app = new_running_sandbox_app();
    let (owner, _cursor, cursor_position, layer) = configure_mouse_fog(&mut app, 40);
    let target = spawn_mouse_fog_target(
        &mut app,
        "MFGI",
        Vector2::new(cursor_position.x + 100, cursor_position.y),
        layer,
        clonk_engine::CATEGORY_OBJECT | clonk_engine::CATEGORY_MOUSE_SELECT | C4D_IGNORE_FOW,
    );
    render_mouse_test_app(&mut app);
    let target_point = mouse_test_object_point(&app, owner, target);
    let target_pointer = app
        .graphics
        .viewport_point_at(GuiPoint::new(target_point.x.ceil(), target_point.y.ceil()))
        .test_value();
    assert!(app.ingame_pointer_fog_blocked(target_pointer));
    assert_eq!(
        app.ingame_mouse_select_target(owner, target_point),
        Some(target)
    );
    let mut commands = install_mouse_network_capture(&mut app);

    app.ingame_last_left_down = None;
    move_cursor(&mut app, target_point, "move onto IgnoreFoW target");
    app.test_left_button(ElementState::Pressed);
    app.test_left_button(ElementState::Released);

    let (direct, player_commands, selections) = commands.take_submitted_mouse_controls();
    assert!(direct.is_empty());
    assert!(player_commands.is_empty());
    let [(_, selection)] = selections.as_slice() else {
        panic!("IgnoreFoW target must remain clickable, got {selections:?}");
    };
    assert_eq!(selection.objects, vec![target.as_u64() as i32]);
}

#[test]
fn fog_keeps_ignore_fow_target_and_jump_captions() {
    let mut app = new_running_sandbox_app();
    let (owner, _cursor, cursor_position, layer) = configure_mouse_fog(&mut app, 0);
    let target = spawn_mouse_fog_target(
        &mut app,
        "M54F",
        Vector2::new(cursor_position.x + 100, cursor_position.y),
        layer,
        clonk_engine::CATEGORY_OBJECT | clonk_engine::CATEGORY_MOUSE_SELECT | C4D_IGNORE_FOW,
    );
    render_mouse_test_app(&mut app);

    let target_point = mouse_test_object_point(&app, owner, target);
    let target_point = GuiPoint::new(target_point.x.ceil(), target_point.y.ceil());
    let target_pointer = app.graphics.viewport_point_at(target_point).test_value();
    assert!(app.ingame_pointer_fog_blocked(target_pointer));
    assert_eq!(
        app.ingame_primary_mouse_target(owner, target_point),
        Some(target)
    );
    let target_cursor = app.engine.mouse_world_cursor(
        owner,
        Some(target),
        ingame_pointer_world_pixel(target_pointer),
        false,
    );
    assert_eq!(target_cursor, MouseWorldCursor::Select(target));

    let move_stably = |app: &mut GameApp, point: GuiPoint| {
        for _ in 0..=INGAME_MOUSE_CAPTION_DELAY {
            move_cursor(app, point, "route stable fog-covered hover move");
        }
    };
    let expected = app
        .ingame_world_cursor_caption(target_cursor, ingame_pointer_world_pixel(target_pointer))
        .test_value();
    move_stably(&mut app, target_point);
    assert_eq!(
        app.ingame_mouse_caption
            .caption
            .as_ref()
            .map(|caption| &caption.text),
        Some(&expected),
        "C4D_IgnoreFoW keeps its world cursor caption in fog"
    );

    let jump = Vector2::new(cursor_position.x + 8, cursor_position.y - 15);
    let (jump_x, jump_y) = app.graphics.world_to_screen(owner, jump).test_value();
    let jump_point = GuiPoint::new(jump_x.ceil(), jump_y.ceil());
    let jump_pointer = app.graphics.viewport_point_at(jump_point).test_value();
    let jump = ingame_pointer_world_pixel(jump_pointer);
    assert!(app.ingame_pointer_fog_blocked(jump_pointer));
    let jump_cursor = app.engine.mouse_world_cursor(
        owner,
        app.ingame_primary_mouse_target(owner, jump_point),
        jump,
        false,
    );
    assert_eq!(jump_cursor, MouseWorldCursor::JumpRight);

    let expected = app
        .ingame_world_cursor_caption(jump_cursor, jump)
        .test_value();
    move_stably(&mut app, jump_point);
    assert_eq!(
        app.ingame_mouse_caption
            .caption
            .as_ref()
            .map(|caption| &caption.text),
        Some(&expected),
        "jump captions remain available when the endpoint is fog-covered"
    );
}

#[test]
fn mouse_fog_turns_moving_object_release_into_noop_command() {
    let mut app = new_running_sandbox_app();
    let (owner, _cursor, cursor_position, layer) = configure_mouse_fog(&mut app, 60);
    let target = spawn_mouse_fog_target(
        &mut app,
        "MFGD",
        Vector2::new(cursor_position.x + 24, cursor_position.y),
        layer,
        clonk_engine::CATEGORY_OBJECT,
    );
    render_mouse_test_app(&mut app);
    let target_point = mouse_test_object_point(&app, owner, target);
    let start = app.graphics.viewport_point_at(target_point).test_value();
    assert!(!app.ingame_pointer_fog_blocked(start));
    assert_eq!(
        app.engine
            .mouse_world_drag_source(owner, target, ingame_pointer_world_pixel(start)),
        Some(MouseDragSource::Carryable)
    );

    let landscape = app.snapshot.landscape.test_ref();
    let width = i32::try_from(landscape.width()).test_value();
    let height = landscape.estimated_height();
    let viewport = app.graphics.viewport_rect(owner).test_value();
    let visible_drag_point = (viewport.y..viewport.y + viewport.height as i32)
        .flat_map(|y| {
            (viewport.x..viewport.x + viewport.width as i32)
                .map(move |x| GuiPoint::new(x as f32, y as f32))
        })
        .find(|point| {
            let Some(pointer) = app.graphics.viewport_point_at(*point) else {
                return false;
            };
            pointer.owner == owner
                && ((point.x - target_point.x).abs() >= 12.0
                    || (point.y - target_point.y).abs() >= 12.0)
                && !app.ingame_pointer_fog_blocked(pointer)
                && app
                    .graphics
                    .object_at_point(&app.snapshot, owner, *point)
                    .is_none()
                && app.ingame_viewport_region(owner, *point).is_none()
        })
        .test_value();
    let (hidden_point, hidden) = (viewport.y..viewport.y + viewport.height as i32)
        .flat_map(|y| {
            (viewport.x..viewport.x + viewport.width as i32)
                .map(move |x| GuiPoint::new(x as f32, y as f32))
        })
        .find_map(|point| {
            let pointer = app.graphics.viewport_point_at(point)?;
            let world = ingame_pointer_world_pixel(pointer);
            (pointer.owner == owner
                && world.x >= 0
                && world.y >= 0
                && world.x < width
                && world.y < height
                && ((point.x - target_point.x).abs() >= 12.0
                    || (point.y - target_point.y).abs() >= 12.0)
                && app.ingame_pointer_fog_blocked(pointer)
                && app
                    .graphics
                    .object_at_point(&app.snapshot, owner, point)
                    .is_none()
                && app.ingame_viewport_region(owner, point).is_none())
            .then_some((point, world))
        })
        .test_value();
    let mut commands = install_mouse_network_capture(&mut app);

    app.ingame_last_left_down = None;
    move_cursor(&mut app, target_point, "move to visible drag source");
    app.test_left_button(ElementState::Pressed);
    move_cursor(
        &mut app,
        visible_drag_point,
        "begin DragMoving in visible terrain",
    );
    assert!(app.mouse_state.is_some_and(|state| state.motion.moved));
    move_cursor(&mut app, hidden_point, "continue DragMoving into fog");
    app.test_left_button(ElementState::Released);

    let (direct, player_commands, selections) = commands.take_submitted_mouse_controls();
    assert!(direct.is_empty());
    assert!(selections.is_empty());
    let [(_, command)] = player_commands.as_slice() else {
        panic!("fog-covered object release must send one no-op, got {player_commands:?}");
    };
    assert_eq!(command.player, owner);
    assert_eq!(command.command, 0);
    assert_eq!((command.x, command.y), (hidden.x, hidden.y));
    assert_eq!((command.target, command.target2), (0, 0));
    assert_eq!(command.add_mode, 1);
}

#[test]
fn mouse_fog_freezes_selection_members_at_last_visible_endpoint() {
    let mut app = new_running_sandbox_app();
    let (owner, _cursor, cursor_position, layer) = configure_mouse_fog(&mut app, 60);
    let first = spawn_mouse_fog_target(
        &mut app,
        "MFG1",
        Vector2::new(cursor_position.x + 24, cursor_position.y),
        layer,
        clonk_engine::CATEGORY_OBJECT,
    );
    let second = spawn_mouse_fog_target(
        &mut app,
        "MFG2",
        Vector2::new(cursor_position.x + 90, cursor_position.y),
        layer,
        clonk_engine::CATEGORY_OBJECT,
    );
    render_mouse_test_app(&mut app);
    let to_screen = |app: &GameApp, world: Vector2| {
        let (x, y) = app.graphics.world_to_screen(owner, world).test_value();
        GuiPoint::new(x.ceil(), y.ceil())
    };
    let start = to_screen(
        &app,
        Vector2::new(cursor_position.x + 10, cursor_position.y - 10),
    );
    let visible_end = to_screen(
        &app,
        Vector2::new(cursor_position.x + 35, cursor_position.y + 10),
    );
    let hidden_end = to_screen(
        &app,
        Vector2::new(cursor_position.x + 110, cursor_position.y + 10),
    );
    for point in [start, visible_end, hidden_end] {
        assert!(app
            .graphics
            .object_at_point(&app.snapshot, owner, point)
            .is_none());
        assert!(app.ingame_viewport_region(owner, point).is_none());
    }
    assert!(app
        .graphics
        .viewport_point_at(visible_end)
        .is_some_and(|pointer| !app.ingame_pointer_fog_blocked(pointer)));
    assert!(app
        .graphics
        .viewport_point_at(hidden_end)
        .is_some_and(|pointer| app.ingame_pointer_fog_blocked(pointer)));

    move_cursor(&mut app, start, "move to selection start");
    app.test_right_button(ElementState::Pressed);
    move_cursor(&mut app, visible_end, "move to visible selection endpoint");
    let visible_motion = app.ingame_right_mouse_state.test_value().motion;
    assert_eq!(
        visible_motion.selection_kind,
        IngameDragSelectionKind::Objects
    );
    assert_eq!(app.ingame_selection_candidates(visible_motion), vec![first]);

    move_cursor(&mut app, hidden_end, "move selection endpoint into fog");
    let hidden_motion = app.ingame_right_mouse_state.test_value().motion;
    assert_eq!(hidden_motion.last.screen, hidden_end);
    assert_eq!(app.ingame_selection_candidates(hidden_motion), vec![first]);
    assert_ne!(
        app.ingame_selection_candidates(hidden_motion),
        vec![first, second],
        "fogged endpoint must not add the hidden member"
    );

    app.engine
        .apply_object_update(
            first,
            ObjectUpdate::new()
                .with_position(Vector2::new(cursor_position.x + 90, cursor_position.y)),
        )
        .test_value();
    app.engine
        .apply_object_update(
            second,
            ObjectUpdate::new()
                .with_position(Vector2::new(cursor_position.x + 24, cursor_position.y)),
        )
        .test_value();
    app.snapshot = app.engine.snapshot();
    assert_eq!(
        app.engine.mouse_drag_carryables_in_rect(
            ingame_pointer_world_pixel(hidden_motion.start),
            ingame_pointer_world_pixel(hidden_motion.selection_last),
        ),
        vec![second],
        "a live rectangle query now disagrees with native's cached Selection"
    );
    assert_eq!(
        app.ingame_selection_candidates(hidden_motion),
        vec![first],
        "fog must freeze object identity, not only rectangle coordinates"
    );

    app.test_right_button(ElementState::Released);
    assert_eq!(app.ingame_dragged_objects, vec![first]);
}

#[test]
fn mouse_fog_origin_drag_into_visible_terrain_uses_release_cursor() {
    let mut app = new_running_sandbox_app();
    let (owner, _cursor, _cursor_position, _layer) = configure_mouse_fog(&mut app, 40);
    render_mouse_test_app(&mut app);
    let viewport = app.graphics.viewport_rect(owner).test_value();
    let points = || {
        (viewport.y..viewport.y + viewport.height as i32).flat_map(|y| {
            (viewport.x..viewport.x + viewport.width as i32)
                .map(move |x| GuiPoint::new(x as f32, y as f32))
        })
    };
    let visible = points()
        .find(|point| {
            let Some(pointer) = app.graphics.viewport_point_at(*point) else {
                return false;
            };
            pointer.owner == owner
                && !app.ingame_pointer_fog_blocked(pointer)
                && app
                    .graphics
                    .object_at_point(&app.snapshot, owner, *point)
                    .is_none()
                && app.ingame_viewport_region(owner, *point).is_none()
                && !app
                    .engine
                    .mouse_jump_zone(owner, ingame_pointer_world_pixel(pointer))
        })
        .test_value();
    let hidden = points()
        .find(|point| {
            let Some(pointer) = app.graphics.viewport_point_at(*point) else {
                return false;
            };
            pointer.owner == owner
                && ((point.x - visible.x).abs() >= 12.0 || (point.y - visible.y).abs() >= 12.0)
                && app.ingame_pointer_fog_blocked(pointer)
                && app
                    .graphics
                    .object_at_point(&app.snapshot, owner, *point)
                    .is_none()
                && app.ingame_viewport_region(owner, *point).is_none()
        })
        .test_value();
    let mut commands = install_mouse_network_capture(&mut app);

    app.ingame_last_left_down = None;
    move_cursor(&mut app, hidden, "move to fog-covered press point");
    app.test_left_button(ElementState::Pressed);
    assert!(app
        .mouse_state
        .is_some_and(|state| state.down_cursor_nothing));
    move_cursor(&mut app, visible, "move held pointer into visible terrain");
    let release = app.ingame_pointer.test_value();
    assert!(app.mouse_state.is_some_and(|state| {
        !state.motion.moved && state.motion.last.screen == release.screen
    }));
    app.test_left_button(ElementState::Released);

    let (direct, player_commands, selections) = commands.take_submitted_mouse_controls();
    assert!(direct.is_empty());
    assert!(selections.is_empty());
    let [(_, command)] = player_commands.as_slice() else {
        panic!("visible release must queue one MoveTo, got {player_commands:?}");
    };
    let release_world = ingame_pointer_world_pixel(release);
    assert_eq!(command.command, CommandId::MoveTo as i32);
    assert_eq!((command.x, command.y), (release_world.x, release_world.y));
    assert_eq!((command.target, command.target2), (0, 0));
}

#[test]
fn mouse_fog_blocks_all_four_landscape_boundaries_even_when_disabled() {
    let mut app = new_state_only_running_sandbox_app();
    let owner = app.local_owner;
    // Disabling never owes the repeller rebuild.
    let _ = app.engine.test_player_mut(owner).set_fog_of_war(false);
    app.snapshot = app.engine.snapshot();
    let landscape = app.snapshot.landscape.test_ref();
    let width = i32::try_from(landscape.width()).test_value();
    let height = landscape.estimated_height();
    let valid_x = width / 2;
    let valid_y = height / 2;
    let pointers = [
        Vector2::new(-1, valid_y),
        Vector2::new(valid_x, -1),
        Vector2::new(width, valid_y),
        Vector2::new(valid_x, height),
    ]
    .map(|world| ViewportPointer {
        owner,
        world: FloatVector2::new(world.x as f32, world.y as f32),
        screen: GuiPoint::new(-100.0, -100.0),
    });
    assert!(pointers
        .iter()
        .all(|pointer| app.ingame_pointer_fog_blocked(*pointer)));
    let mut commands = install_mouse_network_capture(&mut app);

    for pointer in pointers {
        app.handle_ingame_mouse_click(pointer).test_value();
    }

    let (direct, player_commands, selections) = commands.take_submitted_mouse_controls();
    assert!(direct.is_empty());
    assert!(player_commands.is_empty());
    assert!(selections.is_empty());
}

#[test]
fn physical_left_drag_carryable_queues_object_drop_without_direct_controls() {
    // Both physical buttons enter C4MC_Drag_Moving for an Object cursor.
    // Left-up must send the same C4CMD_Drop packet as the already faithful
    // right path; it must not synthesize direction and COM_Throw controls
    // for the cursor's unrelated inventory (C4MouseControl.cpp:909-932,
    // 833-891,1171-1201).
    let mut app = new_running_sandbox_app();
    let owner = app.local_owner;
    let crew = app.engine.test_crew_cursor(owner);
    let mut landscape = Landscape::flat(480, 180);
    landscape.set_world_height(200);
    app.engine.set_landscape(landscape);
    let crew_position = app.engine.test_object_snapshot(crew).position;
    let mut item = test_definition("MLCI", "Mouse left item", "#strict\n");
    item.set_category(clonk_engine::CATEGORY_OBJECT);
    item.set_collectible(true);
    item.set_shape_rect(Some(clonk_engine::DefinitionRect::new(-4, -4, 8, 8)));
    app.engine.register_test_definition(item);
    let layer = app.engine.test_object_snapshot(crew).layer;
    let target_spawn = layer
        .map(|layer| {
            SpawnConfig::new("MLCI")
                .with_position(Vector2::new(crew_position.x - 60, crew_position.y))
                .with_layer(layer)
        })
        .unwrap_or_else(|| {
            SpawnConfig::new("MLCI")
                .with_position(Vector2::new(crew_position.x - 60, crew_position.y))
        });
    let target = app.engine.spawn_test_object(target_spawn);
    app.engine
        .spawn_test_object(SpawnConfig::new("MLCI").with_container(crew));
    render_mouse_test_app(&mut app);
    let target_point = mouse_test_object_point(&app, owner, target);
    let (drop_point, drop_world) =
        mouse_test_empty_point(&mut app, owner, target_point, Some(CommandId::Drop));
    let mut commands = install_mouse_network_capture(&mut app);

    physical_left_drag(&mut app, target_point, drop_point);

    let (direct, player_commands, selections) = commands.take_submitted_mouse_controls();
    assert!(
        direct.is_empty(),
        "a left object drag must not emit direction/COM_Throw controls: {direct:?}"
    );
    assert!(selections.is_empty());
    let [(_, command)] = player_commands.as_slice() else {
        panic!("expected one C4CMD_Drop packet, got {player_commands:?}");
    };
    assert_eq!(command.player, owner);
    assert_eq!(command.command, CommandId::Drop as i32);
    assert_eq!(command.x, drop_world.x);
    assert_eq!(command.y, drop_world.y);
    assert_eq!(command.target, target.as_u64() as i32);
    assert_eq!(command.target2, 0);
    assert_eq!(command.data, 0);
    assert_eq!(command.add_mode, 1);
    assert_eq!(command.by_client, 0);
    assert!(app.ingame_dragged_objects.is_empty());
}

#[test]
fn physical_left_drag_vehicle_queues_push_to_and_control_target() {
    // Grab==1 enters the same moving drag for either button. Control over
    // an OCF_Container selects VehiclePut, represented by PushTo with the
    // container in Target2 (C4MouseControl.cpp:934-961,882-890,
    // 1171-1201).
    let mut app = new_running_sandbox_app();
    let owner = app.local_owner;
    let crew = app.engine.test_crew_cursor(owner);
    let crew_position = app.engine.test_object_snapshot(crew).position;
    let layer = app.engine.test_object_snapshot(crew).layer;

    let mut vehicle = test_definition("MLVH", "Mouse left vehicle", "#strict\n");
    vehicle.set_category(clonk_engine::CATEGORY_VEHICLE);
    vehicle.set_grab(1);
    vehicle.set_shape_rect(Some(clonk_engine::DefinitionRect::new(-5, -5, 10, 10)));
    app.engine.register_test_definition(vehicle);
    let mut container = test_definition("MLCT", "Mouse left container", "#strict\n");
    container.set_category(clonk_engine::CATEGORY_STRUCTURE);
    container.set_grab_put_get(clonk_engine::GRAB_PUT_GET_PUT);
    container.set_shape_rect(Some(clonk_engine::DefinitionRect::new(-6, -6, 12, 12)));
    app.engine.register_test_definition(container);
    let spawn_at = |definition: &str, position: Vector2| {
        layer
            .map(|layer| {
                SpawnConfig::new(definition)
                    .with_position(position)
                    .with_layer(layer)
            })
            .unwrap_or_else(|| SpawnConfig::new(definition).with_position(position))
    };
    let vehicle = app.engine.spawn_test_object(spawn_at(
        "MLVH",
        Vector2::new(crew_position.x - 60, crew_position.y),
    ));
    let container = app.engine.spawn_test_object(spawn_at(
        "MLCT",
        Vector2::new(crew_position.x + 60, crew_position.y),
    ));
    render_mouse_test_app(&mut app);
    let vehicle_point = mouse_test_object_point(&app, owner, vehicle);
    let container_point = mouse_test_object_point(&app, owner, container);
    let start_world = app
        .graphics
        .viewport_point_at(vehicle_point)
        .map(ingame_pointer_world_pixel)
        .test_value();
    assert_eq!(
        app.engine
            .mouse_world_drag_source(owner, vehicle, start_world),
        Some(MouseDragSource::Vehicle)
    );
    assert_eq!(
        app.graphics.object_at_point_with_ocf(
            &app.snapshot,
            owner,
            container_point,
            clonk_engine::ocf::CONTAINER,
        ),
        Some(container)
    );
    let (open_point, open_world) = mouse_test_empty_point(&mut app, owner, vehicle_point, None);
    let mut commands = install_mouse_network_capture(&mut app);

    physical_left_drag(&mut app, vehicle_point, open_point);
    let (direct, player_commands, selections) = commands.take_submitted_mouse_controls();
    assert!(direct.is_empty());
    assert!(selections.is_empty());
    let [(_, open)] = player_commands.as_slice() else {
        panic!("expected one open-ground PushTo, got {player_commands:?}");
    };
    assert_eq!(open.command, CommandId::PushTo as i32);
    assert_eq!(open.target, vehicle.as_u64() as i32);
    assert_eq!(open.target2, 0);
    assert_eq!((open.x, open.y), (open_world.x, open_world.y));
    assert_eq!(open.add_mode, 1);

    app.test_modifiers(ModifiersState::CONTROL);
    physical_left_drag(&mut app, vehicle_point, container_point);
    app.test_modifiers(ModifiersState::empty());
    let (direct, player_commands, selections) = commands.take_submitted_mouse_controls();
    assert!(direct.is_empty());
    assert!(selections.is_empty());
    let [(_, put)] = player_commands.as_slice() else {
        panic!("expected one Control-PushTo, got {player_commands:?}");
    };
    let put_world = app
        .graphics
        .viewport_point_at(GuiPoint::new(
            container_point.x.ceil(),
            container_point.y.ceil(),
        ))
        .map(ingame_pointer_world_pixel)
        .test_value();
    assert_eq!(put.command, CommandId::PushTo as i32);
    assert_eq!(put.target, vehicle.as_u64() as i32);
    assert_eq!(put.target2, container.as_u64() as i32);
    assert_eq!((put.x, put.y), (put_world.x, put_world.y));
    assert_eq!(put.add_mode, 1);
}

#[test]
fn physical_left_object_frame_retains_group_for_set_then_append_drag() {
    // ButtonUpDragSelecting deliberately retains an Objects selection.
    // Dragging either member immediately afterwards moves the complete
    // group in C++ main-list order, using Set then Append
    // (C4MouseControl.cpp:626-645,795-817,1158-1201).
    let mut app = new_running_sandbox_app();
    let owner = app.local_owner;
    let crew = app.engine.test_crew_cursor(owner);
    let mut landscape = Landscape::flat(480, 180);
    landscape.set_world_height(200);
    app.engine.set_landscape(landscape);
    let crew_snapshot = app.engine.test_object_snapshot(crew);
    let mut item = test_definition("MLGR", "Mouse left group item", "#strict\n");
    item.set_category(clonk_engine::CATEGORY_OBJECT);
    item.set_collectible(true);
    item.set_shape_rect(Some(clonk_engine::DefinitionRect::new(-3, -3, 6, 6)));
    app.engine.register_test_definition(item);
    let spawn_at = |position: Vector2| {
        crew_snapshot
            .layer
            .map(|layer| {
                SpawnConfig::new("MLGR")
                    .with_position(position)
                    .with_layer(layer)
            })
            .unwrap_or_else(|| SpawnConfig::new("MLGR").with_position(position))
    };
    let first_position = Vector2::new(crew_snapshot.position.x - 70, crew_snapshot.position.y - 10);
    let second_position = Vector2::new(first_position.x + 24, first_position.y);
    let first = app.engine.spawn_test_object(spawn_at(first_position));
    let second = app.engine.spawn_test_object(spawn_at(second_position));
    render_mouse_test_app(&mut app);
    let first_point = mouse_test_object_point(&app, owner, first);
    let second_point = mouse_test_object_point(&app, owner, second);
    let frame_start = GuiPoint::new(
        first_point.x.min(second_point.x) - 8.0,
        first_point.y.min(second_point.y) - 8.0,
    );
    let frame_end = GuiPoint::new(
        first_point.x.max(second_point.x) + 8.0,
        first_point.y.max(second_point.y) + 8.0,
    );
    for point in [frame_start, frame_end] {
        assert!(app
            .graphics
            .viewport_point_at(point)
            .is_some_and(|pointer| pointer.owner == owner));
        assert_eq!(
            app.graphics.object_at_point(&app.snapshot, owner, point),
            None,
            "selection frame endpoints remain on landscape"
        );
    }
    let first_world = app
        .graphics
        .viewport_point_at(frame_start)
        .map(ingame_pointer_world_pixel)
        .test_value();
    let second_world = app
        .graphics
        .viewport_point_at(frame_end)
        .map(ingame_pointer_world_pixel)
        .test_value();
    let expected_selection = app
        .engine
        .mouse_drag_carryables_in_rect(first_world, second_world);
    assert_eq!(expected_selection.len(), 2);
    let mut commands = install_mouse_network_capture(&mut app);

    app.ingame_last_left_down = None;
    move_cursor(&mut app, frame_start, "move to object-frame start");
    app.test_left_button(ElementState::Pressed);
    move_cursor(&mut app, frame_end, "drag frame over both carryables");
    assert_eq!(
        app.mouse_state
            .expect("left object frame remains live")
            .motion
            .selection_kind,
        IngameDragSelectionKind::Objects
    );
    app.test_left_button(ElementState::Released);
    assert_eq!(app.ingame_dragged_objects, expected_selection);
    assert!(
        app.ingame_last_left_down.is_none(),
        "a moved gesture cannot arm an immediate false LeftDouble"
    );
    let (direct, player_commands, selections) = commands.take_submitted_mouse_controls();
    assert!(direct.is_empty());
    assert!(player_commands.is_empty());
    assert!(selections.is_empty());

    let member_point = mouse_test_object_point(&app, owner, first);
    let (drop_point, drop_world) =
        mouse_test_empty_point(&mut app, owner, member_point, Some(CommandId::Drop));
    move_cursor(&mut app, member_point, "move over selected group member");
    app.test_left_button(ElementState::Pressed);
    move_cursor(&mut app, drop_point, "drag selected group to Drop cursor");
    app.test_left_button(ElementState::Released);

    let (direct, player_commands, selections) = commands.take_submitted_mouse_controls();
    assert!(direct.is_empty());
    assert!(selections.is_empty());
    assert_eq!(player_commands.len(), 2);
    assert_eq!(
        player_commands
            .iter()
            .map(|(_, command)| command.target)
            .collect::<Vec<_>>(),
        expected_selection
            .iter()
            .map(|object| object.as_u64() as i32)
            .collect::<Vec<_>>()
    );
    assert_eq!(
        player_commands
            .iter()
            .map(|(_, command)| command.add_mode)
            .collect::<Vec<_>>(),
        vec![1, 4]
    );
    assert!(player_commands.iter().all(|(_, command)| {
        command.command == CommandId::Drop as i32
            && command.x == drop_world.x
            && command.y == drop_world.y
    }));
    assert!(app.ingame_dragged_objects.is_empty());
}

#[test]
fn physical_left_empty_and_entrance_drags_emit_no_commands() {
    // An empty selecting frame that never locks to Crew/Objects emits no
    // control. Likewise, Entrance overrides an otherwise Carryable
    // down cursor, so moving that target never enters C4MC_Drag_Moving
    // (C4MouseControl.cpp:893-980,1158-1201).
    let mut app = new_running_sandbox_app();
    let owner = app.local_owner;
    let crew = app.engine.test_crew_cursor(owner);
    let crew_snapshot = app.engine.test_object_snapshot(crew);
    let mut hybrid = test_definition("MLEN", "Mouse left entrance", "#strict\n");
    hybrid.set_category(clonk_engine::CATEGORY_STRUCTURE);
    hybrid.set_collectible(true);
    hybrid.set_shape_rect(Some(clonk_engine::DefinitionRect::new(-6, -6, 12, 12)));
    hybrid.set_entrance_rect(Some(clonk_engine::DefinitionRect::new(-6, -6, 12, 12)));
    app.engine.register_test_definition(hybrid);
    let entrance_spawn = crew_snapshot
        .layer
        .map(|layer| {
            SpawnConfig::new("MLEN")
                .with_position(Vector2::new(
                    crew_snapshot.position.x - 60,
                    crew_snapshot.position.y,
                ))
                .with_layer(layer)
        })
        .unwrap_or_else(|| {
            SpawnConfig::new("MLEN").with_position(Vector2::new(
                crew_snapshot.position.x - 60,
                crew_snapshot.position.y,
            ))
        });
    let entrance = app.engine.spawn_test_object(entrance_spawn);
    app.engine
        .spawn_test_object(SpawnConfig::new("MLEN").with_container(crew));
    render_mouse_test_app(&mut app);
    let viewport = app.graphics.viewport_rect(owner).test_value();
    let (empty_start, empty_end) = (viewport.y..viewport.y + viewport.height as i32)
        .flat_map(|y| {
            (viewport.x..viewport.x + viewport.width as i32 - 8).map(move |x| {
                (
                    GuiPoint::new(x as f32 + 0.5, y as f32 + 0.5),
                    GuiPoint::new(x as f32 + 8.5, y as f32 + 0.5),
                )
            })
        })
        .find(|(first, second)| {
            let Some(first_pointer) = app.graphics.viewport_point_at(*first) else {
                return false;
            };
            let Some(second_pointer) = app.graphics.viewport_point_at(*second) else {
                return false;
            };
            if first_pointer.owner != owner
                || second_pointer.owner != owner
                || app.ingame_viewport_region(owner, *first).is_some()
                || app.ingame_viewport_region(owner, *second).is_some()
                || app
                    .graphics
                    .object_at_point(&app.snapshot, owner, *first)
                    .is_some()
                || app
                    .graphics
                    .object_at_point(&app.snapshot, owner, *second)
                    .is_some()
            {
                return false;
            }
            let first_world = ingame_pointer_world_pixel(first_pointer);
            let second_world = ingame_pointer_world_pixel(second_pointer);
            app.engine
                .mouse_drag_crew_in_rect(owner, first_world, second_world)
                .is_empty()
                && app
                    .engine
                    .mouse_drag_carryables_in_rect(first_world, second_world)
                    .is_empty()
        })
        .test_value();
    let mut commands = install_mouse_network_capture(&mut app);

    app.ingame_last_left_down = None;
    move_cursor(&mut app, empty_start, "move to empty frame start");
    app.test_left_button(ElementState::Pressed);
    move_cursor(&mut app, empty_end, "move empty frame");
    assert_eq!(
        app.mouse_state
            .expect("empty frame remains live")
            .motion
            .selection_kind,
        IngameDragSelectionKind::Unknown
    );
    app.test_left_button(ElementState::Released);
    let (direct, player_commands, selections) = commands.take_submitted_mouse_controls();
    assert!(direct.is_empty());
    assert!(player_commands.is_empty());
    assert!(selections.is_empty());
    assert!(app.ingame_dragged_objects.is_empty());

    let entrance_point = mouse_test_object_point(&app, owner, entrance);
    let entrance_world = app
        .graphics
        .viewport_point_at(entrance_point)
        .map(ingame_pointer_world_pixel)
        .test_value();
    assert_eq!(
        app.ingame_primary_mouse_target(owner, entrance_point),
        Some(entrance)
    );
    assert_eq!(
        app.engine
            .mouse_world_drag_source(owner, entrance, entrance_world),
        None,
        "Entrance overrides the otherwise Carryable cursor"
    );
    let (release_point, _) = mouse_test_empty_point(&mut app, owner, entrance_point, None);
    physical_left_drag(&mut app, entrance_point, release_point);
    let (direct, player_commands, selections) = commands.take_submitted_mouse_controls();
    assert!(direct.is_empty());
    assert!(player_commands.is_empty());
    assert!(selections.is_empty());
    assert!(app.ingame_dragged_objects.is_empty());
}

#[test]
fn physical_right_drag_vehicle_queues_cpp_push_to() {
    // A Grab=1 world target enters C4MC_Drag_Moving after the six-pixel
    // threshold and ButtonUpDragMoving sends PushTo with the vehicle as
    // Target at the release coordinates (C4MouseControl.cpp:934-941,
    // 882-890,1171-1227).
    let mut app = new_running_sandbox_app();
    let owner = app.local_owner;
    let crew = app.engine.test_crew_cursor(owner);
    let mut frame = vec![0_u8; 320 * 200 * 4];
    app.test_render(&mut frame);
    let crew_position = app.engine.test_object_snapshot(crew).position;
    let vehicle_position = Vector2::new(crew_position.x - 60, crew_position.y);

    let mut vehicle = test_definition("MVEH", "Mouse vehicle", "#strict\n");
    vehicle.set_category(clonk_engine::CATEGORY_VEHICLE);
    vehicle.set_grab(1);
    vehicle.set_shape_rect(Some(clonk_engine::DefinitionRect::new(-4, -4, 8, 8)));
    app.engine.register_test_definition(vehicle);
    let mut vehicle_spawn = SpawnConfig::new("MVEH").with_position(vehicle_position);
    if let Some(layer) = app.engine.test_object_snapshot(crew).layer {
        vehicle_spawn = vehicle_spawn.with_layer(layer);
    }
    let vehicle = app.engine.spawn_test_object(vehicle_spawn);
    app.snapshot = app.engine.snapshot();
    assert_ne!(
        app.engine
            .object_snapshot(vehicle)
            .expect("vehicle remains live")
            .ocf
            & clonk_engine::ocf::GRAB,
        0,
        "Grab=1 vehicle exposes OCF_Grab"
    );
    app.test_render(&mut frame);
    let vehicle_snapshot = app.snapshot.object(vehicle).cloned().test_value();
    let (vehicle_x, vehicle_y) = app
        .graphics
        .world_to_screen(owner, vehicle_snapshot.position)
        .test_value();
    let direct_pick = app.graphics.object_at_point_with_ocf(
        &app.snapshot,
        owner,
        GuiPoint::new(vehicle_x, vehicle_y),
        clonk_engine::ocf::GRAB,
    );
    assert_eq!(
            direct_pick,
            Some(vehicle),
            "vehicle center pick; object={vehicle_snapshot:?}; cursor={:?}; center=({vehicle_x},{vehicle_y})",
            app.snapshot
                .players
                .iter()
                .find(|player| player.id == owner)
                .and_then(|player| player.cursor)
                .and_then(|cursor| app.snapshot.object(cursor))
        );
    let vehicle_point = GuiPoint::new(vehicle_x, vehicle_y);
    let release_point = [30.0, -30.0]
        .into_iter()
        .map(|dx| GuiPoint::new(vehicle_point.x + dx, vehicle_point.y))
        .find(|point| {
            app.graphics
                .viewport_point_at(*point)
                .is_some_and(|pointer| pointer.owner == owner)
        })
        .test_value();
    let release_world = app
        .graphics
        .viewport_point_at(release_point)
        .map(ingame_pointer_world_pixel)
        .test_value();

    move_cursor(&mut app, vehicle_point, "move over vehicle");
    app.test_right_button(ElementState::Pressed);
    move_cursor(&mut app, release_point, "drag vehicle");
    app.test_right_button(ElementState::Released);

    let commands = app
        .engine
        .test_object_snapshot(crew)
        .command_stack
        .command_views();
    assert_eq!(commands.len(), 1, "Set replaces the previous command stack");
    assert_eq!(commands[0].name, "PushTo");
    assert_eq!(commands[0].target, Some(vehicle));
    assert_eq!(commands[0].target2, None);
    assert_eq!(commands[0].tx, Some(release_world.x));
    assert_eq!(commands[0].ty, Some(release_world.y));
}

#[test]
fn mouse_hover_caption_waits_ten_stable_moves_and_clears_on_miss() {
    let mut app = new_running_sandbox_app();
    let owner = app.local_owner;
    let crew = app.engine.test_crew_cursor(owner);
    let crew = app.engine.test_object_snapshot(crew);
    let mut vehicle = test_definition("MHOV", "Hover wagon", "#strict\n");
    vehicle.set_category(clonk_engine::CATEGORY_VEHICLE);
    vehicle.set_grab(1);
    vehicle.set_shape_rect(Some(clonk_engine::DefinitionRect::new(-5, -5, 10, 10)));
    app.engine.register_test_definition(vehicle);
    let mut spawn =
        SpawnConfig::new("MHOV").with_position(Vector2::new(crew.position.x - 60, crew.position.y));
    if let Some(layer) = crew.layer {
        spawn = spawn.with_layer(layer);
    }
    let target = app.engine.spawn_test_object(spawn);
    render_mouse_test_app(&mut app);
    let target_point = mouse_test_object_point(&app, owner, target);
    let pointer = app.graphics.viewport_point_at(target_point).test_value();
    let world = ingame_pointer_world_pixel(pointer);
    assert_eq!(
        app.engine
            .mouse_world_cursor(owner, Some(target), world, false),
        MouseWorldCursor::Grab(target)
    );

    let move_to = |app: &mut GameApp, point: GuiPoint| {
        move_cursor(app, point, "route physical hover move");
    };
    move_to(&mut app, target_point);
    assert_eq!(app.ingame_mouse_caption.cursor, IngameMouseCursorKind::Grab);
    assert_eq!(app.ingame_mouse_caption.time_on_target, 0);
    assert!(app.ingame_mouse_caption.caption.is_none());
    for _ in 1..INGAME_MOUSE_CAPTION_DELAY {
        move_to(&mut app, target_point);
    }
    assert_eq!(
        app.ingame_mouse_caption.time_on_target,
        INGAME_MOUSE_CAPTION_DELAY - 1
    );
    assert!(app.ingame_mouse_caption.caption.is_none());

    move_to(&mut app, target_point);
    let expected = app
        .ingame_world_cursor_caption(MouseWorldCursor::Grab(target), world)
        .test_value();
    let caption = app.ingame_mouse_caption.caption.test_ref();
    assert_eq!(caption.text, expected);
    assert!(caption.text.contains('|'));

    let (miss, _) = mouse_test_empty_point(&mut app, owner, target_point, None);
    move_to(&mut app, miss);
    assert!(
        app.ingame_mouse_caption.caption.is_none(),
        "the next unmatched native Move clears a non-kept caption"
    );
}

#[test]
fn inventory_hover_caption_is_immediate_and_anchored_to_region_top() {
    let (mut app, owner, _crew, _first, target, region_point) = inventory_region_fixture();
    let viewport = app.graphics.viewport_rect(owner).test_value();
    let (_, region) = app
        .ingame_inventory_region_hit(owner, region_point)
        .test_value();

    move_cursor(&mut app, region_point, "move over inventory region");
    let caption = app.ingame_mouse_caption.caption.test_ref();
    assert_eq!(
        caption.text,
        app.ingame_object_caption_name(target).unwrap()
    );
    assert_eq!(caption.caption_bottom_y, Some(region.y - viewport.y));
    assert_eq!(
        app.ingame_mouse_caption.cursor,
        IngameMouseCursorKind::Region
    );

    let (miss, _) = mouse_test_empty_point(&mut app, owner, region_point, None);
    move_cursor(&mut app, miss, "move away from inventory region");
    assert!(app.ingame_mouse_caption.caption.is_none());
}

#[test]
fn ctrl_region_drags_show_put_and_vehicle_put_captions() {
    for vehicle_drag in [false, true] {
        let mut app = new_running_sandbox_app();
        let owner = app.local_owner;
        let crew = app.engine.test_crew_cursor(owner);
        let crew_state = app.engine.test_object_snapshot(crew);
        let mut dragged = test_definition(
            "M54D",
            if vehicle_drag {
                "Caption wagon"
            } else {
                "Caption item"
            },
            "#strict\n",
        );
        if vehicle_drag {
            dragged.set_category(clonk_engine::CATEGORY_VEHICLE);
            dragged.set_grab(1);
        } else {
            dragged.set_category(clonk_engine::CATEGORY_OBJECT);
            dragged.set_collectible(true);
        }
        app.engine.register_test_definition(dragged);
        let dragged = app
            .engine
            .spawn_test_object(SpawnConfig::new("M54D").with_container(crew));

        let mut container = test_definition("M54C", "Caption container", "#strict\n");
        container.set_category(clonk_engine::CATEGORY_STRUCTURE);
        container.set_grab_put_get(clonk_engine::GRAB_PUT_GET_PUT);
        container.set_shape_rect(Some(clonk_engine::DefinitionRect::new(-6, -6, 12, 12)));
        app.engine.register_test_definition(container);
        let mut container_spawn = SpawnConfig::new("M54C").with_position(Vector2::new(
            crew_state.position.x + 60,
            crew_state.position.y,
        ));
        if let Some(layer) = crew_state.layer {
            container_spawn = container_spawn.with_layer(layer);
        }
        let container = app.engine.spawn_test_object(container_spawn);
        while app.engine.frame() % 5 != 4 {
            app.test_update();
        }
        render_mouse_test_app(&mut app);

        let viewport = app.graphics.viewport_rect(owner).test_value();
        let inventory_point = GuiPoint::new(
            (viewport.x + clonk_frontend::hud::SYMBOL_BORDER + clonk_frontend::hud::SYMBOL_SIZE / 2)
                as f32,
            (viewport.y + viewport.height as i32
                - clonk_frontend::hud::SYMBOL_BORDER
                - clonk_frontend::hud::SYMBOL_SIZE / 2) as f32,
        );
        assert_eq!(
            app.ingame_inventory_region_target(owner, inventory_point),
            Some(dragged)
        );
        let container_point = mouse_test_object_point(&app, owner, container);
        assert_eq!(
            app.graphics.object_at_point_with_ocf(
                &app.snapshot,
                owner,
                container_point,
                clonk_engine::ocf::CONTAINER,
            ),
            Some(container)
        );

        let (key, template, expected_kind, expected_cursor) = if vehicle_drag {
            (
                "IDS_CON_VEHICLEPUT",
                "VEHICLE <%s> INTO <%s>",
                IngameMouseCursorKind::VehiclePut,
                IngameRegionDragCursor::VehiclePut(container),
            )
        } else {
            (
                "IDS_CON_PUT",
                "ITEM <%s> INTO <%s>",
                IngameMouseCursorKind::Put,
                IngameRegionDragCursor::Put(container),
            )
        };
        app.startup_tooltip_resources
            .insert(key.to_string(), template.to_string());
        move_cursor(
            &mut app,
            inventory_point,
            "move onto dragged inventory object",
        );
        app.handle_ingame_mouse_button(ElementState::Pressed)
            .test_value();
        app.test_modifiers(ModifiersState::CONTROL);
        move_cursor(
            &mut app,
            container_point,
            "cross moving-drag threshold over container",
        );
        assert!(
            app.ingame_mouse_caption.caption.is_none(),
            "DragNone starts the moving drag but does not run DragMoving yet"
        );
        app.test_update();

        assert_eq!(app.ingame_mouse_caption.cursor, expected_kind);
        assert_eq!(
            app.mouse_state
                .and_then(|state| state.motion.region_drag_cursor),
            Some(expected_cursor)
        );
        let caption = app.ingame_mouse_caption.caption.test_ref();
        let expected_subject = if vehicle_drag {
            "Caption wagon"
        } else {
            "Caption item"
        };
        assert_eq!(
            caption.text,
            template
                .replacen("%s", expected_subject, 1)
                .replacen("%s", "Caption container", 1)
        );
        assert_eq!(caption.caption_bottom_y, None);
    }
}

#[test]
fn group_put_caption_uses_remaining_live_selection() {
    let (mut app, owner, crew, _first, _second, region_point) = inventory_region_fixture();
    let newest = app
        .engine
        .spawn_test_object(SpawnConfig::new("MITM").with_container(crew));
    let crew_state = app.engine.test_object_snapshot(crew);
    let mut container = test_definition("M54G", "Group container", "#strict\n");
    container.set_category(clonk_engine::CATEGORY_STRUCTURE);
    container.set_grab_put_get(clonk_engine::GRAB_PUT_GET_PUT);
    container.set_shape_rect(Some(clonk_engine::DefinitionRect::new(-6, -6, 12, 12)));
    app.engine.register_test_definition(container);
    let mut spawn = SpawnConfig::new("M54G").with_position(Vector2::new(
        crew_state.position.x + 60,
        crew_state.position.y,
    ));
    if let Some(layer) = crew_state.layer {
        spawn = spawn.with_layer(layer);
    }
    let target = app.engine.spawn_test_object(spawn);
    render_mouse_test_app(&mut app);
    assert_eq!(
        app.ingame_inventory_region_target(owner, region_point),
        Some(newest)
    );
    let target_point = mouse_test_object_point(&app, owner, target);
    app.startup_tooltip_resources
        .insert("IDS_CON_ITEMS".to_owned(), "widgets".to_owned());
    app.startup_tooltip_resources
        .insert("IDS_CON_PUT".to_owned(), "PUT <%s> INTO <%s>".to_owned());

    move_cursor(&mut app, region_point, "hover grouped inventory region");
    app.test_right_button(ElementState::Pressed);
    app.test_modifiers(ModifiersState::CONTROL);
    move_cursor(
        &mut app,
        target_point,
        "cross grouped moving-drag threshold",
    );
    assert_eq!(app.ingame_dragged_objects.len(), 3);

    app.engine
        .apply_object_update(
            newest,
            ObjectUpdate::new().with_status(clonk_engine::ObjectStatus::Deleted),
        )
        .test_value();
    app.snapshot = app.engine.snapshot();
    move_cursor(&mut app, target_point, "refresh DragMoving after deletion");

    assert_eq!(app.ingame_mouse_caption.cursor, IngameMouseCursorKind::Put);
    assert_eq!(
        app.ingame_mouse_caption
            .caption
            .as_ref()
            .expect("remaining grouped selection has a caption")
            .text,
        "PUT <2 widgets> INTO <Group container>"
    );
}

#[test]
fn world_origin_put_caption_uses_dragged_object_name() {
    let mut app = new_running_sandbox_app();
    let owner = app.local_owner;
    let crew = app.engine.test_crew_cursor(owner);
    let crew_state = app.engine.test_object_snapshot(crew);
    let mut item = test_definition("M54W", "World parcel", "#strict\n");
    item.set_category(clonk_engine::CATEGORY_OBJECT);
    item.set_collectible(true);
    item.set_shape_rect(Some(clonk_engine::DefinitionRect::new(-5, -5, 10, 10)));
    app.engine.register_test_definition(item);
    let mut item_spawn = SpawnConfig::new("M54W").with_position(Vector2::new(
        crew_state.position.x - 60,
        crew_state.position.y,
    ));
    if let Some(layer) = crew_state.layer {
        item_spawn = item_spawn.with_layer(layer);
    }
    let item = app.engine.spawn_test_object(item_spawn);

    let mut container = test_definition("M54T", "World bin", "#strict\n");
    container.set_category(clonk_engine::CATEGORY_STRUCTURE);
    container.set_grab_put_get(clonk_engine::GRAB_PUT_GET_PUT);
    container.set_shape_rect(Some(clonk_engine::DefinitionRect::new(-6, -6, 12, 12)));
    app.engine.register_test_definition(container);
    let mut target_spawn = SpawnConfig::new("M54T").with_position(Vector2::new(
        crew_state.position.x + 60,
        crew_state.position.y,
    ));
    if let Some(layer) = crew_state.layer {
        target_spawn = target_spawn.with_layer(layer);
    }
    let target = app.engine.spawn_test_object(target_spawn);
    render_mouse_test_app(&mut app);
    let item_point = mouse_test_object_point(&app, owner, item);
    let target_point = mouse_test_object_point(&app, owner, target);
    app.startup_tooltip_resources
        .insert("IDS_CON_PUT".to_owned(), "PUT <%s> INTO <%s>".to_owned());

    move_cursor(&mut app, item_point, "hover world parcel");
    app.handle_ingame_mouse_button(ElementState::Pressed)
        .test_value();
    app.test_modifiers(ModifiersState::CONTROL);
    move_cursor(&mut app, target_point, "cross world moving-drag threshold");
    assert!(
        app.ingame_mouse_caption.caption.is_none(),
        "the threshold move only enters DragMoving"
    );
    move_cursor(&mut app, target_point, "run DragMoving over world bin");

    assert_eq!(app.ingame_mouse_caption.cursor, IngameMouseCursorKind::Put);
    assert_eq!(
        app.ingame_mouse_caption
            .caption
            .as_ref()
            .expect("world-origin put has a caption")
            .text,
        "PUT <World parcel> INTO <World bin>"
    );
}

#[test]
fn inventory_region_drag_latches_entry_and_selection_at_threshold() {
    let (mut app, owner, crew, _first, target, region_point) = inventory_region_fixture();
    let crew_position = app.engine.test_object_snapshot(crew).position;
    let landscape = app.engine.landscape().test_value();
    let drop_x = crew_position.x + 30;
    let ground_y = (0..landscape.estimated_height())
        .find(|y| landscape.is_solid_at(drop_x, *y))
        .test_value();
    let drop_world = Vector2::new(drop_x, ground_y - 1);
    let (drop_x, drop_y) = app.graphics.world_to_screen(owner, drop_world).test_value();
    let drop_point = GuiPoint::new(drop_x, drop_y);
    assert_eq!(
        app.engine.mouse_drag_carryable_command(owner, drop_world),
        Some(CommandId::Drop)
    );
    let (manager, _events, mut network_commands) =
        NetworkManager::test_stub_with_commands_for_client_id(7);
    app.network = Some(manager);
    let tick = app.local_control_submission_tick();

    move_cursor(&mut app, region_point, "move onto inventory region");
    app.handle_ingame_mouse_button(ElementState::Pressed)
        .test_value();
    move_cursor(&mut app, drop_point, "cross drag threshold");
    assert!(app.mouse_state.is_some_and(|state| {
        state.motion.region_drag_started && state.motion.region_drag_cursor.is_none()
    }));
    app.handle_ingame_mouse_button(ElementState::Released)
        .test_value();
    let (controls, commands, selections) = network_commands.take_submitted_player_inputs();
    assert!(controls.is_empty());
    assert_eq!(
        commands,
        vec![(
            tick,
            PlayerCommandControlData {
                player: owner,
                command: 0,
                x: drop_world.x,
                y: drop_world.y,
                target: 0,
                target2: 0,
                data: 0,
                add_mode: 1,
                by_client: 7,
            },
        )],
        "the threshold event itself still runs DragNone, not DragMoving"
    );
    assert!(selections.is_empty());
    app.network = None;
    app.ingame_last_left_down = None;

    move_cursor(&mut app, region_point, "move onto inventory region");
    app.handle_ingame_mouse_button(ElementState::Pressed)
        .test_value();
    move_cursor(&mut app, drop_point, "cross drag threshold");
    assert!(app
        .mouse_state
        .is_some_and(|state| state.motion.region_drag_started));
    assert_eq!(app.ingame_dragged_objects, vec![target]);
    for _ in 0..5 {
        app.test_update();
    }
    assert_eq!(
        app.mouse_state
            .and_then(|state| state.motion.region_drag_cursor),
        Some(IngameRegionDragCursor::Drop),
        "the Tick5 C4MouseControl::Execute equivalent refreshes a stationary drag"
    );

    let mut inert = test_definition("MNOC", "No longer carryable", "#strict\n");
    inert.set_category(clonk_engine::CATEGORY_OBJECT);
    app.engine.register_test_definition(inert);
    app.engine
        .apply_object_update(
            target,
            ObjectUpdate {
                change_def: Some("MNOC".to_string()),
                ..ObjectUpdate::default()
            },
        )
        .test_value();
    assert_eq!(app.engine.mouse_region_drag_source(target), None);
    let (manager, _events, mut network_commands) =
        NetworkManager::test_stub_with_commands_for_client_id(7);
    app.network = Some(manager);
    let tick = app.local_control_submission_tick();
    app.handle_ingame_mouse_button(ElementState::Released)
        .test_value();

    let (controls, commands, selections) = network_commands.take_submitted_player_inputs();
    assert!(controls.is_empty());
    assert_eq!(
        commands,
        vec![(
            tick,
            PlayerCommandControlData {
                player: owner,
                command: CommandId::Drop as i32,
                x: drop_world.x,
                y: drop_world.y,
                target: target.as_u64() as i32,
                target2: 0,
                data: 0,
                add_mode: 1,
                by_client: 7,
            },
        )]
    );
    assert!(selections.is_empty());
}

#[test]
fn inventory_region_left_drag_vehicle_queues_single_push_to() {
    let mut app = new_running_sandbox_app();
    let owner = app.local_owner;
    let crew = app.engine.test_crew_cursor(owner);
    let mut vehicle = test_definition("MIVH", "Inventory vehicle", "#strict\n");
    vehicle.set_category(clonk_engine::CATEGORY_VEHICLE);
    vehicle.set_grab(1);
    app.engine.register_test_definition(vehicle);
    let vehicle = app
        .engine
        .spawn_test_object(SpawnConfig::new("MIVH").with_container(crew));
    let crew_position = app.engine.test_object_snapshot(crew).position;
    let destination = Vector2::new(crew_position.x + 40, crew_position.y);
    let mut container = test_definition("MIPC", "Put target", "#strict\n");
    container.set_category(clonk_engine::CATEGORY_OBJECT);
    container.set_grab_put_get(clonk_engine::GRAB_PUT_GET_PUT);
    container.set_shape_rect(Some(clonk_engine::DefinitionRect::new(-6, -6, 12, 12)));
    app.engine.register_test_definition(container);
    let put_target = app
        .engine
        .spawn_test_object(SpawnConfig::new("MIPC").with_position(destination));
    app.snapshot = app.engine.snapshot();
    let mut frame = vec![0_u8; 320 * 200 * 4];
    app.test_render(&mut frame);
    let viewport = app.graphics.viewport_rect(owner).test_value();
    let region_point = GuiPoint::new(
        (viewport.x + clonk_frontend::hud::SYMBOL_BORDER + 1) as f32,
        (viewport.y + viewport.height as i32
            - clonk_frontend::hud::SYMBOL_BORDER
            - clonk_frontend::hud::SYMBOL_SIZE / 2) as f32,
    );
    assert_eq!(
        app.ingame_inventory_region_target(owner, region_point),
        Some(vehicle)
    );
    assert_eq!(
        app.engine.mouse_region_drag_source(vehicle),
        Some(MouseDragSource::Vehicle)
    );

    let (x, y) = app
        .graphics
        .world_to_screen(owner, destination)
        .test_value();
    let destination_point = GuiPoint::new(x, y);
    let destination = app
        .graphics
        .viewport_point_at(destination_point)
        .map(ingame_pointer_world_pixel)
        .test_value();
    let (manager, _events, mut network_commands) =
        NetworkManager::test_stub_with_commands_for_client_id(7);
    app.network = Some(manager);
    let tick = app.local_control_submission_tick();

    move_cursor(&mut app, region_point, "move onto vehicle region");
    app.handle_ingame_mouse_button(ElementState::Pressed)
        .test_value();
    move_cursor(&mut app, destination_point, "cross vehicle drag threshold");
    assert_eq!(app.ingame_dragged_objects, vec![vehicle]);
    move_cursor(&mut app, destination_point, "resolve vehicle moving cursor");
    assert_eq!(
        app.mouse_state
            .and_then(|state| state.motion.region_drag_cursor),
        Some(IngameRegionDragCursor::Vehicle)
    );
    // Model the last DragMoving update having resolved Ctrl+container;
    // ClearPointers may delete that stored target before button-up.
    app.mouse_state.test_mut().motion.region_drag_cursor =
        Some(IngameRegionDragCursor::VehiclePut(put_target));
    app.engine
        .apply_object_update(
            put_target,
            ObjectUpdate::new().with_status(clonk_engine::ObjectStatus::Deleted),
        )
        .test_value();
    app.handle_ingame_mouse_button(ElementState::Released)
        .test_value();

    let (controls, commands, selections) = network_commands.take_submitted_player_inputs();
    assert!(controls.is_empty());
    assert_eq!(
        commands,
        vec![(
            tick,
            PlayerCommandControlData {
                player: owner,
                command: CommandId::PushTo as i32,
                x: destination.x,
                y: destination.y,
                target: vehicle.as_u64() as i32,
                target2: 0,
                data: 0,
                add_mode: 1,
                by_client: 7,
            },
        )]
    );
    assert!(selections.is_empty());
}

#[test]
fn command_region_caption_is_immediate_and_anchored_to_region_top() {
    let (mut app, owner, points) = command_bar_fixture(false);
    let point = points
        .iter()
        .find_map(|(command, point)| (*command == 6).then_some(*point))
        .test_value();
    let (_, expected_caption, region) = app.ingame_command_region_hit(owner, point).test_value();

    move_cursor(&mut app, point, "hover command region");

    let caption = app.ingame_mouse_caption.caption.test_ref();
    let viewport = app.graphics.viewport_rect(owner).test_value();
    assert_eq!(caption.text, expected_caption);
    assert_eq!(caption.text, "Sell");
    assert_eq!(caption.caption_bottom_y, Some(region.y - viewport.y));
    assert_eq!(
        app.ingame_mouse_caption.cursor,
        IngameMouseCursorKind::Region
    );
}

#[test]
fn help_cursor_gets_the_delayed_red_help_caption() {
    let mut app = new_running_sandbox_app();
    let (_target, point) = install_mouse_help_target(&mut app, "M54H", "Help hover target", None);
    app.ingame_mouse_help = true;
    let move_to_target = |app: &mut GameApp| {
        move_cursor(app, point, "route stable Help hover move");
    };

    move_to_target(&mut app);
    assert_eq!(app.ingame_mouse_caption.cursor, IngameMouseCursorKind::Help);
    assert_eq!(app.ingame_mouse_caption.time_on_target, 0);
    for _ in 1..INGAME_MOUSE_CAPTION_DELAY {
        move_to_target(&mut app);
    }
    assert!(app.ingame_mouse_caption.caption.is_none());

    move_to_target(&mut app);
    assert!(app.ingame_mouse_help_caption.is_none());
    let expected = app.localized_ingame_mouse_caption("IDS_CON_HELP", "Help", &[], false);
    assert_eq!(
        app.ingame_mouse_caption
            .caption
            .as_ref()
            .map(|caption| caption.text.as_str()),
        Some(expected.as_str())
    );
}

#[test]
fn threshold_region_entry_waits_to_cancel_and_focus_loss_clears_drag() {
    let (mut app, owner, _cursor, _first, _target, region_point) = inventory_region_fixture();
    let viewport = app.graphics.viewport_rect(owner).test_value();
    let start = (viewport.y + 10..viewport.y + viewport.height as i32 - 48)
        .step_by(4)
        .flat_map(|y| {
            (viewport.x + 10..viewport.x + viewport.width as i32 - 36)
                .step_by(4)
                .map(move |x| GuiPoint::new(x as f32, y as f32))
        })
        .find(|point| {
            ((point.x - region_point.x).abs() > 5.0 || (point.y - region_point.y).abs() > 5.0)
                && app
                    .graphics
                    .viewport_point_at(*point)
                    .is_some_and(|pointer| pointer.owner == owner)
                && app
                    .graphics
                    .object_at_point(&app.snapshot, owner, *point)
                    .is_none()
                && app.ingame_viewport_region(owner, *point).is_none()
        })
        .test_value();

    move_cursor(&mut app, start, "move to landscape start");
    app.handle_ingame_mouse_button(ElementState::Pressed)
        .test_value();
    move_cursor(
        &mut app,
        region_point,
        "cross threshold directly into region",
    );
    assert!(app.mouse_state.is_some_and(|state| {
        state.motion.moved
            && state.motion.selection_frame
            && !state.motion.selection_cancelled_by_region
    }));
    move_cursor(&mut app, start, "leave region before a second region event");
    assert!(app.mouse_state.is_some_and(|state| {
        state.motion.selection_frame && !state.motion.selection_cancelled_by_region
    }));

    app.handle_focus_lost().test_value();
    assert!(app.mouse_state.is_none());
    assert!(app.ingame_right_mouse_state.is_none());
    assert!(app.ingame_dragged_objects.is_empty());
    assert!(!app.ingame_moving_drag_active());
}

#[test]
fn physical_inventory_region_drags_left_one_right_all_same_id_items() {
    // DrawIDList stores the first grouped item in the inventory region.
    // A left drag keeps that single target; a right drag expands it to
    // every same-ID object in forward Contents order, then emits Set and
    // Append commands (C4ObjectList.cpp:343-372;
    // C4MouseControl.cpp:942-961,1171-1227).
    let (mut app, owner, crew, first, second, region_point) = inventory_region_fixture();
    let viewport = app.graphics.viewport_rect(owner).test_value();

    let mut hidden_cursor_definition = test_definition("HINV", "Hidden inventory cursor", "");
    hidden_cursor_definition.set_hide_hud_elements(clonk_engine::HIDE_HUD_ELEMENT_INVENTORY);
    app.engine
        .register_test_definition(hidden_cursor_definition);
    let hidden_cursor = app.engine.spawn_test_object(SpawnConfig::new("HINV"));
    app.engine
        .spawn_test_object(SpawnConfig::new("MITM").with_container(hidden_cursor));
    app.engine
        .test_player_mut(owner)
        .set_view_cursor(Some(hidden_cursor));
    app.snapshot = app.engine.snapshot();
    assert!(
        !collect_crew_inventory(
            &app.engine,
            &app.snapshot,
            hidden_cursor,
            clonk_frontend::AdvancedRendererConfig::DEFAULT,
        )
        .is_empty(),
        "the hidden cursor would expose an inventory region without its mask"
    );
    assert_eq!(
        app.ingame_inventory_region_target(owner, region_point),
        None,
        "HH_Inventory creates no invisible clickable region for ViewCursor"
    );
    app.engine.test_player_mut(owner).set_view_cursor(None);
    app.snapshot = app.engine.snapshot();

    let region_left = GuiPoint::new(
        (viewport.x + clonk_frontend::hud::SYMBOL_BORDER + 1) as f32,
        region_point.y,
    );
    let region_right = GuiPoint::new(
        (viewport.x + clonk_frontend::hud::SYMBOL_BORDER + clonk_frontend::hud::SYMBOL_SIZE - 2)
            as f32,
        region_point.y,
    );
    move_cursor(&mut app, region_left, "move to first region edge");
    app.test_right_button(ElementState::Pressed);
    move_cursor(&mut app, region_right, "drag within the same region");
    app.test_right_button(ElementState::Released);
    assert!(
        app.engine
            .object_snapshot(crew)
            .expect("crew remains live")
            .command_stack
            .is_empty(),
        "a moving drag released on a region has no object command case"
    );

    let crew_position = app.engine.test_object_snapshot(crew).position;
    let landscape = app.engine.landscape().test_value();
    let drop_x = crew_position.x + 30;
    let ground_y = (0..landscape.estimated_height())
        .find(|y| landscape.is_solid_at(drop_x, *y))
        .test_value();
    let drop_world = Vector2::new(drop_x, ground_y - 1);
    let (drop_x, drop_y) = app.graphics.world_to_screen(owner, drop_world).test_value();
    let drop_pointer = (GuiPoint::new(drop_x, drop_y), drop_world, CommandId::Drop);
    assert!(
        app.graphics
            .viewport_point_at(drop_pointer.0)
            .is_some_and(|pointer| pointer.owner == owner),
        "ground drop point remains in the local viewport"
    );
    assert_eq!(
        app.engine.mouse_drag_carryable_command(owner, drop_world),
        Some(CommandId::Drop)
    );

    move_cursor(
        &mut app,
        region_point,
        "move onto inventory region for left drag",
    );
    app.handle_ingame_mouse_button(ElementState::Pressed)
        .test_value();
    move_cursor(&mut app, drop_pointer.0, "cross the left-drag threshold");
    move_cursor(
        &mut app,
        drop_pointer.0,
        "resolve the left moving-drag cursor",
    );
    app.handle_ingame_mouse_button(ElementState::Released)
        .test_value();
    let left_commands = app
        .engine
        .test_object_snapshot(crew)
        .command_stack
        .command_views();
    assert_eq!(left_commands.len(), 1);
    assert_eq!(left_commands[0].name, "Drop");
    assert_eq!(left_commands[0].target, Some(second));
    assert_eq!(left_commands[0].tx, Some(drop_pointer.1.x));
    assert_eq!(left_commands[0].ty, Some(drop_pointer.1.y));

    move_cursor(&mut app, region_point, "move onto inventory region");
    app.test_right_button(ElementState::Pressed);
    move_cursor(&mut app, drop_pointer.0, "cross the right-drag threshold");
    move_cursor(
        &mut app,
        drop_pointer.0,
        "resolve the right moving-drag cursor",
    );
    app.test_right_button(ElementState::Released);

    let commands = app
        .engine
        .test_object_snapshot(crew)
        .command_stack
        .command_views();
    assert_eq!(commands.len(), 2);
    let expected_name = match drop_pointer.2 {
        CommandId::Drop => "Drop",
        CommandId::Throw => "Throw",
        other => panic!("unexpected carryable drag command {other:?}"),
    };
    assert!(commands.iter().all(|command| command.name == expected_name));
    assert_eq!(
        commands
            .iter()
            .map(|command| command.target)
            .collect::<Vec<_>>(),
        vec![Some(second), Some(first)]
    );
    assert!(commands.iter().all(|command| {
        command.tx == Some(drop_pointer.1.x) && command.ty == Some(drop_pointer.1.y)
    }));
}

#[test]
fn selected_player_autostop_stops_horizontal_keys_on_release() {
    // LocalControlKeyUp forwards key-up in AutoStopControl mode
    // (C4Game.cpp:3578-3592).
    assert_selected_player_horizontal_release(true);
}

#[test]
fn every_target_reports_the_repeated_key_flag() {
    // C++ picks its repeated-key semantics per windowing backend:
    //
    //   Win32  DoKeyboardInput(..., !!(lParam & 0x40000000), ...)
    //          (C4Viewport.cpp:89,100; C4FullScreen.cpp:59,64)
    //   X11    passes false, then C4Game::DoKeyboardInput re-derives it from
    //          its PressedKeys map inside #ifdef USE_X11 (C4Game.cpp:2143-2154)
    //   SDL    passes a literal false for every keydown AND keyup
    //          (C4FullScreen.cpp:387-400) with no synthesis at all
    //
    // and SDL is the default main loop on Apple (CMakeLists.txt:198-200), so
    // that build cannot tell a held key from a tapped one. The port declines to
    // model that gap: winit reports repeats on every target it runs on, and
    // without the flag `C4Player::InCom` turns a held direction key into a
    // stream of Control*Double callbacks at the host's repeat rate
    // (C4Player.cpp:1532-1533).
    use crate::game_app_input::{engine_key_repeated, BACKEND_SYNTHESIZES_KEY_REPEAT};
    assert!(
        engine_key_repeated(true, BACKEND_SYNTHESIZES_KEY_REPEAT),
        "no supported target hides a held key's repeat from the engine"
    );
    assert!(
        engine_key_repeated(true, true),
        "a keydown for a key that is already down is a repeat"
    );
    assert!(
        !engine_key_repeated(true, false),
        "a backend that cannot report repeats still yields a fresh press"
    );
    // A first press is never a repeat on any backend.
    assert!(!engine_key_repeated(false, true));
    assert!(!engine_key_repeated(false, false));
}

#[test]
fn autostop_ignores_repeated_physical_keydown_until_release() {
    // Winit reports the operating system's key-repeat as another Pressed event.
    // C4Game::LocalControlKey swallows that repeat for AutoStopControl players
    // before C4Player::InCom can turn it into a COM_Double (C4Game.cpp:
    // 3560-3573). A false Down_D makes DFA_PUSH ungrab its target, which is why
    // holding Down while tensioning CATA could unexpectedly deselect the
    // catapult.
    let mut app = GameApp::new(
        320,
        200,
        AudioOptions::default(),
        None,
        RuntimeConfig {
            player_owner: 1,
            player_name: "Repeat tester".to_string(),
            network: None,
            record_enabled: false,
        },
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

    let mut keyboard = AppVirtualKeyboard::new(&mut app);
    keyboard.press(VirtualKeyCode::KeyX);
    let first_press = keyboard.player_control();
    keyboard.press(VirtualKeyCode::KeyX);
    let repeated_press = keyboard.player_control();

    assert_eq!(
        repeated_press.last_com, first_press.last_com,
        "OS key-repeat must not synthesize a gameplay COM_Down_D"
    );
    assert_eq!(
        repeated_press.last_com_down_double, first_press.last_com_down_double,
        "OS key-repeat must not arm the drop/double-down window"
    );
    keyboard.release(VirtualKeyCode::KeyX);
    assert_eq!(keyboard.player_control().pressed_coms, 0);
}

#[test]
fn cursor_portrait_colorization_uses_the_portrayed_objects_owner() {
    let mut app = new_running_sandbox_app();
    let temp = tempdir();
    let def_dir = temp.path().join("PortraitCrew.c4d");
    fs::create_dir(&def_dir).test_value();
    fs::write(
        def_dir.join("DefCore.txt"),
        b"[DefCore]\nid=PCRO\nColorByOwner=1\n",
    )
    .test_value();
    write_test_definition_graphics(&def_dir);
    image::RgbaImage::from_raw(2, 1, vec![10, 20, 30, 255, 40, 50, 60, 255])
        .expect("portrait pixels")
        .save(def_dir.join("Portrait1.png"))
        .test_value();
    image::RgbaImage::from_raw(2, 1, vec![136, 136, 136, 255, 64, 128, 192, 128])
        .expect("overlay pixels")
        .save(def_dir.join("Overlay1.png"))
        .test_value();

    let group = Group::open(&def_dir).test_value();
    let resource = ResourceDefinitionData::load(&group).test_value();
    app.engine
        .register_test_definition(Definition::from_resource(&resource).test_value());
    app.engine
        .register_script_definition("PHST", "Portrait host", "")
        .test_value();
    let viewed_owner = app.local_owner + 1;
    app.engine
        .register_player(
            PlayerConfig::new(viewed_owner, "Viewed").with_color(Some(RgbColor::new(255, 0, 0))),
        )
        .test_value();
    let viewed_object = app
        .engine
        .spawn_test_object(SpawnConfig::new("PHST").with_owner(viewed_owner));
    let mut state = app.engine.capture_state();
    state.crew_object_infos.insert(
        viewed_object,
        clonk_engine::CrewObjectInfo {
            core: Default::default(),
            definition_id: "PHST".to_string(),
            name: "Viewed crew".to_string(),
            death_message: String::new(),
            rank: 0,
            rank_name: "Clonk".to_string(),
            experience: 0,
            rounds: 0,
            death_count: 0,
            total_playing_time: 0,
            birthday: 0,
            age: 0,
            participation: 0,
            in_action_time: 0,
            extra_data: Vec::new(),
            portraits: clonk_engine::CrewPortraitState {
                current: Some(clonk_engine::CrewPortrait {
                    source: Some("PCRO".to_string()),
                    name: "1".to_string(),
                }),
                ..clonk_engine::CrewPortraitState::default()
            },
        },
    );
    app.engine.restore_state(&state).test_value();
    app.snapshot = app.engine.snapshot();
    let mut color_defaults = app.snapshot.clone();
    color_defaults
        .players
        .iter_mut()
        .find(|player| player.id == viewed_owner)
        .test_value()
        .color = None;
    assert_eq!(
        cursor_portrait_owner_color(&color_defaults, viewed_owner),
        0xff
    );
    assert_eq!(cursor_portrait_owner_color(&color_defaults, 99), u32::MAX);

    let mut players = collect_player_overlays(
        &mut app.engine,
        &app.snapshot,
        Some(viewed_object),
        &app.bindings,
        &app.gamepad_bindings,
    );
    let viewport_player = players
        .iter_mut()
        .find(|player| player.owner == app.local_owner)
        .test_value();
    let crew = viewport_player.crew.first_mut().test_value();
    crew.object_id = viewed_object;
    app.display_flags.portraits = true;
    app.populate_crew_portraits(&mut players);

    let portrait = players
        .iter()
        .find(|player| player.owner == app.local_owner)
        .and_then(|player| {
            player
                .crew
                .iter()
                .find(|crew| crew.object_id == viewed_object)
        })
        .test_value();
    assert_eq!(
        portrait.portrait.as_ref().expect("base").pixels(),
        &[10, 20, 30, 255, 40, 50, 60, 255],
        "explicit OverlayN pixels stay in the second C++ surface and do not pre-darken the base"
    );
    assert_eq!(
        portrait
            .portrait_owner_overlay
            .as_ref()
            .expect("owner overlay")
            .pixels(),
        &[136, 136, 136, 255, 64, 128, 192, 128],
        "colored and partial-alpha OverlayN pixels must reach DrawClr intact"
    );
    assert_eq!(
        portrait.portrait_owner_color, 0x00ff_0000,
        "viewport owner differs, so red proves the viewed object's owner won"
    );
}

#[test]
fn tutorial_portrait_spec_resolves_the_colorized_definition_image() {
    // C4Game::DrawTextSpecImage parses Portrait:SCLK::0000ff::1 through
    // C4Portrait::EvaluatePortraitString, resolves portrait "1", and
    // applies 0x0000ff through GetBitmap(dwClr) (C4Game.cpp:4310-4324;
    // C4DefGraphics.cpp:575-606). The tutorial must draw that image, not
    // the old flat-blue placeholder.
    let temp = tempdir();
    let def_dir = temp.path().join("Sorcerer.c4d");
    fs::create_dir(&def_dir).test_value();
    fs::write(
        def_dir.join("DefCore.txt"),
        b"[DefCore]\nid=SCLK\nColorByOwner=1\n",
    )
    .test_value();
    let mut base = Surface::new(1, 1, clonk_graphics::PixelFormat::Rgba8888);
    base.set_pixel(0, 0, Color::new(0, 0, 0, 0)).test_value();
    fs::write(
        def_dir.join("Portrait1.png"),
        encode_surface_to_png(&base).test_value(),
    )
    .test_value();
    let mut overlay = Surface::new(1, 1, clonk_graphics::PixelFormat::Rgba8888);
    overlay
        .set_pixel(0, 0, Color::new(136, 136, 136, 255))
        .test_value();
    fs::write(
        def_dir.join("Overlay1.png"),
        encode_surface_to_png(&overlay).test_value(),
    )
    .test_value();
    let mut captain = Surface::new(2, 1, clonk_graphics::PixelFormat::Rgba8888);
    captain
        .set_pixel(0, 0, Color::opaque(10, 20, 30))
        .test_value();
    captain
        .set_pixel(1, 0, Color::opaque(40, 50, 60))
        .test_value();
    fs::write(
        def_dir.join("PortraitCaptain1.png"),
        encode_surface_to_png(&captain).test_value(),
    )
    .test_value();
    let group = Group::open(&def_dir).test_value();
    let resource = ResourceDefinitionData::load(&group).test_value();
    let definition = Definition::from_resource(&resource).test_value();
    let mut engine = Engine::new();
    engine.register_test_definition(definition);

    let portrait = resolve_message_portrait(&engine, "Portrait:SCLK::0000ff::1").test_value();
    assert_eq!((portrait.width(), portrait.height()), (1, 1));
    assert_eq!(portrait.pixels(), &[0, 0, 136, 255]);
    let fallback_tint =
        resolve_message_portrait_with_color(&engine, "Portrait:SCLK::1", 0xff0000).test_value();
    assert_eq!(fallback_tint.pixels(), &[136, 0, 0, 255]);
    let captain = resolve_message_portrait(&engine, "Portrait:SCLK::ff0000::Captain1").test_value();
    assert_eq!((captain.width(), captain.height()), (2, 1));
    assert_eq!(
        captain.pixels(),
        &[255, 225, 225, 255, 40, 50, 60, 255],
        "missing OverlayCaptain1.png auto-generates an owner mask for its blue shade"
    );
    assert!(
        resolve_message_portrait(&engine, "Portrait:SCLK::0000ff::Missing").is_none(),
        "C++ requires the requested named bitmap; it does not fall back to portrait 1"
    );
}

#[test]
fn cursor_inventory_overlay_uses_real_flag_picture_order_and_count() {
    // DrawCursorInfo forwards the cursor's ordered Contents to DrawIDList
    // (src/C4Viewport.cpp:911-917). C4ObjectListIterator groups pictures
    // only inside each contiguous same-ID chunk, so FLAG/ROCK/FLAG stays
    // three ordered rows (src/C4ObjectList.cpp:343-372,849-903).
    let repository = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .and_then(Path::parent)
        .test_value()
        .to_path_buf();
    let flag_group =
        Group::open(repository.join("content/Objects.c4d/Items.c4d/Equipment.c4d/Flag.c4d"))
            .test_value();
    let flag_resource = clonk_resources::ResourceDefinition::load(&flag_group).test_value();
    let mut engine = Engine::new();
    engine.register_test_definition(Definition::from_resource(&flag_resource).test_value());
    engine
        .register_script_definition("ROCK", "Rock", "")
        .test_value();

    let crew_id = ObjectId::new(1);
    let flag_a = ObjectId::new(2);
    let rock = ObjectId::new(3);
    let flag_b = ObjectId::new(4);
    let mut crew = make_object(crew_id.as_u64(), "CLNK", Vector2::ZERO);
    crew.owner = 0;
    crew.contents = vec![flag_a, rock, flag_b];
    let mut first_flag = make_object(flag_a.as_u64(), "FLAG", Vector2::ZERO);
    first_flag.owner = 0;
    first_flag.color = 0x00c0_2040;
    first_flag.crew_member = false;
    first_flag.container = Some(crew_id);
    let mut rock_object = make_object(rock.as_u64(), "ROCK", Vector2::ZERO);
    rock_object.owner = 0;
    rock_object.crew_member = false;
    rock_object.container = Some(crew_id);
    let mut second_flag = make_object(flag_b.as_u64(), "FLAG", Vector2::ZERO);
    second_flag.owner = 0;
    second_flag.color = 0x00c0_2040;
    second_flag.crew_member = false;
    second_flag.container = Some(crew_id);
    let mut snapshot = make_snapshot(
        vec![crew, first_flag, rock_object, second_flag],
        vec![HudPlayerSnapshot {
            owner: 0,
            crew: vec![crew_id],
            focus: Some(crew_id),
            eliminated: false,
            wealth: 0,
            score: 0,
        }],
    );
    let owner_color = RgbColor::new(0xc0, 0x20, 0x40);
    snapshot.players.push(PlayerState {
        id: 0,
        cursor: Some(crew_id),
        crew: vec![crew_id],
        color: Some(owner_color),
        ..PlayerState::default()
    });

    let bindings = KeyboardBindings::load(None);
    let mut overlays = collect_player_overlays(
        &mut engine,
        &snapshot,
        Some(crew_id),
        &bindings,
        &GamepadBindings::default(),
    );
    populate_crew_inventories(
        &engine,
        &snapshot,
        &mut overlays,
        clonk_frontend::AdvancedRendererConfig::DEFAULT,
    );

    let inventory = &overlays[0].crew[0].inventory;
    assert_eq!(inventory.len(), 3, "noncontiguous ID chunks stay separate");
    assert_eq!(inventory[0].object_id, flag_a);
    assert_eq!(inventory[0].definition_id, "FLAG");
    assert_eq!(inventory[0].count, 1);
    assert_eq!(inventory[1].object_id, rock);
    assert_eq!(inventory[1].definition_id, "ROCK");
    assert_eq!(inventory[1].count, 1);
    assert_eq!(inventory[2].object_id, flag_b);
    assert_eq!(inventory[2].definition_id, "FLAG");
    assert_eq!(inventory[2].count, 1);

    let source = engine.definition_picture_image("FLAG").test_value();
    let picture = inventory[0].picture.test_ref();
    assert_eq!((picture.width(), picture.height()), (64, 64));
    let source_pixels = source.pixels();
    let mask = source.color_mask().test_value();
    if mask.len() == source_pixels.len() {
        assert_eq!(picture.pixels(), source_pixels.as_ref());
        assert_eq!(inventory[0].picture_overlays.len(), 1);
        let owner_picture = &inventory[0].picture_overlays[0].picture;
        let sample = mask
            .chunks_exact(4)
            .position(|pixel| pixel[3] != 0)
            .test_value();
        let offset = sample * 4;
        let overlay = &mask[offset..offset + 4];
        let tint = [owner_color.r, owner_color.g, owner_color.b];
        for (channel, owner) in tint.into_iter().enumerate() {
            let expected = u16::from(overlay[channel]) * u16::from(owner) / 255;
            assert_eq!(owner_picture.pixels()[offset + channel], expected as u8);
        }
        assert_eq!(
            owner_picture.pixels()[offset + 3],
            overlay[3],
            "owner coverage remains on the second HUD pass",
        );
    } else {
        let sample = mask.iter().position(|value| *value != 0).test_value();
        let offset = sample * 4;
        let amount = u16::from(mask[sample]);
        let inverse = 255_u16 - amount;
        let tint = [owner_color.r, owner_color.g, owner_color.b];
        for (channel, owner) in tint.into_iter().enumerate() {
            let expected = (u16::from(source_pixels[offset + channel]) * inverse / 255
                + u16::from(owner) * amount / 255) as u8;
            assert_eq!(picture.pixels()[offset + channel], expected);
        }
        assert_eq!(picture.pixels()[offset + 3], source_pixels[offset + 3]);
    }

    // Buy row definition pictures use the buying player attached to the
    // menu command object, not the base/title owner and not default blue
    // (src/C4ObjectMenu.cpp:217-226; src/C4Def.cpp:1374-1378).
    let buy_item = clonk_engine::ObjectMenuItem {
        caption: "Buy Flag".to_string(),
        info_caption: String::new(),
        command: String::new(),
        command2: String::new(),
        count: 1,
        item_id: "FLAG".to_string(),
        symbol: clonk_engine::ObjectMenuSymbol::Definition,
        image: clonk_engine::ObjectMenuImage::default(),
        presentation_definition_id: None,
        picture_snapshot: None,
        picture_object: None,
        components: Vec::new(),
        selectable: true,
        value: None,
        text_display_progress: -1,
    };
    let buy_color = object_menu_buying_player_color(&snapshot, Some(crew_id));
    assert_eq!(buy_color, 0x00c0_2040);
    let buy_picture = object_menu_item_picture(
        &engine,
        &snapshot,
        &buy_item,
        buy_color,
        &HudGraphics::default(),
        0,
    )
    .test_value();
    let buy_source = engine
        .definition_picture_phase_image("FLAG", 0)
        .or_else(|| engine.definition_picture_image("FLAG"))
        .test_value();
    assert_eq!(
        buy_picture.pixels(),
        inventory_picture_pixels(&buy_source, buy_color),
    );
}

#[test]
fn cursor_inventory_composes_picture_overlay_phase() {
    // Picture2Facet draws MODE_Picture overlays after the base, using the
    // overlay's definition picture and phase (src/C4Object.cpp:3144-3151;
    // src/C4DefGraphics.cpp:655-659,834-855).
    let mut engine = Engine::new();
    let mut base = test_definition("BASE", "Base", "#strict");
    base.set_picture(Some(clonk_engine::DefinitionPicture {
        x: 0,
        y: 0,
        width: 1,
        height: 1,
    }));
    base.set_sprite_image(Some(clonk_engine::DefinitionSpriteImage {
        width: 1,
        height: 1,
        pixels: Arc::from([0xff, 0, 0, 0xff]),
        color_mask: None,
    }));
    engine.register_test_definition(base);

    let mut overlay = test_definition("OVRL", "Overlay", "#strict");
    overlay.set_picture(Some(clonk_engine::DefinitionPicture {
        x: 0,
        y: 0,
        width: 1,
        height: 1,
    }));
    overlay.set_sprite_image(Some(clonk_engine::DefinitionSpriteImage {
        width: 2,
        height: 1,
        pixels: Arc::from([0, 0xff, 0, 0xff, 0, 0, 0xff, 0xff]),
        color_mask: None,
    }));
    engine.register_test_definition(overlay);

    let mut object = make_object(1, "BASE", Vector2::ZERO);
    let mut picture_overlay =
        clonk_engine::ObjectGraphicsOverlay::new(1, clonk_engine::GraphicsOverlayMode::Picture)
            .with_definition(Some("OVRL".to_string()));
    picture_overlay.phase = 1;
    object.graphics_overlays.push(picture_overlay);
    let picture = inventory_object_picture(&engine, &object).test_value();
    assert_eq!(picture.pixels(), &[0, 0, 0xff, 0xff]);

    let object_id = object.id;
    let snapshot = make_snapshot(vec![object], Vec::new());
    let menu_item = clonk_engine::ObjectMenuItem {
        caption: "Overlay".to_string(),
        info_caption: String::new(),
        command: String::new(),
        command2: String::new(),
        count: 1,
        item_id: "BASE".to_string(),
        symbol: clonk_engine::ObjectMenuSymbol::Definition,
        image: clonk_engine::ObjectMenuImage::default(),
        presentation_definition_id: None,
        picture_snapshot: None,
        picture_object: Some(object_id),
        components: Vec::new(),
        selectable: true,
        value: None,
        text_display_progress: -1,
    };
    let menu_picture = object_menu_item_picture(
        &engine,
        &snapshot,
        &menu_item,
        0,
        &HudGraphics::default(),
        0,
    )
    .test_value();
    assert_eq!(menu_picture.pixels(), picture.pixels());
}

#[test]
fn team_header_double_click_moves_all_local_users_once_and_obeys_bulk_gates() {
    let mut app = new_menu_app(640, 480);
    let (chooser, companion) = install_test_classic_host_team_lobby(&mut app);
    let script = clonk_engine::ControlPlayerInfoEntry {
        id: 9,
        team: 1,
        player_type: clonk_engine::PLAYER_INFO_TYPE_SCRIPT,
        name: LegacyCString::from_bytes(b"Script player".to_vec()).test_value(),
        ..Default::default()
    };
    app.control_player_infos.replace_snapshot(
        8,
        [clonk_engine::PlayerInfoControlData {
            client_id: 0,
            flags: clonk_engine::CLIENT_PLAYER_INFO_FLAG_INITIAL,
            players: vec![chooser.clone(), companion.clone(), script.clone()],
            by_client: 0,
        }],
    );
    let (network, _events, mut commands) = NetworkManager::test_stub_with_commands_for_client_id(0);
    app.network = Some(network);
    app.network_mode = Some(NetworkMode::Host(HostSettings {
        bind_addr: SocketAddr::from(([127, 0, 0, 1], 0)),
        player_name: "Host".to_string(),
        prepared: None,
    }));
    {
        let metadata = app.network_team_assignment.test_mut().teams_mut();
        let target = metadata
            .teams
            .iter_mut()
            .find(|team| team.id == 2)
            .test_value();
        target.player_ids = vec![99];
        target.max_players = 1;
        metadata
            .teams
            .iter_mut()
            .find(|team| team.id == 4)
            .test_value()
            .max_players = 0;
    }

    assert!(app.select_classic_lobby_sheet(LobbySheet::Teams));
    let (_, roster) = app.classic_host_lobby_layouts().test_value();
    let point_for_team = |team_id| {
        let header = roster
            .rows
            .iter()
            .find(|layout_row| {
                matches!(
                    app.classic_host_lobby
                        .as_ref()
                        .expect("test lobby")
                        .controller
                        .rows()
                        .get(layout_row.index),
                    Some(LobbyRosterRow::Header(LobbyHeaderRow {
                        kind: LobbyRosterHeader::Team(id),
                        ..
                    })) if *id == team_id
                )
            })
            .test_value();
        GuiPoint::new((header.rect.x + 2) as f32, (header.rect.y + 2) as f32)
    };
    let other_point = point_for_team(1);
    let point = point_for_team(2);

    app.handle_classic_lobby_pointer_move(other_point)
        .test_value();
    app.handle_classic_lobby_pointer_button(ElementState::Pressed, false)
        .test_value();
    app.handle_classic_lobby_pointer_move(point).test_value();
    app.handle_classic_lobby_pointer_button(ElementState::Released, false)
        .test_value();
    app.handle_classic_lobby_pointer_button(ElementState::Pressed, false)
        .test_value();
    app.handle_classic_lobby_pointer_button(ElementState::Released, false)
        .test_value();
    assert!(
        commands.take_player_info_updates().is_empty(),
        "a drag release must not seed a later single click as a double click"
    );
    app.handle_classic_lobby_pointer_button(ElementState::Pressed, false)
        .test_value();
    app.handle_classic_lobby_pointer_button(ElementState::Released, false)
        .test_value();

    let mut moved_chooser = chooser.clone();
    moved_chooser.team = 2;
    let mut moved_companion = companion.clone();
    moved_companion.team = 2;
    assert_eq!(
        commands.take_player_info_updates(),
        vec![clonk_network::PlayerInfoUpdateRequest {
            client_id: 0,
            flags: clonk_engine::CLIENT_PLAYER_INFO_FLAG_INITIAL,
            players: vec![moved_chooser, moved_companion, script],
        }],
        "the physical double click clones one full packet and mutates every local User"
    );
    assert_eq!(
        app.control_player_infos
            .client_update_request(0)
            .expect("authoritative packet")
            .players,
        vec![
            chooser,
            companion,
            clonk_engine::ControlPlayerInfoEntry {
                id: 9,
                team: 1,
                player_type: clonk_engine::PLAYER_INFO_TYPE_SCRIPT,
                name: LegacyCString::from_bytes(b"Script player".to_vec()).unwrap(),
                ..Default::default()
            }
        ],
        "the roster waits for the authoritative player-info echo"
    );

    app.classic_host_lobby
        .test_mut()
        .controller
        .apply_countdown_packet(clonk_frontend::game_lobby::LobbyCountdownPacket::Seconds(
            11,
        ));
    app.process_classic_lobby_actions(vec![
        ClassicLobbyAction::MoveLocalPlayersIntoTeamRequested { team_id: 2 },
    ])
    .test_value();
    assert_eq!(
        commands.take_player_info_updates().len(),
        1,
        "only the final countdown phase locks team selection"
    );

    app.classic_host_lobby
        .test_mut()
        .controller
        .apply_countdown_packet(clonk_frontend::game_lobby::LobbyCountdownPacket::Abort);
    app.network_team_assignment
        .test_mut()
        .teams_mut()
        .teams
        .iter_mut()
        .find(|team| team.id == 4)
        .test_value()
        .max_players = -1;
    app.process_classic_lobby_actions(vec![
        ClassicLobbyAction::MoveLocalPlayersIntoTeamRequested { team_id: 2 },
    ])
    .test_value();
    assert!(
        commands.take_player_info_updates().is_empty(),
        "bulk selection is unavailable when every team is full"
    );

    app.network_team_assignment
        .test_mut()
        .teams_mut()
        .team_distribution = clonk_engine::InitialNetworkTeamDistribution::Random;
    app.process_classic_lobby_actions(vec![
        ClassicLobbyAction::MoveLocalPlayersIntoTeamRequested { team_id: 2 },
    ])
    .test_value();
    assert!(commands.take_player_info_updates().is_empty());
}

#[test]
fn scenario_scroll_wheel_clears_hover_ownership_until_pointer_moves() {
    let mut app = new_real_classic_menu_app(640, 480);
    app.open_scenario_browser();
    let fonts = app.assets.clonk_fonts.as_deref().test_value();
    let layout = clonk_frontend::startup_scensel::scen_sel_layout(640, 480, fonts);
    let point = GuiPoint::new((layout.list.x + 5) as f32, (layout.list.y + 5) as f32);
    move_cursor(&mut app, point, "hover scenario list");
    assert!(app.startup_element_tooltip_pending());

    app.test_mouse_wheel(MouseScrollDelta::LineDelta(0.0, -1.0), 1.0);
    assert_eq!(app.startup_tooltip.pointer_position(), None);
    assert!(!app.startup_element_tooltip_pending());

    move_cursor(&mut app, point, "re-arm scenario hover");
    app.menu_state.set_search_text("unmatched search");
    app.submit_scenario_search().test_value();
    assert_eq!(app.startup_tooltip.pointer_position(), None);
}

#[test]
fn missing_scenario_bootstrap_asset_precedes_generic_fallback() {
    let mut app = new_real_classic_menu_app(320, 200);
    Arc::get_mut(&mut app.assets)
        .test_value()
        .startup_dialog_images
        .remove("StartupScenSelIcons.png")
        .test_value();
    app.open_scenario_browser();
    let mut frame = vec![0x5a; 320 * 200 * 4];

    let error = app
        .render(&mut frame)
        .expect_err("generic scenario browser must be rejected");
    assert_startup_bootstrap_boundary(
        &error,
        vec![ClassicStartupBootstrapIssue::missing(
            "StartupScenSelIcons.png",
        )],
    );
    assert!(frame.iter().all(|byte| *byte == 0x5a));
}

#[test]
fn production_bootstrap_rejects_a_partial_cursor_resolution_set() {
    let mut app = new_menu_app(320, 200);
    let assets = Arc::get_mut(&mut app.assets).test_value();
    assets.classic_hud_resources_required = true;
    assets.cursor_atlas = drag_cursor_atlas();
    assert_eq!(
        assets
            .require_classic_global_gui_bootstrap_resources(&HashMap::new())
            .expect_err("C++ PreInit requires all eight sized cursor sheets"),
        ClassicParityBoundary::GlobalGuiBootstrapResources {
            issues: vec![ClassicGuiBootstrapIssue::missing(
                "CursorSmall..CursorXXXXXLarge",
            )],
        }
    );
}

#[test]
fn ingame_cursor_kinds_map_to_cpp_phases_and_add_rules() {
    let landing = Vector2::new(73, 41);
    let cases = [
        (IngameMouseCursorKind::Region, MouseCursorPhase::Region),
        (IngameMouseCursorKind::Help, MouseCursorPhase::Help),
        (
            IngameMouseCursorKind::Crosshair,
            MouseCursorPhase::Crosshair,
        ),
        (IngameMouseCursorKind::Dig, MouseCursorPhase::Dig),
        (
            IngameMouseCursorKind::DigMaterial,
            MouseCursorPhase::DigMaterial,
        ),
        (IngameMouseCursorKind::Enter, MouseCursorPhase::Enter),
        (IngameMouseCursorKind::Grab, MouseCursorPhase::Grab),
        (IngameMouseCursorKind::Ungrab, MouseCursorPhase::Ungrab),
        (IngameMouseCursorKind::Carryable, MouseCursorPhase::Object),
        (
            IngameMouseCursorKind::DigObject,
            MouseCursorPhase::DigObject,
        ),
        (IngameMouseCursorKind::Chop, MouseCursorPhase::Chop),
        (IngameMouseCursorKind::Build, MouseCursorPhase::Build),
        (IngameMouseCursorKind::Select, MouseCursorPhase::Select),
        (IngameMouseCursorKind::Attack, MouseCursorPhase::Attack),
        (IngameMouseCursorKind::JumpLeft, MouseCursorPhase::JumpLeft),
        (
            IngameMouseCursorKind::JumpRight,
            MouseCursorPhase::JumpRight,
        ),
        (
            IngameMouseCursorKind::Scrolling(MouseCursorPhase::UpLeft),
            MouseCursorPhase::UpLeft,
        ),
        (IngameMouseCursorKind::Drop, MouseCursorPhase::Drop),
        (
            IngameMouseCursorKind::ThrowLeft(landing),
            MouseCursorPhase::ThrowLeft,
        ),
        (
            IngameMouseCursorKind::ThrowRight(landing),
            MouseCursorPhase::ThrowRight,
        ),
        (IngameMouseCursorKind::Put, MouseCursorPhase::Put),
        (IngameMouseCursorKind::Vehicle, MouseCursorPhase::Vehicle),
        (
            IngameMouseCursorKind::VehiclePut,
            MouseCursorPhase::VehiclePut,
        ),
        (
            IngameMouseCursorKind::Construct,
            MouseCursorPhase::Construct,
        ),
        (IngameMouseCursorKind::Nothing, MouseCursorPhase::Nothing),
    ];
    for (kind, phase) in cases {
        assert_eq!(kind.phase(), phase, "{kind:?}");
    }
    assert_eq!(
        IngameMouseCursorKind::ThrowLeft(landing).throw_landing(),
        Some(landing)
    );
    assert_eq!(
        IngameMouseCursorKind::ThrowRight(landing).throw_landing(),
        Some(landing)
    );
    assert_eq!(IngameMouseCursorKind::Drop.throw_landing(), None);

    for kind in [
        IngameMouseCursorKind::Region,
        IngameMouseCursorKind::Select,
        IngameMouseCursorKind::JumpLeft,
        IngameMouseCursorKind::JumpRight,
    ] {
        assert!(!kind.allows_add_marker(), "{kind:?}");
    }
    for kind in [
        IngameMouseCursorKind::Help,
        IngameMouseCursorKind::Grab,
        IngameMouseCursorKind::ThrowRight(landing),
        IngameMouseCursorKind::Nothing,
    ] {
        assert!(kind.allows_add_marker(), "{kind:?}");
    }
}

#[test]
fn scale_native_portrait_selector_keeps_dialog_layers_in_cpp_painter_order() {
    // C4PortraitSelDlg is inserted above C4StartupPlrPropertiesDlg by
    // ShowModalDlg, so Window::Draw finishes every parent element before
    // drawing the selector and its opaque thumbnails/chrome. Screen::Draw
    // then paints an open ComboBox ContextMenu after every dialog
    // (pinned C4StartupPlrSelDlg.cpp:1509-1517;
    // C4FileSelDlg.cpp:628-629; C4Gui.cpp:573-579;
    // C4GuiContainers.cpp:33-44; C4Gui.cpp:669-689).
    for retained_gpu in [false, true] {
        let mut app = new_real_classic_menu_app(640, 480);
        app.graphics.set_runtime_sprite_filtering(3.0, false);
        app.configure_native_startup_fonts(3.0, false);
        app.retained_gpu_ordered_capture_active = retained_gpu;
        open_portrait_selector(
            &mut app,
            "new-player properties dialog",
            vec![
                PortraitLocation::new("User", "/portrait-test"),
                PortraitLocation::new("Home", "/portrait-home"),
            ],
            Vec::new(),
        );
        let selector = app
            .startup_player_properties_dialog
            .test_mut()
            .controller
            .portrait_selector_mut()
            .test_value();
        let combo =
            clonk_frontend::startup_portraitsel::portrait_sel_layout(640, 480, 2).location_combo;
        selector.handle_pointer_down(clonk_frontend::GuiPoint::new(
            (combo.x + combo.w / 2) as f32,
            (combo.y + combo.h / 2) as f32,
        ));

        let (_, _, plan) = render_ordered_test_frame(&mut app, 3.0, 1920, 1440);
        let text_batch = |needle: &str| {
            plan.batches
                .iter()
                .position(|batch| batch.text.iter().any(|command| command.text == needle))
                .unwrap_or_else(|| panic!("captured scale-native text `{needle}`"))
        };
        let parent_batch = text_batch("New player");
        let selector_batch = text_batch("Location:");
        let popup_batch = text_batch("Home");
        assert!(
            parent_batch < selector_batch,
            "parent native text must be committed before selector chrome \
                 (retained GPU: {retained_gpu}): parent={parent_batch}, \
                 selector={selector_batch}"
        );
        assert!(
            selector_batch < popup_batch,
            "selector text must be committed before context-menu chrome \
                 (retained GPU: {retained_gpu}): selector={selector_batch}, \
                 popup={popup_batch}"
        );
        let selector_batch = &plan.batches[selector_batch];
        let popup_batch = &plan.batches[popup_batch];
        if retained_gpu {
            assert!(selector_batch.logical_layer.is_none());
            assert!(
                selector_batch
                    .gpu_recorder
                    .as_ref()
                    .is_some_and(|recorder| !recorder.is_empty()),
                "the retained selector batch must record chrome after parent native text"
            );
            assert!(popup_batch.logical_layer.is_none());
            assert!(
                popup_batch
                    .gpu_recorder
                    .as_ref()
                    .is_some_and(|recorder| !recorder.is_empty()),
                "the retained popup batch must record chrome after selector native text"
            );
        } else {
            assert!(selector_batch.gpu_recorder.is_none());
            assert!(
                selector_batch.logical_layer.is_some(),
                "the CPU selector batch must composite chrome after parent native text"
            );
            assert!(popup_batch.gpu_recorder.is_none());
            assert!(
                popup_batch.logical_layer.is_some(),
                "the CPU popup batch must composite chrome after selector native text"
            );
        }
    }
}

#[test]
fn picture_button_opens_progressive_selector_and_none_preserves_unchecked_icon() {
    let _lock = env_lock().lock();
    let program_data = tempdir();
    let user_data = tempdir();
    let home = tempdir();
    fs::create_dir(home.path().join("Desktop")).test_value();
    fs::create_dir_all(program_data.path().join("planet/System.c4g")).test_value();
    let _guard = EnvGuard::set(&[
        ("LC_INSTALL_ROOT", Some(program_data.path())),
        ("LC_USER_DATA_DIR", Some(user_data.path())),
        ("HOME", Some(home.path())),
    ]);
    let paths = test_app_paths();
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

    let mut app = new_classic_menu_app(640, 480);
    app.app_paths = Some(paths.clone());
    let old_portrait = ImageData::new(1, 1, vec![1, 2, 3, 255]);
    let old_icon = ImageData::new(1, 1, vec![4, 5, 6, 255]);
    let pending = open_player_properties(&mut app, "new player properties");
    pending.replace_images(old_portrait, old_icon.clone());
    let old_icon_update = pending.big_icon_update().clone();

    app.process_startup_player_properties_actions(vec![
        clonk_frontend::startup_plrproperties::PlayerPropertiesAction::ChoosePicture,
    ]);
    let selector = portrait_selector(&app, "Picture opens the nested selector");
    // C4PortraitSelDlg adds the branded user/program paths followed by
    // each existing platform location (pinned C4FileSelDlg.cpp:534-561).
    let expected_locations = vec![
        PortraitLocation::new("LegacyClonk User Path", paths.user_data_dir()),
        PortraitLocation::new(
            "LegacyClonk Program Directory",
            PathBuf::from(format!(
                "{}{}",
                paths.install_root().display(),
                std::path::MAIN_SEPARATOR
            )),
        ),
    ];
    #[cfg(not(target_os = "windows"))]
    let mut expected_locations = expected_locations;
    #[cfg(target_os = "macos")]
    expected_locations.push(PortraitLocation::new("Home", home.path()));
    #[cfg(all(not(target_os = "windows"), not(target_os = "macos")))]
    expected_locations.push(PortraitLocation::new("Home Folder", home.path()));
    #[cfg(not(target_os = "windows"))]
    expected_locations.push(PortraitLocation::new(
        "Desktop",
        home.path().join("Desktop"),
    ));
    #[cfg(not(target_os = "windows"))]
    assert_eq!(selector.locations(), expected_locations);
    assert!(
        selector.locations()[1]
            .path
            .to_string_lossy()
            .ends_with(std::path::MAIN_SEPARATOR),
        "Config.General.ExePath keeps its trailing separator in the selector caption \
             (`C4Config.cpp:1263-1270`, `C4FileSelDlg.cpp:269-271,543`)"
    );
    #[cfg(target_os = "windows")]
    {
        assert_eq!(
            &selector.locations()[..expected_locations.len()],
            expected_locations.as_slice()
        );
        assert_eq!(
            selector.locations().last(),
            Some(&PortraitLocation::new("Home Folder", home.path(),))
        );
        let shell_labels = selector.locations()
            [expected_locations.len()..selector.locations().len() - 1]
            .iter()
            .map(|location| location.label.as_str())
            .collect::<Vec<_>>();
        assert!(
            ["My Documents", "My Pictures", "Desktop"]
                .into_iter()
                .filter(|label| shell_labels.contains(label))
                .eq(shell_labels.iter().copied()),
            "existing Windows shell folders retain the pinned C++ order"
        );
    }
    assert_eq!(
        selector.items()[1].choice(),
        &clonk_frontend::startup_portraitsel::PortraitChoice::None
    );
    assert_eq!(selector.items().len(), 2);
    assert_eq!(selector.items()[0].filename(), Some("Custom.PNG"));
    assert!(matches!(
        selector.items()[0].thumbnail(),
        clonk_frontend::startup_portraitsel::PortraitThumbnail::Pending
    ));
    assert_eq!(
        app.startup_player_properties_dialog
            .as_ref()
            .expect("properties remain open")
            .controller
            .big_icon_update(),
        &old_icon_update,
        "opening the selector does not mutate either image intent"
    );

    app.advance_startup_player_portrait_thumbnail();
    assert!(matches!(
        portrait_selector(&app, "selector remains open").items()[0].thumbnail(),
        clonk_frontend::startup_portraitsel::PortraitThumbnail::Ready(_)
    ));

    for _ in 0..6 {
        let actions = app
            .startup_player_properties_dialog
            .test_mut()
            .controller
            .handle_key_down(KeyCode::Tab);
        assert!(actions.is_empty());
    }
    // C4GuiDialogs.cpp:386-421 and C4FileSelDlg.cpp:162-169,564-572
    // place Location after the wrapped dialog controls. ComboBox Down
    // opens its ContextMenu; two menu Downs highlight index one.
    for key in [KeyCode::Down, KeyCode::Down, KeyCode::Down] {
        let actions = app
            .startup_player_properties_dialog
            .test_mut()
            .controller
            .handle_key_down(key);
        assert!(actions.is_empty());
    }
    let actions = app
        .startup_player_properties_dialog
        .test_mut()
        .controller
        .handle_key_down(KeyCode::Enter);
    app.process_startup_player_properties_actions(actions);
    let selector = portrait_selector(&app, "selector remains open after changing location");
    assert_eq!(selector.current_location_index(), 1);
    assert_eq!(selector.items().len(), 2);
    assert_eq!(selector.items()[0].filename(), Some("Program.BMP"));
    assert_eq!(
        selector.items()[1].choice(),
        &clonk_frontend::startup_portraitsel::PortraitChoice::None
    );

    app.process_startup_player_properties_actions(vec![
        clonk_frontend::startup_plrproperties::PlayerPropertiesAction::PortraitSelectorClosed {
            location_index: 1,
        },
        clonk_frontend::startup_plrproperties::PlayerPropertiesAction::ApplyPicture(
            clonk_frontend::startup_portraitsel::PortraitSelCommit {
                choice: clonk_frontend::startup_portraitsel::PortraitChoice::None,
                set_picture: true,
                set_big_icon: false,
            },
        ),
    ]);
    let controller = &app.startup_player_properties_dialog.test_ref().controller;
    assert!(controller.portrait_selector().is_none());
    assert_eq!(
        controller.portrait_update(),
        &clonk_frontend::startup_plrproperties::PlayerImageUpdate::Clear
    );
    assert_eq!(controller.big_icon_update(), &old_icon_update);
    assert_eq!(controller.big_icon_preview(), Some(&old_icon));

    let before_portrait = controller.portrait_update().clone();
    let before_icon = controller.big_icon_update().clone();
    app.process_startup_player_properties_actions(vec![
        clonk_frontend::startup_plrproperties::PlayerPropertiesAction::ChoosePicture,
    ]);
    let actions = app
        .startup_player_properties_dialog
        .test_mut()
        .controller
        .handle_key_down(KeyCode::Escape);
    assert_eq!(
        actions,
        vec![
            clonk_frontend::startup_plrproperties::PlayerPropertiesAction::PortraitSelectorClosed {
                location_index: 1
            }
        ],
        "C4FileSelDlg.cpp:575-580 persists the current row when Cancel closes the selector"
    );
    app.process_startup_player_properties_actions(actions);
    let controller = &app.startup_player_properties_dialog.test_ref().controller;
    assert!(controller.portrait_selector().is_none());
    assert_eq!(controller.portrait_update(), &before_portrait);
    assert_eq!(controller.big_icon_update(), &before_icon);
    reset_cached_app_paths();
}

#[test]
fn first_portrait_selector_open_extracts_stock_portraits_once() {
    let _lock = env_lock().lock();
    let repository = Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .and_then(Path::parent)
        .test_value();
    let user_data = tempdir();
    let _guard = EnvGuard::set(&[
        ("LC_INSTALL_ROOT", Some(repository)),
        ("LC_USER_DATA_DIR", Some(user_data.path())),
    ]);
    let paths = test_app_paths();
    paths.ensure_user_dirs().test_value();
    let graphics = main_graphics_group(&paths).test_value();

    let mut app = new_classic_menu_app(640, 480);
    app.app_paths = Some(paths.clone());
    app.open_new_startup_player_properties();
    app.process_startup_player_properties_actions(vec![
        clonk_frontend::startup_plrproperties::PlayerPropertiesAction::ChoosePicture,
    ]);

    for (source, destination) in DEFAULT_USER_PORTRAITS {
        assert_eq!(
            fs::read(paths.user_data_dir().join(destination))
                .expect("read extracted stock portrait"),
            graphics
                .read_file(source)
                .expect("read source stock portrait")
        );
    }
    let config = Config::load(paths.config_file()).test_value();
    assert!(config
        .get_in(Some("General"), "UserPortraitsWritten")
        .is_some_and(parse_config_bool));

    let clonk_path = paths.user_data_dir().join("Clonk.png");
    fs::write(&clonk_path, b"user replacement").test_value();
    let actions = app
        .startup_player_properties_dialog
        .test_mut()
        .controller
        .handle_key_down(KeyCode::Escape);
    assert_eq!(
        actions,
        vec![
            clonk_frontend::startup_plrproperties::PlayerPropertiesAction::PortraitSelectorClosed {
                location_index: 0
            }
        ]
    );
    app.process_startup_player_properties_actions(actions);
    fs::remove_file(paths.config_file()).test_value();
    app.process_startup_player_properties_actions(vec![
        clonk_frontend::startup_plrproperties::PlayerPropertiesAction::ChoosePicture,
    ]);
    assert_eq!(
        fs::read(clonk_path).expect("read retained replacement"),
        b"user replacement",
        "C++ keeps UserPortraitsWritten true in process even when its disk state is lost \
             (`C4FileSelDlg.cpp:605-626`)"
    );
    reset_cached_app_paths();
}

#[test]
fn portrait_selector_consumes_the_gamepad_select_alias_cluster() {
    let mut app = new_classic_menu_app(640, 480);
    let pending = open_player_properties(&mut app, "new player properties");
    pending.replace_images(
        ImageData::new(1, 1, vec![1, 2, 3, 255]),
        ImageData::new(1, 1, vec![4, 5, 6, 255]),
    );
    open_default_portrait_selector_on(pending);
    assert!(pending.handle_key_down(KeyCode::Down).is_empty());
    assert_eq!(
        pending
            .portrait_selector()
            .and_then(|selector| selector.selected_index()),
        Some(0)
    );

    let slot = GamepadSlot::new(0);
    app.test_gamepad_events([
        pressed_gui_button(slot, GuiButtonClass::Low),
        pressed_gamepad_action(slot, GamepadActionType::Select),
    ]);

    let controller = &app.startup_player_properties_dialog.test_ref().controller;
    assert!(controller.portrait_selector().is_none());
    assert!(controller.validation_error().is_none());
    assert_eq!(
        controller.portrait_update(),
        &clonk_frontend::startup_plrproperties::PlayerImageUpdate::Clear
    );
    assert_eq!(
        controller.big_icon_update(),
        &clonk_frontend::startup_plrproperties::PlayerImageUpdate::Clear
    );
    assert!(app.status_text.is_empty());

    open_default_portrait_selector_on(
        &mut app.startup_player_properties_dialog.test_mut().controller,
    );
    app.test_gamepad_events([
        pressed_gui_button(slot, GuiButtonClass::High),
        pressed_gamepad_action(slot, GamepadActionType::Cancel),
    ]);
    let controller = &app.startup_player_properties_dialog.test_ref().controller;
    assert!(controller.portrait_selector().is_none());
    assert!(controller.validation_error().is_none());
    assert!(app.status_text.is_empty());
}

#[test]
fn properties_high_cluster_does_not_cancel_the_parent_screen() {
    // Dialog's AnyHighButton binding owns the complete physical input,
    // including Button::Select's abstract MenuToggle alias
    // (`C4GuiDialogs.cpp:364-375`, `C4GamePadCon.cpp:216-241`).
    let mut app = new_classic_menu_app(640, 480);
    app.open_player_selection_dialog();
    app.open_new_startup_player_properties();

    let slot = GamepadSlot::new(0);
    app.test_gamepad_events([
        pressed_gui_button(slot, GuiButtonClass::High),
        pressed_gamepad_action(slot, GamepadActionType::MenuToggle),
    ]);

    assert!(app.startup_player_properties_dialog.is_none());
    assert_eq!(app.startup_view, StartupView::PlayerSelection);
    assert!(app.startup_player_dialog.is_some());
    assert!(!app.exit_requested);
}

#[test]
fn portrait_selector_honors_exact_keyboard_modifiers() {
    let mut app = new_classic_menu_app(640, 480);
    open_portrait_selector(
        &mut app,
        "new player properties",
        vec![PortraitLocation::new("User", PathBuf::from("."))],
        vec![PortraitFileEntry::from_path(PathBuf::from("./King.png")).expect("portrait entry")],
    );

    // ListBox owns Alt+Return, but activation is inert without a selected
    // item. FileSel's dialog-level confirmation binds only bare Return
    // (`C4GuiListBox.cpp:72-81,386-394`,
    // `C4FileSelDlg.cpp:118-123`).
    app.test_modifiers(ModifiersState::ALT);
    app.test_key(VirtualKeyCode::Enter, ElementState::Pressed);
    assert!(app
        .startup_player_properties_dialog
        .as_ref()
        .and_then(|pending| pending.controller.portrait_selector())
        .is_some());
    assert!(app.message_dialogs.is_empty());

    app.test_modifiers(ModifiersState::SHIFT);
    app.test_key(VirtualKeyCode::Tab, ElementState::Pressed);
    assert_eq!(
        portrait_selector(&app, "selector remains open").focus(),
        clonk_frontend::startup_portraitsel::PortraitSelControl::Location
    );

    app.test_modifiers(ModifiersState::empty());
    app.test_key(VirtualKeyCode::Tab, ElementState::Pressed);
    app.test_key(VirtualKeyCode::ArrowDown, ElementState::Pressed);
    app.test_modifiers(ModifiersState::CONTROL);
    app.test_key(VirtualKeyCode::Enter, ElementState::Pressed);

    let selector = portrait_selector(&app, "Ctrl+Enter must not confirm the portrait selector");
    assert_eq!(selector.selected_index(), Some(0));
    assert!(app.status_text.is_empty());
}

#[test]
fn portrait_selector_alt_o_activates_the_native_ok_hotkey() {
    // LanguageUS gives the standard OK button the `&OK` mnemonic, and
    // Dialog routes Alt plus that alphanumeric to Button::OnHotkey
    // (`LanguageUS.txt:531`, `C4GuiButton.cpp:54-77`,
    // `C4GuiDialogs.cpp:359-362,569-580`).
    let mut app = new_classic_menu_app(640, 480);
    open_default_portrait_selector(&mut app, "new player properties");

    app.test_modifiers(ModifiersState::ALT);
    app.test_key(VirtualKeyCode::KeyO, ElementState::Pressed);

    assert_eq!(app.message_dialogs.len(), 1);
    assert!(app
        .startup_player_properties_dialog
        .as_ref()
        .and_then(|pending| pending.controller.portrait_selector())
        .is_some());
}

#[test]
#[cfg(not(target_os = "windows"))]
fn portrait_selector_does_not_bind_keypad_enter_as_return() {
    // FileSel registers K_RETURN. X11 and SDL define that as the main
    // Return key/scancode, distinct from keypad Enter
    // (`C4FileSelDlg.cpp:118-123`, `StdApp.h:107,159`).
    let mut app = new_classic_menu_app(640, 480);
    let controller = open_default_portrait_selector(&mut app, "new player properties");
    controller.handle_key_down(KeyCode::Down);

    app.test_key(VirtualKeyCode::NumpadEnter, ElementState::Pressed);

    assert!(app
        .startup_player_properties_dialog
        .as_ref()
        .and_then(|pending| pending.controller.portrait_selector())
        .is_some());
    assert!(app.message_dialogs.is_empty());
}

#[test]
fn portrait_selector_outside_right_down_aborts_the_location_popup() {
    // Screen aborts a ContextMenu on outside RightDown before routing the
    // underlying event (`C4Gui.cpp:766-776`).
    let mut app = new_classic_menu_app(640, 480);
    let controller = open_portrait_selector(
        &mut app,
        "new player properties",
        vec![
            PortraitLocation::new("User", PathBuf::from(".")),
            PortraitLocation::new("Program", PathBuf::from("..")),
        ],
        Vec::new(),
    );
    controller.handle_key_down_with_tab_direction(KeyCode::Tab, true);
    controller.handle_key_down(KeyCode::Down);
    assert!(controller
        .portrait_selector()
        .expect("selector remains open")
        .is_location_popup_open());

    move_cursor_to(
        &mut app,
        PhysicalPosition::new(0.0, 0.0),
        "move outside location popup",
    );
    app.test_right_button(ElementState::Pressed);

    assert!(!app
        .startup_player_properties_dialog
        .as_ref()
        .and_then(|pending| pending.controller.portrait_selector())
        .expect("selector remains open")
        .is_location_popup_open());
}

#[test]
fn portrait_selector_f5_requires_dialog_keyboard_activation() {
    // FileSel binds F5 through DlgKeyCB. The binding requires the active
    // dialog and is suppressed while Screen owns a ContextMenu
    // (`C4FileSelDlg.cpp:119-123`, `C4Gui.h:1616-1629`,
    // `C4GuiDialogs.cpp:731-743`).
    let current = tempdir();
    let other = tempdir();
    fs::write(current.path().join("Old.png"), b"old").test_value();
    fs::write(other.path().join("Other.png"), b"other").test_value();
    let entries = clonk_frontend::startup_portraitsel::portrait_files_in_location(current.path())
        .test_value();

    let mut app = new_classic_menu_app(640, 480);
    let controller = open_portrait_selector(
        &mut app,
        "new player properties",
        vec![
            PortraitLocation::new("Current", current.path()),
            PortraitLocation::new("Other", other.path()),
        ],
        entries,
    );
    controller.handle_key_down_with_tab_direction(KeyCode::Tab, true);
    controller.handle_key_down(KeyCode::Down);
    controller.handle_key_down(KeyCode::Down);
    controller.handle_key_down(KeyCode::Down);
    fs::write(current.path().join("New.bmp"), b"new").test_value();

    app.test_modifiers(ModifiersState::CONTROL);
    app.test_key(VirtualKeyCode::F5, ElementState::Pressed);
    let selector = portrait_selector(&app, "selector remains open");
    assert!(!selector
        .items()
        .iter()
        .any(|item| item.filename() == Some("New.bmp")));
    assert!(selector.is_location_popup_open());

    app.test_modifiers(ModifiersState::empty());
    app.test_key(VirtualKeyCode::F5, ElementState::Pressed);
    let selector = portrait_selector(&app, "selector remains open");
    assert!(!selector
        .items()
        .iter()
        .any(|item| item.filename() == Some("New.bmp")));
    assert!(selector.is_location_popup_open());

    app.test_key(VirtualKeyCode::Escape, ElementState::Pressed);
    app.test_key(VirtualKeyCode::F5, ElementState::Pressed);
    let selector = portrait_selector(&app, "selector remains open after refresh");
    assert!(selector
        .items()
        .iter()
        .any(|item| item.filename() == Some("New.bmp")));
    assert_eq!(selector.selected_index(), None);
    assert!(!selector.is_location_popup_open());
}

#[test]
fn portrait_selector_errors_use_screen_owned_modals() {
    let mut missing = new_real_classic_menu_app(640, 480);
    open_default_portrait_selector(&mut missing, "new player properties");

    missing.test_key(VirtualKeyCode::Enter, ElementState::Pressed);
    assert!(missing
        .startup_player_properties_dialog
        .as_ref()
        .and_then(|pending| pending.controller.portrait_selector())
        .is_some());
    assert_eq!(missing.message_dialogs.len(), 1);
    assert_eq!(
        missing.message_dialogs[0].state.message(),
        "Please select a file first!"
    );

    let temp = tempdir();
    let corrupt = temp.path().join("Broken.png");
    fs::write(&corrupt, b"not an image").test_value();
    let entry = PortraitFileEntry::from_path(corrupt.clone()).test_value();
    let mut broken = new_real_classic_menu_app(640, 480);
    let controller = open_portrait_selector(
        &mut broken,
        "new player properties",
        vec![PortraitLocation::new("User", temp.path())],
        vec![entry],
    );
    controller.handle_key_down(KeyCode::Down);

    broken.test_key(VirtualKeyCode::Enter, ElementState::Pressed);
    assert!(broken
        .startup_player_properties_dialog
        .as_ref()
        .and_then(|pending| pending.controller.portrait_selector())
        .is_none());
    assert_eq!(broken.message_dialogs.len(), 1);
    assert!(
        broken.message_dialogs[0]
            .state
            .message()
            .starts_with(&format!("Error at graphics file {}: ", corrupt.display())),
        "C4StartupPlrPropertiesDlg formats loader errors through IDS_PRC_NOGFXFILE \
             (`C4StartupPlrSelDlg.cpp:1484-1503`)"
    );
}

#[test]
fn initial_portrait_location_scan_failure_is_silent() {
    // DirectoryIterator failure yields no file entries; UpdateFileList
    // still appends the null tile and displays no error
    // (`C4FileSelDlg.cpp:251-274`, `StdFile.cpp:712-847`).
    let _lock = env_lock().lock();
    let root = tempdir();
    let user_data = root.path().join("not-a-directory");
    fs::write(&user_data, b"file").test_value();
    let program_data = root.path().join("program");
    fs::create_dir_all(program_data.join("planet/System.c4g")).test_value();
    let _guard = EnvGuard::set(&[
        ("LC_INSTALL_ROOT", Some(&program_data)),
        ("LC_USER_DATA_DIR", Some(&user_data)),
        ("HOME", None),
    ]);
    let paths = test_app_paths();
    let mut app = new_classic_menu_app(640, 480);
    app.app_paths = Some(paths);
    app.open_new_startup_player_properties();

    app.process_startup_player_properties_actions(vec![
        clonk_frontend::startup_plrproperties::PlayerPropertiesAction::ChoosePicture,
    ]);

    let pending = app.startup_player_properties_dialog.test_ref();
    let selector = pending.controller.portrait_selector().test_value();
    assert_eq!(selector.items().len(), 1);
    assert_eq!(
        selector.items()[0].choice(),
        &clonk_frontend::startup_portraitsel::PortraitChoice::None
    );
    assert_eq!(selector.validation_error(), None);
}

#[test]
fn portrait_selector_gamepad_low_toggles_the_focused_checkbox_once() {
    let mut app = new_classic_menu_app(640, 480);
    let controller = open_default_portrait_selector(&mut app, "new player properties");
    controller.handle_key_down_with_tab_direction(KeyCode::Tab, false);
    let before = controller.portrait_selector().test_value().set_picture();

    let slot = GamepadSlot::new(0);
    app.test_gamepad_events([
        pressed_gui_button(slot, GuiButtonClass::Low),
        pressed_gamepad_action(slot, GamepadActionType::Select),
    ]);

    let selector = portrait_selector(&app, "checkbox activation must not accept the selector");
    assert_eq!(selector.set_picture(), !before);
    assert_eq!(
        selector.focus(),
        clonk_frontend::startup_portraitsel::PortraitSelControl::SetPicture
    );
    assert!(app.status_text.is_empty());
}

/// Escape on the main menu ends the process with no confirmation and, until
/// now, no trace: the session log simply stopped, which reads exactly like a
/// crash to anyone triaging it (clonk-org/clonk-rs#40). Record which exit ran
/// so the shutdown banner can name it.
#[test]
fn quitting_from_the_main_menu_records_why_the_session_ended() {
    let mut app = new_classic_menu_app(640, 480);
    assert_eq!(app.startup_view, StartupView::MainMenu);
    assert_eq!(app.exit_reason, None);

    app.test_key(VirtualKeyCode::Escape, ElementState::Pressed);

    assert!(app.exit_requested);
    assert_eq!(app.exit_reason, Some("the main menu was closed"));
}

#[test]
fn keyboard_subscreen_back_reconstructs_main() {
    let mut app = new_classic_menu_app(640, 480);
    enter_unported_startup_subscreen(
        &mut app,
        ClassicStartupSubscreen::Options(
            clonk_frontend::startup_options_dlg::OptionsSheet::Keyboard,
        ),
    );

    assert!(app.status_text.is_empty());
    app.test_key(VirtualKeyCode::Backspace, ElementState::Pressed);
    assert_eq!(app.startup_view, StartupView::MainMenu);
}

#[test]
fn unsupported_child_back_paths_reconstruct_retained_parent_state() {
    let mut app = new_classic_menu_app(640, 480);

    enter_about_licenses(&mut app);
    let about_back = clonk_frontend::startup_about_dlg::about_layout(640, 480).buttons[0];
    let about_back = PhysicalPosition::new(
        f64::from(about_back.x + about_back.w / 2),
        f64::from(about_back.y + about_back.h / 2),
    );
    app.test_cursor(about_back);
    app.test_left_button(ElementState::Pressed);
    app.test_left_button(ElementState::Released);
    assert_eq!(app.startup_view, StartupView::About);
    assert_eq!(
        app.startup_about_dialog.as_ref().unwrap().current_page(),
        clonk_frontend::startup_about_dlg::AboutPage::Credits
    );
    app.test_key(VirtualKeyCode::Escape, ElementState::Pressed);
    assert_eq!(app.startup_view, StartupView::MainMenu);

    app.open_network_game_dialog();
    activate_startup_network_chat(&mut app);
    let metrics = clonk_frontend::startup_netdlg::NetDlgFontMetrics {
        caption_back_extent: 51,
        text_ip_extent: 18,
        text_line_height: 22,
        caption_line_height: 25,
        title_line_height: 34,
    };
    let games = clonk_frontend::startup_netdlg::net_dlg_layout(640, 480, &metrics).btn_game_list;
    let games = PhysicalPosition::new(
        f64::from(games.x + games.w / 2),
        f64::from(games.y + games.h / 2),
    );
    app.test_cursor(games);
    app.test_left_button(ElementState::Pressed);
    app.test_left_button(ElementState::Released);
    assert_eq!(
        app.startup_network_dialog.as_ref().unwrap().mode(),
        clonk_frontend::startup_netdlg::NetDlgMode::GameList
    );

    app.open_network_game_dialog();
    activate_startup_network_chat(&mut app);
    app.test_key(VirtualKeyCode::Escape, ElementState::Pressed);
    assert_eq!(app.startup_view, StartupView::MainMenu);
}

#[test]
fn production_gamepad_batch_navigates_main_menu_then_options_sheet() {
    let mut app = new_real_classic_menu_app(640, 480);
    let mut main_menu = vec![0_u8; 640 * 480 * 4];
    app.test_render(&mut main_menu);
    process_primary_direction(
        &mut app,
        ControlButton::Down,
        "supported main-menu gamepad navigation",
    );
    let mut navigated = vec![0_u8; 640 * 480 * 4];
    app.test_render(&mut navigated);
    assert_ne!(
        main_menu, navigated,
        "the D-pad must move the main-menu selection"
    );

    app.open_options_menu();
    let mut program_sheet = vec![0xa9; 640 * 480 * 4];
    app.test_render(&mut program_sheet);
    process_primary_direction(&mut app, ControlButton::Down, "D-pad enters Graphics sheet");
    let mut graphics_sheet = vec![0xa9; 640 * 480 * 4];
    app.test_render(&mut graphics_sheet);
    assert!(graphics_sheet.iter().any(|byte| *byte != 0xa9));
    assert_ne!(
        program_sheet, graphics_sheet,
        "the D-pad must leave the Program sheet for the Graphics sheet"
    );
}

#[test]
fn gamepad_enabled_defaults_true_and_captures_false_before_config_writes() {
    let _lock = env_lock().lock();
    reset_cached_app_paths();
    let user_data = tempdir();
    let (_guard, paths) = exact_loader_test_paths(user_data.path(), None);

    assert!(
        load_gamepads_enabled(Some(&paths)),
        "the omitted native key retains C4ConfigGeneral's true default"
    );
    persist_native_config_values(
        &paths,
        "General",
        &[(
            "GamepadEnabled",
            clonk_app_netplay::NativeConfigValue::RawAscii("false"),
        )],
    )
    .test_value();
    let app = GameApp::new_with_frontend_scenarios(
        320,
        200,
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
            player_name: "Player".to_string(),
            network: None,
            record_enabled: false,
        },
        Some(Vec::new()),
    )
    .test_value();
    assert!(
        !app.gamepads_enabled,
        "startup config writes must not change the process snapshot"
    );
    assert!(!app.gamepad_input_enabled);
    drop(app);
    reset_cached_app_paths();
}

#[test]
fn global_gamepad_disable_drops_events_before_dispatch() {
    let mut app = new_real_classic_menu_app(640, 480);
    let mut initial = vec![0_u8; 640 * 480 * 4];
    app.test_render(&mut initial);
    app.gamepads_enabled = false;
    app.gamepad_input_enabled = false;
    process_primary_direction(
        &mut app,
        ControlButton::Down,
        "globally disabled input is discarded",
    );
    let mut discarded = vec![0_u8; 640 * 480 * 4];
    app.test_render(&mut discarded);
    assert_eq!(
        discarded, initial,
        "disabled events must not reach startup input dispatch"
    );

    app.gamepads_enabled = true;
    app.gamepad_input_enabled = true;
    process_primary_direction(
        &mut app,
        ControlButton::Down,
        "globally enabled input reaches dispatch",
    );
    let mut dispatched = vec![0_u8; 640 * 480 * 4];
    app.test_render(&mut dispatched);
    assert_ne!(
        dispatched, initial,
        "enabled events must move the main-menu selection"
    );
}

#[test]
fn about_gamepad_horizontal_matches_tab_order_and_primary_gui_gate() {
    use clonk_frontend::startup_about_dlg::{AboutFocusTarget, AboutPage};

    let open_about = |gamepad_gui_control| {
        let mut app = new_classic_menu_app(640, 480);
        app.gamepad_gui_control = gamepad_gui_control;
        app.open_about_dialog();
        app.startup_dialog_fade = None;
        app
    };
    let send_direction = |app: &mut GameApp, gamepad: u8, button: ControlButton| {
        let gamepad_gui_control = app.gamepad_gui_control;
        app.process_sourced_gamepad_event_batch(
            [sourced_gamepad_event(
                usize::from(gamepad),
                0,
                pressed_direction(GamepadSlot::new(gamepad), button),
            )],
            gamepad_gui_control,
        )
    };
    let focus = |app: &GameApp| app.startup_about_dialog.test_ref().focused_control();

    let mut disabled = open_about(false);
    send_direction(&mut disabled, 0, ControlButton::Right).test_value();
    assert_eq!(focus(&disabled), None);

    let mut secondary = open_about(true);
    send_direction(&mut secondary, 1, ControlButton::Right).test_value();
    assert_eq!(focus(&secondary), None);

    let mut app = open_about(true);
    send_direction(&mut app, 0, ControlButton::Right).test_value();
    assert_eq!(focus(&app), Some(AboutFocusTarget::Back));
    send_direction(&mut app, 0, ControlButton::Right).test_value();
    assert_eq!(focus(&app), Some(AboutFocusTarget::Update));
    send_direction(&mut app, 0, ControlButton::Left).test_value();
    assert_eq!(focus(&app), Some(AboutFocusTarget::Back));
    send_direction(&mut app, 0, ControlButton::Right).test_value();
    send_direction(&mut app, 0, ControlButton::Right).test_value();
    assert_eq!(focus(&app), Some(AboutFocusTarget::Licenses));

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
    assert_eq!(
        app.startup_about_dialog
            .as_ref()
            .expect("About dialog")
            .current_page(),
        AboutPage::Licenses
    );

    send_direction(&mut app, 0, ControlButton::Right).test_value();
    assert_eq!(focus(&app), Some(AboutFocusTarget::LicenseTabs));
    send_direction(&mut app, 0, ControlButton::Left).test_value();
    assert_eq!(focus(&app), Some(AboutFocusTarget::Update));
}

#[test]
fn horizontal_gamepad_navigation_never_uses_keyboard_back_or_crew_routes() {
    let mut app = new_classic_menu_app(640, 480);

    app.open_options_menu();
    process_primary_direction(
        &mut app,
        ControlButton::Left,
        "Options D-left traverses focus",
    );
    assert_eq!(app.startup_view, StartupView::Options);

    app.open_network_game_dialog();
    process_primary_direction(
        &mut app,
        ControlButton::Left,
        "Network D-left traverses focus",
    );
    assert_eq!(app.startup_view, StartupView::NetworkGame);
    assert_eq!(
        app.startup_network_dialog
            .as_ref()
            .unwrap()
            .focused_control(),
        clonk_frontend::startup_netdlg::NetDlgControl::ChatButton
    );

    app.startup_player_models
        .push(clonk_frontend::startup_plrsel::PlrSelPlayer {
            name: "Gamepad Player".to_string(),
            activated: false,
            big_icon: None,
            portrait: None,
            color_dw: 0,
            score: 0,
            rounds: 0,
            rounds_won: 0,
            rounds_lost: 0,
            total_playing_time: 0,
            comment: String::new(),
        });
    app.open_player_selection_dialog();
    process_primary_direction(
        &mut app,
        ControlButton::Right,
        "Player D-right traverses focus without Crew",
    );
    assert_eq!(app.startup_view, StartupView::PlayerSelection);
    assert_eq!(
        app.startup_player_dialog
            .as_ref()
            .unwrap()
            .focused_control(),
        clonk_frontend::startup_plrsel::PlrSelControl::Back
    );
    process_primary_direction(
        &mut app,
        ControlButton::Left,
        "Player D-left traverses focus without Back",
    );
    assert_eq!(app.startup_view, StartupView::PlayerSelection);
    assert_eq!(
        app.startup_player_dialog
            .as_ref()
            .unwrap()
            .focused_control(),
        clonk_frontend::startup_plrsel::PlrSelControl::PlayerList
    );
}

#[test]
fn options_gamepad_device_claim_switches_and_releases() {
    use clonk_frontend::startup_options_controls::ControlDevice;
    use clonk_frontend::startup_options_dlg::{OptionsDlgAction, OptionsSheet};

    let mut app = new_classic_menu_app(640, 480);
    app.open_options_menu();
    assert_eq!(app.gamepads.options_open_slot(), None);
    let controls = load_options_control_state(
        &app.bindings,
        &app.gamepad_bindings,
        3,
        app.gamepad_gui_control,
    );
    *app.startup_options_dialog.test_mut().controls_mut() = controls;

    app.startup_options_dialog
        .as_mut()
        .test_value()
        .restore_sheet(OptionsSheet::Gamepad);
    app.process_options_dialog_actions(vec![OptionsDlgAction::SheetChanged(OptionsSheet::Gamepad)])
        .test_value();
    assert!(app.gamepads.is_options_slot_live(GamepadSlot::new(0)));

    for set in [2, 1] {
        assert!(app
            .startup_options_dialog
            .as_mut()
            .unwrap()
            .controls_mut()
            .select_set(ControlDevice::Gamepad, set));
        app.process_options_dialog_actions(vec![OptionsDlgAction::GamepadDeviceSelected(set)])
            .test_value();
        assert_eq!(
            app.gamepads.options_open_slot(),
            GamepadSlot::from_index(set)
        );
        for other in 0..3 {
            assert_eq!(
                app.gamepads
                    .is_options_slot_live(GamepadSlot::new(other as u8)),
                other == set
            );
        }
    }

    app.process_options_dialog_actions(vec![OptionsDlgAction::GamepadDeviceSelected(1)])
        .test_value();
    assert_eq!(app.gamepads.options_open_slot(), Some(GamepadSlot::new(1)));
    app.process_options_dialog_actions(vec![OptionsDlgAction::GamepadDeviceSelected(3)])
        .test_value();
    assert_eq!(app.gamepads.options_open_slot(), Some(GamepadSlot::new(1)));

    app.startup_options_dialog
        .as_mut()
        .test_value()
        .restore_sheet(OptionsSheet::Network);
    app.process_options_dialog_actions(vec![OptionsDlgAction::SheetChanged(OptionsSheet::Network)])
        .test_value();
    // Leaving the sheet does not drop the claim: only `~ControlConfigArea`
    // deletes `C4GamePadOpener`, and that area lives as long as the dialog
    // (`C4StartupOptionsDlg.cpp:344-352,988-991`).
    assert_eq!(app.gamepads.options_open_slot(), Some(GamepadSlot::new(1)));

    app.startup_options_dialog
        .as_mut()
        .test_value()
        .restore_sheet(OptionsSheet::Gamepad);
    app.process_options_dialog_actions(vec![OptionsDlgAction::SheetChanged(OptionsSheet::Gamepad)])
        .test_value();
    assert_eq!(app.gamepads.options_open_slot(), Some(GamepadSlot::new(1)));

    let high = |gamepad, slot| {
        sourced_gamepad_event(
            gamepad,
            gamepad as u64,
            pressed_gui_button(slot, GuiButtonClass::High),
        )
    };
    app.process_sourced_gamepad_event_batch([high(2, GamepadSlot::new(2))], false)
        .test_value();
    assert_eq!(app.startup_view, StartupView::Options);
    assert_eq!(app.gamepads.options_open_slot(), Some(GamepadSlot::new(1)));
    app.process_sourced_gamepad_event_batch([high(1, GamepadSlot::new(1))], false)
        .test_value();
    assert_eq!(app.startup_view, StartupView::Options);
    assert_eq!(app.gamepads.options_open_slot(), Some(GamepadSlot::new(1)));

    app.gamepad_gui_control = true;
    app.process_sourced_gamepad_event_batch([high(0, GamepadSlot::new(0))], true)
        .test_value();
    assert_eq!(app.startup_view, StartupView::MainMenu);
    assert_eq!(app.gamepads.options_open_slot(), None);
    app.process_options_dialog_actions(vec![OptionsDlgAction::GamepadDeviceSelected(1)])
        .test_value();
    assert_eq!(app.gamepads.options_open_slot(), None);
}

// `OnGUIGamepadCheckChange` ends in `RecreateDialog(false)`
// (C4StartupOptionsDlg.cpp:429-438), which builds a brand-new dialog through
// `SwitchDialog(SDID_Options)` (:1331) — so both `ControlConfigArea`s are
// reconstructed and their `iSelectedCtrlSet` returns to `C4P_Control_Keyboard1`
// (:284) — then restores the active sheet index (:1332).
#[test]
fn toggling_gamepad_gui_control_recreates_the_options_dialog() {
    use clonk_frontend::startup_options_controls::ControlDevice;
    use clonk_frontend::startup_options_dlg::{OptionsDlgAction, OptionsSheet};

    let mut app = new_classic_menu_app(640, 480);
    app.open_options_menu();
    {
        let dialog = app.startup_options_dialog.test_mut();
        dialog.restore_sheet(OptionsSheet::Gamepad);
        assert!(dialog.controls_mut().select_set(ControlDevice::Keyboard, 2));
    }
    app.process_options_dialog_actions(vec![OptionsDlgAction::GamepadGuiControlChanged(true)])
        .test_value();

    let dialog = app.startup_options_dialog.test_ref();
    assert_eq!(
        dialog.active_sheet(),
        OptionsSheet::Gamepad,
        "SelectSheet(iPage, false) restores the page",
    );
    assert_eq!(
        dialog.controls().selected_set(ControlDevice::Keyboard),
        0,
        "the rebuilt ControlConfigArea starts on set 0",
    );
    assert_eq!(dialog.controls().selected_set(ControlDevice::Gamepad), 0);
    assert!(
        dialog.controls().gamepad_gui_control(),
        "the new dialog reads the flag it was rebuilt for",
    );
}

// `KeySelButton::DrawElement` prints `KeyCode2String(key, true, false)` verbatim
// (C4StartupOptionsDlg.cpp:243), and for the factory-default gamepad config every
// button is `-1` (C4Config.cpp:591-602), which reaches the SDL branch and yields a
// non-null empty name (C4KeyboardInput.cpp:375-381). There is no "Undefined"
// sentinel anywhere in C++.
#[test]
fn factory_default_control_sheets_show_empty_captions_for_unbound_slots() {
    use clonk_frontend::startup_options_controls::{
        ControlCaptureTarget, ControlDevice, CONTROL_KEY_COUNT, CONTROL_SET_COUNT,
    };

    let mut app = new_classic_menu_app(640, 480);
    app.open_options_menu();
    let dialog = app.startup_options_dialog.test_ref();
    for set in 0..CONTROL_SET_COUNT {
        for control in 0..CONTROL_KEY_COUNT {
            assert_eq!(
                dialog.controls().label(ControlCaptureTarget {
                    device: ControlDevice::Gamepad,
                    set,
                    control,
                }),
                Some(""),
                "gamepad {set} control {control} has no default binding",
            );
            // The four keyboard blocks ship fully bound, so those stay named.
            assert!(
                dialog
                    .controls()
                    .label(ControlCaptureTarget {
                        device: ControlDevice::Keyboard,
                        set,
                        control,
                    })
                    .is_some_and(|label| !label.is_empty() && label != "Undefined"),
                "keyboard {set} control {control} keeps its scancode name",
            );
        }
    }
}

// C4StartupOptionsDlg.cpp:176-177 substitutes `LoadKeyDescResStr(iKeyID)` into
// IDS_MSG_PRESSKEY/IDS_MSG_PRESSBTN — the very same `IDS_CTL_*` string the key
// button draws beside its cap (:242). The prompt must therefore quote the sheet's
// own label, not a second hand-written name.
/// `KeySelDialog::KeyDown` rejects only a gamepad/keyboard mismatch and the
/// wrong pad, then stores the raw code and closes
/// (`C4StartupOptionsDlg.cpp:189-200`) — there is no "supported key" filter, so
/// every key ends the capture. The port used to consume codes its config codec
/// cannot encode without rebinding or closing, leaving the modal up.
#[test]
fn control_capture_accepts_every_key_including_ones_the_codec_cannot_encode() {
    use clonk_frontend::startup_options_controls::{ControlCaptureTarget, ControlDevice};
    use clonk_frontend::startup_options_dlg::OptionsDlgAction;

    let mut app = new_classic_menu_app(640, 480);
    app.open_options_menu();
    let target = ControlCaptureTarget {
        device: ControlDevice::Keyboard,
        set: 1,
        control: ControlBindingId::Dig as usize,
    };

    // Codec coverage is per-platform, so the list is filtered at runtime rather
    // than asserting which of these any one target rejects.
    for key in [
        VirtualKeyCode::AudioVolumeUp,
        VirtualKeyCode::MediaPlayPause,
        VirtualKeyCode::BrowserHome,
        VirtualKeyCode::Power,
        VirtualKeyCode::KeyQ,
    ] {
        app.process_options_dialog_actions(vec![OptionsDlgAction::BeginControlCapture(target)])
            .test_value();
        assert!(
            app.message_dialogs.last().is_some(),
            "capture modal is up for {key:?}"
        );
        app.test_key(key, ElementState::Pressed);
        assert!(
            !app.message_dialogs.last().is_some_and(|dialog| matches!(
                dialog.continuation,
                MessageDialogContinuation::OptionsControlCapture(open) if open == target
            )),
            "{key:?} must close the capture modal whether or not it persists"
        );
        assert_eq!(
            app.bindings.key_for_set(1, ControlBindingId::Dig),
            Some(key),
            "{key:?} binds for the session"
        );
        app.test_key(key, ElementState::Released);
    }
}

#[test]
fn control_capture_prompt_quotes_the_sheet_label_for_every_control() {
    use clonk_frontend::startup_options_controls::{
        ControlCaptureTarget, ControlDevice, CONTROL_KEY_COUNT,
    };
    use clonk_frontend::startup_options_dlg::OptionsDlgAction;

    let mut app = new_classic_menu_app(640, 480);
    app.open_options_menu();
    let labels = app
        .startup_options_dialog
        .test_ref()
        .labels()
        .control_keys
        .clone();

    for (control, label) in labels.iter().enumerate().take(CONTROL_KEY_COUNT) {
        let target = ControlCaptureTarget {
            device: ControlDevice::Gamepad,
            set: 0,
            control,
        };
        app.process_options_dialog_actions(vec![OptionsDlgAction::BeginControlCapture(target)])
            .test_value();
        let modal = app.message_dialogs.pop().test_value();
        assert_eq!(
            modal.state.message(),
            format!("Press the button for \"{label}\" on gamepad 1."),
            "control {control} quotes its own sheet label",
        );
    }
    // The three that used to coincide are not the interesting ones; these are
    // the labels the hand-written table got wrong.
    assert_eq!(labels[0], "Select left");
    assert_eq!(labels[4], "Up / Jump");
    assert_eq!(labels[6], "Left");
    assert_eq!(labels[10], "Special 1");
}

/// The port-only push-to-talk rebind reuses the classic `IDS_MSG_DEFINEKEY`
/// modal and the same production key routing, so a modified chord is rejected
/// and a bare key is bound and shown on the button (clonk-org/clonk-rs#452).
#[test]
fn options_voice_key_capture_reuses_the_classic_modal_and_rejects_chords() {
    use clonk_frontend::message_dialog::MessageDialogIcon;
    use clonk_frontend::startup_options_dlg::OptionsDlgAction;

    let mut app = new_classic_menu_app(640, 480);
    app.audio.test_mut().options.voice_push_to_talk = VirtualKeyCode::Backquote;
    app.open_options_menu();
    app.process_options_dialog_actions(vec![OptionsDlgAction::BeginVoicePushToTalkCapture])
        .test_value();

    let modal = app.message_dialogs.last().test_value();
    assert_eq!(modal.state.caption(), "Assign key");
    assert_eq!(
        modal.state.message(),
        "Press the key to hold down while speaking."
    );
    assert_eq!(modal.state.icon(), MessageDialogIcon::Standard(24));

    // A modified chord is consumed and rejected, exactly like a crew binding.
    app.test_modifiers(ModifiersState::SHIFT);
    app.test_key(VirtualKeyCode::KeyV, ElementState::Pressed);
    assert_eq!(
        app.audio.test_ref().options.voice_push_to_talk,
        VirtualKeyCode::Backquote
    );
    assert!(app.message_dialogs.last().is_some_and(|dialog| matches!(
        dialog.continuation,
        MessageDialogContinuation::OptionsVoicePushToTalkCapture
    )));
    app.test_key(VirtualKeyCode::KeyV, ElementState::Released);

    app.test_modifiers(ModifiersState::empty());
    app.test_key(VirtualKeyCode::KeyV, ElementState::Pressed);
    assert_eq!(
        app.audio.test_ref().options.voice_push_to_talk,
        VirtualKeyCode::KeyV
    );
    assert_eq!(
        app.startup_options_dialog
            .as_ref()
            .expect("options dialog")
            .sound()
            .push_to_talk_key,
        "V"
    );
    assert!(
        app.message_dialogs.is_empty(),
        "the modal closes on capture"
    );
    app.test_key(VirtualKeyCode::KeyV, ElementState::Released);
}

#[test]
fn options_key_capture_matches_classic_modal_and_production_input_routing() {
    use clonk_frontend::message_dialog::MessageDialogIcon;
    use clonk_frontend::startup_options_controls::{ControlCaptureTarget, ControlDevice};
    use clonk_frontend::startup_options_dlg::OptionsDlgAction;

    let mut app = new_classic_menu_app(640, 480);
    app.open_options_menu();
    let keyboard_target = ControlCaptureTarget {
        device: ControlDevice::Keyboard,
        set: 2,
        control: ControlBindingId::Dig as usize,
    };
    app.process_options_dialog_actions(vec![OptionsDlgAction::BeginControlCapture(
        keyboard_target,
    )])
    .test_value();
    let keyboard_modal = app.message_dialogs.last().test_value();
    assert_eq!(keyboard_modal.state.caption(), "Assign key");
    assert_eq!(
        keyboard_modal.state.message(),
        "Press the key for \"Dig\" on keyboard block 3."
    );
    assert_eq!(keyboard_modal.state.icon(), MessageDialogIcon::Standard(24));

    let previous_key = app
        .bindings
        .key_for_set(2, ControlBindingId::Dig)
        .test_value();
    app.test_modifiers(ModifiersState::SHIFT);
    app.test_key(VirtualKeyCode::Escape, ElementState::Pressed);
    assert_eq!(
        app.bindings.key_for_set(2, ControlBindingId::Dig),
        Some(previous_key)
    );
    assert!(app.message_dialogs.last().is_some_and(|dialog| matches!(
        dialog.continuation,
        MessageDialogContinuation::OptionsControlCapture(target) if target == keyboard_target
    )));
    app.test_key(VirtualKeyCode::Escape, ElementState::Released);
    app.test_modifiers(ModifiersState::empty());
    app.test_key(VirtualKeyCode::Escape, ElementState::Pressed);
    assert_eq!(
        app.bindings.key_for_set(2, ControlBindingId::Dig),
        Some(VirtualKeyCode::Escape)
    );
    assert_eq!(
        app.startup_options_dialog
            .as_ref()
            .unwrap()
            .controls()
            .label(keyboard_target),
        Some("Escape")
    );
    let mut config = Config::new();
    app.bindings.write_to_config(&mut config);
    assert!(config.get_in(Some("Controls"), "Kbd3Key6").is_some());
    app.test_key(VirtualKeyCode::Escape, ElementState::Released);

    let gamepad_target = ControlCaptureTarget {
        device: ControlDevice::Gamepad,
        set: 2,
        control: ControlBindingId::Dig as usize,
    };
    app.process_options_dialog_actions(vec![OptionsDlgAction::BeginControlCapture(gamepad_target)])
        .test_value();
    let gamepad_modal = app.message_dialogs.last().test_value();
    assert_eq!(gamepad_modal.state.caption(), "Assign key");
    assert_eq!(
        gamepad_modal.state.message(),
        "Press the button for \"Dig\" on gamepad 3."
    );
    assert_eq!(gamepad_modal.state.icon(), MessageDialogIcon::Standard(25));

    let source = sourced_gamepad_event;
    let wrong_slot = GamepadSlot::new(1);
    app.process_sourced_gamepad_event_batch(
        [
            source(1, 0, pressed_gui_button(wrong_slot, GuiButtonClass::High)),
            source(
                1,
                0,
                pressed_gamepad_action(wrong_slot, GamepadActionType::Cancel),
            ),
            source(
                1,
                0,
                pressed_gamepad_button(wrong_slot, LegacyGamepadButton::new(3)),
            ),
        ],
        true,
    )
    .test_value();
    assert!(app.message_dialogs.last().is_some_and(|dialog| matches!(
        dialog.continuation,
        MessageDialogContinuation::OptionsControlCapture(target) if target == gamepad_target
    )));
    assert_eq!(
        app.gamepad_bindings
            .raw_key_for_set(2, ControlBindingId::Dig),
        None
    );
    let selected_slot = GamepadSlot::new(2);
    app.process_sourced_gamepad_event_batch(
        [
            source(2, 1, pressed_direction(selected_slot, ControlButton::Right)),
            source(
                2,
                1,
                pressed_gui_button(selected_slot, GuiButtonClass::High),
            ),
            source(
                2,
                1,
                pressed_gamepad_action(selected_slot, GamepadActionType::MenuToggle),
            ),
            source(
                2,
                1,
                pressed_gamepad_button(selected_slot, LegacyGamepadButton::new(5)),
            ),
        ],
        false,
    )
    .test_value();
    assert!(app.message_dialogs.is_empty());
    assert_eq!(
        app.gamepad_bindings
            .raw_key_for_set(2, ControlBindingId::Dig),
        input::legacy_gamepad_button_key(2, 5)
    );
}

#[test]
fn false_startup_config_never_polls_gamepad_manager() {
    let _lock = env_lock().lock();
    reset_cached_app_paths();
    let user_data = tempdir();
    let (_guard, paths) = exact_loader_test_paths(user_data.path(), None);
    persist_native_config_values(
        &paths,
        "General",
        &[(
            "GamepadEnabled",
            clonk_app_netplay::NativeConfigValue::RawAscii("false"),
        )],
    )
    .test_value();

    let mut app = GameApp::new_with_frontend_scenarios(
        320,
        200,
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
            player_name: "Player".to_string(),
            network: None,
            record_enabled: false,
        },
        Some(Vec::new()),
    )
    .test_value();

    assert!(!app.gamepads_enabled);
    assert!(!app.gamepad_input_enabled);
    assert!(!load_gamepads_enabled(Some(&paths)));
    persist_config_value(&paths, "Network", "Comment", "resaved").test_value();
    assert!(!load_gamepads_enabled(Some(&paths)));
    assert_eq!(app.gamepad_poll_count, 0);
    app.process_gamepad_events().test_value();
    assert_eq!(app.gamepad_poll_count, 0);
}

#[test]
fn disabled_gamepads_neither_dispatch_nor_assign_a_gamepad_set() {
    let mut app = new_running_sandbox_app();
    let original_owner = app.local_owner;
    app.local_controls = LocalControlRegistry::default();
    app.local_controls.initialize(LocalControlInit {
        owner: original_owner,
        preferred_set: GamepadSlot::new(0).control_set(),
        prefers_mouse: false,
        gamepads_enabled: true,
        replay: false,
        disable_mouse: false,
    });
    app.gamepad_input_enabled = false;
    let pressed_coms = |app: &GameApp, owner| {
        app.engine
            .snapshot()
            .players
            .into_iter()
            .find(|player| player.id == owner)
            .test_value()
            .control
            .pressed_coms
    };
    let pressed_before = pressed_coms(&app, original_owner);

    process_primary_direction(
        &mut app,
        ControlButton::Left,
        "disabled gamepad input is inert",
    );

    assert_eq!(pressed_coms(&app, original_owner), pressed_before);

    app.engine.remove_player(original_owner).test_value();
    app.local_controls.remove(original_owner);
    app.selected_player_file = Some(PlayerFile {
        name: "Gamepad preference".to_string(),
        pref_control: GamepadSlot::new(0).control_set(),
        pref_mouse: false,
        ..PlayerFile::default()
    });
    app.gamepads_enabled = false;

    app.join_local_player().test_value();
    let player = app.engine.test_player(app.local_owner);
    assert_eq!(player.control_set(), 0);
    assert_eq!(player.control_preferences(), (4, false));
    assert_eq!(app.local_controls.owner_for_set(4), None);
}

#[test]
fn unconfigured_gamepad_button_emits_no_gameplay_control() {
    // C4ConfigGamepad defaults every Button1..Button12 entry to -1 and
    // C4Game::InitKeyboard skips those entries instead of installing a
    // fallback mapping (pristine 9ffa0a5d src/C4Config.cpp:287-317;
    // src/C4Game.cpp:3439-3452).
    let mut app = new_running_sandbox_app();
    app.local_controls = LocalControlRegistry::default();
    app.local_controls.initialize(LocalControlInit {
        owner: app.local_owner,
        preferred_set: 5,
        prefers_mouse: false,
        gamepads_enabled: true,
        replay: false,
        disable_mouse: false,
    });
    assert!(app.ingame_menu.is_none());

    app.test_gamepad_events([pressed_gamepad_button(
        GamepadSlot::new(1),
        LegacyGamepadButton::new(0),
    )]);

    assert!(
        app.ingame_menu.is_none(),
        "an absent Button10 mapping must not retain Start => PlayerMenu"
    );
}

#[test]
fn nonstartup_modal_stays_unfaded_and_keeps_input_priority() {
    let mut actual_app = new_real_classic_menu_app(320, 200);
    let mut expected_app = new_real_classic_menu_app(320, 200);
    let mut scratch = vec![0_u8; 320 * 200 * 4];
    actual_app.test_render(&mut scratch);
    expected_app.test_render(&mut scratch);
    actual_app
        .handle_main_menu_activation(MainMenuItem::About)
        .test_value();
    expected_app
        .handle_main_menu_activation(MainMenuItem::About)
        .test_value();

    let make_message = || {
        clonk_frontend::message_dialog::MessageDialogState::regular_ok(
            "Message",
            "Caption",
            clonk_frontend::message_dialog::MessageDialogIcon::NOTIFY,
        )
    };
    actual_app
        .push_message_dialog(make_message(), MessageDialogContinuation::None)
        .test_value();
    expected_app
        .push_message_dialog(make_message(), MessageDialogContinuation::None)
        .test_value();

    let mut actual = vec![0_u8; scratch.len()];
    actual_app.test_render(&mut actual);
    let pending = expected_app.message_dialogs.pop().test_value();
    let mut faded_base = vec![0_u8; scratch.len()];
    expected_app.test_render(&mut faded_base);
    expected_app
        .graphics
        .surface_mut()
        .pixels_mut()
        .copy_from_slice(&faded_base);
    expected_app.message_dialogs.push(pending);
    expected_app
        .render_message_dialogs(Some(startup_gamma()))
        .test_value();
    let expected = expected_app.graphics.surface().pixels().to_vec();
    assert_eq!(
        actual, expected,
        "modal pixels must be composed after the fade"
    );

    actual_app.test_key(VirtualKeyCode::Escape, ElementState::Pressed);
    assert!(actual_app.message_dialogs.is_empty());
    assert_eq!(actual_app.startup_view, StartupView::About);
    assert_eq!(actual_app.startup_dialog_fade.as_ref().unwrap().step, 1);

    let mut options = new_real_classic_menu_app(320, 200);
    options.open_options_menu();
    options.test_render(&mut scratch);
    // `Config.General.Preloading` defaults to false on macOS and true everywhere
    // else (`C4Config.cpp:400-403`), so the dialog-local edit has to be the
    // opposite of whatever this platform loaded for the revert to be visible.
    let config_preloading = options
        .startup_options_dialog
        .test_ref()
        .program()
        .preloading;
    options
        .startup_options_dialog
        .test_mut()
        .program_mut()
        .preloading = !config_preloading;
    options
        .process_options_dialog_actions(vec![
            clonk_frontend::startup_options_dlg::OptionsDlgAction::GamepadGuiControlChanged(true),
        ])
        .test_value();
    let recreate = options.startup_dialog_fade.test_ref();
    assert_eq!(recreate.outgoing, Some(StartupDialog::Options));
    assert_eq!(recreate.incoming, StartupDialog::Options);
    // `OnGUIGamepadCheckChange` calls `RecreateDialog(false)`
    // (C4StartupOptionsDlg.cpp:437), which constructs a *new* dialog through
    // `SwitchDialog` (:1331) and so re-reads Config — an unsaved dialog-local
    // edit does not survive (:1325-1334).
    assert_eq!(
        options
            .startup_options_dialog
            .as_ref()
            .expect("recreated Options dialog")
            .program()
            .preloading,
        config_preloading,
        "the rebuilt dialog re-reads Config instead of keeping dialog-local state",
    );
    options.resize(400, 300).test_value();
    let resized = options.startup_dialog_fade.test_ref();
    assert_eq!((resized.width, resized.height, resized.step), (400, 300, 0));
    options.open_network_lobby();
    assert!(
        options.startup_dialog_fade.is_none(),
        "C4GameLobby is not a fading startup dialog"
    );
}

#[test]
fn chart_gamepad_high_close_respects_player_control_priority() {
    let slot = GamepadSlot::new(0);
    let mut chart = new_running_sandbox_app();
    chart.toggle_network_chart();
    chart.test_gamepad_events([
        pressed_gui_button(slot, GuiButtonClass::High),
        pressed_gamepad_action(slot, GamepadActionType::Cancel),
    ]);
    assert!(chart.network_chart_dialog.is_none());
    assert!(chart.message_dialogs.is_empty());

    let mut player = new_running_sandbox_app();
    player.local_controls = LocalControlRegistry::default();
    player.local_controls.initialize(LocalControlInit {
        owner: player.local_owner,
        preferred_set: slot.control_set(),
        prefers_mouse: false,
        gamepads_enabled: true,
        replay: false,
        disable_mouse: false,
    });
    let physical = LegacyGamepadButton::new(3);
    assert!(player.gamepad_bindings.rebind_button(
        0,
        ControlBindingId::Left,
        slot.index(),
        physical.index(),
    ));
    player.toggle_network_chart();
    player.test_gamepad_events([
        pressed_gui_button(slot, GuiButtonClass::High),
        pressed_gamepad_button(slot, physical),
        pressed_gamepad_action(slot, GamepadActionType::Cancel),
    ]);
    assert!(player.network_chart_dialog.is_some());
    assert_ne!(
        player
            .engine
            .player(player.local_owner)
            .expect("local sandbox player")
            .control
            .pressed_coms
            & (1 << clonk_engine::COM_LEFT),
        0
    );
}

#[test]
fn cursor_portrait_does_not_fall_back_to_definition_picture_or_crew_icon() {
    let mut app = new_classic_running_sandbox_app();
    assert!(
        app.graphics.hud_graphics().crew.is_some(),
        "fixture must carry Crew.png so the forbidden fallback is observable"
    );

    let temp = tempdir();
    let def_dir = temp.path().join("NoPortrait.c4d");
    fs::create_dir(&def_dir).test_value();
    fs::write(
        def_dir.join("DefCore.txt"),
        b"[DefCore]\nid=NPOR\nPicture=0,0,1,1\n",
    )
    .test_value();
    write_test_definition_graphics(&def_dir);
    let group = Group::open(&def_dir).test_value();
    let resource = ResourceDefinitionData::load(&group).test_value();
    app.engine
        .register_test_definition(Definition::from_resource(&resource).test_value());
    assert!(
        app.engine.definition_picture_image("NPOR").is_some(),
        "fixture must carry a definition picture"
    );
    let object = app
        .engine
        .spawn_test_object(SpawnConfig::new("NPOR").with_owner(app.local_owner));
    app.snapshot = app.engine.snapshot();

    let mut players = collect_player_overlays(
        &mut app.engine,
        &app.snapshot,
        Some(object),
        &app.bindings,
        &app.gamepad_bindings,
    );
    let crew = players
        .iter_mut()
        .flat_map(|player| &mut player.crew)
        .next()
        .test_value();
    crew.object_id = object;
    crew.portrait = app.graphics.hud_graphics().crew.clone();
    crew.portrait_owner_overlay = app.graphics.hud_graphics().crew.clone();
    app.display_flags.portraits = true;

    app.populate_crew_portraits(&mut players);

    let overlay = players
        .iter()
        .flat_map(|player| &player.crew)
        .find(|crew| crew.object_id == object)
        .test_value();
    assert!(
        overlay.portrait.is_none() && overlay.portrait_owner_overlay.is_none(),
        "neither the definition picture nor Crew.png is a portrait"
    );
}

#[test]
fn sandbox_mouse_toggle_updates_registry_and_reflected_player_state() {
    let mut app = new_running_sandbox_app();
    let owner = app.local_owner;
    let view_mode = |app: &GameApp| {
        app.engine
            .snapshot()
            .players
            .into_iter()
            .find(|player| player.id == owner)
            .test_value()
            .view_mode
    };
    let player = app.engine.test_player(owner);
    assert_eq!((player.control_set(), player.mouse_control()), (0, 1));
    let scrolled_center = Vector2::new(240, 180);
    app.engine
        .replace_player_viewports(
            owner,
            vec![clonk_engine::PlayerViewport::new(scrolled_center)],
        )
        .test_value();
    app.snapshot = app.engine.snapshot();
    let mut frame = vec![0_u8; 320 * 200 * 4];
    app.test_render(&mut frame);
    let rect = app.graphics.viewport_rect(owner).test_value();
    move_cursor_to(
        &mut app,
        PhysicalPosition::new(
            f64::from(rect.x + rect.width as i32 / 2),
            f64::from(rect.y + rect.height as i32 / 2),
        ),
        "retain a pre-toggle gameplay pointer",
    );
    assert!(app.ingame_pointer.is_some());
    app.engine
        .scroll_player_view(owner, Vector2::ZERO, 320, 200, true)
        .test_value();
    assert_eq!(view_mode(&app), PLAYER_VIEW_MODE_SCROLLING);

    app.apply_ingame_menu_action_for_player(owner, MenuAction::ToggleMouseControl)
        .test_value();
    let player = app.engine.test_player(owner);
    assert_eq!((player.control_set(), player.mouse_control()), (0, 0));
    assert_eq!(view_mode(&app), clonk_engine::PLAYER_VIEW_MODE_CURSOR);
    assert_eq!(
        player.viewports()[0].center,
        scrolled_center,
        "disabling mouse control changes only the camera mode"
    );
    assert!(!app.ingame_mouse_init_centered);
    assert!(app.ingame_pointer.is_none());
    assert!(app.ingame_edge_scroll.is_none());
    assert_eq!(app.local_controls.mouse_owner(), None);
    assert!(!app.mouse_control);

    app.apply_ingame_menu_action_for_player(owner, MenuAction::ToggleMouseControl)
        .test_value();
    let player = app.engine.test_player(owner);
    assert_eq!((player.control_set(), player.mouse_control()), (0, 1));
    assert_eq!(app.local_controls.mouse_owner(), Some(owner));
    assert!(app.mouse_control);
}

fn mouse_option_phase(app: &GameApp, player: i32) -> Option<u8> {
    app.ingame_menu
        .get(player)?
        .items()
        .iter()
        .find(|item| item.action == MenuAction::ToggleMouseControl)
        .map(|item| match &item.symbol {
            ingame_menu::MenuSymbol::Options(phase) => *phase,
            symbol => panic!("mouse option uses unexpected symbol {symbol:?}"),
        })
}

#[test]
fn options_mouse_entry_is_on_for_requesting_holder() {
    let mut app = new_running_sandbox_app();
    let holder = app.local_owner;

    let flags = app.option_flags(holder);
    assert_eq!(
        (flags.mouse_shown, flags.mouse),
        (true, true),
        "holder sees the on entry"
    );
    assert!(
        !app.option_flags(OWNER_NONE).mouse_shown,
        "a playerless observer never gets the mouse entry"
    );

    app.apply_ingame_menu_action_for_player(holder, MenuAction::ActivateOptions)
        .test_value();
    assert_eq!(mouse_option_phase(&app, holder), Some(12));
}

#[test]
fn options_mouse_entry_is_hidden_for_non_holder_while_taken() {
    let mut app = new_running_sandbox_app();
    let holder = app.local_owner;
    let other = add_secondary_local_player_for_mouse_option_test(&mut app);
    assert_eq!(app.local_controls.mouse_owner(), Some(holder));

    let flags = app.option_flags(other);
    assert_eq!((flags.mouse_shown, flags.mouse), (false, false));
    app.apply_ingame_menu_action_for_player(other, MenuAction::ActivateOptions)
        .test_value();
    assert_eq!(mouse_option_phase(&app, other), None);

    for action in [
        MenuAction::ToggleSound,
        MenuAction::ToggleMusic,
        MenuAction::ToggleMouseControl,
    ] {
        app.apply_ingame_menu_action_for_player(other, action)
            .test_value();
        assert_eq!(
            mouse_option_phase(&app, other),
            None,
            "every Options reopen remains scoped to the requesting player"
        );
    }
}

#[test]
fn unclaimed_mouse_entry_is_off_for_each_local_player() {
    let mut app = new_running_sandbox_app();
    let primary = app.local_owner;
    let secondary = add_secondary_local_player_for_mouse_option_test(&mut app);

    app.apply_ingame_menu_action_for_player(primary, MenuAction::ToggleMouseControl)
        .test_value();
    assert_eq!(app.local_controls.mouse_owner(), None);

    for player in [primary, secondary] {
        let flags = app.option_flags(player);
        assert_eq!((flags.mouse_shown, flags.mouse), (true, false));
        app.apply_ingame_menu_action_for_player(player, MenuAction::ActivateOptions)
            .test_value();
        assert_eq!(mouse_option_phase(&app, player), Some(11));
    }
}

#[test]
fn restored_mouse_toggle_clears_global_owner_without_promoting_raw_flag() {
    // A save may compile several nonzero per-player MouseControl fields.
    // The last flagged player in C4PlayerList FinalInit order owns the one
    // process-global controller; toggling that player off clears
    // Game.MouseControl and does not promote another player's surviving
    // raw field (pristine 9ffa0a5d src/C4Player.cpp:778-786,2296-2315).
    let mut app = new_running_sandbox_app();
    let primary = app.local_owner;
    let secondary = primary + 1;
    app.engine
        .register_player(PlayerConfig::new(secondary, "Secondary"))
        .test_value();
    app.engine.set_local_players([primary, secondary]);
    app.engine
        .set_player_mouse_control(primary, true)
        .test_value();
    app.engine
        .set_player_mouse_control(secondary, true)
        .test_value();

    app.local_controls = LocalControlRegistry::default();
    for (owner, preferred_set) in [(secondary, 1), (primary, 0)] {
        app.local_controls.initialize_after_restore(
            LocalControlInit {
                owner,
                preferred_set,
                prefers_mouse: false,
                gamepads_enabled: true,
                replay: false,
                disable_mouse: false,
            },
            true,
        );
    }
    app.local_controls.finalize_restored_mouse_owner([
        (primary, PlayerStatus::Active),
        (secondary, PlayerStatus::Active),
    ]);
    app.mouse_control = app.local_controls.mouse_owner().is_some();
    assert_eq!(app.local_controls.mouse_owner(), Some(secondary));

    app.apply_ingame_menu_action_for_player(secondary, MenuAction::ToggleMouseControl)
        .test_value();

    assert_eq!(
        app.engine
            .player(primary)
            .expect("primary player")
            .mouse_control(),
        1,
        "the other raw per-player flag survives"
    );
    assert_eq!(
        app.engine
            .player(secondary)
            .expect("secondary player")
            .mouse_control(),
        0
    );
    assert_eq!(app.local_controls.mouse_owner(), None);
    assert!(!app.mouse_control);
    let primary_flags = app.option_flags(primary);
    assert_eq!(
        (primary_flags.mouse_shown, primary_flags.mouse),
        (true, true)
    );
    let secondary_flags = app.option_flags(secondary);
    assert_eq!(
        (secondary_flags.mouse_shown, secondary_flags.mouse),
        (false, false),
        "a surviving raw local flag remains taken after the global owner clears"
    );
}

#[test]
fn assigned_secondary_mouse_uses_its_own_command_region_to_suppress_edge_pan() {
    let mut app = new_running_sandbox_app();
    let primary = app.local_owner;
    let secondary = primary + 1;
    let primary_crew = app.engine.test_crew_cursor(primary);
    let primary_crew_state = app.engine.test_object_snapshot(primary_crew);

    app.engine
        .register_player(PlayerConfig::new(secondary, "Secondary"))
        .test_value();
    let secondary_crew = app.engine.spawn_test_object(
        SpawnConfig::new(primary_crew_state.definition_id)
            .with_position(primary_crew_state.position)
            .with_owner(secondary)
            .with_crew_member(true),
    );
    app.engine
        .register_script_definition("MSRG", "Secondary mouse region", "#strict\n")
        .test_value();
    let container = app
        .engine
        .spawn_test_object(SpawnConfig::new("MSRG").with_position(primary_crew_state.position));
    app.engine
        .apply_object_update(
            secondary_crew,
            ObjectUpdate::new().with_container(container),
        )
        .test_value();
    app.engine
        .select_crew(secondary, [secondary_crew])
        .test_value();
    app.engine
        .set_crew_cursor(secondary, Some(secondary_crew))
        .test_value();
    app.engine
        .replace_player_viewports(
            secondary,
            vec![clonk_engine::PlayerViewport::new(Vector2::new(800, 180))
                .with_focus(Some(secondary_crew))],
        )
        .test_value();
    app.engine.set_local_players([primary, secondary]);

    app.local_controls = LocalControlRegistry::default();
    for (owner, preferred_set, prefers_mouse) in [(primary, 0, false), (secondary, 1, true)] {
        app.local_controls.initialize(LocalControlInit {
            owner,
            preferred_set,
            prefers_mouse,
            gamepads_enabled: true,
            replay: false,
            disable_mouse: false,
        });
    }
    assert_eq!(app.local_controls.mouse_owner(), Some(secondary));

    app.snapshot = app.engine.snapshot();
    let mut frame = vec![0_u8; 320 * 200 * 4];
    app.test_render(&mut frame);
    let viewport = app.graphics.viewport_rect(secondary).test_value();
    let corner = GuiPoint::new(
        (viewport.x + viewport.width as i32 - 1) as f32,
        (viewport.y + viewport.height as i32 - 1) as f32,
    );
    assert_eq!(
        app.ingame_viewport_region(primary, corner),
        None,
        "the app-local owner does not own the assigned viewport's region"
    );
    assert!(
        matches!(
            app.ingame_viewport_region(secondary, corner),
            Some(IngameViewportRegion::Command(_))
        ),
        "the secondary cursor's Exit pair covers its bottom-right edge"
    );
    let before = app.engine.player(secondary).test_value().viewports()[0].center;

    move_cursor(
        &mut app,
        corner,
        "move assigned secondary pointer onto its command region",
    );

    assert_eq!(
        app.ingame_pointer.map(|pointer| pointer.owner),
        Some(secondary)
    );
    assert!(app.ingame_edge_scroll.is_none());
    assert_eq!(
        app.engine.player(secondary).unwrap().viewports()[0].center,
        before,
        "the assigned viewport's own region suppresses its edge pan"
    );
}

fn establish_free_scroll_test_viewport(
    app: &mut GameApp,
) -> (i32, GuiPoint, GuiPoint, Vector2, Vector2) {
    let owner = app.local_owner;
    let focus = app.engine.test_crew_cursor(owner);
    app.engine
        .replace_player_viewports(
            owner,
            vec![clonk_engine::PlayerViewport::new(Vector2::new(800, 180)).with_focus(Some(focus))],
        )
        .test_value();
    app.snapshot = app.engine.snapshot();
    let mut frame = vec![0_u8; 320 * 200 * 4];
    app.test_render(&mut frame);
    let rect = app.graphics.viewport_rect(owner).test_value();
    let left = GuiPoint::new(rect.x as f32, (rect.y + rect.height as i32 / 2) as f32);
    let center = GuiPoint::new(
        (rect.x + rect.width as i32 / 2) as f32,
        (rect.y + rect.height as i32 / 2) as f32,
    );
    let before = app.engine.player(owner).test_value().viewports()[0].center;
    let retained_center = Vector2::new(rect.width as i32 / 2, rect.height as i32 / 2);
    (owner, left, center, before, retained_center)
}

#[test]
fn first_mouse_move_after_init_centers_before_edge_scroll() {
    let mut app = new_running_sandbox_app();
    let (owner, left, _, before, retained_center) = establish_free_scroll_test_viewport(&mut app);
    app.ingame_mouse_init_centered = false;

    move_cursor(
        &mut app,
        left,
        "first move is evaluated at the viewport center",
    );

    assert!(app.ingame_mouse_init_centered);
    assert_eq!(
        app.ingame_viewport_mouse
            .expect("centered gameplay point is retained")
            .position,
        retained_center
    );
    assert!(app.ingame_edge_scroll.is_none());
    assert_eq!(
        app.engine.player(owner).unwrap().viewports()[0].center,
        before
    );

    move_cursor(
        &mut app,
        left,
        "later edge move uses its physical viewport position",
    );

    assert_eq!(
        app.engine.player(owner).unwrap().viewports()[0].center,
        Vector2::new(before.x - 10, before.y)
    );
    assert_eq!(
        app.ingame_edge_scroll
            .expect("second edge move arms scrolling")
            .edge
            .cursor,
        clonk_frontend::MouseCursorPhase::Left
    );
}

#[test]
fn tick5_initializes_mouse_before_a_later_edge_move() {
    let mut app = new_running_sandbox_app();
    let (owner, left, _, _, retained_center) = establish_free_scroll_test_viewport(&mut app);
    while app.engine.frame() % 5 != 4 {
        app.test_update();
    }
    app.reset_ingame_mouse_control();

    app.test_update();

    assert_eq!(app.engine.frame() % 5, 0);
    assert!(app.ingame_mouse_init_centered);
    assert_eq!(
        app.ingame_viewport_mouse
            .expect("Tick5 retains the centered mouse coordinate")
            .position,
        retained_center
    );
    assert!(app.ingame_edge_scroll.is_none());
    let before = app.engine.player(owner).test_value().viewports()[0].center;

    move_cursor(
        &mut app,
        left,
        "post-Tick5 edge move is no longer swallowed by InitCentered",
    );

    assert_eq!(
        app.engine.player(owner).unwrap().viewports()[0].center,
        Vector2::new(before.x - 10, before.y)
    );
}

#[test]
fn button_and_wheel_moves_consume_mouse_init_centering() {
    let mut app = new_running_sandbox_app();
    let (owner, left, _, _, retained_center) = establish_free_scroll_test_viewport(&mut app);
    app.reset_ingame_mouse_control();

    app.test_left_button(ElementState::Pressed);

    assert!(app.ingame_mouse_init_centered);
    assert_eq!(
        app.ingame_viewport_mouse
            .expect("button Move retains the centered coordinate")
            .position,
        retained_center
    );
    assert!(app.mouse_state.is_some());

    app.reset_ingame_mouse_control();
    app.test_mouse_wheel(MouseScrollDelta::LineDelta(0.0, 1.0), 1.0);

    assert!(app.ingame_mouse_init_centered);
    assert!(app.ingame_viewport_mouse.is_none());
    let before = app.engine.player(owner).test_value().viewports()[0].center;
    move_cursor(
        &mut app,
        left,
        "edge motion after wheel uses the physical viewport point",
    );
    assert_eq!(
        app.engine.player(owner).unwrap().viewports()[0].center,
        Vector2::new(before.x - 10, before.y)
    );
}

#[test]
fn gameplay_wheel_routes_to_assigned_secondary_mouse_owner() {
    // C4MouseControl stores one Player and MouseWheel forwards positive
    // deltas as COM_WheelUp to that player, independent of the hovered
    // viewport (pristine 9ffa0a5d src/C4MouseControl.cpp:147-155,
    // 1040-1046).
    let mut app = new_running_sandbox_app();
    let primary = app.local_owner;
    let secondary = primary + 1;
    let primary_crew = app.engine.test_crew_cursor(primary);
    let primary_crew_state = app.engine.test_object_snapshot(primary_crew);
    app.engine
        .register_player(PlayerConfig::new(secondary, "Secondary"))
        .test_value();
    let secondary_crew = app.engine.spawn_test_object(
        SpawnConfig::new(primary_crew_state.definition_id)
            .with_position(primary_crew_state.position)
            .with_owner(secondary)
            .with_crew_member(true),
    );
    app.engine
        .select_crew(secondary, [secondary_crew])
        .test_value();
    app.engine
        .set_crew_cursor(secondary, Some(secondary_crew))
        .test_value();
    app.engine.set_local_players([primary, secondary]);
    app.local_controls = LocalControlRegistry::default();
    app.local_controls.initialize(LocalControlInit {
        owner: primary,
        preferred_set: 0,
        prefers_mouse: false,
        gamepads_enabled: true,
        replay: false,
        disable_mouse: false,
    });
    app.local_controls.initialize(LocalControlInit {
        owner: secondary,
        preferred_set: 1,
        prefers_mouse: true,
        gamepads_enabled: true,
        replay: false,
        disable_mouse: false,
    });

    for id in ["MWA1", "MWA2", "MWA3"] {
        app.engine
            .register_script_definition(id, id, "#strict\n")
            .test_value();
    }
    for crew in [primary_crew, secondary_crew] {
        for id in ["MWA1", "MWA2", "MWA3"] {
            app.engine
                .spawn_test_object(SpawnConfig::new(id).with_container(crew));
        }
    }
    let contents = |app: &GameApp, crew| app.engine.test_object_snapshot(crew).contents;
    let primary_before = contents(&app, primary_crew);
    let secondary_before = contents(&app, secondary_crew);
    let mut expected_secondary = vec![*secondary_before.last().expect("secondary has inventory")];
    expected_secondary.extend_from_slice(&secondary_before[..secondary_before.len() - 1]);

    app.test_mouse_wheel(MouseScrollDelta::LineDelta(0.0, 1.0), 1.0);

    assert_eq!(contents(&app, primary_crew), primary_before);
    assert_eq!(contents(&app, secondary_crew), expected_secondary);

    app.test_mouse_wheel(
        MouseScrollDelta::PixelDelta(PhysicalPosition::new(0.0, -1.0)),
        2.0,
    );
    assert_eq!(contents(&app, primary_crew), primary_before);
    assert_eq!(contents(&app, secondary_crew), secondary_before);
}

#[test]
fn assigned_observer_key_uses_production_dispatch_and_physical_gate() {
    let parsed = parse_runtime_key_config(b"[Keys]\nNetObsNextPlayer=Alt+N,Right,F5,F6,F7,None\n")
        .test_value();
    assert_eq!(
        parsed.net_observer_next_player,
        vec![
            RuntimeKeyChord::keyboard(VirtualKeyCode::KeyN, ModifiersState::ALT),
            RuntimeKeyChord::keyboard(VirtualKeyCode::ArrowRight, ModifiersState::empty()),
            RuntimeKeyChord::keyboard(VirtualKeyCode::F5, ModifiersState::empty()),
            RuntimeKeyChord::keyboard(VirtualKeyCode::F6, ModifiersState::empty()),
            RuntimeKeyChord::keyboard(VirtualKeyCode::F7, ModifiersState::empty()),
            RuntimeKeyChord {
                physical: RuntimePhysicalKey::Disabled,
                modifiers: ModifiersState::empty(),
            },
        ]
    );

    let mut app = new_running_sandbox_app();
    let first = app.local_owner;
    let second = first + 1;
    app.engine
        .register_player(PlayerConfig::new(second, "Second observer target"))
        .test_value();
    app.clear_physical_viewport_states();
    let observer = app.ownerless_physical_viewport_state();
    app.physical_viewports.push(observer);
    app.physical_viewports_authoritative = true;
    assert!(app.set_physical_film_view(OWNER_NONE));
    app.runtime_key_config_cache = OnceLock::new();
    app.runtime_key_config_cache
        .set(Ok(parsed.clone()))
        .test_value();

    app.test_key(VirtualKeyCode::ArrowRight, ElementState::Pressed);
    assert_eq!(app.film_view_player, Some(OWNER_NONE));

    for key in [VirtualKeyCode::F5, VirtualKeyCode::F6, VirtualKeyCode::F7] {
        assert!(app.set_physical_film_view(OWNER_NONE));
        app.test_key(key, ElementState::Pressed);
        assert_eq!(app.film_view_player, Some(first));

        assert!(app.set_physical_film_view(OWNER_NONE));
        app.keyboard_modifiers = ModifiersState::CONTROL;
        app.test_key(key, ElementState::Pressed);
        assert_eq!(app.film_view_player, Some(OWNER_NONE));
        app.keyboard_modifiers = ModifiersState::empty();
    }
    assert_eq!(runtime_flash_text(&app), Some("Actions"));
    assert!(app.set_physical_film_view(OWNER_NONE));

    app.test_key(VirtualKeyCode::KeyN, ElementState::Pressed);
    assert_eq!(app.film_view_player, Some(OWNER_NONE));
    app.keyboard_modifiers = ModifiersState::ALT;
    app.test_key(VirtualKeyCode::KeyN, ElementState::Pressed);
    assert_eq!(app.film_view_player, Some(first));
    app.test_key(VirtualKeyCode::KeyN, ElementState::Released);
    assert_eq!(app.film_view_player, Some(first));
    app.test_key(VirtualKeyCode::KeyN, ElementState::Pressed);
    assert_eq!(app.film_view_player, Some(second));
    app.test_key(VirtualKeyCode::KeyN, ElementState::Pressed);
    assert_eq!(app.film_view_player, Some(OWNER_NONE));

    app.ingame_menu.replace(
        app.local_owner,
        IngameMenuState::main_menu(
            &MainMenuConditions {
                has_player: false,
                player_count: 2,
                ..MainMenuConditions::default()
            },
            &IngameMenuLabels::default(),
        ),
    );
    app.test_key(VirtualKeyCode::KeyN, ElementState::Pressed);
    assert_eq!(app.film_view_player, Some(OWNER_NONE));
    app.ingame_menu.clear();

    app.start_running_chat(RunningChatMode::All);
    app.test_key(VirtualKeyCode::KeyN, ElementState::Pressed);
    assert_eq!(app.film_view_player, Some(OWNER_NONE));
    app.close_running_chat().test_value();

    app.test_key(VirtualKeyCode::KeyN, ElementState::Pressed);
    assert_eq!(app.film_view_player, Some(first));
    assert!(app.physical_viewports[0].is_no_owner_viewport);
    assert!(app.create_physical_viewport(first, true, true, true));
    app.check_fullscreen_physical_viewports(true);
    assert_eq!(
        app.film_view_player, None,
        "replacing the physical observer viewport drops its temporary target"
    );
    assert!(!app.primary_physical_viewport_is_no_owner());

    let mut owned = new_running_sandbox_app();
    owned.runtime_key_config_cache = OnceLock::new();
    owned.runtime_key_config_cache.set(Ok(parsed)).test_value();
    owned.keyboard_modifiers = ModifiersState::ALT;
    owned.test_key(VirtualKeyCode::KeyN, ElementState::Pressed);
    assert_eq!(owned.film_view_player, None);
}

#[test]
fn reused_player_number_gets_a_distinct_physical_camera_identity() {
    let mut app = new_lightweight_running_sandbox_app();
    let original = app.local_owner;
    let film_target = original + 1;
    app.engine
        .register_player(PlayerConfig::new(film_target, "Film target"))
        .test_value();
    let original_identity = app.physical_viewports[0].physical_identity;
    assert!(app.set_physical_film_view(film_target));

    app.remove_runtime_player_with_viewport_feedback(original)
        .test_value();
    app.engine
        .register_player(PlayerConfig::new(original, "Reused player number"))
        .test_value();
    assert!(app.create_physical_viewport(original, true, true, false));
    let new_identity = app
        .physical_viewports
        .iter()
        .find(|viewport| viewport.uses_live_player_presentation)
        .test_value()
        .physical_identity;
    assert_ne!(original_identity, new_identity);

    assert!(app.set_physical_film_view(original));
    let old = app
        .physical_viewports
        .iter()
        .find(|viewport| viewport.physical_identity == original_identity)
        .test_value();
    let new = app
        .physical_viewports
        .iter()
        .find(|viewport| viewport.physical_identity == new_identity)
        .test_value();
    assert!(!old.uses_live_player_presentation);
    assert!(new.uses_live_player_presentation);
    assert_eq!(old.displayed_player, original);
    assert_eq!(new.displayed_player, original);
}

fn sandbox_pointer_at_world(app: &mut GameApp, owner: i32, world: Vector2) -> ViewportPointer {
    app.snapshot = app.engine.snapshot();
    app.refresh_focus();
    let surface = app.graphics.surface();
    let mut frame = vec![0_u8; surface.width() as usize * surface.height() as usize * 4];
    app.test_render(&mut frame);
    let (screen_x, screen_y) = app.graphics.world_to_screen(owner, world).test_value();
    let screen = GuiPoint::new(screen_x, screen_y);
    let projected = app.graphics.viewport_point_at(screen).test_value();
    assert_eq!(projected.owner, owner);
    assert_eq!(ingame_pointer_world_pixel(projected), world);
    ViewportPointer {
        owner,
        world: FloatVector2::new(world.x as f32, world.y as f32),
        screen,
    }
}

#[test]
fn mouse_left_double_on_solid_queues_dig_and_control_material_data() {
    let mut app = new_running_sandbox_app();
    let owner = app.local_owner;
    let mut landscape = clonk_engine::Landscape::flat(640, 50);
    landscape.set_world_height(200);
    app.engine.set_landscape(landscape);
    app.snapshot = app.engine.snapshot();
    app.refresh_focus();
    let mut frame = vec![0_u8; 320 * 200 * 4];
    app.test_render(&mut frame);
    let viewport = app.graphics.viewport_rect(owner).test_value();
    let pointer = (viewport.y..viewport.y + viewport.height as i32)
        .flat_map(|y| {
            (viewport.x..viewport.x + viewport.width as i32)
                .map(move |x| GuiPoint::new(x as f32 + 0.5, y as f32 + 0.5))
        })
        .find_map(|screen| {
            let pointer = app.graphics.viewport_point_at(screen)?;
            let point = ingame_pointer_world_pixel(pointer);
            (pointer.owner == owner
                && point.x != 0
                && point.y != 0
                && app
                    .engine
                    .landscape()
                    .is_some_and(|landscape| landscape.is_solid_at(point.x, point.y))
                && app.ingame_primary_mouse_target(owner, screen).is_none()
                && app.ingame_viewport_region(owner, screen).is_none()
                && !app.engine.mouse_jump_zone(owner, point))
            .then_some(pointer)
        })
        .test_value();
    let point = ingame_pointer_world_pixel(pointer);
    app.ingame_pointer = Some(pointer);

    let (manager, _event_tx, mut commands) =
        NetworkManager::test_stub_with_commands_for_client_id(7);
    app.network = Some(manager);
    let tick = app.local_control_submission_tick();
    app.on_ingame_mouse_double().test_value();
    assert_eq!(
        commands.take_submitted_player_commands(),
        vec![(
            tick,
            PlayerCommandControlData {
                player: owner,
                command: CommandId::Dig as i32,
                x: point.x,
                y: point.y,
                target: 0,
                target2: 0,
                data: 0,
                add_mode: 1,
                by_client: 7,
            },
        )],
    );

    app.test_modifiers(ModifiersState::CONTROL);
    app.on_ingame_mouse_double().test_value();
    assert_eq!(
        commands.take_submitted_player_commands(),
        vec![(
            tick,
            PlayerCommandControlData {
                player: owner,
                command: CommandId::Dig as i32,
                x: point.x,
                y: point.y,
                target: 0,
                target2: 0,
                data: 1,
                add_mode: 1,
                by_client: 7,
            },
        )],
    );
}

#[test]
fn mouse_jump_zone_click_queues_exact_jump_control() {
    let mut app = new_running_sandbox_app();
    let owner = app.local_owner;
    let cursor = app.engine.test_crew_cursor(owner);
    let position = app.engine.test_object_snapshot(cursor).position;
    let click = Vector2::new(position.x + 8, position.y - 15);
    let pointer = sandbox_pointer_at_world(&mut app, owner, click);
    assert!(app.engine.mouse_jump_zone(owner, click));

    let (manager, _event_tx, mut commands) =
        NetworkManager::test_stub_with_commands_for_client_id(7);
    app.network = Some(manager);
    let tick = app.local_control_submission_tick();
    app.handle_ingame_mouse_click(pointer).test_value();

    assert_eq!(
        commands.take_submitted_player_commands(),
        vec![(
            tick,
            PlayerCommandControlData {
                player: owner,
                command: CommandId::Jump as i32,
                x: click.x,
                y: click.y,
                target: 0,
                target2: 0,
                data: 0,
                add_mode: 1,
                by_client: 7,
            },
        )]
    );

    app.test_modifiers(ModifiersState::SHIFT);
    app.handle_ingame_mouse_click(pointer).test_value();
    assert_eq!(
        commands.take_submitted_player_commands(),
        vec![(
            tick,
            PlayerCommandControlData {
                player: owner,
                command: CommandId::Jump as i32,
                x: click.x,
                y: click.y,
                target: 0,
                target2: 0,
                data: 0,
                add_mode: 1 | 4,
                by_client: 7,
            },
        )]
    );
}

#[test]
fn mouse_jump_zone_contained_or_non_walk_falls_back_to_move_to() {
    for contained in [true, false] {
        let mut app = new_running_sandbox_app();
        let owner = app.local_owner;
        let cursor = app.engine.test_crew_cursor(owner);
        let position = app.engine.test_object_snapshot(cursor).position;
        let click = Vector2::new(position.x + 8, position.y - 15);
        if contained {
            let container = test_definition("MBOX", "Mouse box", "#strict\n");
            app.engine.register_test_definition(container);
            let container = app.engine.spawn_test_object(
                SpawnConfig::new("MBOX").with_position(Vector2::new(position.x + 80, position.y)),
            );
            app.engine
                .apply_object_update(cursor, ObjectUpdate::new().with_container(container))
                .test_value();
        } else {
            app.engine
                .apply_object_update(cursor, ObjectUpdate::new().with_action("Jump"))
                .test_value();
        }
        assert!(
            !app.engine.mouse_jump_zone(owner, click),
            "contained={contained} must disable the jump cursor"
        );
        let pointer = sandbox_pointer_at_world(&mut app, owner, click);
        assert_eq!(
            app.ingame_mouse_select_target(owner, pointer.screen),
            None,
            "fallback point must remain a plain movement click"
        );

        let (manager, _event_tx, mut commands) =
            NetworkManager::test_stub_with_commands_for_client_id(7);
        app.network = Some(manager);
        let tick = app.local_control_submission_tick();
        app.handle_ingame_mouse_click(pointer).test_value();
        assert_eq!(
            commands.take_submitted_player_commands(),
            vec![(
                tick,
                PlayerCommandControlData {
                    player: owner,
                    command: CommandId::MoveTo as i32,
                    x: click.x,
                    y: click.y,
                    target: 0,
                    target2: 0,
                    data: 0,
                    add_mode: 1,
                    by_client: 7,
                },
            )],
            "contained={contained}"
        );
    }
}

#[test]
fn mouse_jump_zone_overrides_overlapping_crew_selection() {
    let mut app = new_running_sandbox_app();
    let owner = app.local_owner;
    let cursor = app.engine.test_crew_cursor(owner);
    let cursor_snapshot = app.engine.test_object_snapshot(cursor);
    let click = Vector2::new(
        cursor_snapshot.position.x + 8,
        cursor_snapshot.position.y - 15,
    );
    let overlap = app.engine.spawn_test_object(
        SpawnConfig::new(cursor_snapshot.definition_id)
            .with_position(click)
            .with_owner(owner)
            .with_crew_member(true),
    );
    app.engine
        .apply_object_update(overlap, ObjectUpdate::new().with_position(click))
        .test_value();
    let mut crew = app.engine.test_player(owner).crew().to_vec();
    crew.push(overlap);
    app.engine.test_player_mut(owner).set_crew(crew);
    app.engine.select_crew(owner, [cursor]).test_value();
    app.engine.set_crew_cursor(owner, Some(cursor)).test_value();
    let pointer = sandbox_pointer_at_world(&mut app, owner, click);
    assert_eq!(
        app.ingame_mouse_select_target(owner, pointer.screen),
        Some(overlap),
        "the regression point must overlap another selectable crew member"
    );
    assert!(app.engine.mouse_jump_zone(owner, click));

    app.handle_ingame_mouse_click(pointer).test_value();

    assert_eq!(app.engine.crew_cursor(owner), Some(cursor));
    let commands = app
        .engine
        .test_object_snapshot(cursor)
        .command_stack
        .command_views();
    assert_eq!(commands.len(), 1);
    assert_eq!(commands[0].name, "Jump");
    assert_eq!(commands[0].tx, Some(click.x));
    assert_eq!(commands[0].ty, Some(click.y));
    assert_eq!(commands[0].target, None);
    assert!(
        app.engine
            .object_snapshot(overlap)
            .expect("overlapping crew survives")
            .command_stack
            .is_empty(),
        "the overlap must neither become selected nor receive the jump"
    );
}

#[test]
fn constructable_raw_item_id_drags_even_when_row_is_not_selectable() {
    let (mut app, owner, menu_point, _valid, _invalid, _world, _c4id) = construction_drag_fixture();
    let (cursor, mut menu) = app
        .engine
        .cursor_object_menu(owner)
        .map(|(cursor, menu)| (cursor, menu.clone()))
        .test_value();
    menu.items[0].selectable = false;
    install_test_cursor_menu(&mut app, cursor, menu);

    move_cursor(&mut app, menu_point, "hover disabled constructable row");
    app.test_left_button(ElementState::Pressed);
    assert!(matches!(
        app.construction_menu_drag.as_ref(),
        Some(ConstructionMenuDrag::Candidate {
            definition_id,
            ..
        }) if definition_id == "BLD1"
    ));
}

#[test]
fn construction_drop_requires_the_original_live_mouse_assignment() {
    for remove_assignment in [false, true] {
        let (mut app, owner, menu_point, valid_point, _invalid, _world, _c4id) =
            construction_drag_fixture();
        let (manager, _events, mut network_commands) =
            NetworkManager::test_stub_with_commands_for_client_id(7);
        app.network = Some(manager);
        begin_construction_drag(&mut app, menu_point, valid_point);

        if remove_assignment {
            app.local_controls.remove(owner);
            assert_eq!(app.local_controls.mouse_owner(), None);
        } else {
            app.mouse_control = false;
        }
        app.test_left_button(ElementState::Released);

        let (controls, commands, selections) = network_commands.take_submitted_player_inputs();
        assert!(controls.is_empty());
        assert!(commands.is_empty());
        assert!(selections.is_empty());
        assert!(app.construction_menu_drag.is_none());
    }
}

#[test]
fn gamepad_gui_control_uses_cpp_signed_integer_truthiness() {
    assert!(!parse_gamepad_gui_control("0"));
    assert!(parse_gamepad_gui_control("1"));
    assert!(parse_gamepad_gui_control("-1"));
    assert!(parse_gamepad_gui_control("2147483647"));
    assert!(!parse_gamepad_gui_control("true"));
    assert!(!parse_gamepad_gui_control("2147483648"));
}

#[test]
fn non_autostop_player_f1_release_falls_through_without_a_stuck_latch() {
    let mut app = new_running_sandbox_app();
    app.bindings
        .rebind(ControlBindingId::Left, VirtualKeyCode::F1);
    app.engine
        .test_player_mut(app.local_owner)
        .control
        .control_style = false;
    app.test_key(VirtualKeyCode::F1, ElementState::Pressed);
    assert!(app.pressed_engine_keys.contains(&VirtualKeyCode::F1));
    app.test_key(VirtualKeyCode::F1, ElementState::Released);
    assert!(!app.pressed_engine_keys.contains(&VirtualKeyCode::F1));
    assert!(!app.runtime_help_visible);

    let mut release_only = new_running_sandbox_app();
    release_only
        .bindings
        .rebind(ControlBindingId::Left, VirtualKeyCode::F1);
    release_only
        .engine
        .test_player_mut(release_only.local_owner)
        .control
        .control_style = false;
    assert!(release_only.show_startup_hint);
    release_only.test_key(VirtualKeyCode::F1, ElementState::Released);
    assert!(release_only.show_startup_hint);
    assert!(!release_only.runtime_help_visible);
}

#[test]
fn running_f4_only_stronger_escape_owns_keyboard_input() {
    let mut app = new_classic_running_sandbox_app();
    let (_events, _commands) = install_running_network_stub(&mut app, 0, 40, 4);
    app.control_clients
        .replace_snapshot([message_client(0, b"Host")]);
    app.test_key(VirtualKeyCode::F4, ElementState::Pressed);

    app.test_key(VirtualKeyCode::Enter, ElementState::Pressed);
    app.test_key(VirtualKeyCode::Enter, ElementState::Released);
    assert!(app.runtime_client_list.is_some());
    assert!(
        app.running_chat_text().is_some(),
        "the lower-priority fullscreen Return binding remains reachable"
    );
    app.close_running_chat().test_value();
    app.test_key(VirtualKeyCode::Escape, ElementState::Pressed);
    assert!(app.runtime_client_list.is_none());
    app.test_key(VirtualKeyCode::Escape, ElementState::Released);
}

/// C++ has no F11 shortcut: `C4KeyboardInput` maps it as an ordinary physical
/// key (C4KeyboardInput.cpp:185-197) and `C4Game::InitKeyboard` registers no
/// action for it (C4Game.cpp:3371-3448). Display mode changes only through the
/// startup Options combo (C4StartupOptionsDlg.cpp:1317-1322).
#[test]
fn f11_reaches_classic_keyconfig_without_toggling_display_mode() {
    // An unbound F11 is inert in the startup screens.
    let mut app = new_menu_app(320, 200);
    app.set_display_mode(DisplayMode::Window);
    let view = app.startup_view;
    app.test_key(VirtualKeyCode::F11, ElementState::Pressed);
    app.test_key(VirtualKeyCode::F11, ElementState::Released);
    assert!(!app.display_flags.is_fullscreen);
    assert_eq!(app.startup_view, view);

    // ... and while running.
    let mut app = new_classic_running_sandbox_app();
    app.set_display_mode(DisplayMode::Window);
    app.test_key(VirtualKeyCode::F11, ElementState::Pressed);
    app.test_key(VirtualKeyCode::F11, ElementState::Released);
    assert!(!app.display_flags.is_fullscreen);
    assert!(app.pending_screenshots.is_empty());

    // A KeyConfig action bound to F11 reaches ordinary classic dispatch.
    app.runtime_key_config_cache = OnceLock::new();
    app.runtime_key_config_cache
        .set(Ok(parse_runtime_key_config(
            b"[Keys]\nToggleShowHelp=F11\n",
        )
        .expect("parse an F11 scoreboard binding")))
        .test_value();
    assert!(!app.runtime_help_visible);
    app.test_key(VirtualKeyCode::F11, ElementState::Pressed);
    assert!(
        app.runtime_help_visible,
        "a KeyConfig action bound to F11 must reach classic dispatch"
    );
    assert!(
        !app.display_flags.is_fullscreen,
        "dispatching the bound action must not change the display mode"
    );
}

/// On the SDL backend `C4KeyCodeEx::String2KeyCode` delegates every physical
/// key name to `SDL_GetScancodeFromName` and accepts any non-UNKNOWN result
/// (C4KeyboardInput.cpp:315-330), so a migrated `KeyConfig.txt` may legitimately
/// name media, browser, modifier or international keys. Only names SDL itself
/// rejects may disable a binding.
#[test]
fn keyconfig_accepts_extended_sdl_scancode_names() {
    let config = parse_runtime_key_config(
        b"[Keys]\n          MusicToggle=Mute\n          SoundToggle=VolumeDown\n          ToggleChat=NonUSBackslash\n          Screenshot=AC Home\n          ScreenshotEx=Left GUI\n          ScoreboardToggle=International3\n          ToggleShowHelp=Paste\n          NetClientListDlgToggle=Calculator\n          MsgBoardScrollUp=AudioNext\n          MsgBoardScrollDown=NotAnSdlScancodeName\n",
    ).test_value();

    let physical = |name: &str| {
        config
            .override_for(name)
            .and_then(|chords| chords.first())
            .map(|chord| chord.physical)
            .unwrap_or_else(|| panic!("{name} has no override"))
    };
    // Media and volume keys.
    assert_eq!(
        physical("MusicToggle"),
        RuntimePhysicalKey::Keyboard(VirtualKeyCode::AudioVolumeMute)
    );
    assert_eq!(
        physical("SoundToggle"),
        RuntimePhysicalKey::Keyboard(VirtualKeyCode::AudioVolumeDown)
    );
    assert_eq!(
        physical("MsgBoardScrollUp"),
        RuntimePhysicalKey::Keyboard(VirtualKeyCode::MediaTrackNext)
    );
    // The extra ISO key next to left Shift, which winit reports as OEM102.
    assert_eq!(
        physical("ToggleChat"),
        RuntimePhysicalKey::Keyboard(VirtualKeyCode::IntlBackslash)
    );
    // Application-control, modifier, international and editing keys.
    assert_eq!(
        physical("Screenshot"),
        RuntimePhysicalKey::Keyboard(VirtualKeyCode::BrowserHome)
    );
    assert_eq!(
        physical("ScreenshotEx"),
        RuntimePhysicalKey::Keyboard(VirtualKeyCode::SuperLeft)
    );
    assert_eq!(
        physical("ScoreboardToggle"),
        RuntimePhysicalKey::Keyboard(VirtualKeyCode::IntlYen)
    );
    assert_eq!(
        physical("ToggleShowHelp"),
        RuntimePhysicalKey::Keyboard(VirtualKeyCode::Paste)
    );
    assert_eq!(
        physical("NetClientListDlgToggle"),
        RuntimePhysicalKey::Keyboard(VirtualKeyCode::LaunchApp2)
    );
    // A name SDL does not know keeps the disabled outcome.
    assert_eq!(physical("MsgBoardScrollDown"), RuntimePhysicalKey::Disabled);

    // A bound extended key dispatches like any other physical key.
    let mut app = new_classic_running_sandbox_app();
    app.runtime_key_config_cache = OnceLock::new();
    app.runtime_key_config_cache
        .set(Ok(parse_runtime_key_config(
            b"[Keys]\nToggleShowHelp=Mute\n",
        )
        .expect("parse the Mute help binding")))
        .test_value();
    assert!(!app.runtime_help_visible);
    app.test_key(VirtualKeyCode::AudioVolumeMute, ElementState::Pressed);
    assert!(
        app.runtime_help_visible,
        "a KeyConfig action bound to an extended SDL scancode must execute"
    );
}

fn platform_ime_test_app() -> GameApp {
    let mut app = test_game_app(320, 200, AudioOptions::default(), None).test_value();
    install_classic_test_assets(&mut app);
    app
}

fn open_platform_ime_test_context(app: &mut GameApp) {
    app.open_context_menu_at(Vec::new(), GuiPoint::new(10.0, 10.0))
        .test_value();
    assert!(app.context_menu.is_some());
}

#[test]
fn platform_ime_follows_the_shell_mode_and_console_ownership() {
    let mut app = platform_ime_test_app();
    app.console_mode = false;

    app.mode = AppMode::Loading;
    assert!(!app.platform_ime_allowed());
    app.mode = AppMode::Menu;
    assert!(app.platform_ime_allowed());
    app.window_active = false;
    assert!(!app.platform_ime_allowed());
    app.window_active = true;

    let (_startup_sender, startup_receiver) = mpsc::channel();
    app.startup_network_connection = Some(StartupNetworkConnection::new(
        startup_receiver,
        None,
        StartupNetworkPurpose::StagedHost,
    ));
    assert!(!app.platform_ime_allowed());
    app.startup_network_connection = None;

    app.push_message_dialog(
        clonk_frontend::message_dialog::MessageDialogState::regular_ok(
            "Notice",
            "Clonk",
            clonk_frontend::message_dialog::MessageDialogIcon::NOTIFY,
        ),
        MessageDialogContinuation::None,
    )
    .test_value();
    assert!(!app.platform_ime_allowed());
    app.finish_message_dialog(clonk_frontend::message_dialog::MessageDialogResult::Ok)
        .test_value();

    app.mode = AppMode::Running;
    assert!(!app.platform_ime_allowed());

    app.console_mode = true;
    assert!(app.platform_ime_allowed());
}

#[test]
fn platform_ime_tracks_game_option_and_running_chat_keyboard_focus() {
    let mut app = platform_ime_test_app();
    app.mode = AppMode::Running;
    app.game_option_input_dialog = Some(PendingGameOptionInputDialog {
        purpose: PendingInputDialogPurpose::GameOption(GameOptionInputKind::Password),
        controller: InputDialogController::new(
            "Password",
            "Options",
            InputDialogIcon::LOCKED_FRONTAL,
        ),
    });
    assert!(app.platform_ime_allowed());
    open_platform_ime_test_context(&mut app);
    assert!(!app.platform_ime_allowed());
    app.close_context_menu_silently();
    app.game_option_input_dialog = None;

    app.start_running_chat(RunningChatMode::All);
    app.set_running_chat_active(false);
    assert!(!app.platform_ime_allowed());
    app.set_running_chat_active(true);
    assert!(app.platform_ime_allowed());
    open_platform_ime_test_context(&mut app);
    assert!(!app.platform_ime_allowed());
}

#[test]
fn platform_ime_tracks_league_signup_and_its_context_menu() {
    let mut app = platform_ime_test_app();
    app.mode = AppMode::Running;
    app.league_signup_dialog = Some(PendingLeagueSignupDialog {
        controller: clonk_frontend::league_signup::LeagueSignupController::new(
            clonk_frontend::league_signup::LeagueSignupConfig::new(
                "Player",
                "league.example",
                clonk_frontend::league_signup::LeagueSignupMode::Login,
            ),
            clonk_frontend::league_signup::LeagueSignupStrings::default(),
        ),
        auth: clonk_network::LeagueAuthRequestHead::default(),
        continuation: LeaguePlayerAuthContinuation::InitialClient {
            request: clonk_network::PlayerInfoUpdateRequest {
                client_id: 7,
                flags: 0,
                players: Vec::new(),
            },
            index: 0,
            server_name: "league.example".to_string(),
        },
    });
    assert!(app.platform_ime_allowed());
    open_platform_ime_test_context(&mut app);
    assert!(!app.platform_ime_allowed());
}

#[test]
fn platform_ime_tracks_external_irc_z_order_and_menu_mode() {
    let mut app = platform_ime_test_app();
    app.mode = AppMode::Running;
    app.external_irc_dialog_visible = true;
    app.scoreboard_dialog = Some(ScoreboardPresentationRequest {
        rows: 0,
        columns: 0,
        show_count: 0,
        layout_revision: 0,
        title_widget_present: false,
        scoreboard: clonk_engine::ScoreboardState::default(),
    });
    app.runtime_default_dialog_order = vec![
        RuntimeDefaultDialog::ExternalIrc,
        RuntimeDefaultDialog::Scoreboard,
    ];
    assert!(!app.platform_ime_allowed());

    app.show_or_raise_runtime_default_dialog(RuntimeDefaultDialog::ExternalIrc);
    assert!(app.platform_ime_allowed());
    open_platform_ime_test_context(&mut app);
    assert!(!app.platform_ime_allowed());

    app.mode = AppMode::Menu;
    assert!(!app.platform_ime_allowed());
    app.close_context_menu_silently();
    assert!(app.platform_ime_allowed());
}
