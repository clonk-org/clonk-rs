//! C4Aul `var` declarations are FUNCTION-scoped: every `var` in the body
//! is allocated (nil) at function entry (C4AulParse builds the whole
//! Fn->VarNamed table during parsing), so reading a var BEFORE its `var`
//! statement yields nil — never an "undefined variable" error. Real
//! content depends on it: Dynamite.c4d/Script.c:29 reads iX/iY three
//! lines before `var iX, iY, iAmount;`.

use clonk_script::{Engine, Value};

fn eval(source: &str) -> Value {
    let mut engine = Engine::new();
    engine.load_script(source).expect("script loads");
    engine.call("Test", &[]).expect("call succeeds")
}

#[test]
fn var_reads_before_declaration_are_nil() {
    let mut engine = Engine::new();
    engine
        .load_script(
            "#strict\n\
             func Probe() {\n\
                 var early = iX;\n\
                 var iX = 7;\n\
                 if (early == 0 && iX == 7) return 1;\n\
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

#[test]
fn classic_for_init_var_inside_if_remains_visible_after_the_block() {
    assert_eq!(
        eval(
            "func Test() {\n\
                 if (true) {\n\
                     for (var i = 0; i < 3; i++) {}\n\
                 }\n\
                 return i;\n\
             }",
        ),
        Value::Int(3)
    );
}

#[test]
fn classic_for_init_var_at_function_top_level_still_works() {
    assert_eq!(
        eval(
            "func Test() {\n\
                 for (var i = 0; i < 3; i++) {}\n\
                 return i;\n\
             }",
        ),
        Value::Int(3)
    );
}

#[test]
fn array_for_in_binder_inside_if_keeps_its_final_item_after_the_block() {
    assert_eq!(
        eval(
            "#strict\n\
             func Test() {\n\
                 if (true) {\n\
                     for (var item in [4, 8, 12]) {}\n\
                 }\n\
                 return item;\n\
             }",
        ),
        Value::Int(12)
    );
}
