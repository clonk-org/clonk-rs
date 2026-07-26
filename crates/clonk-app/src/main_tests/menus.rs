// Spliced into `mod tests` (src/main_tests.rs) via include!: a bare item
// sequence, not a child module, so test ids stay `tests::<fn>`.

    #[test]
    fn l053_help_suppresses_open_ingame_menu_and_right_up_exits() {
        let mut app = new_classic_running_sandbox_app();
        let owner = app.local_owner;
        app.activate_ingame_main_menu_for_player(owner)
            .expect("open player menu for Help interception");
        render_mouse_test_app(&mut app);
        let (width, height) = {
            let surface = app.graphics.surface();
            (surface.width(), surface.height())
        };
        let close = (0..height)
            .flat_map(|y| (0..width).map(move |x| (x, y)))
            .find_map(|(x, y)| {
                let point = GuiPoint::new(x as f32 + 0.5, y as f32 + 0.5);
                (app.ingame_menu_pointer_target(point)
                    == Some((owner, IngameMenuPointerTarget::Close)))
                .then_some(point)
            })
            .expect("player menu exposes its close button");
        let mut commands = install_mouse_network_capture(&mut app);

        app.ingame_mouse_help = true;
        physical_left_click_with_modifiers(
            &mut app,
            close,
            ModifiersState::empty(),
            ModifiersState::empty(),
        );
        assert!(app.ingame_mouse_help);
        assert!(
            app.ingame_menu_belongs_to(owner),
            "Help suppresses already-open player-menu controls"
        );
        assert_eq!(
            commands.take_submitted_mouse_controls(),
            (Vec::new(), Vec::new(), Vec::new())
        );

        app.handle_right_mouse_button(ElementState::Pressed)
            .expect("Help right-down over player menu");
        assert!(app.ingame_mouse_help);
        app.handle_right_mouse_button(ElementState::Released)
            .expect("Help right-up over player menu");
        assert!(!app.ingame_mouse_help);
        assert!(app.ingame_menu_belongs_to(owner));
        assert_eq!(
            commands.take_submitted_mouse_controls(),
            (Vec::new(), Vec::new(), Vec::new()),
            "Help menu interception queues no controls"
        );
    }

    #[test]
    fn l053_help_right_up_exits_without_context_or_crew_cycle() {
        let mut app = new_running_sandbox_app();
        let owner = app.local_owner;
        let (_target, point) =
            install_mouse_help_target(&mut app, "HLP3", "Right target", None);
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
            assert!(app.ingame_mouse_help_caption.is_some());
            app.handle_cursor_moved(PhysicalPosition::new(
                f64::from(release.x),
                f64::from(release.y),
            ))
            .expect("move Help right-click");
            app.handle_right_mouse_button(ElementState::Pressed)
                .expect("Help right-down");
            assert!(app.ingame_mouse_help, "right-down retains Help");
            app.handle_right_mouse_button(ElementState::Released)
                .expect("Help right-up");
            assert!(!app.ingame_mouse_help, "right-up exits Help");
            assert_eq!(app.engine.crew_cursor(owner), cursor);
            assert_eq!(
                commands.take_submitted_mouse_controls(),
                (Vec::new(), Vec::new(), Vec::new()),
                "Help right-up queues neither Context nor player selection"
            );
            assert_eq!(
                app.ingame_mouse_help_caption,
                Some(IngameMouseHelpCaption {
                    text: "Right target".to_string(),
                    keep_moves: 0,
                }),
                "right-up clears KeepCaption without erasing the caption immediately"
            );
            app.update_ingame_pointer(release)
                .expect("the next Move clears the zero-lifetime caption");
            assert!(app.ingame_mouse_help_caption.is_none());
        }
    }

    #[test]
    fn viewport_buttons_dispatch_help_and_player_menu_locally() {
        let mut app = new_running_sandbox_app();
        let owner = app.local_owner;
        render_mouse_test_app(&mut app);
        assert_eq!(app.local_controls.mouse_owner(), Some(owner));

        let help = viewport_button_point(&app, owner, clonk_frontend::hud::ViewportButton::Help);
        let menu =
            viewport_button_point(&app, owner, clonk_frontend::hud::ViewportButton::PlayerMenu);
        let chat = viewport_button_point(&app, owner, clonk_frontend::hud::ViewportButton::Chat);
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
        assert_eq!(
            app.ingame_viewport_region(owner, chat),
            None,
            "the pending external IRC frontend keeps Chat inactive"
        );

        let mut network_commands = install_mouse_network_capture(&mut app);
        physical_left_click_with_modifiers(
            &mut app,
            help,
            ModifiersState::empty(),
            ModifiersState::empty(),
        );
        assert!(app.ingame_mouse_help);
        assert_eq!(
            network_commands.take_submitted_player_inputs(),
            (Vec::new(), Vec::new(), Vec::new()),
            "COM_Help remains process-local"
        );

        app.ingame_mouse_help = false;
        assert!(!app.ingame_menu_belongs_to(owner));
        physical_left_click_with_modifiers(
            &mut app,
            menu,
            ModifiersState::empty(),
            ModifiersState::empty(),
        );
        assert!(app.ingame_menu_belongs_to(owner));
        assert_eq!(
            network_commands.take_submitted_player_inputs(),
            (Vec::new(), Vec::new(), Vec::new()),
            "mouse COM_PlayerMenu is consumed by the local menu"
        );

        app.ingame_menu
            .get_mut(owner)
            .expect("mouse menu is open")
            .set_selection(2);
        physical_left_click_with_modifiers(
            &mut app,
            menu,
            ModifiersState::empty(),
            ModifiersState::empty(),
        );
        assert_eq!(
            app.ingame_menu
                .get(owner)
                .expect("mouse menu remains open")
                .selection(),
            0,
            "a second mouse activation reinitializes the main menu"
        );
        assert_eq!(
            network_commands.take_submitted_player_inputs(),
            (Vec::new(), Vec::new(), Vec::new()),
            "reinitializing the mouse menu remains entirely local"
        );

        app.display_flags.show_commands = false;
        assert_eq!(app.ingame_viewport_region(owner, help), None);
        assert_eq!(app.ingame_viewport_region(owner, menu), None);
    }

    #[test]
    fn ownerless_mouse_viewport_buttons_remain_local_and_open_fullscreen_menu() {
        let mut app = new_classic_running_sandbox_app();
        let removed_owner = app.local_owner;
        app.engine
            .remove_player(removed_owner)
            .expect("remove local player for passive observer");
        app.engine.set_local_players([]);
        app.local_controls = LocalControlRegistry::default();
        app.mouse_control = false;
        render_mouse_test_app(&mut app);

        let viewport = app
            .active_ingame_mouse_viewport()
            .expect("ownerless observer viewport");
        assert_eq!(viewport.owner, OWNER_NONE);
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
        assert_eq!(app.ingame_viewport_region(OWNER_NONE, world), None);
        app.handle_cursor_moved(PhysicalPosition::new(
            f64::from(world.x),
            f64::from(world.y),
        ))
        .expect("move passive observer over world");
        app.handle_mouse_button(ElementState::Pressed)
            .expect("passive world left-down remains inert");
        assert!(
            app.mouse_state.is_none(),
            "passive observers never enter native DragNone world state"
        );
        app.handle_mouse_button(ElementState::Released)
            .expect("release inert passive world click");

        let mut network_commands = install_mouse_network_capture(&mut app);
        app.ingame_last_left_down = None;
        let help = center(help_rect);
        app.handle_cursor_moved(PhysicalPosition::new(
            f64::from(help.x),
            f64::from(help.y),
        ))
        .expect("move passive observer over Help");
        app.handle_mouse_button(ElementState::Pressed)
            .expect("press passive observer Help");
        assert!(!app.ingame_mouse_help, "passive buttons wait for LeftUp");
        app.handle_mouse_button(ElementState::Released)
            .expect("release passive observer Help");
        assert!(app.ingame_mouse_help);
        assert!(
            app.ingame_help_cursor_active(),
            "ownerless Help uses the native Help cursor too"
        );
        assert!(app.ingame_menu.is_none());

        app.handle_right_mouse_button(ElementState::Pressed)
            .expect("press passive observer right button in Help");
        assert!(app.ingame_mouse_help, "right-down retains passive Help");
        app.handle_right_mouse_button(ElementState::Released)
            .expect("release passive observer right button in Help");
        assert!(!app.ingame_mouse_help, "right-up exits passive Help");

        app.ingame_last_left_down = None;
        let menu = center(menu_rect);
        app.handle_cursor_moved(PhysicalPosition::new(
            f64::from(menu.x),
            f64::from(menu.y),
        ))
        .expect("move passive observer over PlayerMenu");
        app.handle_mouse_button(ElementState::Pressed)
            .expect("press passive observer PlayerMenu");
        assert!(app.ingame_menu.is_none(), "passive buttons wait for LeftUp");
        app.handle_mouse_button(ElementState::Released)
            .expect("release passive observer PlayerMenu");
        assert!(app.ingame_menu_belongs_to(OWNER_NONE));
        assert_eq!(
            app.ingame_menu
                .get(OWNER_NONE)
                .expect("observer fullscreen menu")
                .page(),
            ingame_menu::MenuPage::Main
        );

        render_mouse_test_app(&mut app);
        let surface = app.graphics.surface();
        let menu_target = (0..surface.height())
            .flat_map(|y| (0..surface.width()).map(move |x| (x, y)))
            .find_map(|(x, y)| {
                let point = GuiPoint::new(x as f32 + 0.5, y as f32 + 0.5);
                match app.ingame_menu_pointer_target(point) {
                    Some((OWNER_NONE, IngameMenuPointerTarget::Item(index))) => {
                        Some((point, index))
                    }
                    _ => None,
                }
            })
            .expect("fullscreen observer menu exposes pointer-owned items");
        app.handle_cursor_moved(PhysicalPosition::new(
            f64::from(menu_target.0.x),
            f64::from(menu_target.0.y),
        ))
        .expect("move over observer fullscreen menu");
        assert_eq!(
            app.ingame_menu
                .get(OWNER_NONE)
                .expect("observer menu remains open")
                .selection(),
            menu_target.1
        );

        app.handle_key(VirtualKeyCode::Escape, ElementState::Pressed)
            .expect("Escape closes observer fullscreen menu locally");
        assert!(app.ingame_menu.is_none());
        assert_eq!(
            network_commands.take_submitted_player_inputs(),
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
            .expect("Buy COM_Up region");
        let (manager, _events, mut network_commands) =
            NetworkManager::test_stub_with_commands_for_client_id(7);
        app.network = Some(manager);

        app.handle_cursor_moved(PhysicalPosition::new(
            f64::from(point.x),
            f64::from(point.y),
        ))
        .expect("move onto Buy region");
        app.handle_ingame_mouse_button(ElementState::Pressed)
            .expect("AutoStop Buy left-down");
        let (controls, commands, selections) =
            network_commands.take_submitted_player_inputs();
        assert!(commands.is_empty());
        assert!(selections.is_empty());
        let [(queued_owner, event, tick)] = controls.as_slice() else {
            panic!("expected one queued Buy press, got {controls:?}");
        };
        assert_eq!(*queued_owner, owner);
        assert_eq!(
            *event,
            ControlEvent::RawPlayerControl {
                command: 3,
                data: 0,
            }
        );
        app.apply_ready_controls(
            *tick,
            vec![NetworkControl::Player {
                owner,
                event: *event,
            }],
        )
        .expect("execute queued Buy press");
        assert!(
            app.engine.cursor_object_menu(owner).is_some(),
            "COM_Up must open the contained base Buy menu before button-up"
        );
        assert_eq!(
            app.script_menu_pointer_target(point)
                .expect("hit-test opened Buy menu"),
            None,
            "the command-bar release point remains outside the GUI-owned menu"
        );

        app.handle_ingame_mouse_button(ElementState::Released)
            .expect("captured AutoStop Buy left-up");
        let (controls, commands, selections) =
            network_commands.take_submitted_player_inputs();
        assert_eq!(
            controls,
            vec![(
                owner,
                ControlEvent::RawPlayerControl {
                    command: 19,
                    data: 0,
                },
                *tick,
            )]
        );
        assert!(commands.is_empty());
        assert!(selections.is_empty());
    }

    #[test]
    fn script_menu_owns_threshold_crossing_inventory_drag_move() {
        let (mut app, owner, cursor, _first, _target, inventory_point) =
            inventory_region_fixture();
        install_classic_test_assets(&mut app);
        install_test_cursor_menu(&mut app, cursor, two_item_script_menu(cursor));
        assert!(app
            .ingame_inventory_region_target(owner, inventory_point)
            .is_some());
        assert_eq!(
            app.script_menu_pointer_target(inventory_point)
                .expect("hit-test inventory point"),
            None,
            "inventory down begins outside the open GUI menu"
        );
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
            .expect("open script menu has an item beyond the drag threshold");

        app.ingame_last_left_down = None;
        app.handle_cursor_moved(PhysicalPosition::new(
            f64::from(inventory_point.x),
            f64::from(inventory_point.y),
        ))
        .expect("move onto inventory region");
        app.handle_ingame_mouse_button(ElementState::Pressed)
            .expect("inventory left-down");
        app.handle_cursor_moved(PhysicalPosition::new(
            f64::from(menu_point.x),
            f64::from(menu_point.y),
        ))
        .expect("move into GUI menu");
        assert!(app.mouse_state.is_some_and(|state| {
            !state.motion.moved
                && !state.motion.region_drag_started
                && state.motion.region_drag_cursor.is_none()
        }));
        assert!(app.ingame_dragged_objects.is_empty());
        app.handle_ingame_mouse_button(ElementState::Released)
            .expect("GUI-owned menu release");
        assert!(app.mouse_state.is_none());
    }

    #[test]
    fn real_goldrush_talker_opens_the_shipped_decorated_dialog() {
        let mut app =
            real_installed_scenario_app("Western.c4f/Goldrush.c4s", "Goldrush dialog parity");
        let mut baseline = vec![0_u8; 320 * 200 * 4];
        app.render(&mut baseline).expect("baseline renders");
        app.engine
            .tick()
            .expect("settle native ATTACH containment callbacks");

        let owner = app.local_owner;
        let snapshot = app.engine.snapshot();
        let captain = snapshot
            .objects
            .iter()
            .find(|object| {
                object.definition_id == "CVRM" && object.custom_name.as_deref() == Some("Captain")
            })
            .map(|object| object.id)
            .expect("placed Goldrush captain");
        let talker = snapshot
            .objects
            .iter()
            .find(|object| {
                object.definition_id == "_TLK" && object.custom_name.as_deref() == Some("Captain")
            })
            .map(|object| object.id)
            .expect("captain's attached Talker");
        assert_eq!(
            app.engine
                .object_snapshot(talker)
                .expect("Talker remains live")
                .action
                .target,
            Some(captain)
        );
        let cursor = app
            .engine
            .crew_cursor(owner)
            .expect("local Goldrush cursor");
        let talker_index = app
            .engine
            .find_object_index(talker)
            .expect("Talker vector index");
        let result = app
            .engine
            .call_object_function(
                talker_index,
                "ActivateEntrance",
                vec![Value::Object(cursor.as_u64())],
            )
            .expect("shipped Talker entrance activation runs");
        assert!(result.as_bool());

        let (_, first_menu) = app
            .engine
            .cursor_object_menu(owner)
            .expect("DlgCaptainStart opens the player's first line");
        assert_eq!(first_menu.style, 3);
        app.engine
            .player_in_com(owner, clonk_engine::COM_MENU_SHOW_TEXT, 0)
            .expect("reveal the first line");
        app.engine
            .player_in_com(owner, clonk_engine::COM_MENU_CLOSE, 0)
            .expect("close the first line");
        app.engine.tick().expect("advance to DlgCaptain1");

        let (_, menu) = app
            .engine
            .cursor_object_menu(owner)
            .expect("DlgCaptain1 installs the captain's cursor dialog");
        assert_eq!(menu.style, 3);
        assert_eq!(menu.extra, clonk_engine::ObjectMenuExtra::None);
        assert_eq!(menu.items.len(), 2);
        assert!(menu.text_progressing);
        assert!(matches!(
            &menu.items[0].image,
            clonk_engine::ObjectMenuImage::TextSpec { spec, .. }
                if spec.ends_with("::Captain1")
        ));
        let decoration = menu.decoration.as_ref().expect("Western MD69 decoration");
        assert_eq!(decoration.source_definition, "MD69");
        assert_eq!(decoration.background_color, 0x803f3f00);
        assert_eq!(
            (
                decoration.border_top,
                decoration.border_left,
                decoration.border_right,
                decoration.border_bottom,
            ),
            (10, 10, 10, 10)
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
            let facet = facet.expect("all Western MD69 facets exist");
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
        assert_eq!(
            app.engine
                .definition_named_portrait_graphics_image("CVRM", "Captain1")
                .map(|image| (image.width(), image.height())),
            Some((150, 150))
        );
        assert_eq!(
            app.engine
                .definition_sprite_image("MD69", None)
                .map(|image| (image.width(), image.height())),
            Some((128, 128))
        );

        app.snapshot = app.engine.snapshot();
        app.snapshot.hud.messages.clear();
        let mut rendered = vec![0_u8; 320 * 200 * 4];
        app.render(&mut rendered)
            .expect("shipped decorated Dialog renders without fallback");
        assert_ne!(rendered, baseline);
    }

    #[test]
    fn l040_audio_context_selects_configured_linear_resampling() {
        let audio = AudioContext::try_new(AudioOptions {
            prefer_linear_resampling: true,
            ..AudioOptions::default()
        })
        .expect("audio context");

        assert_eq!(audio.system.resampling_mode(), ResamplingMode::Linear);
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
            .register_definition(
                Definition::from_script("MENU", "Menu", script).expect("menu compiles"),
            )
            .expect("menu registers");
        let object = engine
            .spawn_object(SpawnConfig::new("MENU"))
            .expect("menu object spawns");
        let menu = engine
            .debug_object_menu(object.as_u64())
            .expect("menu object exists")
            .expect("Info menu exists");

        let error =
            resolve_script_menu_font_images(&engine, &menu, ScriptTextSpecResources::default())
                .expect_err("missing text image must fail before rendering");
        assert!(error.to_string().contains("{{MISS}}"));
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
                GlobalMessageViewportGeometry {
                    x: 177,
                    y: 73,
                    width: 96,
                },
                Rect::new(177, 73, 101, 65),
            ),
            (
                "Tutorial03",
                Vector2::new(10, -50),
                35,
                FLAG_BOTTOM | FLAG_LEFT | FLAG_WIDTH_REL | FLAG_X_REL,
                GlobalMessageViewportGeometry {
                    x: 49,
                    y: -27,
                    width: 112,
                },
                Rect::new(49, 149, 101, 65),
            ),
            (
                "Tutorial04/06",
                Vector2::new(10, -30),
                35,
                FLAG_BOTTOM | FLAG_LEFT | FLAG_WIDTH_REL | FLAG_X_REL,
                GlobalMessageViewportGeometry {
                    x: 49,
                    y: -7,
                    width: 112,
                },
                Rect::new(49, 169, 101, 65),
            ),
            (
                "Tutorial05",
                Vector2::new(10, -10),
                35,
                FLAG_BOTTOM | FLAG_LEFT | FLAG_WIDTH_REL | FLAG_X_REL,
                GlobalMessageViewportGeometry {
                    x: 49,
                    y: 13,
                    width: 112,
                },
                Rect::new(49, 189, 101, 65),
            ),
            (
                "Tutorial07-10",
                Vector2::new(0, 30),
                0,
                FLAG_HCENTER | FLAG_TOP,
                GlobalMessageViewportGeometry {
                    x: 17,
                    y: 53,
                    width: 0,
                },
                Rect::new(127, 53, 101, 65),
            ),
        ] {
            let geometry = global_message_viewport_geometry(viewport, offset, width, flags);
            assert_eq!(geometry, expected_geometry, "{tutorials}");
            assert_eq!(
                global_portrait_frame_rect(viewport, offset, flags, frame_size),
                expected_frame,
                "{tutorials}"
            );
        }
    }

    #[test]
    fn inventory_and_menu_color_modulation_alpha_fades_without_filling_background() {
        let mut definition =
            Definition::from_script("FADE", "Fade", "").expect("definition compiles");
        definition.set_picture(Some(clonk_engine::DefinitionPicture {
            x: 0,
            y: 0,
            width: 2,
            height: 1,
        }));
        definition.set_sprite_image(Some(clonk_engine::DefinitionSpriteImage {
            width: 2,
            height: 1,
            pixels: Arc::from([
                0, 0, 0, 0xff, // opaque texel
                0, 0, 0, 0, // transparent background texel
            ]),
            color_mask: None,
        }));
        let mut engine = Engine::new();
        engine
            .register_definition(definition)
            .expect("definition registers");

        let mut object = make_object(1, "FADE", Vector2::ZERO);
        object.color_modulation = 0x00ff_ffff;
        let unchanged = inventory_object_picture(&engine, &object).expect("unfaded picture");
        assert_eq!(unchanged.pixels(), &[0, 0, 0, 0xff, 0, 0, 0, 0]);

        object.color_modulation = 0x80ff_ffff;
        let inventory = inventory_object_picture(&engine, &object).expect("inventory picture");
        assert_eq!(
            inventory.pixels(),
            &[0, 0, 0, 0x7f, 0, 0, 0, 0],
            "the fast picture path subtracts C4 transparency from texel opacity"
        );

        let item = clonk_engine::ObjectMenuItem {
            caption: "Fade".to_string(),
            info_caption: String::new(),
            command: String::new(),
            command2: String::new(),
            count: 1,
            item_id: "FADE".to_string(),
            symbol: clonk_engine::ObjectMenuSymbol::Definition,
            image: clonk_engine::ObjectMenuImage::Object { object: object.id },
            presentation_definition_id: Some("FADE".to_string()),
            picture_snapshot: Some(clonk_engine::ObjectMenuPictureSnapshot {
                definition_id: "FADE".to_string(),
                symbol_size: 2,
                base_graphics: None,
                graphics_overlays: Vec::new(),
                blit_mode: 0,
                color: 0,
                color_modulation: 0x80ff_ffff,
                picture_rect: clonk_engine::DefinitionRect::default(),
                rank: None,
            }),
            picture_object: None,
            components: Vec::new(),
            selectable: true,
            value: None,
            text_display_progress: -1,
        };
        let menu = object_menu_item_picture(
            &engine,
            &make_snapshot(Vec::new(), Vec::new()),
            &item,
            0,
            &HudGraphics::default(),
            0,
        )
        .expect("cached menu picture");
        assert_eq!((menu.width(), menu.height()), (2, 2));
        assert_eq!(
            menu.pixels(),
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
            assert!((actual - expected).abs() < 1.0e-5, "{actual} != {expected}");
        }

        let (x, y) = transform.transform_point(10.0, 6.0);
        assert!((x - 14.0).abs() < 1.0e-5);
        assert!(y.abs() < 1.0e-5);
    }

    #[test]
    fn script_menu_images_use_resolved_definition_phase_and_color() {
        let mut definition =
            Definition::from_script("PHAS", "Phases", "").expect("phase definition compiles");
        definition.set_picture(Some(clonk_engine::DefinitionPicture {
            x: 0,
            y: 0,
            width: 1,
            height: 1,
        }));
        definition.set_sprite_image(Some(clonk_engine::DefinitionSpriteImage {
            width: 2,
            height: 1,
            pixels: Arc::from([0xff, 0, 0, 0xff, 0xff, 0xff, 0xff, 0xff]),
            color_mask: Some(Arc::from([0_u8, 0xff])),
        }));
        let mut engine = Engine::new();
        engine
            .register_definition(definition)
            .expect("phase definition registers");
        let snapshot = make_snapshot(Vec::new(), Vec::new());
        let item = clonk_engine::ObjectMenuItem {
            caption: "Indexed color".to_string(),
            info_caption: String::new(),
            command: String::new(),
            command2: String::new(),
            count: 12_345_678,
            item_id: "MISS".to_string(),
            symbol: clonk_engine::ObjectMenuSymbol::Definition,
            image: clonk_engine::ObjectMenuImage::IndexedColor {
                index: 1,
                color: 0x445566,
            },
            presentation_definition_id: Some("PHAS".to_string()),
            picture_snapshot: None,
            picture_object: None,
            components: Vec::new(),
            selectable: false,
            value: None,
            text_display_progress: -1,
        };

        let picture =
            object_menu_item_picture(&engine, &snapshot, &item, 0, &HudGraphics::default(), 0)
                .expect("resolved indexed picture");
        assert_eq!(picture.pixels(), &[0x44, 0x55, 0x66, 0xff]);

        let text_spec = resolve_script_font_image(
            &engine,
            "PHAS: +1trailing",
            0x112233,
            ScriptTextSpecResources::default(),
        )
        .expect("scanf-style TextSpec phase resolves");
        assert_eq!(text_spec.pixels(), &[0x11, 0x22, 0x33, 0xff]);
    }

    #[test]
    fn script_object_menu_image_survives_source_object_deletion() {
        let mut definition =
            Definition::from_script("OBJC", "Object", "").expect("definition compiles");
        definition.set_picture(Some(clonk_engine::DefinitionPicture {
            x: 0,
            y: 0,
            width: 1,
            height: 1,
        }));
        definition.set_sprite_image(Some(clonk_engine::DefinitionSpriteImage {
            width: 1,
            height: 1,
            pixels: Arc::from([0xff, 0xff, 0xff, 0xff]),
            color_mask: Some(Arc::from([0xff_u8])),
        }));
        let mut engine = Engine::new();
        engine
            .register_definition(definition)
            .expect("object definition registers");
        let item = clonk_engine::ObjectMenuItem {
            caption: "Object".to_string(),
            info_caption: String::new(),
            command: String::new(),
            command2: String::new(),
            count: 12_345_678,
            item_id: "NONE".to_string(),
            symbol: clonk_engine::ObjectMenuSymbol::Definition,
            image: clonk_engine::ObjectMenuImage::Object {
                object: ObjectId::new(7),
            },
            presentation_definition_id: Some("OBJC".to_string()),
            picture_snapshot: Some(clonk_engine::ObjectMenuPictureSnapshot {
                definition_id: "OBJC".to_string(),
                symbol_size: 35,
                base_graphics: None,
                graphics_overlays: Vec::new(),
                blit_mode: 0,
                color: 0x123456,
                color_modulation: 0,
                picture_rect: clonk_engine::DefinitionRect::default(),
                rank: None,
            }),
            picture_object: None,
            components: Vec::new(),
            selectable: false,
            value: None,
            text_display_progress: -1,
        };
        let empty_snapshot = make_snapshot(Vec::new(), Vec::new());

        let picture = object_menu_item_picture(
            &engine,
            &empty_snapshot,
            &item,
            0,
            &HudGraphics::default(),
            0,
        )
        .expect("cached picture remains after source deletion");
        assert_eq!(picture.pixels(), &[0x12, 0x34, 0x56, 0xff]);
    }

    #[test]
    fn script_object_menu_overlay_uses_owned_square_and_aspect_fit() {
        let mut engine = Engine::new();
        let mut base = Definition::from_script("BASE", "Base", "").expect("base compiles");
        base.set_picture(Some(clonk_engine::DefinitionPicture {
            x: 0,
            y: 0,
            width: 2,
            height: 1,
        }));
        base.set_sprite_image(Some(clonk_engine::DefinitionSpriteImage {
            width: 2,
            height: 1,
            pixels: Arc::from([0xff, 0, 0, 0xff, 0xff, 0, 0, 0xff]),
            color_mask: None,
        }));
        engine.register_definition(base).expect("base registers");
        let mut overlay = Definition::from_script("OVRL", "Overlay", "").expect("overlay compiles");
        overlay.set_picture(Some(clonk_engine::DefinitionPicture {
            x: 0,
            y: 0,
            width: 1,
            height: 1,
        }));
        overlay.set_sprite_image(Some(clonk_engine::DefinitionSpriteImage {
            width: 1,
            height: 1,
            pixels: Arc::from([0, 0, 0xff, 0xff]),
            color_mask: None,
        }));
        engine
            .register_definition(overlay)
            .expect("overlay registers");
        let item = clonk_engine::ObjectMenuItem {
            caption: "Composite".to_string(),
            info_caption: String::new(),
            command: String::new(),
            command2: String::new(),
            count: 12_345_678,
            item_id: "NONE".to_string(),
            symbol: clonk_engine::ObjectMenuSymbol::Definition,
            image: clonk_engine::ObjectMenuImage::Object {
                object: ObjectId::new(9),
            },
            presentation_definition_id: Some("BASE".to_string()),
            picture_snapshot: Some(clonk_engine::ObjectMenuPictureSnapshot {
                definition_id: "BASE".to_string(),
                symbol_size: 4,
                base_graphics: None,
                graphics_overlays: vec![
                    clonk_engine::ObjectGraphicsOverlay::new(
                        1,
                        clonk_engine::GraphicsOverlayMode::Picture,
                    )
                    .with_definition(Some("OVRL".to_string())),
                ],
                blit_mode: 0,
                color: 0,
                color_modulation: 0,
                picture_rect: clonk_engine::DefinitionRect::default(),
                rank: None,
            }),
            picture_object: None,
            components: Vec::new(),
            selectable: false,
            value: None,
            text_display_progress: -1,
        };
        let picture = object_menu_item_picture(
            &engine,
            &make_snapshot(Vec::new(), Vec::new()),
            &item,
            0,
            &HudGraphics::default(),
            0,
        )
        .expect("owned picture composite");
        assert_eq!((picture.width(), picture.height()), (4, 4));
        for (index, pixel) in picture.pixels().chunks_exact(4).enumerate() {
            let row = index / 4;
            let blue = if (1..3).contains(&row) { 0xfe } else { 0xff };
            assert_eq!(
                pixel,
                &[0, 0, blue, 0xff],
                "opaque software overlay retains BltAlpha's /256 quirk over the red base",
            );
        }

        let mut ranked = item.clone();
        ranked.image = clonk_engine::ObjectMenuImage::ObjectRank {
            object: ObjectId::new(9),
        };
        ranked
            .picture_snapshot
            .as_mut()
            .expect("picture snapshot")
            .graphics_overlays
            .clear();
        let ranked_picture = object_menu_item_picture(
            &engine,
            &make_snapshot(Vec::new(), Vec::new()),
            &ranked,
            0,
            &HudGraphics::default(),
            0,
        )
        .expect("ObjectRank picture");
        assert_eq!((ranked_picture.width(), ranked_picture.height()), (4, 4));
        assert_eq!(&ranked_picture.pixels()[0..4], &[0, 0, 0, 0]);
        assert_eq!(&ranked_picture.pixels()[4 * 4..4 * 5], &[0xff, 0, 0, 0xff]);
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
        let item = clonk_engine::ObjectMenuItem {
            caption: "Rank".to_string(),
            info_caption: String::new(),
            command: String::new(),
            command2: String::new(),
            count: 12_345_678,
            item_id: "CLNK".to_string(),
            symbol: clonk_engine::ObjectMenuSymbol::Definition,
            image: clonk_engine::ObjectMenuImage::Rank { rank: 2 },
            presentation_definition_id: Some("CLNK".to_string()),
            picture_snapshot: None,
            picture_object: None,
            components: Vec::new(),
            selectable: false,
            value: None,
            text_display_progress: -1,
        };

        let picture = object_menu_item_picture(&engine, &snapshot, &item, 0, &hud, 1)
            .expect("extended rank picture");
        assert_eq!((picture.width(), picture.height()), (3, 3));
        assert_eq!(
            &picture.pixels()[0..4],
            &[0, 0, 0xfe, 0xff],
            "captain overlay uses native software BltAlpha /256 composition",
        );
        let bottom_right = ((2 * 3 + 2) * 4) as usize;
        assert_eq!(
            &picture.pixels()[bottom_right..bottom_right + 4],
            &[0xff, 0, 0, 0xff]
        );
    }

    #[test]
    fn menu_state_navigates_folders() {
        let scenarios = sample_scenarios();
        let entries = build_menu_entries(&scenarios, false);
        let menu = StartupMenu::new(entries, test_font(), None).expect("startup menu");
        let mut state = MenuState::new(menu, scenarios);

        assert_eq!(state.current_entries().len(), 1);
        let root_entries = build_menu_entries(state.current_entries(), true);
        assert_eq!(root_entries.len(), 2);
        assert_eq!(root_entries[0].identifier, BACK_ENTRY_IDENTIFIER);
        assert_eq!(root_entries[1].identifier, "folder_missions");
        assert_eq!(state.label_path(), "Scenarios".to_string());
        state.refresh_menu_entries();
        let root_selection = state.select_default_entry();
        assert!(
            matches!(
                root_selection.as_slice(),
                [StartupMenuAction::SelectionChanged(summary)]
                if summary.identifier == "folder_missions"
            ),
            "expected default selection to target folder_missions"
        );

        state.enter_folder("folder_missions");
        assert_eq!(state.current_entries().len(), 1);
        assert_eq!(state.stack.len(), 2);
        let folder_entries = build_menu_entries(state.current_entries(), true);
        assert_eq!(folder_entries.len(), 2);
        assert_eq!(folder_entries[0].identifier, BACK_ENTRY_IDENTIFIER);
        assert_eq!(folder_entries[1].identifier, "scenario_alpha");
        assert_eq!(state.label_path(), "Scenarios / Missions".to_string());
        let folder_selection = state.select_default_entry();
        assert!(
            matches!(
                folder_selection.as_slice(),
                [StartupMenuAction::SelectionChanged(summary)]
                if summary.identifier == "scenario_alpha"
            ),
            "expected default selection to target scenario_alpha"
        );

        state.leave_folder();
        assert_eq!(state.current_entries().len(), 1);
        assert_eq!(state.stack.len(), 1);
        let root_again = build_menu_entries(state.current_entries(), true);
        assert_eq!(root_again.len(), 2);
        assert_eq!(root_again[0].identifier, BACK_ENTRY_IDENTIFIER);
        assert_eq!(root_again[1].identifier, "folder_missions");
        assert_eq!(state.label_path(), "Scenarios".to_string());
        let root_again_selection = state.select_default_entry();
        assert!(
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
        let repository = Path::new(env!("CARGO_MANIFEST_DIR"))
            .parent()
            .and_then(Path::parent)
            .expect("repository root");
        let user_data = tempdir().expect("isolated game-option config");
        let _guard = EnvGuard::set(&[
            ("LC_INSTALL_ROOT", Some(repository)),
            ("LC_USER_DATA_DIR", Some(user_data.path())),
        ]);
        let paths = AppPaths::discover().expect("discover app paths");
        if let Some(parent) = paths.config_file().parent() {
            fs::create_dir_all(parent).expect("create native config directory");
        }
        fs::write(paths.config_file(), b"[General]\r\nFairCrew=true\r\n")
            .expect("seed non-native fair-crew spelling");
        assert!(!load_fair_crew_flag(Some(&paths)));
        assert!(!load_scenario_game_option_values(Some(&paths)).fair_crew);
        fs::remove_file(paths.config_file()).expect("remove non-native fair-crew config");

        for (section, key, value) in [
            ("General", "DefCrewStrength", "777"),
            ("General", "Record", "0"),
            ("Network", "MasterServerSignUp", "0"),
            ("Network", "LeagueServerSignUp", "1"),
            ("Network", "Comment", "old comment"),
            ("Network", "LastPassword", "old password"),
        ] {
            persist_config_value(&paths, section, key, value).expect("seed game option");
        }
        persist_native_config_values(
            &paths,
            "General",
            &[("NoCrew", clonk_app_netplay::NativeConfigValue::RawAscii("true"))],
        )
        .expect("seed C++ NoCrew Boolean");
        assert!(
            load_fair_crew_flag(Some(&paths)),
            "the scen-sel flag reads C4ConfigGeneral's native NoCrew key"
        );
        let values = load_scenario_game_option_values(Some(&paths));
        assert!(values.fair_crew);
        assert_eq!(values.fair_crew_strength, 777);
        assert!(!values.record);
        assert!(!values.master_server_signup);
        assert!(values.league_server_signup);
        assert_eq!(values.comment, "old comment");
        assert_eq!(values.last_password, "old password");

        let scenario_path = user_data.path().join("Forced.c4s");
        fs::create_dir_all(&scenario_path).expect("forced scenario group");
        fs::write(
            scenario_path.join("Scenario.txt"),
            "[Head]\nTitle=Forced\nForcedNoCrew=2\n",
        )
        .expect("forced scenario core");
        let mut forced = FrontendScenario::fallback();
        forced.path = Some(scenario_path);
        assert_eq!(
            scenario_fair_crew_constraint(Some(&forced)),
            FairCrewConstraint::ForceNormal
        );
        let mut controller =
            GameOptionButtons::new(GameOptionContext::LocalSelector, values.clone());
        controller.set_selector_fair_crew_constraint(FairCrewConstraint::ForceNormal);
        let fair = controller
            .view(clonk_frontend::game_option_buttons::GameOptionButton::FairCrew)
            .expect("fair-crew button");
        assert!(!fair.enabled);
        assert_eq!(
            fair.icon,
            clonk_frontend::game_option_buttons::GameOptionIcon::NormalCrewGray
        );

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
                player_name: "Option Tester".to_string(),
                network: None,
                record_enabled: false,
            },
        )
        .expect("game-option app");
        wait_for_menu(&mut app);
        app.open_scenario_browser();
        let fonts = app
            .assets
            .clonk_fonts
            .as_deref()
            .expect("classic GUI fonts");
        let bounds = startup_scensel_game_option_bounds(800, 600, fonts);
        let option_layout = clonk_frontend::game_option_buttons::game_option_buttons_layout(
            bounds,
            GameOptionContext::LocalSelector,
        );
        let scensel_layout = clonk_frontend::startup_scensel::scen_sel_layout(800, 600, fonts);
        assert_eq!(
            option_layout.rect(clonk_frontend::game_option_buttons::GameOptionButton::FairCrew),
            Some(scensel_layout.fair_crew_button)
        );
        assert_eq!(
            option_layout.rect(clonk_frontend::game_option_buttons::GameOptionButton::Record),
            Some(scensel_layout.record_button)
        );

        app.process_game_option_actions(vec![GameOptionAction::FairCrewPreferenceChanged(false)])
            .expect("persist disabled native fair-crew preference");
        let native_config = fs::read(paths.config_file()).expect("read disabled native preference");
        assert!(native_config
            .split(|byte| matches!(*byte, b'\r' | b'\n'))
            .any(|line| line == b"NoCrew=false"));
        assert!(!native_config
            .windows(b"FairCrew=".len())
            .any(|window| window == b"FairCrew="));
        assert!(!load_fair_crew_flag(Some(&paths)));
        assert!(!load_scenario_game_option_values(Some(&paths)).fair_crew);

        app.process_game_option_actions(vec![
            GameOptionAction::RecordPreferenceChanged(true),
            GameOptionAction::InternetSignupChanged {
                enabled: true,
                live_lobby: false,
            },
            GameOptionAction::LeagueSignupChanged(false),
            GameOptionAction::CommentChanged("new comment".to_string()),
        ])
        .expect("persist selector options");
        let config = Config::load(paths.config_file()).expect("reload persisted options");
        assert_eq!(config.get_in(Some("General"), "NoCrew"), Some("false"));
        assert_eq!(config.get_in(Some("General"), "FairCrew"), None);
        assert_eq!(config.get_in(Some("General"), "Record"), Some("1"));
        assert_eq!(
            config.get_in(Some("General"), "DefCrewStrength"),
            Some("777")
        );
        assert_eq!(
            config.get_in(Some("Network"), "MasterServerSignUp"),
            Some("1")
        );
        assert_eq!(
            config.get_in(Some("Network"), "LeagueServerSignUp"),
            Some("0")
        );
        assert_eq!(
            config.get_in(Some("Network"), "Comment"),
            Some("new comment")
        );

        app.process_game_option_actions(vec![GameOptionAction::FairCrewPreferenceChanged(true)])
            .expect("persist native fair-crew preference");
        let config = Config::load(paths.config_file()).expect("reload native fair-crew key");
        assert_eq!(config.get_in(Some("General"), "NoCrew"), Some("true"));
        assert_eq!(config.get_in(Some("General"), "FairCrew"), None);
        let native_config = fs::read(paths.config_file()).expect("read enabled native preference");
        assert!(native_config
            .split(|byte| matches!(*byte, b'\r' | b'\n'))
            .any(|line| line == b"NoCrew=true"));
        assert!(!native_config
            .windows(b"FairCrew=".len())
            .any(|window| window == b"FairCrew="));
        assert!(load_fair_crew_flag(Some(&paths)));
        assert!(load_scenario_game_option_values(Some(&paths)).fair_crew);

        app.scenario_game_options =
            GameOptionButtons::new(GameOptionContext::NetworkHostSelector, values);
        let actions = app.scenario_game_options.handle_hotkey('P');
        app.finish_game_option_input(actions)
            .expect("open classic password input");
        let dialog = app
            .game_option_input_dialog
            .as_ref()
            .expect("password InputDialog");
        assert_eq!(
            dialog.purpose,
            PendingInputDialogPurpose::GameOption(GameOptionInputKind::Password)
        );
        assert_eq!(dialog.controller.caption(), "Password");
        assert_eq!(dialog.controller.text(), "old password");
        app.process_game_option_input_dialog_actions(vec![InputDialogAction::Accepted(
            "new password".to_string(),
        )])
        .expect("accept classic password input");
        assert_eq!(app.scenario_game_options.values().password, "new password");
        assert_eq!(
            Config::load(paths.config_file())
                .expect("reload password")
                .get_in(Some("Network"), "LastPassword"),
            Some("new password")
        );
        reset_cached_app_paths();
    }

    #[test]
    fn game_option_input_dialog_is_modal_and_pointer_capture_is_per_gesture() {
        let _lock = env_lock().lock();
        let repository = Path::new(env!("CARGO_MANIFEST_DIR"))
            .parent()
            .and_then(Path::parent)
            .expect("repository root");
        let user_data = tempdir().expect("isolated input-dialog config");
        let _guard = EnvGuard::set(&[
            ("LC_INSTALL_ROOT", Some(repository)),
            ("LC_USER_DATA_DIR", Some(user_data.path())),
        ]);
        let paths = AppPaths::discover().expect("discover app paths");
        let mut app = GameApp::new(
            800,
            600,
            AudioOptions::default(),
            Some(&paths),
            RuntimeConfig {
                player_owner: 1,
                player_name: "Modal Tester".to_string(),
                network: None,
                record_enabled: false,
            },
        )
        .expect("input-dialog app");
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
        app.finish_game_option_input(actions)
            .expect("open password modal");

        app.handle_modifiers_changed(ModifiersState::CTRL | ModifiersState::ALT)
            .expect("hold combined mnemonic modifiers");
        app.handle_key(VirtualKeyCode::O, ElementState::Pressed)
            .expect("combined modifiers do not activate the exact Alt mnemonic");
        app.handle_key(VirtualKeyCode::O, ElementState::Released)
            .expect("release combined mnemonic probe");
        assert!(app.game_option_input_dialog.is_some());
        app.handle_modifiers_changed(ModifiersState::SHIFT)
            .expect("hold Shift over regular input dialog");
        app.handle_key(VirtualKeyCode::Escape, ElementState::Pressed)
            .expect("modified Escape does not cancel the regular dialog");
        app.handle_key(VirtualKeyCode::Escape, ElementState::Released)
            .expect("release modified Escape probe");
        assert!(app.game_option_input_dialog.is_some());
        app.handle_modifiers_changed(ModifiersState::empty())
            .expect("release modal modifiers");

        for key in [
            VirtualKeyCode::Up,
            VirtualKeyCode::Down,
            VirtualKeyCode::Left,
        ] {
            app.handle_key(key, ElementState::Pressed)
                .expect("modal key down");
            app.handle_key(key, ElementState::Released)
                .expect("modal key up");
        }
        assert_eq!(app.menu_state.menu.selected_index(), selected);
        assert_eq!(app.menu_state.stack.len(), stack_len);
        assert_eq!(app.menu_state.search_text(), "underlying search");
        assert_eq!(app.startup_view, StartupView::ScenarioBrowser);

        app.handle_key(VirtualKeyCode::Apps, ElementState::Pressed)
            .expect("open modal edit context");
        assert!(app.context_menu.is_some());
        assert!(
            GameApp::startup_base_context_menu(app.context_menu.as_ref(), true,).is_none(),
            "modal owns the one context-menu render pass"
        );
        app.handle_key(VirtualKeyCode::Apps, ElementState::Released)
            .expect("consume Apps release inside modal");
        app.close_context_menu_silently();

        let layout = app.game_option_input_layout().expect("modal layout");
        let edit_point = PhysicalPosition::new(
            f64::from(layout.edit.x + layout.edit.w / 2),
            f64::from(layout.edit.y + layout.edit.h / 2),
        );
        app.handle_cursor_moved(edit_point)
            .expect("point into modal edit");
        app.handle_mouse_button(ElementState::Pressed)
            .expect("hold modal left button");
        assert_eq!(
            app.game_option_input_pointer_capture,
            Some(ContextMenuPointerButton::Left)
        );
        app.process_game_option_input_dialog_actions(vec![InputDialogAction::Cancelled])
            .expect("close modal while left is held");
        assert!(app.game_option_input_dialog.is_none());
        app.handle_mouse_button(ElementState::Released)
            .expect("consume modal-owned left release");
        assert_eq!(app.game_option_input_pointer_capture, None);
        assert_eq!(app.menu_state.menu.selected_index(), selected);

        let actions = app.scenario_game_options.handle_hotkey('P');
        app.finish_game_option_input(actions)
            .expect("reopen password modal");
        app.handle_cursor_moved(edit_point)
            .expect("point into reopened modal");
        app.handle_other_mouse_button(ElementState::Pressed)
            .expect("modal middle down");
        assert_eq!(
            app.game_option_input_pointer_capture,
            Some(ContextMenuPointerButton::Other)
        );
        app.handle_other_mouse_button(ElementState::Released)
            .expect("modal middle up");
        assert_eq!(app.game_option_input_pointer_capture, None);
        reset_cached_app_paths();
    }

    #[test]
    fn resize_cancels_selector_option_and_input_dialog_interactions() {
        let _lock = env_lock().lock();
        let repository = Path::new(env!("CARGO_MANIFEST_DIR"))
            .parent()
            .and_then(Path::parent)
            .expect("repository root");
        let user_data = tempdir().expect("isolated resize config");
        let _guard = EnvGuard::set(&[
            ("LC_INSTALL_ROOT", Some(repository)),
            ("LC_USER_DATA_DIR", Some(user_data.path())),
        ]);
        let paths = AppPaths::discover().expect("discover app paths");
        let mut app = new_menu_app_with_paths(800, 600, &paths);
        app.open_scenario_browser();
        app.scenario_game_options.set_focused_button(Some(
            clonk_frontend::game_option_buttons::GameOptionButton::Record,
        ));
        app.menu_state.set_dialog_focus(ScenselDialogFocus::Options);
        app.handle_key(VirtualKeyCode::Space, ElementState::Pressed)
            .expect("hold Record keyboard activation");
        assert!(!app.game_option_consumed_keys.is_empty());
        let record = app
            .scenario_game_options
            .layout()
            .rect(clonk_frontend::game_option_buttons::GameOptionButton::Record)
            .expect("Record bounds");
        app.handle_cursor_moved(PhysicalPosition::new(
            f64::from(record.x + record.w / 2),
            f64::from(record.y + record.h / 2),
        ))
        .expect("point at held Record");
        app.handle_mouse_button(ElementState::Pressed)
            .expect("hold Record pointer activation");
        assert!(app.game_option_pointer_capture);
        app.resize(1024, 768).expect("resize held option strip");
        assert!(app.game_option_consumed_keys.is_empty());
        assert!(!app.game_option_pointer_capture);
        app.handle_key(VirtualKeyCode::Space, ElementState::Released)
            .expect("release cancelled Record keyboard activation");
        app.handle_mouse_button(ElementState::Released)
            .expect("release cancelled Record pointer activation");
        assert!(!app.scenario_game_options.values().record);

        app.scenario_game_options = GameOptionButtons::new(
            GameOptionContext::NetworkHostSelector,
            GameOptionValues::default(),
        );
        app.sync_scenario_game_option_bounds();
        let actions = app.scenario_game_options.handle_hotkey('P');
        app.finish_game_option_input(actions)
            .expect("open resize password modal");
        let input_layout = app.game_option_input_layout().expect("input layout");
        let edit_point = PhysicalPosition::new(
            f64::from(input_layout.edit.x + input_layout.edit.w / 2),
            f64::from(input_layout.edit.y + input_layout.edit.h / 2),
        );
        app.handle_cursor_moved(edit_point)
            .expect("point into password edit");
        app.handle_mouse_button(ElementState::Pressed)
            .expect("start password edit drag");
        app.handle_key(VirtualKeyCode::Tab, ElementState::Pressed)
            .expect("focus password OK button");
        app.handle_key(VirtualKeyCode::Tab, ElementState::Released)
            .expect("release password focus traversal");
        app.handle_key(VirtualKeyCode::Return, ElementState::Pressed)
            .expect("hold password OK button");
        assert_eq!(
            app.game_option_input_pointer_capture,
            Some(ContextMenuPointerButton::Left)
        );
        assert!(!app.game_option_input_consumed_keys.is_empty());
        assert!(app.game_option_input_pointer_position.is_some());
        app.resize(1280, 720).expect("resize open password modal");
        assert!(app.game_option_input_dialog.is_some());
        assert_eq!(app.game_option_input_pointer_capture, None);
        assert!(app.game_option_input_consumed_keys.is_empty());
        assert!(app.game_option_input_pointer_position.is_none());
        assert!(app.game_option_input_last_click.is_none());
        assert!(!app.game_option_pointer_capture);
        app.handle_key(VirtualKeyCode::Return, ElementState::Released)
            .expect("release cancelled modal OK button");
        app.handle_mouse_button(ElementState::Released)
            .expect("release cancelled modal drag");
        assert!(app.game_option_input_dialog.is_some());
        reset_cached_app_paths();
    }

    #[test]
    fn l098_takeover_submenu_lists_only_local_unissued_unassociated_players() {
        let mut app = new_menu_app(640, 480);
        install_test_free_savegame_player_row(&mut app, 50);
        let (network, _events, mut commands) =
            NetworkManager::test_stub_with_commands_for_client_id(7);
        app.network = Some(network);
        app.network_mode = Some(NetworkMode::Client(ClientSettings::new(
            SocketAddr::from(([127, 0, 0, 1], 11_112)),
            "Client",
        )));

        let eligible_league = clonk_engine::ControlPlayerInfoEntry {
            id: 11,
            name: LegacyCString::from_bytes(b"Raw A".to_vec()).unwrap(),
            forced_name: LegacyCString::from_bytes(b"Forced A".to_vec()).unwrap(),
            league_account: LegacyCString::from_bytes(b"League A".to_vec()).unwrap(),
            ..Default::default()
        };
        let join_issued = clonk_engine::ControlPlayerInfoEntry {
            id: 12,
            name: LegacyCString::from_bytes(b"Issued".to_vec()).unwrap(),
            flags: clonk_engine::PLAYER_INFO_FLAG_JOIN_ISSUED,
            ..Default::default()
        };
        let joined_and_removed = clonk_engine::ControlPlayerInfoEntry {
            id: 13,
            name: LegacyCString::from_bytes(b"Joined".to_vec()).unwrap(),
            flags: clonk_engine::PLAYER_INFO_FLAG_JOINED | clonk_engine::PLAYER_INFO_FLAG_REMOVED,
            ..Default::default()
        };
        let associated = clonk_engine::ControlPlayerInfoEntry {
            id: 14,
            name: LegacyCString::from_bytes(b"Associated".to_vec()).unwrap(),
            savegame_player: 90,
            ..Default::default()
        };
        let eligible_forced = clonk_engine::ControlPlayerInfoEntry {
            id: 15,
            name: LegacyCString::from_bytes(b"Raw B".to_vec()).unwrap(),
            forced_name: LegacyCString::from_bytes(b"Forced B".to_vec()).unwrap(),
            ..Default::default()
        };
        app.control_player_infos.replace_snapshot(
            99,
            [
                clonk_engine::PlayerInfoControlData {
                    client_id: 0,
                    players: vec![clonk_engine::ControlPlayerInfoEntry {
                        id: 21,
                        name: LegacyCString::from_bytes(b"Foreign".to_vec()).unwrap(),
                        ..Default::default()
                    }],
                    ..Default::default()
                },
                clonk_engine::PlayerInfoControlData {
                    client_id: 7,
                    flags: clonk_engine::CLIENT_PLAYER_INFO_FLAG_INITIAL,
                    players: vec![
                        eligible_league,
                        join_issued,
                        joined_and_removed,
                        associated,
                        eligible_forced,
                    ],
                    by_client: 7,
                },
            ],
        );

        let entries = app.classic_lobby_takeover_entries(50);
        assert_eq!(
            entries.iter().map(|entry| entry.text.as_str()).collect::<Vec<_>>(),
            vec!["Using League A", "Using Forced B"]
        );
        assert_eq!(
            entries
                .iter()
                .map(|entry| entry.tooltip.as_deref())
                .collect::<Vec<_>>(),
            vec![
                Some("Use this player to continue the savegame"),
                Some("Use this player to continue the savegame"),
            ]
        );
        assert!(entries.iter().all(|entry| entry.icon == ContextMenuIcon::Phase(9)));
        assert_eq!(
            entries.iter().map(|entry| entry.action.clone()).collect::<Vec<_>>(),
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

        app.process_classic_lobby_actions(vec![ClassicLobbyAction::RosterContextRequested {
            row: LobbyRosterId::Player(50),
            position: GuiPoint::new(200.0, 150.0),
        }])
        .expect("free savegame player context opens");
        let root = app.context_menu.as_ref().unwrap().layout().panels[0].rows[0].rect;
        app.handle_context_menu_pointer_move(GuiPoint::new(
            (root.x + 1) as f32,
            (root.y + 1) as f32,
        ))
        .expect("open takeover submenu");
        let layout = app.context_menu.as_ref().unwrap().layout();
        assert_eq!(layout.panels.len(), 2);
        assert_eq!(layout.panels[1].rows.len(), 2);

        let mut rows = app
            .classic_host_lobby
            .as_ref()
            .unwrap()
            .controller
            .rows()
            .to_vec();
        let LobbyRosterRow::Header(header) = &mut rows[0] else {
            panic!("free savegame group header");
        };
        header.kind = LobbyRosterHeader::ReplayPlayers;
        app.classic_host_lobby
            .as_mut()
            .unwrap()
            .controller
            .set_rows(rows);
        app.close_stale_classic_lobby_team_combo();
        assert!(
            app.context_menu.is_none(),
            "regrouping the target as a replay player closes a stale takeover menu"
        );
        app.take_over_classic_lobby_savegame_player(50, 11);
        assert!(
            commands.take_player_info_updates().is_empty(),
            "the activation guard rejects a replay target even when invoked directly"
        );
    }

    #[test]
    fn takeover_submenu_fills_live_at_open() {
        let mut app = new_menu_app(640, 480);
        install_test_free_savegame_player_row(&mut app, 50);
        let (network, _events, mut commands) =
            NetworkManager::test_stub_with_commands_for_client_id(7);
        app.network = Some(network);
        app.network_mode = Some(NetworkMode::Client(ClientSettings::new(
            SocketAddr::from(([127, 0, 0, 1], 11_112)),
            "Client",
        )));

        let first = clonk_engine::ControlPlayerInfoEntry {
            id: 11,
            name: LegacyCString::from_bytes(b"First".to_vec()).unwrap(),
            ..Default::default()
        };
        let second = clonk_engine::ControlPlayerInfoEntry {
            id: 12,
            name: LegacyCString::from_bytes(b"Second".to_vec()).unwrap(),
            ..Default::default()
        };
        let packet_flags = clonk_engine::CLIENT_PLAYER_INFO_FLAG_INITIAL;
        let local_packet = |players: Vec<clonk_engine::ControlPlayerInfoEntry>| {
            clonk_engine::PlayerInfoControlData {
                client_id: 7,
                flags: packet_flags,
                players,
                by_client: 7,
            }
        };
        app.control_player_infos
            .replace_snapshot(99, [local_packet(vec![first.clone()])]);

        app.process_classic_lobby_actions(vec![ClassicLobbyAction::RosterContextRequested {
            row: LobbyRosterId::Player(50),
            position: GuiPoint::new(200.0, 150.0),
        }])
        .expect("free savegame player context opens");
        assert_eq!(
            app.context_menu.as_ref().unwrap().layout().panels.len(),
            1,
            "the Take Over child panel does not exist at root-menu open"
        );

        // A player-info update arrives while the root menu is open. C++
        // fills the children in OnContextTakeOver only at submenu-open
        // (src/C4PlayerInfoListBox.cpp:503-505,535-556), so the submenu must
        // reflect this update rather than a root-open snapshot.
        app.control_player_infos
            .replace_snapshot(100, [local_packet(vec![first.clone(), second.clone()])]);

        let root = app.context_menu.as_ref().unwrap().layout().panels[0].rows[0].rect;
        app.handle_context_menu_pointer_move(GuiPoint::new(
            (root.x + 1) as f32,
            (root.y + 1) as f32,
        ))
        .expect("open takeover submenu");
        let layout = app.context_menu.as_ref().unwrap().layout();
        assert_eq!(layout.panels.len(), 2);
        assert_eq!(
            layout.panels[1].rows.len(),
            2,
            "children are computed from the live packet at submenu-open"
        );

        // Closing the child and re-selecting the parent re-runs the fill
        // callback, so a candidate that issued its join meanwhile drops out.
        app.handle_context_menu_key(VirtualKeyCode::Left, ElementState::Pressed)
            .expect("close the takeover child panel");
        assert_eq!(app.context_menu.as_ref().unwrap().layout().panels.len(), 1);
        let mut issued_first = first.clone();
        issued_first.flags |= clonk_engine::PLAYER_INFO_FLAG_JOIN_ISSUED;
        app.control_player_infos
            .replace_snapshot(101, [local_packet(vec![issued_first, second.clone()])]);
        app.handle_context_menu_key(VirtualKeyCode::Right, ElementState::Pressed)
            .expect("reopen the takeover child panel");
        let layout = app.context_menu.as_ref().unwrap().layout();
        assert_eq!(layout.panels.len(), 2);
        assert_eq!(
            layout.panels[1].rows.len(),
            1,
            "a re-open refills from the live packet like C++"
        );

        // The surviving child is the live-eligible player and activates the
        // exact live association.
        let child = app.context_menu.as_ref().unwrap().layout().panels[1].rows[0].rect;
        app.handle_context_menu_pointer_move(GuiPoint::new(
            (child.x + 1) as f32,
            (child.y + 1) as f32,
        ))
        .expect("select live takeover child");
        assert!(
            app.handle_context_menu_pointer_button(
                ElementState::Pressed,
                ContextMenuPointerButton::Left,
            )
            .expect("activate live takeover child")
        );
        let updates = commands.take_player_info_updates();
        assert_eq!(updates.len(), 1);
        assert_eq!(
            updates[0]
                .players
                .iter()
                .map(|player| (player.id, player.savegame_player))
                .collect::<Vec<_>>(),
            vec![(11, 0), (12, 50)],
            "the activation grabs the live-eligible player only"
        );
        assert!(
            app.handle_context_menu_pointer_button(
                ElementState::Released,
                ContextMenuPointerButton::Left,
            )
            .expect("consume takeover activation release")
        );
    }

    #[test]
    fn l081_player_context_root_matches_cpp_entry_gates() {
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
            name: LegacyCString::from_bytes(b"Replay".to_vec()).unwrap(),
            color: 0x0012_3456,
            original_color: 0x0065_4321,
            ..Default::default()
        };
        let free_script = clonk_engine::ControlPlayerInfoEntry {
            id: 52,
            name: LegacyCString::from_bytes(b"Free script".to_vec()).unwrap(),
            player_type: clonk_engine::PLAYER_INFO_TYPE_SCRIPT,
            ..Default::default()
        };
        app.control_player_infos.replace_snapshot(
            51,
            [
                clonk_engine::PlayerInfoControlData {
                    client_id: 0,
                    flags: clonk_engine::CLIENT_PLAYER_INFO_FLAG_INITIAL,
                    players: vec![chooser.clone()],
                    by_client: 0,
                },
                clonk_engine::PlayerInfoControlData {
                    client_id: 7,
                    flags: clonk_engine::CLIENT_PLAYER_INFO_FLAG_INITIAL,
                    players: vec![associated_script.clone()],
                    by_client: 0,
                },
                clonk_engine::PlayerInfoControlData {
                    client_id: 8,
                    flags: clonk_engine::CLIENT_PLAYER_INFO_FLAG_INITIAL,
                    players: vec![replay_player],
                    by_client: 0,
                },
            ],
        );
        let mut host_snapshot = clonk_network::HostConfig::default()
            .initial_join_snapshot
            .expect("default host JoinData");
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
            .unwrap()
            .controller
            .rows()[0]
            .clone();
        app.classic_host_lobby
            .as_mut()
            .unwrap()
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

        let (_, free) = app
            .classic_lobby_player_context_entries(50)
            .expect("visible free restore row");
        assert_eq!(free.len(), 1);
        assert_eq!(free[0].text, "<c ffffff7f>T</c>ake over");
        assert_eq!(free[0].tooltip.as_deref(), Some("Control the player in the game"));
        assert_eq!(free[0].icon, ContextMenuIcon::Phase(9));
        assert_eq!(free[0].hotkey, Some('T'));
        assert_eq!(free[0].action, None);
        assert!(free[0].has_submenu());
        assert!(
            app.classic_lobby_player_context_entries(52)
                .expect("visible free script row")
                .1
                .is_empty(),
            "native free script rows omit Take Over"
        );
        let (_, replay) = app
            .classic_lobby_player_context_entries(51)
            .expect("visible replay row");
        assert_eq!(
            replay
                .iter()
                .map(|entry| entry.action.clone())
                .collect::<Vec<_>>(),
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
        app.process_classic_lobby_actions(vec![ClassicLobbyAction::RosterContextRequested {
            row: LobbyRosterId::Player(51),
            position: GuiPoint::new(200.0, 150.0),
        }])
        .expect("replay context opens");
        app.close_stale_classic_lobby_team_combo();
        assert!(
            app.context_menu.is_some(),
            "an unchanged replay group keeps its ordinary context menu"
        );
        assert_eq!(app.context_menu_lobby_player, Some((-1, 51, false)));
        app.close_context_menu_silently();
        app.process_classic_lobby_actions(vec![ClassicLobbyAction::RosterContextRequested {
            row: LobbyRosterId::Player(50),
            position: GuiPoint::new(200.0, 150.0),
        }])
        .expect("free restore context opens");
        assert_eq!(
            app.context_menu.as_ref().unwrap().layout().panels[0]
                .rows
                .len(),
            1
        );
        app.close_context_menu_silently();

        let (_, ordinary) = app
            .classic_lobby_player_context_entries(7)
            .expect("visible ordinary row");
        assert_eq!(ordinary.len(), 2);
        assert_eq!(ordinary[0].text, "<c ffffff7f>R</c>emove");
        assert_eq!(ordinary[0].tooltip.as_deref(), Some("Do not join with this player"));
        assert_eq!(ordinary[0].icon, ContextMenuIcon::Phase(34));
        assert_eq!(ordinary[0].hotkey, Some('R'));
        assert_eq!(
            ordinary[0].action,
            Some(AppContextMenuCommand::LobbyPlayerRemove {
                client_id: 0,
                player_id: 7,
            })
        );
        assert_eq!(ordinary[1].text, "New <c ffffff7f>c</c>olor");
        assert_eq!(
            ordinary[1].tooltip.as_deref(),
            Some("Generate a new random player color")
        );
        assert_eq!(ordinary[1].icon, ContextMenuIcon::Phase(9));
        assert_eq!(ordinary[1].hotkey, Some('C'));
        assert_eq!(
            ordinary[1].action,
            Some(AppContextMenuCommand::LobbyPlayerNewColor {
                client_id: 0,
                player_id: 7,
            })
        );

        app.network_team_assignment
            .as_mut()
            .unwrap()
            .teams_mut()
            .team_colors = true;
        let (_, ordinary) = app.classic_lobby_player_context_entries(7).unwrap();
        assert_eq!(ordinary.len(), 1, "a nonzero team color suppresses reroll");
        let (_, script) = app.classic_lobby_player_context_entries(9).unwrap();
        assert_eq!(script.len(), 1, "association suppresses only Remove");
        assert_eq!(
            script[0].action,
            Some(AppContextMenuCommand::LobbyPlayerNewColor {
                client_id: 7,
                player_id: 9,
            }),
            "team zero retains New Color even with team colors enabled"
        );

        app.network_mode = None;
        app.control_clients
            .replace_snapshot([message_client(0, b"Remote owner")]);
        let (_, foreign) = app.classic_lobby_player_context_entries(7).unwrap();
        assert!(foreign.is_empty());
        app.process_classic_lobby_actions(vec![ClassicLobbyAction::RosterContextRequested {
            row: LobbyRosterId::Player(7),
            position: GuiPoint::new(200.0, 150.0),
        }])
        .expect("C++ opens an empty root for an unowned player");
        assert!(app.context_menu.is_some());
        assert_eq!(app.context_menu_lobby_player, Some((0, 7, false)));
    }

    #[test]
    fn l037_context_menu_matches_edit_predicates_and_order() {
        let view = LobbyChatEditView {
            text: "selected text".into(),
            caret: 8,
            selection: Some((0, 8)),
            ..LobbyChatEditView::default()
        };
        let entries = lobby_chat_context_entries(&view, true);
        assert_eq!(
            entries
                .iter()
                .map(|entry| entry.text.as_str())
                .collect::<Vec<_>>(),
            ["Cut", "Copy", "Paste", "Clear", "Select all"]
        );
        assert_eq!(
            entries
                .iter()
                .map(|entry| entry.action.clone())
                .collect::<Vec<_>>(),
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
        assert_eq!(
            entries
                .iter()
                .map(|entry| entry.text.as_str())
                .collect::<Vec<_>>(),
            ["Cut", "Copy", "Clear"]
        );

        assert!(lobby_chat_context_entries(&LobbyChatEditView::default(), false).is_empty());
    }

    #[test]
    fn l037_classic_context_menu_dispatches_to_the_live_edit() {
        let mut app = new_menu_app(640, 480);
        install_test_classic_host_lobby(&mut app);
        app.classic_host_lobby
            .as_mut()
            .expect("classic lobby")
            .controller
            .set_chat_edit_view(LobbyChatEditView {
                text: "select me".into(),
                caret: 9,
                ..LobbyChatEditView::default()
            });

        app.process_classic_lobby_chat_request(LobbyChatRequest::OpenContextMenu {
            anchor: GuiPoint::new(20.0, 20.0),
        })
        .expect("open chat context menu");
        assert!(app.context_menu.is_some());
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
        .expect("dispatch chat context command");
        let view = app
            .classic_host_lobby
            .as_ref()
            .expect("classic lobby remains")
            .controller
            .chat_edit_view();
        assert_eq!(view.selection, Some((0, view.text.len())));
    }

    #[test]
    fn return_to_menu_recreates_music_before_teardown_fade_finishes_like_cpp() {
        clonk_logging::init();
        assert_eq!(GAME_MUSIC_FADE_OUT_MS, 2_000);

        // Music discovery reads process env; hold the env lock so the
        // EnvGuard-based tests cannot redirect paths mid-load.
        let _lock = env_lock().lock();
        let user_data = tempdir().expect("isolated user data");
        let (_guard, paths) = exact_loader_test_paths(user_data.path(), None);
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
        .expect("initialise app with audio");

        let fixture = app
            .audio
            .as_ref()
            .expect("test audio")
            .system
            .load_music(&silent_pcm_wav(20))
            .expect("predecode controlled music fixture");
        app.audio
            .as_mut()
            .expect("test audio")
            .control_music_loads_with(fixture);

        // Menu music is started by `ensure_menu_music()` when asynchronous boot
        // loading completes and the menu is shown; pump boot to that point first.
        wait_for_menu(&mut app);
        let audio = app.audio.as_ref().expect("test audio");
        let controlled = audio
            .controlled_music_loads
            .as_ref()
            .expect("controlled music loading");
        assert_eq!(controlled.requests.len(), 1);
        let frontend = controlled.requests.front().expect("frontend music request");
        assert!(!frontend.looped, "frontend music is non-looping");
        assert!(frontend.identity.is_some(), "frontend music came from the catalog");
        assert_eq!(audio.music_resolver.playlist.as_deref(), Some("Frontend.*"));
        assert!(!audio.system.music_is_playing());
        assert!(
            app.audio
                .as_mut()
                .expect("test audio")
                .complete_next_controlled_music_load()
                .expect("complete frontend music load")
        );
        assert!(app
            .audio
            .as_ref()
            .expect("test audio")
            .system
            .music_is_playing());

        app.start_sandbox_scenario(FrontendScenario::fallback())
            .expect("start sandbox scenario");
        let audio = app.audio.as_ref().expect("test audio");
        let controlled = audio
            .controlled_music_loads
            .as_ref()
            .expect("controlled music loading");
        assert_eq!(controlled.requests.len(), 1);
        let sandbox = controlled.requests.front().expect("sandbox music request");
        assert!(sandbox.looped, "sandbox music is looping");
        assert!(sandbox.identity.is_none(), "sandbox uses the direct music asset");
        assert_eq!(audio.music_resolver.playlist, None);
        assert!(!audio.system.music_is_playing());
        assert!(
            app.audio
                .as_mut()
                .expect("test audio")
                .complete_next_controlled_music_load()
                .expect("complete sandbox music load")
        );
        app.return_to_menu();
        let audio = app.audio.as_ref().expect("test audio");
        assert!(
            !audio.system.music_is_playing(),
            "PreInit reconstruction hard-stops the fading game song"
        );
        assert!(!app.resume_frontend_music_after_fade);
        assert_eq!(
            audio.music_fade_requests,
            [GAME_MUSIC_FADE_OUT_MS],
            "Game.Clear still requests its 2s fade before PreInit cancels it"
        );
        let controlled = audio
            .controlled_music_loads
            .as_ref()
            .expect("controlled music loading");
        assert_eq!(controlled.requests.len(), 1);
        let frontend = controlled
            .requests
            .front()
            .expect("PreInit frontend music request");
        assert!(!frontend.looped, "returned frontend music is non-looping");
        assert!(frontend.identity.is_some(), "returned music came from the catalog");
        assert_eq!(audio.music_resolver.playlist.as_deref(), Some("Frontend.*"));
        assert!(
            app.audio
                .as_mut()
                .expect("test audio")
                .complete_next_controlled_music_load()
                .expect("complete returned frontend music load")
        );
        let audio = app.audio.as_ref().expect("test audio");
        assert!(audio.system.music_is_playing());
        assert_eq!(audio.music_load_pending.load(AtomicOrdering::Acquire), 0);
        assert!(audio
            .controlled_music_loads
            .as_ref()
            .expect("controlled music loading")
            .requests
            .is_empty());

        // Restart/Next Mission also reconstructs at PreInit, but skips
        // C4Startup::DoStartup and therefore must not enqueue Frontend.*.
        app.start_sandbox_scenario(FrontendScenario::fallback())
            .expect("start relaunch source scenario");
        assert!(
            app.audio
                .as_mut()
                .expect("test audio")
                .complete_next_controlled_music_load()
                .expect("complete relaunch source music")
        );
        app.audio
            .as_mut()
            .expect("test audio")
            .set_scenario_music_level(Some(25));
        app.return_to_menu_for_relaunch();
        let audio = app.audio.as_ref().expect("test audio");
        assert!(!audio.system.music_is_playing());
        assert!(!app.resume_frontend_music_after_fade);
        assert_eq!(
            audio.music_fade_requests,
            [GAME_MUSIC_FADE_OUT_MS, GAME_MUSIC_FADE_OUT_MS],
            "each Game.Clear requests its fade before the next PreInit"
        );
        assert!(
            lock_unpoisoned(&audio.music_control)
                .most_recently_played
                .is_none(),
            "the direct-relaunch PreInit generation has no prior song identity"
        );
        assert_eq!(
            lock_unpoisoned(&audio.music_control).scenario_level,
            None,
            "Game.Clear and the reconstructed music system discard scenario volume"
        );
        assert!(audio
            .controlled_music_loads
            .as_ref()
            .expect("controlled music loading")
            .requests
            .is_empty());
    }

    #[test]
    fn l018_menu_cursor_moves_with_cached_startup_frame_and_clears_on_leave() {
        let mut app = new_menu_app(64, 48);
        install_l018_cursor_atlas(&mut app);
        let background = Color::opaque(9, 10, 11);

        let version = app.menu_render_version;
        app.handle_cursor_moved(PhysicalPosition::new(20.0, 18.0))
            .expect("first startup cursor move");
        assert!(app.menu_render_version > version);
        app.graphics.surface_mut().fill(background);
        assert!(app.draw_classic_gui_cursor(None));
        assert_eq!(
            app.graphics.surface().get_pixel(18, 16),
            Some(Color::opaque(0, 40, 200))
        );

        let version = app.menu_render_version;
        app.handle_cursor_moved(PhysicalPosition::new(40.0, 30.0))
            .expect("same-control startup cursor move");
        assert!(app.menu_render_version > version);
        app.graphics.surface_mut().fill(background);
        assert!(app.draw_classic_gui_cursor(None));
        assert_eq!(app.graphics.surface().get_pixel(18, 16), Some(background));
        assert_eq!(
            app.graphics.surface().get_pixel(38, 28),
            Some(Color::opaque(0, 40, 200))
        );

        let version = app.menu_render_version;
        app.pointer_left().expect("leave startup window");
        assert!(app.menu_render_version > version);
        assert!(!app.draw_classic_gui_cursor(None));
    }

    #[test]
    fn l018_loading_dialog_renders_gui_cursor_between_body_and_tooltip_passes() {
        let mut app = new_menu_app(320, 200);
        install_l018_cursor_atlas(&mut app);
        let fonts = app
            .assets
            .clonk_fonts
            .clone()
            .expect("synthetic classic loader fonts");
        app.loader_screen = Some(
            LoaderScreen::new(
                LoaderSelection::startup("LoaderSynthetic.png")
                    .expect("valid synthetic loader selection"),
                ImageData::new(1, 1, vec![7, 8, 9, 255]),
                LoaderResources::new(fonts, ImageData::new(3, 1, vec![255; 12]))
                    .expect("valid synthetic loader resources"),
                LoaderState::initial("Loading"),
            )
            .expect("valid synthetic loader screen"),
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
        .expect("install loading message dialog");
        app.handle_cursor_moved(PhysicalPosition::new(20.0, 18.0))
            .expect("route loading GUI pointer");

        let mut frame = vec![0_u8; 320 * 200 * 4];
        app.render(&mut frame)
            .expect("render loader, dialog, GUI cursor, and tooltip passes");
        let cursor_pixel = ((16 * 320 + 18) * 4) as usize;
        assert_eq!(
            &frame[cursor_pixel..cursor_pixel + 4],
            &[1, 40, 200, 255],
            "standard C4 gamma raises the Region cell's zero channel to one"
        );
    }

    #[test]
    fn l018_running_gui_ownership_matches_cpp_reset_and_dialog_lifetime() {
        let mut app = new_synthetic_running_sandbox_app();
        install_l018_cursor_atlas(&mut app);
        let (width, height) = {
            let surface = app.graphics.surface();
            (surface.width(), surface.height())
        };
        let mut frame = vec![0_u8; width as usize * height as usize * 4];
        app.render(&mut frame).expect("establish running viewport");
        app.open_ingame_menu().expect("show external C4Menu");
        let menu_point = (0..height)
            .flat_map(|y| (0..width).map(move |x| GuiPoint::new(x as f32, y as f32)))
            .find(|point| app.ingame_menu_pointer_target(*point).is_some())
            .expect("visible menu owns at least one output point");
        app.handle_cursor_moved(PhysicalPosition::new(
            f64::from(menu_point.x),
            f64::from(menu_point.y),
        ))
        .expect("route pointer into external C4Menu");
        assert!(app.running_gui_mouse_owned);
        assert!(!app.running_world_mouse_owned);
        assert!(app.ingame_pointer.is_none());

        app.reset_ingame_mouse_control();
        assert!(
            app.running_gui_mouse_owned,
            "C4MouseControl reset must not deactivate C4GUI::CMouse"
        );
        assert!(
            app.running_world_mouse_owned,
            "C4MouseControl::Default independently restores fMouseOwned"
        );
        app.initialize_ingame_mouse_center()
            .expect("execute reset C4MouseControl while menu remains shown");
        let reset_world_pointer = app
            .ingame_pointer
            .expect("reset world mouse remains independently drawable");
        assert!(
            app.classic_gui_cursor_request().is_some(),
            "GUI cursor remains independently drawable after the reset"
        );
        app.runtime_help_visible = true;
        app.close_ingame_menu_for_player(app.local_owner);
        assert!(
            app.running_gui_mouse_owned,
            "Dialog::Close leaves ownership for C4GraphicsSystem::Execute"
        );
        app.reconcile_running_mouse_after_last_gui_close(false)
            .expect("execute last-dialog stationary handoff with F1 help shown");
        assert!(!app.running_gui_mouse_owned);
        assert!(app.running_world_mouse_owned);
        assert!(
            app.ingame_pointer.is_some(),
            "the independently reinitialized world pointer remains active"
        );
        assert_eq!(app.ingame_pointer, Some(reset_world_pointer));

        app.runtime_help_visible = false;
        app.open_ingame_menu().expect("show external C4Menu again");
        app.handle_cursor_moved(PhysicalPosition::new(
            f64::from(menu_point.x),
            f64::from(menu_point.y),
        ))
        .expect("route pointer into second menu");
        assert!(!app.running_world_mouse_owned);
        set_test_scenario_head_flags(&mut app, 1, 1);
        app.render(&mut frame)
            .expect("render film/replay with the shown menu pixels suppressed");
        assert!(
            app.running_gui_mouse_owned,
            "a shown C4Menu remains a C4GUI owner when viewport pixels are suppressed"
        );
        assert!(!app.running_world_mouse_owned);

        let non_cursor_menu_object = app
            .engine
            .spawn_object(SpawnConfig::new("CLNK").with_position(Vector2::new(40, 30)))
            .expect("spawn arbitrary non-cursor menu object");
        install_test_cursor_menu(
            &mut app,
            non_cursor_menu_object,
            two_item_script_menu(non_cursor_menu_object),
        );
        assert_ne!(
            app.engine.crew_cursor(app.local_owner),
            Some(non_cursor_menu_object)
        );
        app.close_ingame_menu_for_player(app.local_owner);
        app.render(&mut frame)
            .expect("retain GUI ownership for a shown non-cursor object menu");
        assert!(app.running_gui_mouse_owned);
        assert!(!app.running_world_mouse_owned);

        app.engine
            .apply_object_update(
                non_cursor_menu_object,
                ObjectUpdate {
                    menu: Some(None),
                    ..ObjectUpdate::default()
                },
            )
            .expect("close arbitrary non-cursor object menu");
        app.render(&mut frame)
            .expect("handoff after the final suppressed object menu closes");
        assert!(!app.running_gui_mouse_owned);
        assert!(app.running_world_mouse_owned);
        assert!(app.ingame_pointer.is_some());
    }

    #[test]
    fn synthetic_classic_test_assets_satisfy_only_the_global_gui_guard() {
        let mut app = new_menu_app(320, 200);
        assert!(
            app.assets
                .require_classic_global_gui_bootstrap_resources(&HashMap::new())
                .is_ok()
        );
        assert!(app.assets.require_classic_startup_bootstrap_resources().is_err());
        assert!(app.assets.require_classic_startup_main_resources().is_err());
        assert!(app.assets.require_classic_ingame_menu_resources().is_err());
        assert!(app.assets.require_classic_game_over_resources().is_err());
        assert!(
            Arc::get_mut(&mut app.assets).is_some(),
            "each app owns a mutable outer asset bundle"
        );
    }

    #[test]
    fn standalone_irc_entry_points_share_the_singleton_dialog_and_alt_c_toggles_it() {
        let mut lobby_app = new_real_classic_menu_app(640, 480);
        install_test_classic_host_lobby(&mut lobby_app);
        lobby_app
            .process_classic_lobby_actions(vec![ClassicLobbyAction::Chat(
                LobbyChatRequest::OpenExternalDialog,
            )])
            .expect("the lobby IRC button opens the standalone dialog");
        assert!(lobby_app.classic_host_lobby.is_some());
        assert!(lobby_app.external_irc_dialog_visible);
        let dialog = lobby_app
            .external_irc_dialog
            .as_ref()
            .expect("the standalone network/chat controller exists");
        assert_eq!(
            dialog.mode(),
            clonk_frontend::startup_netdlg::NetDlgMode::Chat
        );
        assert_eq!(
            dialog.chat_bounds_override(),
            Some(clonk_frontend::startup_netdlg::NetDlgController::standalone_chat_bounds(640, 480))
        );
        let first_dialog_ptr = std::ptr::from_ref(dialog);
        lobby_app
            .process_classic_lobby_actions(vec![ClassicLobbyAction::Chat(
                LobbyChatRequest::OpenExternalDialog,
            )])
            .expect("a repeated lobby request raises the same dialog");
        assert!(lobby_app.external_irc_dialog_visible);
        assert_eq!(
            lobby_app
                .external_irc_dialog
                .as_ref()
                .map(std::ptr::from_ref),
            Some(first_dialog_ptr),
            "raising the singleton must preserve its UI-local controller state"
        );
        lobby_app.hide_external_irc_dialog();
        assert!(!lobby_app.external_irc_dialog_visible);
        assert!(lobby_app.external_irc_dialog.is_none());

        for modifiers in [
            ModifiersState::ALT,
            ModifiersState::ALT | ModifiersState::LOGO,
        ] {
            let mut runtime_app = new_classic_running_sandbox_app();
            runtime_app
                .bindings
                .rebind(ControlBindingId::Left, VirtualKeyCode::C);
            runtime_app
                .handle_modifiers_changed(modifiers)
                .expect("set the exact legacy IRC chord");
            runtime_app.menu_title_drag = Some(MenuTitleDrag::Ingame {
                player: runtime_app.local_owner,
                start_pointer: GuiPoint::new(20.0, 20.0),
                start_location: (40, 50),
            });
            runtime_app
                .handle_key(VirtualKeyCode::C, ElementState::Pressed)
                .expect("runtime Alt+C opens the standalone IRC dialog");
            assert!(runtime_app.external_irc_dialog_visible);
            assert!(
                runtime_app.menu_title_drag.is_none(),
                "activating C4ChatDlg releases an obscured menu-title drag"
            );
            runtime_app
                .handle_cursor_moved(PhysicalPosition::new(300.0, 200.0))
                .expect("standalone dialog owns motion after activation");
            runtime_app
                .handle_mouse_button(ElementState::Released)
                .expect("standalone dialog owns the pending pointer release");
            assert!(runtime_app.external_irc_dialog_visible);

            runtime_app
                .engine
                .player_mut(runtime_app.local_owner)
                .expect("local sandbox player")
                .control
                .pressed_coms = 1 << clonk_engine::COM_LEFT;
            runtime_app
                .handle_key(VirtualKeyCode::C, ElementState::Released)
                .expect("runtime IRC chord release must be consumed");
            assert_ne!(
                runtime_app
                    .engine
                    .player(runtime_app.local_owner)
                    .expect("local sandbox player")
                    .control
                    .pressed_coms
                    & (1 << clonk_engine::COM_LEFT),
                0,
                "runtime IRC release must not leak to modifier-blind player control"
            );
            runtime_app
                .handle_key(VirtualKeyCode::C, ElementState::Pressed)
                .expect("a second runtime Alt+C closes only the IRC UI");
            assert!(!runtime_app.external_irc_dialog_visible);
        }

        let mut ignored_runtime = new_running_sandbox_app();
        for modifiers in [
            ModifiersState::empty(),
            ModifiersState::CTRL,
            ModifiersState::SHIFT,
            ModifiersState::LOGO,
            ModifiersState::ALT | ModifiersState::CTRL,
            ModifiersState::ALT | ModifiersState::SHIFT,
            ModifiersState::ALT | ModifiersState::CTRL | ModifiersState::SHIFT,
        ] {
            ignored_runtime
                .handle_modifiers_changed(modifiers)
                .expect("set non-IRC modifiers");
            assert!(!ignored_runtime
                .handle_runtime_irc_toggle_key(VirtualKeyCode::C, ElementState::Pressed)
                .expect("non-IRC chord is unhandled"));
            assert!(!ignored_runtime.external_irc_dialog_visible);
        }
    }

    #[test]
    fn l046_dialog_hotkeys_use_the_first_sdl_key_name_character() {
        for (key, expected) in [
            (VirtualKeyCode::A, Some('A')),
            (VirtualKeyCode::Key7, Some('7')),
            (VirtualKeyCode::Space, Some('S')),
            (VirtualKeyCode::Up, Some('U')),
            (VirtualKeyCode::Left, Some('L')),
            (VirtualKeyCode::Return, Some('R')),
            (VirtualKeyCode::Escape, Some('E')),
            (VirtualKeyCode::PageUp, Some('P')),
            (VirtualKeyCode::Snapshot, Some('P')),
            (VirtualKeyCode::Numpad1, Some('K')),
            (VirtualKeyCode::Apps, Some('A')),
            (VirtualKeyCode::WebBack, Some('A')),
            (VirtualKeyCode::Minus, None),
            (VirtualKeyCode::Apostrophe, None),
            (VirtualKeyCode::OEM102, None),
        ] {
            assert_eq!(startup_dialog_hotkey(key), expected, "{key:?}");
        }
    }

    #[test]
    fn l046_startup_alt_mnemonics_route_before_plain_gui_keys_and_lower_owners() {
        let mut app = new_real_classic_menu_app(640, 480);

        app.handle_modifiers_changed(ModifiersState::CTRL | ModifiersState::ALT)
            .expect("hold unsupported Ctrl+Alt mask");
        for key in [
            VirtualKeyCode::Down,
            VirtualKeyCode::Return,
            VirtualKeyCode::Space,
            VirtualKeyCode::Escape,
        ] {
            app.handle_key(key, ElementState::Pressed)
                .expect("Ctrl+Alt GUI key down is inert");
            app.handle_key(key, ElementState::Released)
                .expect("Ctrl+Alt GUI key up is inert");
        }
        app.handle_key(VirtualKeyCode::A, ElementState::Pressed)
            .expect("Ctrl+Alt does not dispatch a mnemonic");
        assert_eq!(app.startup_view, StartupView::MainMenu);
        assert!(!app.exit_requested);

        app.handle_modifiers_changed(ModifiersState::ALT | ModifiersState::SHIFT)
            .expect("hold Alt+Shift");
        app.handle_key(VirtualKeyCode::A, ElementState::Pressed)
            .expect("dispatch shifted About mnemonic");
        assert_eq!(app.startup_view, StartupView::About);
        assert!(app.ui_sound_log.is_empty());
        app.show_main_menu();

        app.handle_modifiers_changed(ModifiersState::ALT)
            .expect("hold Alt");
        for key in [
            VirtualKeyCode::Down,
            VirtualKeyCode::Return,
            VirtualKeyCode::Escape,
        ] {
            app.handle_key(key, ElementState::Pressed)
                .expect("unmatched Alt GUI key down is inert");
            app.handle_key(key, ElementState::Released)
                .expect("unmatched Alt GUI key up is inert");
        }
        assert_eq!(app.startup_view, StartupView::MainMenu);
        assert!(!app.exit_requested);

        app.handle_modifiers_changed(ModifiersState::empty())
            .expect("release Alt");
        app.handle_key(VirtualKeyCode::Return, ElementState::Pressed)
            .expect("arm retained Start focus");
        app.handle_key(VirtualKeyCode::Return, ElementState::Released)
            .expect("activate retained Start focus");
        assert_eq!(app.startup_view, StartupView::ScenarioBrowser);
        assert!(app.ui_sound_log.iter().any(|sound| sound == "Click"));
        app.ui_sound_log.clear();
        app.show_main_menu();

        app.handle_key(VirtualKeyCode::Down, ElementState::Pressed)
            .expect("focus Network");
        app.handle_key(VirtualKeyCode::Down, ElementState::Released)
            .expect("release focus key");
        app.ui_sound_log.clear();
        app.handle_modifiers_changed(ModifiersState::ALT)
            .expect("hold Alt");
        app.handle_key(VirtualKeyCode::Space, ElementState::Pressed)
            .expect("SDL Space mnemonic dispatches Start");
        assert_eq!(app.startup_view, StartupView::ScenarioBrowser);
        assert!(
            !app.ui_sound_log.iter().any(|sound| sound == "Click"),
            "mnemonic dispatch must bypass the button Click sound: {:?}",
            app.ui_sound_log
        );

        app.show_main_menu();
        app.open_about_dialog();
        app.ui_sound_log.clear();
        app.handle_key(VirtualKeyCode::Left, ElementState::Pressed)
            .expect("SDL Left mnemonic opens Licenses");
        assert_eq!(
            app.startup_about_dialog
                .as_ref()
                .expect("About dialog")
                .current_page(),
            clonk_frontend::startup_about_dlg::AboutPage::Licenses
        );
        assert!(app.ui_sound_log.is_empty());
        app.handle_key(VirtualKeyCode::Up, ElementState::Pressed)
            .expect("SDL Up mnemonic requests updates");
        assert_eq!(app.message_dialogs.len(), 1);
        assert_eq!(app.message_dialogs[0].state.caption(), "Updates");
        assert!(app.ui_sound_log.is_empty());
        app.finish_message_dialog(clonk_frontend::message_dialog::MessageDialogResult::Ok)
            .expect("dismiss update handoff");

        app.show_main_menu();
        app.handle_game_over()
            .expect("forge stale menu evaluation state");
        app.handle_key(VirtualKeyCode::A, ElementState::Pressed)
            .expect("exclusive game-over owner swallows unmatched startup mnemonics");
        assert!(app.game_over_dialog.is_some());
        assert_eq!(app.startup_view, StartupView::MainMenu);
    }

    #[test]
    fn l047_player_typeahead_stays_behind_rename_and_modal_dialogs() {
        let mut app = new_classic_menu_app(640, 480);
        app.startup_player_models = ["Thomas", "tina"]
            .map(|name| clonk_frontend::startup_plrsel::PlrSelPlayer {
                name: name.to_string(),
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
        app.handle_text_input('T').expect("type into inline rename");
        assert_eq!(
            app.startup_player_dialog
                .as_ref()
                .expect("player dialog")
                .selected_index(),
            Some(0),
            "the covered list must not type-ahead"
        );
        assert_ne!(
            app.startup_crew_rename
                .as_ref()
                .expect("inline rename")
                .edit
                .text(),
            "Crew"
        );
        app.startup_crew_rename = None;

        app.push_message_dialog(
            clonk_frontend::message_dialog::MessageDialogState::regular_ok(
                "Covered",
                "Modal",
                clonk_frontend::message_dialog::MessageDialogIcon::NOTIFY,
            ),
            MessageDialogContinuation::None,
        )
        .expect("open modal dialog");
        app.handle_text_input('T').expect("text is swallowed by modal");
        app.handle_key(VirtualKeyCode::Apps, ElementState::Pressed)
            .expect("Apps is swallowed by modal");
        assert_eq!(
            app.startup_player_dialog
                .as_ref()
                .expect("player dialog")
                .selected_index(),
            Some(0)
        );
        assert!(app.context_menu.is_none());
    }

    #[test]
    fn l071_crew_rename_is_inline_reselects_invalid_and_commits_on_focus_loss() {
        let directory = tempdir().expect("crew rename fixture root");
        let player_path = directory.path().join("Ada.c4p");
        fs::create_dir(&player_path).expect("create player group");
        fs::write(
            player_path.join("Player.txt"),
            "[Player]\nName=Ada\n\n[Preferences]\nColorDw=255\n",
        )
        .expect("write player core");
        for (file_name, name) in [("Alpha.c4i", "Alpha"), ("Taken.c4i", "Taken")] {
            let crew = player_path.join(file_name);
            fs::create_dir(&crew).expect("create crew child");
            fs::write(
                crew.join("ObjectInfo.txt"),
                format!("[ObjectInfo]\nid=CLNK\nName={name}\nParticipation=1\n"),
            )
            .expect("write crew core");
        }
        let player_file = PlayerFile::load_from_path(&player_path).expect("load player fixture");
        let player_model = clonk_frontend::startup_plrsel::PlrSelPlayer {
            name: "Ada".to_string(),
            activated: false,
            big_icon: None,
            portrait: None,
            color_dw: 255,
            score: 0,
            rounds: 0,
            rounds_won: 0,
            rounds_lost: 0,
            total_playing_time: 0,
            comment: String::new(),
        };
        let mut app = new_classic_menu_app(640, 480);
        app.startup_player_files.push(StartupPlayerFile {
            path: player_path.clone(),
            file_name: "Ada.c4p".to_string(),
            player_file,
            render_model: player_model.clone(),
        });
        app.startup_player_models.push(player_model);
        app.open_player_selection_dialog();
        app.process_player_dialog_actions(vec![
            clonk_frontend::startup_plrsel::PlrSelAction::ShowCrew(0),
        ])
        .expect("enter crew mode");

        let alpha_index = app
            .startup_crew_models
            .iter()
            .position(|crew| crew.name == "Alpha")
            .expect("Alpha row");
        app.startup_player_dialog
            .as_mut()
            .expect("player dialog")
            .set_selected_index(Some(alpha_index));
        app.handle_key(VirtualKeyCode::F2, ElementState::Pressed)
            .expect("start inline crew rename");
        let rename = app.startup_crew_rename.as_ref().expect("inline rename");
        assert!(!rename.edit.label_visible());
        assert!(rename.edit.is_focused());
        assert_eq!(rename.edit.selected_text(), Some("Alpha"));
        assert!(app.startup_crew_rename_rect().is_some());
        assert!(app.game_option_input_dialog.is_none());
        for character in "Draft".chars() {
            app.handle_text_input(character)
                .expect("type draft before restarting rename");
        }
        app.handle_key(VirtualKeyCode::F2, ElementState::Pressed)
            .expect("F2 restarts the active rename");
        assert_eq!(
            app.startup_crew_rename
                .as_ref()
                .expect("restarted inline rename")
                .edit
                .selected_text(),
            Some("Alpha")
        );

        let edit_rect = app.startup_crew_rename_rect().expect("rename bounds");
        let edit_point = GuiPoint::new(
            (edit_rect.x + edit_rect.w / 2) as f32,
            (edit_rect.y + edit_rect.h / 2) as f32,
        );
        assert!(app.handle_startup_crew_rename_middle_down(edit_point, None));
        assert!(app
            .startup_crew_rename
            .as_ref()
            .expect("middle-clicked inline rename")
            .edit
            .selection_range()
            .is_none());
        app.handle_key(VirtualKeyCode::F2, ElementState::Pressed)
            .expect("restart after middle click");
        app.startup_crew_rename
            .as_mut()
            .expect("rename before double click")
            .last_click = Some(Instant::now());
        assert!(app.handle_startup_crew_rename_pointer_down(edit_point));
        assert!(!app
            .startup_crew_rename
            .as_ref()
            .expect("double-clicked inline rename")
            .edit
            .is_dragging());
        assert!(app.handle_startup_crew_rename_pointer_up(edit_point));
        app.handle_key(VirtualKeyCode::F2, ElementState::Pressed)
            .expect("restart after double click");
        app.startup_player_dialog
            .as_mut()
            .expect("player dialog")
            .set_pointer_position(Some(edit_point));
        let expected_edit_entries = app.startup_crew_rename_context_entries(false);
        assert!(expected_edit_entries.iter().any(|entry| {
            entry.action
                == Some(AppContextMenuCommand::StartupCrewRename(
                    clonk_frontend::startup_netdlg::NetDlgEditContextCommand::Cut,
                ))
        }));
        app.handle_right_mouse_button(ElementState::Pressed)
            .expect("open inline edit context");
        assert!(app.startup_crew_rename.is_some());
        assert!(matches!(
            app.context_menu
                .as_ref()
                .expect("inline edit context")
                .layout()
                .panels[0]
                .rows
                .len(),
            3 | 4
        ));
        app.close_context_menu_silently();

        let layout = app
            .startup_player_dialog
            .as_ref()
            .expect("player dialog")
            .layout();
        let same_row_point = GuiPoint::new(
            (layout.list_viewport.x + layout.item_height / 2) as f32,
            (layout.list_viewport.y + layout.item_pitch * alpha_index as i32
                - app
                    .startup_player_dialog
                    .as_ref()
                    .expect("player dialog")
                    .list_scroll_offset()
                + layout.item_height / 2) as f32,
        );
        let inert_row_point = GuiPoint::new(
            (layout.list_viewport.x + layout.item_height + layout.item_height / 2) as f32,
            same_row_point.y,
        );
        app.startup_player_dialog
            .as_mut()
            .expect("player dialog")
            .set_pointer_position(Some(inert_row_point));
        app.handle_mouse_button(ElementState::Pressed)
            .expect("press current row outside the edit");
        app.handle_mouse_button(ElementState::Released)
            .expect("release current row outside the edit");
        assert!(app.startup_crew_rename.is_some());

        app.startup_player_dialog
            .as_mut()
            .expect("player dialog")
            .set_pointer_position(Some(same_row_point));
        app.handle_right_mouse_button(ElementState::Pressed)
            .expect("open current crew row context");
        assert!(app.startup_crew_rename.is_some());
        assert_eq!(
            app.context_menu
                .as_ref()
                .expect("crew row context")
                .layout()
                .panels[0]
                .rows
                .len(),
            3
        );
        app.close_context_menu_silently();

        app.startup_player_dialog
            .as_mut()
            .expect("player dialog")
            .set_pointer_position(Some(same_row_point));
        app.handle_mouse_button(ElementState::Pressed)
            .expect("press crew participation checkbox");
        assert!(app.startup_crew_rename.is_some());
        app.handle_mouse_button(ElementState::Released)
            .expect("toggle crew participation checkbox");
        assert!(app.startup_crew_rename.is_none());
        assert!(player_path.join("Alpha.c4i").exists());
        app.handle_key(VirtualKeyCode::F2, ElementState::Pressed)
            .expect("restart after checkbox abort");

        app.handle_key(VirtualKeyCode::Escape, ElementState::Pressed)
            .expect("abort inline crew rename");
        assert!(app.startup_crew_rename.is_none());
        assert!(player_path.join("Alpha.c4i").exists());

        app.handle_key(VirtualKeyCode::F2, ElementState::Pressed)
            .expect("restart rename before row switch");
        for character in "Discarded".chars() {
            app.handle_text_input(character)
                .expect("type name that must be discarded");
        }
        let taken_index = app
            .startup_crew_models
            .iter()
            .position(|crew| crew.name == "Taken")
            .expect("Taken row");
        let other_row_point = GuiPoint::new(
            (layout.list_viewport.x + layout.item_height / 2) as f32,
            (layout.list_viewport.y + layout.item_pitch * taken_index as i32
                - app
                    .startup_player_dialog
                    .as_ref()
                    .expect("player dialog")
                    .list_scroll_offset()
                + layout.item_height / 2) as f32,
        );
        app.startup_player_dialog
            .as_mut()
            .expect("player dialog")
            .set_pointer_position(Some(other_row_point));
        app.handle_right_mouse_button(ElementState::Pressed)
            .expect("switch row through crew context");
        assert!(app.startup_crew_rename.is_none());
        assert!(player_path.join("Alpha.c4i").exists());
        assert!(!player_path.join("Discarded.c4i").exists());
        assert_eq!(
            app.startup_player_dialog
                .as_ref()
                .expect("player dialog")
                .selected_index(),
            Some(taken_index)
        );
        assert!(app.context_menu.is_some());
        app.close_context_menu_silently();
        app.startup_player_dialog
            .as_mut()
            .expect("player dialog")
            .set_selected_index(Some(alpha_index));

        app.handle_key(VirtualKeyCode::F2, ElementState::Pressed)
            .expect("restart inline crew rename");
        for character in "Taken".chars() {
            app.handle_text_input(character).expect("type colliding name");
        }
        app.handle_key(VirtualKeyCode::Return, ElementState::Pressed)
            .expect("submit colliding name");
        let rename = app
            .startup_crew_rename
            .as_ref()
            .expect("invalid rename stays active");
        assert!(rename.edit.is_focused());
        assert_eq!(rename.edit.selected_text(), Some("Taken"));
        let collision = app.message_dialogs.last().expect("collision dialog");
        assert_eq!(collision.state.caption(), "Rename failure.");
        assert_eq!(
            collision.state.message(),
            "A Clonk with the file name \"Taken.c4i\" exists already."
        );
        app.finish_message_dialog(clonk_frontend::message_dialog::MessageDialogResult::Ok)
            .expect("dismiss collision dialog");

        for character in "Renamed".chars() {
            app.handle_text_input(character).expect("type unique name");
        }
        app.handle_key(VirtualKeyCode::Return, ElementState::Pressed)
            .expect("commit inline crew rename");
        assert!(app.startup_crew_rename.is_none());
        assert!(!player_path.join("Alpha.c4i").exists());
        assert!(player_path.join("Renamed.c4i").exists());
        assert_eq!(
            app.startup_player_dialog
                .as_ref()
                .expect("player dialog")
                .focused_control(),
            PlrSelControl::PlayerList
        );

        let renamed_index = app
            .startup_crew_models
            .iter()
            .position(|crew| crew.name == "Renamed")
            .expect("renamed row");
        app.startup_player_dialog
            .as_mut()
            .expect("player dialog")
            .set_selected_index(Some(renamed_index));
        app.handle_key(VirtualKeyCode::F2, ElementState::Pressed)
            .expect("start focus-loss rename");
        let focus_loss_name = "Blurred crew name exceeds thirty";
        let focus_loss_file = crew_file_name_for_title(focus_loss_name);
        for character in focus_loss_name.chars() {
            app.handle_text_input(character)
                .expect("type focus-loss name");
        }
        app.handle_key(VirtualKeyCode::Tab, ElementState::Pressed)
            .expect("commit rename on focus loss");
        assert!(app.startup_crew_rename.is_none());
        assert!(!player_path.join("Renamed.c4i").exists());
        assert!(player_path.join(&focus_loss_file).exists());
        assert_eq!(
            app.startup_player_dialog
                .as_ref()
                .expect("player dialog")
                .focused_control(),
            PlrSelControl::PlayerList
        );

        let focus_loss_index = app
            .startup_crew_models
            .iter()
            .position(|crew| crew.name == focus_loss_name)
            .expect("full untruncated focus-loss renamed row");
        assert!(
            fs::read_to_string(player_path.join(&focus_loss_file).join("ObjectInfo.txt"))
                .expect("read truncated persisted crew core")
                .contains("Name=Blurred crew name exceeds thir")
        );
        app.startup_player_dialog
            .as_mut()
            .expect("player dialog")
            .set_selected_index(Some(focus_loss_index));
        app.handle_key(VirtualKeyCode::F2, ElementState::Pressed)
            .expect("start partial-persistence rename");
        for character in "Partial".chars() {
            app.handle_text_input(character)
                .expect("type partially persisted name");
        }
        fs::rename(
            player_path.join(&focus_loss_file),
            player_path.join("Partial.c4i"),
        )
        .expect("simulate the accepted filename change before RewriteCore fails");
        app.accept_startup_crew_rename_after_rewrite_failure(
            focus_loss_index,
            &player_path,
            &focus_loss_file,
            "Partial.c4i",
            "Partial",
        )
        .expect("accept the partially persisted rename");
        assert!(app.startup_crew_rename.is_none());
        let partial_index = app
            .startup_crew_files
            .iter()
            .position(|entry| entry.file_name == "Partial.c4i")
            .expect("partially persisted crew row");
        assert_eq!(app.startup_crew_models[partial_index].name, "Partial");
        assert_eq!(
            app.startup_crew_files[partial_index].file_name,
            "Partial.c4i"
        );
        assert_eq!(
            app.startup_crew_files[partial_index].crew_info.name,
            "Partial"
        );
        assert!(
            fs::read_to_string(player_path.join("Partial.c4i/ObjectInfo.txt"))
                .expect("read stale core after simulated rewrite failure")
                .contains("Name=Blurred crew name exceeds thir")
        );
        let rewrite_failure = app.message_dialogs.last().expect("rewrite failure dialog");
        assert_eq!(rewrite_failure.state.caption(), "");
        assert_eq!(
            rewrite_failure.state.message(),
            "File modification failure."
        );
        app.finish_message_dialog(clonk_frontend::message_dialog::MessageDialogResult::Ok)
            .expect("dismiss rewrite failure dialog");

        app.handle_key(VirtualKeyCode::F2, ElementState::Pressed)
            .expect("start rename before leaving player selection");
        assert!(app.startup_crew_rename.is_some());
        app.process_player_dialog_actions(vec![
            clonk_frontend::startup_plrsel::PlrSelAction::Back,
        ])
        .expect("leave player selection while rename is active");
        assert_eq!(app.startup_view, StartupView::MainMenu);
        assert!(app.startup_crew_rename.is_none());
    }

    #[test]
    fn player_properties_context_closes_and_opens_the_editor() {
        let mut app = new_classic_menu_app(640, 480);
        let model = clonk_frontend::startup_plrsel::PlrSelPlayer {
            name: "Context Player".to_string(),
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
        };
        app.startup_player_files.push(StartupPlayerFile {
            path: PathBuf::from("Context Player.c4p"),
            file_name: "Context Player.c4p".to_string(),
            player_file: PlayerFile::default(),
            render_model: model.clone(),
        });
        app.startup_player_models.push(model);
        app.open_player_selection_dialog();
        let layout = clonk_frontend::startup_plrsel::plrsel_layout(640, 480);
        app.startup_player_dialog
            .as_mut()
            .unwrap()
            .set_pointer_position(Some(GuiPoint::new(
                (layout.list_client.x + layout.item_height * 2) as f32,
                (layout.list_client.y + layout.item_height / 2) as f32,
            )));
        assert!(app
            .open_startup_player_context_menu(false)
            .expect("open exact player context"));
        assert!(app.context_menu.is_some());
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
            .expect("Properties callback opens the editor");
        assert!(app.context_menu.is_none());
        assert!(matches!(
            app.startup_player_properties_dialog
                .as_ref()
                .map(|pending| pending.controller.mode()),
            Some(clonk_frontend::startup_plrproperties::PlayerPropertiesMode::Edit { index: 0 })
        ));
        assert!(app.status_text.is_empty());
        assert!(app.message_dialogs.is_empty());
        assert_eq!(app.startup_player_models.len(), before_models);
        assert_eq!(app.startup_player_files.len(), before_files);
    }

    #[test]
    fn cached_menu_requests_the_deferred_monitor_gamma_post_pass() {
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
        app.menu_frame_cache = None;

        let mut cold = vec![0_u8; 320 * 240 * 4];
        assert!(app
            .render_for_presentation_with_monitor_defer(&mut cold, false, false, false, true,)
            .expect("render raw cached menu base"));
        let cached = app
            .menu_frame_cache
            .as_ref()
            .expect("cold render caches the raw logical frame")
            .frame
            .clone();
        assert_eq!(cold, cached);

        let mut deferred_hit = vec![0x55; cold.len()];
        assert!(app
            .render_for_presentation_with_monitor_defer(
                &mut deferred_hit,
                false,
                false,
                false,
                true,
            )
            .expect("replay raw cache for a physical post-pass"));
        assert_eq!(deferred_hit, cached);

        let mut direct_hit = vec![0x77; cold.len()];
        assert!(
            !app.render_for_presentation_with_monitor_defer(
                &mut direct_hit,
                false,
                false,
                false,
                false,
            )
            .expect("direct replay applies its own monitor gamma")
        );
        let mut expected = cached;
        configured_gamma.apply_to_rgba_bytes(&mut expected);
        assert_eq!(direct_hit, expected);
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
            .expect("startup caption sheet")
            .clone();
        let pristine_scroll = app
            .assets
            .startup_dialog_images
            .get("GUIScroll.png")
            .expect("startup scroll sheet")
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
            .expect("applied overrides keep the global GUI bundle valid");
        let message = app
            .assets
            .message_dialog_resources()
            .expect("message dialog resources resolve from the rebound sheets");
        assert_eq!(message.progress.pixels()[..4], [0x77, 0x88, 0x99, 0xff]);
        app.assets
            .input_dialog_resources()
            .expect("input dialog resources resolve from the rebound sheets");
        assert_eq!(
            app.assets
                .startup_dialog_images
                .get("GUICaption.png")
                .expect("rebound caption sheet")
                .pixels()[..4],
            [0x11, 0x22, 0x33, 0xff],
            "the caption consumed by every dialog skin must be the override"
        );
        let info = app
            .assets
            .static_info_dialog_resources()
            .expect("info dialog resources resolve from the rebound sheets");
        assert_eq!(info.scroll.pixels()[..4], [0x44, 0x55, 0x66, 0xff]);
        assert_eq!(
            app.ensure_ingame_menu_gfx()
                .caption_bar
                .as_ref()
                .expect("script menus keep a caption bar")
                .pixels()[..4],
            [0x11, 0x22, 0x33, 0xff],
            "script-menu graphics must read the rebound caption sheet"
        );

        // Startup teardown (Resource::Clear + CloseFiles) restores the
        // pristine startup sheets for the next startup generation.
        app.show_main_menu();
        assert!(app.assets.active_gui_sheet_sources.is_empty());
        assert!(app.assets.startup_gui_sheet_images.is_empty());
        assert_eq!(
            app.assets
                .startup_dialog_images
                .get("GUICaption.png")
                .expect("restored caption sheet")
                .pixels()
                .as_ptr(),
            pristine_caption.pixels().as_ptr(),
            "teardown must restore the pristine caption surface"
        );
        assert_eq!(
            app.assets
                .startup_dialog_images
                .get("GUIScroll.png")
                .expect("restored scroll sheet")
                .pixels()
                .as_ptr(),
            pristine_scroll.pixels().as_ptr(),
            "teardown must restore the pristine scroll surface"
        );
        assert!(
            app.ingame_menu_gfx.is_none(),
            "cached script-menu graphics must not outlive the rebound sheets"
        );
    }

    #[test]
    fn active_gui_sheet_overrides_rebind_only_when_the_winning_source_changes() {
        let mut app = new_menu_app(320, 200);
        let pristine_highlight = app
            .assets
            .startup_dialog_images
            .get("GUIButtonHighlight.png")
            .expect("startup highlight sheet")
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
            .expect("applied highlight sheet")
            .pixels()
            .as_ptr();
        assert_eq!(applied_ptr, first[0].image.pixels().as_ptr());
        assert_eq!(
            app.assets
                .button_highlight
                .as_ref()
                .expect("derived button highlight")
                .pixels()[..4],
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
        assert_eq!(
            app.assets
                .startup_dialog_images
                .get("GUIButtonHighlight.png")
                .expect("cached highlight sheet")
                .pixels()
                .as_ptr(),
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
        assert_eq!(
            app.assets
                .startup_dialog_images
                .get("GUIButtonHighlight.png")
                .expect("reloaded highlight sheet")
                .pixels()[..4],
            [0xa0, 0xb0, 0xc0, 0xff],
            "a changed winning source must rebind the sheet"
        );

        // A refresh where the global group wins again restores the pristine
        // surface without waiting for teardown.
        app.install_active_gui_sheet_overrides(&[]);
        assert_eq!(
            app.assets
                .startup_dialog_images
                .get("GUIButtonHighlight.png")
                .expect("restored highlight sheet")
                .pixels()
                .as_ptr(),
            pristine_highlight.pixels().as_ptr(),
            "losing every override must restore the pristine surface"
        );
        assert_eq!(
            app.assets
                .button_highlight
                .as_ref()
                .expect("restored derived highlight")
                .pixels()
                .as_ptr(),
            pristine_highlight.pixels().as_ptr(),
            "derived highlight state must follow the restored sheet"
        );
        assert!(app.assets.active_gui_sheet_sources.is_empty());
        assert!(app.assets.startup_gui_sheet_images.is_empty());
    }

    #[test]
    fn real_mars_full_size_highlight_reaches_host_gui_resources() {
        let _lock = env_lock().lock();
        let user_data = tempdir().expect("isolated Mars GUI user data");
        let (_guard, paths) = exact_loader_test_paths(user_data.path(), None);
        let mut app = new_menu_app_with_paths(320, 200, &paths);
        let scenario =
            resolve_next_mission_scenario(&app.scenario_catalog, "ClonkMars.c4f/01_Fossae.c4s")
                .expect("Mars Fossae is present in the real scenario catalog");
        let setup = build_scenario_loader(
            &scenario,
            &app.scenario_seed_definition_load(),
            &paths,
            app.assets.as_ref(),
        )
        .expect("resolve the real Mars scenario loader and GUI refresh");
        let highlight = setup
            .refreshed_gui_sheet_overrides
            .iter()
            .find(|sheet| sheet.stem == "GUIButtonHighlight")
            .cloned()
            .expect("Mars parent Graphics.c4g wins GUIButtonHighlight");

        // C4GUI::Resource::Load keeps the winning C4FCT_Full dimensions and
        // C4Facet::DrawX stretches that complete source for every consumer
        // (src/C4Gui.cpp:1093; src/C4FacetEx.cpp:137-161;
        // src/C4Facet.cpp:296-304).
        assert!(
            highlight.source.contains("ClonkMars.c4f/Graphics.c4g"),
            "unexpected Mars highlight source: {}",
            highlight.source
        );
        assert_eq!(
            (highlight.image.width(), highlight.image.height()),
            (30, 30)
        );
        app.install_active_gui_sheet_overrides(std::slice::from_ref(&highlight));
        assert_eq!(
            app.assets
                .startup_dialog_images
                .get("GUIButtonHighlight.png")
                .map(|image| (image.width(), image.height())),
            Some((30, 30))
        );
        app.assets
            .network_start_wait_resources()
            .expect("host start-wait accepts the full Mars facet");
        app.assets
            .game_lobby_resources()
            .expect("host lobby accepts the full Mars facet");
        app.assets
            .game_option_resources()
            .expect("game options accept the full Mars facet");
        app.assets
            .input_dialog_resources()
            .expect("input dialogs accept the full Mars facet");
        let scensel = app
            .assets
            .scensel_assets()
            .expect("scenario selector assets remain complete");
        let button_down = app
            .assets
            .dialog_image("GUIButtonDown.png")
            .expect("active down-button plank");
        clonk_frontend::startup_scensel::validate_scensel_button_assets(
            &scensel,
            &button_down,
        )
        .expect("scenario buttons accept the full Mars facet");
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
            assert_eq!(runtime_global_ui_snapshot(&app), before, "{label}");
            assert!(frame.iter().all(|byte| *byte == 0x84), "{label}");
        };

        let pages = vec![
            (
                "C4MainMenu::Main",
                IngameMenuState::main_menu(&MainMenuConditions::default())
                    .expect("nonempty main menu"),
            ),
            (
                "C4MainMenu::Goals",
                IngameMenuState::goals_menu(&[GoalRuleEntry {
                    definition_id: "GOAL".to_string(),
                    name: "Goal".to_string(),
                    description: None,
                    fulfilled: false,
                }]),
            ),
            (
                "C4MainMenu::Rules",
                IngameMenuState::rules_menu(&[GoalRuleEntry {
                    definition_id: "RULE".to_string(),
                    name: "Rule".to_string(),
                    description: None,
                    fulfilled: false,
                }]),
            ),
            (
                "C4MainMenu::NewPlayer",
                IngameMenuState::new_player_menu(&[ingame_menu::NewPlayerEntry {
                    file: "Player.c4p".to_string(),
                    name: "Player".to_string(),
                }]),
            ),
            (
                "C4MainMenu::Savegame",
                IngameMenuState::savegame_menu(&[SaveSlotState { free: true }; 10]),
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
                ),
            ),
            (
                "C4MainMenu::Display",
                IngameMenuState::display_menu(&DisplayFlags::default(), 0),
            ),
            ("C4MainMenu::Surrender", IngameMenuState::surrender_menu()),
            (
                "C4MainMenu::ClientDisconnect",
                IngameMenuState::client_disconnect_menu(),
            ),
            (
                "C4MainMenu::HostDisconnect",
                IngameMenuState::host_disconnect_menu(&[HostDisconnectClientEntry {
                    client_id: 0,
                    caption: "Host (Host)".to_string(),
                    activated: true,
                }]),
            ),
        ];
        assert_eq!(pages.len(), 10, "MenuPage exhaustiveness changed");
        for (label, page) in pages {
            let mut app = new_running_sandbox_app();
            app.ingame_menu.replace(app.local_owner, Some(page));
            check(app, label);
        }

        let mut object = new_running_sandbox_app();
        assert!(object
            .open_object_menu()
            .expect("open app-owned object menu"));
        check(object, "app-owned object menu");

        for mode in [
            SaveBrowserMode::Save {
                suggested_label: "Slot".to_string(),
            },
            SaveBrowserMode::Load,
        ] {
            let mut app = new_running_sandbox_app();
            app.save_browser = Some(SaveBrowserState::new(mode.clone(), Vec::new()));
            check(
                app,
                match mode {
                    SaveBrowserMode::Save { .. } => "save browser",
                    SaveBrowserMode::Load => "load browser",
                },
            );
        }

        let mut scoreboard = new_running_sandbox_app();
        scoreboard.scoreboard_dialog = Some(scoreboard.scoreboard_request());
        check(scoreboard, "visible scoreboard");

        for style in 0..=3 {
            let mut app = new_running_sandbox_app();
            let cursor = app
                .engine
                .crew_cursor(app.local_owner)
                .expect("sandbox cursor");
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
                .expect("install engine script menu");
            check(app, &format!("engine script menu style {style}"));
        }
    }

    #[test]
    fn global_gui_guard_is_first_at_every_external_ui_ingress() {
        let mut app = new_classic_menu_app(320, 200);
        remove_global_gui_sheet(&mut app, "GUISpinBoxArrow.png");
        let version = app.menu_render_version;
        let modifiers = app.keyboard_modifiers;
        let dimensions = {
            let surface = app.graphics.surface();
            (surface.width(), surface.height())
        };
        let engine_game_time = app.engine.game_time();
        let snapshot_game_time = app.snapshot.game_time;
        let mut second_accumulator = Duration::from_millis(125);
        let expect_engine = |result: Result<(), EngineError>| {
            assert!(matches!(
                result,
                Err(EngineError::ClassicMenuParityBoundary { ref detail })
                    if detail.contains("GUISpinBoxArrow")
            ));
        };
        expect_engine(app.handle_modifiers_changed(ModifiersState::SHIFT));
        expect_engine(app.handle_text_input('x'));
        expect_engine(app.handle_key(VirtualKeyCode::A, ElementState::Pressed));
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
            advance_game_clock_from_elapsed(
                &mut app,
                &mut second_accumulator,
                Duration::from_secs(1),
            )
            .map(|_| ()),
        );
        expect_engine(app.update());
        let resize = app
            .resize(640, 480)
            .expect_err("resize must fail at global guard");
        assert!(matches!(
            resize.downcast_ref::<ClassicParityBoundary>(),
            Some(ClassicParityBoundary::GlobalGuiBootstrapResources { .. })
        ));
        assert_eq!(app.menu_render_version, version);
        assert_eq!(app.keyboard_modifiers, modifiers);
        let surface = app.graphics.surface();
        assert_eq!((surface.width(), surface.height()), dimensions);
        assert_eq!(app.engine.game_time(), engine_game_time);
        assert_eq!(app.snapshot.game_time, snapshot_game_time);
        assert_eq!(second_accumulator, Duration::from_millis(125));
        assert!(app.context_menu.is_none());
        assert!(app.message_dialogs.is_empty());
    }

    #[test]
    fn l002_ingame_menu_abort_routes_to_the_same_confirmation() {
        let mut app = new_menu_app(320, 200);
        app.start_sandbox_scenario(FrontendScenario::fallback())
            .expect("start explicit test sandbox");
        let mut menu =
            IngameMenuState::main_menu(&MainMenuConditions::default()).expect("main menu");
        let abort = menu
            .items()
            .iter()
            .position(|item| item.action == MenuAction::Abort)
            .expect("abort item");
        menu.set_selection(abort);
        app.ingame_menu.replace(app.local_owner, Some(menu));
        app.status_text.clear();

        app.handle_menu_command_failsafe(
            app.local_owner,
            ControlCommand::MenuEnter,
            CommandKind::Press,
        )
            .expect("production Abort opens the confirmation");
        assert!(app.message_dialogs.last().is_some_and(|dialog| matches!(
            dialog.continuation,
            MessageDialogContinuation::AbortGame { .. }
        )));
        assert!(
            app.ingame_menu.is_none(),
            "C4Menu::Enter closes the nonpermanent main menu before Abort"
        );
        assert!(matches!(app.mode, AppMode::Running));
        assert!(app.status_text.is_empty());
    }

    #[test]
    fn unported_object_menu_requests_fail_before_generic_object_menu_state_exists() {
        let mut app = new_state_only_menu_app(320, 200);
        app.start_sandbox_scenario(FrontendScenario::fallback())
            .expect("start explicit test sandbox");
        let crew_id = app
            .snapshot
            .objects
            .iter()
            .find(|object| object.crew_member)
            .expect("sandbox crew")
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
            assert!(error.to_string().contains(label), "unexpected {error}");
            assert!(app.object_menu.is_none());
        }

        app.snapshot.menu_requests = vec![clonk_engine::MenuRequest {
            crew_id,
            owner: app.local_owner,
            kind: MenuRequestKind::Construction,
        }];
        app.handle_menu_requests()
            .expect("stale engine-owned construction request is ignored");
        assert!(app.object_menu.is_none());
    }

    #[test]
    fn rust_only_running_function_keys_fail_without_opening_panes() {
        let mut app = new_menu_app(320, 200);
        app.start_sandbox_scenario(FrontendScenario::fallback())
            .expect("start explicit test sandbox");
        for (key, label) in [
            (VirtualKeyCode::F5, "F5"),
            (VirtualKeyCode::F6, "F6"),
            (VirtualKeyCode::F7, "F7"),
        ] {
            let error = app
                .handle_key(key, ElementState::Pressed)
                .expect_err("unported running shortcut must fail");
            assert!(error.to_string().contains(label));
            assert!(app.save_browser.is_none());
        }
    }

    #[test]
    fn screenshot_path_reuses_the_first_numbered_gap() {
        let directory = tempdir().expect("screenshot directory");
        fs::write(directory.path().join("Screenshot001.png"), b"one")
            .expect("occupy first screenshot path");
        fs::write(directory.path().join("Screenshot003.png"), b"three")
            .expect("occupy third screenshot path");

        assert_eq!(
            next_screenshot_path(directory.path()),
            directory.path().join("Screenshot002.png")
        );
    }

    // BoolConfig initializes the Timestamps checkbox from
    // Config.General.ShowLogTimestamps (C4StartupOptionsDlg.cpp:558-560,
    // 749-753; C4Config.cpp:398).
    #[test]
    fn options_dialog_loads_log_timestamps_from_general_config() {
        let install_root = Path::new(env!("CARGO_MANIFEST_DIR"))
            .parent()
            .and_then(Path::parent)
            .expect("repository root");
        let user_data = tempdir().expect("user data");
        let _guard = EnvGuard::set(&[
            ("LC_INSTALL_ROOT", Some(install_root)),
            ("LC_USER_DATA_DIR", Some(user_data.path())),
        ]);
        let paths = AppPaths::discover().expect("discover app paths");
        persist_config_value(&paths, "General", "ShowLogTimestamps", "1")
            .expect("seed timestamp config");
        let mut app = GameApp::new(
            1280,
            720,
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
        wait_for_menu(&mut app);

        app.open_options_menu();

        assert!(
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
        app.render(&mut program_frame)
            .expect("Program remains available without audio");

        app.handle_key(VirtualKeyCode::Down, ElementState::Pressed)
            .expect("Graphics remains available without audio");
        app.handle_key(VirtualKeyCode::Down, ElementState::Released)
            .expect("release Graphics navigation");

        let sound_error = app
            .handle_key(VirtualKeyCode::Down, ElementState::Pressed)
            .expect_err("Sound requires the live audio context");
        assert_engine_parity_boundary(
            sound_error,
            ClassicParityBoundary::RuntimeAudioSystem {
                action: "the startup Options Sound sheet",
            },
        );
        assert_eq!(
            app.startup_options_dialog
                .as_ref()
                .expect("retained options model")
                .active_sheet(),
            clonk_frontend::startup_options_dlg::OptionsSheet::Sound
        );

        let mut frame = vec![0xa5; 320 * 200 * 4];
        let error = app
            .render(&mut frame)
            .expect_err("render preflight must reject guessed Sound state");
        let expected = ClassicParityBoundary::RuntimeAudioSystem {
            action: "the startup Options Sound sheet",
        };
        assert_eq!(
            error.downcast_ref::<ClassicParityBoundary>(),
            Some(&expected)
        );
        assert!(frame.iter().all(|byte| *byte == 0xa5));
    }

    #[test]
    fn secondary_startup_dialogs_route_their_visible_controls() {
        // C4StartupMainDlg switches to concrete dialogs whose controls remain
        // live (C4StartupMainDlg.cpp:209-242). This guards the app-level seam:
        // the parity renderer and the controller must be the same state.
        let _lock = env_lock().lock();
        let install_root = Path::new(env!("CARGO_MANIFEST_DIR"))
            .parent()
            .and_then(Path::parent)
            .expect("repository root");
        let user_data = tempdir().expect("user data");
        let _guard = EnvGuard::set(&[
            ("LC_INSTALL_ROOT", Some(install_root)),
            ("LC_USER_DATA_DIR", Some(user_data.path())),
        ]);
        let paths = AppPaths::discover().expect("discover app paths");
        configure_test_startup_participant(&paths, user_data.path());
        let mut app = GameApp::new(
            1280,
            720,
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
        )
        .expect("initialise app");
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
            app.handle_cursor_moved(point)
                .expect("move over main button");
            app.handle_mouse_button(ElementState::Pressed)
                .expect("press main button");
            app.handle_mouse_button(ElementState::Released)
                .expect("release main button");
        };
        let settle_startup_fade = |app: &mut GameApp| {
            assert!(app.startup_dialog_fade_active());
            let mut frame = vec![0_u8; 1280 * 720 * 4];
            for _ in 0..STARTUP_DIALOG_FADE_STEPS {
                app.render(&mut frame)
                    .expect("complete startup dialog transition");
            }
            assert!(!app.startup_dialog_fade_active());
        };

        click_main_button(&mut app, 0);
        assert_eq!(app.startup_view, StartupView::ScenarioBrowser);
        app.show_main_menu();

        click_main_button(&mut app, 1);
        assert_eq!(app.startup_view, StartupView::NetworkGame);
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
        app.handle_cursor_moved(network_point)
            .expect("move over network Back");
        app.handle_mouse_button(ElementState::Pressed)
            .expect("press network Back");
        app.handle_mouse_button(ElementState::Released)
            .expect("release network Back");
        assert_eq!(app.startup_view, StartupView::MainMenu);
        settle_startup_fade(&mut app);

        let test_player = clonk_frontend::startup_plrsel::PlrSelPlayer {
            name: "Test Player".to_string(),
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
        };
        app.startup_player_files.push(StartupPlayerFile {
            path: user_data.path().join("Test Player.c4p"),
            file_name: "Test Player.c4p".to_string(),
            player_file: PlayerFile::default(),
            render_model: test_player.clone(),
        });
        app.startup_player_models.push(test_player);
        click_main_button(&mut app, 2);
        assert_eq!(app.startup_view, StartupView::PlayerSelection);
        settle_startup_fade(&mut app);
        let player_layout = clonk_frontend::startup_plrsel::plrsel_layout(1280, 720);
        let player_row = PhysicalPosition::new(
            f64::from(player_layout.list_client.x + player_layout.item_height + 4),
            f64::from(player_layout.list_client.y + player_layout.item_height / 2),
        );
        app.handle_cursor_moved(player_row)
            .expect("move over player row");
        app.handle_mouse_button(ElementState::Pressed)
            .expect("press player row");
        app.handle_mouse_button(ElementState::Released)
            .expect("release first player-row click");
        app.handle_mouse_button(ElementState::Pressed)
            .expect("press player row again");
        app.handle_mouse_button(ElementState::Released)
            .expect("double-click opens Properties");
        assert!(matches!(
            app.startup_player_properties_dialog
                .as_ref()
                .map(|pending| pending.controller.mode()),
            Some(clonk_frontend::startup_plrproperties::PlayerPropertiesMode::Edit { index: 0 })
        ));
        assert!(app.status_text.is_empty());
        app.handle_key(VirtualKeyCode::Escape, ElementState::Pressed)
            .expect("cancel player Properties");
        app.handle_key(VirtualKeyCode::Escape, ElementState::Released)
            .expect("release cancel");
        let player_back = player_layout.buttons[0];
        let player_point = PhysicalPosition::new(
            f64::from(player_back.x + player_back.w / 2),
            f64::from(player_back.y + player_back.h / 2),
        );
        app.handle_cursor_moved(player_point)
            .expect("move over player Back");
        app.handle_mouse_button(ElementState::Pressed)
            .expect("press player Back");
        app.handle_mouse_button(ElementState::Released)
            .expect("release player Back");
        assert_eq!(app.startup_view, StartupView::MainMenu);
        settle_startup_fade(&mut app);

        click_main_button(&mut app, 3);
        assert_eq!(app.startup_view, StartupView::Options);
        settle_startup_fade(&mut app);
        app.handle_key(VirtualKeyCode::Down, ElementState::Pressed)
            .expect("select Graphics sheet");
        app.handle_key(VirtualKeyCode::Down, ElementState::Released)
            .expect("release Graphics navigation");
        assert_eq!(
            app.startup_options_dialog
                .as_ref()
                .expect("options state")
                .active_sheet(),
            clonk_frontend::startup_options_dlg::OptionsSheet::Graphics
        );
        app.handle_key(VirtualKeyCode::Down, ElementState::Pressed)
            .expect("advance to the implemented Sound sheet");
        app.handle_key(VirtualKeyCode::Down, ElementState::Released)
            .expect("release Sound navigation");
        assert_eq!(
            app.startup_options_dialog
                .as_ref()
                .expect("options state")
                .active_sheet(),
            clonk_frontend::startup_options_dlg::OptionsSheet::Sound
        );

        app.handle_key(VirtualKeyCode::Down, ElementState::Pressed)
            .expect("advance to the Keyboard sheet");
        app.handle_key(VirtualKeyCode::Down, ElementState::Released)
            .expect("release Keyboard navigation");
        assert_eq!(
            app.startup_options_dialog
                .as_ref()
                .expect("options state")
                .active_sheet(),
            clonk_frontend::startup_options_dlg::OptionsSheet::Keyboard
        );
        app.handle_key(VirtualKeyCode::R, ElementState::Pressed)
            .expect("generic keyboard control pane is disconnected");
        assert!(app.status_text.is_empty());
        app.handle_key(VirtualKeyCode::Back, ElementState::Pressed)
            .expect("Back leaves options");
        assert_eq!(app.startup_view, StartupView::MainMenu);
        settle_startup_fade(&mut app);

        click_main_button(&mut app, 4);
        assert_eq!(app.startup_view, StartupView::About);
        settle_startup_fade(&mut app);
        let about_layout = clonk_frontend::startup_about_dlg::about_layout(1280, 720);
        let licenses = about_layout.buttons[2];
        let licenses_point = PhysicalPosition::new(
            f64::from(licenses.x + licenses.w / 2),
            f64::from(licenses.y + licenses.h / 2),
        );
        app.handle_cursor_moved(licenses_point)
            .expect("move over Licenses");
        app.handle_mouse_button(ElementState::Pressed)
            .expect("press Licenses");
        app.handle_mouse_button(ElementState::Released)
            .expect("open Licenses");
        assert_eq!(
            app.startup_about_dialog
                .as_ref()
                .expect("about state")
                .current_page(),
            clonk_frontend::startup_about_dlg::AboutPage::Licenses
        );
        let mut licenses_frame = vec![0_u8; 1280 * 720 * 4];
        app.render(&mut licenses_frame)
            .expect("render the classic Licenses page");
        assert!(licenses_frame.iter().any(|byte| *byte != 0));

        let about_back = about_layout.buttons[0];
        let about_back_point = PhysicalPosition::new(
            f64::from(about_back.x + about_back.w / 2),
            f64::from(about_back.y + about_back.h / 2),
        );
        app.handle_cursor_moved(about_back_point)
            .expect("move over About Back");
        app.handle_mouse_button(ElementState::Pressed)
            .expect("press About Back");
        app.handle_mouse_button(ElementState::Released)
            .expect("return from licenses");
        assert_eq!(app.startup_view, StartupView::About);

        let mut credits = vec![0_u8; 1280 * 720 * 4];
        app.render(&mut credits).expect("render credits");
        let update = about_layout.buttons[1];
        let update_point = PhysicalPosition::new(
            f64::from(update.x + update.w / 2),
            f64::from(update.y + update.h / 2),
        );
        app.handle_cursor_moved(update_point)
            .expect("move over Update");
        app.handle_mouse_button(ElementState::Pressed)
            .expect("press Update");
        app.handle_mouse_button(ElementState::Released)
            .expect("show launcher update hand-off");
        assert_eq!(app.startup_view, StartupView::About);
        let handoff = app.message_dialogs.last().expect("visible update result");
        assert_eq!(handoff.state.caption(), "Updates");
        assert!(handoff.state.message().contains("launcher or package manager"));
        app.handle_key(VirtualKeyCode::Return, ElementState::Pressed)
            .expect("press update hand-off OK");
        app.handle_key(VirtualKeyCode::Return, ElementState::Released)
            .expect("dismiss update hand-off");
        assert!(app.message_dialogs.is_empty());
        assert_eq!(app.startup_view, StartupView::About);

        app.handle_cursor_moved(about_back_point)
            .expect("move over About Back");
        app.handle_mouse_button(ElementState::Pressed)
            .expect("press About Back");
        app.handle_mouse_button(ElementState::Released)
            .expect("leave About");
        assert_eq!(app.startup_view, StartupView::MainMenu);
        settle_startup_fade(&mut app);

        click_main_button(&mut app, 5);
        assert!(app.take_exit_request(), "Exit button requests shutdown");
        reset_cached_app_paths();
    }

    #[test]
    fn participant_context_helpers_preserve_raw_indices_and_lazy_scan_rules() {
        let _lock = env_lock().lock();
        let install = tempdir().expect("install root");
        let install_root = install.path();
        fs::create_dir_all(install_root.join("planet")).expect("create planet directory");
        fs::write(install_root.join("planet/System.c4g"), b"").expect("create system group marker");
        let user_data = tempdir().expect("user data");
        let player_root = user_data.path().join("Players");
        let ada = player_root.join("Ada.c4p");
        let bob = player_root.join("Bob.c4p");
        let broken = player_root.join("Broken.c4p");
        fs::create_dir_all(&ada).expect("create Ada group");
        fs::create_dir_all(&bob).expect("create Bob group");
        fs::write(&broken, b"not a group").expect("create invalid C4P file");
        fs::write(player_root.join(".Hidden.c4p"), b"hidden").expect("create hidden C4P");
        fs::write(player_root.join("Notes.txt"), b"text").expect("create non-player file");
        let nested = player_root.join("Nested");
        fs::create_dir_all(&nested).expect("create nested directory");
        fs::write(nested.join("Deep.c4p"), b"nested").expect("create nested C4P");

        let _guard = EnvGuard::set(&[
            ("LC_INSTALL_ROOT", Some(install_root)),
            ("LC_USER_DATA_DIR", Some(user_data.path())),
        ]);
        let paths = AppPaths::discover().expect("discover app paths");
        let save_participants = |participants: String| {
            let mut config = Config::new();
            config.set_in(Some("General"), "PlayerPath", player_root.to_string_lossy());
            config.set_in(Some("General"), "Participants", participants);
            fs::create_dir_all(paths.config_file().parent().expect("config parent"))
                .expect("create config directory");
            config.save(paths.config_file()).expect("save config");
        };

        save_participants(format!(
            "{};{};{};{};{}",
            bob.display(),
            ada.display(),
            bob.display(),
            player_root.join("Missing.c4p").display(),
            player_root.join("Notes.txt").display(),
        ));
        update_startup_participant_config(&paths, |_| {}).expect("validate participants");
        assert_eq!(
            startup_participant_references(&paths).expect("read validated participants"),
            vec![
                bob.to_string_lossy().into_owned(),
                ada.to_string_lossy().into_owned(),
            ],
            "validation keeps first spelling and config order while deduplicating"
        );

        save_participants(format!("{};;{}", ada.display(), bob.display()));
        let remove = startup_participant_remove_entries(&paths);
        assert_eq!(remove.len(), 2);
        assert_eq!(remove[0].text, "Ada");
        assert_eq!(remove[0].icon, ContextMenuIcon::Phase(9));
        assert_eq!(
            remove[0].tooltip.as_deref(),
            Some("Remove this player from participation list")
        );
        assert_eq!(
            remove[0].action,
            Some(AppContextMenuCommand::RemoveStartupParticipant(0))
        );
        assert_eq!(
            remove[1].action,
            Some(AppContextMenuCommand::RemoveStartupParticipant(2)),
            "empty raw segments must not renumber callback indices"
        );

        save_participants(format!("{};;{}", bob.display(), ada.display()));
        let removed = remove_startup_participant_config(&paths, 2)
            .expect("remove using fresh raw index")
            .expect("raw index still resolves");
        assert_eq!(removed, ada.to_string_lossy());
        assert_eq!(
            startup_participant_references(&paths).expect("read after removal"),
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
        assert_eq!(names, vec!["Bob", "Broken"]);
        for entry in &add {
            assert_eq!(entry.icon, ContextMenuIcon::Phase(9));
            assert_eq!(
                entry.tooltip.as_deref(),
                Some("Let this player join in next game")
            );
            assert!(matches!(
                entry.action,
                Some(AppContextMenuCommand::AddStartupParticipant(_))
            ));
        }
        assert!(
            add.iter().any(|entry| entry.text == "Broken"),
            "Add scans filenames without opening or parsing C4P groups"
        );
        assert!(!add.iter().any(|entry| entry.text == "Deep"));

        let developer_players = install_root.join("build/DevPlayers");
        fs::create_dir_all(&developer_players).expect("create developer PlayerPath");
        fs::write(developer_players.join("Late.C4P"), b"not parsed").expect("create developer C4P");
        let mut config = Config::new();
        config.set_in(Some("General"), "PlayerPath", "DevPlayers");
        config.set_in(Some("General"), "Participants", "");
        config
            .save(paths.config_file())
            .expect("save relative config");
        let developer_add = startup_participant_add_entries(&paths);
        assert_eq!(developer_add.len(), 1);
        assert_eq!(developer_add[0].text, "Late");
        assert_eq!(
            developer_add[0].action,
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
        let install_root = Path::new(env!("CARGO_MANIFEST_DIR"))
            .parent()
            .and_then(Path::parent)
            .expect("repository root");
        let user_data = tempdir().expect("user data");
        let player_root = user_data.path().join("Players");
        let ada = player_root.join("Ada.c4p");
        let bob = player_root.join("Bob.c4p");
        fs::create_dir_all(&ada).expect("create Ada group");
        fs::write(
            ada.join("Player.txt"),
            "[Player]\nName=Ada\n\n[Preferences]\nColorDw=255\n",
        )
        .expect("write Ada core");
        let _guard = EnvGuard::set(&[
            ("LC_INSTALL_ROOT", Some(install_root)),
            ("LC_USER_DATA_DIR", Some(user_data.path())),
        ]);
        let paths = AppPaths::discover().expect("discover app paths");
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
        fs::create_dir_all(paths.config_file().parent().expect("config parent"))
            .expect("create config directory");
        config.save(paths.config_file()).expect("save config");

        let mut app = GameApp::new(
            1280,
            720,
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
        )
        .expect("initialise app");
        assert_eq!(
            startup_participant_references(&paths).expect("constructor validation"),
            vec![ada.to_string_lossy().into_owned()]
        );
        wait_for_menu(&mut app);

        let participant_rect = app
            .main_menu_state
            .menu
            .participants_rect(&app.main_menu_state.participants_label);
        let label_point = PhysicalPosition::new(
            f64::from(participant_rect.x + participant_rect.w / 2),
            f64::from(participant_rect.y + participant_rect.h / 2),
        );
        app.handle_cursor_moved(PhysicalPosition::new(
            f64::from(participant_rect.x - 1),
            f64::from(participant_rect.y),
        ))
        .expect("move outside participant label");
        app.handle_right_mouse_button(ElementState::Pressed)
            .expect("ignore context outside participant label");
        assert!(app.context_menu.is_none());

        let open = |app: &mut GameApp| {
            app.handle_cursor_moved(label_point)
                .expect("move over participant label");
            app.handle_right_mouse_button(ElementState::Pressed)
                .expect("open participant context menu");
            let layout = app.context_menu.as_ref().expect("root menu").layout();
            assert_eq!(layout.panels.len(), 1);
            assert_eq!(layout.panels[0].rows.len(), 2);
            assert_eq!(layout.panels[0].selected, None);
        };
        let hover_root = |app: &mut GameApp, index: usize| {
            let row = app
                .context_menu
                .as_ref()
                .expect("root menu")
                .layout()
                .panels[0]
                .rows[index]
                .rect;
            app.handle_cursor_moved(PhysicalPosition::new(
                f64::from(row.x + 1),
                f64::from(row.y + 1),
            ))
            .expect("hover root row");
        };
        let activate_child = |app: &mut GameApp, index: usize| {
            let layout = app.context_menu.as_ref().expect("submenu").layout();
            let row = layout.panels[1].rows[index].rect;
            app.handle_cursor_moved(PhysicalPosition::new(
                f64::from(row.x + 1),
                f64::from(row.y + 1),
            ))
            .expect("hover child row");
            app.handle_mouse_button(ElementState::Pressed)
                .expect("activate child row on left-down");
            app.handle_mouse_button(ElementState::Released)
                .expect("release activation button");
        };

        open(&mut app);
        hover_root(&mut app, 1);
        assert!(
            !app.startup_element_tooltip_pending(),
            "captured popup motion must suppress the underlying startup tooltip"
        );
        app.close_context_menu_silently();
        assert!(!app.startup_element_tooltip_pending());

        open(&mut app);
        fs::create_dir_all(&bob).expect("create Bob after root popup opens");
        fs::write(
            bob.join("Player.txt"),
            "[Player]\nName=Bob\n\n[Preferences]\nColorDw=255\n",
        )
        .expect("write Bob core");
        hover_root(&mut app, 0);
        let add_layout = app.context_menu.as_ref().expect("Add submenu").layout();
        assert_eq!(add_layout.panels.len(), 2);
        assert_eq!(add_layout.panels[1].rows.len(), 1);
        activate_child(&mut app, 0);
        assert!(app.context_menu.is_none());
        assert_eq!(
            startup_participant_references(&paths).expect("read after Add"),
            vec![
                ada.to_string_lossy().into_owned(),
                bob.to_string_lossy().into_owned(),
            ]
        );
        assert_eq!(app.main_menu_state.participants_label, "Players: Ada, Bob");

        open(&mut app);
        hover_root(&mut app, 0);
        let empty = app
            .context_menu
            .as_ref()
            .expect("empty Add submenu")
            .layout();
        assert_eq!(empty.panels.len(), 2);
        assert!(empty.panels[1].rows.is_empty());
        assert_eq!(
            (empty.panels[1].bounds.w, empty.panels[1].bounds.h),
            (40, 7)
        );
        app.close_context_menu_silently();

        open(&mut app);
        hover_root(&mut app, 1);
        let remove_layout = app.context_menu.as_ref().expect("Remove submenu").layout();
        assert_eq!(remove_layout.panels.len(), 2);
        assert_eq!(remove_layout.panels[1].rows.len(), 2);
        activate_child(&mut app, 1);
        assert_eq!(
            startup_participant_references(&paths).expect("read after Remove"),
            vec![ada.to_string_lossy().into_owned()]
        );
        assert_eq!(app.main_menu_state.participants_label, "Players: Ada");
        reset_cached_app_paths();
    }

    #[test]
    fn player_context_menu_routes_recursively_without_generic_panes() {
        let _lock = env_lock().lock();
        let install_root = Path::new(env!("CARGO_MANIFEST_DIR"))
            .parent()
            .and_then(Path::parent)
            .expect("repository root");
        let user_data = tempdir().expect("user data");
        let player_root = user_data.path().join("Players");
        for name in ["Ada", "Bob"] {
            let group = player_root.join(format!("{name}.c4p"));
            fs::create_dir_all(&group).expect("create player group");
            fs::write(
                group.join("Player.txt"),
                format!("[Player]\nName={name}\n\n[Preferences]\nColorDw=255\n"),
            )
            .expect("write player core");
        }
        let _guard = EnvGuard::set(&[
            ("LC_INSTALL_ROOT", Some(install_root)),
            ("LC_USER_DATA_DIR", Some(user_data.path())),
        ]);
        let paths = AppPaths::discover().expect("discover app paths");
        let mut config = Config::new();
        config.set_in(Some("General"), "PlayerPath", player_root.to_string_lossy());
        config.set_in(
            Some("General"),
            "Participants",
            player_root.join("Ada.c4p").to_string_lossy(),
        );
        fs::create_dir_all(paths.config_file().parent().expect("config parent"))
            .expect("create config directory");
        config
            .save(paths.config_file())
            .expect("save player config");

        let mut app = GameApp::new(
            1280,
            720,
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
        )
        .expect("initialise app");
        wait_for_menu(&mut app);
        assert_eq!(app.startup_player_models.len(), 2);
        app.open_player_selection_dialog();

        let layout = clonk_frontend::startup_plrsel::plrsel_layout(1280, 720);
        let row_point = |index: usize| {
            PhysicalPosition::new(
                f64::from(layout.list_client.x + 2),
                f64::from(
                    layout.list_client.y
                        + layout.item_pitch * index as i32
                        + layout.item_height / 2,
                ),
            )
        };
        let open_on_row = |app: &mut GameApp, index: usize| {
            app.handle_cursor_moved(row_point(index))
                .expect("move over whole player row");
            app.handle_right_mouse_button(ElementState::Pressed)
                .expect("open row context menu");
        };

        let focus_before = app
            .startup_player_dialog
            .as_ref()
            .expect("player controller")
            .focused_control();
        open_on_row(&mut app, 1);
        let popup = app.context_menu.as_ref().expect("context menu");
        assert_eq!(popup.layout().panels.len(), 1);
        assert_eq!(popup.layout().panels[0].rows.len(), 2);
        assert_eq!(popup.layout().panels[0].selected, None);
        assert_eq!(
            app.startup_player_dialog
                .as_ref()
                .expect("player controller")
                .selected_index(),
            Some(1)
        );
        assert_eq!(
            app.startup_player_dialog
                .as_ref()
                .expect("player controller")
                .focused_control(),
            focus_before,
            "right-down selects the row without stealing keyboard focus"
        );

        let properties = popup.layout().panels[0].rows[0].rect;
        app.handle_cursor_moved(PhysicalPosition::new(
            f64::from(properties.x + 1),
            f64::from(properties.y + 1),
        ))
        .expect("hover Properties");
        app.handle_mouse_button(ElementState::Pressed)
            .expect("activate Properties on left-down");
        assert!(app.context_menu.is_none());
        assert!(matches!(
            app.startup_player_properties_dialog
                .as_ref()
                .map(|pending| pending.controller.mode()),
            Some(clonk_frontend::startup_plrproperties::PlayerPropertiesMode::Edit { index: 1 })
        ));
        assert!(app.message_dialogs.is_empty());
        assert!(app.status_text.is_empty());
        assert_eq!(
            app.context_menu_pointer_capture,
            Some(ContextMenuPointerButton::Left)
        );
        app.handle_mouse_button(ElementState::Released)
            .expect("swallow Properties activation release");
        assert_eq!(app.context_menu_pointer_capture, None);
        app.handle_key(VirtualKeyCode::Escape, ElementState::Pressed)
            .expect("close Properties");
        app.handle_key(VirtualKeyCode::Escape, ElementState::Released)
            .expect("release Properties close");

        open_on_row(&mut app, 1);
        let delete = app
            .context_menu
            .as_ref()
            .expect("context menu")
            .layout()
            .panels[0]
            .rows[1]
            .rect;
        app.handle_cursor_moved(PhysicalPosition::new(
            f64::from(delete.x + 1),
            f64::from(delete.y + 1),
        ))
        .expect("hover Delete");
        app.handle_mouse_button(ElementState::Pressed)
            .expect("activate Delete on left-down");
        assert!(app.context_menu.is_none());
        assert_eq!(app.message_dialogs.len(), 1);
        assert_eq!(app.message_dialogs[0].state.caption(), "Delete");
        assert_eq!(
            app.message_dialogs[0].state.message(),
            "Do you really want to delete player Bob?"
        );
        app.handle_mouse_button(ElementState::Released)
            .expect("swallow Delete activation release before modal");
        assert_eq!(app.message_dialogs.len(), 1);
        assert_eq!(app.context_menu_pointer_capture, None);
        app.finish_message_dialog(clonk_frontend::message_dialog::MessageDialogResult::No)
            .expect("decline deletion");

        open_on_row(&mut app, 1);
        let slot = GamepadSlot::new(0);
        app.process_gamepad_event_batch([
            GamepadEvent::Direction {
                slot,
                button: ControlButton::Down,
                state: ElementState::Pressed,
            },
            GamepadEvent::Direction {
                slot,
                button: ControlButton::Down,
                state: ElementState::Pressed,
            },
            GamepadEvent::GuiButton {
                slot,
                class: GuiButtonClass::Low,
                state: ElementState::Pressed,
            },
            GamepadEvent::Action {
                slot,
                action: GamepadActionType::Select,
                state: ElementState::Pressed,
            },
            GamepadEvent::Button {
                slot,
                button: LegacyGamepadButton::new(0),
                state: ElementState::Pressed,
            },
        ])
        .expect("activate Delete through one controller batch");
        assert!(app.context_menu.is_none());
        assert_eq!(app.message_dialogs.len(), 1);
        app.finish_message_dialog(clonk_frontend::message_dialog::MessageDialogResult::No)
            .expect("decline gamepad deletion");
        app.process_gamepad_event_batch([
            GamepadEvent::GuiButton {
                slot,
                class: GuiButtonClass::Low,
                state: ElementState::Released,
            },
            GamepadEvent::Action {
                slot,
                action: GamepadActionType::Select,
                state: ElementState::Released,
            },
            GamepadEvent::Button {
                slot,
                button: LegacyGamepadButton::new(0),
                state: ElementState::Released,
            },
        ])
        .expect("release captured controller batch");

        open_on_row(&mut app, 1);
        app.handle_cursor_moved(row_point(0))
            .expect("move outside popup to first row");
        app.handle_right_mouse_button(ElementState::Pressed)
            .expect("outside right-down closes and passes through");
        assert_eq!(
            app.startup_player_dialog
                .as_ref()
                .expect("player controller")
                .selected_index(),
            Some(0)
        );
        assert!(
            app.context_menu.is_some(),
            "same down opens the first row popup"
        );

        let mut with_context = vec![0_u8; 1280 * 720 * 4];
        assert!(app.render(&mut with_context).expect("render popup"));
        app.handle_focus_lost()
            .expect("focus loss closes popup silently");
        assert!(app.context_menu.is_none());
        let mut without_context = vec![0_u8; 1280 * 720 * 4];
        assert!(app
            .render(&mut without_context)
            .expect("render after close"));
        assert_ne!(
            with_context, without_context,
            "closed popup must not ghost from cache"
        );
        let stable = without_context.clone();
        assert!(!app
            .render(&mut without_context)
            .expect("replay clean frame"));
        assert_eq!(without_context, stable);

        open_on_row(&mut app, 1);
        app.resize(1024, 640).expect("resize closes popup");
        assert!(app.context_menu.is_none());
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
            .expect("push modal");
        }
        assert_eq!(app.message_dialogs.len(), 2);
        assert_eq!(app.message_dialogs[1].state.caption(), "Second");

        app.handle_key(VirtualKeyCode::Escape, ElementState::Pressed)
            .expect("close top");
        app.handle_key(VirtualKeyCode::Escape, ElementState::Released)
            .expect("swallow top release");
        assert_eq!(app.message_dialogs.len(), 1);
        assert_eq!(app.message_dialogs[0].state.caption(), "First");
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
        .expect("push modal");
        app.handle_key(VirtualKeyCode::Return, ElementState::Pressed)
            .expect("press modal button");
        app.handle_focus_lost().expect("lose focus");
        app.handle_key(VirtualKeyCode::Return, ElementState::Released)
            .expect("release after refocus");
        assert_eq!(
            app.message_dialogs.len(),
            1,
            "a release missing its pre-focus-loss press must not activate"
        );

        app.handle_key(VirtualKeyCode::Escape, ElementState::Pressed)
            .expect("dismiss modal");
        assert!(app
            .message_dialog_consumed_keys
            .contains(&VirtualKeyCode::Escape));
        app.handle_focus_lost().expect("lose focus after dismissal");
        assert!(app.message_dialog_consumed_keys.is_empty());
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
        .expect("push modal");
        let source = |cluster, event| SourcedGamepadEvent {
            gamepad: 0,
            cluster,
            event,
        };
        app.process_sourced_gamepad_event_batch(
            [source(
                30,
                GamepadEvent::GuiButton {
                    slot,
                    class: GuiButtonClass::Low,
                    state: ElementState::Pressed,
                },
            )],
            true,
        )
        .expect("press primary gamepad button");

        app.process_sourced_gamepad_event_batch([source(31, GamepadEvent::Clear { slot })], true)
            .expect("disconnect/reset gamepad");
        assert_eq!(app.message_dialogs.len(), 1);

        app.process_sourced_gamepad_event_batch(
            [
                source(
                    32,
                    GamepadEvent::GuiButton {
                        slot,
                        class: GuiButtonClass::Low,
                        state: ElementState::Released,
                    },
                ),
                source(
                    32,
                    GamepadEvent::Action {
                        slot,
                        action: GamepadActionType::Select,
                        state: ElementState::Released,
                    },
                ),
            ],
            true,
        )
        .expect("release after standalone Clear is a fresh physical cluster");
        assert_eq!(
            app.message_dialogs.len(),
            1,
            "Clear cancels the pressed state before the fresh release cluster"
        );

        app.handle_key(VirtualKeyCode::Escape, ElementState::Pressed)
            .expect("dismiss by keyboard");
        app.handle_key(VirtualKeyCode::Escape, ElementState::Released)
            .expect("release dismiss key");
        app.process_gamepad_event_batch([GamepadEvent::GuiButton {
            slot,
            class: GuiButtonClass::High,
            state: ElementState::Pressed,
        }])
        .expect("next controller input");
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
        app.local_controls.initialize(LocalControlInit {
            owner: app.local_owner,
            preferred_set: 5,
            prefers_mouse: false,
            gamepads_enabled: true,
            replay: false,
            disable_mouse: false,
        });

        app.process_gamepad_event_batch([GamepadEvent::Button {
            slot: GamepadSlot::new(1),
            button: LegacyGamepadButton::new(0),
            state: ElementState::Pressed,
        }])
        .expect("press configured gamepad button");

        assert!(
            app.ingame_menu.is_some(),
            "Button10 must dispatch PlayerMenu to the control-set 5 owner"
        );
    }

    #[test]
    fn modal_message_dialog_keeps_running_simulation_and_clock_alive() {
        let mut app = new_menu_app(640, 480);
        app.start_sandbox_scenario(FrontendScenario::fallback())
            .expect("start sandbox");
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
        .expect("push modal");

        app.update().expect("modal update");
        assert!(
            app.sec1_timer().expect("modal clock pulse"),
            "modal loop must keep the game clock alive"
        );
        assert_eq!(app.engine.frame(), frame + 1);
        assert_eq!(app.engine.game_time(), game_time + 1);
    }

    #[test]
    fn message_dialog_malformed_specific_assets_fail_before_modal_mutation() {
        let mut app = new_menu_app(640, 480);
        Arc::get_mut(&mut app.assets)
            .expect("frontend assets are app-owned")
            .startup_dialog_images
            .insert(
                "GUIIcons.png".to_string(),
                ImageData::new(1, 1, vec![255, 255, 255, 255]),
            );
        let version_before = app.menu_render_version;
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
        assert!(matches!(
            error,
            EngineError::ClassicMenuParityBoundary { ref detail }
                if detail.contains("C4GUI::MessageDialog")
                    && detail.contains("GUIIcons.png")
        ));
        assert!(app.message_dialogs.is_empty());
        assert_eq!(app.menu_render_version, version_before);
    }

    #[test]
    fn menu_input_invalidates_cached_frame() {
        clonk_logging::init();

        let mut app = new_real_classic_menu_app(320, 200);
        let mut frame = vec![0u8; 320 * 200 * 4];
        app.render(&mut frame).expect("render");
        let cached_version = app
            .menu_frame_cache
            .as_ref()
            .expect("cache populated")
            .version;
        app.handle_key(VirtualKeyCode::Down, ElementState::Pressed)
            .expect("key input");
        assert_ne!(
            app.menu_render_version, cached_version,
            "input events must invalidate the cached menu frame"
        );
    }

    #[test]
    fn menu_backdrop_restore_matches_full_recomposition() {
        clonk_logging::init();

        let mut app = new_real_classic_menu_app(320, 200);
        let len = 320 * 200 * 4;
        let mut first = vec![0u8; len];
        app.render(&mut first).expect("cold render");

        // A recomposition that restores the cached static backdrop...
        app.mark_menu_dirty();
        let mut restored = vec![0u8; len];
        app.render(&mut restored).expect("backdrop-restored render");

        // ...must match one composed from scratch.
        app.mark_menu_dirty();
        app.menu_backdrop_cache = StartupBackdropCache::default();
        let mut recomposed = vec![0u8; len];
        app.render(&mut recomposed).expect("full recomposition");

        assert_eq!(
            restored, recomposed,
            "backdrop restore must be pixel-identical to a full recomposition"
        );
        assert_eq!(
            first, restored,
            "unchanged menu state must keep rendering identical frames"
        );
    }

    #[test]
    fn menu_resize_renders_at_new_dimensions() {
        clonk_logging::init();

        let mut app = new_real_classic_menu_app(320, 200);
        let mut frame = vec![0u8; 320 * 200 * 4];
        app.render(&mut frame).expect("render");
        app.resize(400, 300).expect("resize");
        let mut larger = vec![0u8; 400 * 300 * 4];
        app.render(&mut larger).expect("render after resize");
        let cache = app.menu_frame_cache.as_ref().expect("cache after resize");
        assert_eq!(
            (cache.width, cache.height),
            (400, 300),
            "cache must track the resized surface"
        );
    }

    #[inline(never)]
    fn boxed_running_sandbox_app() -> Box<GameApp> {
        Box::new(new_running_sandbox_app())
    }

    #[inline(never)]
    fn boxed_classic_running_sandbox_app() -> Box<GameApp> {
        Box::new(new_classic_running_sandbox_app())
    }

    #[test]
    fn l143_default_z_dialog_order_tracks_show_raise_and_close() {
        let mut app = new_game_over_keyboard_app();
        assert_eq!(
            app.runtime_default_dialog_order_snapshot(),
            vec![RuntimeDefaultDialog::GameOver]
        );

        app.toggle_network_chart();
        configure_runtime_network_role(&mut app, RuntimeNetworkRole::Host);
        app.toggle_runtime_client_list().expect("open client list");
        app.external_irc_dialog_visible = true;
        app.show_or_raise_runtime_default_dialog(RuntimeDefaultDialog::ExternalIrc);
        assert_eq!(
            app.runtime_default_dialog_order_snapshot(),
            vec![
                RuntimeDefaultDialog::GameOver,
                RuntimeDefaultDialog::NetworkChart,
                RuntimeDefaultDialog::ClientList,
                RuntimeDefaultDialog::ExternalIrc,
            ]
        );
        assert!(app.runtime_client_list_above_game_over);
        assert!(app.runtime_top_default_dialog_is_exclusive());

        app.show_or_raise_runtime_default_dialog(RuntimeDefaultDialog::GameOver);
        assert!(app.runtime_default_dialog_is_top(RuntimeDefaultDialog::GameOver));
        assert!(!app.runtime_client_list_above_game_over);
        app.dismiss_game_over_dialog();
        assert!(app.runtime_default_dialog_is_top(RuntimeDefaultDialog::ExternalIrc));
        app.external_irc_dialog_visible = false;
        app.hide_runtime_default_dialog(RuntimeDefaultDialog::ExternalIrc);
        assert!(app.runtime_default_dialog_is_top(RuntimeDefaultDialog::ClientList));
        app.toggle_runtime_client_list().expect("close client list");
        assert!(app.runtime_default_dialog_is_top(RuntimeDefaultDialog::NetworkChart));
        app.toggle_network_chart();
        assert!(app.runtime_default_dialog_order_snapshot().is_empty());
    }

    #[test]
    fn l143_non_left_runtime_dialog_hits_swallow_without_raising() {
        let mut app = new_game_over_keyboard_app();
        app.resize(1280, 720).expect("resize pointer-routing fixture");
        let outside = GuiPoint::new(0.0, 0.0);
        assert!(!app.game_over_dialog_contains_point(outside));
        assert!(app.game_over_pointer_route_hit(outside));

        app.toggle_network_chart();
        let (width, height) = {
            let surface = app.graphics.surface();
            (surface.width(), surface.height())
        };
        let game_over_only = (0..height).step_by(4).find_map(|y| {
            (0..width)
                .step_by(4)
                .map(|x| GuiPoint::new(x as f32, y as f32))
                .find(|point| {
                    app.game_over_dialog_contains_point(*point)
                        && !app.network_chart_contains_point(*point)
                })
        })
        .expect("evaluation has an exposed point outside the chart");
        assert!(app.game_over_dialog_contains_point(game_over_only));
        assert!(!app.network_chart_contains_point(game_over_only));
        assert!(!app.game_over_pointer_route_hit(outside));
        let order = app.runtime_default_dialog_order_snapshot();
        app.running_pointer_position = Some(game_over_only);

        app.handle_mouse_wheel(MouseScrollDelta::LineDelta(0.0, 1.0), 1.0)
            .expect("lower game-over swallows an in-bounds wheel");
        app.handle_right_mouse_button(ElementState::Pressed)
            .expect("lower game-over swallows an in-bounds right press");
        app.handle_right_mouse_button(ElementState::Released)
            .expect("lower game-over swallows an in-bounds right release");
        app.handle_other_mouse_button(ElementState::Pressed)
            .expect("lower game-over swallows an in-bounds middle press");
        app.handle_other_mouse_button(ElementState::Released)
            .expect("lower game-over swallows an in-bounds middle release");
        assert_eq!(app.runtime_default_dialog_order_snapshot(), order);
        assert!(app.runtime_default_dialog_is_top(RuntimeDefaultDialog::NetworkChart));

        app.handle_cursor_moved(PhysicalPosition::new(
            f64::from(game_over_only.x),
            f64::from(game_over_only.y),
        ))
            .expect("move reaches the exposed lower game-over chassis");
        app.handle_mouse_button_classified(ElementState::Pressed, false)
            .expect("left press activates the exposed lower game-over dialog");
        assert!(app.runtime_default_dialog_is_top(RuntimeDefaultDialog::GameOver));
        app.handle_mouse_button_classified(ElementState::Released, false)
            .expect("release the game-over activation gesture");
    }

    #[test]
    fn running_chat_global_bindings_open_above_lower_messages_and_contexts() {
        let notice = || {
            clonk_frontend::message_dialog::MessageDialogState::regular_ok(
                "Lower notice",
                "Message",
                clonk_frontend::message_dialog::MessageDialogIcon::NOTIFY,
            )
        };

        let mut f2 = boxed_running_sandbox_app();
        f2.push_message_dialog(notice(), MessageDialogContinuation::None)
            .expect("push lower message");
        let layout = f2.top_message_dialog_layout().expect("message layout");
        let button = layout.buttons.first().expect("message button").rect;
        f2.handle_cursor_moved(PhysicalPosition::new(
            f64::from(button.x + button.w / 2),
            f64::from(button.y + button.h / 2),
        ))
        .expect("hover lower message button");
        f2.handle_mouse_button(ElementState::Pressed)
            .expect("capture lower message button");
        assert!(f2.message_dialogs[0].state.has_pointer_capture());
        assert_eq!(f2.message_dialog_pointer_capture_index, Some(0));
        f2.handle_key(VirtualKeyCode::F2, ElementState::Pressed)
            .expect("F2 opens chat above lower message");
        assert_eq!(f2.running_chat_text(), Some(""));
        assert_eq!(f2.message_dialogs.len(), 1);
        assert!(f2.message_dialogs[0].state.has_pointer_capture());
        f2.handle_mouse_button(ElementState::Released)
            .expect("release retained lower-message capture through chat");
        assert!(f2.message_dialogs.is_empty());
        assert!(f2.running_chat_active());

        let mut focus_loss = boxed_running_sandbox_app();
        focus_loss
            .push_message_dialog(notice(), MessageDialogContinuation::None)
            .expect("push lower message for focus-loss capture");
        let layout = focus_loss.top_message_dialog_layout().expect("message layout");
        let button = layout.buttons.first().expect("message button").rect;
        focus_loss
            .handle_cursor_moved(PhysicalPosition::new(
                f64::from(button.x + button.w / 2),
                f64::from(button.y + button.h / 2),
            ))
            .expect("hover lower focus-loss button");
        focus_loss
            .handle_mouse_button(ElementState::Pressed)
            .expect("capture lower focus-loss button");
        focus_loss
            .handle_key(VirtualKeyCode::F2, ElementState::Pressed)
            .expect("open chat over retained focus-loss capture");
        focus_loss
            .handle_focus_lost()
            .expect("focus loss clears captures below active chat");
        assert!(!focus_loss.message_dialogs[0].state.has_pointer_capture());
        assert_eq!(focus_loss.message_dialog_pointer_capture_index, None);
        assert!(!focus_loss.primary_pointer_left_down);
        focus_loss
            .handle_mouse_button(ElementState::Released)
            .expect("post-focus release cannot activate lower message");
        assert_eq!(focus_loss.message_dialogs.len(), 1);

        for (modifiers, expected) in [
            (ModifiersState::SHIFT, "/team "),
            (ModifiersState::ALT, "\""),
        ] {
            let mut app = boxed_running_sandbox_app();
            app.push_message_dialog(notice(), MessageDialogContinuation::None)
                .expect("push lower message");
            app.handle_modifiers_changed(modifiers)
                .expect("set chat-open modifier");
            app.handle_key(VirtualKeyCode::Return, ElementState::Pressed)
                .expect("modified Return falls through lower message to chat");
            assert_eq!(app.running_chat_text(), Some(expected));
            assert_eq!(app.message_dialogs.len(), 1);
        }

        let mut bare_return = boxed_running_sandbox_app();
        bare_return
            .push_message_dialog(notice(), MessageDialogContinuation::None)
            .expect("push lower message for bare Return");
        bare_return
            .handle_key(VirtualKeyCode::Return, ElementState::Pressed)
            .expect("bare Return opens chat above nonexclusive lower message");
        assert_eq!(bare_return.running_chat_text(), Some(""));
        assert_eq!(bare_return.message_dialogs.len(), 1);

        let lower_layout = bare_return
            .top_message_dialog_layout()
            .expect("lower message layout under chat");
        let lower_point = PhysicalPosition::new(
            f64::from(lower_layout.bounds.x + 5),
            f64::from(lower_layout.bounds.y + 5),
        );
        bare_return
            .handle_cursor_moved(lower_point)
            .expect("hover lower message outside compact chat");
        bare_return
            .handle_mouse_button(ElementState::Pressed)
            .expect("activate lower shared-screen message");
        bare_return
            .handle_mouse_button(ElementState::Released)
            .expect("release lower shared-screen message");
        assert!(!bare_return.running_chat_active());
        bare_return
            .handle_text_input('x')
            .expect("inactive chat ignores text while lower message owns keys");
        assert_eq!(bare_return.running_chat_text(), Some(""));

        let chat_layout = bare_return.game_option_input_layout().expect("chat layout");
        let chat_point = PhysicalPosition::new(
            f64::from(chat_layout.edit.x + chat_layout.edit.w / 2),
            f64::from(chat_layout.edit.y + chat_layout.edit.h / 2),
        );
        bare_return
            .handle_cursor_moved(chat_point)
            .expect("hover chat above lower message");
        bare_return
            .handle_mouse_button(ElementState::Pressed)
            .expect("reactivate compact chat");
        bare_return
            .handle_mouse_button(ElementState::Released)
            .expect("release compact chat click");
        assert!(bare_return.running_chat_active());
        bare_return
            .handle_text_input('x')
            .expect("reactivated chat accepts text");
        assert_eq!(bare_return.running_chat_text(), Some("x"));

        let mut inactive_return = boxed_running_sandbox_app();
        inactive_return.start_running_chat(RunningChatMode::All);
        inactive_return
            .push_message_dialog(notice(), MessageDialogContinuation::None)
            .expect("push message below visible chat for active-key routing");
        let lower_layout = inactive_return
            .top_message_dialog_layout()
            .expect("inactive-key lower message layout");
        inactive_return
            .handle_cursor_moved(PhysicalPosition::new(
                f64::from(lower_layout.bounds.x + 5),
                f64::from(lower_layout.bounds.y + 5),
            ))
            .expect("hover lower message for active-key routing");
        inactive_return
            .handle_mouse_button(ElementState::Pressed)
            .expect("activate lower message for Return routing");
        inactive_return
            .handle_mouse_button(ElementState::Released)
            .expect("release lower-message activation click");
        inactive_return
            .handle_key(VirtualKeyCode::Return, ElementState::Pressed)
            .expect("active lower message owns Return down");
        assert_eq!(inactive_return.message_dialogs.len(), 1);
        assert!(!inactive_return.running_chat_active());
        inactive_return
            .handle_key(VirtualKeyCode::Return, ElementState::Released)
            .expect("active lower message owns Return up");
        assert!(inactive_return.message_dialogs.is_empty());
        assert!(inactive_return.running_chat_active());

        let mut held_drag = boxed_running_sandbox_app();
        held_drag.start_running_chat(RunningChatMode::All);
        held_drag
            .push_message_dialog(notice(), MessageDialogContinuation::None)
            .expect("push message below chat for held-pointer activation");
        let lower_layout = held_drag
            .top_message_dialog_layout()
            .expect("held-pointer lower message layout");
        let lower_button = lower_layout.buttons.first().expect("lower OK button").rect;
        held_drag
            .handle_cursor_moved(PhysicalPosition::new(
                f64::from(lower_button.x + lower_button.w / 2),
                f64::from(lower_button.y + lower_button.h / 2),
            ))
            .expect("hover lower button for held-pointer activation");
        held_drag
            .handle_mouse_button(ElementState::Pressed)
            .expect("press lower button while chat is visible");
        assert!(!held_drag.running_chat_active());
        let chat_layout = held_drag.game_option_input_layout().expect("held chat layout");
        held_drag
            .handle_cursor_moved(PhysicalPosition::new(
                f64::from(chat_layout.edit.x + chat_layout.edit.w / 2),
                f64::from(chat_layout.edit.y + chat_layout.edit.h / 2),
            ))
            .expect("held left movement activates the hit chat dialog");
        assert!(held_drag.running_chat_active());
        held_drag
            .handle_cursor_moved(PhysicalPosition::new(
                f64::from(lower_button.x + lower_button.w / 2),
                f64::from(lower_button.y + lower_button.h / 2),
            ))
            .expect("active chat retains held routing outside its bounds");
        held_drag
            .handle_mouse_button(ElementState::Released)
            .expect("lower button cannot re-arm after chat activation");
        assert_eq!(held_drag.message_dialogs.len(), 1);

        let mut label_drag = boxed_running_sandbox_app();
        label_drag.start_running_chat(RunningChatMode::All);
        label_drag
            .push_message_dialog(notice(), MessageDialogContinuation::None)
            .expect("push lower message for noncapturing chat-label drag");
        let chat_layout = label_drag.game_option_input_layout().expect("label chat layout");
        let label_point = PhysicalPosition::new(
            f64::from(chat_layout.message.x + chat_layout.message.w / 2),
            f64::from(chat_layout.message.y + chat_layout.message.h / 2),
        );
        let message_layout = label_drag
            .top_message_dialog_layout()
            .expect("label-drag lower message layout");
        let lower_point = PhysicalPosition::new(
            f64::from(message_layout.bounds.x + 5),
            f64::from(message_layout.bounds.y + 5),
        );
        label_drag
            .handle_cursor_moved(label_point)
            .expect("hover the inert chat label");
        label_drag
            .handle_mouse_button(ElementState::Pressed)
            .expect("press the inert chat label");
        assert_eq!(label_drag.game_option_input_pointer_capture, None);
        assert!(label_drag.primary_pointer_left_down);
        label_drag
            .handle_cursor_moved(lower_point)
            .expect("held label drag activates the hit lower message");
        assert!(!label_drag.running_chat_active());
        assert_eq!(label_drag.active_message_dialog_index(), Some(0));
        label_drag
            .handle_mouse_button(ElementState::Released)
            .expect("release the noncapturing label drag");
        assert!(!label_drag.primary_pointer_left_down);

        let mut touch_lower = boxed_running_sandbox_app();
        touch_lower.start_running_chat(RunningChatMode::All);
        touch_lower
            .push_message_dialog(notice(), MessageDialogContinuation::None)
            .expect("push lower message for shared touch routing");
        let message_layout = touch_lower
            .top_message_dialog_layout()
            .expect("touch lower message layout");
        let lower_touch = GuiPoint::new(
            (message_layout.bounds.x + 5) as f32,
            (message_layout.bounds.y + 5) as f32,
        );
        touch_lower
            .handle_touch(TouchPhase::Started, lower_touch)
            .expect("touch starts on the exposed lower message");
        assert!(!touch_lower.running_chat_active());
        assert_eq!(touch_lower.active_message_dialog_index(), Some(0));
        touch_lower
            .handle_touch(TouchPhase::Ended, lower_touch)
            .expect("touch ends on the lower message");

        let mut release_hit = boxed_running_sandbox_app();
        release_hit.start_running_chat(RunningChatMode::All);
        release_hit
            .push_message_dialog(
                notice().with_checkbox("&Remember", false),
                MessageDialogContinuation::None,
            )
            .expect("push checkbox message below captured chat edit");
        let message_layout = release_hit
            .top_message_dialog_layout()
            .expect("checkbox message layout");
        let checkbox = message_layout
            .checkbox
            .as_ref()
            .expect("checkbox layout")
            .square;
        let checkbox_point = PhysicalPosition::new(
            f64::from(checkbox.x + checkbox.w / 2),
            f64::from(checkbox.y + checkbox.h / 2),
        );
        let chat_layout = release_hit.game_option_input_layout().expect("edit chat layout");
        let edit_point = PhysicalPosition::new(
            f64::from(chat_layout.edit.x + 5),
            f64::from(chat_layout.edit.y + chat_layout.edit.h / 2),
        );
        release_hit
            .handle_cursor_moved(edit_point)
            .expect("hover chat edit");
        release_hit
            .handle_mouse_button(ElementState::Pressed)
            .expect("chat edit installs pDragElement");
        assert_eq!(
            release_hit.game_option_input_pointer_capture,
            Some(ContextMenuPointerButton::Left),
        );
        release_hit
            .handle_cursor_moved(checkbox_point)
            .expect("chat edit capture retains held motion over checkbox");
        release_hit
            .handle_mouse_button(ElementState::Released)
            .expect("release clears chat capture before checkbox hit-testing");
        assert_eq!(release_hit.game_option_input_pointer_capture, None);
        assert_eq!(
            release_hit.message_dialogs[0].state.checkbox_checked(),
            Some(true),
        );

        let mut close_active_chat = boxed_running_sandbox_app();
        close_active_chat.start_running_chat(RunningChatMode::All);
        close_active_chat
            .push_message_dialog(notice(), MessageDialogContinuation::None)
            .expect("push lower message for active-chat close cleanup");
        let message_layout = close_active_chat
            .top_message_dialog_layout()
            .expect("close-cleanup message layout");
        let button = message_layout.buttons.first().expect("close-cleanup button").rect;
        close_active_chat
            .handle_cursor_moved(PhysicalPosition::new(
                f64::from(button.x + button.w / 2),
                f64::from(button.y + button.h / 2),
            ))
            .expect("hover cleanup button");
        close_active_chat
            .handle_mouse_button(ElementState::Pressed)
            .expect("capture cleanup button");
        let chat_layout = close_active_chat
            .game_option_input_layout()
            .expect("close-cleanup chat layout");
        close_active_chat
            .handle_cursor_moved(PhysicalPosition::new(
                f64::from(chat_layout.edit.x + 5),
                f64::from(chat_layout.edit.y + chat_layout.edit.h / 2),
            ))
            .expect("held move activates chat above retained capture");
        assert!(close_active_chat.running_chat_active());
        assert_eq!(close_active_chat.message_dialog_pointer_capture_index, Some(0));
        close_active_chat
            .handle_key(VirtualKeyCode::Escape, ElementState::Pressed)
            .expect("closing active chat releases all mouse elements");
        assert!(close_active_chat.running_chat.is_none());
        assert_eq!(close_active_chat.message_dialog_pointer_capture_index, None);
        assert!(!close_active_chat.message_dialogs[0]
            .state
            .has_pointer_capture());

        let mut stacked_active = boxed_running_sandbox_app();
        stacked_active.start_running_chat(RunningChatMode::All);
        stacked_active
            .push_message_dialog(notice(), MessageDialogContinuation::None)
            .expect("push first lower message");
        let first_layout = stacked_active
            .top_message_dialog_layout()
            .expect("first lower message layout");
        stacked_active
            .handle_cursor_moved(PhysicalPosition::new(
                f64::from(first_layout.bounds.x + 5),
                f64::from(first_layout.bounds.y + 5),
            ))
            .expect("hover first lower message");
        stacked_active
            .handle_mouse_button(ElementState::Pressed)
            .expect("activate first lower message");
        stacked_active
            .handle_mouse_button(ElementState::Released)
            .expect("release first lower activation");
        assert_eq!(stacked_active.active_message_dialog_index(), Some(0));

        let vote = || {
            clonk_frontend::message_dialog::MessageDialogState::new(
                "Vote?",
                "Voting",
                clonk_frontend::message_dialog::MessageDialogButtons::YES_NO,
                clonk_frontend::message_dialog::MessageDialogIcon::CONFIRM,
                clonk_frontend::message_dialog::MessageDialogSize::Regular,
                true,
            )
        };
        let small_vote = || {
            clonk_frontend::message_dialog::MessageDialogState::new(
                "Vote?",
                "Voting",
                clonk_frontend::message_dialog::MessageDialogButtons::YES_NO,
                clonk_frontend::message_dialog::MessageDialogIcon::CONFIRM,
                clonk_frontend::message_dialog::MessageDialogSize::Small,
                true,
            )
        };
        let small_notice = || {
            clonk_frontend::message_dialog::MessageDialogState::new(
                "Top notice",
                "Message",
                clonk_frontend::message_dialog::MessageDialogButtons::OK,
                clonk_frontend::message_dialog::MessageDialogIcon::NOTIFY,
                clonk_frontend::message_dialog::MessageDialogSize::Small,
                false,
            )
        };
        stacked_active
            .push_message_dialog(vote(), MessageDialogContinuation::LeagueSurrender)
            .expect("insert second message below inactive chat");
        assert_eq!(stacked_active.active_message_dialog_index(), Some(0));
        stacked_active
            .handle_key(VirtualKeyCode::Return, ElementState::Pressed)
            .expect("previous lower active dialog owns Return down");
        stacked_active
            .handle_key(VirtualKeyCode::Return, ElementState::Released)
            .expect("previous lower active dialog owns Return up");
        assert_eq!(stacked_active.message_dialogs.len(), 1);
        assert!(matches!(
            stacked_active.message_dialogs[0].continuation,
            MessageDialogContinuation::LeagueSurrender
        ));
        assert!(stacked_active.running_chat_active());

        let mut stacked_capture = boxed_running_sandbox_app();
        stacked_capture.start_running_chat(RunningChatMode::All);
        stacked_capture
            .push_message_dialog(notice(), MessageDialogContinuation::None)
            .expect("push captured dialog A below chat");
        let layout = stacked_capture
            .top_message_dialog_layout()
            .expect("captured dialog A layout");
        let button = layout.buttons.first().expect("dialog A button").rect;
        let button_point = PhysicalPosition::new(
            f64::from(button.x + button.w / 2),
            f64::from(button.y + button.h / 2),
        );
        stacked_capture
            .handle_cursor_moved(button_point)
            .expect("hover dialog A button");
        stacked_capture
            .handle_mouse_button(ElementState::Pressed)
            .expect("dialog A acquires global drag capture");
        assert_eq!(stacked_capture.message_dialog_pointer_capture_index, Some(0));
        stacked_capture
            .push_message_dialog(small_vote(), MessageDialogContinuation::LeagueSurrender)
            .expect("insert dialog B above captured A but below chat");
        assert_eq!(stacked_capture.active_message_dialog_index(), Some(0));
        let small_layout = stacked_capture
            .top_message_dialog_layout()
            .expect("smaller dialog B layout");
        let button_gui_point = GuiPoint::new(button_point.x as f32, button_point.y as f32);
        assert!(GameApp::point_in_message_dialog_bounds(
            button_gui_point,
            &small_layout,
        ));
        let a_only_point = PhysicalPosition::new(
            f64::from(layout.bounds.x + 5),
            f64::from(layout.bounds.y + 5),
        );
        assert!(!GameApp::point_in_message_dialog_bounds(
            GuiPoint::new(a_only_point.x as f32, a_only_point.y as f32),
            &small_layout,
        ));

        stacked_capture
            .handle_mouse_button(ElementState::Released)
            .expect("release hit-tests B after clearing A's global capture");
        assert_eq!(stacked_capture.message_dialogs.len(), 2);
        assert_eq!(stacked_capture.active_message_dialog_index(), Some(0));
        assert_eq!(stacked_capture.message_dialog_pointer_capture_index, None);
        assert!(stacked_capture
            .message_dialogs
            .iter()
            .all(|dialog| !dialog.state.has_pointer_capture()));

        let mut exposed_lower = boxed_running_sandbox_app();
        exposed_lower
            .push_message_dialog(notice(), MessageDialogContinuation::None)
            .expect("push regular shared-screen dialog A");
        let regular_layout = exposed_lower
            .top_message_dialog_layout()
            .expect("regular dialog A layout");
        exposed_lower
            .push_message_dialog(small_vote(), MessageDialogContinuation::None)
            .expect("push smaller shared-screen dialog B");
        let small_layout = exposed_lower
            .top_message_dialog_layout()
            .expect("smaller dialog B layout");
        let close = regular_layout
            .close_button
            .expect("regular dialog A close button");
        let exposed_point = PhysicalPosition::new(
            f64::from(close.x + close.w / 2),
            f64::from(close.y + close.h / 2),
        );
        assert!(!GameApp::point_in_message_dialog_bounds(
            GuiPoint::new(exposed_point.x as f32, exposed_point.y as f32),
            &small_layout,
        ));
        exposed_lower
            .handle_cursor_moved(exposed_point)
            .expect("hover the exposed lower dialog A");
        exposed_lower
            .handle_mouse_button(ElementState::Pressed)
            .expect("left-down activates and captures exposed lower dialog A");
        assert_eq!(exposed_lower.active_message_dialog_index(), Some(0));
        assert_eq!(exposed_lower.message_dialog_pointer_capture_index, Some(0));
        let top_point = PhysicalPosition::new(
            f64::from(small_layout.bounds.x + small_layout.bounds.w / 2),
            f64::from(small_layout.bounds.y + small_layout.bounds.h / 2),
        );
        exposed_lower
            .handle_cursor_moved(top_point)
            .expect("held move into B activates it without transferring A capture");
        assert_eq!(exposed_lower.active_message_dialog_index(), Some(1));
        assert_eq!(exposed_lower.message_dialog_pointer_capture_index, Some(0));
        exposed_lower
            .handle_cursor_moved(exposed_point)
            .expect("active B blocks the lower A-only hit while capture remains");
        assert_eq!(exposed_lower.active_message_dialog_index(), Some(1));
        assert_eq!(exposed_lower.message_dialog_pointer_capture_index, Some(0));
        exposed_lower
            .handle_mouse_button(ElementState::Released)
            .expect("A-only release clears A capture without closing it");
        assert_eq!(exposed_lower.message_dialogs.len(), 2);
        assert_eq!(exposed_lower.message_dialog_pointer_capture_index, None);

        let mut inserted_capture = boxed_running_sandbox_app();
        inserted_capture
            .push_message_dialog(notice(), MessageDialogContinuation::None)
            .expect("push dialog A before an asynchronous insertion");
        let regular_layout = inserted_capture
            .top_message_dialog_layout()
            .expect("asynchronous dialog A layout");
        let close = regular_layout
            .close_button
            .expect("asynchronous dialog A close button");
        let close_point = PhysicalPosition::new(
            f64::from(close.x + close.w / 2),
            f64::from(close.y + close.h / 2),
        );
        inserted_capture
            .handle_cursor_moved(close_point)
            .expect("hover dialog A close before insertion");
        inserted_capture
            .handle_mouse_button(ElementState::Pressed)
            .expect("dialog A captures before insertion");
        inserted_capture
            .push_message_dialog(small_vote(), MessageDialogContinuation::LeagueSurrender)
            .expect("insert exclusive dialog B without releasing A capture");
        assert_eq!(inserted_capture.active_message_dialog_index(), Some(1));
        assert_eq!(inserted_capture.message_dialog_pointer_capture_index, Some(0));
        assert!(inserted_capture.message_dialogs[0]
            .state
            .has_pointer_capture());
        let small_layout = inserted_capture
            .top_message_dialog_layout()
            .expect("asynchronous dialog B layout");
        let top_point = PhysicalPosition::new(
            f64::from(small_layout.bounds.x + small_layout.bounds.w / 2),
            f64::from(small_layout.bounds.y + small_layout.bounds.h / 2),
        );
        inserted_capture
            .handle_cursor_moved(top_point)
            .expect("active B owns held motion after insertion");
        inserted_capture
            .handle_mouse_button(ElementState::Released)
            .expect("B hit clears the retained A capture");
        assert_eq!(inserted_capture.message_dialogs.len(), 2);
        assert_eq!(inserted_capture.message_dialog_pointer_capture_index, None);

        let exposed_point = PhysicalPosition::new(
            f64::from(regular_layout.bounds.x + 5),
            f64::from(regular_layout.bounds.y + 5),
        );
        assert!(!GameApp::point_in_message_dialog_bounds(
            GuiPoint::new(exposed_point.x as f32, exposed_point.y as f32),
            &small_layout,
        ));
        inserted_capture
            .handle_cursor_moved(exposed_point)
            .expect("hover A outside the smaller exclusive B");
        inserted_capture
            .handle_mouse_button(ElementState::Pressed)
            .expect("exclusive B still permits shared-screen A hit-testing");
        assert_eq!(inserted_capture.active_message_dialog_index(), Some(0));
        inserted_capture
            .handle_mouse_button(ElementState::Released)
            .expect("release the exposed A click");

        stacked_capture
            .remove_message_dialog_at(1)
            .expect("remove B to press A again");
        stacked_capture
            .handle_cursor_moved(button_point)
            .expect("hover A before a second gesture");
        stacked_capture
            .handle_mouse_button(ElementState::Pressed)
            .expect("dialog A reacquires capture");
        stacked_capture
            .push_message_dialog(small_vote(), MessageDialogContinuation::LeagueSurrender)
            .expect("insert B above A during the second drag");
        stacked_capture
            .handle_cursor_moved(button_point)
            .expect("captured A drags first, then overlapping B activates");
        assert_eq!(stacked_capture.message_dialog_pointer_capture_index, Some(0));
        assert_eq!(stacked_capture.active_message_dialog_index(), Some(1));
        stacked_capture
            .handle_cursor_moved(a_only_point)
            .expect("active B blocks a lower A-only hit while capture remains");
        assert_eq!(stacked_capture.message_dialog_pointer_capture_index, Some(0));
        assert_eq!(stacked_capture.active_message_dialog_index(), Some(1));
        stacked_capture
            .handle_mouse_button(ElementState::Released)
            .expect("A-only release clears A capture without reactivating its button");
        assert_eq!(stacked_capture.message_dialogs.len(), 2);
        assert_eq!(stacked_capture.active_message_dialog_index(), Some(1));
        assert_eq!(stacked_capture.message_dialog_pointer_capture_index, None);
        assert!(stacked_capture
            .message_dialogs
            .iter()
            .all(|dialog| !dialog.state.has_pointer_capture()));

        let mut vote_pointer = boxed_running_sandbox_app();
        vote_pointer
            .push_message_dialog(vote(), MessageDialogContinuation::LeagueSurrender)
            .expect("push exclusive vote for outside-pointer routing");
        vote_pointer.running_pointer_position = Some(GuiPoint::new(0.0, 0.0));
        assert!(!vote_pointer.handle_message_dialog_pointer_move(GuiPoint::new(0.0, 0.0)));
        assert!(
            !vote_pointer
                .handle_message_dialog_pointer_button(ElementState::Pressed)
                .expect("outside vote hit-test falls through to shared Screen scanning")
        );
        assert!(
            !vote_pointer
                .handle_message_dialog_pointer_button(ElementState::Released)
                .expect("outside vote release falls through to shared Screen scanning")
        );

        let mut vote_return = boxed_running_sandbox_app();
        vote_return
            .push_message_dialog(vote(), MessageDialogContinuation::LeagueSurrender)
            .expect("push exclusive vote for bare Return");
        vote_return
            .handle_key(VirtualKeyCode::Return, ElementState::Pressed)
            .expect("bare Return remains owned by exclusive vote");
        assert!(vote_return.running_chat.is_none());
        assert_eq!(vote_return.message_dialogs.len(), 1);
        vote_return
            .handle_key(VirtualKeyCode::Return, ElementState::Released)
            .expect("focused No rejects vote on Return release");
        assert!(vote_return.message_dialogs.is_empty());
        assert_eq!(vote_return.mode, AppMode::Running);

        for (key, modifiers) in [
            (VirtualKeyCode::Return, ModifiersState::CTRL),
            (VirtualKeyCode::Space, ModifiersState::CTRL),
            (VirtualKeyCode::Space, ModifiersState::SHIFT),
            (VirtualKeyCode::Escape, ModifiersState::CTRL),
            (
                VirtualKeyCode::Y,
                ModifiersState::CTRL | ModifiersState::ALT,
            ),
        ] {
            let mut app = boxed_running_sandbox_app();
            app.push_message_dialog(vote(), MessageDialogContinuation::LeagueSurrender)
                .expect("push vote for exact modifier routing");
            app.handle_modifiers_changed(modifiers)
                .expect("set nonmatching GUI modifiers");
            app.handle_key(key, ElementState::Pressed)
                .expect("nonmatching GUI key down is inert");
            app.handle_key(key, ElementState::Released)
                .expect("nonmatching GUI key up is inert");
            assert_eq!(app.message_dialogs.len(), 1);
            assert!(app.running_chat.is_none());
        }

        let mut unmatched_vote_hotkey = boxed_classic_running_sandbox_app();
        unmatched_vote_hotkey
            .push_message_dialog(vote(), MessageDialogContinuation::LeagueSurrender)
            .expect("push exclusive vote for unmatched Alt mnemonic");
        unmatched_vote_hotkey
            .handle_modifiers_changed(ModifiersState::ALT)
            .expect("hold Alt over vote");
        unmatched_vote_hotkey
            .handle_key(VirtualKeyCode::C, ElementState::Pressed)
            .expect("unmatched vote mnemonic falls through to global Alt+C");
        assert!(unmatched_vote_hotkey.external_irc_dialog_visible);
        unmatched_vote_hotkey
            .handle_key(VirtualKeyCode::C, ElementState::Released)
            .expect("global Alt+C release also falls through the vote");
        assert_eq!(unmatched_vote_hotkey.message_dialogs.len(), 1);

        let mut handled_message_hotkey = boxed_running_sandbox_app();
        handled_message_hotkey
            .push_message_dialog(
                vote().with_checkbox("&Don't display again", false),
                MessageDialogContinuation::LeagueSurrender,
            )
            .expect("push checkbox message for down-only mnemonic");
        handled_message_hotkey
            .handle_modifiers_changed(ModifiersState::ALT)
            .expect("hold Alt over checkbox mnemonic");
        assert!(handled_message_hotkey
            .handle_message_dialog_key(VirtualKeyCode::D, ElementState::Pressed)
            .expect("checkbox mnemonic down is handled"));
        assert_eq!(
            handled_message_hotkey.message_dialogs[0]
                .state
                .checkbox_checked(),
            Some(true)
        );
        assert!(!handled_message_hotkey
            .message_dialog_consumed_keys
            .contains(&VirtualKeyCode::D));
        assert!(!handled_message_hotkey
            .handle_message_dialog_key(VirtualKeyCode::D, ElementState::Released)
            .expect("mnemonic release is not owned by the dialog"));

        let mut changed_release = boxed_running_sandbox_app();
        changed_release
            .push_message_dialog(vote(), MessageDialogContinuation::LeagueSurrender)
            .expect("push vote for modifier-changed release");
        changed_release
            .handle_key(VirtualKeyCode::Return, ElementState::Pressed)
            .expect("bare Return presses focused No");
        changed_release
            .handle_modifiers_changed(ModifiersState::CTRL)
            .expect("change modifiers before Return up");
        changed_release
            .handle_key(VirtualKeyCode::Return, ElementState::Released)
            .expect("modified Return up does not match the bare button binding");
        assert_eq!(changed_release.message_dialogs.len(), 1);
        assert!(changed_release.running_chat.is_none());

        let mut exclusive_top_scope = boxed_running_sandbox_app();
        exclusive_top_scope
            .push_message_dialog(notice(), MessageDialogContinuation::None)
            .expect("push ordinary lower A");
        let lower_layout = exclusive_top_scope
            .top_message_dialog_layout()
            .expect("ordinary lower A layout");
        exclusive_top_scope
            .push_message_dialog(small_vote(), MessageDialogContinuation::LeagueSurrender)
            .expect("push smaller exclusive top B");
        let exposed = PhysicalPosition::new(
            f64::from(lower_layout.bounds.x + 5),
            f64::from(lower_layout.bounds.y + 5),
        );
        exclusive_top_scope
            .handle_cursor_moved(exposed)
            .expect("hover exposed ordinary A");
        exclusive_top_scope
            .handle_mouse_button(ElementState::Pressed)
            .expect("activate ordinary A under exclusive B");
        exclusive_top_scope
            .handle_mouse_button(ElementState::Released)
            .expect("release ordinary A activation");
        assert_eq!(exclusive_top_scope.active_message_dialog_index(), Some(0));
        exclusive_top_scope
            .handle_key(VirtualKeyCode::Return, ElementState::Pressed)
            .expect("top exclusive B supplies GUI scope to active A");
        exclusive_top_scope
            .handle_key(VirtualKeyCode::Return, ElementState::Released)
            .expect("active ordinary A accepts Return under B's GUI scope");
        assert_eq!(exclusive_top_scope.message_dialogs.len(), 1);
        assert!(matches!(
            exclusive_top_scope.message_dialogs[0].continuation,
            MessageDialogContinuation::LeagueSurrender
        ));
        assert!(exclusive_top_scope.running_chat.is_none());

        let mut nonexclusive_top_scope = boxed_running_sandbox_app();
        nonexclusive_top_scope
            .push_message_dialog(vote(), MessageDialogContinuation::LeagueSurrender)
            .expect("push exclusive lower A");
        let lower_layout = nonexclusive_top_scope
            .top_message_dialog_layout()
            .expect("exclusive lower A layout");
        nonexclusive_top_scope
            .push_message_dialog(small_notice(), MessageDialogContinuation::None)
            .expect("push smaller nonexclusive top B");
        let exposed = PhysicalPosition::new(
            f64::from(lower_layout.bounds.x + 5),
            f64::from(lower_layout.bounds.y + 5),
        );
        nonexclusive_top_scope
            .handle_cursor_moved(exposed)
            .expect("hover exposed lower vote A");
        nonexclusive_top_scope
            .handle_mouse_button(ElementState::Pressed)
            .expect("activate lower vote A");
        nonexclusive_top_scope
            .handle_mouse_button(ElementState::Released)
            .expect("release lower vote A activation");
        assert_eq!(nonexclusive_top_scope.active_message_dialog_index(), Some(0));
        nonexclusive_top_scope
            .handle_key(VirtualKeyCode::Return, ElementState::Pressed)
            .expect("nonexclusive top B leaves bare Return in global chat scope");
        assert_eq!(nonexclusive_top_scope.running_chat_text(), Some(""));
        assert_eq!(nonexclusive_top_scope.message_dialogs.len(), 2);

        for (key, modifiers, expected) in [
            (VirtualKeyCode::F2, ModifiersState::empty(), ""),
            (VirtualKeyCode::Return, ModifiersState::SHIFT, "/team "),
            (VirtualKeyCode::Return, ModifiersState::ALT, "\""),
        ] {
            let mut app = boxed_running_sandbox_app();
            app.push_message_dialog(vote(), MessageDialogContinuation::LeagueSurrender)
                .expect("push exclusive vote for global chat binding");
            app.handle_modifiers_changed(modifiers)
                .expect("set vote chat-open modifier");
            app.handle_key(key, ElementState::Pressed)
                .expect("unhandled global chat binding falls through exclusive vote");
            assert_eq!(app.running_chat_text(), Some(expected));
            assert_eq!(app.message_dialogs.len(), 1);
        }

        for (key, modifiers, expected) in [
            (VirtualKeyCode::F2, ModifiersState::empty(), ""),
            (VirtualKeyCode::Return, ModifiersState::SHIFT, "/team "),
            (VirtualKeyCode::Return, ModifiersState::ALT, "\""),
        ] {
            let mut app = boxed_running_sandbox_app();
            app.open_context_menu_at(
                vec![ContextMenuEntry::<AppContextMenuCommand>::new("Unrelated")],
                GuiPoint::new(20.0, 20.0),
            )
            .expect("open unrelated context");
            app.handle_modifiers_changed(modifiers)
                .expect("set context chat-open modifier");
            app.handle_key(key, ElementState::Pressed)
                .expect("global chat binding opens underneath unrelated context");
            assert_eq!(app.running_chat_text(), Some(expected));
            assert!(app.context_menu.is_some());
        }
    }

    #[test]
    fn running_chat_uses_compact_bottom_third_dialog_above_log_and_message_dialogs() {
        let mut app = new_classic_running_sandbox_app();
        install_message_fixture(&mut app);
        assert!(
            app.execute_message_control(message_control(
                MESSAGE_TYPE_NORMAL,
                7,
                -1,
                b"before chat",
                7,
            ))
            .displayed
        );
        let board_before = app.message_board_line();

        app.handle_key(VirtualKeyCode::Return, ElementState::Pressed)
            .expect("open running chat");
        let surface_width = app.graphics.surface().width() as i32;
        let surface_height = app.graphics.surface().height() as i32;
        let fonts = app.assets.clonk_fonts.clone().expect("classic fonts");
        let layout = app.game_option_input_layout().expect("chat layout");
        let controller = app.running_chat_controller().expect("chat controller");
        let edit_height = (fonts.text.line_height + 3).max(23);
        let width = surface_width * 4 / 5;
        let height = edit_height + 2;
        let label_width = fonts.text.measure("Chat:", true).0 + 4;

        assert!(controller.is_chat_layout());
        assert_eq!(controller.message(), "Chat:");
        assert_eq!(controller.caption(), "");
        assert_eq!(controller.icon(), InputDialogIcon::None);
        assert_eq!(
            controller.focused_control(),
            clonk_frontend::input_dialog::InputDialogControl::Edit
        );
        assert_eq!(layout.caption, None);
        assert_eq!(layout.close_button, None);
        assert_eq!(layout.bounds.w, width);
        assert_eq!(layout.bounds.h, height);
        assert_eq!(layout.bounds.x, (surface_width - width) / 2);
        assert_eq!(
            layout.bounds.y,
            (surface_height - height) / 2 + surface_height / 3
        );
        assert_eq!(layout.message.w, label_width);
        assert_eq!((layout.icon.w, layout.icon.h), (0, 0));
        assert_eq!((layout.ok_button.w, layout.ok_button.h), (0, 0));
        assert_eq!((layout.cancel_button.w, layout.cancel_button.h), (0, 0));

        for modifiers in [ModifiersState::SHIFT, ModifiersState::ALT] {
            app.handle_modifiers_changed(modifiers)
                .expect("hold modifier over the context-menu key");
            app.handle_key(VirtualKeyCode::Apps, ElementState::Pressed)
                .expect("modified Apps is not the exact context binding");
            app.handle_key(VirtualKeyCode::Apps, ElementState::Released)
                .expect("release modified Apps probe");
            assert!(app.context_menu.is_none());
        }
        app.handle_modifiers_changed(ModifiersState::empty())
            .expect("release context-menu modifier");

        app.handle_key(VirtualKeyCode::Apps, ElementState::Pressed)
            .expect("open context over empty chat");
        app.handle_key(VirtualKeyCode::Apps, ElementState::Released)
            .expect("release context-menu key");
        assert!(app.context_menu.is_some());
        app.handle_modifiers_changed(ModifiersState::SHIFT)
            .expect("hold Shift over empty chat context");
        app.handle_key(VirtualKeyCode::Return, ElementState::Pressed)
            .expect("global allies binding reopens empty chat through its context");
        app.handle_key(VirtualKeyCode::Return, ElementState::Released)
            .expect("release allies binding");
        assert!(app.context_menu.is_none());
        assert_eq!(app.running_chat_text(), Some("/team "));
        app.handle_modifiers_changed(ModifiersState::empty())
            .expect("release Shift");
        app.handle_key(VirtualKeyCode::Escape, ElementState::Pressed)
            .expect("close allies chat");

        app.handle_key(VirtualKeyCode::Return, ElementState::Pressed)
            .expect("reopen empty chat for context say binding");
        app.handle_key(VirtualKeyCode::Apps, ElementState::Pressed)
            .expect("open context for say binding");
        app.handle_key(VirtualKeyCode::Apps, ElementState::Released)
            .expect("release context-menu key");
        app.handle_modifiers_changed(ModifiersState::ALT)
            .expect("hold Alt over empty chat context");
        app.handle_key(VirtualKeyCode::Return, ElementState::Pressed)
            .expect("global say binding reopens empty chat through its context");
        app.handle_key(VirtualKeyCode::Return, ElementState::Released)
            .expect("release say binding");
        assert!(app.context_menu.is_none());
        assert_eq!(app.running_chat_text(), Some("\""));
        app.handle_modifiers_changed(ModifiersState::empty())
            .expect("release Alt");
        app.handle_key(VirtualKeyCode::Escape, ElementState::Pressed)
            .expect("close say chat");

        app.handle_key(VirtualKeyCode::Return, ElementState::Pressed)
            .expect("reopen empty chat for context F2 binding");
        app.handle_key(VirtualKeyCode::Apps, ElementState::Pressed)
            .expect("open context for F2 binding");
        app.handle_key(VirtualKeyCode::Apps, ElementState::Released)
            .expect("release context-menu key");
        app.handle_key(VirtualKeyCode::F2, ElementState::Pressed)
            .expect("global all-chat binding reopens empty chat through its context");
        assert!(app.context_menu.is_none());
        assert_eq!(app.running_chat_text(), Some(""));

        app.handle_key(VirtualKeyCode::Escape, ElementState::Pressed)
            .expect("Escape closes chat without sending");
        assert!(app.running_chat.is_none());
        app.handle_key(VirtualKeyCode::Return, ElementState::Pressed)
            .expect("reopen running chat");
        app.handle_modifiers_changed(ModifiersState::SHIFT)
            .expect("hold Shift");
        app.handle_key(VirtualKeyCode::Escape, ElementState::Pressed)
            .expect("Shift+Escape does not cancel the exact bare binding");
        assert_eq!(app.running_chat_text(), Some(""));
        app.handle_key(VirtualKeyCode::Return, ElementState::Pressed)
            .expect("Shift+Return replaces empty chat with allies mode");
        assert_eq!(app.running_chat_text(), Some("/team "));
        app.handle_modifiers_changed(ModifiersState::empty())
            .expect("release Shift");
        app.handle_key(VirtualKeyCode::Escape, ElementState::Pressed)
            .expect("close allies chat");
        app.handle_key(VirtualKeyCode::Return, ElementState::Pressed)
            .expect("reopen ordinary running chat");
        app.handle_modifiers_changed(ModifiersState::ALT)
            .expect("hold Alt");
        app.handle_key(VirtualKeyCode::Return, ElementState::Pressed)
            .expect("Alt+Return replaces empty chat with say mode");
        assert_eq!(app.running_chat_text(), Some("\""));
        app.handle_modifiers_changed(ModifiersState::empty())
            .expect("release Alt");
        app.handle_key(VirtualKeyCode::Escape, ElementState::Pressed)
            .expect("close say chat");
        app.handle_key(VirtualKeyCode::Return, ElementState::Pressed)
            .expect("reopen ordinary chat for editing");

        for character in "alpha beta".chars() {
            app.handle_text_input(character).expect("type chat text");
        }
        assert_eq!(app.running_chat_text(), Some("alpha beta"));
        app.process_gamepad_event_batch([
            GamepadEvent::GuiButton {
                slot: GamepadSlot::new(0),
                class: GuiButtonClass::High,
                state: ElementState::Pressed,
            },
            GamepadEvent::Action {
                slot: GamepadSlot::new(0),
                action: GamepadActionType::MenuToggle,
                state: ElementState::Pressed,
            },
            GamepadEvent::Button {
                slot: GamepadSlot::new(0),
                button: LegacyGamepadButton::new(8),
                state: ElementState::Pressed,
            },
        ])
        .expect("chat owns the raw gamepad Select cluster");
        assert!(app.ingame_menu.is_none());
        assert_eq!(app.running_chat_text(), Some("alpha beta"));
        let caret_before_alt_navigation = app
            .running_chat_controller()
            .expect("chat controller before Alt navigation probe")
            .caret();
        for modifiers in [
            ModifiersState::ALT,
            ModifiersState::CTRL | ModifiersState::ALT,
            ModifiersState::ALT | ModifiersState::SHIFT,
            ModifiersState::CTRL | ModifiersState::ALT | ModifiersState::SHIFT,
        ] {
            app.handle_modifiers_changed(modifiers)
                .expect("hold an Alt modifier mask over chat edit");
            for key in [VirtualKeyCode::Left, VirtualKeyCode::Back] {
                app.handle_key(key, ElementState::Pressed)
                    .expect("Alt navigation is not an Edit cursor binding");
                app.handle_key(key, ElementState::Released)
                    .expect("release Alt navigation probe");
            }
            assert_eq!(app.running_chat_text(), Some("alpha beta"));
            assert_eq!(
                app.running_chat_controller()
                    .expect("chat remains open after Alt navigation probe")
                    .caret(),
                caret_before_alt_navigation
            );
        }
        app.handle_modifiers_changed(ModifiersState::empty())
            .expect("release Alt navigation modifier");
        app.handle_modifiers_changed(ModifiersState::SHIFT)
            .expect("hold Shift over nonempty chat");
        app.handle_key(VirtualKeyCode::Return, ElementState::Pressed)
            .expect("Shift+Return leaves nonempty chat unchanged");
        assert_eq!(app.running_chat_text(), Some("alpha beta"));
        app.handle_modifiers_changed(ModifiersState::empty())
            .expect("release Shift");
        app.handle_modifiers_changed(ModifiersState::ALT)
            .expect("hold Alt over nonempty chat");
        app.handle_key(VirtualKeyCode::Return, ElementState::Pressed)
            .expect("Alt+Return leaves nonempty chat unchanged");
        assert_eq!(app.running_chat_text(), Some("alpha beta"));
        app.handle_modifiers_changed(ModifiersState::empty())
            .expect("release Alt");
        assert_eq!(
            app.message_board_line(),
            board_before,
            "the message board remains a fading log instead of echoing edit text"
        );

        app.pressed_engine_keys.insert(VirtualKeyCode::A);
        app.engine
            .player_mut(app.local_owner)
            .expect("local sandbox player")
            .control
            .pressed_coms = 1 << clonk_engine::COM_LEFT;
        app.handle_key(VirtualKeyCode::Apps, ElementState::Pressed)
            .expect("open chat context before lower message");
        app.handle_key(VirtualKeyCode::Apps, ElementState::Released)
            .expect("release chat context key");
        assert!(app.context_menu.is_some());
        app.push_message_dialog(
            clonk_frontend::message_dialog::MessageDialogState::regular_ok(
                "Notice",
                "The chat remains the higher input-z dialog.",
                clonk_frontend::message_dialog::MessageDialogIcon::NOTIFY,
            ),
            MessageDialogContinuation::None,
        )
        .expect("push message below chat");
        assert!(app.context_menu.is_some());
        assert!(app.pressed_engine_keys.contains(&VirtualKeyCode::A));
        assert_ne!(
            app.engine
                .player(app.local_owner)
                .expect("local sandbox player")
                .control
                .pressed_coms
                & (1 << clonk_engine::COM_LEFT),
            0
        );
        app.handle_key(VirtualKeyCode::Escape, ElementState::Pressed)
            .expect("close chat context above lower message");
        app.handle_key(VirtualKeyCode::Escape, ElementState::Released)
            .expect("release context close key");
        app.handle_text_input('!')
            .expect("chat receives text above message dialog");
        assert_eq!(app.running_chat_text(), Some("alpha beta!"));
        assert_eq!(app.message_dialogs.len(), 1);
        let mut frame = vec![0_u8; (surface_width * surface_height * 4) as usize];
        app.render(&mut frame)
            .expect("render chat above the lower message dialog");
        assert!(frame.iter().any(|byte| *byte != 0));

        app.handle_modifiers_changed(ModifiersState::CTRL | ModifiersState::SHIFT)
            .expect("hold Ctrl+Shift");
        app.handle_key(VirtualKeyCode::Left, ElementState::Pressed)
            .expect("select previous word in chat edit");
        assert!(
            app.running_chat_controller()
                .and_then(InputDialogController::selected_text)
                .is_some_and(|text| !text.is_empty())
        );
        app.handle_modifiers_changed(ModifiersState::empty())
            .expect("release modifiers");
        let keyboard_selection = app
            .running_chat_controller()
            .expect("chat controller after keyboard selection")
            .selection();

        let start = PhysicalPosition::new(
            f64::from(layout.edit.x + 5),
            f64::from(layout.edit.y + layout.edit.h / 2),
        );
        let end = PhysicalPosition::new(
            f64::from(layout.edit.x + 35),
            f64::from(layout.edit.y + layout.edit.h / 2),
        );
        app.handle_cursor_moved(start)
            .expect("point into chat above message dialog");
        app.handle_mouse_button(ElementState::Pressed)
            .expect("start chat selection");
        let selection_after_down = app
            .running_chat_controller()
            .expect("chat receives pointer down")
            .selection();
        assert!(selection_after_down.is_some_and(|(anchor, caret)| anchor == caret));
        assert_ne!(selection_after_down, keyboard_selection);
        app.handle_cursor_moved(end).expect("drag chat selection");
        app.handle_mouse_button(ElementState::Released)
            .expect("finish chat selection");
        assert!(
            app.running_chat_controller()
                .and_then(InputDialogController::selected_text)
                .is_some_and(|text| !text.is_empty())
        );
        app.handle_right_mouse_button(ElementState::Pressed)
            .expect("open chat edit context menu");
        assert!(app.context_menu.is_some());
        assert_eq!(app.message_dialogs.len(), 1);
        app.handle_right_mouse_button(ElementState::Released)
            .expect("release context-menu button");

        let text_before_context_key = app.running_chat_text().map(str::to_string);
        app.handle_key(VirtualKeyCode::Up, ElementState::Pressed)
            .expect("context menu outranks chat history");
        app.handle_key(VirtualKeyCode::Up, ElementState::Released)
            .expect("release context-menu navigation");
        assert!(app.game_option_input_consumed_keys.is_empty());
        assert_eq!(app.running_chat_text(), text_before_context_key.as_deref());
        assert_eq!(
            app.running_chat.as_ref().map(|chat| chat.history_index),
            Some(-1)
        );

        let caret_before_ctrl_left = app
            .running_chat_controller()
            .expect("chat remains under its context menu")
            .caret();
        app.handle_modifiers_changed(ModifiersState::CTRL)
            .expect("hold Ctrl over chat context menu");
        app.handle_key(VirtualKeyCode::Left, ElementState::Pressed)
            .expect("context makes the parent chat edit inactive");
        assert_eq!(
            app.running_chat_controller()
                .expect("chat remains open")
                .caret(),
            caret_before_ctrl_left
        );
        assert!(app.context_menu.is_some());
        app.handle_modifiers_changed(ModifiersState::empty())
            .expect("release Ctrl");

        app.handle_modifiers_changed(ModifiersState::ALT)
            .expect("hold Alt over chat context menu");
        app.handle_key(VirtualKeyCode::C, ElementState::Pressed)
            .expect("global IRC chord replaces compact chat with the standalone dialog");
        assert!(app.external_irc_dialog_visible);
        assert!(app.running_chat.is_none());
        assert!(app.context_menu.is_none());
        app.handle_key(VirtualKeyCode::C, ElementState::Released)
            .expect("consume global IRC chord release");
        app.handle_key(VirtualKeyCode::C, ElementState::Pressed)
            .expect("second global IRC chord closes the standalone dialog");
        app.handle_key(VirtualKeyCode::C, ElementState::Released)
            .expect("consume closing IRC chord release");
        assert!(!app.external_irc_dialog_visible);
        app.handle_modifiers_changed(ModifiersState::empty())
            .expect("release Alt");
        assert!(app.game_option_input_dialog.is_none());
        assert!(app.context_menu.is_none());
        assert_eq!(app.message_dialogs.len(), 1);
        assert_eq!(app.message_board_line(), board_before);
    }

    #[test]
    fn observer_menu_lists_players_and_live_previews_selection() {
        let mut app = new_state_only_running_sandbox_app();
        let first = app.local_owner;
        let first_info = app
            .engine
            .player(first)
            .expect("sandbox player")
            .player_info_id();
        let second = first + 1;
        let hidden = first + 2;
        let second_info = first_info + 10;
        let hidden_info = first_info + 20;
        app.engine
            .register_player(
                PlayerConfig::new(second, "Second visible")
                    .with_player_info_id(second_info),
            )
            .expect("register second visible observer target");
        app.engine
            .register_player(
                PlayerConfig::new(hidden, "Hidden target").with_player_info_id(hidden_info),
            )
            .expect("register invisible observer target");
        let info = |id, name: &[u8], flags| clonk_engine::ControlPlayerInfoEntry {
            id,
            name: LegacyCString::from_bytes(name.to_vec()).expect("valid player-info name"),
            flags,
            ..clonk_engine::ControlPlayerInfoEntry::default()
        };
        app.control_player_infos.replace_snapshot(
            hidden_info,
            [clonk_engine::PlayerInfoControlData {
                client_id: 0,
                players: vec![
                    info(first_info, b"Player", 0),
                    info(second_info, b"Second visible", 0),
                    info(
                        hidden_info,
                        b"Hidden target",
                        clonk_engine::PLAYER_INFO_FLAG_INVISIBLE,
                    ),
                ],
                ..clonk_engine::PlayerInfoControlData::default()
            }],
        );

        app.clear_physical_viewport_states();
        let observer = app.ownerless_physical_viewport_state();
        let physical_identity = observer.physical_identity;
        app.physical_viewports.push(observer);
        app.physical_viewports_authoritative = true;
        assert!(app.set_physical_film_view(first));

        let open_observer_menu = |app: &mut GameApp| {
            app.ingame_menu.replace(
                OWNER_NONE,
                IngameMenuState::main_menu(&MainMenuConditions {
                    has_player: false,
                    player_count: 3,
                    ..MainMenuConditions::default()
                }),
            );
            assert!(
                app.handle_menu_command(
                    OWNER_NONE,
                    ControlCommand::MenuEnter,
                    CommandKind::Press,
                )
                .expect("open observer target page")
            );
        };
        open_observer_menu(&mut app);

        let menu = app.ingame_menu.get(OWNER_NONE).expect("observer menu");
        assert_eq!(menu.page(), ingame_menu::MenuPage::Observer);
        assert_eq!(
            menu.items()
                .iter()
                .map(|item| (item.caption.as_str(), item.action.clone()))
                .collect::<Vec<_>>(),
            vec![
                ("free view", MenuAction::Observe(ObserverTarget::Free)),
                (
                    "Player",
                    MenuAction::Observe(ObserverTarget::Player(first)),
                ),
                (
                    "Second visible",
                    MenuAction::Observe(ObserverTarget::Player(second)),
                ),
            ]
        );
        assert_eq!(menu.selection(), 1, "current followed player is selected");
        assert!(menu.items().iter().all(|item| item.caption != "Hidden target"));

        assert!(
            app.handle_menu_command(
                OWNER_NONE,
                ControlCommand::MenuDown,
                CommandKind::Press,
            )
            .expect("moving selection previews the next player")
        );
        assert_eq!(app.physical_viewports[0].displayed_player, second);
        assert_eq!(app.film_view_player, Some(second));
        assert!(app.set_physical_film_view(first));
        assert_eq!(
            app.ingame_menu
                .get(OWNER_NONE)
                .map(IngameMenuState::selection),
            Some(2),
            "camera perturbation does not change the highlighted row"
        );
        assert!(
            app.handle_menu_command(
                OWNER_NONE,
                ControlCommand::MenuEnter,
                CommandKind::Press,
            )
            .expect("Enter dispatches the highlighted player target")
        );
        assert!(!app.ingame_menu.contains(OWNER_NONE));
        assert_eq!(app.physical_viewports[0].displayed_player, second);

        open_observer_menu(&mut app);
        assert!(
            app.handle_menu_command(
                OWNER_NONE,
                ControlCommand::MenuDown,
                CommandKind::Press,
            )
            .expect("last player wraps to free view")
        );
        assert_eq!(app.physical_viewports[0].displayed_player, OWNER_NONE);
        assert!(app.set_physical_film_view(first));
        assert!(
            app.handle_menu_command(
                OWNER_NONE,
                ControlCommand::MenuEnter,
                CommandKind::Press,
            )
            .expect("Enter dispatches free view through the same path")
        );
        assert_eq!(app.physical_viewports[0].displayed_player, OWNER_NONE);
        assert_eq!(app.film_view_player, Some(OWNER_NONE));
        assert_eq!(app.physical_viewports[0].physical_identity, physical_identity);
        assert!(app.physical_viewports[0].is_no_owner_viewport);
    }

    #[test]
    fn real_regicide_opens_initial_team_menu_and_hides_disabled_switch() {
        // Regicide's custom active Teams.txt leaves the initial user
        // teamless. C4Player::Execute opens C4MN_TeamSelection with both
        // ordered teams before the player's ScenarioInit can run
        // (C4Player.cpp:159-173,1762-1772; C4MainMenu.cpp:175-236).
        let mut app =
            real_installed_scenario_app("Knights.c4f/Regicide.c4s", "Regicide team chooser");
        wait_for_running(&mut app);

        assert!(
            !app.engine.team_configuration().allow_team_switch,
            "Regicide's parsed Teams.txt keeps mid-round switching disabled"
        );
        assert_eq!(
            app.engine
                .player(app.local_owner)
                .map(clonk_engine::Player::status),
            Some(PlayerStatus::TeamSelection)
        );
        let menu = app
            .ingame_menu
            .as_ref()
            .expect("team selection opens automatically");
        assert_eq!(menu.page(), ingame_menu::MenuPage::TeamSelection);
        assert_eq!(
            menu.items()
                .iter()
                .map(|item| item.action.clone())
                .collect::<Vec<_>>(),
            [MenuAction::SelectTeam(1), MenuAction::SelectTeam(2)]
        );

        let outcome = app
            .ingame_menu
            .as_mut()
            .expect("team menu remains open")
            .handle_command(ControlCommand::MenuEnter, CommandKind::Press)
            .expect("first team activates");
        app.execute_ingame_menu_outcome(outcome)
            .expect("team selection executes");

        let player = app
            .engine
            .player(app.local_owner)
            .expect("selected player remains registered");
        assert_eq!(player.status(), PlayerStatus::Active);
        assert_eq!(player.team(), Some(1));
        assert!(
            app.engine.crew_cursor(app.local_owner).is_some(),
            "Regicide selection must leave the player with usable crew"
        );
        assert!(app.ingame_menu.is_none());

        let owner = app.local_owner;
        app.activate_ingame_main_menu_for_player(owner)
            .expect("open post-selection main menu");
        assert!(!app
            .ingame_menu
            .as_ref()
            .expect("main menu")
            .items()
            .iter()
            .any(|item| item.action == MenuAction::ActivateTeamSelection));
    }

    #[test]
    fn secondary_local_player_controls_own_initial_team_menu() {
        // C4Player stores one C4MainMenu per player. LocalPlayerControl looks
        // up the keyboard-set owner, converts through that player's menu, and
        // TeamSel dispatches DoTeamSelection on the menu's Player
        // (pristine 9ffa0a5d src/C4Player.h:85;
        // src/C4Game.cpp:3572-3624; src/C4MainMenu.cpp:899-908).
        let mut app = new_synthetic_running_sandbox_app();
        let primary = app.local_owner;
        let primary_before = app
            .engine
            .player(primary)
            .map(|player| (player.status(), player.team()))
            .expect("primary local player");
        app.engine.set_teams(vec![
            clonk_engine::TeamInfo::new(1, "West", 0xff0000),
            clonk_engine::TeamInfo::new(2, "East", 0x0000ff),
        ]);
        let secondary = app
            .engine
            .join_player_for_team_selection(JoinPlayerConfig {
                name: "Secondary".to_string(),
                player_info_id: 0,
                score: 0,
                rounds: 0,
                rounds_won: 0,
                rounds_lost: 0,
                total_playing_time: 0,
                team: None,
                color_dw: 0x0000ff,
                pref_color: 0,
                pref_position: 0,
                crew: Vec::new(),
                control_style: false,
                auto_context_menu: false,
                startup_player_count: 2,
            })
            .expect("secondary waits for a team");
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
            prefers_mouse: false,
            gamepads_enabled: true,
            replay: false,
            disable_mouse: false,
        });
        app.handle_key(VirtualKeyCode::Z, ElementState::Pressed)
            .expect("primary holds left");

        app.open_initial_team_selection(secondary);
        assert_eq!(
            app.ingame_menu.as_ref().and_then(IngameMenuState::player),
            Some(secondary)
        );

        // Keyboard set 2 Key4 is Throw; an active C4MainMenu converts it to
        // MenuEnter and selects the first team.
        app.handle_key(VirtualKeyCode::Numpad4, ElementState::Pressed)
            .expect("secondary enters selected team");

        let secondary_player = app.engine.player(secondary).expect("secondary remains");
        assert_eq!(secondary_player.status(), PlayerStatus::Active);
        assert_eq!(secondary_player.team(), Some(1));
        assert!(
            app.engine.crew_cursor(secondary).is_some(),
            "team activation spawns the default native crew"
        );
        assert_eq!(
            app.engine
                .player(primary)
                .map(|player| (player.status(), player.team())),
            Some(primary_before),
            "secondary menu control must not mutate the primary player"
        );
        assert_ne!(
            app.engine
                .snapshot()
                .players
                .into_iter()
                .find(|player| player.id == primary)
                .expect("primary snapshot")
                .control
                .pressed_coms
                & (1 << clonk_engine::COM_LEFT),
            0,
            "closing the secondary menu must clear only secondary controls"
        );
        assert!(app.ingame_menu.is_none());
    }

    #[test]
    fn rules_menu_uses_engine_definition_description_as_tooltip() {
        let mut app = new_running_sandbox_app();
        let player = app.local_owner;
        let mut rule = Definition::from_script("IRUL", "Integrated Rule", "#strict 3\n")
            .expect("rule definition compiles");
        rule.set_category(C4D_RULE);
        rule.set_description(Some("Keep to the rule".to_string()));
        app.engine
            .register_definition(rule)
            .expect("rule definition registers");
        app.engine
            .spawn_object(clonk_engine::SpawnConfig::new("IRUL"))
            .expect("rule object spawns");
        app.snapshot = app.engine.snapshot();

        app.apply_ingame_menu_action_for_player(player, MenuAction::ActivateRules)
            .expect("open rules menu");
        let menu = app.ingame_menu.get(player).expect("rules menu opens");
        assert_eq!(menu.page(), ingame_menu::MenuPage::Rules);
        assert_eq!(
            menu.items()[0].info_caption.as_deref(),
            Some("Keep to the rule")
        );
    }

    #[test]
    fn player_menu_title_close_routes_submenu_back_and_main_closed() {
        // Dialog's Ico_Close calls C4Menu::TryClose on left-up. Submenus run
        // their ActivateMenu:Main close command; the Main page stays closed.
        // Every C4MainMenu::OnClosed queues one synchronized ClearPressed
        // (C4GuiDialogs.cpp:386-425; C4MainMenu.cpp:313-329).
        let mut app = new_classic_running_sandbox_app();
        let (manager, _event_tx, mut commands) = NetworkManager::test_stub_with_commands();
        app.network = Some(manager);
        let tick = app.local_control_submission_tick();
        app.open_ingame_menu().expect("open player menu");
        app.apply_ingame_menu_action(MenuAction::ActivateOptions)
            .expect("open Options submenu");

        let mut frame = vec![0_u8; 320 * 200 * 4];
        app.render(&mut frame).expect("establish local viewport");
        assert!(
            app.ingame_menu_gfx
                .as_ref()
                .is_some_and(|gfx| gfx.show_close_button),
            "the controlling mouse player's title renders its close button"
        );

        let close_rect = |app: &GameApp| {
            let player = app.local_owner;
            let area = app.graphics.viewport_rect(player).expect("local viewport");
            let fallback = app.assets.font_arc();
            let font = clonk_frontend::hud::HudFont::from_set(
                app.assets.clonk_fonts.as_deref(),
                fallback.as_ref(),
            );
            let gfx = IngameMenuGraphics {
                show_commands: app.display_flags.show_commands,
                show_close_button: true,
                ..IngameMenuGraphics::default()
            };
            app
                .ingame_menu
                .get(player)
                .expect("player menu")
                .close_button_rect(area, &font, &gfx)
        };
        let close_point = |app: &GameApp| {
            let close = close_rect(app);
            PhysicalPosition::new(
                f64::from(close.x) + f64::from(close.width) / 2.0,
                f64::from(close.y) + f64::from(close.height) / 2.0,
            )
        };

        app.handle_cursor_moved(close_point(&app))
            .expect("hover Options close");
        app.handle_right_mouse_button(ElementState::Pressed)
            .expect("right-down is consumed by close control");
        app.handle_right_mouse_button(ElementState::Released)
            .expect("right-up is consumed by close control");
        assert_eq!(
            app.ingame_menu.get(app.local_owner).map(IngameMenuState::page),
            Some(ingame_menu::MenuPage::Options),
            "right-click must not invoke Dialog::OnUserClose"
        );
        assert!(commands.take_submitted_local().is_empty());

        let close = close_rect(&app);
        app.handle_cursor_moved(PhysicalPosition::new(
            f64::from(close.x - 1),
            f64::from(close.y) + f64::from(close.height) / 2.0,
        ))
        .expect("hover title background beside close");
        app.handle_mouse_button(ElementState::Pressed)
            .expect("title background mouse down");
        app.handle_cursor_moved(close_point(&app))
            .expect("move onto close after background down");
        app.handle_mouse_button(ElementState::Released)
            .expect("release over close without close capture");
        assert_eq!(
            app.ingame_menu.get(app.local_owner).map(IngameMenuState::page),
            Some(ingame_menu::MenuPage::Options),
            "release-over must not close unless the close button retained left-down"
        );
        assert!(commands.take_submitted_local().is_empty());

        app.handle_cursor_moved(close_point(&app))
            .expect("re-hover the close button after title dragging moved the dialog");
        app.handle_mouse_button(ElementState::Pressed)
            .expect("Options close mouse down");
        assert_eq!(
            app.ingame_menu.get(app.local_owner).map(IngameMenuState::page),
            Some(ingame_menu::MenuPage::Options),
            "IconButton closes on button-up, not button-down"
        );
        assert!(commands.take_submitted_local().is_empty());
        app.handle_mouse_button(ElementState::Released)
            .expect("Options close mouse up");
        assert_eq!(
            app.ingame_menu.get(app.local_owner).map(IngameMenuState::page),
            Some(ingame_menu::MenuPage::Main),
            "Options close command reactivates Main"
        );
        assert_eq!(
            commands.take_submitted_local(),
            vec![(app.local_owner, ControlEvent::ClearPressed, tick)]
        );

        app.handle_cursor_moved(close_point(&app))
            .expect("hover Main close");
        app.handle_mouse_button(ElementState::Pressed)
            .expect("Main close mouse down");
        assert!(app.ingame_menu.contains(app.local_owner));
        assert!(commands.take_submitted_local().is_empty());
        app.handle_mouse_button(ElementState::Released)
            .expect("Main close mouse up");
        assert!(
            !app.ingame_menu.contains(app.local_owner),
            "Main has no close action and remains closed"
        );
        assert_eq!(
            commands.take_submitted_local(),
            vec![(app.local_owner, ControlEvent::ClearPressed, tick)]
        );
    }

    #[test]
    fn player_menu_title_close_visibility_follows_mouse_owner_and_disable_mouse() {
        let mut app = new_classic_running_sandbox_app();
        let owner = app.local_owner;
        app.open_ingame_menu().expect("open mouse owner's menu");
        let mut frame = vec![0_u8; 320 * 200 * 4];
        app.render(&mut frame).expect("render mouse owner's menu");
        assert!(
            app.ingame_menu_gfx
                .as_ref()
                .is_some_and(|gfx| gfx.show_close_button)
        );

        app.ingame_menu.clear();
        app.ingame_menu.replace(
            owner + 1,
            IngameMenuState::main_menu(&MainMenuConditions::default()),
        );
        app.render(&mut frame)
            .expect("render menu not owned by the mouse player");
        assert!(
            !app.ingame_menu_gfx
                .as_ref()
                .is_some_and(|gfx| gfx.show_close_button),
            "a non-controlling player's C4Menu::HasMouse is false"
        );
        app.local_controls = LocalControlRegistry::default();
        app.local_controls.initialize(LocalControlInit {
            owner: owner + 1,
            preferred_set: 1,
            prefers_mouse: true,
            gamepads_enabled: true,
            replay: false,
            disable_mouse: false,
        });
        app.mouse_control = true;
        app.render(&mut frame)
            .expect("render reassigned mouse owner's menu");
        assert!(
            app.ingame_menu_gfx
                .as_ref()
                .is_some_and(|gfx| gfx.show_close_button),
            "close visibility follows the assigned mouse owner, not local_owner"
        );

        app.ingame_menu.clear();
        app.ingame_menu.replace(
            owner,
            IngameMenuState::main_menu(&MainMenuConditions::default()),
        );
        app.local_controls = LocalControlRegistry::default();
        let assignment = app.local_controls.initialize(LocalControlInit {
            owner,
            preferred_set: 0,
            prefers_mouse: true,
            gamepads_enabled: true,
            replay: false,
            disable_mouse: true,
        });
        assert!(!assignment.mouse);
        app.mouse_control_allowed = false;
        app.mouse_control = false;
        app.render(&mut frame).expect("render DisableMouse menu");
        assert!(
            !app.ingame_menu_gfx
                .as_ref()
                .is_some_and(|gfx| gfx.show_close_button),
            "DisableMouse=1 suppresses the title close button"
        );

        let area = app.graphics.viewport_rect(owner).expect("local viewport");
        let fallback = app.assets.font_arc();
        let font = clonk_frontend::hud::HudFont::from_set(
            app.assets.clonk_fonts.as_deref(),
            fallback.as_ref(),
        );
        let close = app
            .ingame_menu
            .get(owner)
            .expect("disabled-mouse player menu")
            .close_button_rect(
                area,
                &font,
                &IngameMenuGraphics {
                    show_commands: app.display_flags.show_commands,
                    show_close_button: true,
                    ..IngameMenuGraphics::default()
                },
            );
        let point = GuiPoint::new(
            (close.x + close.width as i32 / 2) as f32,
            (close.y + close.height as i32 / 2) as f32,
        );
        assert_eq!(
            app.ingame_menu_pointer_target(point),
            None,
            "DisableMouse leaves no invisible close hit target"
        );
    }

    #[test]
    fn construction_menu_drag_uses_five_pixel_gate_and_focus_loss_clears_capture() {
        let (mut app, _owner, menu_point, _valid, _invalid, _world, _c4id) =
            construction_drag_fixture();
        app.handle_cursor_moved(PhysicalPosition::new(
            f64::from(menu_point.x),
            f64::from(menu_point.y),
        ))
        .expect("move over constructable row");
        app.handle_mouse_button(ElementState::Pressed)
            .expect("arm menu drag");

        app.handle_cursor_moved(PhysicalPosition::new(
            f64::from(menu_point.x + 4.0),
            f64::from(menu_point.y),
        ))
        .expect("move four pixels");
        assert!(matches!(
            app.construction_menu_drag.as_ref(),
            Some(ConstructionMenuDrag::Candidate { .. })
        ));
        app.handle_cursor_moved(PhysicalPosition::new(
            f64::from(menu_point.x + MENU_DRAG_THRESHOLD),
            f64::from(menu_point.y),
        ))
        .expect("move exactly five pixels");
        assert!(app.ingame_construction_drag_active());
        assert!(app.mouse_state.is_none());
        assert!(app.ingame_right_mouse_state.is_none());
        assert!(app.ingame_custom_cursor_active());

        app.handle_focus_lost().expect("lose window focus");
        assert!(app.construction_menu_drag.is_none());
        assert!(!app.ingame_custom_cursor_active());
    }

    #[test]
    fn subthreshold_constructable_menu_click_still_enters_item() {
        let (mut app, owner, menu_point, _valid, _invalid, _world, _c4id) =
            construction_drag_fixture();
        let (manager, _events, mut network_commands) =
            NetworkManager::test_stub_with_commands_for_client_id(7);
        app.network = Some(manager);
        let tick = app.local_control_submission_tick();

        app.handle_cursor_moved(PhysicalPosition::new(
            f64::from(menu_point.x),
            f64::from(menu_point.y),
        ))
        .expect("hover constructable row");
        app.handle_mouse_button(ElementState::Pressed)
            .expect("press constructable row");
        app.handle_mouse_button(ElementState::Released)
            .expect("release without crossing drag sensitivity");

        let (controls, commands, selections) = network_commands.take_submitted_player_inputs();
        assert_eq!(
            controls,
            vec![(
                owner,
                ControlEvent::RawPlayerControl {
                    command: clonk_engine::COM_MENU_ENTER,
                    data: 0,
                },
                tick,
            )]
        );
        assert!(commands.is_empty());
        assert!(selections.is_empty());
        assert!(app.construction_menu_drag.is_none());
    }

    #[test]
    fn invalid_construction_menu_drop_sends_nothing_and_clears_drag() {
        let (mut app, _owner, menu_point, valid_point, invalid_point, _world, _c4id) =
            construction_drag_fixture();
        let (manager, _events, mut network_commands) =
            NetworkManager::test_stub_with_commands_for_client_id(7);
        app.network = Some(manager);

        begin_construction_drag(&mut app, menu_point, valid_point);
        assert!(matches!(
            app.construction_menu_drag.as_ref(),
            Some(ConstructionMenuDrag::Active {
                site_valid: true,
                ..
            })
        ));
        app.handle_cursor_moved(PhysicalPosition::new(
            f64::from(invalid_point.x),
            f64::from(invalid_point.y),
        ))
        .expect("move to invalid site");
        assert!(matches!(
            app.construction_menu_drag.as_ref(),
            Some(ConstructionMenuDrag::Active {
                site_valid: false,
                ..
            })
        ));

        app.handle_mouse_button(ElementState::Released)
            .expect("release invalid construction drag");
        let (controls, commands, selections) = network_commands.take_submitted_player_inputs();
        assert!(controls.is_empty());
        assert!(commands.is_empty());
        assert!(selections.is_empty());
        assert!(app.construction_menu_drag.is_none());
    }

    #[test]
    fn construction_menu_drag_refreshes_site_check_without_pointer_motion() {
        let (mut app, _owner, menu_point, valid_point, _invalid, _world, _c4id) =
            construction_drag_fixture();
        begin_construction_drag(&mut app, menu_point, valid_point);
        assert!(matches!(
            app.construction_menu_drag.as_ref(),
            Some(ConstructionMenuDrag::Active {
                site_valid: true,
                ..
            })
        ));

        let mut filled = Landscape::flat(480, 0);
        filled.set_world_height(220);
        app.engine.set_landscape(filled);
        app.update()
            .expect("advance C4MouseControl construction check");
        assert!(matches!(
            app.construction_menu_drag.as_ref(),
            Some(ConstructionMenuDrag::Active {
                site_valid: false,
                ..
            })
        ));
    }

    #[test]
    fn construction_menu_drag_reprojects_stationary_pointer_after_camera_motion() {
        let (mut app, owner, menu_point, valid_point, _invalid, _world, raw_c4id) =
            construction_drag_fixture();
        begin_construction_drag(&mut app, menu_point, valid_point);
        let before = match app.construction_menu_drag.as_ref() {
            Some(ConstructionMenuDrag::Active {
                pointer: Some(pointer),
                ..
            }) => ingame_pointer_world_pixel(*pointer),
            state => panic!("active drag pointer missing: {state:?}"),
        };
        let retained = app
            .ingame_viewport_mouse
            .expect("construction drag retains native VpX/VpY");
        assert!(matches!(
            app.construction_menu_drag.as_ref(),
            Some(ConstructionMenuDrag::Active {
                viewport_index: Some(index),
                ..
            }) if *index == retained.viewport_index
        ));

        app.engine
            .player_mut(owner)
            .expect("construction owner remains live")
            .set_view_offset(Vector2::new(7, 0));
        app.snapshot = app.engine.snapshot();
        let render_snapshot = app.snapshot.clone();
        let viewports = collect_viewport_inputs(&render_snapshot)
            .expect("camera move keeps a local viewport");
        app.graphics.render_frame(&render_snapshot, &viewports);
        let viewport = app
            .graphics
            .active_viewport_projections()
            .into_iter()
            .find(|viewport| viewport.index == retained.viewport_index)
            .expect("retained physical viewport survives camera move");
        let screen = GuiPoint::new(
            viewport.rect.x.saturating_add(retained.position.x) as f32,
            viewport.rect.y.saturating_add(retained.position.y) as f32,
        );
        let expected_pointer = app
            .graphics
            .viewport_output_point_for_index(viewport.index, screen)
            .expect("stationary VpX/VpY reprojects");
        let expected_world = ingame_pointer_world_pixel(expected_pointer);
        assert_ne!(expected_world, before, "camera motion changes the drop site");
        let mut shifted_ground = Landscape::flat(480, expected_world.y);
        shifted_ground.set_world_height(expected_world.y.saturating_add(40));
        app.engine.set_landscape(shifted_ground);
        assert!(
            app.engine.construction_site_valid("BLD1", expected_world),
            "reprojected camera site is buildable"
        );

        app.refresh_construction_menu_drag();
        assert!(matches!(
            app.construction_menu_drag.as_ref(),
            Some(ConstructionMenuDrag::Active {
                pointer: Some(pointer),
                site_valid: true,
                ..
            }) if ingame_pointer_world_pixel(*pointer) == expected_world
        ));

        let (manager, _events, mut network_commands) =
            NetworkManager::test_stub_with_commands_for_client_id(7);
        app.network = Some(manager);
        let tick = app.local_control_submission_tick();
        app.handle_mouse_button(ElementState::Released)
            .expect("release stationary construction drag");
        let (controls, commands, selections) = network_commands.take_submitted_player_inputs();
        assert!(controls.is_empty());
        assert_eq!(
            commands,
            vec![(
                tick,
                PlayerCommandControlData {
                    player: owner,
                    command: CommandId::Construct as i32,
                    x: expected_world.x,
                    y: expected_world.y,
                    target: 0,
                    target2: 0,
                    data: raw_c4id,
                    add_mode: 1,
                    by_client: 7,
                },
            )]
        );
        assert!(selections.is_empty());
    }

    #[test]
    fn queued_cursor_menu_actions_cannot_fire_after_the_menu_closes() {
        // A converted menu action may execute after another synchronized
        // control has closed the menu. It must never reappear as the raw
        // Throw/Dig action that produced it (C4Object.cpp:3369-3371).
        for (definition_id, raw, callback) in [
            (
                "QTHR",
                ControlCommand::Throw,
                "throw_count",
            ),
            ("QDIG", ControlCommand::Dig, "dig_count"),
        ] {
            let mut app = new_state_only_running_sandbox_app();
            let owner = app.local_owner;
            let script = r#"#strict
local throw_count, dig_count;
func ControlThrow() { throw_count = 1; return(1); }
func ControlDig() { dig_count = 1; return(1); }
"#;
            let mut probe = Definition::from_script(definition_id, "Menu race probe", script)
                .expect("probe definition compiles");
            probe.set_category(clonk_engine::CATEGORY_LIVING);
            probe.set_crew_member(true);
            app.engine
                .register_definition(probe)
                .expect("register menu race probe");
            let cursor = app
                .engine
                .spawn_object(
                    SpawnConfig::new(definition_id)
                        .with_owner(owner)
                        .with_crew_member(true),
                )
                .expect("spawn menu race probe");
            let mut crew = app
                .engine
                .player(owner)
                .expect("sandbox player remains live")
                .crew()
                .to_vec();
            crew.push(cursor);
            app.engine
                .player_mut(owner)
                .expect("sandbox player remains live")
                .set_crew(crew);
            app.engine.clear_crew_selection(owner);
            app.engine
                .select_crew(owner, [cursor])
                .expect("select menu race probe");
            app.engine
                .set_crew_cursor(owner, Some(cursor))
                .expect("make menu race probe the cursor");
            install_test_cursor_menu(&mut app, cursor, two_item_script_menu(cursor));

            let (manager, _events, mut commands) =
                NetworkManager::test_stub_with_commands_for_client_id(7);
            app.network = Some(manager);
            app.dispatch_control_event_for_local_player(
                owner,
                ControlEvent::Command {
                    command: raw,
                    kind: CommandKind::Press,
                },
            )
            .expect("queue cursor-menu action");
            let (_, converted, tick) = commands
                .take_submitted_local()
                .pop()
                .expect("converted control was queued");
            app.engine
                .apply_object_update(
                    cursor,
                    ObjectUpdate {
                        menu: Some(None),
                        ..ObjectUpdate::default()
                    },
                )
                .expect("close menu before control execution");
            app.apply_ready_controls(
                tick,
                vec![NetworkControl::Player {
                    owner,
                    event: converted,
                }],
            )
            .expect("execute converted control after close");
            let cursor = app
                .engine
                .object_snapshot(cursor)
                .expect("menu race probe survives");
            for name in ["throw_count", "dig_count"] {
                assert!(
                    cursor
                        .local_vars
                        .get(name)
                        .is_none_or(|value| value == &Value::Nil),
                    "converted {raw:?} must leave {name} unset"
                );
            }

            // Prove the fixture would catch the old raw packet: a second,
            // deliberately unconverted press reaches the corresponding
            // ControlThrow/ControlDig callback immediately.
            app.apply_ready_controls(
                tick.saturating_add(1),
                vec![NetworkControl::Player {
                    owner,
                    event: ControlEvent::Command {
                        command: raw,
                        kind: CommandKind::Press,
                    },
                }],
            )
            .expect("execute deliberate raw gameplay control");
            assert_eq!(
                app.engine
                    .object_snapshot(cursor.id)
                    .expect("menu race probe survives raw control")
                    .local_vars
                    .get(callback),
                Some(&Value::Int(1)),
                "the fixture must observe an unconverted {raw:?} action"
            );
        }
    }

    #[test]
    fn engine_script_menu_is_visible_and_consumes_raw_player_controls() {
        // C4Viewport draws the cursor object's menu (C4Viewport.cpp:
        // 983-995), while C4Player::InCom converts raw controls before
        // gameplay (C4Player.cpp:1502-1513). This is the app half of the
        // mandatory Dragon Rock difficulty/type menu path.
        clonk_logging::init();
        let mut app = new_classic_running_sandbox_app();
        let cursor = app
            .engine
            .crew_cursor(app.local_owner)
            .expect("sandbox cursor");
        let menu = two_item_script_menu(cursor);

        let mut baseline = vec![0u8; 320 * 200 * 4];
        app.render(&mut baseline).expect("baseline render");
        app.engine
            .apply_object_update(
                cursor,
                ObjectUpdate {
                    menu: Some(Some(menu)),
                    ..ObjectUpdate::default()
                },
            )
            .expect("install script menu");
        let mut with_menu = vec![0u8; 320 * 200 * 4];
        app.render(&mut with_menu).expect("menu render");
        assert_ne!(
            with_menu, baseline,
            "an engine-created script menu must be visible"
        );
        let mut before_tooltip = with_menu.clone();
        for _ in 1..89 {
            app.render(&mut before_tooltip).expect("pre-tooltip render");
        }
        let mut with_tooltip = vec![0u8; 320 * 200 * 4];
        app.render(&mut with_tooltip).expect("90th menu render");
        assert_ne!(
            with_tooltip, before_tooltip,
            "C4MN_InfoCaption_Delay shows the tooltip on draw 90"
        );

        app.dispatch_control_event(ControlEvent::Press(ControlButton::Right))
            .expect("right press");
        app.dispatch_control_event(ControlEvent::Release(ControlButton::Right))
            .expect("right release");
        let menu = app
            .engine
            .debug_object_menu(cursor.as_u64())
            .expect("cursor exists")
            .expect("menu open");
        assert_eq!(menu.selection, 1, "release must not navigate twice");
        assert_eq!(
            app.engine
                .object_snapshot(cursor)
                .expect("cursor snapshot")
                .command_direction,
            CommandDirection::Stop,
            "menu navigation must not steer the crew"
        );

        app.dispatch_control_event(ControlEvent::Command {
            command: ControlCommand::Throw,
            kind: CommandKind::Press,
        })
        .expect("enter press");
        app.dispatch_control_event(ControlEvent::Command {
            command: ControlCommand::Throw,
            kind: CommandKind::Release,
        })
        .expect("enter release");
        assert_eq!(app.engine.debug_object_menu(cursor.as_u64()), Some(None));
    }

    #[test]
    fn first_local_menu_press_reveals_progressive_text_before_navigation() {
        // C4Game::LocalPlayerControl performs the asynchronous ConvertCom
        // pass before offline dispatch/network submission. Only this local
        // raw press may become COM_MenuShowText; synchronized controls must
        // not recalculate the choice from client-specific text progress.
        let mut app = new_state_only_running_sandbox_app();
        let cursor = app
            .engine
            .crew_cursor(app.local_owner)
            .expect("sandbox cursor");
        let mut menu = two_item_script_menu(cursor);
        menu.text_progressing = true;
        for item in &mut menu.items {
            item.text_display_progress = 0;
        }
        app.engine
            .apply_object_update(
                cursor,
                ObjectUpdate {
                    menu: Some(Some(menu)),
                    ..ObjectUpdate::default()
                },
            )
            .expect("install progressive script menu");

        app.dispatch_control_event(ControlEvent::Press(ControlButton::Right))
            .expect("first right press reveals text");
        let menu = app
            .engine
            .debug_object_menu(cursor.as_u64())
            .expect("cursor exists")
            .expect("menu stays open");
        assert_eq!(menu.selection, 0, "reveal must not navigate");
        assert!(!menu.text_progressing);
        assert!(menu
            .items
            .iter()
            .all(|item| item.text_display_progress == -1));

        app.dispatch_control_event(ControlEvent::Release(ControlButton::Right))
            .expect("right release");
        app.dispatch_control_event(ControlEvent::Press(ControlButton::Right))
            .expect("second right press navigates");
        assert_eq!(
            app.engine
                .debug_object_menu(cursor.as_u64())
                .expect("cursor exists")
                .expect("menu stays open")
                .selection,
            1
        );
    }

    #[test]
    fn normal_menu_render_rejects_an_unresolved_non_textspec_item_picture() {
        let mut app = new_classic_running_sandbox_app();
        let cursor = app
            .engine
            .crew_cursor(app.local_owner)
            .expect("sandbox cursor");
        let mut menu = two_item_script_menu(cursor);
        menu.style = 0;
        menu.items[0].item_id = "MISS".to_string();
        menu.items[0].image = clonk_engine::ObjectMenuImage::Definition;
        menu.items[0].presentation_definition_id = Some("MISS".to_string());
        assert!(
            object_menu_item_picture(
                &app.engine,
                &app.snapshot,
                &menu.items[0],
                0,
                &HudGraphics::default(),
                menu.style,
            )
            .is_none(),
            "fixture must exercise the unresolved non-TextSpec branch"
        );
        install_test_cursor_menu(&mut app, cursor, menu);

        let mut frame = vec![0_u8; app.graphics.surface().pixels().len()];
        let error = app
            .render(&mut frame)
            .expect_err("Normal menu must fail closed on an unresolved definition image");
        assert!(
            error
                .to_string()
                .contains("unresolved classic menu image at item 0"),
            "unexpected error: {error:#}"
        );
        assert!(
            error.to_string().contains("Definition"),
            "unexpected recipe: {error:#}"
        );
    }

    #[test]
    fn engine_dialog_menu_renders_classic_style_instead_of_fallback() {
        let mut app = new_classic_running_sandbox_app();
        let cursor = app
            .engine
            .crew_cursor(app.local_owner)
            .expect("sandbox cursor");
        let mut menu = two_item_script_menu(cursor);
        menu.caption.clear();
        menu.style = 3;
        menu.columns = 1;
        for item in &mut menu.items {
            item.image = clonk_engine::ObjectMenuImage::None;
        }
        let mut baseline = vec![0_u8; 320 * 200 * 4];
        app.render(&mut baseline).expect("baseline render");
        app.engine
            .apply_object_update(
                cursor,
                ObjectUpdate {
                    menu: Some(Some(menu)),
                    ..ObjectUpdate::default()
                },
            )
            .expect("install dialog menu");
        let mut rendered = vec![0_u8; 320 * 200 * 4];
        app.render(&mut rendered).expect("classic Dialog render");
        assert_ne!(rendered, baseline);
    }

    #[test]
    fn engine_context_menu_is_visible_and_navigable_through_the_app() {
        // C4Player::Execute installs C4MN_Context as style 1 on the cursor;
        // C4Viewport draws that engine-owned menu and C4Player::InCom routes
        // navigation to it before gameplay (C4Object.cpp:1961-1980,
        // 2044-2062; C4Viewport.cpp:983-995; C4Player.cpp:1502-1513).
        clonk_logging::init();
        let mut app = new_classic_running_sandbox_app();
        let cursor = app
            .engine
            .crew_cursor(app.local_owner)
            .expect("sandbox cursor");
        let mut menu = two_item_script_menu(cursor);
        menu.caption = "Hut".to_string();
        menu.identification = serde_json::from_value(serde_json::json!({ "Int": 14 }))
            .expect("integer menu identification deserializes");
        menu.style = 1;
        menu.permanent = true;
        menu.user_menu = false;
        menu.columns = 1;

        let mut baseline = vec![0_u8; 320 * 200 * 4];
        app.render(&mut baseline).expect("baseline render");
        app.engine
            .apply_object_update(
                cursor,
                ObjectUpdate {
                    menu: Some(Some(menu)),
                    ..ObjectUpdate::default()
                },
            )
            .expect("install context menu");
        let mut with_menu = vec![0_u8; 320 * 200 * 4];
        app.render(&mut with_menu).expect("context render");
        assert_ne!(with_menu, baseline, "style-1 context menu must be visible");

        app.dispatch_control_event(ControlEvent::Press(ControlButton::Right))
            .expect("right press");
        app.dispatch_control_event(ControlEvent::Release(ControlButton::Right))
            .expect("right release");
        let menu = app
            .engine
            .debug_object_menu(cursor.as_u64())
            .expect("cursor exists")
            .expect("context remains open");
        assert_eq!(menu.selection, 1);
        let context_identification = serde_json::from_value(serde_json::json!({ "Int": 14 }))
            .expect("integer menu identification deserializes");
        assert_eq!(menu.identification, context_identification);
    }

    #[test]
    fn engine_info_menu_renders_the_classic_style_instead_of_a_fallback() {
        clonk_logging::init();
        let mut app = new_classic_running_sandbox_app();
        let owner = app.local_owner;
        let cursor = app
            .engine
            .crew_cursor(app.local_owner)
            .expect("sandbox cursor");
        let mut menu = two_item_script_menu(cursor);
        menu.caption = "Information".to_string();
        menu.style = 2;
        menu.columns = 1;
        menu.items.truncate(1);
        menu.items[0].caption = "Hidden caption".to_string();
        menu.items[0].info_caption = "<c 00ff00>Classic wrapped information</c>".to_string();
        menu.items[0].command.clear();
        menu.items[0].command2.clear();
        menu.items[0].selectable = false;
        menu.items[0].picture_object = Some(cursor);
        menu.selection = -1;
        menu.user_menu = false;

        let mut baseline = vec![0_u8; 320 * 200 * 4];
        app.render(&mut baseline).expect("baseline render");
        app.engine
            .apply_object_update(
                cursor,
                ObjectUpdate {
                    menu: Some(Some(menu)),
                    ..ObjectUpdate::default()
                },
            )
            .expect("install Info menu");
        let mut with_menu = vec![0_u8; 320 * 200 * 4];
        app.render(&mut with_menu)
            .expect("classic style-2 Info menu renders");
        assert_ne!(with_menu, baseline);
        let initial_location = app
            .script_menu_presentations
            .get(&owner)
            .and_then(|state| state.location)
            .expect("internal Info latches its target-relative location");

        app.engine
            .apply_object_update(
                cursor,
                ObjectUpdate::default().with_position(Vector2::new(280, 160)),
            )
            .expect("move Info target");
        app.snapshot = app.engine.snapshot();
        app.refresh_focus();
        app.render(&mut with_menu)
            .expect("render after target move");
        assert_eq!(
            app.script_menu_presentations
                .get(&owner)
                .and_then(|state| state.location),
            Some(initial_location),
            "C4Menu::SetLocation is one-shot; the menu must not follow a moving target"
        );
    }

    #[test]
    fn engine_script_menu_pointer_selects_enters_and_closes_like_cpp() {
        // C4MenuItem::MouseEnter selects a selectable item, left-up enters
        // it, and Dialog's Ico_Close queues COM_MenuClose
        // (C4Menu.cpp:213-242, 1237-1262; C4ObjectMenu.cpp:461-478).
        clonk_logging::init();
        let mut app = new_classic_running_sandbox_app();
        let cursor = app
            .engine
            .crew_cursor(app.local_owner)
            .expect("sandbox cursor");
        let menu = two_item_script_menu(cursor);
        app.engine
            .apply_object_update(
                cursor,
                ObjectUpdate {
                    menu: Some(Some(menu.clone())),
                    ..ObjectUpdate::default()
                },
            )
            .expect("install script menu");
        let mut frame = vec![0_u8; 320 * 200 * 4];
        app.render(&mut frame).expect("establish viewport layout");

        let (second_item, close_button) = {
            let fallback = app.assets.font_arc();
            let font = clonk_frontend::hud::HudFont::from_set(
                app.assets.clonk_fonts.as_deref(),
                fallback.as_ref(),
            );
            let area = app
                .graphics
                .viewport_rect(app.local_owner)
                .expect("local viewport");
            let layout = object_menu::engine_script_menu_layout(
                area,
                &font,
                &menu,
                app.display_flags.show_commands,
            );
            (
                layout.item_rect(1).expect("second item rect"),
                layout.close_button_rect(),
            )
        };
        let second_point = PhysicalPosition::new(
            f64::from(second_item.x) + 8.0,
            f64::from(second_item.y) + 8.0,
        );
        app.handle_cursor_moved(second_point)
            .expect("hover second item");
        assert_eq!(
            app.engine
                .debug_object_menu(cursor.as_u64())
                .expect("cursor")
                .expect("menu")
                .selection,
            1,
            "hover must select the item under the pointer"
        );
        app.handle_mouse_button(ElementState::Pressed)
            .expect("item mouse down");
        app.handle_mouse_button(ElementState::Released)
            .expect("item mouse up");
        assert_eq!(app.engine.debug_object_menu(cursor.as_u64()), Some(None));

        let mut right_menu = menu.clone();
        right_menu.items[1].command2 = "SetComDir(COMD_Right())".to_string();
        app.engine
            .apply_object_update(
                cursor,
                ObjectUpdate {
                    menu: Some(Some(right_menu)),
                    ..ObjectUpdate::default()
                },
            )
            .expect("reinstall script menu for right enter");
        app.handle_cursor_moved(second_point)
            .expect("hover second item for right enter");
        app.handle_right_mouse_button(ElementState::Pressed)
            .expect("right item mouse down");
        app.handle_right_mouse_button(ElementState::Released)
            .expect("right item mouse up");
        assert_eq!(
            app.engine
                .object_snapshot(cursor)
                .expect("cursor survives right enter")
                .command_direction,
            CommandDirection::Right,
            "right-up must dispatch COM_MenuEnterAll and execute Command2"
        );
        assert_eq!(app.engine.debug_object_menu(cursor.as_u64()), Some(None));

        app.engine
            .apply_object_update(
                cursor,
                ObjectUpdate {
                    menu: Some(Some(menu)),
                    ..ObjectUpdate::default()
                },
            )
            .expect("reinstall script menu");
        let close_point = PhysicalPosition::new(
            f64::from(close_button.x) + 8.0,
            f64::from(close_button.y) + 8.0,
        );
        app.handle_cursor_moved(close_point)
            .expect("hover close button");
        app.handle_mouse_button(ElementState::Pressed)
            .expect("close mouse down");
        app.handle_mouse_button(ElementState::Released)
            .expect("close mouse up");
        assert_eq!(app.engine.debug_object_menu(cursor.as_u64()), Some(None));
    }

    #[test]
    fn l065_running_menu_wheels_are_pixel_persistent_and_never_reach_gameplay() {
        let mut app = new_classic_running_sandbox_app();
        let owner = app.local_owner;
        let cursor = app.engine.crew_cursor(owner).expect("sandbox cursor");
        let menu = long_script_menu(cursor, 12);
        install_test_cursor_menu(&mut app, cursor, menu);
        let mut frame = vec![0_u8; 320 * 200 * 4];
        app.render(&mut frame).expect("seed script presentation");
        let (_events, mut commands) = install_running_network_stub(&mut app, 0, 40, 4);

        let (_, layout) = app
            .script_menu_layout_for_owner(owner, false)
            .expect("script layout resources")
            .expect("open normal script menu");
        assert!(layout.max_scroll_y >= 60);
        let client_point = GuiPoint::new(
            (layout.client.x + 4) as f32,
            (layout.client.y + 4) as f32,
        );
        app.handle_cursor_moved(PhysicalPosition::new(
            f64::from(client_point.x),
            f64::from(client_point.y),
        ))
        .expect("hover script ScrollWindow");
        let selection = app
            .engine
            .cursor_object_menu(owner)
            .expect("script menu open")
            .1
            .selection;
        app.handle_mouse_wheel(MouseScrollDelta::LineDelta(0.0, -1.0), 1.0)
            .expect("script menu consumes wheel down");
        assert_eq!(
            app.script_menu_presentations
                .get(&owner)
                .expect("script presentation")
                .scroll_y,
            60
        );
        assert_eq!(
            app.engine
                .cursor_object_menu(owner)
                .expect("wheel leaves menu open")
                .1
                .selection,
            selection,
            "wheel must not move the synchronized menu selection"
        );
        assert!(commands.take_submitted_local().is_empty());
        app.render(&mut frame)
            .expect("render preserves wheel displacement");
        assert_eq!(
            app.script_menu_presentations
                .get(&owner)
                .expect("script presentation")
                .scroll_y,
            60,
            "redraw must not pin an unchanged selection back into view"
        );

        let (_, geometry) = app
            .script_menu_geometry_for_owner(owner)
            .expect("script geometry resources")
            .expect("script geometry");
        let title = geometry.title.expect("normal menu title");
        let title_point = GuiPoint::new((title.x + 24) as f32, (title.y + 5) as f32);
        app.handle_cursor_moved(PhysicalPosition::new(
            f64::from(title_point.x),
            f64::from(title_point.y),
        ))
        .expect("hover script title");
        app.handle_mouse_wheel(MouseScrollDelta::LineDelta(0.0, -1.0), 1.0)
            .expect("external dialog consumes title wheel");
        assert_eq!(
            app.script_menu_presentations
                .get(&owner)
                .expect("script presentation")
                .scroll_y,
            60,
            "only the ScrollWindow client scrolls"
        );
        assert!(commands.take_submitted_local().is_empty());

        app.engine
            .apply_object_update(
                cursor,
                ObjectUpdate {
                    menu: Some(None),
                    ..ObjectUpdate::default()
                },
            )
            .expect("close script menu");
        app.script_menu_presentations.remove(&owner);
        let players = (0..12)
            .map(|index| NewPlayerEntry {
                file: format!("Player{index}.c4p"),
                name: format!("Player {index}"),
            })
            .collect::<Vec<_>>();
        app.ingame_menu.replace(
            owner,
            Some(IngameMenuState::new_player_menu(&players)),
        );
        app.render(&mut frame).expect("render long player menu");
        let area = app.ingame_menu_area(owner).expect("player viewport");
        let fallback = app.assets.font_arc();
        let font = clonk_frontend::hud::HudFont::from_set(
            app.assets.clonk_fonts.as_deref(),
            fallback.as_ref(),
        );
        let gfx = IngameMenuGraphics {
            show_commands: app.display_flags.show_commands,
            show_close_button: true,
            ..IngameMenuGraphics::default()
        };
        let bounds = app
            .ingame_menu
            .get(owner)
            .expect("player menu")
            .bounds(area, &font, &gfx);
        let player_client = GuiPoint::new((bounds.x + 6) as f32, (bounds.y + 30) as f32);
        assert!(app
            .ingame_menu
            .get(owner)
            .expect("player menu")
            .client_contains(area, &font, &gfx, player_client));
        app.handle_cursor_moved(PhysicalPosition::new(
            f64::from(player_client.x),
            f64::from(player_client.y),
        ))
        .expect("hover player-menu ScrollWindow");
        let selection = app.ingame_menu.get(owner).expect("player menu").selection();
        app.handle_mouse_wheel(MouseScrollDelta::LineDelta(0.0, -1.0), 1.0)
            .expect("player menu consumes wheel down");
        let player_menu = app.ingame_menu.get(owner).expect("player menu");
        assert_eq!(player_menu.scroll_y(), 60);
        assert_eq!(player_menu.selection(), selection);
        assert!(commands.take_submitted_local().is_empty());
        app.render(&mut frame)
            .expect("player-menu redraw preserves wheel displacement");
        assert_eq!(app.ingame_menu.get(owner).unwrap().scroll_y(), 60);
    }

    #[test]
    fn l065_script_menu_scroll_and_drag_state_is_per_viewport_owner() {
        let mut app = new_classic_running_sandbox_app();
        let primary = app.local_owner;
        let secondary = primary + 1;
        let primary_cursor = app.engine.crew_cursor(primary).expect("primary cursor");
        let primary_state = app
            .engine
            .object_snapshot(primary_cursor)
            .expect("primary cursor state");

        app.engine
            .register_player(PlayerConfig::new(secondary, "Secondary"))
            .expect("register secondary player");
        let secondary_position = Vector2::new(
            primary_state.position.x.saturating_add(24),
            primary_state.position.y,
        );
        let secondary_cursor = app
            .engine
            .spawn_object(
                SpawnConfig::new(primary_state.definition_id)
                    .with_position(secondary_position)
                    .with_owner(secondary)
                    .with_crew_member(true),
            )
            .expect("spawn secondary cursor");
        app.engine
            .select_crew(secondary, [secondary_cursor])
            .expect("select secondary cursor");
        app.engine
            .set_crew_cursor(secondary, Some(secondary_cursor))
            .expect("set secondary cursor");
        app.engine
            .replace_player_viewports(
                secondary,
                vec![clonk_engine::PlayerViewport::new(secondary_position)
                    .with_focus(Some(secondary_cursor))],
            )
            .expect("set secondary viewport");
        app.engine.set_local_players([primary, secondary]);
        app.local_controls = LocalControlRegistry::default();
        for (owner, preferred_set, prefers_mouse) in
            [(primary, 0, false), (secondary, 1, true)]
        {
            app.local_controls.initialize(LocalControlInit {
                owner,
                preferred_set,
                prefers_mouse,
                gamepads_enabled: true,
                replay: false,
                disable_mouse: false,
            });
        }
        app.mouse_control = true;
        install_test_cursor_menu(&mut app, primary_cursor, long_script_menu(primary_cursor, 12));
        install_test_cursor_menu(
            &mut app,
            secondary_cursor,
            long_script_menu(secondary_cursor, 12),
        );
        app.snapshot = app.engine.snapshot();

        let mut frame = vec![0_u8; 320 * 200 * 4];
        app.render(&mut frame)
            .expect("render both viewport-owned script menus");
        assert!(app.script_menu_presentations.contains_key(&primary));
        assert!(app.script_menu_presentations.contains_key(&secondary));

        let (_, secondary_layout) = app
            .script_menu_layout_for_owner(secondary, false)
            .expect("secondary layout resources")
            .expect("secondary script menu");
        assert!(secondary_layout.max_scroll_y >= 60);
        let client = PhysicalPosition::new(
            f64::from(secondary_layout.client.x + 4),
            f64::from(secondary_layout.client.y + 4),
        );
        app.handle_cursor_moved(client)
            .expect("hover secondary ScrollWindow");
        app.handle_mouse_wheel(MouseScrollDelta::LineDelta(0.0, -1.0), 1.0)
            .expect("scroll secondary script menu");
        assert_eq!(app.script_menu_presentations[&primary].scroll_y, 0);
        assert_eq!(app.script_menu_presentations[&secondary].scroll_y, 60);
        app.render(&mut frame)
            .expect("retain independent viewport scroll state");
        assert_eq!(app.script_menu_presentations[&primary].scroll_y, 0);
        assert_eq!(app.script_menu_presentations[&secondary].scroll_y, 60);

        let (_, geometry) = app
            .script_menu_geometry_for_owner(secondary)
            .expect("secondary geometry resources")
            .expect("secondary geometry");
        let title = geometry.title.expect("secondary wooden title");
        let start = PhysicalPosition::new(f64::from(title.x + 3), f64::from(title.y + 5));
        app.handle_cursor_moved(start)
            .expect("hover secondary title");
        app.handle_mouse_button(ElementState::Pressed)
            .expect("capture secondary title");
        let destination = PhysicalPosition::new(start.x + 11.0, start.y + 7.0);
        app.handle_cursor_moved(destination)
            .expect("drag secondary title");
        app.handle_mouse_button(ElementState::Released)
            .expect("release secondary title");
        assert_eq!(app.script_menu_presentations[&primary].location, None);
        assert_eq!(
            app.script_menu_presentations[&secondary].location,
            Some((geometry.bounds.x + 11, geometry.bounds.y + 7)),
        );
    }

    #[test]
    fn runtime_music_flash_recurses_through_every_player_and_engine_menu_screen() {
        let every_player_menu_page = || {
            let entry = GoalRuleEntry {
                definition_id: "CLNK".to_string(),
                name: "Entry".to_string(),
                description: None,
                fulfilled: false,
            };
            vec![
                IngameMenuState::main_menu(&MainMenuConditions::default())
                    .expect("default player main menu"),
                IngameMenuState::hostility_menu(&[]),
                IngameMenuState::observer_menu(&[], ObserverTarget::Free),
                IngameMenuState::team_selection_menu(&[TeamSelectionEntry {
                    id: 1,
                    caption: "Team".to_string(),
                    icon_spec: None,
                    color: 0,
                    has_participants: false,
                }]),
                IngameMenuState::goals_menu(std::slice::from_ref(&entry)),
                IngameMenuState::rules_menu(std::slice::from_ref(&entry)),
                IngameMenuState::new_player_menu(&[ingame_menu::NewPlayerEntry {
                    file: "Player.c4p".to_string(),
                    name: "Player".to_string(),
                }]),
                IngameMenuState::savegame_menu(&[SaveSlotState { free: true }; 10]),
                IngameMenuState::options_menu(
                    &OptionFlags {
                        sound: true,
                        music: true,
                        mouse_shown: true,
                        mouse: true,
                    },
                    0,
                ),
                IngameMenuState::display_menu(&DisplayFlags::default(), 0),
                IngameMenuState::surrender_menu(),
                IngameMenuState::client_disconnect_menu(),
                IngameMenuState::host_disconnect_menu(&[HostDisconnectClientEntry {
                    client_id: 0,
                    caption: "Host (Host)".to_string(),
                    activated: true,
                }]),
            ]
        };
        let default_pages = every_player_menu_page();
        let rebound_pages = every_player_menu_page();
        let sound_pages = every_player_menu_page();
        assert_eq!(default_pages.len(), 13);
        let page_index = |page: ingame_menu::MenuPage| match page {
            ingame_menu::MenuPage::Main => 0,
            ingame_menu::MenuPage::Hostility => 1,
            ingame_menu::MenuPage::Observer => 2,
            ingame_menu::MenuPage::TeamSelection => 3,
            ingame_menu::MenuPage::Goals => 4,
            ingame_menu::MenuPage::Rules => 5,
            ingame_menu::MenuPage::NewPlayer => 6,
            ingame_menu::MenuPage::Savegame => 7,
            ingame_menu::MenuPage::Options => 8,
            ingame_menu::MenuPage::Display => 9,
            ingame_menu::MenuPage::Surrender => 10,
            ingame_menu::MenuPage::ClientDisconnect => 11,
            ingame_menu::MenuPage::HostDisconnect => 12,
        };
        let test_music_bytes = silent_pcm_wav(10);
        let load_test_music = |app: &GameApp| {
            app.audio
                .as_ref()
                .expect("test audio")
                .system
                .load_music(&test_music_bytes)
                .expect("load lightweight runtime music fixture")
        };
        let prime_music_toggle_off = |app: &mut GameApp, music: &MusicHandle| {
            app.audio
                .as_ref()
                .expect("test audio")
                .system
                .play_music(music, true)
                .expect("start lightweight runtime music fixture");
            app.runtime_music_enabled = true;
        };

        let mut default_app = new_classic_lightweight_running_sandbox_app();
        let default_music = load_test_music(&default_app);
        let mut rebound_app = new_classic_lightweight_running_sandbox_app();
        rebound_app
            .bindings
            .rebind(ControlBindingId::Left, VirtualKeyCode::F3);
        rebound_app
            .engine
            .player_mut(rebound_app.local_owner)
            .expect("local player")
            .control
            .control_style = true;
        let mut sound_app = new_classic_lightweight_running_sandbox_app();
        let mut covered = [false; 13];
        for ((default_menu, rebound_menu), sound_menu) in default_pages
            .into_iter()
            .zip(rebound_pages)
            .zip(sound_pages)
        {
            let page = default_menu.page();
            covered[page_index(page)] = true;
            assert_eq!(rebound_menu.page(), page);
            assert_eq!(sound_menu.page(), page);

            default_app
                .ingame_menu
                .replace(default_app.local_owner, Some(default_menu));
            prime_music_toggle_off(&mut default_app, &default_music);
            default_app
                .handle_key(VirtualKeyCode::F3, ElementState::Pressed)
                .expect("music producer reaches every player-menu page");
            let draws_before = default_app
                .runtime_flash_message
                .as_ref()
                .expect("localized flash")
                .remaining_draws;
            let mut frame = vec![0_u8; 320 * 200 * 4];
            default_app
                .render(&mut frame)
                .unwrap_or_else(|error| panic!("render flash over {page:?}: {error:#}"));
            assert_eq!(
                default_app
                    .runtime_flash_message
                    .as_ref()
                    .expect("music text lasts more than one draw")
                    .remaining_draws,
                draws_before - 1,
                "page {page:?}"
            );
            assert_eq!(
                default_app.ingame_menu.as_ref().map(IngameMenuState::page),
                Some(page)
            );
            default_app
                .handle_key(VirtualKeyCode::F3, ElementState::Released)
                .expect("release music producer");

            rebound_app
                .ingame_menu
                .replace(rebound_app.local_owner, Some(rebound_menu));
            rebound_app
                .handle_key(VirtualKeyCode::F3, ElementState::Pressed)
                .expect("player priority owns F3 on every page");
            assert!(rebound_app.runtime_flash_message.is_none(), "page {page:?}");
            assert!(rebound_app.ingame_menu.is_some(), "page {page:?}");
            rebound_app
                .handle_key(VirtualKeyCode::F3, ElementState::Released)
                .expect("release rebound player control");
            assert!(!rebound_app
                .pressed_engine_keys
                .contains(&VirtualKeyCode::F3));
            assert_eq!(
                rebound_app
                    .engine
                    .player(rebound_app.local_owner)
                    .expect("local player")
                    .control
                    .pressed_coms
                    & (1 << clonk_engine::COM_LEFT),
                0
            );

            sound_app
                .ingame_menu
                .replace(sound_app.local_owner, Some(sound_menu));
            let sound_before = sound_app
                .audio
                .as_ref()
                .expect("test audio")
                .options
                .sound_enabled;
            sound_app
                .handle_modifiers_changed(ModifiersState::CTRL)
                .expect("set Ctrl");
            sound_app
                .handle_key(VirtualKeyCode::F3, ElementState::Pressed)
                .expect("Ctrl+F3 reaches every player-menu page");
            assert_eq!(
                sound_app
                    .audio
                    .as_ref()
                    .expect("test audio")
                    .options
                    .sound_enabled,
                !sound_before,
                "page {page:?}"
            );
            assert!(sound_app.runtime_flash_message.is_none(), "page {page:?}");
            assert!(sound_app.ingame_menu.is_some(), "page {page:?}");
            sound_app
                .handle_key(VirtualKeyCode::F3, ElementState::Released)
                .expect("release sound producer");
            sound_app
                .handle_modifiers_changed(ModifiersState::empty())
                .expect("release Ctrl");
        }
        assert!(covered.into_iter().all(|covered| covered));

        let mut default_app = new_classic_lightweight_running_sandbox_app();
        let default_music = load_test_music(&default_app);
        let mut rebound = new_classic_lightweight_running_sandbox_app();
        rebound
            .bindings
            .rebind(ControlBindingId::Left, VirtualKeyCode::F3);
        rebound
            .engine
            .player_mut(rebound.local_owner)
            .expect("local player")
            .control
            .control_style = true;
        let mut sound = new_classic_lightweight_running_sandbox_app();
        for style in 0..=3 {
            for text_progressing in [false, true] {
                let install_menu = |app: &mut GameApp| {
                    let cursor = app
                        .engine
                        .crew_cursor(app.local_owner)
                        .expect("sandbox cursor");
                    let mut menu = two_item_script_menu(cursor);
                    menu.style = style;
                    menu.text_progressing = text_progressing;
                    app.engine
                        .apply_object_update(
                            cursor,
                            ObjectUpdate {
                                menu: Some(Some(menu)),
                                ..ObjectUpdate::default()
                            },
                        )
                        .expect("install engine menu style");
                    app.snapshot = app.engine.snapshot();
                };

                install_menu(&mut default_app);
                prime_music_toggle_off(&mut default_app, &default_music);
                default_app
                    .handle_key(VirtualKeyCode::F3, ElementState::Pressed)
                    .expect("music producer reaches every engine menu style");
                let draws_before = default_app
                    .runtime_flash_message
                    .as_ref()
                    .expect("localized flash")
                    .remaining_draws;
                let mut frame = vec![0_u8; 320 * 200 * 4];
                default_app.render(&mut frame).unwrap_or_else(|error| {
                    panic!("render style {style}, progress {text_progressing}: {error:#}")
                });
                assert_eq!(
                    default_app
                        .runtime_flash_message
                        .as_ref()
                        .expect("music text lasts more than one draw")
                        .remaining_draws,
                    draws_before - 1
                );
                assert!(default_app
                    .engine
                    .cursor_object_menu(default_app.local_owner)
                    .is_some());
                default_app
                    .handle_key(VirtualKeyCode::F3, ElementState::Released)
                    .expect("release music producer");

                install_menu(&mut rebound);
                rebound
                    .handle_key(VirtualKeyCode::F3, ElementState::Pressed)
                    .expect("player F3 owns every engine menu style");
                assert!(rebound.runtime_flash_message.is_none());
                assert!(rebound
                    .engine
                    .cursor_object_menu(rebound.local_owner)
                    .is_some());
                rebound
                    .handle_key(VirtualKeyCode::F3, ElementState::Released)
                    .expect("release rebound player control");
                assert!(!rebound.pressed_engine_keys.contains(&VirtualKeyCode::F3));
                assert_eq!(
                    rebound
                        .engine
                        .player(rebound.local_owner)
                        .expect("local player")
                        .control
                        .pressed_coms
                        & (1 << clonk_engine::COM_LEFT),
                    0
                );

                install_menu(&mut sound);
                let before = sound
                    .audio
                    .as_ref()
                    .expect("test audio")
                    .options
                    .sound_enabled;
                sound
                    .handle_modifiers_changed(ModifiersState::CTRL)
                    .expect("set Ctrl");
                sound
                    .handle_key(VirtualKeyCode::F3, ElementState::Pressed)
                    .expect("Ctrl+F3 reaches every engine menu style");
                assert_eq!(
                    sound
                        .audio
                        .as_ref()
                        .expect("test audio")
                        .options
                        .sound_enabled,
                    !before
                );
                assert!(sound.runtime_flash_message.is_none());
                assert!(sound.engine.cursor_object_menu(sound.local_owner).is_some());
                sound
                    .handle_key(VirtualKeyCode::F3, ElementState::Released)
                    .expect("release sound producer");
                sound
                    .handle_modifiers_changed(ModifiersState::empty())
                    .expect("release Ctrl");
            }
        }
    }

    #[test]
    fn runtime_flash_draws_above_f1_help_and_below_recursive_context_gui() {
        let mut help = new_classic_running_sandbox_app();
        help.status_text.clear();
        help.snapshot.hud.messages.clear();
        help.handle_key(VirtualKeyCode::F1, ElementState::Pressed)
            .expect("show help beneath flash");
        help.set_runtime_flash_message("AAAA", RuntimeHelpCharset::Windows1252)
            .expect("install flash above help");
        let flash = help.runtime_flash_message.take().expect("flash state");
        let mut help_only = vec![0_u8; 320 * 200 * 4];
        help.render(&mut help_only).expect("render help-only frame");
        let mut expected = Surface::new(320, 200, PixelFormat::Rgba8888);
        expected.pixels_mut().copy_from_slice(&help_only);
        let gamma = help
            .graphics
            .active_gamma_ramp(&help.snapshot.environment.gamma);
        let fonts = help.assets.clonk_fonts.clone().expect("FontRegular");
        clonk_frontend::flash_message::render_flash_message(
            &mut expected,
            &fonts.text,
            &flash.text,
            flash.y,
            Some(&gamma),
            &MessageFontImages::default(),
        );
        help.runtime_flash_message = Some(flash);
        let mut actual = vec![0_u8; 320 * 200 * 4];
        help.render(&mut actual).expect("render help then flash");
        assert_eq!(actual, expected.pixels());

        let mut context = new_classic_running_sandbox_app();
        context.status_text.clear();
        context.snapshot.hud.messages.clear();
        context
            .open_context_menu_at(
                vec![
                    ContextMenuEntry::<AppContextMenuCommand>::new("Root").with_submenu(vec![
                        ContextMenuEntry::new("Child")
                            .with_submenu(vec![ContextMenuEntry::new("Context above flash")]),
                    ]),
                ],
                GuiPoint::new(120.0, 55.0),
            )
            .expect("open overlapping recursive context");
        for depth in 0..2 {
            context
                .handle_key(VirtualKeyCode::Right, ElementState::Pressed)
                .unwrap_or_else(|error| panic!("open context depth {depth}: {error}"));
            context
                .handle_key(VirtualKeyCode::Right, ElementState::Released)
                .unwrap_or_else(|error| panic!("release context depth {depth}: {error}"));
        }
        context
            .set_runtime_flash_message("AAAAAAAAAAAA", RuntimeHelpCharset::Windows1252)
            .expect("install flash beneath context");
        let menu = context.context_menu.take().expect("detach context");
        let flash = context.runtime_flash_message.clone().expect("flash state");
        let mut flash_only = vec![0_u8; 320 * 200 * 4];
        context.render(&mut flash_only).expect("render flash only");
        let mut expected = Surface::new(320, 200, PixelFormat::Rgba8888);
        expected.pixels_mut().copy_from_slice(&flash_only);
        let gamma = context
            .graphics
            .active_gamma_ramp(&context.snapshot.environment.gamma);
        menu.render(&mut expected, Some(&gamma))
            .expect("compose topmost context");
        context.context_menu = Some(menu);
        context.runtime_flash_message = Some(flash);
        let mut actual = vec![0_u8; 320 * 200 * 4];
        context
            .render(&mut actual)
            .expect("render flash below recursive context");
        assert_eq!(actual, expected.pixels());
    }

    #[test]
    fn runtime_f1_recurses_through_every_player_menu_page_and_priority_layer() {
        let every_player_menu_page = || {
            let entry = GoalRuleEntry {
                definition_id: "CLNK".to_string(),
                name: "Entry".to_string(),
                description: None,
                fulfilled: false,
            };
            vec![
                IngameMenuState::main_menu(&MainMenuConditions::default())
                    .expect("default player main menu"),
                IngameMenuState::hostility_menu(&[]),
                IngameMenuState::observer_menu(&[], ObserverTarget::Free),
                IngameMenuState::team_selection_menu(&[TeamSelectionEntry {
                    id: 1,
                    caption: "Team".to_string(),
                    icon_spec: None,
                    color: 0,
                    has_participants: false,
                }]),
                IngameMenuState::goals_menu(std::slice::from_ref(&entry)),
                IngameMenuState::rules_menu(std::slice::from_ref(&entry)),
                IngameMenuState::new_player_menu(&[ingame_menu::NewPlayerEntry {
                    file: "Player.c4p".to_string(),
                    name: "Player".to_string(),
                }]),
                IngameMenuState::savegame_menu(&[SaveSlotState { free: true }; 10]),
                IngameMenuState::options_menu(
                    &OptionFlags {
                        sound: true,
                        music: true,
                        mouse_shown: true,
                        mouse: true,
                    },
                    0,
                ),
                IngameMenuState::display_menu(&DisplayFlags::default(), 0),
                IngameMenuState::surrender_menu(),
                IngameMenuState::client_disconnect_menu(),
                IngameMenuState::host_disconnect_menu(&[HostDisconnectClientEntry {
                    client_id: 0,
                    caption: "Host (Host)".to_string(),
                    activated: true,
                }]),
            ]
        };
        let default_pages = every_player_menu_page();
        let rebound_pages = every_player_menu_page();
        assert_eq!(
            default_pages.len(),
            13,
            "all native C4MainMenu page roots"
        );
        let page_index = |page: ingame_menu::MenuPage| match page {
            ingame_menu::MenuPage::Main => 0,
            ingame_menu::MenuPage::Hostility => 1,
            ingame_menu::MenuPage::Observer => 2,
            ingame_menu::MenuPage::TeamSelection => 3,
            ingame_menu::MenuPage::Goals => 4,
            ingame_menu::MenuPage::Rules => 5,
            ingame_menu::MenuPage::NewPlayer => 6,
            ingame_menu::MenuPage::Savegame => 7,
            ingame_menu::MenuPage::Options => 8,
            ingame_menu::MenuPage::Display => 9,
            ingame_menu::MenuPage::Surrender => 10,
            ingame_menu::MenuPage::ClientDisconnect => 11,
            ingame_menu::MenuPage::HostDisconnect => 12,
        };
        let mut default_app = new_classic_running_sandbox_app();
        let mut rebound_app = new_running_sandbox_app();
        rebound_app
            .bindings
            .rebind(ControlBindingId::Left, VirtualKeyCode::F1);
        rebound_app
            .engine
            .player_mut(rebound_app.local_owner)
            .expect("local player")
            .control
            .control_style = true;
        let mut covered_pages = [false; 13];

        for (default_menu, rebound_menu) in default_pages.into_iter().zip(rebound_pages) {
            let page = default_menu.page();
            covered_pages[page_index(page)] = true;
            assert_eq!(rebound_menu.page(), page);

            default_app
                .ingame_menu
                .replace(default_app.local_owner, Some(default_menu));
            default_app
                .handle_key(VirtualKeyCode::F1, ElementState::Pressed)
                .expect("default F1 toggles above every player-menu page");
            assert!(default_app.runtime_help_visible, "page {page:?}");
            assert_eq!(
                default_app.ingame_menu.as_ref().map(IngameMenuState::page),
                Some(page)
            );
            default_app
                .handle_key(VirtualKeyCode::F1, ElementState::Released)
                .expect("release default help key");
            default_app
                .handle_key(VirtualKeyCode::F1, ElementState::Pressed)
                .expect("hide default help before the next page");
            default_app
                .handle_key(VirtualKeyCode::F1, ElementState::Released)
                .expect("release default help reset");
            assert!(!default_app.runtime_help_visible, "page {page:?}");

            rebound_app
                .ingame_menu
                .replace(rebound_app.local_owner, Some(rebound_menu));
            rebound_app
                .handle_key(VirtualKeyCode::F1, ElementState::Pressed)
                .expect("PRIO_PlrControl owns F1 across every player-menu page");
            assert!(!rebound_app.runtime_help_visible, "page {page:?}");
            assert!(rebound_app.ingame_menu.is_some(), "page {page:?}");
            rebound_app
                .handle_key(VirtualKeyCode::F1, ElementState::Released)
                .expect("release rebound player control");
            assert!(!rebound_app
                .pressed_engine_keys
                .contains(&VirtualKeyCode::F1));
            assert_eq!(
                rebound_app
                    .engine
                    .player(rebound_app.local_owner)
                    .expect("local player")
                    .control
                    .pressed_coms
                    & (1 << clonk_engine::COM_LEFT),
                0
            );
        }
        assert!(covered_pages.into_iter().all(|covered| covered));

        let mut observer = new_classic_running_sandbox_app();
        observer
            .engine
            .remove_player(observer.local_owner)
            .expect("remove local player for ownerless observer menu");
        observer.snapshot = observer.engine.snapshot();
        observer.ingame_menu.replace(
            observer.local_owner,
            IngameMenuState::main_menu(&MainMenuConditions {
                has_player: false,
                player_count: 0,
                ..MainMenuConditions::default()
            }),
        );
        observer
            .bindings
            .rebind(ControlBindingId::Left, VirtualKeyCode::F1);
        observer
            .handle_key(VirtualKeyCode::F1, ElementState::Pressed)
            .expect("ownerless observer menu suppresses player scope, not Generic help");
        assert!(observer.runtime_help_visible);
        assert!(observer.ingame_menu.is_some());

        let mut object = new_running_sandbox_app();
        assert!(object.open_object_menu().expect("open object menu"));
        object
            .bindings
            .rebind(ControlBindingId::Left, VirtualKeyCode::F1);
        object
            .engine
            .player_mut(object.local_owner)
            .expect("local player")
            .control
            .control_style = true;
        object
            .handle_key(VirtualKeyCode::F1, ElementState::Pressed)
            .expect("PRIO_PlrControl owns F1 over object menus");
        assert!(!object.runtime_help_visible);
        assert!(object.object_menu.is_some());

        let mut message = new_running_sandbox_app();
        message
            .push_message_dialog(
                clonk_frontend::message_dialog::MessageDialogState::regular_ok(
                    "Help",
                    "Nonexclusive",
                    clonk_frontend::message_dialog::MessageDialogIcon::NOTIFY,
                ),
                MessageDialogContinuation::None,
            )
            .expect("push nonexclusive message");
        message
            .bindings
            .rebind(ControlBindingId::Left, VirtualKeyCode::F1);
        message
            .handle_key(VirtualKeyCode::F1, ElementState::Pressed)
            .expect("player priority remains above a nonexclusive message");
        assert!(!message.runtime_help_visible);
        assert_eq!(message.message_dialogs.len(), 1);

        let mut context = new_running_sandbox_app();
        context
            .open_context_menu_at(
                vec![ContextMenuEntry::<AppContextMenuCommand>::new(
                    "Remain open",
                )],
                GuiPoint::new(24.0, 24.0),
            )
            .expect("open nonexclusive context");
        context
            .bindings
            .rebind(ControlBindingId::Left, VirtualKeyCode::F1);
        context
            .handle_key(VirtualKeyCode::F1, ElementState::Pressed)
            .expect("player priority remains above a context callback");
        assert!(!context.runtime_help_visible);
        assert!(context.context_menu.is_some());

        let board_script = r#"global func Initialize()
        {
            SetScoreboardData(SBRD_Caption, SBRD_Caption, "Scores");
        }"#;
        let mut default_scoreboard = new_classic_scoreboard_test_app(board_script);
        toggle_scoreboard(&mut default_scoreboard, ModifiersState::empty());
        let mut scoreboard_only = vec![0_u8; 320 * 200 * 4];
        default_scoreboard
            .render(&mut scoreboard_only)
            .expect("render scoreboard before help");
        default_scoreboard
            .handle_key(VirtualKeyCode::F1, ElementState::Pressed)
            .expect("default F1 toggles beneath scoreboard");
        let mut scoreboard_and_help = vec![0_u8; 320 * 200 * 4];
        default_scoreboard
            .render(&mut scoreboard_and_help)
            .expect("render help beneath scoreboard");
        assert!(default_scoreboard.runtime_help_visible);
        assert!(default_scoreboard.scoreboard_dialog.is_some());
        assert_ne!(scoreboard_and_help, scoreboard_only);

        let mut scoreboard = new_scoreboard_test_app(
            r#"global func Initialize()
            {
                SetScoreboardData(SBRD_Caption, SBRD_Caption, "Scores");
            }"#,
        );
        toggle_scoreboard(&mut scoreboard, ModifiersState::empty());
        assert!(scoreboard.scoreboard_dialog.is_some());
        scoreboard
            .bindings
            .rebind(ControlBindingId::Left, VirtualKeyCode::F1);
        scoreboard
            .handle_key(VirtualKeyCode::F1, ElementState::Pressed)
            .expect("player priority remains above the nonexclusive scoreboard");
        assert!(!scoreboard.runtime_help_visible);
        assert!(scoreboard.scoreboard_dialog.is_some());

        let mut save_browser = new_classic_running_sandbox_app();
        save_browser
            .open_save_browser()
            .expect("open app save browser state");
        save_browser
            .handle_key(VirtualKeyCode::F1, ElementState::Pressed)
            .expect("default F1 toggles over save browser");
        assert!(save_browser.runtime_help_visible);
        assert!(save_browser.save_browser.is_some());

        let mut rebound_save_browser = new_running_sandbox_app();
        rebound_save_browser
            .open_save_browser()
            .expect("open rebound save browser state");
        rebound_save_browser
            .bindings
            .rebind(ControlBindingId::Left, VirtualKeyCode::F1);
        rebound_save_browser
            .handle_key(VirtualKeyCode::F1, ElementState::Pressed)
            .expect("player priority remains active over save browser");
        assert!(!rebound_save_browser.runtime_help_visible);
        assert!(rebound_save_browser.save_browser.is_some());

        let mut game_over = new_game_over_keyboard_app();
        game_over
            .bindings
            .rebind(ControlBindingId::Left, VirtualKeyCode::F1);
        game_over
            .handle_key(VirtualKeyCode::F1, ElementState::Pressed)
            .expect("exclusive evaluation suppresses player control but not Generic help");
        assert!(game_over.runtime_help_visible);
    }

    #[test]
    fn running_context_menu_renders_above_runtime_f1_help() {
        let mut app = new_classic_running_sandbox_app();
        app.status_text.clear();
        app.snapshot.hud.messages.clear();
        let mut baseline = vec![0_u8; 320 * 200 * 4];
        app.render(&mut baseline).expect("render running baseline");
        app.open_context_menu_at(
            vec![ContextMenuEntry::<AppContextMenuCommand>::new(
                "Context above help",
            )],
            GuiPoint::new(120.0, 105.0),
        )
        .expect("open running context menu");
        let mut context_only = vec![0_u8; 320 * 200 * 4];
        app.render(&mut context_only)
            .expect("render visible running context");
        assert_ne!(context_only, baseline, "running context must draw pixels");

        app.handle_key(VirtualKeyCode::F1, ElementState::Pressed)
            .expect("toggle help beneath context");
        let context = app.context_menu.take().expect("detach running context");
        let mut help_only = vec![0_u8; 320 * 200 * 4];
        app.render(&mut help_only)
            .expect("render help without context");
        let mut expected = Surface::new(320, 200, PixelFormat::Rgba8888);
        expected.pixels_mut().copy_from_slice(&help_only);
        let gamma = app
            .graphics
            .active_gamma_ramp(&app.snapshot.environment.gamma);
        context
            .render(&mut expected, Some(&gamma))
            .expect("compose expected topmost context");
        app.context_menu = Some(context);
        let mut help_and_context = vec![0_u8; 320 * 200 * 4];
        app.render(&mut help_and_context)
            .expect("render help below running context");
        assert_ne!(
            help_and_context, context_only,
            "help remains visible outside the panel"
        );
        assert_eq!(
            help_and_context,
            expected.pixels(),
            "running render must compose the context after F1 help"
        );
    }

    #[test]
    fn runtime_f1_recurses_through_all_engine_menu_styles_and_progress_states() {
        let mut app = new_classic_running_sandbox_app();
        let mut rebound = new_classic_running_sandbox_app();
        rebound
            .bindings
            .rebind(ControlBindingId::Left, VirtualKeyCode::F1);
        rebound
            .engine
            .player_mut(rebound.local_owner)
            .expect("local rebound player")
            .control
            .control_style = true;
        let mut menu_only = vec![0_u8; 320 * 200 * 4];
        let mut menu_and_help = vec![0_u8; 320 * 200 * 4];
        for style in 0..=3 {
            for text_progressing in [false, true] {
                let cursor = app
                    .engine
                    .crew_cursor(app.local_owner)
                    .expect("sandbox cursor");
                let mut menu = two_item_script_menu(cursor);
                menu.style = style;
                menu.text_progressing = text_progressing;
                app.engine
                    .apply_object_update(
                        cursor,
                        ObjectUpdate {
                            menu: Some(Some(menu)),
                            ..ObjectUpdate::default()
                        },
                    )
                    .expect("install engine menu style");
                app.snapshot = app.engine.snapshot();
                menu_only.fill(0);
                app.render(&mut menu_only)
                    .expect("render engine menu before F1");
                app.handle_key(VirtualKeyCode::F1, ElementState::Pressed)
                    .expect("default F1 toggles over engine menu");
                menu_and_help.fill(0);
                app.render(&mut menu_and_help)
                    .expect("render F1 above engine menu");
                assert!(
                    app.runtime_help_visible,
                    "style {style}, progress {text_progressing}"
                );
                assert_ne!(menu_and_help, menu_only);
                assert!(app.engine.cursor_object_menu(app.local_owner).is_some());
                app.handle_key(VirtualKeyCode::F1, ElementState::Released)
                    .expect("release default help key");
                app.handle_key(VirtualKeyCode::F1, ElementState::Pressed)
                    .expect("hide default help before the next engine menu");
                app.handle_key(VirtualKeyCode::F1, ElementState::Released)
                    .expect("release default help reset");
                assert!(!app.runtime_help_visible);

                let rebound_cursor = rebound
                    .engine
                    .crew_cursor(rebound.local_owner)
                    .expect("rebound sandbox cursor");
                let mut rebound_menu = two_item_script_menu(rebound_cursor);
                rebound_menu.style = style;
                rebound_menu.text_progressing = text_progressing;
                rebound
                    .engine
                    .apply_object_update(
                        rebound_cursor,
                        ObjectUpdate {
                            menu: Some(Some(rebound_menu)),
                            ..ObjectUpdate::default()
                        },
                    )
                    .expect("install rebound engine menu style");
                rebound.snapshot = rebound.engine.snapshot();
                rebound
                    .handle_key(VirtualKeyCode::F1, ElementState::Pressed)
                    .expect("player F1 owns every engine menu style");
                assert!(!rebound.runtime_help_visible);
                assert!(rebound
                    .engine
                    .cursor_object_menu(rebound.local_owner)
                    .is_some());
                rebound
                    .handle_key(VirtualKeyCode::F1, ElementState::Released)
                    .expect("release rebound player control");
                assert!(!rebound.pressed_engine_keys.contains(&VirtualKeyCode::F1));
                assert_eq!(
                    rebound
                        .engine
                        .player(rebound.local_owner)
                        .expect("local rebound player")
                        .control
                        .pressed_coms
                        & (1 << clonk_engine::COM_LEFT),
                    0
                );
            }
        }
    }

    #[test]
    fn runtime_f4_gamepad_high_requires_active_dialog_and_other_input_reaches_gameplay() {
        let mut active = new_running_sandbox_app();
        let (_events, mut commands) = install_running_network_stub(&mut active, 0, 40, 4);
        route_primary_gamepad_to_local_owner(&mut active);
        active
            .handle_key(VirtualKeyCode::F4, ElementState::Pressed)
            .expect("open active runtime F4 dialog");
        assert!(active.runtime_client_list_strong_gamepad_callback_is_active());
        assert!(active.runtime_client_list_draw_active());

        active
            .process_gamepad_event_batch([
                GamepadEvent::Axis {
                    slot: GamepadSlot::new(0),
                    axis: LegacyGamepadAxis::new(0, true),
                    state: ElementState::Pressed,
                },
                GamepadEvent::Direction {
                    slot: GamepadSlot::new(0),
                    button: ControlButton::Right,
                    state: ElementState::Pressed,
                },
            ])
            .expect("normal F4 leaves player-control gamepad directions in base scope");
        let submitted = commands.take_submitted_local();
        assert_eq!(submitted.len(), 1);
        assert!(matches!(
            submitted[0].1,
            ControlEvent::Press(ControlButton::Right)
        ));

        active
            .process_gamepad_event_batch([
                GamepadEvent::GuiButton {
                    slot: GamepadSlot::new(0),
                    class: GuiButtonClass::High,
                    state: ElementState::Pressed,
                },
                GamepadEvent::Action {
                    slot: GamepadSlot::new(0),
                    action: GamepadActionType::MenuToggle,
                    state: ElementState::Pressed,
                },
            ])
            .expect("active F4 strong High callback owns its raw alias cluster");
        assert!(active.runtime_client_list.is_none());
        assert!(active.ingame_menu.is_none());
        assert!(commands.take_submitted_local().is_empty());

        let mut inactive = new_running_sandbox_app();
        configure_runtime_network_role(&mut inactive, RuntimeNetworkRole::Host);
        inactive
            .push_message_dialog(
                clonk_frontend::message_dialog::MessageDialogState::regular_ok(
                    "Notice",
                    "The older F4 dialog remains inactive",
                    clonk_frontend::message_dialog::MessageDialogIcon::NOTIFY,
                ),
                MessageDialogContinuation::None,
            )
            .expect("show ordinary dialog before F4");
        inactive
            .handle_key(VirtualKeyCode::F4, ElementState::Pressed)
            .expect("insert F4 below the existing z+1 message");
        assert!(!inactive.runtime_client_list_strong_gamepad_callback_is_active());
        assert!(!inactive.runtime_client_list_draw_active());
        inactive
            .process_gamepad_event_batch([GamepadEvent::GuiButton {
                slot: GamepadSlot::new(0),
                class: GuiButtonClass::High,
                state: ElementState::Pressed,
            }])
            .expect("inactive F4 has no strong High callback");
        assert!(inactive.runtime_client_list.is_some());
        assert_eq!(inactive.message_dialogs.len(), 1);
    }

    #[test]
    fn running_only_globals_are_excluded_from_menu_and_loading_modes() {
        let mut menu = new_menu_app(320, 200);
        for key in [
            VirtualKeyCode::F1,
            VirtualKeyCode::F4,
            VirtualKeyCode::Pause,
        ] {
            menu.handle_key(key, ElementState::Pressed)
                .expect("running-only global key is not registered in Menu mode");
            menu.handle_key(key, ElementState::Released)
                .expect("release remains outside the running-only global helper");
        }
        menu.handle_modifiers_changed(ModifiersState::ALT)
            .expect("set menu Alt modifier");
        menu.handle_key(VirtualKeyCode::C, ElementState::Pressed)
            .expect("runtime IRC frontend is not registered in Menu mode");
        menu.handle_key(VirtualKeyCode::C, ElementState::Released)
            .expect("menu IRC chord release remains outside the runtime helper");

        let mut loading = new_running_sandbox_app();
        loading.mode = AppMode::Loading;
        for key in [
            VirtualKeyCode::F1,
            VirtualKeyCode::F4,
            VirtualKeyCode::Pause,
        ] {
            loading
                .handle_key(key, ElementState::Pressed)
                .expect("running-only global key is not registered in Loading mode");
            loading
                .handle_key(key, ElementState::Released)
                .expect("release remains outside the running-only global helper");
        }
        loading
            .handle_modifiers_changed(ModifiersState::ALT)
            .expect("set loading Alt modifier");
        loading
            .handle_key(VirtualKeyCode::C, ElementState::Pressed)
            .expect("runtime IRC frontend is not registered in Loading mode");
        loading
            .handle_key(VirtualKeyCode::C, ElementState::Released)
            .expect("loading IRC chord release remains outside the runtime helper");
    }

    #[test]
    fn l019_window_close_confirms_running_round_and_nonrunning_close_exits() {
        let mut app = new_running_sandbox_app();
        app.update().expect("advance round before declining close");
        let running_frame = app.engine.frame();
        let running_scenario = app
            .active_scenario
            .as_ref()
            .expect("active sandbox scenario")
            .identifier
            .clone();

        app.handle_window_close_requested();
        assert!(app.message_dialogs.last().is_some_and(|dialog| matches!(
            dialog.continuation,
            MessageDialogContinuation::AbortGame { .. }
        )));
        assert!(!app.take_exit_request());
        finish_abort_dialog(
            &mut app,
            clonk_frontend::message_dialog::MessageDialogResult::No,
        );
        assert!(matches!(app.mode, AppMode::Running));
        assert_eq!(app.engine.frame(), running_frame);
        assert_eq!(
            app.active_scenario
                .as_ref()
                .map(|scenario| scenario.identifier.as_str()),
            Some(running_scenario.as_str())
        );

        app.handle_window_close_requested();
        finish_abort_dialog(
            &mut app,
            clonk_frontend::message_dialog::MessageDialogResult::Yes,
        );
        assert!(matches!(app.mode, AppMode::Menu));
        assert!(app.active_scenario.is_none());
        assert!(!app.take_exit_request(), "Yes ends the round, not the process");

        app.handle_window_close_requested();
        assert!(
            app.take_exit_request(),
            "the window-event footer turns this into ControlFlow::Exit so dirty display options persist"
        );

        let mut loading = new_running_sandbox_app();
        loading.mode = AppMode::Loading;
        loading.handle_window_close_requested();
        assert!(loading.take_exit_request());
        assert!(loading.ingame_menu.is_none());
        assert!(loading.message_dialogs.is_empty());
    }

    #[test]
    fn l019_window_close_uses_observer_owner_and_never_exits_on_dialog_refusal() {
        let mut observer = new_running_sandbox_app();
        let removed_owner = observer.local_owner;
        observer
            .engine
            .remove_player(removed_owner)
            .expect("remove local player for passive observer");
        observer.engine.set_local_players([]);
        observer.local_controls = LocalControlRegistry::default();
        observer.snapshot = observer.engine.snapshot();
        observer.refresh_non_authoritative_physical_viewports();
        assert!(observer.primary_physical_viewport_is_no_owner());

        observer.handle_window_close_requested();
        observer.handle_window_close_requested();
        assert!(observer.ingame_menu.is_none());
        assert_eq!(observer.message_dialogs.len(), 1);
        assert!(matches!(
            observer.message_dialogs[0].continuation,
            MessageDialogContinuation::AbortGame { .. }
        ));
        assert!(!observer.take_exit_request());

        let mut game_over = new_game_over_keyboard_app();
        game_over.handle_window_close_requested();
        assert!(game_over.game_over_dialog.is_some());
        assert!(game_over.ingame_menu.is_none());
        assert!(game_over.message_dialogs.is_empty());
        assert!(!game_over.take_exit_request());
    }

    #[test]
    fn l002_bare_escape_opens_abort_confirmation_without_exiting() {
        clonk_logging::init();
        let mut app = new_running_sandbox_app();
        app.status_text.clear();
        app.handle_key(VirtualKeyCode::Escape, ElementState::Pressed)
            .expect("bare Escape opens C4AbortGameDialog");

        assert!(app.message_dialogs.last().is_some_and(|dialog| matches!(
            dialog.continuation,
            MessageDialogContinuation::AbortGame { .. }
        )));
        assert!(app.object_menu.is_none());
        assert!(matches!(app.mode, AppMode::Running));
        assert!(!app.take_exit_request());
        assert!(app.status_text.is_empty());
        assert!(!app.show_abort_dialog(app.local_owner));
        assert_eq!(app.message_dialogs.len(), 1);
    }

    #[test]
    fn abort_dialog_uses_stacked_halt_and_preserves_prior_pause() {
        let mut unpaused = new_running_sandbox_app();
        assert_eq!(unpaused.offline_halt_count, 0);
        assert!(unpaused.show_abort_dialog(unpaused.local_owner));
        assert_eq!(unpaused.offline_halt_count, 1);
        assert!(
            !unpaused.show_abort_dialog(unpaused.local_owner),
            "the singleton abort dialog cannot acquire a second halt lease"
        );
        assert_eq!(unpaused.offline_halt_count, 1);
        finish_abort_dialog(
            &mut unpaused,
            clonk_frontend::message_dialog::MessageDialogResult::No,
        );
        assert_eq!(unpaused.offline_halt_count, 0);

        let mut app = new_running_sandbox_app();
        app.set_runtime_pause(true);
        assert_eq!(app.offline_halt_count, 1);
        app.engine
            .player_mut(app.local_owner)
            .expect("local player")
            .control
            .pressed_coms = 1 << clonk_engine::COM_LEFT;
        let frozen_frame = app.engine.frame();

        assert!(app.show_abort_dialog(app.local_owner));
        assert_eq!(app.offline_halt_count, 2);
        assert!(app.runtime_halt_active());
        app.update().expect("stacked halt keeps the app loop live");
        assert_eq!(app.engine.frame(), frozen_frame);

        finish_abort_dialog(
            &mut app,
            clonk_frontend::message_dialog::MessageDialogResult::No,
        );
        assert_eq!(app.offline_halt_count, 1);
        assert!(app.runtime_halt_active(), "the prior pause remains owned");
        assert_eq!(
            app.engine
                .player(app.local_owner)
                .expect("local player")
                .control
                .pressed_coms,
            0,
            "decline clears every local player's pressed commands"
        );

        assert!(app.show_abort_dialog(app.local_owner));
        assert_eq!(app.offline_halt_count, 2);
        let index = app.message_dialogs.len() - 1;
        app.remove_message_dialog_at(index)
            .expect("silent dialog removal releases its captured lease");
        assert_eq!(app.offline_halt_count, 1);
        app.set_runtime_pause(false);
        assert_eq!(app.offline_halt_count, 0);

        let mut network = new_running_sandbox_app();
        let (_events, _commands) = install_running_network_stub(&mut network, 0, 0, 1);
        assert!(network.show_abort_dialog(network.local_owner));
        assert_eq!(network.offline_halt_count, 0);
        finish_abort_dialog(
            &mut network,
            clonk_frontend::message_dialog::MessageDialogResult::No,
        );
        assert_eq!(network.offline_halt_count, 0);
    }
