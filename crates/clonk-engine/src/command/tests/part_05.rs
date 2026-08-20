    // Contiguous slice 5 of 7 of the `command/tests` battery, spliced by
    // `include!` from the parent module so every test id is unchanged.

    #[test]
    fn home_fails_when_no_base_available() {
        let builder_id = ObjectId::new(550);

        let builder = command_object!(builder_id.as_u64(); owner = 23);

        let objects = command_objects([builder.clone()]);

        let ctx = command_context!(command_ctx(
            objects.get(&builder_id).expect("builder present"),
            &objects,
            0,
        ); position: builder.position);

        let mut state = HomeState::from_request(&request!(Home)).expect("state created");

        let result = state.step(&ctx);
        assert_eq!(result.status, CommandStatus::Failed);
    }

    #[test]
    fn base_scans_break_distance_ties_by_cpp_master_list_order() {
        let actor_id = ObjectId::new(1);
        let lower_id_later = ObjectId::new(2);
        let higher_id_earlier = ObjectId::new(99);

        let actor = command_object!(actor_id.as_u64(); owner = 7; position = Vector2::ZERO);

        let base = |id: ObjectId, position: Vector2, master_list_order: usize| {
            let mut snapshot = command_object!(id.as_u64(); master_list_order = master_list_order;
                position = position);
            // All three commands select bases through C4Object::Base.
            snapshot.base = actor.owner;
            snapshot.owner = 99;
            snapshot
        };

        let mut objects = command_objects([
            actor.clone(),
            base(lower_id_later, Vector2::new(10, 0), 2),
            // d²=116 and d²=100 both truncate to C++ Distance 10;
            // strict improvement therefore retains the earlier base.
            base(higher_id_earlier, Vector2::new(-10, 4), 1),
        ]);
        let players = HashMap::from([(actor.owner, command_player!(0))]);
        let choose = |objects: &CommandObjectSnapshots| {
            let ctx = command_ctx_with_players(&actor, objects, &players, 0);
            let mut sell = SellState::from_request(&request!(Sell)).expect("sell state");
            let mut buy = BuyState::from_request(&request!(Buy)).expect("buy state");
            let mut home = HomeState::from_request(&request!(Home)).expect("home state");
            (
                sell.resolve_base(&ctx),
                buy.resolve_base(&ctx),
                home.resolve_base(&ctx),
            )
        };

        assert_eq!(
            choose(&objects),
            (
                Some(higher_id_earlier),
                Some(higher_id_earlier),
                Some(higher_id_earlier),
            )
        );

        objects
            .get_mut(&lower_id_later)
            .expect("later base present")
            .master_list_order = 1;
        objects
            .get_mut(&higher_id_earlier)
            .expect("earlier base present")
            .master_list_order = 2;
        assert_eq!(
            choose(&objects),
            (
                Some(lower_id_later),
                Some(lower_id_later),
                Some(lower_id_later),
            )
        );
    }

    #[test]
    fn buy_implicit_base_skips_hostility_and_accepts_generic_allied_bases() {
        let actor_id = ObjectId::new(1);
        let hostile_id = ObjectId::new(2);
        let allied_id = ObjectId::new(3);
        let own_id = ObjectId::new(4);

        let actor = command_object!(actor_id.as_u64(); owner = 1; position = Vector2::ZERO);
        let base = |id: ObjectId, player: i32, x: i32, order: usize| {
            let snapshot = command_object!(id.as_u64(); base = player; position = Vector2::new(x, 0);
                master_list_order = order);
            // FindFriendlyBase does not require Structure, Entrance, object
            // ownership, or a non-collectible definition.
            snapshot
        };
        let objects = command_objects([
            actor.clone(),
            base(hostile_id, 2, 1, 1),
            base(allied_id, 3, 5, 2),
            base(own_id, 1, 10, 3),
        ]);
        let player = |hostile_to| command_player!(0, hostile_to: hostile_to);
        let players = HashMap::from([
            (1, player(vec![2])),
            (2, player(Vec::new())),
            (3, player(Vec::new())),
        ]);
        let ctx = command_ctx_with_players(&actor, &objects, &players, 0);
        let mut buy = BuyState::from_request(&request!(Buy)).expect("buy state");

        assert_eq!(buy.resolve_base(&ctx), Some(allied_id));
        assert_eq!(buy.target, Some(allied_id));
    }

    #[test]
    fn exit_init_evaluation_cancels_attach_before_executing_exit() {
        // C4Command::InitEvaluation runs ObjectComCancelAttach and returns
        // before C4Command::Exit executes (C4Command.cpp:1554-1555,
        // 1654-1657). The next Execute performs the actual exit.
        let actor_id = ObjectId::new(49);
        let container_id = ObjectId::new(50);

        let actor = command_object!(actor_id.as_u64(); container = Some(container_id);
            action_name = "Attach".to_string(); action_procedure = ActionProcedure::Attach;
            command_direction = CommandDirection::Right);

        let container = command_object!(container_id.as_u64(); position = Vector2::new(17, 23);
            entrance_status = true);

        let objects = command_objects([actor.clone(), container.clone()]);
        let ctx = command_ctx(&actor, &objects, 0);
        let mut state = ExitState::from_request(&request!(Exit)).expect("state created");

        let evaluation = state.step(&ctx);
        assert_eq!(evaluation.status, CommandStatus::Running);
        assert!(evaluation.update.is_none());
        assert!(evaluation.operations.is_empty());
        let [CommandEvent::ApplyObjectUpdate { object_id, update }] = evaluation.events.as_slice()
        else {
            panic!("unexpected evaluation events: {:?}", evaluation.events);
        };
        assert_eq!(*object_id, actor_id);
        let action = update.action.as_ref().expect("attach changes to ActIdle");
        assert_eq!(action.name.as_deref(), Some("Idle"));
        assert!(
            !action.force,
            "ObjectComCancelAttach uses ordinary SetAction"
        );
        assert!(update.command_direction.is_none());
        assert!(update.container.is_none());
        assert!(update.position.is_none());
        assert!(update.velocity.is_none());

        let execution = state.step(&ctx);
        assert_eq!(execution.status, CommandStatus::Running);
        assert!(execution.operations.is_empty());
        assert!(execution.update.is_none());
        assert_eq!(
            execution.events,
            [CommandEvent::CommandExitObject {
                object_id: actor_id,
                previous_container: container_id,
                position: actor.position,
                jump_after: false,
                command_instance_id: 0,
            }]
        );
    }

    #[test]
    fn exit_carryable_from_collection_area_exits_above_it_and_jumps() {
        // With no entrance-area ejection, C++ places a carryable at the
        // container x / Collection.y-1 point and immediately calls the live
        // ObjectComJump helper (C4Command.cpp:643-649).
        let actor_id = ObjectId::new(51);
        let container_id = ObjectId::new(52);

        let actor = command_object!(actor_id.as_u64(); container = Some(container_id);
            collectible = true; action_name = "Walk".to_string(); action_procedure = ActionProcedure::Walk;
            command_direction = CommandDirection::Left);

        let container = command_object!(container_id.as_u64(); definition_id = "CARR".to_string();
            position = Vector2::new(80, 120); entrance_status = true);

        let objects = command_objects([actor.clone(), container.clone()]);
        let definitions = HashMap::from([(
            container.definition_id.clone(),
            command_definition! { collection_rect: Some(DefinitionRect::new(-10, -25, 20, 30)) },
        )]);
        let ctx = command_ctx_with_definitions(&actor, &objects, &definitions, 0);
        let mut state =
            ExitState::from_request(&request!(Exit, with_evaluated: true)).expect("state created");

        let result = state.step(&ctx);
        assert_eq!(result.status, CommandStatus::Running);
        assert!(result.operations.is_empty());
        assert_eq!(
            result.events,
            [CommandEvent::CommandExitObject {
                object_id: actor_id,
                previous_container: container_id,
                position: Vector2::new(80, 94),
                jump_after: true,
                command_instance_id: 0,
            }]
        );
        assert!(
            result.update.is_none(),
            "the live Exit event preserves Left ComDir for ObjectComJump"
        );
    }

    #[test]
    fn exit_completes_when_not_contained() {
        let actor_id = ObjectId::new(51);
        let actor = snapshot_with_id(actor_id.as_u64());

        let objects = command_objects([actor.clone()]);

        let ctx = command_context!(command_ctx(objects.get(&actor_id).expect("actor present"), &objects, 0); position: actor.position);

        let mut state =
            ExitState::from_request(&request!(Exit, with_evaluated: true)).expect("state created");

        let result = state.step(&ctx);
        assert_eq!(result.status, CommandStatus::Completed);
        assert!(result.update.is_none());
        assert!(result.operations.is_empty());
    }

    #[test]
    fn exit_attempts_live_enter_into_parent_container_when_nested() {
        let actor_id = ObjectId::new(60);
        let container_id = ObjectId::new(70);
        let parent_id = ObjectId::new(80);

        let actor = command_object!(actor_id.as_u64(); container = Some(container_id);
            command_direction = CommandDirection::Right);

        let container = command_object!(container_id.as_u64(); container = Some(parent_id);
            position = Vector2::new(12, 34); entrance_status = true);

        let parent = command_object!(parent_id.as_u64(); position = Vector2::new(100, -20));

        let objects = command_objects([actor.clone(), container, parent.clone()]);

        let ctx = command_context!(command_ctx(objects.get(&actor_id).expect("actor present"), &objects, 10); position: actor.position);

        let mut state =
            ExitState::from_request(&request!(Exit, with_evaluated: true)).expect("state created");

        let result = state.step(&ctx);
        assert_eq!(result.status, CommandStatus::Running);
        assert!(result.update.is_none());
        assert_eq!(
            result.events,
            [CommandEvent::CommandExitIntoParent {
                object_id: actor_id,
                container_id: parent_id,
                command_instance_id: 0,
            }],
            "C4Object::Enter, including callbacks and Status checks, runs live"
        );
    }

    #[test]
    fn exit_leaves_container_when_no_parent() {
        let actor_id = ObjectId::new(90);
        let container_id = ObjectId::new(100);

        let actor = command_object!(actor_id.as_u64(); container = Some(container_id);
            position = Vector2::new(7, 9); command_direction = CommandDirection::Left);

        let container = command_object!(container_id.as_u64(); position = Vector2::new(-40, 5);
            entrance_status = true);

        let objects = command_objects([actor.clone(), container.clone()]);

        let ctx = command_context!(command_ctx(objects.get(&actor_id).expect("actor present"), &objects, 20); position: actor.position);

        let mut state =
            ExitState::from_request(&request!(Exit, with_evaluated: true)).expect("state created");

        let result = state.step(&ctx);
        assert_eq!(result.status, CommandStatus::Running);
        assert!(result.update.is_none());
        assert_eq!(
            result.events,
            [CommandEvent::CommandExitObject {
                object_id: actor_id,
                previous_container: container_id,
                position: actor.position,
                jump_after: false,
                command_instance_id: 0,
            }]
        );
    }

    #[test]
    fn exit_uses_open_entrance_bottom_for_top_level_ejection() {
        // C4Command::Exit places a top-level contained object at the
        // entrance center/bottom, adjusted by the exiting object's shape
        // top (C4Command.cpp:624-645). This is the HUT2/TFLN geometry in
        // Tutorial 4: dropping at the building center makes the flint hit
        // the ground hard enough to ignite.
        let actor_id = ObjectId::new(91);
        let container_id = ObjectId::new(101);

        let actor =
            command_object!(actor_id.as_u64(); container = Some(container_id); shape_top = -3);

        let mut container = command_object!(container_id.as_u64(); position = Vector2::new(586, 245);
            alive = false; entrance_status = true);
        container.ocf |= ocf::ENTRANCE;
        container.entrance = Some(DefinitionRect::new(568, 253, 16, 17));

        let objects = command_objects([actor.clone(), container]);

        let ctx = command_context!(command_ctx(objects.get(&actor_id).expect("actor present"), &objects, 20); position: actor.position);

        let mut state =
            ExitState::from_request(&request!(Exit, with_evaluated: true)).expect("state created");

        let result = state.step(&ctx);
        assert_eq!(result.status, CommandStatus::Running);
        assert!(result.update.is_none());
        assert_eq!(
            result.events,
            [CommandEvent::CommandExitObject {
                object_id: actor_id,
                previous_container: container_id,
                position: Vector2::new(576, 266),
                jump_after: false,
                command_instance_id: 0,
            }]
        );
    }

    #[test]
    fn exit_activates_closed_entrance_and_remains_pending() {
        let actor_id = ObjectId::new(92);
        let container_id = ObjectId::new(102);

        let actor = command_object!(actor_id.as_u64(); container = Some(container_id);
            command_direction = CommandDirection::Right);

        let mut container =
            command_object!(container_id.as_u64(); alive = false; entrance_status = false);
        container.ocf |= ocf::ENTRANCE;

        let objects = command_objects([actor.clone(), container]);
        let ctx = command_context!(command_ctx(objects.get(&actor_id).expect("actor present"), &objects, 20); position: actor.position);

        let mut state =
            ExitState::from_request(&request!(Exit, with_evaluated: true)).expect("state created");
        let result = state.step(&ctx);

        assert_eq!(result.status, CommandStatus::Running);
        assert!(result.update.is_none(), "walking Exit does not pre-stop");
        assert_eq!(result.events.len(), 1);
        match &result.events[0] {
            CommandEvent::ActivateEntrance {
                object_id,
                caller,
                on_result,
                ..
            } => {
                assert_eq!(*object_id, container_id);
                assert_eq!(*caller, actor_id);
                assert_eq!(on_result, &Some(CallResultAction::ResolveExitActivation));
            }
            other => panic!("unexpected event: {other:?}"),
        }

        let mut stack = CommandStack::new();
        stack
            .push_back(request!(Exit, with_evaluated: true))
            .expect("Exit command queues");
        let armed = stack.execute_front(&ctx).expect("Exit arms activation");
        assert!(matches!(
            armed.events.as_slice(),
            [CommandEvent::ActivateEntrance { .. }]
        ));
        assert!(stack.resolve_exit_activation(false, 0).is_some());
        assert!(stack.finished_front_view().is_some());
    }

    #[test]
    fn exit_rechecks_an_opened_door_before_its_interval_expires() {
        // C4Command::Exit executes every frame; UpdateInterval is command
        // lifetime, not a polling delay (C4Command.cpp:624-650,1545-1555).
        // That matters for HUT3's nine-frame OpenDoor and forty-frame
        // DoorOpen window (Hut3.c4d/ActMap.txt:2-26).
        let actor_id = ObjectId::new(93);
        let container_id = ObjectId::new(103);
        let actor = command_object!(actor_id.as_u64(); container = Some(container_id));
        let mut container = command_object!(container_id.as_u64(); entrance_status = false);
        container.ocf |= ocf::ENTRANCE;
        let mut objects = command_objects([actor, container]);
        let mut state = ExitState::from_request(
            &request!(Exit, with_update_interval: 50, with_evaluated: true),
        )
        .expect("state created");

        let actor_snapshot = objects.get(&actor_id).expect("actor present");
        let first_ctx = command_ctx(actor_snapshot, &objects, 100);
        let first = state.step(&first_ctx);
        assert!(matches!(
            first.events.as_slice(),
            [CommandEvent::ActivateEntrance { .. }]
        ));

        objects
            .get_mut(&container_id)
            .expect("container present")
            .entrance_status = true;
        let actor_snapshot = objects.get(&actor_id).expect("actor present");
        let next_ctx = command_ctx(actor_snapshot, &objects, 101);
        let next = state.step(&next_ctx);
        assert_eq!(next.status, CommandStatus::Running);
        assert!(matches!(
            next.events.as_slice(),
            [CommandEvent::CommandExitObject { .. }]
        ));
    }

    #[test]
    fn exit_stops_building_then_rechecks_callback_mutated_containment() {
        let actor_id = ObjectId::new(110);
        let container_id = ObjectId::new(120);
        let callback_container_id = ObjectId::new(121);

        let actor = command_object!(actor_id.as_u64(); container = Some(container_id);
            action_procedure = ActionProcedure::Build);

        let container = command_object!(container_id.as_u64(); position = Vector2::new(0, 0);
            entrance_status = true);

        let objects = command_objects([actor.clone(), container]);

        let ctx = command_context!(command_ctx(objects.get(&actor_id).expect("actor present"), &objects, 30); position: actor.position);

        let mut state =
            ExitState::from_request(&request!(Exit, with_evaluated: true)).expect("state created");

        let stop = state.step(&ctx);
        assert_eq!(stop.status, CommandStatus::Running);
        assert!(stop.update.is_none(), "ObjectComStop must run live");
        assert_eq!(
            stop.events,
            [CommandEvent::ObjectComStopExit {
                object_id: actor_id,
                command_instance_id: 0,
            }]
        );

        // SetAction(Idle/Walk) may execute script callbacks. Native resumes
        // this same Exit invocation only after those callbacks return, so it
        // must re-read Contained and the actor coordinates rather than use
        // the pre-stop snapshots (C4Command.cpp:624-653).
        let mut callback_actor = actor.clone();
        callback_actor.container = Some(callback_container_id);
        callback_actor.action_procedure = ActionProcedure::Walk;
        callback_actor.position = Vector2::new(31, 41);
        let callback_container = command_object!(callback_container_id.as_u64();
            entrance_status = true);
        let callback_objects = command_objects([callback_actor.clone(), callback_container]);
        let callback_ctx = command_ctx(&callback_actor, &callback_objects, 30);

        let resumed = state.resume_after_stop(&callback_ctx);
        assert_eq!(resumed.status, CommandStatus::Running);
        assert!(resumed.update.is_none());
        assert_eq!(
            resumed.events,
            [CommandEvent::CommandExitObject {
                object_id: actor_id,
                previous_container: callback_container_id,
                position: callback_actor.position,
                jump_after: false,
                command_instance_id: 0,
            }]
        );

        // ClearCommands from either action callback unlinks the executing
        // Exit, but its iExec lifetime continues through the rest of this
        // invocation. A same-type replacement must not inherit the old
        // completion, including through the legacy zero-token seam.
        let mut stack = CommandStack::new();
        stack
            .push_front(request!(Exit, with_evaluated: true))
            .expect("original Exit queues");
        let original_instance_id = stack.entries.front().expect("original Exit").instance_id;
        let stop = stack
            .execute_front(&ctx)
            .expect("original Exit stops Build");
        assert!(matches!(
            stop.events.as_slice(),
            [CommandEvent::ObjectComStopExit {
                command_instance_id,
                ..
            }] if *command_instance_id == original_instance_id
        ));
        stack.clear();
        stack
            .push_front(request!(Exit, with_evaluated: true))
            .expect("callback replacement Exit queues");
        let replacement_instance_id = stack.entries.front().expect("replacement Exit").instance_id;
        assert_ne!(replacement_instance_id, original_instance_id);

        let resumed = stack
            .execute_pending_exit_stop(&callback_ctx, 0)
            .expect("detached original Exit resumes");
        let [CommandEvent::CommandExitObject {
            command_instance_id,
            ..
        }] = resumed.events.as_slice()
        else {
            panic!("unexpected detached Exit events: {:?}", resumed.events);
        };
        assert_eq!(*command_instance_id, original_instance_id);
        assert!(
            !stack.finish_command_instance(CommandId::Exit, *command_instance_id),
            "the detached original is no longer in the visible stack"
        );
        let replacement = stack.entries.front().expect("replacement remains");
        assert_eq!(replacement.instance_id, replacement_instance_id);
        assert_eq!(replacement.finished, None);
    }

    #[test]
    fn attack_completes_when_target_lacks_cached_crew_ocf() {
        let attacker_id = ObjectId::new(7);
        let target_id = ObjectId::new(8);

        let attacker = command_object!(attacker_id.as_u64();
            command_direction = CommandDirection::Right);

        let mut target = command_object!(target_id.as_u64(); crew_member = true);
        target.ocf &= !ocf::CREW_MEMBER;

        let objects = command_objects([attacker.clone(), target]);

        let ctx = command_context!(command_ctx(
            objects.get(&attacker_id).expect("attacker present"),
            &objects,
            0,
        ); position: attacker.position);

        let mut state = AttackState::from_request(&request!(Attack, with_target: Some(target_id)))
            .expect("state created");

        let result = state.step(&ctx);
        assert_eq!(result.status, CommandStatus::Completed);
        assert!(result.update.is_none());
    }

    #[test]
    fn attack_moves_to_target_coordinates_without_range_or_cooldown() {
        let attacker_id = ObjectId::new(30);
        let target_id = ObjectId::new(40);

        let attacker = command_object!(attacker_id.as_u64();
            command_direction = CommandDirection::Left);

        let mut target = command_object!(target_id.as_u64(); crew_member = true);
        target.ocf |= ocf::CREW_MEMBER;
        target.position = Vector2::new(5, -5);

        let objects = command_objects([attacker.clone(), target]);

        let ctx = command_context!(command_ctx(
            objects.get(&attacker_id).expect("attacker present"),
            &objects,
            0,
        ); position: attacker.position);

        let parent_request = request!(Attack, with_target: Some(target_id));
        let mut state = AttackState::from_request(&parent_request).expect("state created");

        let result = state.step(&ctx);
        assert_eq!(result.status, CommandStatus::Running);
        assert!(result.update.is_none());
        assert_eq!(result.operations.len(), 1);
        match &result.operations[0] {
            CommandOperation::PushFront(request) => {
                assert_eq!(request.id, CommandId::MoveTo);
                assert_eq!(request.target, None);
                assert_eq!((request.tx, request.ty), (Some(5), Some(-5)));
                assert_eq!(request.update_interval, 10);
            }
            other => panic!("expected move request, got {:?}", other),
        }
        assert_silent_child_failure_propagates(
            parent_request,
            pushed_request(&result.operations, CommandId::MoveTo),
            &ctx,
        );

        let repeated = state.step(&ctx);
        assert_eq!(
            pushed_request(&repeated.operations, CommandId::MoveTo).tx,
            Some(5),
            "Attack re-evaluates every execution without an invented cooldown"
        );
    }

    #[test]
    fn attack_throws_the_first_projectile_before_following_containment() {
        let attacker_id = ObjectId::new(50);
        let target_id = ObjectId::new(51);
        let ordinary_id = ObjectId::new(52);
        let first_projectile_id = ObjectId::new(53);
        let second_projectile_id = ObjectId::new(54);
        let attacker = command_object!(attacker_id.as_u64(); container = Some(ObjectId::new(60));
            contents = vec![ordinary_id, first_projectile_id, second_projectile_id]);
        let mut target = snapshot_with_id(target_id.as_u64());
        target.ocf |= ocf::CREW_MEMBER;
        target.container = Some(ObjectId::new(61));
        target.position = Vector2::new(123, -45);
        let ordinary = command_object!(ordinary_id.as_u64(); definition_id = "WOOD".into());
        let first_projectile = command_object!(first_projectile_id.as_u64();
            definition_id = "ROCK".into());
        let second_projectile = command_object!(second_projectile_id.as_u64();
            definition_id = "FLNT".into());
        let objects = command_objects([
            attacker.clone(),
            target,
            ordinary,
            first_projectile,
            second_projectile,
        ]);
        let definitions = HashMap::from([
            ("WOOD".into(), CommandDefinitionSnapshot::default()),
            ("ROCK".into(), command_definition! { projectile: -2 }),
            ("FLNT".into(), command_definition! { projectile: 1 }),
        ]);
        let ctx = command_ctx_with_definitions(&attacker, &objects, &definitions, 0);
        let mut state = AttackState::from_request(&request!(Attack, with_target: Some(target_id)))
            .expect("state created");

        let result = state.step(&ctx);

        let throw = pushed_request(&result.operations, CommandId::Throw);
        assert_eq!(throw.target, Some(first_projectile_id));
        assert_eq!((throw.tx, throw.ty), (Some(123), Some(-45)));
        assert_eq!(throw.update_interval, 2);
        assert_eq!(throw.mode, CommandMode::SilentSub);
    }

    #[test]
    fn attack_aligns_containment_before_moving() {
        let attacker_id = ObjectId::new(70);
        let target_id = ObjectId::new(71);
        let actor_container = ObjectId::new(72);
        let target_container = ObjectId::new(73);
        let mut attacker = command_object!(attacker_id.as_u64(); container = Some(actor_container));
        let mut target = snapshot_with_id(target_id.as_u64());
        target.ocf |= ocf::CREW_MEMBER;
        target.container = Some(target_container);
        let mut objects = command_objects([attacker.clone(), target.clone()]);
        let request = request!(Attack, with_target: Some(target_id));

        let ctx = command_ctx(&attacker, &objects, 0);
        let mut exiting = AttackState::from_request(&request).expect("state created");
        let exit = pushed_request(&exiting.step(&ctx).operations, CommandId::Exit);
        assert_eq!(exit.target, None);
        assert_eq!(exit.update_interval, 10);
        assert_eq!(exit.mode, CommandMode::SilentSub);

        attacker.container = None;
        objects.insert(attacker_id, attacker.clone());
        let ctx = command_ctx(&attacker, &objects, 1);
        let mut entering = AttackState::from_request(&request).expect("state created");
        let enter = pushed_request(&entering.step(&ctx).operations, CommandId::Enter);
        assert_eq!(enter.target, Some(target_container));
        assert_eq!(enter.update_interval, 10);
        assert_eq!(enter.mode, CommandMode::SilentSub);

        attacker.container = Some(target_container);
        objects.insert(attacker_id, attacker.clone());
        target.container = Some(target_container);
        objects.insert(target_id, target);
        let ctx = command_ctx(&attacker, &objects, 2);
        let mut moving = AttackState::from_request(&request).expect("state created");
        let move_to = pushed_request(&moving.step(&ctx).operations, CommandId::MoveTo);
        assert_eq!(move_to.target, None);
        assert_eq!(move_to.update_interval, 10);
    }

    #[test]
    fn call_accepts_empty_function_name_and_fails_during_execution() {
        let actor = snapshot_with_id(1);
        let target = snapshot_with_id(2);
        let objects = command_objects([actor.clone(), target.clone()]);
        let ctx = command_ctx(&actor, &objects, 0);
        let request = request!(Call, with_target: Some(target.id));
        let mut state = CallState::from_request(&request).expect("textless Call materializes");

        let result = state.step(&ctx);
        assert_eq!(result.status, CommandStatus::Failed);
        assert!(result.update.is_none());
        assert!(result.events.is_empty());
    }

    #[test]
    fn call_restore_without_exact_tx_value_uses_the_legacy_integer_projection() {
        let mut stack = CommandStack::new();
        stack
            .push_front(
                request!(Call, with_target: Some(ObjectId::new(2)), with_tx: Some(37), with_data: CommandData::Text("Work".into())),
            )
            .expect("Call queues");

        fn remove_exact_tx_values(value: &mut serde_json::Value) {
            match value {
                serde_json::Value::Object(fields) => {
                    fields.remove("tx_value");
                    for value in fields.values_mut() {
                        remove_exact_tx_values(value);
                    }
                }
                serde_json::Value::Array(values) => {
                    for value in values {
                        remove_exact_tx_values(value);
                    }
                }
                _ => {}
            }
        }

        let mut encoded = serde_json::to_value(stack.snapshot()).expect("snapshot serializes");
        remove_exact_tx_values(&mut encoded);
        let decoded: CommandStackSnapshot =
            serde_json::from_value(encoded).expect("legacy snapshot deserializes");
        let mut restored = CommandStack::new();
        restored.restore_from_snapshot(&decoded);

        let view = &restored.command_views()[0];
        assert_eq!(view.tx, Some(37));
        assert_eq!(view.tx_value, Some(clonk_script::Value::Int(37)));
    }

    #[test]
    fn call_emits_event_and_completes() {
        let builder_id = ObjectId::new(1);
        let target_id = ObjectId::new(2);
        let target2_id = ObjectId::new(3);

        let builder =
            command_object!(builder_id.as_u64(); command_direction = CommandDirection::Right);

        // C4Command::Call only requires a non-null Target pointer; it does
        // not require Alive or C4OS_NORMAL (C4Command.cpp:2355-2365,
        // C4Object.cpp:2224-2227). Real targets include inactive objects and
        // nonliving structures such as Tutorial07's WRKS.
        let target = command_object!(target_id.as_u64(); alive = false;
            status = crate::ObjectStatus::Inactive);

        let objects = command_objects([builder.clone(), target.clone()]);

        let ctx = command_context!(command_ctx(
            objects.get(&builder_id).expect("builder present"),
            &objects,
            0,
        ); position: builder.position);

        let mut state = CallState::from_request(
            &request!(Call, with_target: Some(target_id), with_target2: Some(target2_id), with_tx_value: clonk_script::Value::Array(vec![
                clonk_script::Value::Int(42),
                clonk_script::Value::String("tagged".into()),
            ]), with_ty: Some(7), with_data: CommandData::Text("ControlCall".into())),
        )
        .expect("state created");

        let result = state.step(&ctx);
        assert_eq!(result.status, CommandStatus::Completed);
        assert!(result.operations.is_empty());
        assert!(
            result.update.is_none(),
            "successful Call does not stop ComDir"
        );
        assert_eq!(result.events.len(), 1);
        match &result.events[0] {
            CommandEvent::CallObjectFunction {
                object_id,
                function,
                caller,
                tx,
                tx_value,
                tx_definition,
                ty,
                target2,
                on_result,
            } => {
                assert_eq!(*object_id, target_id);
                assert_eq!(function, "ControlCall");
                assert_eq!(*caller, builder_id);
                assert_eq!(*tx, None);
                assert_eq!(
                    tx_value,
                    &Some(clonk_script::Value::Array(vec![
                        clonk_script::Value::Int(42),
                        clonk_script::Value::String("tagged".into()),
                    ]))
                );
                assert!(tx_definition.is_none());
                assert_eq!(*ty, Some(7));
                assert_eq!(*target2, Some(target2_id));
                assert!(on_result.is_none());
            }
            other => panic!("unexpected event: {:?}", other),
        }

        let second = state.step(&ctx);
        assert_eq!(second.status, CommandStatus::Completed);
        assert!(second.events.is_empty());
    }

    #[test]
    fn context_requires_target_object() {
        let request = request!(Context);
        assert!(ContextState::from_request(&request).is_err());
    }

    #[test]
    fn context_emits_menu_request() {
        let crew_id = ObjectId::new(77);
        let target_id = ObjectId::new(88);

        let crew = command_object!(crew_id.as_u64(); owner = 42;
            command_direction = CommandDirection::Left);

        let target = snapshot_with_id(target_id.as_u64());

        let objects = command_objects([crew.clone(), target.clone()]);

        let ctx = command_context!(command_ctx(objects.get(&crew_id).expect("crew present"), &objects, 0); position: crew.position);

        let mut state = ContextState::from_request(
            &request!(Context, with_target2: Some(target_id), with_tx: Some(15), with_ty: Some(25)),
        )
        .expect("state created");

        let result = state.step(&ctx);
        assert_eq!(result.status, CommandStatus::Completed);
        let update = result.update.expect("context should stop crew");
        assert_eq!(update.command_direction, Some(CommandDirection::Stop));
        assert!(result.operations.is_empty());
        assert_eq!(result.events.len(), 1);
        match &result.events[0] {
            CommandEvent::OpenMenu(request) => {
                assert_eq!(request.crew_id, crew_id);
                assert_eq!(request.owner, 42);
                match &request.kind {
                    MenuRequestKind::Context { target, position } => {
                        assert_eq!(*target, target_id);
                        assert_eq!(*position, Some(Vector2::new(15, 25)));
                    }
                    other => panic!("unexpected menu kind: {:?}", other),
                }
            }
            other => panic!("unexpected event: {:?}", other),
        }

        let second = state.step(&ctx);
        assert_eq!(second.status, CommandStatus::Completed);
        assert!(second.events.is_empty());
    }

    #[test]
    fn context_skips_menu_when_owner_none() {
        let crew_id = ObjectId::new(101);
        let target_id = ObjectId::new(202);

        let crew = command_object!(crew_id.as_u64(); owner = OWNER_NONE;
            command_direction = CommandDirection::Right);

        let target = snapshot_with_id(target_id.as_u64());

        let objects = command_objects([crew.clone(), target.clone()]);

        let ctx = command_context!(command_ctx(objects.get(&crew_id).expect("crew present"), &objects, 0); position: crew.position);

        let mut state =
            ContextState::from_request(&request!(Context, with_target2: Some(target_id)))
                .expect("state created");

        let result = state.step(&ctx);
        assert_eq!(result.status, CommandStatus::Completed);
        assert!(result.update.is_some());
        assert!(result.events.is_empty());
    }

    #[test]
    fn take_opens_activate_menu() {
        let crew_id = ObjectId::new(101);
        let container_id = ObjectId::new(102);

        let crew = command_object!(crew_id.as_u64(); owner = OWNER_NONE; controller = 23;
            command_direction = CommandDirection::Left; container = Some(container_id));
        let container = snapshot_with_id(container_id.as_u64());

        let objects = command_objects([crew.clone(), container]);

        let ctx = command_context!(command_ctx(objects.get(&crew_id).expect("crew present"), &objects, 0); position: crew.position);

        let mut state = TakeState::from_request(&request!(Take)).expect("take state");

        let result = state.step(&ctx);
        assert_eq!(result.status, CommandStatus::Completed);
        assert!(result.update.is_none());
        assert_eq!(result.events.len(), 1);
        match &result.events[0] {
            CommandEvent::OpenMenu(request) => {
                assert_eq!(request.crew_id, crew_id);
                assert_eq!(request.owner, crew.controller);
                assert!(matches!(request.kind, MenuRequestKind::Activate));
            }
            other => panic!("unexpected event: {:?}", other),
        }

        let second = state.step(&ctx);
        assert_eq!(second.status, CommandStatus::Completed);
        assert!(second.events.is_empty());
        assert!(second.update.is_none());
    }

    #[test]
    fn take2_uncontained_completes_without_recording_base_failure() {
        let crew_id = ObjectId::new(201);

        let crew = command_object!(crew_id.as_u64(); owner = 5;
            command_direction = CommandDirection::Right);

        let objects = command_objects([crew.clone()]);

        let ctx = command_context!(command_ctx(objects.get(&crew_id).expect("crew present"), &objects, 0); position: crew.position);

        let mut stack = CommandStack::new();
        stack
            .push_back(request!(Wait, with_mode: CommandMode::Base))
            .expect("base command queues");
        stack
            .push_front(request!(Take2, with_mode: CommandMode::SilentSub))
            .expect("take2 command queues");

        let result = stack.step(&ctx).expect("take2 executes");
        assert_eq!(result.status, CommandStatus::Completed);
        assert!(result.update.is_none());
        assert!(result.events.is_empty());

        let snapshot = stack.snapshot();
        assert_eq!(snapshot.commands.len(), 1);
        assert_eq!(snapshot.commands[0].failures, 0);
    }

    #[test]
    fn take2_opens_get_menu_for_container() {
        let crew_id = ObjectId::new(301);
        let container_id = ObjectId::new(302);

        let crew = command_object!(crew_id.as_u64(); owner = 9;
            command_direction = CommandDirection::Right; container = Some(container_id));

        let container = command_object!(container_id.as_u64(); owner = 9);

        let objects = command_objects([crew.clone(), container.clone()]);

        let ctx = command_context!(command_ctx(objects.get(&crew_id).expect("crew present"), &objects, 0); position: crew.position);

        let mut state = Take2State::from_request(&request!(Take2)).expect("take2 state");

        let result = state.step(&ctx);
        assert_eq!(result.status, CommandStatus::Completed);
        let update = result
            .update
            .expect("take2 should stop crew before opening menu");
        assert_eq!(update.command_direction, Some(CommandDirection::Stop));
        assert_eq!(result.events.len(), 1);
        match &result.events[0] {
            CommandEvent::OpenMenu(request) => {
                assert_eq!(request.crew_id, crew_id);
                assert_eq!(request.owner, crew.owner);
                match &request.kind {
                    MenuRequestKind::Get { container } => assert_eq!(*container, container_id),
                    other => panic!("unexpected menu kind: {:?}", other),
                }
            }
            other => panic!("unexpected event: {:?}", other),
        }

        let second = state.step(&ctx);
        assert_eq!(second.status, CommandStatus::Completed);
        assert!(second.events.is_empty());
    }

    #[test]
    fn transfer_requires_target() {
        let request = request!(Transfer);
        assert!(TransferState::from_request(&request).is_err());
    }

    #[test]
    fn transfer_requests_move_when_outside_zone() {
        let actor_id = ObjectId::new(10);
        let target_id = ObjectId::new(20);

        let actor = command_object!(actor_id.as_u64(); position = Vector2::new(0, 90);
            command_direction = CommandDirection::Right);

        let target = command_object!(target_id.as_u64(); position = Vector2::new(100, 90));

        let objects = command_objects([actor.clone(), target.clone()]);

        let mut transfer_zones = TransferZoneTable::default();
        transfer_zones.set(
            target_id,
            TransferZoneRect {
                x: 90,
                y: 80,
                width: 20,
                height: 20,
            },
        );

        let mut surface = vec![120; 200];
        surface[89] = 80;
        let landscape = crate::Landscape::new(200, surface).expect("transfer landscape");
        let ctx = command_context!(command_ctx(objects.get(&actor_id).expect("actor present"), &objects, 0); landscape: Some(&landscape),
        position: actor.position,
        transfer_zones: &transfer_zones);

        let parent_request = request!(Transfer, with_target: Some(target_id));
        let mut state = TransferState::from_request(&parent_request).expect("state created");

        let result = state.step(&ctx);
        assert_eq!(result.status, CommandStatus::Running);
        assert!(
            result.update.is_none(),
            "entry MoveTo leaves ComDir Right untouched"
        );
        assert_eq!(result.events.len(), 0);
        assert_eq!(result.operations.len(), 1);
        match &result.operations[0] {
            CommandOperation::PushFront(request) => {
                assert_eq!(request.id, CommandId::MoveTo);
                assert_eq!(request.tx, Some(89));
                assert_eq!(request.ty, Some(69));
                assert_eq!(request.update_interval, 25);
            }
            other => panic!("unexpected operation: {:?}", other),
        }
        assert_silent_child_failure_propagates(
            parent_request,
            pushed_request(&result.operations, CommandId::MoveTo),
            &ctx,
        );

        let ctx_next = command_context!(ctx; frame: 1);
        let next = state.step(&ctx_next);
        assert_eq!(next.status, CommandStatus::Running);
        assert!(
            next.update.is_none(),
            "reissued MoveTo leaves ComDir Right untouched"
        );
        assert!(next.events.is_empty());
        assert_eq!(next.operations.len(), 1);
        match &next.operations[0] {
            CommandOperation::PushFront(request) => {
                assert_eq!(request.id, CommandId::MoveTo);
                assert_eq!(request.tx, Some(89));
                assert_eq!(request.ty, Some(69));
                assert_eq!(request.update_interval, 25);
            }
            other => panic!("unexpected operation: {other:?}"),
        }
        let serialized = serde_json::to_value(&state).expect("transfer state serializes");
        assert!(
            serialized.get("last_move_order").is_none(),
            "Transfer carries no invented move cooldown state"
        );

        let mut blocked_landscape =
            crate::Landscape::new(200, vec![0; 200]).expect("fully solid transfer perimeter");
        blocked_landscape.set_world_height(200);
        let blocked_ctx = command_context!(ctx_next; landscape: Some(&blocked_landscape),
        frame: 1);
        let blocked = state.step(&blocked_ctx);
        assert_eq!(
            blocked.status,
            CommandStatus::Failed,
            "entry-point failure is immediate even after consecutive move orders"
        );
        assert!(blocked.update.is_none());
        assert!(blocked.events.is_empty());
        assert!(blocked.operations.is_empty());
    }

    #[test]
    fn negative_transfer_zone_command_entry_fails_without_panicking() {
        let actor_id = ObjectId::new(10);
        let target_id = ObjectId::new(20);
        let actor = command_object!(actor_id.as_u64(); position = Vector2::new(20, 5));
        let target = snapshot_with_id(target_id.as_u64());
        let objects = command_objects([actor.clone(), target]);
        let mut transfer_zones = TransferZoneTable::default();
        transfer_zones.set(
            target_id,
            TransferZoneRect {
                x: 5,
                y: 5,
                width: -2,
                height: -2,
            },
        );
        let ctx =
            command_context!(command_ctx(&actor, &objects, 0); transfer_zones: &transfer_zones);
        let mut state =
            TransferState::from_request(&request!(Transfer, with_target: Some(target_id)))
                .expect("state created");

        let result = state.step(&ctx);

        assert_eq!(result.status, CommandStatus::Failed);
        assert!(result.operations.is_empty());
        assert!(result.events.is_empty());
    }

    #[test]
    fn transfer_emits_control_transfer_event() {
        let actor_id = ObjectId::new(100);
        let target_id = ObjectId::new(200);

        let actor = command_object!(actor_id.as_u64(); position = Vector2::new(95, 0);
            command_direction = CommandDirection::Right);

        let target = command_object!(target_id.as_u64(); position = Vector2::new(100, 0));

        let objects = command_objects([actor.clone(), target.clone()]);

        let mut transfer_zones = TransferZoneTable::default();
        transfer_zones.set(
            target_id,
            TransferZoneRect {
                x: 90,
                y: -10,
                width: 20,
                height: 40,
            },
        );

        let mut state = TransferState::from_request(
            &request!(Transfer, with_target: Some(target_id), with_tx: Some(42), with_ty: Some(-5)),
        )
        .expect("state created");

        let ctx = command_context!(command_ctx(objects.get(&actor_id).expect("actor present"), &objects, 5); position: actor.position,
        transfer_zones: &transfer_zones);

        let result = state.step(&ctx);
        assert_eq!(result.status, CommandStatus::Running);
        assert!(result.update.is_none());
        assert_eq!(result.operations.len(), 0);
        assert_eq!(result.events.len(), 1);
        match &result.events[0] {
            CommandEvent::ControlTransfer {
                object_id,
                caller,
                tx_value,
                ty,
                command_instance_id,
            } => {
                assert_eq!(*object_id, target_id);
                assert_eq!(*caller, actor_id);
                assert_eq!(tx_value, &clonk_script::Value::Int(42));
                assert_eq!(*ty, -5);
                assert_eq!(*command_instance_id, 0);
            }
            other => panic!("unexpected event: {:?}", other),
        }

        let repeated_same_tick = state.step(&ctx);
        assert!(
            matches!(
                repeated_same_tick.events.as_slice(),
                [CommandEvent::ControlTransfer { .. }]
            ),
            "Tick5 is a frame predicate, not a per-command cooldown"
        );

        let ctx_next = command_context!(command_ctx(objects.get(&actor_id).expect("actor present"), &objects, 6); position: actor.position,
        transfer_zones: &transfer_zones);

        let follow_up = state.step(&ctx_next);
        assert_eq!(follow_up.status, CommandStatus::Running);
        assert!(
            follow_up.update.is_none(),
            "non-Tick5 Transfer leaves ComDir Right untouched"
        );
        assert!(follow_up.events.is_empty());

        let tagged_tx = clonk_script::Value::C4Id("GOLD".to_string());
        let mut tagged_state = TransferState::from_request(
            &request!(Transfer, with_target: Some(target_id), with_tx_value: tagged_tx.clone()),
        )
        .expect("tagged Transfer state");
        let tagged = tagged_state.step(&ctx);
        assert!(matches!(
            tagged.events.as_slice(),
            [CommandEvent::ControlTransfer { tx_value, .. }] if tx_value == &tagged_tx
        ));
    }

    #[test]
    fn transfer_fails_without_zone() {
        let actor_id = ObjectId::new(123);
        let target_id = ObjectId::new(456);

        let actor = command_object!(actor_id.as_u64(); position = Vector2::new(0, 0);
            command_direction = CommandDirection::Right);

        let target = command_object!(target_id.as_u64(); position = Vector2::new(10, 0));

        let objects = command_objects([actor.clone(), target.clone()]);

        let ctx = command_context!(command_ctx(objects.get(&actor_id).expect("actor present"), &objects, 0); position: actor.position);

        let mut state =
            TransferState::from_request(&request!(Transfer, with_target: Some(target_id)))
                .expect("state created");

        let result = state.step(&ctx);
        assert_eq!(result.status, CommandStatus::Failed);
        assert!(
            result.update.is_none(),
            "missing-zone failure leaves ComDir Right untouched"
        );

        let mut missing_target_state =
            TransferState::from_request(&request!(Transfer, with_target: Some(ObjectId::new(789))))
                .expect("state created");
        let missing_target = missing_target_state.step(&ctx);
        assert_eq!(missing_target.status, CommandStatus::Failed);
        assert!(
            missing_target.update.is_none(),
            "missing-target failure leaves ComDir Right untouched"
        );
    }

    #[test]
    fn build_queues_activate_for_internal_vehicle() {
        let builder_id = ObjectId::new(1);
        let target_id = ObjectId::new(2);

        let mut builder =
            command_object!(builder_id.as_u64(); command_direction = CommandDirection::Right);
        builder.physical.can_construct = 1;

        let target = command_object!(target_id.as_u64(); construction = FULL_CON;
            category = CATEGORY_VEHICLE; container = Some(builder_id));

        let objects = command_objects([builder.clone(), target]);

        let ctx = command_ctx(&builder, &objects, 0);

        let mut state = BuildState::from_request(&request!(Build, with_target: Some(target_id)))
            .expect("build state");

        let result = state.step(&ctx);
        assert_eq!(result.status, CommandStatus::Completed);
        assert_eq!(result.operations.len(), 1);
        match &result.operations[0] {
            CommandOperation::PushFront(request) => {
                assert_eq!(request.id, CommandId::Activate);
                assert_eq!(request.target, Some(target_id));
            }
            other => panic!("expected activate request, got {:?}", other),
        }
    }

    // C4Command::Build (C4Command.cpp:823-899) accepts every extant target,
    // not only living objects. Its same-container arm at :887 is guarded by
    // Target->Contained, so two uncontained objects still have to approach.
    #[test]
    fn build_moves_to_uncontained_nonliving_structure() {
        let builder_id = ObjectId::new(1);
        let target_id = ObjectId::new(2);

        let mut builder = command_object!(builder_id.as_u64(); position = Vector2::new(0, 0));
        builder.physical.can_construct = 1;

        let target = command_object!(target_id.as_u64(); position = Vector2::new(100, 0);
            status = ObjectStatus::Normal; alive = false; category = CATEGORY_STRUCTURE;
            construction = FULL_CON * 4 / 5);

        let objects = command_objects([builder.clone(), target]);

        let ctx = command_ctx(&builder, &objects, 0);

        let mut state = BuildState::from_request(&request!(Build, with_target: Some(target_id)))
            .expect("build state");

        let result = state.step(&ctx);
        assert_eq!(result.status, CommandStatus::Running);
        assert!(result.update.is_none(), "must not build remotely");
        assert_eq!(result.operations.len(), 1);
        match &result.operations[0] {
            CommandOperation::PushFront(request) => {
                assert_eq!(request.id, CommandId::MoveTo);
                assert_eq!(request.target, None);
                assert_eq!(request.tx, Some(100));
                assert_eq!(request.ty, Some(0));
            }
            other => panic!("expected MoveTo request, got {other:?}"),
        }
        let first_move = pushed_request(&result.operations, CommandId::MoveTo);
        let reissued = state.step(&ctx);
        assert_eq!(
            pushed_request(&reissued.operations, CommandId::MoveTo),
            first_move,
            "Build reissues MoveTo on its next execution"
        );
    }

    #[test]
    fn build_enters_the_container_of_an_internal_target() {
        // C4Command::Build ignores Tx/Ty and, when the incomplete target is
        // contained elsewhere, enters Target->Contained rather than walking
        // to the placeholder coordinates (C4Command.cpp:823-899). Workshop
        // passes explicit zero slots in AddCommand(...,"Build",pToBuild,...)
        // (Objects.c4d/Structures.c4d/Workshop.c4d/Script.c:76-91).
        let builder_id = ObjectId::new(1);
        let target_id = ObjectId::new(2);
        let workshop_id = ObjectId::new(3);

        let mut builder = command_object!(builder_id.as_u64(); position = Vector2::new(8, 199));
        builder.physical.can_construct = 1;

        let target = command_object!(target_id.as_u64(); position = Vector2::new(150, 184);
            container = Some(workshop_id); alive = false; construction = FULL_CON / 100);

        let workshop = command_object!(workshop_id.as_u64(); position = target.position; alive = false;
            category = CATEGORY_STRUCTURE);

        let objects = command_objects([builder.clone(), target, workshop]);
        let ctx = command_ctx(&builder, &objects, 0);
        let mut state = BuildState::from_request(
            &request!(Build, with_target: Some(target_id), with_tx: Some(0), with_ty: Some(0)),
        )
        .expect("Build state");

        let result = state.step(&ctx);

        assert_eq!(result.status, CommandStatus::Running);
        match result.operations.as_slice() {
            [CommandOperation::PushFront(request)] => {
                assert_eq!(request.id, CommandId::Enter);
                assert_eq!(request.target, Some(workshop_id));
                assert_eq!(request.update_interval, 50);
            }
            other => panic!("expected Enter workshop request, got {other:?}"),
        }
    }

    #[test]
    fn activate_explicit_container_opens_menu_before_movement_logic() {
        let actor_id = ObjectId::new(1);
        let container_id = ObjectId::new(2);
        let actor = command_object!(actor_id.as_u64(); owner = 17; controller = 23;
            command_direction = CommandDirection::Right; action_procedure = ActionProcedure::Dig);
        let container = snapshot_with_id(container_id.as_u64());
        let objects = command_objects([actor.clone(), container]);
        let ctx = command_ctx(&actor, &objects, 0);

        let mut state =
            ActivateState::from_request(&request!(Activate, with_target2: Some(container_id)))
                .expect("activate state");
        let result = state.step(&ctx);

        assert_eq!(result.status, CommandStatus::Completed);
        assert!(result.update.is_none());
        assert!(result.operations.is_empty());
        assert_eq!(
            result.events,
            [CommandEvent::OpenMenu(MenuRequest {
                crew_id: actor_id,
                owner: actor.controller,
                kind: MenuRequestKind::ActivateTarget {
                    container: container_id,
                },
            })]
        );

        let mut stale = ActivateState::from_request(
            &request!(Activate, with_target2: Some(ObjectId::new(999))),
        )
        .expect("stale activate state");
        let stale_result = stale.step(&ctx);
        assert_eq!(stale_result.status, CommandStatus::Failed);
        assert!(stale_result.events.is_empty());

        let deleted_container =
            command_object!(container_id.as_u64(); status = ObjectStatus::Deleted);
        let deleted_objects = command_objects([actor.clone(), deleted_container]);
        let deleted_ctx = command_context!(ctx; objects: &deleted_objects);
        let mut retained_deleted =
            ActivateState::from_request(&request!(Activate, with_target2: Some(container_id)))
                .expect("deleted-target activate state");
        let deleted_result = retained_deleted.step(&deleted_ctx);
        assert_eq!(deleted_result.status, CommandStatus::Completed);
        assert!(matches!(
            deleted_result.events.as_slice(),
            [CommandEvent::OpenMenu(MenuRequest {
                kind: MenuRequestKind::ActivateTarget { container },
                ..
            })] if *container == container_id
        ));

        let mut cleared = CommandStack::new();
        cleared
            .push_front(request!(Activate, with_target2: Some(container_id)))
            .expect("Activate queues");
        assert!(cleared.clear_object_reference(container_id));
        assert_eq!(
            cleared
                .execute_front(&deleted_ctx)
                .map(|result| result.status),
            Some(CommandStatus::Failed)
        );
    }

    #[test]
    fn activate_completes_when_target_outside_container() {
        let actor_id = ObjectId::new(1);
        let target_id = ObjectId::new(2);

        let actor = snapshot_with_id(actor_id.as_u64());

        let target = command_object!(target_id.as_u64(); container = None);

        let objects = command_objects([actor.clone(), target]);

        let ctx = command_ctx(&actor, &objects, 0);

        let mut state =
            ActivateState::from_request(&request!(Activate, with_target: Some(target_id)))
                .expect("state created");

        let result = state.step(&ctx);
        assert_eq!(result.status, CommandStatus::Completed);
        assert!(result.operations.is_empty());
        assert!(result.events.is_empty());
    }

    #[test]
    fn activate_requests_enter_when_actor_outside_container() {
        let actor_id = ObjectId::new(10);
        let container_id = ObjectId::new(20);
        let target_id = ObjectId::new(30);

        let actor = command_object!(actor_id.as_u64(); position = Vector2::new(100, 0));

        let mut container = command_object!(container_id.as_u64(); position = Vector2::new(0, 0);
            ocf = ocf::ENTRANCE | ocf::AVAILABLE);
        container.contents.push(target_id);

        let target = command_object!(target_id.as_u64(); position = container.position;
            container = Some(container_id); collectible = true; construction = FULL_CON);

        let objects = command_objects([actor.clone(), container, target]);

        let ctx = command_ctx(&actor, &objects, 0);

        let mut state = ActivateState::from_request(
            &request!(Activate, with_target: Some(target_id), with_update_interval: 5),
        )
        .expect("state created");

        let result = state.step(&ctx);
        assert_eq!(result.status, CommandStatus::Running);
        assert_eq!(result.operations.len(), 1);
        match &result.operations[0] {
            CommandOperation::PushFront(request) => {
                assert_eq!(request.id, CommandId::Enter);
                assert_eq!(request.target, Some(container_id));
            }
            other => panic!("expected enter request, got {:?}", other),
        }
    }

    #[test]
    fn activate_type_lookup_waits_until_actor_enters_container() {
        let actor_id = ObjectId::new(40);
        let container_id = ObjectId::new(41);
        let exiting_id = ObjectId::new(42);

        let actor = snapshot_with_id(actor_id.as_u64());
        let mut container = command_object!(container_id.as_u64(); ocf = ocf::ENTRANCE);
        container.contents.push(exiting_id);
        let exiting = command_object!(exiting_id.as_u64(); definition_id = "FLNT".into();
            container = Some(container_id); commands = vec![command_view(CommandId::Exit, None)]);

        let objects = command_objects([actor.clone(), container, exiting]);
        let ctx = command_ctx(&actor, &objects, 0);
        let mut state = ActivateState::from_request(
            &request!(Activate, with_target2: Some(container_id), with_data: CommandData::Text("FLNT".into())),
        )
        .expect("Activate state");

        let result = state.step(&ctx);
        assert_eq!(result.status, CommandStatus::Running);
        assert_eq!(state.target, None, "type lookup remains live until entry");
        assert!(matches!(
            result.operations.as_slice(),
            [CommandOperation::PushFront(request)]
                if request.id == CommandId::Enter && request.target == Some(container_id)
        ));
    }

    #[test]
    fn activate_sets_exit_command_on_target_inside_container() {
        let actor_id = ObjectId::new(5);
        let container_id = ObjectId::new(6);
        let target_id = ObjectId::new(7);

        let actor = command_object!(actor_id.as_u64(); owner = 42; controller = 23;
            position = Vector2::new(15, 5); container = Some(container_id));

        let mut container = command_object!(container_id.as_u64(); position = Vector2::new(12, 4));
        container.contents.push(target_id);

        let target = command_object!(target_id.as_u64(); position = container.position;
            container = Some(container_id); collectible = true; construction = FULL_CON);

        let objects = command_objects([actor.clone(), container, target]);

        let ctx = command_ctx(&actor, &objects, 0);

        let mut state =
            ActivateState::from_request(&request!(Activate, with_target: Some(target_id)))
                .expect("state created");

        let result = state.step(&ctx);
        assert_eq!(result.status, CommandStatus::Completed);
        assert_eq!(result.events.len(), 1);
        match &result.events[0] {
            CommandEvent::SetObjectCommand {
                object_id,
                controller,
                request,
            } => {
                assert_eq!(*object_id, target_id);
                assert_eq!(*controller, Some(23));
                assert_eq!(request.id, CommandId::Exit);
            }
            other => panic!("unexpected event: {:?}", other),
        }
    }

    #[test]
    fn activate_multi_count_releases_distinct_non_exiting_targets_in_one_execute() {
        let actor_id = ObjectId::new(50);
        let container_id = ObjectId::new(51);
        let already_exiting_id = ObjectId::new(52);
        let deeper_exit_id = ObjectId::new(53);
        let empty_stack_id = ObjectId::new(54);

        let actor =
            command_object!(actor_id.as_u64(); controller = 23; container = Some(container_id));

        let container = command_object!(container_id.as_u64();
            contents = vec![already_exiting_id, deeper_exit_id, empty_stack_id]);

        let already_exiting = command_object!(already_exiting_id.as_u64();
            definition_id = "FLNT".into(); container = Some(container_id);
            commands = vec![command_view(CommandId::Exit, None)]);

        let mut deeper_exit = command_object!(deeper_exit_id.as_u64(); definition_id = "FLNT".into();
            container = Some(container_id));
        deeper_exit.commands = vec![
            command_view(CommandId::Wait, None),
            command_view(CommandId::Exit, None),
        ];

        let empty_stack = command_object!(empty_stack_id.as_u64(); definition_id = "FLNT".into();
            container = Some(container_id));

        let objects = command_objects([
            actor.clone(),
            container,
            already_exiting,
            deeper_exit,
            empty_stack,
        ]);
        let ctx = command_ctx(&actor, &objects, 0);

        let mut stack = CommandStack::new();
        stack
            .push_front(
                request!(Activate, with_target2: Some(container_id), with_tx: Some(2), with_data: CommandData::Text("FLNT".into())),
            )
            .expect("multi-count Activate queues");

        let result = stack.step(&ctx).expect("Activate executes once");
        assert_eq!(result.status, CommandStatus::Completed);
        assert!(result.operations.is_empty());
        let released = result
            .events
            .iter()
            .map(|event| match event {
                CommandEvent::SetObjectCommand {
                    object_id,
                    controller,
                    request,
                } => {
                    assert_eq!(*controller, Some(actor.controller));
                    assert_eq!(request.id, CommandId::Exit);
                    assert_eq!(request.mode, CommandMode::Base);
                    *object_id
                }
                other => panic!("unexpected Activate event: {other:?}"),
            })
            .collect::<Vec<_>>();
        assert_eq!(released, [deeper_exit_id, empty_stack_id]);
        assert!(
            stack.is_empty(),
            "Finish(true) removes Activate immediately"
        );
        assert_eq!(
            stack.take_successful_finishes(),
            [CommandId::Activate],
            "the whole release loop has one successful finish"
        );
    }

    #[test]
    fn activate_multi_count_partial_failure_keeps_prior_release() {
        let actor_id = ObjectId::new(60);
        let container_id = ObjectId::new(61);
        let target_id = ObjectId::new(62);

        let actor =
            command_object!(actor_id.as_u64(); controller = 9; container = Some(container_id));
        let mut container = snapshot_with_id(container_id.as_u64());
        container.contents.push(target_id);
        let target = command_object!(target_id.as_u64(); definition_id = "FLNT".into();
            container = Some(container_id));

        let objects = command_objects([actor.clone(), container, target]);
        let ctx = command_ctx(&actor, &objects, 0);
        let mut state = ActivateState::from_request(
            &request!(Activate, with_target2: Some(container_id), with_tx: Some(2), with_data: CommandData::Text("FLNT".into())),
        )
        .expect("Activate state");

        let result = state.step(&ctx);
        assert_eq!(result.status, CommandStatus::Failed);
        assert_eq!(state.remaining, 1);
        assert_eq!(result.events.len(), 1);
        assert!(matches!(
            &result.events[0],
            CommandEvent::SetObjectCommand {
                object_id,
                controller: Some(9),
                request,
            } if *object_id == target_id && request.id == CommandId::Exit
        ));
    }
