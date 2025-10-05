use crate::{Engine, Value};

fn load_script(engine: &mut Engine, source: &str) {
    engine.load_script(source).expect("script should load");
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
fn handles_conditionals_and_loops() {
    let mut engine = Engine::new();
    load_script(
        &mut engine,
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
    );

    let sum = engine
        .call("SumUntil", &[Value::Int(5)])
        .expect("call succeeds");
    assert_eq!(sum, Value::Int(15));
}

#[test]
fn supports_strings_and_concatenation() {
    let mut engine = Engine::new();
    load_script(
        &mut engine,
        r#"
        global func Greeting(name) {
            var message = "Hello, " + name;
            return message + "!";
        }
        "#,
    );

    let name = Value::String("World".into());
    let greeting = engine.call("Greeting", &[name]).expect("call succeeds");
    assert_eq!(greeting, Value::String("Hello, World!".into()));
}

#[test]
fn handles_recursion() {
    let mut engine = Engine::new();
    load_script(
        &mut engine,
        r#"
        global func Factorial(n) {
            if (n <= 1) {
                return 1;
            }
            return n * Factorial(n - 1);
        }
        "#,
    );

    let result = engine
        .call("Factorial", &[Value::Int(5)])
        .expect("call succeeds");
    assert_eq!(result, Value::Int(120));
}

#[test]
fn reports_unknown_function() {
    let engine = Engine::new();
    let error = engine.call("Missing", &[]).unwrap_err();
    assert!(format!("{error}").contains("unknown function"));
}
