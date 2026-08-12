use super::*;

fn dispatch_scheduled_global_timer(engine: &Engine, effect: &EffectState) -> Option<Value> {
    crate::TestValueExt::test_value(dispatch_global_effect_callback(
        effect,
        "Timer",
        "FxTimer",
        vec![Value::Int(1)],
        engine.rng.clone(),
        &engine.global_effects,
        engine.physics,
        engine.environment,
        engine.frame,
        engine.host_world_context(),
        engine.game_over_triggered,
        engine.audio_registry.clone(),
    ))
    .3
}

fn scheduled_global_effect(
    name: &str,
    command_target: Option<i32>,
    command_id: Option<&str>,
) -> EffectState {
    let mut effect = EffectState::new(name)
        .with_interval(1)
        .with_command_target(command_target)
        .with_command_id(command_id);
    effect.number = 47;
    effect
}

#[test]
fn scheduled_global_command_target_local_callback_keeps_pre_strict3_effect_number() {
    // `Game.pGlobalEffects` still dispatches through the selected command
    // target's local Fx function (src/C4Effect.cpp:31-57,319-363). Its
    // callback argument set owns the C4Effect number, and pre-STRICT3
    // conversion warns rather than replacing that unconvertible value
    // (src/C4AulExec.cpp:1364-1397,1610-1627,1638-1656).
    let mut engine = Engine::new();
    crate::TestValueExt::test_value(engine.register_script_definition(
        "SGLC",
        "Scheduled-global local callback probe",
        r#"#strict 2
    func FxGlobalLocalTimer(target, object declared_but_unused, int time)
    {
      return(declared_but_unused);
    }
    "#,
    ));
    let target = crate::TestValueExt::test_value(engine.spawn_object(SpawnConfig::new("SGLC")));
    let mut effect = EffectState::new("GlobalLocal")
        .with_interval(1)
        .with_command_target(Some(crate::TestValueExt::test_value(i32::try_from(
            target.as_u64(),
        ))));
    effect.number = 47;

    let (_, _, _, result) = crate::TestValueExt::test_value(dispatch_global_effect_callback(
        &effect,
        "Timer",
        "FxTimer",
        vec![Value::Int(1)],
        engine.rng.clone(),
        &engine.global_effects,
        engine.physics,
        engine.environment,
        engine.frame,
        engine.host_world_context(),
        engine.game_over_triggered,
        engine.audio_registry.clone(),
    ));

    assert!(
        result == Some(Value::Int(47)),
        "the callback receives the original C4Effect number: {result:?}"
    );
}

#[test]
fn scheduled_global_dispatcher_marks_each_callback_carrier_once() {
    // Scheduled global effects have the same callback-source selection as
    // any C4Effect (src/C4Effect.cpp:31-57), while Execute passes their
    // owned number through the timer callback (src/C4Effect.cpp:319-363,
    // especially :345). Every selected pre-STRICT3 entry therefore receives
    // warning-only compatibility at its own call boundary
    // (src/C4AulExec.cpp:1364-1397,1610-1627,1638-1656).
    let mut engine = Engine::new();
    crate::TestValueExt::test_value(engine.register_script_definition(
        "TGCB",
        "Scheduled-global command target carrier",
        r#"#strict 2
    func FxTargetLocalTimer(target, object declared_but_unused, int time)
    {
      return(declared_but_unused);
    }

    global func FxTargetGlobalTimer(target, object declared_but_unused, int time)
    {
      return(declared_but_unused);
    }
    "#,
    ));
    crate::TestValueExt::test_value(engine.register_script_definition(
        "IGLC",
        "Scheduled-global command-ID local carrier",
        r#"#strict 2
    func FxIdLocalTimer(target, object declared_but_unused, int time)
    {
      return(declared_but_unused);
    }
    "#,
    ));
    crate::TestValueExt::test_value(engine.register_script_definition(
        "IGLB",
        "Scheduled-global command-ID global carrier",
        r#"#strict 2
    global func FxIdGlobalTimer(target, object declared_but_unused, int time)
    {
      return(declared_but_unused);
    }
    "#,
    ));
    assert_eq!(
        engine.install_additional_global_scripts(&[(
            "Issue62ScheduledGlobal.c".to_string(),
            r#"#strict 2
global func FxEngineGlobalTimer(target, object declared_but_unused, int time)
{
  return(declared_but_unused);
}
"#
            .to_string(),
        )]),
        1
    );
    let target = crate::TestValueExt::test_value(engine.spawn_object(SpawnConfig::new("TGCB")));
    let command_target = Some(crate::TestValueExt::test_value(i32::try_from(
        target.as_u64(),
    )));

    for effect in [
        scheduled_global_effect("TargetLocal", command_target, None),
        scheduled_global_effect("TargetGlobal", command_target, None),
        scheduled_global_effect("IdLocal", None, Some("IGLC")),
        scheduled_global_effect("IdGlobal", None, Some("IGLB")),
        scheduled_global_effect("EngineGlobal", None, None),
    ] {
        assert_eq!(
            dispatch_scheduled_global_timer(&engine, &effect),
            Some(Value::Int(47)),
            "{} receives the original callback number",
            effect.name
        );
    }
}

#[test]
fn scheduled_global_tick_keeps_strict3_conversion_fail_safe() {
    // At strict 3, C4AulScriptFunc::Exec does not downgrade the mismatch;
    // C4Effect::Execute's fPassErrors=false wrapper turns it into a nil timer
    // result and continues the tick (src/C4Effect.cpp:319-363; src/C4AulExec.cpp:
    // 1610-1627,1638-1656).
    let mut engine = Engine::new();
    assert_eq!(
        engine.install_additional_global_scripts(&[(
            "Issue62StrictScheduledGlobal.c".to_string(),
            r#"#strict 3
static strict3_callback_runs;

global func FxStrictScheduledTimer(target, object declared_but_unused, int time)
{
  strict3_callback_runs = 1;
  return(0);
}

global func ReadStrictScheduledRuns()
{
  return(strict3_callback_runs);
}
"#
            .to_string(),
        )]),
        1
    );
    let mut effect = scheduled_global_effect("StrictScheduled", None, None);
    effect.number = 1;
    engine.global_effects.push(effect);

    crate::TestValueExt::test_value(engine.tick_without_snapshot());
    assert_eq!(
        engine
            .call_engine_global_function("ReadStrictScheduledRuns", &[])
            .expect("strict-3 callback counter reads"),
        Value::Nil,
        "the rejected callback body does not run or alias a value slot"
    );
}
