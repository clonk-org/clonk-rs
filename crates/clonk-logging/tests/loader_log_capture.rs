//! The loading screen's log buffer: arbitrary log events from any thread
//! interleave with the loader's own phase milestones in emission order.
//!
//! `clonk-logging` sets `[lib] test = false` and has no unit-test companion, so
//! this runs as an integration test.

use clonk_logging::{
    activate_loader_log, deactivate_loader_log, loader_log_is_active, loader_log_snapshot,
    push_loader_log_line, LOADER_LOG_CAPACITY,
};

/// `C4LogSystem`'s GUI sink marshals worker-thread events to the main thread
/// and appends them to the same buffer `C4LoaderScreen` draws
/// (`src/C4Log.cpp:208-243`; `src/C4LoaderScreen.cpp:126-177`), so a worker
/// event landing between two milestones keeps its position.
#[test]
fn loader_captures_runtime_log_lines_in_gui_order() {
    // Nothing is retained before the loading screen opens.
    assert!(!loader_log_is_active());
    push_loader_log_line("dropped before the loader opened");
    assert!(loader_log_snapshot().is_empty());

    activate_loader_log();

    push_loader_log_line("Definitions");
    // A worker thread logs between two milestones.
    std::thread::spawn(|| push_loader_log_line("WARNING: material slot exhausted"))
        .join()
        .expect("worker thread");
    push_loader_log_line("Landscape");

    assert_eq!(
        loader_log_snapshot(),
        vec![
            "Definitions".to_owned(),
            "WARNING: material slot exhausted".to_owned(),
            "Landscape".to_owned(),
        ],
        "a worker event between two milestones must keep its position"
    );

    // Empty lines never reach the box, matching C4MessageBoard::AddLog
    // (src/C4MessageBoard.cpp:327-347).
    push_loader_log_line("");
    assert_eq!(loader_log_snapshot().len(), 3);

    // The buffer is bounded and drops the oldest line first.
    (0..LOADER_LOG_CAPACITY).for_each(|index| push_loader_log_line(&format!("line {index}")));
    let snapshot = loader_log_snapshot();
    assert_eq!(snapshot.len(), LOADER_LOG_CAPACITY);
    assert_eq!(snapshot.first().map(String::as_str), Some("line 0"));
    assert_eq!(
        snapshot.last().map(String::as_str),
        Some(format!("line {}", LOADER_LOG_CAPACITY - 1).as_str())
    );

    // Closing the loader releases the buffer and stops capturing.
    deactivate_loader_log();
    assert!(!loader_log_is_active());
    assert!(loader_log_snapshot().is_empty());
    push_loader_log_line("after the loader closed");
    assert!(loader_log_snapshot().is_empty());
}
