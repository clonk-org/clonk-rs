use crate::support::real_scenario::content_root;
use clonk_engine::{AudioCommand, Definition, Engine, SpawnConfig};
use clonk_resources::Group;
use clonk_script::Value;

fn entrance_status(engine: &Engine, object: clonk_engine::ObjectId) -> bool {
    let index = engine
        .find_object_index(object)
        .expect("object remains live");
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
    engine
        .register_script_definition("ENTR", "Entrance probe", script)
        .expect("entrance probe registers");
    let caller = engine
        .spawn_object(SpawnConfig::new("ENTR"))
        .expect("caller spawns");
    let open = engine
        .spawn_object(SpawnConfig::new("ENTR").with_entrance_status(true))
        .expect("open target spawns");

    let caller_index = engine
        .find_object_index(caller)
        .expect("caller remains live");
    assert_eq!(
        engine
            .call_object_function(caller_index, "Probe", Vec::new())
            .expect("same-call entrance probe runs"),
        Value::Array(vec![
            Value::Int(0),
            Value::Int(1),
            Value::Int(1),
            Value::Int(0),
        ])
    );
    assert!(!entrance_status(&engine, caller));
    assert_eq!(
        engine
            .call_object_function(caller_index, "Read", vec![Value::Object(open.as_u64())])
            .expect("explicit target read runs"),
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
    engine
        .register_script_definition("ENTR", "Entrance setter", script)
        .expect("entrance setter registers");
    let caller = engine
        .spawn_object(SpawnConfig::new("ENTR"))
        .expect("caller spawns");
    let target = engine
        .spawn_object(SpawnConfig::new("ENTR"))
        .expect("target spawns");
    let caller_index = engine
        .find_object_index(caller)
        .expect("caller remains live");

    assert_eq!(
        engine
            .call_object_function(caller_index, "Open", vec![Value::Object(target.as_u64())],)
            .expect("foreign SetEntrance runs"),
        Value::Array(vec![Value::Bool(true), Value::Int(1), Value::Int(0)])
    );
    assert!(!entrance_status(&engine, caller));
    assert!(entrance_status(&engine, target));
}

#[test]
fn scenario_set_entrance_updates_explicit_targets_without_context_object() {
    let mut engine = Engine::new();
    engine
        .register_script_definition("ENTR", "Entrance target", "#strict 3")
        .expect("entrance target registers");
    let target = engine
        .spawn_object(SpawnConfig::new("ENTR"))
        .expect("target spawns");
    let witness = engine
        .spawn_object(SpawnConfig::new("ENTR"))
        .expect("witness spawns");
    engine
        .load_scenario_script_with_convention(
            "Scenario entrance setter",
            r#"#strict 3
func Open(object target, object witness)
{
  if (SetEntrance(1, target)) SetEntrance(1, witness);
}
"#,
            true,
        )
        .expect("scenario entrance setter loads");

    engine
        .call_scenario_script_function(
            "Open",
            vec![
                Value::Object(target.as_u64()),
                Value::Object(witness.as_u64()),
            ],
        )
        .expect("scenario SetEntrance runs");

    assert!(entrance_status(&engine, target));
    assert!(
        entrance_status(&engine, witness),
        "the witness proves the first SetEntrance returned true"
    );
}

#[test]
fn shipped_sub_airlock_toggles_once_per_transition() {
    let group = Group::open(content_root().join("Objects.c4d/Vehicles.c4d/Sub.c4d"))
        .expect("shipped Sub group opens");
    let resource = clonk_resources::definition::Definition::load(&group)
        .expect("shipped Sub definition loads");
    let mut engine = Engine::new();
    engine
        .register_definition(Definition::from_resource(&resource).expect("Sub script compiles"))
        .expect("Sub registers");
    let sub = engine
        .spawn_object(SpawnConfig::new("SUB1").with_loaded(true))
        .expect("loaded Sub spawns without Completion dependencies");
    let sub_index = engine.find_object_index(sub).expect("Sub remains live");

    assert_eq!(
        engine
            .call_object_function(sub_index, "OpenAirlock", Vec::new())
            .expect("first OpenAirlock runs"),
        Value::Int(1)
    );
    assert!(entrance_status(&engine, sub));
    assert_eq!(
        engine
            .call_object_function(sub_index, "OpenAirlock", Vec::new())
            .expect("second OpenAirlock runs"),
        Value::Nil
    );
    assert!(entrance_status(&engine, sub));
    assert_eq!(
        engine
            .call_object_function(sub_index, "CloseAirlock", Vec::new())
            .expect("first CloseAirlock runs"),
        Value::Int(1)
    );
    assert!(!entrance_status(&engine, sub));
    assert_eq!(
        engine
            .call_object_function(sub_index, "CloseAirlock", Vec::new())
            .expect("second CloseAirlock runs"),
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
