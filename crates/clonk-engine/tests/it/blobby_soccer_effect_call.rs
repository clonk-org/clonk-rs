use clonk_engine::math::{itofix_prec, C4Fixed, FixedVec2};
use clonk_engine::{Definition, Engine, SpawnConfig};
use clonk_script::Value;

fn register_real_c4_definition(engine: &mut Engine, id: &str, name: &str, source: &str) {
    let mut definition = Definition::from_script(id, name, source)
        .unwrap_or_else(|error| panic!("{id} fixture definition compiles: {error}"));
    definition.set_c4_callback_convention(true);
    engine
        .register_definition(definition)
        .unwrap_or_else(|error| panic!("{id} fixture definition registers: {error}"));
}

#[test]
fn blobby_soccer_int_shield_effect_call_reverses_ball_with_raw_fixed_speed() {
    // Hermetic reduction of `mods/BlobbySoccer.c4s/Ball.c4d/Script.c`'s
    // BALL::FxCheckClonksTimer and `Shield.c4d/Script.c`'s
    // MSHS::FxIntShieldCheckBall. The latter really declares its third
    // callback parameter as `int iEffecttime`, while FxCheckClonksTimer
    // forwards the BALL object as EffectCall's first extra. Pre-STRICT3
    // C4Aul conversion warns but preserves that object
    // (src/C4AulExec.cpp:1364-1397,1610-1627,1638-1656). FnEffectCall retains
    // the extra by value before C4Effect::DoCall resolves the command-id
    // definition (src/C4Script.cpp:5583-5601; src/C4Effect.cpp:439-457).
    // The callback then gives that preserved value to SetSpeed(xdir, ydir,
    // ball), whose SetXDir/SetYDir calls retain raw C4Fixed tenths
    // (src/C4Script.cpp:697-708,724-735).
    let mut engine = Engine::new();
    register_real_c4_definition(&mut engine, "MAG2", "Blobby mage", "#strict 2\n");
    register_real_c4_definition(
        &mut engine,
        "MSHS",
        "Blobby IntShield",
        r#"#strict 2
func Activate(pCaller, pClonk)
{
  if(!pClonk) pClonk = pCaller;
  if(!GetEffect("IntShield", pCaller))
    AddEffect("IntShield", pCaller, 200, 1, 0, MSHS);
  else return(0);
  return(1);
}

global func FxIntShieldCheckBall(object pTarget, int iEffectNumber, int iEffectTime)
{
  return(SetSpeed(0, 40, iEffectTime));
}

global func SetSpeed(int xdir, int ydir, object pBall)
{
  SetXDir(xdir, pBall);
  SetYDir(ydir, pBall);
  return(1);
}
"#,
    );
    register_real_c4_definition(
        &mut engine,
        "BALL",
        "Blobby ball",
        r#"#strict 2
local shielded_mage;

func SetShieldedMage(object mage)
{
  shielded_mage = mage;
  return(1);
}

func FxCheckClonksTimer(pTarget, iEffectNumber)
{
  EffectCall(shielded_mage, GetEffect("IntShield", shielded_mage), "CheckBall", pTarget);
}
"#,
    );

    let mage = engine
        .spawn_object(
            SpawnConfig::new("MAG2")
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
    assert_eq!(
        engine
            .call_object_function(
                ball_index,
                "SetShieldedMage",
                vec![Value::Object(mage.as_u64())],
            )
            .expect("BALL remembers the shielded mage"),
        Value::Int(1)
    );
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
