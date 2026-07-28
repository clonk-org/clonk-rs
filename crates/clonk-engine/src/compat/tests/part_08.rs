// Contiguous slice 8 of 11 of the `compat::tests` battery, spliced by
// `include!` from compat.rs so every test id stays `compat::tests::*`.
// Mostly: object state, commands, effects.

    #[test]
    fn get_contact_and_get_vertex_contact_honor_live_contact_density() {
        let target_id = ObjectId::new(1);
        let vertices = [ObjectVertex::new(0, 0).with_cnat(CNAT_CENTER)];
        let landscape = Landscape::flat(4, 0);
        let target = fixture_world_object(target_id, "TARG")
            .with_energy(0)
            .with_vertices(vertices.to_vec())
        .with_contact_density(crate::CONTACT_DENSITY_SOLID + 1);
        let world = world_with(vec![target], Some(landscape), HashMap::new(), HashMap::new());
        let target = object_reference_value(target_id);
        let (result, _) = with_effect_context(None, &[], world, 1, || {
            Ok::<_, RuntimeError>(Value::Array(vec![
                get_contact(std::slice::from_ref(&target))?,
                get_vertex_contact(&[Value::Int(0), Value::Nil, target])?,
            ]))
        });

        assert_eq!(
            result.expect("contact queries succeed"),
            Value::Array(vec![Value::Int(0), Value::Int(0)])
        );
    }

    #[test]
    fn set_action_targets_records_target_updates() {
        let mut target_map = ValueMap::new();
        target_map.insert("id".into(), Value::Int(42));

        let (result, outcome) =
            with_object_host_context(|| set_action_targets(&[Value::Proplist(target_map.clone())]));

        let value = result.expect("SetActionTargets succeeds");
        assert_eq!(value, Value::Bool(true));
        let update = outcome.object_update.expect("object update recorded");
        let action = update.action.expect("action update exists");
        assert_eq!(
            action.target,
            Some(Some(ObjectId::new(42))),
            "target update recorded",
        );
        // FnSetActionTargets assigns BOTH slots unconditionally with the
        // nil-filled parameters (C4Script.cpp:1108-1116): a one-arg call
        // CLEARS Action.Target2.
        assert_eq!(
            action.target2,
            Some(None),
            "unfilled pTarget2 clears the second target"
        );
    }

    #[test]
    fn set_action_targets_updates_second_slot_when_provided() {
        let mut first = ValueMap::new();
        first.insert("id".into(), Value::Int(5));
        let mut second = ValueMap::new();
        second.insert("id".into(), Value::Int(6));

        let (result, outcome) = with_object_host_context(|| {
            set_action_targets(&[
                Value::Proplist(first.clone()),
                Value::Proplist(second.clone()),
            ])
        });

        let value = result.expect("SetActionTargets succeeds");
        assert_eq!(value, Value::Bool(true));
        let update = outcome.object_update.expect("object update recorded");
        let action = update.action.expect("action update exists");
        assert_eq!(action.target, Some(Some(ObjectId::new(5))));
        assert_eq!(action.target2, Some(Some(ObjectId::new(6))));
    }

    fn set_action_targets_foreign_world(
        target1: Option<ObjectId>,
        target2: Option<ObjectId>,
    ) -> (ObjectId, HostWorldContext) {
        let target_id = ObjectId::new(2);
        let mut state = crate::preview_spawn_state(
            Vector2::ZERO,
            OWNER_NONE,
            OWNER_NONE,
            DEFAULT_CATEGORY,
            crate::FULL_CON,
            crate::CONTACT_DENSITY_SOLID,
            Vec::new(),
        );
        state.action.target = target1;
        state.action.target2 = target2;
        let target = fixture_world_object(target_id, "TARG")
            .with_action_target(target1)
            .with_action_target2(target2)
        .with_full_state(Rc::new(state));
        (target_id, HostWorldContext::from_objects([target]))
    }

    #[test]
    fn set_action_targets_clears_both_slots_on_a_foreign_object() {
        // FnSetActionTargets assigns both fields on an explicit pObj even
        // when it differs from cthr->Obj (C4Script.cpp:1109-1117).
        let (target_id, world) =
            set_action_targets_foreign_world(Some(ObjectId::new(41)), Some(ObjectId::new(42)));
        let target = object_reference_value(target_id);
        let (result, outcome) = with_object_host_context_with_world(world, || {
            Ok(Value::Array(vec![
                set_action_targets(&[Value::Int(0), Value::Int(0), target.clone()])?,
                get_action_target(&[Value::Int(0), target.clone()])?,
                get_action_target(&[Value::Int(1), target])?,
            ]))
        });

        assert_eq!(
            result.expect("foreign SetActionTargets succeeds"),
            Value::Array(vec![Value::Bool(true), Value::Nil, Value::Nil])
        );
        assert!(outcome.object_update.is_none(), "caller remains unchanged");
        let action = outcome
            .other_objects
            .iter()
            .find(|object| object.object_id == target_id)
            .and_then(|object| object.update.as_ref())
            .and_then(|update| update.action.as_ref())
            .expect("foreign action-target update recorded");
        assert_eq!(action.target, Some(None));
        assert_eq!(action.target2, Some(None));
    }

    #[test]
    fn set_action_targets_sets_both_slots_on_a_foreign_object() {
        let (target_id, world) = set_action_targets_foreign_world(None, None);
        let first_id = ObjectId::new(31);
        let second_id = ObjectId::new(32);
        let target = object_reference_value(target_id);
        let first = object_reference_value(first_id);
        let second = object_reference_value(second_id);
        let (result, outcome) = with_object_host_context_with_world(world, || {
            Ok(Value::Array(vec![
                set_action_targets(&[first.clone(), second.clone(), target.clone()])?,
                get_action_target(&[Value::Int(0), target.clone()])?,
                get_action_target(&[Value::Int(1), target])?,
            ]))
        });

        assert_eq!(
            result.expect("foreign SetActionTargets succeeds"),
            Value::Array(vec![Value::Bool(true), first, second])
        );
        assert!(outcome.object_update.is_none(), "caller remains unchanged");
        let action = outcome
            .other_objects
            .iter()
            .find(|object| object.object_id == target_id)
            .and_then(|object| object.update.as_ref())
            .and_then(|update| update.action.as_ref())
            .expect("foreign action-target update recorded");
        assert_eq!(action.target, Some(Some(first_id)));
        assert_eq!(action.target2, Some(Some(second_id)));
    }

    #[test]
    fn get_action_target_reflects_pending_update() {
        let mut target_map = ValueMap::new();
        target_map.insert("id".into(), Value::Int(12));

        let (result, outcome) = with_object_host_context(|| {
            set_action_targets(&[Value::Proplist(target_map.clone())])?;
            get_action_target(&[Value::Int(0)])
        });

        let value = result.expect("GetActionTarget succeeds");
        assert_eq!(value, object_reference_value(ObjectId::new(12)));

        let update = outcome.object_update.expect("object update recorded");
        let action = update.action.expect("action update exists");
        assert_eq!(action.target, Some(Some(ObjectId::new(12))));
    }

    #[test]
    fn get_action_target_reads_world_context() {
        let other = fixture_world_object(ObjectId::new(99), "Dummy")
            .with_action_name("Walk")
            .with_action_target(Some(ObjectId::new(77)));
        let world = HostWorldContext::from_objects(vec![other]);
        let (result, _) = with_object_host_context_with_world(world, || {
            get_action_target(&[Value::Int(0), object_reference_value(ObjectId::new(99))])
        });

        let value = result.expect("GetActionTarget succeeds");
        assert_eq!(value, object_reference_value(ObjectId::new(77)));
    }

    #[test]
    fn get_action_target_returns_nil_for_out_of_range_index() {
        let value = with_object_host_context(|| get_action_target(&[Value::Int(2)]))
            .0
            .expect("GetActionTarget succeeds");
        assert_eq!(value, Value::Nil);
    }

    fn with_walking_host_context<F, T>(func: F) -> (Result<T, RuntimeError>, EffectContextOutcome)
    where
        F: FnOnce() -> Result<T, RuntimeError>,
    {
        // A two-direction active action: SetDir is gated on
        // `Action.Act > ActIdle` and `Inside(iDir, 0, Directions-1)`
        // (C4Object.cpp:4228-4230).
        let mut specs = HashMap::new();
        specs.insert(
            "Walk".to_string(),
            crate::action::ActionSpec::default().with_directions(2),
        );
        let library = ActionLibrary::new(Some("Walk".to_string()), specs);
        with_effect_context(
            Some(HostObjectContext {
                action_name: "Walk".to_string(),
                action_library: library.into(),
                ..idle_object_context()
            }),
            &[],
            HostWorldContext::default(),
            1,
            func,
        )
    }

    #[test]
    fn set_dir_records_direction_update() {
        let (result, outcome) = with_walking_host_context(|| set_dir(&[Value::Int(1)]));
        let value = result.expect("SetDir succeeds");
        assert_eq!(value, Value::Bool(true));
        let update = outcome.object_update.expect("direction update recorded");
        assert_eq!(update.direction, Some(Direction::Right));
    }

    #[test]
    fn set_dir_converts_bool_to_int_like_cpp() {
        // Native function parameters use C4Value::ConvertTo before dispatch;
        // Bool -> Int is CnvOK and preserves the shared 0/1 payload
        // (C4AulExec.cpp:1364-1396; C4Value.cpp:514-518;
        // C4Script.cpp:799-803). Epic Voltage calls SetDir(!i).
        let (result, outcome) = with_walking_host_context(|| set_dir(&[Value::Bool(true)]));
        assert_eq!(result.expect("SetDir converts bool"), Value::Bool(true));
        let update = outcome.object_update.expect("direction update recorded");
        assert_eq!(update.direction, Some(Direction::Right));
    }

    #[test]
    fn set_dir_out_of_range_returns_true_without_changing_direction() {
        // FnSetDir returns true whenever it resolves an object even when
        // C4Object::SetDir rejects iDir outside the current action's
        // Directions range (C4Script.cpp:799-804; C4Object.cpp:4235-4241).
        let (result, outcome) = with_walking_host_context(|| set_dir(&[Value::Int(13)]));
        assert_eq!(result.expect("SetDir runs"), Value::Bool(true));
        assert!(outcome
            .object_update
            .map(|update| update.direction.is_none())
            .unwrap_or(true));
    }

    fn set_phase_target_world(action_name: &str) -> (ObjectId, HostWorldContext) {
        let target_id = ObjectId::new(2);
        let mut specs = HashMap::new();
        specs.insert(
            "Walk".to_string(),
            crate::action::ActionSpec::default().with_length(5),
        );
        let action_library = ActionLibrary::new(Some("Walk".to_string()), specs);
        let mut state = crate::preview_spawn_state(
            Vector2::ZERO,
            OWNER_NONE,
            OWNER_NONE,
            DEFAULT_CATEGORY,
            crate::FULL_CON,
            crate::CONTACT_DENSITY_SOLID,
            Vec::new(),
        );
        state.action = ActionState::new(action_name);
        let target = fixture_world_object(target_id, "TARG")
            .with_action_name(action_name)
        .with_full_state(Rc::new(state));
        let world = HostWorldContext::from_objects([target]).with_definition_metadata(Rc::new(
            HashMap::from([(
                DefinitionId::from("TARG"),
                DefinitionMetadata {
                    action_library: action_library.into(),
                    ..DefinitionMetadata::default()
                },
            )]),
        ));
        (target_id, world)
    }

    #[test]
    fn set_phase_targets_a_foreign_object_like_cpp() {
        let (target_id, world) = set_phase_target_world("Walk");
        let target = object_reference_value(target_id);
        let (result, outcome) = with_object_host_context_with_world(world, || {
            Ok(Value::Array(vec![
                set_phase(&[Value::Int(3), target.clone()])?,
                get_phase(&[target])?,
            ]))
        });

        assert_eq!(
            result.expect("foreign SetPhase succeeds"),
            Value::Array(vec![Value::Bool(true), Value::Int(3)])
        );
        assert!(outcome.object_update.is_none(), "caller remains unchanged");
        let action = outcome
            .other_objects
            .iter()
            .find(|outcome| outcome.object_id == target_id)
            .and_then(|outcome| outcome.update.as_ref())
            .and_then(|update| update.action.as_ref())
            .expect("foreign phase update recorded");
        assert_eq!(action.phase, Some(3));
    }

    #[test]
    fn set_phase_targets_an_object_from_scenario_scope() {
        let (target_id, world) = set_phase_target_world("Walk");
        let target = object_reference_value(target_id);
        let (result, outcome) = with_effect_context(None, &[], world, 1, || {
            Ok::<Value, RuntimeError>(Value::Array(vec![
                set_phase(&[Value::Int(2), target.clone()])?,
                get_phase(&[target])?,
            ]))
        });

        assert_eq!(
            result.expect("scenario SetPhase succeeds"),
            Value::Array(vec![Value::Bool(true), Value::Int(2)])
        );
        let action = outcome
            .other_objects
            .iter()
            .find(|outcome| outcome.object_id == target_id)
            .and_then(|outcome| outcome.update.as_ref())
            .and_then(|update| update.action.as_ref())
            .expect("scenario phase update recorded");
        assert_eq!(action.phase, Some(2));
    }

    #[test]
    fn set_phase_clamps_to_action_length_inclusive_like_cpp() {
        // C4Object::SetPhase (C4Object.cpp:2205-2211): idle → false;
        // BoundBy(iPhase, 0, Length) — INCLUSIVE of Length.
        let mut specs = HashMap::new();
        specs.insert(
            "Walk".to_string(),
            crate::action::ActionSpec::default().with_length(5),
        );
        let library = ActionLibrary::new(Some("Walk".to_string()), specs);
        let (result, outcome) = with_effect_context(
            Some(HostObjectContext {
                action_name: "Walk".to_string(),
                action_library: library.into(),
                ..idle_object_context()
            }),
            &[],
            HostWorldContext::default(),
            1,
            || {
                Ok::<Value, RuntimeError>(Value::Array(vec![
                    set_phase(&[Value::Int(3)])?,
                    get_phase(&[])?,
                    set_phase(&[Value::Int(-4)])?,
                    get_phase(&[])?,
                    set_phase(&[Value::Int(9)])?,
                    get_phase(&[])?,
                ]))
            },
        );
        assert_eq!(
            result.expect("SetPhase runs"),
            Value::Array(vec![
                Value::Bool(true),
                Value::Int(3),
                Value::Bool(true),
                Value::Int(0),
                Value::Bool(true),
                Value::Int(5),
            ])
        );
        let update = outcome.object_update.expect("phase update recorded");
        let action = update.action.expect("action update present");
        assert_eq!(action.phase, Some(5), "9 clamps to Length (5), inclusive");
    }

    #[test]
    fn set_phase_preserves_cpp_bound_by_order_for_negative_length() {
        let library = ActionLibrary::new(
            Some("Odd".to_string()),
            HashMap::from([(
                "Odd".to_string(),
                crate::action::ActionSpec::default().with_length(-3),
            )]),
        );
        let (result, outcome) = with_effect_context(
            Some(HostObjectContext {
                action_name: "Odd".to_string(),
                action_library: library.into(),
                ..idle_object_context()
            }),
            &[],
            HostWorldContext::default(),
            1,
            || {
                Ok::<Value, RuntimeError>(Value::Array(vec![
                    set_phase(&[Value::Int(-4)])?,
                    get_phase(&[])?,
                    set_phase(&[Value::Int(9)])?,
                    get_phase(&[])?,
                ]))
            },
        );

        assert_eq!(
            result.expect("SetPhase runs"),
            Value::Array(vec![
                Value::Bool(true),
                Value::Int(0),
                Value::Bool(true),
                Value::Int(-3),
            ])
        );
        assert_eq!(
            outcome
                .object_update
                .and_then(|update| update.action)
                .and_then(|action| action.phase),
            Some(-3)
        );
    }

    #[test]
    fn set_phase_rejects_an_idle_foreign_target() {
        let (target_id, world) = set_phase_target_world("Idle");
        let (result, outcome) = with_effect_context(None, &[], world, 1, || {
            set_phase(&[Value::Int(2), object_reference_value(target_id)])
        });
        assert_eq!(
            result.expect("idle SetPhase returns bool"),
            Value::Bool(false)
        );
        assert!(outcome.object_update.is_none());
        assert!(outcome.other_objects.iter().all(|outcome| {
            outcome
                .update
                .as_ref()
                .and_then(|update| update.action.as_ref())
                .is_none()
        }));
    }

    #[test]
    fn set_dir_is_a_no_op_on_idle_objects() {
        // C4Object::SetDir bails on `Action.Act <= ActIdle`
        // (C4Object.cpp:4238), but FnSetDir still returns true for the object
        // (C4Script.cpp:799-804).
        let (result, outcome) = with_object_host_context(|| set_dir(&[Value::Int(1)]));
        assert_eq!(result.expect("SetDir runs"), Value::Bool(true));
        assert!(outcome
            .object_update
            .map(|update| update.direction.is_none())
            .unwrap_or(true));
    }

    #[test]
    fn get_dir_observes_effective_direction() {
        let (result, outcome) = with_walking_host_context(|| {
            set_dir(&[Value::Int(1)])?;
            get_dir(&[])
        });
        let value = result.expect("GetDir succeeds");
        assert_eq!(value, Value::Int(Direction::Right.to_script_value()));
        let update = outcome.object_update.expect("direction update recorded");
        assert_eq!(update.direction, Some(Direction::Right));
    }

    #[test]
    fn set_com_dir_records_command_direction_update() {
        let (result, outcome) = with_object_host_context(|| set_com_dir(&[Value::Int(3)]));
        let value = result.expect("SetComDir succeeds");
        assert_eq!(value, Value::Bool(true));
        let update = outcome
            .object_update
            .expect("command direction update recorded");
        assert_eq!(update.command_direction, Some(CommandDirection::Right));
    }

    #[test]
    fn set_com_dir_preserves_raw_int32_like_cpp() {
        // FnSetComDir writes ncomdir directly without validating the COMD_*
        // ring (C4Script.cpp:792-796).
        let (result, outcome) = with_object_host_context(|| set_com_dir(&[Value::Int(200)]));
        assert_eq!(result.expect("SetComDir succeeds"), Value::Bool(true));
        assert_eq!(
            outcome
                .object_update
                .and_then(|update| update.command_direction)
                .map(CommandDirection::to_script_value),
            Some(200)
        );
    }

    #[test]
    fn set_com_dir_targets_a_foreign_object_like_cpp() {
        // FnSetComDir defaults a nil pObj to cthr->Obj, but an explicit pObj
        // receives Action.ComDir directly (C4Script.cpp:792-796). Tutorial
        // machinery such as a derrick therefore controls another object.
        let target_id = ObjectId::new(2);
        let target = fixture_world_object(target_id, "CLNK")
            .with_action_name("Walk")
            .with_energy(0)
        .with_full_state(Rc::new(crate::preview_spawn_state(
            Vector2::ZERO,
            OWNER_NONE,
            OWNER_NONE,
            DEFAULT_CATEGORY,
            crate::FULL_CON,
            crate::CONTACT_DENSITY_SOLID,
            Vec::new(),
        )));
        let world = HostWorldContext::from_objects(vec![target]).with_definition_metadata(Rc::new(
            HashMap::from([(DefinitionId::from("CLNK"), DefinitionMetadata::default())]),
        ));

        let (result, outcome) = with_object_host_context_with_world(world, || {
            set_com_dir(&[
                Value::Int(CommandDirection::Down.to_script_value()),
                object_reference_value(target_id),
            ])
        });

        assert_eq!(result.expect("SetComDir succeeds"), Value::Bool(true));
        let update = outcome
            .other_objects
            .iter()
            .find(|outcome| outcome.object_id == target_id)
            .and_then(|outcome| outcome.update.as_ref())
            .expect("foreign command-direction update recorded");
        assert_eq!(update.command_direction, Some(CommandDirection::Down));
        assert!(outcome.object_update.is_none(), "caller remains unchanged");
    }

    fn world_object_with_command_direction(
        id: ObjectId,
        command_direction: CommandDirection,
    ) -> HostWorldObject {
        let mut state = crate::preview_spawn_state(
            Vector2::ZERO,
            OWNER_NONE,
            OWNER_NONE,
            DEFAULT_CATEGORY,
            crate::FULL_CON,
            crate::CONTACT_DENSITY_SOLID,
            Vec::new(),
        );
        state.command_direction = command_direction;
        fixture_world_object(id, "CLNK")
            .with_action_name("Walk")
            .with_energy(0)
        .with_full_state(Rc::new(state))
    }

    #[test]
    fn get_com_dir_reads_explicit_world_object_without_object_context() {
        let target_id = ObjectId::new(2);
        let world = HostWorldContext::from_objects([world_object_with_command_direction(
            target_id,
            CommandDirection::UpLeft,
        )]);

        let (result, _) = with_effect_context(None, &[], world, 1, || {
            get_com_dir(&[object_reference_value(target_id)])
        });

        assert_eq!(
            result.expect("GetComDir succeeds from a definition context"),
            Value::Int(CommandDirection::UpLeft.to_script_value())
        );
    }

    #[test]
    fn get_com_dir_reads_same_call_staged_foreign_direction() {
        let target_id = ObjectId::new(2);
        let world = HostWorldContext::from_objects([world_object_with_command_direction(
            target_id,
            CommandDirection::Right,
        )]);

        let (result, outcome) = with_object_host_context_with_world(world, || {
            let set_result = set_com_dir(&[
                Value::Int(CommandDirection::DownLeft.to_script_value()),
                object_reference_value(target_id),
            ])?;
            Ok(Value::Array(vec![
                set_result,
                get_com_dir(&[object_reference_value(target_id)])?,
                get_com_dir(&[])?,
            ]))
        });

        assert_eq!(
            result.expect("foreign SetComDir/GetComDir sequence succeeds"),
            Value::Array(vec![
                Value::Bool(true),
                Value::Int(CommandDirection::DownLeft.to_script_value()),
                Value::Int(CommandDirection::Stop.to_script_value()),
            ]),
            "the staged foreign write is visible without changing local no-arg lookup"
        );
        assert!(outcome.object_update.is_none(), "caller remains unchanged");
    }

    #[test]
    fn get_com_dir_observes_effective_command_direction() {
        let (result, outcome) = with_object_host_context(|| {
            set_com_dir(&[Value::Int(4)])?;
            get_com_dir(&[])
        });
        let value = result.expect("GetComDir succeeds");
        assert_eq!(
            value,
            Value::Int(CommandDirection::DownRight.to_script_value())
        );
        let update = outcome
            .object_update
            .expect("command direction update recorded");
        assert_eq!(update.command_direction, Some(CommandDirection::DownRight));
    }

    #[test]
    fn set_command_clears_stack_and_pushes_command() {
        let args = vec![
            Value::String("MoveTo".into()),
            Value::Nil,
            Value::Int(10),
            Value::Int(15),
        ];
        let (result, outcome) = with_object_host_context(|| set_command(&args));
        let value = result.expect("SetCommand succeeds");
        assert_eq!(value, Value::Bool(true));
        // C4Object::SetCommand: NoCollectDelay decrement (C4Object.cpp:
        // 3941-3942), ClearCommands (:3943), then the push.
        assert_eq!(outcome.command_operations.len(), 3);
        match &outcome.command_operations[0] {
            CommandOperation::DecrementNoCollectDelay => {}
            other => panic!("expected decrement operation, got {:?}", other),
        }
        match &outcome.command_operations[1] {
            CommandOperation::Clear => {}
            other => panic!("expected Clear operation, got {:?}", other),
        }
        match &outcome.command_operations[2] {
            CommandOperation::PushFront(request) => {
                assert_eq!(request.id, CommandId::MoveTo);
                assert_eq!(request.tx, Some(10));
                assert_eq!(request.ty, Some(15));
            }
            other => panic!("expected PushFront operation, got {:?}", other),
        }
    }

    #[test]
    fn add_command_pushes_front_without_clearing() {
        let args = vec![
            Value::String("MoveTo".into()),
            Value::Nil,
            Value::Int(5),
            Value::Int(8),
        ];
        let (result, outcome) = with_object_host_context(|| add_command(&args));
        let value = result.expect("AddCommand succeeds");
        assert_eq!(value, Value::Bool(true));
        assert_eq!(outcome.command_operations.len(), 1);
        match &outcome.command_operations[0] {
            CommandOperation::PushFront(request) => {
                assert_eq!(request.id, CommandId::MoveTo);
                assert_eq!(request.tx, Some(5));
                assert_eq!(request.ty, Some(8));
            }
            other => panic!("expected PushFront operation, got {:?}", other),
        }
    }

    #[test]
    fn add_command_preserves_a_negative_update_interval() {
        // C4ValueInt and C4Command::UpdateInterval are signed. C++ stores
        // this word verbatim; Execute only decrements values greater than
        // zero (C4Script.cpp:871-892; C4Command.cpp:1545-1552).
        let args = vec![
            Value::String("Wait".into()),
            Value::Nil,
            Value::Int(0),
            Value::Int(0),
            Value::Nil,
            Value::Int(-4),
        ];
        let (result, outcome) = with_object_host_context(|| add_command(&args));

        assert_eq!(result.expect("AddCommand succeeds"), Value::Bool(true));
        match &outcome.command_operations[0] {
            CommandOperation::PushFront(request) => {
                assert_eq!(request.id, CommandId::Wait);
                assert_eq!(request.update_interval, -4);
            }
            other => panic!("expected PushFront operation, got {other:?}"),
        }
    }

    #[test]
    fn finish_command_rejects_missing_indices_and_uses_cpp_bool_coercion() {
        let (result, outcome) = with_object_host_context(|| {
            assert_eq!(
                add_command(&[Value::Int(0), Value::String("Wait".into())])?,
                Value::Bool(true)
            );
            Ok(Value::Array(vec![
                finish_command(&[Value::Int(0), Value::Bool(true), Value::Int(5)])?,
                finish_command(&[Value::Int(0), Value::Bool(true), Value::Int(-1)])?,
                finish_command(&[Value::Int(0), Value::Int(2)])?,
            ]))
        });

        assert_eq!(
            result.expect("FinishCommand calls succeed"),
            Value::Array(vec![
                Value::Bool(false),
                Value::Bool(false),
                Value::Bool(true),
            ])
        );
        assert!(matches!(
            outcome.command_operations.as_slice(),
            [
                CommandOperation::PushFront(request),
                CommandOperation::Finish {
                    index: 0,
                    success: true
                }
            ] if request.id == CommandId::Wait
        ));
    }

    #[test]
    #[allow(clippy::arc_with_non_send_sync)] // HostWorldContext definition scripts are shared read-only by the fixture.
    fn add_command_targets_a_foreign_worker_like_cpp() {
        // FnAddCommand queues directly on its explicit pObj
        // (C4Script.cpp:871-892). Workshop::SelectProduction invokes it
        // from WRKS script context with the CLNK worker as pObj
        // (Objects.c4d/Structures.c4d/Workshop.c4d/Script.c:68-72).
        let worker_id = ObjectId::new(2);
        let worker = fixture_world_object(worker_id, "CLNK")
            .with_action_name("Walk")
            .with_energy(0)
        .with_full_state(Rc::new(crate::preview_spawn_state(
            Vector2::ZERO,
            OWNER_NONE,
            OWNER_NONE,
            DEFAULT_CATEGORY,
            crate::FULL_CON,
            crate::CONTACT_DENSITY_SOLID,
            Vec::new(),
        )));
        let mut worker_script = clonk_script::Engine::new();
        register_host_functions(&mut worker_script);
        let world = HostWorldContext::from_objects(vec![worker])
            .with_definition_metadata(Rc::new(HashMap::from([(
                DefinitionId::from("CLNK"),
                DefinitionMetadata::default(),
            )])))
            .with_definition_scripts(HashMap::from([(
                DefinitionId::from("CLNK"),
                Arc::new(worker_script),
            )]));
        let args = [
            object_reference_value(worker_id),
            Value::String("Enter".into()),
            object_reference_value(ObjectId::new(1)),
        ];

        let (result, outcome) = with_object_host_context_with_world(world, || add_command(&args));

        assert_eq!(
            result.expect("foreign AddCommand succeeds"),
            Value::Bool(true)
        );
        let operations = &outcome
            .other_objects
            .iter()
            .find(|outcome| outcome.object_id == worker_id)
            .expect("foreign worker outcome recorded")
            .command_operations;
        match operations.as_slice() {
            [CommandOperation::PushFront(request)] => {
                assert_eq!(request.id, CommandId::Enter);
                assert_eq!(request.target, Some(ObjectId::new(1)));
            }
            other => panic!("expected one foreign PushFront, got {other:?}"),
        }
        assert!(
            outcome.command_operations.is_empty(),
            "caller remains unchanged"
        );
    }

    #[test]
    fn append_command_pushes_back() {
        let args = vec![
            Value::String("MoveTo".into()),
            Value::Nil,
            Value::Int(3),
            Value::Int(4),
        ];
        let (result, outcome) = with_object_host_context(|| append_command(&args));
        let value = result.expect("AppendCommand succeeds");
        assert_eq!(value, Value::Bool(true));
        assert_eq!(outcome.command_operations.len(), 1);
        match &outcome.command_operations[0] {
            CommandOperation::PushBack(request) => {
                assert_eq!(request.id, CommandId::MoveTo);
                assert_eq!(request.tx, Some(3));
                assert_eq!(request.ty, Some(4));
            }
            other => panic!("expected PushBack operation, got {:?}", other),
        }
    }

    #[test]
    fn append_command_converts_c4id_data_to_its_integer_payload() {
        // FnAppendCommand uses C4Value::getIntOrID for non-Call command data,
        // preserving the four-byte C4ID payload (C4Script.cpp:903-913).
        let args = vec![
            Value::String("Buy".into()),
            Value::Nil,
            Value::Int(1),
            Value::Int(0),
            Value::Nil,
            Value::Int(0),
            Value::C4Id("LORY".into()),
        ];
        let (result, outcome) = with_object_host_context(|| append_command(&args));

        assert_eq!(result.expect("AppendCommand succeeds"), Value::Bool(true));
        match &outcome.command_operations[0] {
            CommandOperation::PushBack(request) => {
                assert_eq!(
                    request.data,
                    CommandData::Integer(
                        definition_id_to_c4id("LORY").expect("four-byte definition id")
                    )
                );
            }
            other => panic!("expected PushBack operation, got {:?}", other),
        }
    }

    #[test]
    fn angle_matches_the_cpp_quadrant_math() {
        // FnAngle (C4Script.cpp:3255-3280): 0 = up, 90 = right; axis
        // shortcuts; trunc(180*prec*atan2(|dy|,|dx|)/pi) folded per
        // quadrant. The dragon's Flying() computes its target rotation
        // with Angle(iVy, -iVx) (Fantasy.c4d Dragon.c4d Script.c:540).
        let angle = |x1: i32, y1: i32, x2: i32, y2: i32, prec: i32| {
            let args = [
                Value::Int(x1),
                Value::Int(y1),
                Value::Int(x2),
                Value::Int(y2),
                Value::Int(prec),
            ];
            let (result, _) = with_object_host_context(|| angle_func(&args));
            result.expect("Angle succeeds")
        };
        assert_eq!(angle(0, 0, 0, 10, 0), Value::Int(180), "straight down");
        assert_eq!(angle(0, 0, 0, -10, 0), Value::Int(0), "straight up");
        assert_eq!(angle(0, 0, 0, 0, 0), Value::Int(0), "no delta");
        assert_eq!(angle(0, 0, 10, 0, 0), Value::Int(90), "right");
        assert_eq!(angle(0, 0, -10, 0, 0), Value::Int(270), "left");
        assert_eq!(angle(0, 0, 10, -10, 0), Value::Int(45));
        assert_eq!(angle(0, 0, 10, 10, 0), Value::Int(135));
        assert_eq!(angle(0, 0, -10, -10, 0), Value::Int(315));
        assert_eq!(angle(0, 0, -10, 10, 0), Value::Int(225));
        // Precision: 900 - trunc(1800*atan2(2,5)/pi) = 900 - 218.
        assert_eq!(angle(0, 0, 5, -2, 10), Value::Int(682));
    }

    #[test]
    fn get_command_exposes_target_tx_ty_and_data_elements() {
        // FnGetCommand elements (C4Script.cpp:926-945): 1 Target, 2 Tx,
        // 3 C4VInt(Ty), 4 Target2, 5 C4Value(Data, C4V_Any) — zero int
        // Data reads nil. The dragon's Flying() steers by
        // GetCommand(0, 2)/GetCommand(0, 3) (Fantasy.c4d Dragon.c4d
        // Script.c:505-512).
        let world_object = fixture_world_object(ObjectId::new(1), "DRGN")
            .with_action_name("Fly")
            .with_position(Vector2::new(50, 50))
        .with_commands(vec![CommandView {
            name: "MoveTo".into(),
            target: None,
            tx: Some(200),
            tx_value: Some(Value::Int(200)),
            tx_definition: None,
            ty: Some(90),
            target2: None,
            data: CommandData::Integer(0),
            legacy_data: None,
            finished: false,
        }]);
        let world = HostWorldContext::from_objects(vec![world_object]);
        let query = |element: i32| {
            let (result, _) = with_object_host_context_with_world(world.clone(), || {
                get_command(&[Value::Int(0), Value::Int(element)])
            });
            result.expect("GetCommand succeeds")
        };
        assert_eq!(query(0), Value::String("MoveTo".into()));
        assert_eq!(query(1), Value::Nil, "no target object");
        assert_eq!(query(2), Value::Int(200), "Tx");
        assert_eq!(query(3), Value::Int(90), "Ty");
        assert_eq!(query(4), Value::Nil, "no target2");
        assert_eq!(query(5), Value::Nil, "zero Data is nil in C4V_Any");
    }

    #[test]
    fn get_r_projects_raw_rotation_to_the_cpp_signed_range() {
        // FnGetR projects stored r into [-180,180] (C4Script.cpp:1181-1188).
        // SetR stores 350, but scripts observe -10; movement-produced negative
        // rotations remain negative for AdjustWalkRotation/Flying deltas.
        let (result, _) = with_effect_context(
            Some(HostObjectContext {
                energy: 100,
                rotation: 350,
                action_name: "Walk".to_string(),
                direction: Direction::Left,
                ..idle_object_scope(ObjectId::new(1))
            }),
            &[],
            HostWorldContext::default(),
            1,
            || get_r(&[]),
        );
        assert_eq!(result.expect("GetR succeeds"), Value::Int(-10));
        assert_eq!(script_rotation(-190), 170);
        assert_eq!(script_rotation(540), 180);
        assert_eq!(script_rotation(-540), -180);
    }

    fn adjust_walk_rotation_case(
        seed: WalkRotationSeed,
        rotation: i32,
        vertices: &[ObjectVertex],
        landscape: Option<Landscape>,
        args: &[Value],
    ) -> (Result<Value, RuntimeError>, EffectContextOutcome) {
        let world = world_with(
            Vec::<HostWorldObject>::new(),
            landscape,
            HashMap::new(),
            HashMap::new(),
        );
        with_effect_context(
            Some(
                HostObjectContext::with_category(
                    ObjectId::new(1),
                    None,
                    ObjectStatus::Normal,
                    100,
                    0,
                    crate::FULL_CON,
                    OWNER_NONE,
                    Vector2::ZERO,
                    Vector2::ZERO,
                    rotation,
                    &[],
                    "Walk",
                    0,
                    0,
                    0,
                    ActionLibrary::default(),
                    Direction::Left,
                    CommandDirection::Stop,
                    0,
                    None,
                    None,
                    vertices,
                    DEFAULT_CATEGORY,
                    ocf::NORMAL,
                    false,
                    None,
                    None,
                )
                .with_walk_rotation(seed),
            ),
            &[],
            world,
            1,
            || adjust_walk_rotation(args),
        )
    }

    #[test]
    fn adjust_walk_rotation_steers_rdir_toward_the_floor_slope() {
        // C4Object::AdjustWalkRotation floor probe (C4Object.cpp:6023-6065):
        // GBackSolid columns at iAttachX +/- iRangeX around iAttachY, then
        // iDestAngle = (right - left) * (35 / max(iRangeX,1)) — INNER
        // integer division first (35/20 = 1 for the dragon's
        // AdjustWalkRotation(20, 20, 100), Fantasy.c4d Dragon.c4d
        // Script.c:701). rdir = itofix(BoundBy(dest - r, -15, 15)) /
        // (10000 / iSpeed) (C4Object.cpp:6078-6084).
        // Ground: columns 0..32 surface y=25, columns 32..64 surface y=5.
        // Attach at (30, 15): left probe x=10 descends to the y=25 floor
        // (offset +9), right probe x=50 sits inside ground (offset 0) ->
        // dest = (0 - 9) * 1 = -9; r = 0 -> rdir = itofix(-9)/100.
        let mut surface = vec![25; 32];
        surface.extend(vec![5; 32]);
        let landscape = Landscape::new(64, surface).expect("landscape builds");
        let seed = WalkRotationSeed {
            rotateable: 45,
            t_attach: CNAT_BOTTOM,
            attach: ShapeAttachRecord {
                mat_valid: true,
                mat_vehicle: false,
                x: 30,
                y: 15,
                vtx: 0,
            },
            def_attach_vtx_x: 0,
        };
        let args = [Value::Int(20), Value::Int(20), Value::Int(100)];
        let (result, outcome) = adjust_walk_rotation_case(seed, 0, &[], Some(landscape), &args);
        assert_eq!(
            result.expect("AdjustWalkRotation succeeds"),
            Value::Bool(true)
        );
        assert_eq!(
            outcome
                .object_update
                .and_then(|update| update.rotation_velocity),
            Some(C4Fixed::from_raw(-9 * 65536 / 100)),
            "rdir = itofix(-9) / (10000/100)"
        );
    }

    #[test]
    fn adjust_walk_rotation_guards_and_vertex_branch() {
        // Guards (C4Script.cpp:5443-5446): no Rotateable / no bottom
        // attach / no attach material -> false, rdir untouched.
        let args = [Value::Int(20), Value::Int(20), Value::Int(100)];
        let (result, outcome) =
            adjust_walk_rotation_case(WalkRotationSeed::default(), 0, &[], None, &args);
        assert_eq!(result.expect("guarded call runs"), Value::Bool(false));
        assert_eq!(
            outcome
                .object_update
                .and_then(|update| update.rotation_velocity),
            None
        );

        // Attachment at a NON-middle vertex (Def->Shape.VtxX[vtx] != 0):
        // live Shape.VtxX > 0 -> iDestAngle = -50 (C4Object.cpp:6068-6076);
        // r = 0 -> rdir = itofix(BoundBy(-50, -15, 15)) / 100.
        let vertices = vec![ObjectVertex::new(0, 0), ObjectVertex::new(3, 10)];
        let seed = WalkRotationSeed {
            rotateable: 45,
            t_attach: CNAT_BOTTOM,
            attach: ShapeAttachRecord {
                mat_valid: true,
                mat_vehicle: false,
                x: 30,
                y: 15,
                vtx: 1,
            },
            def_attach_vtx_x: 5,
        };
        let (result, outcome) = adjust_walk_rotation_case(seed, 0, &vertices, None, &args);
        assert_eq!(result.expect("vertex branch runs"), Value::Bool(true));
        assert_eq!(
            outcome
                .object_update
                .and_then(|update| update.rotation_velocity),
            Some(C4Fixed::from_raw(-15 * 65536 / 100)),
            "rdir = itofix(-15) / 100"
        );

        // Within +/-2 degrees of the destination the spin clears
        // (C4Object.cpp:6083 `else rdir = 0`).
        let vertices = vec![ObjectVertex::new(0, 0), ObjectVertex::new(3, 10)];
        let seed = WalkRotationSeed {
            rotateable: 45,
            t_attach: CNAT_BOTTOM,
            attach: ShapeAttachRecord {
                mat_valid: true,
                mat_vehicle: false,
                x: 30,
                y: 15,
                vtx: 1,
            },
            def_attach_vtx_x: 5,
        };
        let (result, outcome) = adjust_walk_rotation_case(seed, -49, &vertices, None, &args);
        assert_eq!(result.expect("near-dest call runs"), Value::Bool(true));
        assert_eq!(
            outcome
                .object_update
                .and_then(|update| update.rotation_velocity),
            Some(C4Fixed::ZERO)
        );
    }

    #[test]
    fn host_functions_fill_missing_parameters_like_the_cpp_vm() {
        // C4AulExec fills unfilled parameter slots with nil; engine
        // functions convert nil to 0/false/nullptr instead of failing
        // (C4AulExec parameter filling + C4Value conversions). Each case
        // mirrors the C++ Fn* result for an under-filled call.
        // FnGetCrew(iPlr, index): missing index is 0 (C4Script.cpp:2798);
        // SkiesOfFire's InitializePlayer calls GetCrew(iPlr).
        let (result, _) = with_object_host_context(|| get_crew(&[Value::Int(0)]));
        assert_eq!(result.expect("GetCrew tolerates 1 arg"), Value::Nil);

        // FnSetAction(nullptr) -> false (C4Script.cpp:747-751).
        let (result, _) = with_object_host_context(|| set_action(&[]));
        assert_eq!(result.expect("SetAction() runs"), Value::Bool(false));

        // FnMessage(nullptr) -> false (C4Script.cpp:2421-2424).
        let (result, _) = with_object_host_context(|| message(&[]));
        assert_eq!(result.expect("Message() runs"), Value::Bool(false));

        // FnSetCommand: !szCommand -> false (C4Script.cpp:843-844).
        let (result, _) = with_object_host_context(|| set_command(&[]));
        assert_eq!(result.expect("SetCommand() runs"), Value::Bool(false));

        // FnSetR(0 default) -> SetRotation(0) (C4Script.cpp:737-745); the
        // scope dedupes the no-op write (rotation already 0), so only the
        // success result is observable.
        let (result, _) = with_object_host_context(|| set_r(&[]));
        assert_eq!(result.expect("SetR() runs"), Value::Bool(true));

        // The remaining int/bool-parameter wrappers accept bare calls
        // (all C4ValueInt/bool slots: C4Script.cpp:462-828, 2290-3120,
        // 5416, 5577).
        for (name, result) in [
            ("SetComDir", with_object_host_context(|| set_com_dir(&[])).0),
            ("SetDir", with_object_host_context(|| set_dir(&[])).0),
            ("SetPhase", with_object_host_context(|| set_phase(&[])).0),
            ("DoEnergy", with_object_host_context(|| do_energy(&[])).0),
            ("DoCon", with_object_host_context(|| do_con(&[])).0),
            ("DoDamage", with_object_host_context(|| do_damage(&[])).0),
            (
                "SetPosition",
                with_object_host_context(|| set_position(&[])).0,
            ),
            (
                "SetActionData",
                with_object_host_context(|| set_action_data(&[])).0,
            ),
            (
                "SetCategory",
                with_object_host_context(|| set_category(&[])).0,
            ),
            ("SetOwner", with_object_host_context(|| set_owner(&[])).0),
            ("SetAlive", with_object_host_context(|| set_alive(&[])).0),
            (
                "GetPlayerName",
                with_object_host_context(|| get_player_name(&[])).0,
            ),
            ("GetWealth", with_object_host_context(|| get_wealth(&[])).0),
            (
                "GetCrewCount",
                with_object_host_context(|| get_crew_count(&[])).0,
            ),
            (
                "AddCommand",
                with_object_host_context(|| add_command(&[])).0,
            ),
            (
                "AppendCommand",
                with_object_host_context(|| append_command(&[])).0,
            ),
        ] {
            assert!(result.is_ok(), "{name}() must not error: {result:?}");
        }
    }

    #[test]
    fn min_max_use_c4valueint_conversion_and_ignore_extra_arguments() {
        // FnMin/FnMax each expose two C4ValueInt parameters
        // (C4Script.cpp:3300-3308). C4Aul nil-fills missing slots, converts
        // bool to 0/1, and ignores surplus call arguments before dispatch
        // (C4AulExec.cpp:1364-1396).
        assert_eq!(max_func(&[]).expect("Max()"), Value::Int(0));
        assert_eq!(
            max_func(&[Value::Bool(true), Value::Bool(false), Value::Int(99)])
                .expect("Max ignores surplus args"),
            Value::Int(1)
        );
        assert_eq!(
            min_func(&[Value::Int(-7)]).expect("Min nil-fills val2"),
            Value::Int(-7)
        );
        assert_eq!(
            min_func(&[Value::Bool(true), Value::Int(4), Value::Int(-99)])
                .expect("Min converts bool and ignores surplus args"),
            Value::Int(1)
        );
    }

    #[test]
    fn set_dir_functions_default_missing_arguments_to_zero() {
        // Unfilled C4ValueInt parameters are nil -> 0 (C4AulExec.cpp
        // parameter filling): FnSetRDir()/FnSetXDir() zero the dir and
        // still mobilize (C4Script.cpp:697-732). The dragon stops itself
        // with bare SetRDir()/SetXDir() calls (Fantasy.c4d Dragon.c4d
        // Script.c:689).
        let (result, outcome) = with_object_host_context(|| {
            set_r_dir(&[])?;
            set_x_dir(&[])
        });
        assert_eq!(result.expect("bare SetXDir succeeds"), Value::Bool(true));
        let update = outcome.object_update.expect("velocity update recorded");
        assert_eq!(update.rotation_velocity, Some(C4Fixed::ZERO));
        assert_eq!(update.fixed_velocity_x, Some(C4Fixed::ZERO));
    }

    #[test]
    fn set_command_parses_the_cpp_argument_order() {
        // C++ FnSetCommand(pObj, szCommand, pTarget, Tx, iTy, pTarget2, data,
        // iRetries) has NO update-interval slot: data follows target2 directly
        // (C4Script.cpp:840-867). The dragon issues SetCommand(this(),
        // "MoveTo", 0, x, y, 0, C4CMD_MoveTo_NoPosAdjust=1, C4Command.h:68)
        // (Fantasy.c4d Dragon.c4d Script.c:1565).
        let this = object_reference_value(ObjectId::new(1));
        let args = vec![
            this,
            Value::String("MoveTo".into()),
            Value::Int(0),
            Value::Int(200),
            Value::Int(90),
            Value::Int(0),
            Value::Int(1),
        ];
        let (result, outcome) = with_object_host_context(|| set_command(&args));
        assert_eq!(result.expect("SetCommand succeeds"), Value::Bool(true));
        let request = match &outcome.command_operations[2] {
            CommandOperation::PushFront(request) => request.clone(),
            other => panic!("expected PushFront operation, got {:?}", other),
        };
        assert_eq!(request.id, CommandId::MoveTo);
        assert_eq!(request.tx, Some(200));
        assert_eq!(request.ty, Some(90));
        assert_eq!(request.data, CommandData::Integer(1));
        assert_eq!(
            request.update_interval, 0,
            "SetCommand has no interval slot"
        );
    }

    #[test]
    fn set_command_accepts_definitionless_construct_menu_request() {
        // CLNK::ContextConstruction uses this exact short form, then calls
        // ExecuteCommand so C4Command::Construct opens C4MN_Construction
        // (Objects.c4d/Crew.c4d/Clonk.c4d/Script.c:628-634).
        let args = vec![
            object_reference_value(ObjectId::new(1)),
            Value::String("Construct".into()),
        ];

        let (result, outcome) = with_object_host_context(|| set_command(&args));

        assert_eq!(result.expect("SetCommand succeeds"), Value::Bool(true));
        assert!(matches!(
            outcome.command_operations.as_slice(),
            [
                CommandOperation::DecrementNoCollectDelay,
                CommandOperation::Clear,
                CommandOperation::PushFront(CommandRequest {
                    id: CommandId::Construct,
                    data: CommandData::Integer(0),
                    ..
                })
            ]
        ));
    }

    #[test]
    fn add_command_defaults_to_silent_sub_mode() {
        // C++ FnAddCommand's iBaseMode is a C4ValueInt: an unfilled slot is
        // int 0 = C4CMD_Mode_SilentSub (C4Script.cpp:870, C4Command.h:62) —
        // NOT Base.
        let args = vec![Value::String("Wait".into())];
        let (result, outcome) = with_object_host_context(|| add_command(&args));
        assert_eq!(result.expect("AddCommand succeeds"), Value::Bool(true));
        match &outcome.command_operations[0] {
            CommandOperation::PushFront(request) => {
                assert_eq!(request.mode, CommandMode::SilentSub);
            }
            other => panic!("expected PushFront operation, got {:?}", other),
        }
    }

    #[test]
    fn add_command_call_accepts_the_workshop_c4id_tx_argument() {
        // FnAddCommand deliberately keeps Tx as C4Value for C4CMD_Call
        // (C4Script.cpp:871-892). Workshop::SelectProduction passes its
        // product C4ID in that slot and StartProduction must receive the
        // same typed value (Objects.c4d/Structures.c4d/Workshop.c4d/
        // Script.c:68-81).
        let this = object_reference_value(ObjectId::new(1));
        let args = vec![
            this.clone(),
            Value::String("Call".into()),
            this,
            Value::C4Id("BALN".into()),
            Value::Bool(false),
            Value::Int(0),
            Value::Int(0),
            Value::String("StartProduction".into()),
            Value::Int(0),
            Value::Int(1),
        ];

        let (result, outcome) = with_object_host_context(|| add_command(&args));

        assert_eq!(
            result.expect("Workshop AddCommand succeeds"),
            Value::Bool(true)
        );
        match &outcome.command_operations[0] {
            CommandOperation::PushFront(request) => {
                assert_eq!(request.id, CommandId::Call);
                assert_eq!(request.target, Some(ObjectId::new(1)));
                assert_eq!(
                    request.tx,
                    definition_id_to_c4id("BALN"),
                    "the command retains the C4ID's raw C4Value payload"
                );
                assert_eq!(request.tx_definition.as_deref(), Some("BALN"));
                assert_eq!(request.ty, Some(0));
                assert_eq!(request.data, CommandData::Text("StartProduction".into()));
                assert_eq!(request.mode, CommandMode::Base);
            }
            other => panic!("expected PushFront operation, got {other:?}"),
        }
    }

    #[test]
    fn call_command_wrappers_queue_non_string_data_as_an_empty_function_name() {
        // FnStringPar(data.getStr()) maps every non-string Call data value
        // to "". Set/Add/Append still queue the command, preserving the
        // untouched C4Value Tx for GetCommand and the normal failure path
        // (C4Script.cpp:79-82,840-916).
        let this = object_reference_value(ObjectId::new(1));
        let tx = Value::Array(vec![Value::Bool(false), this.clone()]);

        let set_args = vec![
            this.clone(),
            Value::String("Call".into()),
            this.clone(),
            tx.clone(),
            Value::Int(17),
            Value::Nil,
            Value::Int(99),
        ];
        let (set_result, set_outcome) = with_object_host_context(|| set_command(&set_args));
        assert_eq!(set_result.expect("SetCommand succeeds"), Value::Bool(true));
        let set_request = set_outcome
            .command_operations
            .iter()
            .find_map(|operation| match operation {
                CommandOperation::PushFront(request) => Some(request),
                _ => None,
            })
            .expect("SetCommand queues Call");
        assert_eq!(set_request.data, CommandData::Text(String::new()));
        assert_eq!(set_request.tx_value.as_ref(), Some(&tx));

        let add_args = vec![
            this.clone(),
            Value::String("Call".into()),
            this.clone(),
            tx.clone(),
            Value::Int(17),
            Value::Nil,
            Value::Int(0),
            Value::Array(Vec::new()),
            Value::Int(0),
            Value::Int(1),
        ];
        let (add_result, add_outcome) = with_object_host_context(|| add_command(&add_args));
        assert_eq!(add_result.expect("AddCommand succeeds"), Value::Bool(true));
        let CommandOperation::PushFront(add_request) = &add_outcome.command_operations[0] else {
            panic!("expected AddCommand PushFront");
        };
        assert_eq!(add_request.data, CommandData::Text(String::new()));
        assert_eq!(add_request.tx_value.as_ref(), Some(&tx));

        let append_args = vec![
            this.clone(),
            Value::String("Call".into()),
            this,
            tx.clone(),
            Value::Int(17),
            Value::Nil,
            Value::Int(0),
            Value::Bool(true),
            Value::Int(0),
            Value::Int(1),
        ];
        let (append_result, append_outcome) =
            with_object_host_context(|| append_command(&append_args));
        assert_eq!(
            append_result.expect("AppendCommand succeeds"),
            Value::Bool(true)
        );
        let CommandOperation::PushBack(append_request) = &append_outcome.command_operations[0]
        else {
            panic!("expected AppendCommand PushBack");
        };
        assert_eq!(append_request.data, CommandData::Text(String::new()));
        assert_eq!(append_request.tx_value.as_ref(), Some(&tx));
    }

    #[test]
    fn call_command_function_text_uses_the_native_nul_terminated_prefix() {
        let parse = |text: &str| {
            parse_command_request(
                CommandId::Call,
                &[
                    Value::String("Call".into()),
                    Value::Nil,
                    Value::Int(7),
                    Value::Int(0),
                    Value::Nil,
                    Value::Int(0),
                    Value::String(text.into()),
                ],
                CommandArgLayout::Add,
                "AddCommand",
            )
            .expect("Call parses")
            .data
        };

        assert_eq!(parse("Work\0ignored"), CommandData::Text("Work".into()));
        assert_eq!(parse("\0Work"), CommandData::Text(String::new()));
    }

    #[test]
    fn command_data_any_value_matches_c4value_guess_type() {
        let wood = definition_id_to_c4id("WOOD").expect("four-byte definition id");
        assert_eq!(command_data_any_value(0), Value::Nil);
        assert_eq!(command_data_any_value(9_999), Value::Int(9_999));
        assert_eq!(command_data_any_value(wood), Value::C4Id("WOOD".into()));
        assert_eq!(command_data_any_value(-1), Value::Int(-1));
    }

    #[test]
    fn append_command_leads_with_the_object_slot() {
        // C++ FnAppendCommand(pObj, szCommand, pTarget, Tx, iTy, pTarget2,
        // iUpdateInterval, Data, iRetries, iBaseMode) — the OBJECT slot comes
        // first, like SetCommand/AddCommand (C4Script.cpp:894-916). The dragon
        // queues AppendCommand(this(), "Call", this(), 0,0,0,0, "StopComDir")
        // (Fantasy.c4d Dragon.c4d Script.c:1566).
        let this = object_reference_value(ObjectId::new(1));
        let args = vec![
            this.clone(),
            Value::String("Call".into()),
            this,
            Value::Int(0),
            Value::Int(0),
            Value::Int(0),
            Value::Int(0),
            Value::String("StopComDir".into()),
        ];
        let (result, outcome) = with_object_host_context(|| append_command(&args));
        assert_eq!(result.expect("AppendCommand succeeds"), Value::Bool(true));
        assert_eq!(outcome.command_operations.len(), 1);
        match &outcome.command_operations[0] {
            CommandOperation::PushBack(request) => {
                assert_eq!(request.id, CommandId::Call);
                assert_eq!(request.target, Some(ObjectId::new(1)));
                assert_eq!(request.data, CommandData::Text("StopComDir".into()));
            }
            other => panic!("expected PushBack operation, got {:?}", other),
        }
    }

    #[test]
    fn get_x_returns_current_position() {
        let (result, _) = with_effect_context(
            Some(HostObjectContext {
                position: Vector2::new(42, -7),
                ..idle_object_context()
            }),
            &[],
            HostWorldContext::default(),
            1,
            || get_x(&[]),
        );

        let value = result.expect("GetX succeeds");
        assert_eq!(value, Value::Int(42));
    }

    #[test]
    fn get_y_returns_current_position() {
        let (result, _) = with_effect_context(
            Some(HostObjectContext {
                id: ObjectId::new(2),
                position: Vector2::new(-5, 63),
                ..idle_object_context()
            }),
            &[],
            HostWorldContext::default(),
            1,
            || get_y(&[]),
        );

        let value = result.expect("GetY succeeds");
        assert_eq!(value, Value::Int(63));
    }

    #[test]
    fn get_x_reads_world_when_target_provided() {
        let other = fixture_world_object(ObjectId::new(99), "Dummy")
            .with_position(Vector2::new(-12, 34));
        let world = HostWorldContext::from_objects(vec![other]);
        let args = [object_reference_value(ObjectId::new(99))];

        let (result, _) = with_effect_context(None, &[], world, 1, || get_x(&args));
        let value = result.expect("GetX target succeeds");
        assert_eq!(value, Value::Int(-12));
    }

    #[test]
    fn get_y_returns_nil_for_missing_target() {
        let args = [object_reference_value(ObjectId::new(1234))];
        let (result, _) =
            with_effect_context(None, &[], HostWorldContext::default(), 1, || get_y(&args));
        let value = result.expect("GetY handles missing target");
        assert_eq!(value, Value::Nil);
    }

    #[test]
    fn object_distance_defaults_to_context_object() {
        let context_id = ObjectId::new(1);
        let other_id = ObjectId::new(2);
        let world = HostWorldContext::from_objects(vec![
            fixture_world_object(context_id, "Clonk")
                .with_position(Vector2::new(10, 15)),
            fixture_world_object(other_id, "Dummy")
                .with_position(Vector2::new(25, 30)),
        ]);
        let args = [object_reference_value(other_id)];
        let (result, _) = with_effect_context(
            Some(HostObjectContext {
                id: context_id,
                position: Vector2::new(10, 15),
                ..idle_object_context()
            }),
            &[],
            world,
            3,
            || object_distance(&args),
        );
        let value = result.expect("ObjectDistance succeeds");
        assert_eq!(value, Value::Int(integer_distance(10, 15, 25, 30)));
    }

    #[test]
    fn object_distance_accepts_explicit_anchor_without_host_object() {
        let anchor_id = ObjectId::new(5);
        let other_id = ObjectId::new(6);
        let world = HostWorldContext::from_objects(vec![
            fixture_world_object(anchor_id, "Anchor")
                .with_position(Vector2::new(-40, 12)),
            fixture_world_object(other_id, "Target")
                .with_position(Vector2::new(-10, -18)),
        ]);
        let args = [
            object_reference_value(other_id),
            object_reference_value(anchor_id),
        ];
        let (result, _) = with_effect_context(None, &[], world, 10, || object_distance(&args));
        let value = result.expect("ObjectDistance with explicit anchor succeeds");
        assert_eq!(value, Value::Int(integer_distance(-40, 12, -10, -18)));
    }

    #[test]
    fn object_distance_returns_nil_when_other_missing() {
        let args = [object_reference_value(ObjectId::new(99))];
        let (result, _) = with_effect_context(
            Some(HostObjectContext {
                id: ObjectId::new(3),
                position: Vector2::new(0, 0),
                ..idle_object_context()
            }),
            &[],
            HostWorldContext::default(),
            4,
            || object_distance(&args),
        );
        let value = result.expect("ObjectDistance with missing other succeeds");
        assert_eq!(value, Value::Nil);
    }

    #[test]
    fn object_lookup_resolves_dragon_rock_saved_number_through_vm() {
        // FnObject delegates to SafeObjectPointer (C4Script.cpp:3327-3330).
        // Dragon Rock's GetEndboss resolves the loaded mage as Object(1758).
        let mage_id = ObjectId::new(1758);
        let world = HostWorldContext::from_objects(vec![fixture_world_object(mage_id, "MAGE")]);
        let (result, _) = with_effect_context(None, &[], world, 1759, || {
            let mut script = clonk_script::Engine::new();
            register_host_functions(&mut script);
            script
                .load_script("global func Probe() { return Object(1758); }")
                .map_err(|error| RuntimeError::new(error.to_string()))?;
            script
                .call("Probe", &[])
                .map_err(|error| RuntimeError::new(error.to_string()))
        });

        assert_eq!(
            result.expect("Object lookup succeeds"),
            object_reference_value(mage_id)
        );
    }

    #[test]
    fn object_lookup_accepts_inactive_but_rejects_deleted_or_missing() {
        // C4GameObjects::ObjectPointer checks normal then inactive objects;
        // SafeObjectPointer rejects only Status==C4OS_DELETED
        // (C4GameObjects.cpp:270-276; C4ObjectList.cpp:553-557).
        let id = ObjectId::new(42);
        let lookup = |status: Option<ObjectStatus>| {
            let objects: Vec<HostWorldObject> = status
                .map(|status| {
                    fixture_world_object(id, "TEST")
                        .with_status(status)
                })
                .into_iter()
                .collect();
            with_effect_context(
                None,
                &[],
                HostWorldContext::from_objects(objects),
                43,
                || object_by_number(&[Value::Int(42)]),
            )
            .0
            .expect("Object lookup succeeds")
        };

        assert_eq!(
            lookup(Some(ObjectStatus::Normal)),
            object_reference_value(id)
        );
        assert_eq!(
            lookup(Some(ObjectStatus::Inactive)),
            object_reference_value(id)
        );
        assert_eq!(lookup(Some(ObjectStatus::Deleted)), Value::Nil);
        assert_eq!(lookup(None), Value::Nil);
    }

    #[test]
    fn object_number_returns_the_cpp_enumeration_number_for_every_object_status() {
        // FnObjectNumber returns pObj->Number directly after defaulting a null
        // argument to cthr->Obj; unlike FnObject/ObjectPointer it performs no
        // active/inactive-list or Status check (C4Script.cpp:3321-3325).
        // Real content relies on the stable number in deferred command strings,
        // e.g. MagicBow.c4d/Script.c:114 and NoCamping.c4d/Script.c:50.
        let caller = ObjectId::new(37);
        let normal = ObjectId::new(811);
        let inactive = ObjectId::new(409);
        let deleted = ObjectId::new(1_203);
        let world = HostWorldContext::from_objects(vec![
            fixture_world_object(normal, "NORM"),
            fixture_world_object(inactive, "INAC")
                .with_status(ObjectStatus::Inactive),
            fixture_world_object(deleted, "DEAD")
                .with_status(ObjectStatus::Deleted),
        ]);
        let context = HostObjectContext {
            id: caller,
            ..idle_object_context()
        };
        let (result, _) = with_effect_context(Some(context), &[], world, 1_204, || {
            let mut script = clonk_script::Engine::new();
            register_host_functions(&mut script);
            script
                .load_script(
                    "#strict\nfunc Probe(a, b, c) { var unset; return [ObjectNumber(), ObjectNumber(unset), ObjectNumber(a), ObjectNumber(b), ObjectNumber(c), Format(\"Object(%d)\", ObjectNumber(a))]; }",
                )
                .map_err(|error| RuntimeError::new(error.to_string()))?;
            script
                .call(
                    "Probe",
                    &[
                        object_reference_value(normal),
                        object_reference_value(inactive),
                        object_reference_value(deleted),
                    ],
                )
                .map_err(|error| RuntimeError::new(error.to_string()))
        });

        assert_eq!(
            result.expect("ObjectNumber calls succeed"),
            Value::Array(vec![
                Value::Int(37),
                Value::Int(37),
                Value::Int(811),
                Value::Int(409),
                Value::Int(1_203),
                Value::String("Object(811)".into()),
            ]),
            "object numbers are enumeration values, not list positions"
        );
    }

    #[test]
    fn object_number_without_an_object_context_is_nil() {
        // FnObjectNumber returns an empty optional when both pObj and
        // cthr->Obj are null (C4Script.cpp:3321-3325).
        let mut script = clonk_script::Engine::new();
        register_host_functions(&mut script);
        script
            .load_script("global func Probe() { return ObjectNumber(); }")
            .expect("ObjectNumber probe compiles");

        assert_eq!(
            script
                .call("Probe", &[])
                .expect("ObjectNumber call succeeds"),
            Value::Nil
        );
    }

    #[test]
    fn object_number_keeps_a_deleted_callers_number() {
        // AssignRemoval changes Status before the running script unwinds, but
        // cthr->Obj remains the same pointer and FnObjectNumber reads Number
        // without consulting Status (C4Object.cpp:281-315;
        // C4Script.cpp:3321-3325).
        let deleted_caller = ObjectId::new(73);
        let context = HostObjectContext {
            id: deleted_caller,
            status: ObjectStatus::Deleted,
            ..idle_object_context()
        };
        let (result, _) =
            with_effect_context(Some(context), &[], HostWorldContext::default(), 74, || {
                let mut script = clonk_script::Engine::new();
                register_host_functions(&mut script);
                script
                    .load_script("func Probe() { return ObjectNumber(); }")
                    .map_err(|error| RuntimeError::new(error.to_string()))?;
                script
                    .call("Probe", &[])
                    .map_err(|error| RuntimeError::new(error.to_string()))
            });

        assert_eq!(result.expect("ObjectNumber call succeeds"), Value::Int(73));
    }

    #[test]
    fn get_x_dir_returns_object_velocity() {
        let context = HostObjectContext {
            id: ObjectId::new(7),
            velocity: Vector2::new(12, -3),
            ..idle_object_context()
        };
        let (result, _) =
            with_effect_context(Some(context), &[], HostWorldContext::default(), 1, || {
                get_x_dir(&[])
            });
        let value = result.expect("GetXDir succeeds");
        // C++ GetXDir default precision 10 returns fixtoi(xdir, 10): for a
        // 12 px/frame velocity that is 12 * 10 = 120. `C4Script.cpp:1167`.
        assert_eq!(value, Value::Int(120));
    }

    #[test]
    fn get_y_dir_applies_precision_scaling() {
        let context = HostObjectContext {
            id: ObjectId::new(8),
            velocity: Vector2::new(0, 25),
            ..idle_object_context()
        };
        let args = [Value::Nil, Value::Int(5)];
        let (result, _) =
            with_effect_context(Some(context), &[], HostWorldContext::default(), 1, || {
                get_y_dir(&args)
            });
        let value = result.expect("GetYDir succeeds");
        // C++ GetYDir(precision = 5) returns fixtoi(ydir, 5): for a 25 px/frame
        // velocity that is 25 * 5 = 125. `C4Script.cpp:1174`.
        assert_eq!(value, Value::Int(125));
    }

    #[test]
    fn get_x_dir_reads_world_velocity_when_target_provided() {
        let other = fixture_world_object(ObjectId::new(42), "Dummy")
            .with_velocity(Vector2::new(-8, 3));
        let world = HostWorldContext::from_objects(vec![other]);
        let args = [object_reference_value(ObjectId::new(42))];
        let (result, _) = with_effect_context(None, &[], world, 1, || get_x_dir(&args));
        let value = result.expect("GetXDir target succeeds");
        // GetXDir on another object: fixtoi(xdir, 10) for -8 px/frame = -80.
        assert_eq!(value, Value::Int(-80));
    }

    #[test]
    fn get_velocity_components_preserve_foreign_subpixel_values_and_match_arrow_calls() {
        let caller_script = r#"#strict 2
func Probe(object other)
{
    var unset;
    return [GetXDir(other, 100), GetYDir(other),
            other->GetXDir(unset, 100), other->GetYDir()];
}
"#;
        let mut engine = crate::Engine::with_seed(0);
        engine
            .register_definition(
                crate::Definition::from_script("CALL", "Caller", caller_script)
                    .expect("caller fixture compiles"),
            )
            .expect("caller fixture registers");
        engine
            .register_definition(
                crate::Definition::from_script("OTHR", "Other", "#strict 2\n")
                    .expect("target fixture compiles"),
            )
            .expect("target fixture registers");
        let caller = engine
            .spawn_object(crate::SpawnConfig::new("CALL").with_category(crate::CATEGORY_OBJECT))
            .expect("caller fixture spawns");
        let other = engine
            .spawn_object(
                crate::SpawnConfig::new("OTHR")
                    .with_category(crate::CATEGORY_OBJECT)
                    .with_fixed_velocity(FixedVec2::new(fixed10(26), fixed10(14))),
            )
            .expect("target fixture spawns");

        let result = engine
            .call_object_function(
                engine.find_object_index(caller).expect("caller exists"),
                "Probe",
                vec![object_reference_value(other)],
            )
            .expect("velocity fixture runs");

        assert_eq!(
            result,
            Value::Array(vec![
                Value::Int(260),
                Value::Int(14),
                Value::Int(260),
                Value::Int(14),
            ]),
            "plain and arrow reads retain the same exact fixed-point velocity"
        );
    }

    #[test]
    fn get_x_dir_returns_nil_for_missing_target() {
        let args = [object_reference_value(ObjectId::new(77))];
        let (result, _) = with_effect_context(None, &[], HostWorldContext::default(), 1, || {
            get_x_dir(&args)
        });
        let value = result.expect("GetXDir handles missing target");
        assert_eq!(value, Value::Nil);
    }

    #[test]
    fn set_x_dir_stores_subpixel_fixed_velocity_like_cpp() {
        // C++ FnSetXDir(15) with default precision 10 sets xdir = itofix(15, 10)
        // = 1.5 px/frame (raw 16.16 value 98304). `C4Script.cpp:697`.
        let args = [Value::Int(15)];
        let (result, outcome) = with_object_host_context(|| set_x_dir(&args));
        assert_eq!(result.expect("SetXDir succeeds"), Value::Bool(true));
        let update = outcome.object_update.expect("velocity update recorded");
        // Component-only write (C4Script.cpp:697-705): ydir is untouched.
        let fixed_x = update.fixed_velocity_x.expect("fixed x recorded");
        assert_eq!(fixed_x, itofix_prec(15, 10));
        assert_eq!(fixed_x.val(), 98304);
        assert!(update.fixed_velocity.is_none());
        assert!(update.fixed_velocity_y.is_none());
        // The whole-pixel mirror derives at the fold (fixtoi of the
        // final fixed value) — no int velocity staged here.
        assert!(update.velocity.is_none());
    }

    #[test]
    fn set_y_dir_applies_precision_when_recording_update() {
        // C++ FnSetYDir(5, prec = 5) sets ydir = itofix(5, 5) = 1.0 px/frame
        // (raw 16.16 value 65536). `C4Script.cpp:723`.
        let args = [Value::Int(5), Value::Nil, Value::Int(5)];
        let (result, outcome) = with_object_host_context(|| set_y_dir(&args));
        let value = result.expect("SetYDir succeeds");
        assert_eq!(value, Value::Bool(true));
        let update = outcome.object_update.expect("velocity update recorded");
        // Component-only write (C4Script.cpp:718-732): xdir untouched, the
        // whole-pixel mirror derives at the fold from the final fixed value.
        let fixed_y = update.fixed_velocity_y.expect("fixed y recorded");
        assert_eq!(fixed_y, itofix_prec(5, 5));
        assert_eq!(fixed_y.val(), 65536);
        assert!(update.fixed_velocity.is_none());
        assert!(update.fixed_velocity_x.is_none());
    }

    #[test]
    fn set_y_dir_targets_a_foreign_object_like_cpp() {
        // FnSetYDir writes the explicit pObj's ydir (C4Script.cpp:718-732).
        // DRCK relies on this to stop its PIPH without changing the
        // derrick callback's own velocity (Derrick.c4d/Script.c:92-99).
        let target_id = ObjectId::new(2);
        let target = fixture_world_object(target_id, "PIPH")
            .with_action_name("Drill")
            .with_energy(0)
            .with_velocity(Vector2::new(2, 7))
        .with_full_state(Rc::new(crate::preview_spawn_state(
            Vector2::ZERO,
            OWNER_NONE,
            OWNER_NONE,
            DEFAULT_CATEGORY,
            crate::FULL_CON,
            crate::CONTACT_DENSITY_SOLID,
            Vec::new(),
        )));
        let world = HostWorldContext::from_objects(vec![target]).with_definition_metadata(Rc::new(
            HashMap::from([(DefinitionId::from("PIPH"), DefinitionMetadata::default())]),
        ));

        let (result, outcome) = with_object_host_context_with_world(world, || {
            set_y_dir(&[Value::Int(0), object_reference_value(target_id)])
        });

        assert_eq!(result.expect("SetYDir succeeds"), Value::Bool(true));
        let update = outcome
            .other_objects
            .iter()
            .find(|outcome| outcome.object_id == target_id)
            .and_then(|outcome| outcome.update.as_ref())
            .expect("foreign ydir update recorded");
        assert_eq!(update.fixed_velocity_y, Some(C4Fixed::ZERO));
        assert!(
            update.fixed_velocity_x.is_none(),
            "foreign xdir is untouched"
        );
        assert!(outcome.object_update.is_none(), "caller remains unchanged");
    }

    #[test]
    fn set_r_dir_stores_subpixel_rotation_velocity_like_cpp() {
        // C++ FnSetRDir(10) with default precision 10 sets rdir = itofix(10, 10)
        // = 1.0 deg/frame (raw 16.16 value 65536). `C4Script.cpp:710`.
        let args = [Value::Int(10)];
        let (result, outcome) = with_object_host_context(|| set_r_dir(&args));
        assert_eq!(result.expect("SetRDir succeeds"), Value::Bool(true));
        let update = outcome
            .object_update
            .expect("rotation velocity update recorded");
        let rdir = update
            .rotation_velocity
            .expect("rotation velocity recorded");
        assert_eq!(rdir, itofix_prec(10, 10));
        assert_eq!(rdir.val(), 65536);
    }

    #[test]
    fn get_r_dir_reflects_pending_set_r_dir() {
        // Within a call, GetRDir reflects a prior SetRDir: SetRDir(10) is
        // 1.0 deg/frame, so GetRDir() at default precision 10 returns 10.
        let (result, _) = with_object_host_context(|| {
            set_r_dir(&[Value::Int(10)])?;
            get_r_dir(&[])
        });
        assert_eq!(result.expect("GetRDir succeeds"), Value::Int(10));
    }

    #[test]
    fn get_r_dir_reads_a_foreign_object() {
        // FnGetRDir reads any explicit pObj's live C4Fixed rdir
        // (C4Script.cpp:1182-1188), including its exact fractional state.
        let target_id = ObjectId::new(7);
        let state = crate::preview_spawn_state(
            Vector2::ZERO,
            OWNER_NONE,
            OWNER_NONE,
            DEFAULT_CATEGORY,
            crate::FULL_CON,
            crate::CONTACT_DENSITY_SOLID,
            Vec::new(),
        );
        let target = fixture_world_object(target_id, "ROCK")
            .with_energy(0)
        .with_rotation_velocity(itofix_prec(25, 10))
        .with_full_state(Rc::new(state));
        let world = HostWorldContext::from_objects(vec![target]);

        let (result, _) = with_effect_context(None, &[], world, 8, || {
            get_r_dir(&[object_reference_value(target_id)])
        });
        assert_eq!(result.expect("foreign GetRDir succeeds"), Value::Int(25));
    }

    #[test]
    fn set_r_dir_preserves_negative_precision_like_cpp() {
        // C++ only substitutes the default when precision is zero; a
        // negative denominator reverses the angular velocity sign.
        let args = [Value::Int(10), Value::Nil, Value::Int(-10)];
        let (result, outcome) = with_object_host_context(|| set_r_dir(&args));
        assert_eq!(result.expect("SetRDir succeeds"), Value::Bool(true));
        assert_eq!(
            outcome
                .object_update
                .expect("rotation update")
                .rotation_velocity,
            Some(itofix_prec(10, -10))
        );
    }

    #[test]
    fn set_r_records_same_angle_to_reseed_fixed_rotation() {
        // C4Object::SetRotation does not elide same-angle writes: it resets
        // fix_r and reflows the solid mask via UpdateFace(true).
        let (result, outcome) = with_object_host_context(|| set_r(&[Value::Int(0)]));
        assert_eq!(result.expect("SetR succeeds"), Value::Bool(true));
        assert_eq!(
            outcome.object_update.expect("rotation update").rotation,
            Some(0)
        );
    }

    #[test]
    fn set_r_and_set_r_dir_target_foreign_objects_and_read_back_live() {
        // GoldRush's ELEC GrabAdjustPosition passes found objects explicitly.
        // Both setters mutate that pObj and later reads in the same VM call
        // see the live writes without touching the caller.
        let target_id = ObjectId::new(7);
        let mut state = crate::preview_spawn_state(
            Vector2::ZERO,
            OWNER_NONE,
            OWNER_NONE,
            DEFAULT_CATEGORY,
            crate::FULL_CON,
            crate::CONTACT_DENSITY_SOLID,
            Vec::new(),
        );
        state.rotation = 37;
        let target = fixture_world_object(target_id, "ROCK")
            .with_energy(0)
        .with_full_state(Rc::new(state));
        let world = HostWorldContext::from_objects(vec![target]).with_definition_metadata(Rc::new(
            HashMap::from([(DefinitionId::from("ROCK"), DefinitionMetadata::default())]),
        ));

        let (result, outcome) = with_object_host_context_with_world(world, || {
            assert_eq!(
                set_r_dir(&[Value::Int(0), object_reference_value(target_id)])?,
                Value::Bool(true)
            );
            assert_eq!(
                set_r(&[Value::Int(0), object_reference_value(target_id)])?,
                Value::Bool(true)
            );
            assert_eq!(
                get_r_dir(&[object_reference_value(target_id)])?,
                Value::Int(0)
            );
            get_r(&[object_reference_value(target_id)])
        });
        assert_eq!(
            result.expect("foreign rotation calls succeed"),
            Value::Int(0)
        );
        let update = outcome
            .other_objects
            .iter()
            .find(|outcome| outcome.object_id == target_id)
            .and_then(|outcome| outcome.update.as_ref())
            .expect("foreign rotation update recorded");
        assert_eq!(update.rotation, Some(0));
        assert_eq!(update.rotation_velocity, Some(C4Fixed::ZERO));
        assert_eq!(update.mobile, Some(true));
        assert!(outcome.object_update.is_none(), "caller remains unchanged");
    }

    #[test]
    fn set_x_dir_respects_target_filter() {
        let mut target = ValueMap::new();
        target.insert("id".into(), Value::Int(99));
        let args = [Value::Int(4), Value::Proplist(target)];
        let (result, outcome) = with_object_host_context(|| set_x_dir(&args));
        let value = result.expect("SetXDir returns bool");
        assert_eq!(value, Value::Bool(false));
        assert!(outcome.object_update.is_none());
    }

    #[test]
    fn set_r_dir_respects_target_filter() {
        let mut target = ValueMap::new();
        target.insert("id".into(), Value::Int(99));
        let args = [Value::Int(4), Value::Proplist(target)];
        let (result, outcome) = with_object_host_context(|| set_r_dir(&args));
        let value = result.expect("SetRDir returns bool");
        assert_eq!(value, Value::Bool(false));
        assert!(outcome.object_update.is_none());
    }

    #[test]
    fn set_position_records_object_update() {
        let args = [Value::Int(15), Value::Int(27)];
        let (result, outcome) = with_object_host_context(|| set_position(&args));

        let value = result.expect("SetPosition succeeds");
        assert_eq!(value, Value::Bool(true));
        let update = outcome.object_update.expect("position update recorded");
        assert_eq!(update.position, Some(Vector2::new(15, 27)));
    }

    #[test]
    fn set_position_respects_target_filter() {
        let mut target = ValueMap::new();
        target.insert("id".into(), Value::Int(42));
        let args = [Value::Int(5), Value::Int(6), Value::Proplist(target)];
        let (result, outcome) = with_object_host_context(|| set_position(&args));

        let value = result.expect("SetPosition returns bool");
        assert_eq!(value, Value::Bool(false));
        assert!(outcome.object_update.is_none());
    }

    #[test]
    fn set_position_clamps_coordinates_when_requested() {
        let landscape = Landscape::flat(4, 6);
        let world = world_with(
            Vec::<HostWorldObject>::new(),
            Some(landscape),
            HashMap::new(),
            HashMap::new(),
        );
        let args = [
            Value::Int(10),
            Value::Int(20),
            Value::Nil,
            Value::Bool(true),
        ];
        let (result, outcome) = with_effect_context(
            Some(idle_object_context_with_vertices(&[ObjectVertex::new(0, 0)])),
            &[],
            world,
            1,
            || set_position(&args),
        );

        let value = result.expect("SetPosition returns bool");
        assert_eq!(value, Value::Bool(true));
        let update = outcome.object_update.expect("position update recorded");
        assert_eq!(update.position, Some(Vector2::new(3, 6)));
    }

    #[test]
    fn get_x_rejects_additional_arguments() {
        let (result, _) = with_object_host_context(|| get_x(&[Value::Nil, Value::Nil]));
        let error = result.expect_err("GetX rejects extra arguments");
        assert_eq!(error.to_string(), "GetX expects at most 1 argument: target");
    }

    #[test]
    fn get_effect_uses_context_view() {
        let state = empty_state();
        let (result, _) = with_object_host_context(|| {
            add_effect(&[Value::String("Glow".into()), state.clone(), Value::Int(100)])?;
            get_effect(&[
                Value::String("Glow".into()),
                state.clone(),
                Value::Int(0),
                Value::Int(1),
            ])
        });

        let value = result.expect("GetEffect succeeds");
        assert_eq!(value, Value::String("Glow".into()));
    }

    #[test]
    fn get_effect_scans_the_live_context_without_cloning_its_effect_stack() {
        // FnGetEffect walks pTarget->pEffects in place (C4Script.cpp:5458-5487);
        // a read-only query does not copy the C4Effect list.
        let state = empty_state();
        let (result, _) = with_object_host_context(|| {
            add_effect(&[Value::String("Glow".into()), state.clone(), Value::Int(100)])?;
            reset_effect_snapshot_count();
            let value = get_effect(&[
                Value::String("Glow".into()),
                state.clone(),
                Value::Int(0),
                Value::Int(1),
            ])?;
            assert_eq!(effect_snapshot_count(), 0);
            Ok::<_, RuntimeError>(value)
        });

        assert_eq!(
            result.expect("GetEffect succeeds"),
            Value::String("Glow".into())
        );
    }

    #[test]
    fn get_effect_converts_bool_query_to_c4valueint() {
        // FnGetEffect declares iQueryValue as C4ValueInt
        // (C4Script.cpp:5458), and Bool->Int is a direct conversion
        // (C4Value.cpp:514-518); true therefore selects query 1/name
        // (C4Script.cpp:5473-5477).
        let state = empty_state();
        let (result, _) = with_object_host_context(|| -> Result<Value, RuntimeError> {
            add_effect(&[Value::String("Glow".into()), state.clone(), Value::Int(100)])?;
            get_effect(&[
                Value::String("Glow".into()),
                state,
                Value::Int(0),
                Value::Bool(true),
            ])
        });

        assert_eq!(
            result.expect("bool query converts to one"),
            Value::String("Glow".into())
        );
    }

    #[test]
    fn get_effect_converts_bool_max_priority_to_c4valueint() {
        // FnGetEffect also declares iMaxPriority as C4ValueInt
        // (C4Script.cpp:5458), so bool true converts to signed limit 1
        // (C4Value.cpp:514-518) and admits priority 1
        // (C4Effect.cpp:223-226).
        let mut effect = EffectState::new("Glow").with_priority(1);
        effect.number = 7;
        let (result, _) =
            with_effect_context(None, &[effect], HostWorldContext::default(), 1, || {
                get_effect(&[
                    Value::String("Glow".into()),
                    Value::Nil,
                    Value::Int(0),
                    Value::Int(0),
                    Value::Bool(true),
                ])
            });

        assert_eq!(
            result.expect("bool max priority converts to one"),
            Value::Int(7)
        );
    }

    #[test]
    fn get_effect_zero_max_priority_is_unbounded() {
        // C4Effect::Get only applies its signed priority ceiling when
        // iMaxPriority is nonzero (C4Effect.cpp:223-226, 240-250).
        // FnGetEffect's omitted/nil C4ValueInt slot therefore becomes the
        // same unbounded zero (C4AulExec.cpp:1364-1394).
        let mut effect = EffectState::new("Glow").with_priority(100);
        effect.number = 8;
        let (result, _) =
            with_effect_context(None, &[effect], HostWorldContext::default(), 1, || {
                get_effect(&[
                    Value::String("Glow".into()),
                    Value::Nil,
                    Value::Int(0),
                    Value::Int(0),
                    Value::Int(0),
                ])
            });

        assert_eq!(
            result.expect("zero max priority is unbounded"),
            Value::Int(8)
        );
    }

    #[test]
    fn get_effect_uses_signed_max_priority_and_reports_absolute_priority() {
        // C4Effect::Get accepts negative iMaxPriority and compares the signed
        // stored priority directly (`priority > max`, C4Effect.cpp:223-226),
        // so -100 passes a -50 ceiling. FnGetEffect query 2 then returns
        // Abs(iPriority), not the deactivation sign (C4Script.cpp:5473-5478).
        let mut effect = EffectState::new("Dormant").with_priority(-100);
        effect.number = 9;
        let (result, _) =
            with_effect_context(None, &[effect], HostWorldContext::default(), 1, || {
                get_effect(&[
                    Value::String("Dormant".into()),
                    Value::Nil,
                    Value::Int(0),
                    Value::Int(2),
                    Value::Int(-50),
                ])
            });

        assert_eq!(
            result.expect("negative max priority is valid"),
            Value::Int(100)
        );
    }

    #[test]
    fn get_effect_converts_bool_index_to_c4valueint() {
        // FnGetEffect declares iIndex as C4ValueInt (C4Script.cpp:5458),
        // and Bool->Int is a direct conversion over the shared Data.Int slot
        // (C4Value.cpp:514-518). Hazard's Weapon GetFMData passes the boolean
        // result of `GetEffect(...) || j == 0` back as this index
        // (Weapon.c4d/Script.c:543-545).
        let state = empty_state();
        let (result, _) = with_object_host_context(|| -> Result<Value, RuntimeError> {
            add_effect(&[
                Value::String("Bonus".into()),
                state.clone(),
                Value::Int(100),
            ])?;
            add_effect(&[
                Value::String("Bonus".into()),
                state.clone(),
                Value::Int(100),
            ])?;
            get_effect(&[Value::String("Bonus".into()), state, Value::Bool(true)])
        });

        assert_eq!(
            result.expect("bool index converts to one"),
            Value::Int(1),
            "equal-priority effect #2 is list index 0, so bool true selects #1"
        );
    }
