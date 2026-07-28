// Contiguous slice 1 of 7 of the `command/tests` battery, spliced by
// `include!` from the parent module so every test id is unchanged.

    #[test]
    fn command_experience_gain_matches_every_cpp_bucket() {
        let expected = [
            1, 1, 1, 1, 1, 5, 1, 5, 1, 1, 0, 1, 1, 1, 1, 1, 1, 5, 0, 15, 1, 1, 1, 2, 5, 0, 2, 0, 1,
            1,
        ];
        for (raw, expected) in (1..=30).zip(expected) {
            let command = CommandId::from_raw(raw).expect("all C4CMD values are covered");
            assert_eq!(command.experience_gain(), expected, "{}", command.to_name());
        }
    }

    #[test]
    fn callback_complete_records_native_success_after_command_replacement() {
        let mut acquire = CommandStack::new();
        acquire
            .push_front(
                CommandRequest::new(CommandId::Acquire).with_data(CommandData::Text("WOOD".into())),
            )
            .expect("Acquire queues");
        let acquire_instance = acquire.entries.front().unwrap().instance_id;
        acquire.clear();
        assert!(!acquire
            .resolve_acquire_script_result(acquire_instance, AcquireScriptResult::Complete,));
        assert_eq!(acquire.take_successful_finishes(), [CommandId::Acquire]);

        let mut construct = CommandStack::new();
        construct
            .push_front(
                CommandRequest::new(CommandId::Construct)
                    .with_data(CommandData::Text("HUT1".into())),
            )
            .expect("Construct queues");
        let construct_instance = construct.entries.front().unwrap().instance_id;
        construct.clear();
        assert!(!construct
            .resolve_construct_script_result(construct_instance, AcquireScriptResult::Complete,));
        assert_eq!(construct.take_successful_finishes(), [CommandId::Construct]);
    }

    #[test]
    fn c4_angle_matches_cpp_axis_and_diagonal_boundaries() {
        // C4Math.cpp:33-45 computes atan2f first, then promotes the product
        // with the double literal 180.0 before truncating to int.
        assert_eq!(c4_angle(0, 0, 0, 10), 181);
        assert_eq!(c4_angle(0, 0, 0, -10), 359);
        assert_eq!(c4_angle(0, 0, 10, 10), 134);
    }

    #[test]
    fn c4_angle_inner_angle_matches_cpp_double_chain_exhaustively() {
        fn cpp_inner_angle(dx: i32, dy: i32) -> i32 {
            let radians = (dy as f32).atan2(dx as f32);
            (180.0_f64 * f64::from(radians) * f64::from(std::f32::consts::FRAC_1_PI)) as i32
        }

        for dx in 0..=512 {
            for dy in 0..=512 {
                let folded = c4_angle(0, 0, dx, dy);
                let actual_inner = if dx > 0 { folded - 90 } else { 270 - folded };
                assert_eq!(actual_inner, cpp_inner_angle(dx, dy), "dx={dx}, dy={dy}");
            }
        }
    }

    #[test]
    fn command_definition_data_preserves_equal_looking_c4id_payloads() {
        let packed_raw = u32::from_le_bytes(*b"1111");
        let packed = command_data_to_definition_id(&CommandData::Integer(packed_raw as i32))
            .expect("packed ID remains nonzero");
        let numeric = command_data_to_definition_id(&CommandData::Integer(1111))
            .expect("numeric ID remains nonzero");

        assert_eq!(clonk_script::c4_id_raw(&packed), packed_raw as usize);
        assert_eq!(clonk_script::c4_id_raw(&numeric), 1111);
        assert_ne!(packed, numeric);
        assert_eq!(clonk_script::c4_id_text(&packed), "1111");
        assert_eq!(clonk_script::c4_id_text(&numeric), "1111");
        assert_eq!(definition_id_to_c4id(&packed), Some(packed_raw as i32));
        assert_eq!(definition_id_to_c4id(&numeric), Some(1111));

        let definitions = HashMap::from([
            (
                packed.clone(),
                CommandDefinitionSnapshot {
                    value: 41,
                    ..CommandDefinitionSnapshot::default()
                },
            ),
            (
                numeric.clone(),
                CommandDefinitionSnapshot {
                    value: 42,
                    ..CommandDefinitionSnapshot::default()
                },
            ),
        ]);
        assert_eq!(
            definitions.get(&packed).map(|definition| definition.value),
            Some(41)
        );
        assert_eq!(
            definitions.get(&numeric).map(|definition| definition.value),
            Some(42)
        );
    }

    fn snapshot_with_id(id: u64) -> CommandObjectSnapshot {
        CommandObjectSnapshot {
            contact: 0,
            action_time: 0,
            shape_top: 0,
            shape_height: 20,
            shape: DefinitionRect::new(-8, -10, 16, 20),
            entrance: None,
            id: ObjectId::new(id),
            master_list_order: id as usize,
            definition_id: format!("DEF{id}"),
            position: Vector2::ZERO,
            fixed_position: FixedVec2::ZERO,
            fixed_velocity: FixedVec2::ZERO,
            move_to_range: 0,
            pathfinder: 0,
            no_transfer_zones: 0,
            no_push_enter: 0,
            status: ObjectStatus::Normal,
            destroyed: false,
            category: 0,
            container: None,
            action_name: "Idle".to_string(),
            // Generic command fixtures model a live action unless a test
            // explicitly opts into the built-in ActIdle slot.
            action_idle: false,
            action_disabled: false,
            action_target: None,
            action_target2: None,
            action_procedure: ActionProcedure::Undefined,
            command_direction: CommandDirection::Stop,
            construction: 0,
            direction: Direction::Left,
            physical: PhysicalInfo::default(),
            physical_deferred: false,
            owner: OWNER_NONE,
            controller: OWNER_NONE,
            base: OWNER_NONE,
            crew_member: false,
            selected: false,
            alive: true,
            need_energy: false,
            on_fire: false,
            contents: Vec::new(),
            commands: Vec::new(),
            line_connect: 0,
            ocf: ocf::AVAILABLE,
            entrance_status: false,
            collectible: false,
        }
    }

    fn pushed_request(operations: &[CommandOperation], id: CommandId) -> CommandRequest {
        operations
            .iter()
            .find_map(|operation| match operation {
                CommandOperation::PushFront(request) if request.id == id => Some(request.clone()),
                _ => None,
            })
            .unwrap_or_else(|| panic!("expected {id:?} PushFront operation, got {operations:?}"))
    }

    fn continue_construct_script(state: &mut ConstructState) {
        state.script_pending = true;
        state.script_invoked = true;
        state.script_result = Some(AcquireScriptResult::Continue);
    }

    fn command_view(id: CommandId, target: Option<ObjectId>) -> CommandView {
        CommandView {
            name: id.to_name().to_owned(),
            target,
            tx: None,
            tx_value: None,
            tx_definition: None,
            ty: None,
            target2: None,
            data: CommandData::None,
            legacy_data: None,
            finished: false,
        }
    }

    fn assert_silent_child_failure_propagates(
        parent: CommandRequest,
        child: CommandRequest,
        ctx: &CommandRuntimeContext<'_>,
    ) {
        assert_eq!(child.mode, CommandMode::SilentSub);
        let parent_id = parent.id;
        let child_id = child.id;
        let mut stack = CommandStack::new();
        stack
            .push_back(parent.with_mode(CommandMode::Base))
            .expect("parent command queued");
        stack.push_front(child).expect("child command queued");
        assert!(stack.fail_front_if(child_id));

        let failure = stack.step(ctx).expect("child failure evaluated");
        assert_eq!(failure.status, CommandStatus::Failed);
        let snapshot = stack.snapshot();
        assert_eq!(snapshot.commands.len(), 1);
        assert_eq!(snapshot.commands[0].state.id(), Some(parent_id));
        assert_eq!(snapshot.commands[0].failures, 1);
    }

    fn walking_jumper(position: Vector2) -> CommandObjectSnapshot {
        let mut walker = snapshot_with_id(1);
        walker.position = position;
        walker.action_name = "Walk".into();
        walker.action_idle = false;
        walker.action_procedure = ActionProcedure::Walk;
        walker.crew_member = true;
        walker.ocf |= ocf::CREW_MEMBER;
        walker.shape_top = -10;
        walker
    }

    fn pathfinder_jumper(position: Vector2) -> CommandObjectSnapshot {
        let mut walker = walking_jumper(position);
        walker.crew_member = false;
        walker.ocf &= !ocf::CREW_MEMBER;
        walker.pathfinder = 1;
        walker
    }

    fn jump_ctx<'a>(
        walker: &'a CommandObjectSnapshot,
        objects: &'a CommandObjectSnapshots,
        players: &'a HashMap<i32, CommandPlayerSnapshot>,
        definitions: &'a HashMap<DefinitionId, CommandDefinitionSnapshot>,
        landscape: &'a crate::Landscape,
    ) -> CommandRuntimeContext<'a> {
        CommandRuntimeContext {
            landscape: Some(landscape),
            ..command_ctx_at_frame(walker, objects, players, definitions, 0)
        }
    }

    /// A MoveTo state past its InitEvaluation Execute with the raw Tx/Ty
    /// (the C++ equivalent of an Evaluated command): the movement-control
    /// geometry pins below run against these coordinates directly.
    fn evaluated_move_to(request: &CommandRequest) -> MoveToState {
        let mut state = MoveToState::from_request(request);
        state.evaluated = true;
        state
    }

    fn command_ctx_at_frame<'a>(
        object: &'a CommandObjectSnapshot,
        objects: &'a CommandObjectSnapshots,
        players: &'a HashMap<i32, CommandPlayerSnapshot>,
        definitions: &'a HashMap<DefinitionId, CommandDefinitionSnapshot>,
        frame: u64,
    ) -> CommandRuntimeContext<'a> {
        CommandRuntimeContext {
            landscape: None,
            frame,
            position: object.position,
            object,
            objects,
            players,
            definitions,
            structures_need_energy: false,
            base_buy_enabled: true,
            base_sell_enabled: true,
            transfer_zones: &EMPTY_TRANSFER_ZONES,
            rng: None,
        }
    }

    #[test]
    fn move_to_crew_uses_one_fifth_shape_width_as_target_range() {
        // Crew override the default five-pixel MoveToRange with
        // Shape.Wdt/5. Tutorial05's eight-pixel-wide CLNK at x=156 must
        // therefore walk toward the elevator's centering point x=160
        // instead of treating the four-pixel gap as arrived
        // (C4Command.cpp:286-306; Case.c4d/Script.c:171-220).
        let mut clonk = walking_jumper(Vector2::new(156, 100));
        clonk.shape = DefinitionRect::new(-4, -9, 8, 18);
        let objects = CommandObjectSnapshots::default();
        let players = HashMap::new();
        let definitions = HashMap::new();
        let ctx = command_ctx_at_frame(&clonk, &objects, &players, &definitions, 1);
        let mut state = evaluated_move_to(
            &CommandRequest::new(CommandId::MoveTo)
                .with_tx(Some(160))
                .with_ty(Some(100)),
        );

        let result = state.step(&ctx);

        assert_eq!(result.status, CommandStatus::Running);
        assert_eq!(
            result.update.and_then(|update| update.command_direction),
            Some(CommandDirection::Right),
            "four pixels exceeds CLNK Shape.Wdt/5 = 1"
        );
    }

    #[test]
    fn move_to_missing_coordinates_still_reads_free_move_physical() {
        let mut actor = walking_jumper(Vector2::new(20, 20));
        actor.physical_deferred = true;
        let objects = CommandObjectSnapshots::from_iter([(actor.id, actor.clone())]);
        let players = HashMap::new();
        let definitions = HashMap::new();
        let landscape = crate::Landscape::flat(100, 60);
        let ctx = jump_ctx(&actor, &objects, &players, &definitions, &landscape);
        let mut stack = CommandStack::new();
        stack
            .push_front(CommandRequest::new(CommandId::MoveTo))
            .expect("MoveTo queues");

        let result = stack.execute_front(&ctx).expect("MoveTo evaluates");
        assert!(matches!(
            result.events.as_slice(),
            [CommandEvent::ResolveCommandPhysical { reads: 1, .. }]
        ));
        let view = stack
            .command_views()
            .into_iter()
            .next()
            .expect("MoveTo remains linked");
        assert_eq!(view.tx, Some(0));
        assert_eq!(view.ty, Some(0));
    }

    #[test]
    fn move_to_intermediate_crew_waypoint_uses_asymmetric_3_3_2_range() {
        // Crew waypoints use Shape.Wdt/5 with side/top/bottom factors 3/3/2;
        // the immediate next stack node must itself be MoveTo
        // (C4Command.cpp:218-220,286-304).
        let target = Vector2::new(100, 100);
        for (offset, following_move_to, expected) in [
            (Vector2::new(6, 0), true, CommandStatus::Completed),
            (Vector2::new(7, 0), true, CommandStatus::Running),
            (Vector2::new(0, 6), true, CommandStatus::Completed),
            (Vector2::new(0, 7), true, CommandStatus::Running),
            (Vector2::new(0, -4), true, CommandStatus::Completed),
            (Vector2::new(0, -5), true, CommandStatus::Running),
            (Vector2::new(6, 0), false, CommandStatus::Running),
        ] {
            let mut clonk = walking_jumper(Vector2::new(target.x + offset.x, target.y + offset.y));
            clonk.shape = DefinitionRect::new(-5, -9, 10, 18);
            let objects = CommandObjectSnapshots::default();
            let players = HashMap::new();
            let definitions = HashMap::new();
            let ctx = command_ctx_at_frame(&clonk, &objects, &players, &definitions, 1);
            let mut stack = CommandStack::new();
            stack
                .push_back(
                    CommandRequest::new(CommandId::MoveTo)
                        .with_tx(Some(target.x))
                        .with_ty(Some(target.y))
                        .with_evaluated(true),
                )
                .expect("current MoveTo queues");
            if following_move_to {
                stack
                    .push_back(
                        CommandRequest::new(CommandId::MoveTo)
                            .with_tx(Some(200))
                            .with_ty(Some(100))
                            .with_evaluated(true),
                    )
                    .expect("following MoveTo queues");
            }

            let result = stack.step(&ctx).expect("MoveTo executes");
            assert_eq!(
                result.status, expected,
                "offset={offset:?}, following_move_to={following_move_to}"
            );
            let expected_len =
                usize::from(following_move_to) + usize::from(expected == CommandStatus::Running);
            assert_eq!(stack.len(), expected_len);
        }
    }

    #[test]
    fn move_to_crew_pushes_pathfinder_waypoints_around_blocked_ground() {
        // A solid cave wall below the column surface blocks the direct line.
        // C4Command::MoveTo asks C4PathFinder for a route and pushes its
        // intermediate points as 25-frame MoveTo subcommands, preserving the
        // parent command's Data (C4Command.cpp:193-255).
        let mut landscape = crate::Landscape::with_default_material(100, vec![100; 100], None)
            .expect("cave landscape");
        landscape.set_world_height(100);
        let mut bytes = vec![0; 100 * 100];
        for y in 45..55 {
            for x in 45..47 {
                bytes[y * 100 + x] = 1;
            }
        }
        landscape.set_pixel_grid(crate::landscape::PixelGrid::new(
            100,
            100,
            bytes,
            vec![0, 100],
            vec![None, Some("Earth".to_owned())],
            vec![None; 2],
        ));
        let walker = walking_jumper(Vector2::new(10, 50));
        let objects = CommandObjectSnapshots::default();
        let players = HashMap::new();
        let definitions = HashMap::new();
        let ctx = jump_ctx(&walker, &objects, &players, &definitions, &landscape);
        let mut state = evaluated_move_to(
            &CommandRequest::new(CommandId::MoveTo)
                .with_tx(Some(90))
                .with_ty(Some(50))
                .with_data(CommandData::Integer(COMMAND_FLAG_MOVE_TO_PUSH_TARGET)),
        );

        let result = state.step(&ctx);

        assert_eq!(result.status, CommandStatus::Running);
        assert_eq!(
            result.operations,
            vec![
                CommandOperation::PushFront(
                    CommandRequest::new(CommandId::MoveTo)
                        .with_tx(Some(54))
                        .with_ty(Some(46))
                        .with_data(CommandData::Integer(COMMAND_FLAG_MOVE_TO_PUSH_TARGET))
                        .with_update_interval(25)
                        .with_evaluated(true)
                        .with_mode(CommandMode::SilentSub),
                ),
                CommandOperation::PushFront(
                    CommandRequest::new(CommandId::MoveTo)
                        .with_tx(Some(47))
                        .with_ty(Some(44))
                        .with_data(CommandData::Integer(COMMAND_FLAG_MOVE_TO_PUSH_TARGET))
                        .with_update_interval(25)
                        .with_evaluated(true)
                        .with_mode(CommandMode::SilentSub),
                ),
            ],
            "ObjectAddWaypoint applies the shape offset and pushes each deterministic intermediate waypoint with Data and interval 25 (C4Command.cpp:189-208; C4PathFinder.cpp:383-400)"
        );

        let CommandOperation::PushFront(nearest_waypoint) = &result.operations[1] else {
            panic!("nearest pathfinder operation must push MoveTo");
        };
        let mut waypoint_stack = CommandStack::new();
        waypoint_stack
            .push_front(nearest_waypoint.clone())
            .expect("waypoint queues");
        let first = waypoint_stack.step(&ctx).expect("waypoint executes");
        assert_eq!(
            first.update.and_then(|update| update.command_direction),
            Some(CommandDirection::Right),
            "an evaluated pathfinder waypoint steers on its first Execute"
        );
        assert_eq!(
            waypoint_stack.snapshot().commands[0].update_interval,
            Some(24),
            "the shared command lifetime decrements before waypoint steering"
        );
    }

    #[test]
    fn move_to_direct_pathfinder_success_returns_without_steering_or_recheck_delay() {
        // C4Command::MoveTo returns after every PathFinder::Find call. A
        // successful direct finder ray emits no intermediate waypoint, does
        // not set PathChecked, and must not fall through to steering in the
        // same Execute (C4Command.cpp:236-248). This reversed line makes the
        // command-level PathFree sample (16,8), while C4PathFinder's ray
        // samples (16,7), deterministically reaching that direct-success arm.
        let mut landscape = crate::Landscape::with_default_material(40, vec![40; 40], None)
            .expect("test landscape");
        landscape.set_world_height(40);
        let mut bytes = vec![0; 40 * 40];
        bytes[8 * 40 + 16] = 1;
        landscape.set_pixel_grid(crate::landscape::PixelGrid::new(
            40,
            40,
            bytes,
            vec![0, 100],
            vec![None, Some("Earth".to_owned())],
            vec![None; 2],
        ));

        let start = Vector2::new(30, 10);
        let target = Vector2::new(2, 5);
        assert!(!command_path_free(
            &landscape, start.x, start.y, target.x, target.y
        ));
        let direct_path = PathFinder::new(&landscape, &[])
            .find(start, target)
            .expect("finder's reversed ray is direct");
        assert_eq!(direct_path.waypoints.len(), 2, "start and target only");

        let walker = walking_jumper(start);
        let objects = CommandObjectSnapshots::default();
        let players = HashMap::new();
        let definitions = HashMap::new();
        let mut ctx = jump_ctx(&walker, &objects, &players, &definitions, &landscape);
        ctx.frame = 1;
        let mut state = evaluated_move_to(
            &CommandRequest::new(CommandId::MoveTo)
                .with_tx(Some(target.x))
                .with_ty(Some(target.y)),
        );

        for execution in 1..=2 {
            let result = state.step(&ctx);
            assert_eq!(result.status, CommandStatus::Running);
            assert!(
                result.update.is_none(),
                "execution {execution} must not steer"
            );
            assert!(result.operations.is_empty(), "no intermediate waypoint");
            assert!(!state.path_checked, "direct success has no recheck delay");
            assert_eq!(
                state.pathfinder_settings_update.take(),
                Some((1, true)),
                "execution {execution} invokes the finder again"
            );
        }
    }

    #[test]
    fn pathfinder_waypoint_skips_regrounding_after_solid_offset() {
        // ObjectAddWaypoint first nudges this point left from the ledge via
        // AdjustSolidOffset, then creates an already-evaluated MoveTo. The
        // waypoint must remain mid-air instead of AdjustMoveToTarget dropping
        // it onto the lower surface (C4Command.cpp:189-208,1628-1643).
        let mut surface = vec![110i32; 300];
        for column in surface.iter_mut().take(190).skip(150) {
            *column = 75;
        }
        let landscape =
            crate::Landscape::with_default_material(300, surface, None).expect("landscape");
        let mut walker = walking_jumper(Vector2::new(100, 100));
        walker.shape = DefinitionRect::new(-8, -10, 16, 20);
        let (mut x, mut y) = (149, 75);
        assert!(adjust_solid_offset(
            &landscape,
            &mut x,
            &mut y,
            walker.shape.width / 2,
            walker.shape.height / 2,
        ));
        assert_eq!((x, y), (142, 75));

        let objects = CommandObjectSnapshots::default();
        let players = HashMap::new();
        let definitions = HashMap::new();
        let ctx = jump_ctx(&walker, &objects, &players, &definitions, &landscape);
        let request = CommandRequest::new(CommandId::MoveTo)
            .with_tx(Some(x))
            .with_ty(Some(y))
            .with_data(CommandData::Integer(0))
            .with_update_interval(25)
            .with_evaluated(true)
            .with_mode(CommandMode::SilentSub);
        let mut state = MoveToState::from_request(&request);

        let first = state.step(&ctx);

        assert_eq!((state.tx, state.ty), (Some(142), Some(75)));
        assert_eq!(
            first.update.and_then(|update| update.command_direction),
            Some(CommandDirection::Right),
            "the post-AdjustSolidOffset coordinate steers without an evaluation-only frame"
        );
    }

    #[test]
    fn move_to_skips_pathfinder_when_the_direct_path_is_free() {
        // Transfer zones are consulted only after the ordinary PathFree
        // probe reports solid terrain. A clear line that merely crosses a
        // zone must remain one direct MoveTo (C4Command.cpp:235-252).
        let landscape = crate::Landscape::flat(200, 100);
        let walker = walking_jumper(Vector2::new(20, 50));
        let objects = CommandObjectSnapshots::default();
        let players = HashMap::new();
        let definitions = HashMap::new();
        let mut transfer_zones = TransferZoneTable::default();
        transfer_zones.set(
            ObjectId::new(9),
            TransferZoneRect {
                x: 80,
                y: 40,
                width: 20,
                height: 20,
            },
        );
        let ctx = CommandRuntimeContext {
            landscape: Some(&landscape),
            transfer_zones: &transfer_zones,
            ..command_ctx_at_frame(&walker, &objects, &players, &definitions, 1)
        };
        let mut state = evaluated_move_to(
            &CommandRequest::new(CommandId::MoveTo)
                .with_tx(Some(160))
                .with_ty(Some(50)),
        );

        let result = state.step(&ctx);

        assert!(
            result.operations.is_empty(),
            "a transfer zone alone does not trigger pathfinding"
        );
        assert_eq!(
            result.update.and_then(|update| update.command_direction),
            Some(CommandDirection::Right)
        );
    }

    #[test]
    fn definition_pathfinder_routes_noncrew_and_honors_transfer_zone_opt_out() {
        // C4Command::MoveTo enables path search for OCF_CrewMember OR a
        // nonzero Def->Pathfinder, passes the raw level through SetLevel,
        // and disables zones for Def->NoTransferZones
        // (C4Command.cpp:228-248; C4PathFinder.cpp:552-560).
        let mut landscape = crate::Landscape::with_default_material(100, vec![100; 100], None)
            .expect("split landscape");
        landscape.set_world_height(100);
        let mut bytes = vec![0; 100 * 100];
        for y in 0..100 {
            bytes[y * 100 + 49] = 1;
            bytes[y * 100 + 50] = 1;
        }
        landscape.set_pixel_grid(crate::landscape::PixelGrid::new(
            100,
            100,
            bytes,
            vec![0, 100],
            vec![None, Some("Earth".to_owned())],
            vec![None; 2],
        ));
        let mut mover = snapshot_with_id(1);
        mover.position = Vector2::new(10, 50);
        mover.action_procedure = ActionProcedure::Walk;
        mover.pathfinder = 27;
        let objects = CommandObjectSnapshots::default();
        let players = HashMap::new();
        let definitions = HashMap::new();
        let zone_owner = ObjectId::new(9);
        let mut transfer_zones = TransferZoneTable::default();
        transfer_zones.set(
            zone_owner,
            TransferZoneRect {
                x: 45,
                y: 40,
                width: 10,
                height: 20,
            },
        );

        let enabled = {
            let ctx = CommandRuntimeContext {
                landscape: Some(&landscape),
                transfer_zones: &transfer_zones,
                ..command_ctx_at_frame(&mover, &objects, &players, &definitions, 1)
            };
            evaluated_move_to(
                &CommandRequest::new(CommandId::MoveTo)
                    .with_tx(Some(90))
                    .with_ty(Some(50)),
            )
            .step(&ctx)
        };
        assert_eq!(
            enabled.operations,
            vec![
                CommandOperation::PushFront(
                    CommandRequest::new(CommandId::MoveTo)
                        .with_tx(Some(58))
                        .with_ty(Some(89))
                        .with_data(CommandData::Integer(0))
                        .with_update_interval(25)
                        .with_evaluated(true)
                        .with_mode(CommandMode::SilentSub),
                ),
                CommandOperation::PushFront(
                    CommandRequest::new(CommandId::Transfer)
                        .with_target(Some(zone_owner))
                        .with_tx(Some(55))
                        .with_ty(Some(89))
                        .with_evaluated(true)
                        .with_mode(CommandMode::SilentSub),
                ),
            ],
            "the clamped non-crew search crosses the only available transfer edge"
        );

        mover.no_transfer_zones = 1;
        let disabled = {
            let ctx = CommandRuntimeContext {
                landscape: Some(&landscape),
                transfer_zones: &transfer_zones,
                ..command_ctx_at_frame(&mover, &objects, &players, &definitions, 1)
            };
            evaluated_move_to(
                &CommandRequest::new(CommandId::MoveTo)
                    .with_tx(Some(90))
                    .with_ty(Some(50)),
            )
            .step(&ctx)
        };
        assert_eq!(disabled.status, CommandStatus::Running);
        assert!(
            disabled.operations.is_empty(),
            "without transfer-zone edges, the full-height wall has no route"
        );
    }

    #[test]
    fn move_to_uses_positive_definition_range_for_noncrew_like_cpp() {
        // C4Command::MoveTo replaces the default five-pixel range only when
        // Def->MoveToRange is positive (C4Command.cpp:213-215); signed zero
        // and negative DefCore values retain the default.
        let mut mover = snapshot_with_id(1);
        mover.position = Vector2::new(100, 100);
        mover.fixed_position = FixedVec2::from_ints(100, 100);
        mover.action_name = "Walk".into();
        mover.action_procedure = ActionProcedure::Walk;
        mover.move_to_range = 20;
        let objects = CommandObjectSnapshots::default();
        let players = HashMap::new();
        let definitions = HashMap::new();
        let request = CommandRequest::new(CommandId::MoveTo)
            .with_tx(Some(115))
            .with_ty(Some(100));
        let ctx = command_ctx_at_frame(&mover, &objects, &players, &definitions, 1);
        let mut state = evaluated_move_to(&request);
        assert_eq!(state.step(&ctx).status, CommandStatus::Completed);

        mover.move_to_range = -3;
        let ctx = command_ctx_at_frame(&mover, &objects, &players, &definitions, 1);
        let mut state = evaluated_move_to(&request);
        let result = state.step(&ctx);
        assert_eq!(result.status, CommandStatus::Running);
        assert_eq!(
            result.update.and_then(|update| update.command_direction),
            Some(CommandDirection::Right)
        );
    }

    // C4Command::MoveTo DFA_SWIM arm (C4Command.cpp:370-382): on Tick2
    // frames (Game.iTick2 != 0, i.e. odd FrameCounter) the swimmer steers
    // horizontally toward Tx (with target range); on !Tick2 frames it
    // steers vertically toward Ty with NO range (cy < Ty -> Down).
    #[test]
    fn move_to_swim_steers_horizontal_on_tick2_and_vertical_otherwise() {
        let mut swimmer = snapshot_with_id(1);
        swimmer.position = Vector2::new(100, 100);
        swimmer.action_procedure = ActionProcedure::Swim;
        swimmer.crew_member = true;
        swimmer.ocf |= ocf::CREW_MEMBER;
        let objects = CommandObjectSnapshots::default();
        let players = HashMap::new();
        let definitions = HashMap::new();

        // Target right and below: dx = 60, dy = 40.
        let request = CommandRequest::new(CommandId::MoveTo)
            .with_tx(Some(160))
            .with_ty(Some(140));

        // Odd frame (iTick2 == 1): horizontal arm -> COMD_Right.
        let ctx = command_ctx_at_frame(&swimmer, &objects, &players, &definitions, 1);
        let mut state = evaluated_move_to(&request);
        let result = state.step(&ctx);
        assert_eq!(result.status, CommandStatus::Running);
        assert_eq!(
            result.update.and_then(|update| update.command_direction),
            Some(CommandDirection::Right),
            "Tick2 swim steering is horizontal (C4Command.cpp:372-376)"
        );

        // Even frame (iTick2 == 0): vertical arm -> COMD_Down (cy < Ty).
        let ctx = command_ctx_at_frame(&swimmer, &objects, &players, &definitions, 2);
        let mut state = evaluated_move_to(&request);
        let result = state.step(&ctx);
        assert_eq!(result.status, CommandStatus::Running);
        assert_eq!(
            result.update.and_then(|update| update.command_direction),
            Some(CommandDirection::Down),
            "!Tick2 swim steering is vertical (C4Command.cpp:377-381)"
        );
    }

    #[test]
    fn move_to_float_steers_against_fixed_momentum_like_cpp() {
        // DFA_FLOAT aims for a Float-physical velocity toward the target,
        // then steers from the fixed-point difference to current momentum
        // (C4Command.cpp:393-410). A floater already moving upward while its
        // target is due right therefore corrects DownRight, not merely Right.
        let mut floater = snapshot_with_id(1);
        floater.position = Vector2::new(100, 100);
        floater.fixed_position = FixedVec2::from_ints(100, 100);
        floater.fixed_velocity = FixedVec2::from_ints(0, -1);
        floater.action_procedure = ActionProcedure::Float;
        floater.physical.float = 100;
        let objects = CommandObjectSnapshots::default();
        let players = HashMap::new();
        let definitions = HashMap::new();
        let ctx = command_ctx_at_frame(&floater, &objects, &players, &definitions, 1);
        let mut state = evaluated_move_to(
            &CommandRequest::new(CommandId::MoveTo)
                .with_tx(Some(200))
                .with_ty(Some(100)),
        );

        let result = state.step(&ctx);

        assert_eq!(result.status, CommandStatus::Running);
        assert_eq!(
            result.update.and_then(|update| update.command_direction),
            Some(CommandDirection::DownRight)
        );

        // At the desired rightward velocity the correction falls below
        // FIXED100(20), so this newly created command must explicitly clear
        // the object's pre-existing Right ComDir with COMD_None/Stop.
        floater.command_direction = CommandDirection::Right;
        floater.fixed_velocity = FixedVec2::from_ints(1, 0);
        let ctx = command_ctx_at_frame(&floater, &objects, &players, &definitions, 2);
        let mut state = evaluated_move_to(
            &CommandRequest::new(CommandId::MoveTo)
                .with_tx(Some(200))
                .with_ty(Some(100)),
        );
        let result = state.step(&ctx);
        assert_eq!(
            result.update.and_then(|update| update.command_direction),
            Some(CommandDirection::Stop)
        );
    }

    #[test]
    fn move_to_float_finishes_immediately_inside_range_like_cpp() {
        // The target-reached branch precedes procedure steering and finishes
        // in this Execute (C4Command.cpp:286-307). Besides avoiding a second
        // arrival frame, this keeps DFA_FLOAT from normalizing a zero vector.
        let mut floater = snapshot_with_id(1);
        floater.position = Vector2::new(100, 100);
        floater.fixed_position = FixedVec2::from_ints(100, 100);
        floater.action_procedure = ActionProcedure::Float;
        floater.command_direction = CommandDirection::Right;
        floater.physical.float = 100;
        let objects = CommandObjectSnapshots::default();
        let players = HashMap::new();
        let definitions = HashMap::new();
        let ctx = command_ctx_at_frame(&floater, &objects, &players, &definitions, 1);
        let mut state = evaluated_move_to(
            &CommandRequest::new(CommandId::MoveTo)
                .with_tx(Some(100))
                .with_ty(Some(100)),
        );

        let result = state.step(&ctx);

        assert_eq!(result.status, CommandStatus::Completed);
        assert_eq!(
            result.update.and_then(|update| update.command_direction),
            Some(CommandDirection::Stop)
        );
    }

    // C4Command::MoveTo DFA_SCALE arm (C4Command.cpp:335-338): vertical
    // steering only — cy > Ty + range heads Up, cy < Ty - range heads
    // Down (y grows downward).
    #[test]
    fn move_to_scale_steers_vertically() {
        let mut scaler = snapshot_with_id(1);
        scaler.position = Vector2::new(100, 100);
        scaler.action_procedure = ActionProcedure::Scale;
        scaler.crew_member = true;
        scaler.ocf |= ocf::CREW_MEMBER;
        let objects = CommandObjectSnapshots::default();
        let players = HashMap::new();
        let definitions = HashMap::new();
        let ctx = command_ctx_at_frame(&scaler, &objects, &players, &definitions, 1);

        // Target above and well to the right: DFA_SCALE ignores Tx for
        // steering (no horizontal branch in the arm) and heads Up. The
        // Dir_Left let-go stays quiet: |cy - Ty| = 60 > LetGoRange2 30.
        let mut state = evaluated_move_to(
            &CommandRequest::new(CommandId::MoveTo)
                .with_tx(Some(140))
                .with_ty(Some(40)),
        );
        let result = state.step(&ctx);
        assert_eq!(result.status, CommandStatus::Running);
        assert_eq!(
            result.update.and_then(|update| update.command_direction),
            Some(CommandDirection::Up),
            "scaling toward a higher target heads Up (C4Command.cpp:337)"
        );
    }

    // DFA_SCALE let-go control (C4Command.cpp:339-353): scaling with
    // Action.Dir DIR_Left and the target off the wall to the right
    // (Tx > cx + LetGoRange1 7, |cy - Ty| <= LetGoRange2 30) jumps off
    // with xdir +1 (ObjectComLetGo -> ObjectActionJump(itofix(+1), 0)).
    #[test]
    fn move_to_scale_lets_go_toward_target() {
        let mut scaler = snapshot_with_id(1);
        scaler.position = Vector2::new(100, 100);
        scaler.action_procedure = ActionProcedure::Scale;
        scaler.direction = Direction::Left;
        scaler.crew_member = true;
        scaler.ocf |= ocf::CREW_MEMBER;
        let objects = CommandObjectSnapshots::default();
        let players = HashMap::new();
        let definitions = HashMap::new();
        let ctx = command_ctx_at_frame(&scaler, &objects, &players, &definitions, 1);

        let mut state = evaluated_move_to(
            &CommandRequest::new(CommandId::MoveTo)
                .with_tx(Some(140))
                .with_ty(Some(110)),
        );
        let result = state.step(&ctx);
        assert_eq!(result.status, CommandStatus::Running);
        let update = result.update.expect("let-go update");
        let action = update.action.expect("jump action");
        assert_eq!(action.name.as_deref(), Some("Jump"));
        assert_eq!(
            update.fixed_velocity,
            Some(FixedVec2::new(math::itofix(1), crate::C4Fixed::from_raw(0))),
            "let-go launches with xdir +1, ydir 0 (C4ObjectCom.cpp:310-314)"
        );
    }

    // The contact let-go (C4Command.cpp:347-352,361-366) only fires once
    // the scale action is 3+ frames old ("not if just started").
    #[test]
    fn move_to_scale_contact_let_go_respects_action_time() {
        let mut scaler = snapshot_with_id(1);
        scaler.position = Vector2::new(100, 100);
        scaler.action_procedure = ActionProcedure::Scale;
        scaler.direction = Direction::Right;
        scaler.contact = crate::CNAT_LEFT;
        scaler.crew_member = true;
        scaler.ocf |= ocf::CREW_MEMBER;
        let objects = CommandObjectSnapshots::default();
        let players = HashMap::new();
        let definitions = HashMap::new();

        // Target high above on this side: no target-direction let-go.
        let request = CommandRequest::new(CommandId::MoveTo)
            .with_tx(Some(100))
            .with_ty(Some(20));

        // Action.Time == 2: too fresh, keep scaling.
        scaler.action_time = 2;
        let ctx = command_ctx_at_frame(&scaler, &objects, &players, &definitions, 1);
        let mut state = evaluated_move_to(&request);
        let result = state.step(&ctx);
        assert!(
            result
                .update
                .as_ref()
                .and_then(|update| update.action.as_ref())
                .is_none(),
            "Action.Time <= 2 must not let go (C4Command.cpp:348)"
        );

        // Action.Time == 3 with contact: let go against the facing (-1).
        scaler.action_time = 3;
        let ctx = command_ctx_at_frame(&scaler, &objects, &players, &definitions, 1);
        let mut state = evaluated_move_to(&request);
        let result = state.step(&ctx);
        let update = result.update.expect("let-go update");
        assert_eq!(
            update.action.and_then(|action| action.name),
            Some("Jump".into())
        );
        assert_eq!(
            update.fixed_velocity,
            Some(FixedVec2::new(
                math::itofix(-1),
                crate::C4Fixed::from_raw(0)
            )),
            "DIR_Right contact let-go jumps with xdir -1 (C4Command.cpp:365)"
        );
    }

    // C4Command::MoveTo DFA_HANGLE arm (C4Command.cpp:384-391):
    // horizontal steering; |Angle(cx,cy,Tx,Ty)| > LetGoHangleAngle 110
    // drops off the ceiling (ObjectComLetGo(0) — Jump with zero xdir).
    #[test]
    fn move_to_hangle_steers_horizontal_and_drops_past_angle() {
        let mut hangler = snapshot_with_id(1);
        hangler.position = Vector2::new(100, 100);
        hangler.action_procedure = ActionProcedure::Hang;
        hangler.crew_member = true;
        hangler.ocf |= ocf::CREW_MEMBER;
        let objects = CommandObjectSnapshots::default();
        let players = HashMap::new();
        let definitions = HashMap::new();

        // Target right, slightly below: Angle = 99 <= 110 keeps hangling;
        // steer Right. No vertical branch in the arm.
        let ctx = command_ctx_at_frame(&hangler, &objects, &players, &definitions, 1);
        let mut state = evaluated_move_to(
            &CommandRequest::new(CommandId::MoveTo)
                .with_tx(Some(160))
                .with_ty(Some(110)),
        );
        let result = state.step(&ctx);
        let update = result.update.expect("steer update");
        assert_eq!(update.command_direction, Some(CommandDirection::Right));
        assert!(update.action.is_none(), "within LetGoHangleAngle: no drop");

        // Target straight below: Angle = 180 > 110 -> ObjectComLetGo(0).
        let ctx = command_ctx_at_frame(&hangler, &objects, &players, &definitions, 1);
        let mut state = evaluated_move_to(
            &CommandRequest::new(CommandId::MoveTo)
                .with_tx(Some(100))
                .with_ty(Some(160)),
        );
        let result = state.step(&ctx);
        assert_eq!(result.status, CommandStatus::Running);
        let update = result.update.expect("drop update");
        assert_eq!(
            update.action.and_then(|action| action.name),
            Some("Jump".into())
        );
        assert_eq!(
            update.fixed_velocity,
            Some(FixedVec2::new(math::itofix(0), crate::C4Fixed::from_raw(0))),
            "hangle drop has zero launch velocity (C4Command.cpp:390)"
        );
    }

    // C4Command::MoveTo DFA_FLIGHT arm (C4Command.cpp:414-417): no ComDir
    // steering at all — only FlightControl, which re-arms the Fly action
    // for a CanFly Pathfinder object with the target in the ±60° top sector.
    #[test]
    fn move_to_noncrew_pathfinder_flight_control_and_disabled_gate() {
        let landscape = crate::Landscape::flat(300, 110);
        let mut flyer = snapshot_with_id(1);
        flyer.position = Vector2::new(100, 100);
        flyer.action_procedure = ActionProcedure::Flight;
        flyer.pathfinder = 1;
        flyer.physical.can_fly = 1;
        flyer.shape_top = -10;
        assert_eq!(flyer.ocf & ocf::CREW_MEMBER, 0);
        let objects = CommandObjectSnapshots::default();
        let players = HashMap::new();
        let definitions = HashMap::new();
        let mut ctx = command_ctx_at_frame(&flyer, &objects, &players, &definitions, 1);
        ctx.landscape = Some(&landscape);

        // Target up and slightly right (angle 9, distance 70, sky above):
        // FlightControl takes off; the flight arm never assigns ComDir.
        let mut state = evaluated_move_to(
            &CommandRequest::new(CommandId::MoveTo)
                .with_tx(Some(110))
                .with_ty(Some(30)),
        );
        let result = state.step(&ctx);
        assert_eq!(result.status, CommandStatus::Running);
        assert!(
            result.update.is_none(),
            "DFA_FLIGHT never steers ComDir (C4Command.cpp:414-417)"
        );
        assert!(
            matches!(
                result.events.as_slice(),
                [CommandEvent::MoveToFlightControlTakeoff { .. }]
            ),
            "FlightControl takes off (C4Command.cpp:1843)"
        );
        let resumed = state.resume_after_flight_control(&ctx);
        assert!(
            resumed.operations.is_empty(),
            "DFA_FLIGHT has no walking JumpControl tail"
        );

        let mut disabled_flyer = flyer.clone();
        disabled_flyer.action_disabled = true;
        let mut disabled_ctx =
            command_ctx_at_frame(&disabled_flyer, &objects, &players, &definitions, 1);
        disabled_ctx.landscape = Some(&landscape);
        let mut disabled_state = evaluated_move_to(
            &CommandRequest::new(CommandId::MoveTo)
                .with_tx(Some(110))
                .with_ty(Some(30)),
        );
        let disabled_result = disabled_state.step(&disabled_ctx);
        assert!(
            disabled_result.update.is_none(),
            "ObjectDisabled suppresses FlightControl without inventing flight steering"
        );
        assert!(
            disabled_result.events.is_empty(),
            "ObjectDisabled suppresses the callbackful Fly transition"
        );
    }

    // C4CMD_MoveTo InitEvaluation (C4Command.cpp:1634-1643): the first
    // Execute only evaluates (returns true — no movement that frame);
    // AdjustMoveToTarget grounds a mid-air target unless Data carries
    // C4CMD_MoveTo_NoPosAdjust (C4Command.h:68).
    #[test]
    fn move_to_init_evaluation_adjusts_target_unless_no_pos_adjust() {
        let landscape = crate::Landscape::flat(300, 110);
        // Standing walker: center y 100, feet on the 110 surface.
        let walker = walking_jumper(Vector2::new(100, 100));
        let objects = CommandObjectSnapshots::default();
        let players = HashMap::new();
        let definitions = HashMap::new();
        let mut ctx = command_ctx_at_frame(&walker, &objects, &players, &definitions, 1);
        ctx.landscape = Some(&landscape);

        // Mid-air target straight up: AdjustMoveToTarget drops it to the
        // bottom of free space (109) then lifts it Shape.Hgt/2 -> y 99,
        // one pixel off the walker's center — inside the crew range.
        let request = CommandRequest::new(CommandId::MoveTo)
            .with_tx(Some(100))
            .with_ty(Some(50));
        assert!(
            !request.evaluated,
            "ordinary Enter/JumpControl/script MoveTos retain fInitEvaluation=true"
        );
        let mut state = MoveToState::from_request(&request); // unevaluated
        let first = state.step(&ctx);
        assert_eq!(first.status, CommandStatus::Running);
        assert!(
            first.update.is_none() && first.operations.is_empty(),
            "the evaluation Execute does nothing else (C4Command.cpp:1555)"
        );
        let mut ctx = command_ctx_at_frame(&walker, &objects, &players, &definitions, 2);
        ctx.landscape = Some(&landscape);
        let second = state.step(&ctx);
        assert_eq!(second.status, CommandStatus::Completed);
        assert_eq!(
            second.update.and_then(|update| update.command_direction),
            Some(CommandDirection::Stop),
            "the adjusted in-range target finishes immediately (C4Command.cpp:294-307)"
        );

        // NoPosAdjust keeps the raw (100,50), but DFA_WALK has no vertical
        // steering arm: the command remains pending without touching ComDir.
        let request = request.with_data(CommandData::Integer(1));
        let mut state = MoveToState::from_request(&request); // unevaluated
        let mut ctx = command_ctx_at_frame(&walker, &objects, &players, &definitions, 1);
        ctx.landscape = Some(&landscape);
        let _ = state.step(&ctx);
        let mut ctx = command_ctx_at_frame(&walker, &objects, &players, &definitions, 2);
        ctx.landscape = Some(&landscape);
        let second = state.step(&ctx);
        assert!(
            second
                .update
                .and_then(|update| update.command_direction)
                .is_none(),
            "NoPosAdjust leaves the mid-air target without inventing vertical Walk steering"
        );
    }

    // C4CMD_MoveTo InitEvaluation target absorption (C4Command.cpp:1637):
    // Tx/Ty become Target->x/y ONCE and Target clears — the destination
    // does not follow the target afterwards.
    #[test]
    fn move_to_absorbs_target_position_once() {
        let walker = walking_jumper(Vector2::new(100, 100));
        let target_id = ObjectId::new(9);
        let mut target = snapshot_with_id(9);
        target.position = Vector2::new(200, 100);
        let mut objects = CommandObjectSnapshots::default();
        objects.insert(target_id, target);
        let players = HashMap::new();
        let definitions = HashMap::new();

        let request = CommandRequest::new(CommandId::MoveTo).with_target(Some(target_id));
        let mut state = MoveToState::from_request(&request); // unevaluated
        let ctx = command_ctx_at_frame(&walker, &objects, &players, &definitions, 1);
        let _ = state.step(&ctx); // evaluation frame
        let ctx = command_ctx_at_frame(&walker, &objects, &players, &definitions, 2);
        let result = state.step(&ctx);
        assert_eq!(
            result.update.and_then(|update| update.command_direction),
            Some(CommandDirection::Right),
            "steers toward the absorbed (200,100)"
        );

        // Target teleports left; the command keeps heading for 200.
        objects.get_mut(&target_id).expect("target").position = Vector2::new(0, 100);
        let ctx = command_ctx_at_frame(&walker, &objects, &players, &definitions, 3);
        let result = state.step(&ctx);
        assert_ne!(
            result.update.and_then(|update| update.command_direction),
            Some(CommandDirection::Left),
            "Tx/Ty were absorbed once — no live following (C4Command.cpp:1637)"
        );
    }

    #[test]
    fn move_to_update_interval_is_cpp_lifetime_not_step_throttle() {
        // C4Command::Execute decrements UpdateInterval as a lifetime, but
        // still executes MoveTo on every non-expiring frame
        // (C4Command.cpp:1545-1555).
        let objects = CommandObjectSnapshots::default();
        let players = HashMap::new();
        let definitions = HashMap::new();
        let request = CommandRequest::new(CommandId::MoveTo)
            .with_tx(Some(200))
            .with_ty(Some(100))
            .with_update_interval(4);
        let mut stack = CommandStack::new();
        stack.push_front(request).expect("MoveTo queues");

        let mut walker = walking_jumper(Vector2::new(100, 100));
        let ctx = command_ctx_at_frame(&walker, &objects, &players, &definitions, 0);
        assert_eq!(
            stack.step(&ctx).expect("evaluation executes").status,
            CommandStatus::Running
        );

        let ctx = command_ctx_at_frame(&walker, &objects, &players, &definitions, 1);
        let result = stack.step(&ctx).expect("MoveTo executes");
        assert_eq!(
            result.update.and_then(|update| update.command_direction),
            Some(CommandDirection::Right)
        );

        walker.position = Vector2::new(210, 100);
        let ctx = command_ctx_at_frame(&walker, &objects, &players, &definitions, 2);
        let result = stack.step(&ctx).expect("MoveTo executes again");
        assert_eq!(
            result.update.and_then(|update| update.command_direction),
            Some(CommandDirection::Left),
            "MoveTo executes again on the next frame"
        );

        let ctx = command_ctx_at_frame(&walker, &objects, &players, &definitions, 3);
        assert_eq!(
            stack.step(&ctx).expect("interval expires").status,
            CommandStatus::Completed
        );
        assert!(stack.is_empty());
    }

    #[test]
    fn move_to_exits_a_container_before_steering() {
        // C4Command::MoveTo always delegates Exit before its path and
        // movement logic, which lets Build's automatic Acquire return from
        // a base with the component (C4Command.cpp:213-217).
        let container_id = ObjectId::new(9);
        let mut walker = walking_jumper(Vector2::new(100, 100));
        walker.container = Some(container_id);
        let objects = CommandObjectSnapshots::from_iter([(container_id, snapshot_with_id(9))]);
        let players = HashMap::new();
        let definitions = HashMap::new();
        let ctx = command_ctx_at_frame(&walker, &objects, &players, &definitions, 1);
        let mut state = evaluated_move_to(
            &CommandRequest::new(CommandId::MoveTo)
                .with_tx(Some(200))
                .with_ty(Some(100)),
        );

        let result = state.step(&ctx);

        assert_eq!(result.status, CommandStatus::Running);
        assert!(result.operations.iter().any(|operation| matches!(
            operation,
            CommandOperation::PushFront(request)
                if request.id == CommandId::Exit
                    && request.mode == CommandMode::SilentSub
                    && request.update_interval == 50
        )));
        assert!(result.update.is_none(), "contained MoveTo does not steer");
    }

    // C4Command::MoveTo pushing (C4Command.cpp:257-265): without the
    // C4CMD_MoveTo_PushTarget Data flag (C4Command.h:69) — or against a
    // Grab=2 grab-only target — a pushing mover lets go (UnGrab sub-
    // command) and marks itself for re-evaluation.
    #[test]
    fn move_to_push_without_push_target_flag_ungrabs() {
        let vehicle_id = ObjectId::new(7);
        let mut vehicle = snapshot_with_id(7);
        vehicle.position = Vector2::new(95, 100);
        let mut pusher = walking_jumper(Vector2::new(100, 100));
        pusher.action_procedure = ActionProcedure::Push;
        pusher.action_target = Some(vehicle_id);
        let mut objects = CommandObjectSnapshots::default();
        objects.insert(vehicle_id, vehicle);
        let players = HashMap::new();
        let mut definitions = HashMap::new();
        definitions.insert(
            "DEF7".to_string(),
            CommandDefinitionSnapshot {
                value: 0,
                can_chop: false,
                chop_action: None,
                constructable: false,
                grab: 1,
                ..CommandDefinitionSnapshot::default()
            },
        );

        // Data 0: pushing not desired -> UnGrab, still running.
        let mut state = evaluated_move_to(
            &CommandRequest::new(CommandId::MoveTo)
                .with_tx(Some(200))
                .with_ty(Some(100)),
        );
        let ctx = command_ctx_at_frame(&pusher, &objects, &players, &definitions, 1);
        let result = state.step(&ctx);
        assert_eq!(result.status, CommandStatus::Running);
        match result.operations.first() {
            Some(CommandOperation::PushFront(request)) => {
                assert_eq!(request.id, CommandId::UnGrab);
                assert_eq!(request.update_interval, 50);
            }
            other => panic!("expected UnGrab, got {other:?}"),
        }
        assert!(
            !state.evaluated,
            "vehicle control may have blocked evaluation — re-evaluate (C4Command.cpp:263)"
        );
    }

    // With the PushTarget flag the mover keeps pushing: cx/cy come from
    // the pushed vehicle (C4Command.cpp:271-277) and the DFA_PUSH arm
    // steers horizontally only (:329-333).
    #[test]
    fn move_to_push_with_flag_steers_from_vehicle_position() {
        let vehicle_id = ObjectId::new(7);
        let mut vehicle = snapshot_with_id(7);
        vehicle.position = Vector2::new(95, 100);
        let mut pusher = walking_jumper(Vector2::new(100, 100));
        pusher.action_procedure = ActionProcedure::Push;
        pusher.action_target = Some(vehicle_id);
        let mut objects = CommandObjectSnapshots::default();
        objects.insert(vehicle_id, vehicle);
        let players = HashMap::new();
        let mut definitions = HashMap::new();
        definitions.insert(
            "DEF7".to_string(),
            CommandDefinitionSnapshot {
                value: 0,
                can_chop: false,
                chop_action: None,
                constructable: false,
                grab: 1,
                ..CommandDefinitionSnapshot::default()
            },
        );

        // Target far below the vehicle's column: the vehicle position
        // override yields dx 0 and the push arm ignores dy entirely.
        let mut state = evaluated_move_to(
            &CommandRequest::new(CommandId::MoveTo)
                .with_tx(Some(95))
                .with_ty(Some(160))
                .with_data(CommandData::Integer(2)),
        );
        let ctx = command_ctx_at_frame(&pusher, &objects, &players, &definitions, 1);
        let result = state.step(&ctx);
        assert_eq!(result.status, CommandStatus::Running);
        assert!(result.operations.is_empty(), "no UnGrab with PushTarget");
        assert_eq!(
            result.update.and_then(|update| update.command_direction),
            None,
            "DFA_PUSH steers horizontally from the vehicle position only"
        );

        // Grab-only target (Grab=2) lets go even with the flag.
        definitions.get_mut("DEF7").expect("def").grab = 2;
        let mut state = evaluated_move_to(
            &CommandRequest::new(CommandId::MoveTo)
                .with_tx(Some(200))
                .with_ty(Some(100))
                .with_data(CommandData::Integer(2)),
        );
        let ctx = command_ctx_at_frame(&pusher, &objects, &players, &definitions, 1);
        let result = state.step(&ctx);
        match result.operations.first() {
            Some(CommandOperation::PushFront(request)) => {
                assert_eq!(request.id, CommandId::UnGrab, "Grab=2 (C4Command.cpp:260)");
            }
            other => panic!("expected UnGrab, got {other:?}"),
        }
    }

    #[test]
    fn move_to_push_intermediate_waypoint_steers_from_actor_position() {
        let vehicle_id = ObjectId::new(7);
        let mut vehicle = snapshot_with_id(7);
        vehicle.position = Vector2::new(95, 100);
        let mut pusher = walking_jumper(Vector2::new(100, 100));
        pusher.action_procedure = ActionProcedure::Push;
        pusher.action_target = Some(vehicle_id);
        let objects = CommandObjectSnapshots::from_iter([(vehicle_id, vehicle)]);
        let players = HashMap::new();
        let definitions = HashMap::from([(
            "DEF7".to_string(),
            CommandDefinitionSnapshot {
                value: 0,
                can_chop: false,
                chop_action: None,
                constructable: false,
                grab: 1,
                ..CommandDefinitionSnapshot::default()
            },
        )]);
        let ctx = command_ctx_at_frame(&pusher, &objects, &players, &definitions, 1);
        let mut stack = CommandStack::new();
        stack
            .push_back(
                CommandRequest::new(CommandId::MoveTo)
                    .with_tx(Some(95))
                    .with_ty(Some(160))
                    .with_data(CommandData::Integer(COMMAND_FLAG_MOVE_TO_PUSH_TARGET))
                    .with_evaluated(true),
            )
            .expect("intermediate MoveTo queues");
        stack
            .push_back(
                CommandRequest::new(CommandId::MoveTo)
                    .with_tx(Some(100))
                    .with_ty(Some(100))
                    .with_data(CommandData::Integer(COMMAND_FLAG_MOVE_TO_PUSH_TARGET))
                    .with_evaluated(true),
            )
            .expect("final MoveTo queues");

        let result = stack.step(&ctx).expect("intermediate MoveTo executes");

        assert_eq!(result.status, CommandStatus::Running);
        assert!(result.operations.is_empty(), "PushTarget keeps the grab");
        assert_eq!(
            result.update.and_then(|update| update.command_direction),
            Some(CommandDirection::Left),
            "the intermediate waypoint measures x=95 from clonk x=100, not vehicle x=95"
        );
        assert_eq!(stack.command_names(), vec!["MoveTo", "MoveTo"]);

        let mut pusher_at_waypoint = pusher.clone();
        pusher_at_waypoint.position = Vector2::new(95, 160);
        let waypoint_ctx =
            command_ctx_at_frame(&pusher_at_waypoint, &objects, &players, &definitions, 2);
        let waypoint = stack
            .step(&waypoint_ctx)
            .expect("intermediate MoveTo completes");
        assert_eq!(waypoint.status, CommandStatus::Completed);
        assert_eq!(stack.command_names(), vec!["MoveTo"]);

        let final_ctx = command_ctx_at_frame(&pusher, &objects, &players, &definitions, 3);
        let final_result = stack.step(&final_ctx).expect("final MoveTo executes");
        assert_eq!(final_result.status, CommandStatus::Running);
        assert!(
            final_result.operations.is_empty(),
            "PushTarget keeps the grab"
        );
        assert_eq!(
            final_result
                .update
                .and_then(|update| update.command_direction),
            Some(CommandDirection::Right),
            "the final waypoint uses factor-1 from vehicle x=95, not factor-3 or clonk x=100"
        );
        assert_eq!(stack.command_names(), vec!["MoveTo"]);
    }

    // C4Command::JumpControl trigger 1 (C4Command.cpp:1861-1872): target
    // in the ±(35±10)° diagonal, path free, farther than 30, 15px head
    // room -> a C4CMD_Jump goes on TOP of the MoveTo.
    #[test]
    fn move_to_noncrew_pathfinder_diagonal_free_jump_like_cpp() {
        let landscape = crate::Landscape::flat(300, 110);
        let walker = pathfinder_jumper(Vector2::new(100, 100));
        let objects = CommandObjectSnapshots::default();
        let players = HashMap::new();
        let definitions = HashMap::new();
        let ctx = jump_ctx(&walker, &objects, &players, &definitions, &landscape);

        // Angle(100,100 -> 140,43) = 90 - trunc(atan2(57,40)) = 36 — inside
        // 35±10; distance 70 > 30; sky above.
        let mut state = evaluated_move_to(
            &CommandRequest::new(CommandId::MoveTo)
                .with_tx(Some(140))
                .with_ty(Some(43)),
        );
        let result = state.step(&ctx);
        assert_eq!(result.status, CommandStatus::Running);
        assert_eq!(result.operations.len(), 1, "one jump op");
        match &result.operations[0] {
            CommandOperation::PushFront(request) => {
                assert_eq!(request.id, CommandId::Jump);
                assert_eq!((request.tx, request.ty), (Some(140), Some(43)));
            }
            other => panic!("expected jump, got {other:?}"),
        }
    }

    #[test]
    fn move_to_flight_takeoff_defers_walk_jump_until_after_callbacks() {
        let landscape = crate::Landscape::flat(300, 110);
        let mut walker = pathfinder_jumper(Vector2::new(100, 100));
        walker.physical.can_fly = 1;
        let objects = CommandObjectSnapshots::default();
        let players = HashMap::new();
        let definitions = HashMap::new();
        let ctx = jump_ctx(&walker, &objects, &players, &definitions, &landscape);
        let mut state = evaluated_move_to(
            &CommandRequest::new(CommandId::MoveTo)
                .with_tx(Some(140))
                .with_ty(Some(43)),
        );

        let takeoff = state.step(&ctx);
        assert!(
            takeoff.operations.is_empty(),
            "JumpControl waits for Fly callbacks"
        );
        assert!(matches!(
            takeoff.events.as_slice(),
            [CommandEvent::MoveToFlightControlTakeoff {
                command_instance_id: 0,
                ..
            }]
        ));

        let mut unchanged = state.clone();
        let unchanged_result = unchanged.resume_after_flight_control(&ctx);
        assert!(matches!(
            unchanged_result.operations.as_slice(),
            [CommandOperation::PushFront(request)] if request.id == CommandId::Jump
        ));

        // A Fly callback can ChangeDef and remove Pathfinder. JumpControl
        // must see that fresh state rather than queueing the stale jump.
        let mut callback_mutated = walker.clone();
        callback_mutated.pathfinder = 0;
        let mutated_ctx = jump_ctx(
            &callback_mutated,
            &objects,
            &players,
            &definitions,
            &landscape,
        );
        let mutated_result = state.resume_after_flight_control(&mutated_ctx);
        assert!(mutated_result.operations.is_empty());
    }

    #[test]
    fn detached_deferred_move_to_retains_exact_flight_tail() {
        let landscape = crate::Landscape::flat(300, 110);
        let mut walker = pathfinder_jumper(Vector2::new(100, 100));
        walker.physical_deferred = true;
        let objects = CommandObjectSnapshots::default();
        let players = HashMap::new();
        let definitions = HashMap::new();
        let ctx = jump_ctx(&walker, &objects, &players, &definitions, &landscape);
        let mut stack = CommandStack::new();
        stack
            .push_front(
                CommandRequest::new(CommandId::MoveTo)
                    .with_tx(Some(140))
                    .with_ty(Some(43))
                    .with_evaluated(true),
            )
            .expect("MoveTo queues");

        let first = stack.execute_front(&ctx).expect("physical read stages");
        let command_instance_id = match first.events.as_slice() {
            [CommandEvent::ResolveCommandPhysical {
                command_instance_id,
                ..
            }] => *command_instance_id,
            other => panic!("unexpected first events: {other:?}"),
        };
        assert_ne!(command_instance_id, 0);

        stack.clear();
        let mut physical = PhysicalInfo::default();
        physical.can_fly = 1;
        let takeoff = stack
            .execute_pending_physical(
                &ctx,
                crate::PhysicsSettings::default().gravity_as_c4fixed(),
                command_instance_id,
                physical,
            )
            .expect("detached physical MoveTo resumes");
        assert!(takeoff.operations.is_empty());
        assert!(matches!(
            takeoff.events.as_slice(),
            [CommandEvent::MoveToFlightControlTakeoff {
                command_instance_id: event_id,
                ..
            }] if *event_id == command_instance_id
        ));
        assert_eq!(stack.detached_move_to_flights.len(), 1);

        let mut callback_mutated = walker.clone();
        callback_mutated.pathfinder = 0;
        let mutated_ctx = jump_ctx(
            &callback_mutated,
            &objects,
            &players,
            &definitions,
            &landscape,
        );
        let resumed = stack
            .execute_pending_move_to_flight(&mutated_ctx, command_instance_id)
            .expect("exact detached MoveTo flight tail resumes");
        assert!(resumed.operations.is_empty());
        assert!(stack.detached_move_to_flights.is_empty());
    }

    // Trigger 3 (C4Command.cpp:1896-1908): CNAT_RIGHT wall contact with
    // the target up the wall (angle ≈ ±80°) jumps without a path check.
    #[test]
    fn move_to_noncrew_pathfinder_low_side_contact_jump_like_cpp() {
        let landscape = crate::Landscape::flat(300, 110);
        let mut walker = pathfinder_jumper(Vector2::new(100, 100));
        walker.contact = crate::CNAT_RIGHT;
        let objects = CommandObjectSnapshots::default();
        let players = HashMap::new();
        let definitions = HashMap::new();
        let ctx = jump_ctx(&walker, &objects, &players, &definitions, &landscape);

        // Angle(100,100 -> 140,93) = 90 - trunc(atan2(7,40)) = 81; 81-80=1
        // inside ±50.
        let mut state = evaluated_move_to(
            &CommandRequest::new(CommandId::MoveTo)
                .with_tx(Some(140))
                .with_ty(Some(93)),
        );
        let result = state.step(&ctx);
        assert_eq!(
            result.operations.len(),
            1,
            "right-contact jump fires (left mirror uses angle+80)"
        );
        match &result.operations[0] {
            CommandOperation::PushFront(request) => assert_eq!(request.id, CommandId::Jump),
            other => panic!("expected jump, got {other:?}"),
        }
    }

    // Trigger 2 (C4Command.cpp:1874-1893): target overhead on a ledge —
    // side-move first (pushed on top), then the jump.
    #[test]
    fn move_to_noncrew_pathfinder_high_angle_side_move_jump_like_cpp() {
        // Ledge: surface 110 everywhere except a plateau (top 75) right of
        // the target.
        let mut surface = vec![110i32; 300];
        for column in surface.iter_mut().take(190).skip(150) {
            *column = 75;
        }
        let landscape =
            crate::Landscape::with_default_material(300, surface, None).expect("landscape");
        let walker = pathfinder_jumper(Vector2::new(140, 100));
        let objects = CommandObjectSnapshots::default();
        let players = HashMap::new();
        let definitions = HashMap::new();
        let ctx = jump_ctx(&walker, &objects, &players, &definitions, &landscape);

        // Target on the plateau edge: (148,72): angle = trunc(atan2(28,8))
        // = 74 -> 90-74 = 16?? — angle must sit within ±30 of straight up.
        // (148,72): dx 8, dy -28 -> angle 90-74=16 NOT <= 30? inside(16,-30,30) yes.
        // cy - ty = 28 inside 10..40. SolidOnWhichSide(148,72): plateau
        // solid at x>=150 -> +1 -> side point x = 140 - 23 = 117 (clear
        // ground), adjust drops it to the 110 surface (|dy|<=20 from 100
        // fails?) — pick ty=75 edge instead for a shallower drop.
        let mut state = evaluated_move_to(
            &CommandRequest::new(CommandId::MoveTo)
                .with_tx(Some(148))
                .with_ty(Some(72)),
        );
        let result = state.step(&ctx);
        assert_eq!(
            result.operations.len(),
            2,
            "side-move lands on top of the jump (AddCommand pushes front twice)"
        );
        match (&result.operations[0], &result.operations[1]) {
            (CommandOperation::PushFront(jump), CommandOperation::PushFront(side_move)) => {
                assert_eq!(jump.id, CommandId::Jump);
                assert_eq!(side_move.id, CommandId::MoveTo);
                assert_eq!(side_move.update_interval, 50);
            }
            other => panic!("expected jump + side move, got {other:?}"),
        }
    }

    #[test]
    fn move_to_idle_fails_after_arrival_check_and_feeds_base_failures() {
        let mut idle = snapshot_with_id(1);
        idle.position = Vector2::new(100, 100);
        idle.action_idle = true;
        idle.action_procedure = ActionProcedure::Undefined;
        let objects = CommandObjectSnapshots::default();
        let players = HashMap::new();
        let definitions = HashMap::new();

        let mut stack = CommandStack::new();
        stack
            .push_back(
                CommandRequest::new(CommandId::Wait)
                    .with_update_interval(50)
                    .with_mode(CommandMode::Base),
            )
            .expect("base Wait queues");
        stack
            .push_front(
                CommandRequest::new(CommandId::MoveTo)
                    .with_tx(Some(200))
                    .with_ty(Some(100))
                    .with_evaluated(true)
                    .with_mode(CommandMode::SilentSub),
            )
            .expect("MoveTo queues");
        let ctx = command_ctx_at_frame(&idle, &objects, &players, &definitions, 1);
        let result = stack.execute_front(&ctx).expect("MoveTo executes");
        assert_eq!(result.status, CommandStatus::Failed);
        assert_eq!(stack.entries[1].failures, 1);
        assert!(
            result.update.is_none(),
            "idle failure itself does not steer"
        );

        // Native arrival precedes Action.Act<=ActIdle, so the same idle
        // object succeeds when it is already inside the target range.
        let mut arrived = evaluated_move_to(
            &CommandRequest::new(CommandId::MoveTo)
                .with_tx(Some(100))
                .with_ty(Some(100)),
        );
        let result = arrived.step(&ctx);
        assert_eq!(result.status, CommandStatus::Completed);

        // A real ActMap entry may also be named "Idle". The exact bit, not
        // the string, distinguishes it from the built-in inactive slot.
        let mut active_idle = idle.clone();
        active_idle.action_idle = false;
        let ctx = command_ctx_at_frame(&active_idle, &objects, &players, &definitions, 1);
        let mut moving = evaluated_move_to(
            &CommandRequest::new(CommandId::MoveTo)
                .with_tx(Some(200))
                .with_ty(Some(100)),
        );
        assert_eq!(moving.step(&ctx).status, CommandStatus::Running);
    }

    #[test]
    fn move_to_walk_vertical_offset_leaves_command_direction_untouched() {
        let mut walker = walking_jumper(Vector2::new(100, 100));
        walker.command_direction = CommandDirection::Left;
        let objects = CommandObjectSnapshots::default();
        let players = HashMap::new();
        let definitions = HashMap::new();
        let ctx = command_ctx_at_frame(&walker, &objects, &players, &definitions, 1);
        let mut state = evaluated_move_to(
            &CommandRequest::new(CommandId::MoveTo)
                .with_tx(Some(100))
                .with_ty(Some(140)),
        );

        let result = state.step(&ctx);

        assert_eq!(result.status, CommandStatus::Running);
        assert!(
            result
                .update
                .as_ref()
                .and_then(|update| update.command_direction)
                .is_none(),
            "DFA_WALK has no vertical or catch-all ComDir assignment"
        );
    }

    #[test]
    fn follow_completes_for_unselected_crew() {
        let follower_id = ObjectId::new(1);
        let target_id = ObjectId::new(2);

        let mut follower = snapshot_with_id(follower_id.as_u64());
        follower.crew_member = true;
        follower.owner = 42;
        follower.selected = false;

        let mut target = snapshot_with_id(target_id.as_u64());
        target.position = Vector2::new(20, 0);
        target.crew_member = true;

        let mut objects = CommandObjectSnapshots::default();
        objects.insert(follower.id, follower.clone());
        objects.insert(target.id, target);

        let players: HashMap<i32, CommandPlayerSnapshot> = HashMap::new();
        let definitions: HashMap<DefinitionId, CommandDefinitionSnapshot> = HashMap::new();
        let ctx = CommandRuntimeContext {
            position: follower.position,
            ..command_ctx_at_frame(
                objects.get(&follower_id).expect("follower present"),
                &objects,
                &players,
                &definitions,
                0,
            )
        };

        let mut state = FollowState::from_request(
            &CommandRequest::new(CommandId::Follow).with_target(Some(target_id)),
        )
        .expect("state created");

        let result = state.step(&ctx);
        assert_eq!(result.status, CommandStatus::Completed);
    }

    #[test]
    fn follow_self_remains_running_and_preserves_cpp_command_lifetime() {
        let follower_id = ObjectId::new(2);
        let mut follower = snapshot_with_id(follower_id.as_u64());
        follower.crew_member = true;
        follower.owner = 42;
        follower.selected = true;
        follower.action_procedure = ActionProcedure::Walk;
        follower.command_direction = CommandDirection::DownRight;

        let objects = CommandObjectSnapshots::from_iter([(follower_id, follower)]);
        let players = HashMap::new();
        let definitions = HashMap::new();
        let follower = objects.get(&follower_id).expect("follower present");
        let ctx = command_ctx_at_frame(follower, &objects, &players, &definitions, 0);

        let mut stack = CommandStack::new();
        stack
            .push_back(CommandRequest::new(CommandId::Follow).with_target(Some(follower_id)))
            .expect("self-Follow queues");
        stack
            .push_back(CommandRequest::new(CommandId::Wait))
            .expect("trailing command queues");

        for _ in 0..2 {
            let result = stack.step(&ctx).expect("self-Follow executes");
            assert_eq!(result.status, CommandStatus::Running);
            assert!(result.update.is_none());
            assert!(result.operations.is_empty());
            assert_eq!(stack.command_names(), ["Follow", "Wait"]);
            let views = stack.command_views();
            assert_eq!(views[0].target, Some(follower_id));
            assert!(!views[0].finished);
            assert!(stack.take_successful_finishes().is_empty());
        }
    }

    #[test]
    fn follow_enters_the_targets_container() {
        let follower_id = ObjectId::new(3);
        let target_id = ObjectId::new(4);
        let hut_id = ObjectId::new(5);

        let mut follower = snapshot_with_id(follower_id.as_u64());
        follower.position = Vector2::new(10, 10);
        follower.crew_member = true;
        follower.owner = 1;
        follower.selected = true;
        follower.command_direction = CommandDirection::Left;

        let mut target = snapshot_with_id(target_id.as_u64());
        target.container = Some(hut_id);

        let mut hut = snapshot_with_id(hut_id.as_u64());
        hut.position = Vector2::new(10, 10);
        hut.shape = DefinitionRect::new(0, 0, 20, 20);
        hut.entrance = Some(hut.shape);
        hut.ocf = ocf::ENTRANCE | ocf::AVAILABLE;
        hut.entrance_status = true;
        hut.category = CATEGORY_STRUCTURE;

        let objects = CommandObjectSnapshots::from_iter([(follower_id, follower), (target_id, target), (hut_id, hut)]);
        let players = HashMap::new();
        let definitions = HashMap::new();
        let follower = objects.get(&follower_id).expect("follower present");
        let ctx = command_ctx_at_frame(follower, &objects, &players, &definitions, 0);
        let mut follow = FollowState::from_request(
            &CommandRequest::new(CommandId::Follow).with_target(Some(target_id)),
        )
        .expect("Follow state");

        let result = follow.step(&ctx);
        assert_eq!(result.status, CommandStatus::Running);
        assert!(result.update.is_none(), "Follow does not stop before Enter");
        let enter = pushed_request(&result.operations, CommandId::Enter);
        assert_eq!(enter.target, Some(hut_id));
        assert_eq!(enter.update_interval, 50);
        assert_eq!(enter.mode, CommandMode::SilentSub);

        // The requested Enter is immediately executable at the hut and
        // emits the ordered containment event used by the live engine.
        let mut enter_state = EnterState::from_request(&enter).expect("Enter state");
        let entered = enter_state.step(&ctx);
        assert_eq!(entered.status, CommandStatus::Completed);
        assert_eq!(
            entered.events,
            vec![CommandEvent::EnterObject {
                object_id: follower_id,
                container_id: hut_id,
            }]
        );

        let mut contained_objects = objects.clone();
        contained_objects
            .get_mut(&follower_id)
            .expect("follower present")
            .container = Some(ObjectId::new(9));
        let contained_follower = contained_objects
            .get(&follower_id)
            .expect("follower present");
        let contained_ctx = command_ctx_at_frame(
            contained_follower,
            &contained_objects,
            &players,
            &definitions,
            1,
        );
        let exit_result = follow.step(&contained_ctx);
        assert!(exit_result.update.is_none());
        let exit = pushed_request(&exit_result.operations, CommandId::Exit);
        assert_eq!(exit.target, None);
        assert_eq!(exit.update_interval, 50);
        assert_eq!(exit.mode, CommandMode::SilentSub);
    }

    #[test]
    fn follow_grabs_copies_and_releases_the_targets_vehicle() {
        let follower_id = ObjectId::new(6);
        let target_id = ObjectId::new(7);
        let lorry_id = ObjectId::new(8);

        let mut follower = snapshot_with_id(follower_id.as_u64());
        follower.crew_member = true;
        follower.owner = 1;
        follower.selected = true;
        follower.command_direction = CommandDirection::Left;

        let mut target = snapshot_with_id(target_id.as_u64());
        target.action_procedure = ActionProcedure::Push;
        target.action_target = Some(lorry_id);
        target.command_direction = CommandDirection::Right;

        let lorry = snapshot_with_id(lorry_id.as_u64());
        let mut objects = CommandObjectSnapshots::from_iter([
            (follower_id, follower),
            (target_id, target),
            (lorry_id, lorry),
        ]);
        let players = HashMap::new();
        let definitions = HashMap::new();
        let mut follow = FollowState::from_request(
            &CommandRequest::new(CommandId::Follow).with_target(Some(target_id)),
        )
        .expect("Follow state");

        let follower = objects.get(&follower_id).expect("follower present");
        let ctx = command_ctx_at_frame(follower, &objects, &players, &definitions, 0);
        let grab_result = follow.step(&ctx);
        assert_eq!(grab_result.status, CommandStatus::Running);
        assert!(grab_result.update.is_none());
        let grab = pushed_request(&grab_result.operations, CommandId::Grab);
        assert_eq!(grab.target, Some(lorry_id));
        assert_eq!(grab.update_interval, 0);
        assert_eq!(grab.mode, CommandMode::SilentSub);

        let follower = objects.get_mut(&follower_id).expect("follower present");
        follower.action_procedure = ActionProcedure::Push;
        follower.action_target = Some(lorry_id);
        follower.command_direction = CommandDirection::Left;
        let follower = objects.get(&follower_id).expect("follower present");
        let ctx = command_ctx_at_frame(follower, &objects, &players, &definitions, 1);
        let copy_result = follow.step(&ctx);
        assert!(copy_result.operations.is_empty());
        assert_eq!(
            copy_result
                .update
                .and_then(|update| update.command_direction),
            Some(CommandDirection::Right)
        );

        objects
            .get_mut(&follower_id)
            .expect("follower present")
            .command_direction = CommandDirection::Right;
        objects
            .get_mut(&target_id)
            .expect("target present")
            .command_direction = CommandDirection::DownRight;
        let follower = objects.get(&follower_id).expect("follower present");
        let ctx = command_ctx_at_frame(follower, &objects, &players, &definitions, 2);
        let next_copy = follow.step(&ctx);
        assert_eq!(
            next_copy.update.and_then(|update| update.command_direction),
            Some(CommandDirection::DownRight),
            "Follow copies the pushed target's current ComDir every evaluation"
        );

        objects
            .get_mut(&target_id)
            .expect("target present")
            .action_procedure = ActionProcedure::Walk;
        let follower = objects.get(&follower_id).expect("follower present");
        let ctx = command_ctx_at_frame(follower, &objects, &players, &definitions, 3);
        let ungrab_result = follow.step(&ctx);
        assert_eq!(ungrab_result.status, CommandStatus::Running);
        assert!(ungrab_result.update.is_none());
        let ungrab = pushed_request(&ungrab_result.operations, CommandId::UnGrab);
        assert_eq!(ungrab.target, None);
        assert_eq!(ungrab.update_interval, 0);
        assert_eq!(ungrab.mode, CommandMode::SilentSub);
    }

    #[test]
    fn follow_requests_move_when_out_of_range() {
        let follower_id = ObjectId::new(10);
        let target_id = ObjectId::new(20);

        let mut follower = snapshot_with_id(follower_id.as_u64());
        follower.crew_member = true;
        follower.owner = 1;
        follower.selected = true;
        follower.command_direction = CommandDirection::Left;

        let mut target = snapshot_with_id(target_id.as_u64());
        target.position = Vector2::new(100, 20);
        target.crew_member = true;
        target.alive = false;

        let mut objects = CommandObjectSnapshots::default();
        objects.insert(follower.id, follower.clone());
        objects.insert(target.id, target);

        let players: HashMap<i32, CommandPlayerSnapshot> = HashMap::new();
        let definitions: HashMap<DefinitionId, CommandDefinitionSnapshot> = HashMap::new();
        let ctx = CommandRuntimeContext {
            position: follower.position,
            ..command_ctx_at_frame(
                objects.get(&follower_id).expect("follower present"),
                &objects,
                &players,
                &definitions,
                0,
            )
        };

        let mut state = FollowState::from_request(
            &CommandRequest::new(CommandId::Follow).with_target(Some(target_id)),
        )
        .expect("state created");

        let result = state.step(&ctx);
        assert_eq!(result.status, CommandStatus::Running);
        assert!(
            result.update.is_none(),
            "Follow must not force ComDir Stop before its MoveTo child"
        );
        assert_eq!(result.operations.len(), 1);
        match &result.operations[0] {
            CommandOperation::PushFront(request) => {
                assert_eq!(request.id, CommandId::MoveTo);
                assert_eq!(request.target, None);
                assert_eq!((request.tx, request.ty), (Some(100), Some(20)));
                assert_eq!(request.update_interval, 10);
                assert_eq!(request.mode, CommandMode::SilentSub);
            }
            other => panic!("expected move request, got {:?}", other),
        }
        let first_move = pushed_request(&result.operations, CommandId::MoveTo);
        let reissued = state.step(&ctx);
        assert_eq!(
            pushed_request(&reissued.operations, CommandId::MoveTo),
            first_move,
            "Follow reissues MoveTo on its next execution"
        );

        objects.get_mut(&target_id).expect("target present").status = ObjectStatus::Deleted;
        let follower = objects.get(&follower_id).expect("follower present");
        let removed_ctx = command_ctx_at_frame(follower, &objects, &players, &definitions, 1);
        let removed = state.step(&removed_ctx);
        assert_eq!(removed.status, CommandStatus::Running);
        assert!(removed.update.is_none());
        assert_eq!(
            pushed_request(&removed.operations, CommandId::MoveTo),
            first_move,
            "a detached iExec command retains its raw target pointer"
        );

        let mut cleared = CommandStack::new();
        cleared
            .push_front(CommandRequest::new(CommandId::Follow).with_target(Some(target_id)))
            .expect("Follow queues");
        assert!(cleared.clear_object_reference(target_id));
        assert_eq!(
            cleared
                .execute_front(&removed_ctx)
                .map(|result| result.status),
            Some(CommandStatus::Failed)
        );
    }

    // FnGetCommand serves the LIVE C4Command fields (C4Script.cpp:
    // 926-945) — the snapshot stack (which backs the world-context views
    // every frame) must carry the same elements, not nil.
    #[test]
    fn snapshot_command_views_expose_live_elements() {
        let mut stack = CommandStack::new();
        stack
            .push_front(
                CommandRequest::new(CommandId::MoveTo)
                    .with_tx(Some(140))
                    .with_ty(Some(90))
                    .with_data(CommandData::Integer(1)),
            )
            .expect("push");

        let snapshot = stack.snapshot();
        let views = snapshot.command_views();
        assert_eq!(views.len(), 1);
        assert_eq!(views[0].name, "MoveTo");
        assert_eq!(
            (views[0].tx, views[0].ty),
            (Some(140), Some(90)),
            "restored views keep Tx/Ty (C4Script.cpp:934-937)"
        );
        assert_eq!(views[0].data, CommandData::Integer(1));

        let mut restored = CommandStack::new();
        restored.restore_from_snapshot(&snapshot);
        let views = restored.command_views();
        assert_eq!(
            (views[0].tx, views[0].ty),
            (Some(140), Some(90)),
            "a restored stack keeps its elements"
        );
    }

    // The element view follows the InitEvaluation rewrites: MoveTo's
    // Target folds into Tx/Ty and clears (C4Command.cpp:1637), so
    // GetCommand element 1 goes nil and element 2 reads the absorbed X.
    #[test]
    fn command_views_follow_move_to_target_absorption() {
        let walker = walking_jumper(Vector2::new(100, 100));
        let target_id = ObjectId::new(9);
        let mut target = snapshot_with_id(9);
        target.position = Vector2::new(200, 100);
        let mut objects = CommandObjectSnapshots::default();
        objects.insert(target_id, target);
        let players = HashMap::new();
        let definitions = HashMap::new();

        let mut stack = CommandStack::new();
        stack
            .push_front(CommandRequest::new(CommandId::MoveTo).with_target(Some(target_id)))
            .expect("push");

        let ctx = command_ctx_at_frame(&walker, &objects, &players, &definitions, 1);
        let _ = stack.step(&ctx); // InitEvaluation execute

        let views = stack.command_views();
        assert_eq!(views[0].target, None, "Target cleared (C4Command.cpp:1637)");
        assert_eq!(
            (views[0].tx, views[0].ty),
            (Some(200), Some(100)),
            "Tx/Ty absorbed the target position"
        );

        // The same live values survive the snapshot round-trip.
        let views = stack.snapshot().command_views();
        assert_eq!(views[0].target, None);
        assert_eq!((views[0].tx, views[0].ty), (Some(200), Some(100)));
    }

    // Acquire's element view changes to the defaulted 500/250 search range
    // on its evaluation-only first Execute (C4Command.cpp:1666-1670).
    #[test]
    fn acquire_init_evaluation_consumes_first_execute_and_defaults_ranges() {
        let mut stack = CommandStack::new();
        stack
            .push_front(
                CommandRequest::new(CommandId::Acquire).with_data(CommandData::Text("WOOD".into())),
            )
            .expect("push");

        let views = stack.command_views();
        assert_eq!((views[0].tx, views[0].ty), (None, None));

        let actor = snapshot_with_id(7);
        let objects = CommandObjectSnapshots::default();
        let players = HashMap::new();
        let definitions = HashMap::new();
        let ctx = command_ctx_at_frame(&actor, &objects, &players, &definitions, 1);
        let evaluation = stack.step(&ctx).expect("Acquire evaluates");
        assert_eq!(evaluation.status, CommandStatus::Running);
        assert!(evaluation.update.is_none());
        assert!(evaluation.events.is_empty());
        assert!(evaluation.operations.is_empty());

        let views = stack.command_views();
        assert_eq!(
            (views[0].tx, views[0].ty),
            (Some(500), Some(250)),
            "defaulted search range is live-visible (C4Command.cpp:1668-1669)"
        );
        assert_eq!(views[0].data, CommandData::Text("WOOD".into()));

        let views = stack.snapshot().command_views();
        assert_eq!((views[0].tx, views[0].ty), (Some(500), Some(250)));

        let handler = stack.step(&ctx).expect("Acquire handler executes");
        assert!(matches!(
            handler.events.as_slice(),
            [CommandEvent::ControlCommandAcquire { .. }]
        ));
    }

    #[test]
    fn wait_takes_its_duration_from_data_then_tx() {
        // C4CMD_Wait InitEvaluation (C4Command.cpp:1659-1663): a nonzero
        // Data overrides the update interval, else a nonzero Tx does. The
        // dragon waits via SetCommand(this(), "Wait", 0,0,0,0, 10) — data
        // slot 10, no interval (Fantasy.c4d Dragon.c4d Script.c:1649).
        let from_data = CommandRequest::new(CommandId::Wait).with_data(CommandData::Integer(10));
        assert_eq!(
            WaitState::from_request(&from_data).remaining,
            Some(10),
            "Data overrides the interval"
        );

        let from_tx = CommandRequest::new(CommandId::Wait).with_tx(Some(7));
        assert_eq!(
            WaitState::from_request(&from_tx).remaining,
            Some(7),
            "Tx is the fallback duration"
        );

        let from_interval = CommandRequest::new(CommandId::Wait)
            .with_update_interval(3)
            .with_data(CommandData::Integer(10));
        assert_eq!(
            WaitState::from_request(&from_interval).remaining,
            Some(10),
            "Data wins even when an interval is present"
        );

        let negative_data =
            CommandRequest::new(CommandId::Wait).with_data(CommandData::Integer(-7));
        assert_eq!(
            WaitState::from_request(&negative_data).remaining,
            Some(-7),
            "native Wait installs signed nonzero Data verbatim"
        );

        let negative_tx = CommandRequest::new(CommandId::Wait).with_tx(Some(-9));
        assert_eq!(
            WaitState::from_request(&negative_tx).remaining,
            Some(-9),
            "native Wait installs signed nonzero Tx verbatim"
        );
    }

    #[test]
    fn negative_wait_intervals_survive_evaluation_execution_and_snapshot_restore() {
        let actor = snapshot_with_id(52);
        let objects = CommandObjectSnapshots::default();
        let players = HashMap::new();
        let definitions = HashMap::new();
        let ctx = command_ctx_at_frame(&actor, &objects, &players, &definitions, 0);

        let mut direct = CommandStack::new();
        direct
            .push_front(CommandRequest::new(CommandId::Wait).with_update_interval(-4))
            .expect("Wait queues");
        assert_eq!(
            direct.step(&ctx).expect("direct Wait evaluates").status,
            CommandStatus::Running
        );
        assert_eq!(direct.legacy_save_commands()[0].update_interval, -4);
        assert_eq!(
            direct.step(&ctx).expect("direct Wait executes").status,
            CommandStatus::Running
        );
        assert_eq!(direct.snapshot().commands[0].update_interval, Some(-4));

        let mut stack = CommandStack::new();
        stack
            .push_front(
                CommandRequest::new(CommandId::Wait)
                    .with_update_interval(-4)
                    .with_data(CommandData::Integer(-7)),
            )
            .expect("Wait queues");

        let evaluation = stack.step(&ctx).expect("Wait evaluates");
        assert_eq!(evaluation.status, CommandStatus::Running);
        assert_eq!(
            stack.legacy_save_commands()[0].update_interval,
            -7,
            "InitEvaluation replaces the raw interval with signed Data"
        );

        let execution = stack.step(&ctx).expect("negative Wait executes");
        assert_eq!(execution.status, CommandStatus::Running);
        assert_eq!(stack.legacy_save_commands()[0].update_interval, -7);

        let snapshot = stack.snapshot();
        assert_eq!(snapshot.commands[0].update_interval, Some(-7));
        let mut restored = CommandStack::new();
        restored.restore_from_snapshot(&snapshot);
        assert_eq!(restored.legacy_save_commands()[0].update_interval, -7);
        assert_eq!(
            restored
                .step(&ctx)
                .expect("restored negative Wait executes")
                .status,
            CommandStatus::Running
        );
        assert_eq!(restored.legacy_save_commands()[0].update_interval, -7);
    }

    #[test]
    fn wait_stops_dig_and_completes_after_interval() {
        let actor_id = ObjectId::new(50);
        let mut actor = snapshot_with_id(actor_id.as_u64());
        actor.action_procedure = ActionProcedure::Dig;
        actor.command_direction = CommandDirection::Left;

        let mut objects = CommandObjectSnapshots::default();
        objects.insert(actor.id, actor.clone());

        let players: HashMap<i32, CommandPlayerSnapshot> = HashMap::new();
        let definitions: HashMap<DefinitionId, CommandDefinitionSnapshot> = HashMap::new();

        let mut stack = CommandStack::new();
        stack
            .push_front(CommandRequest::new(CommandId::Wait).with_update_interval(3))
            .expect("Wait queues");

        let ctx0 = CommandRuntimeContext {
            position: actor.position,
            ..command_ctx_at_frame(
                objects.get(&actor_id).expect("actor present"),
                &objects,
                &players,
                &definitions,
                0,
            )
        };

        let result0 = stack.step(&ctx0).expect("Wait executes");
        assert_eq!(result0.status, CommandStatus::Running);
        assert!(
            result0.update.is_none(),
            "InitEvaluation consumes the first Execute before Wait stops Dig"
        );

        let ctx1 = CommandRuntimeContext {
            position: actor.position,
            ..command_ctx_at_frame(
                objects.get(&actor_id).expect("actor present"),
                &objects,
                &players,
                &definitions,
                1,
            )
        };

        let result1 = stack.step(&ctx1).expect("Wait executes again");
        assert_eq!(result1.status, CommandStatus::Running);
        let update1 = result1
            .update
            .expect("the post-evaluation Wait should stop digging");
        assert_eq!(update1.command_direction, Some(CommandDirection::Stop));
        let action_update = update1.action.expect("wait should reset the action");
        assert_eq!(action_update.name.as_deref(), Some("Idle"));

        let ctx2 = CommandRuntimeContext {
            position: actor.position,
            ..command_ctx_at_frame(
                objects.get(&actor_id).expect("actor present"),
                &objects,
                &players,
                &definitions,
                2,
            )
        };

        let result2 = stack.step(&ctx2).expect("Wait interval expires");
        assert_eq!(result2.status, CommandStatus::Completed);
        assert!(stack.is_empty());
    }

