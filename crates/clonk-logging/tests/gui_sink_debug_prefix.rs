use std::fs;

use clonk_core::log_target::SCRIPT_LOG_TARGET;
use clonk_logging::GameLogCapture;

mod common;

use common::unique_temp_dir;

#[test]
fn debug_and_trace_lines_carry_a_severity_marker() {
    // The GUI sinks show the message alone, so the severity marker is the only
    // thing distinguishing a diagnostic from the content text around it. Debug
    // and trace lines carried no marker and read as game output.
    let log_path = unique_temp_dir("clonk-logging-guidebug").join("Clonk.log");
    let capture = GameLogCapture::default();
    clonk_logging::init_verbose_with_file_and_capture(true, &log_path, None, Some(capture.clone()))
        .expect("initialize the session log");

    tracing::debug!(target: SCRIPT_LOG_TARGET, "probe");
    tracing::info!(target: SCRIPT_LOG_TARGET, "Alpha is dead.");

    assert_eq!(
        capture.take(),
        vec!["DEBUG: probe".to_string(), "Alpha is dead.".to_string()]
    );

    if let Some(parent) = log_path.parent() {
        let _ = fs::remove_dir_all(parent);
    }
}
