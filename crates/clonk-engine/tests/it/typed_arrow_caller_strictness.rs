use clonk_engine::{Engine, ObjectId, SpawnConfig};
use clonk_script::Value;

const TYPED_CALLEE: &str = r#"
public func KeepZero(int value)
{
    return value;
}
"#;

const STRICT_CALLER: &str = r#"
#strict 3

public func DefinitionIdCall()
{
    return TARG->KeepZero(0);
}

public func NamespacedObjectCall(object target)
{
    return target->TARG::KeepZero(0);
}
"#;

const NONSTRICT_CALLER: &str = r#"
public func DefinitionIdCall()
{
    return TARG->KeepZero(0);
}

public func NamespacedObjectCall(object target)
{
    return target->TARG::KeepZero(0);
}
"#;

fn call(engine: &mut Engine, object_id: ObjectId, function: &str, args: Vec<Value>) -> Value {
    let index = crate::support::TestValueExt::test_value(engine.find_object_index(object_id));
    engine
        .call_object_function(index, function, args)
        .unwrap_or_else(|error| panic!("{function} succeeds: {error}"))
}

fn fixture() -> (Engine, ObjectId, ObjectId, ObjectId) {
    let mut engine = Engine::new();
    crate::support::TestValueExt::test_value(engine.register_script_definition(
        "TARG",
        "NONSTRICT typed callee",
        TYPED_CALLEE,
    ));
    crate::support::TestValueExt::test_value(engine.register_script_definition(
        "STRC",
        "strict-3 caller",
        STRICT_CALLER,
    ));
    crate::support::TestValueExt::test_value(engine.register_script_definition(
        "NSTR",
        "NONSTRICT caller",
        NONSTRICT_CALLER,
    ));

    let strict =
        crate::support::TestValueExt::test_value(engine.spawn_object(SpawnConfig::new("STRC")));
    let nonstrict =
        crate::support::TestValueExt::test_value(engine.spawn_object(SpawnConfig::new("NSTR")));
    let target =
        crate::support::TestValueExt::test_value(engine.spawn_object(SpawnConfig::new("TARG")));
    (engine, strict, nonstrict, target)
}

fn assert_source_strictness_is_preserved(function: &str) {
    let (mut engine, strict, nonstrict, target) = fixture();
    let args = if function == "NamespacedObjectCall" {
        vec![Value::Object(target.as_u64())]
    } else {
        Vec::new()
    };
    assert_eq!(
        call(&mut engine, strict, function, args.clone()),
        Value::Int(0),
        "strict-3 source keeps a zero argument typed as int across {function}"
    );
    assert_eq!(
        call(&mut engine, nonstrict, function, args),
        Value::Nil,
        "NONSTRICT source eagerly normalizes the same zero across {function}"
    );
}

#[test]
fn definition_id_arrow_call_preserves_the_script_callers_source_strictness() {
    assert_source_strictness_is_preserved("DefinitionIdCall");
}

#[test]
fn namespaced_object_arrow_call_preserves_the_script_callers_source_strictness() {
    assert_source_strictness_is_preserved("NamespacedObjectCall");
}
