use std::fs;

use clonk_core::log_target::SCRIPT_LOG_TARGET;
use clonk_logging::GameLogCapture;

mod common;

use common::unique_temp_dir;

#[test]
fn script_log_text_survives_a_level_word_in_its_own_body() {
    // `C4LogSystem::GuiSink` formats with "%*%v" (src/C4Log.cpp:185-200): the
    // level prefix from `LogLevelPrefixFormatterFlag` (src/C4Log.cpp:44-76)
    // followed by the message payload alone. The prefix comes from the record's
    // level, so message text is never scanned for one — content is free to talk
    // about errors and warnings.
    let log_path = unique_temp_dir("clonk-logging-guisink").join("Clonk.log");
    let capture = GameLogCapture::default();
    clonk_logging::init_verbose_with_file_and_capture(
        false,
        &log_path,
        None,
        Some(capture.clone()),
    )
    .expect("initialize the session log");

    tracing::info!(target: SCRIPT_LOG_TARGET, "Alpha triggers the ERROR trap and dies.");
    tracing::warn!(target: SCRIPT_LOG_TARGET, "Reactor status: WARN level reached");
    tracing::error!(target: SCRIPT_LOG_TARGET, "pipe ERROR while saving");

    assert_eq!(
        capture.take(),
        vec![
            "Alpha triggers the ERROR trap and dies.".to_string(),
            "WARNING: Reactor status: WARN level reached".to_string(),
            "ERROR: pipe ERROR while saving".to_string(),
        ]
    );

    if let Some(parent) = log_path.parent() {
        let _ = fs::remove_dir_all(parent);
    }
}
