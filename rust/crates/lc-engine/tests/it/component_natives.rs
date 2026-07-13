use std::collections::HashSet;

use crate::support::real_scenario::load_installed_scenario;
use lc_engine::{math, SpawnConfig, Vector2};
use lc_script::Value;

#[test]
fn dead_fish_embowel_uses_the_trappers_custom_components() {
    // FnSplit2Components asks the SOURCE definition for custom components,
    // executing GetCustomComponents on the dead fish with the native-call
    // object (the Trapper) as builder. It then consumes rdir/ydir/xdir via
    // Rnd3 before Random(360) for every requested piece, creates each piece
    // with the fish as creator and owner source, and enters the fish's saved
    // container before finally removing it (src/C4Script.cpp:415-454;
    // src/C4Def.cpp:1266-1355).
    let mut engine = load_installed_scenario("Western.c4f/Goldrush.c4s", 0);
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
