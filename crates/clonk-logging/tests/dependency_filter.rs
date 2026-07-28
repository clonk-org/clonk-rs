use std::{env, process::Command};

const CHILD_MODE: &str = "LC_DEPENDENCY_FILTER_CHILD";
const APP_MARKER: &str = "application debug marker";
const DEPENDENCY_MARKER: &str = "dependency debug marker";

#[test]
fn an_explicit_directive_keeps_the_dependency_suppression() {
    // Raising the level is what a user does to capture a bug report. If that
    // also unmuted the graphics stack, the report would be mostly wgpu.
    if env::var_os(CHILD_MODE).is_some() {
        clonk_logging::init();
        tracing::debug!("{APP_MARKER}");
        tracing::debug!(target: "wgpu_core::device", "{DEPENDENCY_MARKER}");
        tracing::debug!(target: "naga::front", "{DEPENDENCY_MARKER}");
        return;
    }

    let output = Command::new(env::current_exe().expect("integration-test executable"))
        .args([
            "--exact",
            "an_explicit_directive_keeps_the_dependency_suppression",
            "--nocapture",
        ])
        .env(CHILD_MODE, "1")
        .env_remove("LC_LOG")
        .env("RUST_LOG", "debug")
        .output()
        .expect("run the isolated logging child");

    assert!(
        output.status.success(),
        "child failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        stderr.contains(APP_MARKER),
        "the explicit directive still raises our own level"
    );
    assert!(
        !stderr.contains(DEPENDENCY_MARKER),
        "dependency debug output survived an explicit directive: {stderr}"
    );
}
