// Contiguous slice 9 of 11 of the `compat::tests` battery, spliced by
// `include!` from compat.rs so every test id stays `compat::tests::*`.
// Mostly: object state, effects, players.

    #[test]
    fn get_effect_negative_index_returns_nil() {
        // C4Effect::Get compares with `if (iIndex--) continue`, so a negative
        // named-effect index simply exhausts the list and returns nullptr
        // (C4Effect.cpp:215-236). Hazard's Weapon bonus loop deliberately
        // probes `i - 1` while i is still nil/zero (Weapon.c4d/Script.c:551-554).
        let state = empty_state();
        let (result, _) = with_object_host_context(|| -> Result<Value, RuntimeError> {
            add_effect(&[
                Value::String("Bonus".into()),
                state.clone(),
                Value::Int(100),
            ])?;
            get_effect(&[Value::String("Bonus".into()), state, Value::Int(-1)])
        });

        assert_eq!(result.expect("negative index is not an error"), Value::Nil);
    }

    #[test]
    fn get_effect_returns_command_target_metadata() {
        let state = empty_state();
        let mut target_map = ValueMap::new();
        target_map.insert("id".into(), Value::Int(7));
        let target = Value::Proplist(target_map);

        let (result, _) = with_object_host_context(|| -> Result<Value, RuntimeError> {
            add_effect(&[
                Value::String("Glow".into()),
                state.clone(),
                Value::Int(100),
                Value::Int(1),
                target.clone(),
                Value::C4Id("BARL".into()),
            ])?;
            get_effect(&[
                Value::String("Glow".into()),
                state.clone(),
                Value::Int(0),
                Value::Int(4),
            ])
        });
        let value = result.test_value();
        assert_eq!(value, Value::Object(7));

        let (result, _) = with_object_host_context(|| -> Result<Value, RuntimeError> {
            add_effect(&[
                Value::String("Glow".into()),
                state.clone(),
                Value::Int(100),
                Value::Int(1),
                target.clone(),
                Value::C4Id("BARL".into()),
            ])?;
            get_effect(&[
                Value::String("Glow".into()),
                state.clone(),
                Value::Int(0),
                Value::Int(5),
            ])
        });
        let value = result.test_value();
        assert_eq!(value, Value::C4Id("BARL".into()));
    }

    #[test]
    fn get_effect_command_metadata_keeps_object_and_c4id_types() {
        let command_script = r#"#strict 2
public func Ping() { return 42; }
public func Probe(object carrier)
{
    AddEffect("Typed", carrier, 100, 0, this(), BARL);
    var target = GetEffect("Typed", carrier, 0, 4);
    var same = target == this(), ping = target->Ping();
    var initial_id = GetEffect("Typed", carrier, 0, 5) == CMND;
    ChangeDef(NEWD);
    return [same, ping, initial_id,
            GetEffect("Typed", carrier, 0, 5) == NEWD,
            GetEffect("Typed", carrier, 0, 7)];
}
"#;
        let mut engine = crate::Engine::with_seed(0);
        engine.register_test_definition(test_definition("CMND", "Command target", command_script));
        engine.register_test_definition(test_definition("HOLD", "Effect holder", "#strict 2"));
        engine.register_test_definition(test_definition("NEWD", "Changed command target", "#strict 2"));
        let command = engine.spawn_test_object(crate::SpawnConfig::new("CMND"));
        let holder = engine.spawn_test_object(crate::SpawnConfig::new("HOLD"));
        let command_index = engine
            .find_object_index(command).test_value();

        assert_eq!(
            engine
                .call_object_function(command_index, "Probe", vec![object_reference_value(holder)],)
                .expect("typed GetEffect probe runs"),
            Value::Array(vec![
                Value::Bool(true),
                Value::Int(42),
                Value::Bool(true),
                Value::Bool(true),
                Value::Nil,
            ])
        );
    }

    #[test]
    fn add_effect_accepts_c4id_command_target_id_like_cpp() {
        // FnAddEffect's idCmdTarget parameter IS a C4ID
        // (C4Script.cpp:5450) — real content passes id literals
        // (`AddEffect(..., 0, SWRD)`), never strings.
        let state = empty_state();
        let (result, _) = with_object_host_context(|| -> Result<Value, RuntimeError> {
            add_effect(&[
                Value::String("Glow".into()),
                state.clone(),
                Value::Int(100),
                Value::Int(1),
                Value::Nil,
                Value::C4Id("BARL".into()),
            ])?;
            get_effect(&[
                Value::String("Glow".into()),
                state.clone(),
                Value::Int(0),
                Value::Int(5),
            ])
        });
        let value = result.test_value();
        assert_eq!(value, Value::C4Id("BARL".into()));
    }

    #[test]
    fn get_effect_query_zero_returns_effect_number_like_cpp() {
        // FnGetEffect query 0 returns pEffect->iNumber (C4Script.cpp:5481)
        // — the per-object monotonic handle (C4Effect.cpp:76-78), NOT a
        // list position. Removing an earlier effect must not renumber the
        // survivor: Control2Effect feeds this handle straight into
        // EffectCall (Clonk.c4d/Script.c:860-875).
        let state = empty_state();
        let (result, _) = with_object_host_context(|| -> Result<Value, RuntimeError> {
            add_effect(&[
                Value::String("First".into()),
                state.clone(),
                Value::Int(100),
            ])?;
            add_effect(&[
                Value::String("XControl".into()),
                state.clone(),
                Value::Int(100),
            ])?;
            remove_effect(&[Value::String("First".into()), state.clone()])?;
            get_effect(&[Value::String("*Control*".into()), state.clone()])
        });
        let value = result.test_value();
        assert_eq!(
            value,
            Value::Int(2),
            "the surviving effect keeps its allocated iNumber"
        );
    }

    #[test]
    fn get_effect_without_name_resolves_by_number_like_cpp() {
        // FnGetEffect with no/empty name treats iIndex as the effect
        // NUMBER (C4Script.cpp:5471-5476 -> C4Effect::Get(iNumber, true),
        // C4Effect.cpp:240-256). Control2Effect's
        // `GetEffect(0, this(), iEffect, 1)` relies on it.
        let state = empty_state();
        let (result, _) = with_object_host_context(|| -> Result<Value, RuntimeError> {
            add_effect(&[
                Value::String("First".into()),
                state.clone(),
                Value::Int(100),
            ])?;
            add_effect(&[
                Value::String("XControl".into()),
                state.clone(),
                Value::Int(100),
            ])?;
            remove_effect(&[Value::String("First".into()), state.clone()])?;
            get_effect(&[Value::Int(0), state.clone(), Value::Int(2), Value::Int(1)])
        });
        let value = result.test_value();
        assert_eq!(
            value,
            Value::String("XControl".into()),
            "number 2 resolves the effect even at list position 0"
        );
    }

    #[test]
    fn effect_call_returns_nil_for_falsy_call_name_or_unknown_number() {
        // FnEffectCall safety (C4Script.cpp:5591-5598): an empty/nil call
        // name and a number that resolves no effect are silent C4VNull,
        // never errors.
        let state = empty_state();
        let (result, _) = with_object_host_context(|| -> Result<Value, RuntimeError> {
            let number = add_effect(&[Value::String("Potion".into()), state.clone()])?;
            assert_eq!(
                effect_call(&[state.clone(), number.clone(), Value::Nil])?,
                Value::Nil,
                "nil call name is a silent nil"
            );
            assert_eq!(
                effect_call(&[state.clone(), number, Value::String(String::new().into())])?,
                Value::Nil,
                "empty call name is a silent nil"
            );
            effect_call(&[
                state.clone(),
                Value::Int(99),
                Value::String("Activate".into()),
            ])
        });
        let value = result.test_value();
        assert_eq!(value, Value::Nil, "unknown effect number is a silent nil");
    }

    #[test]
    fn effect_call_rejects_truthy_non_string_call_name() {
        // C4String* conversion: a truthy non-string parameter is a C++
        // ConvertTo error (C4AulExec.cpp:1364-1396).
        let state = empty_state();
        let (result, _) = with_object_host_context(|| -> Result<Value, RuntimeError> {
            effect_call(&[state.clone(), Value::Int(1), Value::Int(7)])
        });
        result.expect_err("truthy int call name errors like C++ ConvertTo");
    }

    #[test]
    fn effect_call_without_command_target_is_nil_when_no_callback_exists() {
        // C4Effect::DoCall with no command target resolves against
        // Game.ScriptEngine (C4Effect.cpp:450-452); with no such global
        // function the call is a silent C4VNull (:454-455).
        let state = empty_state();
        let (result, _) = with_object_host_context(|| -> Result<Value, RuntimeError> {
            let number = add_effect(&[Value::String("Potion".into()), state.clone()])?;
            effect_call(&[state.clone(), number, Value::String("Activate".into())])
        });
        let value = result.test_value();
        assert_eq!(value, Value::Nil);
    }

    #[test]
    fn effect_call_selects_live_callback_metadata_without_cloning_the_effect_stack() {
        // FnEffectCall resolves the numbered C4Effect node in place before
        // C4Effect::DoCall (C4Script.cpp:5589-5601); it does not copy the list.
        let state = empty_state();
        let (result, _) = with_object_host_context(|| -> Result<Value, RuntimeError> {
            let number = add_effect(&[Value::String("Potion".into()), state.clone()])?;
            reset_effect_snapshot_count();
            let value =
                effect_call(&[state.clone(), number, Value::String("Activate".into())])?;
            assert_eq!(effect_snapshot_count(), 0);
            Ok(value)
        });

        assert_eq!(result.expect("EffectCall succeeds"), Value::Nil);
    }

    #[test]
    fn get_effect_count_filters_by_name_and_priority() {
        let state = empty_state();
        let (result, _) = with_object_host_context(|| -> Result<Value, RuntimeError> {
            add_effect(&[Value::String("Glow".into()), state.clone(), Value::Int(120)])?;
            add_effect(&[Value::String("Spark".into()), state.clone(), Value::Int(80)])?;
            add_effect(&[Value::String("Flame".into()), state.clone(), Value::Int(50)])?;
            get_effect_count(&[Value::Nil, state.clone()])
        });
        let value = result.test_value();
        assert_eq!(value, Value::Int(3));

        let (result, _) = with_object_host_context(|| -> Result<Value, RuntimeError> {
            add_effect(&[Value::String("Glow".into()), state.clone(), Value::Int(120)])?;
            add_effect(&[Value::String("Spark".into()), state.clone(), Value::Int(80)])?;
            add_effect(&[Value::String("Flame".into()), state.clone(), Value::Int(50)])?;
            get_effect_count(&[Value::String("Glow".into()), state.clone()])
        });
        let value = result.test_value();
        assert_eq!(value, Value::Int(1));

        let (result, _) = with_object_host_context(|| -> Result<Value, RuntimeError> {
            add_effect(&[Value::String("Glow".into()), state.clone(), Value::Int(120)])?;
            add_effect(&[Value::String("Spark".into()), state.clone(), Value::Int(80)])?;
            add_effect(&[Value::String("Flame".into()), state.clone(), Value::Int(50)])?;
            get_effect_count(&[Value::Nil, state.clone(), Value::Int(90)])
        });
        let value = result.test_value();
        assert_eq!(value, Value::Int(2));
    }

    #[test]
    fn get_effect_count_scans_the_live_context_without_cloning_its_effect_stack() {
        // FnGetEffectCount scans pTarget->pEffects in place
        // (C4Script.cpp:5559-5568); counting does not copy the C4Effect list.
        let state = empty_state();
        let (result, _) = with_object_host_context(|| -> Result<Value, RuntimeError> {
            add_effect(&[Value::String("Glow".into()), state.clone(), Value::Int(120)])?;
            add_effect(&[Value::String("Spark".into()), state.clone(), Value::Int(80)])?;
            reset_effect_snapshot_count();
            let value = get_effect_count(&[Value::Nil, state.clone()])?;
            assert_eq!(effect_snapshot_count(), 0);
            Ok(value)
        });

        assert_eq!(result.expect("GetEffectCount succeeds"), Value::Int(2));
    }

    #[test]
    fn wildcard_match_follows_cpp_swildcardmatchex() {
        // SWildcardMatchEx (C4Strings.cpp:531-562) via FnWildcardMatch
        // (C4Script.cpp:5606-5609): `*`/`?` with backtracking, int 0/1 result.
        // Clonk.c4d Script.c:587 gates riding controls on
        // `WildcardMatch(GetAction(), "Ride*")`.
        let m = |s: &str, w: &str| {
            wildcard_match(&[Value::String(s.into()), Value::String(w.into())]).test_value()
        };
        assert_eq!(m("Walk", "Ride*"), Value::Int(0));
        assert_eq!(m("Ride", "Ride*"), Value::Int(1));
        assert_eq!(m("RideStill", "Ride*"), Value::Int(1));
        assert_eq!(m("IntJnRAimControl", "*Control*"), Value::Int(1));
        assert_eq!(m("abc", "*b"), Value::Int(0));
        assert_eq!(m("ab", "a?"), Value::Int(1));
        assert_eq!(m("ab", "*"), Value::Int(1));
        assert_eq!(m("", ""), Value::Int(1));
        // FnStringPar maps nil (and Set0'd falsy pars) to "" (C4Script.cpp:78-81).
        assert_eq!(
            wildcard_match(&[Value::Nil, Value::Int(0)]).expect("falsy args succeed"),
            Value::Int(1)
        );
        assert_eq!(
            wildcard_match(&[Value::String("x".into()), Value::Nil]).expect("nil wildcard"),
            Value::Int(0)
        );
    }

    #[test]
    fn effect_name_filters_wildcard_match_like_cpp() {
        // C4Effect::Get/GetCount wildcard-compare effect names
        // (C4Effect.cpp:229,263 via SWildcardMatchEx), and FnRemoveEffect
        // resolves named removals through the same Get (C4Script.cpp:5494);
        // CLNK Control2Effect relies on `GetEffect("*Control*", this(), i)`.
        let state = empty_state();
        let (result, _) = with_object_host_context(|| -> Result<Value, RuntimeError> {
            add_effect(&[
                Value::String("IntJnRAimControl".into()),
                state.clone(),
                Value::Int(100),
            ])?;
            add_effect(&[Value::String("Glow".into()), state.clone(), Value::Int(50)])?;
            let count = get_effect_count(&[Value::String("*Control*".into()), state.clone()])?;
            assert_eq!(count, Value::Int(1));
            let number = get_effect(&[Value::String("*Control*".into()), state.clone()])?;
            assert!(matches!(number, Value::Int(n) if n > 0));
            remove_effect(&[Value::String("*Contr?l*".into()), state.clone()])?;
            get_effect_count(&[Value::Nil, state.clone()])
        });
        assert_eq!(
            result.expect("wildcard filter chain succeeds"),
            Value::Int(1)
        );
    }

    #[test]
    fn get_effect_count_accepts_falsy_name_like_cpp_set0() {
        // Pre-#strict-3 scripts pass falsy values where a C4String* is
        // expected; C4AulExec.cpp:1370-1374 Set0()s them to nil before
        // conversion, so `GetEffectCount(0, this())` (Clonk.c4d Script.c:863
        // Control2Effect) counts all effects like a nil name.
        let state = empty_state();
        let (result, _) = with_object_host_context(|| -> Result<Value, RuntimeError> {
            add_effect(&[Value::String("Glow".into()), state.clone(), Value::Int(120)])?;
            add_effect(&[Value::String("Spark".into()), state.clone(), Value::Int(80)])?;
            get_effect_count(&[Value::Int(0), state.clone()])
        });
        let value = result.test_value();
        assert_eq!(value, Value::Int(2));

        let (result, _) = with_object_host_context(|| -> Result<Value, RuntimeError> {
            add_effect(&[Value::String("Glow".into()), state.clone(), Value::Int(120)])?;
            get_effect_count(&[Value::Bool(false), state.clone()])
        });
        let value = result.test_value();
        assert_eq!(value, Value::Int(1));

        let (result, _) = with_object_host_context(|| -> Result<Value, RuntimeError> {
            get_effect_count(&[Value::Int(7), state.clone()])
        });
        result.expect_err("GetEffectCount with truthy int name errors like C++ ConvertTo");
    }

    #[test]
    fn get_and_remove_effect_accept_falsy_name_like_cpp_set0() {
        // Same Set0 path as above: `GetEffect(0, this(), i)` follows the
        // GetEffectCount(0, …) call in Control2Effect (Clonk.c4d Script.c:868)
        // and JumpAndRun.c:86 calls `RemoveEffect(0, this(), number)`. A
        // falsy name means BY-NUMBER resolution (C4Script.cpp:5474-5476,
        // 5502-5507): the AddEffect handle round-trips, number 0 finds
        // nothing (numbers start at 1, C4Effect.cpp:76-78).
        let state = empty_state();
        let (result, _) = with_object_host_context(|| -> Result<Value, RuntimeError> {
            let number =
                add_effect(&[Value::String("Glow".into()), state.clone(), Value::Int(120)])?;
            assert!(matches!(number, Value::Int(n) if n > 0));
            assert_eq!(
                get_effect(&[Value::Int(0), state.clone(), number.clone()])?,
                number,
                "the AddEffect handle resolves by number"
            );
            assert_eq!(
                get_effect(&[Value::Int(0), state.clone()])?,
                Value::Nil,
                "number 0 matches no effect like C4Effect::Get"
            );
            remove_effect(&[Value::Int(0), state.clone(), number])?;
            get_effect_count(&[Value::Nil, state.clone()])
        });
        let value = result.test_value();
        assert_eq!(value, Value::Int(0));
    }

    #[test]
    fn get_effect_count_reads_state_snapshot_when_no_context() {
        let mut glow = ValueMap::new();
        glow.insert("name".into(), Value::String("Glow".into()));
        glow.insert("priority".into(), Value::Int(100));
        glow.insert("interval".into(), Value::Int(1));
        glow.insert("timer".into(), Value::Int(0));

        let mut spark = ValueMap::new();
        spark.insert("name".into(), Value::String("Spark".into()));
        spark.insert("priority".into(), Value::Int(60));
        spark.insert("interval".into(), Value::Int(1));
        spark.insert("timer".into(), Value::Int(0));

        let mut upper = ValueMap::new();
        upper.insert("name".into(), Value::String("Upper".into()));
        upper.insert("priority".into(), Value::Int(-200));
        upper.insert("interval".into(), Value::Int(1));
        upper.insert("timer".into(), Value::Int(0));

        let state = {
            let mut map = ValueMap::new();
            map.insert(
                "effects".into(),
                Value::Array(vec![
                    Value::Proplist(glow),
                    Value::Proplist(spark),
                    Value::Proplist(upper),
                ]),
            );
            Value::Proplist(map)
        };

        let value = get_effect_count(&[Value::Nil, state.clone(), Value::Nil]).test_value();
        assert_eq!(value, Value::Int(3));

        let value = get_effect_count(&[Value::Nil, state.clone(), Value::Int(0)]).test_value();
        assert_eq!(value, Value::Int(3));

        let value = get_effect_count(&[Value::Nil, state.clone(), Value::Int(100)]).test_value();
        assert_eq!(value, Value::Int(3));

        let value = get_effect_count(&[Value::Nil, state.clone(), Value::Int(-100)]).test_value();
        assert_eq!(value, Value::Int(1));

        let value = get_effect_count(&[Value::String("Spark".into()), state, Value::Int(50)]).test_value();
        assert_eq!(value, Value::Int(0));
    }

    #[test]
    fn effect_var_reads_and_writes_values() {
        let state = empty_state();
        let (result, outcome) = with_object_host_context(|| -> Result<Value, RuntimeError> {
            add_effect(&[
                Value::String("Spark".into()),
                state.clone(),
                Value::Int(100),
                Value::Int(1),
                Value::Nil,
                Value::Nil,
                Value::Int(3),
                object_reference_value(ObjectId::new(44)),
            ])?;

            let initial = effect_var(&[Value::Int(0), state.clone(), Value::Int(1)])?;
            assert_eq!(initial, Value::Nil);

            let object = effect_var(&[Value::Int(1), state.clone(), Value::Int(1)])?;
            assert_eq!(object, Value::Nil);

            effect_var(&[Value::Int(0), state.clone(), Value::Int(1), Value::Int(3)])?;
            effect_var(&[
                Value::Int(1),
                state.clone(),
                Value::Int(1),
                object_reference_value(ObjectId::new(44)),
            ])?;

            let unset = effect_var(&[Value::Int(2), state.clone(), Value::Int(1)])?;
            assert_eq!(unset, Value::Nil);

            let updated = effect_var(&[
                Value::Int(2),
                state.clone(),
                Value::Int(1),
                Value::String("beam".into()),
            ])?;
            assert_eq!(updated, Value::String("beam".into()));

            let reread = effect_var(&[Value::Int(2), state.clone(), Value::Int(1)])?;
            assert_eq!(reread, Value::String("beam".into()));

            Ok(Value::Nil)
        });

        result.test_value();
        assert_eq!(outcome.object.len(), 4);
        match &outcome.object[3] {
            // EffectVar writes fold as number-keyed UPDATEs — an Add
            // would resurrect an effect killed earlier the same frame.
            EffectCommand::Update(effect) => {
                assert_eq!(effect.vars().len(), 3);
                assert_eq!(effect.vars()[0], EffectVarValue::Int(3));
                assert_eq!(effect.vars()[1], EffectVarValue::Object(44));
                assert_eq!(effect.vars()[2], EffectVarValue::String("beam".into()));
            }
            other => panic!("unexpected command: {other:?}"),
        }
    }

    #[test]
    fn effect_var_reads_from_state_without_context() {
        let mut effect_map = ValueMap::new();
        effect_map.insert("name".into(), Value::String("Glow".into()));
        effect_map.insert("priority".into(), Value::Int(80));
        effect_map.insert("interval".into(), Value::Int(1));
        effect_map.insert("timer".into(), Value::Int(0));
        effect_map.insert(
            "vars".into(),
            Value::Array(vec![Value::Int(9), Value::String("pulse".into())]),
        );

        let mut state_map = ValueMap::new();
        state_map.insert(
            "effects".into(),
            Value::Array(vec![Value::Proplist(effect_map.clone())]),
        );
        let state = Value::Proplist(state_map);

        let read_value = effect_var(&[Value::Int(0), state.clone(), Value::Int(1)]).test_value();
        assert_eq!(read_value, Value::Int(9));

        let read_string = effect_var(&[Value::Int(1), state.clone(), Value::Int(1)]).test_value();
        assert_eq!(read_string, Value::String("pulse".into()));

        let set_result = effect_var(&[Value::Int(0), state, Value::Int(1), Value::Int(5)]);
        assert!(set_result.is_err());
    }

    #[test]
    fn set_action_records_object_update() {
        let args = vec![Value::String("Walk".into())];
        let (result, outcome) = with_object_host_context_actions(&["Walk"], || set_action(&args));
        let value = result.test_value();
        assert_eq!(value, Value::Bool(true));
        let update = outcome.object_update.test_value();
        let action = update.action.test_value();
        assert_eq!(action.name.as_deref(), Some("Walk"));
        assert!(outcome.object_commands.is_empty());
        assert!(!outcome.destroy_object);
    }

    #[test]
    fn set_action_sets_the_action_targets_like_cpp() {
        // FnSetAction (C4Script.cpp:747-753): the object arguments are the
        // ACTION's targets — SetActionByName(name, pTarget, pTarget2) —
        // never a which-object guard.
        let mut target_map = ValueMap::new();
        target_map.insert("id".into(), Value::Int(2));
        let args = vec![Value::String("Jump".into()), Value::Proplist(target_map)];
        let (result, outcome) = with_object_host_context_actions(&["Jump"], || set_action(&args));
        let value = result.test_value();
        assert_eq!(value, Value::Bool(true));
        let update = outcome.object_update.test_value();
        let action = update.action.test_value();
        assert_eq!(action.target, Some(Some(ObjectId::new(2))));
    }

    #[test]
    fn set_action_nil_targets_preserve_the_existing_ones_like_cpp() {
        // C4Object::SetAction assigns the targets ONLY when given
        // (C4Object.cpp:4123-4125: `if (pTarget) Action.Target = pTarget;`)
        // — SetAction(name, nil, nil) keeps the previous targets, for
        // explicit nils and omitted arguments alike.
        let args = vec![Value::String("Jump".into()), Value::Nil, Value::Nil];
        let (result, outcome) = with_object_host_context_actions(&["Jump"], || set_action(&args));
        let value = result.test_value();
        assert_eq!(value, Value::Bool(true));
        let update = outcome.object_update.test_value();
        let action = update.action.test_value();
        assert_eq!(action.target, None, "nil target1 must not stage a clear");
        assert_eq!(action.target2, None, "nil target2 must not stage a clear");
    }

    #[test]
    fn set_action_idle_sentinel_ignores_supplied_targets_like_cpp() {
        let target1 = Value::Object(2);
        let target2 = Value::Object(3);
        for name in ["Idle", "ActIdle"] {
            let args = vec![Value::String(name.into()), target1.clone(), target2.clone()];
            let (result, outcome) =
                with_object_host_context_actions(&["Idle", "ActIdle"], || set_action(&args));
            assert_eq!(result.expect("SetAction returns bool"), Value::Bool(true));
            let action = outcome
                .object_update
                .expect("action update recorded")
                .action.test_value();
            assert_eq!(action.name.as_deref(), Some("Idle"));
            assert_eq!(action.target, None, "{name} must ignore target1");
            assert_eq!(action.target2, None, "{name} must ignore target2");
        }
    }

    #[test]
    fn same_slot_set_action_always_clears_phase_delay_like_cpp() {
        let args = [Value::String("Idle".into())];
        let (result, outcome) = with_object_host_context_actions_and_ticks(&[], 6, || {
            Ok::<_, RuntimeError>(Value::Array(vec![set_action(&args)?, get_act_time(&[])?]))
        });
        assert_eq!(
            result.expect("SetAction returns bool"),
            Value::Array(vec![Value::Bool(true), Value::Int(6)]),
            "same-slot SetAction preserves Action.Time"
        );
        let action = outcome
            .object_update
            .expect("action update recorded")
            .action.test_value();
        assert_eq!(action.name.as_deref(), Some("Idle"));
        assert_eq!(action.phase, None, "already-zero phase needs no write");
        assert_eq!(action.ticks, Some(0), "PhaseDelay resets unconditionally");
    }

    #[allow(clippy::arc_with_non_send_sync)]
    fn call_fight_with_fixture(
        caller_ready: bool,
        target_ready: bool,
        caller_reject_body: Option<&str>,
        target_reject_body: Option<&str>,
        args: Vec<Value>,
    ) -> (Result<Value, RuntimeError>, EffectContextOutcome) {
        let caller_id = ObjectId::new(1);
        let target_id = ObjectId::new(2);
        let library = ActionLibrary::new(
            Some("Walk".to_string()),
            HashMap::from([
                ("Walk".to_string(), ActionSpec::default()),
                ("Fight".to_string(), ActionSpec::default()),
                ("Checked".to_string(), ActionSpec::default()),
            ]),
        );
        let caller_ocf = ocf::NORMAL | if caller_ready { ocf::FIGHT_READY } else { 0 };
        let target_ocf = ocf::NORMAL | if target_ready { ocf::FIGHT_READY } else { 0 };

        let world_object = |id, definition: &str, cached_ocf| {
            let mut state = crate::preview_spawn_state(
                Vector2::ZERO,
                OWNER_NONE,
                OWNER_NONE,
                DEFAULT_CATEGORY,
                crate::FULL_CON,
                crate::CONTACT_DENSITY_SOLID,
                Vec::new(),
            );
            state.action = crate::ActionState::new("Walk");
            state.ocf = cached_ocf;
            fixture_world_object(id, definition)
                .with_action_name("Walk")
                .with_ocf(cached_ocf)
                .with_full_state(Rc::new(state))
        };

        let build_script = |probe: bool, reject_body: Option<&str>| {
            let mut script = ScriptEngine::new();
            register_host_functions(&mut script);
            let mut source = String::from("#strict 2\n");
            if probe {
                source.push_str("func Probe(target, clonk) { return FightWith(target, clonk); }\n");
            }
            if let Some(body) = reject_body {
                source.push_str("func RejectFight(who) { ");
                source.push_str(body);
                source.push_str(" }\n");
            }
            script
                .load_script(&source).test_value();
            Arc::new(script)
        };

        let world = HostWorldContext::from_objects([
            world_object(caller_id, "CLNK", caller_ocf),
            world_object(target_id, "TARG", target_ocf),
        ])
        .with_definition_metadata(Rc::new(HashMap::from([
            (
                DefinitionId::from("CLNK"),
                DefinitionMetadata {
                    action_library: library.clone().into(),
                    ..DefinitionMetadata::default()
                },
            ),
            (
                DefinitionId::from("TARG"),
                DefinitionMetadata {
                    action_library: library.clone().into(),
                    ..DefinitionMetadata::default()
                },
            ),
        ])))
        .with_definition_scripts(HashMap::from([
            (
                DefinitionId::from("CLNK"),
                build_script(true, caller_reject_body),
            ),
            (
                DefinitionId::from("TARG"),
                build_script(false, target_reject_body),
            ),
        ]));
        let caller = HostObjectContext {
            id: caller_id,
            action_name: "Walk".to_string(),
            action_library: library.into(),
            ..idle_object_context()
        }
        .with_definition_id("CLNK")
        .with_ocf(caller_ocf);

        with_effect_context(Some(caller), &[], world, 3, || {
            call_world_object_own_function(caller_id, "Probe", &args)
                .unwrap_or_else(|| Err(RuntimeError::new("FightWith fixture Probe is missing")))
        })
    }

    fn fight_with_action(
        outcome: &EffectContextOutcome,
        object: ObjectId,
    ) -> Option<&ActionUpdate> {
        if object == ObjectId::new(1) {
            outcome.object_update.as_ref()?.action.as_ref()
        } else {
            outcome
                .other_objects
                .iter()
                .find(|entry| entry.object_id == object)?
                .update
                .as_ref()?
                .action
                .as_ref()
        }
    }

    #[test]
    fn fight_with_sets_mutual_fight_actions_and_defaults_nil_clonk_to_caller() {
        let caller_id = ObjectId::new(1);
        let target_id = ObjectId::new(2);
        let (result, outcome) = call_fight_with_fixture(
            true,
            true,
            Some("return(who != this());"),
            Some("return(who != this());"),
            vec![object_reference_value(target_id), Value::Nil],
        );

        assert_eq!(result.expect("FightWith succeeds"), Value::Bool(true));
        let caller_action =
            fight_with_action(&outcome, caller_id).test_value();
        assert_eq!(caller_action.name.as_deref(), Some("Fight"));
        assert_eq!(caller_action.target, Some(Some(target_id)));
        let target_action =
            fight_with_action(&outcome, target_id).test_value();
        assert_eq!(target_action.name.as_deref(), Some("Fight"));
        assert_eq!(target_action.target, Some(Some(caller_id)));
    }

    #[test]
    fn fight_with_requires_fight_ready_on_both_objects_without_side_effects() {
        for (caller_ready, target_ready) in [(false, true), (true, false)] {
            let (result, outcome) = call_fight_with_fixture(
                caller_ready,
                target_ready,
                Some("SetAction(\"Checked\"); return(false);"),
                Some("SetAction(\"Checked\"); return(false);"),
                vec![object_reference_value(ObjectId::new(2)), Value::Nil],
            );

            assert_eq!(result.expect("FightWith returns false"), Value::Bool(false));
            assert!(outcome.object_update.is_none(), "caller remains unchanged");
            assert!(
                outcome.other_objects.is_empty(),
                "OCF failure runs no callbacks or actions"
            );
        }
    }

    #[test]
    fn fight_with_target_rejection_runs_first_and_skips_clonk_callback() {
        let (result, outcome) = call_fight_with_fixture(
            true,
            true,
            Some("SetAction(\"Checked\"); return(false);"),
            Some("SetAction(\"Checked\"); return(true);"),
            vec![object_reference_value(ObjectId::new(2)), Value::Nil],
        );

        assert_eq!(
            result.expect("target veto returns false"),
            Value::Bool(false)
        );
        assert!(
            fight_with_action(&outcome, ObjectId::new(1)).is_none(),
            "clonk callback and Fight action are both skipped"
        );
        let target_action = fight_with_action(&outcome, ObjectId::new(2)).test_value();
        assert_eq!(target_action.name.as_deref(), Some("Checked"));
    }

    #[test]
    fn fight_with_clonk_rejection_runs_after_target_and_before_actions() {
        let (result, outcome) = call_fight_with_fixture(
            true,
            true,
            Some("SetAction(\"Checked\"); return(true);"),
            Some("SetAction(\"Checked\"); return(false);"),
            vec![object_reference_value(ObjectId::new(2)), Value::Nil],
        );

        assert_eq!(
            result.expect("clonk veto returns false"),
            Value::Bool(false)
        );
        let caller_action = fight_with_action(&outcome, ObjectId::new(1)).test_value();
        assert_eq!(caller_action.name.as_deref(), Some("Checked"));
        let target_action = fight_with_action(&outcome, ObjectId::new(2)).test_value();
        assert_eq!(target_action.name.as_deref(), Some("Checked"));
    }

    #[test]
    fn fight_with_nil_target_returns_false_without_callbacks_or_actions() {
        let (result, outcome) = call_fight_with_fixture(
            true,
            true,
            Some("SetAction(\"Checked\"); return(false);"),
            Some("SetAction(\"Checked\"); return(false);"),
            vec![Value::Nil, object_reference_value(ObjectId::new(1))],
        );

        assert_eq!(
            result.expect("nil target returns false"),
            Value::Bool(false)
        );
        assert!(outcome.object_update.is_none(), "caller remains unchanged");
        assert!(
            outcome.other_objects.is_empty(),
            "nil target runs no callbacks or actions"
        );
    }

    fn set_object_status_target_world(
        target_status: ObjectStatus,
    ) -> (ObjectId, ObjectId, HostWorldContext) {
        let target_id = ObjectId::new(2);
        let holder_id = ObjectId::new(3);
        let mut target_state = crate::preview_spawn_state(
            Vector2::ZERO,
            OWNER_NONE,
            OWNER_NONE,
            DEFAULT_CATEGORY,
            crate::FULL_CON,
            crate::CONTACT_DENSITY_SOLID,
            Vec::new(),
        );
        target_state.status = target_status;
        target_state.action.target = Some(target_id);
        let target = fixture_world_object(target_id, "TARG")
            .with_status(target_status)
            .with_action_target(Some(target_id))
        .with_full_state(Rc::new(target_state));

        let mut holder_state = crate::preview_spawn_state(
            Vector2::ZERO,
            OWNER_NONE,
            OWNER_NONE,
            DEFAULT_CATEGORY,
            crate::FULL_CON,
            crate::CONTACT_DENSITY_SOLID,
            Vec::new(),
        );
        holder_state.action.target = Some(target_id);
        holder_state.layer = Some(target_id);
        let mut holder_commands = CommandStack::new();
        holder_commands
            .push_back(CommandRequest::new(CommandId::Follow).with_target(Some(target_id))).test_value();
        let holder = fixture_world_object(holder_id, "HOLD")
            .with_action_target(Some(target_id))
        .with_full_state(Rc::new(holder_state))
        .with_commands(holder_commands.command_views())
        .with_command_stack(holder_commands.snapshot());

        let mut script = ScriptEngine::new();
        register_host_functions(&mut script);
        let world = HostWorldContext::from_objects([target, holder])
            .with_definition_metadata(Rc::new(HashMap::from([
                (DefinitionId::from("TARG"), DefinitionMetadata::default()),
                (DefinitionId::from("HOLD"), DefinitionMetadata::default()),
            ])))
            .with_definition_scripts(HashMap::from([(
                DefinitionId::from("TARG"),
                Arc::new(script),
            )]));
        (target_id, holder_id, world)
    }

    #[test]
    fn set_object_status_records_update() {
        let args = vec![Value::Int(ObjectStatus::Inactive.to_script_value())];
        let (result, outcome) = with_object_host_context(|| set_object_status(&args));
        let value = result.test_value();
        assert_eq!(value, Value::Bool(true));
        let update = outcome.object_update.test_value();
        assert_eq!(update.status, Some(ObjectStatus::Inactive));
    }

    #[test]
    fn set_object_status_deactivates_a_foreign_target_without_clearing_pointers() {
        let (target_id, holder_id, world) = set_object_status_target_world(ObjectStatus::Normal);
        let target = object_reference_value(target_id);
        let holder = object_reference_value(holder_id);
        let (result, outcome) = with_object_host_context_with_world(world, || {
            Ok(Value::Array(vec![
                set_object_status(&[
                    Value::Int(ObjectStatus::Inactive.to_script_value()),
                    target.clone(),
                ])?,
                get_object_status(std::slice::from_ref(&target))?,
                get_action_target(&[Value::Int(0), holder.clone()])?,
                get_command(&[holder.clone(), Value::Int(1)])?,
                get_object_layer(&[holder])?,
            ]))
        });

        assert_eq!(
            result.expect("foreign SetObjectStatus succeeds"),
            Value::Array(vec![
                Value::Bool(true),
                Value::Int(ObjectStatus::Inactive.to_script_value()),
                object_reference_value(target_id),
                object_reference_value(target_id),
                object_reference_value(target_id),
            ])
        );
        assert!(outcome.object_update.is_none(), "caller remains unchanged");
        let update = outcome
            .other_objects
            .iter()
            .find(|object| object.object_id == target_id)
            .and_then(|object| object.update.as_ref()).test_value();
        assert_eq!(update.status, Some(ObjectStatus::Inactive));
        assert!(
            outcome
                .other_objects
                .iter()
                .all(|object| object.object_id != holder_id),
            "clear=false leaves inter-object pointers untouched"
        );
    }

    #[test]
    fn set_object_status_clear_pointers_clears_foreign_action_and_command_targets() {
        let (target_id, holder_id, world) = set_object_status_target_world(ObjectStatus::Normal);
        let target = object_reference_value(target_id);
        let holder = object_reference_value(holder_id);
        let (result, outcome) = with_object_host_context_with_world(world, || {
            Ok(Value::Array(vec![
                get_action_target(&[Value::Int(0), holder.clone()])?,
                get_command(&[holder.clone(), Value::Int(1)])?,
                get_object_layer(std::slice::from_ref(&holder))?,
                set_object_status(&[
                    Value::Int(ObjectStatus::Inactive.to_script_value()),
                    target.clone(),
                    Value::Bool(true),
                ])?,
                get_action_target(&[Value::Int(0), holder.clone()])?,
                get_command(&[holder.clone(), Value::Int(1)])?,
                get_object_layer(&[holder])?,
                get_object_status(&[target])?,
            ]))
        });

        assert_eq!(
            result.expect("foreign SetObjectStatus succeeds"),
            Value::Array(vec![
                object_reference_value(target_id),
                object_reference_value(target_id),
                object_reference_value(target_id),
                Value::Bool(true),
                Value::Nil,
                Value::Nil,
                Value::Nil,
                Value::Int(ObjectStatus::Inactive.to_script_value()),
            ])
        );
        assert!(outcome.object_update.is_none(), "caller remains unchanged");
        let target_outcome = outcome
            .other_objects
            .iter()
            .find(|object| object.object_id == target_id).test_value();
        let target_update = target_outcome
            .update
            .as_ref().test_value();
        assert_eq!(target_update.status, Some(ObjectStatus::Inactive));
        assert_eq!(
            target_update
                .action
                .as_ref()
                .expect("self action pointer clear recorded")
                .target,
            Some(None)
        );
        let holder_outcome = outcome
            .other_objects
            .iter()
            .find(|object| object.object_id == holder_id).test_value();
        let holder_update = holder_outcome
            .update
            .as_ref().test_value();
        assert_eq!(
            holder_update
                .action
                .as_ref()
                .expect("holder action target clear recorded")
                .target,
            Some(None)
        );
        assert_eq!(holder_update.layer, Some(None));
        match holder_outcome.command_operations.as_slice() {
            [CommandOperation::Restore(snapshot)] => assert_eq!(
                snapshot
                    .command_views()
                    .first()
                    .expect("restored holder command")
                    .target,
                None
            ),
            other => panic!("expected cleared holder command restore, got {other:?}"),
        }
    }

    #[test]
    fn set_object_status_same_status_is_a_side_effect_free_success() {
        let (target_id, holder_id, world) = set_object_status_target_world(ObjectStatus::Inactive);
        let target = object_reference_value(target_id);
        let holder = object_reference_value(holder_id);
        let (result, outcome) = with_object_host_context_with_world(world, || {
            Ok(Value::Array(vec![
                set_object_status(&[
                    Value::Int(ObjectStatus::Inactive.to_script_value()),
                    target,
                    Value::Bool(true),
                ])?,
                get_action_target(&[Value::Int(0), holder.clone()])?,
                get_command(&[holder, Value::Int(1)])?,
            ]))
        });
        assert_eq!(
            result.expect("same-status call succeeds"),
            Value::Array(vec![
                Value::Bool(true),
                object_reference_value(target_id),
                object_reference_value(target_id),
            ])
        );
        assert!(outcome.object_update.is_none());
        assert!(outcome.other_objects.is_empty());
        assert!(outcome.player_commands.is_empty());
    }

    #[test]
    fn set_object_status_rejects_a_deleted_target() {
        let (target_id, _, world) = set_object_status_target_world(ObjectStatus::Deleted);
        let (result, outcome) = with_object_host_context_with_world(world, || {
            set_object_status(&[
                Value::Int(ObjectStatus::Inactive.to_script_value()),
                object_reference_value(target_id),
            ])
        });
        assert_eq!(result.expect("dead target is rejected"), Value::Bool(false));
        assert!(outcome.object_update.is_none());
        assert!(outcome.other_objects.is_empty());
        assert!(outcome.player_commands.is_empty());
    }

    #[test]
    fn set_object_status_rejects_deleted() {
        let args = vec![Value::Int(ObjectStatus::Deleted.to_script_value())];
        let (result, outcome) = with_object_host_context(|| set_object_status(&args));
        let value = result.test_value();
        assert_eq!(value, Value::Bool(false));
        assert!(outcome.object_update.is_none());
    }

    #[test]
    fn get_object_status_reflects_pending_update() {
        let (result, outcome) = with_object_host_context(|| -> Result<Value, RuntimeError> {
            let set_value =
                set_object_status(&[Value::Int(ObjectStatus::Inactive.to_script_value())])?;
            assert_eq!(set_value, Value::Bool(true));
            get_object_status(&[])
        });

        let value = result.test_value();
        assert_eq!(value, Value::Int(ObjectStatus::Inactive.to_script_value()));
        let update = outcome.object_update.test_value();
        assert_eq!(update.status, Some(ObjectStatus::Inactive));
    }

    #[test]
    fn get_entrance_without_an_object_returns_nil() {
        assert_eq!(
            get_entrance(&[]).expect("GetEntrance without a host context succeeds"),
            Value::Nil
        );
    }

    #[test]
    fn get_owner_returns_current_owner() {
        let (result, _) = with_effect_context(
            Some(HostObjectContext {
                owner: 5,
                controller: 5,
                ..idle_object_context()
            }),
            &[],
            HostWorldContext::default(),
            1,
            || get_owner(&[]),
        );

        let value = result.test_value();
        assert_eq!(value, Value::Int(5));
    }

    #[test]
    fn get_owner_reads_world_when_target_provided() {
        let world = HostWorldContext::from_objects(vec![fixture_world_object(
            ObjectId::new(7),
            "Dummy",
        )
            .with_owner(42)]);
        let args = [object_reference_value(ObjectId::new(7))];
        let (result, _) = with_effect_context(None, &[], world, 1, || get_owner(&args));

        let value = result.test_value();
        assert_eq!(value, Value::Int(42));
    }

    #[test]
    fn crew_member_reads_raw_effective_definitions_or_nil_like_cpp() {
        let script = r#"#strict
func Probe(object pRaw, object pRuntimeCrew)
{
    var unset;
    return [CrewMember(), CrewMember(pRaw), CrewMember(unset), pRaw->CrewMember(), CrewMember(pRuntimeCrew)];
}

func ChangeAndProbe()
{
    ChangeDef(ZERO);
    return CrewMember();
}
"#;
        let caller_dir = tempfile::Builder::new()
            .prefix("lc-test-")
            .tempdir().test_value();
        std::fs::write(
            caller_dir.path().join("DefCore.txt"),
            b"[DefCore]\nid=CALL\nName=Caller\nCrewMember=-2\n",
        ).test_value();
        std::fs::write(caller_dir.path().join("Script.c"), script).test_value();
        let caller_group =
            clonk_resources::Group::open(caller_dir.path()).test_value();
        let caller_resource = clonk_resources::ResourceDefinition::load(&caller_group).test_value();
        let caller_definition =
            crate::Definition::from_resource(&caller_resource).test_value();
        assert_eq!(caller_definition.crew_member_value(), -2);
        assert!(caller_definition.is_crew());
        let mut raw_definition =
            test_definition("RAWW", "Raw", "#strict\n");
        raw_definition.set_crew_member_value(7);
        let zero_definition =
            test_definition("ZERO", "Zero", "#strict\n");

        let mut engine = crate::Engine::with_seed(0);
        engine.register_test_definition(caller_definition);
        engine.register_test_definition(raw_definition);
        engine.register_test_definition(zero_definition);

        // Runtime crew membership is deliberately the inverse of the
        // definitions. FnCrewMember reads Def->CrewMember, not Object state
        // or OCF_CrewMember.
        let caller = engine.spawn_test_object(crate::SpawnConfig::new("CALL").with_crew_member(false));
        let raw = engine.spawn_test_object(crate::SpawnConfig::new("RAWW").with_crew_member(false));
        let runtime_crew = engine.spawn_test_object(crate::SpawnConfig::new("ZERO").with_crew_member(true));
        let caller_index = engine.find_object_index(caller).test_value();

        assert_eq!(
            engine
                .call_object_function(
                    caller_index,
                    "Probe",
                    vec![
                        object_reference_value(raw),
                        object_reference_value(runtime_crew),
                    ],
                )
                .expect("CrewMember probe runs"),
            Value::Array(vec![
                Value::Int(-2),
                Value::Int(7),
                Value::Int(-2),
                Value::Int(7),
                Value::Int(0),
            ])
        );

        let changed = engine
            .call_object_function(caller_index, "ChangeAndProbe", vec![]).test_value();

        let carrier = HostObjectContext {
            id: ObjectId::new(99),
            ..idle_object_context()
        }
        .with_definition_id("RAWW");
        let (global, _) = with_effect_context_with_state_and_definition(
            Some(carrier),
            Some(DefinitionId::from("CALL")),
            None,
            &[],
            engine.host_world_context(),
            4,
            false,
            || crew_member(&[]),
        );
        assert_eq!(
            global.expect("definition-only CrewMember succeeds"),
            Value::Nil,
            "cthr->Def must not substitute for a missing cthr->Obj"
        );

        assert_eq!(
            changed,
            Value::Int(0),
            "same-call ChangeDef must switch the effective definition"
        );
    }

    fn set_owner_target_world(
        owner: i32,
        controller: i32,
        players: Vec<PlayerState>,
    ) -> (ObjectId, HostWorldContext) {
        let target_id = ObjectId::new(2);
        let target_state = crate::preview_spawn_state(
            Vector2::ZERO,
            owner,
            controller,
            DEFAULT_CATEGORY,
            crate::FULL_CON,
            crate::CONTACT_DENSITY_SOLID,
            Vec::new(),
        );
        let target = fixture_world_object(target_id, "TARG")
            .with_owner(owner)
        .with_full_state(Rc::new(target_state));
        let mut script = ScriptEngine::new();
        register_host_functions(&mut script);
        let world = HostWorldContext::from_objects_with_players([target], players)
            .with_definition_metadata(Rc::new(HashMap::from([(
                DefinitionId::from("TARG"),
                DefinitionMetadata::default(),
            )])))
            .with_definition_scripts(HashMap::from([(
                DefinitionId::from("TARG"),
                Arc::new(script),
            )]));
        (target_id, world)
    }

    #[test]
    fn set_owner_records_owner_update() {
        let (result, outcome) = with_effect_context(
            Some(HostObjectContext {
                owner: 1,
                controller: 1,
                ..idle_object_context()
            }),
            &[],
            HostWorldContext::from_objects_with_players(
                Vec::<HostWorldObject>::new(),
                [PlayerState {
                    id: 3,
                    ..PlayerState::default()
                }],
            ),
            1,
            || set_owner(&[Value::Int(3)]),
        );

        let value = result.test_value();
        assert_eq!(value, Value::Bool(true));
        let update = outcome.object_update.test_value();
        assert_eq!(update.owner, Some(3));
        assert_eq!(update.controller, Some(3));
    }

    #[test]
    fn set_owner_targets_a_foreign_object_for_a_valid_player() {
        // FnSetOwner forwards the explicit pObj to C4Object::SetOwner
        // (C4Script.cpp:821-827), which updates Owner and Controller.
        let (target_id, world) = set_owner_target_world(
            OWNER_NONE,
            4,
            vec![PlayerState {
                id: 1,
                ..PlayerState::default()
            }],
        );

        let (result, outcome) = with_object_host_context_with_world(world, || {
            set_owner(&[Value::Int(1), object_reference_value(target_id)])
        });

        assert_eq!(result.expect("SetOwner succeeds"), Value::Bool(true));
        assert!(outcome.object_update.is_none(), "caller remains unchanged");
        assert_eq!(outcome.other_objects.len(), 1, "target changes once");
        let update = outcome.other_objects[0]
            .update
            .as_ref().test_value();
        assert_eq!(update.owner, Some(1));
        assert_eq!(update.controller, Some(1));
    }

    #[test]
    fn set_owner_rejects_an_invalid_player_without_changes() {
        let world = HostWorldContext::from_objects_with_players(
            Vec::<HostWorldObject>::new(),
            [PlayerState {
                id: 1,
                ..PlayerState::default()
            }],
        );
        let (result, outcome) =
            with_object_host_context_with_world(world, || set_owner(&[Value::Int(7)]));

        assert_eq!(result.expect("SetOwner returns bool"), Value::Bool(false));
        assert!(outcome.object_update.is_none());
        assert!(outcome.other_objects.is_empty());
    }

    #[test]
    fn set_owner_accepts_no_owner_for_a_foreign_object() {
        let (target_id, world) = set_owner_target_world(1, 4, Vec::new());
        let (result, outcome) = with_object_host_context_with_world(world, || {
            set_owner(&[Value::Int(OWNER_NONE), object_reference_value(target_id)])
        });

        assert_eq!(result.expect("SetOwner succeeds"), Value::Bool(true));
        assert!(outcome.object_update.is_none(), "caller remains unchanged");
        assert_eq!(outcome.other_objects.len(), 1, "target changes once");
        let update = outcome.other_objects[0]
            .update
            .as_ref().test_value();
        assert_eq!(update.owner, Some(OWNER_NONE));
        assert_eq!(update.controller, Some(OWNER_NONE));
    }

    fn set_category_target_world(target_id: ObjectId, category: i32) -> HostWorldContext {
        let state = crate::preview_spawn_state(
            Vector2::ZERO,
            OWNER_NONE,
            OWNER_NONE,
            category,
            crate::FULL_CON,
            crate::CONTACT_DENSITY_SOLID,
            Vec::new(),
        );
        let target = HostWorldObject::with_category(
            target_id,
            "TARG",
            ObjectStatus::Normal,
            "Idle",
            None,
            None,
            None,
            OWNER_NONE,
            category,
            100,
            crate::FULL_CON,
            0,
            Vector2::ZERO,
            Vector2::ZERO,
            0,
            Vec::new(),
            0,
            0,
            0,
            None,
            None,
        )
        .with_full_state(Rc::new(state));
        HostWorldContext::from_objects([target])
    }

    fn foreign_category_update(outcome: &EffectContextOutcome, target: ObjectId) -> Option<i32> {
        outcome
            .other_objects
            .iter()
            .find(|object| object.object_id == target)?
            .update
            .as_ref()?
            .category
    }

    #[test]
    fn set_category_changes_a_foreign_target_and_leaves_the_caller_unchanged() {
        let target_id = ObjectId::new(2);
        let world = set_category_target_world(target_id, crate::CATEGORY_VEHICLE);
        let target = object_reference_value(target_id);
        let (result, outcome) = with_object_host_context_with_world(world, || {
            Ok(Value::Array(vec![
                set_category(&[Value::Int(crate::CATEGORY_STATIC_BACK), target.clone()])?,
                get_category(&[target])?,
            ]))
        });

        assert_eq!(
            result.expect("foreign SetCategory succeeds"),
            Value::Array(vec![
                Value::Bool(true),
                Value::Int(crate::CATEGORY_STATIC_BACK),
            ])
        );
        assert!(outcome.object_update.is_none(), "caller remains unchanged");
        assert_eq!(
            foreign_category_update(&outcome, target_id),
            Some(crate::CATEGORY_STATIC_BACK)
        );
    }

    #[test]
    fn set_category_merges_the_foreign_targets_own_sort_bits() {
        let target_id = ObjectId::new(2);
        let world = set_category_target_world(target_id, crate::CATEGORY_VEHICLE);
        let target = object_reference_value(target_id);
        let expected = crate::CATEGORY_MAGIC | crate::CATEGORY_VEHICLE;
        let (result, outcome) = with_object_host_context_with_world(world, || {
            Ok(Value::Array(vec![
                set_category(&[Value::Int(crate::CATEGORY_MAGIC), target.clone()])?,
                get_category(&[target])?,
            ]))
        });

        assert_eq!(
            result.expect("foreign SetCategory merge succeeds"),
            Value::Array(vec![Value::Bool(true), Value::Int(expected)])
        );
        assert!(outcome.object_update.is_none(), "caller remains unchanged");
        assert_eq!(foreign_category_update(&outcome, target_id), Some(expected));
    }

    #[test]
    fn set_category_does_not_invent_sort_bits_when_the_target_has_none() {
        let target_id = ObjectId::new(2);
        let world = set_category_target_world(target_id, 0);
        let target = object_reference_value(target_id);
        let expected = crate::CATEGORY_MAGIC;
        let (result, outcome) = with_object_host_context_with_world(world, || {
            Ok(Value::Array(vec![
                set_category(&[Value::Int(expected), target.clone()])?,
                get_category(&[target])?,
            ]))
        });

        assert_eq!(
            result.expect("foreign SetCategory succeeds"),
            Value::Array(vec![Value::Bool(true), Value::Int(expected)])
        );
        assert!(outcome.object_update.is_none(), "caller remains unchanged");
        assert_eq!(foreign_category_update(&outcome, target_id), Some(expected));
    }

    #[test]
    fn set_category_accepts_an_explicit_target_without_an_object_context() {
        let target_id = ObjectId::new(2);
        let world = set_category_target_world(target_id, crate::CATEGORY_VEHICLE);
        let target = object_reference_value(target_id);
        let (result, outcome) = with_effect_context(None, &[], world, 3, || {
            Ok::<_, RuntimeError>(Value::Array(vec![
                set_category(&[Value::Int(crate::CATEGORY_STATIC_BACK), target.clone()])?,
                get_category(&[target])?,
            ]))
        });

        assert_eq!(
            result.expect("scenario-scope SetCategory succeeds"),
            Value::Array(vec![
                Value::Bool(true),
                Value::Int(crate::CATEGORY_STATIC_BACK),
            ])
        );
        assert!(outcome.object_update.is_none());
        assert_eq!(
            foreign_category_update(&outcome, target_id),
            Some(crate::CATEGORY_STATIC_BACK)
        );
    }

    #[test]
    fn set_alive_records_alive_update() {
        let (result, outcome) = with_effect_context(
            Some(
                idle_object_context()
                .with_alive(true),
            ),
            &[],
            HostWorldContext::default(),
            1,
            || set_alive(&[Value::Bool(false)]),
        );

        let value = result.test_value();
        assert_eq!(value, Value::Bool(true));
        let update = outcome.object_update.test_value();
        assert_eq!(update.alive, Some(false));
    }

    fn set_alive_target_world(target_id: ObjectId, alive: bool) -> HostWorldContext {
        let mut state = crate::preview_spawn_state(
            Vector2::ZERO,
            OWNER_NONE,
            OWNER_NONE,
            DEFAULT_CATEGORY,
            crate::FULL_CON,
            crate::CONTACT_DENSITY_SOLID,
            Vec::new(),
        );
        state.alive = alive;
        let target = fixture_world_object(target_id, "CLNK")
        .with_alive(alive)
        .with_full_state(Rc::new(state));
        HostWorldContext::from_objects(vec![target])
    }

    #[test]
    fn set_alive_revives_a_foreign_target_and_exposes_the_staged_state() {
        let target_id = ObjectId::new(2);
        let world = set_alive_target_world(target_id, false);
        let target = object_reference_value(target_id);
        let (result, outcome) = with_object_host_context_with_world(world, || {
            Ok(Value::Array(vec![
                set_alive(&[Value::Bool(true), target.clone()])?,
                get_alive(std::slice::from_ref(&target))?,
                get_alive(&[])?,
            ]))
        });

        assert_eq!(
            result.expect("foreign SetAlive succeeds"),
            Value::Array(vec![
                Value::Bool(true),
                Value::Bool(true),
                Value::Bool(true),
            ])
        );
        assert!(outcome.object_update.is_none(), "caller remains unchanged");
        let update = outcome
            .other_objects
            .iter()
            .find(|object| object.object_id == target_id)
            .and_then(|object| object.update.as_ref()).test_value();
        assert_eq!(update.alive, Some(true));
    }

    #[test]
    fn set_alive_clears_a_foreign_target() {
        let target_id = ObjectId::new(2);
        let world = set_alive_target_world(target_id, true);
        let target = object_reference_value(target_id);
        let (result, outcome) = with_object_host_context_with_world(world, || {
            Ok(Value::Array(vec![
                set_alive(&[Value::Bool(false), target.clone()])?,
                get_alive(&[target])?,
            ]))
        });

        assert_eq!(
            result.expect("foreign SetAlive succeeds"),
            Value::Array(vec![Value::Bool(true), Value::Bool(false)])
        );
        assert!(outcome.object_update.is_none(), "caller remains unchanged");
        let update = outcome
            .other_objects
            .iter()
            .find(|object| object.object_id == target_id)
            .and_then(|object| object.update.as_ref()).test_value();
        assert_eq!(update.alive, Some(false));
    }

    #[test]
    fn get_alive_returns_current_state() {
        let (result, _) = with_effect_context(
            Some(
                idle_object_context()
                .with_alive(false),
            ),
            &[],
            HostWorldContext::default(),
            1,
            || get_alive(&[]),
        );

        let value = result.test_value();
        assert_eq!(value, Value::Bool(false));
    }

    #[test]
    fn get_alive_reads_world_when_target_provided() {
        let world = HostWorldContext::from_objects(vec![HostWorldObject::new(
            ObjectId::new(7),
            "Dummy",
            ObjectStatus::Normal,
            "Idle",
            None,
            None,
            None,
            OWNER_NONE,
            100,
            crate::FULL_CON,
            Vector2::ZERO,
            Vector2::ZERO,
            Vec::new(),
            0,
            0,
            None,
        )
        .with_alive(false)]);
        let args = [object_reference_value(ObjectId::new(7))];
        let (result, _) = with_effect_context(None, &[], world, 1, || get_alive(&args));

        let value = result.test_value();
        assert_eq!(value, Value::Bool(false));
    }

    #[test]
    fn kill_pads_discards_and_exposes_same_call_death() {
        // Typed C4Aul dispatch pads target=nil, converts the bool slot, and
        // discards surplus values. The first live call succeeds; Alive is
        // already false to the rest of the VM call, so a repeat is false
        // (oracle-src-pinned src/C4Script.cpp:335-345;
        // src/C4Object.cpp:1164-1205).
        let args = [
            Value::Nil,
            Value::Bool(true),
            Value::String("discarded".to_string().into()),
        ];
        let (result, outcome) = with_object_host_context(|| {
            assert_eq!(kill(&args)?, Value::Bool(true));
            assert_eq!(get_alive(&[])?, Value::Bool(false));
            assert_eq!(kill(&[])?, Value::Bool(false));
            Ok::<_, RuntimeError>(())
        });
        result.test_value();
        assert!(
            outcome.other_objects.is_empty(),
            "the active object's synchronous death stays on its update channel"
        );
        let update = outcome
            .object_update.test_value();
        assert_eq!(update.alive, Some(false));
        assert_eq!(update.selected, Some(false));
    }

    #[test]
    fn kill_foreign_target_uses_the_valid_callers_controller() {
        // FnKill attributes a foreign target to the valid calling Controller
        // before completing AssignDeath synchronously
        // (oracle-src-pinned src/C4Script.cpp:335-345;
        // src/C4Object.cpp:1164-1205).
        let target_state = Rc::new(crate::preview_spawn_state(
            Vector2::ZERO,
            1,
            1,
            crate::DEFAULT_CATEGORY,
            crate::FULL_CON,
            crate::CONTACT_DENSITY_SOLID,
            Vec::new(),
        ));
        let target = fixture_world_object(ObjectId::new(7), "Dummy")
            .with_owner(1)
        .with_full_state(target_state);
        let world = HostWorldContext::from_objects_with_players(
            [target],
            [PlayerState {
                id: 0,
                ..PlayerState::default()
            }],
        )
        .with_definition_metadata(Rc::new(HashMap::from([(
            DefinitionId::from("Dummy"),
            DefinitionMetadata::default(),
        )])));
        let caller = HostObjectContext {
            owner: 0,
            controller: 0,
            ..idle_object_context()
        };
        let (result, outcome) = with_effect_context(Some(caller), &[], world, 8, || {
            assert_eq!(
                kill(&[object_reference_value(ObjectId::new(7))])?,
                Value::Bool(true)
            );
            assert_eq!(
                get_alive(&[object_reference_value(ObjectId::new(7))])?,
                Value::Bool(false),
                "foreign GetAlive reads the live nested Kill preview"
            );
            Ok::<_, RuntimeError>(())
        });
        result.test_value();
        let death = outcome
            .other_objects
            .iter()
            .find(|outcome| outcome.object_id == ObjectId::new(7)).test_value();
        assert_eq!(
            death.update.as_ref().and_then(|update| update.alive),
            Some(false),
            "foreign AssignDeath completes before copy-out"
        );
        assert_eq!(death.assign_death, None);
        assert_eq!(
            death
                .update
                .as_ref()
                .and_then(|update| update.energy_loss_cause),
            Some(0),
            "a valid calling Controller receives kill attribution"
        );
    }

    #[test]
    fn do_energy_runs_assign_death_before_the_invoking_script_continues() {
        // C4Object::DoEnergy calls AssignDeath inline on the nonzero -> zero
        // transition, and AssignDeath calls Death before DoEnergy returns
        // (oracle-src-pinned src/C4Object.cpp:1164-1205,1372-1393).
        let mut definition = test_definition("CLNK", "Clonk", r#"#strict
        local order;
        public func Trigger()
        {
            order = 1;
            DoEnergy(-10, 0, true);
            order = order * 10 + 3;
            return order;
        }
        protected func Death()
        {
            order = order * 10 + 2;
        }
        "#);
        definition.set_physical(PhysicalInfo {
            energy: C4_MAX_PHYSICAL,
            ..PhysicalInfo::default()
        });
        let death_calls = Arc::new(std::sync::atomic::AtomicUsize::new(0));
        let observed_death_calls = Arc::clone(&death_calls);
        definition.set_debugger_hooks(clonk_script::DebuggerHooks::new().with_on_call(
            move |name, _| {
                if name == "Death" {
                    observed_death_calls.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
                }
            },
        ));
        let mut engine = crate::Engine::with_seed(0);
        engine.register_test_definition(definition);
        let clonk = engine.spawn_test_object(SpawnConfig::new("CLNK"));
        let index = engine.find_object_index(clonk).test_value();
        engine.objects[index].state.energy = 10;
        engine.objects[index].state.alive = true;

        let result = engine
            .call_object_function(index, "Trigger", Vec::new()).test_value();
        assert_eq!(engine.objects[index].state.energy, 0);
        assert_eq!(
            death_calls.load(std::sync::atomic::Ordering::SeqCst),
            1,
            "the synchronous lifecycle must call Death exactly once"
        );
        assert_eq!(
            result,
            Value::Int(123),
            "Death must run between DoEnergy and the following statement"
        );
    }

    #[test]
    fn punch_suppresses_catch_blow_after_lethal_death_removes_target() {
        // ObjectComPunch continues its native fling bookkeeping after
        // DoEnergy, but the final pTarget->Call(CatchBlow) is a no-op when
        // synchronous Death made Status zero (oracle-src-pinned
        // src/C4ObjectCom.cpp:737-767; src/C4Object.cpp:2224-2227).
        let catch_calls = Arc::new(std::sync::atomic::AtomicUsize::new(0));
        let observed_catch_calls = Arc::clone(&catch_calls);
        let attacker_definition = test_definition("PATK", "Punch attacker", r#"#strict
        public func Trigger(target)
        {
            return Punch(target, 10);
        }
        "#);
        let mut target_definition = test_definition("PTGT", "Punch target", r#"#strict
        protected func Death()
        {
            RemoveObject();
        }
        protected func CatchBlow(strength, attacker)
        {
            return 1;
        }
        "#);
        target_definition.set_category(crate::CATEGORY_OBJECT | crate::CATEGORY_LIVING);
        target_definition.configure_actions(
            Some("Idle".to_string()),
            HashMap::from([
                ("Idle".to_string(), crate::ActionSpec::default()),
                ("Dead".to_string(), crate::ActionSpec::default()),
                ("Tumble".to_string(), crate::ActionSpec::default()),
                ("GetPunched".to_string(), crate::ActionSpec::default()),
            ]),
        );
        target_definition.set_debugger_hooks(clonk_script::DebuggerHooks::new().with_on_call(
            move |name, _| {
                if name == "CatchBlow" {
                    observed_catch_calls.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
                }
            },
        ));

        let mut engine = crate::Engine::with_seed(0);
        engine.register_test_definition(attacker_definition);
        engine.register_test_definition(target_definition);
        let attacker = engine.spawn_test_object(SpawnConfig::new("PATK"));
        let target = engine.spawn_test_object(SpawnConfig::new("PTGT").with_alive(true));
        let target_index = engine.find_object_index(target).test_value();
        engine.objects[target_index].state.energy = 1;
        let attacker_index = engine.find_object_index(attacker).test_value();

        assert_eq!(
            engine
                .call_object_function(
                    attacker_index,
                    "Trigger",
                    vec![object_reference_value(target)],
                )
                .expect("lethal Punch completes"),
            Value::Bool(true)
        );
        assert!(engine.objects[target_index].destroyed);
        assert_eq!(
            catch_calls.load(std::sync::atomic::Ordering::SeqCst),
            0,
            "C4Object::Call suppresses CatchBlow after Death removes the target"
        );
    }

    #[test]
    fn do_energy_stops_damage_effect_walk_after_target_removal() {
        // C4Effect::DoDamage rechecks pObj->Status after every Fx*Damage
        // callback and returns immediately once the target was removed
        // (oracle-src-pinned src/C4Effect.cpp:427-436).
        let first_calls = Arc::new(std::sync::atomic::AtomicUsize::new(0));
        let second_calls = Arc::new(std::sync::atomic::AtomicUsize::new(0));
        let observed_first = Arc::clone(&first_calls);
        let observed_second = Arc::clone(&second_calls);
        let mut target_definition = test_definition("TARG", "Target", r#"#strict
        public func Setup()
        {
            AddEffect("First", this, 10, 0, this);
            AddEffect("Second", this, 20, 0, this);
        }
        protected func FxFirstDamage(target, number, change, cause, caused_by)
        {
            RemoveObject(target);
            return change;
        }
        protected func FxSecondDamage(target, number, change, cause, caused_by)
        {
            return change;
        }
        "#);
        target_definition.set_debugger_hooks(clonk_script::DebuggerHooks::new().with_on_call(
            move |name, _| match name {
                "FxFirstDamage" => {
                    observed_first.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
                }
                "FxSecondDamage" => {
                    observed_second.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
                }
                _ => {}
            },
        ));
        let caller_definition = test_definition("CALL", "Caller", r#"#strict
        public func Trigger(target)
        {
            return DoEnergy(-1, target, true);
        }
        "#);

        let mut engine = crate::Engine::with_seed(0);
        engine.register_test_definition(target_definition);
        engine.register_test_definition(caller_definition);
        let target = engine.spawn_test_object(SpawnConfig::new("TARG").with_alive(true));
        let caller = engine.spawn_test_object(SpawnConfig::new("CALL"));
        let target_index = engine.find_object_index(target).test_value();
        engine
            .call_object_function(target_index, "Setup", Vec::new()).test_value();
        let caller_index = engine.find_object_index(caller).test_value();

        assert_eq!(
            engine
                .call_object_function(
                    caller_index,
                    "Trigger",
                    vec![object_reference_value(target)],
                )
                .expect("foreign DoEnergy returns"),
            Value::Bool(true)
        );
        assert_eq!(first_calls.load(std::sync::atomic::Ordering::SeqCst), 1);
        assert_eq!(
            second_calls.load(std::sync::atomic::Ordering::SeqCst),
            0,
            "the removed target stops the live C4Effect list walk"
        );
    }

    #[test]
    fn do_energy_skips_a_later_damage_effect_removed_by_an_earlier_hook() {
        // C4Effect::DoDamage reads IsDead() from each live list node when it
        // reaches that node; an earlier hook may therefore suppress a later
        // hook by marking its effect dead (oracle-src-pinned
        // src/C4Effect.cpp:427-436; src/C4Script.cpp:5487-5513).
        let first_calls = Arc::new(std::sync::atomic::AtomicUsize::new(0));
        let second_calls = Arc::new(std::sync::atomic::AtomicUsize::new(0));
        let observed_first = Arc::clone(&first_calls);
        let observed_second = Arc::clone(&second_calls);
        let mut target_definition = test_definition("TARG", "Target", r#"#strict
        public func Setup()
        {
            AddEffect("First", this, 10, 0, this);
            AddEffect("Second", this, 20, 0, this);
        }
        public func Trigger()
        {
            return DoEnergy(-1, this, true);
        }
        protected func FxFirstDamage(target, number, change, cause, caused_by)
        {
            RemoveEffect("Second", target, 0, true);
            return change;
        }
        protected func FxSecondDamage(target, number, change, cause, caused_by)
        {
            return change;
        }
        "#);
        target_definition.set_debugger_hooks(clonk_script::DebuggerHooks::new().with_on_call(
            move |name, _| match name {
                "FxFirstDamage" => {
                    observed_first.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
                }
                "FxSecondDamage" => {
                    observed_second.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
                }
                _ => {}
            },
        ));

        let mut engine = crate::Engine::with_seed(0);
        engine.register_test_definition(target_definition);
        let target = engine.spawn_test_object(SpawnConfig::new("TARG").with_alive(true));
        let target_index = engine.find_object_index(target).test_value();
        engine
            .call_object_function(target_index, "Setup", Vec::new()).test_value();

        assert_eq!(
            engine
                .call_object_function(target_index, "Trigger", Vec::new())
                .expect("DoEnergy returns"),
            Value::Bool(true)
        );
        assert_eq!(first_calls.load(std::sync::atomic::Ordering::SeqCst), 1);
        assert_eq!(
            second_calls.load(std::sync::atomic::Ordering::SeqCst),
            0,
            "DoDamage must read the later node's live dead state"
        );
    }

    #[test]
    fn do_energy_damage_walk_follows_live_next_after_callback_additions() {
        // C4Effect::DoDamage advances through the current node's live pNext
        // only after Fx*Damage returns. An effect inserted after that node is
        // therefore visited in this chain; one inserted before it is not
        // (oracle-src-pinned src/C4Effect.cpp:427-436,59-93).
        let definition = test_definition("TARG", "Target", r#"#strict
        local order;
        public func Setup()
        {
            AddEffect("First", this, 10, 0, this);
            AddEffect("Second", this, 30, 0, this);
        }
        public func Trigger()
        {
            order = 0;
            DoEnergy(-1, this, true);
            return order;
        }
        protected func FxFirstDamage(target, number, change, cause, caused_by)
        {
            order = order * 10 + 1;
            AddEffect("Before", this, 5, 0, this);
            AddEffect("Between", this, 20, 0, this);
            return change;
        }
        protected func FxBeforeDamage(target, number, change, cause, caused_by)
        {
            order = order * 10 + 9;
            return change;
        }
        protected func FxBetweenDamage(target, number, change, cause, caused_by)
        {
            order = order * 10 + 2;
            return change;
        }
        protected func FxSecondDamage(target, number, change, cause, caused_by)
        {
            order = order * 10 + 3;
            return change;
        }
        "#);

        let mut engine = crate::Engine::with_seed(0);
        engine.register_test_definition(definition);
        let target = engine.spawn_test_object(SpawnConfig::new("TARG").with_alive(true));
        let target_index = engine.find_object_index(target).test_value();
        engine
            .call_object_function(target_index, "Setup", Vec::new()).test_value();

        assert_eq!(
            engine
                .call_object_function(target_index, "Trigger", Vec::new())
                .expect("DoEnergy returns"),
            Value::Int(123),
            "the walk visits the new successor before the old successor without restarting at the new head"
        );
    }

    #[test]
    fn kill_remove_death_revival_and_forcing_match_cpp_order_exactly_once() {
        // FnKill enters AssignDeath inline; RemoveDeath may revive and deny its
        // effect removal, while forced death continues through Death and final OCF
        // (oracle-src-pinned src/C4Script.cpp:335-345;
        // src/C4Object.cpp:1164-1205; src/C4Effect.cpp:407-425;
        // src/C4Object.h:361).
        fn run(forced: bool) -> (Value, bool, String, usize, usize, usize, Value) {
            let mut definition = test_definition("CLNK", "Clonk", r#"#strict
            local order, stop_alive, stop_ocf_alive;
            public func Trigger(forced)
            {
                order = 1;
                AddEffect("Guard", this, 1, 0, this);
                Kill(this, forced);
                order = order * 10 + 3;
                return order;
            }
            protected func FxGuardStop(target, number, reason)
            {
                stop_alive = GetAlive(target);
                stop_ocf_alive = GetOCF(target) & OCF_Alive;
                if (reason == 4)
                {
                    SetAlive(true, target);
                    return -1;
                }
                return 0;
            }
            protected func Death()
            {
                order = order * 10 + 2;
                return 0;
            }
            public func StopProbe()
            {
                if (stop_alive) return -1;
                return stop_ocf_alive;
            }
            "#);
            definition.set_category(crate::CATEGORY_OBJECT | crate::CATEGORY_LIVING);
            definition.configure_actions(
                Some("Idle".to_string()),
                HashMap::from([
                    ("Idle".to_string(), crate::ActionSpec::default()),
                    ("Dead".to_string(), crate::ActionSpec::default()),
                ]),
            );

            let stop_calls = Arc::new(std::sync::atomic::AtomicUsize::new(0));
            let death_calls = Arc::new(std::sync::atomic::AtomicUsize::new(0));
            let observed_stops = Arc::clone(&stop_calls);
            let observed_deaths = Arc::clone(&death_calls);
            definition.set_debugger_hooks(clonk_script::DebuggerHooks::new().with_on_call(
                move |name, _| match name {
                    "FxGuardStop" => {
                        observed_stops.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
                    }
                    "Death" => {
                        observed_deaths.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
                    }
                    _ => {}
                },
            ));

            let mut engine = crate::Engine::with_seed(0);
            engine.register_test_definition(definition);
            let clonk = engine.spawn_test_object(SpawnConfig::new("CLNK"));
            let index = engine.find_object_index(clonk).test_value();
            engine.objects[index].state.alive = true;
            engine.refresh_object_ocf(index);

            let result = engine
                .call_object_function(index, "Trigger", vec![Value::Bool(forced)]).test_value();
            let stop_probe = engine
                .call_object_function(index, "StopProbe", Vec::new()).test_value();
            let object = &engine.objects[index];
            (
                result,
                object.state.alive,
                object.state.action.name.clone(),
                object.state.effects.len(),
                stop_calls.load(std::sync::atomic::Ordering::SeqCst),
                death_calls.load(std::sync::atomic::Ordering::SeqCst),
                stop_probe,
            )
        }

        let revived = run(false);
        assert_eq!(revived.0, Value::Int(13));
        assert!(
            revived.1,
            "RemoveDeath SetAlive revival aborts ordinary Kill"
        );
        assert_eq!(revived.2, "Idle");
        assert_eq!(revived.3, 1, "Stop=-1 restores the effect node");
        assert_eq!(revived.4, 1, "RemoveDeath Stop runs exactly once");
        assert_eq!(revived.5, 0, "an accepted revival suppresses Death");
        assert_eq!(
            revived.6,
            Value::Int(crate::ocf::ALIVE as i32),
            "AssignDeath's raw Alive=false is visible while cached OCF stays stale during Stop"
        );

        let forced = run(true);
        assert_eq!(
            forced.0,
            Value::Int(123),
            "forced Kill runs Death before the invoking script resumes"
        );
        assert!(
            !forced.1,
            "forced Kill overrides the Stop callback's revival"
        );
        assert_eq!(forced.2, "Dead");
        assert_eq!(forced.3, 1, "forced death still honors Stop=-1");
        assert_eq!(forced.4, 1, "forced RemoveDeath Stop runs exactly once");
        assert_eq!(forced.5, 1, "forced Kill calls Death exactly once");
        assert_eq!(
            forced.6,
            Value::Int(crate::ocf::ALIVE as i32),
            "forced death uses the same raw-Alive/cached-OCF ordering"
        );
    }

    #[test]
    fn assign_death_clear_all_dispatches_stop_from_each_current_live_node() {
        // C4Effect::ClearAll freezes the recursive node order, but reads the
        // lower node's current pFnStop only after the higher callback returns.
        // ChangeEffect therefore changes the later Stop callback
        // (oracle-src-pinned src/C4Effect.cpp:407-425;
        // src/C4Script.cpp:5516-5543).
        let high_calls = Arc::new(std::sync::atomic::AtomicUsize::new(0));
        let old_low_calls = Arc::new(std::sync::atomic::AtomicUsize::new(0));
        let changed_low_calls = Arc::new(std::sync::atomic::AtomicUsize::new(0));
        let observed_high = Arc::clone(&high_calls);
        let observed_old_low = Arc::clone(&old_low_calls);
        let observed_changed_low = Arc::clone(&changed_low_calls);
        let mut definition = test_definition("TARG", "Target", r#"#strict
        public func Trigger()
        {
            AddEffect("Low", this, 10, 0, this);
            AddEffect("High", this, 20, 0, this);
            return Kill(this, true);
        }
        protected func FxHighStop(target, number, reason)
        {
            if (reason == 4) ChangeEffect("Low", target, 0, "Changed", -1);
            return 0;
        }
        protected func FxLowStop(target, number, reason)
        {
            return 0;
        }
        protected func FxChangedStop(target, number, reason)
        {
            return 0;
        }
        "#);
        definition.set_debugger_hooks(clonk_script::DebuggerHooks::new().with_on_call(
            move |name, _| match name {
                "FxHighStop" => {
                    observed_high.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
                }
                "FxLowStop" => {
                    observed_old_low.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
                }
                "FxChangedStop" => {
                    observed_changed_low.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
                }
                _ => {}
            },
        ));

        let mut engine = crate::Engine::with_seed(0);
        engine.register_test_definition(definition);
        let target = engine.spawn_test_object(SpawnConfig::new("TARG").with_alive(true));
        let target_index = engine.find_object_index(target).test_value();

        assert_eq!(
            engine
                .call_object_function(target_index, "Trigger", Vec::new())
                .expect("forced Kill returns"),
            Value::Bool(true)
        );
        assert_eq!(high_calls.load(std::sync::atomic::Ordering::SeqCst), 1);
        assert_eq!(
            old_low_calls.load(std::sync::atomic::Ordering::SeqCst),
            0,
            "ClearAll must not dispatch the lower node's stale callback"
        );
        assert_eq!(
            changed_low_calls.load(std::sync::atomic::Ordering::SeqCst),
            1,
            "ClearAll must dispatch the callback installed before that node unwinds"
        );
    }

    #[test]
    fn assign_death_keeps_stop_ocf_stale_but_refreshes_dead_action_callbacks() {
        // AssignDeath's raw Alive write stays invisible to cached OCF until
        // SetAction performs SetOCF, which also makes a dead living object
        // non-inflammable (oracle-src-pinned src/C4Object.cpp:564-568,
        // 1164-1205, 4165-4169).
        let mut definition = test_definition("DCOF", "Death OCF", r#"#strict
        local stop_alive, stop_ocf_alive, stop_ocf_crew, stop_ocf_inflammable;
        local start_ocf_alive, start_ocf_crew, start_ocf_inflammable;
        local abort_ocf_alive, abort_ocf_crew, abort_ocf_inflammable;
        local death_ocf_alive, death_ocf_crew, death_ocf_inflammable;
        public func Trigger()
        {
            AddEffect("Probe", this, 1, 0, this);
            Kill(this, true);
            return 1;
        }
        protected func FxProbeStop()
        {
            stop_alive = GetAlive();
            stop_ocf_alive = GetOCF() & OCF_Alive;
            stop_ocf_crew = GetOCF() & OCF_CrewMember;
            stop_ocf_inflammable = GetOCF() & OCF_Inflammable;
            return 0;
        }
        protected func DeadStart()
        {
            start_ocf_alive = GetOCF() & OCF_Alive;
            start_ocf_crew = GetOCF() & OCF_CrewMember;
            start_ocf_inflammable = GetOCF() & OCF_Inflammable;
        }
        protected func WalkAbort()
        {
            abort_ocf_alive = GetOCF() & OCF_Alive;
            abort_ocf_crew = GetOCF() & OCF_CrewMember;
            abort_ocf_inflammable = GetOCF() & OCF_Inflammable;
        }
        protected func Death()
        {
            death_ocf_alive = GetOCF() & OCF_Alive;
            death_ocf_crew = GetOCF() & OCF_CrewMember;
            death_ocf_inflammable = GetOCF() & OCF_Inflammable;
        }
        "#);
        definition.set_category(crate::CATEGORY_OBJECT | crate::CATEGORY_LIVING);
        definition.set_crew_member(true);
        definition.set_fire_properties(1, false, false);
        definition.configure_actions(
            Some("Walk".to_string()),
            HashMap::from([
                (
                    "Walk".to_string(),
                    crate::ActionSpec::default().with_abort_call("WalkAbort"),
                ),
                (
                    "Dead".to_string(),
                    crate::ActionSpec::default().with_start_call("DeadStart"),
                ),
            ]),
        );

        let mut engine = crate::Engine::with_seed(0);
        engine.register_test_definition(definition);
        let object_id = engine.spawn_test_object(SpawnConfig::new("DCOF")
            .with_alive(true)
            .with_action(ActionState::new("Walk")));
        let index = engine
            .find_object_index(object_id).test_value();
        engine.refresh_object_ocf(index);

        engine
            .call_object_function(index, "Trigger", Vec::new()).test_value();

        let locals = &engine.objects[index].state.local_vars;
        assert_eq!(locals.get("stop_alive"), Some(&Value::Bool(false)));
        assert_eq!(
            locals.get("stop_ocf_alive"),
            Some(&Value::Int(crate::ocf::ALIVE as i32)),
            "the initial raw Alive write leaves OCF stale during RemoveDeath"
        );
        assert_eq!(
            locals.get("stop_ocf_crew"),
            Some(&Value::Int(crate::ocf::CREW_MEMBER as i32))
        );
        assert_eq!(
            locals.get("stop_ocf_inflammable"),
            Some(&Value::Int(crate::ocf::INFLAMMABLE as i32)),
            "the initial raw Alive write leaves OCF_Inflammable stale during RemoveDeath"
        );
        for name in [
            "start_ocf_alive",
            "start_ocf_crew",
            "start_ocf_inflammable",
            "abort_ocf_alive",
            "abort_ocf_crew",
            "abort_ocf_inflammable",
            "death_ocf_alive",
            "death_ocf_crew",
            "death_ocf_inflammable",
        ] {
            assert_eq!(
                locals.get(name),
                Some(&Value::Int(0)),
                "{name} observes SetAction's explicit SetOCF"
            );
        }
        assert_eq!(
            engine.objects[index].state.ocf & crate::ocf::INFLAMMABLE,
            0,
            "AssignDeath's final SetOCF keeps a dead living object non-inflammable"
        );
    }

    #[test]
    fn set_category_after_kill_refreshes_and_persists_the_later_ocf() {
        // C4Object::SetCategory assigns Category, calls Resort, and then
        // SetOCF. When it follows synchronous AssignDeath, this later cache
        // must win both immediately and at copy-out (oracle-src-pinned
        // src/C4Object.h:311; src/C4Object.cpp:564-568,602-624,
        // 1164-1205).
        let mut definition = test_definition("DCTG", "Death category", r#"#strict
        local before_category_ocf, after_category_ocf;
        public func Trigger()
        {
            Kill(this, true);
            before_category_ocf = GetOCF();
            SetCategory(C4D_Object);
            after_category_ocf = GetOCF();
            return true;
        }
        "#);
        definition.set_category(crate::CATEGORY_OBJECT | crate::CATEGORY_LIVING);
        definition.set_fire_properties(1, false, false);
        definition.configure_actions(
            Some("Idle".to_string()),
            HashMap::from([
                ("Idle".to_string(), crate::ActionSpec::default()),
                ("Dead".to_string(), crate::ActionSpec::default()),
            ]),
        );

        let mut engine = crate::Engine::with_seed(0);
        engine.register_test_definition(definition);
        let target = engine.spawn_test_object(SpawnConfig::new("DCTG").with_alive(true));
        let index = engine
            .find_object_index(target).test_value();
        engine.refresh_object_ocf(index);

        assert_eq!(
            engine
                .call_object_function(index, "Trigger", Vec::new())
                .expect("death-category trigger succeeds"),
            Value::Bool(true)
        );

        let state = &engine.objects[index].state;
        let before = state
            .local_vars
            .get("before_category_ocf")
            .and_then(Value::as_c4_int).test_value() as u32;
        let after = state
            .local_vars
            .get("after_category_ocf")
            .and_then(Value::as_c4_int).test_value() as u32;
        assert_ne!(before & crate::ocf::LIVING, 0);
        assert_eq!(before & crate::ocf::INFLAMMABLE, 0);
        assert_eq!(after & crate::ocf::LIVING, 0);
        assert_ne!(after & crate::ocf::INFLAMMABLE, 0);
        assert_eq!(state.category, crate::CATEGORY_OBJECT);
        assert_eq!(
            state.ocf, after,
            "AssignDeath's earlier final-cache transport must not overwrite SetCategory's later SetOCF"
        );
    }

    #[test]
    fn kill_foreign_target_completes_death_before_caller_resumes_exactly_once() {
        // FnKill calls a foreign target's AssignDeath before returning, so Death's
        // writes and Alive=false are immediately visible to the caller
        // (oracle-src-pinned src/C4Script.cpp:335-345;
        // src/C4Object.cpp:1164-1205).
        let death_calls = Arc::new(std::sync::atomic::AtomicUsize::new(0));
        let observed_deaths = Arc::clone(&death_calls);
        let mut target_definition = test_definition("TARG", "Target", r#"#strict
        local death_count;
        protected func Death()
        {
            death_count++;
            return 0;
        }
        public func DeathCount()
        {
            return death_count;
        }
        "#);
        target_definition.configure_actions(
            Some("Idle".to_string()),
            HashMap::from([
                ("Idle".to_string(), crate::ActionSpec::default()),
                ("Dead".to_string(), crate::ActionSpec::default()),
            ]),
        );
        target_definition.set_debugger_hooks(clonk_script::DebuggerHooks::new().with_on_call(
            move |name, _| {
                if name == "Death" {
                    observed_deaths.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
                }
            },
        ));
        let caller_definition = test_definition("CALL", "Caller", r#"#strict
        public func Trigger(target)
        {
            if (!Kill(target)) return -1;
            if (GetAlive(target)) return -2;
            return target->DeathCount();
        }
        "#);

        let mut engine = crate::Engine::with_seed(0);
        engine.register_test_definition(target_definition);
        engine.register_test_definition(caller_definition);
        let caller = engine.spawn_test_object(SpawnConfig::new("CALL"));
        let target = engine.spawn_test_object(SpawnConfig::new("TARG"));
        let caller_index = engine
            .find_object_index(caller).test_value();
        let target_index = engine
            .find_object_index(target).test_value();
        engine.objects[target_index].state.alive = true;

        assert_eq!(
            engine
                .call_object_function(
                    caller_index,
                    "Trigger",
                    vec![object_reference_value(target)],
                )
                .expect("foreign Kill succeeds"),
            Value::Int(1),
            "Death and the raw Alive write are visible before Kill returns"
        );
        assert!(
            !engine.objects[target_index].state.alive,
            "foreign target's final death state folds once"
        );
        assert_eq!(
            death_calls.load(std::sync::atomic::Ordering::SeqCst),
            1,
            "foreign target Death runs exactly once"
        );
    }
