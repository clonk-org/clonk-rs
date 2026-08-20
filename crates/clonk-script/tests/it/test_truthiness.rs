// Parity: a non-nil string/array/proplist is truthy even when empty.
//
// C++ oracle: control-flow truthiness uses `C4Value::operator bool`
// (C4Value.h:185 -> `C4V_Data::operator bool`, :76), which is raw-nonzero on the
// underlying `Data` union — a *pointer* for strings/arrays/maps. A non-nil
// C4String/C4ValueArray/C4ValueHash has a non-null pointer, so empty `""`, `[]`,
// and `{}` are TRUTHY; only nil and integer/bool zero are falsy. (`if`, `while`,
// `!`, `&&`, `||` all key off this, via AB_CONDN/AB_JUMPAND, C4AulExec.cpp.)
//
// The Rust VM previously used `!is_empty()` for strings/arrays/proplists, making
// empty containers falsy — a divergence for content that treats a string/array as
// a present/absent flag.

use clonk_script::Value;

eval_cases! {
    empty_string_is_truthy_like_cpp:
        r#"func Test() { if ("") { return 1; } return 0; }"# => Value::Int(1);
    empty_array_is_truthy_like_cpp:
        "#strict\nfunc Test() { if ([]) { return 1; } return 0; }" => Value::Int(1);

    // "" is truthy, so !"" is false.
    not_of_empty_string_is_false:
        r#"func Test() { return !""; }"# => Value::Bool(false);
    nil_remains_falsy:
        "#strict 3\nfunc Test() { if (nil) { return 1; } return 0; }" => Value::Int(0);
    zero_int_remains_falsy:
        "#strict 3\nfunc Test() { if (0) { return 1; } return 0; }" => Value::Int(0);
}
