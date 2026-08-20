use std::fs;

mod common;

use common::unique_temp_dir;

#[test]
fn the_startup_banner_records_the_build_and_platform() {
    // The port version and the engine version diverge deliberately, so a log
    // that quotes neither cannot be matched to a build at all.
    let directory = unique_temp_dir("clonk-logging-banner");
    let log_path = directory.join("Clonk.log");
    clonk_logging::init_verbose_with_file(false, &log_path).expect("initialize the session log");
    clonk_logging::log_startup_banner("0.4.0", "4.9.11");

    let session_log = fs::read_to_string(&log_path).expect("read the session log");
    for expected in [
        "port_version=\"0.4.0\"",
        "engine_version=\"4.9.11\"",
        "os=",
        "arch=",
        "starting clonk",
    ] {
        assert!(
            session_log.contains(expected),
            "the banner is missing {expected}: {session_log}"
        );
    }

    let _ = fs::remove_dir_all(&directory);
}
