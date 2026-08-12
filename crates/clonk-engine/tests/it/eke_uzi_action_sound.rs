use crate::support::real_scenario::{join_local_player_on_team, load_installed_scenario};
use clonk_engine::{AudioCommand, ObjectUpdate, SpawnConfig};
use clonk_script::Value;

/// The Uzi's firing noise is not a script `Sound()` call — it is the ActMap
/// `Sound=UZ_Shoot` of its Shoot action. `C4Object::SetAction` starts that
/// entry as an object-attached loop on entry and stops it on leave
/// (oracle-src-pinned src/C4Object.cpp:4149-4152,4186-4190), so a weapon that
/// holds Shoot through `NextAction=Shoot` rattles continuously.
#[test]
fn eke_uzi_shoot_action_loops_its_actmap_sound() {
    let mut engine = load_installed_scenario(
        "EkeReloaded.c4f/InterplanetaryCivilwar.c4f/GoldPlateau.c4s",
        0,
    );
    let owner = join_local_player_on_team(&mut engine, "Eke uzi action sound", 1);
    let clonk = crate::support::TestValueExt::test_value(engine.crew_cursor(owner));
    let uzi = crate::support::TestValueExt::test_value(
        engine.spawn_object(SpawnConfig::new("UZ5B").with_container(clonk)),
    );
    crate::support::TestValueExt::test_value(
        engine.spawn_object(SpawnConfig::new("CA5B").with_container(clonk)),
    );
    // Uzi::Shooting aborts unless the carrier holds an "Uzi*" action and the
    // weapon is Contents.First (Eke Weapons.c4d/Uzi.c4d/Script.c:32-44).
    crate::support::TestValueExt::test_value(
        engine.apply_object_update(clonk, ObjectUpdate::new().with_action("UziWalk")),
    );

    let uzi_index = crate::support::TestValueExt::test_value(engine.find_object_index(uzi));
    assert_eq!(
        engine
            .call_object_function(uzi_index, "Activate", vec![Value::Object(clonk.as_u64())])
            .expect("the shipped reload callback completes"),
        Value::Int(1)
    );
    // Activate parks the weapon in Stop, and ControlThrow refuses to fire out
    // of Stop ("zu schnelles Feuern unterbinden"). Let that action expire.
    for _ in 0..20 {
        crate::support::TestValueExt::test_value(engine.tick());
    }

    let uzi_index = crate::support::TestValueExt::test_value(engine.find_object_index(uzi));
    assert_eq!(
        engine
            .call_object_function(
                uzi_index,
                "ControlThrow",
                vec![Value::Object(clonk.as_u64())],
            )
            .expect("the shipped firing callback completes"),
        Value::Int(1)
    );
    assert_eq!(
        engine
            .object_snapshot(uzi)
            .expect("the UZ5B remains live")
            .action
            .name,
        "Shoot",
        "ControlThrow arms the repeating Shoot action"
    );

    let mut shoot_starts = 0_usize;
    let mut stops = 0_usize;
    for _ in 0..12 {
        let presentation = crate::support::TestValueExt::test_value(engine.tick());
        for event in &presentation.audio {
            match event {
                AudioCommand::PlaySound {
                    name,
                    target,
                    volume,
                    looped,
                    ..
                } if name == "UZ_Shoot" => {
                    assert_eq!(*target, Some(uzi), "the act sound attaches to the weapon");
                    assert_eq!(*volume, 100, "StartSoundEffect passes volume 100");
                    assert!(*looped, "the act sound is started with fLoop set");
                    shoot_starts += 1;
                }
                AudioCommand::StopSound { name, .. } if name == "UZ_Shoot" => stops += 1,
                _ => {}
            }
        }
    }
    assert_eq!(
        shoot_starts, 1,
        "entering Shoot starts UZ_Shoot exactly once; NextAction=Shoot re-enters \
         the same slot and must not restart it"
    );
    assert_eq!(stops, 0, "the weapon never leaves Shoot during this window");
}
