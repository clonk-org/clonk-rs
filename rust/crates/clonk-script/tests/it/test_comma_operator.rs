// C4Script has no generic comma operator. Commas remain delimiters, including
// the legacy pre-STRICT2 `return(first, unused...)` compatibility form.

use clonk_script::{Engine, Script, ScriptError, Value, ValueMap};
use std::sync::{
    atomic::{AtomicUsize, Ordering},
    Arc, Mutex,
};

fn assert_function_quarantined(source: &str, function: &str) {
    let script = Script::compile(source).expect("recoverable parse error keeps the script");
    assert!(
        !script.parse_diagnostics().is_empty(),
        "expected a parse diagnostic for {source:?}"
    );

    let mut engine = Engine::new();
    engine.add_script(script);
    let error = engine
        .call(function, &[])
        .expect_err("invalid comma expression must quarantine its function");
    assert!(error.to_string().contains("parse error"));
}

#[test]
fn legacy_adjacent_return_parentheses_returns_first_and_evaluates_the_rest() {
    let side_effects = Arc::new(AtomicUsize::new(0));
    let observed_side_effects = Arc::clone(&side_effects);
    let mut engine = Engine::new();
    engine.register_host_function("SideEffect", move |_| {
        observed_side_effects.fetch_add(1, Ordering::SeqCst);
        Ok(Value::Int(42))
    });
    engine
        .load_script("#strict\nfunc Probe() { return(0, SideEffect()); }")
        .expect("legacy adjacent return syntax compiles");

    assert_eq!(
        engine.call("Probe", &[]).expect("Probe executes"),
        Value::Nil,
        "pre-#strict-2 return(first, unused...) returns its first value"
    );
    assert_eq!(
        side_effects.load(Ordering::SeqCst),
        1,
        "legacy unused return parameters still execute for side effects"
    );
}

#[test]
fn legacy_spaced_return_parentheses_returns_first_and_evaluates_the_rest() {
    let evaluation_order = Arc::new(Mutex::new(Vec::new()));
    let observed_order = Arc::clone(&evaluation_order);
    let mut engine = Engine::new();
    engine.register_host_function("Mark", move |args| {
        let Some(Value::Int(marker)) = args.first() else {
            panic!("Mark requires one integer")
        };
        observed_order.lock().expect("order lock").push(*marker);
        Ok(Value::Int(*marker))
    });
    engine
        .load_script(
            "#strict\n\
             func One() { return (Mark(1), Mark(2)); }\n\
             func Zero() { return (0, Mark(7)); }",
        )
        .expect("legacy spaced return syntax compiles");

    assert_eq!(
        engine.call("One", &[]).expect("One executes"),
        Value::Int(1),
        "tokenizer whitespace must not disable the legacy return-parameter path"
    );
    assert_eq!(
        *evaluation_order.lock().expect("order lock"),
        vec![1, 2],
        "both operands execute from left to right"
    );

    evaluation_order.lock().expect("order lock").clear();
    assert_eq!(
        engine.call("Zero", &[]).expect("Zero executes"),
        Value::Nil,
        "the first falsy value remains the return value"
    );
    assert_eq!(
        *evaluation_order.lock().expect("order lock"),
        vec![7],
        "the unused return parameter still executes exactly once"
    );
}

#[test]
fn comma_nested_inside_a_call_does_not_trigger_legacy_return_parameters() {
    let mut engine = Engine::new();
    engine.register_host_function("Second", |args| {
        Ok(args.get(1).cloned().unwrap_or(Value::Nil))
    });
    engine
        .load_script("#strict\nfunc Probe() { return (Second(1, 2)); }")
        .expect("nested call comma compiles");

    assert_eq!(
        engine.call("Probe", &[]).expect("Probe executes"),
        Value::Int(2),
        "only commas directly inside the return parentheses are legacy parameters"
    );
}

#[test]
fn strict2_does_not_enter_the_legacy_multi_parameter_return_path() {
    assert_function_quarantined(
        "#strict 2\nfunc Probe() { return(0, 42); }",
        "Probe",
    );
}

#[test]
fn mgsm_line_24_pattern() {
    // Exact pattern from MGSM line 24
    let source = r#"func Test() { if (!SetAction("Wait")) return (0, RemoveObject()); }"#;
    let result = clonk_script::Script::compile(source);
    if let Err(e) = &result {
        eprintln!(
            "Error: line {}, col {}: {}",
            e.line(),
            e.column(),
            e.message()
        );
    }
    assert!(result.is_ok());
}

#[test]
fn nonstrict_spaced_return_parentheses_return_the_first_value() {
    let mut engine = Engine::new();
    engine
        .load_script("func Test() { return (1, 2); }")
        .expect("nonstrict legacy return syntax loads");

    assert_eq!(
        engine.call("Test", &[]).expect("Test executes"),
        Value::Int(1),
        "nonstrict spaced return parameters keep the first value"
    );
}

#[test]
fn comma_with_three_expressions() {
    // return (expr1, expr2, expr3)
    let source = r#"func Test() { return (1, 2, 3); }"#;
    let result = clonk_script::Script::compile(source);
    if let Err(e) = &result {
        eprintln!(
            "Error: line {}, col {}: {}",
            e.line(),
            e.column(),
            e.message()
        );
    }
    assert!(result.is_ok());
}

#[test]
fn comma_with_function_calls() {
    // return (1, Message(...), Sound(...))
    let source = r#"func Test() { return (1, Message("test"), Sound("Click")); }"#;
    let result = clonk_script::Script::compile(source);
    if let Err(e) = &result {
        eprintln!(
            "Error: line {}, col {}: {}",
            e.line(),
            e.column(),
            e.message()
        );
    }
    assert!(result.is_ok());
}

#[test]
fn comma_with_assignment() {
    // return (1, var = 0)
    let source = r#"func Test() { var x; return (1, x = 42); }"#;
    let result = clonk_script::Script::compile(source);
    if let Err(e) = &result {
        eprintln!(
            "Error: line {}, col {}: {}",
            e.line(),
            e.column(),
            e.message()
        );
    }
    assert!(result.is_ok());
}

#[test]
fn generic_comma_expressions_in_assignments_are_rejected() {
    assert_function_quarantined("func Test() { var x; x = (1, 2); }", "Test");
    assert_function_quarantined("func Test() { var x = (0, 42); }", "Test");
}

#[test]
fn comma_in_if_condition() {
    assert_function_quarantined(
        "func Test() { var x; if ((x = 5, x > 0)) return 1; }",
        "Test",
    );
}

#[test]
fn comma_in_while_condition() {
    assert_function_quarantined(
        "func Test() { var x; while ((x = x + 1, x < 10)) {} }",
        "Test",
    );
}

#[test]
fn legacy_if_while_parameter_lists_keep_first_and_evaluate_surplus() {
    for directive in ["", "#strict\n"] {
        let evaluation_order = Arc::new(Mutex::new(Vec::new()));
        let observed_order = Arc::clone(&evaluation_order);
        let mut engine = Engine::new();
        engine.register_host_function("Mark", move |args| {
            let [Value::Int(marker), value] = args else {
                panic!("Mark requires an integer marker and a value")
            };
            observed_order.lock().expect("order lock").push(*marker);
            Ok(value.clone())
        });

        let script = Script::compile(&format!(
            r#"{directive}
            func EmptyIf() {{ if() return 90; return 11; }}
            func EmptyWhile() {{ while() return 90; return 12; }}
            func SurplusIf() {{
                if(Mark(1, 1), Mark(2, 1), Mark(3, 1)) return 13;
                return 90;
            }}
            func FalseFirst() {{
                if(Mark(4, 1 - 1), Mark(5, 1)) return 90;
                return 14;
            }}
            func MissingFirst() {{
                if(, Mark(6, 1)) return 90;
                return 15;
            }}
            func TrailingSlot() {{
                if(Mark(7, 1),) return 16;
                return 90;
            }}
            func LateReferenceTrue() {{
                var value = 1 - 1;
                if(value, value = 1) return 17;
                return 90;
            }}
            func LateReferenceFalse() {{
                var value = 1;
                if(value, value = 1 - 1) return 90;
                return 18;
            }}
            func SurplusWhile() {{
                var i = 1 - 1;
                while(Mark(10 + i, 2 - i), Mark(20 + i, 1), Mark(30 + i, 1))
                    i += 1;
                return i;
            }}
            func Forward(first, ...) {{ if(...) return 19; return 0; }}
            "#
        ))
        .expect("legacy condition parameter lists compile");

        let warning_messages = script
            .parse_diagnostics()
            .iter()
            .map(|diagnostic| diagnostic.message())
            .collect::<Vec<_>>();
        assert_eq!(
            warning_messages
                .iter()
                .filter(|message| { **message == "if: passing 2 parameters, but only 1 are used" })
                .count(),
            5
        );
        assert_eq!(
            warning_messages
                .iter()
                .filter(|message| { **message == "if: passing 3 parameters, but only 1 are used" })
                .count(),
            1
        );
        assert_eq!(
            warning_messages
                .iter()
                .filter(|message| {
                    **message == "while: passing 3 parameters, but only 1 are used"
                })
                .count(),
            1
        );
        assert_eq!(warning_messages.len(), 7);

        engine.add_script(script);
        for (function, expected) in [
            ("EmptyIf", Value::Int(11)),
            ("EmptyWhile", Value::Int(12)),
            ("SurplusIf", Value::Int(13)),
            ("FalseFirst", Value::Int(14)),
            ("MissingFirst", Value::Int(15)),
            ("TrailingSlot", Value::Int(16)),
            ("LateReferenceTrue", Value::Int(17)),
            ("LateReferenceFalse", Value::Int(18)),
            ("SurplusWhile", Value::Int(2)),
        ] {
            assert_eq!(
                engine.call(function, &[]).expect("condition executes"),
                expected
            );
        }
        assert_eq!(
            engine
                .call("Forward", &[Value::Nil, Value::Int(1)])
                .expect("ellipsis forwards the first unnamed parameter"),
            Value::Int(19)
        );
        assert_eq!(
            engine
                .call("Forward", &[Value::Nil])
                .expect("missing unnamed parameter is nil"),
            Value::Nil
        );
        assert_eq!(
            *evaluation_order.lock().expect("order lock"),
            vec![1, 2, 3, 4, 5, 6, 7, 10, 20, 30, 11, 21, 31, 12, 22, 32]
        );
    }

    for level in [2, 3] {
        let mut exact_engine = Engine::new();
        exact_engine
            .load_script(&format!(
                "#strict {level}\n\
                 func Exact() {{ var i = 0; if (1) i = 1; while (i < 2) i += 1; return i; }}"
            ))
            .expect("modern exact-one conditions compile");
        assert_eq!(
            exact_engine
                .call("Exact", &[])
                .expect("exact conditions run"),
            Value::Int(2)
        );

        for body in [
            "if() return 1;",
            "if(1, 2) return 1;",
            "if(1,) return 1;",
            "while() return 1;",
            "while(1, 2) return 1;",
            "while(1,) return 1;",
        ] {
            assert_function_quarantined(
                &format!("#strict {level}\nfunc Invalid() {{ {body} }}"),
                "Invalid",
            );
        }
    }
}

#[test]
fn legacy_condition_path_references_pin_cpp_container_elements() {
    let script = Script::compile(
        r#"
        #strict
        func SameArraySlot() {
            var values = [0];
            if(values[0], values[0] = 1) return 20;
            return 90;
        }
        func PinnedArrayRoot() {
            var values = [1];
            if(values[0], values = [0]) return 21;
            return 90;
        }
        func PinnedArrayAncestor() {
            var values = [[1]];
            if(values[0][0], values[0] = [0]) return 22;
            return 90;
        }
        func SamePropertySlot(values) {
            if(values["item"], values["item"] = 1) return 23;
            return 90;
        }
        func PinnedPropertyRoot(values, replacement) {
            if(values["item"], values = replacement) return 24;
            return 90;
        }
        func PinnedPropertyAncestor(values, replacement) {
            if(values["nested"]["item"], values["nested"] = replacement) return 25;
            return 90;
        }
        func Rebind(&values) { values = [1]; return 0; }
        func ReentrantRhs() {
            var values = [1];
            if(values[0], values[0] = Rebind(values)) return 90;
            return 26;
        }
        func RebindAncestor(&values) { values[0] = [1]; return 0; }
        func ReentrantAncestorRhs() {
            var values = [[1]];
            if(values[0][0], values[0][0] = RebindAncestor(values)) return 90;
            return 27;
        }
        func SingleReentrantRhs() {
            var values = [1];
            if(values[0] = Rebind(values)) return 90;
            return 27;
        }
        func ReentrantCompound() {
            var values = [1];
            if(1, values[0] += Rebind(values)) {}
            return 27;
        }
        func DiscardedArrayReferenceGrows() {
            var values = [];
            if(1, values[2]) {}
            return values;
        }
        func MissingMapSlot(values) {
            if(1, values["missing"]) {}
            return values;
        }
        func DiscardedReferenceDetaches() {
            var values = [1], alias = values;
            if(1, values[0]) {}
            return values == alias;
        }
        func CopyAfterPin() {
            var values = [1], alias;
            if(values[0], alias = values) {}
            return values == alias;
        }
        func DiscardedRefLivesThroughList() {
            var values = [1], alias;
            if(1, values[0], alias = values) {}
            return values == alias;
        }
        func RootSelfCopyReplacesPinnedContainer() {
            var values = [1];
            if(values[0], values = values, values[0] = 0) return 90;
            return 28;
        }
        func PrefixRefs() {
            var up = -1, down = 1;
            if(++up, up = 1) {
                if(--down, down = 1) return 29;
            }
            return 90;
        }
        func Identity(value) { return value; }
        func NestedValueDoesNotGrow() {
            var values = [], alias = values;
            if(1, Identity(values[2])) {}
            return [values, values == alias];
        }
        func NestedBinaryDoesNotGrow() {
            var values = [];
            if(1, values[2] + 0) {}
            return values;
        }
        func ReadPinnedValue(&slot) { return slot[2]; }
        func NestedPinnedValueDoesNotGrow() {
            var values = [[]];
            if(1, ReadPinnedValue(values[0])) {}
            return values[0];
        }
        func SetZero(&value) { value = 0; return 1; }
        func ReferenceArgumentsStayLive() {
            var prefix = 0, assignment = 0;
            if(1, SetZero(++prefix), SetZero(assignment = 2)) {}
            return [prefix, assignment];
        }
        func InvalidDiscardedIndex() {
            var value = 1;
            if(1, value[0]) {}
            return 90;
        }
        func CopyWithPinned(&slot, &root) {
            var alias;
            if(1, alias = root) {}
            return root == alias;
        }
        func ExistingPinnedRefCopy() {
            var values = [1];
            return CopyWithPinned(values[0], values);
        }
        func RebindRootForIndex(&values) { values = [[9]]; return 0; }
        func ResolvedContainerIndexedAgain() {
            var values = [[2]];
            if(1, values[0][RebindRootForIndex(values)] = 3) {}
            return 31;
        }
        func AcceptCollapsedReference(&slot, marker) { return 32; }
        func CollapsedReferenceArgument() {
            var values = [[2]];
            if(1, AcceptCollapsedReference(values[0][RebindRootForIndex(values)], MarkCollapsed())) {}
            return 33;
        }
        "#,
    )
    .expect("legacy container-reference conditions compile");

    assert_eq!(
        script
            .parse_diagnostics()
            .iter()
            .filter(|diagnostic| {
                diagnostic.message() == "if: passing 2 parameters, but only 1 are used"
            })
            .count(),
        22
    );
    assert_eq!(
        script
            .parse_diagnostics()
            .iter()
            .filter(|diagnostic| {
                diagnostic.message() == "if: passing 3 parameters, but only 1 are used"
            })
            .count(),
        3
    );
    assert_eq!(script.parse_diagnostics().len(), 25);

    let collapsed_mark_count = Arc::new(AtomicUsize::new(0));
    let observed_collapsed_mark_count = Arc::clone(&collapsed_mark_count);
    let mut engine = Engine::new();
    engine.register_host_function("MarkCollapsed", move |args| {
        assert!(args.is_empty());
        observed_collapsed_mark_count.fetch_add(1, Ordering::SeqCst);
        Ok(Value::Nil)
    });
    engine.add_script(script);
    for (function, expected) in [
        ("SameArraySlot", Value::Int(20)),
        ("PinnedArrayRoot", Value::Int(21)),
        ("PinnedArrayAncestor", Value::Int(22)),
        (
            "DiscardedArrayReferenceGrows",
            Value::Array(vec![Value::Nil, Value::Nil, Value::Nil]),
        ),
        ("DiscardedReferenceDetaches", Value::Bool(false)),
        ("CopyAfterPin", Value::Bool(false)),
        ("DiscardedRefLivesThroughList", Value::Bool(false)),
        ("RootSelfCopyReplacesPinnedContainer", Value::Int(90)),
        ("PrefixRefs", Value::Int(29)),
        (
            "NestedValueDoesNotGrow",
            Value::Array(vec![Value::Array(Vec::new()), Value::Bool(true)]),
        ),
        ("NestedBinaryDoesNotGrow", Value::Array(Vec::new())),
        ("NestedPinnedValueDoesNotGrow", Value::Array(Vec::new())),
        (
            "ReferenceArgumentsStayLive",
            Value::Array(vec![Value::Nil, Value::Nil]),
        ),
        ("ExistingPinnedRefCopy", Value::Bool(false)),
    ] {
        assert_eq!(
            engine.call(function, &[]).expect("condition executes"),
            expected,
            "{function} must retain the C++ element-reference behavior"
        );
    }

    let item_map =
        |value| Value::Proplist(ValueMap::from([("item".to_string(), Value::Int(value))]));
    assert_eq!(
        engine
            .call("SamePropertySlot", &[item_map(0)])
            .expect("same map slot remains referenced"),
        Value::Int(23)
    );
    assert_eq!(
        engine
            .call("PinnedPropertyRoot", &[item_map(1), item_map(0)])
            .expect("map root replacement resolves the old element"),
        Value::Int(24)
    );
    let nested = Value::Proplist(ValueMap::from([("nested".to_string(), item_map(1))]));
    assert_eq!(
        engine
            .call("PinnedPropertyAncestor", &[nested, item_map(0)])
            .expect("map ancestor replacement resolves the old element"),
        Value::Int(25)
    );
    assert_eq!(
        engine
            .call("MissingMapSlot", &[Value::Proplist(ValueMap::new())])
            .expect("a reference read inserts the missing map slot"),
        Value::Proplist(ValueMap::from([("missing".to_string(), Value::Nil)]))
    );
    for function in [
        "ReentrantRhs",
        "ReentrantAncestorRhs",
        "SingleReentrantRhs",
        "ResolvedContainerIndexedAgain",
    ] {
        let error = engine
            .call(function, &[])
            .expect_err("container destruction resolves the assignment target to a value");
        let ScriptError::Runtime(error) = error else {
            panic!("expected runtime error for {function}, got {error}");
        };
        assert_eq!(
            error.message(),
            "operator \"=\" left side: got \"int\", but expected \"&\"!",
            "{function}"
        );
    }
    let error = engine
        .call("ReentrantCompound", &[])
        .expect_err("container destruction invalidates a compound target");
    let ScriptError::Runtime(error) = error else {
        panic!("expected runtime error, got {error}");
    };
    assert_eq!(
        error.message(),
        "operator \"+=\" left side: got \"int\", but expected \"int&\"!"
    );
    let error = engine
        .call("InvalidDiscardedIndex", &[])
        .expect_err("AB_ARRAYA_R validates a discarded reference immediately");
    let ScriptError::Runtime(error) = error else {
        panic!("expected runtime error, got {error}");
    };
    assert_eq!(
        error.message(),
        "indexed access: can't access int by index!"
    );
    let error = engine
        .call("CollapsedReferenceArgument", &[])
        .expect_err("a resolved value cannot satisfy a reference parameter");
    let ScriptError::Runtime(error) = error else {
        panic!("expected runtime error, got {error}");
    };
    assert_eq!(
        error.message(),
        "call to \"AcceptCollapsedReference\" parameter 1: got \"int\", but expected \"&\"!"
    );
    assert_eq!(collapsed_mark_count.load(Ordering::SeqCst), 1);
}

#[test]
fn legacy_condition_effectvar_host_paths_pin_cpp_container_elements() {
    let slot = Arc::new(Mutex::new(Value::Nil));
    let host_slot = Arc::clone(&slot);
    let mut engine = Engine::new();
    engine.register_host_function_with_arity("EffectVar", 3, move |args| {
        let mut slot = host_slot.lock().expect("EffectVar slot lock");
        match args {
            [_, _, _] => Ok(slot.clone()),
            [_, _, _, replacement] => {
                *slot = replacement.clone();
                Ok(replacement.clone())
            }
            _ => panic!("invalid EffectVar frame: {args:?}"),
        }
    });
    let script = Script::compile(
        r#"
        #strict
        func DiscardedEffectVarReferenceGrows() {
            EffectVar(0, 0, 1) = [];
            if(1, EffectVar(0, 0, 1)[2]) {}
            return EffectVar(0, 0, 1);
        }
        func PinnedEffectVarRoot() {
            EffectVar(0, 0, 1) = [1];
            if(EffectVar(0, 0, 1)[0], EffectVar(0, 0, 1) = [0]) return 34;
            return 90;
        }
        func PinnedEffectVarAncestor() {
            EffectVar(0, 0, 1) = [[1]];
            if(EffectVar(0, 0, 1)[0][0], EffectVar(0, 0, 1)[0] = [0]) return 35;
            return 90;
        }
        func SameEffectVarSlotStaysLive() {
            EffectVar(0, 0, 1) = [0];
            if(EffectVar(0, 0, 1)[0], EffectVar(0, 0, 1)[0] = 1) return 36;
            return 90;
        }
        func ConvertedEffectVarAddressMatches() {
            EffectVar(1, 0, 1) = [0];
            if(EffectVar(1 == 1, 0, 1)[0], EffectVar(1, 0, 1)[0] = 1) return 37;
            return 90;
        }
        func ReplaceEffectVarRoot() {
            EffectVar(0, 0, 1) = [9];
            return 0;
        }
        func EffectVarValueAccessStaysLive() {
            EffectVar(0, 0, 1) = [1];
            return EffectVar(0, 0, 1)[ReplaceEffectVarRoot()];
        }
        func InvalidDiscardedEffectVarIndex() {
            EffectVar(0, 0, 1) = 1;
            if(1, EffectVar(0, 0, 1)[0]) {}
            return 90;
        }
        "#,
    )
    .expect("EffectVar path conditions compile");
    assert_eq!(
        script
            .parse_diagnostics()
            .iter()
            .filter(|diagnostic| {
                diagnostic.message() == "if: passing 2 parameters, but only 1 are used"
            })
            .count(),
        6
    );
    assert_eq!(script.parse_diagnostics().len(), 6);
    engine.add_script(script);

    assert_eq!(
        engine
            .call("DiscardedEffectVarReferenceGrows", &[])
            .expect("a discarded host-backed reference grows its array"),
        Value::Array(vec![Value::Nil, Value::Nil, Value::Nil])
    );
    assert_eq!(
        engine
            .call("PinnedEffectVarRoot", &[])
            .expect("host root replacement resolves the old element"),
        Value::Int(34)
    );
    assert_eq!(
        engine
            .call("PinnedEffectVarAncestor", &[])
            .expect("host ancestor replacement resolves the old element"),
        Value::Int(35)
    );
    assert_eq!(
        engine
            .call("SameEffectVarSlotStaysLive", &[])
            .expect("same host slot replacement keeps the reference live"),
        Value::Int(36)
    );
    assert_eq!(
        engine
            .call("ConvertedEffectVarAddressMatches", &[])
            .expect("converted host arguments identify the same EffectVar slot"),
        Value::Int(37)
    );
    assert_eq!(
        engine
            .call("EffectVarValueAccessStaysLive", &[])
            .expect("SetNoRef does not rewrite a reference-returning call"),
        Value::Int(9)
    );
    let error = engine
        .call("InvalidDiscardedEffectVarIndex", &[])
        .expect_err("a discarded host-backed reference validates eagerly");
    let ScriptError::Runtime(error) = error else {
        panic!("expected runtime error, got {error}");
    };
    assert_eq!(
        error.message(),
        "indexed access: can't access int by index!"
    );
}

#[test]
fn nested_comma_expressions() {
    // The outer comma is the legacy return delimiter; the nested comma is
    // still an invalid generic expression.
    assert_function_quarantined("func Test() { return (1, (2, 3)); }", "Test");
}

#[test]
fn lock_pattern() {
    // Pattern from Lock.c4d scripts
    let source = r#"func Test() { return (1, Message("test"), Sound("Error")); }"#;
    let result = clonk_script::Script::compile(source);
    if let Err(e) = &result {
        eprintln!(
            "Error: line {}, col {}: {}",
            e.line(),
            e.column(),
            e.message()
        );
    }
    assert!(result.is_ok());
}

#[test]
fn kingdoms_pattern() {
    // Pattern from Kingdoms scripts
    let source = r#"func Test() { var clonk; if (!clonk) return (0, RemoveObject()); }"#;
    let result = clonk_script::Script::compile(source);
    if let Err(e) = &result {
        eprintln!(
            "Error: line {}, col {}: {}",
            e.line(),
            e.column(),
            e.message()
        );
    }
    assert!(result.is_ok());
}

#[test]
fn comma_in_var_decl_without_parens_is_rejected_like_cpp() {
    // C4Script has NO comma operator (it is absent from C4ScriptOpMap,
    // src/C4AulParse.cpp:423). Inside a `var` declaration the comma is a
    // *declarator separator* (`var a = 1, b = 2;` declares two variables), so
    // C++ `Parse_Var` (src/C4AulParse.cpp:3252) parses the initializer with
    // `Parse_Expression()` — which stops at the comma — and then expects another
    // variable NAME. `var x = 1, 2;` therefore fails in C++ ("variable name"
    // expected, finding the int `2`). The Rust port must reject it identically.
    let rejected = clonk_script::Script::compile(r#"func Test() { var x = 1, 2; }"#)
        .expect("the invalid function body is quarantined instead of aborting the script");
    assert!(
        !rejected.parse_diagnostics().is_empty(),
        "unparenthesized comma in a var declaration must produce a parse diagnostic"
    );
    let mut engine = Engine::new();
    engine.add_script(rejected);
    let error = engine
        .call("Test", &[])
        .expect_err("calling the quarantined function must surface its parse error");
    assert!(error.to_string().contains("parse error"));

    // The standard multi-declarator form must keep compiling: here the comma is
    // a declarator separator (`var a = 1` then `b = 2`), which C++ Parse_Var
    // supports directly.
    assert!(
        clonk_script::Script::compile(r#"func Test() { var a = 1, b = 2; return a + b; }"#).is_ok(),
        "standard multi-declarator var should compile (comma is a declarator separator)"
    );
}
