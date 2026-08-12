use clonk_engine::{Engine, SpawnConfig, Vector2};
use clonk_script::Value;

#[test]
fn strict3_global_call_bypasses_local_override_and_clears_implicit_object_context() {
    let script = r#"#strict 3
local marker;
func GetX() { return 777; }
func Probe() { return [GetX(), global->GetX(), global->GetX(this()), global->GetID(), GetID()]; }
func ProbeBuiltins() {
    Local(0) = 9;
    marker = 10;
    return [
        global->this(),
        global->Local(0),
        global->Local(0, this()),
        global->LocalN("marker"),
        global->LocalN("marker", this()),
        global->SetLocal(0, 11),
        global->SetLocal(0, 12, this()),
        Local(0),
        global->eval("this()")
    ];
}
global func ProbeGlobalFrame() {
    return [this(), Local(0), LocalN("marker"), SetLocal(0, 13), eval("this()")];
}
func ProbeGlobalWrapper() { return global->ProbeGlobalFrame(); }
"#;
    let mut engine = Engine::new();
    crate::support::TestValueExt::test_value(engine.register_script_definition(
        "GLOB",
        "Global-call probe",
        script,
    ));
    let object = crate::support::TestValueExt::test_value(
        engine.spawn_object(SpawnConfig::new("GLOB").with_position(Vector2::new(42, 100))),
    );
    let index = crate::support::TestValueExt::test_value(engine.find_object_index(object));

    assert_eq!(
        engine
            .call_object_function(index, "Probe", Vec::new())
            .expect("global calls run"),
        Value::Array(vec![
            Value::Int(777),
            Value::Nil,
            Value::Int(42),
            Value::Nil,
            Value::C4Id("GLOB".into()),
        ])
    );
    assert_eq!(
        engine
            .call_object_function(index, "ProbeBuiltins", Vec::new())
            .expect("global VM builtins run"),
        Value::Array(vec![
            Value::Nil,
            Value::Nil,
            Value::Int(9),
            Value::Nil,
            Value::Int(10),
            Value::Bool(false),
            Value::Int(12),
            Value::Int(12),
            Value::Nil,
        ])
    );
    assert_eq!(
        engine
            .call_object_function(index, "ProbeGlobalWrapper", Vec::new())
            .expect("global script frame runs without object context"),
        Value::Array(vec![
            Value::Nil,
            Value::Nil,
            Value::Nil,
            Value::Bool(false),
            Value::Nil,
        ])
    );
}
