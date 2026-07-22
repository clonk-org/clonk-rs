use std::env;
use std::path::PathBuf;

use clonk_engine::scenario::LegacyDefinitionResolver;
use clonk_engine::{Engine, JoinPlayerConfig, ObjectUpdate, Scenario, ScenarioError};
use clonk_resources::Group;

struct ContentResolver {
    root: PathBuf,
}

impl LegacyDefinitionResolver for ContentResolver {
    fn resolve_definition_groups(
        &self,
        _scenario: &Group,
        identifier: &str,
    ) -> Result<Vec<Group>, ScenarioError> {
        Group::open(self.root.join(identifier.replace('\\', "/")))
            .map(|group| vec![group])
            .map_err(ScenarioError::Resources)
    }
}

fn content_root() -> PathBuf {
    env::var_os("LC_CONTENT_ROOT")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../content"))
}

#[test]
fn tutorial03_real_sawmill_processes_a_pure_wood_tree() {
    // SAWM's real TimerCall=ContentsCheck consumes a contained object only
    // when ComponentAll says every positive component is WOOD, creates one
    // WOOD per component, removes the source and starts Saw
    // (Objects.c4d/Structures.c4d/Sawmill.c4d/Script.c:166-197).
    let content = content_root();
    let resolver = ContentResolver {
        root: content.clone(),
    };
    let scenario = Scenario::load_from_path_with(
        content.join("Tutorial.c4f/Tutorial03.c4s"),
        &resolver,
    )
    .expect("Tutorial03 and its real Objects.c4d load");
    let mut engine = Engine::with_seed(0);
    scenario.apply(&mut engine).expect("Tutorial03 applies");
    engine
        .join_player(JoinPlayerConfig {
            name: "Sawmill production tester".to_string(),
            player_info_id: 0,
            score: 0,
            rounds: 0,
            rounds_won: 0,
            rounds_lost: 0,
            total_playing_time: 0,
            team: None,
            color_dw: 0xff_00_00,
            pref_color: 0,
            pref_position: 0,
            crew: Vec::new(),
            control_style: false,
            auto_context_menu: false,
            startup_player_count: 1,
        })
        .expect("Tutorial03 player joins and places ready buildings");

    let snapshot = engine.snapshot();
    let sawmill = snapshot
        .objects
        .iter()
        .find(|object| object.definition_id == "SAWM")
        .expect("Tutorial03 places its ready SAWM")
        .id;
    let tree = snapshot
        .objects
        .iter()
        .find(|object| {
            object.definition_id == "TRE2"
                && object.components.get("WOOD").copied() == Some(5)
                && object
                    .components
                    .iter()
                    .all(|(id, count)| id == "WOOD" || *count == 0)
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

    engine
        .apply_object_update(tree, ObjectUpdate::new().with_container(sawmill))
        .expect("put the real chopped-tree milestone into SAWM");
    assert!(
        engine
            .object_snapshot(sawmill)
            .expect("SAWM survives containment")
            .contents
            .contains(&tree),
        "the real tree must enter SAWM before its TimerCall runs"
    );

    for _ in 0..80 {
        engine.tick_without_snapshot().expect("real SAWM production frame");
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
