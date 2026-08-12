//! The Eke Reloaded airbike parks itself by zeroing its own Float physical.
//!
//! `Airbike::ControlDown` (grounded branch) and `Airbike::Entrance` both end
//! with `SetPhysical("Float", 0, 2)` after moving the pilot back to `Walk`
//! (content/EkeReloaded.c4d/Weapons.c4d/Airbike.c4d/Script.c:74-90,452-461).
//! The bike keeps its `Fly` action, whose `Procedure=FLOAT` then reads that
//! physical as its per-axis velocity bound: `lLimit = FIXED100(0)` clamps both
//! `xdir` and `ydir` to exactly zero on the very next `ExecAction`, and
//! DFA_FLOAT adds no gravity, so the bike stops dead where the pilot stepped
//! off (oracle C4Object.cpp:5291-5309).

use crate::support::real_scenario::prepare_installed_scenario;
use clonk_engine::{Engine, JoinPlayerConfig, ObjectId, COM_LEFT};

const SCENARIO: &str = "EkeReloaded.c4f/InterplanetaryCivilwar.c4f/AirbikeFight.c4s";

fn tick(engine: &mut Engine, frames: u32) {
    for _ in 0..frames {
        engine.tick_without_snapshot().expect("the frame executes");
    }
}

/// One whole pixel per frame in `C4Fixed` raw units.
const FIXED_ONE: i32 = 1 << 16;

/// `ObjectSnapshot::fixed_velocity` is a sparse sidecar: it is recorded only
/// while the raw value carries sub-pixel detail the integer `velocity` cannot
/// express, so a whole-pixel speed reconstructs losslessly from `velocity`.
fn raw_velocity(engine: &Engine, object: ObjectId) -> (i32, i32) {
    let snapshot = engine
        .object_snapshot(object)
        .expect("the airbike remains live");
    snapshot
        .fixed_velocity
        .map(|velocity| (velocity.x.val(), velocity.y.val()))
        .unwrap_or((
            snapshot.velocity.x * FIXED_ONE,
            snapshot.velocity.y * FIXED_ONE,
        ))
}

fn float_physical(engine: &Engine, object: ObjectId) -> Option<i32> {
    engine
        .object_snapshot(object)
        .expect("the airbike remains live")
        .temporary_physical
        .map(|physical| physical.float)
}

#[test]
fn airbike_dismount_parks_the_bike_dead_still() {
    let mut engine = prepare_installed_scenario(SCENARIO, 0).instantiate();
    let owner = engine
        .join_player(JoinPlayerConfig {
            name: "Airbike pilot".into(),
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
            control_style: true,
            auto_context_menu: false,
            startup_player_count: 1,
        })
        .expect("the local virtual player joins")
        .initialized()
        .expect("AirbikeFight needs no runtime team selection")
        .number;
    let airbike = engine
        .snapshot()
        .objects
        .into_iter()
        .find(|object| object.definition_id == "AB5B" && object.owner == owner)
        .map(|object| object.id)
        .expect("InitializeClonk creates one airbike per player");

    // Fly left until the bike sits on its `[Physical] Float=200` bound —
    // FIXED100(200) = 2.0 px/frame (oracle C4Object.cpp:5293,5306-5307).
    engine
        .player_in_com(owner, COM_LEFT, 0)
        .expect("the turn reaches the airbike");
    tick(&mut engine, 30);
    assert_eq!(
        raw_velocity(&engine, airbike).0,
        -2 * FIXED_ONE,
        "the airbike reaches its Float physical bound before the dismount"
    );

    // The shipped dismount body, as `Airbike::ControlDown` runs it once the
    // bike is grounded (Airbike.c4d/Script.c:74-90).
    let airbike_index = engine
        .find_object_index(airbike)
        .expect("the airbike has an index");
    engine
        .call_object_function(airbike_index, "Entrance", Vec::new())
        .expect("the shipped dismount runs");
    assert_eq!(
        float_physical(&engine, airbike),
        Some(0),
        "the dismount zeroes the airbike's Float physical"
    );

    tick(&mut engine, 1);
    assert_eq!(
        raw_velocity(&engine, airbike),
        (0, 0),
        "lLimit = FIXED100(0) clamps both axes to a dead stop on the next \
         ExecAction (oracle C4Object.cpp:5306-5307)"
    );

    // `Flying()` restores Float to 200 on its next StartCall and `PilotLost`
    // leaves COMD_Down, so the parked bike sinks straight down from rest —
    // it never resumes the heading it was carrying (Airbike.c4d/Script.c:
    // 296-306,430-438).
    tick(&mut engine, 12);
    assert_eq!(
        raw_velocity(&engine, airbike).0,
        0,
        "nothing gives the parked airbike horizontal speed back"
    );
}
