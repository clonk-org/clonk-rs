// Spliced into `mod tests` (src/main_tests.rs) via include!: a bare item
// sequence, not a child module, so test ids stay `tests::<fn>`.

macro_rules! menus1_fixture {
    (message_geometry: $x:expr, $y:expr, $width:expr $(,)?) => {
        GlobalMessageViewportGeometry {
            x: $x,
            y: $y,
            width: $width,
        }
    };
    (definition_picture: $width:expr, $height:expr $(,)?) => {
        clonk_engine::DefinitionPicture {
            x: 0,
            y: 0,
            width: $width,
            height: $height,
        }
    };
    (sprite_image: $width:expr, $height:expr, $pixels:expr, $color_mask:expr $(,)?) => {
        clonk_engine::DefinitionSpriteImage {
            width: $width,
            height: $height,
            pixels: $pixels,
            color_mask: $color_mask,
        }
    };
    (menu_item: $caption:expr, $count:expr, $item_id:expr, $symbol:expr, $image:expr, $presentation_definition_id:expr, $picture_snapshot:expr, $selectable:expr $(,)?) => {
        clonk_engine::ObjectMenuItem {
            caption: $caption,
            info_caption: String::new(),
            command: String::new(),
            command2: String::new(),
            count: $count,
            item_id: $item_id,
            symbol: $symbol,
            image: $image,
            presentation_definition_id: $presentation_definition_id,
            picture_snapshot: $picture_snapshot,
            picture_object: None,
            components: Vec::new(),
            selectable: $selectable,
            value: None,
            text_display_progress: -1,
        }
    };
    (menu_picture: $definition_id:expr, $symbol_size:expr, $graphics_overlays:expr, $color:expr, $color_modulation:expr $(,)?) => {
        clonk_engine::ObjectMenuPictureSnapshot {
            definition_id: $definition_id,
            symbol_size: $symbol_size,
            base_graphics: None,
            graphics_overlays: $graphics_overlays,
            blit_mode: 0,
            color: $color,
            color_modulation: $color_modulation,
            picture_rect: clonk_engine::DefinitionRect::default(),
            rank: None,
        }
    };
    (player_info_id_name: $id:expr, $name:expr $(,)?) => {
        clonk_engine::ControlPlayerInfoEntry {
            id: $id,
            name: $name,
            ..Default::default()
        }
    };
    (roster_context: $row:expr $(,)?) => {
        ClassicLobbyAction::RosterContextRequested {
            row: $row,
            position: GuiPoint::new(200.0, 150.0),
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
    (player_selection: $name:expr, $color_dw:expr $(,)?) => {
        clonk_frontend::startup_plrsel::PlrSelPlayer {
            name: $name,
            activated: false,
            big_icon: None,
            portrait: None,
            color_dw: $color_dw,
            score: 0,
            rounds: 0,
            rounds_won: 0,
            rounds_lost: 0,
            total_playing_time: 0,
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
}

#[test]
fn eliminated_player_mouse_menu_keeps_new_player_reentry_surface() {
    // C4Viewport keeps the eliminated notice but continues to draw the local
    // PlayerMenu control (src/C4Viewport.cpp:836-880,965-976,1511-1525).
    // C4Game::LocalPlayerControl handles that command before its eliminated
    // player early return (src/C4Game.cpp:3595-3622), and ActivateMain offers
    // NewPlayer without an Eliminated gate when capacity allows
    // (src/C4MainMenu.cpp:643-687).
    let mut app = new_classic_running_sandbox_app();
    let owner = app.local_owner;
    app.snapshot = app.engine.snapshot();
    app.snapshot
        .players
        .iter_mut()
        .find(|player| player.id == owner)
        .test_value()
        .status = PlayerStatus::Eliminated;
    let before_players = app.engine.snapshot().players;

    app.dispatch_control_event_for_local_player(
        owner,
        ControlEvent::Command {
            command: ControlCommand::PlayerMenu,
            kind: CommandKind::Press,
        },
    )
    .test_value();

    let menu = app.ingame_menu.get(owner).test_value();
    main_assert!(menu.items().iter().any(|item| item.action == MenuAction::ActivateNewPlayer));
    main_assert!(app.ingame_menu_has_visible_surface(owner), "the eliminated viewport still exposes the C++ PlayerMenu re-entry surface");
    main_assert_eq!(app.engine.snapshot().players => before_players, "opening the local PlayerMenu does not mutate synchronized player state");
}

#[test]
fn help_suppresses_open_ingame_menu_and_right_up_exits() {
    let mut app = new_classic_running_sandbox_app();
    let owner = app.local_owner;
    app.activate_ingame_main_menu_for_player(owner).test_value();
    render_mouse_test_app(&mut app);
    let (width, height) = {
        let surface = app.graphics.surface();
        (surface.width(), surface.height())
    };
    let close = (0..height)
        .flat_map(|y| (0..width).map(move |x| (x, y)))
        .find_map(|(x, y)| {
            let point = GuiPoint::new(x as f32 + 0.5, y as f32 + 0.5);
            (app.ingame_menu_pointer_target(point) == Some((owner, IngameMenuPointerTarget::Close)))
                .then_some(point)
        })
        .test_value();
    let mut commands = install_mouse_network_capture(&mut app);

    app.ingame_mouse_help = true;
    physical_left_click_with_modifiers(
        &mut app,
        close,
        ModifiersState::empty(),
        ModifiersState::empty(),
    );
    main_assert!(app.ingame_mouse_help);
    main_assert!(app.ingame_menu_belongs_to(owner), "Help suppresses already-open player-menu controls");
    main_assert_eq!(commands.take_submitted_mouse_controls() => (Vec::new(), Vec::new(), Vec::new()));

    app.test_right_button(ElementState::Pressed);
    main_assert!(app.ingame_mouse_help);
    app.test_right_button(ElementState::Released);
    main_assert!(!app.ingame_mouse_help);
    main_assert!(app.ingame_menu_belongs_to(owner));
    main_assert_eq!(commands.take_submitted_mouse_controls() => (Vec::new(), Vec::new(), Vec::new()), "Help menu interception queues no controls");
}

#[test]
fn help_right_up_exits_without_context_or_crew_cycle() {
    let mut app = new_running_sandbox_app();
    let owner = app.local_owner;
    let (_target, point) = install_mouse_help_target(&mut app, "HLP3", "Right target", None);
    let (empty, _) = mouse_test_empty_point(&mut app, owner, point, None);
    let cursor = app.engine.crew_cursor(owner);
    let mut commands = install_mouse_network_capture(&mut app);

    for release in [point, empty] {
        app.ingame_mouse_help = true;
        physical_left_click_with_modifiers(
            &mut app,
            point,
            ModifiersState::empty(),
            ModifiersState::empty(),
        );
        main_assert!(app.ingame_mouse_help_caption.is_some());
        app.test_cursor(PhysicalPosition::new(
            f64::from(release.x),
            f64::from(release.y),
        ));
        app.test_right_button(ElementState::Pressed);
        main_assert!(app.ingame_mouse_help, "right-down retains Help");
        app.test_right_button(ElementState::Released);
        main_assert!(!app.ingame_mouse_help, "right-up exits Help");
        main_assert_eq!(app.engine.crew_cursor(owner) => cursor);
        main_assert_eq!(commands.take_submitted_mouse_controls() => (Vec::new(), Vec::new(), Vec::new()), "Help right-up queues neither Context nor player selection");
        main_assert_eq!(
            app.ingame_mouse_help_caption =>
            Some(IngameMouseHelpCaption {
                text: "Right target".to_string(),
                keep_moves: 0,
            }),
            "right-up clears KeepCaption without erasing the caption immediately"
        );
        app.update_ingame_pointer(release).test_value();
        main_assert!(app.ingame_mouse_help_caption.is_none());
    }
}

#[test]
fn viewport_buttons_dispatch_help_and_player_menu_locally() {
    let mut app = new_running_sandbox_app();
    let owner = app.local_owner;
    render_mouse_test_app(&mut app);
    main_assert_eq!(app.local_controls.mouse_owner() => Some(owner));

    let help = viewport_button_point(&app, owner, clonk_frontend::hud::ViewportButton::Help);
    let menu = viewport_button_point(&app, owner, clonk_frontend::hud::ViewportButton::PlayerMenu);
    let chat = viewport_button_point(&app, owner, clonk_frontend::hud::ViewportButton::Chat);
    main_assert_eq!(app.ingame_viewport_region(owner, help) => Some(IngameViewportRegion::ViewportButton(clonk_frontend::hud::ViewportButton::Help,)));
    main_assert_eq!(app.ingame_viewport_region(owner, menu) => Some(IngameViewportRegion::ViewportButton(clonk_frontend::hud::ViewportButton::PlayerMenu,)));
    main_assert_eq!(app.ingame_viewport_region(owner, chat) => None, "the pending external IRC frontend keeps Chat inactive");

    let mut network_commands = install_mouse_network_capture(&mut app);
    physical_left_click_with_modifiers(
        &mut app,
        help,
        ModifiersState::empty(),
        ModifiersState::empty(),
    );
    main_assert!(app.ingame_mouse_help);
    main_assert_eq!(network_commands.take_submitted_player_inputs() => (Vec::new(), Vec::new(), Vec::new()), "COM_Help remains process-local");

    app.ingame_mouse_help = false;
    main_assert!(!app.ingame_menu_belongs_to(owner));
    physical_left_click_with_modifiers(
        &mut app,
        menu,
        ModifiersState::empty(),
        ModifiersState::empty(),
    );
    main_assert!(app.ingame_menu_belongs_to(owner));
    main_assert_eq!(network_commands.take_submitted_player_inputs() => (Vec::new(), Vec::new(), Vec::new()), "mouse COM_PlayerMenu is consumed by the local menu");

    app.ingame_menu.get_mut(owner).test_value().set_selection(2);
    physical_left_click_with_modifiers(
        &mut app,
        menu,
        ModifiersState::empty(),
        ModifiersState::empty(),
    );
    main_assert_eq!(app.ingame_menu.get(owner).expect("mouse menu remains open").selection() => 0, "a second mouse activation reinitializes the main menu");
    main_assert_eq!(network_commands.take_submitted_player_inputs() => (Vec::new(), Vec::new(), Vec::new()), "reinitializing the mouse menu remains entirely local");

    app.display_flags.show_commands = false;
    main_assert_eq!(app.ingame_viewport_region(owner, help) => None);
    main_assert_eq!(app.ingame_viewport_region(owner, menu) => None);
}

#[test]
fn ownerless_mouse_viewport_buttons_remain_local_and_open_fullscreen_menu() {
    let mut app = new_classic_running_sandbox_app();
    let removed_owner = app.local_owner;
    app.engine.remove_player(removed_owner).test_value();
    app.engine.set_local_players([]);
    app.local_controls = LocalControlRegistry::default();
    app.mouse_control = false;
    render_mouse_test_app(&mut app);

    let viewport = app.active_ingame_mouse_viewport().test_value();
    main_assert_eq!(viewport.owner => OWNER_NONE);
    let help_rect = clonk_frontend::hud::viewport_button_rect(
        viewport.rect,
        clonk_frontend::hud::ViewportButton::Help,
    );
    let menu_rect = clonk_frontend::hud::viewport_button_rect(
        viewport.rect,
        clonk_frontend::hud::ViewportButton::PlayerMenu,
    );
    let center = |rect: Rect| {
        GuiPoint::new(
            rect.x as f32 + rect.width as f32 / 2.0,
            rect.y as f32 + rect.height as f32 / 2.0,
        )
    };

    let world = GuiPoint::new(
        viewport.rect.x as f32 + viewport.rect.width as f32 / 2.0,
        viewport.rect.y as f32 + viewport.rect.height as f32 / 2.0,
    );
    main_assert_eq!(app.ingame_viewport_region(OWNER_NONE, world) => None);
    app.test_cursor(PhysicalPosition::new(
        f64::from(world.x),
        f64::from(world.y),
    ));
    app.test_left_button(ElementState::Pressed);
    main_assert!(app.mouse_state.is_none(), "passive observers never enter native DragNone world state");
    app.test_left_button(ElementState::Released);

    let mut network_commands = install_mouse_network_capture(&mut app);
    app.ingame_last_left_down = None;
    let help = center(help_rect);
    app.test_cursor(PhysicalPosition::new(f64::from(help.x), f64::from(help.y)));
    app.test_left_button(ElementState::Pressed);
    main_assert!(!app.ingame_mouse_help, "passive buttons wait for LeftUp");
    app.test_left_button(ElementState::Released);
    main_assert!(app.ingame_mouse_help);
    main_assert!(app.ingame_help_cursor_active(), "ownerless Help uses the native Help cursor too");
    main_assert!(app.ingame_menu.is_none());

    app.test_right_button(ElementState::Pressed);
    main_assert!(app.ingame_mouse_help, "right-down retains passive Help");
    app.test_right_button(ElementState::Released);
    main_assert!(!app.ingame_mouse_help, "right-up exits passive Help");

    app.ingame_last_left_down = None;
    let menu = center(menu_rect);
    app.test_cursor(PhysicalPosition::new(f64::from(menu.x), f64::from(menu.y)));
    app.test_left_button(ElementState::Pressed);
    main_assert!(app.ingame_menu.is_none(), "passive buttons wait for LeftUp");
    app.test_left_button(ElementState::Released);
    main_assert!(app.ingame_menu_belongs_to(OWNER_NONE));
    main_assert_eq!(app.ingame_menu.get(OWNER_NONE).expect("observer fullscreen menu").page() => ingame_menu::MenuPage::Main);

    render_mouse_test_app(&mut app);
    let surface = app.graphics.surface();
    let menu_target = (0..surface.height())
        .flat_map(|y| (0..surface.width()).map(move |x| (x, y)))
        .find_map(|(x, y)| {
            let point = GuiPoint::new(x as f32 + 0.5, y as f32 + 0.5);
            match app.ingame_menu_pointer_target(point) {
                Some((OWNER_NONE, IngameMenuPointerTarget::Item(index))) => Some((point, index)),
                _ => None,
            }
        })
        .test_value();
    app.test_cursor(PhysicalPosition::new(
        f64::from(menu_target.0.x),
        f64::from(menu_target.0.y),
    ));
    main_assert_eq!(app.ingame_menu.get(OWNER_NONE).expect("observer menu remains open").selection() => menu_target.1);

    app.test_key(VirtualKeyCode::Escape, ElementState::Pressed);
    main_assert!(app.ingame_menu.is_none());
    main_assert_eq!(
        network_commands.take_submitted_player_inputs() =>
        (Vec::new(), Vec::new(), Vec::new()),
        "observer button/menu input never enters synchronized player controls"
    );
}

#[test]
fn hud_command_autostop_release_survives_menu_opened_by_press() {
    let (mut app, owner, points) = command_bar_fixture(true);
    install_classic_test_assets(&mut app);
    let point = points
        .iter()
        .find_map(|(command, point)| (*command == 3).then_some(*point))
        .test_value();
    let (manager, _events, mut network_commands) =
        NetworkManager::test_stub_with_commands_for_client_id(7);
    app.network = Some(manager);

    app.test_cursor(PhysicalPosition::new(
        f64::from(point.x),
        f64::from(point.y),
    ));
    app.handle_ingame_mouse_button(ElementState::Pressed)
        .test_value();
    let (controls, commands, selections) = network_commands.take_submitted_player_inputs();
    main_assert!(commands.is_empty());
    main_assert!(selections.is_empty());
    let [(queued_owner, event, tick)] = controls.as_slice() else {
        panic!("expected one queued Buy press, got {controls:?}");
    };
    main_assert_eq!(*queued_owner => owner);
    main_assert_eq!(*event => ControlEvent::RawPlayerControl {command: 3, data: 0,});
    app.apply_ready_controls(
        *tick,
        vec![NetworkControl::Player {
            owner,
            event: *event,
        }],
    )
    .test_value();
    main_assert!(app.engine.cursor_object_menu(owner).is_some(), "COM_Up must open the contained base Buy menu before button-up");
    main_assert_eq!(
        app.script_menu_pointer_target(point)
            .expect("hit-test opened Buy menu") =>
        None,
        "the command-bar release point remains outside the GUI-owned menu"
    );

    app.handle_ingame_mouse_button(ElementState::Released)
        .test_value();
    let (controls, commands, selections) = network_commands.take_submitted_player_inputs();
    main_assert_eq!(controls => vec![(owner, ControlEvent::RawPlayerControl {command: 19, data: 0,}, *tick,)]);
    main_assert!(commands.is_empty());
    main_assert!(selections.is_empty());
}

#[test]
fn script_menu_owns_threshold_crossing_inventory_drag_move() {
    let (mut app, owner, cursor, _first, _target, inventory_point) = inventory_region_fixture();
    install_classic_test_assets(&mut app);
    install_test_cursor_menu(&mut app, cursor, two_item_script_menu(cursor));
    main_assert!(app.ingame_inventory_region_target(owner, inventory_point).is_some());
    main_assert_eq!(app.script_menu_pointer_target(inventory_point).expect("hit-test inventory point") => None, "inventory down begins outside the open GUI menu");
    let (width, height) = {
        let surface = app.graphics.surface();
        (surface.width() as i32, surface.height() as i32)
    };
    let menu_point = (0..height)
        .flat_map(|y| (0..width).map(move |x| GuiPoint::new(x as f32 + 0.5, y as f32 + 0.5)))
        .find(|point| {
            matches!(
                app.script_menu_pointer_target(*point),
                Ok(Some(EngineScriptMenuPointerTarget::Item(_)))
            ) && ((point.x - inventory_point.x).abs() > 5.0
                || (point.y - inventory_point.y).abs() > 5.0)
        })
        .test_value();

    app.ingame_last_left_down = None;
    app.test_cursor(PhysicalPosition::new(
        f64::from(inventory_point.x),
        f64::from(inventory_point.y),
    ));
    app.handle_ingame_mouse_button(ElementState::Pressed)
        .test_value();
    app.test_cursor(PhysicalPosition::new(
        f64::from(menu_point.x),
        f64::from(menu_point.y),
    ));
    main_assert!(app.mouse_state.is_some_and(|state| {!state.motion.moved && !state.motion.region_drag_started && state.motion.region_drag_cursor.is_none()}));
    main_assert!(app.ingame_dragged_objects.is_empty());
    app.handle_ingame_mouse_button(ElementState::Released)
        .test_value();
    main_assert!(app.mouse_state.is_none());
}

#[test]
fn real_goldrush_talker_opens_the_shipped_decorated_dialog() {
    let mut app = real_installed_scenario_app("Western.c4f/Goldrush.c4s", "Goldrush dialog parity");
    let mut baseline = vec![0_u8; 320 * 200 * 4];
    app.test_render(&mut baseline);
    app.engine.test_tick();

    let owner = app.local_owner;
    let snapshot = app.engine.snapshot();
    let captain = snapshot
        .objects
        .iter()
        .find(|object| {
            object.definition_id == "CVRM" && object.custom_name.as_deref() == Some("Captain")
        })
        .map(|object| object.id)
        .test_value();
    let talker = snapshot
        .objects
        .iter()
        .find(|object| {
            object.definition_id == "_TLK" && object.custom_name.as_deref() == Some("Captain")
        })
        .map(|object| object.id)
        .test_value();
    main_assert_eq!(app.engine.object_snapshot(talker).expect("Talker remains live").action.target => Some(captain));
    let cursor = app.engine.test_crew_cursor(owner);
    let talker_index = app.engine.find_object_index(talker).test_value();
    let result = app
        .engine
        .call_object_function(
            talker_index,
            "ActivateEntrance",
            vec![Value::Object(cursor.as_u64())],
        )
        .test_value();
    main_assert!(result.as_bool());

    let (_, first_menu) = app.engine.cursor_object_menu(owner).test_value();
    main_assert_eq!(first_menu.style => 3);
    app.engine
        .player_in_com(owner, clonk_engine::COM_MENU_SHOW_TEXT, 0)
        .test_value();
    app.engine
        .player_in_com(owner, clonk_engine::COM_MENU_CLOSE, 0)
        .test_value();
    app.engine.test_tick();

    let (_, menu) = app.engine.cursor_object_menu(owner).test_value();
    main_assert_eq!(menu.style => 3);
    main_assert_eq!(menu.extra => clonk_engine::ObjectMenuExtra::None);
    main_assert_eq!(menu.items.len() => 2);
    main_assert!(menu.text_progressing);
    main_assert!(matches!(&menu.items[0].image, clonk_engine::ObjectMenuImage::TextSpec { spec, .. } if spec.ends_with("::Captain1")));
    let decoration = menu.decoration.test_ref();
    main_assert_eq!(decoration.source_definition => "MD69");
    main_assert_eq!(decoration.background_color => 0x803f3f00);
    main_assert_eq!((decoration.border_top, decoration.border_left, decoration.border_right, decoration.border_bottom,) => (10, 10, 10, 10));
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
            (0, 0, 28, 30, -10, -10),
            (30, 0, 71, 25, 0, -10),
            (101, 0, 30, 25, 0, -10),
            (98, 31, 30, 71, 0, 0),
            (98, 101, 30, 30, 0, 0),
            (28, 101, 71, 30, 0, 0),
            (0, 101, 28, 30, -10, 0),
            (0, 31, 30, 71, -10, 0),
        ]
    );
    main_assert_eq!(app.engine.definition_named_portrait_graphics_image("CVRM", "Captain1").map(|image| (image.width(), image.height())) => Some((150, 150)));
    main_assert_eq!(app.engine.definition_sprite_image("MD69", None).map(|image| (image.width(), image.height())) => Some((128, 128)));

    app.snapshot = app.engine.snapshot();
    app.snapshot.hud.messages.clear();
    let mut rendered = vec![0_u8; 320 * 200 * 4];
    app.test_render(&mut rendered);
    main_assert_ne!(rendered => baseline);
}

#[test]
fn audio_context_selects_configured_linear_resampling() {
    let audio = AudioContext::try_new(AudioOptions {
        prefer_linear_resampling: true,
        ..AudioOptions::default()
    })
    .test_value();

    main_assert_eq!(audio.system.resampling_mode() => ResamplingMode::Linear);
}

#[test]
fn info_menu_preflight_rejects_unresolved_text_images() {
    let script = r#"
        func Initialize()
        {
            CreateMenu(MENU, this(), this(), 0, "Info", 0, 2);
            AddMenuItem("", "", MENU, this(), 0, 0, "{{MISS}} unavailable");
        }
        "#;
    let mut engine = Engine::new();
    engine
        .register_script_definition("MENU", "Menu", script)
        .test_value();
    let object = engine.spawn_test_object(SpawnConfig::new("MENU"));
    let menu = engine
        .debug_object_menu(object.as_u64())
        .expect("menu object exists")
        .test_value();

    let error = resolve_script_menu_font_images(&engine, &menu, ScriptTextSpecResources::default())
        .expect_err("missing text image must fail before rendering");
    main_assert!(error.to_string().contains("{{MISS}}"));
}

#[test]
fn tutorial_portrait_geometry_matches_every_shipped_position_family() {
    // C4Viewport::Execute supplies the player's DrawX/DrawY/ViewWdt/ViewHgt
    // facet (src/C4Viewport.cpp:1146-1149). C4GM_XRel, C4GM_YRel and
    // C4GM_WidthRel are integer percentages of that facet, not the whole
    // backbuffer (src/C4GameMessage.cpp:109-111,136-137). These rows are
    // the SetTutorialMessagePos calls in Tutorial01-10: 01/02 use 30%
    // Top|Left at XRel=50; 03/04/05/06 use 35% Bottom|Left at XRel=10;
    // 07/08/09/10 use HCenter|Top.
    let viewport = Rect::new(17, 23, 321, 241);
    let frame_size = (101, 65);
    for (tutorials, offset, width, flags, expected_geometry, expected_frame) in [
        (
            "Tutorial01/02",
            Vector2::new(50, 50),
            30,
            FLAG_TOP | FLAG_LEFT | FLAG_WIDTH_REL | FLAG_X_REL,
            menus1_fixture!(message_geometry: 177, 73, 96),
            Rect::new(177, 73, 101, 65),
        ),
        (
            "Tutorial03",
            Vector2::new(10, -50),
            35,
            FLAG_BOTTOM | FLAG_LEFT | FLAG_WIDTH_REL | FLAG_X_REL,
            menus1_fixture!(message_geometry: 49, -27, 112),
            Rect::new(49, 149, 101, 65),
        ),
        (
            "Tutorial04/06",
            Vector2::new(10, -30),
            35,
            FLAG_BOTTOM | FLAG_LEFT | FLAG_WIDTH_REL | FLAG_X_REL,
            menus1_fixture!(message_geometry: 49, -7, 112),
            Rect::new(49, 169, 101, 65),
        ),
        (
            "Tutorial05",
            Vector2::new(10, -10),
            35,
            FLAG_BOTTOM | FLAG_LEFT | FLAG_WIDTH_REL | FLAG_X_REL,
            menus1_fixture!(message_geometry: 49, 13, 112),
            Rect::new(49, 189, 101, 65),
        ),
        (
            "Tutorial07-10",
            Vector2::new(0, 30),
            0,
            FLAG_HCENTER | FLAG_TOP,
            menus1_fixture!(message_geometry: 17, 53, 0),
            Rect::new(127, 53, 101, 65),
        ),
    ] {
        let geometry = global_message_viewport_geometry(viewport, offset, width, flags);
        main_assert_eq!(geometry => expected_geometry, "{tutorials}");
        main_assert_eq!(global_portrait_frame_rect(viewport, offset, flags, frame_size) => expected_frame, "{tutorials}");
    }
}

#[test]
fn inventory_and_menu_color_modulation_alpha_fades_without_filling_background() {
    let mut definition = test_definition("FADE", "Fade", "");
    definition.set_picture(Some(menus1_fixture!(definition_picture: 2, 1)));
    definition.set_sprite_image(Some(menus1_fixture!(
        sprite_image:
            2,
            1,
            Arc::from([
                        0, 0, 0, 0xff, // opaque texel
                        0, 0, 0, 0, // transparent background texel
                    ]),
            None,
    )));
    let mut engine = Engine::new();
    engine.register_test_definition(definition);

    let mut object = make_object(1, "FADE", Vector2::ZERO);
    object.color_modulation = 0x00ff_ffff;
    let unchanged = inventory_object_picture(&engine, &object).test_value();
    main_assert_eq!(unchanged.pixels() => &[0, 0, 0, 0xff, 0, 0, 0, 0]);

    object.color_modulation = 0x80ff_ffff;
    let inventory = inventory_object_picture(&engine, &object).test_value();
    main_assert_eq!(inventory.pixels() => &[0, 0, 0, 0x7f, 0, 0, 0, 0], "the fast picture path subtracts C4 transparency from texel opacity");

    let item = menus1_fixture!(
        menu_item:
            "Fade".to_string(),
            1,
            "FADE".to_string(),
            clonk_engine::ObjectMenuSymbol::Definition,
            clonk_engine::ObjectMenuImage::Object { object: object.id },
            Some("FADE".to_string()),
            Some(menus1_fixture!(menu_picture: "FADE".to_string(), 2, Vec::new(), 0, 0x80ff_ffff)),
            true,
    );
    let menu = object_menu_item_picture(
        &engine,
        &make_snapshot(Vec::new(), Vec::new()),
        &item,
        0,
        &HudGraphics::default(),
        0,
    )
    .test_value();
    main_assert_eq!((menu.width(), menu.height()) => (2, 2));
    main_assert_eq!(
        menu.pixels() =>
        &[0, 0, 0, 0x7f, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0],
        "the menu fades opaque texels without filling transparent background or padding"
    );
}

#[test]
fn picture_overlay_transform_keeps_shear_and_projective_row_at_center() {
    // C4GraphicsOverlay::DrawPicture scales only c/f, then rebases the
    // complete matrix around the picture center. This asymmetric matrix
    // distinguishes that path from the former diagonal scale reduction.
    let transform = centered_picture_transform(
        [0.8, -0.3, 2.0, 0.25, 1.2, -3.0, 0.01, -0.02, 1.0],
        2.0,
        10.0,
        6.0,
    );
    let expected = [0.9, -0.5, 8.0, 0.31, 1.08, -9.58, 0.01, -0.02, 1.02];
    for (actual, expected) in transform.mat.into_iter().zip(expected) {
        main_assert!((actual - expected).abs() < 1.0e-5, "{actual} != {expected}");
    }

    let (x, y) = transform.transform_point(10.0, 6.0);
    main_assert!((x - 14.0).abs() < 1.0e-5);
    main_assert!(y.abs() < 1.0e-5);
}

#[test]
fn script_menu_images_use_resolved_definition_phase_and_color() {
    let mut definition = test_definition("PHAS", "Phases", "");
    definition.set_picture(Some(menus1_fixture!(definition_picture: 1, 1)));
    definition.set_sprite_image(Some(menus1_fixture!(
        sprite_image:
            2,
            1,
            Arc::from([0xff, 0, 0, 0xff, 0xff, 0xff, 0xff, 0xff]),
            Some(Arc::from([0_u8, 0xff])),
    )));
    let mut engine = Engine::new();
    engine.register_test_definition(definition);
    let snapshot = make_snapshot(Vec::new(), Vec::new());
    let item = menus1_fixture!(
        menu_item:
            "Indexed color".to_string(),
            12_345_678,
            "MISS".to_string(),
            clonk_engine::ObjectMenuSymbol::Definition,
            clonk_engine::ObjectMenuImage::IndexedColor {
                        index: 1,
                        color: 0x445566,
                    },
            Some("PHAS".to_string()),
            None,
            false,
    );

    let picture =
        object_menu_item_picture(&engine, &snapshot, &item, 0, &HudGraphics::default(), 0)
            .test_value();
    main_assert_eq!(picture.pixels() => &[0x44, 0x55, 0x66, 0xff]);

    let text_spec = resolve_script_font_image(
        &engine,
        "PHAS: +1trailing",
        0x112233,
        ScriptTextSpecResources::default(),
    )
    .test_value();
    main_assert_eq!(text_spec.pixels() => &[0x11, 0x22, 0x33, 0xff]);
}

#[test]
fn script_object_menu_image_survives_source_object_deletion() {
    let mut definition = test_definition("OBJC", "Object", "");
    definition.set_picture(Some(menus1_fixture!(definition_picture: 1, 1)));
    definition.set_sprite_image(Some(menus1_fixture!(
        sprite_image:
            1,
            1,
            Arc::from([0xff, 0xff, 0xff, 0xff]),
            Some(Arc::from([0xff_u8])),
    )));
    let mut engine = Engine::new();
    engine.register_test_definition(definition);
    let item = menus1_fixture!(
        menu_item:
            "Object".to_string(),
            12_345_678,
            "NONE".to_string(),
            clonk_engine::ObjectMenuSymbol::Definition,
            clonk_engine::ObjectMenuImage::Object {
                        object: ObjectId::new(7),
                    },
            Some("OBJC".to_string()),
            Some(menus1_fixture!(menu_picture: "OBJC".to_string(), 35, Vec::new(), 0x123456, 0)),
            false,
    );
    let empty_snapshot = make_snapshot(Vec::new(), Vec::new());

    let picture = object_menu_item_picture(
        &engine,
        &empty_snapshot,
        &item,
        0,
        &HudGraphics::default(),
        0,
    )
    .test_value();
    main_assert_eq!(picture.pixels() => &[0x12, 0x34, 0x56, 0xff]);
}

#[test]
fn script_object_menu_overlay_uses_owned_square_and_aspect_fit() {
    let mut engine = Engine::new();
    let mut base = test_definition("BASE", "Base", "");
    base.set_picture(Some(menus1_fixture!(definition_picture: 2, 1)));
    base.set_sprite_image(Some(
        menus1_fixture!(sprite_image: 2, 1, Arc::from([0xff, 0, 0, 0xff, 0xff, 0, 0, 0xff]), None),
    ));
    engine.register_test_definition(base);
    let mut overlay = test_definition("OVRL", "Overlay", "");
    overlay.set_picture(Some(menus1_fixture!(definition_picture: 1, 1)));
    overlay.set_sprite_image(Some(
        menus1_fixture!(sprite_image: 1, 1, Arc::from([0, 0, 0xff, 0xff]), None),
    ));
    engine.register_test_definition(overlay);
    let item = menus1_fixture!(
        menu_item:
            "Composite".to_string(),
            12_345_678,
            "NONE".to_string(),
            clonk_engine::ObjectMenuSymbol::Definition,
            clonk_engine::ObjectMenuImage::Object {
                        object: ObjectId::new(9),
                    },
            Some("BASE".to_string()),
            Some(menus1_fixture!(
                menu_picture:
                    "BASE".to_string(),
                    4,
                    vec![clonk_engine::ObjectGraphicsOverlay::new(
                                    1,
                                    clonk_engine::GraphicsOverlayMode::Picture,
                                )
                                .with_definition(Some("OVRL".to_string()))],
                    0,
                    0,
            )),
            false,
    );
    let picture = object_menu_item_picture(
        &engine,
        &make_snapshot(Vec::new(), Vec::new()),
        &item,
        0,
        &HudGraphics::default(),
        0,
    )
    .test_value();
    main_assert_eq!((picture.width(), picture.height()) => (4, 4));
    for (index, pixel) in picture.pixels().chunks_exact(4).enumerate() {
        let row = index / 4;
        let blue = if (1..3).contains(&row) { 0xfe } else { 0xff };
        main_assert_eq!(pixel => &[0, 0, blue, 0xff], "opaque software overlay retains BltAlpha's /256 quirk over the red base",);
    }

    let mut ranked = item.clone();
    ranked.image = clonk_engine::ObjectMenuImage::ObjectRank {
        object: ObjectId::new(9),
    };
    ranked.picture_snapshot.test_mut().graphics_overlays.clear();
    let ranked_picture = object_menu_item_picture(
        &engine,
        &make_snapshot(Vec::new(), Vec::new()),
        &ranked,
        0,
        &HudGraphics::default(),
        0,
    )
    .test_value();
    main_assert_eq!((ranked_picture.width(), ranked_picture.height()) => (4, 4));
    main_assert_eq!(&ranked_picture.pixels()[0..4] => &[0, 0, 0, 0]);
    main_assert_eq!(&ranked_picture.pixels()[4 * 4..4 * 5] => &[0xff, 0, 0, 0xff]);
}

/// A Context `ObjectRank` row builds its facet from the menu's live
/// `GetItemHeight()` - `fctSymbol.Create(H * 2, H)` with the object left and
/// the rank right - while every other style keeps the add-time
/// `GetSymbolSize()` (C4Script.cpp:1717-1728; C4Menu.cpp:650-652).
#[test]
fn context_object_rank_snapshot_uses_runtime_item_height() {
    let mut definition = test_definition("OBJC", "Object", "");
    definition.set_picture(Some(menus1_fixture!(definition_picture: 2, 2)));
    definition.set_sprite_image(Some(menus1_fixture!(sprite_image: 2, 2, Arc::from([0xff, 0, 0, 0xff].repeat(4).as_slice()), None)));
    let mut engine = Engine::new();
    engine.register_test_definition(definition);

    let item = menus1_fixture!(
        menu_item:
            String::new(),
            0,
            String::new(),
            clonk_engine::ObjectMenuSymbol::default(),
            clonk_engine::ObjectMenuImage::ObjectRank {
                        object: ObjectId::new(9),
                    },
            Some("OBJC".into()),
            Some(clonk_engine::ObjectMenuPictureSnapshot {
                        definition_id: "OBJC".into(),
                        // The add-time GetSymbolSize() a non-Context menu keeps.
                        symbol_size: 4,
                        base_graphics: None,
                        graphics_overlays: Vec::new(),
                        blit_mode: 0,
                        color: 0,
                        color_modulation: 0,
                        picture_rect: clonk_engine::DefinitionRect::default(),
                        rank: None,
                    }),
            false,
    );
    let snapshot = make_snapshot(Vec::new(), Vec::new());
    let extent = |style: i32, context_item_height: Option<i32>| {
        let picture = clonk_app_core::pictures::object_menu_item_picture_with_context_height(
            &engine,
            &snapshot,
            &item,
            0,
            &HudGraphics::default(),
            style,
            ScriptTextSpecResources::default(),
            15,
            context_item_height,
        )
        .test_value();
        (picture.width(), picture.height())
    };

    // Context: the resolved row height wins over the add-time symbol size,
    // and the facet stays twice as wide as it is tall.
    main_assert_eq!(extent(1, Some(16)) => (32, 16));
    main_assert_eq!(extent(1, Some(9)) => (18, 9));
    // Without a resolved height the add-time size is the fallback.
    main_assert_eq!(extent(1, None) => (8, 4));
    // Every other style keeps the square add-time symbol size regardless.
    main_assert_eq!(extent(0, Some(16)) => (4, 4));
    main_assert_eq!(extent(3, Some(16)) => (4, 4));
}

#[test]
fn script_rank_menu_image_composes_extended_captain_symbol() {
    let mut rank_pixels = Vec::new();
    for _ in 0..3 {
        for _ in 0..3 {
            rank_pixels.extend_from_slice(&[0xff, 0, 0, 0xff]);
        }
        for _ in 0..3 {
            rank_pixels.extend_from_slice(&[0, 0xff, 0, 0xff]);
        }
    }
    let mut captain_pixels = Vec::new();
    for _ in 0..9 {
        captain_pixels.extend_from_slice(&[0, 0, 0xff, 0xff]);
    }
    let hud = HudGraphics {
        rank: Some(ImageData::new(6, 3, rank_pixels)),
        captain: Some(ImageData::new(3, 3, captain_pixels)),
        ..HudGraphics::default()
    };
    let engine = Engine::new();
    let snapshot = make_snapshot(Vec::new(), Vec::new());
    let item = menus1_fixture!(
        menu_item:
            "Rank".to_string(),
            12_345_678,
            "CLNK".to_string(),
            clonk_engine::ObjectMenuSymbol::Definition,
            clonk_engine::ObjectMenuImage::Rank { rank: 2 },
            Some("CLNK".to_string()),
            None,
            false,
    );

    let picture = object_menu_item_picture(&engine, &snapshot, &item, 0, &hud, 1).test_value();
    main_assert_eq!((picture.width(), picture.height()) => (3, 3));
    main_assert_eq!(&picture.pixels()[0..4] => &[0, 0, 0xfe, 0xff], "captain overlay uses native software BltAlpha /256 composition",);
    let bottom_right = ((2 * 3 + 2) * 4) as usize;
    main_assert_eq!(&picture.pixels()[bottom_right..bottom_right + 4] => &[0xff, 0, 0, 0xff]);
}

#[test]
fn menu_state_navigates_folders() {
    let scenarios = sample_scenarios();
    let entries = build_menu_entries(&scenarios, false);
    let menu = StartupMenu::new(entries, test_font(), None).test_value();
    let mut state = MenuState::new(menu, scenarios);

    main_assert_eq!(state.current_entries().len() => 1);
    let root_entries = build_menu_entries(state.current_entries(), true);
    main_assert_eq!(root_entries.len() => 2);
    main_assert_eq!(root_entries[0].identifier => BACK_ENTRY_IDENTIFIER);
    main_assert_eq!(root_entries[1].identifier => "folder_missions");
    main_assert_eq!(state.label_path() => "Scenarios".to_string());
    state.refresh_menu_entries();
    let root_selection = state.select_default_entry();
    main_assert!(
        matches!(
            root_selection.as_slice(),
            [StartupMenuAction::SelectionChanged(summary)]
            if summary.identifier == "folder_missions"
        ),
        "expected default selection to target folder_missions"
    );

    state.enter_folder("folder_missions");
    main_assert_eq!(state.current_entries().len() => 1);
    main_assert_eq!(state.stack.len() => 2);
    let folder_entries = build_menu_entries(state.current_entries(), true);
    main_assert_eq!(folder_entries.len() => 2);
    main_assert_eq!(folder_entries[0].identifier => BACK_ENTRY_IDENTIFIER);
    main_assert_eq!(folder_entries[1].identifier => "scenario_alpha");
    main_assert_eq!(state.label_path() => "Scenarios / Missions".to_string());
    let folder_selection = state.select_default_entry();
    main_assert!(
        matches!(
            folder_selection.as_slice(),
            [StartupMenuAction::SelectionChanged(summary)]
            if summary.identifier == "scenario_alpha"
        ),
        "expected default selection to target scenario_alpha"
    );

    state.leave_folder();
    main_assert_eq!(state.current_entries().len() => 1);
    main_assert_eq!(state.stack.len() => 1);
    let root_again = build_menu_entries(state.current_entries(), true);
    main_assert_eq!(root_again.len() => 2);
    main_assert_eq!(root_again[0].identifier => BACK_ENTRY_IDENTIFIER);
    main_assert_eq!(root_again[1].identifier => "folder_missions");
    main_assert_eq!(state.label_path() => "Scenarios".to_string());
    let root_again_selection = state.select_default_entry();
    main_assert!(
        root_again_selection.is_empty()
            || matches!(
                root_again_selection.as_slice(),
                [StartupMenuAction::SelectionChanged(summary)]
                if summary.identifier == "folder_missions"
            ),
        "expected default selection to target folder_missions after returning to root"
    );
}

#[test]
fn scenario_game_options_load_persist_force_and_use_classic_input_dialog() {
    let _lock = env_lock().lock();
    let user_data = tempdir();
    let (_guard, paths) = guarded_test_app_paths(None, user_data.path());
    if let Some(parent) = paths.config_file().parent() {
        fs::create_dir_all(parent).test_value();
    }
    fs::write(paths.config_file(), b"[General]\r\nFairCrew=true\r\n").test_value();
    main_assert!(!load_fair_crew_flag(Some(&paths)));
    main_assert!(!load_scenario_game_option_values(Some(&paths)).fair_crew);
    fs::remove_file(paths.config_file()).test_value();

    for (section, key, value) in [
        ("General", "DefCrewStrength", "777"),
        ("General", "Record", "0"),
        ("Network", "MasterServerSignUp", "0"),
        ("Network", "LeagueServerSignUp", "1"),
        ("Network", "Comment", "old comment"),
        ("Network", "LastPassword", "old password"),
    ] {
        persist_config_value(&paths, section, key, value).test_value();
    }
    persist_native_config_values(
        &paths,
        "General",
        &[(
            "NoCrew",
            clonk_app_netplay::NativeConfigValue::RawAscii("true"),
        )],
    )
    .test_value();
    main_assert!(load_fair_crew_flag(Some(&paths)), "the scen-sel flag reads C4ConfigGeneral's native NoCrew key");
    let values = load_scenario_game_option_values(Some(&paths));
    main_assert!(values.fair_crew);
    main_assert_eq!(values.fair_crew_strength => 777);
    main_assert!(!values.record);
    main_assert!(!values.master_server_signup);
    main_assert!(values.league_server_signup);
    main_assert_eq!(values.comment => "old comment");
    main_assert_eq!(values.last_password => "old password");

    let scenario_path = user_data.path().join("Forced.c4s");
    fs::create_dir_all(&scenario_path).test_value();
    fs::write(
        scenario_path.join("Scenario.txt"),
        "[Head]\nTitle=Forced\nForcedNoCrew=2\n",
    )
    .test_value();
    let mut forced = FrontendScenario::fallback();
    forced.path = Some(scenario_path);
    main_assert_eq!(scenario_fair_crew_constraint(Some(&forced)) => FairCrewConstraint::ForceNormal);
    let mut controller = GameOptionButtons::new(GameOptionContext::LocalSelector, values.clone());
    controller.set_selector_fair_crew_constraint(FairCrewConstraint::ForceNormal);
    let fair = controller
        .view(clonk_frontend::game_option_buttons::GameOptionButton::FairCrew)
        .test_value();
    main_assert!(!fair.enabled);
    main_assert_eq!(fair.icon => clonk_frontend::game_option_buttons::GameOptionIcon::NormalCrewGray);

    let mut app = GameApp::new(
        800,
        600,
        disabled_audio_options(),
        Some(&paths),
        test_runtime_config_with("Option Tester".to_string(), false),
    )
    .test_value();
    wait_for_menu(&mut app);
    app.open_scenario_browser();
    let fonts = app.assets.clonk_fonts.as_deref().test_value();
    let bounds = startup_scensel_game_option_bounds(800, 600, fonts);
    let option_layout = clonk_frontend::game_option_buttons::game_option_buttons_layout(
        bounds,
        GameOptionContext::LocalSelector,
    );
    let scensel_layout = clonk_frontend::startup_scensel::scen_sel_layout(800, 600, fonts);
    main_assert_eq!(option_layout.rect(clonk_frontend::game_option_buttons::GameOptionButton::FairCrew) => Some(scensel_layout.fair_crew_button));
    main_assert_eq!(option_layout.rect(clonk_frontend::game_option_buttons::GameOptionButton::Record) => Some(scensel_layout.record_button));

    app.process_game_option_actions(vec![GameOptionAction::FairCrewPreferenceChanged(false)])
        .test_value();
    let native_config = fs::read(paths.config_file()).test_value();
    main_assert!(native_config.split(|byte| matches!(*byte, b'\r' | b'\n')).any(|line| line == b"NoCrew=false"));
    main_assert!(!native_config.windows(b"FairCrew=".len()).any(|window| window == b"FairCrew="));
    main_assert!(!load_fair_crew_flag(Some(&paths)));
    main_assert!(!load_scenario_game_option_values(Some(&paths)).fair_crew);

    app.process_game_option_actions(vec![
        GameOptionAction::RecordPreferenceChanged(true),
        GameOptionAction::InternetSignupChanged {
            enabled: true,
            live_lobby: false,
        },
        GameOptionAction::LeagueSignupChanged(false),
        GameOptionAction::CommentChanged("new comment".to_string()),
    ])
    .test_value();
    // No C++ game-option surface saves the file, so the four toggles above sit
    // in the deferred store until a save surface runs; the subject here is the
    // written content, so flush the way a clean shutdown would.
    app.flush_deferred_config();
    let config = Config::load(paths.config_file()).test_value();
    main_assert_eq!(config.get_in(Some("General"), "NoCrew") => Some("false"));
    main_assert_eq!(config.get_in(Some("General"), "FairCrew") => None);
    main_assert_eq!(config.get_in(Some("General"), "Record") => Some("1"));
    main_assert_eq!(config.get_in(Some("General"), "DefCrewStrength") => Some("777"));
    main_assert_eq!(config.get_in(Some("Network"), "MasterServerSignUp") => Some("1"));
    main_assert_eq!(config.get_in(Some("Network"), "LeagueServerSignUp") => Some("0"));
    main_assert_eq!(config.get_in(Some("Network"), "Comment") => Some("new comment"));

    app.process_game_option_actions(vec![GameOptionAction::FairCrewPreferenceChanged(true)])
        .test_value();
    let config = Config::load(paths.config_file()).test_value();
    main_assert_eq!(config.get_in(Some("General"), "NoCrew") => Some("true"));
    main_assert_eq!(config.get_in(Some("General"), "FairCrew") => None);
    let native_config = fs::read(paths.config_file()).test_value();
    main_assert!(native_config.split(|byte| matches!(*byte, b'\r' | b'\n')).any(|line| line == b"NoCrew=true"));
    main_assert!(!native_config.windows(b"FairCrew=".len()).any(|window| window == b"FairCrew="));
    main_assert!(load_fair_crew_flag(Some(&paths)));
    main_assert!(load_scenario_game_option_values(Some(&paths)).fair_crew);

    app.scenario_game_options =
        GameOptionButtons::new(GameOptionContext::NetworkHostSelector, values);
    let actions = app.scenario_game_options.handle_hotkey('P');
    app.finish_game_option_input(actions).test_value();
    let dialog = app.game_option_input_dialog.test_ref();
    main_assert_eq!(dialog.purpose => PendingInputDialogPurpose::GameOption(GameOptionInputKind::Password));
    main_assert_eq!(dialog.controller.caption() => "Password");
    main_assert_eq!(dialog.controller.text() => "old password");
    app.process_game_option_input_dialog_actions(vec![InputDialogAction::Accepted(
        "new password".to_string(),
    )])
    .test_value();
    main_assert_eq!(app.scenario_game_options.values().password => "new password");
    // `SCopy(szPass, Config.Network.LastPassword, ...)` writes memory only
    // (C4Network2Dialogs.cpp:748). `LastPassword` is a `CFG_MaxString` field,
    // so the flush has to hand the native bytes to the escaped writer.
    app.flush_deferred_config();
    main_assert_eq!(Config::load(paths.config_file()).expect("reload password").get_in(Some("Network"), "LastPassword") => Some("new password"));
    reset_cached_app_paths();
}

#[test]
fn a_game_option_reaches_the_file_only_at_a_save_surface() {
    // The whole C++ tree holds seven `Config.Save()` calls — the Options
    // dialog (C4StartupOptionsDlg.cpp:1183), the updater (C4UpdateDlg.cpp:338,347),
    // the masterserver redirect (C4StartupNetDlg.cpp:314) and three in
    // `C4Application` — and not one of them is a game-option surface. A toggle
    // therefore lives in the process-wide `Config` until the shutdown write
    // (C4Application.cpp:367), so a crash discards it.
    let _lock = env_lock().lock();
    let fixture = tempdir();
    let (_guard, paths) = exact_loader_test_paths(fixture.path(), None);
    fs::create_dir_all(paths.config_file().parent().test_value()).test_value();
    fs::write(
        paths.config_file(),
        b"[General]\r\nRecord=0\r\n\r\n[Network]\r\nComment=\"old comment\"\r\n",
    )
    .test_value();
    let mut app = new_state_only_menu_app(320, 200);
    app.app_paths = Some(paths.clone());

    app.process_game_option_actions(vec![
        GameOptionAction::RecordPreferenceChanged(true),
        GameOptionAction::CommentChanged("deferred comment".to_string()),
    ])
    .test_value();

    main_assert_eq!(app.deferred_config.get("General", "Record") => Some("1"));
    let unflushed = Config::load(paths.config_file()).test_value();
    main_assert_eq!(unflushed.get_in(Some("General"), "Record") => Some("0"));
    main_assert_eq!(unflushed.get_in(Some("Network"), "Comment") => Some("old comment"), "the file keeps what this session started from");

    app.flush_deferred_config();
    let saved = Config::load(paths.config_file()).test_value();
    main_assert_eq!(saved.get_in(Some("General"), "Record") => Some("1"));
    main_assert_eq!(saved.get_in(Some("Network"), "Comment") => Some("deferred comment"));
    // `Network.Comment` is a `CFG_MaxString` field, so the flush has to keep
    // C++'s quoted form rather than writing the scalar through unchanged.
    let native = fs::read(paths.config_file()).test_value();
    main_assert!(native.windows(b"Comment=\"deferred comment\"".len()).any(|window| window == b"Comment=\"deferred comment\""));
    reset_cached_app_paths();
}

#[test]
fn a_pending_game_option_survives_rebuilding_the_option_buttons() {
    // C++ rebuilds `C4GameOptionButtons` from the one process-wide `Config`
    // that its own toggles just wrote — record at C4Network2Dialogs.cpp:648
    // over :715, league at :629 over :686, comment at :768 over :776 and the
    // remembered password at :733 over :748 — so re-entering the scenario
    // browser shows what this session set, not what is still on disk.
    let _lock = env_lock().lock();
    let fixture = tempdir();
    let (_guard, paths) = exact_loader_test_paths(fixture.path(), None);
    fs::create_dir_all(paths.config_file().parent().test_value()).test_value();
    fs::write(
        paths.config_file(),
        b"[General]\r\nRecord=0\r\n\r\n[Network]\r\nComment=\"old comment\"\r\nLeagueServerSignUp=0\r\nLastPassword=\"old pass\"\r\n",
    )
    .test_value();
    let mut app = new_state_only_menu_app(320, 200);
    app.app_paths = Some(paths.clone());

    app.process_game_option_actions(vec![
        GameOptionAction::RecordPreferenceChanged(true),
        GameOptionAction::LeagueSignupChanged(true),
        GameOptionAction::CommentChanged("live comment".to_string()),
        GameOptionAction::PasswordChanged {
            remember_for_next_round: Some("live pass".to_string()),
            password: String::new(),
        },
    ])
    .test_value();

    let rebuilt = app.scenario_game_option_values();
    main_assert!(rebuilt.record, "the Record toggle survives the rebuild");
    main_assert!(rebuilt.league_server_signup);
    main_assert_eq!(rebuilt.comment => "live comment");
    main_assert_eq!(rebuilt.last_password => "live pass");
    // Nothing has been written yet — the file is still what the session
    // started from.
    let on_disk = Config::load(paths.config_file()).test_value();
    main_assert_eq!(on_disk.get_in(Some("General"), "Record") => Some("0"));
    main_assert_eq!(on_disk.get_in(Some("Network"), "Comment") => Some("old comment"));
    reset_cached_app_paths();
}

#[test]
fn game_option_input_dialog_is_modal_and_pointer_capture_is_per_gesture() {
    let _lock = env_lock().lock();
    let user_data = tempdir();
    let (_guard, paths) = guarded_test_app_paths(None, user_data.path());
    let mut app = GameApp::new(
        800,
        600,
        AudioOptions::default(),
        Some(&paths),
        test_runtime_config_with("Modal Tester".to_string(), false),
    )
    .test_value();
    wait_for_menu(&mut app);
    app.open_scenario_browser();
    app.menu_state.set_search_text("underlying search");
    let selected = app.menu_state.menu.selected_index();
    let stack_len = app.menu_state.stack.len();
    app.scenario_game_options = GameOptionButtons::new(
        GameOptionContext::NetworkHostSelector,
        GameOptionValues::default(),
    );
    let actions = app.scenario_game_options.handle_hotkey('P');
    app.finish_game_option_input(actions).test_value();

    app.test_modifiers(ModifiersState::CONTROL | ModifiersState::ALT);
    app.test_key(VirtualKeyCode::KeyO, ElementState::Pressed);
    app.test_key(VirtualKeyCode::KeyO, ElementState::Released);
    main_assert!(app.game_option_input_dialog.is_some());
    app.test_modifiers(ModifiersState::SHIFT);
    app.test_key(VirtualKeyCode::Escape, ElementState::Pressed);
    app.test_key(VirtualKeyCode::Escape, ElementState::Released);
    main_assert!(app.game_option_input_dialog.is_some());
    app.test_modifiers(ModifiersState::empty());

    for key in [
        VirtualKeyCode::ArrowUp,
        VirtualKeyCode::ArrowDown,
        VirtualKeyCode::ArrowLeft,
    ] {
        app.test_key(key, ElementState::Pressed);
        app.test_key(key, ElementState::Released);
    }
    main_assert_eq!(app.menu_state.menu.selected_index() => selected);
    main_assert_eq!(app.menu_state.stack.len() => stack_len);
    main_assert_eq!(app.menu_state.search_text() => "underlying search");
    main_assert_eq!(app.startup_view => StartupView::ScenarioBrowser);

    app.test_key(VirtualKeyCode::ContextMenu, ElementState::Pressed);
    main_assert!(app.context_menu.is_some());
    main_assert!(GameApp::startup_base_context_menu(app.context_menu.as_ref(), true,).is_none(), "modal owns the one context-menu render pass");
    app.test_key(VirtualKeyCode::ContextMenu, ElementState::Released);
    app.close_context_menu_silently();

    let layout = app.game_option_input_layout().test_value();
    let edit_point = PhysicalPosition::new(
        f64::from(layout.edit.x + layout.edit.w / 2),
        f64::from(layout.edit.y + layout.edit.h / 2),
    );
    app.test_cursor(edit_point);
    app.test_left_button(ElementState::Pressed);
    main_assert_eq!(app.game_option_input_pointer_capture => Some(ContextMenuPointerButton::Left));
    app.process_game_option_input_dialog_actions(vec![InputDialogAction::Cancelled])
        .test_value();
    main_assert!(app.game_option_input_dialog.is_none());
    app.test_left_button(ElementState::Released);
    main_assert_eq!(app.game_option_input_pointer_capture => None);
    main_assert_eq!(app.menu_state.menu.selected_index() => selected);

    let actions = app.scenario_game_options.handle_hotkey('P');
    app.finish_game_option_input(actions).test_value();
    app.test_cursor(edit_point);
    app.handle_other_mouse_button(ElementState::Pressed)
        .test_value();
    main_assert_eq!(app.game_option_input_pointer_capture => Some(ContextMenuPointerButton::Other));
    app.handle_other_mouse_button(ElementState::Released)
        .test_value();
    main_assert_eq!(app.game_option_input_pointer_capture => None);
    reset_cached_app_paths();
}

#[test]
fn resize_cancels_selector_option_and_input_dialog_interactions() {
    let _lock = env_lock().lock();
    let user_data = tempdir();
    let (_guard, paths) = guarded_test_app_paths(None, user_data.path());
    let mut app = new_menu_app_with_paths(800, 600, &paths);
    app.open_scenario_browser();
    app.scenario_game_options.set_focused_button(Some(
        clonk_frontend::game_option_buttons::GameOptionButton::Record,
    ));
    app.menu_state.set_dialog_focus(ScenselDialogFocus::Options);
    app.test_key(VirtualKeyCode::Space, ElementState::Pressed);
    main_assert!(!app.game_option_consumed_keys.is_empty());
    let record = app
        .scenario_game_options
        .layout()
        .rect(clonk_frontend::game_option_buttons::GameOptionButton::Record)
        .test_value();
    app.test_cursor(PhysicalPosition::new(
        f64::from(record.x + record.w / 2),
        f64::from(record.y + record.h / 2),
    ));
    app.test_left_button(ElementState::Pressed);
    main_assert!(app.game_option_pointer_capture);
    app.resize(1024, 768).test_value();
    main_assert!(app.game_option_consumed_keys.is_empty());
    main_assert!(!app.game_option_pointer_capture);
    app.test_key(VirtualKeyCode::Space, ElementState::Released);
    app.test_left_button(ElementState::Released);
    main_assert!(!app.scenario_game_options.values().record);

    app.scenario_game_options = GameOptionButtons::new(
        GameOptionContext::NetworkHostSelector,
        GameOptionValues::default(),
    );
    app.sync_scenario_game_option_bounds();
    let actions = app.scenario_game_options.handle_hotkey('P');
    app.finish_game_option_input(actions).test_value();
    let input_layout = app.game_option_input_layout().test_value();
    let edit_point = PhysicalPosition::new(
        f64::from(input_layout.edit.x + input_layout.edit.w / 2),
        f64::from(input_layout.edit.y + input_layout.edit.h / 2),
    );
    app.test_cursor(edit_point);
    app.test_left_button(ElementState::Pressed);
    app.test_key(VirtualKeyCode::Tab, ElementState::Pressed);
    app.test_key(VirtualKeyCode::Tab, ElementState::Released);
    app.test_key(VirtualKeyCode::Enter, ElementState::Pressed);
    main_assert_eq!(app.game_option_input_pointer_capture => Some(ContextMenuPointerButton::Left));
    main_assert!(!app.game_option_input_consumed_keys.is_empty());
    main_assert!(app.game_option_input_pointer_position.is_some());
    app.resize(1280, 720).test_value();
    main_assert!(app.game_option_input_dialog.is_some());
    main_assert_eq!(app.game_option_input_pointer_capture => None);
    main_assert!(app.game_option_input_consumed_keys.is_empty());
    main_assert!(app.game_option_input_pointer_position.is_none());
    main_assert!(app.game_option_input_last_click.is_none());
    main_assert!(!app.game_option_pointer_capture);
    app.test_key(VirtualKeyCode::Enter, ElementState::Released);
    app.test_left_button(ElementState::Released);
    main_assert!(app.game_option_input_dialog.is_some());
    reset_cached_app_paths();
}

#[test]
fn takeover_submenu_lists_only_local_unissued_unassociated_players() {
    let mut app = new_menu_app(640, 480);
    install_test_free_savegame_player_row(&mut app, 50);
    let (network, _events, mut commands) = NetworkManager::test_stub_with_commands_for_client_id(7);
    app.network = Some(network);
    app.network_mode = Some(NetworkMode::Client(ClientSettings::new(
        SocketAddr::from(([127, 0, 0, 1], 11_112)),
        "Client",
    )));

    let eligible_league = clonk_engine::ControlPlayerInfoEntry {
        id: 11,
        name: LegacyCString::from_bytes(b"Raw A".to_vec()).test_value(),
        forced_name: LegacyCString::from_bytes(b"Forced A".to_vec()).test_value(),
        league_account: LegacyCString::from_bytes(b"League A".to_vec()).test_value(),
        ..Default::default()
    };
    let join_issued = clonk_engine::ControlPlayerInfoEntry {
        id: 12,
        name: LegacyCString::from_bytes(b"Issued".to_vec()).test_value(),
        flags: clonk_engine::PLAYER_INFO_FLAG_JOIN_ISSUED,
        ..Default::default()
    };
    let joined_and_removed = clonk_engine::ControlPlayerInfoEntry {
        id: 13,
        name: LegacyCString::from_bytes(b"Joined".to_vec()).test_value(),
        flags: clonk_engine::PLAYER_INFO_FLAG_JOINED | clonk_engine::PLAYER_INFO_FLAG_REMOVED,
        ..Default::default()
    };
    let associated = clonk_engine::ControlPlayerInfoEntry {
        id: 14,
        name: LegacyCString::from_bytes(b"Associated".to_vec()).test_value(),
        savegame_player: 90,
        ..Default::default()
    };
    let eligible_forced = clonk_engine::ControlPlayerInfoEntry {
        id: 15,
        name: LegacyCString::from_bytes(b"Raw B".to_vec()).test_value(),
        forced_name: LegacyCString::from_bytes(b"Forced B".to_vec()).test_value(),
        ..Default::default()
    };
    app.control_player_infos.replace_snapshot(
        99,
        [
            clonk_engine::PlayerInfoControlData::new(
                0,
                0,
                vec![menus1_fixture!(
                    player_info_id_name:
                        21,
                        LegacyCString::from_bytes(b"Foreign".to_vec()).unwrap(),
                )],
                -1,
            ),
            clonk_engine::PlayerInfoControlData::new(
                7,
                clonk_engine::CLIENT_PLAYER_INFO_FLAG_INITIAL,
                vec![
                    eligible_league,
                    join_issued,
                    joined_and_removed,
                    associated,
                    eligible_forced,
                ],
                7,
            ),
        ],
    );

    let entries = app.classic_lobby_takeover_entries(50);
    main_assert_eq!(entries.iter().map(|entry| entry.text.as_str()).collect::<Vec<_>>() => vec!["Using League A", "Using Forced B"]);
    main_assert_eq!(
        entries
            .iter()
            .map(|entry| entry.tooltip.as_deref())
            .collect::<Vec<_>>() =>
        vec![
            Some("Use this player to continue the savegame"),
            Some("Use this player to continue the savegame"),
        ]
    );
    main_assert!(entries.iter().all(|entry| entry.icon == ContextMenuIcon::Phase(9)));
    main_assert_eq!(
        entries
            .iter()
            .map(|entry| entry.action.clone())
            .collect::<Vec<_>>() =>
        vec![
            Some(AppContextMenuCommand::LobbyPlayerTakeOver {
                savegame_player_id: 50,
                player_id: 11,
            }),
            Some(AppContextMenuCommand::LobbyPlayerTakeOver {
                savegame_player_id: 50,
                player_id: 15,
            }),
        ]
    );

    app.process_classic_lobby_actions(vec![
        menus1_fixture!(roster_context: LobbyRosterId::Player(50)),
    ])
    .test_value();
    let root = app.context_menu.as_ref().test_value().layout().panels[0].rows[0].rect;
    app.handle_context_menu_pointer_move(GuiPoint::new((root.x + 1) as f32, (root.y + 1) as f32))
        .test_value();
    let layout = app.context_menu.as_ref().test_value().layout();
    main_assert_eq!(layout.panels.len() => 2);
    main_assert_eq!(layout.panels[1].rows.len() => 2);

    let mut rows = app
        .classic_host_lobby
        .as_ref()
        .test_value()
        .controller
        .rows()
        .to_vec();
    let LobbyRosterRow::Header(header) = &mut rows[0] else {
        panic!("free savegame group header");
    };
    header.kind = LobbyRosterHeader::ReplayPlayers;
    app.classic_host_lobby
        .as_mut()
        .test_value()
        .controller
        .set_rows(rows);
    app.close_stale_classic_lobby_team_combo();
    main_assert!(app.context_menu.is_none(), "regrouping the target as a replay player closes a stale takeover menu");
    app.take_over_classic_lobby_savegame_player(50, 11);
    main_assert!(commands.take_player_info_updates().is_empty(), "the activation guard rejects a replay target even when invoked directly");
}

// Every visible C4MainMenu string goes through LoadResStr against the
// active table at page-construction time (C4MainMenu.cpp:59-732;
// C4Player.cpp:1801), so a language change reaches the menus through the
// next Activate*/refill rather than being frozen at compile time.
#[test]
fn ingame_menu_uses_active_language_resources_for_all_pages() {
    use clonk_app_menus::ingame_menu::{
        DisplayFlags, GoalRuleEntry, HostDisconnectClientEntry, HostilityEntry, NewPlayerEntry,
        ObserverPlayerEntry, ObserverTarget, OptionFlags, SaveSlotState, TeamSelectionEntry,
        UpperBoardMode,
    };

    let mut app = new_running_sandbox_app();
    // A key absent from the table keeps its shipped LanguageUS.txt value,
    // which is exactly what C4ResStrTable falls back to.
    main_assert_eq!(app.ingame_menu_labels().goals => IngameMenuLabels::default().goals);

    for (key, value) in [
        ("IDS_MENU_CPMAIN", "[Spielermenü]"),
        ("IDS_MENU_OBSERVER", "[Zuschauermenü]"),
        ("IDS_MENU_CPGOALS", "[Ziele]"),
        ("IDS_MENU_CPGOALSINFO", "[Zielinfo]"),
        ("IDS_MENU_CPRULES", "[Regeln]"),
        ("IDS_MENU_CPRULESINFO", "[Regelinfo]"),
        ("IDS_TEXT_VIEW", "[Ansicht]"),
        ("IDS_TEXT_DETERMINEPLAYERVIEWTOFOLL", "[Ansichtinfo]"),
        ("IDS_MENU_CPATTACK", "[Angriff]"),
        ("IDS_MENU_CPATTACKINFO", "[Angriffinfo]"),
        ("IDS_MSG_SELTEAM", "[Team wählen]"),
        ("IDS_MSG_ALLOWSYOUTOJOINADIFFERENT", "[Teaminfo]"),
        ("IDS_MSG_JOINTEAM", "[Team %s beitreten]"),
        ("IDS_MENU_CPNEWPLAYER", "[Spieler beitreten]"),
        ("IDS_MENU_CPNEWPLAYERINFO", "[Beitrittinfo]"),
        ("IDS_MENU_NEWPLAYER", "[Beitritt: %s]"),
        ("IDS_MENU_NOPLRFILES", "[Keine Spielerdateien]"),
        ("IDS_MENU_CPSAVEGAME", "[Speichern]"),
        ("IDS_MENU_CPSAVEGAMEINFO", "[Speicherinfo]"),
        ("IDS_MNU_OPTIONS", "[Optionen]"),
        ("IDS_MNU_OPTIONSINFO", "[Optioneninfo]"),
        ("IDS_MENU_DISCONNECT", "[Trennen]"),
        ("IDS_TEXT_KICKCERTAINCLIENTSFROMTHE", "[Kickinfo]"),
        ("IDS_TEXT_DISCONNECTTHEGAMEFROMTHES", "[Trenninfo]"),
        ("IDS_MENU_DISCONNECTCLIENT", "[Client trennen]"),
        ("IDS_MENU_DISCONNECTFROMSERVER", "[Vom Host trennen?]"),
        ("IDS_MENU_CPSURRENDER", "[Aufgeben]"),
        ("IDS_MENU_CPSURRENDERINFO", "[Aufgabeinfo]"),
        ("IDS_MENU_SURRENDER", "[Sicher?]"),
        ("IDS_MENU_ABORT", "[Abbrechen]"),
        ("IDS_MENU_ABORT_DESC", "[Abbruchinfo]"),
        ("IDS_MENU_ATTACK", "[%s angreifen]"),
        ("IDS_MENU_NOATTACK", "[%s nicht angreifen]"),
        ("IDS_MENU_ATTACKHOSTILE", "[feindlich] "),
        ("IDS_MENU_ATTACKFRIENDLY", "[freundlich] "),
        ("IDS_MENU_ATTACKNOT", "[nicht] "),
        ("IDS_MENU_ATTACKINFO", "[%s ist %sund wird %sangegriffen]"),
        ("IDS_MSG_FREEVIEW", "[Freie Sicht]"),
        ("IDS_MSG_FREELYSCROLLAROUNDTHEMAP", "[Sichtinfo]"),
        ("IDS_TEXT_FOLLOWVIEWOFPLAYER", "[Folge %s]"),
        ("IDS_DLG_SOUND", "[Klang]"),
        ("IDS_MNU_MUSIC", "[Musik]"),
        ("IDS_MNU_MOUSECONTROL", "[Maussteuerung]"),
        ("IDS_MENU_DISPLAY", "[Anzeige]"),
        ("IDS_MNU_PLAYERNAMES", "[Spielernamen]"),
        ("IDS_MENU_PLAYERNAMES_DESC", "[Spielernameninfo]"),
        ("IDS_MNU_CLONKNAMES", "[Clonknamen]"),
        ("IDS_MENU_CLONKNAMES_DESC", "[Clonknameninfo]"),
        ("IDS_MNU_PORTRAITS", "[Portraits]"),
        ("IDS_MENU_SHOWCOMMANDS", "[Befehle]"),
        ("IDS_MENU_SHOWCOMMANDKEYS", "[Tasten]"),
        ("IDS_MNU_UPPERBOARD", "[Titelleiste]"),
        ("IDS_MNU_UPPERBOARD_OFF", "[Aus]"),
        ("IDS_MNU_UPPERBOARD_NORMAL", "[Normal]"),
        ("IDS_MNU_UPPERBOARD_SMALL", "[Klein]"),
        ("IDS_MNU_UPPERBOARD_MINI", "[Minimal unten]"),
        ("IDS_MNU_FPS", "[Bildrate]"),
        ("IDS_MNU_CLOCK", "[Uhr]"),
        ("IDS_MNU_WHITECHAT", "[Weisser Chat]"),
        ("IDS_DESC_WHITECHAT_INGAME", "[Chatinfo]"),
        ("IDS_BTN_YES", "[Ja]"),
        ("IDS_BTN_NO", "[Nein]"),
    ] {
        app.startup_tooltip_resources
            .insert(key.to_string(), value.to_string());
    }
    let labels = app.ingame_menu_labels();

    let captions = |menu: &IngameMenuState| {
        menu.items()
            .iter()
            .map(|item| item.caption.clone())
            .collect::<Vec<_>>()
    };
    let tooltips = |menu: &IngameMenuState| {
        menu.items()
            .iter()
            .filter_map(|item| item.info_caption.clone())
            .collect::<Vec<_>>()
    };

    let main = IngameMenuState::main_menu(
        &MainMenuConditions {
            has_player: true,
            player_count: 2,
            max_players: 4,
            team_switch_allowed: true,
            network_enabled: true,
            network_host: true,
            network_has_clients: true,
            is_fullscreen: true,
            ..MainMenuConditions::default()
        },
        &labels,
    )
    .test_value();
    main_assert_eq!(main.caption() => "[Spielermenü]");
    main_assert_eq!(
        captions(&main) =>
        [
            "[Ziele]",
            "[Regeln]",
            "[Angriff]",
            "[Team wählen]",
            "[Spieler beitreten]",
            "[Speichern]",
            "[Optionen]",
            "[Trennen]",
            "[Aufgeben]",
            "[Abbrechen]",
        ]
    );
    main_assert_eq!(
        tooltips(&main) =>
        [
            "[Zielinfo]",
            "[Regelinfo]",
            "[Angriffinfo]",
            "[Teaminfo]",
            "[Beitrittinfo]",
            "[Speicherinfo]",
            "[Optioneninfo]",
            "[Kickinfo]",
            "[Aufgabeinfo]",
            "[Abbruchinfo]",
        ]
    );
    let observer = IngameMenuState::main_menu(
        &MainMenuConditions {
            has_player: false,
            network_enabled: true,
            ..MainMenuConditions::default()
        },
        &labels,
    )
    .test_value();
    main_assert_eq!(observer.caption() => "[Zuschauermenü]");
    main_assert_eq!(captions(&observer) => ["[Ansicht]", "[Spieler beitreten]", "[Optionen]", "[Trennen]", "[Abbrechen]",]);

    // IDS_MENU_ATTACK/_NOATTACK and IDS_MENU_ATTACKINFO, whose hostile,
    // friendly and not fragments each carry their own trailing space.
    let hostility = IngameMenuState::hostility_menu(
        &[
            HostilityEntry {
                opponent: 1,
                name: "Ada".to_string(),
                hostile: true,
                opponent_hostile: true,
            },
            HostilityEntry {
                opponent: 2,
                name: "Bo".to_string(),
                hostile: false,
                opponent_hostile: false,
            },
        ],
        &labels,
    );
    main_assert_eq!(hostility.items()[0].caption => "[Ada angreifen]");
    main_assert_eq!(hostility.items()[1].caption => "[Bo nicht angreifen]");
    main_assert_eq!(tooltips(&hostility) => ["[Ada ist [feindlich] und wird angegriffen]", "[Bo ist [freundlich] und wird [nicht] angegriffen]",]);

    let observer_page = IngameMenuState::observer_menu(
        &[ObserverPlayerEntry {
            id: 3,
            name: "Cid".to_string(),
        }],
        ObserverTarget::Free,
        &labels,
    );
    main_assert_eq!(captions(&observer_page)[0] => "[Freie Sicht]");
    main_assert_eq!(tooltips(&observer_page) => ["[Sichtinfo]", "[Folge Cid]"]);

    let options = IngameMenuState::options_menu(
        &OptionFlags {
            sound: false,
            music: false,
            mouse_shown: true,
            mouse: false,
        },
        0,
        &labels,
    );
    main_assert_eq!(captions(&options) => ["[Klang]", "[Musik]", "[Maussteuerung]", "[Anzeige]"]);

    let display = IngameMenuState::display_menu(
        &DisplayFlags {
            is_fullscreen: true,
            upper_board: UpperBoardMode::Small,
            ..DisplayFlags::default()
        },
        0,
        &labels,
    );
    main_assert_eq!(
        captions(&display) =>
        [
            "[Spielernamen]",
            "[Clonknamen]",
            "[Portraits]",
            "[Befehle]",
            "[Tasten]",
            "[Titelleiste]: [Klein]",
            "[Bildrate]",
            "[Uhr]",
            "[Weisser Chat]",
        ]
    );
    main_assert_eq!(tooltips(&display) => ["[Spielernameninfo]", "[Clonknameninfo]", "[Chatinfo]",]);

    let teams = [TeamSelectionEntry {
        id: 4,
        caption: "Alpha".to_string(),
        icon_spec: None,
        color: 0,
        has_participants: false,
    }];
    let switch = IngameMenuState::team_switch_menu(&teams, &labels);
    main_assert_eq!(tooltips(&switch) => ["[Team Alpha beitreten]"]);

    let savegame = IngameMenuState::savegame_menu(&[SaveSlotState { free: true }; 10], &labels);
    main_assert_eq!(captions(&savegame)[0] => "[Speichern]");
    main_assert_eq!(tooltips(&savegame)[0] => "[Speicherinfo]");

    let surrender = IngameMenuState::surrender_menu(&labels);
    main_assert_eq!(surrender.caption() => "[Sicher?]");
    main_assert_eq!(captions(&surrender) => ["[Ja]", "[Nein]"]);

    let part = IngameMenuState::client_disconnect_menu(&labels);
    main_assert_eq!(part.caption() => "[Vom Host trennen?]");
    main_assert_eq!(captions(&part) => ["[Ja]", "[Nein]"]);

    let kick = IngameMenuState::host_disconnect_menu(
        &[HostDisconnectClientEntry {
            client_id: 1,
            caption: "Remote (Nick)".to_string(),
            activated: true,
        }],
        &labels,
    );
    main_assert_eq!(kick.caption() => "[Client trennen]");

    let goal = menus1_fixture!(goal_rule: "GOAL".to_string(), "Settle".to_string());
    main_assert_eq!(IngameMenuState::goals_menu(std::slice::from_ref(&goal), &labels).caption() => "[Ziele]");
    main_assert_eq!(IngameMenuState::rules_menu(std::slice::from_ref(&goal), &labels).caption() => "[Regeln]");

    let new_player = IngameMenuState::new_player_menu(
        &[NewPlayerEntry {
            name: "Clonko".to_string(),
            file: "Clonko.c4p".to_string(),
        }],
        &labels,
    );
    main_assert_eq!(new_player.caption() => "[Keine Spielerdateien]");
    main_assert_eq!(captions(&new_player) => ["[Beitritt: Clonko]"]);
}

#[test]
fn takeover_submenu_fills_live_at_open() {
    let mut app = new_menu_app(640, 480);
    install_test_free_savegame_player_row(&mut app, 50);
    let (network, _events, mut commands) = NetworkManager::test_stub_with_commands_for_client_id(7);
    app.network = Some(network);
    app.network_mode = Some(NetworkMode::Client(ClientSettings::new(
        SocketAddr::from(([127, 0, 0, 1], 11_112)),
        "Client",
    )));

    let first = menus1_fixture!(
        player_info_id_name:
            11,
            LegacyCString::from_bytes(b"First".to_vec()).test_value(),
    );
    let second = menus1_fixture!(
        player_info_id_name:
            12,
            LegacyCString::from_bytes(b"Second".to_vec()).test_value(),
    );
    let packet_flags = clonk_engine::CLIENT_PLAYER_INFO_FLAG_INITIAL;
    let local_packet = |players: Vec<clonk_engine::ControlPlayerInfoEntry>| {
        clonk_engine::PlayerInfoControlData::new(7, packet_flags, players, 7)
    };
    app.control_player_infos
        .replace_snapshot(99, [local_packet(vec![first.clone()])]);

    app.process_classic_lobby_actions(vec![
        menus1_fixture!(roster_context: LobbyRosterId::Player(50)),
    ])
    .test_value();
    main_assert_eq!(app.context_menu.as_ref().unwrap().layout().panels.len() => 1, "the Take Over child panel does not exist at root-menu open");

    // A player-info update arrives while the root menu is open. C++
    // fills the children in OnContextTakeOver only at submenu-open
    // (src/C4PlayerInfoListBox.cpp:503-505,535-556), so the submenu must
    // reflect this update rather than a root-open snapshot.
    app.control_player_infos
        .replace_snapshot(100, [local_packet(vec![first.clone(), second.clone()])]);

    let root = app.context_menu.as_ref().test_value().layout().panels[0].rows[0].rect;
    app.handle_context_menu_pointer_move(GuiPoint::new((root.x + 1) as f32, (root.y + 1) as f32))
        .test_value();
    let layout = app.context_menu.as_ref().test_value().layout();
    main_assert_eq!(layout.panels.len() => 2);
    main_assert_eq!(layout.panels[1].rows.len() => 2, "children are computed from the live packet at submenu-open");

    // Closing the child and re-selecting the parent re-runs the fill
    // callback, so a candidate that issued its join meanwhile drops out.
    app.handle_context_menu_key(VirtualKeyCode::ArrowLeft, ElementState::Pressed)
        .test_value();
    main_assert_eq!(app.context_menu.as_ref().unwrap().layout().panels.len() => 1);
    let mut issued_first = first.clone();
    issued_first.flags |= clonk_engine::PLAYER_INFO_FLAG_JOIN_ISSUED;
    app.control_player_infos
        .replace_snapshot(101, [local_packet(vec![issued_first, second.clone()])]);
    app.handle_context_menu_key(VirtualKeyCode::ArrowRight, ElementState::Pressed)
        .test_value();
    let layout = app.context_menu.as_ref().test_value().layout();
    main_assert_eq!(layout.panels.len() => 2);
    main_assert_eq!(layout.panels[1].rows.len() => 1, "a re-open refills from the live packet like C++");

    // The surviving child is the live-eligible player and activates the
    // exact live association.
    let child = app.context_menu.as_ref().test_value().layout().panels[1].rows[0].rect;
    app.handle_context_menu_pointer_move(GuiPoint::new((child.x + 1) as f32, (child.y + 1) as f32))
        .test_value();
    main_assert!(app.handle_context_menu_pointer_button(ElementState::Pressed, ContextMenuPointerButton::Left,).expect("activate live takeover child"));
    let updates = commands.take_player_info_updates();
    main_assert_eq!(updates.len() => 1);
    main_assert_eq!(
        updates[0]
            .players
            .iter()
            .map(|player| (player.id, player.savegame_player))
            .collect::<Vec<_>>() =>
        vec![(11, 0), (12, 50)],
        "the activation grabs the live-eligible player only"
    );
    main_assert!(app.handle_context_menu_pointer_button(ElementState::Released, ContextMenuPointerButton::Left,).expect("consume takeover activation release"));
}

#[test]
fn player_context_root_matches_cpp_entry_gates() {
    let mut app = new_menu_app(640, 480);
    let (mut chooser, _) = install_test_classic_host_team_lobby(&mut app);
    chooser.color = 0x00ab_cdef;
    let associated_script = clonk_engine::ControlPlayerInfoEntry {
        id: 9,
        player_type: clonk_engine::PLAYER_INFO_TYPE_SCRIPT,
        savegame_player: 90,
        color: 0x0012_3456,
        original_color: 0x0065_4321,
        ..Default::default()
    };
    let replay_player = clonk_engine::ControlPlayerInfoEntry {
        id: 51,
        name: LegacyCString::from_bytes(b"Replay".to_vec()).test_value(),
        color: 0x0012_3456,
        original_color: 0x0065_4321,
        ..Default::default()
    };
    let free_script = clonk_engine::ControlPlayerInfoEntry {
        id: 52,
        name: LegacyCString::from_bytes(b"Free script".to_vec()).test_value(),
        player_type: clonk_engine::PLAYER_INFO_TYPE_SCRIPT,
        ..Default::default()
    };
    app.control_player_infos.replace_snapshot(
        51,
        [
            clonk_engine::PlayerInfoControlData::new(
                0,
                clonk_engine::CLIENT_PLAYER_INFO_FLAG_INITIAL,
                vec![chooser.clone()],
                0,
            ),
            clonk_engine::PlayerInfoControlData::new(
                7,
                clonk_engine::CLIENT_PLAYER_INFO_FLAG_INITIAL,
                vec![associated_script.clone()],
                0,
            ),
            clonk_engine::PlayerInfoControlData::new(
                8,
                clonk_engine::CLIENT_PLAYER_INFO_FLAG_INITIAL,
                vec![replay_player],
                0,
            ),
        ],
    );
    let mut host_snapshot = clonk_network::HostConfig::default()
        .initial_join_snapshot
        .test_value();
    host_snapshot.parameters.restore_player_infos = clonk_network::PlayerInfoListSnapshot {
        last_player_id: 52,
        clients: vec![clonk_network::ClientPlayerInfosSnapshot {
            client_id: 0,
            flags: 0,
            players: vec![free_script],
        }],
    };
    app.host_join_snapshot = Some(host_snapshot);
    let player_row = |id, client_id, name: &str, team| {
        LobbyRosterRow::Player(LobbyPlayerRow {
            id,
            client_id,
            name: name.to_string(),
            color: [0xff; 4],
            icon: LobbyRosterIcon::Standard(7),
            joined_player_overlay: None,
            team: Some(LobbyTeamValue {
                id: team,
                name: format!("Team {team}"),
                selectable: false,
            }),
            league_score: None,
            league_rank: None,
        })
    };
    let client_row = app
        .classic_host_lobby
        .as_ref()
        .test_value()
        .controller
        .rows()[0]
        .clone();
    app.classic_host_lobby
        .as_mut()
        .test_value()
        .controller
        .set_rows(vec![
            LobbyRosterRow::Header(LobbyHeaderRow {
                kind: LobbyRosterHeader::UnassignedSavegamePlayers,
                label: "Player assignment".to_string(),
                icon: LobbyRosterIcon::Standard(12),
                can_add_player: false,
            }),
            player_row(50, -1, "Free restore", 0),
            player_row(52, -1, "Free script", 0),
            LobbyRosterRow::Header(LobbyHeaderRow {
                kind: LobbyRosterHeader::ReplayPlayers,
                label: "Replay players".to_string(),
                icon: LobbyRosterIcon::Standard(21),
                can_add_player: false,
            }),
            player_row(51, -1, "Replay", 0),
            client_row,
            player_row(7, 0, "Chooser", 1),
            player_row(9, 7, "Script", 0),
        ]);
    app.network_mode = Some(NetworkMode::Host(HostSettings {
        bind_addr: SocketAddr::from(([127, 0, 0, 1], 0)),
        player_name: "Host".to_string(),
        prepared: None,
    }));

    let (_, free) = app.classic_lobby_player_context_entries(50).test_value();
    main_assert_eq!(free.len() => 1);
    main_assert_eq!(free[0].text => "<c ffffff7f>T</c>ake over");
    main_assert_eq!(free[0].tooltip.as_deref() => Some("Control the player in the game"));
    main_assert_eq!(free[0].icon => ContextMenuIcon::Phase(9));
    main_assert_eq!(free[0].hotkey => Some('T'));
    main_assert_eq!(free[0].action => None);
    main_assert!(free[0].has_submenu());
    main_assert!(app.classic_lobby_player_context_entries(52).expect("visible free script row").1.is_empty(), "native free script rows omit Take Over");
    let (_, replay) = app.classic_lobby_player_context_entries(51).test_value();
    main_assert_eq!(
        replay
            .iter()
            .map(|entry| entry.action.clone())
            .collect::<Vec<_>>() =>
        vec![
            Some(AppContextMenuCommand::LobbyPlayerRemove {
                client_id: -1,
                player_id: 51,
            }),
            Some(AppContextMenuCommand::LobbyPlayerNewColor {
                client_id: -1,
                player_id: 51,
            }),
        ],
        "a replay player has native ordinary entries, not free-savegame Take Over"
    );
    app.process_classic_lobby_actions(vec![
        menus1_fixture!(roster_context: LobbyRosterId::Player(51)),
    ])
    .test_value();
    app.close_stale_classic_lobby_team_combo();
    main_assert!(app.context_menu.is_some(), "an unchanged replay group keeps its ordinary context menu");
    main_assert_eq!(app.context_menu_lobby_player => Some((-1, 51, false)));
    app.close_context_menu_silently();
    app.process_classic_lobby_actions(vec![
        menus1_fixture!(roster_context: LobbyRosterId::Player(50)),
    ])
    .test_value();
    main_assert_eq!(app.context_menu.as_ref().unwrap().layout().panels[0].rows.len() => 1);
    app.close_context_menu_silently();

    let (_, ordinary) = app.classic_lobby_player_context_entries(7).test_value();
    main_assert_eq!(ordinary.len() => 2);
    main_assert_eq!(ordinary[0].text => "<c ffffff7f>R</c>emove");
    main_assert_eq!(ordinary[0].tooltip.as_deref() => Some("Do not join with this player"));
    main_assert_eq!(ordinary[0].icon => ContextMenuIcon::Phase(34));
    main_assert_eq!(ordinary[0].hotkey => Some('R'));
    main_assert_eq!(ordinary[0].action => Some(AppContextMenuCommand::LobbyPlayerRemove {client_id: 0, player_id: 7,}));
    main_assert_eq!(ordinary[1].text => "New <c ffffff7f>c</c>olor");
    main_assert_eq!(ordinary[1].tooltip.as_deref() => Some("Generate a new random player color"));
    main_assert_eq!(ordinary[1].icon => ContextMenuIcon::Phase(9));
    main_assert_eq!(ordinary[1].hotkey => Some('C'));
    main_assert_eq!(ordinary[1].action => Some(AppContextMenuCommand::LobbyPlayerNewColor {client_id: 0, player_id: 7,}));

    app.network_team_assignment
        .as_mut()
        .test_value()
        .teams_mut()
        .team_colors = true;
    let (_, ordinary) = app.classic_lobby_player_context_entries(7).test_value();
    main_assert_eq!(ordinary.len() => 1, "a nonzero team color suppresses reroll");
    let (_, script) = app.classic_lobby_player_context_entries(9).test_value();
    main_assert_eq!(script.len() => 1, "association suppresses only Remove");
    main_assert_eq!(
        script[0].action =>
        Some(AppContextMenuCommand::LobbyPlayerNewColor {
            client_id: 7,
            player_id: 9,
        }),
        "team zero retains New Color even with team colors enabled"
    );

    app.network_mode = None;
    app.control_clients
        .replace_snapshot([message_client(0, b"Remote owner")]);
    let (_, foreign) = app.classic_lobby_player_context_entries(7).test_value();
    main_assert!(foreign.is_empty());
    app.process_classic_lobby_actions(vec![
        menus1_fixture!(roster_context: LobbyRosterId::Player(7)),
    ])
    .test_value();
    main_assert!(app.context_menu.is_some());
    main_assert_eq!(app.context_menu_lobby_player => Some((0, 7, false)));
}

#[test]
fn context_menu_matches_edit_predicates_and_order() {
    let view = LobbyChatEditView {
        text: "selected text".into(),
        caret: 8,
        selection: Some((0, 8)),
        ..LobbyChatEditView::default()
    };
    let entries = lobby_chat_context_entries(&view, true);
    main_assert_eq!(entries.iter().map(|entry| entry.text.as_str()).collect::<Vec<_>>() => ["Cut", "Copy", "Paste", "Clear", "Select all"]);
    main_assert_eq!(
        entries
            .iter()
            .map(|entry| entry.action.clone())
            .collect::<Vec<_>>() =>
        [
            Some(AppContextMenuCommand::LobbyChat(
                LobbyChatContextCommand::Cut,
            )),
            Some(AppContextMenuCommand::LobbyChat(
                LobbyChatContextCommand::Copy,
            )),
            Some(AppContextMenuCommand::LobbyChat(
                LobbyChatContextCommand::Paste,
            )),
            Some(AppContextMenuCommand::LobbyChat(
                LobbyChatContextCommand::Clear,
            )),
            Some(AppContextMenuCommand::LobbyChat(
                LobbyChatContextCommand::SelectAll,
            )),
        ]
    );

    let whole = LobbyChatEditView {
        text: "all".into(),
        caret: 3,
        selection: Some((0, 3)),
        ..LobbyChatEditView::default()
    };
    let entries = lobby_chat_context_entries(&whole, false);
    main_assert_eq!(entries.iter().map(|entry| entry.text.as_str()).collect::<Vec<_>>() => ["Cut", "Copy", "Clear"]);

    main_assert!(lobby_chat_context_entries(&LobbyChatEditView::default(), false).is_empty());
}

#[test]
fn classic_context_menu_dispatches_to_the_live_edit() {
    let mut app = new_menu_app(640, 480);
    install_test_classic_host_lobby(&mut app);
    app.classic_host_lobby
        .test_mut()
        .controller
        .set_chat_edit_view(LobbyChatEditView {
            text: "select me".into(),
            caret: 9,
            ..LobbyChatEditView::default()
        });

    app.process_classic_lobby_chat_request(LobbyChatRequest::OpenContextMenu {
        anchor: GuiPoint::new(20.0, 20.0),
    })
    .test_value();
    main_assert!(app.context_menu.is_some());
    app.process_context_menu_outcome(ContextMenuOutcome {
        captured: true,
        pass_through: false,
        focus_suppressed: true,
        events: vec![
            ContextMenuEvent::Closed,
            ContextMenuEvent::Activated(AppContextMenuCommand::LobbyChat(
                LobbyChatContextCommand::SelectAll,
            )),
        ],
    })
    .test_value();
    let view = app
        .classic_host_lobby
        .test_ref()
        .controller
        .chat_edit_view();
    main_assert_eq!(view.selection => Some((0, view.text.len())));
}

#[test]
fn return_to_menu_recreates_music_before_teardown_fade_finishes_like_cpp() {
    clonk_logging::init();
    main_assert_eq!(GAME_MUSIC_FADE_OUT_MS => 2_000);

    // Music discovery reads process env; hold the env lock so the
    // EnvGuard-based tests cannot redirect paths mid-load.
    let _lock = env_lock().lock();
    let user_data = tempdir();
    let (_guard, paths) = exact_loader_test_paths(user_data.path(), None);
    let mut app = test_game_app(320, 200, AudioOptions::default(), Some(&paths)).test_value();

    let fixture = app
        .audio
        .test_ref()
        .system
        .load_music(&silent_pcm_wav(20))
        .test_value();
    app.audio.test_mut().control_music_loads_with(fixture);

    // Menu music is started by `ensure_menu_music()` when asynchronous boot
    // loading completes and the menu is shown; pump boot to that point first.
    wait_for_menu(&mut app);
    let audio = app.audio.test_ref();
    let controlled = audio.controlled_music_loads.test_ref();
    main_assert_eq!(controlled.requests.len() => 1);
    let frontend = controlled.requests.front().test_value();
    main_assert!(!frontend.looped, "frontend music is non-looping");
    main_assert!(frontend.identity.is_some(), "frontend music came from the catalog");
    main_assert_eq!(audio.music_resolver.playlist.as_deref() => Some("Frontend.*"));
    main_assert!(!audio.system.music_is_playing());
    main_assert!(app.audio.as_mut().expect("test audio").complete_next_controlled_music_load().expect("complete frontend music load"));
    main_assert!(app.audio.as_ref().expect("test audio").system.music_is_playing());

    app.start_sandbox_scenario(FrontendScenario::fallback())
        .test_value();
    let audio = app.audio.test_ref();
    let controlled = audio.controlled_music_loads.test_ref();
    main_assert_eq!(controlled.requests.len() => 1);
    let sandbox = controlled.requests.front().test_value();
    main_assert!(sandbox.looped, "sandbox music is looping");
    main_assert!(sandbox.identity.is_none(), "sandbox uses the direct music asset");
    main_assert_eq!(audio.music_resolver.playlist => None);
    main_assert!(!audio.system.music_is_playing());
    main_assert!(app.audio.as_mut().expect("test audio").complete_next_controlled_music_load().expect("complete sandbox music load"));
    app.return_to_menu();
    let audio = app.audio.test_ref();
    main_assert!(!audio.system.music_is_playing(), "PreInit reconstruction hard-stops the fading game song");
    main_assert!(!app.resume_frontend_music_after_fade);
    main_assert_eq!(audio.music_fade_requests => [GAME_MUSIC_FADE_OUT_MS], "Game.Clear still requests its 2s fade before PreInit cancels it");
    let controlled = audio.controlled_music_loads.test_ref();
    main_assert_eq!(controlled.requests.len() => 1);
    let frontend = controlled.requests.front().test_value();
    main_assert!(!frontend.looped, "returned frontend music is non-looping");
    main_assert!(frontend.identity.is_some(), "returned music came from the catalog");
    main_assert_eq!(audio.music_resolver.playlist.as_deref() => Some("Frontend.*"));
    main_assert!(app.audio.as_mut().expect("test audio").complete_next_controlled_music_load().expect("complete returned frontend music load"));
    let audio = app.audio.test_ref();
    main_assert!(audio.system.music_is_playing());
    main_assert_eq!(audio.music_load_pending.load(AtomicOrdering::Acquire) => 0);
    main_assert!(audio.controlled_music_loads.as_ref().expect("controlled music loading").requests.is_empty());

    // Restart/Next Mission also reconstructs at PreInit, but skips
    // C4Startup::DoStartup and therefore must not enqueue Frontend.*.
    app.start_sandbox_scenario(FrontendScenario::fallback())
        .test_value();
    main_assert!(app.audio.as_mut().expect("test audio").complete_next_controlled_music_load().expect("complete relaunch source music"));
    app.audio.test_mut().set_scenario_music_level(Some(25));
    app.return_to_menu_for_relaunch();
    let audio = app.audio.test_ref();
    main_assert!(!audio.system.music_is_playing());
    main_assert!(!app.resume_frontend_music_after_fade);
    main_assert_eq!(audio.music_fade_requests => [GAME_MUSIC_FADE_OUT_MS, GAME_MUSIC_FADE_OUT_MS], "each Game.Clear requests its fade before the next PreInit");
    main_assert!(lock_unpoisoned(&audio.music_control).most_recently_played.is_none(), "the direct-relaunch PreInit generation has no prior song identity");
    main_assert_eq!(lock_unpoisoned(&audio.music_control).scenario_level => None, "Game.Clear and the reconstructed music system discard scenario volume");
    main_assert!(audio.controlled_music_loads.as_ref().expect("controlled music loading").requests.is_empty());
}

#[test]
fn menu_cursor_moves_and_clears_on_leave() {
    let mut app = new_menu_app(64, 48);
    install_l018_cursor_atlas(&mut app);
    let background = Color::opaque(9, 10, 11);

    app.test_cursor(PhysicalPosition::new(20.0, 18.0));
    app.graphics.surface_mut().fill(background);
    main_assert!(app.draw_classic_gui_cursor(None));
    main_assert_eq!(app.graphics.surface().get_pixel(18, 16) => Some(Color::opaque(0, 40, 200)));

    app.test_cursor(PhysicalPosition::new(40.0, 30.0));
    app.graphics.surface_mut().fill(background);
    main_assert!(app.draw_classic_gui_cursor(None));
    main_assert_eq!(app.graphics.surface().get_pixel(18, 16) => Some(background));
    main_assert_eq!(app.graphics.surface().get_pixel(38, 28) => Some(Color::opaque(0, 40, 200)));

    app.pointer_left().test_value();
    main_assert!(!app.draw_classic_gui_cursor(None));
}

#[test]
fn loading_dialog_renders_gui_cursor_between_body_and_tooltip_passes() {
    let mut app = new_menu_app(320, 200);
    install_l018_cursor_atlas(&mut app);
    let fonts = app.assets.clonk_fonts.clone().test_value();
    app.loader_screen = Some(
        LoaderScreen::new(
            LoaderSelection::startup("LoaderSynthetic.png")
                .expect("valid synthetic loader selection"),
            ImageData::new(1, 1, vec![7, 8, 9, 255]),
            LoaderResources::new(fonts, ImageData::new(3, 1, vec![255; 12]))
                .expect("valid synthetic loader resources"),
            LoaderState::initial("Loading"),
        )
        .test_value(),
    );
    app.loader_error = None;
    app.loader_render_error = None;
    app.mode = AppMode::Loading;
    app.push_message_dialog(
        clonk_frontend::message_dialog::MessageDialogState::regular_ok(
            "Wait",
            "Loading",
            clonk_frontend::message_dialog::MessageDialogIcon::NOTIFY,
        ),
        MessageDialogContinuation::None,
    )
    .test_value();
    app.test_cursor(PhysicalPosition::new(20.0, 18.0));

    let mut frame = vec![0_u8; 320 * 200 * 4];
    app.test_render(&mut frame);
    let cursor_pixel = ((16 * 320 + 18) * 4) as usize;
    main_assert_eq!(&frame[cursor_pixel..cursor_pixel + 4] => &[1, 40, 200, 255], "standard C4 gamma raises the Region cell's zero channel to one");
}

#[test]
fn running_gui_ownership_matches_cpp_reset_and_dialog_lifetime() {
    let mut app = new_synthetic_running_sandbox_app();
    install_l018_cursor_atlas(&mut app);
    let (width, height) = {
        let surface = app.graphics.surface();
        (surface.width(), surface.height())
    };
    let mut frame = vec![0_u8; width as usize * height as usize * 4];
    app.test_render(&mut frame);
    app.open_ingame_menu().test_value();
    let menu_point = (0..height)
        .flat_map(|y| (0..width).map(move |x| GuiPoint::new(x as f32, y as f32)))
        .find(|point| app.ingame_menu_pointer_target(*point).is_some())
        .test_value();
    app.test_cursor(PhysicalPosition::new(
        f64::from(menu_point.x),
        f64::from(menu_point.y),
    ));
    main_assert!(app.running_gui_mouse_owned);
    main_assert!(!app.running_world_mouse_owned);
    main_assert!(app.ingame_pointer.is_none());

    app.reset_ingame_mouse_control();
    main_assert!(app.running_gui_mouse_owned, "C4MouseControl reset must not deactivate C4GUI::CMouse");
    main_assert!(app.running_world_mouse_owned, "C4MouseControl::Default independently restores fMouseOwned");
    app.initialize_ingame_mouse_center().test_value();
    let reset_world_pointer = app.ingame_pointer.test_value();
    main_assert!(app.classic_gui_cursor_request().is_some(), "GUI cursor remains independently drawable after the reset");
    app.runtime_help_visible = true;
    app.close_ingame_menu_for_player(app.local_owner);
    main_assert!(app.running_gui_mouse_owned, "Dialog::Close leaves ownership for C4GraphicsSystem::Execute");
    app.reconcile_running_mouse_after_last_gui_close(false)
        .test_value();
    main_assert!(!app.running_gui_mouse_owned);
    main_assert!(app.running_world_mouse_owned);
    main_assert!(app.ingame_pointer.is_some(), "the independently reinitialized world pointer remains active");
    main_assert_eq!(app.ingame_pointer => Some(reset_world_pointer));

    app.runtime_help_visible = false;
    app.open_ingame_menu().test_value();
    app.test_cursor(PhysicalPosition::new(
        f64::from(menu_point.x),
        f64::from(menu_point.y),
    ));
    main_assert!(!app.running_world_mouse_owned);
    set_test_scenario_head_flags(&mut app, 1, 1);
    app.test_render(&mut frame);
    main_assert!(app.running_gui_mouse_owned, "a shown C4Menu remains a C4GUI owner when viewport pixels are suppressed");
    main_assert!(!app.running_world_mouse_owned);

    let non_cursor_menu_object = app
        .engine
        .spawn_test_object(SpawnConfig::new("CLNK").with_position(Vector2::new(40, 30)));
    install_test_cursor_menu(
        &mut app,
        non_cursor_menu_object,
        two_item_script_menu(non_cursor_menu_object),
    );
    main_assert_ne!(app.engine.crew_cursor(app.local_owner) => Some(non_cursor_menu_object));
    app.close_ingame_menu_for_player(app.local_owner);
    app.test_render(&mut frame);
    main_assert!(app.running_gui_mouse_owned);
    main_assert!(!app.running_world_mouse_owned);

    app.engine
        .apply_object_update(
            non_cursor_menu_object,
            ObjectUpdate {
                menu: Some(None),
                ..ObjectUpdate::default()
            },
        )
        .test_value();
    app.test_render(&mut frame);
    main_assert!(!app.running_gui_mouse_owned);
    main_assert!(app.running_world_mouse_owned);
    main_assert!(app.ingame_pointer.is_some());
}

#[test]
fn synthetic_classic_test_assets_satisfy_only_the_global_gui_guard() {
    let mut app = new_menu_app(320, 200);
    main_assert!(app.assets.require_classic_global_gui_bootstrap_resources(&HashMap::new()).is_ok());
    main_assert!(app.assets.require_classic_startup_bootstrap_resources().is_err());
    main_assert!(app.assets.require_classic_startup_main_resources().is_err());
    main_assert!(app.assets.require_classic_ingame_menu_resources().is_err());
    main_assert!(app.assets.require_classic_game_over_resources().is_err());
    main_assert!(Arc::get_mut(&mut app.assets).is_some(), "each app owns a mutable outer asset bundle");
}

#[test]
fn standalone_irc_entry_points_share_the_singleton_dialog_and_alt_c_toggles_it() {
    let mut lobby_app = new_real_classic_menu_app(640, 480);
    install_test_classic_host_lobby(&mut lobby_app);
    lobby_app
        .process_classic_lobby_actions(vec![ClassicLobbyAction::Chat(
            LobbyChatRequest::OpenExternalDialog,
        )])
        .test_value();
    main_assert!(lobby_app.classic_host_lobby.is_some());
    main_assert!(lobby_app.external_irc_dialog_visible);
    let dialog = lobby_app.external_irc_dialog.test_ref();
    main_assert_eq!(dialog.mode() => clonk_frontend::startup_netdlg::NetDlgMode::Chat);
    main_assert_eq!(dialog.chat_bounds_override() => Some(clonk_frontend::startup_netdlg::NetDlgController::standalone_chat_bounds(640, 480)));
    let first_dialog_ptr = std::ptr::from_ref(dialog);
    lobby_app
        .process_classic_lobby_actions(vec![ClassicLobbyAction::Chat(
            LobbyChatRequest::OpenExternalDialog,
        )])
        .test_value();
    main_assert!(lobby_app.external_irc_dialog_visible);
    main_assert_eq!(
        lobby_app
            .external_irc_dialog
            .as_ref()
            .map(std::ptr::from_ref) =>
        Some(first_dialog_ptr),
        "raising the singleton must preserve its UI-local controller state"
    );
    lobby_app.hide_external_irc_dialog();
    main_assert!(!lobby_app.external_irc_dialog_visible);
    main_assert!(lobby_app.external_irc_dialog.is_none());

    for modifiers in [
        ModifiersState::ALT,
        ModifiersState::ALT | ModifiersState::SUPER,
    ] {
        let mut runtime_app = new_classic_running_sandbox_app();
        runtime_app
            .bindings
            .rebind(ControlBindingId::Left, VirtualKeyCode::KeyC);
        runtime_app.test_modifiers(modifiers);
        runtime_app.menu_title_drag = Some(MenuTitleDrag::Ingame {
            player: runtime_app.local_owner,
            start_pointer: GuiPoint::new(20.0, 20.0),
            start_location: (40, 50),
        });
        runtime_app.test_key(VirtualKeyCode::KeyC, ElementState::Pressed);
        main_assert!(runtime_app.external_irc_dialog_visible);
        main_assert!(runtime_app.menu_title_drag.is_none(), "activating C4ChatDlg releases an obscured menu-title drag");
        runtime_app.test_cursor(PhysicalPosition::new(300.0, 200.0));
        runtime_app.test_left_button(ElementState::Released);
        main_assert!(runtime_app.external_irc_dialog_visible);

        runtime_app
            .engine
            .test_player_mut(runtime_app.local_owner)
            .control
            .pressed_coms = 1 << clonk_engine::COM_LEFT;
        runtime_app.test_key(VirtualKeyCode::KeyC, ElementState::Released);
        main_assert_ne!(
            runtime_app
                .engine
                .player(runtime_app.local_owner)
                .expect("local sandbox player")
                .control
                .pressed_coms
                & (1 << clonk_engine::COM_LEFT) =>
            0,
            "runtime IRC release must not leak to modifier-blind player control"
        );
        runtime_app.test_key(VirtualKeyCode::KeyC, ElementState::Pressed);
        main_assert!(!runtime_app.external_irc_dialog_visible);
    }

    let mut ignored_runtime = new_running_sandbox_app();
    for modifiers in [
        ModifiersState::empty(),
        ModifiersState::CONTROL,
        ModifiersState::SHIFT,
        ModifiersState::SUPER,
        ModifiersState::ALT | ModifiersState::CONTROL,
        ModifiersState::ALT | ModifiersState::SHIFT,
        ModifiersState::ALT | ModifiersState::CONTROL | ModifiersState::SHIFT,
    ] {
        ignored_runtime.test_modifiers(modifiers);
        main_assert!(!ignored_runtime.handle_runtime_irc_toggle_key(VirtualKeyCode::KeyC, ElementState::Pressed).expect("non-IRC chord is unhandled"));
        main_assert!(!ignored_runtime.external_irc_dialog_visible);
    }
}

#[test]
fn dialog_hotkeys_use_the_first_sdl_key_name_character() {
    for (key, expected) in [
        (VirtualKeyCode::KeyA, Some('A')),
        (VirtualKeyCode::Digit7, Some('7')),
        (VirtualKeyCode::Space, Some('S')),
        (VirtualKeyCode::ArrowUp, Some('U')),
        (VirtualKeyCode::ArrowLeft, Some('L')),
        (VirtualKeyCode::Enter, Some('R')),
        (VirtualKeyCode::Escape, Some('E')),
        (VirtualKeyCode::PageUp, Some('P')),
        (VirtualKeyCode::PrintScreen, Some('P')),
        (VirtualKeyCode::Numpad1, Some('K')),
        (VirtualKeyCode::ContextMenu, Some('A')),
        (VirtualKeyCode::BrowserBack, Some('A')),
        (VirtualKeyCode::Minus, None),
        (VirtualKeyCode::Quote, None),
        (VirtualKeyCode::IntlBackslash, None),
    ] {
        main_assert_eq!(startup_dialog_hotkey(key) => expected, "{key:?}");
    }
}

/// `Dialog::KeyHotkey` is registered for both `KEYS_Alt` and
/// `KEYS_Alt | KEYS_Shift` on every dialog, so the network browser's
/// advertised mnemonics activate exactly like the main menu's
/// (src/C4GuiDialogs.cpp:363-364,574-582; src/C4GuiButton.cpp:73-79).
#[test]
fn netdlg_alt_mnemonics_activate_visible_buttons() {
    let mut app = new_real_classic_menu_app(640, 480);
    app.open_network_game_dialog();
    main_assert_eq!(app.startup_view => StartupView::NetworkGame);
    let signup = app.startup_network_dialog.test_ref().masterserver_signup();

    // Alt+I toggles Internet; Alt+Shift+R toggles Record.
    app.test_modifiers(ModifiersState::ALT);
    app.test_key(VirtualKeyCode::KeyI, ElementState::Pressed);
    main_assert_eq!(app.startup_network_dialog.as_ref().expect("network dialog").masterserver_signup() => !signup);
    app.test_modifiers(ModifiersState::ALT | ModifiersState::SHIFT);
    app.test_key(VirtualKeyCode::KeyI, ElementState::Pressed);
    main_assert_eq!(app.startup_network_dialog.as_ref().expect("network dialog").masterserver_signup() => signup);

    // Alt+C reaches the Chat tab; there Refresh and Join are not drawn, so
    // their mnemonics are inert while New game still activates.
    app.test_modifiers(ModifiersState::ALT);
    app.test_key(VirtualKeyCode::KeyC, ElementState::Pressed);
    main_assert!(app.startup_network_dialog.as_ref().expect("network dialog").is_chat_mode());
    app.test_key(VirtualKeyCode::KeyD, ElementState::Pressed);
    app.test_key(VirtualKeyCode::KeyJ, ElementState::Pressed);
    main_assert_eq!(app.startup_view => StartupView::NetworkGame);
    app.test_key(VirtualKeyCode::KeyG, ElementState::Pressed);
    main_assert!(!app.startup_network_dialog.as_ref().expect("network dialog").is_chat_mode());

    // A covering modal owns the keyboard, so the dialog beneath is inert.
    app.handle_game_over().test_value();
    app.test_key(VirtualKeyCode::KeyN, ElementState::Pressed);
    main_assert_eq!(app.startup_view => StartupView::NetworkGame);
}

#[test]
fn startup_alt_mnemonics_route_before_plain_gui_keys_and_lower_owners() {
    let mut app = new_real_classic_menu_app(640, 480);

    app.test_modifiers(ModifiersState::CONTROL | ModifiersState::ALT);
    for key in [
        VirtualKeyCode::ArrowDown,
        VirtualKeyCode::Enter,
        VirtualKeyCode::Space,
        VirtualKeyCode::Escape,
    ] {
        app.test_key(key, ElementState::Pressed);
        app.test_key(key, ElementState::Released);
    }
    app.test_key(VirtualKeyCode::KeyA, ElementState::Pressed);
    main_assert_eq!(app.startup_view => StartupView::MainMenu);
    main_assert!(!app.exit_requested);

    app.test_modifiers(ModifiersState::ALT | ModifiersState::SHIFT);
    app.test_key(VirtualKeyCode::KeyA, ElementState::Pressed);
    main_assert_eq!(app.startup_view => StartupView::About);
    main_assert!(app.ui_sound_log.is_empty());
    app.show_main_menu();

    app.test_modifiers(ModifiersState::ALT);
    for key in [
        VirtualKeyCode::ArrowDown,
        VirtualKeyCode::Enter,
        VirtualKeyCode::Escape,
    ] {
        app.test_key(key, ElementState::Pressed);
        app.test_key(key, ElementState::Released);
    }
    main_assert_eq!(app.startup_view => StartupView::MainMenu);
    main_assert!(!app.exit_requested);

    app.test_modifiers(ModifiersState::empty());
    app.test_key(VirtualKeyCode::Enter, ElementState::Pressed);
    app.test_key(VirtualKeyCode::Enter, ElementState::Released);
    main_assert_eq!(app.startup_view => StartupView::ScenarioBrowser);
    main_assert!(app.ui_sound_log.iter().any(|sound| sound == "Click"));
    app.ui_sound_log.clear();
    app.show_main_menu();

    app.test_key(VirtualKeyCode::ArrowDown, ElementState::Pressed);
    app.test_key(VirtualKeyCode::ArrowDown, ElementState::Released);
    app.ui_sound_log.clear();
    app.test_modifiers(ModifiersState::ALT);
    app.test_key(VirtualKeyCode::Space, ElementState::Pressed);
    main_assert_eq!(app.startup_view => StartupView::ScenarioBrowser);
    main_assert!(!app.ui_sound_log.iter().any(|sound| sound == "Click"), "mnemonic dispatch must bypass the button Click sound: {:?}", app.ui_sound_log);

    app.show_main_menu();
    app.open_about_dialog();
    app.ui_sound_log.clear();
    app.test_key(VirtualKeyCode::ArrowLeft, ElementState::Pressed);
    main_assert_eq!(app.startup_about_dialog.as_ref().expect("About dialog").current_page() => clonk_frontend::startup_about_dlg::AboutPage::Licenses);
    main_assert!(app.ui_sound_log.is_empty());
    app.test_key(VirtualKeyCode::ArrowUp, ElementState::Pressed);
    main_assert_eq!(app.message_dialogs.len() => 1);
    main_assert_eq!(app.message_dialogs[0].state.caption() => "Check for Updates");
    main_assert!(app.ui_sound_log.is_empty());
    app.finish_message_dialog(clonk_frontend::message_dialog::MessageDialogResult::Cancel)
        .test_value();

    app.show_main_menu();
    app.handle_game_over().test_value();
    app.test_key(VirtualKeyCode::KeyA, ElementState::Pressed);
    main_assert!(app.game_over_dialog.is_some());
    main_assert_eq!(app.startup_view => StartupView::MainMenu);
}

#[test]
fn player_typeahead_stays_behind_rename_and_modal_dialogs() {
    let mut app = new_classic_menu_app(640, 480);
    app.startup_player_models = ["Thomas", "tina"]
        .map(|name| menus1_fixture!(player_selection: name.to_string(), 0xff))
        .into_iter()
        .collect();
    app.open_player_selection_dialog();

    app.startup_crew_rename = Some(StartupCrewRenameState {
        index: 0,
        player_path: PathBuf::from("Player.c4p"),
        file_name: "Crew.c4i".to_string(),
        edit: RenameEdit::new("Crew", Some(PlrSelControl::PlayerList)),
        last_click: None,
        ignore_pointer_up: false,
    });
    app.test_text_input('T');
    main_assert_eq!(app.startup_player_dialog.as_ref().expect("player dialog").selected_index() => Some(0), "the covered list must not type-ahead");
    main_assert_ne!(app.startup_crew_rename.as_ref().expect("inline rename").edit.text() => "Crew");
    app.startup_crew_rename = None;

    app.push_message_dialog(
        clonk_frontend::message_dialog::MessageDialogState::regular_ok(
            "Covered",
            "Modal",
            clonk_frontend::message_dialog::MessageDialogIcon::NOTIFY,
        ),
        MessageDialogContinuation::None,
    )
    .test_value();
    app.test_text_input('T');
    app.test_key(VirtualKeyCode::ContextMenu, ElementState::Pressed);
    main_assert_eq!(app.startup_player_dialog.as_ref().expect("player dialog").selected_index() => Some(0));
    main_assert!(app.context_menu.is_none());
}

#[test]
fn crew_rename_is_inline_reselects_invalid_and_commits_on_focus_loss() {
    let directory = tempdir();
    let player_path = directory.path().join("Ada.c4p");
    fs::create_dir(&player_path).test_value();
    fs::write(
        player_path.join("Player.txt"),
        "[Player]\nName=Ada\n\n[Preferences]\nColorDw=255\n",
    )
    .test_value();
    for (file_name, name) in [("Alpha.c4i", "Alpha"), ("Taken.c4i", "Taken")] {
        let crew = player_path.join(file_name);
        fs::create_dir(&crew).test_value();
        fs::write(
            crew.join("ObjectInfo.txt"),
            format!("[ObjectInfo]\nid=CLNK\nName={name}\nParticipation=1\n"),
        )
        .test_value();
    }
    let player_file = PlayerFile::load_from_path(&player_path).test_value();
    let player_model = menus1_fixture!(player_selection: "Ada".to_string(), 255);
    let mut app = new_classic_menu_app(640, 480);
    app.startup_player_files.push(menus1_fixture!(
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

    let alpha_index = app
        .startup_crew_models
        .iter()
        .position(|crew| crew.name == "Alpha")
        .test_value();
    app.startup_player_dialog
        .test_mut()
        .set_selected_index(Some(alpha_index));
    app.test_key(VirtualKeyCode::F2, ElementState::Pressed);
    let rename = app.startup_crew_rename.test_ref();
    main_assert!(!rename.edit.label_visible());
    main_assert!(rename.edit.is_focused());
    main_assert_eq!(rename.edit.selected_text() => Some("Alpha"));
    main_assert!(app.startup_crew_rename_rect().is_some());
    main_assert!(app.game_option_input_dialog.is_none());
    for character in "Draft".chars() {
        app.test_text_input(character);
    }
    app.test_key(VirtualKeyCode::F2, ElementState::Pressed);
    main_assert_eq!(app.startup_crew_rename.as_ref().expect("restarted inline rename").edit.selected_text() => Some("Alpha"));

    let edit_rect = app.startup_crew_rename_rect().test_value();
    let edit_point = GuiPoint::new(
        (edit_rect.x + edit_rect.w / 2) as f32,
        (edit_rect.y + edit_rect.h / 2) as f32,
    );
    main_assert!(app.handle_startup_crew_rename_middle_down(edit_point, None));
    main_assert!(app.startup_crew_rename.as_ref().expect("middle-clicked inline rename").edit.selection_range().is_none());
    app.test_key(VirtualKeyCode::F2, ElementState::Pressed);
    app.startup_crew_rename.test_mut().last_click = Some(Instant::now());
    main_assert!(app.handle_startup_crew_rename_pointer_down(edit_point));
    main_assert!(!app.startup_crew_rename.as_ref().expect("double-clicked inline rename").edit.is_dragging());
    main_assert!(app.handle_startup_crew_rename_pointer_up(edit_point));
    app.test_key(VirtualKeyCode::F2, ElementState::Pressed);
    app.startup_player_dialog
        .test_mut()
        .set_pointer_position(Some(edit_point));
    let expected_edit_entries = app.startup_crew_rename_context_entries(false);
    main_assert!(expected_edit_entries.iter().any(|entry| {
        entry.action
            == Some(AppContextMenuCommand::StartupCrewRename(
                clonk_frontend::startup_netdlg::NetDlgEditContextCommand::Cut,
            ))
    }));
    app.test_right_button(ElementState::Pressed);
    main_assert!(app.startup_crew_rename.is_some());
    main_assert!(matches!(app.context_menu.as_ref().expect("inline edit context").layout().panels[0].rows.len(), 3 | 4));
    app.close_context_menu_silently();

    let layout = app.startup_player_dialog.test_ref().layout();
    let same_row_point = GuiPoint::new(
        (layout.list_viewport.x + layout.item_height / 2) as f32,
        (layout.list_viewport.y + layout.item_pitch * alpha_index as i32
            - app.startup_player_dialog.test_ref().list_scroll_offset()
            + layout.item_height / 2) as f32,
    );
    let inert_row_point = GuiPoint::new(
        (layout.list_viewport.x + layout.item_height + layout.item_height / 2) as f32,
        same_row_point.y,
    );
    app.startup_player_dialog
        .test_mut()
        .set_pointer_position(Some(inert_row_point));
    app.test_left_button(ElementState::Pressed);
    app.test_left_button(ElementState::Released);
    main_assert!(app.startup_crew_rename.is_some());

    app.startup_player_dialog
        .test_mut()
        .set_pointer_position(Some(same_row_point));
    app.test_right_button(ElementState::Pressed);
    main_assert!(app.startup_crew_rename.is_some());
    main_assert_eq!(app.context_menu.as_ref().expect("crew row context").layout().panels[0].rows.len() => 3);
    app.close_context_menu_silently();

    app.startup_player_dialog
        .test_mut()
        .set_pointer_position(Some(same_row_point));
    app.test_left_button(ElementState::Pressed);
    main_assert!(app.startup_crew_rename.is_some());
    app.test_left_button(ElementState::Released);
    main_assert!(app.startup_crew_rename.is_none());
    main_assert!(player_path.join("Alpha.c4i").exists());
    app.test_key(VirtualKeyCode::F2, ElementState::Pressed);

    app.test_key(VirtualKeyCode::Escape, ElementState::Pressed);
    main_assert!(app.startup_crew_rename.is_none());
    main_assert!(player_path.join("Alpha.c4i").exists());

    app.test_key(VirtualKeyCode::F2, ElementState::Pressed);
    for character in "Discarded".chars() {
        app.test_text_input(character);
    }
    let taken_index = app
        .startup_crew_models
        .iter()
        .position(|crew| crew.name == "Taken")
        .test_value();
    let other_row_point = GuiPoint::new(
        (layout.list_viewport.x + layout.item_height / 2) as f32,
        (layout.list_viewport.y + layout.item_pitch * taken_index as i32
            - app.startup_player_dialog.test_ref().list_scroll_offset()
            + layout.item_height / 2) as f32,
    );
    app.startup_player_dialog
        .test_mut()
        .set_pointer_position(Some(other_row_point));
    app.test_right_button(ElementState::Pressed);
    main_assert!(app.startup_crew_rename.is_none());
    main_assert!(player_path.join("Alpha.c4i").exists());
    main_assert!(!player_path.join("Discarded.c4i").exists());
    main_assert_eq!(app.startup_player_dialog.as_ref().expect("player dialog").selected_index() => Some(taken_index));
    main_assert!(app.context_menu.is_some());
    app.close_context_menu_silently();
    app.startup_player_dialog
        .test_mut()
        .set_selected_index(Some(alpha_index));

    app.test_key(VirtualKeyCode::F2, ElementState::Pressed);
    for character in "Taken".chars() {
        app.test_text_input(character);
    }
    app.test_key(VirtualKeyCode::Enter, ElementState::Pressed);
    let rename = app.startup_crew_rename.test_ref();
    main_assert!(rename.edit.is_focused());
    main_assert_eq!(rename.edit.selected_text() => Some("Taken"));
    let collision = app.message_dialogs.last().test_value();
    main_assert_eq!(collision.state.caption() => "Rename failure.");
    main_assert_eq!(collision.state.message() => "A Clonk with the file name \"Taken.c4i\" exists already.");
    app.finish_message_dialog(clonk_frontend::message_dialog::MessageDialogResult::Ok)
        .test_value();

    for character in "Renamed".chars() {
        app.test_text_input(character);
    }
    app.test_key(VirtualKeyCode::Enter, ElementState::Pressed);
    main_assert!(app.startup_crew_rename.is_none());
    main_assert!(!player_path.join("Alpha.c4i").exists());
    main_assert!(player_path.join("Renamed.c4i").exists());
    main_assert_eq!(app.startup_player_dialog.as_ref().expect("player dialog").focused_control() => PlrSelControl::PlayerList);

    let renamed_index = app
        .startup_crew_models
        .iter()
        .position(|crew| crew.name == "Renamed")
        .test_value();
    app.startup_player_dialog
        .test_mut()
        .set_selected_index(Some(renamed_index));
    app.test_key(VirtualKeyCode::F2, ElementState::Pressed);
    let focus_loss_name = "Blurred crew name exceeds thirty";
    let focus_loss_file = crew_file_name_for_title(focus_loss_name);
    for character in focus_loss_name.chars() {
        app.test_text_input(character);
    }
    app.test_key(VirtualKeyCode::Tab, ElementState::Pressed);
    main_assert!(app.startup_crew_rename.is_none());
    main_assert!(!player_path.join("Renamed.c4i").exists());
    main_assert!(player_path.join(&focus_loss_file).exists());
    main_assert_eq!(app.startup_player_dialog.as_ref().expect("player dialog").focused_control() => PlrSelControl::PlayerList);

    let focus_loss_index = app
        .startup_crew_models
        .iter()
        .position(|crew| crew.name == focus_loss_name)
        .test_value();
    main_assert!(
        fs::read_to_string(player_path.join(&focus_loss_file).join("ObjectInfo.txt"))
            .expect("read truncated persisted crew core")
            .contains("Name=Blurred crew name exceeds thir")
    );
    app.startup_player_dialog
        .test_mut()
        .set_selected_index(Some(focus_loss_index));
    app.test_key(VirtualKeyCode::F2, ElementState::Pressed);
    for character in "Partial".chars() {
        app.test_text_input(character);
    }
    fs::rename(
        player_path.join(&focus_loss_file),
        player_path.join("Partial.c4i"),
    )
    .test_value();
    app.accept_startup_crew_rename_after_rewrite_failure(
        focus_loss_index,
        &player_path,
        &focus_loss_file,
        "Partial.c4i",
        "Partial",
    )
    .test_value();
    main_assert!(app.startup_crew_rename.is_none());
    let partial_index = app
        .startup_crew_files
        .iter()
        .position(|entry| entry.file_name == "Partial.c4i")
        .test_value();
    main_assert_eq!(app.startup_crew_models[partial_index].name => "Partial");
    main_assert_eq!(app.startup_crew_files[partial_index].file_name => "Partial.c4i");
    main_assert_eq!(app.startup_crew_files[partial_index].crew_info.name => "Partial");
    main_assert!(
        fs::read_to_string(player_path.join("Partial.c4i/ObjectInfo.txt"))
            .expect("read stale core after simulated rewrite failure")
            .contains("Name=Blurred crew name exceeds thir")
    );
    let rewrite_failure = app.message_dialogs.last().test_value();
    main_assert_eq!(rewrite_failure.state.caption() => "");
    main_assert_eq!(rewrite_failure.state.message() => "File modification failure.");
    app.finish_message_dialog(clonk_frontend::message_dialog::MessageDialogResult::Ok)
        .test_value();

    app.test_key(VirtualKeyCode::F2, ElementState::Pressed);
    main_assert!(app.startup_crew_rename.is_some());
    app.process_player_dialog_actions(vec![clonk_frontend::startup_plrsel::PlrSelAction::Back])
        .test_value();
    main_assert_eq!(app.startup_view => StartupView::MainMenu);
    main_assert!(app.startup_crew_rename.is_none());
}

#[test]
fn player_properties_context_closes_and_opens_the_editor() {
    let mut app = new_classic_menu_app(640, 480);
    let model = menus1_fixture!(player_selection: "Context Player".to_string(), 0xff);
    app.startup_player_files.push(menus1_fixture!(
        startup_player:
            PathBuf::from("Context Player.c4p"),
            "Context Player.c4p".to_string(),
            PlayerFile::default(),
            model.clone(),
    ));
    app.startup_player_models.push(model);
    app.open_player_selection_dialog();
    let layout = clonk_frontend::startup_plrsel::plrsel_layout(640, 480);
    app.startup_player_dialog
        .as_mut()
        .test_value()
        .set_pointer_position(Some(GuiPoint::new(
            (layout.list_client.x + layout.item_height * 2) as f32,
            (layout.list_client.y + layout.item_height / 2) as f32,
        )));
    main_assert!(app.open_startup_player_context_menu(false).expect("open exact player context"));
    main_assert!(app.context_menu.is_some());
    let before_models = app.startup_player_models.len();
    let before_files = app.startup_player_files.len();

    app.process_context_menu_outcome(ContextMenuOutcome {
        captured: true,
        pass_through: false,
        focus_suppressed: true,
        events: vec![
            ContextMenuEvent::Closed,
            ContextMenuEvent::Sound(ContextMenuSound::Click),
            ContextMenuEvent::Activated(AppContextMenuCommand::StartupPlayer(
                PlrSelPlayerContextCommand::PlayerProperties(0),
            )),
        ],
    })
    .test_value();
    main_assert!(app.context_menu.is_none());
    main_assert!(matches!(
        app.startup_player_properties_dialog
            .as_ref()
            .map(|pending| pending.controller.mode()),
        Some(clonk_frontend::startup_plrproperties::PlayerPropertiesMode::Edit { index: 0 })
    ));
    main_assert!(app.status_text.is_empty());
    main_assert!(app.message_dialogs.is_empty());
    main_assert_eq!(app.startup_player_models.len() => before_models);
    main_assert_eq!(app.startup_player_files.len() => before_files);
}

#[test]
fn menu_render_defers_or_applies_the_monitor_gamma_post_pass() {
    let mut app = new_real_classic_menu_app(320, 240);
    let configured_gamma =
        clonk_graphics::GammaRamp::from_control_points([0x101010, 0x707070, 0xe0e0e0]);
    app.loader_gamma = Some(configured_gamma.clone());
    app.graphics
        .set_advanced_renderer_config(clonk_frontend::AdvancedRendererConfig {
            shader: false,
            use_shader_gamma: true,
            disable_gamma: false,
            ..clonk_frontend::AdvancedRendererConfig::DEFAULT
        });
    app.main_menu_state.menu.set_gamma_ramp(None);

    // A deferred pass leaves the raw logical frame for the physical
    // post-pass; the same render without the deferral applies the ramp
    // itself.
    let mut deferred = vec![0x55; 320 * 240 * 4];
    main_assert!(app.render_for_presentation_with_monitor_defer(&mut deferred, false, false, false, true,).expect("render the raw logical menu base"));

    let mut direct = vec![0x77; deferred.len()];
    main_assert!(app.render_for_presentation_with_monitor_defer(&mut direct, false, false, false, false,).expect("direct render applies its own monitor gamma"));

    let mut expected = deferred.clone();
    configured_gamma.apply_to_rgba_bytes(&mut expected);
    main_assert_eq!(direct => expected);
    main_assert_ne!(direct => deferred, "the configured monitor ramp must change the presented pixels");
}

fn solid_gui_sheet(pixel: [u8; 4], width: u32, height: u32) -> ImageData {
    let mut pixels = Vec::with_capacity((width * height * 4) as usize);
    for _ in 0..width * height {
        pixels.extend_from_slice(&pixel);
    }
    ImageData::new(width, height, pixels)
}

fn gui_sheet_override(
    stem: &'static str,
    canonical_name: &'static str,
    source: &str,
    pixel: [u8; 4],
) -> ClassicGuiSheetOverride {
    ClassicGuiSheetOverride {
        stem,
        canonical_name,
        source: source.to_string(),
        image: solid_gui_sheet(pixel, 64, 32),
    }
}

#[test]
fn active_scenario_gui_overrides_reach_dialogs_and_script_menus() {
    let mut app = new_menu_app(320, 200);
    let pristine_caption = app
        .assets
        .startup_dialog_images
        .get("GUICaption.png")
        .test_value()
        .clone();
    let pristine_scroll = app
        .assets
        .startup_dialog_images
        .get("GUIScroll.png")
        .test_value()
        .clone();
    let overrides = vec![
        gui_sheet_override(
            "GUICaption",
            "GUICaption.png",
            "Hazard.c4f/Graphics.c4g:GUICaption.png",
            [0x11, 0x22, 0x33, 0xff],
        ),
        gui_sheet_override(
            "GUIScroll",
            "GUIScroll.png",
            "Hazard.c4f/Graphics.c4g:GUIScroll.png",
            [0x44, 0x55, 0x66, 0xff],
        ),
        gui_sheet_override(
            "GUIProgress",
            "GUIProgress.png",
            "Hazard.c4f/Graphics.c4g:GUIProgress.png",
            [0x77, 0x88, 0x99, 0xff],
        ),
    ];
    app.install_active_gui_sheet_overrides(&overrides);

    // The rebound C4GUI::Resource sheets stay boundary-clean and reach
    // every reusable running dialog and the script-menu graphics.
    app.assets
        .require_classic_global_gui_bootstrap_resources(&HashMap::new())
        .test_value();
    let message = app.assets.message_dialog_resources().test_value();
    main_assert_eq!(message.progress.pixels()[..4] => [0x77, 0x88, 0x99, 0xff]);
    app.assets.input_dialog_resources().test_value();
    main_assert_eq!(
        app.assets
            .startup_dialog_images
            .get("GUICaption.png")
            .expect("rebound caption sheet")
            .pixels()[..4] =>
        [0x11, 0x22, 0x33, 0xff],
        "the caption consumed by every dialog skin must be the override"
    );
    let info = app.assets.static_info_dialog_resources().test_value();
    main_assert_eq!(info.scroll.pixels()[..4] => [0x44, 0x55, 0x66, 0xff]);
    main_assert_eq!(
        app.ensure_ingame_menu_gfx()
            .caption_bar
            .as_ref()
            .expect("script menus keep a caption bar")
            .pixels()[..4] =>
        [0x11, 0x22, 0x33, 0xff],
        "script-menu graphics must read the rebound caption sheet"
    );

    // Startup teardown (Resource::Clear + CloseFiles) restores the
    // pristine startup sheets for the next startup generation.
    app.show_main_menu();
    main_assert!(app.assets.active_gui_sheet_sources.is_empty());
    main_assert!(app.assets.startup_gui_sheet_images.is_empty());
    main_assert_eq!(
        app.assets
            .startup_dialog_images
            .get("GUICaption.png")
            .expect("restored caption sheet")
            .pixels()
            .as_ptr() =>
        pristine_caption.pixels().as_ptr(),
        "teardown must restore the pristine caption surface"
    );
    main_assert_eq!(
        app.assets
            .startup_dialog_images
            .get("GUIScroll.png")
            .expect("restored scroll sheet")
            .pixels()
            .as_ptr() =>
        pristine_scroll.pixels().as_ptr(),
        "teardown must restore the pristine scroll surface"
    );
    main_assert!(app.ingame_menu_gfx.is_none(), "cached script-menu graphics must not outlive the rebound sheets");
}

#[test]
fn active_gui_sheet_overrides_rebind_only_when_the_winning_source_changes() {
    let mut app = new_menu_app(320, 200);
    let pristine_highlight = app
        .assets
        .startup_dialog_images
        .get("GUIButtonHighlight.png")
        .test_value()
        .clone();

    let first = vec![gui_sheet_override(
        "GUIButtonHighlight",
        "GUIButtonHighlight.png",
        "Hazard.c4f/Graphics.c4g:GUIButtonHighlight.png",
        [0x10, 0x20, 0x30, 0xff],
    )];
    app.install_active_gui_sheet_overrides(&first);
    let applied_ptr = app
        .assets
        .startup_dialog_images
        .get("GUIButtonHighlight.png")
        .test_value()
        .pixels()
        .as_ptr();
    main_assert_eq!(applied_ptr => first[0].image.pixels().as_ptr());
    main_assert_eq!(
        app.assets
            .button_highlight
            .as_ref()
            .expect("derived button highlight")
            .pixels()[..4] =>
        [0x10, 0x20, 0x30, 0xff],
        "derived highlight state must recompute from the rebound sheet"
    );

    // A repeated refresh with the same winning source is the C++
    // group-id cache hit: the surface is not reloaded even though the
    // refresh decoded a fresh image.
    let repeat = vec![gui_sheet_override(
        "GUIButtonHighlight",
        "GUIButtonHighlight.png",
        "Hazard.c4f/Graphics.c4g:GUIButtonHighlight.png",
        [0xa0, 0xb0, 0xc0, 0xff],
    )];
    app.install_active_gui_sheet_overrides(&repeat);
    main_assert_eq!(
        app.assets
            .startup_dialog_images
            .get("GUIButtonHighlight.png")
            .expect("cached highlight sheet")
            .pixels()
            .as_ptr() =>
        applied_ptr,
        "an unchanged winning source must not reload the sheet"
    );

    // A different winning source is a changed group id: reload.
    let changed = vec![gui_sheet_override(
        "GUIButtonHighlight",
        "GUIButtonHighlight.png",
        "Extra.c4g/Graphics.c4g:GUIButtonHighlight.png",
        [0xa0, 0xb0, 0xc0, 0xff],
    )];
    app.install_active_gui_sheet_overrides(&changed);
    main_assert_eq!(
        app.assets
            .startup_dialog_images
            .get("GUIButtonHighlight.png")
            .expect("reloaded highlight sheet")
            .pixels()[..4] =>
        [0xa0, 0xb0, 0xc0, 0xff],
        "a changed winning source must rebind the sheet"
    );

    // A refresh where the global group wins again restores the pristine
    // surface without waiting for teardown.
    app.install_active_gui_sheet_overrides(&[]);
    main_assert_eq!(
        app.assets
            .startup_dialog_images
            .get("GUIButtonHighlight.png")
            .expect("restored highlight sheet")
            .pixels()
            .as_ptr() =>
        pristine_highlight.pixels().as_ptr(),
        "losing every override must restore the pristine surface"
    );
    main_assert_eq!(
        app.assets
            .button_highlight
            .as_ref()
            .expect("restored derived highlight")
            .pixels()
            .as_ptr() =>
        pristine_highlight.pixels().as_ptr(),
        "derived highlight state must follow the restored sheet"
    );
    main_assert!(app.assets.active_gui_sheet_sources.is_empty());
    main_assert!(app.assets.startup_gui_sheet_images.is_empty());
}

#[test]
fn real_mars_full_size_highlight_reaches_host_gui_resources() {
    let _lock = env_lock().lock();
    let user_data = tempdir();
    let (_guard, paths) = exact_loader_test_paths(user_data.path(), None);
    let mut app = new_menu_app_with_paths(320, 200, &paths);
    let scenario =
        resolve_next_mission_scenario(&app.scenario_catalog, "ClonkMars.c4f/01_Fossae.c4s")
            .test_value();
    let setup = build_scenario_loader(
        &scenario,
        &app.scenario_seed_definition_load(),
        &paths,
        app.assets.as_ref(),
    )
    .test_value();
    let highlight = setup
        .refreshed_gui_sheet_overrides
        .iter()
        .find(|sheet| sheet.stem == "GUIButtonHighlight")
        .cloned()
        .test_value();

    // C4GUI::Resource::Load keeps the winning C4FCT_Full dimensions and
    // C4Facet::DrawX stretches that complete source for every consumer
    // (src/C4Gui.cpp:1093; src/C4FacetEx.cpp:137-161;
    // src/C4Facet.cpp:296-304).
    main_assert!(highlight.source.contains("ClonkMars.c4f/Graphics.c4g"), "unexpected Mars highlight source: {}", highlight.source);
    main_assert_eq!((highlight.image.width(), highlight.image.height()) => (30, 30));
    app.install_active_gui_sheet_overrides(std::slice::from_ref(&highlight));
    main_assert_eq!(app.assets.startup_dialog_images.get("GUIButtonHighlight.png").map(|image| (image.width(), image.height())) => Some((30, 30)));
    app.assets.network_start_wait_resources().test_value();
    app.assets.game_lobby_resources().test_value();
    app.assets.game_option_resources().test_value();
    app.assets.input_dialog_resources().test_value();
    let scensel = app.assets.scensel_assets().test_value();
    let button_down = app.assets.dialog_image("GUIButtonDown.png").test_value();
    clonk_frontend::startup_scensel::validate_scensel_button_assets(&scensel, &button_down)
        .test_value();
}

#[test]
fn real_mars_upper_board_keeps_the_product_logo() {
    // C4GraphicsResource::Init resolves Logo.png over the registered
    // Graphics.c4g group set, so a scenario folder that ships its own copy
    // wins over planet/ (src/C4GraphicsResource.cpp:418-470), and
    // C4UpperBoard::Execute draws that facet centered on the board
    // (src/C4UpperBoard.cpp:88-92). ClonkMars must therefore not carry a
    // stale copy of the base-game logo: unlike Hazard's own total-conversion
    // branding, it only restates the product name, which this port rebranded.
    let _lock = env_lock().lock();
    let user_data = tempdir();
    let (_guard, paths) = exact_loader_test_paths(user_data.path(), None);
    let app = new_menu_app_with_paths(320, 200, &paths);
    let scenario =
        resolve_next_mission_scenario(&app.scenario_catalog, "ClonkMars.c4f/01_Fossae.c4s")
            .test_value();
    let product = app.assets.hud_graphics().logo.clone().test_value();
    let mars = app
        .loaded_game_graphics_resources(&scenario, None)
        .expect("resolve the real Mars game graphics")
        .hud_graphics
        .logo
        .clone()
        .test_value();

    main_assert_eq!((mars.width(), mars.height()) => (product.width(), product.height()), "a Mars scenario must draw the product logo on its upper board");
    main_assert_eq!(mars.pixels() => product.pixels());
}

#[test]
fn running_global_gui_guard_precedes_every_recursive_menu_screen() {
    let check = |mut app: GameApp, label: &str| {
        app.scoreboard_initial_reconcile_pending = true;
        let before = runtime_global_ui_snapshot(&app);
        remove_global_gui_sheet(&mut app, "GUIBigArrows.png");
        let mut frame = vec![0x84; 320 * 200 * 4];
        let error = app
            .render(&mut frame)
            .expect_err("running menu bypassed global GUI preflight");
        assert_global_gui_boundary(
            &error,
            vec![ClassicGuiBootstrapIssue::missing("GUIBigArrows")],
        );
        main_assert_eq!(runtime_global_ui_snapshot(&app) => before, "{label}");
        main_assert!(frame.iter().all(|byte| *byte == 0x84), "{label}");
    };

    let pages = vec![
        (
            "C4MainMenu::Main",
            IngameMenuState::main_menu(
                &MainMenuConditions::default(),
                &IngameMenuLabels::default(),
            )
            .expect("nonempty main menu"),
        ),
        (
            "C4MainMenu::Goals",
            IngameMenuState::goals_menu(
                &[menus1_fixture!(goal_rule: "GOAL".to_string(), "Goal".to_string())],
                &IngameMenuLabels::default(),
            ),
        ),
        (
            "C4MainMenu::Rules",
            IngameMenuState::rules_menu(
                &[menus1_fixture!(goal_rule: "RULE".to_string(), "Rule".to_string())],
                &IngameMenuLabels::default(),
            ),
        ),
        (
            "C4MainMenu::NewPlayer",
            IngameMenuState::new_player_menu(
                &[ingame_menu::NewPlayerEntry {
                    file: "Player.c4p".to_string(),
                    name: "Player".to_string(),
                }],
                &IngameMenuLabels::default(),
            ),
        ),
        (
            "C4MainMenu::Savegame",
            IngameMenuState::savegame_menu(
                &[SaveSlotState { free: true }; 10],
                &IngameMenuLabels::default(),
            ),
        ),
        (
            "C4MainMenu::Options",
            IngameMenuState::options_menu(
                &OptionFlags {
                    sound: true,
                    music: true,
                    mouse_shown: true,
                    mouse: true,
                },
                0,
                &IngameMenuLabels::default(),
            ),
        ),
        (
            "C4MainMenu::Display",
            IngameMenuState::display_menu(
                &DisplayFlags::default(),
                0,
                &IngameMenuLabels::default(),
            ),
        ),
        (
            "C4MainMenu::Surrender",
            IngameMenuState::surrender_menu(&IngameMenuLabels::default()),
        ),
        (
            "C4MainMenu::ClientDisconnect",
            IngameMenuState::client_disconnect_menu(&IngameMenuLabels::default()),
        ),
        (
            "C4MainMenu::HostDisconnect",
            IngameMenuState::host_disconnect_menu(
                &[HostDisconnectClientEntry {
                    client_id: 0,
                    caption: "Host (Host)".to_string(),
                    activated: true,
                }],
                &IngameMenuLabels::default(),
            ),
        ),
    ];
    main_assert_eq!(pages.len() => 10, "MenuPage exhaustiveness changed");
    for (label, page) in pages {
        let mut app = new_running_sandbox_app();
        app.ingame_menu.replace(app.local_owner, Some(page));
        check(app, label);
    }

    let mut object = new_running_sandbox_app();
    main_assert!(object.open_object_menu().expect("open app-owned object menu"));
    check(object, "app-owned object menu");

    let mut scoreboard = new_running_sandbox_app();
    scoreboard.scoreboard_dialog = Some(scoreboard.scoreboard_request());
    check(scoreboard, "visible scoreboard");

    for style in 0..=3 {
        let mut app = new_running_sandbox_app();
        let cursor = app.engine.test_crew_cursor(app.local_owner);
        let mut menu = two_item_script_menu(cursor);
        menu.style = style;
        app.engine
            .apply_object_update(
                cursor,
                ObjectUpdate {
                    menu: Some(Some(menu)),
                    ..ObjectUpdate::default()
                },
            )
            .test_value();
        check(app, &format!("engine script menu style {style}"));
    }
}

#[test]
fn global_gui_guard_is_first_at_every_external_ui_ingress() {
    let mut app = new_classic_menu_app(320, 200);
    remove_global_gui_sheet(&mut app, "GUISpinBoxArrow.png");
    let modifiers = app.keyboard_modifiers;
    let dimensions = {
        let surface = app.graphics.surface();
        (surface.width(), surface.height())
    };
    let engine_game_time = app.engine.game_time();
    let snapshot_game_time = app.snapshot.game_time;
    let mut second_accumulator = Duration::from_millis(125);
    let expect_engine = |result: Result<(), EngineError>| {
        main_assert!(matches!(result, Err(EngineError::ClassicMenuParityBoundary { ref detail }) if detail.contains("GUISpinBoxArrow")));
    };
    expect_engine(app.handle_modifiers_changed(ModifiersState::SHIFT));
    expect_engine(app.handle_text_input('x'));
    expect_engine(app.handle_key(VirtualKeyCode::KeyA, ElementState::Pressed));
    expect_engine(app.handle_key(VirtualKeyCode::F11, ElementState::Pressed));
    expect_engine(app.handle_focus_lost());
    expect_engine(app.process_gamepad_event_batch([GamepadEvent::Clear {
        slot: GamepadSlot::new(0),
    }]));
    expect_engine(app.handle_cursor_moved(PhysicalPosition::new(10.0, 10.0)));
    expect_engine(app.handle_mouse_button(ElementState::Pressed));
    expect_engine(app.handle_right_mouse_button(ElementState::Pressed));
    expect_engine(app.handle_other_mouse_button(ElementState::Pressed));
    expect_engine(app.handle_mouse_wheel(MouseScrollDelta::LineDelta(0.0, 1.0), 1.0));
    expect_engine(app.handle_touch(TouchPhase::Started, GuiPoint::new(10.0, 10.0)));
    expect_engine(app.pointer_left());
    expect_engine(app.sec1_timer().map(|_| ()));
    expect_engine(
        advance_game_clock_from_elapsed(&mut app, &mut second_accumulator, Duration::from_secs(1))
            .map(|_| ()),
    );
    expect_engine(app.update());
    let resize = app
        .resize(640, 480)
        .expect_err("resize must fail at global guard");
    main_assert!(matches!(resize.downcast_ref::<ClassicParityBoundary>(), Some(ClassicParityBoundary::GlobalGuiBootstrapResources { .. })));
    main_assert_eq!(app.keyboard_modifiers => modifiers);
    let surface = app.graphics.surface();
    main_assert_eq!((surface.width(), surface.height()) => dimensions);
    main_assert_eq!(app.engine.game_time() => engine_game_time);
    main_assert_eq!(app.snapshot.game_time => snapshot_game_time);
    main_assert_eq!(second_accumulator => Duration::from_millis(125));
    main_assert!(app.context_menu.is_none());
    main_assert!(app.message_dialogs.is_empty());
}

#[test]
fn ingame_menu_abort_routes_to_the_same_confirmation() {
    let mut app = new_menu_app(320, 200);
    app.start_sandbox_scenario(FrontendScenario::fallback())
        .test_value();
    let mut menu =
        IngameMenuState::main_menu(&MainMenuConditions::default(), &IngameMenuLabels::default())
            .test_value();
    let abort = menu
        .items()
        .iter()
        .position(|item| item.action == MenuAction::Abort)
        .test_value();
    menu.set_selection(abort);
    app.ingame_menu.replace(app.local_owner, Some(menu));
    app.status_text.clear();

    app.handle_menu_command_failsafe(
        app.local_owner,
        ControlCommand::MenuEnter,
        CommandKind::Press,
    )
    .test_value();
    main_assert!(app.message_dialogs.last().is_some_and(|dialog| matches!(dialog.continuation, MessageDialogContinuation::AbortGame { .. })));
    main_assert!(app.ingame_menu.is_none(), "C4Menu::Enter closes the nonpermanent main menu before Abort");
    main_assert!(matches!(app.mode, AppMode::Running));
    main_assert!(app.status_text.is_empty());
}

#[test]
fn unported_object_menu_requests_fail_before_generic_object_menu_state_exists() {
    let mut app = new_state_only_menu_app(320, 200);
    app.start_sandbox_scenario(FrontendScenario::fallback())
        .test_value();
    let crew_id = app
        .snapshot
        .objects
        .iter()
        .find(|object| object.crew_member)
        .test_value()
        .id;

    for (kind, label) in [
        (MenuRequestKind::Activate, "Activate"),
        (
            MenuRequestKind::ActivateTarget { container: crew_id },
            "Activate",
        ),
        (MenuRequestKind::Get { container: crew_id }, "Get"),
    ] {
        app.object_menu = None;
        app.snapshot.menu_requests = vec![clonk_engine::MenuRequest {
            crew_id,
            owner: app.local_owner,
            kind,
        }];
        let error = app
            .handle_menu_requests()
            .expect_err("generic app-owned object menu must fail at creation");
        main_assert!(error.to_string().contains(label), "unexpected {error}");
        main_assert!(app.object_menu.is_none());
    }

    app.snapshot.menu_requests = vec![clonk_engine::MenuRequest {
        crew_id,
        owner: app.local_owner,
        kind: MenuRequestKind::Construction,
    }];
    app.handle_menu_requests().test_value();
    main_assert!(app.object_menu.is_none());
}

#[test]
fn running_function_keys_without_bindings_are_ignored() {
    let mut app = new_menu_app(320, 200);
    app.start_sandbox_scenario(FrontendScenario::fallback())
        .test_value();
    // C4Game registers bare F9/Ctrl+F9 for screenshots and Ctrl+F5..F8 for
    // diagnostics, but no bare F5/F6/F7 save/load route (C4Game.cpp:3373-3374,
    // 3386-3389).
    for (key, label) in [
        (VirtualKeyCode::F5, "F5"),
        (VirtualKeyCode::F6, "F6"),
        (VirtualKeyCode::F7, "F7"),
    ] {
        app.handle_key(key, ElementState::Pressed)
            .unwrap_or_else(|error| panic!("unsupported {label} must be ignored: {error}"));
        main_assert!(app.ingame_menu.is_none());
        main_assert!(app.object_menu.is_none());
        main_assert!(app.pending_screenshots.is_empty());
    }
}

#[test]
fn activate_savegame_opens_classic_ten_slot_menu() {
    let mut app = new_classic_running_sandbox_app();
    app.apply_ingame_menu_action(MenuAction::ActivateSavegame)
        .test_value();

    // C4MainMenu::ActivateSavegame constructs slots 1..10 before returning
    // to the main menu (C4MainMenu.cpp:422-500).
    let menu = app.ingame_menu.get(app.local_owner).test_value();
    main_assert_eq!(menu.page() => ingame_menu::MenuPage::Savegame);
    main_assert_eq!(menu.items().len() => 10);
    main_assert!(menu.items().iter().enumerate().all(|(index, item)| item.action == MenuAction::SaveSlot((index + 1) as u8)));
}

#[test]
fn screenshot_path_reuses_the_first_numbered_gap() {
    let directory = tempdir();
    fs::write(directory.path().join("Screenshot001.png"), b"one").test_value();
    fs::write(directory.path().join("Screenshot003.png"), b"three").test_value();

    main_assert_eq!(next_screenshot_path(directory.path()) => directory.path().join("Screenshot002.png"));
}

// BoolConfig initializes the Timestamps checkbox from
// Config.General.ShowLogTimestamps (C4StartupOptionsDlg.cpp:558-560,
// 749-753; C4Config.cpp:398).
#[test]
fn options_dialog_loads_log_timestamps_from_general_config() {
    let user_data = tempdir();
    let (_guard, paths) = guarded_test_app_paths(None, user_data.path());
    persist_config_value(&paths, "General", "ShowLogTimestamps", "1").test_value();
    let mut app = test_game_app(1280, 720, AudioOptions::default(), Some(&paths)).test_value();
    wait_for_menu(&mut app);

    app.open_options_menu();

    main_assert!(
        app.startup_options_dialog
            .as_ref()
            .expect("options dialog")
            .program()
            .show_log_timestamps,
        "the live checkbox must reflect General.ShowLogTimestamps"
    );
}

#[test]
fn options_sound_sheet_fails_typed_before_pixels_without_audio_context() {
    let mut app = new_real_classic_menu_app(320, 200);
    app.audio = None;
    app.open_options_menu();

    let mut program_frame = vec![0_u8; 320 * 200 * 4];
    app.test_render(&mut program_frame);

    app.test_key(VirtualKeyCode::ArrowDown, ElementState::Pressed);
    app.test_key(VirtualKeyCode::ArrowDown, ElementState::Released);

    let sound_error = app
        .handle_key(VirtualKeyCode::ArrowDown, ElementState::Pressed)
        .expect_err("Sound requires the live audio context");
    assert_engine_parity_boundary(
        sound_error,
        ClassicParityBoundary::RuntimeAudioSystem {
            action: "the startup Options Audio sheet",
        },
    );
    main_assert_eq!(app.startup_options_dialog.as_ref().expect("retained options model").active_sheet() => clonk_frontend::startup_options_dlg::OptionsSheet::Sound);

    let mut frame = vec![0xa5; 320 * 200 * 4];
    let error = app
        .render(&mut frame)
        .expect_err("render preflight must reject guessed Sound state");
    let expected = ClassicParityBoundary::RuntimeAudioSystem {
        action: "the startup Options Audio sheet",
    };
    main_assert_eq!(error.downcast_ref::<ClassicParityBoundary>() => Some(&expected));
    main_assert!(frame.iter().all(|byte| *byte == 0xa5));
}

#[test]
fn secondary_startup_dialogs_route_their_visible_controls() {
    // C4StartupMainDlg switches to concrete dialogs whose controls remain
    // live (C4StartupMainDlg.cpp:209-242). This guards the app-level seam:
    // the parity renderer and the controller must be the same state.
    let _lock = env_lock().lock();
    let user_data = tempdir();
    let (_guard, paths) = guarded_test_app_paths(None, user_data.path());
    configure_test_startup_participant(&paths, user_data.path());
    let mut app = GameApp::new(
        1280,
        720,
        disabled_audio_options(),
        Some(&paths),
        test_runtime_config_with("Player".to_string(), false),
    )
    .test_value();
    wait_for_menu(&mut app);
    app.startup_player_files.clear();
    app.startup_player_models.clear();
    let main_layout = clonk_frontend::main_menu_layout(1280, 720);
    let click_main_button = |app: &mut GameApp, index: usize| {
        let button = main_layout.buttons[index];
        let point = PhysicalPosition::new(
            f64::from(button.x + button.w / 2),
            f64::from(button.y + button.h / 2),
        );
        app.test_cursor(point);
        app.test_left_button(ElementState::Pressed);
        app.test_left_button(ElementState::Released);
    };
    let settle_startup_fade = |app: &mut GameApp| {
        main_assert!(app.startup_dialog_fade_active());
        app.startup_dialog_fade.test_mut().step = STARTUP_DIALOG_FADE_STEPS - 1;
        let mut frame = vec![0_u8; 1280 * 720 * 4];
        app.test_render(&mut frame);
        main_assert!(!app.startup_dialog_fade_active());
    };

    click_main_button(&mut app, 0);
    main_assert_eq!(app.startup_view => StartupView::ScenarioBrowser);
    app.show_main_menu();

    click_main_button(&mut app, 1);
    main_assert_eq!(app.startup_view => StartupView::NetworkGame);
    settle_startup_fade(&mut app);
    let metrics = clonk_frontend::startup_netdlg::NetDlgFontMetrics {
        caption_back_extent: 51,
        text_ip_extent: 18,
        text_line_height: 22,
        caption_line_height: 25,
        title_line_height: 34,
    };
    let network_back =
        clonk_frontend::startup_netdlg::net_dlg_layout(1280, 720, &metrics).buttons[0];
    let network_point = PhysicalPosition::new(
        f64::from(network_back.x + network_back.w / 2),
        f64::from(network_back.y + network_back.h / 2),
    );
    app.test_cursor(network_point);
    app.test_left_button(ElementState::Pressed);
    app.test_left_button(ElementState::Released);
    main_assert_eq!(app.startup_view => StartupView::MainMenu);
    settle_startup_fade(&mut app);

    let test_player = menus1_fixture!(player_selection: "Test Player".to_string(), 0xff);
    app.startup_player_files.push(menus1_fixture!(
        startup_player:
            user_data.path().join("Test Player.c4p"),
            "Test Player.c4p".to_string(),
            PlayerFile::default(),
            test_player.clone(),
    ));
    app.startup_player_models.push(test_player);
    click_main_button(&mut app, 2);
    main_assert_eq!(app.startup_view => StartupView::PlayerSelection);
    settle_startup_fade(&mut app);
    let player_layout = clonk_frontend::startup_plrsel::plrsel_layout(1280, 720);
    let player_row = PhysicalPosition::new(
        f64::from(player_layout.list_client.x + player_layout.item_height + 4),
        f64::from(player_layout.list_client.y + player_layout.item_height / 2),
    );
    app.test_cursor(player_row);
    app.test_left_button(ElementState::Pressed);
    app.test_left_button(ElementState::Released);
    app.test_left_button(ElementState::Pressed);
    app.test_left_button(ElementState::Released);
    main_assert!(matches!(
        app.startup_player_properties_dialog
            .as_ref()
            .map(|pending| pending.controller.mode()),
        Some(clonk_frontend::startup_plrproperties::PlayerPropertiesMode::Edit { index: 0 })
    ));
    main_assert!(app.status_text.is_empty());
    app.test_key(VirtualKeyCode::Escape, ElementState::Pressed);
    app.test_key(VirtualKeyCode::Escape, ElementState::Released);
    let player_back = player_layout.buttons[0];
    let player_point = PhysicalPosition::new(
        f64::from(player_back.x + player_back.w / 2),
        f64::from(player_back.y + player_back.h / 2),
    );
    app.test_cursor(player_point);
    app.test_left_button(ElementState::Pressed);
    app.test_left_button(ElementState::Released);
    main_assert_eq!(app.startup_view => StartupView::MainMenu);
    settle_startup_fade(&mut app);

    click_main_button(&mut app, 3);
    main_assert_eq!(app.startup_view => StartupView::Options);
    settle_startup_fade(&mut app);
    app.test_key(VirtualKeyCode::ArrowDown, ElementState::Pressed);
    app.test_key(VirtualKeyCode::ArrowDown, ElementState::Released);
    main_assert_eq!(app.startup_options_dialog.as_ref().expect("options state").active_sheet() => clonk_frontend::startup_options_dlg::OptionsSheet::Graphics);
    app.test_key(VirtualKeyCode::ArrowDown, ElementState::Pressed);
    app.test_key(VirtualKeyCode::ArrowDown, ElementState::Released);
    main_assert_eq!(app.startup_options_dialog.as_ref().expect("options state").active_sheet() => clonk_frontend::startup_options_dlg::OptionsSheet::Sound);

    app.test_key(VirtualKeyCode::ArrowDown, ElementState::Pressed);
    app.test_key(VirtualKeyCode::ArrowDown, ElementState::Released);
    main_assert_eq!(app.startup_options_dialog.as_ref().expect("options state").active_sheet() => clonk_frontend::startup_options_dlg::OptionsSheet::Keyboard);
    app.test_key(VirtualKeyCode::KeyR, ElementState::Pressed);
    main_assert!(app.status_text.is_empty());
    app.test_key(VirtualKeyCode::Backspace, ElementState::Pressed);
    main_assert_eq!(app.startup_view => StartupView::MainMenu);
    settle_startup_fade(&mut app);

    click_main_button(&mut app, 4);
    main_assert_eq!(app.startup_view => StartupView::About);
    settle_startup_fade(&mut app);
    let about_layout = clonk_frontend::startup_about_dlg::about_layout(1280, 720);
    let licenses = about_layout.buttons[2];
    let licenses_point = PhysicalPosition::new(
        f64::from(licenses.x + licenses.w / 2),
        f64::from(licenses.y + licenses.h / 2),
    );
    app.test_cursor(licenses_point);
    app.test_left_button(ElementState::Pressed);
    app.test_left_button(ElementState::Released);
    main_assert_eq!(app.startup_about_dialog.as_ref().expect("about state").current_page() => clonk_frontend::startup_about_dlg::AboutPage::Licenses);
    let mut licenses_frame = vec![0_u8; 1280 * 720 * 4];
    app.test_render(&mut licenses_frame);
    main_assert!(licenses_frame.iter().any(|byte| *byte != 0));

    let about_back = about_layout.buttons[0];
    let about_back_point = PhysicalPosition::new(
        f64::from(about_back.x + about_back.w / 2),
        f64::from(about_back.y + about_back.h / 2),
    );
    app.test_cursor(about_back_point);
    app.test_left_button(ElementState::Pressed);
    app.test_left_button(ElementState::Released);
    main_assert_eq!(app.startup_view => StartupView::About);

    let mut credits = vec![0_u8; 1280 * 720 * 4];
    app.test_render(&mut credits);
    let update = about_layout.buttons[1];
    let update_point = PhysicalPosition::new(
        f64::from(update.x + update.w / 2),
        f64::from(update.y + update.h / 2),
    );
    app.test_cursor(update_point);
    app.test_left_button(ElementState::Pressed);
    app.test_left_button(ElementState::Released);
    main_assert_eq!(app.startup_view => StartupView::About);
    let wait = app.message_dialogs.last().test_value();
    main_assert_eq!(wait.state.caption() => "Check for Updates");
    main_assert_eq!(wait.state.message() => "Checking for updates...");
    main_assert!(app.update_check.is_some());
    app.test_key(VirtualKeyCode::Escape, ElementState::Pressed);
    app.test_key(VirtualKeyCode::Escape, ElementState::Released);
    main_assert!(app.message_dialogs.is_empty());
    main_assert!(app.update_check.is_none());
    main_assert_eq!(app.startup_view => StartupView::About);

    app.test_cursor(about_back_point);
    app.test_left_button(ElementState::Pressed);
    app.test_left_button(ElementState::Released);
    main_assert_eq!(app.startup_view => StartupView::MainMenu);
    settle_startup_fade(&mut app);

    click_main_button(&mut app, 5);
    main_assert!(app.take_exit_request(), "Exit button requests shutdown");
    reset_cached_app_paths();
}

#[test]
fn participant_context_helpers_preserve_raw_indices_and_lazy_scan_rules() {
    let _lock = env_lock().lock();
    let install = tempdir();
    let install_root = install.path();
    fs::create_dir_all(install_root.join("planet")).test_value();
    fs::write(install_root.join("planet/System.c4g"), b"").test_value();
    let user_data = tempdir();
    let player_root = user_data.path().join("Players");
    let ada = player_root.join("Ada.c4p");
    let bob = player_root.join("Bob.c4p");
    let broken = player_root.join("Broken.c4p");
    fs::create_dir_all(&ada).test_value();
    fs::create_dir_all(&bob).test_value();
    fs::write(&broken, b"not a group").test_value();
    fs::write(player_root.join(".Hidden.c4p"), b"hidden").test_value();
    fs::write(player_root.join("Notes.txt"), b"text").test_value();
    let nested = player_root.join("Nested");
    fs::create_dir_all(&nested).test_value();
    fs::write(nested.join("Deep.c4p"), b"nested").test_value();

    let (_guard, paths) = guarded_test_app_paths(Some(install_root), user_data.path());
    let save_participants = |participants: String| {
        let mut config = Config::new();
        config.set_in(Some("General"), "PlayerPath", player_root.to_string_lossy());
        config.set_in(Some("General"), "Participants", participants);
        fs::create_dir_all(paths.config_file().parent().test_value()).test_value();
        config.save(paths.config_file()).test_value();
    };

    save_participants(format!(
        "{};{};{};{};{}",
        bob.display(),
        ada.display(),
        bob.display(),
        player_root.join("Missing.c4p").display(),
        player_root.join("Notes.txt").display(),
    ));
    update_startup_participant_config(&paths, |_| {}).test_value();
    main_assert_eq!(
        startup_participant_references(&paths).expect("read validated participants") =>
        vec![
            bob.to_string_lossy().into_owned(),
            ada.to_string_lossy().into_owned(),
        ],
        "validation keeps first spelling and config order while deduplicating"
    );

    save_participants(format!("{};;{}", ada.display(), bob.display()));
    let remove = startup_participant_remove_entries(&paths);
    main_assert_eq!(remove.len() => 2);
    main_assert_eq!(remove[0].text => "Ada");
    main_assert_eq!(remove[0].icon => ContextMenuIcon::Phase(9));
    main_assert_eq!(remove[0].tooltip.as_deref() => Some("Remove this player from participation list"));
    main_assert_eq!(remove[0].action => Some(AppContextMenuCommand::RemoveStartupParticipant(0)));
    main_assert_eq!(remove[1].action => Some(AppContextMenuCommand::RemoveStartupParticipant(2)), "empty raw segments must not renumber callback indices");

    save_participants(format!("{};;{}", bob.display(), ada.display()));
    let removed = remove_startup_participant_config(&paths, 2)
        .expect("remove using fresh raw index")
        .test_value();
    main_assert_eq!(removed => ada.to_string_lossy());
    main_assert_eq!(
        startup_participant_references(&paths).expect("read after removal") =>
        vec![bob.to_string_lossy().into_owned()],
        "activation re-reads the captured raw index instead of a stale filename"
    );

    save_participants(ada.to_string_lossy().into_owned());
    let add = startup_participant_add_entries(&paths);
    let mut names = add
        .iter()
        .map(|entry| entry.text.as_str())
        .collect::<Vec<_>>();
    names.sort_unstable();
    main_assert_eq!(names => vec!["Bob", "Broken"]);
    for entry in &add {
        main_assert_eq!(entry.icon => ContextMenuIcon::Phase(9));
        main_assert_eq!(entry.tooltip.as_deref() => Some("Let this player join in next game"));
        main_assert!(matches!(entry.action, Some(AppContextMenuCommand::AddStartupParticipant(_))));
    }
    main_assert!(add.iter().any(|entry| entry.text == "Broken"), "Add scans filenames without opening or parsing C4P groups");
    main_assert!(!add.iter().any(|entry| entry.text == "Deep"));

    let developer_players = install_root.join("build/DevPlayers");
    fs::create_dir_all(&developer_players).test_value();
    fs::write(developer_players.join("Late.C4P"), b"not parsed").test_value();
    let mut config = Config::new();
    config.set_in(Some("General"), "PlayerPath", "DevPlayers");
    config.set_in(Some("General"), "Participants", "");
    config.save(paths.config_file()).test_value();
    let developer_add = startup_participant_add_entries(&paths);
    main_assert_eq!(developer_add.len() => 1);
    main_assert_eq!(developer_add[0].text => "Late");
    main_assert_eq!(
        developer_add[0].action =>
        Some(AppContextMenuCommand::AddStartupParticipant(
            Path::new("DevPlayers")
                .join("Late.C4P")
                .to_string_lossy()
                .into_owned(),
        )),
        "developer ExePath variants use the same relative reference as player discovery"
    );
    reset_cached_app_paths();
}

#[test]
fn participant_context_menu_opens_recursively_adds_removes_and_allows_empty_children() {
    let _lock = env_lock().lock();
    let install_root = test_repository_root();
    let user_data = tempdir();
    let player_root = user_data.path().join("Players");
    let ada = player_root.join("Ada.c4p");
    let bob = player_root.join("Bob.c4p");
    fs::create_dir_all(&ada).test_value();
    fs::write(
        ada.join("Player.txt"),
        "[Player]\nName=Ada\n\n[Preferences]\nColorDw=255\n",
    )
    .test_value();
    let (_guard, paths) = guarded_test_app_paths(Some(install_root), user_data.path());
    let mut config = Config::new();
    config.set_in(Some("General"), "PlayerPath", player_root.to_string_lossy());
    config.set_in(
        Some("General"),
        "Participants",
        format!(
            "{};{};{};{}",
            ada.display(),
            player_root.join("Missing.c4p").display(),
            player_root.join("Notes.txt").display(),
            ada.display(),
        ),
    );
    fs::create_dir_all(paths.config_file().parent().test_value()).test_value();
    config.save(paths.config_file()).test_value();

    let mut app = GameApp::new(
        1280,
        720,
        disabled_audio_options(),
        Some(&paths),
        test_runtime_config_with("Player".to_string(), false),
    )
    .test_value();
    main_assert_eq!(startup_participant_references(&paths).expect("constructor validation") => vec![ada.to_string_lossy().into_owned()]);
    wait_for_menu(&mut app);

    let participant_rect = app
        .main_menu_state
        .menu
        .participants_rect(&app.main_menu_state.participants_label);
    let label_point = PhysicalPosition::new(
        f64::from(participant_rect.x + participant_rect.w / 2),
        f64::from(participant_rect.y + participant_rect.h / 2),
    );
    app.test_cursor(PhysicalPosition::new(
        f64::from(participant_rect.x - 1),
        f64::from(participant_rect.y),
    ));
    app.test_right_button(ElementState::Pressed);
    main_assert!(app.context_menu.is_none());

    let open = |app: &mut GameApp| {
        app.test_cursor(label_point);
        app.test_right_button(ElementState::Pressed);
        let layout = app.context_menu.test_ref().layout();
        main_assert_eq!(layout.panels.len() => 1);
        main_assert_eq!(layout.panels[0].rows.len() => 2);
        main_assert_eq!(layout.panels[0].selected => None);
    };
    let hover_root = |app: &mut GameApp, index: usize| {
        let row = app.context_menu.test_ref().layout().panels[0].rows[index].rect;
        app.test_cursor(PhysicalPosition::new(
            f64::from(row.x + 1),
            f64::from(row.y + 1),
        ));
    };
    let activate_child = |app: &mut GameApp, index: usize| {
        let layout = app.context_menu.test_ref().layout();
        let row = layout.panels[1].rows[index].rect;
        app.test_cursor(PhysicalPosition::new(
            f64::from(row.x + 1),
            f64::from(row.y + 1),
        ));
        app.test_left_button(ElementState::Pressed);
        app.test_left_button(ElementState::Released);
    };

    open(&mut app);
    hover_root(&mut app, 1);
    main_assert!(!app.startup_element_tooltip_pending(), "captured popup motion must suppress the underlying startup tooltip");
    app.close_context_menu_silently();
    main_assert!(!app.startup_element_tooltip_pending());

    open(&mut app);
    fs::create_dir_all(&bob).test_value();
    fs::write(
        bob.join("Player.txt"),
        "[Player]\nName=Bob\n\n[Preferences]\nColorDw=255\n",
    )
    .test_value();
    hover_root(&mut app, 0);
    let add_layout = app.context_menu.test_ref().layout();
    main_assert_eq!(add_layout.panels.len() => 2);
    main_assert_eq!(add_layout.panels[1].rows.len() => 1);
    activate_child(&mut app, 0);
    main_assert!(app.context_menu.is_none());
    main_assert_eq!(
        startup_participant_references(&paths).expect("read after Add") =>
        vec![
            ada.to_string_lossy().into_owned(),
            bob.to_string_lossy().into_owned(),
        ]
    );
    main_assert_eq!(app.main_menu_state.participants_label => "Players: Ada, Bob");

    open(&mut app);
    hover_root(&mut app, 0);
    let empty = app.context_menu.test_ref().layout();
    main_assert_eq!(empty.panels.len() => 2);
    main_assert!(empty.panels[1].rows.is_empty());
    main_assert_eq!((empty.panels[1].bounds.w, empty.panels[1].bounds.h) => (40, 7));
    app.close_context_menu_silently();

    open(&mut app);
    hover_root(&mut app, 1);
    let remove_layout = app.context_menu.test_ref().layout();
    main_assert_eq!(remove_layout.panels.len() => 2);
    main_assert_eq!(remove_layout.panels[1].rows.len() => 2);
    activate_child(&mut app, 1);
    main_assert_eq!(startup_participant_references(&paths).expect("read after Remove") => vec![ada.to_string_lossy().into_owned()]);
    main_assert_eq!(app.main_menu_state.participants_label => "Players: Ada");
    reset_cached_app_paths();
}

#[test]
fn player_context_menu_routes_recursively_without_generic_panes() {
    let _lock = env_lock().lock();
    let install_root = test_repository_root();
    let user_data = tempdir();
    let player_root = user_data.path().join("Players");
    for name in ["Ada", "Bob"] {
        let group = player_root.join(format!("{name}.c4p"));
        fs::create_dir_all(&group).test_value();
        fs::write(
            group.join("Player.txt"),
            format!("[Player]\nName={name}\n\n[Preferences]\nColorDw=255\n"),
        )
        .test_value();
    }
    let (_guard, paths) = guarded_test_app_paths(Some(install_root), user_data.path());
    let mut config = Config::new();
    config.set_in(Some("General"), "PlayerPath", player_root.to_string_lossy());
    config.set_in(
        Some("General"),
        "Participants",
        player_root.join("Ada.c4p").to_string_lossy(),
    );
    fs::create_dir_all(paths.config_file().parent().test_value()).test_value();
    config.save(paths.config_file()).test_value();

    let mut app = GameApp::new(
        1280,
        720,
        disabled_audio_options(),
        Some(&paths),
        test_runtime_config_with("Player".to_string(), false),
    )
    .test_value();
    wait_for_menu(&mut app);
    main_assert_eq!(app.startup_player_models.len() => 2);
    app.open_player_selection_dialog();

    let layout = clonk_frontend::startup_plrsel::plrsel_layout(1280, 720);
    let row_point = |index: usize| {
        PhysicalPosition::new(
            f64::from(layout.list_client.x + 2),
            f64::from(
                layout.list_client.y + layout.item_pitch * index as i32 + layout.item_height / 2,
            ),
        )
    };
    let open_on_row = |app: &mut GameApp, index: usize| {
        app.test_cursor(row_point(index));
        app.test_right_button(ElementState::Pressed);
    };

    let focus_before = app.startup_player_dialog.test_ref().focused_control();
    open_on_row(&mut app, 1);
    let popup = app.context_menu.test_ref();
    main_assert_eq!(popup.layout().panels.len() => 1);
    main_assert_eq!(popup.layout().panels[0].rows.len() => 2);
    main_assert_eq!(popup.layout().panels[0].selected => None);
    main_assert_eq!(app.startup_player_dialog.as_ref().expect("player controller").selected_index() => Some(1));
    main_assert_eq!(
        app.startup_player_dialog
            .as_ref()
            .expect("player controller")
            .focused_control() =>
        focus_before,
        "right-down selects the row without stealing keyboard focus"
    );

    let properties = popup.layout().panels[0].rows[0].rect;
    app.test_cursor(PhysicalPosition::new(
        f64::from(properties.x + 1),
        f64::from(properties.y + 1),
    ));
    app.test_left_button(ElementState::Pressed);
    main_assert!(app.context_menu.is_none());
    main_assert!(matches!(
        app.startup_player_properties_dialog
            .as_ref()
            .map(|pending| pending.controller.mode()),
        Some(clonk_frontend::startup_plrproperties::PlayerPropertiesMode::Edit { index: 1 })
    ));
    main_assert!(app.message_dialogs.is_empty());
    main_assert!(app.status_text.is_empty());
    main_assert_eq!(app.context_menu_pointer_capture => Some(ContextMenuPointerButton::Left));
    app.test_left_button(ElementState::Released);
    main_assert_eq!(app.context_menu_pointer_capture => None);
    app.test_key(VirtualKeyCode::Escape, ElementState::Pressed);
    app.test_key(VirtualKeyCode::Escape, ElementState::Released);

    open_on_row(&mut app, 1);
    let delete = app.context_menu.test_ref().layout().panels[0].rows[1].rect;
    app.test_cursor(PhysicalPosition::new(
        f64::from(delete.x + 1),
        f64::from(delete.y + 1),
    ));
    app.test_left_button(ElementState::Pressed);
    main_assert!(app.context_menu.is_none());
    main_assert_eq!(app.message_dialogs.len() => 1);
    main_assert_eq!(app.message_dialogs[0].state.caption() => "Delete");
    main_assert_eq!(app.message_dialogs[0].state.message() => "Do you really want to delete player Bob?");
    app.test_left_button(ElementState::Released);
    main_assert_eq!(app.message_dialogs.len() => 1);
    main_assert_eq!(app.context_menu_pointer_capture => None);
    app.finish_message_dialog(clonk_frontend::message_dialog::MessageDialogResult::No)
        .test_value();

    open_on_row(&mut app, 1);
    let slot = GamepadSlot::new(0);
    app.test_gamepad_events([
        gamepad_direction_event(slot, ControlButton::Down, ElementState::Pressed),
        gamepad_direction_event(slot, ControlButton::Down, ElementState::Pressed),
        gamepad_gui_button_event(slot, GuiButtonClass::Low, ElementState::Pressed),
        gamepad_action_event(slot, GamepadActionType::Select, ElementState::Pressed),
        gamepad_button_event(slot, LegacyGamepadButton::new(0), ElementState::Pressed),
    ]);
    main_assert!(app.context_menu.is_none());
    main_assert_eq!(app.message_dialogs.len() => 1);
    app.finish_message_dialog(clonk_frontend::message_dialog::MessageDialogResult::No)
        .test_value();
    app.test_gamepad_events([
        gamepad_gui_button_event(slot, GuiButtonClass::Low, ElementState::Released),
        gamepad_action_event(slot, GamepadActionType::Select, ElementState::Released),
        gamepad_button_event(slot, LegacyGamepadButton::new(0), ElementState::Released),
    ]);

    open_on_row(&mut app, 1);
    app.test_cursor(row_point(0));
    app.test_right_button(ElementState::Pressed);
    main_assert_eq!(app.startup_player_dialog.as_ref().expect("player controller").selected_index() => Some(0));
    main_assert!(app.context_menu.is_some(), "same down opens the first row popup");

    let mut with_context = vec![0_u8; 1280 * 720 * 4];
    main_assert!(app.render(&mut with_context).expect("render popup"));
    app.handle_focus_lost().test_value();
    main_assert!(app.context_menu.is_none());
    let mut without_context = vec![0_u8; 1280 * 720 * 4];
    main_assert!(app.render(&mut without_context).expect("render after close"));
    main_assert_ne!(with_context => without_context, "a closed popup must not ghost into the next frame");
    let stable = without_context.clone();
    main_assert!(app.render(&mut without_context).expect("recompose the clean frame"));
    main_assert_eq!(without_context => stable);

    open_on_row(&mut app, 1);
    app.resize(1024, 640).test_value();
    main_assert!(app.context_menu.is_none());
    reset_cached_app_paths();
}

#[test]
fn message_dialog_stack_closes_only_the_top_entry() {
    let mut app = new_menu_app(640, 480);
    for caption in ["First", "Second"] {
        app.push_message_dialog(
            clonk_frontend::message_dialog::MessageDialogState::regular_ok(
                caption,
                caption,
                clonk_frontend::message_dialog::MessageDialogIcon::NOTIFY,
            ),
            MessageDialogContinuation::None,
        )
        .test_value();
    }
    main_assert_eq!(app.message_dialogs.len() => 2);
    main_assert_eq!(app.message_dialogs[1].state.caption() => "Second");

    app.test_key(VirtualKeyCode::Escape, ElementState::Pressed);
    app.test_key(VirtualKeyCode::Escape, ElementState::Released);
    main_assert_eq!(app.message_dialogs.len() => 1);
    main_assert_eq!(app.message_dialogs[0].state.caption() => "First");
}

#[test]
fn message_dialog_focus_loss_cancels_held_input_and_stale_release_guards() {
    let mut app = new_menu_app(640, 480);
    app.push_message_dialog(
        clonk_frontend::message_dialog::MessageDialogState::regular_ok(
            "Message",
            "Caption",
            clonk_frontend::message_dialog::MessageDialogIcon::NOTIFY,
        ),
        MessageDialogContinuation::None,
    )
    .test_value();
    app.test_key(VirtualKeyCode::Enter, ElementState::Pressed);
    app.handle_focus_lost().test_value();
    app.test_key(VirtualKeyCode::Enter, ElementState::Released);
    main_assert_eq!(app.message_dialogs.len() => 1, "a release missing its pre-focus-loss press must not activate");

    app.test_key(VirtualKeyCode::Escape, ElementState::Pressed);
    main_assert!(app.message_dialog_consumed_keys.contains(&VirtualKeyCode::Escape));
    app.handle_focus_lost().test_value();
    main_assert!(app.message_dialog_consumed_keys.is_empty());
}

#[test]
fn gamepad_clear_cancels_pressed_modal_state_while_dialog_stays_open() {
    let mut app = new_menu_app(640, 480);
    let slot = GamepadSlot::new(0);
    app.push_message_dialog(
        clonk_frontend::message_dialog::MessageDialogState::regular_ok(
            "Message",
            "Caption",
            clonk_frontend::message_dialog::MessageDialogIcon::NOTIFY,
        ),
        MessageDialogContinuation::None,
    )
    .test_value();
    let source = |cluster, event| SourcedGamepadEvent {
        gamepad: 0,
        cluster,
        event,
    };
    app.process_sourced_gamepad_event_batch(
        [source(
            30,
            gamepad_gui_button_event(slot, GuiButtonClass::Low, ElementState::Pressed),
        )],
        true,
    )
    .test_value();

    app.process_sourced_gamepad_event_batch([source(31, GamepadEvent::Clear { slot })], true)
        .test_value();
    main_assert_eq!(app.message_dialogs.len() => 1);

    app.process_sourced_gamepad_event_batch(
        [
            source(
                32,
                gamepad_gui_button_event(slot, GuiButtonClass::Low, ElementState::Released),
            ),
            source(
                32,
                gamepad_action_event(slot, GamepadActionType::Select, ElementState::Released),
            ),
        ],
        true,
    )
    .test_value();
    main_assert_eq!(app.message_dialogs.len() => 1, "Clear cancels the pressed state before the fresh release cluster");

    app.test_key(VirtualKeyCode::Escape, ElementState::Pressed);
    app.test_key(VirtualKeyCode::Escape, ElementState::Released);
    app.test_gamepad_events([gamepad_gui_button_event(
        slot,
        GuiButtonClass::High,
        ElementState::Pressed,
    )]);
}

#[test]
fn configured_gamepad_button10_routes_player_menu_to_control_set_five_owner() {
    // Button10 is logical control index 9 (PlayerMenu). The stored value
    // is the full physical C++ keycode: slot 1/raw button 0 is
    // 0x0042010a = 4325642 (pristine 9ffa0a5d
    // src/C4KeyboardInput.h:57-80; src/C4Game.cpp:3439-3452;
    // src/C4ObjectCom.cpp:874-900; src/C4Constants.h:84-93).
    let mut config = Config::new();
    config.set_in(Some("Gamepad1"), "Button10", "4325642");

    let mut app = new_running_sandbox_app();
    app.gamepad_bindings = GamepadBindings::from_config(&config);
    app.local_controls = LocalControlRegistry::default();
    app.local_controls
        .initialize(test_local_control_init(app.local_owner, 5, false, false));

    app.test_gamepad_events([gamepad_button_event(
        GamepadSlot::new(1),
        LegacyGamepadButton::new(0),
        ElementState::Pressed,
    )]);

    main_assert!(app.ingame_menu.is_some(), "Button10 must dispatch PlayerMenu to the control-set 5 owner");
}

#[test]
fn modal_message_dialog_keeps_running_simulation_and_clock_alive() {
    let mut app = new_menu_app(640, 480);
    app.start_sandbox_scenario(FrontendScenario::fallback())
        .test_value();
    let frame = app.engine.frame();
    let game_time = app.engine.game_time();
    app.push_message_dialog(
        clonk_frontend::message_dialog::MessageDialogState::regular_ok(
            "Message",
            "Caption",
            clonk_frontend::message_dialog::MessageDialogIcon::NOTIFY,
        ),
        MessageDialogContinuation::None,
    )
    .test_value();

    app.test_update();
    main_assert!(app.sec1_timer().expect("modal clock pulse"), "modal loop must keep the game clock alive");
    main_assert_eq!(app.engine.frame() => frame + 1);
    main_assert_eq!(app.engine.game_time() => game_time + 1);
}

#[test]
fn message_dialog_malformed_specific_assets_fail_before_modal_mutation() {
    let mut app = new_menu_app(640, 480);
    Arc::get_mut(&mut app.assets)
        .test_value()
        .startup_dialog_images
        .insert(
            "GUIIcons.png".to_string(),
            ImageData::new(1, 1, vec![255, 255, 255, 255]),
        );
    let error = app
        .push_message_dialog(
            clonk_frontend::message_dialog::MessageDialogState::regular_ok(
                "Message",
                "Caption",
                clonk_frontend::message_dialog::MessageDialogIcon::ERROR,
            ),
            MessageDialogContinuation::None,
        )
        .expect_err("malformed message resources must fail at open time");
    main_assert!(matches!(
        error,
        EngineError::ClassicMenuParityBoundary { ref detail }
            if detail.contains("C4GUI::MessageDialog")
                && detail.contains("GUIIcons.png")
    ));
    main_assert!(app.message_dialogs.is_empty());
}

#[test]
fn menu_input_changes_the_composed_frame() {
    clonk_logging::init();

    let mut app = new_real_classic_menu_app(320, 200);
    let mut before = vec![0u8; 320 * 200 * 4];
    app.test_render(&mut before);
    app.test_key(VirtualKeyCode::ArrowDown, ElementState::Pressed);
    let mut after = vec![0u8; 320 * 200 * 4];
    app.test_render(&mut after);
    main_assert_ne!(before => after, "input events must change what the menu presents");
}

#[test]
fn menu_backdrop_restore_matches_full_recomposition() {
    clonk_logging::init();

    let mut app = new_real_classic_menu_app(320, 200);
    let len = 320 * 200 * 4;
    let mut first = vec![0u8; len];
    app.test_render(&mut first);

    // A recomposition that restores the cached static backdrop...
    let mut restored = vec![0u8; len];
    app.test_render(&mut restored);

    // ...must match one composed from scratch.
    app.menu_backdrop_cache = StartupBackdropCache::default();
    let mut recomposed = vec![0u8; len];
    app.test_render(&mut recomposed);

    main_assert_eq!(restored => recomposed, "backdrop restore must be pixel-identical to a full recomposition");
    main_assert_eq!(first => restored, "unchanged menu state must keep rendering identical frames");
}

#[test]
fn menu_resize_renders_at_new_dimensions() {
    clonk_logging::init();

    let mut app = new_real_classic_menu_app(320, 200);
    let mut frame = vec![0u8; 320 * 200 * 4];
    app.test_render(&mut frame);
    app.resize(400, 300).test_value();
    let mut larger = vec![0u8; 400 * 300 * 4];
    app.test_render(&mut larger);
    let surface = app.graphics.surface();
    main_assert_eq!((surface.width(), surface.height()) => (400, 300), "the composed surface must track the resized window");
    main_assert!(larger.iter().any(|byte| *byte != 0), "the resized menu must reach the enlarged frame");
}

/// `General.Participants` is a `CFG_MaxString` escaped-string field
/// (`is_cpp_escaped_config_field`), so C++ reads and writes it quoted. The
/// deferred store flushed it through the raw writer, which emits it bare — a
/// shape a LegacyClonk install sharing the file does not read back as the same
/// value.
#[test]
fn a_deferred_participant_list_is_flushed_in_its_quoted_native_form() {
    let _lock = env_lock().lock();
    let fixture = tempdir();
    let (_guard, paths) = exact_loader_test_paths(fixture.path(), None);
    fs::create_dir_all(paths.config_file().parent().test_value()).test_value();
    fs::write(
        paths.config_file(),
        b"[General]\r\nParticipants=\"Old.c4p\"\r\n",
    )
    .test_value();
    let mut app = new_state_only_menu_app(320, 200);
    app.app_paths = Some(paths.clone());

    app.defer_participant_list("Alice.c4p;Bob.c4p");
    main_assert_eq!(app.deferred_config.get("General", "Participants") => Some("Alice.c4p;Bob.c4p"), "the running session reads its own pending list");

    app.flush_deferred_config();

    let native = fs::read(paths.config_file()).test_value();
    let expected = b"Participants=\"Alice.c4p;Bob.c4p\"";
    main_assert!(
        native
            .windows(expected.len())
            .any(|window| window == expected),
        "flushed config kept the quoted native form, got {}",
        String::from_utf8_lossy(&native)
    );
    reset_cached_app_paths();
}
