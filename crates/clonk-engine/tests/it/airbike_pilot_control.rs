//! Steering an Eke Reloaded airbike from the seat, against the C++ oracle.
//!
//! Every airbike input arrives through the script override: the seated SFT is
//! `Procedure=ATTACH` (SFT.c4d/ActMap.txt:518-526) and neither
//! `C4Object::DirectCom`'s procedure switch nor `AutoStopDirectCom` has a
//! DFA_ATTACH arm (oracle C4Object.cpp:3419-3570,3653-3752), so
//! `CallControl`'s object script override (C4Object.cpp:3399-3403) is the only
//! thing that can move the bike. SFT::ControlLeft forwards to the bike through
//! `Control2Airbike` (SFT.c4d/Script.c:40-59,279-286).

use crate::support::real_scenario::prepare_installed_scenario;
use clonk_engine::{
    CommandDirection, Engine, JoinPlayerConfig, ObjectId, COM_DOUBLE, COM_LEFT, COM_RELEASE_OFFSET,
    COM_RIGHT, COM_UP,
};

const SCENARIO: &str = "EkeReloaded.c4f/InterplanetaryCivilwar.c4f/AirbikeFight.c4s";

/// One whole pixel per frame in `C4Fixed` raw units.
const FIXED_ONE: i32 = 1 << 16;
/// `FloatAccel = FIXED100(10)` (oracle C4Movement.cpp:33). `FIXED100` divides
/// in integer arithmetic, so this is 6553 raw, not 6553.6.
const FLOAT_ACCEL_RAW: i32 = 10 * FIXED_ONE / 100;

fn join_pilot(engine: &mut Engine, auto_stop: bool) -> i32 {
    engine
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
            control_style: auto_stop,
            auto_context_menu: false,
            startup_player_count: 1,
        })
        .expect("the local virtual player joins")
        .initialized()
        .expect("AirbikeFight needs no runtime team selection")
        .number
}

fn tick(engine: &mut Engine, frames: u32) {
    for _ in 0..frames {
        engine.tick_without_snapshot().expect("the frame executes");
    }
}

fn action_name(engine: &Engine, object: ObjectId) -> String {
    engine
        .object_snapshot(object)
        .expect("the object remains live")
        .action
        .name
}

fn com_dir(engine: &Engine, object: ObjectId) -> CommandDirection {
    engine
        .object_snapshot(object)
        .expect("the object remains live")
        .command_direction
}

/// `ObjectSnapshot::fixed_velocity` is a sparse sidecar: it is recorded only
/// while the raw value carries sub-pixel detail the integer `velocity` cannot
/// express, so a whole-pixel speed reconstructs losslessly from `velocity`.
fn raw_velocity(engine: &Engine, object: ObjectId) -> (i32, i32) {
    let snapshot = engine
        .object_snapshot(object)
        .expect("the object remains live");
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
        .expect("the object remains live")
        .temporary_physical
        .map(|physical| physical.float)
}

/// AirbikeFight seats every joined SFT on its own airbike and points that bike
/// up (InterplanetaryCivilwar.c4f/AirbikeFight.c4s/Script.c:28-63).
fn seated(engine: &mut Engine, auto_stop: bool) -> (i32, ObjectId, ObjectId) {
    let owner = join_pilot(engine, auto_stop);
    let sft = engine
        .crew_cursor(owner)
        .expect("AirbikeFight joins with a selected SFT");
    assert_eq!(
        action_name(engine, sft),
        "AirbikeFly",
        "InitializeClonk seats the fresh SFT on its airbike"
    );
    let airbike = engine
        .snapshot()
        .objects
        .into_iter()
        .find(|object| object.definition_id == "AB5B" && object.owner == owner)
        .map(|object| object.id)
        .expect("InitializeClonk creates one airbike per player");
    (owner, sft, airbike)
}

/// DFA_FLOAT steering: each ComDir adds `FloatAccel` = FIXED100(10) to one
/// axis and both axes clamp to `lLimit = FIXED100(Float)`, with no
/// deceleration case and no gravity (oracle C4Object.cpp:5291-5309,
/// C4Movement.cpp:33). The airbike ships `[Physical] Float=200`, so its bound
/// is 2.0 px/frame and it takes 20 frames of acceleration to reach it.
#[test]
fn airbike_steering_accelerates_at_float_accel_up_to_the_float_physical() {
    let prepared = prepare_installed_scenario(SCENARIO, 0);
    for auto_stop in [false, true] {
        let mut engine = prepared.instantiate();
        let (owner, _sft, airbike) = seated(&mut engine, auto_stop);
        assert_eq!(
            float_physical(&engine, airbike),
            Some(200),
            "Flying() holds the Float physical at its 200 floor \
             (Airbike.c4d/Script.c:296-306)"
        );

        engine
            .player_in_com(owner, COM_LEFT, 0)
            .expect("the turn reaches the airbike");
        assert_eq!(
            com_dir(&engine, airbike),
            CommandDirection::Left,
            "Airbike::ControlLeft sets COMD_Left (auto_stop={auto_stop})"
        );
        // The bike enters this test carrying the scenario's COMD_Up momentum,
        // so pin the axis the turn owns.
        let start = raw_velocity(&engine, airbike).0;
        tick(&mut engine, 5);
        assert_eq!(
            raw_velocity(&engine, airbike).0 - start,
            -5 * FLOAT_ACCEL_RAW,
            "five frames of FloatAccel (auto_stop={auto_stop})"
        );

        tick(&mut engine, 40);
        assert_eq!(
            raw_velocity(&engine, airbike).0,
            -2 * FIXED_ONE,
            "COMD_Left saturates at -FIXED100(200) (auto_stop={auto_stop})"
        );

        // COMD_Stop has no arm in the DFA_FLOAT switch, so a steered airbike
        // coasts: nothing decelerates it and no friction applies.
        engine
            .player_in_com(owner, COM_LEFT + COM_RELEASE_OFFSET, 0)
            .expect("releasing the turn key succeeds");
        tick(&mut engine, 20);
        assert_eq!(
            raw_velocity(&engine, airbike).0,
            -2 * FIXED_ONE,
            "releasing the key never brakes a DFA_FLOAT object \
             (auto_stop={auto_stop})"
        );

        // The opposite turn is the only brake, and it pays the same
        // FloatAccel per frame that built the speed up.
        engine
            .player_in_com(owner, COM_RIGHT, 0)
            .expect("the reverse turn reaches the airbike");
        tick(&mut engine, 10);
        assert_eq!(
            raw_velocity(&engine, airbike).0,
            -2 * FIXED_ONE + 10 * FLOAT_ACCEL_RAW,
            "the reverse turn is the only brake a DFA_FLOAT object has \
             (auto_stop={auto_stop})"
        );
    }
}

/// `Airbike::ControlLeftDouble` trades the Float physical up to 800 and swaps
/// the action to `Hyperfly` (Airbike.c4d/Script.c:33-42). Leaving Hyperfly
/// runs `Flying()`, which walks the physical back down by 50 per StartCall;
/// `Fly` is `Length=1, Delay=3`, and DFA_FLOAT never touches `iPhaseAdvance`
/// (oracle C4Object.cpp:5291-5309,5463-5487), so that is one step per three
/// frames down to the 200 floor.
#[test]
fn airbike_hyperfly_boost_decays_one_float_step_every_three_frames() {
    let prepared = prepare_installed_scenario(SCENARIO, 0);
    for auto_stop in [false, true] {
        let mut engine = prepared.instantiate();
        let (owner, _sft, airbike) = seated(&mut engine, auto_stop);
        tick(&mut engine, 3);

        // `C4Player::InCom` raises the repeated com to COM_Left | COM_Double
        // (oracle C4Player.cpp:1532-1533).
        engine
            .player_in_com(owner, COM_LEFT, 0)
            .expect("the first turn reaches the airbike");
        engine
            .player_in_com(owner, COM_LEFT + COM_RELEASE_OFFSET, 0)
            .expect("releasing the turn key succeeds");
        engine
            .player_in_com(owner, COM_LEFT, 0)
            .expect("the second turn reaches the airbike");
        assert_eq!(
            action_name(&engine, airbike),
            "Hyperfly",
            "the double tap engages the boost (auto_stop={auto_stop})"
        );
        assert_eq!(
            float_physical(&engine, airbike),
            Some(800),
            "Hyperfly raises the Float physical to 800 (auto_stop={auto_stop})"
        );

        // Hyperflying() never decays it, and `ControlUp` only writes ComDir.
        // Only a left/right turn forces `SetAction("Fly")` and with it the
        // `Flying()` StartCall (Airbike.c4d/Script.c:21-31,44-58,64-71).
        tick(&mut engine, 12);
        engine
            .player_in_com(owner, COM_UP, 0)
            .expect("the climb reaches the airbike");
        assert_eq!(
            float_physical(&engine, airbike),
            Some(800),
            "Hyperflying() holds the boost (auto_stop={auto_stop})"
        );

        engine
            .player_in_com(owner, COM_RIGHT, 0)
            .expect("steering out of Hyperfly succeeds");
        for step in 0..12 {
            assert_eq!(
                float_physical(&engine, airbike),
                Some(750 - step * 50),
                "one 50-point Flying() step per Delay=3 frames \
                 (step {step}, auto_stop={auto_stop})"
            );
            tick(&mut engine, 3);
        }
        assert_eq!(
            float_physical(&engine, airbike),
            Some(200),
            "the decay stops at the 200 floor (auto_stop={auto_stop})"
        );
    }
}

/// Classic and Jump'n'Run must steer identically from the seat. The two styles
/// diverge only in `C4Object::DirectCom`'s tail — the per-procedure switch
/// versus `AutoStopDirectCom` — and neither has a DFA_ATTACH arm, so a com the
/// SFT chain consumes never reaches either, and a com it does not consume
/// reaches nothing.
#[test]
fn airbike_steering_is_identical_under_both_control_styles() {
    let prepared = prepare_installed_scenario(SCENARIO, 0);
    let mut traces = Vec::new();
    for auto_stop in [false, true] {
        let mut engine = prepared.instantiate();
        let (owner, sft, airbike) = seated(&mut engine, auto_stop);
        let mut trace = Vec::new();
        for com in [COM_LEFT, COM_UP, COM_RIGHT, COM_LEFT | COM_DOUBLE] {
            engine
                .player_in_com(owner, com, 0)
                .expect("the com reaches the chain");
            tick(&mut engine, 7);
            trace.push((
                action_name(&engine, airbike),
                com_dir(&engine, airbike),
                raw_velocity(&engine, airbike),
                action_name(&engine, sft),
                com_dir(&engine, sft),
            ));
        }
        traces.push(trace);
    }
    assert_eq!(
        traces[0], traces[1],
        "the seated pilot is DFA_ATTACH, which neither control style routes"
    );
    assert!(
        traces[0]
            .iter()
            .all(|(_, _, _, pilot_action, pilot_com_dir)| {
                pilot_action == "AirbikeFly" && *pilot_com_dir == CommandDirection::Stop
            }),
        "steering the airbike must never walk its pilot: {:?}",
        traces[0]
    );
}
