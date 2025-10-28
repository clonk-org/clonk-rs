// Test for increment/decrement on various lvalue types

#[test]
fn effectvar_three_args_pre_increment() {
    // ++EffectVar(0, pTarget, iEffect)
    let source = r#"func Test() { var pTarget, iEffect; ++EffectVar(0, pTarget, iEffect); }"#;
    let result = lc_script::Script::compile(source);
    if let Err(e) = &result {
        eprintln!("Error: line {}, col {}: {}", e.line(), e.column(), e.message());
    }
    assert!(result.is_ok());
}

#[test]
fn effectvar_three_args_pre_decrement() {
    // --EffectVar(0, pTarget, iEffect)
    let source = r#"func Test() { var pTarget, iEffect; --EffectVar(0, pTarget, iEffect); }"#;
    let result = lc_script::Script::compile(source);
    if let Err(e) = &result {
        eprintln!("Error: line {}, col {}: {}", e.line(), e.column(), e.message());
    }
    assert!(result.is_ok());
}

#[test]
fn effectvar_three_args_post_increment() {
    // EffectVar(0, pTarget, iEffect)++
    let source = r#"func Test() { var pTarget, iEffect; EffectVar(0, pTarget, iEffect)++; }"#;
    let result = lc_script::Script::compile(source);
    if let Err(e) = &result {
        eprintln!("Error: line {}, col {}: {}", e.line(), e.column(), e.message());
    }
    assert!(result.is_ok());
}

#[test]
fn effectvar_three_args_post_decrement() {
    // EffectVar(0, pTarget, iEffect)--
    let source = r#"func Test() { var pTarget, iEffect; EffectVar(0, pTarget, iEffect)--; }"#;
    let result = lc_script::Script::compile(source);
    if let Err(e) = &result {
        eprintln!("Error: line {}, col {}: {}", e.line(), e.column(), e.message());
    }
    assert!(result.is_ok());
}

#[test]
fn localn_two_args_pre_increment() {
    // ++LocalN("key", obj)
    let source = r#"func Test() { var obj; ++LocalN("count", obj); }"#;
    let result = lc_script::Script::compile(source);
    if let Err(e) = &result {
        eprintln!("Error: line {}, col {}: {}", e.line(), e.column(), e.message());
    }
    assert!(result.is_ok());
}

#[test]
fn localn_two_args_post_decrement() {
    // LocalN("key", obj)--
    let source = r#"func Test() { var obj; LocalN("active", obj)--; }"#;
    let result = lc_script::Script::compile(source);
    if let Err(e) = &result {
        eprintln!("Error: line {}, col {}: {}", e.line(), e.column(), e.message());
    }
    assert!(result.is_ok());
}

#[test]
fn localn_one_arg_pre_increment() {
    // ++LocalN("key")
    let source = r#"func Test() { ++LocalN("counter"); }"#;
    let result = lc_script::Script::compile(source);
    if let Err(e) = &result {
        eprintln!("Error: line {}, col {}: {}", e.line(), e.column(), e.message());
    }
    assert!(result.is_ok());
}

#[test]
fn var_zero_args_pre_decrement() {
    // --Var()
    let source = r#"func Test() { --Var(); }"#;
    let result = lc_script::Script::compile(source);
    if let Err(e) = &result {
        eprintln!("Error: line {}, col {}: {}", e.line(), e.column(), e.message());
    }
    assert!(result.is_ok());
}

#[test]
fn var_zero_args_post_increment() {
    // Var()++
    let source = r#"func Test() { Var()++; }"#;
    let result = lc_script::Script::compile(source);
    if let Err(e) = &result {
        eprintln!("Error: line {}, col {}: {}", e.line(), e.column(), e.message());
    }
    assert!(result.is_ok());
}

#[test]
fn local_zero_args_pre_increment() {
    // ++Local()
    let source = r#"func Test() { ++Local(); }"#;
    let result = lc_script::Script::compile(source);
    if let Err(e) = &result {
        eprintln!("Error: line {}, col {}: {}", e.line(), e.column(), e.message());
    }
    assert!(result.is_ok());
}

#[test]
fn local_two_args_pre_increment() {
    // ++Local(0, obj)
    let source = r#"func Test() { var obj; ++Local(0, obj); }"#;
    let result = lc_script::Script::compile(source);
    if let Err(e) = &result {
        eprintln!("Error: line {}, col {}: {}", e.line(), e.column(), e.message());
    }
    assert!(result.is_ok());
}

#[test]
fn var_two_args_post_decrement() {
    // Var(0, obj)--
    let source = r#"func Test() { var obj; Var(0, obj)--; }"#;
    let result = lc_script::Script::compile(source);
    if let Err(e) = &result {
        eprintln!("Error: line {}, col {}: {}", e.line(), e.column(), e.message());
    }
    assert!(result.is_ok());
}

#[test]
fn warp_line_147_exact_pattern() {
    // Exact pattern from WARP line 147
    let source = r#"func Test() { var pTarget, iEffect, pObj; EffectVar(++EffectVar(0, pTarget, iEffect), pTarget, iEffect) = pObj; }"#;
    let result = lc_script::Script::compile(source);
    if let Err(e) = &result {
        eprintln!("Error: line {}, col {}: {}", e.line(), e.column(), e.message());
    }
    assert!(result.is_ok());
}

#[test]
fn skyrace_var_decrement_pattern() {
    // Pattern from Skyrace.c4s: --Var() in if condition
    let source = r#"func Test() { if (!--Var()) return("Done"); }"#;
    let result = lc_script::Script::compile(source);
    if let Err(e) = &result {
        eprintln!("Error: line {}, col {}: {}", e.line(), e.column(), e.message());
    }
    assert!(result.is_ok());
}

#[test]
fn nested_increment_in_condition() {
    // Pattern: if((--EffectVar(0, pTarget, iEffectNumber))<=0)
    let source = r#"func Test() { var pTarget, iEffectNumber; if((--EffectVar(0, pTarget, iEffectNumber))<=0) return(-1); }"#;
    let result = lc_script::Script::compile(source);
    if let Err(e) = &result {
        eprintln!("Error: line {}, col {}: {}", e.line(), e.column(), e.message());
    }
    assert!(result.is_ok());
}
