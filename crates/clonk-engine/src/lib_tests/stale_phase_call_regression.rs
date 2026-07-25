use super::*;
use std::sync::{Arc, Mutex};

fn install_physical_actions(definition: &mut Definition, actions: Vec<(&str, ActionSpec)>) {
    definition.configure_actions(
        None,
        actions
            .iter()
            .map(|(name, spec)| ((*name).to_string(), spec.clone()))
            .collect(),
    );
    definition.configure_physical_actions(
        actions
            .into_iter()
            .map(|(name, spec)| (name.to_string(), spec))
            .collect(),
    );
}

#[test]
fn phase_call_keeps_stale_function_owner_but_uses_changed_def_act_map() -> Result<(), EngineError> {
    let calls = Arc::new(Mutex::new(Vec::new()));
    let mut old_hooks = DebuggerHooks::new();
    {
        let calls = Arc::clone(&calls);
        old_hooks.set_on_call(move |name, _| {
            if name == "OnPhase" {
                calls.lock().unwrap().push(format!("old:{name}"));
            }
        });
    }
    let mut new_hooks = DebuggerHooks::new();
    {
        let calls = Arc::clone(&calls);
        new_hooks.set_on_call(move |name, _| {
            if matches!(name, "OnPhase" | "NewZeroStart" | "NewOneStart") {
                calls.lock().unwrap().push(format!("new:{name}"));
            }
        });
    }

    let mut old = Definition::from_script(
        "POLD",
        "Old phase owner",
        r#"#strict
local marker;
protected func OnPhase()
{
    marker = 1;
    ChangeDef(PNW1);
    return 1;
}
"#,
    )?;
    old.set_c4_callback_convention(true);
    old.set_debugger_hooks(old_hooks);
    let source = ActionSpec::default()
        // ChangeDef's mandatory SetAction(ActIdle) resets the live
        // phase to zero. A zero stale Length proves ExecAction still
        // performs the old comparison and numeric NextAction afterward.
        .with_length(0)
        .with_delay(1)
        .with_phase_call("OnPhase")
        .with_next_index(1);
    install_physical_actions(
        &mut old,
        vec![("Source", source), ("OldTarget", ActionSpec::default())],
    );

    let mut new = Definition::from_script(
        "PNW1",
        "New phase target",
        r#"#strict
local marker;
protected func OnPhase() { marker = 2; return 1; }
protected func NewZeroStart() { marker = marker * 10 + 4; return 1; }
protected func NewOneStart() { marker = marker * 10 + 3; return 1; }
"#,
    )?;
    new.set_c4_callback_convention(true);
    new.set_debugger_hooks(new_hooks);
    install_physical_actions(
        &mut new,
        vec![
            (
                "NewZero",
                ActionSpec::default().with_start_call("NewZeroStart"),
            ),
            (
                "NewOne",
                ActionSpec::default().with_start_call("NewOneStart"),
            ),
        ],
    );

    let mut engine = Engine::new();
    engine.register_definition(old)?;
    engine.register_definition(new)?;
    let mut action = ActionState::new("Source");
    action.act_map_index = Some(0);
    let object = engine.spawn_object(
        SpawnConfig::new("POLD")
            .with_action(action)
            .with_loaded(true),
    )?;

    engine.tick_without_snapshot()?;

    let index = engine.find_object_index(object).expect("object remains");
    assert_eq!(engine.objects[index].definition_id, "PNW1");
    assert_eq!(
        (
            engine.objects[index].state.action.name.as_str(),
            engine.objects[index].state.action.act_map_index,
        ),
        ("NewOne", Some(1)),
        "stale numeric NextAction resolves in the changed definition",
    );
    assert_eq!(
        engine.objects[index].state.local_vars.get("marker"),
        Some(&Value::Int(13)),
    );
    assert_eq!(
        calls.lock().unwrap().as_slice(),
        ["old:OnPhase", "new:NewOneStart"],
            "the stale PhaseCall function runs on its old script, then current target callbacks run on the new script",
    );
    Ok(())
}

#[test]
fn removed_phase_receiver_still_runs_phase_end_start_before_stopping() -> Result<(), EngineError> {
    let calls = Arc::new(Mutex::new(Vec::new()));
    let mut hooks = DebuggerHooks::new();
    {
        let calls = Arc::clone(&calls);
        hooks.set_on_call(move |name, _| {
            if matches!(name, "OnPhase" | "OnStart" | "OnEnd") {
                calls.lock().unwrap().push(name.to_string());
            }
        });
    }
    let mut definition = Definition::from_script(
        "PDEL",
        "Deleted phase receiver",
        r#"#strict
protected func OnPhase() { RemoveObject(); return 1; }
protected func OnStart() { return 1; }
protected func OnEnd() { return 1; }
"#,
    )?;
    definition.set_c4_callback_convention(true);
    definition.set_debugger_hooks(hooks);
    let source = ActionSpec::default()
        .with_length(0)
        .with_delay(1)
        .with_phase_call("OnPhase")
        .with_end_call("OnEnd")
        .with_next_index(1);
    install_physical_actions(
        &mut definition,
        vec![
            ("Source", source),
            ("Target", ActionSpec::default().with_start_call("OnStart")),
        ],
    );

    let mut engine = Engine::new();
    engine.register_definition(definition)?;
    let mut action = ActionState::new("Source");
    action.act_map_index = Some(0);
    engine.spawn_object(
        SpawnConfig::new("PDEL")
            .with_action(action)
            .with_loaded(true),
    )?;

    engine.tick_without_snapshot()?;

    assert_eq!(
        calls.lock().unwrap().as_slice(),
        ["OnPhase", "OnStart"],
        "SetAction starts the target even with Status=0, then suppresses old EndCall",
    );
    Ok(())
}

#[test]
fn script_set_action_coerces_incomplete_objects_to_act_idle_and_skips_start_call(
) -> Result<(), EngineError> {
    let mut definition = Definition::from_script(
        "PINC",
        "Partial action gate",
        r#"#strict
local walk_started, old_aborted, aborted_phase, abort_saw_action;
public func Probe()
{
    walk_started = 0;
    old_aborted = 0;
    aborted_phase = -1;
    abort_saw_action = "";
    return SetAction("Walk");
}
protected func WalkStarted() { walk_started++; return 1; }
protected func OldAborted(phase)
{
    old_aborted++;
    aborted_phase = phase;
    abort_saw_action = GetAction();
    return 1;
}
"#,
    )?;
    definition.set_c4_callback_convention(true);
    install_physical_actions(
        &mut definition,
        vec![
            ("Old", ActionSpec::default().with_abort_call("OldAborted")),
            ("Walk", ActionSpec::default().with_start_call("WalkStarted")),
        ],
    );

    let mut engine = Engine::new();
    engine.register_definition(definition)?;
    let mut action = ActionState::new("Old");
    action.act_map_index = Some(0);
    action.phase = 7;
    let object = engine.spawn_object(
        SpawnConfig::new("PINC")
            .with_action(action)
            .with_loaded(true),
    )?;
    let index = engine.find_object_index(object).expect("object exists");
    // Establish a state that can only arise after an already-active
    // object loses construction. The call below is the seam under test.
    engine.objects[index].state.construction = FULL_CON / 2;

    assert_eq!(
        engine.call_object_function(index, "Probe", Vec::new())?,
        Value::Bool(true),
        "the requested slot is valid, so SetAction succeeds despite coercion",
    );

    let object = &engine.objects[index].state;
    assert_eq!(
        (object.action.name.as_str(), object.action.act_map_index),
        ("Idle", None),
    );
    assert_eq!(object.local_vars.get("walk_started"), Some(&Value::Nil));
    assert_eq!(object.local_vars.get("old_aborted"), Some(&Value::Int(1)));
    assert_eq!(object.local_vars.get("aborted_phase"), Some(&Value::Int(7)));
    assert_eq!(
        object.local_vars.get("abort_saw_action"),
        Some(&Value::String("Idle".into())),
        "AbortCall runs after Action.Act has become ActIdle",
    );
    Ok(())
}

#[test]
fn natural_phase_end_refreshes_ocf_before_start_and_end_callbacks() -> Result<(), EngineError> {
    let mut definition = Definition::from_script(
        "POCF",
        "Natural action OCF refresh",
        r#"#strict
local callback_order, start_saw_fight_ready, end_saw_fight_ready;
protected func TargetStarted()
{
    callback_order = callback_order * 10 + 1;
    if (GetOCF() & OCF_FightReady) start_saw_fight_ready = 1;
    return 1;
}
protected func SourceEnded()
{
    callback_order = callback_order * 10 + 2;
    if (GetOCF() & OCF_FightReady) end_saw_fight_ready = 1;
    return 1;
}
"#,
    )?;
    definition.set_c4_callback_convention(true);
    definition.set_category(CATEGORY_OBJECT | CATEGORY_LIVING);
    install_physical_actions(
        &mut definition,
        vec![
            (
                "Source",
                ActionSpec::default()
                    .with_length(1)
                    .with_delay(1)
                    .with_end_call("SourceEnded")
                    .with_disabled(true)
                    .with_next_index(1),
            ),
            (
                "Target",
                ActionSpec::default().with_start_call("TargetStarted"),
            ),
        ],
    );

    let mut engine = Engine::new();
    engine.register_definition(definition)?;
    let mut action = ActionState::new("Source");
    action.act_map_index = Some(0);
    let object = engine.spawn_object(
        SpawnConfig::new("POCF")
            .with_action(action)
            .with_alive(true)
            .with_loaded(true),
    )?;

    engine.tick_without_snapshot()?;

    let index = engine.find_object_index(object).expect("object remains");
    let state = &engine.objects[index].state;
    assert_eq!(
        (state.action.name.as_str(), state.action.act_map_index),
        ("Target", Some(1)),
    );
    assert_eq!(
        state.local_vars.get("callback_order"),
        Some(&Value::Int(12))
    );
    assert_eq!(
        state.local_vars.get("start_saw_fight_ready"),
        Some(&Value::Int(1)),
    );
    assert_eq!(
        state.local_vars.get("end_saw_fight_ready"),
        Some(&Value::Int(1)),
        "SetOCF runs after action selection and before both callbacks",
    );
    Ok(())
}
