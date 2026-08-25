//! The princess cage in Dragon Rock: unlocking it with the key.
//!
//! Reported as unopenable in clonk-org/clonk-rs#982. The interaction is a
//! grab-then-throw on a shipped `C4D_Structure`, which reaches script through
//! the grab-control overload rather than through the ordinary throw command,
//! so nothing in the generic command tests covers it.

use crate::support::real_scenario::{join_local_player_on_team, load_installed_scenario};
use crate::support::EngineTestExt;
use clonk_engine::{
    Engine, ObjectId, ObjectUpdate, SpawnConfig, COM_DOWN, COM_RELEASE_OFFSET, COM_THROW,
};

/// Dragon Rock with a player, past the difficulty and character menus.
fn dragon_rock_with_a_knight() -> (Engine, i32, ObjectId) {
    let mut engine = load_installed_scenario("Fantasy.c4f/Drachenfels.c4s", 0);
    let owner = join_local_player_on_team(&mut engine, "Dragon Rock cage parity", 1);
    // Normal difficulty, then the initially selected KNIG — the same two
    // shipped menus every other Drachenfels test walks through
    // (Drachenfels.c4s/Script.c:86-128,150-178).
    crate::support::TestValueExt::test_value(engine.player_in_com(owner, COM_THROW, 0));
    crate::support::TestValueExt::test_value(engine.player_in_com(owner, COM_THROW, 0));
    let knight = crate::support::TestValueExt::test_value(engine.crew_cursor(owner));
    (engine, owner, knight)
}

fn shipped_cage(engine: &Engine) -> ObjectId {
    crate::support::TestValueExt::test_value(
        engine
            .snapshot()
            .objects
            .iter()
            .find(|object| object.definition_id == "_CAG" && object.status.is_active())
            .map(|object| object.id),
    )
}

/// Put the knight on the cage and grab it, leaving it in DFA_PUSH.
fn grab_the_cage(engine: &mut Engine, owner: i32, knight: ObjectId, cage: ObjectId) {
    let at = engine.test_object_snapshot(cage).position;
    crate::support::TestValueExt::test_value(
        engine.apply_object_update(
            knight,
            ObjectUpdate::new()
                .with_position(at)
                .with_action("Walk")
                .clear_container(),
        ),
    );
    for _ in 0..5 {
        crate::support::TestValueExt::test_value(engine.tick_without_snapshot());
    }
    assert_ne!(
        engine.test_object_snapshot(cage).ocf & clonk_engine::ocf::GRAB,
        0,
        "`Grab=1` in the cage's DefCore is what makes it a push target"
    );
    // These players join on classic control, where the double Down is what
    // reaches `ObjectComDownDouble` and queues Grab against the object the
    // Clonk stands on (C4ObjectCom.cpp:573-588; C4Player.cpp:1522-1531).
    crate::support::TestValueExt::test_value(engine.player_in_com(owner, COM_DOWN, 0));
    crate::support::TestValueExt::test_value(engine.player_in_com(
        owner,
        COM_DOWN + COM_RELEASE_OFFSET,
        0,
    ));
    crate::support::TestValueExt::test_value(engine.player_in_com(owner, COM_DOWN, 0));
    for _ in 0..80 {
        if engine.object_snapshot(knight).is_some_and(|object| {
            object.action.name == "Push" && object.action.target == Some(cage)
        }) {
            break;
        }
        crate::support::TestValueExt::test_value(engine.tick_without_snapshot());
    }
    let pushing = engine.test_object_snapshot(knight);
    assert_eq!(pushing.action.name, "Push", "the knight must grab the cage");
    assert_eq!(pushing.action.target, Some(cage));
}

/// Throwing while pushing the cage, carrying the key, opens it.
///
/// `_CAG` declares Version 4,9,8, so DFA_PUSH takes the grab-control overload
/// branch and hands COM_Throw to the target's own `ControlThrow` instead of
/// the Clonk's throw (C4Object.cpp:3520-3560). The cage's script then finds
/// `_KEY` in the Clonk, switches to `Open` and drops its solid mask
/// (Fantasy.c4f/Drachenfels.c4s/Cage.c4d/Script.c:7-20).
#[test]
fn a_carried_key_opens_the_shipped_princess_cage() {
    let (mut engine, owner, knight) = dragon_rock_with_a_knight();
    let cage = shipped_cage(&engine);
    assert_ne!(
        engine.test_object_snapshot(cage).action.name,
        "Open",
        "the shipped cage starts closed"
    );

    engine.spawn_test_object(SpawnConfig::new("_KEY").with_container(knight));
    grab_the_cage(&mut engine, owner, knight, cage);

    crate::support::TestValueExt::test_value(engine.player_in_com(owner, COM_THROW, 0));
    assert_eq!(
        engine.test_object_snapshot(cage).action.name,
        "Open",
        "the key must unlock the cage"
    );
}

/// Without the key the cage stays shut.
///
/// The script's early `return` on a missing `_KEY` is what separates a locked
/// cage from an open one; a port that opened it regardless would pass the test
/// above while breaking the puzzle.
#[test]
fn the_cage_stays_shut_without_the_key() {
    let (mut engine, owner, knight) = dragon_rock_with_a_knight();
    let cage = shipped_cage(&engine);
    assert!(
        !engine
            .snapshot()
            .objects
            .iter()
            .any(|object| object.definition_id == "_KEY" && object.container == Some(knight)),
        "this case needs the knight to start without a key"
    );

    grab_the_cage(&mut engine, owner, knight, cage);
    crate::support::TestValueExt::test_value(engine.player_in_com(owner, COM_THROW, 0));

    assert_ne!(
        engine.test_object_snapshot(cage).action.name,
        "Open",
        "an empty-handed Clonk must not unlock the cage"
    );
}
