    // Spliced into `mod tests` (src/main_tests.rs) via include!: a bare item
    // sequence, not a child module, so test ids stay `tests::<fn>`.

    #[cfg(unix)]
    #[test]
    fn definition_graphics_route_reopens_a_projected_non_ascii_path() {
        use std::os::unix::ffi::OsStringExt as _;

        let _lock = env_lock().lock();
        let user_data = tempdir().expect("native definition user data");
        let content = tempdir().expect("native definition content");
        let (_guard, paths) = exact_loader_test_paths(user_data.path(), Some(content.path()));
        let definition = content
            .path()
            .join(OsString::from_vec(b"Native-\xe2\x98\x83.c4d".to_vec()));
        fs::create_dir_all(&definition).expect("native-byte definition group");
        let scenario = content.path().join("NativePath.c4s");
        fs::create_dir_all(&scenario).expect("native-byte scenario group");
        fs::write(scenario.join("Scenario.txt"), "[Head]\nTitle=Native Path\n")
            .expect("native-byte scenario core");
        let scenario_group = Group::open(&scenario).expect("open native-byte scenario");
        let head = ScenarioLoaderHead::load_from_group(&scenario_group).expect("load head");
        let registrations = definition_graphics_source_registrations(
            &head,
            &scenario_group,
            &ScenarioDefinitionLoad::Fixed {
                modules: vec![path_as_legacy_text(&definition)],
                definition_root: None,
            },
            &paths,
            0,
        )
        .expect("graphics route reopens projected native path");
        assert_eq!(registrations[0].group.root(), definition.as_path());
    }

    #[test]
    fn definition_path_directory_probe_applies_both_cpp_trailing_removals() {
        let separator = std::path::MAIN_SEPARATOR as u8;
        assert_eq!(definition_path_directory_probe(&[separator]), None);
        assert_eq!(
            definition_path_directory_probe(&[separator, separator]),
            None
        );
        assert_eq!(
            definition_path_directory_probe(&[b'D', b'e', b'f', b's', separator]),
            Some(b"Defs".to_vec())
        );
        let alternate = if separator == b'/' { b'\\' } else { b'/' };
        assert_eq!(
            definition_path_directory_probe(&[b'D', b'e', b'f', b's', alternate]),
            Some(b"Defs".to_vec())
        );
    }

    #[test]
    fn hud_inventory_left_click_queues_exact_contents_only() {
        let (mut app, owner, _crew, _first, target, region_point) = inventory_region_fixture();
        let click_point = GuiPoint::new(region_point.x, region_point.y - 14.0);
        assert_eq!(
            app.ingame_viewport_region(owner, click_point),
            Some(IngameViewportRegion::Inventory(target))
        );
        let behind = app
            .graphics
            .viewport_point_at(click_point)
            .map(ingame_pointer_world_pixel)
            .expect("HUD point has a world position behind it");
        let mut selectable = Definition::from_script("MHIT", "HUD overlap", "#strict\n")
            .expect("selectable definition compiles");
        selectable.set_category(clonk_engine::CATEGORY_OBJECT | clonk_engine::CATEGORY_MOUSE_SELECT);
        selectable.set_collectible(true);
        selectable.set_shape_rect(Some(clonk_engine::DefinitionRect::new(-8, -8, 16, 16)));
        app.engine
            .register_definition(selectable)
            .expect("register selectable overlap");
        let cursor_layer = app
            .engine
            .crew_cursor(owner)
            .and_then(|cursor| app.engine.object_snapshot(cursor))
            .and_then(|cursor| cursor.layer);
        let mut overlap_spawn = SpawnConfig::new("MHIT")
            .with_position(behind)
            .with_owner(owner);
        if let Some(layer) = cursor_layer {
            overlap_spawn = overlap_spawn.with_layer(layer);
        }
        let overlap = app
            .engine
            .spawn_object(overlap_spawn)
            .expect("spawn selectable object behind HUD");
        app.engine
            .apply_object_update(overlap, ObjectUpdate::new().with_position(behind))
            .expect("pin selectable object behind HUD");
        app.snapshot = app.engine.snapshot();
        let mut frame = vec![0_u8; 320 * 200 * 4];
        app.render(&mut frame).expect("render overlap behind HUD");
        assert_eq!(
                app.ingame_mouse_select_target(owner, click_point),
                Some(overlap),
                "fixture must catch a leaked selection; overlap={:?}, projected={:?}, behind={behind:?}, region={click_point:?}, raw_pick={:?}",
                app.snapshot.object(overlap),
                app.graphics.world_to_screen(owner, behind),
                app.graphics.object_at_point(&app.snapshot, owner, click_point),
            );
        let (manager, _events, mut network_commands) =
            NetworkManager::test_stub_with_commands_for_client_id(7);
        app.network = Some(manager);
        let tick = app.local_control_submission_tick();

        app.handle_cursor_moved(PhysicalPosition::new(
            f64::from(click_point.x),
            f64::from(click_point.y),
        ))
        .expect("move onto inventory region");
        app.handle_ingame_mouse_button(ElementState::Pressed)
            .expect("inventory left-down");
        assert_eq!(
            network_commands.take_submitted_player_inputs(),
            (Vec::new(), Vec::new(), Vec::new()),
            "classic control queues nothing until button-up"
        );
        app.handle_ingame_mouse_button(ElementState::Released)
            .expect("inventory left-up");

        let (controls, commands, selections) = network_commands.take_submitted_player_inputs();
        assert_eq!(
            controls,
            vec![(
                owner,
                ControlEvent::RawPlayerControl {
                    command: 9,
                    data: target.as_u64() as i32,
                },
                tick,
            )]
        );
        assert!(commands.is_empty(), "HUD click must not queue MoveTo");
        assert!(
            selections.is_empty(),
            "HUD click must not select world crew"
        );
    }

    #[test]
    fn hud_inventory_autostop_queues_stored_press_and_release() {
        let (mut app, owner, _crew, _first, target, region_point) = inventory_region_fixture();
        app.engine
            .player_mut(owner)
            .expect("local player")
            .control
            .control_style = true;
        let viewport = app
            .graphics
            .viewport_rect(owner)
            .expect("local sandbox viewport");
        let down = GuiPoint::new(
            (viewport.x + clonk_frontend::hud::SYMBOL_BORDER + 1) as f32,
            region_point.y,
        );
        let outside = GuiPoint::new(down.x - 3.0, down.y);
        assert_eq!(
            app.ingame_viewport_region(owner, down),
            Some(IngameViewportRegion::Inventory(target))
        );
        assert_eq!(app.ingame_viewport_region(owner, outside), None);
        let (manager, _events, mut network_commands) =
            NetworkManager::test_stub_with_commands_for_client_id(7);
        app.network = Some(manager);
        let tick = app.local_control_submission_tick();

        app.handle_cursor_moved(PhysicalPosition::new(f64::from(down.x), f64::from(down.y)))
            .expect("move onto inventory edge");
        app.handle_ingame_mouse_button(ElementState::Pressed)
            .expect("AutoStop inventory left-down");
        let (controls, commands, selections) = network_commands.take_submitted_player_inputs();
        assert_eq!(
            controls,
            vec![(
                owner,
                ControlEvent::RawPlayerControl {
                    command: 9,
                    data: target.as_u64() as i32,
                },
                tick,
            )]
        );
        assert!(commands.is_empty());
        assert!(selections.is_empty());

        app.handle_cursor_moved(PhysicalPosition::new(
            f64::from(outside.x),
            f64::from(outside.y),
        ))
        .expect("move just outside the region");
        assert!(
            app.mouse_state.is_some_and(|state| !state.motion.moved),
            "three pixels must remain Drag_None"
        );
        app.handle_ingame_mouse_button(ElementState::Released)
            .expect("AutoStop inventory left-up");
        let (controls, commands, selections) = network_commands.take_submitted_player_inputs();
        assert_eq!(
            controls,
            vec![(
                owner,
                ControlEvent::RawPlayerControl {
                    command: 25,
                    data: target.as_u64() as i32,
                },
                tick,
            )],
            "COM_Contents+16 retains the down-time target number"
        );
        assert!(commands.is_empty());
        assert!(selections.is_empty());
    }

    #[test]
    fn definition_sprite_carries_the_raw_defcore_picture_rect() {
        // C4GraphicsOverlay::UpdateFacet takes pSourceGfx->pDef->PictureRect
        // verbatim for MODE_IngamePicture/MODE_Picture
        // (src/C4DefGraphics.cpp:660-664). It must arrive UNSCALED: C4Facet::DrawT
        // applies the definition Scale to the source crop only
        // (src/C4Facet.cpp:74-79), and the rect also serves as the fZoomToShape
        // denominator and the destination extent.
        let mut app = new_running_sandbox_app();

        let mut with_picture = Definition::from_script("PIC2", "Pictured", "#strict\n")
            .expect("pictured definition compiles");
        with_picture.set_shape_rect(Some(clonk_engine::DefinitionRect::new(-8, -10, 16, 20)));
        with_picture.set_picture(Some(clonk_engine::DefinitionPicture {
            x: 192,
            y: 100,
            width: 32,
            height: 40,
        }));
        with_picture.set_sprite_image(Some(clonk_engine::DefinitionSpriteImage {
            width: 1,
            height: 1,
            pixels: Arc::from([0xff, 0, 0, 0xff]),
            color_mask: None,
        }));
        app.engine
            .register_definition(with_picture)
            .expect("register pictured definition");

        // C4DefCore::Load replaces a missing Picture with the shape-sized
        // top-left facet, ignoring the shape offsets (src/C4Def.cpp:222-224).
        let mut without_picture = Definition::from_script("PIC0", "Unpictured", "#strict\n")
            .expect("unpictured definition compiles");
        without_picture.set_shape_rect(Some(clonk_engine::DefinitionRect::new(-8, -10, 16, 20)));
        without_picture.set_picture(Some(clonk_engine::DefinitionPicture {
            x: 0,
            y: 0,
            width: 16,
            height: 20,
        }));
        without_picture.set_sprite_image(Some(clonk_engine::DefinitionSpriteImage {
            width: 1,
            height: 1,
            pixels: Arc::from([0xff, 0, 0, 0xff]),
            color_mask: None,
        }));
        app.engine
            .register_definition(without_picture)
            .expect("register unpictured definition");

        app.rebuild_definition_sprites();

        let picture_of = |id: &str| {
            app.graphics
                .object_sprite(&sprite_map_key(id, None))
                .expect("definition sprite is installed")
                .picture
        };
        assert_eq!(
            picture_of("PIC2"),
            Some(clonk_engine::DefinitionRect::new(192, 100, 32, 40)),
        );
        assert_eq!(
            picture_of("PIC0"),
            Some(clonk_engine::DefinitionRect::new(0, 0, 16, 20)),
        );
    }

    #[test]
    fn running_graphics_recreation_keeps_script_particle_catalog() {
        // FI5B has a deliberately transparent object facet. Its shipped
        // Flying callback presents the launched flame exclusively through
        // global Fire2 particles (Flamethrower.c4d/Fire.c4d/Script.c:20-46).
        // C++ keeps the loaded particle definitions in Game.Particles across
        // viewport recreation and draws GlobalParticles after normal objects
        // (oracle-src-pinned src/C4Particles.cpp:118-189;
        // src/C4Viewport.cpp:1071-1079).
        let mut app = new_running_sandbox_app();
        let flame = ResourceParticleDefinition {
            core: clonk_resources::ParticleDefinitionCore {
                name: "Fire2".to_string(),
                init_fn: "StdInit".to_string(),
                exec_fn: "StdExec".to_string(),
                draw_fn: "Std".to_string(),
                additive: 1,
                attach: 1,
                alpha_fade: 10,
                ..clonk_resources::ParticleDefinitionCore::default()
            },
            image: GraphicsImage::new(1, 1, vec![255, 96, 0, 255]),
            facet: clonk_resources::ParticleFacet {
                width: 1,
                height: 1,
                ..clonk_resources::ParticleFacet::default()
            },
        };
        app.engine
            .register_particle_resource(&flame)
            .expect("register Fire2 render fixture");
        app.rebuild_definition_sprites();
        assert!(
            app.graphics.particle_sprite("Fire2").is_some(),
            "precondition: the scenario definition rebuild installs Fire2"
        );

        let label = app.scenario_label.clone();
        let ground = app.fallback_ground;
        app.configure_running_state(label, ground);
        assert!(
            app.graphics.particle_sprite("Fire2").is_some(),
            "entering the running presentation must retain script particle graphics"
        );

        app.resize(321, 201).expect("resize running presentation");
        assert!(
            app.graphics.particle_sprite("Fire2").is_some(),
            "resizing the running presentation must retain script particle graphics"
        );
    }

    #[test]
    fn viewport_buttons_use_only_the_exact_mouse_viewport() {
        let mut app = new_running_sandbox_app();
        let owner = app.local_owner;
        let focus = app.engine.crew_cursor(owner).expect("sandbox cursor");
        app.engine
            .replace_player_viewports(
                owner,
                vec![
                    clonk_engine::PlayerViewport::new(Vector2::new(240, 180)).with_focus(Some(focus)),
                    clonk_engine::PlayerViewport::new(Vector2::new(720, 180)).with_focus(Some(focus)),
                ],
            )
            .expect("install same-owner split viewports");
        render_mouse_test_app(&mut app);

        let viewports = app
            .graphics
            .active_viewport_projections()
            .into_iter()
            .filter(|viewport| viewport.owner == owner)
            .collect::<Vec<_>>();
        assert_eq!(viewports.len(), 2);
        let point = |viewport: ActiveViewportProjection| {
            let rect = clonk_frontend::hud::viewport_button_rect(
                viewport.rect,
                clonk_frontend::hud::ViewportButton::Help,
            );
            GuiPoint::new(rect.x as f32 + 1.0, rect.y as f32 + 1.0)
        };
        assert_eq!(
            app.ingame_viewport_region(owner, point(viewports[0])),
            Some(IngameViewportRegion::ViewportButton(
                clonk_frontend::hud::ViewportButton::Help,
            ))
        );
        assert_eq!(
            app.ingame_viewport_region(owner, point(viewports[1])),
            None,
            "the same player's second viewport gets only the keyboard menu hint"
        );
    }

    #[test]
    fn viewport_button_stack_is_wired_into_the_late_app_render() {
        let mut app = new_classic_running_sandbox_app();
        app.display_flags.show_commands = false;
        app.display_flags.show_command_keys = false;
        render_mouse_test_app(&mut app);

        let viewport = app
            .active_ingame_mouse_viewport()
            .expect("sandbox mouse viewport");
        let gamma = app
            .graphics
            .active_gamma_ramp(&app.snapshot.environment.gamma);
        app.graphics.update_overlay(&GraphicsOverlay {
            frame_text: "",
            status_text: "",
            debug_hud: false,
            viewport_overlays_visible: true,
            players: Vec::new(),
            game_time_seconds: 0,
            message_board: MessageBoardOverlay::default(),
            crew_name_labels: Vec::new(),
            clock_text: None,
            frames_per_second: None,
            upper_board_mode: clonk_frontend::hud::UpperBoardMode::Full,
            show_portraits: false,
            show_commands: true,
            show_command_keys: false,
        });
        app.graphics.surface_mut().fill(Color::transparent());
        app.graphics
            .draw_viewport_control_overlays(Some(viewport.index), false, None, Some(&gamma));
        let isolated = app.graphics.surface().pixels().to_vec();

        app.display_flags.show_commands = true;
        render_mouse_test_app(&mut app);
        let rendered = app.graphics.surface().pixels();
        let width = app.graphics.surface().width() as usize;
        for button in [
            clonk_frontend::hud::ViewportButton::Help,
            clonk_frontend::hud::ViewportButton::PlayerMenu,
        ] {
            let rect = clonk_frontend::hud::viewport_button_rect(viewport.rect, button);
            let mut opaque_pixels = 0;
            for y in rect.y..rect.y + rect.height as i32 {
                for x in rect.x..rect.x + rect.width as i32 {
                    let index = (y as usize * width + x as usize) * 4;
                    if isolated[index + 3] == u8::MAX {
                        opaque_pixels += 1;
                        assert_eq!(
                            &rendered[index..index + 4],
                            &isolated[index..index + 4],
                            "late app render omitted or obscured {button:?} at ({x},{y})"
                        );
                    }
                }
            }
            assert!(
                opaque_pixels > 100,
                "the isolated {button:?} control must contain a substantial opaque facet"
            );
        }
    }

    #[test]
    fn hud_command_bar_left_click_queues_exact_drawn_coms_only() {
        let (mut app, owner, points) = command_bar_fixture(false);
        let (manager, _events, mut network_commands) =
            NetworkManager::test_stub_with_commands_for_client_id(7);
        app.network = Some(manager);

        for (command, point) in points {
            app.ingame_last_left_down = None;
            app.handle_cursor_moved(PhysicalPosition::new(
                f64::from(point.x),
                f64::from(point.y),
            ))
            .expect("move onto command region");
            let tick = app.local_control_submission_tick();
            app.handle_ingame_mouse_button(ElementState::Pressed)
                .expect("command left-down");
            assert_eq!(
                network_commands.take_submitted_player_inputs(),
                (Vec::new(), Vec::new(), Vec::new()),
                "classic COM {command} waits for button-up"
            );
            app.handle_ingame_mouse_button(ElementState::Released)
                .expect("command left-up");
            let (controls, commands, selections) = network_commands.take_submitted_player_inputs();
            assert_eq!(
                controls,
                vec![(
                    owner,
                    ControlEvent::RawPlayerControl { command, data: 0 },
                    tick,
                )]
            );
            assert!(commands.is_empty(), "COM {command} leaked a world command");
            assert!(
                selections.is_empty(),
                "COM {command} leaked world selection"
            );
        }
    }

    #[test]
    fn selection_drag_entering_hud_region_is_cancelled() {
        let (mut app, owner, _crew, _first, _target, region_point) = inventory_region_fixture();
        let viewport = app.graphics.viewport_rect(owner).expect("local viewport");
        let (start, crossed) = (viewport.y + 12..viewport.y + viewport.height as i32 - 48)
            .step_by(4)
            .flat_map(|y| {
                (viewport.x + 12..viewport.x + viewport.width as i32 - 36)
                    .step_by(4)
                    .map(move |x| {
                        (
                            GuiPoint::new(x as f32, y as f32),
                            GuiPoint::new((x + 12) as f32, (y + 8) as f32),
                        )
                    })
            })
            .find(|(start, crossed)| {
                [*start, *crossed].into_iter().all(|point| {
                    app.graphics
                        .viewport_point_at(point)
                        .is_some_and(|pointer| pointer.owner == owner)
                        && app
                            .graphics
                            .object_at_point(&app.snapshot, owner, point)
                            .is_none()
                        && app.ingame_viewport_region(owner, point).is_none()
                })
            })
            .expect("empty landscape selection-frame points");
        let (manager, _events, mut network_commands) =
            NetworkManager::test_stub_with_commands_for_client_id(7);
        app.network = Some(manager);

        app.handle_cursor_moved(PhysicalPosition::new(
            f64::from(start.x),
            f64::from(start.y),
        ))
        .expect("move to frame start");
        app.handle_ingame_mouse_button(ElementState::Pressed)
            .expect("landscape left-down");
        app.handle_cursor_moved(PhysicalPosition::new(
            f64::from(crossed.x),
            f64::from(crossed.y),
        ))
        .expect("cross drag threshold");
        assert!(app
            .mouse_state
            .is_some_and(|state| state.motion.moved && state.motion.selection_frame));

        app.handle_cursor_moved(PhysicalPosition::new(
            f64::from(region_point.x),
            f64::from(region_point.y),
        ))
        .expect("enter inventory region");
        assert!(app.ingame_selection_frame().is_none());
        assert!(app.mouse_state.is_some_and(|state| {
            !state.motion.selection_frame && state.motion.selection_cancelled_by_region
        }));
        app.handle_ingame_mouse_button(ElementState::Released)
            .expect("release cancelled frame over region");
        let (controls, commands, selections) = network_commands.take_submitted_player_inputs();
        assert_eq!(
            controls,
            vec![(
                owner,
                ControlEvent::RawPlayerControl {
                    command: 0,
                    data: 0,
                },
                app.local_control_submission_tick(),
            )],
            "C++ sends the default copied DownRegion after cancellation"
        );
        assert!(commands.is_empty());
        assert!(selections.is_empty());
    }

    #[test]
    fn l013_full_speed_runs_unpaced_skips_requested_renders_and_slow_restores_timer() {
        let mut app = new_running_sandbox_app();
        let mut schedule = frame_schedule_for_mode(
            app.mode,
            app.engine.game_tick_delay_ms(),
            app.engine.game_tick_delay_revision(),
            app.max_refresh_delay_ms,
        );
        let mut accumulator = Duration::ZERO;
        let first_frame = app.engine.frame();
        app.full_speed = true;
        app.frame_skip = 3;

        for offset in 1..=6 {
            let outcome = advance_simulation_pass(&mut app, &mut schedule, &mut accumulator)
                .expect("execute one unpaced FullSpeed pass");
            let frame = first_frame + offset;
            assert_eq!(outcome.executed_frames, 1);
            assert_eq!(app.engine.frame(), frame);
            assert_eq!(outcome.skip_redraw, frame.rem_euclid(3) != 0);
            assert_eq!(accumulator, Duration::ZERO);
        }

        app.process_running_chat_text("/slow");
        assert!(!app.full_speed);
        assert_eq!(app.frame_skip, 1);
        let paced_frame = app.engine.frame();
        let waiting = advance_simulation_pass(&mut app, &mut schedule, &mut accumulator)
            .expect("normal pacing waits without elapsed time");
        assert_eq!(waiting.executed_frames, 0);
        accumulator += Duration::from_millis(27);
        assert_eq!(
            advance_simulation_pass(&mut app, &mut schedule, &mut accumulator)
                .expect("27ms remains below the normal tick")
                .executed_frames,
            0
        );
        accumulator += Duration::from_millis(1);
        let paced = advance_simulation_pass(&mut app, &mut schedule, &mut accumulator)
            .expect("the 28th millisecond executes one normal tick");
        assert_eq!(paced.executed_frames, 1);
        assert!(!paced.skip_redraw);
        assert_eq!(app.engine.frame(), paced_frame + 1);

        app.process_running_chat_text("/fast 1");
        let fast_one = advance_simulation_pass(&mut app, &mut schedule, &mut accumulator)
            .expect("/fast 1 is still unpaced");
        assert_eq!(fast_one.executed_frames, 1);
        assert!(!fast_one.skip_redraw);
    }

    #[test]
    fn automatic_frame_skip_uses_cpp_strict_slow_graphics_threshold() {
        let mut frame_skip = AutomaticFrameSkip::default();
        let tick_delay = Duration::from_millis(28);

        frame_skip.finish_graphics_pass(true, tick_delay, tick_delay);
        assert!(!frame_skip.begin_graphics_pass(true));

        frame_skip.finish_graphics_pass(true, tick_delay + Duration::from_millis(1), tick_delay);
        assert!(frame_skip.begin_graphics_pass(true));
    }

    #[test]
    fn presentation_detail_steps_down_only_on_a_sustained_overrun() {
        // Quality reduction must be invisible on hardware that copes: a single
        // slow frame (a shader compile, a texture upload, an OS hiccup) must
        // never cost the player fire particles.
        let budget = Duration::from_millis(28);
        let mut governor = PresentationDetailGovernor::default();
        assert_eq!(governor.detail(), PresentationDetail::Full);

        for _ in 0..DETAIL_STEP_DOWN_PASSES - 1 {
            governor.record_graphics_pass(true, Duration::from_millis(40), budget);
            assert_eq!(
                governor.detail(),
                PresentationDetail::Full,
                "an overrun shorter than the streak must not degrade anything"
            );
        }
        governor.record_graphics_pass(true, Duration::from_millis(40), budget);
        assert_eq!(governor.detail(), PresentationDetail::NoFireParticles);

        // Still over budget after the first step: give up the gamma resolve
        // pass too, then stop — there is nothing cheaper left to trade.
        for _ in 0..DETAIL_STEP_DOWN_PASSES {
            governor.record_graphics_pass(true, Duration::from_millis(40), budget);
        }
        assert_eq!(governor.detail(), PresentationDetail::NoGammaPass);
        for _ in 0..DETAIL_STEP_DOWN_PASSES * 4 {
            governor.record_graphics_pass(true, Duration::from_millis(400), budget);
        }
        assert_eq!(
            governor.detail(),
            PresentationDetail::NoGammaPass,
            "the ladder has a bottom; it must not wrap or keep counting"
        );
    }

    #[test]
    fn presentation_detail_recovers_only_with_real_headroom() {
        let budget = Duration::from_millis(28);
        let mut governor = PresentationDetailGovernor::default();
        for _ in 0..DETAIL_STEP_DOWN_PASSES {
            governor.record_graphics_pass(true, Duration::from_millis(40), budget);
        }
        assert_eq!(governor.detail(), PresentationDetail::NoFireParticles);

        // Just inside budget is the deadband: recovering there would step back
        // up into the very cost that caused the overrun and oscillate.
        for _ in 0..DETAIL_STEP_UP_PASSES * 2 {
            governor.record_graphics_pass(true, Duration::from_millis(27), budget);
        }
        assert_eq!(governor.detail(), PresentationDetail::NoFireParticles);

        // Comfortable headroom restores detail, one step at a time.
        for _ in 0..DETAIL_STEP_UP_PASSES {
            governor.record_graphics_pass(true, Duration::from_millis(5), budget);
        }
        assert_eq!(governor.detail(), PresentationDetail::Full);

        // Disabling automatic degradation restores full detail immediately.
        let mut governor = PresentationDetailGovernor::default();
        for _ in 0..DETAIL_STEP_DOWN_PASSES * 2 {
            governor.record_graphics_pass(true, Duration::from_millis(40), budget);
        }
        assert_ne!(governor.detail(), PresentationDetail::Full);
        governor.record_graphics_pass(false, Duration::from_millis(400), budget);
        assert_eq!(governor.detail(), PresentationDetail::Full);
    }

    #[test]
    fn framebuffer_backends_widen_to_gl_before_giving_up() {
        use pixels::wgpu::Backends;

        // `Backends::PRIMARY` is VULKAN | METAL | DX12 | BROWSER_WEBGPU — it
        // contains no GL/GLES at all. That is right on desktop (the GL backend
        // probes for libEGL and logs noise on macOS) but on a Raspberry Pi it
        // is the difference between running and aborting at startup, because a
        // board without a usable Vulkan driver produces no adapter and
        // `PixelsBuilder::build` then fails out of `main`.
        let attempts = framebuffer_backend_attempts(None);
        assert_eq!(
            attempts.first().copied(),
            Some(Backends::PRIMARY),
            "the desktop-clean set is still tried first"
        );
        assert!(
            attempts
                .last()
                .is_some_and(|backends| backends.contains(Backends::GL)),
            "a board with only GLES must still get an adapter attempt"
        );
        assert!(attempts.len() >= 2);

        // An explicit WGPU_BACKEND is an instruction, not a hint: never widen
        // past what the operator asked for.
        assert_eq!(
            framebuffer_backend_attempts(Some(Backends::VULKAN)),
            vec![Backends::VULKAN],
        );
    }

    #[test]
    fn a_simulation_burst_yields_to_the_event_loop_once_its_budget_is_spent() {
        // On hardware that cannot hold the tick budget, one application pass
        // drains the whole clamped backlog before the event loop ever gets a
        // chance to draw. The burst budget bounds that, while still executing
        // at least one frame so the simulation always makes progress.
        let mut app = new_running_sandbox_app();
        let mut schedule = frame_schedule_for_mode(
            app.mode,
            app.engine.game_tick_delay_ms(),
            app.engine.game_tick_delay_revision(),
            app.max_refresh_delay_ms,
        );

        // FREEZE the unbudgeted behaviour first: a full backlog drains at once.
        let mut accumulator = MAX_ACCUMULATED_TIME;
        let drained = advance_simulation_pass(&mut app, &mut schedule, &mut accumulator)
            .expect("an unbudgeted pass drains the clamped backlog");
        assert_eq!(
            drained.executed_frames,
            (MAX_ACCUMULATED_TIME.as_millis() / schedule.simulation_interval.as_millis()) as u32,
            "the whole backlog runs inside one pass when nothing bounds it"
        );

        // The same backlog under an exhausted budget yields after one frame.
        let mut accumulator = MAX_ACCUMULATED_TIME;
        let before = app.engine.frame();
        let bounded = advance_simulation_pass_within(
            &mut app,
            &mut schedule,
            &mut accumulator,
            Duration::ZERO,
        )
        .expect("a budgeted pass still executes one frame");
        assert_eq!(
            bounded.executed_frames, 1,
            "an exhausted budget yields after one frame, it does not stall"
        );
        assert_eq!(app.engine.frame(), before + 1);
        assert!(
            accumulator >= schedule.simulation_interval,
            "the unspent backlog is retained for the next pass, not discarded"
        );
    }

    #[test]
    fn render_floor_reserves_a_share_of_the_wall_clock_for_drawing() {
        // A machine that cannot hold the tick budget must still repaint.
        // `AutomaticFrameSkip` alone cannot do this: it is a one-shot latch on
        // the *graphics* opportunity, while the cost that starves drawing is a
        // catch-up burst inside one simulation pass. The floor bounds that
        // burst so drawing keeps roughly RENDER_RESERVE_PERCENT of the wall
        // clock, exactly the reservation Spring's game controller makes.
        let mut floor = RenderFloor::default();
        let base = Instant::now();

        // With no measurement yet the burst may run a full simulation period.
        assert_eq!(
            floor.simulation_burst_budget(Duration::from_millis(28)),
            Duration::from_millis(28),
            "an unmeasured graphics pass must not shorten the first burst"
        );

        // A 3 ms draw is cheap: simulation may run 85/15 of it before yielding.
        floor.record_presentation(base, Duration::from_millis(3));
        assert_eq!(
            floor.simulation_burst_budget(Duration::from_millis(28)),
            Duration::from_millis(28),
            "the burst never drops below one simulation period"
        );

        // A 30 ms draw on a slow machine buys simulation 170 ms, not forever.
        floor.record_presentation(base, Duration::from_millis(30));
        assert_eq!(
            floor.simulation_burst_budget(Duration::from_millis(28)),
            Duration::from_millis(170),
        );

        // And the reservation is itself capped by the hard repaint floor.
        floor.record_presentation(base, Duration::from_millis(400));
        assert_eq!(
            floor.simulation_burst_budget(Duration::from_millis(28)),
            MAX_TIME_BETWEEN_RENDERS,
            "no burst may outrun the hard repaint floor"
        );
    }

    #[test]
    fn render_floor_forces_a_repaint_at_two_hertz_however_deep_the_skip() {
        // `/fast 500` and the network catch-up divisor can both suppress every
        // graphics opportunity for an unbounded number of frames. The floor is
        // the only thing that guarantees the window still updates.
        let mut floor = RenderFloor::default();
        let base = Instant::now();
        floor.record_presentation(base, Duration::from_millis(5));

        assert!(!floor.must_present(base + Duration::from_millis(499)));
        assert!(floor.must_present(base + MAX_TIME_BETWEEN_RENDERS));
        assert!(floor.must_present(base + Duration::from_secs(30)));

        // A repaint rearms it.
        floor.record_presentation(base + Duration::from_secs(30), Duration::from_millis(5));
        assert!(!floor.must_present(base + Duration::from_secs(30)));

        // Before the very first repaint the floor is armed from the first ask,
        // so a game that never draws still gets its first frame on time.
        let mut fresh = RenderFloor::default();
        assert!(!fresh.must_present(base));
        assert!(fresh.must_present(base + MAX_TIME_BETWEEN_RENDERS));
    }

    #[test]
    fn automatic_frame_skip_never_skips_two_consecutive_graphics_passes() {
        let mut frame_skip = AutomaticFrameSkip::default();
        frame_skip.finish_graphics_pass(true, Duration::from_millis(29), Duration::from_millis(28));

        assert!(frame_skip.begin_graphics_pass(true));
        assert!(!frame_skip.begin_graphics_pass(true));
    }

    #[test]
    fn automatic_frame_skip_freezes_cpp_parameter_precedence() {
        assert!(configured_auto_frame_skip(b""));
        assert!(!configured_auto_frame_skip(
            b"[Graphics]\nAutoFrameSkip=false\n"
        ));
        assert!(!frozen_auto_frame_skip(true, Some(false), None));
        assert!(frozen_auto_frame_skip(false, Some(false), Some(true)));
    }

    #[test]
    fn graphics_deadline_ignores_early_wakes_and_coalesces_missed_periods() {
        let base = Instant::now();
        let interval = Duration::from_millis(14);
        let deadline = base + interval;

        assert!(base + Duration::from_millis(10) < deadline);
        assert_eq!(
            advance_graphics_deadline(deadline, deadline, interval),
            base + Duration::from_millis(28)
        );
        let late = base + Duration::from_millis(50);
        assert_eq!(
            advance_graphics_deadline(deadline, late, interval),
            late + interval
        );
    }

    #[test]
    fn hud_game_time_reads_the_engine_snapshot() {
        let mut app = new_state_only_menu_app(320, 200);
        app.snapshot.game_time = 61;
        assert_eq!(app.game_time_seconds(), 61);
    }

    fn make_packed_test_entry_unreadable(group: &mut [u8], entry_index: usize) {
        const HEADER_SIZE: usize = 204;
        const ENTRY_SIZE: usize = 316;
        const SIZE_OFFSET: usize = 268;

        let size = HEADER_SIZE + entry_index * ENTRY_SIZE + SIZE_OFFSET;
        group[size..size + 4].copy_from_slice(&i32::MAX.to_le_bytes());
    }

    #[test]
    fn configured_fullscreen_reaches_platform_startup_path() {
        let install = tempdir().expect("install root");
        let user_data = tempdir().expect("user data");
        fs::create_dir_all(install.path().join("planet")).expect("planet directory");
        fs::write(install.path().join("planet/System.c4g"), b"stub").expect("system group stub");
        let config_file = user_data.path().join("fullscreen.config");
        fs::write(&config_file, "[Graphics]\nDisplayMode=Window\n").expect("seed windowed config");
        let _guard = EnvGuard::set(&[
            ("LC_INSTALL_ROOT", Some(install.path())),
            ("LC_USER_DATA_DIR", Some(user_data.path())),
            ("LC_CONFIG_FILE", None),
        ]);
        let paths = AppPaths::discover_with_config_file(Some(&config_file))
            .expect("discover fullscreen config paths");
        let windowed = DisplayOptions::load(Some(&paths));
        assert_eq!(windowed.mode, DisplayMode::Window);
        assert!(!defer_startup_fullscreen_until_resumed(windowed.mode));
        assert!(!should_reconcile_deferred_fullscreen(windowed.mode, false));
        assert!(
            startup_window_builder(&windowed, PhysicalSize::new(800, 600))
                .window_attributes()
                .fullscreen
                .is_none()
        );

        fs::write(&config_file, "[Graphics]\nDisplayMode=Fullscreen\n")
            .expect("select fullscreen mode");
        let display = DisplayOptions::load(Some(&paths));
        assert_eq!(display.mode, DisplayMode::Fullscreen);
        assert_eq!(
            should_reconcile_deferred_fullscreen(display.mode, false),
            cfg!(target_os = "macos")
        );
        assert!(!should_reconcile_deferred_fullscreen(display.mode, true));

        let builder = startup_window_builder(&display, PhysicalSize::new(800, 600));

        if defer_startup_fullscreen_until_resumed(display.mode) {
            assert!(builder.window_attributes().fullscreen.is_none());
        } else {
            assert!(matches!(
                builder.window_attributes().fullscreen.as_ref(),
                Some(Fullscreen::Borderless(None))
            ));
        }
    }

    #[test]
    fn positional_mix_ownerless_viewport_listens_at_its_live_center() {
        let player_listener = make_object(1, "Listener", Vector2::new(350, 100));
        let mut snapshot = make_snapshot(vec![player_listener.clone()], Vec::new());
        snapshot.players = vec![PlayerState {
            id: 1,
            view_cursor: Some(player_listener.id),
            ..Default::default()
        }];
        let viewports = [audio_viewport(0, OWNER_NONE, Vector2::new(0, 100))];

        assert_eq!(
            compute_positional_mix_values(Vector2::new(350, 100), &snapshot, &viewports),
            (50, 0.7),
            "a NO_OWNER viewport has no player listener override",
        );
        assert_eq!(
            compute_positional_mix_values(Vector2::new(350, 100), &snapshot, &[]),
            (0, 0.0),
            "no active C4Viewport means no audibility or pan contribution",
        );
    }

    #[test]
    fn rendered_object_audibility_cache_retains_until_the_next_completed_render() {
        let line = make_object(1, "LINE", Vector2::new(2_000, 100));
        let mut snapshot = make_snapshot(vec![line.clone()], Vec::new());
        snapshot.definition_lines.insert(
            line.definition_id.clone(),
            clonk_engine::DefinitionLineMetadata {
                line: 1,
                ..Default::default()
            },
        );
        let viewports = [audio_viewport(0, OWNER_NONE, Vector2::new(0, 100))];
        let calls = HashMap::from([(
            line.id,
            vec![RenderedAudibilityCall::World {
                point: Vector2::new(350, 100),
            }],
        )]);
        let mut audio = empty_test_audio_context();
        audio.cache_rendered_object_audibility(&calls, &snapshot, &viewports);
        let retained = audio.rendered_object_audibility.clone();

        audio.update_channels(&snapshot, &viewports, true);
        assert_eq!(
            audio.rendered_object_audibility, retained,
            "a sound tick without a completed render retains the prior draw cache",
        );

        // C4Object::GetAudibility (C4Object.cpp:5622-5628) only recomputes
        // when Audible is -1, which C4GraphicsSystem::Execute's
        // ResetAudibility sets. Movement between the draw and the mix leaves
        // the drawn pair in place and observably apart from the origin mix.
        snapshot.objects[0].position.x += 1;
        assert_eq!(
            compute_mix_values_for_with_rendered_audibility(
                100,
                Some(line.id),
                None,
                &snapshot,
                &viewports,
                &audio.rendered_object_audibility,
            ),
            (0.5, 0.7),
            "movement without a new render keeps the retained draw audibility",
        );
        assert_eq!(
            compute_mix_values_for(100, Some(line.id), None, &snapshot, &viewports),
            (0.0, 1.0),
            "the live origin mix remains observably different",
        );

        audio.cache_rendered_object_audibility(&HashMap::new(), &snapshot, &viewports);
        assert!(
            audio.rendered_object_audibility.is_empty(),
            "the next completed render replaces rather than extends the cache",
        );
        audio.cache_rendered_object_audibility(&calls, &snapshot, &viewports);
        audio.reset_sound_system_generation();
        assert!(audio.rendered_object_audibility.is_empty());
    }

    #[test]
    fn rendered_audibility_reduction_clamps_each_parallax_pan_call() {
        let mut target = make_object(1, "PARA", Vector2::ZERO);
        target.category |= C4D_PARALLAX;
        let snapshot = make_snapshot(vec![target.clone()], Vec::new());
        let calls = HashMap::from([(
            target.id,
            vec![
                RenderedAudibilityCall::Parallax {
                    point: Vector2::new(1_000, 0),
                    rendered_center: Vector2::ZERO,
                },
                RenderedAudibilityCall::Parallax {
                    point: Vector2::new(-10, 0),
                    rendered_center: Vector2::ZERO,
                },
            ],
        )]);

        assert_eq!(
            reduce_rendered_object_audibility(&calls, &snapshot, &[], &HashMap::new())[&target.id].pan,
            98,
            "each integer contribution is clamped before the next one is added",
        );
    }

    #[test]
    fn inactive_parallax_cache_retains_pan_while_normal_objects_reset_it() {
        let mut target = make_object(1, "PARA", Vector2::ZERO);
        target.category |= C4D_PARALLAX;
        target.status = ObjectStatus::Inactive;
        let snapshot = make_snapshot(vec![target.clone()], Vec::new());
        let previous = HashMap::from([(
            target.id,
            CachedObjectAudibilityMix {
                object_position: target.position,
                audibility: 37,
                pan: 80,
            },
        )]);
        let no_calls = HashMap::new();
        assert_eq!(
            reduce_rendered_object_audibility(&no_calls, &snapshot, &[], &previous),
            previous,
            "inactive MODE_Object targets are outside the frame reset loop",
        );

        let calls = HashMap::from([(
            target.id,
            vec![RenderedAudibilityCall::Parallax {
                point: Vector2::new(100, 0),
                rendered_center: Vector2::ZERO,
            }],
        )]);
        assert_eq!(
            reduce_rendered_object_audibility(&calls, &snapshot, &[], &previous)[&target.id].pan,
            100,
            "the next inactive parallax contribution starts from retained pan",
        );

        let mut normal_snapshot = snapshot;
        normal_snapshot.objects[0].status = ObjectStatus::Normal;
        assert_eq!(
            reduce_rendered_object_audibility(&calls, &normal_snapshot, &[], &previous)[&target.id].pan,
            20,
            "normal objects begin every completed frame with reset pan",
        );
    }

    #[test]
    fn object_bound_mix_uses_only_active_viewport_listener_and_pan() {
        let dir = tempdir().expect("tempdir");
        let scenario = dir.path().join("ObjectMix.c4s");
        fs::create_dir_all(&scenario).expect("create scenario group");
        fs::write(scenario.join("Impact.wav"), silent_pcm_wav(10_000)).expect("write object sound");

        let mut audio = AudioContext::try_new(AudioOptions::default()).expect("audio context");
        audio.configure_scenario(Some(&scenario));

        let source = make_object(1, "SNDS", Vector2::new(350, 100));
        let listener = make_object(2, "LIST", Vector2::new(500, 100));
        let remote_listener = make_object(3, "RMTE", source.position);
        let mut snapshot = make_snapshot(
            vec![source.clone(), listener.clone(), remote_listener.clone()],
            vec![HudPlayerSnapshot {
                owner: 8,
                crew: vec![remote_listener.id],
                focus: Some(remote_listener.id),
                eliminated: false,
                wealth: 0,
                score: 0,
            }],
        );
        snapshot.players = vec![
            PlayerState {
                id: 7,
                view_target: Some(listener.id),
                cursor: Some(remote_listener.id),
                ..Default::default()
            },
            PlayerState {
                id: 8,
                view_cursor: Some(remote_listener.id),
                ..Default::default()
            },
        ];
        let viewports = [audio_viewport(0, 7, Vector2::new(0, 100))];
        let mut cursor_fallback = snapshot.clone();
        cursor_fallback.players[0].view_cursor = Some(listener.id);
        cursor_fallback.players[0].view_target = Some(remote_listener.id);
        assert_eq!(
            compute_positional_mix_values(source.position, &cursor_fallback, &viewports),
            (79, 0.7),
            "ViewCursor takes precedence over ViewTarget and Cursor",
        );
        cursor_fallback.players[0].view_cursor = None;
        cursor_fallback.players[0].view_target = None;
        cursor_fallback.players[0].cursor = Some(listener.id);
        assert_eq!(
            compute_positional_mix_values(source.position, &cursor_fallback, &viewports),
            (79, 0.7),
            "Cursor is used when ViewCursor and ViewTarget are null",
        );
        cursor_fallback.players[0].cursor = None;
        assert_eq!(
            compute_positional_mix_values(source.position, &cursor_fallback, &viewports),
            (50, 0.7),
            "the live viewport center is the final listener fallback",
        );

        audio
            .start_sound(
                "Impact",
                Some(source.id),
                100,
                false,
                false,
                None,
                &snapshot,
                &viewports,
            )
            .expect("object sound starts");
        let key = SoundInstanceKey::new("Impact", Some(source.id));
        assert_eq!(audio.active_channels[&key].detached_mix, Some((0.79, 0.7)));
        assert!(audio.active_channels[&key].channel.is_some());

        audio.update_channels(&snapshot, &[], true);
        assert_eq!(audio.active_channels[&key].detached_mix, Some((0.0, 0.0)));
        assert!(audio.active_channels[&key].channel.is_none());

        let moved_viewports = [audio_viewport(0, 7, Vector2::new(100, 100))];
        audio.update_channels(&snapshot, &moved_viewports, true);
        assert_eq!(audio.active_channels[&key].detached_mix, Some((0.79, 0.5)));
        assert!(audio.active_channels[&key].channel.is_some());

        audio.detach_object_sounds(source.id, source.position, &snapshot, &viewports);
        let info = audio
            .active_channels
            .values()
            .find(|info| info.sample_name == "impact.wav")
            .expect("detached instance remains live");
        assert_eq!(info.target, None);
        assert_eq!(info.detached_mix, Some((0.79, 0.7)));
        audio.update_channels(&snapshot, &[], true);
        assert_eq!(
            audio
                .active_channels
                .values()
                .find(|info| info.sample_name == "impact.wav")
                .and_then(|info| info.detached_mix),
            Some((0.79, 0.7)),
        );
    }

    #[test]
    fn global_portrait_frame_position_is_independent_of_text_alignment() {
        // C4GM_Left/C4GM_Right position the portrait frame. Only C4GM_ALeft /
        // ACenter / ARight select TextOut alignment (src/C4GameMessage.cpp:
        // 101,140-168).
        let viewport = Rect::new(30, 40, 640, 480);
        let offset = Vector2::new(12, 18);
        let positioned = FLAG_LEFT | FLAG_TOP;
        let frame = global_portrait_frame_rect(viewport, offset, positioned, (121, 77));
        let aligned_frame =
            global_portrait_frame_rect(viewport, offset, positioned | FLAG_ALIGN_RIGHT, (121, 77));

        assert_eq!(frame, aligned_frame);
        assert_eq!(
            message_horizontal_alignment(positioned | FLAG_ALIGN_RIGHT, true),
            HorizontalAlignment::Right
        );
        assert_eq!(
            message_horizontal_alignment(FLAG_RIGHT | FLAG_ALIGN_LEFT, true),
            HorizontalAlignment::Left
        );
        assert_eq!(
            message_horizontal_alignment(FLAG_RIGHT, true),
            HorizontalAlignment::Left,
            "portrait frame position must not change the default text alignment"
        );
    }

    #[test]
    fn running_render_draws_supported_globals_and_ignores_remote_or_missing_targets() {
        let mut app = new_classic_running_sandbox_app();
        let visible = clonk_engine::MessageSnapshot {
            id: 1,
            kind: MessageKind::Global,
            lines: vec!["Visible".to_string()],
            target: None,
            player: None,
            offset: Vector2::ZERO,
            color: 0xff20_3040,
            flags: 0,
            width: None,
            decoration: None,
            frame_decoration: None,
            portrait: None,
        };
        let mut remote = visible.clone();
        remote.id = 2;
        remote.kind = MessageKind::GlobalPlayer;
        remote.player = Some(app.local_owner + 1);
        let mut missing_target = visible.clone();
        missing_target.id = 3;
        missing_target.kind = MessageKind::Target;
        missing_target.target = Some(ObjectId::new(u64::MAX));

        let mut baseline = vec![0; 320 * 200 * 4];
        app.snapshot.hud.messages = vec![remote.clone(), missing_target.clone()];
        app.render(&mut baseline)
            .expect("remote and missing-target messages are not client-visible");

        app.snapshot.hud.messages = vec![remote, missing_target, visible];
        let mut rendered = vec![0; 320 * 200 * 4];
        app.render(&mut rendered)
            .expect("supported global C4GameMessage renders");
        assert_ne!(rendered, baseline, "the visible global contributes pixels");
    }

    #[test]
    fn same_owner_split_checks_target_in_second_exact_viewport() {
        let mut app = new_running_sandbox_app();
        let target = app.snapshot.objects.first_mut().expect("sandbox target");
        target.position = Vector2::new(1_000, 180);
        let target_id = target.id;
        let player = app
            .snapshot
            .players
            .iter_mut()
            .find(|player| player.id == app.local_owner)
            .expect("sandbox local player");
        // This probe isolates exact-viewport geometry. Runtime mouse setup
        // correctly enables FoW, whose missing visibility bitmap is tested by
        // the dedicated fail-closed cases below.
        player.fog_of_war = false;
        player.force_fog_of_war = true;
        player.viewports = vec![
            clonk_engine::PlayerViewport::new(Vector2::new(100, 180)).with_focus(Some(target_id)),
            clonk_engine::PlayerViewport::new(Vector2::new(1_000, 180)).with_focus(Some(target_id)),
        ];
        app.snapshot.hud.messages = vec![clonk_engine::MessageSnapshot {
            id: 1,
            kind: MessageKind::TargetPlayer,
            lines: vec!["A".to_string()],
            target: Some(target_id),
            player: Some(app.local_owner),
            offset: Vector2::ZERO,
            color: 0xffff_ffff,
            flags: 0,
            width: None,
            decoration: None,
            frame_decoration: None,
            portrait: None,
        }];

        let messages = std::mem::take(&mut app.snapshot.hud.messages);
        let mut baseline = vec![0; 320 * 200 * 4];
        app.render(&mut baseline)
            .expect("render same-owner split baseline");
        app.snapshot.hud.messages = messages;
        let mut rendered = vec![0; 320 * 200 * 4];
        app.render(&mut rendered)
            .expect("the second same-owner viewport receives the target message");

        let projections = app.graphics.active_viewport_projections();
        assert_eq!(projections.len(), 2);
        assert_eq!([projections[0].index, projections[1].index], [0, 1]);
        assert_eq!(projections[0].owner, projections[1].owner);
        let target = app.snapshot.object(target_id).expect("target remains live");
        let shape_height = app
            .engine
            .definition_shape_rect(&target.definition_id)
            .map(|shape| shape.height)
            .unwrap_or(0);
        let first = c4_message_target_position(target, Vector2::ZERO, shape_height, projections[0]);
        let second = c4_message_target_position(target, Vector2::ZERO, shape_height, projections[1]);
        assert!(!projections[0].contains_logical_point(first));
        assert!(projections[1].contains_logical_point(second));
        let changed = rendered
            .chunks_exact(4)
            .zip(baseline.chunks_exact(4))
            .enumerate()
            .filter_map(|(index, (actual, before))| (actual != before).then_some(index))
            .collect::<Vec<_>>();
        assert!(!changed.is_empty(), "the target message contributes pixels");
        let viewport = projections[1].rect;
        assert!(changed.iter().all(|index| {
            let x = (*index % 320) as i32;
            let y = (*index / 320) as i32;
            x >= viewport.x
                && x < viewport.x + viewport.width as i32
                && y >= viewport.y
                && y < viewport.y + viewport.height as i32
        }));
    }

    #[test]
    fn live_temporary_physicals_feed_all_integer_hud_bar_ranges() {
        let mut app = new_state_only_running_sandbox_app();
        let crew = app
            .engine
            .crew_cursor(app.local_owner)
            .expect("sandbox player has a live cursor");
        let mut update = ObjectUpdate::new();
        update.energy = Some(1);
        update.magic_energy = Some(1_000);
        update.breath = Some(1);
        app.engine
            .apply_object_update(crew, update)
            .expect("install half-full raw HUD levels");
        let target_object =
            i32::try_from(crew.as_u64()).expect("sandbox cursor id fits script control");
        for (name, value) in [("Energy", 2), ("Magic", 2_000), ("Breath", 2)] {
            app.engine
                .execute_script_control(
                    &clonk_engine::ScriptControlData {
                        target_object,
                        strictness: clonk_engine::ScriptStrictness::Strict3,
                        script: clonk_engine::LegacyCString::from_bytes(
                            format!("SetPhysical(\"{name}\", {value}, 2)").into_bytes(),
                        )
                        .expect("temporary-physical script is NUL-free"),
                        by_client: 0,
                    },
                    ScriptControlPolicy::live(false),
                )
                .unwrap_or_else(|error| panic!("install temporary {name} physical: {error}"));
        }
        app.snapshot = app.engine.snapshot();

        let crew_index = app
            .engine
            .find_object_index(crew)
            .expect("cursor remains live");
        let before_object = app.engine.object_snapshot(crew).expect("cursor snapshot");
        let before_physical = app.engine.object_physical(crew_index);
        let overlays = collect_player_overlays(
            &mut app.engine,
            &app.snapshot,
            Some(crew),
            &app.bindings,
            &app.gamepad_bindings,
        );
        assert_eq!(app.engine.object_snapshot(crew), Some(before_object));
        assert_eq!(app.engine.object_physical(crew_index), before_physical);

        let crew = overlays
            .iter()
            .find(|player| player.owner == app.local_owner)
            .and_then(|player| player.crew.iter().find(|entry| entry.object_id == crew))
            .expect("live cursor reaches the HUD overlay");
        assert_eq!((crew.energy, crew.energy_capacity), (1, 2));
        assert_eq!((crew.magic_energy, crew.magic_capacity), (1_000, 2_000));
        assert_eq!((crew.breath, crew.breath_capacity), (1, 2));

        let columns = [
            [220, 0, 0, 255],
            [70, 0, 0, 255],
            [0, 220, 0, 255],
            [0, 70, 0, 255],
            [0, 0, 220, 255],
            [0, 0, 70, 255],
        ];
        let pixels = (0..3).flat_map(|_| columns.into_iter().flatten()).collect();
        let hud = HudGraphics {
            energy_bars: Some(ImageData::new(6, 3, pixels)),
            ..HudGraphics::default()
        };
        let mut surface = Surface::new(40, 200, PixelFormat::Rgba8888);
        let viewport = clonk_graphics::Rect::new(0, 0, 40, 200);
        for (kind, slot, level, range) in [
            (
                clonk_frontend::hud::HudBarKind::Energy,
                0,
                crew.energy,
                crew.energy_capacity,
            ),
            (
                clonk_frontend::hud::HudBarKind::Magic,
                1,
                crew.magic_energy / 1_000,
                crew.magic_capacity / 1_000,
            ),
            (
                clonk_frontend::hud::HudBarKind::Breath,
                2,
                crew.breath,
                crew.breath_capacity,
            ),
        ] {
            clonk_frontend::hud::draw_level_bar(
                &mut surface,
                &hud,
                viewport,
                kind,
                slot,
                level,
                range,
                true,
            );
        }

        let empty = [
            Color::opaque(70, 0, 0),
            Color::opaque(0, 70, 0),
            Color::opaque(0, 0, 70),
        ];
        let filled = [
            Color::opaque(220, 0, 0),
            Color::opaque(0, 220, 0),
            Color::opaque(0, 0, 220),
        ];
        for (slot, x) in [5_u32, 7, 9].into_iter().enumerate() {
            assert_eq!(
                surface.get_pixel(x, 107),
                Some(empty[slot]),
                "105px half bar keeps local row 52 empty"
            );
            assert_eq!(
                surface.get_pixel(x, 108),
                Some(filled[slot]),
                "105px half bar begins filling at local row 53"
            );
        }
    }

    #[test]
    fn l049_player_overlay_projects_transient_hud_flags() {
        let mut app = new_lightweight_running_sandbox_app();
        let owner = app.local_owner;
        let player = app
            .snapshot
            .players
            .iter_mut()
            .find(|player| player.id == owner)
            .expect("sandbox player");
        player.view_wealth = -1;
        player.view_value = -1;

        assert!(!app.engine.scenario_value_gain_enabled());
        let overlays = collect_player_overlays(
            &mut app.engine,
            &app.snapshot,
            None,
            &app.bindings,
            &app.gamepad_bindings,
        );
        let overlay = overlays
            .iter()
            .find(|player| player.owner == owner)
            .expect("sandbox overlay");
        assert!(
            overlay.view_wealth,
            "nonzero ViewWealth uses C++ truthiness"
        );
        assert!(
            !overlay.view_value,
            "ViewValue stays hidden when Game.ValueGain is disabled"
        );

        set_test_scenario_value_gain(&mut app, -1);
        let player = app
            .snapshot
            .players
            .iter_mut()
            .find(|player| player.id == owner)
            .expect("restored sandbox player");
        player.view_wealth = 0;
        player.view_value = -1;
        let overlays = collect_player_overlays(
            &mut app.engine,
            &app.snapshot,
            None,
            &app.bindings,
            &app.gamepad_bindings,
        );
        let overlay = overlays
            .iter()
            .find(|player| player.owner == owner)
            .expect("restored sandbox overlay");
        assert!(!overlay.view_wealth);
        assert!(
            overlay.view_value,
            "nonzero ViewValue is visible when Game.ValueGain is enabled"
        );
    }

    #[test]
    fn viewport_overlay_collection_skips_unpresented_remote_players() {
        let mut app = new_lightweight_running_sandbox_app();
        let local_owner = app.local_owner;
        let remote_owner = local_owner + 77;
        let mut remote_player = app
            .snapshot
            .players
            .first()
            .expect("sandbox player")
            .clone();
        remote_player.id = remote_owner;
        remote_player.name = "Remote benchmark player".to_string();
        app.snapshot.players.push(remote_player);
        let mut remote_hud = app
            .snapshot
            .hud
            .players
            .first()
            .expect("sandbox HUD player")
            .clone();
        remote_hud.owner = remote_owner;
        app.snapshot.hud.players.push(remote_hud);

        // C4GraphicsSystem iterates physical viewports, and each
        // C4Viewport::DrawOverlay resolves only that viewport's Player
        // (src/C4GraphicsSystem.cpp:167-170; src/C4Viewport.cpp:836-897).
        let owned_viewport = [ViewportInput::owned_without_focus(
            local_owner,
            Vector2::ZERO,
            1.0,
        )];
        let overlays = collect_player_overlays_for_viewports(
            &mut app.engine,
            &app.snapshot,
            None,
            &app.bindings,
            &app.gamepad_bindings,
            &owned_viewport,
        );
        assert_eq!(
            overlays
                .iter()
                .map(|overlay| overlay.owner)
                .collect::<Vec<_>>(),
            vec![local_owner]
        );

        // NO_OWNER is the native exception: C4Game::DrawCursors(NO_OWNER)
        // traverses every player, so their cursor-label data remains needed
        // (src/C4Game.cpp:1852-1887).
        let observer_viewport = [ViewportInput::ownerless(Vector2::ZERO, 1.0)];
        let overlays = collect_player_overlays_for_viewports(
            &mut app.engine,
            &app.snapshot,
            None,
            &app.bindings,
            &app.gamepad_bindings,
            &observer_viewport,
        );
        assert_eq!(overlays.len(), 2);
    }

    #[test]
    fn hud_graphics_loads_gamepad_startup_phases() {
        let paths = AppPaths::discover().expect("discover repository install");
        let graphics =
            GraphicsResource::open(paths.planet_dir().join("Graphics.c4g")).expect("open Graphics.c4g");
        let hud = FrontendAssets::load_hud_graphics(&graphics);
        let gamepad = hud.gamepad.as_ref().expect("Gamepad.png loaded into HUD");
        assert_eq!((gamepad.width(), gamepad.height()), (320, 36));
    }

    #[test]
    fn cursor_inventory_separates_and_draws_tflint_picture_rects() {
        // C4Object::CanConcatPictureWith compares per-object PictureRect unless
        // APS_Graphics is set, and Picture2Facet uses the own rect when Wdt is
        // nonzero (src/C4Object.cpp:3123-3129,6173-6213). T-Flint switches from
        // its DefCore picture (0,12,64,64) to (0,76,64,64) while active.
        let repository = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .parent()
            .and_then(Path::parent)
            .expect("repository root")
            .to_path_buf();
        let flint_group =
            Group::open(repository.join("content/Objects.c4d/Items.c4d/Weapons.c4d/TFlint.c4d"))
                .expect("open real TFLN definition");
        let flint_resource =
            clonk_resources::ResourceDefinition::load(&flint_group).expect("load real TFLN definition");
        let mut engine = Engine::new();
        engine
            .register_definition(
                Definition::from_resource(&flint_resource).expect("compile real TFLN definition"),
            )
            .expect("register TFLN");

        let crew_id = ObjectId::new(1);
        let idle_id = ObjectId::new(2);
        let active_id = ObjectId::new(3);
        let mut crew = make_object(crew_id.as_u64(), "CLNK", Vector2::ZERO);
        crew.owner = 0;
        crew.contents = vec![idle_id, active_id];
        let mut idle = make_object(idle_id.as_u64(), "TFLN", Vector2::ZERO);
        idle.owner = 0;
        idle.crew_member = false;
        idle.container = Some(crew_id);
        let mut active_json =
            serde_json::to_value(make_object(active_id.as_u64(), "TFLN", Vector2::ZERO))
                .expect("active flint serializes");
        active_json["owner"] = serde_json::json!(0);
        active_json["crew_member"] = serde_json::json!(false);
        active_json["container"] = serde_json::json!(crew_id.as_u64());
        active_json["picture_rect"] = serde_json::json!({"x": 0, "y": 76, "width": 64, "height": 64});
        active_json["color_modulation"] = serde_json::json!(0x0040_80c0_u32);
        let active: ObjectSnapshot =
            serde_json::from_value(active_json).expect("active flint deserializes");

        let snapshot = make_snapshot(
            vec![crew, idle, active.clone()],
            vec![HudPlayerSnapshot {
                owner: 0,
                crew: vec![crew_id],
                focus: Some(crew_id),
                eliminated: false,
                wealth: 0,
                score: 0,
            }],
        );

        let inventory = collect_crew_inventory(
            &engine,
            &snapshot,
            crew_id,
            clonk_frontend::AdvancedRendererConfig::DEFAULT,
        );
        assert_eq!(inventory.len(), 2, "different PictureRects do not stack");
        assert_eq!(inventory[0].object_id, idle_id);
        assert_eq!(inventory[1].object_id, active_id);
        let idle_picture = inventory[0].picture.as_ref().expect("idle picture");
        let active_picture = inventory[1].picture.as_ref().expect("active picture");
        assert_eq!((idle_picture.width(), idle_picture.height()), (64, 64));
        assert_eq!((active_picture.width(), active_picture.height()), (64, 64));
        assert_ne!(idle_picture.pixels(), active_picture.pixels());

        // C4ObjectMenu refills Sell/Get/Contents rows from the iterator's
        // representative object via Picture2Facet, not from the bare item ID
        // (src/C4ObjectMenu.cpp:246-264,286-313). The app must therefore use
        // the same object-picture compositor as the inventory HUD.
        let menu_item = clonk_engine::ObjectMenuItem {
            caption: "Get T-Flint".to_string(),
            info_caption: String::new(),
            command: String::new(),
            command2: String::new(),
            count: 1,
            item_id: "TFLN".to_string(),
            symbol: clonk_engine::ObjectMenuSymbol::Definition,
            image: clonk_engine::ObjectMenuImage::default(),
            presentation_definition_id: None,
            picture_snapshot: None,
            picture_object: Some(active_id),
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
        .expect("menu row picture");
        assert_eq!(menu_picture.pixels(), active_picture.pixels());

        // C4Object::Picture2Facet runs PrepareDrawing before drawing the
        // selected rect (src/C4Object.cpp:3138-3154); ColorMod therefore
        // modulates the inventory picture, not only the in-world sprite.
        let mut unmodulated = active.clone();
        unmodulated.color_modulation = 0;
        let unmodulated =
            inventory_object_picture(&engine, &unmodulated).expect("unmodulated active picture");
        let modulation = Color::new(0x40, 0x80, 0xc0, 0);
        for (source, actual) in unmodulated
            .pixels()
            .chunks_exact(4)
            .zip(active_picture.pixels().chunks_exact(4))
        {
            let expected =
                Color::new(source[0], source[1], source[2], source[3]).modulate_clr(modulation);
            assert_eq!(actual, &[expected.r, expected.g, expected.b, expected.a]);
        }
    }

    #[test]
    fn direct_hud_inventory_alpha_uses_the_full_renderer_snapshot() {
        let modulation = Color::new(255, 255, 255, 64);
        let prepared_alpha = |mode, shader, no_alpha_add| {
            let config = clonk_frontend::AdvancedRendererConfig {
                shader,
                no_alpha_add,
                ..clonk_frontend::AdvancedRendererConfig::DEFAULT
            };
            let mut pixels = vec![80, 100, 120, 192];
            prepare_inventory_pixels(&mut pixels, modulation, mode, config);
            let mut owner = vec![80, 100, 120, 192];
            prepare_inventory_owner_pixels(&mut owner, modulation, mode, config);
            assert_eq!(owner[3], pixels[3], "owner and base live passes agree");
            pixels[3]
        };

        assert_eq!(prepared_alpha(BlitMode::Normal, false, false), 128);
        assert_eq!(prepared_alpha(BlitMode::Normal, false, true), 192);
        assert_eq!(prepared_alpha(BlitMode::Normal, true, false), 128);
        assert_eq!(prepared_alpha(BlitMode::Normal, true, true), 128);
        assert_eq!(prepared_alpha(BlitMode::Mod2, false, false), 128);
        assert_eq!(prepared_alpha(BlitMode::Mod2, false, true), 128);
        assert_eq!(prepared_alpha(BlitMode::Mod2, true, false), 192);
        assert_eq!(prepared_alpha(BlitMode::Mod2, true, true), 192);
        assert_eq!(
            prepared_inventory_alpha(
                192,
                Color::new(255, 255, 255, 0),
                BlitMode::Normal,
                clonk_frontend::AdvancedRendererConfig {
                    shader: false,
                    no_alpha_add: true,
                    ..clonk_frontend::AdvancedRendererConfig::DEFAULT
                },
            ),
            192,
            "exact packed C4 white keeps GL_REPLACE alpha",
        );
    }

    #[test]
    fn script_text_spec_icons_use_the_exact_classic_facets() {
        fn phase_sheet(cell: u32, columns: u32, rows: u32) -> ImageData {
            let width = cell * columns;
            let height = cell * rows;
            let mut pixels = vec![0_u8; (width * height * 4) as usize];
            for phase in 0..columns * rows {
                let phase_x = (phase % columns) * cell;
                let phase_y = (phase / columns) * cell;
                for y in phase_y..phase_y + cell {
                    for x in phase_x..phase_x + cell {
                        let offset = ((y * width + x) * 4) as usize;
                        pixels[offset..offset + 4].copy_from_slice(&[phase as u8, 1, 2, 255]);
                    }
                }
            }
            ImageData::new(width, height, pixels)
        }

        let standard = phase_sheet(40, 6, 9);
        let extended = phase_sheet(64, 4, 4);
        let score = ImageData::new(2, 1, vec![91, 92, 93, 255, 94, 95, 96, 255]);
        let resources = ScriptTextSpecResources {
            gui_icons: Some(&standard),
            gui_icons_extended: Some(&extended),
            score: Some(&score),
        };
        let engine = Engine::new();

        for (spec, phase, size) in [
            ("Ico:Locked suffix", 13, 64),
            ("Ico:League", 8, 64),
            ("Ico:GameRunning", 30, 40),
            ("Ico:Lobby", 31, 40),
            ("Ico:RuntimeJoin", 32, 40),
            ("Ico:FairCrew", 2, 64),
        ] {
            let image = resolve_script_font_image(&engine, spec, 0xff, resources)
                .unwrap_or_else(|| panic!("{spec} resolves"));
            assert_eq!((image.width(), image.height()), (size, size), "{spec}");
            assert_eq!(image.pixels()[0], phase, "{spec}");
        }

        let settlement = resolve_script_font_image(&engine, "Ico:Settlement", 0xff, resources)
            .expect("settlement score facet resolves");
        assert_eq!(settlement, score);
    }

    #[test]
    fn l018_running_render_draws_resolved_world_cursor() {
        let mut app = new_synthetic_running_sandbox_app();
        install_l018_cursor_atlas(&mut app);
        let (width, height) = {
            let surface = app.graphics.surface();
            (surface.width(), surface.height())
        };
        let mut frame = vec![0_u8; width as usize * height as usize * 4];
        app.render(&mut frame).expect("establish running viewport");
        let viewport = app
            .graphics
            .active_viewport_projections()
            .into_iter()
            .find(|viewport| viewport.owner == app.local_owner)
            .expect("mouse owner viewport");
        let point = GuiPoint::new(
            (viewport.rect.x + viewport.rect.width as i32 / 2) as f32,
            (viewport.rect.y + viewport.rect.height as i32 / 2) as f32,
        );
        app.handle_cursor_moved(PhysicalPosition::new(
            f64::from(point.x),
            f64::from(point.y),
        ))
        .expect("route running mouse pointer");
        let retained = app.window_mouse_position;
        app.window_active = false;
        app.handle_focus_lost()
            .expect("clear running hover on focus loss");
        assert!(app.ingame_pointer.is_none());
        app.handle_focus_gained()
            .expect("reproject stationary pointer after focus gain");
        assert_eq!(app.window_mouse_position, retained);
        app.ingame_mouse_caption.cursor = IngameMouseCursorKind::Grab;
        app.ingame_mouse_caption.caption = None;
        app.running_gui_mouse_owned = false;
        let pointer = app.ingame_pointer.expect("running viewport owns pointer");

        app.render(&mut frame)
            .expect("draw resolved running cursor");
        let origin_x = (pointer.screen.x as i32 - 2) as u32;
        let origin_y = (pointer.screen.y as i32 - 2) as u32;
        assert_eq!(
            app.graphics.surface().get_pixel(origin_x, origin_y),
            Some(Color::opaque(3, 43, 200))
        );

        app.external_irc_dialog_visible = true;
        app.running_world_mouse_owned = true;
        app.pointer_left()
            .expect("leave while a running dialog is shown");
        assert!(
            app.ingame_pointer.is_some(),
            "fixture exercises the dialog-owned pointer-left early return"
        );
        app.external_irc_dialog_visible = false;
        app.render(&mut frame)
            .expect("render after restoring the OS pointer outside the client");
        assert_ne!(
            app.graphics.surface().get_pixel(origin_x, origin_y),
            Some(Color::opaque(3, 43, 200)),
            "the retained world pointer must not draw outside the client area"
        );
    }

    #[test]
    fn l018_passive_observer_renders_region_cursor() {
        let mut app = new_synthetic_running_sandbox_app();
        install_l018_cursor_atlas(&mut app);
        app.engine.set_local_players([]);
        app.local_controls = LocalControlRegistry::default();
        app.mouse_control = false;
        app.snapshot = app.engine.snapshot();
        let (width, height) = {
            let surface = app.graphics.surface();
            (surface.width(), surface.height())
        };
        let mut frame = vec![0_u8; width as usize * height as usize * 4];
        app.render(&mut frame)
            .expect("establish passive physical viewport");
        let viewport = app
            .graphics
            .active_viewport_projections()
            .into_iter()
            .find(|viewport| viewport.is_no_owner_viewport)
            .expect("passive viewport");
        let point = GuiPoint::new(
            (viewport.rect.x + viewport.rect.width as i32 / 2) as f32,
            (viewport.rect.y + viewport.rect.height as i32 / 2) as f32,
        );
        app.handle_cursor_moved(PhysicalPosition::new(
            f64::from(point.x),
            f64::from(point.y),
        ))
        .expect("route passive observer pointer");
        assert_eq!(
            app.ingame_mouse_caption.cursor,
            IngameMouseCursorKind::Region
        );

        app.render(&mut frame)
            .expect("render passive Region cursor");
        assert_eq!(
            app.ingame_mouse_caption.cursor,
            IngameMouseCursorKind::Region
        );
        assert!(
            app.graphics
                .surface()
                .pixels()
                .chunks_exact(4)
                .any(|pixel| pixel == [1, 40, 200, 255]),
            "passive Region cell must reach the composed frame"
        );
    }

    #[test]
    fn l018_running_render_draws_throw_point_and_shift_add_marker() {
        let mut app = new_synthetic_running_sandbox_app();
        install_l018_cursor_atlas(&mut app);
        let (width, height) = {
            let surface = app.graphics.surface();
            (surface.width(), surface.height())
        };
        let mut frame = vec![0_u8; width as usize * height as usize * 4];
        app.render(&mut frame).expect("establish running viewport");
        let viewport = app
            .graphics
            .active_viewport_projections()
            .into_iter()
            .find(|viewport| viewport.owner == app.local_owner)
            .expect("mouse owner viewport");
        let point = GuiPoint::new(
            (viewport.rect.x + viewport.rect.width as i32 / 2) as f32,
            (viewport.rect.y + viewport.rect.height as i32 / 2) as f32,
        );
        app.handle_cursor_moved(PhysicalPosition::new(
            f64::from(point.x),
            f64::from(point.y),
        ))
        .expect("route running pointer");
        let pointer = app.ingame_pointer.expect("running viewport pointer");
        let pointer_world = ingame_pointer_world_pixel(pointer);
        let landing = Vector2::new(pointer_world.x.saturating_add(24), pointer_world.y);
        app.ingame_mouse_caption.cursor = IngameMouseCursorKind::ThrowRight(landing);
        app.ingame_mouse_caption.caption = None;
        app.keyboard_modifiers = ModifiersState::SHIFT;
        app.running_gui_mouse_owned = false;

        app.render(&mut frame)
            .expect("render throw cursor, landing point, and Add marker");
        for (phase, color) in [
            ("throw", [21, 61, 200, 255]),
            ("landing Point", [27, 67, 200, 255]),
            ("Shift Add", [31, 71, 200, 255]),
        ] {
            assert!(
                app.graphics
                    .surface()
                    .pixels()
                    .chunks_exact(4)
                    .any(|pixel| pixel == color),
                "{phase} cursor cell must reach the composed frame"
            );
        }
    }

    #[test]
    fn every_global_gui_sheet_is_eagerly_required_and_malformed_fails_closed() {
        for (stem, canonical_name) in CLASSIC_GLOBAL_GUI_SHEETS {
            let mut missing = new_classic_menu_app(320, 200);
            remove_global_gui_sheet(&mut missing, canonical_name);
            let error = missing
                .assets
                .require_classic_global_gui_bootstrap_resources(&HashMap::new())
                .expect_err("every global sheet is unconditional");
            assert_eq!(
                error,
                ClassicParityBoundary::GlobalGuiBootstrapResources {
                    issues: vec![ClassicGuiBootstrapIssue::missing(stem)],
                },
                "missing {stem} must retain oracle order and identity"
            );

            let mut malformed = new_classic_menu_app(320, 200);
            Arc::get_mut(&mut malformed.assets)
                .expect("frontend assets are app-owned")
                .startup_dialog_images
                .insert(canonical_name.to_string(), ImageData::new(0, 1, Vec::new()));
            let error = malformed
                .assets
                .require_classic_global_gui_bootstrap_resources(&HashMap::new())
                .expect_err("malformed global sheet must fail");
            assert_eq!(
                error,
                ClassicParityBoundary::GlobalGuiBootstrapResources {
                    issues: vec![ClassicGuiBootstrapIssue::malformed(
                        stem,
                        "a non-empty decoded RGBA surface",
                        "0x1 with 0 bytes",
                    )],
                }
            );
        }
    }

    #[test]
    fn global_gui_fonts_require_initialized_glyph_atlases() {
        for name in CLASSIC_GLOBAL_GUI_FONTS {
            let mut app = new_classic_menu_app(320, 200);
            let assets = Arc::get_mut(&mut app.assets).expect("frontend assets are app-owned");
            if name == "FontTooltip" {
                let mut empty = clonk_graphics::clonk_font::ClonkFont::new(22);
                empty.cell_height = empty.line_height;
                empty.h_space = 0;
                assets.global_tooltip_font = Some(Arc::new(empty));
            } else {
                let fonts = assets.clonk_fonts.as_deref().expect("global GUI fonts");
                let replacement = clonk_frontend::ClonkFontSet {
                    title: if name == "FontTitle" {
                        clonk_graphics::clonk_font::ClonkFont::new(fonts.title.line_height)
                    } else {
                        fonts.title.clone()
                    },
                    caption: if name == "FontCaption" {
                        clonk_graphics::clonk_font::ClonkFont::new(fonts.caption.line_height)
                    } else {
                        fonts.caption.clone()
                    },
                    text: if name == "FontRegular" {
                        clonk_graphics::clonk_font::ClonkFont::new(fonts.text.line_height)
                    } else {
                        fonts.text.clone()
                    },
                    main_small: fonts.main_small.clone(),
                    mini: if name == "FontTiny" {
                        clonk_graphics::clonk_font::ClonkFont::new(fonts.mini.line_height)
                    } else {
                        fonts.mini.clone()
                    },
                };
                assets.clonk_fonts = Some(Arc::new(replacement));
            }
            let error = assets
                .require_classic_global_gui_bootstrap_resources(&HashMap::new())
                .expect_err("metric-valid empty font must fail");
            assert!(matches!(
                error,
                ClassicParityBoundary::GlobalGuiBootstrapResources { ref issues }
                    if issues.len() == 1
                        && issues[0].resource == name
                        && matches!(&issues[0].defect,
                            ClassicGuiBootstrapDefect::Malformed { .. })
            ));
        }
    }

    #[test]
    fn mid_round_graphics_group_arrival_rebinds_changed_sheets_only() {
        // C4GraphicsResource::Init stays re-callable for network overloadings
        // (C4GraphicsResource.cpp:278-292): a Graphics-bearing group arriving
        // mid-round re-registers (RegisterMainGroups, :376-382) and LoadFile
        // reloads only sheets whose winning group id changed (:418-470).
        let _lock = env_lock().lock();
        let user_data = tempdir().expect("mid-round user data");
        let (_guard, paths) = exact_loader_test_paths(user_data.path(), None);
        let packs = tempdir().expect("mid-round packs");
        let pack_sheet = |module: &str, sheets: &[(&str, [u8; 4])]| {
            let pack = install_network_definition_pack(packs.path(), module, "MIDR");
            let graphics = pack.join("Graphics.c4g");
            fs::create_dir_all(&graphics).expect("pack Graphics.c4g");
            for (name, pixel) in sheets {
                image::RgbaImage::from_pixel(8, 4, image::Rgba(*pixel))
                    .save(graphics.join(name))
                    .expect("write pack sheet");
            }
            pack
        };
        let pack_a = pack_sheet(
            "PackA.c4d",
            &[
                ("GUICaption.png", [0x31, 0x11, 0x11, 0xff]),
                ("GUIScroll.png", [0x32, 0x22, 0x22, 0xff]),
            ],
        );
        let pack_b = pack_sheet("PackB.c4d", &[("GUIIcons.png", [0x33, 0x33, 0x33, 0xff])]);
        let pack_c = install_network_definition_pack(packs.path(), "PackC.c4d", "MIDC");
        let corrupt_graphics = pack_c.join("Graphics.c4g");
        fs::create_dir_all(&corrupt_graphics).expect("corrupt pack Graphics.c4g");
        fs::write(corrupt_graphics.join("GUISubmenu.png"), b"not a png")
            .expect("write corrupt GUISubmenu");
        let round = tempdir().expect("active round scenario");
        let combined = round.path().join("Combined2.c4s");
        fs::create_dir_all(&combined).expect("combined scenario group");
        fs::write(combined.join("Scenario.txt"), "[Head]\nTitle=Mid Round\n")
            .expect("combined scenario core");

        let arrival = |id: i32, path: &Path| NetworkEvent::ResourceComplete {
            resource_id: id,
            core: clonk_engine::NetworkResourceCore {
                resource_type: clonk_network::HostResourceType::Definitions as u8,
                id,
                loadable: true,
                filename: clonk_engine::LegacyCString::from_bytes(b"MidRound.c4d".to_vec())
                    .expect("valid mid-round pack name"),
                ..Default::default()
            },
            path: path.to_path_buf(),
            local: true,
        };
        let sheet_ptr = |app: &GameApp, name: &str| {
            app.assets
                .startup_dialog_images
                .get(name)
                .expect("global GUI sheet")
                .pixels()
                .as_ptr()
        };

        let mut app = new_menu_app_with_paths(320, 200, &paths);
        let mut frontend = FrontendScenario::fallback();
        frontend.identifier = "Combined2.c4s".to_string();
        frontend.path = Some(combined.clone());
        app.active_scenario = Some(frontend.clone());
        app.active_definition_load = Some(ScenarioDefinitionLoad::Fixed {
            modules: Vec::new(),
            definition_root: None,
        });
        app.network_mode = Some(NetworkMode::Client(ClientSettings::new(
            SocketAddr::from(([127, 0, 0, 1], 11_112)),
            "Client",
        )));
        let (manager, events) = NetworkManager::test_stub();
        app.network = Some(manager);

        events
            .send(arrival(31, &pack_a))
            .expect("queue first mid-round arrival");
        app.process_network_events()
            .expect("first arrival re-registers and rebinds");
        assert_eq!(
            app.assets
                .startup_dialog_images
                .get("GUICaption.png")
                .expect("overloaded caption")
                .pixels()[..4],
            [0x31, 0x11, 0x11, 0xff]
        );
        assert_eq!(
            app.assets
                .startup_dialog_images
                .get("GUIScroll.png")
                .expect("overloaded scroll")
                .pixels()[..4],
            [0x32, 0x22, 0x22, 0xff]
        );
        assert!(app.active_global_gui_failures.is_empty());
        let caption_ptr = sheet_ptr(&app, "GUICaption.png");
        let scroll_ptr = sheet_ptr(&app, "GUIScroll.png");

        // A second Graphics-bearing arrival rebinds only its own new winner:
        // unchanged winners keep their loaded surfaces (the group-id cache).
        events
            .send(arrival(32, &pack_b))
            .expect("queue second mid-round arrival");
        app.process_network_events()
            .expect("second arrival rebinds only changed sheets");
        assert_eq!(sheet_ptr(&app, "GUICaption.png"), caption_ptr);
        assert_eq!(sheet_ptr(&app, "GUIScroll.png"), scroll_ptr);
        assert_eq!(
            app.assets
                .startup_dialog_images
                .get("GUIIcons.png")
                .expect("overloaded icons")
                .pixels()[..4],
            [0x33, 0x33, 0x33, 0xff]
        );
        let icons_ptr = sheet_ptr(&app, "GUIIcons.png");

        // A re-arrival of an already registered root is the
        // idRegisteredMainGroupSetFiles skip: every winner stays loaded.
        events
            .send(arrival(33, &pack_a))
            .expect("queue duplicate mid-round arrival");
        app.process_network_events()
            .expect("duplicate arrival reloads nothing");
        assert_eq!(sheet_ptr(&app, "GUICaption.png"), caption_ptr);
        assert_eq!(sheet_ptr(&app, "GUIScroll.png"), scroll_ptr);
        assert_eq!(sheet_ptr(&app, "GUIIcons.png"), icons_ptr);

        // A malformed winner in an arriving group fails typed before pixels.
        events
            .send(arrival(34, &pack_c))
            .expect("queue corrupt mid-round arrival");
        let error = app
            .process_network_events()
            .expect_err("a malformed mid-round winner fails typed");
        assert_engine_parity_boundary(
                error,
                ClassicParityBoundary::GlobalGuiBootstrapResources {
                    issues: vec![ClassicGuiBootstrapIssue::malformed(
                        "GUISubmenu",
                        "a readable selected bmp/jpeg/jpg/png RGBA surface",
                        format!(
                            "{root}:GUISubmenu.png: failed to decode exact classic image entry `GUISubmenu.png` from {root}",
                            root = corrupt_graphics.display()
                        ),
                    )],
                },
            );
        assert_eq!(sheet_ptr(&app, "GUICaption.png"), caption_ptr);
        assert_eq!(sheet_ptr(&app, "GUIScroll.png"), scroll_ptr);
        assert_eq!(sheet_ptr(&app, "GUIIcons.png"), icons_ptr);

        // A host rebuilds its set from the retained activation inputs
        // (OpenScenario chain + effective definition roots) and overloads
        // identically.
        let mut host_app = new_menu_app_with_paths(320, 200, &paths);
        host_app.active_scenario = Some(frontend);
        host_app.active_definition_load = Some(ScenarioDefinitionLoad::Fixed {
            modules: Vec::new(),
            definition_root: None,
        });
        host_app.network_mode = Some(NetworkMode::Host(HostSettings {
            bind_addr: SocketAddr::from(([127, 0, 0, 1], 11_113)),
            player_name: "Host".to_string(),
            prepared: None,
        }));
        let (host_manager, host_events) = NetworkManager::test_stub();
        host_app.network = Some(host_manager);
        host_events
            .send(arrival(41, &pack_a))
            .expect("queue host mid-round arrival");
        host_app
            .process_network_events()
            .expect("host arrival re-registers and rebinds");
        assert_eq!(
            host_app
                .assets
                .startup_dialog_images
                .get("GUICaption.png")
                .expect("host overloaded caption")
                .pixels()[..4],
            [0x31, 0x11, 0x11, 0xff]
        );
    }

    #[test]
    fn global_gui_guard_precedes_every_overlay_constructor_without_mutation() {
        let boundary = || ClassicParityBoundary::GlobalGuiBootstrapResources {
            issues: vec![ClassicGuiBootstrapIssue::missing("GUISpinBoxArrow")],
        };
        let check =
            |app: &GameApp, before: RuntimeGlobalUiSnapshot, error: EngineError, label: &str| {
                assert_engine_parity_boundary(error, boundary());
                assert_eq!(runtime_global_ui_snapshot(app), before, "{label}");
            };

        let mut definition = new_classic_menu_app(640, 480);
        definition
            .open_context_menu_at(
                vec![ContextMenuEntry::<AppContextMenuCommand>::new("Retained")],
                GuiPoint::new(20.0, 20.0),
            )
            .expect("open retained context");
        remove_global_gui_sheet(&mut definition, "GUISpinBoxArrow.png");
        let before = runtime_global_ui_snapshot(&definition);
        let error = definition
            .open_definition_selector(FrontendScenario::fallback())
            .expect_err("definition selector must reject before closing context");
        check(
            &definition,
            before,
            error,
            "definition selector constructor",
        );

        let mut context = new_classic_menu_app(640, 480);
        remove_global_gui_sheet(&mut context, "GUISpinBoxArrow.png");
        let before = runtime_global_ui_snapshot(&context);
        let error = context
            .open_context_menu_at(
                vec![ContextMenuEntry::<AppContextMenuCommand>::new(
                    "Never opened",
                )],
                GuiPoint::new(20.0, 20.0),
            )
            .expect_err("context constructor must reject before modal mutation");
        check(&context, before, error, "context-menu constructor");

        let mut input = new_classic_menu_app(640, 480);
        input
            .open_context_menu_at(
                vec![ContextMenuEntry::<AppContextMenuCommand>::new("Retained")],
                GuiPoint::new(20.0, 20.0),
            )
            .expect("open retained context");
        remove_global_gui_sheet(&mut input, "GUISpinBoxArrow.png");
        let before = runtime_global_ui_snapshot(&input);
        let error = input
            .open_game_option_input_dialog(GameOptionInputDialogRequest {
                kind: GameOptionInputKind::Password,
                message: "Password",
                caption: "Password",
                icon: clonk_frontend::game_option_buttons::GameOptionIcon::Locked,
                max_text: 31,
                initial_text: "retained".to_string(),
                chat_layout: false,
            })
            .expect_err("input constructor must reject before closing context");
        check(&input, before, error, "game-option input constructor");

        let mut message = new_running_sandbox_app();
        message
            .ingame_menu
            .replace(message.local_owner, Some(IngameMenuState::surrender_menu()));
        message.pressed_engine_keys.insert(VirtualKeyCode::A);
        remove_global_gui_sheet(&mut message, "GUISpinBoxArrow.png");
        let before = runtime_global_ui_snapshot(&message);
        let error = message
            .push_message_dialog(
                clonk_frontend::message_dialog::MessageDialogState::regular_ok(
                    "Never opened",
                    "Message",
                    clonk_frontend::message_dialog::MessageDialogIcon::NOTIFY,
                ),
                MessageDialogContinuation::None,
            )
            .expect_err("message constructor must reject before controls or menu mutation");
        check(&message, before, error, "message-dialog constructor");

        let mut game_over = new_running_sandbox_app();
        game_over.ingame_menu.replace(
            game_over.local_owner,
            Some(IngameMenuState::surrender_menu()),
        );
        game_over.scoreboard_initial_reconcile_pending = true;
        game_over.pressed_engine_keys.insert(VirtualKeyCode::A);
        remove_global_gui_sheet(&mut game_over, "GUISpinBoxArrow.png");
        let before = runtime_global_ui_snapshot(&game_over);
        let error = game_over
            .handle_game_over()
            .expect_err("game-over constructor must reject before recording/UI mutation");
        check(&game_over, before, error, "game-over constructor");
    }

    #[test]
    fn m06_l033_startup_fade_modulates_retained_draws_and_text_like_cpp() {
        let source = [200_u8, 100, 50, 128];
        let faded_batch = |opacity| {
            let mut surface = Surface::new(2, 2, PixelFormat::Rgba8888);
            surface.begin_gpu_scene_capture();
            surface.fill(Color::new(source[0], source[1], source[2], source[3]));
            let mut batch = NativePresentationBatch {
                logical_layer: None,
                clip: None,
                native_loader_text: false,
                text: vec![clonk_graphics::clonk_font::CapturedClonkText {
                    role: clonk_graphics::clonk_font::ClonkFontRole::GuiText,
                    x: 0,
                    y: 0,
                    text: "fade".to_owned(),
                    color: source,
                    align: clonk_graphics::clonk_font::TextAlign::Left,
                    markup: false,
                    clip: None,
                    gamma: None,
                    images: Vec::new(),
                }],
                fonts: None,
                gpu_recorder: surface.take_gpu_scene_capture(),
            };
            apply_startup_fade_to_batch(&mut batch, opacity)
                .expect("byte-derived fade colors are exactly representable");
            let scene = batch
                .gpu_recorder
                .take()
                .expect("captured fade command")
                .into_scene([2, 2], Color::transparent(), startup_gamma());
            let clonk_graphics::GpuCommand::Solid { vertices, .. } = &scene.commands[0] else {
                panic!("fill did not produce a retained solid command");
            };
            (
                vertices[0]
                    .color
                    .map(|channel| (channel * 255.0).round() as u8),
                batch.text[0].color,
            )
        };

        let incoming = startup_dialog_fade_opacity(10);
        let outgoing = startup_dialog_fade_opacity(90);
        for opacity in [outgoing, incoming] {
            let expected = clonk_graphics::gpu_scene::modulate_rgba8_by_packed_c4(
                source,
                startup_fade_packed_modulation(opacity),
            );
            let (draw, text) = faded_batch(opacity);
            assert_eq!(draw, expected, "retained draw at opacity {opacity}");
            assert_eq!(text, expected, "semantic text at opacity {opacity}");
        }
        assert_ne!(faded_batch(outgoing), faded_batch(incoming));
        assert_eq!(
            faded_batch(u8::MAX),
            (source, source),
            "C4GUI disables modulation at the fully visible endpoint"
        );

        let mut app = new_real_menu_app(320, 200);
        app.startup_dialog_fade = None;
        app.graphics.set_runtime_sprite_filtering(1.0, false);
        app.configure_native_startup_fonts(1.0, false);
        app.handle_main_menu_activation(MainMenuItem::About)
            .expect("start retained Main-to-About transition");
        let presentation = retained_test_presentation(&app);
        let frame = app
            .render_retained_gpu_frame(presentation)
            .expect("retain first startup fade frame");
        assert_retained_frame_has_commands("startup fade", &frame);
        assert!(
            frame.layers.len() >= 3,
            "startup fade must retain underlay, outgoing, and incoming painter layers"
        );
    }

    #[test]
    fn m06_l033_surface_error_policy_rebuilds_or_retries_only_recoverable_errors() {
        let classify = |surface| {
            retained_gpu_present_recovery(
                &anyhow::Error::new(pixels::Error::Surface(surface)).context("retained presentation"),
            )
        };
        assert_eq!(
            classify(pixels::wgpu::SurfaceError::Lost),
            RetainedGpuPresentRecovery::RebuildDevice
        );
        assert_eq!(
            classify(pixels::wgpu::SurfaceError::Outdated),
            RetainedGpuPresentRecovery::RebuildDevice
        );
        assert_eq!(
            classify(pixels::wgpu::SurfaceError::Timeout),
            RetainedGpuPresentRecovery::Retry
        );
        assert_eq!(
            classify(pixels::wgpu::SurfaceError::OutOfMemory),
            RetainedGpuPresentRecovery::Fatal
        );
        let observed_device_loss =
            anyhow::Error::new(gpu_renderer::GpuRendererError::DeviceRecreationRequired {
                reason: gpu_renderer::RetainedGpuRecreateReason::DeviceLost,
                detail: "Parent device is lost".to_owned(),
            })
            .context("retained presentation");
        assert_eq!(
            retained_gpu_present_recovery(&observed_device_loss),
            RetainedGpuPresentRecovery::RebuildDevice
        );
        let queue_submit_loss =
            "Error in Queue::submit: Validation Error: Parent device is lost".to_owned();
        let detail = wgpu_device_loss_panic_detail(&queue_submit_loss)
            .expect("wgpu 0.16 fatal submit loss remains recoverable");
        assert_eq!(
            retained_gpu_present_recovery(&retained_gpu_device_loss_error(detail)),
            RetainedGpuPresentRecovery::RebuildDevice
        );
        let unrelated_panic = "index out of bounds".to_owned();
        assert_eq!(wgpu_device_loss_panic_detail(&unrelated_panic), None);
        let validation = anyhow::Error::new(gpu_renderer::GpuRendererError::DeviceFatal {
            reason: gpu_renderer::RetainedGpuFatalReason::Validation,
            detail: "invalid bind group".to_owned(),
        });
        assert_eq!(
            retained_gpu_present_recovery(&validation),
            RetainedGpuPresentRecovery::Fatal
        );
    }

    struct FakeSystemFontProvider {
        family: String,
        bytes: Arc<[u8]>,
        face_index: u32,
        requests: Mutex<Vec<(String, u32)>>,
    }

    impl FakeSystemFontProvider {
        fn new(family: impl Into<String>, bytes: impl Into<Arc<[u8]>>) -> Self {
            Self {
                family: family.into(),
                bytes: bytes.into(),
                face_index: 0,
                requests: Mutex::new(Vec::new()),
            }
        }

        fn requests(&self) -> Vec<(String, u32)> {
            self.requests.lock().unwrap().clone()
        }

        fn clear_requests(&self) {
            self.requests.lock().unwrap().clear();
        }
    }

    impl system_fonts::SystemFontProvider for FakeSystemFontProvider {
        fn resolve(&self, family: &str, weight: u32) -> Option<system_fonts::SystemFontFace> {
            self.requests
                .lock()
                .unwrap()
                .push((family.to_string(), weight));
            self.family
                .eq_ignore_ascii_case(family)
                .then(|| system_fonts::SystemFontFace {
                    bytes: self.bytes.clone(),
                    face_index: self.face_index,
                })
        }
    }

    #[test]
    fn l091_system_family_fallback_preserves_precedence_and_failure_boundary() {
        let _lock = env_lock().lock();
        let root = tempdir().expect("system font fixture");
        install_global_gui_and_loader_test_root(root.path());
        let user = root.path().join("user");
        let _guard = EnvGuard::set(&[
            ("LC_INSTALL_ROOT", Some(root.path())),
            ("LC_CONTENT_DIR", None),
            ("LC_USER_DATA_DIR", Some(user.as_path())),
        ]);
        let paths = AppPaths::discover().expect("system font paths");
        paths.ensure_user_dirs().expect("system font user dirs");
        let font_bytes: Arc<[u8]> = fs::read(root.path().join("planet/System.c4g/Endeavour.ttf"))
            .expect("fixture vector font")
            .into();
        let provider = FakeSystemFontProvider::new("Mock System Face", font_bytes.clone());

        let gui = resolve_classic_font_bundle_for_request_with_system_fonts(
            &paths,
            "mock system face",
            14,
            &[],
            &[],
            &provider,
        )
        .expect("exact case-insensitive system family resolves GUI fonts");
        let native = gui
            .native_source
            .expect("raw-size system family keeps a scale-native source");
        assert_eq!(native.bytes.as_ref(), font_bytes.as_ref());
        assert_eq!(native.face_index, 0);
        resolve_classic_startup_font_bundle_for_request_with_system_fonts(
            &paths,
            "MOCK SYSTEM FACE",
            14,
            &[],
            &[],
            &provider,
        )
        .expect("same system family resolves the startup book fonts");
        assert!(
            provider.requests().iter().all(|(_, weight)| *weight == 400),
            "the requested FontDef weight reaches system lookup"
        );

        provider.clear_requests();
        resolve_classic_font_bundle_for_request_with_system_fonts(
            &paths,
            "Endeavour",
            14,
            &[],
            &[],
            &provider,
        )
        .expect("catalog font wins before system lookup");
        assert!(provider.requests().is_empty());

        let explicit_file = tempfile::Builder::new()
            .prefix("lc-font-")
            .suffix(".ttf")
            .tempfile_in(".")
            .expect("short explicit font file");
        fs::write(explicit_file.path(), font_bytes.as_ref()).expect("explicit font bytes");
        let explicit_face = explicit_file
            .path()
            .file_name()
            .and_then(|name| name.to_str())
            .expect("UTF-8 fixture filename");
        assert!(
            clonk_script::c4_string_bytes(explicit_face).len() <= 30,
            "the explicit-file precedence fixture must fit C4MaxName"
        );
        resolve_classic_font_bundle_for_request_with_system_fonts(
            &paths,
            explicit_face,
            14,
            &[],
            &[],
            &provider,
        )
        .expect("readable explicit file wins before system lookup");
        assert!(provider.requests().is_empty());

        let missing = resolve_classic_font_bundle_for_request_with_system_fonts(
            &paths,
            "Definitely Missing Font",
            14,
            &[],
            &[],
            &provider,
        )
        .err()
        .expect("genuinely missing family keeps the typed failure boundary");
        assert!(missing.to_string().contains("is unavailable"));

        let malformed = FakeSystemFontProvider::new("Malformed System Face", b"not a font".as_slice());
        let error = resolve_classic_font_bundle_for_request_with_system_fonts(
            &paths,
            "Malformed System Face",
            14,
            &[],
            &[],
            &malformed,
        )
        .err()
        .expect("malformed system bytes cannot be substituted or accepted");
        assert!(error
            .to_string()
            .contains("failed to initialize classic vector font"));
    }

    #[test]
    fn font_catalog_skips_bad_optional_candidates_and_falls_through_matching_faces() {
        let _lock = env_lock().lock();
        let root = tempdir().expect("font candidate fallback fixture");
        install_global_gui_and_loader_test_root(root.path());
        let user = root.path().join("user");
        let _guard = EnvGuard::set(&[
            ("LC_INSTALL_ROOT", Some(root.path())),
            ("LC_CONTENT_DIR", None),
            ("LC_USER_DATA_DIR", Some(user.as_path())),
        ]);
        let paths = AppPaths::discover().expect("font fallback paths");
        paths.ensure_user_dirs().expect("font fallback user dirs");
        let system_font =
            fs::read(root.path().join("planet/System.c4g/Endeavour.ttf")).expect("fixture vector font");

        // FreeType accepts trailing bytes in an sfnt. Distinct sentinels let
        // the winning same-face registration be observed without changing
        // either usable font's rendered face.
        let mut oldest_font = system_font.clone();
        oldest_font.extend_from_slice(b"oldest-registration");
        let mut newest_usable_font = system_font.clone();
        newest_usable_font.extend_from_slice(b"newest-usable-registration");
        let oldest = Group::from_raw_memory(
            PathBuf::from("oldest-fonts.c4g"),
            packed_test_group(&[("FallbackFace.ttf", false, &oldest_font)]),
        )
        .expect("oldest font group");
        let newest_usable = Group::from_raw_memory(
            PathBuf::from("newest-usable-fonts.c4g"),
            packed_test_group(&[("FallbackFace.ttf", false, &newest_usable_font)]),
        )
        .expect("newest usable font group");

        let mut unreadable_vector = packed_test_group(&[("Unreadable.ttf", false, b"optional vector")]);
        make_packed_test_entry_unreadable(&mut unreadable_vector, 0);
        let unreadable_vector = Group::from_raw_memory(
            PathBuf::from("unreadable-vector-fonts.c4g"),
            unreadable_vector,
        )
        .expect("unreadable vector group still opens");
        assert!(unreadable_vector.read_file("Unreadable.ttf").is_err());

        let mut unreadable_definitions =
            packed_test_group(&[("Fonts.txt", false, b"[Font]\nName=MustNotLoad\n")]);
        make_packed_test_entry_unreadable(&mut unreadable_definitions, 0);
        let unreadable_definitions = Group::from_raw_memory(
            PathBuf::from("unreadable-font-definitions.c4g"),
            unreadable_definitions,
        )
        .expect("unreadable font-definition group still opens");
        assert!(unreadable_definitions.read_file("Fonts.txt").is_err());

        let corrupt_face = Group::from_raw_memory(
            PathBuf::from("corrupt-font-face.c4g"),
            packed_test_group(&[("FallbackFace.ttf", false, b"not a font")]),
        )
        .expect("corrupt readable font group");
        assert_eq!(
            corrupt_face
                .read_file("FallbackFace.ttf")
                .expect("corrupt face remains readable")
                .as_slice(),
            b"not a font"
        );

        let registration = |registration_order, group| LoaderGroupRegistration {
            priority: 100,
            registration_order,
            group,
        };
        let registrations = vec![
            registration(0, oldest),
            registration(1, newest_usable),
            registration(2, unreadable_vector.clone()),
            registration(3, unreadable_definitions.clone()),
            registration(4, corrupt_face.clone()),
        ];
        let provider = FakeSystemFontProvider::new("FallbackFace", system_font.clone());

        let bundle = resolve_classic_font_bundle_for_request_with_system_fonts(
            &paths,
            "FallbackFace",
            14,
            &registrations,
            &[],
            &provider,
        )
        .expect("corrupt newest face falls through to the newest usable registration");
        assert_eq!(
            bundle
                .native_source
                .expect("matching winning candidates retain a native source")
                .bytes
                .as_ref(),
            newest_usable_font.as_slice()
        );
        resolve_classic_startup_font_bundle_for_request_with_system_fonts(
            &paths,
            "FallbackFace",
            14,
            &registrations,
            &[],
            &provider,
        )
        .expect("startup fonts use the same full candidate chain");
        assert!(
            provider.requests().is_empty(),
            "the system face is attempted only after every matching registered face"
        );

        let bad_registrations = [
            registration(2, unreadable_vector),
            registration(3, unreadable_definitions),
            registration(4, corrupt_face),
        ];
        let system_bundle = resolve_classic_font_bundle_for_request_with_system_fonts(
            &paths,
            "FallbackFace",
            14,
            &bad_registrations,
            &[],
            &provider,
        )
        .expect("system face follows the exhausted corrupt registered chain");
        assert_eq!(
            system_bundle
                .native_source
                .expect("system fallback supplies a native source")
                .bytes
                .as_ref(),
            system_font.as_slice()
        );
        assert!(!provider.requests().is_empty());
        assert!(provider
            .requests()
            .iter()
            .all(|(family, weight)| family == "FallbackFace" && *weight == 400));
    }

    #[test]
    fn general_font_size_sixteen_builds_the_cpp_derived_app_bundle() {
        let _lock = env_lock().lock();
        let root = tempdir().expect("configured font fixture");
        install_global_gui_and_loader_test_root(root.path());
        let user = root.path().join("user");
        let _guard = EnvGuard::set(&[
            ("LC_INSTALL_ROOT", Some(root.path())),
            ("LC_CONTENT_DIR", None),
            ("LC_USER_DATA_DIR", Some(user.as_path())),
        ]);
        let paths = AppPaths::discover().expect("configured font paths");
        paths.ensure_user_dirs().expect("configured font user dirs");
        fs::write(
            paths.config_file(),
            "[General]\nFontName=Endeavour\nFontSize=16\n",
        )
        .expect("configured font values");

        let bundle = resolve_classic_font_bundle(&paths, None, &[], &[])
            .expect("configured font bundle resolves");
        assert_eq!(bundle.fonts.mini.line_height, 20); // 16*12/14 = 13px
        assert_eq!(bundle.fonts.main_small.line_height, 22); // 16*13/14 = 14px
        assert_eq!(bundle.fonts.text.line_height, 25); // 16px
        assert_eq!(bundle.fonts.caption.line_height, 28); // 16*16/14 = 18px
        assert_eq!(bundle.fonts.title.line_height, 39); // 16*22/14 = 25px
        assert_eq!(bundle.tooltip.line_height, 25);
        assert_eq!(bundle.tooltip.h_space, 0);
        assert!(
            bundle.native_source.is_none(),
            "the fixed native builder must not impersonate a size-16 role map"
        );
    }

    #[test]
    fn graphics_group_double_reversal_makes_actual_parent_beat_later_origin() {
        let directory = tempdir().expect("graphics tiers");
        let actual = directory.path().join("actual.c4f");
        let origin = directory.path().join("origin.c4f");
        let fallback = directory.path().join("Graphics.c4g");
        for (path, pixel) in [(&actual, [255, 0, 0, 255]), (&origin, [0, 0, 255, 255])] {
            let graphics = path.join("Graphics.c4g");
            fs::create_dir_all(&graphics).expect("local graphics");
            write_preview_png(&graphics.join("GUIProgress.png"), pixel);
        }
        fs::create_dir(&fallback).expect("fallback graphics");
        let registrations = vec![
            LoaderGroupRegistration {
                priority: 101,
                registration_order: 1,
                group: Group::open(&actual).expect("actual group"),
            },
            LoaderGroupRegistration {
                priority: 101,
                registration_order: 2,
                group: Group::open(&origin).expect("origin group"),
            },
        ];
        let graphics = loader_graphics_registrations(&registrations).expect("graphics children");
        let image = load_named_graphics_image(
            "GUIProgress",
            &graphics,
            &Group::open(&fallback).expect("fallback group"),
        )
        .expect("progress image");
        assert_eq!(image.pixels(), [255, 0, 0, 255]);
    }

    #[test]
    fn graphics_stem_resolver_matches_find_suitable_file_extension_bug() {
        let directory = tempdir().expect("global GUI resolver");
        let base = directory.path().join("base.c4g");
        let override_group = directory.path().join("override.c4g");
        fs::create_dir(&base).expect("base graphics group");
        fs::create_dir(&override_group).expect("override graphics group");
        write_preview_png(&base.join("GUIBigArrows.png"), [255, 0, 0, 255]);
        write_preview_image(
            &override_group.join("GUIBigArrows.bmp"),
            [0, 0, 255, 255],
            image::ImageFormat::Bmp,
        );
        let registrations = vec![LoaderGroupRegistration {
            priority: 200,
            registration_order: 0,
            group: Group::open(&override_group).expect("override group"),
        }];
        let base_group = Group::open(&base).expect("base group");
        let selected = select_named_graphics_image_source("GUIBigArrows", &registrations, &base_group)
            .expect("select globally later png");
        assert!(!selected.from_registration);
        assert_eq!(
            selected.source.entry.relative_path,
            PathBuf::from("GUIBigArrows.png")
        );
        assert_eq!(
            decode_selected_loader(&selected.source)
                .expect("decode selected base png")
                .pixels(),
            [255, 0, 0, 255],
            "FindSuitableFile never updates iPrio, so a base png replaces a scenario-priority bmp"
        );

        write_preview_png(&override_group.join("GUIBigArrows.png"), [0, 255, 0, 255]);
        let registrations = vec![LoaderGroupRegistration {
            priority: 200,
            registration_order: 0,
            group: Group::open(&override_group).expect("reopen override group"),
        }];
        let selected = select_named_graphics_image_source("GUIBigArrows", &registrations, &base_group)
            .expect("select equal-priority last extension");
        assert_eq!(
            selected.source.entry.relative_path,
            PathBuf::from("GUIBigArrows.png")
        );
        assert_eq!(
            decode_selected_loader(&selected.source)
                .expect("decode selected png")
                .pixels(),
            [0, 255, 0, 255]
        );
    }

    #[test]
    fn game_graphics_refreshes_hud_cursor_and_palette_then_reverts_at_preinit() {
        let directory = tempdir().expect("game graphics refresh fixture");
        let base_path = directory.path().join("base.c4g");
        let scenario_path = directory.path().join("scenario.c4g");
        let sized_path = directory.path().join("sized.c4g");
        for path in [&base_path, &scenario_path, &sized_path] {
            fs::create_dir(path).expect("graphics group");
        }
        let write_solid = |path: &Path, width: u32, height: u32, pixel: [u8; 4]| {
            image::RgbaImage::from_pixel(width, height, image::Rgba(pixel))
                .save(path)
                .expect("write solid graphics image");
        };
        for stem in [
            "Player",
            "Flag",
            "Crew",
            "Score",
            "Wealth",
            "Captain",
            "Fire",
            "Menu",
            "UpperBoard",
            "Logo",
            "Construction",
            "Energy",
            "Magic",
            "Arrow",
            "Exit",
            "Hand",
            "Build",
            "EnergyBars",
            "SelectMark",
            "Control",
            "Gamepad",
            "Background",
            "Options",
            "Liquid",
        ] {
            write_preview_png(&base_path.join(format!("{stem}.png")), [9, 8, 7, 255]);
        }
        write_preview_image(
            &base_path.join("Rank.bmp"),
            [20, 30, 40, 255],
            image::ImageFormat::Bmp,
        );
        for stem in [
            "CursorXXXXXLarge",
            "CursorXXXXLarge",
            "CursorXXXLarge",
            "CursorXXLarge",
            "CursorXLarge",
            "CursorLarge",
            "CursorMedium",
            "CursorSmall",
        ] {
            let pixel = if stem == "CursorLarge" {
                [10, 20, 30, 255]
            } else {
                [1, 1, 1, 255]
            };
            write_solid(&base_path.join(format!("{stem}.png")), 78, 2, pixel);
        }
        let mut base_palette = vec![0_u8; GamePalette::BYTE_LEN];
        base_palette[6 * 3..6 * 3 + 3].copy_from_slice(&[63, 63, 63]);
        base_palette[10 * 3..10 * 3 + 3].copy_from_slice(&[10, 0, 0]);
        fs::write(base_path.join("C4.pal"), base_palette).expect("base C4.pal");
        let base = Group::open(&base_path).expect("base graphics");
        let startup =
            resolve_game_graphics_resources(&[], &base, None, true).expect("startup graphics bundle");

        let mut scenario_palette = vec![0_u8; GamePalette::BYTE_LEN];
        scenario_palette[6 * 3..6 * 3 + 3].copy_from_slice(&[3, 4, 5]);
        scenario_palette[10 * 3..10 * 3 + 3].copy_from_slice(&[1, 2, 3]);
        fs::write(scenario_path.join("C4.pal"), scenario_palette).expect("scenario C4.pal");
        write_preview_png(&scenario_path.join("Control.png"), [80, 90, 100, 255]);
        write_preview_png(&scenario_path.join("Gamepad.png"), [140, 150, 160, 255]);
        write_preview_png(&scenario_path.join("Options.png"), [110, 120, 130, 255]);
        write_preview_png(&scenario_path.join("Liquid.png"), [170, 180, 190, 255]);
        let mut embedded_palette = [[0_u8; 3]; 256];
        embedded_palette[10] = [0, 255, 0];
        let indexed_rank = clonk_resources::bitmap::IndexedBitmap {
            width: 1,
            height: 1,
            indices: vec![10],
        }
        .encode_with_palette(&embedded_palette)
        .expect("indexed scenario Rank.bmp");
        fs::write(scenario_path.join("Rank.bmp"), indexed_rank).expect("scenario Rank.bmp");
        write_solid(
            &scenario_path.join("Cursor.png"),
            39 * 13,
            13,
            [200, 0, 0, 255],
        );
        write_solid(
            &scenario_path.join("CursorLarge.png"),
            39 * 13,
            13,
            [0, 200, 0, 255],
        );
        let scenario_registrations = [LoaderGroupRegistration {
            priority: 200,
            registration_order: 0,
            group: Group::open(&scenario_path).expect("scenario graphics"),
        }];
        let active = resolve_game_graphics_resources(
            &scenario_registrations,
            &base,
            Some(Arc::clone(&startup.cursor_atlas)),
            true,
        )
        .expect("active scenario graphics bundle");
        assert_eq!(
            active
                .hud_graphics
                .rank
                .as_ref()
                .expect("scenario rank")
                .pixels(),
            [4, 8, 12, 255],
            "indexed BMP pixels use the selected game palette, not the embedded BMP palette"
        );
        assert_eq!(
            active
                .hud_graphics
                .control
                .as_ref()
                .expect("scenario control")
                .pixels(),
            [80, 90, 100, 255]
        );
        assert_eq!(
            active
                .hud_graphics
                .gamepad
                .as_ref()
                .expect("scenario gamepad")
                .pixels(),
            [140, 150, 160, 255],
            "the active group set reloads the startup gamepad facet"
        );
        assert_eq!(active.palette.color(6), Color::opaque(12, 16, 20));
        assert_eq!(
            active
                .options
                .as_deref()
                .expect("scenario options")
                .pixels(),
            [110, 120, 130, 255]
        );
        assert_eq!(
            active
                .liquid_animation
                .as_deref()
                .expect("scenario liquid animation")
                .pixels(),
            [170, 180, 190, 255]
        );
        assert_eq!(
            active
                .cursor_atlas
                .image_for_resolution(1280)
                .expect("cached large cursor")
                .pixels()[..4],
            [10, 20, 30, 255],
            "a valid legacy Cursor suppresses sized overrides, then the cached pre-game size wins"
        );

        write_solid(
            &sized_path.join("CursorLarge.png"),
            39 * 13,
            13,
            [0, 200, 0, 255],
        );
        let sized = resolve_game_graphics_resources(
            &[LoaderGroupRegistration {
                priority: 200,
                registration_order: 0,
                group: Group::open(&sized_path).expect("sized cursor graphics"),
            }],
            &base,
            Some(Arc::clone(&startup.cursor_atlas)),
            true,
        )
        .expect("sized cursor override bundle");
        let sized_large = sized
            .cursor_atlas
            .image_for_resolution(1280)
            .expect("overridden large cursor");
        assert_eq!(sized_large.height(), 13);
        assert_eq!(&sized_large.pixels()[..4], &[0, 200, 0, 255]);
        assert_eq!(
            (
                35 * sized_large.height(),
                36 * sized_large.height(),
                38 * sized_large.height()
            ),
            (455, 468, 494)
        );

        let mut app = new_menu_app(64, 64);
        let startup_hud = app.assets.hud_graphics();
        let startup_palette = app.assets.game_palette();
        app.active_game_graphics = Some(active.clone());
        app.configure_running_state("Overridden".to_string(), 64);
        assert_eq!(
            app.graphics
                .hud_graphics()
                .rank
                .as_ref()
                .expect("active rank")
                .pixels(),
            [4, 8, 12, 255]
        );
        assert_eq!(
            app.graphics.game_palette().color(6),
            Color::opaque(12, 16, 20)
        );
        assert_eq!(
            app.ensure_ingame_menu_gfx()
                .options
                .as_ref()
                .expect("active options sheet")
                .pixels(),
            [110, 120, 130, 255]
        );
        app.resize(80, 80).expect("active graphics survive resize");
        assert_eq!(
            app.graphics
                .hud_graphics()
                .control
                .as_ref()
                .expect("active control after resize")
                .pixels(),
            [80, 90, 100, 255]
        );
        app.return_to_menu();
        assert!(app.active_game_graphics.is_none());
        assert_eq!(app.graphics.hud_graphics().as_ref(), startup_hud.as_ref());
        assert_eq!(
            app.graphics.game_palette().as_ref(),
            startup_palette.as_ref()
        );
    }

    #[test]
    fn initial_extra_override_rebinds_canonical_and_malformed_winner_never_falls_back() {
        let _lock = env_lock().lock();

        let valid = tempdir().expect("valid initial Extra fixture");
        install_global_gui_test_root(valid.path(), None);
        let extra_graphics = valid.path().join("planet/Extra.c4g/Graphics.c4g");
        fs::create_dir_all(&extra_graphics).expect("valid Extra Graphics.c4g");
        write_preview_png(
            &extra_graphics.join("GUIBigArrows.png"),
            [0x12, 0x34, 0x56, 0xff],
        );
        {
            let user = valid.path().join("user");
            let _guard = EnvGuard::set(&[
                ("LC_INSTALL_ROOT", Some(valid.path())),
                ("LC_CONTENT_DIR", None),
                ("LC_USER_DATA_DIR", Some(user.as_path())),
            ]);
            let paths = AppPaths::discover().expect("valid initial Extra paths");
            let assets = FrontendAssets::load(Some(&paths));
            assets
                .require_classic_global_gui_bootstrap_resources(&HashMap::new())
                .expect("valid initial Extra override is accepted");
            assert_eq!(
                assets
                    .startup_dialog_images
                    .get("GUIBigArrows.png")
                    .expect("canonical renderer key rebound")
                    .pixels(),
                [0x12, 0x34, 0x56, 0xff]
            );
        }

        let malformed = tempdir().expect("malformed initial Extra fixture");
        install_global_gui_test_root(malformed.path(), None);
        let extra_graphics = malformed.path().join("planet/Extra.c4g/Graphics.c4g");
        fs::create_dir_all(&extra_graphics).expect("malformed Extra Graphics.c4g");
        write_preview_image(
            &extra_graphics.join("GUIBigArrows.png"),
            [0xaa, 0xbb, 0xcc, 0xff],
            image::ImageFormat::Bmp,
        );
        {
            let user = malformed.path().join("user");
            let _guard = EnvGuard::set(&[
                ("LC_INSTALL_ROOT", Some(malformed.path())),
                ("LC_CONTENT_DIR", None),
                ("LC_USER_DATA_DIR", Some(user.as_path())),
            ]);
            let paths = AppPaths::discover().expect("malformed initial Extra paths");
            let assets = FrontendAssets::load(Some(&paths));
            let error = assets
                .require_classic_global_gui_bootstrap_resources(&HashMap::new())
                .expect_err("malformed winning png must not fall back to base PNG");
            assert!(matches!(
                error,
                ClassicParityBoundary::GlobalGuiBootstrapResources { ref issues }
                    if issues.len() == 1
                        && issues[0].resource == "GUIBigArrows"
                        && matches!(&issues[0].defect,
                            ClassicGuiBootstrapDefect::Malformed { .. })
            ));
            assert!(
                !assets
                    .startup_dialog_images
                    .contains_key("GUIBigArrows.png"),
                "a malformed winning source must remove the lower base image"
            );
        }
    }

    #[test]
    fn real_app_constructor_and_system_font_sources_follow_global_order() {
        let _lock = env_lock().lock();

        let missing = tempdir().expect("missing initial global sheet fixture");
        install_global_gui_test_root(missing.path(), Some("GUISpinBoxArrow.png"));
        {
            let user = missing.path().join("user");
            let _guard = EnvGuard::set(&[
                ("LC_INSTALL_ROOT", Some(missing.path())),
                ("LC_CONTENT_DIR", None),
                ("LC_USER_DATA_DIR", Some(user.as_path())),
            ]);
            let paths = AppPaths::discover().expect("missing-sheet fixture paths");
            let error = test_game_app(320, 200, AudioOptions::default(), Some(&paths))
            .err()
            .expect("real app construction must stop at the global bundle");
            assert_global_gui_boundary(
                &error,
                vec![ClassicGuiBootstrapIssue::missing("GUISpinBoxArrow")],
            );
        }

        let active_mapping = tempdir().expect("active Fonts.txt fixture");
        install_global_gui_test_root(active_mapping.path(), None);
        fs::write(
                active_mapping.path().join("planet/System.c4g/Fonts.txt"),
                "[Font]\nName=Endeavour\nSize=14\nLogFont=Endeavour,12\nSmallFont=Endeavour,13\nFont=Endeavour,14\nCaptionFont=Endeavour,16\nTitleFont=Endeavour,22\n",
            )
            .expect("write active Fonts.txt mapping");
        {
            let user = active_mapping.path().join("user");
            let _guard = EnvGuard::set(&[
                ("LC_INSTALL_ROOT", Some(active_mapping.path())),
                ("LC_CONTENT_DIR", None),
                ("LC_USER_DATA_DIR", Some(user.as_path())),
            ]);
            let paths = AppPaths::discover().expect("active Fonts.txt paths");
            let assets = FrontendAssets::load(Some(&paths));
            assets
                .require_classic_global_gui_bootstrap_resources(&HashMap::new())
                .expect("active RX font mappings resolve");
            let fonts = assets.clonk_fonts.as_deref().expect("mapped GUI fonts");
            assert_eq!(fonts.mini.line_height, 18);
            assert_eq!(fonts.text.line_height, 22);
            assert_eq!(fonts.caption.line_height, 25);
            assert_eq!(fonts.title.line_height, 34);
        }

        let ambiguous = tempdir().expect("ambiguous Endeavour fixture");
        install_global_gui_test_root(ambiguous.path(), None);
        fs::copy(
            ambiguous.path().join("planet/System.c4g/Endeavour.ttf"),
            ambiguous.path().join("planet/System.c4g/Endeavour.otf"),
        )
        .expect("add ambiguous Endeavour source");
        {
            let user = ambiguous.path().join("user");
            let _guard = EnvGuard::set(&[
                ("LC_INSTALL_ROOT", Some(ambiguous.path())),
                ("LC_CONTENT_DIR", None),
                ("LC_USER_DATA_DIR", Some(user.as_path())),
            ]);
            let paths = AppPaths::discover().expect("ambiguous font paths");
            let assets = FrontendAssets::load(Some(&paths));
            assets
                .require_classic_global_gui_bootstrap_resources(&HashMap::new())
                .expect("later matching vector source wins like C++");
        }
    }

    #[test]
    fn l008_scale_fifty_options_close_and_rejected_test_preserve_raw_value() {
        use clonk_frontend::message_dialog::MessageDialogResult;
        use clonk_frontend::startup_options_dlg::OptionsDlgAction;

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
        paths.ensure_user_dirs().expect("create user directories");
        fs::write(
            paths.config_file(),
            "[Graphics]\nResolutionX=800\nResolutionY=600\nScale=50\nDisplayMode=0\n",
        )
        .expect("seed scale-fifty config");

        let mut app = new_menu_app_with_paths(800, 600, &paths);
        app.open_options_menu();
        let graphics = app
            .startup_options_dialog
            .as_ref()
            .expect("Options dialog")
            .graphics();
        assert_eq!(graphics.applied_scale_percent, 50);
        assert_eq!(graphics.proposed_scale_percent, 100);

        let action = graphics
            .request_scale_test()
            .expect("bounded UI offers a scale-one test");
        app.process_options_dialog_actions(vec![OptionsDlgAction::Graphics(action)])
            .expect("begin scale test");
        assert_eq!(
            app.pending_options_display_requests.pop_front(),
            Some(OptionsDisplayRequest::SetScale {
                percent: 100,
                persist: false,
            })
        );
        app.finish_message_dialog(MessageDialogResult::No)
            .expect("reject scale test");
        assert_eq!(
            app.pending_options_display_requests.pop_front(),
            Some(OptionsDisplayRequest::SetScale {
                percent: 50,
                persist: false,
            })
        );
        let graphics = app
            .startup_options_dialog
            .as_ref()
            .expect("Options dialog after rejection")
            .graphics();
        assert_eq!(graphics.applied_scale_percent, 50);
        assert_eq!(graphics.proposed_scale_percent, 100);

        app.process_options_dialog_actions(vec![OptionsDlgAction::Back])
            .expect("save and close Options");
        let persisted = Config::load(paths.config_file()).expect("reload saved config");
        assert_eq!(persisted.get_in(Some("Graphics"), "Scale"), Some("50"));
    }

    #[test]
    fn options_font_size_rebuilds_all_startup_font_sets_and_recreates() {
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
        paths.ensure_user_dirs().expect("create user directories");
        fs::write(
            paths.config_file(),
            "[General]\nFontName=Endeavour\nFontSize=14\n",
        )
        .expect("seed font config");
        let mut app = test_game_app(1280, 720, AudioOptions::default(), Some(&paths))
        .expect("initialise app");
        wait_for_menu(&mut app);
        app.open_options_menu();

        app.apply_options_font_selection(None, Some(16))
            .expect("select size 16");

        assert_eq!(
            app.startup_options_dialog
                .as_ref()
                .unwrap()
                .program()
                .font_size,
            "16"
        );
        assert_eq!(
            app.assets.clonk_fonts.as_ref().unwrap().text.line_height,
            25
        );
        assert_eq!(
            app.assets
                .options_book_fonts
                .as_ref()
                .unwrap()
                .book
                .line_height,
            25
        );
        assert_eq!(app.assets.book_fonts.as_ref().unwrap().text.line_height, 25);
        assert_eq!(
            app.assets
                .plrsel_book_fonts
                .as_ref()
                .unwrap()
                .text
                .line_height,
            25
        );
        let config = Config::load(paths.config_file()).expect("reload selected font");
        assert_eq!(
            config.get_in(Some("General"), "FontName"),
            Some("Endeavour")
        );
        assert_eq!(config.get_in(Some("General"), "FontSize"), Some("16"));

        let before_failure = fs::read(paths.config_file()).expect("font config before failure");
        app.apply_options_font_selection(Some("Definitely Missing Font".to_string()), None)
            .expect("report invalid font");
        let error = app.message_dialogs.last().expect("font error dialog");
        assert_eq!(error.state.message(), "Error initializing fonts");
        assert_eq!(
            error.state.icon(),
            clonk_frontend::message_dialog::MessageDialogIcon::ERROR
        );
        assert_eq!(
            app.startup_options_dialog
                .as_ref()
                .unwrap()
                .program()
                .font_size,
            "16"
        );
        assert_eq!(
            app.assets.clonk_fonts.as_ref().unwrap().text.line_height,
            25
        );
        assert_eq!(fs::read(paths.config_file()).unwrap(), before_failure);

        drop(app);
        let mut restarted = test_game_app(1280, 720, AudioOptions::default(), Some(&paths))
        .expect("restart with persisted font selection");
        wait_for_menu(&mut restarted);
        assert_eq!(
            restarted
                .assets
                .clonk_fonts
                .as_ref()
                .unwrap()
                .text
                .line_height,
            25
        );
        assert_eq!(
            restarted
                .assets
                .options_book_fonts
                .as_ref()
                .unwrap()
                .book
                .line_height,
            25
        );
        assert_eq!(
            restarted
                .assets
                .book_fonts
                .as_ref()
                .unwrap()
                .text
                .line_height,
            25
        );
        assert_eq!(
            restarted
                .assets
                .plrsel_book_fonts
                .as_ref()
                .unwrap()
                .text
                .line_height,
            25
        );
        restarted.open_player_selection_dialog();
        let player_layout = restarted
            .startup_player_dialog
            .as_ref()
            .expect("player dialog with persisted font")
            .layout();
        assert_eq!(player_layout.item_height, 29);
        assert_eq!(
            player_layout,
            clonk_frontend::startup_plrsel::plrsel_layout_with_fonts(
                1280,
                720,
                restarted.assets.clonk_fonts.as_deref().unwrap(),
                restarted.assets.plrsel_book_fonts.as_deref().unwrap(),
            )
        );
        restarted.open_options_menu();
        assert_eq!(
            restarted
                .startup_options_dialog
                .as_ref()
                .unwrap()
                .program()
                .font_size,
            "16"
        );

        fs::remove_file(paths.config_file()).expect("remove font config");
        fs::create_dir(paths.config_file()).expect("block font config writes");
        restarted
            .apply_options_font_selection(None, Some(18))
            .expect("report font persistence failure");
        assert_eq!(
            restarted
                .startup_options_dialog
                .as_ref()
                .unwrap()
                .program()
                .font_size,
            "16"
        );
        assert_eq!(
            restarted
                .assets
                .clonk_fonts
                .as_ref()
                .unwrap()
                .text
                .line_height,
            25
        );
        assert_eq!(
            restarted
                .message_dialogs
                .last()
                .expect("font persistence error")
                .state
                .message(),
            "Error initializing fonts"
        );
    }

    #[test]
    fn l091_options_system_font_rebuilds_persists_and_rolls_back_missing_face() {
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
        paths.ensure_user_dirs().expect("create user directories");
        fs::write(
            paths.config_file(),
            "[General]\nFontName=Endeavour\nFontSize=14\n",
        )
        .expect("seed font config");
        let mut app = test_game_app(1280, 720, AudioOptions::default(), Some(&paths))
        .expect("initialise app");
        wait_for_menu(&mut app);
        app.open_options_menu();
        app.configure_native_startup_fonts(2.0, false);

        let prior_gui = app.assets.clonk_fonts.clone().expect("initial GUI fonts");
        let prior_book = app.assets.book_fonts.clone().expect("initial book fonts");
        let prior_options = app
            .assets
            .options_book_fonts
            .clone()
            .expect("initial options fonts");
        let prior_player = app
            .assets
            .plrsel_book_fonts
            .clone()
            .expect("initial player-selection fonts");
        let font_bytes: Arc<[u8]> = fs::read(install_root.join("planet/System.c4g/Endeavour.ttf"))
            .expect("valid fake-provider font bytes")
            .into();
        let provider = FakeSystemFontProvider::new("Mock System Face", font_bytes.clone());
        let dialog_count = app.message_dialogs.len();

        app.apply_options_font_selection_with_system_fonts(
            Some("Mock System Face".to_string()),
            None,
            &provider,
        )
        .expect("select fake-provider system face");

        assert_eq!(app.message_dialogs.len(), dialog_count);
        assert_eq!(
            app.startup_options_dialog
                .as_ref()
                .expect("reopened Options")
                .program()
                .font_face,
            "Mock System Face"
        );
        let config = Config::load(paths.config_file()).expect("reload system font selection");
        assert_eq!(
            config.get_in(Some("General"), "FontName"),
            Some("Mock System Face")
        );
        assert!(!Arc::ptr_eq(
            app.assets.clonk_fonts.as_ref().unwrap(),
            &prior_gui
        ));
        assert!(!Arc::ptr_eq(
            app.assets.book_fonts.as_ref().unwrap(),
            &prior_book
        ));
        assert!(!Arc::ptr_eq(
            app.assets.options_book_fonts.as_ref().unwrap(),
            &prior_options
        ));
        assert!(!Arc::ptr_eq(
            app.assets.plrsel_book_fonts.as_ref().unwrap(),
            &prior_player
        ));
        let native_source = app
            .assets
            .startup_native_font_source
            .as_ref()
            .expect("system face retains scale-native source");
        assert_eq!(native_source.bytes.as_ref(), font_bytes.as_ref());
        assert_eq!(native_source.face_index, 0);
        assert_eq!(
            app.native_startup_fonts
                .as_ref()
                .expect("system face builds at application scale")
                .scale(),
            2.0
        );

        let selected_gui = app.assets.clonk_fonts.clone().unwrap();
        let before_failure = fs::read(paths.config_file()).expect("config before missing face");
        app.apply_options_font_selection_with_system_fonts(
            Some("Definitely Missing Font".to_string()),
            None,
            &provider,
        )
        .expect("report unavailable system face");
        assert_eq!(app.message_dialogs.len(), dialog_count + 1);
        assert_eq!(
            app.startup_options_dialog
                .as_ref()
                .unwrap()
                .program()
                .font_face,
            "Mock System Face"
        );
        assert!(Arc::ptr_eq(
            app.assets.clonk_fonts.as_ref().unwrap(),
            &selected_gui
        ));
        assert_eq!(fs::read(paths.config_file()).unwrap(), before_failure);
    }

    #[test]
    fn l072_options_scale_enter_submit_times_out_reverts_and_yes_commits() {
        use clonk_frontend::message_dialog::MessageDialogResult;
        use clonk_frontend::startup_options_dlg::OptionsDlgAction;
        use clonk_frontend::startup_options_graphics::GraphicsSheetAction;

        let mut app = new_classic_menu_app(640, 480);
        app.open_options_menu();
        app.process_options_dialog_actions(vec![OptionsDlgAction::OpenGraphicsScaleText])
            .expect("open scale spinbox editor");
        app.game_option_input_dialog
            .as_mut()
            .expect("scale spinbox editor")
            .controller
            .set_input_text("225");
        app.handle_key(VirtualKeyCode::Return, ElementState::Pressed)
            .expect("submit scale spinbox editor with Enter");
        assert!(app.game_option_input_dialog.is_none());
        assert_eq!(
            app.startup_options_dialog
                .as_ref()
                .unwrap()
                .graphics()
                .proposed_scale_percent,
            225
        );
        assert_eq!(
            app.pending_options_display_requests.pop_front(),
            Some(OptionsDisplayRequest::SetScale {
                percent: 225,
                persist: false,
            })
        );
        assert!(app
            .message_dialogs
            .last()
            .is_some_and(|dialog| dialog.state.message().contains("12 seconds")));
        assert!(matches!(
            app.message_dialogs
                .last()
                .map(|dialog| &dialog.continuation),
            Some(MessageDialogContinuation::OptionsScaleTest {
                old_percent: 100,
                new_percent: 225,
                remaining_seconds: 12,
            })
        ));
        app.handle_key(VirtualKeyCode::Return, ElementState::Released)
            .expect("release scale editor Enter");
        assert_eq!(app.message_dialogs.len(), 1);
        for _ in 0..12 {
            app.sec1_timer().expect("advance scale countdown");
        }
        assert!(app.message_dialogs.is_empty());
        assert_eq!(
            app.pending_options_display_requests.pop_front(),
            Some(OptionsDisplayRequest::SetScale {
                percent: 100,
                persist: false,
            })
        );
        assert_eq!(
            app.startup_options_dialog
                .as_ref()
                .unwrap()
                .graphics()
                .proposed_scale_percent,
            100
        );

        app.startup_options_dialog
            .as_mut()
            .unwrap()
            .graphics_mut()
            .set_proposed_scale_percent(175);
        app.process_options_dialog_actions(vec![OptionsDlgAction::Graphics(
            GraphicsSheetAction::TestScale {
                old_percent: 100,
                new_percent: 175,
            },
        )])
        .expect("begin accepted scale test");
        assert!(app.pending_options_display_requests.pop_front().is_some());
        app.finish_message_dialog(MessageDialogResult::Yes)
            .expect("accept scale");
        assert_eq!(
            app.pending_options_display_requests.pop_front(),
            Some(OptionsDisplayRequest::SetScale {
                percent: 175,
                persist: true,
            })
        );
        assert_eq!(
            app.startup_options_dialog
                .as_ref()
                .unwrap()
                .graphics()
                .applied_scale_percent,
            175
        );

        app.startup_options_dialog
            .as_mut()
            .unwrap()
            .graphics_mut()
            .set_proposed_scale_percent(200);
        app.process_options_dialog_actions(vec![OptionsDlgAction::Graphics(
            GraphicsSheetAction::TestScale {
                old_percent: 175,
                new_percent: 200,
            },
        )])
        .expect("begin rejected scale test");
        assert!(app.pending_options_display_requests.pop_front().is_some());
        app.finish_message_dialog(MessageDialogResult::No)
            .expect("reject scale");
        assert_eq!(
            app.pending_options_display_requests.pop_front(),
            Some(OptionsDisplayRequest::SetScale {
                percent: 175,
                persist: false,
            })
        );
    }

    fn set_test_scenario_value_gain(app: &mut GameApp, value_gain: i32) {
        let mut state = app.engine.capture_state();
        let values = state
            .scenario_values
            .as_mut()
            .expect("captured state retains Game.C4S values");
        let mut encoded = serde_json::to_value(&*values).expect("serialize Game.C4S values");
        let game = encoded
            .get_mut("sections")
            .and_then(serde_json::Value::as_array_mut)
            .and_then(|sections| {
                sections.iter_mut().find(|section| {
                    section.get("name").and_then(serde_json::Value::as_str) == Some("Game")
                })
            })
            .expect("Game.C4S contains Game");
        let entry = game
            .get_mut("entries")
            .and_then(serde_json::Value::as_array_mut)
            .and_then(|entries| {
                entries.iter_mut().find(|entry| {
                    entry.get("name").and_then(serde_json::Value::as_str) == Some("ValueGain")
                })
            })
            .expect("Game contains ValueGain");
        entry["values"] = serde_json::json!([{ "Int": value_gain }]);
        *values = serde_json::from_value(encoded).expect("deserialize adjusted Game.C4S values");
        app.engine
            .restore_state(&state)
            .expect("restore adjusted Game.C4S values");
        app.snapshot = app.engine.snapshot();
        assert_eq!(app.engine.scenario_value_gain_enabled(), value_gain != 0);
    }

    #[test]
    fn l120_msgboard_command_reaches_continuous_multiline_render() {
        let mut app = new_classic_running_sandbox_app();
        app.clear_message_board_log();
        app.process_running_chat_text("/msgboard 3");

        let width = app.graphics.surface().width() as usize;
        let height = app.graphics.surface().height() as usize;
        let line_height = app.graphics.message_board_line_height() as usize;
        let mut without_lines = vec![0_u8; width * height * 4];
        app.render(&mut without_lines)
            .expect("render empty continuous message board");

        for line in ["Alpha", "Bravo", "Charlie"] {
            app.enqueue_control_message_board_line(line.to_string());
        }
        let mut with_lines = vec![0_u8; width * height * 4];
        app.render(&mut with_lines)
            .expect("render /msgboard continuous lines");

        let output_y = height - 4 * line_height;
        let band_changed = |top: usize| {
            (top..top + line_height).any(|y| {
                (0..width).any(|x| {
                    let pixel = (y * width + x) * 4;
                    without_lines[pixel..pixel + 4] != with_lines[pixel..pixel + 4]
                })
            })
        };
        assert!(band_changed(output_y));
        assert!(
            band_changed(output_y + line_height),
            "/msgboard 3 must render more than one simultaneous message line"
        );
    }

    #[test]
    fn physical_mouse_click_targets_assigned_secondary_viewport_when_hovering_primary() {
        // C++ stores one player in C4MouseControl, resolves that player's
        // first viewport for every move, clamps the physical point into its
        // output rectangle, and emits MoveTo with the stored player number
        // (C4GraphicsSystem.cpp:476-484; C4MouseControl.cpp:147-155,
        // 203-216,1148-1152,1216-1227).
        let mut app = new_running_sandbox_app();
        let primary = app.local_owner;
        let secondary = primary + 1;
        let primary_crew = app
            .engine
            .crew_cursor(primary)
            .expect("sandbox primary cursor");
        let primary_crew_state = app
            .engine
            .object_snapshot(primary_crew)
            .expect("sandbox primary crew remains live");

        app.engine
            .register_player(PlayerConfig::new(secondary, "Secondary"))
            .expect("register secondary runtime player");
        let secondary_position = Vector2::new(
            primary_crew_state.position.x.saturating_add(24),
            primary_crew_state.position.y,
        );
        let secondary_crew = app
            .engine
            .spawn_object(
                SpawnConfig::new(primary_crew_state.definition_id)
                    .with_position(secondary_position)
                    .with_owner(secondary)
                    .with_crew_member(true),
            )
            .expect("spawn secondary crew");
        app.engine
            .select_crew(secondary, [secondary_crew])
            .expect("select secondary crew");
        app.engine
            .set_crew_cursor(secondary, Some(secondary_crew))
            .expect("set secondary cursor");
        app.engine
            .replace_player_viewports(
                secondary,
                vec![clonk_engine::PlayerViewport::new(secondary_position)
                    .with_focus(Some(secondary_crew))],
            )
            .expect("set secondary viewport");
        app.engine.set_local_players([primary, secondary]);

        app.local_controls = LocalControlRegistry::default();
        let primary_control = app.local_controls.initialize(LocalControlInit {
            owner: primary,
            preferred_set: 0,
            prefers_mouse: false,
            gamepads_enabled: true,
            replay: false,
            disable_mouse: false,
        });
        let secondary_control = app.local_controls.initialize(LocalControlInit {
            owner: secondary,
            preferred_set: 1,
            prefers_mouse: true,
            gamepads_enabled: true,
            replay: false,
            disable_mouse: false,
        });
        assert!(!primary_control.mouse);
        assert!(secondary_control.mouse);
        assert_eq!(app.local_controls.mouse_owner(), Some(secondary));

        app.snapshot = app.engine.snapshot();
        let mut frame = vec![0_u8; 320 * 200 * 4];
        app.render(&mut frame)
            .expect("establish both local viewports");
        let primary_viewport = app
            .graphics
            .viewport_rect(primary)
            .expect("primary viewport");
        let secondary_viewport = app
            .graphics
            .viewport_rect(secondary)
            .expect("secondary viewport");
        assert_ne!(primary_viewport, secondary_viewport);

        let (physical_point, _) = (primary_viewport.y
            ..primary_viewport.y + primary_viewport.height as i32)
            .flat_map(|y| {
                (primary_viewport.x..primary_viewport.x + primary_viewport.width as i32)
                    .map(move |x| GuiPoint::new(x as f32 + 0.5, y as f32 + 0.5))
            })
            .find_map(|point| {
                let hovered = app.graphics.viewport_output_point_at(point)?;
                let projected = app
                    .graphics
                    .viewport_output_point_for_owner(secondary, point)?;
                (hovered.owner == primary
                    && projected.owner == secondary
                    && projected.screen != point
                    && app
                        .graphics
                        .crew_at_point(&app.snapshot, secondary, projected.screen)
                        .is_none())
                .then_some((point, projected))
            })
            .expect("primary viewport has a point clear of secondary crew after clamping");
        let expected_pointer = app
            .graphics
            .viewport_output_point_for_owner(
                secondary,
                GuiPoint::new(physical_point.x.ceil(), physical_point.y.ceil()),
            )
            .expect("C++ ceil-quantized point projects through the assigned viewport");
        assert!(
            physical_point.x < secondary_viewport.x as f32
                || physical_point.x >= (secondary_viewport.x + secondary_viewport.width as i32) as f32
                || physical_point.y < secondary_viewport.y as f32
                || physical_point.y >= (secondary_viewport.y + secondary_viewport.height as i32) as f32,
            "the physical point lies outside the mouse owner's viewport"
        );
        let primary_commands = app
            .engine
            .object_snapshot(primary_crew)
            .expect("primary crew before click")
            .command_stack
            .command_views();

        app.handle_cursor_moved(PhysicalPosition::new(
            f64::from(physical_point.x),
            f64::from(physical_point.y),
        ))
        .expect("physical move over primary viewport");
        assert_eq!(
            app.ingame_pointer,
            Some(expected_pointer),
            "C4MouseControl projects through its assigned player's viewport"
        );
        app.handle_mouse_button(ElementState::Pressed)
            .expect("physical left-down");
        app.handle_mouse_button(ElementState::Released)
            .expect("physical left-up");

        let secondary_commands = app
            .engine
            .object_snapshot(secondary_crew)
            .expect("secondary crew after click")
            .command_stack
            .command_views();
        assert_eq!(secondary_commands.len(), 1);
        assert_eq!(secondary_commands[0].name, "MoveTo");
        assert_eq!(secondary_commands[0].target, None);
        assert_eq!(
            secondary_commands[0].tx,
            Some(expected_pointer.world.x as i32)
        );
        assert_eq!(
            secondary_commands[0].ty,
            Some(expected_pointer.world.y as i32)
        );
        assert_eq!(
            app.engine
                .object_snapshot(primary_crew)
                .expect("primary crew after click")
                .command_stack
                .command_views(),
            primary_commands,
            "the physically hovered primary player must receive no command"
        );
    }

    #[test]
    fn mouse_viewport_edge_pan_repeats_until_an_interior_move() {
        // UpdateScrolling applies one ten-pixel step during the physical Move,
        // and MouseControl::Execute repeats it after every successful game
        // frame while Scrolling remains set. An interior move clears only the
        // repeat state: C4PVM_Scrolling stays frozen until an ordinary player
        // command calls ResetCursorView (C4MouseControl.cpp:133-145,664-692;
        // C4Player.cpp:926-928,1491-1521,1692-1715).
        let mut app = new_running_sandbox_app();
        let owner = app.local_owner;
        app.graphics.set_scroll_smooth(1);
        let focus = app.engine.crew_cursor(owner).expect("sandbox cursor");
        app.engine
            .replace_player_viewports(
                owner,
                vec![clonk_engine::PlayerViewport::new(Vector2::new(800, 180)).with_focus(Some(focus))],
            )
            .expect("place camera away from every scroll bound");
        app.snapshot = app.engine.snapshot();
        let mut frame = vec![0_u8; 320 * 200 * 4];
        app.render(&mut frame).expect("establish mouse viewport");
        let rect = app
            .graphics
            .viewport_rect(owner)
            .expect("mouse owner viewport");
        let left = GuiPoint::new(rect.x as f32, (rect.y + rect.height as i32 / 2) as f32);
        assert!(app.ingame_viewport_region(owner, left).is_none());
        assert!(app
            .script_menu_pointer_target(left)
            .expect("left edge target query")
            .is_none());
        let view_state = |app: &GameApp| {
            let snapshot = app.engine.snapshot();
            let player = snapshot
                .players
                .iter()
                .find(|player| player.id == owner)
                .expect("mouse owner remains");
            (player.viewports[0].center, player.view_mode)
        };
        let (before, _) = view_state(&app);

        app.handle_cursor_moved(PhysicalPosition::new(f64::from(left.x), f64::from(left.y)))
            .expect("move onto left viewport edge");
        let left_edge = app
            .ingame_edge_scroll
            .expect("left edge retains continuous scrolling")
            .edge;
        assert_eq!(left_edge.delta, Vector2::new(-10, 0));
        assert_eq!(left_edge.cursor, clonk_frontend::MouseCursorPhase::Left);
        assert_eq!(
            view_state(&app),
            (
                Vector2::new(before.x - 10, before.y),
                clonk_engine::PLAYER_VIEW_MODE_SCROLLING,
            )
        );
        app.render(&mut frame)
            .expect("render the scrolling-mode camera target");
        let projection = app
            .graphics
            .active_viewport_projections()
            .into_iter()
            .find(|projection| projection.owner == owner)
            .expect("mouse owner projection");
        let scrolled_center = view_state(&app).0;
        assert_eq!(
            (projection.target_x, projection.target_y),
            (
                scrolled_center.x - projection.logical_width / 2,
                scrolled_center.y - projection.logical_height / 2,
            ),
            "C4PVM_Scrolling removes the normal camera dead zone on both axes"
        );

        app.update().expect("first continuous edge-scroll tick");
        app.update().expect("second continuous edge-scroll tick");
        assert_eq!(
            view_state(&app).0,
            Vector2::new(before.x - 30, before.y),
            "no extra OS motion event is required for each ten-pixel step"
        );

        let interior = GuiPoint::new(
            (rect.x + rect.width as i32 / 2) as f32,
            (rect.y + rect.height as i32 / 2) as f32,
        );
        app.handle_cursor_moved(PhysicalPosition::new(
            f64::from(interior.x),
            f64::from(interior.y),
        ))
        .expect("leave viewport edge");
        assert!(app.ingame_edge_scroll.is_none());
        let stopped = view_state(&app).0;
        for _ in 0..6 {
            app.update().expect("interior Tick5 refresh window");
        }
        assert_eq!(view_state(&app).0, stopped);
        assert_eq!(
            view_state(&app).1,
            clonk_engine::PLAYER_VIEW_MODE_SCROLLING,
            "leaving the border stops pan but does not itself restore cursor mode"
        );

        app.engine
            .player_in_com(owner, clonk_engine::COM_RIGHT, 0)
            .expect("ordinary player input resets the camera mode");
        assert_eq!(view_state(&app).1, clonk_engine::PLAYER_VIEW_MODE_CURSOR);
    }

    #[test]
    fn continuous_edge_execute_reprojects_world_pointer_before_scrolling_again() {
        let mut app = new_running_sandbox_app();
        let owner = app.local_owner;
        let focus = app.engine.crew_cursor(owner).expect("sandbox cursor");
        app.engine
            .replace_player_viewports(
                owner,
                vec![clonk_engine::PlayerViewport::new(Vector2::new(800, 180)).with_focus(Some(focus))],
            )
            .expect("place camera away from every scroll bound");
        app.snapshot = app.engine.snapshot();
        app.display_flags.show_commands = false;
        app.graphics.set_scroll_smooth(1);
        let mut frame = vec![0_u8; 320 * 200 * 4];
        app.render(&mut frame).expect("establish mouse viewport");
        let rect = app.graphics.viewport_rect(owner).expect("owner viewport");
        let right = GuiPoint::new(
            (rect.x + rect.width as i32 - 1) as f32,
            (rect.y + rect.height as i32 / 2) as f32,
        );

        app.handle_cursor_moved(PhysicalPosition::new(
            f64::from(right.x),
            f64::from(right.y),
        ))
        .expect("arm continuous right-edge scrolling");
        let stale = app.ingame_pointer.expect("physical move projects pointer");
        let after_move = app.engine.player(owner).unwrap().viewports()[0].center;

        app.render(&mut frame)
            .expect("render the camera position from the first scroll step");
        let scroll = app
            .ingame_edge_scroll
            .expect("right edge remains armed after render");
        let expected = app
            .graphics
            .viewport_output_point_for_index(scroll.viewport_index, scroll.screen)
            .expect("retained viewport still projects the edge point");
        assert_ne!(
            expected.world, stale.world,
            "the rendered camera movement must change the fixed screen point's world coordinate"
        );
        assert_eq!(
            app.ingame_pointer,
            Some(stale),
            "rendering alone does not synthesize C4MouseControl::Move"
        );

        app.update()
            .expect("continuous Execute reprojects before its next scroll step");

        assert_eq!(app.ingame_pointer, Some(expected));
        assert_eq!(
            app.engine.player(owner).unwrap().viewports()[0].center,
            Vector2::new(after_move.x + 10, after_move.y)
        );
    }

    #[test]
    fn gui_consumed_pointer_move_clears_edge_pan_and_prevents_later_ticks() {
        let mut app = new_running_sandbox_app();
        let owner = app.local_owner;
        let focus = app.engine.crew_cursor(owner).expect("sandbox cursor");
        app.engine
            .replace_player_viewports(
                owner,
                vec![clonk_engine::PlayerViewport::new(Vector2::new(800, 180)).with_focus(Some(focus))],
            )
            .expect("place camera away from every scroll bound");
        app.snapshot = app.engine.snapshot();
        let mut frame = vec![0_u8; 320 * 200 * 4];
        app.render(&mut frame).expect("establish mouse viewport");
        let rect = app.graphics.viewport_rect(owner).expect("owner viewport");
        let left = PhysicalPosition::new(
            f64::from(rect.x),
            f64::from(rect.y + rect.height as i32 / 2),
        );

        app.handle_cursor_moved(left)
            .expect("arm continuous left-edge scrolling");
        assert!(app.ingame_edge_scroll.is_some());
        app.open_context_menu_at(
            vec![ContextMenuEntry::<AppContextMenuCommand>::new(
                "Remain open",
            )],
            GuiPoint::new(20.0, 20.0),
        )
        .expect("open retained running context menu");
        assert!(
            app.ingame_edge_scroll.is_some(),
            "opening the popup alone does not synthesize a pointer move"
        );
        let row = app
            .context_menu
            .as_ref()
            .expect("running context menu")
            .layout()
            .panels[0]
            .rows[0]
            .rect;
        let stopped = app.engine.player(owner).unwrap().viewports()[0].center;

        app.handle_cursor_moved(PhysicalPosition::new(
            f64::from(row.x + 1),
            f64::from(row.y + 1),
        ))
        .expect("route pointer movement into the context menu");
        assert!(app.context_menu.is_some());
        assert!(app.ingame_pointer.is_none());
        assert!(app.ingame_edge_scroll.is_none());

        for _ in 0..6 {
            app.update().expect("running context-menu simulation tick");
        }
        assert_eq!(
            app.engine.player(owner).unwrap().viewports()[0].center,
            stopped,
            "neither continuous Execute nor Tick5 may revive a GUI-consumed edge move"
        );
        assert!(app.ingame_edge_scroll.is_none());
    }

    #[test]
    fn continuous_execute_rechecks_retained_viewport_x_after_resize_without_reclamping() {
        let mut app = new_running_sandbox_app();
        let owner = app.local_owner;
        let focus = app.engine.crew_cursor(owner).expect("sandbox cursor");
        app.engine
            .replace_player_viewports(
                owner,
                vec![clonk_engine::PlayerViewport::new(Vector2::new(800, 180)).with_focus(Some(focus))],
            )
            .expect("place camera away from every scroll bound");
        app.snapshot = app.engine.snapshot();
        let mut frame = vec![0_u8; 320 * 200 * 4];
        app.display_flags.show_commands = false;
        app.render(&mut frame).expect("establish original viewport");
        let original = app.graphics.viewport_rect(owner).expect("owner viewport");
        let right = PhysicalPosition::new(
            f64::from(original.x + original.width as i32 - 1),
            f64::from(original.y + original.height as i32 / 2),
        );

        app.handle_cursor_moved(right)
            .expect("move onto the original right edge");
        assert_eq!(
            app.ingame_viewport_mouse
                .expect("C4MouseControl VpX/VpY retained")
                .position
                .x,
            original.width as i32 - 1
        );
        assert_eq!(
            app.ingame_edge_scroll
                .expect("original right edge remains armed")
                .edge
                .delta,
            Vector2::new(10, 0)
        );
        let stopped = app.engine.player(owner).unwrap().viewports()[0].center;

        app.resize(480, 200).expect("widen running viewport");
        let mut wider_frame = vec![0_u8; 480 * 200 * 4];
        app.render(&mut wider_frame)
            .expect("establish widened viewport layout");
        let wider = app.graphics.viewport_rect(owner).expect("wider viewport");
        assert!(wider.width > original.width);
        assert_eq!(
            app.ingame_viewport_mouse
                .expect("resize retains native VpX/VpY")
                .position
                .x,
            original.width as i32 - 1
        );
        assert!(
            original.width as i32 - 1 < wider.width as i32 - 1,
            "the retained right edge is now an interior viewport coordinate"
        );
        assert!(
            app.ingame_edge_scroll.is_some(),
            "native Scrolling stays armed until the next Execute reevaluates VpX"
        );

        app.update()
            .expect("next continuous Execute reevaluates resized VpX");
        assert_eq!(
            app.engine.player(owner).unwrap().viewports()[0].center,
            stopped,
            "Execute must test retained VpX against the new width, not clamp it back to the edge"
        );
        assert!(app.ingame_edge_scroll.is_none());
    }

    #[test]
    fn height_only_resize_retains_right_edge_continuous_pan() {
        let mut app = new_running_sandbox_app();
        let owner = app.local_owner;
        let focus = app.engine.crew_cursor(owner).expect("sandbox cursor");
        app.engine
            .replace_player_viewports(
                owner,
                vec![clonk_engine::PlayerViewport::new(Vector2::new(800, 180)).with_focus(Some(focus))],
            )
            .expect("place camera away from every scroll bound");
        app.snapshot = app.engine.snapshot();
        app.display_flags.show_commands = false;
        let mut frame = vec![0_u8; 320 * 200 * 4];
        app.render(&mut frame).expect("establish original viewport");
        let original = app.graphics.viewport_rect(owner).expect("owner viewport");
        let right = GuiPoint::new(
            (original.x + original.width as i32 - 1) as f32,
            (original.y + original.height as i32 / 2) as f32,
        );

        app.handle_cursor_moved(PhysicalPosition::new(
            f64::from(right.x),
            f64::from(right.y),
        ))
        .expect("arm continuous right-edge scrolling");
        let after_move = app.engine.player(owner).unwrap().viewports()[0].center;

        app.resize(320, 240).expect("grow only the running height");
        let mut taller_frame = vec![0_u8; 320 * 240 * 4];
        app.render(&mut taller_frame)
            .expect("establish height-only resized layout");
        let taller = app.graphics.viewport_rect(owner).expect("taller viewport");
        assert_eq!(taller.width, original.width);
        assert!(taller.height > original.height);
        assert_eq!(
            app.ingame_viewport_mouse
                .expect("resize retains native VpX/VpY")
                .position
                .x,
            taller.width as i32 - 1
        );

        app.update()
            .expect("continuous Execute keeps the retained right edge live");

        assert_eq!(
            app.engine.player(owner).unwrap().viewports()[0].center,
            Vector2::new(after_move.x + 10, after_move.y)
        );
        let scroll = app
            .ingame_edge_scroll
            .expect("right edge remains armed after the height-only resize");
        assert_eq!(scroll.edge.delta, Vector2::new(10, 0));
        assert_eq!(scroll.edge.cursor, clonk_frontend::MouseCursorPhase::Right);
    }

    #[test]
    fn tick5_starts_edge_pan_after_suppressing_viewport_region_disappears() {
        let mut app = new_running_sandbox_app();
        while app.engine.frame() % 5 != 4 {
            app.update()
                .expect("align the next simulation frame to Tick5");
        }
        let owner = app.local_owner;
        let focus = app.engine.crew_cursor(owner).expect("sandbox cursor");
        let focus_position = app
            .engine
            .object_snapshot(focus)
            .expect("sandbox cursor remains live")
            .position;
        app.engine
            .register_script_definition("MREG", "Mouse region fixture", "#strict\n")
            .expect("register region fixture");
        let container = app
            .engine
            .spawn_object(SpawnConfig::new("MREG").with_position(focus_position))
            .expect("spawn cursor container");
        app.engine
            .apply_object_update(focus, ObjectUpdate::new().with_container(container))
            .expect("put cursor into fixture to expose the Exit command region");
        app.engine
            .replace_player_viewports(
                owner,
                vec![clonk_engine::PlayerViewport::new(Vector2::new(800, 180)).with_focus(Some(focus))],
            )
            .expect("place camera away from every scroll bound");
        app.snapshot = app.engine.snapshot();
        let mut frame = vec![0_u8; 320 * 200 * 4];
        app.render(&mut frame)
            .expect("establish command region and viewport");
        let rect = app.graphics.viewport_rect(owner).expect("owner viewport");
        let left = GuiPoint::new(rect.x as f32, (rect.y + rect.height as i32 / 2) as f32);
        let corner = GuiPoint::new(
            (rect.x + rect.width as i32 - 1) as f32,
            (rect.y + rect.height as i32 - 1) as f32,
        );
        assert!(
            matches!(
                app.ingame_viewport_region(owner, corner),
                Some(IngameViewportRegion::Command(_))
            ),
            "the contained cursor's Exit pair covers the bottom-right edge"
        );

        app.handle_cursor_moved(PhysicalPosition::new(f64::from(left.x), f64::from(left.y)))
            .expect("enter scrolling mode before the suppressing region move");
        assert!(app.ingame_edge_scroll.is_some());
        app.handle_cursor_moved(PhysicalPosition::new(
            f64::from(corner.x),
            f64::from(corner.y),
        ))
        .expect("move retained VpX/VpY onto the command region");
        assert!(app.ingame_edge_scroll.is_none());
        let before_tick5 = app.engine.player(owner).unwrap().viewports()[0].center;

        app.display_flags.show_commands = false;
        assert!(app.ingame_viewport_region(owner, corner).is_none());
        app.update()
            .expect("Tick5 reevaluates the retained corner after region removal");
        assert_eq!(app.engine.frame() % 5, 0);
        assert_eq!(
            app.engine.player(owner).unwrap().viewports()[0].center,
            Vector2::new(before_tick5.x + 10, before_tick5.y + 10)
        );
        let resumed = app
            .ingame_edge_scroll
            .expect("disappearing region exposes the retained corner");
        assert_eq!(resumed.edge.delta, Vector2::new(10, 10));
        assert_eq!(
            resumed.edge.cursor,
            clonk_frontend::MouseCursorPhase::DownRight
        );
    }

    #[test]
    fn mouse_viewport_corner_pans_both_axes_and_uses_diagonal_cursor() {
        let mut app = new_running_sandbox_app();
        let owner = app.local_owner;
        let focus = app.engine.crew_cursor(owner).expect("sandbox cursor");
        app.engine
            .replace_player_viewports(
                owner,
                vec![clonk_engine::PlayerViewport::new(Vector2::new(800, 180)).with_focus(Some(focus))],
            )
            .expect("place camera away from every scroll bound");
        app.snapshot = app.engine.snapshot();
        let mut frame = vec![0_u8; 320 * 200 * 4];
        app.render(&mut frame).expect("establish mouse viewport");
        let rect = app.graphics.viewport_rect(owner).expect("owner viewport");
        let corner = GuiPoint::new(rect.x as f32, rect.y as f32);
        assert!(app.ingame_viewport_region(owner, corner).is_none());
        assert!(app
            .script_menu_pointer_target(corner)
            .expect("corner target query")
            .is_none());
        let before = app.engine.player(owner).unwrap().viewports()[0].center;

        app.handle_cursor_moved(PhysicalPosition::new(
            f64::from(corner.x),
            f64::from(corner.y),
        ))
        .expect("move onto upper-left corner");

        assert_eq!(
            app.engine.player(owner).unwrap().viewports()[0].center,
            Vector2::new(before.x - 10, before.y - 10)
        );
        let corner_edge = app
            .ingame_edge_scroll
            .expect("corner retains edge state")
            .edge;
        assert_eq!(corner_edge.delta, Vector2::new(-10, -10));
        assert_eq!(corner_edge.cursor, clonk_frontend::MouseCursorPhase::UpLeft);
    }

    #[test]
    fn fullscreen_mouse_edge_pan_uses_the_forty_pixel_overflow_bound() {
        let mut app = new_running_sandbox_app();
        let owner = app.local_owner;
        let focus = app.engine.crew_cursor(owner).expect("sandbox cursor");
        let mut frame = vec![0_u8; 320 * 200 * 4];
        app.render(&mut frame).expect("establish mouse viewport");
        let projection = app
            .graphics
            .active_viewport_projections()
            .into_iter()
            .find(|viewport| viewport.owner == owner)
            .expect("owner projection");
        let minimum_x = projection.logical_width / 2 - 40;
        let y = 180;
        app.engine
            .set_player_viewport(
                owner,
                0,
                clonk_engine::PlayerViewport::new(Vector2::new(minimum_x + 5, y))
                    .with_focus(Some(focus)),
            )
            .expect("place camera just inside fullscreen overflow bound");
        app.snapshot = app.engine.snapshot();
        app.render(&mut frame).expect("render positioned camera");
        let rect = app.graphics.viewport_rect(owner).expect("owner viewport");
        let left = GuiPoint::new(rect.x as f32, (rect.y + rect.height as i32 / 2) as f32);

        app.handle_cursor_moved(PhysicalPosition::new(f64::from(left.x), f64::from(left.y)))
            .expect("scroll into fullscreen overflow bound");

        assert_eq!(
            app.engine.player(owner).unwrap().viewports()[0].center,
            Vector2::new(minimum_x, y),
            "the remaining five pixels clamp instead of applying all ten"
        );
    }

    #[test]
    fn ownerless_viewport_edge_scrolls_passive_camera_without_player_mutation() {
        // IsPassive still runs UpdateScrolling, but ScrollView has no pPlayer
        // and writes C4Viewport::ViewX/Y directly. Build that exact active
        // viewport without changing the sandbox engine's player records
        // (C4MouseControl.cpp:244-257,1328-1345).
        let mut app = new_running_sandbox_app();
        let engine_viewports = app
            .engine
            .player(app.local_owner)
            .expect("sandbox player")
            .viewports()
            .to_vec();
        app.local_controls = LocalControlRegistry::default();
        let snapshot = app.snapshot.clone();
        let focus = snapshot.objects.first().expect("sandbox focus object");
        app.graphics.render_frame(
            &snapshot,
            &[ViewportInput::new(
                OWNER_NONE,
                Vector2::new(1024, 180),
                1.0,
                focus,
            )],
        );
        let before = app.graphics.active_viewport_projections()[0];
        assert_eq!(before.owner, OWNER_NONE);
        let left = GuiPoint::new(
            before.rect.x as f32,
            (before.rect.y + before.rect.height as i32 / 2) as f32,
        );

        app.handle_cursor_moved(PhysicalPosition::new(f64::from(left.x), f64::from(left.y)))
            .expect("move passive pointer onto left edge");

        let after_move = app.graphics.active_viewport_projections()[0];
        assert_eq!(after_move.content_origin_x, before.content_origin_x - 10.0);
        assert_eq!(after_move.content_origin_y, before.content_origin_y);
        assert_eq!(
            app.ingame_edge_scroll
                .expect("passive edge state remains live")
                .edge
                .cursor,
            clonk_frontend::MouseCursorPhase::Left
        );
        assert_eq!(
            app.engine
                .player(app.local_owner)
                .expect("sandbox player remains")
                .viewports(),
            engine_viewports.as_slice(),
            "ownerless ScrollView must not mutate an unrelated player"
        );

        app.update().expect("passive continuous edge-scroll tick");
        let after_tick = app.graphics.active_viewport_projections()[0];
        assert_eq!(after_tick.content_origin_x, before.content_origin_x - 20.0);
    }

    #[test]
    fn zero_object_observer_uses_anchor_free_ownerless_viewport() {
        let mut app = new_running_sandbox_app();
        app.snapshot.objects.clear();
        app.snapshot.hud.local_players.clear();
        app.local_controls = LocalControlRegistry::default();
        app.engine.set_local_players([]);
        app.refresh_non_authoritative_physical_viewports();
        app.focus_id = None;
        app.focus_snapshot = None;

        let inputs = collect_viewport_inputs(&app.snapshot)
            .expect("zero-object observer viewport is object-independent");
        assert_eq!(inputs.len(), 1);
        assert_eq!(inputs[0].owner, OWNER_NONE);
        assert!(inputs[0].focus.is_none());

        let mut frame = vec![0_u8; 320 * 200 * 4];
        app.render_running(&mut frame, false)
            .expect("zero-object observer renders without a synthetic focus anchor");
        assert!(app.graphics.active_viewport_projections()[0].is_no_owner_viewport);
    }

    #[test]
    fn focusless_scrolling_player_uses_anchor_free_owned_viewport() {
        let mut app = new_running_sandbox_app();
        let owner = app.local_owner;
        let player = app
            .snapshot
            .players
            .iter_mut()
            .find(|player| player.id == owner)
            .expect("sandbox player remains declared");
        player.view_mode = PLAYER_VIEW_MODE_SCROLLING;
        player.cursor = None;
        player.view_cursor = None;
        player.crew.clear();
        player.viewports[0].focus = None;
        app.snapshot.objects.clear();
        app.focus_id = None;
        app.focus_snapshot = None;

        let inputs = collect_viewport_inputs(&app.snapshot)
            .expect("scrolling camera center remains valid without a live object");
        assert_eq!(inputs.len(), 1);
        assert_eq!(inputs[0].owner, owner);
        assert!(inputs[0].focus.is_none());

        let mut frame = vec![0_u8; 320 * 200 * 4];
        app.render_running(&mut frame, false)
            .expect("focusless scrolling player viewport renders");
        let projection = app.graphics.active_viewport_projections()[0];
        assert_eq!(projection.owner, owner);
        assert!(!projection.is_no_owner_viewport);
    }

    #[test]
    fn l052_automatic_retirement_closes_viewport_and_releases_local_control() {
        // C4Player::Execute decrements RetireDelay for 60 frames, then
        // C4PlayerList::Retire takes the same viewport-close path as an
        // explicit CID_RemovePlr (C4Player.cpp:2015-2021, 930-970).
        let mut app = new_lightweight_running_sandbox_app();
        let player = app.local_owner;
        let secondary = player + 1;
        let primary_crew = app
            .engine
            .crew_cursor(player)
            .expect("sandbox primary cursor");
        let primary_crew_state = app
            .engine
            .object_snapshot(primary_crew)
            .expect("sandbox primary crew remains live");
        app.engine
            .register_player(PlayerConfig::new(secondary, "Retained player"))
            .expect("register active player that prevents game over");
        let secondary_crew = app
            .engine
            .spawn_object(
                SpawnConfig::new(primary_crew_state.definition_id)
                    .with_position(primary_crew_state.position)
                    .with_owner(secondary)
                    .with_crew_member(true),
            )
            .expect("spawn retained player's crew");
        app.engine
            .select_crew(secondary, [secondary_crew])
            .expect("select retained player's crew");
        app.engine
            .set_crew_cursor(secondary, Some(secondary_crew))
            .expect("set retained player's cursor");
        app.engine
            .replace_player_viewports(player, Vec::new())
            .expect("clear camera payload without closing the physical viewport");
        app.snapshot = app.engine.snapshot();
        app.ui_sound_log.clear();

        app.engine
            .set_player_surrendered(player, true)
            .expect("start native 60-frame retirement delay");
        for frame in 1..60 {
            app.update().expect("advance retirement delay");
            assert!(
                app.engine.player(player).is_some(),
                "player retired before frame {frame}"
            );
            assert!(app.ui_sound_log.is_empty());
        }
        app.update().expect("retire player on frame 60");

        assert!(app.engine.player(player).is_none());
        assert_eq!(app.local_controls.assignment(player), None);
        assert!(!app.snapshot.hud.local_players.contains(&player));
        assert_eq!(
            app.ui_sound_log
                .iter()
                .filter(|sound| sound.as_str() == "CloseViewport")
                .count(),
            1,
            "automatic retirement closes all matching viewports once"
        );
        let viewports = collect_viewport_inputs(&app.snapshot)
            .expect("retirement leaves the silent ownerless fallback");
        assert_eq!(viewports.len(), 1);
        assert_eq!(viewports[0].owner, OWNER_NONE);
    }

    #[test]
    fn construction_drag_keeps_hud_regions_blocking_the_world_site() {
        let (mut app, owner, menu_point, _valid, _invalid, _world, _c4id) = construction_drag_fixture();
        let cursor = app.engine.crew_cursor(owner).expect("sandbox cursor");
        let mut item = Definition::from_script("B33I", "HUD item", "#strict\n").expect("item compiles");
        item.set_category(clonk_engine::CATEGORY_OBJECT);
        item.set_collectible(true);
        app.engine
            .register_definition(item)
            .expect("item registers");
        let carried = app
            .engine
            .spawn_object(SpawnConfig::new("B33I").with_container(cursor))
            .expect("inventory item spawns");
        app.snapshot = app.engine.snapshot();
        let mut frame = vec![0_u8; 320 * 200 * 4];
        app.render(&mut frame).expect("render inventory HUD region");

        let viewport = app.graphics.viewport_rect(owner).expect("local viewport");
        let hud_point = GuiPoint::new(
            (viewport.x + clonk_frontend::hud::SYMBOL_BORDER + clonk_frontend::hud::SYMBOL_SIZE / 2)
                as f32,
            (viewport.y + viewport.height as i32
                - clonk_frontend::hud::SYMBOL_BORDER
                - clonk_frontend::hud::SYMBOL_SIZE / 2) as f32,
        );
        assert_eq!(
            app.ingame_viewport_region(owner, hud_point),
            Some(IngameViewportRegion::Inventory(carried))
        );
        let world = app
            .graphics
            .viewport_output_point_at(hud_point)
            .map(ingame_pointer_world_pixel)
            .expect("HUD output retains a world point");
        let mut landscape = Landscape::flat(480, world.y);
        landscape.set_world_height(world.y.saturating_add(40));
        app.engine.set_landscape(landscape);
        assert!(
            app.engine.construction_site_valid("BLD1", world),
            "terrain behind the HUD is otherwise a valid construction site"
        );

        begin_construction_drag(&mut app, menu_point, hud_point);
        assert!(matches!(
            app.construction_menu_drag.as_ref(),
            Some(ConstructionMenuDrag::Active {
                pointer: Some(_),
                site_valid: false,
                ..
            })
        ));
    }

    #[test]
    fn construction_drop_uses_cached_last_phase_without_release_recheck() {
        let (mut app, owner, menu_point, valid_point, _invalid, valid_world, raw_c4id) =
            construction_drag_fixture();
        let (manager, _events, mut network_commands) =
            NetworkManager::test_stub_with_commands_for_client_id(7);
        app.network = Some(manager);
        let tick = app.local_control_submission_tick();
        begin_construction_drag(&mut app, menu_point, valid_point);

        let mut filled = Landscape::flat(480, 0);
        filled.set_world_height(220);
        app.engine.set_landscape(filled);
        app.handle_mouse_button(ElementState::Released)
            .expect("release before the next per-frame phase refresh");

        let (controls, commands, selections) = network_commands.take_submitted_player_inputs();
        assert!(controls.is_empty());
        assert_eq!(
            commands,
            vec![(
                tick,
                PlayerCommandControlData {
                    player: owner,
                    command: CommandId::Construct as i32,
                    x: valid_world.x,
                    y: valid_world.y,
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
    fn l065_title_drag_is_captured_exactly_and_resize_resets_location() {
        let mut app = new_classic_running_sandbox_app();
        let owner = app.local_owner;
        let cursor = app.engine.crew_cursor(owner).expect("sandbox cursor");
        install_test_cursor_menu(&mut app, cursor, long_script_menu(cursor, 8));
        let mut frame = vec![0_u8; 320 * 200 * 4];
        app.render(&mut frame).expect("seed script presentation");
        let (_, geometry) = app
            .script_menu_geometry_for_owner(owner)
            .expect("script geometry resources")
            .expect("script geometry");
        let title = geometry.title.expect("script title");
        let start = GuiPoint::new((title.x + 2) as f32, (title.y + 5) as f32);
        app.handle_cursor_moved(PhysicalPosition::new(
            f64::from(start.x),
            f64::from(start.y),
        ))
        .expect("hover wooden title");
        assert_eq!(
            app.script_menu_pointer_target(start)
                .expect("title hit-test resources"),
            Some(EngineScriptMenuPointerTarget::Title)
        );
        app.handle_mouse_button(ElementState::Pressed)
            .expect("capture wooden title");
        assert!(matches!(
            app.menu_title_drag,
            Some(MenuTitleDrag::Script { .. })
        ));
        let destination = GuiPoint::new(start.x - 400.0, start.y + 17.0);
        app.handle_cursor_moved(PhysicalPosition::new(
            f64::from(destination.x),
            f64::from(destination.y),
        ))
        .expect("drag stays captured outside dialog and viewport");
        let expected = (
            geometry.bounds.x.saturating_sub(400),
            geometry.bounds.y.saturating_add(17),
        );
        assert_eq!(
            app.script_menu_presentations
                .get(&owner)
                .and_then(|state| state.location),
            Some(expected),
            "native title drag applies the exact pointer delta without a threshold or clamp"
        );
        app.handle_mouse_button(ElementState::Released)
            .expect("release retained title capture outside");
        assert!(app.menu_title_drag.is_none());
        let retained = app
            .script_menu_presentations
            .get(&owner)
            .and_then(|state| state.location);
        app.handle_cursor_moved(PhysicalPosition::new(10.0, 10.0))
            .expect("ordinary move after release");
        assert_eq!(
            app.script_menu_presentations
                .get(&owner)
                .and_then(|state| state.location),
            retained
        );
        app.resize(360, 220).expect("viewport resize");
        assert!(app.menu_title_drag.is_none());
        assert_eq!(
            app.script_menu_presentations
                .get(&owner)
                .and_then(|state| state.location),
            None,
            "viewport ResetLocation restores anchored placement"
        );

        let mut player_app = new_classic_running_sandbox_app();
        let player = player_app.local_owner;
        let players = (0..8)
            .map(|index| NewPlayerEntry {
                file: format!("Player{index}.c4p"),
                name: format!("Player {index}"),
            })
            .collect::<Vec<_>>();
        player_app
            .ingame_menu
            .replace(player, Some(IngameMenuState::new_player_menu(&players)));
        player_app
            .render(&mut frame)
            .expect("seed player-menu presentation");
        let area = player_app
            .ingame_menu_area(player)
            .expect("player viewport");
        let bounds = {
            let fallback = player_app.assets.font_arc();
            let font = clonk_frontend::hud::HudFont::from_set(
                player_app.assets.clonk_fonts.as_deref(),
                fallback.as_ref(),
            );
            let gfx = IngameMenuGraphics {
                show_commands: player_app.display_flags.show_commands,
                show_close_button: true,
                ..IngameMenuGraphics::default()
            };
            player_app
                .ingame_menu
                .get(player)
                .expect("player menu")
                .bounds(area, &font, &gfx)
        };
        let start = GuiPoint::new((bounds.x + 2) as f32, (bounds.y + 5) as f32);
        player_app
            .handle_cursor_moved(PhysicalPosition::new(
                f64::from(start.x),
                f64::from(start.y),
            ))
            .expect("hover player-menu title");
        player_app
            .handle_mouse_button(ElementState::Pressed)
            .expect("capture player-menu title");
        let destination = GuiPoint::new(start.x + 11.0, start.y - 9.0);
        player_app
            .handle_cursor_moved(PhysicalPosition::new(
                f64::from(destination.x),
                f64::from(destination.y),
            ))
            .expect("drag player menu");
        let moved_x = {
            let fallback = player_app.assets.font_arc();
            let font = clonk_frontend::hud::HudFont::from_set(
                player_app.assets.clonk_fonts.as_deref(),
                fallback.as_ref(),
            );
            let gfx = IngameMenuGraphics {
                show_commands: player_app.display_flags.show_commands,
                show_close_button: true,
                ..IngameMenuGraphics::default()
            };
            player_app
                .ingame_menu
                .get(player)
                .expect("player menu")
                .bounds(area, &font, &gfx)
                .x
        };
        assert_eq!(moved_x, bounds.x + 11);
        player_app
            .handle_mouse_button(ElementState::Released)
            .expect("release player-menu title");
        player_app
            .resize(360, 220)
            .expect("reset player menu location");
        assert!(player_app.menu_title_drag.is_none());
    }

    #[test]
    fn runtime_flash_renders_non_cp1252_utf8_through_font_regular() {
        let mut app = new_classic_running_sandbox_app();
        app.status_text.clear();
        app.snapshot.hud.messages.clear();
        let mut baseline = vec![0_u8; 320 * 200 * 4];
        app.render(&mut baseline).expect("render Unicode baseline");
        app.set_runtime_flash_message("\u{100}", RuntimeHelpCharset::Utf8)
            .expect("install UTF-8 FontRegular flash");
        let mut unicode = vec![0_u8; 320 * 200 * 4];
        app.render(&mut unicode).expect("render Unicode flash");
        assert_ne!(unicode, baseline);
        assert_eq!(
            app.runtime_flash_message
                .as_ref()
                .expect("three UTF-8 byte draws remain")
                .remaining_draws,
            3
        );
    }

    #[test]
    fn runtime_flash_counts_successful_draws_survives_resize_and_resets_with_game() {
        let mut app = new_running_sandbox_app();
        app.status_text.clear();
        app.snapshot.hud.messages.clear();
        app.set_runtime_flash_message("A", RuntimeHelpCharset::Windows1252)
            .expect("install two-pass flash");
        let before_update = app.runtime_flash_message.clone();
        app.update().expect("ordinary game update");
        assert_eq!(
            app.runtime_flash_message, before_update,
            "ticks do not age flash"
        );
        app.set_runtime_flash_message("", RuntimeHelpCharset::Windows1252)
            .expect("clear after tick probe");
        let mut baseline = vec![0_u8; 320 * 200 * 4];
        app.render(&mut baseline).expect("render flash baseline");
        app.set_runtime_flash_message("A", RuntimeHelpCharset::Windows1252)
            .expect("reinstall two-pass flash");

        let mut first = vec![0_u8; 320 * 200 * 4];
        app.render(&mut first).expect("first visible flash pass");
        assert_ne!(first, baseline);
        assert_eq!(
            app.runtime_flash_message
                .as_ref()
                .expect("one pass remains")
                .remaining_draws,
            1
        );
        let mut final_visible = vec![0_u8; 320 * 200 * 4];
        app.render(&mut final_visible)
            .expect("final visible flash pass");
        assert_eq!(final_visible, first);
        assert!(app.runtime_flash_message.is_none());
        let mut expired = vec![0_u8; 320 * 200 * 4];
        app.render(&mut expired).expect("post-expiry frame");
        assert_eq!(expired, baseline);

        app.set_runtime_flash_message("AB", RuntimeHelpCharset::Windows1252)
            .expect("install resize-persistent flash");
        let before_resize = app.runtime_flash_message.clone();
        app.resize(321, 200).expect("resize running presentation");
        assert_eq!(app.runtime_flash_message, before_resize);

        app.configure_running_state("Next game".to_string(), DEFAULT_GROUND_HEIGHT);
        assert!(app.runtime_flash_message.is_none());
        app.set_runtime_flash_message("AB", RuntimeHelpCharset::Windows1252)
            .expect("install before menu return");
        app.return_to_menu();
        assert!(app.runtime_flash_message.is_none());
    }

    #[test]
    fn runtime_help_and_flash_resolve_fontregular_images() {
        let mut app = new_classic_running_sandbox_app();
        let resolved =
            resolve_font_images_in_texts(&app.engine, ["{{CLNK}}"], app.script_text_spec_resources());
        assert!(
            resolved.font_image("CLNK").is_some(),
            "the live FontRegular provider resolves installed definitions"
        );
        app.status_text.clear();
        app.snapshot.hud.messages.clear();
        hold_message_board_for_frame_comparison(&mut app);
        app.runtime_help_text_cache = OnceLock::new();
        app.runtime_help_text_cache
            .set(Ok(RuntimeHelpColumns {
                left: "<i>{{CLNK}}</i>".to_string(),
                right: String::new(),
            }))
            .expect("install image-bearing help columns");
        app.runtime_help_visible = true;

        let mut help = vec![0_u8; 320 * 200 * 4];
        app.render(&mut help)
            .expect("italic FontRegular help image renders");
        app.runtime_help_visible = false;
        let mut baseline = vec![0_u8; 320 * 200 * 4];
        app.render(&mut baseline)
            .expect("render overlay-free baseline");
        assert_ne!(help, baseline, "resolved help image contributes pixels");

        app.set_runtime_flash_message("<i>{{CLNK}}</i>", RuntimeHelpCharset::Windows1252)
            .expect("valid italic/image flash installs");
        let before = app
            .runtime_flash_message
            .as_ref()
            .expect("flash state")
            .remaining_draws;
        let mut flash = vec![0_u8; 320 * 200 * 4];
        app.render(&mut flash)
            .expect("italic FontRegular flash image renders");
        assert_ne!(flash, baseline, "resolved flash image contributes pixels");
        assert_eq!(
            app.runtime_flash_message
                .as_ref()
                .expect("more image flash passes remain")
                .remaining_draws,
            before - 1,
        );
    }

    #[test]
    fn l031_debug_keys_toggle_render_flags_and_exact_flashes() {
        let names = RuntimeFlashProducerBoundary::ALL.map(|producer| match producer {
            RuntimeFlashProducerBoundary::ObserverPrompt => "ObserverPrompt",
            RuntimeFlashProducerBoundary::ObserverClear => "ObserverClear",
            RuntimeFlashProducerBoundary::RuntimeJoin => "RuntimeJoin",
            RuntimeFlashProducerBoundary::ControlRate => "ControlRate",
            RuntimeFlashProducerBoundary::FairCrew => "FairCrew",
        });
        assert_eq!(names.len(), 5);
        assert_eq!(names.into_iter().collect::<HashSet<_>>().len(), 5);

        let mut app = new_running_sandbox_app();
        app.handle_modifiers_changed(ModifiersState::CTRL)
            .expect("set exact debug modifiers");
        app.handle_key(VirtualKeyCode::F5, ElementState::Pressed)
            .expect("enable debug mode");
        assert!(app.engine.debug_mode());
        assert_eq!(runtime_flash_text(&app), Some("Debug mode: on"));
        app.bindings
            .rebind(ControlBindingId::Left, VirtualKeyCode::F5);
        app.engine
            .player_mut(app.local_owner)
            .expect("local player")
            .control
            .pressed_coms = 1 << clonk_engine::COM_LEFT;
        app.handle_key(VirtualKeyCode::F5, ElementState::Released)
            .expect("debug mode has no Up callback");
        assert_ne!(
            app.engine
                .player(app.local_owner)
                .expect("local player")
                .control
                .pressed_coms
                & (1 << clonk_engine::COM_LEFT),
            0,
            "debug callback Up must not leak into modifier-blind player control"
        );

        app.handle_key(VirtualKeyCode::F6, ElementState::Pressed)
            .expect("enable vertices and entrances");
        let flags = app.graphics.debug_draw_flags();
        assert!(flags.show_vertices && flags.show_entrance);
        assert_eq!(runtime_flash_text(&app), Some("Entrance+Vertices: on"));
        app.handle_key(VirtualKeyCode::F6, ElementState::Pressed)
            .expect("disable vertices and entrances");
        let flags = app.graphics.debug_draw_flags();
        assert!(!flags.show_vertices && !flags.show_entrance);
        assert_eq!(runtime_flash_text(&app), Some("Entrance+Vertices: off"));

        for (expected, action, command, pathfinder) in [
            ("Actions", true, false, false),
            ("Commands", false, true, false),
            ("Pathfinder", false, false, true),
            ("Actions/Commands/Pathfinder: off", false, false, false),
        ] {
            app.handle_key(VirtualKeyCode::F7, ElementState::Pressed)
                .expect("cycle the action overlay");
            let flags = app.graphics.debug_draw_flags();
            assert_eq!(
                (flags.show_action, flags.show_command, flags.show_pathfinder),
                (action, command, pathfinder)
            );
            assert_eq!(runtime_flash_text(&app), Some(expected));
        }

        app.handle_key(VirtualKeyCode::F8, ElementState::Pressed)
            .expect("enable solid-mask display");
        assert!(app.graphics.debug_draw_flags().show_solid_mask);
        assert_eq!(runtime_flash_text(&app), Some("SolidMasks: on"));
        app.handle_key(VirtualKeyCode::F8, ElementState::Pressed)
            .expect("disable solid-mask display");
        assert!(!app.graphics.debug_draw_flags().show_solid_mask);
        assert_eq!(runtime_flash_text(&app), Some("SolidMasks: off"));

        let mut flags = app.graphics.debug_draw_flags();
        flags.show_net_status = true;
        app.graphics.set_debug_draw_flags(flags);
        app.handle_key(VirtualKeyCode::F5, ElementState::Pressed)
            .expect("disable debug mode");
        assert!(!app.engine.debug_mode());
        assert_eq!(
            app.graphics.debug_draw_flags(),
            clonk_frontend::DebugDrawFlags::default()
        );
        assert_eq!(runtime_flash_text(&app), Some("Debug mode: off"));
        assert_eq!(app.mode, AppMode::Running);
        assert!(!app.exit_requested);
    }

    #[test]
    fn runtime_f1_language_parser_preserves_cpp_boundaries_and_font_safety() {
        let malformed = parse_runtime_help_language_table(
            b"junk\r\nIDS_CON_HELP=Help\r\nIDS_CTL_MUSIC=Mu\\nsic\r\nIDS_CTL_SOUND=a=b\r\n",
            "malformed fixture",
        )
        .expect("parse malformed fixture without recovering swallowed keys");
        assert!(!malformed.contains_key("IDS_CON_HELP"));
        assert_eq!(
            malformed.get("IDS_CTL_MUSIC").map(String::as_str),
            Some("Mu\r\nsic")
        );
        assert_eq!(
            malformed.get("IDS_CTL_SOUND").map(String::as_str),
            Some("a=b")
        );

        let cp1252 = parse_runtime_help_language_table(
            b"IDS_LANG_CHARSET=\r\nIDS_CON_HELP=\x80\r\n",
            "CP1252 fixture",
        )
        .expect("default classic charset is CP1252");
        assert_eq!(cp1252.get("IDS_CON_HELP").map(String::as_str), Some("€"));

        let raw = parse_runtime_language_bytes_table(
            b"IDS_LANG_CHARSET=RUSSIAN\r\nIDS_DESC_DATEREC=\xcf\xf0\xe8\xe2\xe5\xf2\\n%s\r\n",
            "raw CP1251 fixture",
        )
        .expect("save descriptions keep legacy code-page bytes opaque");
        assert_eq!(
            raw.entries.get("IDS_LANG_CHARSET").map(Vec::as_slice),
            Some(b"RUSSIAN".as_slice())
        );
        assert_eq!(
            raw.entries.get("IDS_DESC_DATEREC").map(Vec::as_slice),
            Some(b"\xcf\xf0\xe8\xe2\xe5\xf2\r\n%s".as_slice())
        );

        let utf8 = parse_runtime_help_language_table(
            "IDS_LANG_CHARSET=UTF-8\nIDS_CON_HELP=Hilfe ä\n".as_bytes(),
            "UTF-8 fixture",
        )
        .expect("table-owned UTF-8 charset");
        assert_eq!(
            utf8.get("IDS_CON_HELP").map(String::as_str),
            Some("Hilfe ä")
        );

        for supported in ["<i>Help</i>", "{{CLNK}}"] {
            let mut table = HashMap::new();
            table.insert("IDS_CON_HELP".to_string(), supported.to_string());
            let columns = build_runtime_help_columns(&table)
                .expect("valid FontRegular markup reaches the renderer");
            assert!(
                columns.left.contains(supported),
                "help column must preserve {supported:?}"
            );
        }
        let mut unicode = HashMap::new();
        unicode.insert("IDS_CON_HELP".to_string(), "Помощь".to_string());
        assert!(
            build_runtime_help_columns(&unicode).is_ok(),
            "UTF-8 FontRegular dynamically supports non-CP1252 scalars"
        );
        let mut oversized = HashMap::new();
        oversized.insert("IDS_CON_HELP".to_string(), "x".repeat(2501));
        let error = build_runtime_help_columns(&oversized)
            .expect_err("oversized C++ TextOut line must fail closed");
        assert!(error.to_string().contains("2500-byte TextOut buffer"));
    }

    #[test]
    fn graphics_smoke_level_loads_legacy_default_and_configured_value() {
        let _lock = env_lock().lock();
        let install = tempdir().expect("smoke-level install fixture");
        let user_data = tempdir().expect("smoke-level user fixture");
        fs::create_dir_all(install.path().join("planet/System.c4g"))
            .expect("fixture System.c4g directory");
        let _guard = EnvGuard::set(&[
            ("LC_INSTALL_ROOT", Some(install.path())),
            ("LC_USER_DATA_DIR", Some(user_data.path())),
        ]);
        let paths = AppPaths::discover().expect("discover smoke-level fixture");
        paths.ensure_user_dirs().expect("create fixture user dirs");

        assert_eq!(
            load_graphics_smoke_level(Some(&paths)),
            clonk_engine::DEFAULT_SMOKE_LEVEL
        );
        fs::write(paths.config_file(), "[Graphics]\nSmokeLevel=73\n")
            .expect("write custom smoke level");
        assert_eq!(load_graphics_smoke_level(Some(&paths)), 73);

        fs::write(paths.config_file(), "[Graphics]\nSmokeLevel=invalid\n")
            .expect("write invalid smoke level");
        assert_eq!(
            load_graphics_smoke_level(Some(&paths)),
            clonk_engine::DEFAULT_SMOKE_LEVEL
        );
    }

    #[test]
    fn liquid_animation_requires_both_legacy_graphics_switches() {
        let _lock = env_lock().lock();
        let install = tempdir().expect("liquid-animation config install fixture");
        let user_data = tempdir().expect("liquid-animation config user fixture");
        fs::create_dir_all(install.path().join("planet/System.c4g"))
            .expect("fixture System.c4g directory");
        let _guard = EnvGuard::set(&[
            ("LC_INSTALL_ROOT", Some(install.path())),
            ("LC_USER_DATA_DIR", Some(user_data.path())),
        ]);
        let paths = AppPaths::discover().expect("discover liquid-animation fixture");
        paths.ensure_user_dirs().expect("create fixture user dirs");

        assert!(!load_graphics_color_animation(Some(&paths)));
        for config in [
            "[Graphics]\nColorAnimation=1\n",
            "[Graphics]\nShader=1\n",
            "[Graphics]\nColorAnimation=1\nShader=invalid\n",
        ] {
            fs::write(paths.config_file(), config).expect("write disabled graphics matrix");
            assert!(!load_graphics_color_animation(Some(&paths)));
        }
        fs::write(
            paths.config_file(),
            "[Graphics]\nColorAnimation=1\nShader=1\n",
        )
        .expect("write enabled graphics matrix");
        assert!(load_graphics_color_animation(Some(&paths)));
    }

    #[test]
    fn runtime_f1_help_toggles_on_each_down_renders_and_release_falls_through() {
        for modifiers in [ModifiersState::empty(), ModifiersState::LOGO] {
            let mut app = new_classic_running_sandbox_app();
            app.status_text.clear();
            app.snapshot.hud.messages.clear();
            app.handle_modifiers_changed(modifiers)
                .expect("set keyboard modifiers");

            let mut before_pixels = vec![0_u8; 320 * 200 * 4];
            app.render(&mut before_pixels).expect("render before F1");
            app.handle_key(VirtualKeyCode::F1, ElementState::Pressed)
                .expect("show F1 help");
            assert!(app.runtime_help_visible);
            app.handle_key(VirtualKeyCode::F1, ElementState::Released)
                .expect("F1 release has no help callback");
            assert!(app.runtime_help_visible);

            let mut after_pixels = vec![0_u8; 320 * 200 * 4];
            app.render(&mut after_pixels).expect("render after F1");
            assert_ne!(after_pixels, before_pixels);

            // Repeated key-down events execute ToggleShowHelp each time.
            app.handle_key(VirtualKeyCode::F1, ElementState::Pressed)
                .expect("repeat hides F1 help");
            assert!(!app.runtime_help_visible);
            let mut hidden_again = vec![0_u8; 320 * 200 * 4];
            app.render(&mut hidden_again).expect("render hidden help");
            assert_eq!(hidden_again, before_pixels);

            app.handle_key(VirtualKeyCode::F1, ElementState::Pressed)
                .expect("show help before new-game reset");
            assert!(app.runtime_help_visible);
            app.configure_running_state("Next game".to_string(), DEFAULT_GROUND_HEIGHT);
            assert!(!app.runtime_help_visible);
        }
    }

    #[test]
    fn l002_ownerless_escape_opens_fullscreen_abort_confirmation() {
        let mut app = new_running_sandbox_app();
        let removed_owner = app.local_owner;
        app.engine
            .remove_player(removed_owner)
            .expect("remove local player for passive observer");
        app.engine.set_local_players([]);
        app.local_controls = LocalControlRegistry::default();
        app.snapshot = app.engine.snapshot();
        app.refresh_non_authoritative_physical_viewports();
        assert!(app.primary_physical_viewport_is_no_owner());

        app.handle_key(VirtualKeyCode::Escape, ElementState::Pressed)
            .expect("ownerless Escape opens fullscreen abort confirmation");
        assert!(app.message_dialogs.last().is_some_and(|dialog| matches!(
            dialog.continuation,
            MessageDialogContinuation::AbortGame { .. }
        )));
        assert!(app.ingame_menu.is_none());
        assert!(!app.ingame_menu_belongs_to(app.local_owner));
        assert!(matches!(app.mode, AppMode::Running));
        assert!(!app.take_exit_request());
    }

    #[test]
    fn material_render_info_preserves_cpp_color_alpha_and_overlay_fields() {
        // C4MaterialCore::CompileFunc parses Color and then ColorX into the
        // same 3x3 array, followed by the 3x2 Alpha array, TextureOverlay,
        // and OverlayType (C4Material.cpp:170-204; C4Material.h:79-105).
        let library = clonk_resources::MaterialLibrary::parse(
            "[Material]\n\
                 Name=Earth\n\
                 Color=1,2,3,4,5,6,7,8,9\n\
                 ColorX=11,12,13,14\n\
                 Alpha=21,22,23,24\n\
                 Density=50\n\
                 TextureOverlay=Smooth\n\
                 OverlayType=8\n\
                 PXSGfx=Snow\n\
                 PXSGfxRt=1,2,16,8,-8,-4\n\
                 PXSGfxSize=10\n",
        )
        .expect("material library");
        let definition = library.get("Earth").expect("Earth material");

        assert_eq!(
            material_render_info(definition),
            clonk_frontend::MaterialRenderInfo::new(
                [11, 12, 13, 14, 0, 0, 0, 0, 0],
                [21, 22, 23, 24, 0, 0],
                Some("Smooth".to_string()),
                8,
                50,
            )
            .with_placement(70)
            .with_pxs_graphics(Some("Snow".to_string()), [1, 2, 16, 8, -8, -4], 10),
        );
    }

    #[test]
    fn material_render_info_preserves_signed_placement_and_defaults_only_zero() {
        // C4MaterialCore::Load substitutes the density-derived placement only
        // when Placement is exactly zero; negative and positive values reach
        // the landscape shading metadata unchanged (C4Material.cpp:145-158).
        for (placement, expected) in [(-17, -17), (0, 70), (23, 23)] {
            let library = clonk_resources::MaterialLibrary::parse(&format!(
                "[Material]\nName=Earth\nDensity=50\nPlacement={placement}\n"
            ))
            .expect("material library");
            let definition = library.get("Earth").expect("Earth material");

            assert_eq!(material_render_placement(definition), expected);
            assert_eq!(
                material_render_info(definition),
                clonk_frontend::MaterialRenderInfo::new([0; 9], [0; 6], None, 0, 50)
                    .with_placement(expected),
            );
        }
    }

    #[test]
    fn material_render_info_defaults_pxs_size_to_facet_width() {
        // PXSGfxSize defaults to PXSGfxRt.Wdt during material compilation
        // (C4Material.cpp:205-207).
        let library = clonk_resources::MaterialLibrary::parse(
            "[Material]\nName=Lava\nPXSGfx=Lava\nPXSGfxRt=0,0,32,32,-16,-16\n",
        )
        .expect("material library");
        let definition = library.get("Lava").expect("Lava material");

        assert_eq!(
            material_render_info(definition),
            clonk_frontend::MaterialRenderInfo::new([0; 9], [0; 6], None, 0, 0)
                .with_placement(5)
                .with_pxs_graphics(Some("Lava".to_string()), [0, 0, 32, 32, -16, -16], 32),
        );
    }

    #[test]
    fn hazard_tutorial_inherits_parent_material_metadata_and_textures() {
        // C4Game opens the NRT_Material chain from the scenario through its
        // parent groups before the global material group. Hazard's Tutorial
        // has no local Material.c4g: Rain and Industrial1.png live in the
        // parent Hazard.c4f/Material.c4g (C4GameParameters.cpp:211-223;
        // C4Game.cpp:899-965).
        let _env_lock = crate::tests::env_lock().lock();
        reset_cached_app_paths();
        let repository = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .parent()
            .and_then(Path::parent)
            .expect("repository root")
            .to_path_buf();
        let _guard = EnvGuard::set(&[("LC_INSTALL_ROOT", Some(repository.as_path()))]);
        let tutorial = repository.join("content/Hazard.c4f/Tutorial.c4s");

        let render_info = load_material_render_info(&tutorial, None);
        assert_eq!(
            render_info.get("rain"),
            Some(
                &clonk_frontend::MaterialRenderInfo::new(
                    [7, 35, 140, 7, 35, 140, 7, 35, 140],
                    [0; 6],
                    Some("Liquid".to_string()),
                    0,
                    25,
                )
                .with_placement(10)
            ),
        );
        assert_eq!(
            render_info.get("ashes"),
            Some(
                &clonk_frontend::MaterialRenderInfo::new(
                    [90, 78, 40, 82, 80, 78, 100, 96, 90],
                    [0; 6],
                    Some("Spots".to_string()),
                    0,
                    50,
                )
                .with_placement(30)
                .with_pxs_graphics(Some("Ashes".to_string()), [0, 0, 32, 32, -16, -16], 32,),
            ),
            "the parent default PXSGfxSize=32 must beat global Ashes size 6",
        );
        assert!(
            load_scenario_material_textures(&tutorial, None).contains_key("industrial1"),
            "parent-group texture must load through Group::open_child"
        );

        let paths = cached_app_paths().expect("repository app paths");
        let scenario = Scenario::load_from_path_with_languages(
            &tutorial,
            &InstallDefinitionResolver::new(Some(paths)),
            &["US"],
        )
        .expect("Hazard tutorial loads through the authoritative material chain");
        let mut engine = Engine::new();
        scenario
            .apply_before_players(&mut engine)
            .expect("Hazard material library applies");
        let rain = engine
            .materials()
            .id_of("Rain")
            .and_then(|id| engine.materials().get_by_id(id))
            .expect("parent-only Rain material reaches engine physics");
        assert_eq!(rain.density(), 25);

        reset_cached_app_paths();
    }

    #[test]
    fn presentation_texture_long_name_collisions_keep_the_latest_surface() {
        let root = tempdir().expect("temp texture images");
        let first_path = root.path().join("first.png");
        let second_path = root.path().join("second.png");
        write_preview_png(&first_path, [10, 20, 30, 255]);
        write_preview_png(&second_path, [40, 50, 60, 255]);

        let prefix = "ColliderPrefixX";
        assert_eq!(clonk_script::c4_string_bytes(prefix).len(), 15);
        let mut source = clonk_resources::MutableGroup::new("Material.c4g");
        source
            .add_file("Broken.png", b"not a PNG".to_vec())
            .unwrap();
        source
            .add_file(
                "ColliderPrefixXA.png",
                fs::read(&first_path).expect("first PNG bytes"),
            )
            .unwrap();
        source
            .add_file(
                "ColliderPrefixXB.png",
                fs::read(&second_path).expect("second PNG bytes"),
            )
            .unwrap();
        source
            .add_file(
                "Mislabeled.bmp",
                fs::read(&first_path).expect("mislabeled PNG bytes"),
            )
            .unwrap();
        source
            .add_file(
                "IndexedOnly.bmp",
                clonk_resources::bitmap::IndexedBitmap {
                    width: 1,
                    height: 1,
                    indices: vec![5],
                }
                .encode()
                .expect("indexed BMP bytes"),
            )
            .unwrap();
        let group = Group::from_raw_memory(
            PathBuf::from("Material.c4g"),
            source.pack_raw().expect("packed material group"),
        )
        .expect("open material group");

        let mut textures = HashMap::new();
        let mut inventory = Vec::new();
        absorb_material_texture_group(&group, &mut textures, &mut inventory);
        assert_eq!(
            inventory,
            vec![
                "Broken".to_string(),
                prefix.to_string(),
                prefix.to_string(),
                "IndexedOnly".to_string(),
            ],
            "full long names both admit before their fixed identities collide"
        );
        let broken = textures
            .get("broken")
            .and_then(MaterialTextureSurface::surface32_image)
            .expect("invalid PNG retains an empty Surface32 identity");
        assert_eq!(
            (broken.width(), broken.height(), broken.pixels()),
            (0, 0, &[][..])
        );
        assert_eq!(
            textures
                .get(&clonk_resources::material::c4_name_key(prefix))
                .expect("colliding PNG surface")
                .surface32_image()
                .expect("latest collider is a Surface32 PNG")
                .pixels(),
            &[40, 50, 60, 255],
            "the later fixed-name collision shadows the earlier surface"
        );
        assert_eq!(
            textures
                .get("indexedonly")
                .expect("indexed BMP surface")
                .indexed_pixels(),
            Some((1, 1, [2].as_slice())),
            "Surface8 admission applies AllowColor(0, 2, true)"
        );
        assert!(!inventory.iter().any(|name| name == "Mislabeled"));
    }

    #[test]
    fn material_render_bytes_keep_the_cpp_uint32_low_byte() {
        // C4MaterialCore stores Color/Alpha as uint32 arrays and the INI
        // compiler reads them with strtoul (C4Material.h:79-81;
        // StdCompiler.cpp:651-654). Palette/texture composition then narrows
        // to uint8, so out-of-range values wrap instead of clamping.
        assert_eq!(
            material_value_array::<4>(Some(vec![255, 256, -1, 511])),
            [255, 0, 255, 255],
        );
    }

    #[test]
    fn set_plr_show_command_request_force_enables_display_once() {
        let mut app = new_state_only_menu_app(320, 200);
        app.display_flags.show_commands = false;
        app.show_commands_requests.request_enable();
        app.apply_show_commands_enable_request();
        assert!(app.display_flags.show_commands);

        // The native call writes true once; a later user toggle remains off
        // until another SetPlrShowCommand call.
        app.display_flags.show_commands = false;
        app.apply_show_commands_enable_request();
        assert!(!app.display_flags.show_commands);
    }
