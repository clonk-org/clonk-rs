use std::{fs, io};

mod common;

use common::unique_temp_dir;

#[test]
fn a_failed_install_leaves_the_session_log_untouched() {
    // Test harnesses and embedders install their own global dispatcher, which
    // makes our install fail. Discovering that only after rotating and
    // truncating would spend the user's log file on a session that never
    // recorded anything.
    let directory = unique_temp_dir("clonk-logging-failed");
    let log_path = directory.join("Clonk.log");
    fs::write(&log_path, "prior session\n").expect("seed the previous session log");

    tracing::subscriber::set_global_default(tracing_subscriber::registry())
        .expect("install a foreign global dispatcher");

    let error = clonk_logging::init_verbose_with_file(false, &log_path)
        .expect_err("installing over a foreign dispatcher must fail");

    assert_eq!(error.kind(), io::ErrorKind::AlreadyExists);
    assert_eq!(
        fs::read_to_string(&log_path).expect("read the session log"),
        "prior session\n",
        "the existing session log is left alone"
    );
    assert!(
        !directory.join("Clonk.previous.log").exists(),
        "nothing was rotated"
    );

    let _ = fs::remove_dir_all(&directory);
}
