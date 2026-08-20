use std::fs;

use clonk_core::log_target::{SCRIPT_LOG_TARGET, SCRIPT_TRACE_TARGET};
use clonk_logging::GameLogCapture;

mod common;

use common::unique_temp_dir;

#[test]
fn only_c4script_log_lines_reach_the_message_board_sink() {
    // `C4LogSystem` attaches its GuiSink to the loggers whose output C++ shows
    // in-game; `Log()` from C4Script is the stream that carries content lines
    // such as Hazard's kill messages (src/C4Log.cpp:226-240;
    // src/C4Script.cpp FnLog). Engine-internal Rust tracing has no C++
    // counterpart and must stay out of the message board.
    let log_path = unique_temp_dir("clonk-logging-gamelog").join("Clonk.log");
    let capture = GameLogCapture::default();
    clonk_logging::init_verbose_with_file_and_capture(
        false,
        &log_path,
        None,
        Some(capture.clone()),
    )
    .expect("initialize the session log");

    tracing::info!(target: SCRIPT_LOG_TARGET, "Alpha riddles Beta to death.");
    tracing::info!(target: SCRIPT_TRACE_TARGET, "call trace noise");
    tracing::info!("engine session log initialized");

    assert_eq!(
        capture.take(),
        vec!["Alpha riddles Beta to death.".to_string()]
    );
    let session_log = fs::read_to_string(&log_path).expect("read the session log");
    assert!(
        session_log.contains("engine session log initialized"),
        "the file sink still receives every logged line"
    );

    if let Some(parent) = log_path.parent() {
        let _ = fs::remove_dir_all(parent);
    }
}
