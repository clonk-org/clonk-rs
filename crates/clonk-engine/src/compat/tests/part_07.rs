// Contiguous slice 7 of 11 of the `compat::tests` battery, spliced by
// `include!` from compat.rs so every test id stays `compat::tests::*`.
// Mostly: object state, world, effects.

    #[test]
    fn set_wind_clamps_to_bounds() {
        let (result, delta) = with_environment_context(EnvironmentSettings::new(0), 0, || {
            set_wind(&[Value::Int(150)])?;
            get_wind(&[])
        });

        let value = result.expect("SetWind/GetWind succeeds");
        assert_eq!(value, Value::Int(100));
        assert_eq!(delta.wind, Some(100));
    }

    #[test]
    fn set_temperature_updates_context() {
        let (result, delta) = with_environment_context(EnvironmentSettings::new(0), 42, || {
            set_temperature(&[Value::Int(-30)])?;
            get_temperature(&[])
        });

        let value = result.expect("SetTemperature/GetTemperature succeeds");
        assert_eq!(value, Value::Int(-30));
        assert_eq!(delta.temperature, Some(-30));
    }

    #[test]
    fn set_climate_clamps_and_updates() {
        let (result, delta) = with_environment_context(EnvironmentSettings::new(0), 0, || {
            set_climate(&[Value::Int(-80)])?;
            get_climate(&[])
        });

        let value = result.expect("SetClimate/GetClimate succeeds");
        assert_eq!(value, Value::Int(-50));
        assert_eq!(delta.climate, Some(-50));
    }

    #[test]
    fn set_season_clamps_and_updates() {
        // FnSetSeason/FnGetSeason (C4Script.cpp:3025-3033, registered
        // :6894-6895) -> C4Weather::SetSeason/GetSeason: BoundBy(iSeason,
        // 0, 100) (C4Weather.cpp:229-238).
        let (result, delta) = with_environment_context(EnvironmentSettings::new(0), 0, || {
            set_season(&[Value::Int(120)])?;
            get_season(&[])
        });

        let value = result.expect("SetSeason/GetSeason succeeds");
        assert_eq!(value, Value::Int(100));
        assert_eq!(delta.season, Some(100));
    }

    #[test]
    fn merged_environment_delta_forwards_nested_season_and_gamma_handling() {
        // Nested/script host outcomes are folded into the outer callback's
        // EnvironmentDelta. SetSeason must survive that merge together with
        // the marker saying its in-order GammaRamp was already queued.
        let mut target = EnvironmentDelta::default();
        let source = EnvironmentDelta {
            season: Some(37),
            season_gamma_handled: true,
            ..EnvironmentDelta::default()
        };

        crate::merge_environment_delta(&mut target, &source);

        assert_eq!(target.season, Some(37));
        assert!(target.season_gamma_handled);
    }

    #[test]
    fn random_requires_context_for_positive_ranges() {
        let error = random(&[Value::Int(5)]).expect_err("Random without context fails");
        assert_eq!(error.message(), "Random: host context unavailable");
    }

    #[test]
    fn async_random_is_bounded_and_leaves_synced_rng_untouched() {
        // FnAsyncRandom uses SafeRandom rather than the synchronized Random
        // ledger (C4Script.cpp:3367-3370, C4Random.h:40-75).
        SCRIPT_SAFE_RNG.with(|rng| {
            *rng.borrow_mut() = crate::particles::SafeRng::new(7);
        });
        let mut script = ScriptEngine::new();
        register_host_functions(&mut script);
        script
            .load_script("#strict 2\nfunc Probe() { return [AsyncRandom(10), AsyncRandom(0)]; }")
            .expect("AsyncRandom probe compiles");

        let initial_rng = LcgRng::new(41);
        let guard = enter_random_context(initial_rng.clone());
        let result = script.call("Probe", &[]).expect("AsyncRandom succeeds");
        let final_rng = guard.finish();

        let Value::Array(values) = result else {
            panic!("AsyncRandom probe must return an array");
        };
        assert!(matches!(values[0], Value::Int(value) if (0..10).contains(&value)));
        assert_eq!(values[1], Value::Int(0));
        assert_eq!(final_rng.count, initial_rng.count);
        assert_eq!(final_rng.hold, initial_rng.hold);
    }

    #[test]
    fn async_random_per_frame_keeps_headless_sync_counts_seed_independent(
    ) -> Result<(), crate::EngineError> {
        const SCRIPT: &str = r#"
            global func Step(state, frame, random)
            {
                AsyncRandom(32768);
                return 0;
            }
        "#;

        fn replay(safe_seed: u32) -> Result<Vec<i32>, crate::EngineError> {
            let initial_safe_rng = crate::particles::SafeRng::new(safe_seed);
            SCRIPT_SAFE_RNG.with(|rng| {
                *rng.borrow_mut() = initial_safe_rng.clone();
            });
            let mut engine = crate::Engine::with_seed(43);
            engine.install_scenario_script("AsyncRandomReplay", SCRIPT)?;

            let mut random_counts = Vec::new();
            for _ in 0..16 {
                engine.tick_without_snapshot()?;
                random_counts.push(engine.sync_check(0).random_count);
            }
            SCRIPT_SAFE_RNG.with(|rng| {
                assert_ne!(
                    *rng.borrow(),
                    initial_safe_rng,
                    "the per-frame script must advance the unsynced stream"
                );
            });
            Ok(random_counts)
        }

        let first = replay(1)?;
        let second = replay(2)?;
        assert_eq!(first, second, "SafeRandom must not affect RandomCount");
        Ok(())
    }

    #[test]
    fn random_edge_ranges_follow_the_cpp_ledger() {
        // C4Random.h:40-61: RandomCount++ is UNCONDITIONAL; range 0
        // returns 0 without advancing the hold; nil converts to 0 at the
        // host boundary (C4AulExec.cpp:1364-1396); a negative range goes
        // through the unsigned modulo (usual arithmetic conversions), so
        // the hold DOES advance.
        let guard = enter_random_context(LcgRng::new(0));
        let zero = random(&[Value::Int(0)]).expect("zero range succeeds");
        assert_eq!(zero, Value::Int(0));
        let nil = random(&[Value::Nil]).expect("nil converts to 0");
        assert_eq!(nil, Value::Int(0));
        let missing = random(&[]).expect("missing argument converts to 0");
        assert_eq!(missing, Value::Int(0));
        let negative = random(&[Value::Int(-3)]).expect("negative range succeeds");
        let rng = guard.finish();
        // Three zero-ish draws (count++ only) plus one negative draw that
        // advances the hold like C++'s unsigned modulo.
        assert_eq!(rng.count, 4, "RandomCount++ is unconditional");
        let mut reference = LcgRng::new(0);
        reference.random(0);
        reference.random(0);
        reference.random(0);
        assert_eq!(Value::Int(reference.random(-3)), negative);
        assert_eq!(rng.hold, reference.hold, "negative ranges advance the hold");
    }

    proptest! {
        #[test]
        fn set_wind_clamps_across_range(raw in any::<i32>()) {
            let (result, delta) = with_environment_context(EnvironmentSettings::new(0), 0, || {
                set_wind(&[Value::Int(raw)])?;
                get_wind(&[])
            });

            let expected = raw.clamp(-100, 100);
            prop_assert!(matches!(result, Ok(Value::Int(value)) if value == expected));
            prop_assert_eq!(delta.wind, Some(expected));
            prop_assert!(delta.temperature.is_none());
            prop_assert!(delta.climate.is_none());
        }

        #[test]
        fn set_temperature_clamps_across_range(raw in any::<i32>()) {
            let (result, delta) = with_environment_context(EnvironmentSettings::new(0), 0, || {
                set_temperature(&[Value::Int(raw)])?;
                get_temperature(&[])
            });

            let expected = raw.clamp(-100, 100);
            prop_assert!(matches!(result, Ok(Value::Int(value)) if value == expected));
            prop_assert_eq!(delta.temperature, Some(expected));
            prop_assert!(delta.wind.is_none());
            prop_assert!(delta.climate.is_none());
        }

        #[test]
        fn set_climate_clamps_across_range(raw in any::<i32>()) {
            let (result, delta) = with_environment_context(EnvironmentSettings::new(0), 0, || {
                set_climate(&[Value::Int(raw)])?;
                get_climate(&[])
            });

            let expected = raw.clamp(-50, 50);
            prop_assert!(matches!(result, Ok(Value::Int(value)) if value == expected));
            prop_assert_eq!(delta.climate, Some(expected));
            prop_assert!(delta.wind.is_none());
            prop_assert!(delta.temperature.is_none());
        }

        #[test]
        fn random_matches_cpp_lcg(seed in any::<u64>(), range in 1i32..=1024) {
            let mut expected_rng = LcgRng::new(seed as u32);
            let expected = expected_rng.random(range);

            let guard = enter_random_context(LcgRng::new(seed as u32));
            let value = random(&[Value::Int(range)]).expect("Random with context succeeds");
            let _ = guard.finish();

            prop_assert_eq!(value, Value::Int(expected));
            prop_assert!(expected >= 0 && expected < range);
        }

        #[test]
        fn random_sequence_remains_deterministic(seed in any::<u64>()) {
            let mut expected_rng = LcgRng::new(seed as u32);
            let expected = [
                expected_rng.random(100),
                expected_rng.random(100),
                expected_rng.random(100),
            ];

            let guard = enter_random_context(LcgRng::new(seed as u32));
            let first = random(&[Value::Int(100)]).expect("first draw succeeds");
            let second = random(&[Value::Int(100)]).expect("second draw succeeds");
            let third = random(&[Value::Int(100)]).expect("third draw succeeds");
            let _ = guard.finish();

            prop_assert_eq!(first, Value::Int(expected[0]));
            prop_assert_eq!(second, Value::Int(expected[1]));
            prop_assert_eq!(third, Value::Int(expected[2]));
        }
    }

    #[test]
    fn add_effect_keeps_constructor_values_out_of_effect_vars() {
        let state = empty_state();
        let (result, outcome) = with_object_host_context(|| {
            add_effect(&[
                Value::String("Glow".into()),
                state.clone(),
                Value::Int(120),
                Value::Int(2),
                Value::Nil,
                Value::Nil,
                Value::Int(7),
                Value::Bool(true),
            ])
        });

        let value = result.expect("AddEffect succeeds");
        assert_eq!(value, Value::Int(1));
        assert_eq!(outcome.object.len(), 1);
        match &outcome.object[0] {
            EffectCommand::Add {
                effect,
                constructor_values,
            } => {
                assert!(effect.vars().is_empty());
                assert_eq!(
                    constructor_values.as_ref(),
                    Some(&[Value::Int(7), Value::Bool(true), Value::Nil, Value::Nil,])
                );
            }
            other => panic!("unexpected command: {other:?}"),
        }
    }

    #[test]
    fn add_effect_preserves_interior_nil_rvals_without_shifting() {
        let state = empty_state();
        let (result, outcome) = with_object_host_context(|| {
            add_effect(&[
                Value::String("Glow".into()),
                state,
                Value::Int(120),
                Value::Int(2),
                Value::Nil,
                Value::Nil,
                Value::Nil,
                Value::Int(42),
            ])
        });

        assert_eq!(result.expect("AddEffect succeeds"), Value::Int(1));
        let EffectCommand::Add {
            effect,
            constructor_values,
        } = &outcome.object[0]
        else {
            panic!("expected an Add command");
        };
        assert!(effect.vars().is_empty());
        assert_eq!(
            constructor_values.as_ref(),
            Some(&[Value::Nil, Value::Int(42), Value::Nil, Value::Nil]),
            "slot six is rVal1 even when nil; rVal2 cannot slide left"
        );
    }

    #[test]
    fn remove_effect_rejects_when_missing() {
        let state = empty_state();
        let (result, _) =
            with_object_host_context(|| remove_effect(&[Value::Nil, state.clone(), Value::Int(0)]));
        let value = result.expect("RemoveEffect succeeds");
        assert_eq!(value, Value::Bool(false));
    }

    #[test]
    fn add_and_remove_effect_flow() {
        let state = empty_state();
        let (result, outcome) = with_object_host_context(|| -> Result<Value, RuntimeError> {
            add_effect(&[Value::String("Glow".into()), state.clone(), Value::Int(100)])?;
            remove_effect(&[Value::String("Glow".into()), state.clone()])
        });

        let value = result.expect("calls succeed");
        assert_eq!(value, Value::Bool(true));
        assert_eq!(outcome.object.len(), 2);
        assert!(matches!(outcome.object[0], EffectCommand::Add { .. }));
        assert!(matches!(
            outcome.object[1],
            EffectCommand::RemoveNumber {
                number: 1,
                no_callbacks: false
            }
        ));
    }

    #[test]
    fn remove_effect_can_skip_callbacks() {
        let state = empty_state();
        let (result, outcome) = with_object_host_context(|| -> Result<Value, RuntimeError> {
            add_effect(&[Value::String("Glow".into()), state.clone(), Value::Int(100)])?;
            remove_effect(&[
                Value::String("Glow".into()),
                state.clone(),
                Value::Int(0),
                Value::Bool(true),
            ])
        });

        let value = result.expect("calls succeed");
        assert_eq!(value, Value::Bool(true));
        assert_eq!(outcome.object.len(), 2);
        assert!(matches!(outcome.object[0], EffectCommand::Add { .. }));
        assert!(matches!(
            outcome.object[1],
            EffectCommand::RemoveNumber {
                number: 1,
                no_callbacks: true
            }
        ));
    }

    #[test]
    fn change_effect_renames_retimes_and_supports_number_lookup() {
        let mut fade = EffectState::new("IntFade")
            .with_priority(100)
            .with_interval(7)
            .with_timer(4);
        fade.number = 11;
        let mut keep = EffectState::new("Keep")
            .with_priority(-120)
            .with_interval(9)
            .with_timer(6);
        keep.number = 12;
        let mut omitted = EffectState::new("Omitted")
            .with_priority(80)
            .with_interval(5)
            .with_timer(3);
        omitted.number = 13;
        let mut long = EffectState::new("Long")
            .with_priority(70)
            .with_interval(4)
            .with_timer(2);
        long.number = 14;
        let effects = [fade, keep, omitted, long];

        let mut script = ScriptEngine::new();
        register_host_functions(&mut script);
        script
            .load_script(
                r#"#strict 2
func Probe(state) {
  var unset;
  var renamed = ChangeEffect("Int*", this(), 0, "IntFadeOut", 10);
  var preserved = ChangeEffect("Keep", this(), 0, "KeepOut", -1);
  var empty_rejected = ChangeEffect("KeepOut", this(), 0, "", 1);
  var nil_rejected = ChangeEffect("KeepOut", this(), 0, unset, 1);
  var missing_rejected = ChangeEffect("Missing", this(), 0, "StillMissing", 1);
  var by_number = ChangeEffect(unset, this(), 12, "ByNumber", -7);
  var omitted_timer = ChangeEffect("Omitted", this(), 0, "Reset");
  var clamped = ChangeEffect(unset, this(), 14, "abcdefghijklmnopqrstuvwxyz1234567890", -1);
  return [
    renamed,
    GetEffect("IntFadeOut", this(), 0, 3),
    GetEffect("IntFadeOut", this(), 0, 6),
    preserved,
    empty_rejected,
    nil_rejected,
    missing_rejected,
    by_number,
    GetEffect(unset, this(), 12, 1),
    GetEffect(unset, this(), 12, 3),
    GetEffect(unset, this(), 12, 6),
    omitted_timer,
    GetEffect(unset, this(), 13, 3),
    GetEffect(unset, this(), 13, 6),
    clamped,
    GetEffect(unset, this(), 14, 1)
  ];
}
"#,
            )
            .expect("ChangeEffect probe compiles");

        let state = empty_state();
        let (result, outcome) = with_effect_context(
            Some(HostObjectContext {
                effects: &effects,
                ..idle_object_context()
            }),
            &[],
            HostWorldContext::default(),
            1,
            || {
                script
                    .call_with_locals_and_this(
                        "Probe",
                        &[state],
                        &HashMap::new(),
                        object_reference_value(ObjectId::new(1)),
                    )
                    .map(|(value, _)| value)
                    .map_err(|error| RuntimeError::new(error.to_string()))
            },
        );

        assert_eq!(
            result.expect("ChangeEffect probe runs"),
            Value::Array(vec![
                Value::Bool(true),
                Value::Int(10),
                Value::Int(0),
                Value::Bool(true),
                Value::Bool(false),
                Value::Bool(false),
                Value::Bool(false),
                Value::Bool(true),
                Value::String("ByNumber".into()),
                Value::Int(9),
                Value::Int(6),
                Value::Bool(true),
                Value::Int(0),
                Value::Int(0),
                Value::Bool(true),
                Value::String("abcdefghijklmnopqrstuvwxyz1234".into()),
            ])
        );
        assert_eq!(
            outcome
                .object
                .iter()
                .filter(|command| matches!(command, EffectCommand::Update(_)))
                .count(),
            5,
            "only successful changes emit updates"
        );
    }

    #[test]
    fn set_action_respects_no_other_action() {
        let mut specs = HashMap::new();
        specs.insert(
            "Idle".to_string(),
            ActionSpec::default().with_no_other_action(true),
        );
        specs.insert("Walk".to_string(), ActionSpec::default());
        let library = ActionLibrary::new(Some("Idle".to_string()), specs);

        let (result, outcome) = with_effect_context(
            Some(HostObjectContext {
                action_library: library.clone().into(),
                ..idle_object_context()
            }),
            &[],
            HostWorldContext::default(),
            1,
            || set_action(&[Value::String("Walk".into())]),
        );

        let value = result.expect("SetAction returns bool");
        assert_eq!(value, Value::Bool(false));
        assert!(outcome.object_update.is_none());

        let (result, outcome) = with_effect_context(
            Some(HostObjectContext {
                id: ObjectId::new(2),
                action_library: library.into(),
                ..idle_object_context()
            }),
            &[],
            HostWorldContext::default(),
            1,
            || set_action(&[Value::String("Idle".into())]),
        );

        let value = result.expect("SetAction returns bool");
        assert_eq!(value, Value::Bool(true));
        let update = outcome.object_update.expect("pending update exists");
        let action = update.action.expect("action update recorded");
        assert_eq!(action.name.as_deref(), Some("Idle"));
        assert!(!action.force);
    }

    #[test]
    fn set_action_resolves_byte_equivalent_projected_names() {
        let mut specs = HashMap::new();
        specs.insert("Idle".to_string(), ActionSpec::default());
        specs.insert("\u{ff}".to_string(), ActionSpec::default());
        let library = ActionLibrary::new(Some("Idle".to_string()), specs);
        let projected = format!(
            "{}{}",
            clonk_script::c4_string_from_bytes(&[0xc3]),
            clonk_script::c4_string_from_bytes(&[0xbf])
        );
        let (result, outcome) = with_effect_context(
            Some(HostObjectContext {
                action_library: library.into(),
                ..idle_object_context()
            }),
            &[],
            HostWorldContext::default(),
            1,
            || set_action(&[Value::String(projected.into())]),
        );
        assert_eq!(result.expect("SetAction succeeds"), Value::Bool(true));
        assert_eq!(
            outcome
                .object_update
                .and_then(|update| update.action)
                .and_then(|action| action.name),
            Some("\u{ff}".into())
        );
    }

    #[test]
    fn action_data_setters_share_missing_material_fallback() {
        let mut specs = HashMap::new();
        specs.insert(
            "Idle".to_string(),
            ActionSpec::default().with_procedure("bridge"),
        );
        let library = ActionLibrary::new(Some("Idle".to_string()), specs);

        let (result, outcome) = with_effect_context(
            Some(HostObjectContext {
                action_library: library.into(),
                ..idle_object_context()
            }),
            &[],
            HostWorldContext::default(),
            1,
            || {
                Ok::<_, RuntimeError>(Value::Array(vec![
                    set_bridge_action_data(&[
                        Value::Int(0),
                        Value::Bool(false),
                        Value::Bool(false),
                        Value::Int(512),
                    ])?,
                    get_action_data(&[])?,
                    set_action_data(&[Value::Int(512)])?,
                    get_action_data(&[])?,
                ]))
            },
        );

        assert_eq!(
            result.expect("action-data setters return values"),
            Value::Array(vec![
                Value::Bool(true),
                Value::Int(255),
                Value::Bool(true),
                Value::Int(255),
            ])
        );
        let update = outcome.object_update.expect("object update recorded");
        let action = update.action.expect("action update present");
        assert_eq!(action.data, Some(255));
    }

    fn set_action_data_target_world(
        procedure: &str,
        status: ObjectStatus,
        initial_data: i32,
        materials: Option<Rc<MaterialSet>>,
    ) -> (ObjectId, HostWorldContext) {
        let target_id = ObjectId::new(2);
        let action_library = ActionLibrary::new(
            Some("Work".to_string()),
            HashMap::from([(
                "Work".to_string(),
                ActionSpec::default().with_procedure(procedure),
            )]),
        );
        let mut state = crate::preview_spawn_state(
            Vector2::ZERO,
            OWNER_NONE,
            OWNER_NONE,
            DEFAULT_CATEGORY,
            crate::FULL_CON,
            crate::CONTACT_DENSITY_SOLID,
            Vec::new(),
        );
        state.status = status;
        state.action = crate::ActionState::new("Work");
        state.action.data = initial_data;
        let target = fixture_world_object(target_id, "TARG")
            .with_status(status)
            .with_action_name("Work")
            .with_action_procedure(Some(procedure.to_string()))
            .with_action_data(initial_data)
        .with_full_state(Rc::new(state));
        let world = HostWorldContext::from_objects([target])
            .with_definition_metadata(Rc::new(HashMap::from([(
                DefinitionId::from("TARG"),
                DefinitionMetadata {
                    action_library: action_library.into(),
                    ..DefinitionMetadata::default()
                },
            )])))
            .with_materials(materials);
        (target_id, world)
    }

    fn foreign_action_data_update(outcome: &EffectContextOutcome, target: ObjectId) -> Option<i32> {
        outcome
            .other_objects
            .iter()
            .find(|object| object.object_id == target)?
            .update
            .as_ref()?
            .action
            .as_ref()?
            .data
    }

    #[test]
    fn set_action_data_foreign_attach_uses_the_targets_vertex_validation() {
        let valid = (29 << 8) | 29;
        for (data, expected_result, expected_read, expected_update) in [
            (valid, true, valid, Some(valid)),
            (30, false, 7, None),
            ((30 << 8) | 1, false, 7, None),
        ] {
            let (target_id, world) =
                set_action_data_target_world("attach", ObjectStatus::Normal, 7, None);
            let target = object_reference_value(target_id);
            let (result, outcome) = with_object_host_context_with_world(world, || {
                Ok(Value::Array(vec![
                    set_action_data(&[Value::Int(data), target.clone()])?,
                    get_action_data(&[target])?,
                ]))
            });

            assert_eq!(
                result.expect("foreign SetActionData runs"),
                Value::Array(vec![
                    Value::Bool(expected_result),
                    Value::Int(expected_read),
                ])
            );
            assert!(outcome.object_update.is_none(), "caller remains unchanged");
            assert_eq!(
                foreign_action_data_update(&outcome, target_id),
                expected_update
            );
        }
    }

    #[test]
    fn set_action_data_foreign_bridge_clamps_material_and_preserves_sentinel() {
        let library = clonk_resources::MaterialLibrary::parse(
            "[Material Sky]\nName=Sky\n\n[Material Earth]\nName=Earth\n",
        )
        .expect("material library parses");
        let materials = Rc::new(MaterialSet::from_resource_library(&library));
        let material_count = materials.len() as i32;

        for (case_materials, data, expected_material) in [
            (materials.clone(), material_count + 5, material_count - 1),
            (materials.clone(), -1, -1),
            (Rc::new(MaterialSet::new()), 0, -1),
        ] {
            let expected = encode_bridge_action_data(0, false, false, expected_material);
            let (target_id, world) = set_action_data_target_world(
                "bridge",
                ObjectStatus::Normal,
                0x1234_0301,
                Some(case_materials),
            );
            let target = object_reference_value(target_id);
            let (result, outcome) = with_effect_context(None, &[], world, 1, || {
                Ok::<_, RuntimeError>(Value::Array(vec![
                    set_action_data(&[Value::Int(data), target.clone()])?,
                    get_action_data(&[target])?,
                ]))
            });

            assert_eq!(
                result.expect("scenario-scope SetActionData runs"),
                Value::Array(vec![Value::Bool(true), Value::Int(expected)])
            );
            assert!(outcome.object_update.is_none());
            assert_eq!(
                foreign_action_data_update(&outcome, target_id),
                Some(expected),
                "SetActionData bridge conversion clears length and flags"
            );
        }
    }

    #[test]
    #[allow(clippy::arc_with_non_send_sync)] // HostWorldContext owns definition scripts in Arc; this fixture stays on one thread.
    fn set_bridge_action_data_accepts_the_fifth_object_parameter() {
        // FnSetBridgeActionData's pObj is parameter five and may differ from
        // cthr->Obj (C4Script.cpp:757-765). LOAM::StartBridge runs on LOAM,
        // first ObjectSetActions the CLNK, then writes bridge data to that
        // foreign CLNK (Loam.c4d/Script.c:82-96).
        let loam = ObjectId::new(1);
        let clonk = ObjectId::new(2);
        let library = ActionLibrary::new(
            Some("Walk".to_string()),
            HashMap::from([
                (
                    "Walk".to_string(),
                    ActionSpec::default().with_procedure("walk"),
                ),
                (
                    "Bridge".to_string(),
                    ActionSpec::default().with_procedure("bridge"),
                ),
            ]),
        );
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
        let target = fixture_world_object(clonk, "CLNK")
            .with_action_name("Walk")
            .with_action_procedure(Some("walk".into()))
        .with_full_state(Rc::new(state));
        let mut script = ScriptEngine::new();
        register_host_functions(&mut script);
        let materials = clonk_resources::MaterialLibrary::parse(
            "[Material Sky]\nName=Sky\n\n[Material Earth]\nName=Earth\n",
        )
        .expect("material library parses");
        let world = HostWorldContext::from_objects(vec![target])
            .with_definition_metadata(Rc::new(HashMap::from([(
                DefinitionId::from("CLNK"),
                DefinitionMetadata {
                    action_library: library.into(),
                    ..DefinitionMetadata::default()
                },
            )])))
            .with_definition_scripts(HashMap::from([(
                DefinitionId::from("CLNK"),
                Arc::new(script),
            )]))
            .with_materials(Some(Rc::new(MaterialSet::from_resource_library(
                &materials,
            ))));
        let (result, outcome) = with_effect_context(
            Some(HostObjectContext {
                id: loam,
                ..idle_object_context()
            }),
            &[],
            world,
            1,
            || {
                assert_eq!(
                    object_set_action(&[
                        object_reference_value(clonk),
                        Value::String("Bridge".into()),
                    ])?,
                    Value::Bool(true)
                );
                set_bridge_action_data(&[
                    Value::Int(100),
                    Value::Bool(true),
                    Value::Bool(false),
                    Value::Int(7),
                    object_reference_value(clonk),
                ])
            },
        );

        assert_eq!(
            result.expect("SetBridgeActionData succeeds"),
            Value::Bool(true)
        );
        let target = outcome
            .other_objects
            .iter()
            .find(|outcome| outcome.object_id == clonk)
            .expect("foreign CLNK outcome recorded");
        let action = target
            .update
            .as_ref()
            .expect("foreign update recorded")
            .action
            .as_ref()
            .expect("action update present");
        assert_eq!(action.name.as_deref(), Some("Bridge"));
        assert_eq!(
            action.data,
            Some(encode_bridge_action_data(100, true, false, 1)),
            "C4Action::SetBridgeData clamps material 7 to Num-1"
        );
    }

    #[test]
    fn set_action_data_rejects_invalid_attach_vertices() {
        let mut specs = HashMap::new();
        specs.insert(
            "Idle".to_string(),
            ActionSpec::default().with_procedure("attach"),
        );
        let library = ActionLibrary::new(Some("Idle".to_string()), specs);

        let (result, outcome) = with_effect_context(
            Some(HostObjectContext {
                action_library: library.into(),
                ..idle_object_context()
            }),
            &[],
            HostWorldContext::default(),
            1,
            || set_action_data(&[Value::Int(31 << 8)]),
        );

        let value = result.expect("SetActionData returns bool");
        assert_eq!(value, Value::Bool(false));
        assert!(outcome.object_update.is_none());
    }

    #[test]
    fn set_action_data_and_bridge_data_use_cpp_status_truthiness() {
        let library = clonk_resources::MaterialLibrary::parse(
            "[Material Sky]\nName=Sky\n\n[Material Earth]\nName=Earth\n",
        )
        .expect("material library parses");
        let materials = Rc::new(MaterialSet::from_resource_library(&library));

        for (status, expected, expected_update) in [
            (ObjectStatus::Inactive, true, Some(1)),
            (ObjectStatus::Deleted, false, None),
        ] {
            // C++ tests `!pObj->Status`: Deleted=0 is rejected, while
            // Inactive=2 remains a valid object (C4Object.h:39-41).
            let (target_id, world) =
                set_action_data_target_world("bridge", status, 0, Some(materials.clone()));
            let target = object_reference_value(target_id);
            let (result, outcome) = with_effect_context(None, &[], world, 1, || {
                Ok::<_, RuntimeError>(Value::Array(vec![
                    set_bridge_action_data(&[
                        Value::Int(0),
                        Value::Bool(false),
                        Value::Bool(false),
                        Value::Int(1),
                        target.clone(),
                    ])?,
                    set_action_data(&[Value::Int(1), target])?,
                ]))
            });

            assert_eq!(
                result.expect("action-data setters run"),
                Value::Array(vec![Value::Bool(expected), Value::Bool(expected)])
            );
            assert_eq!(
                foreign_action_data_update(&outcome, target_id),
                expected_update
            );
        }
    }

    #[test]
    fn get_action_data_returns_zero_by_default() {
        let (result, outcome) = with_object_host_context(|| get_action_data(&[]));
        let value = result.expect("GetActionData succeeds");
        assert_eq!(value, Value::Int(0));
        assert!(outcome.object_update.is_none());
    }

    #[test]
    fn get_action_data_reflects_pending_update() {
        let (result, outcome) = with_effect_context(
            Some(idle_object_context()),
            &[],
            HostWorldContext::default(),
            1,
            || {
                set_action_data(&[Value::Int(42)])?;
                get_action_data(&[])
            },
        );

        let value = result.expect("GetActionData succeeds");
        assert_eq!(value, Value::Int(42));
        let update = outcome.object_update.expect("action update recorded");
        let action = update.action.expect("action update exists");
        assert_eq!(action.data, Some(42));
    }

    #[test]
    fn get_action_data_reads_world_context() {
        let other = fixture_world_object(ObjectId::new(23), "Dummy")
            .with_action_name("Walk")
            .with_action_data(77);
        let world = HostWorldContext::from_objects(vec![other]);
        let (result, _) = with_effect_context(None, &[], world, 1, || {
            let mut target = ValueMap::new();
            target.insert("id".into(), Value::Int(23));
            get_action_data(&[Value::Proplist(target.into_iter().collect())])
        });

        let value = result.expect("GetActionData succeeds");
        assert_eq!(value, Value::Int(77));
    }

    #[test]
    fn get_action_data_respects_target_filter() {
        let (result, _) = with_object_host_context(|| {
            let mut target = ValueMap::new();
            target.insert("id".into(), Value::Int(99));
            get_action_data(&[Value::Proplist(target.into_iter().collect())])
        });

        let value = result.expect("GetActionData succeeds");
        assert_eq!(value, Value::Nil);
    }

    #[test]
    fn get_action_data_returns_nil_without_context() {
        let value = get_action_data(&[]).expect("GetActionData succeeds without context");
        assert_eq!(value, Value::Nil);
    }

    #[test]
    fn get_action_returns_idle_by_default() {
        let (result, outcome) = with_object_host_context(|| get_action(&[]));
        let value = result.expect("GetAction succeeds");
        assert_eq!(value, Value::String("Idle".into()));
        assert!(outcome.object_update.is_none());
    }

    #[test]
    fn get_action_reflects_pending_update() {
        let mut specs = HashMap::new();
        specs.insert("Idle".to_string(), ActionSpec::default());
        specs.insert("Walk".to_string(), ActionSpec::default());
        let library = ActionLibrary::new(Some("Idle".to_string()), specs);

        let (result, outcome) = with_effect_context(
            Some(HostObjectContext {
                action_library: library.into(),
                ..idle_object_context()
            }),
            &[],
            HostWorldContext::default(),
            1,
            || {
                set_action(&[Value::String("Walk".into())])?;
                get_action(&[])
            },
        );

        let value = result.expect("SetAction/GetAction succeed");
        assert_eq!(value, Value::String("Walk".into()));
        let update = outcome.object_update.expect("action update recorded");
        let action = update.action.expect("action update exists");
        assert_eq!(action.name.as_deref(), Some("Walk"));
    }

    #[test]
    fn get_procedure_returns_nil_when_unspecified() {
        let mut specs = HashMap::new();
        specs.insert("Idle".to_string(), ActionSpec::default());
        let library = ActionLibrary::new(Some("Idle".to_string()), specs);

        let (result, _) = with_effect_context(
            Some(HostObjectContext {
                action_library: library.into(),
                ..idle_object_context()
            }),
            &[],
            HostWorldContext::default(),
            1,
            || get_procedure(&[]),
        );

        let value = result.expect("GetProcedure succeeds");
        assert_eq!(value, Value::Nil);
    }

    #[test]
    fn get_procedure_returns_configured_value() {
        let mut specs = HashMap::new();
        specs.insert(
            "Idle".to_string(),
            ActionSpec::default().with_procedure("walk"),
        );
        let library = ActionLibrary::new(Some("Idle".to_string()), specs);

        let (result, _) = with_effect_context(
            Some(HostObjectContext {
                action_library: library.into(),
                ..idle_object_context()
            }),
            &[],
            HostWorldContext::default(),
            1,
            || get_procedure(&[]),
        );

        let value = result.expect("GetProcedure succeeds");
        assert_eq!(value, Value::String("walk".into()));
    }

    #[test]
    fn get_procedure_reflects_pending_action_change() {
        let mut specs = HashMap::new();
        specs.insert(
            "Idle".to_string(),
            ActionSpec::default().with_procedure("walk"),
        );
        specs.insert(
            "Float".to_string(),
            ActionSpec::default().with_procedure("float"),
        );
        let library = ActionLibrary::new(Some("Idle".to_string()), specs);

        let (result, _) = with_effect_context(
            Some(HostObjectContext {
                action_library: library.into(),
                ..idle_object_context()
            }),
            &[],
            HostWorldContext::default(),
            1,
            || {
                set_action(&[Value::String("Float".into())])?;
                get_procedure(&[])
            },
        );

        let value = result.expect("SetAction/GetProcedure succeed");
        assert_eq!(value, Value::String("float".into()));
    }

    #[test]
    fn get_procedure_reads_world_context() {
        let world = HostWorldContext::from_objects(vec![fixture_world_object(
            ObjectId::new(42),
            "Dummy",
        )
            .with_action_name("Swim")
            .with_action_procedure(Some("swim".to_string()))]);
        let (result, _) = with_effect_context(None, &[], world, 1, || {
            let mut target = ValueMap::new();
            target.insert("id".into(), Value::Int(42));
            get_procedure(&[Value::Proplist(target.into_iter().collect())])
        });

        let value = result.expect("GetProcedure succeeds");
        assert_eq!(value, Value::String("swim".into()));
    }

    #[test]
    fn get_action_respects_target_filter() {
        let (result, _) = with_object_host_context(|| {
            let mut target = ValueMap::new();
            target.insert("id".into(), Value::Int(99));
            let target = Value::Proplist(target.into_iter().collect());
            get_action(&[target])
        });

        let value = result.expect("GetAction succeeds");
        assert_eq!(value, Value::Nil);
    }

    #[test]
    fn get_action_reads_other_object_from_world() {
        let other = fixture_world_object(ObjectId::new(99), "Dummy")
            .with_action_name("Walk");
        let world = HostWorldContext::from_objects(vec![other]);
        let (result, _) = with_object_host_context_with_world(world, || {
            let mut target = ValueMap::new();
            target.insert("id".into(), Value::Int(99));
            get_action(&[Value::Proplist(target.into_iter().collect())])
        });

        let value = result.expect("GetAction succeeds");
        assert_eq!(value, Value::String("Walk".into()));
    }

    #[test]
    fn get_action_uses_world_without_context() {
        let world = HostWorldContext::from_objects(vec![fixture_world_object(
            ObjectId::new(7),
            "Dummy",
        )
            .with_action_name("Dig")]);
        let (result, _) = with_effect_context(None, &[], world, 1, || {
            let mut target = ValueMap::new();
            target.insert("id".into(), Value::Int(7));
            get_action(&[Value::Proplist(target.into_iter().collect())])
        });

        let value = result.expect("GetAction resolves world lookup");
        assert_eq!(value, Value::String("Dig".into()));
    }

    #[test]
    fn get_action_returns_nil_without_context() {
        let value = get_action(&[]).expect("GetAction succeeds without context");
        assert_eq!(value, Value::Nil);
    }

    #[test]
    fn get_act_time_returns_zero_by_default() {
        let (result, outcome) = with_object_host_context(|| get_act_time(&[]));
        let value = result.expect("GetActTime succeeds");
        assert_eq!(value, Value::Int(0));
        assert!(outcome.object_update.is_none());
    }

    #[test]
    fn get_act_time_reflects_pending_update() {
        let (result, outcome) = with_effect_context(
            Some(HostObjectContext {
                action_ticks: 7,
                ..idle_object_context()
            }),
            &[],
            HostWorldContext::default(),
            1,
            || {
                // Re-setting the SAME action keeps Action.Time but always
                // clears the distinct PhaseDelay (C4Object.cpp:4138-4146).
                set_action(&[Value::String("Idle".into())])?;
                get_act_time(&[])
            },
        );

        let value = result.expect("GetActTime succeeds");
        assert_eq!(value, Value::Int(7));
        let update = outcome.object_update.expect("action update recorded");
        let action = update.action.expect("action update exists");
        assert_eq!(action.ticks, Some(0), "PhaseDelay resets independently");
    }

    #[test]
    fn get_act_time_resets_on_action_change() {
        let mut specs = HashMap::new();
        specs.insert("Idle".to_string(), ActionSpec::default());
        specs.insert("Walk".to_string(), ActionSpec::default());
        let library = ActionLibrary::new(Some("Idle".to_string()), specs);

        let (result, outcome) = with_effect_context(
            Some(HostObjectContext {
                action_ticks: 5,
                action_library: library.into(),
                ..idle_object_context()
            }),
            &[],
            HostWorldContext::default(),
            1,
            || {
                set_action(&[Value::String("Walk".into())])?;
                get_act_time(&[])
            },
        );

        let value = result.expect("GetActTime succeeds");
        assert_eq!(value, Value::Int(0));
        let update = outcome.object_update.expect("action update recorded");
        let action = update.action.expect("action update exists");
        assert_eq!(action.ticks, Some(0));
    }

    #[test]
    fn get_act_time_reads_world_context() {
        let other = fixture_world_object(ObjectId::new(23), "Dummy")
            .with_action_name("Walk")
            .with_action_ticks(12);
        let world = HostWorldContext::from_objects(vec![other]);
        let (result, _) = with_effect_context(None, &[], world, 1, || {
            let mut target = ValueMap::new();
            target.insert("id".into(), Value::Int(23));
            get_act_time(&[Value::Proplist(target.into_iter().collect())])
        });

        let value = result.expect("GetActTime succeeds");
        assert_eq!(value, Value::Int(12));
    }

    #[test]
    fn get_act_time_returns_nil_without_context() {
        let value = get_act_time(&[]).expect("GetActTime succeeds without context");
        assert_eq!(value, Value::Nil);
    }

    #[test]
    fn get_vertex_num_counts_vertices() {
        let vertices = [ObjectVertex::new(0, 0), ObjectVertex::new(1, -1)];
        let (result, _) = with_effect_context(
            Some(idle_object_context_with_vertices(&vertices)),
            &[],
            HostWorldContext::default(),
            1,
            || {
                assert_eq!(
                    get_vertex_num(&[object_reference_value(ObjectId::new(1))])?,
                    Value::Int(2),
                    "the explicit C4 object argument used by Warp must resolve"
                );
                get_vertex_num(&[])
            },
        );

        let value = result.expect("GetVertexNum succeeds");
        assert_eq!(value, Value::Int(2));
    }

    #[test]
    fn get_def_bottom_uses_the_untransformed_definition_shape() {
        // FnGetDefBottom returns `pObj->y + pObj->Def->Shape.y +
        // pObj->Def->Shape.Hgt`, defaults pObj to cthr->Obj, and returns
        // nil without either (C4Script.cpp:4445-4449). The object's live
        // vertices and construction/rotation are deliberately irrelevant.
        let other_id = ObjectId::new(2);
        let other = fixture_world_object(other_id, "OTHR")
            .with_energy(0)
            .with_position(Vector2::new(10, 50))
            .with_vertices(vec![ObjectVertex::new(0, 900)]);
        let definitions = Rc::new(HashMap::from([
            (
                DefinitionId::from("SELF"),
                DefinitionMetadata {
                    shape: Some(DefinitionRect::new(-2, -6, 4, 12)),
                    ..DefinitionMetadata::default()
                },
            ),
            (
                DefinitionId::from("OTHR"),
                DefinitionMetadata {
                    shape: Some(DefinitionRect::new(1, 2, 4, 7)),
                    ..DefinitionMetadata::default()
                },
            ),
        ]));
        let world =
            HostWorldContext::from_objects(vec![other]).with_definition_metadata(definitions);
        let live_vertices = [ObjectVertex::new(0, 1_000)];
        let object = HostObjectContext {
            energy: 0,
            position: Vector2::new(100, 200),
            ..idle_object_context_with_vertices(&live_vertices)
        }
        .with_definition_id("SELF");
        let (result, _) = with_effect_context(Some(object), &[], world, 1, || {
            Ok::<_, RuntimeError>(Value::Array(vec![
                get_def_bottom(&[])?,
                get_def_bottom(&[Value::Nil])?,
                get_def_bottom(&[object_reference_value(other_id)])?,
            ]))
        });

        assert_eq!(
            result.expect("GetDefBottom calls succeed"),
            Value::Array(vec![Value::Int(206), Value::Int(206), Value::Int(59)])
        );
        assert_eq!(
            get_def_bottom(&[]).expect("context-free call runs"),
            Value::Nil
        );
    }

    #[test]
    fn get_obj_dimensions_reflect_the_calling_objects_live_shape() {
        // System.c4g forwards GetObjWidth/Height to GetObjectVal
        // (planet/System.c4g/GetXVal.c:73-79). FnGetObjectVal reflects the
        // live object serialization (C4Script.cpp:4185-4195), where Shape
        // is inline (C4Object.cpp:2795) and exposes Width/Height
        // (C4Shape.cpp:496-502). Same-call SetShape is visible immediately.
        let mut script = clonk_script::Engine::new();
        register_host_functions(&mut script);
        script
            .load_script(
                r#"
                #strict 2
                func Probe()
                {
                    var before_wdt = GetObjWidth();
                    var before_hgt = GetObjHeight();
                    SetShape(-3, -4, 27, 41);
                    return [before_wdt, before_hgt, GetObjWidth(), GetObjHeight()];
                }
                "#,
            )
            .expect("dimension probe compiles");
        let definitions = Rc::new(HashMap::from([(
            DefinitionId::from("SELF"),
            DefinitionMetadata {
                shape: Some(DefinitionRect::new(-10, -15, 20, 30)),
                ..DefinitionMetadata::default()
            },
        )]));
        let world = HostWorldContext::default().with_definition_metadata(definitions);
        let object = HostObjectContext {
            energy: 0,
            ..idle_object_context()
        }
        .with_definition_id("SELF");
        let (result, _) =
            with_effect_context(Some(object), &[], world, 1, || script.call("Probe", &[]));

        assert_eq!(
            result.expect("GetObjWidth/Height succeed"),
            Value::Array(vec![
                Value::Int(20),
                Value::Int(30),
                Value::Int(27),
                Value::Int(41),
            ])
        );
    }

    #[test]
    fn set_r_updateface_orders_with_set_shape_in_same_call() {
        // SetRotation ends in UpdateFace(true), so it discards an earlier
        // SetShape; a later SetShape must still win (C4Object.cpp:5632-5647;
        // C4Script.cpp:5182-5196).
        let mut script = clonk_script::Engine::new();
        register_host_functions(&mut script);
        script
            .load_script(
                r#"
                #strict 2
                func ShapeThenRotate()
                {
                    SetShape(-3, -4, 27, 41);
                    SetR(90);
                    return [GetObjWidth(), GetObjHeight()];
                }
                func RotateThenShape()
                {
                    SetR(90);
                    SetShape(-3, -4, 27, 41);
                    return [GetObjWidth(), GetObjHeight()];
                }
                "#,
            )
            .expect("shape ordering probe compiles");
        let definitions = Rc::new(HashMap::from([(
            DefinitionId::from("SELF"),
            DefinitionMetadata {
                shape: Some(DefinitionRect::new(-10, -15, 20, 30)),
                rotateable: 1,
                ..DefinitionMetadata::default()
            },
        )]));
        let run = |function| {
            let world =
                HostWorldContext::default().with_definition_metadata(Rc::clone(&definitions));
            let object = HostObjectContext {
                energy: 0,
                ..idle_object_context()
            }
            .with_definition_id("SELF");
            with_effect_context(Some(object), &[], world, 1, || script.call(function, &[]))
        };

        let (shape_then_rotate, outcome) = run("ShapeThenRotate");
        assert_eq!(
            shape_then_rotate.expect("shape-then-rotate succeeds"),
            Value::Array(vec![Value::Int(40), Value::Int(40)])
        );
        assert_eq!(
            outcome.object_update.expect("object update").shape_override,
            Some(None)
        );

        let (rotate_then_shape, outcome) = run("RotateThenShape");
        assert_eq!(
            rotate_then_shape.expect("rotate-then-shape succeeds"),
            Value::Array(vec![Value::Int(27), Value::Int(41)])
        );
        assert_eq!(
            outcome.object_update.expect("object update").shape_override,
            Some(Some(DefinitionRect::new(-3, -4, 27, 41)))
        );
    }

    #[test]
    fn loaded_string_table_identity_survives_get_object_val_reflection() {
        let strings = clonk_script::new_string_registrations();
        clonk_script::register_loaded_c4_string(&strings, 3, "loaded");
        let loaded = clonk_script::resolve_c4_string(&strings, 3).expect("loaded S3 resolves");
        let runtime = clonk_script::C4StringValue::from("runtime");

        let mut reflection = ObjectValueReflection::default();
        let path = ["Object", "Locals"];
        push_reflected_c4value(&mut reflection, &path, &Value::String(loaded));
        push_reflected_c4value(&mut reflection, &path, &Value::String(runtime));

        assert_eq!(reflection.get("Locals", None, 0), Some(Value::from("S")));
        assert_eq!(reflection.get("Locals", None, 1), Some(Value::Int(3)));
        assert_eq!(reflection.get("Locals", None, 2), Some(Value::from("S")));
        assert_eq!(reflection.get("Locals", None, 3), Some(Value::Int(-1)));
    }

    #[test]
    fn get_object_val_reflects_serialized_object_cache_fields() {
        let target = ObjectId::new(7);
        let default_target = ObjectId::new(8);
        let reset_target = ObjectId::new(9);
        let container = ObjectId::new(2);
        let resolved_target = ObjectId::new(42);
        let layer = ObjectId::new(3);
        let mut state = crate::preview_spawn_state(
            Vector2::new(11, 22),
            OWNER_NONE,
            OWNER_NONE,
            DEFAULT_CATEGORY,
            FULL_CON,
            crate::CONTACT_DENSITY_SOLID,
            Vec::new(),
        );
        state.container = Some(container);
        state.action.target2 = Some(resolved_target);
        state.layer = Some(layer);
        let world_object = fixture_world_object(target, "SELF")
            .with_action_target2(Some(resolved_target))
            .with_energy(0)
            .with_position(state.position)
            .with_velocity(state.velocity)
            .with_container(Some(container))
        .with_compiler_fields(
            12,
            -9,
            77,
            crate::ObjectCompilerCache {
                info: "stale crew name".to_string(),
                // Live pointer is non-null, but the never-enumerated cache is
                // still zero.
                contained: 0,
                // Missing target after Denumerate retains the unresolved word.
                action_target1: 999,
                // Old Game.Objects.Enumerated encoding survives Denumerate.
                action_target2: 1_000_000_042,
                // Signed cache words are reflected without ObjectId coercion.
                layer: -7,
            },
        )
        .with_full_state(Rc::new(state));
        let default_world_object = fixture_world_object(default_target, "SELF")
            .with_energy(0);
        let mut reset_state = crate::preview_spawn_state(
            Vector2::new(3, 4),
            OWNER_NONE,
            OWNER_NONE,
            DEFAULT_CATEGORY,
            FULL_CON,
            crate::CONTACT_DENSITY_SOLID,
            Vec::new(),
        );
        reset_state.container = Some(container);
        let reset_world_object = fixture_world_object(reset_target, "SELF")
            .with_energy(0)
            .with_position(reset_state.position)
            .with_velocity(reset_state.velocity)
            .with_container(Some(container))
        .with_compiler_fields(
            0,
            0,
            -1,
            crate::ObjectCompilerCache {
                contained: 321,
                ..crate::ObjectCompilerCache::default()
            },
        )
        .with_full_state(Rc::new(reset_state));
        let world = HostWorldContext::from_objects(vec![
            world_object,
            default_world_object,
            reset_world_object,
        ])
        .with_definition_metadata(Rc::new(HashMap::from([(
            DefinitionId::from("SELF"),
            DefinitionMetadata::default(),
        )])));
        let cases = [
            ("Info", Value::String("stale crew name".into())),
            ("MotionX", Value::Int(12)),
            ("MotionY", Value::Int(-9)),
            ("LastSolidAtchFrame", Value::Int(77)),
            ("Contained", Value::Int(0)),
            ("ActionTarget1", Value::Int(999)),
            ("ActionTarget2", Value::Int(1_000_000_042)),
            ("Layer", Value::Int(-7)),
        ];

        let (result, _) = with_effect_context(None, &[], world, 10, || {
            for (entry, expected) in &cases {
                for section in [Value::Nil, Value::String("Object".into())] {
                    assert_eq!(
                        get_object_val(&[
                            Value::String((*entry).into()),
                            section,
                            object_reference_value(target),
                            Value::Int(0),
                        ])?,
                        expected.clone(),
                        "{entry} differs between root and Object-section lookup",
                    );
                }
                assert_eq!(
                    get_object_val(&[
                        Value::String((*entry).into()),
                        Value::Nil,
                        object_reference_value(target),
                        Value::Int(1),
                    ])?,
                    Value::Nil,
                    "{entry} exposes more than its single compiler primitive",
                );
            }
            assert_eq!(
                get_object_val(&[
                    Value::String("ActionTarget1".into()),
                    Value::String("Action".into()),
                    object_reference_value(target),
                    Value::Int(0),
                ])?,
                Value::Nil,
                "ActionTarget1 is inline under Object, not an Action section",
            );
            for (entry, expected) in [
                ("Info", Value::String(String::new().into())),
                ("MotionX", Value::Int(0)),
                ("MotionY", Value::Int(0)),
                ("LastSolidAtchFrame", Value::Int(-1)),
                ("Contained", Value::Int(0)),
                ("ActionTarget1", Value::Int(0)),
                ("ActionTarget2", Value::Int(0)),
                ("Layer", Value::Int(0)),
            ] {
                assert_eq!(
                    get_object_val(&[
                        Value::String(entry.into()),
                        Value::String("Object".into()),
                        object_reference_value(default_target),
                        Value::Int(0),
                    ])?,
                    expected,
                    "{entry} compiler default differs from C++",
                );
            }
            assert_eq!(
                get_object_val(&[
                    Value::String("Contained".into()),
                    Value::Nil,
                    object_reference_value(reset_target),
                    Value::Int(0),
                ])?,
                Value::Int(321),
                "typed pointer assignment preserves a stale number cache",
            );
            assert!(exit_object_at_current_position(reset_target)?);
            assert_eq!(
                get_object_val(&[
                    Value::String("Contained".into()),
                    Value::Nil,
                    object_reference_value(reset_target),
                    Value::Int(0),
                ])?,
                Value::Int(0),
                "Exit's literal-null assignment resets the cache immediately",
            );
            Ok::<_, RuntimeError>(())
        });
        result.expect("serialized cache reflection probes succeed");
    }

    #[test]
    fn get_object_val_follows_cpp_primitive_indexing_sections_and_types() {
        // FnGetObjectVal decompiles C4Object::CompileFunc through the
        // C4ValueGetCompiler. Shape and Action are inline under Object;
        // Offset trims trailing zeros, C4Fixed starts with its "F" format
        // character, and same-name entries across sibling sections share
        // one no-section entry counter (C4Script.cpp:3937-4040,4185-4195).
        let target = ObjectId::new(7);
        let mut action = ActionState::new("Walk");
        action.time = 17;
        action.ticks = 3;
        action.data = 44;
        action.phase = 2;
        let mut state = crate::preview_spawn_state(
            Vector2::new(11, 22),
            4,
            5,
            DEFAULT_CATEGORY,
            FULL_CON,
            crate::CONTACT_DENSITY_SOLID,
            Vec::new(),
        );
        state.custom_name = Some("Oracle".to_string());
        state.energy = 123;
        state.need_energy = true;
        state.action = action.clone();
        state.mobile = true;
        state.on_fire = true;
        state.in_liquid = true;
        state.entrance_status = true;
        state.own_mass = 37;
        state.plr_view_range = 650;
        state.local_vars = HashMap::from([
            ("named".to_string(), Value::Int(7)),
            ("__local_0".to_string(), Value::Int(42)),
        ]);
        let temporary = PhysicalInfo {
            energy: 900,
            ..PhysicalInfo::default()
        };
        let action_library = ActionLibrary::new(
            Some("Walk".to_string()),
            HashMap::from([("Walk".to_string(), ActionSpec::default().with_length(5))]),
        );
        state.temporary_physical = Some(temporary);
        let base_graphics = ObjectBaseGraphics {
            definition: "SKIN".to_string(),
            graphics_name: Some("Alt".to_string()),
            blit_mode: 0,
        };
        state.base_graphics = Some(base_graphics.clone());
        let fixed_position = FixedVec2::new(C4Fixed::from_raw(12_345), itofix(22));
        let fixed_velocity = FixedVec2::new(C4Fixed::from_raw(333), C4Fixed::from_raw(-444));
        let fixed_rotation = C4Fixed::from_raw(22_222);
        let world_object = HostWorldObject::with_category(
            target,
            "SELF",
            ObjectStatus::Normal,
            action.name.clone(),
            None,
            None,
            None,
            state.owner,
            state.category,
            state.energy,
            state.construction,
            state.damage,
            state.position,
            state.velocity,
            state.rotation,
            Vec::new(),
            state.action.data,
            state.action.time,
            state.action.phase,
            None,
            None,
        )
        .with_fixed_motion(fixed_position, fixed_velocity)
        .with_fixed_rotation(fixed_rotation)
        .with_in_liquid(true)
        .with_need_energy(true)
        .with_full_state(Rc::new(state.clone()));
        let definitions = Rc::new(HashMap::from([(
            DefinitionId::from("SELF"),
            DefinitionMetadata {
                name: "Self".to_string(),
                shape: Some(DefinitionRect::new(0, -4, 27, 41)),
                mass: 50,
                action_library: action_library.clone().into(),
                ..DefinitionMetadata::default()
            },
        )]));
        let world = HostWorldContext::from_objects(vec![world_object])
            .with_definition_metadata(definitions);
        let object = HostObjectContext {
            energy: state.energy,
            damage: state.damage,
            construction: (state.construction).max(0),
            owner: state.owner,
            controller: state.owner,
            position: state.position,
            velocity: state.velocity,
            rotation: state.rotation,
            action_name: action.name,
            action_ticks: action.time,
            action_data: action.data,
            action_phase: action.phase,
            action_library: action_library.into(),
            direction: Direction::Left,
            category: state.category,
            ..idle_object_scope(target)
        }
        .with_definition_id("SELF")
        .with_script_fixed_position(Some(fixed_position))
        .with_script_fixed_velocity(Some(fixed_velocity))
        .with_script_fixed_rotation(Some(fixed_rotation))
        .with_own_mass(37)
        .with_in_liquid(true)
        .with_need_energy(true)
        .with_plr_view_range(650)
        .with_base_graphics(Some(base_graphics))
        .with_physicals(None, Some(temporary), Vec::new(), PhysicalInfo::default());

        let (result, _) = with_effect_context(Some(object), &[], world, 8, || {
            set_phase(&[Value::Int(4)])?;
            let call = |entry: &str, section: Value, entry_nr: Value| {
                get_object_val(&[
                    Value::String(entry.to_string().into()),
                    section,
                    object_reference_value(target),
                    entry_nr,
                ])
            };
            let phase_delay_after_set_phase = call("PhaseDelay", Value::Nil, Value::Int(0))?;
            set_action(&[Value::String("Walk".to_string().into())])?;
            Ok::<_, RuntimeError>(Value::Array(vec![
                call("Offset", Value::Nil, Value::Nil)?,
                call("Offset", Value::Nil, Value::Bool(false))?,
                call("Offset", Value::Nil, Value::Bool(true))?,
                call("Offset", Value::Nil, Value::Int(2))?,
                call("Offset", Value::Nil, Value::Int(-1))?,
                call("Offset", Value::String("Object".into()), Value::Int(1))?,
                call("Offset", Value::String("Shape".into()), Value::Int(0))?,
                call("offset", Value::Nil, Value::Int(0))?,
                call("id", Value::Nil, Value::Int(0))?,
                call("Name", Value::Nil, Value::Int(0))?,
                call("Alive", Value::Nil, Value::Int(0))?,
                call("OwnMass", Value::Nil, Value::Int(0))?,
                call("Mobile", Value::Nil, Value::Int(0))?,
                call("OnFire", Value::Nil, Value::Int(0))?,
                call("InLiquid", Value::Nil, Value::Int(0))?,
                call("EntranceStatus", Value::Nil, Value::Int(0))?,
                call("PhysicalTemporary", Value::Nil, Value::Int(0))?,
                call("NeedEnergy", Value::Nil, Value::Int(0))?,
                call("PlrViewRange", Value::Nil, Value::Int(0))?,
                call("ActionTime", Value::Nil, Value::Int(0))?,
                call("ActionData", Value::Nil, Value::Int(0))?,
                phase_delay_after_set_phase,
                call("PhaseDelay", Value::Nil, Value::Int(0))?,
                call("FixX", Value::Nil, Value::Int(0))?,
                call("FixX", Value::Nil, Value::Int(1))?,
                call("FixR", Value::Nil, Value::Int(1))?,
                call("XDir", Value::Nil, Value::Int(1))?,
                call("YDir", Value::Nil, Value::Int(1))?,
                call("Energy", Value::Nil, Value::Int(0))?,
                call("Energy", Value::Nil, Value::Int(1))?,
                call("Energy", Value::String("Object".into()), Value::Int(0))?,
                call("Energy", Value::String("Physical".into()), Value::Int(0))?,
                call("ActionTime", Value::String("Action".into()), Value::Int(0))?,
                call("Object", Value::Nil, Value::Int(0))?,
                call("Physical", Value::Nil, Value::Int(0))?,
                call("Graphics", Value::Nil, Value::Int(0))?,
                call("Graphics", Value::Nil, Value::Int(1))?,
                call("LocalNamed", Value::Nil, Value::Int(0))?,
                call("LocalNamed", Value::Nil, Value::Int(1))?,
                call("Locals", Value::Nil, Value::Int(0))?,
                call("Locals", Value::Nil, Value::Int(1))?,
                call("Locals", Value::Nil, Value::Int(2))?,
            ]))
        });

        assert_eq!(
            result.expect("GetObjectVal probes succeed"),
            Value::Array(vec![
                Value::Int(0),
                Value::Int(0),
                Value::Int(-4),
                Value::Nil,
                Value::Nil,
                Value::Int(-4),
                Value::Nil,
                Value::Nil,
                Value::C4Id("SELF".to_string()),
                Value::String("Oracle".to_string().into()),
                Value::Bool(true),
                Value::Int(37),
                Value::Bool(true),
                Value::Bool(true),
                Value::Bool(true),
                Value::Bool(true),
                Value::Bool(true),
                Value::Bool(true),
                Value::Int(650),
                Value::Int(17),
                Value::Int(44),
                Value::Int(3),
                Value::Int(0),
                Value::String("F".to_string().into()),
                // C4Object::SetAction resets fix_x/fix_y to the current
                // integer position before returning (C4Object.cpp:4144).
                Value::Int(itofix(11).val()),
                Value::Int(22_222),
                Value::Int(333),
                Value::Int(-444),
                Value::Int(123),
                Value::Int(900),
                Value::Int(123),
                Value::Int(900),
                Value::Nil,
                Value::Nil,
                Value::Nil,
                Value::C4Id("SKIN".to_string()),
                Value::String("Alt".to_string().into()),
                Value::Int(1),
                Value::String("named".to_string().into()),
                Value::Int(1),
                Value::String("i".to_string().into()),
                Value::Int(42),
            ])
        );
    }

    #[test]
    fn add_vertex_appends_to_the_calling_objects_live_shape() {
        // FnAddVertex defaults a null pObj to cthr->Obj and returns the bool
        // from C4Shape::AddVertex (C4Script.cpp:1274-1278); AddVertex only
        // appends X/Y and increments VtxNum (C4Shape.cpp:26-32).
        let (result, outcome) =
            with_object_host_context(|| add_vertex(&[Value::Int(17), Value::Int(-9)]));

        assert_eq!(result.expect("AddVertex succeeds"), Value::Bool(true));
        let update = outcome.object_update.expect("live shape update recorded");
        assert_eq!(
            update
                .shape_vertices
                .as_ref()
                .expect("fixed shape slots recorded")
                .active(),
            &[ObjectVertex::new(17, -9)]
        );
        assert_eq!(update.live_vertices, Some(vec![ObjectVertex::new(17, -9)]));
        assert_eq!(
            update.vertices, None,
            "AddVertex must not enable C4Object::fOwnVertices"
        );
    }

    #[test]
    fn add_vertex_targets_foreign_shapes_and_stops_at_the_cpp_limit() {
        // C4Shape::AddVertex fails once VtxNum reaches C4D_MaxVertex (30)
        // and leaves the shape unchanged (C4Shape.cpp:26-32;
        // C4Constants.h C4D_MaxVertex). FnAddVertex forwards that bool for
        // an explicit pObj and returns false without any object
        // (C4Script.cpp:1274-1278).
        let target_id = ObjectId::new(2);
        let vertices: Vec<ObjectVertex> = (0..29)
            .map(|index| ObjectVertex::new(index, -index))
            .collect();
        let target = fixture_world_object(target_id, "LINE")
            .with_energy(0)
            .with_vertices(vertices.clone())
        .with_full_state(Rc::new(crate::preview_spawn_state(
            Vector2::ZERO,
            OWNER_NONE,
            OWNER_NONE,
            DEFAULT_CATEGORY,
            crate::FULL_CON,
            crate::CONTACT_DENSITY_SOLID,
            vertices,
        )));
        let target_value = object_reference_value(target_id);
        let world = HostWorldContext::from_objects(vec![target]).with_definition_metadata(Rc::new(
            HashMap::from([(DefinitionId::from("LINE"), DefinitionMetadata::default())]),
        ));
        let (result, outcome) = with_object_host_context_with_world(world, || {
            Ok::<_, RuntimeError>(Value::Array(vec![
                add_vertex(&[Value::Int(29), Value::Int(-29), target_value.clone()])?,
                add_vertex(&[Value::Int(30), Value::Int(-30), target_value.clone()])?,
            ]))
        });

        assert_eq!(
            result.expect("explicit AddVertex calls succeed"),
            Value::Array(vec![Value::Bool(true), Value::Bool(false)])
        );
        let update = outcome.other_objects[0]
            .update
            .as_ref()
            .and_then(|update| update.shape_vertices.as_ref())
            .expect("foreign fixed-slot shape update recorded")
            .active();
        assert_eq!(update.len(), 30);
        assert_eq!(update[29], ObjectVertex::new(29, -29));
        assert_eq!(
            add_vertex(&[]).expect("context-free call runs"),
            Value::Bool(false)
        );
    }

    fn with_vertex_host_context<F, T>(
        vertices: &[ObjectVertex],
        func: F,
    ) -> (Result<T, RuntimeError>, EffectContextOutcome)
    where
        F: FnOnce() -> Result<T, RuntimeError>,
    {
        with_effect_context(
            Some(idle_object_context_with_vertices(vertices)),
            &[],
            HostWorldContext::default(),
            1,
            func,
        )
    }

    fn distinct_shape_vertices() -> Vec<ObjectVertex> {
        vec![
            ObjectVertex::new(10, 11)
                .with_cnat(CNAT_LEFT)
                .with_friction(101),
            ObjectVertex::new(20, 21)
                .with_cnat(CNAT_RIGHT)
                .with_friction(202),
            ObjectVertex::new(30, 31)
                .with_cnat(CNAT_BOTTOM)
                .with_friction(303),
        ]
    }

    #[test]
    fn remove_vertex_shifts_only_xy_for_middle_and_zero_slots() {
        // C4Shape::RemoveVertex shifts VtxX/VtxY only; VtxCNAT and
        // VtxFriction remain attached to their absolute slots
        // (C4Shape.cpp:346-354).
        let vertices = distinct_shape_vertices();
        let (middle_result, middle_outcome) =
            with_vertex_host_context(&vertices, || remove_vertex(&[Value::Int(1)]));
        assert_eq!(
            middle_result.expect("middle removal succeeds"),
            Value::Bool(true)
        );
        let middle = middle_outcome
            .object_update
            .as_ref()
            .and_then(|update| update.shape_vertices.as_ref())
            .expect("middle removal records fixed shape slots")
            .active();
        assert_eq!(
            middle,
            &[
                vertices[0],
                ObjectVertex::new(vertices[2].x, vertices[2].y)
                    .with_cnat(vertices[1].cnat)
                    .with_friction(vertices[1].friction),
            ]
        );

        let (zero_result, zero_outcome) =
            with_vertex_host_context(&vertices, || remove_vertex(&[Value::Int(0)]));
        assert_eq!(
            zero_result.expect("zero removal succeeds"),
            Value::Bool(true)
        );
        let zero = zero_outcome
            .object_update
            .as_ref()
            .and_then(|update| update.shape_vertices.as_ref())
            .expect("zero removal records fixed shape slots")
            .active();
        assert_eq!(
            zero,
            &[
                ObjectVertex::new(vertices[1].x, vertices[1].y)
                    .with_cnat(vertices[0].cnat)
                    .with_friction(vertices[0].friction),
                ObjectVertex::new(vertices[2].x, vertices[2].y)
                    .with_cnat(vertices[1].cnat)
                    .with_friction(vertices[1].friction),
            ]
        );
    }

    #[test]
    fn remove_vertex_rejects_invalid_indices_without_mutating_shape() {
        // C4Shape::RemoveVertex returns false when iPos is outside
        // [0,VtxNum) and leaves the complete fixed-slot buffer untouched
        // (C4Shape.cpp:346-354).
        let vertices = distinct_shape_vertices();
        let (result, outcome) = with_vertex_host_context(&vertices, || {
            Ok::<_, RuntimeError>(Value::Array(vec![
                remove_vertex(&[Value::Int(-1)])?,
                remove_vertex(&[Value::Int(vertices.len() as i32)])?,
            ]))
        });
        assert_eq!(
            result.expect("invalid removals return normally"),
            Value::Array(vec![Value::Bool(false), Value::Bool(false)])
        );
        assert!(
            outcome.object_update.is_none(),
            "invalid removals must not stage any object mutation"
        );
        assert_eq!(
            remove_vertex(&[Value::Int(0)]).expect("context-free removal runs"),
            Value::Bool(false)
        );
    }

    #[test]
    fn remove_all_then_add_back_restores_every_vertex_tuple() {
        // WarpUSpellData repeatedly removes slot zero and later AddVertex
        // restores only X/Y. The fixed CNAT/friction slots must survive the
        // zero-active-vertex interval (Warp.c:242-264; C4Shape.cpp:26-31,
        // 346-354).
        let vertices = distinct_shape_vertices();
        let coordinates = vertices
            .iter()
            .map(|vertex| (vertex.x, vertex.y))
            .collect::<Vec<_>>();
        let (result, outcome) = with_vertex_host_context(&vertices, || {
            let mut results = Vec::new();
            for _ in 0..vertices.len() {
                results.push(remove_vertex(&[Value::Int(0)])?);
            }
            results.push(get_vertex_num(&[])?);
            for (x, y) in &coordinates {
                results.push(add_vertex(&[Value::Int(*x), Value::Int(*y)])?);
            }
            Ok::<_, RuntimeError>(Value::Array(results))
        });
        assert_eq!(
            result.expect("remove/add round trip succeeds"),
            Value::Array(vec![
                Value::Bool(true),
                Value::Bool(true),
                Value::Bool(true),
                Value::Int(0),
                Value::Bool(true),
                Value::Bool(true),
                Value::Bool(true),
            ])
        );
        let update = outcome
            .object_update
            .expect("round trip records shape update");
        assert_eq!(
            update
                .shape_vertices
                .as_ref()
                .expect("fixed slots survive the round trip")
                .active(),
            vertices
        );
        assert_eq!(update.live_vertices, Some(vertices.clone()));
        assert_eq!(
            update.vertices, None,
            "RemoveVertex/AddVertex must not enable own-vertex mode"
        );
    }

    #[test]
    fn remove_vertex_mutates_an_explicit_foreign_target() {
        let target_id = ObjectId::new(2);
        let vertices = distinct_shape_vertices();
        let target = fixture_world_object(target_id, "TARG")
            .with_energy(0)
            .with_vertices(vertices.clone())
        .with_full_state(Rc::new(crate::preview_spawn_state(
            Vector2::ZERO,
            OWNER_NONE,
            OWNER_NONE,
            DEFAULT_CATEGORY,
            crate::FULL_CON,
            crate::CONTACT_DENSITY_SOLID,
            vertices.clone(),
        )));
        let world = HostWorldContext::from_objects(vec![target]).with_definition_metadata(Rc::new(
            HashMap::from([(DefinitionId::from("TARG"), DefinitionMetadata::default())]),
        ));
        let target_value = object_reference_value(target_id);
        let (result, outcome) = with_object_host_context_with_world(world, || {
            remove_vertex(&[Value::Int(1), target_value])
        });
        assert_eq!(result.expect("foreign removal succeeds"), Value::Bool(true));
        let foreign = outcome
            .other_objects
            .iter()
            .find(|object| object.object_id == target_id)
            .and_then(|object| object.update.as_ref())
            .and_then(|update| update.shape_vertices.as_ref())
            .expect("foreign target receives its fixed-slot update")
            .active();
        assert_eq!(
            foreign,
            &[
                vertices[0],
                ObjectVertex::new(vertices[2].x, vertices[2].y)
                    .with_cnat(vertices[1].cnat)
                    .with_friction(vertices[1].friction),
            ]
        );
        assert!(outcome.object_update.is_none());
    }

    #[test]
    fn get_vertex_returns_requested_attributes() {
        let vertex = ObjectVertex::new(2, -3)
            .with_cnat(CNAT_CENTER | CNAT_BOTTOM)
            .with_friction(7);
        let vertices = [vertex];
        let (x, _) = with_effect_context(
            Some(idle_object_context_with_vertices(&vertices)),
            &[],
            HostWorldContext::default(),
            1,
            || {
                Ok::<Value, RuntimeError>(Value::Array(vec![
                    get_vertex(&[Value::Int(0), Value::Int(0)])?,
                    get_vertex(&[Value::Int(0), Value::Nil])?,
                    get_vertex(&[Value::Int(0)])?,
                    get_vertex(&[Value::Int(0), Value::Bool(false)])?,
                    get_vertex(&[Value::Int(0), Value::Bool(true)])?,
                ]))
            },
        );
        assert_eq!(
            x.expect("x succeeds"),
            Value::Array(vec![
                Value::Int(2),
                Value::Int(2),
                Value::Int(2),
                Value::Int(2),
                Value::Int(-3),
            ])
        );
        let (y, _) = with_effect_context(
            Some(idle_object_context_with_vertices(&vertices)),
            &[],
            HostWorldContext::default(),
            1,
            || get_vertex(&[Value::Int(0), Value::Int(1)]),
        );
        assert_eq!(y.expect("y succeeds"), Value::Int(-3));
        let (cnat, _) = with_effect_context(
            Some(idle_object_context_with_vertices(&vertices)),
            &[],
            HostWorldContext::default(),
            1,
            || get_vertex(&[Value::Int(0), Value::Int(2)]),
        );
        assert_eq!(
            cnat.expect("cnat succeeds"),
            Value::Int((CNAT_CENTER | CNAT_BOTTOM) as i32)
        );
        let (friction, _) = with_effect_context(
            Some(idle_object_context_with_vertices(&vertices)),
            &[],
            HostWorldContext::default(),
            1,
            || get_vertex(&[Value::Int(0), Value::Int(3)]),
        );
        assert_eq!(friction.expect("friction succeeds"), Value::Int(7));
    }

    #[test]
    fn get_vertex_contact_uses_landscape_sampling() {
        let vertices = [ObjectVertex::new(0, 0).with_cnat(CNAT_CENTER | CNAT_BOTTOM)];
        let landscape = Landscape::flat(8, 0);
        let world = world_with(
            Vec::<HostWorldObject>::new(),
            Some(landscape),
            HashMap::new(),
            HashMap::new(),
        );
        let (result, _) = with_effect_context(
            Some(idle_object_context_with_vertices(&vertices)),
            &[],
            world,
            1,
            || get_vertex_contact(&[Value::Int(0)]),
        );

        let value = result.expect("GetVertexContact succeeds");
        assert_eq!(value, Value::Int((CNAT_CENTER | CNAT_BOTTOM) as i32));
    }

    #[test]
    fn get_contact_defaults_to_vertex_zero_and_preserves_explicit_all_mask() {
        let vertices = [
            ObjectVertex::new(0, -5).with_cnat(CNAT_TOP),
            ObjectVertex::new(0, 0).with_cnat(CNAT_CENTER | CNAT_BOTTOM),
        ];
        let landscape = Landscape::flat(4, 0);
        let world = world_with(
            Vec::<HostWorldObject>::new(),
            Some(landscape),
            HashMap::new(),
            HashMap::new(),
        );
        let object = object_reference_value(ObjectId::new(1));
        let (result, _) = with_effect_context(
            Some(idle_object_context_with_vertices(&vertices)),
            &[],
            world,
            1,
            || {
                Ok::<_, RuntimeError>(Value::Array(vec![
                    get_contact(std::slice::from_ref(&object))?,
                    get_contact(&[object.clone(), Value::Nil])?,
                    get_contact(&[object.clone(), Value::Int(-1)])?,
                    get_contact(&[object, Value::Int(-1), Value::Int(CNAT_CENTER as i32)])?,
                ]))
            },
        );

        let value = result.expect("GetContact succeeds");
        assert_eq!(
            value,
            Value::Array(vec![
                Value::Int(0),
                Value::Int(0),
                Value::Int((CNAT_CENTER | CNAT_BOTTOM) as i32),
                Value::Int(CNAT_CENTER as i32),
            ])
        );
    }

