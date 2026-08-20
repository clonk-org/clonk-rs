// Parity: integer operators coerce nil->0 and bool->0/1, like C++ _getInt().
//
// C++ oracle: src/C4AulExec.cpp. Every integer operator validates Int-compatible
// operands through `CheckOpPars` and then reads them with `_getInt()`, e.g.
// AB_Sum (line 540): pPar1->SetInt(pPar1->_getInt() + pPar2->_getInt()).
// src/C4Value.h:170 `_getInt() const { return Data.Int; }` — Int and Bool share
// the Data.Int storage (bool is 0/1) and nil's Data is 0. So in legacy strict
// levels nil, false, and true behave as 0, 0, and 1 in these operators.
//
// The Rust VM previously required both operands to be Value::Int and threw a
// type error otherwise, diverging from C++ whenever a comparison result (Bool)
// or a nil (e.g. a failed lookup) flowed into arithmetic.

use clonk_script::Value;

eval_test! { nil_coerces_to_zero_in_addition {
    // _getInt(nil) == 0  =>  nil + 5 == 5
    "func Test() { var empty; return empty + 5; }" => Value::Int(5);
    "func Test() { var empty; return 1 + empty; }" => Value::Int(1);
} }

eval_test! { bool_coerces_to_int_in_addition {
    "func Test() { return true + 1; }" => Value::Int(2);
    "func Test() { return false + 1; }" => Value::Int(1);
} }

#[test]
fn addition_rejects_string_operands_instead_of_concatenating() {
    for (expression, expected) in [
        (
            r#""a" + "b""#,
            "cannot apply '+' to operands of type string and string",
        ),
        (
            r#""a" + 1"#,
            "cannot apply '+' to operands of type string and int",
        ),
        (
            r#"1 + "b""#,
            "cannot apply '+' to operands of type int and string",
        ),
    ] {
        assert_eq!(
            crate::support::runtime_error(&format!("func Test() {{ return {expression}; }}"), &[],),
            expected
        );
    }
}

#[test]
fn strict_two_string_addition_errors_without_affecting_int_add_or_concat() {
    assert_eq!(
        crate::support::runtime_error("#strict 2\nfunc Test() { return \"a\" + 1; }", &[],),
        "cannot apply '+' to operands of type string and int"
    );
    assert_eq!(
        crate::support::eval(
            "#strict 2\nfunc Test() { var empty; return [true + 1, empty + 2, \"a\" .. 1]; }"
        ),
        Value::Array(vec![
            Value::Int(2),
            Value::Int(2),
            Value::String("a1".into()),
        ])
    );
}

eval_test! { comparison_result_flows_into_arithmetic {
    // (3 > 2) is bool true == 1  =>  +10 == 11
    "func Test() { return (3 > 2) + 10; }" => Value::Int(11);
    // (2 > 9) is bool false == 0  =>  5 * 0 == 0
    "func Test() { return 5 * (2 > 9); }" => Value::Int(0);
} }

eval_test! { nil_coerces_to_zero_in_comparison {
    // 0 < 5 == true
    "func Test() { var empty; return empty < 5; }" => Value::Bool(true);
} }

eval_test! { bool_coerces_in_bitwise_and_multiply {
    // 7 & 1 == 1
    "func Test() { return 7 & true; }" => Value::Int(1);
    // nil * 3 == 0
    "func Test() { var empty; return empty * 3; }" => Value::Int(0);
} }

eval_test! { bitwise_or_and_xor_share_left_associative_precedence {
    // C4ScriptOpMap gives | and ^ the same priority, so mixed chains fold
    // from the left in either order.
    "func Test() { return 1 | 1 ^ 1; }" => Value::Int(0);
    "func Test() { return 1 ^ 1 | 1; }" => Value::Int(1);
} }

eval_test! { unary_minus_coerces_like_cpp {
    // C4AulExec.cpp:468-470 AB_Neg: SetInt(-_getInt())
    "func Test() { var empty; return -empty; }" => Value::Int(0);
    "func Test() { return -true; }" => Value::Int(-1);
} }

eval_test! { unary_bitnot_coerces_like_cpp {
    // C4AulExec.cpp:460-462 AB_BitNot: SetInt(~_getInt()); ~0 == -1, ~1 == -2
    "func Test() { var empty; return ~empty; }" => Value::Int(-1);
    "func Test() { return ~true; }" => Value::Int(-2);
} }
