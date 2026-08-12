use crate::support::EngineTestExt;
use clonk_engine::{Definition, Engine, ObjectId, SpawnConfig};
use clonk_script::Value;

const CALLER_OWNER: i32 = 7;
const RECEIVER_OWNER: i32 = 9;

const CALLER_BODY: &str = r#"
local observed;

public func CaptureOwner(owner)
{
    observed = owner;
    return 1;
}

public func ObjectDefault()
{
    observed = -99;
    return CreateObject(TARG);
}

public func ObjectArg(owner)
{
    observed = -99;
    return CreateObject(TARG, 0, 0, owner);
}

public func ConstructionDefault()
{
    observed = -99;
    return CreateConstruction(TARG);
}

public func ConstructionArg(owner)
{
    observed = -99;
    return CreateConstruction(TARG, 0, 0, owner, 100);
}

public func ArrowDefault(target)
{
    return target->CreateObject(TARG);
}

public func ArrowArg(target, owner)
{
    return target->CreateObject(TARG, 0, 0, owner);
}
"#;

const TARGET_SCRIPT: &str = r#"#strict 3
protected func Construction(object creator)
{
    if (creator) creator->~CaptureOwner(GetOwner());
}
"#;

fn caller_source(strict: bool) -> String {
    if strict {
        format!("#strict 3\n{CALLER_BODY}")
    } else {
        CALLER_BODY.to_string()
    }
}

fn fixture() -> (Engine, ObjectId, ObjectId, ObjectId) {
    let mut engine = Engine::new();

    let mut target = crate::support::TestValueExt::test_value(Definition::from_script(
        "TARG",
        "Created target",
        TARGET_SCRIPT,
    ));
    target.set_constructable(true);
    engine.register_test_definition(target);
    engine.register_test_script_definition("STRC", "Strict creator", &caller_source(true));
    engine.register_test_script_definition("NSTR", "Nonstrict creator", &caller_source(false));
    engine.register_test_definition(crate::support::TestValueExt::test_value(
        Definition::from_script(
            "RECV",
            "Arrow receiver",
            "#strict 3\npublic func CaptureOwner(owner) { return 1; }\n",
        ),
    ));

    let strict = engine.spawn_test_object(SpawnConfig::new("STRC").with_owner(CALLER_OWNER));
    let nonstrict = engine.spawn_test_object(SpawnConfig::new("NSTR").with_owner(CALLER_OWNER));
    let receiver = engine.spawn_test_object(SpawnConfig::new("RECV").with_owner(RECEIVER_OWNER));
    (engine, strict, nonstrict, receiver)
}

fn call(engine: &mut Engine, caller: ObjectId, function: &str, args: Vec<Value>) -> Value {
    let index = engine
        .find_object_index(caller)
        .unwrap_or_else(|| panic!("{function} caller remains active"));
    engine
        .call_object_function(index, function, args)
        .unwrap_or_else(|error| panic!("{function} succeeds: {error}"))
}

fn assert_observed_owner(engine: &Engine, caller: ObjectId, expected: i32) {
    assert_eq!(
        engine
            .test_object_snapshot(caller)
            .local_vars
            .get("observed"),
        Some(&Value::Int(expected))
    );
}

fn assert_created_owner(engine: &Engine, value: Value, expected: i32) {
    let Value::Object(raw_id) = value else {
        panic!("creation should return an object, got {value:?}");
    };
    assert_eq!(
        engine.test_object_snapshot(ObjectId::new(raw_id)).owner,
        expected
    );
}

#[test]
fn create_object_and_construction_owner_follow_immediate_caller_strictness() {
    let (mut engine, strict, nonstrict, receiver) = fixture();

    for (caller, expected_default, expected_nil, expected_three) in [
        (strict, 0, 0, 3),
        (nonstrict, CALLER_OWNER, CALLER_OWNER, CALLER_OWNER),
    ] {
        let created = call(&mut engine, caller, "ObjectDefault", Vec::new());
        assert_observed_owner(&engine, caller, expected_default);
        assert_created_owner(&engine, created, expected_default);

        let created = call(&mut engine, caller, "ObjectArg", vec![Value::Nil]);
        assert_observed_owner(&engine, caller, expected_nil);
        assert_created_owner(&engine, created, expected_nil);

        let created = call(&mut engine, caller, "ObjectArg", vec![Value::Int(3)]);
        assert_observed_owner(&engine, caller, expected_three);
        assert_created_owner(&engine, created, expected_three);

        // The omitted completion defaults to zero and removes the site after
        // Construction. Its synchronous callback still observes the owner.
        assert_eq!(
            call(&mut engine, caller, "ConstructionDefault", Vec::new()),
            Value::Nil
        );
        assert_observed_owner(&engine, caller, expected_default);

        let created = call(&mut engine, caller, "ConstructionArg", vec![Value::Nil]);
        assert_observed_owner(&engine, caller, expected_nil);
        assert_created_owner(&engine, created, expected_nil);

        let created = call(&mut engine, caller, "ConstructionArg", vec![Value::Int(3)]);
        assert_observed_owner(&engine, caller, expected_three);
        assert_created_owner(&engine, created, expected_three);
    }

    // AB_CALL native fallback keeps the OUTER script function as cthr->Caller
    // while making the receiver cthr->Obj. Strict preserves owner 0;
    // NONSTRICT replaces even an explicit 3 with the receiver's owner.
    let created = call(
        &mut engine,
        strict,
        "ArrowDefault",
        vec![Value::Object(receiver.as_u64())],
    );
    assert_created_owner(&engine, created, 0);
    let created = call(
        &mut engine,
        nonstrict,
        "ArrowArg",
        vec![Value::Object(receiver.as_u64()), Value::Int(3)],
    );
    assert_created_owner(&engine, created, RECEIVER_OWNER);
}
