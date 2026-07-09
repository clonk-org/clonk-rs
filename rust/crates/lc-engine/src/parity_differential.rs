//! Phase-1 C++↔Rust differential parity check.
//!
//! Runs the determinism-critical primitives (`C4Fixed`, the LCG RNG, and the
//! per-frame sub-pixel accumulation) through the Rust port and asserts they are
//! byte-for-byte identical to the C++ golden oracle in
//! `parity/golden/parity_golden.json`. That golden is produced from the REAL
//! engine code (`src/Fixed.h`, `src/Fixed.cpp`'s `SineTable`, `src/C4Random.h`,
//! and `src/C4ScriptKiller.h`) by `parity/oracle/gen_golden.sh` — so this is a
//! genuine differential against the C++ oracle, not a Rust-vs-Rust regression.
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

use crate::material::{consume_corrosion_effect_rng, evaluate_corrosion};
use crate::math::{fixed10, fixed100, fixed256, fixtoi, fixtoi_prec, itofix, itofix_prec, C4Fixed};
use crate::rng::LcgRng;
use crate::{Definition, Engine, PlayerConfig, SpawnConfig, CATEGORY_OBJECT, OWNER_NONE};

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

    // 11. Movement: per-frame sub-pixel accumulation (the Theme-C core).
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
