//! `planet/System.c4g/EkeAirbikeSteering.c` — the port's hold-to-steer airbike.
//!
//! A deliberate divergence from the oracle, so every test here has an A/B twin
//! in `airbike_pilot_control`, which runs the same scenario with the append
//! removed and pins what LegacyClonk does.

use crate::support::real_scenario::prepare_installed_scenario;
use clonk_engine::{
    CommandDirection, Engine, JoinPlayerConfig, ObjectId, Vector2, COM_DIG, COM_DOUBLE, COM_DOWN,
    COM_LEFT, COM_RELEASE_OFFSET, COM_RIGHT, COM_SPECIAL2, COM_THROW, COM_UP, COM_WHEEL_DOWN,
};

const SCENARIO: &str = "EkeReloaded.c4f/InterplanetaryCivilwar.c4f/AirbikeFight.c4s";
const STEERING_APPEND: &str = "EkeAirbikeSteering.c";

/// One whole pixel per frame in `C4Fixed` raw units.
const FIXED_ONE: i32 = 1 << 16;
/// `FloatAccel = FIXED100(10)` (oracle C4Movement.cpp:33). `FIXED100` divides
/// in integer arithmetic, so this is 6553 raw, not 6553.6.
const FLOAT_ACCEL_RAW: i32 = 10 * FIXED_ONE / 100;

fn join_pilot(engine: &mut Engine, auto_stop: bool) -> i32 {
    engine
        .join_player(JoinPlayerConfig {
            control_style: auto_stop,
            ..crate::support::join_player_config("Airbike pilot")
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

fn float_physical(engine: &Engine, object: ObjectId) -> Option<i32> {
    engine
        .object_snapshot(object)
        .expect("the object remains live")
        .temporary_physical
        .map(|physical| physical.float)
}

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

struct Ride {
    owner: i32,
    airbike: ObjectId,
}

fn seated(engine: &mut Engine, auto_stop: bool) -> Ride {
    let owner = join_pilot(engine, auto_stop);
    let airbike = engine
        .snapshot()
        .objects
        .into_iter()
        .find(|object| object.definition_id == "AB5B" && object.owner == owner)
        .map(|object| object.id)
        .expect("InitializeClonk creates one airbike per player");
    // AirbikeFight commands the fresh bike upward
    // (InterplanetaryCivilwar.c4f/AirbikeFight.c4s/Script.c:57-61), which the
    // glide adopts rather than cancels, so it really is climbing. Take the
    // wheel and settle it before each test so the axis under test starts from
    // rest — and prove it by position, not by a sampled velocity, which reads
    // zero for any drift below one pixel per frame.
    engine
        .player_in_com(owner, COM_DOWN, 0)
        .expect("the pilot takes the wheel");
    engine
        .player_in_com(owner, COM_DOWN + COM_RELEASE_OFFSET, 0)
        .expect("and lets go again");
    tick(engine, 90);
    let settled = engine
        .object_snapshot(airbike)
        .expect("the airbike remains live")
        .position;
    tick(engine, 60);
    assert_eq!(
        engine
            .object_snapshot(airbike)
            .expect("the airbike remains live")
            .position,
        settled,
        "an unsteered airbike is at rest, not creeping below one pixel a frame"
    );
    Ride { owner, airbike }
}

/// The port flies the airbike twice as fast as the shipped bike. `Flying()`
/// floors the Float physical at 400 rather than 200
/// (Airbike.c4d/Script.c:303-314), and DFA_FLOAT clamps each axis to
/// `FIXED100(Float)` (oracle C4Object.cpp:5291-5309), so a held direction
/// accelerates to 4.0 px/frame instead of 2.0. `FloatAccel` is untouched, so
/// reaching the raised bound takes twice as long.
#[test]
fn the_airbike_cruises_at_twice_the_shipped_float_bound() {
    let prepared = prepare_installed_scenario(SCENARIO, 0);
    for auto_stop in [false, true] {
        let mut engine = prepared.instantiate();
        let ride = seated(&mut engine, auto_stop);
        assert_eq!(
            float_physical(&engine, ride.airbike),
            Some(400),
            "Flying() holds the raised cruise floor (auto_stop={auto_stop})"
        );

        engine
            .player_in_com(ride.owner, COM_LEFT, 0)
            .expect("the turn reaches the airbike");
        tick(&mut engine, 39);
        assert_eq!(
            raw_velocity(&engine, ride.airbike).0,
            -39 * FLOAT_ACCEL_RAW,
            "the ramp is still one FloatAccel step a frame, and 39 of them do \
             not yet reach the raised bound (auto_stop={auto_stop})"
        );

        tick(&mut engine, 40);
        assert_eq!(
            raw_velocity(&engine, ride.airbike).0,
            -4 * FIXED_ONE,
            "the held direction saturates at FIXED100(400) (auto_stop={auto_stop})"
        );
    }
}

/// Releasing the key stops the acceleration and the glide bleeds the momentum
/// off at exactly the rate `FloatAccel` built it up.
///
/// Both control styles: Jump'n'Run additionally re-syncs through
/// `ControlUpdate`, but classic has only the press/release pair, so it is the
/// style that proves the held-direction bookkeeping stands on its own.
#[test]
fn releasing_the_steering_key_brings_the_airbike_to_a_stop() {
    let prepared = prepare_installed_scenario(SCENARIO, 0);
    for auto_stop in [false, true] {
        let mut engine = prepared.instantiate();
        let ride = seated(&mut engine, auto_stop);

        engine
            .player_in_com(ride.owner, COM_LEFT, 0)
            .expect("the turn reaches the airbike");
        tick(&mut engine, 41);
        assert_eq!(
            raw_velocity(&engine, ride.airbike).0,
            -4 * FIXED_ONE,
            "holding the key still saturates at the Float physical bound \
             (auto_stop={auto_stop})"
        );

        engine
            .player_in_com(ride.owner, COM_LEFT + COM_RELEASE_OFFSET, 0)
            .expect("releasing the turn key succeeds");
        assert_eq!(
            com_dir(&engine, ride.airbike),
            CommandDirection::Stop,
            "the release stops asking the engine to accelerate \
             (auto_stop={auto_stop})"
        );

        tick(&mut engine, 40);
        assert_eq!(
            raw_velocity(&engine, ride.airbike),
            (0, 0),
            "the glide brakes the airbike to a full stop (auto_stop={auto_stop})"
        );
    }
}

/// Two held keys fly the diagonal between them; releasing one leaves the
/// other. The shipped script can only ever name a pure axis.
#[test]
fn holding_two_steering_keys_flies_the_diagonal() {
    let mut engine = prepare_installed_scenario(SCENARIO, 0).instantiate();
    let ride = seated(&mut engine, true);

    engine
        .player_in_com(ride.owner, COM_LEFT, 0)
        .expect("the turn reaches the airbike");
    engine
        .player_in_com(ride.owner, COM_UP, 0)
        .expect("the climb reaches the airbike");
    assert_eq!(
        com_dir(&engine, ride.airbike),
        CommandDirection::UpLeft,
        "both held keys combine into one diagonal ComDir"
    );

    tick(&mut engine, 41);
    let (x, y) = raw_velocity(&engine, ride.airbike);
    assert_eq!(
        (x, y),
        (-4 * FIXED_ONE, -4 * FIXED_ONE),
        "both axes accelerate to the Float physical bound"
    );

    engine
        .player_in_com(ride.owner, COM_UP + COM_RELEASE_OFFSET, 0)
        .expect("releasing the climb succeeds");
    assert_eq!(
        com_dir(&engine, ride.airbike),
        CommandDirection::Left,
        "releasing one axis leaves the other steering"
    );

    tick(&mut engine, 40);
    let (x, y) = raw_velocity(&engine, ride.airbike);
    assert_eq!(
        (x, y),
        (-4 * FIXED_ONE, 0),
        "the released axis brakes while the held one holds its bound"
    );
}

/// Rolling one direction onto its opposite and letting go of the newer key
/// leaves the older one steering.
///
/// The held state has to be a key set for this: a composed ComDir has nowhere
/// to record that the first key is still down. Classic control is the arm that
/// proves it, because Jump'n'Run would paper over a wrong answer on the next
/// `ControlUpdate` (oracle C4Object.cpp:3321-3339, AutoStopControl only).
#[test]
fn releasing_the_newer_of_two_opposite_keys_keeps_the_older_one_steering() {
    let prepared = prepare_installed_scenario(SCENARIO, 0);
    for auto_stop in [false, true] {
        let mut engine = prepared.instantiate();
        let ride = seated(&mut engine, auto_stop);

        engine
            .player_in_com(ride.owner, COM_LEFT, 0)
            .expect("the turn reaches the airbike");
        // Roll onto the opposite key without letting the first one go.
        engine
            .player_in_com(ride.owner, COM_RIGHT, 0)
            .expect("the opposite turn reaches the airbike");
        assert_eq!(
            com_dir(&engine, ride.airbike),
            CommandDirection::Stop,
            "two opposite keys cancel, exactly as Coms2ComDir resolves them \
             (auto_stop={auto_stop})"
        );

        engine
            .player_in_com(ride.owner, COM_RIGHT + COM_RELEASE_OFFSET, 0)
            .expect("releasing the newer key succeeds");
        assert_eq!(
            com_dir(&engine, ride.airbike),
            CommandDirection::Left,
            "the key still held goes back to steering (auto_stop={auto_stop})"
        );

        tick(&mut engine, 41);
        assert_eq!(
            raw_velocity(&engine, ride.airbike).0,
            -4 * FIXED_ONE,
            "and it drives the bike, rather than the brake stopping it \
             (auto_stop={auto_stop})"
        );
    }
}

/// Turning no longer cancels a burst. `ControlThrow` starts `Shoot`, whose
/// StartCall fires and decrements ammo every `Delay=2` frames
/// (Airbike.c4d/ActMap.txt, Script.c:371-395).
#[test]
fn steering_does_not_cancel_the_airbike_gun() {
    let mut engine = prepare_installed_scenario(SCENARIO, 0).instantiate();
    let ride = seated(&mut engine, true);

    engine
        .player_in_com(ride.owner, COM_THROW, 0)
        .expect("the trigger reaches the airbike");
    assert_eq!(
        action_name(&engine, ride.airbike),
        "Shoot",
        "ControlThrow starts the burst"
    );
    tick(&mut engine, 6);
    let fired = engine
        .object_snapshot(ride.airbike)
        .expect("the airbike remains live")
        .local_vars
        .get("ammo")
        .cloned();

    engine
        .player_in_com(ride.owner, COM_LEFT, 0)
        .expect("the turn reaches the airbike");
    assert_eq!(
        action_name(&engine, ride.airbike),
        "Shoot",
        "steering must not drop the Shoot action"
    );
    assert_eq!(
        com_dir(&engine, ride.airbike),
        CommandDirection::Left,
        "the bike still turns while firing"
    );

    tick(&mut engine, 6);
    assert_ne!(
        engine
            .object_snapshot(ride.airbike)
            .expect("the airbike remains live")
            .local_vars
            .get("ammo")
            .cloned(),
        fired,
        "the burst keeps consuming ammo through the turn"
    );
}

/// The Hyperfly boost keeps its shipped shape and becomes hold-to-dash: the
/// double tap raises the Float physical to 800 so the held direction can climb
/// past the ordinary 2.0 bound, releasing brakes, and a turn ends it
/// (Airbike.c4d/Script.c:33-42,44-58).
#[test]
fn the_hyperfly_boost_is_held_rather_than_latched() {
    let mut engine = prepare_installed_scenario(SCENARIO, 0).instantiate();
    let ride = seated(&mut engine, true);

    engine
        .player_in_com(ride.owner, COM_LEFT, 0)
        .expect("the first turn reaches the airbike");
    engine
        .player_in_com(ride.owner, COM_LEFT + COM_RELEASE_OFFSET, 0)
        .expect("releasing the turn key succeeds");
    engine
        .player_in_com(ride.owner, COM_LEFT, 0)
        .expect("the second turn reaches the airbike");
    assert_eq!(
        action_name(&engine, ride.airbike),
        "Hyperfly",
        "the double tap still engages the boost"
    );
    assert_eq!(
        com_dir(&engine, ride.airbike),
        CommandDirection::Left,
        "the double tap registers the key it was pressed with"
    );

    // The boosted bound is FIXED100(1600) = 16.0 px/frame, so holding pushes
    // the bike past the 4.0 the un-boosted physical allows.
    tick(&mut engine, 45);
    assert!(
        raw_velocity(&engine, ride.airbike).0 < -4 * FIXED_ONE,
        "the held dash climbs past the ordinary bound: {:?}",
        raw_velocity(&engine, ride.airbike)
    );

    engine
        .player_in_com(ride.owner, COM_LEFT + COM_RELEASE_OFFSET, 0)
        .expect("releasing the turn key succeeds");
    assert_eq!(
        action_name(&engine, ride.airbike),
        "Fly",
        "letting go ends the dash, so the Float ladder can decay again"
    );
    tick(&mut engine, 120);
    assert_eq!(
        raw_velocity(&engine, ride.airbike),
        (0, 0),
        "releasing brakes out of the dash like any other heading"
    );
    assert_eq!(
        float_physical(&engine, ride.airbike),
        Some(400),
        "the 4x bound is not left latched on a bike at rest"
    );

    // Climbing from rest must not inherit a boost nobody paid for.
    engine
        .player_in_com(ride.owner, COM_UP, 0)
        .expect("the climb reaches the airbike");
    tick(&mut engine, 60);
    assert_eq!(
        raw_velocity(&engine, ride.airbike).1,
        -4 * FIXED_ONE,
        "an unboosted climb saturates at the ordinary Float physical bound"
    );
}

/// The dash bound doubles along with the cruise bound: the double tap raises
/// the Float physical to 1600 rather than the shipped 800
/// (Airbike.c4d/Script.c:33-42), i.e. 16.0 px/frame. The shipped 50-point
/// decay step is unchanged, so leaving the dash walks the physical back to the
/// raised 400 floor instead of 200, taking twice as many steps to get there
/// (Script.c:303-314).
#[test]
fn the_hyperfly_dash_doubles_the_shipped_boost_bound() {
    let mut engine = prepare_installed_scenario(SCENARIO, 0).instantiate();
    let ride = seated(&mut engine, true);

    engine
        .player_in_com(ride.owner, COM_LEFT, 0)
        .expect("the first turn reaches the airbike");
    engine
        .player_in_com(ride.owner, COM_LEFT + COM_RELEASE_OFFSET, 0)
        .expect("releasing the turn key succeeds");
    engine
        .player_in_com(ride.owner, COM_LEFT, 0)
        .expect("the second turn reaches the airbike");
    assert_eq!(
        action_name(&engine, ride.airbike),
        "Hyperfly",
        "the double tap engages the boost"
    );
    assert_eq!(
        float_physical(&engine, ride.airbike),
        Some(1600),
        "the dash raises the Float physical to twice the shipped boost"
    );

    // One FloatAccel step a frame, so the raised dash bound is 161 frames out
    // rather than the 81 the shipped 800 needed.
    // Saturating a 16.0 px/frame dash takes 161 frames and crosses more than
    // 1200 pixels, while AirbikeFight's landscape is under 400 wide and
    // `BorderBound=7` parks the bike on the edge long before that. Fly it on
    // the spot: the teleport writes position only, so the dash keeps
    // integrating across it.
    let start = engine
        .object_snapshot(ride.airbike)
        .expect("the airbike remains live")
        .position;
    for _ in 0..18 {
        tick(&mut engine, 10);
        let index = engine
            .find_object_index(ride.airbike)
            .expect("the airbike has an index");
        engine.force_object_position(index, start);
    }
    assert_eq!(
        raw_velocity(&engine, ride.airbike).0,
        -16 * FIXED_ONE,
        "the held dash saturates at FIXED100(1600), twice the shipped bound"
    );

    // Letting go returns the bike to Fly, whose StartCall fires at once and
    // then once per Delay=3 frames, stepping the physical down by 50.
    engine
        .player_in_com(ride.owner, COM_LEFT + COM_RELEASE_OFFSET, 0)
        .expect("releasing the turn key succeeds");
    assert_eq!(
        float_physical(&engine, ride.airbike),
        Some(1550),
        "leaving the dash takes the first decay step immediately"
    );
    tick(&mut engine, 23 * 3);
    assert_eq!(
        float_physical(&engine, ride.airbike),
        Some(400),
        "the decay stops at the raised cruise floor, not the shipped 200"
    );
}

/// The GPED remote control still flies the airbike.
///
/// It steers by calling the airbike's own controls
/// (`target -> ControlLeft(this())`, GPED.c4d/Script.c:15-73), so it runs
/// straight through the replaced handlers and their held-direction
/// bookkeeping; and because it has no release counterpart, it stays on the
/// shipped latched physics. This pins both halves: the bike must still turn on
/// command, and it must still reach its Float physical bound rather than being
/// braked by a held direction the remote pilot can never let go of.
#[test]
fn the_gped_remote_control_still_flies_the_airbike() {
    let mut engine = prepare_installed_scenario(SCENARIO, 0).instantiate();
    let owner = join_pilot(&mut engine, true);
    let sft = engine.crew_cursor(owner).expect("the seated SFT");
    let airbike = engine
        .snapshot()
        .objects
        .into_iter()
        .find(|object| object.definition_id == "AB5B" && object.owner == owner)
        .map(|object| object.id)
        .expect("InitializeClonk creates one airbike per player");

    // Dismount, select the GPED and hand it the bike the shipped way.
    let airbike_index = engine
        .find_object_index(airbike)
        .expect("the airbike has an index");
    engine
        .call_object_function(airbike_index, "Entrance", Vec::new())
        .expect("the shipped dismount runs");
    let gped = engine
        .object_snapshot(sft)
        .expect("the SFT remains live")
        .contents
        .into_iter()
        .find(|&object| {
            engine
                .object_snapshot(object)
                .is_some_and(|snapshot| snapshot.definition_id == "GP5B")
        })
        .expect("InitializeClonk equips one GPED");
    // Only the selected item receives the SFT's forwarded controls
    // (SFT.c4d/Script.c:288-295).
    for _ in 0..8 {
        if engine
            .object_snapshot(sft)
            .expect("the SFT remains live")
            .contents
            .first()
            == Some(&gped)
        {
            break;
        }
        engine
            .player_in_com(owner, COM_WHEEL_DOWN, 0)
            .expect("shifting the inventory succeeds");
    }
    // [Special2] rotates the GPED from Blaster to Control mode, then [Dig]
    // double activates it, and GPED::ControlDig hands it to
    // Airbike::ControlRequest (GPED.c4d/Script.c:90-156,
    // Airbike.c4d/Script.c:180-199,477-489).
    engine
        .player_in_com(owner, COM_SPECIAL2, 0)
        .expect("the mode switch reaches the GPED");
    engine
        .player_in_com(owner, COM_DIG | COM_DOUBLE, 0)
        .expect("activating the GPED succeeds");
    engine
        .player_in_com(owner, COM_DIG + COM_RELEASE_OFFSET, 0)
        .expect("releasing Dig succeeds");
    assert_eq!(
        action_name(&engine, gped),
        "AirbikeFly",
        "ControlRequest puts the GPED into the remote-control action"
    );

    // The GPED steers through the airbike's own controls, so the appended
    // handlers run with a GP5B controller. Each must still command one pure
    // axis: no release ever reaches the bike on this path (no definition in
    // `content/` declares a `Control*Released`), so a held-direction model
    // would latch write-only and the operator could never null an axis again.
    engine
        .player_in_com(owner, COM_LEFT, 0)
        .expect("the turn reaches the GPED");
    assert_eq!(
        com_dir(&engine, airbike),
        CommandDirection::Left,
        "the remote control still steers the airbike"
    );
    tick(&mut engine, 48);
    assert_eq!(
        raw_velocity(&engine, airbike).0,
        -4 * FIXED_ONE,
        "a remote-controlled airbike still reaches its Float physical bound"
    );

    engine
        .player_in_com(owner, COM_UP, 0)
        .expect("the climb reaches the GPED");
    assert_eq!(
        com_dir(&engine, airbike),
        CommandDirection::Up,
        "a remote climb replaces the turn instead of accumulating a diagonal"
    );

    // With the horizontal request cancelled, the shipped physics keep the
    // momentum: nothing brakes a remote-controlled bike.
    tick(&mut engine, 40);
    assert_eq!(
        raw_velocity(&engine, airbike).0,
        -4 * FIXED_ONE,
        "the remote-control path stays on the shipped latched physics"
    );
}

/// The dismount rule and the abandoned bike are untouched: `ControlDown` over
/// solid ground still parks the bike and puts the pilot back on his feet
/// (Airbike.c4d/Script.c:74-90).
#[test]
fn the_shipped_dismount_and_abandoned_physics_are_unchanged() {
    let prepared = prepare_installed_scenario(SCENARIO, 0);
    for with_append in [false, true] {
        let mut engine = if with_append {
            prepared.instantiate()
        } else {
            prepared.instantiate_without_system_script(STEERING_APPEND)
        };
        let owner = join_pilot(&mut engine, true);
        let sft = engine.crew_cursor(owner).expect("the seated SFT");
        let airbike = engine
            .snapshot()
            .objects
            .into_iter()
            .find(|object| object.definition_id == "AB5B" && object.owner == owner)
            .map(|object| object.id)
            .expect("InitializeClonk creates one airbike per player");

        // Airborne: Down steers instead of dismounting, either way.
        engine
            .player_in_com(owner, COM_DOWN, 0)
            .expect("the dive reaches the airbike");
        assert_eq!(
            action_name(&engine, sft),
            "AirbikeFly",
            "an airborne Down must not dismount (with_append={with_append})"
        );
        assert_eq!(
            com_dir(&engine, airbike),
            CommandDirection::Down,
            "an airborne Down dives (with_append={with_append})"
        );

        // The shipped dismount body, as the grounded branch runs it.
        let airbike_index = engine
            .find_object_index(airbike)
            .expect("the airbike has an index");
        engine
            .call_object_function(airbike_index, "Entrance", Vec::new())
            .expect("the shipped dismount runs");
        let dismounted = action_name(&engine, sft);
        assert!(
            dismounted.ends_with("Walk"),
            "the dismounted SFT walks again, not {dismounted} (with_append={with_append})"
        );
        tick(&mut engine, 1);
        assert_eq!(
            raw_velocity(&engine, airbike),
            (0, 0),
            "the parked airbike stops dead (with_append={with_append})"
        );
    }
}

/// Descending and then pressing Down again on touchdown must still dismount.
/// The second press is promoted to `DownDouble` by `C4Player::InCom`
/// (oracle C4Player.cpp:1522-1536), while the shipped SFT deliberately leaves
/// `ControlDownDouble` unhandled (SFT.c4d/Script.c:145-151).
#[test]
fn pressing_down_again_after_touchdown_dismounts_the_airbike() {
    let prepared = prepare_installed_scenario(SCENARIO, 0);
    for auto_stop in [false, true] {
        let mut engine = prepared.instantiate();
        let owner = join_pilot(&mut engine, auto_stop);
        let sft = engine.crew_cursor(owner).expect("the seated SFT");
        let airbike = engine
            .snapshot()
            .objects
            .into_iter()
            .find(|object| object.definition_id == "AB5B" && object.owner == owner)
            .map(|object| object.id)
            .expect("InitializeClonk creates one airbike per player");

        // Start descending in open air and keep Down held through touchdown.
        engine
            .player_in_com(owner, COM_DOWN, 0)
            .expect("the descent reaches the airbike");
        assert_eq!(
            action_name(&engine, sft),
            "AirbikeFly",
            "the first airborne Down descends without dismounting \
             (auto_stop={auto_stop})"
        );

        // Put the bike exactly eleven pixels above solid ground, matching the
        // shipped `GBackSolid(0, 11)` dismount probe (Airbike.c4d/Script.c:78).
        let airbike_state = engine
            .object_snapshot(airbike)
            .expect("the airbike remains live");
        let landscape = engine.landscape().expect("the scenario landscape");
        let (ground_x, solid_y) = landscape
            .surface()
            .iter()
            .enumerate()
            .skip(18)
            .take(landscape.width() as usize - 36)
            .find_map(|(x, &surface_y)| {
                let x = i32::try_from(x).ok()?;
                let position = Vector2::new(x, surface_y - 11);
                let probe_is_solid = engine.debug_landscape_is_solid(x, surface_y);
                let bike_is_stuck = airbike_state.vertices.iter().any(|vertex| {
                    engine.debug_landscape_is_solid(position.x + vertex.x, position.y + vertex.y)
                });
                (probe_is_solid && !bike_is_stuck).then_some((x, surface_y))
            })
            .expect("one solid landscape surface");
        let airbike_index = engine
            .find_object_index(airbike)
            .expect("the airbike has an index");
        engine.force_object_position(airbike_index, Vector2::new(ground_x, solid_y - 11));

        // Brake on touchdown, then press Down to dismount. This press is a
        // DownDouble because the descent remains in the native double-click
        // window, but it must retain Down's grounded meaning.
        engine
            .player_in_com(owner, COM_DOWN + COM_RELEASE_OFFSET, 0)
            .expect("releasing the descent succeeds");
        engine
            .player_in_com(owner, COM_DOWN, 0)
            .expect("the landing press reaches the seated pilot");
        assert_eq!(
            engine
                .player(owner)
                .expect("the pilot's player remains live")
                .control
                .last_com,
            i32::from(COM_DOWN | COM_DOUBLE),
            "the landing press really took the native DownDouble route \
             (auto_stop={auto_stop})"
        );
        assert!(
            action_name(&engine, sft).ends_with("Walk"),
            "the landing press dismounts instead of disappearing as \
             ControlDownDouble (auto_stop={auto_stop})"
        );
    }
}
