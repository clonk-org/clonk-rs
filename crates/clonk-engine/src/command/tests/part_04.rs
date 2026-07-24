// Contiguous slice 4 of 7 of the `command/tests` battery, spliced by
// `include!` from the parent module so every test id is unchanged.

    #[test]
    fn null_target_grab_ungrabs_then_fails_into_base_retry() {
        let actor_id = ObjectId::new(310);
        let pushed_id = ObjectId::new(320);
        let mut actor = snapshot_with_id(actor_id.as_u64());
        actor.action_procedure = ActionProcedure::Push;
        actor.action_target = Some(pushed_id);

        let pushed = snapshot_with_id(pushed_id.as_u64());
        let mut objects = HashMap::from([(actor_id, actor), (pushed_id, pushed)]);
        let players = HashMap::new();
        let definitions = HashMap::new();

        let mut stack = CommandStack::new();
        stack
            .push_back(
                CommandRequest::new(CommandId::Wait)
                    .with_retries(1)
                    .with_mode(CommandMode::Base),
            )
            .expect("base queues");
        stack
            .push_front(CommandRequest::new(CommandId::Grab).with_mode(CommandMode::SilentSub))
            .expect("C++ accepts a targetless Grab");

        let actor = objects.get(&actor_id).expect("actor present");
        let ctx = move_to_ctx_at_frame(actor, &objects, &players, &definitions, 1);
        let first = stack.step(&ctx).expect("targetless Grab executes");
        assert_eq!(first.status, CommandStatus::Running);
        assert_eq!(
            stack.snapshot().command_names(),
            vec!["UnGrab", "Grab", "Wait"],
            "a pushing actor lets go before the null-target failure"
        );
        assert_eq!(stack.snapshot().commands[2].failures, 0);

        // The live UnGrab path owns the action transition and callback. Its
        // command is complete before the original Grab executes again.
        stack.pop_front();
        let actor = objects.get_mut(&actor_id).expect("actor present");
        actor.action_procedure = ActionProcedure::Walk;
        actor.action_target = None;

        let actor = objects.get(&actor_id).expect("actor present");
        let ctx = move_to_ctx_at_frame(actor, &objects, &players, &definitions, 2);
        let failed = stack.step(&ctx).expect("Grab re-executes");
        assert_eq!(failed.status, CommandStatus::Failed);
        let after_failure = stack.snapshot();
        assert_eq!(after_failure.command_names(), vec!["Wait"]);
        assert_eq!(after_failure.commands[0].failures, 1);
        assert_eq!(after_failure.commands[0].retries, 1);

        let retry = stack.step(&ctx).expect("base consumes its failure");
        assert_eq!(retry.status, CommandStatus::Running);
        let during_retry = stack.snapshot();
        assert_eq!(during_retry.command_names(), vec!["Retry", "Wait"]);
        assert_eq!(during_retry.commands[1].failures, 0);
        assert_eq!(during_retry.commands[1].retries, 0);
    }

    #[test]
    fn grab_subcommand_rechecks_fulfilled_condition_on_next_execution() {
        let actor_id = ObjectId::new(311);
        let target_id = ObjectId::new(321);
        let mut actor = snapshot_with_id(actor_id.as_u64());
        actor.position = Vector2::new(10, 10);
        let mut target = snapshot_with_id(target_id.as_u64());
        target.position = Vector2::new(14, 12);
        target.shape = DefinitionRect::new(4, 2, 20, 20);
        target.ocf = ocf::GRAB | ocf::AVAILABLE;
        let mut objects = HashMap::from([(actor_id, actor), (target_id, target)]);
        let players = HashMap::new();
        let definitions = HashMap::new();

        let mut stack = CommandStack::new();
        stack
            .push_back(CommandRequest::new(CommandId::Wait).with_mode(CommandMode::Base))
            .expect("base queues");
        stack
            .push_front(
                CommandRequest::new(CommandId::Grab)
                    .with_target(Some(target_id))
                    .with_update_interval(40)
                    .with_mode(CommandMode::SilentSub),
            )
            .expect("Grab queues");

        let actor = objects.get(&actor_id).expect("actor present");
        let ctx = move_to_ctx_at_frame(actor, &objects, &players, &definitions, 1);
        let first = stack.step(&ctx).expect("Grab executes");
        assert_eq!(
            first.events,
            vec![CommandEvent::AttemptGrab {
                actor_id,
                target_id,
            }]
        );
        assert_eq!(stack.resolve_grab_attempt(target_id, false), Some(true));

        let actor = objects.get_mut(&actor_id).expect("actor present");
        actor.action_procedure = ActionProcedure::Push;
        actor.action_target = Some(target_id);
        let actor = objects.get(&actor_id).expect("actor present");
        let ctx = move_to_ctx_at_frame(actor, &objects, &players, &definitions, 2);
        let fulfilled = stack.step(&ctx).expect("Grab rechecks");
        assert_eq!(fulfilled.status, CommandStatus::Completed);

        let snapshot = stack.snapshot();
        assert_eq!(snapshot.commands.len(), 1);
        assert_eq!(snapshot.commands[0].state.id(), Some(CommandId::Wait));
        assert_eq!(snapshot.commands[0].failures, 0);
    }

    #[test]
    fn reject_grab_finishes_direct_command_as_silent_base() {
        let target = ObjectId::new(321);
        let mut stack = CommandStack::new();
        stack
            .push_front(CommandRequest::new(CommandId::Grab).with_target(Some(target)))
            .expect("Grab queues");
        let Some(CommandState::Grab(state)) =
            stack.entries.front_mut().map(|entry| &mut entry.state)
        else {
            panic!("Grab is front");
        };
        state.reject_pending = true;

        assert_eq!(stack.resolve_grab_attempt(target, true), Some(true));
        let snapshot = stack.snapshot();
        assert_eq!(snapshot.commands[0].mode, CommandMode::SilentBase);
        assert_eq!(snapshot.commands[0].finished, Some(CommandStatus::Failed));
        assert_eq!(snapshot.commands[0].failures, 0);
    }

    #[test]
    fn reject_grab_propagates_sub_failure_to_first_unfinished_command() {
        let target = ObjectId::new(322);
        let mut stack = CommandStack::new();
        stack
            .push_back(CommandRequest::new(CommandId::Wait).with_mode(CommandMode::Base))
            .expect("base queues");
        stack
            .push_front(
                CommandRequest::new(CommandId::Grab)
                    .with_target(Some(target))
                    .with_mode(CommandMode::Sub),
            )
            .expect("Grab queues");
        let Some(CommandState::Grab(state)) =
            stack.entries.front_mut().map(|entry| &mut entry.state)
        else {
            panic!("Grab is front");
        };
        state.reject_pending = true;

        assert_eq!(stack.resolve_grab_attempt(target, true), Some(true));
        let snapshot = stack.snapshot();
        assert_eq!(snapshot.commands[0].mode, CommandMode::Sub);
        assert_eq!(snapshot.commands[0].finished, Some(CommandStatus::Failed));
        assert_eq!(snapshot.commands[1].failures, 1);
    }

    #[test]
    fn reject_grab_resolves_marked_command_below_callback_added_front() {
        let target = ObjectId::new(323);
        let mut stack = CommandStack::new();
        stack
            .push_front(CommandRequest::new(CommandId::Grab).with_target(Some(target)))
            .expect("Grab queues");
        let Some(CommandState::Grab(state)) =
            stack.entries.front_mut().map(|entry| &mut entry.state)
        else {
            panic!("Grab is front");
        };
        state.reject_pending = true;
        stack
            .push_front(CommandRequest::new(CommandId::Wait))
            .expect("callback command queues");

        assert_eq!(stack.resolve_grab_attempt(target, true), Some(true));
        let snapshot = stack.snapshot();
        assert_eq!(snapshot.commands[0].state.id(), Some(CommandId::Wait));
        assert_eq!(snapshot.commands[0].finished, None);
        assert_eq!(snapshot.commands[1].state.id(), Some(CommandId::Grab));
        assert_eq!(snapshot.commands[1].finished, Some(CommandStatus::Failed));
        assert_eq!(snapshot.commands[1].mode, CommandMode::SilentBase);
    }

    #[test]
    fn reject_grab_does_not_finish_same_target_replacement() {
        let target = ObjectId::new(324);
        let mut stack = CommandStack::new();
        stack
            .push_front(CommandRequest::new(CommandId::Grab).with_target(Some(target)))
            .expect("Grab queues");
        let Some(CommandState::Grab(state)) =
            stack.entries.front_mut().map(|entry| &mut entry.state)
        else {
            panic!("Grab is front");
        };
        state.reject_pending = true;

        stack.clear();
        stack
            .push_front(CommandRequest::new(CommandId::Grab).with_target(Some(target)))
            .expect("replacement Grab queues");
        assert_eq!(stack.resolve_grab_attempt(target, true), Some(true));
        assert_eq!(stack.snapshot().commands[0].finished, None);
    }

    #[test]
    fn detached_grab_preserves_clear_pointer_order() {
        fn marked_grab(target: ObjectId) -> CommandStack {
            let mut stack = CommandStack::new();
            stack
                .push_front(CommandRequest::new(CommandId::Grab).with_target(Some(target)))
                .expect("Grab queues");
            let CommandState::Grab(state) = &mut stack.entries[0].state else {
                panic!("Grab is front");
            };
            state.reject_pending = true;
            stack
        }

        let target = ObjectId::new(326);
        let mut cleared_first = marked_grab(target);
        assert!(cleared_first.clear_object_reference(target));
        cleared_first.clear();
        cleared_first
            .push_front(CommandRequest::new(CommandId::Grab).with_target(Some(target)))
            .expect("replacement queues");
        assert_eq!(
            cleared_first.resolve_grab_attempt(target, false),
            Some(false)
        );
        assert_eq!(cleared_first.snapshot().commands[0].finished, None);

        let mut replaced_first = marked_grab(target);
        replaced_first.clear();
        replaced_first
            .push_front(CommandRequest::new(CommandId::Grab).with_target(Some(target)))
            .expect("replacement queues");
        assert!(replaced_first.clear_object_reference(target));
        assert_eq!(
            replaced_first.resolve_grab_attempt(target, false),
            Some(true)
        );
        assert_eq!(replaced_first.snapshot().commands[0].finished, None);
    }

    #[test]
    fn detached_same_target_attempts_resolve_lifo() {
        let target = ObjectId::new(327);
        let mut stack = CommandStack::new();
        stack
            .push_front(CommandRequest::new(CommandId::Grab).with_target(Some(target)))
            .expect("outer Grab queues");
        let CommandState::Grab(outer) = &mut stack.entries[0].state else {
            panic!("outer Grab is front");
        };
        outer.reject_pending = true;
        outer.target_cleared = true;
        stack
            .push_front(CommandRequest::new(CommandId::Grab).with_target(Some(target)))
            .expect("nested Grab queues");
        let CommandState::Grab(nested) = &mut stack.entries[0].state else {
            panic!("nested Grab is front");
        };
        nested.reject_pending = true;

        stack.clear();
        assert_eq!(stack.resolve_grab_attempt(target, false), Some(true));
        assert_eq!(stack.resolve_grab_attempt(target, false), Some(false));
    }

    #[test]
    fn grab_attempt_tracks_cleared_target_without_legacy_request() {
        let target = ObjectId::new(325);
        let mut stack = CommandStack::new();
        stack
            .push_front(CommandRequest::new(CommandId::Grab).with_target(Some(target)))
            .expect("Grab queues");
        stack.entries[0].request = None;
        let CommandState::Grab(state) = &mut stack.entries[0].state else {
            panic!("Grab is front");
        };
        state.reject_pending = true;
        assert_eq!(stack.resolve_grab_attempt(target, false), Some(true));

        let CommandState::Grab(state) = &mut stack.entries[0].state else {
            panic!("Grab is front");
        };
        state.reject_pending = true;
        assert!(stack.clear_object_reference(target));
        assert_eq!(stack.resolve_grab_attempt(target, false), Some(false));
    }

    #[test]
    fn grab_completes_when_already_pushing_target() {
        let actor_id = ObjectId::new(330);
        let target_id = ObjectId::new(340);

        let mut actor = snapshot_with_id(actor_id.as_u64());
        actor.action_procedure = ActionProcedure::Push;
        actor.action_target = Some(target_id);
        actor.command_direction = CommandDirection::Right;

        let mut target = snapshot_with_id(target_id.as_u64());
        target.ocf = ocf::GRAB | ocf::AVAILABLE;

        let mut objects = HashMap::new();
        objects.insert(actor.id, actor.clone());
        objects.insert(target.id, target);

        let players: HashMap<i32, CommandPlayerSnapshot> = HashMap::new();
        let definitions: HashMap<DefinitionId, CommandDefinitionSnapshot> = HashMap::new();
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

        let mut state = GrabState::from_request(
            &CommandRequest::new(CommandId::Grab).with_target(Some(target_id)),
        )
        .expect("state created");

        let result = state.step(&ctx);
        assert_eq!(result.status, CommandStatus::Completed);
        assert!(result.update.is_none());
        assert!(result.operations.is_empty());
    }

    #[test]
    fn grab_requests_ungrab_when_pushing_other_target() {
        let actor_id = ObjectId::new(350);
        let target_id = ObjectId::new(360);
        let other_id = ObjectId::new(361);

        let mut actor = snapshot_with_id(actor_id.as_u64());
        actor.action_procedure = ActionProcedure::Push;
        actor.action_target = Some(other_id);
        actor.command_direction = CommandDirection::Right;

        let mut target = snapshot_with_id(target_id.as_u64());
        target.position = Vector2::new(15, 0);
        target.ocf = ocf::GRAB | ocf::AVAILABLE;

        let mut other = snapshot_with_id(other_id.as_u64());
        other.ocf = ocf::GRAB | ocf::AVAILABLE;

        let mut objects = HashMap::new();
        objects.insert(actor.id, actor.clone());
        objects.insert(target.id, target);
        objects.insert(other.id, other);

        let players: HashMap<i32, CommandPlayerSnapshot> = HashMap::new();
        let definitions: HashMap<DefinitionId, CommandDefinitionSnapshot> = HashMap::new();
        let ctx = CommandRuntimeContext {
            landscape: None,
            frame: 3,
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
        assert!(result.update.is_none());
        assert_eq!(result.operations.len(), 1);
        match &result.operations[0] {
            CommandOperation::PushFront(request) => {
                assert_eq!(request.id, CommandId::UnGrab);
                assert_eq!(request.update_interval, 50);
            }
            other => panic!("unexpected operation: {:?}", other),
        }
    }

    #[test]
    fn push_to_completes_when_target_in_destination_container() {
        let actor_id = ObjectId::new(400);
        let target_id = ObjectId::new(401);
        let destination_id = ObjectId::new(402);

        let mut actor = snapshot_with_id(actor_id.as_u64());
        actor.action_procedure = ActionProcedure::Dig;
        actor.command_direction = CommandDirection::Right;

        let mut target = snapshot_with_id(target_id.as_u64());
        target.container = Some(destination_id);
        target.ocf |= ocf::GRAB;

        let destination = snapshot_with_id(destination_id.as_u64());

        let mut objects = HashMap::new();
        objects.insert(actor.id, actor.clone());
        objects.insert(target.id, target);
        objects.insert(destination.id, destination);

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

        let mut state = PushToState::from_request(
            &CommandRequest::new(CommandId::PushTo)
                .with_target(Some(target_id))
                .with_target2(Some(destination_id)),
        )
        .expect("state created");

        let result = state.step(&ctx);
        assert_eq!(result.status, CommandStatus::Completed);
        assert!(
            result.update.is_none(),
            "container fulfillment precedes PushTo's work-action stop"
        );
        assert!(result.operations.is_empty());
    }

    #[test]
    fn push_to_requests_activate_when_target_contained_elsewhere() {
        let actor_id = ObjectId::new(410);
        let target_id = ObjectId::new(411);
        let container_id = ObjectId::new(412);

        let actor = snapshot_with_id(actor_id.as_u64());

        let mut target = snapshot_with_id(target_id.as_u64());
        target.position = Vector2::new(30, 0);
        target.container = Some(container_id);
        target.ocf |= ocf::GRAB;

        let container = snapshot_with_id(container_id.as_u64());

        let mut objects = HashMap::new();
        objects.insert(actor.id, actor.clone());
        objects.insert(target.id, target);
        objects.insert(container.id, container);

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

        let mut state = PushToState::from_request(
            &CommandRequest::new(CommandId::PushTo).with_target(Some(target_id)),
        )
        .expect("state created");

        let result = state.step(&ctx);
        assert_eq!(result.status, CommandStatus::Running);
        assert_eq!(result.operations.len(), 1);
        match &result.operations[0] {
            CommandOperation::PushFront(request) => {
                assert_eq!(request.id, CommandId::Activate);
                assert_eq!(request.target, Some(target_id));
                assert_eq!(request.update_interval, 40);
            }
            other => panic!("expected activate request, got {:?}", other),
        }
    }

    #[test]
    fn construct_without_definition_opens_menu_after_capability_gate() {
        let builder_id = ObjectId::new(1);
        let mut builder = snapshot_with_id(builder_id.as_u64());
        builder.owner = 42;
        builder.command_direction = CommandDirection::Right;
        builder.physical.can_construct = 1;
        let objects = HashMap::from([(builder_id, builder.clone())]);
        let players = HashMap::from([(
            42,
            CommandPlayerSnapshot {
                status: PlayerStatus::Eliminated,
                surrendered: false,
                wealth: 0,
                home_base_material: HashMap::new(),
                home_base_material_entries: Vec::new(),
                knowledge: Vec::new(),
                hostile_to: Vec::new(),
            },
        )]);
        let definitions = HashMap::new();
        let ctx = CommandRuntimeContext {
            landscape: None,
            frame: 0,
            position: builder.position,
            object: &builder,
            objects: &objects,
            players: &players,
            definitions: &definitions,
            structures_need_energy: false,
            base_buy_enabled: true,
            base_sell_enabled: true,
            transfer_zones: &EMPTY_TRANSFER_ZONES,
            rng: None,
        };

        let mut state = ConstructState::from_request(&CommandRequest::new(CommandId::Construct));
        let result = state.step(&ctx);

        assert_eq!(result.status, CommandStatus::Completed);
        assert!(result.update.is_none());
        assert_eq!(
            result.events,
            [CommandEvent::OpenMenu(MenuRequest {
                crew_id: builder_id,
                owner: 42,
                kind: MenuRequestKind::Construction,
            })]
        );

        let mut incapable = builder.clone();
        incapable.physical.can_construct = 0;
        let incapable_ctx = CommandRuntimeContext {
            object: &incapable,
            ..ctx.clone()
        };
        let mut incapable_state =
            ConstructState::from_request(&CommandRequest::new(CommandId::Construct));
        let incapable_result = incapable_state.step(&incapable_ctx);
        assert_eq!(incapable_result.status, CommandStatus::Failed);
        assert!(incapable_result.events.is_empty());
    }

    #[test]
    fn construct_spawns_construction_and_queues_build() {
        let builder_id = ObjectId::new(1);
        let kit_id = ObjectId::new(2);
        let construction_definition = "STRT".to_string();

        let mut builder = snapshot_with_id(builder_id.as_u64());
        builder.position = Vector2::new(10, 0);
        builder.command_direction = CommandDirection::Right;
        builder.physical.can_construct = 1;
        builder.owner = 42;
        builder.contents.push(kit_id);

        let mut kit = snapshot_with_id(kit_id.as_u64());
        kit.definition_id = CONKIT_DEFINITION.into();
        kit.collectible = true;
        kit.construction = FULL_CON;
        kit.container = Some(builder_id);
        kit.position = builder.position;
        kit.alive = false;

        let mut objects = HashMap::new();
        objects.insert(builder_id, builder.clone());
        objects.insert(kit_id, kit);

        let mut players: HashMap<i32, CommandPlayerSnapshot> = HashMap::new();
        players.insert(
            42,
            CommandPlayerSnapshot {
                status: PlayerStatus::Active,
                surrendered: false,
                wealth: 100,
                home_base_material: HashMap::new(),
                home_base_material_entries: Vec::new(),
                knowledge: vec![construction_definition.clone()],
                hostile_to: Vec::new(),
            },
        );

        let mut definitions: HashMap<DefinitionId, CommandDefinitionSnapshot> = HashMap::new();
        definitions.insert(
            construction_definition.clone(),
            CommandDefinitionSnapshot {
                value: 0,
                can_chop: false,
                chop_action: None,
                constructable: true,
                grab: 0,
                ..CommandDefinitionSnapshot::default()
            },
        );

        let ctx = CommandRuntimeContext {
            landscape: None,
            frame: 0,
            position: builder.position,
            object: objects.get(&builder_id).expect("builder present"),
            objects: &objects,
            players: &players,
            definitions: &definitions,
            structures_need_energy: false,
            base_buy_enabled: true,

            base_sell_enabled: true,
            transfer_zones: &EMPTY_TRANSFER_ZONES,
            rng: None,
        };

        let mut state = ConstructState::from_request(
            &CommandRequest::new(CommandId::Construct)
                .with_data(CommandData::Text(construction_definition.clone()))
                .with_tx(Some(10))
                .with_ty(Some(0)),
        );
        continue_construct_script(&mut state);

        let first = state.step(&ctx);
        assert_eq!(first.status, CommandStatus::Running);
        assert_eq!(first.events.len(), 1);

        match &first.events[0] {
            CommandEvent::SpawnConstruction {
                actor_id,
                definition_id,
                owner,
                position,
                kit_id: event_kit,
                command_instance_id,
            } => {
                assert_eq!(*actor_id, builder_id);
                assert_eq!(definition_id, &construction_definition);
                assert_eq!(*owner, 42);
                assert_eq!(*position, Vector2::new(10, 0));
                assert_eq!(*event_kit, kit_id);
                assert_eq!(*command_instance_id, 0);
            }
            other => panic!("unexpected event: {:?}", other),
        }

        let construction_id = ObjectId::new(3);
        let mut construction = snapshot_with_id(construction_id.as_u64());
        construction.definition_id = construction_definition.clone();
        construction.position = Vector2::new(10, 0);
        construction.owner = 42;
        construction.construction = 1;
        objects.insert(construction_id, construction);

        let mut updated_builder = builder.clone();
        updated_builder.contents.clear();
        objects.insert(builder_id, updated_builder);

        let ctx_after_spawn = CommandRuntimeContext {
            landscape: None,
            frame: 1,
            position: builder.position,
            object: objects.get(&builder_id).expect("builder present"),
            objects: &objects,
            players: &players,
            definitions: &definitions,
            structures_need_energy: false,
            base_buy_enabled: true,

            base_sell_enabled: true,
            transfer_zones: &EMPTY_TRANSFER_ZONES,
            rng: None,
        };

        let mut creation_failed_state = state.clone();
        let second = state.resume_after_spawn(&ctx_after_spawn, Some(construction_id));
        assert_eq!(second.status, CommandStatus::Completed);
        assert_eq!(second.operations.len(), 1);
        match &second.operations[0] {
            CommandOperation::PushFront(request) => {
                assert_eq!(request.id, CommandId::Build);
                assert_eq!(request.target, Some(construction_id));
                assert_eq!(request.tx, None);
                assert_eq!(request.ty, None);
                assert_eq!(request.update_interval, 0);
                assert_eq!(request.mode, CommandMode::SilentSub);
            }
            other => panic!("unexpected operation: {:?}", other),
        }

        let creation_failed = creation_failed_state.resume_after_spawn(&ctx_after_spawn, None);
        assert_eq!(creation_failed.status, CommandStatus::Completed);
        assert!(matches!(
            creation_failed.operations.as_slice(),
            [CommandOperation::PushFront(request)]
                if request.id == CommandId::Build
                    && request.target == Some(ObjectId::new(0))
        ));
        assert!(!creation_failed_state.spawn_requested);
    }

    #[test]
    fn construct_spawned_site_scan_uses_cpp_master_list_order() {
        let builder_id = ObjectId::new(1);
        let lower_id_later = ObjectId::new(2);
        let higher_id_earlier = ObjectId::new(99);
        let inactive_first = ObjectId::new(100);
        let definition_id = "STRT";
        let site = Vector2::new(10, 20);

        let mut builder = snapshot_with_id(builder_id.as_u64());
        builder.owner = 7;

        let matching_site = |id: ObjectId, master_list_order: usize| {
            let mut snapshot = snapshot_with_id(id.as_u64());
            snapshot.master_list_order = master_list_order;
            snapshot.definition_id = definition_id.into();
            snapshot.owner = builder.owner;
            snapshot.position = site;
            snapshot.construction = 1;
            snapshot
        };

        let mut inactive = matching_site(inactive_first, 0);
        inactive.status = ObjectStatus::Deleted;
        let mut objects = HashMap::from([
            (builder_id, builder.clone()),
            (lower_id_later, matching_site(lower_id_later, 2)),
            (higher_id_earlier, matching_site(higher_id_earlier, 1)),
            (inactive_first, inactive),
        ]);
        let players = HashMap::new();
        let definitions = HashMap::new();
        let state = ConstructState::from_request(
            &CommandRequest::new(CommandId::Construct)
                .with_data(CommandData::Text(definition_id.into()))
                .with_tx(Some(site.x))
                .with_ty(Some(site.y)),
        );
        let choose = |objects: &HashMap<ObjectId, CommandObjectSnapshot>| {
            let ctx = CommandRuntimeContext {
                landscape: None,
                frame: 0,
                position: builder.position,
                object: &builder,
                objects,
                players: &players,
                definitions: &definitions,
                structures_need_energy: false,
                base_buy_enabled: true,
                base_sell_enabled: true,
                transfer_zones: &EMPTY_TRANSFER_ZONES,
                rng: None,
            };
            state.find_spawned_construction(&ctx, definition_id, site)
        };

        assert_eq!(choose(&objects), Some(higher_id_earlier));

        // Mutating only the modeled master-list ranks must flip the result;
        // HashMap bucket order and ObjectId order remain unchanged.
        objects
            .get_mut(&lower_id_later)
            .expect("later site present")
            .master_list_order = 1;
        objects
            .get_mut(&higher_id_earlier)
            .expect("earlier site present")
            .master_list_order = 2;
        assert_eq!(choose(&objects), Some(lower_id_later));
    }

    #[test]
    fn construct_requests_acquire_when_missing_conkit() {
        let builder_id = ObjectId::new(5);
        let construction_definition = "STRT".to_string();

        let mut builder = snapshot_with_id(builder_id.as_u64());
        builder.position = Vector2::new(0, 0);
        builder.command_direction = CommandDirection::Right;
        builder.physical.can_construct = 1;
        builder.owner = 7;

        let mut objects = HashMap::new();
        objects.insert(builder_id, builder.clone());

        let mut players: HashMap<i32, CommandPlayerSnapshot> = HashMap::new();
        players.insert(
            7,
            CommandPlayerSnapshot {
                status: PlayerStatus::Active,
                surrendered: false,
                wealth: 100,
                home_base_material: HashMap::new(),
                home_base_material_entries: Vec::new(),
                knowledge: vec![construction_definition.clone()],
                hostile_to: Vec::new(),
            },
        );

        let mut definitions: HashMap<DefinitionId, CommandDefinitionSnapshot> = HashMap::new();
        definitions.insert(
            construction_definition.clone(),
            CommandDefinitionSnapshot {
                value: 0,
                can_chop: false,
                chop_action: None,
                constructable: true,
                grab: 0,
                ..CommandDefinitionSnapshot::default()
            },
        );

        let ctx = CommandRuntimeContext {
            landscape: None,
            frame: 0,
            position: builder.position,
            object: objects.get(&builder_id).expect("builder present"),
            objects: &objects,
            players: &players,
            definitions: &definitions,
            structures_need_energy: false,
            base_buy_enabled: true,

            base_sell_enabled: true,
            transfer_zones: &EMPTY_TRANSFER_ZONES,
            rng: None,
        };

        let mut state = ConstructState::from_request(
            &CommandRequest::new(CommandId::Construct)
                .with_data(CommandData::Text(construction_definition))
                .with_tx(Some(8))
                .with_ty(Some(2)),
        );
        continue_construct_script(&mut state);

        let result = state.step(&ctx);
        assert_eq!(result.status, CommandStatus::Running);
        assert_eq!(result.events.len(), 0);
        assert_eq!(result.operations.len(), 1);
        match &result.operations[0] {
            CommandOperation::PushFront(request) => {
                assert_eq!(request.id, CommandId::Acquire);
                assert_eq!(request.mode, CommandMode::Sub);
                assert_eq!(request.update_interval, 50);
                assert_eq!(request.retries, 5);
            }
            other => panic!("unexpected operation: {:?}", other),
        }
    }

    #[test]
    fn construct_helper_adopts_build_and_tracks_the_primary_construct() {
        let builder_id = ObjectId::new(1);
        let primary_id = ObjectId::new(2);
        let construction_id = ObjectId::new(3);
        let mut builder = snapshot_with_id(builder_id.as_u64());
        builder.physical.can_construct = 1;
        let mut primary = snapshot_with_id(primary_id.as_u64());
        primary.commands = vec![
            command_view(CommandId::Build, Some(construction_id)),
            command_view(CommandId::Construct, None),
        ];
        let mut objects =
            HashMap::from([(builder_id, builder.clone()), (primary_id, primary.clone())]);
        let players = HashMap::new();
        let definitions = HashMap::new();
        let ctx = move_to_ctx_at_frame(&builder, &objects, &players, &definitions, 0);
        let request = CommandRequest::new(CommandId::Construct)
            .with_target(Some(primary_id))
            .with_data(CommandData::Text("MISSING".into()))
            .with_tx(Some(6))
            .with_ty(Some(0));
        let mut state = ConstructState::from_request(&request);

        let adopted = state.step(&ctx);

        assert_eq!(adopted.status, CommandStatus::Completed);
        assert_eq!(adopted.operations.len(), 2);
        let CommandOperation::PushFront(build) = &adopted.operations[0] else {
            panic!("first helper command is Build");
        };
        assert_eq!(build.id, CommandId::Build);
        assert_eq!(build.target, Some(construction_id));
        assert_eq!(build.update_interval, 0);
        assert_eq!(build.mode, CommandMode::SilentSub);
        let CommandOperation::PushFront(move_to) = &adopted.operations[1] else {
            panic!("second helper command is MoveTo");
        };
        assert_eq!(move_to.id, CommandId::MoveTo);
        assert_eq!((move_to.tx, move_to.ty), (Some(6), Some(0)));
        assert_eq!(move_to.update_interval, 50);

        primary.commands = vec![command_view(CommandId::Construct, None)];
        objects.insert(primary_id, primary.clone());
        let mut wider_builder = builder.clone();
        wider_builder.move_to_range = 6;
        let wider_ctx = move_to_ctx_at_frame(&wider_builder, &objects, &players, &definitions, 1);
        let mut waiting = ConstructState::from_request(&request);
        let waited = waiting.step(&wider_ctx);
        assert_eq!(waited.status, CommandStatus::Running);
        let wait = pushed_request(&waited.operations, CommandId::Wait);
        assert_eq!(wait.update_interval, 10);
        assert_eq!(wait.mode, CommandMode::SilentSub);

        primary.commands = vec![command_view(CommandId::Build, Some(construction_id))];
        objects.insert(primary_id, primary);
        let failed_ctx = move_to_ctx_at_frame(&builder, &objects, &players, &definitions, 2);
        let mut orphaned = ConstructState::from_request(&request);
        let failed = orphaned.step(&failed_ctx);
        assert_eq!(failed.status, CommandStatus::Failed);
        assert!(matches!(
            failed.events.as_slice(),
            [CommandEvent::NativeCommandSuccess {
                object_id,
                command: CommandId::Construct,
            }] if *object_id == builder_id
        ));
        assert_eq!(
            pushed_request(&failed.operations, CommandId::Build).target,
            Some(construction_id),
            "native queues the adopted Build before noticing Construct vanished"
        );
    }

    #[test]
    fn construct_auto_site_stages_control_override_results() {
        let builder_id = ObjectId::new(10);
        let target2_id = ObjectId::new(11);
        let definition_id = "STRT".to_string();
        let mut builder = snapshot_with_id(builder_id.as_u64());
        builder.position = Vector2::new(50, 40);
        builder.physical.can_construct = 1;
        builder.owner = 7;
        let objects = HashMap::from([(builder_id, builder.clone())]);
        let players = HashMap::new();
        let definition = CommandDefinitionSnapshot {
            shape: Some(DefinitionRect::new(-10, -20, 20, 20)),
            category: CATEGORY_STRUCTURE,
            constructable: false,
            ..CommandDefinitionSnapshot::default()
        };
        let definitions = HashMap::from([(definition_id.clone(), definition)]);
        let mut landscape = crate::Landscape::flat(200, 100);
        landscape.set_world_height(400);
        let expected = landscape
            .find_con_site_spot(50, 40, 20, 20, 20, |_, _, _, _| false)
            .map(|(x, y)| Vector2::new(x, y))
            .expect("flat landscape has a construction site");
        let mut ctx = move_to_ctx_at_frame(&builder, &objects, &players, &definitions, 0);
        ctx.landscape = Some(&landscape);
        let request = CommandRequest::new(CommandId::Construct)
            .with_target2(Some(target2_id))
            .with_data(CommandData::Text(definition_id.clone()));
        let mut state = ConstructState::from_request(&request);

        let first = state.step(&ctx);

        assert_eq!(first.status, CommandStatus::Running);
        assert_eq!(state.site, Some(expected));
        assert!(matches!(
            first.events.as_slice(),
            [CommandEvent::ControlCommandConstruction {
                caller,
                target: None,
                site,
                target2: Some(event_target2),
                definition_id: event_definition,
                command_instance_id: 0,
            }] if *caller == builder_id
                && *site == expected
                && *event_target2 == target2_id
                && event_definition == &definition_id
        ));

        state.script_result = Some(AcquireScriptResult::Handled);
        let handled = state.step(&ctx);
        assert_eq!(handled.status, CommandStatus::Running);
        assert!(handled.events.is_empty());
        let repeated = state.step(&ctx);
        assert!(matches!(
            repeated.events.as_slice(),
            [CommandEvent::ControlCommandConstruction { .. }]
        ));

        state.script_result = Some(AcquireScriptResult::Complete);
        assert_eq!(state.step(&ctx).status, CommandStatus::Completed);

        let mut failed = ConstructState::from_request(&request);
        failed.site = Some(expected);
        failed.script_pending = true;
        failed.script_invoked = true;
        failed.script_result = Some(AcquireScriptResult::Failed);
        assert_eq!(failed.step(&ctx).status, CommandStatus::Failed);

        let mut continued = ConstructState::from_request(&request);
        continued.site = Some(expected);
        continue_construct_script(&mut continued);
        let fallback = continued.step(&ctx);
        assert_eq!(fallback.status, CommandStatus::Running);
        assert_eq!(
            pushed_request(&fallback.operations, CommandId::Acquire).retries,
            5,
            "late constructability does not preempt the overload or Acquire"
        );
    }

    #[test]
    fn construct_move_range_and_construction_check_match_cpp_order() {
        let builder_id = ObjectId::new(20);
        let kit_id = ObjectId::new(21);
        let blocker_id = ObjectId::new(22);
        let definition_id = "STRT".to_string();
        let mut builder = snapshot_with_id(builder_id.as_u64());
        builder.physical.can_construct = 1;
        builder.owner = 9;
        builder.contents.push(kit_id);
        let mut kit = snapshot_with_id(kit_id.as_u64());
        kit.definition_id = CONKIT_DEFINITION.into();
        kit.container = Some(builder_id);
        kit.alive = false;
        let definition = CommandDefinitionSnapshot {
            shape: Some(DefinitionRect::new(-10, -20, 20, 20)),
            category: CATEGORY_STRUCTURE,
            constructable: true,
            ..CommandDefinitionSnapshot::default()
        };
        let definitions = HashMap::from([(definition_id.clone(), definition.clone())]);
        let players = HashMap::new();
        let objects = HashMap::from([(builder_id, builder.clone()), (kit_id, kit.clone())]);
        let ctx = move_to_ctx_at_frame(&builder, &objects, &players, &definitions, 0);
        let request = CommandRequest::new(CommandId::Construct)
            .with_data(CommandData::Text(definition_id.clone()))
            .with_tx(Some(6))
            .with_ty(Some(0));
        let mut state = ConstructState::from_request(&request);
        continue_construct_script(&mut state);

        let moved = state.step(&ctx);

        let move_to = pushed_request(&moved.operations, CommandId::MoveTo);
        assert_eq!(move_to.update_interval, 50);
        assert_eq!(move_to.mode, CommandMode::SilentSub);

        let mut wider_builder = builder.clone();
        wider_builder.move_to_range = 6;
        let wider_objects =
            HashMap::from([(builder_id, wider_builder.clone()), (kit_id, kit.clone())]);
        let wider_ctx =
            move_to_ctx_at_frame(&wider_builder, &wider_objects, &players, &definitions, 1);
        let mut wider_state = ConstructState::from_request(&request);
        continue_construct_script(&mut wider_state);
        let spawned = wider_state.step(&wider_ctx);
        assert!(spawned
            .events
            .iter()
            .any(|event| matches!(event, CommandEvent::SpawnConstruction { .. })));

        let mut landscape = crate::Landscape::flat(200, 100);
        landscape.set_world_height(400);
        let site = landscape
            .find_con_site_spot(50, 40, 20, 20, 20, |_, _, _, _| false)
            .map(|(x, y)| Vector2::new(x, y))
            .expect("flat landscape has a site");
        let mut at_site_builder = builder.clone();
        at_site_builder.position = site;
        let mut blocker = snapshot_with_id(blocker_id.as_u64());
        blocker.category = CATEGORY_STRUCTURE;
        blocker.shape = DefinitionRect::new(site.x - 5, site.y - 20, 10, 20);
        let blocked_objects = HashMap::from([
            (builder_id, at_site_builder.clone()),
            (kit_id, kit),
            (blocker_id, blocker),
        ]);
        let mut blocked_ctx = move_to_ctx_at_frame(
            &at_site_builder,
            &blocked_objects,
            &players,
            &definitions,
            2,
        );
        blocked_ctx.landscape = Some(&landscape);
        let mut blocked_state = ConstructState::from_request(
            &CommandRequest::new(CommandId::Construct)
                .with_data(CommandData::Text(definition_id))
                .with_tx(Some(site.x))
                .with_ty(Some(site.y)),
        );
        continue_construct_script(&mut blocked_state);
        let blocked = blocked_state.step(&blocked_ctx);
        assert_eq!(blocked.status, CommandStatus::Failed);
        assert!(
            !blocked
                .events
                .iter()
                .any(|event| matches!(event, CommandEvent::SpawnConstruction { .. })),
            "failed checks retain the conkit"
        );
        // The rejected site check reports the C++ IDS_OBJ_NOOTHER feedback
        // naming the overlap blocker (C4Landscape.cpp:2159-2163).
        assert!(
            blocked.events.iter().any(|event| matches!(
                event,
                CommandEvent::ConstructionCheckRejected {
                    failure: ConstructionCheckFailure::Blocked(blocked_by),
                    ..
                } if *blocked_by == blocker_id
            )),
            "unexpected events: {:?}",
            blocked.events
        );
    }

    #[test]
    fn construct_overlap_checks_the_builder_and_raw_live_shape() {
        let builder_id = ObjectId::new(30);
        let blocker_id = ObjectId::new(31);
        let mut builder = snapshot_with_id(builder_id.as_u64());
        builder.position = Vector2::new(50, 50);
        builder.category = CATEGORY_STRUCTURE;
        builder.shape_top = -10;
        builder.shape_height = 10;
        builder.shape = DefinitionRect::new(45, 40, 10, 10);
        let players = HashMap::new();
        let definitions = HashMap::new();
        let mut objects = HashMap::from([(builder_id, builder.clone())]);
        let ctx = move_to_ctx_at_frame(&builder, &objects, &players, &definitions, 0);

        assert!(ConstructState::overlaps_construction_rect(
            &ctx,
            40,
            30,
            20,
            20,
            CATEGORY_STRUCTURE,
        ));

        builder.category = 0;
        objects.insert(builder_id, builder.clone());
        let mut short_blocker = snapshot_with_id(blocker_id.as_u64());
        short_blocker.position = Vector2::new(50, 55);
        short_blocker.category = CATEGORY_STRUCTURE;
        short_blocker.shape_top = 0;
        short_blocker.shape_height = 5;
        short_blocker.shape = DefinitionRect::new(45, 42, 10, 18);
        objects.insert(blocker_id, short_blocker);
        let ctx = move_to_ctx_at_frame(&builder, &objects, &players, &definitions, 1);
        assert!(
            !ConstructState::overlaps_construction_rect(&ctx, 40, 30, 20, 20, CATEGORY_STRUCTURE,),
            "the eighteen-pixel action-area expansion is not C4Object::Shape"
        );
    }

    #[test]
    fn push_to_requests_grab_when_actor_not_pushing() {
        let actor_id = ObjectId::new(420);
        let target_id = ObjectId::new(421);

        let mut actor = snapshot_with_id(actor_id.as_u64());
        actor.action_procedure = ActionProcedure::Walk;

        let mut target = snapshot_with_id(target_id.as_u64());
        target.position = Vector2::new(30, 0);
        target.ocf |= ocf::GRAB;

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

        let mut state = PushToState::from_request(
            &CommandRequest::new(CommandId::PushTo).with_target(Some(target_id)),
        )
        .expect("state created");

        let result = state.step(&ctx);
        assert_eq!(result.status, CommandStatus::Running);
        assert_eq!(result.operations.len(), 1);
        match &result.operations[0] {
            CommandOperation::PushFront(request) => {
                assert_eq!(request.id, CommandId::Grab);
                assert_eq!(request.target, Some(target_id));
                assert_eq!(request.update_interval, 40);
            }
            other => panic!("expected grab request, got {:?}", other),
        }
        let first_grab = pushed_request(&result.operations, CommandId::Grab);
        let reissued = state.step(&ctx);
        assert_eq!(
            pushed_request(&reissued.operations, CommandId::Grab),
            first_grab,
            "PushTo reissues Grab on its next execution while not pushing the target"
        );
    }

    #[test]
    fn push_to_reissues_grab_after_failed_attempt_and_retry() {
        let actor_id = ObjectId::new(422);
        let target_id = ObjectId::new(423);
        let mut actor = snapshot_with_id(actor_id.as_u64());
        actor.action_procedure = ActionProcedure::Walk;
        let mut target = snapshot_with_id(target_id.as_u64());
        target.position = Vector2::new(30, 0);
        target.ocf |= ocf::GRAB;
        let objects = HashMap::from([(actor_id, actor), (target_id, target)]);
        let players = HashMap::new();
        let definitions = HashMap::new();
        let actor = objects.get(&actor_id).expect("actor present");
        let ctx = move_to_ctx_at_frame(actor, &objects, &players, &definitions, 10);

        let mut stack = CommandStack::new();
        stack
            .push_front(
                CommandRequest::new(CommandId::PushTo)
                    .with_target(Some(target_id))
                    .with_retries(1),
            )
            .expect("PushTo queues");

        let evaluation = stack.step(&ctx).expect("PushTo evaluates");
        assert_eq!(evaluation.status, CommandStatus::Running);
        assert!(evaluation.update.is_none());
        assert!(evaluation.events.is_empty());
        assert_eq!(stack.command_names(), vec!["PushTo"]);

        stack.step(&ctx).expect("PushTo requests Grab");
        assert_eq!(stack.command_names(), vec!["Grab", "PushTo"]);

        let attempt = stack.step(&ctx).expect("Grab attempts target");
        assert_eq!(
            attempt.events,
            vec![CommandEvent::AttemptGrab {
                actor_id,
                target_id,
            }]
        );
        assert_eq!(stack.resolve_grab_attempt(target_id, true), Some(true));
        stack.clear_finished_fronts();

        stack.step(&ctx).expect("PushTo schedules its retry");
        assert_eq!(stack.command_names(), vec!["Retry", "PushTo"]);
        assert!(stack.complete_front_if(CommandId::Retry));

        stack.step(&ctx).expect("PushTo reissues Grab");
        assert_eq!(stack.command_names(), vec!["Grab", "PushTo"]);
    }

    #[test]
    fn push_to_requests_enter_when_destination_requires_container() {
        let actor_id = ObjectId::new(430);
        let target_id = ObjectId::new(431);
        let destination_id = ObjectId::new(432);

        let mut actor = snapshot_with_id(actor_id.as_u64());
        actor.action_procedure = ActionProcedure::Push;
        actor.action_target = Some(target_id);

        let mut target = snapshot_with_id(target_id.as_u64());
        target.position = Vector2::new(60, 0);
        target.ocf |= ocf::GRAB;

        let destination = snapshot_with_id(destination_id.as_u64());

        let mut objects = HashMap::new();
        objects.insert(actor.id, actor.clone());
        objects.insert(target.id, target);
        objects.insert(destination.id, destination);

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

        let mut state = PushToState::from_request(
            &CommandRequest::new(CommandId::PushTo)
                .with_target(Some(target_id))
                .with_target2(Some(destination_id)),
        )
        .expect("state created");

        let result = state.step(&ctx);
        assert_eq!(result.status, CommandStatus::Running);
        assert_eq!(result.operations.len(), 1);
        match &result.operations[0] {
            CommandOperation::PushFront(request) => {
                assert_eq!(request.id, CommandId::Enter);
                assert_eq!(request.target, Some(destination_id));
                assert_eq!(request.update_interval, 40);
            }
            other => panic!("expected enter request, got {:?}", other),
        }
    }

    #[test]
    fn push_to_requests_move_to_target_position() {
        let actor_id = ObjectId::new(440);
        let target_id = ObjectId::new(441);

        let mut actor = snapshot_with_id(actor_id.as_u64());
        actor.action_procedure = ActionProcedure::Push;
        actor.action_target = Some(target_id);

        let mut target = snapshot_with_id(target_id.as_u64());
        target.position = Vector2::new(80, 0);
        target.ocf |= ocf::GRAB;

        let mut objects = HashMap::new();
        objects.insert(actor.id, actor.clone());
        objects.insert(target.id, target);

        let players: HashMap<i32, CommandPlayerSnapshot> = HashMap::new();
        let definitions: HashMap<DefinitionId, CommandDefinitionSnapshot> = HashMap::new();
        let ctx = CommandRuntimeContext {
            landscape: None,
            frame: 8,
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

        let mut state = PushToState::from_request(
            &CommandRequest::new(CommandId::PushTo)
                .with_target(Some(target_id))
                .with_tx(Some(100))
                .with_ty(Some(0)),
        )
        .expect("state created");

        let result = state.step(&ctx);
        assert_eq!(result.status, CommandStatus::Running);
        assert_eq!(result.operations.len(), 1);
        match &result.operations[0] {
            CommandOperation::PushFront(request) => {
                assert_eq!(request.id, CommandId::MoveTo);
                assert_eq!(request.tx, Some(100));
                assert_eq!(request.ty, Some(0));
                assert_eq!(request.update_interval, 40);
            }
            other => panic!("expected moveto request, got {:?}", other),
        }
        let first_move = pushed_request(&result.operations, CommandId::MoveTo);
        let reissued = state.step(&ctx);
        assert_eq!(
            pushed_request(&reissued.operations, CommandId::MoveTo),
            first_move,
            "PushTo reissues MoveTo on its next execution"
        );
    }

    #[test]
    fn push_to_init_evaluation_grounds_once_with_free_move_and_raw_shape_height() {
        let landscape = crate::Landscape::flat(300, 110);
        let actor_id = ObjectId::new(445);
        let target_id = ObjectId::new(446);

        let mut actor = snapshot_with_id(actor_id.as_u64());
        actor.action_procedure = ActionProcedure::Push;
        actor.action_target = Some(target_id);
        actor.shape_height = 20;
        let mut target = snapshot_with_id(target_id.as_u64());
        target.position = Vector2::new(80, 0);
        target.ocf |= ocf::GRAB;
        let mut objects = HashMap::from([(actor_id, actor.clone()), (target_id, target)]);
        let players = HashMap::new();
        let definitions = HashMap::new();

        let mut stack = CommandStack::new();
        stack
            .push_front(
                CommandRequest::new(CommandId::PushTo)
                    .with_target(Some(target_id))
                    .with_tx(Some(100))
                    .with_ty(Some(50)),
            )
            .expect("PushTo queues");
        let mut ctx = move_to_ctx_at_frame(&actor, &objects, &players, &definitions, 1);
        ctx.landscape = Some(&landscape);
        let evaluation = stack.step(&ctx).expect("PushTo evaluates");
        assert_eq!(evaluation.status, CommandStatus::Running);
        assert!(evaluation.update.is_none());
        assert!(evaluation.events.is_empty());
        assert_eq!(
            (stack.command_views()[0].tx, stack.command_views()[0].ty),
            (Some(100), Some(99)),
            "walking PushTo drops to the surface and lifts raw Shape.Hgt/2"
        );

        // Changing the live shape after InitEvaluation must not re-ground
        // the stored destination on the handler Execute.
        objects
            .get_mut(&actor_id)
            .expect("actor present")
            .shape_height = 40;
        let taller_actor = objects.get(&actor_id).expect("actor present");
        let mut ctx = move_to_ctx_at_frame(taller_actor, &objects, &players, &definitions, 2);
        ctx.landscape = Some(&landscape);
        let handler = stack.step(&ctx).expect("PushTo handler executes");
        assert_eq!(handler.status, CommandStatus::Running);
        let snapshot = stack.snapshot();
        assert_eq!(snapshot.command_names(), vec!["MoveTo", "PushTo"]);
        let move_to = snapshot.commands[0]
            .request
            .as_ref()
            .expect("MoveTo request retained");
        assert_eq!((move_to.tx, move_to.ty), (Some(100), Some(99)));

        // FreeMoveTo accepts the mid-air y unchanged for a CanFly actor.
        let mut flyer = actor;
        flyer.physical.can_fly = 1;
        let flyer_objects = HashMap::from([
            (actor_id, flyer.clone()),
            (
                target_id,
                objects.get(&target_id).expect("target present").clone(),
            ),
        ]);
        let mut free_stack = CommandStack::new();
        free_stack
            .push_front(
                CommandRequest::new(CommandId::PushTo)
                    .with_target(Some(target_id))
                    .with_tx(Some(100))
                    .with_ty(Some(50)),
            )
            .expect("PushTo queues");
        let mut ctx = move_to_ctx_at_frame(&flyer, &flyer_objects, &players, &definitions, 1);
        ctx.landscape = Some(&landscape);
        let _ = free_stack.step(&ctx).expect("free PushTo evaluates");
        assert_eq!(
            (
                free_stack.command_views()[0].tx,
                free_stack.command_views()[0].ty
            ),
            (Some(100), Some(50))
        );
    }

    #[test]
    fn push_to_completes_with_wait_and_ungrab_when_in_position() {
        let actor_id = ObjectId::new(450);
        let target_id = ObjectId::new(451);

        let mut actor = snapshot_with_id(actor_id.as_u64());
        actor.command_direction = CommandDirection::Right;
        actor.action_procedure = ActionProcedure::Dig;

        let mut target = snapshot_with_id(target_id.as_u64());
        target.position = Vector2::new(5, -5);
        target.ocf |= ocf::GRAB;

        let mut objects = HashMap::new();
        objects.insert(actor.id, actor.clone());
        objects.insert(target.id, target);

        let players: HashMap<i32, CommandPlayerSnapshot> = HashMap::new();
        let definitions: HashMap<DefinitionId, CommandDefinitionSnapshot> = HashMap::new();
        let ctx = CommandRuntimeContext {
            landscape: None,
            frame: 12,
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

        let mut stack = CommandStack::new();
        stack
            .push_front(CommandRequest::new(CommandId::PushTo).with_target(Some(target_id)))
            .expect("PushTo queues");
        let evaluation = stack.step(&ctx).expect("PushTo evaluates");
        assert_eq!(evaluation.status, CommandStatus::Running);
        assert!(evaluation.update.is_none());
        assert!(evaluation.events.is_empty());
        let result = stack.step(&ctx).expect("PushTo executes");
        assert_eq!(result.status, CommandStatus::Completed);
        let update = result.update.expect("push_to should stop actor");
        assert_eq!(update.command_direction, Some(CommandDirection::Stop));
        assert!(
            update.action.is_none(),
            "position fulfillment precedes PushTo's work-action stop"
        );
        let snapshot = stack.snapshot();
        assert_eq!(snapshot.command_names(), vec!["Wait", "UnGrab", "PushTo"]);
        assert_eq!(snapshot.commands[0].update_interval, Some(10));
        assert_eq!(snapshot.commands[0].mode, CommandMode::SilentSub);
        assert_eq!(snapshot.commands[1].update_interval, Some(0));
        assert_eq!(snapshot.commands[1].mode, CommandMode::SilentSub);
        assert_eq!(
            snapshot.commands[2].finished,
            Some(CommandStatus::Completed)
        );
    }

    #[test]
    fn ungrab_defers_callbackful_object_com_ungrab_to_the_live_engine() {
        let actor_id = ObjectId::new(370);

        let mut actor = snapshot_with_id(actor_id.as_u64());
        actor.action_procedure = ActionProcedure::Push;
        actor.command_direction = CommandDirection::Left;

        let mut objects = HashMap::new();
        objects.insert(actor.id, actor.clone());

        let players: HashMap<i32, CommandPlayerSnapshot> = HashMap::new();
        let definitions: HashMap<DefinitionId, CommandDefinitionSnapshot> = HashMap::new();
        let ctx = CommandRuntimeContext {
            landscape: None,
            frame: 4,
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

        let mut state = UnGrabState::from_request(&CommandRequest::new(CommandId::UnGrab));

        let result = state.step(&ctx);
        assert_eq!(result.status, CommandStatus::Running);
        assert!(result.update.is_none());
        assert_eq!(
            result.events,
            [CommandEvent::ObjectComUnGrabCommand {
                actor_id,
                command_instance_id: 0,
            }]
        );
    }

    #[test]
    fn ungrab_still_runs_live_helper_and_trailing_stop_when_not_pushing() {
        let actor_id = ObjectId::new(380);

        let mut actor = snapshot_with_id(actor_id.as_u64());
        actor.action_procedure = ActionProcedure::Walk;
        actor.command_direction = CommandDirection::Stop;

        let mut objects = HashMap::new();
        objects.insert(actor.id, actor.clone());

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

        let mut state = UnGrabState::from_request(&CommandRequest::new(CommandId::UnGrab));

        let result = state.step(&ctx);
        assert_eq!(result.status, CommandStatus::Running);
        assert!(result.update.is_none());
        assert!(result.operations.is_empty());
        assert_eq!(
            result.events,
            [CommandEvent::ObjectComUnGrabCommand {
                actor_id,
                command_instance_id: 0,
            }]
        );
    }

    #[test]
    fn jump_command_defers_targeted_jump_to_the_live_engine() {
        let actor_id = ObjectId::new(400);

        let mut actor = snapshot_with_id(actor_id.as_u64());
        actor.action_procedure = ActionProcedure::Walk;

        let mut objects = HashMap::new();
        objects.insert(actor.id, actor.clone());

        let players: HashMap<i32, CommandPlayerSnapshot> = HashMap::new();
        let definitions: HashMap<DefinitionId, CommandDefinitionSnapshot> = HashMap::new();
        let ctx = CommandRuntimeContext {
            landscape: None,
            frame: 6,
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

        let mut state = JumpState::from_request(
            &CommandRequest::new(CommandId::Jump).with_tx(Some(actor.position.x + 10)),
        );

        let result = state.step(&ctx);
        // C4Command::Jump calls ObjectComJump before Finish(true), so the
        // live event must run while the command is still active
        // (C4Command.cpp:1056-1067).
        assert_eq!(result.status, CommandStatus::Running);
        assert!(result.update.is_none());
        assert_eq!(
            result.events,
            vec![CommandEvent::ObjectComJump {
                object_id: actor_id,
                tx: actor.position.x + 10,
            }]
        );
    }

    #[test]
    fn jump_launches_with_con_scaled_walk_and_jump_physicals() {
        // ObjectComJump (C4ObjectCom.cpp:284-296): TXDir = ±ValByPhysical(280,
        // Walk)*Con/FullCon, ydir = -ValByPhysical(1000, Jump)*Con/FullCon,
        // applied with the Jump action (ObjectActionJump, :48-61).
        let actor_id = ObjectId::new(402);

        let mut actor = snapshot_with_id(actor_id.as_u64());
        actor.action_procedure = ActionProcedure::Walk;
        actor.construction = FULL_CON;
        actor.physical = PhysicalInfo {
            walk: 35_000,
            jump: 40_000,
            ..PhysicalInfo::default()
        };

        let mut objects = HashMap::new();
        objects.insert(actor.id, actor.clone());

        let players: HashMap<i32, CommandPlayerSnapshot> = HashMap::new();
        let definitions: HashMap<DefinitionId, CommandDefinitionSnapshot> = HashMap::new();
        let ctx = CommandRuntimeContext {
            landscape: None,
            frame: 6,
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

        let velocity = object_com_jump_launch(
            ctx.object.construction,
            ctx.object.physical,
            ctx.object.command_direction,
            Direction::Right,
        );
        // Full Con: TXDir = +ValByPhysical(280, 35000) = raw 64225,
        // ydir = -ValByPhysical(1000, 40000) = raw -262144.
        assert_eq!(velocity.x.val(), 64225);
        assert_eq!(velocity.y.val(), -262144);

        // Half Con scales both (C4ObjectCom.cpp:287-288).
        let mut small = actor.clone();
        small.construction = FULL_CON / 2;
        let mut objects = HashMap::new();
        objects.insert(small.id, small.clone());
        let ctx = CommandRuntimeContext {
            landscape: None,
            frame: 6,
            position: small.position,
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
        let velocity = object_com_jump_launch(
            ctx.object.construction,
            ctx.object.physical,
            ctx.object.command_direction,
            Direction::Right,
        );
        assert_eq!(velocity.x.val(), 32112);
        assert_eq!(velocity.y.val(), -131072);
    }

    #[test]
    fn jump_with_multidirection_facing_has_zero_horizontal_launch() {
        // ObjectComJump initializes TXDir to zero and only changes it for a
        // left/right ComDir or exact DIR_Left/DIR_Right facing
        // (C4ObjectCom.cpp:284-296).
        let actor_id = ObjectId::new(403);
        let mut actor = snapshot_with_id(actor_id.as_u64());
        actor.action_procedure = ActionProcedure::Walk;
        actor.construction = FULL_CON;
        actor.direction = Direction::from_raw(8);
        actor.command_direction = CommandDirection::Stop;
        actor.physical = PhysicalInfo {
            walk: 35_000,
            jump: 40_000,
            ..PhysicalInfo::default()
        };

        let objects = HashMap::from([(actor.id, actor.clone())]);
        let players = HashMap::new();
        let definitions = HashMap::new();
        let ctx = CommandRuntimeContext {
            landscape: None,
            frame: 6,
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

        let velocity = object_com_jump_launch(
            ctx.object.construction,
            ctx.object.physical,
            ctx.object.command_direction,
            ctx.object.direction,
        );
        assert_eq!(velocity.x, crate::C4Fixed::ZERO);
        assert!(velocity.y < crate::C4Fixed::ZERO);
    }

    #[test]
    fn jump_with_zero_physicals_still_overwrites_both_velocities() {
        // ObjectComJump always passes its calculated TXDir/iPhysicalJump to
        // ObjectActionJump (C4ObjectCom.cpp:284-307), which unconditionally
        // assigns both xdir and ydir (C4ObjectCom.cpp:48-61). Zero physicals
        // therefore stop any pre-existing motion rather than preserving it.
        let actor_id = ObjectId::new(404);
        let mut actor = snapshot_with_id(actor_id.as_u64());
        actor.action_procedure = ActionProcedure::Walk;
        actor.construction = FULL_CON;
        actor.fixed_velocity = FixedVec2::new(crate::itofix(3), crate::itofix(-4));
        actor.physical = PhysicalInfo::default();

        let objects = HashMap::from([(actor.id, actor.clone())]);
        let players = HashMap::new();
        let definitions = HashMap::new();
        let ctx = CommandRuntimeContext {
            landscape: None,
            frame: 6,
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

        let velocity = object_com_jump_launch(
            ctx.object.construction,
            ctx.object.physical,
            ctx.object.command_direction,
            ctx.object.direction,
        );
        assert_eq!(
            velocity,
            FixedVec2::new(crate::C4Fixed::ZERO, crate::C4Fixed::ZERO)
        );
    }

    #[test]
    fn jump_command_defers_the_walk_gate_to_live_object_com_jump() {
        let actor_id = ObjectId::new(401);

        let mut actor = snapshot_with_id(actor_id.as_u64());
        actor.action_procedure = ActionProcedure::Hang;

        let mut objects = HashMap::new();
        objects.insert(actor.id, actor.clone());

        let players: HashMap<i32, CommandPlayerSnapshot> = HashMap::new();
        let definitions: HashMap<DefinitionId, CommandDefinitionSnapshot> = HashMap::new();
        let ctx = CommandRuntimeContext {
            landscape: None,
            frame: 7,
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

        let mut state = JumpState::from_request(
            &CommandRequest::new(CommandId::Jump).with_tx(Some(actor.position.x - 15)),
        );

        let result = state.step(&ctx);
        assert_eq!(result.status, CommandStatus::Running);
        assert!(result.update.is_none());
        assert_eq!(
            result.events,
            vec![CommandEvent::ObjectComJump {
                object_id: actor_id,
                tx: actor.position.x - 15,
            }]
        );
    }

    #[test]
    fn throw_requests_get_for_the_exact_missing_item() {
        let actor_id = ObjectId::new(410);
        let target_id = ObjectId::new(420);

        let mut actor = snapshot_with_id(actor_id.as_u64());
        actor.contents.clear();
        actor.action_procedure = ActionProcedure::Push;

        let mut item = snapshot_with_id(target_id.as_u64());
        item.definition_id = "STON".into();
        item.collectible = true;
        item.construction = FULL_CON;

        let mut objects = HashMap::new();
        objects.insert(actor.id, actor.clone());
        objects.insert(item.id, item);

        let players: HashMap<i32, CommandPlayerSnapshot> = HashMap::new();
        let definitions: HashMap<DefinitionId, CommandDefinitionSnapshot> = HashMap::new();
        let ctx = CommandRuntimeContext {
            landscape: None,
            frame: 32,
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

        let mut state = ThrowState::from_request(
            &CommandRequest::new(CommandId::Throw)
                .with_target(Some(target_id))
                .with_tx(Some(100))
                .with_ty(Some(70))
                .with_update_interval(1),
        )
        .expect("state created");

        let result = state.step(&ctx);
        assert_eq!(result.status, CommandStatus::Running);
        assert!(result.update.is_none());
        let expected = CommandRequest::new(CommandId::Get)
            .with_target(Some(target_id))
            .with_update_interval(40)
            .with_mode(CommandMode::SilentSub);
        assert_eq!(
            result.operations,
            vec![CommandOperation::PushFront(expected.clone())],
            "a specific Target is fetched by object identity, never by definition"
        );
        let reissued = state.step(&ctx);
        assert_eq!(
            reissued.operations,
            vec![CommandOperation::PushFront(expected)],
            "Throw reissues the same exact-object Get on its next execution"
        );
    }

    #[test]
    fn throw_coordinate_sentinel_treats_each_missing_field_as_zero() {
        let x_only =
            ThrowState::from_request(&CommandRequest::new(CommandId::Throw).with_tx(Some(17)))
                .expect("x-only Throw");
        assert_eq!(x_only.throw_position(), Some(Vector2::new(17, 0)));

        let y_only =
            ThrowState::from_request(&CommandRequest::new(CommandId::Throw).with_ty(Some(-9)))
                .expect("y-only Throw");
        assert_eq!(y_only.throw_position(), Some(Vector2::new(0, -9)));

        let zero = ThrowState::from_request(
            &CommandRequest::new(CommandId::Throw)
                .with_tx(Some(0))
                .with_ty(Some(0)),
        )
        .expect("zero Throw");
        assert_eq!(zero.throw_position(), None);
    }

    #[test]
    fn throw_moves_to_the_computed_ballistic_position() {
        let actor_id = ObjectId::new(430);
        let target_id = ObjectId::new(440);

        let mut actor = snapshot_with_id(actor_id.as_u64());
        actor.position = Vector2::new(100, 99);
        actor.action_procedure = ActionProcedure::Walk;
        actor.physical.throw = 50_000;
        actor.contents = vec![target_id];

        let mut item = snapshot_with_id(target_id.as_u64());
        item.definition_id = "STON".into();
        item.collectible = true;
        item.construction = FULL_CON;
        item.container = Some(actor_id);

        let mut objects = HashMap::new();
        objects.insert(actor.id, actor.clone());
        objects.insert(item.id, item);

        let mut landscape = crate::Landscape::flat(200, 100);
        landscape.set_world_height(150);
        let gravity = math::fixed100(20);
        let raw_target = Vector2::new(100, 70);
        let throw_force = math::val_by_physical(400, actor.physical.throw);
        assert_eq!(
            landscape.find_throwing_position(
                raw_target,
                FixedVec2::new(throw_force, -throw_force),
                actor.shape_height,
                gravity,
            ),
            Some(Vector2::new(94, 99)),
            "the preferred +X launch searches left from the target"
        );

        let players: HashMap<i32, CommandPlayerSnapshot> = HashMap::new();
        let definitions: HashMap<DefinitionId, CommandDefinitionSnapshot> = HashMap::new();
        let ctx = CommandRuntimeContext {
            landscape: Some(&landscape),
            frame: 48,
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

        let mut state = ThrowState::from_request(
            &CommandRequest::new(CommandId::Throw)
                .with_target(Some(target_id))
                .with_tx(Some(raw_target.x))
                .with_ty(Some(raw_target.y))
                .with_update_interval(1),
        )
        .expect("state created");

        let result = state.step_with_gravity(&ctx, gravity);
        assert_eq!(result.status, CommandStatus::Running);
        assert_eq!(result.operations.len(), 1);
        match &result.operations[0] {
            CommandOperation::PushFront(request) => {
                assert_eq!(request.id, CommandId::MoveTo);
                assert_eq!((request.tx, request.ty), (Some(94), Some(99)));
                assert_eq!(request.update_interval, 20);
                assert_eq!(request.mode, CommandMode::SilentSub);
                assert_ne!(
                    (request.tx, request.ty),
                    (Some(raw_target.x), Some(raw_target.y)),
                    "MoveTo must use FindThrowingPosition, not raw Throw coordinates"
                );
            }
            other => panic!("unexpected operation: {:?}", other),
        }
        let first_move = pushed_request(&result.operations, CommandId::MoveTo);
        let reissued = state.step_with_gravity(&ctx, gravity);
        assert_eq!(
            pushed_request(&reissued.operations, CommandId::MoveTo),
            first_move,
            "Throw reissues MoveTo on its next execution"
        );
    }

    #[test]
    fn targeted_throw_fails_when_both_ballistic_searches_hit_walls() {
        let actor_id = ObjectId::new(441);
        let item_id = ObjectId::new(442);
        let raw_target = Vector2::new(100, 70);

        let mut actor = snapshot_with_id(actor_id.as_u64());
        actor.position = Vector2::new(100, 99);
        actor.action_procedure = ActionProcedure::Walk;
        actor.physical.throw = 50_000;
        actor.contents = vec![item_id];

        let mut item = snapshot_with_id(item_id.as_u64());
        item.container = Some(actor_id);

        // Full-height two-column walls catch the exact two-pixel trajectory
        // samples in each direction. The adjacent wall column then makes
        // SemiAboveSolid abort that directional surface search.
        let mut surface = vec![100; 200];
        for x in [98_usize, 99, 101, 102] {
            surface[x] = 0;
        }
        let mut landscape = crate::Landscape::new(200, surface).expect("walled landscape");
        landscape.set_world_height(150);
        let gravity = math::fixed100(20);
        let throw_force = math::val_by_physical(400, actor.physical.throw);
        for direction in [-1, 1] {
            assert_eq!(
                landscape.find_throwing_position(
                    raw_target,
                    FixedVec2::new(throw_force * direction, -throw_force),
                    actor.shape_height,
                    gravity,
                ),
                None
            );
        }

        let objects = HashMap::from([(actor_id, actor.clone()), (item_id, item)]);
        let players = HashMap::new();
        let definitions = HashMap::new();
        let ctx = CommandRuntimeContext {
            landscape: Some(&landscape),
            frame: 49,
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
        let mut state = ThrowState::from_request(
            &CommandRequest::new(CommandId::Throw)
                .with_target(Some(item_id))
                .with_tx(Some(raw_target.x))
                .with_ty(Some(raw_target.y)),
        )
        .expect("Throw state");

        let result = state.step_with_gravity(&ctx, gravity);
        assert_eq!(result.status, CommandStatus::Failed);
        assert!(result.update.is_none());
        assert!(result.operations.is_empty());
        assert!(result.events.is_empty());
    }

    #[test]
    fn throw_sets_throw_action_when_in_range() {
        // C4Command::Throw faces/stops at the targeted position, then
        // ObjectActionThrow performs the action-gated exit atomically
        // (C4Command.cpp:950-957; C4ObjectCom.cpp:120-137).
        let actor_id = ObjectId::new(450);
        let target_id = ObjectId::new(460);

        let mut actor = snapshot_with_id(actor_id.as_u64());
        // FindThrowingPosition returns (94,99); x=99 is the inclusive
        // default MoveToRange boundary, while the raw target is (100,70).
        actor.position = Vector2::new(99, 99);
        actor.shape_top = -10;
        actor.direction = Direction::Left;
        actor.action_procedure = ActionProcedure::Walk;
        actor.physical.throw = 50_000;
        actor.contents = vec![target_id];
        actor.container = Some(ObjectId::new(999));

        let mut item = snapshot_with_id(target_id.as_u64());
        item.definition_id = "STON".into();
        item.collectible = true;
        item.construction = FULL_CON;
        item.container = Some(actor_id);

        let mut objects = HashMap::new();
        objects.insert(actor.id, actor.clone());
        objects.insert(item.id, item);

        let mut landscape = crate::Landscape::flat(200, 100);
        landscape.set_world_height(150);
        let gravity = math::fixed100(20);
        let players: HashMap<i32, CommandPlayerSnapshot> = HashMap::new();
        let definitions: HashMap<DefinitionId, CommandDefinitionSnapshot> = HashMap::new();
        let rng = std::cell::RefCell::new(crate::LcgRng::seed_from_u64(7));
        let expected_rng = rng.borrow().clone();
        let ctx = CommandRuntimeContext {
            landscape: Some(&landscape),
            frame: 52,
            position: actor.position,
            object: objects.get(&actor_id).expect("actor present"),
            objects: &objects,
            players: &players,
            definitions: &definitions,
            structures_need_energy: false,
            base_buy_enabled: true,

            base_sell_enabled: true,
            transfer_zones: &EMPTY_TRANSFER_ZONES,
            rng: Some(&rng),
        };

        let mut state = ThrowState::from_request(
            &CommandRequest::new(CommandId::Throw)
                .with_target(Some(target_id))
                .with_tx(Some(100))
                .with_ty(Some(70))
                .with_update_interval(1),
        )
        .expect("state created");

        let result = state.step_with_gravity(&ctx, gravity);
        assert_eq!(result.status, CommandStatus::Running);
        assert!(result.operations.is_empty());
        assert_eq!(result.update, None, "SetDir must run before ComDir=Stop");
        assert_eq!(result.events.len(), 1);
        assert_eq!(
            result.events[0],
            CommandEvent::ObjectComSetDirThrow {
                object_id: actor_id,
                direction: Direction::Right,
                command_instance_id: 0,
            },
            "targeted Throw exposes SetDir's callback boundary"
        );

        let result = state.resume_after_prelude(&ctx, gravity);
        assert_eq!(result.status, CommandStatus::Running);
        assert!(result.operations.is_empty());
        let update = result
            .update
            .expect("post-SetDir Throw stops command motion");
        assert_eq!(update.command_direction, Some(CommandDirection::Stop));
        assert!(update.action.is_none(), "the engine event gates SetAction");
        assert_eq!(update.direction, None);
        assert_eq!(result.events.len(), 1);
        let CommandEvent::ThrowObject {
            actor_id: event_actor,
            object_id,
            complete_command_on_success,
            command_instance_id,
        } = &result.events[0]
        else {
            panic!("targeted Throw must emit its atomic throw event even while contained")
        };
        assert_eq!(*event_actor, actor_id);
        assert_eq!(*object_id, target_id);
        assert!(*complete_command_on_success);
        assert_eq!(*command_instance_id, 0);
        assert_eq!(*rng.borrow(), expected_rng, "the event owns the RNG draw");
    }

    #[test]
    fn untargeted_throw_runs_inline_put_take_without_ungrabbing() {
        // C4Command::Throw only ungrabs for a targeted-coordinate throw.
        // With no coordinates, DFA_PUSH instead calls ObjectComPutTake on
        // Action.Target and immediately finishes (C4Command.cpp:910-984,
        // especially :927-934 and :973-979).
        let actor_id = ObjectId::new(470);
        let push_target_id = ObjectId::new(471);
        let item_id = ObjectId::new(472);

        let mut actor = snapshot_with_id(actor_id.as_u64());
        actor.action_procedure = ActionProcedure::Push;
        actor.action_target = Some(push_target_id);
        actor.contents = vec![item_id];

        let mut item = snapshot_with_id(item_id.as_u64());
        item.definition_id = "STON".into();
        item.collectible = true;
        item.construction = FULL_CON;
        item.container = Some(actor_id);

        let mut push_target = snapshot_with_id(push_target_id.as_u64());
        push_target.ocf = ocf::GRAB;

        let mut objects = HashMap::new();
        objects.insert(actor.id, actor.clone());
        objects.insert(item.id, item);
        objects.insert(push_target.id, push_target);

        let players: HashMap<i32, CommandPlayerSnapshot> = HashMap::new();
        let definitions: HashMap<DefinitionId, CommandDefinitionSnapshot> = HashMap::new();
        let ctx = CommandRuntimeContext {
            landscape: None,
            frame: 60,
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

        let mut state = ThrowState::from_request(
            &CommandRequest::new(CommandId::Throw)
                .with_target(Some(item_id))
                .with_update_interval(1),
        )
        .expect("state created");

        let result = state.step(&ctx);
        assert_eq!(result.status, CommandStatus::Running);
        assert!(result.operations.is_empty());
        assert_eq!(
            result.events,
            vec![CommandEvent::ObjectComPutTake {
                actor_id,
                target_id: push_target_id,
                requested_item: Some(item_id),
                command: CommandId::Throw,
                command_instance_id: 0,
            }]
        );
    }

    #[test]
    fn home_completes_when_container_base_matches_despite_foreign_owner() {
        let builder_id = ObjectId::new(510);
        let base_id = ObjectId::new(520);

        let mut builder = snapshot_with_id(builder_id.as_u64());
        builder.owner = 7;
        builder.container = Some(base_id);
        builder.command_direction = CommandDirection::Right;

        let mut base = snapshot_with_id(base_id.as_u64());
        base.owner = 99;
        base.base = 7;

        let mut objects = HashMap::new();
        objects.insert(builder.id, builder.clone());
        objects.insert(base.id, base);

        let players: HashMap<i32, CommandPlayerSnapshot> = HashMap::new();
        let definitions: HashMap<DefinitionId, CommandDefinitionSnapshot> = HashMap::new();
        let ctx = CommandRuntimeContext {
            landscape: None,
            frame: 0,
            position: builder.position,
            object: objects.get(&builder_id).expect("builder present"),
            objects: &objects,
            players: &players,
            definitions: &definitions,
            structures_need_energy: false,
            base_buy_enabled: true,

            base_sell_enabled: true,
            transfer_zones: &EMPTY_TRANSFER_ZONES,
            rng: None,
        };

        let mut state =
            HomeState::from_request(&CommandRequest::new(CommandId::Home)).expect("state created");

        let result = state.step(&ctx);
        assert_eq!(result.status, CommandStatus::Completed);
        assert!(result.update.is_none());
        assert!(result.operations.is_empty());
        assert!(result.events.is_empty());
    }

    #[test]
    fn home_requests_enter_when_not_in_base() {
        let builder_id = ObjectId::new(530);
        let base_id = ObjectId::new(540);

        let mut builder = snapshot_with_id(builder_id.as_u64());
        builder.owner = 11;
        builder.position = Vector2::new(0, 0);
        builder.command_direction = CommandDirection::Right;

        let mut base = snapshot_with_id(base_id.as_u64());
        base.owner = 99;
        base.base = 11;
        base.alive = false;
        base.position = Vector2::new(100, 0);

        let mut objects = HashMap::new();
        objects.insert(builder.id, builder.clone());
        objects.insert(base.id, base);

        let players: HashMap<i32, CommandPlayerSnapshot> = HashMap::new();
        let definitions: HashMap<DefinitionId, CommandDefinitionSnapshot> = HashMap::new();
        let ctx = CommandRuntimeContext {
            landscape: None,
            frame: 0,
            position: builder.position,
            object: objects.get(&builder_id).expect("builder present"),
            objects: &objects,
            players: &players,
            definitions: &definitions,
            structures_need_energy: false,
            base_buy_enabled: true,

            base_sell_enabled: true,
            transfer_zones: &EMPTY_TRANSFER_ZONES,
            rng: None,
        };

        let mut state =
            HomeState::from_request(&CommandRequest::new(CommandId::Home)).expect("state created");

        let result = state.step(&ctx);
        assert_eq!(result.status, CommandStatus::Running);
        assert!(result.update.is_none());
        assert_eq!(result.operations.len(), 1);
        match &result.operations[0] {
            CommandOperation::PushFront(request) => {
                assert_eq!(request.id, CommandId::Enter);
                assert_eq!(request.target, Some(base_id));
                assert_eq!(request.update_interval, 0);
                assert_eq!(request.mode, CommandMode::SilentSub);
            }
            other => panic!("unexpected operation: {:?}", other),
        }
        let repeated = state.step(&ctx);
        assert_eq!(repeated.status, CommandStatus::Running);
        assert_eq!(
            repeated.operations, result.operations,
            "Home has no invented same-frame Enter cooldown"
        );
    }

