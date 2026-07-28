// Contiguous slice 2 of 7 of the `command/tests` battery, spliced by
// `include!` from the parent module so every test id is unchanged.

    #[test]
    fn wait_data_duration_completes_on_eleventh_execute() {
        let actor = snapshot_with_id(51);
        let objects = HashMap::new();
        let players = HashMap::new();
        let definitions = HashMap::new();
        let ctx = command_ctx_at_frame(&actor, &objects, &players, &definitions, 0);
        let mut stack = CommandStack::new();
        stack
            .push_front(CommandRequest::new(CommandId::Wait).with_data(CommandData::Integer(10)))
            .expect("Wait queues");

        for execution in 1..=10 {
            let result = stack.step(&ctx).expect("Wait remains queued");
            assert_eq!(
                result.status,
                CommandStatus::Running,
                "Data=10 must still be waiting on execution {execution}"
            );
            assert_eq!(stack.len(), 1);
            if execution == 1 {
                assert_eq!(stack.snapshot().commands[0].update_interval, Some(10));
            }
        }

        let completed = stack.step(&ctx).expect("Wait interval expires");
        assert_eq!(completed.status, CommandStatus::Completed);
        assert!(stack.is_empty());
    }

    #[test]
    fn get_pursuit_moves_with_the_random_offset_like_cpp() {
        // C4Command::Get outside pursuit (C4Command.cpp:1288-1290): target
        // not in collection range and not in jump range -> AddCommand
        // MoveTo(Target->x + Random(15) - 7, Target->y, 25). The Random
        // draw advances the synced ledger.
        let actor_id = ObjectId::new(501);
        let target_id = ObjectId::new(502);
        let mut actor = snapshot_with_id(actor_id.as_u64());
        actor.position = Vector2::new(100, 100);
        let mut target = snapshot_with_id(target_id.as_u64());
        target.position = Vector2::new(300, 100);
        target.collectible = true;
        target.construction = FULL_CON;

        let mut objects = HashMap::new();
        objects.insert(actor_id, actor);
        objects.insert(target_id, target);
        let players = HashMap::new();
        let definitions: HashMap<DefinitionId, CommandDefinitionSnapshot> = HashMap::new();
        let rng = std::cell::RefCell::new(crate::LcgRng::seed_from_u64(7));
        let expected_offset = {
            let mut probe = rng.borrow().clone();
            probe.random(15) - 7
        };
        let actor_snapshot = objects.get(&actor_id).expect("actor present");
        let ctx = CommandRuntimeContext {
            rng: Some(&rng),
            landscape: None,
            frame: 0,
            position: actor_snapshot.position,
            object: actor_snapshot,
            objects: &objects,
            players: &players,
            definitions: &definitions,
            structures_need_energy: false,
            base_buy_enabled: true,
            base_sell_enabled: true,
            transfer_zones: &EMPTY_TRANSFER_ZONES,
        };

        let parent_request = CommandRequest::new(CommandId::Get).with_target(Some(target_id));
        let mut state = GetState::from_request(&parent_request).expect("state created");
        let result = state.step(&ctx);

        let move_to = result
            .operations
            .iter()
            .find_map(|operation| match operation {
                CommandOperation::PushFront(request) if request.id == CommandId::MoveTo => {
                    Some(request)
                }
                _ => None,
            })
            .expect("pursuit pushes MoveTo");
        assert_eq!(
            move_to.tx,
            Some(300 + expected_offset),
            "MoveTo x = Target->x + Random(15) - 7 (C4Command.cpp:1290)"
        );
        assert_eq!(move_to.ty, Some(100), "MoveTo y = Target->y");
        assert_eq!(move_to.update_interval, 25, "iUpdateInterval 25");
        assert_eq!(
            rng.borrow().count,
            crate::LcgRng::seed_from_u64(7).count + 1,
            "exactly one ledger draw"
        );
        let reissued = state.step(&ctx);
        assert_eq!(
            pushed_request(&reissued.operations, CommandId::MoveTo).update_interval,
            25,
            "Get reissues its pursuit MoveTo on the next execution"
        );
        assert_eq!(
            rng.borrow().count,
            crate::LcgRng::seed_from_u64(7).count + 2,
            "reissued pursuit performs the next C++ random-offset draw"
        );
        assert_silent_child_failure_propagates(parent_request, move_to.clone(), &ctx);
    }

    #[test]
    fn get_side_jump_preserves_count_and_collection_limit_stack_order() {
        // C4Command::Get queues Jump, optional Drop, side MoveTo, then the
        // unconditional random-offset MoveTo. AddCommand pushes each entry
        // to the front, reversing that call order on the live stack
        // (C4Command.cpp:1272-1290).
        let actor_id = ObjectId::new(511);
        let target_id = ObjectId::new(512);
        let mut actor = snapshot_with_id(actor_id.as_u64());
        actor.position = Vector2::new(100, 100);
        actor.contents = vec![ObjectId::new(513), ObjectId::new(514)];
        let actor_definition = actor.definition_id.clone();

        let mut target = snapshot_with_id(target_id.as_u64());
        target.position = Vector2::new(100, 60);
        target.collectible = true;
        target.construction = FULL_CON;

        let mut first_content = snapshot_with_id(513);
        first_content.container = Some(actor_id);
        let mut second_content = snapshot_with_id(514);
        second_content.container = Some(actor_id);
        let objects = HashMap::from([
            (actor_id, actor),
            (target_id, target),
            (first_content.id, first_content),
            (second_content.id, second_content),
        ]);
        let players = HashMap::new();
        let definitions = HashMap::from([(
            actor_definition,
            CommandDefinitionSnapshot {
                collection_limit: 2,
                ..CommandDefinitionSnapshot::default()
            },
        )]);
        let rng = std::cell::RefCell::new(crate::LcgRng::seed_from_u64(17));
        let (expected_side_x, expected_random_x) = {
            let mut probe = rng.borrow().clone();
            let side = if probe.random(2) != 0 { -1 } else { 1 };
            (100 + side * 40, 100 + probe.random(15) - 7)
        };
        let actor_snapshot = objects.get(&actor_id).expect("actor present");
        let ctx = CommandRuntimeContext {
            rng: Some(&rng),
            landscape: None,
            frame: 0,
            position: actor_snapshot.position,
            object: actor_snapshot,
            objects: &objects,
            players: &players,
            definitions: &definitions,
            structures_need_energy: false,
            base_buy_enabled: true,
            base_sell_enabled: true,
            transfer_zones: &EMPTY_TRANSFER_ZONES,
        };

        let mut stack = CommandStack::new();
        stack
            .push_back(CommandRequest::new(CommandId::Get).with_target(Some(target_id)))
            .expect("Get queues");
        let result = stack.step(&ctx).expect("Get executes");

        assert_eq!(result.status, CommandStatus::Running);
        assert_eq!(
            stack.command_names(),
            vec!["MoveTo", "MoveTo", "Drop", "Jump", "Get"]
        );
        let views = stack.command_views();
        assert_eq!(
            (views[0].tx, views[0].ty),
            (Some(expected_random_x), Some(60))
        );
        assert_eq!(
            (views[1].tx, views[1].ty),
            (Some(expected_side_x), Some(100))
        );
        assert_eq!(stack.entries[0].update_interval, 25);
        assert_eq!(stack.entries[1].update_interval, 50);
        assert_eq!(views[2].target, None, "CollectionLimit Drop is targetless");
        assert_eq!(
            (views[3].tx, views[3].ty),
            (Some(0), Some(0)),
            "plain Get forwards its zero count/Ty instead of target coordinates"
        );
        assert_eq!(
            rng.borrow().count,
            crate::LcgRng::seed_from_u64(17).count + 2,
            "side selection and pursuit offset consume exactly two draws"
        );

        let mut counted = GetState::from_request(
            &CommandRequest::new(CommandId::Get)
                .with_target(Some(target_id))
                .with_tx(Some(3))
                .with_ty(Some(17)),
        )
        .expect("counted Get state");
        let counted_result = counted.step(&ctx);
        let jump = pushed_request(&counted_result.operations, CommandId::Jump);
        assert_eq!((jump.tx, jump.ty), (Some(3), Some(17)));
    }

    #[test]
    fn get_in_solid_uses_inclusive_dig_range_and_reissues_dig() {
        let actor_id = ObjectId::new(503);
        let target_id = ObjectId::new(504);
        let mut actor = snapshot_with_id(actor_id.as_u64());
        // The flat-landscape scans below both select (90,98). This is the
        // inclusive -15 horizontal edge of DigOutPositionRange.
        actor.position = Vector2::new(75, 98);
        let mut target = snapshot_with_id(target_id.as_u64());
        target.position = Vector2::new(100, 100);
        target.collectible = true;
        target.construction = FULL_CON;
        target.ocf |= ocf::IN_SOLID;

        let mut landscape = crate::Landscape::flat(300, 100);
        landscape.set_world_height(200);
        assert_eq!(
            landscape.find_closest_free(target.position, -120, 120, -1, -1),
            Some(Vector2::new(90, 98))
        );
        assert_eq!(
            landscape.find_closest_free(target.position, -140, 140, -40, 40),
            Some(Vector2::new(90, 98))
        );

        let objects = HashMap::from([(actor_id, actor), (target_id, target)]);
        let players = HashMap::new();
        let definitions = HashMap::new();
        let actor = objects.get(&actor_id).expect("actor present");
        let ctx = CommandRuntimeContext {
            landscape: Some(&landscape),
            frame: 10,
            position: actor.position,
            object: actor,
            objects: &objects,
            players: &players,
            definitions: &definitions,
            structures_need_energy: false,
            base_buy_enabled: true,
            base_sell_enabled: true,
            transfer_zones: &EMPTY_TRANSFER_ZONES,
            rng: None,
        };
        let mut state = GetState::from_request(
            &CommandRequest::new(CommandId::Get).with_target(Some(target_id)),
        )
        .expect("Get state");

        let first = state.step(&ctx);
        let first_dig = pushed_request(&first.operations, CommandId::Dig);
        assert_eq!(first_dig.target, None);
        assert_eq!((first_dig.tx, first_dig.ty), (Some(100), Some(104)));
        assert_eq!(first_dig.update_interval, 50);
        assert_eq!(first_dig.mode, CommandMode::SilentSub);
        let reissued = state.step(&ctx);

        assert_eq!(
            pushed_request(&reissued.operations, CommandId::Dig),
            first_dig,
            "Get reissues Dig on the next evaluation while OCF_InSolid remains set"
        );
    }

    #[test]
    fn get_in_solid_moves_to_the_preferred_free_staging_position() {
        let actor_id = ObjectId::new(505);
        let target_id = ObjectId::new(506);
        let mut actor = snapshot_with_id(actor_id.as_u64());
        actor.position = Vector2::new(20, 20);
        let mut target = snapshot_with_id(target_id.as_u64());
        target.position = Vector2::new(100, 100);
        target.collectible = true;
        target.construction = FULL_CON;
        target.ocf |= ocf::IN_SOLID;

        // The all-angle scan first reaches (100,90) at r=10,a=0. The
        // good-angle scan excludes a=-40..40 and first reaches (80,100) at
        // r=20,a=-90, which is still less than ten times farther away.
        let mut landscape = crate::Landscape::flat(200, 200);
        landscape.set_world_height(200);
        let mut pixels = vec![1; 200 * 200];
        pixels[90 * 200 + 100] = 0;
        pixels[100 * 200 + 80] = 0;
        landscape.set_pixel_grid(crate::landscape::PixelGrid::new(
            200,
            200,
            pixels,
            vec![0, 100],
            vec![None, Some("Earth".to_owned())],
            vec![None; 2],
        ));
        assert_eq!(
            landscape.find_closest_free(target.position, -120, 120, -1, -1),
            Some(Vector2::new(100, 90))
        );
        assert_eq!(
            landscape.find_closest_free(target.position, -140, 140, -40, 40),
            Some(Vector2::new(80, 100))
        );

        let objects = HashMap::from([(actor_id, actor), (target_id, target)]);
        let players = HashMap::new();
        let definitions = HashMap::new();
        let actor = objects.get(&actor_id).expect("actor present");
        let ctx = CommandRuntimeContext {
            landscape: Some(&landscape),
            frame: 10,
            position: actor.position,
            object: actor,
            objects: &objects,
            players: &players,
            definitions: &definitions,
            structures_need_energy: false,
            base_buy_enabled: true,
            base_sell_enabled: true,
            transfer_zones: &EMPTY_TRANSFER_ZONES,
            rng: None,
        };
        let mut state = GetState::from_request(
            &CommandRequest::new(CommandId::Get).with_target(Some(target_id)),
        )
        .expect("Get state");

        let result = state.step(&ctx);
        assert_eq!(result.status, CommandStatus::Running);
        let move_to = pushed_request(&result.operations, CommandId::MoveTo);
        assert_eq!(move_to.target, None);
        assert_eq!((move_to.tx, move_to.ty), (Some(80), Some(100)));
        assert_eq!(move_to.update_interval, 50);
        assert_eq!(move_to.mode, CommandMode::SilentSub);
        let reissued = state.step(&ctx);
        assert_eq!(
            pushed_request(&reissued.operations, CommandId::MoveTo),
            move_to,
            "Get reissues its computed staging MoveTo on the next evaluation"
        );
    }

    #[test]
    fn get_in_solid_rejects_a_good_angle_position_exactly_ten_times_farther() {
        let target = Vector2::new(100, 100);
        let mut landscape = crate::Landscape::flat(200, 200);
        landscape.set_world_height(200);
        let mut pixels = vec![1; 200 * 200];
        // General: r=10,a=0 => (100,90). Good-angle: r=100,a=-90
        // => (0,100). The latter is exactly ten times farther, and C++'s
        // strict comparison must retain the general result.
        pixels[90 * 200 + 100] = 0;
        pixels[100 * 200] = 0;
        landscape.set_pixel_grid(crate::landscape::PixelGrid::new(
            200,
            200,
            pixels,
            vec![0, 100],
            vec![None, Some("Earth".to_owned())],
            vec![None; 2],
        ));
        assert_eq!(
            landscape.find_closest_free(target, -120, 120, -1, -1),
            Some(Vector2::new(100, 90))
        );
        assert_eq!(
            landscape.find_closest_free(target, -140, 140, -40, 40),
            Some(Vector2::new(0, 100))
        );
        assert_eq!(
            GetState::dig_out_position(&landscape, target),
            Some(Vector2::new(100, 90))
        );
    }

    #[test]
    fn get_in_solid_fails_when_find_closest_free_finds_no_pixel() {
        let actor_id = ObjectId::new(507);
        let target_id = ObjectId::new(508);
        let mut actor = snapshot_with_id(actor_id.as_u64());
        actor.position = Vector2::new(150, 150);
        let mut target = snapshot_with_id(target_id.as_u64());
        target.position = Vector2::new(150, 150);
        target.collectible = true;
        target.construction = FULL_CON;
        target.ocf |= ocf::IN_SOLID;

        let mut landscape = crate::Landscape::flat(300, 0);
        landscape.set_world_height(300);
        assert_eq!(
            landscape.find_closest_free(target.position, -120, 120, -1, -1),
            None
        );

        let objects = HashMap::from([(actor_id, actor), (target_id, target)]);
        let players = HashMap::new();
        let definitions = HashMap::new();
        let actor = objects.get(&actor_id).expect("actor present");
        let ctx = CommandRuntimeContext {
            landscape: Some(&landscape),
            frame: 10,
            position: actor.position,
            object: actor,
            objects: &objects,
            players: &players,
            definitions: &definitions,
            structures_need_energy: false,
            base_buy_enabled: true,
            base_sell_enabled: true,
            transfer_zones: &EMPTY_TRANSFER_ZONES,
            rng: None,
        };
        let mut state = GetState::from_request(
            &CommandRequest::new(CommandId::Get).with_target(Some(target_id)),
        )
        .expect("Get state");

        let result = state.step(&ctx);
        assert_eq!(result.status, CommandStatus::Failed);
        assert!(result.operations.is_empty());
        assert!(result.events.is_empty());
    }

    #[test]
    fn get_transfers_item_when_in_range() {
        // C4Command::Get only requires a live target pointer with
        // OCF_Carryable. That bit is independent of FullCon, so a nonliving,
        // half-built construction kit remains a valid Get target
        // (C4Object.cpp:558-560; C4Command.cpp:1129-1152,1206-1216).
        let actor_id = ObjectId::new(100);
        let target_id = ObjectId::new(200);

        let mut actor = snapshot_with_id(actor_id.as_u64());
        actor.position = Vector2::new(0, 0);
        actor.ocf = ocf::AVAILABLE | ocf::ALIVE;
        actor.collectible = false;

        let mut item = snapshot_with_id(target_id.as_u64());
        item.position = Vector2::new(8, 0);
        item.ocf = ocf::AVAILABLE | ocf::CARRYABLE;
        item.collectible = true;
        item.construction = FULL_CON / 2;
        item.alive = false;

        let mut objects = HashMap::new();
        objects.insert(actor.id, actor.clone());
        objects.insert(item.id, item);

        let players: HashMap<i32, CommandPlayerSnapshot> = HashMap::new();
        let definitions: HashMap<DefinitionId, CommandDefinitionSnapshot> = HashMap::new();

        let actor_snapshot = objects.get(&actor_id).expect("actor present");
        let ctx = CommandRuntimeContext {
            landscape: None,
            frame: 0,
            position: actor_snapshot.position,
            object: actor_snapshot,
            objects: &objects,
            players: &players,
            definitions: &definitions,
            structures_need_energy: false,
            base_buy_enabled: true,

            base_sell_enabled: true,
            transfer_zones: &EMPTY_TRANSFER_ZONES,
            rng: None,
        };

        let mut state = GetState::from_request(
            &CommandRequest::new(CommandId::Get).with_target(Some(target_id)),
        )
        .expect("state created");

        let result = state.step(&ctx);
        assert_eq!(result.status, CommandStatus::Running);
        assert!(result.operations.is_empty());
        assert_eq!(result.events.len(), 1);

        match &result.events[0] {
            CommandEvent::GetObject {
                actor_id: event_actor,
                object_id,
                command_instance_id,
            } => {
                assert_eq!(*event_actor, actor_id);
                assert_eq!(*object_id, target_id);
                assert_eq!(*command_instance_id, 0);
            }
            other => panic!("unexpected event: {:?}", other),
        }
    }

    #[test]
    fn get_subcommand_rechecks_collection_on_next_execution() {
        let actor_id = ObjectId::new(101);
        let target_id = ObjectId::new(201);
        let mut actor = snapshot_with_id(actor_id.as_u64());
        actor.ocf = ocf::AVAILABLE | ocf::ALIVE;
        actor.collectible = false;
        let mut item = snapshot_with_id(target_id.as_u64());
        item.position = Vector2::new(8, 0);
        item.collectible = true;
        item.construction = FULL_CON;
        let mut objects = HashMap::from([(actor_id, actor), (target_id, item)]);
        let players = HashMap::new();
        let definitions = HashMap::new();

        let mut stack = CommandStack::new();
        stack
            .push_back(CommandRequest::new(CommandId::Wait).with_mode(CommandMode::Base))
            .expect("base queues");
        stack
            .push_front(
                CommandRequest::new(CommandId::Get)
                    .with_target(Some(target_id))
                    .with_update_interval(40)
                    .with_mode(CommandMode::SilentSub),
            )
            .expect("Get queues");
        let get_instance_id = stack.entries.front().expect("Get remains").instance_id;

        let actor = objects.get(&actor_id).expect("actor present");
        let ctx = command_ctx_at_frame(actor, &objects, &players, &definitions, 4);
        let first = stack.step(&ctx).expect("Get executes");
        assert_eq!(
            first.events,
            vec![CommandEvent::GetObject {
                actor_id,
                object_id: target_id,
                command_instance_id: get_instance_id,
            }]
        );

        objects
            .get_mut(&actor_id)
            .expect("actor present")
            .contents
            .push(target_id);
        objects.get_mut(&target_id).expect("item present").container = Some(actor_id);
        let actor = objects.get(&actor_id).expect("actor present");
        let ctx = command_ctx_at_frame(actor, &objects, &players, &definitions, 5);
        let collected = stack.step(&ctx).expect("Get rechecks");
        assert_eq!(collected.status, CommandStatus::Completed);

        let snapshot = stack.snapshot();
        assert_eq!(snapshot.commands.len(), 1);
        assert_eq!(snapshot.commands[0].state.id(), Some(CommandId::Wait));
        assert_eq!(snapshot.commands[0].failures, 0);
    }

    #[test]
    fn get_interval_expires_successfully_after_exact_execution_count() {
        // C4Command::Execute decrements UpdateInterval before dispatching
        // Get. The unreachable handler therefore runs 39 times, re-adding
        // MoveTo whenever it regains the front, and execution 40 succeeds
        // without running Get again (C4Command.cpp:1545-1552).
        let actor_id = ObjectId::new(102);
        let target_id = ObjectId::new(202);
        let actor = snapshot_with_id(actor_id.as_u64());
        let mut target = snapshot_with_id(target_id.as_u64());
        target.position = Vector2::new(300, 0);
        target.collectible = true;
        target.construction = FULL_CON;
        let objects = HashMap::from([(actor_id, actor), (target_id, target)]);
        let players = HashMap::new();
        let definitions = HashMap::new();

        let mut stack = CommandStack::new();
        stack
            .push_back(CommandRequest::new(CommandId::Wait).with_mode(CommandMode::Base))
            .expect("base queues");
        stack
            .push_front(
                CommandRequest::new(CommandId::Get)
                    .with_target(Some(target_id))
                    .with_update_interval(40)
                    .with_mode(CommandMode::SilentSub),
            )
            .expect("Get queues");

        for execution in 1..=39 {
            let actor = objects.get(&actor_id).expect("actor present");
            let ctx = command_ctx_at_frame(actor, &objects, &players, &definitions, 7);
            let result = stack.step(&ctx).expect("Get executes");
            assert_eq!(result.status, CommandStatus::Running);
            assert_eq!(
                stack.command_names(),
                vec!["MoveTo", "Get", "Wait"],
                "handler execution {execution} re-adds its pursuit MoveTo"
            );
            let snapshot = stack.snapshot();
            assert_eq!(
                snapshot.commands[1].update_interval,
                Some(40 - execution),
                "shared lifetime decrements once per Get execution"
            );
            assert!(stack.complete_front_if(CommandId::MoveTo));
        }

        let actor = objects.get(&actor_id).expect("actor present");
        let ctx = command_ctx_at_frame(actor, &objects, &players, &definitions, 7);
        let expired = stack.step(&ctx).expect("Get expires");
        assert_eq!(expired.status, CommandStatus::Completed);
        assert!(expired.events.is_empty());
        assert!(expired.operations.is_empty());
        let snapshot = stack.snapshot();
        assert_eq!(snapshot.commands.len(), 1);
        assert_eq!(snapshot.commands[0].state.id(), Some(CommandId::Wait));
        assert_eq!(snapshot.commands[0].failures, 0);
    }

    #[test]
    fn get_resolves_nonliving_item_by_container_and_definition() {
        // C4Command::Get resolves Target2->Contents.Find(Data) without an
        // Alive check, then collects the carryable target from the actor's
        // container (C4Command.cpp:1138-1152,1206-1216).
        let actor_id = ObjectId::new(100);
        let container_id = ObjectId::new(200);
        let item_id = ObjectId::new(300);

        let mut actor = snapshot_with_id(actor_id.as_u64());
        actor.container = Some(container_id);

        let mut container = snapshot_with_id(container_id.as_u64());
        container.contents.push(item_id);

        let mut item = snapshot_with_id(item_id.as_u64());
        item.definition_id = "CNKT".into();
        item.container = Some(container_id);
        item.collectible = true;
        item.construction = FULL_CON;
        item.alive = false;

        let objects = HashMap::from([
            (actor_id, actor),
            (container_id, container),
            (item_id, item),
        ]);
        let players = HashMap::new();
        let definitions = HashMap::new();
        let actor_snapshot = objects.get(&actor_id).expect("actor present");
        let ctx = CommandRuntimeContext {
            landscape: None,
            frame: 0,
            position: actor_snapshot.position,
            object: actor_snapshot,
            objects: &objects,
            players: &players,
            definitions: &definitions,
            structures_need_energy: false,
            base_buy_enabled: true,
            base_sell_enabled: true,
            transfer_zones: &EMPTY_TRANSFER_ZONES,
            rng: None,
        };
        let mut state = GetState::from_request(
            &CommandRequest::new(CommandId::Get)
                .with_target2(Some(container_id))
                .with_data(CommandData::Text("CNKT".into())),
        )
        .expect("state created");

        let result = state.step(&ctx);

        assert_eq!(result.status, CommandStatus::Running);
        assert!(result.events.iter().any(|event| matches!(
            event,
            CommandEvent::GetObject { actor_id: event_actor, object_id, .. }
                if *event_actor == actor_id && *object_id == item_id
        )));
    }

    #[test]
    fn get_container_definition_uses_first_match_before_carryable_gate() {
        let actor_id = ObjectId::new(101);
        let container_id = ObjectId::new(201);
        let first_id = ObjectId::new(301);
        let later_id = ObjectId::new(302);

        let mut actor = snapshot_with_id(actor_id.as_u64());
        actor.container = Some(container_id);

        let mut container = snapshot_with_id(container_id.as_u64());
        container.contents = vec![first_id, later_id];

        let mut first = snapshot_with_id(first_id.as_u64());
        first.definition_id = "CNKT".into();
        first.container = Some(container_id);
        first.collectible = false;
        first.construction = FULL_CON;
        first.ocf = ocf::AVAILABLE | ocf::FULL_CON;

        let mut later = snapshot_with_id(later_id.as_u64());
        later.definition_id = "CNKT".into();
        later.container = Some(container_id);
        later.collectible = true;
        later.construction = FULL_CON;
        later.ocf = ocf::AVAILABLE | ocf::FULL_CON | ocf::CARRYABLE;

        let objects = HashMap::from([
            (actor_id, actor),
            (container_id, container),
            (first_id, first),
            (later_id, later),
        ]);
        let players = HashMap::new();
        let definitions = HashMap::new();
        let actor = objects.get(&actor_id).expect("actor present");
        let ctx = command_ctx_at_frame(actor, &objects, &players, &definitions, 0);
        let mut state = GetState::from_request(
            &CommandRequest::new(CommandId::Get)
                .with_target2(Some(container_id))
                .with_data(CommandData::Text("CNKT".into())),
        )
        .expect("Get state");

        let result = state.step(&ctx);

        assert_eq!(state.target, Some(first_id));
        assert_eq!(result.status, CommandStatus::Failed);
        assert!(result.update.is_none());
        assert!(result.operations.is_empty());
        assert!(result.events.is_empty());
    }

    #[test]
    fn get_uses_grab_get_definition_policy_before_container_entrance() {
        // OCF_Grab alone does not permit taking contents through a pushed
        // container. C++ consults Def->GrabPutGet & C4D_Grab_Get first and
        // falls through to OCF_Entrance when that bit is absent
        // (C4Command.cpp:1226-1243).
        let actor_id = ObjectId::new(101);
        let container_id = ObjectId::new(201);
        let item_id = ObjectId::new(301);
        let actor = snapshot_with_id(actor_id.as_u64());
        let mut container = snapshot_with_id(container_id.as_u64());
        container.definition_id = "BOX".into();
        container.status = ObjectStatus::Inactive;
        container.alive = false;
        container.ocf = ocf::AVAILABLE | ocf::GRAB | ocf::ENTRANCE;
        container.contents.push(item_id);
        let mut item = snapshot_with_id(item_id.as_u64());
        item.definition_id = "WOOD".into();
        item.alive = false;
        item.collectible = true;
        item.construction = FULL_CON;
        item.container = Some(container_id);
        let mut objects = HashMap::from([
            (actor_id, actor),
            (container_id, container),
            (item_id, item),
        ]);
        let players = HashMap::new();
        let mut definitions = HashMap::from([(
            DefinitionId::from("BOX"),
            CommandDefinitionSnapshot {
                grab: 1,
                grab_put_get: 0,
                ..CommandDefinitionSnapshot::default()
            },
        )]);
        let actor = objects.get(&actor_id).expect("actor present");
        let ctx = command_ctx_at_frame(actor, &objects, &players, &definitions, 0);
        let mut state =
            GetState::from_request(&CommandRequest::new(CommandId::Get).with_target(Some(item_id)))
                .expect("state created");

        let result = state.step(&ctx);

        assert_eq!(result.status, CommandStatus::Running);
        assert!(result.update.is_none());
        assert!(result.events.is_empty());
        assert!(matches!(
            result.operations.as_slice(),
            [CommandOperation::PushFront(request)]
                if request.id == CommandId::Enter
                    && request.target == Some(container_id)
                    && request.update_interval == 50
                    && request.mode == CommandMode::SilentSub
        ));

        objects
            .get_mut(&container_id)
            .expect("container present")
            .ocf &= !ocf::ENTRANCE;
        let actor = objects.get(&actor_id).expect("actor present");
        let ctx = command_ctx_at_frame(actor, &objects, &players, &definitions, 1);
        let mut state =
            GetState::from_request(&CommandRequest::new(CommandId::Get).with_target(Some(item_id)))
                .expect("state created");
        let sealed = state.step(&ctx);
        assert_eq!(sealed.status, CommandStatus::Failed);
        assert!(sealed.update.is_none());
        assert!(sealed.operations.is_empty());
        assert!(sealed.events.is_empty());

        definitions
            .get_mut("BOX")
            .expect("container definition present")
            .grab_put_get = crate::GRAB_PUT_GET_GET;
        let actor = objects.get(&actor_id).expect("actor present");
        let ctx = command_ctx_at_frame(actor, &objects, &players, &definitions, 2);
        let mut state =
            GetState::from_request(&CommandRequest::new(CommandId::Get).with_target(Some(item_id)))
                .expect("state created");
        let grab_get = state.step(&ctx);
        assert_eq!(grab_get.status, CommandStatus::Running);
        assert!(grab_get.update.is_none());
        assert!(grab_get.events.is_empty());
        assert!(matches!(
            grab_get.operations.as_slice(),
            [CommandOperation::PushFront(request)]
                if request.id == CommandId::Grab
                    && request.target == Some(container_id)
                    && request.update_interval == 50
                    && request.mode == CommandMode::SilentSub
        ));
    }

    #[test]
    fn get_no_get_item_in_same_container_remains_running_untouched() {
        let actor_id = ObjectId::new(102);
        let container_id = ObjectId::new(202);
        let item_id = ObjectId::new(302);
        let mut actor = snapshot_with_id(actor_id.as_u64());
        actor.container = Some(container_id);
        let container = snapshot_with_id(container_id.as_u64());
        let mut item = snapshot_with_id(item_id.as_u64());
        item.definition_id = "LOCK".into();
        item.container = Some(container_id);
        item.collectible = true;
        item.construction = FULL_CON;
        let objects = HashMap::from([
            (actor_id, actor),
            (container_id, container),
            (item_id, item),
        ]);
        let players = HashMap::new();
        let definitions = HashMap::from([(
            DefinitionId::from("LOCK"),
            CommandDefinitionSnapshot {
                no_get: true,
                ..CommandDefinitionSnapshot::default()
            },
        )]);
        let actor = objects.get(&actor_id).expect("actor present");
        let ctx = command_ctx_at_frame(actor, &objects, &players, &definitions, 0);
        let mut state =
            GetState::from_request(&CommandRequest::new(CommandId::Get).with_target(Some(item_id)))
                .expect("state created");

        for _ in 0..2 {
            let result = state.step(&ctx);
            assert_eq!(result.status, CommandStatus::Running);
            assert!(result.update.is_none());
            assert!(result.operations.is_empty());
            assert!(result.events.is_empty());
        }
    }

    #[test]
    fn get_requests_exit_when_actor_contained() {
        let actor_id = ObjectId::new(1);
        let container_id = ObjectId::new(2);
        let target_id = ObjectId::new(3);

        let mut actor = snapshot_with_id(actor_id.as_u64());
        actor.container = Some(container_id);
        actor.ocf = ocf::AVAILABLE | ocf::ALIVE;
        actor.collectible = false;

        let mut container = snapshot_with_id(container_id.as_u64());
        container.position = Vector2::new(0, 0);

        let mut item = snapshot_with_id(target_id.as_u64());
        item.position = Vector2::new(20, 0);
        item.ocf = ocf::AVAILABLE | ocf::FULL_CON;
        item.collectible = true;
        item.construction = FULL_CON;

        let mut objects = HashMap::new();
        objects.insert(actor.id, actor.clone());
        objects.insert(container.id, container);
        objects.insert(item.id, item);

        let players: HashMap<i32, CommandPlayerSnapshot> = HashMap::new();
        let definitions: HashMap<DefinitionId, CommandDefinitionSnapshot> = HashMap::new();
        let actor_snapshot = objects.get(&actor_id).expect("actor present");
        let ctx = CommandRuntimeContext {
            landscape: None,
            frame: 0,
            position: actor_snapshot.position,
            object: actor_snapshot,
            objects: &objects,
            players: &players,
            definitions: &definitions,
            structures_need_energy: false,
            base_buy_enabled: true,

            base_sell_enabled: true,
            transfer_zones: &EMPTY_TRANSFER_ZONES,
            rng: None,
        };

        let mut state = GetState::from_request(
            &CommandRequest::new(CommandId::Get).with_target(Some(target_id)),
        )
        .expect("state created");

        let result = state.step(&ctx);
        assert_eq!(result.status, CommandStatus::Running);
        assert_eq!(result.operations.len(), 1);
        match &result.operations[0] {
            CommandOperation::PushFront(request) => {
                assert_eq!(request.id, CommandId::Exit);
            }
            other => panic!("expected exit request, got {:?}", other),
        }
    }

    #[test]
    fn get_requests_ungrab_when_pushing_other_target() {
        let actor_id = ObjectId::new(1);
        let pushed_id = ObjectId::new(2);
        let container_id = ObjectId::new(3);
        let target_id = ObjectId::new(4);

        let mut actor = snapshot_with_id(actor_id.as_u64());
        actor.action_procedure = ActionProcedure::Push;
        actor.action_target = Some(pushed_id);
        actor.ocf = ocf::AVAILABLE | ocf::ALIVE;
        actor.collectible = false;

        let mut pushed = snapshot_with_id(pushed_id.as_u64());
        pushed.position = Vector2::new(0, 0);

        let mut container = snapshot_with_id(container_id.as_u64());
        container.ocf = ocf::AVAILABLE | ocf::GRAB;
        container.position = Vector2::new(10, 0);

        let mut item = snapshot_with_id(target_id.as_u64());
        item.container = Some(container_id);
        item.position = container.position;
        item.ocf = ocf::AVAILABLE | ocf::FULL_CON;
        item.collectible = true;
        item.construction = FULL_CON;

        let mut objects = HashMap::new();
        objects.insert(actor.id, actor.clone());
        objects.insert(pushed.id, pushed);
        objects.insert(container.id, container);
        objects.insert(item.id, item);

        let players: HashMap<i32, CommandPlayerSnapshot> = HashMap::new();
        let definitions: HashMap<DefinitionId, CommandDefinitionSnapshot> = HashMap::new();
        let actor_snapshot = objects.get(&actor_id).expect("actor present");
        let ctx = CommandRuntimeContext {
            landscape: None,
            frame: 0,
            position: actor_snapshot.position,
            object: actor_snapshot,
            objects: &objects,
            players: &players,
            definitions: &definitions,
            structures_need_energy: false,
            base_buy_enabled: true,

            base_sell_enabled: true,
            transfer_zones: &EMPTY_TRANSFER_ZONES,
            rng: None,
        };

        let mut state = GetState::from_request(
            &CommandRequest::new(CommandId::Get).with_target(Some(target_id)),
        )
        .expect("state created");

        let result = state.step(&ctx);
        assert_eq!(result.status, CommandStatus::Running);
        assert_eq!(result.operations.len(), 1);
        match &result.operations[0] {
            CommandOperation::PushFront(request) => {
                assert_eq!(request.id, CommandId::UnGrab);
            }
            other => panic!("expected ungrab request, got {:?}", other),
        }
    }

    #[test]
    fn get_pushing_container_transfers_only_with_grab_get_definition_bit() {
        let actor_id = ObjectId::new(5);
        let container_id = ObjectId::new(6);
        let item_id = ObjectId::new(7);

        let mut actor = snapshot_with_id(actor_id.as_u64());
        actor.action_procedure = ActionProcedure::Push;
        actor.action_target = Some(container_id);

        let mut container = snapshot_with_id(container_id.as_u64());
        container.definition_id = "BOX".into();
        container.ocf = ocf::AVAILABLE | ocf::GRAB | ocf::ENTRANCE;
        container.contents.push(item_id);

        let mut item = snapshot_with_id(item_id.as_u64());
        item.definition_id = "WOOD".into();
        item.container = Some(container_id);
        item.collectible = true;
        item.construction = FULL_CON;

        let objects = HashMap::from([
            (actor_id, actor),
            (container_id, container),
            (item_id, item),
        ]);
        let players = HashMap::new();
        let mut definitions = HashMap::from([(
            DefinitionId::from("BOX"),
            CommandDefinitionSnapshot {
                grab: 1,
                grab_put_get: 0,
                ..CommandDefinitionSnapshot::default()
            },
        )]);
        let actor = objects.get(&actor_id).expect("actor present");
        let ctx = command_ctx_at_frame(actor, &objects, &players, &definitions, 0);
        let mut state =
            GetState::from_request(&CommandRequest::new(CommandId::Get).with_target(Some(item_id)))
                .expect("state created");

        let blocked = state.step(&ctx);
        assert_eq!(blocked.status, CommandStatus::Running);
        assert!(blocked.update.is_none());
        assert!(blocked.events.is_empty(), "non-Grab_Get must not transfer");
        assert!(matches!(
            blocked.operations.as_slice(),
            [CommandOperation::PushFront(request)]
                if request.id == CommandId::Enter
                    && request.target == Some(container_id)
                    && request.update_interval == 50
                    && request.mode == CommandMode::SilentSub
        ));

        definitions
            .get_mut("BOX")
            .expect("container definition present")
            .grab_put_get = crate::GRAB_PUT_GET_GET;
        let ctx = command_ctx_at_frame(actor, &objects, &players, &definitions, 1);
        let mut state =
            GetState::from_request(&CommandRequest::new(CommandId::Get).with_target(Some(item_id)))
                .expect("state created");
        let allowed = state.step(&ctx);
        assert_eq!(allowed.status, CommandStatus::Running);
        assert!(allowed.operations.is_empty());
        assert_eq!(
            allowed.events,
            vec![CommandEvent::GetObject {
                actor_id,
                object_id: item_id,
                command_instance_id: 0,
            }]
        );
    }

    #[test]
    fn get_fails_for_non_collectible_target() {
        let actor_id = ObjectId::new(1);
        let target_id = ObjectId::new(2);

        let mut actor = snapshot_with_id(actor_id.as_u64());
        actor.ocf = ocf::AVAILABLE | ocf::ALIVE;
        actor.collectible = false;

        let mut item = snapshot_with_id(target_id.as_u64());
        item.position = Vector2::new(8, 0);
        item.ocf = ocf::AVAILABLE | ocf::FULL_CON;
        item.collectible = false;
        item.construction = FULL_CON;

        let mut objects = HashMap::new();
        objects.insert(actor.id, actor.clone());
        objects.insert(item.id, item);

        let players: HashMap<i32, CommandPlayerSnapshot> = HashMap::new();
        let definitions: HashMap<DefinitionId, CommandDefinitionSnapshot> = HashMap::new();
        let actor_snapshot = objects.get(&actor_id).expect("actor present");
        let ctx = CommandRuntimeContext {
            landscape: None,
            frame: 0,
            position: actor_snapshot.position,
            object: actor_snapshot,
            objects: &objects,
            players: &players,
            definitions: &definitions,
            structures_need_energy: false,
            base_buy_enabled: true,

            base_sell_enabled: true,
            transfer_zones: &EMPTY_TRANSFER_ZONES,
            rng: None,
        };

        let mut state = GetState::from_request(
            &CommandRequest::new(CommandId::Get).with_target(Some(target_id)),
        )
        .expect("state created");

        let result = state.step(&ctx);
        assert_eq!(result.status, CommandStatus::Failed);
        assert!(result.operations.is_empty());
        assert!(result.events.is_empty());
    }

    #[test]
    fn put_requested_definition_missing_fails_without_fallback() {
        let actor_id = ObjectId::new(590);
        let item_id = ObjectId::new(591);
        let container_id = ObjectId::new(592);

        let mut actor = snapshot_with_id(actor_id.as_u64());
        actor.contents = vec![item_id];

        let mut item = snapshot_with_id(item_id.as_u64());
        item.definition_id = "ROCK".into();
        item.container = Some(actor_id);

        let target_container = snapshot_with_id(container_id.as_u64());

        let objects = HashMap::from([
            (actor.id, actor.clone()),
            (item.id, item),
            (target_container.id, target_container),
        ]);

        let players = HashMap::new();
        let definitions = HashMap::new();
        let actor_snapshot = objects.get(&actor_id).expect("actor present");
        let ctx = command_ctx_at_frame(actor_snapshot, &objects, &players, &definitions, 0);
        let mut state = PutState::from_request(
            &CommandRequest::new(CommandId::Put)
                .with_target(Some(container_id))
                .with_data(CommandData::Text("WOOD".into())),
        )
        .expect("state created");

        let result = state.step(&ctx);
        assert_eq!(result.status, CommandStatus::Failed);
        assert!(result.update.is_none());
        assert!(result.operations.is_empty());
        assert!(result.events.is_empty());

        let mut empty_actor = actor;
        empty_actor.contents.clear();
        let empty_objects = HashMap::from([
            (empty_actor.id, empty_actor.clone()),
            (container_id, snapshot_with_id(container_id.as_u64())),
        ]);
        let empty_ctx =
            command_ctx_at_frame(&empty_actor, &empty_objects, &players, &definitions, 1);
        let mut empty_state = PutState::from_request(
            &CommandRequest::new(CommandId::Put)
                .with_target(Some(container_id))
                .with_data(CommandData::Text("WOOD".into())),
        )
        .expect("state created");
        assert_eq!(empty_state.step(&empty_ctx).status, CommandStatus::Failed);
    }

    #[test]
    fn put_inside_target_uses_live_object_com_put() {
        let actor_id = ObjectId::new(600);
        let item_id = ObjectId::new(601);
        let container_id = ObjectId::new(602);

        let mut actor = snapshot_with_id(actor_id.as_u64());
        actor.position = Vector2::new(50, 50);
        actor.container = Some(container_id);
        actor.contents = vec![item_id];

        let mut item = snapshot_with_id(item_id.as_u64());
        item.container = Some(actor_id);
        item.position = actor.position;

        let mut target_container = snapshot_with_id(container_id.as_u64());
        target_container.position = Vector2::new(54, 80);

        let mut objects = HashMap::new();
        objects.insert(actor.id, actor.clone());
        objects.insert(item.id, item);
        objects.insert(target_container.id, target_container.clone());

        let players: HashMap<i32, CommandPlayerSnapshot> = HashMap::new();
        let definitions: HashMap<DefinitionId, CommandDefinitionSnapshot> = HashMap::new();
        let actor_snapshot = objects.get(&actor_id).expect("actor present");
        let ctx = CommandRuntimeContext {
            landscape: None,
            frame: 0,
            position: actor_snapshot.position,
            object: actor_snapshot,
            objects: &objects,
            players: &players,
            definitions: &definitions,
            structures_need_energy: false,
            base_buy_enabled: true,

            base_sell_enabled: true,
            transfer_zones: &EMPTY_TRANSFER_ZONES,
            rng: None,
        };

        let mut state = PutState::from_request(
            &CommandRequest::new(CommandId::Put).with_target(Some(container_id)),
        )
        .expect("state created");

        let result = state.step(&ctx);
        assert_eq!(result.status, CommandStatus::Running);
        assert!(result.operations.is_empty());
        assert_eq!(
            result.events,
            vec![CommandEvent::ObjectComPut {
                actor_id,
                target_id: container_id,
                object_id: item_id,
                ungrab_on_success: false,
                command_instance_id: 0,
            }]
        );
        state.put_pending = false; // the engine event resolver clears this

        objects.get_mut(&item_id).expect("item present").container = Some(container_id);
        objects
            .get_mut(&actor_id)
            .expect("actor present")
            .contents
            .clear();
        let actor_snapshot = objects.get(&actor_id).expect("actor present");
        let ctx = command_ctx_at_frame(actor_snapshot, &objects, &players, &definitions, 1);
        assert_eq!(state.step(&ctx).status, CommandStatus::Completed);
    }

    #[test]
    fn put_requests_exit_when_actor_in_other_container() {
        let actor_id = ObjectId::new(610);
        let item_id = ObjectId::new(611);
        let target_container_id = ObjectId::new(612);
        let current_container_id = ObjectId::new(613);

        let mut actor = snapshot_with_id(actor_id.as_u64());
        actor.container = Some(current_container_id);
        actor.contents = vec![item_id];

        let mut item = snapshot_with_id(item_id.as_u64());
        item.container = Some(actor_id);

        let mut target_container = snapshot_with_id(target_container_id.as_u64());
        target_container.position = Vector2::new(0, 0);

        let mut current_container = snapshot_with_id(current_container_id.as_u64());
        current_container.position = Vector2::new(-20, 0);

        let mut objects = HashMap::new();
        objects.insert(actor.id, actor.clone());
        objects.insert(item.id, item);
        objects.insert(target_container.id, target_container);
        objects.insert(current_container.id, current_container);

        let players: HashMap<i32, CommandPlayerSnapshot> = HashMap::new();
        let definitions: HashMap<DefinitionId, CommandDefinitionSnapshot> = HashMap::new();
        let actor_snapshot = objects.get(&actor_id).expect("actor present");
        let ctx = CommandRuntimeContext {
            landscape: None,
            frame: 0,
            position: actor_snapshot.position,
            object: actor_snapshot,
            objects: &objects,
            players: &players,
            definitions: &definitions,
            structures_need_energy: false,
            base_buy_enabled: true,

            base_sell_enabled: true,
            transfer_zones: &EMPTY_TRANSFER_ZONES,
            rng: None,
        };

        let mut state = PutState::from_request(
            &CommandRequest::new(CommandId::Put).with_target(Some(target_container_id)),
        )
        .expect("state created");

        let result = state.step(&ctx);
        assert_eq!(result.status, CommandStatus::Running);
        assert_eq!(result.operations.len(), 1);
        match &result.operations[0] {
            CommandOperation::PushFront(request) => {
                assert_eq!(request.id, CommandId::Exit);
            }
            other => panic!("expected exit request, got {:?}", other),
        }
    }

    #[test]
    fn put_waits_only_for_nearby_uncontained_hit_speed_item() {
        let actor_id = ObjectId::new(620);
        let item_id = ObjectId::new(621);
        let container_id = ObjectId::new(622);

        let mut actor = snapshot_with_id(actor_id.as_u64());
        actor.position = Vector2::new(0, 0);

        let mut item = snapshot_with_id(item_id.as_u64());
        item.position = Vector2::new(79, 0);
        item.ocf |= ocf::HIT_SPEED1;

        let container = snapshot_with_id(container_id.as_u64());

        let mut objects = HashMap::from([
            (actor.id, actor.clone()),
            (item.id, item),
            (container.id, container),
        ]);

        let players = HashMap::new();
        let definitions = HashMap::new();
        let mut state = PutState::from_request(
            &CommandRequest::new(CommandId::Put)
                .with_target(Some(container_id))
                .with_target2(Some(item_id)),
        )
        .expect("state created");

        {
            let actor_snapshot = objects.get(&actor_id).expect("actor present");
            let ctx = command_ctx_at_frame(actor_snapshot, &objects, &players, &definitions, 0);
            for _ in 0..2 {
                let waiting = state.step(&ctx);
                assert_eq!(waiting.status, CommandStatus::Running);
                assert!(waiting.operations.is_empty());
                assert!(waiting.events.is_empty());
            }
        }

        for (position, contained, hit_speed) in [
            (Vector2::new(80, 0), None, true),
            (Vector2::new(79, 0), Some(ObjectId::new(999)), true),
            (Vector2::new(79, 0), None, false),
        ] {
            let item = objects.get_mut(&item_id).expect("item present");
            item.position = position;
            item.container = contained;
            if hit_speed {
                item.ocf |= ocf::HIT_SPEED1;
            } else {
                item.ocf &= !ocf::HIT_SPEED1;
            }
            let actor_snapshot = objects.get(&actor_id).expect("actor present");
            let ctx = command_ctx_at_frame(actor_snapshot, &objects, &players, &definitions, 1);
            let mut state = PutState::from_request(
                &CommandRequest::new(CommandId::Put)
                    .with_target(Some(container_id))
                    .with_target2(Some(item_id)),
            )
            .expect("state created");
            let get = state.step(&ctx);
            assert_eq!(
                get.operations,
                vec![CommandOperation::PushFront(
                    CommandRequest::new(CommandId::Get)
                        .with_target(Some(item_id))
                        .with_update_interval(40)
                        .with_mode(CommandMode::SilentSub)
                )]
            );
        }
    }

    #[test]
    fn put_contained_target_fails_before_dig_stop() {
        let actor_id = ObjectId::new(623);
        let item_id = ObjectId::new(624);
        let container_id = ObjectId::new(625);
        let outer_id = ObjectId::new(626);

        let mut actor = snapshot_with_id(actor_id.as_u64());
        actor.action_procedure = ActionProcedure::Dig;
        actor.command_direction = CommandDirection::Right;
        actor.contents = vec![item_id];

        let mut item = snapshot_with_id(item_id.as_u64());
        item.container = Some(actor_id);

        let mut container = snapshot_with_id(container_id.as_u64());
        container.container = Some(outer_id);
        let outer = snapshot_with_id(outer_id.as_u64());

        let players = HashMap::new();
        let definitions = HashMap::new();
        let objects = HashMap::from([
            (actor_id, actor.clone()),
            (item_id, item),
            (container_id, container),
            (outer_id, outer),
        ]);
        let ctx = command_ctx_at_frame(&actor, &objects, &players, &definitions, 0);
        let mut state = PutState::from_request(
            &CommandRequest::new(CommandId::Put).with_target(Some(container_id)),
        )
        .expect("state created");
        let result = state.step(&ctx);
        assert_eq!(result.status, CommandStatus::Failed);
        assert!(result.update.is_none());
        assert!(result.operations.is_empty());
        assert!(result.events.is_empty());
    }

    #[test]
    fn put_collection_queues_throw_at_definition_center_with_live_gravity() {
        let actor_id = ObjectId::new(627);
        let item_id = ObjectId::new(628);
        let target_id = ObjectId::new(629);

        let mut actor = snapshot_with_id(actor_id.as_u64());
        actor.position = Vector2::new(40, 99);
        actor.contents = vec![item_id];
        actor.physical.throw = 50_000;

        let mut item = snapshot_with_id(item_id.as_u64());
        item.definition_id = "ITEM".into();
        item.container = Some(actor_id);

        let mut target = snapshot_with_id(target_id.as_u64());
        target.definition_id = "TARG".into();
        target.position = Vector2::new(105, 75);
        target.ocf |= ocf::COLLECTION | ocf::ENTRANCE;

        let objects = HashMap::from([
            (actor_id, actor.clone()),
            (item_id, item),
            (target_id, target),
        ]);
        let players = HashMap::new();
        let definitions = HashMap::from([
            (
                "ITEM".into(),
                CommandDefinitionSnapshot {
                    fragile: false,
                    ..CommandDefinitionSnapshot::default()
                },
            ),
            (
                "TARG".into(),
                CommandDefinitionSnapshot {
                    collection_rect: Some(DefinitionRect::new(-15, -15, 20, 20)),
                    ..CommandDefinitionSnapshot::default()
                },
            ),
        ]);
        let mut landscape = crate::Landscape::flat(200, 100);
        landscape.set_world_height(150);
        let ctx = CommandRuntimeContext {
            landscape: Some(&landscape),
            frame: 0,
            position: actor.position,
            object: objects.get(&actor_id).expect("actor present"),
            objects: &objects,
            players: &players,
            definitions: &definitions,
            structures_need_energy: false,
            base_buy_enabled: true,
            base_sell_enabled: true,
            transfer_zones: &EMPTY_TRANSFER_ZONES,
            rng: None,
        };

        let request = CommandRequest::new(CommandId::Put)
            .with_target(Some(target_id))
            .with_target2(Some(item_id));
        let mut state = PutState::from_request(&request).expect("Put state");
        let result = state.step_with_gravity(&ctx, math::fixed100(20));
        assert_eq!(
            result.operations,
            vec![CommandOperation::PushFront(
                CommandRequest::new(CommandId::Throw)
                    .with_target(Some(item_id))
                    .with_tx(Some(100))
                    .with_ty(Some(70))
                    .with_update_interval(5)
                    .with_mode(CommandMode::SilentSub)
            )]
        );
        assert!(result.events.is_empty(), "Put must not teleport the item");

        let mut high_gravity = PutState::from_request(&request).expect("Put state");
        let fallback = high_gravity.step_with_gravity(&ctx, math::fixed100(40));
        assert_eq!(
            fallback.operations,
            vec![CommandOperation::PushFront(
                CommandRequest::new(CommandId::Enter)
                    .with_target(Some(target_id))
                    .with_update_interval(50)
                    .with_mode(CommandMode::SilentSub)
            )],
            "Put's preflight must use the scenario gravity"
        );

        let mut equal_distance_objects = objects.clone();
        equal_distance_objects
            .get_mut(&target_id)
            .expect("target present")
            .position = Vector2::new(94, 99);
        let mut equal_distance_definitions = definitions.clone();
        equal_distance_definitions
            .get_mut("TARG")
            .expect("target definition present")
            .collection_rect = Some(DefinitionRect::new(0, -39, 12, 20));
        {
            let equal_distance_ctx = CommandRuntimeContext {
                landscape: Some(&landscape),
                frame: 1,
                position: actor.position,
                object: equal_distance_objects
                    .get(&actor_id)
                    .expect("actor present"),
                objects: &equal_distance_objects,
                players: &players,
                definitions: &equal_distance_definitions,
                structures_need_energy: false,
                base_buy_enabled: true,
                base_sell_enabled: true,
                transfer_zones: &EMPTY_TRANSFER_ZONES,
                rng: None,
            };
            let mut equal_distance = PutState::from_request(&request).expect("Put state");
            let strict_fallback =
                equal_distance.step_with_gravity(&equal_distance_ctx, math::fixed100(20));
            assert_eq!(
                strict_fallback.operations,
                vec![CommandOperation::PushFront(
                    CommandRequest::new(CommandId::Enter)
                        .with_target(Some(target_id))
                        .with_update_interval(50)
                        .with_mode(CommandMode::SilentSub)
                )],
                "a throwing position equally far from the actor is not closer"
            );
        }

        equal_distance_objects
            .get_mut(&target_id)
            .expect("target present")
            .ocf &= !ocf::ENTRANCE;
        let no_route_ctx = CommandRuntimeContext {
            landscape: Some(&landscape),
            frame: 2,
            position: actor.position,
            object: equal_distance_objects
                .get(&actor_id)
                .expect("actor present"),
            objects: &equal_distance_objects,
            players: &players,
            definitions: &equal_distance_definitions,
            structures_need_energy: false,
            base_buy_enabled: true,
            base_sell_enabled: true,
            transfer_zones: &EMPTY_TRANSFER_ZONES,
            rng: None,
        };
        let mut no_route = PutState::from_request(&request).expect("Put state");
        let no_route = no_route.step_with_gravity(&no_route_ctx, math::fixed100(20));
        assert_eq!(no_route.status, CommandStatus::Running);
        assert!(no_route.operations.is_empty());
        assert!(no_route.events.is_empty());
    }

    #[test]
    fn put_fragile_item_uses_grab_put_before_entrance_and_arms_ty() {
        let actor_id = ObjectId::new(630);
        let item_id = ObjectId::new(631);
        let target_id = ObjectId::new(632);

        let mut actor = snapshot_with_id(actor_id.as_u64());
        actor.position = Vector2::new(40, 99);
        actor.contents = vec![item_id];
        actor.physical.throw = 50_000;

        let mut item = snapshot_with_id(item_id.as_u64());
        item.definition_id = "FRAG".into();
        item.container = Some(actor_id);

        let mut target = snapshot_with_id(target_id.as_u64());
        target.definition_id = "TARG".into();
        target.position = Vector2::new(105, 75);
        target.ocf |= ocf::COLLECTION | ocf::ENTRANCE;

        let mut objects = HashMap::from([
            (actor_id, actor.clone()),
            (item_id, item),
            (target_id, target),
        ]);
        let players = HashMap::new();
        let definitions = HashMap::from([
            (
                "FRAG".into(),
                CommandDefinitionSnapshot {
                    fragile: true,
                    ..CommandDefinitionSnapshot::default()
                },
            ),
            (
                "TARG".into(),
                CommandDefinitionSnapshot {
                    collection_rect: Some(DefinitionRect::new(-15, -15, 20, 20)),
                    grab_put_get: crate::GRAB_PUT_GET_PUT,
                    ..CommandDefinitionSnapshot::default()
                },
            ),
        ]);
        let mut landscape = crate::Landscape::flat(200, 100);
        landscape.set_world_height(150);
        let mut state = PutState::from_request(
            &CommandRequest::new(CommandId::Put)
                .with_target(Some(target_id))
                .with_target2(Some(item_id)),
        )
        .expect("Put state");
        {
            let ctx = CommandRuntimeContext {
                landscape: Some(&landscape),
                frame: 0,
                position: objects.get(&actor_id).expect("actor present").position,
                object: objects.get(&actor_id).expect("actor present"),
                objects: &objects,
                players: &players,
                definitions: &definitions,
                structures_need_energy: false,
                base_buy_enabled: true,
                base_sell_enabled: true,
                transfer_zones: &EMPTY_TRANSFER_ZONES,
                rng: None,
            };
            let grab = state.step_with_gravity(&ctx, math::fixed100(20));
            assert_eq!(
                grab.operations,
                vec![CommandOperation::PushFront(
                    CommandRequest::new(CommandId::Grab)
                        .with_target(Some(target_id))
                        .with_update_interval(50)
                        .with_mode(CommandMode::SilentSub)
                )]
            );
            assert_eq!(state.put_ty, 1);
        }

        let actor = objects.get_mut(&actor_id).expect("actor present");
        actor.action_procedure = ActionProcedure::Push;
        actor.action_target = Some(target_id);
        let ctx = CommandRuntimeContext {
            landscape: Some(&landscape),
            frame: 1,
            position: objects.get(&actor_id).expect("actor present").position,
            object: objects.get(&actor_id).expect("actor present"),
            objects: &objects,
            players: &players,
            definitions: &definitions,
            structures_need_energy: false,
            base_buy_enabled: true,
            base_sell_enabled: true,
            transfer_zones: &EMPTY_TRANSFER_ZONES,
            rng: None,
        };
        let put = state.step_with_gravity(&ctx, math::fixed100(20));
        assert_eq!(
            put.events,
            vec![CommandEvent::ObjectComPut {
                actor_id,
                target_id,
                object_id: item_id,
                ungrab_on_success: true,
                command_instance_id: 0,
            }]
        );
    }

    #[test]
    fn put_wrong_push_target_ungrabs_before_exiting_own_container() {
        let actor_id = ObjectId::new(633);
        let item_id = ObjectId::new(634);
        let target_id = ObjectId::new(635);
        let own_container_id = ObjectId::new(636);
        let pushed_id = ObjectId::new(637);

        let mut actor = snapshot_with_id(actor_id.as_u64());
        actor.container = Some(own_container_id);
        actor.contents = vec![item_id];
        actor.action_procedure = ActionProcedure::Push;
        actor.action_target = Some(pushed_id);
        let mut item = snapshot_with_id(item_id.as_u64());
        item.container = Some(actor_id);
        let objects = HashMap::from([
            (actor_id, actor.clone()),
            (item_id, item),
            (target_id, snapshot_with_id(target_id.as_u64())),
            (
                own_container_id,
                snapshot_with_id(own_container_id.as_u64()),
            ),
            (pushed_id, snapshot_with_id(pushed_id.as_u64())),
        ]);
        let players = HashMap::new();
        let definitions = HashMap::new();
        let ctx = command_ctx_at_frame(&actor, &objects, &players, &definitions, 0);
        let mut state = PutState::from_request(
            &CommandRequest::new(CommandId::Put)
                .with_target(Some(target_id))
                .with_target2(Some(item_id)),
        )
        .expect("Put state");

        let result = state.step(&ctx);
        assert_eq!(
            result.operations,
            vec![CommandOperation::PushFront(
                CommandRequest::new(CommandId::UnGrab)
                    .with_update_interval(50)
                    .with_mode(CommandMode::SilentSub)
            )]
        );
    }

    #[test]
    fn drop_omitted_and_zero_coordinates_queue_the_same_plain_object_com_drop() {
        let actor_id = ObjectId::new(630);
        let item_id = ObjectId::new(631);

        let mut actor = snapshot_with_id(actor_id.as_u64());
        actor.position = Vector2::new(10, 10);
        actor.contents = vec![item_id];
        actor.command_direction = CommandDirection::Right;

        let mut item = snapshot_with_id(item_id.as_u64());
        item.container = Some(actor_id);
        item.position = actor.position;

        let mut objects = HashMap::new();
        objects.insert(actor.id, actor.clone());
        objects.insert(item.id, item);

        let players: HashMap<i32, CommandPlayerSnapshot> = HashMap::new();
        let definitions: HashMap<DefinitionId, CommandDefinitionSnapshot> = HashMap::new();
        let actor_snapshot = objects.get(&actor_id).expect("actor present");
        let ctx = CommandRuntimeContext {
            landscape: None,
            frame: 0,
            position: actor_snapshot.position,
            object: actor_snapshot,
            objects: &objects,
            players: &players,
            definitions: &definitions,
            structures_need_energy: false,
            base_buy_enabled: true,

            base_sell_enabled: true,
            transfer_zones: &EMPTY_TRANSFER_ZONES,
            rng: None,
        };

        for request in [
            CommandRequest::new(CommandId::Drop),
            CommandRequest::new(CommandId::Drop)
                .with_tx(Some(0))
                .with_ty(Some(0)),
        ] {
            let mut state = DropState::from_request(&request);
            let result = state.step(&ctx);
            assert_eq!(result.status, CommandStatus::Running);
            assert!(result.operations.is_empty());
            assert_eq!(result.update, None, "plain Drop preserves Action.ComDir");
            assert_eq!(
                result.events,
                vec![CommandEvent::ObjectComDrop {
                    actor_id,
                    object_id: item_id,
                    command_instance_id: 0,
                }]
            );
        }
    }

    #[test]
    fn pending_drop_finishes_only_after_callback_commands_clear() {
        let mut stack = CommandStack::new();
        stack
            .push_front(CommandRequest::new(CommandId::Drop))
            .expect("Drop queues");
        let drop_instance_id = stack.entries.front().expect("Drop remains").instance_id;
        let Some(ActiveCommand {
            state: CommandState::Drop(state),
            ..
        }) = stack.entries.front_mut()
        else {
            panic!("front command should be Drop");
        };
        state.completion_pending = true;

        // An Exit callback may AddCommand before C4Command::Drop calls
        // Finish(true). The Drop is then finished below that new front and
        // must not report ControlCommandFinished until it is uncovered.
        stack
            .push_front(CommandRequest::new(CommandId::Wait))
            .expect("callback command queues");
        assert!(stack.finish_pending_drop(drop_instance_id));
        assert_eq!(stack.finished_front_view(), None);
        assert!(
            !stack.finish_pending_drop(drop_instance_id),
            "pending marker is one-shot"
        );

        stack.pop_front();
        assert_eq!(
            stack
                .finished_front_view()
                .expect("finished Drop is now visible")
                .name,
            "Drop"
        );
    }

    #[test]
    fn pending_put_take_resolution_matches_the_exact_command_instance() {
        let mut throws = CommandStack::new();
        throws
            .push_front(CommandRequest::new(CommandId::Throw))
            .expect("outer Throw queues");
        let outer_throw = throws
            .entries
            .front()
            .expect("outer Throw remains")
            .instance_id;
        let CommandState::Throw(state) = &mut throws.entries.front_mut().unwrap().state else {
            panic!("outer command should be Throw");
        };
        state.put_take_pending = true;
        throws
            .push_front(CommandRequest::new(CommandId::Throw))
            .expect("inner Throw queues");
        let inner_throw = throws
            .entries
            .front()
            .expect("inner Throw remains")
            .instance_id;
        let CommandState::Throw(state) = &mut throws.entries.front_mut().unwrap().state else {
            panic!("inner command should be Throw");
        };
        state.put_take_pending = true;
        assert_ne!(outer_throw, inner_throw);

        // FinishCommand/SetCommand may remove the nested command while its
        // native helper is still returning. Its result must not consume the
        // same-kind outer marker that is now first in the list.
        throws.clear_front();
        assert!(!throws.finish_pending_throw(inner_throw));
        assert!(!throws.clear_pending_put_take(CommandId::Throw, inner_throw));
        assert!(matches!(
            &throws.entries.front().unwrap().state,
            CommandState::Throw(state) if state.put_take_pending
        ));
        assert!(throws.finish_pending_throw(outer_throw));

        let mut drops = CommandStack::new();
        drops
            .push_front(CommandRequest::new(CommandId::Drop))
            .expect("outer Drop queues");
        let outer_drop = drops
            .entries
            .front()
            .expect("outer Drop remains")
            .instance_id;
        let CommandState::Drop(state) = &mut drops.entries.front_mut().unwrap().state else {
            panic!("outer command should be Drop");
        };
        state.completion_pending = true;
        drops
            .push_front(CommandRequest::new(CommandId::Drop))
            .expect("inner Drop queues");
        let inner_drop = drops
            .entries
            .front()
            .expect("inner Drop remains")
            .instance_id;
        let CommandState::Drop(state) = &mut drops.entries.front_mut().unwrap().state else {
            panic!("inner command should be Drop");
        };
        state.completion_pending = true;
        drops.clear_front();
        assert!(!drops.finish_pending_drop(inner_drop));
        assert!(matches!(
            &drops.entries.front().unwrap().state,
            CommandState::Drop(state) if state.completion_pending
        ));
        assert!(drops.finish_pending_drop(outer_drop));
    }

    #[test]
    fn exact_failed_result_does_not_touch_a_same_kind_replacement() {
        let mut stack = CommandStack::new();
        stack
            .push_front(CommandRequest::new(CommandId::Exit))
            .expect("original Exit queues");
        let original = stack.entries.front().expect("original Exit").instance_id;
        stack
            .push_front(CommandRequest::new(CommandId::Exit))
            .expect("replacement Exit queues");
        let replacement = stack.entries.front().expect("replacement Exit").instance_id;

        assert!(stack.fail_command_instance(CommandId::Exit, original));
        assert_eq!(
            stack
                .entries
                .iter()
                .find(|entry| entry.instance_id == original)
                .expect("original Exit remains")
                .failures,
            1
        );
        assert_eq!(
            stack
                .entries
                .iter()
                .find(|entry| entry.instance_id == replacement)
                .expect("replacement Exit remains")
                .failures,
            0
        );

        stack.entries.retain(|entry| entry.instance_id != original);
        assert!(!stack.fail_command_instance(CommandId::Exit, original));
        assert_eq!(
            stack.entries.front().expect("replacement remains").failures,
            0
        );
    }

    #[test]
    fn command_instance_ids_survive_live_restore_but_persisted_stacks_get_fresh_ids() {
        let mut stack = CommandStack::new();
        stack
            .push_front(CommandRequest::new(CommandId::Throw))
            .expect("Throw queues");
        let instance_id = stack.entries.front().expect("Throw remains").instance_id;
        let CommandState::Throw(state) = &mut stack.entries.front_mut().unwrap().state else {
            panic!("command should be Throw");
        };
        state.put_take_pending = true;
        let snapshot = stack.snapshot();

        let mut live_restore = CommandStack::new();
        live_restore.restore_from_snapshot(&snapshot);
        assert_eq!(
            live_restore.entries.front().unwrap().instance_id,
            instance_id,
            "callback Restore retains the in-flight native-command identity"
        );
        assert!(live_restore.finish_pending_throw(instance_id));

        let encoded = serde_json::to_value(&snapshot).expect("snapshot serializes");
        assert!(
            encoded.get("next_instance_id").is_none(),
            "the runtime allocator is not savegame state"
        );
        assert!(
            encoded["commands"][0].get("instance_id").is_none(),
            "native command identity is not serialized"
        );
        let decoded: CommandStackSnapshot =
            serde_json::from_value(encoded.clone()).expect("snapshot deserializes");
        let mut restored = CommandStack::new();
        restored
            .push_front(CommandRequest::new(CommandId::Wait))
            .expect("allocator advances");
        restored.clear();
        restored.restore_from_snapshot(&decoded);
        let persisted_id = restored.entries.front().unwrap().instance_id;
        assert_ne!(persisted_id, 0);
        assert_ne!(persisted_id, instance_id);
        assert!(restored.finish_pending_throw(persisted_id));
        restored
            .push_front(CommandRequest::new(CommandId::Wait))
            .expect("new command queues");
        assert_ne!(restored.entries.front().unwrap().instance_id, persisted_id);

        let legacy: CommandStackSnapshot =
            serde_json::from_value(encoded).expect("legacy snapshot deserializes");
        let mut restored_legacy = CommandStack::new();
        restored_legacy.restore_from_snapshot(&legacy);
        let migrated_id = restored_legacy.entries.front().unwrap().instance_id;
        assert_ne!(migrated_id, 0);
        assert!(
            restored_legacy.finish_pending_throw(0),
            "a serialized legacy event with token zero uses the compatibility fallback"
        );
        restored_legacy
            .push_front(CommandRequest::new(CommandId::Wait))
            .expect("post-migration command queues");
        assert_ne!(
            restored_legacy.entries.front().unwrap().instance_id,
            migrated_id
        );
    }

    #[test]
    fn persisted_zero_event_resolves_to_the_fresh_pending_command_identity() {
        let actor_id = ObjectId::new(621);
        let target_id = ObjectId::new(622);
        let mut original = CommandStack::new();
        original
            .push_front(CommandRequest::new(CommandId::Throw))
            .expect("Throw queues");
        let original_id = original.entries.front().expect("Throw remains").instance_id;
        let CommandState::Throw(state) = &mut original.entries.front_mut().unwrap().state else {
            panic!("command should be Throw");
        };
        state.put_take_pending = true;

        let encoded_snapshot =
            serde_json::to_value(original.snapshot()).expect("snapshot serializes");
        let encoded_event = serde_json::to_value(CommandEvent::ObjectComPutTake {
            actor_id,
            target_id,
            requested_item: None,
            command: CommandId::Throw,
            command_instance_id: original_id,
        })
        .expect("event serializes");
        let restored_event: CommandEvent =
            serde_json::from_value(encoded_event).expect("event deserializes");
        let CommandEvent::ObjectComPutTake {
            command_instance_id,
            ..
        } = restored_event
        else {
            panic!("event should remain ObjectComPutTake");
        };
        assert_eq!(command_instance_id, 0, "runtime ids are omitted from saves");

        let decoded: CommandStackSnapshot =
            serde_json::from_value(encoded_snapshot).expect("snapshot deserializes");
        let mut restored = CommandStack::new();
        restored
            .push_front(CommandRequest::new(CommandId::Wait))
            .expect("advance the allocator");
        restored.clear();
        restored.restore_from_snapshot(&decoded);
        let fresh_id = restored
            .entries
            .front()
            .expect("Throw restored")
            .instance_id;
        assert_ne!(fresh_id, 0);
        assert_ne!(fresh_id, original_id);
        assert_eq!(
            restored.resolve_event_instance_id(
                CommandEventInstanceKind::PutTake(CommandId::Throw),
                command_instance_id,
            ),
            fresh_id
        );
        assert_eq!(
            restored.resolve_event_instance_id(
                CommandEventInstanceKind::PutTake(CommandId::Throw),
                987,
            ),
            987,
            "an already-bound runtime event passes through unchanged"
        );
    }

    #[test]
    fn zero_event_resolution_uses_the_event_specific_pending_marker() {
        let mut throws = CommandStack::new();
        throws
            .push_front(CommandRequest::new(CommandId::Throw))
            .expect("outer Throw queues");
        let pending_throw = throws.entries.front().expect("outer Throw").instance_id;
        let CommandState::Throw(state) = &mut throws.entries.front_mut().unwrap().state else {
            panic!("outer command should be Throw");
        };
        state.put_take_pending = true;
        throws
            .push_front(CommandRequest::new(CommandId::Throw))
            .expect("inner Throw queues");
        let exact_throw = throws.entries.front().expect("inner Throw").instance_id;
        throws.entries.front_mut().unwrap().finished = Some(CommandStatus::Completed);
        assert_eq!(
            throws.resolve_event_instance_id(CommandEventInstanceKind::Exact(CommandId::Throw), 0,),
            exact_throw,
            "Exact includes a still-linked finished command"
        );
        assert_eq!(
            throws
                .resolve_event_instance_id(CommandEventInstanceKind::PutTake(CommandId::Throw), 0,),
            pending_throw,
            "PutTake skips a same-kind command without its in-flight marker"
        );

        let mut digs = CommandStack::new();
        digs.push_front(CommandRequest::new(CommandId::Dig))
            .expect("outer Dig queues");
        let pending_dig = digs.entries.front().expect("outer Dig").instance_id;
        let CommandState::Dig(state) = &mut digs.entries.front_mut().unwrap().state else {
            panic!("outer command should be Dig");
        };
        state.start_pending = true;
        digs.push_front(CommandRequest::new(CommandId::Dig))
            .expect("replacement Dig queues");
        assert_eq!(
            digs.resolve_event_instance_id(CommandEventInstanceKind::Dig, 0),
            pending_dig,
            "Dig skips a same-kind command without its in-flight marker"
        );

        let mut gets = CommandStack::new();
        gets.push_front(CommandRequest::new(CommandId::Get).with_target(Some(ObjectId::new(91))))
            .expect("outer Get queues");
        let pending_get = gets.entries.front().expect("outer Get").instance_id;
        let CommandState::Get(state) = &mut gets.entries.front_mut().unwrap().state else {
            panic!("outer command should be Get");
        };
        state.enter_pending = true;
        gets.push_front(CommandRequest::new(CommandId::Get).with_target(Some(ObjectId::new(92))))
            .expect("replacement Get queues");
        assert_eq!(
            gets.resolve_event_instance_id(CommandEventInstanceKind::Get, 0),
            pending_get,
            "Get skips a same-kind command without its in-flight marker"
        );

        let acquire_request = || {
            CommandRequest::new(CommandId::Acquire).with_data(CommandData::Text("WOOD".to_owned()))
        };
        let mut acquires = CommandStack::new();
        acquires
            .push_front(acquire_request())
            .expect("outer Acquire queues");
        let pending_acquire = acquires.entries.front().expect("outer Acquire").instance_id;
        let CommandState::Acquire(state) = &mut acquires.entries.front_mut().unwrap().state else {
            panic!("outer command should be Acquire");
        };
        state.script_pending = true;
        acquires
            .push_front(acquire_request())
            .expect("inner Acquire queues");
        assert_eq!(
            acquires.resolve_event_instance_id(
                CommandEventInstanceKind::Script(CommandId::Acquire),
                0,
            ),
            pending_acquire
        );

        let mut constructs = CommandStack::new();
        constructs
            .push_front(
                CommandRequest::new(CommandId::Construct)
                    .with_data(CommandData::Text("HUT1".to_owned())),
            )
            .expect("Construct queues");
        let pending_construct = constructs
            .entries
            .front()
            .expect("Construct remains")
            .instance_id;
        let CommandState::Construct(state) = &mut constructs.entries.front_mut().unwrap().state
        else {
            panic!("command should be Construct");
        };
        state.script_pending = true;
        assert_eq!(
            constructs.resolve_event_instance_id(
                CommandEventInstanceKind::Script(CommandId::Construct),
                0,
            ),
            pending_construct
        );
    }

    #[test]
    fn zero_event_resolution_finds_detached_preludes_and_exit_activation() {
        let mut exits = CommandStack::new();
        exits
            .push_front(CommandRequest::new(CommandId::Exit))
            .expect("Exit queues");
        let detached_exit = exits.entries.front().expect("Exit remains").instance_id;
        let CommandState::Exit(state) = &mut exits.entries.front_mut().unwrap().state else {
            panic!("command should be Exit");
        };
        state.stop_continuation = true;
        exits.clear();
        assert_eq!(
            exits.resolve_event_instance_id(CommandEventInstanceKind::Prelude(CommandId::Exit), 0,),
            detached_exit
        );
        exits
            .push_front(CommandRequest::new(CommandId::Exit))
            .expect("replacement Exit queues");
        let attached_exit = exits.entries.front().expect("replacement Exit").instance_id;
        let CommandState::Exit(state) = &mut exits.entries.front_mut().unwrap().state else {
            panic!("replacement command should be Exit");
        };
        state.stop_continuation = true;
        assert_eq!(
            exits.resolve_event_instance_id(CommandEventInstanceKind::Prelude(CommandId::Exit), 0,),
            attached_exit,
            "an attached pending command takes precedence over retained bodies"
        );

        let mut throws = CommandStack::new();
        throws
            .push_front(CommandRequest::new(CommandId::Throw))
            .expect("Throw queues");
        let detached_throw = throws.entries.front().expect("Throw remains").instance_id;
        let CommandState::Throw(state) = &mut throws.entries.front_mut().unwrap().state else {
            panic!("command should be Throw");
        };
        state
            .continuations
            .push(ThrowContinuation::AfterObjectComStop);
        throws.clear();
        assert_eq!(
            throws
                .resolve_event_instance_id(CommandEventInstanceKind::Prelude(CommandId::Throw), 0,),
            detached_throw
        );

        let mut drops = CommandStack::new();
        drops
            .push_front(CommandRequest::new(CommandId::Drop))
            .expect("Drop queues");
        let detached_drop = drops.entries.front().expect("Drop remains").instance_id;
        let CommandState::Drop(state) = &mut drops.entries.front_mut().unwrap().state else {
            panic!("command should be Drop");
        };
        state
            .continuations
            .push(DropContinuation::AfterObjectComStop);
        drops.clear();
        assert_eq!(
            drops.resolve_event_instance_id(CommandEventInstanceKind::Prelude(CommandId::Drop), 0,),
            detached_drop
        );

        let mut activation = CommandStack::new();
        activation
            .push_front(CommandRequest::new(CommandId::Exit))
            .expect("Exit queues");
        let detached_activation = activation
            .entries
            .front()
            .expect("Exit remains")
            .instance_id;
        let CommandState::Exit(state) = &mut activation.entries.front_mut().unwrap().state else {
            panic!("command should be Exit");
        };
        state.activation_pending = 1;
        activation.clear();
        assert_eq!(
            activation.resolve_event_instance_id(CommandEventInstanceKind::ExitActivation, 0),
            detached_activation
        );
    }

    #[test]
    fn zero_token_throw_and_drop_preludes_pin_the_resolved_command_instance() {
        let actor_id = ObjectId::new(624);
        let item_id = ObjectId::new(625);
        let mut digging = snapshot_with_id(actor_id.as_u64());
        digging.action_procedure = ActionProcedure::Dig;
        digging.contents = vec![item_id];
        let mut item = snapshot_with_id(item_id.as_u64());
        item.container = Some(actor_id);
        let digging_objects = HashMap::from([(actor_id, digging.clone()), (item_id, item.clone())]);
        let players = HashMap::new();
        let definitions = HashMap::new();
        let digging_ctx = command_ctx_at_frame(
            digging_objects.get(&actor_id).expect("actor present"),
            &digging_objects,
            &players,
            &definitions,
            1,
        );
        let mut walking = digging;
        walking.action_procedure = ActionProcedure::Walk;
        let walking_objects = HashMap::from([(actor_id, walking), (item_id, item)]);
        let walking_ctx = command_ctx_at_frame(
            walking_objects.get(&actor_id).expect("actor present"),
            &walking_objects,
            &players,
            &definitions,
            1,
        );

        let mut throws = CommandStack::new();
        throws
            .push_front(CommandRequest::new(CommandId::Throw).with_target(Some(item_id)))
            .expect("original Throw queues");
        let original_throw = throws.entries.front().expect("original Throw").instance_id;
        assert!(matches!(
            throws
                .execute_front(&digging_ctx)
                .expect("Throw starts")
                .events
                .as_slice(),
            [CommandEvent::ObjectComStopThrow { .. }]
        ));
        throws.clear();
        throws
            .push_front(CommandRequest::new(CommandId::Throw).with_target(Some(item_id)))
            .expect("replacement Throw queues");
        let replacement_throw = throws
            .entries
            .front()
            .expect("replacement Throw")
            .instance_id;
        let resumed_throw = throws
            .execute_pending_throw_prelude(
                &walking_ctx,
                crate::PhysicsSettings::default().gravity_as_c4fixed(),
                0,
            )
            .expect("deserialized Throw prelude resumes");
        let [CommandEvent::ThrowObject {
            command_instance_id,
            ..
        }] = resumed_throw.events.as_slice()
        else {
            panic!("unexpected Throw events: {:?}", resumed_throw.events);
        };
        assert_eq!(*command_instance_id, original_throw);
        assert!(!throws.finish_command_instance(CommandId::Throw, *command_instance_id));
        assert_eq!(
            throws.entries.front().unwrap().instance_id,
            replacement_throw
        );
        assert_eq!(throws.entries.front().unwrap().finished, None);

        let mut drops = CommandStack::new();
        drops
            .push_front(CommandRequest::new(CommandId::Drop).with_target(Some(item_id)))
            .expect("original Drop queues");
        let original_drop = drops.entries.front().expect("original Drop").instance_id;
        assert!(matches!(
            drops
                .execute_front(&digging_ctx)
                .expect("Drop starts")
                .events
                .as_slice(),
            [CommandEvent::ObjectComStopDrop { .. }]
        ));
        drops.clear();
        drops
            .push_front(CommandRequest::new(CommandId::Drop).with_target(Some(item_id)))
            .expect("replacement Drop queues");
        let replacement_drop = drops.entries.front().expect("replacement Drop").instance_id;
        let CommandState::Drop(replacement_state) = &mut drops.entries.front_mut().unwrap().state
        else {
            panic!("replacement command should be Drop");
        };
        replacement_state.completion_pending = true;
        let resumed_drop = drops
            .execute_pending_drop_prelude(&walking_ctx, 0)
            .expect("deserialized Drop prelude resumes");
        let [CommandEvent::ObjectComDrop {
            command_instance_id,
            ..
        }] = resumed_drop.events.as_slice()
        else {
            panic!("unexpected Drop events: {:?}", resumed_drop.events);
        };
        assert_eq!(*command_instance_id, original_drop);
        assert!(!drops.finish_pending_drop(*command_instance_id));
        assert_eq!(drops.entries.front().unwrap().instance_id, replacement_drop);
        assert_eq!(drops.entries.front().unwrap().finished, None);
    }

    #[test]
    fn detached_throw_prelude_retains_its_failure_feedback_context() {
        let actor_id = ObjectId::new(619);
        let mut digging = snapshot_with_id(actor_id.as_u64());
        digging.action_procedure = ActionProcedure::Dig;
        let digging_objects = HashMap::from([(actor_id, digging.clone())]);
        let players = HashMap::new();
        let definitions = HashMap::new();
        let digging_ctx = command_ctx_at_frame(
            digging_objects.get(&actor_id).expect("actor present"),
            &digging_objects,
            &players,
            &definitions,
            1,
        );
        let mut stack = CommandStack::new();
        stack
            .push_front(
                CommandRequest::new(CommandId::Throw)
                    .with_tx(Some(100))
                    .with_ty(Some(20))
                    .with_mode(CommandMode::Base),
            )
            .expect("Throw queues");
        let command_instance_id = stack.entries.front().unwrap().instance_id;
        let initial = stack.execute_front(&digging_ctx).expect("Throw starts");
        assert!(matches!(
            initial.events.as_slice(),
            [CommandEvent::ObjectComStopThrow { .. }]
        ));

        stack.clear();
        let mut walking = digging;
        walking.action_procedure = ActionProcedure::Walk;
        let walking_objects = HashMap::from([(actor_id, walking)]);
        let walking_ctx = command_ctx_at_frame(
            walking_objects.get(&actor_id).expect("actor present"),
            &walking_objects,
            &players,
            &definitions,
            1,
        );
        let resumed = stack
            .execute_pending_throw_prelude(
                &walking_ctx,
                crate::PhysicsSettings::default().gravity_as_c4fixed(),
                command_instance_id,
            )
            .expect("detached Throw resumes");
        assert_eq!(resumed.status, CommandStatus::Failed);
        assert!(matches!(
            resumed.events.as_slice(),
            [CommandEvent::FailureFeedback { feedback, .. }]
                if feedback.command.name == "Throw"
        ));
    }

