use std::{env, fs, path::PathBuf, process::Command};

mod common;

use common::unique_temp_dir;

const CHILD_LOG_PATH: &str = "LC_PANIC_HOOK_CHILD_LOG";
const PANIC_MESSAGE: &str = "deliberate child panic";

#[test]
fn a_panic_reaches_the_session_log() {
    // The default hook writes straight to stderr, bypassing the writer stack —
    // and a windowed build has no stderr at all. Without this the one event
    // that ends the session is missing from the file the user sends us.
    if let Some(path) = env::var_os(CHILD_LOG_PATH) {
        let log_path = PathBuf::from(path);
        clonk_logging::init_verbose_with_file(false, &log_path).expect("initialize the log");
        clonk_logging::install_panic_hook();
        panic!("{PANIC_MESSAGE}");
    }

    let directory = unique_temp_dir("clonk-logging-panic");
    let log_path = directory.join("Clonk.log");
    let output = Command::new(env::current_exe().expect("integration-test executable"))
        .args(["--exact", "a_panic_reaches_the_session_log", "--nocapture"])
        .env(CHILD_LOG_PATH, &log_path)
        .env_remove("LC_LOG")
        .env_remove("RUST_LOG")
        .output()
        .expect("run the panicking child");

    assert!(
        !output.status.success(),
        "the child was expected to panic and fail"
    );
    let session_log = fs::read_to_string(&log_path).expect("read the session log");
    assert!(
        session_log.contains(PANIC_MESSAGE),
        "the panic payload is missing from the session log: {session_log}"
    );
    assert!(
        session_log.contains("panic_hook.rs"),
        "the panic location is missing from the session log: {session_log}"
    );

    let _ = fs::remove_dir_all(&directory);
}
