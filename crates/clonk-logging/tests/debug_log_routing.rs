//! `DebugLog` routing: the session log always keeps it, the GUI sinks only
//! while the round has debug mode enabled.
//!
//! `clonk-logging` sets `[lib] test = false` and has no unit-test companion, so
//! this runs as an integration test.

use clonk_core::log_target::set_debug_mode_presentation;
use clonk_logging::{
    debug_log_reaches_gui, debug_log_reaches_stderr, SCRIPT_DEBUG_LOG_TARGET, SCRIPT_LOG_TARGET,
};

/// `Log()` output is unconditional; `DebugLog()` output follows the round. The
/// session log keeps both regardless, which is why only the GUI and stderr
/// routes are decided here.
#[test]
fn debug_log_file_and_gui_routes_follow_runtime_debug_mode() {
    // Before round setup C++ starts with its GUI debug sink off
    // (C4Log.cpp:327-388), so nothing debug-only is presented.
    set_debug_mode_presentation(false);
    assert!(!debug_log_reaches_gui(SCRIPT_DEBUG_LOG_TARGET));
    assert!(debug_log_reaches_gui(SCRIPT_LOG_TARGET));

    // Round initialization with DebugMode on (C4Game.cpp:447-454).
    set_debug_mode_presentation(true);
    assert!(debug_log_reaches_gui(SCRIPT_DEBUG_LOG_TARGET));
    assert!(debug_log_reaches_gui(SCRIPT_LOG_TARGET));

    // Ctrl+F5 / synchronized DisableDebug toggles it back off.
    set_debug_mode_presentation(false);
    assert!(!debug_log_reaches_gui(SCRIPT_DEBUG_LOG_TARGET));

    // Game clear leaves it off (C4Game.cpp:640-652).
    set_debug_mode_presentation(true);
    set_debug_mode_presentation(false);
    assert!(!debug_log_reaches_gui(SCRIPT_DEBUG_LOG_TARGET));

    // Engine-internal targets never reach a GUI sink either way.
    set_debug_mode_presentation(true);
    assert!(!debug_log_reaches_gui("clonk-engine"));
    set_debug_mode_presentation(false);

    // The stderr route follows the operator's verbosity, not debug mode; the
    // session log keeps the diagnostics at every level.
    assert!(!debug_log_reaches_stderr("info"));
    assert!(debug_log_reaches_stderr("debug"));
    assert!(debug_log_reaches_stderr("trace"));
}
