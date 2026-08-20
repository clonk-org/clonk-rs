use crate::support::{runtime_error as shared_runtime_error, try_eval};
use clonk_script::{ScriptError, Value};

fn call_test(source: &str) -> Result<Value, ScriptError> {
    try_eval(source, &[])
}

fn runtime_error(source: &str) -> String {
    shared_runtime_error(source, &[])
}

#[test]
fn single_variable_for_in_over_nil_reports_the_cpp_type_error() {
    let error = runtime_error(
        r#"
        func Test() {
            var values;
            for (var value in values);
            return 1;
        }
        "#,
    );

    assert_eq!(error, "for: array expected, but got nil!");
}

#[test]
fn single_variable_for_in_over_int_reports_the_cpp_type_error() {
    let error = runtime_error(
        r#"
        func Test() {
            for (var value in 5);
            return 1;
        }
        "#,
    );

    assert_eq!(error, "for: array expected, but got int!");
}

#[test]
fn single_variable_for_in_over_empty_array_continues_after_the_loop() {
    assert_eq!(
        call_test(
            r#"
            #strict
            func Test() {
                for (var value in []);
                return 1;
            }
            "#,
        )
        .expect("an empty array should run zero iterations"),
        Value::Int(1)
    );
}
