use std::env;
use std::path::PathBuf;

use lc_engine::scenario::LegacyDefinitionResolver;
use lc_engine::{Engine, JoinPlayerConfig, Scenario, ScenarioError};
use lc_resources::Group;

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

fn load_tutorial05() -> (Engine, i32) {
    let content = env::var_os("LC_CONTENT_ROOT")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../../content"));
    let path = content.join("Tutorial.c4f/Tutorial05.c4s");
    let resolver = ContentResolver {
        root: content.clone(),
    };
    let scenario = Scenario::load_from_path_with(&path, &resolver)
        .unwrap_or_else(|error| panic!("Tutorial05 loads: {error}"));
    let mut engine = Engine::with_seed(0);
    scenario
        .apply(&mut engine)
        .unwrap_or_else(|error| panic!("Tutorial05 applies: {error}"));
    let joined = engine
        .join_player(JoinPlayerConfig {
            name: "Tutorial 5 route".to_string(),
            player_info_id: 0,
            score: 0,
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
        .unwrap_or_else(|error| panic!("Tutorial05 player joins: {error}"));
    (engine, joined.number)
}

#[test]
fn tutorial05_partial_elevator_starts_with_its_built_component_fraction() {
    // NewObject's initial DoCon calls ComponentConGain
    // (C4Object.cpp:1428-1465, especially :1464; :519-526). At 80% the
    // real ELEV therefore already owns floor(4*80%) WOOD and floor(2*80%)
    // METL; the player only has to deliver the remaining one of each.
    let (engine, _) = load_tutorial05();
    let elevator = engine
        .snapshot()
        .objects
        .into_iter()
        .find(|object| object.definition_id == "ELEV")
        .expect("Tutorial05 creates its elevator construction");
    assert_eq!(elevator.construction, 80_000);
    assert_eq!(elevator.components.get("WOOD"), Some(&3));
    assert_eq!(elevator.components.get("METL"), Some(&1));
}

// InitRules creates the scenario's CNMT object and UpdateRules maps its
// presence to C4RULE_ConstructionNeedsMaterial (C4Game.cpp:4016-4046).
// C4Object::Build then refuses to advance past the component ratio while no
// full-con material is available (C4Object.cpp:1690-1738). Tutorial05 relies
// on that stall before teaching the player to catapult WOOD and METL uphill.
#[test]
fn tutorial05_cnmt_rule_stalls_the_unfed_elevator_at_eighty_percent() {
    let (mut engine, _) = load_tutorial05();
    let elevator = engine
        .snapshot()
        .objects
        .into_iter()
        .find(|object| object.definition_id == "ELEV")
        .expect("Tutorial05 creates its elevator construction");
    assert_eq!(elevator.construction, 80_000);

    // Script1 naturally commands the first CLNK to build. No controls or
    // state injection supply the missing fourth WOOD or second METL.
    for _ in 0..240 {
        engine.tick().expect("Tutorial05 opening frame");
    }

    let stalled = engine
        .object_snapshot(elevator.id)
        .expect("the elevator construction survives");
    assert_eq!(
        stalled.construction, 80_000,
        "CNMT must prevent free construction progress"
    );
}
