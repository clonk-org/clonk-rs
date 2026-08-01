use crate::support::real_scenario::load_installed_scenario;
use clonk_engine::math::{itofix_prec, C4Fixed, FixedVec2};
use clonk_engine::{SpawnConfig, Vector2};
use clonk_script::Value;

#[test]
fn blobby_soccer_int_shield_effect_call_reverses_ball_with_raw_fixed_speed() {
    // FnEffectCall retains its extras by value before C4Effect::DoCall resolves
    // FxIntShieldCheckBall in the command-id definition
    // (src/C4Script.cpp:5583-5595; src/C4Effect.cpp:439-457). That callback
    // passes the BALL through SetSpeed, whose tenths become raw C4Fixed XDir /
    // YDir on the supplied object (src/C4Script.cpp:698-708,724-735).
    let mut engine = load_installed_scenario("mods/BlobbySoccer.c4s", 0);
    for definition in ["BALL", "MAG2", "MSHS"] {
        assert!(
            engine.definition(definition).is_some(),
            "BlobbySoccer supplies {definition}"
        );
    }

    let mage = engine
        .spawn_object(
            SpawnConfig::new("MAG2")
                .with_position(Vector2::new(400, 275))
                .with_owner(1)
                .with_alive(true)
                .with_crew_member(true),
        )
        .expect("BlobbySoccer mage spawns");
    let shield = engine
        .spawn_object(SpawnConfig::new("MSHS"))
        .expect("BlobbySoccer shield spawns");
    let ball = engine
        .spawn_object(
            SpawnConfig::new("BALL")
                .with_position(Vector2::new(400, 300))
                .with_fixed_velocity(FixedVec2::new(C4Fixed::ZERO, itofix_prec(-40, 10))),
        )
        .expect("BlobbySoccer ball spawns");

    let shield_index = engine
        .find_object_index(shield)
        .expect("shield remains live during activation");
    assert_eq!(
        engine
            .call_object_function(
                shield_index,
                "Activate",
                vec![Value::Object(mage.as_u64()), Value::Object(mage.as_u64())],
            )
            .expect("MSHS::Activate executes"),
        Value::Int(1)
    );

    let mage_snapshot = engine.object_snapshot(mage).expect("mage remains live");
    let shield_effect = mage_snapshot
        .effects
        .iter()
        .find(|effect| effect.name == "IntShield")
        .expect("MSHS::Activate attaches IntShield to the mage");
    assert_eq!(shield_effect.command_id.as_deref(), Some("MSHS"));

    let ball_index = engine
        .find_object_index(ball)
        .expect("ball remains live before its aura timer");
    engine
        .call_object_function(
            ball_index,
            "FxCheckClonksTimer",
            vec![Value::Object(ball.as_u64()), Value::Int(1)],
        )
        .expect("BALL::FxCheckClonksTimer executes");

    let ball_snapshot = engine
        .object_snapshot(ball)
        .expect("aura callback keeps the ball live");
    let fixed_velocity = ball_snapshot.fixed_velocity.unwrap_or_else(|| {
        FixedVec2::from_ints(ball_snapshot.velocity.x, ball_snapshot.velocity.y)
    });
    assert_eq!(fixed_velocity.x.val(), 0);
    assert_eq!(fixed_velocity.y.val(), itofix_prec(40, 10).val());
    assert!(ball_snapshot.mobile, "SetYDir mobilizes the BALL");
}
