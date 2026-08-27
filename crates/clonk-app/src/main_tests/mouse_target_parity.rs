// Spliced into `mod tests` (src/main_tests.rs) via include!: a bare item
// sequence, not a child module, so xtask's exact parity filter can find it.

#[derive(Clone, Debug, serde::Deserialize)]
struct MouseTargetEventGolden {
    case: String,
    action: i32,
    mutation_kind: i32,
    mutation_object: i32,
    mutation_x: i32,
    mutation_y: i32,
    mutation_ocf: u32,
    tick5: i32,
    fog: i32,
    execute_before_action: i32,
    acquire_mode: i32,
    cursor_mode: i32,
    region: i32,
    x: i32,
    y: i32,
    put_vehicle: i32,
    control: i32,
    shift: i32,
    down_region_right_com: i32,
    player: i32,
    viewport_view_x: i32,
    viewport_view_y: i32,
    viewport_width: i32,
    viewport_height: i32,
    cached_target_before: i32,
    crew_cursor: i32,
    crew: Vec<i32>,
    selection_before: Vec<i32>,
    objects: Vec<MouseTargetObjectGolden>,
    target_after: i32,
    refill_ocf_out: u32,
    packets: Vec<MouseTargetPacketGolden>,
    selection_after: Vec<i32>,
}

#[derive(Clone, Debug, serde::Deserialize)]
struct MouseTargetObjectGolden {
    id: i32,
    order: i32,
    x: i32,
    y: i32,
    shape_x: i32,
    shape_y: i32,
    wdt: i32,
    hgt: i32,
    status: i32,
    category: i32,
    ocf: u32,
    owner: i32,
    wwng: i32,
}

#[derive(Clone, Debug, PartialEq, Eq, serde::Deserialize)]
struct MouseTargetPacketGolden {
    kind: i32,
    player: i32,
    command: i32,
    x: i32,
    y: i32,
    target: i32,
    target2: i32,
    data: i32,
    add_mode: i32,
}

fn mouse_target_event_rows() -> Vec<MouseTargetEventGolden> {
    let golden: serde_json::Value =
        serde_json::from_str(include_str!("../../../../parity/golden/parity_golden.json"))
            .test_value();
    serde_json::from_value(golden["mouse_target_events"].clone()).test_value()
}

fn mouse_target_cursor_kind(cursor: i32) -> IngameMouseCursorKind {
    match cursor {
        0 => IngameMouseCursorKind::Region,
        7 => IngameMouseCursorKind::Select,
        22 => IngameMouseCursorKind::Put,
        25 => IngameMouseCursorKind::VehiclePut,
        34 => IngameMouseCursorKind::Nothing,
        other => panic!("mouse-target golden uses unsupported cursor {other}"),
    }
}

fn mouse_target_object_status(status: i32) -> ObjectStatus {
    match status {
        0 => ObjectStatus::Deleted,
        1 => ObjectStatus::Normal,
        2 => ObjectStatus::Inactive,
        other => panic!("mouse-target golden uses unsupported object status {other}"),
    }
}

fn mouse_target_definition_id(object: &MouseTargetObjectGolden) -> String {
    if object.wwng != 0 {
        "WWNG".to_string()
    } else {
        format!("P{:03}", object.id.rem_euclid(1_000))
    }
}

fn mouse_target_drag(
    pointer: ViewportPointer,
    source: ViewportPointer,
    target: ObjectId,
) -> IngameButtonMouseState {
    let mut drag = IngameButtonMouseState::new(source, Some(target), false);
    drag.motion.last = pointer;
    drag.motion.moved = true;
    drag.motion.world_drag_started = true;
    drag
}

fn mouse_target_position_pointer(owner: i32, position: Vector2) -> ViewportPointer {
    ViewportPointer {
        owner,
        world: FloatVector2::new(position.x as f32, position.y as f32),
        screen: GuiPoint::new(-100.0, -100.0),
    }
}

fn mouse_target_app(row: &MouseTargetEventGolden) -> GameApp {
    let width = u32::try_from(row.viewport_width).test_value();
    let height = u32::try_from(row.viewport_height).test_value();
    let audio_options = AudioOptions {
        sound_enabled: false,
        music_enabled: false,
        menu_music_enabled: false,
        menu_sound_enabled: false,
        ..AudioOptions::default()
    };
    let mut app = GameApp::new_with_frontend_scenarios(
        width,
        height,
        audio_options,
        None,
        RuntimeConfig {
            player_owner: row.player,
            player_name: "Player".to_string(),
            network: None,
            record_enabled: false,
        },
        Some(Vec::new()),
    )
    .test_value();
    install_synthetic_classic_test_assets(&mut app);
    app.start_sandbox_scenario_with_definitions(
        FrontendScenario::fallback(),
        SandboxDefinitionLoad::None,
    )
    .test_value();
    wait_for_running(&mut app);
    app.live_input.ingame_mouse_init_centered = true;
    if let Some(cursor) = app.engine.crew_cursor(row.player) {
        let mut update = ObjectUpdate::new();
        update.plr_view_range = Some(500);
        app.engine.apply_object_update(cursor, update).test_value();
        app.snapshot = app.engine.snapshot();
    }
    hold_message_board_for_frame_comparison(&mut app);
    install_synthetic_sandbox_crew_definition(&mut app);
    app
}

fn mouse_target_render_app(app: &mut GameApp, row: &MouseTargetEventGolden) {
    let width = usize::try_from(row.viewport_width).test_value();
    let height = usize::try_from(row.viewport_height).test_value();
    let pixel_count = width
        .checked_mul(height)
        .and_then(|pixels| pixels.checked_mul(4))
        .test_value();
    app.snapshot = app.engine.snapshot();
    let mut frame = vec![0_u8; pixel_count];
    app.render(&mut frame).test_value();
}

fn mouse_target_render_pointer(app: &mut GameApp, row: &MouseTargetEventGolden) -> ViewportPointer {
    app.display_flags.scroll_smooth = 1;
    app.graphics.set_scroll_smooth(1);
    mouse_target_render_app(app, row);
    let current = app
        .graphics
        .active_viewport_projections()
        .into_iter()
        .find(|viewport| viewport.owner == row.player)
        .test_value();
    let scroll_range = (row.viewport_width / 10).min(row.viewport_height / 10);
    let center_x = row.viewport_view_x
        + row.viewport_width / 2
        + if current.target_x < row.viewport_view_x {
            scroll_range
        } else if current.target_x > row.viewport_view_x {
            -scroll_range
        } else {
            0
        };
    let center_y = row.viewport_view_y
        + row.viewport_height / 2
        + if current.target_y < row.viewport_view_y {
            scroll_range
        } else if current.target_y > row.viewport_view_y {
            -scroll_range
        } else {
            0
        };
    app.engine.test_player_mut(row.player).set_viewport(
        0,
        clonk_engine::PlayerViewport::new(Vector2::new(center_x, center_y)),
    );
    app.engine
        .test_player_mut(row.player)
        .set_view_center(Vector2::new(center_x, center_y));
    mouse_target_render_app(app, row);

    let viewport = app
        .graphics
        .active_viewport_projections()
        .into_iter()
        .find(|viewport| viewport.owner == row.player)
        .test_value();
    main_assert_eq!((viewport.target_x, viewport.target_y) => (row.viewport_view_x, row.viewport_view_y));
    main_assert_eq!((viewport.logical_width, viewport.logical_height) => (row.viewport_width, row.viewport_height));
    let viewport_point = Vector2::new(
        row.x.saturating_sub(row.viewport_view_x),
        row.y.saturating_sub(row.viewport_view_y),
    );
    let screen = GuiPoint::new(
        (viewport.rect.x + viewport_point.x) as f32,
        (viewport.rect.y + viewport_point.y) as f32,
    );
    let projected = app.graphics.viewport_point_at(screen).test_value();
    main_assert_eq!(projected.owner => row.player);
    main_assert_eq!(ingame_pointer_world_pixel(projected) => Vector2::new(row.x, row.y));
    ViewportPointer {
        owner: row.player,
        world: FloatVector2::new(row.x as f32, row.y as f32),
        screen,
    }
}

fn mouse_target_fixture(
    row: &MouseTargetEventGolden,
) -> (GameApp, HashMap<i32, ObjectId>, ViewportPointer) {
    let mut app = mouse_target_app(row);
    main_assert_eq!(app.players.local_owner => row.player, "{} event player", row.case);
    let owner = row.player;
    let landscape_width = row
        .viewport_width
        .saturating_add(row.viewport_view_x.max(0))
        .max(row.x.saturating_add(1))
        .max(1);
    let landscape_height = row
        .viewport_height
        .saturating_add(row.viewport_view_y.max(0))
        .max(row.y.saturating_add(1))
        .max(1);
    let mut landscape = Landscape::flat(
        u32::try_from(landscape_width).test_value(),
        landscape_height,
    );
    landscape.set_world_height(landscape_height);
    app.engine.set_landscape(landscape);

    let existing_cursor = app.engine.test_crew_cursor(owner);
    let mut ids = HashMap::from([(row.crew_cursor, existing_cursor)]);
    let crew_ids = row
        .crew
        .iter()
        .copied()
        .collect::<std::collections::HashSet<_>>();
    let selection_ids = row
        .selection_before
        .iter()
        .copied()
        .collect::<std::collections::HashSet<_>>();
    let mut registered = std::collections::HashSet::new();

    let mut spawn_rows = row
        .objects
        .iter()
        .filter(|object| !crew_ids.contains(&object.id))
        .collect::<Vec<_>>();
    // GraphicsSystem reconstructs C++'s front-to-back master scan by walking
    // the Rust snapshot in reverse. Spawn descending oracle order so its
    // smallest `order` remains the first FindVisObject candidate.
    spawn_rows.sort_by_key(|object| std::cmp::Reverse(object.order));
    for object in spawn_rows {
        let definition_id = mouse_target_definition_id(object);
        if registered.insert(definition_id.clone()) {
            let mut definition = test_definition(&definition_id, &definition_id, "#strict\n");
            definition.set_category(object.category);
            definition.set_shape_rect(Some(clonk_engine::DefinitionRect::new(
                object.shape_x,
                object.shape_y,
                object.wdt,
                object.hgt,
            )));
            if object.ocf & clonk_engine::ocf::GRAB != 0
                || selection_ids.contains(&object.id) && row.put_vehicle != 0
            {
                definition.set_grab(1);
            }
            if object.ocf & clonk_engine::ocf::CONTAINER != 0 {
                definition.set_grab_put_get(clonk_engine::GRAB_PUT_GET_PUT);
            }
            if selection_ids.contains(&object.id) && row.put_vehicle == 0 {
                definition.set_collectible(true);
            }
            app.engine.register_test_definition(definition);
            app.object_sprites.insert(
                definition_id.clone(),
                clonk_frontend::DefinitionSprite {
                    image: clonk_gui::ImageData::new(1, 1, vec![255, 255, 255, 255]),
                    actions: HashMap::new(),
                    color_mask: None,
                    graphics_scale: 1.0,
                    shape: Some(clonk_engine::DefinitionRect::new(
                        object.shape_x,
                        object.shape_y,
                        object.wdt,
                        object.hgt,
                    )),
                    fire_top: 0,
                    rotateable: 0,
                    line: 0,
                    stretch_growth: false,
                    top_face: None,
                    picture: None,
                },
            );
        }
        let actual = app.engine.spawn_test_object(
            SpawnConfig::new(&definition_id)
                .with_position(Vector2::new(object.x, object.y))
                .with_owner(object.owner),
        );
        let mut update = ObjectUpdate::new();
        update.position = Some(Vector2::new(object.x, object.y));
        update.status = Some(mouse_target_object_status(object.status));
        update.category = Some(object.category);
        update.ocf_override = Some(if selection_ids.contains(&object.id) {
            if row.put_vehicle != 0 {
                object.ocf | clonk_engine::ocf::GRAB
            } else {
                object.ocf | clonk_engine::ocf::CARRYABLE
            }
        } else {
            object.ocf
        });
        app.engine.apply_object_update(actual, update).test_value();
        ids.insert(object.id, actual);
    }

    for crew_id in &row.crew {
        let object = row
            .objects
            .iter()
            .find(|object| object.id == *crew_id)
            .test_value();
        let actual = if *crew_id == row.crew_cursor {
            existing_cursor
        } else {
            let cursor = app.engine.test_object_snapshot(existing_cursor);
            app.engine.spawn_test_object(
                SpawnConfig::new(cursor.definition_id)
                    .with_position(Vector2::new(object.x, object.y))
                    .with_owner(owner)
                    .with_crew_member(true),
            )
        };
        app.engine
            .apply_object_update(
                actual,
                ObjectUpdate {
                    position: Some(Vector2::new(object.x, object.y)),
                    layer: Some(None),
                    status: Some(mouse_target_object_status(object.status)),
                    category: Some(object.category),
                    crew_member: Some(true),
                    ocf_override: Some(object.ocf),
                    ..ObjectUpdate::default()
                },
            )
            .test_value();
        ids.insert(*crew_id, actual);
    }
    let crew = row
        .crew
        .iter()
        .map(|id| *ids.get(id).test_value())
        .collect::<Vec<_>>();
    app.engine.test_player_mut(owner).set_crew(crew);
    app.engine
        .set_crew_cursor(owner, ids.get(&row.crew_cursor).copied())
        .test_value();
    app.update_sprite_cache();

    if row.fog != 0 {
        let _ = app.engine.test_player_mut(owner).set_fog_of_war(true);
        let mut view = ObjectUpdate::new();
        view.plr_view_range = Some(0);
        app.engine
            .apply_object_update(existing_cursor, view)
            .test_value();
    }

    app.snapshot = app.engine.snapshot();
    let pointer = mouse_target_render_pointer(&mut app, row);
    (app, ids, pointer)
}

fn mouse_target_oracle_id(ids: &HashMap<i32, ObjectId>, actual: Option<ObjectId>) -> i32 {
    actual.map_or(0, |actual| {
        ids.iter()
            .find_map(|(oracle, candidate)| (*candidate == actual).then_some(*oracle))
            .unwrap_or_else(|| panic!("unmapped mouse-target object {actual:?}"))
    })
}

fn mouse_target_oracle_ids(ids: &HashMap<i32, ObjectId>, actual: &[ObjectId]) -> Vec<i32> {
    actual
        .iter()
        .map(|object| mouse_target_oracle_id(ids, Some(*object)))
        .collect()
}

fn mouse_target_acquire(
    app: &mut GameApp,
    ids: &HashMap<i32, ObjectId>,
    row: &MouseTargetEventGolden,
    mut pointer: ViewportPointer,
) -> Option<IngameButtonMouseState> {
    let owner = row.player;
    if row.region != 0 {
        main_assert_eq!(row.down_region_right_com => 0, "{} region RightCom", row.case);
        pointer.screen =
            viewport_button_point(app, owner, clonk_frontend::hud::ViewportButton::Help);
        main_assert!(
            app.ingame_viewport_region(owner, pointer.screen).is_some(),
            "{} region fixture is reachable",
            row.case
        );
    }
    let acquired = match row.acquire_mode {
        1 => {
            app.update_ingame_pointer(pointer.screen).test_value();
            app.retained_ingame_mouse_target()
        }
        2 | 3 => app.graphics.object_at_point_with_ocf(
            &app.snapshot,
            owner,
            pointer.screen,
            clonk_engine::ocf::CONTAINER,
        ),
        other => panic!("mouse-target golden uses unsupported acquire mode {other}"),
    };
    main_assert_eq!(mouse_target_oracle_id(ids, acquired) => row.cached_target_before, "{} cached target", row.case);
    app.live_input.ingame_mouse_target = acquired;
    app.live_input.ingame_mouse_caption.cursor = mouse_target_cursor_kind(row.cursor_mode);
    app.live_input.ingame_pointer = Some(pointer);
    app.ingame_dragged_objects = row
        .selection_before
        .iter()
        .map(|id| *ids.get(id).test_value())
        .collect();

    if row.acquire_mode == 1 {
        return None;
    }
    let first = *app.ingame_dragged_objects.first().test_value();
    let source_position = app.engine.test_object_snapshot(first).position;
    let drag = mouse_target_drag(
        pointer,
        mouse_target_position_pointer(owner, source_position),
        first,
    );
    app.mouse_state = Some(drag);
    Some(drag)
}

fn mouse_target_mutate(
    app: &mut GameApp,
    ids: &HashMap<i32, ObjectId>,
    row: &MouseTargetEventGolden,
) {
    let update = match row.mutation_kind {
        0 => return,
        1 => ObjectUpdate::new().with_position(Vector2::new(row.mutation_x, row.mutation_y)),
        2 => ObjectUpdate {
            ocf_override: Some(row.mutation_ocf),
            ..ObjectUpdate::default()
        },
        3 => ObjectUpdate {
            status: Some(ObjectStatus::Deleted),
            ..ObjectUpdate::default()
        },
        other => panic!("mouse-target golden uses unsupported mutation {other}"),
    };
    let target = *ids.get(&row.mutation_object).test_value();
    app.engine.apply_object_update(target, update).test_value();
    app.snapshot = app.engine.snapshot();
}

fn mouse_target_packets(
    commands: &mut network::TestNetworkCommands,
    ids: &HashMap<i32, ObjectId>,
) -> Vec<MouseTargetPacketGolden> {
    let (controls, player_commands, selections) = commands.take_submitted_mouse_controls();
    let mut packets = Vec::new();
    packets.extend(controls.into_iter().map(|(owner, event, _)| match event {
        ControlEvent::RawPlayerControl { command, data } => MouseTargetPacketGolden {
            kind: 3,
            player: owner,
            command: i32::from(command),
            x: 0,
            y: 0,
            target: 0,
            target2: 0,
            data,
            add_mode: 0,
        },
        other => panic!("unexpected mouse-target direct control {other:?}"),
    }));
    // The network test probe groups synchronized packet variants. In every
    // oracle row that mixes them, C++ emits all selects before its command.
    packets.extend(selections.into_iter().map(|(_, selection)| {
        MouseTargetPacketGolden {
            kind: 1,
            player: selection.player,
            command: 0,
            x: 0,
            y: 0,
            target: selection
                .objects
                .first()
                .copied()
                .map(|object| ObjectId::new(object as u64))
                .map_or(0, |object| mouse_target_oracle_id(ids, Some(object))),
            target2: 0,
            data: selection.objects.len() as i32,
            add_mode: 0,
        }
    }));
    packets.extend(player_commands.into_iter().map(|(_, command)| {
        let object_number = |number: i32| {
            if number != 0 {
                mouse_target_oracle_id(ids, Some(ObjectId::new(number as u64)))
            } else {
                0
            }
        };
        MouseTargetPacketGolden {
            kind: 2,
            player: command.player,
            command: command.command,
            x: command.x,
            y: command.y,
            target: object_number(command.target),
            target2: object_number(command.target2),
            data: command.data,
            add_mode: command.add_mode,
        }
    }));
    packets
}

fn mouse_target_run_event(row: &MouseTargetEventGolden) {
    let (mut app, ids, pointer) = mouse_target_fixture(row);
    let drag = mouse_target_acquire(&mut app, &ids, row, pointer);
    mouse_target_mutate(&mut app, &ids, row);

    let retained_before_action = app.retained_ingame_mouse_target();
    let context_refill = (1..=3).contains(&row.action)
        && row.region == 0
        && retained_before_action.is_none_or(|target| {
            app.snapshot
                .object(target)
                .is_some_and(|object| object.definition_id == "WWNG")
        });
    let modifiers = if row.control != 0 {
        ModifiersState::CONTROL
    } else {
        ModifiersState::empty()
    } | if row.shift != 0 {
        ModifiersState::SHIFT
    } else {
        ModifiersState::empty()
    };
    app.test_modifiers(modifiers);
    if row.execute_before_action != 0 {
        main_assert_eq!(row.tick5 => 0, "{} executes on the Tick5 refresh frame", row.case);
        app.update_ingame_pointer(pointer.screen).test_value();
    }
    let mut commands = install_mouse_network_capture(&mut app);
    match row.action {
        1..=3 => app.test_right_button(ElementState::Released),
        4 => {
            let finished = app
                .finish_ingame_moved_drag(
                    drag.unwrap_or_else(|| panic!("{} has no acquired drag", row.case)),
                    false,
                )
                .unwrap_or_else(|error| panic!("{} drag failed: {error}", row.case));
            main_assert!(finished, "{} drag source was not classified", row.case);
        }
        other => panic!("mouse-target golden uses unsupported action {other}"),
    }

    let actual_target = mouse_target_oracle_id(&ids, app.retained_ingame_mouse_target());
    main_assert_eq!(actual_target => row.target_after, "{} target_after", row.case);
    let actual_refill_ocf = if row.execute_before_action != 0 || context_refill {
        app.retained_ingame_mouse_target()
            .and_then(|target| app.snapshot.object(target))
            .map_or(0, |target| target.ocf)
    } else {
        0
    };
    main_assert_eq!(actual_refill_ocf => row.refill_ocf_out, "{} refill_ocf_out", row.case);
    main_assert_eq!(mouse_target_packets(&mut commands, &ids) => row.packets, "{} packets", row.case);
    main_assert_eq!(mouse_target_oracle_ids(&ids, &app.ingame_dragged_objects) => row.selection_after, "{} selection_after", row.case);
}

#[test]
fn parity_differential_matches_cpp_golden() {
    // C4Game::FindVisObject/GetTargetObject and C4MouseControl's retained
    // Move/Tick5, RightUp and ButtonUpDragMoving chain. The golden compiles
    // the pinned C++ bodies at C4Game.cpp:1426-1498 and
    // C4MouseControl.cpp:158-163,742-769,1171-1201,1230-1259,1318-1325.
    let rows = mouse_target_event_rows();
    main_assert_eq!(rows.len() => 13, "mouse-target event matrix remains complete");
    for row in &rows {
        mouse_target_run_event(row);
    }
}
