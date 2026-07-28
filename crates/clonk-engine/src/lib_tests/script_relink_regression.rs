use super::*;

fn register(engine: &mut Engine, id: &str, source: &str) {
    engine
        .register_script_definition(id, id, source)
        .expect("fixture definition registers");
}

fn call(engine: &mut Engine, object: ObjectId, function: &str) -> Value {
    let index = engine.find_object_index(object).expect("object exists");
    engine
        .call_object_function(index, function, Vec::new())
        .expect("fixture function runs")
}

fn link_initial_scripts(engine: &mut Engine) {
    engine.resolve_appends();
    engine.resolve_includes().expect("initial scripts link");
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
    let constant = engine
        .script_global_consts
        .borrow()
        .get("Shared")
        .expect("constant was preparsed")
        .borrow()
        .clone();
    let Value::String(constant) = constant else {
        panic!("Shared is a string constant");
    };

    link_initial_scripts(&mut engine);
    let source = engine
        .script_link_sources
        .iter()
        .find_map(|source| match source {
            ScriptLinkSource::Script { name, script, .. } if name == "System/A.c" => {
                Some(Arc::clone(script))
            }
            _ => None,
        })
        .expect("system host remains installed");
    let Value::String(initial_literal) = source.call("Literal", &[]).expect("literal runs") else {
        panic!("Literal returns a string");
    };
    assert!(
        constant.ptr_eq(&initial_literal),
        "Parse must set Hold on the constant's preparsed identity"
    );

    engine
        .relink_scripts()
        .expect("native Clear/reparse succeeds");
    let source = engine
        .script_link_sources
        .iter()
        .find_map(|source| match source {
            ScriptLinkSource::Script { name, script, .. } if name == "System/A.c" => {
                Some(Arc::clone(script))
            }
            _ => None,
        })
        .expect("system host remains installed");
    let Value::String(relinked_literal) = source.call("Literal", &[]).expect("literal runs") else {
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

    engine.relink_scripts().expect("initial scripts link");
    let base = engine
        .spawn_object(SpawnConfig::new("BASE"))
        .expect("base object spawns");
    let child = engine
        .spawn_object(SpawnConfig::new("CHLD"))
        .expect("child object spawns");
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
        let definition = engine.definitions.get("BASE").expect("base definition");
        (
            definition.function_count(),
            definition.linked_function_count(),
        )
    };
    engine.relink_scripts().expect("second relink succeeds");
    let definition = engine.definitions.get("BASE").expect("base definition");
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
    engine
        .load_scenario_script_with_convention(
            "Scenario/Script.c",
            "#strict\n\
                 global func GlobalLayer() { return inherited() * 10 + 3; }\n\
                 func Probe() { return GlobalLayer(); }",
            true,
        )
        .expect("scenario script loads");
    assert_eq!(
        engine.install_scenario_global_scripts(&[(
            "Scenario/System/Last.c".into(),
            "#strict\nglobal func GlobalLayer() { return inherited() * 10 + 4; }".into(),
        )]),
        1
    );
    engine.relink_scripts().expect("scripts relink");

    let owner = engine
        .spawn_object(SpawnConfig::new("OWNR"))
        .expect("owner object spawns");
    let caller = engine
        .spawn_object(SpawnConfig::new("CALL"))
        .expect("caller object spawns");
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
    engine.relink_scripts().expect("repeat relink succeeds");
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
        engine.relink_scripts().expect("global functions relink");
        let declaring = engine
            .spawn_object(SpawnConfig::new("GFA1"))
            .expect("declaring object spawns");
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
    engine
        .load_scenario_script_with_convention(
            "Scenario/Script.c",
            "#strict 2\n\
                 global func F() { return 1; }\n\
                 func CallF() { return F(); }",
            true,
        )
        .expect("scenario script loads first");
    assert_eq!(
        engine.install_scenario_global_scripts(&[(
            "Scenario/System/Override.c".into(),
            "global func F() { return 2; }".into(),
        )]),
        1,
    );
    engine.relink_scripts().expect("scenario scripts relink");

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
    engine.relink_scripts().expect("scripts relink");
    let declaring_script = engine
        .definitions
        .get("ADEF")
        .expect("declaring definition remains")
        .script_arc();
    let destination_script = engine
        .definitions
        .get("BDEF")
        .expect("destination definition remains")
        .script_arc();
    let caller = engine
        .spawn_object(SpawnConfig::new("BDEF"))
        .expect("destination object spawns");

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
    engine.relink_scripts().expect("system scripts relink");
    let system_script = engine
        .script_link_sources
        .iter()
        .find_map(|source| match source {
            ScriptLinkSource::Script { name, script, .. } if name == "System/Order.c" => {
                Some(Arc::clone(script))
            }
            _ => None,
        })
        .expect("retained System host exists");
    let first = engine
        .spawn_object(SpawnConfig::new("BDEF"))
        .expect("first destination object spawns");
    let second = engine
        .spawn_object(SpawnConfig::new("BDEF"))
        .expect("second destination object spawns");
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
             func Helper() { return 1; }",
    );
    register(
        &mut engine,
        "BDEF",
        "global func Cmp(object first, object second) {\n\
                 return Helper();\n\
             }\n\
             func Helper() {\n\
                 if (GetID() == BDEF) return -1;\n\
                 return 1;\n\
             }\n\
             func Trigger() { return Queue(); }",
    );
    engine.relink_scripts().expect("global comparator relinks");
    let declaring_script = engine
        .definitions
        .get("BDEF")
        .expect("comparator definition remains")
        .script_arc();
    let first = engine
        .spawn_object(SpawnConfig::new("BDEF"))
        .expect("first destination object spawns");
    let second = engine
        .spawn_object(SpawnConfig::new("BDEF"))
        .expect("second destination object spawns");
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
        "BDEF's global Cmp resolves BDEF's local Helper, not ADEF's conflicting helper"
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
    engine.relink_scripts().expect("global comparator relinks");
    let first = engine
        .spawn_object(SpawnConfig::new("BDEF"))
        .expect("first object spawns");
    let second = engine
        .spawn_object(SpawnConfig::new("BDEF"))
        .expect("second object spawns");
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
    engine.relink_scripts().expect("initial globals link");
    let early = engine
        .spawn_object(SpawnConfig::new("EARL"))
        .expect("early object spawns");
    let late = engine
        .spawn_object(SpawnConfig::new("LATE"))
        .expect("late object spawns");
    let caller = engine
        .spawn_object(SpawnConfig::new("CALL"))
        .expect("caller object spawns");
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

    engine.relink_scripts().expect("repeat relink succeeds");
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
    engine
        .definitions
        .get_mut("PRAA")
        .expect("first parent")
        .set_clonk_names(Some("Alpha".into()));
    engine
        .definitions
        .get_mut("PRBB")
        .expect("second parent")
        .set_clonk_names(Some("Beta".into()));
    engine
        .definitions
        .get_mut("APNM")
        .expect("append source")
        .set_clonk_names(Some("Append".into()));

    engine.relink_scripts().expect("includes link");
    let child = engine.definitions.get("META").expect("child definition");
    assert_eq!(child.clonk_names(), Some("Alpha"));
    assert!(child.has_step);

    assert!(engine
        .reload_definition_script("META", "#include PRBB")
        .expect("child source reloads"));
    let child = engine.definitions.get("META").expect("child definition");
    assert_eq!(child.clonk_names(), Some("Beta"));
    assert!(!child.has_step);

    assert!(engine
        .reload_definition_script("META", "")
        .expect("include removal reloads"));
    let child = engine.definitions.get("META").expect("child definition");
    assert_eq!(child.clonk_names(), Some("Append"));
    assert!(!child.has_step);

    engine.relink_scripts().expect("repeat relink succeeds");
    assert_eq!(
        engine
            .definitions
            .get("META")
            .expect("child definition")
            .clonk_names(),
        Some("Append")
    );
}
