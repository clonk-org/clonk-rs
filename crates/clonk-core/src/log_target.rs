//! Tracing targets that route engine output to a specific C++ log sink.
//!
//! `C4LogSystem` attaches its `GuiSink` to the loggers whose output C++ shows
//! in-game (`src/C4Log.cpp:226-240`). Rust models that attachment as a tracing
//! target, so the names below are a contract between the engine crates that
//! emit and `clonk-logging`, which routes. They live here because both sides
//! already depend on `clonk-core`; a literal on either side would let a rename
//! disconnect the message board without a compile error.

/// Target of the C4Script `Log()`/`DebugLog()` stream, whose lines
/// `C4MessageBoard::AddLog` shows in-game.
pub const SCRIPT_LOG_TARGET: &str = "clonk-script";

/// Target of the C4Script `DebugLog()` stream. It is kept apart from
/// [`SCRIPT_LOG_TARGET`] because the two are routed differently: `DebugLog`
/// diagnostics always persist to the session log, but only reach the in-game
/// message board and developer console while the round has debug mode enabled
/// (`src/C4Game.cpp:447-454,640-652`).
pub const SCRIPT_DEBUG_LOG_TARGET: &str = "clonk-script-debug";

/// Whether debug-mode presentation is currently enabled for the running round.
/// The engine publishes it from every site that mutates `debug_mode`;
/// `clonk-logging` reads it when deciding whether `DebugLog` reaches a GUI sink.
/// It lives here because both crates already depend on this one.
static DEBUG_MODE_PRESENTATION: std::sync::atomic::AtomicBool =
    std::sync::atomic::AtomicBool::new(false);

/// Publishes the round's debug mode. Off before round setup, matching C++'s
/// initially disabled GUI sink (`src/C4Log.cpp:327-388`).
pub fn set_debug_mode_presentation(enabled: bool) {
    DEBUG_MODE_PRESENTATION.store(enabled, std::sync::atomic::Ordering::Relaxed);
}

/// Whether `DebugLog` output currently reaches the GUI sinks.
pub fn debug_mode_presentation() -> bool {
    DEBUG_MODE_PRESENTATION.load(std::sync::atomic::Ordering::Relaxed)
}

/// Target of the C4Script call-trace stream (`C4AulDebug`), which C++ writes to
/// the log but never to the message board.
pub const SCRIPT_TRACE_TARGET: &str = "clonk-script-trace";

/// Target of the C4Script profiler report, which C++ likewise keeps out of the
/// message board.
pub const SCRIPT_PROFILER_TARGET: &str = "clonk-script-profiler";
