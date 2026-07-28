// Contiguous slice 7 of 7 of the `command/tests` battery, spliced by
// `include!` from the parent module so every test id is unchanged.

    #[test]
    fn command_request_defaults_to_cpp_add_command_silent_sub_mode() {
        // C4Object::AddCommand defaults iBaseMode to zero, which is
        // C4CMD_Mode_SilentSub (C4Object.h:221-225; C4Command.h:62).
        assert_eq!(
            CommandRequest::new(CommandId::Wait).mode,
            CommandMode::SilentSub
        );
    }

    #[test]
    fn grab_lost_clears_only_to_a_push_to_with_a_predecessor() {
        let target = ObjectId::new(7);
        let request = |id| match id {
            CommandId::PushTo => CommandRequest::new(CommandId::PushTo).with_target(Some(target)),
            other => CommandRequest::new(other),
        };

        let mut stack = CommandStack::new();
        for id in [CommandId::MoveTo, CommandId::PushTo, CommandId::Wait] {
            stack.push_back(request(id)).expect("command queues");
        }
        stack.clear_to_first_push_to();
        assert_eq!(stack.command_names(), vec!["PushTo", "Wait"]);

        // C++ tests pCom->Next, so a PushTo already at the head is not a
        // match. A later PushTo can still become the preserved successor.
        let mut duplicate = CommandStack::new();
        for id in [
            CommandId::PushTo,
            CommandId::MoveTo,
            CommandId::PushTo,
            CommandId::Wait,
        ] {
            duplicate.push_back(request(id)).expect("command queues");
        }
        duplicate.clear_to_first_push_to();
        assert_eq!(duplicate.command_names(), vec!["PushTo", "Wait"]);

        let mut head_only = CommandStack::new();
        for id in [CommandId::PushTo, CommandId::Wait] {
            head_only.push_back(request(id)).expect("command queues");
        }
        head_only.clear_to_first_push_to();
        assert_eq!(head_only.command_names(), vec!["PushTo", "Wait"]);
    }

    #[test]
    fn command_stack_snapshot_preserves_acquire_state() {
        let builder_id = ObjectId::new(10);
        let item_id = ObjectId::new(11);

        let mut builder = snapshot_with_id(builder_id.as_u64());
        builder.ocf = ocf::AVAILABLE | ocf::ALIVE;
        builder.collectible = false;
        builder.position = Vector2::new(0, 0);

        let mut item = snapshot_with_id(item_id.as_u64());
        item.definition_id = "WOOD".into();
        item.position = Vector2::new(50, 0);
        item.collectible = true;
        item.ocf = ocf::AVAILABLE | ocf::FULL_CON;
        item.construction = FULL_CON;

        let mut objects = HashMap::new();
        objects.insert(builder.id, builder);
        objects.insert(item.id, item);

        let builder_snapshot = objects.get(&builder_id).expect("builder present");
        let players: HashMap<i32, CommandPlayerSnapshot> = HashMap::new();
        let definitions: HashMap<DefinitionId, CommandDefinitionSnapshot> = HashMap::new();

        let mut stack = CommandStack::new();
        stack
            .push_back(
                CommandRequest::new(CommandId::Acquire)
                    .with_data(CommandData::Text("WOOD".into()))
                    .with_mode(CommandMode::Base),
            )
            .expect("command enqueued");

        let ctx_initial = command_ctx_at_frame(
            builder_snapshot,
            &objects,
            &players,
            &definitions,
            0,
        );

        let evaluation = stack.step(&ctx_initial).expect("Acquire evaluates");
        assert_eq!(evaluation.status, CommandStatus::Running);
        assert!(evaluation.events.is_empty());
        assert!(evaluation.operations.is_empty());

        let script_step = stack.step(&ctx_initial).expect("script step evaluates");
        assert_eq!(script_step.status, CommandStatus::Running);
        assert_eq!(
            stack.len(),
            1,
            "script phase should not enqueue additional commands"
        );
        assert!(
            matches!(
                script_step.events.first(),
                Some(CommandEvent::ControlCommandAcquire { .. })
            ),
            "expected control command event during first acquire evaluation"
        );

        assert!(
            stack.set_acquire_script_result(AcquireScriptResult::Continue),
            "script result should be stored on acquire state"
        );

        let second_step = stack.step(&ctx_initial).expect("second step evaluates");
        assert_eq!(second_step.status, CommandStatus::Running);
        assert_eq!(
            stack.len(),
            2,
            "move command should be queued after script phase"
        );

        let snapshot = stack.snapshot();
        assert_eq!(snapshot.commands.len(), 2);
        match &snapshot.commands[0].state {
            CommandState::Get(state) => {
                assert_eq!(
                    state.target,
                    Some(item_id),
                    "get command should target the acquire candidate"
                );
            }
            other => panic!("expected get command at front, got {:?}", other),
        }
        assert_eq!(snapshot.commands[0].mode, CommandMode::SilentSub);
        match &snapshot.commands[1].state {
            CommandState::Acquire(_) => {}
            other => panic!("expected acquire command second, got {:?}", other),
        }
        assert_eq!(snapshot.commands[1].mode, CommandMode::Base);

        let encoded = serde_json::to_value(&snapshot).expect("serialize snapshot");
        let acquire_entry = encoded["commands"]
            .as_array()
            .and_then(|commands| {
                commands
                    .iter()
                    .find(|entry| entry["state"].get("Acquire").is_some())
            })
            .expect("acquire state present");
        let acquire_state = &acquire_entry["state"]["Acquire"];
        assert!(
            acquire_state.get("candidate").is_none(),
            "Acquire rescans instead of serializing a cross-tick candidate cache"
        );
        assert_eq!(snapshot.commands[0].failures, 0);

        let ctx_followup = command_ctx_at_frame(
            builder_snapshot,
            &objects,
            &players,
            &definitions,
            25,
        );

        let original_second = stack.step(&ctx_followup).expect("second step evaluates");

        let mut restored = CommandStack::new();
        restored.restore_from_snapshot(&snapshot);
        let restored_second = restored
            .step(&ctx_followup)
            .expect("restored step evaluates");

        assert_eq!(original_second, restored_second);
    }

    #[test]
    fn command_stack_snapshot_preserves_buy_state() {
        let base_id = ObjectId::new(200);

        let mut stack = CommandStack::new();
        stack
            .push_back(
                CommandRequest::new(CommandId::Buy)
                    .with_target(Some(base_id))
                    .with_tx(Some(3))
                    .with_data(CommandData::Text("WOOD".into()))
                    .with_update_interval(25),
            )
            .expect("buy command queued");

        let snapshot = stack.snapshot();
        assert_eq!(snapshot.commands.len(), 1);
        assert_eq!(snapshot.commands[0].update_interval, Some(25));
        match &snapshot.commands[0].state {
            CommandState::Buy(state) => {
                assert_eq!(state.target, Some(base_id));
                assert_eq!(state.update_interval, 25);
                assert_eq!(state.remaining_count, 3);
                assert!(!state.evaluation_pending);
            }
            other => panic!("expected buy state, got {:?}", other),
        }
        let mut restored = CommandStack::new();
        restored.restore_from_snapshot(&snapshot);
        assert_eq!(restored.snapshot(), snapshot);
    }

    #[test]
    fn command_failure_feedback_mode_matrix_matches_cpp_fail() {
        fn run(
            mode: CommandMode,
            base_retries: Option<i32>,
        ) -> (Option<CommandFailureFeedback>, i32) {
            let mut stack = CommandStack::new();
            if let Some(retries) = base_retries {
                stack
                    .push_back(
                        CommandRequest::new(CommandId::Wait)
                            .with_retries(retries)
                            .with_mode(CommandMode::Base),
                    )
                    .expect("base queues");
            }
            stack
                .push_front(CommandRequest::new(CommandId::Wait).with_mode(mode))
                .expect("failed command queues");
            stack.entries[0].finished = Some(CommandStatus::Failed);

            let feedback = stack.record_failure_at(0);
            let base_failures = base_retries.map_or(0, |_| stack.entries[1].failures);
            (feedback, base_failures)
        }

        assert!(run(CommandMode::SilentSub, None).0.is_some());
        assert_eq!(run(CommandMode::SilentSub, Some(0)), (None, 1));

        let (feedback, failures) = run(CommandMode::Sub, Some(0));
        assert!(feedback.is_some());
        assert_eq!(failures, 1);
        assert_eq!(run(CommandMode::Sub, Some(1)), (None, 1));
        assert!(run(CommandMode::Sub, None).0.is_some());

        let (feedback, failures) = run(CommandMode::Base, Some(1));
        assert!(feedback.is_some());
        assert_eq!(failures, 0);
        assert_eq!(run(CommandMode::SilentBase, Some(0)), (None, 0));
    }

    #[test]
    fn terminal_call_failure_emits_frozen_feedback_without_pre_stop() {
        let actor_id = ObjectId::new(1);
        let missing_target = ObjectId::new(2);
        let target2 = ObjectId::new(3);
        let mut actor = snapshot_with_id(actor_id.as_u64());
        actor.command_direction = CommandDirection::Right;
        let objects = HashMap::from([(actor_id, actor.clone())]);
        let players = HashMap::new();
        let definitions = HashMap::new();
        let ctx = command_ctx_at_frame(&actor, &objects, &players, &definitions, 0);

        let mut stack = CommandStack::new();
        stack
            .push_front(
                CommandRequest::new(CommandId::Call)
                    .with_target(Some(missing_target))
                    .with_target2(Some(target2))
                    .with_tx_definition("WOOD".into())
                    .with_ty(Some(17))
                    .with_data(CommandData::Text("Work".into()))
                    .with_mode(CommandMode::Base),
            )
            .expect("Call queues");

        let result = stack.execute_front(&ctx).expect("Call fails");
        assert_eq!(result.status, CommandStatus::Failed);
        assert!(
            result.update.is_none(),
            "CallFailed must observe the old ComDir"
        );
        let Some(CommandEvent::FailureFeedback {
            actor_id: event_actor,
            feedback,
        }) = result.events.last()
        else {
            panic!("failure feedback must be the final command event");
        };
        assert_eq!(*event_actor, actor_id);
        assert_eq!(feedback.command.name, "Call");
        assert_eq!(feedback.command.target, Some(missing_target));
        assert_eq!(feedback.command.target2, Some(target2));
        assert_eq!(feedback.command.tx_definition.as_deref(), Some("WOOD"));
        assert_eq!(feedback.command.ty, Some(17));

        let encoded = serde_json::to_value(feedback).expect("feedback serializes");
        let decoded: CommandFailureFeedback =
            serde_json::from_value(encoded).expect("feedback deserializes");
        assert_eq!(decoded, *feedback);
    }

    #[test]
    fn missing_build_target_leaves_stop_to_mode_gated_failure_tail() {
        let actor_id = ObjectId::new(1);
        let actor = CommandObjectSnapshot {
            command_direction: CommandDirection::Right,
            ..snapshot_with_id(actor_id.as_u64())
        };
        let objects = HashMap::from([(actor_id, actor.clone())]);
        let players = HashMap::new();
        let definitions = HashMap::new();
        let ctx = command_ctx_at_frame(&actor, &objects, &players, &definitions, 0);

        let mut stack = CommandStack::new();
        stack
            .push_front(
                CommandRequest::new(CommandId::Build)
                    .with_target(Some(ObjectId::new(99)))
                    .with_mode(CommandMode::SilentBase),
            )
            .expect("Build queues");
        let result = stack.execute_front(&ctx).expect("Build fails");
        assert_eq!(result.status, CommandStatus::Failed);
        assert!(result.update.is_none());
        assert!(result
            .events
            .iter()
            .all(|event| !matches!(event, CommandEvent::FailureFeedback { .. })));
    }

    #[test]
    fn silent_base_no_push_enter_and_get_failures_do_not_pre_stop() {
        let actor_id = ObjectId::new(1);
        let target_id = ObjectId::new(2);
        let actor = CommandObjectSnapshot {
            command_direction: CommandDirection::Right,
            no_push_enter: 1,
            ..snapshot_with_id(actor_id.as_u64())
        };
        let players = HashMap::new();
        let definitions = HashMap::new();

        let target = snapshot_with_id(target_id.as_u64());
        let objects = HashMap::from([(actor_id, actor.clone()), (target_id, target)]);
        let ctx = command_ctx_at_frame(&actor, &objects, &players, &definitions, 0);
        let mut enter = CommandStack::new();
        enter
            .push_front(
                CommandRequest::new(CommandId::Enter)
                    .with_target(Some(target_id))
                    .with_mode(CommandMode::SilentBase),
            )
            .expect("Enter queues");
        let result = enter.execute_front(&ctx).expect("Enter fails");
        assert_eq!(result.status, CommandStatus::Failed);
        assert!(result.update.is_none());
        assert!(result.events.is_empty());
        assert!(result.operations.is_empty());

        let mut target = snapshot_with_id(target_id.as_u64());
        target.collectible = false;
        let objects = HashMap::from([(actor_id, actor.clone()), (target_id, target)]);
        let ctx = command_ctx_at_frame(&actor, &objects, &players, &definitions, 0);
        let mut get = CommandStack::new();
        get.push_front(
            CommandRequest::new(CommandId::Get)
                .with_target(Some(target_id))
                .with_mode(CommandMode::SilentBase),
        )
        .expect("Get queues");
        let result = get.execute_front(&ctx).expect("Get fails");
        assert_eq!(result.status, CommandStatus::Failed);
        assert!(result.update.is_none());
        assert!(result
            .events
            .iter()
            .all(|event| !matches!(event, CommandEvent::FailureFeedback { .. })));
    }

    #[test]
    fn get_event_target_freezes_clear_pointer_order_at_detachment() {
        let target = ObjectId::new(98);

        let mut attached = CommandStack::new();
        attached
            .push_front(CommandRequest::new(CommandId::Get).with_target(Some(target)))
            .expect("Get queues");
        let attached_id = attached.entries.front().expect("Get remains").instance_id;
        let CommandState::Get(state) = &mut attached.entries.front_mut().unwrap().state else {
            panic!("command should be Get");
        };
        state.enter_pending = true;
        assert!(attached.clear_object_reference(target));
        assert_eq!(
            attached.get_event_target_after_callback(attached_id, target),
            None,
            "ClearPointers reaches a still-linked executing Get"
        );

        let mut detached = CommandStack::new();
        detached
            .push_front(CommandRequest::new(CommandId::Get).with_target(Some(target)))
            .expect("Get queues");
        let detached_id = detached.entries.front().expect("Get remains").instance_id;
        let CommandState::Get(state) = &mut detached.entries.front_mut().unwrap().state else {
            panic!("command should be Get");
        };
        state.enter_pending = true;
        detached.clear();
        detached
            .push_front(CommandRequest::new(CommandId::Get).with_target(Some(ObjectId::new(99))))
            .expect("replacement Get queues");
        let CommandState::Get(state) = &mut detached.entries.front_mut().unwrap().state else {
            panic!("replacement command should be Get");
        };
        state.enter_pending = true;
        assert_eq!(
            detached.get_event_target_after_callback(detached_id, target),
            Some(target),
            "an unlinked iExec Get retains its raw Target"
        );
        let detached_resolution = detached
            .resolve_get_attempt(detached_id, GetAttemptDisposition::Fail)
            .expect("the detached native Get still runs its failure tail");
        assert_eq!(
            detached_resolution
                .feedback
                .as_ref()
                .map(|feedback| feedback.command.name.as_str()),
            Some("Get")
        );
        assert!(matches!(
            &detached.entries.front().expect("replacement remains").state,
            CommandState::Get(state) if state.enter_pending
        ));

        let mut cleared_then_detached = CommandStack::new();
        cleared_then_detached
            .push_front(CommandRequest::new(CommandId::Get).with_target(Some(target)))
            .expect("Get queues");
        let cleared_id = cleared_then_detached
            .entries
            .front()
            .expect("Get remains")
            .instance_id;
        let CommandState::Get(state) =
            &mut cleared_then_detached.entries.front_mut().unwrap().state
        else {
            panic!("command should be Get");
        };
        state.enter_pending = true;
        assert!(cleared_then_detached.clear_object_reference(target));
        cleared_then_detached.clear();
        let rebound_cleared_id =
            cleared_then_detached.resolve_event_instance_id(CommandEventInstanceKind::Get, 0);
        assert_eq!(rebound_cleared_id, cleared_id);
        assert_eq!(
            cleared_then_detached.get_event_target_after_callback(rebound_cleared_id, target),
            None,
            "a restored zero-token event must rebind to the detached null Target"
        );
        assert!(
            cleared_then_detached
                .resolve_get_attempt(rebound_cleared_id, GetAttemptDisposition::Continue)
                .is_some(),
            "the detached continuation consumes its frozen Get body"
        );
        assert!(cleared_then_detached.detached_get_attempts.is_empty());
    }

    #[test]
    fn async_get_and_cleared_grab_surface_failure_feedback() {
        let target = ObjectId::new(99);

        let mut get_stack = CommandStack::new();
        get_stack
            .push_front(
                CommandRequest::new(CommandId::Get)
                    .with_target(Some(target))
                    .with_mode(CommandMode::Base),
            )
            .expect("Get queues");
        let get_instance_id = get_stack.entries[0].instance_id;
        let CommandState::Get(get) = &mut get_stack.entries[0].state else {
            panic!("Get is front");
        };
        get.enter_pending = true;
        let get_resolution = get_stack
            .resolve_get_attempt(get_instance_id, GetAttemptDisposition::Fail)
            .expect("pending Get resolves");
        assert_eq!(
            get_resolution
                .feedback
                .as_ref()
                .map(|feedback| feedback.command.name.as_str()),
            Some("Get")
        );

        let mut grab_stack = CommandStack::new();
        grab_stack
            .push_front(
                CommandRequest::new(CommandId::Grab)
                    .with_target(Some(target))
                    .with_mode(CommandMode::Base),
            )
            .expect("Grab queues");
        let CommandState::Grab(grab) = &mut grab_stack.entries[0].state else {
            panic!("Grab is front");
        };
        grab.reject_pending = true;
        assert!(grab_stack.clear_object_reference(target));
        assert!(grab_stack.fail_pending_grab_if_target_cleared(target));
        assert_eq!(
            grab_stack
                .take_failure_feedback()
                .map(|feedback| feedback.command.name),
            Some("Grab".into())
        );
        assert!(grab_stack.take_failure_feedback().is_none());
    }

    #[test]
    fn targetless_enter_materializes_and_increments_the_base_failure_counter() {
        let actor = snapshot_with_id(1);
        let objects = HashMap::from([(actor.id, actor.clone())]);
        let players = HashMap::new();
        let definitions = HashMap::new();
        let ctx = command_ctx_at_frame(&actor, &objects, &players, &definitions, 0);

        let mut stack = CommandStack::new();
        stack
            .push_back(CommandRequest::new(CommandId::Wait).with_mode(CommandMode::Base))
            .expect("base queues");
        stack
            .push_front(CommandRequest::new(CommandId::Enter).with_mode(CommandMode::SilentSub))
            .expect("C++ links a targetless Enter");
        assert_eq!(stack.command_names(), vec!["Enter", "Wait"]);

        let failed = stack.step(&ctx).expect("targetless Enter executes");
        assert_eq!(failed.status, CommandStatus::Failed);
        let snapshot = stack.snapshot();
        assert_eq!(snapshot.command_names(), vec!["Wait"]);
        assert_eq!(snapshot.commands[0].failures, 1);
    }

    #[test]
    fn targetless_call_materializes_before_parent_failure_handling() {
        let actor = snapshot_with_id(2);
        let objects = HashMap::from([(actor.id, actor.clone())]);
        let players = HashMap::new();
        let definitions = HashMap::new();
        let ctx = command_ctx_at_frame(&actor, &objects, &players, &definitions, 0);

        let mut stack = CommandStack::new();
        stack
            .push_back(CommandRequest::new(CommandId::Wait).with_mode(CommandMode::Base))
            .expect("parent queues");
        let call = CommandRequest::new(CommandId::Call)
            .with_data(CommandData::Text("Work".into()))
            .with_mode(CommandMode::SilentSub);
        let mut parent_result = CommandStepResult::running(None)
            .with_operations(vec![CommandOperation::PushFront(call.clone())]);
        stack.apply_result_operations(&mut parent_result);

        let snapshot = stack.snapshot();
        assert_eq!(snapshot.command_names(), vec!["Call", "Wait"]);
        assert!(matches!(snapshot.commands[0].state, CommandState::Call(_)));
        assert_eq!(snapshot.command_views()[0].data, call.data);

        // The materialized Call and its native command identity
        // survive save/restore, so a parent latch can never outlive a child
        // merely because typed request conversion rejected its fields.
        let encoded = serde_json::to_value(&snapshot).expect("snapshot serializes");
        let decoded: CommandStackSnapshot =
            serde_json::from_value(encoded).expect("snapshot deserializes");
        let mut restored = CommandStack::new();
        restored.restore_from_snapshot(&decoded);
        let failed = restored.step(&ctx).expect("malformed Call executes");
        assert_eq!(failed.status, CommandStatus::Failed);
        let after = restored.snapshot();
        assert_eq!(after.command_names(), vec!["Wait"]);
        assert_eq!(after.commands[0].failures, 1);
    }

    #[test]
    fn failed_full_stack_push_does_not_permanently_latch_its_parent() {
        let actor_id = ObjectId::new(3);
        let pushed_id = ObjectId::new(4);
        let mut actor = snapshot_with_id(actor_id.as_u64());
        actor.action_procedure = ActionProcedure::Push;
        actor.action_target = Some(pushed_id);
        let pushed = snapshot_with_id(pushed_id.as_u64());
        let objects = HashMap::from([(actor_id, actor.clone()), (pushed_id, pushed)]);
        let players = HashMap::new();
        let definitions = HashMap::new();
        let ctx = command_ctx_at_frame(&actor, &objects, &players, &definitions, 0);

        let mut stack = CommandStack::new();
        stack
            .push_back(
                CommandRequest::new(CommandId::Dig)
                    .with_tx(Some(0))
                    .with_ty(Some(0))
                    .with_mode(CommandMode::Base),
            )
            .expect("Dig queues");
        for _ in 1..MAX_COMMAND_STACK {
            stack
                .push_back(CommandRequest::new(CommandId::Wait))
                .expect("tail fills command stack");
        }

        let first = stack.step(&ctx).expect("Dig attempts UnGrab");
        assert_eq!(first.status, CommandStatus::Running);
        assert_eq!(stack.len(), MAX_COMMAND_STACK);
        assert_eq!(stack.command_names()[0], "Dig");

        // Once capacity becomes available, executing the still-front parent
        // clears the stale latch and retries the child instead of hanging.
        stack.entries.pop_back().expect("free one command slot");
        let retry = stack.step(&ctx).expect("Dig retries UnGrab");
        assert_eq!(retry.status, CommandStatus::Running);
        assert_eq!(stack.command_names()[0], "UnGrab");
    }

    #[test]
    fn typed_malformed_exceptions_keep_native_evaluation_and_guard_order() {
        let actor = snapshot_with_id(5);
        let objects = HashMap::from([(actor.id, actor.clone())]);
        let players = HashMap::new();
        let definitions = HashMap::new();
        let ctx = command_ctx_at_frame(&actor, &objects, &players, &definitions, 0);

        let dig = DigState::from_request(&CommandRequest::new(CommandId::Dig))
            .expect("missing Dig coordinates are numeric zero");
        assert_eq!(dig.target, Vector2::ZERO);

        let mut unselected = actor.clone();
        unselected.crew_member = true;
        unselected.owner = 7;
        unselected.selected = false;
        let unselected_ctx = command_ctx_at_frame(&unselected, &objects, &players, &definitions, 0);
        let mut follow = FollowState::from_request(&CommandRequest::new(CommandId::Follow))
            .expect("targetless Follow still links");
        assert_eq!(
            follow.step(&unselected_ctx).status,
            CommandStatus::Completed
        );

        let mut push_to = CommandStack::new();
        push_to
            .push_front(CommandRequest::new(CommandId::PushTo))
            .expect("targetless PushTo links");
        let evaluation = push_to.step(&ctx).expect("PushTo evaluates");
        assert_eq!(evaluation.status, CommandStatus::Running);
        assert_eq!(
            (push_to.command_views()[0].tx, push_to.command_views()[0].ty),
            (Some(0), Some(0))
        );
        assert_eq!(
            push_to.step(&ctx).expect("PushTo guard executes").status,
            CommandStatus::Failed
        );

        let mut acquire = CommandStack::new();
        acquire
            .push_front(CommandRequest::new(CommandId::Acquire))
            .expect("Data=0 Acquire links");
        assert_eq!(
            acquire.step(&ctx).expect("Acquire evaluates").status,
            CommandStatus::Running
        );
        assert_eq!(
            acquire.step(&ctx).expect("Acquire guard executes").status,
            CommandStatus::Failed
        );
    }

    #[test]
    fn failing_subcommand_increments_base_failures_and_schedules_retry() {
        let actor_id = ObjectId::new(1);
        let mut actor = snapshot_with_id(actor_id.as_u64());
        actor.ocf = ocf::AVAILABLE | ocf::ALIVE;
        actor.collectible = false;
        actor.position = Vector2::new(0, 0);
        actor.command_direction = CommandDirection::Right;

        let mut objects = HashMap::new();
        objects.insert(actor.id, actor.clone());

        let players: HashMap<i32, CommandPlayerSnapshot> = HashMap::new();
        let definitions: HashMap<DefinitionId, CommandDefinitionSnapshot> = HashMap::new();
        let ctx = command_ctx_at_frame(&actor, &objects, &players, &definitions, 0);

        let mut stack = CommandStack::new();
        let wait_request = CommandRequest::new(CommandId::Wait)
            .with_update_interval(1)
            .with_retries(1)
            .with_mode(CommandMode::Base);
        stack.push_back(wait_request).expect("wait command queued");
        stack
            .push_front(
                CommandRequest::new(CommandId::Enter)
                    .with_target(Some(ObjectId::new(999)))
                    .with_mode(CommandMode::SilentSub),
            )
            .expect("enter command queued");

        let initial_snapshot = stack.snapshot();
        assert_eq!(initial_snapshot.commands.len(), 2);
        match &initial_snapshot.commands[1].state {
            CommandState::Wait(_) => {
                assert_eq!(initial_snapshot.commands[1].retries, 1);
            }
            other => panic!("expected wait command as base, got {:?}", other),
        }

        let first = stack.step(&ctx).expect("enter should evaluate");
        assert_eq!(first.status, CommandStatus::Failed);
        let snapshot = stack.snapshot();
        assert_eq!(snapshot.commands.len(), 1);
        assert_eq!(snapshot.commands[0].failures, 1);
        assert_eq!(snapshot.commands[0].retries, 1);

        let second = stack.step(&ctx).expect("wait should evaluate");
        assert_eq!(second.status, CommandStatus::Running);
        assert!(second.update.is_none(), "delegated retries do not pre-stop");
        assert!(second.events.is_empty());

        let post_snapshot = stack.snapshot();
        assert_eq!(post_snapshot.commands.len(), 2);
        match &post_snapshot.commands[1].state {
            CommandState::Wait(_) => {
                assert_eq!(post_snapshot.commands[1].failures, 0);
                assert_eq!(post_snapshot.commands[1].retries, 0);
            }
            other => panic!(
                "expected wait command after retry scheduling, got {:?}",
                other
            ),
        }
        match &post_snapshot.commands[0].state {
            CommandState::Retry(_) => {}
            other => panic!("expected retry command at front, got {:?}", other),
        }
    }

    #[test]
    fn nested_subcommand_failure_retries_middle_before_reaching_base() {
        // C4Command::GetBaseCommand returns the next unfinished command,
        // regardless of mode (C4Command.cpp:2498-2508).
        let actor_id = ObjectId::new(1);
        let mut actor = snapshot_with_id(actor_id.as_u64());
        actor.ocf = ocf::AVAILABLE | ocf::ALIVE;
        actor.collectible = false;

        let mut objects = HashMap::new();
        objects.insert(actor.id, actor.clone());
        let players = HashMap::new();
        let definitions = HashMap::new();
        let ctx = command_ctx_at_frame(&actor, &objects, &players, &definitions, 0);

        let mut stack = CommandStack::new();
        stack
            .push_back(CommandRequest::new(CommandId::Wait).with_mode(CommandMode::Base))
            .expect("base command queued");
        stack
            .push_front(
                CommandRequest::new(CommandId::Enter)
                    .with_target(Some(ObjectId::new(998)))
                    .with_retries(1)
                    .with_mode(CommandMode::Sub),
            )
            .expect("middle command queued");
        stack
            .push_front(
                CommandRequest::new(CommandId::Enter)
                    .with_target(Some(ObjectId::new(999)))
                    .with_mode(CommandMode::SilentSub),
            )
            .expect("leaf command queued");

        let leaf_failure = stack.step(&ctx).expect("leaf should evaluate");
        assert_eq!(leaf_failure.status, CommandStatus::Failed);
        let after_leaf = stack.snapshot();
        assert_eq!(after_leaf.commands.len(), 2);
        assert_eq!(after_leaf.commands[0].mode, CommandMode::Sub);
        assert_eq!(after_leaf.commands[0].failures, 1);
        assert_eq!(after_leaf.commands[0].retries, 1);
        assert_eq!(after_leaf.commands[1].failures, 0);

        let middle_retry = stack.step(&ctx).expect("middle should consume its retry");
        assert_eq!(middle_retry.status, CommandStatus::Running);
        let during_retry = stack.snapshot();
        assert_eq!(during_retry.commands.len(), 3);
        assert!(matches!(
            during_retry.commands[0].state,
            CommandState::Retry(_)
        ));
        assert_eq!(during_retry.commands[1].failures, 0);
        assert_eq!(during_retry.commands[1].retries, 0);
        assert_eq!(during_retry.commands[2].failures, 0);

        for _ in 0..10 {
            stack.step(&ctx).expect("retry should evaluate");
        }
        let middle_failure = stack.step(&ctx).expect("middle should evaluate again");
        assert_eq!(middle_failure.status, CommandStatus::Failed);
        let after_middle = stack.snapshot();
        assert_eq!(after_middle.commands.len(), 1);
        assert_eq!(after_middle.commands[0].failures, 1);
    }

    #[test]
    fn subcommand_failure_skips_finished_entry_and_targets_any_mode() {
        // GetBaseCommand skips finished commands and returns the first live
        // command below them without inspecting BaseMode (C4Command.cpp:2498-2508).
        let actor_id = ObjectId::new(1);
        let actor = snapshot_with_id(actor_id.as_u64());
        let mut objects = HashMap::new();
        objects.insert(actor.id, actor.clone());
        let players = HashMap::new();
        let definitions = HashMap::new();
        let ctx = command_ctx_at_frame(&actor, &objects, &players, &definitions, 0);

        let mut stack = CommandStack::new();
        stack
            .push_back(CommandRequest::new(CommandId::Wait).with_mode(CommandMode::SilentSub))
            .expect("live tail queued");
        stack
            .push_front(CommandRequest::new(CommandId::Wait))
            .expect("finished middle queued");
        stack
            .push_front(
                CommandRequest::new(CommandId::Enter)
                    .with_target(Some(ObjectId::new(999)))
                    .with_mode(CommandMode::SilentSub),
            )
            .expect("failing leaf queued");
        stack.finish_entry_public(1, true);

        let leaf_failure = stack.step(&ctx).expect("leaf should evaluate");
        assert_eq!(leaf_failure.status, CommandStatus::Failed);
        let snapshot = stack.snapshot();
        assert_eq!(snapshot.commands.len(), 1);
        assert_eq!(snapshot.commands[0].mode, CommandMode::SilentSub);
        assert_eq!(snapshot.commands[0].failures, 1);
    }

    #[test]
    fn command_stack_put_resolves_live_object_com_put_attempt() {
        let actor_id = ObjectId::new(20);
        let item_id = ObjectId::new(21);
        let container_id = ObjectId::new(22);

        let mut actor = snapshot_with_id(actor_id.as_u64());
        actor.ocf = ocf::AVAILABLE | ocf::ALIVE;
        actor.collectible = false;
        actor.position = Vector2::new(0, 0);
        actor.container = Some(container_id);
        actor.contents.push(item_id);

        let mut item = snapshot_with_id(item_id.as_u64());
        item.definition_id = "WOOD".into();
        item.position = actor.position;
        item.collectible = true;
        item.ocf = ocf::AVAILABLE | ocf::FULL_CON;
        item.construction = FULL_CON;
        item.container = Some(actor_id);

        let mut container = snapshot_with_id(container_id.as_u64());
        container.position = Vector2::new(10, 0);
        container.collectible = false;
        container.category = CATEGORY_STRUCTURE;
        container.ocf = ocf::AVAILABLE | ocf::ENTRANCE;

        let mut objects = HashMap::new();
        objects.insert(actor.id, actor.clone());
        objects.insert(item.id, item.clone());
        objects.insert(container.id, container.clone());

        let players: HashMap<i32, CommandPlayerSnapshot> = HashMap::new();
        let definitions: HashMap<DefinitionId, CommandDefinitionSnapshot> = HashMap::new();

        let mut stack = CommandStack::new();
        stack
            .push_back(
                CommandRequest::new(CommandId::Put)
                    .with_target(Some(container_id))
                    .with_target2(Some(item_id)),
            )
            .expect("put enqueued");

        let actor_snapshot = objects.get(&actor_id).expect("actor present");
        let ctx = command_ctx_at_frame(actor_snapshot, &objects, &players, &definitions, 0);

        let command_instance_id = stack.entries.front().expect("Put remains").instance_id;
        let result = stack.step(&ctx).expect("put evaluates");
        assert_eq!(result.status, CommandStatus::Running);
        assert!(result.operations.is_empty());
        assert_eq!(
            result.events,
            vec![CommandEvent::ObjectComPut {
                actor_id,
                target_id: container_id,
                object_id: item_id,
                ungrab_on_success: false,
                command_instance_id,
            }]
        );
        assert!(stack
            .resolve_put_attempt(command_instance_id, true)
            .is_none());

        assert_eq!(stack.len(), 1, "Put finishes on the following execute");
        objects.get_mut(&item_id).expect("item present").container = Some(container_id);
        objects
            .get_mut(&actor_id)
            .expect("actor present")
            .contents
            .clear();
        let actor_snapshot = objects.get(&actor_id).expect("actor present");
        let ctx = command_ctx_at_frame(actor_snapshot, &objects, &players, &definitions, 1);
        let result = stack.step(&ctx).expect("Put observes transferred item");
        assert_eq!(result.status, CommandStatus::Completed);
        assert_eq!(stack.len(), 0);
    }

    #[test]
    fn object_com_put_result_stays_bound_to_detached_command_across_restore() {
        let target_id = ObjectId::new(23);
        let item_id = ObjectId::new(24);
        let mut engine_stack = CommandStack::new();
        engine_stack
            .push_front(
                CommandRequest::new(CommandId::Put)
                    .with_target(Some(target_id))
                    .with_target2(Some(item_id))
                    .with_mode(CommandMode::Base),
            )
            .expect("outer Put queues");
        let outer_id = engine_stack.entries.front().unwrap().instance_id;
        let CommandState::Put(outer_state) = &mut engine_stack.entries.front_mut().unwrap().state
        else {
            panic!("outer command should be Put");
        };
        outer_state.put_pending = true;

        // Model a callback-side SetCommand(Put): the old native C4Command is
        // unlinked but retained by iExec while the replacement is visible.
        let mut callback_stack = engine_stack.clone();
        callback_stack.clear();
        callback_stack
            .push_front(
                CommandRequest::new(CommandId::Put)
                    .with_target(Some(target_id))
                    .with_target2(Some(item_id)),
            )
            .expect("replacement Put queues");
        let replacement_id = callback_stack.entries.front().unwrap().instance_id;
        let CommandState::Put(replacement_state) =
            &mut callback_stack.entries.front_mut().unwrap().state
        else {
            panic!("replacement command should be Put");
        };
        replacement_state.put_pending = true;

        engine_stack.restore_from_snapshot(&callback_stack.snapshot());
        assert_eq!(
            engine_stack.entries.front().unwrap().instance_id,
            replacement_id
        );
        assert_eq!(engine_stack.detached_put_attempts.len(), 1);
        assert_eq!(
            engine_stack.resolve_event_instance_id(CommandEventInstanceKind::Put, 0),
            replacement_id,
            "a persisted zero token rebinds only to the visible pending Put"
        );

        let feedback = engine_stack
            .resolve_put_attempt(outer_id, false)
            .expect("detached Base Put runs its own failure tail");
        assert_eq!(feedback.command.name, "Put");
        assert!(feedback.command.finished);
        assert!(engine_stack.detached_put_attempts.is_empty());
        assert!(matches!(
            &engine_stack.entries.front().unwrap().state,
            CommandState::Put(state) if state.put_pending
        ));
        assert!(engine_stack.entries.front().unwrap().finished.is_none());

        assert!(engine_stack
            .resolve_put_attempt(replacement_id, true)
            .is_none());
        assert!(matches!(
            &engine_stack.entries.front().unwrap().state,
            CommandState::Put(state) if !state.put_pending
        ));
    }

    #[test]
    fn get_target_state_stays_bound_to_detached_command_across_restore() {
        let target = ObjectId::new(25);
        let replacement_target = ObjectId::new(26);

        for clear_before_detach in [false, true] {
            let mut engine_stack = CommandStack::new();
            engine_stack
                .push_front(CommandRequest::new(CommandId::Get).with_target(Some(target)))
                .expect("outer Get queues");
            let outer_id = engine_stack.entries.front().unwrap().instance_id;
            let CommandState::Get(outer_state) =
                &mut engine_stack.entries.front_mut().unwrap().state
            else {
                panic!("outer command should be Get");
            };
            outer_state.enter_pending = true;
            if clear_before_detach {
                assert!(engine_stack.clear_object_reference(target));
            }

            let mut callback_stack = engine_stack.clone();
            callback_stack.clear();
            callback_stack
                .push_front(
                    CommandRequest::new(CommandId::Get).with_target(Some(replacement_target)),
                )
                .expect("replacement Get queues");
            let replacement_id = callback_stack.entries.front().unwrap().instance_id;
            let CommandState::Get(replacement_state) =
                &mut callback_stack.entries.front_mut().unwrap().state
            else {
                panic!("replacement command should be Get");
            };
            replacement_state.enter_pending = true;

            engine_stack.restore_from_snapshot(&callback_stack.snapshot());
            assert_eq!(
                engine_stack.entries.front().unwrap().instance_id,
                replacement_id
            );
            assert_eq!(engine_stack.detached_get_attempts.len(), 1);
            assert_eq!(
                engine_stack.get_event_target_after_callback(outer_id, target),
                (!clear_before_detach).then_some(target),
                "restore must preserve the old Get's pointer state at detachment"
            );
            assert!(engine_stack
                .resolve_get_attempt(outer_id, GetAttemptDisposition::Continue)
                .is_some());
            assert!(engine_stack.detached_get_attempts.is_empty());
            assert!(matches!(
                &engine_stack.entries.front().unwrap().state,
                CommandState::Get(state) if state.enter_pending
            ));
            assert!(engine_stack.entries.front().unwrap().finished.is_none());
        }
    }

    #[test]
    fn object_com_put_runtime_identity_is_not_persisted() {
        let event = CommandEvent::ObjectComPut {
            actor_id: ObjectId::new(25),
            target_id: ObjectId::new(26),
            object_id: ObjectId::new(27),
            ungrab_on_success: true,
            command_instance_id: 91,
        };
        let encoded = serde_json::to_value(event).expect("ObjectComPut serializes");
        assert!(encoded.get("command_instance_id").is_none());
        let decoded: CommandEvent =
            serde_json::from_value(encoded).expect("ObjectComPut deserializes");
        assert!(matches!(
            decoded,
            CommandEvent::ObjectComPut {
                command_instance_id: 0,
                ..
            }
        ));
    }

    #[test]
    fn buy_emits_a_live_evaluation_instead_of_static_spawn_events() {
        let builder_id = ObjectId::new(1);
        let base_id = ObjectId::new(2);

        let mut builder = snapshot_with_id(builder_id.as_u64());
        builder.owner = 42;
        builder.position = Vector2::new(10, 5);
        builder.command_direction = CommandDirection::Right;

        let mut base = snapshot_with_id(base_id.as_u64());
        base.owner = 42;
        base.base = 42;
        base.position = Vector2::new(20, 10);
        base.category = CATEGORY_STRUCTURE;
        base.ocf = ocf::AVAILABLE | ocf::ENTRANCE;
        base.collectible = false;

        let mut objects = HashMap::new();
        objects.insert(builder.id, builder.clone());
        objects.insert(base.id, base.clone());

        let mut home_base = HashMap::new();
        home_base.insert("WOOD".to_string(), 2);

        let mut players: HashMap<i32, CommandPlayerSnapshot> = HashMap::new();
        players.insert(
            42,
            CommandPlayerSnapshot {
                status: PlayerStatus::Active,
                surrendered: false,
                wealth: 100,
                home_base_material: home_base,
                home_base_material_entries: Vec::new(),
                knowledge: Vec::new(),
                hostile_to: Vec::new(),
            },
        );

        let mut definitions: HashMap<DefinitionId, CommandDefinitionSnapshot> = HashMap::new();
        definitions.insert(
            "WOOD".to_string(),
            CommandDefinitionSnapshot {
                value: 25,
                can_chop: false,
                chop_action: None,
                constructable: false,
                grab: 0,
                ..CommandDefinitionSnapshot::default()
            },
        );

        let ctx = CommandRuntimeContext {
            position: builder.position,
            ..command_ctx_at_frame(
                objects.get(&builder_id).expect("builder present"),
                &objects,
                &players,
                &definitions,
                0,
            )
        };

        let mut state = BuyState::from_request(
            &CommandRequest::new(CommandId::Buy).with_data(CommandData::Text("WOOD".into())),
        )
        .expect("state created");

        let result = state.step(&ctx);
        assert_eq!(result.status, CommandStatus::Running);
        assert_eq!(result.operations.len(), 0);
        assert!(result.update.is_none());
        assert_eq!(
            result.events,
            vec![CommandEvent::EvaluateBuy {
                actor_id: builder_id,
                base_id,
                definition_id: "WOOD".into(),
                buyer: 42,
                payer: 42,
                count: 0,
            }]
        );
        assert!(state.evaluation_pending);
    }

    #[test]
    fn explicit_buy_obeys_the_global_capability_gate() {
        let builder_id = ObjectId::new(1);
        let target_id = ObjectId::new(2);
        let item_id = ObjectId::new(3);

        let mut builder = snapshot_with_id(builder_id.as_u64());
        builder.owner = 42;
        builder.position = Vector2::new(0, 0);
        builder.command_direction = CommandDirection::Right;

        let mut target = snapshot_with_id(target_id.as_u64());
        target.base = 42;
        target.position = Vector2::new(100, 0);
        target.category = CATEGORY_STRUCTURE;
        target.ocf = ocf::AVAILABLE | ocf::ENTRANCE;
        target.collectible = false;
        target.contents.push(item_id);

        let mut item = snapshot_with_id(item_id.as_u64());
        item.definition_id = "WOOD".into();
        item.collectible = true;
        item.construction = FULL_CON;
        item.container = Some(target_id);
        item.position = target.position;

        let mut objects = HashMap::new();
        objects.insert(builder.id, builder.clone());
        objects.insert(target.id, target);
        objects.insert(item.id, item);

        let mut players: HashMap<i32, CommandPlayerSnapshot> = HashMap::new();
        players.insert(
            42,
            CommandPlayerSnapshot {
                status: PlayerStatus::Active,
                surrendered: false,
                wealth: 100,
                home_base_material: HashMap::new(),
                home_base_material_entries: Vec::new(),
                knowledge: Vec::new(),
                hostile_to: Vec::new(),
            },
        );

        let mut definitions: HashMap<DefinitionId, CommandDefinitionSnapshot> = HashMap::new();
        definitions.insert(
            "WOOD".to_string(),
            CommandDefinitionSnapshot {
                value: 25,
                can_chop: false,
                chop_action: None,
                constructable: false,
                grab: 0,
                ..CommandDefinitionSnapshot::default()
            },
        );

        let ctx = CommandRuntimeContext {
            position: builder.position,
            base_buy_enabled: false,
            ..command_ctx_at_frame(
                objects.get(&builder_id).expect("builder present"),
                &objects,
                &players,
                &definitions,
                0,
            )
        };

        let parent_request = CommandRequest::new(CommandId::Buy)
            .with_target(Some(target_id))
            .with_data(CommandData::Text("WOOD".into()));
        let mut state = BuyState::from_request(&parent_request).expect("state created");

        let result = state.step(&ctx);
        assert_eq!(result.status, CommandStatus::Failed);
        assert!(result.update.is_none());
        assert!(result.operations.is_empty());
        assert!(result.events.is_empty());
    }

    #[test]
    fn sell_without_definition_is_the_internal_menu_command() {
        // C4Command::Sell treats Data=0 as "open C4MN_Sell" rather than
        // rejecting the command (C4Command.cpp:2052-2057).
        let request = CommandRequest::new(CommandId::Sell);
        let state = SellState::from_request(&request).expect("menu command is valid");
        assert!(state.definition_id.is_empty());
    }

    #[test]
    fn sell_requests_enter_when_outside() {
        let builder_id = ObjectId::new(1);
        let base_id = ObjectId::new(2);
        let item_id = ObjectId::new(3);

        let mut builder = snapshot_with_id(builder_id.as_u64());
        builder.owner = 7;
        builder.position = Vector2::new(-50, 0);
        builder.command_direction = CommandDirection::Left;

        let mut base = snapshot_with_id(base_id.as_u64());
        base.owner = 7;
        base.base = 7;
        base.position = Vector2::new(0, 0);
        base.category = CATEGORY_STRUCTURE;
        base.ocf = ocf::AVAILABLE | ocf::ENTRANCE;
        base.collectible = false;
        base.contents.push(item_id);

        let mut item = snapshot_with_id(item_id.as_u64());
        item.definition_id = "ORE1".into();
        item.collectible = true;
        item.container = Some(base_id);
        item.position = base.position;

        let mut objects = HashMap::new();
        objects.insert(builder.id, builder.clone());
        objects.insert(base.id, base.clone());
        objects.insert(item.id, item);

        let mut players: HashMap<i32, CommandPlayerSnapshot> = HashMap::new();
        players.insert(
            7,
            CommandPlayerSnapshot {
                status: PlayerStatus::Active,
                surrendered: false,
                wealth: 100,
                home_base_material: HashMap::new(),
                home_base_material_entries: Vec::new(),
                knowledge: Vec::new(),
                hostile_to: Vec::new(),
            },
        );

        let mut definitions: HashMap<DefinitionId, CommandDefinitionSnapshot> = HashMap::new();
        definitions.insert(
            "ORE1".to_string(),
            CommandDefinitionSnapshot {
                value: 30,
                can_chop: false,
                chop_action: None,
                constructable: false,
                grab: 0,
                ..CommandDefinitionSnapshot::default()
            },
        );

        let ctx = CommandRuntimeContext {
            position: builder.position,
            ..command_ctx_at_frame(
                objects.get(&builder_id).expect("builder present"),
                &objects,
                &players,
                &definitions,
                0,
            )
        };

        let mut state = SellState::from_request(
            &CommandRequest::new(CommandId::Sell)
                .with_target(Some(base_id))
                .with_data(CommandData::Text("ORE1".into())),
        )
        .expect("state created");

        let result = state.step(&ctx);
        assert_eq!(result.status, CommandStatus::Running);
        assert!(result.update.is_none());
        assert_eq!(result.operations.len(), 1);
        match &result.operations[0] {
            CommandOperation::PushFront(request) => {
                assert_eq!(request.id, CommandId::Enter);
                assert_eq!(request.target, Some(base_id));
                assert_eq!(request.update_interval, 50);
                assert_eq!(request.mode, CommandMode::SilentSub);
            }
            other => panic!("expected enter request, got {:?}", other),
        }
    }

    #[test]
    fn sell_emits_a_live_evaluation_when_inside() {
        let builder_id = ObjectId::new(1);
        let base_id = ObjectId::new(2);
        let item_id = ObjectId::new(3);

        let mut builder = snapshot_with_id(builder_id.as_u64());
        builder.owner = 11;
        builder.container = Some(base_id);
        builder.position = Vector2::new(0, 0);
        builder.command_direction = CommandDirection::Right;

        let mut base = snapshot_with_id(base_id.as_u64());
        base.owner = 11;
        base.base = 11;
        base.position = Vector2::new(0, 0);
        base.category = CATEGORY_STRUCTURE;
        base.ocf = ocf::AVAILABLE | ocf::ENTRANCE;
        base.collectible = false;
        base.contents.push(item_id);

        let mut item = snapshot_with_id(item_id.as_u64());
        item.definition_id = "ORE1".into();
        item.collectible = true;
        item.container = Some(base_id);
        item.position = base.position;
        item.ocf = ocf::AVAILABLE | ocf::FULL_CON;
        item.construction = FULL_CON;

        let mut objects = HashMap::new();
        objects.insert(builder.id, builder.clone());
        objects.insert(base.id, base.clone());
        objects.insert(item.id, item.clone());

        let mut players: HashMap<i32, CommandPlayerSnapshot> = HashMap::new();
        players.insert(
            11,
            CommandPlayerSnapshot {
                status: PlayerStatus::Active,
                surrendered: false,
                wealth: 10,
                home_base_material: HashMap::new(),
                home_base_material_entries: Vec::new(),
                knowledge: Vec::new(),
                hostile_to: Vec::new(),
            },
        );

        let mut definitions: HashMap<DefinitionId, CommandDefinitionSnapshot> = HashMap::new();
        definitions.insert(
            "ORE1".to_string(),
            CommandDefinitionSnapshot {
                value: 15,
                can_chop: false,
                chop_action: None,
                constructable: false,
                grab: 0,
                ..CommandDefinitionSnapshot::default()
            },
        );

        let ctx = CommandRuntimeContext {
            position: builder.position,
            ..command_ctx_at_frame(
                objects.get(&builder_id).expect("builder present"),
                &objects,
                &players,
                &definitions,
                0,
            )
        };

        let mut state = SellState::from_request(
            &CommandRequest::new(CommandId::Sell)
                .with_target(Some(base_id))
                .with_data(CommandData::Text("ORE1".into())),
        )
        .expect("state created");

        let result = state.step(&ctx);
        assert_eq!(result.status, CommandStatus::Running);
        assert!(result.operations.is_empty());
        assert!(result.update.is_none());
        assert_eq!(
            result.events,
            vec![CommandEvent::EvaluateSell {
                actor_id: builder_id,
                base_id,
                definition_id: "ORE1".into(),
                preferred: None,
                count: 0,
            }]
        );
        assert!(state.evaluation_pending);

        let follow_up = state.step(&ctx);
        assert_eq!(follow_up.status, CommandStatus::Running);
        assert!(follow_up.events.is_empty());
    }

    #[test]
    fn sell_fails_when_disabled() {
        let builder_id = ObjectId::new(1);
        let base_id = ObjectId::new(2);

        let mut builder = snapshot_with_id(builder_id.as_u64());
        builder.owner = 5;
        builder.container = Some(base_id);
        builder.position = Vector2::new(0, 0);

        let mut base = snapshot_with_id(base_id.as_u64());
        base.owner = 5;
        base.base = 5;
        base.position = Vector2::new(0, 0);
        base.category = CATEGORY_STRUCTURE;
        base.ocf = ocf::AVAILABLE | ocf::ENTRANCE;
        base.collectible = false;

        let mut objects = HashMap::new();
        objects.insert(builder.id, builder.clone());
        objects.insert(base.id, base.clone());

        let mut players: HashMap<i32, CommandPlayerSnapshot> = HashMap::new();
        players.insert(
            5,
            CommandPlayerSnapshot {
                status: PlayerStatus::Active,
                surrendered: false,
                wealth: 0,
                home_base_material: HashMap::new(),
                home_base_material_entries: Vec::new(),
                knowledge: Vec::new(),
                hostile_to: Vec::new(),
            },
        );

        let definitions: HashMap<DefinitionId, CommandDefinitionSnapshot> = HashMap::new();

        let ctx = CommandRuntimeContext {
            position: builder.position,
            base_sell_enabled: false,
            ..command_ctx_at_frame(
                objects.get(&builder_id).expect("builder present"),
                &objects,
                &players,
                &definitions,
                0,
            )
        };

        let mut state = SellState::from_request(
            &CommandRequest::new(CommandId::Sell)
                .with_target(Some(base_id))
                .with_data(CommandData::Text("ORE1".into())),
        )
        .expect("state created");

        let result = state.step(&ctx);
        assert_eq!(result.status, CommandStatus::Failed);
    }

    #[test]
    fn buy_checks_stock_before_requesting_entry() {
        let builder_id = ObjectId::new(1);
        let target_id = ObjectId::new(2);
        let item_id = ObjectId::new(3);

        let mut builder = snapshot_with_id(builder_id.as_u64());
        builder.owner = 7;
        builder.position = Vector2::new(8, 0);
        builder.command_direction = CommandDirection::Left;

        let mut target = snapshot_with_id(target_id.as_u64());
        target.base = 7;
        target.position = Vector2::new(0, 0);
        target.category = CATEGORY_STRUCTURE;
        target.ocf = ocf::AVAILABLE | ocf::ENTRANCE;
        target.collectible = false;
        target.contents.push(item_id);

        let mut item = snapshot_with_id(item_id.as_u64());
        item.definition_id = "WOOD".into();
        item.collectible = true;
        item.construction = FULL_CON;
        item.container = Some(target_id);
        item.position = target.position;

        let mut objects = HashMap::new();
        objects.insert(builder.id, builder.clone());
        objects.insert(target.id, target);
        objects.insert(item.id, item);

        let mut players: HashMap<i32, CommandPlayerSnapshot> = HashMap::new();
        players.insert(
            7,
            CommandPlayerSnapshot {
                status: PlayerStatus::Active,
                surrendered: false,
                wealth: 50,
                home_base_material: HashMap::new(),
                home_base_material_entries: Vec::new(),
                knowledge: Vec::new(),
                hostile_to: Vec::new(),
            },
        );

        let mut definitions: HashMap<DefinitionId, CommandDefinitionSnapshot> = HashMap::new();
        definitions.insert(
            "WOOD".to_string(),
            CommandDefinitionSnapshot {
                value: 5,
                can_chop: false,
                chop_action: None,
                constructable: false,
                grab: 0,
                ..CommandDefinitionSnapshot::default()
            },
        );

        let ctx = CommandRuntimeContext {
            position: builder.position,
            ..command_ctx_at_frame(
                objects.get(&builder_id).expect("builder present"),
                &objects,
                &players,
                &definitions,
                0,
            )
        };

        let mut state = BuyState::from_request(
            &CommandRequest::new(CommandId::Buy)
                .with_target(Some(target_id))
                .with_data(CommandData::Text("WOOD".into())),
        )
        .expect("state created");

        let result = state.step(&ctx);
        assert_eq!(result.status, CommandStatus::Failed);
        assert!(result.update.is_none());
        assert!(result.operations.is_empty());
        assert!(result.events.is_empty());
    }

    #[test]
    fn buy_does_not_transfer_matching_target_contents() {
        let builder_id = ObjectId::new(1);
        let target_id = ObjectId::new(2);
        let item_id = ObjectId::new(3);

        let mut builder = snapshot_with_id(builder_id.as_u64());
        builder.owner = 5;
        builder.position = Vector2::new(0, 0);
        builder.command_direction = CommandDirection::Stop;
        builder.container = Some(target_id);

        let mut target = snapshot_with_id(target_id.as_u64());
        target.base = 5;
        target.position = Vector2::new(0, 0);
        target.category = CATEGORY_STRUCTURE;
        target.ocf = ocf::AVAILABLE | ocf::ENTRANCE;
        target.collectible = false;
        target.contents.push(item_id);

        let mut item = snapshot_with_id(item_id.as_u64());
        item.definition_id = "WOOD".into();
        item.collectible = true;
        item.construction = FULL_CON;
        item.container = Some(target_id);
        item.position = target.position;

        let mut objects = HashMap::new();
        objects.insert(builder.id, builder.clone());
        objects.insert(target.id, target);
        objects.insert(item.id, item);

        let mut players: HashMap<i32, CommandPlayerSnapshot> = HashMap::new();
        players.insert(
            5,
            CommandPlayerSnapshot {
                status: PlayerStatus::Active,
                surrendered: false,
                wealth: 40,
                home_base_material: HashMap::new(),
                home_base_material_entries: Vec::new(),
                knowledge: Vec::new(),
                hostile_to: Vec::new(),
            },
        );

        let mut definitions: HashMap<DefinitionId, CommandDefinitionSnapshot> = HashMap::new();
        definitions.insert(
            "WOOD".to_string(),
            CommandDefinitionSnapshot {
                value: 15,
                can_chop: false,
                chop_action: None,
                constructable: false,
                grab: 0,
                ..CommandDefinitionSnapshot::default()
            },
        );

        let ctx = CommandRuntimeContext {
            position: builder.position,
            ..command_ctx_at_frame(
                objects.get(&builder_id).expect("builder present"),
                &objects,
                &players,
                &definitions,
                10,
            )
        };

        let mut state = BuyState::from_request(
            &CommandRequest::new(CommandId::Buy)
                .with_target(Some(target_id))
                .with_data(CommandData::Text("WOOD".into())),
        )
        .expect("state created");

        let result = state.step(&ctx);
        assert_eq!(result.status, CommandStatus::Failed);
        assert!(result.operations.is_empty());
        assert!(result.update.is_none());
        assert!(result.events.is_empty());
    }

    #[test]
    fn chop_at_range_waits_for_walk_then_starts_nonforced_like_cpp() {
        // C4Command::Chop checks Target->At(cObj->x,cObj->y), plus the
        // horizontal 4..9 range, before calling ObjectComChop. ObjectComChop
        // only starts a non-forced Chop action while walking. The real TRE2
        // shape reaches 28px vertically and trees are nonliving objects
        // (C4Command.cpp:778-812; C4ObjectCom.cpp:162-165, 678-688).
        let builder_id = ObjectId::new(1);
        let target_id = ObjectId::new(2);

        let mut builder = snapshot_with_id(builder_id.as_u64());
        builder.position = Vector2::new(6, 14);
        builder.physical.can_chop = 1;
        builder.action_name = "Swim".into();
        builder.action_procedure = ActionProcedure::Swim;
        builder.command_direction = CommandDirection::Right;
        builder.fixed_velocity = FixedVec2::from_ints(3, -2);
        let builder_definition = builder.definition_id.clone();

        let mut target = snapshot_with_id(target_id.as_u64());
        target.position = Vector2::new(0, 0);
        target.shape = DefinitionRect::new(-20, -28, 40, 56);
        target.ocf = ocf::CHOP | ocf::AVAILABLE;
        target.alive = false;

        let mut objects = HashMap::new();
        objects.insert(builder.id, builder);
        objects.insert(target.id, target);

        let players: HashMap<i32, CommandPlayerSnapshot> = HashMap::new();
        let mut definitions: HashMap<DefinitionId, CommandDefinitionSnapshot> = HashMap::new();
        definitions.insert(
            builder_definition.clone(),
            CommandDefinitionSnapshot {
                value: 0,
                can_chop: true,
                chop_action: Some("Chop".into()),
                constructable: false,
                grab: 0,
                ..CommandDefinitionSnapshot::default()
            },
        );

        let mut state = ChopState::from_request(
            &CommandRequest::new(CommandId::Chop).with_target(Some(target_id)),
        )
        .expect("state created");

        {
            let builder_entry = objects.get(&builder_id).expect("builder present");
            let ctx = command_ctx_at_frame(builder_entry, &objects, &players, &definitions, 0);

            let result = state.step(&ctx);
            assert_eq!(result.status, CommandStatus::Running);
            assert!(result.operations.is_empty());
            assert!(result.events.is_empty());

            let update = result.update.expect("expected stop update while swimming");
            assert_eq!(update.command_direction, Some(CommandDirection::Stop));
            assert!(update.action.is_none());
            assert!(update.velocity.is_none());
            assert!(update.fixed_velocity.is_none());
        }

        let builder = objects.get_mut(&builder_id).expect("builder present");
        builder.action_name = "Walk".into();
        builder.action_procedure = ActionProcedure::Walk;
        builder.command_direction = CommandDirection::Stop;

        let builder_entry = objects.get(&builder_id).expect("builder present");
        let ctx = command_ctx_at_frame(builder_entry, &objects, &players, &definitions, 1);

        let result = state.step(&ctx);
        assert_eq!(result.status, CommandStatus::Running);
        assert!(result.operations.is_empty());
        assert!(result.events.is_empty());

        let update = result.update.expect("expected Chop update while walking");
        assert!(update.command_direction.is_none());
        assert!(update.velocity.is_none());
        assert!(update.fixed_velocity.is_none());
        let action_update = update.action.expect("action update");
        assert_eq!(action_update.name, Some("Chop".into()));
        assert_eq!(action_update.target, Some(Some(target_id)));
        assert_eq!(action_update.phase, Some(0));
        assert_eq!(action_update.ticks, Some(0));
        assert!(!action_update.force);
    }

    #[test]
    fn chop_requests_move_when_far() {
        let builder_id = ObjectId::new(1);
        let target_id = ObjectId::new(2);

        let mut builder = snapshot_with_id(builder_id.as_u64());
        builder.position = Vector2::new(30, 0);
        builder.physical.can_chop = 1;
        let builder_definition = builder.definition_id.clone();

        let mut target = snapshot_with_id(target_id.as_u64());
        target.position = Vector2::new(0, 0);
        target.ocf = ocf::CHOP | ocf::AVAILABLE;

        let mut objects = HashMap::new();
        objects.insert(builder.id, builder);
        objects.insert(target.id, target);

        let players: HashMap<i32, CommandPlayerSnapshot> = HashMap::new();
        let mut definitions: HashMap<DefinitionId, CommandDefinitionSnapshot> = HashMap::new();
        definitions.insert(
            builder_definition,
            CommandDefinitionSnapshot {
                value: 0,
                can_chop: true,
                chop_action: Some("Chop".into()),
                constructable: false,
                grab: 0,
                ..CommandDefinitionSnapshot::default()
            },
        );

        let builder_entry = objects.get(&builder_id).expect("builder present");
        let ctx = command_ctx_at_frame(builder_entry, &objects, &players, &definitions, 0);

        let mut state = ChopState::from_request(
            &CommandRequest::new(CommandId::Chop).with_target(Some(target_id)),
        )
        .expect("state created");

        let result = state.step(&ctx);
        assert_eq!(result.status, CommandStatus::Running);
        assert!(result.update.is_none(), "approach must preserve ComDir");
        assert!(!result.operations.is_empty());
        match &result.operations[0] {
            CommandOperation::PushFront(request) => assert_eq!(request.id, CommandId::MoveTo),
            other => panic!("unexpected operation: {:?}", other),
        }
        let first_move = pushed_request(&result.operations, CommandId::MoveTo);
        let reissued = state.step(&ctx);
        assert_eq!(
            pushed_request(&reissued.operations, CommandId::MoveTo),
            first_move,
            "Chop reissues MoveTo on its next execution"
        );
    }

    #[test]
    fn chop_uses_physical_capability_and_moves_away_at_four_pixel_boundary() {
        // C4Command::Chop reads GetPhysical()->CanChop independently of the
        // ActMap. Outside Target::At, |dx| == 4 queues the ordinary +/-6
        // approach and then the immediate +/-15 move-away command because
        // the latter threshold is strictly less than 5 (C4Command.cpp:783-819).
        let builder_id = ObjectId::new(1);
        let target_id = ObjectId::new(2);

        let mut builder = snapshot_with_id(builder_id.as_u64());
        builder.position = Vector2::new(4, 0);
        builder.physical.can_chop = 1;
        let builder_definition = builder.definition_id.clone();

        let mut target = snapshot_with_id(target_id.as_u64());
        target.position = Vector2::ZERO;
        target.shape = DefinitionRect::new(-3, -3, 6, 6);
        target.ocf = ocf::CHOP | ocf::AVAILABLE;

        let objects = HashMap::from([(builder_id, builder), (target_id, target)]);
        let players = HashMap::new();
        let definitions = HashMap::from([(
            builder_definition,
            CommandDefinitionSnapshot {
                can_chop: false,
                chop_action: None,
                ..CommandDefinitionSnapshot::default()
            },
        )]);
        let builder = objects.get(&builder_id).expect("builder present");
        let ctx = command_ctx_at_frame(builder, &objects, &players, &definitions, 0);
        let mut state = ChopState::from_request(
            &CommandRequest::new(CommandId::Chop).with_target(Some(target_id)),
        )
        .expect("state created");

        let result = state.step(&ctx);
        assert_eq!(result.status, CommandStatus::Running);
        assert_eq!(result.operations.len(), 2);
        match result.operations.as_slice() {
            [CommandOperation::PushFront(approach), CommandOperation::PushFront(move_away)] => {
                assert_eq!(approach.id, CommandId::MoveTo);
                assert_eq!((approach.tx, approach.ty), (Some(6), Some(0)));
                assert_eq!(approach.update_interval, 50);
                assert_eq!(move_away.id, CommandId::MoveTo);
                assert_eq!((move_away.tx, move_away.ty), (Some(15), Some(0)));
                assert_eq!(move_away.update_interval, 50);
            }
            other => panic!("expected approach then move-away requests, got {other:?}"),
        }
    }

    #[test]
    fn chop_requests_ungrab_when_pushing() {
        let builder_id = ObjectId::new(1);
        let target_id = ObjectId::new(2);

        let mut builder = snapshot_with_id(builder_id.as_u64());
        builder.position = Vector2::new(30, 0);
        builder.physical.can_chop = 1;
        builder.action_procedure = ActionProcedure::Push;
        builder.action_target = Some(ObjectId::new(99));
        let builder_definition = builder.definition_id.clone();

        let mut target = snapshot_with_id(target_id.as_u64());
        target.position = Vector2::new(0, 0);
        target.ocf = ocf::CHOP | ocf::AVAILABLE;

        let mut objects = HashMap::new();
        objects.insert(builder.id, builder);
        objects.insert(target.id, target);

        let players: HashMap<i32, CommandPlayerSnapshot> = HashMap::new();
        let mut definitions: HashMap<DefinitionId, CommandDefinitionSnapshot> = HashMap::new();
        definitions.insert(
            builder_definition,
            CommandDefinitionSnapshot {
                value: 0,
                can_chop: true,
                chop_action: Some("Chop".into()),
                constructable: false,
                grab: 0,
                ..CommandDefinitionSnapshot::default()
            },
        );

        let builder_entry = objects.get(&builder_id).expect("builder present");
        let ctx = command_ctx_at_frame(builder_entry, &objects, &players, &definitions, 0);

        let mut state = ChopState::from_request(
            &CommandRequest::new(CommandId::Chop).with_target(Some(target_id)),
        )
        .expect("state created");

        let result = state.step(&ctx);
        assert_eq!(result.status, CommandStatus::Running);
        assert!(result.update.is_none(), "Push branch must preserve ComDir");
        assert_eq!(result.operations.len(), 1);
        match &result.operations[0] {
            CommandOperation::PushFront(request) => assert_eq!(request.id, CommandId::UnGrab),
            other => panic!("unexpected operation: {:?}", other),
        }
    }

    #[test]
    fn chop_completes_when_target_not_choppable() {
        let builder_id = ObjectId::new(1);
        let target_id = ObjectId::new(2);

        let mut builder = snapshot_with_id(builder_id.as_u64());
        builder.position = Vector2::new(6, 0);
        builder.physical.can_chop = 1;
        let builder_definition = builder.definition_id.clone();

        let mut target = snapshot_with_id(target_id.as_u64());
        target.position = Vector2::new(0, 0);
        target.ocf = ocf::AVAILABLE;

        let mut objects = HashMap::new();
        objects.insert(builder.id, builder);
        objects.insert(target.id, target);

        let players: HashMap<i32, CommandPlayerSnapshot> = HashMap::new();
        let mut definitions: HashMap<DefinitionId, CommandDefinitionSnapshot> = HashMap::new();
        definitions.insert(
            builder_definition,
            CommandDefinitionSnapshot {
                value: 0,
                can_chop: true,
                chop_action: Some("Chop".into()),
                constructable: false,
                grab: 0,
                ..CommandDefinitionSnapshot::default()
            },
        );

        let builder_entry = objects.get(&builder_id).expect("builder present");
        let ctx = command_ctx_at_frame(builder_entry, &objects, &players, &definitions, 0);

        let mut state = ChopState::from_request(
            &CommandRequest::new(CommandId::Chop).with_target(Some(target_id)),
        )
        .expect("state created");

        let result = state.step(&ctx);
        assert_eq!(result.status, CommandStatus::Completed);
        assert!(
            result.update.is_none(),
            "non-choppable completion must preserve ComDir"
        );
    }

    #[test]
    fn chop_fails_when_physical_can_chop_is_zero_despite_chop_action() {
        let builder_id = ObjectId::new(1);
        let target_id = ObjectId::new(2);

        let mut builder = snapshot_with_id(builder_id.as_u64());
        builder.position = Vector2::new(10, 0);
        builder.physical.can_chop = 0;
        let builder_definition = builder.definition_id.clone();

        let mut target = snapshot_with_id(target_id.as_u64());
        target.position = Vector2::new(0, 0);
        target.ocf = ocf::CHOP | ocf::AVAILABLE;

        let mut objects = HashMap::new();
        objects.insert(builder.id, builder);
        objects.insert(target.id, target);

        let players: HashMap<i32, CommandPlayerSnapshot> = HashMap::new();
        let mut definitions: HashMap<DefinitionId, CommandDefinitionSnapshot> = HashMap::new();
        definitions.insert(
            builder_definition,
            CommandDefinitionSnapshot {
                value: 0,
                can_chop: true,
                chop_action: Some("Chop".into()),
                constructable: false,
                grab: 0,
                ..CommandDefinitionSnapshot::default()
            },
        );

        let builder_entry = objects.get(&builder_id).expect("builder present");
        let ctx = command_ctx_at_frame(builder_entry, &objects, &players, &definitions, 0);

        let mut state = ChopState::from_request(
            &CommandRequest::new(CommandId::Chop).with_target(Some(target_id)),
        )
        .expect("state created");

        let result = state.step(&ctx);
        assert_eq!(result.status, CommandStatus::Failed);
    }

    #[test]
    fn legacy_compiled_command_restores_live_fields_and_executes() {
        let saved = LegacyCommandSave {
            view: CommandView {
                name: "MoveTo".into(),
                target: None,
                tx: Some(100),
                tx_value: Some(clonk_script::Value::Int(100)),
                tx_definition: None,
                ty: Some(100),
                target2: None,
                data: CommandData::Integer(0),
                legacy_data: None,
                finished: false,
            },
            update_interval: -4,
            evaluated: -2,
            path_checked: 7,
            finished: 0,
            failures: 0,
            retries: 3,
            permit: -4,
            base_mode: 99,
            text: "unused-but-persisted".into(),
        };
        let snapshot = CommandStackSnapshot::from_legacy_save_commands(vec![saved.clone()])
            .expect("legacy command compiles");
        let projected = snapshot.legacy_save_commands();
        assert_eq!(projected, [saved]);

        let mut stack = CommandStack::new();
        stack.restore_from_snapshot(&snapshot);
        let mut actor = snapshot_with_id(1);
        actor.position = Vector2::new(100, 100);
        let objects = HashMap::new();
        let players = HashMap::new();
        let definitions = HashMap::new();
        let ctx = command_ctx_at_frame(&actor, &objects, &players, &definitions, 1);
        let result = stack.execute_front(&ctx).expect("restored MoveTo executes");
        assert_eq!(result.status, CommandStatus::Completed);
        assert_eq!(stack.legacy_save_commands()[0].update_interval, -4);
    }
