//! `C4Effect` executes every `Fx*` callback on the effect's **command
//! target**: `pFnTimer->Exec(pEffect->pCommandTarget, {C4VObj(pObj), ...})`
//! (src/C4Effect.cpp:345, and the same shape at :129 Start, :392/:417 Stop,
//! :434 Damage, :282 Effect). `C4AulScriptFunc::Exec` makes that first
//! argument the calling object (src/C4AulExec.cpp:1638-1648), so every native
//! that falls back to `cthr->Obj` reads the command target; the affected
//! object reaches script only as parameter 1.
//!
//! ClonkMars' Spaceclonk depends on it: its helmet lamp is an `LGHC` driven by
//! an effect whose command target is the clonk, and the timer's first
//! statement is `if (!GetAlive()) { pTarget->RemoveObject(); return -1; }`
//! (Living.c4d/Spaceclonk.c4d/Script.c:28-31,325-329).

use crate::support::real_scenario::load_installed_scenario;
use clonk_engine::{
    Definition, Engine, ObjectId, PlayerConfig, SpawnConfig, Vector2, CATEGORY_LIVING,
    CATEGORY_VEHICLE,
};
use clonk_script::Value;

const PLAYER: i32 = 3;

const COMMAND_TARGET: &str = r#"#strict 2
local probe_alive, probe_action, probe_category, probe_name, probe_owner;
local probe_id, probe_param, probe_this;

func Arm(object pTarget)
{
  return AddEffect("Probe", pTarget, 100, 1, this());
}

func FxProbeTimer(object pTarget, int iNumber, int iTime)
{
  probe_alive = GetAlive();
  probe_action = GetAction();
  probe_category = GetCategory();
  probe_name = GetName();
  probe_owner = GetOwner();
  probe_id = GetID();
  probe_param = pTarget;
  probe_this = this();
  return -1;
}
"#;

const CARRIER: &str = r#"#strict 2
func Initialize()
{
  SetAction("Idle");
}
"#;

#[test]
fn effect_callbacks_run_implicit_object_natives_on_the_command_target() {
    let mut engine = Engine::new();
    for (id, name, script) in [
        ("CTGT", "Conrad", COMMAND_TARGET),
        ("CARR", "Rock", CARRIER),
    ] {
        let mut definition =
            Definition::from_script(id, name, script).expect("definition compiles");
        // Real content receives object references, not the synthetic state
        // proplists the command-DSL fixtures use.
        definition.set_c4_callback_convention(true);
        engine
            .register_definition(definition)
            .expect("definition registers");
    }

    let carrier = engine
        .spawn_object(SpawnConfig::new("CARR").with_category(CATEGORY_VEHICLE))
        .expect("carrier spawns");
    let commander = engine
        .spawn_object(
            SpawnConfig::new("CTGT")
                .with_category(CATEGORY_LIVING)
                .with_owner(PLAYER)
                .with_alive(true),
        )
        .expect("command target spawns");

    let index = engine
        .find_object_index(commander)
        .expect("command target is live");
    engine
        .call_object_function(index, "Arm", vec![Value::Object(carrier.as_u64())])
        .expect("the cross-targeted effect arms");
    engine.tick_without_snapshot().expect("timer frame runs");

    let locals = engine
        .object_snapshot(commander)
        .expect("command target survives")
        .local_vars;
    let probe = |name: &str| locals.get(name).cloned().unwrap_or(Value::Nil);

    assert_eq!(
        probe("probe_param"),
        Value::Object(carrier.as_u64()),
        "the affected object reaches script only as parameter 1"
    );
    assert_eq!(
        probe("probe_this"),
        Value::Object(commander.as_u64()),
        "this() is the command target"
    );
    assert!(
        probe("probe_alive").as_bool(),
        "GetAlive must read the command target, not the lifeless carrier"
    );
    assert_eq!(probe("probe_id"), Value::C4Id("CTGT".to_string()));
    assert_eq!(probe("probe_name"), Value::String("Conrad".into()));
    assert_eq!(probe("probe_owner"), Value::Int(PLAYER));
    assert_eq!(probe("probe_category"), Value::Int(CATEGORY_LIVING));
    assert_eq!(probe("probe_action"), Value::String("Idle".into()));
}

#[test]
fn mars_spaceclonk_headlamp_survives_its_first_timer_tick() {
    // The Spaceclonk builds an LGHC light cone and drives it from an effect
    // whose target is the lamp and whose command target is the clonk
    // (Living.c4d/Spaceclonk.c4d/Script.c:28-31). `FxHeadlampTimer` opens with
    // `if (!GetAlive()) { pTarget->RemoveObject(); return -1; }` (:325-329):
    // resolved against the lamp — a C4D_Vehicle that is never alive — that
    // branch deletes the crew's only directional light one frame after every
    // spawn.
    let mut engine = load_installed_scenario("ClonkMars.c4f/01_Fossae.c4s", 0);
    engine
        .register_player(PlayerConfig::new(PLAYER, "Mars headlamp tester"))
        .expect("test player registers");

    let clonk = engine
        .spawn_object(
            SpawnConfig::new("SCNK")
                .with_owner(PLAYER)
                .with_controller(PLAYER)
                .with_alive(true)
                .with_crew_member(true)
                .with_position(Vector2::new(300, 200)),
        )
        .expect("Spaceclonk spawns");

    let lamps = |engine: &Engine| -> Vec<ObjectId> {
        engine
            .objects
            .iter()
            .filter(|object| object.definition_id.as_str() == "LGHC" && !object.destroyed)
            .map(|object| object.id)
            .collect()
    };
    assert_eq!(
        lamps(&engine).len(),
        1,
        "the Spaceclonk builds exactly one helmet lamp"
    );

    for _ in 0..5 {
        engine.tick_without_snapshot().expect("headlamp frame runs");
    }

    assert_eq!(
        lamps(&engine).len(),
        1,
        "the headlamp timer reads GetAlive on its live command target, so the \
         lamp must outlive its first tick"
    );
    assert!(
        engine
            .object_snapshot(clonk)
            .is_some_and(|snapshot| snapshot.alive),
        "the Spaceclonk itself stays alive across those frames"
    );
}
