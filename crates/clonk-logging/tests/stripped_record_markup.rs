use std::fs;

mod common;

use common::unique_temp_dir;

#[test]
fn the_session_log_strips_renderer_markup_from_every_field() {
    // Content strings keep their markup so the engine can color them where they
    // are drawn — `Parameters.ScenarioTitle` holds `Title.txt` verbatim for the
    // upper board (src/C4Game.cpp:254-256). A log file has no renderer, so the
    // tags would reach its reader as text. C++ strips in the formatter that
    // writes the payload rather than at each call site
    // (src/C4Log.cpp:103-135,302-303), which is also what keeps a message
    // nobody thought to strip — here the title quoted inside an error — clean.
    let directory = unique_temp_dir("clonk-logging-stripped");
    let log_path = directory.join("Clonk.log");
    clonk_logging::init_verbose_with_file(false, &log_path).expect("initialize the session log");

    tracing::info!(
        scenario = "Queron <c ff2c28>3.41</c>",
        "applying loaded scenario"
    );
    tracing::error!(
        error = "Failed to start Queron <c ff2c28>3.41</c>: no participating player",
        "failed to start scenario"
    );

    let logged = fs::read_to_string(&log_path).expect("read the session log");
    assert!(
        logged.contains("Queron 3.41"),
        "the title survives without its tags: {logged:?}"
    );
    assert!(
        logged.contains("Failed to start Queron 3.41: no participating player"),
        "a title quoted inside a message is stripped too: {logged:?}"
    );
    assert!(
        !logged.contains("<c ff2c28>"),
        "no markup reaches the log: {logged:?}"
    );

    let _ = fs::remove_dir_all(&directory);
}
