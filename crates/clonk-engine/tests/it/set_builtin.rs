use clonk_engine::{Engine, SpawnConfig};
use clonk_script::Value;

fn call_probe(id: &str, script: &str) -> Value {
    let mut engine = Engine::new();
    crate::support::TestValueExt::test_value(engine.register_script_definition(
        id,
        "Set builtin probe",
        script,
    ));
    let object =
        crate::support::TestValueExt::test_value(engine.spawn_object(SpawnConfig::new(id)));
    let index = crate::support::TestValueExt::test_value(engine.find_object_index(object));
    crate::support::TestValueExt::test_value(engine.call_object_function(
        index,
        "Probe",
        Vec::new(),
    ))
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
fn inc_builtin_mutates_integer_refs_and_preserves_unconvertible_refs() {
    let result = call_probe(
        "INCB",
        r#"#strict 3
protected func Probe()
{
    var value = 5;
    var inc_result = Inc(value, 3);
    var text = "unchanged";
    var text_result = Inc(text, 3);
    return [value, inc_result, text, text_result];
}
"#,
    );

    assert_eq!(
        result,
        Value::Array(vec![
            Value::Int(8),
            Value::Int(8),
            Value::String("unchanged".to_string().into()),
            Value::Nil,
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

#[test]
fn equal_compares_raw_scalar_payloads_and_container_identity() {
    let result = call_probe(
        "EQUL",
        r#"#strict 3
protected func Probe()
{
    var array = [1, 2];
    var alias = array;
    var map = { x = 1 };
    var map_alias = map;
    return [Equal(1, 1), Equal(1, "1"), Equal(1, true),
            Equal(array, alias), Equal(array, [1, 2]),
            Equal(map, map_alias), Equal(map, { x = 1 })];
}
"#,
    );

    assert_eq!(
        result,
        Value::Array(vec![
            Value::Bool(true),
            Value::Bool(false),
            Value::Bool(true),
            Value::Bool(true),
            Value::Bool(false),
            Value::Bool(true),
            Value::Bool(false),
        ])
    );
}

#[test]
fn dec_builtin_mutates_integer_refs_and_preserves_unconvertible_refs() {
    let result = call_probe(
        "DECB",
        r#"#strict 3
protected func Probe()
{
    var value = 5;
    var dec_result = Dec(value, 2);
    var text = "unchanged";
    var text_result = Dec(text, 2);
    var nil_diff_result = Dec(value, nil);
    return [value, dec_result, text, text_result, nil_diff_result];
}
"#,
    );

    assert_eq!(
        result,
        Value::Array(vec![
            Value::Int(3),
            Value::Int(3),
            Value::String("unchanged".to_string().into()),
            Value::Nil,
            Value::Int(3),
        ])
    );
}

#[test]
fn dec_builtin_preserves_global_reference_dispatch_and_evaluates_lvalues_once() {
    let result = call_probe(
        "DECG",
        r#"#strict 3
protected func Probe()
{
    var values = [8, 10];
    var index = 0;
    var direct_result = Dec(values[index++], 2);
    var global_result = global->Dec(values[index++], 3);
    return [index, values, direct_result, global_result];
}
"#,
    );

    assert_eq!(
        result,
        Value::Array(vec![
            Value::Int(2),
            Value::Array(vec![Value::Int(6), Value::Int(7)]),
            Value::Int(6),
            Value::Int(7),
        ])
    );
}
