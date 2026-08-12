use crate::support::real_scenario::load_installed_scenario;
use crate::support::EngineTestExt;
use clonk_engine::{
    Engine, JoinPlayerConfig, ObjectId, SpawnConfig, COM_DIG, COM_DOWN, COM_LEFT,
    COM_RELEASE_OFFSET, COM_RIGHT, COM_THROW, COM_UP, COM_WHEEL_DOWN,
};
use clonk_script::Value;
use std::collections::HashMap;

fn local(engine: &Engine, object: ObjectId, name: &str) -> Value {
    engine
        .object_snapshot(object)
        .unwrap_or_else(|| panic!("object {object:?} remains live"))
        .local_vars
        .get(name)
        .cloned()
        .unwrap_or(Value::Nil)
}

fn join_guiding_player(engine: &mut Engine, auto_stop: bool) -> i32 {
    crate::support::TestValueExt::test_value(
        crate::support::TestValueExt::test_value(engine.join_player(JoinPlayerConfig {
            name: "Missile guidance".into(),
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
        }))
        .initialized(),
    )
    .number
}

/// Arms the joined SFT with a loaded RL5B, fires it and switches on the
/// remote guidance, returning `(owner, launcher, missile)`.
fn launch_guided_missile(engine: &mut Engine, auto_stop: bool) -> (i32, ObjectId, ObjectId) {
    let owner = join_guiding_player(engine, auto_stop);
    let sft = crate::support::TestValueExt::test_value(engine.crew_cursor(owner));

    // A launcher reloaded from a RocketBox carries ammo 100 and one MS5B
    // (EkeReloaded.c4d/Weapons.c4d/RocketLauncher.c4d/Script.c:118-131).
    let mut ammo = HashMap::new();
    ammo.insert("ammo".to_string(), Value::Int(100));
    let launcher = engine.spawn_test_object(
        SpawnConfig::new("RL5B")
            .with_loaded(true)
            .with_container(sft)
            .with_local_vars(ammo),
    );
    engine.spawn_test_object(
        SpawnConfig::new("MS5B")
            .with_loaded(true)
            .with_container(launcher),
    );

    // Select the launcher through the engine's own shift control, then let
    // SFT::CheckArmed swap in the RocketLauncherWalk action ControlThrow
    // requires (SFT.c4d/Script.c:385-421).
    for _ in 0..8 {
        if engine.test_object_snapshot(sft).contents.first() == Some(&launcher) {
            break;
        }
        crate::support::TestValueExt::test_value(engine.player_in_com(owner, COM_WHEEL_DOWN, 0));
    }
    for _ in 0..4 {
        let sft_index = engine.test_object_index(sft);
        engine.call_test_object_function(sft_index, "CheckArmed", Vec::new());
        if engine.test_object_snapshot(sft).action.name == "RocketLauncherWalk" {
            break;
        }
    }

    crate::support::TestValueExt::test_value(engine.player_in_com(owner, COM_THROW, 0));
    let missile = match local(engine, launcher, "missile") {
        Value::Object(id) => ObjectId::new(id),
        other => panic!("ControlThrow launched no missile: {other:?}"),
    };
    assert_eq!(
        local(engine, missile, "command"),
        Value::String("Straight".into()),
        "a freshly launched RL5B missile flies straight"
    );

    crate::support::TestValueExt::test_value(engine.player_in_com(owner, COM_DIG, 0));
    assert_eq!(
        local(engine, launcher, "guiding"),
        Value::Int(1),
        "ControlDig arms the launcher's guiding local"
    );

    (owner, launcher, missile)
}

fn steering_command(engine: &Engine, missile: ObjectId) -> Value {
    local(engine, missile, "command")
}

/// Hold-to-steer: releasing the turn key straightens the guided missile.
///
/// This is an intentional divergence from the oracle. In C++ a key-up becomes
/// COM_Left_R (C4Constants.h:199-211), `ComName` turns it into "LeftReleased"
/// (C4ObjectCom.cpp:814-821) and `C4Object::CallControl` calls
/// `~ControlLeftReleased` (C4Object.cpp:3348-3366) — but shipped Eke content
/// defines no `*Released` handler in the SFT -> RL5B -> MS5B chain, so the
/// latched turn survived the release and only [Down]/[Up] cleared it.
/// `planet/System.c4g/EkeGuidedMissile.c` supplies those handlers.
#[test]
fn eke_missile_stops_turning_when_the_turn_key_is_released() {
    for auto_stop in [false, true] {
        let mut engine = load_installed_scenario(
            "EkeReloaded.c4f/InterplanetaryCivilwar.c4f/MissileMatch.c4s",
            0,
        );
        let (owner, _launcher, missile) = launch_guided_missile(&mut engine, auto_stop);

        crate::support::TestValueExt::test_value(engine.player_in_com(owner, COM_LEFT, 0));
        assert_eq!(
            steering_command(&engine, missile),
            Value::String("Left".into()),
            "holding Left turns the missile (auto_stop={auto_stop})"
        );
        crate::support::TestValueExt::test_value(engine.tick_without_snapshot());
        assert_eq!(
            engine.test_object_snapshot(missile).rotation,
            85,
            "the held turn advances r by rdir * 5 (C4Movement.cpp:389-392)"
        );

        crate::support::TestValueExt::test_value(engine.player_in_com(
            owner,
            COM_LEFT + COM_RELEASE_OFFSET,
            0,
        ));
        assert_eq!(
            steering_command(&engine, missile),
            Value::String("Straight".into()),
            "releasing Left stops the turn (auto_stop={auto_stop})"
        );
        crate::support::TestValueExt::test_value(engine.tick_without_snapshot());
        assert_eq!(
            engine.test_object_snapshot(missile).rotation,
            85,
            "the released turn leaves the heading alone"
        );

        crate::support::TestValueExt::test_value(engine.player_in_com(owner, COM_RIGHT, 0));
        crate::support::TestValueExt::test_value(engine.player_in_com(
            owner,
            COM_RIGHT + COM_RELEASE_OFFSET,
            0,
        ));
        assert_eq!(
            steering_command(&engine, missile),
            Value::String("Straight".into()),
            "releasing Right stops the turn (auto_stop={auto_stop})"
        );
    }
}

/// Rolling from one turn key onto the other must keep the newer turn: the
/// stale release only straightens the direction it actually owns.
#[test]
fn eke_missile_keeps_the_newer_turn_when_the_previous_key_is_released() {
    let mut engine = load_installed_scenario(
        "EkeReloaded.c4f/InterplanetaryCivilwar.c4f/MissileMatch.c4s",
        0,
    );
    let (owner, _launcher, missile) = launch_guided_missile(&mut engine, false);

    crate::support::TestValueExt::test_value(engine.player_in_com(owner, COM_LEFT, 0));
    crate::support::TestValueExt::test_value(engine.player_in_com(owner, COM_RIGHT, 0));
    assert_eq!(
        steering_command(&engine, missile),
        Value::String("Right".into())
    );

    crate::support::TestValueExt::test_value(engine.player_in_com(
        owner,
        COM_LEFT + COM_RELEASE_OFFSET,
        0,
    ));
    assert_eq!(
        steering_command(&engine, missile),
        Value::String("Right".into()),
        "the stale Left release must not cancel the held Right turn"
    );

    crate::support::TestValueExt::test_value(engine.player_in_com(
        owner,
        COM_RIGHT + COM_RELEASE_OFFSET,
        0,
    ));
    assert_eq!(
        steering_command(&engine, missile),
        Value::String("Straight".into()),
        "releasing the owning key stops the turn"
    );
}

/// [Down]/[Up] keep their shipped meaning: an explicit straighten.
#[test]
fn eke_missile_down_and_up_still_straighten_the_guided_missile() {
    for auto_stop in [false, true] {
        let mut engine = load_installed_scenario(
            "EkeReloaded.c4f/InterplanetaryCivilwar.c4f/MissileMatch.c4s",
            0,
        );
        let (owner, _launcher, missile) = launch_guided_missile(&mut engine, auto_stop);

        crate::support::TestValueExt::test_value(engine.player_in_com(owner, COM_LEFT, 0));
        assert_eq!(
            steering_command(&engine, missile),
            Value::String("Left".into()),
            "Left latches the left turn (auto_stop={auto_stop})"
        );
        crate::support::TestValueExt::test_value(engine.tick_without_snapshot());
        let turning = engine.test_object_snapshot(missile);
        assert_eq!(
            turning.rotation, 85,
            "MS5B::Flying applies SetRDir(-10), and C4Movement advances \
             fix_r by rdir * 5 each frame (C4Movement.cpp:389-392)"
        );

        crate::support::TestValueExt::test_value(engine.player_in_com(owner, COM_DOWN, 0));
        assert_eq!(
            steering_command(&engine, missile),
            Value::String("Straight".into()),
            "Down clears the latched turn (auto_stop={auto_stop})"
        );
        crate::support::TestValueExt::test_value(engine.tick_without_snapshot());
        let straight = engine.test_object_snapshot(missile);
        assert_eq!(
            straight.rotation, 85,
            "SetRDir(0) stops the spin on the next Flying phase call"
        );

        crate::support::TestValueExt::test_value(engine.player_in_com(owner, COM_RIGHT, 0));
        assert_eq!(
            steering_command(&engine, missile),
            Value::String("Right".into()),
            "Right latches the right turn (auto_stop={auto_stop})"
        );
        crate::support::TestValueExt::test_value(engine.player_in_com(owner, COM_UP, 0));
        assert_eq!(
            steering_command(&engine, missile),
            Value::String("Straight".into()),
            "RL5B::ControlUp forwards to ControlDown (auto_stop={auto_stop})"
        );
    }
}
