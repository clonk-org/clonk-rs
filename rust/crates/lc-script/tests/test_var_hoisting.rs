//! C4Aul `var` declarations are FUNCTION-scoped: every `var` in the body
//! is allocated (nil) at function entry (C4AulParse builds the whole
//! Fn->VarNamed table during parsing), so reading a var BEFORE its `var`
//! statement yields nil — never an "undefined variable" error. Real
//! content depends on it: Dynamite.c4d/Script.c:29 reads iX/iY three
//! lines before `var iX, iY, iAmount;`.

use lc_script::{Engine, Value};

#[test]
fn var_reads_before_declaration_are_nil() {
    let mut engine = Engine::new();
    engine
        .load_script(
            "#strict\n\
             func Probe() {\n\
                 var early = iX;\n\
                 var iX = 7;\n\
                 if (early == nil && iX == 7) return 1;\n\
                 return 0;\n\
             }\n",
        )
        .expect("script loads");
    let result = engine.call("Probe", &[]).expect("call succeeds");
    assert_eq!(result, Value::Int(1));
}

#[test]
fn vars_declared_in_nested_blocks_hoist_to_the_function() {
    let mut engine = Engine::new();
    engine
        .load_script(
            "#strict\n\
             func Probe(check) {\n\
                 if (check) { return iDeep; }\n\
                 while (0) { var iDeep = 3; }\n\
                 return 9;\n\
             }\n",
        )
        .expect("script loads");
    let result = engine
        .call("Probe", &[Value::Int(1)])
        .expect("call succeeds");
    assert_eq!(result, Value::Nil, "nested-block vars exist from entry");
}
