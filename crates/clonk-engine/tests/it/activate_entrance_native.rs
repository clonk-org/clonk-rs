use crate::support::EngineTestExt;
use std::fs;

use clonk_engine::scenario::LegacyDefinitionResolver;
use clonk_engine::{
    ocf, Definition, DefinitionRect, Engine, ObjectId, ObjectUpdate, PlayerConfig, Scenario,
    ScenarioError, SpawnConfig, Vector2, CATEGORY_LIVING, CATEGORY_OBJECT,
};
use clonk_resources::Group;
use clonk_script::Value;
use tempfile::tempdir;

const BASE_PLAYER: i32 = 1;
const VISITOR_CONTROLLER: i32 = 2;
const BASE_OWNER: i32 = 3;

struct LocalDefinitionResolver;

impl LegacyDefinitionResolver for LocalDefinitionResolver {
    fn resolve_definition_groups(
        &self,
        scenario: &Group,
        identifier: &str,
    ) -> Result<Vec<Group>, ScenarioError> {
        scenario
            .open_child(identifier.replace('\\', "/"))
            .map(|group| vec![group])
            .map_err(ScenarioError::Resources)
    }
}

fn local_int(engine: &Engine, object: ObjectId, name: &str) -> i32 {
    match engine
        .object_snapshot(object)
        .and_then(|snapshot| snapshot.local_vars.get(name).cloned())
    {
        Some(Value::Int(value)) => value,
        _ => 0,
    }
}

fn install_entrance_fixture(
    engine: &mut Engine,
    has_entrance_ocf: bool,
    hostile: bool,
    reject_hostile_entrance: Option<bool>,
) -> ObjectId {
    engine.register_test_player(PlayerConfig::new(BASE_PLAYER, "Base Player"));
    engine.register_test_player(PlayerConfig::new(VISITOR_CONTROLLER, "Visitor"));
    engine.register_test_player(PlayerConfig::new(BASE_OWNER, "Keeper"));
    crate::support::TestValueExt::test_value(engine.set_hostility(
        VISITOR_CONTROLLER,
        BASE_PLAYER,
        hostile,
    ));
    if let Some(enabled) = reject_hostile_entrance {
        engine.set_base_reject_entrance_enabled(enabled);
    }

    let mut visitor = crate::support::TestValueExt::test_value(Definition::from_script(
        "VSTR",
        "Visitor",
        r#"#strict
    public func StartEnter(pTarget) { return(SetCommand(this(), "Enter", pTarget)); }
    public func StartExit()
    {
      var queued = SetCommand(this(), "Exit");
      ExecuteCommand(); // InitEvaluation
      return(queued);
    }
    local observed_after_execute;
    public func ExecuteExitAndRead(pTarget)
    {
      SetCommand(this(), "Exit");
      ExecuteCommand(); // InitEvaluation
      ExecuteCommand();
      observed_after_execute = pTarget->ReadActivateCount();
      return(observed_after_execute);
    }
    public func ExecuteExitAfterRemovingEntrance(pTarget)
    {
      ChangeDef(NENT, pTarget);
      SetCommand(this(), "Exit");
      ExecuteCommand(); // InitEvaluation
      ExecuteCommand();
      return(GetCommand());
    }
    "#,
    ));
    visitor.set_c4_callback_convention(true);
    visitor.set_category(CATEGORY_OBJECT | CATEGORY_LIVING);
    visitor.set_crew_member(true);

    let mut base = crate::support::TestValueExt::test_value(Definition::from_script(
        "BASE",
        "Base",
        r#"#strict
    local activate_count, activate_caller, replace_exit_once, reenter_exit_once;
    public func ReadActivateCount() { return(activate_count); }
    public func ReplaceExitOnce()
    {
      replace_exit_once = 1;
      return(1);
    }
    public func ReenterExitOnce()
    {
      reenter_exit_once = 1;
      return(1);
    }
    protected func ActivateEntrance(pCaller)
    {
      activate_count += 1;
      activate_caller = pCaller;
      if (reenter_exit_once && activate_count == 1)
      {
    ExecuteCommand(pCaller);
    return(0);
      }
      if (replace_exit_once && activate_count == 1)
      {
    SetCommand(pCaller, "Exit");
    return(0);
      }
      SetEntrance(1);
      return(1);
    }
    "#,
    ));
    base.set_c4_callback_convention(true);
    base.set_can_be_base(true);
    base.set_shape_rect(Some(DefinitionRect::new(-20, -20, 40, 40)));
    if has_entrance_ocf {
        base.set_entrance_rect(Some(DefinitionRect::new(-20, -20, 40, 40)));
    }

    let mut no_entrance = crate::support::TestValueExt::test_value(Definition::from_script(
        "NENT",
        "No entrance",
        r#"#strict
    protected func ActivateEntrance(pCaller)
    {
      SetEntrance(1);
      return(1);
    }
    "#,
    ));
    no_entrance.set_c4_callback_convention(true);
    no_entrance.set_can_be_base(true);
    no_entrance.set_shape_rect(Some(DefinitionRect::new(-20, -20, 40, 40)));

    engine.register_test_definition(visitor);
    engine.register_test_definition(base);
    engine.register_test_definition(no_entrance);

    spawn_closed_base(engine)
}

fn spawn_closed_base(engine: &mut Engine) -> ObjectId {
    let base = engine.spawn_test_object(
        SpawnConfig::new("BASE")
            .with_owner(BASE_OWNER)
            .with_position(Vector2::new(100, 120)),
    );
    let mut closed_base = ObjectUpdate::new().with_base(BASE_PLAYER);
    closed_base.entrance_status = Some(false);
    crate::support::TestValueExt::test_value(engine.apply_object_update(base, closed_base));

    base
}

fn entrance_fixture(
    has_entrance_ocf: bool,
    hostile: bool,
    reject_hostile_entrance: bool,
) -> (Engine, ObjectId) {
    let mut engine = Engine::new();
    let base = install_entrance_fixture(
        &mut engine,
        has_entrance_ocf,
        hostile,
        Some(reject_hostile_entrance),
    );
    (engine, base)
}

fn spawn_visitor(engine: &mut Engine, container: Option<ObjectId>) -> ObjectId {
    let visitor = SpawnConfig::new("VSTR")
        .with_owner(BASE_PLAYER)
        .with_controller(VISITOR_CONTROLLER)
        .with_alive(true)
        .with_crew_member(true)
        .with_position(Vector2::new(100, 100));
    let visitor = engine.spawn_test_object(visitor);
    if let Some(container) = container {
        crate::support::TestValueExt::test_value(
            engine.apply_object_update(visitor, ObjectUpdate::new().with_container(container)),
        );
    }
    visitor
}

fn call(engine: &mut Engine, object: ObjectId, function: &str, args: Vec<Value>) -> Value {
    let index = engine.test_object_index(object);
    engine
        .call_object_function(index, function, args)
        .unwrap_or_else(|error| panic!("{function} executes: {error}"))
}

fn assert_hostile_entrance_message(engine: &Engine, base: ObjectId) {
    let messages = engine.snapshot().hud.messages;
    assert_eq!(messages.len(), 1, "hostile rejection emits one message");
    assert_eq!(messages[0].target, Some(base));
    assert_eq!(messages[0].player, None);
    assert_eq!(messages[0].lines, ["Keeper hostile.", "No entrance!"]);
}

#[test]
fn hostile_enter_rejects_before_activate_entrance_and_reports_base_owner() {
    let (mut engine, base) = entrance_fixture(true, true, true);
    let base_now = engine.test_object_snapshot(base);
    assert_eq!(base_now.base, BASE_PLAYER);
    assert_eq!(base_now.owner, BASE_OWNER);
    assert_ne!(base_now.ocf & ocf::ENTRANCE, 0, "base has a live entrance");

    let visitor = spawn_visitor(&mut engine, None);
    let visitor_now = engine.test_object_snapshot(visitor);
    assert_eq!(visitor_now.owner, BASE_PLAYER);
    assert_eq!(visitor_now.controller, VISITOR_CONTROLLER);
    assert_eq!(
        call(
            &mut engine,
            visitor,
            "StartEnter",
            vec![Value::Object(base.as_u64())],
        ),
        Value::Bool(true)
    );
    crate::support::TestValueExt::test_value(engine.tick_without_snapshot());

    assert_eq!(local_int(&engine, base, "activate_count"), 0);
    assert_eq!(engine.test_object_snapshot(visitor).container, None);
    assert_eq!(
        engine
            .test_object_snapshot(visitor)
            .command_stack
            .command_names(),
        ["Enter"],
        "a rejected closed Enter remains pending like C++"
    );
    assert_hostile_entrance_message(&engine, base);
}

#[test]
fn hostile_exit_rejects_before_activate_entrance_and_fails() {
    let (mut engine, base) = entrance_fixture(true, true, true);
    let base_now = engine.test_object_snapshot(base);
    assert_eq!(base_now.base, BASE_PLAYER);
    assert_ne!(base_now.ocf & ocf::ENTRANCE, 0, "base has a live entrance");

    let visitor = spawn_visitor(&mut engine, Some(base));
    assert_eq!(
        call(&mut engine, visitor, "StartExit", Vec::new()),
        Value::Bool(true)
    );
    crate::support::TestValueExt::test_value(engine.tick_without_snapshot());

    assert_eq!(local_int(&engine, base, "activate_count"), 0);
    assert_eq!(
        engine.test_object_snapshot(visitor).container,
        Some(base),
        "hostile Exit does not eject the visitor"
    );
    assert_hostile_entrance_message(&engine, base);
    assert!(
        engine
            .test_object_snapshot(visitor)
            .command_stack
            .command_names()
            .is_empty(),
        "ActivateEntrance=false synchronously fails and removes Exit"
    );
}

#[test]
fn script_execute_command_observes_native_entrance_activation_before_returning() {
    // FnExecuteCommand synchronously runs C4Object::ExecuteCommand. The
    // statement after it must therefore observe C4Object::ActivateEntrance's
    // script callback, not a command event deferred until the outer call
    // unwinds (C4Script.cpp:835-838; C4Command.cpp:644-650).
    let (mut engine, base) = entrance_fixture(true, false, true);
    let visitor = spawn_visitor(&mut engine, Some(base));

    assert_eq!(
        call(
            &mut engine,
            visitor,
            "ExecuteExitAndRead",
            vec![Value::Object(base.as_u64())],
        ),
        Value::Int(1),
        "the next script statement sees ActivateEntrance immediately"
    );
    assert_eq!(local_int(&engine, base, "activate_count"), 1);
    assert_eq!(local_int(&engine, visitor, "observed_after_execute"), 1);
    assert_eq!(
        engine.test_object_snapshot(visitor).container,
        Some(base),
        "successful activation leaves Exit pending for its next execution"
    );
}

#[test]
fn script_execute_command_synchronously_rejects_hostile_exit() {
    let (mut engine, base) = entrance_fixture(true, true, true);
    let visitor = spawn_visitor(&mut engine, Some(base));

    assert_eq!(
        call(
            &mut engine,
            visitor,
            "ExecuteExitAndRead",
            vec![Value::Object(base.as_u64())],
        ),
        Value::Nil
    );
    assert_eq!(local_int(&engine, base, "activate_count"), 0);
    assert!(
        engine
            .test_object_snapshot(visitor)
            .command_stack
            .command_names()
            .is_empty(),
        "hostile ExecuteCommand resolves the failed Exit before returning"
    );
    assert_hostile_entrance_message(&engine, base);
}

#[test]
fn script_execute_command_uses_same_call_cached_entrance_ocf() {
    let (mut engine, base) = entrance_fixture(true, false, true);
    let visitor = spawn_visitor(&mut engine, Some(base));

    assert_eq!(
        call(
            &mut engine,
            visitor,
            "ExecuteExitAfterRemovingEntrance",
            vec![Value::Object(base.as_u64())],
        ),
        Value::Nil,
        "the OCF-rejected Exit finishes before ExecuteCommand returns"
    );
    let base_index = engine.test_object_index(base);
    assert_eq!(engine.objects[base_index].definition_id, "NENT");
    assert_eq!(engine.objects[base_index].state.ocf & ocf::ENTRANCE, 0);
    assert!(
        !engine.objects[base_index].state.entrance_status,
        "stale OCF must not call NENT's ActivateEntrance callback"
    );
    assert!(engine
        .test_object_snapshot(visitor)
        .command_stack
        .command_names()
        .is_empty());
}

#[test]
fn closed_exit_without_current_entrance_ocf_skips_callback_and_fails() {
    let (mut engine, base) = entrance_fixture(false, false, false);
    assert_eq!(
        engine.test_object_snapshot(base).ocf & ocf::ENTRANCE,
        0,
        "fixture has no current OCF_Entrance"
    );
    let visitor = spawn_visitor(&mut engine, Some(base));
    assert_eq!(
        call(&mut engine, visitor, "StartExit", Vec::new()),
        Value::Bool(true)
    );

    crate::support::TestValueExt::test_value(engine.tick_without_snapshot());
    assert_eq!(local_int(&engine, base, "activate_count"), 0);
    assert_eq!(engine.test_object_snapshot(visitor).container, Some(base));
    assert!(engine.snapshot().hud.messages.is_empty());
    assert!(engine
        .test_object_snapshot(visitor)
        .command_stack
        .command_names()
        .is_empty());
}

#[test]
fn disabling_base_reject_gate_allows_hostile_activate_entrance() {
    let (mut engine, base) = entrance_fixture(true, true, false);
    let visitor = spawn_visitor(&mut engine, None);
    assert_eq!(
        call(
            &mut engine,
            visitor,
            "StartEnter",
            vec![Value::Object(base.as_u64())],
        ),
        Value::Bool(true)
    );

    crate::support::TestValueExt::test_value(engine.tick_without_snapshot());
    assert_eq!(local_int(&engine, base, "activate_count"), 1);
    assert_eq!(
        engine
            .object_snapshot(base)
            .and_then(|snapshot| snapshot.local_vars.get("activate_caller").cloned()),
        Some(Value::Object(visitor.as_u64()))
    );
    assert!(engine.snapshot().hud.messages.is_empty());

    crate::support::TestValueExt::test_value(engine.tick_without_snapshot());
    assert_eq!(engine.test_object_snapshot(visitor).container, Some(base));
    assert_eq!(
        local_int(&engine, base, "activate_count"),
        1,
        "the open recheck does not call ActivateEntrance twice"
    );
}

#[test]
fn base_reject_entrance_flag_survives_state_restore() {
    let (mut engine, base) = entrance_fixture(true, true, false);
    let state = engine.capture_state();
    assert_eq!(state.base_reject_entrance_enabled, Some(false));

    engine.set_base_reject_entrance_enabled(true);
    crate::support::TestValueExt::test_value(engine.restore_state(&state));

    let visitor = spawn_visitor(&mut engine, None);
    assert_eq!(
        call(
            &mut engine,
            visitor,
            "StartEnter",
            vec![Value::Object(base.as_u64())],
        ),
        Value::Bool(true)
    );
    crate::support::TestValueExt::test_value(engine.tick_without_snapshot());
    assert_eq!(
        local_int(&engine, base, "activate_count"),
        1,
        "restoring a flag-off scenario must not re-enable hostile rejection"
    );
    assert!(engine.snapshot().hud.messages.is_empty());
}

#[test]
fn legacy_section_switch_projects_its_base_reject_entrance_mask(
) -> Result<(), Box<dyn std::error::Error>> {
    // C4ScenarioSection overlays its Scenario.txt onto the main core. A
    // section that explicitly masks RejectEntrance off must install that
    // runtime policy when LoadScenarioSection switches to it.
    let temp = tempdir()?;
    let scenario_dir = temp.path().join("EntranceSections.c4s");
    fs::create_dir(&scenario_dir)?;
    let probe = scenario_dir.join("Defs.c4d/Probe.c4d");
    fs::create_dir_all(&probe)?;
    fs::write(
        probe.join("DefCore.txt"),
        "[DefCore]\nid=PRBE\nName=Probe\nCategory=1\n",
    )?;
    fs::write(probe.join("Script.c"), "#strict\n")?;
    image::RgbaImage::from_pixel(1, 1, image::Rgba([1, 2, 3, 255]))
        .save(probe.join("Graphics.png"))?;
    fs::write(
        scenario_dir.join("Scenario.txt"),
        "[Head]\nTitle=Entrance section projection\n\
         [Definitions]\nDefinition1=Defs.c4d\n\
         [Game]\nBaseFunctionality=BASEFUNC_RejectEntrance\n",
    )?;
    fs::write(
        scenario_dir.join("Script.c"),
        "#strict\n\
         global func SwitchMaskOff() { return LoadScenarioSection(\"Target\", 0); }\n\
         global func SwitchMaskOn() { return LoadScenarioSection(\"main\", 0); }\n",
    )?;
    let target = scenario_dir.join("SectTarget.c4g");
    fs::create_dir(&target)?;
    fs::write(target.join("Scenario.txt"), "[Game]\nBaseFunctionality=0\n")?;

    let scenario = Scenario::load_from_path_with(&scenario_dir, &LocalDefinitionResolver)?;
    let mut engine = Engine::new();
    scenario.apply(&mut engine)?;
    assert_eq!(
        engine.capture_state().base_reject_entrance_enabled,
        Some(true),
        "the main section enables hostile entrance rejection"
    );

    let main_base = install_entrance_fixture(&mut engine, true, true, None);
    let main_visitor = spawn_visitor(&mut engine, None);
    call(
        &mut engine,
        main_visitor,
        "StartEnter",
        vec![Value::Object(main_base.as_u64())],
    );
    engine.tick_without_snapshot()?;
    assert_eq!(local_int(&engine, main_base, "activate_count"), 0);
    assert_hostile_entrance_message(&engine, main_base);

    engine.call_scenario_script_function("SwitchMaskOff", Vec::new())?;
    assert_eq!(engine.debug_current_scenario_section(), "Target");
    assert_eq!(
        engine.capture_state().base_reject_entrance_enabled,
        Some(false),
        "the target section's explicit zero mask becomes live"
    );

    let target_base = spawn_closed_base(&mut engine);
    let target_visitor = spawn_visitor(&mut engine, None);
    call(
        &mut engine,
        target_visitor,
        "StartEnter",
        vec![Value::Object(target_base.as_u64())],
    );
    engine.tick_without_snapshot()?;
    assert_eq!(
        local_int(&engine, target_base, "activate_count"),
        1,
        "the same hostile visitor reaches the callback after the section mask disables rejection"
    );

    engine.call_scenario_script_function("SwitchMaskOn", Vec::new())?;
    assert_eq!(engine.debug_current_scenario_section(), "main");
    assert_eq!(
        engine.capture_state().base_reject_entrance_enabled,
        Some(true),
        "switching back restores the main section's rejection mask"
    );

    Ok(())
}

#[test]
fn false_activation_does_not_fail_callback_replacement_exit() {
    let (mut engine, base) = entrance_fixture(true, false, true);
    let visitor = spawn_visitor(&mut engine, Some(base));
    crate::support::TestValueExt::test_value(engine.apply_object_update(
        visitor,
        ObjectUpdate::new().with_command_direction(clonk_engine::CommandDirection::Right),
    ));
    assert_eq!(
        call(&mut engine, base, "ReplaceExitOnce", Vec::new()),
        Value::Int(1)
    );
    assert_eq!(
        call(&mut engine, visitor, "StartExit", Vec::new()),
        Value::Bool(true)
    );

    crate::support::TestValueExt::test_value(engine.tick_without_snapshot());
    assert_eq!(local_int(&engine, base, "activate_count"), 1);
    assert_eq!(
        engine.test_object_snapshot(visitor).command_direction,
        clonk_engine::CommandDirection::Stop,
        "the detached false Exit still runs C4Command::Fail feedback"
    );
    assert_eq!(
        engine
            .test_object_snapshot(visitor)
            .command_stack
            .command_names(),
        ["Exit"],
        "the callback-installed Exit survives the old false result"
    );

    crate::support::TestValueExt::test_value(engine.tick_without_snapshot());
    assert_eq!(
        local_int(&engine, base, "activate_count"),
        1,
        "InitEvaluation does not activate the entrance"
    );

    crate::support::TestValueExt::test_value(engine.tick_without_snapshot());
    assert_eq!(
        local_int(&engine, base, "activate_count"),
        2,
        "the replacement executes instead of inheriting the old failure"
    );
    let base_index = engine.test_object_index(base);
    assert!(engine.objects[base_index].state.entrance_status);
    assert_eq!(
        engine.test_object_snapshot(visitor).container,
        Some(base),
        "a successful activation leaves Exit pending until its recheck"
    );

    crate::support::TestValueExt::test_value(engine.tick_without_snapshot());
    assert_eq!(engine.test_object_snapshot(visitor).container, None);
}

#[test]
fn reentrant_activation_resolves_each_attempt_on_the_same_exit() {
    let (mut engine, base) = entrance_fixture(true, false, true);
    let visitor = spawn_visitor(&mut engine, Some(base));
    assert_eq!(
        call(&mut engine, base, "ReenterExitOnce", Vec::new()),
        Value::Int(1)
    );

    assert_eq!(
        call(
            &mut engine,
            visitor,
            "ExecuteExitAndRead",
            vec![Value::Object(base.as_u64())],
        ),
        Value::Int(2),
        "the guarded inner ExecuteCommand runs a second activation"
    );
    assert!(
        engine
            .test_object_snapshot(visitor)
            .command_stack
            .command_names()
            .is_empty(),
        "the outer false result still fails the same Exit after inner success"
    );
}
