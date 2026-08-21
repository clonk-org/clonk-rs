//! Arbitrary tracing output reaches the loading screen while it is up
//! (clonk-org/clonk-rs#570).
//!
//! `loader_log_capture.rs` covers the buffer itself — push, snapshot, capacity,
//! and that deactivating clears it. What it does not cover is the half the
//! loading screen actually depends on: that a *real* `tracing` event, emitted
//! from anywhere in the process, lands in that buffer without anyone calling
//! `push_loader_log_line`. That is the difference between a loader showing its
//! own coarse phase milestones and a loader showing what is really happening.
//!
//! The sink is deliberately filtered rather than taking everything
//! (`loader_log_sink`): it takes the same events the message board does, so the
//! loader shows what C++ shows and no more. Both halves are asserted here,
//! because a capture that took *every* target would spill Rust-internal
//! diagnostics with no C++ `Log()` counterpart onto the player's loading
//! screen.

use clonk_core::log_target::SCRIPT_LOG_TARGET;
use clonk_logging::{
    activate_loader_log, deactivate_loader_log, loader_log_is_active, loader_log_snapshot,
};

mod common;

use common::unique_temp_dir;

#[test]
fn a_live_tracing_event_reaches_the_loader_without_an_explicit_push() {
    let log_path = unique_temp_dir("clonk-logging-loader-tracing").join("Clonk.log");
    clonk_logging::init_verbose_with_file(false, &log_path).expect("initialize the session log");

    // Nothing is retained before the loading screen opens its buffer.
    assert!(!loader_log_is_active());
    tracing::info!(target: SCRIPT_LOG_TARGET, "before the loader opened");
    assert!(loader_log_snapshot().is_empty());

    activate_loader_log();
    tracing::info!(target: SCRIPT_LOG_TARGET, "loading Castle.c4s");
    tracing::warn!(target: SCRIPT_LOG_TARGET, "missing definition CLNK");

    let captured = loader_log_snapshot();
    assert_eq!(
        captured,
        vec![
            "loading Castle.c4s".to_string(),
            // The GUI sinks carry the level prefix, so a warning is
            // distinguishable on the loading screen exactly as it is on the
            // message board.
            "WARNING: missing definition CLNK".to_string(),
        ],
        "an ordinary tracing event must reach the loader with no explicit push"
    );

    // An engine-internal target has no C++ `Log()` counterpart and must not
    // reach the player's loading screen.
    tracing::info!(target: "clonk_engine::internal", "landscape chunk 42 rebuilt");
    assert_eq!(
        loader_log_snapshot(),
        captured,
        "a target the message board would not show must not reach the loader either"
    );

    // Closing the loading screen stops retention, so a later round's events do
    // not accumulate behind it.
    deactivate_loader_log();
    tracing::info!(target: SCRIPT_LOG_TARGET, "after the loader closed");
    assert!(loader_log_snapshot().is_empty());

    if let Some(parent) = log_path.parent() {
        let _ = std::fs::remove_dir_all(parent);
    }
}
