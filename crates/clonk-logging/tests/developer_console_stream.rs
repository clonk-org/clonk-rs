use std::{
    env, fs,
    path::PathBuf,
    process,
    time::{SystemTime, UNIX_EPOCH},
};

use clonk_core::log_target::SCRIPT_LOG_TARGET;
use clonk_logging::{ConsoleLogCapture, GameLogCapture};

#[test]
fn the_developer_console_sees_engine_lines_the_message_board_does_not() {
    // C++ attaches the GuiSink to the C4Script logger alone, while the console
    // shows the whole log stream (src/C4Log.cpp:226-240). Both render with the
    // sink's `%*%v` pattern, so neither carries a timestamp or a level token.
    let log_path = unique_temp_dir().join("Clonk.log");
    let console = ConsoleLogCapture::default();
    let board = GameLogCapture::default();
    clonk_logging::init_verbose_with_file_and_capture(
        false,
        &log_path,
        Some(console.clone()),
        Some(board.clone()),
    )
    .expect("initialize the session log");

    tracing::info!(target: SCRIPT_LOG_TARGET, "Alpha is dead.");
    tracing::warn!("engine could not open the scenario");

    assert_eq!(
        console.take(),
        "Alpha is dead.\nWARNING: engine could not open the scenario\n"
    );
    assert_eq!(board.take(), vec!["Alpha is dead.".to_string()]);

    let session_log = fs::read_to_string(&log_path).expect("read the session log");
    assert!(
        session_log.contains("engine could not open the scenario"),
        "the file sink still receives every logged line"
    );
    // The session log is read by a developer, who needs to know which
    // subsystem a line came from — and which target to name in `LC_LOG`.
    assert!(
        session_log.contains(SCRIPT_LOG_TARGET),
        "the session log records the event target: {session_log}"
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
        env::temp_dir().join(format!("clonk-logging-console-{}-{nonce}", process::id()));
    fs::create_dir_all(&directory).expect("create test directory");
    directory
}
