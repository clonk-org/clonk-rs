// Spliced into `mod tests` (src/main_tests.rs) via include!: a bare item
// sequence, not a child module, so test ids stay `tests::<fn>`.

#[test]
fn console_open_close_and_message_fallback_follow_app_state() {
    let mut startup = new_state_only_menu_app(320, 200);
    startup.console_mode = true;
    let (boot_sender, boot_receiver) = mpsc::channel();
    startup.boot_loading = Some(BootLoadingState::new(boot_receiver));
    startup.mode = AppMode::Loading;
    startup
        .process_console_command(
            "/open \"Missions/My Round/Scenario.txt\" /network /lobby:17 \"/comment:console game\"",
        )
        .test_value();
    main_assert_eq!(startup.classic_command_line.scenario => Some(PathBuf::from("Missions/My Round")));
    main_assert_eq!(startup.classic_command_line.network_active => Some(true));
    main_assert_eq!(startup.classic_command_line.lobby_timeout => Some(Some(17)));
    main_assert_eq!(startup.classic_command_line.comment.as_deref() => Some("console game"));
    main_assert!(startup.auto_start_classic_command_line_scenario);
    startup.process_console_command("/close").test_value();
    main_assert_eq!(startup.mode => AppMode::Loading);
    main_assert!(startup.boot_loading.is_some());
    main_assert!(!startup.auto_start_classic_command_line_scenario);
    boot_sender
        .send(BootLoadingEvent::Finished(None))
        .test_value();
    startup.poll_boot_loading();
    main_assert_eq!(startup.mode => AppMode::Menu);
    main_assert!(startup.boot_loading.is_none());
    main_assert!(startup.console_startup_active());

    let (_query_sender, query_receiver) = mpsc::channel::<
        std::result::Result<ClassicDirectReferenceQueryResult, NetworkStartError>,
    >();
    startup.classic_direct_reference_query = Some(ClassicDirectReferenceQuery {
        receiver: query_receiver,
    });
    let pending_join_arguments = startup.classic_command_line.clone();
    startup
        .process_console_command("/open Replacement.c4s")
        .test_value();
    main_assert_eq!(startup.classic_command_line => pending_join_arguments);
    startup.process_console_command("/close").test_value();
    main_assert_eq!(startup.mode => AppMode::Menu);
    main_assert!(startup.classic_direct_reference_query.is_none());
    main_assert!(startup.console_startup_active());
    startup
        .process_console_command("/open /comment:replacement")
        .test_value();
    main_assert_eq!(startup.classic_command_line.comment.as_deref() => Some("replacement"));

    let mut running = new_state_only_lightweight_running_sandbox_app();
    let (network, _events, mut commands) = NetworkManager::test_stub_with_commands_for_client_id(0);
    running.network = Some(network);
    running
        .process_console_command("administrator message")
        .test_value();
    main_assert_eq!(
        commands.take_submitted_messages() =>
        vec![MessageControlData {
            message_type: MESSAGE_TYPE_NORMAL,
            player: running.local_owner,
            to_player: -1,
            message: LegacyCString::from_bytes(b"administrator message".to_vec())
                .expect("fixture message is NUL-free"),
            by_client: 0,
        }]
    );
    running.full_speed = true;
    running.frame_skip = 9;
    running.process_console_command("/close").test_value();
    main_assert_eq!(running.mode => AppMode::Menu);
    main_assert!(running.active_scenario.is_none());
    main_assert!(!running.full_speed);
    main_assert_eq!(running.frame_skip => 1);
}

#[test]
fn running_chat_does_not_capture_release_from_active_world_moving_drag() {
    let mut app = new_running_sandbox_app();
    let owner = app.local_owner;
    let crew = app.engine.test_crew_cursor(owner);
    let crew_position = app.engine.test_object_snapshot(crew).position;
    let mut item = test_definition("CHDG", "Chat drag item", "#strict\n");
    item.set_category(clonk_engine::CATEGORY_OBJECT);
    item.set_collectible(true);
    item.set_shape_rect(Some(clonk_engine::DefinitionRect::new(-4, -4, 8, 8)));
    app.engine.register_test_definition(item);
    let layer = app.engine.test_object_snapshot(crew).layer;
    let spawn = layer
        .map(|layer| {
            SpawnConfig::new("CHDG")
                .with_position(Vector2::new(crew_position.x - 60, crew_position.y))
                .with_layer(layer)
        })
        .unwrap_or_else(|| {
            SpawnConfig::new("CHDG")
                .with_position(Vector2::new(crew_position.x - 60, crew_position.y))
        });
    let target = app.engine.spawn_test_object(spawn);
    render_mouse_test_app(&mut app);
    let start = mouse_test_object_point(&app, owner, target);
    let (end, _) = mouse_test_empty_point(&mut app, owner, start, None);

    app.test_cursor(PhysicalPosition::new(
        f64::from(start.x),
        f64::from(start.y),
    ));
    app.test_left_button(ElementState::Pressed);
    app.test_cursor(PhysicalPosition::new(f64::from(end.x), f64::from(end.y)));
    main_assert!(app.mouse_state.is_some_and(|state| { state.motion.moved && state.motion.world_drag_started }));

    app.start_running_chat(RunningChatMode::All);
    app.push_message_dialog(
        clonk_frontend::message_dialog::MessageDialogState::regular_ok(
            "Lower notice",
            "Message",
            clonk_frontend::message_dialog::MessageDialogIcon::NOTIFY,
        ),
        MessageDialogContinuation::None,
    )
    .test_value();
    main_assert!(app.mouse_state.is_some_and(|state| state.motion.world_drag_started));
    app.test_left_button(ElementState::Released);
    main_assert!(app.mouse_state.is_none());
    main_assert!(app.running_chat.is_some());
    main_assert_eq!(app.message_dialogs.len() => 1);
    main_assert_eq!(app.running_chat_text() => Some(""));
}

fn message_speech_test_engine(samples: &[String]) -> Engine {
    let mut engine = Engine::with_seed(1);
    engine
        .register_player(
            PlayerConfig::new(0, "Player")
                .with_viewports([clonk_engine::PlayerViewport::new(Vector2::ZERO)]),
        )
        .test_value();
    engine.configure_sound_samples(samples);
    engine
}

fn execute_message_speech_for_audio(
    engine: &mut Engine,
    audio: &mut AudioContext,
    script: &str,
    audio_snapshot: &SimulationSnapshot,
) -> Vec<clonk_engine::MessageSnapshot> {
    let control = clonk_engine::ScriptControlData {
        script: LegacyCString::from_bytes(script.as_bytes().to_vec()).test_value(),
        by_client: 0,
        ..clonk_engine::ScriptControlData::default()
    };
    engine
        .execute_script_control(&control, ScriptControlPolicy::live(false))
        .expect("message script executes")
        .test_value();

    let mut snapshot = audio_snapshot.clone();
    snapshot.audio = std::mem::take(&mut engine.pending_audio);
    let mut runtime_music_enabled = false;
    let outcomes = audio.process_audio_with_viewports(&snapshot, &[], &mut runtime_music_enabled);
    engine.apply_speech_playback_outcomes(outcomes)
}

#[test]
fn message_speech_falls_back_when_new_instance_is_rejected() {
    let dir = tempdir();
    let scenario = dir.path().join("Speech.c4s");
    fs::create_dir_all(&scenario).test_value();
    fs::write(scenario.join("Speech.wav"), silent_pcm_wav(10_000)).test_value();
    fs::write(scenario.join("Blocker.wav"), silent_pcm_wav(10_000)).test_value();

    let make_audio = |max_channels| {
        let mut audio = AudioContext::try_new(AudioOptions {
            max_channels,
            ..AudioOptions::default()
        })
        .test_value();
        audio.configure_scenario(Some(&scenario));
        audio
    };
    let empty_snapshot = make_snapshot(Vec::new(), Vec::new());

    // StartSoundEffect calls NewInstance directly, so the second global
    // request reaches resolved-sample near dedup even though its raw name
    // and target match the first request.
    let mut duplicate_audio = make_audio(2);
    let samples = duplicate_audio.available_sound_samples();
    let mut duplicate_engine = message_speech_test_engine(&samples);
    let messages = execute_message_speech_for_audio(
        &mut duplicate_engine,
        &mut duplicate_audio,
        r#"Message("hidden$Speech")"#,
        &empty_snapshot,
    );
    main_assert!(messages.is_empty(), "a created instance suppresses text");
    let messages = execute_message_speech_for_audio(
        &mut duplicate_engine,
        &mut duplicate_audio,
        r#"Message("duplicate fallback$Speech")"#,
        &empty_snapshot,
    );
    main_assert_eq!(messages.len() => 1);
    main_assert_eq!(messages[0].kind => MessageKind::Global);
    main_assert_eq!(messages[0].lines => ["duplicate fallback"]);
    main_assert_eq!(duplicate_audio.active_channels.len() => 1);

    // Twenty spatially separated logical instances consume the native
    // per-sample allowance even when they are inaudible and channel-less.
    let mut capped_audio = make_audio(1);
    let sources = (0..20)
        .map(|index| {
            make_object(
                index + 1,
                "SPCH",
                Vector2::new(i32::try_from(index).test_value() * 51, 0),
            )
        })
        .collect::<Vec<_>>();
    let capped_snapshot = make_snapshot(sources.clone(), Vec::new());
    for source in &sources {
        main_assert!(capped_audio.try_start_sound("Speech", Some(source.id), 100, false, true, None, &capped_snapshot, &[],).expect("seed speech instance"));
    }
    let samples = capped_audio.available_sound_samples();
    let mut capped_engine = message_speech_test_engine(&samples);
    let messages = execute_message_speech_for_audio(
        &mut capped_engine,
        &mut capped_audio,
        r#"PlayerMessage(0,"cap fallback$Speech")"#,
        &capped_snapshot,
    );
    main_assert_eq!(messages.len() => 1);
    main_assert_eq!(messages[0].kind => MessageKind::GlobalPlayer);
    main_assert_eq!(messages[0].lines => ["cap fallback"]);
    main_assert_eq!(capped_audio.active_channels.len() => 20);

    // A distinct sample occupying the only mixer slot makes the initial
    // channel allocation fail; the rejected speech is never inserted.
    let mut channel_audio = make_audio(1);
    channel_audio
        .start_sound(
            "Blocker",
            None,
            100,
            false,
            true,
            None,
            &empty_snapshot,
            &[],
        )
        .test_value();
    let samples = channel_audio.available_sound_samples();
    let mut channel_engine = message_speech_test_engine(&samples);
    let messages = execute_message_speech_for_audio(
        &mut channel_engine,
        &mut channel_audio,
        r#"PlrMessage("channel fallback$Speech",0)"#,
        &empty_snapshot,
    );
    main_assert_eq!(messages.len() => 1);
    main_assert_eq!(messages[0].kind => MessageKind::GlobalPlayer);
    main_assert_eq!(messages[0].lines => ["channel fallback"]);
    main_assert!(channel_audio.active_channel_key("Speech", None).is_none());

    // Muting skips physical allocation but still creates C++'s logical
    // instance, so text remains suppressed.
    let mut muted_audio = make_audio(1);
    muted_audio.options.sound_enabled = false;
    let samples = muted_audio.available_sound_samples();
    let mut muted_engine = message_speech_test_engine(&samples);
    let messages = execute_message_speech_for_audio(
        &mut muted_engine,
        &mut muted_audio,
        r#"Message("muted$Speech")"#,
        &empty_snapshot,
    );
    main_assert!(messages.is_empty());
    let muted = muted_audio
        .active_channels
        .get(&SoundInstanceKey::new("Speech", None))
        .test_value();
    main_assert!(muted.channel.is_none());

    // Filename inventory rejection remains synchronous and never emits a
    // frontend command.
    let mut missing_engine = message_speech_test_engine(&[]);
    let mut missing_audio = make_audio(1);
    let messages = execute_message_speech_for_audio(
        &mut missing_engine,
        &mut missing_audio,
        r#"Message("missing fallback$Absent")"#,
        &empty_snapshot,
    );
    main_assert_eq!(messages.len() => 1);
    main_assert_eq!(messages[0].lines => ["missing fallback"]);
}

#[test]
fn missing_player_scoped_messages_are_not_drawable() {
    // C4GameMessage::Draw compares C4GM_*Player's raw Player field to
    // each viewport owner. FnPlayerMessage(42, ...) therefore reaches no
    // normal viewport when player 42 is absent (C4GameMessage.cpp:104,180).
    let app = new_state_only_running_sandbox_app();
    let viewports = [ActiveViewportProjection {
        index: 0,
        identity: None,
        owner: app.local_owner,
        is_no_owner_viewport: false,
        rect: Rect::new(0, 0, 320, 200),
        content_rect: Rect::new(0, 0, 320, 200),
        target_x: 0,
        target_y: 0,
        logical_width: 320,
        logical_height: 200,
        content_origin_x: 0.0,
        content_origin_y: 0.0,
        zoom: 1.0,
    }];
    main_assert_ne!(viewports[0].owner => 42);

    let mut message = clonk_engine::MessageSnapshot {
        id: 1,
        kind: MessageKind::GlobalPlayer,
        lines: vec!["Secret".to_string()],
        target: None,
        player: Some(42),
        offset: Vector2::ZERO,
        color: 0xffff_ffff,
        flags: 0,
        width: None,
        decoration: None,
        frame_decoration: None,
        portrait: None,
    };
    main_assert_eq!(app.hud_message_drawability(&message, &viewports) => HudMessageDrawability::NotDrawable);

    message.kind = MessageKind::TargetPlayer;
    message.target = app.snapshot.objects.first().map(|object| object.id);
    main_assert!(message.target.is_some());
    main_assert_eq!(app.hud_message_drawability(&message, &viewports) => HudMessageDrawability::NotDrawable);
}

#[test]
fn scale_one_point_five_message_batch_carries_its_isolated_clipper() {
    let mut app = new_classic_running_sandbox_app();
    let decoration = clonk_engine::ObjectMenuFrameDecoration {
        source_definition: "TEST".to_string(),
        background_color: 0x8032_3232,
        border_top: 0,
        border_left: 0,
        border_right: 0,
        border_bottom: 0,
        top: None,
        top_right: None,
        right: None,
        bottom_right: None,
        bottom: None,
        bottom_left: None,
        left: None,
        top_left: None,
    };
    app.snapshot.hud.messages = vec![clonk_engine::MessageSnapshot {
        id: 1,
        kind: MessageKind::GlobalPlayer,
        lines: vec!["Fractional clip".to_string()],
        target: None,
        player: Some(app.local_owner),
        offset: Vector2::new(1, 1),
        color: 0xffff_ffff,
        flags: FLAG_LEFT | FLAG_TOP,
        width: Some(120),
        decoration: Some("TEST".to_string()),
        frame_decoration: Some(decoration),
        // Requesting a portrait selects the framed message layout. The
        // missing test definition is harmless; the frame still supplies
        // an isolated raster layer next to the captured text.
        portrait: Some("Portrait:TEST::000000::1".to_string()),
    }];
    install_native_test_fonts(&mut app, 1.5);

    let (_, _, plan) = render_ordered_test_frame(&mut app, 1.5, 480, 300);
    let viewport = app
        .graphics
        .active_viewport_projections()
        .into_iter()
        .find(|viewport| viewport.owner == app.local_owner)
        .test_value()
        .rect;
    let batch = plan
        .batches
        .iter()
        .find(|batch| {
            batch
                .text
                .iter()
                .any(|command| command.text == "Fractional clip")
        })
        .test_value();

    main_assert!(batch.logical_layer.is_some(), "message frame is rasterized");
    main_assert_eq!(batch.clip => Some(viewport));
    main_assert!(batch.text.iter().all(|command| command.clip == Some(viewport)));
    main_assert_eq!(
        plan.batches
            .iter()
            .filter(|candidate| candidate.clip.is_some())
            .count() =>
        1,
        "unproven HUD and scoreboard batches keep full-frame composition"
    );
}

#[test]
fn secondary_local_viewport_draws_its_player_global_message_only_there() {
    let mut app = new_classic_running_sandbox_app();
    let local = app
        .snapshot
        .players
        .iter()
        .find(|player| player.id == app.local_owner)
        .cloned()
        .test_value();
    let mut secondary = local;
    secondary.id = app.local_owner + 1;
    secondary.name = "Secondary local".to_string();
    app.snapshot.players.push(secondary.clone());
    app.snapshot.hud.local_players.push(secondary.id);
    let secondary_viewport = app.owned_physical_viewport_state(secondary.id, true);
    app.physical_viewports.push(secondary_viewport);
    app.physical_viewports_authoritative = true;
    app.update_film_viewport_availability();
    app.snapshot.hud.messages = vec![clonk_engine::MessageSnapshot {
        id: 1,
        kind: MessageKind::GlobalPlayer,
        lines: vec!["Secondary".to_string()],
        target: None,
        player: Some(secondary.id),
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
    app.test_render(&mut baseline);
    app.snapshot.hud.messages = messages;
    let mut rendered = vec![0; 320 * 200 * 4];
    app.test_render(&mut rendered);
    let viewport = app
        .graphics
        .active_viewport_projections()
        .into_iter()
        .find(|viewport| viewport.owner == secondary.id)
        .test_value()
        .rect;
    let changed = rendered
        .chunks_exact(4)
        .zip(baseline.chunks_exact(4))
        .enumerate()
        .filter_map(|(index, (actual, before))| (actual != before).then_some(index))
        .collect::<Vec<_>>();
    main_assert!(!changed.is_empty(), "the secondary message contributes pixels");
    main_assert!(changed.iter().all(|index| {
        let x = (*index % 320) as i32;
        let y = (*index / 320) as i32;
        x >= viewport.x
            && x < viewport.x + viewport.width as i32
            && y >= viewport.y
            && y < viewport.y + viewport.height as i32
    }));
}

#[test]
fn target_message_regular_parallax_matches_cpp_integer_order() {
    let mut target = make_object(1, "Parallax", Vector2::new(1_000, 50));
    target.category |= C4D_PARALLAX;
    target
        .local_vars
        .insert("__local_0".to_string(), Value::Int(50));
    target
        .local_vars
        .insert("__local_1".to_string(), Value::Int(150));
    let viewport = ActiveViewportProjection {
        index: 0,
        identity: None,
        owner: 1,
        is_no_owner_viewport: false,
        rect: Rect::new(10, 20, 400, 200),
        content_rect: Rect::new(10, 20, 400, 200),
        target_x: 201,
        target_y: -99,
        logical_width: 400,
        logical_height: 200,
        content_origin_x: 201.0,
        content_origin_y: -99.0,
        zoom: 1.0,
    };

    let position = c4_message_target_position(&target, Vector2::new(7, 11), 21, viewport);

    main_assert_eq!(position => Vector2::new(1_107, 95));
}

#[test]
fn target_message_zero_parallax_negative_position_anchors_right_bottom() {
    let mut target = make_object(1, "Parallax", Vector2::new(-20, -30));
    target.category |= C4D_PARALLAX;
    target
        .local_vars
        .insert("__local_0".to_string(), Value::Int(0));
    target
        .local_vars
        .insert("__local_1".to_string(), Value::Int(0));
    let viewport = ActiveViewportProjection {
        index: 0,
        identity: None,
        owner: 1,
        is_no_owner_viewport: false,
        rect: Rect::new(10, 20, 800, 400),
        content_rect: Rect::new(30, 50, 760, 340),
        target_x: 123,
        target_y: -45,
        logical_width: 400,
        logical_height: 200,
        content_origin_x: 133.0,
        content_origin_y: -30.0,
        zoom: 2.0,
    };

    let position = c4_message_target_position(&target, Vector2::new(4, 6), 20, viewport);

    main_assert_eq!(position => Vector2::new(507, 116));
    main_assert!(viewport.contains_logical_point(position));
    let output = viewport.logical_to_output(position);
    main_assert_eq!(output => (778.0, 342.0));
    main_assert!(viewport.contains_output_point(output));
}

#[test]
fn fractional_zoom_rounded_border_keeps_logical_edge_message_drawable() {
    let mut app = new_running_sandbox_app();
    let player = app
        .snapshot
        .players
        .iter_mut()
        .find(|player| player.id == app.local_owner)
        .test_value();
    // Model SetFoW(false) so this case exercises rounding independently
    // of the visibility-bitmap boundary.
    player.fog_of_war = false;
    player.force_fog_of_war = true;
    let target = app.snapshot.objects.first_mut().test_value();
    target.position = Vector2::new(100, 50);
    let target_id = target.id;
    let shape_height = app
        .engine
        .definition_shape_rect(&target.definition_id)
        .map(|shape| shape.height)
        .unwrap_or(0);
    let message = clonk_engine::MessageSnapshot {
        id: 1,
        kind: MessageKind::TargetPlayer,
        lines: vec!["Logical edge".to_string()],
        target: Some(target_id),
        player: Some(app.local_owner),
        offset: Vector2::new(0, shape_height / 2 + 5),
        color: 0xffff_ffff,
        flags: 0,
        width: None,
        decoration: None,
        frame_decoration: None,
        portrait: None,
    };
    let viewport = ActiveViewportProjection {
        index: 0,
        identity: None,
        owner: app.local_owner,
        is_no_owner_viewport: false,
        rect: Rect::new(10, 20, 100, 100),
        // One logical border pixel at 0.4x rounds to zero output pixels.
        content_rect: Rect::new(10, 20, 100, 100),
        target_x: 100,
        target_y: 0,
        logical_width: 250,
        logical_height: 250,
        content_origin_x: 101.0,
        content_origin_y: 0.0,
        zoom: 0.4,
    };
    let target = app.snapshot.object(target_id).test_value();
    let position = c4_message_target_position(target, message.offset, shape_height, viewport);
    main_assert!(viewport.contains_logical_point(position));
    let output = viewport.logical_to_output(position);
    main_assert!(output.0 > 9.0 && output.0 < 10.0);
    main_assert!(!viewport.contains_output_point(output));

    main_assert_eq!(
        app.hud_message_drawability(&message, &[viewport]) =>
        HudMessageDrawability::Drawable,
        "C++ accepts the logical edge without a second physical-rect test"
    );
}

#[test]
fn offscreen_target_player_message_does_not_trigger_renderer_boundary() {
    let mut app = new_running_sandbox_app();
    let target = app
        .snapshot
        .objects
        .first()
        .map(|object| object.id)
        .test_value();
    app.snapshot.hud.messages = vec![clonk_engine::MessageSnapshot {
        id: 1,
        kind: MessageKind::TargetPlayer,
        lines: vec!["Offscreen".to_string()],
        target: Some(target),
        player: Some(app.local_owner),
        offset: Vector2::new(1_000_000, 1_000_000),
        color: 0xffff_ffff,
        flags: 0,
        width: None,
        decoration: None,
        frame_decoration: None,
        portrait: None,
    }];

    let mut frame = vec![0; 320 * 200 * 4];
    app.test_render(&mut frame);
}

#[test]
fn target_messages_render_only_for_cpp_visibility_and_fog() {
    let mut app = new_running_sandbox_app();
    let target = app
        .snapshot
        .objects
        .first()
        .map(|object| object.id)
        .test_value();
    let shape_height = app
        .snapshot
        .object(target)
        .and_then(|object| app.engine.definition_shape_rect(&object.definition_id))
        .map(|shape| shape.height)
        .unwrap_or(0);
    let mut message = clonk_engine::MessageSnapshot {
        id: 1,
        kind: MessageKind::Target,
        lines: vec!["A".to_string()],
        target: Some(target),
        player: None,
        // Cancel the native half-shape/+5 lift so the FoW probe is the
        // target's own position and its strict range is unambiguous.
        offset: Vector2::new(0, shape_height / 2 + 5),
        color: 0xffff_ffff,
        flags: 0,
        width: None,
        decoration: None,
        frame_decoration: None,
        portrait: None,
    };
    let target_position = app.snapshot.object(target).test_value().position;
    let player = app
        .snapshot
        .players
        .iter_mut()
        .find(|player| player.id == app.local_owner)
        .test_value();
    player.fog_of_war = true;
    player.force_fog_of_war = true;
    player.viewports =
        vec![clonk_engine::PlayerViewport::new(target_position).with_focus(Some(target))];
    let target_object = app
        .snapshot
        .objects
        .iter_mut()
        .find(|object| object.id == target)
        .test_value();
    target_object.visibility = clonk_engine::VIS_ALL;
    target_object.category &= !C4D_IGNORE_FOW;
    target_object.plr_view_range = 128;
    app.snapshot.fow_players.insert(
        app.local_owner,
        clonk_engine::FogOfWarPlayerFrame {
            view_objects: vec![target],
            view_target: None,
        },
    );

    let mut baseline = vec![0; 320 * 200 * 4];
    app.test_render(&mut baseline);
    let viewports = app.graphics.active_viewport_projections();
    main_assert_eq!(app.hud_message_drawability(&message, &viewports) => HudMessageDrawability::Drawable);
    let viewport = viewports
        .iter()
        .copied()
        .find(|viewport| viewport.owner == app.local_owner)
        .test_value();
    let anchor = app
        .target_message_position_for_viewport(&message, viewport)
        .test_value();
    let output_anchor = viewport.logical_to_output(anchor);
    app.snapshot.hud.messages = vec![message.clone()];
    let mut visible = vec![0; 320 * 200 * 4];
    app.test_render(&mut visible);
    let changed = visible
        .chunks_exact(4)
        .zip(baseline.chunks_exact(4))
        .enumerate()
        .filter_map(|(index, (actual, before))| (actual != before).then_some(index))
        .collect::<Vec<_>>();
    main_assert!(!changed.is_empty(), "the visible target message draws pixels");
    main_assert!(changed.iter().all(|index| {
        let x = (*index % 320) as i32;
        let y = (*index / 320) as i32;
        x >= viewport.rect.x
            && x < viewport.rect.x + viewport.rect.width as i32
            && y >= viewport.rect.y
            && y < viewport.rect.y + viewport.rect.height as i32
            && y <= output_anchor.1.round() as i32
    }));

    // An active FoW player with no revealing view object silently skips
    // the message, rather than failing the entire overlay batch.
    app.snapshot.hud.messages.clear();
    app.snapshot.fow_players.insert(
        app.local_owner,
        clonk_engine::FogOfWarPlayerFrame {
            view_objects: Vec::new(),
            view_target: None,
        },
    );
    let mut fog_baseline = vec![0; 320 * 200 * 4];
    app.test_render(&mut fog_baseline);
    let viewports = app.graphics.active_viewport_projections();
    main_assert_eq!(app.hud_message_drawability(&message, &viewports) => HudMessageDrawability::NotDrawable);
    app.snapshot.hud.messages = vec![message.clone()];
    let mut fogged = vec![0; 320 * 200 * 4];
    app.test_render(&mut fogged);
    main_assert_eq!(fogged => fog_baseline);

    // C4GM_Target additionally honors C4Object::IsVisible. The player-
    // scoped target variant deliberately does not use that predicate.
    app.snapshot.hud.messages.clear();
    app.snapshot
        .players
        .iter_mut()
        .find(|player| player.id == app.local_owner)
        .test_value()
        .fog_of_war = false;
    app.snapshot
        .objects
        .iter_mut()
        .find(|object| object.id == target)
        .test_value()
        .visibility = clonk_engine::VIS_NONE;
    let mut visibility_baseline = vec![0; 320 * 200 * 4];
    app.test_render(&mut visibility_baseline);
    let viewports = app.graphics.active_viewport_projections();
    main_assert_eq!(app.hud_message_drawability(&message, &viewports) => HudMessageDrawability::NotDrawable);
    app.snapshot.hud.messages = vec![message.clone()];
    let mut invisible = vec![0; 320 * 200 * 4];
    app.test_render(&mut invisible);
    main_assert_eq!(invisible => visibility_baseline);

    message.kind = MessageKind::TargetPlayer;
    message.player = Some(app.local_owner);
    main_assert_eq!(
        app.hud_message_drawability(&message, &app.graphics.active_viewport_projections()) =>
        HudMessageDrawability::Drawable,
        "C4GM_TargetPlayer bypasses C4Object::IsVisible"
    );

    message.kind = MessageKind::Target;
    message.player = None;
    app.snapshot
        .players
        .iter_mut()
        .find(|player| player.id == app.local_owner)
        .test_value()
        .fog_of_war = true;
    let target_object = app
        .snapshot
        .objects
        .iter_mut()
        .find(|object| object.id == target)
        .test_value();
    target_object.visibility = clonk_engine::VIS_ALL;
    target_object.category |= C4D_IGNORE_FOW;
    main_assert_eq!(
        app.hud_message_drawability(&message, &app.graphics.active_viewport_projections()) =>
        HudMessageDrawability::Drawable,
        "C4D_IgnoreFoW bypasses only the FoW predicate"
    );
}

#[test]
fn hidden_startup_irc_warning_connects_immediately() {
    let _lock = env_lock().lock();
    reset_cached_app_paths();
    let user_data = tempdir();
    let (_guard, paths) = exact_loader_test_paths(user_data.path(), None);
    let (address, server) = spawn_loopback_irc_server();
    persist_config_value(&paths, "Startup", "HideMsgIRCDangerous", "1").test_value();
    persist_config_value(&paths, "Network", "MasterServerSignUp", "0").test_value();
    persist_config_value(&paths, "IRC", "Server2", &address).test_value();
    persist_config_value(&paths, "IRC", "Nick", "HiddenNick").test_value();
    persist_config_value(&paths, "IRC", "RealName", "Hidden Name").test_value();
    persist_config_value(&paths, "IRC", "Channel", "#hidden").test_value();
    let mut app = new_menu_app_with_paths(640, 480, &paths);
    install_classic_test_assets(&mut app);
    app.open_network_game_dialog();
    let mut login = app.startup_network_dialog.test_ref().chat_login();
    main_assert_eq!(login.server => address);
    main_assert_eq!(login.nick => "HiddenNick");
    main_assert_eq!(login.real_name => "Hidden Name");
    main_assert_eq!(login.channel => "#hidden");
    login.password = "not-persisted".into();
    let dialog_count = app.message_dialogs.len();

    app.request_startup_irc_connection(login).test_value();
    main_assert_eq!(app.message_dialogs.len() => dialog_count);
    let client = app.startup_irc_client.test_ref();
    main_assert!(matches!(client.recv_event_timeout(Duration::from_secs(2)), Ok(clonk_network::IrcClientEvent::Connected)));
    let persisted = Config::load(paths.config_file()).test_value();
    main_assert_eq!(persisted.get_in(Some("IRC"), "Nick") => Some("HiddenNick"));
    main_assert_eq!(persisted.get_in(Some("IRC"), "Password") => None);
    drop(app);
    server.test_join();
    reset_cached_app_paths();
}

#[test]
fn active_irc_runtime_hud_chat_button_opens_ui_without_disconnecting_transport() {
    let (address, server) = spawn_loopback_irc_server();
    let handle = clonk_network::IrcClientHandle::connect_with_timeout(
        clonk_network::IrcConnectConfig::new(
            address.clone(),
            b"Clonker".to_vec(),
            b"Clonker".to_vec(),
        ),
        Duration::from_secs(2),
    )
    .test_value();
    main_assert!(matches!(handle.recv_event_timeout(Duration::from_secs(2)), Ok(clonk_network::IrcClientEvent::Connected)));

    let mut app = new_classic_running_sandbox_app();
    app.startup_irc_server = address;
    app.startup_irc_client = Some(handle);
    render_mouse_test_app(&mut app);
    let owner = app.local_owner;
    let chat = viewport_button_point(&app, owner, clonk_frontend::hud::ViewportButton::Chat);
    main_assert_eq!(app.ingame_viewport_region(owner, chat) => Some(IngameViewportRegion::ViewportButton(clonk_frontend::hud::ViewportButton::Chat,)));

    physical_left_click_with_modifiers(
        &mut app,
        chat,
        ModifiersState::empty(),
        ModifiersState::empty(),
    );
    main_assert!(app.external_irc_dialog_visible);
    main_assert!(app.startup_irc_client_active());
    app.hide_external_irc_dialog();
    main_assert!(app.startup_irc_client_active());

    drop(app);
    server.test_join();
}

#[test]
fn standalone_irc_validation_disconnect_and_window_close_use_classic_modal_ownership() {
    use clonk_frontend::message_dialog::{
        MessageDialogButton, MessageDialogIcon, MessageDialogResult,
    };
    use clonk_frontend::startup_netdlg::{
        NetDlgAction, NetDlgChatConnectionState, NetDlgChatPage, NetDlgChatSnapshot,
        NetDlgChatValidationError,
    };

    let mut app = new_real_classic_menu_app(640, 480);
    app.startup_network_dialog = Some(app.new_network_dialog_controller());
    let embedded_controller_ptr = app.startup_network_dialog.as_ref().map(std::ptr::from_ref);
    app.show_external_irc_dialog().test_value();
    main_assert_ne!(
        app.external_irc_dialog.as_ref().map(std::ptr::from_ref) =>
        embedded_controller_ptr,
        "C4ChatDlg must own a distinct C4ChatControl from StartupNetDlg"
    );
    app.process_network_dialog_actions(vec![NetDlgAction::ChatValidationFailed(
        NetDlgChatValidationError::InvalidPassword,
    )])
    .test_value();
    let validation = app.message_dialogs.last().test_value();
    main_assert_eq!(validation.state.icon() => MessageDialogIcon::ERROR);
    main_assert!(validation.state.message().contains("31"));
    app.finish_message_dialog(MessageDialogResult::Ok)
        .test_value();

    app.external_irc_dialog
        .test_mut()
        .sync_chat_snapshot(NetDlgChatSnapshot {
            connection_state: NetDlgChatConnectionState::Connected,
            server: "irc.example.test".into(),
            nick: "Clonker".into(),
            ..NetDlgChatSnapshot::default()
        });
    main_assert_eq!(app.external_irc_dialog.as_ref().unwrap().chat_page() => NetDlgChatPage::Chats);
    app.process_network_dialog_actions(vec![NetDlgAction::ChatDisconnectConfirmationRequested])
        .test_value();
    let confirmation = app.message_dialogs.last().test_value();
    main_assert!(matches!(confirmation.continuation, MessageDialogContinuation::StartupIrcDisconnectConfirm));
    main_assert_eq!(confirmation.state.caption() => "Chat");
    main_assert_eq!(confirmation.state.button_label(MessageDialogButton::Cancel) => "Abort");
    app.finish_message_dialog(MessageDialogResult::Cancel)
        .test_value();
    main_assert_eq!(app.external_irc_dialog.as_ref().unwrap().chat_page() => NetDlgChatPage::Chats);

    app.process_network_dialog_actions(vec![NetDlgAction::ChatDisconnectConfirmationRequested])
        .test_value();
    app.finish_message_dialog(MessageDialogResult::Ok)
        .test_value();
    main_assert_eq!(app.external_irc_dialog.as_ref().unwrap().chat_page() => NetDlgChatPage::Login);
    main_assert!(app.external_irc_dialog_visible);

    app.process_network_dialog_actions(vec![NetDlgAction::ChatDialogCloseRequested])
        .test_value();
    main_assert!(!app.external_irc_dialog_visible);
    main_assert!(app.external_irc_dialog.is_none());
    main_assert_eq!(app.startup_network_dialog.as_ref().map(std::ptr::from_ref) => embedded_controller_ptr);

    app.show_external_irc_dialog().test_value();
    app.external_irc_dialog
        .as_mut()
        .test_value()
        .force_chat_mode_and_focus();
    let original_nick = app
        .external_irc_dialog
        .as_ref()
        .test_value()
        .chat_login()
        .nick;
    app.test_key(VirtualKeyCode::ContextMenu, ElementState::Pressed);
    main_assert!(app.context_menu.is_some());
    app.test_text_input('x');
    main_assert_eq!(app.external_irc_dialog.as_ref().unwrap().chat_login().nick => original_nick);
    app.test_key(VirtualKeyCode::Escape, ElementState::Pressed);
    app.test_key(VirtualKeyCode::Escape, ElementState::Released);
    main_assert!(app.context_menu.is_none());
    main_assert!(app.external_irc_dialog_visible);
    app.test_text_input('x');
    main_assert_eq!(app.external_irc_dialog.as_ref().unwrap().chat_login().nick => format!("{original_nick}x"));
    app.hide_external_irc_dialog();
    app.show_external_irc_dialog().test_value();
    main_assert_eq!(app.external_irc_dialog.as_ref().unwrap().chat_login().nick => original_nick, "closing standalone chat discards its unsent edit state");

    let initial_bounds = app
        .external_irc_dialog
        .as_ref()
        .and_then(|dialog| dialog.chat_bounds_override())
        .test_value();
    let embedded_pointer = GuiPoint::new(17.0, 19.0);
    app.startup_network_dialog
        .as_mut()
        .test_value()
        .set_pointer_position(Some(embedded_pointer));
    let drag_start = GuiPoint::new(
        (initial_bounds.x + 20) as f32,
        (initial_bounds.y + 8) as f32,
    );
    app.test_touch(TouchPhase::Started, drag_start);
    app.test_touch(TouchPhase::Cancelled, drag_start);
    app.test_touch(
        TouchPhase::Moved,
        GuiPoint::new(drag_start.x + 50.0, drag_start.y + 40.0),
    );
    main_assert_eq!(
        app.external_irc_dialog
            .as_ref()
            .and_then(|dialog| dialog.chat_bounds_override()) =>
        Some(initial_bounds),
        "touch cancellation clears the standalone caption capture"
    );
    main_assert_eq!(
        app.startup_network_dialog
            .as_ref()
            .and_then(|dialog| dialog.pointer_position()) =>
        Some(embedded_pointer),
        "standalone cancellation must not mutate the embedded startup sheet"
    );
}

#[test]
fn message_control_authenticates_players_and_applies_running_visibility() {
    let mut app = new_state_only_running_sandbox_app();
    install_message_fixture(&mut app);

    let spoofed =
        app.execute_message_control(message_control(MESSAGE_TYPE_NORMAL, 7, -1, b"spoofed", 8));
    main_assert!(spoofed.rejected);
    main_assert!(app.message_board.log_history.is_empty());

    let normal =
        app.execute_message_control(message_control(MESSAGE_TYPE_NORMAL, 7, -1, b"hello", 7));
    main_assert!(normal.displayed);
    main_assert_eq!(app.message_board_line().as_deref() => Some("<c 123456><Sender> hello"));

    let queued =
        app.execute_message_control(message_control(MESSAGE_TYPE_NORMAL, 7, -1, b"second", 7));
    main_assert!(queued.displayed);
    main_assert_eq!(app.message_board_line().as_deref() => Some("<c 123456><Sender> second"));
    main_assert_eq!(app.message_board.log_history.iter().map(String::as_str).collect::<Vec<_>>() => vec!["<c 123456><Sender> hello", "<c 123456><Sender> second"]);
    app.scroll_message_board(true);
    main_assert_eq!(app.message_board_line().as_deref() => Some("<c 123456><Sender> hello"));
    app.scroll_message_board(false);
    main_assert_eq!(app.message_board_line().as_deref() => Some("<c 123456><Sender> second"));

    app.clear_message_board_log();
    let missing_player = app.execute_message_control(message_control(
        MESSAGE_TYPE_NORMAL,
        999,
        -1,
        b"client message",
        7,
    ));
    main_assert!(missing_player.displayed);
    main_assert_eq!(latest_message_board_logical_entry(&app).as_deref() => Some("<Remote> client message"),);

    app.clear_message_board_log();
    app.engine
        .set_hostility(7, app.local_owner, true)
        .test_value();
    let hostile_team =
        app.execute_message_control(message_control(MESSAGE_TYPE_TEAM, 7, -1, b"hidden", 7));
    main_assert!(!hostile_team.displayed);
    main_assert!(app.message_board.log_history.is_empty());
    app.engine
        .set_hostility(7, app.local_owner, false)
        .test_value();
    main_assert!(app.execute_message_control(message_control(MESSAGE_TYPE_TEAM, 7, -1, b"allied", 7,)).displayed);

    app.clear_message_board_log();
    main_assert!(app.execute_message_control(message_control(MESSAGE_TYPE_PRIVATE, 7, app.local_owner, b"local", 7,)).displayed);
    app.clear_message_board_log();
    main_assert!(!app.execute_message_control(message_control(MESSAGE_TYPE_PRIVATE, 7, 99, b"hidden", 7,)).displayed);
    main_assert!(app.message_board.log_history.is_empty());
}

#[test]
fn running_chat_classifies_private_and_say_and_submits_normal_controls() {
    let mut app = new_running_sandbox_app();
    install_message_fixture(&mut app);
    app.snapshot = app.engine.snapshot();

    let private = parse_running_message_control(
        "/private Sender secret",
        app.local_owner,
        false,
        &app.snapshot,
    )
    .expect("parse private")
    .test_value();
    main_assert_eq!(private.message_type => MESSAGE_TYPE_PRIVATE);
    main_assert_eq!(private.to_player => 7);
    main_assert_eq!(private.message.as_bytes() => b"secret");

    let say = parse_running_message_control("\"hello", app.local_owner, false, &app.snapshot)
        .expect("parse say")
        .test_value();
    main_assert_eq!(say.message_type => MESSAGE_TYPE_SAY);
    main_assert_eq!(say.message.as_bytes() => b"\"hello\"");

    let (network, _events, mut commands) = NetworkManager::test_stub_with_commands_for_client_id(0);
    app.network = Some(network);
    app.test_key(VirtualKeyCode::Enter, ElementState::Pressed);
    for character in "hello".chars() {
        app.test_text_input(character);
    }
    app.test_key(VirtualKeyCode::Enter, ElementState::Pressed);
    main_assert!(app.running_chat.is_none());
    main_assert_eq!(
        commands.take_submitted_messages() =>
        vec![MessageControlData {
            message_type: MESSAGE_TYPE_NORMAL,
            player: app.local_owner,
            to_player: -1,
            message: clonk_engine::LegacyCString::from_bytes(b"hello".to_vec())
                .expect("fixture is NUL-free"),
            by_client: 0,
        }]
    );

    app.test_key(VirtualKeyCode::Enter, ElementState::Pressed);
    for character in "Sen".chars() {
        app.test_text_input(character);
    }
    app.test_key(VirtualKeyCode::Tab, ElementState::Pressed);
    main_assert_eq!(app.running_chat_text() => Some("Sender"));
    let sound_enabled = app.audio.test_ref().options.sound_enabled;
    app.keyboard_modifiers = ModifiersState::CONTROL;
    app.test_key(VirtualKeyCode::F3, ElementState::Pressed);
    main_assert_eq!(app.audio.as_ref().expect("sandbox audio context").options.sound_enabled => sound_enabled);
    app.keyboard_modifiers = ModifiersState::empty();
}

#[test]
fn running_help_clear_and_case_sensitive_unknown() {
    let mut app = new_state_only_running_sandbox_app();
    app.enqueue_control_message_board_line("old line".to_string());

    app.process_running_chat_text("/help");
    let help_entries = message_board_logical_entries(&app);
    main_assert!(help_entries.iter().any(|line| line.contains("Commands available during game")));
    main_assert!(help_entries.iter().any(|line| line.starts_with("/clear - ")));
    main_assert!(help_entries.iter().any(|line| line.starts_with("/fast [x] - ")));
    main_assert!(help_entries.iter().any(|line| line.starts_with("/slow - ")));
    main_assert!(help_entries.iter().all(|line| !line.contains("Unknown command")));

    app.process_running_chat_text("/clear");
    main_assert!(app.message_board.log_history.is_empty());
    main_assert!(app.message_board.current_line().is_none());

    app.process_running_chat_text("/Clear");
    main_assert!(latest_message_board_logical_entry(&app).as_deref().is_some_and(|line| line.contains("Unknown command") && line.contains("Clear")));
}

#[test]
fn chart_elevation_keeps_visual_order_separate_from_reactivated_chat_input() {
    let mut app = new_running_sandbox_app();
    app.start_running_chat(RunningChatMode::All);
    app.toggle_network_chart();
    app.activate_runtime_default_dialog(RuntimeDefaultDialog::NetworkChart);
    main_assert!(app.network_chart_elevated);
    main_assert!(app.network_chart_is_active_dialog());
    main_assert!(!app.running_chat_active());

    app.set_running_chat_active(true);
    main_assert!(app.running_chat_active());
    main_assert!(app.network_chart_renders_elevated());
    main_assert!(!app.network_chart_is_active_dialog());

    let resources = app.assets.network_chart_resources().test_value();
    let preferred = scoreboard_preferred_rect(
        app.graphics
            .preferred_dialog_rect(app.mouse_control.then_some(app.local_owner)),
    );
    let chart = app
        .network_chart_dialog
        .test_ref()
        .layout(preferred, resources)
        .chart;
    let chart_point = GuiPoint::new(
        chart.x.saturating_add(chart.w / 2) as f32,
        chart.y.saturating_add(chart.h / 2) as f32,
    );
    app.running_pointer_position = Some(chart_point);
    app.handle_mouse_button_classified(ElementState::Pressed, false)
        .test_value();
    main_assert!(app.network_chart_is_active_dialog());
    main_assert!(!app.running_chat_active());
    app.handle_mouse_button_classified(ElementState::Released, false)
        .test_value();

    app.set_running_chat_active(true);
    app.toggle_network_chart();
    main_assert!(app.network_chart_dialog.is_none());
    main_assert!(app.running_chat_active(), "closing an inactive elevated chart must not overwrite reactivated chat ownership");
}

#[test]
fn chart_restores_projected_successor_after_active_underlay_is_removed() {
    let mut app = new_running_sandbox_app();
    for caption in ["First", "Second"] {
        app.push_message_dialog(
            clonk_frontend::message_dialog::MessageDialogState::new(
                caption,
                caption,
                clonk_frontend::message_dialog::MessageDialogButtons::OK,
                clonk_frontend::message_dialog::MessageDialogIcon::None,
                clonk_frontend::message_dialog::MessageDialogSize::Small,
                false,
            ),
            MessageDialogContinuation::None,
        )
        .test_value();
    }
    let successor = app.message_dialogs[0].running_stack_id;
    let removed = app.message_dialogs[1].running_stack_id;

    app.toggle_network_chart();
    app.activate_runtime_default_dialog(RuntimeDefaultDialog::NetworkChart);
    main_assert!(app.network_chart_elevated_owns_input());

    app.message_dialog_active_index = Some(1);
    app.activate_running_dialog(RunningDialogStackEntry::Message(removed));
    main_assert!(app.network_chart_renders_elevated());
    main_assert!(!app.network_chart_is_active_dialog());

    let (_, was_active) = app.remove_message_dialog_at(1).test_value();
    main_assert!(was_active);
    main_assert_eq!(app.running_active_dialog => Some(RunningDialogStackEntry::Message(successor)));
    main_assert!(app.network_chart_elevated_owns_input());
    main_assert!(app.network_chart_is_active_dialog());
    main_assert_eq!(app.active_message_dialog_index() => None);

    app.toggle_network_chart();
    main_assert!(app.network_chart_dialog.is_none());
    main_assert_eq!(app.message_dialog_active_index => Some(0));
    main_assert_eq!(app.active_message_dialog_index() => Some(0));
}

#[test]
fn chart_hide_restores_projected_message_instead_of_inactive_chat() {
    let mut app = new_running_sandbox_app();
    app.push_message_dialog(
        clonk_frontend::message_dialog::MessageDialogState::regular_ok(
            "Notice",
            "Message under chart",
            clonk_frontend::message_dialog::MessageDialogIcon::NOTIFY,
        ),
        MessageDialogContinuation::None,
    )
    .test_value();
    let message = app.message_dialogs[0].running_stack_id;
    app.start_running_chat(RunningChatMode::All);
    app.set_running_chat_active(false);
    app.message_dialog_active_index = Some(0);
    app.activate_running_dialog(RunningDialogStackEntry::Message(message));

    app.toggle_network_chart();
    app.activate_runtime_default_dialog(RuntimeDefaultDialog::NetworkChart);
    main_assert!(app.network_chart_elevated_owns_input());
    main_assert_eq!(app.running_active_dialog => Some(RunningDialogStackEntry::Message(message)));

    app.toggle_network_chart();
    main_assert!(app.network_chart_dialog.is_none());
    main_assert_eq!(app.message_dialog_active_index => Some(0));
    main_assert!(!app.running_chat_active());
}

#[test]
fn running_chat_multiline_paste_submits_lines_and_retains_final_text() {
    let mut app = new_running_sandbox_app();
    install_message_fixture(&mut app);
    app.snapshot = app.engine.snapshot();
    let (network, _events, mut commands) = NetworkManager::test_stub_with_commands_for_client_id(0);
    app.network = Some(network);
    app.test_key(VirtualKeyCode::Enter, ElementState::Pressed);

    let layout = app.game_option_input_layout().test_value();
    let fonts = app.assets.clonk_fonts.clone().test_value();
    let actions = app
        .running_chat_controller_mut()
        .test_value()
        .apply_context_command(
            InputDialogContextCommand::Paste,
            Some("first\nsecond"),
            &layout,
            &fonts.text,
        );
    app.finish_game_option_input_dialog_actions(actions)
        .test_value();
    main_assert_eq!(app.running_chat_text() => Some("second"));
    main_assert_eq!(
        commands.take_submitted_messages() =>
        vec![MessageControlData {
            message_type: MESSAGE_TYPE_NORMAL,
            player: app.local_owner,
            to_player: -1,
            message: clonk_engine::LegacyCString::from_bytes(b"first".to_vec())
                .expect("fixture is NUL-free"),
            by_client: 0,
        }]
    );
    main_assert_eq!(app.message_input_history.front().map(String::as_str) => Some("first"));

    app.test_key(VirtualKeyCode::Enter, ElementState::Pressed);
    main_assert!(app.running_chat.is_none());
    main_assert_eq!(
        commands.take_submitted_messages() =>
        vec![MessageControlData {
            message_type: MESSAGE_TYPE_NORMAL,
            player: app.local_owner,
            to_player: -1,
            message: clonk_engine::LegacyCString::from_bytes(b"second".to_vec())
                .expect("fixture is NUL-free"),
            by_client: 0,
        }]
    );
}

#[test]
fn running_chat_history_scrolls_replacement_and_preserves_offset_when_cleared() {
    let mut app = new_running_sandbox_app();
    app.message_input_history.push_front("history".to_string());
    app.start_running_chat(RunningChatMode::All);
    for character in "long text ".repeat(100).chars() {
        app.test_text_input(character);
    }
    let long_scroll = app
        .running_chat_controller()
        .test_value()
        .horizontal_scroll();
    main_assert!(long_scroll > 0);

    app.test_key(VirtualKeyCode::ArrowUp, ElementState::Pressed);
    main_assert_eq!(app.running_chat_text() => Some("history"));
    let history_scroll = app
        .running_chat_controller()
        .test_value()
        .horizontal_scroll();
    main_assert!(history_scroll < long_scroll);
    main_assert_eq!(app.running_chat_controller().expect("history chat controller").selection() => Some((0, "history".len())));

    app.test_key(VirtualKeyCode::ArrowDown, ElementState::Pressed);
    main_assert_eq!(app.running_chat_text() => Some(""));
    main_assert_eq!(app.running_chat_controller().expect("cleared chat controller").horizontal_scroll() => history_scroll);
}

#[test]
fn running_chat_close_forgets_releases_swallowed_by_the_modal() {
    let mut app = new_running_sandbox_app();
    app.start_running_chat(RunningChatMode::All);
    app.pressed_engine_keys.insert(VirtualKeyCode::KeyA);
    app.pressed_engine_keys.insert(VirtualKeyCode::Tab);
    app.scoreboard_tab_raw_pressed = true;

    app.test_key(VirtualKeyCode::KeyA, ElementState::Released);
    app.test_key(VirtualKeyCode::Tab, ElementState::Released);
    // `C4Game::DoKeyboardInput` clears its `PressedKeys` entry on every
    // key-up ahead of the scope decision (C4Game.cpp:2143-2155), so a release
    // the chat modal swallows still drops the physical latch. Only
    // `scoreboard_tab_raw_pressed`, which has no oracle counterpart and is
    // maintained inside the scoreboard route, survives to the close below.
    main_assert!(!app.pressed_engine_keys.contains(&VirtualKeyCode::KeyA));
    main_assert!(app.scoreboard_tab_raw_pressed);

    app.test_key(VirtualKeyCode::Escape, ElementState::Pressed);
    main_assert!(app.pressed_engine_keys.is_empty());
    main_assert!(!app.scoreboard_tab_raw_pressed);

    app.test_key(VirtualKeyCode::KeyA, ElementState::Pressed);
    main_assert!(app.pressed_engine_keys.contains(&VirtualKeyCode::KeyA));
}

#[test]
fn running_chat_exclusive_scope_blocks_rebound_tab_player_control() {
    let mut app = new_running_sandbox_app();
    app.bindings
        .rebind(ControlBindingId::Left, VirtualKeyCode::Tab);
    app.start_running_chat(RunningChatMode::All);

    for context_open in [false, true] {
        if context_open {
            app.test_key(VirtualKeyCode::ContextMenu, ElementState::Pressed);
            app.test_key(VirtualKeyCode::ContextMenu, ElementState::Released);
            main_assert!(app.context_menu.is_some());
        }
        app.engine
            .test_player_mut(app.local_owner)
            .control
            .pressed_coms = 0;
        app.test_key(VirtualKeyCode::Tab, ElementState::Pressed);
        app.test_key(VirtualKeyCode::Tab, ElementState::Released);
        main_assert_eq!(app.engine.player(app.local_owner).expect("local sandbox player").control.pressed_coms & (1 << clonk_engine::COM_LEFT) => 0);
        if context_open {
            app.test_key(VirtualKeyCode::Escape, ElementState::Pressed);
            app.test_key(VirtualKeyCode::Escape, ElementState::Released);
        }
    }
}

#[test]
fn running_chat_shared_screen_pointer_lifecycle_matches_classic_mouse() {
    let notice = || {
        clonk_frontend::message_dialog::MessageDialogState::regular_ok(
            "Lower notice",
            "Message",
            clonk_frontend::message_dialog::MessageDialogIcon::NOTIFY,
        )
    };

    let mut app = new_classic_running_sandbox_app();
    app.start_running_chat(RunningChatMode::All);
    for character in "alpha beta".chars() {
        app.test_text_input(character);
    }
    app.test_modifiers(ModifiersState::ALT);
    main_assert!(!app.handle_game_option_input_dialog_key(VirtualKeyCode::F11, ElementState::Pressed).expect("non-character Alt key has no input-dialog mnemonic"));
    main_assert!(!app
        .handle_game_option_input_dialog_key(VirtualKeyCode::F11, ElementState::Released)
        .expect("non-character Alt release is also down-only fallthrough"));
    app.test_key(VirtualKeyCode::KeyC, ElementState::Pressed);
    main_assert!(app.external_irc_dialog_visible);
    main_assert!(app.running_chat.is_none());
    app.test_key(VirtualKeyCode::KeyC, ElementState::Released);
    app.test_key(VirtualKeyCode::KeyC, ElementState::Pressed);
    app.test_key(VirtualKeyCode::KeyC, ElementState::Released);
    main_assert!(!app.external_irc_dialog_visible);
    app.test_modifiers(ModifiersState::empty());
    app.start_running_chat(RunningChatMode::All);
    for character in "alpha beta".chars() {
        app.test_text_input(character);
    }
    app.push_message_dialog(notice(), MessageDialogContinuation::None)
        .test_value();
    main_assert_eq!(app.game_option_input_activity() => (true, true));
    let message_layout = app.top_message_dialog_layout().test_value();
    let message_button = message_layout.buttons[0].rect;
    let message_point = PhysicalPosition::new(
        f64::from(message_button.x + message_button.w / 2),
        f64::from(message_button.y + message_button.h / 2),
    );
    app.test_cursor(message_point);
    main_assert!(app.message_dialogs[0].state.has_pointer_hover());

    let chat_layout = app.game_option_input_layout().test_value();
    let chat_point = PhysicalPosition::new(
        f64::from(chat_layout.edit.x + 5),
        f64::from(chat_layout.edit.y + chat_layout.edit.h / 2),
    );
    app.test_cursor(chat_point);
    app.test_left_button(ElementState::Pressed);
    app.test_left_button(ElementState::Released);
    app.test_cursor(message_point);
    app.test_key(VirtualKeyCode::ContextMenu, ElementState::Pressed);
    app.test_key(VirtualKeyCode::ContextMenu, ElementState::Released);
    let context_row = app.context_menu.test_ref().layout().panels[0].rows[0].rect;
    app.test_cursor(PhysicalPosition::new(
        f64::from(context_row.x + 1),
        f64::from(context_row.y + 1),
    ));
    main_assert!(!app.message_dialogs[0].state.has_pointer_hover());
    app.test_key(VirtualKeyCode::Escape, ElementState::Pressed);
    app.test_key(VirtualKeyCode::Escape, ElementState::Released);

    app.test_cursor(chat_point);
    app.test_left_button(ElementState::Pressed);
    main_assert!(app.running_chat_controller().expect("chat controller during drag").has_positional_pointer_drag());
    let caret_before_context_drag = app.running_chat_controller().test_value().caret();
    app.test_key(VirtualKeyCode::ContextMenu, ElementState::Pressed);
    app.test_key(VirtualKeyCode::ContextMenu, ElementState::Released);
    let context_panel = app.context_menu.test_ref().layout().panels[0].bounds;
    app.test_cursor(PhysicalPosition::new(
        f64::from(context_panel.x + context_panel.w - 2),
        f64::from(context_panel.y + 1),
    ));
    main_assert_ne!(app.running_chat_controller().expect("chat controller after context drag").caret() => caret_before_context_drag);
    main_assert!(app.running_chat_controller().expect("chat drag remains retained until up").has_positional_pointer_drag());
    app.test_left_button(ElementState::Released);
    main_assert!(!app.running_chat_controller().expect("chat remains after context drag release").has_pointer_capture());
    if app.context_menu.is_some() {
        app.test_key(VirtualKeyCode::Escape, ElementState::Pressed);
        app.test_key(VirtualKeyCode::Escape, ElementState::Released);
    }

    let lower_point = PhysicalPosition::new(
        f64::from(message_layout.bounds.x + 5),
        f64::from(message_layout.bounds.y + 5),
    );
    app.test_cursor(lower_point);
    app.test_left_button(ElementState::Pressed);
    app.test_left_button(ElementState::Released);
    main_assert_eq!(app.game_option_input_activity() => (false, true));

    let start = GuiPoint::new(
        (chat_layout.edit.x + 5) as f32,
        (chat_layout.edit.y + chat_layout.edit.h / 2) as f32,
    );
    let end = GuiPoint::new(
        (chat_layout.edit.x + 90) as f32,
        (chat_layout.edit.y + chat_layout.edit.h / 2) as f32,
    );
    app.test_touch(TouchPhase::Started, start);
    app.test_touch(TouchPhase::Ended, end);
    main_assert!(app.running_chat_controller().and_then(InputDialogController::selected_text).is_some_and(|text| !text.is_empty()));
    main_assert!(!app.running_chat_controller().expect("chat remains open").has_pointer_capture());

    let mut cursor_exit = new_running_sandbox_app();
    let checkbox_dialog = notice().with_checkbox("&Remember", false);
    cursor_exit
        .push_message_dialog(checkbox_dialog, MessageDialogContinuation::None)
        .test_value();
    let checkbox_layout = cursor_exit.top_message_dialog_layout().test_value();
    let checkbox = checkbox_layout.checkbox.test_value().square;
    let checkbox_point = PhysicalPosition::new(
        f64::from(checkbox.x + checkbox.w / 2),
        f64::from(checkbox.y + checkbox.h / 2),
    );
    cursor_exit.test_cursor(checkbox_point);
    main_assert!(cursor_exit.message_dialogs[0].state.has_pointer_hover());
    cursor_exit.pointer_left().test_value();
    main_assert!(cursor_exit.running_pointer_position.is_none());
    main_assert!(!cursor_exit.message_dialogs[0].state.has_pointer_hover());
    cursor_exit.test_left_button(ElementState::Released);
    main_assert_eq!(cursor_exit.message_dialogs[0].state.checkbox_checked() => Some(false));

    let button = checkbox_layout.buttons[0].rect;
    cursor_exit.test_cursor(PhysicalPosition::new(
        f64::from(button.x + button.w / 2),
        f64::from(button.y + button.h / 2),
    ));
    cursor_exit.test_left_button(ElementState::Pressed);
    main_assert_eq!(cursor_exit.message_dialog_pointer_capture_index => Some(0));
    cursor_exit.resize(360, 240).test_value();
    main_assert_eq!(cursor_exit.message_dialog_pointer_capture_index => None);
    main_assert!(!cursor_exit.message_dialogs[0].state.has_pointer_capture());
    main_assert!(!cursor_exit.message_dialogs[0].state.has_pointer_hover());

    let mut menu = new_menu_app(320, 200);
    let stationary_dialog = notice();
    let fonts = menu.assets.clonk_fonts.clone().test_value();
    let stationary_layout = stationary_dialog.layout(320, 200, &fonts.text);
    let button = stationary_layout.buttons[0].rect;
    let stationary_point = PhysicalPosition::new(
        f64::from(button.x + button.w / 2),
        f64::from(button.y + button.h / 2),
    );
    menu.test_cursor(stationary_point);
    menu.push_message_dialog(stationary_dialog, MessageDialogContinuation::None)
        .test_value();
    menu.test_left_button(ElementState::Pressed);
    menu.test_left_button(ElementState::Released);
    main_assert!(menu.message_dialogs.is_empty());

    menu.open_game_option_input_dialog(GameOptionInputDialogRequest {
        kind: GameOptionInputKind::Password,
        message: "Password",
        caption: "Password",
        icon: clonk_frontend::game_option_buttons::GameOptionIcon::Locked,
        max_text: 31,
        initial_text: "alpha beta".to_string(),
        chat_layout: false,
    })
    .test_value();
    let input_layout = menu.game_option_input_layout().test_value();
    let input_start = PhysicalPosition::new(
        f64::from(input_layout.edit.x + 5),
        f64::from(input_layout.edit.y + input_layout.edit.h / 2),
    );
    menu.test_cursor(input_start);
    menu.test_left_button(ElementState::Pressed);
    main_assert!(menu.game_option_input_dialog.as_ref().expect("regular input dialog").controller.has_positional_pointer_drag());
    menu.test_key(VirtualKeyCode::ContextMenu, ElementState::Pressed);
    menu.test_key(VirtualKeyCode::ContextMenu, ElementState::Released);
    let input_context = menu.context_menu.test_ref().layout().panels[0].bounds;
    menu.test_cursor(PhysicalPosition::new(
        f64::from(input_context.x + input_context.w - 2),
        f64::from(input_context.y + 1),
    ));
    menu.test_left_button(ElementState::Released);
    main_assert!(!menu.game_option_input_dialog.as_ref().expect("regular input remains open").controller.has_pointer_capture());
}

#[test]
fn message_board_history_keeps_append_time_width_across_upper_board_modes() {
    let mut app = new_classic_running_sandbox_app();
    let full_message = "X".repeat(200);
    app.enqueue_control_message_board_line(full_message.clone());
    let full_lines = app.message_board.log_history.len();
    main_assert!(full_lines > 1, "Full mode stores wrapped physical lines");
    main_assert_ne!(app.message_board_line().as_deref() => Some(full_message.as_str()));
    let visible_len =
        clonk_script::c4_string_byte_len(app.message_board_line().as_deref().test_value());
    app.message_board.empty = false;
    app.message_board.fader = 0;
    app.message_board.delay = -1;
    app.advance_message_board_overlay();
    main_assert_eq!(app.message_board.delay => visible_len as i32 - 1, "the native delay uses the selected physical line's C4 byte length");
    let full_history = app.message_board.log_history.clone();

    app.apply_ingame_menu_action(MenuAction::Display(DisplayToggle::UpperBoard))
        .test_value();
    app.apply_ingame_menu_action(MenuAction::Display(DisplayToggle::UpperBoard))
        .test_value();
    main_assert_eq!(app.message_board.log_history => full_history, "reinitializing LBWidth does not reflow existing native log lines");

    let before_mini = app.message_board.log_history.len();
    app.enqueue_control_message_board_line("Y".repeat(200));
    let mini_lines = app.message_board.log_history.len() - before_mini;
    main_assert!(mini_lines > full_lines, "future Mini messages use the newly shortened log-buffer width");
}

#[test]
fn runtime_pause_halts_offline_ticks_and_draws_the_exact_hold_message() {
    let mut app = new_classic_running_sandbox_app();
    app.test_modifiers(ModifiersState::SUPER);
    let frame_before_pause = app.engine.frame();
    app.test_key(VirtualKeyCode::Pause, ElementState::Pressed);
    main_assert_ne!(app.offline_halt_count => 0);
    for _ in 0..3 {
        app.test_update();
    }
    main_assert_eq!(app.engine.frame() => frame_before_pause);
    let mut schedule = frame_schedule_for_mode(
        app.mode,
        app.engine.game_tick_delay_ms(),
        app.engine.game_tick_delay_revision(),
        app.max_refresh_delay_ms,
    );
    let mut accumulator = schedule.simulation_interval;
    let halted_pass =
        advance_simulation_pass(&mut app, &mut schedule, &mut accumulator).test_value();
    main_assert!(halted_pass.did_update);
    main_assert_eq!(halted_pass.executed_frames => 0);
    main_assert!(!halted_pass.skip_redraw);
    main_assert_eq!(app.engine.frame() => frame_before_pause);

    let mut frame = vec![0_u8; app.graphics.surface().pixels().len()];
    app.render_ordered_native_base(&mut frame).test_value();
    let hold_messages = app
        .pending_native_presentation
        .test_ref()
        .batches
        .iter()
        .flat_map(|batch| batch.text.iter())
        .filter(|command| command.text == "Pause")
        .collect::<Vec<_>>();
    let [hold] = hold_messages.as_slice() else {
        panic!("expected one fullscreen Pause hold message, got {hold_messages:?}");
    };
    let font = &app.assets.clonk_fonts.test_ref().text;
    main_assert_eq!(hold.role => clonk_graphics::clonk_font::ClonkFontRole::GuiText);
    main_assert_eq!((hold.x, hold.y) => (160, 100 - font.line_height * 2));
    main_assert_eq!(hold.color => [255, 255, 255, 255]);
    main_assert_eq!(hold.align => clonk_graphics::clonk_font::TextAlign::Center);

    app.test_key(VirtualKeyCode::Pause, ElementState::Released);
    main_assert_ne!(app.offline_halt_count => 0, "release does not toggle");
    app.test_key(VirtualKeyCode::Pause, ElementState::Pressed);
    main_assert_eq!(app.offline_halt_count => 0);
    app.test_update();
    main_assert_eq!(app.engine.frame() => frame_before_pause + 1);

    app.test_key(VirtualKeyCode::Pause, ElementState::Pressed);
    main_assert!(!app.take_exit_request());
    app.return_to_menu();
    main_assert_eq!(app.offline_halt_count => 0, "Game::Default clears the halt");
}

#[test]
fn c4script_log_lines_reach_the_running_message_board() {
    // C4LogSystem::GuiSink hands every logged line to the message board
    // while it is active, and LogNotify scrolls it into view
    // (src/C4Log.cpp:226-240; src/C4MessageBoard.cpp:327-347,354-366).
    // Content death/kill announcements — Hazard's Killstats, for one —
    // are ordinary C4Script Log() calls and must land there.
    use tracing_subscriber::fmt::writer::MakeWriter;

    let capture = clonk_logging::GameLogCapture::default();
    // The GuiSink layer projects each event before it reaches the capture,
    // so what the board drains is the message alone.
    let write_line = |text: &str| {
        let mut writer = capture.make_writer();
        std::io::Write::write_all(&mut writer, format!("{text}\n").as_bytes()).test_value();
    };

    let mut app = new_state_only_running_sandbox_app();
    app.game_log_capture = Some(capture.clone());
    write_line("Beta is dead.");
    app.drain_game_log_capture();
    main_assert_eq!(app.message_board_line().as_deref() => Some("Beta is dead."));

    // C4MessageBoard::AddLog returns before touching the log buffer while
    // the board is inactive, which outside a game it always is
    // (C4MessageBoard::Init/Clear).
    let mut menu = new_state_only_menu_app(320, 200);
    menu.game_log_capture = Some(capture.clone());
    write_line("The goal has been chosen: Alienhunt");
    menu.drain_game_log_capture();
    main_assert!(menu.message_board.log_history.is_empty());

    // The drained line is consumed, not replayed into the next game.
    app.drain_game_log_capture();
    main_assert_eq!(app.message_board.log_history.iter().map(String::as_str).collect::<Vec<_>>() => vec!["Player join: Player", "Beta is dead."]);
}
