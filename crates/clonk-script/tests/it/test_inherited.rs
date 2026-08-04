//! In strict scripts, `inherited(...)` calls the function this one overloaded
//! (Fn->OwnerOverloaded, C4AulParse.cpp:2775-2798); `_inherited` is the safe
//! spelling that yields nil instead of an error when no parent exists
//! (C4AUL_SafeInherited, C4AulParse.cpp:55-56). Overloads arise when a later
//! script redefines a name (C4AulScriptEngine link order) or when an
//! #include'd parent defines the same function.

use clonk_script::{Engine, Script, Value};

fn compile_clean(source: &str) -> Script {
    let script = Script::compile(source).expect("script compiles");
    assert!(
        script.parse_diagnostics().is_empty(),
        "unexpected parse diagnostics: {:?}",
        script.parse_diagnostics()
    );
    script
}

#[test]
fn nonstrict_inherited_is_rejected() {
    for spelling in ["inherited", "_inherited"] {
        let source = format!(
            "func Broken() {{ return {spelling}(); }}\n\
             func Healthy() {{ return 7; }}"
        );
        let script = Script::compile(&source).expect("body error is recovered");
        assert_eq!(script.parse_diagnostics().len(), 1, "source: {source}");
        assert_eq!(
            script.parse_diagnostics()[0].message(),
            "inherited disabled; use #strict syntax!",
            "source: {source}"
        );

        let mut engine = Engine::new();
        engine.add_script(script);
        assert_eq!(
            engine
                .call("Healthy", &[])
                .expect("recovery keeps sibling functions"),
            Value::Int(7)
        );
        let error = engine
            .call("Broken", &[])
            .expect_err("the rejected function retains a parse-error sentinel");
        assert!(
            error
                .to_string()
                .contains("inherited disabled; use #strict syntax!"),
            "{error}"
        );
    }
}

#[test]
fn safe_inherited_without_parent_is_nil() {
    let source = r#"
        #strict
        global func Construction() { return _inherited(); }
    "#;
    let mut engine = Engine::new();
    engine.add_script(compile_clean(source));
    assert_eq!(
        engine.call("Construction", &[]).expect("call succeeds"),
        Value::Nil
    );
}

#[test]
fn safe_inherited_without_parent_evaluates_discarded_arguments() {
    let source = r#"
        #strict
        local calls;
        func SideEffect() { calls++; return 99; }
        func Construction() { return [_inherited(SideEffect()), calls]; }
    "#;
    let mut engine = Engine::new();
    engine.add_script(compile_clean(source));
    assert_eq!(
        engine.call("Construction", &[]).expect("call succeeds"),
        Value::Array(vec![Value::Nil, Value::Int(1)])
    );
}

#[test]
fn plain_inherited_without_parent_errors() {
    let source = r#"
        #strict
        global func Construction() { return inherited(); }
    "#;
    let mut engine = Engine::new();
    engine.add_script(compile_clean(source));
    assert!(engine.call("Construction", &[]).is_err());
}

#[test]
fn later_script_overloads_earlier_and_reaches_it_via_inherited() {
    let mut engine = Engine::new();
    engine.add_script(compile_clean("global func F() { return 1; }"));
    engine.add_script(compile_clean(
        "#strict\nglobal func F() { return inherited() + 10; }",
    ));
    assert_eq!(
        engine.call("F", &[]).expect("call succeeds"),
        Value::Int(11)
    );
}

#[test]
fn inherited_forwards_arguments() {
    let mut engine = Engine::new();
    engine.add_script(compile_clean("global func F(a, b) { return a + b; }"));
    engine.add_script(compile_clean(
        "#strict\nglobal func F(a, b) { return inherited(a, b) * 2; }",
    ));
    assert_eq!(
        engine
            .call("F", &[Value::Int(2), Value::Int(3)])
            .expect("call succeeds"),
        Value::Int(10)
    );
}

#[test]
fn include_parent_function_is_reachable_via_inherited() {
    // The #include seam: the child keeps its own function and the parent's
    // becomes its overload target (C4AulLink include handling). GLOBAL
    // functions are never copied by includes (C4AulLink.cpp:127 — they
    // live at the engine, where install chaining forms their overloads),
    // so the seam is pinned with public functions.
    let mut parent = Engine::new();
    parent.add_script(compile_clean("public func F() { return 5; }"));
    let mut child = Engine::new();
    child.add_script(compile_clean(
        "#strict\npublic func F() { return _inherited() + 1; }",
    ));
    child.merge_from(&parent);
    assert_eq!(child.call("F", &[]).expect("call succeeds"), Value::Int(6));

    // A global func in the parent is NOT copied into the child.
    let mut global_parent = Engine::new();
    global_parent.add_script(compile_clean("global func G() { return 5; }"));
    let mut plain_child = Engine::new();
    plain_child.add_script(compile_clean("// empty\n"));
    plain_child.merge_from(&global_parent);
    assert!(
        !plain_child.has_function("G"),
        "includes never copy global funcs (C4AulLink.cpp:127)"
    );
}

#[test]
fn same_script_redefinition_reaches_the_earlier_definition_via_inherited() {
    // C4AulScript::ParseFn links a redefinition in the SAME script to the
    // earlier definition (`Fn->OwnerOverloaded = Fn->Owner->
    // GetOverloadedFunc(Fn)`, C4AulParse.cpp:1404-1406). The Coach.c4d menu
    // idiom relies on it: the implementation is followed by
    // `public func ControlDownDouble(pByObject) { [$TxtGetoff$]
    // return(inherited(pByObject)); }` (Coach.c4d/Script.c) — the wrapper
    // adds the menu description and forwards to the real body.
    let source = r#"
        #strict
        public func F(a) { return a + 1; }
        public func F(a) { return inherited(a) * 10; }
    "#;
    let mut engine = Engine::new();
    engine.add_script(compile_clean(source));
    assert_eq!(
        engine.call("F", &[Value::Int(2)]).expect("call succeeds"),
        Value::Int(30),
        "the later definition wins and inherited() reaches the earlier one"
    );
}

#[test]
fn global_func_inherited_skips_the_same_hosts_definition_scope_func() {
    // A `global func` belongs to the ENGINE script: C4AulParse.cpp:1610-1614
    // builds it as `new C4AulScriptFunc(a->Engine, Idtf)` and leaves only an
    // UNNAMED `C4AulFunc(a, nullptr)` link in the declaring script. Its
    // overload target is then searched in that owner's list
    // (C4AulParse.cpp:1406-1408, "*MUST* check Fn->Owner-list, because it may
    // be the engine (due to linked globals)"), and C4Aul.cpp:266-277 walks the
    // list backwards comparing `SEqual(ByFunc->Name, f->Name)` — the unnamed
    // link never matches, so the declaring host's own definition-scope
    // function is unreachable from the global. MetalMagic's
    // MagixRoom.c4d/Script.c declares exactly this pair for `GetDir`/`SetDir`.
    let source = r#"
        #strict
        public func F() { return 99; }
        global func F() { return _inherited(); }
    "#;
    let mut engine = Engine::new();
    engine.add_script(compile_clean(source));
    assert_eq!(
        engine.call("F", &[]).expect("call succeeds"),
        Value::Nil,
        "the global func has no engine-side overload, so _inherited is nil"
    );
}

#[test]
fn global_func_inherited_reaches_the_engine_native_it_overloads() {
    // MetalMagic's MagixRoom.c4d/Script.c:271-282 is this exact shape: the
    // global `SetDir(dir, obj)` handles its own definition and forwards
    // everything else to the engine's `AddFunc(pEngine, "SetDir", FnSetDir)`
    // (C4Script.cpp:6934) through `_inherited`. The engine native is what the
    // engine-owned overload search terminates on (C4Aul.cpp:281-288 finds no
    // owner above the engine, leaving `OwnerOverloaded` null and the call to
    // the same-name native).
    let mut engine = Engine::new();
    engine.register_host_function_with_arity("SetDir", 2, |_| Ok(Value::Int(42)));
    engine.add_script(compile_clean(
        r#"
        #strict
        local direction;
        public func SetDir(dir) { direction = dir; return direction; }
        global func SetDir(dir, obj) { return _inherited(dir, obj); }
        "#,
    ));
    assert_eq!(
        engine
            .call("SetDir", &[Value::Int(7), Value::Nil])
            .expect("call succeeds"),
        Value::Int(42),
        "the global forwards to the native, not to the definition-scope func"
    );
}

#[test]
fn global_func_inherited_reaches_the_previous_global_not_a_foreign_local() {
    // Two hosts declaring the same global is the shipped MetalMagic /
    // MetalMagicExtra pairing (both MagixRoom.c4d/Script.c files declare
    // `global func GetDir`). The later global's owner list is the engine's,
    // so it reaches the earlier GLOBAL — never the definition-scope function
    // sitting beside it in its own host (C4AulParse.cpp:1608-1615).
    let mut engine = Engine::new();
    engine.add_script(compile_clean("global func F() { return 1; }"));
    engine.add_script(compile_clean(
        "#strict\n\
         public func F() { return 99; }\n\
         global func F() { return _inherited() + 5; }",
    ));
    assert_eq!(
        engine.call("F", &[]).expect("call succeeds"),
        Value::Int(6),
        "the engine-owned chain skips the later host's definition-scope func"
    );
}

#[test]
fn definition_scope_inherited_prefers_an_older_own_func_over_a_global() {
    // The other half of the same C4Aul rule. `GetOverloadedFunc` walks the
    // DECLARING host's list backwards first (C4Aul.cpp:269-276), and a
    // `global func`'s presence there is the UNNAMED `C4AulFunc(a, nullptr)`
    // link (C4AulParse.cpp:1613) — `SEqual(ByFunc->Name, f->Name)` never
    // matches it, so the walk steps over the global and stops at the earlier
    // definition-scope declaration.
    let source = r#"
        #strict
        public func F() { return 1; }
        global func F() { return 2; }
        public func F() { return inherited(); }
    "#;
    let mut engine = Engine::new();
    engine.add_script(compile_clean(source));
    assert_eq!(
        engine.call("F", &[]).expect("call succeeds"),
        Value::Int(1),
        "the interposed global is skipped, not taken as the overload target"
    );
}

#[test]
fn definition_scope_inherited_hops_to_the_engine_when_its_own_host_has_none() {
    // Only when the host's own backward walk finds nothing does
    // `GetOverloadedFunc` hop to the owner — `if (!f && Owner) { if ((f =
    // Owner->GetFuncRecursive(ByFunc->Name))) ... }` (C4Aul.cpp:281-288). A
    // definition script's owner IS the script engine (C4Def.cpp:649
    // `Script.Reg2List(&Game.ScriptEngine, &Game.ScriptEngine)`), so a
    // definition-scope function can legitimately overload a global. The
    // reverse direction is structurally impossible, which is why this rule is
    // asymmetric.
    let source = r#"
        #strict
        global func F() { return 1; }
        public func F() { return inherited() + 10; }
    "#;
    let mut engine = Engine::new();
    engine.add_script(compile_clean(source));
    assert_eq!(
        engine.call("F", &[]).expect("call succeeds"),
        Value::Int(11),
        "with no earlier own declaration the walk hops to the engine's global"
    );
}
