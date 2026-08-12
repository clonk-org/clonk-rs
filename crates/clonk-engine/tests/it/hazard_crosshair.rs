use crate::support::real_scenario::{join_local_player, load_installed_scenario};
use clonk_engine::{Engine, ObjectId, ObjectUpdate, SpawnConfig, COM_RELEASE_OFFSET, COM_THROW};
use clonk_script::Value;
use std::collections::HashMap;

/// `C4CMD_MoveTo` — the command a left click issues (src/C4Command.h:26).
const CMD_MOVE_TO: i32 = 2;

/// Arm the tutorial's Hazard Clonk with `weapon` and hand it player control.
fn armed_hazard_clonk(engine: &mut Engine, weapon: &str) -> (i32, ObjectId, ObjectId) {
    let owner = join_local_player(engine, "Hazard crosshair parity");
    let clonk = crate::support::TestValueExt::test_value(
        engine
            .snapshot()
            .objects
            .into_iter()
            .find(|object| object.owner == owner && object.definition_id == "HZCK"),
    )
    .id;
    // Hazard's tutorial disables its crew until the scripted introduction
    // completes; enable the fixture so C4Player::InCom routes controls here.
    let mut ready = ObjectUpdate::new().with_action("Walk");
    ready.crew_disabled = Some(false);
    crate::support::TestValueExt::test_value(engine.apply_object_update(clonk, ready));
    crate::support::TestValueExt::test_value(engine.select_crew(owner, [clonk]));
    crate::support::TestValueExt::test_value(engine.set_crew_cursor(owner, Some(clonk)));
    let weapon = engine
        .spawn_object(
            SpawnConfig::new(weapon)
                .with_owner(owner)
                .with_container(clonk),
        )
        .unwrap_or_else(|error| panic!("equip {weapon}: {error}"));
    crate::support::TestValueExt::test_value(
        engine.apply_object_update(clonk, ObjectUpdate::new().with_contents_front(weapon)),
    );
    (owner, clonk, weapon)
}

/// Hazard's crosshair rides on `DFA_ATTACH`, which forces its position to
/// `Target->x + Target->Shape.VtxX[Data & 255] - Shape.VtxX[Data >> 8]`
/// (oracle-src-pinned src/C4Object.cpp:5390-5395). `Initialize` pushes its own
/// vertex 0 out to `CH_Distance` with
/// `SetVertex(0, 1, CH_Distance, 0, 2)` and `SetAngle(90)` rotates the shape,
/// so the reticle orbits the aiming Clonk at 60 pixels — not on top of it
/// (Hazard.c4d/Crew.c4d/HazardClonk.c4d/Crosshair.c4d/Script.c:5,12-18,45-48;
/// Libraries.c4d/Functionalities.c4d/CanAim.c4d/Script.c:202-206).
#[test]
fn hazard_crosshair_orbits_the_aiming_clonk_at_ch_distance() {
    let mut engine = load_installed_scenario("Hazard.c4f/Tutorial.c4s", 0);
    // BZWP reports FM_Aim=1, so the first Throw enters crosshair aiming
    // instead of firing (Items.c4d/Weapons.c4d/Weapon.c4d/Script.c:146-153).
    let (owner, clonk, _weapon) = armed_hazard_clonk(&mut engine, "BZWP");

    crate::support::TestValueExt::test_value(engine.player_in_com(owner, COM_THROW, 0));
    crate::support::TestValueExt::test_value(engine.player_in_com(
        owner,
        COM_THROW + COM_RELEASE_OFFSET,
        0,
    ));

    // DFA_ATTACH forces the position from C4Object::ExecAction, and the
    // UpdateAiming effect moves the Clonk's own attach vertex onto the weapon
    // muzzle. Two frames settle both (the crosshair executes after the Clonk
    // only from the second frame on).
    crate::support::TestValueExt::test_value(engine.tick());
    crate::support::TestValueExt::test_value(engine.tick());

    let snapshot = engine.snapshot();
    let clonk_state = crate::support::TestValueExt::test_value(
        snapshot.objects.iter().find(|object| object.id == clonk),
    );
    let crosshair = crate::support::TestValueExt::test_value(
        snapshot
            .objects
            .iter()
            .find(|object| object.definition_id == "HCRH"),
    );

    assert_eq!(
        crosshair.rotation, 90,
        "InitCrosshair aims straight ahead with SetAngle(90)"
    );
    let attach_vertex =
        crate::support::TestValueExt::test_value(crosshair.vertices.first().copied());
    assert_eq!(
        (attach_vertex.x, attach_vertex.y),
        (-60, 0),
        "C4Shape::Rotate maps the (0, CH_Distance) own vertex onto (-60, 0) \
         at r=90 — the reticle sits a full CH_Distance out, not on the Clonk"
    );

    // WeaponAt(&x, &y, &r) reports the muzzle in 1/1000 pixels, and
    // UpdateVertices writes it onto the Clonk's own vertex 0
    // (CanAim.c4d/Script.c:218-224), so the reticle pivots at the weapon.
    // C4Object.cpp:5390-5395 ForcePosition: Target->x + Target vertex -
    // own vertex.
    let clonk_vertex =
        crate::support::TestValueExt::test_value(clonk_state.vertices.first().copied());
    assert_eq!(
        (crosshair.position.x, crosshair.position.y),
        (
            clonk_state.position.x + clonk_vertex.x - attach_vertex.x,
            clonk_state.position.y + clonk_vertex.y - attach_vertex.y,
        ),
        "DFA_ATTACH forces the crosshair onto the Clonk's attach vertex"
    );
    assert_ne!(
        (clonk_vertex.x, clonk_vertex.y),
        (0, 0),
        "UpdateVertices must have received WeaponAt's muzzle offset through \
         its reference parameters"
    );
}

/// The mouse fire path. A left click sends `C4CMD_MoveTo`, whose
/// `fControl` `SetCommand` calls the crew's `~ControlCommand`
/// (oracle-src-pinned src/C4Object.cpp:3965-3982); Hazard routes that into
/// `DoMouseAiming`, which re-aims and then calls `FireAimWeapon()` ->
/// `this->~Control2Contents("ControlThrow")`
/// (Hazard.c4d/Crew.c4d/HazardClonk.c4d/Script.c:527-552;
/// Libraries.c4d/Functionalities.c4d/CanAim.c4d/Script.c:36-38,119-142).
/// `Bazooka::LaunchRocket` opens with `var x, y; user->WeaponEnd(x, y)`
/// (Items.c4d/Weapons.c4d/Bazooka.c4d/Script.c:96,100-111) — the same
/// cross-definition `&` call as `UpdateVertices` — so no `&` binding means no
/// rocket, which is exactly "I cannot fire".
#[test]
fn hazard_mouse_click_while_aiming_launches_the_bazooka_rocket() {
    let mut engine = load_installed_scenario("Hazard.c4f/Tutorial.c4s", 0);
    let (owner, clonk, weapon) = armed_hazard_clonk(&mut engine, "BZWP");
    // WEPN::CheckAmmo counts the weapon's own MIAM, whose round count lives in
    // the ammo object's numbered local 0 (Hazard.c4d/System.c4g/Ammo.c:20-64).
    crate::support::TestValueExt::test_value(
        engine.spawn_object(
            SpawnConfig::new("MIAM")
                .with_owner(owner)
                .with_container(weapon)
                .with_local_vars(HashMap::from([("__local_0".to_string(), Value::Int(5))])),
        ),
    );

    // First Throw enters crosshair aiming (FM_Aim = 1).
    crate::support::TestValueExt::test_value(engine.player_in_com(owner, COM_THROW, 0));
    crate::support::TestValueExt::test_value(engine.player_in_com(
        owner,
        COM_THROW + COM_RELEASE_OFFSET,
        0,
    ));
    crate::support::TestValueExt::test_value(engine.tick());
    assert!(
        engine
            .snapshot()
            .objects
            .iter()
            .any(|object| object.definition_id == "HCRH"),
        "the Clonk must be aiming before the click fires"
    );

    // Now the click: MoveTo at a point ahead of the Clonk.
    let position = crate::support::TestValueExt::test_value(engine.object_snapshot(clonk)).position;
    crate::support::TestValueExt::test_value(engine.execute_player_command(
        owner,
        CMD_MOVE_TO,
        position.x + 80,
        position.y - 20,
        0,
        0,
        0,
        1,
    ));

    assert!(
        engine
            .snapshot()
            .objects
            .iter()
            .any(|object| object.definition_id == "MISS"),
        "DoMouseAiming must fire the bazooka and launch its MISS rocket"
    );
}
