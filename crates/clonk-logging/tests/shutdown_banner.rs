use std::fs;

mod common;

use common::unique_temp_dir;

#[test]
fn the_shutdown_banner_records_that_the_session_ended_on_purpose() {
    // Without it a log that simply stops is equally consistent with a clean
    // quit and with the process being destroyed, which is the fork every
    // "it just vanished" report gets stuck on (clonk-org/clonk-rs#40).
    let directory = unique_temp_dir("clonk-logging-shutdown");
    let log_path = directory.join("Clonk.log");
    clonk_logging::init_verbose_with_file(false, &log_path).expect("initialize the session log");
    clonk_logging::log_shutdown_banner("the main menu was closed");

    let session_log = fs::read_to_string(&log_path).expect("read the session log");
    for expected in ["reason=\"the main menu was closed\"", "stopping clonk"] {
        assert!(
            session_log.contains(expected),
            "the banner is missing {expected}: {session_log}"
        );
    }

    let _ = fs::remove_dir_all(&directory);
}
