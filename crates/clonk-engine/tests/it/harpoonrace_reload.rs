use crate::support::real_scenario::{join_local_player, load_installed_scenario};
use clonk_engine::{
    CNAT_BOTTOM, COM_DOWN, COM_RELEASE_OFFSET, COM_THROW, COM_UP, FULL_CON, ObjectId, ObjectUpdate,
    Vector2,
};
use clonk_script::Value;

fn local(engine: &clonk_engine::Engine, object: clonk_engine::ObjectId, name: &str) -> Value {
    engine
        .object_snapshot(object)
        .unwrap_or_else(|| panic!("object {object:?} remains live"))
        .local_vars
        .get(name)
        .cloned()
        .unwrap_or_else(|| panic!("object {object:?} declares local {name}"))
}

#[test]
fn harpoonrace_player_controls_enter_adjust_and_fire_standing_aim() {
    // Eke's standing aim is HP5B-specific: the first Throw from HarpoonWalk
    // enters HarpoonAim, Up/Down change its phase, and the next Throw fires at
    // that phase (Harpoon.c4d/Script.c:51-87,127-168). C++ delivers those
    // controls through C4Player::InCom and C4Object::CallControl before its
    // procedure fallback (oracle-src-pinned src/C4Player.cpp:1490-1554;
    // src/C4Object.cpp:3321-3339,3395-3438).
    let mut engine = load_installed_scenario(
        "EkeReloaded.c4f/InterplanetaryCivilwar.c4f/HarpoonRace.c4s",
        0,
    );
    let owner = join_local_player(&mut engine, "HarpoonRace standing aim parity");
    let sft = engine
        .crew_cursor(owner)
        .expect("HarpoonRace joins with a selected SFT");

    engine
        .player_in_com(owner, COM_THROW, 0)
        .expect("the first Throw enters standing aim");
    let aimed = engine.object_snapshot(sft).expect("the SFT remains live");
    assert_eq!(aimed.action.name, "HarpoonAim");
    assert_eq!(aimed.action.phase, 5);
    let harpoon = aimed
        .contents
        .iter()
        .copied()
        .find(|&object| {
            engine
                .object_snapshot(object)
                .is_some_and(|snapshot| snapshot.definition_id == "HP5B")
        })
        .expect("HarpoonRace equips HP5B");
    assert_eq!(local(&engine, harpoon, "ammo"), Value::Int(100));

    engine
        .player_in_com(owner, COM_UP, 0)
        .expect("Up adjusts the standing aim");
    assert_eq!(
        engine
            .object_snapshot(sft)
            .expect("the SFT remains live")
            .action
            .phase,
        4
    );
    engine
        .player_in_com(owner, COM_UP + COM_RELEASE_OFFSET, 0)
        .expect("Up release completes");
    engine
        .player_in_com(owner, COM_DOWN, 0)
        .expect("Down adjusts the standing aim");
    assert_eq!(
        engine
            .object_snapshot(sft)
            .expect("the SFT remains live")
            .action
            .phase,
        5
    );

    engine
        .player_in_com(owner, COM_THROW, 0)
        .expect("the second Throw fires the aimed harpoon");
    assert_eq!(local(&engine, harpoon, "ammo"), Value::Int(0));
    let launched_object = |name: &str, expected_definition: &str| {
        let Value::Object(raw_id) = local(&engine, harpoon, name) else {
            panic!("HP5B {name} points at the launched object");
        };
        let id = ObjectId::new(raw_id);
        assert_eq!(
            engine
                .object_snapshot(id)
                .unwrap_or_else(|| panic!("HP5B {name} remains live"))
                .definition_id,
            expected_definition
        );
    };
    launched_object("rope", "RP5B");
    launched_object("arrow", "AW5B");
}

#[test]
fn harpoonrace_lethal_fall_damage_kills_and_relaunches_the_sft() {
    // C++ calls ContactBottom while walking a moving shape into terrain
    // (oracle-src-pinned src/C4Movement.cpp:284-321). SF5B's callback
    // applies lethal FallDamage, and C4Object::DoEnergy immediately runs
    // AssignDeath (SFT.c4d/Script.c:345-381; oracle-src-pinned
    // src/C4Object.cpp:1164-1205,1372-1393). SF5B::Death then asks
    // HarpoonRace::RelaunchPlayer to create and select a replacement SF5B
    // (SFT.c4d/Script.c:702-721; HarpoonRace.c4s/Script.c:62-80).
    let mut engine = load_installed_scenario(
        "EkeReloaded.c4f/InterplanetaryCivilwar.c4f/HarpoonRace.c4s",
        0,
    );
    let owner = join_local_player(&mut engine, "HarpoonRace death parity");
    let original = engine
        .crew_cursor(owner)
        .expect("HarpoonRace joins with a selected SFT");
    let sft_count_before = engine.object_count_for_definition("SF5B");
    let vertices = engine
        .object_snapshot(original)
        .expect("the original SFT is live")
        .vertices;
    let bottom = vertices
        .iter()
        .find(|vertex| vertex.cnat & CNAT_BOTTOM != 0)
        .expect("SF5B has a bottom contact vertex");
    let landing = {
        let landscape = engine
            .landscape()
            .expect("HarpoonRace has its real generated landscape");
        let landscape_width =
            i32::try_from(landscape.width()).expect("landscape width fits an i32");
        (8..landscape_width - 8)
            .find_map(|surface_x| {
                (12..landscape.estimated_height()).find_map(|surface_y| {
                    let center = Vector2::new(surface_x - bottom.x, surface_y - bottom.y - 1);
                    let clear_before = vertices.iter().all(|vertex| {
                        !landscape.is_solid_at(center.x + vertex.x, center.y + vertex.y)
                    });
                    let bottom_hits_after_one_pixel = landscape.is_solid_at(surface_x, surface_y);
                    let other_vertices_clear_after_one_pixel = vertices
                        .iter()
                        .filter(|vertex| vertex.cnat & CNAT_BOTTOM == 0)
                        .all(|vertex| {
                            !landscape.is_solid_at(center.x + vertex.x, center.y + vertex.y + 1)
                        });
                    (clear_before
                        && bottom_hits_after_one_pixel
                        && other_vertices_clear_after_one_pixel)
                        .then_some(center)
                })
            })
            .expect("the real landscape has an open solid landing surface")
    };
    let mut lethal_landing = ObjectUpdate::new()
        .with_position(landing)
        .with_energy(10_000)
        .with_velocity(Vector2::new(0, 100))
        .with_action("HarpoonJump");
    lethal_landing.mobile = Some(true);
    engine
        .apply_object_update(original, lethal_landing)
        .expect("prepare a lethal landing");

    engine
        .tick_without_snapshot()
        .expect("the moving SFT collides with the real landscape");

    let dead = engine
        .object_snapshot(original)
        .expect("C++ retains the dead SFT object");
    assert_eq!(dead.energy, 0);
    assert!(!dead.alive, "zero energy must mark the original SFT dead");
    assert_eq!(dead.action.name, "Dead");

    let replacement = engine
        .crew_cursor(owner)
        .expect("HarpoonRace immediately selects the relaunched SFT");
    assert_ne!(replacement, original);
    assert_eq!(
        engine.object_count_for_definition("SF5B"),
        sft_count_before + 1,
        "the synchronous Death callback must relaunch exactly once"
    );
    assert_eq!(
        engine.crew_members(owner),
        vec![replacement],
        "AssignDeath removes the corpse and leaves exactly one crew member"
    );
    let replacement = engine
        .object_snapshot(replacement)
        .expect("the replacement SFT remains live");
    assert_eq!(replacement.definition_id, "SF5B");
    assert!(replacement.alive);
    assert_eq!(replacement.energy, 70_000);
    assert_eq!(replacement.action.name, "HarpoonWalk");
}

#[test]
fn harpoonrace_automatic_rope_break_reloads_for_a_second_shot() {
    // The shipped chain is intentionally exercised end-to-end:
    // HarpoonRace InitializeClonk gives SF5B an HP5B and selects HarpoonWalk
    // (HarpoonRace.c4s/Script.c:51-59); SF5B::ControlThrow forwards to its
    // first content (SFT.c4d/Script.c:154-169); and HP5B first enters
    // HarpoonAim, then launches AW5B and connects RP5B
    // (Harpoon.c4d/Script.c:63-87,127-156).
    let mut engine = load_installed_scenario(
        "EkeReloaded.c4f/InterplanetaryCivilwar.c4f/HarpoonRace.c4s",
        0,
    );
    let owner = join_local_player(&mut engine, "HarpoonRace reload parity");
    let sft = engine
        .crew_cursor(owner)
        .expect("HarpoonRace joins with a selected SFT");
    assert_eq!(
        engine
            .object_snapshot(sft)
            .expect("the SFT is live")
            .definition_id,
        "SF5B"
    );
    let harpoon = engine
        .object_snapshot(sft)
        .expect("the SFT is live")
        .contents
        .iter()
        .copied()
        .find(|&object| {
            engine
                .object_snapshot(object)
                .is_some_and(|snapshot| snapshot.definition_id == "HP5B")
        })
        .expect("HarpoonRace equips the SFT with HP5B");
    assert_eq!(
        engine
            .object_snapshot(sft)
            .expect("the SFT is live")
            .contents
            .first(),
        Some(&harpoon),
        "the shipped HP5B is the selected first content"
    );

    let sft_index = engine.find_object_index(sft).expect("the SFT has an index");
    assert_eq!(
        engine
            .call_object_function(sft_index, "ControlThrow", Vec::new())
            .expect("the first shipped Throw callback completes"),
        Value::Int(1)
    );
    assert_eq!(
        engine
            .object_snapshot(sft)
            .expect("the SFT remains live")
            .action
            .name,
        "HarpoonAim",
        "the first Throw enters aiming"
    );

    let sft_index = engine.find_object_index(sft).expect("the SFT has an index");
    assert_eq!(
        engine
            .call_object_function(sft_index, "ControlThrow", Vec::new())
            .expect("the firing Throw callback completes"),
        Value::Int(1)
    );
    assert_eq!(local(&engine, harpoon, "ammo"), Value::Int(0));
    let rope = match local(&engine, harpoon, "rope") {
        Value::Object(number) => clonk_engine::ObjectId::new(number),
        value => panic!("firing stores the live RP5B in HP5B.rope, got {value:?}"),
    };
    let rope_snapshot = engine
        .object_snapshot(rope)
        .expect("firing creates the RP5B rope");
    assert_eq!(rope_snapshot.definition_id, "RP5B");
    assert_eq!(rope_snapshot.action.target, Some(harpoon));
    let arrow = rope_snapshot
        .action
        .target2
        .expect("RP5B targets the fired AW5B");
    assert_eq!(
        engine.object_count_for_definition_in_container("AW5B", harpoon),
        0,
        "the first arrow left HP5B when fired"
    );

    // Deterministically make the fired arrow incomplete. C++ DFA_CONNECT
    // treats an endpoint below FullCon as a broken line, calls LineBreak(true),
    // and then AssignRemoval (oracle src/C4Object.cpp:5368-5375).
    // AssignRemoval invokes Destruction (:240-260), so RP5B calls
    // HP5B::BreakRope; BreakRope
    // creates a fresh AW5B and restores ammo to 100
    // (Rope.c4d/Script.c:20-27; Harpoon.c4d/Script.c:324-336).
    engine
        .apply_object_update(arrow, ObjectUpdate::new().with_construction(FULL_CON - 1))
        .expect("the fired arrow becomes incomplete");
    engine
        .tick_without_snapshot()
        .expect("the broken RP5B executes its CONNECT lifecycle");

    assert!(
        engine.object_snapshot(rope).is_none(),
        "the broken RP5B is removed"
    );
    assert_eq!(
        local(&engine, harpoon, "rope"),
        Value::Nil,
        "AssignRemoval clears the dead RP5B reference from HP5B"
    );
    assert_eq!(
        local(&engine, harpoon, "ammo"),
        Value::Int(100),
        "RP5B::Destruction reaches HP5B::BreakRope and reloads anchor ammo"
    );
    assert_eq!(
        engine.object_count_for_definition_in_container("AW5B", harpoon),
        1,
        "BreakRope creates the replacement arrow"
    );

    let sft_index = engine.find_object_index(sft).expect("the SFT has an index");
    assert_eq!(
        engine
            .call_object_function(sft_index, "ControlThrow", Vec::new())
            .expect("the second firing callback completes"),
        Value::Int(1)
    );
    assert_eq!(local(&engine, harpoon, "ammo"), Value::Int(0));
    let second_rope = match local(&engine, harpoon, "rope") {
        Value::Object(number) => clonk_engine::ObjectId::new(number),
        value => panic!("the reloaded Harpoon stores a second RP5B, got {value:?}"),
    };
    assert_ne!(second_rope, rope, "the second shot creates a fresh rope");
    let second_rope = engine
        .object_snapshot(second_rope)
        .expect("the second shot's RP5B is live");
    assert_eq!(second_rope.definition_id, "RP5B");
    assert_eq!(second_rope.action.target, Some(harpoon));
    let second_arrow = second_rope
        .action
        .target2
        .expect("the second RP5B targets a fresh AW5B");
    assert_eq!(
        engine
            .object_snapshot(second_arrow)
            .expect("the second shot's AW5B is live")
            .definition_id,
        "AW5B"
    );
}
