//! Phase-1 C++↔Rust differential parity check.
//!
//! Runs the determinism-critical primitives (`C4Fixed`, the LCG RNG, and the
//! per-frame sub-pixel accumulation) through the Rust port and asserts they are
//! byte-for-byte identical to the C++ golden oracle in
//! `parity/golden/parity_golden.json`. That golden is produced from the REAL
//! engine code (`src/Fixed.h`, `src/Fixed.cpp`'s `SineTable`, `src/C4Random.h`,
//! `src/C4ScriptKiller.h`, `src/C4LandscapePath.h`, and
//! `src/C4ActionDirection.h`, `src/C4ActionCallbacks.h`, and
//! `src/C4SolidMaskBitmap.h`, mechanically extracted DFA_PUSH/DFA_PULL/DFA_FIGHT
//! direction blocks from `src/C4Object.cpp`, `C4PlayerList::GetCount` and
//! `Join`'s capacity block from `src/C4PlayerList.cpp`, plus complete `FnEval`,
//! DirectExec's temporary context setup, `C4Effect::Execute`, C4AulScriptFunc's engine-call
//! forwarding and script-context setup, `FnGetX`/`FnGetY`,
//! `C4Object::DigOutMaterialCast`,
//! C4Object::ExecLife's breathable-supply block,
//! `C4Game::ShakeObjects`, `C4Object::Fling`, `C4Landscape::ClearPix`,
//! `BlastFreePix`, `BlastFree`, `ExecuteScan`, and `DoScan` bodies and the
//! `C4SGame::ConvertGoals`, `C4Game::InitRules`/`InitGoals`, and
//! bottom/top/side-flight `C4Object::ContactAction` arms, and the ordinary
//! unattached `C4Object::DoMovement` translation/rotation blocks with complete
//! `ContactCheck`, `TargetBounds`, `C4Shape::Rotate`, redirection and friction
//! helpers) by
//! `parity/oracle/gen_golden.sh` — so this is a genuine differential against
//! the C++ oracle, not a Rust-vs-Rust regression.
//!
//! This gates Theme C (wiring fixed precision through physics): the gravity /
//! velocity sub-pixel accumulation and bounded per-pixel collision matrices are
//! exactly the arithmetic and ordering Theme C extends. Full content scenarios
//! remain the subject of a future live-bridge differential.
//!
//! On any divergence the test panics with the first mismatch (section, index,
//! field, C++ value vs Rust value).

use clonk_resources::{Group, MaterialLibrary, MutableGroup};
use clonk_script::{c4_hash_combine, cnv_fn, C4VType, Value as ScriptValue, ValueMap};
use serde_json::Value;

use crate::compat::{cos_func, sin_func, sqrt_func, LandscapeOperation};
use crate::landscape::{Landscape, LandscapeRasterState, PixelGrid};
use crate::material::{
    consume_corrosion_effect_rng, evaluate_corrosion, MaterialInteractionEvent, MaterialSet,
};
use crate::math::{
    fixed10, fixed100, fixed256, fixtoi, fixtoi_prec, itofix, itofix_prec, C4Fixed, FixedVec2,
};
use crate::rng::LcgRng;
use crate::scenario::{
    parse_serialized_c4value, GameParameterRuleGoalLists, LegacyDefinitionResolver,
    MapPixelClassifier, ScenarioError, ScenarioIdListEntry, SerializedC4ValueResolution,
};
use crate::{
    contact_action_wall_tumble_x, ActionSpec, ActionState, CommandDirection, Definition,
    DefinitionPicture, DefinitionRect, DefinitionSpriteImage, DefinitionTargetRect, Direction,
    EffectVarValue, Engine, EngineError, JoinPlayerConfig, ObjectBaseGraphics, ObjectId,
    ObjectStatus, ObjectUpdate, PhysicalInfo, PhysicsSettings, PlayerConfig, Scenario,
    ShapeAttachRecord, SpawnConfig, CATEGORY_LIVING, CATEGORY_OBJECT, CATEGORY_STATIC_BACK,
    CATEGORY_VEHICLE, OWNER_NONE,
};
use std::collections::{BTreeSet, HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

const GOLDEN: &str = concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/../../parity/golden/parity_golden.json"
);

fn load_golden() -> Value {
    let text = std::fs::read_to_string(GOLDEN).unwrap_or_else(|e| {
        panic!(
            "could not read C++ golden at {GOLDEN}: {e}\n\
             Generate it with `parity/oracle/gen_golden.sh`."
        )
    });
    serde_json::from_str(&text).expect("golden parity JSON parses")
}

fn native_material_set(files: &[(&str, &[u8])]) -> MaterialSet {
    let mut packed_group = MutableGroup::new("Material.c4g");
    for (name, bytes) in files {
        packed_group
            .add_file(*name, bytes.to_vec())
            .unwrap_or_else(|error| panic!("native parity material {name} adds: {error}"));
    }
    let packed = packed_group
        .pack_raw()
        .expect("native parity material group packs");
    let group = Group::from_raw_memory(PathBuf::from("Material.c4g"), packed)
        .expect("native parity material group reopens");
    let library = MaterialLibrary::from_group(&group)
        .expect("native parity material group compiles as C4MaterialCore files");
    MaterialSet::from_resource_library(&library)
}

fn i(v: &Value, key: &str) -> i64 {
    v.get(key)
        .and_then(Value::as_i64)
        .unwrap_or_else(|| panic!("golden entry missing integer field `{key}`: {v}"))
}

fn u(v: &Value, key: &str) -> u64 {
    v.get(key)
        .and_then(Value::as_u64)
        .unwrap_or_else(|| panic!("golden entry missing unsigned field `{key}`: {v}"))
}

fn register_real_c4_effect_definition(engine: &mut Engine, id: &str, name: &str, source: &str) {
    let mut definition = Definition::from_script(id, name, source)
        .unwrap_or_else(|error| panic!("{id} effect fixture compiles: {error}"));
    // Production resource loading enables this on every real C4Script
    // definition (scenario/core.rs:303-307); the command-DSL proplist
    // convention is intentionally test-fixture-only.
    definition.set_c4_callback_convention(true);
    engine
        .register_definition(definition)
        .unwrap_or_else(|error| panic!("{id} effect fixture registers: {error}"));
}

const EFFECT_LIFECYCLE_PROBE: &str = r#"
static lifecycle_state, lifecycle_count, lifecycle_randoms, lifecycle_receivers;

global func LifecycleReset()
{
  lifecycle_state = lifecycle_count = 0;
  lifecycle_randoms = CreateArray(16);
  lifecycle_receivers = CreateArray(16);
  return 1;
}

global func LifecycleRecord(int code, object receiver)
{
  lifecycle_receivers[lifecycle_count] = !!receiver;
  lifecycle_randoms[lifecycle_count] = Random(17);
  lifecycle_count += 1;
  lifecycle_state = lifecycle_state * 10 + code;
  return 0;
}

global func LifecycleState() { return lifecycle_state; }
global func LifecycleRandoms()
{
  SetLength(lifecycle_randoms, lifecycle_count);
  return lifecycle_randoms;
}
global func LifecycleReceivers()
{
  SetLength(lifecycle_receivers, lifecycle_count);
  return lifecycle_receivers;
}
"#;

#[derive(Clone)]
struct EffectLifecycleCall {
    callback: String,
    args: Vec<ScriptValue>,
}

type EffectLifecycleTrace = Arc<Mutex<Vec<EffectLifecycleCall>>>;

fn register_effect_lifecycle_definition(
    engine: &mut Engine,
    id: &str,
    name: &str,
    body: &str,
    trace: &EffectLifecycleTrace,
) {
    let source = format!("#strict 3\n{EFFECT_LIFECYCLE_PROBE}\n{body}");
    let mut definition = Definition::from_script(id, name, &source)
        .unwrap_or_else(|error| panic!("{id} effect lifecycle fixture compiles: {error}"));
    definition.set_c4_callback_convention(true);
    let observed = Arc::clone(trace);
    definition.set_debugger_hooks(clonk_script::DebuggerHooks::new().with_on_call(
        move |callback, args| {
            if callback.starts_with("Fx")
                && ["Start", "Timer", "Stop", "Effect", "Add", "Damage"]
                    .iter()
                    .any(|suffix| callback.ends_with(suffix))
            {
                observed
                    .lock()
                    .unwrap_or_else(|poisoned| poisoned.into_inner())
                    .push(EffectLifecycleCall {
                        callback: callback.to_owned(),
                        args: args.to_vec(),
                    });
            }
        },
    ));
    engine
        .register_definition(definition)
        .unwrap_or_else(|error| panic!("{id} effect lifecycle fixture registers: {error}"));
}

fn effect_lifecycle_i32(value: ScriptValue, field: &str) -> i32 {
    match value {
        ScriptValue::Int(value) => value,
        ScriptValue::Bool(value) => i32::from(value),
        ScriptValue::RawBool(value) => i32::from(value != 0),
        ScriptValue::Nil => 0,
        value => panic!("effect lifecycle `{field}` has unexpected value {value:?}"),
    }
}

fn effect_lifecycle_i32_array(value: ScriptValue, field: &str) -> Vec<i32> {
    match value {
        ScriptValue::Array(values) => values
            .into_iter()
            .map(|value| effect_lifecycle_i32(value, field))
            .collect(),
        value => panic!("effect lifecycle `{field}` has unexpected value {value:?}"),
    }
}

fn effect_lifecycle_arg(value: &ScriptValue) -> Value {
    match value {
        ScriptValue::Int(value) => Value::from(*value),
        ScriptValue::Bool(value) => Value::from(i32::from(*value)),
        ScriptValue::RawBool(value) => Value::from(i32::from(*value != 0)),
        ScriptValue::String(value) => Value::from(value.to_string()),
        ScriptValue::C4Id(value) => Value::from(value.clone()),
        ScriptValue::Object(_) | ScriptValue::Proplist(_) => Value::from("object"),
        ScriptValue::Nil => Value::Null,
        ScriptValue::Array(value) => {
            panic!("effect lifecycle callback has array argument {value:?}")
        }
    }
}

fn effect_lifecycle_effects(effects: &[crate::effect::EffectState]) -> Value {
    Value::Array(
        effects
            .iter()
            .map(|effect| {
                serde_json::json!({
                    "name": effect.name,
                    "number": effect.number,
                    "priority": effect.priority,
                    "time": effect.timer,
                    "interval": effect.interval,
                })
            })
            .collect(),
    )
}

fn effect_lifecycle_state(engine: &mut Engine, function: &str) -> ScriptValue {
    engine
        .call_engine_global_function(function, &[])
        .unwrap_or_else(|error| panic!("effect lifecycle `{function}` reads: {error}"))
}

fn finish_effect_lifecycle_case(
    case: &str,
    seed: u32,
    result: i32,
    effects: &[crate::effect::EffectState],
    engine: &mut Engine,
    trace: &EffectLifecycleTrace,
) -> Value {
    let state = effect_lifecycle_i32(
        effect_lifecycle_state(engine, "LifecycleState"),
        "callback state",
    );
    let randoms = effect_lifecycle_i32_array(
        effect_lifecycle_state(engine, "LifecycleRandoms"),
        "callback randoms",
    );
    let receivers = effect_lifecycle_i32_array(
        effect_lifecycle_state(engine, "LifecycleReceivers"),
        "callback receivers",
    );
    let calls = trace
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
        .clone();
    assert_eq!(
        calls.len(),
        randoms.len(),
        "effect lifecycle `{case}` records one Random draw per callback"
    );
    assert_eq!(
        calls.len(),
        receivers.len(),
        "effect lifecycle `{case}` records every callback receiver"
    );
    let trace = calls
        .into_iter()
        .zip(randoms)
        .zip(receivers)
        .map(|((call, random), receiver)| {
            serde_json::json!({
                "callback": call.callback,
                "receiver": if receiver == 0 { "nil" } else { "command" },
                "args": call.args.iter().map(effect_lifecycle_arg).collect::<Vec<_>>(),
                "random": random,
            })
        })
        .collect::<Vec<_>>();
    serde_json::json!({
        "case": case,
        "seed": seed,
        "result": result,
        "effects": effect_lifecycle_effects(effects),
        "trace": trace,
        "state": state,
        "random_count": engine.rng.count,
        "random_hold": engine.rng.hold,
    })
}

fn effect_lifecycle_entry(
    name: &str,
    number: i32,
    priority: i32,
    interval: i32,
    command_target: Option<i32>,
    command_id: Option<&str>,
) -> crate::effect::EffectState {
    let mut effect = crate::effect::EffectState::new(name)
        .with_priority(priority)
        .with_interval(interval)
        .with_command_target(command_target)
        .with_command_id(command_id);
    effect.number = number;
    effect.start_dispatched = true;
    effect
}

fn run_effect_lifecycle_case(case: &str, seed: u32) -> Value {
    let trace = EffectLifecycleTrace::default();
    match case {
        "object_start_timer_kill" => {
            let mut engine = Engine::new();
            register_effect_lifecycle_definition(
                &mut engine,
                "ELOA",
                "Object effect lifecycle",
                r#"
public func RunObject()
{
  LifecycleReset();
  return AddEffect("Object", this(), 100, 1, nil, ELOA, 11, 12, 13, 14);
}
func FxObjectStart(object target, int number, int temp, int a, int b, int c, int d)
{
  LifecycleRecord(1, this());
  return 0;
}
func FxObjectTimer(object target, int number, int time)
{
  LifecycleRecord(2, this());
  return -1;
}
func FxObjectStop(object target, int number)
{
  LifecycleRecord(3, this());
  return 0;
}
"#,
                &trace,
            );
            let object = engine
                .spawn_object(SpawnConfig::new("ELOA"))
                .expect("object lifecycle carrier spawns");
            let index = engine
                .find_object_index(object)
                .expect("object lifecycle carrier exists");
            engine.rng = LcgRng::new(seed);
            let result = effect_lifecycle_i32(
                engine
                    .call_object_function(index, "RunObject", Vec::new())
                    .expect("object lifecycle effect starts"),
                "object result",
            );
            engine
                .tick_without_snapshot()
                .expect("object lifecycle timer kills the effect");
            let effects = engine.objects[index].state.effects.clone();
            finish_effect_lifecycle_case(case, seed, result, &effects, &mut engine, &trace)
        }
        "global_start_timer" => {
            let mut engine = Engine::new();
            register_effect_lifecycle_definition(
                &mut engine,
                "ELOB",
                "Global effect lifecycle",
                r#"
public func RunGlobal()
{
  LifecycleReset();
  return AddEffect("Global", nil, 100, 1, nil, nil, 21, 22, 23, 24);
}
global func FxGlobalStart(object target, int number, int temp, int a, int b, int c, int d)
{
  LifecycleRecord(1, this());
  return 0;
}
global func FxGlobalTimer(object target, int number, int time)
{
  LifecycleRecord(2, this());
  return 0;
}
"#,
                &trace,
            );
            let caller = engine
                .spawn_object(SpawnConfig::new("ELOB"))
                .expect("global lifecycle caller spawns");
            let index = engine
                .find_object_index(caller)
                .expect("global lifecycle caller exists");
            engine.rng = LcgRng::new(seed);
            let result = effect_lifecycle_i32(
                engine
                    .call_object_function(index, "RunGlobal", Vec::new())
                    .expect("global lifecycle effect starts"),
                "global result",
            );
            engine
                .tick_without_snapshot()
                .expect("global lifecycle timer runs");
            let effects = engine.global_effects().to_vec();
            finish_effect_lifecycle_case(case, seed, result, &effects, &mut engine, &trace)
        }
        "start_deny_reserves_number" => {
            let mut engine = Engine::new();
            register_effect_lifecycle_definition(
                &mut engine,
                "ELOC",
                "Denied effect number lifecycle",
                r#"
public func RunDenied()
{
  LifecycleReset();
  var denied = AddEffect("Denied", this(), 100, 0, nil, ELOC);
  var survivor = AddEffect("Survivor", this(), 50, 0, nil, nil);
  return denied * 10 + survivor;
}
func FxDeniedStart(object target, int number, int temp, a, b, c, d)
{
  LifecycleRecord(1, this());
  return -1;
}
"#,
                &trace,
            );
            let object = engine
                .spawn_object(SpawnConfig::new("ELOC"))
                .expect("denied lifecycle carrier spawns");
            let index = engine
                .find_object_index(object)
                .expect("denied lifecycle carrier exists");
            engine.rng = LcgRng::new(seed);
            let result = effect_lifecycle_i32(
                engine
                    .call_object_function(index, "RunDenied", Vec::new())
                    .expect("denied lifecycle effects construct"),
                "denied result",
            );
            let effects = engine.objects[index].state.effects.clone();
            finish_effect_lifecycle_case(case, seed, result, &effects, &mut engine, &trace)
        }
        "add_annul_temp_bracket" => {
            let mut engine = Engine::new();
            register_effect_lifecycle_definition(
                &mut engine,
                "ELOD",
                "Annulled effect lifecycle",
                r#"
public func RunAnnul()
{
  LifecycleReset();
  return AddEffect("New", this(), 20, 35, nil, ELOD, 31, 32, 33, 34);
}
func FxAbsorberEffect(string name, object target, int number, unused, int a, int b, int c, int d)
{
  LifecycleRecord(4, this());
  return -3;
}
func FxUpperEffect(string name, object target, int number, unused, int a, int b, int c, int d)
{
  LifecycleRecord(4, this());
  return 0;
}
func FxUpperStop(object target, int number, int reason, bool temp)
{
  LifecycleRecord(3, this());
  return 0;
}
func FxAbsorberAdd(object target, int number, string name, int interval, int a, int b, int c, int d, unused)
{
  LifecycleRecord(5, this());
  return 0;
}
func FxUpperStart(object target, int number, int temp)
{
  LifecycleRecord(1, this());
  return 0;
}
"#,
                &trace,
            );
            let object = engine
                .spawn_object(SpawnConfig::new("ELOD"))
                .expect("annul lifecycle carrier spawns");
            let index = engine
                .find_object_index(object)
                .expect("annul lifecycle carrier exists");
            engine.objects[index].state.effects = vec![
                effect_lifecycle_entry("Absorber", 1, 50, 0, None, Some("ELOD")),
                effect_lifecycle_entry("Upper", 2, 200, 0, None, Some("ELOD")),
            ];
            engine.rng = LcgRng::new(seed);
            let result = effect_lifecycle_i32(
                engine
                    .call_object_function(index, "RunAnnul", Vec::new())
                    .expect("annul lifecycle negotiation runs"),
                "annul result",
            );
            let effects = engine.objects[index].state.effects.clone();
            finish_effect_lifecycle_case(case, seed, result, &effects, &mut engine, &trace)
        }
        "clear_all_tail_first_stop_deny" => {
            let mut engine = Engine::new();
            register_effect_lifecycle_definition(
                &mut engine,
                "ELOE",
                "Global ClearAll lifecycle",
                r#"
global func FxUpperStop(object target, int number, int reason)
{
  LifecycleRecord(3, this());
  ChangeEffect("Lower", target, 0, "Renamed", 0);
  AddEffect("Added", target, 150, 0);
  return -1;
}
global func FxLowerStop(object target, int number, int reason)
{
  LifecycleRecord(9, this());
  return 0;
}
global func FxRenamedStop(object target, int number, int reason)
{
  LifecycleRecord(7, this());
  return 0;
}
"#,
                &trace,
            );
            effect_lifecycle_state(&mut engine, "LifecycleReset");
            engine.global_effects = vec![
                effect_lifecycle_entry("Lower", 1, 100, 0, None, None),
                effect_lifecycle_entry("Upper", 2, 200, 0, None, None),
            ];
            engine.rng = LcgRng::new(seed);
            engine
                .clear_global_effects_for_scenario_section()
                .expect("global ClearAll lifecycle runs");
            let effects = engine.global_effects().to_vec();
            finish_effect_lifecycle_case(case, seed, 0, &effects, &mut engine, &trace)
        }
        "negative_priority_one_barrier" => {
            let mut engine = Engine::new();
            register_effect_lifecycle_definition(
                &mut engine,
                "ELOH",
                "Negative effect priority lifecycle",
                r#"
public func RunNegative()
{
  LifecycleReset();
  return AddEffect("Negative", this(), -200, 0, nil, ELOH);
}
func FxUpperStop(object target, int number, int reason, bool temp)
{
  LifecycleRecord(1, this());
  return 0;
}
func FxNegativeStart(object target, int number, int temp, a, b, c, d)
{
  LifecycleRecord(2, this());
  return 0;
}
func FxUpperStart(object target, int number, int temp)
{
  LifecycleRecord(3, this());
  return 0;
}
"#,
                &trace,
            );
            let object = engine
                .spawn_object(SpawnConfig::new("ELOH"))
                .expect("negative-priority lifecycle carrier spawns");
            let index = engine
                .find_object_index(object)
                .expect("negative-priority lifecycle carrier exists");
            engine.objects[index].state.effects = vec![
                effect_lifecycle_entry("One", 1, 1, 0, None, Some("ELOH")),
                effect_lifecycle_entry("Upper", 2, 100, 0, None, Some("ELOH")),
            ];
            engine.rng = LcgRng::new(seed);
            let result = effect_lifecycle_i32(
                engine
                    .call_object_function(index, "RunNegative", Vec::new())
                    .expect("negative-priority effect constructs"),
                "negative-priority result",
            );
            let effects = engine.objects[index].state.effects.clone();
            finish_effect_lifecycle_case(case, seed, result, &effects, &mut engine, &trace)
        }
        "temp_remove_killed_suspended_frame" => {
            let mut engine = Engine::new();
            register_effect_lifecycle_definition(
                &mut engine,
                "ELOK",
                "Suspended temp-removal lifecycle",
                r#"
public func RunTempRemoval()
{
  LifecycleReset();
  return CheckEffect("Pending", this(), 50, 0);
}
func FxAnchorEffect(string name, object target, int number, unused, a, b, c, d)
{
  LifecycleRecord(4, this());
  return -3;
}
func FxHighestStop(object target, int number, int reason, bool temp)
{
  LifecycleRecord(1, this());
  RemoveEffect("Suspended", target, 0, true);
  return 0;
}
func FxSuspendedStop(object target, int number, int reason, bool temp)
{
  LifecycleRecord(2, this());
  return 0;
}
func FxHighestStart(object target, int number, int temp)
{
  LifecycleRecord(3, this());
  return 0;
}
"#,
                &trace,
            );
            let object = engine
                .spawn_object(SpawnConfig::new("ELOK"))
                .expect("suspended-temp lifecycle carrier spawns");
            let index = engine
                .find_object_index(object)
                .expect("suspended-temp lifecycle carrier exists");
            engine.objects[index].state.effects = vec![
                effect_lifecycle_entry("Anchor", 1, 100, 0, None, Some("ELOK")),
                effect_lifecycle_entry("Suspended", 2, 200, 0, None, Some("ELOK")),
                effect_lifecycle_entry("Highest", 3, 300, 0, None, Some("ELOK")),
            ];
            engine.rng = LcgRng::new(seed);
            let result = effect_lifecycle_i32(
                engine
                    .call_object_function(index, "RunTempRemoval", Vec::new())
                    .expect("suspended temp-removal recursion runs"),
                "suspended temp-removal result",
            );
            let effects = engine.objects[index].state.effects.clone();
            finish_effect_lifecycle_case(case, seed, result, &effects, &mut engine, &trace)
        }
        "damage_live_mutation" => {
            let mut engine = Engine::new();
            register_effect_lifecycle_definition(
                &mut engine,
                "ELOF",
                "Damage effect lifecycle",
                r#"
func FxFirstDamage(object target, int number, int change, int cause, int caused_by)
{
  LifecycleRecord(6, this());
  RemoveEffect("Victim", target, 0, true);
  AddEffect("Replacement", target, 150, 0, nil, ELOF);
  return change + 1;
}
func FxReplacementDamage(object target, int number, int change, int cause, int caused_by)
{
  LifecycleRecord(6, this());
  return change + 2;
}
"#,
                &trace,
            );
            let object = engine
                .spawn_object(SpawnConfig::new("ELOF"))
                .expect("damage lifecycle carrier spawns");
            let index = engine
                .find_object_index(object)
                .expect("damage lifecycle carrier exists");
            engine.objects[index].state.effects = vec![
                effect_lifecycle_entry("First", 1, 100, 0, None, Some("ELOF")),
                effect_lifecycle_entry("Victim", 2, 200, 0, None, Some("ELOF")),
            ];
            effect_lifecycle_state(&mut engine, "LifecycleReset");
            engine.rng = LcgRng::new(seed);
            engine
                .change_object_damage(index, 10, 0, OWNER_NONE)
                .expect("damage lifecycle chain runs");
            let result = engine.objects[index].state.damage;
            let effects = engine.objects[index].state.effects.clone();
            finish_effect_lifecycle_case(case, seed, result, &effects, &mut engine, &trace)
        }
        "command_target_lost_silent" => {
            let mut engine = Engine::new();
            register_effect_lifecycle_definition(
                &mut engine,
                "ELOG",
                "Lost command target lifecycle",
                r#"
public func Drop(object target)
{
  RemoveObject(target);
  return 0;
}
func FxCommandedTimer(object target, int number, int time)
{
  LifecycleRecord(2, this());
  return 0;
}
"#,
                &trace,
            );
            let carrier = engine
                .spawn_object(SpawnConfig::new("ELOG"))
                .expect("command-target lifecycle carrier spawns");
            let command_target = engine
                .spawn_object(SpawnConfig::new("ELOG"))
                .expect("effect command target spawns");
            let carrier_index = engine
                .find_object_index(carrier)
                .expect("command-target lifecycle carrier exists");
            let command_target_number = i32::try_from(command_target.as_u64())
                .expect("effect command target number fits C4Object pointer slot");
            engine.objects[carrier_index].state.effects = vec![effect_lifecycle_entry(
                "Commanded",
                1,
                100,
                0,
                Some(command_target_number),
                Some("ELOG"),
            )];
            effect_lifecycle_state(&mut engine, "LifecycleReset");
            engine.rng = LcgRng::new(seed);
            let result = effect_lifecycle_i32(
                engine
                    .call_object_function(
                        carrier_index,
                        "Drop",
                        vec![ScriptValue::Object(command_target.as_u64())],
                    )
                    .expect("command-target removal runs"),
                "command-target result",
            );
            let carrier_index = engine
                .find_object_index(carrier)
                .expect("command-target lifecycle carrier remains");
            let effects = engine.objects[carrier_index].state.effects.clone();
            finish_effect_lifecycle_case(case, seed, result, &effects, &mut engine, &trace)
        }
        "timer_error_is_nil_after_side_effects" => {
            let mut engine = Engine::new();
            register_effect_lifecycle_definition(
                &mut engine,
                "ELOI",
                "Effect callback error lifecycle",
                r#"
func FxErrorTimer(object target, int number, int time)
{
  LifecycleRecord(8, this());
  FatalError("effect lifecycle timer failure");
  return 0;
}
"#,
                &trace,
            );
            let object = engine
                .spawn_object(SpawnConfig::new("ELOI"))
                .expect("effect-error lifecycle carrier spawns");
            let index = engine
                .find_object_index(object)
                .expect("effect-error lifecycle carrier exists");
            engine.objects[index].state.effects = vec![effect_lifecycle_entry(
                "Error",
                1,
                100,
                1,
                None,
                Some("ELOI"),
            )];
            effect_lifecycle_state(&mut engine, "LifecycleReset");
            engine.rng = LcgRng::new(seed);
            engine
                .tick_without_snapshot()
                .expect("effect timer errors are fail-safe");
            let result = engine.objects[index].state.effects[0].priority;
            let effects = engine.objects[index].state.effects.clone();
            finish_effect_lifecycle_case(case, seed, result, &effects, &mut engine, &trace)
        }
        "object_command_target_start" => {
            let mut engine = Engine::new();
            register_effect_lifecycle_definition(
                &mut engine,
                "ELOJ",
                "Object-commanded effect lifecycle",
                r#"
local command_state;

public func RunCommand(object command_target)
{
  LifecycleReset();
  return AddEffect("Command", this(), 100, 0, command_target, nil, 41, 42, 43, 44);
}
public func CommandState() { return command_state; }
func FxCommandStart(object target, int number, int temp, int a, int b, int c, int d)
{
  LifecycleRecord(1, this());
  command_state = 77;
  return 0;
}
"#,
                &trace,
            );
            let carrier = engine
                .spawn_object(SpawnConfig::new("ELOJ"))
                .expect("object-command lifecycle carrier spawns");
            let command_target = engine
                .spawn_object(SpawnConfig::new("ELOJ"))
                .expect("object-command lifecycle receiver spawns");
            let carrier_index = engine
                .find_object_index(carrier)
                .expect("object-command lifecycle carrier exists");
            let command_index = engine
                .find_object_index(command_target)
                .expect("object-command lifecycle receiver exists");
            engine.rng = LcgRng::new(seed);
            engine
                .call_object_function(
                    carrier_index,
                    "RunCommand",
                    vec![ScriptValue::Object(command_target.as_u64())],
                )
                .expect("object-commanded effect starts");
            let result = effect_lifecycle_i32(
                engine
                    .call_object_function(command_index, "CommandState", Vec::new())
                    .expect("command target mutation reads"),
                "object-command result",
            );
            let effects = engine.objects[carrier_index].state.effects.clone();
            finish_effect_lifecycle_case(case, seed, result, &effects, &mut engine, &trace)
        }
        other => panic!("unhandled effect_lifecycle case `{other}`"),
    }
}

/// Assert two values are equal, panicking with a precise first-divergence report.
fn expect_eq(section: &str, index: usize, field: &str, cpp: i64, rust: i64) {
    if cpp != rust {
        write_parity_diff_from_environment(
            section,
            index,
            field,
            serde_json::json!(cpp),
            serde_json::json!(rust),
        );
    }
    assert_eq!(
        cpp, rust,
        "PARITY DIVERGENCE in `{section}` entry {index} field `{field}`: \
         C++ golden = {cpp}, Rust = {rust}"
    );
}

fn expect_eq_u64(section: &str, index: usize, field: &str, cpp: u64, rust: u64) {
    if cpp != rust {
        write_parity_diff_from_environment(
            section,
            index,
            field,
            serde_json::json!(cpp),
            serde_json::json!(rust),
        );
    }
    assert_eq!(
        cpp, rust,
        "PARITY DIVERGENCE in `{section}` entry {index} field `{field}`: \
         C++ golden = {cpp}, Rust = {rust}"
    );
}

fn expect_rng_state(section: &str, case: &Value, rng: &LcgRng) {
    expect_rng_state_at(section, 0, case, rng);
}

fn expect_rng_state_at(section: &str, index: usize, case: &Value, rng: &LcgRng) {
    expect_eq(
        section,
        index,
        "random_count",
        i(case, "random_count"),
        i64::from(rng.count),
    );
    expect_eq_u64(
        section,
        index,
        "random_hold",
        u(case, "random_hold"),
        u64::from(rng.hold),
    );
}

fn register_smoke_probe(engine: &mut Engine) {
    engine
        .register_particle_definition(
            crate::particles::ParticleDefCore {
                name: "Smoke".into(),
                init_fn: "SmokeInit".into(),
                exec_fn: "SmokeExec".into(),
                draw_fn: "Smoke".into(),
                min_lifetime: 10,
                max_lifetime: 10,
                ..Default::default()
            },
            4,
            1.0,
        )
        .expect("parity Smoke particle definition registers");
}

fn smoke_probe_count(engine: &Engine) -> i64 {
    engine
        .particle_system()
        .particles()
        .iter()
        .filter(|particle| particle.def_name == "Smoke")
        .count() as i64
}

fn landscape_material_snapshot(
    engine: &Engine,
    width: u32,
    height: u32,
) -> Vec<Option<crate::material::MaterialId>> {
    let landscape = engine
        .landscape()
        .expect("parity landscape remains available");
    (0..height as i32)
        .flat_map(|y| (0..width as i32).map(move |x| (x, y)))
        .map(|(x, y)| landscape.material_at(x, y))
        .collect()
}

fn landscape_material_changes(
    before: &[Option<crate::material::MaterialId>],
    engine: &Engine,
    width: u32,
    height: u32,
) -> Vec<(
    i32,
    i32,
    Option<crate::material::MaterialId>,
    Option<crate::material::MaterialId>,
)> {
    let landscape = engine
        .landscape()
        .expect("parity landscape remains available");
    (0..height as i32)
        .flat_map(|y| (0..width as i32).map(move |x| (x, y)))
        .enumerate()
        .filter_map(|(index, (x, y))| {
            let after = landscape.material_at(x, y);
            (before[index] != after).then_some((x, y, before[index], after))
        })
        .collect()
}

fn clear_instability_probe_trace() {
    crate::mass_mover::MASS_MOVER_INSTABILITY_PROBES.with(|probes| probes.borrow_mut().clear());
}

fn take_instability_probe_trace() -> Vec<(i32, i32, Option<crate::material::MaterialId>)> {
    crate::mass_mover::MASS_MOVER_INSTABILITY_PROBES
        .with(|probes| std::mem::take(&mut *probes.borrow_mut()))
}

/// The 8x40 material grid `parity/oracle/oracle_main.cpp`'s `splash_effect`
/// scaffolds, and the `SplashHost` over it: water (liquid and instable), a
/// liquid that is NOT instable, and granite.
struct SplashProbe {
    grid: [[i32; SplashProbe::WIDTH as usize]; SplashProbe::HEIGHT as usize],
    rng: LcgRng,
    bubbles: Vec<[i32; 2]>,
    casts: Vec<[i32; 5]>,
    extractions: i64,
}

impl SplashProbe {
    const WIDTH: i32 = 8;
    const HEIGHT: i32 = 40;
    const MAP: [(i32, bool); 3] = [(25, true), (25, false), (50, false)];

    /// Water at or below `water_top`, granite at or below `floor_top`, sky
    /// above, using material `liquid_mat` for the water body.
    fn new(water_top: i32, floor_top: i32, liquid_mat: i32) -> Self {
        let mut grid = [[-1; Self::WIDTH as usize]; Self::HEIGHT as usize];
        for (y, row) in grid.iter_mut().enumerate() {
            let y = y as i32;
            row.fill(if y >= floor_top {
                2
            } else if y >= water_top {
                liquid_mat
            } else {
                -1
            });
        }
        Self {
            grid,
            rng: LcgRng::new(0),
            bubbles: Vec::new(),
            casts: Vec::new(),
            extractions: 0,
        }
    }

    fn water_column(water_top: i32) -> Self {
        Self::new(water_top, Self::HEIGHT, 0)
    }

    fn material(&self, x: i32, y: i32) -> Option<usize> {
        (0..Self::WIDTH).contains(&x).then_some(())?;
        (0..Self::HEIGHT).contains(&y).then_some(())?;
        usize::try_from(self.grid[y as usize][x as usize]).ok()
    }

    fn density(&self, x: i32, y: i32) -> i32 {
        self.material(x, y).map_or(0, |mat| Self::MAP[mat].0)
    }
}

impl crate::engine_splash::SplashHost for SplashProbe {
    type Error = std::convert::Infallible;

    fn splash_is_semi_solid(&self, x: i32, y: i32) -> bool {
        self.density(x, y) >= 25
    }

    fn splash_material_is_liquid(&self, x: i32, y: i32) -> bool {
        self.material(x, y)
            .map(|mat| Self::MAP[mat])
            .is_some_and(|(density, instable)| (25..50).contains(&density) && instable)
    }

    fn splash_is_liquid(&self, x: i32, y: i32) -> bool {
        (25..50).contains(&self.density(x, y))
    }

    fn splash_random(&mut self, upper_bound: i32) -> Result<i32, Self::Error> {
        Ok(self.rng.random(upper_bound))
    }

    fn splash_bubble_out(&mut self, x: i32, y: i32) -> Result<(), Self::Error> {
        self.bubbles.push([x, y]);
        Ok(())
    }

    /// C++ hands `PXS::Create` whatever `ExtractMaterial` returned, and
    /// `Create` drops an invalid material (C4PXS.cpp:210) — so the extraction
    /// is counted either way and only a real material casts.
    fn splash_extract_and_cast(
        &mut self,
        source: crate::Vector2,
        destination: crate::Vector2,
        velocity: FixedVec2,
    ) -> Result<(), Self::Error> {
        self.extractions += 1;
        let Some(material) = self
            .material(source.x, source.y)
            .filter(|mat| (25..50).contains(&Self::MAP[*mat].0))
        else {
            return Ok(());
        };
        self.grid[source.y as usize][source.x as usize] = -1;
        self.casts.push([
            material as i32,
            destination.x,
            destination.y,
            fixtoi_prec(velocity.x, 100),
            fixtoi_prec(velocity.y, 100),
        ]);
        Ok(())
    }
}

/// The 24x16 landscape `parity/oracle/oracle_main.cpp`'s `shape_contact`
/// scaffolds: sky above y=10, earth below, a water pocket at x=3..5 and a
/// pillar at x=17..18, with the border configuration under test. Installing it
/// on the engine is what resolves the grid's material names.
fn install_contact_oracle_landscape(
    engine: &mut Engine,
    left_open: i32,
    right_open: i32,
    top_open: bool,
    bottom_open: bool,
) {
    const WIDTH: u32 = 24;
    const HEIGHT: i32 = 16;

    let mut bytes = vec![0u8; WIDTH as usize * HEIGHT as usize];
    for y in 0..HEIGHT {
        for x in 0..WIDTH as i32 {
            let mut byte = u8::from(y >= 10);
            if y >= 11 && (3..=5).contains(&x) {
                byte = 2;
            }
            if (17..=18).contains(&x) && y >= 6 {
                byte = 1;
            }
            bytes[y as usize * WIDTH as usize + x as usize] = byte;
        }
    }
    let mut densities = vec![0; 128];
    densities[1] = 50;
    densities[2] = 30;
    let mut material_names = vec![None; 128];
    material_names[1] = Some("Earth".to_string());
    material_names[2] = Some("Water".to_string());

    let mut landscape = Landscape::flat(WIDTH, HEIGHT);
    landscape.set_pixel_grid(PixelGrid::new(
        WIDTH,
        HEIGHT as u32,
        bytes,
        densities,
        material_names,
        vec![None; 128],
    ));
    landscape.set_border_open(left_open, right_open, top_open, bottom_open);
    let vehicle = engine
        .materials
        .id_of("Vehicle")
        .expect("the fixture declares Vehicle");
    landscape.set_vehicle_material(Some(vehicle));
    engine.set_landscape(landscape);
}

/// The material library the `shape_contact` grid's bytes map onto.
/// The bytes a packed parent stores for its `RawChild.c4g` entry.
///
/// Read back through a fresh open, so the comparison is against what the
/// container actually holds rather than against the standalone file it was
/// built from — a parent may pack a child differently from how it sat on disk,
/// and the claim under test is only that a *rewrite* leaves those stored bytes
/// alone.
fn raw_child_bytes(packed: &[u8]) -> Vec<u8> {
    clonk_resources::Group::from_top_level_memory("RawParent.c4g".into(), packed.to_vec())
        .ok()
        .and_then(|group| group.read_file("RawChild.c4g").ok())
        .unwrap_or_default()
}

fn contact_oracle_materials() -> clonk_resources::MaterialLibrary {
    clonk_resources::MaterialLibrary::parse(
        r#"
        [Material Earth]
        Name=Earth
        Density=50

        [Material Water]
        Name=Water
        Density=30

        [Material Vehicle]
        Name=Vehicle
        Density=100
        "#,
    )
    .expect("contact oracle materials parse")
}

fn expect_json_eq(section: &str, index: usize, field: &str, cpp: Value, rust: Value) {
    if cpp != rust {
        write_parity_diff_from_environment(section, index, field, cpp.clone(), rust.clone());
    }
    assert_eq!(
        cpp, rust,
        "PARITY DIVERGENCE in `{section}` entry {index} field `{field}`: \
         C++ golden = {cpp}, Rust = {rust}"
    );
}

fn write_parity_diff_from_environment(
    section: &str,
    index: usize,
    field: &str,
    cpp: Value,
    rust: Value,
) {
    let directory = std::env::var_os("LC_TEST_ARTIFACT_DIR")
        .or_else(|| std::env::var_os("LC_DEV_CHECK_ARTIFACT_DIR"));
    let Some(directory) = directory else {
        return;
    };
    match write_parity_diff_artifact(
        std::path::Path::new(&directory),
        section,
        index,
        field,
        cpp,
        rust,
    ) {
        Ok(path) => eprintln!("C++/Rust parity diff: {}", path.display()),
        Err(error) => eprintln!("failed to preserve C++/Rust parity diff: {error}"),
    }
}

fn write_parity_diff_artifact(
    directory: &std::path::Path,
    section: &str,
    index: usize,
    field: &str,
    cpp: Value,
    rust: Value,
) -> std::io::Result<std::path::PathBuf> {
    std::fs::create_dir_all(directory)?;
    let path = directory.join("cpp-rust-diff.json");
    let artifact = serde_json::json!({
        "schema": "legacyclonk.cpp-rust-diff.v1",
        "section": section,
        "entry": index,
        "field": field,
        "cpp": cpp,
        "rust": rust,
        "golden": "parity/golden/parity_golden.json",
        "reproduce": "cargo xtask parity verify",
    });
    let temporary = directory.join(format!(".cpp-rust-diff-{}.tmp", std::process::id()));
    let bytes = serde_json::to_vec_pretty(&artifact).map_err(std::io::Error::other)?;
    std::fs::write(&temporary, bytes)?;
    std::fs::rename(&temporary, &path)?;
    Ok(path)
}

#[test]
fn parity_divergence_artifact_is_structured_and_reproducible() {
    let temp = tempfile::tempdir().expect("temporary artifact directory");
    let path = write_parity_diff_artifact(
        temp.path(),
        "movement[gravity]",
        7,
        "fix_y",
        serde_json::json!(65_536),
        serde_json::json!(65_535),
    )
    .expect("parity artifact writes");
    let artifact: Value =
        serde_json::from_reader(std::fs::File::open(path).expect("parity artifact opens"))
            .expect("parity artifact parses");

    assert_eq!(artifact["schema"], "legacyclonk.cpp-rust-diff.v1");
    assert_eq!(artifact["section"], "movement[gravity]");
    assert_eq!(artifact["entry"], 7);
    assert_eq!(artifact["field"], "fix_y");
    assert_eq!(artifact["cpp"], 65_536);
    assert_eq!(artifact["rust"], 65_535);
    assert_eq!(artifact["golden"], "parity/golden/parity_golden.json");
    assert_eq!(artifact["reproduce"], "cargo xtask parity verify");
}

/// Reconstruct the `script_value_convert` source value the oracle emitted for a
/// given case name (must stay in sync with `conv_cases` in oracle_main.cpp).
fn convert_case_value(name: &str) -> ScriptValue {
    match name {
        "nil" => ScriptValue::Nil,
        "int_0" => ScriptValue::Int(0),
        "int_5000" => ScriptValue::Int(5000),
        "int_9999" => ScriptValue::Int(9999),
        "int_10000" => ScriptValue::Int(10000),
        "int_neg1" => ScriptValue::Int(-1),
        "bool_true" => ScriptValue::Bool(true),
        "bool_false" => ScriptValue::Bool(false),
        "id_CLNK" => ScriptValue::C4Id("CLNK".to_string()),
        "string" => ScriptValue::String("x".to_string().into()),
        "array" => ScriptValue::Array(Vec::new()),
        "map" => ScriptValue::Proplist(ValueMap::new()),
        other => panic!("unknown script_value_convert case `{other}`"),
    }
}

#[derive(Debug, PartialEq, Eq)]
struct C4ValueStatefulConversionOutcome {
    ok: bool,
    value: ScriptValue,
    is_ref: bool,
    target: Option<ScriptValue>,
    rng_delta: i32,
}

fn c4value_scalar_payload(value: &ScriptValue) -> i64 {
    match value {
        ScriptValue::Nil => 0,
        ScriptValue::Int(value) => i64::from(*value),
        ScriptValue::Bool(value) => i64::from(*value),
        ScriptValue::RawBool(value) => *value as u64 as i64,
        ScriptValue::C4Id(value) => clonk_script::c4_id_raw(value) as u64 as i64,
        ScriptValue::Object(value) => *value as i64,
        ScriptValue::String(_) | ScriptValue::Array(_) | ScriptValue::Proplist(_) => {
            panic!("stateful C4Value conversion probe produced a pointer-backed payload")
        }
    }
}

fn run_c4value_stateful_conversion(case: &str) -> C4ValueStatefulConversionOutcome {
    let rng = LcgRng::new(0xC4A1_0E00);
    let random_count_before = rng.count;

    let (ok, value, is_ref, target) = match case {
        "set_object_null" => (
            true,
            // Fresh C4Value::SetObject(nullptr) delegates to Set and
            // canonicalizes the zero payload to C4V_Any (C4Value.h:195;
            // C4Value.cpp:121-143).
            ScriptValue::from_c4_object_handle(0),
            false,
            None,
        ),
        "int_zero_to_c4id_then_set_copy" => {
            let converted_observation = Arc::new(std::sync::Mutex::new(None));
            let observed = Arc::clone(&converted_observation);
            let mut script = clonk_script::Engine::new();
            script.register_host_reference_function(
                "ConvertZeroToId",
                std::iter::empty::<usize>(),
                move |args| {
                    let argument = args.first().expect("converted argument is present");
                    let value = argument.read()?;
                    *observed.lock().expect("conversion observation lock") =
                        Some((value.clone(), argument.is_reference()));
                    Ok(ScriptValue::Bool(true))
                },
            );
            assert!(script.set_host_function_parameter_types("ConvertZeroToId", [C4VType::C4Id]));
            script.register_host_function("SetCopy", |args| {
                Ok(args.first().cloned().unwrap_or(ScriptValue::Nil))
            });
            assert!(script.set_host_function_parameter_types("SetCopy", [C4VType::Any]));
            script
                .load_script("#strict 3\nfunc RunZeroConversion() { return ConvertZeroToId(0); }")
                .expect("strict3 conversion driver loads");

            // FnCnvInt2Id writes C4V_C4ID directly, retaining the zero tag
            // (C4Value.cpp:469-478). A second external call initializes a
            // fresh C4Value parameter through Set, which canonicalizes that
            // copied zero payload back to C4V_Any (:121-143).
            let converted_result = script.call("RunZeroConversion", &[]);
            let converted_ok = converted_result.is_ok();
            converted_result.expect("zero int converts to a retained zero C4ID");
            let (observed_value, observed_is_ref) = converted_observation
                .lock()
                .expect("conversion observation lock")
                .clone()
                .expect("typed host body observed the converted value");
            let copied_result = script.call("SetCopy", std::slice::from_ref(&observed_value));
            let copied_ok = copied_result.is_ok();
            let copied = copied_result.expect("ordinary C4Value::Set copy succeeds");
            (
                converted_ok && copied_ok,
                observed_value,
                observed_is_ref,
                Some(copied),
            )
        }
        "reference_int_seven_to_c4id" => {
            let detached_observation = Arc::new(std::sync::Mutex::new(None));
            let observed = Arc::clone(&detached_observation);
            let mut script = clonk_script::Engine::new();
            script.register_host_reference_function(
                "ConvertReferenceToId",
                std::iter::empty::<usize>(),
                move |args| {
                    let argument = args.first().expect("converted argument is present");
                    let value = argument.read()?;
                    *observed.lock().expect("reference observation lock") =
                        Some((value.clone(), argument.is_reference()));
                    Ok(value)
                },
            );
            assert!(
                script.set_host_function_parameter_types("ConvertReferenceToId", [C4VType::C4Id])
            );

            // A plain typed parameter receives a dereferenced copy. FnCnvDeref
            // calls Deref (`Set(GetRefVal())`) and retries conversion, so the
            // converted value is no longer a reference and the source cell
            // remains Int(7) (C4Value.h:221-223; C4Value.cpp:445-451).
            let conversion_result =
                script.call_with_ref_args("ConvertReferenceToId", &[ScriptValue::Int(7)]);
            let conversion_ok = conversion_result.is_ok();
            let (converted, targets) =
                conversion_result.expect("reference conversion detaches and succeeds");
            let (observed_value, observed_is_ref) = detached_observation
                .lock()
                .expect("reference observation lock")
                .clone()
                .expect("typed host body observed the detached value");
            assert_eq!(converted, observed_value);
            (
                conversion_ok,
                converted,
                observed_is_ref,
                Some(targets.into_iter().next().expect("referent is returned")),
            )
        }
        other => panic!("unknown c4value_stateful_conversion case `{other}`"),
    };

    C4ValueStatefulConversionOutcome {
        ok,
        value,
        is_ref,
        target,
        rng_delta: rng.count - random_count_before,
    }
}

#[test]
fn c4value_stateful_conversion_rust_driver_uses_vm_transitions() {
    let set_null = run_c4value_stateful_conversion("set_object_null");
    assert_eq!(set_null.value, ScriptValue::Nil);
    assert_eq!(set_null.target, None);

    let set_copy = run_c4value_stateful_conversion("int_zero_to_c4id_then_set_copy");
    assert_eq!(set_copy.value, ScriptValue::C4Id("NONE".into()));
    assert!(!set_copy.is_ref);
    assert_eq!(set_copy.target, Some(ScriptValue::Nil));

    let deref = run_c4value_stateful_conversion("reference_int_seven_to_c4id");
    assert_eq!(deref.value, ScriptValue::C4Id("0007".into()));
    assert!(!deref.is_ref);
    assert_eq!(deref.target, Some(ScriptValue::Int(7)));
    assert_eq!(deref.rng_delta, 0);
}

/// One `scenario_sections` fixture node. Nothing the sweep reads comes from
/// these fields — it looks at the name, the current-section pointer and
/// `fModified` only — so every other value is the neutral default.
fn parity_scenario_section_spec(name: &str) -> crate::scenario::ScenarioSectionSpec {
    crate::scenario::ScenarioSectionSpec {
        name: name.to_owned(),
        source_group: None,
        landscape: None,
        landscape_systems: crate::scenario::ScenarioLandscapeSystems::default(),
        exact_landscape: false,
        texmap_lookups: Vec::new(),
        resynthesize_static_map: false,
        map_creator: None,
        s2_overload: None,
        gravity: crate::scenario::LegacyC4SVal::new(100, 0, 10, 200),
        post_init_map_callbacks: crate::map_creator_s2::PostInitMapCallbacks::default(),
        keep_map_creator: false,
        no_initialize: false,
        objects: Vec::new(),
        scenario_values: crate::scenario::ScenarioValueStore::default(),
        base_reject_entrance_enabled: true,
        base_extinguish_enabled: true,
        environment: crate::EnvironmentSettings::default(),
    }
}

fn parity_script_bool(value: ScriptValue, field: &str) -> bool {
    match value {
        ScriptValue::Bool(value) => value,
        ScriptValue::RawBool(value) => value != 0,
        ScriptValue::Int(value) => value != 0,
        other => panic!("scenario-section parity `{field}` returned {other:?}"),
    }
}

/// Drive the real Rust C4Script host for the bounded C++
/// `FnLoadScenarioSection` rows. The C++ half extracts only the host boundary
/// and a callback-shaped prefix/suffix; this driver executes an actual
/// `Initialize` callback, so the continuation result is observed through the
/// VM's global state rather than manufactured in the comparator.
fn parity_scenario_section_host_case(case: &Value, case_index: usize) {
    const SECTION: &str = "scenario_section_host_lifecycle";
    let name = case["case"].as_str().unwrap_or("?");
    let target = match name {
        "success_prefix_suffix" => "next",
        "failure_prefix_suffix" => "missing",
        "empty_name" => "",
        other => panic!("{SECTION} has unknown host case `{other}`"),
    };
    let configured = if name == "success_prefix_suffix" {
        vec![
            parity_scenario_section_spec("main"),
            parity_scenario_section_spec("next"),
        ]
    } else {
        vec![parity_scenario_section_spec("main")]
    };

    let observer = r#"#strict 3
static trace, switch_result;

global func ResetScenarioHostTrace()
{
  trace = switch_result = 0;
  return true;
}

global func RecordTrace(int value)
{
  trace = trace * 10 + value;
  return true;
}

global func RecordSwitchResult(bool value)
{
  switch_result = !!value;
  trace = trace * 10 + switch_result;
  return true;
}

global func ReadScenarioHostTrace() { return trace; }
global func ReadScenarioHostResult() { return switch_result; }
"#;
    let callback = format!(
        "#strict 3\nfunc Initialize() {{\n  RecordTrace(1);\n  var switched = LoadScenarioSection(\"{target}\");\n  RecordSwitchResult(switched);\n  RecordTrace(3);\n  return 0;\n}}\n"
    );

    let mut engine = Engine::new();
    engine.configure_scenario_sections(&configured);
    assert_eq!(
        engine
            .install_global_scripts(&[
                ("System.c4g/ScenarioHostParity.c".into(), observer.into(),)
            ]),
        1,
        "{SECTION} row {case_index} installs the observer"
    );
    crate::TestValueExt::test_value(
        engine.call_engine_global_function("ResetScenarioHostTrace", &[]),
    );

    let mut definition = Definition::from_script("HOST", "Scenario host callback", &callback)
        .unwrap_or_else(|error| panic!("HOST fixture compiles: {error}"));
    definition.set_c4_callback_convention(true);
    engine
        .register_definition(definition)
        .unwrap_or_else(|error| panic!("{SECTION} row {case_index} registers callback: {error}"));
    crate::TestValueExt::test_value(
        engine.spawn_object(SpawnConfig::new("HOST").with_id(ObjectId::new(1))),
    );

    let trace = match crate::TestValueExt::test_value(
        engine.call_engine_global_function("ReadScenarioHostTrace", &[]),
    ) {
        ScriptValue::Int(value) => value,
        ScriptValue::Bool(value) => i32::from(value),
        ScriptValue::RawBool(value) => i32::from(value != 0),
        other => panic!("{SECTION} row {case_index} trace is {other:?}"),
    };
    let switch_result = parity_script_bool(
        crate::TestValueExt::test_value(
            engine.call_engine_global_function("ReadScenarioHostResult", &[]),
        ),
        "switch result",
    );
    let rust = serde_json::json!({
        "case": name,
        "return": switch_result,
        "trace": [(trace / 100) % 10, (trace / 10) % 10, trace % 10],
    });
    expect_json_eq(SECTION, case_index, "row", case.clone(), rust);
}

/// Exercise real Engine object creation, the script SetObjectStatus host, and
/// the section loader's active/inactive lists. The C++ row compares only the
/// source-backed Construction count and final list membership. Native
/// C4Object::Init has no honest Rust counter-equivalent, and the extracted
/// DoCon call intentionally remains an abstract C++ stub, so neither is
/// represented as a cross-language golden field. Rust still observes its real
/// Initialize callbacks and asserts that both newly-created objects run them
/// exactly once.
fn parity_scenario_section_lifecycle_case(case: &Value, case_index: usize) {
    const SECTION: &str = "scenario_section_host_lifecycle";
    let observer = r#"#strict 3
static initialize_count, construction_count;

global func ResetLifecycleCounts()
{
  initialize_count = 0;
  construction_count = 0;
  return true;
}

global func RecordInitialize()
{
  initialize_count += 1;
  return true;
}

global func RecordConstruction()
{
  construction_count += 1;
  return true;
}

global func ReadInitializeCount() { return initialize_count; }
global func ReadConstructionCount() { return construction_count; }
"#;
    let mut inactive_definition = Definition::from_script(
        "INAC",
        "Inactive lifecycle object",
        r#"#strict 3
func Construction()
{
  RecordConstruction();
  return 0;
}

func Initialize()
{
  RecordInitialize();
  return 0;
}

public func DeactivateForSection()
{
  return SetObjectStatus(C4OS_INACTIVE, this());
}
"#,
    )
    .unwrap_or_else(|error| panic!("INAC fixture compiles: {error}"));
    inactive_definition.set_c4_callback_convention(true);
    let mut active_definition = Definition::from_script(
        "ACTV",
        "Active lifecycle object",
        r#"#strict 3
func Construction()
{
  RecordConstruction();
  return 0;
}

func Initialize()
{
  RecordInitialize();
  return 0;
}
"#,
    )
    .unwrap_or_else(|error| panic!("ACTV fixture compiles: {error}"));
    active_definition.set_c4_callback_convention(true);

    let mut engine = Engine::new();
    engine.configure_scenario_sections(&[
        parity_scenario_section_spec("main"),
        parity_scenario_section_spec("next"),
    ]);
    assert_eq!(
        engine.install_global_scripts(&[(
            "System.c4g/ScenarioLifecycleParity.c".into(),
            observer.into(),
        )]),
        1,
        "{SECTION} lifecycle row {case_index} installs the observer"
    );
    crate::TestValueExt::test_value(
        engine.call_engine_global_function("ResetLifecycleCounts", &[]),
    );
    engine
        .register_definition(inactive_definition)
        .unwrap_or_else(|error| panic!("{SECTION} lifecycle registers INAC: {error}"));
    engine
        .register_definition(active_definition)
        .unwrap_or_else(|error| panic!("{SECTION} lifecycle registers ACTV: {error}"));

    let inactive = crate::TestValueExt::test_value(
        engine.spawn_object(SpawnConfig::new("INAC").with_id(ObjectId::new(1))),
    );
    let inactive_index = engine
        .find_object_index(inactive)
        .unwrap_or_else(|| panic!("{SECTION} lifecycle inactive object exists"));
    let deactivated = parity_script_bool(
        crate::TestValueExt::test_value(engine.call_object_function(
            inactive_index,
            "DeactivateForSection",
            Vec::new(),
        )),
        "deactivate",
    );
    let active = crate::TestValueExt::test_value(
        engine.spawn_object(SpawnConfig::new("ACTV").with_id(ObjectId::new(2))),
    );
    let active_before = engine
        .objects
        .iter()
        .filter(|object| !object.destroyed && object.state.status == ObjectStatus::Normal)
        .map(|object| object.id.as_u64() as i64)
        .collect::<Vec<_>>();
    let loaded =
        crate::TestValueExt::test_value(engine.load_scenario_section("next", 0, Vec::new()));
    let initialize_count = match crate::TestValueExt::test_value(
        engine.call_engine_global_function("ReadInitializeCount", &[]),
    ) {
        ScriptValue::Int(value) => value,
        other => panic!("{SECTION} lifecycle initialize count is {other:?}"),
    };
    let construction_count = match crate::TestValueExt::test_value(
        engine.call_engine_global_function("ReadConstructionCount", &[]),
    ) {
        ScriptValue::Int(value) => value,
        other => panic!("{SECTION} lifecycle construction count is {other:?}"),
    };
    assert_eq!(
        initialize_count, 2,
        "{SECTION} lifecycle initializes each created object exactly once"
    );
    let live = |status: ObjectStatus| {
        engine
            .objects
            .iter()
            .filter(|object| !object.destroyed && object.state.status == status)
            .map(|object| object.id.as_u64() as i64)
            .collect::<Vec<_>>()
    };
    let inactive_numbers = live(ObjectStatus::Inactive);
    let active_numbers = live(ObjectStatus::Normal);
    let removed_numbers = active_before
        .iter()
        .copied()
        .filter(|id| {
            !engine
                .objects
                .iter()
                .any(|object| !object.destroyed && object.id.as_u64() as i64 == *id)
        })
        .collect::<Vec<_>>();
    let rust = serde_json::json!({
        "case": case["case"].as_str().unwrap_or("?"),
        "deactivated": deactivated,
        "loaded": loaded,
        "construction_count": construction_count,
        "inactive_numbers": inactive_numbers,
        "active_numbers": active_numbers,
        "removed_numbers": removed_numbers,
    });
    expect_json_eq(SECTION, case_index, "row", case.clone(), rust);

    // Keep the explicit active handle in the setup: this makes it clear the
    // second object was created before the synchronous switch, while all
    // observations above come from Engine state after the switch.
    let _ = active;
}

/// Stand-in for the temporary group `C4ScenarioSection::EnsureTempStore`
/// leaves behind, which is what C++ hands to `C4Group::Add`. Only the entry
/// name is compared, so the payload just has to be a loadable image.
fn parity_frozen_section_image(name: &str) -> Vec<u8> {
    let mut group = MutableGroup::new(format!("Sect{name}.c4g"));
    group
        .add_file("Objects.txt", name.as_bytes().to_vec())
        .expect("compose frozen scenario section");
    group.pack_raw().expect("pack frozen scenario section")
}

#[derive(Debug, PartialEq, Eq)]
struct C4ValueDenumerationOutcome {
    value: ScriptValue,
    rng_delta: i32,
}

/// Parse and denumerate one saved C4Value after both active and inactive
/// object lists exist. Native `C4Value::DenumeratePointer` searches those
/// lists in that order and handles explicit object tags differently from old
/// untyped pointer words (C4Value.cpp:684-715).
fn run_c4value_denumeration(encoded: &str) -> C4ValueDenumerationOutcome {
    let rng = LcgRng::new(0xC4A1_0E00);
    let random_count_before = rng.count;
    // 7/10 are active and 8/11 inactive in the C++ fixture. Rust resolves
    // against the completed combined object-number table, which is the state
    // visible after both native lookups have run.
    let object_numbers = HashSet::from([7_u64, 8, 10, 11]);
    let string_registrations = clonk_script::new_string_registrations();
    let resolution = SerializedC4ValueResolution {
        object_numbers: &object_numbers,
        string_registrations: &string_registrations,
    };
    let value = parse_serialized_c4value(encoded, 1)
        .expect("C++ denumeration fixture encoding parses")
        .resolve(&resolution);
    C4ValueDenumerationOutcome {
        value,
        rng_delta: rng.count - random_count_before,
    }
}

#[test]
fn c4value_denumeration_rust_driver_distinguishes_explicit_and_legacy_misses() {
    // C4Value.cpp:684-715: explicit O misses clear, while old A+offset misses
    // retain their word and pass through GuessType.
    for (encoded, expected) in [
        ("O7", ScriptValue::Object(7)),
        ("O8", ScriptValue::Object(8)),
        ("O9", ScriptValue::Nil),
        ("A1000000010", ScriptValue::Object(10)),
        ("A1000000011", ScriptValue::Object(11)),
        ("A1000000012", ScriptValue::Int(1_000_000_012)),
    ] {
        let outcome = run_c4value_denumeration(encoded);
        assert_eq!(outcome.value, expected, "denumerating {encoded}");
        assert_eq!(outcome.rng_delta, 0);
    }
}

#[derive(Debug, PartialEq, Eq)]
struct C4ValueRuntimeOperationOutcome {
    result: ScriptValue,
    aliases: i64,
    array: ScriptValue,
    error: String,
    rng_delta: i32,
}

fn c4value_runtime_observation(value: ScriptValue) -> (ScriptValue, i64, ScriptValue) {
    let ScriptValue::Array(mut returned) = value else {
        panic!("runtime C4Value driver returned a non-array observation")
    };
    assert_eq!(
        returned.len(),
        2,
        "runtime driver returns observation + array"
    );
    let array = returned.pop().expect("runtime driver returns its array");
    let observation = returned
        .pop()
        .expect("runtime driver returns its operation observation");
    let ScriptValue::Array(mut observation) = observation else {
        panic!("runtime C4Value observation is not a result/alias pair")
    };
    assert_eq!(
        observation.len(),
        2,
        "runtime observation returns result + alias count"
    );
    let aliases = observation
        .pop()
        .and_then(|value| value.as_c4_int())
        .map(i64::from)
        .expect("runtime observation alias count is an integer");
    let result = observation
        .pop()
        .expect("runtime observation includes its result");
    (result, aliases, array)
}

fn c4value_runtime_error(error: clonk_script::ScriptError) -> String {
    match error {
        clonk_script::ScriptError::Runtime(error) => error.message().to_owned(),
        clonk_script::ScriptError::Parse(..) => {
            panic!("runtime C4Value driver unexpectedly returned a parse error: {error}")
        }
    }
}

fn c4value_runtime_array_state(array: &ScriptValue) -> String {
    let ScriptValue::Array(values) = array else {
        panic!("runtime C4Value driver did not preserve an array")
    };
    let slots = values
        .iter()
        .enumerate()
        .filter(|(_, value)| !matches!(value, ScriptValue::Nil))
        .map(|(index, value)| {
            format!(
                ";slot{index}={}",
                crate::live_c4_save::encode_value_with_current_string_ids(value)
            )
        })
        .collect::<String>();
    format!("size={}{}", values.len(), slots)
}

fn c4value_runtime_type_name(value: &ScriptValue) -> &'static str {
    if matches!(value, ScriptValue::Nil) {
        "any"
    } else {
        value.type_name()
    }
}

/// Exercise C4ValueList element lookup and C4Value reference relocation through
/// the public script VM. These are the Rust counterparts of C4Value.cpp:37-297
/// and C4ValueList.cpp:28-90,143-183 used by the C++ oracle.
fn run_c4value_runtime_operation(case: &str) -> C4ValueRuntimeOperationOutcome {
    let rng = LcgRng::new(0xC4A1_0E00);
    let random_count_before = rng.count;

    let (result, aliases, array, error) = match case {
        "element_refs_survive_growth" => {
            let mut script = clonk_script::Engine::new();
            script.register_host_reference_function("MutateRuntimeRefs", [0, 1, 2], |args| {
                let first = args
                    .first()
                    .ok_or_else(|| clonk_script::RuntimeError::new("missing first array ref"))?;
                let second = args
                    .get(1)
                    .ok_or_else(|| clonk_script::RuntimeError::new("missing second array ref"))?;
                let grown = args
                    .get(2)
                    .ok_or_else(|| clonk_script::RuntimeError::new("missing grown array ref"))?;
                first.write(ScriptValue::Int(7))?;
                second.write(ScriptValue::Int(8))?;
                grown.write(ScriptValue::Int(4))?;
                let aliases = i32::from(first.is_reference()) + i32::from(second.is_reference());
                let sum = [first, second, grown]
                    .into_iter()
                    .try_fold(0_i32, |sum, argument| {
                        argument
                            .read()?
                            .as_c4_int()
                            .and_then(|value| sum.checked_add(value))
                            .ok_or_else(|| {
                                clonk_script::RuntimeError::new(
                                    "runtime array reference did not contain an integer",
                                )
                            })
                    })?;
                Ok(ScriptValue::Array(vec![
                    ScriptValue::Int(sum),
                    ScriptValue::Int(aliases),
                ]))
            });
            assert!(script.set_host_function_parameter_types(
                "MutateRuntimeRefs",
                [C4VType::Ref, C4VType::Ref, C4VType::Ref]
            ));
            script
                .load_script(
                    "#strict 3\n\
                     func RuntimeElementRefs() {\n\
                       var values = [1, 2];\n\
                       var observed = MutateRuntimeRefs(values[0], values[1], values[3]);\n\
                       return [observed, values];\n\
                     }",
                )
                .expect("runtime element-reference driver loads");
            let returned = script
                .call("RuntimeElementRefs", &[])
                .expect("runtime element-reference driver executes");
            let (result, aliases, array) = c4value_runtime_observation(returned);
            (result, aliases, array, String::new())
        }
        "value_read_missing_no_growth" => {
            let mut script = clonk_script::Engine::new();
            script.register_host_reference_function(
                "ObserveRuntimeValue",
                std::iter::empty::<usize>(),
                |args| {
                    let value = args
                        .first()
                        .ok_or_else(|| clonk_script::RuntimeError::new("missing observed value"))?;
                    Ok(ScriptValue::Array(vec![
                        value.read()?,
                        ScriptValue::Int(i32::from(value.is_reference())),
                    ]))
                },
            );
            script
                .load_script(
                    "#strict 3\n\
                     func RuntimeMissingRead() {\n\
                       var values = [1];\n\
                       var observed = ObserveRuntimeValue(values[5]);\n\
                       return [observed, values];\n\
                     }",
                )
                .expect("runtime missing-read driver loads");
            let returned = script
                .call("RuntimeMissingRead", &[])
                .expect("runtime missing-read driver executes");
            let (result, aliases, array) = c4value_runtime_observation(returned);
            (result, aliases, array, String::new())
        }
        "mutable_negative_clamps_and_grows" => {
            let mut script = clonk_script::Engine::new();
            script.register_host_reference_function("WriteRuntimeRef", [0], |args| {
                let target = args
                    .first()
                    .ok_or_else(|| clonk_script::RuntimeError::new("missing writable array ref"))?;
                target.write(ScriptValue::Int(6))?;
                Ok(ScriptValue::Array(vec![
                    target.read()?,
                    ScriptValue::Int(i32::from(target.is_reference())),
                ]))
            });
            assert!(script.set_host_function_parameter_types("WriteRuntimeRef", [C4VType::Ref]));
            script
                .load_script(
                    "#strict 3\n\
                     func RuntimeNegativeRef() {\n\
                       var values = [];\n\
                       var observed = WriteRuntimeRef(values[-9]);\n\
                       return [observed, values];\n\
                     }",
                )
                .expect("runtime negative-reference driver loads");
            let returned = script
                .call("RuntimeNegativeRef", &[])
                .expect("runtime negative-reference driver executes");
            let (result, aliases, array) = c4value_runtime_observation(returned);
            (result, aliases, array, String::new())
        }
        "wrong_type_index_error" | "max_index_error" => {
            let (function, statement) = if case == "wrong_type_index_error" {
                ("RuntimeWrongType", "return runtime_values[\"bad\"];")
            } else {
                ("RuntimeMaxIndex", "runtime_values[1000000] = 2;")
            };
            let mut script = clonk_script::Engine::new();
            script.set_global_variables(clonk_script::new_global_variables());
            script
                .load_script(&format!(
                    "#strict 3\n\
                     static runtime_values;\n\
                     func RuntimeReset() {{ runtime_values = [1]; }}\n\
                     func RuntimeState() {{ return runtime_values; }}\n\
                     func {function}() {{ {statement} }}"
                ))
                .expect("runtime array-error driver loads");
            script
                .call("RuntimeReset", &[])
                .expect("runtime array-error fixture initializes");
            let error = script
                .call(function, &[])
                .expect_err("runtime array-error fixture must reject the index");
            let array = script
                .call("RuntimeState", &[])
                .expect("runtime array-error fixture state remains readable");
            (ScriptValue::Nil, 0, array, c4value_runtime_error(error))
        }
        other => panic!("unknown c4value_runtime_operations case `{other}`"),
    };

    C4ValueRuntimeOperationOutcome {
        result,
        aliases,
        array,
        error,
        rng_delta: rng.count - random_count_before,
    }
}

#[test]
fn c4value_runtime_operations_rust_driver_uses_live_array_references() {
    // Mirrors C4Value.cpp:37-297 and C4ValueList.cpp:28-90,143-183.
    let growth = run_c4value_runtime_operation("element_refs_survive_growth");
    assert_eq!(growth.result, ScriptValue::Int(19));
    assert_eq!(growth.aliases, 2);
    assert_eq!(
        c4value_runtime_array_state(&growth.array),
        "size=4;slot0=i7;slot1=i8;slot3=i4"
    );

    let missing = run_c4value_runtime_operation("value_read_missing_no_growth");
    assert_eq!(missing.result, ScriptValue::Nil);
    assert_eq!(missing.aliases, 0);
    assert_eq!(
        c4value_runtime_array_state(&missing.array),
        "size=1;slot0=i1"
    );

    let negative = run_c4value_runtime_operation("mutable_negative_clamps_and_grows");
    assert_eq!(negative.result, ScriptValue::Int(6));
    assert_eq!(negative.aliases, 1);
    assert_eq!(
        c4value_runtime_array_state(&negative.array),
        "size=1;slot0=i6"
    );

    let wrong_type = run_c4value_runtime_operation("wrong_type_index_error");
    assert_eq!(
        wrong_type.error,
        "array access: can not convert \"string\" to int"
    );
    assert_eq!(
        c4value_runtime_array_state(&wrong_type.array),
        "size=1;slot0=i1"
    );

    let max_index = run_c4value_runtime_operation("max_index_error");
    assert_eq!(max_index.error, "out of memory");
    assert_eq!(
        c4value_runtime_array_state(&max_index.array),
        "size=1;slot0=i1"
    );
}

fn action_direction_engine() -> (Engine, crate::ObjectId) {
    let mut definition =
        Definition::from_script("WIPF", "Wipf", "#strict\n").expect("oracle fixture compiles");
    definition.configure_actions(
        Some("Walk".to_string()),
        HashMap::from([
            (
                "Walk".to_string(),
                ActionSpec::default()
                    .with_procedure("WALK")
                    .with_directions(2)
                    .with_length(18)
                    .with_delay(2)
                    .with_next("Walk")
                    .with_turn_action("Turn"),
            ),
            (
                "Turn".to_string(),
                ActionSpec::default()
                    .with_procedure("NONE")
                    .with_directions(2)
                    .with_length(6)
                    .with_delay(2)
                    .with_next("Walk"),
            ),
        ]),
    );
    definition.set_physical(PhysicalInfo {
        walk: 80_000,
        ..PhysicalInfo::default()
    });

    let mut engine = Engine::with_seed(0);
    engine.set_physics(PhysicsSettings::new(0, 20, -20));
    engine
        .register_definition(definition)
        .expect("oracle fixture registers");
    let id = engine
        .spawn_object(
            SpawnConfig::new("WIPF")
                .with_position(crate::Vector2::new(541, 629))
                .with_fixed_position(FixedVec2::new(
                    C4Fixed::from_raw(35_468_082),
                    C4Fixed::from_raw(41_222_142),
                ))
                .with_fixed_velocity(FixedVec2::new(
                    C4Fixed::from_raw(-52_430),
                    C4Fixed::from_raw(65_534),
                ))
                .with_action(ActionState::new("Walk"))
                .with_direction(Direction::Right)
                .with_command_direction(CommandDirection::Right)
                .with_category(CATEGORY_OBJECT)
                .with_mobile(true)
                .with_loaded(true),
        )
        .expect("oracle fixture spawns");
    (engine, id)
}

fn action_push_pull_fight_direction_engine(case: &Value) -> (Engine, crate::ObjectId) {
    let name = case["name"]
        .as_str()
        .expect("procedure-direction case has a name");
    let (action_name, procedure, walk) = match name {
        "push_positive_subpixel" => ("Push", "PUSH", 1),
        "pull_positive_subpixel" => ("Pull", "PULL", 1),
        "fight_target_right_negative_velocity" | "fight_equal_x_negative_velocity" => {
            ("Fight", "FIGHT", 35_000)
        }
        other => panic!("unknown procedure-direction case `{other}`"),
    };
    let actor_script = r#"#strict
local turn_starts, turn_start_dir;
protected func TurnStart()
{
    turn_starts = turn_starts + 1;
    turn_start_dir = GetDir();
    return true;
}
"#;
    let mut actor = Definition::from_script("ACTR", "Actor", actor_script)
        .expect("procedure-direction actor compiles");
    actor.set_c4_callback_convention(true);
    actor.set_shape_rect(Some(DefinitionRect::new(-8, -8, 16, 16)));
    actor.set_physical(PhysicalInfo {
        walk,
        push: 100_000,
        ..PhysicalInfo::default()
    });
    actor.configure_actions(
        Some(action_name.to_string()),
        HashMap::from([
            (
                action_name.to_string(),
                ActionSpec::default()
                    .with_procedure(procedure)
                    .with_directions(2)
                    .with_flip_dir(1)
                    .with_turn_action("Turn"),
            ),
            (
                "Turn".to_string(),
                ActionSpec::default()
                    .with_directions(2)
                    .with_flip_dir(1)
                    .with_start_call("TurnStart"),
            ),
        ]),
    );

    let mut target = Definition::from_script("TRGT", "Target", "#strict\n")
        .expect("procedure-direction target compiles");
    target.set_shape_rect(Some(DefinitionRect::new(-8, -8, 16, 16)));
    target.set_grab(1);
    target.set_mass(200);
    target.configure_actions(
        Some("Fight".to_string()),
        HashMap::from([(
            "Fight".to_string(),
            ActionSpec::default()
                .with_procedure("FIGHT")
                .with_directions(2),
        )]),
    );

    let mut engine = Engine::with_seed(0);
    engine.set_physics(PhysicsSettings::new(0, 20, -20));
    engine
        .register_definition(actor)
        .expect("procedure-direction actor registers");
    engine
        .register_definition(target)
        .expect("procedure-direction target registers");
    let target_id = engine
        .spawn_object(
            SpawnConfig::new("TRGT")
                .with_category(if procedure == "FIGHT" {
                    CATEGORY_OBJECT
                } else {
                    CATEGORY_VEHICLE
                })
                .with_position(crate::Vector2::new(i(case, "target_x") as i32, 0))
                .with_action(ActionState::new("Fight")),
        )
        .expect("procedure-direction target spawns");
    let mut action = ActionState::new(action_name);
    action.target = Some(target_id);
    let direction = match i(case, "initial_direction") {
        0 => Direction::Left,
        1 => Direction::Right,
        other => panic!("invalid procedure-direction fixture direction {other}"),
    };
    let initial_xdir = if procedure == "FIGHT" {
        C4Fixed::from_raw(i(case, "xdir_raw") as i32)
    } else {
        C4Fixed::ZERO
    };
    let actor_id = engine
        .spawn_object(
            SpawnConfig::new("ACTR")
                .with_category(CATEGORY_OBJECT)
                .with_position(crate::Vector2::new(i(case, "actor_x") as i32, 0))
                .with_action(action)
                .with_direction(direction)
                .with_command_direction(CommandDirection::Right)
                .with_fixed_velocity(FixedVec2::new(initial_xdir, C4Fixed::ZERO))
                .with_mobile(true),
        )
        .expect("procedure-direction actor spawns");
    let actor_idx = engine
        .find_object_index(actor_id)
        .expect("procedure-direction actor exists");
    engine.objects[actor_idx].state.draw_transform = None;
    (engine, actor_id)
}

fn swim_action_direction_engine() -> (Engine, crate::ObjectId) {
    let mut definition =
        Definition::from_script("FISH", "Fish", "#strict\n").expect("oracle fixture compiles");
    definition.configure_actions(
        Some("Swim".to_string()),
        HashMap::from([
            (
                "Swim".to_string(),
                ActionSpec::default()
                    .with_procedure("SWIM")
                    .with_directions(2)
                    .with_length(20)
                    .with_delay(1)
                    .with_next("Swim")
                    .with_turn_action("Turn"),
            ),
            (
                "Turn".to_string(),
                ActionSpec::default()
                    .with_procedure("SWIM")
                    .with_directions(2)
                    .with_length(15)
                    .with_delay(3)
                    .with_next("Swim"),
            ),
        ]),
    );
    definition.set_physical(PhysicalInfo {
        swim: 100_000,
        ..PhysicalInfo::default()
    });

    let mut engine = Engine::with_seed(0);
    engine.set_physics(PhysicsSettings::new(0, 20, -20));
    engine
        .register_definition(definition)
        .expect("oracle fixture registers");
    let mut action = ActionState::new("Swim");
    action.phase = 3;
    action.time = 103;
    let id = engine
        .spawn_object(
            SpawnConfig::new("FISH")
                .with_position(crate::Vector2::new(873, 438))
                .with_fixed_position(FixedVec2::new(
                    C4Fixed::from_raw(57_212_928),
                    C4Fixed::from_raw(28_737_532),
                ))
                .with_fixed_velocity(FixedVec2::new(C4Fixed::ZERO, C4Fixed::from_raw(-6_556)))
                .with_action(action)
                .with_direction(Direction::Right)
                .with_command_direction(CommandDirection::Left)
                .with_category(CATEGORY_OBJECT)
                .with_mobile(true)
                .with_loaded(true),
        )
        .expect("oracle fixture spawns");
    let idx = engine.find_object_index(id).expect("swimmer exists");
    engine.objects[idx].state.in_liquid = true;
    (engine, id)
}

fn action_callbacks_engine(case: &str) -> (Engine, crate::ObjectId) {
    let script = r#"#strict
local callbackOrder, startCount, oldCount;

protected func Activity()
{
    SetAction("New");
    return 1;
}

protected func OnStart()
{
    callbackOrder = callbackOrder * 10 + 1;
    startCount = startCount + 1;
    return 1;
}

protected func OnEnd()
{
    callbackOrder = callbackOrder * 10 + 2;
    oldCount = oldCount + 1;
    return 1;
}

protected func OnAbort()
{
    callbackOrder = callbackOrder * 10 + 3;
    oldCount = oldCount + 1;
    return 1;
}
"#;
    let mut definition =
        Definition::from_script("ACBK", "Action callbacks", script).expect("fixture compiles");
    definition.set_c4_callback_convention(true);
    let mut old = ActionSpec::default();
    match case {
        "script_start_only" => {
            definition.set_timer(1);
            definition.set_timer_call(Some("Activity".to_string()));
        }
        "script_start_abort" => {
            definition.set_timer(1);
            definition.set_timer_call(Some("Activity".to_string()));
            old = old.with_abort_call("OnAbort");
        }
        "natural_start_end" => {
            old = old
                .with_length(1)
                .with_delay(1)
                .with_next("New")
                .with_end_call("OnEnd");
        }
        other => panic!("unknown action_callbacks case `{other}`"),
    }
    definition.configure_actions(
        Some("Old".to_string()),
        HashMap::from([
            ("Old".to_string(), old),
            (
                "New".to_string(),
                ActionSpec::default().with_start_call("OnStart"),
            ),
        ]),
    );

    let mut engine = Engine::with_seed(0);
    engine
        .register_definition(definition)
        .expect("fixture registers");
    let id = engine
        .spawn_object(
            SpawnConfig::new("ACBK")
                .with_action(ActionState::new("Old"))
                .with_category(CATEGORY_OBJECT)
                .with_local_vars(HashMap::from([
                    ("callbackOrder".to_string(), ScriptValue::Int(0)),
                    ("startCount".to_string(), ScriptValue::Int(0)),
                    ("oldCount".to_string(), ScriptValue::Int(0)),
                ]))
                .with_loaded(true),
        )
        .expect("fixture spawns");
    (engine, id)
}

fn action_callback_local(engine: &Engine, id: crate::ObjectId, name: &str) -> i64 {
    engine
        .find_object_index(id)
        .and_then(|idx| engine.objects[idx].state.local_vars.get(name))
        .and_then(|value| match value {
            ScriptValue::Int(value) => Some(i64::from(*value)),
            _ => None,
        })
        .unwrap_or(0)
}

fn connect_removal_engine(geometry_break: bool) -> (Engine, crate::ObjectId) {
    let script = r#"#strict
local callbackOrder, lineBreakCount, lineBreakArgumentPresent, lineBreakAutomatic, destructionCount;

protected func LineBreak(automatic)
{
    callbackOrder = callbackOrder * 10 + 1;
    lineBreakCount = lineBreakCount + 1;
    if (GetType(automatic) != 0) lineBreakArgumentPresent = 1;
    if (automatic) lineBreakAutomatic = 1;
    return 1;
}

protected func Destruction()
{
    callbackOrder = callbackOrder * 10 + 2;
    destructionCount = destructionCount + 1;
    return 1;
}
"#;
    let mut definition =
        Definition::from_script("RPLN", "CONNECT removal line", script).expect("fixture compiles");
    definition.set_c4_callback_convention(true);
    definition.set_line(1);
    if geometry_break {
        definition.set_shape_vertices(vec![crate::ObjectVertex::new(0, 0)]);
    }
    definition.configure_actions(
        Some("Connect".to_string()),
        HashMap::from([(
            "Connect".to_string(),
            ActionSpec::default().with_procedure("CONNECT"),
        )]),
    );

    let mut engine = Engine::with_seed(0);
    engine
        .register_definition(definition)
        .expect("fixture registers");
    if geometry_break {
        engine
            .register_script_definition("CEND", "CONNECT endpoint", "#strict\n")
            .expect("endpoint fixture registers");
    }
    let mut action = ActionState::new("Connect");
    if geometry_break {
        action.target = Some(
            engine
                .spawn_object(SpawnConfig::new("CEND").with_position(crate::Vector2::new(10, 0)))
                .expect("first endpoint spawns"),
        );
        action.target2 = Some(
            engine
                .spawn_object(SpawnConfig::new("CEND").with_position(crate::Vector2::new(20, 0)))
                .expect("second endpoint spawns"),
        );
    }
    let id = engine
        .spawn_object(
            SpawnConfig::new("RPLN")
                .with_action(action)
                .with_category(CATEGORY_OBJECT)
                .with_local_vars(HashMap::from([
                    ("callbackOrder".to_string(), ScriptValue::Int(0)),
                    ("lineBreakCount".to_string(), ScriptValue::Int(0)),
                    ("lineBreakArgumentPresent".to_string(), ScriptValue::Int(0)),
                    ("lineBreakAutomatic".to_string(), ScriptValue::Int(0)),
                    ("destructionCount".to_string(), ScriptValue::Int(0)),
                ]))
                .with_loaded(true),
        )
        .expect("fixture spawns");
    (engine, id)
}

fn expect_connect_removal_case(golden: &Value, section: &str, geometry_break: bool) {
    let case = &golden[section];
    let (mut engine, id) = connect_removal_engine(geometry_break);
    let idx = engine.find_object_index(id).expect("line exists");
    assert!(!engine
        .exec_connect_line(idx)
        .expect("CONNECT break branch executes"));
    expect_eq(
        section,
        0,
        "line_break_count",
        i(case, "line_break_count"),
        action_callback_local(&engine, id, "lineBreakCount"),
    );
    expect_eq(
        section,
        0,
        "line_break_argument_count",
        i(case, "line_break_argument_count"),
        action_callback_local(&engine, id, "lineBreakArgumentPresent"),
    );
    expect_eq(
        section,
        0,
        "line_break_automatic",
        i(case, "line_break_automatic"),
        action_callback_local(&engine, id, "lineBreakAutomatic"),
    );
    expect_eq(
        section,
        0,
        "destruction_count",
        i(case, "destruction_count"),
        action_callback_local(&engine, id, "destructionCount"),
    );
    expect_eq(
        section,
        0,
        "callback_order",
        i(case, "callback_order"),
        action_callback_local(&engine, id, "callbackOrder"),
    );
    let object = &engine.objects[idx];
    expect_eq(
        section,
        0,
        "status",
        i(case, "status"),
        i64::from(object.state.status.to_script_value()),
    );
}

fn solid_mask_sprite(alpha: u8) -> DefinitionSpriteImage {
    const WIDTH: u32 = 220;
    const HEIGHT: u32 = 87;
    const SOURCE_X: usize = 219;
    const SOURCE_Y: usize = 86;
    let mut pixels = vec![0; (WIDTH * HEIGHT * 4) as usize];
    pixels[(SOURCE_Y * WIDTH as usize + SOURCE_X) * 4 + 3] = alpha;
    DefinitionSpriteImage {
        width: WIDTH,
        height: HEIGHT,
        pixels: Arc::from(pixels.into_boxed_slice()),
        color_mask: None,
    }
}

/// Sprite whose pixels encode their own coordinates (R = x, G = y), so the
/// first pixel of a cropped facet recovers the source rect's origin.
fn coordinate_sprite(size: u32) -> DefinitionSpriteImage {
    let mut pixels = vec![0; (size * size * 4) as usize];
    for y in 0..size {
        for x in 0..size {
            let base = ((y * size + x) * 4) as usize;
            pixels[base] = x as u8;
            pixels[base + 1] = y as u8;
            pixels[base + 3] = 255;
        }
    }
    DefinitionSpriteImage {
        width: size,
        height: size,
        pixels: Arc::from(pixels.into_boxed_slice()),
        color_mask: None,
    }
}

fn def_picture_scale_engine(scale_percent: u32, picture: DefinitionPicture) -> Engine {
    let mut definition =
        Definition::from_script("PSCL", "Picture Scale", "#strict\n").expect("fixture compiles");
    definition.set_shape_rect(Some(DefinitionRect::new(0, 0, 1, 1)));
    definition.set_picture(Some(picture));
    // C4Def.cpp:745 `Scale = C4DefCore::Scale / 100.0f`, as wired at lib.rs:12841.
    definition.set_graphics_scale(scale_percent as f32 / 100.0);
    definition.set_sprite_image(Some(coordinate_sprite(256)));

    let mut engine = Engine::with_seed(7);
    engine
        .register_definition(definition)
        .expect("definition registers");
    engine
}

fn solid_mask_graphics_engine() -> (Engine, crate::ObjectId) {
    let mut definition =
        Definition::from_script("CTWR", "Castle Tower", "#strict\n").expect("fixture compiles");
    definition.set_shape_rect(Some(DefinitionRect::new(0, 0, 1, 1)));
    definition.set_solid_mask(Some(DefinitionTargetRect::new(219, 86, 1, 1, 0, 0)));
    definition.set_sprite_image(Some(solid_mask_sprite(0)));
    definition.set_sprite_variants(HashMap::from([("2".to_string(), solid_mask_sprite(255))]));

    let mut engine = Engine::with_seed(7);
    let grid = PixelGrid::new(
        3,
        3,
        vec![0; 9],
        vec![0, 100, 100],
        vec![None, Some("Earth".into()), Some("Vehicle".into())],
        vec![None; 3],
    );
    let mut landscape = Landscape::flat(3, 3);
    landscape.set_pixel_grid(grid);
    engine.set_landscape(landscape);
    engine
        .register_definition(definition)
        .expect("definition registers");
    let id = engine
        .spawn_object(
            SpawnConfig::new("CTWR")
                .with_position(crate::Vector2::new(1, 1))
                .with_loaded(true),
        )
        .expect("tower spawns");
    (engine, id)
}

struct RuleGoalParityResolver {
    roots: Vec<PathBuf>,
}

impl LegacyDefinitionResolver for RuleGoalParityResolver {
    fn resolve_definition_groups(
        &self,
        scenario: &Group,
        identifier: &str,
    ) -> Result<Vec<Group>, ScenarioError> {
        let mut groups = Vec::new();
        let normalized = identifier.replace('\\', "/");
        let path = Path::new(&normalized);

        if let Ok(child) = scenario.open_child(path) {
            groups.push(child);
        }
        for root in &self.roots {
            let candidate = root.join(path);
            if !candidate.exists() {
                continue;
            }
            let group = Group::open(&candidate)?;
            if groups
                .iter()
                .all(|existing| existing.root() != group.root())
            {
                groups.push(group);
            }
        }
        if groups.is_empty() {
            Err(ScenarioError::LegacyDefinitionNotFound {
                path: identifier.to_string(),
            })
        } else {
            Ok(groups)
        }
    }
}

fn golden_scenario_id_list(case: &Value, key: &str) -> Vec<ScenarioIdListEntry> {
    case[key]
        .as_array()
        .unwrap_or_else(|| panic!("network rule/goal case field `{key}` is an array"))
        .iter()
        .map(|entry| {
            ScenarioIdListEntry::new(
                entry["id"]
                    .as_str()
                    .unwrap_or_else(|| panic!("`{key}` entry has an id")),
                i(entry, "count") as i32,
            )
        })
        .collect()
}

fn scenario_id_list_text(entries: &[ScenarioIdListEntry]) -> String {
    entries
        .iter()
        .map(|entry| format!("{}={};", entry.id, entry.count))
        .collect()
}

fn indexed_bmp_2x2() -> Vec<u8> {
    const WIDTH: u32 = 2;
    const HEIGHT: u32 = 2;
    const STRIDE: usize = 4;
    const DATA_OFFSET: usize = 14 + 40 + 256 * 4;
    let file_size = DATA_OFFSET + STRIDE * HEIGHT as usize;
    let mut bytes = Vec::with_capacity(file_size);
    bytes.extend_from_slice(b"BM");
    bytes.extend_from_slice(&(file_size as u32).to_le_bytes());
    bytes.extend_from_slice(&0u32.to_le_bytes());
    bytes.extend_from_slice(&(DATA_OFFSET as u32).to_le_bytes());
    bytes.extend_from_slice(&40u32.to_le_bytes());
    bytes.extend_from_slice(&(WIDTH as i32).to_le_bytes());
    bytes.extend_from_slice(&(HEIGHT as i32).to_le_bytes());
    bytes.extend_from_slice(&1u16.to_le_bytes());
    bytes.extend_from_slice(&8u16.to_le_bytes());
    for _ in 0..4 {
        bytes.extend_from_slice(&0u32.to_le_bytes());
    }
    bytes.extend_from_slice(&256u32.to_le_bytes());
    bytes.extend_from_slice(&0u32.to_le_bytes());
    bytes.resize(file_size, 0);
    bytes
}

fn rust_network_rule_goal_placement(case: &Value, case_index: usize) {
    let name = case["name"]
        .as_str()
        .expect("network rule/goal case has a name");
    let scenario_rules = golden_scenario_id_list(case, "scenario_rules");
    let scenario_goals = golden_scenario_id_list(case, "scenario_goals");
    let parameter_rules = golden_scenario_id_list(case, "parameter_rules");
    let parameter_goals = golden_scenario_id_list(case, "parameter_goals");

    let fixture = tempfile::tempdir().expect("network rule/goal parity fixture");
    let definitions_root = fixture.path().join("Defs.c4d");
    let goal_ids = scenario_goals
        .iter()
        .chain(parameter_goals.iter())
        .map(|entry| entry.id.as_str())
        .collect::<HashSet<_>>();
    let mut definition_ids = scenario_rules
        .iter()
        .chain(scenario_goals.iter())
        .chain(parameter_rules.iter())
        .chain(parameter_goals.iter())
        .map(|entry| entry.id.clone())
        .collect::<BTreeSet<_>>();
    definition_ids.insert("GOAL".to_string());
    for id in definition_ids {
        let definition = definitions_root.join(format!("{id}.c4d"));
        std::fs::create_dir_all(&definition).expect("definition directory");
        let category = if goal_ids.contains(id.as_str()) {
            4096
        } else {
            8192
        };
        std::fs::write(
            definition.join("DefCore.txt"),
            format!("[DefCore]\nid={id}\nName={id}\nCategory={category}\n"),
        )
        .expect("definition core writes");
        image::RgbaImage::from_pixel(1, 1, image::Rgba([1, 2, 3, 255]))
            .save(definition.join("Graphics.png"))
            .expect("definition graphics writes");
    }

    let scenario_directory = fixture.path().join("RuleGoalParity.c4s");
    std::fs::create_dir_all(&scenario_directory).expect("scenario directory");
    let energy_default = if name == "harpoonrace_join_data" {
        String::new()
    } else {
        "StructNeedEnergy=0\n".to_string()
    };
    std::fs::write(
        scenario_directory.join("Scenario.txt"),
        format!(
            "[Head]\nTitle=RuleGoalParity\n\n\
             [Definitions]\nDefinition1=Defs.c4d\n\n\
             [Game]\n{energy_default}Goals={}\nRules={}\n\n\
             [Landscape]\nMapZoom=10\n",
            scenario_id_list_text(&scenario_goals),
            scenario_id_list_text(&scenario_rules),
        ),
    )
    .expect("scenario core writes");
    std::fs::write(scenario_directory.join("Landscape.bmp"), indexed_bmp_2x2())
        .expect("scenario landscape writes");

    let resolver = RuleGoalParityResolver {
        roots: vec![fixture.path().to_path_buf()],
    };
    let scenario =
        Scenario::load_from_path_with(&scenario_directory, &resolver).expect("scenario loads");

    if name == "harpoonrace_join_data" {
        let defaults = scenario
            .lobby_metadata()
            .expect("legacy scenario has lobby metadata")
            .game_parameter_defaults();
        let rust_rules = defaults
            .rules()
            .iter()
            .map(|entry| serde_json::json!({"id": entry.id(), "count": entry.count()}))
            .collect::<Vec<_>>();
        let rust_goals = defaults
            .goals()
            .iter()
            .map(|entry| serde_json::json!({"id": entry.id(), "count": entry.count()}))
            .collect::<Vec<_>>();
        expect_json_eq(
            "network_rule_goal_placement",
            case_index,
            "parameter_rules",
            case["parameter_rules"].clone(),
            Value::Array(rust_rules),
        );
        expect_json_eq(
            "network_rule_goal_placement",
            case_index,
            "parameter_goals",
            case["parameter_goals"].clone(),
            Value::Array(rust_goals),
        );
    }

    let synchronized =
        GameParameterRuleGoalLists::new(parameter_rules.clone(), parameter_goals.clone());
    let mut engine = Engine::with_seed(7);
    scenario
        .apply_before_players_for_game_start(
            &mut engine,
            true,
            None,
            None,
            None,
            Some(&synchronized),
            None,
        )
        .expect("network scenario applies");
    let snapshot = engine.snapshot();
    let rule_ids = parameter_rules
        .iter()
        .map(|entry| entry.id.as_str())
        .collect::<HashSet<_>>();
    let goal_ids = parameter_goals
        .iter()
        .map(|entry| entry.id.as_str())
        .collect::<HashSet<_>>();
    let rust_rule_objects = snapshot
        .objects
        .iter()
        .filter(|object| rule_ids.contains(object.definition_id.as_str()))
        .map(|object| Value::String(object.definition_id.clone()))
        .collect();
    let rust_goal_objects = snapshot
        .objects
        .iter()
        .filter(|object| goal_ids.contains(object.definition_id.as_str()))
        .map(|object| Value::String(object.definition_id.clone()))
        .collect();
    expect_json_eq(
        "network_rule_goal_placement",
        case_index,
        "rule_objects",
        case["rule_objects"].clone(),
        Value::Array(rust_rule_objects),
    );
    expect_json_eq(
        "network_rule_goal_placement",
        case_index,
        "goal_objects",
        case["goal_objects"].clone(),
        Value::Array(rust_goal_objects),
    );
}

fn player_join_capacity_config(name: &str, player_info_id: i32) -> JoinPlayerConfig {
    JoinPlayerConfig {
        name: name.to_string(),
        player_info_id,
        score: 0,
        rounds: 0,
        rounds_won: 0,
        rounds_lost: 0,
        total_playing_time: 0,
        team: None,
        color_dw: 0xff0000,
        pref_color: 0,
        pref_position: 0,
        crew: Vec::new(),
        control_style: false,
        auto_context_menu: false,
        startup_player_count: 1,
    }
}

fn player_names(engine: &Engine) -> Value {
    Value::Array(
        engine
            .players()
            .map(|player| Value::String(player.name().to_string()))
            .collect(),
    )
}

fn savegame_matching_entry(case: &Value, side: &str) -> crate::control::ControlPlayerInfoEntry {
    let name = case[format!("{side}_name")]
        .as_array()
        .expect("savegame_player_matching name is a C++ oracle byte array")
        .iter()
        .map(|byte| byte.as_u64().expect("name byte is a number") as u8)
        .collect::<Vec<_>>();
    // The oracle emits Latin-1 bytes, which is what the engine stores.
    let filename = case[format!("{side}_filename")]
        .as_str()
        .expect("savegame_player_matching filename is a string")
        .as_bytes()
        .to_vec();
    crate::control::ControlPlayerInfoEntry {
        name: crate::control::LegacyCString::from_bytes(name)
            .expect("oracle names carry no interior NUL"),
        filename: crate::control::LegacyCString::from_bytes(filename)
            .expect("oracle filenames carry no interior NUL"),
        original_color: i(case, &format!("{side}_color")) as u32,
        ..Default::default()
    }
}

fn rust_savegame_player_matching(case: &Value, case_index: usize) {
    const SECTION: &str = "savegame_player_matching";
    let current = savegame_matching_entry(case, "current");
    let saved = savegame_matching_entry(case, "saved");
    let cpp = case["matches"]
        .as_array()
        .expect("savegame_player_matching matches is a C++ oracle array");
    assert_eq!(
        cpp.len(),
        4,
        "{SECTION} case {case_index} records one result per MatchingLevel"
    );
    for (level, expected) in cpp.iter().enumerate() {
        let expected = expected
            .as_bool()
            .expect("savegame_player_matching result is a bool");
        expect_json_eq(
            SECTION,
            case_index,
            &format!("matches[{level}]"),
            Value::Bool(expected),
            Value::Bool(crate::savegame_association::savegame_players_match(
                &current,
                &saved,
                level as u8,
            )),
        );
    }
}

/// C4IDList.cpp:33-103 — component order, which participates in the replay
/// hash but had no comparable field on the C++ side.
///
/// The list is a `std::vector<Entry>`, so position is meaningful and the same
/// ID may appear more than once with independent counts — the shipped Bazooka
/// `DefCore` does exactly that. A comparator that only checked ID/count pairs
/// would pass a model that collapsed the repeat, which is why the rows carry
/// the ordered entries themselves.
fn rust_component_order(case: &Value, case_index: usize) {
    const SECTION: &str = "component_order";

    let entries = |key: &str| -> Vec<(String, i32)> {
        case[key]
            .as_array()
            .unwrap_or_else(|| panic!("{SECTION} {key} is a C++ oracle array"))
            .iter()
            .map(|entry| {
                (
                    entry["id"]
                        .as_str()
                        .expect("component id is a string")
                        .to_owned(),
                    i(entry, "count") as i32,
                )
            })
            .collect()
    };

    // Built with `push`, not `set`: the parsed DefCore appends every entry it
    // reads, which is the only way a repeat can exist at all.
    let mut list = entries("initial")
        .into_iter()
        .map(|(id, count)| (crate::DefinitionId::from(id.as_str()), count))
        .collect::<crate::ComponentList>();

    if let Some(set) = case.get("set").filter(|set| !set.is_null()) {
        list.set(
            crate::DefinitionId::from(set["id"].as_str().expect("set id is a string")),
            i(set, "count") as i32,
        );
    }

    expect_json_eq(
        SECTION,
        case_index,
        "entries",
        serde_json::json!(entries("entries")
            .into_iter()
            .map(|(id, count)| serde_json::json!({"id": id, "count": count}))
            .collect::<Vec<_>>()),
        serde_json::json!(list
            .iter()
            .map(|(id, count)| serde_json::json!({"id": id.as_str(), "count": count}))
            .collect::<Vec<_>>()),
    );
    expect_eq(
        SECTION,
        case_index,
        "number_of_ids",
        i(case, "number_of_ids"),
        list.len() as i64,
    );

    // `GetIDCount` resolves through `findId`, which returns the **first**
    // matching entry; a later repeat is unreachable by ID.
    for lookup in case["lookups"]
        .as_array()
        .expect("component_order lookups is a C++ oracle array")
    {
        let id = lookup["id"].as_str().expect("lookup id is a string");
        expect_eq(
            SECTION,
            case_index,
            &format!("lookups[{id}]"),
            i(lookup, "count"),
            list.get(id).unwrap_or(0) as i64,
        );
    }
}

/// C4PlayerInfo.cpp:1373-1391 — the pass loop the four matching levels run
/// inside, which decides *which* savegame player each join ends up with.
///
/// The per-level predicate is already compared by `savegame_player_matching`;
/// what this adds is the loop's own semantics — pass ordering, first-accepting
/// candidate, the eligibility test, and which associations C++ calls "wild".
fn rust_savegame_association(case: &Value, case_index: usize) {
    const SECTION: &str = "savegame_association";

    let players = |key: &str| -> Vec<crate::control::ControlPlayerInfoEntry> {
        case[key]
            .as_array()
            .unwrap_or_else(|| panic!("{SECTION} {key} is a C++ oracle array"))
            .iter()
            .map(|player| {
                let name = player["name"]
                    .as_array()
                    .expect("player name is a C++ oracle byte array")
                    .iter()
                    .map(|byte| byte.as_u64().expect("name byte is a number") as u8)
                    .collect::<Vec<_>>();
                crate::control::ControlPlayerInfoEntry {
                    id: i(player, "id") as i32,
                    name: crate::control::LegacyCString::from_bytes(name)
                        .expect("oracle names carry no interior NUL"),
                    filename: crate::control::LegacyCString::from_bytes(
                        player["filename"]
                            .as_str()
                            .expect("player filename is a string")
                            .as_bytes()
                            .to_vec(),
                    )
                    .expect("oracle filenames carry no interior NUL"),
                    original_color: u(player, "color") as u32,
                    ..Default::default()
                }
            })
            .collect()
    };

    let mut participants = players("participants");
    let savegame_players = players("savegame_players");
    let wild = crate::savegame_association::associate_savegame_players(
        &mut participants,
        &savegame_players,
    );

    expect_json_eq(
        SECTION,
        case_index,
        "associations",
        case["associations"].clone(),
        serde_json::json!(participants
            .iter()
            .map(|player| player.savegame_player)
            .collect::<Vec<_>>()),
    );
    expect_json_eq(
        SECTION,
        case_index,
        "wild",
        case["wild"].clone(),
        serde_json::json!(wild
            .iter()
            .map(|takeover| serde_json::json!({
                "participant": takeover.participant,
                "savegame_player": takeover.savegame_player,
            }))
            .collect::<Vec<_>>()),
    );
}

fn rust_player_join_capacity(case: &Value, case_index: usize) {
    const SECTION: &str = "player_join_capacity";
    let initial_names = case["names_before"]
        .as_array()
        .expect("player join capacity names_before is a C++ oracle array");
    let mut engine = Engine::with_seed(0);
    for (index, name) in initial_names.iter().enumerate() {
        let name = name
            .as_str()
            .expect("player join capacity initial name is a string");
        engine
            .join_player(player_join_capacity_config(name, index as i32 + 1))
            .unwrap_or_else(|error| panic!("initial player `{name}` joins: {error}"));
    }

    expect_eq(
        SECTION,
        case_index,
        "count_before",
        i(case, "count_before"),
        engine.players().count() as i64,
    );
    expect_json_eq(
        SECTION,
        case_index,
        "names_before",
        case["names_before"].clone(),
        player_names(&engine),
    );

    let maximum = i(case, "max_players") as i32;
    let joining_name = case["joining_name"]
        .as_str()
        .expect("player join capacity joining_name is a string");
    engine.set_max_players(maximum);
    let result = engine.join_player(player_join_capacity_config(
        joining_name,
        initial_names.len() as i32 + 1,
    ));
    let accepted = match result {
        Ok(_) => true,
        Err(EngineError::TooManyPlayers { .. }) => false,
        Err(error) => panic!("unexpected player join error for `{joining_name}`: {error}"),
    };

    expect_json_eq(
        SECTION,
        case_index,
        "accepted",
        case["accepted"].clone(),
        serde_json::json!(accepted),
    );
    expect_eq(
        SECTION,
        case_index,
        "count_after",
        i(case, "count_after"),
        engine.players().count() as i64,
    );
    expect_json_eq(
        SECTION,
        case_index,
        "names_after",
        case["names_after"].clone(),
        player_names(&engine),
    );
}

fn rust_breath_refill_callback_order(case: &Value, case_index: usize) {
    const SECTION: &str = "breath_refill_callback_order";
    const CALLBACK_PHYSICAL_BREATH: i32 = 7;

    let name = case["name"]
        .as_str()
        .unwrap_or_else(|| panic!("{SECTION} row {case_index} is missing its name"));
    let callback_defined = match name {
        "goldwipfcaves_missing_callback" => false,
        "goldwipfcaves_mutating_callback" => true,
        other => panic!("{SECTION} row {case_index} has unexpected name {other}"),
    };
    assert_eq!(
        i(case, "callback_defined"),
        callback_defined as i64,
        "{SECTION} row {case_index} callback availability"
    );
    let physical_before = i(case, "physical_before") as i32;
    let state_before = i(case, "state_before") as i32;
    assert_eq!(
        (physical_before, state_before),
        (-2_009_260_032, i32::MAX),
        "{SECTION} row {case_index} must retain the exact Goldwipfcaves raw pair"
    );

    let mut source = String::from(
        r#"#strict 3
local deep_breath_calls, callback_breath, callback_breath_after, callback_order;

public func ResetBreathProbe()
{
    deep_breath_calls = callback_breath = callback_breath_after = callback_order = 0;
    return 1;
}
"#,
    );
    if callback_defined {
        // The callback observes the live raw Breath before C4Object.cpp:919,
        // then changes the physical maximum and clamps raw Breath to a nonzero
        // sentinel through the real DoBreath host (C4Script.cpp:502-506;
        // C4Object.cpp:1406-1413). Assigning the delta or either maximum cannot
        // reproduce the later post-callback +=.
        source.push_str(
            r#"

protected func DeepBreath()
{
    callback_order = callback_order * 10 + 1;
    deep_breath_calls += 1;
    callback_breath = GetObjectVal("Breath");
    SetPhysical("Breath", 7, 2);
    DoBreath(0);
    callback_breath_after = GetObjectVal("Breath");
    return 1;
}
"#,
        );
    }
    source.push_str(
        r#"

public func ObserveCompletedSupply()
{
    callback_order = callback_order * 10 + 2;
    return [
        deep_breath_calls, callback_breath, callback_breath_after,
        callback_order, GetObjectVal("Breath")
    ];
}
"#,
    );
    let mut definition =
        Definition::from_script("BRTH", "Breath refill callback-order oracle", &source)
            .expect("breath-refill oracle definition compiles");
    definition.set_c4_callback_convention(true);
    definition.set_physical(PhysicalInfo {
        breath: physical_before,
        ..PhysicalInfo::default()
    });

    let mut engine = Engine::with_seed(0);
    engine
        .register_definition(definition)
        .expect("breath-refill oracle definition registers");
    let mut spawn = SpawnConfig::new("BRTH")
        .with_alive(true)
        .with_category(CATEGORY_LIVING);
    spawn.breath = Some(state_before);
    let object = engine
        .spawn_object(spawn)
        .expect("breath-refill oracle object spawns");
    let object_index = engine
        .find_object_index(object)
        .expect("breath-refill oracle object remains");
    engine
        .call_object_function(object_index, "ResetBreathProbe", Vec::new())
        .expect("breath-refill oracle probe resets");

    // Frame five selects the exact breathable-supply arm without also running
    // the Tick3, Tick10, Tick35, or Tick255 ExecLife arms.
    engine
        .exec_object_life(object_index, 5)
        .expect("breath-refill oracle life step succeeds");
    let trace = engine
        .call_object_function(object_index, "ObserveCompletedSupply", Vec::new())
        .expect("breath-refill oracle trace reads");
    let trace = match trace {
        ScriptValue::Array(values) => values
            .into_iter()
            .map(|value| match value {
                ScriptValue::Int(value) => value,
                ScriptValue::Bool(value) => i32::from(value),
                ScriptValue::RawBool(value) => i32::from(value != 0),
                other => panic!("{SECTION} trace contains unexpected value {other:?}"),
            })
            .collect::<Vec<_>>(),
        other => panic!("{SECTION} trace is not an array: {other:?}"),
    };
    assert_eq!(
        trace.len(),
        5,
        "{SECTION} trace must contain calls, callback breaths, order, and final breath"
    );

    let state_after = engine.objects[object_index].state.breath;
    assert_eq!(
        trace[4], state_after,
        "{SECTION} post-block script observation must match engine state"
    );
    let physical_after = engine.object_physical(object_index).breath;
    let expected_physical_after = if callback_defined {
        CALLBACK_PHYSICAL_BREATH
    } else {
        physical_before
    };
    assert_eq!(
        physical_after, expected_physical_after,
        "{SECTION} row {case_index} callback-dependent physical state"
    );
    let addend_before = if callback_defined {
        trace[2]
    } else {
        state_before
    };
    let deep_breath_condition = physical_before.wrapping_sub(state_before) > physical_before / 2;

    let rust = serde_json::json!({
        "name": name,
        "callback_defined": i32::from(callback_defined),
        "physical_before": physical_before,
        "state_before": state_before,
        "take": state_after.wrapping_sub(addend_before),
        "physical_half": physical_before / 2,
        "deep_breath_condition": i32::from(deep_breath_condition),
        "deep_breath_call_attempts": i32::from(deep_breath_condition),
        "deep_breath_calls": trace[0],
        "callback_name_matches": i32::from(deep_breath_condition),
        "callback_breath": trace[1],
        "callback_breath_after": trace[2],
        "callback_order": trace[3],
        "physical_after": physical_after,
        "state_after": state_after,
    });
    expect_json_eq(SECTION, case_index, "row", case.clone(), rust);
}

fn rust_set_graphics_missing_lookup(case: &Value, case_index: usize) {
    const SECTION: &str = "set_graphics_missing_lookup";
    const SCRIPT: &str = r#"#strict 3
public func SelectKnownBase() { return SetGraphics("Known"); }
public func SelectMissingBase() { return SetGraphics("Missing"); }
public func SelectKnownOverlay()
{
    return SetGraphics("Known", this, GetID(), 1, GFXOV_MODE_Base);
}
public func SelectMissingOverlay()
{
    return SetGraphics("Missing", this, GetID(), 1, GFXOV_MODE_Base);
}
"#;

    let name = case["name"]
        .as_str()
        .unwrap_or_else(|| panic!("{SECTION} row {case_index} is missing its name"));
    let (setup_function, missing_function) = match name {
        "base_missing_name" => ("SelectKnownBase", "SelectMissingBase"),
        "overlay_missing_name" => ("SelectKnownOverlay", "SelectMissingOverlay"),
        other => panic!("{SECTION} row {case_index} has unexpected name {other}"),
    };

    let mut definition = Definition::from_script("SGFX", "SetGraphics lookup oracle", SCRIPT)
        .expect("SetGraphics lookup oracle compiles");
    definition.set_c4_callback_convention(true);
    definition.set_sprite_image(Some(solid_mask_sprite(255)));
    definition.set_sprite_variants(HashMap::from([(
        clonk_resources::material::c4_name_key("Known"),
        solid_mask_sprite(255),
    )]));

    let mut engine = Engine::with_seed(0);
    engine
        .register_definition(definition)
        .expect("SetGraphics lookup oracle registers");
    let object = engine
        .spawn_object(SpawnConfig::new("SGFX"))
        .expect("SetGraphics lookup oracle object spawns");
    let object_index = engine
        .find_object_index(object)
        .expect("SetGraphics lookup oracle object remains");
    let setup_result = engine
        .call_object_function(object_index, setup_function, Vec::new())
        .expect("SetGraphics lookup oracle setup succeeds");
    assert!(
        matches!(
            setup_result,
            ScriptValue::Bool(true) | ScriptValue::RawBool(1)
        ),
        "{SECTION} row {case_index} must establish its known graphics first: {setup_result:?}"
    );

    let state_before = engine.objects[object_index].state.clone();
    let missing_result = engine
        .call_object_function(object_index, missing_function, Vec::new())
        .expect("missing SetGraphics lookup returns normally");
    let result = match missing_result {
        ScriptValue::Bool(value) => i32::from(value),
        ScriptValue::RawBool(value) => i32::from(value != 0),
        ScriptValue::Int(value) => value,
        other => panic!("{SECTION} row {case_index} returned unexpected value {other:?}"),
    };
    let state_after = &engine.objects[object_index].state;
    let overlay_before = state_before.graphics_overlays.first();
    let overlay_after = state_after.graphics_overlays.first();
    let rust = serde_json::json!({
        "name": name,
        "result": result,
        "base_name_before": state_before
            .base_graphics
            .as_ref()
            .and_then(|graphics| graphics.graphics_name.as_deref()),
        "base_name_after": state_after
            .base_graphics
            .as_ref()
            .and_then(|graphics| graphics.graphics_name.as_deref()),
        "overlay_count_before": state_before.graphics_overlays.len(),
        "overlay_count_after": state_after.graphics_overlays.len(),
        "overlay_name_before": overlay_before.and_then(|overlay| overlay.graphics_name.as_deref()),
        "overlay_name_after": overlay_after.and_then(|overlay| overlay.graphics_name.as_deref()),
        "overlay_mode_before": overlay_before.map(|overlay| overlay.mode as i32),
        "overlay_mode_after": overlay_after.map(|overlay| overlay.mode as i32),
    });
    expect_json_eq(SECTION, case_index, "row", case.clone(), rust);
}

#[test]
fn parity_differential_matches_cpp_golden() {
    let golden = load_golden();

    // C4DefGraphics.cpp:221-229, C4Object.cpp:5894-5910, and
    // C4Script.cpp:4372-4442. Both rows first select an existing named graphic
    // through the real SetGraphics script host, then prove that a missing base
    // or overlay name returns false without changing the established state.
    let set_graphics_missing_cases = golden["set_graphics_missing_lookup"]
        .as_array()
        .expect("set_graphics_missing_lookup is a C++ oracle array");
    assert_eq!(
        set_graphics_missing_cases.len(),
        2,
        "set_graphics_missing_lookup must retain base and overlay rows"
    );
    for (case_index, case) in set_graphics_missing_cases.iter().enumerate() {
        rust_set_graphics_missing_lookup(case, case_index);
    }

    // C4Object.cpp:915-919. The oracle mechanically extracts the complete
    // breathable-supply block and runs the exact malformed Goldwipfcaves raw
    // pair. The missing-callback row pins the overflowing final +=; the second
    // row changes both the physical maximum and raw state in DeepBreath, making
    // the pre-callback delta and callback-before-add ordering independently
    // observable.
    let breath_refill_cases = golden["breath_refill_callback_order"]
        .as_array()
        .expect("breath_refill_callback_order is a C++ oracle array");
    assert_eq!(
        breath_refill_cases.len(),
        2,
        "breath_refill_callback_order must retain both exact raw-pair rows"
    );
    for (case_index, case) in breath_refill_cases.iter().enumerate() {
        rust_breath_refill_callback_order(case, case_index);
    }

    // C4SGame::ConvertGoals and C4Game::InitRules/InitGoals
    // (oracle-src-pinned src/C4Scenario.cpp:506-556;
    // src/C4Game.cpp:4056-4076). HarpoonRace drives the same authored lists
    // through both converters; the count-edge case then makes local
    // Scenario.txt leakage observable while applying the synchronized lists.
    for (case_index, case) in golden["network_rule_goal_placement"]
        .as_array()
        .expect("network_rule_goal_placement is a C++ oracle array")
        .iter()
        .enumerate()
    {
        rust_network_rule_goal_placement(case, case_index);
    }

    // C4PlayerList.cpp:172-178,288-294. The C++ oracle compiles the exact
    // linked-list count and Join capacity gate; Rust seeds and attempts every
    // row through Engine::join_player, including the zero-is-closed boundary.
    let player_join_capacity_cases = golden["player_join_capacity"]
        .as_array()
        .expect("player_join_capacity is a C++ oracle array");
    let player_join_capacity_names = player_join_capacity_cases
        .iter()
        .map(|case| {
            case["name"]
                .as_str()
                .expect("player_join_capacity case has a name")
        })
        .collect::<Vec<_>>();
    assert_eq!(
        player_join_capacity_names,
        [
            "zero_rejects_empty",
            "below_limit_accepts",
            "at_limit_rejects",
        ],
        "player_join_capacity must retain its exact ordered three-row matrix"
    );
    for (case_index, case) in player_join_capacity_cases.iter().enumerate() {
        rust_player_join_capacity(case, case_index);
    }

    // C4PlayerInfo.cpp:1102-1118. The four MatchingLevel passes
    // RestoreSavegameInfos runs (:1373-1391) when it associates joining players
    // with a savegame's stored players. The C++ oracle compiles the extracted
    // switch, including PML_PlrFileName's fallthrough into PML_PlrName, so a
    // file-name match alone never associates.
    let savegame_matching_cases = golden["savegame_player_matching"]
        .as_array()
        .expect("savegame_player_matching is a C++ oracle array");
    assert_eq!(
        savegame_matching_cases.len(),
        9,
        "savegame_player_matching must retain its exact nine-row matrix"
    );
    for (case_index, case) in savegame_matching_cases.iter().enumerate() {
        rust_savegame_player_matching(case, case_index);
    }

    // C4PlayerInfo.cpp:1373-1391. The pass loop those levels run inside: no
    // shipped scenario sets Head.SaveGame, so this path is reachable only from
    // runtime-written saves and had no differential coverage at all.
    let savegame_association_cases = golden["savegame_association"]
        .as_array()
        .expect("savegame_association is a C++ oracle array");
    assert_eq!(
        savegame_association_cases.len(),
        6,
        "savegame_association must retain its exact six-row matrix"
    );
    for (case_index, case) in savegame_association_cases.iter().enumerate() {
        rust_savegame_association(case, case_index);
    }

    // C4IDList.cpp:33-103. Component order is inside the replay hash, so a
    // model that reordered or collapsed entries is a desync this comparator
    // can now see directly rather than only as an eventual hash mismatch.
    let component_order_cases = golden["component_order"]
        .as_array()
        .expect("component_order is a C++ oracle array");
    assert_eq!(
        component_order_cases.len(),
        6,
        "component_order must retain its exact six-row matrix"
    );
    for (case_index, case) in component_order_cases.iter().enumerate() {
        rust_component_order(case, case_index);
    }

    // 0. C4PXSSystem slot allocation (C4PXS.cpp:181-204, 426-437). The order a
    //    freed slot is handed back in decides the chunk-major execution order
    //    of every later pass, so it is compared against the real `New` rather
    //    than assumed. The golden frees high-index-first, which a
    //    most-recently-freed allocator would answer differently on the very
    //    next call.
    {
        let mut system = crate::pxs::PxsSystem::default();
        let mut live: Vec<(usize, usize)> = Vec::new();
        let material = crate::material::MaterialId::new(1).expect("material 1");
        // The oracle locates a returned pointer; the port has no pointer to
        // hand back, so each pixel carries a unique x and is located by it.
        let locate = |system: &crate::pxs::PxsSystem, tag: i32| {
            for chunk in 0..crate::pxs::PXS_MAX_CHUNK {
                for slot in 0..crate::pxs::PXS_CHUNK_SIZE {
                    if let Some(pxs) = system.peek_slot(chunk, slot) {
                        if pxs.x == itofix(tag) {
                            return Some((chunk, slot));
                        }
                    }
                }
            }
            None
        };

        for (idx, e) in golden["pxs_allocation"]
            .as_array()
            .unwrap()
            .iter()
            .enumerate()
        {
            let step = e["step"].as_str().unwrap_or_default();
            let (chunk, slot) = (i(e, "chunk"), i(e, "slot"));
            if let Some(freed) = step.strip_prefix("free") {
                let which: usize = freed.parse().expect("a free step names an index");
                let (chunk_at, slot_at) = live[which];
                expect_eq("pxs_allocation", idx, "chunk", chunk, chunk_at as i64);
                expect_eq("pxs_allocation", idx, "slot", slot, slot_at as i64);
                system.clear_slot(chunk_at, slot_at);
                live.remove(which);
                continue;
            }
            let tag = idx as i32;
            assert!(
                system.create_unchecked(material, itofix(tag), itofix(0), itofix(0), itofix(0),),
                "the golden sequence never exhausts the chunk table"
            );
            let placed = locate(&system, tag).expect("the created pixel is in a slot");
            expect_eq("pxs_allocation", idx, "chunk", chunk, placed.0 as i64);
            expect_eq("pxs_allocation", idx, "slot", slot, placed.1 as i64);
            live.push(placed);
        }
    }

    // 0b. mrfPoof's synchronised-draw discipline (C4Material.cpp:663-688). The
    //     arm extracts the landscape material, then draws Rnd3 twice: smoke on
    //     the first zero, a positional sound on the second. Both draws happen
    //     unconditionally and in that order, and — the parity fact worth
    //     pinning — neither touches the synchronised ledger, because Rnd3 reads
    //     the Randomize3 table rather than `Random`. A port that skipped the
    //     sound's draw when it had no sound to play, or that routed either
    //     through `Random`, would desynchronise everything downstream.
    for (idx, e) in golden["material_poof_reaction"]
        .as_array()
        .unwrap()
        .iter()
        .enumerate()
    {
        let seed = i(e, "seed") as i32;
        let library = MaterialLibrary::parse(
            r#"
            [Material Vacuum]
            Name=Vacuum
            Density=0

            [Material Water]
            Name=Water
            Density=25

            [Material Granite]
            Name=Granite
            Density=50
            "#,
        )
        .expect("poof RNG oracle materials parse");
        const WDT: u32 = 5;
        const HGT: u32 = 5;
        const PX: i32 = 2;
        const PY: i32 = 2;
        let mut bytes = vec![0; WDT as usize * HGT as usize];
        bytes[PY as usize * WDT as usize + PX as usize] = 2;
        let mut densities = vec![0; 128];
        densities[1] = 25;
        densities[2] = 50;
        let mut names = vec![None; 128];
        names[1] = Some("Water".to_string());
        names[2] = Some("Granite".to_string());

        let mut engine = Engine::with_seed(0);
        engine.configure_materials_from_library(&library);
        register_smoke_probe(&mut engine);
        let mut landscape = Landscape::flat(WDT, HGT as i32);
        landscape.set_pixel_grid(PixelGrid::new(
            WDT,
            HGT,
            bytes,
            densities,
            names,
            vec![None; 128],
        ));
        landscape.set_world_height(HGT as i32);
        engine.set_landscape(landscape);
        engine.rng = LcgRng::new(seed as u32);
        engine.rng.randomize3();

        let water = crate::material::MaterialId::new(1).expect("poof Water material");
        let granite = crate::material::MaterialId::new(2).expect("poof Granite material");
        let mut pixel = crate::pxs::Pxs {
            mat: water.into(),
            x: itofix(PX),
            y: itofix(PY),
            xdir: C4Fixed::ZERO,
            ydir: C4Fixed::ZERO,
        };
        let (mut x, mut y) = (PX, PY);
        let mut pos_changed = false;
        let material_before = engine
            .landscape()
            .and_then(|landscape| landscape.material_at(PX, PY));
        let handled = engine.execute_pxs_reaction(
            crate::material::MaterialReaction {
                kind: crate::material::MaterialReactionKind::Poof,
                user_defined: false,
                insertion_check: false,
            },
            &mut x,
            &mut y,
            PX,
            PY,
            &mut pixel,
            Some(granite),
            match i(e, "event") {
                0 => MaterialInteractionEvent::PxsPos,
                1 => MaterialInteractionEvent::PxsMove,
                _ => MaterialInteractionEvent::MassMove,
            },
            &mut pos_changed,
        );
        let material_after = engine
            .landscape()
            .and_then(|landscape| landscape.material_at(PX, PY));
        let extractions = i64::from(material_before.is_some() && material_after.is_none());
        let sounds = engine
            .pending_audio
            .iter()
            .filter(|command| {
                matches!(
                    command,
                    crate::AudioCommand::PlaySoundAt { name, .. } if name == "Pshshsh"
                )
            })
            .count() as i64;

        expect_eq(
            "material_poof_reaction",
            idx,
            "handled",
            i(e, "handled"),
            i64::from(handled),
        );
        expect_eq(
            "material_poof_reaction",
            idx,
            "extractions",
            i(e, "extractions"),
            extractions,
        );
        expect_eq(
            "material_poof_reaction",
            idx,
            "smoke",
            i(e, "smoke"),
            smoke_probe_count(&engine),
        );
        expect_eq(
            "material_poof_reaction",
            idx,
            "sound",
            i(e, "sound"),
            sounds,
        );
        expect_eq(
            "material_poof_reaction",
            idx,
            "random_count",
            i(e, "random_count"),
            i64::from(engine.rng.count),
        );
        expect_eq_u64(
            "material_poof_reaction",
            idx,
            "random_hold",
            u(e, "random_hold"),
            u64::from(engine.rng.hold),
        );
    }

    // 0c. C4MassMoverSet::Create's slot scan (C4MassMover.cpp:67-94). The
    //     search starts AFTER `CreatePtr` and wraps at the chunk end, so a slot
    //     freed behind the cursor is not reused until the cursor comes round to
    //     it — the opposite of the PXS allocator above, which always hands back
    //     the lowest free slot. Where a mover lands decides whether the frame's
    //     descending `Execute` pass reaches it again this pass or only the next,
    //     so the sequence of chosen slots is parity state.
    //
    //     The oracle stubs `Init` to succeed, which also holds its `Count` at
    //     zero, so `Create`'s `Count == C4MassMoverChunk` gate never fires there
    //     and this section pins the scan alone; the gate has its own test
    //     (`create_gate_is_exact_equality_on_count`).
    {
        let mut set = crate::mass_mover::MassMoverSet::default();
        let material = crate::material::MaterialId::new(1).expect("material 1");
        let take = |set: &mut crate::mass_mover::MassMoverSet| {
            set.find_free_slot()
                .map(|index| {
                    set.fill_slot(
                        index,
                        crate::mass_mover::MassMover {
                            mat: material,
                            x: 7,
                            y: 9,
                        },
                    );
                })
                .is_some()
        };

        for (idx, e) in golden["mass_mover_allocation"]
            .as_array()
            .unwrap()
            .iter()
            .enumerate()
        {
            let step = e["step"].as_str().unwrap_or_default();
            let ok = match step {
                "free_behind" | "free_for_wrap" => {
                    set.cease(1);
                    true
                }
                "free_behind_again" => {
                    set.cease(2);
                    true
                }
                // Fill the chunk, then record where the cursor stopped.
                "full" => {
                    while take(&mut set) {}
                    false
                }
                _ => take(&mut set),
            };
            expect_eq(
                "mass_mover_allocation",
                idx,
                "ok",
                i(e, "ok"),
                i64::from(ok),
            );
            expect_eq(
                "mass_mover_allocation",
                idx,
                "create_ptr",
                i(e, "create_ptr"),
                i64::from(set.create_ptr()),
            );
        }
    }

    // 0d. Splash's draw stream (C4Effect.cpp:801-836), the liquid-entry effect
    //     that `C4Object::UpdateInLiquid` and the movement InLiquid check fire
    //     on entry. Two things make it worth pinning against the real body
    //     rather than a restatement:
    //
    //     * both `Random` pairs are written with an explicit r2-before-r1
    //       temporary to force the evaluation order, so a port that draws them
    //       left to right swaps every bubble's x and y offset; and
    //     * the extraction inside the loop empties the very pixel the liquid
    //       test reads, so the first iteration takes four draws and every later
    //       one takes two. The draw COUNT is landscape-dependent, which is what
    //       makes a wrong one desynchronise everything downstream rather than
    //       merely move some spray.
    for (idx, e) in golden["splash_effect"]
        .as_array()
        .unwrap()
        .iter()
        .enumerate()
    {
        // The grid the oracle scaffolds for each case.
        let mut probe = match e["case"].as_str().unwrap_or_default() {
            "roofed" => SplashProbe::water_column(4),
            "not_instable" => SplashProbe::new(18, SplashProbe::HEIGHT, 1),
            "in_sky" => SplashProbe::water_column(30),
            "shallow" => SplashProbe::new(19, 22, 0),
            _ => SplashProbe::water_column(18),
        };
        probe.rng = LcgRng::new(i(e, "seed") as u32);
        crate::engine_splash::run_splash(&mut probe, 4, 20, i(e, "amt") as i32)
            .expect("the probe is infallible");

        expect_json_eq(
            "splash_effect",
            idx,
            "bubbles",
            e["bubbles"].clone(),
            serde_json::json!(probe.bubbles),
        );
        expect_json_eq(
            "splash_effect",
            idx,
            "casts",
            e["casts"].clone(),
            serde_json::json!(probe.casts),
        );
        expect_eq(
            "splash_effect",
            idx,
            "extractions",
            i(e, "extractions"),
            probe.extractions,
        );
        expect_eq(
            "splash_effect",
            idx,
            "random_count",
            i(e, "random_count"),
            i64::from(probe.rng.count),
        );
        expect_eq(
            "splash_effect",
            idx,
            "random_hold",
            i(e, "random_hold"),
            i64::from(probe.rng.hold),
        );
    }

    // 0e. C4Object::UpdateInLiquid (C4Object.cpp:6093-6110) and the probe it
    //     reads through (:5632-5635), driven through the same helpers both live
    //     call sites use (`engine/movement.rs`, `compat/object_state.rs`).
    //     Entry is edge-triggered and carries the splash; leaving is a bare flag
    //     clear. The probe sits at `y + Float * Con / FullCon - 1`, so a
    //     half-built object starts swimming at a different pixel — while the
    //     splash still originates at the object's own `y + 1`, which is why
    //     `float_reaches_water` enters the liquid and splashes nothing.
    for (idx, e) in golden["in_liquid_transition"]
        .as_array()
        .unwrap()
        .iter()
        .enumerate()
    {
        // (water_top, y, was_in_liquid, con, float, mass, hit_speed, wdt, hgt)
        let full = crate::FULL_CON;
        let case = e["case"].as_str().unwrap_or_default();
        let (water_top, y, was, con, float_line, mass, hit, wdt, hgt) = match case {
            "enter_splash" => (18, 20, false, full, 0, 10, true, 8, 10),
            "enter_no_hitspeed" => (18, 20, false, full, 0, 10, false, 8, 10),
            "enter_mass_boundary" => (18, 20, false, full, 0, 3, true, 8, 10),
            "enter_mass_above" => (18, 20, false, full, 0, 4, true, 8, 10),
            "stays_wet" => (18, 20, true, full, 0, 10, true, 8, 10),
            "stays_dry" => (30, 20, false, full, 0, 10, true, 8, 10),
            "leaves" => (30, 20, true, full, 0, 10, true, 8, 10),
            "float_reaches_water" => (18, 14, false, full, 6, 10, true, 8, 10),
            "half_con_falls_short" => (18, 14, false, full / 2, 6, 10, true, 8, 10),
            "large_object_clamps" => (18, 20, false, full, 0, 10, true, 40, 40),
            "small_object_amount" => (18, 20, false, full, 0, 10, true, 5, 6),
            other => panic!("unhandled in_liquid_transition case `{other}`"),
        };

        let mut probe = SplashProbe::water_column(water_top);
        probe.rng = LcgRng::new(i(e, "seed") as u32);

        let probe_y = crate::engine_splash::liquid_probe_y(y, float_line, con);
        let wet = crate::engine_splash::SplashHost::splash_is_liquid(&probe, 4, probe_y);

        let mut in_liquid = was;
        if crate::engine_splash::entered_liquid(wet, was) {
            let ocf = if hit { crate::ocf::HIT_SPEED2 } else { 0 };
            if crate::engine_splash::should_splash(wet, was, ocf, mass) {
                let amount = crate::engine_splash::splash_amount(wdt, hgt);
                crate::engine_splash::run_splash(&mut probe, 4, y + 1, amount)
                    .expect("the probe is infallible");
            }
            in_liquid = true;
        } else if !wet && was {
            in_liquid = false;
        }

        expect_eq(
            "in_liquid_transition",
            idx,
            "probe_y",
            i(e, "probe_y"),
            i64::from(probe_y),
        );
        expect_eq(
            "in_liquid_transition",
            idx,
            "wet",
            i(e, "wet"),
            i64::from(wet),
        );
        expect_eq(
            "in_liquid_transition",
            idx,
            "in_liquid",
            i(e, "in_liquid"),
            i64::from(in_liquid),
        );
        expect_eq(
            "in_liquid_transition",
            idx,
            "bubbles",
            i(e, "bubbles"),
            probe.bubbles.len() as i64,
        );
        expect_eq(
            "in_liquid_transition",
            idx,
            "casts",
            i(e, "casts"),
            probe.casts.len() as i64,
        );
        expect_eq(
            "in_liquid_transition",
            idx,
            "random_count",
            i(e, "random_count"),
            i64::from(probe.rng.count),
        );
        expect_eq(
            "in_liquid_transition",
            idx,
            "random_hold",
            i(e, "random_hold"),
            i64::from(probe.rng.hold),
        );
    }

    // 0f. C4Weather::Execute's disaster block (C4Weather.cpp:104-148). Four
    //     gates in a fixed order, and each gate spends its `Random(100)` level
    //     test EVEN AT LEVEL ZERO — so `all_levels_zero` draws 1629 times over
    //     400 ticks and fires nothing, while the same seed at full levels draws
    //     1696 and fires 37 disasters. A port that skipped the test when the
    //     level was zero, or reordered the gates, would desynchronise from the
    //     first tick a gate opens.
    //
    //     The launch helpers create an object and call Activate; the oracle
    //     records their arguments instead. The fixture registers those four
    //     definitions with no script functions, so both sides spend exactly the
    //     draws `Execute` itself makes.
    for (case_index, case) in golden["weather_execute"]
        .as_array()
        .unwrap()
        .iter()
        .enumerate()
    {
        let width = i(case, "width") as i32;
        let height = i(case, "height") as i32;

        let mut engine = Engine::with_seed(i(case, "seed") as u64);
        // C4Weather's Launch helpers create the effect object and call
        // Activate on it (C4Weather.cpp:153-165, 196-203, 178-184); the port
        // only records the weather event when that call succeeds, so the
        // fixture's definitions answer it. The body draws nothing, which is
        // what keeps the ledger comparable.
        for id in ["METO", "FXL1", "FXQ1", "FXV1"] {
            engine
                .register_definition(
                    Definition::from_script(
                        id,
                        id,
                        "#strict\npublic func Activate() { return 1; }\n",
                    )
                    .expect("weather effect compiles"),
                )
                .expect("weather effect registers");
        }
        let mut landscape = Landscape::flat(width as u32, height);
        landscape.set_world_height(height);
        // C4Landscape's TopOpen decides where the meteor spawns and whether it
        // gets a downward ydir (C4Weather.cpp:117-119).
        landscape.set_border_open(0, 0, i(case, "top_open") != 0, false);
        engine.landscape = Some(landscape);

        let mut environment = crate::EnvironmentSettings::new(0);
        environment.meteorite = i(case, "meteorite") as i32;
        environment.lightning = i(case, "lightning") as i32;
        environment.earthquake = i(case, "earthquake") as i32;
        environment.volcano = i(case, "volcano") as i32;
        engine.set_environment(environment);
        engine.weather_events.clear();

        let rows = case["ticks"].as_array().unwrap();
        let mut row_index = 0;
        for tick in 0..400_i64 {
            let before = engine.weather_events.len();
            engine
                .tick_weather_events((tick as u64 + 1) * 10)
                .expect("weather tick runs");
            let fired = engine.weather_events[before..]
                .iter()
                .map(|event| match event {
                    // The oracle records the meteorite's spawn arguments; the
                    // port carries only its x on the event and the rest on the
                    // object it spawned, so only x is compared here.
                    crate::WeatherEvent::Meteorite { x } => ("meteorite", *x),
                    crate::WeatherEvent::Lightning { position } => ("lightning", *position),
                    crate::WeatherEvent::Earthquake { x, .. } => ("earthquake", *x),
                    crate::WeatherEvent::Volcano { x, .. } => ("volcano", *x),
                })
                .collect::<Vec<_>>();

            let Some(row) = rows.get(row_index).filter(|row| i(row, "tick") == tick) else {
                assert!(
                    fired.is_empty(),
                    "PARITY DIVERGENCE in `weather_execute` case {case_index}: tick {tick} fired \
                     {fired:?} where the C++ golden recorded nothing"
                );
                continue;
            };
            row_index += 1;

            let expected = row["events"]
                .as_array()
                .unwrap()
                .iter()
                // `meteorite_rdir` is the oracle's continuation row for the
                // meteor's rotation velocity, not a second event.
                .filter(|event| event["kind"].as_str() != Some("meteorite_rdir"))
                .map(|event| {
                    (
                        event["kind"].as_str().unwrap_or_default().to_owned(),
                        i(event, "a") as i32,
                    )
                })
                .collect::<Vec<_>>();
            let actual = fired
                .iter()
                .map(|(kind, x)| ((*kind).to_owned(), *x))
                .collect::<Vec<_>>();
            assert_eq!(
                expected, actual,
                "PARITY DIVERGENCE in `weather_execute` case {case_index} tick {tick} events"
            );
            expect_eq(
                "weather_execute",
                case_index,
                "random_count",
                i(row, "random_count"),
                i64::from(engine.rng.count),
            );
            expect_eq_u64(
                "weather_execute",
                case_index,
                "random_hold",
                u(row, "random_hold"),
                u64::from(engine.rng.hold),
            );
        }
        assert_eq!(
            row_index,
            rows.len(),
            "PARITY DIVERGENCE in `weather_execute` case {case_index}: \
             the port never reached every recorded tick"
        );
    }

    // 0g. C4Shape::ContactCheck (C4Shape.cpp:370-406), the per-pixel probe every
    //     step of C4Object::DoMovement runs, for this bounded matrix. It decides
    //     ContactCNAT, ContactCount and the per-vertex VtxContactCNAT, so a
    //     vertex that answers differently by one pixel moves the object
    //     differently for the rest of the frame.
    //
    //     Its density reads go through GetPix's border rules
    //     (C4Landscape.h:163-180), where a CLOSED border answers MCVehic —
    //     solid — rather than sky. That is what stops an object at the edge of
    //     the map instead of letting it walk out of the world, and the
    //     `*_border` cases pin it from both sides.
    {
        let library = contact_oracle_materials();

        for (idx, case) in golden["shape_contact_check"]
            .as_array()
            .unwrap()
            .iter()
            .enumerate()
        {
            let mut engine = Engine::with_seed(0);
            engine.configure_materials_from_library(&library);
            install_contact_oracle_landscape(
                &mut engine,
                i(case, "left_open") as i32,
                i(case, "right_open") as i32,
                i(case, "top_open") != 0,
                i(case, "bottom_open") != 0,
            );
            let landscape = engine
                .landscape()
                .expect("contact oracle landscape remains");

            let rows = case["vertices"].as_array().expect("case vertices");
            let vertices = rows
                .iter()
                .map(|row| {
                    crate::ObjectVertex::new(i(row, "x") as i32, i(row, "y") as i32)
                        .with_cnat(i(row, "cnat") as u32)
                })
                .collect::<Vec<_>>();
            let position = crate::Vector2::new(i(case, "at_x") as i32, i(case, "at_y") as i32);
            let contact = crate::shape_contact_check(
                &vertices,
                position,
                landscape,
                &engine.materials,
                &[],
                None,
                i(case, "contact_density") as i32,
            );

            expect_eq(
                "shape_contact_check",
                idx,
                "any",
                i(case, "any"),
                i64::from(u8::from(contact.is_contact())),
            );
            expect_eq(
                "shape_contact_check",
                idx,
                "contact_cnat",
                i(case, "contact_cnat"),
                i64::from(contact.contact_cnat),
            );
            expect_eq(
                "shape_contact_check",
                idx,
                "contact_count",
                i(case, "contact_count"),
                i64::from(contact.count()),
            );
            for (vertex_index, row) in rows.iter().enumerate() {
                expect_eq(
                    "shape_contact_check",
                    idx,
                    "vertex contact_cnat",
                    i(row, "contact_cnat"),
                    i64::from(contact.vertex_contacts[vertex_index]),
                );
                // C4Shape stores VtxContactMat, which the port does not carry on
                // ShapeContact — so the material is compared through the
                // landscape probe both sides read, GBackMat
                // (C4Wrappers.h:179-182). A CNAT_NoCollision vertex is skipped
                // before that assignment, so its golden value is the fixture's
                // own initialiser rather than an engine answer.
                if i(row, "cnat") & 64 != 0 {
                    continue;
                }
                let expected = match i(row, "mat") {
                    -1 => None,
                    1 => Some("Earth"),
                    2 => Some("Water"),
                    3 => Some("Vehicle"),
                    other => panic!("unmapped oracle material index {other}"),
                };
                let actual = landscape
                    .border_material_at(
                        position.x + i(row, "x") as i32,
                        position.y + i(row, "y") as i32,
                    )
                    .and_then(|id| engine.materials.get_by_id(id))
                    .map(|material| material.name().to_owned());
                assert_eq!(
                    expected,
                    actual.as_deref(),
                    "PARITY DIVERGENCE in `shape_contact_check` entry {idx} vertex \
                     {vertex_index} material"
                );
            }
        }
    }

    // 0h. C4Object::TargetBounds (C4Movement.cpp:128-164), the clamp
    //     SideBounds and VerticalBounds run every movement target through. Both
    //     comparisons are strict, so sitting exactly on a limit is not a
    //     crossing; and when the limits cross each other, clamping to the low
    //     one puts the target above the high one, so BOTH bounds fire with the
    //     low contact first.
    //
    //     The port splits the C++ body: `target_bounds` returns which bounds
    //     fired, and its callers clear `fixed_velocity.x` for the side pair and
    //     `.y` for the vertical one. The golden records the C++ zeroing for the
    //     record; what is compared here is the clamp and the contact order the
    //     shared function decides.
    for (idx, case) in golden["target_bounds"]
        .as_array()
        .unwrap()
        .iter()
        .enumerate()
    {
        let mut target = i(case, "target") as i32;
        let contacts = crate::target_bounds(
            &mut target,
            i(case, "low") as i32,
            i(case, "high") as i32,
            i(case, "cnat_low") as u32,
            i(case, "cnat_hi") as u32,
        );

        expect_eq(
            "target_bounds",
            idx,
            "bounded",
            i(case, "bounded"),
            i64::from(target),
        );
        let expected = case["contacts"]
            .as_array()
            .unwrap()
            .iter()
            .map(|value| value.as_i64().unwrap())
            .collect::<Vec<_>>();
        let actual = contacts
            .into_iter()
            .flatten()
            .map(i64::from)
            .collect::<Vec<_>>();
        assert_eq!(
            expected, actual,
            "PARITY DIVERGENCE in `target_bounds` entry {idx} contacts"
        );
    }

    // 0i. C4Shape::Attach (C4Shape.cpp:165-271), the search attached movement
    //     runs instead of the ordinary collision loop. The two branches differ
    //     in a way that shows up in play: the old-style search loops vertices
    //     OUTSIDE and the range inside, so a second matching vertex starts from
    //     the position the first already moved to — `two_vertices_old_style`
    //     ends up BELOW the surface at y=11 — while CNAT_MultiAttach loops the
    //     range outside and takes the nearest attachment across all vertices,
    //     landing on the surface at y=9. That is the "stucking" the C++ comment
    //     at C4Shape.cpp:179-194 describes, and it is why both branches exist.
    //
    //     `closed_border_no_attach` pins the other asymmetry worth knowing: a
    //     closed border answers solid to a density probe, but Attach also
    //     requires `ax >= 0`, so an object can CONTACT the edge of the map
    //     without attaching to it.
    {
        let library = contact_oracle_materials();

        for (idx, case) in golden["shape_attach"]
            .as_array()
            .unwrap()
            .iter()
            .enumerate()
        {
            let mut engine = Engine::with_seed(0);
            engine.configure_materials_from_library(&library);
            install_contact_oracle_landscape(
                &mut engine,
                i(case, "left_open") as i32,
                i(case, "right_open") as i32,
                i(case, "top_open") != 0,
                i(case, "bottom_open") != 0,
            );
            let landscape = engine.landscape().expect("attach oracle landscape remains");

            let vertices = case["vertices"]
                .as_array()
                .expect("case vertices")
                .iter()
                .map(|row| {
                    crate::ObjectVertex::new(i(row, "x") as i32, i(row, "y") as i32)
                        .with_cnat(i(row, "cnat") as u32)
                })
                .collect::<Vec<_>>();
            let mut position = crate::Vector2::new(i(case, "at_x") as i32, i(case, "at_y") as i32);
            let mut record = crate::ShapeAttachRecord::default();
            let attached = crate::shape_attach(
                &vertices,
                &mut position,
                i(case, "attach") as u32,
                landscape,
                &engine.materials,
                &[],
                None,
                50,
                &mut record,
            );

            expect_eq(
                "shape_attach",
                idx,
                "attached",
                i(case, "attached"),
                i64::from(u8::from(attached)),
            );
            expect_eq(
                "shape_attach",
                idx,
                "x",
                i(case, "x"),
                i64::from(position.x),
            );
            expect_eq(
                "shape_attach",
                idx,
                "y",
                i(case, "y"),
                i64::from(position.y),
            );
            // C4Shape keeps AttachMat itself; the port keeps only whether the
            // attachment landed on a valid material and whether that material
            // is Vehicle, so the oracle's index is compared through those two.
            expect_eq(
                "shape_attach",
                idx,
                "attach_mat valid",
                i64::from(i(case, "attach_mat") >= 0),
                i64::from(u8::from(record.mat_valid)),
            );
            expect_eq(
                "shape_attach",
                idx,
                "attach_mat vehicle",
                i64::from(i(case, "attach_mat") == 3),
                i64::from(u8::from(record.mat_vehicle)),
            );
            // The position fields only overwrite on success
            // (C4Shape.cpp:217-219, 253-255).
            if attached {
                expect_eq(
                    "shape_attach",
                    idx,
                    "attach_x",
                    i(case, "attach_x"),
                    i64::from(record.x),
                );
                expect_eq(
                    "shape_attach",
                    idx,
                    "attach_y",
                    i(case, "attach_y"),
                    i64::from(record.y),
                );
                expect_eq(
                    "shape_attach",
                    idx,
                    "attach_vtx",
                    i(case, "attach_vtx"),
                    i64::from(record.vtx),
                );
            }
        }
    }

    // 0j-a. Raw C4ObjectList contents ordering and link identity. The oracle
    //       compiles Add/Remove/GetLink/InsertLink/RemoveLink/ShiftContents
    //       directly from C4ObjectList.cpp:110-238 (Add), :240-268 (Remove),
    //       :310-318 (GetLink), :614-618 (RemoveLink), :620-636 (InsertLink)
    //       and :815-831 (ShiftContents). Rust
    //       drives the corresponding loaded tail-add, runtime Enter, script
    //       ShiftContents and removal/re-entry paths; the local serial ledger
    //       only normalizes Rust's per-object generations to C++'s global link
    //       allocation counter.
    {
        struct ContentsListProbe {
            engine: Engine,
            container: ObjectId,
            tags: HashMap<ObjectId, String>,
            serials: HashMap<(ObjectId, u64), u64>,
            next_serial: u64,
        }

        impl ContentsListProbe {
            fn new() -> Self {
                let mut engine = Engine::with_seed(0);
                let container = Definition::from_script(
                    "CONT",
                    "CONT",
                    "#strict\npublic func ShiftToB() { return ShiftContents(); }\n",
                )
                .expect("contents-list container compiles");
                engine
                    .register_definition(container)
                    .expect("contents-list container registers");
                for id in [
                    "SAME", "HIGH", "LOW", "MID", "ITMA", "ITMB", "ITMC", "SBA", "SBB", "LINE",
                    "UNSO", "DEAD", "NEWC",
                ] {
                    let mut definition = Definition::from_script(id, id, "#strict\n")
                        .expect("contents-list item compiles");
                    if id == "LINE" {
                        definition.set_line(1);
                    }
                    engine
                        .register_definition(definition)
                        .expect("contents-list item registers");
                }
                let container = engine
                    .spawn_object(SpawnConfig::new("CONT"))
                    .expect("contents-list container spawns");
                Self {
                    engine,
                    container,
                    tags: HashMap::new(),
                    serials: HashMap::new(),
                    next_serial: 0,
                }
            }

            fn add(
                &mut self,
                tag: &str,
                definition: &str,
                category: i32,
                loaded: bool,
            ) -> ObjectId {
                let mut config = SpawnConfig::new(definition).with_category(category);
                if loaded {
                    config = config.with_container(self.container).with_loaded(true);
                }
                let object = self
                    .engine
                    .spawn_object(config)
                    .expect("contents-list item spawns");
                self.tags.insert(object, tag.to_string());
                if loaded {
                    self.record_allocation(object);
                }
                object
            }

            fn record_allocation(&mut self, object: ObjectId) {
                self.next_serial += 1;
                let generation = self.generation(object);
                assert_ne!(generation, 0, "a contained object has a live link");
                assert!(
                    self.serials
                        .insert((object, generation), self.next_serial)
                        .is_none(),
                    "every C4ObjectLink incarnation is unique"
                );
            }

            fn generation(&self, object: ObjectId) -> u64 {
                let index = self
                    .engine
                    .find_object_index(object)
                    .expect("contents-list item remains materialized");
                self.engine.objects[index].state.contents_link_generation
            }

            fn enter(&mut self, object: ObjectId) {
                let index = self
                    .engine
                    .find_object_index(object)
                    .expect("contents-list item remains materialized");
                let previous = self.engine.objects[index].state.container;
                self.engine
                    .apply_container_change(object, previous, Some(self.container), false)
                    .expect("runtime contents insertion succeeds");
                self.record_allocation(object);
            }

            fn exit(&mut self, object: ObjectId) {
                self.engine
                    .apply_container_change(object, Some(self.container), None, false)
                    .expect("runtime contents removal succeeds");
            }

            fn set_unsorted(&mut self, object: ObjectId) {
                let index = self
                    .engine
                    .find_object_index(object)
                    .expect("contents-list item remains materialized");
                self.engine.objects[index].unsorted = true;
            }

            fn set_deleted(&mut self, object: ObjectId) {
                let index = self
                    .engine
                    .find_object_index(object)
                    .expect("contents-list item remains materialized");
                self.engine.objects[index].state.status = ObjectStatus::Deleted;
            }

            fn make_picture_distinct(&mut self, object: ObjectId) {
                let index = self
                    .engine
                    .find_object_index(object)
                    .expect("contents-list item remains materialized");
                self.engine.objects[index].state.color_modulation = 0x0080_8080;
            }

            fn shift_to_b(&mut self) {
                let index = self
                    .engine
                    .find_object_index(self.container)
                    .expect("contents-list container remains materialized");
                let result = self
                    .engine
                    .call_object_function(index, "ShiftToB", Vec::new())
                    .expect("ShiftContents callback succeeds");
                assert_eq!(result, ScriptValue::Bool(true));
            }

            fn links(&self) -> Vec<(ObjectId, u64)> {
                let index = self
                    .engine
                    .find_object_index(self.container)
                    .expect("contents-list container remains materialized");
                self.engine.objects[index]
                    .state
                    .contents
                    .iter()
                    .copied()
                    .map(|object| (object, self.generation(object)))
                    .collect()
            }

            fn tag(&self, object: ObjectId) -> String {
                self.tags
                    .get(&object)
                    .expect("every probe object has a tag")
                    .clone()
            }

            fn snapshot(&self) -> (Vec<String>, Vec<u64>) {
                self.links()
                    .into_iter()
                    .map(|(object, generation)| {
                        let tag = self.tag(object);
                        let serial = *self
                            .serials
                            .get(&(object, generation))
                            .expect("every live link incarnation has a serial");
                        (tag, serial)
                    })
                    .unzip()
            }
        }

        let cases = golden["contents_list_order"].as_array().unwrap();
        assert_eq!(
            cases
                .iter()
                .filter_map(|case| case["case"].as_str())
                .collect::<Vec<_>>(),
            [
                "same_id_newest_first",
                "relative_category_descending",
                "equal_category_new_cluster_first",
                "static_back_skips_id_cluster",
                "line_object_tail",
                "unsorted_object_tail",
                "dead_and_unsorted_existing_ignored",
                "st_none_tail",
                "shift_contents_preserves_link",
                "remove_add_allocates_fresh_link",
            ],
            "the mechanically extracted contents-list matrix is complete"
        );
        for (idx, case) in cases.iter().enumerate() {
            let name = case["case"].as_str().unwrap_or_default();
            let mut probe = ContentsListProbe::new();
            let mut tracked_before = None;
            let mut tracked_after = None;
            let mut iterator_after_remove = None;
            match name {
                "same_id_newest_first" => {
                    probe.add("old", "SAME", CATEGORY_OBJECT, true);
                    let new = probe.add("new", "SAME", CATEGORY_OBJECT, false);
                    probe.enter(new);
                }
                "relative_category_descending" => {
                    probe.add("high", "HIGH", CATEGORY_OBJECT, true);
                    probe.add("low", "LOW", CATEGORY_STATIC_BACK, true);
                    let middle = probe.add("middle", "MID", CATEGORY_LIVING, false);
                    probe.enter(middle);
                }
                "equal_category_new_cluster_first" => {
                    probe.add("old-a", "ITMA", CATEGORY_OBJECT, true);
                    probe.add("old-c", "ITMC", CATEGORY_OBJECT, true);
                    let new_b = probe.add("new-b", "ITMB", CATEGORY_OBJECT, false);
                    probe.enter(new_b);
                }
                "static_back_skips_id_cluster" => {
                    probe.add("old-b", "SBB", CATEGORY_STATIC_BACK, true);
                    probe.add("old-a", "SBA", CATEGORY_STATIC_BACK, true);
                    let new_a = probe.add("new-a", "SBA", CATEGORY_STATIC_BACK, false);
                    probe.enter(new_a);
                }
                "line_object_tail" => {
                    probe.add("low", "LOW", CATEGORY_STATIC_BACK, true);
                    let line = probe.add("line", "LINE", CATEGORY_OBJECT, false);
                    probe.enter(line);
                }
                "unsorted_object_tail" => {
                    probe.add("low", "LOW", CATEGORY_STATIC_BACK, true);
                    let unsorted = probe.add("unsorted", "UNSO", CATEGORY_OBJECT, false);
                    probe.set_unsorted(unsorted);
                    probe.enter(unsorted);
                }
                "dead_and_unsorted_existing_ignored" => {
                    let dead = probe.add("dead", "DEAD", CATEGORY_OBJECT, true);
                    let unsorted = probe.add("unsorted", "UNSO", CATEGORY_OBJECT, true);
                    probe.add("low", "LOW", CATEGORY_STATIC_BACK, true);
                    probe.set_deleted(dead);
                    probe.set_unsorted(unsorted);
                    let new = probe.add("new", "NEWC", CATEGORY_LIVING, false);
                    probe.enter(new);
                }
                "st_none_tail" => {
                    probe.add("old", "SAME", CATEGORY_OBJECT, true);
                    probe.add("new", "SAME", CATEGORY_OBJECT, true);
                }
                "shift_contents_preserves_link" => {
                    probe.add("a", "SAME", CATEGORY_OBJECT, true);
                    let b = probe.add("b", "SAME", CATEGORY_OBJECT, true);
                    probe.add("c", "SAME", CATEGORY_OBJECT, true);
                    // Keep the oracle's one-definition list. A distinct
                    // picture makes public C4Object::ShiftContents select b,
                    // then both engines execute the raw list rotation.
                    probe.make_picture_distinct(b);
                    tracked_before = probe.serials.get(&(b, probe.generation(b))).copied();
                    probe.shift_to_b();
                    tracked_after = probe.serials.get(&(b, probe.generation(b))).copied();
                }
                "remove_add_allocates_fresh_link" => {
                    probe.add("a", "SAME", CATEGORY_OBJECT, true);
                    let b = probe.add("b", "SAME", CATEGORY_OBJECT, true);
                    let c = probe.add("c", "SAME", CATEGORY_OBJECT, true);
                    tracked_before = probe.serials.get(&(b, probe.generation(b))).copied();
                    let mut iterator = crate::direct_com::RemovalSafeContentsIterator::new(
                        probe.container,
                        &[(b, probe.generation(b)), (c, probe.generation(c))],
                    );
                    probe.exit(b);
                    probe.enter(b);
                    iterator_after_remove = iterator
                        .next(&probe.links())
                        .map(|object| probe.tag(object));
                    tracked_after = probe.serials.get(&(b, probe.generation(b))).copied();
                }
                other => panic!("unknown contents_list_order oracle case `{other}`"),
            }

            let (order, serials) = probe.snapshot();
            expect_json_eq(
                "contents_list_order",
                idx,
                "order",
                case["order"].clone(),
                serde_json::json!(order),
            );
            expect_json_eq(
                "contents_list_order",
                idx,
                "serials",
                case["serials"].clone(),
                serde_json::json!(serials),
            );
            if case.get("tracked_serial_before").is_some() {
                expect_eq_u64(
                    "contents_list_order",
                    idx,
                    "tracked_serial_before",
                    u(case, "tracked_serial_before"),
                    tracked_before.expect("tracked case records its initial link"),
                );
                expect_eq_u64(
                    "contents_list_order",
                    idx,
                    "tracked_serial_after",
                    u(case, "tracked_serial_after"),
                    tracked_after.expect("tracked case records its final link"),
                );
            }
            if let Some(expected) = case.get("iterator_after_remove") {
                expect_json_eq(
                    "contents_list_order",
                    idx,
                    "iterator_after_remove",
                    expected.clone(),
                    serde_json::json!(iterator_after_remove),
                );
            }
        }
    }

    // 0j-b. The container lifecycle: C4Object::Enter, Exit and Collect
    //     (C4Object.cpp:1532-1563, 1566-1637, 5693-5717), all three compiled
    //     from mechanically extracted bodies. What is pinned is the ORDER of
    //     their script calls and the re-checks between them:
    //
    //       * the recursion guard runs AFTER RejectEntrance, and
    //         RejectCollection only when the caller asked for the flag;
    //       * a Collection2 that exits the object abandons Entrance;
    //       * the re-check after Entrance tests the CONTAINER's status, not the
    //         entering object's, so directly clearing only the object's Status
    //         still reaches the base auto-sell tail while removing the
    //         container does not;
    //       * Exit reports failure when a Departure callback put the object
    //         back into a container, having already done everything; and
    //       * Collect's three Hit calls are gated on their own OCF bits and
    //         stop at the first that removes the object.
    //
    //     The oracle's `calls` list also records bookkeeping the port does not
    //     expose (SetOCF, UpdateMass, CloseMenu, UpdateFace); those entries
    //     document where the mutations sit between the script calls, and what
    //     is compared here is the script calls, which both engines can name.
    {
        // Base-11 digits, one per script callback, in the order they ran. Eight
        // calls is the longest sequence in the matrix, so the encoding stays
        // inside i32.
        let digit_of = |call: &str| -> Option<i64> {
            Some(match call {
                // The oracle records the PSF_ names verbatim, `~` and all.
                "~RejectEntrance" => 1,
                "~RejectCollect" => 2,
                "~Collection2" => 3,
                "~Entrance" => 4,
                "~Collection" => 5,
                "~Ejection" => 6,
                "~Departure" => 7,
                "~Hit" => 8,
                "~Hit2" => 9,
                "~Hit3" => 10,
                _ => return None,
            })
        };

        for (idx, case) in golden["container_lifecycle"]
            .as_array()
            .unwrap()
            .iter()
            .enumerate()
        {
            let name = case["case"].as_str().unwrap_or_default();
            let op = case["op"].as_str().unwrap_or_default();

            // Each configured callback's port-side effect, mirroring the
            // oracle's Effect for this case.
            let reject_entrance =
                i64::from(name == "enter_rejected" || name == "collect_enter_refused");
            let reject_collection = i64::from(name == "collect_rejected_by_container");
            let entrance_body = match name {
                "enter_entrance_clears_own_status" => "RemoveObject();",
                "enter_entrance_exits_object" => "Exit();",
                "enter_entrance_removes_container" => "pContainer->RemoveObject();",
                _ => "",
            };
            let departure_body = match name {
                "exit_reentered_by_script" => "Enter(FindObject(OUTS));",
                _ => "",
            };
            let hit_body = match name {
                "collect_hit_kills" => "RemoveObject();",
                _ => "",
            };
            let collection2_body = match name {
                "enter_collection2_exits_object" => "Exit(pObj);",
                _ => "",
            };

            let object_script = format!(
                "#strict\n\
                 static callback_log;\n\
                 protected func RejectEntrance(pTarget) {{ callback_log = callback_log * 11 + 1; return {reject_entrance}; }}\n\
                 protected func Entrance(pContainer) {{ callback_log = callback_log * 11 + 4; {entrance_body} }}\n\
                 protected func Departure(pContainer) {{ callback_log = callback_log * 11 + 7; {departure_body} }}\n\
                 protected func Hit() {{ callback_log = callback_log * 11 + 8; {hit_body} }}\n\
                 protected func Hit2() {{ callback_log = callback_log * 11 + 9; }}\n\
                 protected func Hit3() {{ callback_log = callback_log * 11 + 10; }}\n\
                 public func DoEnterNull() {{ return Enter(0); }}\n\
                 public func DoEnterSelf() {{ return Enter(this()); }}\n\
                 public func DoEnter(pTarget) {{ return Enter(pTarget); }}\n\
                 public func DoExit() {{ return Exit(this(), 106, 115, 33, 1, 2, 3); }}\n"
            );
            let container_script = format!(
                "#strict\n\
                 static callback_log;\n\
                 protected func RejectCollect(idDef, pObj) {{ callback_log = callback_log * 11 + 2; return {reject_collection}; }}\n\
                 protected func Collection2(pObj) {{ callback_log = callback_log * 11 + 3; {collection2_body} }}\n\
                 protected func Collection(pObj) {{ callback_log = callback_log * 11 + 5; }}\n\
                 protected func Ejection(pObj) {{ callback_log = callback_log * 11 + 6; }}\n\
                 public func DoCollect(pItem) {{ return Collect(pItem); }}\n\
                 public func ReadLog() {{ return callback_log; }}\n\
                 public func ResetLog() {{ callback_log = 0; return 1; }}\n"
            );

            let mut engine = Engine::with_seed(0);
            // The script-level Collect needs the collector to carry
            // OCF_Collection before it will reach C4Object::Collect at all
            // (C4Script.cpp:391-413), which a DefCore collection rect is what
            // grants.
            let mut container_definition =
                Definition::from_script("CTCN", "CTCN", container_script.as_str())
                    .expect("container lifecycle fixture compiles");
            container_definition
                .set_collection_rect(Some(crate::DefinitionRect::new(-12, -10, 24, 12)));
            engine
                .register_definition(container_definition)
                .expect("container lifecycle fixture registers");
            let mut object_definition =
                Definition::from_script("CTOB", "CTOB", object_script.as_str())
                    .expect("container lifecycle fixture compiles");
            object_definition.configure_actions(
                None,
                HashMap::from([(
                    "Attach".to_string(),
                    ActionSpec::default().with_procedure("ATTACH"),
                )]),
            );
            engine
                .register_definition(object_definition)
                .expect("container lifecycle fixture registers");
            // The old container an already-contained object exits from. It
            // also owns the surviving log reader for target-removal cases.
            engine
                .register_definition(
                    Definition::from_script(
                        "OUTS",
                        "OUTS",
                        "#strict\n\
                         static callback_log;\n\
                         protected func Ejection(pObj) { callback_log = callback_log * 11 + 6; }\n\
                         protected func Collection2(pObj) { callback_log = callback_log * 11 + 3; }\n\
                         public func ReadLog() { return callback_log; }\n\
                         public func ResetLog() { callback_log = 0; return 1; }\n",
                    )
                    .expect("container lifecycle fixture compiles"),
                )
                .expect("container lifecycle fixture registers");

            let mut object_config = SpawnConfig::new("CTOB")
                .with_controller(5)
                .with_position(crate::Vector2::new(5, 7))
                .with_fixed_position(FixedVec2::new(itofix(5), itofix(7)))
                .with_fixed_velocity(FixedVec2::new(
                    C4Fixed::from_raw(1111),
                    C4Fixed::from_raw(-2222),
                ))
                .with_mobile(false);
            if name == "enter_living_keeps_controller" {
                object_config = object_config
                    .with_alive(true)
                    .with_category(crate::CATEGORY_LIVING);
            }
            if name == "collect_cancels_attach" {
                object_config = object_config.with_action(ActionState::new("Attach"));
            }
            let object = engine
                .spawn_object(object_config)
                .expect("lifecycle object spawns");
            let container = engine
                .spawn_object(
                    SpawnConfig::new("CTCN")
                        .with_controller(9)
                        .with_position(crate::Vector2::new(31, 37))
                        .with_fixed_position(FixedVec2::new(itofix(31), itofix(37)))
                        .with_fixed_velocity(FixedVec2::new(
                            C4Fixed::from_raw(12345),
                            C4Fixed::from_raw(-23456),
                        )),
                )
                .expect("lifecycle container spawns");
            let outside = engine
                .spawn_object(
                    SpawnConfig::new("OUTS")
                        .with_controller(2)
                        .with_position(crate::Vector2::new(71, 73))
                        .with_fixed_position(FixedVec2::new(itofix(71), itofix(73)))
                        .with_fixed_velocity(FixedVec2::new(
                            C4Fixed::from_raw(-34567),
                            C4Fixed::from_raw(45678),
                        )),
                )
                .expect("lifecycle outside container spawns");

            // `exit_not_contained` is the one case that must start free.
            if name == "enter_from_container" || (op == "exit" && name != "exit_not_contained") {
                let object_index = engine.find_object_index(object).expect("object exists");
                let outside_index = engine
                    .find_object_index(outside)
                    .expect("outside container exists");
                engine.objects[object_index].state.container = Some(outside);
                engine.objects[object_index].state.contents_link_generation = 1;
                engine.objects[outside_index].state.contents.push(object);
                engine.refresh_object_ocf(object_index);
            }
            if name == "enter_recursive" {
                let container_index = engine
                    .find_object_index(container)
                    .expect("container exists");
                let object_index = engine.find_object_index(object).expect("object exists");
                engine.objects[container_index].state.container = Some(object);
                engine.objects[container_index]
                    .state
                    .contents_link_generation = 1;
                engine.objects[object_index].state.contents.push(container);
                engine.refresh_object_ocf(container_index);
            }
            // Both sides derive hit-speed OCF bits from raw speed
            // (|xdir| + |ydir| >= 1.5 / 2 / 6), and Collect defers its
            // CopyMotion until after the Hit calls precisely so they are still
            // live there.
            if name.starts_with("collect_hit") {
                let index = engine.find_object_index(object).expect("object exists");
                let speed = if name == "collect_hit_speeds" { 9 } else { 3 };
                engine.objects[index].fixed_velocity = FixedVec2::new(itofix(speed), C4Fixed::ZERO);
                engine.objects[index].state.ocf |=
                    crate::movement_hit_speed_flags(engine.objects[index].fixed_velocity);
            }

            let object_index = engine.find_object_index(object).expect("object exists");
            let container_index = engine
                .find_object_index(container)
                .expect("container exists");
            let outside_index = engine
                .find_object_index(outside)
                .expect("outside container exists");
            engine
                .call_object_function(outside_index, "ResetLog", Vec::new())
                .expect("the log resets");

            let target_value = crate::compat::object_reference_value(container);
            let (runner_index, function, arguments) = match op {
                "enter" => match name {
                    "enter_null_target" => (object_index, "DoEnterNull", Vec::new()),
                    "enter_self" => (object_index, "DoEnterSelf", Vec::new()),
                    _ => (object_index, "DoEnter", vec![target_value]),
                },
                "exit" => (object_index, "DoExit", Vec::new()),
                _ => (
                    container_index,
                    "DoCollect",
                    vec![crate::compat::object_reference_value(object)],
                ),
            };
            let result = engine
                .call_object_function(runner_index, function, arguments)
                .expect("the lifecycle operation runs");

            expect_eq(
                "container_lifecycle",
                idx,
                "result",
                i(case, "result"),
                i64::from(
                    matches!(result, ScriptValue::Bool(true))
                        || matches!(result, ScriptValue::Int(value) if value != 0),
                ),
            );

            // The script-call order, encoded the same way on both sides.
            let expected_log = case["calls"]
                .as_array()
                .unwrap()
                .iter()
                .filter_map(|call| digit_of(call.as_str().unwrap_or_default()))
                .fold(0_i64, |log, digit| log * 11 + digit);
            // `callback_log = 0` folds its literal zero to nil below strict 3
            // (see the `zero_literal` section), so an untouched log reads as
            // Nil rather than Int(0).
            let log = match engine
                .call_object_function(outside_index, "ReadLog", Vec::new())
                .expect("the log reads back")
            {
                ScriptValue::Int(value) => i64::from(value),
                ScriptValue::Nil | ScriptValue::Bool(false) => 0,
                other => panic!("unexpected callback log value {other:?}"),
            };
            expect_eq(
                "container_lifecycle",
                idx,
                "callback order",
                expected_log,
                log,
            );

            let object = &engine.objects[object_index];
            let target = &engine.objects[container_index];
            let outside_index = engine
                .find_object_index(outside)
                .expect("outside container exists");
            let outside_state = &engine.objects[outside_index];
            expect_eq(
                "container_lifecycle",
                idx,
                "outside_contents",
                i(case, "outside_contents"),
                outside_state.state.contents.len() as i64,
            );
            // This oracle callback deliberately clears Status directly to
            // isolate Enter's guard placement. The real Rust RemoveObject
            // also unlinks containment, so only that row's link state differs.
            if name != "enter_entrance_clears_own_status" {
                for (field, rust) in [
                    (
                        "contained_is_target",
                        i64::from(u8::from(object.state.container == Some(container))),
                    ),
                    (
                        "contained_is_outside",
                        i64::from(u8::from(object.state.container == Some(outside))),
                    ),
                    ("target_contents", target.state.contents.len() as i64),
                ] {
                    expect_eq("container_lifecycle", idx, field, i(case, field), rust);
                }
            }
            for (field, rust) in [
                ("status", i64::from(object.state.status.to_script_value())),
                (
                    "target_status",
                    i64::from(target.state.status.to_script_value()),
                ),
                ("controller", i64::from(object.state.controller)),
                ("mobile", i64::from(u8::from(object.state.mobile))),
                ("in_liquid", i64::from(u8::from(object.state.in_liquid))),
                (
                    "ocf",
                    i64::from(
                        object.state.ocf
                            & (crate::ocf::NORMAL
                                | crate::ocf::HIT_SPEED1
                                | crate::ocf::HIT_SPEED2
                                | crate::ocf::HIT_SPEED3
                                | crate::ocf::HIT_SPEED4
                                | crate::ocf::NOT_CONTAINED
                                | crate::ocf::IN_LIQUID
                                | crate::ocf::IN_FREE
                                | crate::ocf::AVAILABLE),
                    ),
                ),
                (
                    "action_idle",
                    i64::from(u8::from(object.state.action.name == "Idle")),
                ),
                ("x", i64::from(object.state.position.x)),
                ("y", i64::from(object.state.position.y)),
                ("r", i64::from(object.state.rotation)),
                ("xdir", i64::from(object.fixed_velocity.x.val())),
                ("ydir", i64::from(object.fixed_velocity.y.val())),
                ("rdir", i64::from(object.rotation_velocity.val())),
            ] {
                // This row intentionally contrasts a direct C++ Status clear
                // with Rust's full RemoveObject teardown to isolate Enter's
                // guard. Its containment-derived cached OCF is consequently
                // outside the comparable state, like its raw links above.
                if name == "enter_entrance_clears_own_status" && field == "ocf" {
                    continue;
                }
                expect_eq("container_lifecycle", idx, field, i(case, field), rust);
            }
        }
    }

    // 0k. C4Effect::Check (C4Effect.cpp:271-316), the negotiation every
    //     AddEffect runs before a new effect exists. Three effects sit in the
    //     list at priorities 100, 60 and 20 and each case configures what their
    //     checker callbacks answer:
    //
    //       * priority 1 is always allowed and asks nobody;
    //       * only effects of AT LEAST the incoming priority are asked, so a
    //         low-priority denier cannot stop anything, and dead or
    //         callback-less effects are skipped;
    //       * a Deny short-circuits the walk, while an Annul only NOMINATES its
    //         effect — the walk continues and the LAST annulling effect is the
    //         one that absorbs, so `last_annul_wins` comes back with the third
    //         effect's number;
    //       * the AnnulCalls form brackets the FxAdd in temp-remove/temp-readd
    //         of the effects above the absorber, and both halves of that
    //         bracket test `pNext`, so an absorber at the end of the list gets
    //         no bracket at all; and
    //       * an FxAdd that answers Start_Deny kills the absorber and reports
    //         Annul rather than a number.
    //
    //     The port shows the bracket as temp Stop/Start callbacks on the upper
    //     effects rather than as one call, so the trace is normalised to the
    //     oracle's markers: the fixture logs a temp Stop/Start only from the
    //     middle effect, which is the one above the absorber in every bracketed
    //     case here.
    {
        let digit_of = |call: &str| -> Option<i64> {
            Some(match call {
                "EffectA" => 1,
                "EffectB" => 2,
                "EffectC" => 3,
                "Add" => 4,
                "TempRemoveUpper" => 5,
                "TempReaddUpper" => 6,
                "Kill" => 7,
                _ => return None,
            })
        };

        for (idx, case) in golden["effect_check"]
            .as_array()
            .unwrap()
            .iter()
            .enumerate()
        {
            let name = case["case"].as_str().unwrap_or_default();
            let priority = i(case, "priority") as i32;

            // What each existing effect's checker answers, and what the
            // absorbing effect's Add returns, recovered from the case name the
            // oracle emitted.
            let (results, add_result, dead, functionless) = match name {
                "priority_one_asks_nobody" => ([-1, -1, -1], 0, [false; 3], false),
                "all_accept" => ([0, 0, 0], 0, [false; 3], false),
                "first_denies" => ([-1, 0, 0], 0, [false; 3], false),
                "second_denies" => ([0, -1, 0], 0, [false; 3], false),
                "low_priority_denier_ignored" => ([-1, -1, -1], 0, [false; 3], false),
                "dead_effect_skipped" => ([-1, 0, 0], 0, [true, false, false], false),
                "functionless_effect_skipped" => ([-1, 0, 0], 0, [false; 3], true),
                "annul_absorbs" => ([-2, 0, 0], 0, [false; 3], false),
                "last_annul_wins" => ([-2, 0, -2], 0, [false; 3], false),
                "deny_after_annul" => ([-2, -1, 0], 0, [false; 3], false),
                "annul_calls_brackets_add" => ([-3, 0, 0], 0, [false; 3], false),
                "annul_calls_on_last_effect" => ([0, 0, -3], 0, [false; 3], false),
                "add_denies_kills_absorber" => ([-2, 0, 0], -1, [false; 3], false),
                "annul_calls_add_denies" => ([-3, 0, 0], -1, [false; 3], false),
                other => panic!("unhandled effect_check case `{other}`"),
            };

            // Only the middle effect reports its temp bracket, matching the
            // oracle's single TempRemoveUpper/TempReaddUpper markers.
            let mut script = String::from("#strict 2\nstatic fx_log, fx_armed;\n");
            for (index, id) in ["A", "B", "C"].into_iter().enumerate() {
                let digit = index + 1;
                let checker = if functionless && index == 0 {
                    String::new()
                } else {
                    format!(
                        "func FxEffect{id}Effect(string name, object target, int number) {{ if (!fx_armed) return 0; fx_log = fx_log * 11 + {digit}; return {}; }}\n",
                        results[index]
                    )
                };
                script.push_str(&checker);
                script.push_str(&format!(
                    "func FxEffect{id}Add(object target, int number, string name, int interval) {{ fx_log = fx_log * 11 + 4; return {add_result}; }}\n"
                ));
                if index == 1 {
                    script.push_str(&format!(
                        "func FxEffect{id}Stop(object target, int number, int reason, bool temp) {{ if (temp) fx_log = fx_log * 11 + 5; return 0; }}\n"
                    ));
                    script.push_str(&format!(
                        "func FxEffect{id}Start(object target, int number, int temp) {{ if (temp) fx_log = fx_log * 11 + 6; return 0; }}\n"
                    ));
                } else {
                    // The absorber's own non-temp Stop is how a Kill shows.
                    script.push_str(&format!(
                        "func FxEffect{id}Stop(object target, int number, int reason, bool temp) {{ if (!temp) fx_log = fx_log * 11 + 7; return 0; }}\n"
                    ));
                }
            }
            script.push_str(
                "func Arm() { AddEffect(\"EffectA\", this(), 100, 0, this()); AddEffect(\"EffectB\", this(), 60, 0, this()); AddEffect(\"EffectC\", this(), 20, 0, this()); fx_log = 0; fx_armed = 1; return 1; }\n",
            );
            script.push_str(&format!(
                "func Run() {{ return CheckEffect(\"Newcomer\", this(), {priority}, 35); }}\n"
            ));
            script.push_str("func ReadLog() { return fx_log; }\n");

            let mut engine = Engine::with_seed(0);
            engine
                .register_definition(
                    Definition::from_script("EFCK", "Effect check", &script)
                        .expect("effect check fixture compiles"),
                )
                .expect("effect check fixture registers");
            let object = engine
                .spawn_object(SpawnConfig::new("EFCK"))
                .expect("effect check object spawns");
            let index = engine.find_object_index(object).expect("object exists");
            engine
                .call_object_function(index, "Arm", Vec::new())
                .expect("the three effects are added");
            // A dead effect is one whose priority is zero (C4Effects.h:110),
            // in both engines.
            if dead[0] {
                if let Some(effect) = engine.objects[index]
                    .state
                    .effects
                    .iter_mut()
                    .find(|effect| effect.name == "EffectA")
                {
                    effect.priority = 0;
                }
            }

            let result = engine
                .call_object_function(index, "Run", Vec::new())
                .expect("CheckEffect runs");
            let result = match result {
                ScriptValue::Int(value) => i64::from(value),
                ScriptValue::Nil | ScriptValue::Bool(false) => 0,
                other => panic!("unexpected CheckEffect result {other:?}"),
            };
            expect_eq("effect_check", idx, "result", i(case, "result"), result);

            let expected_log = case["trace"]
                .as_array()
                .unwrap()
                .iter()
                .filter_map(|call| digit_of(call.as_str().unwrap_or_default()))
                .fold(0_i64, |log, digit| log * 11 + digit);
            let log = match engine
                .call_object_function(index, "ReadLog", Vec::new())
                .expect("the log reads back")
            {
                ScriptValue::Int(value) => i64::from(value),
                ScriptValue::Nil | ScriptValue::Bool(false) => 0,
                other => panic!("unexpected effect log value {other:?}"),
            };
            expect_eq("effect_check", idx, "callback order", expected_log, log);
        }
    }

    // 0l. C4Effect::Execute (C4Effect.cpp:319-363), the per-frame effect pass.
    //     It walks the list unlinking dead effects as it goes, advances each
    //     survivor's clock FIRST, and only then tests `iTime % iIntervall` — so
    //     an effect created this frame with interval 1 fires immediately, and
    //     one with a non-zero starting time lands on different frames. An
    //     interval with no timer function at all is killed the moment the
    //     boundary arrives (:355-357), and a timer answering
    //     `C4Fx_Execute_Kill` finishes its effect, which the NEXT pass unlinks.
    for (idx, case) in golden["effect_execute"]
        .as_array()
        .unwrap()
        .iter()
        .enumerate()
    {
        let name = case["case"].as_str().unwrap_or_default();
        // (priority, interval, has_timer, timer_result, start_time) per effect,
        // recovered from the case the oracle emitted.
        let rows: [(i32, i32, bool, i32, i32); 3] = match name {
            "interval_zero_never_fires" => [
                (100, 0, true, 0, 0),
                (60, 0, true, 0, 0),
                (20, 0, true, 0, 0),
            ],
            "interval_two_fires_every_other" => [
                (100, 2, true, 0, 0),
                (60, 0, true, 0, 0),
                (20, 0, true, 0, 0),
            ],
            "interval_one_fires_every_frame" => [
                (100, 1, true, 0, 0),
                (60, 0, true, 0, 0),
                (20, 0, true, 0, 0),
            ],
            "start_time_shifts_boundary" => [
                (100, 3, true, 0, 1),
                (60, 0, true, 0, 0),
                (20, 0, true, 0, 0),
            ],
            "timer_kills_then_unlinks" => [
                (100, 1, true, -1, 0),
                (60, 0, true, 0, 0),
                (20, 0, true, 0, 0),
            ],
            "interval_without_timer_dies" => [
                (100, 2, false, 0, 0),
                (60, 0, true, 0, 0),
                (20, 0, true, 0, 0),
            ],
            "dead_head_unlinked" => [
                (100, 0, true, 0, 0),
                (60, 0, true, 0, 0),
                (0, 0, true, 0, 0),
            ],
            "dead_middle_unlinked" => [
                (100, 0, true, 0, 0),
                (0, 0, true, 0, 0),
                (20, 0, true, 0, 0),
            ],
            "dead_tail_unlinked" => [(0, 0, true, 0, 0), (60, 0, true, 0, 0), (20, 0, true, 0, 0)],
            "all_dead_unlinked" => [(0, 0, true, 0, 0), (0, 0, true, 0, 0), (0, 0, true, 0, 0)],
            other => panic!("unhandled effect_execute case `{other}`"),
        };

        let mut script = String::from("#strict 2\nstatic fx_log;\n");
        for (index, id) in ["A", "B", "C"].into_iter().enumerate() {
            let (_, _, has_timer, timer_result, _) = rows[index];
            let digit = index + 1;
            if has_timer {
                script.push_str(&format!(
                    "func FxEffect{id}Timer(object target, int number, int time) {{ fx_log = fx_log * 11 + {digit}; return {timer_result}; }}\n"
                ));
            }
            script.push_str(&format!(
                "func FxEffect{id}Start(object target, int number, int temp) {{ return 0; }}\n"
            ));
        }
        script.push_str("func Arm() {\n");
        for (index, id) in ["A", "B", "C"].into_iter().enumerate() {
            let (priority, interval, ..) = rows[index];
            // A zero priority would be refused outright, so every effect is
            // added alive and the dead ones are zeroed afterwards.
            let add_priority = if priority == 0 {
                10 * (index as i32 + 1)
            } else {
                priority
            };
            script.push_str(&format!(
                "  AddEffect(\"Effect{id}\", this(), {add_priority}, {interval}, this());\n"
            ));
        }
        script.push_str("  fx_log = 0; return 1;\n}\n");
        script.push_str("func ReadLog() { return fx_log; }\n");
        script.push_str("func ResetLog() { fx_log = 0; return 1; }\n");

        let mut engine = Engine::with_seed(0);
        engine
            .register_definition(
                Definition::from_script("EFEX", "Effect execute", &script)
                    .expect("effect execute fixture compiles"),
            )
            .expect("effect execute fixture registers");
        let object = engine
            .spawn_object(SpawnConfig::new("EFEX"))
            .expect("effect execute object spawns");
        let index = engine.find_object_index(object).expect("object exists");
        engine
            .call_object_function(index, "Arm", Vec::new())
            .expect("the three effects are added");
        for (row, id) in rows.iter().zip(["A", "B", "C"]) {
            let (priority, _, _, _, start_time) = *row;
            let effect_name = format!("Effect{id}");
            if let Some(effect) = engine.objects[index]
                .state
                .effects
                .iter_mut()
                .find(|effect| effect.name == effect_name)
            {
                if priority == 0 {
                    effect.priority = 0;
                }
                effect.timer = start_time;
            }
        }

        for pass in case["passes"].as_array().unwrap() {
            engine
                .call_object_function(index, "ResetLog", Vec::new())
                .expect("the log resets");
            engine.tick().expect("the effect frame runs");

            let expected_log = pass["calls"]
                .as_array()
                .unwrap()
                .iter()
                .filter_map(|call| match call.as_str().unwrap_or_default() {
                    "EffectA" => Some(1_i64),
                    "EffectB" => Some(2),
                    "EffectC" => Some(3),
                    // The oracle records the Kill the pass performed; the port
                    // shows it as the effect being gone on the next pass, which
                    // the live list below compares.
                    _ => None,
                })
                .fold(0_i64, |log, digit| log * 11 + digit);
            let log = match engine
                .call_object_function(index, "ReadLog", Vec::new())
                .expect("the log reads back")
            {
                ScriptValue::Int(value) => i64::from(value),
                ScriptValue::Nil | ScriptValue::Bool(false) => 0,
                other => panic!("unexpected effect log value {other:?}"),
            };
            let frame = i(pass, "frame");
            expect_eq("effect_execute", idx, "timer calls", expected_log, log);

            let expected_live = pass["live"]
                .as_array()
                .unwrap()
                .iter()
                .map(|value| value.as_str().unwrap_or_default().to_owned())
                .collect::<Vec<_>>();
            let live = engine.objects[index]
                .state
                .effects
                .iter()
                .map(|effect| effect.name.clone())
                .collect::<Vec<_>>();
            assert_eq!(
                expected_live, live,
                "PARITY DIVERGENCE in `effect_execute` entry {idx} frame {frame} live effects"
            );
        }
    }

    // 0l-b. Complete the remaining C4Effect callback lifecycle against the
    //     mechanically lifted constructor, Kill, ClearAll, DoDamage and
    //     ClearPointers bodies (C4Effect.cpp:31-152,271-316,365-469). Each
    //     callback records its exact receiver and parameter vector, performs
    //     one synchronized Random(17) draw and mutates shared state. The
    //     linked list projection then catches number reservation, priority /
    //     timer state, live callback mutation and command-target loss in the
    //     same row as callback ordering and RNG state.
    for (idx, case) in golden["effect_lifecycle"]
        .as_array()
        .expect("effect_lifecycle golden section is an array")
        .iter()
        .enumerate()
    {
        let name = case["case"]
            .as_str()
            .expect("effect_lifecycle case has a name");
        let rust = run_effect_lifecycle_case(name, i(case, "seed") as u32);
        expect_json_eq("effect_lifecycle", idx, "row", case.clone(), rust);
    }

    // 0m. C4Object::AssignRemoval (C4Object.cpp:240-320), the object teardown.
    //     The order is what this pins:
    //
    //       * the CONTAINER's ContentsDestruction runs before the object's own
    //         Destruction, and each is followed by a `Status` re-check because
    //         the callback may already have removed the object — a callback
    //         that does so stops everything after it;
    //       * the object's contents are torn down BEFORE it leaves its own
    //         container, so a dying object's cargo still sees it as their
    //         container; and
    //       * `fExitContents` decides whether that cargo is Exited (spilled) or
    //         removed recursively, each one running its own full teardown.
    //
    //     The oracle also records bookkeeping the port does not expose
    //     (SetOCF, UpdateMass, SetActionIdle, ClearPointers, particles, the
    //     info retire); those entries document where the mutations sit between
    //     the script calls, and the comparison here is over the script calls
    //     plus the end state both engines can name.
    for (idx, case) in golden["object_removal"]
        .as_array()
        .unwrap()
        .iter()
        .enumerate()
    {
        let name = case["case"].as_str().unwrap_or_default();
        let contents = i(case, "own_contents");
        let _ = contents;

        // Which fixture shape this case needs.
        let contained = matches!(
            name,
            "contained" | "contents_destruction_deletes" | "contained_with_contents"
        );
        let cargo = match name {
            "already_deleted"
            | "contents_removed_recursively"
            | "contents_exited"
            | "contained_with_contents" => 2,
            "contents_destruction_deletes" | "destruction_deletes" => 1,
            _ => 0,
        };
        let exit_contents = name == "contents_exited";
        let destruction_body = if name == "destruction_deletes" {
            "if (!rm_fired) { rm_fired = 1; RemoveObject(); }"
        } else {
            ""
        };
        let contents_destruction_body = if name == "contents_destruction_deletes" {
            "if (!rm_fired) { rm_fired = 1; RemoveObject(pObj); }"
        } else {
            ""
        };

        let object_script = format!(
            "#strict 2\n\
             static rm_log, rm_fired;\n\
             protected func Destruction() {{ rm_log = rm_log * 11 + 2; {destruction_body} }}\n\
             protected func ContentsDestruction(pObj) {{ rm_log = rm_log * 11 + 1; }}\n\
             public func ReadLog() {{ return rm_log; }}\n"
        );
        let container_script = format!(
            "#strict 2\n\
             static rm_log, rm_fired;\n\
             protected func ContentsDestruction(pObj) {{ rm_log = rm_log * 11 + 1; {contents_destruction_body} }}\n\
             protected func Destruction() {{ rm_log = rm_log * 11 + 2; }}\n\
             public func ReadLog() {{ return rm_log; }}\n\
             public func ResetLog() {{ rm_log = 0; return 1; }}\n"
        );
        // The cargo carries the same recorders, so a recursive teardown shows.
        let cargo_script = "#strict 2\n\
             static rm_log;\n\
             protected func Destruction() { rm_log = rm_log * 11 + 2; }\n\
             protected func ContentsDestruction(pObj) { rm_log = rm_log * 11 + 1; }\n";

        let mut engine = Engine::with_seed(0);
        for (id, script) in [
            ("RMOB", object_script.as_str()),
            ("RMCN", container_script.as_str()),
            ("RMCG", cargo_script),
        ] {
            engine
                .register_definition(
                    Definition::from_script(id, id, script).expect("removal fixture compiles"),
                )
                .expect("removal fixture registers");
        }

        let container = engine
            .spawn_object(SpawnConfig::new("RMCN"))
            .expect("removal container spawns");
        let object = engine
            .spawn_object(SpawnConfig::new("RMOB"))
            .expect("removal object spawns");
        let index = engine.find_object_index(object).expect("object exists");
        if contained {
            engine.objects[index].state.container = Some(container);
        }
        let mut cargo_ids = Vec::new();
        for _ in 0..cargo {
            let id = engine
                .spawn_object(SpawnConfig::new("RMCG").with_container(object))
                .expect("cargo spawns");
            cargo_ids.push(id);
        }
        if name == "inactive_reactivated_first" {
            engine.objects[index].state.status = ObjectStatus::Inactive;
        }
        // The oracle's `already_deleted` row is an object whose status is
        // already zero while its cargo is still attached — a state a real first
        // removal cannot leave behind, so it is set directly.
        if name == "already_deleted" {
            engine.objects[index].state.status = ObjectStatus::Deleted;
        }

        let container_index = engine
            .find_object_index(container)
            .expect("container exists");
        engine
            .call_object_function(container_index, "ResetLog", Vec::new())
            .expect("the log resets");

        // `already_deleted` is the second removal of an object already gone.
        let runner_script = format!(
            "#strict 2\npublic func Run(object pTarget) {{ RemoveObject(pTarget, {}); return 1; }}\n",
            i32::from(exit_contents)
        );
        engine
            .register_definition(
                Definition::from_script("RMRN", "RMRN", &runner_script)
                    .expect("removal runner compiles"),
            )
            .expect("removal runner registers");
        let runner = engine
            .spawn_object(SpawnConfig::new("RMRN"))
            .expect("removal runner spawns");
        let runner_index = engine.find_object_index(runner).expect("runner exists");
        let target = crate::compat::object_reference_value(object);
        let _ = &target;
        engine
            .call_object_function(runner_index, "Run", vec![target])
            .expect("the removal runs");

        let expected_log = case["calls"]
            .as_array()
            .unwrap()
            .iter()
            .filter_map(|call| match call.as_str().unwrap_or_default() {
                "~ContentsDestruction" => Some(1_i64),
                "~Destruction" => Some(2),
                _ => None,
            })
            .fold(0_i64, |log, digit| log * 11 + digit);
        let log = match engine
            .call_object_function(container_index, "ReadLog", Vec::new())
            .expect("the log reads back")
        {
            ScriptValue::Int(value) => i64::from(value),
            ScriptValue::Nil | ScriptValue::Bool(false) => 0,
            other => panic!("unexpected removal log value {other:?}"),
        };
        expect_eq("object_removal", idx, "callback order", expected_log, log);

        // The cargo's fate: removed with the object, or spilled into the world.
        let surviving_cargo = cargo_ids
            .iter()
            .filter(|id| {
                engine
                    .find_object_index(**id)
                    .is_some_and(|index| engine.objects[index].state.status.is_active())
            })
            .count();
        let expected_cargo = case["content_status"]
            .as_array()
            .unwrap()
            .iter()
            .take(cargo as usize)
            .filter(|status| status.as_i64() != Some(0))
            .count();
        assert_eq!(
            expected_cargo, surviving_cargo,
            "PARITY DIVERGENCE in `object_removal` entry {idx} surviving cargo"
        );
    }

    // 0n. C4Object::AssignDeath (C4Object.cpp:1164-1205). Two orderings carry
    //     it, and both are the kind a port gets subtly wrong:
    //
    //       * the death-causing player is read BEFORE the effect clear —
    //         because those callbacks can meddle with the flags — and handed to
    //         the Death callback at the very END, so what the script sees is
    //         the cause as it stood when the object started dying; and
    //       * `Alive` is cleared BEFORE that clear, so a dying object cannot
    //         recurse into its own death.
    //
    //     An effect clear that puts the object back on its feet ABORTS the
    //     kill — the object stays alive, keeps its selection, and never reaches
    //     the Death callback — unless the kill was forced.
    for (idx, case) in golden["object_death"]
        .as_array()
        .unwrap()
        .iter()
        .enumerate()
    {
        let name = case["case"].as_str().unwrap_or_default();
        let forced = i(case, "forced") != 0;
        let alive = name != "already_dead";
        let resurrects = name.starts_with("resurrected");
        let cargo = if name == "contents_exited" || name == "already_dead" {
            2
        } else {
            0
        };

        // An effect whose Stop callback revives the object is how a script
        // reaches C4Object::AssignDeath's resurrection abort.
        let revive_body = if resurrects {
            "if (!dth_fired) { dth_fired = 1; SetAlive(1, pTarget); }"
        } else {
            ""
        };
        let object_script = format!(
            "#strict 2\n\
             static dth_log, dth_player, dth_fired;\n\
             protected func Death(int iCausedBy) {{ dth_log = dth_log * 11 + 1; dth_player = iCausedBy; }}\n\
             func FxReviveStop(object pTarget, int number, int reason, bool temp) {{ if (!temp) {{ {revive_body} }} return 0; }}\n\
             func FxReviveStart(object pTarget, int number, int temp) {{ return 0; }}\n\
             public func Arm() {{ AddEffect(\"Revive\", this(), 100, 0, this()); dth_log = 0; dth_player = -1; return 1; }}\n\
             public func ReadLog() {{ return dth_log; }}\n\
             public func ReadPlayer() {{ return dth_player; }}\n"
        );

        let mut engine = Engine::with_seed(0);
        engine
            .register_definition(
                Definition::from_script("DTOB", "DTOB", &object_script)
                    .expect("death fixture compiles"),
            )
            .expect("death fixture registers");
        engine
            .register_definition(
                Definition::from_script("DTCG", "DTCG", "#strict 2\n")
                    .expect("death cargo compiles"),
            )
            .expect("death cargo registers");
        engine
            .register_player(PlayerConfig::new(0, "death owner"))
            .expect("death owner registers");

        let object = engine
            .spawn_object(
                SpawnConfig::new("DTOB")
                    .with_owner(0)
                    .with_alive(alive)
                    .with_category(crate::CATEGORY_LIVING),
            )
            .expect("death object spawns");
        let index = engine.find_object_index(object).expect("object exists");
        // The cause the oracle configures, which the Death callback must carry.
        engine.objects[index].last_energy_loss_cause = 3;
        let mut cargo_ids = Vec::new();
        for _ in 0..cargo {
            cargo_ids.push(
                engine
                    .spawn_object(SpawnConfig::new("DTCG").with_container(object))
                    .expect("cargo spawns"),
            );
        }
        if resurrects {
            engine
                .call_object_function(index, "Arm", Vec::new())
                .expect("the reviving effect is added");
        }

        let killer_script = format!(
            "#strict 2\npublic func Run(object pTarget) {{ Kill(pTarget, {}); return 1; }}\n",
            i32::from(forced)
        );
        engine
            .register_definition(
                Definition::from_script("DTKL", "DTKL", &killer_script).expect("killer compiles"),
            )
            .expect("killer registers");
        let killer = engine
            .spawn_object(SpawnConfig::new("DTKL"))
            .expect("killer spawns");
        let killer_index = engine.find_object_index(killer).expect("killer exists");
        engine
            .call_object_function(
                killer_index,
                "Run",
                vec![crate::compat::object_reference_value(object)],
            )
            .expect("the kill runs");

        let expected_log = case["calls"]
            .as_array()
            .unwrap()
            .iter()
            .filter(|call| call.as_str() == Some("~Death"))
            .fold(0_i64, |log, _| log * 11 + 1);
        let index = engine.find_object_index(object).expect("object survives");
        let log = match engine
            .call_object_function(index, "ReadLog", Vec::new())
            .expect("the log reads back")
        {
            ScriptValue::Int(value) => i64::from(value),
            ScriptValue::Nil | ScriptValue::Bool(false) => 0,
            other => panic!("unexpected death log value {other:?}"),
        };
        expect_eq("object_death", idx, "death callback", expected_log, log);

        expect_eq(
            "object_death",
            idx,
            "alive_after",
            i(case, "alive_after"),
            i64::from(u8::from(engine.objects[index].state.alive)),
        );

        // The cause player the callback was handed, when it ran at all.
        if expected_log != 0 {
            let seen = match engine
                .call_object_function(index, "ReadPlayer", Vec::new())
                .expect("the cause reads back")
            {
                ScriptValue::Int(value) => i64::from(value),
                ScriptValue::Nil | ScriptValue::Bool(false) => 0,
                other => panic!("unexpected cause value {other:?}"),
            };
            expect_eq(
                "object_death",
                idx,
                "death_player_seen",
                i(case, "death_player_seen"),
                seen,
            );
        }

        // Contents are EXITED by a death, not removed — a dying Clonk drops
        // its load rather than taking it along.
        let still_contained = cargo_ids
            .iter()
            .filter(|id| {
                engine
                    .find_object_index(**id)
                    .and_then(|index| engine.objects[index].state.container)
                    == Some(object)
            })
            .count();
        let expected_contained = if i(case, "contents_contained") != 0 {
            cargo
        } else {
            0
        };
        assert_eq!(
            expected_contained, still_contained,
            "PARITY DIVERGENCE in `object_death` entry {idx} contents still contained"
        );
    }

    // 0o. C4Object::ChangeDef (C4Object.cpp:1207-1255), compiled beside the
    //     real Enter/Exit so its container round-trip runs the production
    //     bodies. The headline is what that round-trip does NOT do: the object
    //     leaves and re-enters with `fCalls = false`, so a definition change
    //     inside a container fires neither Ejection/Departure on the way out
    //     nor Collection2/Entrance on the way back — a script watching its
    //     contents sees nothing. `RejectEntrance` is the exception, because
    //     Enter asks it before `fCalls` is ever consulted.
    //
    //     Two smaller facts ride along: that Exit is passed `0, 0, 0`, so a
    //     contained object loses its rotation as a side effect of changing
    //     definition; and a non-rotateable target zeroes `r`, `fix_r` and
    //     `rdir` outright.
    for (idx, case) in golden["object_change_def"]
        .as_array()
        .unwrap()
        .iter()
        .enumerate()
    {
        let name = case["case"].as_str().unwrap_or_default();
        let contained = name == "contained_round_trip";
        let rotateable = name != "non_rotateable_drops_rotation";
        let start_rotation = i(case, "rotation");
        let start_rotation = if name == "unknown_definition" || contained || !rotateable {
            90
        } else {
            start_rotation
        };

        let container_script = "#strict 2\n\
             static cd_log;\n\
             protected func Collection2(pObj) { cd_log = cd_log * 11 + 3; }\n\
             protected func Ejection(pObj) { cd_log = cd_log * 11 + 6; }\n\
             public func ReadLog() { return cd_log; }\n\
             public func ResetLog() { cd_log = 0; return 1; }\n";
        // RejectEntrance is asked on the ENTERING object, not the container
        // (C4Object.cpp:1578) — and because the re-entry happens after the
        // definition has already changed, it resolves on the NEW definition's
        // script.
        let object_script = "#strict 2\n\
             static cd_log;\n\
             protected func RejectEntrance(pTarget) { cd_log = cd_log * 11 + 1; return 0; }\n\
             protected func Entrance(pContainer) { cd_log = cd_log * 11 + 4; }\n\
             protected func Departure(pContainer) { cd_log = cd_log * 11 + 7; }\n";

        let mut engine = Engine::with_seed(0);
        let mut target_definition = Definition::from_script("CDNW", "CDNW", object_script)
            .expect("new definition compiles");
        target_definition.set_rotateable(i32::from(rotateable));
        engine
            .register_definition(target_definition)
            .expect("new definition registers");
        engine
            .register_definition(
                Definition::from_script("CDOB", "CDOB", object_script)
                    .expect("old definition compiles"),
            )
            .expect("old definition registers");
        engine
            .register_definition(
                Definition::from_script("CDCN", "CDCN", container_script)
                    .expect("container compiles"),
            )
            .expect("container registers");

        let container = engine
            .spawn_object(SpawnConfig::new("CDCN"))
            .expect("container spawns");
        let object = engine
            .spawn_object(SpawnConfig::new("CDOB"))
            .expect("object spawns");
        let index = engine.find_object_index(object).expect("object exists");
        engine.objects[index].state.rotation = start_rotation as i32;
        engine.objects[index].rotation_velocity = itofix(1);
        if contained {
            engine.objects[index].state.container = Some(container);
        }

        let container_index = engine
            .find_object_index(container)
            .expect("container exists");
        engine
            .call_object_function(container_index, "ResetLog", Vec::new())
            .expect("the log resets");

        let runner_script = format!(
            "#strict 2\npublic func Run(object pTarget) {{ return ChangeDef({}, pTarget); }}\n",
            if name == "unknown_definition" {
                "ZZZZ"
            } else {
                "CDNW"
            }
        );
        engine
            .register_definition(
                Definition::from_script("CDRN", "CDRN", &runner_script).expect("runner compiles"),
            )
            .expect("runner registers");
        let runner = engine
            .spawn_object(SpawnConfig::new("CDRN"))
            .expect("runner spawns");
        let runner_index = engine.find_object_index(runner).expect("runner exists");
        let changed = engine
            .call_object_function(
                runner_index,
                "Run",
                vec![crate::compat::object_reference_value(object)],
            )
            .expect("the change runs");
        expect_eq(
            "object_change_def",
            idx,
            "changed",
            i(case, "changed"),
            i64::from(
                matches!(changed, ScriptValue::Bool(true))
                    || matches!(changed, ScriptValue::Int(value) if value != 0),
            ),
        );

        let index = engine.find_object_index(object).expect("object survives");
        let expected_id = if i(case, "changed") != 0 {
            "CDNW"
        } else {
            "CDOB"
        };
        assert_eq!(
            expected_id, engine.objects[index].definition_id,
            "PARITY DIVERGENCE in `object_change_def` entry {idx} definition"
        );
        expect_eq(
            "object_change_def",
            idx,
            "rotation",
            i(case, "rotation"),
            i64::from(engine.objects[index].state.rotation),
        );
        expect_eq(
            "object_change_def",
            idx,
            "rdir",
            i(case, "rdir"),
            i64::from(engine.objects[index].rotation_velocity.val()),
        );

        let expected_log = case["calls"]
            .as_array()
            .unwrap()
            .iter()
            .filter_map(|call| match call.as_str().unwrap_or_default() {
                "~RejectEntrance" => Some(1_i64),
                "~Collection2" => Some(3),
                "~Entrance" => Some(4),
                "~Ejection" => Some(6),
                "~Departure" => Some(7),
                _ => None,
            })
            .fold(0_i64, |log, digit| log * 11 + digit);
        let log = match engine
            .call_object_function(container_index, "ReadLog", Vec::new())
            .expect("the log reads back")
        {
            ScriptValue::Int(value) => i64::from(value),
            ScriptValue::Nil | ScriptValue::Bool(false) => 0,
            other => panic!("unexpected change-def log value {other:?}"),
        };
        expect_eq(
            "object_change_def",
            idx,
            "callback order",
            expected_log,
            log,
        );
    }

    // 0p. C4MouseControl::UpdateCursorTarget's OCF priority cascade
    //     (C4MouseControl.cpp:481-521). Every rule is an UNCONDITIONAL
    //     overwrite, so the LAST match wins rather than the first: a candidate
    //     that is at once carryable, choppable and alive walks the whole ladder
    //     and ends on the rule furthest down it. Adding an OCF bit can only
    //     move the cursor later in that order, never earlier.
    //
    //     The `ocf` the cascade tests is NOT the search mask it started from:
    //     `GetTargetObject` takes it by reference and `GetOCFForPos` overwrites
    //     it with the target's position-filtered OCF (`:1318-1326`), which is
    //     what the port computes as `object_ocf_for_pos`. The first Enter rule
    //     is the one place that reads the object's CACHED OCF instead, so
    //     containers stay enterable across their whole shape.
    {
        for (idx, case) in golden["mouse_cursor_cascade"]
            .as_array()
            .unwrap()
            .iter()
            .enumerate()
        {
            let name = case["case"].as_str().unwrap_or_default();
            let filtered_ocf = i(case, "ocf") as u32;
            let cached_entrance = i(case, "target_ocf") as u32 & crate::ocf::ENTRANCE != 0;
            let owner = i(case, "owner") as i32;
            let player = i(case, "player") as i32;
            let dx = i(case, "dx") as i32;

            let mut engine = Engine::with_seed(0);
            let mut definition = Definition::from_script("MCUR", "MCUR", "#strict 2\n")
                .expect("cursor fixture compiles");
            definition.set_category(i(case, "category") as i32);
            // A twenty-wide shape, which is what the chop rule's thirds are
            // measured against.
            definition.set_shape_rect(Some(crate::DefinitionRect::new(-10, -10, 20, 20)));
            // `object_ocf_for_pos` position-filters the Entrance bit, so
            // whether the definition has an entrance area is what decides if
            // that bit survives into the mask the cascade tests. The oracle
            // states the filtered mask directly; this is how the port is made
            // to produce it — an entrance area covering the pointer when the
            // filtered mask kept the bit, and none when it did not.
            if filtered_ocf & crate::ocf::ENTRANCE != 0 {
                definition.set_entrance_rect(Some(crate::DefinitionRect::new(-10, -10, 20, 20)));
            }
            engine
                .register_definition(definition)
                .expect("cursor fixture registers");
            // The Ungrab rule asks the crew cursor's PROCEDURE, which resolves
            // through the definition's action map — an action merely named
            // "Push" is not enough.
            let mut crew_definition = Definition::from_script("MCLK", "MCLK", "#strict 2\n")
                .expect("crew fixture compiles");
            crew_definition.configure_actions(
                Some("Push".to_owned()),
                HashMap::from([(
                    "Push".to_owned(),
                    crate::ActionSpec::default().with_procedure("PUSH"),
                )]),
            );
            engine
                .register_definition(crew_definition)
                .expect("crew fixture registers");
            let target = engine
                .spawn_object(
                    SpawnConfig::new("MCUR")
                        .with_owner(owner)
                        .with_alive(i(case, "alive") != 0),
                )
                .expect("target spawns");
            let index = engine.find_object_index(target).expect("target exists");
            // Set the position rather than spawning at it: a spawn y is the
            // object's BOTTOM, and this shape has an offset, so passing it
            // through SpawnConfig would put the object ten pixels off and move
            // the chop rule's range with it.
            engine.objects[index].state.position = crate::Vector2::new(100, 100);
            engine.objects[index].state.category = i(case, "category") as i32;
            // `object_ocf_for_pos` returns the cached OCF untouched unless
            // Entrance or Collection is set, so the cached and filtered masks
            // are the same everywhere except the container cases — where the
            // definition has no entrance rect, so the position filter clears
            // that bit exactly as the oracle's two columns describe.
            engine.objects[index].state.ocf = if cached_entrance {
                filtered_ocf | crate::ocf::ENTRANCE
            } else {
                filtered_ocf
            };

            // Hostility needs both players registered, which in turn makes
            // `player_crew_roster` read the registered (empty) crew instead of
            // falling back to the owned-crew scan. The hostile cases do not
            // need the crew rule — their Select comes from the MouseSelect
            // category — so the two setups are kept apart.
            let hostile = i(case, "hostile") != 0;
            if hostile {
                for id in [owner, player] {
                    if id >= 0 {
                        engine
                            .register_player(PlayerConfig::new(id, "cursor player"))
                            .expect("player registers");
                    }
                }
                if let Some(first) = engine.players.get_mut(&player.max(0)) {
                    first.set_hostile_towards(owner, true);
                }
            }

            // The player's own cursor: pushing this target turns Grab into
            // Ungrab, and being a crew member turns the Alive rule into Select.
            if !hostile && (i(case, "pushing") != 0 || i(case, "in_crew") != 0) {
                let crew = if i(case, "in_crew") != 0 {
                    target
                } else {
                    engine
                        .spawn_object(SpawnConfig::new("MCLK").with_owner(player.max(0)))
                        .expect("crew spawns")
                };
                // The cursor has to be an owned crew member of that player.
                let crew_index = engine.find_object_index(crew).expect("crew exists");
                engine.objects[crew_index].state.owner = player.max(0);
                engine.objects[crew_index].state.crew_member = true;
                engine
                    .set_crew_cursor(player.max(0), Some(crew))
                    .expect("the crew cursor is set");
                if i(case, "pushing") != 0 {
                    engine.objects[crew_index].state.action.name = "Push".to_owned();
                    engine.objects[crew_index].state.action.target = Some(target);
                }
            }

            let cursor = engine.mouse_world_cursor(
                player,
                Some(target),
                crate::Vector2::new(100 + dx, 100),
                false,
            );
            let actual = match cursor {
                crate::MouseWorldCursor::Crosshair => 0,
                crate::MouseWorldCursor::Enter(_) => 1,
                crate::MouseWorldCursor::Grab(_) => 2,
                crate::MouseWorldCursor::Ungrab(_) => 3,
                crate::MouseWorldCursor::Carryable(_) => 4,
                crate::MouseWorldCursor::DigObject(_) => 5,
                crate::MouseWorldCursor::Chop(_) => 6,
                crate::MouseWorldCursor::Build(_) => 7,
                crate::MouseWorldCursor::Select(_) => 8,
                crate::MouseWorldCursor::Attack(_) => 9,
                other => panic!("unexpected cursor {other:?} for `{name}`"),
            };
            expect_eq(
                "mouse_cursor_cascade",
                idx,
                "cursor",
                i(case, "cursor"),
                actual,
            );
        }
    }

    // 0q. C4GameSave's save-policy matrix: the base query functions
    //     (C4GameSave.h:59-72) and each specialization's overrides
    //     (:117-188). Every one is a pure function of Sync, fInitial and the
    //     constructor flags, and together they decide what a written save
    //     actually contains -- which components survive, whose player files
    //     are embedded, and whether the landscape is stored exactly.
    //
    //     Several entries invert in ways a port is likely to get backwards.
    //     `GetKeepTitle` is `!IsExact()`, so the SCENARIO save is the one that
    //     keeps the localized title, image and icon while a savegame deletes
    //     them. `GetSaveUserPlayerFiles` is `IsExact()` for every variant
    //     except the savegame, which overrides it to false because resuming
    //     players bring their own files. And C4GameSaveScenario overrides
    //     `GetSaveScriptPlayers`/`GetSaveScriptPlayerFiles` to a flat true
    //     while leaving the user-player pair at `IsExact()`, so a saved
    //     scenario keeps script players and drops user ones.
    //
    //     The port models the four non-initial variants. `record_initial`,
    //     `network_initial` (fInitial, which suppresses runtime data) and the
    //     streaming record (fCopyScenario = false) have no `LiveC4SavePolicy`
    //     counterpart, so their rows are skipped rather than approximated;
    //     the same goes for the origin pair, which the port applies through
    //     the scenario-core writers instead of a policy predicate.
    {
        use crate::live_c4_save::LiveC4SavePolicy;

        let mut compared = 0;
        for (idx, case) in golden["game_save_policy"]
            .as_array()
            .unwrap()
            .iter()
            .enumerate()
        {
            let name = case["case"].as_str().unwrap_or_default();
            let policy = match name {
                "scenario" => LiveC4SavePolicy::Scenario {
                    force_exact_landscape: false,
                },
                "scenario_exact_landscape_and_origin" => LiveC4SavePolicy::Scenario {
                    force_exact_landscape: true,
                },
                "savegame" => LiveC4SavePolicy::Savegame {
                    target_group_name: "Savegame.c4s",
                },
                "record_runtime" => LiveC4SavePolicy::Record,
                "network_runtime" => LiveC4SavePolicy::RuntimeNetwork,
                _ => continue,
            };
            compared += 1;

            let players = policy.player_policy();
            for (field, expected, actual) in [
                (
                    "keep_title",
                    i(case, "keep_title"),
                    policy.keeps_title_components(),
                ),
                (
                    "save_desc",
                    i(case, "save_desc"),
                    policy.saves_description(),
                ),
                (
                    "copy_scenario",
                    i(case, "copy_scenario"),
                    policy.copies_source_scenario(),
                ),
                (
                    "create_small_file",
                    i(case, "create_small_file"),
                    policy.creates_small_player_files(),
                ),
                (
                    "force_exact_landscape",
                    i(case, "force_exact_landscape"),
                    policy.forces_runtime_landscape(),
                ),
                (
                    "save_user_players",
                    i(case, "save_user_players"),
                    players.save_user_players,
                ),
                (
                    "save_script_players",
                    i(case, "save_script_players"),
                    players.save_script_players,
                ),
                (
                    "save_user_player_files",
                    i(case, "save_user_player_files"),
                    players.embed_user_player_files,
                ),
                (
                    "save_script_player_files",
                    i(case, "save_script_player_files"),
                    players.embed_script_player_files,
                ),
                ("is_exact", i(case, "is_exact"), policy.is_exact()),
                ("is_synced", i(case, "is_synced"), policy.is_synchronized()),
            ] {
                expect_eq("game_save_policy", idx, field, expected, i64::from(actual));
            }
        }
        assert_eq!(
            compared, 5,
            "every modelled save variant must be compared; the golden's case \
             names changed if this trips"
        );
    }

    // 0r. `C4GameSave::GetSortOrder` returns C4FLS_Scenario for every
    //     specialization (C4GameSave.h:63, no override), and `Close()` applies
    //     it to the finished group (C4GameSave.cpp:508-510). That single string
    //     IS the component order a saved scenario is written in, so a reader
    //     walking the group sees Scenario.txt before Game.txt before Objects.txt
    //     only because of it.
    for (idx, case) in golden["game_save_sort_order"]
        .as_array()
        .unwrap()
        .iter()
        .enumerate()
    {
        let expected = case["order"].as_str().unwrap();
        let actual =
            clonk_resources::group_writer::standard_sort_list_for_filename(b"Savegame.c4s")
                .expect("a .c4s group selects the stock scenario sort list");
        assert_eq!(
            expected, actual,
            "game_save_sort_order[{idx}]: group sort order diverges from C++"
        );
    }

    // 0s. WildcardMatch (StdFile.cpp:337-366), the matcher `C4Group::GetEntry`
    //     applies while walking stored entry order (C4Group.cpp:1221,:1230) and
    //     that every stock sort list is evaluated through.
    //
    //     It is case-insensitive, `?` matches exactly one character and never
    //     the end of the string, and `*` matches any run including the empty
    //     one -- with real backtracking, so a pattern like `a*b*c` may have to
    //     retry a star from several positions before it succeeds.
    for (idx, case) in golden["wildcard_match"]
        .as_array()
        .unwrap()
        .iter()
        .enumerate()
    {
        let pattern = case["pattern"].as_str().unwrap();
        let name = case["name"].as_str().unwrap();
        let actual =
            clonk_resources::group::group_name_wildcard_match(pattern.as_bytes(), name.as_bytes());
        expect_eq(
            "wildcard_match",
            idx,
            "match",
            i(case, "match"),
            i64::from(actual),
        );
    }

    // 0t. C4ConfigGeneral::GetLanguageSequence (C4Config.cpp:1492-1507), the
    //     condensing pass that derives `LanguageEx` from `Language`
    //     (`:1471-1473`) and appends a scenario's fallback list
    //     (C4StartupOptionsDlg.cpp:1219).
    //
    //     The condensing is not a validation pass, which is the part a rewrite
    //     tends to get wrong: a segment is TRUNCATED to its first two bytes
    //     rather than rejected, so `DE - Deutsch` becomes `DE` and `English`
    //     becomes `En`. Case is preserved, a one-character code stays one
    //     character, duplicates are kept, and only a segment that is empty
    //     after the leading-whitespace skip is dropped -- which is why
    //     `DE,,US` yields two entries and `,,,` yields none.
    for (idx, case) in golden["config_language_sequence"]
        .as_array()
        .unwrap()
        .iter()
        .enumerate()
    {
        let source = case["source"].as_str().unwrap();
        let sequence = clonk_core::std_config::language_sequence(source);
        expect_eq(
            "config_language_sequence",
            idx,
            "count",
            i(case, "count"),
            sequence.len() as i64,
        );
        assert_eq!(
            case["target"].as_str().unwrap(),
            sequence.join(","),
            "config_language_sequence[{idx}]: condensed sequence diverges from C++"
        );
    }

    // 0u. C4Value::operator== (C4Value.cpp:862-919) over the full ordered
    //     cross-type matrix.
    //
    //     The operator is a nested switch on the LEFT tag and then the right,
    //     so it is not obliged to be symmetric -- and generating every ordered
    //     pair shows it is asymmetric in exactly ONE place: the object arm
    //     demands an equal tag as well as an equal payload, which makes
    //     `nil == object_zero` true while `object_zero == nil` is false. That
    //     is worth pinning precisely because the differing arms look like they
    //     should produce more asymmetry than they do.
    //
    //     The other rules the matrix fixes: Any, Int and C4ID interconvert on
    //     the raw payload, and Bool joins them EXCEPT against a C4ID -- a Bool
    //     and a C4ID carrying the same word stay unequal in both directions.
    //     Strings and arrays compare by CONTENT, not by backing pointer, so two
    //     distinct allocations of `abc` are equal and the array arm recurses
    //     back through this operator element-wise (C4ValueList.h:49,:67).
    //
    //     C4IDs here carry only payloads the port can also build: an all-digit
    //     id of four or more characters parses numerically, which is what lets
    //     a Bool and a C4ID share a word on both sides. Maps are not covered.
    {
        use clonk_script::Value;

        fn named(name: &str) -> Value {
            match name {
                "nil" => Value::Nil,
                "int_zero" => Value::Int(0),
                "int_one" => Value::Int(1),
                "int_minus_one" => Value::Int(-1),
                "bool_false" => Value::Bool(false),
                "bool_true" => Value::Bool(true),
                "c4id_zero" => Value::C4Id("0000".to_string()),
                "c4id_one" => Value::C4Id("0001".to_string()),
                "object_zero" => Value::Object(0),
                "object_five" => Value::Object(5),
                // Two independent allocations of the same text, so a
                // pointer-identity comparison would report them unequal.
                "string_abc" | "string_abc_other_allocation" => Value::String("abc".into()),
                "string_xyz" => Value::String("xyz".into()),
                "array_one_two" | "array_one_two_other_allocation" => {
                    Value::Array(vec![Value::Int(1), Value::Int(2)])
                }
                "array_one_three" => Value::Array(vec![Value::Int(1), Value::Int(3)]),
                other => panic!("unknown c4value_operator_equal operand `{other}`"),
            }
        }

        for (idx, case) in golden["c4value_operator_equal"]
            .as_array()
            .unwrap()
            .iter()
            .enumerate()
        {
            let left = named(case["left"].as_str().unwrap());
            let right = named(case["right"].as_str().unwrap());
            expect_eq(
                "c4value_operator_equal",
                idx,
                "equal",
                i(case, "equal"),
                i64::from(left.c4_operator_equals(&right)),
            );
        }
    }

    // 0v. Stateful C4Value conversion/copy probes. Unlike
    //     `script_value_convert`, these rows observe the post-operation tag,
    //     payload, reference state, referent/destination and serialized form.
    //     They execute C4Value::Set/ConvertTo/Deref semantics rather than only
    //     comparing the conversion-table classification (C4Value.cpp:121-143,
    //     :445-478; C4Value.h:195,221-223).
    {
        let section = golden["c4value_stateful_conversion"]
            .as_array()
            .expect("c4value_stateful_conversion is a C++ oracle array");
        for (index, case) in section.iter().enumerate() {
            let name = case["case"]
                .as_str()
                .expect("c4value_stateful_conversion case has a name");
            let outcome = run_c4value_stateful_conversion(name);
            let target_type = outcome
                .target
                .as_ref()
                .map_or(-1, |value| value.c4v_type().index() as i64);
            let target_payload = outcome.target.as_ref().map_or(0, c4value_scalar_payload);
            let serialized =
                crate::live_c4_save::encode_value_with_current_string_ids(&outcome.value);

            for (field, actual) in [
                ("ok", i64::from(outcome.ok)),
                ("type", outcome.value.c4v_type().index() as i64),
                ("payload", c4value_scalar_payload(&outcome.value)),
                ("is_ref", i64::from(outcome.is_ref)),
                ("target_type", target_type),
                ("target_payload", target_payload),
                ("rng_delta", i64::from(outcome.rng_delta)),
            ] {
                expect_eq(
                    "c4value_stateful_conversion",
                    index,
                    field,
                    i(case, field),
                    actual,
                );
            }
            expect_json_eq(
                "c4value_stateful_conversion",
                index,
                "serialized",
                case["serialized"].clone(),
                serde_json::json!(serialized),
            );
        }
    }

    // 0w. Saved C4Value object pointers resolve through active and inactive
    //     lists. Explicit C4V_C4ObjectEnum misses clear to nil; legacy
    //     C4V_Any+offset misses retain their word and GuessType to int
    //     (C4Value.cpp:684-715; C4ObjectList.h:32-34).
    {
        let section = golden["c4value_denumeration"]
            .as_array()
            .expect("c4value_denumeration is a C++ oracle array");
        for (index, case) in section.iter().enumerate() {
            let encoded = case["encoded"]
                .as_str()
                .expect("c4value_denumeration case has an encoding");
            let outcome = run_c4value_denumeration(encoded);
            let serialized =
                crate::live_c4_save::encode_value_with_current_string_ids(&outcome.value);

            for (field, actual) in [
                ("type", outcome.value.c4v_type().index() as i64),
                ("payload", c4value_scalar_payload(&outcome.value)),
                ("rng_delta", i64::from(outcome.rng_delta)),
            ] {
                expect_eq("c4value_denumeration", index, field, i(case, field), actual);
            }
            expect_json_eq(
                "c4value_denumeration",
                index,
                "serialized",
                case["serialized"].clone(),
                serde_json::json!(serialized),
            );
        }
    }

    // 0x. Stateful array operations preserve old element references across
    //     growth, distinguish value reads from mutable indexing, clamp negative
    //     indices and report native index failures without mutating the array
    //     (C4Value.cpp:37-297; C4ValueList.cpp:28-90,143-183).
    {
        let section = golden["c4value_runtime_operations"]
            .as_array()
            .expect("c4value_runtime_operations is a C++ oracle array");
        for (index, case) in section.iter().enumerate() {
            let name = case["case"]
                .as_str()
                .expect("c4value_runtime_operations case has a name");
            let outcome = run_c4value_runtime_operation(name);
            let result = crate::live_c4_save::encode_value_with_current_string_ids(&outcome.result);
            let mutated = c4value_runtime_array_state(&outcome.array);
            let serialized =
                crate::live_c4_save::encode_value_with_current_string_ids(&outcome.array);

            for (field, actual) in [
                ("result", result),
                (
                    "type",
                    c4value_runtime_type_name(&outcome.result).to_owned(),
                ),
                ("mutated", mutated),
                ("error", outcome.error),
                ("serialized", serialized),
            ] {
                expect_json_eq(
                    "c4value_runtime_operations",
                    index,
                    field,
                    case[field].clone(),
                    serde_json::json!(actual),
                );
            }
            expect_eq(
                "c4value_runtime_operations",
                index,
                "aliases",
                i(case, "aliases"),
                outcome.aliases,
            );
            expect_eq(
                "c4value_runtime_operations",
                index,
                "rng_delta",
                i(case, "rng_delta"),
                i64::from(outcome.rng_delta),
            );
        }
    }

    // 0y. C4ValueHash hashes keys first, then compares only the matching
    //     bucket with C4Value::Equals(MAXSTRICT) (C4ValueHash.h:39-48;
    //     C4ValueHash.cpp:77-80,117-136). For C4V_Bool, both operations use
    //     `_getBool()`, so every nonzero raw payload is the same map key even
    //     though C4Value::operator== compares those raw payloads exactly
    //     (C4Value.cpp:823-852,862-919,965-988).
    {
        use clonk_script::{Value, ValueMap};

        fn named(name: &str) -> Value {
            match name {
                "bool_false" => Value::Bool(false),
                "bool_true" => Value::Bool(true),
                "bool_two" => Value::from_c4_bool_raw(2),
                "bool_seven" => Value::from_c4_bool_raw(7),
                "int_one" => Value::Int(1),
                other => panic!("unknown c4value_map_key_lookup operand `{other}`"),
            }
        }

        for (idx, case) in golden["c4value_map_key_lookup"]
            .as_array()
            .unwrap()
            .iter()
            .enumerate()
        {
            let inserted = named(case["inserted"].as_str().unwrap());
            let probe = named(case["probe"].as_str().unwrap());
            expect_eq(
                "c4value_map_key_lookup",
                idx,
                "inserted_hash",
                u(case, "inserted_hash") as i64,
                inserted.c4_value_hash() as i64,
            );
            expect_eq(
                "c4value_map_key_lookup",
                idx,
                "probe_hash",
                u(case, "probe_hash") as i64,
                probe.c4_value_hash() as i64,
            );
            expect_eq(
                "c4value_map_key_lookup",
                idx,
                "operator_equal",
                i(case, "operator_equal"),
                i64::from(inserted.c4_operator_equals(&probe)),
            );

            let mut map = ValueMap::new();
            map.insert_key(inserted, Value::Int(1));
            expect_eq(
                "c4value_map_key_lookup",
                idx,
                "found",
                i(case, "found"),
                i64::from(map.get_key(&probe).is_some()),
            );
        }
    }

    // 0u. C4Config::AdaptToCurrentVersion (C4Config.cpp:1631-1676), the config
    //     post-init migration C4Config::Load runs right after DeterminePaths
    //     (`:1110-1112`). It repairs a config written by an older build and
    //     then stamps it with C4XVERBUILD, so it is also the reason a config
    //     is migrated exactly once.
    //
    //     Three details are what a rewrite gets wrong, and each is pinned
    //     below: the `case 347` arm FALLS THROUGH into 346, so 347 gets both
    //     the channel reset and the music repair while 346 gets only the
    //     music; the `<= 359` block runs independently of the switch, so it
    //     also applies to a config with no Version at all; and the address
    //     rewrites are SEqual comparisons against the retired defaults, so a
    //     deliberately customized server survives untouched.
    //
    //     Version 349 is absent from the golden on purpose: its arm is
    //     `#ifdef __APPLE__`, so a recorded value would depend on the host
    //     that ran the generator. `std_config`'s own tests cover it.
    for (idx, case) in golden["config_adapt_version"]
        .as_array()
        .unwrap()
        .iter()
        .enumerate()
    {
        let mut config = clonk_core::std_config::Config::new();
        config.set_in(Some("General"), "Version", i(case, "version").to_string());
        config.set_in(Some("General"), "Preloading", "1");
        config.set_in(Some("Graphics"), "Shader", "0");
        config.set_in(Some("Graphics"), "DisableGamma", "1");
        config.set_in(Some("Sound"), "RXMusic", "0");
        config.set_in(Some("Sound"), "MaxChannels", "7");
        config.set_in(Some("Network"), "ServerAddress", "league.clonkspot.org:80");
        config.set_in(
            Some("Network"),
            "AlternateServerAddress",
            "league.clonkspot.org:80",
        );
        config.set_in(
            Some("Network"),
            "UpdateServerAddress",
            "update.clonkspot.org/lc/update",
        );
        config.set_in(Some("Network"), "PuncherAddress", "clonk.de:11115");

        clonk_core::std_config::adapt_to_current_version(&mut config);

        let flag = |section: &str, key: &str| -> i64 {
            i64::from(
                config
                    .get_in(Some(section), key)
                    .is_some_and(|value| value == "1"),
            )
        };
        let text = |section: &str, key: &str| -> String {
            config
                .get_in(Some(section), key)
                .unwrap_or_default()
                .to_string()
        };

        expect_eq(
            "config_adapt_version",
            idx,
            "out_version",
            i(case, "out_version"),
            config
                .get_in(Some("General"), "Version")
                .and_then(|value| value.parse::<i64>().ok())
                .unwrap_or(-1),
        );
        for (field, section, key) in [
            ("preloading", "General", "Preloading"),
            ("shader", "Graphics", "Shader"),
            ("disable_gamma", "Graphics", "DisableGamma"),
            ("rx_music", "Sound", "RXMusic"),
        ] {
            expect_eq(
                "config_adapt_version",
                idx,
                field,
                i(case, field),
                flag(section, key),
            );
        }
        expect_eq(
            "config_adapt_version",
            idx,
            "max_channels",
            i(case, "max_channels"),
            config
                .get_in(Some("Sound"), "MaxChannels")
                .and_then(|value| value.parse::<i64>().ok())
                .unwrap_or(-1),
        );
        for (field, key) in [
            ("server", "ServerAddress"),
            ("alternate_server", "AlternateServerAddress"),
            ("update_server", "UpdateServerAddress"),
            ("puncher", "PuncherAddress"),
        ] {
            assert_eq!(
                case[field].as_str().unwrap(),
                text("Network", key),
                "config_adapt_version[{idx}]: {field} diverges from C++"
            );
        }
    }

    // 1. itofix (whole-integer + precision-denominated).
    for (idx, e) in golden["itofix"].as_array().unwrap().iter().enumerate() {
        let (x, prec, raw) = (i(e, "x") as i32, i(e, "prec") as i32, i(e, "raw"));
        let rust = if prec == 1 {
            itofix(x).val()
        } else {
            itofix_prec(x, prec).val()
        };
        expect_eq("itofix", idx, "raw", raw, rust as i64);
    }

    // 2. fixtoi (rounding back to integer, whole + precision-multiplied).
    for (idx, e) in golden["fixtoi"].as_array().unwrap().iter().enumerate() {
        let (raw, prec, result) = (i(e, "raw") as i32, i(e, "prec") as i32, i(e, "result"));
        let f = C4Fixed::from_raw(raw);
        let rust = if prec == 1 {
            fixtoi(f)
        } else {
            fixtoi_prec(f, prec)
        };
        expect_eq("fixtoi", idx, "result", result, rust as i64);
    }

    // 3. arithmetic (+, -, *, /) and the FIXED100/256/10 helper constants.
    for (idx, e) in golden["arith"].as_array().unwrap().iter().enumerate() {
        if e.get("a").is_some() {
            let (a, b) = (i(e, "a") as i32, i(e, "b") as i32);
            let (fa, fb) = (itofix(a), itofix(b));
            expect_eq("arith", idx, "add", i(e, "add"), (fa + fb).val() as i64);
            expect_eq("arith", idx, "sub", i(e, "sub"), (fa - fb).val() as i64);
            expect_eq("arith", idx, "mul", i(e, "mul"), (fa * fb).val() as i64);
            expect_eq("arith", idx, "div", i(e, "div"), (fa / fb).val() as i64);
        } else {
            expect_eq(
                "arith",
                idx,
                "fixed100_10",
                i(e, "fixed100_10"),
                fixed100(10).val() as i64,
            );
            expect_eq(
                "arith",
                idx,
                "fixed256_10",
                i(e, "fixed256_10"),
                fixed256(10).val() as i64,
            );
            expect_eq(
                "arith",
                idx,
                "fixed10_10",
                i(e, "fixed10_10"),
                fixed10(10).val() as i64,
            );
        }
    }

    // 4. trig (Sin/Cos via the shared SineTable).
    for (idx, e) in golden["trig"].as_array().unwrap().iter().enumerate() {
        let deg = i(e, "deg") as i32;
        let angle = itofix(deg);
        expect_eq(
            "trig",
            idx,
            "sin",
            i(e, "sin"),
            angle.sin_deg().val() as i64,
        );
        expect_eq(
            "trig",
            idx,
            "cos",
            i(e, "cos"),
            angle.cos_deg().val() as i64,
        );
    }

    // 4b. Script FnSin/FnCos default radius: omitted integer parameters are
    // zero-filled and only precision is corrected to one (C4Script.cpp:
    // 3224-3238).
    for (idx, e) in golden["script_trig_default_radius"]
        .as_array()
        .unwrap()
        .iter()
        .enumerate()
    {
        let deg = i(e, "deg") as i32;
        let ScriptValue::Int(rust_sin) =
            sin_func(&[ScriptValue::Int(deg)]).expect("script Sin oracle input succeeds")
        else {
            panic!("script Sin did not return int")
        };
        let ScriptValue::Int(rust_cos) =
            cos_func(&[ScriptValue::Int(deg)]).expect("script Cos oracle input succeeds")
        else {
            panic!("script Cos did not return int")
        };
        expect_eq(
            "script_trig_default_radius",
            idx,
            "sin",
            i(e, "sin"),
            i64::from(rust_sin),
        );
        expect_eq(
            "script_trig_default_radius",
            idx,
            "cos",
            i(e, "cos"),
            i64::from(rust_cos),
        );
    }

    // 4c. Script FnSqrt: the two correction steps around the truncated double
    // root, whose `iSqrt * iSqrt` products are C4ValueInt and wrap above
    // 46340^2 (C4Script.cpp:3240-3247, C4Value.h:62).
    for (idx, e) in golden["script_sqrt"].as_array().unwrap().iter().enumerate() {
        let value = i(e, "value") as i32;
        let ScriptValue::Int(rust_root) =
            sqrt_func(&[ScriptValue::Int(value)]).expect("script Sqrt oracle input succeeds")
        else {
            panic!("script Sqrt did not return int")
        };
        expect_eq(
            "script_sqrt",
            idx,
            "root",
            i(e, "root"),
            i64::from(rust_root),
        );
    }

    // 5. RNG: the LCG sequence and RandomCount semantics (incl. range 0).
    {
        let rr = &golden["rng_random"];
        let seed = i(rr, "seed") as u32;
        let mut rng = LcgRng::new(seed);
        for (idx, e) in rr["sequence"].as_array().unwrap().iter().enumerate() {
            let range = i(e, "range") as i32;
            let val = i(e, "val");
            expect_eq("rng_random", idx, "val", val, rng.random(range) as i64);
        }
        expect_eq(
            "rng_random",
            0,
            "count_after",
            i(rr, "count_after"),
            rng.count as i64,
        );
        rng.random(0); // range 0: returns 0 but still increments count
        expect_eq(
            "rng_random",
            0,
            "count_after_zero",
            i(rr, "count_after_zero"),
            rng.count as i64,
        );
    }

    // 5b. Stateless SeededRandom, including zero range and u32 overflow.
    for (idx, entry) in golden["rng_seeded_random"]
        .as_array()
        .expect("rng_seeded_random is an array")
        .iter()
        .enumerate()
    {
        expect_eq_u64(
            "rng_seeded_random",
            idx,
            "val",
            u(entry, "val"),
            u64::from(LcgRng::seeded_random(
                u(entry, "seed") as u32,
                u(entry, "range") as u32,
            )),
        );
    }

    // 6. Randomize3 buffer values + the Rnd3 circular-buffer sequence.
    {
        let rr = &golden["rng_randomize3"];
        let seed = i(rr, "seed") as u32;
        // Buffer values are `random(3) - 1` ×500 (what randomize3 fills).
        let mut builder = LcgRng::new(seed);
        for (idx, b) in rr["buffer"].as_array().unwrap().iter().enumerate() {
            let cpp = b.as_i64().unwrap();
            expect_eq(
                "rng_randomize3.buffer",
                idx,
                "entry",
                cpp,
                (builder.random(3) - 1) as i64,
            );
        }
        // Rnd3 sequence exercises randomize3() + rnd3() end to end.
        let mut rng = LcgRng::new(seed);
        rng.randomize3();
        for (idx, b) in rr["rnd3_sequence"].as_array().unwrap().iter().enumerate() {
            let cpp = b.as_i64().unwrap();
            expect_eq(
                "rng_randomize3.rnd3_sequence",
                idx,
                "entry",
                cpp,
                rng.rnd3() as i64,
            );
        }
    }

    // 6b. C4Object::DigOutMaterialCast: drive a real Rust DigRect through a
    // Dig2ObjectRatio material, then compare the cast and twenty subsequent
    // draws with the mechanically extracted C++ body/LC_RNG_TRACE ledger.
    {
        let case = &golden["dig2object_rng"];
        let object_x = i(case, "object_x") as i32;
        let object_y = i(case, "object_y") as i32;
        let shape_y = i(case, "shape_y") as i32;
        let shape_height = i(case, "shape_height") as i32;

        let mut digger =
            Definition::from_script("DGRR", "Digger", "").expect("digger fixture compiles");
        digger.set_shape_rect(Some(DefinitionRect::new(-2, shape_y, 4, shape_height)));
        let mut gem = Definition::from_script("GEM_", "Gem", "").expect("gem fixture compiles");
        gem.set_rotateable(1);

        let material_source = r#"
            [Material Earth]
            Name=Earth
            Density=80
            DigFree=1
            Dig2Object=GEM_
            Dig2ObjectRatio=1
        "#;
        let library = clonk_resources::MaterialLibrary::parse(material_source)
            .expect("Dig2Object material fixture parses");
        let materials = MaterialSet::from_resource_library(&library);

        let mut engine = Engine::with_seed(0);
        engine
            .register_definition(digger)
            .expect("digger fixture registers");
        engine
            .register_definition(gem)
            .expect("gem fixture registers");
        engine.set_materials(materials);

        let mut pixels = vec![0_u8; 25];
        pixels[object_y as usize * 5 + object_x as usize] = 10;
        let mut densities = vec![0_i32; 128];
        densities[10] = 80;
        let mut material_names = vec![None; 128];
        material_names[10] = Some("Earth".to_string());
        let grid = PixelGrid::new(5, 5, pixels, densities, material_names, vec![None; 128]);
        let mut landscape = Landscape::flat(5, 5);
        landscape.set_pixel_grid(grid);
        engine.set_landscape(landscape);

        let digger_id = engine
            .spawn_object(
                SpawnConfig::new("DGRR")
                    .with_position(crate::Vector2::new(object_x, object_y))
                    .with_loaded(true),
            )
            .expect("digger fixture spawns");
        engine.rng = LcgRng::new(i(case, "seed") as u32);
        expect_eq(
            "dig2object_rng.rng_before",
            0,
            "count",
            i(&case["rng_before"], "count"),
            i64::from(engine.rng.count),
        );
        expect_eq_u64(
            "dig2object_rng.rng_before",
            0,
            "hold",
            u(&case["rng_before"], "hold"),
            u64::from(engine.rng.hold),
        );

        engine.apply_landscape_operations(vec![LandscapeOperation::DigRect {
            origin: crate::Vector2::new(object_x, object_y),
            width: 1,
            height: 1,
            requested: false,
            by_object: Some(digger_id),
        }]);

        let snapshot = engine.snapshot();
        let gems: Vec<_> = snapshot
            .objects
            .iter()
            .filter(|object| object.definition_id == "GEM_")
            .collect();
        expect_eq(
            "dig2object_rng.spawn",
            0,
            "count",
            i(&case["spawn"], "count"),
            gems.len() as i64,
        );
        let gem = gems.first().expect("Dig2Object fixture spawned one gem");
        expect_eq(
            "dig2object_rng.spawn",
            0,
            "x",
            i(&case["spawn"], "x"),
            i64::from(gem.position.x),
        );
        expect_eq(
            "dig2object_rng.spawn",
            0,
            "y",
            i(&case["spawn"], "y"),
            i64::from(gem.position.y),
        );
        expect_eq(
            "dig2object_rng.spawn",
            0,
            "rotation",
            i(&case["spawn"], "rotation"),
            i64::from(gem.rotation),
        );
        expect_eq(
            "dig2object_rng.rng_after_cast",
            0,
            "count",
            i(&case["rng_after_cast"], "count"),
            i64::from(engine.rng.count),
        );
        expect_eq_u64(
            "dig2object_rng.rng_after_cast",
            0,
            "hold",
            u(&case["rng_after_cast"], "hold"),
            u64::from(engine.rng.hold),
        );

        for (index, draw) in case["next"]
            .as_array()
            .expect("dig2object_rng.next is an array")
            .iter()
            .enumerate()
        {
            let range = i(draw, "range") as i32;
            expect_eq(
                "dig2object_rng.next",
                index,
                "value",
                i(draw, "value"),
                i64::from(engine.rng.random(range)),
            );
        }
        expect_eq(
            "dig2object_rng.rng_after",
            0,
            "count",
            i(&case["rng_after"], "count"),
            i64::from(engine.rng.count),
        );
        expect_eq_u64(
            "dig2object_rng.rng_after",
            0,
            "hold",
            u(&case["rng_after"], "hold"),
            u64::from(engine.rng.hold),
        );
    }

    // 6c. C4Game::ShakeObjects master-list selection, RNG consumption, and
    // raw C4Object::Fling fallback. The oracle compiles the complete method
    // bodies mechanically extracted from C4Game.cpp and C4Object.cpp.
    {
        let case = &golden["shake_objects"];
        let objects = case["objects"]
            .as_array()
            .expect("shake_objects.objects is an array");
        let caller_row = objects
            .iter()
            .find(|row| row["name"].as_str() == Some("caller"))
            .expect("shake_objects oracle includes caller row");
        let caused_by = i(case, "caused_by") as i32;
        let script = format!(
            "#strict\npublic func Shake() {{ SetController({caused_by}); ShakeObjects({}, {}, {}); SetController(-1); }}\n",
            i(case, "x"),
            i(case, "y"),
            i(case, "range")
        );
        let mut caller = Definition::from_script("SHKO", "Shake oracle", &script)
            .expect("shake oracle caller compiles");
        caller.set_category(CATEGORY_OBJECT);
        let mut target = Definition::from_script("SHKT", "Shake target", "#strict\n")
            .expect("shake oracle target compiles");
        target.set_category(CATEGORY_LIVING | CATEGORY_OBJECT);

        let mut engine = Engine::with_seed(i(case, "seed") as u64);
        engine
            .register_definition(caller)
            .expect("caller registers");
        engine
            .register_definition(target)
            .expect("target registers");
        engine
            .register_player(PlayerConfig::new(caused_by, "Shake cause"))
            .expect("shake cause player registers");

        let spawn_row = |engine: &mut Engine,
                         row: &Value,
                         definition_id: &str,
                         container: Option<crate::ObjectId>| {
            let config = SpawnConfig::new(definition_id)
                .with_custom_name(row["name"].as_str().expect("row name"))
                .with_position(crate::Vector2::new(i(row, "x") as i32, i(row, "y") as i32))
                .with_fixed_velocity(FixedVec2::new(
                    C4Fixed::from_raw(i(row, "xdir_before") as i32),
                    C4Fixed::from_raw(i(row, "ydir_before") as i32),
                ))
                .with_category(i(row, "category") as i32)
                .with_controller(OWNER_NONE)
                .with_alive(i(row, "ocf") as u32 & crate::ocf::ALIVE != 0);
            let id = engine
                .spawn_object(config)
                .expect("shake oracle row spawns");
            let index = engine.find_object_index(id).expect("shake row exists");
            let attach_mat = i(row, "attach_mat");
            engine.objects[index].state.status =
                ObjectStatus::from_script_value(i(row, "status") as i32)
                    .expect("valid C4Object status");
            engine.objects[index].state.container = container;
            engine.objects[index].state.t_attach = i(row, "t_attach_before") as u32;
            engine.objects[index].frame_t_attach = i(row, "t_attach_before") as u32;
            engine.objects[index].state.shape_attach = ShapeAttachRecord {
                mat_valid: attach_mat >= 0,
                mat_vehicle: attach_mat == 1,
                x: i(row, "x") as i32,
                y: i(row, "y") as i32,
                vtx: 0,
            };
            engine.objects[index].state.mobile = false;
            id
        };

        let caller_id = spawn_row(&mut engine, caller_row, "SHKO", None);
        let mut ids = HashMap::from([("caller".to_string(), caller_id)]);
        for row in objects {
            let name = row["name"].as_str().expect("row name");
            if name == "caller" {
                continue;
            }
            let container = (i(row, "contained") != 0).then_some(caller_id);
            ids.insert(
                name.to_string(),
                spawn_row(&mut engine, row, "SHKT", container),
            );
        }
        let master_order = objects
            .iter()
            .map(|row| ids[row["name"].as_str().expect("row name")])
            .collect::<Vec<_>>();
        engine.execution.exec_list = master_order.iter().rev().copied().collect();

        let rng_before = &case["rng_before"];
        expect_eq(
            "shake_objects.rng_before",
            0,
            "count",
            i(rng_before, "count"),
            engine.rng.count as i64,
        );
        expect_eq_u64(
            "shake_objects.rng_before",
            0,
            "hold",
            u(rng_before, "hold"),
            u64::from(engine.rng.hold),
        );
        expect_eq(
            "shake_objects.rng_before",
            0,
            "rnd3_ptr",
            i(rng_before, "rnd3_ptr"),
            engine.rng.rnd3_ptr() as i64,
        );

        let caller_index = engine
            .find_object_index(caller_id)
            .expect("shake caller exists");
        engine
            .call_object_function(caller_index, "Shake", Vec::new())
            .expect("ShakeObjects executes");

        let rng_after = &case["rng_after"];
        expect_eq(
            "shake_objects.rng_after",
            0,
            "count",
            i(rng_after, "count"),
            engine.rng.count as i64,
        );
        expect_eq_u64(
            "shake_objects.rng_after",
            0,
            "hold",
            u(rng_after, "hold"),
            u64::from(engine.rng.hold),
        );
        expect_eq(
            "shake_objects.rng_after",
            0,
            "rnd3_ptr",
            i(rng_after, "rnd3_ptr"),
            engine.rng.rnd3_ptr() as i64,
        );

        for (index, row) in objects.iter().enumerate() {
            let name = row["name"].as_str().expect("row name");
            let object_index = engine
                .find_object_index(ids[name])
                .unwrap_or_else(|| panic!("shake oracle row `{name}` remains"));
            let object = &engine.objects[object_index];
            expect_eq(
                "shake_objects.objects",
                index,
                "xdir_after",
                i(row, "xdir_after"),
                object.fixed_velocity.x.val() as i64,
            );
            expect_eq(
                "shake_objects.objects",
                index,
                "ydir_after",
                i(row, "ydir_after"),
                object.fixed_velocity.y.val() as i64,
            );
            expect_eq(
                "shake_objects.objects",
                index,
                "t_attach_after",
                i(row, "t_attach_after"),
                i64::from(object.state.t_attach),
            );
            expect_eq(
                "shake_objects.objects",
                index,
                "mobile_after",
                i(row, "mobile_after"),
                i64::from(u8::from(object.state.mobile)),
            );
            expect_eq(
                "shake_objects.objects",
                index,
                "controller_after",
                i(row, "controller_after"),
                i64::from(object.state.controller),
            );
        }
    }

    // 6c-2. C4Game::BlastObjects (C4Game.cpp:1265-1319) and C4Object::Blast
    // (C4Object.cpp:1416-1426), both compiled from the mechanically extracted
    // bodies. Three things are pinned that nothing else in the tree covers:
    //
    //   * the two independent hit tests — a direct hit widens the shape by five
    //     pixels on every side, while the shock wave is a plain `<= level`
    //     square around the object's own position, so `far_out_of_range` takes
    //     neither and `boundary_in`/`boundary_out` straddle the shock wave
    //     alone;
    //   * the shock-wave gate's shape (category, NoHorizontalMove, then a Grab
    //     of exactly 1 excusing vehicles and DFA_FLOAT actors); and
    //   * that the whole call consumes NOTHING from the synchronised stream and
    //     one Rnd3 per fling. `RandomCount` and `RandomHold` are unchanged
    //     across it while the Rnd3 pointer advances once per flung object, so a
    //     port that routed the fling's sign through `Random` would desynchronise
    //     every later frame while looking correct in isolation.
    //
    // `blast_incinerate_gate` runs the same fixture for Blast's
    // `Damage >= Def->BlastIncinerate` arm. It is a separate case, and one
    // without an RNG comparison, because the oracle records the `Incinerate`
    // call where the port starts the real fire effect — and that effect draws.
    for section in ["blast_objects", "blast_incinerate_gate"] {
        let case = &golden[section];
        let rows = case["objects"].as_array().expect("blast objects array");
        let blast_x = i(case, "x") as i32;
        let blast_y = i(case, "y") as i32;
        let level = i(case, "level") as i32;
        let caused_by = i(case, "caused_by") as i32;

        let mut engine = Engine::with_seed(i(case, "seed") as u64);
        let caller_script = format!(
            "#strict\npublic func Boom() {{ SetController({caused_by}); BlastObjects({blast_x}, {blast_y}, {level}); SetController(-1); }}\n"
        );
        let mut caller = Definition::from_script("BLSO", "Blast oracle", &caller_script)
            .expect("blast oracle caller compiles");
        caller.set_category(CATEGORY_OBJECT);
        engine
            .register_definition(caller)
            .expect("blast caller registers");
        engine
            .register_player(PlayerConfig::new(caused_by, "Blast cause"))
            .expect("blast cause player registers");

        // One definition per row: Grab, NoHorizontalMove, BlastIncinerate, mass
        // and the shape rect are all C4Def state, and the DFA_FLOAT row needs
        // an action map of its own.
        let mut ids = HashMap::new();
        let mut master_order = Vec::new();
        for (index, row) in rows.iter().enumerate() {
            let name = row["name"].as_str().expect("row name");
            let definition_id = format!("BL{index:02}");
            let mut definition = Definition::from_script(&definition_id, name, "#strict\n")
                .expect("blast row definition compiles");
            definition.set_category(i(row, "category") as i32);
            definition.set_mass(i(row, "mass") as i32);
            definition.set_grab(i(row, "grab") as i32);
            definition.set_no_horizontal_move(i(row, "no_horizontal_move") as i32);
            definition.set_blast_incinerate(i(row, "blast_incinerate") as i32);
            definition.set_shape_rect(Some(crate::DefinitionRect::new(
                i(row, "shape_x") as i32,
                i(row, "shape_y") as i32,
                i(row, "wdt") as i32,
                i(row, "hgt") as i32,
            )));
            // The oracle's `procedure` column is the C4Def ActMap entry the
            // object's action points at; -1 is ActIdle.
            if i(row, "procedure") >= 0 {
                definition.configure_actions(
                    Some("Float".to_owned()),
                    HashMap::from([(
                        "Float".to_owned(),
                        crate::ActionSpec::default().with_procedure("FLOAT"),
                    )]),
                );
            }
            engine
                .register_definition(definition)
                .expect("blast row definition registers");

            let id = engine
                .spawn_object(
                    SpawnConfig::new(&definition_id)
                        .with_custom_name(name)
                        .with_category(i(row, "category") as i32)
                        .with_controller(OWNER_NONE)
                        .with_alive(i(row, "alive") != 0),
                )
                .expect("blast oracle row spawns");
            let object_index = engine.find_object_index(id).expect("blast row exists");
            // Set the position rather than spawning at it: a spawn y is the
            // object's BOTTOM (C4Game::CreateObject), and these rows carry a
            // shape offset, so passing the oracle's y through SpawnConfig would
            // place the object eight pixels off its own coordinate.
            engine.objects[object_index].state.position =
                crate::Vector2::new(i(row, "x") as i32, i(row, "y") as i32);
            engine.objects[object_index].state.status =
                ObjectStatus::from_script_value(i(row, "status") as i32)
                    .expect("valid C4Object status");
            engine.objects[object_index].state.mobile = false;
            ids.insert(name.to_owned(), id);
            master_order.push(id);
        }
        // The oracle's contained row sits inside the first row's object, which
        // is what keeps it out of the uncontained arm entirely.
        if let Some(contained) = ids.get("contained").copied() {
            let container = ids["living_center"];
            let contained_index = engine
                .find_object_index(contained)
                .expect("contained row exists");
            engine.objects[contained_index].state.container = Some(container);
        }

        // The caller is not one of the oracle's rows; it sits far outside both
        // hit tests so it can run the blast without taking part in it.
        let caller_id = engine
            .spawn_object(
                SpawnConfig::new("BLSO")
                    .with_position(crate::Vector2::new(5_000, 5_000))
                    .with_controller(OWNER_NONE),
            )
            .expect("blast caller spawns");
        master_order.push(caller_id);
        engine.execution.exec_list = master_order.iter().rev().copied().collect();

        let compares_rng = case.get("rng_before").is_some();
        if compares_rng {
            let rng_before = &case["rng_before"];
            expect_eq(
                "blast_objects.rng_before",
                0,
                "count",
                i(rng_before, "count"),
                engine.rng.count as i64,
            );
            expect_eq_u64(
                "blast_objects.rng_before",
                0,
                "hold",
                u(rng_before, "hold"),
                u64::from(engine.rng.hold),
            );
            expect_eq(
                "blast_objects.rng_before",
                0,
                "rnd3_ptr",
                i(rng_before, "rnd3_ptr"),
                engine.rng.rnd3_ptr() as i64,
            );
        }

        let caller_index = engine
            .find_object_index(caller_id)
            .expect("blast caller exists");
        engine
            .call_object_function(caller_index, "Boom", Vec::new())
            .expect("BlastObjects executes");

        if compares_rng {
            let rng_after = &case["rng_after"];
            expect_eq(
                "blast_objects.rng_after",
                0,
                "count",
                i(rng_after, "count"),
                engine.rng.count as i64,
            );
            expect_eq_u64(
                "blast_objects.rng_after",
                0,
                "hold",
                u(rng_after, "hold"),
                u64::from(engine.rng.hold),
            );
            expect_eq(
                "blast_objects.rng_after",
                0,
                "rnd3_ptr",
                i(rng_after, "rnd3_ptr"),
                engine.rng.rnd3_ptr() as i64,
            );
        }

        for (index, row) in rows.iter().enumerate() {
            let name = row["name"].as_str().expect("row name");
            let object_index = engine
                .find_object_index(ids[name])
                .unwrap_or_else(|| panic!("blast oracle row `{name}` remains"));
            let object = &engine.objects[object_index];
            expect_eq(
                section,
                index,
                "xdir_after",
                i(row, "xdir_after"),
                object.fixed_velocity.x.val() as i64,
            );
            expect_eq(
                section,
                index,
                "ydir_after",
                i(row, "ydir_after"),
                object.fixed_velocity.y.val() as i64,
            );
            expect_eq(
                section,
                index,
                "mobile_after",
                i(row, "mobile_after"),
                i64::from(u8::from(object.state.mobile)),
            );
            expect_eq(
                section,
                index,
                "controller_after",
                i(row, "controller_after"),
                i64::from(object.state.controller),
            );
            // The oracle records DoDamage's arguments; the port runs the real
            // body, which for a plain fixture with no rules or effects lands on
            // exactly that sum.
            expect_eq(
                section,
                index,
                "damage_sum",
                i(row, "damage_sum"),
                i64::from(object.state.damage),
            );
            // Likewise Incinerate: the oracle counts the call, the port sets
            // the flag the real effect leads to.
            expect_eq(
                section,
                index,
                "incinerate_calls",
                i(row, "incinerate_calls"),
                i64::from(u8::from(object.state.on_fire)),
            );
        }
    }

    // 6c. C4Landscape::BlastFree (C4Landscape.cpp:881-888, 941-960,
    // 1022-1062): the oracle mechanically compiles the complete ClearPix,
    // BlastFreePix, and BlastFree bodies. A 7x7 authoritative Surface8 plane
    // mixes Earth/Granite and IFT pixels; Earth clears to sky/Tunnel+IFT,
    // Granite probabilistically shifts to Rock while preserving IFT. Compare
    // the pre-mutation BlastMatCount, every final byte, and exact RNG state.
    {
        let case = &golden["blast_free"];
        let library = clonk_resources::MaterialLibrary::parse(
            r#"
            [Material Earth]
            Name=Earth
            Density=100
            BlastFree=1

            [Material Granite]
            Name=Granite
            Density=100
            BlastShiftTo=Rock-Rough

            [Material Rock]
            Name=Rock
            Density=100
            BlastFree=1

            [Material Tunnel]
            Name=Tunnel
            Density=0
            TextureOverlay=Smooth2
        "#,
        )
        .expect("BlastFree oracle materials parse");

        let width = i(case, "width") as u32;
        let height = i(case, "height") as u32;
        let initial_bytes = case["initial_bytes"]
            .as_array()
            .expect("blast_free.initial_bytes is an array")
            .iter()
            .map(|byte| byte.as_u64().expect("pixel byte") as u8)
            .collect::<Vec<_>>();
        let mut densities = [0; 128];
        densities[1] = 100;
        densities[2] = 100;
        densities[3] = 100;
        densities[5] = 100;
        let mut names = vec![None; 128];
        names[1] = Some("Earth".to_string());
        names[2] = Some("Granite".to_string());
        names[3] = Some("Rock".to_string());
        names[4] = Some("Tunnel".to_string());
        names[5] = Some("Rock".to_string());
        names[6] = Some("Tunnel".to_string());
        let mut textures = vec![None; 128];
        textures[1] = Some("Smooth".to_string());
        textures[2] = Some("Smooth".to_string());
        textures[3] = Some("Smooth".to_string());
        textures[4] = Some("Rough".to_string());
        textures[5] = Some("Rough".to_string());
        textures[6] = Some("Smooth2".to_string());
        let grid = PixelGrid::new(
            width,
            height,
            initial_bytes,
            densities.to_vec(),
            names.clone(),
            textures.clone(),
        );

        let classifier = MapPixelClassifier::from_slots_with_library(
            densities,
            names,
            textures,
            vec![None; 128],
            library.clone(),
            vec![
                "Smooth".to_string(),
                "Rough".to_string(),
                "Smooth2".to_string(),
            ],
        );
        let mut texmap = classifier.into_runtime_state();
        texmap.set_default_material_entry("Earth", 1);
        texmap.set_default_material_entry("Granite", 2);
        texmap.set_default_material_entry("Rock", 3);
        texmap.set_default_material_entry("Tunnel", 6);
        let zero_texmap = texmap.clone();

        let mut engine = Engine::with_seed(i(case, "seed") as u64);
        engine.configure_materials_from_library(&library);
        let mut landscape = Landscape::flat(width, height as i32);
        landscape.set_pixel_grid(grid);
        landscape.set_raster_state(LandscapeRasterState::new(1, 0, texmap));
        engine.set_landscape(landscape);

        let rng_before = &case["rng_before"];
        expect_eq(
            "blast_free.rng_before",
            0,
            "count",
            i(rng_before, "count"),
            i64::from(engine.rng.count),
        );
        expect_eq_u64(
            "blast_free.rng_before",
            0,
            "hold",
            u(rng_before, "hold"),
            u64::from(engine.rng.hold),
        );
        expect_eq(
            "blast_free.rng_before",
            0,
            "rnd3_ptr",
            i(rng_before, "rnd3_ptr"),
            i64::from(engine.rng.rnd3_ptr()),
        );

        let result = engine
            .blast_circle(
                crate::Vector2::new(i(case, "x") as i32, i(case, "y") as i32),
                i(case, "radius") as i32,
                Some(i(case, "controller") as i32),
            )
            .expect("BlastFree oracle blast applies");

        let counts = &case["pre_counts"];
        for (index, name) in ["Earth", "Granite", "Rock", "Tunnel"]
            .into_iter()
            .enumerate()
        {
            let material = engine
                .materials()
                .id_of(name)
                .unwrap_or_else(|| panic!("BlastFree oracle material `{name}` exists"));
            let rust = result
                .pixel_count_by_material
                .get(&material)
                .copied()
                .unwrap_or_default();
            expect_eq(
                "blast_free.pre_counts",
                index,
                &name.to_ascii_lowercase(),
                i(counts, &name.to_ascii_lowercase()),
                i64::from(rust),
            );
        }

        let expected_bytes = case["final_bytes"]
            .as_array()
            .expect("blast_free.final_bytes is an array");
        let landscape = engine.landscape().expect("BlastFree landscape remains");
        for (index, expected) in expected_bytes.iter().enumerate() {
            let x = index as i32 % width as i32;
            let y = index as i32 / width as i32;
            expect_eq(
                "blast_free.final_bytes",
                index,
                "byte",
                expected.as_i64().expect("golden pixel byte"),
                i64::from(
                    landscape
                        .grid_byte_at(x, y)
                        .unwrap_or_else(|| panic!("BlastFree pixel ({x},{y}) exists")),
                ),
            );
        }

        let rng_after = &case["rng_after"];
        expect_eq(
            "blast_free.rng_after",
            0,
            "count",
            i(rng_after, "count"),
            i64::from(engine.rng.count),
        );
        expect_eq_u64(
            "blast_free.rng_after",
            0,
            "hold",
            u(rng_after, "hold"),
            u64::from(engine.rng.hold),
        );
        expect_eq(
            "blast_free.rng_after",
            0,
            "rnd3_ptr",
            i(rng_after, "rnd3_ptr"),
            i64::from(engine.rng.rnd3_ptr()),
        );

        let zero = &case["zero_radius"];
        let zero_x = i(zero, "x") as i32;
        let zero_y = i(zero, "y") as i32;
        let mut zero_bytes = vec![0; width as usize * height as usize];
        zero_bytes[zero_y as usize * width as usize + zero_x as usize] =
            i(zero, "initial_byte") as u8;
        let zero_grid = PixelGrid::new(
            width,
            height,
            zero_bytes,
            zero_texmap.densities.clone(),
            zero_texmap.material_names.clone(),
            zero_texmap.texture_names.clone(),
        );
        let mut zero_landscape = Landscape::flat(width, height as i32);
        zero_landscape.set_pixel_grid(zero_grid);
        zero_landscape.set_raster_state(LandscapeRasterState::new(1, 0, zero_texmap));
        let mut zero_engine = Engine::with_seed(i(zero, "seed") as u64);
        zero_engine.configure_materials_from_library(&library);
        zero_engine.set_landscape(zero_landscape);

        expect_eq(
            "blast_free.zero_radius.rng_before",
            0,
            "count",
            i(&zero["rng_before"], "count"),
            i64::from(zero_engine.rng.count),
        );
        expect_eq_u64(
            "blast_free.zero_radius.rng_before",
            0,
            "hold",
            u(&zero["rng_before"], "hold"),
            u64::from(zero_engine.rng.hold),
        );
        let zero_result = zero_engine
            .blast_circle(crate::Vector2::new(zero_x, zero_y), 0, Some(7))
            .expect("zero-radius BlastFree oracle blast applies");
        let earth = zero_engine
            .materials()
            .id_of("Earth")
            .expect("zero-radius oracle Earth exists");
        expect_eq(
            "blast_free.zero_radius",
            0,
            "pre_count",
            i(zero, "pre_count"),
            i64::from(
                zero_result
                    .pixel_count_by_material
                    .get(&earth)
                    .copied()
                    .unwrap_or_default(),
            ),
        );
        expect_eq(
            "blast_free.zero_radius",
            0,
            "final_byte",
            i(zero, "final_byte"),
            i64::from(
                zero_engine
                    .landscape()
                    .and_then(|landscape| landscape.grid_byte_at(zero_x, zero_y))
                    .expect("zero-radius center pixel remains addressable"),
            ),
        );
        expect_eq(
            "blast_free.zero_radius.rng_after",
            0,
            "count",
            i(&zero["rng_after"], "count"),
            i64::from(zero_engine.rng.count),
        );
        expect_eq_u64(
            "blast_free.zero_radius.rng_after",
            0,
            "hold",
            u(&zero["rng_after"], "hold"),
            u64::from(zero_engine.rng.hold),
        );
    }

    // 6d. C4Landscape::ExecuteScan / DoScan (C4Landscape.cpp:89-230). The
    // C++ oracle mechanically compiles both complete production bodies. Its
    // 6x8 Surface8 fixture has six Water pixels in every column, scans two
    // columns per frame, and freezes at four pixels per conversion pass
    // (`TempConvStrength=3` includes the starting pixel). Compare the exact
    // material counts and wrapping ScanX cursor after every Engine::tick.
    {
        let case = &golden["landscape_scan"];
        let width = i(case, "width") as u32;
        let height = i(case, "height") as u32;
        let water_depth = i(case, "water_depth") as u32;
        let water_byte = i(case, "water_byte") as u8;
        let ice_byte = i(case, "ice_byte") as u8;
        expect_eq(
            "landscape_scan",
            0,
            "scan_speed",
            i(case, "scan_speed"),
            i64::from((width as i32 / 500).clamp(2, 15)),
        );
        let library = clonk_resources::MaterialLibrary::parse(&format!(
            r#"
            [Material Water]
            Name=Water
            Density=30
            BelowTempConvert={}
            BelowTempConvertDir={}
            BelowTempConvertTo=Ice
            TempConvStrength={}

            [Material Ice]
            Name=Ice
            Density=80
            "#,
            i(case, "below_temperature"),
            i(case, "direction"),
            i(case, "strength"),
        ))
        .expect("landscape scan oracle materials parse");

        let mut bytes = vec![0; width as usize * height as usize];
        for y in 0..water_depth {
            bytes[y as usize * width as usize..(y + 1) as usize * width as usize].fill(water_byte);
        }
        let mut densities = vec![0; 128];
        densities[water_byte as usize] = 30;
        densities[ice_byte as usize] = 80;
        let mut material_names = vec![None; 128];
        material_names[water_byte as usize] = Some("Water".to_string());
        material_names[ice_byte as usize] = Some("Ice".to_string());
        let grid = PixelGrid::new(
            width,
            height,
            bytes,
            densities,
            material_names,
            vec![None; 128],
        );

        let mut engine = Engine::with_seed(0);
        engine.configure_materials_from_library(&library);
        engine.set_environment(
            crate::EnvironmentSettings::new(0)
                .with_temperature(i(case, "temperature") as i32)
                .with_climate(0)
                .with_temperature_range(0),
        );
        let mut landscape = Landscape::flat(width, height as i32);
        landscape.set_pixel_grid(grid);
        engine.set_landscape(landscape);

        let states = case["states"]
            .as_array()
            .expect("landscape_scan.states is an array");
        for (index, state) in states.iter().enumerate() {
            expect_eq(
                "landscape_scan.states",
                index,
                "frame",
                i(state, "frame"),
                index as i64,
            );
            let landscape = engine
                .landscape()
                .expect("landscape scan oracle landscape remains");
            let grid = landscape
                .pixel_grid()
                .expect("landscape scan oracle pixel grid remains");
            let water = grid
                .bytes()
                .iter()
                .filter(|&&byte| byte & 0x7f == water_byte)
                .count();
            let ice = grid
                .bytes()
                .iter()
                .filter(|&&byte| byte & 0x7f == ice_byte)
                .count();
            expect_eq(
                "landscape_scan.states",
                index,
                "scan_x",
                i(state, "scan_x"),
                i64::from(landscape.scan_x()),
            );
            expect_eq(
                "landscape_scan.states",
                index,
                "water",
                i(state, "water"),
                water as i64,
            );
            expect_eq(
                "landscape_scan.states",
                index,
                "ice",
                i(state, "ice"),
                ice as i64,
            );
            if index + 1 < states.len() {
                engine
                    .tick_without_snapshot()
                    .expect("landscape scan oracle frame executes");
            }
        }
    }

    // 6e. C4Object::ContactAction's bottom DFA_FLIGHT arm
    // (C4Object.cpp:4336-4351). The C++ oracle mechanically compiles that
    // complete switch arm and the real ObjectActionFlat helper. In particular,
    // a low-speed action with ObjectDisabled=1 takes the same FlatUp path as
    // OCF_HitSpeed4; a low-speed enabled action falls through to Walk.
    for (index, case) in golden["contact_action_bottom_flight"]
        .as_array()
        .expect("contact_action_bottom_flight is an array")
        .iter()
        .enumerate()
    {
        let mut definition = Definition::from_script("CFLI", "Contact flight oracle", "#strict\n")
            .expect("contact flight oracle compiles");
        definition.configure_actions(
            Some("Flight".to_string()),
            HashMap::from([
                (
                    "Flight".to_string(),
                    ActionSpec::default()
                        .with_procedure("FLIGHT")
                        .with_disabled(i(case, "disabled") != 0),
                ),
                ("FlatUp".to_string(), ActionSpec::default()),
                ("KneelDown".to_string(), ActionSpec::default()),
                (
                    "Walk".to_string(),
                    ActionSpec::default().with_procedure("WALK"),
                ),
            ]),
        );

        let mut engine = Engine::with_seed(0);
        engine
            .register_definition(definition)
            .expect("contact flight oracle registers");
        let id = engine
            .spawn_object(
                SpawnConfig::new("CFLI")
                    .with_action(ActionState::new("Flight"))
                    .with_direction(Direction::Right)
                    .with_fixed_velocity(FixedVec2::new(
                        C4Fixed::from_raw(i(case, "xdir_before") as i32),
                        C4Fixed::from_raw(i(case, "ydir_before") as i32),
                    ))
                    .with_category(CATEGORY_OBJECT)
                    .with_loaded(true),
            )
            .expect("contact flight oracle object spawns");
        let object_index = engine
            .find_object_index(id)
            .expect("contact flight oracle object exists");
        engine.objects[object_index].state.ocf = i(case, "ocf") as u32;
        let definition_id = engine.objects[object_index].definition_id.clone();
        engine
            .exec_contact_action(object_index, crate::CNAT_BOTTOM, &definition_id)
            .expect("bottom flight ContactAction executes");

        let object = &engine.objects[object_index];
        let action_after = match object.state.action.name.as_str() {
            "Flight" => 0,
            "FlatUp" => 1,
            "KneelDown" => 2,
            "Walk" => 3,
            action => panic!("unexpected contact-flight action `{action}`"),
        };
        expect_eq(
            "contact_action_bottom_flight",
            index,
            "action_after",
            i(case, "action_after"),
            action_after,
        );
        expect_eq(
            "contact_action_bottom_flight",
            index,
            "direction_after",
            i(case, "direction_after"),
            i64::from(object.state.direction.to_script_value()),
        );
        expect_eq(
            "contact_action_bottom_flight",
            index,
            "xdir_after",
            i(case, "xdir_after"),
            i64::from(object.fixed_velocity.x.val()),
        );
        expect_eq(
            "contact_action_bottom_flight",
            index,
            "ydir_after",
            i(case, "ydir_after"),
            i64::from(object.fixed_velocity.y.val()),
        );
    }

    // 6f. C4Object::ContactAction's ceiling and wall DFA_FLIGHT arms
    // (C4Object.cpp:4400-4500), including the common unresolved-flight tail.
    // The enabled controls take Hangle/Scale. At the same low speed, a
    // disabled action must take Tumble instead; the tail then slides it free
    // and zeroes the transient +/-FIXED100(150) wall velocity.
    for (index, case) in golden["contact_action_top_side_flight"]
        .as_array()
        .expect("contact_action_top_side_flight is an array")
        .iter()
        .enumerate()
    {
        let mut definition =
            Definition::from_script("CFTS", "Contact top/side oracle", "#strict\n")
                .expect("contact top/side oracle compiles");
        definition.configure_actions(
            Some("Flight".to_string()),
            HashMap::from([
                (
                    "Flight".to_string(),
                    ActionSpec::default()
                        .with_procedure("FLIGHT")
                        .with_disabled(i(case, "disabled") != 0),
                ),
                (
                    "Tumble".to_string(),
                    ActionSpec::default().with_procedure("FLIGHT"),
                ),
                (
                    "Scale".to_string(),
                    ActionSpec::default().with_procedure("SCALE"),
                ),
                (
                    "Hangle".to_string(),
                    ActionSpec::default().with_procedure("HANGLE"),
                ),
            ]),
        );
        definition.set_physical(PhysicalInfo {
            can_scale: i(case, "can_scale") as i32,
            can_hangle: i(case, "can_hangle") as i32,
            ..PhysicalInfo::default()
        });

        let mut engine = Engine::with_seed(0);
        engine
            .register_definition(definition)
            .expect("contact top/side oracle registers");
        let id = engine
            .spawn_object(
                SpawnConfig::new("CFTS")
                    .with_position(crate::Vector2::new(
                        i(case, "x_before") as i32,
                        i(case, "y_before") as i32,
                    ))
                    .with_action(ActionState::new("Flight"))
                    .with_direction(Direction::Right)
                    .with_fixed_velocity(FixedVec2::new(
                        C4Fixed::from_raw(i(case, "xdir_before") as i32),
                        C4Fixed::from_raw(i(case, "ydir_before") as i32),
                    ))
                    .with_category(CATEGORY_OBJECT)
                    .with_loaded(true),
            )
            .expect("contact top/side oracle object spawns");
        let object_index = engine
            .find_object_index(id)
            .expect("contact top/side oracle object exists");
        engine.objects[object_index].state.ocf = i(case, "ocf") as u32;
        let definition_id = engine.objects[object_index].definition_id.clone();
        let contact = i(case, "contact") as u32;
        engine
            .exec_contact_action(object_index, contact, &definition_id)
            .expect("top/side flight ContactAction executes");

        let object = &engine.objects[object_index];
        let action_after = match object.state.action.name.as_str() {
            "Flight" => 0,
            "Tumble" => 4,
            "Scale" => 5,
            "Hangle" => 6,
            action => panic!("unexpected top/side contact action `{action}`"),
        };
        let xdir_before_flight_stuck = if i(case, "disabled") != 0 {
            i64::from(contact_action_wall_tumble_x(contact).val())
        } else {
            0
        };
        for (field, actual) in [
            ("action_after", action_after),
            (
                "direction_after",
                i64::from(object.state.direction.to_script_value()),
            ),
            ("xdir_before_flight_stuck", xdir_before_flight_stuck),
            ("ydir_before_flight_stuck", 0),
            ("x_after", i64::from(object.state.position.x)),
            ("y_after", i64::from(object.state.position.y)),
            ("xdir_after", i64::from(object.fixed_velocity.x.val())),
            ("ydir_after", i64::from(object.fixed_velocity.y.val())),
        ] {
            expect_eq(
                "contact_action_top_side_flight",
                index,
                field,
                i(case, field),
                actual,
            );
        }
    }

    // 7. Material corrosion execution RNG ordering.
    for (idx, e) in golden["material_corrode_rng"]
        .as_array()
        .unwrap()
        .iter()
        .enumerate()
    {
        let seed = i(e, "seed") as u32;
        let custom = i(e, "custom") != 0;
        let mut rng = LcgRng::new(seed);
        let success = if custom {
            evaluate_corrosion(0, 0, Some(i(e, "rate") as i32), &mut rng)
        } else {
            evaluate_corrosion(
                i(e, "corrosive") as i32,
                i(e, "corrode") as i32,
                None,
                &mut rng,
            )
        };
        if success {
            consume_corrosion_effect_rng(&mut rng);
        }
        expect_eq(
            "material_corrode_rng",
            idx,
            "success",
            i(e, "success"),
            success as i64,
        );
        expect_eq(
            "material_corrode_rng",
            idx,
            "count",
            i(e, "count"),
            rng.count as i64,
        );
        expect_eq(
            "material_corrode_rng",
            idx,
            "hold",
            i(e, "hold"),
            rng.hold as i64,
        );
    }

    // 8. Mass-mover transfer RNG ordering: Random(10) before Rnd3().
    for (case_idx, e) in golden["mass_mover_transfer_rng"]
        .as_array()
        .unwrap()
        .iter()
        .enumerate()
    {
        let seed = i(e, "seed") as u32;
        let mut rng = LcgRng::new(seed);
        rng.randomize3();
        for (idx, call) in e["calls"].as_array().unwrap().iter().enumerate() {
            let random10 = rng.random(10);
            let rnd3 = rng.rnd3();
            let label = format!("mass_mover_transfer_rng[{case_idx}]");
            expect_eq(
                &label,
                idx,
                "random10",
                i(call, "random10"),
                random10 as i64,
            );
            expect_eq(&label, idx, "rnd3", i(call, "rnd3"), rnd3 as i64);
            expect_eq(
                &label,
                idx,
                "execute_immediately",
                i(call, "execute_immediately"),
                (rnd3 == 0) as i64,
            );
        }
        expect_eq(
            "mass_mover_transfer_rng",
            case_idx,
            "count",
            i(e, "count"),
            rng.count as i64,
        );
        expect_eq(
            "mass_mover_transfer_rng",
            case_idx,
            "hold",
            i(e, "hold"),
            rng.hold as i64,
        );
    }

    // 9. C4Value map-key hash: C4Value.cpp:923-1029.
    {
        let section = &golden["script_value_hash"];
        expect_eq_u64(
            "script_value_hash",
            0,
            "sizeof_size_t",
            u(section, "sizeof_size_t"),
            std::mem::size_of::<usize>() as u64,
        );

        for (idx, e) in section["hash_combine"]
            .as_array()
            .unwrap()
            .iter()
            .enumerate()
        {
            let seed = u(e, "seed") as usize;
            let next = u(e, "next") as usize;
            expect_eq_u64(
                "script_value_hash.hash_combine",
                idx,
                "hash",
                u(e, "hash"),
                c4_hash_combine(seed, next) as u64,
            );
        }

        let mut map = ValueMap::new();
        map.insert("a".to_string(), ScriptValue::Int(1));
        map.insert(
            "b".to_string(),
            ScriptValue::Array(vec![ScriptValue::Int(2), ScriptValue::Int(3)]),
        );
        let mut reversed = ValueMap::new();
        reversed.insert(
            "b".to_string(),
            ScriptValue::Array(vec![ScriptValue::Int(2), ScriptValue::Int(3)]),
        );
        reversed.insert("a".to_string(), ScriptValue::Int(1));

        let mixed_entries = [
            (ScriptValue::Int(42), ScriptValue::String("int".into())),
            (ScriptValue::Bool(true), ScriptValue::Int(7)),
            (ScriptValue::C4Id("CLNK".into()), ScriptValue::Bool(false)),
            (
                ScriptValue::Object(77),
                ScriptValue::String("object".into()),
            ),
            (
                ScriptValue::Array(vec![ScriptValue::Int(1), ScriptValue::Bool(true)]),
                ScriptValue::C4Id("1337".into()),
            ),
        ];
        let mixed = ValueMap::from(mixed_entries.clone());
        let mixed_reversed = mixed_entries.into_iter().rev().collect();

        let cases = [
            ("nil", ScriptValue::Nil),
            ("int_zero", ScriptValue::Int(0)),
            ("int_42", ScriptValue::Int(42)),
            ("int_minus_one", ScriptValue::Int(-1)),
            ("bool_false", ScriptValue::Bool(false)),
            ("bool_true", ScriptValue::Bool(true)),
            ("id_CLNK", ScriptValue::C4Id("CLNK".to_string())),
            ("id_1337", ScriptValue::C4Id("1337".to_string())),
            ("string_empty", ScriptValue::String(String::new().into())),
            ("string_alpha", ScriptValue::String("alpha".to_string().into())),
            (
                "string_16",
                ScriptValue::String("abcdefghijklmnop".to_string().into()),
            ),
            (
                "string_24",
                ScriptValue::String("abcdefghijklmnopqrstuvwx".to_string().into()),
            ),
            (
                "string_40",
                ScriptValue::String("abcdefghijklmnopqrstuvwxyz0123456789ABCD".to_string().into()),
            ),
            (
                "string_80",
                ScriptValue::String(
                    "0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef"
                        .to_string()
                        .into(),
                ),
            ),
            (
                "array_1_true_x",
                ScriptValue::Array(vec![
                    ScriptValue::Int(1),
                    ScriptValue::Bool(true),
                    ScriptValue::String("x".to_string().into()),
                ]),
            ),
            ("map_a1_b23", ScriptValue::Proplist(map)),
            ("map_b23_a1", ScriptValue::Proplist(reversed)),
            ("map_mixed_keys", ScriptValue::Proplist(mixed)),
            (
                "map_mixed_keys_reversed",
                ScriptValue::Proplist(mixed_reversed),
            ),
        ];
        for (idx, (name, value)) in cases.iter().enumerate() {
            let entry = section["values"]
                .as_array()
                .unwrap()
                .iter()
                .find(|candidate| candidate["name"].as_str() == Some(*name))
                .unwrap_or_else(|| panic!("missing script_value_hash case `{name}`"));
            expect_eq_u64(
                "script_value_hash.values",
                idx,
                "hash",
                u(entry, "hash"),
                value.c4_value_hash() as u64,
            );
        }
    }

    // 9b. C4ScriptCnvMap conversion table + ConvertTo dispatch: C4Value.cpp:488-598.
    {
        let section = &golden["script_value_convert"];
        expect_eq(
            "script_value_convert",
            0,
            "type_count",
            i(section, "type_count"),
            C4VType::ALL.len() as i64,
        );

        // The 81-cell classification grid, source row × destination column.
        for (row, row_str) in section["table"].as_array().unwrap().iter().enumerate() {
            for (col, code) in row_str.as_str().unwrap().chars().enumerate() {
                let rust = cnv_fn(C4VType::ALL[row], C4VType::ALL[col]).code();
                assert_eq!(
                    code, rust,
                    "PARITY DIVERGENCE in `script_value_convert.table` cell [{row}][{col}]: \
                     C++ golden = {code}, Rust = {rust}"
                );
            }
        }

        // Per-(value, target type, #strict) ConvertTo results.
        for (idx, e) in section["convert"].as_array().unwrap().iter().enumerate() {
            let value = convert_case_value(e["name"].as_str().unwrap());
            expect_eq(
                "script_value_convert.convert",
                idx,
                "from",
                i(e, "from"),
                value.c4v_type().index() as i64,
            );
            let to = C4VType::ALL[i(e, "to") as usize];
            let strict = i(e, "strict") != 0;
            expect_eq(
                "script_value_convert.convert",
                idx,
                "result",
                i(e, "result"),
                value.convert_to(to, strict) as i64,
            );
        }
    }

    // 10. FnGetKiller/FnSetKiller (C4Script.cpp:1333-1347), whose C++
    // implementation delegates to the production C4ScriptKiller helper used
    // by the oracle. Drive the Rust HOST FUNCTIONS through the real script VM
    // so registration, default-self behavior, foreign/arrow dispatch and the
    // pending-update seam all participate in the differential.
    {
        let section = &golden["script_killer"];
        let caller_script = r#"#strict
local iInitial, iSetSelf, iReadSelf, iInvalid, iAfterInvalid;
local iClearSelf, iReadCleared, iSetForeign, iReadForeign;
local iArrowClear, iArrowRead;
func Trigger(object pOther) {
    iInitial = GetKiller();
    iSetSelf = SetKiller(1);
    iReadSelf = GetKiller();
    iInvalid = SetKiller(9);
    iAfterInvalid = GetKiller();
    iClearSelf = SetKiller(-1);
    iReadCleared = GetKiller();
    iSetForeign = SetKiller(1, pOther);
    iReadForeign = GetKiller(pOther);
    iArrowClear = pOther->SetKiller(-1);
    iArrowRead = pOther->GetKiller();
    return(1);
}
"#;
        let mut engine = Engine::with_seed(0);
        engine
            .register_player(PlayerConfig::new(1, "P1"))
            .expect("killer differential player registers");
        engine
            .register_script_definition("CALL", "Caller", caller_script)
            .expect("killer differential caller registers");
        engine
            .register_script_definition("OTHR", "Other", "#strict\n")
            .expect("killer differential target registers");
        let caller_id = engine
            .spawn_object(SpawnConfig::new("CALL").with_category(CATEGORY_OBJECT))
            .expect("killer differential caller spawns");
        let other_id = engine
            .spawn_object(SpawnConfig::new("OTHR").with_category(CATEGORY_OBJECT))
            .expect("killer differential target spawns");
        let caller_idx = engine
            .find_object_index(caller_id)
            .expect("killer differential caller exists");
        engine
            .call_object_function(
                caller_idx,
                "Trigger",
                vec![ScriptValue::Object(other_id.as_u64())],
            )
            .expect("killer differential script runs");

        let caller_idx = engine
            .find_object_index(caller_id)
            .expect("killer differential caller remains");
        let locals = &engine.objects[caller_idx].state.local_vars;
        let rust_local = |name: &str| match locals.get(name) {
            Some(ScriptValue::Int(value)) => i64::from(*value),
            Some(ScriptValue::Bool(value)) => i64::from(*value),
            value => panic!("killer differential local `{name}` has unexpected value {value:?}"),
        };
        for (idx, (golden_key, local_name)) in [
            ("initial", "iInitial"),
            ("set_self", "iSetSelf"),
            ("read_self", "iReadSelf"),
            ("set_invalid", "iInvalid"),
            ("after_invalid", "iAfterInvalid"),
            ("clear_self", "iClearSelf"),
            ("read_cleared", "iReadCleared"),
            ("set_foreign", "iSetForeign"),
            ("read_foreign", "iReadForeign"),
            ("arrow_clear", "iArrowClear"),
            ("arrow_read", "iArrowRead"),
        ]
        .into_iter()
        .enumerate()
        {
            expect_eq(
                "script_killer",
                idx,
                golden_key,
                i(section, golden_key),
                rust_local(local_name),
            );
        }
        expect_eq(
            "script_killer",
            11,
            "self_final",
            i(section, "self_final"),
            i64::from(engine.objects[caller_idx].last_energy_loss_cause),
        );
        let other_idx = engine
            .find_object_index(other_id)
            .expect("killer differential target remains");
        expect_eq(
            "script_killer",
            12,
            "foreign_final",
            i(section, "foreign_final"),
            i64::from(engine.objects[other_idx].last_energy_loss_cause),
        );

        // No C4Aul object context: invoke the same registered Rust hosts from
        // a bare clonk-script engine. This matches C4ScriptKiller's null/null
        // oracle cases and pins the NO_OWNER/false fallbacks.
        let mut bare = clonk_script::Engine::new();
        crate::compat::register_host_functions(&mut bare);
        bare.add_script(
            clonk_script::Script::compile(
                "global func ReadNoContext() { return GetKiller(); }\n\
                 global func WriteNoContext() { return SetKiller(1); }\n",
            )
            .expect("bare killer differential script compiles"),
        );
        let bare_result = |function: &str, bare: &mut clonk_script::Engine| match bare
            .call(function, &[])
            .unwrap_or_else(|error| panic!("bare killer call `{function}` failed: {error}"))
        {
            ScriptValue::Int(value) => i64::from(value),
            ScriptValue::Bool(value) => i64::from(value),
            value => panic!("bare killer call `{function}` returned {value:?}"),
        };
        expect_eq(
            "script_killer",
            13,
            "get_no_context",
            i(section, "get_no_context"),
            bare_result("ReadNoContext", &mut bare),
        );
        expect_eq(
            "script_killer",
            14,
            "set_no_context",
            i(section, "set_no_context"),
            bare_result("WriteNoContext", &mut bare),
        );
        expect_eq(
            "script_killer",
            15,
            "no_owner_constant",
            i(section, "get_no_context"),
            i64::from(OWNER_NONE),
        );
    }

    // 10b. FnEval -> DirectExec context selection (C4Script.cpp:4501-4513;
    // C4AulExec.cpp:1674-1683). The C++ oracle executes both mechanically
    // extracted production blocks. Rust drives the same three contexts through
    // real C4Script: the object sentinel requires both its named local and its
    // definition-owned function, while DefinitionCall supplies Def without Obj
    // and global->eval clears both so Game.Script owns the expression.
    {
        let object_script = r#"#strict 2
local power;
func Probe()
{
    power = 50;
    return eval("Explode(power)");
}
func Explode(value) { return value + 1; }
"#;
        let definition_script = r#"#strict
func DefinitionProbe() { return eval("DefinitionHelper()"); }
func DefinitionHelper() { return 62; }
"#;
        let definition_caller_script = r#"#strict
func Probe() { return DefinitionCall(DEFV, "DefinitionProbe"); }
"#;
        let global_caller_script = r#"#strict 3
func Probe() { return global->eval("ScenarioHelper()"); }
"#;
        let scenario_script = r#"#strict 3
func ScenarioHelper() { return 73; }
"#;

        let mut engine = Engine::with_seed(29);
        for (id, name, script) in [
            ("OBJV", "Eval object receiver", object_script),
            ("DEFV", "Eval definition receiver", definition_script),
            ("CALL", "Eval definition caller", definition_caller_script),
            ("GEVL", "Eval game caller", global_caller_script),
        ] {
            engine
                .register_definition(
                    Definition::from_script(id, name, script)
                        .unwrap_or_else(|error| panic!("{name} compiles: {error}")),
                )
                .unwrap_or_else(|error| panic!("{name} registers: {error}"));
        }
        engine
            .install_scenario_script_with_convention("Scenario", scenario_script, true)
            .expect("eval differential scenario script installs");
        let object = engine
            .spawn_object(SpawnConfig::new("OBJV"))
            .expect("eval differential object receiver spawns");
        let definition_caller = engine
            .spawn_object(SpawnConfig::new("CALL"))
            .expect("eval differential definition caller spawns");
        let global_caller = engine
            .spawn_object(SpawnConfig::new("GEVL"))
            .expect("eval differential game caller spawns");

        let call_probe = |engine: &mut Engine, id| {
            let index = engine
                .find_object_index(id)
                .expect("eval differential caller remains");
            match engine
                .call_object_function(index, "Probe", Vec::new())
                .expect("eval differential probe runs")
            {
                ScriptValue::Int(value) => i64::from(value),
                value => panic!("eval differential probe returned {value:?}"),
            }
        };
        let rust_results = HashMap::from([
            (
                "object_definition",
                (1_i64, 1_i64, 2_i64, 1_i64, call_probe(&mut engine, object)),
            ),
            (
                "definition_only",
                (
                    0_i64,
                    1_i64,
                    1_i64,
                    2_i64,
                    call_probe(&mut engine, definition_caller),
                ),
            ),
            (
                "game_script",
                (
                    0_i64,
                    0_i64,
                    3_i64,
                    3_i64,
                    call_probe(&mut engine, global_caller),
                ),
            ),
        ]);

        for (index, case) in golden["eval_direct_exec_context"]
            .as_array()
            .expect("eval_direct_exec_context is a C++ oracle array")
            .iter()
            .enumerate()
        {
            let name = case["name"]
                .as_str()
                .expect("eval_direct_exec_context case has a name");
            let &(has_object, has_definition, caller_strict, receiver, result) = rust_results
                .get(name)
                .unwrap_or_else(|| panic!("unknown eval_direct_exec_context case `{name}`"));
            for (field, rust) in [
                ("has_object", has_object),
                ("has_definition", has_definition),
                ("caller_strict", caller_strict),
                ("expected_receiver", receiver),
                ("receiver", receiver),
                ("scope_valid", 1),
                ("direct_strict", caller_strict),
                ("result", result),
            ] {
                expect_eq(
                    "eval_direct_exec_context",
                    index,
                    field,
                    i(case, field),
                    rust,
                );
            }
        }
    }

    // 10c. C4Effect::Execute passes pCommandTarget—not the affected pForObj—
    // to C4AulFunc::Exec (oracle-src-pinned src/C4Effect.cpp:319-363).
    // With only idCommandTarget set, the mechanically extracted C++ path
    // therefore gives FnGetX/FnGetY a null cthr->Obj while retaining the
    // carrier as the timer's first argument (src/C4AulExec.cpp:330-364,
    // 1638-1649; src/C4Script.cpp:1198-1202,1293-1297).
    {
        let section = &golden["definition_commanded_effect_position"];
        let carrier_script = r#"#strict 2
func Arm()
{
    return AddEffect("Origin", this(), 100, 1, 0, PROB);
}
"#;
        let callback_script = r#"#strict 2
func FxOriginTimer(object target, int number, int time)
{
    EffectVar(0, target, number) = GetX();
    EffectVar(1, target, number) = GetY();
    EffectVar(2, target, number) = GetX(target);
    EffectVar(3, target, number) = GetY(target);
    EffectVar(4, target, number) = time;
    EffectVar(5, target, number) = !this();
    EffectVar(6, target, number) = GetID(target) == CARR;
    EffectVar(7, target, number) = number;
    return 0;
}
"#;

        let mut carrier = Definition::from_script("CARR", "Effect carrier", carrier_script)
            .expect("effect receiver differential carrier compiles");
        carrier.set_c4_callback_convention(true);
        let mut callback = Definition::from_script("PROB", "Effect callback", callback_script)
            .expect("effect receiver differential callback compiles");
        callback.set_c4_callback_convention(true);

        let mut engine = Engine::with_seed(0);
        engine
            .register_definition(carrier)
            .expect("effect receiver differential carrier registers");
        engine
            .register_definition(callback)
            .expect("effect receiver differential callback registers");
        let carrier = engine
            .spawn_object(
                SpawnConfig::new("CARR")
                    .with_position(crate::Vector2::new(
                        i(section, "carrier_x") as i32,
                        i(section, "carrier_y") as i32,
                    ))
                    .with_mobile(false),
            )
            .expect("effect receiver differential carrier spawns");
        let carrier_index = engine
            .find_object_index(carrier)
            .expect("effect receiver differential carrier exists");
        engine
            .call_object_function(carrier_index, "Arm", Vec::new())
            .expect("definition-commanded effect installs");
        engine
            .tick_without_snapshot()
            .expect("definition-commanded effect timer runs");

        let carrier_index = engine
            .find_object_index(carrier)
            .expect("effect receiver differential carrier remains");
        let carrier_state = &engine.objects[carrier_index].state;
        let effect = carrier_state
            .effects
            .iter()
            .find(|effect| effect.name == "Origin")
            .expect("definition-commanded effect remains active");
        let var_i64 = |index: usize, field: &str| match effect.var(index) {
            EffectVarValue::Int(value) => i64::from(value),
            EffectVarValue::Bool(value) => i64::from(value),
            EffectVarValue::RawBool(value) => i64::from(value != 0),
            value => panic!(
                "definition_commanded_effect_position `{field}` has unexpected value {value:?}"
            ),
        };
        let position_var = |index: usize, field: &str| match effect.var(index) {
            EffectVarValue::Nil => Value::Null,
            EffectVarValue::Int(value) => Value::from(value),
            value => panic!(
                "definition_commanded_effect_position `{field}` has unexpected value {value:?}"
            ),
        };

        for (index, (field, rust)) in [
            ("carrier_x", i64::from(carrier_state.position.x)),
            ("carrier_y", i64::from(carrier_state.position.y)),
            (
                "has_id_command_target",
                i64::from(effect.command_id.as_deref() == Some("PROB")),
            ),
            (
                "command_target_is_null",
                i64::from(effect.command_target.is_none()),
            ),
            (
                "callback_ran",
                i64::from(!matches!(effect.var(4), EffectVarValue::Nil)),
            ),
            (
                "callback_receiver_is_null",
                var_i64(5, "callback_receiver_is_null"),
            ),
            (
                "callback_target_is_carrier",
                var_i64(6, "callback_target_is_carrier"),
            ),
            ("number", var_i64(7, "number")),
            ("time", var_i64(4, "time")),
        ]
        .into_iter()
        .enumerate()
        {
            expect_eq(
                "definition_commanded_effect_position",
                index,
                field,
                i(section, field),
                rust,
            );
        }
        for (index, (field, effect_var)) in [
            ("implicit_x", 0_usize),
            ("implicit_y", 1),
            ("explicit_x", 2),
            ("explicit_y", 3),
        ]
        .into_iter()
        .enumerate()
        {
            expect_json_eq(
                "definition_commanded_effect_position",
                index,
                field,
                section[field].clone(),
                position_var(effect_var, field),
            );
        }
    }

    // 10d. C4Effect routes the warning-only conversion marker at its
    // callback boundary only. The golden is emitted by the pinned C++
    // Execute and DoCall bodies together with their extracted script-function
    // conversion entry, so verify both deferred Timer and EffectCall paths.
    {
        let section = &golden["effect_callback_conversion"];
        let raw_fixed_x = |engine: &Engine, object| {
            let snapshot = engine
                .object_snapshot(object)
                .expect("effect conversion carrier remains live");
            snapshot
                .fixed_velocity
                .unwrap_or_else(|| FixedVec2::from_ints(snapshot.velocity.x, snapshot.velocity.y))
                .x
                .val()
        };

        let mut timer_pre_strict3 = Engine::new();
        register_real_c4_effect_definition(
            &mut timer_pre_strict3,
            "TMHP",
            "Timer warning conversion host",
            r#"#strict 2
func Arm()
{
  return(AddEffect("Oracle", this(), 100, 1, 0, TMCP));
}
func Read()
{
  return(ReadTimerPreStrict3Value());
}
"#,
        );
        register_real_c4_effect_definition(
            &mut timer_pre_strict3,
            "TMCP",
            "Timer warning conversion callback",
            r#"#strict 2
static callback_value;
func FxOracleTimer(int target, int number, int time)
{
  callback_value = GetType(target) == 4;
  return(0);
}
global func ReadTimerPreStrict3Value() { return(callback_value); }
"#,
        );
        let timer_pre_object = timer_pre_strict3
            .spawn_object(SpawnConfig::new("TMHP"))
            .expect("pre-strict3 timer carrier spawns");
        let timer_pre_index = timer_pre_strict3
            .find_object_index(timer_pre_object)
            .expect("pre-strict3 timer carrier exists");
        timer_pre_strict3
            .call_object_function(timer_pre_index, "Arm", Vec::new())
            .expect("pre-strict3 timer installs");
        timer_pre_strict3
            .tick_without_snapshot()
            .expect("pre-strict3 timer warns and runs");
        let timer_pre_reader = timer_pre_strict3
            .spawn_object(SpawnConfig::new("TMCP"))
            .expect("pre-strict3 timer callback reader spawns");
        let timer_pre_index = timer_pre_strict3
            .find_object_index(timer_pre_reader)
            .expect("pre-strict3 timer callback reader remains live");
        let timer_pre_value = timer_pre_strict3
            .call_object_function(timer_pre_index, "ReadTimerPreStrict3Value", Vec::new())
            .expect("pre-strict3 timer callback value reads");
        let mut timer_strict3 = Engine::new();
        register_real_c4_effect_definition(
            &mut timer_strict3,
            "TMHS",
            "Strict timer conversion host",
            r#"#strict 3
func Arm()
{
  return(AddEffect("Oracle", this(), 100, 1, nil, TMCS));
}
func Read()
{
  return(ReadTimerStrict3Value());
}
"#,
        );
        register_real_c4_effect_definition(
            &mut timer_strict3,
            "TMCS",
            "Strict timer conversion callback",
            r#"#strict 3
static callback_value;
func FxOracleTimer(int target, int number, int time)
{
  callback_value = 1;
  return(0);
}
global func ReadTimerStrict3Value() { return(callback_value); }
"#,
        );
        let timer_strict_object = timer_strict3
            .spawn_object(SpawnConfig::new("TMHS"))
            .expect("strict timer carrier spawns");
        let timer_strict_index = timer_strict3
            .find_object_index(timer_strict_object)
            .expect("strict timer carrier exists");
        timer_strict3
            .call_object_function(timer_strict_index, "Arm", Vec::new())
            .expect("strict timer installs");
        timer_strict3
            .tick_without_snapshot()
            .expect("strict timer conversion fails safe");
        let timer_strict_reader = timer_strict3
            .spawn_object(SpawnConfig::new("TMCS"))
            .expect("strict timer callback reader spawns");
        let timer_strict_index = timer_strict3
            .find_object_index(timer_strict_reader)
            .expect("strict timer callback reader remains live");
        let timer_strict_value = timer_strict3
            .call_object_function(timer_strict_index, "ReadTimerStrict3Value", Vec::new())
            .expect("strict timer callback value reads");

        let mut timer_strict3_reference = Engine::new();
        register_real_c4_effect_definition(
            &mut timer_strict3_reference,
            "TMHR",
            "Strict timer reference host",
            r#"#strict 3
func Arm()
{
  return(AddEffect("Oracle", this(), 100, 1, nil, TMCR));
}
func Read()
{
  return(ReadTimerStrict3ReferenceValue());
}
"#,
        );
        register_real_c4_effect_definition(
            &mut timer_strict3_reference,
            "TMCR",
            "Strict timer reference callback",
            r#"#strict 3
static callback_value;
func FxOracleTimer(int &target, int number, int time)
{
  SetXDir(17, target);
  callback_value = 1;
  return(0);
}
global func ReadTimerStrict3ReferenceValue() { return(callback_value); }
"#,
        );
        let timer_reference_object = timer_strict3_reference
            .spawn_object(SpawnConfig::new("TMHR"))
            .expect("strict reference timer carrier spawns");
        let timer_reference_index = timer_strict3_reference
            .find_object_index(timer_reference_object)
            .expect("strict reference timer carrier exists");
        timer_strict3_reference
            .call_object_function(timer_reference_index, "Arm", Vec::new())
            .expect("strict reference timer installs");
        timer_strict3_reference
            .tick_without_snapshot()
            .expect("strict reference timer conversion fails safe");
        let timer_reference_reader = timer_strict3_reference
            .spawn_object(SpawnConfig::new("TMCR"))
            .expect("strict reference timer callback reader spawns");
        let timer_reference_index = timer_strict3_reference
            .find_object_index(timer_reference_reader)
            .expect("strict reference timer callback reader remains live");
        let timer_reference_value = timer_strict3_reference
            .call_object_function(
                timer_reference_index,
                "ReadTimerStrict3ReferenceValue",
                Vec::new(),
            )
            .expect("strict reference timer callback value reads");

        let mut call_pre_strict3 = Engine::new();
        register_real_c4_effect_definition(
            &mut call_pre_strict3,
            "ECHP",
            "EffectCall warning conversion host",
            r#"#strict 2
func Probe()
{
  var number = AddEffect("Oracle", this(), 100, 0, 0, ECCP);
  return(EffectCall(this(), number, "Probe", this()));
}
"#,
        );
        register_real_c4_effect_definition(
            &mut call_pre_strict3,
            "ECCP",
            "EffectCall warning conversion callback",
            r#"#strict 2
func FxOracleProbe(object target, int number, int declared_but_unused)
{
  var id_matches = GetID(target) == ECHP;
  var same_object = target == declared_but_unused;
  var type_is_object = GetType(target) == 4;
  GetNeededMatStr(target);
  SetXDir(17, target);
  return([id_matches, same_object, type_is_object]);
}
"#,
        );
        let call_pre_object = call_pre_strict3
            .spawn_object(SpawnConfig::new("ECHP"))
            .expect("pre-strict3 EffectCall carrier spawns");
        let call_pre_index = call_pre_strict3
            .find_object_index(call_pre_object)
            .expect("pre-strict3 EffectCall carrier exists");
        let call_pre_result = call_pre_strict3
            .call_object_function(call_pre_index, "Probe", Vec::new())
            .expect("pre-strict3 EffectCall warns and runs");

        let mut call_strict3 = Engine::new();
        register_real_c4_effect_definition(
            &mut call_strict3,
            "ECHS",
            "Strict EffectCall conversion host",
            r#"#strict 3
func Probe()
{
  var number = AddEffect("Oracle", this(), 100, 0, nil, ECCS);
  return(EffectCall(this(), number, "Probe", this()));
}
func Read()
{
  return(ReadEffectCallStrict3Value());
}
"#,
        );
        register_real_c4_effect_definition(
            &mut call_strict3,
            "ECCS",
            "Strict EffectCall conversion callback",
            r#"#strict 3
static callback_value;
func FxOracleProbe(object target, int number, int declared_but_unused)
{
  callback_value = 1;
  return(0);
}
global func ReadEffectCallStrict3Value() { return(callback_value); }
"#,
        );
        let call_strict_object = call_strict3
            .spawn_object(SpawnConfig::new("ECHS"))
            .expect("strict EffectCall carrier spawns");
        let call_strict_index = call_strict3
            .find_object_index(call_strict_object)
            .expect("strict EffectCall carrier exists");
        let call_strict_rejected = call_strict3
            .call_object_function(call_strict_index, "Probe", Vec::new())
            .is_err();
        let call_strict_reader = call_strict3
            .spawn_object(SpawnConfig::new("ECCS"))
            .expect("strict EffectCall callback reader spawns");
        let call_strict_index = call_strict3
            .find_object_index(call_strict_reader)
            .expect("strict EffectCall callback reader remains live");
        let call_strict_value = call_strict3
            .call_object_function(call_strict_index, "ReadEffectCallStrict3Value", Vec::new())
            .expect("strict EffectCall callback value reads");

        let mut call_strict3_reference = Engine::new();
        register_real_c4_effect_definition(
            &mut call_strict3_reference,
            "ECHR",
            "Strict EffectCall reference host",
            r#"#strict 3
func Probe()
{
  var number = AddEffect("Oracle", this(), 100, 0, nil, ECCR);
  return(EffectCall(this(), number, "Probe", this()));
}
func Read()
{
  return(ReadEffectCallStrict3ReferenceValue());
}
"#,
        );
        register_real_c4_effect_definition(
            &mut call_strict3_reference,
            "ECCR",
            "Strict EffectCall reference callback",
            r#"#strict 3
static callback_value;
func FxOracleProbe(object target, int number, int &declared_but_unused)
{
  SetXDir(17, target);
  callback_value = 1;
  return(0);
}
global func ReadEffectCallStrict3ReferenceValue() { return(callback_value); }
"#,
        );
        let call_reference_object = call_strict3_reference
            .spawn_object(SpawnConfig::new("ECHR"))
            .expect("strict reference EffectCall carrier spawns");
        let call_reference_index = call_strict3_reference
            .find_object_index(call_reference_object)
            .expect("strict reference EffectCall carrier exists");
        let call_reference_rejected = call_strict3_reference
            .call_object_function(call_reference_index, "Probe", Vec::new())
            .is_err();
        let call_reference_reader = call_strict3_reference
            .spawn_object(SpawnConfig::new("ECCR"))
            .expect("strict reference EffectCall callback reader spawns");
        let call_reference_index = call_strict3_reference
            .find_object_index(call_reference_reader)
            .expect("strict reference EffectCall callback reader remains live");
        let call_reference_value = call_strict3_reference
            .call_object_function(
                call_reference_index,
                "ReadEffectCallStrict3ReferenceValue",
                Vec::new(),
            )
            .expect("strict reference EffectCall callback value reads");

        let fields = [
            (
                "pre_strict3_callback_ran",
                i64::from(!matches!(&timer_pre_value, ScriptValue::Nil)),
            ),
            (
                "pre_strict3_original_object",
                i64::from(matches!(&timer_pre_value, ScriptValue::Bool(true))),
            ),
            (
                "strict3_rejected",
                i64::from(matches!(&timer_strict_value, ScriptValue::Nil)),
            ),
            (
                "strict3_callback_ran",
                i64::from(!matches!(&timer_strict_value, ScriptValue::Nil)),
            ),
            (
                "strict3_reference_rejected",
                i64::from(matches!(&timer_reference_value, ScriptValue::Nil)),
            ),
            (
                "strict3_reference_callback_ran",
                i64::from(!matches!(&timer_reference_value, ScriptValue::Nil)),
            ),
            (
                "strict3_reference_object_mutated",
                i64::from(raw_fixed_x(&timer_strict3_reference, timer_reference_object) != 0),
            ),
            (
                "effect_call_pre_strict3_callback_ran",
                i64::from(matches!(&call_pre_result, ScriptValue::Array(_))),
            ),
            (
                "effect_call_pre_strict3_type_is_object",
                i64::from(matches!(
                    &call_pre_result,
                    ScriptValue::Array(values)
                        if matches!(values.get(2), Some(ScriptValue::Bool(true)))
                )),
            ),
            (
                "effect_call_pre_strict3_identity_matches",
                i64::from(matches!(
                    &call_pre_result,
                    ScriptValue::Array(values)
                        if matches!(values.get(1), Some(ScriptValue::Bool(true)))
                )),
            ),
            (
                "effect_call_pre_strict3_id_matches",
                i64::from(matches!(
                    &call_pre_result,
                    ScriptValue::Array(values)
                        if matches!(values.first(), Some(ScriptValue::Bool(true)))
                )),
            ),
            (
                "effect_call_pre_strict3_target_equals_extra",
                i64::from(matches!(
                    &call_pre_result,
                    ScriptValue::Array(values)
                        if matches!(values.get(1), Some(ScriptValue::Bool(true)))
                )),
            ),
            (
                "effect_call_pre_strict3_object_mutated",
                i64::from(
                    raw_fixed_x(&call_pre_strict3, call_pre_object) == itofix_prec(17, 10).val(),
                ),
            ),
            (
                "effect_call_strict3_rejected",
                i64::from(call_strict_rejected),
            ),
            (
                "effect_call_strict3_callback_ran",
                i64::from(!matches!(&call_strict_value, ScriptValue::Nil)),
            ),
            (
                "effect_call_strict3_reference_rejected",
                i64::from(call_reference_rejected),
            ),
            (
                "effect_call_strict3_reference_callback_ran",
                i64::from(!matches!(&call_reference_value, ScriptValue::Nil)),
            ),
            (
                "effect_call_strict3_reference_object_mutated",
                i64::from(raw_fixed_x(&call_strict3_reference, call_reference_object) != 0),
            ),
        ];
        for (index, (field, rust)) in fields.into_iter().enumerate() {
            expect_eq(
                "effect_callback_conversion",
                index,
                field,
                i(section, field),
                rust,
            );
        }
    }

    // 11. C4Landscape::_PathFree (C4Landscape.cpp:890-915): PixCnt scans the
    //     authoritative Surface8 bytes. The second case is the minimized
    //     Goldrush frame-143 divergence: one water pixel at the right edge of
    //     a 17x15 cell must make the whole coarse cell occupied.
    for (idx, case) in golden["landscape_path"]
        .as_array()
        .unwrap()
        .iter()
        .enumerate()
    {
        let mut bytes = vec![0; 17 * 15];
        let pixel_x = i(case, "pixel_x") as i32;
        let pixel_y = i(case, "pixel_y") as i32;
        if pixel_x >= 0 && pixel_y >= 0 {
            bytes[pixel_y as usize * 17 + pixel_x as usize] = 1;
        }
        let mut densities = vec![0; 2];
        densities[1] = i(case, "density") as i32;
        let grid = PixelGrid::new(17, 15, bytes, densities, vec![None; 2], vec![None; 2]);
        let mut landscape = Landscape::flat(17, 15);
        landscape.set_pixel_grid(grid);
        expect_eq(
            "landscape_path",
            idx,
            "free",
            i(case, "free"),
            i64::from(landscape.path_free(0, 0, 16, 14, &crate::MaterialSet::new())),
        );
    }

    // 12. C4Object::ExecAction DFA_WALK + SetDir ordering
    //     (C4Object.cpp:4796-4826, 4249-4265, 4100-4187). This is the
    //     minimized Goldrush frame-170 WIPF case: Right ComDir accelerates a
    //     negative residual xdir to raw -19662, which rounds to zero but must
    //     still request Left, fire TurnAction and snap fix_x before movement.
    {
        let section = &golden["action_direction"];

        let (mut exec_action, id) = action_direction_engine();
        let idx = exec_action
            .find_object_index(id)
            .expect("oracle object exists");
        expect_eq(
            "action_direction",
            0,
            "returned_early",
            0,
            i64::from(
                exec_action
                    .apply_physics_at_index(idx)
                    .expect("action-direction physics applies"),
            ),
        );
        let object = &exec_action.objects[idx];
        expect_eq(
            "action_direction",
            0,
            "steered_xdir",
            i(section, "steered_xdir"),
            i64::from(object.fixed_velocity.x.val()),
        );
        expect_eq(
            "action_direction",
            0,
            "action_is_turn",
            i(section, "action_is_turn"),
            i64::from(object.state.action.name == "Turn"),
        );
        expect_eq(
            "action_direction",
            0,
            "direction",
            i(section, "direction"),
            i64::from(object.state.direction.to_script_value()),
        );
        expect_eq(
            "action_direction",
            0,
            "command_direction",
            i(section, "command_direction"),
            i64::from(object.state.command_direction.to_script_value()),
        );
        expect_eq(
            "action_direction",
            0,
            "fix_x_after_set_dir",
            i(section, "fix_x_after_set_dir"),
            i64::from(object.fixed_position.x.val()),
        );

        let (mut full_frame, id) = action_direction_engine();
        full_frame
            .tick_without_snapshot()
            .expect("oracle frame executes");
        let object = &full_frame.objects[full_frame
            .find_object_index(id)
            .expect("oracle object survives")];
        expect_eq(
            "action_direction",
            0,
            "action_phase",
            i(section, "action_phase"),
            i64::from(object.state.action.phase),
        );
        expect_eq(
            "action_direction",
            0,
            "action_time",
            i(section, "action_time"),
            i64::from(object.state.action.time),
        );
        expect_eq(
            "action_direction",
            0,
            "fix_x_after_move",
            i(section, "fix_x_after_move"),
            i64::from(object.fixed_position.x.val()),
        );
    }

    // 12b. Exact DFA_PUSH/PULL raw-xdir direction blocks and DFA_FIGHT's
    //      target-relative direction block (C4Object.cpp:5106-5108,
    //      5189-5192,5241-5243). These run through the full Rust executor so
    //      a later integer-velocity direction tail cannot mask the result.
    let procedure_direction_cases = golden["action_push_pull_fight_direction"]
        .as_array()
        .expect("procedure-direction golden is an array");
    let expected_procedure_direction_names = [
        "push_positive_subpixel",
        "pull_positive_subpixel",
        "fight_target_right_negative_velocity",
        "fight_equal_x_negative_velocity",
    ];
    assert_eq!(
        procedure_direction_cases.len(),
        expected_procedure_direction_names.len(),
        "procedure-direction golden must retain the complete extracted matrix"
    );
    for (case, expected_name) in procedure_direction_cases
        .iter()
        .zip(expected_procedure_direction_names)
    {
        assert_eq!(
            case["name"].as_str(),
            Some(expected_name),
            "procedure-direction golden row order/name drifted"
        );
    }
    for (idx, case) in procedure_direction_cases.iter().enumerate() {
        let name = case["name"]
            .as_str()
            .expect("procedure-direction case has a name");
        let (mut engine, actor_id) = action_push_pull_fight_direction_engine(case);
        let actor_idx = engine
            .find_object_index(actor_id)
            .expect("procedure-direction actor exists");
        let returned_early = engine
            .apply_physics_at_index(actor_idx)
            .expect("procedure-direction physics applies");
        assert!(
            !returned_early,
            "procedure-direction case `{name}` must reach the native phase tail"
        );
        let actor_idx = engine
            .find_object_index(actor_id)
            .expect("procedure-direction actor survives");
        let actor = &engine.objects[actor_idx];
        let turn_starts = match actor.state.local_vars.get("turn_starts") {
            Some(ScriptValue::Int(count)) => i64::from(*count),
            _ => 0,
        };
        let turn_start_dir = match actor.state.local_vars.get("turn_start_dir") {
            Some(ScriptValue::Int(direction)) => i64::from(*direction),
            _ => -1,
        };
        let action_is_turn = i64::from(actor.state.action.name == "Turn");
        // FlipDir=1 plus the deliberately cleared transform is controlled
        // instrumentation for zero-versus-one SetDir calls. The golden field
        // itself remains the C++ scaffold's explicit call count.
        let set_dir_call_probe = i64::from(actor.state.draw_transform.is_some());

        expect_eq(
            "action_push_pull_fight_direction",
            idx,
            "set_dir_calls",
            i(case, "set_dir_calls"),
            set_dir_call_probe,
        );
        expect_eq(
            "action_push_pull_fight_direction",
            idx,
            "runs_turn_action",
            i(case, "runs_turn_action"),
            action_is_turn,
        );
        expect_eq(
            "action_push_pull_fight_direction",
            idx,
            "turn_starts",
            i(case, "runs_turn_action"),
            turn_starts,
        );
        expect_eq(
            "action_push_pull_fight_direction",
            idx,
            "turn_start_dir",
            i(case, "turn_start_dir"),
            turn_start_dir,
        );
        expect_eq(
            "action_push_pull_fight_direction",
            idx,
            "direction",
            i(case, "direction"),
            i64::from(actor.state.direction.to_script_value()),
        );
        if matches!(name, "push_positive_subpixel" | "pull_positive_subpixel") {
            expect_eq(
                "action_push_pull_fight_direction",
                idx,
                "xdir_raw",
                i(case, "xdir_raw"),
                i64::from(actor.fixed_velocity.x.val()),
            );
            expect_eq(
                "action_push_pull_fight_direction",
                idx,
                "xdir_pixel",
                i(case, "xdir_pixel"),
                i64::from(actor.state.velocity.x),
            );
        }
    }

    // 13. C4Object::ExecAction DFA_SWIM + SetDir ordering
    //     (C4Object.cpp:4946-4984, 4235-4254, 4168-4169). This is the
    //     minimized Goldrush frame-219 FISH case: Left ComDir creates a raw
    //     negative xdir, which must fire Swim.TurnAction and snap both fixed
    //     coordinates before movement while stale Swim advances Turn's phase.
    {
        let section = &golden["action_swim_direction"];

        let (mut exec_action, id) = swim_action_direction_engine();
        let idx = exec_action
            .find_object_index(id)
            .expect("oracle swimmer exists");
        expect_eq(
            "action_swim_direction",
            0,
            "returned_early",
            0,
            i64::from(
                exec_action
                    .apply_physics_at_index(idx)
                    .expect("swim-direction physics applies"),
            ),
        );
        let object = &exec_action.objects[idx];
        expect_eq(
            "action_swim_direction",
            0,
            "steered_xdir",
            i(section, "steered_xdir"),
            i64::from(object.fixed_velocity.x.val()),
        );
        expect_eq(
            "action_swim_direction",
            0,
            "steered_ydir",
            i(section, "steered_ydir"),
            i64::from(object.fixed_velocity.y.val()),
        );
        expect_eq(
            "action_swim_direction",
            0,
            "action_is_turn",
            i(section, "action_is_turn"),
            i64::from(object.state.action.name == "Turn"),
        );
        expect_eq(
            "action_swim_direction",
            0,
            "direction",
            i(section, "direction"),
            i64::from(object.state.direction.to_script_value()),
        );
        expect_eq(
            "action_swim_direction",
            0,
            "command_direction",
            i(section, "command_direction"),
            i64::from(object.state.command_direction.to_script_value()),
        );
        expect_eq(
            "action_swim_direction",
            0,
            "fix_x_after_set_dir",
            i(section, "fix_x_after_set_dir"),
            i64::from(object.fixed_position.x.val()),
        );
        expect_eq(
            "action_swim_direction",
            0,
            "fix_y_after_set_dir",
            i(section, "fix_y_after_set_dir"),
            i64::from(object.fixed_position.y.val()),
        );

        let (mut full_frame, id) = swim_action_direction_engine();
        full_frame
            .tick_without_snapshot()
            .expect("oracle frame executes");
        let object = &full_frame.objects[full_frame
            .find_object_index(id)
            .expect("oracle swimmer survives")];
        expect_eq(
            "action_swim_direction",
            0,
            "action_phase",
            i(section, "action_phase"),
            i64::from(object.state.action.phase),
        );
        expect_eq(
            "action_swim_direction",
            0,
            "action_time",
            i(section, "action_time"),
            i64::from(object.state.action.time),
        );
        expect_eq(
            "action_swim_direction",
            0,
            "fix_x_after_move",
            i(section, "fix_x_after_move"),
            i64::from(object.fixed_position.x.val()),
        );
        expect_eq(
            "action_swim_direction",
            0,
            "fix_y_after_move",
            i(section, "fix_y_after_move"),
            i64::from(object.fixed_position.y.val()),
        );
    }

    // 14. C4Object::SetAction callback dispatch (C4Object.cpp:4172-4208).
    //     Minimized from Goldrush frame 192, WIPF #565: script SetAction
    //     synchronously fires the new StartCall exactly once and before the
    //     old AbortCall; natural phase wraps likewise fire Start before End.
    for (idx, case) in golden["action_callbacks"]
        .as_array()
        .expect("action_callbacks is an array")
        .iter()
        .enumerate()
    {
        let name = case["name"]
            .as_str()
            .expect("action_callbacks case has a name");
        let (mut engine, id) = action_callbacks_engine(name);
        engine
            .tick_without_snapshot()
            .expect("callback fixture frame executes");
        expect_eq(
            "action_callbacks",
            idx,
            "completed",
            i(case, "completed"),
            i64::from(engine.find_object_index(id).is_some()),
        );
        expect_eq(
            "action_callbacks",
            idx,
            "callback_order",
            i(case, "callback_order"),
            action_callback_local(&engine, id, "callbackOrder"),
        );
        expect_eq(
            "action_callbacks",
            idx,
            "start_count",
            i(case, "start_count"),
            action_callback_local(&engine, id, "startCount"),
        );
        expect_eq(
            "action_callbacks",
            idx,
            "old_count",
            i(case, "old_count"),
            action_callback_local(&engine, id, "oldCount"),
        );
    }

    // 14b. C4Object.cpp DFA_CONNECT missing-target branch (5368-5376 in the
    //      pinned oracle): LineBreak(true) runs before AssignRemoval, whose
    //      Destruction callback runs while the line is still live. Call the
    //      real Engine procedure directly so its deleted object's callback
    //      locals remain observable before end-of-frame tombstone cleanup.
    expect_connect_removal_case(&golden, "connect_missing_target_removal", false);

    // 14c. The later geometry-break branch (pinned C4Object.cpp:5435-5441)
    //      calls LineBreak with no argument before the same AssignRemoval.
    //      A one-vertex line makes real C4Shape::LineConnect fail its pinned
    //      C4Shape.cpp:275 guard in both oracle and Rust fixtures.
    expect_connect_removal_case(&golden, "connect_geometry_break_removal", true);

    // 15. C4SolidMask constructor bitmap selection (C4SolidMask.cpp:400-412,
    //     C4Object.cpp:5908-5923). Minimized from Goldrush frame 184, CTWR
    //     #1351: source pixel (219,86) is transparent in Graphics.png but
    //     opaque in Graphics2.png. SetGraphics selects Graphics2 and rebuilds
    //     the put solid mask immediately.
    {
        let cases = golden["solid_mask_graphics"]
            .as_array()
            .expect("solid_mask_graphics is an array");
        let (mut engine, id) = solid_mask_graphics_engine();
        let vehicle = engine
            .landscape()
            .and_then(Landscape::grid_vehicle_byte)
            .expect("vehicle material exists");

        for (idx, case) in cases.iter().enumerate() {
            if i(case, "selected_variant") != 0 {
                let mut update = ObjectUpdate::new();
                update.base_graphics = Some(Some(ObjectBaseGraphics {
                    definition: "CTWR".to_string(),
                    graphics_name: Some("2".to_string()),
                    blit_mode: 0,
                }));
                engine
                    .apply_object_update(id, update)
                    .expect("SetGraphics update applies");
            }

            let object =
                &engine.objects[engine.find_object_index(id).expect("tower object survives")];
            let active_variant = object
                .state
                .base_graphics
                .as_ref()
                .and_then(|graphics| graphics.graphics_name.as_deref())
                == Some("2");
            expect_eq(
                "solid_mask_graphics",
                idx,
                "active_variant",
                i(case, "active_variant"),
                i64::from(active_variant),
            );
            expect_eq(
                "solid_mask_graphics",
                idx,
                "source_x",
                i(case, "source_x"),
                219,
            );
            expect_eq(
                "solid_mask_graphics",
                idx,
                "source_y",
                i(case, "source_y"),
                86,
            );
            let mask_pixel = engine
                .landscape()
                .and_then(|landscape| landscape.grid_byte_at(1, 1))
                .map_or(0, |pixel| if pixel == vehicle { 0xff } else { 0x00 });
            expect_eq(
                "solid_mask_graphics",
                idx,
                "mask_pixel",
                i(case, "mask_pixel"),
                mask_pixel,
            );
        }
    }

    // 15b. DefCore Scale -> Picture facet rect (C4Def.cpp:745 percent->float,
    //      C4Def.cpp:1341 Picture2Facet, C4Rect.cpp:37-44 Scaled). The Picture
    //      rect is authored in GAME units; the phase offset is composed there
    //      and only the resulting rect is scaled into bitmap space, so the
    //      truncation applies to the already-offset x. This is the contract any
    //      HD (Scale != 100) content depends on.
    for (idx, case) in golden["def_picture_scale"]
        .as_array()
        .expect("def_picture_scale is an array")
        .iter()
        .enumerate()
    {
        let engine = def_picture_scale_engine(
            u(case, "scale_percent") as u32,
            DefinitionPicture {
                x: i(case, "picture_x") as i32,
                y: i(case, "picture_y") as i32,
                width: i(case, "picture_wdt") as i32,
                height: i(case, "picture_hgt") as i32,
            },
        );
        let image = engine
            .definition_picture_phase_image("PSCL", i(case, "phase") as i32)
            .expect("scaled picture facet");
        expect_eq(
            "def_picture_scale",
            idx,
            "wdt",
            i(case, "wdt"),
            i64::from(image.width()),
        );
        expect_eq(
            "def_picture_scale",
            idx,
            "hgt",
            i(case, "hgt"),
            i64::from(image.height()),
        );
        // R/G of the first pixel are the source coordinates the crop started at.
        let pixels = image.pixels();
        expect_eq(
            "def_picture_scale",
            idx,
            "x",
            i(case, "x"),
            i64::from(pixels[0]),
        );
        expect_eq(
            "def_picture_scale",
            idx,
            "y",
            i(case, "y"),
            i64::from(pixels[1]),
        );
    }

    // 16. Movement: per-frame sub-pixel accumulation (the Theme-C core).
    //    fix_x += xdir; fix_y += (ydir += gravity); matching C4Movement.cpp.
    for scn in golden["movement"].as_array().unwrap() {
        let name = scn["name"].as_str().unwrap_or("?");
        let mut fix_x = itofix(0);
        let mut fix_y = itofix(0);
        let xdir = C4Fixed::from_raw(i(scn, "xdir") as i32);
        let mut ydir = C4Fixed::from_raw(i(scn, "ydir0") as i32);
        let grav = C4Fixed::from_raw(i(scn, "grav") as i32);
        for (frame, fr) in scn["frames"].as_array().unwrap().iter().enumerate() {
            ydir += grav;
            fix_x += xdir;
            fix_y += ydir;
            let label = format!("movement[{name}]");
            expect_eq(&label, frame, "fix_x", i(fr, "fix_x"), fix_x.val() as i64);
            expect_eq(&label, frame, "fix_y", i(fr, "fix_y"), fix_y.val() as i64);
            expect_eq(&label, frame, "xdir", i(fr, "xdir"), xdir.val() as i64);
            expect_eq(&label, frame, "ydir", i(fr, "ydir"), ydir.val() as i64);
            expect_eq(&label, frame, "x", i(fr, "x"), fixtoi(fix_x) as i64);
            expect_eq(&label, frame, "y", i(fr, "y"), fixtoi(fix_y) as i64);
        }
    }

    // 16b. pxs_execute: the per-tick PXS step itself (C4PXS.cpp:28-135), which
    //      `movement` above deliberately excludes and `pxs_allocation` does not
    //      reach. This drives the port's real `execute_pxs` rather than a
    //      re-implementation, and compares raw C4Fixed alongside the RNG
    //      ledger — a wrong draw count shows up even when the position agrees.
    for scn in golden["pxs_execute"].as_array().unwrap() {
        let name = scn["name"].as_str().unwrap_or("?");
        let label = format!("pxs_execute[{name}]");

        // Densities and WindDrift mirror the oracle's material map exactly.
        // MaterialId is the 0-based library index (MaterialSet::from_resource_
        // library), and C++'s Map[0] is a real entry, so the leading Vacuum
        // keeps Earth at 1 and Water at 2 on both sides.
        let library = clonk_resources::MaterialLibrary::parse(
            r#"
            [Material Vacuum]
            Name=Vacuum
            Density=0

            [Material Earth]
            Name=Earth
            Density=50

            [Material Water]
            Name=Water
            Density=25
            WindDrift=40
            "#,
        )
        .expect("pxs execute oracle materials parse");

        const WDT: u32 = 16;
        const HGT: u32 = 12;
        const EARTH_BYTE: u8 = 1;
        let bytes = vec![0u8; WDT as usize * HGT as usize];
        let mut densities = vec![0; 128];
        densities[EARTH_BYTE as usize] = 50;
        let mut material_names = vec![None; 128];
        material_names[EARTH_BYTE as usize] = Some("Earth".to_string());
        let grid = PixelGrid::new(WDT, HGT, bytes, densities, material_names, vec![None; 128]);

        let mut engine = Engine::with_seed(0);
        engine.configure_materials_from_library(&library);
        // `gravity_as_c4fixed` is `fixed100(gravity) / 5`, so 100 yields the
        // oracle's `Gravity = FIXED100(20)` (raw 13107).
        engine.set_physics(PhysicsSettings::new(100, 1000, -1000));
        let mut landscape = Landscape::flat(WDT, HGT as i32);
        landscape.set_pixel_grid(grid);
        // `estimated_height` is the port's GBackHgt, but it only answers the
        // real height once pinned; otherwise it estimates from surface depths,
        // which an empty grid reports as zero and the bounds check then reads
        // as out-of-world.
        landscape.set_world_height(HGT as i32);
        engine.set_landscape(landscape);
        engine.rng = LcgRng::new(i(scn, "seed") as u32);

        let mut pixel = crate::pxs::Pxs {
            mat: crate::pxs::PxsMaterial::from_raw(i(scn, "mat") as i32),
            x: C4Fixed::from_raw(i(scn, "x0") as i32),
            y: C4Fixed::from_raw(i(scn, "y0") as i32),
            xdir: C4Fixed::from_raw(i(scn, "xdir0") as i32),
            ydir: C4Fixed::from_raw(i(scn, "ydir0") as i32),
        };
        let mut deactivated = false;

        for (frame, fr) in scn["frames"].as_array().unwrap().iter().enumerate() {
            if !deactivated {
                match engine.execute_pxs(pixel) {
                    Some(next) => pixel = next,
                    None => deactivated = true,
                }
            }
            expect_eq(&label, frame, "x", i(fr, "x"), pixel.x.val() as i64);
            expect_eq(&label, frame, "y", i(fr, "y"), pixel.y.val() as i64);
            expect_eq(
                &label,
                frame,
                "xdir",
                i(fr, "xdir"),
                pixel.xdir.val() as i64,
            );
            expect_eq(
                &label,
                frame,
                "ydir",
                i(fr, "ydir"),
                pixel.ydir.val() as i64,
            );
            expect_eq(
                &label,
                frame,
                "deactivated",
                i64::from(fr["deactivated"].as_bool().unwrap_or(false)),
                i64::from(deactivated),
            );
            expect_eq(
                &label,
                frame,
                "mat",
                i(fr, "mat"),
                if deactivated {
                    -1
                } else {
                    pixel.mat.raw() as i64
                },
            );
            expect_rng_state_at(&label, frame, fr, &engine.rng);
        }
    }

    // 16b2. Full PXS lifecycle: the real system walk around C4PXS::Execute.
    //
    // The single-pixel section above pins C4PXS.cpp:28-137 in isolation. This
    // sequence additionally lifts C4PXSSystem::Execute (C4PXS.cpp:218-240),
    // Deactivate/Delete (:139-149, :426-437), Create's free-slot reuse
    // (:181-216), and Synchronize (:401-404). Four occupied slots make the
    // ascending execution order visible in the two splash particles' distinct
    // raw xdir values; one custom Insert dies into landscape, while a masked
    // reaction exists but deliberately continues through the contact loop.
    {
        let lifecycle = golden
            .get("pxs_lifecycle")
            .and_then(Value::as_object)
            .expect("pxs_lifecycle is an object");
        let steps = lifecycle["steps"]
            .as_array()
            .expect("pxs_lifecycle.steps is an array");
        let step_named = |name: &str| {
            steps
                .iter()
                .find(|step| step["step"].as_str() == Some(name))
                .unwrap_or_else(|| panic!("pxs_lifecycle golden is missing step `{name}`"))
        };

        let library = MaterialLibrary::parse(
            r#"
            [Material]
            Name=Vacuum
            Density=0

            [Material]
            Name=Earth
            Density=50

            [Material]
            Name=InsertWater
            Density=25
            WindDrift=20
            SplashRate=0
            MaxSlide=0

            [Reaction]
            Type=Insert
            TargetSpec=Earth
            CheckSlide=0

            [Material]
            Name=SplashWater
            Density=25
            WindDrift=20
            SplashRate=1
            MaxSlide=0

            [Material]
            Name=MaskedWater
            Density=25
            WindDrift=20
            SplashRate=0
            MaxSlide=0

            [Reaction]
            Type=Insert
            TargetSpec=Earth
            CheckSlide=0
            ExecMask=1
            "#,
        )
        .expect("PXS lifecycle materials parse");

        const WDT: u32 = 16;
        const HGT: u32 = 12;
        const EARTH_BYTE: u8 = 1;
        let mut bytes = vec![0_u8; WDT as usize * HGT as usize];
        for x in 0..WDT as usize {
            bytes[10 * WDT as usize + x] = EARTH_BYTE;
        }
        let mut densities = vec![0; 128];
        densities[1] = 50;
        densities[2] = 25;
        densities[3] = 25;
        densities[4] = 25;
        let mut material_names = vec![None; 128];
        material_names[1] = Some("Earth".to_string());
        material_names[2] = Some("InsertWater".to_string());
        material_names[3] = Some("SplashWater".to_string());
        material_names[4] = Some("MaskedWater".to_string());
        let grid = PixelGrid::new(WDT, HGT, bytes, densities, material_names, vec![None; 128]);

        let mut engine = Engine::with_seed(0);
        engine.configure_materials_from_library(&library);
        engine.set_physics(PhysicsSettings::new(100, 1000, -1000));
        let mut landscape = Landscape::flat(WDT, HGT as i32);
        landscape.set_pixel_grid(grid);
        landscape.set_world_height(HGT as i32);
        engine.set_landscape(landscape);
        engine.rng = LcgRng::new(lifecycle["seed"].as_i64().unwrap_or(0) as u32);
        engine.rng.randomize3();

        let initial_landscape = landscape_material_snapshot(&engine, WDT, HGT);
        let insert = crate::material::MaterialId::new(2).expect("InsertWater id");
        let splash = crate::material::MaterialId::new(3).expect("SplashWater id");
        let masked = crate::material::MaterialId::new(4).expect("MaskedWater id");
        for (mat, x) in [(insert, 2), (splash, 6), (splash, 10), (masked, 13)] {
            assert!(engine.create_pxs(mat, itofix(x), itofix(9), C4Fixed::ZERO, itofix(1),));
        }

        let check_step = |label: &str, step: &Value, engine: &Engine| {
            expect_eq(
                label,
                0,
                "execute_count",
                i(step, "execute_count"),
                engine.pxs_system.execute_count() as i64,
            );
            expect_eq(
                label,
                0,
                "live",
                i(step, "live"),
                engine.pxs_system.count() as i64,
            );
            expect_eq(
                label,
                0,
                "chunk0_allocated",
                i64::from(step["chunk0_allocated"].as_bool().unwrap_or(false)),
                i64::from(engine.pxs_system.chunk_allocated(0)),
            );
            let chunk_count = (0..crate::pxs::PXS_CHUNK_SIZE)
                .filter(|slot| engine.pxs_system.peek_slot(0, *slot).is_some())
                .count();
            expect_eq(
                label,
                0,
                "chunk0_count",
                i(step, "chunk0_count"),
                chunk_count as i64,
            );

            for expected in step["slots"].as_array().expect("pxs_lifecycle step slots") {
                let slot = i(expected, "slot") as usize;
                let actual = engine.pxs_system.peek_slot(0, slot);
                expect_eq(
                    label,
                    slot,
                    "mat",
                    i(expected, "mat"),
                    actual.map_or(-1, |pixel| pixel.mat.raw() as i64),
                );
                if let Some(pixel) = actual {
                    for (field, cpp, rust) in [
                        ("x", i(expected, "x"), i64::from(pixel.x.val())),
                        ("y", i(expected, "y"), i64::from(pixel.y.val())),
                        ("xdir", i(expected, "xdir"), i64::from(pixel.xdir.val())),
                        ("ydir", i(expected, "ydir"), i64::from(pixel.ydir.val())),
                    ] {
                        expect_eq(label, slot, field, cpp, rust);
                    }
                }
            }

            let changes = landscape_material_changes(&initial_landscape, engine, WDT, HGT);
            let expected_insertions = step["insertions"]
                .as_array()
                .expect("pxs_lifecycle step insertions");
            expect_eq(
                label,
                0,
                "insertions",
                expected_insertions.len() as i64,
                changes.len() as i64,
            );
            for (index, (expected, actual)) in
                expected_insertions.iter().zip(changes.iter()).enumerate()
            {
                let (x, y, before, after) = *actual;
                expect_eq(label, index, "insert_x", i(expected, "x"), i64::from(x));
                expect_eq(label, index, "insert_y", i(expected, "y"), i64::from(y));
                expect_eq(
                    label,
                    index,
                    "insert_before",
                    -1,
                    before.map_or(-1, |material| material.index() as i64),
                );
                expect_eq(
                    label,
                    index,
                    "insert_mat",
                    i(expected, "mat"),
                    after.map_or(-1, |material| material.index() as i64),
                );
            }

            expect_rng_state(label, step, &engine.rng);
            expect_eq(
                label,
                0,
                "rnd3_ptr",
                i(step, "rnd3_ptr"),
                i64::from(engine.rng.rnd3_ptr()),
            );

            // The oracle mechanically executes C4PXSSystem::Save
            // (C4PXS.cpp:324-360). Its full-component hash catches tag,
            // allocated-chunk ordering, every slot, and retained dead payload;
            // decoded leading slots make a byte divergence readable. Compare
            // this separately from ordinary live-slot equality: dead fields
            // are serialization state, never execution state (Execute gates
            // on Mat != MNone at :233-238).
            let expected_saved = step["saved_slots"]
                .as_array()
                .expect("pxs_lifecycle saved slots");
            let component = engine.pxs_system.to_c4b();
            expect_eq(
                label,
                0,
                "save_ok",
                i64::from(step["save_ok"].as_bool().expect("save_ok is boolean")),
                1,
            );
            expect_eq(
                label,
                0,
                "save_present",
                i64::from(
                    step["save_present"]
                        .as_bool()
                        .expect("save_present is boolean"),
                ),
                i64::from(component.is_some()),
            );
            expect_eq(
                label,
                0,
                "save_len",
                i(step, "save_len"),
                component.as_ref().map_or(0, Vec::len) as i64,
            );
            let save_tag = component
                .as_deref()
                .and_then(|bytes| bytes.get(..4))
                .map(|bytes| {
                    i32::from_le_bytes(bytes.try_into().expect("four-byte PXS format tag"))
                })
                .unwrap_or(-1);
            expect_eq(
                label,
                0,
                "save_tag",
                i(step, "save_tag"),
                i64::from(save_tag),
            );
            let save_hash = component
                .as_deref()
                .unwrap_or_default()
                .iter()
                .fold(14_695_981_039_346_656_037_u64, |hash, byte| {
                    (hash ^ u64::from(*byte)).wrapping_mul(1_099_511_628_211)
                });
            expect_eq_u64(label, 0, "save_hash", u(step, "save_hash"), save_hash);
            for expected in expected_saved {
                let component = component
                    .as_ref()
                    .expect("saved slots require a PXS.c4b component");
                let slot = i(expected, "slot") as usize;
                let offset = 4 + slot * 20;
                for (field, key) in ["mat", "x", "y", "xdir", "ydir"].into_iter().enumerate() {
                    let raw = i32::from_le_bytes(
                        component[offset + field * 4..offset + field * 4 + 4]
                            .try_into()
                            .expect("one serialized PXS field"),
                    );
                    expect_eq(label, slot, key, i(expected, key), i64::from(raw));
                }
            }
        };

        engine.tick_pxs();
        check_step(
            "pxs_lifecycle[after_first_execute]",
            step_named("after_first_execute"),
            &engine,
        );

        engine.pxs_system.synchronize();
        check_step(
            "pxs_lifecycle[after_synchronize]",
            step_named("after_synchronize"),
            &engine,
        );

        assert!(engine.create_pxs(insert, itofix(4), itofix(9), C4Fixed::ZERO, itofix(1),));
        check_step(
            "pxs_lifecycle[after_reuse]",
            step_named("after_reuse"),
            &engine,
        );

        engine.tick_pxs();
        check_step(
            "pxs_lifecycle[after_second_execute]",
            step_named("after_second_execute"),
            &engine,
        );

        // Empty chunks are retained by the pass in which their final PXS
        // dies, then deleted at the head of the following system Execute
        // (C4PXS.cpp:218-240). Reset only the fixture world/system; the
        // synchronized RNG continues from the mixed-slot sequence above.
        engine.pxs_system.clear();
        let mut bytes = vec![0_u8; WDT as usize * HGT as usize];
        for x in 0..WDT as usize {
            bytes[10 * WDT as usize + x] = EARTH_BYTE;
        }
        let mut densities = vec![0; 128];
        densities[1] = 50;
        densities[2] = 25;
        densities[3] = 25;
        densities[4] = 25;
        let mut material_names = vec![None; 128];
        material_names[1] = Some("Earth".to_string());
        material_names[2] = Some("InsertWater".to_string());
        material_names[3] = Some("SplashWater".to_string());
        material_names[4] = Some("MaskedWater".to_string());
        let grid = PixelGrid::new(WDT, HGT, bytes, densities, material_names, vec![None; 128]);
        let mut landscape = Landscape::flat(WDT, HGT as i32);
        landscape.set_pixel_grid(grid);
        landscape.set_world_height(HGT as i32);
        engine.set_landscape(landscape);
        assert!(engine.create_pxs(insert, itofix(8), itofix(9), C4Fixed::ZERO, itofix(1),));

        engine.tick_pxs();
        check_step(
            "pxs_lifecycle[cleanup_after_death]",
            step_named("cleanup_after_death"),
            &engine,
        );
        engine.tick_pxs();
        check_step(
            "pxs_lifecycle[cleanup_after_following_execute]",
            step_named("cleanup_after_following_execute"),
            &engine,
        );

        // The real system frees an empty allocation only when the outer loop
        // reaches that chunk. A synchronous insertion from full chunk 0 may
        // therefore revive empty chunk 1 first; its untouched tombstones must
        // survive into Save (C4PXS.cpp:218-240,324-349).
        let revival = &lifecycle["chunk_revival"];
        let revival_library = MaterialLibrary::parse(
            "[Material Water]\nName=Water\nDensity=25\nFriction=0\nSplashRate=0\nMaxSlide=10\n",
        )
        .expect("chunk-revival material parses");
        let revival_materials = crate::MaterialSet::from_resource_library(&revival_library);
        let water = revival_materials.id_of("Water").expect("Water material id");
        let mut revival_engine = Engine::with_seed(3);
        revival_engine.set_materials(revival_materials);
        let mut bytes = vec![0_u8; 12 * 12];
        for y in 6..12 {
            for x in 0..=6 {
                bytes[y * 12 + x] = 20;
            }
        }
        let mut densities = vec![0; 128];
        densities[20] = 25;
        let mut material_names = vec![None; 128];
        material_names[20] = Some("Water".to_string());
        let grid = PixelGrid::new(12, 12, bytes, densities, material_names, vec![None; 128]);
        let mut landscape = Landscape::with_default_material(12, vec![6; 12], Some(water))
            .expect("chunk-revival landscape");
        landscape.set_world_height(12);
        landscape.set_pixel_grid(grid);
        revival_engine.set_landscape(landscape);

        let dead = crate::pxs::Pxs {
            mat: water.into(),
            x: C4Fixed::from_raw(0x1122_3344),
            y: C4Fixed::from_raw(-17),
            xdir: C4Fixed::from_raw(0x5566_7788),
            ydir: C4Fixed::from_raw(i32::MIN + 31),
        };
        assert!(revival_engine.pxs_system.create_at(1, 7, dead));
        revival_engine.pxs_system.clear_slot(1, 7);
        assert!(revival_engine.create_pxs(water, itofix(3), itofix(7), C4Fixed::ZERO, itofix(1),));
        for slot in 1..crate::pxs::PXS_CHUNK_SIZE {
            assert!(revival_engine.pxs_system.create_at(
                0,
                slot,
                crate::pxs::Pxs {
                    mat: water.into(),
                    x: itofix(10),
                    y: itofix(2),
                    xdir: C4Fixed::ZERO,
                    ydir: C4Fixed::ZERO,
                },
            ));
        }

        revival_engine.tick_pxs();
        let component = revival_engine.pxs_system.to_c4b();
        expect_eq(
            "pxs_lifecycle[chunk_revival]",
            0,
            "execute_count",
            i(revival, "execute_count"),
            revival_engine.pxs_system.execute_count() as i64,
        );
        expect_eq(
            "pxs_lifecycle[chunk_revival]",
            0,
            "chunk1_allocated",
            i64::from(revival["chunk1_allocated"].as_bool().unwrap_or(false)),
            i64::from(revival_engine.pxs_system.chunk_allocated(1)),
        );
        expect_eq(
            "pxs_lifecycle[chunk_revival]",
            0,
            "save_ok",
            i64::from(revival["save_ok"].as_bool().unwrap_or(false)),
            1,
        );
        expect_eq(
            "pxs_lifecycle[chunk_revival]",
            0,
            "save_present",
            i64::from(revival["save_present"].as_bool().unwrap_or(false)),
            i64::from(component.is_some()),
        );
        expect_eq(
            "pxs_lifecycle[chunk_revival]",
            0,
            "save_len",
            i(revival, "save_len"),
            component.as_ref().map_or(0, Vec::len) as i64,
        );
        let component = component.expect("revived chunk has a saved component");
        let record = 4 + (crate::pxs::PXS_CHUNK_SIZE + 7) * 20;
        for (field, key) in ["mat", "x", "y", "xdir", "ydir"].into_iter().enumerate() {
            let raw = i32::from_le_bytes(
                component[record + field * 4..record + field * 4 + 4]
                    .try_into()
                    .expect("one revived tombstone field"),
            );
            expect_eq(
                "pxs_lifecycle[chunk_revival]",
                7,
                key,
                i(&revival["saved_slot7"], key),
                i64::from(raw),
            );
        }
    }

    // 16b9. ContactVtxCNAT/Weight/Friction (C4Movement.cpp:58-95), the
    //        ordered vertex helpers the per-pixel collision response reads.
    //        Weight deliberately differs from friction: a contacted centre
    //        vertex has weight zero, so C++ continues to the first later
    //        contacted vertex whose x is non-zero; friction always returns
    //        the first contacted slot.
    for (index, case) in golden["contact_vtx_helpers"]
        .as_array()
        .expect("contact_vtx_helpers is an array")
        .iter()
        .enumerate()
    {
        let vertices = case["vertices"]
            .as_array()
            .expect("contact_vtx_helpers.vertices is an array");
        let mut definition =
            Definition::from_script("CVTX", "Contact vertex helper oracle", "#strict\n")
                .expect("contact vertex helper oracle compiles");
        definition.set_shape_vertices(
            vertices
                .iter()
                .map(|vertex| {
                    crate::ObjectVertex::new(i(vertex, "x") as i32, 0)
                        .with_friction(i(vertex, "friction") as i32)
                })
                .collect(),
        );

        let mut engine = Engine::with_seed(0);
        engine
            .register_definition(definition)
            .expect("contact vertex helper oracle registers");
        let object_id = engine
            .spawn_object(SpawnConfig::new("CVTX"))
            .expect("contact vertex helper oracle spawns");
        let object_index = engine
            .find_object_index(object_id)
            .expect("contact vertex helper oracle object exists");
        engine.objects[object_index].frame_vertex_contacts = vertices
            .iter()
            .map(|vertex| i(vertex, "contact") as u32)
            .collect();

        let object = &engine.objects[object_index];
        for (field, actual) in [
            ("weight", i64::from(object.live_contact_first_weight())),
            ("friction", i64::from(object.live_contact_first_friction())),
            (
                "has_left",
                i64::from(object.live_contact_has_vertex_cnat(crate::CNAT_LEFT)),
            ),
            (
                "has_right",
                i64::from(object.live_contact_has_vertex_cnat(crate::CNAT_RIGHT)),
            ),
        ] {
            expect_eq("contact_vtx_helpers", index, field, i(case, field), actual);
        }
    }

    // 16b10. do_movement: the unattached half of `C4Object::DoMovement`
    //       (C4Movement.cpp:254-322) — the per-pixel collision loop.
    //
    //       Each axis accumulates its fixed target, clamps it through
    //       Side/VerticalBounds, and then walks ONE PIXEL AT A TIME with a
    //       ContactCheck per step. Three properties are pinned:
    //
    //       * on contact the loop rewrites the fixed coordinate back to the
    //         whole pixel (`fix_x = itofix(x)`), DISCARDING the sub-pixel
    //         remainder — invisible to `fixtoi()` and exactly the "stops one
    //         subpixel earlier" desync the issue names, which is why the raw
    //         `C4Fixed` is compared here and not the whole coordinate;
    //       * the axes respond ASYMMETRICALLY. A horizontal contact redirects
    //         xdir into ydir and rubs friction off *ydir*; a vertical contact
    //         rubs friction off *xdir* first, then picks from the contact
    //         vertices' CNAT — slide left, else slide right, else bleed ydir
    //         into rdir (rotatable, non-living, single contact only), else zero
    //         ydir;
    //       * horizontal runs to completion BEFORE vertical begins, so a
    //         diagonal move is two independent walks rather than one.
    //
    //       The fixture drives `exec_object_movement`, the port's `DoMovement`,
    //       with an idle action and no script — matching the oracle's
    //       `Action.Act = ActIdle`, `t_attach = 0` object, so the arms this
    //       section does not cover (DigFree, ContactAction, the Hit callbacks)
    //       are inert on both sides rather than silently differing.
    for case in golden["do_movement"].as_array().unwrap() {
        let name = case["name"].as_str().unwrap_or("?");
        let label = format!("do_movement[{name}]");

        const WDT: u32 = 24;
        const HGT: i32 = 16;
        const SOLID: u8 = 1;

        let floor_y = i(case, "floor_y") as i32;
        let wall_x = i(case, "wall_x") as i32;

        let mut definition = Definition::from_script("MOVP", "Mover", "#strict\n")
            .expect("oracle movement fixture compiles");
        // The same three vertices the oracle fixture carries: one bottom, one
        // left, one right, so either axis can contact independently.
        // ContactVtxFriction returns the FIRST contacted vertex's value
        // (C4Movement.cpp:89-97), so the per-vertex friction is part of the
        // fixture on both sides — without it the friction arms are vacuous.
        let friction = i(case, "vtx_friction") as i32;
        let vertex = |x: i32, y: i32, cnat: u32| crate::ObjectVertex {
            friction,
            ..crate::ObjectVertex::new(x, y).with_cnat(cnat)
        };
        definition.set_shape_vertices(vec![
            vertex(0, 1, crate::CNAT_BOTTOM),
            vertex(-1, 0, crate::CNAT_LEFT),
            vertex(1, 0, crate::CNAT_RIGHT),
        ]);
        definition.set_shape_rect(Some(crate::DefinitionRect::new(-1, -1, 3, 3)));
        // DefCore defaults this to 0 (C4Def.cpp:162,384), so the landscape
        // clamp only runs where a case asks for it.
        definition.set_border_bound(i(case, "border_bound") as i32);

        let mut engine = Engine::with_seed(0);
        let mut bytes = vec![0u8; WDT as usize * HGT as usize];
        if floor_y >= 0 {
            for gy in floor_y..HGT {
                for gx in 0..WDT as usize {
                    bytes[gy as usize * WDT as usize + gx] = SOLID;
                }
            }
        }
        if wall_x >= 0 {
            for gy in 0..HGT {
                bytes[gy as usize * WDT as usize + wall_x as usize] = SOLID;
            }
        }
        let mut landscape = Landscape::flat(WDT, HGT);
        landscape.set_pixel_grid(PixelGrid::new(
            WDT,
            HGT as u32,
            bytes,
            vec![0, 50],
            vec![None, Some("Granite".to_owned())],
            vec![None; 2],
        ));
        landscape.set_world_height(HGT);
        engine.set_landscape(landscape);
        // Gravity would add to ydir before the loop runs; the oracle block has
        // no such term, so the fixture removes it rather than modelling it.
        engine.set_physics(PhysicsSettings::new(0, 20, -20));
        engine
            .register_definition(definition)
            .expect("oracle movement fixture registers");

        let object = engine
            .spawn_object(
                SpawnConfig::new("MOVP")
                    .with_position(crate::Vector2::new(
                        i(case, "x0") as i32,
                        i(case, "y0") as i32,
                    ))
                    .with_fixed_position(FixedVec2::new(
                        itofix(i(case, "x0") as i32),
                        itofix(i(case, "y0") as i32),
                    ))
                    .with_fixed_velocity(FixedVec2::new(
                        itofix_prec(i(case, "xdir_n") as i32, i(case, "xdir_d") as i32),
                        itofix_prec(i(case, "ydir_n") as i32, i(case, "ydir_d") as i32),
                    )),
            )
            .expect("oracle movement fixture spawns");
        let index = engine
            .find_object_index(object)
            .expect("oracle movement fixture index");
        engine.objects[index].state.alive = case["alive"].as_bool().unwrap_or(false);

        let definition_id = engine.objects[index].definition_id.clone();
        let action_library = engine
            .definitions
            .get(&definition_id)
            .expect("movement fixture definition")
            .action_library()
            .clone();
        engine
            .exec_object_movement(index, &action_library, &definition_id, &[])
            .expect("movement fixture step");

        let object = &engine.objects[index];
        expect_eq(
            &label,
            0,
            "x",
            i(case, "x"),
            i64::from(object.state.position.x),
        );
        expect_eq(
            &label,
            0,
            "y",
            i(case, "y"),
            i64::from(object.state.position.y),
        );
        expect_eq(
            &label,
            0,
            "fix_x",
            i(case, "fix_x"),
            i64::from(object.fixed_position.x.val()),
        );
        expect_eq(
            &label,
            0,
            "fix_y",
            i(case, "fix_y"),
            i64::from(object.fixed_position.y.val()),
        );
        expect_eq(
            &label,
            0,
            "xdir",
            i(case, "xdir"),
            i64::from(object.fixed_velocity.x.val()),
        );
        expect_eq(
            &label,
            0,
            "ydir",
            i(case, "ydir"),
            i64::from(object.fixed_velocity.y.val()),
        );
    }

    // 16b11. A differential matrix over the complete unattached per-pixel
    //         translation stage (C4Movement.cpp:254-322). Unlike the compact
    //         smoke block above, these rows record every ContactCheck candidate
    //         and cover both directions on both axes, raw fixed snapping,
    //         border material rules, ordered multi-vertex friction/weight,
    //         horizontal-before-vertical aggregation, synchronous Contact*
    //         ordering and RNG, and a real stationary C4SolidMask wall.
    for (case_index, case) in golden["do_movement_collision_matrix"]
        .as_array()
        .expect("do_movement_collision_matrix is an array")
        .iter()
        .enumerate()
    {
        const WIDTH: u32 = 24;
        const HEIGHT: i32 = 16;
        const EARTH: u8 = 1;
        const VEHICLE: u8 = 3;

        let name = case["name"].as_str().unwrap_or("?");
        let label = format!("do_movement_collision_matrix[{name}]");
        let callback_return_mask = i(case, "callback_return_mask") as u32;
        let callback_random = i(case, "callback_random") != 0;
        let random_call = if callback_random { "Random(17);" } else { "" };
        let callback_result = |cnat| i32::from(callback_return_mask & cnat != 0);
        let script = format!(
            r#"#strict 2
local callback_order, callback_count;

protected func ContactLeft()
{{
    callback_order = callback_order * 16 + 1;
    callback_count++;
    {random_call}
    return {left_result};
}}

protected func ContactRight()
{{
    callback_order = callback_order * 16 + 2;
    callback_count++;
    {random_call}
    return {right_result};
}}

protected func ContactTop()
{{
    callback_order = callback_order * 16 + 4;
    callback_count++;
    {random_call}
    return {top_result};
}}

protected func ContactBottom()
{{
    callback_order = callback_order * 16 + 8;
    callback_count++;
    {random_call}
    return {bottom_result};
}}
"#,
            left_result = callback_result(crate::CNAT_LEFT),
            right_result = callback_result(crate::CNAT_RIGHT),
            top_result = callback_result(crate::CNAT_TOP),
            bottom_result = callback_result(crate::CNAT_BOTTOM),
        );
        let mut definition = Definition::from_script("MOVM", "Movement matrix", &script)
            .expect("movement matrix definition compiles");
        definition.set_c4_callback_convention(true);
        definition.set_contact_function_calls(i(case, "callbacks") != 0);
        definition.set_border_bound(i(case, "border_bound") as i32);
        definition.set_rotateable(i32::from(i(case, "rotatable") != 0));
        definition.set_shape_rect(Some(DefinitionRect::new(-2, -2, 5, 5)));
        definition.set_shape_vertices(
            case["vertices"]
                .as_array()
                .expect("movement matrix vertices")
                .iter()
                .map(|vertex| crate::ObjectVertex {
                    friction: i(vertex, "friction") as i32,
                    ..crate::ObjectVertex::new(i(vertex, "x") as i32, i(vertex, "y") as i32)
                        .with_cnat(i(vertex, "cnat") as u32)
                })
                .collect(),
        );

        let mut engine = Engine::with_seed(i(case, "seed") as u64);
        engine.configure_materials_from_library(&contact_oracle_materials());
        let floor_y = i(case, "floor_y") as i32;
        let ceiling_y = i(case, "ceiling_y") as i32;
        let wall_x = i(case, "wall_x") as i32;
        let wall_material = i(case, "wall_material") as u8;
        let mut bytes = vec![0_u8; WIDTH as usize * HEIGHT as usize];
        if floor_y >= 0 {
            for y in floor_y..HEIGHT {
                for x in 0..WIDTH as usize {
                    bytes[y as usize * WIDTH as usize + x] = EARTH;
                }
            }
        }
        if ceiling_y >= 0 {
            for y in 0..=ceiling_y {
                for x in 0..WIDTH as usize {
                    bytes[y as usize * WIDTH as usize + x] = EARTH;
                }
            }
        }
        if wall_x >= 0 && wall_material != VEHICLE {
            for y in 0..HEIGHT {
                bytes[y as usize * WIDTH as usize + wall_x as usize] = wall_material;
            }
        }
        let mut densities = vec![0; 128];
        densities[EARTH as usize] = 50;
        densities[VEHICLE as usize] = 100;
        let mut material_names = vec![None; 128];
        material_names[EARTH as usize] = Some("Earth".to_string());
        material_names[VEHICLE as usize] = Some("Vehicle".to_string());
        let mut landscape = Landscape::flat(WIDTH, HEIGHT);
        landscape.set_pixel_grid(PixelGrid::new(
            WIDTH,
            HEIGHT as u32,
            bytes,
            densities,
            material_names,
            vec![None; 128],
        ));
        landscape.set_world_height(HEIGHT);
        landscape.set_border_open(
            i(case, "left_open") as i32,
            i(case, "right_open") as i32,
            i(case, "top_open") != 0,
            i(case, "bottom_open") != 0,
        );
        let vehicle = engine
            .materials
            .id_of("Vehicle")
            .expect("movement matrix declares Vehicle");
        landscape.set_vehicle_material(Some(vehicle));
        engine.set_landscape(landscape);
        engine
            .register_definition(definition)
            .expect("movement matrix definition registers");

        if wall_material == VEHICLE {
            assert_eq!(
                engine
                    .landscape()
                    .and_then(|landscape| landscape.grid_byte_at(wall_x, i(case, "y0") as i32)),
                Some(0),
                "{label}: vehicle wall must not be prepainted terrain"
            );
            let mut blocker = Definition::from_script("MSKB", "Mask blocker", "#strict 2\n")
                .expect("movement mask blocker compiles");
            blocker.set_solid_mask(Some(DefinitionTargetRect::new(0, 0, 1, HEIGHT, 0, 0)));
            engine
                .register_definition(blocker)
                .expect("movement mask blocker registers");
            let blocker_id = engine
                .spawn_object(
                    SpawnConfig::new("MSKB").with_position(crate::Vector2::new(wall_x, 0)),
                )
                .expect("movement mask blocker spawns");
            let blocker_index = engine
                .find_object_index(blocker_id)
                .expect("movement mask blocker remains");
            assert!(
                engine.objects[blocker_index].solid_mask_bake.is_some(),
                "{label}: stationary blocker must own a live baked mask"
            );
            assert_eq!(
                engine
                    .landscape()
                    .and_then(|landscape| landscape.grid_byte_at(wall_x, i(case, "y0") as i32)),
                Some(VEHICLE),
                "{label}: the stationary object must contribute the wall through its live solid mask"
            );
        }

        let x0 = i(case, "x0") as i32;
        let y0 = i(case, "y0") as i32;
        let object_id = engine
            .spawn_object(
                SpawnConfig::new("MOVM")
                    .with_position(crate::Vector2::new(x0, y0))
                    .with_fixed_position(FixedVec2::new(
                        itofix(x0) + C4Fixed::from_raw(i(case, "fix_x_offset") as i32),
                        itofix(y0) + C4Fixed::from_raw(i(case, "fix_y_offset") as i32),
                    ))
                    .with_fixed_velocity(FixedVec2::new(
                        itofix_prec(i(case, "xdir_n") as i32, i(case, "xdir_d") as i32),
                        itofix_prec(i(case, "ydir_n") as i32, i(case, "ydir_d") as i32),
                    )),
            )
            .expect("movement matrix object spawns");
        let object_index = engine
            .find_object_index(object_id)
            .expect("movement matrix object remains");
        {
            let object = &mut engine.objects[object_index];
            // Programmatic spawn performs the initial construction bottom
            // adjustment; the extracted C++ fixture starts from the authored
            // C4Object coordinates directly.
            object.state.position = crate::Vector2::new(x0, y0);
            object.fixed_position = FixedVec2::new(
                itofix(x0) + C4Fixed::from_raw(i(case, "fix_x_offset") as i32),
                itofix(y0) + C4Fixed::from_raw(i(case, "fix_y_offset") as i32),
            );
            object.state.alive = i(case, "alive") != 0;
        }
        engine.objects[object_index]
            .state
            .local_vars
            .insert("callback_order".to_string(), ScriptValue::Int(0));
        engine.objects[object_index]
            .state
            .local_vars
            .insert("callback_count".to_string(), ScriptValue::Int(0));

        let (outcome, trace) = engine
            .parity_advance_live_position_per_pixel(object_index)
            .expect("movement matrix translation succeeds");
        let traces = &trace.probes;
        let object = &engine.objects[object_index];
        for (field, actual) in [
            ("x", i64::from(object.state.position.x)),
            ("y", i64::from(object.state.position.y)),
            ("fix_x", i64::from(object.fixed_position.x.val())),
            ("fix_y", i64::from(object.fixed_position.y.val())),
            ("xdir", i64::from(object.fixed_velocity.x.val())),
            ("ydir", i64::from(object.fixed_velocity.y.val())),
            ("rdir", i64::from(object.rotation_velocity.val())),
            ("motion_x", i64::from(object.motion_x)),
            ("motion_y", i64::from(object.motion_y)),
            ("any_contact", i64::from(outcome.any_contact)),
            ("contacts", i64::from(outcome.contact_cnat)),
            ("redirect_yr", i64::from(outcome.redirect_yr)),
            ("t_contact", i64::from(object.frame_t_contact)),
            ("contact_count", i64::from(object.frame_shape_contact_count)),
            ("contact_cnat", i64::from(object.frame_shape_contact_cnat)),
        ] {
            expect_eq(&label, case_index, field, i(case, field), actual);
        }
        expect_json_eq(
            &label,
            case_index,
            "vertex_contacts",
            case["vertex_contacts"].clone(),
            serde_json::json!(object.frame_vertex_contacts),
        );

        let expected_traces = case["probes"].as_array().expect("movement matrix probes");
        expect_eq(
            &label,
            case_index,
            "probe_count",
            expected_traces.len() as i64,
            traces.len() as i64,
        );
        for (probe_index, (expected, actual)) in expected_traces.iter().zip(traces).enumerate() {
            let probe_label = format!("{label}.probe[{probe_index}]");
            for (field, actual_value) in [
                ("x", i64::from(actual.position.x)),
                ("y", i64::from(actual.position.y)),
                ("object_x", i64::from(actual.object_position.x)),
                ("object_y", i64::from(actual.object_position.y)),
                ("fix_x", i64::from(actual.fixed_position.x.val())),
                ("fix_y", i64::from(actual.fixed_position.y.val())),
                ("rotation", i64::from(actual.rotation)),
                ("fix_r", i64::from(actual.fixed_rotation.val())),
                ("xdir", i64::from(actual.fixed_velocity.x.val())),
                ("ydir", i64::from(actual.fixed_velocity.y.val())),
                ("rdir", i64::from(actual.rotation_velocity.val())),
                ("motion_x", i64::from(actual.motion_x)),
                ("motion_y", i64::from(actual.motion_y)),
                ("result", i64::from(actual.contact_count)),
                ("t_contact", i64::from(actual.t_contact)),
                ("contact_count", i64::from(actual.contact_count)),
                ("contact_cnat", i64::from(actual.contact_cnat)),
                ("random_count", i64::from(actual.random_count)),
            ] {
                expect_eq(
                    &probe_label,
                    case_index,
                    field,
                    i(expected, field),
                    actual_value,
                );
            }
            expect_json_eq(
                &probe_label,
                case_index,
                "vertex_contacts",
                expected["vertex_contacts"].clone(),
                serde_json::json!(actual.vertex_contacts),
            );
            expect_eq_u64(
                &probe_label,
                case_index,
                "random_hold",
                u(expected, "random_hold"),
                u64::from(actual.random_hold),
            );
            expect_eq(
                &probe_label,
                case_index,
                "result_is_contact",
                i64::from(i(expected, "result") != 0),
                i64::from(actual.result),
            );
        }

        let expected_callback_order = case["callback_order"]
            .as_array()
            .expect("movement matrix callback_order");
        let encoded_callback_order = expected_callback_order.iter().fold(0_i64, |order, cnat| {
            order * 16 + cnat.as_i64().expect("callback direction is an integer")
        });
        expect_eq(
            &label,
            case_index,
            "callback_order",
            encoded_callback_order,
            action_callback_local(&engine, object_id, "callback_order"),
        );
        expect_eq(
            &label,
            case_index,
            "callback_count",
            expected_callback_order.len() as i64,
            action_callback_local(&engine, object_id, "callback_count"),
        );
        expect_eq(
            &label,
            case_index,
            "contact_invocations",
            i(case, "contact_invocations"),
            trace.contact_invocations as i64,
        );
        expect_rng_state_at(&label, case_index, case, &engine.rng);
    }

    // 16b11b. C4Object::DoMovement's *attached* walk (C4Movement.cpp:324-370).
    //         A separate loop from the unattached one above: every step
    //         re-runs Shape.Attach (C4Shape.cpp:165-271), an attachment that
    //         moves the step target overrides the momentum target and zeroes
    //         that axis' velocity, a contact aborts the walk by snapping both
    //         target and raw accumulator back, and a failed attachment raises
    //         fNoAttach. Raw C4Fixed values are compared, never fixtoi alone.
    for (case_index, case) in golden["do_movement_attached"]
        .as_array()
        .expect("do_movement_attached is an array")
        .iter()
        .enumerate()
    {
        const WIDTH: u32 = 24;
        const HEIGHT: i32 = 16;
        const EARTH: u8 = 1;
        const VEHICLE: u8 = 3;

        let name = case["name"].as_str().unwrap_or("?");
        let label = format!("do_movement_attached[{name}]");

        let mut definition = Definition::from_script("ATCH", "Attached movement", "#strict 2\n")
            .expect("attached movement definition compiles");
        definition.set_border_bound(i(case, "border_bound") as i32);
        definition.set_shape_rect(Some(DefinitionRect::new(-2, -2, 5, 5)));
        definition.set_shape_vertices(
            case["vertices"]
                .as_array()
                .expect("attached movement vertices")
                .iter()
                .map(|vertex| crate::ObjectVertex {
                    friction: i(vertex, "friction") as i32,
                    ..crate::ObjectVertex::new(i(vertex, "x") as i32, i(vertex, "y") as i32)
                        .with_cnat(i(vertex, "cnat") as u32)
                })
                .collect(),
        );

        let mut engine = Engine::with_seed(i(case, "seed") as u64);
        engine.configure_materials_from_library(&contact_oracle_materials());
        let floor_y = i(case, "floor_y") as i32;
        let ceiling_y = i(case, "ceiling_y") as i32;
        let wall_x = i(case, "wall_x") as i32;
        let wall_material = i(case, "wall_material") as u8;
        let mut bytes = vec![0_u8; WIDTH as usize * HEIGHT as usize];
        if floor_y >= 0 {
            for y in floor_y..HEIGHT {
                for x in 0..WIDTH as usize {
                    bytes[y as usize * WIDTH as usize + x] = EARTH;
                }
            }
        }
        if ceiling_y >= 0 {
            for y in 0..=ceiling_y {
                for x in 0..WIDTH as usize {
                    bytes[y as usize * WIDTH as usize + x] = EARTH;
                }
            }
        }
        if wall_x >= 0 && wall_material != VEHICLE {
            for y in 0..HEIGHT {
                bytes[y as usize * WIDTH as usize + wall_x as usize] = wall_material;
            }
        }
        let mut densities = vec![0; 128];
        densities[EARTH as usize] = 50;
        densities[VEHICLE as usize] = 100;
        let mut material_names = vec![None; 128];
        material_names[EARTH as usize] = Some("Earth".to_string());
        material_names[VEHICLE as usize] = Some("Vehicle".to_string());
        let mut landscape = Landscape::flat(WIDTH, HEIGHT);
        landscape.set_pixel_grid(PixelGrid::new(
            WIDTH,
            HEIGHT as u32,
            bytes,
            densities,
            material_names,
            vec![None; 128],
        ));
        landscape.set_world_height(HEIGHT);
        landscape.set_border_open(
            i(case, "left_open") as i32,
            i(case, "right_open") as i32,
            i(case, "top_open") != 0,
            i(case, "bottom_open") != 0,
        );
        let vehicle = engine
            .materials
            .id_of("Vehicle")
            .expect("attached movement declares Vehicle");
        landscape.set_vehicle_material(Some(vehicle));
        engine.set_landscape(landscape);
        engine
            .register_definition(definition)
            .expect("attached movement definition registers");

        if wall_material == VEHICLE {
            let mut blocker = Definition::from_script("ATMB", "Mask blocker", "#strict 2\n")
                .expect("attached mask blocker compiles");
            blocker.set_solid_mask(Some(DefinitionTargetRect::new(0, 0, 1, HEIGHT, 0, 0)));
            engine
                .register_definition(blocker)
                .expect("attached mask blocker registers");
            engine
                .spawn_object(
                    SpawnConfig::new("ATMB").with_position(crate::Vector2::new(wall_x, 0)),
                )
                .expect("attached mask blocker spawns");
        }

        // `x`/`y` are the *post-walk* position the golden reports, so the
        // authored start travels separately as `start_x`/`start_y`.
        let start_x = i(case, "start_x") as i32;
        let start_y = i(case, "start_y") as i32;
        let object_id = engine
            .spawn_object(
                SpawnConfig::new("ATCH").with_position(crate::Vector2::new(start_x, start_y)),
            )
            .expect("attached movement object spawns");
        let object_index = engine
            .find_object_index(object_id)
            .expect("attached movement object remains");
        {
            let object = &mut engine.objects[object_index];
            object.state.position = crate::Vector2::new(start_x, start_y);
            object.fixed_position = FixedVec2::new(
                itofix(start_x) + C4Fixed::from_raw(i(case, "fix_x_offset") as i32),
                itofix(start_y) + C4Fixed::from_raw(i(case, "fix_y_offset") as i32),
            );
            object.fixed_velocity = FixedVec2::new(
                itofix_prec(i(case, "xdir_n") as i32, i(case, "xdir_d") as i32),
                itofix_prec(i(case, "ydir_n") as i32, i(case, "ydir_d") as i32),
            );
            object.state.alive = true;
            // `Action.t_attach` as ExecAction would have latched it
            // (C4Object.cpp:4692); this section drives the movement walk
            // directly, so the latch is set here instead.
            object.frame_t_attach = i(case, "t_attach") as u32;
        }

        let (outcome, trace) = engine
            .parity_advance_live_position_per_pixel(object_index)
            .expect("attached movement translation succeeds");
        let object = &engine.objects[object_index];
        for (field, actual) in [
            ("x", i64::from(object.state.position.x)),
            ("y", i64::from(object.state.position.y)),
            ("fix_x", i64::from(object.fixed_position.x.val())),
            ("fix_y", i64::from(object.fixed_position.y.val())),
            ("xdir", i64::from(object.fixed_velocity.x.val())),
            ("ydir", i64::from(object.fixed_velocity.y.val())),
            ("motion_x", i64::from(object.motion_x)),
            ("motion_y", i64::from(object.motion_y)),
            ("no_attach", i64::from(outcome.no_attach)),
            ("any_contact", i64::from(outcome.any_contact)),
            ("contacts", i64::from(outcome.contact_cnat)),
            ("t_contact", i64::from(object.frame_t_contact)),
            (
                "attach_mat_valid",
                i64::from(object.state.shape_attach.mat_valid),
            ),
            (
                "attach_mat_vehicle",
                i64::from(object.state.shape_attach.mat_vehicle),
            ),
            ("attach_x", i64::from(object.state.shape_attach.x)),
            ("attach_y", i64::from(object.state.shape_attach.y)),
            ("attach_vtx", i64::from(object.state.shape_attach.vtx)),
        ] {
            expect_eq(&label, case_index, field, i(case, field), actual);
        }

        let expected_traces = case["probes"].as_array().expect("attached movement probes");
        expect_eq(
            &label,
            case_index,
            "probe_count",
            expected_traces.len() as i64,
            trace.probes.len() as i64,
        );
        for (probe_index, (expected, actual)) in
            expected_traces.iter().zip(&trace.probes).enumerate()
        {
            let probe_label = format!("{label}.probe[{probe_index}]");
            for (field, actual_value) in [
                ("x", i64::from(actual.position.x)),
                ("y", i64::from(actual.position.y)),
                ("object_x", i64::from(actual.object_position.x)),
                ("object_y", i64::from(actual.object_position.y)),
                ("fix_x", i64::from(actual.fixed_position.x.val())),
                ("fix_y", i64::from(actual.fixed_position.y.val())),
                ("xdir", i64::from(actual.fixed_velocity.x.val())),
                ("ydir", i64::from(actual.fixed_velocity.y.val())),
                ("motion_x", i64::from(actual.motion_x)),
                ("motion_y", i64::from(actual.motion_y)),
                ("t_contact", i64::from(actual.t_contact)),
            ] {
                expect_eq(
                    &probe_label,
                    case_index,
                    field,
                    i(expected, field),
                    actual_value,
                );
            }
            expect_eq(
                &probe_label,
                case_index,
                "result_is_contact",
                i64::from(i(expected, "result") != 0),
                i64::from(actual.result),
            );
        }
        expect_eq(
            &label,
            case_index,
            "contact_invocations",
            i(case, "contact_invocations"),
            trace.contact_invocations as i64,
        );
        expect_rng_state_at(&label, case_index, case, &engine.rng);
    }

    // 16b12. C4Object::DoMovement's rotation walk (C4Movement.cpp:372-436).
    //         Rotation advances one whole degree at a time from an absolute
    //         definition shape, probes each result, restores the last accepted
    //         shape on collision, and conditionally redirects rdir into ydir.
    for (case_index, case) in golden["do_movement_rotation_matrix"]
        .as_array()
        .expect("do_movement_rotation_matrix is an array")
        .iter()
        .enumerate()
    {
        const WIDTH: u32 = 24;
        const HEIGHT: i32 = 16;
        let name = case["name"].as_str().unwrap_or("?");
        let label = format!("do_movement_rotation_matrix[{name}]");
        let mut definition = Definition::from_script("ROTM", "Rotation matrix", "#strict 2\n")
            .expect("rotation matrix definition compiles");
        definition.set_rotateable(i(case, "rotateable") as i32);
        definition.set_shape_rect(Some(DefinitionRect::new(-1, -1, 3, 12)));
        definition.set_shape_vertices(
            case["vertices"]
                .as_array()
                .expect("rotation matrix vertices")
                .iter()
                .map(|vertex| crate::ObjectVertex {
                    friction: i(vertex, "friction") as i32,
                    ..crate::ObjectVertex::new(i(vertex, "x") as i32, i(vertex, "y") as i32)
                        .with_cnat(i(vertex, "cnat") as u32)
                })
                .collect(),
        );

        let mut engine = Engine::with_seed(i(case, "seed") as u64);
        engine.configure_materials_from_library(&contact_oracle_materials());
        let wall_x = i(case, "wall_x") as i32;
        let mut bytes = vec![0_u8; WIDTH as usize * HEIGHT as usize];
        if wall_x >= 0 {
            for y in 0..HEIGHT {
                bytes[y as usize * WIDTH as usize + wall_x as usize] = 1;
            }
        }
        let mut densities = vec![0; 128];
        densities[1] = 50;
        densities[3] = 100;
        let mut material_names = vec![None; 128];
        material_names[1] = Some("Earth".to_string());
        material_names[3] = Some("Vehicle".to_string());
        let mut landscape = Landscape::flat(WIDTH, HEIGHT);
        landscape.set_pixel_grid(PixelGrid::new(
            WIDTH,
            HEIGHT as u32,
            bytes,
            densities,
            material_names,
            vec![None; 128],
        ));
        landscape.set_world_height(HEIGHT);
        landscape.set_border_open(0, 0, true, false);
        let vehicle = engine
            .materials
            .id_of("Vehicle")
            .expect("rotation matrix declares Vehicle");
        landscape.set_vehicle_material(Some(vehicle));
        engine.set_landscape(landscape);
        engine
            .register_definition(definition)
            .expect("rotation matrix definition registers");

        let x0 = i(case, "x0") as i32;
        let y0 = i(case, "y0") as i32;
        let rotation0 = i(case, "rotation0") as i32;
        let fixed_rotation = itofix(rotation0) + C4Fixed::from_raw(i(case, "fix_r_raw") as i32);
        let object_id = engine
            .spawn_object(
                SpawnConfig::new("ROTM")
                    .with_position(crate::Vector2::new(x0, y0))
                    .with_fixed_position(FixedVec2::new(itofix(x0), itofix(y0)))
                    .with_rotation(rotation0)
                    .with_fixed_rotation(fixed_rotation)
                    .with_rotation_velocity(itofix_prec(
                        i(case, "rdir_n") as i32,
                        i(case, "rdir_d") as i32,
                    ))
                    .with_fixed_velocity(FixedVec2::new(
                        C4Fixed::ZERO,
                        itofix_prec(i(case, "ydir_n") as i32, i(case, "ydir_d") as i32),
                    )),
            )
            .expect("rotation matrix object spawns");
        let object_index = engine
            .find_object_index(object_id)
            .expect("rotation matrix object remains");
        {
            let object = &mut engine.objects[object_index];
            object.state.position = crate::Vector2::new(x0, y0);
            object.fixed_position = FixedVec2::new(itofix(x0), itofix(y0));
            object.state.rotation = rotation0;
            object.fixed_rotation = fixed_rotation;
            object.rotation_velocity =
                itofix_prec(i(case, "rdir_n") as i32, i(case, "rdir_d") as i32);
            object.fixed_velocity.y =
                itofix_prec(i(case, "ydir_n") as i32, i(case, "ydir_d") as i32);
            object.refresh_shape_geometry();
        }

        let ((rotation_contact, rotation_contacts, turned), trace) = engine
            .parity_advance_live_rotation(object_index, i(case, "redirect_yr") != 0)
            .expect("rotation matrix walk succeeds");
        let traces = &trace.probes;
        let object = &engine.objects[object_index];
        let shape = object
            .current_shape_rect()
            .expect("rotation matrix shape remains");
        for (field, actual) in [
            ("x", i64::from(object.state.position.x)),
            ("y", i64::from(object.state.position.y)),
            ("fix_x", i64::from(object.fixed_position.x.val())),
            ("fix_y", i64::from(object.fixed_position.y.val())),
            ("rotation", i64::from(object.state.rotation)),
            ("fix_r", i64::from(object.fixed_rotation.val())),
            ("rdir", i64::from(object.rotation_velocity.val())),
            ("ydir", i64::from(object.fixed_velocity.y.val())),
            ("motion_x", i64::from(object.motion_x)),
            ("motion_y", i64::from(object.motion_y)),
            ("any_contact", i64::from(rotation_contact)),
            ("contacts", i64::from(rotation_contacts)),
            ("turned", i64::from(turned)),
            ("t_contact", i64::from(object.frame_t_contact)),
            ("contact_count", i64::from(object.frame_shape_contact_count)),
            ("contact_cnat", i64::from(object.frame_shape_contact_cnat)),
            ("shape_x", i64::from(shape.x)),
            ("shape_y", i64::from(shape.y)),
            ("shape_wdt", i64::from(shape.width)),
            ("shape_hgt", i64::from(shape.height)),
        ] {
            expect_eq(&label, case_index, field, i(case, field), actual);
        }
        expect_json_eq(
            &label,
            case_index,
            "final_vertices",
            case["final_vertices"].clone(),
            Value::Array(
                object
                    .state
                    .vertices
                    .iter()
                    .map(|vertex| {
                        serde_json::json!({
                            "x": vertex.x,
                            "y": vertex.y,
                            "cnat": vertex.cnat,
                            "friction": vertex.friction,
                        })
                    })
                    .collect(),
            ),
        );
        expect_eq(
            &label,
            case_index,
            "update_pos_calls",
            i(case, "update_pos_calls"),
            trace.update_pos_calls as i64,
        );

        let expected_traces = case["probes"].as_array().expect("rotation matrix probes");
        expect_eq(
            &label,
            case_index,
            "probe_count",
            expected_traces.len() as i64,
            traces.len() as i64,
        );
        for (probe_index, (expected, actual)) in expected_traces.iter().zip(traces).enumerate() {
            let probe_label = format!("{label}.probe[{probe_index}]");
            for (field, actual_value) in [
                ("x", i64::from(actual.position.x)),
                ("y", i64::from(actual.position.y)),
                ("object_x", i64::from(actual.object_position.x)),
                ("object_y", i64::from(actual.object_position.y)),
                ("fix_x", i64::from(actual.fixed_position.x.val())),
                ("fix_y", i64::from(actual.fixed_position.y.val())),
                ("rotation", i64::from(actual.rotation)),
                ("fix_r", i64::from(actual.fixed_rotation.val())),
                ("xdir", i64::from(actual.fixed_velocity.x.val())),
                ("ydir", i64::from(actual.fixed_velocity.y.val())),
                ("rdir", i64::from(actual.rotation_velocity.val())),
                ("motion_x", i64::from(actual.motion_x)),
                ("motion_y", i64::from(actual.motion_y)),
                ("result", i64::from(actual.contact_count)),
                ("t_contact", i64::from(actual.t_contact)),
                ("contact_count", i64::from(actual.contact_count)),
                ("contact_cnat", i64::from(actual.contact_cnat)),
                ("random_count", i64::from(actual.random_count)),
            ] {
                expect_eq(
                    &probe_label,
                    case_index,
                    field,
                    i(expected, field),
                    actual_value,
                );
            }
            expect_json_eq(
                &probe_label,
                case_index,
                "vertex_contacts",
                expected["vertex_contacts"].clone(),
                serde_json::json!(actual.vertex_contacts),
            );
            expect_eq_u64(
                &probe_label,
                case_index,
                "random_hold",
                u(expected, "random_hold"),
                u64::from(actual.random_hold),
            );
        }
        expect_rng_state_at(&label, case_index, case, &engine.rng);
    }

    // 16b12b. The landscape-aware half of C4Shape::LineConnect
    //          (C4Shape.cpp:273-331): the endpoint move when the new path is
    //          free, the three-range bend search seeded from ForLine's reported
    //          intersection, the old-endpoint PathFreeIgnoreVehicle fallback,
    //          and the ordered vertex list each produces. The ignore-vehicle
    //          predicate compares a material *index* against C4M_Solid = 50
    //          (C4Landscape.cpp:2044-2048; C4Wrappers.h:68-71), which is why an
    //          ordinary earth wall still finds the fallback and only a
    //          high-index material can break the line.
    for (case_index, case) in golden["line_connect_routing"]
        .as_array()
        .expect("line_connect_routing is an array")
        .iter()
        .enumerate()
    {
        const WIDTH: u32 = 40;
        const HEIGHT: i32 = 30;
        const EARTH: u8 = 1;
        const VEHICLE: u8 = 3;
        const HIGH_INDEX: u8 = 60;

        let name = case["name"].as_str().unwrap_or("?");
        let label = format!("line_connect_routing[{name}]");

        // The oracle grid stores material indices directly, so the Rust
        // material map has to place each one at the same index: Earth at 1,
        // Vehicle at 3, and a solid material at 60 whose index alone is what
        // makes the ignore-vehicle check block.
        let mut library = String::new();
        for index in 0..=u32::from(HIGH_INDEX) {
            let (name, density) = match u8::try_from(index).unwrap_or(0) {
                EARTH => ("Earth".to_string(), 50),
                VEHICLE => ("Vehicle".to_string(), 100),
                HIGH_INDEX => ("HighIndex".to_string(), 100),
                other => (format!("Filler{other:02}"), 0),
            };
            library.push_str(&format!(
                "\n[Material {name}]\nName={name}\nDensity={density}\n"
            ));
        }
        let materials = clonk_resources::MaterialLibrary::parse(&library)
            .expect("line connect materials parse");

        let mut engine = Engine::with_seed(0);
        engine.configure_materials_from_library(&materials);
        assert_eq!(
            engine.materials.id_of("HighIndex").map(|id| id.index()),
            Some(usize::from(HIGH_INDEX)),
            "{label}: the high-index material must sit at the oracle's index"
        );

        let mut bytes = vec![0_u8; WIDTH as usize * HEIGHT as usize];
        for wall in case["walls"].as_array().expect("line connect walls") {
            let (x, y) = (i(wall, "x") as i32, i(wall, "y") as i32);
            let (w, h) = (i(wall, "w") as i32, i(wall, "h") as i32);
            let material = i(wall, "material") as u8;
            for gy in y..(y + h).min(HEIGHT) {
                for gx in x..(x + w).min(WIDTH as i32) {
                    if gx >= 0 && gy >= 0 {
                        bytes[gy as usize * WIDTH as usize + gx as usize] = material;
                    }
                }
            }
        }
        let mut densities = vec![0; 128];
        let mut material_names = vec![None; 128];
        for byte in [EARTH, VEHICLE, HIGH_INDEX] {
            let name = match byte {
                EARTH => "Earth",
                VEHICLE => "Vehicle",
                _ => "HighIndex",
            };
            densities[usize::from(byte)] = if byte == EARTH { 50 } else { 100 };
            material_names[usize::from(byte)] = Some(name.to_string());
        }
        let mut landscape = Landscape::flat(WIDTH, HEIGHT);
        landscape.set_pixel_grid(PixelGrid::new(
            WIDTH,
            HEIGHT as u32,
            bytes,
            densities,
            material_names,
            vec![None; 128],
        ));
        landscape.set_world_height(HEIGHT);
        let vehicle = engine
            .materials
            .id_of("Vehicle")
            .expect("line connect declares Vehicle");
        landscape.set_vehicle_material(Some(vehicle));
        // The grid resolves its byte→material table against the engine's
        // material map, so the landscape has to be installed before any path
        // check reads it; a standalone landscape answers "no material" for
        // every byte and would make every path look free.
        engine.set_landscape(landscape);

        let mut vertices = case["start_vertices"]
            .as_array()
            .expect("line connect start vertices")
            .iter()
            .map(|vertex| crate::ObjectVertex::new(i(vertex, "x") as i32, i(vertex, "y") as i32))
            .collect::<Vec<_>>();
        let endpoint = usize::try_from(i(case, "cvtx")).expect("line connect endpoint index");
        let direction = isize::try_from(i(case, "ld")).expect("line connect direction");
        let target = crate::Vector2::new(i(case, "tx") as i32, i(case, "ty") as i32);

        let installed = engine.landscape().expect("line connect landscape installs");
        let connected = Engine::line_connect_endpoint(
            Some(installed),
            &mut vertices,
            target,
            endpoint,
            direction,
        );

        expect_eq(
            &label,
            case_index,
            "connected",
            i(case, "connected"),
            i64::from(connected),
        );
        expect_eq(
            &label,
            case_index,
            "vertex_count",
            i(case, "vertex_count"),
            vertices.len() as i64,
        );
        expect_json_eq(
            &label,
            case_index,
            "vertices",
            case["vertices"].clone(),
            serde_json::json!(vertices
                .iter()
                .map(|vertex| serde_json::json!({"x": vertex.x, "y": vertex.y}))
                .collect::<Vec<_>>()),
        );
    }

    // 16b13. The DoMovement tail bridges its aggregate contact mask into
    //         ContactAction (C4Movement.cpp:467-472). A wall probe followed by
    //         a floor probe leaves the last-probe mask at Bottom but hands
    //         Right|Bottom to ContactAction; the exact DFA_FLIGHT bottom arm
    //         takes precedence and changes Flight to Walk
    //         (C4Object.cpp:4336-4351).
    for (case_index, case) in golden["do_movement_contact_action_handoff"]
        .as_array()
        .expect("do_movement_contact_action_handoff is an array")
        .iter()
        .enumerate()
    {
        const WIDTH: u32 = 24;
        const HEIGHT: i32 = 16;
        let label = "do_movement_contact_action_handoff";
        let mut definition = Definition::from_script("MCAH", "Movement handoff", "#strict 2\n")
            .expect("movement handoff definition compiles");
        definition.set_shape_rect(Some(DefinitionRect::new(-2, -2, 5, 5)));
        definition.set_physical(PhysicalInfo {
            can_scale: 1,
            ..PhysicalInfo::default()
        });
        definition.set_shape_vertices(vec![
            crate::ObjectVertex::new(0, 1).with_cnat(crate::CNAT_BOTTOM),
            crate::ObjectVertex::new(1, 0).with_cnat(crate::CNAT_RIGHT),
        ]);
        definition.configure_actions(
            Some("Flight".to_string()),
            HashMap::from([
                (
                    "Flight".to_string(),
                    ActionSpec::default().with_procedure("FLIGHT"),
                ),
                ("FlatUp".to_string(), ActionSpec::default()),
                ("KneelDown".to_string(), ActionSpec::default()),
                (
                    "Walk".to_string(),
                    ActionSpec::default().with_procedure("WALK"),
                ),
                (
                    "Scale".to_string(),
                    ActionSpec::default().with_procedure("SCALE"),
                ),
            ]),
        );

        let mut engine = Engine::with_seed(i(case, "seed") as u64);
        engine.configure_materials_from_library(&contact_oracle_materials());
        let mut bytes = vec![0_u8; WIDTH as usize * HEIGHT as usize];
        for y in 0..HEIGHT {
            for x in 0..WIDTH as i32 {
                if y >= 10 || x == 12 {
                    bytes[y as usize * WIDTH as usize + x as usize] = 1;
                }
            }
        }
        let mut densities = vec![0; 128];
        densities[1] = 50;
        densities[3] = 100;
        let mut material_names = vec![None; 128];
        material_names[1] = Some("Earth".to_string());
        material_names[3] = Some("Vehicle".to_string());
        let mut landscape = Landscape::flat(WIDTH, HEIGHT);
        landscape.set_pixel_grid(PixelGrid::new(
            WIDTH,
            HEIGHT as u32,
            bytes,
            densities,
            material_names,
            vec![None; 128],
        ));
        landscape.set_world_height(HEIGHT);
        landscape.set_border_open(0, 0, true, false);
        let vehicle = engine
            .materials
            .id_of("Vehicle")
            .expect("movement handoff declares Vehicle");
        landscape.set_vehicle_material(Some(vehicle));
        engine.set_landscape(landscape);
        engine
            .register_definition(definition)
            .expect("movement handoff definition registers");
        let object_id = engine
            .spawn_object(
                SpawnConfig::new("MCAH")
                    .with_position(crate::Vector2::new(8, 6))
                    .with_fixed_position(FixedVec2::new(itofix(8), itofix(6)))
                    .with_fixed_velocity(FixedVec2::new(itofix(4), itofix(4)))
                    .with_action(ActionState::new("Flight"))
                    .with_direction(Direction::Right)
                    .with_category(CATEGORY_OBJECT),
            )
            .expect("movement handoff object spawns");
        let object_index = engine
            .find_object_index(object_id)
            .expect("movement handoff object remains");
        {
            let object = &mut engine.objects[object_index];
            object.state.position = crate::Vector2::new(8, 6);
            object.fixed_position = FixedVec2::new(itofix(8), itofix(6));
            object.state.ocf = 0;
        }
        let definition_id = engine.objects[object_index].definition_id.clone();
        let action_library = engine
            .definitions
            .get(&definition_id)
            .expect("movement handoff definition remains")
            .action_library()
            .clone();
        let (_, trace) = engine
            .parity_exec_object_movement(object_index, &action_library, &definition_id, &[])
            .expect("movement handoff full DoMovement succeeds");

        let object = &engine.objects[object_index];
        let action_after = match object.state.action.name.as_str() {
            "Flight" => 0,
            "FlatUp" => 1,
            "KneelDown" => 2,
            "Walk" => 3,
            "Scale" => 4,
            action => panic!("unexpected movement-handoff action `{action}`"),
        };
        for (field, actual) in [
            ("x", i64::from(object.state.position.x)),
            ("y", i64::from(object.state.position.y)),
            ("fix_x", i64::from(object.fixed_position.x.val())),
            ("fix_y", i64::from(object.fixed_position.y.val())),
            ("motion_x", i64::from(object.motion_x)),
            ("motion_y", i64::from(object.motion_y)),
            (
                "pre_tail_t_contact",
                i64::from(trace.pre_contact_action_t_contact.unwrap_or(u32::MAX)),
            ),
            ("contacts", i64::from(object.frame_t_contact)),
            ("final_t_contact", i64::from(object.frame_t_contact)),
            ("contact_action_called", i64::from(action_after != 0)),
            ("action_after", action_after),
            (
                "direction_after",
                i64::from(object.state.direction.to_script_value()),
            ),
            ("xdir_after", i64::from(object.fixed_velocity.x.val())),
            ("ydir_after", i64::from(object.fixed_velocity.y.val())),
        ] {
            expect_eq(label, case_index, field, i(case, field), actual);
        }
        expect_rng_state_at(label, case_index, case, &engine.rng);
    }

    // 16b8. cross_map_reactions: which builtin reaction each (PXS material,
    //       landscape material) pair gets, from the selection loop in
    //       `C4MaterialMap::CrossMapMaterials` (C4Material.cpp:311-346).
    //
    //       This is the decision every arm section depends on, and it has two
    //       properties worth pinning. The chain is an if/else-if LADDER, so its
    //       ORDER is the behaviour: InMatConvert wins over everything, then poof
    //       (incindiary vs extinguisher), then incinerate (incindiary vs
    //       inflammable), then corrode (corrosive vs corrode), then insert as
    //       the fallthrough. And every rung but convert sits behind
    //       `MatDensity(PXS) <= MatDensity(LS)`, so a heavier PXS material
    //       hitting a lighter landscape one gets NO reaction at all.
    //
    //       The material set is adversarial about both. Magma is incindiary AND
    //       corrosive while Tinder is inflammable AND corroding, so
    //       Magma→Tinder separates "incinerate before corrode" from any other
    //       arrangement; Acid reaches corrode against the same Tinder. Snow
    //       declares InMatConvert=Water. Tinder and Granite are heavy, so their
    //       rows are mostly the density gate. Sky participates on both axes,
    //       because C++'s loops start at -1.
    {
        let library = clonk_resources::MaterialLibrary::parse(
            r#"
            [Material Water]
            Name=Water
            Density=25
            Extinguisher=1

            [Material Magma]
            Name=Magma
            Density=25
            Incindiary=1
            Corrosive=100

            [Material Acid]
            Name=Acid
            Density=25
            Corrosive=100

            [Material Tinder]
            Name=Tinder
            Density=50
            Inflammable=1
            Corrode=100

            [Material Granite]
            Name=Granite
            Density=100

            [Material Snow]
            Name=Snow
            Density=25
            InMatConvert=Water
            "#,
        )
        .expect("cross map oracle materials parse");

        let mut engine = Engine::with_seed(0);
        engine.configure_materials_from_library(&library);

        for case in golden["cross_map_reactions"].as_array().unwrap() {
            let pxs = i(case, "pxs") as i32;
            let ls = i(case, "ls") as i32;
            let label = format!(
                "cross_map_reactions[{}->{}]",
                case["pxs_name"].as_str().unwrap_or("?"),
                case["ls_name"].as_str().unwrap_or("?")
            );

            // C++ indexes the map from -1, where -1 is sky and 0 is the first
            // declared material — the same numbering the port assigns, since
            // this fixture declares no reserved slot ahead of Water.
            let to_id = |index: i32| {
                (index >= 0)
                    .then(|| crate::material::MaterialId::new(index as usize))
                    .flatten()
            };

            let reaction = engine.materials.reaction_for_event(
                to_id(pxs),
                to_id(ls),
                MaterialInteractionEvent::PxsMove,
            );
            let got = match reaction.kind {
                crate::material::MaterialReactionKind::None => "none",
                crate::material::MaterialReactionKind::Convert { .. } => "convert",
                crate::material::MaterialReactionKind::Poof => "poof",
                crate::material::MaterialReactionKind::Incinerate => "incinerate",
                crate::material::MaterialReactionKind::Corrode { .. } => "corrode",
                crate::material::MaterialReactionKind::Insert => "insert",
                crate::material::MaterialReactionKind::Script { .. } => "script",
            };
            assert_eq!(
                case["reaction"].as_str().unwrap_or("?"),
                got,
                "PARITY DIVERGENCE in `{label}` field `reaction`",
            );
        }
    }

    // 16b10. custom_reaction_overlay: the custom pass in
    //       `C4MaterialMap::CrossMapMaterials` resolves ConvertMat, tries a
    //       literal TargetSpec before its category keywords, expands direct
    //       and inverse categories, and applies authored reactions in order
    //       (C4Material.cpp:386-472). `SetMatReaction` then performs Reverse
    //       and writes the landscape-major table slot (C4Material.cpp:488-494).
    //
    //       The runtime row also drives all three events through ExecMask=5
    //       and CheckSlide=1, mirroring mrfUserCheck's mask-before-slide order
    //       (C4Material.cpp:612-624). The masked PxsMove entry must lose the
    //       insertion check before it can run, while both allowed entries keep
    //       it; the masked user-defined no-op still suppresses natural Insert.
    //
    //       Quoted TargetSpec and ConvertMat fixtures deliberately travel
    //       through a packed Material.c4g: C++ stores those compiled bytes
    //       verbatim (C4Material.cpp:60-68; StdCompiler.cpp:734-742,936-998),
    //       and the resolver compares them without trimming.
    {
        let custom = &golden["custom_reaction_overlay"];

        let reaction_name = |kind: crate::material::MaterialReactionKind| match kind {
            crate::material::MaterialReactionKind::None => "none",
            crate::material::MaterialReactionKind::Convert { .. } => "convert",
            crate::material::MaterialReactionKind::Poof => "poof",
            crate::material::MaterialReactionKind::Incinerate => "incinerate",
            crate::material::MaterialReactionKind::Corrode { .. } => "corrode",
            crate::material::MaterialReactionKind::Insert => "insert",
            crate::material::MaterialReactionKind::Script { .. } => "script",
        };

        for (index, case) in custom["target_masks"]
            .as_array()
            .expect("custom target-mask cases are an array")
            .iter()
            .enumerate()
        {
            let spec = case["spec"].as_str().expect("custom target spec");
            let inverse = case["inverse"].as_bool().expect("custom inverse flag");
            let source = format!(
                "[Material]\nName=Source\nDensity=10\n\n\
                 [Reaction]\nType=Poof\nTargetSpec={spec}\nInverseSpec={}\n\n\
                 [Material]\nName=Vacuum\nDensity=0\nIncindiary=1\nCorrosive=1\n\n\
                 [Material]\nName=Water\nDensity=25\nExtinguisher=1\nCorrosive=1\nCorrode=1\n\n\
                 [Material]\nName=Rock\nDensity=50\nInflammable=1\nCorrode=1\n",
                i32::from(inverse),
            );
            let library = MaterialLibrary::parse(&source)
                .unwrap_or_else(|error| panic!("custom TargetSpec {spec} parses: {error}"));
            let materials = MaterialSet::from_resource_library(&library);
            let source = materials.id_of("Source").expect("custom source exists");
            let targets = [
                None,
                Some(source),
                Some(materials.id_of("Vacuum").expect("Vacuum exists")),
                Some(materials.id_of("Water").expect("Water exists")),
                Some(materials.id_of("Rock").expect("Rock exists")),
            ];
            let mask = targets
                .into_iter()
                .enumerate()
                .fold(0u32, |mask, (bit, target)| {
                    let reaction = materials.reaction_for_event(
                        Some(source),
                        target,
                        MaterialInteractionEvent::PxsMove,
                    );
                    if reaction.user_defined {
                        mask | (1u32 << bit)
                    } else {
                        mask
                    }
                });
            expect_eq(
                &format!("custom_reaction_overlay.target_masks[{spec},{inverse}]"),
                index,
                "mask",
                i(case, "mask"),
                i64::from(mask),
            );
        }

        for (index, case) in custom["literal_keyword"]
            .as_array()
            .expect("literal-keyword cases are an array")
            .iter()
            .enumerate()
        {
            let inverse = case["inverse"].as_bool().expect("literal inverse flag");
            let source = format!(
                "[Material]\nName=Source\nDensity=10\n\n\
                 [Reaction]\nType=Poof\nTargetSpec=sOlId\nInverseSpec={}\n\n\
                 [Material]\nName=Solid\nDensity=10\n\n\
                 [Material]\nName=Rock\nDensity=50\n",
                i32::from(inverse),
            );
            let library =
                MaterialLibrary::parse(&source).expect("literal keyword-shadow materials parse");
            let materials = MaterialSet::from_resource_library(&library);
            let source = materials.id_of("Source").expect("literal source exists");
            let targets = [
                None,
                Some(source),
                Some(materials.id_of("Solid").expect("literal Solid exists")),
                Some(materials.id_of("Rock").expect("literal Rock exists")),
            ];
            let mask = targets
                .into_iter()
                .enumerate()
                .fold(0u32, |mask, (bit, target)| {
                    if materials
                        .reaction_for_event(Some(source), target, MaterialInteractionEvent::PxsMove)
                        .user_defined
                    {
                        mask | (1u32 << bit)
                    } else {
                        mask
                    }
                });
            expect_eq(
                &format!("custom_reaction_overlay.literal_keyword[{inverse}]"),
                index,
                "mask",
                i(case, "mask"),
                i64::from(mask),
            );
        }

        let ordering_library = MaterialLibrary::parse(
            r#"
            [Material]
            Name=Source
            Density=10

            [Reaction]
            Type=Poof
            TargetSpec=All

            [Reaction]
            Type=Insert
            TargetSpec=Target

            [Material]
            Name=Target
            Density=50
            "#,
        )
        .expect("same-source ordering materials parse");
        let ordering = MaterialSet::from_resource_library(&ordering_library);
        let source = ordering.id_of("Source").expect("ordering source exists");
        let target = ordering.id_of("Target").expect("ordering target exists");
        let got_target = reaction_name(
            ordering
                .reaction_for_event(
                    Some(source),
                    Some(target),
                    MaterialInteractionEvent::PxsMove,
                )
                .kind,
        );
        let got_sky = reaction_name(
            ordering
                .reaction_for_event(Some(source), None, MaterialInteractionEvent::PxsMove)
                .kind,
        );
        assert_eq!(
            custom["ordering"]["same_source_target"]
                .as_str()
                .expect("same-source target tag"),
            got_target,
            "PARITY DIVERGENCE in `custom_reaction_overlay.ordering` field `same_source_target`",
        );
        assert_eq!(
            custom["ordering"]["same_source_sky"]
                .as_str()
                .expect("same-source sky tag"),
            got_sky,
            "PARITY DIVERGENCE in `custom_reaction_overlay.ordering` field `same_source_sky`",
        );

        let reverse_library = MaterialLibrary::parse(
            r#"
            [Material]
            Name=First
            Density=10

            [Reaction]
            Type=Poof
            TargetSpec=Later

            [Material]
            Name=Later
            Density=50

            [Reaction]
            Type=Corrode
            TargetSpec=First
            Reverse=1
            "#,
        )
        .expect("reverse collision materials parse");
        let reverse = MaterialSet::from_resource_library(&reverse_library);
        let first = reverse.id_of("First").expect("First exists");
        let later = reverse.id_of("Later").expect("Later exists");
        let got_reverse = reaction_name(
            reverse
                .reaction_for_event(Some(first), Some(later), MaterialInteractionEvent::PxsMove)
                .kind,
        );
        assert_eq!(
            custom["ordering"]["reverse_collision"]
                .as_str()
                .expect("reverse collision tag"),
            got_reverse,
            "PARITY DIVERGENCE in `custom_reaction_overlay.ordering` field `reverse_collision`",
        );

        let literal_sky_library = MaterialLibrary::parse(
            r#"
            [Material]
            Name=Source
            Density=10

            [Reaction]
            Type=Convert
            TargetSpec=Target
            ConvertMat=sKy

            [Material]
            Name=Target
            Density=50

            [Material]
            Name=Sky
            Density=0
            "#,
        )
        .expect("literal Sky ConvertMat materials parse");
        let literal_sky = MaterialSet::from_resource_library(&literal_sky_library);
        let source = literal_sky
            .id_of("Source")
            .expect("literal Sky source exists");
        let target = literal_sky
            .id_of("Target")
            .expect("literal Sky target exists");
        let crate::material::MaterialReactionKind::Convert {
            target: convert_target,
            ..
        } = literal_sky
            .reaction_for_event(
                Some(source),
                Some(target),
                MaterialInteractionEvent::PxsMove,
            )
            .kind
        else {
            panic!("literal Sky fixture must install Convert");
        };
        let convert_target_name = convert_target
            .and_then(|id| literal_sky.get_by_id(id))
            .map(crate::material::Material::name)
            .unwrap_or("none");
        assert_eq!(
            custom["resolution"]["convert_literal_sky"]
                .as_str()
                .expect("literal Sky oracle target"),
            convert_target_name,
            "PARITY DIVERGENCE in `custom_reaction_overlay.resolution` field `convert_literal_sky`",
        );

        let quoted = native_material_set(&[
            (
                "ConvertSource.c4m",
                b"[Material]\r\nName=ConvertSource\r\nDensity=10\r\n\r\n[Reaction]\r\nType=Convert\r\nTargetSpec=ConvertTarget\r\nConvertMat=\"Water \"\r\n",
            ),
            (
                "TargetSource.c4m",
                b"[Material]\r\nName=TargetSource\r\nDensity=10\r\n\r\n[Reaction]\r\nType=Poof\r\nTargetSpec=\" Solid\"\r\n",
            ),
            (
                "ConvertTarget.c4m",
                b"[Material]\r\nName=ConvertTarget\r\nDensity=50\r\n",
            ),
            (
                "Water.c4m",
                b"[Material]\r\nName=Water\r\nDensity=25\r\n",
            ),
            (
                "Rock.c4m",
                b"[Material]\r\nName=Rock\r\nDensity=50\r\n",
            ),
        ]);
        let convert_source = quoted
            .id_of("ConvertSource")
            .expect("quoted convert source exists");
        let convert_target = quoted
            .id_of("ConvertTarget")
            .expect("quoted convert target exists");
        let crate::material::MaterialReactionKind::Convert {
            target: spaced_target,
            ..
        } = quoted
            .reaction_for_event(
                Some(convert_source),
                Some(convert_target),
                MaterialInteractionEvent::PxsMove,
            )
            .kind
        else {
            panic!("quoted ConvertMat fixture must install Convert");
        };
        let spaced_target_name = spaced_target
            .and_then(|id| quoted.get_by_id(id))
            .map(crate::material::Material::name)
            .unwrap_or("none");
        assert_eq!(
            custom["resolution"]["convert_trailing_space"]
                .as_str()
                .expect("spaced ConvertMat oracle target"),
            spaced_target_name,
            "PARITY DIVERGENCE in `custom_reaction_overlay.resolution` field `convert_trailing_space`",
        );

        let target_source = quoted
            .id_of("TargetSource")
            .expect("quoted TargetSpec source exists");
        let rock = quoted.id_of("Rock").expect("quoted TargetSpec Rock exists");
        let targets = [None, Some(target_source), Some(rock)];
        let quoted_target_mask =
            targets
                .into_iter()
                .enumerate()
                .fold(0u32, |mask, (bit, target)| {
                    if quoted
                        .reaction_for_event(
                            Some(target_source),
                            target,
                            MaterialInteractionEvent::PxsMove,
                        )
                        .user_defined
                    {
                        mask | (1u32 << bit)
                    } else {
                        mask
                    }
                });
        expect_eq(
            "custom_reaction_overlay.resolution",
            0,
            "target_leading_space_mask",
            i(&custom["resolution"], "target_leading_space_mask"),
            i64::from(quoted_target_mask),
        );

        let exec_mask = u(&custom["runtime"], "exec_mask");
        let runtime_library = MaterialLibrary::parse(&format!(
            r#"
            [Material]
            Name=Source
            Density=10

            [Reaction]
            Type=Poof
            TargetSpec=Target
            ExecMask={exec_mask}
            CheckSlide=1

            [Material]
            Name=Target
            Density=50
            "#,
        ))
        .expect("runtime custom-reaction materials parse");
        let runtime_set = MaterialSet::from_resource_library(&runtime_library);
        let source = runtime_set.id_of("Source").expect("runtime source exists");
        let target = runtime_set.id_of("Target").expect("runtime target exists");
        let events = [
            MaterialInteractionEvent::PxsPos,
            MaterialInteractionEvent::PxsMove,
            MaterialInteractionEvent::MassMove,
        ];
        let mut installed_mask = 0u32;
        let mut allowed_mask = 0u32;
        let mut reactions = Vec::new();
        for (bit, event) in events.into_iter().enumerate() {
            let reaction = runtime_set.reaction_for_event(Some(source), Some(target), event);
            if reaction.user_defined {
                installed_mask |= 1u32 << bit;
            }
            if reaction.kind == crate::material::MaterialReactionKind::Poof {
                allowed_mask |= 1u32 << bit;
            }
            reactions.push(reaction);
        }
        expect_eq(
            "custom_reaction_overlay.runtime",
            0,
            "installed_mask",
            i(&custom["runtime"], "installed_mask"),
            i64::from(installed_mask),
        );
        expect_eq(
            "custom_reaction_overlay.runtime",
            0,
            "allowed_mask",
            i(&custom["runtime"], "allowed_mask"),
            i64::from(allowed_mask),
        );
        assert_eq!(
            custom["runtime"]["check_slide"]
                .as_bool()
                .expect("runtime CheckSlide oracle value"),
            reactions[0].insertion_check,
            "PARITY DIVERGENCE in `custom_reaction_overlay.runtime` field `check_slide`",
        );
        assert_eq!(
            reactions[1].kind,
            crate::material::MaterialReactionKind::None,
            "masked PxsMove must be a user no-op, not the natural Insert",
        );
        assert!(reactions[1].user_defined);
        assert!(
            reactions[0].insertion_check && reactions[2].insertion_check,
            "allowed custom-reaction events retain CheckSlide=true",
        );
        expect_eq(
            "custom_reaction_overlay.runtime",
            0,
            "insert_check_calls",
            i(&custom["runtime"], "insert_check_calls"),
            i64::from(reactions[1].insertion_check),
        );
    }

    // 16b7. dig_free: `C4Landscape::DigFree` (C4Landscape.cpp:1023-1044) walks a
    //       circle row by row, and two of its details are easy to "tidy" into
    //       something that digs a different shape.
    //
    //       `iLineWidth` is declared OUTSIDE the row loop, and the bottom-edge
    //       pass reads it after the loop has ended — so the bottom edge is as
    //       wide as the LAST row was, not as wide as the circle. And a row whose
    //       half-width computes to 0 still digs one pixel, via the
    //       `+ (iLineWidth == 0)` bump that appears in the loop bound and again
    //       in the right-hand edge position.
    //
    //       `DigFreeSinglePix` (C4Landscape.h:255-259) clears its pixel only
    //       when it is DENSER than the neighbour toward `(dx, dy)`, so the edge
    //       passes do nothing inside a uniform block and bite exactly at a
    //       material boundary. The fixtures place those boundaries deliberately:
    //       sky below row 10 for the bottom edge, and an optional sky column for
    //       the side.
    //
    //       DigFree returns void, so the cleared pixels are the only thing
    //       either engine can be asked about — which is also what a desync would
    //       consist of. DigFreePix's unconditional instability probe is not
    //       compared here; it is pinned by
    //       `dig_free_pix_probes_even_undiggable_pixels` in mass_mover.rs.
    //
    //       `rad <= 0` is deliberately uncovered: C++ leaves `iLineWidth`
    //       uninitialised when the row loop never runs and the bottom pass then
    //       reads it, so there is no defined behaviour to pin. The port guards.
    for case in golden["dig_free"].as_array().unwrap() {
        let name = case["name"].as_str().unwrap_or("?");
        let label = format!("dig_free[{name}]");

        let library = clonk_resources::MaterialLibrary::parse(
            r#"
            [Material Vacuum]
            Name=Vacuum
            Density=0

            [Material Water]
            Name=Water
            Density=25
            DigFree=1

            [Material Sand]
            Name=Sand
            Density=50
            DigFree=1

            [Material Granite]
            Name=Granite
            Density=100
            DigFree=1
            "#,
        )
        .expect("dig free oracle materials parse");

        const WDT: u32 = 16;
        const HGT: u32 = 12;
        const GRANITE: u8 = 3;

        let sky_from_row = i(case, "sky_from_row") as i32;
        let sky_column = i(case, "sky_column") as i32;

        let mut bytes = vec![0u8; WDT as usize * HGT as usize];
        for gy in 0..HGT as usize {
            if sky_from_row >= 0 && gy >= sky_from_row as usize {
                continue;
            }
            for gx in 0..WDT as usize {
                bytes[gy * WDT as usize + gx] = GRANITE;
            }
        }
        if sky_column >= 0 {
            for gy in 0..HGT as usize {
                bytes[gy * WDT as usize + sky_column as usize] = 0;
            }
        }

        let mut densities = vec![0; 128];
        densities[1] = 25;
        densities[2] = 50;
        densities[GRANITE as usize] = 100;
        let mut material_names = vec![None; 128];
        material_names[1] = Some("Water".to_string());
        material_names[2] = Some("Sand".to_string());
        material_names[GRANITE as usize] = Some("Granite".to_string());
        let grid = PixelGrid::new(WDT, HGT, bytes, densities, material_names, vec![None; 128]);

        let mut engine = Engine::with_seed(0);
        engine.configure_materials_from_library(&library);
        engine.set_physics(PhysicsSettings::new(100, 1000, -1000));
        let mut landscape = Landscape::flat(WDT, HGT as i32);
        landscape.set_pixel_grid(grid);
        landscape.set_world_height(HGT as i32);
        engine.set_landscape(landscape);

        let before: Vec<Option<crate::material::MaterialId>> = (0..HGT as i32)
            .flat_map(|gy| (0..WDT as i32).map(move |gx| (gx, gy)))
            .map(|(gx, gy)| {
                engine
                    .landscape()
                    .and_then(|landscape| landscape.border_material_at(gx, gy))
            })
            .collect();

        engine.execute_dig_circle_operation(
            crate::Vector2::new(i(case, "tx") as i32, i(case, "ty") as i32),
            i(case, "rad") as i32,
            false,
            None,
        );

        // Emitted in row-major order as `y,x` pairs, which is the order this
        // scan produces.
        let changed: Vec<String> = (0..HGT as i32)
            .flat_map(|gy| (0..WDT as i32).map(move |gx| (gx, gy)))
            .enumerate()
            .filter_map(|(index, (gx, gy))| {
                let after = engine
                    .landscape()
                    .and_then(|landscape| landscape.border_material_at(gx, gy));
                (after != before[index]).then(|| format!("{gy},{gx}"))
            })
            .collect();

        expect_eq(
            &label,
            0,
            "changed_count",
            i(case, "changed_count"),
            changed.len() as i64,
        );
        assert_eq!(
            case["changed"].as_str().unwrap_or(""),
            changed.join(";"),
            "PARITY DIVERGENCE in `{label}` field `changed`",
        );
    }

    // 16b9. dig_free_mat: `C4Landscape::DigFreeMat`
    //       (C4Landscape.cpp:1012-1020) rejects an invalid material before its
    //       x-major/y-minor rectangle walk, compares the exact
    //       `Pix2Mat[GetPix]` material, and hands only matches to DigFreePix.
    //       The exact read matters for a nonzero texmap slot that resolves to
    //       MNone: the derived column material must not make it a match
    //       (C4Landscape.h:173-176; C4Wrappers.h:120-128).
    //
    //       DigFreePix clears only a material with DigFree set, but its trailing
    //       CheckInstabilityRange runs for every match (C4Landscape.cpp:918-925).
    //       Thus a resolved DigFree=0 target leaves Surface8 untouched while
    //       still exposing the rectangle order. ClearPix preserves IFT and
    //       writes the default Tunnel byte for an IFT target
    //       (C4Landscape.cpp:881-888), so every raw byte is compared.
    for case in golden["dig_free_mat"].as_array().unwrap() {
        let name = case["name"].as_str().unwrap_or("?");
        let label = format!("dig_free_mat[{name}]");
        let library = clonk_resources::MaterialLibrary::parse(
            r#"
            [Material Vacuum]
            Name=Vacuum
            Density=0

            [Material Water]
            Name=Water
            Density=25
            DigFree=1

            [Material Sand]
            Name=Sand
            Density=50
            DigFree=1

            [Material Granite]
            Name=Granite
            Density=100
            DigFree=1

            [Material Tunnel]
            Name=Tunnel
            Density=0

            [Material Undiggable]
            Name=Undiggable
            Density=100
            DigFree=0
            Instable=1
            "#,
        )
        .expect("DigFreeMat oracle materials parse");

        let width = i(case, "width") as u32;
        let height = i(case, "height") as u32;
        let initial_bytes = case["initial_bytes"]
            .as_array()
            .expect("dig_free_mat.initial_bytes is an array")
            .iter()
            .map(|byte| byte.as_u64().expect("DigFreeMat pixel byte") as u8)
            .collect::<Vec<_>>();
        let mut densities = vec![0; 128];
        densities[1] = 25;
        densities[2] = 50;
        densities[3] = 100;
        densities[5] = 100;
        densities[6] = 100;
        // Slot 7 is deliberately unresolved in the normal case while still
        // carrying solid density, so the column fallback would answer Granite.
        densities[7] = 100;
        let mut material_names = vec![None; 128];
        material_names[1] = Some("Water".to_string());
        material_names[2] = Some("Sand".to_string());
        material_names[3] = Some("Granite".to_string());
        material_names[4] = Some("Tunnel".to_string());
        material_names[5] = Some("Granite".to_string());
        material_names[6] = Some("Undiggable".to_string());
        material_names[7] = Some("Ghost".to_string());
        let grid = PixelGrid::new(
            width,
            height,
            initial_bytes,
            densities,
            material_names,
            vec![None; 128],
        );

        let mut engine = Engine::with_seed(0);
        engine.configure_materials_from_library(&library);
        let mut landscape = Landscape::flat(width, height as i32);
        landscape.set_pixel_grid(grid);
        landscape.set_world_height(height as i32);
        engine.set_landscape(landscape);

        let granite = engine
            .materials
            .id_of("Granite")
            .expect("DigFreeMat Granite exists");
        engine
            .landscape
            .as_mut()
            .expect("DigFreeMat landscape exists")
            .set_default_solid_material(Some(granite));

        let material_index = i(case, "material") as usize;
        let material =
            crate::material::MaterialId::new(material_index).expect("DigFreeMat material id fits");
        if engine.materials.get_by_id(material).is_none() {
            // Reproduce the stale Pix2Mat integer in the C++ invalid-material
            // row. Engine::set_landscape normally resolves only loaded names,
            // so this test-only remap deliberately carries the invalid id.
            let loaded = engine.materials.clone();
            engine
                .landscape
                .as_mut()
                .expect("DigFreeMat landscape exists")
                .resolve_grid_materials(|name| {
                    if name == "Ghost" {
                        Some(material)
                    } else {
                        loaded.id_of(name)
                    }
                });
        }

        crate::mass_mover::MASS_MOVER_INSTABILITY_PROBES.with(|probes| probes.borrow_mut().clear());
        engine.dig_free_material_rect(
            crate::Vector2::new(i(case, "tx") as i32, i(case, "ty") as i32),
            i(case, "wdt") as i32,
            i(case, "hgt") as i32,
            material,
        );

        let expected_bytes = case["final_bytes"]
            .as_array()
            .expect("dig_free_mat.final_bytes is an array");
        let landscape = engine.landscape().expect("DigFreeMat landscape remains");
        for (index, expected) in expected_bytes.iter().enumerate() {
            let x = index as i32 % width as i32;
            let y = index as i32 / width as i32;
            expect_eq(
                &label,
                index,
                "surface8_byte",
                expected.as_i64().expect("golden DigFreeMat pixel byte"),
                i64::from(
                    landscape
                        .grid_byte_at(x, y)
                        .unwrap_or_else(|| panic!("DigFreeMat pixel ({x},{y}) exists")),
                ),
            );
        }

        let probes =
            crate::mass_mover::MASS_MOVER_INSTABILITY_PROBES.with(|probes| probes.borrow().clone());
        let probe_stride = i(case, "probe_stride") as usize;
        assert_ne!(probe_stride, 0, "{label} probe stride is nonzero");
        assert_eq!(
            probes.len() % probe_stride,
            0,
            "PARITY DIVERGENCE in `{label}` field `probe_stride`: {} lower-level probes cannot be grouped by C++ stride {probe_stride}",
            probes.len(),
        );
        let direct_probes = probes
            .chunks(probe_stride)
            .map(|chunk| format!("{},{}", chunk[0].0, chunk[0].1))
            .collect::<Vec<_>>();
        expect_eq(
            &label,
            0,
            "probe_count",
            i(case, "probe_count"),
            direct_probes.len() as i64,
        );
        assert_eq!(
            case["probe_order"].as_str().unwrap_or(""),
            direct_probes.join(";"),
            "PARITY DIVERGENCE in `{label}` field `probe_order`",
        );
    }

    // 16b6. extract_material: `C4Landscape::ExtractMaterial` (C4Landscape.cpp:
    //       1191-1199) and the `FindMatTop` walk it depends on
    //       (C4Landscape.cpp:1161-1189).
    //
    //       ExtractMaterial does NOT clear the pixel it was handed. It reads
    //       the material there, walks FindMatTop up that material's own column,
    //       and clears the pixel it ends on — so extracting from the middle of
    //       a column removes its TOP. A port that cleared the requested pixel
    //       would return the very same material while taking it from the wrong
    //       place, which no return-code check can see. The cleared coordinates
    //       are therefore what is compared.
    //
    //       FindMatTop's own loop is indented in a way that misreads: `if
    //       (fLeft)` carries no braces, so it governs only the left if/else-if
    //       chain and the `if (fRight)` below it is an INDEPENDENT statement.
    //       Both sides are examined in the same `cslide` iteration, left first,
    //       and the `break` leaves `cslide` at the matching distance — which is
    //       how far the slide then moves.
    for case in golden["extract_material"].as_array().unwrap() {
        let name = case["name"].as_str().unwrap_or("?");
        let label = format!("extract_material[{name}]");

        // Granite's MaxSlide is 0 and Water's is 4, so the same shape makes one
        // walk straight up and the other step sideways.
        let library = clonk_resources::MaterialLibrary::parse(
            r#"
            [Material Vacuum]
            Name=Vacuum
            Density=0

            [Material Water]
            Name=Water
            Density=25
            MaxSlide=4

            [Material Sand]
            Name=Sand
            Density=50
            MaxSlide=2

            [Material Granite]
            Name=Granite
            Density=100
            "#,
        )
        .expect("extract material oracle materials parse");

        const WDT: u32 = 16;
        const HGT: u32 = 12;

        let mat = i(case, "mat") as u8;
        let (x0, x1) = (i(case, "x0") as usize, i(case, "x1") as usize);
        let (y_top, y_bottom) = (i(case, "y_top") as usize, i(case, "y_bottom") as usize);
        let step_x = i(case, "step_x") as i32;

        let mut bytes = vec![0u8; WDT as usize * HGT as usize];
        if mat != 0 {
            for gx in x0..=x1 {
                for gy in y_top..=y_bottom {
                    bytes[gy * WDT as usize + gx] = mat;
                }
            }
            // The tie case wants a step on BOTH sides at the same distance.
            if x0 != x1 {
                bytes[(y_top - 1) * WDT as usize + x0] = mat;
                bytes[(y_top - 1) * WDT as usize + x1] = mat;
            }
            if step_x >= 0 {
                bytes[y_top * WDT as usize + step_x as usize] = mat;
                bytes[(y_top - 1) * WDT as usize + step_x as usize] = mat;
            }
        }

        let mut densities = vec![0; 128];
        densities[1] = 25;
        densities[2] = 50;
        densities[3] = 100;
        let mut material_names = vec![None; 128];
        material_names[1] = Some("Water".to_string());
        material_names[2] = Some("Sand".to_string());
        material_names[3] = Some("Granite".to_string());
        let grid = PixelGrid::new(WDT, HGT, bytes, densities, material_names, vec![None; 128]);

        let mut engine = Engine::with_seed(0);
        engine.configure_materials_from_library(&library);
        engine.set_physics(PhysicsSettings::new(100, 1000, -1000));
        let mut landscape = Landscape::flat(WDT, HGT as i32);
        landscape.set_pixel_grid(grid);
        landscape.set_world_height(HGT as i32);
        engine.set_landscape(landscape);

        let (fx, fy) = (i(case, "fx") as i32, i(case, "fy") as i32);
        let before = landscape_material_snapshot(&engine, WDT, HGT);
        clear_instability_probe_trace();
        let extracted = engine.extract_material(fx, fy);
        let changes = landscape_material_changes(&before, &engine, WDT, HGT);
        let cleared = changes
            .iter()
            .find(|(_, _, before, after)| before.is_some() && after.is_none());
        let lower_probes = take_instability_probe_trace();
        const PROBE_STRIDE: usize = 5;
        assert_eq!(
            lower_probes.len() % PROBE_STRIDE,
            0,
            "PARITY DIVERGENCE in `{label}`: ExtractMaterial lower-level instability probes do not form C++ CheckInstabilityRange calls",
        );
        let direct_probes = lower_probes
            .chunks(PROBE_STRIDE)
            .map(|chunk| (chunk[0].0, chunk[0].1))
            .collect::<Vec<_>>();

        expect_eq(
            &label,
            0,
            "result",
            i(case, "result"),
            extracted.map_or(-1, |material| material.index() as i64),
        );
        expect_eq(
            &label,
            0,
            "cleared",
            i(case, "cleared"),
            i64::from(cleared.is_some()),
        );
        expect_eq(
            &label,
            0,
            "clear_x",
            i(case, "clear_x"),
            cleared.map_or(-1, |(x, _, _, _)| i64::from(*x)),
        );
        expect_eq(
            &label,
            0,
            "clear_y",
            i(case, "clear_y"),
            cleared.map_or(-1, |(_, y, _, _)| i64::from(*y)),
        );
        expect_eq(
            &label,
            0,
            "probes",
            i(case, "probes"),
            direct_probes.len() as i64,
        );
        expect_eq(
            &label,
            0,
            "probe_x",
            i(case, "probe_x"),
            direct_probes.first().map_or(-1, |(x, _)| i64::from(*x)),
        );
        expect_eq(
            &label,
            0,
            "probe_y",
            i(case, "probe_y"),
            direct_probes.first().map_or(-1, |(_, y)| i64::from(*y)),
        );
    }

    // 16b5. insert_material: `C4Landscape::InsertMaterial` (C4Landscape.cpp:
    //       1201-1269) — the landscape-mutation half of a material reaction,
    //       and the destination every `Insert` arm above only recorded a call
    //       to.
    //
    //       Several of its decisions read as typos until you check them against
    //       the source, and each is one a port can plausibly "clean up": a
    //       density-0 material returns **true** having done nothing; the bounds
    //       test accepts `ty == Height` while stopping `tx` at `Width - 1`; the
    //       non-push-pull climb applies its primitive slide as two INDEPENDENT
    //       `if`s, so a row with both neighbours free moves left and straight
    //       back; and insert-thrust re-inserts the displaced material
    //       RECURSIVELY one row up, after the new pixel is already written.
    //
    //       What is compared is the landscape delta rather than a count of
    //       SetPix calls: it is an observable both engines produce, and it is
    //       what a desync would actually consist of. Every case is paired with
    //       one that differs in a single input, so a flattened branch changes
    //       the delta rather than merely the return value.
    for case in golden["insert_material"].as_array().unwrap() {
        let name = case["name"].as_str().unwrap_or("?");
        let label = format!("insert_material[{name}]");

        // Granite matches the closed border's C4M_Vehicle density, so
        // inserting it enters the climb loop; Sand sits between Water and the
        // floor, so the floor refuses it while it still displaces Water.
        let library = clonk_resources::MaterialLibrary::parse(
            r#"
            [Material Vacuum]
            Name=Vacuum
            Density=0

            [Material Water]
            Name=Water
            Density=25
            MaxSlide=4

            [Material Sand]
            Name=Sand
            Density=50
            MaxSlide=2

            [Material Granite]
            Name=Granite
            Density=100
            "#,
        )
        .expect("insert material oracle materials parse");

        const WDT: u32 = 16;
        const HGT: u32 = 12;
        const GRANITE: u8 = 3;
        const WATER: u8 = 1;

        let gap_x = i(case, "gap_x") as i32;
        let water_row = case["water_row"].as_bool().unwrap_or(false);

        // Sky above row 10, Granite floor on rows 10 and 11.
        let mut bytes = vec![0u8; WDT as usize * HGT as usize];
        for gy in 10..HGT as usize {
            for gx in 0..WDT as usize {
                bytes[gy * WDT as usize + gx] = GRANITE;
            }
        }
        if gap_x >= 0 {
            for gy in 10..HGT as usize {
                bytes[gy * WDT as usize + gap_x as usize] = 0;
            }
        }
        if water_row {
            for gx in 0..WDT as usize {
                bytes[9 * WDT as usize + gx] = WATER;
            }
        }

        let mut densities = vec![0; 128];
        densities[1] = 25;
        densities[2] = 50;
        densities[GRANITE as usize] = 100;
        let mut material_names = vec![None; 128];
        material_names[1] = Some("Water".to_string());
        material_names[2] = Some("Sand".to_string());
        material_names[GRANITE as usize] = Some("Granite".to_string());
        let grid = PixelGrid::new(
            WDT,
            HGT,
            bytes.clone(),
            densities,
            material_names,
            vec![None; 128],
        );

        let mut engine = Engine::with_seed(0);
        engine.configure_materials_from_library(&library);
        engine.set_physics(PhysicsSettings::new(100, 1000, -1000));
        engine.set_landscape_insert_thrust(case["insert_thrust"].as_bool().unwrap_or(false));
        let mut landscape = Landscape::flat(WDT, HGT as i32);
        landscape.set_pixel_grid(grid);
        landscape.set_world_height(HGT as i32);
        engine.set_landscape(landscape);

        let before: Vec<Option<crate::material::MaterialId>> = (0..HGT as i32)
            .flat_map(|gy| (0..WDT as i32).map(move |gx| (gx, gy)))
            .map(|(gx, gy)| {
                engine
                    .landscape()
                    .and_then(|landscape| landscape.border_material_at(gx, gy))
            })
            .collect();

        let result = engine.insert_material(
            crate::material::MaterialId::new(i(case, "mat") as usize)
                .expect("oracle insert material"),
            i(case, "tx") as i32,
            i(case, "ty") as i32,
            i(case, "vx") as i32,
            i(case, "vy") as i32,
        );

        // The delta is emitted in row-major order as `y,x,mat` triples, which
        // is the order this scan produces.
        let changed: Vec<String> = (0..HGT as i32)
            .flat_map(|gy| (0..WDT as i32).map(move |gx| (gx, gy)))
            .enumerate()
            .filter_map(|(index, (gx, gy))| {
                let after = engine
                    .landscape()
                    .and_then(|landscape| landscape.border_material_at(gx, gy));
                (after != before[index]).then(|| {
                    format!(
                        "{gy},{gx},{}",
                        after.map(|id| id.index()).unwrap_or_default()
                    )
                })
            })
            .collect();

        let pxs: Vec<&crate::pxs::Pxs> = engine.pxs_system.iter().collect();

        expect_eq(
            &label,
            0,
            "result",
            i64::from(case["result"].as_bool().unwrap_or(false)),
            i64::from(result),
        );
        expect_eq(
            &label,
            0,
            "changed_count",
            i(case, "changed_count"),
            changed.len() as i64,
        );
        assert_eq!(
            case["changed"].as_str().unwrap_or(""),
            changed.join(";"),
            "PARITY DIVERGENCE in `{label}` field `changed`",
        );
        expect_eq(
            &label,
            0,
            "pxs_created",
            i(case, "pxs_created"),
            pxs.len() as i64,
        );
        expect_eq(
            &label,
            0,
            "pxs_x",
            i(case, "pxs_x"),
            pxs.first().map_or(-1, |pixel| i64::from(fixtoi(pixel.x))),
        );
        expect_eq(
            &label,
            0,
            "pxs_y",
            i(case, "pxs_y"),
            pxs.first().map_or(-1, |pixel| i64::from(fixtoi(pixel.y))),
        );
        expect_eq(
            &label,
            0,
            "pxs_mat",
            i(case, "pxs_mat"),
            pxs.first().map_or(-1, |pixel| pixel.mat.raw() as i64),
        );
        expect_eq(
            &label,
            0,
            "pxs_x_raw",
            i(case, "pxs_x_raw"),
            pxs.first().map_or(-1, |pixel| pixel.x.val() as i64),
        );
        expect_eq(
            &label,
            0,
            "pxs_y_raw",
            i(case, "pxs_y_raw"),
            pxs.first().map_or(-1, |pixel| pixel.y.val() as i64),
        );
        expect_eq(
            &label,
            0,
            "pxs_xdir",
            i(case, "pxs_xdir"),
            pxs.first().map_or(-1, |pixel| pixel.xdir.val() as i64),
        );
        expect_eq(
            &label,
            0,
            "pxs_ydir",
            i(case, "pxs_ydir"),
            pxs.first().map_or(-1, |pixel| pixel.ydir.val() as i64),
        );
    }

    // 16b4. corrode_arm: `mrfCorrode`'s movement arm (C4Material.cpp:691-745),
    //       whose draw ledger is conditional in three separate places.
    //
    //       A non-user reaction rolls `Random(100) < Corrosive` and only then
    //       `Random(100) < Corrode` — C++'s `&&` short-circuits, so a failed
    //       first roll spends ONE draw, not two. A user reaction spends one
    //       draw against its own CorrosionRate instead. And `!Random(5)` opens
    //       the smoke, with `Random(3)` for its level drawn ONLY when it does,
    //       before `!Random(20)` decides the sound.
    //
    //       A port that evaluated both halves eagerly, or drew the smoke level
    //       unconditionally, would clear the same pixel and desynchronise every
    //       draw after it. That is why the count is compared beside the
    //       verdict, and why the short-circuit row is the one to watch.
    for case in golden["corrode_arm"].as_array().unwrap() {
        let name = case["name"].as_str().unwrap_or("?");
        let label = format!("corrode_arm[{name}]");

        // Lava is the corrosive (Corrosive 100 makes its half certain), Granite
        // the corrodible floor (Corrode 100 makes the second half certain), and
        // Water's Corrosive 0 fails the FIRST half so the second is never
        // reached.
        let library = clonk_resources::MaterialLibrary::parse(
            r#"
            [Material Vacuum]
            Name=Vacuum
            Density=0

            [Material Water]
            Name=Water
            Density=25
            SplashRate=1
            MaxSlide=4

            [Material Lava]
            Name=Lava
            Density=25
            Incindiary=1
            Corrosive=100
            MaxSlide=4

            [Material Granite]
            Name=Granite
            Density=50
            Corrode=100
            "#,
        )
        .expect("corrode arm oracle materials parse");

        const WDT: u32 = 16;
        const HGT: u32 = 12;
        const GRANITE: u8 = 3;
        let px = i(case, "x0") as i32;
        let py = i(case, "y0") as i32;

        let mut bytes = vec![0u8; WDT as usize * HGT as usize];
        for gy in 0..HGT as usize {
            for gx in 0..WDT as usize {
                if gx != px as usize {
                    bytes[gy * WDT as usize + gx] = GRANITE;
                }
            }
        }
        for gx in 0..WDT as usize {
            bytes[10 * WDT as usize + gx] = GRANITE;
        }
        // Successful corrosion needs a real pixel to clear. On the rows where
        // C++ instead records an InsertMaterial call, leave the target cell as
        // sky so the port's real insertion becomes observable rather than
        // being refused by the denser Granite collision material passed below.
        if i(case, "inserted") == 0 {
            bytes[py as usize * WDT as usize + px as usize] = GRANITE;
        }

        let mut densities = vec![0; 128];
        densities[1] = 25;
        densities[2] = 25;
        densities[GRANITE as usize] = 50;
        let mut material_names = vec![None; 128];
        material_names[1] = Some("Water".to_string());
        material_names[2] = Some("Lava".to_string());
        material_names[GRANITE as usize] = Some("Granite".to_string());
        let grid = PixelGrid::new(WDT, HGT, bytes, densities, material_names, vec![None; 128]);

        let mut engine = Engine::with_seed(0);
        engine.configure_materials_from_library(&library);
        register_smoke_probe(&mut engine);
        engine.set_physics(PhysicsSettings::new(100, 1000, -1000));
        let mut landscape = Landscape::flat(WDT, HGT as i32);
        landscape.set_pixel_grid(grid);
        landscape.set_world_height(HGT as i32);
        engine.set_landscape(landscape);
        engine.rng = LcgRng::new(i(case, "seed") as u32);
        engine.rng.randomize3();
        let draws_before = engine.rng.count;

        let material_before = engine
            .landscape()
            .and_then(|landscape| landscape.border_material_at(px, py));

        let user_defined = case["user_defined"].as_bool().unwrap_or(false);
        let pxs_mat = i(case, "pxs_mat") as i32;
        let reaction = crate::material::MaterialReaction {
            kind: crate::material::MaterialReactionKind::Corrode {
                // The non-user roll reads the two material properties; the user
                // roll ignores them for its own rate.
                corrosive_strength: if pxs_mat == 2 { 100 } else { 0 },
                corrode_resistance: 100,
                corrosion_probability: user_defined.then(|| i(case, "corrosion_rate") as i32),
            },
            user_defined,
            insertion_check: true,
        };
        let mut pixel = crate::pxs::Pxs {
            mat: crate::material::MaterialId::new(pxs_mat as usize)
                .expect("oracle pxs material")
                .into(),
            x: itofix(px),
            y: itofix(py),
            xdir: C4Fixed::from_raw(i(case, "xdir0") as i32),
            ydir: C4Fixed::from_raw(i(case, "ydir0") as i32),
        };
        let (mut x, mut y) = (px, py);
        let mut pos_changed = false;
        clear_instability_probe_trace();
        let handled = engine.execute_pxs_reaction(
            reaction,
            &mut x,
            &mut y,
            px,
            py,
            &mut pixel,
            crate::material::MaterialId::new(i(case, "ls_mat") as usize),
            match i(case, "event") {
                0 => MaterialInteractionEvent::PxsPos,
                1 => MaterialInteractionEvent::PxsMove,
                _ => MaterialInteractionEvent::MassMove,
            },
            &mut pos_changed,
        );

        // The clear is observable as the landscape pixel going empty.
        let material_after = engine
            .landscape()
            .and_then(|landscape| landscape.border_material_at(px, py));
        let cleared = i64::from(material_before.is_some() && material_after != material_before);
        let lower_probes = take_instability_probe_trace();
        const PROBE_STRIDE: usize = 5;
        assert_eq!(
            lower_probes.len() % PROBE_STRIDE,
            0,
            "PARITY DIVERGENCE in `{label}`: Corrode lower-level instability probes do not form C++ CheckInstabilityRange calls",
        );
        let instability_probes = lower_probes.len() / PROBE_STRIDE;
        let sounds = engine
            .pending_audio
            .iter()
            .filter(|command| {
                matches!(
                    command,
                    crate::AudioCommand::PlaySoundAt { name, .. } if name == "Corrode"
                )
            })
            .count();
        let inserted = i64::from(
            material_before.map(Into::into) != Some(pixel.mat)
                && material_after.map(Into::into) == Some(pixel.mat),
        );

        expect_eq(
            &label,
            0,
            "handled",
            i64::from(case["handled"].as_bool().unwrap_or(false)),
            i64::from(handled),
        );
        expect_eq(&label, 0, "cleared", i(case, "cleared"), cleared);
        expect_eq(&label, 0, "x", i(case, "x"), i64::from(x));
        expect_eq(&label, 0, "y", i(case, "y"), i64::from(y));
        expect_eq(&label, 0, "xdir", i(case, "xdir"), pixel.xdir.val() as i64);
        expect_eq(&label, 0, "ydir", i(case, "ydir"), pixel.ydir.val() as i64);
        expect_eq(
            &label,
            0,
            "pos_changed",
            i64::from(case["pos_changed"].as_bool().unwrap_or(false)),
            i64::from(pos_changed),
        );
        expect_eq(
            &label,
            0,
            "instability_probes",
            i(case, "instability_probes"),
            instability_probes as i64,
        );
        expect_eq(
            &label,
            0,
            "smoke",
            i(case, "smoke"),
            smoke_probe_count(&engine),
        );
        expect_eq(&label, 0, "sounds", i(case, "sounds"), sounds as i64);
        expect_eq(&label, 0, "inserted", i(case, "inserted"), inserted);
        expect_eq(
            &label,
            0,
            "draws",
            i(case, "draws"),
            i64::from(engine.rng.count - draws_before),
        );
        expect_rng_state(&label, case, &engine.rng);
    }

    // 16b3. poof_arm: `mrfPoof`'s movement arm (C4Material.cpp:663-688).
    //
    //       `material_poof_reaction` runs the position and mass-move arms and
    //       pins their extraction plus both Rnd3 effects, but every row is
    //       `handled: 1`. The movement-only insertion check is where the
    //       **unhandled** outcome lives, so it needs this separate matrix.
    //
    //       `meePXSMove` is where the unhandled outcome lives. A non-user
    //       reaction runs `mrfInsertCheck` first, and a splash that prevents
    //       the interaction returns having extracted nothing and drawn nothing:
    //       both draws are downstream of the check, so a port that extracted or
    //       drew before checking would desynchronise everything after it. A
    //       user reaction runs that check in `mrfUserCheck` instead, at the top
    //       of the function, and the body's own call is gated off — which the
    //       draw count catches, because running it twice doubles the draws.
    for case in golden["poof_arm"].as_array().unwrap() {
        let name = case["name"].as_str().unwrap_or("?");
        let label = format!("poof_arm[{name}]");

        let library = clonk_resources::MaterialLibrary::parse(
            r#"
            [Material Vacuum]
            Name=Vacuum
            Density=0

            [Material Water]
            Name=Water
            Density=25
            SplashRate=1
            MaxSlide=4

            [Material Lava]
            Name=Lava
            Density=25
            Incindiary=1
            MaxSlide=4

            [Material Granite]
            Name=Granite
            Density=50
            "#,
        )
        .expect("poof arm oracle materials parse");

        const WDT: u32 = 16;
        const HGT: u32 = 12;
        const GRANITE: u8 = 3;
        let px = i(case, "x0") as i32;
        let py = i(case, "y0") as i32;

        let mut bytes = vec![0u8; WDT as usize * HGT as usize];
        for gy in 0..HGT as usize {
            for gx in 0..WDT as usize {
                if gx != px as usize {
                    bytes[gy * WDT as usize + gx] = GRANITE;
                }
            }
        }
        for gx in 0..WDT as usize {
            bytes[10 * WDT as usize + gx] = GRANITE;
        }
        // The arm extracts at the LANDSCAPE position, which the oracle passes
        // as (iLSPosX, iLSPosY) = the pixel's own cell. Put a material there so
        // the extraction is observable as the pixel going empty.
        bytes[py as usize * WDT as usize + px as usize] = GRANITE;

        let mut densities = vec![0; 128];
        densities[1] = 25;
        densities[2] = 25;
        densities[GRANITE as usize] = 50;
        let mut material_names = vec![None; 128];
        material_names[1] = Some("Water".to_string());
        material_names[2] = Some("Lava".to_string());
        material_names[GRANITE as usize] = Some("Granite".to_string());
        let grid = PixelGrid::new(WDT, HGT, bytes, densities, material_names, vec![None; 128]);

        let mut engine = Engine::with_seed(0);
        engine.configure_materials_from_library(&library);
        engine.set_physics(PhysicsSettings::new(100, 1000, -1000));
        let mut landscape = Landscape::flat(WDT, HGT as i32);
        landscape.set_pixel_grid(grid);
        landscape.set_world_height(HGT as i32);
        engine.set_landscape(landscape);
        engine.rng = LcgRng::new(i(case, "seed") as u32);
        engine.rng.randomize3();
        let draws_before = engine.rng.count;

        let before = landscape_material_snapshot(&engine, WDT, HGT);

        let reaction = crate::material::MaterialReaction {
            kind: crate::material::MaterialReactionKind::Poof,
            user_defined: case["user_defined"].as_bool().unwrap_or(false),
            insertion_check: true,
        };
        let mut pixel = crate::pxs::Pxs {
            mat: crate::pxs::PxsMaterial::from_raw(i(case, "pxs_mat") as i32),
            x: itofix(px),
            y: itofix(py),
            xdir: C4Fixed::from_raw(i(case, "xdir0") as i32),
            ydir: C4Fixed::from_raw(i(case, "ydir0") as i32),
        };
        let (mut x, mut y) = (px, py);
        let mut pos_changed = false;
        let handled = engine.execute_pxs_reaction(
            reaction,
            &mut x,
            &mut y,
            px,
            py,
            &mut pixel,
            crate::material::MaterialId::new(i(case, "ls_mat") as usize),
            match i(case, "event") {
                0 => MaterialInteractionEvent::PxsPos,
                1 => MaterialInteractionEvent::PxsMove,
                _ => MaterialInteractionEvent::MassMove,
            },
            &mut pos_changed,
        );

        // The extraction is observable as one landscape pixel going empty;
        // deriving its coordinates from the complete delta keeps the oracle's
        // ExtractMaterial recorder fields load-bearing.
        let changes = landscape_material_changes(&before, &engine, WDT, HGT);
        let extracted = changes
            .iter()
            .filter(|(_, _, before, after)| before.is_some() && after.is_none())
            .collect::<Vec<_>>();

        expect_eq(
            &label,
            0,
            "handled",
            i64::from(case["handled"].as_bool().unwrap_or(false)),
            i64::from(handled),
        );
        expect_eq(
            &label,
            0,
            "extractions",
            i(case, "extractions"),
            extracted.len() as i64,
        );
        expect_eq(&label, 0, "x", i(case, "x"), i64::from(x));
        expect_eq(&label, 0, "y", i(case, "y"), i64::from(y));
        expect_eq(&label, 0, "xdir", i(case, "xdir"), pixel.xdir.val() as i64);
        expect_eq(&label, 0, "ydir", i(case, "ydir"), pixel.ydir.val() as i64);
        expect_eq(
            &label,
            0,
            "extract_x",
            i(case, "extract_x"),
            extracted.first().map_or(-1, |(x, _, _, _)| i64::from(*x)),
        );
        expect_eq(
            &label,
            0,
            "extract_y",
            i(case, "extract_y"),
            extracted.first().map_or(-1, |(_, y, _, _)| i64::from(*y)),
        );
        expect_eq(
            &label,
            0,
            "draws",
            i(case, "draws"),
            i64::from(engine.rng.count - draws_before),
        );
        expect_rng_state(&label, case, &engine.rng);
        expect_eq(
            &label,
            0,
            "pos_changed",
            i64::from(case["pos_changed"].as_bool().unwrap_or(false)),
            i64::from(pos_changed),
        );
    }

    // 16b2. incinerate_arm: `mrfIncinerate` (C4Material.cpp:747-771), whose
    //       three arms are asymmetric in ways a port is likely to flatten.
    //
    //       `meeMassMove` and `meePXSPos` report **unhandled** when the pixel
    //       does not ignite — unhandled means the caller keeps looking, so
    //       answering "handled" there silently swallows the pixel. `meePXSMove`
    //       runs the insertion check FIRST, so a splash that prevents the
    //       interaction returns before anything burns; and it is the only arm
    //       that inserts a pixel which failed to ignite rather than dropping it.
    //
    //       Ignition is derived from the fixture on both sides, never dictated:
    //       the target pixel is inflammable or it is not, and the separate input
    //       is whether a FLAM already stands in the 8x20 rect at (x-4, y-1) that
    //       suppresses a second one (C4Landscape.cpp:1478-1488).
    for case in golden["incinerate_arm"].as_array().unwrap() {
        let name = case["name"].as_str().unwrap_or("?");
        let label = format!("incinerate_arm[{name}]");

        // Same Map as the insert arm, with Lava additionally Inflammable —
        // Incindiary is the PXS's own smoke property, Inflammable is whether
        // the landscape material catches, and the two are read for different
        // reasons.
        let library = clonk_resources::MaterialLibrary::parse(
            r#"
            [Material Vacuum]
            Name=Vacuum
            Density=0

            [Material Water]
            Name=Water
            Density=25
            SplashRate=1
            MaxSlide=4

            [Material Lava]
            Name=Lava
            Density=25
            Incindiary=1
            Inflammable=1
            MaxSlide=4

            [Material Granite]
            Name=Granite
            Density=50
            "#,
        )
        .expect("incinerate arm oracle materials parse");

        const WDT: u32 = 16;
        const HGT: u32 = 12;
        const GRANITE: u8 = 3;
        let px = i(case, "x0") as i32;
        let py = i(case, "y0") as i32;

        let mut bytes = vec![0u8; WDT as usize * HGT as usize];
        for gy in 0..HGT as usize {
            for gx in 0..WDT as usize {
                if gx != px as usize {
                    bytes[gy * WDT as usize + gx] = GRANITE;
                }
            }
        }
        for gx in 0..WDT as usize {
            bytes[10 * WDT as usize + gx] = GRANITE;
        }
        // The target pixel is whatever this row is about.
        bytes[py as usize * WDT as usize + px as usize] = i(case, "target_mat") as u8;

        let mut densities = vec![0; 128];
        densities[1] = 25;
        densities[2] = 25;
        densities[GRANITE as usize] = 50;
        let mut material_names = vec![None; 128];
        material_names[1] = Some("Water".to_string());
        material_names[2] = Some("Lava".to_string());
        material_names[GRANITE as usize] = Some("Granite".to_string());
        let grid = PixelGrid::new(WDT, HGT, bytes, densities, material_names, vec![None; 128]);

        let mut engine = Engine::with_seed(0);
        engine.configure_materials_from_library(&library);
        engine.set_physics(PhysicsSettings::new(100, 1000, -1000));
        // `C4Landscape::Incinerate` creates a FLAM, so the definition has to
        // exist for ignition to be possible at all.
        engine
            .register_definition(
                crate::Definition::from_script(crate::FIRE_DEFINITION_ID, "Fire", "#strict\n")
                    .expect("FLAM definition compiles"),
            )
            .expect("FLAM definition registers");
        let mut landscape = Landscape::flat(WDT, HGT as i32);
        landscape.set_pixel_grid(grid);
        landscape.set_world_height(HGT as i32);
        engine.set_landscape(landscape);

        if case["flam_here"].as_bool().unwrap_or(false) {
            // Inside the 8x20 rect at (x-4, y-1) that C++ tests with FindObject.
            engine
                .spawn_object(
                    crate::SpawnConfig::new(crate::FIRE_DEFINITION_ID)
                        .with_position(crate::Vector2::new(px, py)),
                )
                .expect("the suppressing FLAM spawns");
        }

        engine.rng = LcgRng::new(i(case, "seed") as u32);
        engine.rng.randomize3();
        let draws_before = engine.rng.count;
        let flams_before = engine
            .snapshot()
            .objects
            .iter()
            .filter(|object| object.definition_id == crate::FIRE_DEFINITION_ID)
            .count();
        let landscape_before = landscape_material_snapshot(&engine, WDT, HGT);

        // mrfIncinerate is not available as a user reaction (C++ asserts it),
        // so there is no user-defined row.
        let reaction = crate::material::MaterialReaction {
            kind: crate::material::MaterialReactionKind::Incinerate,
            user_defined: false,
            insertion_check: true,
        };
        let mut pixel = crate::pxs::Pxs {
            mat: crate::pxs::PxsMaterial::from_raw(i(case, "pxs_mat") as i32),
            x: itofix(px),
            y: itofix(py),
            xdir: C4Fixed::from_raw(i(case, "xdir0") as i32),
            ydir: C4Fixed::from_raw(i(case, "ydir0") as i32),
        };
        let (mut x, mut y) = (px, py);
        let mut pos_changed = false;
        crate::engine_landscape_ops::MATERIAL_INCINERATE_PROBES
            .with(|probes| probes.borrow_mut().clear());
        let handled = engine.execute_pxs_reaction(
            reaction,
            &mut x,
            &mut y,
            px,
            py,
            &mut pixel,
            crate::material::MaterialId::new(i(case, "ls_mat") as usize),
            match i(case, "event") {
                0 => MaterialInteractionEvent::PxsPos,
                1 => MaterialInteractionEvent::PxsMove,
                _ => MaterialInteractionEvent::MassMove,
            },
            &mut pos_changed,
        );

        let flams_created = engine
            .snapshot()
            .objects
            .iter()
            .filter(|object| object.definition_id == crate::FIRE_DEFINITION_ID)
            .count()
            - flams_before;
        let incinerate_probes = crate::engine_landscape_ops::MATERIAL_INCINERATE_PROBES
            .with(|probes| std::mem::take(&mut *probes.borrow_mut()));
        assert!(
            incinerate_probes
                .iter()
                .all(|&(probe_x, probe_y)| (probe_x, probe_y) == (x, y)),
            "PARITY DIVERGENCE in `{label}`: Incinerate probe coordinates differ from the reaction's final position",
        );
        let landscape_changes = landscape_material_changes(&landscape_before, &engine, WDT, HGT);
        let inserted = landscape_changes
            .iter()
            .filter(|(_, _, before, after)| {
                before.map(Into::into) != Some(pixel.mat)
                    && after.map(Into::into) == Some(pixel.mat)
            })
            .collect::<Vec<_>>();

        expect_eq(
            &label,
            0,
            "handled",
            i64::from(case["handled"].as_bool().unwrap_or(false)),
            i64::from(handled),
        );
        expect_eq(
            &label,
            0,
            "flams_created",
            i(case, "flams_created"),
            flams_created as i64,
        );
        expect_eq(
            &label,
            0,
            "incinerate_calls",
            i(case, "incinerate_calls"),
            incinerate_probes.len() as i64,
        );
        expect_eq(
            &label,
            0,
            "inserted",
            i(case, "inserted"),
            inserted.len() as i64,
        );
        expect_eq(
            &label,
            0,
            "inserted_mat",
            i(case, "inserted_mat"),
            inserted
                .first()
                .and_then(|(_, _, _, material)| *material)
                .map_or(-1, |material| material.index() as i64),
        );
        expect_eq(
            &label,
            0,
            "inserted_x",
            i(case, "inserted_x"),
            inserted.first().map_or(-1, |(x, _, _, _)| i64::from(*x)),
        );
        expect_eq(
            &label,
            0,
            "inserted_y",
            i(case, "inserted_y"),
            inserted.first().map_or(-1, |(_, y, _, _)| i64::from(*y)),
        );
        expect_eq(&label, 0, "x", i(case, "x"), i64::from(x));
        expect_eq(&label, 0, "y", i(case, "y"), i64::from(y));
        expect_eq(&label, 0, "xdir", i(case, "xdir"), pixel.xdir.val() as i64);
        expect_eq(&label, 0, "ydir", i(case, "ydir"), pixel.ydir.val() as i64);
        expect_eq(
            &label,
            0,
            "pos_changed",
            i64::from(case["pos_changed"].as_bool().unwrap_or(false)),
            i64::from(pos_changed),
        );
        expect_eq(
            &label,
            0,
            "draws",
            i(case, "draws"),
            i64::from(engine.rng.count - draws_before),
        );
        expect_rng_state(&label, case, &engine.rng);
    }

    // 16c. insert_check: `mrfInsertCheck` (C4Material.cpp:567-609) with the
    //      `FindMatSlide` it calls (C4Landscape.cpp:1247-1277) — the arm every
    //      falling pixel takes on landing, which `pxs_execute` deliberately
    //      excludes because it needs the reaction table. Its RNG ledger is
    //      property-dependent, so the draw count is compared alongside the
    //      rewritten position and velocity.
    for case in golden["insert_check"].as_array().unwrap() {
        let name = case["name"].as_str().unwrap_or("?");
        let label = format!("insert_check[{name}]");

        // Indices match the oracle's Map: 0 Vacuum, 1 Water (SplashRate 1 makes
        // the roll certain), 2 Lava (incendiary), 3 Granite (the floor).
        let library = clonk_resources::MaterialLibrary::parse(
            r#"
            [Material Vacuum]
            Name=Vacuum
            Density=0

            [Material Water]
            Name=Water
            Density=25
            SplashRate=1
            MaxSlide=4

            [Material Lava]
            Name=Lava
            Density=25
            Incindiary=1
            MaxSlide=4

            [Material Granite]
            Name=Granite
            Density=50
            "#,
        )
        .expect("insert check oracle materials parse");

        const WDT: u32 = 16;
        const HGT: u32 = 12;
        const GRANITE: u8 = 3;
        let hole = i(case, "hole") as i32;
        let mut bytes = vec![0u8; WDT as usize * HGT as usize];
        if case["floor"].as_bool().unwrap_or(false) {
            for gx in 0..WDT as i32 {
                if gx != hole {
                    bytes[10 * WDT as usize + gx as usize] = GRANITE;
                }
            }
        }
        if case["walled"].as_bool().unwrap_or(false) {
            for gy in 0..HGT as usize {
                for gx in 0..WDT as usize {
                    if gx != 8 {
                        bytes[gy * WDT as usize + gx] = GRANITE;
                    }
                }
            }
        }
        let mut densities = vec![0; 128];
        densities[GRANITE as usize] = 50;
        let mut material_names = vec![None; 128];
        material_names[GRANITE as usize] = Some("Granite".to_string());
        let grid = PixelGrid::new(WDT, HGT, bytes, densities, material_names, vec![None; 128]);

        let mut engine = Engine::with_seed(0);
        engine.configure_materials_from_library(&library);
        register_smoke_probe(&mut engine);
        engine.set_physics(PhysicsSettings::new(100, 1000, -1000));
        let mut landscape = Landscape::flat(WDT, HGT as i32);
        landscape.set_pixel_grid(grid);
        landscape.set_world_height(HGT as i32);
        engine.set_landscape(landscape);
        engine.rng = LcgRng::new(i(case, "seed") as u32);
        engine.rng.randomize3();
        let draws_before = engine.rng.count;

        let mut x = i(case, "x0") as i32;
        let mut y = i(case, "y0") as i32;
        let mut xdir = C4Fixed::from_raw(i(case, "xdir0") as i32);
        let mut ydir = C4Fixed::from_raw(i(case, "ydir0") as i32);
        let mut pos_changed = false;
        let verdict = engine.mrf_insert_check(
            &mut x,
            &mut y,
            &mut xdir,
            &mut ydir,
            crate::material::MaterialId::new(i(case, "pxs_mat") as usize)
                .expect("oracle pxs material"),
            crate::material::MaterialId::new(i(case, "ls_mat") as usize),
            &mut pos_changed,
        );

        expect_eq(
            &label,
            0,
            "verdict",
            i64::from(case["verdict"].as_bool().unwrap_or(false)),
            i64::from(verdict),
        );
        expect_eq(&label, 0, "x", i(case, "x"), i64::from(x));
        expect_eq(&label, 0, "y", i(case, "y"), i64::from(y));
        expect_eq(&label, 0, "xdir", i(case, "xdir"), xdir.val() as i64);
        expect_eq(&label, 0, "ydir", i(case, "ydir"), ydir.val() as i64);
        expect_eq(
            &label,
            0,
            "pos_changed",
            i64::from(case["pos_changed"].as_bool().unwrap_or(false)),
            i64::from(pos_changed),
        );
        expect_eq(
            &label,
            0,
            "smoke",
            i(case, "smoke"),
            smoke_probe_count(&engine),
        );
        expect_eq(
            &label,
            0,
            "draws",
            i(case, "draws"),
            i64::from(engine.rng.count - draws_before),
        );
        expect_rng_state(&label, case, &engine.rng);
    }

    // 16d. convert_check: `mrfConvert` (C4Material.cpp:626-661) with the
    //      `mrfUserCheck` wrapper it calls. Three rules a port can lose in
    //      translation:
    //
    //      * C++'s `case meePXSMove:` falls **through** into `meePXSPos` when
    //        the reaction is user-defined, so a user conversion fires on a
    //        move event where a hardcoded one breaks out. Rust has no implicit
    //        fallthrough, so this is an easy arm to drop.
    //      * A *successful* conversion returns `false` — "not handled", the
    //        caller keeps going — while a conversion whose target is not
    //        loaded returns `true` and kills the pixel. The verdict reads
    //        backwards from the intuitive one.
    //      * The `meeMassMove` arm hands the PXS system the mover's
    //        **original** material, not the convert target: that case jumps
    //        straight past the reassignment above it.
    //
    //      The port splits the mass-move arm out into
    //      `Engine::execute_mass_move_reaction`, because that event needs
    //      engine state the PXS path does not have. Driving both against the
    //      one lifted C++ function is the point — it shows the split kept the
    //      behaviour.
    for case in golden["convert_check"].as_array().unwrap() {
        let name = case["name"].as_str().unwrap_or("?");
        let label = format!("convert_check[{name}]");

        // Indices match the oracle's Map: 0 Vacuum, 1 Water, 2 Lava (which
        // carries the hardcoded InMatConvert to Granite at depth 2), 3 Granite.
        let library = clonk_resources::MaterialLibrary::parse(
            r#"
            [Material Vacuum]
            Name=Vacuum
            Density=0

            [Material Water]
            Name=Water
            Density=25

            [Material Lava]
            Name=Lava
            Density=25
            InMatConvert=Granite
            InMatConvertTo=Granite
            InMatConvertDepth=2

            [Material Granite]
            Name=Granite
            Density=50
            "#,
        )
        .expect("convert oracle materials parse");

        const WDT: u32 = 16;
        const HGT: u32 = 12;
        let x0 = i(case, "x0") as i32;
        let y0 = i(case, "y0") as i32;
        let xdir0 = C4Fixed::from_raw(i(case, "xdir0") as i32);
        let ydir0 = C4Fixed::from_raw(i(case, "ydir0") as i32);
        let user_defined = case["user_defined"].as_bool().unwrap_or(false);
        // Hardcoded conversions read the depth off the material; user ones
        // carry their own, and every user case here leaves it at 0.
        let depth = if user_defined {
            i(case, "depth") as i32
        } else {
            2
        };
        let ls_mat = i(case, "ls_mat") as usize;
        let event = match i(case, "event") {
            0 => MaterialInteractionEvent::PxsPos,
            1 => MaterialInteractionEvent::PxsMove,
            _ => MaterialInteractionEvent::MassMove,
        };

        let mut bytes = vec![0u8; WDT as usize * HGT as usize];
        if event == MaterialInteractionEvent::MassMove {
            // The mass-move entry derives its own reaction from the landscape
            // material under the mover, so the pixel goes at (x0, y0).
            bytes[y0 as usize * WDT as usize + x0 as usize] = ls_mat as u8;
        } else if case["matching_above"].as_bool().unwrap_or(false) && depth != 0 {
            bytes[(y0 - depth) as usize * WDT as usize + x0 as usize] = ls_mat as u8;
        }
        let mut densities = vec![0; 128];
        densities[ls_mat] = 50;
        let mut material_names = vec![None; 128];
        material_names[ls_mat] = Some("Granite".to_string());
        let grid = PixelGrid::new(WDT, HGT, bytes, densities, material_names, vec![None; 128]);

        let mut engine = Engine::with_seed(0);
        engine.configure_materials_from_library(&library);
        let mut landscape = Landscape::flat(WDT, HGT as i32);
        landscape.set_pixel_grid(grid);
        landscape.set_world_height(HGT as i32);
        engine.set_landscape(landscape);
        engine.rng = LcgRng::new(i(case, "seed") as u32);
        engine.rng.randomize3();
        let draws_before = engine.rng.count;

        let pxs_mat = crate::material::MaterialId::new(i(case, "pxs_mat0") as usize)
            .expect("oracle pxs material");

        if event == MaterialInteractionEvent::MassMove {
            let execution = engine.execute_mass_move_reaction(pxs_mat, x0, y0, x0, y0);
            let (created, created_mat) = match execution {
                crate::material::MaterialReactionExecution::Converted(mat) => {
                    (1, mat.index() as i64)
                }
                _ => (0, -1),
            };
            expect_eq(
                &label,
                0,
                "handled",
                i64::from(case["handled"].as_bool().unwrap_or(false)),
                i64::from(!matches!(
                    execution,
                    crate::material::MaterialReactionExecution::Unhandled
                )),
            );
            expect_eq(&label, 0, "x", i(case, "x"), i64::from(x0));
            expect_eq(&label, 0, "y", i(case, "y"), i64::from(y0));
            expect_eq(&label, 0, "xdir", i(case, "xdir"), xdir0.val() as i64);
            expect_eq(&label, 0, "ydir", i(case, "ydir"), ydir0.val() as i64);
            expect_eq(
                &label,
                0,
                "pos_changed",
                i64::from(case["pos_changed"].as_bool().unwrap_or(false)),
                0,
            );
            expect_eq(
                &label,
                0,
                "draws",
                i(case, "draws"),
                i64::from(engine.rng.count - draws_before),
            );
            expect_rng_state(&label, case, &engine.rng);
            expect_eq(
                &label,
                0,
                "pxs_mat",
                i(case, "pxs_mat"),
                pxs_mat.index() as i64,
            );
            expect_eq(&label, 0, "pxs_created", i(case, "pxs_created"), created);
            expect_eq(
                &label,
                0,
                "pxs_created_mat",
                i(case, "pxs_created_mat"),
                created_mat,
            );
            continue;
        }

        let target = if user_defined {
            i(case, "convert_mat") as usize
        } else {
            3
        };
        let reaction = crate::material::MaterialReaction {
            kind: crate::material::MaterialReactionKind::Convert {
                target: crate::material::MaterialId::new(target),
                depth: (depth != 0).then_some(depth),
            },
            user_defined,
            // The oracle drives mrfConvert with CheckSlide off, so the
            // mrfUserCheck splash/slide branch stays out of this section —
            // `insert_check` covers it directly.
            insertion_check: false,
        };
        let mut pixel = crate::pxs::Pxs {
            mat: pxs_mat.into(),
            x: itofix(x0),
            y: itofix(y0),
            xdir: xdir0,
            ydir: ydir0,
        };
        let (mut x, mut y) = (x0, y0);
        let mut pos_changed = false;
        let handled = engine.execute_pxs_reaction(
            reaction,
            &mut x,
            &mut y,
            x0,
            y0,
            &mut pixel,
            crate::material::MaterialId::new(ls_mat),
            event,
            &mut pos_changed,
        );

        expect_eq(
            &label,
            0,
            "handled",
            i64::from(case["handled"].as_bool().unwrap_or(false)),
            i64::from(handled),
        );
        expect_eq(&label, 0, "x", i(case, "x"), i64::from(x));
        expect_eq(&label, 0, "y", i(case, "y"), i64::from(y));
        expect_eq(&label, 0, "xdir", i(case, "xdir"), pixel.xdir.val() as i64);
        expect_eq(&label, 0, "ydir", i(case, "ydir"), pixel.ydir.val() as i64);
        expect_eq(
            &label,
            0,
            "pos_changed",
            i64::from(case["pos_changed"].as_bool().unwrap_or(false)),
            i64::from(pos_changed),
        );
        expect_eq(
            &label,
            0,
            "draws",
            i(case, "draws"),
            i64::from(engine.rng.count - draws_before),
        );
        expect_rng_state(&label, case, &engine.rng);
        let created = engine.pxs_system.iter().collect::<Vec<_>>();
        expect_eq(
            &label,
            0,
            "pxs_created",
            i(case, "pxs_created"),
            created.len() as i64,
        );
        expect_eq(
            &label,
            0,
            "pxs_created_mat",
            i(case, "pxs_created_mat"),
            created.first().map_or(-1, |pixel| pixel.mat.raw() as i64),
        );
        // C++ assigns the target id *before* validating it, so a failed
        // conversion leaves `iPxsMat` holding an unloaded index
        // (C4Material.cpp:646-649); the port leaves the id alone. Neither is
        // observable — the caller deactivates the pixel on the `true` return
        // and `Deactivate` overwrites Mat — so the material is compared where
        // the conversion actually took, which is where it is read.
        if !handled {
            expect_eq(
                &label,
                0,
                "pxs_mat",
                i(case, "pxs_mat"),
                pixel.mat.raw() as i64,
            );
        }
    }

    // 16e. insert_arm: `mrfInsert` (C4Material.cpp:773-798) — the arm a pixel
    //      takes to stop being PXS and become landscape. Only `meePXSMove`
    //      inserts; the other two events break straight out.
    //
    //      The rule worth an oracle is the placement of its splash/slide
    //      check: it sits *inside* the movement case behind a `!fUserDefined`
    //      gate, because a user-defined reaction already ran the same check on
    //      the way in through `mrfUserCheck`. Lose that gate and every
    //      inserting user pixel runs the check twice, spending twice the
    //      synchronized draws — a desync that leaves the position untouched
    //      and so hides from any comparison that only looks at where the pixel
    //      ended up. The draw count is compared for exactly that reason.
    for case in golden["insert_arm"].as_array().unwrap() {
        let name = case["name"].as_str().unwrap_or("?");
        let label = format!("insert_arm[{name}]");

        // Indices match the oracle's Map: 0 Vacuum, 1 Water (SplashRate 1
        // makes the roll certain), 2 Lava (incendiary), 3 Granite.
        let library = clonk_resources::MaterialLibrary::parse(
            r#"
            [Material Vacuum]
            Name=Vacuum
            Density=0

            [Material Water]
            Name=Water
            Density=25
            SplashRate=1
            MaxSlide=4

            [Material Lava]
            Name=Lava
            Density=25
            Incindiary=1
            MaxSlide=4

            [Material Granite]
            Name=Granite
            Density=50
            "#,
        )
        .expect("insert arm oracle materials parse");

        const WDT: u32 = 16;
        const HGT: u32 = 12;
        const GRANITE: u8 = 3;
        let px = i(case, "x0") as i32;
        let py = i(case, "y0") as i32;
        // Boxed in over a solid floor, so `FindMatSlide` has no target and the
        // check's verdict is decided by the splash arm alone.
        let mut bytes = vec![0u8; WDT as usize * HGT as usize];
        for gy in 0..HGT as usize {
            for gx in 0..WDT as usize {
                if gx != px as usize {
                    bytes[gy * WDT as usize + gx] = GRANITE;
                }
            }
        }
        for gx in 0..WDT as usize {
            bytes[10 * WDT as usize + gx] = GRANITE;
        }
        let mut densities = vec![0; 128];
        densities[1] = 25;
        densities[2] = 25;
        densities[GRANITE as usize] = 50;
        let mut material_names = vec![None; 128];
        material_names[1] = Some("Water".to_string());
        material_names[2] = Some("Lava".to_string());
        material_names[GRANITE as usize] = Some("Granite".to_string());
        let grid = PixelGrid::new(WDT, HGT, bytes, densities, material_names, vec![None; 128]);

        let mut engine = Engine::with_seed(0);
        engine.configure_materials_from_library(&library);
        engine.set_physics(PhysicsSettings::new(100, 1000, -1000));
        let mut landscape = Landscape::flat(WDT, HGT as i32);
        landscape.set_pixel_grid(grid);
        landscape.set_world_height(HGT as i32);
        engine.set_landscape(landscape);
        engine.rng = LcgRng::new(i(case, "seed") as u32);
        engine.rng.randomize3();
        let draws_before = engine.rng.count;
        let landscape_before = landscape_material_snapshot(&engine, WDT, HGT);

        let reaction = crate::material::MaterialReaction {
            kind: crate::material::MaterialReactionKind::Insert,
            user_defined: case["user_defined"].as_bool().unwrap_or(false),
            insertion_check: case["insertion_check"].as_bool().unwrap_or(false),
        };
        let mut pixel = crate::pxs::Pxs {
            mat: crate::pxs::PxsMaterial::from_raw(i(case, "pxs_mat") as i32),
            x: itofix(px),
            y: itofix(py),
            xdir: C4Fixed::from_raw(i(case, "xdir0") as i32),
            ydir: C4Fixed::from_raw(i(case, "ydir0") as i32),
        };
        let (mut x, mut y) = (px, py);
        let mut pos_changed = false;
        let handled = engine.execute_pxs_reaction(
            reaction,
            &mut x,
            &mut y,
            px,
            py,
            &mut pixel,
            crate::material::MaterialId::new(i(case, "ls_mat") as usize),
            match i(case, "event") {
                0 => MaterialInteractionEvent::PxsPos,
                1 => MaterialInteractionEvent::PxsMove,
                _ => MaterialInteractionEvent::MassMove,
            },
            &mut pos_changed,
        );

        expect_eq(
            &label,
            0,
            "handled",
            i64::from(case["handled"].as_bool().unwrap_or(false)),
            i64::from(handled),
        );
        expect_eq(&label, 0, "x", i(case, "x"), i64::from(x));
        expect_eq(&label, 0, "y", i(case, "y"), i64::from(y));
        expect_eq(&label, 0, "xdir", i(case, "xdir"), pixel.xdir.val() as i64);
        expect_eq(&label, 0, "ydir", i(case, "ydir"), pixel.ydir.val() as i64);
        expect_eq(
            &label,
            0,
            "pos_changed",
            i64::from(case["pos_changed"].as_bool().unwrap_or(false)),
            i64::from(pos_changed),
        );
        expect_eq(
            &label,
            0,
            "draws",
            i(case, "draws"),
            i64::from(engine.rng.count - draws_before),
        );
        expect_rng_state(&label, case, &engine.rng);
        // The oracle stubs `InsertMaterial` to a recorder — that mutation is a
        // whole landscape operation of its own and earns its own section — so
        // the fixture makes the real insertion a single visible landscape
        // delta. Comparing the complete delta catches an extra or misplaced
        // insertion as well as a missing call.
        let changes = landscape_material_changes(&landscape_before, &engine, WDT, HGT);
        expect_eq(
            &label,
            0,
            "inserted",
            i(case, "inserted"),
            changes.len() as i64,
        );
        if let Some((inserted_x, inserted_y, before, after)) = changes.first().copied() {
            expect_eq(
                &label,
                0,
                "inserted_before",
                -1,
                before.map_or(-1, |id| id.index() as i64),
            );
            expect_eq(
                &label,
                0,
                "inserted_mat",
                i(case, "inserted_mat"),
                after.map_or(-1, |id| id.index() as i64),
            );
            expect_eq(
                &label,
                0,
                "inserted_x",
                i(case, "inserted_x"),
                i64::from(inserted_x),
            );
            expect_eq(
                &label,
                0,
                "inserted_y",
                i(case, "inserted_y"),
                i64::from(inserted_y),
            );
        }
    }

    // 16f. pxs_slots: `C4PXSSystem::Create` and `Cast`
    //      (C4PXS.cpp:207-215,309-321) —
    //      the layer above the allocator. `pxs_allocation` already owns `New`'s
    //      slot choice, over four slots freed out of order; this covers what
    //      that one cannot reach:
    //
    //      * **`Cast` draws ydir's random first.** The C++ pulls both into
    //        named locals under a `// force argument evaluation order` comment,
    //        and the one drawn *first* (`r2`) is the one used for ydir. Reading
    //        them in argument order swaps the velocities while drawing exactly
    //        as many numbers — invisible to any draw-count check, which is why
    //        the raw fixed values are compared per slot.
    //      * **Per-slot state and chunk counts**, rather than only which slot a
    //        returned pointer landed in.
    //      * **The chunk boundary.** Chunk 0 holds 500 slots; `pxs_allocation`
    //        never creates enough particles to spill into chunk 1.
    //
    //      The steps run as one sequence against one system, so a wrong slot
    //      choice early shows up in every later step.
    {
        let library = MaterialLibrary::parse(
            r#"
            [Material]
            Name=Vacuum
            [Material]
            Name=Earth
            [Material]
            Name=Water
            [Material]
            Name=Granite
            "#,
        )
        .expect("PXS slot material map parses");
        let materials = crate::MaterialSet::from_resource_library(&library);
        let mut system = crate::pxs::PxsSystem::default();
        let mut rng = LcgRng::new(0x5151);
        rng.randomize3();
        let mut mark = rng.count;

        let check =
            |label: &str, step: &serde_json::Value, system: &crate::pxs::PxsSystem, draws: i32| {
                expect_eq(label, 0, "draws", i(step, "draws"), i64::from(draws));
                expect_eq(label, 0, "live", i(step, "live"), system.count() as i64);
                for chunk in step["chunks"].as_array().unwrap() {
                    let index = i(chunk, "i") as usize;
                    expect_eq(
                        label,
                        index,
                        "chunk_alloc",
                        i64::from(chunk["alloc"].as_bool().unwrap_or(false)),
                        i64::from(system.chunk_allocated(index)),
                    );
                    expect_eq(label, index, "chunk_count", i(chunk, "count"), {
                        (0..crate::pxs::PXS_CHUNK_SIZE)
                            .filter(|slot| system.peek_slot(index, *slot).is_some())
                            .count() as i64
                    });
                }
                for slot in step["slots"].as_array().unwrap() {
                    let index = i(slot, "i") as usize;
                    let live = system.peek_slot(0, index);
                    let mat = live.map(|pxs| pxs.mat.raw() as i64).unwrap_or(-1);
                    expect_eq(label, index, "slot_mat", i(slot, "mat"), mat);
                    // Ordinary execution equality compares a dead slot by Mat
                    // only: Execute and Load gate on Mat != MNone. Its retained
                    // raw payload is serialization state instead, and the
                    // integrated `pxs_lifecycle.saved_slots` section compares
                    // those exact PXS.c4b bytes (C4PXS.cpp:139-149, 324-349).
                    if let Some(pxs) = live {
                        expect_eq(label, index, "slot_x", i(slot, "x"), pxs.x.val() as i64);
                        expect_eq(label, index, "slot_y", i(slot, "y"), pxs.y.val() as i64);
                        expect_eq(
                            label,
                            index,
                            "slot_xdir",
                            i(slot, "xdir"),
                            pxs.xdir.val() as i64,
                        );
                        expect_eq(
                            label,
                            index,
                            "slot_ydir",
                            i(slot, "ydir"),
                            pxs.ydir.val() as i64,
                        );
                    }
                }
            };

        let steps = golden["pxs_slots"].as_array().unwrap();
        let step_named = |name: &str| {
            steps
                .iter()
                .find(|step| step["step"].as_str() == Some(name))
                .unwrap_or_else(|| panic!("pxs_slots golden is missing step `{name}`"))
        };

        // Create validates against the loaded material map before New may
        // allocate a chunk (C4PXS.cpp:207-215). Drive both the checked Rust
        // allocator and its production operation fold with a representable
        // MaterialId immediately beyond this four-entry map.
        let mut invalid_engine = Engine::with_seed(0);
        invalid_engine.configure_materials_from_library(&library);
        let invalid_material =
            crate::material::MaterialId::new(99).expect("representable invalid id");
        let create_result = invalid_engine.create_pxs(
            invalid_material,
            itofix(7),
            itofix(8),
            C4Fixed::ZERO,
            C4Fixed::ZERO,
        );
        invalid_engine.apply_landscape_operations(vec![LandscapeOperation::CastPxs {
            material: invalid_material,
            position: crate::Vector2::new(7, 8),
            velocities: vec![FixedVec2::new(C4Fixed::ZERO, C4Fixed::ZERO)],
        }]);
        let invalid_step = step_named("invalid_create");
        expect_eq(
            "pxs_slots[invalid_create]",
            0,
            "result",
            i64::from(invalid_step["result"].as_bool().unwrap_or(true)),
            i64::from(create_result),
        );
        check(
            "pxs_slots[invalid_create]",
            invalid_step,
            &invalid_engine.pxs_system,
            0,
        );

        system.cast(
            &materials,
            &mut rng,
            crate::material::MaterialId::new(2).unwrap(),
            3,
            30,
            40,
            20,
        );
        check(
            "pxs_slots[cast_three]",
            step_named("cast_three"),
            &system,
            rng.count - mark,
        );
        mark = rng.count;

        system.clear_slot(0, 1);
        check(
            "pxs_slots[free_middle]",
            step_named("free_middle"),
            &system,
            rng.count - mark,
        );
        mark = rng.count;

        system.cast(
            &materials,
            &mut rng,
            crate::material::MaterialId::new(1).unwrap(),
            1,
            10,
            12,
            4,
        );
        check(
            "pxs_slots[reuse_freed_slot]",
            step_named("reuse_freed_slot"),
            &system,
            rng.count - mark,
        );
        mark = rng.count;

        let granite = crate::material::MaterialId::new(3).unwrap();
        while system.count() < crate::pxs::PXS_CHUNK_SIZE {
            system.create(
                &materials,
                granite,
                itofix(1),
                itofix(2),
                C4Fixed::ZERO,
                C4Fixed::ZERO,
            );
        }
        check(
            "pxs_slots[fill_chunk]",
            step_named("fill_chunk"),
            &system,
            rng.count - mark,
        );
        mark = rng.count;

        system.create(
            &materials,
            granite,
            itofix(7),
            itofix(8),
            C4Fixed::ZERO,
            C4Fixed::ZERO,
        );
        check(
            "pxs_slots[spill_to_chunk1]",
            step_named("spill_to_chunk1"),
            &system,
            rng.count - mark,
        );
    }

    // 16g. pxs_load: `C4PXSSystem::Load` (C4PXS.cpp:362-399). Its accept/reject
    //      decision is pure arithmetic on the file length — the four-byte
    //      number-format tag is detected by the remainder being *exactly 4*,
    //      never by reading a magic value, so an untagged file and a tagged one
    //      with the same payload must load identically. Everything after that
    //      follows: the 1..2 format range, the chunk ceiling, and a per-chunk
    //      recount that has to attribute live slots to the chunk they sit in.
    //
    //      The float-format conversion sits *inside* the `Mat != MNone` branch,
    //      so it never touches a dead slot. The golden carries a compact recipe
    //      rather than the bytes — one case is 21 chunks, 210 KB — and both
    //      sides build the buffer from it.
    for case in golden["pxs_load"].as_array().unwrap() {
        let name = case["name"].as_str().unwrap_or("?");
        let label = format!("pxs_load[{name}]");

        let component = |chunks: usize, tag: i32, extra: usize, input: &serde_json::Value| {
            let mut bytes = Vec::new();
            if tag != 0 {
                bytes.extend_from_slice(&tag.to_le_bytes());
            }
            let payload_start = bytes.len();
            for _ in 0..chunks {
                for _ in 0..crate::pxs::PXS_CHUNK_SIZE {
                    bytes.extend_from_slice(&(-1i32).to_le_bytes());
                    bytes.extend_from_slice(&[0u8; 16]);
                }
            }
            for live in input.as_array().into_iter().flatten() {
                let offset = payload_start
                    + (i(live, "chunk") as usize * crate::pxs::PXS_CHUNK_SIZE
                        + i(live, "slot") as usize)
                        * 20;
                for (field, key) in ["mat", "x", "y", "xdir", "ydir"].iter().enumerate() {
                    let value = i(live, key) as i32;
                    bytes[offset + field * 4..offset + field * 4 + 4]
                        .copy_from_slice(&value.to_le_bytes());
                }
            }
            bytes.extend(std::iter::repeat_n(0u8, extra));
            bytes
        };
        let bytes = component(
            i(case, "chunks") as usize,
            i(case, "tag") as i32,
            i(case, "extra") as usize,
            &case["input"],
        );

        // The preload is the content standing when this Load runs. Load
        // clears before it validates, so a refused component empties it.
        let mut system = crate::pxs::PxsSystem::default();
        let preload_chunks = i(case, "preload_chunks") as usize;
        if preload_chunks > 0 {
            system
                .load_c4b(&component(preload_chunks, 1, 0, &case["preload"]))
                .unwrap_or_else(|error| panic!("{label}: preload must load: {error}"));
        }
        system.set_execute_count(i(case, "count_before") as usize);

        // C++ reads the entry through a C4Group and returns false when it is
        // absent, before it reaches Clear; the port is handed the bytes, so
        // absence is the caller's concern and never enters `load_c4b`.
        let present = case["present"].as_bool().unwrap_or(true);
        let ok = present && system.load_c4b(&bytes).is_ok();
        expect_eq(
            &label,
            0,
            "ok",
            i64::from(case["ok"].as_bool().unwrap_or(false)),
            i64::from(ok),
        );
        expect_eq(
            &label,
            0,
            "count_after",
            i(case, "count_after"),
            system.execute_count() as i64,
        );
        for (index, count) in case["counts"].as_array().unwrap().iter().enumerate() {
            let live = (0..crate::pxs::PXS_CHUNK_SIZE)
                .filter(|slot| system.peek_slot(index, *slot).is_some())
                .count() as i64;
            expect_eq(
                &label,
                index,
                "chunk_count",
                count.as_i64().unwrap_or(-1),
                live,
            );
        }
        for live in case["loaded"].as_array().unwrap() {
            let chunk = i(live, "chunk") as usize;
            let slot = i(live, "slot") as usize;
            let pxs = system
                .peek_slot(chunk, slot)
                .unwrap_or_else(|| panic!("{label}: chunk {chunk} slot {slot} did not load"));
            expect_eq(&label, slot, "mat", i(live, "mat"), pxs.mat.raw() as i64);
            expect_eq(&label, slot, "x", i(live, "x"), pxs.x.val() as i64);
            expect_eq(&label, slot, "y", i(live, "y"), pxs.y.val() as i64);
            expect_eq(&label, slot, "xdir", i(live, "xdir"), pxs.xdir.val() as i64);
            expect_eq(&label, slot, "ydir", i(live, "ydir"), pxs.ydir.val() as i64);
        }
    }

    // 16h. save_runtime_sequence: `C4GameSave::SaveRuntimeData`
    //      (C4GameSave.cpp:188-262) — the ordered component sweep the save
    //      policy queries drive. `game_save_policy` pins what each variant
    //      *decides*; this pins what that decision then *does*.
    //
    //      The rule worth an oracle is the one that reads backwards: scenario
    //      sections are written for an **exact** save, and the Title component
    //      for a **non-exact** one. Getting that pair the same way round is a
    //      coin flip from the names alone.
    //
    //      Two things are deliberately not compared. The golden's failure
    //      cases pin that a Script/Title/Info write is `nofail` while
    //      Landscape/Teams aborts; the port reports an aborted save as `Err`
    //      rather than a truncated sweep, so there is no ordered counterpart.
    //      And the `else` arm that deletes Game.txt/PlayerInfos.txt is
    //      unreachable for every shipped variant — the base returns
    //      `IsExact()` for both player queries and `C4GameSaveScenario`, the
    //      only non-exact one, overrides `GetSaveScriptPlayers` to a flat true
    //      — so the golden does not exercise it either.
    for case in golden["save_runtime_sequence"].as_array().unwrap() {
        let name = case["case"].as_str().unwrap_or("?");
        if !case["failing"].as_str().unwrap_or("").is_empty() {
            continue;
        }
        let label = format!("save_runtime_sequence[{name}]");
        let trace: Vec<&str> = case["trace"]
            .as_array()
            .unwrap()
            .iter()
            .filter_map(|step| step.as_str())
            .collect();

        let policy = match name {
            "scenario" => crate::live_c4_save::LiveC4SavePolicy::Scenario {
                force_exact_landscape: false,
            },
            "savegame" => crate::live_c4_save::LiveC4SavePolicy::Savegame {
                target_group_name: "Savegame.c4s",
            },
            "record_runtime" => crate::live_c4_save::LiveC4SavePolicy::Record,
            _ => crate::live_c4_save::LiveC4SavePolicy::RuntimeNetwork,
        };

        // The Title *decision* is not re-checked here: `game_save_policy`
        // already owns `GetKeepTitle`, and an assertion on it here was caught
        // by that section rather than this one. What is left, and what nothing
        // else covers, is the sweep that acts on the decision.
        //
        // The scenario variant is the one that saves without a landscape, so
        // it is the one where the real sweep can be run end to end. Every host
        // is modified, so each one the sweep reaches leaves a mutation and the
        // Script/Title/Info order is observable.
        if name != "scenario" {
            continue;
        }
        let hosts: Vec<&str> = trace
            .iter()
            .copied()
            .filter(|step| matches!(*step, "Script" | "Title" | "Info"))
            .collect();
        let replacement = |name: &'static str| {
            crate::live_c4_save::LiveC4ComponentHost::Replace(
                crate::live_c4_save::LiveC4SaveComponentRef {
                    name,
                    payload: b"x",
                },
            )
        };
        let modules: Vec<String> = Vec::new();
        let spec = crate::live_c4_save::LiveC4SaveSpec {
            title: "Sequence",
            definition_modules: &modules,
            definition_executable_path: "",
            definition_path: "",
            origin: "",
            music_enabled: false,
            copied_material_group_is_file: false,
            title_component: replacement("Title.txt"),
            info_component: replacement("Info.txt"),
            script_component: replacement("Script.c"),
        };

        let mut engine = Engine::new();
        let mut landscape =
            Landscape::with_default_material(16, vec![8; 16], None).expect("save landscape");
        landscape.set_world_height(8);
        engine.set_landscape(landscape);

        let save = engine
            .serialize_live_c4_save_with_policy(spec, policy)
            .unwrap_or_else(|error| panic!("{label}: the save must succeed, got {error:?}"));

        let ported: Vec<&str> = save
            .component_host_mutations
            .iter()
            .map(|mutation| match mutation {
                crate::live_c4_save::LiveC4SaveComponentMutation::Replace(component) => {
                    match component.name.as_str() {
                        "Script.c" => "Script",
                        "Title.txt" => "Title",
                        _ => "Info",
                    }
                }
                crate::live_c4_save::LiveC4SaveComponentMutation::Delete { .. } => "delete",
            })
            .collect();
        assert_eq!(
            ported, hosts,
            "PARITY DIVERGENCE in `{label}` field `component_hosts`: C++ golden = {hosts:?}, Rust = {ported:?}"
        );
    }

    // 16h1. c4group_sort: `C4Group::Sort` and `C4Group::SortByList`
    //       (C4Group.cpp:2300-2337,2366-2380), driven through the real linked
    //       group sources rather than a recorder.
    //
    //       Four rules, none readable off the sort list itself:
    //
    //         * rank order is DESCENDING — `SortRank` gives an earlier pattern
    //           a higher number and the sort keeps the higher rank first
    //           (`:2316-2318`), so reading it as an ascending index inverts
    //           every group;
    //         * a name matching no pattern ranks 0 and sinks below every
    //           listed one, rather than staying where it was;
    //         * equal ranks break on a case-insensitive comparison of the
    //           name (`:2320`);
    //         * the list is chosen by matching the GROUP's own filename
    //           against `C4CFN_FLS`, so an unlisted group filename sorts
    //           nothing at all (`SortByList` returns false at `:2368`) — which
    //           is also what `c4group -s` relies on by passing a null list.
    //
    //       A name byte above 0x7f is deliberately absent from the golden:
    //       `stricmp` is locale-dependent there and the golden is recorded on
    //       one host and compared on another. `clonk-resources`'
    //       `secondary_sort_orders_high_bytes_like_native_stricmp` keeps that
    //       case on the Rust side, where both halves run on the same host.
    {
        let rows = golden["c4group_sort"].as_array().unwrap();
        assert_eq!(
            rows.len(),
            5,
            "PARITY DIVERGENCE in `c4group_sort`: the golden must retain its exact 5-row matrix"
        );
        for row in rows {
            let name = row["case"].as_str().unwrap_or("?");
            let group = row["group"].as_str().unwrap_or("?");
            let label = format!("c4group_sort[{name}]");
            let expected: Vec<&str> = row["order"]
                .as_array()
                .unwrap()
                .iter()
                .filter_map(|entry| entry.as_str())
                .collect();

            // The golden carries the fixture's own insertion order, so the
            // port is fed exactly what C++ was fed. Reconstructing it instead
            // would be a guess, and a guess that happened to be sorted could
            // not tell "sorted correctly" from "never sorted at all" — which
            // is precisely the hole a load-bearing injection found here.
            let input: Vec<&str> = row["input"]
                .as_array()
                .unwrap()
                .iter()
                .filter_map(|entry| entry.as_str())
                .collect();
            let mut group_writer = MutableGroup::new_bytes(group.as_bytes().to_vec());
            for entry in &input {
                group_writer
                    .add_file_bytes_with_metadata(entry.as_bytes().to_vec(), vec![b'x'], 1, false)
                    .unwrap_or_else(|error| panic!("{label}: could not add `{entry}`: {error}"));
            }

            // Two different C++ entry points reach "no sort happened", and the
            // port models them through two different calls. Mapping both onto
            // `resort_for_filename_bytes` would have compared the wrong thing:
            //   * a NULL sort list is the caller declining to sort at all
            //     (`SortByList` returns false at C4Group.cpp:2368) — that is
            //     `c4group -s`, whose port counterpart is an empty sort list;
            //   * an unlisted GROUP filename means the table lookup found no
            //     pattern, which is what `resort_for_filename_bytes` reports by
            //     returning false.
            let reordered = if name == "a_null_sort_list_keeps_insertion_order" {
                group_writer.sort("")
            } else {
                group_writer.resort_for_filename_bytes(group.as_bytes().to_vec())
            };
            if input == expected {
                assert!(
                    !reordered,
                    "PARITY DIVERGENCE in `{label}`: C++ never reached Sort for this row, so the port must report no reordering"
                );
            }
            let actual = group_writer.entry_names();
            assert_eq!(
                actual, expected,
                "PARITY DIVERGENCE in `{label}` field `order`: C++ golden = {expected:?}, Rust = {actual:?}"
            );
        }
    }

    // 16h1b. c4group_raw_child_rewrite: `C4Group::Save`/`Close` stream every
    //        entry back out through `AppendEntry2StdFile`
    //        (C4Group.cpp:907-1050,1090+). A stored child group the caller
    //        never opened has to survive that byte-for-byte; only a child the
    //        caller actually replaced may be materialized anew. The port
    //        reaches the same decision through `Group::requires_rewrite`,
    //        which keeps `raw_image()` verbatim when nothing forced a repack.
    for case in golden["c4group_raw_child_rewrite"]
        .as_array()
        .expect("c4group_raw_child_rewrite is an array")
    {
        use clonk_resources::{Group, MutableGroup};

        let name = case["name"].as_str().unwrap_or("?");
        let label = format!("c4group_raw_child_rewrite[{name}]");
        let modify_sibling = i(case, "modify_sibling") != 0;
        let modify_child = i(case, "modify_child") != 0;

        // The child, packed on its own exactly as the oracle builds it first.
        let mut child = MutableGroup::new("RawChild.c4g");
        child
            .add_file_bytes_with_metadata("Inside.txt", b"child-payload".to_vec(), 1000, false)
            .expect("child entry adds");
        let child_image = child.pack().expect("child packs");
        let child_crc = child.contents_crc();

        let mut parent = MutableGroup::new("RawParent.c4g");
        parent
            .add_file_bytes_with_metadata("Sibling.txt", b"sibling-v1".to_vec(), 2000, false)
            .expect("sibling adds");
        parent
            .add_packed_child_bytes_with_metadata(
                "RawChild.c4g",
                child_image.clone(),
                child_crc,
                3000,
                false,
            )
            .expect("packed child adds");
        let packed = parent.pack().expect("parent packs");

        let stored_before = raw_child_bytes(&packed);

        // The rewrite: reopen, apply the case's modification, pack again.
        let opened = Group::from_top_level_memory("RawParent.c4g".into(), packed.clone())
            .expect("parent opens");
        let mut rewritten = MutableGroup::from_group(&opened).expect("parent copies");
        if modify_sibling {
            rewritten
                .add_file_bytes_with_metadata(
                    "Sibling.txt",
                    b"sibling-v2-longer".to_vec(),
                    3000,
                    false,
                )
                .expect("sibling replaces");
        }
        if modify_child {
            rewritten
                .add_packed_child_bytes_with_metadata(
                    "RawChild.c4g",
                    b"replaced-child".to_vec(),
                    0,
                    4000,
                    false,
                )
                .expect("child replaces");
        }
        let repacked = rewritten.pack().expect("parent repacks");
        let stored_after = raw_child_bytes(&repacked);

        let preserved = !stored_after.is_empty() && stored_after == stored_before;
        expect_eq(
            &label,
            0,
            "child_bytes_preserved",
            i(case, "child_bytes_preserved"),
            i64::from(preserved),
        );

        let reopened = Group::from_top_level_memory("RawParent.c4g".into(), repacked)
            .expect("rewritten parent opens");
        let entries = reopened.entries().expect("rewritten parent lists entries");
        let expected = case["entries"]
            .as_array()
            .expect("c4group_raw_child_rewrite entries");
        expect_eq(
            &label,
            0,
            "entry_count",
            expected.len() as i64,
            entries.len() as i64,
        );
        for (expected, actual) in expected.iter().zip(&entries) {
            let entry_label = format!(
                "{label}.entry[{}]",
                expected["name"].as_str().unwrap_or("?")
            );
            expect_json_eq(
                &entry_label,
                0,
                "name",
                expected["name"].clone(),
                serde_json::json!(String::from_utf8_lossy(&actual.name_bytes)),
            );
            // Absolute packed sizes are deliberately not compared. They depend
            // on how each engine *constructs* a group, which is a wider claim
            // than this section owns and which the two do not currently agree
            // on to the byte (clonk-org/clonk-rs#1191 records the measurement).
            // What matters here is that a rewrite does not disturb a child it
            // was not asked to touch, which `child_bytes_preserved` states
            // directly.
            expect_eq(
                &entry_label,
                0,
                "child",
                i(expected, "child"),
                i64::from(actual.is_directory),
            );
            // C++'s public surface reports the time it stored and the CRC it
            // computes on demand; `Packed`, `HasCRC` and `Executable` sit
            // behind `C4GroupEntry`'s access specifier and are not compared.
            expect_eq(
                &entry_label,
                0,
                "time",
                i(expected, "time"),
                i64::from(actual.time),
            );
        }
    }

    // 16h2. scenario_sections: `C4GameSave::SaveScenarioSections`
    //       (C4GameSave.cpp:111-137) — the one step of the exact-save sweep
    //       `save_runtime_sequence` records reaching and then stubs out.
    //
    //       The rule that cannot be read off the call site is the ORDER. The
    //       sweep walks `Game.pScenarioSections`, and `C4ScenarioSection`'s
    //       constructor PREPENDS (`C4Scenario.cpp:557-566`), so it runs in
    //       reverse construction order — and the implicit node for the
    //       departing section, which `C4Game::LoadScenarioSection` creates at
    //       the first switch (`C4Game.cpp:4094-4097`), is therefore the one it
    //       reaches first. Two more read backwards from their names: the
    //       CURRENT section is deleted and never re-added even when it is
    //       modified, and a modified section's `Add` result is discarded, so
    //       the sweep has no failure exit at all.
    //
    //       C++ emits DeleteEntry and Add as separate calls where the port
    //       carries one `Replace`; the port's app layer expands that back into
    //       delete-then-add (`developer_console_save.rs`), so the comparison
    //       expands it the same way rather than weakening the golden.
    //
    //       Scope: this is the sweep over a GIVEN section list. Discovery of
    //       that list differs deliberately — C++ takes C4Group entry order,
    //       the port normalizes to a host-independent sort — so both sides are
    //       handed the same construction order here.
    {
        let sections = golden["scenario_sections"].as_array().unwrap();
        assert_eq!(
            sections.len(),
            6,
            "PARITY DIVERGENCE in `scenario_sections`: the golden must retain its exact 6-row matrix"
        );

        // Construction order for the C++ half is `configured[1..]` followed by
        // the implicit root, which is why the port is configured root-first
        // and the switch registers it afterwards.
        struct SectionCase<'a> {
            configured: &'a [&'a str],
            switch_to: Option<&'a str>,
            modified: &'a [&'a str],
        }
        let cases = [
            SectionCase {
                configured: &[],
                switch_to: None,
                modified: &[],
            },
            SectionCase {
                configured: &["main", "Alpha", "Cave"],
                switch_to: None,
                modified: &[],
            },
            SectionCase {
                configured: &["main", "Alpha", "Cave"],
                switch_to: None,
                modified: &["Alpha"],
            },
            SectionCase {
                configured: &["main", "Alpha", "Cave"],
                switch_to: Some("Cave"),
                modified: &[],
            },
            SectionCase {
                configured: &["main", "Alpha", "Cave"],
                switch_to: Some("Cave"),
                modified: &["main", "Cave"],
            },
            SectionCase {
                configured: &["main", "Alpha", "beta", "Gamma"],
                switch_to: Some("beta"),
                modified: &["main", "Alpha", "Gamma"],
            },
        ];

        for (case, golden_case) in cases.iter().zip(sections) {
            let name = golden_case["case"].as_str().unwrap_or("?");
            let label = format!("scenario_sections[{name}]");
            let expected: Vec<&str> = golden_case["trace"]
                .as_array()
                .unwrap()
                .iter()
                .filter_map(|step| step.as_str())
                .collect();

            let mut engine = Engine::new();
            if !case.configured.is_empty() {
                let specs: Vec<_> = case
                    .configured
                    .iter()
                    .map(|name| parity_scenario_section_spec(name))
                    .collect();
                engine.configure_scenario_sections(&specs);
            }
            if let Some(target) = case.switch_to {
                assert!(
                    engine
                        .load_scenario_section(target, 0, Vec::new())
                        .unwrap_or_else(|error| panic!("{label}: section switch failed: {error}")),
                    "{label}: section switch did not find `{target}`"
                );
            }
            // Set every flag explicitly: the switch above owns whether it
            // marks the departing section modified, and this section is about
            // the sweep, not about that decision.
            for configured in case.configured {
                let modified = case.modified.contains(configured);
                let key = configured.to_ascii_lowercase();
                let section = engine
                    .scenario_section_state
                    .sections
                    .get_mut(&key)
                    .unwrap_or_else(|| panic!("{label}: `{configured}` was not configured"));
                section.modified = modified;
                // A modified section with no frozen image is rebuilt from live
                // state, and a rebuild failure is reported as a bare delete.
                // Freeze one so each row exercises the arm it names.
                section.frozen_group = modified.then(|| parity_frozen_section_image(configured));
            }

            let actual: Vec<String> = engine
                .live_c4_scenario_section_mutations()
                .into_iter()
                .flat_map(|mutation| match mutation {
                    crate::live_c4_save::LiveC4SaveScenarioSectionMutation::Delete { name } => {
                        vec![format!("delete:{name}")]
                    }
                    crate::live_c4_save::LiveC4SaveScenarioSectionMutation::Replace(section) => {
                        vec![
                            format!("delete:{}", section.name),
                            format!("add:{}", section.name),
                        ]
                    }
                })
                .collect();

            assert!(
                golden_case["ok"].as_bool().unwrap_or(false),
                "{label}: the C++ sweep has no failure exit, so no row may record one"
            );
            assert_eq!(
                actual, expected,
                "PARITY DIVERGENCE in `{label}` field `trace`: C++ golden = {expected:?}, Rust = {actual:?}"
            );
        }
    }

    // C4Script.cpp:5401-5408 and C4Game.cpp:1102-1173,5987-6009,4190-4201.
    // The C++ oracle intentionally bounds this extension to the real host
    // bool boundary, NewObject/CreateObject ordering, StatusDeactivate list
    // movement, and the active-object teardown block. Exercise the matching
    // continuation and lifecycle through the real Engine VM and consume every
    // row so this extension cannot silently become an unreferenced fixture.
    let host_lifecycle_cases = golden["scenario_section_host_lifecycle"]
        .as_array()
        .expect("scenario_section_host_lifecycle is a C++ oracle array");
    assert_eq!(
        host_lifecycle_cases.len(),
        4,
        "scenario_section_host_lifecycle must retain all bounded host/lifecycle rows"
    );
    for (case_index, case) in host_lifecycle_cases.iter().take(3).enumerate() {
        parity_scenario_section_host_case(case, case_index);
    }
    parity_scenario_section_lifecycle_case(&host_lifecycle_cases[3], 3);

    // 16i. c4value_type_tags: `GetC4VID` / `GetC4VFromID` (C4Value.cpp:368-420)
    //      and the substitution `C4Value::CompileFunc` applies over them
    //      (C4Value.cpp:722-729). This is the character every saved script
    //      value leads with.
    //
    //      Three of the pairs differ only by **case** — `i`/`I` is Int vs
    //      C4ID, `o`/`O` is object vs enumerated object, `a`/`A` is array vs
    //      any — so a slip does not fail a load, it silently changes the
    //      value's type on the way back in. And the tag a live object is
    //      written with is **not** its own: `CompileFunc` substitutes the
    //      *enumerated* tag, because the object number rather than the pointer
    //      is what goes to disk. `compile_char` carries that, and it is what
    //      the port's encoder has to agree with.
    for row in golden["c4value_type_tags"].as_array().unwrap() {
        let kind = row["type"].as_str().unwrap_or("?");
        let label = format!("c4value_type_tags[{kind}]");
        // `Reference` and `ObjectEnum` are C4Aul-internal: the port's `Value`
        // has no variant that can hold either, so there is nothing to encode.
        // Their rows stay in the golden as the C++ record of the tag space.
        let value = match kind {
            "Nil" => ScriptValue::Nil,
            "Int" => ScriptValue::Int(7),
            "Bool" => ScriptValue::Bool(true),
            "Object" => ScriptValue::Object(12),
            "C4Id" => ScriptValue::C4Id("GOLD".to_owned()),
            "String" => ScriptValue::String("x".to_owned().into()),
            "Array" => ScriptValue::Array(Vec::new()),
            "Proplist" => ScriptValue::Proplist(Default::default()),
            _ => continue,
        };
        let encoded = crate::live_c4_save::encode_value_with_current_string_ids(&value);
        let leading = encoded.chars().next().unwrap_or('?');
        // Compared as characters rather than through `expect_eq`: the whole
        // point of this section is that `o` and `O` look alike, and a
        // divergence printed as 111 against 79 hides exactly that.
        let expected = row["compile_char"]
            .as_str()
            .and_then(|tag| tag.chars().next())
            .unwrap_or('?');
        assert_eq!(
            leading, expected,
            "PARITY DIVERGENCE in `{label}` field `compile_char`: C++ golden = '{expected}', Rust = '{leading}' (encoded {encoded:?})"
        );
    }

    // 16j. save_core: `C4GameSave::SaveCore` (C4GameSave.cpp:58-107) with the
    //      `AdjustCore` override that runs at the end of it
    //      (C4GameSave.cpp:541-551, 576-585, 612-616).
    //
    //      The rule that needs *both* is `NetworkGame`: `SaveCore` zeroes it
    //      for every save, and `C4GameSaveNetwork::AdjustCore` then sets it
    //      back. The field's final value is decided by that sequence, so
    //      neither function's own test can show it.
    //
    //      Three more ride along. `NoInitialize` and `SaveGame` are written
    //      only for a **non-initial** save, so an initial one keeps the
    //      scenario's own values — and `NetworkRuntimeJoin` is `!fInitial`,
    //      which is how `network_initial` and `network_runtime` differ. The
    //      title is overwritten only when `GetKeepTitle()` is false, the same
    //      inversion the Title component has in `save_runtime_sequence`; the
    //      port expresses it structurally, since only the exact serializers
    //      take a title at all.
    for case in golden["save_core"].as_array().unwrap() {
        let name = case["case"].as_str().unwrap_or("?");
        let label = format!("save_core[{name}]");

        // The savegame icon ladder is a pure function of the destination name
        // and is compared on its own; the full save that normally reaches it
        // needs a landscape this section is not about.
        if let Some(group) = match name {
            "savegame_no_slot" => Some("Savegame.c4s"),
            "savegame_slot_1" => Some("Save1.c4s"),
            "savegame_slot_10" => Some("Save10.c4s"),
            "savegame_slot_11" => Some("Save11.c4s"),
            _ => None,
        } {
            expect_eq(
                &label,
                0,
                "icon",
                i(case, "icon"),
                i64::from(crate::live_c4_save::savegame_icon(group)),
            );
        }

        // `Origin` and the store's own title are seeded from a scenario core
        // the port keeps private, so what is compared is the derived flags
        // plus whether a passed title reaches the output at all.
        const PASSED_TITLE: &str = "Passed Title";
        let store = crate::scenario::ScenarioValueStore::default();
        let modules: Vec<String> = Vec::new();
        let bytes = match name {
            "scenario" | "origin_copied_when_empty" | "origin_kept_when_present" => {
                store.serialize_runtime_scenario_save()
            }
            "record_runtime" => {
                store.serialize_runtime_record_save(PASSED_TITLE, &modules, "", "", "")
            }
            "network_runtime" | "network_initial" => {
                store.serialize_runtime_network_save(PASSED_TITLE, &modules, "", "", "")
            }
            _ => store.serialize_runtime_savegame(PASSED_TITLE, &modules, "", "", "", 29),
        };
        let text = String::from_utf8_lossy(&bytes).into_owned();
        // The core mixes i32 and bool fields, so a value is written either as
        // `1` or as `true`; both mean set. An absent key is the writer eliding
        // a default (`push_value`), which is also unset.
        let flag = |key: &str| -> i64 {
            text.lines()
                .find_map(|line| line.strip_prefix(key))
                .map_or(0, |value| match value.trim() {
                    "true" => 1,
                    "false" => 0,
                    number => number.parse::<i64>().unwrap_or(0),
                })
        };

        // `network_initial` has no port counterpart: every serializer here is
        // the non-initial form, and an initial save writes its core through a
        // different path. Its row stays in the golden as the C++ record of the
        // `!fInitial` gate.
        if name == "network_initial" {
            continue;
        }
        expect_eq(
            &label,
            0,
            "no_initialize",
            i64::from(case["no_initialize"].as_bool().unwrap_or(false)),
            flag("NoInitialize="),
        );
        expect_eq(
            &label,
            0,
            "save_game",
            i64::from(case["save_game"].as_bool().unwrap_or(false)),
            flag("SaveGame="),
        );
        expect_eq(
            &label,
            0,
            "network_game",
            i64::from(case["network_game"].as_bool().unwrap_or(false)),
            flag("NetworkGame="),
        );
        expect_eq(
            &label,
            0,
            "network_runtime_join",
            i64::from(case["network_runtime_join"].as_bool().unwrap_or(false)),
            flag("NetworkRuntimeJoin="),
        );
        expect_eq(
            &label,
            0,
            "replay",
            i64::from(case["replay"].as_bool().unwrap_or(false)),
            flag("Replay="),
        );
        expect_eq(
            &label,
            0,
            "forced_gfx_mode",
            i(case, "forced_gfx_mode"),
            flag("ForcedGfxMode="),
        );

        // The GetKeepTitle inversion: a non-exact save keeps the scenario's own
        // title, so the caller's never appears; an exact one overwrites with it.
        let carries_passed_title = text.contains(PASSED_TITLE);
        let cpp_overwrote = case["title"].as_str() == Some("Original Title")
            || case["title"]
                .as_str()
                .is_some_and(|title| title.contains("Original Title"));
        expect_eq(
            &label,
            0,
            "title_overwritten",
            i64::from(cpp_overwrote),
            i64::from(carries_passed_title),
        );
    }

    // 17. DFA_FLOAT clamps raw C4Fixed directions to FIXED100(Physical.Float),
    // including the zero default for a real resource without [Physical]
    // (C4InfoCore.cpp:239-242; C4Object.cpp:5291-5310). Resource provenance
    // and the FXP1-shaped fixture are covered by the focused engine test; this
    // bounded oracle keeps the raw clamp itself in the C++ golden.
    for (idx, case) in golden["native_float"]
        .as_array()
        .unwrap()
        .iter()
        .enumerate()
    {
        let limit = fixed100(i(case, "physical_float") as i32);
        let mut xdir = C4Fixed::from_raw(i(case, "xdir_before") as i32);
        let mut ydir = C4Fixed::from_raw(i(case, "ydir_before") as i32);
        if ydir < -limit {
            ydir = -limit;
        }
        if ydir > limit {
            ydir = limit;
        }
        if xdir > limit {
            xdir = limit;
        }
        if xdir < -limit {
            xdir = -limit;
        }
        expect_eq(
            "native_float",
            idx,
            "xdir_after",
            i(case, "xdir_after"),
            i64::from(xdir.val()),
        );
        expect_eq(
            "native_float",
            idx,
            "ydir_after",
            i(case, "ydir_after"),
            i64::from(ydir.val()),
        );
    }
}
