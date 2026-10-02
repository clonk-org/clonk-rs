//! The upward Team Downhill race must remain playable after its starting
//! platform is destroyed (clonk-org/clonk-rs#1826). This is an intentional
//! content-policy change, not a change to the C++ landscape or movement rules.

use crate::support::real_scenario::{
    join_local_player_on_team, load_installed_scenario, object_with_definition,
    prepare_installed_scenario,
};
use crate::support::EngineTestExt;
use clonk_engine::{Engine, SpawnConfig, Vector2};
use clonk_script::Value;

const SCENARIO: &str = "Collection.c4f/Races.c4f/AbwaertsFalschrum.c4s";
const APPEND: &str = "UpwardRaceSpawn.c";
// The authored starting platform begins at y=3710; JoinPlayer places crew
// two pixels above their eventual standing position (Script.c:88-93).
const START_PLATFORM_Y: i32 = 3710;

fn destroy_starting_platform(engine: &mut Engine) {
    let probe = object_with_definition(engine, "BRPR").unwrap_or_else(|| {
        engine
            .register_script_definition(
                "BRPR",
                "Spawn bridge probe",
                r#"#strict 2
            public func ClearStart()
            {
                return FreeRect(0, 3600, LandscapeWidth(), LandscapeHeight()-3600);
            }
            "#,
            )
            .expect("register terrain probe");
        engine.spawn_test_object(SpawnConfig::new("BRPR"))
    });
    let index = engine.test_object_index(probe);
    engine
        .call_object_function(index, "ClearStart", Vec::new())
        .expect("remove the complete starting platform");
}

#[test]
fn joining_upward_race_restores_footing_after_starting_platform_is_destroyed() {
    let mut engine = load_installed_scenario(SCENARIO, 0);
    destroy_starting_platform(&mut engine);
    let player = join_local_player_on_team(&mut engine, "Climber", 1);
    let clonk = engine.crew_members(player)[0];
    let start = engine.test_object_snapshot(clonk).position;
    assert_eq!(start.y, 3698, "the scenario places the Clonk at its start");
    assert_eq!(
        engine.debug_landscape_material_name(start.x, start.y + 10),
        None
    );

    engine
        .tick_without_snapshot()
        .expect("finish spawn placement");

    // LOAM::BridgeMaterial returns Earth (Objects.c4d/Items.c4d/Materials.c4d/Loam.c4d/Script.c:15-18).
    assert_eq!(
        engine
            .debug_landscape_material_name(start.x, START_PLATFORM_Y)
            .as_deref(),
        Some("Earth"),
        "the arriving player gets solid loam beneath their feet"
    );
    for _ in 0..30 {
        engine.tick_without_snapshot().expect("stand on the bridge");
    }
    let standing = engine.test_object_snapshot(clonk);
    assert!(standing.alive, "the player survives the destroyed start");
    assert_eq!(standing.position.y, START_PLATFORM_Y - 10);
}

#[test]
fn upward_race_relaunch_restores_a_destroyed_bridge_without_regrowing_it_midlife() {
    let mut engine = load_installed_scenario(SCENARIO, 0);
    destroy_starting_platform(&mut engine);
    let player = join_local_player_on_team(&mut engine, "Climber", 1);
    engine
        .tick_without_snapshot()
        .expect("complete the first join");
    let first = engine.crew_members(player)[0];
    let start = engine.test_object_snapshot(first).position;
    destroy_starting_platform(&mut engine);
    engine
        .tick_without_snapshot()
        .expect("bridge is not permanent protection");
    assert!(!engine
        .landscape()
        .expect("race landscape")
        .is_solid_at(start.x, START_PLATFORM_Y));

    // The shipped suicide rule calls Kill, then CLNK::Death broadcasts
    // RelaunchPlayer when the last living crew member dies
    // (Objects.c4d/Crew.c4d/Clonk.c4d/Script.c:533-550).
    let suicide = object_with_definition(&engine, "_SUI").expect("the race has its suicide rule");
    let index = engine.test_object_index(suicide);
    engine
        .call_object_function(index, "Activate", vec![Value::Int(player)])
        .expect("relaunch through the shipped death callbacks");
    let replacement = engine.crew_members(player)[0];
    assert_ne!(replacement, first);
    let respawn = engine.test_object_snapshot(replacement).position;
    assert_eq!(respawn.y, 3698);
    engine
        .tick_without_snapshot()
        .expect("restore respawn footing");
    assert_eq!(
        engine
            .debug_landscape_material_name(respawn.x, START_PLATFORM_Y)
            .as_deref(),
        Some("Earth")
    );
    for _ in 0..30 {
        engine
            .tick_without_snapshot()
            .expect("stand after relaunch");
    }
    assert!(engine.test_object_snapshot(replacement).alive);
    assert_eq!(
        engine.test_object_snapshot(replacement).position.y,
        START_PLATFORM_Y - 10
    );

    // Repeated deaths may choose new x coordinates, but every bridge stays
    // on the original platform row. None may be built above that row.
    for _ in 0..4 {
        destroy_starting_platform(&mut engine);
        let index = engine.test_object_index(suicide);
        engine
            .call_object_function(index, "Activate", vec![Value::Int(player)])
            .expect("repeat a normal relaunch");
        let clonk = engine.crew_members(player)[0];
        let spawn = engine.test_object_snapshot(clonk).position;
        assert_eq!(spawn.y, 3698);
        engine
            .tick_without_snapshot()
            .expect("restore the same platform row");
        assert_eq!(
            engine
                .debug_landscape_material_name(spawn.x, START_PLATFORM_Y)
                .as_deref(),
            Some("Earth")
        );
        for y in spawn.y..START_PLATFORM_Y {
            assert!(
                !engine
                    .landscape()
                    .expect("race landscape")
                    .is_solid_at(spawn.x, y),
                "bridge grew above the platform at y={y}"
            );
        }
    }

    // Checkpoints keep their authored behavior; only the starting platform
    // is repaired, so advancing up the course cannot create higher bridges.
    let sign = engine.spawn_test_object(
        SpawnConfig::new("SGNL")
            .with_owner(player)
            .with_position(Vector2::new(250, 3650)),
    );
    let checkpoint = engine.test_object_snapshot(sign).position;
    destroy_starting_platform(&mut engine);
    let index = engine.test_object_index(suicide);
    engine
        .call_object_function(index, "Activate", vec![Value::Int(player)])
        .expect("relaunch at the checkpoint");
    let replacement = engine.crew_members(player)[0];
    assert_eq!(
        engine.test_object_snapshot(replacement).position,
        checkpoint
    );
    engine
        .tick_without_snapshot()
        .expect("leave checkpoint footing unchanged");
    assert!(!engine
        .landscape()
        .expect("race landscape")
        .is_solid_at(checkpoint.x, checkpoint.y + 10));
}

#[test]
fn upward_race_compatibility_content_keeps_the_authored_missing_ground() {
    let prepared = prepare_installed_scenario(SCENARIO, 0);
    let mut engine = prepared.instantiate_without_system_script(APPEND);
    destroy_starting_platform(&mut engine);
    let player = join_local_player_on_team(&mut engine, "Legacy climber", 1);
    let start = engine
        .test_object_snapshot(engine.crew_members(player)[0])
        .position;
    engine.tick_without_snapshot().expect("legacy spawn tick");
    assert_eq!(
        engine.debug_landscape_material_name(start.x, start.y + 10),
        None
    );
}

#[test]
fn intact_upward_spawn_and_other_races_keep_their_authored_state() {
    for path in [
        SCENARIO,
        "Collection.c4f/Races.c4f/AbwaertsExtremTeam2.c4s",
        "Races.c4f/Caverace.c4s",
    ] {
        let prepared = prepare_installed_scenario(path, 0);
        let mut normal = prepared.instantiate();
        let mut authored = prepared.instantiate_without_system_script(APPEND);
        for engine in [&mut normal, &mut authored] {
            join_local_player_on_team(engine, "Climber", 1);
            for _ in 0..3 {
                engine.tick_without_snapshot().expect("settle the spawn");
            }
        }
        assert!(
            normal.snapshot() == authored.snapshot(),
            "{path}: spawn state changed"
        );
    }
}
