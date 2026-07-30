use std::{env, process::Command};

const CHILD_MODE: &str = "LC_DEFAULT_INIT_CHILD";
const MARKER: &str = "default init marker";

#[test]
fn default_init_writes_to_stderr_without_escapes() {
    // `init()` serves the tool binaries, whose stdout is their deliverable and
    // is parsed by scripts. Diagnostics belong on stderr, and colour belongs
    // only on a terminal.
    if env::var_os(CHILD_MODE).is_some() {
        clonk_logging::init();
        tracing::info!("{MARKER}");
        return;
    }

    let output = Command::new(env::current_exe().expect("integration-test executable"))
        .args([
            "--exact",
            "default_init_writes_to_stderr_without_escapes",
            "--nocapture",
        ])
        .env(CHILD_MODE, "1")
        .env_remove("LC_LOG")
        .env_remove("RUST_LOG")
        .output()
        .expect("run the isolated logging child");

    assert!(
        output.status.success(),
        "child failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    let stdout = String::from_utf8_lossy(&output.stdout);
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        !stdout.contains(MARKER),
        "log output landed on stdout: {stdout:?}"
    );
    assert!(stderr.contains(MARKER), "log output missing from stderr");
    assert!(
        !stderr.contains('\u{1b}'),
        "ANSI escapes written to a stderr that is not a terminal"
    );
}
