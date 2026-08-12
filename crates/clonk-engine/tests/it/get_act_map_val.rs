use crate::support::EngineTestExt;
use std::fs;
use std::path::Path;

use crate::support::real_scenario::content_root;
use clonk_engine::{physical_action_graphics_key, ActionState, Definition, Engine, SpawnConfig};
use clonk_resources::{Group, ResourceDefinition};
use clonk_script::Value;
use tempfile::tempdir;

fn write_definition(path: &Path, id: &str, action: &str, length: i32, script: &str) {
    crate::support::TestValueExt::test_value(fs::create_dir_all(path));
    crate::support::TestValueExt::test_value(fs::write(
        path.join("DefCore.txt"),
        format!("[DefCore]\nid={id}\nName={id}\n"),
    ));
    crate::support::TestValueExt::test_value(fs::write(
        path.join("ActMap.txt"),
        format!("[Action]\nName={action}\nLength={length}\n"),
    ));
    crate::support::TestValueExt::test_value(fs::write(path.join("Script.c"), script));
}

#[test]
fn implicit_get_act_map_val_keeps_the_suspended_definition_after_change_def() {
    let root = crate::support::TestValueExt::test_value(tempdir());
    let old = root.path().join("Old.c4d");
    let new = root.path().join("New.c4d");
    write_definition(
        &old,
        "OLD1",
        "Probe",
        11,
        r#"#strict 2
func Probe()
{
  var before = GetActMapVal("Length", "Probe");
  var sound = GetActMapVal("Sound", "Probe");
  var disabled = GetActMapVal("ObjectDisabled", "Probe");
  var step = GetActMapVal("Step", "Probe");
  ChangeDef(NEW2);
  return [before, sound, disabled, step,
          GlobalActLength(),
          GetActMapVal("Length", "Probe"),
          GetActMapVal("Length", "Probe", 0),
          GetActMapVal("Length", "Probe", OLD1),
          GetActMapVal("Length", "Probe", NEW2),
          DefinitionCall(NEW2, "DefinitionActLength"),
          GameCall("ScenarioActLength")];
}
"#,
    );
    crate::support::TestValueExt::test_value(fs::write(
        old.join("ActMap.txt"),
        "[Action]\nName=Probe\nLength=11\nSound=Zap\nObjectDisabled=2\nStep=-11\n\
         [Action]\nName=Probe\nLength=99\n",
    ));
    write_definition(
        &new,
        "NEW2",
        "Probe",
        22,
        r#"#strict 2
func DefinitionActLength()
{
  return GetActMapVal("Length", "Probe");
}
"#,
    );

    let old = crate::support::TestValueExt::test_value(ResourceDefinition::load(
        &crate::support::TestValueExt::test_value(Group::open(&old)),
    ));
    let new = crate::support::TestValueExt::test_value(ResourceDefinition::load(
        &crate::support::TestValueExt::test_value(Group::open(&new)),
    ));
    let mut engine = Engine::new();
    engine.install_global_scripts(&[(
        "GetActMapValGlobal.c".to_string(),
        r#"#strict 2
global func GlobalActLength()
{
  return GetActMapVal("Length", "Probe");
}
"#
        .to_string(),
    )]);
    crate::support::TestValueExt::test_value(engine.install_scenario_script(
        "GetActMapValScenario.c",
        r#"#strict 2
    func ScenarioActLength()
    {
      return GetActMapVal("Length", "Probe");
    }
    "#,
    ));
    engine.register_test_definition(crate::support::TestValueExt::test_value(
        Definition::from_resource(&old),
    ));
    engine.register_test_definition(crate::support::TestValueExt::test_value(
        Definition::from_resource(&new),
    ));
    let object = engine.spawn_test_object(
        SpawnConfig::new("OLD1")
            .with_loaded(true)
            .with_action(ActionState::new("Probe")),
    );
    let index = engine.test_object_index(object);

    assert_eq!(
        engine.call_test_object_function(index, "Probe", vec![]),
        Value::Array(vec![
            Value::Int(11),
            Value::String("Zap".to_string().into()),
            Value::Int(2),
            Value::Int(-11),
            Value::Int(11),
            Value::Int(11),
            Value::Int(11),
            Value::Int(11),
            Value::Int(22),
            Value::Int(22),
            Value::Nil,
        ])
    );
}

#[test]
fn duplicate_actions_keep_cpp_first_name_and_last_next_action_semantics() {
    let root = crate::support::TestValueExt::test_value(tempdir());
    let path = root.path().join("Duplicate.c4d");
    crate::support::TestValueExt::test_value(fs::create_dir_all(&path));
    crate::support::TestValueExt::test_value(fs::write(
        path.join("DefCore.txt"),
        "[DefCore]\nid=DUPA\nName=Duplicate action probe\n",
    ));
    crate::support::TestValueExt::test_value(fs::write(
        path.join("ActMap.txt"),
        "[Action]\nName=Source\nLength=1\nDelay=1\nNextAction=Dup\nFacet=0,0,4,4\n\
         [Action]\nName=Dup\nLength=2\nDelay=1\nNextAction=Hold\nStartCall=FirstStart\nFacet=1,2,10,11\n\
         [Action]\nName=Dup\nLength=5\nDelay=1\nNextAction=Hold\nNoOtherAction=1\nAbortCall=LastAbort\nFacet=3,4,30,31\n",
    ));
    crate::support::TestValueExt::test_value(fs::write(
        path.join("Script.c"),
        "#strict 2\nlocal callback_result;\n\
         func FirstStart() { callback_result += 1; }\n\
         func LastAbort() { callback_result += 10; }\n\
         func TryRebind() { return SetAction(\"Dup\"); }\n\
         func ForceRebind() { callback_result = 0; return [SetAction(\"Dup\", 0, 0, true), callback_result]; }\n\
         func FirstLength() { return GetActMapVal(\"Length\", \"Dup\"); }\n",
    ));

    let resource = crate::support::TestValueExt::test_value(ResourceDefinition::load(
        &crate::support::TestValueExt::test_value(Group::open(&path)),
    ));
    let definition = crate::support::TestValueExt::test_value(Definition::from_resource(&resource));
    let graphics = definition.action_graphics();
    assert_eq!(
        graphics
            .get("Dup")
            .and_then(|value| value.facet.as_ref())
            .map(|facet| facet.width),
        Some(10)
    );
    assert_eq!(
        graphics
            .get(&physical_action_graphics_key(2))
            .and_then(|value| value.facet.as_ref())
            .map(|facet| facet.width),
        Some(30),
        "physical graphics stay aligned with the later duplicate"
    );

    let mut engine = Engine::new();
    engine.register_test_definition(definition);
    let object = engine.spawn_test_object(
        SpawnConfig::new("DUPA")
            .with_loaded(true)
            .with_action(ActionState::new("Source")),
    );

    crate::support::TestValueExt::test_value(engine.tick_without_snapshot());
    let snapshot = engine.test_object_snapshot(object);
    assert_eq!(snapshot.action.name, "Dup");
    assert_eq!(snapshot.action.act_map_index, Some(2));
    for _ in 0..3 {
        crate::support::TestValueExt::test_value(engine.tick_without_snapshot());
    }
    assert_eq!(
        engine.test_object_snapshot(object).action.phase,
        3,
        "the last duplicate's Length=5 remains active after the transition"
    );

    let index = engine.test_object_index(object);
    assert_eq!(
        engine.call_test_object_function(index, "FirstLength", vec![]),
        Value::Int(2),
        "GetActMapVal scans duplicate names from the start"
    );
    assert_eq!(
        engine.call_test_object_function(index, "TryRebind", vec![]),
        Value::Bool(false),
        "the later duplicate's NoOtherAction blocks the first same-name slot"
    );
    assert_eq!(
        engine.test_object_snapshot(object).action.act_map_index,
        Some(2)
    );
    assert_eq!(
        engine.call_test_object_function(index, "ForceRebind", vec![]),
        Value::Array(vec![Value::Bool(true), Value::Int(1)]),
        "forced SetAction runs the first slot's StartCall and suppresses AbortCall"
    );
    let snapshot = engine.test_object_snapshot(object);
    assert_eq!(snapshot.action.act_map_index, Some(1));
    assert_eq!(snapshot.action.phase, 0);
}

#[test]
fn shipped_hazard_trail_gets_its_real_facet_width_by_entry_index() {
    let group = crate::support::TestValueExt::test_value(Group::open(
        content_root().join("Hazard.c4d/Items.c4d/Weapons.c4d/Weapon.c4d/Shot.c4d/Trail.c4d"),
    ));
    let resource = crate::support::TestValueExt::test_value(ResourceDefinition::load(&group));
    let mut engine = Engine::new();
    engine.register_test_definition(crate::support::TestValueExt::test_value(
        Definition::from_resource(&resource),
    ));
    engine.register_test_script_definition("SHOT", "Trail shot stub", "#strict\n");
    let shot = engine.spawn_test_object(SpawnConfig::new("SHOT").with_loaded(true));
    let trail = engine.spawn_test_object(SpawnConfig::new("TRAI").with_loaded(true));
    let index = engine.test_object_index(trail);

    assert_eq!(
        engine.call_test_object_function(
            index,
            "Set",
            vec![
                Value::Int(20),
                Value::Int(100),
                Value::Object(shot.as_u64()),
                Value::String("Travel".to_string().into()),
            ],
        ),
        Value::Nil
    );
    assert_eq!(
        engine
            .test_object_snapshot(trail)
            .draw_transform
            .expect("Trail DrawTransform ran")
            .matrix(),
        [1.0, 0.0, -11.0, 0.0, 0.0, 0.0, 0.0, 0.0, 1.0],
        "real Trail Script.c:69 uses Travel Facet.Wdt through entry_nr=2"
    );
}
