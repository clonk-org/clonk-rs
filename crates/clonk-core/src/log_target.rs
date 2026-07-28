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

/// Target of the C4Script call-trace stream (`C4AulDebug`), which C++ writes to
/// the log but never to the message board.
pub const SCRIPT_TRACE_TARGET: &str = "clonk-script-trace";

/// Target of the C4Script profiler report, which C++ likewise keeps out of the
/// message board.
pub const SCRIPT_PROFILER_TARGET: &str = "clonk-script-profiler";
