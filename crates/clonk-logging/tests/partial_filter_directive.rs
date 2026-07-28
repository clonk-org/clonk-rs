use std::{env, process::Command};

const CHILD_MODE: &str = "LC_PARTIAL_FILTER_CHILD";
const INFO_MARKER: &str = "partial filter info marker";
const ERROR_MARKER: &str = "partial filter error marker";

fn run_child(mode: &str, directive: &str) -> String {
    let output = Command::new(env::current_exe().expect("integration-test executable"))
        .args(["--exact", mode, "--nocapture"])
        .env(CHILD_MODE, "1")
        .env_remove("RUST_LOG")
        .env("LC_LOG", directive)
        .output()
        .expect("run the isolated logging child");
    assert!(
        output.status.success(),
        "child failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    String::from_utf8_lossy(&output.stderr).into_owned()
}

fn emit_markers() {
    clonk_logging::init();
    tracing::info!("{INFO_MARKER}");
    tracing::error!("{ERROR_MARKER}");
}

#[test]
fn a_partially_invalid_directive_keeps_the_part_that_parses() {
    // Dropping the whole directive over one typo moves the level in whichever
    // direction the default happens to sit — here the user asked to go quieter
    // and would be made louder.
    if env::var_os(CHILD_MODE).is_some() {
        emit_markers();
        return;
    }
    let stderr = run_child(
        "a_partially_invalid_directive_keeps_the_part_that_parses",
        "error,bogus=definitely-not-a-level",
    );
    assert!(
        stderr.contains(ERROR_MARKER),
        "the directive that parses is still applied"
    );
    assert!(
        !stderr.contains(INFO_MARKER),
        "the requested level was not applied: {stderr}"
    );
}

#[test]
fn an_empty_directive_falls_back_to_the_default_level() {
    // Shell wrappers routinely export an unset variable as the empty string.
    if env::var_os(CHILD_MODE).is_some() {
        emit_markers();
        return;
    }
    let stderr = run_child("an_empty_directive_falls_back_to_the_default_level", "");
    assert!(
        stderr.contains(INFO_MARKER),
        "an empty directive silenced the default level: {stderr}"
    );
}
