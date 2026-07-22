//! Phase-1 C++↔Rust differential parity check.
//!
//! Runs the determinism-critical primitives (`C4Fixed`, the LCG RNG, and the
//! per-frame sub-pixel accumulation) through the Rust port and asserts they are
//! byte-for-byte identical to the C++ golden oracle in
//! `parity/golden/parity_golden.json`. That golden is produced from the REAL
//! engine code (`src/Fixed.h`, `src/Fixed.cpp`'s `SineTable`, `src/C4Random.h`,
//! `src/C4ScriptKiller.h`, `src/C4LandscapePath.h`, and
//! `src/C4ActionDirection.h`, `src/C4ActionCallbacks.h`, and
//! `src/C4SolidMaskBitmap.h`, plus complete `C4Object::DigOutMaterialCast`,
//! `C4Game::ShakeObjects`, `C4Object::Fling`, `C4Landscape::ClearPix`,
//! `BlastFreePix`, `BlastFree`, `ExecuteScan`, and `DoScan` bodies and the
//! bottom/top/side-flight `C4Object::ContactAction` arms) by
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

use clonk_script::{c4_hash_combine, cnv_fn, C4VType, Value as ScriptValue, ValueMap};
use serde_json::Value;

use crate::landscape::{Landscape, LandscapeRasterState, PixelGrid};
use crate::compat::{cos_func, sin_func, LandscapeOperation};
use crate::material::{consume_corrosion_effect_rng, evaluate_corrosion, MaterialSet};
use crate::math::{
    fixed10, fixed100, fixed256, fixtoi, fixtoi_prec, itofix, itofix_prec, C4Fixed,
    FixedVec2,
};
use crate::rng::LcgRng;
use crate::scenario::MapPixelClassifier;
use crate::{
    contact_action_wall_tumble_x, ActionSpec, ActionState, CommandDirection, Definition,
    DefinitionRect, DefinitionSpriteImage, DefinitionTargetRect, Direction, Engine,
    ObjectBaseGraphics, ObjectStatus, ObjectUpdate, PhysicalInfo, PhysicsSettings, PlayerConfig,
    ShapeAttachRecord, SpawnConfig, CATEGORY_LIVING, CATEGORY_OBJECT, OWNER_NONE,
};
use std::collections::HashMap;
use std::sync::Arc;

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
            .register_definition(
                Definition::from_script("CEND", "CONNECT endpoint", "#strict\n")
                    .expect("endpoint fixture compiles"),
            )
            .expect("endpoint fixture registers");
    }
    let mut action = ActionState::new("Connect");
    if geometry_break {
        action.target = Some(
            engine
                .spawn_object(
                    SpawnConfig::new("CEND").with_position(crate::Vector2::new(10, 0)),
                )
                .expect("first endpoint spawns"),
        );
        action.target2 = Some(
            engine
                .spawn_object(
                    SpawnConfig::new("CEND").with_position(crate::Vector2::new(20, 0)),
                )
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
                    (
                        "lineBreakArgumentPresent".to_string(),
                        ScriptValue::Int(0),
                    ),
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
        digger.set_shape_rect(Some(DefinitionRect::new(
            -2,
            shape_y,
            4,
            shape_height,
        )));
        let mut gem =
            Definition::from_script("GEM_", "Gem", "").expect("gem fixture compiles");
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
        let grid = PixelGrid::new(
            5,
            5,
            pixels,
            densities,
            material_names,
            vec![None; 128],
        );
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
                engine.tick_without_snapshot().expect("landscape scan oracle frame executes");
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
            .exec_contact_action(object_index, crate::CNAT_BOTTOM, &definition_id, &[])
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
        let mut definition = Definition::from_script("CFTS", "Contact top/side oracle", "#strict\n")
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
            .exec_contact_action(object_index, contact, &definition_id, &[])
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
        let bare_result = |function: &str, bare: &mut clonk_script::Engine| {
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
        full_frame.tick_without_snapshot().expect("oracle frame executes");
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
        full_frame.tick_without_snapshot().expect("oracle frame executes");
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
        engine.tick_without_snapshot().expect("callback fixture frame executes");
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
