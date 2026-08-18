use super::*;

trait TestEngineExt {
    fn spawn_test_object(&mut self, config: SpawnConfig) -> ObjectId;
}

impl TestEngineExt for Engine {
    #[track_caller]
    fn spawn_test_object(&mut self, config: SpawnConfig) -> ObjectId {
        crate::TestValueExt::test_value(self.spawn_object(config))
    }
}

fn register(engine: &mut Engine, id: &str, source: &str) {
    crate::TestValueExt::test_value(engine.register_script_definition(id, id, source));
}

fn call(engine: &mut Engine, object: ObjectId, function: &str) -> Value {
    let index = crate::TestValueExt::test_value(engine.find_object_index(object));
    crate::TestValueExt::test_value(engine.call_object_function(index, function, Vec::new()))
}

fn link_initial_scripts(engine: &mut Engine) {
    engine.resolve_appends();
    crate::TestValueExt::test_value(engine.resolve_includes());
}

#[test]
fn link_surfaces_an_unresolvable_hard_inherited_per_definition() {
    // The engine-side half of C4AulScript::Parse's link-time report
    // (C4AulParse.cpp:2799 raising, :3563-3586 catching and counting). ADEF's
    // `inherited` has no target anywhere; BDEF's resolves through C4Aul's
    // owner hop into the engine table (C4Aul.cpp:281-288), so only ADEF is
    // reported — a chain-only oracle would report both.
    let mut engine = Engine::new();
    register(
        &mut engine,
        "ADEF",
        "#strict\nfunc Orphan() { return inherited(); }",
    );
    register(
        &mut engine,
        "BDEF",
        "#strict\n\
         global func Hop() { return 1; }\n\
         func Hop() { return inherited() + 10; }",
    );
    crate::TestValueExt::test_value(engine.relink_scripts());

    let reported = |id: &str| {
        crate::TestValueExt::test_value(engine.definitions.get(id))
            .script
            .unresolved_inherited_diagnostics()
    };
    let orphan = reported("ADEF");
    assert_eq!(orphan.len(), 1, "{orphan:?}");
    assert_eq!(orphan[0].function, "Orphan");
    assert!(
        reported("BDEF").is_empty(),
        "the engine hop resolves BDEF's inherited: {:?}",
        reported("BDEF")
    );
}

/// An `#appendto` host's `inherited` reaches the chain it is appended *into*,
/// so its overload target exists only once appends and includes have run. C4Aul
/// never sees the intermediate state — it binds `inherited` with every func
/// table already built (`C4AulParse.cpp:1406`) and the appended function's
/// `Fn->Owner` is the target — so judging the source host earlier reports a
/// function that resolves perfectly well, and truncating it there
/// (`C4AulParse.cpp:3553-3577`) would then be copied onto the target.
#[test]
fn an_appendto_hosts_hard_inherited_resolves_through_its_target() {
    let mut engine = Engine::new();
    register(&mut engine, "BASE", "#strict\nfunc Layer() { return 1; }");
    register(
        &mut engine,
        "APND",
        "#strict\n#appendto BASE\nfunc Layer() { return 10 + inherited(); }",
    );
    link_initial_scripts(&mut engine);

    let base = engine.spawn_test_object(SpawnConfig::new("BASE"));
    assert_eq!(
        call(&mut engine, base, "Layer"),
        Value::Int(11),
        "the appended body still reaches the base implementation"
    );
    // The append source's *own* copy has no overload of its own and raises,
    // exactly as it did before the truncation existed — only the wording moved
    // to C4Aul's. What must not happen is that copy being truncated first and
    // then linked onto the target, which is what judging the host before
    // `resolve_appends` produced.
    let own = engine.spawn_test_object(SpawnConfig::new("APND"));
    let index = crate::TestValueExt::test_value(engine.find_object_index(own));
    assert!(engine
        .call_object_function(index, "Layer", Vec::new())
        .is_err());
}

#[test]
fn initial_link_preparses_every_host_constant_before_function_literal_holds() {
    let mut engine = Engine::new();
    assert_eq!(
        engine.install_global_scripts(&[(
            "System/A.c".into(),
            "func Literal() { return \"a\"; }".into(),
        )]),
        1
    );
    assert!(
        engine.script_string_registration_order().is_empty(),
        "preparse must discard the earlier host's function body"
    );

    register(
        &mut engine,
        "LATE",
        "static const Later = \"b\";\nfunc Constant() { return Later; }",
    );
    assert_eq!(engine.script_string_registration_order(), ["b"]);

    link_initial_scripts(&mut engine);
    assert_eq!(
        engine.script_string_registration_order(),
        ["b", "a"],
        "the global Parse pass runs only after every host was preparsed"
    );
}

#[test]
fn initial_literal_hold_reuses_later_static_constant_identity() {
    let mut engine = Engine::new();
    assert_eq!(
        engine.install_global_scripts(&[(
            "System/A.c".into(),
            "func Literal() { return \"shared\"; }".into(),
        )]),
        1
    );
    assert!(engine.script_string_registration_order().is_empty());

    register(
        &mut engine,
        "LATE",
        "static const Shared = \"shared\";\nfunc Constant() { return Shared; }",
    );
    let constant =
        crate::TestValueExt::test_value(engine.script_global_consts.borrow().get("Shared"))
            .borrow()
            .clone();
    let Value::String(constant) = constant else {
        panic!("Shared is a string constant");
    };

    link_initial_scripts(&mut engine);
    let source =
        crate::TestValueExt::test_value(engine.script_link_sources.iter().find_map(|source| {
            match source {
                ScriptLinkSource::Script { name, script, .. } if name == "System/A.c" => {
                    Some(Arc::clone(script))
                }
                _ => None,
            }
        }));
    let Value::String(initial_literal) =
        crate::TestValueExt::test_value(source.call("Literal", &[]))
    else {
        panic!("Literal returns a string");
    };
    assert!(
        constant.ptr_eq(&initial_literal),
        "Parse must set Hold on the constant's preparsed identity"
    );

    crate::TestValueExt::test_value(engine.relink_scripts());
    let source =
        crate::TestValueExt::test_value(engine.script_link_sources.iter().find_map(|source| {
            match source {
                ScriptLinkSource::Script { name, script, .. } if name == "System/A.c" => {
                    Some(Arc::clone(script))
                }
                _ => None,
            }
        }));
    let Value::String(relinked_literal) =
        crate::TestValueExt::test_value(source.call("Literal", &[]))
    else {
        panic!("Literal returns a string");
    };
    assert!(
        !constant.ptr_eq(&relinked_literal),
        "Clear unregisters a held identity even while the constant still references it"
    );
}

#[test]
fn reload_rebuilds_append_include_copies_once_and_keeps_globals() {
    let mut engine = Engine::new();
    register(&mut engine, "INCA", "func Layer() { return 1; }");
    register(
        &mut engine,
        "INCB",
        "#strict\nfunc Layer() { return 10 + inherited(); }",
    );
    register(
        &mut engine,
        "BASE",
        "#strict 2\n\
             #include INCA\n\
             #include INCB\n\
             static Kept;\n\
             static const ReloadConst = 7;\n\
             func Seed() { Kept = 41; return Kept; }\n\
             func Globals() { return [Kept, ReloadConst]; }\n\
             func Layer() { return 100 + inherited(); }",
    );
    register(
        &mut engine,
        "APNX",
        "#strict\n#appendto BASE\nfunc Layer() { return 1000 + inherited(); }",
    );
    register(
        &mut engine,
        "APNY",
        "#strict\n#appendto BASE\nfunc Layer() { return 10000 + inherited(); }",
    );
    register(&mut engine, "CHLD", "#include BASE");

    crate::TestValueExt::test_value(engine.relink_scripts());
    let base = engine.spawn_test_object(SpawnConfig::new("BASE"));
    let child = engine.spawn_test_object(SpawnConfig::new("CHLD"));
    assert_eq!(call(&mut engine, base, "Layer"), Value::Int(11_111));
    assert_eq!(call(&mut engine, child, "Layer"), Value::Int(11_111));
    assert_eq!(call(&mut engine, base, "Seed"), Value::Int(41));

    assert!(engine
        .reload_definition_script(
            "APNY",
            "#strict\n#appendto BASE\nfunc Layer() { return 20000 + inherited(); }",
        )
        .expect("append source reloads"));
    assert_eq!(call(&mut engine, base, "Layer"), Value::Int(21_111));
    assert_eq!(call(&mut engine, child, "Layer"), Value::Int(21_111));

    assert!(engine
        .reload_definition_script(
            "BASE",
            "#strict 2\n\
                     #include INCA\n\
                     #include INCB\n\
                     static Kept;\n\
                     static const ReloadConst = 9;\n\
                     func Seed() { Kept = 99; return Kept; }\n\
                     func Globals() { return [Kept, ReloadConst]; }\n\
                     func Layer() { return 200 + inherited(); }",
        )
        .expect("base source reloads"));
    assert_eq!(call(&mut engine, base, "Layer"), Value::Int(21_211));
    assert_eq!(call(&mut engine, child, "Layer"), Value::Int(21_211));
    assert_eq!(
        call(&mut engine, base, "Globals"),
        Value::Array(vec![Value::Int(41), Value::Int(9)])
    );

    let (function_count, linked_function_count) = {
        let definition = crate::TestValueExt::test_value(engine.definitions.get("BASE"));
        (
            definition.function_count(),
            definition.linked_function_count(),
        )
    };
    crate::TestValueExt::test_value(engine.relink_scripts());
    let definition = crate::TestValueExt::test_value(engine.definitions.get("BASE"));
    assert_eq!(definition.function_count(), function_count);
    assert_eq!(definition.linked_function_count(), linked_function_count);
    assert_eq!(call(&mut engine, base, "Layer"), Value::Int(21_211));
    assert_eq!(
        call(&mut engine, base, "Globals"),
        Value::Array(vec![Value::Int(41), Value::Int(9)])
    );
    assert!(!engine
        .reload_definition_script("MISS", "func Nope() {}")
        .expect("unknown reload is a clean miss"));
}

#[test]
fn relink_replays_interleaved_global_hosts_and_declaring_links() {
    let mut engine = Engine::new();
    assert_eq!(
        engine.install_global_scripts(&[(
            "System/Base.c".into(),
            "global func GlobalLayer() { return 1; }".into(),
        )]),
        1
    );
    register(
        &mut engine,
        "OWNR",
        "#strict\n\
             global func GlobalLayer() { return inherited() * 10 + 2; }\n\
             func Probe() { return GlobalLayer(); }",
    );
    register(
        &mut engine,
        "CALL",
        "func Probe() { return GlobalLayer(); }",
    );
    crate::TestValueExt::test_value(engine.load_scenario_script_with_convention(
        "Scenario/Script.c",
        "#strict\n\
                 global func GlobalLayer() { return inherited() * 10 + 3; }\n\
                 func Probe() { return GlobalLayer(); }",
        true,
    ));
    assert_eq!(
        engine.install_scenario_global_scripts(&[(
            "Scenario/System/Last.c".into(),
            "#strict\nglobal func GlobalLayer() { return inherited() * 10 + 4; }".into(),
        )]),
        1
    );
    crate::TestValueExt::test_value(engine.relink_scripts());

    let owner = engine.spawn_test_object(SpawnConfig::new("OWNR"));
    let caller = engine.spawn_test_object(SpawnConfig::new("CALL"));
    assert_eq!(call(&mut engine, owner, "Probe"), Value::Int(1_234));
    assert_eq!(call(&mut engine, caller, "Probe"), Value::Int(1_234));
    assert_eq!(
        engine
            .scenario_script
            .as_ref()
            .expect("scenario remains installed")
            .script
            .call("Probe", &[])
            .expect("scenario probe runs"),
        Value::Int(1_234)
    );

    let counts = engine
        .definitions
        .iter()
        .map(|(id, definition)| (id.clone(), definition.linked_function_count()))
        .collect::<HashMap<_, _>>();
    crate::TestValueExt::test_value(engine.relink_scripts());
    for (id, count) in counts {
        assert_eq!(
            engine
                .definitions
                .get(&id)
                .expect("definition remains")
                .linked_function_count(),
            count
        );
    }
    assert_eq!(call(&mut engine, owner, "Probe"), Value::Int(1_234));
    assert_eq!(call(&mut engine, caller, "Probe"), Value::Int(1_234));
}

#[test]
fn declaring_definition_calls_use_the_latest_engine_global_chain() {
    for (later_source, expected) in [
        ("global func F() { return 2; }", 2),
        ("#strict\nglobal func F() { return _inherited() + 10; }", 11),
    ] {
        let mut engine = Engine::new();
        register(
            &mut engine,
            "GFA1",
            "#strict 2\n\
                 global func F() { return 1; }\n\
                 func CallF() { return F(); }",
        );
        register(&mut engine, "GFB1", later_source);
        crate::TestValueExt::test_value(engine.relink_scripts());
        let declaring = engine.spawn_test_object(SpawnConfig::new("GFA1"));
        assert_eq!(
            call(&mut engine, declaring, "CallF"),
            Value::Int(expected),
            "later declaration: {later_source}",
        );
    }
}

#[test]
fn scenario_script_calls_use_the_later_scenario_system_global() {
    let mut engine = Engine::new();
    crate::TestValueExt::test_value(engine.load_scenario_script_with_convention(
        "Scenario/Script.c",
        "#strict 2\n\
                 global func F() { return 1; }\n\
                 func CallF() { return F(); }",
        true,
    ));
    assert_eq!(
        engine.install_scenario_global_scripts(&[(
            "Scenario/System/Override.c".into(),
            "global func F() { return 2; }".into(),
        )]),
        1,
    );
    crate::TestValueExt::test_value(engine.relink_scripts());

    assert_eq!(
        engine
            .scenario_script
            .as_ref()
            .expect("scenario script remains installed")
            .script
            .call("CallF", &[])
            .expect("scenario CallF runs"),
        Value::Int(2),
    );
}

#[test]
fn relink_keeps_global_resort_lookup_bound_to_the_declaring_definition() {
    let mut engine = Engine::new();
    register(
        &mut engine,
        "ADEF",
        "global func Queue() { return ResortObjects(\"Cmp\"); }\n\
             func Cmp(object first, object second) { return -11; }",
    );
    register(
        &mut engine,
        "BDEF",
        "func Cmp(object first, object second) { return 22; }\n\
             func Trigger() { return Queue(); }",
    );
    crate::TestValueExt::test_value(engine.relink_scripts());
    let declaring_script =
        crate::TestValueExt::test_value(engine.definitions.get("ADEF")).script_arc();
    let destination_script =
        crate::TestValueExt::test_value(engine.definitions.get("BDEF")).script_arc();
    let caller = engine.spawn_test_object(SpawnConfig::new("BDEF"));

    assert_eq!(call(&mut engine, caller, "Trigger"), Value::Bool(true));
    let [ObjectOrderCommand::OrderFuncAll { order, category }] =
        engine.pending_object_order_commands.as_slice()
    else {
        panic!(
            "unexpected relinked order queue: {:?}",
            engine.pending_object_order_commands
        );
    };
    assert_eq!(order.host_identity, declaring_script.host_identity());
    assert_ne!(order.host_identity, destination_script.host_identity());
    assert_eq!(order.script_name, "ADEF");
    assert_eq!(order.definition_context.as_deref(), Some("ADEF"));
    assert_eq!(order.function, "Cmp");
    assert_eq!(*category, CATEGORY_SORT_LIMIT);
}

#[test]
fn retained_system_host_owns_and_executes_its_local_resort_comparator() {
    let mut engine = Engine::new();
    assert_eq!(
        engine.install_global_scripts(&[(
            "System/Order.c".into(),
            "global func Queue() { return ResortObjects(\"Cmp\"); }\n\
                 func Cmp(object first, object second) { return -1; }"
                .into(),
        )]),
        1
    );
    register(
        &mut engine,
        "BDEF",
        "func Cmp(object first, object second) { return 1; }\n\
             func Trigger() { return Queue(); }",
    );
    crate::TestValueExt::test_value(engine.relink_scripts());
    let system_script =
        crate::TestValueExt::test_value(engine.script_link_sources.iter().find_map(|source| {
            match source {
                ScriptLinkSource::Script { name, script, .. } if name == "System/Order.c" => {
                    Some(Arc::clone(script))
                }
                _ => None,
            }
        }));
    let first = engine.spawn_test_object(SpawnConfig::new("BDEF"));
    let second = engine.spawn_test_object(SpawnConfig::new("BDEF"));
    assert_eq!(engine.debug_exec_order(), [first, second]);

    assert_eq!(call(&mut engine, first, "Trigger"), Value::Bool(true));
    let [ObjectOrderCommand::OrderFuncAll { order, .. }] =
        engine.pending_object_order_commands.as_slice()
    else {
        panic!("expected one System-host OrderFunc request");
    };
    assert_eq!(order.host_identity, system_script.host_identity());
    assert_eq!(order.script_name, "System/Order.c");
    assert_eq!(order.definition_context, None);

    engine.execute_object_order_commands();
    assert_eq!(
        engine.debug_exec_order(),
        [second, first],
        "the System-local -1 comparator wins over BDEF's +1 comparator"
    );
}

#[test]
fn global_resort_comparator_executes_without_a_definition_context() {
    let mut engine = Engine::new();
    register(
        &mut engine,
        "ADEF",
        "global func Queue() { return ResortObjects(\"Cmp\"); }\n\
             global func Helper(object obj) { return 1; }",
    );
    register(
        &mut engine,
        "BDEF",
        // Both helpers are `global` because `Cmp` is engine-owned and
        // therefore resolves identifiers in the ENGINE table, not in its
        // declaring definition (C4AulParse.cpp:2818-2823). BDEF registers
        // after ADEF, and the engine's function map head-inserts same-name
        // entries (C4Aul.cpp:76-79, :613-629), so BDEF's is the one found.
        // The sorted object arrives as a parameter rather than through an
        // implicit context, because a comparator runs with cthr->Def == null.
        "global func Cmp(object first, object second) {\n\
                 return Helper(first);\n\
             }\n\
             global func Helper(object obj) {\n\
                 if (GetID(obj) == BDEF) return -1;\n\
                 return 1;\n\
             }\n\
             func Trigger() { return Queue(); }",
    );
    crate::TestValueExt::test_value(engine.relink_scripts());
    let declaring_script =
        crate::TestValueExt::test_value(engine.definitions.get("BDEF")).script_arc();
    let first = engine.spawn_test_object(SpawnConfig::new("BDEF"));
    let second = engine.spawn_test_object(SpawnConfig::new("BDEF"));
    assert_eq!(engine.debug_exec_order(), [first, second]);

    assert_eq!(call(&mut engine, first, "Trigger"), Value::Bool(true));
    let [ObjectOrderCommand::OrderFuncAll { order, .. }] =
        engine.pending_object_order_commands.as_slice()
    else {
        panic!("expected one global-comparator OrderFunc request");
    };
    assert_eq!(order.definition_context, None);
    assert!(order.engine_global);
    assert_eq!(order.host_identity, declaring_script.host_identity());
    assert_eq!(order.script_name, "BDEF");

    engine.execute_object_order_commands();
    assert_eq!(
        engine.debug_exec_order(),
        [second, first],
        "BDEF's global Cmp resolves the newest engine-table Helper, not ADEF's older one"
    );
}

#[test]
fn queued_global_resort_pins_its_body_across_relink() {
    let mut engine = Engine::new();
    register(
        &mut engine,
        "ADEF",
        "global func Queue() { return ResortObjects(\"Cmp\"); }",
    );
    register(
        &mut engine,
        "BDEF",
        "global func Cmp(object first, object second) { return -1; }\n\
             func Trigger() { return Queue(); }",
    );
    crate::TestValueExt::test_value(engine.relink_scripts());
    let first = engine.spawn_test_object(SpawnConfig::new("BDEF"));
    let second = engine.spawn_test_object(SpawnConfig::new("BDEF"));
    assert_eq!(call(&mut engine, first, "Trigger"), Value::Bool(true));

    assert!(engine
        .reload_definition_script(
            "BDEF",
            "global func Cmp(object first, object second) { return 1; }\n\
                     func Trigger() { return Queue(); }",
        )
        .expect("comparator definition reloads"));
    engine.execute_object_order_commands();
    assert_eq!(
        engine.debug_exec_order(),
        [second, first],
        "the queued -1 body wins over the reloaded +1 function"
    );
}

#[test]
fn reloaded_definition_globals_move_to_the_engine_function_tail() {
    let mut engine = Engine::new();
    assert_eq!(
        engine.install_global_scripts(&[(
            "System/Base.c".into(),
            "global func Layer() { return 1; }".into(),
        )]),
        1
    );
    register(
        &mut engine,
        "EARL",
        "#strict\n\
             global func Layer() { return inherited() * 10 + 2; }\n\
             func Own() { return Layer(); }",
    );
    register(
        &mut engine,
        "LATE",
        "#strict\n\
             global func Layer() { return inherited() * 10 + 3; }\n\
             func Own() { return Layer(); }",
    );
    register(&mut engine, "CALL", "func Probe() { return Layer(); }");
    crate::TestValueExt::test_value(engine.relink_scripts());
    let early = engine.spawn_test_object(SpawnConfig::new("EARL"));
    let late = engine.spawn_test_object(SpawnConfig::new("LATE"));
    let caller = engine.spawn_test_object(SpawnConfig::new("CALL"));
    assert_eq!(call(&mut engine, early, "Own"), Value::Int(123));
    assert_eq!(call(&mut engine, late, "Own"), Value::Int(123));
    assert_eq!(call(&mut engine, caller, "Probe"), Value::Int(123));

    assert!(engine
        .reload_definition_script(
            "EARL",
            "#strict\n\
                     global func Layer() { return inherited() * 10 + 4; }\n\
                     func Own() { return Layer(); }",
        )
        .expect("early definition reloads"));
    assert_eq!(call(&mut engine, early, "Own"), Value::Int(134));
    assert_eq!(call(&mut engine, late, "Own"), Value::Int(134));
    assert_eq!(call(&mut engine, caller, "Probe"), Value::Int(134));

    crate::TestValueExt::test_value(engine.relink_scripts());
    assert_eq!(call(&mut engine, early, "Own"), Value::Int(134));
    assert_eq!(call(&mut engine, late, "Own"), Value::Int(134));
    assert_eq!(call(&mut engine, caller, "Probe"), Value::Int(134));
}

#[test]
fn relink_resets_include_metadata_and_refreshes_step_flags() {
    let mut engine = Engine::new();
    register(&mut engine, "PRAA", "func Step() { return 1; }");
    register(&mut engine, "PRBB", "");
    register(&mut engine, "META", "#include PRAA\n#include PRBB");
    register(&mut engine, "APNM", "#appendto META");
    crate::TestValueExt::test_value(engine.definitions.get_mut("PRAA"))
        .set_clonk_names(Some("Alpha".into()));
    crate::TestValueExt::test_value(engine.definitions.get_mut("PRBB"))
        .set_clonk_names(Some("Beta".into()));
    crate::TestValueExt::test_value(engine.definitions.get_mut("APNM"))
        .set_clonk_names(Some("Append".into()));

    crate::TestValueExt::test_value(engine.relink_scripts());
    let child = crate::TestValueExt::test_value(engine.definitions.get("META"));
    assert_eq!(child.clonk_names(), Some("Alpha"));
    assert!(child.has_step);

    assert!(engine
        .reload_definition_script("META", "#include PRBB")
        .expect("child source reloads"));
    let child = crate::TestValueExt::test_value(engine.definitions.get("META"));
    assert_eq!(child.clonk_names(), Some("Beta"));
    assert!(!child.has_step);

    assert!(engine
        .reload_definition_script("META", "")
        .expect("include removal reloads"));
    let child = crate::TestValueExt::test_value(engine.definitions.get("META"));
    assert_eq!(child.clonk_names(), Some("Append"));
    assert!(!child.has_step);

    crate::TestValueExt::test_value(engine.relink_scripts());
    assert_eq!(
        engine
            .definitions
            .get("META")
            .expect("child definition")
            .clonk_names(),
        Some("Append")
    );
}
