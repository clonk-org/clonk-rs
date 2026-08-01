use clonk_engine::{EffectState, Engine, SpawnConfig};
use clonk_script::Value;

#[test]
fn conversion_policy_reaches_every_effect_check_callback_carrier() {
    // C4Effect retains one exact Fx callback but may execute it through an
    // object-local, object-global, definition-local, definition-global, or
    // Game.ScriptEngine carrier. Every branch receives the same owned-value
    // argument set and pre-STRICT3 warning policy (src/C4Effect.cpp:31-57,
    // 271-287,439-456).
    let mut engine = Engine::new();
    engine
        .register_script_definition(
            "DLOC",
            "Definition-local effect checker",
            r#"#strict 2
static definition_local_checks;
func FxDefinitionLocalEffect(id new_name)
{
  ++definition_local_checks;
  return(0);
}
global func ReadDefinitionLocalChecks() { return(definition_local_checks); }
global func ResetDefinitionLocalChecks() { definition_local_checks = 0; }
"#,
        )
        .expect("definition-local checker registers");
    engine
        .register_script_definition(
            "DGLB",
            "Definition-global effect checker",
            r#"#strict 2
static definition_global_checks;
global func FxDefinitionGlobalEffect(id new_name)
{
  ++definition_global_checks;
  return(0);
}
global func ReadDefinitionGlobalChecks() { return(definition_global_checks); }
global func ResetDefinitionGlobalChecks() { definition_global_checks = 0; }
"#,
        )
        .expect("definition-global checker registers");
    assert_eq!(
        engine.install_additional_global_scripts(&[(
            "Issue58System.c".to_string(),
            r#"#strict 2
static engine_global_checks;
global func FxEngineGlobalEffect(id new_name)
{
  ++engine_global_checks;
  return(0);
}
global func ReadEngineGlobalChecks() { return(engine_global_checks); }
global func ResetEngineGlobalChecks() { engine_global_checks = 0; }
"#
            .to_string(),
        )]),
        1
    );
    engine
        .register_script_definition(
            "CRRS",
            "Effect callback carrier probe",
            r#"#strict 2
local object_local_checks;
local object_global_checks;

func FxObjectLocalEffect(id new_name)
{
  ++object_local_checks;
  return(0);
}

global func FxObjectGlobalEffect(id new_name)
{
  ++object_global_checks;
  return(0);
}

func Install()
{
  AddEffect("ObjectLocal", this(), 500, 0, this());
  AddEffect("ObjectGlobal", this(), 500, 0, this());
  AddEffect("DefinitionLocal", this(), 500, 0, 0, DLOC);
  AddEffect("DefinitionGlobal", this(), 500, 0, 0, DGLB);
  AddEffect("EngineGlobal", this(), 500, 0);
}

func Probe()
{
  Install();
  object_local_checks = object_global_checks = 0;
  ResetDefinitionLocalChecks();
  ResetDefinitionGlobalChecks();
  ResetEngineGlobalChecks();
  var trigger = AddEffect("Trigger", this(), 100, 0, this(), 0, "Door");
  return([trigger, object_local_checks, object_global_checks,
          ReadDefinitionLocalChecks(), ReadDefinitionGlobalChecks(),
          ReadEngineGlobalChecks()]);
}
"#,
        )
        .expect("carrier probe registers");

    let probe = engine
        .spawn_object(SpawnConfig::new("CRRS"))
        .expect("carrier probe spawns");
    let index = engine
        .find_object_index(probe)
        .expect("carrier probe remains live");
    let result = engine
        .call_object_function(index, "Probe", Vec::new())
        .expect("all five checker carriers tolerate the legacy mismatch");
    let Value::Array(values) = result else {
        panic!("carrier probe returns an array, got {result:?}");
    };
    assert!(matches!(values.first(), Some(Value::Int(number)) if *number > 0));
    assert_eq!(
        &values[1..],
        &[
            Value::Int(1),
            Value::Int(1),
            Value::Int(1),
            Value::Int(1),
            Value::Int(1),
        ],
        "each callback carrier executes its strict-2 checker once"
    );
}

#[test]
fn global_effect_check_carriers_pass_values_to_strict3_reference_parameters() {
    // Both a definition-selected global and the Game.ScriptEngine fallback
    // receive C4Effect's owned C4Values. A strict-3 `&` checker errors; the
    // script AddEffect caller passes errors through instead of letting the
    // checker body deny the pending effect (src/C4Effect.cpp:271-287,439-456;
    // src/C4AulExec.cpp:1364-1397).
    let mut engine = Engine::new();
    engine
        .register_script_definition(
            "DREF",
            "Definition-global reference checker",
            r#"#strict 3
global func FxDefinitionReferenceEffect(&new_name) { return(-1); }
"#,
        )
        .expect("definition-global reference checker registers");
    assert_eq!(
        engine.install_additional_global_scripts(&[(
            "Issue58StrictSystem.c".to_string(),
            r#"#strict 3
global func FxEngineReferenceEffect(&new_name) { return(-1); }
"#
            .to_string(),
        )]),
        1
    );
    engine
        .register_script_definition(
            "GREF",
            "Global reference carrier driver",
            r#"#strict 3
func DefinitionGlobal()
{
  AddEffect("DefinitionReference", this(), 200, 0, nil, DREF);
  return(AddEffect("DefinitionPending", this(), 100, 0, this()));
}
func EngineGlobal()
{
  AddEffect("EngineReference", this(), 200, 0);
  return(AddEffect("EnginePending", this(), 100, 0, this()));
}
"#,
        )
        .expect("global reference carrier driver registers");

    for (function, callback) in [
        ("DefinitionGlobal", "FxDefinitionReferenceEffect"),
        ("EngineGlobal", "FxEngineReferenceEffect"),
    ] {
        let object = engine
            .spawn_object(SpawnConfig::new("GREF"))
            .expect("global reference probe spawns");
        let index = engine
            .find_object_index(object)
            .expect("global reference probe remains live");
        let error = engine
            .call_object_function(index, function, Vec::new())
            .expect_err("strict-3 reference mismatch remains fatal with value arguments");
        let diagnostic = format!("{error:?}");
        assert!(
            diagnostic.contains(callback)
                && diagnostic.contains(r#"got \"string\", but expected \"&\"!"#),
            "unexpected strict-3 checker error: {diagnostic}"
        );
    }
}

#[test]
fn fresh_spawn_effect_checks_use_real_callback_conversion_policy() {
    // A fresh SpawnConfig is host-created rather than script AddEffect, but
    // its EffectCommand::Add entries construct live effects and dispatch the
    // real definition callback through EffectEventKind::Check. Loaded save
    // effects skip callbacks entirely. The fresh path must therefore retain
    // C4Effect::Check's pre-STRICT3 conversion behavior rather than treating
    // it as a command-DSL-only fixture (src/C4Effect.cpp:271-287).
    let mut engine = Engine::new();
    engine
        .register_script_definition(
            "DFCK",
            "Fresh effect checker",
            r#"#strict 2
func FxGuardEffect(id new_name) { return(-1); }
"#,
        )
        .expect("fresh effect checker registers");
    let checker = EffectState::new("Guard")
        .with_priority(200)
        .with_command_id(Some("DFCK"));
    let pending = EffectState::new("Pending").with_priority(100);

    let object = engine
        .spawn_object(SpawnConfig::new("DFCK").with_effects(vec![checker, pending]))
        .expect("fresh object with effects spawns");
    let snapshot = engine
        .object_snapshot(object)
        .expect("fresh effect object remains live");
    assert_eq!(
        snapshot
            .effects
            .iter()
            .filter(|effect| effect.priority != 0)
            .map(|effect| effect.name.as_str())
            .collect::<Vec<_>>(),
        vec!["Guard"],
        "the real strict-2 checker runs and denies the lower-priority pending effect"
    );
}
