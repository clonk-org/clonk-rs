use super::*;

fn control(
    target_object: i32,
    strictness: ScriptStrictness,
    source: &str,
    by_client: i32,
) -> ScriptControlData {
    ScriptControlData {
        target_object,
        strictness,
        script: LegacyCString::from_bytes(source.as_bytes().to_vec())
            .expect("fixture script contains no NUL"),
        by_client,
    }
}

fn engine_with_probe() -> Engine {
    let mut engine = Engine::with_seed(7);
    assert_eq!(
        engine.install_global_scripts(&[(
            "ControlProbe.c".to_string(),
            "static ControlProbe;\n\
                 global func GlobalOnly() { return 37; }"
                .to_string(),
        )]),
        1
    );
    engine
}

fn global_value(engine: &Engine, name: &str) -> Value {
    let cell = engine
        .script_globals
        .borrow()
        .get(name)
        .cloned()
        .expect("fixture global is registered");
    let value = cell.borrow().clone();
    value
}

fn run_gate_case(
    league: bool,
    by_client: i32,
    policy: ScriptControlPolicy,
) -> (Option<Value>, Value) {
    let mut engine = engine_with_probe();
    engine.set_league_game(league);
    let result = engine
        .execute_script_control(
            &control(
                SCRIPT_SCOPE_GLOBAL,
                ScriptStrictness::Strict3,
                "ControlProbe = 17",
                by_client,
            ),
            policy,
        )
        .expect("script-control gate is not an engine error");
    (result, global_value(&engine, "ControlProbe"))
}

#[test]
fn script_control_gate_matches_league_host_console_and_replay_policy() {
    assert_eq!(
        run_gate_case(true, 0, ScriptControlPolicy::live(true)),
        (None, Value::Nil),
        "league blocks even a host with an active console"
    );
    assert_eq!(
        run_gate_case(false, 4, ScriptControlPolicy::live(false)),
        (None, Value::Nil),
        "a live non-host needs an active console"
    );
    assert_eq!(
        run_gate_case(
            false,
            4,
            ScriptControlPolicy {
                is_replay: true,
                console_active: true,
                allow_scripting_in_replays: false,
            },
        ),
        (None, Value::Nil),
        "replay permission does not fall back to Console.Active"
    );

    for (by_client, policy, label) in [
        (0, ScriptControlPolicy::live(false), "live host"),
        (0, ScriptControlPolicy::replay(false), "replayed host"),
        (4, ScriptControlPolicy::live(true), "live console peer"),
        (
            4,
            ScriptControlPolicy::replay(true),
            "permitted replay peer",
        ),
    ] {
        assert_eq!(
            run_gate_case(false, by_client, policy),
            (Some(Value::Int(17)), Value::Int(17)),
            "{label} executes"
        );
    }
}

#[test]
fn script_control_uses_packet_strictness_for_direct_exec() {
    for (strictness, expected) in [
        (ScriptStrictness::NonStrict, Value::Nil),
        (ScriptStrictness::Strict1, Value::Nil),
        (ScriptStrictness::Strict2, Value::Nil),
        (ScriptStrictness::Strict3, Value::Int(0)),
    ] {
        let mut engine = Engine::with_seed(1);
        assert_eq!(
            engine
                .execute_script_control(
                    &control(SCRIPT_SCOPE_GLOBAL, strictness, "0", 0),
                    ScriptControlPolicy::live(false),
                )
                .expect("strict expression executes"),
            Some(expected),
        );
    }

    let mut engine = Engine::with_seed(1);
    engine
        .register_definition(
            Definition::from_script("STC3", "Strict target", "#strict 3")
                .expect("strict target compiles"),
        )
        .expect("strict target registers");
    let object = engine
        .spawn_object(SpawnConfig::new("STC3"))
        .expect("strict target spawns");
    assert_eq!(
        engine
            .execute_script_control(
                &control(
                    i32::try_from(object.as_u64()).expect("fixture id fits i32"),
                    ScriptStrictness::NonStrict,
                    "0",
                    0,
                ),
                ScriptControlPolicy::live(false),
            )
            .expect("object expression executes"),
        Some(Value::Nil),
        "packet NONSTRICT overrides the destination definition's strict 3"
    );
}

#[test]
fn script_control_preserves_native_packet_source_bytes() {
    let mut engine = Engine::with_seed(1);
    let control = ScriptControlData {
        target_object: SCRIPT_SCOPE_GLOBAL,
        strictness: ScriptStrictness::Strict3,
        script: LegacyCString::from_bytes(vec![b'"', 0xe9, 0xff, b'"'])
            .expect("raw script packet is NUL-free"),
        by_client: 0,
    };
    let value = engine
        .execute_script_control(&control, ScriptControlPolicy::live(false))
        .expect("raw packet executes")
        .expect("host packet is accepted");
    assert_eq!(
        value,
        Value::String(clonk_script::c4_string_from_bytes(&[0xe9, 0xff]).into())
    );
}

#[test]
fn script_control_distinguishes_console_global_and_safe_object_scopes() {
    let mut engine = engine_with_probe();
    engine
        .install_scenario_script("Scenario", "#strict 3\nfunc ScenarioOnly() { return 41; }")
        .expect("scenario script installs");
    engine
        .register_definition(
            Definition::from_script(
                "TARG",
                "Target",
                "#strict 3\nlocal Marker;\nfunc ReadMarker() { return Marker; }",
            )
            .expect("target definition compiles"),
        )
        .expect("target definition registers");

    let normal = engine
        .spawn_object(SpawnConfig::new("TARG"))
        .expect("normal object spawns");
    let inactive = engine
        .spawn_object(SpawnConfig::new("TARG").with_status(ObjectStatus::Inactive))
        .expect("inactive object spawns");
    let deleted = engine
        .spawn_object(SpawnConfig::new("TARG"))
        .expect("deleted fixture initially spawns");
    let deleted_index = engine.find_object_index(deleted).expect("object exists");
    engine.objects[deleted_index].mark_destroyed();

    assert_eq!(
        engine
            .execute_script_control(
                &control(
                    SCRIPT_SCOPE_CONSOLE,
                    ScriptStrictness::Strict3,
                    "ScenarioOnly()",
                    0,
                ),
                ScriptControlPolicy::live(false),
            )
            .expect("console scope executes"),
        Some(Value::Int(41)),
        "console scope is the scenario-script host"
    );
    assert_eq!(
        engine
            .execute_script_control(
                &control(
                    SCRIPT_SCOPE_GLOBAL,
                    ScriptStrictness::Strict3,
                    "ScenarioOnly()",
                    0,
                ),
                ScriptControlPolicy::live(false),
            )
            .expect("missing global function is fail-safe"),
        Some(Value::Nil),
        "global scope must not see scenario-local functions"
    );
    assert_eq!(
        engine
            .execute_script_control(
                &control(
                    SCRIPT_SCOPE_GLOBAL,
                    ScriptStrictness::Strict3,
                    "GlobalOnly()",
                    0,
                ),
                ScriptControlPolicy::live(false),
            )
            .expect("global function executes"),
        Some(Value::Int(37)),
        "global scope is wired to the engine-global function table"
    );

    for (object, marker) in [(normal, 11), (inactive, 12)] {
        assert_eq!(
            engine
                .execute_script_control(
                    &control(
                        i32::try_from(object.as_u64()).expect("fixture id fits i32"),
                        ScriptStrictness::Strict3,
                        &format!("Marker = {marker}"),
                        0,
                    ),
                    ScriptControlPolicy::live(false),
                )
                .expect("object scope executes"),
            Some(Value::Int(marker)),
        );
        let index = engine
            .find_object_index(object)
            .expect("object remains present");
        assert_eq!(
            engine
                .call_object_function(index, "ReadMarker", Vec::new())
                .expect("object local can be read"),
            Value::Int(marker),
            "normal and inactive objects retain object-context writes"
        );
    }

    for (target, value) in [
        (999_999, 21),
        (
            i32::try_from(deleted.as_u64()).expect("fixture id fits i32"),
            22,
        ),
    ] {
        assert_eq!(
            engine
                .execute_script_control(
                    &control(
                        target,
                        ScriptStrictness::Strict3,
                        &format!("ControlProbe = {value}"),
                        0,
                    ),
                    ScriptControlPolicy::live(false),
                )
                .expect("missing/deleted scope falls back"),
            Some(Value::Int(value)),
        );
        assert_eq!(global_value(&engine, "ControlProbe"), Value::Int(value));
    }
}

#[test]
fn script_control_matches_cpp_global_and_object_state_differential() {
    // Frozen C++ differential for two CID_Script controls: global
    // `SetGravity(77)` leaves gravity at 77, then object-scoped
    // `SetPosition(12,34)` moves only the addressed object. Running the
    // identical expressions through Rust pins both scope and host-effect
    // folding without modifying the read-only C++ oracle.
    let mut engine = Engine::with_seed(1);
    engine
        .register_definition(
            Definition::from_script("DIFF", "Differential target", "#strict 3")
                .expect("target definition compiles"),
        )
        .expect("target definition registers");
    let object = engine
        .spawn_object(SpawnConfig::new("DIFF").with_position(Vector2::new(1, 2)))
        .expect("target object spawns");

    engine
        .execute_script_control(
            &control(
                SCRIPT_SCOPE_GLOBAL,
                ScriptStrictness::Strict3,
                "SetGravity(77)",
                0,
            ),
            ScriptControlPolicy::live(false),
        )
        .expect("global control executes");
    engine
        .execute_script_control(
            &control(
                i32::try_from(object.as_u64()).expect("fixture id fits i32"),
                ScriptStrictness::Strict3,
                "SetPosition(12,34)",
                0,
            ),
            ScriptControlPolicy::live(false),
        )
        .expect("object control executes");

    assert_eq!(engine.physics().gravity, 77);
    assert_eq!(
        engine
            .object_snapshot(object)
            .expect("target remains")
            .position,
        Vector2::new(12, 34)
    );
}
