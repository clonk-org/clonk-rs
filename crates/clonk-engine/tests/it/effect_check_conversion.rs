use clonk_engine::math::{itofix_prec, FixedVec2};
use clonk_engine::{Definition, EffectState, Engine, SpawnConfig};
use clonk_script::Value;

#[test]
fn real_c4_effect_call_preserves_object_identity_and_object_only_hosts() {
    // `C4Effect::DoCall` passes the affected C4Object first and preserves the
    // by-value FnEffectCall extras (src/C4Effect.cpp:439-457;
    // src/C4Script.cpp:5583-5595). Real loaded C4Script uses the C++ callback
    // argument convention, not the synthetic command-DSL state proplist.
    let mut engine = Engine::new();
    let mut host = Definition::from_script(
        "C4HP",
        "Real C4 effect callback host",
        r#"#strict 2
func Probe()
{
  var number = AddEffect("Canonical", this(), 100, 0, 0, C4CB);
  return(EffectCall(this(), number, "Probe", this()));
}
"#,
    )
    .expect("real C4 host compiles");
    host.set_c4_callback_convention(true);
    engine
        .register_definition(host)
        .expect("real C4 host registers");
    let mut callback = Definition::from_script(
        "C4CB",
        "Real C4 effect callback",
        r#"#strict 2
func FxCanonicalProbe(object target, int number, int declared_but_unused)
{
  var id_matches = GetID(target) == C4HP;
  var same_object = target == declared_but_unused;
  var type_is_object = GetType(target) == 4;
  GetNeededMatStr(target);
  SetXDir(17, target);
  return([id_matches, same_object, type_is_object]);
}
"#,
    )
    .expect("real C4 callback compiles");
    callback.set_c4_callback_convention(true);
    engine
        .register_definition(callback)
        .expect("real C4 callback registers");

    let object = engine
        .spawn_object(SpawnConfig::new("C4HP"))
        .expect("real C4 carrier spawns");
    let index = engine
        .find_object_index(object)
        .expect("real C4 carrier remains live");
    assert_eq!(
        engine
            .call_object_function(index, "Probe", Vec::new())
            .expect("pre-strict3 effect callback warns and continues"),
        Value::Array(vec![
            Value::Bool(true),
            Value::Bool(true),
            Value::Bool(true)
        ]),
        "GetID, object equality, and GetType must see C4VObj rather than a proplist"
    );
    let snapshot = engine
        .object_snapshot(object)
        .expect("object-only callback host keeps carrier live");
    let fixed_velocity = snapshot
        .fixed_velocity
        .unwrap_or_else(|| FixedVec2::from_ints(snapshot.velocity.x, snapshot.velocity.y));
    assert_eq!(fixed_velocity.x.val(), itofix_prec(17, 10).val());
}

#[test]
fn effect_call_object_local_callback_keeps_a_pre_strict3_object_argument() {
    // FnEffectCall retains its by-value extra arguments before C4Effect::DoCall
    // dispatches the selected Fx callback (src/C4Script.cpp:5583-5595;
    // src/C4Effect.cpp:439-457). C4AulFunc::Exec asks pre-STRICT3 callbacks to
    // warn, rather than abort, on a failed conversion (src/C4AulExec.cpp:1364-1397,
    // 1610-1627,1638-1656), so the declared-but-unused int still receives the
    // original object value.
    let mut engine = Engine::new();
    engine
        .register_script_definition(
            "OLOC",
            "Object-local EffectCall conversion probe",
            r#"#strict 2
func Probe()
{
  AddEffect("Aura", this(), 100, 0, this());
  return(EffectCall(this(), GetEffect("Aura", this()), "Probe", this()));
}

func FxAuraProbe(object target, int number, int declared_but_unused)
{
  return(declared_but_unused);
}
"#,
        )
        .expect("object-local EffectCall probe registers");
    let object = engine
        .spawn_object(SpawnConfig::new("OLOC"))
        .expect("probe object spawns");
    let index = engine
        .find_object_index(object)
        .expect("probe object remains live");

    assert_eq!(
        engine
            .call_object_function(index, "Probe", Vec::new())
            .expect("pre-strict3 effect callback warns and continues"),
        Value::Object(object.as_u64())
    );
}

#[test]
fn deferred_effect_timer_keeps_a_pre_strict3_object_argument() {
    // C4Effect::Execute invokes Fx*Timer with the affected object first
    // (src/C4Effect.cpp:319-363, especially :345). Like the constructor,
    // C4AulScriptFunc::Exec applies warning-only conversion for an effect
    // callback below #strict 3 (src/C4AulExec.cpp:1610-1627,1638-1656).
    let mut engine = Engine::new();
    engine
        .register_script_definition(
            "DTMR",
            "Deferred timer conversion probe",
            r#"#strict 2
local timer_calls;

func Install()
{
  return(AddEffect("Pulse", this(), 100, 1, this()));
}

func FxPulseTimer(int declared_but_unused, int number, int time)
{
  ++timer_calls;
  return(-1);
}

func ReadTimerCalls()
{
  return(timer_calls);
}
"#,
        )
        .expect("deferred timer probe registers");
    let object = engine
        .spawn_object(SpawnConfig::new("DTMR"))
        .expect("probe object spawns");
    let index = engine
        .find_object_index(object)
        .expect("probe object remains live");
    engine
        .call_object_function(index, "Install", Vec::new())
        .expect("effect installs");

    engine.tick().expect("deferred timer dispatch succeeds");

    let index = engine
        .find_object_index(object)
        .expect("timer callback keeps object live");
    assert_eq!(
        engine
            .call_object_function(index, "ReadTimerCalls", Vec::new())
            .expect("timer-call counter reads"),
        Value::Int(1)
    );
}

#[test]
fn effect_call_reaches_every_callback_carrier_with_pre_strict3_warning_conversion() {
    // C4Effect selects the callback source through command target, command
    // ID, or Game.ScriptEngine (src/C4Effect.cpp:31-57), then `DoCall`
    // invokes the resolved function with its owned callback values
    // (src/C4Effect.cpp:439-457). Pre-STRICT3 function conversion warns and
    // keeps the original value (src/C4AulExec.cpp:1364-1397,1610-1627,
    // 1638-1656).
    let mut engine = Engine::new();
    engine
        .register_script_definition(
            "ELOC",
            "Definition-local EffectCall carrier",
            r#"#strict 2
func FxDefinitionLocalProbe(object target, int number, int declared_but_unused)
{
  return(declared_but_unused);
}
"#,
        )
        .expect("definition-local carrier registers");
    engine
        .register_script_definition(
            "EGLB",
            "Definition-global EffectCall carrier",
            r#"#strict 2
global func FxDefinitionGlobalProbe(object target, int number, int declared_but_unused)
{
  return(declared_but_unused);
}
"#,
        )
        .expect("definition-global carrier registers");
    assert_eq!(
        engine.install_additional_global_scripts(&[(
            "Issue62System.c".to_string(),
            r#"#strict 2
global func FxEngineGlobalProbe(object target, int number, int declared_but_unused)
{
  return(declared_but_unused);
}
"#
            .to_string(),
        )]),
        1
    );
    engine
        .register_script_definition(
            "ECRR",
            "EffectCall carrier driver",
            r#"#strict 2
func FxObjectLocalProbe(object target, int number, int declared_but_unused)
{
  return(declared_but_unused);
}

global func FxObjectGlobalProbe(object target, int number, int declared_but_unused)
{
  return(declared_but_unused);
}

func Probe()
{
  var object_local = AddEffect("ObjectLocal", this(), 500, 0, this());
  var object_global = AddEffect("ObjectGlobal", this(), 500, 0, this());
  var definition_local = AddEffect("DefinitionLocal", this(), 500, 0, 0, ELOC);
  var definition_global = AddEffect("DefinitionGlobal", this(), 500, 0, 0, EGLB);
  var engine_global = AddEffect("EngineGlobal", this(), 500, 0);
  return([
    EffectCall(this(), object_local, "Probe", this()),
    EffectCall(this(), object_global, "Probe", this()),
    EffectCall(this(), definition_local, "Probe", this()),
    EffectCall(this(), definition_global, "Probe", this()),
    EffectCall(this(), engine_global, "Probe", this())
  ]);
}
"#,
        )
        .expect("carrier driver registers");
    let object = engine
        .spawn_object(SpawnConfig::new("ECRR"))
        .expect("carrier driver spawns");
    let index = engine
        .find_object_index(object)
        .expect("carrier driver remains live");

    assert_eq!(
        engine
            .call_object_function(index, "Probe", Vec::new())
            .expect("all pre-strict3 callback carriers warn and continue"),
        Value::Array(vec![Value::Object(object.as_u64()); 5])
    );
}

#[test]
fn effect_callback_warning_is_pre_strict3_only_and_does_not_leak_to_nested_calls() {
    // C4AulScriptFunc::Exec enables warning-only conversion only when the
    // selected callback is below strict 3 (src/C4AulExec.cpp:1610-1627,
    // 1638-1656). The callback's C4AulParSet owns values, so a strict-3 `&`
    // parameter rejects rather than aliasing EffectCall's argument
    // (src/C4Script.cpp:5583-5595).
    let strict_callback = r#"#strict 3
local reference_callback_ran;

func CallInt()
{
  AddEffect("Int", this(), 100, 0, this());
  return(EffectCall(this(), GetEffect("Int", this()), "Probe", this()));
}

func FxIntProbe(object target, int number, int declared_but_unused)
{
  return(declared_but_unused);
}

func CallReference()
{
  AddEffect("Reference", this(), 100, 0, this());
  return(EffectCall(this(), GetEffect("Reference", this()), "Probe", this()));
}

func FxReferenceProbe(object target, int number, &declared_but_unused)
{
  reference_callback_ran = 1;
  declared_but_unused = nil;
  return(0);
}

func ReadReferenceCallbackRan()
{
  return(reference_callback_ran);
}

func OrdinaryStrictInt(int value)
{
  return(value);
}

func OuterEffectCallKeepsStrict()
{
  AddEffect("Outer", this(), 100, 0, this());
  return(EffectCall(this(), this(), "Probe", this()));
}

func FxOuterProbe(object target, int number, object value)
{
  return(value);
}
"#;
    let mut engine = Engine::new();
    engine
        .register_script_definition("S3FX", "Strict effect callback probe", strict_callback)
        .expect("strict callback probe registers");
    let object = engine
        .spawn_object(SpawnConfig::new("S3FX"))
        .expect("strict callback object spawns");
    let index = engine
        .find_object_index(object)
        .expect("strict callback object remains live");

    for (function, callback, expected) in [
        ("CallInt", "FxIntProbe", r#"expected \"int\""#),
        ("CallReference", "FxReferenceProbe", r#"expected \"&\""#),
    ] {
        let error = engine
            .call_object_function(index, function, Vec::new())
            .expect_err("strict-3 callback conversion remains fatal");
        let diagnostic = format!("{error:?}");
        assert!(
            diagnostic.contains(callback) && diagnostic.contains(expected),
            "unexpected strict-3 callback diagnostic: {diagnostic}"
        );
    }
    assert_eq!(
        engine
            .call_object_function(index, "ReadReferenceCallbackRan", Vec::new())
            .expect("reference callback state reads"),
        Value::Nil,
        "a rejected owned value never enters the `&` callback body"
    );
    let ordinary_error = engine
        .call_object_function(
            index,
            "OrdinaryStrictInt",
            vec![Value::Object(object.as_u64())],
        )
        .expect_err("ordinary host calls remain strict");
    assert!(
        format!("{ordinary_error:?}").contains(r#"expected \"int\""#),
        "the effect marker does not leak into ordinary calls"
    );
    let outer_error = engine
        .call_object_function(index, "OuterEffectCallKeepsStrict", Vec::new())
        .expect_err("the outer EffectCall host invocation remains ordinary");
    let outer_diagnostic = format!("{outer_error:?}");
    assert!(
        outer_diagnostic.contains("EffectCall") && outer_diagnostic.contains(r#"expected \"int\""#),
        "the effect marker must not loosen EffectCall's own native arguments: {outer_diagnostic}"
    );

    let mut nested_engine = Engine::new();
    nested_engine
        .register_script_definition(
            "NSTC",
            "Nested callback conversion probe",
            r#"#strict 2
func Call()
{
  AddEffect("Nested", this(), 100, 0, this());
  return(EffectCall(this(), GetEffect("Nested", this()), "Probe", this()));
}

func FxNestedProbe(object target, int number, int declared_but_unused)
{
  return(NestedStrictInt(declared_but_unused));
}

func NestedStrictInt(int value)
{
  return(value);
}
"#,
        )
        .expect("nested conversion probe registers");
    let object = nested_engine
        .spawn_object(SpawnConfig::new("NSTC"))
        .expect("nested conversion object spawns");
    let index = nested_engine
        .find_object_index(object)
        .expect("nested conversion object remains live");
    let nested_error = nested_engine
        .call_object_function(index, "Call", Vec::new())
        .expect_err("nested call does not inherit effect callback warning mode");
    assert!(
        format!("{nested_error:?}").contains("NestedStrictInt")
            && format!("{nested_error:?}").contains(r#"expected \"int\""#),
        "nested script call must retain ordinary conversion behavior"
    );
}

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
