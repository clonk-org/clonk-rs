use clonk_script::{DebuggerHooks, Engine, RuntimeError, Value};
use std::sync::{Arc, Mutex};

fn load_script(engine: &mut Engine, source: &str) {
    crate::support::load_script(engine, source);
}

#[test]
fn executes_basic_arithmetic() {
    let mut engine = Engine::new();
    load_script(
        &mut engine,
        r#"
        global func Add(a, b) {
            return a + b;
        }
        func Double(x) {
            var value = Add(x, x);
            return value;
        }
        "#,
    );

    let result = engine
        .call("Add", &[Value::Int(21), Value::Int(21)])
        .expect("call succeeds");
    assert_eq!(result, Value::Int(42));

    let double = engine
        .call("Double", &[Value::Int(7)])
        .expect("call succeeds");
    assert_eq!(double, Value::Int(14));
}

#[test]
fn nonstrict_standalone_goto_returns_immediately() {
    fn run(source: &str) -> Value {
        let mut engine = Engine::new();
        engine.register_host_function("goto", |args| {
            Ok(args.first().cloned().unwrap_or(Value::Nil))
        });
        load_script(&mut engine, source);
        engine.call("Probe", &[]).expect("Probe runs")
    }

    for (directive, expected) in [
        ("", Value::Int(41)),
        ("#strict\n", Value::Int(99)),
        ("#strict 2\n", Value::Int(99)),
        ("#strict 3\n", Value::Int(99)),
    ] {
        assert_eq!(
            run(&format!(
                "{directive}func Probe() {{ goto(40 + 1); return 99; }}"
            )),
            expected,
            "only a NONSTRICT bare goto statement returns implicitly"
        );
    }

    assert_eq!(
        run("func Probe() { goto(40 + 1) + 1; return 99; }"),
        Value::Int(41),
        "C++ returns the goto result before evaluating its parsed suffix"
    );
    assert_eq!(
        run("func Probe() { var value = goto(41); return value + 1; }"),
        Value::Int(42),
        "an embedded goto call is an ordinary expression"
    );
    assert_eq!(
        run("func Probe() { (goto(41)); return 99; }"),
        Value::Int(99),
        "a parenthesized goto does not start the legacy statement path"
    );
    assert_eq!(
        run("func Goto(value) { return value; } func Probe() { Goto(41); return 99; }"),
        Value::Int(99),
        "the legacy spelling check is case-sensitive"
    );
    assert_eq!(
        run("func Probe() { var goto; goto(41); return 99; }"),
        Value::Int(99),
        "a named binding takes precedence over the legacy goto hack"
    );
    assert_eq!(
        run("func Probe() { goto(41); return 99; }\n#strict\n"),
        Value::Int(99),
        "the final origin strictness applies even when its directive is later"
    );

    let malformed =
        clonk_script::Script::compile("#strict 2\nfunc Probe() { goto(@); return 99; }")
            .expect("function recovery retains the malformed script");
    assert!(
        malformed
            .parse_diagnostics()
            .iter()
            .any(|error| error.message() == "unexpected character '@'"),
        "the leading-call probe must not consume and hide lexer errors"
    );
}

run_cases! {
    handles_conditionals_and_loops:
        r#"
        global func SumUntil(limit) {
            var acc = 0;
            var current = 1;
            while (current <= limit) {
                acc = acc + current;
                current = current + 1;
            }
            return acc;
        }
        "#,
        "SumUntil", &[Value::Int(5)] => Value::Int(15);

    supports_strings_and_concatenation:
        r#"
        global func Greeting(name) {
            var message = "Hello, " .. name;
            return message .. "!";
        }
        "#,
        "Greeting", &[Value::String("World".into())] => Value::String("Hello, World!".into());

    handles_recursion:
        r#"
        global func Factorial(n) {
            if (n <= 1) {
                return 1;
            }
            return n * Factorial(n - 1);
        }
        "#,
        "Factorial", &[Value::Int(5)] => Value::Int(120);
}

#[test]
fn reports_unknown_function() {
    let engine = Engine::new();
    let error = engine.call("Missing", &[]).unwrap_err();
    assert!(format!("{error}").contains("unknown function"));
}

#[test]
fn host_function_can_be_called_directly() {
    let mut engine = Engine::new();
    engine.register_host_function("HostAdd", |args| {
        let lhs = match args.first() {
            Some(Value::Int(value)) => *value,
            _ => {
                return Err(RuntimeError::new(
                    "HostAdd expects first argument to be an int",
                ))
            }
        };
        let rhs = match args.get(1) {
            Some(Value::Int(value)) => *value,
            _ => {
                return Err(RuntimeError::new(
                    "HostAdd expects second argument to be an int",
                ))
            }
        };
        Ok(Value::Int(lhs + rhs))
    });

    let result = engine
        .call("HostAdd", &[Value::Int(40), Value::Int(2)])
        .expect("host call succeeds");
    assert_eq!(result, Value::Int(42));
}

#[test]
fn script_can_call_host_function() {
    let mut engine = Engine::new();
    engine.register_host_function("HostMul", |args| {
        let lhs = match args.first() {
            Some(Value::Int(value)) => *value,
            _ => {
                return Err(RuntimeError::new(
                    "HostMul expects first argument to be an int",
                ))
            }
        };
        let rhs = match args.get(1) {
            Some(Value::Int(value)) => *value,
            _ => {
                return Err(RuntimeError::new(
                    "HostMul expects second argument to be an int",
                ))
            }
        };
        Ok(Value::Int(lhs * rhs))
    });

    load_script(
        &mut engine,
        r#"
        global func DoubleProduct(a, b) {
            return HostMul(a, b) * 2;
        }
        "#,
    );

    let result = engine
        .call("DoubleProduct", &[Value::Int(3), Value::Int(4)])
        .expect("script call succeeds");
    assert_eq!(result, Value::Int(24));
}

#[test]
fn host_function_errors_propagate() {
    let mut engine = Engine::new();
    engine.register_host_function("HostFail", |_| Err(RuntimeError::new("host failure")));

    let error = engine.call("HostFail", &[]).unwrap_err();
    assert!(format!("{error}").contains("host failure"));
}

run_cases! {
    supports_arrays_and_indexing:
        r#"
        #strict
        global func ThirdElement() {
            var arr = [1, 2, 3, 4];
            return arr[2];
        }
        "#,
        "ThirdElement", &[] => Value::Int(3);

    array_literal_empty_slots_match_cpp:
        r#"
        #strict
        global func EmptySlots() {
            return [[], [,], [,,], [1,], [1,,2], [,1,,], [[,],[2,]], [3,4]];
        }
        "#,
        "EmptySlots", &[] =>
        Value::Array(vec![
            Value::Array(vec![]),
            Value::Array(vec![Value::Nil, Value::Nil]),
            Value::Array(vec![Value::Nil, Value::Nil, Value::Nil]),
            Value::Array(vec![Value::Int(1), Value::Nil]),
            Value::Array(vec![Value::Int(1), Value::Nil, Value::Int(2)]),
            Value::Array(vec![Value::Nil, Value::Int(1), Value::Nil, Value::Nil]),
            Value::Array(vec![
                Value::Array(vec![Value::Nil, Value::Nil]),
                Value::Array(vec![Value::Int(2), Value::Nil]),
            ]),
            Value::Array(vec![Value::Int(3), Value::Int(4)]),
        ]);

    supports_proplists_and_nested_access:
        r#"
        #strict 3
        global func ProplistQuery() {
            var data = { foo = 42, nested = { value = 7 }, numbers = [5, 9] };
            return data.foo + data.nested.value + data.numbers[1];
        }
        "#,
        "ProplistQuery", &[] => Value::Int(58);

    statement_map_literal_evaluates_key_and_value_side_effects:
        r#"
        #strict 3
        static calls;
        func Mark(amount) { calls += amount; return calls; }
        global func StatementMap() {
            calls = 0;
            { [Mark(1)] = Mark(10), nested = { value = Mark(100) } };
            return calls;
        }
        "#,
        "StatementMap", &[] => Value::Int(111);

    assigns_to_proplist_properties:
        r#"
        #strict 3
        global func Mutate() {
            var data = { foo = 1, nested = { value = 2 } };
            data.foo = data.foo + 41;
            data.nested.value = data.foo - 36;
            data.new_field = 3;
            return data.foo + data.nested.value + data.new_field;
        }
        "#,
        "Mutate", &[] => Value::Int(51);
}

#[test]
fn property_assignment_reports_type_errors() {
    let mut engine = Engine::new();
    load_script(
        &mut engine,
        r#"
        #strict 3
        global func BadAssign() {
            var value = 5;
            value.foo = 1;
        }
        "#,
    );

    let error = engine.call("BadAssign", &[]).unwrap_err();
    assert!(format!("{error}").contains("cannot assign property 'foo'"));
}

#[test]
fn effect_callbacks_dispatch_via_engine_helper() {
    let mut engine = Engine::new();
    load_script(
        &mut engine,
        r#"
        global func FxFireStart(effect, target) {
            return effect + target;
        }
        "#,
    );

    let effect_result = engine
        .call_effect_callback("Fire", "Start", &[Value::Int(10), Value::Int(5)])
        .expect("effect dispatch succeeds");
    assert_eq!(effect_result, Some(Value::Int(15)));

    let missing = engine
        .call_effect_callback("Fire", "Stop", &[])
        .expect("missing callback returns None");
    assert!(missing.is_none());
}

#[test]
fn debugger_hooks_capture_call_and_return() {
    let mut engine = Engine::new();
    load_script(
        &mut engine,
        r#"
        global func AddOne(value) {
            return value + 1;
        }
        "#,
    );

    let call_log: Arc<Mutex<Vec<String>>> = Arc::new(Mutex::new(Vec::new()));
    let return_log: Arc<Mutex<Vec<String>>> = Arc::new(Mutex::new(Vec::new()));

    let mut hooks = DebuggerHooks::new();
    {
        let call_log = Arc::clone(&call_log);
        hooks.set_on_call(move |name, args| {
            assert_eq!(args, [Value::Int(41)]);
            let mut log = call_log.lock().unwrap();
            log.push(format!("{name}({})", args.len()));
        });
    }
    {
        let return_log = Arc::clone(&return_log);
        hooks.set_on_return(move |name, value| {
            let mut log = return_log.lock().unwrap();
            log.push(format!("{name} -> {value}"));
        });
    }
    engine.set_debugger_hooks(hooks);

    let result = engine
        .call("AddOne", &[Value::Int(41)])
        .expect("call succeeds");
    assert_eq!(result, Value::Int(42));

    let call_entries = call_log.lock().unwrap().clone();
    assert_eq!(call_entries, vec!["AddOne(1)".to_string()]);
    let return_entries = return_log.lock().unwrap().clone();
    assert_eq!(return_entries, vec!["AddOne -> 42".to_string()]);
}

const CANONICAL_SCENARIO: &str = include_str!("../../src/fixtures/canonical/basic.aul");

#[test]
fn canonical_scenario_parity_harness() {
    let mut engine = Engine::new();
    engine
        .load_script(CANONICAL_SCENARIO)
        .expect("canonical script loads");

    let array_sum = engine
        .call("CanonicalArrayCheck", &[])
        .expect("array parity call succeeds");
    assert_eq!(array_sum, Value::Int(21));

    let proplist = engine
        .call("CanonicalProplistCheck", &[])
        .expect("proplist parity call succeeds");
    assert_eq!(proplist, Value::Int(53));

    let effect = engine
        .call_effect_callback("Canonical", "Start", &[Value::Int(7)])
        .expect("effect callback dispatches");
    assert_eq!(effect, Some(Value::Int(7)));
}

run_cases! {
    supports_access_modifiers_on_functions:
        r#"
        private func PrivateHelper() {
            return 10;
        }

        protected func ProtectedHelper() {
            return 20;
        }

        public func PublicHelper() {
            return 30;
        }

        global func GlobalHelper() {
            return 40;
        }

        global func CallAll() {
            return PrivateHelper() + ProtectedHelper() + PublicHelper() + GlobalHelper();
        }
        "#,
        "CallAll", &[] => Value::Int(100);
}

#[test]
fn return_statement_handles_parenthesized_expressions_with_operators() {
    let mut engine = Engine::new();
    load_script(
        &mut engine,
        r#"
        global func ReturnParenDivide() {
            return (255*100)/100;
        }

        global func ReturnParenAdd() {
            return (100)+50;
        }

        global func ReturnParenMultiply() {
            return (10)*5;
        }

        global func ReturnComplexExpr() {
            return (255*GetIntensity())/100;
        }

        private func GetIntensity() {
            return 80;
        }
        "#,
    );

    assert_eq!(
        engine
            .call("ReturnParenDivide", &[])
            .expect("call succeeds"),
        Value::Int(255)
    );
    assert_eq!(
        engine.call("ReturnParenAdd", &[]).expect("call succeeds"),
        Value::Int(150)
    );
    assert_eq!(
        engine
            .call("ReturnParenMultiply", &[])
            .expect("call succeeds"),
        Value::Int(50)
    );
    assert_eq!(
        engine
            .call("ReturnComplexExpr", &[])
            .expect("call succeeds"),
        Value::Int(204)
    );
}

#[test]
fn array_index_assignment_works() {
    let mut engine = Engine::new();
    load_script(
        &mut engine,
        r#"
        #strict
        global func TestArrayIndexAssignment() {
            var arr = [0, 0, 0];
            arr[0] = 10;
            arr[1] = 20;
            arr[2] = 30;
            return arr[0] + arr[1] + arr[2];
        }

        global func TestNestedArrayAssignment() {
            var matrix = [[0, 0], [0, 0]];
            matrix[0][0] = 1;
            matrix[0][1] = 2;
            matrix[1][0] = 3;
            matrix[1][1] = 4;
            return matrix[0][0] + matrix[0][1] + matrix[1][0] + matrix[1][1];
        }
        "#,
    );

    assert_eq!(
        engine
            .call("TestArrayIndexAssignment", &[])
            .expect("call succeeds"),
        Value::Int(60)
    );
    assert_eq!(
        engine
            .call("TestNestedArrayAssignment", &[])
            .expect("call succeeds"),
        Value::Int(10)
    );
}
