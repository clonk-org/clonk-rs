// Contiguous slice 2 of 11 of the `compat::tests` battery, spliced by
// `include!` from compat.rs so every test id stays `compat::tests::*`.
// Mostly: misc, object state, objects.

    #[test]
    fn resort_queues_explicit_context_and_global_category_work() {
        let world = HostWorldContext::from_objects(vec![fixture_world_object(
            ObjectId::new(2),
            "Dummy")],
        );
        let (result, outcome) = with_object_host_context_with_world(world.clone(), || {
            assert_eq!(resort(&[])?, Value::Nil);
            assert_eq!(resort(&[Value::Object(2)])?, Value::Nil);
            Ok::<_, RuntimeError>(())
        });
        result.expect("object Resort calls succeed");
        assert_eq!(
            outcome.object_order_commands,
            [
                ObjectOrderCommand::ResortObject(ObjectId::new(1)),
                ObjectOrderCommand::ResortObject(ObjectId::new(2)),
            ]
        );

        let (result, outcome) = with_effect_context(None, &[], world, 3, || resort(&[]));
        assert_eq!(result.expect("global Resort succeeds"), Value::Nil);
        assert_eq!(
            outcome.object_order_commands,
            [ObjectOrderCommand::SortByCategory]
        );
    }

    #[test]
    fn legacy_butterfly_helpers_match_cpp() {
        // The tutorial `_BTF` script uses the old wrapper functions registered
        // by C4Script.cpp:6679-6701 and caller Var slots from :3372-3396.
        // Missing operands are the VM's nil slots and convert to 0/false.
        let mut engine = clonk_script::Engine::new();
        register_host_functions(&mut engine);
        engine
            .load_script(
                r#"
                #strict
                func Probe() {
                    SetVar(0, Sum(1, 2));
                    SetVar(1, GreaterThan(4));
                    SetVar(2, LessThan(-1));
                    return [Var(0), Var(1), Var(2), SEqual("Fly", "Fly"),
                            Not(0), Or(0, 0, 1)];
                }
                "#,
            )
            .expect("legacy helper probe compiles");

        assert_eq!(
            engine.call("Probe", &[]).expect("legacy helpers run"),
            Value::Array(vec![
                Value::Int(3),
                Value::Int(1),
                Value::Int(1),
                Value::Int(1),
                Value::Bool(true),
                Value::Bool(true),
            ])
        );

        let split_utf8 = format!(
            "{}{}",
            clonk_script::c4_string_from_bytes(&[0xc3]),
            clonk_script::c4_string_from_bytes(&[0xbf])
        );
        assert_eq!(
            legacy_s_equal(&[
                Value::String("\u{ff}".into()),
                Value::String(split_utf8.into())
            ])
            .expect("byte-equivalent strings compare"),
            Value::Int(1)
        );
    }

    #[test]
    fn set_var_matches_c4_value_list_index_growth_and_limit() {
        let mut engine = clonk_script::Engine::new();
        register_host_functions(&mut engine);
        engine
            .load_script(
                r#"
                #strict 3
                func High() { return [SetVar(15, 42), Var(15)]; }
                func Negative() { Var(0) = 1; return [SetVar(-1, 7), Var(0), Var(-1)]; }
                func Last() { return [SetVar(999999, 9), Var(999999)]; }
                func TooLarge() { return SetVar(1000000, 9); }
                "#,
            )
            .expect("SetVar boundary probe compiles");

        assert_eq!(
            engine.call("High", &[]).expect("high SetVar index runs"),
            Value::Array(vec![Value::Int(42), Value::Int(42)])
        );
        assert_eq!(
            engine
                .call("Negative", &[])
                .expect("negative SetVar index runs"),
            Value::Array(vec![Value::Int(7), Value::Int(7), Value::Int(7)])
        );
        assert_eq!(
            engine
                .call("Last", &[])
                .expect("last valid SetVar index runs"),
            Value::Array(vec![Value::Int(9), Value::Int(9)])
        );
        match engine.call("TooLarge", &[]) {
            Err(clonk_script::ScriptError::Runtime(error)) => {
                assert_eq!(error.message(), "out of memory")
            }
            other => panic!("expected SetVar allocation limit error, got {other:?}"),
        }

        // FnSetVar returns nil before touching NumVars when there is no caller.
        assert_eq!(
            set_var(&[Value::Int(LEGACY_MAX_ARRAY_SIZE), Value::Int(9),])
                .expect("SetVar without a caller succeeds"),
            Value::Nil
        );
    }

    #[test]
    fn dec_var_decrements_the_callers_var_slot() {
        let mut engine = clonk_script::Engine::new();
        register_host_functions(&mut engine);
        engine
            .load_script(
                r#"
                #strict 3
                func Probe() {
                    SetVar(0, 5);
                    return [DecVar(0), Var(0), DecVar(1), Var(1)];
                }
                "#,
            )
            .expect("DecVar caller-slot probe compiles");

        assert_eq!(
            engine.call("Probe", &[]).expect("DecVar probe runs"),
            Value::Array(vec![
                Value::Int(4),
                Value::Int(4),
                Value::Int(-1),
                Value::Int(-1),
            ])
        );
    }

    #[test]
    fn dec_var_without_a_script_caller_returns_nil() {
        assert_eq!(
            dec_var(&[Value::Int(0)]).expect("DecVar without a caller succeeds"),
            Value::Nil
        );
    }

    #[test]
    fn inc_var_prefix_increments_the_immediate_callers_var_slot() {
        let mut engine = clonk_script::Engine::new();
        register_host_functions(&mut engine);
        engine
            .load_script(
                r#"
                #strict 3
                func Inner() {
                    SetVar(0, 4);
                    var result = IncVar(0);
                    var unset = IncVar(1);
                    SetVar(2, true);
                    var boolean = IncVar(2);
                    var negative = IncVar(-1);
                    return [result, Var(0), unset, Var(1), boolean, Var(2), negative, Var(0)];
                }
                func Outer() {
                    SetVar(0, 90);
                    return [Inner(), Var(0)];
                }
                func Last() {
                    SetVar(999999, 9);
                    return [IncVar(999999), Var(999999)];
                }
                func TooLarge() { return IncVar(1000000); }
                func Surplus() {
                    var result = IncVar(3, SetVar(4, 7));
                    return [result, Var(3), Var(4)];
                }
                "#,
            )
            .expect("IncVar caller-slot probe compiles");

        assert_eq!(
            engine.call("Outer", &[]).expect("nested IncVar probe runs"),
            Value::Array(vec![
                Value::Array(vec![
                    Value::Int(5),
                    Value::Int(6),
                    Value::Int(1),
                    Value::Int(1),
                    Value::Int(2),
                    Value::Int(2),
                    Value::Int(6),
                    Value::Int(6),
                ]),
                Value::Int(90),
            ]),
            "IncVar mutates only the immediate script caller's NumVars"
        );
        assert_eq!(
            engine.call("Last", &[]).expect("last slot increments"),
            Value::Array(vec![Value::Int(10), Value::Int(10)])
        );
        match engine.call("TooLarge", &[]) {
            Err(clonk_script::ScriptError::Runtime(error)) => {
                assert_eq!(error.message(), "out of memory")
            }
            other => panic!("expected IncVar allocation limit error, got {other:?}"),
        }
        assert_eq!(
            engine
                .call("Surplus", &[])
                .expect("surplus argument is evaluated then discarded"),
            Value::Array(vec![Value::Int(1), Value::Int(1), Value::Int(7)])
        );
    }

    #[test]
    fn inc_var_without_a_script_caller_returns_nil_before_slot_access() {
        let mut engine = clonk_script::Engine::new();
        register_host_functions(&mut engine);
        assert_eq!(
            engine
                .call("IncVar", &[Value::Int(LEGACY_MAX_ARRAY_SIZE)])
                .expect("direct IncVar succeeds without a caller"),
            Value::Nil,
            "the no-caller return precedes the NumVars allocation limit check"
        );
    }

    #[test]
    fn fire_potion_varn_rotation_loop_matches_cpp() {
        let mut engine = clonk_script::Engine::new();
        register_host_functions(&mut engine);
        engine
            .load_script(
                r#"
                #strict 3
                func Probe() {
                    var pitch = 90, yaw = 90, roll = 90;
                    var zoom = 14;

                    var point1x = -zoom, point1y = 0, point1z = 0;
                    var point2x = +zoom, point2y = 0, point2z = 0;
                    var point3x = 0, point3y = -zoom, point3z = 0;
                    var point4x = 0, point4y = +zoom, point4z = 0;
                    var point5x = 0, point5y = 0, point5z = +zoom;
                    var point6x = 0, point6y = 0, point6z = -zoom;

                    for (var i = 1; i <= 6; i++) {
                        var tempx = VarN(Format("point%dx", i));
                        var tempy = VarN(Format("point%dy", i));
                        VarN(Format("point%dx", i)) = Cos(roll, tempx) - Sin(roll, tempy);
                        VarN(Format("point%dy", i)) = Cos(roll, tempy) + Sin(roll, tempx);
                    }
                    for (var i = 1; i <= 6; i++) {
                        var tempy = VarN(Format("point%dy", i));
                        var tempz = VarN(Format("point%dz", i));
                        VarN(Format("point%dy", i)) = Cos(pitch, tempy) - Sin(pitch, tempz);
                        VarN(Format("point%dz", i)) = Cos(pitch, tempz) + Sin(pitch, tempy);
                    }
                    for (var i = 1; i <= 6; i++) {
                        var tempx = VarN(Format("point%dx", i));
                        var tempz = VarN(Format("point%dz", i));
                        VarN(Format("point%dx", i)) = Cos(yaw, tempx) - Sin(yaw, tempz);
                        VarN(Format("point%dz", i)) = Cos(yaw, tempz) + Sin(yaw, tempx);
                    }

                    return [
                        point1x, point1y, point1z,
                        point2x, point2y, point2z,
                        point3x, point3y, point3z,
                        point4x, point4y, point4z,
                        point5x, point5y, point5z,
                        point6x, point6y, point6z
                    ];
                }
                "#,
            )
            .expect("reduced Fire potion rotation loop compiles");

        assert_eq!(
            engine.call("Probe", &[]).expect("Fire rotation loop runs"),
            Value::Array(vec![
                Value::Int(14),
                Value::Int(0),
                Value::Int(0),
                Value::Int(-14),
                Value::Int(0),
                Value::Int(0),
                Value::Int(0),
                Value::Int(0),
                Value::Int(14),
                Value::Int(0),
                Value::Int(0),
                Value::Int(-14),
                Value::Int(0),
                Value::Int(-14),
                Value::Int(0),
                Value::Int(0),
                Value::Int(14),
                Value::Int(0),
            ])
        );
    }

    #[test]
    fn script_sin_and_cos_use_the_cpp_fixed_point_table() {
        // FnSin/FnCos convert the angle with itofix(angle, precision), use
        // C4Fixed's shared SineTable, then fixtoi(result, radius)
        // (C4Script.cpp:3224-3238; Fixed.h:188-218). Host floating point is
        // not lockstep-safe and differs by whole pixels at rounding edges.
        // Gold Rush frame 14367 pins the smallest such edge: WMPF #4052 is
        // placed by Cos(300, 1). SineTable[3000] is 32767, so C++ rounds to
        // zero; binary floating point rounded the mathematical 0.5 to one.
        assert_eq!(
            cos_func(&[Value::Int(300), Value::Int(1)]).expect("Cos succeeds"),
            Value::Int(0)
        );

        // Missing/nil integer parameters convert to zero. FnSin/FnCos only
        // default precision, not radius (C4Script.cpp:3224-3238).
        for args in [vec![Value::Int(90)], vec![Value::Int(90), Value::Nil]] {
            assert_eq!(sin_func(&args).expect("Sin succeeds"), Value::Int(0));
        }
        for args in [vec![Value::Int(0)], vec![Value::Int(0), Value::Nil]] {
            assert_eq!(cos_func(&args).expect("Cos succeeds"), Value::Int(0));
        }

        for angle in -359..=359 {
            for radius in -100..=100 {
                let fixed_angle = itofix_prec(angle, 1);
                let expected_sin = fixtoi_prec(fixed_angle.sin_deg(), radius);
                let expected_cos = fixtoi_prec(fixed_angle.cos_deg(), radius);
                assert_eq!(
                    sin_func(&[Value::Int(angle), Value::Int(radius)]).expect("Sin succeeds"),
                    Value::Int(expected_sin),
                    "Sin({angle}, {radius})"
                );
                assert_eq!(
                    cos_func(&[Value::Int(angle), Value::Int(radius)]).expect("Cos succeeds"),
                    Value::Int(expected_cos),
                    "Cos({angle}, {radius})"
                );
            }
        }
    }

    #[test]
    fn arc_sin_and_arc_cos_match_cpp_inverse_trig_and_rounding() {
        // FnArcSin/FnArcCos use double-precision libm in degrees and round
        // with floor(angle + 0.5), including for negative angles
        // (C4Script.cpp:3276-3298). The 10_000-radius pairs straddle the
        // 30.5-degree boundary on either side.
        let mut engine = clonk_script::Engine::new();
        register_host_functions(&mut engine);
        engine
            .load_script(
                r#"
                #strict
                func Probe() {
                    return [
                        ArcCos(0, 100), ArcCos(100, 100),
                        ArcCos(-100, 100), ArcCos(50, 100),
                        ArcCos(7, 0), ArcCos(101, 100),
                        ArcSin(50, 100), ArcSin(7, 0),
                        ArcCos(8617, 10000), ArcCos(8616, 10000),
                        ArcSin(-5075, 10000), ArcSin(-5076, 10000)
                    ];
                }
                "#,
            )
            .expect("inverse-trig probe compiles");

        assert_eq!(
            engine.call("Probe", &[]).expect("inverse trig runs"),
            Value::Array(vec![
                Value::Int(90),
                Value::Int(0),
                Value::Int(180),
                Value::Int(60),
                Value::Int(0),
                Value::Int(0),
                Value::Int(30),
                Value::Int(0),
                Value::Int(30),
                Value::Int(31),
                Value::Int(-30),
                Value::Int(-31),
            ])
        );
        assert_eq!(round_inverse_angle(30.5), 31);
        assert_eq!(round_inverse_angle(-30.5), -30);
    }

    #[test]
    fn sqrt_reproduces_cpp_correction_steps_including_the_wrapped_product() {
        // FnSqrt (C4Script.cpp:3240-3247) truncates the double root, then
        // nudges it up and back down with two `iSqrt * iSqrt` comparisons. On
        // `C4ValueInt = int32_t` (C4Value.h:62) the second product wraps for
        // any input above the largest representable square 46340^2, so the
        // decrement never fires and C++ returns one *more* than floor(sqrt).
        // Every value in [2147395601, 2147483647] takes that branch.
        let mut engine = clonk_script::Engine::new();
        register_host_functions(&mut engine);
        engine
            .load_script(
                r#"
                #strict
                func Probe() {
                    return [
                        Sqrt(-100), Sqrt(0), Sqrt(1), Sqrt(2),
                        Sqrt(15), Sqrt(16), Sqrt(24), Sqrt(25),
                        Sqrt(2147395599), Sqrt(2147395600),
                        Sqrt(2147395601), Sqrt(2147450880),
                        Sqrt(2147483647)
                    ];
                }
                "#,
            )
            .expect("Sqrt probe compiles");

        assert_eq!(
            engine.call("Probe", &[]).expect("Sqrt runs"),
            Value::Array(vec![
                Value::Int(0),
                Value::Int(0),
                Value::Int(1),
                Value::Int(1),
                Value::Int(3),
                Value::Int(4),
                Value::Int(4),
                Value::Int(5),
                Value::Int(46339),
                Value::Int(46340),
                Value::Int(46341),
                Value::Int(46341),
                Value::Int(46341),
            ])
        );
    }

    fn empty_state() -> Value {
        let mut map = ValueMap::new();
        map.insert("effects".into(), Value::Array(Vec::new()));
        Value::Proplist(map)
    }

    fn with_object_host_context<F, T>(func: F) -> (Result<T, RuntimeError>, EffectContextOutcome)
    where
        F: FnOnce() -> Result<T, RuntimeError>,
    {
        with_object_host_context_with_world(HostWorldContext::default(), func)
    }

    fn object_host_context_with_physical_energy(
        energy: i32,
        maximum: i32,
    ) -> HostObjectContext<'static> {
        HostObjectContext {
            energy,
            ..idle_object_context()
        }
        .with_physicals(
            None,
            None,
            Vec::new(),
            PhysicalInfo {
                energy: maximum,
                ..PhysicalInfo::default()
            },
        )
    }

    fn with_object_host_context_actions<F, T>(
        actions: &[&str],
        func: F,
    ) -> (Result<T, RuntimeError>, EffectContextOutcome)
    where
        F: FnOnce() -> Result<T, RuntimeError>,
    {
        with_object_host_context_actions_and_ticks(actions, 0, func)
    }

    fn with_object_host_context_actions_and_ticks<F, T>(
        actions: &[&str],
        action_ticks: i32,
        func: F,
    ) -> (Result<T, RuntimeError>, EffectContextOutcome)
    where
        F: FnOnce() -> Result<T, RuntimeError>,
    {
        let specs = actions
            .iter()
            .map(|name| ((*name).to_string(), ActionSpec::default()))
            .collect();
        with_effect_context(
            Some(HostObjectContext {
                action_ticks,
                action_library: ActionLibrary::new(None, specs).into(),
                ..idle_object_context()
            }),
            &[],
            HostWorldContext::default(),
            1,
            func,
        )
    }

    fn with_object_host_context_with_world<F, T>(
        world: HostWorldContext,
        func: F,
    ) -> (Result<T, RuntimeError>, EffectContextOutcome)
    where
        F: FnOnce() -> Result<T, RuntimeError>,
    {
        with_object_host_context_with_world_and_next_id(world, 1, func)
    }

    fn with_object_host_context_with_world_and_next_id<F, T>(
        world: HostWorldContext,
        next_object_id: u64,
        func: F,
    ) -> (Result<T, RuntimeError>, EffectContextOutcome)
    where
        F: FnOnce() -> Result<T, RuntimeError>,
    {
        with_effect_context(
            Some(idle_object_context()),
            &[],
            world,
            next_object_id,
            func,
        )
    }

    fn with_environment_context<F, T>(
        settings: EnvironmentSettings,
        frame: u64,
        func: F,
    ) -> (Result<T, RuntimeError>, EnvironmentDelta)
    where
        F: FnOnce() -> Result<T, RuntimeError>,
    {
        let guard = enter_environment_context(settings, frame);
        let result = func();
        let delta = guard.finish();
        (result, delta)
    }

    fn with_physics_context<F, T>(
        settings: PhysicsSettings,
        func: F,
    ) -> (Result<T, RuntimeError>, PhysicsDelta)
    where
        F: FnOnce() -> Result<T, RuntimeError>,
    {
        let guard = enter_physics_context(settings);
        let result = func();
        let delta = guard.finish();
        (result, delta)
    }

    #[derive(Debug)]
    struct RecordedEvent {
        level: Level,
        target: String,
        message: String,
    }

    #[derive(Clone)]
    struct RecordingLayer {
        records: Arc<Mutex<Vec<RecordedEvent>>>,
    }

    impl RecordingLayer {
        fn new(records: Arc<Mutex<Vec<RecordedEvent>>>) -> Self {
            Self { records }
        }
    }

    impl<S> Layer<S> for RecordingLayer
    where
        S: tracing::Subscriber,
    {
        fn on_event(&self, event: &tracing::Event<'_>, _ctx: Context<'_, S>) {
            let mut visitor = MessageVisitor::default();
            event.record(&mut visitor);
            let message = visitor.message.unwrap_or_default();
            let record = RecordedEvent {
                level: *event.metadata().level(),
                target: event.metadata().target().to_string(),
                message,
            };
            self.records.lock().unwrap().push(record);
        }
    }

    #[derive(Default)]
    struct MessageVisitor {
        message: Option<String>,
    }

    impl Visit for MessageVisitor {
        fn record_debug(&mut self, field: &Field, value: &dyn fmt::Debug) {
            if field.name() == "message" {
                let mut text = format!("{value:?}");
                if let Some(stripped) = text
                    .strip_prefix('"')
                    .and_then(|inner| inner.strip_suffix('"'))
                {
                    text = stripped.to_string();
                }
                self.message = Some(text);
            }
        }

        fn record_str(&mut self, field: &Field, value: &str) {
            if field.name() == "message" {
                self.message = Some(value.to_string());
            }
        }
    }

    #[test]
    fn get_keys_returns_c4valuehash_key_order() {
        let mut map = ValueMap::new();
        map.insert("beta".into(), Value::Int(2));
        map.insert("alpha".into(), Value::Int(1));
        map.insert_key(Value::Int(7), Value::String("seven".into()));
        let result = get_keys(&[Value::Proplist(map)]).expect("GetKeys succeeds");
        match result {
            Value::Array(entries) => {
                assert_eq!(
                    entries,
                    vec![
                        Value::String("beta".into()),
                        Value::String("alpha".into()),
                        Value::Int(7),
                    ]
                );
            }
            other => panic!("expected array, got {:?}", other),
        }
    }

    #[test]
    fn map_data_string_uses_typed_keys_in_insertion_order() {
        let mut map = ValueMap::new();
        map.insert("b".into(), Value::Int(2));
        map.insert("a".into(), Value::Int(1));
        map.insert_key(Value::Int(7), Value::String("seven".into()));

        assert_eq!(
            value_to_data_string(&Value::Proplist(map)),
            r#"{ "b" = 2, "a" = 1, 7 = "seven" }"#
        );
    }

    #[test]
    fn removed_direct_object_map_entries_are_erased_but_nested_refs_are_cleared() {
        let removed_id = ObjectId::new(7);
        let mut map = ValueMap::new();
        map.insert_key(Value::Object(7), Value::Int(1));
        map.insert("direct_value".into(), Value::Object(7));
        map.insert_key(
            Value::Array(vec![Value::Object(7)]),
            Value::String("nested_key".into()),
        );
        map.insert("nested_value".into(), Value::Array(vec![Value::Object(7)]));
        map.insert_key(Value::Object(8), Value::String("live".into()));
        let mut value = Value::Proplist(map);

        clear_removed_object_references(&mut value, &HashSet::from([removed_id]));

        let Value::Proplist(map) = value else {
            panic!("map remains a map");
        };
        assert_eq!(map.len(), 3);
        assert_eq!(
            map.get_key(&Value::Array(vec![Value::Nil])),
            Some(&Value::String("nested_key".into()))
        );
        assert_eq!(
            map.get("nested_value"),
            Some(&Value::Array(vec![Value::Nil]))
        );
        assert_eq!(
            map.get_key(&Value::Object(8)),
            Some(&Value::String("live".into()))
        );
    }

    #[test]
    fn effect_var_json_round_trip_preserves_map_key_order_and_types() {
        let mut map = ValueMap::new();
        map.insert("beta".into(), Value::Int(2));
        map.insert("alpha".into(), Value::Int(1));
        map.insert_key(Value::Bool(true), Value::String("typed".into()));
        let original = Value::Proplist(map);

        let effect_value = value_to_effect_var(&original);
        let json = serde_json::to_string(&effect_value).expect("effect value serializes");
        let restored: EffectVarValue =
            serde_json::from_str(&json).expect("effect value deserializes");
        let restored = effect_var_to_value(&restored);

        assert_eq!(restored, original);
        let Value::Proplist(restored) = restored else {
            unreachable!();
        };
        assert_eq!(
            restored.keys().cloned().collect::<Vec<_>>(),
            vec![
                Value::String("beta".into()),
                Value::String("alpha".into()),
                Value::Bool(true),
            ]
        );
    }

    #[test]
    fn effect_var_runtime_conversion_preserves_hidden_map_slots() {
        let hidden = clonk_script::C4StringValue::new("retained".to_owned());
        let mut map = ValueMap::new();
        map.recycle_value_slot(Value::String(hidden.clone()));

        let effect_value = value_to_effect_var(&Value::Proplist(map));
        let EffectVarValue::Proplist(effect_map) = &effect_value else {
            panic!("effect value remains a map");
        };
        let Some(Value::String(effect_hidden)) = effect_map.hidden_values().next() else {
            panic!("effect map retains its detached mapped slot");
        };
        assert!(effect_hidden.ptr_eq(&hidden));

        let Value::Proplist(mut restored) = effect_var_to_value(&effect_value) else {
            panic!("script value remains a map");
        };
        let Some(Value::String(restored_hidden)) = restored.hidden_values().next() else {
            panic!("script map restores its detached mapped slot");
        };
        assert!(restored_hidden.ptr_eq(&hidden));

        restored.insert_key(Value::Int(1), Value::Nil);
        assert!(
            restored.is_empty(),
            "assigning nil to a reused nonnil slot removes the new key"
        );
        assert_eq!(
            restored.hidden_values().cloned().collect::<Vec<_>>(),
            vec![Value::Nil]
        );
    }

    #[test]
    fn no_container_and_any_container_return_cpp_sentinels() {
        // FnNoContainer/FnAnyContainer return the FindObject container
        // sentinels NO_CONTAINER=124 / ANY_CONTAINER=123 (C4Object.h:83-84,
        // C4Script.cpp:6731-6732).
        assert_eq!(no_container(&[]).expect("NoContainer"), Value::Int(124));
        assert_eq!(any_container(&[]).expect("AnyContainer"), Value::Int(123));
    }

    #[test]
    fn act_idle_without_context_is_nil() {
        // FnActIdle returns nullopt -> nil without an object
        // (C4Script.cpp:1831-1836).
        assert_eq!(act_idle(&[]).expect("ActIdle"), Value::Nil);
    }

    #[test]
    fn check_energy_need_chain_validates_object_arguments_and_has_nil_without_context() {
        // FnCheckEnergyNeedChain declares one C4Object* parameter, defaults a
        // null parameter to cthr->Obj, and returns nullopt when both are null
        // (C4Script.cpp:1832-1837). The Aul parameter conversion rejects a
        // truthy string before the native function runs.
        assert_eq!(
            check_energy_need_chain(&[]).expect("no-context probe"),
            Value::Nil
        );
        let error = check_energy_need_chain(&[Value::String("not an object".to_string().into())])
            .expect_err("truthy strings are not object parameters");
        assert!(
            error.message().contains("expected object"),
            "unexpected error: {}",
            error.message()
        );
    }

    #[test]
    fn get_act_map_val_reflects_the_complete_cpp_entry_table_and_indexes() {
        assert!(get_act_map_val(&[
            Value::Nil,
            Value::Nil,
            Value::Nil,
            Value::String("bad index".to_string().into()),
        ])
        .expect_err("native entry_nr conversion precedes body lookup")
        .message()
        .contains("expected int"));
        let mut action = clonk_resources::ActionDefinition::default();
        action.procedure = Some("ODD".to_string());
        action.directions = Some(12);
        action.flip_dir = Some(13);
        action.length = Some(14);
        action.attach = 15;
        action.delay = Some(16);
        action.facet = Some(clonk_resources::definition::ActionFacet {
            x: 1,
            y: 2,
            width: 20,
            height: 30,
            target_x: 4,
            target_y: 5,
        });
        action.facet_base = true;
        action.facet_top_face = true;
        action.facet_target_stretch = true;
        action.next_action = Some("Missing".to_string());
        action.no_other_action = true;
        action.end_call = Some("End".to_string());
        action.abort_call = Some("Abort".to_string());
        action.phase_call = Some("Phase".to_string());
        action.sound = Some("TravelSound".to_string());
        action.disabled = true;
        action.dig_free = Some(9);
        action.energy_usage = -10;
        action.in_liquid_action = Some("Swim".to_string());
        action.turn_action = Some("Turn".to_string());
        action.reverse = true;
        action.step = Some(11);
        action.reflected_ints = HashMap::from([
            ("Directions".to_string(), -2),
            ("FlipDir".to_string(), -3),
            ("Length".to_string(), -4),
            ("Attach".to_string(), -5),
            ("Delay".to_string(), -6),
            ("FacetBase".to_string(), 2),
            ("FacetTopFace".to_string(), -7),
            ("FacetTargetStretch".to_string(), 3),
            ("NoOtherAction".to_string(), 2),
            ("ObjectDisabled".to_string(), -8),
            ("DigFree".to_string(), -9),
            ("EnergyUsage".to_string(), -10),
            ("Reverse".to_string(), 2),
            ("Step".to_string(), -11),
        ]);

        let default_action = clonk_resources::ActionDefinition::default();
        let mut action_library = ActionLibrary::new(
            None,
            HashMap::from([
                ("Laser".to_string(), ActionSpec::default()),
                ("Defaults".to_string(), ActionSpec::default()),
                (String::new(), ActionSpec::default()),
            ]),
        );
        action_library.set_reflections(HashMap::from([
            (
                "Laser".to_string(),
                crate::action::C4ActionReflection::from_resource("Laser", &action),
            ),
            (
                "Defaults".to_string(),
                crate::action::C4ActionReflection::from_resource("Defaults", &default_action),
            ),
            (
                String::new(),
                crate::action::C4ActionReflection::from_resource("", &default_action),
            ),
        ]));
        let metadata = DefinitionMetadata {
            action_library: action_library.into(),
            ..DefinitionMetadata::default()
        };
        let world = HostWorldContext::default().with_definition_metadata(Rc::new(HashMap::from([
            (DefinitionId::from("TST1"), metadata),
            (DefinitionId::from("EMP1"), DefinitionMetadata::default()),
        ])));
        let (result, _) = with_definition_effect_context_with_state(
            DefinitionId::from("TST1"),
            &[],
            world,
            1,
            false,
            || {
                let query = |entry: &str, action: &str, id: Value, index: i32| {
                    get_act_map_val(&[
                        Value::String(entry.to_string().into()),
                        Value::String(action.to_string().into()),
                        id,
                        Value::Int(index),
                    ])
                };
                Ok::<Value, RuntimeError>(Value::Array(vec![
                    query("Name", "Laser", Value::C4Id("TST1".into()), 0)?,
                    query("Procedure", "Laser", Value::C4Id("TST1".into()), 0)?,
                    query("Directions", "Laser", Value::C4Id("TST1".into()), 0)?,
                    query("FlipDir", "Laser", Value::C4Id("TST1".into()), 0)?,
                    query("Length", "Laser", Value::C4Id("TST1".into()), 0)?,
                    query("Attach", "Laser", Value::C4Id("TST1".into()), 0)?,
                    query("Delay", "Laser", Value::C4Id("TST1".into()), 0)?,
                    query("Facet", "Laser", Value::C4Id("TST1".into()), 0)?,
                    query("Facet", "Laser", Value::C4Id("TST1".into()), 1)?,
                    query("Facet", "Laser", Value::C4Id("TST1".into()), 2)?,
                    query("Facet", "Laser", Value::C4Id("TST1".into()), 3)?,
                    query("Facet", "Laser", Value::C4Id("TST1".into()), 4)?,
                    query("Facet", "Laser", Value::C4Id("TST1".into()), 5)?,
                    query("FacetBase", "Laser", Value::C4Id("TST1".into()), 0)?,
                    query("FacetTopFace", "Laser", Value::C4Id("TST1".into()), 0)?,
                    query("FacetTargetStretch", "Laser", Value::C4Id("TST1".into()), 0)?,
                    query("NextAction", "Laser", Value::C4Id("TST1".into()), 0)?,
                    query("NoOtherAction", "Laser", Value::C4Id("TST1".into()), 0)?,
                    query("StartCall", "Laser", Value::C4Id("TST1".into()), 0)?,
                    query("EndCall", "Laser", Value::C4Id("TST1".into()), 0)?,
                    query("AbortCall", "Laser", Value::C4Id("TST1".into()), 0)?,
                    query("PhaseCall", "Laser", Value::C4Id("TST1".into()), 0)?,
                    query("Sound", "Laser", Value::C4Id("TST1".into()), 0)?,
                    query("ObjectDisabled", "Laser", Value::C4Id("TST1".into()), 0)?,
                    query("DigFree", "Laser", Value::C4Id("TST1".into()), 0)?,
                    query("EnergyUsage", "Laser", Value::C4Id("TST1".into()), 0)?,
                    query("InLiquidAction", "Laser", Value::C4Id("TST1".into()), 0)?,
                    query("TurnAction", "Laser", Value::C4Id("TST1".into()), 0)?,
                    query("Reverse", "Laser", Value::C4Id("TST1".into()), 0)?,
                    query("Step", "Laser", Value::C4Id("TST1".into()), 0)?,
                    query("Sound", "Defaults", Value::Nil, 0)?,
                    query("Directions", "Defaults", Value::Nil, 0)?,
                    query("Facet", "Defaults", Value::Nil, 5)?,
                    query("Facet", "Laser", Value::Nil, 6)?,
                    query("Length", "Laser", Value::Nil, 1)?,
                    query("Length", "Laser", Value::Nil, -1)?,
                    query("Disabled", "Laser", Value::Nil, 0)?,
                    query("Length", "laser", Value::Nil, 0)?,
                    query("Length", "Idle", Value::Nil, 0)?,
                    query("Length", "Idle", Value::C4Id("EMP1".into()), 0)?,
                    get_act_map_val(&[
                        Value::String("Length".to_string().into()),
                        Value::Nil,
                        Value::C4Id("TST1".into()),
                        Value::Int(0),
                    ])?,
                    query("Length", "Laser", Value::C4Id("MISS".into()), 0)?,
                ]))
            },
        );

        assert_eq!(
            result.expect("complete GetActMapVal probes succeed"),
            Value::Array(vec![
                Value::String("Laser".into()),
                Value::String("ODD".into()),
                Value::Int(-2),
                Value::Int(-3),
                Value::Int(-4),
                Value::Int(-5),
                Value::Int(-6),
                Value::Int(1),
                Value::Int(2),
                Value::Int(20),
                Value::Int(30),
                Value::Int(4),
                Value::Int(5),
                Value::Int(2),
                Value::Int(-7),
                Value::Int(3),
                Value::String("Missing".into()),
                Value::Int(2),
                Value::String(String::new().into()),
                Value::String("End".into()),
                Value::String("Abort".into()),
                Value::String("Phase".into()),
                Value::String("TravelSound".into()),
                Value::Int(-8),
                Value::Int(-9),
                Value::Int(-10),
                Value::String("Swim".into()),
                Value::String("Turn".into()),
                Value::Int(2),
                Value::Int(-11),
                Value::String(String::new().into()),
                Value::Int(1),
                Value::Int(0),
                Value::Nil,
                Value::Nil,
                Value::Nil,
                Value::Nil,
                Value::Nil,
                Value::Nil,
                Value::Nil,
                Value::Int(1),
                Value::Nil,
            ])
        );
    }

    #[test]
    fn def_core_c4id_reflection_uses_the_native_typed_payload() {
        let reflected = |value: Option<&str>| {
            DefCoreValueStore::c4id(value)
                .into_iter()
                .next()
                .expect("C4ID entries always expose one compiler primitive")
                .into_value()
        };
        let typed = |value: &str| {
            Value::C4Id(clonk_script::c4_id_from_raw(clonk_script::c4_id_parse(
                value,
            )))
        };

        assert_eq!(reflected(Some("NONE")), Value::Nil);
        assert_eq!(reflected(Some("0000")), Value::Nil);
        assert_eq!(reflected(Some("none")), typed("none"));
        assert_eq!(reflected(Some("AB-C")), typed("AB-C"));
        assert_eq!(reflected(Some("0001")), Value::C4Id("0001".to_string()));
    }

    #[test]
    fn get_def_core_val_reflects_line_type_and_connection_bits() {
        // GetDefCoreVal reflects the named C4Def compiler entries
        // (C4Script.cpp:4170-4180). Line and LineConnect are compiled as
        // bitfields in C4Def::CompileFunc (C4Def.cpp:333-351); LNKT reads
        // both to choose and finish real lines (Linekit.c4d/Script.c:61-96,
        // 108-173).
        let world = HostWorldContext::with_landscape(
            Vec::<HostWorldObject>::new(),
            None,
            HashMap::from([(
                DefinitionId::from("PWRL"),
                DefinitionMetadata {
                    line: 1,
                    line_connect: crate::LINE_CONNECT_POWER_INPUT
                        | crate::LINE_CONNECT_POWER_OUTPUT,
                    ..DefinitionMetadata::default()
                },
            )]),
            Vec::new(),
            HashMap::new(),
            HashMap::new(),
            1,
            false,
        );
        let (result, _) = with_effect_context(None, &[], world, 1, || {
            Ok::<Value, RuntimeError>(Value::Array(vec![
                get_def_core_val(&[
                    Value::String("Line".into()),
                    Value::String("DefCore".into()),
                    Value::C4Id("PWRL".into()),
                ])?,
                get_def_core_val(&[
                    Value::String("LineConnect".into()),
                    Value::String("DefCore".into()),
                    Value::C4Id("PWRL".into()),
                ])?,
            ]))
        });

        assert_eq!(
            result.expect("GetDefCoreVal succeeds"),
            Value::Array(vec![
                Value::Int(1),
                Value::Int(
                    (crate::LINE_CONNECT_POWER_INPUT | crate::LINE_CONNECT_POWER_OUTPUT) as i32
                )
            ])
        );
    }

    #[test]
    fn get_def_core_val_reflects_container_fields_with_cpp_section_and_index_rules() {
        // FnGetDefCoreVal defaults a missing id to the executing definition
        // and reflects C4Def::CollectionLimit and GrabPutGet through the
        // DefCore compiler (C4Script.cpp:4170-4180; C4Def.cpp:311,364-373).
        // The compiler accepts a null/empty or exact `DefCore` section and
        // only scalar index zero; default scalar values are integer zero,
        // not nil.
        let world = HostWorldContext::default().with_definition_metadata(Rc::new(HashMap::from([
            (
                DefinitionId::from("KAJO"),
                DefinitionMetadata {
                    collection_limit: 5,
                    grab_put_get: 3,
                    ..DefinitionMetadata::default()
                },
            ),
            (DefinitionId::from("UNLM"), DefinitionMetadata::default()),
        ])));
        let (result, _) = with_definition_effect_context_with_state(
            DefinitionId::from("KAJO"),
            &[],
            world,
            1,
            false,
            || {
                Ok::<Value, RuntimeError>(Value::Array(vec![
                    get_def_core_val(&[Value::String("CollectionLimit".into())])?,
                    get_def_core_val(&[
                        Value::String("CollectionLimit".into()),
                        Value::String("DefCore".into()),
                        Value::C4Id("KAJO".into()),
                    ])?,
                    get_def_core_val(&[
                        Value::String("CollectionLimit".into()),
                        Value::Nil,
                        Value::C4Id("KAJO".into()),
                    ])?,
                    get_def_core_val(&[
                        Value::String("CollectionLimit".into()),
                        Value::String(String::new().into()),
                        Value::C4Id("KAJO".into()),
                    ])?,
                    get_def_core_val(&[
                        Value::String("CollectionLimit".into()),
                        Value::String("DefCore".into()),
                        Value::C4Id("UNLM".into()),
                    ])?,
                    get_def_core_val(&[Value::String("GrabPutGet".into())])?,
                    get_def_core_val(&[
                        Value::String("GrabPutGet".into()),
                        Value::String("DefCore".into()),
                        Value::C4Id("KAJO".into()),
                    ])?,
                    get_def_core_val(&[
                        Value::String("GrabPutGet".into()),
                        Value::Int(0),
                        Value::C4Id("KAJO".into()),
                    ])?,
                    get_def_core_val(&[
                        Value::String("GrabPutGet".into()),
                        Value::String("DefCore".into()),
                        Value::C4Id("UNLM".into()),
                    ])?,
                    get_def_core_val(&[
                        Value::String("CollectionLimit".into()),
                        Value::String("Other".into()),
                        Value::C4Id("KAJO".into()),
                    ])?,
                    get_def_core_val(&[
                        Value::String("CollectionLimit".into()),
                        Value::String("defcore".into()),
                        Value::C4Id("KAJO".into()),
                    ])?,
                    get_def_core_val(&[
                        Value::String("CollectionLimit".into()),
                        Value::String("DefCore".into()),
                        Value::C4Id("KAJO".into()),
                        Value::Int(1),
                    ])?,
                    get_def_core_val(&[
                        Value::String("CollectionLimit".into()),
                        Value::String("DefCore".into()),
                        Value::C4Id("KAJO".into()),
                        Value::Int(-1),
                    ])?,
                ]))
            },
        );

        assert_eq!(
            result.expect("GetDefCoreVal CollectionLimit probes succeed"),
            Value::Array(vec![
                Value::Int(5),
                Value::Int(5),
                Value::Int(5),
                Value::Int(5),
                Value::Int(0),
                Value::Int(3),
                Value::Int(3),
                Value::Int(3),
                Value::Int(0),
                Value::Nil,
                Value::Nil,
                Value::Nil,
                Value::Nil,
            ])
        );
    }

    #[test]
    fn get_def_core_val_reflects_rect_vertices_types_and_physical_section_like_cpp() {
        let mut definition = crate::Definition::from_script("TST1", "Reflection Test", "#strict 2")
            .expect("definition compiles");
        definition.set_version([5, 1, 2, 0, 0]);
        definition.require_defs = vec!["REQ1".to_string()];
        definition.max_user_select = 7;
        definition.set_shape_rect(Some(DefinitionRect::new(-5, 6, 70, 80)));
        definition.set_picture(Some(crate::DefinitionPicture {
            x: 21,
            y: 22,
            width: 23,
            height: 24,
        }));
        definition.set_entrance_rect(Some(DefinitionRect::new(11, 12, 13, 14)));
        definition.set_collection_rect(Some(DefinitionRect::new(31, 32, 33, 34)));
        definition.set_solid_mask(Some(crate::DefinitionTargetRect::new(
            41, 42, 43, 44, 45, 46,
        )));
        definition.set_shape_vertex_slots(
            3,
            &[
                ObjectVertex::new(10, -1).with_cnat(1).with_friction(4),
                ObjectVertex::new(0, 0),
                ObjectVertex::new(30, 9).with_cnat(3).with_friction(6),
            ],
        );
        definition.set_components(vec![crate::DefinitionComponent {
            id: DefinitionId::from("COMP"),
            count: -3,
        }]);
        definition.set_timer_call(Some("Tick".to_string()));
        definition.set_burn_turn_to(Some("BURN".to_string()));
        definition.set_base_auto_sell(true);
        definition.set_float_line(111);
        let mut physical = PhysicalInfo::default();
        physical.float = 222;
        physical.energy = 333;
        definition.set_physical(physical);
        definition.def_core_reflected_ints.extend([
            ("Exclusive".to_string(), 7),
            ("NoGet".to_string(), -8),
            ("Scale".to_string(), -5),
            ("CollectionLimit".to_string(), -4),
        ]);

        let metadata = DefinitionMetadata {
            fire: DefinitionFireMetadata {
                def_core_values: DefCoreValueStore::from_definition(&definition).into(),
                ..DefinitionFireMetadata::default()
            },
            ..DefinitionMetadata::default()
        };
        let world =
            HostWorldContext::default().with_definition_metadata(Rc::new(HashMap::from([(
                DefinitionId::from("TST1"),
                metadata,
            )])));
        let (result, _) = with_definition_effect_context_with_state(
            DefinitionId::from("TST1"),
            &[],
            world,
            1,
            false,
            || {
                let query = |entry: &str, section: Option<&str>, index: i32| {
                    get_def_core_val(&[
                        Value::String(entry.to_string().into()),
                        section
                            .map(|section| Value::String(section.to_string().into()))
                            .unwrap_or(Value::Nil),
                        Value::C4Id("TST1".to_string()),
                        Value::Int(index),
                    ])
                };
                Ok::<Value, RuntimeError>(Value::Array(vec![
                    query("Entrance", Some("DefCore"), 0)?,
                    query("Entrance", Some("DefCore"), 1)?,
                    query("Entrance", Some("DefCore"), 2)?,
                    query("Entrance", Some("DefCore"), 3)?,
                    query("Picture", Some("DefCore"), 0)?,
                    query("Picture", Some("DefCore"), 1)?,
                    query("Picture", Some("DefCore"), 2)?,
                    query("Picture", Some("DefCore"), 3)?,
                    query("Collection", Some("DefCore"), 0)?,
                    query("Collection", Some("DefCore"), 3)?,
                    query("VertexX", Some("DefCore"), 0)?,
                    query("VertexX", Some("DefCore"), 1)?,
                    query("VertexX", Some("DefCore"), 2)?,
                    query("VertexX", Some("DefCore"), 3)?,
                    query("Vertices", Some("DefCore"), 0)?,
                    query("Width", Some("DefCore"), 0)?,
                    query("Height", Some("DefCore"), 0)?,
                    query("Offset", Some("DefCore"), 0)?,
                    query("Offset", Some("DefCore"), 1)?,
                    query("SolidMask", Some("DefCore"), 5)?,
                    query("Version", Some("DefCore"), 2)?,
                    query("Version", Some("DefCore"), 3)?,
                    query("RequireDef", Some("DefCore"), 0)?,
                    query("Components", Some("DefCore"), 0)?,
                    query("Components", Some("DefCore"), 1)?,
                    query("TimerCall", Some("DefCore"), 0)?,
                    query("BurnTo", Some("DefCore"), 0)?,
                    query("BaseAutoSell", Some("DefCore"), 0)?,
                    query("Float", None, 0)?,
                    query("Float", None, 1)?,
                    query("Float", Some("Physical"), 0)?,
                    query("Energy", Some("Physical"), 0)?,
                    query("Exclusive", Some("DefCore"), 0)?,
                    query("NoGet", Some("DefCore"), 0)?,
                    query("Scale", Some("DefCore"), 0)?,
                    query("CollectionLimit", Some("DefCore"), 0)?,
                    query("Width", Some("DefCore"), 1)?,
                    query("Width", Some("defcore"), 0)?,
                    query("width", Some("DefCore"), 0)?,
                    query("Unknown", Some("DefCore"), 0)?,
                    query("Entrance", Some("DefCore"), -1)?,
                ]))
            },
        );

        assert_eq!(
            result.expect("GetDefCoreVal probes succeed"),
            Value::Array(vec![
                Value::Int(11),
                Value::Int(12),
                Value::Int(13),
                Value::Int(14),
                Value::Int(21),
                Value::Int(22),
                Value::Int(23),
                Value::Int(24),
                Value::Int(31),
                Value::Int(34),
                Value::Int(10),
                Value::Int(0),
                Value::Int(30),
                Value::Nil,
                Value::Int(3),
                Value::Int(70),
                Value::Int(80),
                Value::Int(-5),
                Value::Int(6),
                Value::Int(46),
                Value::Int(2),
                Value::Nil,
                Value::C4Id("REQ1".to_string()),
                Value::C4Id("COMP".to_string()),
                Value::Int(-3),
                Value::String("Tick".to_string().into()),
                Value::C4Id("BURN".to_string()),
                Value::Bool(true),
                Value::Int(111),
                Value::Int(222),
                Value::Int(222),
                Value::Int(333),
                Value::Int(7),
                Value::Int(-8),
                Value::Int(-5),
                Value::Int(-4),
                Value::Nil,
                Value::Nil,
                Value::Nil,
                Value::Nil,
                Value::Nil,
            ])
        );
    }

    #[test]
    fn def_core_value_store_covers_the_complete_cpp_compiler_entry_tables() {
        let definition = crate::Definition::from_script("TST1", "Reflection Test", "")
            .expect("definition compiles");
        let values = DefCoreValueStore::from_definition(&definition);
        let expected_def_core = [
            "id",
            "Version",
            "Name",
            "RequireDef",
            "Category",
            "MaxUserSelect",
            "Timer",
            "TimerCall",
            "ContactCalls",
            "Width",
            "Height",
            "Offset",
            "Vertices",
            "VertexX",
            "VertexY",
            "VertexCNAT",
            "VertexFriction",
            "ContactDensity",
            "FireTop",
            "Value",
            "Mass",
            "Components",
            "SolidMask",
            "TopFace",
            "Picture",
            "PictureFE",
            "Entrance",
            "Collection",
            "CollectionLimit",
            "Placement",
            "Exclusive",
            "ContactIncinerate",
            "BlastIncinerate",
            "BurnTo",
            "Base",
            "Line",
            "LineConnect",
            "LineIntersect",
            "Prey",
            "Edible",
            "CrewMember",
            "NoStandardCrew",
            "Growth",
            "Rebuy",
            "Construction",
            "ConstructTo",
            "Grab",
            "GrabPutGet",
            "Collectible",
            "Rotate",
            "RotatedEntrance",
            "Chop",
            "Float",
            "ContainBlast",
            "ColorByOwner",
            "ColorByMaterial",
            "HorizontalFix",
            "BorderBound",
            "LiftTop",
            "UprightAttach",
            "StretchGrowth",
            "Basement",
            "NoBurnDecay",
            "IncompleteActivity",
            "AttractLightning",
            "Oversize",
            "Fragile",
            "Explosive",
            "Projectile",
            "NoPushEnter",
            "DragImagePicture",
            "VehicleControl",
            "Pathfinder",
            "MoveToRange",
            "NoComponentMass",
            "NoStabilize",
            "ClosedContainer",
            "SilentCommands",
            "NoBurnDamage",
            "TemporaryCrew",
            "SmokeRate",
            "BlitMode",
            "NoBreath",
            "ConSizeOff",
            "NoSell",
            "NoGet",
            "NoFight",
            "RotatedSolidmasks",
            "NoTransferZones",
            "AutoContextMenu",
            "NeededGfxMode",
            "AllowPictureStack",
            "HideHUDBars",
            "HideHUDElements",
            "Scale",
            "BaseAutoSell",
        ];
        let expected_physical = [
            "Energy",
            "Breath",
            "Walk",
            "Jump",
            "Scale",
            "Hangle",
            "Dig",
            "Swim",
            "Throw",
            "Push",
            "Fight",
            "Magic",
            "Float",
            "CanScale",
            "CanHangle",
            "CanDig",
            "CanConstruct",
            "CanChop",
            "CanFly",
            "CorrosionResist",
            "BreatheWater",
        ];

        assert_eq!(
            values.def_core.keys().copied().collect::<HashSet<_>>(),
            expected_def_core.into_iter().collect::<HashSet<_>>()
        );
        assert_eq!(
            values.physical.keys().copied().collect::<HashSet<_>>(),
            expected_physical.into_iter().collect::<HashSet<_>>()
        );
    }

    #[test]
    fn falsy_zero_converts_to_any_parameter_type_like_cpp() {
        // Pre-strict3 callers reset any falsy parameter to nil before the
        // type check (C4AulExec.cpp:1372 `!pPars[i]` -> Set0), and nil
        // converts to every type (C4Value.cpp FnCnvGuess). TIPI's
        // Initialize calls SetGraphics(0) meaning "default graphics".
        match set_graphics(&[Value::Int(0)]) {
            Ok(_) => {}
            Err(error) => assert!(
                !error.message().contains("expected string"),
                "SetGraphics(0) must mean default graphics, got: {}",
                error.message()
            ),
        }
    }

    #[test]
    fn engine_function_parameters_default_to_nil_zero_like_cpp() {
        // Every C4Aul call carries 10 parameter slots, unfilled = nil
        // (C4Aul.h:104-121); nil converts to int 0, so GBackSolid() with no
        // arguments queries the object's own position (DRAI/WTFL/LAFL
        // action-start scripts call it bare).
        let solid = g_back_solid(&[]).expect("GBackSolid() succeeds");
        assert!(matches!(solid, Value::Bool(_)));
        let liquid = g_back_liquid(&[Value::Nil]).expect("GBackLiquid(nil) succeeds");
        assert!(matches!(liquid, Value::Bool(_)));
    }

    #[test]
    fn get_name_zero_object_slot_selects_definition_like_cpp() {
        // FnGetName's arguments are positional: (object, id). A falsy first
        // slot still consumes the object parameter (C4Script.cpp:992-1005).
        // Dragon Rock calls GetName(0, idSpell) in UpdateTransferZone.
        let world = HostWorldContext::with_landscape(
            Vec::<HostWorldObject>::new(),
            None,
            HashMap::from([(
                "FIRE".into(),
                DefinitionMetadata {
                    name: "Fire".into(),
                    ..DefinitionMetadata::default()
                },
            )]),
            Vec::new(),
            HashMap::new(),
            HashMap::new(),
            1,
            false,
        );
        let (result, _) = with_effect_context(None, &[], world, 1, || {
            get_name(&[Value::Int(0), Value::C4Id("FIRE".into())])
        });

        assert_eq!(
            result.expect("GetName succeeds"),
            Value::String("Fire".into())
        );
    }

    #[test]
    fn get_desc_resolves_caller_object_explicit_object_and_definition() {
        let message_window = r#"#strict
global func MessageWindow(string pMsg, int iForPlr, id idIcon, string pCaption)
{
    if (!idIcon) idIcon = GetID();
    if (!pCaption) pCaption = GetName();
    var pCursor = GetCursor(iForPlr);
    if (!CreateMenu(idIcon, pCursor, pCursor, 0, pCaption, 0, 2)) return();
    AddMenuItem(pCaption, "", TIM1, pCursor, 0, 0, pMsg);
    return 1;
}
"#;
        let script = r#"#strict 2
public func Probe(object other)
{
    return [GetDesc(), GetDesc(0, RULE), GetDesc(0, NOPE),
            GetDesc(other, RULE)];
}
public func Activate(int player) { return MessageWindow(GetDesc(), player); }
"#;
        let mut engine = crate::Engine::with_seed(0);
        assert_eq!(
            engine.install_global_scripts(&[("Helpers.c".into(), message_window.into())]),
            1
        );
        engine
            .register_player(crate::PlayerConfig::new(0, "Player"))
            .expect("player registers");
        let mut goal =
            crate::Definition::from_script("GOAL", "Goal", script).expect("goal fixture compiles");
        goal.set_description(Some("Goal description".into()));
        engine.register_definition(goal).expect("goal registers");
        for (id, name, description) in [
            ("RULE", "Rule", "Rule description"),
            ("OTHR", "Other", "Other description"),
        ] {
            let mut definition = crate::Definition::from_script(id, name, "#strict 2")
                .expect("description fixture compiles");
            definition.set_description(Some(description.into()));
            engine
                .register_definition(definition)
                .expect("description fixture registers");
        }
        let mut crew = crate::Definition::from_script("CLNK", "Clonk", "#strict 2")
            .expect("crew fixture compiles");
        crew.set_crew_member(true);
        engine.register_definition(crew).expect("crew registers");

        let goal = engine
            .spawn_object(SpawnConfig::new("GOAL"))
            .expect("goal spawns");
        let other = engine
            .spawn_object(SpawnConfig::new("OTHR"))
            .expect("other object spawns");
        let crew = engine
            .spawn_object(
                SpawnConfig::new("CLNK")
                    .with_alive(true)
                    .with_owner(0)
                    .with_crew_member(true),
            )
            .expect("crew spawns");
        engine.select_crew(0, [crew]).expect("crew selects");
        engine
            .set_crew_cursor(0, Some(crew))
            .expect("crew cursor sets");
        let goal_index = engine.find_object_index(goal).expect("goal exists");

        assert_eq!(
            engine
                .call_object_function(goal_index, "Probe", vec![object_reference_value(other)],)
                .expect("GetDesc and MessageWindow fixture runs"),
            Value::Array(vec![
                Value::String("Goal description".into()),
                Value::String("Rule description".into()),
                Value::Nil,
                Value::String("Other description".into()),
            ])
        );
        assert_eq!(
            engine
                .call_object_function(goal_index, "Activate", vec![Value::Int(0)])
                .expect("goal MessageWindow opens"),
            Value::Int(1)
        );
        let menu = engine
            .debug_object_menu(crew.as_u64())
            .expect("crew remains")
            .expect("description menu is open");
        assert_eq!(menu.style, 2);
        assert_eq!(menu.caption, "Goal");
        assert_eq!(menu.symbol_id, "GOAL");
        assert_eq!(menu.items.len(), 1);
        assert_eq!(menu.items[0].info_caption, "Goal description");
    }

    #[test]
    fn get_value_runs_cpp_object_definition_and_base_callback_chains() {
        // FnGetValue selects the definition branch when idDef is nonzero;
        // otherwise it defaults the object to cthr->Obj. C4Object::GetValue
        // calls CalcValue (or CalcDefValue), applies Con, then CalcSellValue;
        // C4Def::GetValue calls CalcDefValue then CalcBuyValue
        // (C4Script.cpp:1366-1375; C4Object.cpp:2118-2141;
        // C4Def.cpp:839-858).
        let caller_script = r#"#strict
local iObject, iDefinition, iSelf, vUnknown;
func Trigger(object item, object base)
{
    iObject = GetValue(item, 0, base, 3);
    iDefinition = GetValue(0, VALU, base, 4);
    iSelf = GetValue();
    vUnknown = GetValue(0, NOPE);
    return(1);
}
"#;
        let value_script = r#"#strict
func CalcValue(object base, int player) { return(20 + player); }
func CalcDefValue(object base, int player) { return(40 + player); }
"#;
        let base_script = r#"#strict
func CalcBuyValue(id definition, int value) { return(value + 100); }
func CalcSellValue(object item, int value) { return(value + 200); }
"#;

        let mut engine = crate::Engine::with_seed(0);
        let mut caller = crate::Definition::from_script("CALL", "Caller", caller_script)
            .expect("caller fixture compiles");
        caller.set_value(9);
        engine
            .register_definition(caller)
            .expect("caller fixture registers");
        let mut valued = crate::Definition::from_script("VALU", "Valued", value_script)
            .expect("valued fixture compiles");
        valued.set_value(50);
        engine
            .register_definition(valued)
            .expect("valued fixture registers");
        engine
            .register_definition(
                crate::Definition::from_script("BASE", "Base", base_script)
                    .expect("base fixture compiles"),
            )
            .expect("base fixture registers");

        let caller = engine
            .spawn_object(crate::SpawnConfig::new("CALL"))
            .expect("caller fixture spawns");
        let item = engine
            .spawn_object(crate::SpawnConfig::new("VALU").with_construction(crate::FULL_CON / 2))
            .expect("valued fixture spawns");
        let base = engine
            .spawn_object(crate::SpawnConfig::new("BASE"))
            .expect("base fixture spawns");
        let caller_index = engine.find_object_index(caller).expect("caller index");
        engine
            .call_object_function(
                caller_index,
                "Trigger",
                vec![Value::Object(item.as_u64()), Value::Object(base.as_u64())],
            )
            .expect("GetValue fixture runs");

        let snapshot = engine.object_snapshot(caller).expect("caller remains live");
        assert_eq!(snapshot.local_vars.get("iObject"), Some(&Value::Int(211)));
        assert_eq!(
            snapshot.local_vars.get("iDefinition"),
            Some(&Value::Int(144))
        );
        assert_eq!(snapshot.local_vars.get("iSelf"), Some(&Value::Int(9)));
        assert_eq!(snapshot.local_vars.get("vUnknown"), Some(&Value::Nil));
    }

    #[test]
    fn set_name_self_is_immediately_visible_and_nil_restores_fallback() {
        // FnSetName's ordinary-object branch writes CustomName immediately;
        // C4Object::GetName then resolves custom -> info -> definition
        // (C4Script.cpp:1008-1061; C4Object.cpp:2103-2116).
        let script = r#"#strict
local sInitial, bSet, sRenamed, bClear, sCleared;
func Trigger()
{
    sInitial = GetName();
    bSet = SetName("Renamed");
    sRenamed = GetName();
    bClear = SetName();
    sCleared = GetName();
    return(1);
}
"#;
        let mut engine = crate::Engine::with_seed(0);
        engine
            .register_definition(
                crate::Definition::from_script("CALL", "Caller", script)
                    .expect("name fixture compiles"),
            )
            .expect("name fixture registers");
        let caller = engine
            .spawn_object(
                crate::SpawnConfig::new("CALL")
                    .with_category(crate::CATEGORY_OBJECT)
                    .with_loaded(true)
                    .with_custom_name("Saved Caller"),
            )
            .expect("name fixture spawns");
        let caller_index = engine
            .find_object_index(caller)
            .expect("name fixture exists");

        engine
            .call_object_function(caller_index, "Trigger", Vec::new())
            .expect("name fixture runs");

        let snapshot = engine
            .object_snapshot(caller)
            .expect("name fixture remains");
        assert_eq!(
            snapshot.local_vars.get("sInitial"),
            Some(&Value::String("Saved Caller".into()))
        );
        assert_eq!(snapshot.local_vars.get("bSet"), Some(&Value::Bool(true)));
        assert_eq!(
            snapshot.local_vars.get("sRenamed"),
            Some(&Value::String("Renamed".into()))
        );
        assert_eq!(snapshot.local_vars.get("bClear"), Some(&Value::Bool(true)));
        assert_eq!(
            snapshot.local_vars.get("sCleared"),
            Some(&Value::String("Caller".into()))
        );
        assert_eq!(snapshot.custom_name, None);
    }

    #[test]
    fn get_name_prefers_live_crew_info_and_tracks_same_call_definition_changes() {
        // C4Object::GetName is CustomName -> Info->Name -> Def->Name.
        // MakeCrewMember installs Info synchronously, clearing CustomName
        // must reveal it again, and ChangeDef changes the final fallback in
        // the same call (C4Object.cpp:2103-2116, C4Player.cpp:1167-1210).
        let script = r#"#strict 2
func Probe(object crew, object plain)
{
    var made = MakeCrewMember(crew, 0);
    var info_name = GetName(crew);
    var set = SetName("Alias", crew);
    var alias = GetName(crew);
    var cleared = SetName(0, crew);
    var restored = GetName(crew);
    var plain_name = GetName(plain);
    var changed = ChangeDef(NEWW, plain);
    var changed_name = GetName(plain);
    return [made, info_name, set, alias, cleared, restored,
            plain_name, changed, changed_name];
}
func Read(object crew) { return GetName(crew); }
"#;
        let mut engine = crate::Engine::with_seed(0);
        engine
            .register_player(crate::PlayerConfig::new(0, "Player"))
            .expect("player registers");
        engine.set_standard_names(Some("Twonky\n".to_owned()));
        engine
            .register_definition(
                crate::Definition::from_script("CALL", "Caller", script).expect("driver compiles"),
            )
            .expect("driver registers");
        let mut crew = crate::Definition::from_script("CREW", "Crew", "").expect("crew compiles");
        crew.set_crew_member(true);
        engine.register_definition(crew).expect("crew registers");
        engine
            .register_definition(
                crate::Definition::from_script("PLAI", "Plain", "").expect("plain compiles"),
            )
            .expect("plain registers");
        engine
            .register_definition(
                crate::Definition::from_script("NEWW", "Changed", "").expect("changed compiles"),
            )
            .expect("changed registers");

        let caller = engine
            .spawn_object(crate::SpawnConfig::new("CALL"))
            .expect("driver spawns");
        let crew = engine
            .spawn_object(
                crate::SpawnConfig::new("CREW")
                    .with_owner(0)
                    .with_crew_member(false),
            )
            .expect("crew spawns");
        let plain = engine
            .spawn_object(crate::SpawnConfig::new("PLAI"))
            .expect("plain object spawns");
        let caller_index = engine.find_object_index(caller).expect("driver exists");

        assert_eq!(
            engine
                .call_object_function(
                    caller_index,
                    "Probe",
                    vec![Value::Object(crew.as_u64()), Value::Object(plain.as_u64()),],
                )
                .expect("name probe runs"),
            Value::Array(vec![
                Value::Bool(true),
                Value::String("Twonky".into()),
                Value::Bool(true),
                Value::String("Alias".into()),
                Value::Bool(true),
                Value::String("Twonky".into()),
                Value::String("Plain".into()),
                Value::Bool(true),
                Value::String("Changed".into()),
            ])
        );
        assert_eq!(
            engine
                .call_object_function(caller_index, "Read", vec![Value::Object(crew.as_u64())],)
                .expect("folded crew name reads"),
            Value::String("Twonky".into())
        );
        assert_eq!(
            engine
                .object_snapshot(crew)
                .expect("crew remains")
                .custom_name,
            None
        );
        assert_eq!(
            engine
                .object_snapshot(plain)
                .expect("plain remains")
                .definition_id,
            "NEWW"
        );
    }

    #[test]
    fn set_name_foreign_and_arrow_targets_are_live_and_persisted() {
        // FnSetName accepts an explicit object, while AB_CALL host fallback
        // makes an arrow target the default cthr->Obj (C4Script.cpp:1008-1061;
        // C4AulExec.cpp:1216-1305).
        let script = r#"#strict
local bForeign, sForeign, bArrow, sArrow;
func Trigger(object pOther)
{
    bForeign = SetName("Foreign", pOther);
    sForeign = GetName(pOther);
    bArrow = pOther->SetName("Arrow");
    sArrow = pOther->GetName();
    return(1);
}
"#;
        let mut engine = crate::Engine::with_seed(0);
        engine
            .register_definition(
                crate::Definition::from_script("CALL", "Caller", script)
                    .expect("caller fixture compiles"),
            )
            .expect("caller fixture registers");
        engine
            .register_definition(
                crate::Definition::from_script("OTHR", "Other", "#strict\n")
                    .expect("target fixture compiles"),
            )
            .expect("target fixture registers");
        let caller = engine
            .spawn_object(crate::SpawnConfig::new("CALL").with_category(crate::CATEGORY_OBJECT))
            .expect("caller fixture spawns");
        let other = engine
            .spawn_object(crate::SpawnConfig::new("OTHR").with_category(crate::CATEGORY_OBJECT))
            .expect("target fixture spawns");
        let caller_index = engine
            .find_object_index(caller)
            .expect("caller fixture exists");

        engine
            .call_object_function(caller_index, "Trigger", vec![Value::Object(other.as_u64())])
            .expect("foreign name fixture runs");

        let caller_snapshot = engine
            .object_snapshot(caller)
            .expect("caller fixture remains");
        assert_eq!(
            caller_snapshot.local_vars.get("bForeign"),
            Some(&Value::Bool(true))
        );
        assert_eq!(
            caller_snapshot.local_vars.get("sForeign"),
            Some(&Value::String("Foreign".into()))
        );
        assert_eq!(
            caller_snapshot.local_vars.get("bArrow"),
            Some(&Value::Bool(true))
        );
        assert_eq!(
            caller_snapshot.local_vars.get("sArrow"),
            Some(&Value::String("Arrow".into()))
        );
        assert_eq!(
            engine
                .object_snapshot(other)
                .expect("target fixture remains")
                .custom_name
                .as_deref(),
            Some("Arrow")
        );
    }

    #[test]
    fn set_name_info_form_updates_live_and_persistent_names_with_cpp_duplicates() {
        // FnSetName(..., true) writes the linked C4ObjectInfo, rejects empty
        // and overlong names, checks the CURRENT owner's whole info list
        // case-insensitively, and clears CustomName after success
        // (C4Script.cpp:1024-1056; C4ObjectInfoList.cpp:93-110).
        let script = r#"#strict 2
func SetAlias(object target) { return SetName("Alias", target); }
func RenameInfo(object target, string name, bool make_valid)
{
    return [SetName(name, target, 0, true, make_valid),
            GetName(target),
            GetObjectInfoCoreVal("Name", "ObjectInfo", target)];
}
"#;
        let mut engine = crate::Engine::with_seed(0);
        let mut definition = crate::Definition::from_script("CREW", "Crew", script)
            .expect("crew-name fixture compiles");
        definition.set_crew_member(true);
        engine
            .register_definition(definition)
            .expect("crew-name fixture registers");

        let mut start = crate::scenario::PlayerStart::default();
        start.ready_crew = vec![("CREW".to_string(), 1)];
        engine.set_player_starts(vec![start]);
        let info = |name: &str, experience: i32| crate::player_file::CrewInfo {
            id: "CREW".to_string(),
            name: name.to_string(),
            core: Default::default(),
            rank_name: "Clonk".to_string(),
            experience,
            physical: crate::PhysicalInfo::default(),
            ..Default::default()
        };
        engine
            .join_player(crate::JoinPlayerConfig {
                name: "Name owner".to_string(),
                player_info_id: 1,
                score: 0,
                rounds: 0,
                rounds_won: 0,
                rounds_lost: 0,
                total_playing_time: 0,
                team: None,
                color_dw: 0xff0000,
                pref_color: 0,
                pref_position: 0,
                crew: vec![info("Target", 30), info("Ada", 20), info("Ada2", 10)],
                control_style: false,
                auto_context_menu: false,
                startup_player_count: 1,
            })
            .expect("name owner joins");

        let target = engine.player(0).expect("player exists").crew()[0];
        let target_index = engine
            .find_object_index(target)
            .expect("ready crew object exists");
        assert_eq!(
            engine
                .call_object_function(
                    target_index,
                    "SetAlias",
                    vec![Value::Object(target.as_u64())],
                )
                .expect("ordinary alias succeeds"),
            Value::Bool(true)
        );
        assert_eq!(
            engine
                .object_snapshot(target)
                .expect("target remains")
                .custom_name
                .as_deref(),
            Some("Alias")
        );

        let rename = |engine: &mut crate::Engine, name: &str, make_valid: bool| {
            engine
                .call_object_function(
                    target_index,
                    "RenameInfo",
                    vec![
                        Value::Object(target.as_u64()),
                        Value::String(name.to_string().into()),
                        Value::Bool(make_valid),
                    ],
                )
                .expect("info rename probe runs")
        };
        assert_eq!(
            rename(&mut engine, "Nova", false),
            Value::Array(vec![
                Value::Bool(true),
                Value::String("Nova".into()),
                Value::String("Nova".into()),
            ])
        );
        assert_eq!(
            engine
                .object_snapshot(target)
                .expect("target remains")
                .custom_name,
            None
        );

        for invalid in ["", "1234567890123456789012345678901"] {
            assert_eq!(
                rename(&mut engine, invalid, true),
                Value::Array(vec![
                    Value::Bool(false),
                    Value::String("Nova".into()),
                    Value::String("Nova".into()),
                ])
            );
        }
        assert_eq!(
            rename(&mut engine, "ada", false),
            Value::Array(vec![
                Value::Bool(false),
                Value::String("Nova".into()),
                Value::String("Nova".into()),
            ])
        );
        assert_eq!(
            rename(&mut engine, "ada", true),
            Value::Array(vec![
                Value::Bool(true),
                Value::String("ada3".into()),
                Value::String("ada3".into()),
            ])
        );

        assert_eq!(
            engine
                .crew_object_info(target)
                .expect("target retains live info")
                .name,
            "ada3"
        );

        let eight_high_bytes = clonk_script::c4_string_from_bytes(&[0xff; 8]);
        assert_eq!(
            rename(&mut engine, &eight_high_bytes, false),
            Value::Array(vec![
                Value::Bool(true),
                Value::String(eight_high_bytes.clone().into()),
                Value::String(eight_high_bytes.clone().into()),
            ]),
            "eight native bytes are not miscounted as 32 UTF-8 storage bytes"
        );
        let thirty_one_high_bytes = clonk_script::c4_string_from_bytes(&[0xff; 31]);
        assert_eq!(
            rename(&mut engine, &thirty_one_high_bytes, false),
            Value::Array(vec![
                Value::Bool(false),
                Value::String(eight_high_bytes.clone().into()),
                Value::String(eight_high_bytes.clone().into()),
            ])
        );
        assert_eq!(
            rename(&mut engine, "ada3", false),
            Value::Array(vec![
                Value::Bool(true),
                Value::String("ada3".into()),
                Value::String("ada3".into()),
            ])
        );
        let state = engine.capture_state();
        let link = state.crew_info_links[&target];
        assert_eq!(
            state.crew_info_rosters[&link.player_id][link.roster_index].name,
            "ada3"
        );
    }

    #[test]
    fn set_name_definition_form_is_live_persistent_and_exclusive_with_info_form() {
        // The id branch mutates C4Def::Name immediately. Combining it with
        // fSetInInfo is rejected before either target is changed
        // (C4Script.cpp:1012-1022).
        let script = r#"#strict 2
func RenameDefinition(string name, bool set_in_info, object target)
{
    return [SetName(name, 0, CREW, set_in_info),
            GetName(0, CREW), GetName(target),
            GetDefCoreVal("Name", "DefCore", CREW)];
}
"#;
        let mut engine = crate::Engine::with_seed(0);
        engine
            .register_definition(
                crate::Definition::from_script("CREW", "Crew", script)
                    .expect("definition-name fixture compiles"),
            )
            .expect("definition-name fixture registers");
        let target = engine
            .spawn_object(crate::SpawnConfig::new("CREW"))
            .expect("info-less target spawns");
        let target_index = engine.find_object_index(target).expect("target exists");

        assert_eq!(
            engine
                .call_object_function(
                    target_index,
                    "RenameDefinition",
                    vec![
                        Value::String("Renamed".into()),
                        Value::Bool(false),
                        Value::Object(target.as_u64()),
                    ],
                )
                .expect("definition rename runs"),
            Value::Array(vec![
                Value::Bool(true),
                Value::String("Renamed".into()),
                Value::String("Renamed".into()),
                Value::String("Renamed".into()),
            ])
        );
        assert_eq!(engine.definition_name("CREW"), Some("Renamed"));

        assert_eq!(
            engine
                .call_object_function(
                    target_index,
                    "RenameDefinition",
                    vec![
                        Value::String("Blocked".into()),
                        Value::Bool(true),
                        Value::Object(target.as_u64()),
                    ],
                )
                .expect("exclusive-form probe runs"),
            Value::Array(vec![
                Value::Bool(false),
                Value::String("Renamed".into()),
                Value::String("Renamed".into()),
                Value::String("Renamed".into()),
            ])
        );
        assert_eq!(engine.definition_name("CREW"), Some("Renamed"));
    }

    #[test]
    fn set_visibility_is_live_for_self_foreign_and_arrow_calls() {
        // FnSet/GetVisibility directly mutate/read C4Object::Visibility;
        // explicit targets and AB_CALL arrow defaults therefore see writes
        // made earlier in the same callback (C4Script.cpp:3860-3877).
        let script = r#"#strict
local iInitial, bSelf, iSelf, bForeign, iForeign, iArrow;
func Trigger(object pOther)
{
    iInitial = GetVisibility();
    bSelf = SetVisibility(VIS_Owner | VIS_Enemies);
    iSelf = GetVisibility();
    bForeign = SetVisibility(VIS_None, pOther);
    iForeign = GetVisibility(pOther);
    iArrow = pOther->GetVisibility();
    return(1);
}
"#;
        let mut engine = crate::Engine::with_seed(0);
        engine
            .register_definition(
                crate::Definition::from_script("CALL", "Caller", script)
                    .expect("caller fixture compiles"),
            )
            .expect("caller fixture registers");
        engine
            .register_definition(
                crate::Definition::from_script("OTHR", "Other", "#strict\n")
                    .expect("target fixture compiles"),
            )
            .expect("target fixture registers");
        let caller = engine
            .spawn_object(crate::SpawnConfig::new("CALL").with_category(crate::CATEGORY_OBJECT))
            .expect("caller fixture spawns");
        let other = engine
            .spawn_object(crate::SpawnConfig::new("OTHR").with_category(crate::CATEGORY_OBJECT))
            .expect("target fixture spawns");

        engine
            .call_object_function(
                engine.find_object_index(caller).expect("caller exists"),
                "Trigger",
                vec![Value::Object(other.as_u64())],
            )
            .expect("visibility fixture runs");

        let caller_snapshot = engine.object_snapshot(caller).expect("caller remains");
        assert_eq!(
            caller_snapshot.local_vars.get("iInitial"),
            Some(&Value::Int(0))
        );
        assert_eq!(
            caller_snapshot.local_vars.get("bSelf"),
            Some(&Value::Bool(true))
        );
        assert_eq!(
            caller_snapshot.local_vars.get("iSelf"),
            Some(&Value::Int(10))
        );
        assert_eq!(
            caller_snapshot.local_vars.get("bForeign"),
            Some(&Value::Bool(true))
        );
        assert_eq!(
            caller_snapshot.local_vars.get("iForeign"),
            Some(&Value::Int(1))
        );
        assert_eq!(
            caller_snapshot.local_vars.get("iArrow"),
            Some(&Value::Int(1))
        );
        assert_eq!(
            caller_snapshot.visibility,
            crate::VIS_OWNER | crate::VIS_ENEMIES
        );
        assert_eq!(
            engine
                .object_snapshot(other)
                .expect("target remains")
                .visibility,
            crate::VIS_NONE
        );
    }

    #[test]
    fn modulate_color_matches_cpp_packed_channel_math() {
        // FnModulateColor uses packed AARRGGBB, divides each RGB product by
        // 256, and combines inverted alpha with a + b - a*b/256.
        assert_eq!(
            modulate_color(&[Value::Int(0x0a40_2010), Value::Int(0x1420_1008)])
                .expect("colors modulate"),
            Value::Int(0x1e08_0200)
        );
        assert_eq!(
            modulate_color(&[Value::Int(-1), Value::Int(-1)]).expect("signed colors modulate"),
            Value::Int(-65_794)
        );
        assert_eq!(
            modulate_color(&[Value::Nil, Value::Int(0x0010_2030)])
                .expect("nil uses the C++ default first color"),
            Value::Int(0x000f_1f2f)
        );
        let mut engine = ScriptEngine::new();
        register_host_functions(&mut engine);
        assert_eq!(
            engine
                .call(
                    "ModulateColor",
                    &[Value::Int(0), Value::Int(0x0010_2030), Value::Int(99)],
                )
                .expect("the native boundary eagerly nils a callerless zero"),
            Value::Int(0x000f_1f2f)
        );
    }

    #[test]
    fn optional_native_int_keeps_non_nil_raw_bool_zero_present() {
        let high_word_bool = Value::from_c4_bool_data_raw(1usize.checked_shl(32).unwrap_or(2));
        assert_eq!(
            parse_native_optional_i32(Some(&high_word_bool), "Probe", "optional integer",)
                .expect("raw Bool payload extracts"),
            Some(if usize::BITS > 32 { 0 } else { 2 })
        );

        let mut engine = ScriptEngine::new();
        register_host_functions(&mut engine);
        assert_eq!(
            engine
                .call("ModulateColor", &[high_word_bool, Value::Int(-1)])
                .expect("a non-nil optional zero remains present"),
            Value::Int(if usize::BITS > 32 {
                -16_777_216
            } else {
                -16_777_215
            })
        );
    }

    #[test]
    fn create_object_accepts_id_values_like_cpp() {
        // FnCreateObject's first parameter is a C4ID (C4Script.cpp:1892);
        // content passes id literals (BAS7/_ROA/NTIP Construction).
        let error = create_object(&[Value::C4Id("ROCK".into())])
            .expect_err("no engine context in unit test");
        assert!(
            !error.message().contains("expected string"),
            "id must be a valid definition argument, got: {}",
            error.message()
        );
    }

    #[test]
    fn get_keys_rejects_nil() {
        let error = get_keys(&[Value::Nil]).expect_err("GetKeys should fail for nil");
        assert_eq!(error.message(), "GetKeys(): map expected, got 0");
    }

    #[test]
    fn get_values_returns_c4valuehash_key_order() {
        let mut map = ValueMap::new();
        map.insert("beta".into(), Value::Int(2));
        map.insert("alpha".into(), Value::Int(1));
        map.insert_key(Value::Int(7), Value::String("seven".into()));
        let result = get_values(&[Value::Proplist(map)]).expect("GetValues succeeds");
        match result {
            Value::Array(entries) => {
                assert_eq!(
                    entries,
                    vec![Value::Int(2), Value::Int(1), Value::String("seven".into()),]
                );
            }
            other => panic!("expected array, got {:?}", other),
        }
    }

    #[test]
    fn get_values_rejects_nil() {
        let error = get_values(&[Value::Nil]).expect_err("GetValues should fail for nil");
        assert_eq!(error.message(), "GetValues(): map expected, got 0");
    }

    fn death_announce_world(death_message: Option<&str>, film: i32) -> HostWorldContext {
        let target = ObjectId::new(1);
        let definitions = Rc::new(HashMap::from([(
            DefinitionId::from("TEST"),
            DefinitionMetadata {
                name: "Fallback Clonk".to_string(),
                ..DefinitionMetadata::default()
            },
        )]));
        let mut world = HostWorldContext::from_objects([scenario_section_world_object(
            target.as_u64(),
            ObjectStatus::Normal,
        )])
        .with_definition_metadata(definitions)
        .with_scenario_values(Rc::new(ScenarioValueStore::with_film_for_test(film)));
        if let Some(death_message) = death_message {
            world = world.with_crew_infos(Rc::new(HashMap::from([(
                target,
                CrewObjectInfo {
                    definition_id: DefinitionId::from("TEST"),
                    name: "Roster Clonk".to_string(),
                    death_message: death_message.to_string(),
                    core: Default::default(),
                    rank: 0,
                    rank_name: "Clonk".to_string(),
                    experience: 0,
                    participation: 1,
                    rounds: 0,
                    death_count: 0,
                    total_playing_time: 0,
                    birthday: 0,
                    age: 0,
                    in_action_time: 0,
                    extra_data: Vec::new(),
                    portraits: CrewPortraitState::default(),
                },
            )])));
        }
        world
    }

    #[test]
    fn death_announce_uses_live_crew_message_verbatim_without_safe_random() {
        let script = r#"#strict 2
func Announce()
{
    return [GetObjectInfoCoreVal("DeathMessage"), DeathAnnounce()];
}
"#;
        let mut engine = crate::Engine::with_seed(0);
        let mut definition = crate::Definition::from_script("CREW", "Crew", script)
            .expect("death-message fixture compiles");
        definition.set_crew_member(true);
        engine
            .register_definition(definition)
            .expect("death-message fixture registers");
        let mut start = crate::scenario::PlayerStart::default();
        start.ready_crew = vec![("CREW".to_string(), 1)];
        engine.set_player_starts(vec![start]);
        engine
            .join_player(crate::JoinPlayerConfig {
                name: "Death message owner".to_string(),
                player_info_id: 1,
                score: 0,
                rounds: 0,
                rounds_won: 0,
                rounds_lost: 0,
                total_playing_time: 0,
                team: None,
                color_dw: 0xff0000,
                pref_color: 0,
                pref_position: 0,
                crew: vec![crate::player_file::CrewInfo {
                    id: "CREW".to_string(),
                    name: "Ada".to_string(),
                    death_message: "Remember me // exactly  ".to_string(),
                    physical: crate::PhysicalInfo::default(),
                    ..Default::default()
                }],
                control_style: false,
                auto_context_menu: false,
                startup_player_count: 1,
            })
            .expect("death-message owner joins");
        let crew = engine.player(0).expect("player exists").crew()[0];
        let crew_index = engine.find_object_index(crew).expect("crew remains live");

        let initial_rng = crate::particles::SafeRng::new(224);
        SCRIPT_SAFE_RNG.with(|rng| *rng.borrow_mut() = initial_rng.clone());
        assert_eq!(
            engine
                .call_object_function(crew_index, "Announce", Vec::new())
                .expect("DeathAnnounce succeeds"),
            Value::Array(vec![
                Value::String("Remember me // exactly  ".into()),
                Value::Bool(true),
            ])
        );
        let messages = engine
            .snapshot()
            .hud
            .messages
            .into_iter()
            .filter(|message| message.target == Some(crew))
            .collect::<Vec<_>>();
        assert_eq!(messages.len(), 1);
        assert_eq!(messages[0].lines.join("|"), "Remember me // exactly  ");
        SCRIPT_SAFE_RNG.with(|rng| assert_eq!(*rng.borrow(), initial_rng));
    }

    #[test]
    fn death_announce_film_suppresses_messages_and_objectless_calls_still_fail() {
        let initial_rng = crate::particles::SafeRng::new(225);
        SCRIPT_SAFE_RNG.with(|rng| *rng.borrow_mut() = initial_rng.clone());
        let (result, outcome) = with_object_host_context_with_world(
            death_announce_world(Some("Must stay hidden"), 2),
            || death_announce(&[]),
        );
        assert_eq!(
            result.expect("Film DeathAnnounce succeeds"),
            Value::Bool(true)
        );
        assert!(outcome.messages.is_empty());

        let (objectless, outcome) = with_effect_context(
            None,
            &[],
            death_announce_world(Some("Still hidden"), 2),
            2,
            || death_announce(&[]),
        );
        assert_eq!(
            objectless.expect("objectless DeathAnnounce is a clean failure"),
            Value::Bool(false)
        );
        assert!(outcome.messages.is_empty());
        SCRIPT_SAFE_RNG.with(|rng| assert_eq!(*rng.borrow(), initial_rng));
    }

    #[test]
    fn death_announce_info_less_fallback_keeps_all_seven_safe_random_choices() {
        const MESSAGES: [&str; 7] = [
            "Fallback Clonk is dead.",
            "Fallback Clonk has|deceased.",
            "Fallback Clonk|rests in peace.",
            "Fallback Clonk is dead.",
            "Fallback Clonk has|deceased.",
            "Fallback Clonk|rests in peace.",
            "Fallback Clonk is dead.",
        ];
        let mut seen = HashSet::new();
        for seed in 1..=512 {
            let initial_rng = crate::particles::SafeRng::new(seed);
            let mut expected_rng = initial_rng.clone();
            let choice = expected_rng.random(7) as usize;
            SCRIPT_SAFE_RNG.with(|rng| *rng.borrow_mut() = initial_rng);

            let (result, outcome) =
                with_object_host_context_with_world(death_announce_world(None, 0), || {
                    death_announce(&[])
                });
            assert_eq!(result.expect("fallback succeeds"), Value::Bool(true));
            assert!(matches!(
                outcome.messages.as_slice(),
                [MessageCommand::Add(MessageSpec {
                    kind: MessageKind::Target,
                    text,
                    target: Some(target),
                    player: None,
                    ..
                })] if text == MESSAGES[choice] && *target == ObjectId::new(1)
            ));
            SCRIPT_SAFE_RNG.with(|rng| assert_eq!(*rng.borrow(), expected_rng));
            seen.insert(choice);
            if seen.len() == 7 {
                break;
            }
        }
        assert_eq!(seen.len(), 7, "SafeRandom(7) retains every index");
    }

    #[test]
    fn message_formats_and_registers_global_message() {
        let args = [
            Value::String("Score %03d".into()),
            Value::Nil,
            Value::Int(7),
        ];
        let (result, outcome) = with_object_host_context(|| message(&args));
        assert_eq!(result.expect("Message succeeds"), Value::Bool(true));
        assert_eq!(outcome.messages.len(), 1);
        match &outcome.messages[0] {
            MessageCommand::Add(spec) => {
                assert_eq!(spec.kind, MessageKind::Global);
                assert_eq!(spec.text, "Score 007");
                assert!(spec.target.is_none());
                assert!(spec.player.is_none());
            }
            MessageCommand::Append { .. } => panic!("plain Message cannot append"),
            MessageCommand::PendingSpeech(_) => panic!("plain Message cannot defer speech"),
        }
    }

    #[test]
    fn empty_plain_message_hosts_forward_cpp_clear_operations() {
        // FnMessage/FnPlayerMessage/FnPlrMessage pass an explicit empty
        // C4String through GameMsg* to C4GameMessageList::New. New first
        // removes conflicting non-multiple messages, then succeeds without
        // adding a replacement (C4Script.cpp:2395-2462;
        // C4GameMessage.cpp:290-305).
        let (result, outcome) =
            with_object_host_context(|| message(&[Value::String(String::new().into())]));
        assert_eq!(result.expect("empty Message succeeds"), Value::Bool(true));
        assert!(matches!(
            outcome.messages.as_slice(),
            [MessageCommand::Add(MessageSpec {
                kind: MessageKind::Global,
                text,
                ..
            })] if text.is_empty()
        ));

        let mut player = PlayerState::default();
        player.id = 1;
        let world = HostWorldContext::from_objects_with_players(
            Vec::<HostWorldObject>::new(),
            vec![player.clone()],
        );
        let (result, outcome) = with_object_host_context_with_world(world, || {
            player_message(&[Value::Int(1), Value::String(String::new().into())])
        });
        assert_eq!(
            result.expect("empty PlayerMessage succeeds"),
            Value::Bool(true)
        );
        assert!(matches!(
            outcome.messages.as_slice(),
            [MessageCommand::Add(MessageSpec {
                kind: MessageKind::GlobalPlayer,
                text,
                player: Some(1),
                ..
            })] if text.is_empty()
        ));

        let world = HostWorldContext::from_objects_with_players(
            Vec::<HostWorldObject>::new(),
            vec![player],
        );
        let (result, outcome) = with_object_host_context_with_world(world, || {
            plr_message(&[Value::String(String::new().into()), Value::Int(1)])
        });
        assert_eq!(
            result.expect("empty PlrMessage succeeds"),
            Value::Bool(true)
        );
        assert!(matches!(
            outcome.messages.as_slice(),
            [MessageCommand::Add(MessageSpec {
                kind: MessageKind::GlobalPlayer,
                text,
                player: Some(1),
                ..
            })] if text.is_empty()
        ));
    }

    #[test]
    fn custom_message_unfilled_owner_defaults_to_player_zero() {
        // FnCustomMessage receives iOwner as C4ValueInt: omitted and nil
        // arguments convert to zero, while only an explicit NO_OWNER (-1)
        // selects the all-player message variants (C4Script.cpp:5995-6033).
        let target = ObjectId::new(1);
        let cases = [
            (
                vec![
                    Value::String("target omitted".into()),
                    object_reference_value(target),
                ],
                MessageKind::TargetPlayer,
                Some(target),
                Some(0),
            ),
            (
                vec![
                    Value::String("target nil".into()),
                    object_reference_value(target),
                    Value::Nil,
                ],
                MessageKind::TargetPlayer,
                Some(target),
                Some(0),
            ),
            (
                vec![
                    Value::String("target everyone".into()),
                    object_reference_value(target),
                    Value::Int(OWNER_NONE),
                ],
                MessageKind::Target,
                Some(target),
                None,
            ),
            (
                vec![Value::String("global omitted".into())],
                MessageKind::GlobalPlayer,
                None,
                Some(0),
            ),
            (
                vec![Value::String("global nil".into()), Value::Nil, Value::Nil],
                MessageKind::GlobalPlayer,
                None,
                Some(0),
            ),
            (
                vec![
                    Value::String("global everyone".into()),
                    Value::Nil,
                    Value::Int(OWNER_NONE),
                ],
                MessageKind::Global,
                None,
                None,
            ),
        ];

        for (args, expected_kind, expected_target, expected_player) in cases {
            let (result, outcome) = with_object_host_context(|| custom_message(&args));
            assert_eq!(result.expect("CustomMessage succeeds"), Value::Bool(true));
            assert_eq!(outcome.messages.len(), 1);
            match &outcome.messages[0] {
                MessageCommand::Add(spec) => {
                    assert_eq!(spec.kind, expected_kind);
                    assert_eq!(spec.target, expected_target);
                    assert_eq!(spec.player, expected_player);
                }
                MessageCommand::Append { .. } => {
                    panic!("CustomMessage cannot append")
                }
                MessageCommand::PendingSpeech(_) => {
                    panic!("CustomMessage cannot defer speech")
                }
            }
        }
    }

    #[test]
    #[allow(clippy::arc_with_non_send_sync)] // Definition scripts are single-threaded host fixtures.
    fn custom_message_c4id_decoration_matches_tutorial_call() {
        // FnCustomMessage's decoration parameter is C4ID and succeeds only
        // when C4Id2Def resolves it (C4Script.cpp:5995-6002). Tutorial.c
        // passes the literal DECO, which reaches the Rust VM as Value::C4Id.
        // C4GameMessage::Init immediately snapshots FrameDecoration::SetByDef;
        // use Tutorial01's actual DECO callback and all eight ActMap facets.
        let tutorial_facets = [
            (
                "TopLeft",
                crate::DefinitionActionFacet {
                    x: 0,
                    y: 0,
                    width: 16,
                    height: 16,
                    target_x: -8,
                    target_y: -7,
                },
            ),
            (
                "Top",
                crate::DefinitionActionFacet {
                    x: 16,
                    y: 0,
                    width: 58,
                    height: 12,
                    target_x: 0,
                    target_y: -7,
                },
            ),
            (
                "TopRight",
                crate::DefinitionActionFacet {
                    x: 74,
                    y: 0,
                    width: 16,
                    height: 16,
                    target_x: -7,
                    target_y: -7,
                },
            ),
            (
                "Right",
                crate::DefinitionActionFacet {
                    x: 74,
                    y: 16,
                    width: 16,
                    height: 58,
                    target_x: -7,
                    target_y: 0,
                },
            ),
            (
                "BottomRight",
                crate::DefinitionActionFacet {
                    x: 74,
                    y: 74,
                    width: 16,
                    height: 16,
                    target_x: -7,
                    target_y: -8,
                },
            ),
            (
                "Bottom",
                crate::DefinitionActionFacet {
                    x: 16,
                    y: 76,
                    width: 58,
                    height: 16,
                    target_x: 0,
                    target_y: -6,
                },
            ),
            (
                "BottomLeft",
                crate::DefinitionActionFacet {
                    x: 0,
                    y: 74,
                    width: 16,
                    height: 16,
                    target_x: -8,
                    target_y: -8,
                },
            ),
            (
                "Left",
                crate::DefinitionActionFacet {
                    x: 0,
                    y: 16,
                    width: 16,
                    height: 58,
                    target_x: -8,
                    target_y: 0,
                },
            ),
        ];
        let action_graphics = tutorial_facets
            .iter()
            .map(|(suffix, facet)| {
                (
                    format!("FrameDeco{suffix}"),
                    crate::DefinitionActionGraphics {
                        facet: Some(facet.clone()),
                        ..crate::DefinitionActionGraphics::default()
                    },
                )
            })
            .collect();
        let mut script = ScriptEngine::new();
        script
            .load_script("protected func FrameDecorationBackClr() { return(-2144193998); }")
            .expect("DECO callback compiles");
        let world = HostWorldContext::default()
            .with_definition_metadata(Rc::new(HashMap::from([(
                "DECO".into(),
                DefinitionMetadata {
                    action_graphics,
                    ..DefinitionMetadata::default()
                },
            )])))
            .with_definition_scripts(HashMap::from([("DECO".into(), Arc::new(script))]));
        let args = [
            Value::String("@Welcome to the world of Clonk.$01_01_en.ogg".into()),
            Value::Nil,
            Value::Int(0),
            Value::Int(50),
            Value::Int(50),
            Value::Int(0x00ff_ffff),
            Value::C4Id("DECO".into()),
            Value::String("Portrait:SCLK::0000ff::1".into()),
            Value::Int(
                (crate::message::FLAG_TOP
                    | FLAG_LEFT
                    | FLAG_WIDTH_REL
                    | FLAG_X_REL
                    | FLAG_DROP_SPEECH) as i32,
            ),
            Value::Int(30),
        ];
        let (result, outcome) =
            with_object_host_context_with_world(world, || custom_message(&args));

        assert_eq!(result.expect("CustomMessage succeeds"), Value::Bool(true));
        assert_eq!(outcome.messages.len(), 1);
        match &outcome.messages[0] {
            MessageCommand::Add(spec) => {
                assert_eq!(spec.decoration.as_deref(), Some("DECO"));
                assert_eq!(spec.portrait.as_deref(), Some("Portrait:SCLK::0000ff::1"));
                let frame = spec
                    .frame_decoration
                    .as_ref()
                    .expect("C4GameMessage snapshots DECO at creation");
                assert_eq!(frame.source_definition, "DECO");
                assert_eq!(frame.background_color, 0x8032_3232);
                assert_eq!(
                    (
                        frame.border_top,
                        frame.border_left,
                        frame.border_right,
                        frame.border_bottom,
                    ),
                    (0, 0, 0, 0)
                );
                assert_eq!(
                    [
                        frame.top_left.as_ref(),
                        frame.top.as_ref(),
                        frame.top_right.as_ref(),
                        frame.right.as_ref(),
                        frame.bottom_right.as_ref(),
                        frame.bottom.as_ref(),
                        frame.bottom_left.as_ref(),
                        frame.left.as_ref(),
                    ],
                    tutorial_facets.each_ref().map(|(_, facet)| Some(facet))
                );
            }
            MessageCommand::Append { .. } => panic!("CustomMessage cannot append"),
            MessageCommand::PendingSpeech(_) => panic!("CustomMessage cannot defer speech"),
        }
    }
