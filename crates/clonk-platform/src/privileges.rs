//! Effective-UID startup guard.
//!
//! `C4WinMain.cpp:251-255` refuses to run as root before the debug facilities
//! and application initialization:
//!
//! ```c
//! if (!geteuid())
//! {
//!     printf("Do not run %s as root!\n", argc ? argv[0] : "this program");
//!     return C4XRV_Failure;
//! }
//! ```

/// `C4XRV_Failure` (`C4Constants.h:100`).
pub const STARTUP_FAILURE_EXIT_CODE: i32 = 1;

/// The refusal for `effective_uid`, or `None` when startup may proceed.
/// `argv0` is `argv[0]`; C++ falls back to `"this program"` when `argc` is zero
/// (`C4WinMain.cpp:253`).
pub fn root_startup_refusal(effective_uid: u32, argv0: Option<&str>) -> Option<String> {
    (effective_uid == 0).then(|| {
        let program = argv0.unwrap_or("this program");
        format!("Do not run {program} as root!")
    })
}

/// The live guard `main` calls: reads the real effective UID and applies
/// [`root_startup_refusal`]. Non-Unix targets never refuse.
#[cfg(unix)]
pub fn root_startup_refusal_for_current_process(argv0: Option<&str>) -> Option<String> {
    extern "C" {
        fn geteuid() -> u32;
    }
    // SAFETY: `geteuid` is always available and cannot fail.
    root_startup_refusal(unsafe { geteuid() }, argv0)
}

#[cfg(test)]
mod tests {
    use super::*;

    // C4WinMain.cpp:251-255 — the guard runs before the crash handlers and
    // application initialization, so it is the first thing `main` consults.
    // A privileged child cannot be spawned from an unprivileged test run, so
    // the effective UID is supplied directly here; `main` feeds it the real
    // `geteuid()` through `root_startup_refusal_for_current_process`.
    #[test]
    fn unix_effective_root_is_rejected_before_bootstrap() {
        assert_eq!(
            root_startup_refusal(0, Some("/usr/local/bin/clonk")).as_deref(),
            Some("Do not run /usr/local/bin/clonk as root!")
        );
        // :253 — the `argc ? argv[0] : "this program"` fallback.
        assert_eq!(
            root_startup_refusal(0, None).as_deref(),
            Some("Do not run this program as root!")
        );
        // Any non-zero effective UID proceeds untouched.
        assert_eq!(root_startup_refusal(1, Some("clonk")), None);
        assert_eq!(root_startup_refusal(501, Some("clonk")), None);
        // The live reader agrees for the (unprivileged) test process.
        #[cfg(unix)]
        assert_eq!(
            root_startup_refusal_for_current_process(Some("clonk")),
            None
        );
    }
}
