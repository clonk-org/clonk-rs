use std::fs;

mod common;

use common::unique_temp_dir;

#[test]
fn a_fatal_error_reaches_the_session_log() {
    // An error returned out of `main` is printed by the Rust runtime to stderr
    // and nowhere else, so in a windowed build the one event that ended the
    // session is missing from the file a bug report attaches — the session log
    // just stops (clonk-org/clonk-rs#40). Route it through the log first.
    let directory = unique_temp_dir("clonk-logging-fatal");
    let log_path = directory.join("Clonk.log");
    clonk_logging::init_verbose_with_file(false, &log_path).expect("initialize the session log");
    clonk_logging::log_fatal_error("application event loop failed: broken pipe");

    let session_log = fs::read_to_string(&log_path).expect("read the session log");
    for expected in [
        "error=\"application event loop failed: broken pipe\"",
        "stopping clonk after a fatal error",
    ] {
        assert!(
            session_log.contains(expected),
            "the fatal error is missing {expected}: {session_log}"
        );
    }

    let _ = fs::remove_dir_all(&directory);
}
