//! Phase-1 C++↔Rust differential parity check.
//!
//! Runs the determinism-critical primitives (`C4Fixed`, the LCG RNG, and the
//! per-frame sub-pixel accumulation) through the Rust port and asserts they are
//! byte-for-byte identical to the C++ golden oracle in
//! `parity/golden/parity_golden.json`. That golden is produced from the REAL
//! engine code (`src/Fixed.h`, `src/Fixed.cpp`'s `SineTable`, `src/C4Random.h`,
//! `src/C4ScriptKiller.h`, `src/C4LandscapePath.h`, and
//! `src/C4ActionDirection.h`, `src/C4ActionCallbacks.h`, and
//! `src/C4SolidMaskBitmap.h`, plus complete `C4Game::ShakeObjects` and
//! `C4Object::Fling` bodies) by
//! `parity/oracle/gen_golden.sh` — so this is a genuine differential against
//! the C++ oracle, not a Rust-vs-Rust regression.
//!
//! This gates Theme C (wiring fixed precision through physics): the gravity /
//! velocity sub-pixel accumulation the harness exercises is exactly the
//! arithmetic Theme C extends. The C++ per-pixel collision loop (item 4) is out
//! of scope here and is the subject of a future live-bridge differential.
//!
//! On any divergence the test panics with the first mismatch (section, index,
//! field, C++ value vs Rust value).

use lc_script::{c4_hash_combine, cnv_fn, C4VType, Value as ScriptValue};
use serde_json::Value;

use crate::landscape::{Landscape, PixelGrid};
use crate::material::{consume_corrosion_effect_rng, evaluate_corrosion};
use crate::math::{
    fixed10, fixed100, fixed256, fixtoi, fixtoi_prec, itofix, itofix_prec, C4Fixed,
    FixedVec2,
};
use crate::rng::LcgRng;
use crate::{
    ActionSpec, ActionState, CommandDirection, Definition, DefinitionRect, DefinitionSpriteImage,
    DefinitionTargetRect, Direction, Engine, ObjectBaseGraphics, ObjectStatus, ObjectUpdate,
    PhysicalInfo, PhysicsSettings, PlayerConfig, ShapeAttachRecord, SpawnConfig, CATEGORY_LIVING,
    CATEGORY_OBJECT, OWNER_NONE,
};
use std::collections::HashMap;
use std::sync::Arc;

const GOLDEN: &str = concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/../../../parity/golden/parity_golden.json"
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

/// Assert two values are equal, panicking with a precise first-divergence report.
fn expect_eq(section: &str, index: usize, field: &str, cpp: i64, rust: i64) {
    assert_eq!(
        cpp, rust,
        "PARITY DIVERGENCE in `{section}` entry {index} field `{field}`: \
         C++ golden = {cpp}, Rust = {rust}"
    );
}

fn expect_eq_u64(section: &str, index: usize, field: &str, cpp: u64, rust: u64) {
    assert_eq!(
        cpp, rust,
        "PARITY DIVERGENCE in `{section}` entry {index} field `{field}`: \
         C++ golden = {cpp}, Rust = {rust}"
    );
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
        "string" => ScriptValue::String("x".to_string()),
        "array" => ScriptValue::Array(Vec::new()),
        "map" => ScriptValue::Proplist(std::collections::HashMap::new()),
        other => panic!("unknown script_value_convert case `{other}`"),
    }
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
                .with_fixed_velocity(FixedVec2::new(
                    C4Fixed::ZERO,
                    C4Fixed::from_raw(-6_556),
                ))
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

#[test]
fn parity_differential_matches_cpp_golden() {
    let golden = load_golden();

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

    // 6b. C4Game::ShakeObjects master-list selection, RNG consumption, and
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
        engine.exec_list = master_order.iter().rev().copied().collect();

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

        let mut map = std::collections::HashMap::new();
        map.insert("a".to_string(), ScriptValue::Int(1));
        map.insert(
            "b".to_string(),
            ScriptValue::Array(vec![ScriptValue::Int(2), ScriptValue::Int(3)]),
        );
        let mut reversed = std::collections::HashMap::new();
        reversed.insert(
            "b".to_string(),
            ScriptValue::Array(vec![ScriptValue::Int(2), ScriptValue::Int(3)]),
        );
        reversed.insert("a".to_string(), ScriptValue::Int(1));

        let cases = [
            ("nil", ScriptValue::Nil),
            ("int_zero", ScriptValue::Int(0)),
            ("int_42", ScriptValue::Int(42)),
            ("int_minus_one", ScriptValue::Int(-1)),
            ("bool_false", ScriptValue::Bool(false)),
            ("bool_true", ScriptValue::Bool(true)),
            ("id_CLNK", ScriptValue::C4Id("CLNK".to_string())),
            ("id_1337", ScriptValue::C4Id("1337".to_string())),
            ("string_empty", ScriptValue::String(String::new())),
            ("string_alpha", ScriptValue::String("alpha".to_string())),
            (
                "string_16",
                ScriptValue::String("abcdefghijklmnop".to_string()),
            ),
            (
                "string_24",
                ScriptValue::String("abcdefghijklmnopqrstuvwx".to_string()),
            ),
            (
                "string_40",
                ScriptValue::String("abcdefghijklmnopqrstuvwxyz0123456789ABCD".to_string()),
            ),
            (
                "string_80",
                ScriptValue::String(
                    "0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef"
                        .to_string(),
                ),
            ),
            (
                "array_1_true_x",
                ScriptValue::Array(vec![
                    ScriptValue::Int(1),
                    ScriptValue::Bool(true),
                    ScriptValue::String("x".to_string()),
                ]),
            ),
            ("map_a1_b23", ScriptValue::Proplist(map)),
            ("map_b23_a1", ScriptValue::Proplist(reversed)),
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
            .register_definition(
                Definition::from_script("CALL", "Caller", caller_script)
                    .expect("killer differential caller compiles"),
            )
            .expect("killer differential caller registers");
        engine
            .register_definition(
                Definition::from_script("OTHR", "Other", "#strict\n")
                    .expect("killer differential target compiles"),
            )
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
        // a bare lc-script engine. This matches C4ScriptKiller's null/null
        // oracle cases and pins the NO_OWNER/false fallbacks.
        let mut bare = lc_script::Engine::new();
        crate::compat::register_host_functions(&mut bare);
        bare.add_script(
            lc_script::Script::compile(
                "global func ReadNoContext() { return GetKiller(); }\n\
                 global func WriteNoContext() { return SetKiller(1); }\n",
            )
            .expect("bare killer differential script compiles"),
        );
        let bare_result = |function: &str, bare: &mut lc_script::Engine| {
            match bare
                .call(function, &[])
                .unwrap_or_else(|error| panic!("bare killer call `{function}` failed: {error}"))
            {
                ScriptValue::Int(value) => i64::from(value),
                ScriptValue::Bool(value) => i64::from(value),
                value => panic!("bare killer call `{function}` returned {value:?}"),
            }
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
        full_frame.tick().expect("oracle frame executes");
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
        full_frame.tick().expect("oracle frame executes");
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
        engine.tick().expect("callback fixture frame executes");
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
}
