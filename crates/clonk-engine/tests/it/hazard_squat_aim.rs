use crate::support::real_scenario::{join_local_player, load_installed_scenario};
use clonk_engine::{
    landscape::PixelGrid, Landscape, ObjectUpdate, SpawnConfig, Vector2, COM_DOWN,
    COM_RELEASE_OFFSET,
};
use clonk_script::Value;
use std::collections::HashMap;

fn hazard_pixel_landscape() -> Landscape {
    let width = 400_u32;
    let height = 1000_u32;
    let bytes = (0..height)
        .flat_map(|y| (0..width).map(move |x| u8::from(y >= 400 && x <= 52)))
        .collect();
    let mut landscape = Landscape::flat(400, 400);
    landscape.set_world_height(height as i32);
    landscape.set_pixel_grid(PixelGrid::new(
        width,
        height,
        bytes,
        vec![0, 100],
        vec![None, None],
        vec![None, None],
    ));
    landscape
}

#[test]
fn hazard_three_down_presses_enter_squat_aim_with_a_pistol() {
    // Hazard documents this as "Press stop triple to aim": the second Down
    // arms C4Player::LastComDownDouble and the third ordinary ControlDown sees
    // that latch before entering AimSquat (HazardClonk.c4d/Script.c:118-141,
    // 785-807; oracle-src-pinned src/C4Player.cpp:1490-1554).
    let mut engine = load_installed_scenario("Hazard.c4f/Tutorial.c4s", 0);
    let owner = join_local_player(&mut engine, "Hazard squat-aim parity");
    let hazard_clonk = engine
        .snapshot()
        .objects
        .into_iter()
        .find(|object| object.owner == owner && object.definition_id == "HZCK")
        .expect("Hazard joins with an HZCK")
        .id;
    // Hazard's tutorial temporarily disables its crew until the scripted
    // introduction completes. Enable the fixture so C4Player::InCom routes
    // controls to the same HZCK whose ControlDown behavior we are testing.
    let mut ready = ObjectUpdate::new().with_action("Walk");
    ready.crew_disabled = Some(false);
    engine
        .apply_object_update(hazard_clonk, ready)
        .expect("enable the tutorial HZCK for player control");
    engine
        .select_crew(owner, [hazard_clonk])
        .expect("select the tutorial HZCK");
    engine
        .set_crew_cursor(owner, Some(hazard_clonk))
        .expect("make the tutorial HZCK the control cursor");
    assert_eq!(engine.crew_cursor(owner), Some(hazard_clonk));

    let pistol = engine
        .spawn_object(
            SpawnConfig::new("PIWP")
                .with_owner(owner)
                .with_container(hazard_clonk),
        )
        .expect("equip Hazard's aimable pistol");
    engine
        .apply_object_update(
            hazard_clonk,
            ObjectUpdate::new().with_contents_front(pistol),
        )
        .expect("select the pistol");

    for press in 1..=3 {
        engine
            .player_in_com(owner, COM_DOWN, 0)
            .unwrap_or_else(|error| panic!("Down press {press} reaches C4Player::InCom: {error}"));
        engine
            .player_in_com(owner, COM_DOWN + COM_RELEASE_OFFSET, 0)
            .unwrap_or_else(|error| {
                panic!("Down release {press} reaches C4Player::InCom: {error}")
            });
    }

    let aimed = engine
        .object_snapshot(hazard_clonk)
        .expect("the HZCK remains live");
    assert_eq!(
        aimed.action.name, "AimSquat",
        "PIWP's deterministic FM_Aim=0 path must enter Hazard's generic firearm aim"
    );
}

#[test]
fn hazard_pistol_bullet_keeps_its_cpp_velocity_through_the_first_frame() {
    // Pistol::Fire1 launches SHT1 at speed 250, and SHT1's Travel action is
    // DFA_FLOAT with Physical.Float=100000. C++ bounds the axes only to
    // FIXED100(Float), or 1000 px/frame, so its roughly 25 px/frame launch
    // remains bit-exact through ExecAction
    // (Hazard.c4d/Items.c4d/Weapons.c4d/Pistol.c4d/Script.c:122-129;
    // Weapon.c4d/Shot.c4d/DefCore.txt:15-16;
    // oracle-src-pinned src/C4Object.cpp:5291-5310).
    let mut engine = load_installed_scenario("Hazard.c4f/Tutorial.c4s", 0);
    // The loaded fixture places HZCK at y=899. Keep its old narrow solid
    // contact column in a real pixel plane while leaving the shot tunnel
    // clear; the test must not depend on a column-only collision substitute.
    engine.set_landscape(hazard_pixel_landscape());
    let owner = join_local_player(&mut engine, "Hazard bullet parity");
    let hazard_clonk = engine
        .snapshot()
        .objects
        .into_iter()
        .find(|object| object.owner == owner && object.definition_id == "HZCK")
        .expect("Hazard joins with an HZCK")
        .id;
    let mut ready = ObjectUpdate::new().with_action("Walk");
    ready.crew_disabled = Some(false);
    engine
        .apply_object_update(hazard_clonk, ready)
        .expect("enable the tutorial HZCK for player control");
    engine
        .select_crew(owner, [hazard_clonk])
        .expect("select the tutorial HZCK");
    engine
        .set_crew_cursor(owner, Some(hazard_clonk))
        .expect("make the tutorial HZCK the control cursor");

    let pistol = engine
        .spawn_object(
            SpawnConfig::new("PIWP")
                .with_owner(owner)
                .with_container(hazard_clonk),
        )
        .expect("equip Hazard's pistol");
    engine
        .apply_object_update(
            hazard_clonk,
            ObjectUpdate::new().with_contents_front(pistol),
        )
        .expect("select the pistol");
    engine
        .spawn_object(
            SpawnConfig::new("STAM")
                .with_owner(owner)
                .with_container(pistol)
                .with_local_vars(HashMap::from([("__local_0".to_string(), Value::Int(12))])),
        )
        .expect("load the pistol's standard ammunition");

    let pistol_idx = engine.find_object_index(pistol).expect("pistol exists");
    engine
        .call_object_function(
            pistol_idx,
            "SetUser",
            vec![Value::Object(hazard_clonk.as_u64())],
        )
        .expect("bind the pistol to its Hazard Clonk");
    let clonk_idx = engine
        .find_object_index(hazard_clonk)
        .expect("Hazard Clonk exists");
    engine
        .call_object_function(clonk_idx, "StartAiming", Vec::new())
        .expect("enter Hazard crosshair aiming");
    engine.tick().expect("settle the aiming effect");

    let position = engine
        .object_snapshot(hazard_clonk)
        .expect("Hazard Clonk remains live")
        .position;
    let clonk_idx = engine
        .find_object_index(hazard_clonk)
        .expect("Hazard Clonk exists");
    engine
        .call_object_function(
            clonk_idx,
            "DoMouseAiming",
            vec![Value::Int(position.x + 80), Value::Int(position.y - 20)],
        )
        .expect("aim and fire the selected pistol");

    let launched = engine
        .snapshot()
        .objects
        .into_iter()
        .find(|object| object.definition_id == "SHT1")
        .expect("Pistol::Fire1 creates an SHT1 bullet");
    let launch_velocity = launched
        .fixed_velocity
        .expect("SHT1 keeps its raw C4Fixed launch velocity");
    assert_eq!(
        (launch_velocity.x.val(), launch_velocity.y.val()),
        (1_592_524, -393_216),
        "seed 0 must reproduce the pinned C++ 76-degree pistol launch"
    );
    assert!(
        launch_velocity.x.abs() > clonk_engine::math::itofix(12),
        "the Hazard bullet must start above the synthetic 12 px/frame cap"
    );

    engine
        .tick()
        .expect("the bullet's first full frame executes");
    let advanced = engine
        .object_snapshot(launched.id)
        .expect("the clear-landscape bullet remains live");
    assert_eq!(
        advanced.fixed_velocity,
        Some(launch_velocity),
        "DFA_FLOAT and its Traveling callback must not steepen the bullet trajectory"
    );
}

#[test]
fn hazard_pistol_bullet_sweeps_through_an_alien_on_its_first_frame() {
    // HitCheck is definition-commanded (`pCommandTarget == nullptr`,
    // `idCommandTarget == SHT1`), so C++ executes FxHitCheckTimer with no
    // `this` object. Find_OnLine therefore adds the global GetX()/GetY()
    // origin (nil -> zero) to the already-absolute old/new bullet positions,
    // and the first timer sweep intersects this ALN2
    // (Hazard.c4d/Items.c4d/Weapons.c4d/Weapon.c4d/Shot.c4d/Script.c:72,
    // 245-295; planet/System.c4g/FindObject.c:49-50;
    // oracle-src-pinned src/C4Effect.cpp:328-355).
    let mut engine = load_installed_scenario("Hazard.c4f/Tutorial.c4s", 0);
    // The loaded fixture places HZCK at y=899. Keep its old narrow solid
    // contact column in a real pixel plane while leaving the shot tunnel
    // clear; the test must not depend on a column-only collision substitute.
    engine.set_landscape(hazard_pixel_landscape());
    let owner = join_local_player(&mut engine, "Hazard alien-hit parity");
    let hazard_clonk = engine
        .snapshot()
        .objects
        .into_iter()
        .find(|object| object.owner == owner && object.definition_id == "HZCK")
        .expect("Hazard joins with an HZCK")
        .id;
    let mut ready = ObjectUpdate::new().with_action("Walk");
    ready.crew_disabled = Some(false);
    engine
        .apply_object_update(hazard_clonk, ready)
        .expect("enable the tutorial HZCK for player control");
    engine
        .select_crew(owner, [hazard_clonk])
        .expect("select the tutorial HZCK");
    engine
        .set_crew_cursor(owner, Some(hazard_clonk))
        .expect("make the tutorial HZCK the control cursor");

    let pistol = engine
        .spawn_object(
            SpawnConfig::new("PIWP")
                .with_owner(owner)
                .with_container(hazard_clonk),
        )
        .expect("equip Hazard's pistol");
    engine
        .apply_object_update(
            hazard_clonk,
            ObjectUpdate::new().with_contents_front(pistol),
        )
        .expect("select the pistol");
    engine
        .spawn_object(
            SpawnConfig::new("STAM")
                .with_owner(owner)
                .with_container(pistol)
                .with_local_vars(HashMap::from([("__local_0".to_string(), Value::Int(12))])),
        )
        .expect("load the pistol's standard ammunition");

    let pistol_idx = engine.find_object_index(pistol).expect("pistol exists");
    engine
        .call_object_function(
            pistol_idx,
            "SetUser",
            vec![Value::Object(hazard_clonk.as_u64())],
        )
        .expect("bind the pistol to its Hazard Clonk");
    let clonk_idx = engine
        .find_object_index(hazard_clonk)
        .expect("Hazard Clonk exists");
    engine
        .call_object_function(clonk_idx, "StartAiming", Vec::new())
        .expect("enter Hazard crosshair aiming");
    engine.tick().expect("settle the aiming effect");

    let position = engine
        .object_snapshot(hazard_clonk)
        .expect("Hazard Clonk remains live")
        .position;
    let clonk_idx = engine
        .find_object_index(hazard_clonk)
        .expect("Hazard Clonk exists");
    engine
        .call_object_function(
            clonk_idx,
            "DoMouseAiming",
            vec![Value::Int(position.x + 80), Value::Int(position.y - 20)],
        )
        .expect("aim and fire the selected pistol");

    let launched = engine
        .snapshot()
        .objects
        .into_iter()
        .find(|object| object.definition_id == "SHT1")
        .expect("Pistol::Fire1 creates an SHT1 bullet");
    let launch_velocity = launched
        .fixed_velocity
        .expect("SHT1 keeps its raw C4Fixed launch velocity");
    assert_eq!(
        (launch_velocity.x.val(), launch_velocity.y.val()),
        (1_592_524, -393_216),
        "seed 0 pins the C++ 76-degree first shot"
    );
    let alien_position = Vector2::new(launched.position.x + 25, launched.position.y + 2);

    let alien = engine
        .spawn_object(
            SpawnConfig::new("ALN2")
                .with_position(alien_position)
                .with_loaded(true)
                .with_alive(true)
                .with_energy(10_000)
                .with_mobile(false),
        )
        .expect("place a stationary Hazard alien across the first bullet segment");
    let energy_before = engine
        .object_snapshot(alien)
        .expect("the alien starts live")
        .energy;
    assert_eq!(
        energy_before, 10_000,
        "the pinned ALN2 starts at its C++ Physical.Energy"
    );

    engine
        .tick()
        .expect("the bullet and its HitCheck effect execute");

    assert!(
        engine.object_snapshot(launched.id).is_none(),
        "the swept HitCheck must consume the bullet when it crosses the alien"
    );
    assert!(
        engine.object_snapshot(alien).is_none(),
        "the standard pistol's 10,000 exact damage must one-shot ALN2"
    );
}
