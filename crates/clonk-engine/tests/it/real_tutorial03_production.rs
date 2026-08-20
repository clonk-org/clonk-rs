use crate::support::real_scenario::{
    join_local_player_with_preferences, load_raw_content_scenario,
};
use clonk_engine::{Engine, ObjectUpdate};

#[test]
fn tutorial03_real_sawmill_processes_a_pure_wood_tree() {
    // SAWM's real TimerCall=ContentsCheck consumes a contained object only
    // when ComponentAll says every positive component is WOOD, creates one
    // WOOD per component, removes the source and starts Saw
    // (Objects.c4d/Structures.c4d/Sawmill.c4d/Script.c:166-197).
    let scenario = crate::support::TestValueExt::test_value(load_raw_content_scenario(
        "Tutorial.c4f/Tutorial03.c4s",
    ));
    let mut engine = Engine::with_seed(0);
    crate::support::TestValueExt::test_value(scenario.apply(&mut engine));
    join_local_player_with_preferences(&mut engine, "Sawmill production tester", false, false);

    let snapshot = engine.snapshot();
    let sawmill = crate::support::TestValueExt::test_value(
        snapshot
            .objects
            .iter()
            .find(|object| object.definition_id == "SAWM"),
    )
    .id;
    let tree = snapshot
        .objects
        .iter()
        .find(|object| {
            object.definition_id == "TRE2"
                && object.components.get("WOOD") == Some(5)
                && object
                    .components
                    .iter()
                    .all(|(id, count)| id == "WOOD" || count == 0)
        })
        .unwrap_or_else(|| {
            let trees = snapshot
                .objects
                .iter()
                .filter(|object| object.definition_id == "TRE2")
                .map(|object| (object.id, object.components.clone()))
                .collect::<Vec<_>>();
            panic!("Tutorial03 saves a pure five-WOOD TRE2; loaded={trees:?}")
        })
        .id;
    let wood_before = snapshot
        .objects
        .iter()
        .filter(|object| object.definition_id == "WOOD")
        .count();

    crate::support::TestValueExt::test_value(
        engine.apply_object_update(tree, ObjectUpdate::new().with_container(sawmill)),
    );
    assert!(
        engine
            .object_snapshot(sawmill)
            .expect("SAWM survives containment")
            .contents
            .contains(&tree),
        "the real tree must enter SAWM before its TimerCall runs"
    );

    for _ in 0..80 {
        crate::support::TestValueExt::test_value(engine.tick_without_snapshot());
        if engine.object_snapshot(tree).is_none() {
            break;
        }
    }

    let after = engine.snapshot();
    assert!(
        engine.object_snapshot(tree).is_none(),
        "ContentsCheck must remove the consumed TRE2"
    );
    assert_eq!(
        after
            .objects
            .iter()
            .filter(|object| object.definition_id == "WOOD")
            .count(),
        wood_before + 5,
        "TRE2's five real component units must become five WOOD objects"
    );
    assert_eq!(
        engine
            .object_snapshot(sawmill)
            .expect("SAWM survives production")
            .action
            .name,
        "Saw",
        "ContentsCheck starts the real saw animation after conversion"
    );
}
