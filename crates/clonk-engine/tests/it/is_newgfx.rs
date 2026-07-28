use crate::support::real_scenario::content_root;
use clonk_engine::scenario::load_system_scripts;
use clonk_engine::{Definition, Engine, SpawnConfig, Vector2};
use clonk_resources::Group;
use clonk_script::Value;

#[test]
fn is_newgfx_returns_true_unconditionally() {
    let mut engine = Engine::new();
    engine
        .register_definition(
            Definition::from_script(
                "NGFX",
                "IsNewgfx probe",
                r#"#strict
func Probe()
{
  return IsNewgfx();
}
"#,
            )
            .expect("IsNewgfx probe compiles"),
        )
        .expect("IsNewgfx probe registers");
    let probe = engine
        .spawn_object(SpawnConfig::new("NGFX"))
        .expect("IsNewgfx probe spawns");
    let probe_index = engine.find_object_index(probe).expect("probe remains live");

    assert_eq!(
        engine
            .call_object_function(probe_index, "Probe", Vec::new())
            .expect("IsNewgfx is script-callable"),
        Value::Bool(true)
    );
}

#[test]
fn shipped_revaluation_newgfx_branch_executes() {
    let content = content_root();
    let group = Group::open(content.join("Objects.c4d/Magic.c4d/Revaluation.c4d"))
        .expect("shipped Revaluation group opens");
    let resource = clonk_resources::definition::Definition::load(&group)
        .expect("shipped Revaluation definition loads");
    let system_group = Group::open(
        content
            .parent()
            .expect("content root has a repository parent")
            .join("planet/System.c4g"),
    )
    .expect("installed System.c4g opens");
    let system_scripts =
        load_system_scripts(&system_group).expect("installed System.c4g scripts load");
    let mut engine = Engine::with_seed(0);
    engine.install_global_scripts(&system_scripts);
    engine
        .register_definition(
            Definition::from_resource(&resource).expect("shipped Revaluation script compiles"),
        )
        .expect("shipped Revaluation registers");
    for (id, name) in [
        ("TARG", "Revaluation target"),
        ("GOLD", "Gold"),
        ("ROCK", "Rock"),
    ] {
        engine
            .register_script_definition(id, name, "#strict\n")
            .expect("support definition registers");
    }

    let position = Vector2::new(40, 40);
    let spell = engine
        .spawn_object(
            SpawnConfig::new("RVLT")
                .with_position(position)
                .with_loaded(true),
        )
        .expect("shipped Revaluation object spawns");
    let target = engine
        .spawn_object(SpawnConfig::new("TARG").with_position(position))
        .expect("Revaluation target spawns");
    let gold = engine
        .spawn_object(SpawnConfig::new("GOLD").with_container(target))
        .expect("target carries gold");
    let rock = engine
        .spawn_object(SpawnConfig::new("ROCK").with_position(position))
        .expect("nearby rock spawns in free air");
    let spell_index = engine
        .find_object_index(spell)
        .expect("Revaluation object remains live");

    assert_eq!(
        engine
            .call_object_function(
                spell_index,
                "NoRevaluation",
                vec![
                    Value::Object(target.as_u64()),
                    Value::Object(spell.as_u64()),
                    Value::String("unused".to_owned().into()),
                ],
            )
            .expect("shipped IsNewgfx branch executes without an unknown-function error"),
        Value::Int(1)
    );
    assert_eq!(
        engine
            .object_snapshot(rock)
            .expect("converted rock remains live")
            .definition_id,
        "GOLD"
    );
    assert!(
        engine
            .object_snapshot(gold)
            .is_none_or(|gold| !gold.status.is_active()),
        "the spell consumes the carried gold after taking the IsNewgfx branch"
    );
}
