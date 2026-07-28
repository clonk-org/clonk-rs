use std::collections::{HashMap, HashSet};

use crate::support::real_scenario::PreparedInstalledScenario;
use clonk_engine::{math, Definition, DefinitionId, SpawnConfig, Vector2};
use clonk_script::Value;

#[test]
fn component_all_uses_the_instance_custom_recipe_and_calling_builder() {
    const CUSTOM_SCRIPT: &str = r#"#strict 2
local seen_builder, seen_instance;

protected func GetCustomComponents(object builder)
{
    seen_builder = builder;
    seen_instance = this();
    return [WOOD, WOOD];
}
"#;
    const BUILDER_SCRIPT: &str = r#"#strict 2
public func Probe(object custom, object mixed, object pure)
{
    return [ComponentAll(custom, WOOD), ComponentAll(custom, METL),
            ComponentAll(mixed, WOOD), ComponentAll(pure, WOOD)];
}
"#;
    const GLOBAL_ONLY_SCRIPT: &str = r#"#strict 2
global func GetCustomComponents(object builder)
{
    return [WOOD, WOOD];
}
"#;

    let mut engine = clonk_engine::Engine::new();
    for (id, name, script) in [
        ("WOOD", "Wood", ""),
        ("METL", "Metal", ""),
        ("CUST", "Custom recipe", CUSTOM_SCRIPT),
        ("PLAI", "Plain recipe", GLOBAL_ONLY_SCRIPT),
        ("BULD", "Builder", BUILDER_SCRIPT),
    ] {
        engine
            .register_script_definition(id, name, script)
            .expect("definition registers");
    }

    let custom = engine
        .spawn_object(
            SpawnConfig::new("CUST")
                .with_components(HashMap::from([(DefinitionId::from("METL"), 1)])),
        )
        .expect("custom-recipe object spawns with a conflicting instance ledger");
    let mixed = engine
        .spawn_object(SpawnConfig::new("PLAI").with_components(HashMap::from([
            (DefinitionId::from("WOOD"), 1),
            (DefinitionId::from("METL"), 1),
        ])))
        .expect("mixed fallback object spawns");
    let pure = engine
        .spawn_object(SpawnConfig::new("PLAI").with_components(HashMap::from([
            (DefinitionId::from("WOOD"), 2),
            (DefinitionId::from("METL"), 0),
        ])))
        .expect("pure fallback object spawns");
    let builder = engine
        .spawn_object(SpawnConfig::new("BULD"))
        .expect("builder spawns");

    let builder_index = engine.find_object_index(builder).expect("builder index");
    assert_eq!(
        engine
            .call_object_function(
                builder_index,
                "Probe",
                vec![
                    Value::Object(custom.as_u64()),
                    Value::Object(mixed.as_u64()),
                    Value::Object(pure.as_u64()),
                ],
            )
            .expect("ComponentAll probe executes"),
        Value::Array(vec![
            Value::Bool(true),
            Value::Bool(false),
            Value::Bool(false),
            Value::Bool(true),
        ])
    );

    let custom_state = engine
        .object_snapshot(custom)
        .expect("custom-recipe object remains");
    assert_eq!(
        custom_state.local_vars.get("seen_builder"),
        Some(&Value::Object(builder.as_u64())),
        "cthr->Obj is passed as the custom-recipe builder"
    );
    assert_eq!(
        custom_state.local_vars.get("seen_instance"),
        Some(&Value::Object(custom.as_u64())),
        "the callback executes with the queried object as its instance"
    );
}

pub(super) fn get_component_definition_branch_uses_custom_recipe_and_builder(
    prepared: &PreparedInstalledScenario,
) {
    // FnGetComponent's idDef branch passes the calling object as builder to
    // C4Def::GetCustomComponents. The shipped dead fish returns two FSHM and
    // one FSHB only for a trapper/Indian builder; indexed reads collapse the
    // adjacent duplicate FSHM entries (C4Script.cpp:2679-2691;
    // C4Def.cpp:1278-1320).
    let mut engine = prepared.instantiate();
    let query = Definition::from_script(
        "QRY1",
        "Component query",
        r#"#strict 2
public func IsTrapper() { return true; }
public func Probe()
{
    return [
        GetComponent(FSHM, 0, 0, DFSH),
        GetComponent(MEAT, 0, 0, DFSH),
        GetComponent(FSHB, 0, 0, DFSH),
        GetComponent(0, 0, 0, DFSH),
        GetComponent(0, 1, 0, DFSH),
        GetComponent(0, 2, 0, DFSH),
        GetComponent(FSHM, 0, 0, FSHM),
        GetComponent(0, 0, 0, FSHM),
        GetComponent(0, 1, 0, FSHM)
    ];
}
"#,
    )
    .expect("component query compiles");
    engine
        .register_definition(query)
        .expect("component query registers");
    let builder = engine
        .spawn_object(SpawnConfig::new("QRY1"))
        .expect("component builder spawns");
    let index = engine
        .find_object_index(builder)
        .expect("component builder index");

    assert_eq!(
        engine
            .call_object_function(index, "Probe", Vec::new())
            .expect("definition component query runs"),
        Value::Array(vec![
            Value::Int(2),
            Value::Int(0),
            Value::Int(1),
            Value::C4Id("FSHM".into()),
            Value::C4Id("FSHB".into()),
            Value::Nil,
            Value::Int(1),
            Value::C4Id("FSHM".into()),
            Value::Nil,
        ])
    );
}

pub(super) fn dead_fish_embowel_uses_the_trappers_custom_components(
    prepared: &PreparedInstalledScenario,
) {
    // FnSplit2Components asks the SOURCE definition for custom components,
    // executing GetCustomComponents on the dead fish with the native-call
    // object (the Trapper) as builder. It then consumes rdir/ydir/xdir via
    // Rnd3 before Random(360) for every requested piece, creates each piece
    // with the fish as creator and owner source, and enters the fish's saved
    // container before finally removing it (src/C4Script.cpp:415-454;
    // src/C4Def.cpp:1266-1355).
    let mut engine = prepared.instantiate();
    let trapper = engine
        .spawn_object(
            SpawnConfig::new("TRPR")
                .with_position(Vector2::new(320, 120))
                .with_owner(3)
                .with_controller(7),
        )
        .expect("the real Western Trapper spawns");
    let fish = engine
        .spawn_object(
            SpawnConfig::new("DFSH")
                .with_position(Vector2::new(320, 120))
                .with_owner(5)
                .with_container(trapper),
        )
        .expect("the real dead fish spawns in the Trapper's inventory");
    let objects_before = engine
        .snapshot()
        .objects
        .into_iter()
        .map(|object| object.id)
        .collect::<HashSet<_>>();
    let mut expected_rng = engine.rng.clone();
    let expected_piece_motion = (0..3)
        .map(|_| {
            let rdir = expected_rng.rnd3();
            let ydir = expected_rng.rnd3();
            let xdir = expected_rng.rnd3();
            let rotation = expected_rng.random(360);
            (xdir, ydir, rdir, rotation)
        })
        .collect::<Vec<_>>();

    let fish_index = engine.find_object_index(fish).expect("dead fish index");
    assert_eq!(
        engine
            .call_object_function(fish_index, "Embowel", vec![Value::Object(trapper.as_u64())],)
            .expect("the shipped DFSH::Embowel callback completes"),
        Value::Int(1)
    );

    assert!(
        engine
            .object_snapshot(fish)
            .is_none_or(|object| !object.status.is_active()),
        "Split2Components removes the source after creating its pieces"
    );
    let mut pieces = engine
        .snapshot()
        .objects
        .into_iter()
        .filter(|object| {
            !objects_before.contains(&object.id)
                && matches!(object.definition_id.as_str(), "FSHM" | "FSHB")
        })
        .collect::<Vec<_>>();
    pieces.sort_by_key(|object| object.id);
    assert_eq!(
        pieces
            .iter()
            .map(|object| object.definition_id.as_str())
            .collect::<Vec<_>>(),
        ["FSHM", "FSHM", "FSHB"],
        "DFSH::GetCustomComponents(TRPR) replaces the default meat recipe"
    );
    assert!(pieces.iter().all(|object| {
        object.container == Some(trapper) && object.owner == 5 && object.controller == 7
    }));

    // Enter copies only the container's x/y motion. The rotateable FSHB
    // keeps Split2Components' third sampled rotation and rdir, while the two
    // non-rotateable FSHM objects discard both in C4Object::Init.
    assert_eq!(pieces[0].rotation, 0);
    assert_eq!(pieces[1].rotation, 0);
    assert_eq!(pieces[2].rotation, expected_piece_motion[2].3);
    assert_eq!(
        pieces[2].rotation_velocity.unwrap_or_default(),
        math::itofix(expected_piece_motion[2].2)
    );
    assert_eq!(
        engine.rng, expected_rng,
        "three Rnd3 reads then Random(360) are consumed per requested piece"
    );
}
