    #[test]
    fn find_func_condition_calls_function_on_candidates_like_cpp() {
        // C4FindObjectFunc (C4FindObject.cpp:124-136, 653-662): a
        // [C4FO_Func=60, name, pars...] criterion calls `name` on each
        // candidate object as the call context (`this`, never a parameter),
        // with array slot 2 -> par 0. An object whose def has no overload of
        // the function fails the check silently (FindSameNameFunc miss,
        // C4FindObject.cpp:658-659), and the result converts with raw
        // C4Value truthiness (C4Value.h:183-185): any nonzero int matches.
        let finder_script = r#"#strict
        global func ProbeFind() {
            return FindObject2([60, "IsHot", 3]);
        }
        "#;
        let probe_script = r#"
        func IsHot(threshold) {
            return GetOwner() - threshold; // raw truthiness: nonzero matches
        }
        "#;

        let mut engine = Engine::with_seed(7);
        engine
            .register_definition(
                Definition::from_script("FNDR", "Finder", finder_script).expect("finder compiles"),
            )
            .expect("finder registers");
        engine
            .register_definition(
                Definition::from_script("PROB", "Probe", probe_script).expect("probe compiles"),
            )
            .expect("probe registers");

        let finder = engine
            .spawn_object(SpawnConfig::new("FNDR"))
            .expect("finder spawns");
        // `C4ObjectList::Add(stMain)` links a new object ahead of the first
        // same-category/same-id entry (C4ObjectList.cpp:155-163), so the main
        // list walks newest-first and these spawn in reverse of the walk.
        // Owner 3 evaluates to 0 (falsy) and is met first; owner 5 is the
        // first truthy match; the finder's own def has no IsHot at all.
        let _late = engine
            .spawn_object(SpawnConfig::new("PROB").with_owner(9))
            .expect("probe spawns");
        let hit = engine
            .spawn_object(SpawnConfig::new("PROB").with_owner(5))
            .expect("probe spawns");
        let _miss = engine
            .spawn_object(SpawnConfig::new("PROB").with_owner(3))
            .expect("probe spawns");
        engine.tick_without_snapshot().expect("tick succeeds");

        let finder_idx = engine.find_object_index(finder).expect("finder exists");
        let result = engine
            .call_object_function(finder_idx, "ProbeFind", Vec::new())
            .expect("ProbeFind runs");
        assert_eq!(
            result,
            Value::Object(hit.as_u64()),
            "first candidate whose IsHot(3) returns nonzero wins (C4FindObject.cpp:188-194)"
        );
    }

    #[test]
    fn find_func_callback_side_effects_apply_to_candidates() {
        // C++ takes no precautions against Check side effects
        // (C4FindObject.cpp:186-199 only re-checks Status): the callback
        // mutates each candidate live. The copy-in/copy-out port folds the
        // nested scopes into the outcome and commits them when the outer
        // call returns — including VM-final local variables.
        let finder_script = r#"#strict
        global func TagAll() {
            return ObjectCount2([60, "Tag"]);
        }
        "#;
        let probe_script = r#"
        local tagged;
        func Tag() {
            tagged = tagged + 1;
            return 1;
        }
        "#;

        let mut engine = Engine::with_seed(7);
        engine
            .register_definition(
                Definition::from_script("FNDR", "Finder", finder_script).expect("finder compiles"),
            )
            .expect("finder registers");
        engine
            .register_definition(
                Definition::from_script("PROB", "Probe", probe_script).expect("probe compiles"),
            )
            .expect("probe registers");

        let finder = engine
            .spawn_object(SpawnConfig::new("FNDR"))
            .expect("finder spawns");
        let p1 = engine
            .spawn_object(SpawnConfig::new("PROB"))
            .expect("probe spawns");
        let p2 = engine
            .spawn_object(SpawnConfig::new("PROB"))
            .expect("probe spawns");
        engine.tick_without_snapshot().expect("tick succeeds");

        let finder_idx = engine.find_object_index(finder).expect("finder exists");
        let result = engine
            .call_object_function(finder_idx, "TagAll", Vec::new())
            .expect("TagAll runs");
        assert_eq!(result, Value::Int(2), "both probes match");
        for id in [p1, p2] {
            let idx = engine.find_object_index(id).expect("probe exists");
            assert_eq!(
                engine.objects[idx].state.local_vars.get("tagged"),
                Some(&Value::Int(1)),
                "the callback ran exactly once per candidate and its local-var \
                 write was committed"
            );
        }
    }

    #[test]
    fn find_func_callbacks_expose_live_mutations_to_later_checks() {
        // C4FindObjectAnd hands every child the same live C4Object pointer.
        // A Func child can therefore change a primitive field before the
        // next child, and a callback on an earlier object can change a later
        // candidate before its Check begins (C4FindObject.cpp:180-225,
        // 445-450, 576-579, 653-662).
        let finder_script = r#"#strict
        global func FindPromoted(object later) {
            return FindObjects(
                [20, "PROB"],
                [60, "Promote", later],
                [22, 8]
            );
        }
        "#;
        let probe_script = r#"
        func Promote(later) {
            if (GetOwner() == 1) {
                SetCategory(8);
                SetCategory(8, later);
            }
            return true;
        }
        "#;

        let mut engine = Engine::with_seed(7);
        engine
            .register_definition(
                Definition::from_script("FNDR", "Finder", finder_script).expect("finder compiles"),
            )
            .expect("finder registers");
        engine
            .register_definition(
                Definition::from_script("PROB", "Probe", probe_script).expect("probe compiles"),
            )
            .expect("probe registers");
        let finder = engine
            .spawn_object(SpawnConfig::new("FNDR"))
            .expect("finder spawns");
        let first = engine
            .spawn_object(
                SpawnConfig::new("PROB")
                    .with_owner(1)
                    .with_category(CATEGORY_OBJECT),
            )
            .expect("first probe spawns");
        let later = engine
            .spawn_object(
                SpawnConfig::new("PROB")
                    .with_owner(2)
                    .with_category(CATEGORY_VEHICLE),
            )
            .expect("later probe spawns");
        engine.tick_without_snapshot().expect("tick succeeds");

        let finder_idx = engine.find_object_index(finder).expect("finder exists");
        assert_eq!(
            engine
                .call_object_function(
                    finder_idx,
                    "FindPromoted",
                    vec![Value::Object(later.as_u64())],
                )
                .expect("live Find_Func search succeeds"),
            Value::Array(vec![
                Value::Object(first.as_u64()),
                Value::Object(later.as_u64()),
            ]),
            "same-candidate and later-candidate Category checks see callback writes"
        );
    }

    #[test]
    fn find_func_parameters_clear_removed_object_references_between_candidates() {
        // C4FindObjectFunc stores parameters as registered C4Values. Normal
        // AssignRemoval clears the stored object pointer before the next
        // candidate callback (C4Object.cpp:311-313; C4FindObject.cpp:
        // 645-650), rather than replaying the original dead pointer.
        let finder_script = r#"#strict
        global func Survivors(object victim) {
            return FindObjects(
                [20, "PROB"],
                [60, "RemoveThenObserve", victim]
            );
        }
        "#;
        let probe_script = r#"
        func RemoveThenObserve(victim) {
            var no_object;
            if (GetOwner() == 1) {
                RemoveObject(victim);
                return false;
            }
            return victim == no_object;
        }
        "#;

        let mut engine = Engine::with_seed(7);
        engine
            .register_definition(
                Definition::from_script("FNDR", "Finder", finder_script).expect("finder compiles"),
            )
            .expect("finder registers");
        engine
            .register_definition(
                Definition::from_script("PROB", "Probe", probe_script).expect("probe compiles"),
            )
            .expect("probe registers");
        engine
            .register_definition(
                Definition::from_script("VICT", "Victim", "").expect("victim compiles"),
            )
            .expect("victim registers");
        let finder = engine
            .spawn_object(SpawnConfig::new("FNDR"))
            .expect("finder spawns");
        let first = engine
            .spawn_object(
                SpawnConfig::new("PROB")
                    .with_owner(1)
                    .with_category(CATEGORY_OBJECT),
            )
            .expect("first probe spawns");
        let later = engine
            .spawn_object(
                SpawnConfig::new("PROB")
                    .with_owner(2)
                    .with_category(CATEGORY_VEHICLE),
            )
            .expect("later probe spawns");
        let victim = engine
            .spawn_object(SpawnConfig::new("VICT"))
            .expect("victim spawns");
        engine.tick_without_snapshot().expect("tick succeeds");

        let finder_idx = engine.find_object_index(finder).expect("finder exists");
        assert_eq!(
            engine
                .call_object_function(
                    finder_idx,
                    "Survivors",
                    vec![Value::Object(victim.as_u64())],
                )
                .expect("Find_Func parameter probe succeeds"),
            Value::Array(vec![Value::Object(later.as_u64())]),
            "the later candidate receives nil for the removed parameter"
        );
        let first_idx = engine.find_object_index(first).expect("first remains");
        assert_eq!(engine.objects[first_idx].state.status, ObjectStatus::Normal);
        let victim_idx = engine
            .find_object_index(victim)
            .expect("victim not swept yet");
        assert_eq!(
            engine.objects[victim_idx].state.status,
            ObjectStatus::Deleted
        );
    }

    #[test]
    fn find_object2_func_stops_after_the_first_live_match() {
        // C4FindObject::Find returns immediately without evaluating later
        // candidates once a live object passes Check (C4FindObject.cpp:
        // 180-199). FindMany-style collection would run both callbacks.
        let finder_script = r#"#strict
        global func First() { return FindObject2([20, "PROB"], [60, "Match"]); }
        "#;
        let probe_script = r#"
        local calls;
        func Match() { calls = calls + 1; return true; }
        "#;

        let mut engine = Engine::with_seed(7);
        engine
            .register_definition(
                Definition::from_script("FNDR", "Finder", finder_script).expect("finder compiles"),
            )
            .expect("finder registers");
        engine
            .register_definition(
                Definition::from_script("PROB", "Probe", probe_script).expect("probe compiles"),
            )
            .expect("probe registers");
        let finder = engine
            .spawn_object(SpawnConfig::new("FNDR"))
            .expect("finder spawns");
        let first = engine
            .spawn_object(SpawnConfig::new("PROB").with_category(CATEGORY_OBJECT))
            .expect("first probe spawns");
        let later = engine
            .spawn_object(SpawnConfig::new("PROB").with_category(CATEGORY_VEHICLE))
            .expect("later probe spawns");
        engine.tick_without_snapshot().expect("tick succeeds");

        let finder_idx = engine.find_object_index(finder).expect("finder exists");
        assert_eq!(
            engine
                .call_object_function(finder_idx, "First", Vec::new())
                .expect("FindObject2 succeeds"),
            Value::Object(first.as_u64())
        );
        let first_idx = engine.find_object_index(first).expect("first exists");
        let later_idx = engine.find_object_index(later).expect("later exists");
        assert_eq!(
            engine.objects[first_idx].state.local_vars.get("calls"),
            Some(&Value::Int(1))
        );
        assert_eq!(
            engine.objects[later_idx].state.local_vars.get("calls"),
            None,
            "the later callback is never evaluated"
        );
    }

    #[test]
    fn find_func_removal_uses_each_cpp_driver_status_rule() {
        // Find rechecks Status after Check and continues past a removed
        // truthy candidate. Count deliberately has no post-Check recheck and
        // still counts that same truthy result (C4FindObject.cpp:164-199).
        let finder_script = r#"#strict
        global func FirstLive() {
            return FindObject2([20, "PROB"], [60, "RemoveFirst"]);
        }
        global func CountRemoved() {
            return ObjectCount2([20, "COUN"], [60, "RemoveAndMatch"]);
        }
        "#;
        let probe_script = r#"
        func RemoveFirst() {
            if (GetOwner() == 1) { RemoveObject(); }
            return true;
        }
        "#;
        let counter_script = r#"
        func RemoveAndMatch() { RemoveObject(); return true; }
        "#;

        let mut engine = Engine::with_seed(7);
        for (id, name, script) in [
            ("FNDR", "Finder", finder_script),
            ("PROB", "Probe", probe_script),
            ("COUN", "Counter", counter_script),
        ] {
            engine
                .register_definition(
                    Definition::from_script(id, name, script).expect("definition compiles"),
                )
                .expect("definition registers");
        }
        let finder = engine
            .spawn_object(SpawnConfig::new("FNDR"))
            .expect("finder spawns");
        let removed_probe = engine
            .spawn_object(
                SpawnConfig::new("PROB")
                    .with_owner(1)
                    .with_category(CATEGORY_OBJECT),
            )
            .expect("first probe spawns");
        let live_probe = engine
            .spawn_object(
                SpawnConfig::new("PROB")
                    .with_owner(2)
                    .with_category(CATEGORY_VEHICLE),
            )
            .expect("second probe spawns");
        let counted = engine
            .spawn_object(SpawnConfig::new("COUN"))
            .expect("counter spawns");
        engine.tick_without_snapshot().expect("tick succeeds");

        let finder_idx = engine.find_object_index(finder).expect("finder exists");
        assert_eq!(
            engine
                .call_object_function(finder_idx, "FirstLive", Vec::new())
                .expect("FindObject2 succeeds"),
            Value::Object(live_probe.as_u64())
        );
        let finder_idx = engine.find_object_index(finder).expect("finder remains");
        assert_eq!(
            engine
                .call_object_function(finder_idx, "CountRemoved", Vec::new())
                .expect("ObjectCount2 succeeds"),
            Value::Int(1),
            "Count includes a truthy callback even when it removes its object"
        );
        for id in [removed_probe, counted] {
            let index = engine.find_object_index(id).expect("not swept yet");
            assert_eq!(engine.objects[index].state.status, ObjectStatus::Deleted);
        }
    }

    #[test]
    fn find_func_callback_error_aborts_calling_script() {
        // fPassErrors=true (C4FindObject.cpp:661): a runtime error inside
        // the callback rethrows out of Check/Find and aborts the calling
        // script (C4AulExec.cpp:1318-1342).
        let finder_script = r#"#strict
        global func Boom() {
            return FindObject2([60, "Explode"]);
        }
        "#;
        let probe_script = r#"
        func Explode() {
            return NoSuchFunctionAnywhere();
        }
        "#;

        let mut engine = Engine::with_seed(7);
        engine
            .register_definition(
                Definition::from_script("FNDR", "Finder", finder_script).expect("finder compiles"),
            )
            .expect("finder registers");
        engine
            .register_definition(
                Definition::from_script("PROB", "Probe", probe_script).expect("probe compiles"),
            )
            .expect("probe registers");
        let finder = engine
            .spawn_object(SpawnConfig::new("FNDR"))
            .expect("finder spawns");
        engine
            .spawn_object(SpawnConfig::new("PROB"))
            .expect("probe spawns");
        engine.tick_without_snapshot().expect("tick succeeds");

        let finder_idx = engine.find_object_index(finder).expect("finder exists");
        let result = engine.call_object_function(finder_idx, "Boom", Vec::new());
        assert!(
            result.is_err(),
            "the callback's runtime error passes through FindObject2"
        );
    }

    #[test]
    fn find_func_unknown_name_prunes_search_silently() {
        // IsImpossible = !pFunc (C4FindObject.cpp:664-667): a name unknown
        // to every script makes the criterion impossible — FindMany returns
        // an empty array without iterating, and no error is raised.
        let finder_script = r#"#strict
        global func Hunt() {
            return ObjectCount2([60, "NoSuchPredicate"]);
        }
        "#;

        let mut engine = Engine::with_seed(7);
        engine
            .register_definition(
                Definition::from_script("FNDR", "Finder", finder_script).expect("finder compiles"),
            )
            .expect("finder registers");
        let finder = engine
            .spawn_object(SpawnConfig::new("FNDR"))
            .expect("finder spawns");
        engine.tick_without_snapshot().expect("tick succeeds");

        let finder_idx = engine.find_object_index(finder).expect("finder exists");
        let result = engine
            .call_object_function(finder_idx, "Hunt", Vec::new())
            .expect("Hunt runs without error");
        assert_eq!(result, Value::Int(0));
    }

    #[test]
    fn find_func_uses_raw_truthiness_not_get_bool() {
        // C4Value::operator bool (C4Value.h:76,183-185): the Check result is
        // raw-data truthiness — a string is true (nonnull pointer), with no
        // getBool-style type conversion; 0/false/nil are the only falses.
        let finder_script = r#"#strict
        global func CountStrings() { return ObjectCount2([60, "GiveString"]); }
        global func CountZeroes() { return ObjectCount2([60, "GiveZero"]); }
        global func CountFalses() { return ObjectCount2([60, "GiveFalse"]); }
        "#;
        let probe_script = r#"
        func GiveString() { return "yes"; }
        func GiveZero() { return 0; }
        func GiveFalse() { return false; }
        "#;

        let mut engine = Engine::with_seed(7);
        engine
            .register_definition(
                Definition::from_script("FNDR", "Finder", finder_script).expect("finder compiles"),
            )
            .expect("finder registers");
        engine
            .register_definition(
                Definition::from_script("PROB", "Probe", probe_script).expect("probe compiles"),
            )
            .expect("probe registers");
        let finder = engine
            .spawn_object(SpawnConfig::new("FNDR"))
            .expect("finder spawns");
        engine
            .spawn_object(SpawnConfig::new("PROB"))
            .expect("probe spawns");
        engine.tick_without_snapshot().expect("tick succeeds");

        let finder_idx = engine.find_object_index(finder).expect("finder exists");
        let strings = engine
            .call_object_function(finder_idx, "CountStrings", Vec::new())
            .expect("runs");
        assert_eq!(strings, Value::Int(1), "a string return is raw-truthy");
        let zeroes = engine
            .call_object_function(finder_idx, "CountZeroes", Vec::new())
            .expect("runs");
        assert_eq!(zeroes, Value::Int(0));
        let falses = engine
            .call_object_function(finder_idx, "CountFalses", Vec::new())
            .expect("runs");
        assert_eq!(falses, Value::Int(0));
    }

    #[test]
    fn find_func_evaluates_the_calling_object_too() {
        // The C++ Find walks the full object list — the caller is a
        // candidate like any other (its live scope serves as the call
        // context when the predicate runs on it).
        let finder_script = r#"#strict
        local mark;
        func AmI() { return 1; }
        global func HuntSelf() { return ObjectCount2([60, "AmI"]); }
        "#;

        let mut engine = Engine::with_seed(7);
        engine
            .register_definition(
                Definition::from_script("FNDR", "Finder", finder_script).expect("finder compiles"),
            )
            .expect("finder registers");
        let finder = engine
            .spawn_object(SpawnConfig::new("FNDR"))
            .expect("finder spawns");
        engine.tick_without_snapshot().expect("tick succeeds");

        let finder_idx = engine.find_object_index(finder).expect("finder exists");
        let result = engine
            .call_object_function(finder_idx, "HuntSelf", Vec::new())
            .expect("HuntSelf runs");
        assert_eq!(
            result,
            Value::Int(1),
            "the caller matches its own predicate"
        );
    }

    #[test]
    fn sort_func_orders_ascending_by_cached_values_like_cpp() {
        // C4SortObjectFunc (C4FindObject.cpp:934-956): [C4SO_Func=160, name,
        // pars...] calls `name` once per object in find order via
        // PrepareCache (C4FindObject.cpp:819-832), converts with getInt(),
        // and stable-sorts ascending ("least return values first",
        // C4FindObject.h:61).
        let finder_script = r#"#strict
        global func Ranked() {
            return FindObjects([20, "PROB"], [160, "Rank"]);
        }
        "#;
        let probe_script = r#"
        local calls;
        func Rank() {
            calls = calls + 1;
            return GetOwner();
        }
        "#;

        let mut engine = Engine::with_seed(7);
        engine
            .register_definition(
                Definition::from_script("FNDR", "Finder", finder_script).expect("finder compiles"),
            )
            .expect("finder registers");
        engine
            .register_definition(
                Definition::from_script("PROB", "Probe", probe_script).expect("probe compiles"),
            )
            .expect("probe registers");
        let finder = engine
            .spawn_object(SpawnConfig::new("FNDR"))
            .expect("finder spawns");
        let p5 = engine
            .spawn_object(SpawnConfig::new("PROB").with_owner(5))
            .expect("probe spawns");
        let p1 = engine
            .spawn_object(SpawnConfig::new("PROB").with_owner(1))
            .expect("probe spawns");
        let p3 = engine
            .spawn_object(SpawnConfig::new("PROB").with_owner(3))
            .expect("probe spawns");
        engine.tick_without_snapshot().expect("tick succeeds");

        let finder_idx = engine.find_object_index(finder).expect("finder exists");
        let result = engine
            .call_object_function(finder_idx, "Ranked", Vec::new())
            .expect("Ranked runs");
        assert_eq!(
            result,
            Value::Array(vec![
                Value::Object(p1.as_u64()),
                Value::Object(p3.as_u64()),
                Value::Object(p5.as_u64()),
            ]),
            "ascending by Rank() return"
        );
        // PrepareCache evaluates exactly once per object.
        for id in [p1, p3, p5] {
            let idx = engine.find_object_index(id).expect("probe exists");
            assert_eq!(
                engine.objects[idx].state.local_vars.get("calls"),
                Some(&Value::Int(1)),
                "cached: one call per object (C4FindObject.cpp:826-829)"
            );
        }
    }

    #[test]
    fn sort_func_callbacks_expose_live_mutations_to_later_cached_criteria() {
        // C4SortObjectMultiple prepares each child cache in order. The first
        // Sort_Func cache can mutate an object before the later Distance
        // cache dereferences that same live pointer (C4FindObject.cpp:
        // 819-832, 877-883, 908-911, 934-956).
        let finder_script = r#"#strict
        global func Ranked(object later) {
            return FindObjects(
                [20, "PROB"],
                [102, [160, "MoveLater", later], [110, 0, 0]]
            );
        }
        "#;
        let probe_script = r#"
        func MoveLater(later) {
            if (GetOwner() == 1) { SetPosition(0, 0, later); }
            return 0;
        }
        "#;

        let mut engine = Engine::with_seed(7);
        engine
            .register_definition(
                Definition::from_script("FNDR", "Finder", finder_script).expect("finder compiles"),
            )
            .expect("finder registers");
        engine
            .register_definition(
                Definition::from_script("PROB", "Probe", probe_script).expect("probe compiles"),
            )
            .expect("probe registers");
        let finder = engine
            .spawn_object(SpawnConfig::new("FNDR"))
            .expect("finder spawns");
        let first = engine
            .spawn_object(
                SpawnConfig::new("PROB")
                    .with_owner(1)
                    .with_category(CATEGORY_OBJECT)
                    .with_position(Vector2::new(10, 0)),
            )
            .expect("first probe spawns");
        let later = engine
            .spawn_object(
                SpawnConfig::new("PROB")
                    .with_owner(2)
                    .with_category(CATEGORY_VEHICLE)
                    .with_position(Vector2::new(100, 0)),
            )
            .expect("later probe spawns");
        engine.tick_without_snapshot().expect("tick succeeds");

        let finder_idx = engine.find_object_index(finder).expect("finder exists");
        assert_eq!(
            engine
                .call_object_function(finder_idx, "Ranked", vec![Value::Object(later.as_u64())],)
                .expect("cached sort succeeds"),
            Value::Array(vec![
                Value::Object(later.as_u64()),
                Value::Object(first.as_u64()),
            ]),
            "the Distance cache sees the preceding Sort_Func position write"
        );
    }

    #[test]
    fn find_object2_with_sort_uses_uncached_pairwise_compare_like_cpp() {
        // The single-result Find path keeps a running best and calls the
        // UNCACHED Compare(candidate, best) per passing candidate
        // (C4FindObject.cpp:188-199) — CompareGetValue runs for obj1 then
        // obj2 in hardcoded order ("might lead to desyncs otherwise
        // [Icewing]", C4FindObject.cpp:834-842), with no PrepareCache. So
        // the first match is never evaluated on its own, and the running
        // best re-evaluates on every later comparison.
        let finder_script = r#"#strict
        global func Best() {
            return FindObject2([20, "PROB"], [160, "Rank"]);
        }
        "#;
        let probe_script = r#"
        local calls;
        local rank;
        func Construction() { return 1; }
        func SetRank(value) { rank = value; return 1; }
        func Rank() {
            calls = calls + 1;
            return rank;
        }
        "#;

        let mut engine = Engine::with_seed(7);
        engine
            .register_definition(
                Definition::from_script("FNDR", "Finder", finder_script).expect("finder compiles"),
            )
            .expect("finder registers");
        engine
            .register_definition(
                Definition::from_script("PROB", "Probe", probe_script).expect("probe compiles"),
            )
            .expect("probe registers");
        let finder = engine
            .spawn_object(SpawnConfig::new("FNDR"))
            .expect("finder spawns");
        let a = engine
            .spawn_object(SpawnConfig::new("PROB"))
            .expect("probe spawns");
        let b = engine
            .spawn_object(SpawnConfig::new("PROB"))
            .expect("probe spawns");
        let c = engine
            .spawn_object(SpawnConfig::new("PROB"))
            .expect("probe spawns");
        engine.tick_without_snapshot().expect("tick succeeds");
        for (id, rank) in [(a, 1), (b, 2), (c, 3)] {
            let idx = engine.find_object_index(id).expect("probe exists");
            engine
                .call_object_function(idx, "SetRank", vec![Value::Int(rank)])
                .expect("SetRank runs");
        }

        let finder_idx = engine.find_object_index(finder).expect("finder exists");
        let result = engine
            .call_object_function(finder_idx, "Best", Vec::new())
            .expect("Best runs");
        assert_eq!(result, Value::Object(a.as_u64()), "least rank wins");
        // A is best from the start: evaluated in Compare(B,A) and
        // Compare(C,A); B and C once each; nobody is evaluated for the
        // first match itself.
        for (id, expected_calls) in [(a, 2), (b, 1), (c, 1)] {
            let idx = engine.find_object_index(id).expect("probe exists");
            assert_eq!(
                engine.objects[idx].state.local_vars.get("calls"),
                Some(&Value::Int(expected_calls)),
                "uncached Compare evaluation counts (C4FindObject.cpp:188-199)"
            );
        }
    }

    #[test]
    fn sort_func_callbacks_expose_live_mutations_to_later_uncached_criteria() {
        // The single-result path compares candidate then best for each
        // criterion. During Compare(second, first), the Func value for the
        // second object moves the first before Distance reads either object.
        let finder_script = r#"#strict
        global func Best(object first) {
            return FindObject2(
                [20, "PROB"],
                [102, [160, "MoveBest", first], [110, 0, 0]]
            );
        }
        "#;
        let probe_script = r#"
        func MoveBest(first) {
            if (GetOwner() == 2) { SetPosition(0, 0, first); }
            return 0;
        }
        "#;

        let mut engine = Engine::with_seed(7);
        engine
            .register_definition(
                Definition::from_script("FNDR", "Finder", finder_script).expect("finder compiles"),
            )
            .expect("finder registers");
        engine
            .register_definition(
                Definition::from_script("PROB", "Probe", probe_script).expect("probe compiles"),
            )
            .expect("probe registers");
        let finder = engine
            .spawn_object(SpawnConfig::new("FNDR"))
            .expect("finder spawns");
        let first = engine
            .spawn_object(
                SpawnConfig::new("PROB")
                    .with_owner(1)
                    .with_category(CATEGORY_OBJECT)
                    .with_position(Vector2::new(100, 0)),
            )
            .expect("first probe spawns");
        let second = engine
            .spawn_object(
                SpawnConfig::new("PROB")
                    .with_owner(2)
                    .with_category(CATEGORY_VEHICLE)
                    .with_position(Vector2::new(10, 0)),
            )
            .expect("second probe spawns");
        engine.tick_without_snapshot().expect("tick succeeds");

        let finder_idx = engine.find_object_index(finder).expect("finder exists");
        assert_eq!(
            engine
                .call_object_function(finder_idx, "Best", vec![Value::Object(first.as_u64())],)
                .expect("uncached sort succeeds"),
            Value::Object(first.as_u64()),
            "Distance sees the mutation made earlier in the same Compare"
        );
    }

    #[test]
    fn find_object2_sort_func_rejects_a_candidate_removed_during_compare() {
        // Find rechecks a would-be winner's Status after Compare. The second
        // object deliberately returns the smaller key but removes itself;
        // it must not replace the first live best (C4FindObject.cpp:188-199).
        let finder_script = r#"#strict
        global func Best() {
            return FindObject2([20, "PROB"], [160, "RankAndVanish"]);
        }
        "#;
        let probe_script = r#"
        func RankAndVanish() {
            if (GetOwner() == 2) {
                RemoveObject();
                return -1;
            }
            return 0;
        }
        "#;

        let mut engine = Engine::with_seed(7);
        engine
            .register_definition(
                Definition::from_script("FNDR", "Finder", finder_script).expect("finder compiles"),
            )
            .expect("finder registers");
        engine
            .register_definition(
                Definition::from_script("PROB", "Probe", probe_script).expect("probe compiles"),
            )
            .expect("probe registers");
        let finder = engine
            .spawn_object(SpawnConfig::new("FNDR"))
            .expect("finder spawns");
        let first = engine
            .spawn_object(
                SpawnConfig::new("PROB")
                    .with_owner(1)
                    .with_category(CATEGORY_OBJECT),
            )
            .expect("first probe spawns");
        let removed = engine
            .spawn_object(
                SpawnConfig::new("PROB")
                    .with_owner(2)
                    .with_category(CATEGORY_VEHICLE),
            )
            .expect("second probe spawns");
        engine.tick_without_snapshot().expect("tick succeeds");

        let finder_idx = engine.find_object_index(finder).expect("finder exists");
        assert_eq!(
            engine
                .call_object_function(finder_idx, "Best", Vec::new())
                .expect("sorted FindObject2 succeeds"),
            Value::Object(first.as_u64())
        );
        let removed_idx = engine.find_object_index(removed).expect("not swept yet");
        assert_eq!(
            engine.objects[removed_idx].state.status,
            ObjectStatus::Deleted
        );
    }

    #[test]
    fn find_objects_replaces_objects_destroyed_during_sort_with_nil() {
        // After sorting, objects a Sort_Func callback deleted are REPLACED
        // with nullptr instead of erased (CheckObjectStatusAfterSort,
        // replace_if — C4FindObject.cpp:223, 362, 372-375), so a FindObjects
        // result can legitimately contain nil entries.
        let finder_script = r#"#strict
        global func Cull() {
            return FindObjects([20, "PROB"], [160, "Rate"]);
        }
        "#;
        let probe_script = r#"
        func Rate() {
            if (GetOwner() == 5) {
                RemoveObject();
            }
            return GetOwner();
        }
        "#;

        let mut engine = Engine::with_seed(7);
        engine
            .register_definition(
                Definition::from_script("FNDR", "Finder", finder_script).expect("finder compiles"),
            )
            .expect("finder registers");
        engine
            .register_definition(
                Definition::from_script("PROB", "Probe", probe_script).expect("probe compiles"),
            )
            .expect("probe registers");
        let finder = engine
            .spawn_object(SpawnConfig::new("FNDR"))
            .expect("finder spawns");
        let p5 = engine
            .spawn_object(SpawnConfig::new("PROB").with_owner(5))
            .expect("probe spawns");
        let p1 = engine
            .spawn_object(SpawnConfig::new("PROB").with_owner(1))
            .expect("probe spawns");
        let p3 = engine
            .spawn_object(SpawnConfig::new("PROB").with_owner(3))
            .expect("probe spawns");
        engine.tick_without_snapshot().expect("tick succeeds");

        let finder_idx = engine.find_object_index(finder).expect("finder exists");
        let result = engine
            .call_object_function(finder_idx, "Cull", Vec::new())
            .expect("Cull runs");
        assert_eq!(
            result,
            Value::Array(vec![
                Value::Object(p1.as_u64()),
                Value::Object(p3.as_u64()),
                Value::Nil,
            ]),
            "the destroyed owner-5 probe keeps its sorted slot as nil"
        );
        // Deletion is deferred like C++ (swept at end of frame); the commit
        // shows as Deleted status immediately.
        let p5_idx = engine.find_object_index(p5).expect("not yet swept");
        assert_eq!(
            engine.objects[p5_idx].state.status,
            ObjectStatus::Deleted,
            "the RemoveObject from the sort callback was committed"
        );
    }

    #[test]
    fn sort_func_converts_results_with_get_int() {
        // CompareGetValue converts with getInt() (C4FindObject.cpp:955;
        // C4Value.h:159): bools become 0/1, pointer types (strings, arrays)
        // become 0 — unlike Find_Func's raw truthiness.
        let finder_script = r#"#strict
        global func Ranked() {
            return FindObjects([20, "PROB"], [160, "Rank"]);
        }
        "#;
        // String-returning ranks all convert to 0 → stable sort keeps
        // collection order; a true bool ranks as 1 (after the zeros).
        let probe_script = r#"
        func Rank() {
            if (GetOwner() == 7) { return true; }
            return "not an int";
        }
        "#;

        let mut engine = Engine::with_seed(7);
        engine
            .register_definition(
                Definition::from_script("FNDR", "Finder", finder_script).expect("finder compiles"),
            )
            .expect("finder registers");
        engine
            .register_definition(
                Definition::from_script("PROB", "Probe", probe_script).expect("probe compiles"),
            )
            .expect("probe registers");
        let finder = engine
            .spawn_object(SpawnConfig::new("FNDR"))
            .expect("finder spawns");
        let bool_probe = engine
            .spawn_object(SpawnConfig::new("PROB").with_owner(7))
            .expect("probe spawns");
        let s1 = engine
            .spawn_object(SpawnConfig::new("PROB").with_owner(1))
            .expect("probe spawns");
        let s2 = engine
            .spawn_object(SpawnConfig::new("PROB").with_owner(2))
            .expect("probe spawns");
        engine.tick_without_snapshot().expect("tick succeeds");

        let finder_idx = engine.find_object_index(finder).expect("finder exists");
        let result = engine
            .call_object_function(finder_idx, "Ranked", Vec::new())
            .expect("Ranked runs");
        assert_eq!(
            result,
            Value::Array(vec![
                Value::Object(s2.as_u64()),
                Value::Object(s1.as_u64()),
                Value::Object(bool_probe.as_u64()),
            ]),
            "strings rank 0 (stable, newest-first collection order); true ranks 1"
        );
    }

    #[test]
    fn global_function_bodies_resolve_in_engine_scope() {
        // A `global func` is owned by Game.ScriptEngine, so calls in its body
        // resolve through that engine, not the invoking object's definition
        // (C4AulParse.cpp:2808-2813). Installation reaches already-registered
        // definitions too.
        let def_script = r#"
        func Shadowed() { return 2; }
        global func Probe() { return Helper(6); }
        global func Probe2() { return Shadowed(); }
        "#;

        let mut engine = Engine::with_seed(7);
        engine
            .register_definition(
                Definition::from_script("TSTD", "Test", def_script).expect("script compiles"),
            )
            .expect("definition registers");
        let loaded = engine.install_global_scripts(&[(
            "System.c4g/Test.c".to_string(),
            "global func Helper(n) { return n * 7; }\nglobal func Shadowed() { return 1; }\n"
                .to_string(),
        )]);
        assert_eq!(loaded, 1);

        let id = engine
            .spawn_object(SpawnConfig::new("TSTD"))
            .expect("object spawns");
        engine.tick_without_snapshot().expect("tick succeeds");
        let idx = engine.find_object_index(id).expect("object exists");
        assert_eq!(
            engine
                .call_object_function(idx, "Probe", Vec::new())
                .expect("Probe runs"),
            Value::Int(42),
            "the global Helper resolves from a definition script"
        );
        assert_eq!(
            engine
                .call_object_function(idx, "Probe2", Vec::new())
                .expect("Probe2 runs"),
            Value::Int(1),
            "the global function stays in engine scope"
        );
    }

    #[test]
    fn global_script_parse_recovery_keeps_later_good_functions() {
        let mut engine = Engine::with_seed(7);
        let loaded = engine.install_global_scripts(&[(
            "System.c4g/Recover.c".to_string(),
            "global func BrokenGlobal() { , }\n\
             global func HealthyGlobal() { return 42; }\n"
                .to_string(),
        )]);
        assert_eq!(
            loaded, 1,
            "one bad function does not skip its System.c4g script"
        );

        engine
            .register_definition(
                Definition::from_script(
                    "TSTD",
                    "Test",
                    "func ReadHealthy() { return HealthyGlobal(); }\n\
                     func CallBroken() { return BrokenGlobal(); }",
                )
                .expect("definition compiles"),
            )
            .expect("definition registers");
        let id = engine
            .spawn_object(SpawnConfig::new("TSTD"))
            .expect("object spawns");
        let idx = engine.find_object_index(id).expect("object exists");
        assert_eq!(
            engine
                .call_object_function(idx, "ReadHealthy", Vec::new())
                .expect("the healthy global function runs"),
            Value::Int(42)
        );
        engine
            .call_object_function(idx, "CallBroken", Vec::new())
            .expect_err("the broken global function remains an erroring symbol");
    }

    #[test]
    fn global_numbered_slots_are_live_references_like_cpp() {
        // FnGlobal returns C4ValueList::operator[](index).GetRef()
        // (C4Script.cpp:3404-3407); the mutable list clamps negative indices
        // to zero and grows on demand (C4ValueList.cpp:50-64).
        let script = r#"#strict
        global func & Numbered(index) { return Global(index); }
        func Probe() {
            Global(-4) = 7;
            ++Global(2);
            Global(2) += 4;
            Numbered(3) = 9;
            return [Global(0), Global(2), Global(), Global(3)];
        }
        "#;
        let mut engine = Engine::with_seed(7);
        engine
            .register_definition(
                Definition::from_script("TSTD", "Test", script).expect("script compiles"),
            )
            .expect("definition registers");
        let id = engine
            .spawn_object(SpawnConfig::new("TSTD"))
            .expect("object spawns");
        let idx = engine.find_object_index(id).expect("object exists");

        assert_eq!(
            engine
                .call_object_function(idx, "Probe", Vec::new())
                .expect("Probe runs"),
            Value::Array(vec![
                Value::Int(7),
                Value::Int(5),
                Value::Int(7),
                Value::Int(9),
            ])
        );
    }

    #[test]
    fn globaln_returns_the_declared_named_global_by_reference() {
        // FnGlobalN looks up an existing GlobalNamed entry and returns its
        // C4Value reference (C4Script.cpp:4607-4617). Reference-returning
        // script functions preserve that exact cell (C4AulExec.cpp:416-430).
        let script = r#"#strict
        static spell;
        global func & Dynamic(name) { return GlobalN(name); }
        func Probe() {
            spell = 11;
            GlobalN("spell") += 2;
            Dynamic("spell")++;
            return [spell, GlobalN("spell")];
        }
        "#;
        let mut engine = Engine::with_seed(7);
        engine
            .register_definition(
                Definition::from_script("TSTD", "Test", script).expect("script compiles"),
            )
            .expect("definition registers");
        let id = engine
            .spawn_object(SpawnConfig::new("TSTD"))
            .expect("object spawns");
        let idx = engine.find_object_index(id).expect("object exists");

        assert_eq!(
            engine
                .call_object_function(idx, "Probe", Vec::new())
                .expect("Probe runs"),
            Value::Array(vec![Value::Int(14), Value::Int(14)])
        );
    }

    #[test]
    fn global_access_matches_cpp_missing_bounds_and_parameter_types() {
        // Engine-call parameter conversion precedes FnGlobal/FnGlobalN
        // (C4AulExec.cpp:1362-1396); C4ValueList rejects index MaxSize and
        // GlobalN returns nil (not a new cell) on a missing name
        // (C4ValueList.cpp:50-64; C4Script.cpp:4607-4617).
        let script = r#"#strict 3
        func Coerce() {
            Global(true) = 3;
            Global(nil) = 4;
            return [Global(1), Global(0), GlobalN("missing")];
        }
        func TooLarge() { return Global(1000000); }
        func BadIndexType() { return Global("0"); }
        func BadNameType() { return GlobalN(0); }
        func MissingWrite() { GlobalN("missing") = 1; }
        "#;
        let mut engine = Engine::with_seed(7);
        engine
            .register_definition(
                Definition::from_script("TSTD", "Test", script).expect("script compiles"),
            )
            .expect("definition registers");
        let id = engine
            .spawn_object(SpawnConfig::new("TSTD"))
            .expect("object spawns");
        let idx = engine.find_object_index(id).expect("object exists");

        assert_eq!(
            engine
                .call_object_function(idx, "Coerce", Vec::new())
                .expect("bool and nil convert like C4Value"),
            Value::Array(vec![Value::Int(3), Value::Int(4), Value::Nil])
        );
        for function in ["TooLarge", "BadIndexType", "BadNameType", "MissingWrite"] {
            assert!(
                engine
                    .call_object_function(idx, function, Vec::new())
                    .is_err(),
                "{function} must reject the same invalid access as C++"
            );
        }
    }

    #[test]
    fn numbered_globals_are_shared_across_script_hosts() {
        // Game.ScriptEngine.Global is one table for every definition host
        // (C4Aul.h:549; FnGlobal in C4Script.cpp:3404-3407).
        let mut engine = Engine::with_seed(7);
        engine
            .register_definition(
                Definition::from_script(
                    "WRTR",
                    "Writer",
                    "func Put() { Global(8) = 41; return Global(8); }",
                )
                .expect("writer compiles"),
            )
            .expect("writer registers");
        engine
            .register_definition(
                Definition::from_script("READ", "Reader", "func Read() { return Global(8); }")
                    .expect("reader compiles"),
            )
            .expect("reader registers");
        let writer = engine
            .spawn_object(SpawnConfig::new("WRTR"))
            .expect("writer spawns");
        let reader = engine
            .spawn_object(SpawnConfig::new("READ"))
            .expect("reader spawns");
        let writer_idx = engine.find_object_index(writer).expect("writer exists");
        assert_eq!(
            engine
                .call_object_function(writer_idx, "Put", Vec::new())
                .expect("write succeeds"),
            Value::Int(41)
        );
        let reader_idx = engine.find_object_index(reader).expect("reader exists");
        assert_eq!(
            engine
                .call_object_function(reader_idx, "Read", Vec::new())
                .expect("read succeeds"),
            Value::Int(41)
        );
    }

    #[test]
    fn script_globals_survive_json_restore_with_live_object_references() {
        // C4AulScriptEngine::CompileFunc saves `Globals` and `GlobalNamed`,
        // then DenumerateVariablePointers recursively resolves their object
        // references after objects load (C4Aul.cpp:506-520;
        // C4Value.cpp:686-713).
        let writer_script = r#"#strict
        static named_scalar, named_refs;
        func Seed(object target) {
            Global(2) = 17;
            Global(8) = [target, [target]];
            GlobalN("named_scalar") = 23;
            GlobalN("named_refs") = [target, [target]];
            return true;
        }
        "#;
        let reader_script = r#"#strict
        func Read() {
            return [Global(2), Global(8), GlobalN("named_scalar"), GlobalN("named_refs")];
        }
        func Mutate() {
            Global(2) += 1;
            GlobalN("named_scalar") += 2;
            return [Global(2), GlobalN("named_scalar")];
        }
        "#;
        let register = |engine: &mut Engine| {
            engine
                .register_definition(
                    Definition::from_script("WRTR", "Writer", writer_script)
                        .expect("writer compiles"),
                )
                .expect("writer registers");
            engine
                .register_definition(
                    Definition::from_script("READ", "Reader", reader_script)
                        .expect("reader compiles"),
                )
                .expect("reader registers");
            engine
                .register_definition(simple_definition("TARG"))
                .expect("target registers");
        };

        let mut engine = Engine::with_seed(7);
        register(&mut engine);
        let writer = engine
            .spawn_object(SpawnConfig::new("WRTR"))
            .expect("writer spawns");
        let reader = engine
            .spawn_object(SpawnConfig::new("READ"))
            .expect("reader spawns");
        let target = engine
            .spawn_object(SpawnConfig::new("TARG"))
            .expect("target spawns");
        let writer_idx = engine.find_object_index(writer).expect("writer exists");
        engine
            .call_object_function(
                writer_idx,
                "Seed",
                vec![Value::Object(target.as_u64())],
            )
            .expect("globals seed");

        let snapshot = engine.snapshot();
        assert_eq!(
            snapshot.script_globals.numbered.keys().copied().collect::<Vec<_>>(),
            vec![2, 8],
            "numbered globals stay sparse"
        );
        assert_eq!(snapshot.script_globals.named.len(), 2);
        let object_refs = Value::Array(vec![
            Value::Object(target.as_u64()),
            Value::Array(vec![Value::Object(target.as_u64())]),
        ]);
        let expected = Value::Array(vec![
            Value::Int(17),
            object_refs.clone(),
            Value::Int(23),
            object_refs,
        ]);
        let read_globals = |engine: &mut Engine| {
            let reader_idx = engine
                .find_object_index(reader)
                .expect("reader restores");
            engine
                .call_object_function(reader_idx, "Read", Vec::new())
                .expect("cross-host read succeeds")
        };

        let mut snapshot_restored = Engine::with_seed(88);
        register(&mut snapshot_restored);
        snapshot_restored
            .restore_snapshot(&snapshot)
            .expect("snapshot restores");
        assert_eq!(read_globals(&mut snapshot_restored), expected);

        let json = engine
            .capture_state()
            .to_json_string()
            .expect("state serializes");
        let state = EngineState::from_json_str(&json).expect("state deserializes");
        let mut restored = Engine::with_seed(99);
        register(&mut restored);
        restored.restore_state(&state).expect("state restores");

        let reader_idx = restored
            .find_object_index(reader)
            .expect("reader restores");
        assert_eq!(read_globals(&mut restored), expected);
        assert_eq!(
            restored
                .call_object_function(reader_idx, "Mutate", Vec::new())
                .expect("restored references stay mutable"),
            Value::Array(vec![Value::Int(18), Value::Int(25)])
        );
    }

    #[test]
    fn global_restore_denumerates_missing_objects_and_maps_declared_names() {
        // C4ValueMapData loads through a temporary saved name list, then
        // SetNameList copies only names still registered by the fresh script
        // engine (C4ValueMap.cpp:236-295). DenumeratePointer clears missing
        // object references recursively (C4Value.cpp:686-713).
        let old_script = r#"#strict
        static kept, obsolete;
        func Seed(object target) {
            Global(4) = [target, [target]];
            GlobalN("kept") = target;
            GlobalN("obsolete") = 9;
        }
        "#;
        let new_script = r#"#strict
        static kept, added;
        func Read() { return [Global(4), GlobalN("kept"), GlobalN("added")]; }
        func SetAdded() { GlobalN("added") = 5; return GlobalN("added"); }
        func SetObsolete() { GlobalN("obsolete") = 5; }
        "#;
        let mut engine = Engine::with_seed(7);
        engine
            .register_definition(
                Definition::from_script("HOLD", "Holder", old_script).expect("old script compiles"),
            )
            .expect("old holder registers");
        engine
            .register_definition(simple_definition("TARG"))
            .expect("target registers");
        let holder = engine
            .spawn_object(SpawnConfig::new("HOLD"))
            .expect("holder spawns");
        let target = engine
            .spawn_object(SpawnConfig::new("TARG"))
            .expect("target spawns");
        let holder_idx = engine.find_object_index(holder).expect("holder exists");
        engine
            .call_object_function(
                holder_idx,
                "Seed",
                vec![Value::Object(target.as_u64())],
            )
            .expect("globals seed");

        let mut state = engine.capture_state();
        state.objects.retain(|object| object.snapshot.id != target);
        let mut restored = Engine::with_seed(0);
        restored
            .register_definition(
                Definition::from_script("HOLD", "Holder", new_script).expect("new script compiles"),
            )
            .expect("new holder registers");
        restored
            .register_definition(simple_definition("TARG"))
            .expect("target definition registers");
        restored.restore_state(&state).expect("state restores");

        let holder_idx = restored
            .find_object_index(holder)
            .expect("holder restores");
        assert_eq!(
            restored
                .call_object_function(holder_idx, "Read", Vec::new())
                .expect("globals read"),
            Value::Array(vec![
                Value::Array(vec![Value::Nil, Value::Array(vec![Value::Nil])]),
                Value::Nil,
                Value::Nil,
            ])
        );
        assert_eq!(
            restored
                .call_object_function(holder_idx, "SetAdded", Vec::new())
                .expect("new declaration owns a live nil cell"),
            Value::Int(5)
        );
        assert!(
            restored
                .call_object_function(holder_idx, "SetObsolete", Vec::new())
                .is_err(),
            "a saved name absent from the fresh declaration list is dropped"
        );
    }

    #[test]
    fn object_local_restore_denumerates_missing_object_references() {
        // C++ oracle: after every object is loaded, C4Object::DenumeratePointers
        // recursively denumerates both numbered Local and LocalNamed values
        // (src/C4Object.cpp:2914-2924; src/C4Value.cpp:684-713). A saved
        // C4V_C4ObjectEnum whose object no longer exists becomes nil.
        let holder_script = r#"#strict
        local named_refs;
        func Seed(object target) {
            named_refs = [target, [target]];
            Local(2) = [target];
        }
        func Read() { return [named_refs, Local(2)]; }
        "#;
        let register = |engine: &mut Engine| {
            engine
                .register_definition(
                    Definition::from_script("HOLD", "Holder", holder_script)
                        .expect("holder compiles"),
                )
                .expect("holder registers");
            engine
                .register_definition(simple_definition("TARG"))
                .expect("target registers");
        };

        let mut engine = Engine::with_seed(7);
        register(&mut engine);
        let holder = engine
            .spawn_object(SpawnConfig::new("HOLD"))
            .expect("holder spawns");
        let target = engine
            .spawn_object(SpawnConfig::new("TARG"))
            .expect("target spawns");
        let holder_idx = engine.find_object_index(holder).expect("holder exists");
        engine
            .call_object_function(
                holder_idx,
                "Seed",
                vec![Value::Object(target.as_u64())],
            )
            .expect("locals seed");

        let mut state = engine.capture_state();
        state.objects.retain(|object| object.snapshot.id != target);

        let mut restored = Engine::with_seed(0);
        register(&mut restored);
        restored.restore_state(&state).expect("state restores");
        let holder_idx = restored
            .find_object_index(holder)
            .expect("holder restores");
        assert_eq!(
            restored
                .call_object_function(holder_idx, "Read", Vec::new())
                .expect("locals read"),
            Value::Array(vec![
                Value::Array(vec![Value::Nil, Value::Array(vec![Value::Nil])]),
                Value::Array(vec![Value::Nil]),
            ])
        );
    }

    #[test]
    fn object_restore_denumerates_missing_direct_object_pointers() {
        // C++ oracle: after all objects compile, C4Object::DenumeratePointers
        // resolves Contained, Action.Target, Action.Target2, and pLayer
        // together. A saved number absent from both live object lists becomes
        // null rather than aborting the load (src/C4Object.cpp:2914-2919;
        // src/C4EnumeratedObjectPtr.cpp:32-42).
        let mut engine = Engine::with_seed(7);
        engine
            .register_definition(simple_definition("HOLD"))
            .expect("holder registers");
        engine
            .register_definition(simple_definition("TARG"))
            .expect("target registers");
        let holder = engine
            .spawn_object(SpawnConfig::new("HOLD"))
            .expect("holder spawns");
        let target = engine
            .spawn_object(SpawnConfig::new("TARG"))
            .expect("target spawns");

        let mut state = engine.capture_state();
        let saved_holder = state
            .objects
            .iter_mut()
            .find(|object| object.snapshot.id == holder)
            .expect("saved holder exists");
        saved_holder.snapshot.container = Some(target);
        saved_holder.snapshot.action.target = Some(target);
        saved_holder.snapshot.action.target2 = Some(target);
        saved_holder.snapshot.layer = Some(target);
        state.objects.retain(|object| object.snapshot.id != target);

        let mut restored = Engine::with_seed(0);
        restored
            .register_definition(simple_definition("HOLD"))
            .expect("holder registers");
        restored
            .register_definition(simple_definition("TARG"))
            .expect("target definition registers");
        restored.restore_state(&state).expect("state restores");

        let holder = restored.object_snapshot(holder).expect("holder restores");
        assert_eq!(holder.container, None);
        assert_eq!(holder.action.target, None);
        assert_eq!(holder.action.target2, None);
        assert_eq!(holder.layer, None);
    }

    #[test]
    fn player_restore_denumerates_missing_object_pointers() {
        // C++ oracle: loading player runtime data denumerates Cursor,
        // ViewCursor, and Captain, then rebuilds Crew through DenumerateRead
        // (src/C4Player.cpp:1556-1614,1631-1633,1789-1796). Missing object
        // numbers become null and never remain visible to GetViewCursor.
        let mut engine = Engine::with_seed(7);
        engine
            .register_definition(simple_definition("HOLD"))
            .expect("holder registers");
        engine
            .register_definition(simple_definition("TARG"))
            .expect("target registers");
        engine
            .spawn_object(SpawnConfig::new("HOLD"))
            .expect("holder spawns");
        let target = engine
            .spawn_object(SpawnConfig::new("TARG"))
            .expect("target spawns");

        let mut state = engine.capture_state();
        state.players = vec![PlayerState {
            id: 1,
            cursor: Some(target),
            view_cursor: Some(target),
            captain: Some(target),
            viewports: vec![
                PlayerViewport::new(Vector2::new(12, 34)).with_focus(Some(target)),
            ],
            crew: vec![target],
            ..PlayerState::default()
        }];
        state.crew_selection.insert(
            1,
            CrewSelectionState {
                selected: vec![target],
                cursor: Some(target),
            },
        );
        state.objects.retain(|object| object.snapshot.id != target);

        let mut restored = Engine::with_seed(0);
        restored
            .register_definition(simple_definition("HOLD"))
            .expect("holder registers");
        restored
            .register_definition(simple_definition("TARG"))
            .expect("target definition registers");
        restored.restore_state(&state).expect("state restores");

        let snapshot = restored.snapshot();
        let player = snapshot
            .players
            .iter()
            .find(|player| player.id == 1)
            .expect("player restores");
        assert_eq!(player.cursor, None);
        assert_eq!(player.captain, None);
        assert!(player.crew.is_empty());
        assert_eq!(player.viewports[0].focus, None);
        assert_eq!(player.viewports[0].center, Vector2::new(12, 34));
    }

    #[test]
    fn player_restore_final_init_reseeds_only_active_missing_captains() -> Result<(), EngineError> {
        let rule = Definition::from_script("KILC", "Kill the Captain", "#strict 2")?;
        let mut crew = Definition::from_script("CREW", "Crew", "#strict 2")?;
        crew.set_crew_member(true);

        let mut engine = Engine::with_seed(0);
        engine.register_definition(rule.clone())?;
        engine.register_definition(crew.clone())?;
        engine.spawn_object(SpawnConfig::new("KILC"))?;
        let active_crew = engine.spawn_object(
            SpawnConfig::new("CREW")
                .with_owner(1)
                .with_crew_member(true),
        )?;
        engine.spawn_object(
            SpawnConfig::new("CREW")
                .with_owner(2)
                .with_crew_member(true),
        )?;
        engine.register_player(PlayerConfig::new(1, "Active"))?;
        engine.register_player(
            PlayerConfig::new(2, "Inactive").with_status(PlayerStatus::Inactive),
        )?;

        let mut state = engine.capture_state();
        let active = state
            .players
            .iter_mut()
            .find(|player| player.id == 1)
            .expect("active player saves");
        assert_eq!(active.captain, Some(active_crew));
        active.captain = None;
        assert_eq!(
            state
                .players
                .iter()
                .find(|player| player.id == 2)
                .expect("inactive player saves")
                .captain,
            None
        );

        let mut restored = Engine::with_seed(0);
        restored.register_definition(rule)?;
        restored.register_definition(crew)?;
        restored.restore_state(&state)?;
        assert_eq!(
            restored.player(1).and_then(Player::captain),
            None,
            "raw runtime compilation precedes C4Game::InitGameFinal"
        );
        restored.finalize_restored_players()?;
        assert_eq!(
            restored.player(1).and_then(Player::captain),
            Some(active_crew),
            "loaded FinalInit re-elects a missing captain while KILC is live"
        );
        assert_eq!(
            restored.player(2).and_then(Player::captain),
            None,
            "C4Player::FinalInit skips inactive players"
        );
        Ok(())
    }

    #[test]
    fn get_captain_tracks_the_shipped_kill_the_captain_rule_identity() -> Result<(), EngineError> {
        let repository = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
        let group = clonk_resources::Group::open(
            repository.join("content/Objects.c4d/Rules.c4d/KillTheCaptain.c4d"),
        )
        .expect("shipped KillTheCaptain definition opens");
        let resource = ResourceDefinitionData::load(&group)
            .expect("shipped KillTheCaptain definition loads");

        let mut engine = Engine::with_seed(0);
        engine.register_definition(Definition::from_resource(&resource)?)?;
        let mut crew = Definition::from_script("CREW", "Crew", "#strict 2")?;
        crew.set_crew_member(true);
        engine.register_definition(crew)?;
        engine.register_definition(Definition::from_script(
            "PROB",
            "Captain probe",
            r#"#strict 2
public func Read(int player)
{
    return [GetCaptain(player), GetHiRank(player),
            GetCaptain(-1), GetCaptain(99)];
}
public func RemoveCaptain(int player)
{
    RemoveObject(GetCaptain(player));
    return [GetCaptain(player), GetHiRank(player)];
}
"#,
        )?)?;
        let rule = engine.spawn_object(SpawnConfig::new("KILC"))?;
        let probe = engine.spawn_object(SpawnConfig::new("PROB"))?;

        engine.set_teams(vec![TeamInfo::new(1, "Team", 0x00f4_0000)]);
        let mut start = PlayerStart::default();
        start.ready_crew = vec![("CREW".to_string(), 2)];
        engine.set_player_starts(vec![start]);
        engine.join_player(JoinPlayerConfig {
            name: "Captain owner".to_string(),
            player_info_id: 1,
            score: 0,
            rounds: 0,
            rounds_won: 0,
            rounds_lost: 0,
            total_playing_time: 0,
            team: Some(1),
            color_dw: 0x00f4_0000,
            pref_color: 0,
            pref_position: 0,
            crew: vec![
                player_file::CrewInfo {
                    id: "CREW".to_string(),
                    name: "Runner-up".to_string(),
                    death_message: String::new(),
                    core: Default::default(),
                    rank: 1,
                    rank_name: "Ensign".to_string(),
                    experience: 1_000,
                    rounds: 0,
                    physical: PhysicalInfo::default(),
                    death_count: 0,
                    total_playing_time: 0,
                    birthday: 0,
                    age: 0,
                    participation: 1,
                    in_action: false,
                    was_in_action: false,
                    in_action_time: 0,
                    has_died: false,
                    extra_data: Vec::new(),
                    portraits: Default::default(),
                },
                player_file::CrewInfo {
                    id: "CREW".to_string(),
                    name: "Captain".to_string(),
                    death_message: String::new(),
                    core: Default::default(),
                    rank: 5,
                    rank_name: "Lieutenant Colonel".to_string(),
                    experience: 5_000,
                    rounds: 0,
                    physical: PhysicalInfo::default(),
                    death_count: 0,
                    total_playing_time: 0,
                    birthday: 0,
                    age: 0,
                    participation: 1,
                    in_action: false,
                    was_in_action: false,
                    in_action_time: 0,
                    has_died: false,
                    extra_data: Vec::new(),
                    portraits: Default::default(),
                },
            ],
            control_style: false,
            auto_context_menu: false,
            startup_player_count: 1,
        })?;

        let roster = engine.player(0).expect("player joins").crew().to_vec();
        let captain = roster
            .iter()
            .copied()
            .find(|id| engine.crew_object_info(*id).is_some_and(|info| info.rank == 5))
            .expect("highest-ranked crew exists");
        let runner_up = roster
            .iter()
            .copied()
            .find(|id| *id != captain)
            .expect("runner-up crew exists");
        let probe_index = engine.find_object_index(probe).expect("probe exists");
        let rule_index = engine.find_object_index(rule).expect("rule exists");

        assert_eq!(
            engine.call_object_function(probe_index, "Read", vec![Value::Int(0)])?,
            Value::Array(vec![
                object_reference_value(captain),
                object_reference_value(captain),
                Value::Nil,
                Value::Nil,
            ])
        );
        engine.clear_crew_selection(0);
        engine.select_crew(0, [runner_up])?;
        assert_eq!(
            engine.call_object_function(probe_index, "Read", vec![Value::Int(0)])?,
            Value::Array(vec![
                object_reference_value(captain),
                object_reference_value(captain),
                Value::Nil,
                Value::Nil,
            ]),
            "selection changes do not replace the stored captain"
        );
        engine.call_object_function(rule_index, "Execute", Vec::new())?;
        assert_eq!(
            engine.player(0).expect("player remains registered").status(),
            PlayerStatus::Active,
            "the shipped check keeps a player whose captain is present"
        );
        assert_eq!(
            engine.call_object_function(
                probe_index,
                "RemoveCaptain",
                vec![Value::Int(0)],
            )?,
            Value::Array(vec![Value::Nil, object_reference_value(runner_up)]),
            "ClearPointers nulls Captain without electing the surviving crew"
        );

        engine.call_object_function(rule_index, "Execute", Vec::new())?;
        assert_eq!(
            engine.player(0).expect("player remains registered").status(),
            PlayerStatus::Eliminated,
            "the shipped rule completes its captain check without a runtime abort"
        );
        assert!(engine.find_object_index(rule).is_some(), "rule remains live");
        Ok(())
    }

    #[test]
    fn removed_player_view_pointers_clear_and_cursor_mode_falls_back() {
        // C4Player::ClearPointers nulls ViewTarget/ViewCursor synchronously;
        // target mode keeps its last ViewX/Y, while the next player-phase
        // UpdateView in cursor mode falls back ViewCursor -> Cursor
        // (C4Player.cpp:55-73,917-928,1693-1713).
        let script = r#"
        func Configure(int player, object cursor, object view_cursor, object target) {
            SetCursor(player, cursor, true, true, true);
            SetViewCursor(player, view_cursor);
            return SetPlrView(player, target);
        }
        func AimAndRemove(int player) {
            SetCursor(player, this(), true, true, true);
            SetPlrView(player, this());
            RemoveObject();
            return GetPlrView(player);
        }
        "#;
        let mut engine = Engine::with_seed(7);
        engine
            .register_definition(
                Definition::from_script("CAMR", "Camera", script).expect("camera compiles"),
            )
            .expect("camera registers");
        engine
            .register_definition(simple_definition("TARG"))
            .expect("target registers");
        let caller = engine
            .spawn_object(SpawnConfig::new("CAMR"))
            .expect("camera caller spawns");
        let removed_same_call = engine
            .spawn_object(SpawnConfig::new("CAMR"))
            .expect("same-call target spawns");
        let cursor = engine
            .spawn_object(SpawnConfig::new("TARG").with_position(Vector2::new(10, 20)))
            .expect("cursor spawns");
        let view_cursor = engine
            .spawn_object(SpawnConfig::new("TARG").with_position(Vector2::new(30, 40)))
            .expect("view cursor spawns");
        let target = engine
            .spawn_object(SpawnConfig::new("TARG").with_position(Vector2::new(50, 60)))
            .expect("view target spawns");
        engine
            .register_player(PlayerConfig::new(15, "Viewer"))
            .expect("player registers");
        let removed_index = engine
            .find_object_index(removed_same_call)
            .expect("same-call target exists");
        assert_eq!(
            engine
                .call_object_function(removed_index, "AimAndRemove", vec![Value::Int(15)])
                .expect("same-call target removes"),
            Value::Nil
        );
        let cleared = engine
            .snapshot()
            .players
            .into_iter()
            .find(|state| state.id == 15)
            .expect("player remains");
        assert_eq!(cleared.view_mode, PLAYER_VIEW_MODE_TARGET);
        assert_eq!(cleared.cursor, None);
        assert_eq!(
            cleared.view_target, None,
            "SetPlrView -> RemoveObject must clear the engine pointer in the same callback fold"
        );
        assert!(
            engine
                .snapshot()
                .crew_selection
                .get(&15)
                .and_then(|selection| selection.cursor)
                .is_none(),
            "removed Cursor must not resurrect through authoritative crew_selection"
        );
        let caller_index = engine.find_object_index(caller).expect("caller exists");
        assert_eq!(
            engine
                .call_object_function(
                    caller_index,
                    "Configure",
                    vec![
                        Value::Int(15),
                        Value::Object(cursor.as_u64()),
                        Value::Object(view_cursor.as_u64()),
                        Value::Object(target.as_u64()),
                    ],
                )
                .expect("camera configures"),
            Value::Bool(true)
        );
        engine.tick_player_systems().expect("player view updates");
        let view = engine
            .snapshot()
            .players
            .into_iter()
            .find(|state| state.id == 15)
            .expect("player remains");
        assert_eq!(view.view_mode, PLAYER_VIEW_MODE_TARGET);
        assert_eq!(view.view_target, Some(target));
        assert_eq!(view.viewports[0].focus, Some(view_cursor));
        assert_eq!(view.viewports[0].center, Vector2::new(50, 60));

        engine
            .apply_object_update(
                target,
                ObjectUpdate::new().with_status(ObjectStatus::Deleted),
            )
            .expect("target removes");
        let view = engine
            .snapshot()
            .players
            .into_iter()
            .find(|state| state.id == 15)
            .expect("player remains");
        assert_eq!(view.view_mode, PLAYER_VIEW_MODE_TARGET);
        assert_eq!(view.view_target, None);
        assert_eq!(view.viewports[0].center, Vector2::new(50, 60));

        engine
            .player_in_com(15, COM_LEFT, 0)
            .expect("non-release input resets view mode");
        engine.tick_player_systems().expect("cursor view updates");
        let view = engine
            .snapshot()
            .players
            .into_iter()
            .find(|state| state.id == 15)
            .expect("player remains");
        assert_eq!(view.view_mode, PLAYER_VIEW_MODE_CURSOR);
        assert_eq!(view.viewports[0].center, Vector2::new(30, 40));

        engine
            .apply_object_update(
                view_cursor,
                ObjectUpdate::new().with_status(ObjectStatus::Deleted),
            )
            .expect("view cursor removes");
        let view = engine
            .snapshot()
            .players
            .into_iter()
            .find(|state| state.id == 15)
            .expect("player remains");
        assert_eq!(view.view_cursor, None);
        assert_eq!(view.viewports[0].focus, Some(cursor));
        assert_eq!(view.viewports[0].center, Vector2::new(30, 40));
        engine.tick_player_systems().expect("fallback view updates");
        let view = engine
            .snapshot()
            .players
            .into_iter()
            .find(|state| state.id == 15)
            .expect("player remains");
        assert_eq!(view.viewports[0].center, Vector2::new(10, 20));
    }

    #[test]
    fn object_local_restore_maps_values_to_current_definition_names() {
        // C++ oracle: LocalNamed first compiles the saved names into a
        // temporary C4ValueMapNames, then C4Object::CompileFunc switches to
        // Def->Script.LocalNamed. OnNameListChanged copies values by name,
        // so removed declarations disappear and reordered names keep their
        // values (src/C4ValueMap.cpp:163-195,236-293;
        // src/C4Object.cpp:2815,2858-2865).
        let old_script = r#"
        local kept, obsolete;
        func Seed() {
            kept = 17;
            obsolete = 23;
            Local(2) = 31;
        }
        "#;
        let new_script = r#"#strict
        local added, kept;
        func Read() { return [kept, added, Local(2)]; }
        "#;

        let mut engine = Engine::with_seed(7);
        engine
            .register_definition(
                Definition::from_script("HOLD", "Holder", old_script)
                    .expect("old holder compiles"),
            )
            .expect("old holder registers");
        let holder = engine
            .spawn_object(SpawnConfig::new("HOLD"))
            .expect("holder spawns");
        let holder_idx = engine.find_object_index(holder).expect("holder exists");
        engine
            .call_object_function(holder_idx, "Seed", Vec::new())
            .expect("locals seed");
        let state = engine.capture_state();

        let mut restored = Engine::with_seed(0);
        restored
            .register_definition(
                Definition::from_script("HOLD", "Holder", new_script)
                    .expect("new holder compiles"),
            )
            .expect("new holder registers");
        restored.restore_state(&state).expect("state restores");

        let local_vars = &restored
            .object_snapshot(holder)
            .expect("holder restores")
            .local_vars;
        assert_eq!(local_vars.get("kept"), Some(&Value::Int(17)));
        assert_eq!(local_vars.get("__local_2"), Some(&Value::Int(31)));
        assert!(
            !local_vars.contains_key("obsolete"),
            "a saved name absent from Def->Script.LocalNamed is dropped"
        );
        let holder_idx = restored
            .find_object_index(holder)
            .expect("holder restores");
        assert_eq!(
            restored
                .call_object_function(holder_idx, "Read", Vec::new())
                .expect("remapped locals read"),
            Value::Array(vec![Value::Int(17), Value::Nil, Value::Int(31)])
        );
    }

    #[test]
    fn effect_restore_denumerates_command_targets_and_variables() {
        // C++ oracle: C4Effect::DenumeratePointers resolves the command target
        // and recursively denumerates EffectVars (src/C4Effect.cpp:186-198).
        // Object effects run that pass from C4Object.cpp:2914-2930; global
        // effects run it from C4Game.cpp:2491-2494.
        let mut engine = Engine::with_seed(7);
        engine
            .register_definition(simple_definition("HOLD"))
            .expect("holder registers");
        engine
            .register_definition(simple_definition("TARG"))
            .expect("target registers");
        let holder = engine
            .spawn_object(SpawnConfig::new("HOLD"))
            .expect("holder spawns");
        let target = engine
            .spawn_object(SpawnConfig::new("TARG"))
            .expect("target spawns");

        let saved_effect = EffectState::new("SavedRefs")
            .with_command_target(Some(target.as_u64() as i32))
            .with_vars(vec![
                EffectVarValue::Object(target.as_u64()),
                EffectVarValue::Array(vec![
                    EffectVarValue::Object(target.as_u64()),
                    EffectVarValue::Object(holder.as_u64()),
                ]),
            ]);
        let mut state = engine.capture_state();
        state
            .objects
            .iter_mut()
            .find(|object| object.snapshot.id == holder)
            .expect("saved holder exists")
            .snapshot
            .effects = vec![saved_effect.clone()];
        state.global_effects = vec![saved_effect];
        state.objects.retain(|object| object.snapshot.id != target);

        let mut restored = Engine::with_seed(0);
        restored
            .register_definition(simple_definition("HOLD"))
            .expect("holder registers");
        restored
            .register_definition(simple_definition("TARG"))
            .expect("target registers");
        restored.restore_state(&state).expect("state restores");

        let expected_vars = vec![
            EffectVarValue::Nil,
            EffectVarValue::Array(vec![
                EffectVarValue::Nil,
                EffectVarValue::Object(holder.as_u64()),
            ]),
        ];
        let object_effect = &restored
            .object_snapshot(holder)
            .expect("holder restores")
            .effects[0];
        assert_eq!(object_effect.command_target, None);
        assert_eq!(object_effect.vars(), expected_vars);
        assert_eq!(restored.global_effects()[0].command_target, None);
        assert_eq!(restored.global_effects()[0].vars(), expected_vars);
    }

    #[test]
    fn command_restore_denumerates_missing_target_and_target2() {
        // C++ oracle: C4Command persists Target and Target2 as enumerated
        // object pointers, then C4Command::DenumeratePointers resolves both
        // after every object has loaded (src/C4Command.cpp:2393-2421;
        // src/C4Object.cpp:2914-2929). Missing objects therefore restore as
        // null command fields rather than retaining their saved object number.
        let script = r#"
        func Arm(object target) {
            return SetCommand(this(), "MoveTo", target, 0, 0, target);
        }
        "#;
        let register = |engine: &mut Engine| {
            engine
                .register_definition(
                    Definition::from_script("HOLD", "Holder", script)
                        .expect("holder compiles"),
                )
                .expect("holder registers");
            engine
                .register_definition(simple_definition("TARG"))
                .expect("target registers");
        };

        let mut engine = Engine::with_seed(7);
        register(&mut engine);
        let holder = engine
            .spawn_object(SpawnConfig::new("HOLD"))
            .expect("holder spawns");
        let target = engine
            .spawn_object(SpawnConfig::new("TARG"))
            .expect("target spawns");
        let holder_idx = engine.find_object_index(holder).expect("holder exists");
        assert_eq!(
            engine
                .call_object_function(holder_idx, "Arm", vec![Value::Object(target.as_u64())])
                .expect("command arms"),
            Value::Bool(true)
        );

        let mut state = engine.capture_state();
        state.objects.retain(|object| object.snapshot.id != target);

        let mut restored = Engine::with_seed(0);
        register(&mut restored);
        restored.restore_state(&state).expect("state restores");
        let commands = restored
            .object_snapshot(holder)
            .expect("holder restores")
            .command_stack
            .command_views();
        assert_eq!(commands.len(), 1);
        assert_eq!(commands[0].target, None);
        assert_eq!(commands[0].target2, None);
    }

    #[test]
    fn legacy_state_without_script_globals_restores_defaults() {
        // Saves from before script-global persistence have no corresponding
        // C4AulScriptEngine section. Treat that omission as empty `Globals`
        // and nil values for every declaration in the fresh GlobalNamed name
        // list (C4Aul.cpp:513-520; C4ValueMap.cpp:236-295).
        let script = r#"#strict
        static named;
        func Seed() { Global(3) = 11; GlobalN("named") = 12; }
        func Read() { return [Global(3), GlobalN("named")]; }
        "#;
        let register = |engine: &mut Engine| {
            engine
                .register_definition(
                    Definition::from_script("HOLD", "Holder", script)
                        .expect("holder compiles"),
                )
                .expect("holder registers");
        };

        let mut saved = Engine::with_seed(7);
        register(&mut saved);
        let holder = saved
            .spawn_object(SpawnConfig::new("HOLD"))
            .expect("holder spawns");
        let mut legacy: serde_json::Value = serde_json::from_str(
            &saved
                .capture_state()
                .to_json_string()
                .expect("state serializes"),
        )
        .expect("state JSON parses");
        legacy
            .as_object_mut()
            .expect("engine state is an object")
            .remove("script_globals");
        let legacy: EngineState =
            serde_json::from_value(legacy).expect("legacy state without globals parses");
        assert!(legacy.script_globals.is_empty());

        let mut restored = Engine::with_seed(99);
        register(&mut restored);
        let stale_holder = restored
            .spawn_object(SpawnConfig::new("HOLD"))
            .expect("stale holder spawns");
        let stale_idx = restored
            .find_object_index(stale_holder)
            .expect("stale holder exists");
        restored
            .call_object_function(stale_idx, "Seed", Vec::new())
            .expect("stale globals seed");

        restored.restore_state(&legacy).expect("legacy state restores");
        let holder_idx = restored
            .find_object_index(holder)
            .expect("saved holder restores");
        assert_eq!(
            restored
                .call_object_function(holder_idx, "Read", Vec::new())
                .expect("default globals read"),
            Value::Array(vec![Value::Nil, Value::Nil]),
            "omitted old-save globals clear stale numbered and named values"
        );
    }

    #[test]
    fn declaring_host_global_link_inherits_engine_overload() {
        let mut engine = Engine::with_seed(7);
        assert_eq!(
            engine.install_global_scripts(&[(
                "System.c4g/Base.c".to_string(),
                "global func Override() { return 3; }".to_string(),
            )]),
            1
        );
        engine
            .register_definition(
                Definition::from_script(
                    "TSTD",
                    "Test",
                    "#strict\nglobal func Override() { return inherited() * 10 + 4; }\n\
                     func Probe() { return Override(); }",
                )
                .expect("script compiles"),
            )
            .expect("definition registers");

        let id = engine
            .spawn_object(SpawnConfig::new("TSTD"))
            .expect("object spawns");
        let idx = engine.find_object_index(id).expect("object exists");
        assert_eq!(
            engine
                .call_object_function(idx, "Probe", Vec::new())
                .expect("Probe runs"),
            Value::Int(34),
            "the declaration-site FnLink carries the engine overload chain"
        );
    }

    #[test]
    fn rejected_duplicate_definition_does_not_leak_globals() {
        let mut engine = Engine::with_seed(7);
        engine
            .register_definition(
                Definition::from_script("TSTD", "First", "func Probe() { return 1; }")
                    .expect("first script compiles"),
            )
            .expect("first definition registers");

        let duplicate = Definition::from_script(
            "TSTD",
            "Duplicate",
            "global func LeakedFromRejectedDefinition() { return 99; }",
        )
        .expect("duplicate script compiles");
        assert!(matches!(
            engine.register_definition(duplicate),
            Err(EngineError::DefinitionAlreadyExists(id)) if id == "TSTD"
        ));
        assert!(
            !engine
                .global_script_functions
                .as_deref()
                .is_some_and(|functions| functions.contains_key("LeakedFromRejectedDefinition"))
        );
    }

    #[test]
    fn system_static_consts_register_globally_and_later_scripts_overwrite() {
        // System.c4g scripts are children of Game.ScriptEngine and their
        // static const declarations go through RegisterGlobalConstant just
        // like definition scripts (C4Aul.cpp:484-492). A later declaration
        // overwrites the shared value, including for already-linked hosts.
        let definition = Definition::from_script(
            "TSTD",
            "Test",
            "#strict\nstatic const SYSTEM_VALUE = 23;\n\
             func Probe() { return(SYSTEM_VALUE()); }\n",
        )
        .expect("definition compiles");
        let mut engine = Engine::with_seed(7);
        assert_eq!(
            engine.install_global_scripts(&[(
                "System.c4g/First.c".to_string(),
                "#strict\nstatic const SYSTEM_VALUE = 17;\n".to_string(),
            )]),
            1
        );
        engine
            .register_definition(definition)
            .expect("definition registers");

        let id = engine
            .spawn_object(SpawnConfig::new("TSTD"))
            .expect("object spawns");
        let idx = engine.find_object_index(id).expect("object exists");
        assert_eq!(
            engine
                .call_object_function(idx, "Probe", Vec::new())
                .expect("definition constant resolves"),
            Value::Int(23)
        );

        assert_eq!(
            engine.install_scenario_global_scripts(&[(
                "Scenario/System.c4g/Override.c".to_string(),
                "#strict\nstatic const SYSTEM_VALUE = 42;\n".to_string(),
            )]),
            1
        );
        let idx = engine.find_object_index(id).expect("object exists");
        assert_eq!(
            engine
                .call_object_function(idx, "Probe", Vec::new())
                .expect("overridden constant resolves"),
            Value::Int(42)
        );
    }

    #[test]
    fn definition_call_falls_back_to_global_functions() {
        // `id->Func(...)` (AB_CALL) resolves via FindSameNameFunc: the
        // def's own function first, else a GLOBAL script function running
        // in definition scope (C4AulExec.cpp:1259-1261, C4Aul.cpp:130-148).
        // System.c4g Explode.c relies on it: the exploding object removes
        // itself, then runs `exploding_id->DoExplosion(...)`.
        let def_script = r#"#strict
local iGot;
func Trigger() {
    iGot = GetID()->GlobalHelper(6);
    return(1);
}
"#;
        let mut engine = Engine::with_seed(0);
        engine
            .register_definition(
                Definition::from_script("TSTD", "Test", def_script).expect("script compiles"),
            )
            .expect("definition registers");
        let loaded = engine.install_global_scripts(&[(
            "System.c4g/Test.c".to_string(),
            "#strict\nglobal func GlobalHelper(n) { return(n * 7); }\n".to_string(),
        )]);
        assert_eq!(loaded, 1);

        let id = engine
            .spawn_object(SpawnConfig::new("TSTD").with_category(CATEGORY_OBJECT))
            .expect("object spawns");
        let idx = engine.find_object_index(id).expect("object exists");
        engine
            .call_object_function(idx, "Trigger", Vec::new())
            .expect("trigger runs");
        let idx = engine.find_object_index(id).expect("object exists");
        assert_eq!(
            engine.objects[idx].state.local_vars.get("iGot"),
            Some(&Value::Int(42)),
            "the global function runs as a definition call"
        );
    }

    // FnIncinerateLandscape (C4Script.cpp:253-261) -> C4Landscape::
    // Incinerate (C4Landscape.cpp:1430-1441): inflammable material at the
    // (caller-relative) point spawns one FLAM unless another FLAM already
    // burns in the (x-4, y-1, 8, 20) rect (C4Game::FindObject range check,
    // position-in-rect); sky/solid-rock points return false.
    #[test]
    fn incinerate_landscape_spawns_one_flam_on_inflammable_material() {
        let caller_script = r#"#strict
local iFirst, iSecond, iSky;
func Trigger() {
    iFirst = IncinerateLandscape(8 - GetX(), 45 - GetY());
    iSecond = IncinerateLandscape(8 - GetX(), 45 - GetY());
    iSky = IncinerateLandscape(8 - GetX(), 2 - GetY());
    return(1);
}
"#;
        let library = MaterialLibrary::parse(
            r#"
            [Material Oil]
            Name=Oil
            Density=100
            Friction=25
            Inflammable=-1
        "#,
        )
        .expect("material library parses");
        let materials = MaterialSet::from_resource_library(&library);
        let oil = materials.id_of("Oil").expect("oil exists");
        let mut engine = Engine::with_seed(0);
        engine.set_materials(materials);
        let mut landscape = Landscape::flat_with_material(17, 40, Some(oil));
        landscape.set_world_height(80);
        engine.set_landscape(landscape);
        engine
            .register_definition(
                Definition::from_script("FLAM", "Fire", "#strict\n").expect("flam compiles"),
            )
            .expect("flam registers");
        engine
            .register_definition(
                Definition::from_script("CALL", "Caller", caller_script).expect("caller compiles"),
            )
            .expect("caller registers");

        let id = engine
            .spawn_object(SpawnConfig::new("CALL").with_category(CATEGORY_OBJECT))
            .expect("caller spawns");
        let idx = engine.find_object_index(id).expect("caller exists");
        engine
            .call_object_function(idx, "Trigger", Vec::new())
            .expect("trigger runs");

        let idx = engine.find_object_index(id).expect("caller exists");
        let locals = &engine.objects[idx].state.local_vars;
        assert_eq!(
            locals.get("iFirst"),
            Some(&Value::Bool(true)),
            "inflammable point incinerates"
        );
        assert_eq!(
            locals.get("iSecond"),
            Some(&Value::Bool(false)),
            "a FLAM already burning in the rect blocks the second"
        );
        assert_eq!(
            locals.get("iSky"),
            Some(&Value::Bool(false)),
            "sky point does not incinerate"
        );
        let flams = engine
            .objects
            .iter()
            .filter(|object| object.definition_id == "FLAM" && !object.destroyed)
            .count();
        assert_eq!(flams, 1, "exactly one FLAM spawned");
    }

    // FnCreateConstruction takes a C4ID first parameter
    // (C4Script.cpp:1911-1912) — DoExplosion's `CreateConstruction(FXB1,
    // x, y+level, cause_plr, level*5)` passes the id value directly.
    #[test]
    fn create_construction_accepts_id_values() {
        let caller_script = r#"#strict
local aSite;
func Trigger() {
    aSite = CreateConstruction(BLST, 0, 0, -1, 50);
    return(1);
}
"#;
        let mut engine = Engine::with_seed(0);
        engine
            .register_definition(
                Definition::from_script("BLST", "Blast", "#strict\n").expect("blast compiles"),
            )
            .expect("blast registers");
        engine
            .register_definition(
                Definition::from_script("CALL", "Caller", caller_script).expect("caller compiles"),
            )
            .expect("caller registers");

        let id = engine
            .spawn_object(SpawnConfig::new("CALL").with_category(CATEGORY_OBJECT))
            .expect("caller spawns");
        let idx = engine.find_object_index(id).expect("caller exists");
        engine
            .call_object_function(idx, "Trigger", Vec::new())
            .expect("trigger runs");

        let site = engine
            .objects
            .iter()
            .find(|object| object.definition_id == "BLST")
            .expect("construction site exists");
        assert_eq!(
            site.state.construction,
            FULL_CON / 2,
            "iCompletion * FullCon / 100 (C4Script.cpp:1930)"
        );
    }

    #[test]
    fn create_construction_finishes_new_object_lifecycle_before_return() {
        // FnCreateConstruction calls C4Game::CreateObjectConstruction, which
        // enters C4Game::NewObject: Construction runs at Con=0 and the supplied
        // bottom position, initial DoCon keeps that bottom fixed, then a FullCon
        // transition calls Completion and Initialize before the host function
        // returns (C4Script.cpp:1911-1934; C4Game.cpp:1180-1212,1110-1129;
        // C4Object.cpp:1432-1518). LastWill immediately reads the basement local
        // created by CST3's inherited Construction callback.
        let site_script = r#"#strict
local child, construction_y, completion_y, initialize_y;
func Construction(object creator) {
    construction_y = GetY();
    child = CreateObject(CHLD, 0, 8, GetOwner());
    return true;
}
func Completion() { completion_y = GetY(); return true; }
func Initialize() { initialize_y = GetY(); return true; }
func Probe() {
    return [GetCon(), GetY(), construction_y, completion_y, initialize_y, child, GetY(child)];
}
"#;
        let caller_script = r#"#strict
local result;
func Trigger() {
    var site = CreateConstruction(SITE, 100, 200, -1, 100, false, false);
    result = site->Probe();
    return true;
}
"#;
        let mut engine = Engine::with_seed(0);
        let mut site =
            Definition::from_script("SITE", "Site", site_script).expect("site compiles");
        site.set_shape_rect(Some(DefinitionRect::new(-10, -20, 20, 40)));
        engine.register_definition(site).expect("site registers");
        engine
            .register_definition(
                Definition::from_script("CHLD", "Child", "#strict\n")
                    .expect("child compiles"),
            )
            .expect("child registers");
        engine
            .register_definition(
                Definition::from_script("CALL", "Caller", caller_script)
                    .expect("caller compiles"),
            )
            .expect("caller registers");

        let caller = engine
            .spawn_object(SpawnConfig::new("CALL").with_category(CATEGORY_OBJECT))
            .expect("caller spawns");
        let caller_index = engine.find_object_index(caller).expect("caller exists");
        engine
            .call_object_function(caller_index, "Trigger", Vec::new())
            .expect("construction trigger runs");

        let child = engine
            .objects
            .iter()
            .find(|object| object.definition_id == "CHLD")
            .expect("Construction synchronously creates its child");
        let caller = engine.object_snapshot(caller).expect("caller remains");
        assert_eq!(
            caller.local_vars.get("result"),
            Some(&Value::Array(vec![
                Value::Int(100),
                Value::Int(180),
                Value::Int(200),
                Value::Int(180),
                Value::Int(180),
                Value::Object(child.id.as_u64()),
                Value::Int(208),
            ]))
        );
    }

    // C4Id2Def failure means NO object and a silent nullptr return:
    // C4Game::CreateObject (C4Game.cpp:1146), CreateObjectConstruction
    // (C4Game.cpp:1183). Goldrush's explosion chain hits it - FXB1 is
    // referenced by System.c4g Explode.c but not loaded by the scenario.
    #[test]
    fn create_object_and_construction_return_nil_for_unknown_definitions() {
        let caller_script = r#"#strict
local aObj, aSite;
func Trigger() {
    aObj = CreateObject(FXB1, 0, 0, -1);
    aSite = CreateConstruction(FXB1, 0, 0, -1, 50);
    return(1);
}
"#;
        let mut engine = Engine::with_seed(0);
        engine
            .register_definition(
                Definition::from_script("CALL", "Caller", caller_script).expect("caller compiles"),
            )
            .expect("caller registers");

        let id = engine
            .spawn_object(SpawnConfig::new("CALL").with_category(CATEGORY_OBJECT))
            .expect("caller spawns");
        let idx = engine.find_object_index(id).expect("caller exists");
        engine
            .call_object_function(idx, "Trigger", Vec::new())
            .expect("trigger runs");

        let idx = engine.find_object_index(id).expect("caller exists");
        let locals = &engine.objects[idx].state.local_vars;
        assert_eq!(locals.get("aObj"), Some(&Value::Nil), "CreateObject nil");
        assert_eq!(
            locals.get("aSite"),
            Some(&Value::Nil),
            "CreateConstruction nil"
        );
        assert_eq!(engine.objects.len(), 1, "no spawn registered");
    }

    // FnDistance (C4Script.cpp:3316-3319) -> Distance (C4Math.cpp:22-31):
    // integer euclidean distance with the exact post-sqrt adjustment;
    // FnSetViewOffset (C4Script.cpp:5676-5687): ValidPlr gate, then true
    // even without a viewport (sync safety — the headless C++ path).
    // Both run in System.c4g's FxShakeEffectTimer (Explode.c:188-200).
    #[test]
    fn distance_and_set_view_offset_match_cpp() {
        let caller_script = r#"#strict
local iPyth, iDiag, iZero, iBadPlr, iGoodPlr;
func Trigger() {
    iPyth = Distance(0, 0, 3, 4);
    iDiag = Distance(0, 0, 1, 1);
    iZero = Distance(-7, 9, -7, 9);
    iBadPlr = SetViewOffset(9, 5, 5);
    iGoodPlr = SetViewOffset(1, 5, 5);
    return(1);
}
"#;
        let mut engine = Engine::with_seed(0);
        engine
            .register_player(PlayerConfig::new(1, "P1"))
            .expect("player registers");
        engine
            .register_definition(
                Definition::from_script("CALL", "Caller", caller_script).expect("caller compiles"),
            )
            .expect("caller registers");

        let id = engine
            .spawn_object(SpawnConfig::new("CALL").with_category(CATEGORY_OBJECT))
            .expect("caller spawns");
        let idx = engine.find_object_index(id).expect("caller exists");
        engine
            .call_object_function(idx, "Trigger", Vec::new())
            .expect("trigger runs");

        let idx = engine.find_object_index(id).expect("caller exists");
        let locals = &engine.objects[idx].state.local_vars;
        assert_eq!(locals.get("iPyth"), Some(&Value::Int(5)));
        assert_eq!(
            locals.get("iDiag"),
            Some(&Value::Int(1)),
            "sqrt(2) adjusts down to the floor (C4Math.cpp:28-30)"
        );
        assert_eq!(locals.get("iZero"), Some(&Value::Int(0)));
        assert_eq!(
            locals.get("iBadPlr"),
            Some(&Value::Bool(false)),
            "invalid player rejected"
        );
        assert_eq!(
            locals.get("iGoodPlr"),
            Some(&Value::Bool(true)),
            "no viewport is sync-safe true"
        );
    }

    #[test]
    fn call_runs_own_def_script_function_like_cpp() {
        // FnCall (C4Script.cpp:3424-3432): Call(name, p0..p8) runs `name` on
        // the calling object itself (C4Object::Call → own def script,
        // AA_PRIVATE, C4Object.cpp:2197-2201). Resolution is owner-scoped
        // script functions ONLY — engine (host) functions are never found
        // (GetSFunc → FuncLookUp.GetFunc(name, owner), C4Aul.cpp:295-298,
        // 562-576) — and a missing function returns C4VNull either way ('~'
        // only silences the log, C4Aul.cpp:314-330). Runtime errors in the
        // callee propagate (fPassErrors=true, C4Script.cpp:3431).
        let script = r#"
        func Helper(a, b) {
            DoEnergy(0 - a);
            return a * b;
        }
        global func Run() { return Call("Helper", 6, 7); }
        global func RunMissing() { return Call("NoSuchHelper"); }
        global func RunFailsafe() { return Call("~NoSuchHelper"); }
        global func RunEngineFn() { return Call("GetWind"); }
        "#;

        let mut definition =
            Definition::from_script("CLLR", "Caller", script).expect("script compiles");
        definition.set_physical(PhysicalInfo {
            energy: 100_000,
            ..PhysicalInfo::default()
        });
        let mut engine = Engine::with_seed(7);
        engine
            .register_definition(definition)
            .expect("definition registers");
        let id = engine
            .spawn_object(SpawnConfig::new("CLLR").with_energy(50))
            .expect("object spawns");
        engine.tick_without_snapshot().expect("tick succeeds");

        let idx = engine.find_object_index(id).expect("object exists");
        let energy_before = engine.objects[idx].state.energy;
        let result = engine
            .call_object_function(idx, "Run", Vec::new())
            .expect("Run succeeds");
        assert_eq!(result, Value::Int(42), "callee return value verbatim");
        let idx = engine.find_object_index(id).expect("object exists");
        assert!(
            engine.objects[idx].state.energy < energy_before,
            "the callee's DoEnergy on `this` committed (live scope, in-place path)"
        );

        for function in ["RunMissing", "RunFailsafe", "RunEngineFn"] {
            let result = engine
                .call_object_function(idx, function, Vec::new())
                .expect("call-family misses are not errors");
            assert_eq!(
                result,
                Value::Nil,
                "{function}: missing/engine-only functions return C4VNull"
            );
        }
    }

    #[test]
    fn arrow_calls_resolve_on_the_target_object_like_cpp() {
        // `obj->Method(args)` is AB_CALL (C4AulExec.cpp:1216-1305): the
        // function resolves on the TARGET's def via FindSameNameFunc
        // (C4Aul.cpp:130-148 — own script functions, then global/engine
        // functions running with the TARGET's context). `->~` forgives only
        // a MISSING FUNCTION (:1262-1267); a falsy target throws even for
        // `->~` (:1224-1226).
        let caller_script = r#"
        global func Poke(target) { return target->Secret(21); }
        global func PokeMissing(target) { return target->NoSuch(); }
        global func PokeMissingSafe(target) { return target->~NoSuch(); }
        global func PokeNil() { var no_target; return no_target->~Anything(); }
        global func PokeEngineFn(target) { return target->GetID(); }
        global func PokeNamespaced(target) { return target->OTHR::Secret(22); }
        global func PokeNamespacedMissing(target) { return target->OTHR::NamedOnly(); }
        global func PokeNamespacedGlobal(target) { return target->OTHR::GlobalOnly(); }
        global func AssignNamespaced(target) { target->OTHR::Slot() = 9; return 1; }
        "#;
        let probe_script = r#"
        local tag, target_slot;
        public func Secret(v) {
            tag = v;
            return v * 2;
        }
        public func &Slot() { return target_slot; }
        "#;
        let namespace_script = r#"
        local named_slot;
        public func Secret(v) { return v + 1000; }
        public func NamedOnly() { return 77; }
        public func GlobalOnly() { return 99; }
        public func &Slot() { return named_slot; }
        "#;

        let mut engine = Engine::with_seed(7);
        engine
            .register_definition(
                Definition::from_script("CLLR", "Caller", caller_script).expect("caller compiles"),
            )
            .expect("caller registers");
        engine
            .register_definition(
                Definition::from_script("PROB", "Probe", probe_script).expect("probe compiles"),
            )
            .expect("probe registers");
        engine
            .register_definition(
                Definition::from_script("OTHR", "Namespace", namespace_script)
                    .expect("namespace compiles"),
            )
            .expect("namespace registers");
        assert_eq!(
            engine.install_global_scripts(&[(
                "System.c4g/NamespaceCall.c".to_string(),
                "global func GlobalOnly() { return GetID(); }".to_string(),
            )]),
            1
        );
        let caller = engine
            .spawn_object(SpawnConfig::new("CLLR"))
            .expect("caller spawns");
        let probe = engine
            .spawn_object(SpawnConfig::new("PROB"))
            .expect("probe spawns");
        let namespace_target = engine
            .spawn_object(SpawnConfig::new("OTHR"))
            .expect("namespace target spawns");
        engine.tick_without_snapshot().expect("tick succeeds");

        let caller_idx = engine.find_object_index(caller).expect("caller exists");
        let target_arg = vec![Value::Object(probe.as_u64())];

        // Resolves Secret on PROB (the caller has no Secret) and runs it
        // with PROB's context: its local var commits.
        let result = engine
            .call_object_function(caller_idx, "Poke", target_arg.clone())
            .expect("arrow call succeeds");
        assert_eq!(result, Value::Int(42));
        let probe_idx = engine.find_object_index(probe).expect("probe exists");
        assert_eq!(
            engine.objects[probe_idx].state.local_vars.get("tag"),
            Some(&Value::Int(21)),
            "the callee ran on the TARGET object"
        );

        // Engine functions resolve through the FindSameNameFunc global
        // fallback and run with the TARGET's context.
        let result = engine
            .call_object_function(caller_idx, "PokeEngineFn", target_arg.clone())
            .expect("engine-fn arrow call succeeds");
        assert_eq!(result, Value::C4Id("PROB".into()), "GetID of the TARGET");

        // AB_CALLNS is ignored by the C++ executor. The paired AB_CALL
        // therefore re-resolves the parse-time function name on the target:
        // PROB's override wins over OTHR's same-name function.
        let result = engine
            .call_object_function(caller_idx, "PokeNamespaced", target_arg.clone())
            .expect("namespaced arrow call succeeds");
        assert_eq!(result, Value::Int(44));
        let probe_idx = engine.find_object_index(probe).expect("probe exists");
        assert_eq!(
            engine.objects[probe_idx].state.local_vars.get("tag"),
            Some(&Value::Int(22)),
            "the target override, not the namespaced definition, ran"
        );

        engine
            .call_object_function(caller_idx, "PokeNamespacedMissing", target_arg.clone())
            .expect_err("the namespace's function cannot satisfy a target-def miss");

        let result = engine
            .call_object_function(caller_idx, "PokeNamespacedGlobal", target_arg.clone())
            .expect("namespaced arrow call retains the global fallback");
        assert_eq!(
            result,
            Value::C4Id("PROB".into()),
            "the global fallback ran in the target object's scope"
        );

        engine
            .call_object_function(caller_idx, "AssignNamespaced", target_arg.clone())
            .expect("namespaced reference call succeeds");
        let probe_idx = engine.find_object_index(probe).expect("probe exists");
        assert_eq!(
            engine.objects[probe_idx]
                .state
                .local_vars
                .get("target_slot"),
            Some(&Value::Int(9)),
            "reference dispatch also re-resolves Slot on the target definition"
        );
        assert!(
            !engine.objects[probe_idx]
                .state
                .local_vars
                .contains_key("named_slot"),
            "the namespace definition's reference function did not run"
        );

        let result = engine
            .call_object_function(
                caller_idx,
                "PokeNamespaced",
                vec![Value::Object(namespace_target.as_u64())],
            )
            .expect("same-definition namespaced call stays valid");
        assert_eq!(result, Value::Int(1022));

        // Missing function: error for `->`, nil for `->~`.
        // (The engine wraps the VM error; the distinction that matters is
        // error-vs-nil between -> and ->~ below.)
        engine
            .call_object_function(caller_idx, "PokeMissing", target_arg.clone())
            .expect_err("missing function on -> is an error");
        let result = engine
            .call_object_function(caller_idx, "PokeMissingSafe", target_arg)
            .expect("->~ forgives the missing function");
        assert_eq!(result, Value::Nil);

        // Falsy target: error even for `->~` (the exact "target is zero"
        // message is pinned by the clonk-script unit test).
        engine
            .call_object_function(caller_idx, "PokeNil", Vec::new())
            .expect_err("falsy target throws even for ->~");
    }

    #[test]
    fn arrow_reference_return_assigns_the_target_objects_local() {
        // Kingdoms' THRN line 195 uses this exact C4Aul sequence:
        // `pReviveObject->SacrificeMade()=1`. AB_CALL installs the target
        // function's reference return in the caller stack cell
        // (C4AulExec.cpp:1290-1299); AB_RETURN preserves `func &`
        // (C4AulExec.cpp:1054-1067); AB_Set writes through it (:858-865).
        let caller_script = r#"
            public func Mark(target) {
                target->SacrificeMade() = 1;
                return 1;
            }
        "#;
        let revive_script = r#"
            local sacrifice_made;
            public func & SacrificeMade() { return sacrifice_made; }
        "#;

        let mut engine = Engine::with_seed(7);
        engine
            .register_definition(
                Definition::from_script("CALL", "Caller", caller_script)
                    .expect("caller compiles"),
            )
            .expect("caller registers");
        engine
            .register_definition(
                Definition::from_script("RVIV", "Revive", revive_script)
                    .expect("revive compiles"),
            )
            .expect("revive registers");
        let caller = engine
            .spawn_object(SpawnConfig::new("CALL"))
            .expect("caller spawns");
        let revive = engine
            .spawn_object(SpawnConfig::new("RVIV"))
            .expect("revive spawns");
        engine.tick_without_snapshot().expect("tick succeeds");

        let caller_idx = engine.find_object_index(caller).expect("caller exists");
        engine
            .call_object_function(
                caller_idx,
                "Mark",
                vec![Value::Object(revive.as_u64())],
            )
            .expect("reference assignment succeeds");

        let revive_idx = engine.find_object_index(revive).expect("revive exists");
        assert_eq!(
            engine.objects[revive_idx]
                .state
                .local_vars
                .get("sacrifice_made"),
            Some(&Value::Int(1))
        );
    }

    #[test]
    fn object_call_family_runs_target_def_script_function_like_cpp() {
        // FnObjectCall/FnProtectedCall/FnPrivateCall (C4Script.cpp:3434-3449,
        // 3502-3534): all three resolve in the TARGET object's def script
        // with failsafe=true (silent C4VNull on a miss). The access-level
        // difference (AA_PUBLIC vs AA_PROTECTED vs AA_PRIVATE) only LOGS on
        // violation — the call still executes ("don't even break in strict
        // execution", C4Aul.cpp:332-342) — so behavior is identical, and
        // even a `private func` runs via plain ObjectCall. Nil target →
        // C4VNull; script pars[2..=9] shift to callee Par(0..=7).
        let caller_script = r#"
        global func Poke(target) { return ObjectCall(target, "Secret", 21); }
        global func PokeProtected(target) { return ProtectedCall(target, "Secret", 5); }
        global func PokePrivate(target) { return PrivateCall(target, "Secret", 7); }
        global func PokeGlobal(target) { return ObjectCall(target, "SomeGlobalOnlyFunc"); }
        global func PokeGlobalProtected(target) { return ProtectedCall(target, "SomeGlobalOnlyFunc"); }
        global func PokeGlobalPrivate(target) { return PrivateCall(target, "SomeGlobalOnlyFunc"); }
        global func PokeGlobalArrow(target) { return target->SomeGlobalOnlyFunc(); }
        global func PokeNil() { return ObjectCall(0, "Secret"); }
        global func PokeMissing(target) { return ObjectCall(target, "NoSuch"); }
        "#;
        let probe_script = r#"
        local tag;
        private func Secret(v) {
            tag = tag + v;
            return v * 2;
        }
        "#;

        let mut engine = Engine::with_seed(7);
        engine
            .register_definition(
                Definition::from_script("CLLR", "Caller", caller_script).expect("caller compiles"),
            )
            .expect("caller registers");
        engine
            .register_definition(
                Definition::from_script("PROB", "Probe", probe_script).expect("probe compiles"),
            )
            .expect("probe registers");
        assert_eq!(
            engine.install_global_scripts(&[(
                "System.c4g/ObjectCall.c".to_string(),
                "global func SomeGlobalOnlyFunc() { return GetID(); }".to_string(),
            )]),
            1
        );
        let caller = engine
            .spawn_object(SpawnConfig::new("CLLR"))
            .expect("caller spawns");
        let probe = engine
            .spawn_object(SpawnConfig::new("PROB"))
            .expect("probe spawns");
        engine.tick_without_snapshot().expect("tick succeeds");

        let caller_idx = engine.find_object_index(caller).expect("caller exists");
        let target_arg = vec![Value::Object(probe.as_u64())];
        for (function, expected) in [
            ("Poke", Value::Int(42)),
            ("PokeProtected", Value::Int(10)),
            ("PokePrivate", Value::Int(14)),
        ] {
            let result = engine
                .call_object_function(caller_idx, function, target_arg.clone())
                .expect("call succeeds");
            assert_eq!(result, expected, "{function}: callee return verbatim");
        }
        let probe_idx = engine.find_object_index(probe).expect("probe exists");
        assert_eq!(
            engine.objects[probe_idx].state.local_vars.get("tag"),
            Some(&Value::Int(33)),
            "the target's local-var writes committed (21 + 5 + 7)"
        );

        for function in ["PokeGlobal", "PokeGlobalProtected", "PokeGlobalPrivate"] {
            let result = engine
                .call_object_function(caller_idx, function, target_arg.clone())
                .expect("global-only ObjectCall-family miss is not an error");
            assert_eq!(
                result,
                Value::Nil,
                "{function}: owner-scoped GetSFunc must not resolve a global"
            );
        }
        let result = engine
            .call_object_function(caller_idx, "PokeGlobalArrow", target_arg.clone())
            .expect("arrow call retains its global fallback");
        assert_eq!(
            result,
            Value::C4Id("PROB".into()),
            "the global arrow callee runs in the target object's scope"
        );

        let result = engine
            .call_object_function(caller_idx, "PokeNil", Vec::new())
            .expect("nil target is not an error");
        assert_eq!(result, Value::Nil, "nil target → C4VNull");
        let result = engine
            .call_object_function(caller_idx, "PokeMissing", target_arg)
            .expect("missing function is not an error");
        assert_eq!(result, Value::Nil, "failsafe miss → C4VNull");
    }

    #[test]
    fn definition_call_runs_def_script_without_object_context_like_cpp() {
        // FnDefinitionCall (C4Script.cpp:3451-3468): DefinitionCall(id, name,
        // p0..p7) runs `name` on the DEFINITION's script with Obj=nullptr —
        // always failsafe (the "~" prefix, :3457-3459), so a miss or an
        // unknown id is a silent C4VNull. The callee has no object context.
        let caller_script = r#"
        global func Spawn(targetdef) { return DefinitionCall(targetdef, "Factory", 4); }
        global func SpawnMissing(targetdef) { return DefinitionCall(targetdef, "NoSuch"); }
        global func SpawnBadId() { return DefinitionCall(XXXX, "Factory"); }
        "#;
        let factory_script = r#"
        func Factory(n) {
            if (this()) { return 0 - 1; }
            return n * 11;
        }
        "#;

        let mut engine = Engine::with_seed(7);
        engine
            .register_definition(
                Definition::from_script("CLLR", "Caller", caller_script).expect("caller compiles"),
            )
            .expect("caller registers");
        engine
            .register_definition(
                Definition::from_script("FCTY", "Factory", factory_script)
                    .expect("factory compiles"),
            )
            .expect("factory registers");
        let caller = engine
            .spawn_object(SpawnConfig::new("CLLR"))
            .expect("caller spawns");
        engine.tick_without_snapshot().expect("tick succeeds");

        let caller_idx = engine.find_object_index(caller).expect("caller exists");
        let result = engine
            .call_object_function(caller_idx, "Spawn", vec![Value::C4Id("FCTY".into())])
            .expect("Spawn succeeds");
        assert_eq!(
            result,
            Value::Int(44),
            "the def-script function ran with no object context (this() nil)"
        );
        let result = engine
            .call_object_function(caller_idx, "SpawnMissing", vec![Value::C4Id("FCTY".into())])
            .expect("missing function is not an error");
        assert_eq!(result, Value::Nil);
        let result = engine
            .call_object_function(caller_idx, "SpawnBadId", Vec::new())
            .expect("unknown id is not an error");
        assert_eq!(result, Value::Nil, "C4Id2Def failure → C4VNull");
    }

    #[test]
    fn game_call_runs_scenario_script_function_like_cpp() {
        // FnGameCall (C4Script.cpp:3470-3484): scenario script host ONLY,
        // always failsafe, Obj=nullptr. The lookup is owner-scoped — a
        // `global func` in a definition's script is NOT found
        // (C4Aul.cpp:295-298,562-576).
        let caller_script = r#"
        global func CallerGlobal() { return 99; }
        global func Ping() { return GameCall("Answer", 6); }
        global func PingMissing() { return GameCall("NoSuch"); }
        global func PingDefGlobal() { return GameCall("CallerGlobal"); }
        global func PingScenarioGlobal() { return GameCall("ScenarioGlobal"); }
        "#;

        let mut engine = Engine::with_seed(7);
        engine
            .register_definition(
                Definition::from_script("CLLR", "Caller", caller_script).expect("caller compiles"),
            )
            .expect("caller registers");
        engine
            .install_scenario_script(
                "Scenario",
                r#"
                func Answer(n) { return n * 7; }
                global func ScenarioGlobal() { return 123; }
                "#,
            )
            .expect("scenario installs");
        let caller = engine
            .spawn_object(SpawnConfig::new("CLLR"))
            .expect("caller spawns");
        engine.tick_without_snapshot().expect("tick succeeds");

        let caller_idx = engine.find_object_index(caller).expect("caller exists");
        let result = engine
            .call_object_function(caller_idx, "Ping", Vec::new())
            .expect("Ping succeeds");
        assert_eq!(result, Value::Int(42));
        let result = engine
            .call_object_function(caller_idx, "PingMissing", Vec::new())
            .expect("missing is not an error");
        assert_eq!(result, Value::Nil);
        let result = engine
            .call_object_function(caller_idx, "PingDefGlobal", Vec::new())
            .expect("def-script global is not visible to GameCall");
        assert_eq!(
            result,
            Value::Nil,
            "owner-scoped lookup: definition globals are not in the scenario host"
        );
        let result = engine
            .call_object_function(caller_idx, "PingScenarioGlobal", Vec::new())
            .expect("scenario global lookup remains failsafe");
        assert_eq!(
            result,
            Value::Nil,
            "scenario global declarations are engine-owned behind an unnamed link"
        );
    }

    #[test]
    fn definition_category_exposes_defcore_category_bits() {
        // Menus filter goal/rule objects by their DefCore Category
        // (C4MainMenu.cpp:392-400).
        let mut engine = Engine::with_seed(7);
        let mut goal_def =
            Definition::from_script("GOAL", "Goal", "").expect("goal compiles");
        goal_def.set_category(1 << 5); // C4D_Goal
        engine
            .register_definition(goal_def)
            .expect("goal registers");
        // normalize_category keeps the goal bit (a display-category default
        // may be OR'd in, mirroring C4Def defaults).
        let category = engine.definition_category("GOAL").expect("category");
        assert_ne!(category & (1 << 5), 0);
        assert_eq!(engine.definition_category("NONE"), None);
    }

    #[test]
    fn get_definition_uses_cpp_runtime_id_order_and_parameter_conversion() {
        // C4Game sorts Game.Defs by raw C4ID before runtime (C4Game.cpp:112;
        // C4Def.cpp:1394-1405), then FnGetDefinition/C4DefList::GetDef index
        // that order (C4Script.cpp:2668-2677; C4Def.cpp:1141-1158). Zero
        // category means C4D_All; filtering preserves that order. Negative
        // indices become out-of-range size_t, and C4Aul nil-fills/converts
        // the two C4ValueInt ABI slots while ignoring surplus call arguments
        // (C4AulExec.cpp:1364-1396).
        let mut engine = Engine::with_seed(7);
        for (id, category) in [
            ("AAAZ", CATEGORY_STRUCTURE),
            ("MIDM", CATEGORY_OBJECT),
            ("ZZZA", CATEGORY_OBJECT),
        ] {
            let mut definition =
                Definition::from_script(id, id, "").expect("definition compiles");
            definition.set_category(category);
            engine
                .register_definition(definition)
                .expect("definition registers");
        }
        engine
            .register_definition(
                Definition::from_script(
                    "CALL",
                    "Caller",
                    r#"#strict
                    func Probe() {
                        return [
                            GetDefinition(),
                            GetDefinition(1),
                            GetDefinition(0, C4D_Structure),
                            GetDefinition(1, C4D_Object),
                            GetDefinition(true, C4D_Object, 123),
                            GetDefinition(-1),
                            GetDefinition(99)
                        ];
                    }
                    "#,
                )
                .expect("caller compiles"),
            )
            .expect("caller registers");

        let caller = engine
            .spawn_object(SpawnConfig::new("CALL"))
            .expect("caller spawns");
        let caller_index = engine.find_object_index(caller).expect("caller exists");
        let result = engine
            .call_object_function(caller_index, "Probe", Vec::new())
            .expect("Probe runs");
        assert_eq!(
            result,
            Value::Array(vec![
                Value::C4Id("ZZZA".into()),
                Value::C4Id("CALL".into()),
                Value::C4Id("AAAZ".into()),
                Value::C4Id("MIDM".into()),
                Value::C4Id("MIDM".into()),
                Value::Nil,
                Value::Nil,
            ])
        );
    }

    #[test]
    fn scenario_get_definition_uses_the_same_engine_catalog() {
        // Scenario functions call the same global FnGetDefinition over
        // Game.Defs as definition/object functions (C4Script.cpp:2668-2677),
        // so the snapshot callback bridge must retain the runtime raw-C4ID
        // order established by C4DefList::SortByID (C4Game.cpp:112).
        let mut engine = Engine::with_seed(7);
        for (id, category) in [
            ("AAAZ", CATEGORY_STRUCTURE),
            ("MIDM", CATEGORY_OBJECT),
            ("ZZZA", CATEGORY_OBJECT),
        ] {
            let mut definition =
                Definition::from_script(id, id, "").expect("definition compiles");
            definition.set_category(category);
            engine
                .register_definition(definition)
                .expect("definition registers");
        }

        let created = engine
            .install_scenario_script_with_convention(
                "Scenario",
                r#"
                func Initialize() {
                    CreateObject(GetDefinition(0, C4D_Object), 0, 0, -1);
                }
                "#,
                true,
            )
            .expect("scenario initializes");
        assert_eq!(created.len(), 1);
        let index = engine
            .find_object_index(created[0])
            .expect("scenario-created object exists");
        assert_eq!(engine.objects[index].definition_id.as_str(), "ZZZA");
    }

    #[test]
    fn scenario_initialize_reads_the_live_material_table_like_cpp() {
        // Game.Script.Call(PSF_Initialize) reaches FnMaterial, which resolves
        // through the live Game.Material table (C4Game.cpp:2731-2734;
        // C4Script.cpp:2482-2485; C4Material.cpp:302-308).
        let library = MaterialLibrary::parse(
            r#"
            [Material Earth]
            Name=Earth
            Density=100
            "#,
        )
        .expect("material library parses");
        let mut engine = Engine::with_seed(7);
        engine.configure_materials_from_library(&library);
        engine
            .register_definition(simple_definition("MARK"))
            .expect("marker registers");

        let created = engine
            .install_scenario_script_with_convention(
                "Scenario",
                r#"
                func Initialize() {
                    if (Material("Earth") >= 0) CreateObject(MARK, 0, 0, -1);
                }
                "#,
                true,
            )
            .expect("scenario initializes");

        assert_eq!(created.len(), 1, "Earth lookup creates the marker");
        let marker = engine
            .object_snapshot(created[0])
            .expect("scenario-created marker exists");
        assert_eq!(marker.definition_id.as_str(), "MARK");
    }

    #[test]
    fn game_call_ex_broadcasts_to_goal_rule_environment_objects_like_cpp() {
        // FnGameCallEx (C4Script.cpp:3486-3500) → GRBroadcast
        // (C4ScriptHost.cpp:234-248): every LIVE object whose Category has
        // a C4D_Goal|C4D_Rule|C4D_Environment bit is called first (list
        // order, results DISCARDED — fRejectTest=false), then the scenario
        // script; only the scenario result is returned. Plain-category
        // objects are never called.
        let caller_script = r#"
        global func Shout() { return GameCallEx("Roll", 5); }
        global func ShoutGlobalOnly() { return GameCallEx("GlobalOnly", 5); }
        "#;
        let listener_script = r#"
        local hits, promote;
        func SetPromote(object target) { promote = target; }
        func Roll(n) {
            hits = hits + n;
            if (promote) SetCategory(524288, promote);
            return 1000; // discarded by the broadcast
        }
        func MarkGlobalOnly() { hits = hits + 100; return hits; }
        global func GlobalOnly(n) { return MarkGlobalOnly(); }
        "#;

        let mut engine = Engine::with_seed(7);
        engine
            .register_definition(
                Definition::from_script("CLLR", "Caller", caller_script).expect("caller compiles"),
            )
            .expect("caller registers");
        let mut goal_def =
            Definition::from_script("GOAL", "Goal", listener_script).expect("goal compiles");
        goal_def.set_category(1 << 5); // C4D_Goal
        engine
            .register_definition(goal_def)
            .expect("goal registers");
        let mut rule_def =
            Definition::from_script("RULE", "Rule", listener_script).expect("rule compiles");
        rule_def.set_category(1 << 19); // C4D_Rule
        engine
            .register_definition(rule_def)
            .expect("rule registers");
        engine
            .register_definition(
                Definition::from_script("PLAI", "Plain", listener_script).expect("plain compiles"),
            )
            .expect("plain registers");
        engine
            .install_scenario_script(
                "Scenario",
                r#"
                func Roll(n) { return n * 3; }
                "#,
            )
            .expect("scenario installs");

        let caller = engine
            .spawn_object(SpawnConfig::new("CLLR"))
            .expect("caller spawns");
        // Distinct definitions in one sorting category are inserted at the
        // front of that category bracket. Spawn the plain object first so
        // the later goal link precedes it in C++ master-list order.
        let plain = engine
            .spawn_object(SpawnConfig::new("PLAI"))
            .expect("plain spawns");
        let goal = engine
            .spawn_object(SpawnConfig::new("GOAL"))
            .expect("goal spawns");
        let rule = engine
            .spawn_object(SpawnConfig::new("RULE"))
            .expect("rule spawns");
        engine.tick_without_snapshot().expect("tick succeeds");

        let goal_index = engine.find_object_index(goal).expect("goal exists");
        assert_eq!(
            engine
                .call_object_function(
                    goal_index,
                    "SetPromote",
                    vec![Value::Object(plain.as_u64())],
                )
                .expect("promotion target stores"),
            Value::Nil
        );

        let caller_idx = engine.find_object_index(caller).expect("caller exists");
        let result = engine
            .call_object_function(caller_idx, "Shout", Vec::new())
            .expect("Shout succeeds");
        assert_eq!(
            result,
            Value::Int(15),
            "only the scenario script's result returns (goal/rule results discarded)"
        );
        let plain_idx = engine.find_object_index(plain).expect("plain exists");
        assert_eq!(
            engine.objects[plain_idx].state.category & (1 << 19),
            1 << 19,
            "the goal callback's foreign SetCategory is live before the later list node"
        );
        let result = engine
            .call_object_function(caller_idx, "ShoutGlobalOnly", Vec::new())
            .expect("global-only broadcast remains failsafe");
        assert_eq!(
            result,
            Value::Nil,
            "neither object hosts nor Game.Script own the named global function"
        );
        for (id, expected) in [(goal, Some(&Value::Int(5))), (rule, Some(&Value::Int(5)))] {
            let idx = engine.find_object_index(id).expect("listener exists");
            assert_eq!(
                engine.objects[idx].state.local_vars.get("hits"),
                expected,
                "goal/rule objects were each called once"
            );
        }
        assert_eq!(
            engine.objects[plain_idx].state.local_vars.get("hits"),
            Some(&Value::Int(5)),
            "an earlier callback's category write admits the later object at its turn"
        );
    }

    #[test]
    fn engine_grbroadcast_uses_master_order_and_rechecks_later_category() {
        // C4GameScriptHost::GRBroadcast walks Game.Objects First -> Next and
        // reads Category and Status at each link (C4ScriptHost.cpp:234-247).
        // The mutator is last in storage but first in master order: it admits
        // a later plain object, deactivates a later environment object, and
        // creates a marker (mutating the object list during the walk).
        let mut engine = Engine::with_seed(7);
        engine
            .register_definition(
                Definition::from_script("MARK", "Marker", "").expect("marker compiles"),
            )
            .expect("marker registers");

        let mut promoted = Definition::from_script(
            "PROM",
            "Promoted",
            r#"
            func PreInitializePlayer(int player) {
                CreateObject(MARK, 20, 0, -1);
            }
            "#,
        )
        .expect("promoted listener compiles");
        // C4D_Parallax is outside the GRBroadcast mask and, like the three
        // broadcast-only bits, has no low category-sort bit.
        promoted.set_category(CATEGORY_PARALLAX);
        engine
            .register_definition(promoted)
            .expect("promoted listener registers");

        let mut tail = Definition::from_script(
            "TAIL",
            "Tail",
            r#"
            func PreInitializePlayer(int player) {
                CreateObject(MARK, 30, 0, -1);
            }
            "#,
        )
        .expect("tail listener compiles");
        tail.set_category(1 << 5); // C4D_Goal
        engine
            .register_definition(tail)
            .expect("tail listener registers");

        let mut skipped = Definition::from_script(
            "SKIP",
            "Skipped",
            r#"
            func PreInitializePlayer(int player) {
                CreateObject(MARK, 99, 0, -1);
            }
            "#,
        )
        .expect("skipped listener compiles");
        skipped.set_category(1 << 6); // C4D_Environment
        engine
            .register_definition(skipped)
            .expect("skipped listener registers");

        let mut mutator = Definition::from_script(
            "MUTR",
            "Mutator",
            r#"
            local promote, deactivate;
            func Configure(object later, object skipped) {
                promote = later;
                deactivate = skipped;
            }
            func PreInitializePlayer(int player) {
                CreateObject(MARK, 10, 0, -1);
                SetCategory(524288, promote);
                SetObjectStatus(2, deactivate, false);
            }
            "#,
        )
        .expect("mutator compiles");
        mutator.set_category(1 << 19); // C4D_Rule
        engine
            .register_definition(mutator)
            .expect("mutator registers");

        // Deliberately oppose storage and C++ category-sorted master order.
        let promoted = engine
            .spawn_object(SpawnConfig::new("PROM"))
            .expect("promoted listener spawns");
        let tail = engine
            .spawn_object(SpawnConfig::new("TAIL"))
            .expect("tail listener spawns");
        let skipped = engine
            .spawn_object(SpawnConfig::new("SKIP"))
            .expect("skipped listener spawns");
        let mutator = engine
            .spawn_object(SpawnConfig::new("MUTR"))
            .expect("mutator spawns");
        assert_eq!(
            engine.exec_list.iter().rev().copied().collect::<Vec<_>>(),
            vec![mutator, skipped, tail, promoted],
            "fixture must distinguish forward master order from storage order"
        );

        let mutator_index = engine
            .find_object_index(mutator)
            .expect("mutator exists");
        engine
            .call_object_function(
                mutator_index,
                "Configure",
                vec![
                    Value::Object(promoted.as_u64()),
                    Value::Object(skipped.as_u64()),
                ],
            )
            .expect("mutator configures");
        engine
            .install_scenario_script_with_convention(
                "Scenario",
                r#"
                global func PreInitializePlayer(int player) {
                    CreateObject(MARK, 40, 0, -1);
                }
                "#,
                true,
            )
            .expect("scenario installs");

        engine
            .register_player(PlayerConfig::new(0, "Player"))
            .expect("player registers");

        assert_eq!(
            engine
                .snapshot()
                .objects
                .iter()
                .filter(|object| object.definition_id.as_str() == "MARK")
                .map(|object| object.position.x)
                .collect::<Vec<_>>(),
            vec![10, 30, 20, 40],
            "master-order listeners run first, live status/category decide later nodes, and the scenario runs last"
        );
        let skipped_index = engine
            .find_object_index(skipped)
            .expect("deactivated listener remains addressable");
        assert_eq!(
            engine.objects[skipped_index].state.status,
            ObjectStatus::Inactive
        );
    }

    #[test]
    fn set_hostility_runs_reject_and_change_broadcasts_like_cpp() {
        // FnSetHostility performs a rejecting GRBroadcast before the write,
        // then broadcasts OnHostilityChange after the live declaration is
        // visible (C4Script.cpp:2521-2537). GRBroadcast visits live
        // goal/rule/environment objects before the scenario host
        // (C4ScriptHost.cpp:234-248).
        let caller_script = r#"
        func TryHostile() { return SetHostility(1, 2, true, true, false); }
        func ForceHostile() { return SetHostility(1, 2, true, true, true); }
        func ReadHostile() { return Hostile(1, 2, true); }
        "#;
        let rule_script = r#"#strict
        local reject, seen_new, seen_old, seen_live;
        func SetReject(value) { reject = value; }
        func RejectHostilityChange(player, opponent, hostile) { return reject; }
        func OnHostilityChange(player, opponent, hostile, old) {
            seen_new = hostile;
            seen_old = old;
            seen_live = Hostile(player, opponent, true);
        }
        func Seen() { return [seen_new, seen_old, seen_live]; }
        "#;

        let mut engine = Engine::with_seed(7);
        engine
            .register_definition(
                Definition::from_script("CALL", "Caller", caller_script).expect("caller compiles"),
            )
            .expect("caller registers");
        let mut rule =
            Definition::from_script("RULE", "Rule", rule_script).expect("rule compiles");
        rule.set_category(1 << 19); // C4D_Rule
        engine.register_definition(rule).expect("rule registers");
        engine
            .register_player(PlayerConfig::new(1, "Alice"))
            .expect("Alice registers");
        engine
            .register_player(PlayerConfig::new(2, "Bob"))
            .expect("Bob registers");
        let caller = engine
            .spawn_object(SpawnConfig::new("CALL"))
            .expect("caller spawns");
        let rule = engine
            .spawn_object(SpawnConfig::new("RULE"))
            .expect("rule spawns");
        engine.tick_without_snapshot().expect("tick succeeds");

        let call = |engine: &mut Engine, id: ObjectId, function: &str, args: Vec<Value>| {
            let index = engine.find_object_index(id).expect("object exists");
            engine
                .call_object_function(index, function, args)
                .expect("script call succeeds")
        };
        assert_eq!(
            call(&mut engine, rule, "SetReject", vec![Value::Int(1)]),
            Value::Nil
        );
        assert_eq!(
            call(&mut engine, caller, "TryHostile", Vec::new()),
            Value::Bool(false),
            "a truthy rule callback rejects the declaration"
        );
        assert_eq!(
            call(&mut engine, caller, "ReadHostile", Vec::new()),
            Value::Bool(false)
        );

        assert_eq!(
            call(&mut engine, rule, "SetReject", vec![Value::Int(0)]),
            Value::Nil
        );
        assert_eq!(
            call(&mut engine, caller, "TryHostile", Vec::new()),
            Value::Bool(true)
        );
        assert_eq!(
            call(&mut engine, rule, "Seen", Vec::new()),
            Value::Array(vec![
                Value::Bool(true),
                Value::Nil,
                Value::Bool(true),
            ]),
            "OnHostilityChange receives old/new state and observes the live write"
        );

        // fNoCalls skips only the rejection test. The post-change callback
        // still runs and receives old=true (C4Script.cpp:2526-2536).
        assert_eq!(
            call(&mut engine, rule, "SetReject", vec![Value::Int(1)]),
            Value::Nil
        );
        assert_eq!(
            call(&mut engine, caller, "ForceHostile", Vec::new()),
            Value::Bool(true)
        );
        assert_eq!(
            call(&mut engine, rule, "Seen", Vec::new()),
            Value::Array(vec![
                Value::Bool(true),
                Value::Bool(true),
                Value::Bool(true),
            ])
        );
        assert!(
            engine
                .players()
                .find(|player| player.id() == 1)
                .is_some_and(|player| player.is_hostile_towards(2)),
            "the accepted declaration persists in C4Player state"
        );
    }

    fn bool_parameter_hostility_fixture() -> (Engine, ObjectId) {
        let script = r#"
        func ApplyHostility(value) {
            SetHostility(1, 2, value, true, true);
            return Hostile(1, 2, true);
        }
        "#;
        let mut engine = Engine::with_seed(7);
        engine
            .register_definition(
                Definition::from_script("CALL", "Caller", script).expect("caller compiles"),
            )
            .expect("caller registers");
        engine
            .register_player(PlayerConfig::new(1, "Alice"))
            .expect("Alice registers");
        engine
            .register_player(PlayerConfig::new(2, "Bob"))
            .expect("Bob registers");
        let caller = engine
            .spawn_object(SpawnConfig::new("CALL"))
            .expect("caller spawns");
        engine.tick_without_snapshot().expect("tick succeeds");
        (engine, caller)
    }

    #[test]
    fn bool_host_parameter_accepts_every_truthy_non_scalar_value() {
        let (mut engine, caller) = bool_parameter_hostility_fixture();
        let caller_index = engine.find_object_index(caller).expect("caller exists");

        for argument in [
            Value::Object(caller.as_u64()),
            Value::String("truthy".into()),
            Value::String(String::new().into()),
            Value::C4Id("CLNK".into()),
            Value::Array(Vec::new()),
            Value::Proplist(Default::default()),
        ] {
            assert_eq!(
                engine
                    .call_object_function(
                        caller_index,
                        "ApplyHostility",
                        vec![Value::Bool(false)],
                    )
                    .expect("false resets hostility"),
                Value::Bool(false)
            );
            assert_eq!(
                engine
                    .call_object_function(caller_index, "ApplyHostility", vec![argument.clone()])
                    .expect("truthy non-scalar bool parameter is accepted"),
                Value::Bool(true),
                "{argument:?} must coerce to true"
            );
        }
    }

    #[test]
    fn bool_host_parameter_preserves_scalar_and_zero_payload_coercions() {
        let (mut engine, caller) = bool_parameter_hostility_fixture();
        let caller_index = engine.find_object_index(caller).expect("caller exists");

        for (argument, expected) in [
            (Value::Bool(true), true),
            (Value::Bool(false), false),
            (Value::Int(-7), true),
            (Value::Int(0), false),
            (Value::Nil, false),
            (Value::Object(0), false),
            (Value::C4Id(String::new()), false),
            (Value::C4Id("NONE".into()), false),
            (Value::C4Id("0000".into()), false),
        ] {
            assert_eq!(
                engine
                    .call_object_function(caller_index, "ApplyHostility", vec![argument.clone()])
                    .expect("bool parameter is accepted"),
                Value::Bool(expected),
                "{argument:?} bool coercion changed"
            );
        }
    }

    fn team_switch_fixture(
        switcher_team: i32,
        league_game: bool,
    ) -> (Engine, ObjectId, ObjectId) {
        let caller_script = r#"
        func Switch(no_calls) { return SetPlayerTeam(1, 1, no_calls); }
        "#;
        let rule_script = r#"#strict
        local reject, reject_calls, switch_calls, seen;
        func Initialize() {
            reject = 0;
            reject_calls = 0;
            switch_calls = 0;
        }
        func SetReject(value) { reject = value; }
        func RejectTeamSwitch(player, new_team) {
            reject_calls = reject_calls + 1;
            return reject;
        }
        func OnTeamSwitch(player, new_team, old_team) {
            switch_calls = switch_calls + 1;
            seen = [
                player, new_team, old_team, GetPlayerTeam(player),
                Hostile(player, 2, true), Hostile(2, player, true),
                Hostile(player, 3, true), Hostile(3, player, true),
                GetHomebaseMaterial(player, BRCK)
            ];
        }
        func Seen() { return [reject_calls, switch_calls, seen]; }
        "#;

        let mut engine = Engine::with_seed(7);
        engine.set_league_game(league_game);
        engine.set_teams(vec![
            TeamInfo::new(1, "Red", 0x00f4_0000),
            TeamInfo::new(2, "Blue", 0x0000_c800),
        ]);
        engine.set_team_home_base_rule(true);
        engine
            .register_definition(
                Definition::from_script("CALL", "Caller", caller_script)
                    .expect("caller compiles"),
            )
            .expect("caller registers");
        let mut rule =
            Definition::from_script("RULE", "Rule", rule_script).expect("rule compiles");
        rule.set_category(1 << 19); // C4D_Rule
        engine.register_definition(rule).expect("rule registers");
        engine
            .register_definition(
                Definition::from_script("BRCK", "Brick", "").expect("brick compiles"),
            )
            .expect("brick registers");

        engine
            .register_player(
                PlayerConfig::new(1, "Alice")
                    .with_team(Some(switcher_team))
                    .with_home_base_material(HashMap::from([("BRCK".to_string(), 2)])),
            )
            .expect("Alice registers");
        engine
            .register_player(
                PlayerConfig::new(2, "Bob")
                    .with_team(Some(1))
                    .with_home_base_material(HashMap::from([("BRCK".to_string(), 7)])),
            )
            .expect("Bob registers");
        engine
            .register_player(
                PlayerConfig::new(3, "Carol")
                    .with_team(Some(2))
                    .with_home_base_material(HashMap::from([("BRCK".to_string(), 2)])),
            )
            .expect("Carol registers");

        // Deliberately seed relations opposite to what team 1 will require:
        // SetTeamHostility must clear both Alice/Bob declarations and set
        // both Alice/Carol declarations after the accepted switch.
        engine
            .set_hostility(1, 2, true)
            .expect("Alice hostility seeds");
        engine
            .set_hostility(2, 1, true)
            .expect("Bob hostility seeds");
        engine
            .set_hostility(1, 3, false)
            .expect("Alice alliance seeds");
        engine
            .set_hostility(3, 1, false)
            .expect("Carol alliance seeds");

        let caller = engine
            .spawn_object(SpawnConfig::new("CALL"))
            .expect("caller spawns");
        let rule = engine
            .spawn_object(SpawnConfig::new("RULE"))
            .expect("rule spawns");
        engine.tick_without_snapshot().expect("tick succeeds");
        (engine, caller, rule)
    }

    fn call_team_switch_fixture(
        engine: &mut Engine,
        object: ObjectId,
        function: &str,
        args: Vec<Value>,
    ) -> Value {
        let index = engine.find_object_index(object).expect("object exists");
        engine
            .call_object_function(index, function, args)
            .expect("script call succeeds")
    }

    #[test]
    fn set_player_team_rejects_then_switches_with_live_cpp_side_effects() {
        // FnSetPlayerTeam first offers a rejecting GRBroadcast, then moves the
        // player, imports the target captain's homebase material, refreshes
        // hostility in both directions and broadcasts the completed change
        // (C4Script.cpp:5730-5784; C4Player.cpp:1022-1034,2354-2367).
        let (mut engine, caller, rule) = team_switch_fixture(2, false);

        assert_eq!(
            call_team_switch_fixture(
                &mut engine,
                rule,
                "SetReject",
                vec![Value::Int(1)],
            ),
            Value::Nil
        );
        assert_eq!(
            call_team_switch_fixture(
                &mut engine,
                caller,
                "Switch",
                vec![Value::Bool(false)],
            ),
            Value::Bool(false),
            "a truthy RejectTeamSwitch vetoes every mutation"
        );
        assert_eq!(engine.player(1).expect("Alice exists").team(), Some(2));
        assert_eq!(
            engine
                .player(1)
                .expect("Alice exists")
                .home_base_material()
                .get("BRCK"),
            Some(&2)
        );
        assert!(engine
            .player(1)
            .expect("Alice exists")
            .is_hostile_towards(2));
        assert!(!engine
            .player(1)
            .expect("Alice exists")
            .is_hostile_towards(3));
        assert_eq!(
            call_team_switch_fixture(&mut engine, rule, "Seen", Vec::new()),
            Value::Array(vec![Value::Int(1), Value::Nil, Value::Nil])
        );

        assert_eq!(
            call_team_switch_fixture(
                &mut engine,
                rule,
                "SetReject",
                vec![Value::Int(0)],
            ),
            Value::Nil
        );
        assert_eq!(
            call_team_switch_fixture(
                &mut engine,
                caller,
                "Switch",
                vec![Value::Bool(false)],
            ),
            Value::Bool(true)
        );
        assert_eq!(
            call_team_switch_fixture(&mut engine, rule, "Seen", Vec::new()),
            Value::Array(vec![
                Value::Int(2),
                Value::Int(1),
                Value::Array(vec![
                    Value::Int(1),
                    Value::Int(1),
                    Value::Int(2),
                    Value::Int(1),
                    Value::Bool(false),
                    Value::Bool(false),
                    Value::Bool(true),
                    Value::Bool(true),
                    Value::Int(7),
                ]),
            ]),
            "OnTeamSwitch sees the final team, hostilities and homebase synchronously"
        );

        let alice = engine.player(1).expect("Alice exists");
        assert_eq!(alice.team(), Some(1));
        assert!(!alice.is_hostile_towards(2));
        assert!(alice.is_hostile_towards(3));
        assert_eq!(alice.home_base_material().get("BRCK"), Some(&7));
        let bob = engine.player(2).expect("Bob exists");
        assert!(!bob.is_hostile_towards(1));
        assert_eq!(bob.home_base_material().get("BRCK"), Some(&7));
        assert!(engine
            .player(3)
            .expect("Carol exists")
            .is_hostile_towards(1));
    }

    #[test]
    fn set_player_team_imports_the_team_info_captains_exact_home_base_list() {
        let mut engine = Engine::new();
        engine.set_teams(vec![
            TeamInfo::new(1, "Ordered", 0).with_player_ids(vec![20, 10]),
            TeamInfo::new(2, "Old", 0).with_player_ids(vec![30]),
        ]);
        engine.set_team_home_base_rule(true);
        engine
            .register_definition(
                Definition::from_script(
                    "CALL",
                    "Caller",
                    "func Switch() { return SetPlayerTeam(1, 1, false); }",
                )
                .expect("team switch probe compiles"),
            )
            .expect("team switch probe registers");
        for (number, info_id, team) in [(1, 30, 2), (2, 10, 1), (5, 20, 1)] {
            engine
                .register_player(
                    PlayerConfig::new(number, format!("Player {number}"))
                        .with_player_info_id(info_id)
                        .with_team(Some(team)),
                )
                .expect("player registers");
        }
        let captain_material = vec![("ZINC".into(), -3), ("BRIK".into(), 0)];
        engine
            .player_mut(5)
            .expect("team-info captain")
            .set_home_base_material_entries(captain_material.clone());
        engine
            .player_mut(2)
            .expect("lower runtime number")
            .set_home_base_material_entries(vec![("ROCK".into(), 9)]);
        let caller = engine
            .spawn_object(SpawnConfig::new("CALL"))
            .expect("caller spawns");
        let caller_index = engine.find_object_index(caller).expect("caller index");

        assert_eq!(
            engine
                .call_object_function(caller_index, "Switch", Vec::new())
                .expect("team switch succeeds"),
            Value::Bool(true)
        );
        assert_eq!(
            engine
                .player(1)
                .expect("switcher")
                .home_base_material_entries(),
            captain_material
        );
    }

    #[test]
    fn set_player_team_does_not_skip_an_unjoined_team_info_captain() {
        let mut engine = Engine::new();
        engine.set_teams(vec![
            TeamInfo::new(1, "Ordered", 0).with_player_ids(vec![20, 10]),
            TeamInfo::new(2, "Old", 0).with_player_ids(vec![30]),
        ]);
        engine.set_team_home_base_rule(true);
        engine
            .register_definition(
                Definition::from_script(
                    "CALL",
                    "Caller",
                    "func Switch() { return SetPlayerTeam(1, 1, false); }",
                )
                .expect("team switch probe compiles"),
            )
            .expect("team switch probe registers");
        engine
            .register_player(
                PlayerConfig::new(1, "Switcher")
                    .with_player_info_id(30)
                    .with_team(Some(2)),
            )
            .expect("switcher registers");
        engine
            .register_player(
                PlayerConfig::new(2, "Live noncaptain")
                    .with_player_info_id(10)
                    .with_team(Some(1)),
            )
            .expect("noncaptain registers");
        let original = vec![("ZINC".into(), -4)];
        engine
            .player_mut(1)
            .expect("switcher")
            .set_home_base_material_entries(original.clone());
        engine
            .player_mut(2)
            .expect("live noncaptain")
            .set_home_base_material_entries(vec![("ROCK".into(), 9)]);
        let caller = engine
            .spawn_object(SpawnConfig::new("CALL"))
            .expect("caller spawns");
        let caller_index = engine.find_object_index(caller).expect("caller index");

        assert_eq!(
            engine
                .call_object_function(caller_index, "Switch", Vec::new())
                .expect("team switch succeeds"),
            Value::Bool(true)
        );
        assert_eq!(
            engine
                .player(1)
                .expect("switcher")
                .home_base_material_entries(),
            original
        );
    }

    #[test]
    fn set_player_team_same_team_short_circuits_and_league_refuses_first() {
        let (mut same_team, caller, rule) = team_switch_fixture(1, false);
        call_team_switch_fixture(
            &mut same_team,
            rule,
            "SetReject",
            vec![Value::Int(1)],
        );
        assert_eq!(
            call_team_switch_fixture(
                &mut same_team,
                caller,
                "Switch",
                vec![Value::Bool(false)],
            ),
            Value::Bool(true),
            "the existing membership succeeds before callback dispatch"
        );
        assert_eq!(
            call_team_switch_fixture(&mut same_team, rule, "Seen", Vec::new()),
            Value::Array(vec![Value::Nil, Value::Nil, Value::Nil])
        );
        assert_eq!(same_team.player(1).expect("Alice exists").team(), Some(1));

        let (mut league, caller, rule) = team_switch_fixture(2, true);
        call_team_switch_fixture(
            &mut league,
            rule,
            "SetReject",
            vec![Value::Int(1)],
        );
        assert_eq!(
            call_team_switch_fixture(
                &mut league,
                caller,
                "Switch",
                vec![Value::Bool(false)],
            ),
            Value::Bool(false),
            "league refusal precedes player, team and callback work"
        );
        assert_eq!(league.player(1).expect("Alice exists").team(), Some(2));
        assert_eq!(
            call_team_switch_fixture(&mut league, rule, "Seen", Vec::new()),
            Value::Array(vec![Value::Nil, Value::Nil, Value::Nil])
        );
        assert_eq!(
            league
                .player(1)
                .expect("Alice exists")
                .home_base_material()
                .get("BRCK"),
            Some(&2)
        );
        assert!(league
            .player(1)
            .expect("Alice exists")
            .is_hostile_towards(2));
        assert!(!league
            .player(1)
            .expect("Alice exists")
            .is_hostile_towards(3));
    }

    #[test]
    fn set_player_team_no_calls_changes_only_team_membership() {
        // fNoCalls still performs the validated team move, but skips both
        // broadcasts, SetTeamHostility and SyncHomebaseMaterialFromTeam.
        let (mut engine, caller, rule) = team_switch_fixture(2, false);
        call_team_switch_fixture(
            &mut engine,
            rule,
            "SetReject",
            vec![Value::Int(1)],
        );

        assert_eq!(
            call_team_switch_fixture(
                &mut engine,
                caller,
                "Switch",
                vec![Value::Bool(true)],
            ),
            Value::Bool(true)
        );
        assert_eq!(engine.player(1).expect("Alice exists").team(), Some(1));
        assert_eq!(
            call_team_switch_fixture(&mut engine, rule, "Seen", Vec::new()),
            Value::Array(vec![Value::Nil, Value::Nil, Value::Nil])
        );

        let alice = engine.player(1).expect("Alice exists");
        assert!(alice.is_hostile_towards(2));
        assert!(!alice.is_hostile_towards(3));
        assert_eq!(alice.home_base_material().get("BRCK"), Some(&2));
        let bob = engine.player(2).expect("Bob exists");
        assert!(bob.is_hostile_towards(1));
        assert_eq!(bob.home_base_material().get("BRCK"), Some(&7));
        assert!(!engine
            .player(3)
            .expect("Carol exists")
            .is_hostile_towards(1));
    }

    #[test]
    fn create_menu_opens_a_script_menu_and_get_menu_reads_it_like_cpp() {
        // FnCreateMenu (C4Script.cpp:1426-1459): inits the object's menu with
        // Identification = idMenuID ? idMenuID : iSymbol (C4Menu::InitMenu,
        // C4Menu.cpp:355). FnGetMenu (C4Script.cpp:1418-1424): an ACTIVE
        // menu returns that Identification, no menu returns C4MN_None (0).
        let script = r#"
        func OpenMenu() { return CreateMenu(WIPF, this(), this(), 0, "Choose"); }
        func OpenAliased() { return CreateMenu(WIPF, this(), this(), 0, "Choose", 0, 1, 0, MENU); }
        func OpenEqualDialog() { return CreateMenu(WIPF, this(), this(), 0, "Dialog", 0, 131); }
        func ReadMenu() { return GetMenu(this()); }
        "#;
        let mut engine = Engine::with_seed(7);
        engine
            .register_definition(
                Definition::from_script("CLNK", "Clonk", script).expect("script compiles"),
            )
            .expect("definition registers");
        let clonk = engine
            .spawn_object(SpawnConfig::new("CLNK"))
            .expect("clonk spawns");
        engine.tick_without_snapshot().expect("tick succeeds");

        let idx = engine.find_object_index(clonk).expect("clonk exists");
        assert_eq!(
            engine
                .call_object_function(idx, "ReadMenu", Vec::new())
                .expect("ReadMenu succeeds"),
            Value::Int(0),
            "no menu -> C4MN_None (C4Script.cpp:1423)"
        );
        assert_eq!(
            engine
                .call_object_function(idx, "OpenMenu", Vec::new())
                .expect("OpenMenu succeeds"),
            Value::Bool(true),
            "FnCreateMenu returns true on success"
        );
        let idx = engine.find_object_index(clonk).expect("clonk exists");
        assert_eq!(
            engine
                .call_object_function(idx, "ReadMenu", Vec::new())
                .expect("ReadMenu succeeds"),
            Value::C4Id("WIPF".into()),
            "active menu -> its Identification (the symbol id by default)"
        );
        let menu = engine
            .debug_object_menu(clonk.as_u64())
            .expect("clonk exists")
            .expect("menu is open");
        assert_eq!(menu.caption, "Choose");
        assert_eq!(menu.symbol_id, "WIPF");
        assert_eq!(menu.style, 0, "C4MN_Style_Normal");
        assert!(!menu.permanent, "fPermanent defaults false");
        assert_eq!(menu.selection, -1, "C4Menu::Default Selection (-1)");
        assert_eq!(
            menu.command_object,
            Some(clonk),
            "pCommandObj -> CB_Object callbacks"
        );

        // idMenuID overrides the symbol as Identification (C4Script.cpp:1452).
        assert_eq!(
            engine
                .call_object_function(idx, "OpenAliased", Vec::new())
                .expect("OpenAliased succeeds"),
            Value::Bool(true)
        );
        let idx = engine.find_object_index(clonk).expect("clonk exists");
        assert_eq!(
            engine
                .call_object_function(idx, "ReadMenu", Vec::new())
                .expect("ReadMenu succeeds"),
            Value::C4Id("MENU".into()),
            "idMenuID wins over the symbol id"
        );
        let menu = engine
            .debug_object_menu(clonk.as_u64())
            .expect("clonk exists")
            .expect("menu is open");
        assert_eq!(menu.style, 1, "iStyle stored (C4MN_Style_Context)");
        assert_eq!(
            menu.symbol_id, "WIPF",
            "idMenuID changes identity but not the title-bar symbol"
        );

        engine
            .call_object_function(idx, "OpenEqualDialog", Vec::new())
            .expect("EqualItemHeight dialog opens");
        let menu = engine
            .debug_object_menu(clonk.as_u64())
            .expect("clonk exists")
            .expect("dialog is open");
        assert_eq!(menu.style, 3, "base style is masked with C4MN_Style_BaseMask");
        assert!(
            menu.equal_item_height,
            "C4MN_Style_EqualItemHeight survives base-style masking"
        );
        assert_eq!(menu.columns, 1);
    }

    #[test]
    fn cross_object_get_menu_reads_the_targets_live_menu() {
        // AB_CALL gives an engine function the target as cthr->Obj, so
        // `other->GetMenu()` reads other->Menu, not the caller's menu
        // (C4AulExec.cpp:1216-1305; C4Script.cpp:1418-1424).
        let target_script = r#"
        func OpenMenu() { return CreateMenu(WIPF, this(), this(), 0, "Choose"); }
        "#;
        let caller_script = r#"
        func Probe(other) { return other->GetMenu(); }
        "#;
        let mut engine = Engine::with_seed(7);
        engine
            .register_definition(
                Definition::from_script("TRGT", "Target", target_script)
                    .expect("target compiles"),
            )
            .expect("target registers");
        engine
            .register_definition(
                Definition::from_script("CALL", "Caller", caller_script)
                    .expect("caller compiles"),
            )
            .expect("caller registers");
        let target = engine
            .spawn_object(SpawnConfig::new("TRGT"))
            .expect("target spawns");
        let caller = engine
            .spawn_object(SpawnConfig::new("CALL"))
            .expect("caller spawns");
        engine.tick_without_snapshot().expect("tick succeeds");

        let target_index = engine.find_object_index(target).expect("target exists");
        engine
            .call_object_function(target_index, "OpenMenu", Vec::new())
            .expect("menu opens");
        let caller_index = engine.find_object_index(caller).expect("caller exists");
        assert_eq!(
            engine
                .call_object_function(
                    caller_index,
                    "Probe",
                    vec![object_reference_value(target)],
                )
                .expect("cross-object GetMenu succeeds"),
            Value::C4Id("WIPF".into())
        );
    }

    #[test]
    fn scenario_callback_get_menu_reads_the_live_crew_menu() {
        // Game.Script's AB_CALL passes the live destination object to
        // FnGetMenu; runtime-only C4Object::Menu must not disappear behind
        // a serialized snapshot (C4AulExec.cpp:1228-1297;
        // C4Script.cpp:1412-1417).
        let clonk_script = r#"
        func OpenMenu() { return CreateMenu(WIPF, this(), this(), 0, "Choose"); }
        "#;
        let mut clonk = Definition::from_script("CLNK", "Clonk", clonk_script)
            .expect("clonk compiles");
        clonk.set_crew_member(true);
        let mut engine = Engine::with_seed(7);
        engine.register_definition(clonk).expect("clonk registers");
        engine
            .register_player(PlayerConfig::new(0, "Player"))
            .expect("player registers");
        let clonk = engine
            .spawn_object(
                SpawnConfig::new("CLNK")
                    .with_owner(0)
                    .with_crew_member(true),
            )
            .expect("clonk spawns");
        engine.tick_without_snapshot().expect("tick succeeds");
        let clonk_index = engine.find_object_index(clonk).expect("clonk exists");
        engine
            .call_object_function(clonk_index, "OpenMenu", Vec::new())
            .expect("menu opens");
        engine
            .install_scenario_script_with_convention(
                "Scenario",
                r#"
                func Probe() {
                    if (GetHiRank(0)->GetMenu() == WIPF) SetWealth(0, 77);
                }
                "#,
                true,
            )
            .expect("scenario installs");

        engine
            .call_scenario_script_function("Probe", Vec::new())
            .expect("Probe succeeds");
        assert_eq!(
            engine
                .players()
                .find(|player| player.id() == 0)
                .map(Player::wealth),
            Some(77)
        );
    }

