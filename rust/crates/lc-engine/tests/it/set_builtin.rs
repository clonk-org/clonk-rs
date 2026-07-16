use lc_engine::{Definition, Engine, SpawnConfig};
use lc_script::Value;

fn call_probe(id: &str, script: &str) -> Value {
    let mut engine = Engine::new();
    engine
        .register_definition(
            Definition::from_script(id, "Set builtin probe", script)
                .expect("probe script compiles"),
        )
        .expect("probe definition registers");
    let object = engine
        .spawn_object(SpawnConfig::new(id))
        .expect("probe object spawns");
    let index = engine
        .find_object_index(object)
        .expect("probe object exists");
    engine
        .call_object_function(index, "Probe", Vec::new())
        .expect("probe runs")
}

#[test]
fn set_builtin_writes_var_numbered_local_and_named_local_references() {
    let result = call_probe(
        "SETB",
        r#"#strict
local x;

protected func Probe()
{
    var var_result = Set(Var(0), 7);
    var local_result = Set(Local(2), 9);
    var named_result = Set(LocalN("x"), 11);
    return [Var(0), var_result, Local(2), local_result,
            x, LocalN("x"), named_result];
}
"#,
    );

    assert_eq!(
        result,
        Value::Array(vec![
            Value::Int(7),
            Value::Int(7),
            Value::Int(9),
            Value::Int(9),
            Value::Int(11),
            Value::Int(11),
            Value::Int(11),
        ])
    );
}

#[test]
fn object_set_function_shadows_the_host_builtin() {
    let result = call_probe(
        "SETS",
        r#"#strict
protected func Set(destination, source)
{
    return [destination, source, 99];
}

protected func Probe()
{
    Var(0) = 3;
    return [Set(Var(0), 8), Var(0)];
}
"#,
    );

    assert_eq!(
        result,
        Value::Array(vec![
            Value::Array(vec![Value::Int(3), Value::Int(8), Value::Int(99)]),
            Value::Int(3),
        ])
    );
}

#[test]
fn is_ref_matches_cpp_any_parameter_dereference() {
    let result = call_probe(
        "ISRF",
        r#"#strict 3
protected func Probe()
{
    return [IsRef(Var(0)), IsRef(0), IsRef(GetX())];
}
"#,
    );

    assert_eq!(
        result,
        Value::Array(vec![
            Value::Bool(false),
            Value::Bool(false),
            Value::Bool(false),
        ])
    );
}
