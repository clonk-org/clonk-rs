// Parity: `==` / `!=` semantics depend on the script's #strict level.
//
// C++ oracle: C4Value::Equals (C4Value.cpp:823):
//   NONSTRICT / STRICT1 -> _getRaw() == other._getRaw()  (raw Data compare)
//   STRICT2             -> operator==                     (cross-type numeric)
//   STRICT3             -> type-checked: different types are never equal
// The raw / cross-type levels make the numeric coincidences equal (0 == nil,
// 1 == true, 0 == false), because Int/Bool/nil share the integer Data slot.
// Pointer-backed strings/arrays/maps compare by raw identity below STRICT2,
// by content at STRICT2, and with matching outer types at STRICT3.

use crate::support::eval;
use clonk_script::{C4VType, Engine, RuntimeError, ScriptCallOutcome, Value, ValueMap};

#[derive(Debug)]
struct PauseRequest;

fn eval_with_format(source: &str) -> Value {
    let mut engine = Engine::new();
    engine.register_host_function("Format", |_| Ok(Value::String("a".into())));
    engine.load_script(source).expect("loads");
    engine.call("Test", &[]).expect("call")
}

eval_cases! {
    nonstrict_treats_zero_and_nil_as_equal:
        "func Test() { var empty; return (1 - 1) == empty; }" => Value::Bool(true);
}

eval_test! { nonstrict_treats_one_and_true_as_equal {
    "func Test() { return 1 == true; }" => Value::Bool(true);
    "func Test() { return 0 == false; }" => Value::Bool(true);
} }

#[test]
fn strict1_strings_compare_raw_identity() {
    assert_eq!(
        eval_with_format("#strict\nfunc Test() { return Format(\"%s\", \"a\") == \"a\"; }"),
        Value::Bool(false)
    );
    assert_eq!(
        eval("#strict\nfunc Test() { var s = \"a\"; return s == \"a\"; }"),
        Value::Bool(true)
    );
    assert_eq!(
        eval_with_format(
            "#strict\nfunc Test() { var s = Format(\"%s\", \"a\"); var t = s; return s == t; }"
        ),
        Value::Bool(true)
    );
    assert_eq!(
        eval("#strict\nfunc Same(string s) { return s == \"a\"; } func Test() { return Same(\"a\"); }"),
        Value::Bool(true)
    );
    assert_eq!(
        eval(
            "#strict\nfunc Literal() { return \"a\"; } func Test() { return Literal() == \"a\"; }"
        ),
        Value::Bool(true)
    );
    assert_eq!(
        eval_with_format("#strict\nfunc Identity(s) { return s; } func Test() { var s = Format(\"%s\", \"a\"); return Identity(s) == s; }"),
        Value::Bool(true)
    );
    assert_eq!(
        eval_with_format("#strict\nfunc Test() { var s = Format(\"%s\", \"a\"); var t = s ?? \"fallback\"; return t == s; }"),
        Value::Bool(true)
    );
    assert_eq!(
        eval_with_format("#strict\nfunc SetLiteral(&s) { s = \"a\"; } func Test() { var s = Format(\"%s\", \"a\"); SetLiteral(s); return s == \"a\"; }"),
        Value::Bool(true)
    );
    assert_eq!(
        eval("#strict\nfunc Test() { var a = [\"a\"]; return a[0] == \"a\"; }"),
        Value::Bool(true)
    );
    assert_eq!(
        eval_with_format("#strict\nfunc Test() { var a = [Format(\"%s\", \"a\")]; return a[0] == a[0] && a[0] != \"a\"; }"),
        Value::Bool(true)
    );
    assert_eq!(
        eval("#strict\nfunc Test() { var empty; var a = [empty]; a[0] = \"a\"; return a[0] == \"a\"; }"),
        Value::Bool(true)
    );
    assert_eq!(
        eval("#strict\nfunc Test() { var a = [\"a\"] .. []; return a[0] == \"a\"; }"),
        Value::Bool(true)
    );
    assert_eq!(
        eval("#strict\nfunc Test() { SetLocal(0, \"a\"); return Local(0) == \"a\"; }"),
        Value::Bool(true)
    );
    assert_eq!(
        eval("#strict\nfunc Test() { return SetLocal(0, \"a\") == \"a\"; }"),
        Value::Bool(true)
    );
    let mut engine = Engine::new();
    engine
        .load_script("#strict\nfunc Test(target) { return target->SetLocal(0, \"a\") == \"a\"; }")
        .expect("loads");
    assert_eq!(
        engine.call("Test", &[Value::Object(1)]).expect("call"),
        Value::Bool(true)
    );
}

#[test]
fn strict1_arrays_compare_raw_identity() {
    assert_eq!(
        eval("#strict\nfunc Test() { return [1] == [1]; }"),
        Value::Bool(false)
    );
    assert_eq!(
        eval("#strict\nfunc Test() { var a = [1]; var b = a; return a == b; }"),
        Value::Bool(true)
    );
    assert_eq!(
        eval("#strict\nfunc Same(&a) { return a == a; } func Test() { var a = [1]; return Same(a); }"),
        Value::Bool(true)
    );
    assert_eq!(
        eval("#strict\nfunc Test() { var a = [1]; var b = a; b[0] = 1; return a == b; }"),
        Value::Bool(false)
    );
    assert_eq!(
        eval("#strict\nfunc Touch(&value) {} func Test() { var a = [1]; var b = a; Touch(a[0]); return a == b; }"),
        Value::Bool(false)
    );
    assert_eq!(
        eval("#strict\nfunc Touch(&value) {} func Test() { var inner = [1]; var a = [inner]; var b = inner; Touch(a[0][0]); return a[0] == b; }"),
        Value::Bool(false)
    );
    let mut engine = Engine::new();
    engine
        .load_script("#strict\nfunc Test(a) { var b = a; b[\"x\"] = 1; return a == b; }")
        .expect("map identity probe loads");
    assert_eq!(
        engine
            .call(
                "Test",
                &[Value::Proplist(ValueMap::from([("x", Value::Int(1),)]))],
            )
            .expect("map identity probe runs"),
        Value::Bool(false)
    );
    assert_eq!(
        eval("#strict\nfunc Test() { Local(0) = [1]; var old = Local(0); SetLocal(0, \"x\"); return Local(0) == old; }"),
        Value::Bool(false)
    );
}

#[test]
fn strict2_pointer_values_keep_content_equality() {
    assert_eq!(
        eval_with_format("#strict 2\nfunc Test() { return Format(\"%s\", \"a\") == \"a\"; }"),
        Value::Bool(true)
    );
    assert_eq!(
        eval("#strict 2\nfunc Test() { return [1] == [1]; }"),
        Value::Bool(true)
    );
}

#[test]
fn strict1_shared_cells_and_string_constants_keep_identity() {
    let globals = clonk_script::new_global_variables();
    let constants = clonk_script::new_global_variables();
    let mut engine = Engine::new();
    engine.set_global_variables(globals);
    engine.set_global_constants(constants);
    engine.register_host_function("Format", |_| Ok(Value::String("a".into())));
    crate::support::load_script(&mut engine, "#strict\nstatic s; static const S = \"a\";\nfunc Test() { s = Format(\"%s\", \"a\"); var t = s; return [s == s, s == t, S == \"a\", S() == \"a\"]; }");
    engine.adopt_statics_into_globals();

    assert_eq!(
        engine.call("Test", &[]).expect("call"),
        Value::Array(vec![
            Value::Bool(true),
            Value::Bool(true),
            Value::Bool(true),
            Value::Bool(true),
        ])
    );
}

eval_test! { strict3_distinguishes_types {
    "#strict 3\nfunc Test() { return 0 == nil; }" => Value::Bool(false);
    "#strict 3\nfunc Test() { return 1 == true; }" => Value::Bool(false);
} }

eval_cases! {
    strict3_not_equal_is_inverse:
        "#strict 3\nfunc Test() { return 0 != nil; }" => Value::Bool(true);
}

#[test]
fn strict_three_zero_c4id_remains_typed_for_equality() {
    let mut engine = Engine::new();
    engine.register_host_reference_function(
        "NativeEquality",
        std::iter::empty::<usize>(),
        |args| {
            let forward = args[0].c4_equals(&args[1], 3)?;
            let reverse = args[1].c4_equals(&args[0], 3)?;
            let legacy = args[0].c4_equals(&args[1], 2)?;
            let raw = args[0].c4_equals(&args[1], 1)?;
            Ok(Value::Array(vec![
                Value::Bool(forward),
                Value::Bool(reverse),
                Value::Bool(legacy),
                Value::Bool(raw),
                Value::Bool(!args[0].read()?.as_bool()),
            ]))
        },
    );
    assert!(
        engine.set_host_function_parameter_types("NativeEquality", [C4VType::C4Id, C4VType::C4Id])
    );
    engine.register_host_reference_function(
        "NativeStrict2MixedEquality",
        std::iter::empty::<usize>(),
        |args| {
            Ok(Value::Array(vec![
                Value::Bool(args[0].c4_equals(&args[1], 2)?),
                Value::Bool(args[1].c4_equals(&args[0], 2)?),
                Value::Bool(args[0].c4_equals(&args[1], 1)?),
            ]))
        },
    );
    assert!(engine.set_host_function_parameter_types(
        "NativeStrict2MixedEquality",
        [C4VType::C4Id, C4VType::Bool]
    ));
    engine.register_host_function("ToId", |args| Ok(args[0].clone()));
    assert!(engine.set_host_function_parameter_types("ToId", [C4VType::C4Id]));
    engine.register_host_reference_function("SlotIsId", [0], |args| {
        Ok(Value::Bool(args[0].read()?.c4v_type() == C4VType::C4Id))
    });
    assert!(engine.set_host_function_parameter_types("SlotIsId", [C4VType::Ref]));
    engine.register_host_reference_function("SetForwarded", [0], |args| {
        assert!(args[0].write(Value::Int(9))?);
        Ok(Value::Nil)
    });
    assert!(engine.set_host_function_parameter_types("SetForwarded", [C4VType::Ref]));
    engine.register_host_function("RetainedZeroIdArray", |_| {
        Ok(Value::Array(vec![Value::C4Id("NONE".into())]))
    });
    engine.register_host_function("RetainedZeroIdMapValue", |_| {
        let mut values = ValueMap::new();
        values.insert_key(Value::Int(1), Value::C4Id("NONE".into()));
        Ok(Value::Proplist(values))
    });
    engine.register_host_function("RawZeroId", |_| Ok(Value::C4Id("NONE".into())));
    engine
        .load_script(
            r#"#strict 3
               func &Entry(array values) { return values[0]; }
               global func &GlobalZero(id value) { return ToId(0); }
               func GlobalZeroMap() { return { [global->GlobalZero(0)] = 1 }; }
               func InheritedZero(id value) { return ToId(0); }
               func InheritedZero(id value) { return inherited(0); }
               func FreshReturnZero() { return ToId(0); }
               func RetainedZeroIdMap() {
                   var values = RetainedZeroIdArray();
                   return { [Entry(values)] = 1 };
               }
               func FreshMapReferenceValue() {
                   var values = RetainedZeroIdArray();
                   return { [1] = Entry(values) } == {};
               }
               func SameValueAssignRetainsZeroId() {
                   var values = RetainedZeroIdArray();
                   return SlotIsId(Entry(values) = ToId(0));
               }
               func ReferenceAssignCollapsesZeroId() {
                   var values = RetainedZeroIdArray();
                   return SlotIsId(Entry(values) = Entry(values));
               }
               func ShortCircuitSameAssignRetainsZeroId() {
                   var values = RetainedZeroIdArray();
                   return SlotIsId(Entry(values) = (ToId(0) ?? 1));
               }
               func CoalescingAssignmentKeepsZeroId() {
                   var values = RetainedZeroIdArray();
                   return SlotIsId(Entry(values) ??= ToId(1));
               }
               func ShortCircuitReferenceMap() {
                   var values = RetainedZeroIdArray();
                   return { [Entry(values) ?? 1] = 1 };
               }
               func ShortCircuitReferenceArgument() {
                   var values = RetainedZeroIdArray();
                   return SlotIsId(Entry(values) ?? 1);
               }
               func &ChooseRightReference(&value) { return nil ?? value; }
               func ShortCircuitPlainReferenceArgument() {
                   var values = RetainedZeroIdArray();
                   return SlotIsId(ChooseRightReference(Entry(values)));
               }
               func RejectValueAsReference() { return SlotIsId(ToId(0)); }
               func AssignMapTypedZero() {
                   var values = { [1] = nil };
                   values[1] = ToId(0);
                   return values == {};
               }
               func AssignMapReferenceZero() {
                   var values = { [1] = nil };
                   var retained = RetainedZeroIdArray();
                   values[1] = Entry(retained);
                   return values == { [1] = nil };
               }
               func AssignMapSameZero() {
                   var values = RetainedZeroIdMapValue();
                   values[1] = ToId(0);
                   return SlotIsId(values[1]);
               }
               func ObserveTypedSlot(id value) { return SlotIsId(value); }
               func Forwarded() { SetForwarded(...); return Par(0); }
               func ForwardedProbe() { return Forwarded(1); }
               func ForwardedNilProbe() { return Forwarded(nil); }
               func ArrayEntry(array value) { return value == nil; }
               func ConcatAssignSameZero() {
                   var left = RetainedZeroIdMapValue();
                   left ..= RetainedZeroIdMapValue();
                   return SlotIsId(left[1]);
               }
               func ConcatAssignDifferentZero() {
                   var left = { [1] = nil };
                   left ..= RetainedZeroIdMapValue();
                   return left == {};
               }
               func ConcatCopyZero() {
                   var left = RetainedZeroIdMapValue();
                   return (left .. RetainedZeroIdMapValue()) == {};
               }
               func ConcatArrayZero() {
                   var out = [] .. RetainedZeroIdArray();
                   return [out == [nil], SlotIsId(Entry(out))];
               }
               func ScriptEquality(id value) {
                   return [
                       value == nil, nil == value,
                       value != nil, nil != value,
                       value == 0, 0 == value,
                       !value
                   ];
               }
               func Test() {
                   var nil_map = { [nil] = 1 };
                   return [
                       NativeEquality(0, nil),
                       NativeStrict2MixedEquality(0, false),
                       NativeEquality(0, 0),
                       ObserveTypedSlot(0),
                       SameValueAssignRetainsZeroId(),
                       ReferenceAssignCollapsesZeroId(),
                       ShortCircuitSameAssignRetainsZeroId(),
                       CoalescingAssignmentKeepsZeroId(),
                       AssignMapTypedZero(),
                       AssignMapReferenceZero(),
                       AssignMapSameZero(),
                       ForwardedProbe(),
                       ForwardedNilProbe(),
                       ScriptEquality(0),
                       ToId(0) == nil,
                       (ToId(0) ?? 1) == nil,
                       (ToId(0) && 1) == nil,
                       (nil || ToId(0)) == nil,
                       InheritedZero(0) == nil,
                       FreshReturnZero() == nil,
                       RawZeroId() == nil,
                       [ToId(0)] == [nil],
                       { [1] = ToId(0) } == {},
                       FreshMapReferenceValue(),
                       GlobalZeroMap() == nil_map,
                       ConcatArrayZero(),
                       ConcatAssignSameZero(),
                       ConcatAssignDifferentZero(),
                       ConcatCopyZero(),
                       ShortCircuitReferenceArgument(),
                       ShortCircuitPlainReferenceArgument(),
                       ShortCircuitReferenceMap() == nil_map,
                       RetainedZeroIdMap() == nil_map,
                       nil_map == RetainedZeroIdMap(),
                       [RetainedZeroIdMap()] == [nil_map]
                   ];
               }
               "#,
        )
        .expect("typed zero-ID comparison script loads");

    let script_copy = Value::Array(vec![
        Value::Bool(true),
        Value::Bool(true),
        Value::Bool(false),
        Value::Bool(false),
        Value::Bool(false),
        Value::Bool(false),
        Value::Bool(true),
    ]);
    assert_eq!(
        engine.call("Test", &[]).expect("comparison probes run"),
        Value::Array(vec![
            // FnCnvInt2Id writes a retained C4V_C4ID tag for the first zero,
            // while nil remains C4V_Any. STRICT3 compares those tags first;
            // STRICT2 and raw-payload truthiness retain their legacy behavior.
            Value::Array(vec![
                Value::Bool(false),
                Value::Bool(false),
                Value::Bool(true),
                Value::Bool(true),
                Value::Bool(true),
            ]),
            // STRICT2's native operator is asymmetric by type, but C4ID and
            // Bool reject one another in both orders even at payload zero.
            // NONSTRICT/STRICT1 still compare their raw zero payloads.
            Value::Array(vec![
                Value::Bool(false),
                Value::Bool(false),
                Value::Bool(true),
            ]),
            Value::Array(vec![
                Value::Bool(true),
                Value::Bool(true),
                Value::Bool(true),
                Value::Bool(true),
                Value::Bool(true),
            ]),
            // A script-to-script conversion mutates the callee's live slot;
            // passing that slot by reference exposes the retained C4ID tag.
            Value::Bool(true),
            // C4Value::Set returns early when an identical retained zero ID is
            // assigned over the destination, so the tag is not erased.
            Value::Bool(true),
            // A reference RHS first passes through FnCnvDeref and loses the
            // tag before AB_Set writes it back to the destination.
            Value::Bool(false),
            // Jump-based ??/&&/|| expressions retain the selected operand's
            // stack slot. Assignment over the same typed destination and
            // ??='s type test therefore keep the zero-ID tag.
            Value::Bool(true),
            Value::Bool(true),
            // A map-owned slot removes a different typed-zero assignment;
            // FnCnvDeref first collapses a reference RHS, while an identical
            // typed destination takes Set's early return.
            Value::Bool(true),
            Value::Bool(true),
            Value::Bool(true),
            // `...` forwards AB_PARN_R aliases, allowing a native reference
            // parameter to update the forwarded parameter slot.
            Value::Int(9),
            Value::Int(9),
            // Reading the converted script parameter by value goes through
            // C4Value::Set and therefore canonicalizes the zero tag to nil.
            script_copy.clone(),
            // Direct AB_FUNC returns reuse their converted first argument as
            // the return slot. Both an ordinary call and inherited() retain
            // the identical zero-ID tag in that slot.
            Value::Bool(false),
            Value::Bool(false),
            Value::Bool(false),
            Value::Bool(false),
            Value::Bool(false),
            Value::Bool(true),
            Value::Bool(true),
            // Fresh array elements use C4Value::Set; a fresh map value does
            // too and CheckRemoveFromMap then erases its key. AB_CALLGLOBAL
            // stores its result in the nil target slot before map-key copying.
            Value::Bool(true),
            Value::Bool(true),
            Value::Bool(true),
            Value::Bool(true),
            Value::Array(vec![Value::Bool(true), Value::Bool(false)]),
            // In-place map concat preserves an identical typed destination;
            // a different destination and nonassigning concat both erase it.
            Value::Bool(true),
            Value::Bool(true),
            Value::Bool(true),
            Value::Bool(true),
            Value::Bool(true),
            // A selected reference operand remains raw until AB_MAP copies
            // GetRefVal(), so the retained zero ID is still a distinct key.
            Value::Bool(false),
            Value::Bool(false),
            Value::Bool(false),
            Value::Bool(false),
        ]),
        "retained zero IDs stay typed only until a C4Value::Set copy"
    );
    assert!(
        engine
            .call("RejectValueAsReference", &[])
            .expect_err("a retained value is not a reference")
            .to_string()
            .contains(r#"got "id", but expected "&""#),
        "a reference-typed parameter observes the un-copied value tag"
    );
    assert_eq!(
        engine
            .call("ScriptEquality", &[Value::Int(0)])
            .expect("engine-entry conversion runs"),
        script_copy,
        "engine entry converts first, then Set-copies its temporary parameter slots"
    );
    assert_eq!(
        engine
            .call("ObserveTypedSlot", &[Value::Int(0)])
            .expect("external typed entry runs"),
        Value::Bool(false),
        "external script entry Set-copies converted slots before the body"
    );
    assert_eq!(
        engine
            .call("ToId", &[Value::Int(0)])
            .expect("external native conversion runs"),
        Value::Nil,
        "external native Exec eagerly clears falsy values before conversion"
    );
    assert_eq!(
        engine
            .call("RawZeroId", &[])
            .expect("external native return runs"),
        Value::C4Id("NONE".into()),
        "external native Exec returns its C4Value by copy without a stack Set"
    );
    assert_eq!(
        engine
            .call("ArrayEntry", &[Value::C4Id("NONE".into())])
            .expect("external C4AulParSet copy precedes conversion"),
        Value::Bool(true)
    );
}

#[test]
fn yielded_global_reference_result_is_materialized_once_before_map_keying() {
    let mut engine = Engine::new();
    engine.register_host_function_with_arity("Pause", 0, |_| {
        Err(RuntimeError::host_continuation(PauseRequest, Value::Nil))
    });
    engine.register_host_function_with_arity("ToId", 1, |args| Ok(args[0].clone()));
    assert!(engine.set_host_function_parameter_types("ToId", [C4VType::C4Id]));
    engine
        .load_script(
            r#"#strict 3
               global func &GlobalZero() { return ToId(0); }
               global func &GlobalZeroYield() { Pause(); return ToId(0); }
               func SyncMap() { return { [global->GlobalZero()] = 1 }; }
               func YieldMap() { return { [global->GlobalZeroYield()] = 1 }; }
               "#,
        )
        .expect("yielding global result script loads");

    let expected = engine.call("SyncMap", &[]).expect("synchronous map builds");
    let suspension = match engine
        .call_with_continuation("YieldMap", &[])
        .expect("global callee yields")
    {
        ScriptCallOutcome::Suspended(suspension) => suspension,
        ScriptCallOutcome::Complete(_) => panic!("yielding global callee completed early"),
    };
    let yielded = match engine
        .resume_script_continuation(suspension)
        .expect("global callee resumes")
    {
        ScriptCallOutcome::Complete(value) => value,
        ScriptCallOutcome::Suspended(_) => panic!("global callee yielded twice"),
    };
    assert_eq!(yielded, expected);
}

eval_test! { same_type_equality_holds_at_all_levels {
    "func Test() { return 5 == 5; }" => Value::Bool(true);
    "#strict 3\nfunc Test() { return 5 == 5; }" => Value::Bool(true);
    "func Test() { var left, right; return left == right; }" => Value::Bool(true);
    "#strict 3\nfunc Test() { return 7 != 8; }" => Value::Bool(true);
} }
