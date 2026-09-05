use std::error::Error;

use clonk_script::{RuntimeError, ScriptError, ScriptErrorDiagnostic, Value};

fn assert_send_sync_error<T: Error + Send + Sync + 'static>() {}

#[test]
fn ordinary_diagnostics_are_send_sync_errors() {
    assert_send_sync_error::<ScriptErrorDiagnostic>();
}

#[test]
fn diagnostics_preserve_parse_and_runtime_display() {
    let parse = ScriptError::parse("bad token", 4, 9)
        .into_diagnostic()
        .expect("parse errors have no continuation");
    assert_eq!(parse.to_string(), "parse error at 4:9: bad token");
    assert_eq!(parse.message(), "bad token");
    assert!(parse.call_frames().is_empty());

    let runtime = ScriptError::from(RuntimeError::new("bad value"))
        .into_diagnostic()
        .expect("ordinary runtime errors have no continuation");
    assert_eq!(runtime.to_string(), "runtime error: bad value");
    assert_eq!(runtime.message(), "bad value");
    assert!(runtime.call_frames().is_empty());
}

#[test]
fn continuation_errors_are_rejected_by_diagnostic_conversion() {
    let error = ScriptError::from(RuntimeError::host_continuation((), Value::Nil));
    let returned = error
        .into_diagnostic()
        .expect_err("a host continuation must remain in the VM-facing channel");
    assert!(returned
        .to_string()
        .contains("script execution suspended by host"));
}
