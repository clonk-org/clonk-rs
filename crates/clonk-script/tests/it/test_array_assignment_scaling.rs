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
//! Warm process, aarch64 host, nextest's `test` profile.
//!
//! | build | assignments | ns per assignment |
//! |---|---|---|
//! | 20x20 | 400 | 7,397 |
//! | 80x80 | 6,400 | **67,165** |
//!
//! A four-fold side gives a nine-fold per-assignment cost — the O(len) copy the
//! issue describes, still present.
//!
//! # What the cost is not
//!
//! The obvious suspect is `AssignmentTarget::Index` reading the target through
//! the lvalue (`reference.read()`) purely to learn whether it is an object,
//! which clones the whole array. It is not the cause: replacing that read with
//! a borrow-only `peek_object` that never copies left the curve unchanged —
//! 13,679 ns at 20x20 against 147,390 ns at 80x80, the same nine-fold growth.
//! That change was therefore reverted rather than landed.
//!
//! So the copy is somewhere the element write reaches after the lvalue is
//! built. `write_path` mutates in place through `&mut Value` and
//! `detach_container_identity_at_path` early-returns unless the identity is
//! genuinely shared (`Rc::strong_count > 1`), so the next places to look are
//! how the assignment obtains its root cell and whether the identity bookkeeping
//! around it copies the container.

use clonk_script::{Engine, Value};
use std::time::Instant;

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

    eprintln!("  side | ns per assignment");
    let mut previous: Option<(i32, f64)> = None;
    for side in [20, 40, 80] {
        let each = build_grid(side);
        eprintln!("  {side:>4} | {each:>17.0}");
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
