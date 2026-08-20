use crate::support::real_scenario::content_root;
use crate::support::EngineTestExt;
use clonk_engine::{
    AudioCommand, CommandDirection, Definition, Engine, ObjectUpdate, SpawnConfig, COM_RIGHT,
};
use clonk_resources::Group;
use clonk_script::Value;

fn entrance_status(engine: &Engine, object: clonk_engine::ObjectId) -> bool {
    let index = engine.test_object_index(object);
    engine.objects[index].state.entrance_status
}

#[test]
fn get_entrance_reads_explicit_targets_and_same_call_set_entrance_writes() {
    let script = r#"#strict
func Probe()
{
  var before = GetEntrance();
  SetEntrance(1);
  var after_set = GetEntrance(0);
  var explicit_self = GetEntrance(this());
  SetEntrance(0);
  return([before, after_set, explicit_self, GetEntrance()]);
}

func Read(object target)
{
  return(GetEntrance(target));
}
"#;
    let mut engine = Engine::new();
    engine.register_test_script_definition("ENTR", "Entrance probe", script);
    let caller = engine.spawn_test_object(SpawnConfig::new("ENTR"));
    let open = engine.spawn_test_object(SpawnConfig::new("ENTR").with_entrance_status(true));

    let caller_index = engine.test_object_index(caller);
    assert_eq!(
        engine.call_test_object_function(caller_index, "Probe", Vec::new()),
        Value::Array(vec![
            Value::Int(0),
            Value::Int(1),
            Value::Int(1),
            Value::Int(0),
        ])
    );
    assert!(!entrance_status(&engine, caller));
    assert_eq!(
        engine.call_test_object_function(caller_index, "Read", vec![Value::Object(open.as_u64())]),
        Value::Int(1)
    );
}

#[test]
fn set_entrance_updates_explicit_foreign_target() {
    let script = r#"#strict 3
func Open(object target)
{
  return [SetEntrance(1, target), GetEntrance(target), GetEntrance()];
}
"#;
    let mut engine = Engine::new();
    engine.register_test_script_definition("ENTR", "Entrance setter", script);
    let caller = engine.spawn_test_object(SpawnConfig::new("ENTR"));
    let target = engine.spawn_test_object(SpawnConfig::new("ENTR"));
    let caller_index = engine.test_object_index(caller);

    assert_eq!(
        engine.call_test_object_function(
            caller_index,
            "Open",
            vec![Value::Object(target.as_u64())],
        ),
        Value::Array(vec![Value::Bool(true), Value::Int(1), Value::Int(0)])
    );
    assert!(!entrance_status(&engine, caller));
    assert!(entrance_status(&engine, target));
}

#[test]
fn scenario_set_entrance_updates_explicit_targets_without_context_object() {
    let mut engine = Engine::new();
    engine.register_test_script_definition("ENTR", "Entrance target", "#strict 3");
    let target = engine.spawn_test_object(SpawnConfig::new("ENTR"));
    let witness = engine.spawn_test_object(SpawnConfig::new("ENTR"));
    crate::support::TestValueExt::test_value(engine.load_scenario_script_with_convention(
        "Scenario entrance setter",
        r#"#strict 3
    func Open(object target, object witness)
    {
      if (SetEntrance(1, target)) SetEntrance(1, witness);
    }
    "#,
        true,
    ));

    engine.call_test_scenario_script_function(
        "Open",
        vec![
            Value::Object(target.as_u64()),
            Value::Object(witness.as_u64()),
        ],
    );

    assert!(entrance_status(&engine, target));
    assert!(
        entrance_status(&engine, witness),
        "the witness proves the first SetEntrance returned true"
    );
}

/// Shipped `SUB1` steers from its passenger, and the two control styles reach
/// that through **different shipped callbacks** which must agree.
///
/// Classic control goes through `ContainedRight`, which guards its call on
/// `!GetPlrJumpAndRunControl(clonk->GetController())` and composes the new
/// heading out of the current one with `ComDirTransform(GetComDir(),
/// COMD_Right)`. Jump'n'Run is deliberately deferred past that guard to
/// `ContainedUpdate`, which calls `SetDirection(comdir)` with the aggregated
/// direction instead (`Objects.c4d/Vehicles.c4d/Sub.c4d/Script.c:66-69,93-101`).
/// Either way `SetDirection` only reaches `SetComDir` while the boat is in its
/// `Swim` action (`:27-45`).
///
/// `C4Object::ContainedControl` is what routes a contained crew member's
/// command to its container at all (C4Object.cpp:3246-3282). The airlock
/// subcase below covers the entrance toggle; this covers the other half the
/// vehicle matrix names — a passenger actually steering.
///
/// Asserting the two styles land on the *same* heading is the point. Each
/// alone would pass with one path silently broken, and Jump'n'Run is the
/// default a new player gets while classic is what the guard is written for.
#[test]
fn shipped_sub_passenger_steering_agrees_across_control_styles() {
    let mut headings = Vec::new();
    for jump_and_run in [false, true] {
        let mut engine = crate::support::real_scenario::load_tutorial(7, 0);
        let owner = crate::support::real_scenario::join_local_player(&mut engine, "sub control");
        crate::support::TestValueExt::test_value(engine.player_mut(owner))
            .control
            .control_style = jump_and_run;
        let pilot = crate::support::TestValueExt::test_value(engine.crew_cursor(owner));

        let sub = engine.spawn_test_object(
            SpawnConfig::new("SUB1")
                .with_loaded(true)
                .with_owner(owner)
                .with_in_liquid(true),
        );
        crate::support::TestValueExt::test_value(
            engine.apply_object_update(
                sub,
                ObjectUpdate::new()
                    .with_action("Swim")
                    .with_command_direction(CommandDirection::Stop),
            ),
        );
        engine.debug_set_in_liquid(sub, true);
        crate::support::TestValueExt::test_value(
            engine.apply_object_update(pilot, ObjectUpdate::new().with_container(sub)),
        );

        crate::support::TestValueExt::test_value(engine.player_in_com(owner, COM_RIGHT, 0));
        headings.push(engine.test_object_snapshot(sub).command_direction);
    }

    assert_ne!(
        headings[0],
        CommandDirection::Stop,
        "classic control routes ContainedRight into SetDirection"
    );
    assert_eq!(
        headings[0], headings[1],
        "Jump'n'Run reaches the same heading through ContainedUpdate: {headings:?}"
    );
}

#[test]
fn shipped_sub_airlock_toggles_once_per_transition() {
    let group = crate::support::TestValueExt::test_value(Group::open(
        content_root().join("Objects.c4d/Vehicles.c4d/Sub.c4d"),
    ));
    let resource = crate::support::TestValueExt::test_value(
        clonk_resources::definition::Definition::load(&group),
    );
    let mut engine = Engine::new();
    engine.register_test_definition(crate::support::TestValueExt::test_value(
        Definition::from_resource(&resource),
    ));
    let sub = engine.spawn_test_object(SpawnConfig::new("SUB1").with_loaded(true));
    let sub_index = engine.test_object_index(sub);

    assert_eq!(
        engine.call_test_object_function(sub_index, "OpenAirlock", Vec::new()),
        Value::Int(1)
    );
    assert!(entrance_status(&engine, sub));
    assert_eq!(
        engine.call_test_object_function(sub_index, "OpenAirlock", Vec::new()),
        Value::Nil
    );
    assert!(entrance_status(&engine, sub));
    assert_eq!(
        engine.call_test_object_function(sub_index, "CloseAirlock", Vec::new()),
        Value::Int(1)
    );
    assert!(!entrance_status(&engine, sub));
    assert_eq!(
        engine.call_test_object_function(sub_index, "CloseAirlock", Vec::new()),
        Value::Nil
    );
    assert!(!entrance_status(&engine, sub));

    let sounds = engine
        .pending_audio
        .iter()
        .filter_map(|command| match command {
            AudioCommand::PlaySound { name, target, .. } if *target == Some(sub) => {
                Some(name.as_str())
            }
            _ => None,
        })
        .collect::<Vec<_>>();
    assert_eq!(sounds, ["Airlock1", "Airlock2"]);
}
