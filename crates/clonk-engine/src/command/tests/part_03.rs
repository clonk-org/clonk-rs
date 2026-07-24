// Contiguous slice 3 of 7 of the `command/tests` battery, spliced by
// `include!` from the parent module so every test id is unchanged.

    #[test]
    fn detached_throw_failure_updates_its_detached_outer_base() {
        let actor_id = ObjectId::new(622);
        let mut walking = snapshot_with_id(actor_id.as_u64());
        walking.action_procedure = ActionProcedure::Walk;
        let objects = HashMap::from([(actor_id, walking)]);
        let players = HashMap::new();
        let definitions = HashMap::new();
        let ctx = move_to_ctx_at_frame(
            objects.get(&actor_id).expect("actor present"),
            &objects,
            &players,
            &definitions,
            1,
        );
        let mut stack = CommandStack::new();
        let request = || {
            CommandRequest::new(CommandId::Throw)
                .with_tx(Some(100))
                .with_ty(Some(20))
        };
        stack
            .push_front(request().with_mode(CommandMode::Base))
            .expect("outer Throw queues");
        let outer_instance_id = stack.entries.front().unwrap().instance_id;
        let CommandState::Throw(outer) = &mut stack.entries.front_mut().unwrap().state else {
            panic!("outer command should be Throw");
        };
        outer
            .continuations
            .push(ThrowContinuation::AfterObjectComStop);
        stack
            .push_front(request().with_mode(CommandMode::SilentSub))
            .expect("inner Throw queues");
        let inner_instance_id = stack.entries.front().unwrap().instance_id;
        let CommandState::Throw(inner) = &mut stack.entries.front_mut().unwrap().state else {
            panic!("inner command should be Throw");
        };
        inner
            .continuations
            .push(ThrowContinuation::AfterObjectComStop);

        stack.clear();
        let resumed = stack
            .execute_pending_throw_prelude(
                &ctx,
                crate::PhysicsSettings::default().gravity_as_c4fixed(),
                inner_instance_id,
            )
            .expect("detached inner Throw resumes");
        assert_eq!(resumed.status, CommandStatus::Failed);
        assert!(
            resumed
                .events
                .iter()
                .all(|event| !matches!(event, CommandEvent::FailureFeedback { .. })),
            "SilentSub failure with a retained base suppresses direct feedback"
        );
        let outer = stack
            .detached_throw_preludes
            .iter()
            .find(|candidate| candidate.entry.instance_id == outer_instance_id)
            .expect("outer iExec Throw remains retained");
        assert_eq!(outer.entry.failures, 1);
    }

    #[test]
    fn detached_throw_failure_uses_nonprelude_and_next_unfinished_bases() {
        let actor_id = ObjectId::new(623);
        let mut walking = snapshot_with_id(actor_id.as_u64());
        walking.action_procedure = ActionProcedure::Walk;
        let objects = HashMap::from([(actor_id, walking)]);
        let players = HashMap::new();
        let definitions = HashMap::new();
        let ctx = move_to_ctx_at_frame(
            objects.get(&actor_id).expect("actor present"),
            &objects,
            &players,
            &definitions,
            1,
        );
        let inner_request = || {
            CommandRequest::new(CommandId::Throw)
                .with_tx(Some(100))
                .with_ty(Some(20))
                .with_mode(CommandMode::SilentSub)
        };

        let mut cleared = CommandStack::new();
        cleared
            .push_front(CommandRequest::new(CommandId::Drop))
            .expect("outer Drop queues");
        let CommandState::Drop(outer) = &mut cleared.entries.front_mut().unwrap().state else {
            panic!("outer command should be Drop");
        };
        outer.completion_pending = true;
        cleared
            .push_front(inner_request())
            .expect("inner Throw queues");
        let inner_id = cleared.entries.front().unwrap().instance_id;
        let CommandState::Throw(inner) = &mut cleared.entries.front_mut().unwrap().state else {
            panic!("inner command should be Throw");
        };
        inner
            .continuations
            .push(ThrowContinuation::AfterObjectComStop);
        cleared.clear();
        let result = cleared
            .execute_pending_throw_prelude(
                &ctx,
                crate::PhysicsSettings::default().gravity_as_c4fixed(),
                inner_id,
            )
            .expect("inner Throw resumes");
        assert_eq!(result.status, CommandStatus::Failed);
        assert!(result
            .events
            .iter()
            .all(|event| !matches!(event, CommandEvent::FailureFeedback { .. })));

        let mut skipped = CommandStack::new();
        skipped
            .push_front(CommandRequest::new(CommandId::Wait))
            .expect("base2 queues");
        let base2_id = skipped.entries.front().unwrap().instance_id;
        skipped
            .push_front(CommandRequest::new(CommandId::Wait))
            .expect("base1 queues");
        let base1_id = skipped.entries.front().unwrap().instance_id;
        skipped
            .push_front(inner_request())
            .expect("inner Throw queues");
        let inner_id = skipped.entries.front().unwrap().instance_id;
        let CommandState::Throw(inner) = &mut skipped.entries.front_mut().unwrap().state else {
            panic!("inner command should be Throw");
        };
        inner
            .continuations
            .push(ThrowContinuation::AfterObjectComStop);
        skipped.clear_front();
        skipped.finish_entry_public(0, true);
        let result = skipped
            .execute_pending_throw_prelude(
                &ctx,
                crate::PhysicsSettings::default().gravity_as_c4fixed(),
                inner_id,
            )
            .expect("detached inner Throw resumes");
        assert_eq!(result.status, CommandStatus::Failed);
        assert!(result
            .events
            .iter()
            .all(|event| !matches!(event, CommandEvent::FailureFeedback { .. })));
        let base1 = skipped
            .entries
            .iter()
            .find(|entry| entry.instance_id == base1_id)
            .expect("base1 remains linked");
        let base2 = skipped
            .entries
            .iter()
            .find(|entry| entry.instance_id == base2_id)
            .expect("base2 remains linked");
        assert_eq!(base1.failures, 0);
        assert_eq!(base2.failures, 1);
    }

    #[test]
    fn detached_base_modes_do_not_increment_an_unrelated_base() {
        for (mode, expects_feedback) in
            [(CommandMode::Base, true), (CommandMode::SilentBase, false)]
        {
            let mut stack = CommandStack::new();
            stack
                .push_front(CommandRequest::new(CommandId::Wait))
                .expect("base queues");
            let base_chain = stack
                .entries
                .iter()
                .map(DetachedCommandBase::from)
                .collect::<Vec<_>>();
            let failed =
                ActiveCommand::from_request(CommandRequest::new(CommandId::Throw).with_mode(mode))
                    .expect("failed command constructs");
            assert_eq!(
                stack
                    .record_detached_failure(&failed, &base_chain)
                    .is_some(),
                expects_feedback
            );
            assert_eq!(stack.entries.front().unwrap().failures, 0);
        }
    }

    #[test]
    fn detached_drop_keeps_raw_removed_target_and_queues_get() {
        let actor_id = ObjectId::new(620);
        let item_id = ObjectId::new(621);
        let mut digging = snapshot_with_id(actor_id.as_u64());
        digging.action_procedure = ActionProcedure::Dig;
        digging.contents = vec![item_id];
        let mut item = snapshot_with_id(item_id.as_u64());
        item.container = Some(actor_id);
        let initial_objects = HashMap::from([(actor_id, digging.clone()), (item_id, item.clone())]);
        let players = HashMap::new();
        let definitions = HashMap::new();
        let initial_ctx = move_to_ctx_at_frame(
            initial_objects.get(&actor_id).expect("actor present"),
            &initial_objects,
            &players,
            &definitions,
            1,
        );
        let mut stack = CommandStack::new();
        stack
            .push_front(CommandRequest::new(CommandId::Drop).with_target(Some(item_id)))
            .expect("Drop queues");
        let command_instance_id = stack.entries.front().unwrap().instance_id;
        let initial = stack.execute_front(&initial_ctx).expect("Drop starts");
        assert!(matches!(
            initial.events.as_slice(),
            [CommandEvent::ObjectComStopDrop { .. }]
        ));

        stack.clear();
        assert!(
            !stack.clear_object_reference(item_id),
            "ClearPointers does not walk the unlinked iExec command"
        );
        let mut walking = digging;
        walking.action_procedure = ActionProcedure::Walk;
        walking.contents.clear();
        item.destroyed = true;
        let resumed_objects = HashMap::from([(actor_id, walking), (item_id, item)]);
        let resumed_ctx = move_to_ctx_at_frame(
            resumed_objects.get(&actor_id).expect("actor present"),
            &resumed_objects,
            &players,
            &definitions,
            1,
        );
        let resumed = stack
            .execute_pending_drop_prelude(&resumed_ctx, command_instance_id)
            .expect("detached Drop resumes");
        assert_eq!(resumed.status, CommandStatus::Running);
        assert_eq!(stack.command_names(), ["Get"]);
        assert_eq!(stack.command_views()[0].target, Some(item_id));
    }

    #[test]
    fn targeted_drop_stops_comdir_but_keeps_coordinates_out_of_the_exit_event() {
        // Tx/Ty are only the C4Command::Drop move-to goal. Once in range,
        // C++ writes COMD_Stop and ObjectComDrop computes the exit from the
        // actor/item shapes (C4Command.cpp:1015-1033).
        let actor_id = ObjectId::new(632);
        let item_id = ObjectId::new(633);
        let mut actor = snapshot_with_id(actor_id.as_u64());
        actor.position = Vector2::new(10, 10);
        actor.command_direction = CommandDirection::Right;
        actor.contents = vec![item_id];
        let mut item = snapshot_with_id(item_id.as_u64());
        item.container = Some(actor_id);
        let objects = HashMap::from([(actor_id, actor), (item_id, item)]);
        let players = HashMap::new();
        let definitions = HashMap::new();
        let ctx = move_to_ctx_at_frame(
            objects.get(&actor_id).expect("actor present"),
            &objects,
            &players,
            &definitions,
            0,
        );
        let mut state = DropState::from_request(
            &CommandRequest::new(CommandId::Drop)
                .with_tx(Some(11))
                .with_ty(Some(10)),
        );

        let result = state.step(&ctx);
        assert_eq!(result.status, CommandStatus::Running);
        assert_eq!(
            result.update.and_then(|update| update.command_direction),
            Some(CommandDirection::Stop)
        );
        assert_eq!(
            result.events,
            vec![CommandEvent::ObjectComDrop {
                actor_id,
                object_id: item_id,
                command_instance_id: 0,
            }]
        );
    }

    #[test]
    fn targeted_drop_ungrabs_before_contained_or_push_put_branches() {
        let actor_id = ObjectId::new(634);
        let item_id = ObjectId::new(635);
        let container_id = ObjectId::new(636);
        let pushed_id = ObjectId::new(637);
        let mut actor = snapshot_with_id(actor_id.as_u64());
        actor.position = Vector2::new(10, 10);
        actor.container = Some(container_id);
        actor.action_procedure = ActionProcedure::Push;
        actor.action_target = Some(pushed_id);
        actor.contents = vec![item_id];
        let mut item = snapshot_with_id(item_id.as_u64());
        item.container = Some(actor_id);
        let mut objects = HashMap::from([
            (actor_id, actor),
            (item_id, item),
            (container_id, snapshot_with_id(container_id.as_u64())),
            (pushed_id, snapshot_with_id(pushed_id.as_u64())),
        ]);
        let players = HashMap::new();
        let definitions = HashMap::new();
        let request = CommandRequest::new(CommandId::Drop)
            .with_tx(Some(11))
            .with_ty(Some(10));
        let mut state = DropState::from_request(&request);
        let ctx = move_to_ctx_at_frame(
            objects.get(&actor_id).expect("actor present"),
            &objects,
            &players,
            &definitions,
            0,
        );

        let result = state.step(&ctx);
        assert_eq!(result.status, CommandStatus::Running);
        assert!(result.events.is_empty());
        assert!(matches!(
            result.operations.as_slice(),
            [CommandOperation::PushFront(CommandRequest {
                id: CommandId::UnGrab,
                ..
            })]
        ));

        // Once UnGrab returns, the still-contained actor takes the targeted
        // drop branch rather than the later contained Put branch.
        objects
            .get_mut(&actor_id)
            .expect("actor present")
            .action_procedure = ActionProcedure::Walk;
        let ctx = move_to_ctx_at_frame(
            objects.get(&actor_id).expect("actor present"),
            &objects,
            &players,
            &definitions,
            1,
        );
        let result = state.step(&ctx);
        assert_eq!(result.status, CommandStatus::Running);
        assert_eq!(
            result.events,
            vec![CommandEvent::ObjectComDrop {
                actor_id,
                object_id: item_id,
                command_instance_id: 0,
            }]
        );
    }

    #[test]
    fn targeted_drop_uses_cpp_five_pixel_default_range() {
        let actor_id = ObjectId::new(638);
        let item_id = ObjectId::new(639);
        let mut actor = snapshot_with_id(actor_id.as_u64());
        actor.position = Vector2::new(10, 10);
        actor.contents = vec![item_id];
        let mut item = snapshot_with_id(item_id.as_u64());
        item.container = Some(actor_id);
        let objects = HashMap::from([(actor_id, actor), (item_id, item)]);
        let players = HashMap::new();
        let definitions = HashMap::new();
        let ctx = move_to_ctx_at_frame(
            objects.get(&actor_id).expect("actor present"),
            &objects,
            &players,
            &definitions,
            0,
        );
        let mut state = DropState::from_request(
            &CommandRequest::new(CommandId::Drop)
                .with_tx(Some(16))
                .with_ty(Some(10)),
        );

        let result = state.step(&ctx);
        assert_eq!(result.status, CommandStatus::Running);
        assert!(result.events.is_empty());
        assert!(matches!(
            result.operations.as_slice(),
            [CommandOperation::PushFront(CommandRequest {
                id: CommandId::MoveTo,
                ..
            })]
        ));
    }

    #[test]
    fn drop_gets_an_explicit_outside_item_before_dropping_it() {
        let actor_id = ObjectId::new(644);
        let item_id = ObjectId::new(645);
        let actor = snapshot_with_id(actor_id.as_u64());
        let mut item = snapshot_with_id(item_id.as_u64());
        item.position = Vector2::new(20, 10);
        let objects = HashMap::from([(actor_id, actor), (item_id, item)]);
        let players = HashMap::new();
        let definitions = HashMap::new();
        let ctx = move_to_ctx_at_frame(
            objects.get(&actor_id).expect("actor present"),
            &objects,
            &players,
            &definitions,
            0,
        );
        let mut state = DropState::from_request(
            &CommandRequest::new(CommandId::Drop).with_target(Some(item_id)),
        );

        let result = state.step(&ctx);
        assert_eq!(result.status, CommandStatus::Running);
        assert!(result.update.is_none());
        assert!(result.events.is_empty());
        assert!(matches!(
            result.operations.as_slice(),
            [CommandOperation::PushFront(request)]
                if request.id == CommandId::Get
                    && request.target == Some(item_id)
                    && request.update_interval == 40
                    && request.mode == CommandMode::SilentSub
        ));
    }

    #[test]
    fn drop_requests_move_to_coordinates() {
        let actor_id = ObjectId::new(640);
        let item_id = ObjectId::new(641);

        let mut actor = snapshot_with_id(actor_id.as_u64());
        actor.position = Vector2::new(0, 0);
        actor.contents = vec![item_id];

        let mut item = snapshot_with_id(item_id.as_u64());
        item.container = Some(actor_id);

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

        let mut state = DropState::from_request(
            &CommandRequest::new(CommandId::Drop)
                .with_tx(Some(120))
                .with_ty(Some(0)),
        );

        let result = state.step(&ctx);
        assert_eq!(result.status, CommandStatus::Running);
        assert!(
            result.operations.iter().any(|operation| match operation {
                CommandOperation::PushFront(request) => {
                    request.id == CommandId::MoveTo
                        && request.tx == Some(120)
                        && request.ty == Some(0)
                }
                _ => false,
            }),
            "drop should request movement towards target coordinates"
        );
        let first_move = pushed_request(&result.operations, CommandId::MoveTo);
        let reissued = state.step(&ctx);
        assert_eq!(
            pushed_request(&reissued.operations, CommandId::MoveTo),
            first_move,
            "Drop reissues MoveTo on its next execution"
        );
    }

    #[test]
    fn drop_runs_inline_put_take_when_actor_contained() {
        let actor_id = ObjectId::new(650);
        let item_id = ObjectId::new(651);
        let container_id = ObjectId::new(652);

        let mut actor = snapshot_with_id(actor_id.as_u64());
        actor.container = Some(container_id);
        actor.contents = vec![item_id];

        let mut item = snapshot_with_id(item_id.as_u64());
        item.container = Some(actor_id);

        let mut container = snapshot_with_id(container_id.as_u64());
        container.position = Vector2::new(0, 0);

        let mut objects = HashMap::new();
        objects.insert(actor.id, actor.clone());
        objects.insert(item.id, item);
        objects.insert(container.id, container);

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

        // Explicit 0/0 is C++'s untargeted sentinel and must still reach the
        // later contained Put branch.
        let mut stack = CommandStack::new();
        stack
            .push_front(
                CommandRequest::new(CommandId::Drop)
                    .with_tx(Some(0))
                    .with_ty(Some(0)),
            )
            .expect("Drop queues");
        let command_instance_id = stack.entries.front().expect("Drop remains").instance_id;
        let result = stack.execute_front(&ctx).expect("Drop executes");
        assert_eq!(result.status, CommandStatus::Running);
        assert!(result.operations.is_empty());
        assert_eq!(
            result.events,
            vec![CommandEvent::ObjectComPutTake {
                actor_id,
                target_id: container_id,
                requested_item: None,
                command: CommandId::Drop,
                command_instance_id,
            }]
        );
    }

    #[test]
    fn drop_runs_inline_put_take_when_pushing_target() {
        let actor_id = ObjectId::new(660);
        let item_id = ObjectId::new(661);
        let pushed_id = ObjectId::new(662);

        let mut actor = snapshot_with_id(actor_id.as_u64());
        actor.action_procedure = ActionProcedure::Push;
        actor.action_target = Some(pushed_id);
        actor.contents = vec![item_id];

        let mut item = snapshot_with_id(item_id.as_u64());
        item.container = Some(actor_id);

        let mut pushed = snapshot_with_id(pushed_id.as_u64());
        pushed.position = Vector2::new(0, 0);

        let mut objects = HashMap::new();
        objects.insert(actor.id, actor.clone());
        objects.insert(item.id, item);
        objects.insert(pushed.id, pushed);

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

        let mut state = DropState::from_request(&CommandRequest::new(CommandId::Drop));
        let result = state.step(&ctx);
        assert_eq!(result.status, CommandStatus::Running);
        assert!(result.operations.is_empty());
        assert_eq!(
            result.events,
            vec![CommandEvent::ObjectComPutTake {
                actor_id,
                target_id: pushed_id,
                requested_item: None,
                command: CommandId::Drop,
                command_instance_id: 0,
            }]
        );
    }

    fn step_dig_once(actor: CommandObjectSnapshot, request: CommandRequest) -> CommandStepResult {
        let actor_id = actor.id;
        let mut objects = HashMap::new();
        objects.insert(actor_id, actor);
        let players: HashMap<i32, CommandPlayerSnapshot> = HashMap::new();
        let definitions: HashMap<DefinitionId, CommandDefinitionSnapshot> = HashMap::new();
        let ctx = move_to_ctx_at_frame(
            objects.get(&actor_id).expect("actor present"),
            &objects,
            &players,
            &definitions,
            0,
        );
        let mut state = DigState::from_request(&request).expect("dig state created");
        state.step(&ctx)
    }

    #[test]
    fn dig_requests_ungrab_when_pushing() {
        let actor_id = ObjectId::new(60);
        let mut actor = snapshot_with_id(actor_id.as_u64());
        actor.action_procedure = ActionProcedure::Push;
        actor.command_direction = CommandDirection::Right;

        let mut objects = HashMap::new();
        objects.insert(actor.id, actor.clone());

        let players: HashMap<i32, CommandPlayerSnapshot> = HashMap::new();
        let definitions: HashMap<DefinitionId, CommandDefinitionSnapshot> = HashMap::new();

        let request = CommandRequest::new(CommandId::Dig)
            .with_tx(Some(15))
            .with_ty(Some(25));
        let mut state = DigState::from_request(&request).expect("state created");

        let ctx = CommandRuntimeContext {
            landscape: None,
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

        let result = state.step(&ctx);
        assert_eq!(result.status, CommandStatus::Running);
        assert!(
            result.operations.iter().any(|operation| match operation {
                CommandOperation::PushFront(request) => request.id == CommandId::UnGrab,
                _ => false,
            }),
            "dig should request ungrab when pushing"
        );
    }

    #[test]
    fn dig_requests_exit_when_contained() {
        let actor_id = ObjectId::new(61);
        let mut actor = snapshot_with_id(actor_id.as_u64());
        actor.container = Some(ObjectId::new(99));

        let mut objects = HashMap::new();
        objects.insert(actor.id, actor.clone());

        let players: HashMap<i32, CommandPlayerSnapshot> = HashMap::new();
        let definitions: HashMap<DefinitionId, CommandDefinitionSnapshot> = HashMap::new();

        let request = CommandRequest::new(CommandId::Dig)
            .with_tx(Some(0))
            .with_ty(Some(0));
        let mut state = DigState::from_request(&request).expect("state created");

        let ctx = CommandRuntimeContext {
            landscape: None,
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

        let result = state.step(&ctx);
        assert_eq!(result.status, CommandStatus::Running);
        assert!(
            result.operations.iter().any(|operation| match operation {
                CommandOperation::PushFront(request) => request.id == CommandId::Exit,
                _ => false,
            }),
            "dig should request exit when contained"
        );
    }

    #[test]
    fn dig_requests_live_object_com_dig_when_walking() {
        let actor_id = ObjectId::new(62);
        let mut actor = snapshot_with_id(actor_id.as_u64());
        actor.action_procedure = ActionProcedure::Walk;
        actor.command_direction = CommandDirection::Stop;
        actor.physical.can_dig = 1;

        let mut objects = HashMap::new();
        objects.insert(actor.id, actor.clone());

        let players: HashMap<i32, CommandPlayerSnapshot> = HashMap::new();
        let definitions: HashMap<DefinitionId, CommandDefinitionSnapshot> = HashMap::new();

        let request = CommandRequest::new(CommandId::Dig)
            .with_tx(Some(actor.position.x))
            .with_ty(Some(actor.position.y + 20))
            .with_data(CommandData::Integer(1));
        let mut state = DigState::from_request(&request).expect("state created");

        let ctx = CommandRuntimeContext {
            landscape: None,
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

        let result = state.step(&ctx);
        assert_eq!(result.status, CommandStatus::Running);
        assert_eq!(
            result.events,
            vec![CommandEvent::ObjectComDig {
                actor_id,
                dig_out_material: true,
                direction: Some(CommandDirection::DownLeft),
                command_instance_id: 0,
            }],
            "the live helper retains the C++ tx-DigRange steering quirk"
        );
        assert!(result.update.is_none());
    }

    #[test]
    fn dig_completes_when_within_move_range() {
        let actor_id = ObjectId::new(63);
        let mut actor = snapshot_with_id(actor_id.as_u64());
        actor.action_procedure = ActionProcedure::Dig;
        actor.command_direction = CommandDirection::Left;

        let mut objects = HashMap::new();
        objects.insert(actor.id, actor.clone());

        let players: HashMap<i32, CommandPlayerSnapshot> = HashMap::new();
        let definitions: HashMap<DefinitionId, CommandDefinitionSnapshot> = HashMap::new();

        let request = CommandRequest::new(CommandId::Dig)
            .with_tx(Some(actor.position.x))
            .with_ty(Some(actor.position.y));
        let mut state = DigState::from_request(&request).expect("state created");

        let ctx = CommandRuntimeContext {
            landscape: None,
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

        let result = state.step(&ctx);
        assert_eq!(result.status, CommandStatus::Completed);
        let update = result.update.expect("dig should stop when done");
        assert_eq!(update.command_direction, Some(CommandDirection::Stop));
        let action_update = update.action.expect("dig should reset to idle");
        assert_eq!(action_update.name.as_deref(), Some("Idle"));
    }

    #[test]
    fn dig_reached_check_uses_bottom_center_target() {
        let mut actor = snapshot_with_id(64);
        actor.position = Vector2::new(100, 93);
        actor.shape_top = -10;
        actor.action_procedure = ActionProcedure::Dig;

        let request = CommandRequest::new(CommandId::Dig)
            .with_tx(Some(100))
            .with_ty(Some(100));
        let result = step_dig_once(actor.clone(), request.clone());
        assert_eq!(
            result.status,
            CommandStatus::Completed,
            "raw y=100 is adjusted to the actor bottom-center y=93"
        );

        actor.position.y = 100;
        let result = step_dig_once(actor, request);
        assert_eq!(
            result.status,
            CommandStatus::Running,
            "the raw target itself is seven pixels outside the default range"
        );
    }

    #[test]
    fn dig_steering_keeps_cpp_downleft_quirk_and_emits_no_plain_up() {
        let mut actor = snapshot_with_id(65);
        actor.position = Vector2::new(100, 100);
        actor.shape_top = -10;
        actor.action_procedure = ActionProcedure::Dig;

        let below = step_dig_once(
            actor.clone(),
            CommandRequest::new(CommandId::Dig)
                .with_tx(Some(100))
                .with_ty(Some(127)),
        );
        assert_eq!(
            below.update.and_then(|update| update.command_direction),
            Some(CommandDirection::DownLeft),
            "cx == tx below the target is overwritten from Down to DownLeft"
        );

        actor.command_direction = CommandDirection::Right;
        for (target_x, expected) in [
            (90, Some(CommandDirection::UpLeft)),
            (99, None),
            (100, None),
            (101, None),
            (110, Some(CommandDirection::UpRight)),
        ] {
            let result = step_dig_once(
                actor.clone(),
                CommandRequest::new(CommandId::Dig)
                    .with_tx(Some(target_x))
                    .with_ty(Some(73)),
            );
            let emitted = result.update.and_then(|update| update.command_direction);
            assert_eq!(emitted, expected, "raw target x={target_x}");
            assert_ne!(emitted, Some(CommandDirection::Up));
        }
    }

    #[test]
    fn dig_routes_can_dig_failure_through_the_live_helper() {
        let mut actor = snapshot_with_id(66);
        actor.action_procedure = ActionProcedure::Walk;
        actor.physical.can_dig = 0;

        let result = step_dig_once(
            actor,
            CommandRequest::new(CommandId::Dig)
                .with_tx(Some(0))
                .with_ty(Some(20)),
        );
        assert_eq!(result.status, CommandStatus::Running);
        assert!(result.update.is_none(), "the Dig action never starts");
        assert!(result.operations.is_empty());
        assert!(matches!(
            result.events.as_slice(),
            [CommandEvent::ObjectComDig { actor_id, .. }] if *actor_id == ObjectId::new(66)
        ));
    }

    #[test]
    fn dig_uses_positive_definition_move_to_range() {
        let mut actor = snapshot_with_id(67);
        actor.position = Vector2::new(100, 100);
        actor.shape_top = -10;
        actor.action_procedure = ActionProcedure::Dig;

        let request = CommandRequest::new(CommandId::Dig)
            .with_tx(Some(115))
            .with_ty(Some(107));
        assert_eq!(
            step_dig_once(actor.clone(), request.clone()).status,
            CommandStatus::Running,
            "fifteen pixels exceeds the default move-to range"
        );

        actor.move_to_range = 20;
        assert_eq!(
            step_dig_once(actor, request).status,
            CommandStatus::Completed,
            "positive DefCore MoveToRange overrides the default five"
        );
    }

    #[test]
    fn dig_scale_and_hangle_use_object_com_let_go() {
        for procedure in [ActionProcedure::Scale, ActionProcedure::Hang] {
            for (direction, xdirf) in [(Direction::Left, 1), (Direction::Right, -1)] {
                let mut actor = snapshot_with_id(68);
                actor.action_procedure = procedure;
                actor.direction = direction;
                let result = step_dig_once(
                    actor,
                    CommandRequest::new(CommandId::Dig)
                        .with_tx(Some(100))
                        .with_ty(Some(100)),
                );
                assert_eq!(result.status, CommandStatus::Running);
                let update = result.update.expect("let-go produces an object update");
                assert_eq!(
                    update
                        .action
                        .as_ref()
                        .and_then(|action| action.name.as_deref()),
                    Some("Jump")
                );
                assert_eq!(
                    update.fixed_velocity,
                    Some(FixedVec2::new(math::itofix(xdirf), crate::C4Fixed::ZERO))
                );
            }
        }
    }

    #[test]
    fn dig_out_material_reasserts_data_while_already_digging() {
        let mut actor = snapshot_with_id(69);
        actor.action_procedure = ActionProcedure::Dig;
        actor.command_direction = CommandDirection::DownLeft;

        let result = step_dig_once(
            actor,
            CommandRequest::new(CommandId::Dig)
                .with_tx(Some(0))
                .with_ty(Some(20))
                .with_data(CommandData::Integer(1)),
        );
        assert_eq!(result.status, CommandStatus::Running);
        let update = result.update.expect("dig-out data is refreshed");
        assert_eq!(update.command_direction, None);
        let action = update.action.expect("data-only action update exists");
        assert_eq!(action.name, None, "the Dig action is not restarted");
        assert_eq!(action.data, Some(1));
    }

    #[test]
    fn retry_command_waits_then_completes() {
        let actor = snapshot_with_id(60);
        let actor_id = actor.id;

        let mut objects = HashMap::new();
        objects.insert(actor_id, actor.clone());

        let players: HashMap<i32, CommandPlayerSnapshot> = HashMap::new();
        let definitions: HashMap<DefinitionId, CommandDefinitionSnapshot> = HashMap::new();

        let mut stack = CommandStack::new();
        stack
            .push_back(CommandRequest::new(CommandId::Retry).with_update_interval(3))
            .expect("retry command accepted");

        for frame in 0..2 {
            let ctx = CommandRuntimeContext {
                landscape: None,
                frame,
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
            let result = stack.step(&ctx).expect("running result");
            assert_eq!(result.status, CommandStatus::Running);
            assert!(result.update.is_none());
        }

        let ctx = CommandRuntimeContext {
            landscape: None,
            frame: 2,
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
        let result = stack.step(&ctx).expect("completion result");
        assert_eq!(result.status, CommandStatus::Completed);
        assert!(result.update.is_none());
    }

    #[test]
    fn enter_enters_target_when_in_range() {
        let actor_id = ObjectId::new(30);
        let target_id = ObjectId::new(40);

        let mut actor = snapshot_with_id(actor_id.as_u64());
        actor.position = Vector2::new(10, 10);
        actor.command_direction = CommandDirection::Right;

        let mut target = snapshot_with_id(target_id.as_u64());
        target.position = Vector2::new(18, 16);
        // C4Command::Enter checks Target->At(cx, cy) — the actor point in
        // the target's absolute shape (C4Command.cpp:586-588).
        target.shape = DefinitionRect::new(target.position.x - 10, target.position.y - 10, 20, 20);
        target.entrance = Some(target.shape);
        target.ocf = ocf::ENTRANCE | ocf::AVAILABLE;
        target.entrance_status = true;
        target.category = CATEGORY_STRUCTURE;

        let mut objects = HashMap::new();
        objects.insert(actor.id, actor.clone());
        objects.insert(target.id, target);

        let players: HashMap<i32, CommandPlayerSnapshot> = HashMap::new();
        let definitions: HashMap<DefinitionId, CommandDefinitionSnapshot> = HashMap::new();
        let ctx = CommandRuntimeContext {
            landscape: None,
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

        let mut state = EnterState::from_request(
            &CommandRequest::new(CommandId::Enter).with_target(Some(target_id)),
        )
        .expect("state created");

        let result = state.step(&ctx);
        assert_eq!(result.status, CommandStatus::Completed);
        let update = result.update.expect("enter should produce an update");
        assert_eq!(update.command_direction, Some(CommandDirection::Stop));
        assert!(
            update.container.is_none(),
            "C4Object::Enter is an ordered engine event, not a plain delta"
        );
        assert_eq!(result.events.len(), 1);
        assert_eq!(
            result.events[0],
            CommandEvent::EnterObject {
                object_id: actor_id,
                container_id: target_id,
            }
        );
        assert!(result.operations.is_empty());
    }

    #[test]
    fn enter_accepts_inactive_targets_but_not_contained_target_geometry() {
        let actor_id = ObjectId::new(36);
        let target_id = ObjectId::new(46);
        let actor = snapshot_with_id(actor_id.as_u64());
        let mut target = snapshot_with_id(target_id.as_u64());
        target.status = ObjectStatus::Inactive;
        target.shape = DefinitionRect::new(-10, -10, 20, 20);
        target.entrance = Some(DefinitionRect::new(-3, -3, 6, 6));
        target.ocf = ocf::ENTRANCE;
        target.entrance_status = true;
        let mut objects = HashMap::from([(actor_id, actor), (target_id, target)]);
        let players = HashMap::new();
        let definitions = HashMap::new();
        let request = CommandRequest::new(CommandId::Enter).with_target(Some(target_id));

        let actor = objects.get(&actor_id).expect("actor present");
        let ctx = move_to_ctx_at_frame(actor, &objects, &players, &definitions, 0);
        let mut state = EnterState::from_request(&request).expect("Enter state");
        let inactive = state.step(&ctx);
        assert_eq!(inactive.status, CommandStatus::Completed);
        assert!(matches!(
            inactive.events.as_slice(),
            [CommandEvent::EnterObject { container_id, .. }] if *container_id == target_id
        ));

        objects
            .get_mut(&target_id)
            .expect("target present")
            .container = Some(ObjectId::new(99));
        let actor = objects.get(&actor_id).expect("actor present");
        let ctx = move_to_ctx_at_frame(actor, &objects, &players, &definitions, 1);
        let mut state = EnterState::from_request(&request).expect("Enter state");
        let contained = state.step(&ctx);
        assert_eq!(contained.status, CommandStatus::Running);
        assert!(contained.events.is_empty());
        assert!(matches!(
            contained.operations.as_slice(),
            [CommandOperation::PushFront(request)]
                if request.id == CommandId::MoveTo
                    && request.target.is_none()
                    && request.tx == Some(0)
                    && request.ty == Some(0)
        ));
    }

    #[test]
    fn enter_nil_and_cleared_targets_fail_on_execution() {
        let actor_id = ObjectId::new(37);
        let target_id = ObjectId::new(47);
        let actor = snapshot_with_id(actor_id.as_u64());
        let target = snapshot_with_id(target_id.as_u64());
        let objects = HashMap::from([(actor_id, actor), (target_id, target)]);
        let players = HashMap::new();
        let definitions = HashMap::new();
        let actor = objects.get(&actor_id).expect("actor present");
        let ctx = move_to_ctx_at_frame(actor, &objects, &players, &definitions, 0);

        let mut nil_target = CommandStack::new();
        nil_target
            .push_front(CommandRequest::new(CommandId::Enter))
            .expect("C++ queues Enter before its handler rejects nil Target");
        assert_eq!(
            nil_target.execute_front(&ctx).map(|result| result.status),
            Some(CommandStatus::Failed)
        );

        let mut cleared_target = CommandStack::new();
        cleared_target
            .push_front(CommandRequest::new(CommandId::Enter).with_target(Some(target_id)))
            .expect("Enter queues");
        assert!(cleared_target.clear_object_reference(target_id));
        assert_eq!(
            cleared_target
                .execute_front(&ctx)
                .map(|result| result.status),
            Some(CommandStatus::Failed)
        );

        let mut retained_objects = objects.clone();
        let retained_target = retained_objects
            .get_mut(&target_id)
            .expect("target tombstone remains addressable");
        retained_target.status = ObjectStatus::Deleted;
        retained_target.destroyed = true;
        let retained_actor = retained_objects.get(&actor_id).expect("actor present");
        let retained_ctx =
            move_to_ctx_at_frame(retained_actor, &retained_objects, &players, &definitions, 0);
        let mut retained = EnterState::from_request(
            &CommandRequest::new(CommandId::Enter).with_target(Some(target_id)),
        )
        .expect("retained Enter state");
        let result = retained.step(&retained_ctx);
        assert_eq!(result.status, CommandStatus::Running);
        assert!(
            result.operations.is_empty(),
            "GetEntranceArea rejects Status==0, so Enter stays pending without MoveTo"
        );
    }

    #[test]
    fn command_references_accept_inactive_status_and_contents_like_cpp() {
        let actor_id = ObjectId::new(70);
        let target_id = ObjectId::new(71);
        let source_id = ObjectId::new(72);
        let container_id = ObjectId::new(73);
        let deleted_conkit_id = ObjectId::new(74);
        let conkit_id = ObjectId::new(75);
        let deleted_content_id = ObjectId::new(76);
        let content_id = ObjectId::new(77);
        let deleted_linekit_id = ObjectId::new(78);
        let linekit_id = ObjectId::new(79);

        let mut actor = snapshot_with_id(actor_id.as_u64());
        actor.owner = 0;
        actor.controller = 0;
        actor.command_direction = CommandDirection::Right;
        actor.action_procedure = ActionProcedure::Walk;
        actor.physical.can_chop = 1;
        actor.physical.can_construct = 1;
        actor.contents = vec![deleted_conkit_id, conkit_id, deleted_linekit_id, linekit_id];

        let mut target = snapshot_with_id(target_id.as_u64());
        target.status = ObjectStatus::Inactive;
        target.alive = false;
        target.ocf = 0;
        target.command_direction = CommandDirection::Left;
        target.need_energy = true;
        target.line_connect = LINE_CONNECT_POWER_INPUT;

        let mut source = snapshot_with_id(source_id.as_u64());
        source.status = ObjectStatus::Inactive;
        source.alive = false;
        source.ocf = ocf::POWER_SUPPLY;
        source.line_connect = crate::LINE_CONNECT_POWER_OUTPUT;
        source.shape = DefinitionRect::new(-10, -10, 20, 20);

        let mut container = snapshot_with_id(container_id.as_u64());
        container.status = ObjectStatus::Inactive;
        container.alive = false;
        container.ocf = ocf::ENTRANCE;
        container.entrance_status = false;
        container.contents = vec![deleted_content_id, content_id];

        let mut deleted_conkit = snapshot_with_id(deleted_conkit_id.as_u64());
        deleted_conkit.definition_id = CONKIT_DEFINITION.into();
        deleted_conkit.status = ObjectStatus::Deleted;
        deleted_conkit.destroyed = true;
        deleted_conkit.container = Some(actor_id);

        let mut conkit = snapshot_with_id(conkit_id.as_u64());
        conkit.definition_id = CONKIT_DEFINITION.into();
        conkit.status = ObjectStatus::Inactive;
        conkit.alive = false;
        conkit.container = Some(actor_id);

        let mut deleted_linekit = snapshot_with_id(deleted_linekit_id.as_u64());
        deleted_linekit.definition_id = LINEKIT_DEFINITION.into();
        deleted_linekit.status = ObjectStatus::Deleted;
        deleted_linekit.destroyed = true;
        deleted_linekit.container = Some(actor_id);

        let mut linekit = snapshot_with_id(linekit_id.as_u64());
        linekit.definition_id = LINEKIT_DEFINITION.into();
        linekit.status = ObjectStatus::Inactive;
        linekit.alive = false;
        linekit.container = Some(actor_id);

        let mut deleted_content = snapshot_with_id(deleted_content_id.as_u64());
        deleted_content.definition_id = "ROCK".into();
        deleted_content.status = ObjectStatus::Deleted;
        deleted_content.destroyed = true;
        deleted_content.container = Some(container_id);

        let mut content = snapshot_with_id(content_id.as_u64());
        content.definition_id = "ROCK".into();
        content.status = ObjectStatus::Inactive;
        content.alive = false;
        content.collectible = true;
        content.container = Some(container_id);

        let objects = HashMap::from([
            (actor_id, actor.clone()),
            (target_id, target),
            (source_id, source),
            (container_id, container),
            (deleted_conkit_id, deleted_conkit),
            (conkit_id, conkit),
            (deleted_linekit_id, deleted_linekit),
            (linekit_id, linekit),
            (deleted_content_id, deleted_content),
            (content_id, content),
        ]);
        let players = HashMap::new();
        let definitions = HashMap::new();
        let mut transfer_zones = TransferZoneTable::default();
        transfer_zones.set(
            target_id,
            TransferZoneRect {
                x: -5,
                y: -5,
                width: 10,
                height: 10,
            },
        );
        let actor = objects.get(&actor_id).expect("actor present");
        let ctx = CommandRuntimeContext {
            landscape: None,
            frame: 0,
            position: actor.position,
            object: actor,
            objects: &objects,
            players: &players,
            definitions: &definitions,
            structures_need_energy: true,
            base_buy_enabled: true,
            base_sell_enabled: true,
            transfer_zones: &transfer_zones,
            rng: None,
        };

        let mut follow = FollowState::from_request(
            &CommandRequest::new(CommandId::Follow).with_target(Some(target_id)),
        )
        .expect("Follow state");
        let follow_result = follow.step(&ctx);
        assert_eq!(follow_result.status, CommandStatus::Running);
        assert_eq!(
            follow_result
                .update
                .and_then(|update| update.command_direction),
            Some(CommandDirection::Left),
            "inactive Follow target remains a retained pointer"
        );

        let mut chop = ChopState::from_request(
            &CommandRequest::new(CommandId::Chop).with_target(Some(target_id)),
        )
        .expect("Chop state");
        assert_eq!(
            chop.step(&ctx).status,
            CommandStatus::Completed,
            "an inactive non-choppable target takes C++'s assume-chopped success"
        );

        let mut push_to = PushToState::from_request(
            &CommandRequest::new(CommandId::PushTo)
                .with_target(Some(target_id))
                .with_tx(Some(0))
                .with_ty(Some(0))
                .with_evaluated(true),
        )
        .expect("PushTo state");
        assert_eq!(push_to.step(&ctx).status, CommandStatus::Completed);

        let mut transfer = TransferState::from_request(
            &CommandRequest::new(CommandId::Transfer).with_target(Some(target_id)),
        )
        .expect("Transfer state");
        let transfer_result = transfer.step(&ctx);
        assert_eq!(transfer_result.status, CommandStatus::Running);
        assert!(matches!(
            transfer_result.events.as_slice(),
            [CommandEvent::ControlTransfer { object_id, .. }] if *object_id == target_id
        ));

        let mut context = ContextState::from_request(
            &CommandRequest::new(CommandId::Context).with_target2(Some(target_id)),
        )
        .expect("Context state");
        let context_result = context.step(&ctx);
        assert_eq!(context_result.status, CommandStatus::Completed);
        assert!(matches!(
            context_result.events.as_slice(),
            [CommandEvent::OpenMenu(MenuRequest {
                kind: MenuRequestKind::Context { target, .. },
                ..
            })] if *target == target_id
        ));

        let mut call = CallState::from_request(
            &CommandRequest::new(CommandId::Call)
                .with_target(Some(target_id))
                .with_target2(Some(source_id))
                .with_data(CommandData::Text("Work".into())),
        )
        .expect("Call state");
        let call_result = call.step(&ctx);
        assert_eq!(call_result.status, CommandStatus::Completed);
        assert!(matches!(
            call_result.events.as_slice(),
            [CommandEvent::CallObjectFunction {
                object_id,
                target2: Some(argument),
                ..
            }] if *object_id == target_id && *argument == source_id
        ));

        let mut energy = EnergyState::from_request(
            &CommandRequest::new(CommandId::Energy)
                .with_target(Some(target_id))
                .with_target2(Some(source_id)),
        )
        .expect("Energy state");
        let energy_result = energy.step(&ctx);
        assert_eq!(energy_result.status, CommandStatus::Running);
        assert!(matches!(
            energy_result.events.as_slice(),
            [CommandEvent::CreateLine { from, to, .. }]
                if *from == source_id && *to == linekit_id
        ));

        let mut auto_energy = EnergyState::from_request(
            &CommandRequest::new(CommandId::Energy).with_target(Some(target_id)),
        )
        .expect("auto-source Energy state");
        assert_eq!(
            auto_energy.step(&ctx).status,
            CommandStatus::Failed,
            "inactive objects are retained references, not active Game.FindObject candidates"
        );

        let construct = ConstructState::from_request(
            &CommandRequest::new(CommandId::Construct).with_data(CommandData::Text("HUT1".into())),
        );
        assert_eq!(construct.builder_has_conkit(&ctx), Some(conkit_id));

        let activate = ActivateState::from_request(
            &CommandRequest::new(CommandId::Activate)
                .with_target2(Some(container_id))
                .with_data(CommandData::Text("ROCK".into())),
        )
        .expect("Activate state");
        assert_eq!(
            activate.find_release_candidate(&ctx, container_id, &HashSet::new()),
            Some(content_id)
        );
        let mut activate_inactive = ActivateState::from_request(
            &CommandRequest::new(CommandId::Activate).with_target(Some(target_id)),
        )
        .expect("inactive-target Activate state");
        assert_eq!(
            activate_inactive.step(&ctx).status,
            CommandStatus::Completed
        );
        let mut activate_deleted = CommandStack::new();
        activate_deleted
            .push_front(
                CommandRequest::new(CommandId::Activate).with_target(Some(deleted_content_id)),
            )
            .expect("removed-target Activate queues");
        assert!(activate_deleted.clear_object_reference(deleted_content_id));
        assert_eq!(
            activate_deleted
                .execute_front(&ctx)
                .map(|result| result.status),
            Some(CommandStatus::Failed)
        );

        let mut get = GetState::from_request(
            &CommandRequest::new(CommandId::Get)
                .with_target2(Some(container_id))
                .with_data(CommandData::Text("ROCK".into())),
        )
        .expect("Get state");
        assert_eq!(get.resolve_target(&ctx), Some(content_id));

        let mut put_contents = PutState::from_request(
            &CommandRequest::new(CommandId::Put).with_target(Some(container_id)),
        )
        .expect("Put contents state");
        assert_eq!(
            put_contents
                .resolve_item(&ctx)
                .expect("contents lookup succeeds")
                .map(|(id, _)| id),
            Some(conkit_id)
        );

        let mut put = PutState::from_request(
            &CommandRequest::new(CommandId::Put)
                .with_target(Some(container_id))
                .with_target2(Some(content_id)),
        )
        .expect("Put state");
        assert_eq!(put.step(&ctx).status, CommandStatus::Completed);

        let mut acquire = AcquireState::from_request(
            &CommandRequest::new(CommandId::Acquire)
                .with_data(CommandData::Text(CONKIT_DEFINITION.into()))
                .with_evaluated(true),
        )
        .expect("Acquire state");
        assert_eq!(acquire.step(&ctx).status, CommandStatus::Completed);

        let mut drop_state = DropState::from_request(&CommandRequest::new(CommandId::Drop));
        assert_eq!(
            drop_state.resolve_item(&ctx).map(|(id, _)| id),
            Some(conkit_id)
        );

        let throw_state =
            ThrowState::from_request(&CommandRequest::new(CommandId::Throw)).expect("Throw state");
        let throw_result = throw_state.step_object_com_throw(&ctx, false);
        assert!(matches!(
            throw_result.events.as_slice(),
            [CommandEvent::ThrowObject { object_id, .. }] if *object_id == conkit_id
        ));
        let mut cleared_throw_stack = CommandStack::new();
        cleared_throw_stack
            .push_front(CommandRequest::new(CommandId::Throw).with_target(Some(deleted_content_id)))
            .expect("removed-target Throw queues");
        assert!(cleared_throw_stack.clear_object_reference(deleted_content_id));
        let CommandState::Throw(cleared_throw) = &mut cleared_throw_stack
            .entries
            .front_mut()
            .expect("Throw remains")
            .state
        else {
            panic!("Throw remains at the front");
        };
        let gravity = crate::PhysicsSettings::default().gravity_as_c4fixed();
        let cleared_throw_result = cleared_throw.step_after_object_com_stop(&ctx, gravity);
        assert_eq!(cleared_throw.target, None);
        assert!(matches!(
            cleared_throw_result.events.as_slice(),
            [CommandEvent::ThrowObject { object_id, .. }] if *object_id == conkit_id
        ));

        let mut contained_actor = actor.clone();
        contained_actor.container = Some(container_id);
        let contained_ctx = CommandRuntimeContext {
            object: &contained_actor,
            ..ctx.clone()
        };
        let mut exit =
            ExitState::from_request(&CommandRequest::new(CommandId::Exit).with_evaluated(true))
                .expect("Exit state");
        let exit_result = exit.step(&contained_ctx);
        assert_eq!(exit_result.status, CommandStatus::Running);
        assert!(matches!(
            exit_result.events.as_slice(),
            [CommandEvent::ActivateEntrance { object_id, .. }] if *object_id == container_id
        ));

        let mut take2 =
            Take2State::from_request(&CommandRequest::new(CommandId::Take2)).expect("Take2 state");
        let take2_result = take2.step(&contained_ctx);
        assert_eq!(take2_result.status, CommandStatus::Completed);
        assert!(matches!(
            take2_result.events.as_slice(),
            [CommandEvent::OpenMenu(MenuRequest {
                kind: MenuRequestKind::Get { container },
                ..
            })] if *container == container_id
        ));

        let required_targets = [
            CommandRequest::new(CommandId::Build).with_target(Some(target_id)),
            CommandRequest::new(CommandId::Transfer).with_target(Some(target_id)),
            CommandRequest::new(CommandId::Chop).with_target(Some(target_id)),
            CommandRequest::new(CommandId::Put).with_target(Some(target_id)),
            CommandRequest::new(CommandId::Attack).with_target(Some(target_id)),
            CommandRequest::new(CommandId::Context).with_target2(Some(target_id)),
            CommandRequest::new(CommandId::Energy).with_target(Some(target_id)),
        ];
        for request in required_targets {
            let command = request.id;
            let mut stack = CommandStack::new();
            stack.push_front(request).expect("command queues");
            assert!(
                stack.clear_object_reference(target_id),
                "{} Target/Target2 is cleared",
                command.to_name()
            );
            let cleared_target = match &stack.entries.front().expect("entry retained").state {
                CommandState::Build(state) => state.target,
                CommandState::Transfer(state) => state.target,
                CommandState::Chop(state) => state.target,
                CommandState::Put(state) => state.container,
                CommandState::Attack(state) => state.target,
                CommandState::Context(state) => state.target,
                CommandState::Energy(state) => state.target,
                _ => panic!("unexpected required-target state"),
            };
            assert_eq!(cleared_target, ObjectId::new(0), "{}", command.to_name());
            assert_eq!(
                stack.execute_front(&ctx).map(|result| result.status),
                Some(CommandStatus::Failed),
                "cleared {} cannot execute through the still-resolvable inactive object",
                command.to_name()
            );
        }
    }

    #[test]
    fn enter_rechecks_an_opened_door_before_its_interval_expires() {
        // UpdateInterval is a command lifetime decremented before every
        // execution; it never throttles C4Command::Enter. After the first
        // call activates a closed door, the very next frame may enter it
        // (C4Command.cpp:545-616,1545-1555).
        let actor_id = ObjectId::new(32);
        let target_id = ObjectId::new(42);
        let mut actor = snapshot_with_id(actor_id.as_u64());
        actor.position = Vector2::new(10, 10);
        let mut target = snapshot_with_id(target_id.as_u64());
        target.shape = DefinitionRect::new(0, 0, 20, 20);
        target.entrance = Some(target.shape);
        target.ocf = ocf::ENTRANCE;
        target.entrance_status = false;
        let mut objects = HashMap::from([(actor_id, actor), (target_id, target)]);
        let players = HashMap::new();
        let definitions = HashMap::new();
        let mut stack = CommandStack::new();
        stack
            .push_front(
                CommandRequest::new(CommandId::Enter)
                    .with_target(Some(target_id))
                    .with_update_interval(50),
            )
            .expect("Enter queues");

        let actor_snapshot = objects.get(&actor_id).expect("actor present");
        let first_ctx = move_to_ctx_at_frame(actor_snapshot, &objects, &players, &definitions, 100);
        let first = stack.step(&first_ctx).expect("Enter executes");
        assert!(matches!(
            first.events.as_slice(),
            [CommandEvent::ActivateEntrance { .. }]
        ));

        objects
            .get_mut(&target_id)
            .expect("target present")
            .entrance_status = true;
        let actor_snapshot = objects.get(&actor_id).expect("actor present");
        let next_ctx = move_to_ctx_at_frame(actor_snapshot, &objects, &players, &definitions, 101);
        let next = stack.step(&next_ctx).expect("Enter rechecks");
        assert_eq!(next.status, CommandStatus::Completed);
        assert_eq!(
            next.events,
            vec![CommandEvent::EnterObject {
                object_id: actor_id,
                container_id: target_id,
            }]
        );
        assert!(stack.is_empty());
    }

    #[test]
    fn enter_interval_expires_successfully_after_exact_execution_count() {
        // The counter belongs to Execute, not ctx.frame. Enter(50) runs
        // its handler 49 times, then succeeds before handler #50 even when
        // all executions happen in the same frame (C4Command.cpp:1545-1552).
        let actor_id = ObjectId::new(33);
        let target_id = ObjectId::new(43);
        let actor = snapshot_with_id(actor_id.as_u64());
        let mut target = snapshot_with_id(target_id.as_u64());
        target.shape = DefinitionRect::new(-10, -10, 20, 20);
        target.entrance = Some(target.shape);
        target.ocf = ocf::ENTRANCE;
        target.entrance_status = false;
        let objects = HashMap::from([(actor_id, actor), (target_id, target)]);
        let players = HashMap::new();
        let definitions = HashMap::new();

        let mut stack = CommandStack::new();
        stack
            .push_back(CommandRequest::new(CommandId::Wait).with_mode(CommandMode::Base))
            .expect("base queues");
        stack
            .push_front(
                CommandRequest::new(CommandId::Enter)
                    .with_target(Some(target_id))
                    .with_update_interval(50)
                    .with_mode(CommandMode::SilentSub),
            )
            .expect("Enter queues");

        for execution in 1..=49 {
            let actor = objects.get(&actor_id).expect("actor present");
            let ctx = move_to_ctx_at_frame(actor, &objects, &players, &definitions, 7);
            let result = stack.step(&ctx).expect("Enter executes");
            assert_eq!(result.status, CommandStatus::Running);
            assert!(matches!(
                result.events.as_slice(),
                [CommandEvent::ActivateEntrance { .. }]
            ));

            if execution == 17 {
                let snapshot = stack.snapshot();
                assert_eq!(snapshot.commands[0].update_interval, Some(33));
                let mut restored = CommandStack::new();
                restored.restore_from_snapshot(&snapshot);
                stack = restored;
            }
        }

        let actor = objects.get(&actor_id).expect("actor present");
        let ctx = move_to_ctx_at_frame(actor, &objects, &players, &definitions, 7);
        let expired = stack.step(&ctx).expect("Enter expires");
        assert_eq!(expired.status, CommandStatus::Completed);
        assert!(expired.events.is_empty(), "expiry skips the handler");

        let snapshot = stack.snapshot();
        assert_eq!(snapshot.commands.len(), 1);
        assert_eq!(snapshot.commands[0].state.id(), Some(CommandId::Wait));
        assert_eq!(snapshot.commands[0].failures, 0);
    }

    #[test]
    fn enter_without_entrance_moves_to_center_and_expires_successfully() {
        let actor_id = ObjectId::new(35);
        let target_id = ObjectId::new(45);
        let mut actor = snapshot_with_id(actor_id.as_u64());
        actor.command_direction = CommandDirection::Right;
        let mut target = snapshot_with_id(target_id.as_u64());
        target.position = Vector2::new(75, 35);
        target.shape = DefinitionRect::new(65, 25, 20, 20);
        target.ocf = ocf::AVAILABLE;
        target.entrance = None;
        let objects = HashMap::from([(actor_id, actor), (target_id, target)]);
        let players = HashMap::new();
        let definitions = HashMap::new();

        let mut stack = CommandStack::new();
        stack
            .push_back(CommandRequest::new(CommandId::Wait).with_mode(CommandMode::Base))
            .expect("base queues");
        stack
            .push_front(
                CommandRequest::new(CommandId::Enter)
                    .with_target(Some(target_id))
                    .with_update_interval(2)
                    .with_mode(CommandMode::SilentSub),
            )
            .expect("Enter queues");

        let actor = objects.get(&actor_id).expect("actor present");
        let ctx = move_to_ctx_at_frame(actor, &objects, &players, &definitions, 7);
        let first = stack.step(&ctx).expect("Enter executes");
        assert_eq!(first.status, CommandStatus::Running);
        assert!(first.update.is_none(), "far Enter preserves ComDir");
        assert!(first.events.is_empty());
        assert!(
            first.operations.is_empty(),
            "stack applies child operations"
        );
        let snapshot = stack.snapshot();
        assert_eq!(snapshot.commands.len(), 3);
        assert_eq!(snapshot.commands[0].state.id(), Some(CommandId::MoveTo));
        let request = snapshot.commands[0]
            .request
            .as_ref()
            .expect("MoveTo retains its request");
        assert_eq!(request.id, CommandId::MoveTo);
        assert!(request.target.is_none());
        assert_eq!(request.tx, Some(75));
        assert_eq!(request.ty, Some(35));
        assert_eq!(request.update_interval, 50);
        assert!(!request.evaluated);

        assert!(stack.complete_front_if(CommandId::MoveTo));
        let expired = stack.step(&ctx).expect("Enter expires");
        assert_eq!(expired.status, CommandStatus::Completed);
        assert!(expired.events.is_empty());
        assert!(expired.operations.is_empty());
        let snapshot = stack.snapshot();
        assert_eq!(snapshot.commands.len(), 1);
        assert_eq!(snapshot.commands[0].state.id(), Some(CommandId::Wait));
        assert_eq!(snapshot.commands[0].failures, 0);
    }

    #[test]
    fn interval_one_succeeds_before_a_handler_without_interval_state() {
        let actor_id = ObjectId::new(34);
        let actor = snapshot_with_id(actor_id.as_u64());
        let objects = HashMap::from([(actor_id, actor)]);
        let players = HashMap::new();
        let definitions = HashMap::new();
        let actor = objects.get(&actor_id).expect("actor present");
        let ctx = move_to_ctx_at_frame(actor, &objects, &players, &definitions, 9);

        let mut stack = CommandStack::new();
        stack
            .push_front(
                CommandRequest::new(CommandId::Jump)
                    .with_tx(Some(100))
                    .with_update_interval(1),
            )
            .expect("Jump queues");
        let result = stack.step(&ctx).expect("Jump expires");
        assert_eq!(result.status, CommandStatus::Completed);
        assert!(result.events.is_empty(), "ObjectComJump must not run");
        assert!(stack.is_empty());
    }

    fn enter_push_fixture(
        vehicle_grab: i32,
        pushed_target: ObjectId,
        actor_position: Vector2,
        vehicle_position: Vector2,
    ) -> (
        CommandObjectSnapshot,
        HashMap<ObjectId, CommandObjectSnapshot>,
        HashMap<DefinitionId, CommandDefinitionSnapshot>,
    ) {
        let actor_id = ObjectId::new(31);
        let vehicle_id = ObjectId::new(41);
        let entrance_id = ObjectId::new(42);

        let mut actor = snapshot_with_id(actor_id.as_u64());
        actor.position = actor_position;
        actor.command_direction = CommandDirection::Right;
        actor.action_procedure = ActionProcedure::Push;
        actor.action_target = Some(pushed_target);
        actor.controller = 7;

        let mut vehicle = snapshot_with_id(vehicle_id.as_u64());
        vehicle.position = vehicle_position;
        vehicle.controller = 9;

        let mut entrance = snapshot_with_id(entrance_id.as_u64());
        entrance.position = Vector2::new(100, 0);
        entrance.shape = DefinitionRect::new(90, -10, 20, 20);
        entrance.entrance = Some(entrance.shape);
        entrance.ocf = ocf::ENTRANCE | ocf::AVAILABLE;
        entrance.entrance_status = false;
        entrance.category = CATEGORY_STRUCTURE;

        let objects = HashMap::from([
            (actor_id, actor.clone()),
            (vehicle_id, vehicle.clone()),
            (entrance_id, entrance.clone()),
        ]);
        let definitions = HashMap::from([
            (
                vehicle.definition_id.clone(),
                CommandDefinitionSnapshot {
                    value: 0,
                    can_chop: false,
                    chop_action: None,
                    constructable: false,
                    grab: vehicle_grab,
                    ..CommandDefinitionSnapshot::default()
                },
            ),
            (
                entrance.definition_id.clone(),
                CommandDefinitionSnapshot {
                    value: 0,
                    can_chop: false,
                    chop_action: None,
                    constructable: false,
                    grab: 1,
                    ..CommandDefinitionSnapshot::default()
                },
            ),
        ]);
        (actor, objects, definitions)
    }

    fn assert_enter_requests_ungrab(result: &CommandStepResult) {
        assert_eq!(result.status, CommandStatus::Running);
        assert!(result.update.is_none());
        assert!(result.events.is_empty());
        assert!(matches!(
            result.operations.as_slice(),
            [CommandOperation::PushFront(request)]
                if request.id == CommandId::UnGrab
                    && request.update_interval == 50
                    && request.mode == CommandMode::SilentSub
        ));
    }

    #[test]
    fn enter_push_ungrabs_grab_only_vehicle_or_missing_push_flag() {
        let vehicle_id = ObjectId::new(41);
        let entrance_id = ObjectId::new(42);
        let players = HashMap::new();

        for (grab, push_flag) in [(2, true), (1, false)] {
            let (actor, objects, definitions) =
                enter_push_fixture(grab, vehicle_id, Vector2::ZERO, Vector2::new(100, 0));
            let ctx = move_to_ctx_at_frame(&actor, &objects, &players, &definitions, 5);
            let mut request = CommandRequest::new(CommandId::Enter).with_target(Some(entrance_id));
            if push_flag {
                request = request.with_data(CommandData::Integer(COMMAND_FLAG_ENTER_PUSH_TARGET));
            }
            let mut state = EnterState::from_request(&request).expect("Enter state");
            assert_enter_requests_ungrab(&state.step(&ctx));
        }
    }

    #[test]
    fn enter_push_ungrabs_when_vehicle_is_the_entrance_target() {
        let entrance_id = ObjectId::new(42);
        let (actor, objects, definitions) =
            enter_push_fixture(1, entrance_id, Vector2::new(100, 0), Vector2::ZERO);
        let players = HashMap::new();
        let ctx = move_to_ctx_at_frame(&actor, &objects, &players, &definitions, 5);
        let request = CommandRequest::new(CommandId::Enter)
            .with_target(Some(entrance_id))
            .with_data(CommandData::Integer(COMMAND_FLAG_ENTER_PUSH_TARGET));
        let mut state = EnterState::from_request(&request).expect("Enter state");
        assert_enter_requests_ungrab(&state.step(&ctx));
    }

    #[test]
    fn enter_push_uses_vehicle_position_and_sets_its_enter_command() {
        let vehicle_id = ObjectId::new(41);
        let entrance_id = ObjectId::new(42);
        let players = HashMap::new();
        let request = CommandRequest::new(CommandId::Enter)
            .with_target(Some(entrance_id))
            .with_data(CommandData::Integer(COMMAND_FLAG_ENTER_PUSH_TARGET));

        // Actor inside but vehicle outside: the Enter must keep approaching.
        let (actor, objects, definitions) =
            enter_push_fixture(1, vehicle_id, Vector2::new(100, 0), Vector2::ZERO);
        let ctx = move_to_ctx_at_frame(&actor, &objects, &players, &definitions, 5);
        let mut state = EnterState::from_request(&request).expect("Enter state");
        let outside = state.step(&ctx);
        assert_eq!(outside.status, CommandStatus::Running);
        assert!(outside.events.is_empty());
        assert!(matches!(
            outside.operations.as_slice(),
            [CommandOperation::PushFront(request)]
                if request.id == CommandId::MoveTo
                    && request.target.is_none()
                    && request.tx == Some(100)
                    && request.ty == Some(0)
                    && request.data
                        == CommandData::Integer(COMMAND_FLAG_MOVE_TO_PUSH_TARGET)
        ));

        // Actor outside but vehicle inside: assign Enter to the vehicle and
        // finish the actor's command without entering the actor itself.
        let (actor, objects, definitions) =
            enter_push_fixture(1, vehicle_id, Vector2::ZERO, Vector2::new(100, 0));
        let ctx = move_to_ctx_at_frame(&actor, &objects, &players, &definitions, 5);
        let mut state = EnterState::from_request(&request).expect("Enter state");
        let inside = state.step(&ctx);
        assert_eq!(inside.status, CommandStatus::Completed);
        assert!(inside.operations.is_empty());
        assert_eq!(
            inside.update.and_then(|update| update.command_direction),
            Some(CommandDirection::Stop)
        );
        assert!(matches!(
            inside.events.as_slice(),
            [CommandEvent::SetObjectCommand {
                object_id,
                controller: None,
                request,
            }] if *object_id == vehicle_id
                && request.id == CommandId::Enter
                && request.target == Some(entrance_id)
                && request.mode == CommandMode::Base
        ));
    }

    #[test]
    fn enter_inside_shape_outside_entrance_moves_to_entrance_center() {
        let actor_id = ObjectId::new(31);
        let target_id = ObjectId::new(41);

        let mut actor = snapshot_with_id(actor_id.as_u64());
        actor.position = Vector2::new(111, 0);
        actor.command_direction = CommandDirection::Left;

        let mut target = snapshot_with_id(target_id.as_u64());
        target.position = Vector2::new(120, 0);
        target.shape = DefinitionRect::new(target.position.x - 10, target.position.y - 10, 20, 20);
        target.entrance = Some(DefinitionRect::new(124, -4, 6, 8));
        target.ocf = ocf::ENTRANCE | ocf::AVAILABLE;
        target.category = CATEGORY_STRUCTURE;

        let mut objects = HashMap::new();
        objects.insert(actor.id, actor.clone());
        objects.insert(target.id, target);

        let players: HashMap<i32, CommandPlayerSnapshot> = HashMap::new();
        let definitions: HashMap<DefinitionId, CommandDefinitionSnapshot> = HashMap::new();
        let ctx = CommandRuntimeContext {
            landscape: None,
            frame: 5,
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

        let parent_request = CommandRequest::new(CommandId::Enter).with_target(Some(target_id));
        let mut state = EnterState::from_request(&parent_request).expect("state created");

        let result = state.step(&ctx);
        assert_eq!(result.status, CommandStatus::Running);
        assert!(
            result.update.is_none(),
            "the far branch does not stop ComDir before MoveTo executes"
        );
        assert!(result.events.is_empty());
        assert_eq!(result.operations.len(), 1);
        match &result.operations[0] {
            CommandOperation::PushFront(request) => {
                assert_eq!(request.id, CommandId::MoveTo);
                assert_eq!(request.target, None);
                assert_eq!((request.tx, request.ty), (Some(127), Some(0)));
                assert_eq!(request.update_interval, 50);
                assert_eq!(
                    request.data,
                    CommandData::None,
                    "no Enter_PushTarget: the MoveTo gets no PushTarget either"
                );
            }
            other => panic!("unexpected operation: {:?}", other),
        }
        let first_move = pushed_request(&result.operations, CommandId::MoveTo);
        let reissued = state.step(&ctx);
        assert_eq!(
            pushed_request(&reissued.operations, CommandId::MoveTo),
            first_move,
            "Enter reissues MoveTo on its next execution"
        );
        assert_silent_child_failure_propagates(parent_request, first_move, &ctx);

        // C4Command::Enter passes C4CMD_MoveTo_PushTarget through when
        // its own Data carries C4CMD_Enter_PushTarget (C4Command.cpp:615).
        let mut state = EnterState::from_request(
            &CommandRequest::new(CommandId::Enter)
                .with_target(Some(target_id))
                .with_data(CommandData::Integer(2)),
        )
        .expect("state created");
        let result = state.step(&ctx);
        match &result.operations[0] {
            CommandOperation::PushFront(request) => {
                assert_eq!(request.id, CommandId::MoveTo);
                assert_eq!(
                    request.data,
                    CommandData::Integer(2),
                    "Enter_PushTarget -> MoveTo_PushTarget (C4Command.cpp:615)"
                );
            }
            other => panic!("unexpected operation: {:?}", other),
        }
    }

    #[test]
    fn enter_reissues_move_to_immediately_after_child_is_removed() {
        let actor_id = ObjectId::new(42);
        let target_id = ObjectId::new(43);
        let actor = snapshot_with_id(actor_id.as_u64());
        let mut target = snapshot_with_id(target_id.as_u64());
        target.position = Vector2::new(120, 0);
        target.shape = DefinitionRect::new(110, -10, 20, 20);
        target.entrance = Some(target.shape);
        target.ocf = ocf::ENTRANCE | ocf::AVAILABLE;
        let objects = HashMap::from([(actor_id, actor), (target_id, target)]);
        let players = HashMap::new();
        let definitions = HashMap::new();

        let mut stack = CommandStack::new();
        stack
            .push_front(CommandRequest::new(CommandId::Enter).with_target(Some(target_id)))
            .expect("Enter queues");

        let actor = objects.get(&actor_id).expect("actor present");
        let ctx = move_to_ctx_at_frame(actor, &objects, &players, &definitions, 10);
        stack.step(&ctx).expect("Enter requests MoveTo");
        assert_eq!(stack.command_names(), vec!["MoveTo", "Enter"]);
        let first_move = stack
            .entries
            .front()
            .and_then(|entry| entry.request.clone())
            .expect("MoveTo request retained");
        assert!(stack.complete_front_if(CommandId::MoveTo));

        let ctx = move_to_ctx_at_frame(actor, &objects, &players, &definitions, 11);
        stack.step(&ctx).expect("Enter reissues MoveTo");
        assert_eq!(stack.command_names(), vec!["MoveTo", "Enter"]);
        assert_eq!(
            stack
                .entries
                .front()
                .and_then(|entry| entry.request.clone()),
            Some(first_move)
        );
    }

    #[test]
    fn enter_moves_toward_the_target_while_contained_elsewhere() {
        // Even while its point lies inside the target shape and entrance,
        // a contained actor may not take the direct-enter branch. It queues
        // MoveTo, whose own first movement step exits the old container
        // (C4Command.cpp:586-615,213-217).
        let actor_id = ObjectId::new(51);
        let current_container_id = ObjectId::new(52);
        let target_id = ObjectId::new(53);
        let mut actor = snapshot_with_id(actor_id.as_u64());
        actor.position = Vector2::new(120, 20);
        actor.container = Some(current_container_id);
        let current_container = snapshot_with_id(current_container_id.as_u64());
        let mut target = snapshot_with_id(target_id.as_u64());
        target.position = Vector2::new(120, 20);
        target.shape = DefinitionRect::new(110, 10, 20, 20);
        target.entrance = Some(DefinitionRect::new(115, 15, 10, 10));
        target.ocf = ocf::ENTRANCE;
        target.entrance_status = true;
        let objects = HashMap::from([
            (actor_id, actor.clone()),
            (current_container_id, current_container),
            (target_id, target),
        ]);
        let players = HashMap::new();
        let definitions = HashMap::new();
        let ctx = move_to_ctx_at_frame(&actor, &objects, &players, &definitions, 10);
        let mut state = EnterState::from_request(
            &CommandRequest::new(CommandId::Enter).with_target(Some(target_id)),
        )
        .expect("Enter state");

        let result = state.step(&ctx);

        assert_eq!(result.status, CommandStatus::Running);
        assert!(
            result.events.is_empty(),
            "contained actors never direct-enter"
        );
        match result.operations.as_slice() {
            [CommandOperation::PushFront(request)] => {
                assert_eq!(request.id, CommandId::MoveTo);
                assert_eq!(request.target, None);
                assert_eq!((request.tx, request.ty), (Some(120), Some(20)));
                assert_eq!(request.update_interval, 50);
            }
            other => panic!("expected MoveTo target entrance, got {other:?}"),
        }
    }

    #[test]
    fn grab_requests_move_when_far() {
        let actor_id = ObjectId::new(200);
        let target_id = ObjectId::new(300);

        let mut actor = snapshot_with_id(actor_id.as_u64());
        actor.position = Vector2::new(0, 0);
        actor.command_direction = CommandDirection::Left;
        actor.action_procedure = ActionProcedure::Walk;

        let mut target = snapshot_with_id(target_id.as_u64());
        target.position = Vector2::new(60, 0);
        target.shape = DefinitionRect::new(target.position.x - 10, target.position.y - 10, 20, 20);
        // C++ passes OCF_All to At: any nonzero OCF qualifies; the target
        // does not need OCF_Grab.
        target.ocf = ocf::NORMAL;

        let mut objects = HashMap::new();
        objects.insert(actor.id, actor.clone());
        objects.insert(target.id, target.clone());

        let players: HashMap<i32, CommandPlayerSnapshot> = HashMap::new();
        let definitions: HashMap<DefinitionId, CommandDefinitionSnapshot> = HashMap::new();
        let ctx = CommandRuntimeContext {
            landscape: None,
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

        let mut state = GrabState::from_request(
            &CommandRequest::new(CommandId::Grab).with_target(Some(target_id)),
        )
        .expect("state created");

        let result = state.step(&ctx);
        assert_eq!(result.status, CommandStatus::Running);
        assert!(
            result.update.is_none(),
            "C4Command::Grab changes ComDir only after an accepted at-target attempt"
        );
        assert!(result.events.is_empty());
        assert_eq!(result.operations.len(), 1);
        match &result.operations[0] {
            CommandOperation::PushFront(request) => {
                assert_eq!(request.id, CommandId::MoveTo);
                assert_eq!(request.tx, Some(target.position.x));
                assert_eq!(request.ty, Some(target.position.y));
                assert_eq!(request.update_interval, 50);
            }
            other => panic!("unexpected operation: {:?}", other),
        }
        let first_move = pushed_request(&result.operations, CommandId::MoveTo);
        let reissued = state.step(&ctx);
        assert_eq!(
            pushed_request(&reissued.operations, CommandId::MoveTo),
            first_move,
            "Grab reissues MoveTo on its next execution"
        );
    }

    #[test]
    fn grab_retained_status_zero_target_queues_move_without_stopping() {
        let actor_id = ObjectId::new(201);
        let target_id = ObjectId::new(301);
        let mut actor = snapshot_with_id(actor_id.as_u64());
        actor.position = Vector2::new(10, 10);
        actor.command_direction = CommandDirection::Left;
        actor.action_procedure = ActionProcedure::Walk;

        let mut target = snapshot_with_id(target_id.as_u64());
        target.position = Vector2::new(40, 10);
        target.shape = DefinitionRect::new(30, 0, 20, 20);
        target.ocf = ocf::NORMAL;
        target.status = crate::ObjectStatus::Inactive;
        target.alive = false;

        let objects = HashMap::from([(actor_id, actor.clone()), (target_id, target)]);
        let players = HashMap::new();
        let definitions = HashMap::new();
        let ctx = move_to_ctx_at_frame(&actor, &objects, &players, &definitions, 0);
        let mut state = GrabState::from_request(
            &CommandRequest::new(CommandId::Grab).with_target(Some(target_id)),
        )
        .expect("Grab state");

        let result = state.step(&ctx);

        assert_eq!(result.status, CommandStatus::Running);
        assert!(result.update.is_none());
        assert!(result.events.is_empty());
        assert!(matches!(
            result.operations.as_slice(),
            [CommandOperation::PushFront(request)]
                if request.id == CommandId::MoveTo
                    && request.tx == Some(40)
                    && request.ty == Some(10)
        ));

        let mut retained_deleted_objects = objects.clone();
        let deleted_target = retained_deleted_objects
            .get_mut(&target_id)
            .expect("target remains addressable as a tombstone");
        deleted_target.status = crate::ObjectStatus::Deleted;
        deleted_target.destroyed = true;
        let deleted_ctx = move_to_ctx_at_frame(
            retained_deleted_objects
                .get(&actor_id)
                .expect("actor present"),
            &retained_deleted_objects,
            &players,
            &definitions,
            0,
        );
        let mut deleted_state = GrabState::from_request(
            &CommandRequest::new(CommandId::Grab).with_target(Some(target_id)),
        )
        .expect("deleted-target Grab state");
        let deleted_result = deleted_state.step(&deleted_ctx);
        assert_eq!(deleted_result.status, CommandStatus::Running);
        assert!(matches!(
            deleted_result.operations.as_slice(),
            [CommandOperation::PushFront(request)]
                if request.id == CommandId::MoveTo
                    && request.tx == Some(40)
                    && request.ty == Some(10)
        ));
    }

    #[test]
    fn grab_starts_push_when_in_range() {
        let actor_id = ObjectId::new(310);
        let target_id = ObjectId::new(320);

        let mut actor = snapshot_with_id(actor_id.as_u64());
        actor.position = Vector2::new(10, 10);
        actor.command_direction = CommandDirection::Right;
        actor.action_procedure = ActionProcedure::Walk;

        let mut target = snapshot_with_id(target_id.as_u64());
        target.position = Vector2::new(14, 12);
        // C4Command::Grab tests the actor point in the target's shape
        // (Target->At, C4Command.cpp:689-691).
        target.shape = DefinitionRect::new(target.position.x - 10, target.position.y - 10, 20, 20);
        target.ocf = ocf::GRAB | ocf::AVAILABLE;

        let mut objects = HashMap::new();
        objects.insert(actor.id, actor.clone());
        objects.insert(target.id, target);

        let players: HashMap<i32, CommandPlayerSnapshot> = HashMap::new();
        let definitions: HashMap<DefinitionId, CommandDefinitionSnapshot> = HashMap::new();
        let ctx = CommandRuntimeContext {
            landscape: None,
            frame: 1,
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

        let mut state = GrabState::from_request(
            &CommandRequest::new(CommandId::Grab).with_target(Some(target_id)),
        )
        .expect("state created");

        let result = state.step(&ctx);
        assert_eq!(result.status, CommandStatus::Running);
        assert!(result.operations.is_empty());
        assert!(
            result.update.is_none(),
            "RejectGrabbed must run before Stop or Push"
        );
        assert_eq!(
            result.events,
            vec![CommandEvent::AttemptGrab {
                actor_id,
                target_id,
            }]
        );
    }

    #[test]
    fn grab_at_uses_ocf_all_and_requires_an_uncontained_target() {
        let actor_id = ObjectId::new(312);
        let target_id = ObjectId::new(322);
        let container_id = ObjectId::new(323);
        let mut actor = snapshot_with_id(actor_id.as_u64());
        actor.position = Vector2::new(10, 10);
        actor.action_procedure = ActionProcedure::Walk;

        let mut target = snapshot_with_id(target_id.as_u64());
        target.position = Vector2::new(10, 10);
        target.shape = DefinitionRect::new(0, 0, 20, 20);

        let run = |target: CommandObjectSnapshot| {
            let objects = HashMap::from([(actor_id, actor.clone()), (target_id, target)]);
            let players = HashMap::new();
            let definitions = HashMap::new();
            let ctx = move_to_ctx_at_frame(
                objects.get(&actor_id).expect("actor present"),
                &objects,
                &players,
                &definitions,
                1,
            );
            GrabState::from_request(
                &CommandRequest::new(CommandId::Grab).with_target(Some(target_id)),
            )
            .expect("Grab state")
            .step(&ctx)
        };

        target.ocf = ocf::NONE;
        let zero_ocf = run(target.clone());
        assert!(zero_ocf.events.is_empty());
        assert!(matches!(
            zero_ocf.operations.as_slice(),
            [CommandOperation::PushFront(request)] if request.id == CommandId::MoveTo
        ));

        target.ocf = ocf::NORMAL;
        target.container = Some(container_id);
        let contained = run(target.clone());
        assert!(contained.events.is_empty());
        assert!(matches!(
            contained.operations.as_slice(),
            [CommandOperation::PushFront(request)] if request.id == CommandId::MoveTo
        ));

        target.container = None;
        let normal = run(target);
        assert_eq!(
            normal.events,
            vec![CommandEvent::AttemptGrab {
                actor_id,
                target_id,
            }]
        );
    }

