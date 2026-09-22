//! C4Aul keywords are contextual in variable declarations too: the C++
//! tokenizer emits plain ATT_IDTF for every word and `Parse_Var` takes the
//! identifier as-is, so `var func, objhgt = ...` is legal — real content
//! relies on it (planet/System.c4g/Commits.c:269 `var func, objhgt=...`).

use clonk_script::Value;

run_cases! {
    // Commits.c declares `var func, objhgt=...` and never touches `func`
    // again; the declaration alone must compile and the sibling initializer
    // must work.
    func_keyword_is_a_valid_var_name: r#"
        global func Probe() {
            var func, objhgt = 5;
            return objhgt + 2;
    }
"#, "Probe", &[] => Value::Int(7);

    func_keyword_binding_can_be_read_and_reassigned: r#"
        func Test() {
            var func = "BuyItem";
            func = "BuyPack";
            return func;
        }
    "#, "Test", &[] => Value::String("BuyPack".into());

    // `Parse_Statement` tries parameters, `var`s, locals and statics before it
    // looks at the access keywords (C4AulParse.cpp:1976-2011, 2168-2171), so a
    // declared `global` is a variable in statement position too. Modern
    // Combat's SoundEffects keeps `local ... global` and assigns it in `Set`.
    local_named_global_is_assigned_in_a_statement: r#"
        #strict 2
        local global;
        func Test(bool fGlobal) {
            global = fGlobal;
            return global;
        }
    "#, "Test", &[Value::Bool(true)] => Value::Bool(true);

    var_named_global_is_assigned_in_a_statement: r#"
        #strict 2
        func Test(int x) {
            var global;
            global = x;
            return global + 1;
        }
    "#, "Test", &[Value::Int(4)] => Value::Int(5);
}
