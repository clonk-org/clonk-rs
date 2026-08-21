//! Manual probe: how does indexed assignment scale with array length?
//!
//! clonk-org/clonk-rs#759 reports that `a[i] = v` costs O(len), making an
//! `n`x`n` build O(n³). This reproduces it in `clonk-script` alone, with no
//! host functions involved, so the next attempt starts from a runnable
//! measurement.
//!
//! It reports numbers and asserts only that the grid was built, so it is
//! `#[ignore]`d like the tree's other manual timing probes. Run it with:
//!
//! ```sh
//! cargo nextest run -p clonk-script --run-ignored all --no-capture \
//!     -E 'test(test_array_assignment_scaling::)'
//! ```
//!
//! # Recorded measurement
//!
//! Warm process, aarch64 host, nextest's `test` profile, ns per assignment.
//!
//! | side | nested, before | nested, after | flat, before | flat, after |
//! |---|---|---|---|---|
//! | 20 | 5,887 | 3,426 | 4,339 | 2,324 |
//! | 40 | 15,655 | 7,157 | 33,561 | 7,408 |
//! | 80 | **53,447** | **22,465** | **93,755** | **22,625** |
//!
//! # What the two copies were
//!
//! The `flat` column is what found them. Assigning into a *flat* array of the
//! same total length turned out to be just as superlinear as the nested build,
//! which ruled out the nesting and pointed at the single element write:
//!
//! * `AssignmentTarget::Index` asked the lvalue for its base through `read()`
//!   purely to learn whether it was an object, and that copied the whole
//!   container. It now asks `object_target()`, which walks by reference and
//!   clones only an object -- a bare id.
//! * `read_path` opened with `value.clone()` of the **root**, so every read
//!   through a path copied the entire container before walking it. It now walks
//!   by reference and clones only what it returns.
//!
//! An earlier attempt replaced only the first of those and measured *slower*,
//! which is why it was reverted. The lesson is in the `flat` column: without a
//! control that holds the element count fixed and removes the nesting, a
//! partial fix is indistinguishable from no fix.
//!
//! # What is left
//!
//! The curve is no longer dominated by a copy, but it is not flat either --
//! roughly 3x for 2x the side, against 4x for a true O(len) per write. What
//! remains is small enough that reallocation as the array grows and cache
//! behaviour are plausible explanations, and separating those needs a probe
//! that pre-sizes the array rather than growing it.

use clonk_script::{Engine, Value};
use std::time::Instant;

/// The same number of assignments into a FLAT array of the same total length,
/// so nesting is the only difference. If this is flat and `build_grid` is not,
/// the cost is in walking/rebuilding the nesting rather than in the element
/// write itself.
fn build_flat(side: i32) -> f64 {
    let total = side * side;
    let source = format!(
        "#strict 2\n\
         global func Test() {{\n\
         \tvar a = [];\n\
         \tfor (var i = 0; i < {total}; i++) a[i] = i;\n\
         \treturn a[{last}];\n\
         }}\n",
        total = total,
        last = total - 1
    );
    let mut engine = Engine::new();
    engine.load_script(&source).expect("flat script loads");
    let started = Instant::now();
    let result = engine.call("Test", &[]).expect("flat probe runs");
    let elapsed = started.elapsed().as_secs_f64();
    assert_eq!(result, Value::Int(total - 1), "the flat array was built");
    elapsed * 1e9 / f64::from(total)
}

fn build_grid(side: i32) -> f64 {
    let source = format!(
        "#strict 2\n\
         global func Test() {{\n\
         \tvar a = [];\n\
         \tfor (var x = 0; x < {side}; x++) {{\n\
         \t\ta[x] = [];\n\
         \t\tfor (var y = 0; y < {side}; y++) a[x][y] = y;\n\
         \t}}\n\
         \treturn a[{last}][{last}];\n\
         }}\n",
        side = side,
        last = side - 1
    );
    let mut engine = Engine::new();
    engine.load_script(&source).expect("grid script loads");
    let start = Instant::now();
    let result = engine.call("Test", &[]).expect("grid builds");
    let elapsed = start.elapsed().as_secs_f64();
    assert_eq!(
        result,
        Value::Int(side - 1),
        "the grid must actually be built"
    );
    elapsed * 1e9 / f64::from(side * side)
}

#[test]
#[ignore = "manual timing probe"]
fn nested_array_assignment_cost_per_element() {
    // Warm the process so the first measured size does not carry setup.
    let _ = build_grid(10);

    eprintln!("  side | nested ns | flat ns (same total assignments)");
    let mut previous: Option<(i32, f64)> = None;
    for side in [20, 40, 80] {
        let each = build_grid(side);
        let flat = build_flat(side);
        eprintln!("  {side:>4} | {each:>9.0} | {flat:>7.0}");
        if let Some((previous_side, previous_each)) = previous {
            eprintln!(
                "       | {:.1}x the cost for {:.0}x the side",
                each / previous_each,
                f64::from(side) / f64::from(previous_side)
            );
        }
        previous = Some((side, each));
    }
}
