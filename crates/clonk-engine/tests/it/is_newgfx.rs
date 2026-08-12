use crate::support::real_scenario::content_root;
use crate::support::EngineTestExt;
use clonk_engine::scenario::load_system_scripts;
use clonk_engine::{Definition, Engine, SpawnConfig, Vector2};
use clonk_resources::Group;
use clonk_script::Value;

#[test]
fn is_newgfx_returns_true_unconditionally() {
    let mut engine = Engine::new();
    engine.register_test_definition(crate::support::TestValueExt::test_value(
        Definition::from_script(
            "NGFX",
            "IsNewgfx probe",
            r#"#strict
        func Probe()
        {
          return IsNewgfx();
        }
        "#,
        ),
    ));
    let probe = engine.spawn_test_object(SpawnConfig::new("NGFX"));
    let probe_index = engine.test_object_index(probe);

    assert_eq!(
        engine.call_test_object_function(probe_index, "Probe", Vec::new()),
        Value::Bool(true)
    );
}

#[test]
fn shipped_revaluation_newgfx_branch_executes() {
    let content = content_root();
    let group = crate::support::TestValueExt::test_value(Group::open(
        content.join("Objects.c4d/Magic.c4d/Revaluation.c4d"),
    ));
    let resource = crate::support::TestValueExt::test_value(
        clonk_resources::definition::Definition::load(&group),
    );
    let system_group = crate::support::TestValueExt::test_value(Group::open(
        crate::support::TestValueExt::test_value(content.parent()).join("planet/System.c4g"),
    ));
    let system_scripts =
        crate::support::TestValueExt::test_value(load_system_scripts(&system_group));
    let mut engine = Engine::with_seed(0);
    engine.install_global_scripts(&system_scripts);
    engine.register_test_definition(crate::support::TestValueExt::test_value(
        Definition::from_resource(&resource),
    ));
    for (id, name) in [
        ("TARG", "Revaluation target"),
        ("GOLD", "Gold"),
        ("ROCK", "Rock"),
    ] {
        engine.register_test_script_definition(id, name, "#strict\n");
    }

    let position = Vector2::new(40, 40);
    let spell = engine.spawn_test_object(
        SpawnConfig::new("RVLT")
            .with_position(position)
            .with_loaded(true),
    );
    let target = engine.spawn_test_object(SpawnConfig::new("TARG").with_position(position));
    let gold = engine.spawn_test_object(SpawnConfig::new("GOLD").with_container(target));
    let rock = engine.spawn_test_object(SpawnConfig::new("ROCK").with_position(position));
    let spell_index = engine.test_object_index(spell);

    assert_eq!(
        engine.call_test_object_function(
            spell_index,
            "NoRevaluation",
            vec![
                Value::Object(target.as_u64()),
                Value::Object(spell.as_u64()),
                Value::String("unused".to_owned().into()),
            ],
        ),
        Value::Int(1)
    );
    assert_eq!(engine.test_object_snapshot(rock).definition_id, "GOLD");
    assert!(
        engine
            .object_snapshot(gold)
            .is_none_or(|gold| !gold.status.is_active()),
        "the spell consumes the carried gold after taking the IsNewgfx branch"
    );
}
