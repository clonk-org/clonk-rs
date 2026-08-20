    // Contiguous slice 6 of 7 of the `command/tests` battery, spliced by
    // `include!` from the parent module so every test id is unchanged.

    #[test]
    fn build_queues_energy_for_structures_needing_power() {
        let builder_id = ObjectId::new(10);
        let target_id = ObjectId::new(20);

        let mut builder = snapshot_with_id(builder_id.as_u64());
        builder.physical.can_construct = 1;

        let target = command_object!(target_id.as_u64(); construction = FULL_CON;
            line_connect = LINE_CONNECT_POWER_INPUT);

        let objects = command_objects([builder.clone(), target]);

        let ctx =
            command_context!(command_ctx(&builder, &objects, 0); structures_need_energy: true);

        let mut state = BuildState::from_request(&request!(Build, with_target: Some(target_id)))
            .expect("build state");

        let result = state.step(&ctx);
        assert_eq!(result.status, CommandStatus::Completed);
        assert_eq!(result.operations.len(), 1);
        match &result.operations[0] {
            CommandOperation::PushFront(request) => {
                assert_eq!(request.id, CommandId::Energy);
                assert_eq!(request.target, Some(target_id));
            }
            other => panic!("expected energy request, got {:?}", other),
        }
    }

    #[test]
    fn build_zero_can_construct_reports_cannot_build() {
        let builder_id = ObjectId::new(10);
        let target_id = ObjectId::new(20);

        let mut builder = command_object!(builder_id.as_u64(); crew_member = true);
        builder.ocf |= ocf::CREW_MEMBER;
        let target = command_object!(target_id.as_u64(); construction = FULL_CON);
        let objects = command_objects([builder.clone(), target]);
        let ctx = command_ctx(&builder, &objects, 0);
        let mut stack = CommandStack::new();
        stack
            .push_front(request!(Build, with_target: Some(target_id), with_mode: CommandMode::Base))
            .expect("build command queued");

        let result = stack.execute_front(&ctx).expect("build executed");
        assert_eq!(result.status, CommandStatus::Failed);
        assert_eq!(
            result.failure_reason,
            Some(CommandFailureReason::CannotBuild)
        );
        assert!(result.update.is_none());
        assert!(result.operations.is_empty());
        assert_eq!(result.events.len(), 1);
        match &result.events[0] {
            CommandEvent::FailureFeedback { actor_id, feedback } => {
                assert_eq!(*actor_id, builder_id);
                assert_eq!(feedback.command.name, "Build");
                assert_eq!(feedback.command.target, Some(target_id));
                assert_eq!(feedback.reason, Some(CommandFailureReason::CannotBuild));
            }
            other => panic!("expected Build failure feedback, got {other:?}"),
        }
    }

    #[test]
    fn deferred_physical_waits_for_native_command_gates() {
        let actor_id = ObjectId::new(10);
        let target_id = ObjectId::new(20);
        let actor = command_object!(actor_id.as_u64(); physical_deferred = true;
            action_name = "Walk".into(); action_procedure = ActionProcedure::Walk);
        let target = snapshot_with_id(target_id.as_u64());

        let objects = command_objects([actor.clone()]);
        let ctx = command_ctx(&actor, &objects, 0);
        let mut missing_build = CommandStack::new();
        missing_build
            .push_front(request!(Build, with_target: Some(target_id)))
            .expect("Build queues");
        let result = missing_build
            .execute_front(&ctx)
            .expect("missing-target Build executes");
        assert_eq!(result.status, CommandStatus::Failed);
        assert!(result
            .events
            .iter()
            .all(|event| !matches!(event, CommandEvent::ResolveCommandPhysical { .. })));

        let objects = command_objects([actor.clone(), target]);
        let ctx = command_ctx(&actor, &objects, 0);
        let mut expired_build = CommandStack::new();
        expired_build
            .push_front(request!(Build, with_target: Some(target_id), with_update_interval: 1))
            .expect("expiring Build queues");
        let result = expired_build
            .execute_front(&ctx)
            .expect("expiring Build executes");
        assert_eq!(result.status, CommandStatus::Completed);
        assert!(result.events.is_empty());

        let mut untargeted_throw = CommandStack::new();
        untargeted_throw
            .push_front(request!(Throw))
            .expect("Throw queues");
        let result = untargeted_throw
            .execute_front(&ctx)
            .expect("untargeted Throw executes");
        assert!(result
            .events
            .iter()
            .all(|event| !matches!(event, CommandEvent::ResolveCommandPhysical { .. })));
    }

    #[test]
    fn build_resumes_detached_with_second_physical_read() {
        let actor_id = ObjectId::new(10);
        let target_id = ObjectId::new(20);
        let mut actor = command_object!(actor_id.as_u64(); physical_deferred = true);
        actor.physical.can_construct = 0;
        let target = command_object!(target_id.as_u64(); construction = FULL_CON);
        let objects = command_objects([actor.clone(), target]);
        let ctx = command_ctx(&actor, &objects, 0);

        let mut stack = CommandStack::new();
        stack
            .push_front(request!(Build, with_target: Some(target_id)))
            .expect("Build queues");
        let suspended = stack.execute_front(&ctx).expect("Build suspends");
        let (reads, command_instance_id) = match suspended.events.as_slice() {
            [CommandEvent::ResolveCommandPhysical {
                object_id,
                reads,
                command_instance_id,
            }] => {
                assert_eq!(*object_id, actor_id);
                (*reads, *command_instance_id)
            }
            events => panic!("expected physical resolution, got {events:?}"),
        };
        assert_eq!(reads, 2);
        assert_ne!(command_instance_id, 0);

        stack.clear();
        assert!(stack.is_empty());
        let physical = PhysicalInfo {
            can_construct: 1,
            ..PhysicalInfo::default()
        };
        let resumed = stack
            .execute_pending_physical(
                &ctx,
                crate::PhysicsSettings::default().gravity_as_c4fixed(),
                command_instance_id,
                physical,
            )
            .expect("detached Build resumes");
        assert_eq!(resumed.status, CommandStatus::Completed);
        assert_eq!(stack.take_successful_finishes(), [CommandId::Build]);
    }

    #[test]
    fn build_skips_energy_already_commanded_for_target() {
        let builder_id = ObjectId::new(10);
        let target_id = ObjectId::new(20);
        let other_id = ObjectId::new(30);

        let mut builder = snapshot_with_id(builder_id.as_u64());
        builder.physical.can_construct = 1;
        let target = command_object!(target_id.as_u64(); construction = FULL_CON;
            line_connect = LINE_CONNECT_POWER_INPUT);
        let other = command_object!(other_id.as_u64();
            commands = vec![command_view(CommandId::Energy, Some(target_id))]);
        let objects = command_objects([builder.clone(), target, other]);
        let ctx =
            command_context!(command_ctx(&builder, &objects, 0); structures_need_energy: true);
        let mut state = BuildState::from_request(&request!(Build, with_target: Some(target_id)))
            .expect("build state");

        let result = state.step(&ctx);
        assert_eq!(result.status, CommandStatus::Completed);
        assert!(result.operations.is_empty());
    }

    #[test]
    fn build_reach_requires_walk_inside_target_shape() {
        let builder_id = ObjectId::new(10);
        let target_id = ObjectId::new(20);
        let run = |position: Vector2, procedure: ActionProcedure| {
            let mut builder = command_object!(builder_id.as_u64(); position = position);
            builder.physical.can_construct = 1;
            builder.action_procedure = procedure;
            let target = command_object!(target_id.as_u64(); position = Vector2::new(100, 100);
                shape = DefinitionRect::new(120, 90, 20, 20));
            let objects = command_objects([builder.clone(), target]);
            let ctx = command_ctx(&builder, &objects, 0);
            let mut state =
                BuildState::from_request(&request!(Build, with_target: Some(target_id)))
                    .expect("build state");
            state.step(&ctx)
        };

        let inside_walking = run(Vector2::new(125, 100), ActionProcedure::Walk);
        assert_eq!(inside_walking.status, CommandStatus::Running);
        assert!(inside_walking.operations.is_empty());
        assert_eq!(inside_walking.events.len(), 1);
        assert!(matches!(
            inside_walking.events[0],
            CommandEvent::ObjectComBuild {
                object_id,
                target_id: event_target,
                stop_first: true,
            } if object_id == builder_id && event_target == target_id
        ));

        let inside_not_walking = run(Vector2::new(125, 100), ActionProcedure::Undefined);
        assert!(inside_not_walking.events.is_empty());
        let move_request = pushed_request(&inside_not_walking.operations, CommandId::MoveTo);
        assert_eq!((move_request.tx, move_request.ty), (Some(100), Some(100)));
        assert_eq!(move_request.update_interval, 50);
        assert_eq!(move_request.mode, CommandMode::SilentSub);

        // This point is inside the old coarse +/-9 reach box but outside the
        // actual target shape, so it must still approach.
        let outside_walking = run(Vector2::new(105, 100), ActionProcedure::Walk);
        assert!(outside_walking.events.is_empty());
        let move_request = pushed_request(&outside_walking.operations, CommandId::MoveTo);
        assert_eq!((move_request.tx, move_request.ty), (Some(100), Some(100)));
    }

    #[test]
    fn build_push_requests_silent_ungrab() {
        let builder_id = ObjectId::new(10);
        let target_id = ObjectId::new(20);
        let mut builder = snapshot_with_id(builder_id.as_u64());
        builder.physical.can_construct = 1;
        builder.action_procedure = ActionProcedure::Push;
        let target = snapshot_with_id(target_id.as_u64());
        let objects = command_objects([builder.clone(), target]);
        let ctx = command_ctx(&builder, &objects, 0);
        let mut state = BuildState::from_request(&request!(Build, with_target: Some(target_id)))
            .expect("build state");

        let result = state.step(&ctx);
        let request = pushed_request(&result.operations, CommandId::UnGrab);
        assert_eq!(result.status, CommandStatus::Running);
        assert!(result.update.is_none());
        assert!(result.events.is_empty());
        assert_eq!(request.target, None);
        assert_eq!(request.update_interval, 50);
        assert_eq!(request.mode, CommandMode::SilentSub);
    }

    #[test]
    fn build_dig_stops_then_resumes_same_execute() {
        let builder_id = ObjectId::new(10);
        let target_id = ObjectId::new(20);
        let mut builder = command_object!(builder_id.as_u64(); position = Vector2::new(125, 100));
        builder.physical.can_construct = 1;
        builder.action_procedure = ActionProcedure::Dig;
        let target = command_object!(target_id.as_u64(); position = Vector2::new(100, 100);
            shape = DefinitionRect::new(120, 90, 20, 20));
        let mut objects = command_objects([builder.clone(), target.clone()]);
        let ctx = command_ctx(&builder, &objects, 0);
        let mut stack = CommandStack::new();
        stack
            .push_front(request!(Build, with_target: Some(target_id), with_mode: CommandMode::Base))
            .expect("build command queued");

        let stopped = stack.execute_front(&ctx).expect("dig build executed");
        assert_eq!(stopped.status, CommandStatus::Running);
        let command_instance_id = match stopped.events.as_slice() {
            [CommandEvent::ObjectComStopBuild {
                object_id,
                command_instance_id,
            }] if *object_id == builder_id => *command_instance_id,
            events => panic!("expected exact Build stop event, got {events:?}"),
        };
        let mut callback_replaced = stack.clone();
        callback_replaced.restore_from_snapshot(&CommandStack::new().snapshot());
        assert!(callback_replaced.is_empty());

        builder.action_name = "Walk".into();
        builder.action_procedure = ActionProcedure::Walk;
        objects.insert(builder_id, builder.clone());
        objects.insert(target_id, target);
        let walk_ctx = command_ctx(&builder, &objects, 0);
        let detached = callback_replaced
            .execute_pending_build_stop(&walk_ctx, command_instance_id)
            .expect("callback-replaced Build retained its native continuation");
        assert!(matches!(
            detached.events.as_slice(),
            [CommandEvent::ObjectComBuild {
                object_id,
                target_id: event_target,
                stop_first: true,
            }] if *object_id == builder_id && *event_target == target_id
        ));
        let resumed = stack
            .execute_pending_build_stop(&walk_ctx, command_instance_id)
            .expect("Build resumed after ObjectComStop");
        assert_eq!(resumed.status, CommandStatus::Running);
        assert!(matches!(
            resumed.events.as_slice(),
            [CommandEvent::ObjectComBuild {
                object_id,
                target_id: event_target,
                stop_first: true,
            }] if *object_id == builder_id && *event_target == target_id
        ));
    }

    #[test]
    fn build_structure_only_builds_internal_target() {
        let builder_id = ObjectId::new(10);
        let target_id = ObjectId::new(20);
        for category in [CATEGORY_STRUCTURE, CATEGORY_STATIC_BACK] {
            let mut builder =
                command_object!(builder_id.as_u64(); position = Vector2::new(125, 100));
            builder.physical.can_construct = 1;
            builder.category = category;
            builder.action_procedure = ActionProcedure::Walk;
            let target = command_object!(target_id.as_u64(); position = Vector2::new(100, 100);
                shape = DefinitionRect::new(120, 90, 20, 20));
            let objects = command_objects([builder.clone(), target]);
            let ctx = command_ctx(&builder, &objects, 0);
            let mut state =
                BuildState::from_request(&request!(Build, with_target: Some(target_id)))
                    .expect("build state");
            let result = state.step(&ctx);
            assert_eq!(result.status, CommandStatus::Failed, "category={category}");
            assert!(result.events.is_empty());
            assert!(result.operations.is_empty());
        }

        let mut builder = snapshot_with_id(builder_id.as_u64());
        builder.physical.can_construct = 1;
        builder.category = CATEGORY_STRUCTURE;
        let target = command_object!(target_id.as_u64(); container = Some(builder_id));
        let objects = command_objects([builder.clone(), target]);
        let ctx = command_ctx(&builder, &objects, 0);
        let mut state = BuildState::from_request(&request!(Build, with_target: Some(target_id)))
            .expect("build state");
        let result = state.step(&ctx);
        assert!(matches!(
            result.events.as_slice(),
            [CommandEvent::ObjectComBuild {
                object_id,
                target_id: event_target,
                stop_first: false,
            }] if *object_id == builder_id && *event_target == target_id
        ));
    }

    #[test]
    fn build_defers_energy_to_linekit_cobuilder() {
        let builder_id = ObjectId::new(10);
        let target_id = ObjectId::new(20);
        let other_id = ObjectId::new(30);
        let kit_id = ObjectId::new(40);
        let mut builder = snapshot_with_id(builder_id.as_u64());
        builder.physical.can_construct = 1;
        builder.commands = vec![command_view(CommandId::Build, Some(target_id))];
        let target = command_object!(target_id.as_u64(); construction = FULL_CON;
            line_connect = LINE_CONNECT_POWER_INPUT);
        let mut other = command_object!(other_id.as_u64();
            commands = vec![command_view(CommandId::Build, Some(target_id))]);
        other.contents.push(kit_id);
        let kit = command_object!(kit_id.as_u64(); definition_id = LINEKIT_DEFINITION.into();
            container = Some(other_id));
        let mut objects = command_objects([builder.clone(), target, other, kit]);
        let ctx =
            command_context!(command_ctx(&builder, &objects, 0); structures_need_energy: true);
        let mut state = BuildState::from_request(&request!(Build, with_target: Some(target_id)))
            .expect("build state");

        let result = state.step(&ctx);
        assert_eq!(result.status, CommandStatus::Completed);
        assert!(result.operations.is_empty());

        let current_kit_id = ObjectId::new(41);
        builder.contents.push(current_kit_id);
        let current_kit = command_object!(current_kit_id.as_u64();
            definition_id = LINEKIT_DEFINITION.into(); container = Some(builder_id));
        objects.insert(builder_id, builder.clone());
        objects.insert(current_kit_id, current_kit);
        let ctx =
            command_context!(command_ctx(&builder, &objects, 1); structures_need_energy: true);
        let mut state = BuildState::from_request(&request!(Build, with_target: Some(target_id)))
            .expect("build state");

        let result = state.step(&ctx);
        let energy = pushed_request(&result.operations, CommandId::Energy);
        assert_eq!(result.status, CommandStatus::Completed);
        assert_eq!(energy.target, Some(target_id));
    }

    #[test]
    fn energy_acquire_child_failure_propagates_to_parent() {
        // Energy's missing-linekit AddCommand uses the default SilentSub
        // mode (C4Command.cpp:2268-2272).
        let builder_id = ObjectId::new(10);
        let target_id = ObjectId::new(20);
        let supply_id = ObjectId::new(30);

        let builder = command_object!(builder_id.as_u64(); owner = 1);
        let target = command_object!(target_id.as_u64(); position = Vector2::new(100, 0);
            line_connect = LINE_CONNECT_POWER_INPUT);
        let mut supply = command_object!(supply_id.as_u64();
            line_connect = crate::LINE_CONNECT_POWER_OUTPUT);
        supply.ocf |= ocf::POWER_SUPPLY;

        let objects = command_objects([target, supply]);
        let ctx =
            command_context!(command_ctx(&builder, &objects, 0); structures_need_energy: true);
        let parent_request = request!(Energy, with_target: Some(target_id));
        let mut state = EnergyState::from_request(&parent_request).expect("energy state");

        let result = state.step(&ctx);
        let acquire = pushed_request(&result.operations, CommandId::Acquire);
        assert_silent_child_failure_propagates(parent_request, acquire, &ctx);
    }

    #[test]
    fn energy_rejects_non_input_before_disabled_rule_completion() {
        let builder = snapshot_with_id(10);
        let target_id = ObjectId::new(20);
        let target = snapshot_with_id(target_id.as_u64());
        let objects = command_objects([target]);
        let ctx = command_ctx(&builder, &objects, 0);
        let mut state = EnergyState::from_request(&request!(Energy, with_target: Some(target_id)))
            .expect("energy state");

        let result = state.step(&ctx);

        assert_eq!(result.status, CommandStatus::Failed);
        assert!(result.operations.is_empty());
        assert!(result.events.is_empty());
    }

    #[test]
    fn energy_completes_when_connected_target_does_not_need_energy() {
        let builder_id = ObjectId::new(10);
        let target_id = ObjectId::new(20);
        let supply_id = ObjectId::new(30);
        let linekit_id = ObjectId::new(40);
        let line_id = ObjectId::new(50);

        let mut builder = command_object!(builder_id.as_u64(); owner = 1);
        builder.contents.push(linekit_id);
        let target = command_object!(target_id.as_u64(); line_connect = LINE_CONNECT_POWER_INPUT;
            need_energy = false);
        let mut supply = snapshot_with_id(supply_id.as_u64());
        supply.ocf |= ocf::POWER_SUPPLY;
        supply.line_connect = crate::LINE_CONNECT_POWER_OUTPUT;
        let linekit = command_object!(linekit_id.as_u64(); definition_id = LINEKIT_DEFINITION.into();
            container = Some(builder_id));
        let line = command_object!(line_id.as_u64(); definition_id = POWERLINE_DEFINITION.into();
            action_name = CONNECT_ACTION.into(); action_target = Some(supply_id);
            action_target2 = Some(target_id));

        let mut objects = command_objects([target.clone(), supply, linekit, line]);
        let mut state = EnergyState::from_request(&request!(Energy, with_target: Some(target_id)))
            .expect("energy state");
        let result = {
            let mut ctx = command_ctx(&builder, &objects, 0);
            ctx.structures_need_energy = true;
            state.step(&ctx)
        };

        assert_eq!(result.status, CommandStatus::Completed);
        assert!(result.update.is_none());
        assert!(result.operations.is_empty());
        assert!(result.events.is_empty(), "the carried linekit is untouched");
        assert_eq!(state.linekit, None);

        objects
            .get_mut(&target_id)
            .expect("target present")
            .need_energy = true;
        let mut needs_energy_state =
            EnergyState::from_request(&request!(Energy, with_target: Some(target_id)))
                .expect("energy state");
        let continued = {
            let mut ctx = command_ctx(&builder, &objects, 1);
            ctx.structures_need_energy = true;
            needs_energy_state.step(&ctx)
        };
        assert_eq!(continued.status, CommandStatus::Running);
        assert!(continued.events.iter().any(|event| matches!(
            event,
            CommandEvent::CreateLine { from, to, .. }
                if *from == supply_id && *to == linekit_id
        )));
    }

    #[test]
    fn energy_does_not_skip_closest_non_output_supply() {
        let builder = snapshot_with_id(10);
        let target_id = ObjectId::new(20);
        let closest_id = ObjectId::new(30);
        let farther_id = ObjectId::new(40);
        let target = command_object!(target_id.as_u64(); line_connect = LINE_CONNECT_POWER_INPUT);
        let mut closest = command_object!(closest_id.as_u64(); position = Vector2::new(10, 0));
        closest.ocf |= ocf::POWER_SUPPLY;
        let mut farther = command_object!(farther_id.as_u64(); position = Vector2::new(20, 0));
        farther.ocf |= ocf::POWER_SUPPLY;
        farther.line_connect = crate::LINE_CONNECT_POWER_OUTPUT;
        let objects = command_objects([target, closest, farther]);
        let mut ctx = command_ctx(&builder, &objects, 0);
        ctx.structures_need_energy = true;
        let mut state = EnergyState::from_request(&request!(Energy, with_target: Some(target_id)))
            .expect("energy state");

        let result = state.step(&ctx);

        assert_eq!(result.status, CommandStatus::Failed);
        assert_eq!(state.source, Some(closest_id));
        assert!(result.operations.is_empty());
        assert!(result.events.is_empty());
    }

    #[test]
    fn energy_range_uses_cpp_integer_distance() {
        let builder = snapshot_with_id(10);
        let target_id = ObjectId::new(20);
        let supply_id = ObjectId::new(30);
        let target = command_object!(target_id.as_u64(); line_connect = LINE_CONNECT_POWER_INPUT);
        let supply = command_object!(supply_id.as_u64(); position = Vector2::new(650, 1);
            line_connect = crate::LINE_CONNECT_POWER_OUTPUT);
        let objects = command_objects([target, supply]);
        let mut ctx = command_ctx(&builder, &objects, 0);
        ctx.structures_need_energy = true;
        let mut state = EnergyState::from_request(
            &request!(Energy, with_target: Some(target_id), with_target2: Some(supply_id)),
        )
        .expect("energy state");

        let result = state.step(&ctx);

        assert_eq!(result.status, CommandStatus::Running);
        assert_eq!(
            pushed_request(&result.operations, CommandId::Acquire).data,
            CommandData::Integer(definition_id_to_c4id(LINEKIT_DEFINITION).expect("linekit C4ID"))
        );
    }

    #[test]
    fn energy_reuses_carried_line_and_retargets_to_far_endpoint() {
        let builder_id = ObjectId::new(10);
        let target_id = ObjectId::new(20);
        let selected_supply_id = ObjectId::new(30);
        let plain_kit_id = ObjectId::new(40);
        let attached_kit_id = ObjectId::new(41);
        let later_kit_id = ObjectId::new(42);
        let line_id = ObjectId::new(50);
        let later_same_kit_line_id = ObjectId::new(51);
        let earlier_other_kit_line_id = ObjectId::new(52);
        let far_endpoint_id = ObjectId::new(60);
        let ignored_far_endpoint_id = ObjectId::new(61);
        let other_kit_far_endpoint_id = ObjectId::new(62);

        let builder = command_object!(builder_id.as_u64(); owner = 1;
            command_direction = CommandDirection::Right;
            contents = vec![plain_kit_id, attached_kit_id, later_kit_id]);
        let target = command_object!(target_id.as_u64(); line_connect = LINE_CONNECT_POWER_INPUT);
        let selected_supply = command_object!(selected_supply_id.as_u64();
            position = Vector2::new(100, 0); line_connect = crate::LINE_CONNECT_POWER_OUTPUT);
        let plain_kit = command_object!(plain_kit_id.as_u64();
            definition_id = LINEKIT_DEFINITION.into(); container = Some(builder_id));
        let attached_kit = command_object!(attached_kit_id.as_u64();
            definition_id = LINEKIT_DEFINITION.into(); container = Some(builder_id));
        let later_kit = command_object!(later_kit_id.as_u64();
            definition_id = LINEKIT_DEFINITION.into(); container = Some(builder_id));
        let line = command_object!(line_id.as_u64(); master_list_order = 2;
            definition_id = POWERLINE_DEFINITION.into(); action_name = CONNECT_ACTION.into();
            action_target = Some(far_endpoint_id); action_target2 = Some(attached_kit_id));
        let mut later_same_kit_line = line.clone();
        later_same_kit_line.id = later_same_kit_line_id;
        later_same_kit_line.master_list_order = 3;
        later_same_kit_line.action_target = Some(attached_kit_id);
        later_same_kit_line.action_target2 = Some(ignored_far_endpoint_id);
        let mut earlier_other_kit_line = line.clone();
        earlier_other_kit_line.id = earlier_other_kit_line_id;
        earlier_other_kit_line.master_list_order = 1;
        earlier_other_kit_line.action_target = Some(later_kit_id);
        earlier_other_kit_line.action_target2 = Some(other_kit_far_endpoint_id);
        let far_endpoint = command_object!(far_endpoint_id.as_u64(); position = Vector2::new(100, 0);
            shape = DefinitionRect::new(92, -10, 16, 20); line_connect = crate::LINE_CONNECT_POWER_OUTPUT);
        let ignored_far_endpoint = snapshot_with_id(ignored_far_endpoint_id.as_u64());
        let other_kit_far_endpoint = snapshot_with_id(other_kit_far_endpoint_id.as_u64());
        let mut objects = command_objects([
            target,
            selected_supply,
            plain_kit,
            attached_kit,
            later_kit,
            line,
            later_same_kit_line,
            earlier_other_kit_line,
            far_endpoint,
            ignored_far_endpoint,
            other_kit_far_endpoint,
        ]);
        let mut ctx = command_ctx(&builder, &objects, 0);
        ctx.structures_need_energy = true;
        let mut state = EnergyState::from_request(
            &request!(Energy, with_target: Some(target_id), with_target2: Some(selected_supply_id)),
        )
        .expect("energy state");

        let result = state.step(&ctx);

        assert_eq!(result.status, CommandStatus::Completed);
        assert_eq!(state.source, Some(far_endpoint_id));
        assert_eq!(state.linekit, Some(attached_kit_id));
        assert_eq!(state.line, Some(line_id));
        let line_update = result
            .events
            .iter()
            .find_map(|event| match event {
                CommandEvent::ApplyObjectUpdate { object_id, update } if *object_id == line_id => {
                    Some(update)
                }
                _ => None,
            })
            .expect("existing line is retargeted");
        let action = line_update.action.as_ref().expect("Connect action update");
        assert_eq!(action.name.as_deref(), Some(CONNECT_ACTION));
        assert_eq!(action.target, Some(Some(far_endpoint_id)));
        assert_eq!(action.target2, Some(Some(target_id)));
        let removed: Vec<_> = result
            .events
            .iter()
            .filter_map(|event| match event {
                CommandEvent::ApplyObjectUpdate { object_id, update }
                    if update.status == Some(ObjectStatus::Deleted) =>
                {
                    Some(*object_id)
                }
                _ => None,
            })
            .collect();
        assert_eq!(removed, vec![attached_kit_id]);

        let mut moving_builder = builder.clone();
        moving_builder.position = Vector2::new(100, 0);
        let mut moving_state = EnergyState::from_request(
            &request!(Energy, with_target: Some(target_id), with_target2: Some(selected_supply_id)),
        )
        .expect("energy state");
        let moving = {
            let mut ctx = command_ctx(&moving_builder, &objects, 1);
            ctx.structures_need_energy = true;
            moving_state.step(&ctx)
        };
        assert_eq!(moving.status, CommandStatus::Running);
        assert_eq!(
            pushed_request(&moving.operations, CommandId::MoveTo).target,
            Some(target_id)
        );
        assert_eq!(moving_state.line, Some(line_id));

        for stale_line in [line_id, later_same_kit_line_id, earlier_other_kit_line_id] {
            objects
                .get_mut(&stale_line)
                .expect("line present")
                .action_name = "Idle".into();
        }
        let resumed = {
            let mut ctx = command_ctx(&builder, &objects, 2);
            ctx.structures_need_energy = true;
            moving_state.step(&ctx)
        };
        assert_eq!(resumed.status, CommandStatus::Running);
        assert_eq!(moving_state.line, None);
        assert!(
            resumed.events.is_empty(),
            "the disconnected kit is retained"
        );
        assert_eq!(
            pushed_request(&resumed.operations, CommandId::MoveTo).target,
            Some(far_endpoint_id),
            "Energy returns to the live source instead of using the stale line"
        );

        let malformed_line_id = ObjectId::new(53);
        let malformed_line = command_object!(malformed_line_id.as_u64(); master_list_order = 0;
            definition_id = POWERLINE_DEFINITION.into(); action_name = CONNECT_ACTION.into();
            action_target = Some(plain_kit_id); action_target2 = None);
        objects.insert(malformed_line_id, malformed_line);
        let mut malformed_state = EnergyState::from_request(
            &request!(Energy, with_target: Some(target_id), with_target2: Some(selected_supply_id)),
        )
        .expect("energy state");
        let malformed = {
            let mut ctx = command_ctx(&builder, &objects, 3);
            ctx.structures_need_energy = true;
            malformed_state.step(&ctx)
        };
        assert_eq!(malformed.status, CommandStatus::Completed);
        assert_eq!(malformed_state.source, None);
        assert_eq!(malformed_state.line, Some(malformed_line_id));
        let malformed_action = malformed
            .events
            .iter()
            .find_map(|event| match event {
                CommandEvent::ApplyObjectUpdate { object_id, update }
                    if *object_id == malformed_line_id =>
                {
                    update.action.as_ref()
                }
                _ => None,
            })
            .expect("malformed line is still selected");
        assert_eq!(malformed_action.target, None);
        assert_eq!(malformed_action.target2, Some(Some(target_id)));
        assert!(malformed.events.iter().any(|event| matches!(
            event,
            CommandEvent::ApplyObjectUpdate { object_id, update }
                if *object_id == plain_kit_id
                    && update.status == Some(ObjectStatus::Deleted)
        )));
    }

    #[test]
    fn energy_starts_a_power_line_at_the_nearby_supply() {
        // C4Command::Energy keeps running after it has a line kit: at the
        // supply it creates PWRL from the supply to that kit
        // (C4Command.cpp:2259-2289).
        let builder_id = ObjectId::new(10);
        let target_id = ObjectId::new(20);
        let supply_id = ObjectId::new(30);
        let linekit_id = ObjectId::new(40);

        let mut builder = command_object!(builder_id.as_u64(); owner = 1);
        builder.contents.push(linekit_id);
        builder.command_direction = CommandDirection::Right;

        let target = command_object!(target_id.as_u64(); position = Vector2::new(100, 0);
            shape = DefinitionRect::new(92, -10, 16, 20); line_connect = LINE_CONNECT_POWER_INPUT);

        let mut supply = command_object!(supply_id.as_u64(); definition_id = "POWR".into();
            line_connect = crate::LINE_CONNECT_POWER_OUTPUT);
        supply.ocf |= ocf::POWER_SUPPLY;

        let linekit = command_object!(linekit_id.as_u64(); definition_id = LINEKIT_DEFINITION.into();
            container = Some(builder_id));

        let objects = command_objects([target, supply, linekit]);
        let ctx =
            command_context!(command_ctx(&builder, &objects, 0); structures_need_energy: true);
        let mut state = EnergyState::from_request(&request!(Energy, with_target: Some(target_id)))
            .expect("energy state");

        let result = state.step(&ctx);

        assert_eq!(result.status, CommandStatus::Running);
        assert!(result.update.is_none());
        assert!(result.events.iter().any(|event| matches!(
            event,
            CommandEvent::CreateLine {
                definition_id,
                owner: 1,
                from,
                to,
            } if definition_id == "PWRL"
                && *from == supply_id
                && *to == linekit_id
        )));

        let line_id = ObjectId::new(50);
        let line = command_object!(line_id.as_u64(); definition_id = POWERLINE_DEFINITION.into();
            owner = 1; action_name = CONNECT_ACTION.into(); action_target = Some(supply_id);
            action_target2 = Some(linekit_id));
        let mut connected_objects = objects.clone();
        connected_objects.insert(line_id, line);
        let mut at_target_builder = builder.clone();
        at_target_builder.position = Vector2::new(100, 0);
        let connected_ctx = command_context!(command_ctx(&at_target_builder, &connected_objects, 1); structures_need_energy: true);
        let connected = state.step(&connected_ctx);
        assert_eq!(connected.status, CommandStatus::Completed);
        assert_eq!(
            connected.update.and_then(|update| update.command_direction),
            Some(CommandDirection::Stop),
            "Energy stops only on the final connection"
        );
    }

    #[test]
    fn energy_source_scan_breaks_distance_ties_by_cpp_master_list_order() {
        let actor_id = ObjectId::new(1);
        let target_id = ObjectId::new(2);
        let lower_id_later = ObjectId::new(3);
        let higher_id_earlier = ObjectId::new(99);

        let actor = command_object!(actor_id.as_u64(); owner = 7);
        let target = command_object!(target_id.as_u64(); position = Vector2::ZERO;
            line_connect = LINE_CONNECT_POWER_INPUT);

        let supply = |id: ObjectId, x: i32, master_list_order: usize| {
            let mut snapshot = command_object!(id.as_u64(); master_list_order = master_list_order;
                position = Vector2::new(x, 0));
            snapshot.ocf |= ocf::POWER_SUPPLY;
            snapshot.line_connect = crate::LINE_CONNECT_POWER_OUTPUT;
            snapshot
        };
        let mut objects = command_objects([
            actor.clone(),
            target,
            supply(lower_id_later, 10, 2),
            supply(higher_id_earlier, -10, 1),
        ]);
        let choose = |objects: &CommandObjectSnapshots| {
            let ctx =
                command_context!(command_ctx(&actor, objects, 0); structures_need_energy: true);
            let mut state =
                EnergyState::from_request(&request!(Energy, with_target: Some(target_id)))
                    .expect("energy state");
            state.resolve_source(&ctx, target_id)
        };

        assert_eq!(choose(&objects), Some(higher_id_earlier));

        objects
            .get_mut(&lower_id_later)
            .expect("later supply present")
            .master_list_order = 1;
        objects
            .get_mut(&higher_id_earlier)
            .expect("earlier supply present")
            .master_list_order = 2;
        assert_eq!(choose(&objects), Some(lower_id_later));
    }

    #[test]
    fn energy_spawned_line_uses_cpp_master_list_order_and_both_endpoints() {
        let actor_id = ObjectId::new(1);
        let target_id = ObjectId::new(2);
        let source_id = ObjectId::new(3);
        let selected_kit_id = ObjectId::new(4);
        let other_kit_id = ObjectId::new(5);
        let newborn_line_id = ObjectId::new(6);
        let older_selected_line_id = ObjectId::new(90);
        let later_other_line_id = ObjectId::new(99);

        let actor = command_object!(actor_id.as_u64(); owner = 7);
        let target = snapshot_with_id(target_id.as_u64());
        let source = snapshot_with_id(source_id.as_u64());
        let selected_kit = snapshot_with_id(selected_kit_id.as_u64());
        let other_kit = snapshot_with_id(other_kit_id.as_u64());
        let line = |id: ObjectId, kit: ObjectId, master_list_order: usize| {
            command_object!(id.as_u64(); master_list_order = master_list_order;
                definition_id = POWERLINE_DEFINITION.into(); owner = actor.owner;
                action_target = Some(source_id); action_target2 = Some(kit))
        };

        let objects = command_objects([
            actor.clone(),
            target,
            source.clone(),
            selected_kit,
            other_kit,
            line(newborn_line_id, selected_kit_id, 9),
            line(older_selected_line_id, selected_kit_id, 4),
            line(later_other_line_id, other_kit_id, 10),
        ]);
        let ctx = command_context!(command_ctx(&actor, &objects, 0); structures_need_energy: true);
        let state = EnergyState::from_request(&request!(Energy, with_target: Some(target_id)))
            .expect("energy state");

        assert_eq!(
            state.spawned_line(&ctx, &source, selected_kit_id),
            Some(newborn_line_id)
        );
    }

    #[test]
    fn acquire_completes_when_inventory_contains_item() {
        let builder_id = ObjectId::new(1);
        let item_id = ObjectId::new(2);

        let mut builder = command_object!(builder_id.as_u64(); ocf = ocf::AVAILABLE | ocf::ALIVE;
            collectible = false);
        builder.contents.push(item_id);

        let item = command_object!(item_id.as_u64(); definition_id = "WOOD".into();
            ocf = ocf::AVAILABLE | ocf::FULL_CON; collectible = true; construction = FULL_CON;
            container = Some(builder_id));

        let objects = command_objects([builder, item]);

        let builder_snapshot = objects.get(&builder_id).expect("builder present");
        let ctx = command_ctx(builder_snapshot, &objects, 0);

        let mut state = AcquireState::from_request(
            &request!(Acquire, with_data: CommandData::Text("WOOD".into())),
        )
        .expect("state created");

        let result = state.step(&ctx);
        assert_eq!(result.status, CommandStatus::Completed);
        assert!(result.operations.is_empty());
    }

    #[test]
    fn acquire_requests_get_for_candidate() {
        let builder_id = ObjectId::new(10);
        let item_id = ObjectId::new(20);

        let builder = command_object!(builder_id.as_u64(); ocf = ocf::AVAILABLE | ocf::ALIVE;
            collectible = false);

        let mut item = command_object!(item_id.as_u64(); definition_id = "WOOD".into();
            position = Vector2::new(100, 0); ocf = ocf::AVAILABLE | ocf::FULL_CON; collectible = true;
            construction = FULL_CON);
        // Construction components are nonliving. C4Command::Acquire filters
        // by OCF_Available/full construction/fire, never OCF_Alive
        // (C4Command.cpp:2105-2132).
        item.alive = false;

        let objects = command_objects([builder, item]);

        let builder_snapshot = objects.get(&builder_id).expect("builder present");
        let ctx = command_ctx(builder_snapshot, &objects, 0);

        let mut state = AcquireState::from_request(
            &request!(Acquire, with_data: CommandData::Text("WOOD".into())),
        )
        .expect("state created");

        let script = state.step(&ctx);
        match script.events.first() {
            Some(CommandEvent::ControlCommandAcquire {
                caller,
                definition_id,
                ..
            }) => {
                assert_eq!(*caller, builder_id);
                assert_eq!(definition_id, "WOOD");
            }
            other => panic!("expected acquire control command, got {:?}", other),
        }
        assert!(script.operations.is_empty());

        state.script_result = Some(AcquireScriptResult::Continue);
        let result = state.step(&ctx);
        assert_eq!(result.status, CommandStatus::Running);
        assert_eq!(result.operations.len(), 1);
        match &result.operations[0] {
            CommandOperation::PushFront(request) => {
                assert_eq!(request.id, CommandId::Get);
                assert_eq!(request.target, Some(item_id));
                assert_eq!(request.mode, CommandMode::SilentSub);
            }
            other => panic!("expected get request, got {:?}", other),
        }

        // C4CMD_Acquire InitEvaluation (C4Command.cpp:1666-1670) only
        // replaces a ZERO Tx/Ty with 500/250 — a negative range stays
        // negative and the Inside(cx-px, -Tx, +Tx) test (:2115-2116)
        // then matches nothing: the nearby item is NOT found and the
        // command falls through to Buy (:2136).
        assert_eq!(
            (state.range_x, state.range_y),
            (500, 250),
            "zero ranges default (C4Command.cpp:1668-1669)"
        );
        let mut state = AcquireState::from_request(
            &request!(Acquire, with_data: CommandData::Text("WOOD".into()), with_tx: Some(-50), with_ty: Some(-50)),
        )
        .expect("state created");
        assert_eq!(
            (state.range_x, state.range_y),
            (-50, -50),
            "negative ranges keep their sign (C4Command.cpp:1668 replaces only 0)"
        );
        let _ = state.step(&ctx);
        state.script_result = Some(AcquireScriptResult::Continue);
        let result = state.step(&ctx);
        match &result.operations[0] {
            CommandOperation::PushFront(request) => {
                assert_eq!(
                    request.id,
                    CommandId::Buy,
                    "empty signed range finds no material -> Buy (C4Command.cpp:2136)"
                );
            }
            other => panic!("expected buy request, got {:?}", other),
        }
    }

    #[test]
    fn acquire_uses_squared_distance_and_cpp_master_list_order_for_ties() {
        let builder_id = ObjectId::new(1);
        let manhattan_favorite_id = ObjectId::new(2);
        let later_tie_id = ObjectId::new(3);
        let earlier_tie_id = ObjectId::new(99);

        let builder = command_object!(builder_id.as_u64(); collectible = false);

        let item = |id: ObjectId, position: Vector2, master_list_order: usize| {
            command_object!(id.as_u64(); definition_id = "WOOD".into();
                position = position; master_list_order = master_list_order;
                ocf = ocf::AVAILABLE | ocf::FULL_CON; collectible = true; construction = FULL_CON)
        };
        // Manhattan prefers (0,6): 6 < 7. C++ squared distance prefers the
        // 3-4-5 candidate: 25 < 36.
        let manhattan_favorite = item(manhattan_favorite_id, Vector2::new(0, 6), 0);
        let later_tie = item(later_tie_id, Vector2::new(4, 3), 2);
        let earlier_tie = item(earlier_tie_id, Vector2::new(-4, 3), 1);

        let mut objects = command_objects([later_tie, manhattan_favorite, builder.clone()]);
        let state = AcquireState::from_request(
            &request!(Acquire, with_data: CommandData::Text("WOOD".into())),
        )
        .expect("acquire state");
        let choose = |objects: &CommandObjectSnapshots| {
            let ctx = command_ctx(&builder, objects, 0);
            state.find_candidate(&ctx)
        };

        assert_eq!(choose(&objects), Some(later_tie_id));

        // Equal squared distances follow forward Game.Objects order. Swap
        // only those ranks and require the winner to swap too, independent
        // of HashMap iteration or object IDs.
        objects.insert(earlier_tie_id, earlier_tie);
        assert_eq!(choose(&objects), Some(earlier_tie_id));
        objects
            .get_mut(&later_tie_id)
            .expect("later tie present")
            .master_list_order = 1;
        objects
            .get_mut(&earlier_tie_id)
            .expect("earlier tie present")
            .master_list_order = 2;
        assert_eq!(choose(&objects), Some(later_tie_id));
    }

    #[test]
    fn acquire_skips_burning_and_source_or_drain_pipe_connected_candidates() {
        let builder_id = ObjectId::new(1);
        let burning_id = ObjectId::new(2);
        let source_connected_id = ObjectId::new(3);
        let drain_connected_id = ObjectId::new(4);
        let available_id = ObjectId::new(5);

        let builder = command_object!(builder_id.as_u64(); collectible = false);
        let item = |id: ObjectId, x: i32| {
            command_object!(id.as_u64(); definition_id = "WOOD".into();
                position = Vector2::new(x, 0); ocf = ocf::AVAILABLE | ocf::FULL_CON; collectible = true;
                construction = FULL_CON)
        };
        let mut burning = item(burning_id, 1);
        burning.on_fire = true;
        let source_connected = item(source_connected_id, 2);
        let drain_connected = item(drain_connected_id, 3);
        let available = item(available_id, 4);

        let source_pipe = command_object!(10; definition_id = SOURCE_PIPE_DEFINITION.into();
            action_name = CONNECT_ACTION.into(); action_target = Some(source_connected_id));
        let drain_pipe = command_object!(11; definition_id = DRAIN_PIPE_DEFINITION.into();
            action_name = CONNECT_ACTION.into(); action_target2 = Some(drain_connected_id));
        // Exact action and target matching matter: neither decoy may hide
        // the otherwise valid fallback candidate.
        let wrong_action = command_object!(12; definition_id = SOURCE_PIPE_DEFINITION.into();
            action_name = "Idle".into(); action_target = Some(available_id));
        let wrong_target = command_object!(13; definition_id = DRAIN_PIPE_DEFINITION.into();
            action_name = CONNECT_ACTION.into(); action_target = Some(builder_id));

        let mut objects = CommandObjectSnapshots::default();
        for snapshot in [
            burning,
            source_connected,
            drain_connected,
            available,
            source_pipe,
            drain_pipe,
            wrong_action,
            wrong_target,
        ] {
            objects.insert(snapshot.id, snapshot);
        }
        objects.insert(builder_id, builder.clone());
        let ctx = command_ctx(&builder, &objects, 0);
        let state = AcquireState::from_request(
            &request!(Acquire, with_data: CommandData::Text("WOOD".into())),
        )
        .expect("acquire state");

        assert_eq!(state.find_candidate(&ctx), Some(available_id));
    }

    #[test]
    fn acquire_rescans_after_get_returns_and_prefers_a_new_nearer_candidate() {
        let builder_id = ObjectId::new(1);
        let far_id = ObjectId::new(2);
        let near_id = ObjectId::new(3);
        let builder = command_object!(builder_id.as_u64(); collectible = false);
        let make_item = |id: ObjectId, x: i32| {
            command_object!(id.as_u64(); definition_id = "WOOD".into();
                position = Vector2::new(x, 0); ocf = ocf::AVAILABLE | ocf::FULL_CON; collectible = true;
                construction = FULL_CON)
        };

        let initial_objects = command_objects([builder.clone(), make_item(far_id, 20)]);
        let initial_ctx = command_ctx(&builder, &initial_objects, 0);
        let mut state = AcquireState::from_request(
            &request!(Acquire, with_data: CommandData::Text("WOOD".into()), with_update_interval: 50),
        )
        .expect("acquire state");
        let _ = state.step(&initial_ctx);
        state.script_result = Some(AcquireScriptResult::Continue);
        let first = state.step(&initial_ctx);
        assert!(matches!(
            first.operations.first(),
            Some(CommandOperation::PushFront(request))
                if request.id == CommandId::Get && request.target == Some(far_id)
        ));

        let mut later_objects = initial_objects.clone();
        later_objects.insert(near_id, make_item(near_id, 5));
        let later_ctx = command_ctx(&builder, &later_objects, 1);
        let script = state.step(&later_ctx);
        assert!(matches!(
            script.events.first(),
            Some(CommandEvent::ControlCommandAcquire { .. })
        ));
        state.script_result = Some(AcquireScriptResult::Continue);
        let rescanned = state.step(&later_ctx);
        assert!(matches!(
            rescanned.operations.first(),
            Some(CommandOperation::PushFront(request))
                if request.id == CommandId::Get && request.target == Some(near_id)
        ));
    }

    #[test]
    fn acquire_transfers_item_from_shared_container() {
        let builder_id = ObjectId::new(1);
        let container_id = ObjectId::new(2);
        let item_id = ObjectId::new(3);

        let builder = command_object!(builder_id.as_u64(); ocf = ocf::AVAILABLE | ocf::ALIVE;
            collectible = false; container = Some(container_id));

        let container = command_object!(container_id.as_u64(); position = Vector2::new(0, 0));

        let item = command_object!(item_id.as_u64(); definition_id = "WOOD".into();
            construction = FULL_CON; ocf = ocf::AVAILABLE | ocf::FULL_CON; collectible = true;
            container = Some(container_id));

        let objects = command_objects([builder, container, item]);

        let builder_snapshot = objects.get(&builder_id).expect("builder present");
        let ctx = command_ctx(builder_snapshot, &objects, 42);

        let mut state = AcquireState::from_request(
            &request!(Acquire, with_data: CommandData::Text("WOOD".into())),
        )
        .expect("state created");

        let script = state.step(&ctx);
        assert!(matches!(
            script.events.first(),
            Some(CommandEvent::ControlCommandAcquire { .. })
        ));

        state.script_result = Some(AcquireScriptResult::Continue);
        let result = state.step(&ctx);
        assert_eq!(result.status, CommandStatus::Running);
        assert_eq!(result.operations.len(), 1);
        match &result.operations[0] {
            CommandOperation::PushFront(request) => {
                assert_eq!(request.id, CommandId::Get);
                assert_eq!(request.target, Some(item_id));
                assert_eq!(request.mode, CommandMode::SilentSub);
            }
            other => panic!("expected get request, got {:?}", other),
        }
        assert!(
            result.events.is_empty(),
            "acquire should delegate transfer to Get command"
        );
    }

    #[test]
    fn acquire_enters_container_when_adjacent() {
        let builder_id = ObjectId::new(1);
        let container_id = ObjectId::new(2);
        let item_id = ObjectId::new(3);

        let builder = command_object!(builder_id.as_u64(); ocf = ocf::AVAILABLE | ocf::ALIVE;
            collectible = false; position = Vector2::new(0, 0));

        let container = command_object!(container_id.as_u64(); position = Vector2::new(4, 0);
            ocf = ocf::AVAILABLE | ocf::ENTRANCE);

        let item = command_object!(item_id.as_u64(); definition_id = "WOOD".into();
            construction = FULL_CON; ocf = ocf::AVAILABLE | ocf::FULL_CON; collectible = true;
            container = Some(container_id); position = container.position);

        let objects = command_objects([builder, container, item]);

        let builder_snapshot = objects.get(&builder_id).expect("builder present");
        let ctx = command_ctx(builder_snapshot, &objects, 100);

        let mut state = AcquireState::from_request(
            &request!(Acquire, with_data: CommandData::Text("WOOD".into())),
        )
        .expect("state created");

        let script = state.step(&ctx);
        dbg!(state.script_pending, script.events.len());
        state.script_result = Some(AcquireScriptResult::Continue);
        let result = state.step(&ctx);
        dbg!(state.script_pending, result.operations.len());
        assert_eq!(result.status, CommandStatus::Running);
        assert!(result.events.is_empty());
        assert_eq!(result.operations.len(), 1);
        match &result.operations[0] {
            CommandOperation::PushFront(request) => {
                assert_eq!(request.id, CommandId::Get);
                assert_eq!(request.target, Some(item_id));
            }
            other => panic!("expected get request, got {:?}", other),
        }
    }

    #[test]
    fn acquire_requests_buy_when_no_candidate() {
        let builder_id = ObjectId::new(10);

        let builder = command_object!(builder_id.as_u64(); ocf = ocf::AVAILABLE | ocf::ALIVE;
            collectible = false);

        let objects = command_objects([builder.clone()]);

        let ctx = command_ctx(&builder, &objects, 0);

        let mut state = AcquireState::from_request(
            &request!(Acquire, with_data: CommandData::Text("WOOD".into())),
        )
        .expect("state created");

        let _ = state.step(&ctx);
        state.script_result = Some(AcquireScriptResult::Continue);
        let result = state.step(&ctx);
        assert_eq!(result.status, CommandStatus::Running);
        assert_eq!(result.operations.len(), 1);
        match &result.operations[0] {
            CommandOperation::PushFront(request) => {
                assert_eq!(request.id, CommandId::Buy);
                assert_eq!(request.update_interval, 100);
                assert_eq!(request.mode, CommandMode::Sub);
            }
            other => panic!("expected buy request, got {:?}", other),
        }

        let later_ctx = command_ctx(&builder, &objects, 10);

        let _ = state.step(&later_ctx);
        state.script_result = Some(AcquireScriptResult::Continue);
        let second = state.step(&later_ctx);
        assert!(second.operations.is_empty());
    }

    #[test]
    fn acquire_retries_buy_after_cooldown() {
        let builder_id = ObjectId::new(11);

        let builder = command_object!(builder_id.as_u64(); ocf = ocf::AVAILABLE | ocf::ALIVE;
            collectible = false);

        let objects = command_objects([builder.clone()]);

        let mut state = AcquireState::from_request(
            &request!(Acquire, with_data: CommandData::Text("WOOD".into())),
        )
        .expect("state created");

        let initial_ctx = command_ctx(&builder, &objects, 0);

        let _ = state.step(&initial_ctx);
        state.script_result = Some(AcquireScriptResult::Continue);
        let initial = state.step(&initial_ctx);
        assert_eq!(initial.status, CommandStatus::Running);
        assert_eq!(initial.operations.len(), 1);
        match &initial.operations[0] {
            CommandOperation::PushFront(request) => assert_eq!(request.id, CommandId::Buy),
            other => panic!("expected initial buy request, got {:?}", other),
        }

        let mid_ctx = command_ctx(&builder, &objects, 60);

        let _ = state.step(&mid_ctx);
        state.script_result = Some(AcquireScriptResult::Continue);
        let mid = state.step(&mid_ctx);
        assert_eq!(mid.status, CommandStatus::Running);
        assert!(
            mid.operations.is_empty(),
            "buy request should not repeat before cooldown elapses"
        );

        let retry_ctx = command_ctx(&builder, &objects, 150);

        let _ = state.step(&retry_ctx);
        state.script_result = Some(AcquireScriptResult::Continue);
        let retry = state.step(&retry_ctx);
        assert_eq!(retry.status, CommandStatus::Running);
        let buy_requests: Vec<_> = retry
            .operations
            .iter()
            .filter_map(|operation| match operation {
                CommandOperation::PushFront(request) if request.id == CommandId::Buy => Some(()),
                _ => None,
            })
            .collect();
        assert_eq!(
            buy_requests.len(),
            1,
            "expected a single buy retry after cooldown elapsed"
        );
    }

    #[test]
    fn acquire_requests_get_when_in_other_container() {
        let builder_id = ObjectId::new(1);
        let current_container_id = ObjectId::new(2);
        let target_container_id = ObjectId::new(3);
        let item_id = ObjectId::new(4);

        let mut builder = command_object!(builder_id.as_u64(); ocf = ocf::AVAILABLE | ocf::ALIVE;
            collectible = false; container = Some(current_container_id));

        let current_container = command_object!(current_container_id.as_u64();
            position = Vector2::new(5, 5); ocf = ocf::AVAILABLE | ocf::ENTRANCE);

        builder.position = current_container.position;

        let target_container = command_object!(target_container_id.as_u64();
            position = Vector2::new(20, 0); ocf = ocf::AVAILABLE | ocf::ENTRANCE);

        let item = command_object!(item_id.as_u64(); definition_id = "WOOD".into();
            construction = FULL_CON; ocf = ocf::AVAILABLE | ocf::FULL_CON; collectible = true;
            container = Some(target_container_id); position = target_container.position);

        let objects = command_objects([builder, current_container, target_container, item]);

        let builder_snapshot = objects.get(&builder_id).expect("builder present");
        let players: HashMap<i32, CommandPlayerSnapshot> = HashMap::new();
        let definitions: HashMap<DefinitionId, CommandDefinitionSnapshot> = HashMap::new();
        let ctx = command_ctx(builder_snapshot, &objects, 0);

        let mut stack = CommandStack::new();
        stack
            .push_back(
                request!(Acquire, with_data: CommandData::Text("WOOD".into()), with_mode: CommandMode::Base),
            )
            .expect("command queued");

        let evaluation = stack.step(&ctx).expect("Acquire evaluates");
        assert_eq!(evaluation.status, CommandStatus::Running);
        assert!(evaluation.events.is_empty());
        assert!(evaluation.operations.is_empty());
        let script = stack.step(&ctx).expect("script stage");
        assert!(matches!(
            script.events.first(),
            Some(CommandEvent::ControlCommandAcquire { .. })
        ));

        assert!(stack.set_acquire_script_result(AcquireScriptResult::Continue));
        let mut frame = ctx.frame + 1;
        let initial_len = stack.len();
        loop {
            let step_ctx = command_context!(command_ctx_at_frame(ctx.object, ctx.objects, ctx.players, ctx.definitions, frame); position: ctx.position,
            structures_need_energy: ctx.structures_need_energy,
            base_buy_enabled: ctx.base_buy_enabled,
            base_sell_enabled: ctx.base_sell_enabled,
            transfer_zones: ctx.transfer_zones);
            let step_result = stack.step(&step_ctx).expect("acquire evaluation");
            assert_eq!(step_result.status, CommandStatus::Running);
            if stack.len() > initial_len {
                break;
            }
            frame += 1;
            assert!(
                frame < 1000,
                "test timeout - no new command after {} frames",
                frame
            );
        }

        // Verify that a Get command was pushed to the stack
        assert_eq!(
            stack.len(),
            2,
            "expected Get command to be pushed onto stack"
        );
    }

    #[test]
    fn acquire_leaves_container_for_loose_item() {
        let builder_id = ObjectId::new(1);
        let current_container_id = ObjectId::new(2);
        let item_id = ObjectId::new(3);

        let mut builder = command_object!(builder_id.as_u64(); ocf = ocf::AVAILABLE | ocf::ALIVE;
            collectible = false; container = Some(current_container_id));

        let current_container = command_object!(current_container_id.as_u64();
            position = Vector2::new(5, 5); ocf = ocf::AVAILABLE | ocf::ENTRANCE);

        builder.position = current_container.position;

        let item = command_object!(item_id.as_u64(); definition_id = "WOOD".into();
            construction = FULL_CON; ocf = ocf::AVAILABLE | ocf::FULL_CON; collectible = true;
            position = Vector2::new(30, 0));

        let objects = command_objects([builder, current_container, item]);

        let builder_snapshot = objects.get(&builder_id).expect("builder present");
        let players: HashMap<i32, CommandPlayerSnapshot> = HashMap::new();
        let definitions: HashMap<DefinitionId, CommandDefinitionSnapshot> = HashMap::new();
        let ctx = command_ctx(builder_snapshot, &objects, 0);

        let mut stack = CommandStack::new();
        stack
            .push_back(request!(Acquire, with_data: CommandData::Text("WOOD".into())))
            .expect("command queued");

        let evaluation = stack.step(&ctx).expect("Acquire evaluates");
        assert_eq!(evaluation.status, CommandStatus::Running);
        assert!(evaluation.events.is_empty());
        assert!(evaluation.operations.is_empty());
        let script = stack.step(&ctx).expect("script stage");
        assert!(matches!(
            script.events.first(),
            Some(CommandEvent::ControlCommandAcquire { .. })
        ));

        assert!(stack.set_acquire_script_result(AcquireScriptResult::Continue));
        let mut frame = ctx.frame + 1;
        let initial_len = stack.len();
        loop {
            let step_ctx = command_context!(command_ctx_at_frame(ctx.object, ctx.objects, ctx.players, ctx.definitions, frame); position: ctx.position,
            structures_need_energy: ctx.structures_need_energy,
            base_buy_enabled: ctx.base_buy_enabled,
            base_sell_enabled: ctx.base_sell_enabled,
            transfer_zones: ctx.transfer_zones);
            let step_result = stack.step(&step_ctx).expect("acquire evaluation");
            assert_eq!(step_result.status, CommandStatus::Running);
            if stack.len() > initial_len {
                break;
            }
            frame += 1;
            assert!(
                frame < 1000,
                "test timeout - no new command after {} frames",
                frame
            );
        }

        // Verify that a Get command was pushed to the stack
        assert_eq!(
            stack.len(),
            2,
            "expected Get command to be pushed onto stack"
        );
    }

    #[test]
    fn acquire_attaches_to_grabbable_container() {
        let builder_id = ObjectId::new(1);
        let container_id = ObjectId::new(2);
        let item_id = ObjectId::new(3);

        let builder = command_object!(builder_id.as_u64(); ocf = ocf::AVAILABLE | ocf::ALIVE;
            collectible = false; position = Vector2::new(0, 0));

        let container = command_object!(container_id.as_u64(); position = Vector2::new(6, 0);
            ocf = ocf::AVAILABLE | ocf::GRAB);

        let item = command_object!(item_id.as_u64(); definition_id = "WOOD".into();
            construction = FULL_CON; ocf = ocf::AVAILABLE | ocf::FULL_CON; collectible = true;
            container = Some(container_id); position = container.position);

        let objects = command_objects([builder.clone(), container.clone(), item]);

        let players: HashMap<i32, CommandPlayerSnapshot> = HashMap::new();
        let definitions: HashMap<DefinitionId, CommandDefinitionSnapshot> = HashMap::new();
        let ctx = command_ctx(&builder, &objects, 0);

        let mut stack = CommandStack::new();
        stack
            .push_back(request!(Acquire, with_data: CommandData::Text("WOOD".into())))
            .expect("command queued");

        let evaluation = stack.step(&ctx).expect("Acquire evaluates");
        assert_eq!(evaluation.status, CommandStatus::Running);
        assert!(evaluation.events.is_empty());
        assert!(evaluation.operations.is_empty());
        let script = stack.step(&ctx).expect("script stage");
        assert!(matches!(
            script.events.first(),
            Some(CommandEvent::ControlCommandAcquire { .. })
        ));

        assert!(stack.set_acquire_script_result(AcquireScriptResult::Continue));
        let initial_len = stack.len();
        let mut frame = ctx.frame + 1;
        loop {
            let step_ctx = command_context!(command_ctx_at_frame(ctx.object, ctx.objects, ctx.players, ctx.definitions, frame); position: ctx.position,
            structures_need_energy: ctx.structures_need_energy,
            base_buy_enabled: ctx.base_buy_enabled,
            base_sell_enabled: ctx.base_sell_enabled,
            transfer_zones: ctx.transfer_zones);
            let step_result = stack.step(&step_ctx).expect("acquire evaluation");
            assert_eq!(step_result.status, CommandStatus::Running);
            // `CommandStack::step` applies the command's operations to the stack
            // internally (it drains `result.operations`), so detect the requested
            // Get by the new front entry rather than by `result.operations` (which
            // is always empty here). The bounded frame guard keeps a never-pushed
            // regression a failure rather than an infinite hang.
            if stack.len() > initial_len {
                break;
            }
            frame += 1;
            assert!(
                frame < 1000,
                "test timeout - no Get command after {frame} frames"
            );
        }

        // Acquire should request a Get for the WOOD held inside the grabbable
        // container; verify the pushed sub-command targets the contained item.
        let snapshot = stack.snapshot();
        assert_eq!(snapshot.commands.len(), 2);
        match &snapshot.commands[0].state {
            CommandState::Get(state) => {
                assert_eq!(state.target, Some(item_id));
            }
            other => panic!("expected get command at front, got {other:?}"),
        }
        match &snapshot.commands[1].state {
            CommandState::Acquire(_) => {}
            other => panic!("expected acquire command beneath get, got {other:?}"),
        }
    }

    #[test]
    fn acquire_script_handled_skips_default_logic() {
        let builder_id = ObjectId::new(5);
        let builder = command_object!(builder_id.as_u64(); ocf = ocf::AVAILABLE | ocf::ALIVE;
            collectible = false);

        let objects = command_objects([builder.clone()]);

        let ctx = command_ctx(&builder, &objects, 0);

        let mut state = AcquireState::from_request(
            &request!(Acquire, with_data: CommandData::Text("WOOD".into())),
        )
        .expect("state created");

        let script_step = state.step(&ctx);
        assert!(matches!(
            script_step.events.first(),
            Some(CommandEvent::ControlCommandAcquire { .. })
        ));

        state.script_result = Some(AcquireScriptResult::Handled);
        let second = state.step(&ctx);
        assert_eq!(second.status, CommandStatus::Running);
        assert!(second.operations.is_empty());
    }

    #[test]
    fn acquire_script_complete_finishes_command() {
        let builder_id = ObjectId::new(6);
        let builder = command_object!(builder_id.as_u64(); ocf = ocf::AVAILABLE | ocf::ALIVE;
            collectible = false);

        let objects = command_objects([builder.clone()]);

        let ctx = command_ctx(&builder, &objects, 0);

        let mut state = AcquireState::from_request(
            &request!(Acquire, with_data: CommandData::Text("WOOD".into())),
        )
        .expect("state created");

        let _ = state.step(&ctx);
        state.script_result = Some(AcquireScriptResult::Complete);
        let second = state.step(&ctx);
        assert_eq!(second.status, CommandStatus::Completed);
    }

    #[test]
    fn acquire_script_failed_marks_command_failed() {
        let builder_id = ObjectId::new(7);
        let builder = command_object!(builder_id.as_u64(); ocf = ocf::AVAILABLE | ocf::ALIVE;
            collectible = false);

        let objects = command_objects([builder.clone()]);

        let ctx = command_ctx(&builder, &objects, 0);

        let mut state = AcquireState::from_request(
            &request!(Acquire, with_data: CommandData::Text("WOOD".into())),
        )
        .expect("state created");

        let _ = state.step(&ctx);
        state.script_result = Some(AcquireScriptResult::Failed);
        let second = state.step(&ctx);
        assert_eq!(second.status, CommandStatus::Failed);
    }
