// Test for increment/decrement on various lvalue types

// ++EffectVar(0, pTarget, iEffect)
crate::support::compile_case!(
    effectvar_three_args_pre_increment,
    r#"func Test() { var pTarget, iEffect; ++EffectVar(0, pTarget, iEffect); }"#
);

// --EffectVar(0, pTarget, iEffect)
crate::support::compile_case!(
    effectvar_three_args_pre_decrement,
    r#"func Test() { var pTarget, iEffect; --EffectVar(0, pTarget, iEffect); }"#
);

// EffectVar(0, pTarget, iEffect)++
crate::support::compile_case!(
    effectvar_three_args_post_increment,
    r#"func Test() { var pTarget, iEffect; EffectVar(0, pTarget, iEffect)++; }"#
);

// EffectVar(0, pTarget, iEffect)--
crate::support::compile_case!(
    effectvar_three_args_post_decrement,
    r#"func Test() { var pTarget, iEffect; EffectVar(0, pTarget, iEffect)--; }"#
);

// ++LocalN("key", obj)
crate::support::compile_case!(
    localn_two_args_pre_increment,
    r#"func Test() { var obj; ++LocalN("count", obj); }"#
);

// LocalN("key", obj)--
crate::support::compile_case!(
    localn_two_args_post_decrement,
    r#"func Test() { var obj; LocalN("active", obj)--; }"#
);

// ++LocalN("key")
crate::support::compile_case!(
    localn_one_arg_pre_increment,
    r#"func Test() { ++LocalN("counter"); }"#
);

// --Var()
crate::support::compile_case!(var_zero_args_pre_decrement, r#"func Test() { --Var(); }"#);

// Var()++
crate::support::compile_case!(var_zero_args_post_increment, r#"func Test() { Var()++; }"#);

#[test]
fn increment_resolves_side_effectful_var_lvalue_once() {
    // C++ compiles the operand to one C4Value reference and AB_Inc1 mutates
    // that reference in place (C4AulExec.cpp:450-454). ComboMenu::CheckSpells
    // relies on this exact nested expression when counting one MGUP candidate:
    // `++Var(Var(13+iCount++*2) = key)`.
    let mut engine = clonk_script::Engine::new();
    engine
        .load_script(
            r#"
                #strict
                func Test()
                {
                    var iCount = 0;
                    ++Var(Var(13 + iCount++ * 2) = 4);
                    return [iCount, Var(4), Var(13), Var(15)];
                }
            "#,
        )
        .expect("nested Var lvalue compiles");

    assert_eq!(
        engine.call("Test", &[]).expect("nested increment executes"),
        clonk_script::Value::Array(vec![
            clonk_script::Value::Int(1),
            clonk_script::Value::Int(1),
            clonk_script::Value::Int(4),
            clonk_script::Value::Nil,
        ])
    );
}

#[test]
fn increment_resolves_side_effectful_effectvar_lvalue_once() {
    // EffectVar is also a reference-returning C++ engine function. Its slot
    // address is evaluated before AB_Inc1 and cannot be recomputed for the
    // write (C4AulExec.cpp:450-454; C4Script.cpp:5569-5594).
    let mut engine = clonk_script::Engine::new();
    engine
        .load_script(
            r#"
                #strict
                func Test()
                {
                    var i = 0;
                    var result = ++EffectVar(i++);
                    return [i, result];
                }
            "#,
        )
        .expect("side-effectful EffectVar lvalue compiles");

    assert_eq!(
        engine
            .call("Test", &[])
            .expect("nested EffectVar increment executes"),
        clonk_script::Value::Array(vec![
            clonk_script::Value::Int(1),
            clonk_script::Value::Int(1),
        ])
    );
}

#[test]
fn indexed_effectvar_array_assignment_writes_through_the_returned_reference() {
    // FnEffectVar returns EffectVars[i].GetRef(); AB_ARRAYA_R retains an
    // element reference and AB_Set writes through it (C4Script.cpp:5571-5580;
    // C4AulExec.cpp:858-865,906-919). MTNL's timer uses this exact shape,
    // including the side-effectful EffectVar index.
    let mut engine = clonk_script::Engine::new();
    let slots = std::sync::Arc::new(std::sync::Mutex::new(std::collections::HashMap::new()));
    let host_slots = slots.clone();
    engine.register_host_function("EffectVar", move |args| {
        let index = match args.first() {
            Some(clonk_script::Value::Int(index)) => *index,
            _ => 0,
        };
        let mut slots = host_slots.lock().expect("EffectVar test slots lock");
        if let Some(value) = args.get(3) {
            slots.insert(index, value.clone());
        }
        Ok(slots
            .get(&index)
            .cloned()
            .unwrap_or(clonk_script::Value::Nil))
    });
    engine
        .load_script(
            r#"
                #strict
                func Test()
                {
                    EffectVar(7, 0, 1) = [10, 20];
                    EffectVar(3, 0, 1) = 0;
                    EffectVar(7, 0, 1)[EffectVar(3, 0, 1)++] = 42;
                    return [EffectVar(7, 0, 1), EffectVar(3, 0, 1)];
                }
            "#,
        )
        .expect("indexed EffectVar assignment compiles");

    assert_eq!(
        engine
            .call("Test", &[])
            .expect("indexed EffectVar assignment executes"),
        clonk_script::Value::Array(vec![
            clonk_script::Value::Array(vec![
                clonk_script::Value::Int(42),
                clonk_script::Value::Int(20),
            ]),
            clonk_script::Value::Int(1),
        ])
    );
}

// ++Local()
crate::support::compile_case!(
    local_zero_args_pre_increment,
    r#"func Test() { ++Local(); }"#
);

// ++Local(0, obj)
crate::support::compile_case!(
    local_two_args_pre_increment,
    r#"func Test() { var obj; ++Local(0, obj); }"#
);

// Var(0, obj)--
crate::support::compile_case!(
    var_two_args_post_decrement,
    r#"func Test() { var obj; Var(0, obj)--; }"#
);

// Exact pattern from WARP line 147
crate::support::compile_case!(
    warp_line_147_exact_pattern,
    r#"func Test() { var pTarget, iEffect, pObj; EffectVar(++EffectVar(0, pTarget, iEffect), pTarget, iEffect) = pObj; }"#
);

// Pattern from Skyrace.c4s: --Var() in if condition
crate::support::compile_case!(
    skyrace_var_decrement_pattern,
    r#"func Test() { if (!--Var()) return("Done"); }"#
);

// Pattern: if((--EffectVar(0, pTarget, iEffectNumber))<=0)
crate::support::compile_case!(
    nested_increment_in_condition,
    r#"func Test() { var pTarget, iEffectNumber; if((--EffectVar(0, pTarget, iEffectNumber))<=0) return(-1); }"#
);
