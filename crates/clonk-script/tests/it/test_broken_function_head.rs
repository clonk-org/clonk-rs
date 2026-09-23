//! C4Aul registers a new-style function before it reads the parameter list,
//! and adds each parameter name as it goes (C4AulParse.cpp:1602-1695). A list
//! that fails to parse is reported, but the function stays, and the parser pass
//! compiles it from just after its name (C4AulParse.cpp:128-129, 1400-1427,
//! 3551). The pinned oracle confirms it: `GameCall("Split", 1)` on the script
//! below throws `syntax error: see previous parser error for details.` and
//! aborts its caller, instead of finding nothing.

use clonk_script::{Engine, Value};

const BROKEN_HEAD: &str = "#strict\nfunc Split(first == second)\n{\n  return 0;\n}\n";

#[test]
fn a_function_whose_parameter_list_fails_to_parse_is_kept() {
    let mut engine = Engine::new();
    engine
        .load_script(BROKEN_HEAD)
        .expect("a broken head is quarantined");

    assert!(engine.has_function("Split"));
    let error = engine
        .call("Split", &[Value::Int(1)])
        .expect_err("the rest of the head is compiled as a body that fails");
    assert!(
        !error.to_string().contains("unknown function"),
        "got: {error}"
    );
}

#[test]
fn a_strict_two_head_without_its_brace_compiles_the_rest_as_its_body() {
    // Under #strict 2 a head without `{` is an error. C4Aul throws it after the
    // closed parameter list moved the body start past the ')'
    // (C4AulParse.cpp:1687, 1698-1704). The pinned oracle's
    // `GameCall("NoBrace", 41)` on this script returned 42.
    let mut engine = Engine::new();
    engine
        .load_script("#strict 2\n\nfunc NoBrace(a) return a + 1;\n\nfunc Next() { return 0; }\n")
        .expect("a broken head is quarantined");

    assert_eq!(
        engine
            .call("NoBrace", &[Value::Int(41)])
            .expect("the statement after the head is the body"),
        Value::Int(42)
    );
    assert!(engine.has_function("Next"));
}

#[test]
fn a_variadic_head_without_its_parenthesis_compiles_the_rest_as_its_body() {
    // C4Aul starts the body after `...` before it matches the ')'
    // (C4AulParse.cpp:1642-1646), so the block after the broken head is the
    // body. The pinned oracle's `GameCall("Variadic", 41)` on this script
    // returned 42.
    let mut engine = Engine::new();
    engine
        .load_script("#strict\n\nfunc Variadic(first, ... { return first + 1; }\n")
        .expect("a broken head is quarantined");

    assert_eq!(
        engine
            .call("Variadic", &[Value::Int(41)])
            .expect("the body after the ellipsis runs"),
        Value::Int(42)
    );
}
