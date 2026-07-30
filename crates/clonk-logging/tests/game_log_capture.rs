use std::{
    env, fs,
    path::PathBuf,
    process,
    time::{SystemTime, UNIX_EPOCH},
};

use clonk_core::log_target::SCRIPT_LOG_TARGET;
use clonk_logging::GameLogCapture;

#[test]
fn the_message_board_drops_empty_lines_and_drains_once() {
    // `C4MessageBoard::AddLog` ignores empty messages
    // (src/C4MessageBoard.cpp:327-347), and the sink marshals each line to the
    // application thread exactly once (src/C4Log.cpp:226-240).
    let log_path = unique_temp_dir().join("Clonk.log");
    let capture = GameLogCapture::default();
    clonk_logging::init_verbose_with_file_and_capture(
        false,
        &log_path,
        None,
        Some(capture.clone()),
    )
    .expect("initialize the session log");

    tracing::info!(target: SCRIPT_LOG_TARGET, "Alpha shows Beta that he ain't bullet-proof.");
    tracing::info!(target: SCRIPT_LOG_TARGET, "");

    assert_eq!(
        capture.take(),
        vec!["Alpha shows Beta that he ain't bullet-proof.".to_string()]
    );
    assert!(
        capture.take().is_empty(),
        "a drained capture reports no further lines"
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
    let directory = env::temp_dir().join(format!("clonk-logging-board-{}-{nonce}", process::id()));
    fs::create_dir_all(&directory).expect("create test directory");
    directory
}
