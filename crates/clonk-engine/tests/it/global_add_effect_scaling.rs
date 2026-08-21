//! Manual scaling probe for clonk-org/clonk-rs#749.
//!
//! The issue reports that global `AddEffect` is quadratic while object-scope is
//! flat, and blames ClonkMars `03_Chaos` taking ten-plus seconds to activate on
//! it. This probe separates the candidates that ClonkMars `TEMP`
//! (`Environment.c4d/Temperature.c4d`) actually combines, because its
//! `CreateLandTempEffects` does two things at once per cell:
//!
//! ```c
//! LandTempEffects[x][y] = AddEffect("LandTemp", 0, 10, 0, 0, 0, x, y);
//! ```
//!
//! a global `AddEffect` **and** an assignment into a nested array, the latter
//! being the subject of clonk-org/clonk-rs#759. Timing them apart is the only
//! way to attribute the cost.
//!
//! ```sh
//! cargo nextest run -p clonk-engine-integration-tests --test engine_it \
//!   --run-ignored all --no-capture -E 'test(global_add_effect_scaling::)'
//! ```
//!
//! # What this measured, on an aarch64 host
//!
//! Per 100-call batch, at 0/100/200/300/400 pre-existing effects, in ms:
//!
//! | workload | b0 | b1 | b2 | b3 | b4 | total |
//! |---|---|---|---|---|---|---|
//! | global `AddEffect` | 3.0 | 5.4 | 8.0 | 9.1 | 12.6 | 38 |
//! | object `AddEffect` | 1.4 | 3.8 | 5.2 | 7.5 | 9.9 | 28 |
//! | global `AddEffect` + `Fx*Start` | 36 | 108 | 181 | 254 | 336 | 915 |
//! | object `AddEffect` + `Fx*Start` | 37 | 107 | 179 | 254 | 330 | 907 |
//! | nested array store | 0.6 | 0.7 | 0.7 | 0.7 | 0.8 | 3.4 |
//!
//! Three conclusions, none of which match the issue as written:
//!
//! 1. **The reported blowup is gone.** The issue measured the global column at
//!    29/153/397/764/1254 ms — 2.6 s for 500 adds. It is now 38 ms.
//!    clonk-org/clonk-rs#748 and clonk-org/clonk-rs#843 removed it.
//! 2. **There is no global/object asymmetry left.** The two scopes now cost the
//!    same with callbacks and without, so the issue's "strongest clue" no longer
//!    points anywhere.
//! 3. **The nested array store is flat**, so clonk-org/clonk-rs#759 does not
//!    explain the ClonkMars cost either.
//!
//! What remains is the `Fx*Start` path, and it is **C++-mandated**, not a port
//! defect. 200 adds dispatch 20,100 Start calls of which 19,900 are temporary,
//! plus 19,900 temporary Stops — exactly `n(n-1)/2` each way. `C4Effect`'s
//! constructor prepends an effect whose priority is not strictly greater than
//! the head's (C4Effect.cpp:80-94), so with a run of equal-priority effects the
//! new one lands at the front and the entire existing run is its `pNext`; the
//! constructor then calls `TempRemoveUpperEffects`/`TempReaddUpperEffects` over
//! all of it (`:128-133`, `:473-491`). Both are guarded on `pFnStart`, which is
//! precisely why the no-callback column above is cheap and the callback column
//! is quadratic.
//!
//! ClonkMars `TEMP` hits this squarely: every `LandTemp` effect shares priority
//! 10 and `FxLandTempStart` exists, and `Initialize` builds the grid twice.
//! Making that flat would require diverging from C++'s callback order, which
//! the issue's own constraints forbid. The remaining lever is the per-dispatch
//! cost (~3.7 µs here), which belongs with the effect-dispatch work in
//! clonk-org/clonk-rs#291, not with the add path.

use std::time::Instant;

use crate::support::EngineTestExt;
use clonk_engine::{Engine, SpawnConfig};
use clonk_script::Value;

const BATCHES: usize = 5;
const PER_BATCH: i32 = 100;

/// `AddGlobalBatch`/`AddObjectBatch` define no `Fx*` callbacks, so nothing
/// dispatches back into script and the whole cost is the add path. The
/// `Started` variant adds an `Fx*Start`, and `NestedStoreBatch` isolates the
/// nested array assignment with no effect at all.
const SCRIPT: &str = r#"#strict 2
static g_grid, g_starts, g_temp_starts, g_stops, g_temp_stops;

func AddGlobalBatch(int count)
{
  var i;
  while (i < count) { AddEffect("Land", 0, 50); i++; }
  return i;
}

func AddObjectBatch(int count)
{
  var i;
  while (i < count) { AddEffect("Land", this(), 50); i++; }
  return i;
}

func AddGlobalStartedBatch(int count)
{
  var i;
  while (i < count) { AddEffect("Warm", 0, 50); i++; }
  return i;
}

func AddObjectStartedBatch(int count)
{
  var i;
  while (i < count) { AddEffect("Warm", this(), 50); i++; }
  return i;
}

global func FxWarmStart(object pTarget, int iNumber, bool fTemp)
{
  g_starts++;
  if (fTemp) g_temp_starts++;
  return 0;
}

global func FxWarmStop(object pTarget, int iNumber, bool fTemp)
{
  g_stops++;
  if (fTemp) g_temp_stops++;
  return 0;
}

func StartCount() { return g_starts; }
func TempStartCount() { return g_temp_starts; }
func StopCount() { return g_stops; }
func ResetCounts() { g_starts = 0; g_temp_starts = 0; g_stops = 0; g_temp_stops = 0; return 0; }

func ResetGrid()
{
  g_grid = CreateArray(100);
  var i;
  while (i < 100) { g_grid[i] = CreateArray(0); i++; }
  return i;
}

func NestedStoreBatch(int count)
{
  var i;
  while (i < count) { g_grid[i % 100][i / 100] = i; i++; }
  return i;
}
"#;

fn batch_millis(function: &str, reset: bool) -> Vec<f64> {
    let mut engine = Engine::new();
    engine.register_test_script_definition("EFSC", "Effect scaling probe", SCRIPT);
    let object = engine.spawn_test_object(SpawnConfig::new("EFSC"));
    let index = engine
        .find_object_index(object)
        .expect("the probe object is in the object list");
    if reset {
        engine.call_test_object_function(index, "ResetGrid", vec![]);
    }

    (0..BATCHES)
        .map(|_| {
            let started = Instant::now();
            engine.call_test_object_function(index, function, vec![Value::Int(PER_BATCH)]);
            started.elapsed().as_secs_f64() * 1000.0
        })
        .collect()
}

#[test]
#[ignore = "manual profiling probe; reports numbers and asserts nothing"]
fn global_add_effect_cost_per_batch_against_object_scope() {
    let workloads = [
        ("global AddEffect", batch_millis("AddGlobalBatch", false)),
        ("object AddEffect", batch_millis("AddObjectBatch", false)),
        (
            "global AddEffect + FxStart",
            batch_millis("AddGlobalStartedBatch", false),
        ),
        (
            "object AddEffect + FxStart",
            batch_millis("AddObjectStartedBatch", false),
        ),
        ("nested array store", batch_millis("NestedStoreBatch", true)),
    ];

    {
        let mut engine = Engine::new();
        engine.register_test_script_definition("EFSC", "Effect scaling probe", SCRIPT);
        let object = engine.spawn_test_object(SpawnConfig::new("EFSC"));
        let index = engine
            .find_object_index(object)
            .expect("the probe object is in the object list");
        engine.call_test_object_function(index, "ResetCounts", vec![]);
        engine.call_test_object_function(index, "AddGlobalStartedBatch", vec![Value::Int(200)]);
        eprintln!(
            "200 global adds dispatched: starts {:?} (of which temp {:?}), stops {:?}",
            engine.call_test_object_function(index, "StartCount", vec![]),
            engine.call_test_object_function(index, "TempStartCount", vec![]),
            engine.call_test_object_function(index, "StopCount", vec![]),
        );
    }

    eprintln!("--- cost per {PER_BATCH}-call batch, at 0/100/200/300/400 existing ---");
    for (label, batches) in &workloads {
        let cells = batches
            .iter()
            .map(|ms| format!("{ms:>7.1}"))
            .collect::<Vec<_>>()
            .join(" ");
        eprintln!(
            "{label:<28}{cells}   total {:>7.1} ms  last/first {:>5.1}x",
            batches.iter().sum::<f64>(),
            batches[BATCHES - 1] / batches[0].max(f64::MIN_POSITIVE),
        );
    }
}
