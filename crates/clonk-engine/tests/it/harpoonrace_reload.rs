use crate::support::real_scenario::{join_local_player, load_installed_scenario};
use clonk_engine::{ObjectUpdate, FULL_CON};
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
