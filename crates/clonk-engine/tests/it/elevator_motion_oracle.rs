use std::error::Error;

use clonk_engine::{
    math::C4Fixed, CommandDirection, Engine, JoinPlayerConfig, Landscape, ObjectId, COM_DIG,
    COM_DOWN, COM_RIGHT, COM_UP,
};

use crate::support::real_scenario::load_tutorial;
use crate::support::virtual_player::VirtualPlayer;

const DOWN_POSITION_PREFIX: [i32; 21] = [
    0, 6_553, 19_659, 39_318, 65_530, 98_295, 137_613, 183_484, 235_908, 294_885, 327_674, 399_757,
    478_393, 563_582, 655_324, 753_619, 858_467, 969_868, 1_087_822, 1_212_329, 1_310_708,
];

const UP_POSITION_PREFIX: [i32; 23] = [
    0, 0, -6_553, -19_659, -39_318, -65_530, -98_295, -137_613, -183_484, -235_908, -294_885,
    -360_415, -432_498, -511_134, -596_323, -688_065, -786_360, -891_208, -1_002_609, -1_120_563,
    -1_238_517, -1_356_471, -1_474_425,
];

fn load_tutorial07() -> (Engine, i32) {
    let mut engine = load_tutorial(7, 0);
    let owner = crate::support::TestValueExt::test_value(engine.join_player(JoinPlayerConfig {
        name: "Tutorial 7 elevator oracle".to_owned(),
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
        auto_context_menu: true,
        startup_player_count: 1,
    }))
    .number();
    (engine, owner)
}

fn tutorial_message_contains(engine: &Engine, needle: &str) -> bool {
    engine.message_line_contains(needle)
}

fn object_with_definition(engine: &Engine, definition: &str) -> Option<ObjectId> {
    engine.first_object_for_definition(definition)
}

fn raw_fixed(value: Option<C4Fixed>, pixels: i32) -> i32 {
    value.map_or(pixels * 65_536, C4Fixed::val)
}

struct CaseFrameExpectation {
    tick: u32,
    start_y: i32,
    delta_y: i32,
    y_dir: i32,
    action: &'static str,
    action_time: i32,
    com_dir: CommandDirection,
}

fn assert_case_frame(engine: &Engine, elevator_case: ObjectId, expected: CaseFrameExpectation) {
    let case = crate::support::TestValueExt::test_value(engine.object_snapshot(elevator_case));
    let fixed_y = raw_fixed(
        case.fixed_position.map(|position| position.y),
        case.position.y,
    );
    let fixed_y_dir = raw_fixed(
        case.fixed_velocity.map(|velocity| velocity.y),
        case.velocity.y,
    );
    assert_eq!(
        fixed_y - expected.start_y,
        expected.delta_y,
        "fixed y at tick {}",
        expected.tick
    );
    assert_eq!(
        fixed_y_dir, expected.y_dir,
        "fixed ydir at tick {}",
        expected.tick
    );
    assert_eq!(
        case.action.name, expected.action,
        "action at tick {}",
        expected.tick
    );
    assert_eq!(
        case.action.time, expected.action_time,
        "action time at tick {}",
        expected.tick
    );
    assert_eq!(
        case.command_direction, expected.com_dir,
        "command direction at tick {}",
        expected.tick
    );
}

fn expected_down_position(tick: u32) -> i32 {
    match tick {
        0..=20 => DOWN_POSITION_PREFIX[tick as usize],
        21..=76 => 1_441_780 + (tick as i32 - 21) * 131_072 + i32::from(tick >= 30) * 12,
        77 => 8_650_752,
        _ => unreachable!("the oracle records only ticks 0..=77"),
    }
}

fn expected_up_position(tick: u32) -> i32 {
    match tick {
        0..=22 => UP_POSITION_PREFIX[tick as usize],
        23..=80 => {
            -1_598_932 - (tick as i32 - 23) * 124_507
                + i32::from(tick >= 25) * 19_503
                + i32::from(tick >= 50) * 32_483
                + i32::from(tick >= 75) * 32_483
        }
        81 => -8_585_216,
        _ => unreachable!("the oracle records only ticks 0..=81"),
    }
}

#[test]
#[cfg_attr(
    not(target_os = "macos"),
    ignore = "recording-host material order; required macOS CI job"
)]
fn tutorial07_seed_zero_landscape_matches_cpp_surface8() {
    // Oracle: the C++ engine's Surface8 after C4Landscape::Init for
    // Tutorial07 with LC_PIN_SEED=0. This covers all 680x480 pixels, not
    // selected terrain samples (C4Landscape.cpp:554-580).
    let engine = load_tutorial(7, 0);
    let grid = crate::support::TestValueExt::test_value(
        engine.landscape().and_then(Landscape::pixel_grid),
    );
    assert_eq!((grid.width(), grid.height()), (680, 480));
    let hash = grid
        .bytes()
        .iter()
        .fold(0xcbf2_9ce4_8422_2325_u64, |hash, byte| {
            (hash ^ u64::from(*byte)).wrapping_mul(0x0000_0100_0000_01b3)
        });
    assert_eq!(hash, 0x2310_7266_3100_b0cd);
}

#[test]
fn tutorial07_elevator_matches_cpp_fixed_motion_and_shaft_contact() -> Result<(), Box<dyn Error>> {
    // Oracle: LegacyClonk with LC_PIN_SEED=0, after the real Tutorial07
    // Script12 "Good luck" handoff. ELEC::ControlDig/ControlUp come from
    // Case.c4d/Script.c:303-333; DFA_FLOAT acceleration/contact are
    // C4Object.cpp:5268-5287 and C4Movement.cpp:218-265. The ignored
    // Tutorial07ElevatorOracle.c4s probe captured every fixed frame.
    let (mut engine, owner) = load_tutorial07();
    let clonk = crate::support::TestValueExt::test_value(engine.crew_cursor(owner));
    let elevator_case =
        crate::support::TestValueExt::test_value(object_with_definition(&engine, "ELEC"));
    let mut player = VirtualPlayer::new(&mut engine, owner);

    player.wait_until("Tutorial07 says Good luck", 2_000, |engine| {
        tutorial_message_contains(engine, "Good luck")
    })?;
    let landscape = crate::support::TestValueExt::test_value(player.engine().landscape());
    assert_eq!(
        [335, 336, 337, 338].map(|y| landscape.is_solid_at(88, y)),
        [false; 4],
        "the seed-zero C++ oracle has no stray solids in ELEC's shaft"
    );
    player.hold_until(COM_RIGHT, "the Clonk reaches ELEC", 100, |engine| {
        engine
            .object_snapshot(clonk)
            .zip(engine.object_snapshot(elevator_case))
            .is_some_and(|(clonk, elevator)| (clonk.position.x - elevator.position.x).abs() <= 5)
    })?;
    player.tap(COM_DOWN)?;
    player.wait_until("the Clonk grabs ELEC", 60, |engine| {
        engine.object_snapshot(clonk).is_some_and(|object| {
            object.action.name == "Push" && object.action.target == Some(elevator_case)
        })
    })?;
    while !player.engine().frame().is_multiple_of(5) {
        player.ticks(1)?;
    }

    player.press(COM_DIG)?;
    let down_start = crate::support::TestValueExt::test_value(
        player.engine().object_snapshot(elevator_case).map(|case| {
            raw_fixed(
                case.fixed_position.map(|position| position.y),
                case.position.y,
            )
        }),
    );
    assert_eq!(down_start, 12_779_520, "C++ starts ELEC at y=195");
    for tick in 0..=77 {
        let stopped = tick == 77;
        let y_dir = match tick {
            0 | 77 => 0,
            1..=20 => 6_553 * tick as i32,
            _ => 131_072,
        };
        assert_case_frame(
            player.engine(),
            elevator_case,
            CaseFrameExpectation {
                tick,
                start_y: down_start,
                delta_y: expected_down_position(tick),
                y_dir,
                action: if stopped { "Wait" } else { "Drill" },
                action_time: if stopped { 0 } else { tick as i32 },
                com_dir: if stopped {
                    CommandDirection::Stop
                } else {
                    CommandDirection::Down
                },
            },
        );
        if tick < 77 {
            player.ticks(1)?;
        }
    }
    assert_eq!(
        player
            .engine()
            .object_snapshot(elevator_case)
            .expect("ELEC reaches the shaft floor")
            .position
            .y,
        327,
        "C++ contacts the Tutorial07 shaft floor at y=327"
    );

    player.ticks(220 - 77)?;
    player.release(COM_DIG)?;
    player.ticks(20)?;
    player.press(COM_UP)?;
    let up_start = crate::support::TestValueExt::test_value(
        player.engine().object_snapshot(elevator_case).map(|case| {
            raw_fixed(
                case.fixed_position.map(|position| position.y),
                case.position.y,
            )
        }),
    );
    assert_eq!(up_start, 21_430_272, "C++ starts ascent at y=327");
    for tick in 0..=81 {
        let stopped = tick == 81;
        let y_dir = match tick {
            0..=1 | 81 => 0,
            2..=19 => -6_553 * (tick as i32 - 1),
            20..=22 => -117_954,
            _ => -124_507,
        };
        let com_dir = match tick {
            2..=19 | 23 => CommandDirection::Up,
            _ => CommandDirection::Stop,
        };
        assert_case_frame(
            player.engine(),
            elevator_case,
            CaseFrameExpectation {
                tick,
                start_y: up_start,
                delta_y: expected_up_position(tick),
                y_dir,
                action: if stopped { "Wait" } else { "Ride" },
                action_time: if stopped { 1 } else { tick as i32 },
                com_dir,
            },
        );
        if tick < 81 {
            player.ticks(1)?;
        }
    }
    player.release(COM_UP)?;

    assert_eq!(
        player
            .engine()
            .object_snapshot(elevator_case)
            .expect("ELEC survives the round trip")
            .position
            .y,
        196,
        "C++ returns ELEC to its y=196 top contact"
    );
    Ok(())
}
