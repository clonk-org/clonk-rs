use std::{
    env, fs,
    path::PathBuf,
    process,
    time::{SystemTime, UNIX_EPOCH},
};

use clonk_logging::GameLogCapture;

#[test]
fn only_c4script_log_lines_reach_the_message_board_sink() {
    // `C4LogSystem` attaches its GuiSink to the loggers whose output C++ shows
    // in-game; `Log()` from C4Script is the stream that carries content lines
    // such as Hazard's kill messages (src/C4Log.cpp:226-240;
    // src/C4Script.cpp FnLog). Engine-internal Rust tracing has no C++
    // counterpart and must stay out of the message board.
    let log_path = unique_temp_dir().join("Clonk.log");
    let capture = GameLogCapture::default();
    clonk_logging::init_verbose_with_file_and_capture(
        false,
        &log_path,
        None,
        Some(capture.clone()),
    )
    .expect("initialize the session log");

    tracing::info!(target: "clonk-script", "Alpha riddles Beta to death.");
    tracing::info!(target: "clonk-script-trace", "call trace noise");
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

fn unique_temp_dir() -> PathBuf {
    let nonce = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("system time after epoch")
        .as_nanos();
    let directory =
        env::temp_dir().join(format!("clonk-logging-gamelog-{}-{nonce}", process::id()));
    fs::create_dir_all(&directory).expect("create test directory");
    directory
}
