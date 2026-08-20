//! `++`/`--` on nil: AB_Inc1/AB_Dec1 run CheckOpPar<C4V_Int> which converts
//! nil to 0 before the operation (C4AulExec.cpp:450-458 via C4Value
//! FnCnvGuess, C4Value.cpp:453-466) — `var i; while(i++ < n)` is the standard
//! loop idiom in pre-strict content (e.g. Objects.c4d Loam placer).

use clonk_script::Value;

run_cases! {
    postfix_increment_on_nil_counts_from_zero: r#"
        global func Probe() {
            var i;
            var hits;
            while (i++ < 3) hits = hits + 1;
            return i;
        }
    "#, "Probe", &[] => Value::Int(4);

    prefix_increment_on_nil_yields_one: r#"
        global func Probe() {
            var i;
            return ++i;
        }
    "#, "Probe", &[] => Value::Int(1);

    postfix_decrement_on_nil_yields_zero_then_minus_one: r#"
        global func Probe() {
            var i;
            var first = i--;
            return first * 100 + i;
        }
    "#, "Probe", &[] => Value::Int(-1);
}
