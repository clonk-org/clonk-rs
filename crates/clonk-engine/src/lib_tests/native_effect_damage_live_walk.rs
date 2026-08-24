use super::*;
use crate::lib_test_support::{register_fixture, spawn_fixture, EngineTestExt};

#[test]
fn negative_priority_effect_constructor_links_at_the_list_head() {
    // C4Effect's constructor compares Abs(existing priority) with the RAW
    // requested priority (C4Effect.cpp:80-94). Any non-positive request takes
    // the head-insertion branch, even when its magnitude exceeds the head.
    let mut engine = Engine::with_seed(23);
    register_fixture!(
        engine,
        "NNEG",
        "Negative effect priority insertion",
        r#"#strict 3
public func Arm()
{
    AddEffect("Positive", this(), 100, 0, this());
    AddEffect("Negative", this(), -200, 0, this());
    return true;
}
"#,
        set_c4_callback_convention(true)
    );
    let object = spawn_fixture!(engine, "NNEG");
    let index = engine.test_object_index(object);

    assert_eq!(
        engine.call_test_object_function(index, "Arm", Vec::new()),
        Value::Bool(true)
    );
    assert_eq!(
        engine.objects[index]
            .state
            .effects
            .iter()
            .map(|effect| (effect.name.as_str(), effect.priority))
            .collect::<Vec<_>>(),
        vec![("Negative", -200), ("Positive", 100)]
    );
}

#[test]
fn negative_priority_start_temp_wraps_its_live_positive_suffix() {
    // A negative request links at the head, so the constructor's pNext is the
    // existing positive effect. With an Fx*Start, C++ temp-stops that live
    // suffix tail-first, starts the new effect, then re-starts the suffix
    // (C4Effect.cpp:80-94,118-139,477-510).
    let mut engine = Engine::with_seed(27);
    register_fixture!(
        engine,
        "NTMP",
        "Negative effect temp suffix",
        r#"#strict 3
local trace;

public func Arm()
{
    AddEffect("Positive", this(), 100, 0, this());
    trace = 0;
    AddEffect("Negative", this(), -200, 0, this());
    return true;
}

public func ReadTrace() { return trace; }

func FxPositiveStart(object target, int number, int reason)
{
    if (reason == 1) trace = trace * 10 + 3;
    return 0;
}

func FxPositiveStop(object target, int number, int reason, bool temporary)
{
    if (reason == 1 && temporary) trace = trace * 10 + 1;
    return 0;
}

func FxNegativeStart(object target, int number, int reason)
{
    trace = trace * 10 + 2;
    return 0;
}
"#,
        set_c4_callback_convention(true)
    );
    let object = spawn_fixture!(engine, "NTMP");
    let index = engine.test_object_index(object);

    assert_eq!(
        engine.call_test_object_function(index, "Arm", Vec::new()),
        Value::Bool(true)
    );
    assert_eq!(
        engine.call_test_object_function(index, "ReadTrace", Vec::new()),
        Value::Int(123)
    );
}

#[test]
fn priority_one_node_stops_negative_constructor_temp_recursion() {
    // TempRemoveUpperEffects returns immediately when its first recursive
    // pNext frame has priority 1. With a negative constructor linked before
    // [priority-1, priority-100], that boundary prevents the later positive
    // node from receiving temp callbacks (C4Effect.cpp:477-492).
    let mut engine = Engine::with_seed(28);
    register_fixture!(
        engine,
        "NPR1",
        "Priority-one temp boundary",
        r#"#strict 3
local trace;

public func Arm()
{
    AddEffect("Boundary", this(), 1, 0, this());
    AddEffect("Positive", this(), 100, 0, this());
    trace = 0;
    AddEffect("Negative", this(), -200, 0, this());
    return true;
}

public func ReadTrace() { return trace; }

func FxPositiveStart(object target, int number, int reason)
{
    if (reason == 1) trace = trace * 10 + 3;
    return 0;
}

func FxPositiveStop(object target, int number, int reason, bool temporary)
{
    if (reason == 1 && temporary) trace = trace * 10 + 1;
    return 0;
}

func FxNegativeStart(object target, int number, int reason)
{
    trace = trace * 10 + 2;
    return 0;
}
"#,
        set_c4_callback_convention(true)
    );
    let object = spawn_fixture!(engine, "NPR1");
    let index = engine.test_object_index(object);

    assert_eq!(
        engine.call_test_object_function(index, "Arm", Vec::new()),
        Value::Bool(true)
    );
    assert_eq!(
        engine.call_test_object_function(index, "ReadTrace", Vec::new()),
        Value::Int(2)
    );
}

#[test]
fn native_damage_error_keeps_pre_error_effect_vars_and_rng() {
    // Fx*Damage uses fail-safe Exec: a runtime error produces nil/getInt()==0
    // but does not roll back mutations or synchronized RNG draws made before
    // the unwind (C4Effect.cpp:427-437; C4AulExec.cpp:1318-1342).
    let mut engine = Engine::with_seed(37);
    register_fixture!(
        engine,
        "NERR",
        "Native damage error recovery",
        r#"#strict 2
public func Arm()
{
    AddEffect("Fault", this(), 100, 0, this());
    return true;
}

func FxFaultDamage(target, number, change, cause, caused_by)
{
    EffectVar(0, target, number) = 88;
    EffectVar(1, target, number) = Random(113);
    NoSuchFunctionAnywhere();
    return 99;
}
"#,
        set_c4_callback_convention(true)
    );
    let object = spawn_fixture!(engine, "NERR");
    let index = engine.test_object_index(object);
    assert_eq!(
        engine.call_test_object_function(index, "Arm", Vec::new()),
        Value::Bool(true)
    );
    let mut expected_rng = engine.rng.clone();
    let expected_draw = expected_rng.random(113);

    crate::TestValueExt::test_value(engine.change_object_damage(index, 10, 0, OWNER_NONE));

    assert_eq!(engine.objects[index].state.damage, 0);
    assert_eq!(
        (
            engine.objects[index].state.effects[0].var(0),
            engine.objects[index].state.effects[0].var(1),
        ),
        (EffectVarValue::Int(88), EffectVarValue::Int(expected_draw),)
    );
    assert_eq!(engine.rng, expected_rng);
}

#[test]
fn native_damage_walk_observes_a_removed_successor_and_its_replacement() {
    // C4Effect::DoDamage advances through the live pNext chain only after each
    // Fx*Damage callback returns (C4Effect.cpp:427-437). First marks Victim
    // dead and appends Replacement behind that still-linked dead node; the
    // same walk must skip Victim and then reach Replacement. A frozen effect
    // snapshot would instead call the removed callback and miss the new one.
    let mut engine = Engine::with_seed(29);
    register_fixture!(
        engine,
        "NDMG",
        "Native damage live walk",
        r#"#strict 3
local trace;

public func Arm()
{
    trace = 0;
    AddEffect("First", this(), 100, 0, this());
    AddEffect("Victim", this(), 200, 0, this());
    return true;
}

public func ReadTrace() { return trace; }

func FxFirstDamage(object target, int number, int change, int cause, int caused_by)
{
    trace = trace * 10 + 1;
    RemoveEffect("Victim", target, 0, true);
    AddEffect("Replacement", target, 150, 0, target);
    return change + 1;
}

func FxVictimDamage(object target, int number, int change, int cause, int caused_by)
{
    trace = trace * 10 + 9;
    return 99;
}

func FxReplacementDamage(object target, int number, int change, int cause, int caused_by)
{
    trace = trace * 10 + 2;
    return change + 2;
}
"#,
        set_c4_callback_convention(true)
    );
    let object = spawn_fixture!(engine, "NDMG");
    let index = engine.test_object_index(object);
    assert_eq!(
        engine.call_test_object_function(index, "Arm", Vec::new()),
        Value::Bool(true)
    );

    crate::TestValueExt::test_value(engine.change_object_damage(index, 10, 0, OWNER_NONE));

    assert_eq!(engine.objects[index].state.damage, 13);
    assert_eq!(
        engine.call_test_object_function(index, "ReadTrace", Vec::new()),
        Value::Int(12)
    );
    assert_eq!(
        engine.objects[index]
            .state
            .effects
            .iter()
            .map(|effect| (effect.name.as_str(), effect.number, effect.priority))
            .collect::<Vec<_>>(),
        vec![("First", 1, 100), ("Victim", 2, 0), ("Replacement", 3, 150),]
    );
}
