//! C4Aul `var` declarations are FUNCTION-scoped: every `var` in the body
//! is allocated (nil) at function entry (C4AulParse builds the whole
//! Fn->VarNamed table during parsing), so reading a var BEFORE its `var`
//! statement yields nil — never an "undefined variable" error. Real
//! content depends on it: Dynamite.c4d/Script.c:29 reads iX/iY three
//! lines before `var iX, iY, iAmount;`.

use clonk_script::{Engine, Value};

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

eval_cases! {
    classic_for_init_var_inside_if_remains_visible_after_the_block:
            "func Test() {\n\
                 if (true) {\n\
                     for (var i = 0; i < 3; i++) {}\n\
                 }\n\
                 return i;\n\
             }" => Value::Int(3);

    classic_for_init_var_at_function_top_level_still_works:
            "func Test() {\n\
                 for (var i = 0; i < 3; i++) {}\n\
                 return i;\n\
             }" => Value::Int(3);

    // C4AulParseState::Parse_Var emits AB_IVARN only when `=` follows
    // (C4AulParse.cpp:3252-3283), so this declaration must leave the value
    // from the preceding loop intact.
    for_init_var_without_initializer_preserves_existing_value:
            "func Test() {\n\
                 var total = 0;\n\
                 for (var i = 0; i < 3; i++) total += 1;\n\
                 for (var i; i < 5; i++) total += 10;\n\
                 return total;\n\
             }" => Value::Int(23);

    // A continue keeps this function on the AST-walk VM path, which must
    // preserve the same function-scoped value as the compiled path.
    for_init_var_without_initializer_preserves_existing_value_in_ast_vm:
            "func Test() {\n\
                 var total = 0;\n\
                 for (var i = 0; i < 3; i++) {\n\
                     if (false) continue;\n\
                     total += 1;\n\
                 }\n\
                 for (var i; i < 5; i++) total += 10;\n\
                 return total;\n\
             }" => Value::Int(23);

    array_for_in_binder_inside_if_keeps_its_final_item_after_the_block:
            "#strict\n\
             func Test() {\n\
                 if (true) {\n\
                     for (var item in [4, 8, 12]) {}\n\
                 }\n\
                 return item;\n\
             }" => Value::Int(12);
}

eval_cases! {
    // A `var` statement without `=` compiles to no bytecode at all:
    // C4AulParseState::Parse_Var emits AB_IVARN only inside its `=` branch
    // (C4AulParse.cpp:3252-3283), so the hoisted slot keeps its value.
    plain_var_declaration_without_initializer_preserves_existing_value:
            "func Test() {\n\
                 var x = 7;\n\
                 var x;\n\
                 return x;\n\
             }" => Value::Int(7);

    // An unreachable `continue` denies the function a compiled plan, so this
    // runs on the AST-walk VM and must agree with the compiled form above.
    plain_var_declaration_without_initializer_preserves_existing_value_in_ast_vm:
            "func Test() {\n\
                 var x = 7;\n\
                 while (false) { continue; }\n\
                 var x;\n\
                 return x;\n\
             }" => Value::Int(7);

    // The same holds for a multi-name declaration: none of `a`, `b`, `c` has
    // an `=`, so all three keep the values assigned before the statement.
    plain_multi_var_declaration_preserves_every_existing_value_in_ast_vm:
            "func Test() {\n\
                 var a = 1, b = 2, c = 4;\n\
                 while (false) { continue; }\n\
                 var a, b, c;\n\
                 return a + b + c;\n\
             }" => Value::Int(7);

    // Only the name that carries `=` is stored; the bare ones are untouched.
    mixed_var_declaration_stores_only_the_initialized_name_in_ast_vm:
            "func Test() {\n\
                 var a = 1, b = 2;\n\
                 while (false) { continue; }\n\
                 var a, b = 30;\n\
                 return a * 100 + b;\n\
             }" => Value::Int(130);
}
