//! `-[NSApplication terminate:]` — Cmd+Q, the Dock's Quit item, and log-out.
//!
//! `C4Application::Quit` reaches `C4Game::Clear` → `Network.Clear()` →
//! `LeagueEnd(); DeinitLeague();` (`C4Game.cpp:581`; `C4Network2.cpp:746-763`)
//! for *every* way the loop unwinds, so a native host always de-registers. SDL
//! makes that hold on macOS by cancelling AppKit's terminate and re-posting it
//! as `SDL_QUIT` (`StdAppUnix.cpp:809-815`).
//!
//! winit does not: it implements only `applicationWillTerminate:`
//! (`winit-0.30.13/src/platform_impl/macos/app_state.rs:69-72`), never
//! `applicationShouldTerminate:`, so the app first hears about this quit from
//! inside AppKit's own terminate — where `Event::LoopExiting` is dispatched and
//! where joining worker threads is exactly what must not happen. `run_app`
//! never returns either, so neither of the port's two de-registration routes
//! runs and the game stays registered until the league server times it out.
//!
//! The fix mirrors `CStdApp::Quit`'s `fQuitMsgReceived` latch
//! (`StdAppUnix.cpp:256-259`): answer `applicationShouldTerminate:` with
//! `NSTerminateCancel`, record that a quit was asked for, and let the ordinary
//! quit path run on the next loop turn — the same path Cmd+Q through the port's
//! own menu already takes. Nothing blocks inside the AppKit callback.

use std::sync::atomic::{AtomicBool, Ordering};

/// Set from `applicationShouldTerminate:`, consumed on the next loop turn.
static TERMINATE_REQUESTED: AtomicBool = AtomicBool::new(false);

/// Whether AppKit asked to terminate since the last call.
///
/// Consuming clears it, so a single terminate produces a single quit even if
/// the user keeps pressing Cmd+Q while the league `End` is in flight.
pub(crate) fn take_terminate_request() -> bool {
    TERMINATE_REQUESTED.swap(false, Ordering::SeqCst)
}

/// Records an AppKit terminate. Exposed for the routing tests, which cannot
/// raise a real one.
pub(crate) fn note_terminate_request() {
    TERMINATE_REQUESTED.store(true, Ordering::SeqCst);
}

/// Installs the `applicationShouldTerminate:` answer on the running app.
///
/// Must run after winit has installed its own delegate
/// (`winit-0.30.13/src/platform_impl/macos/event_loop.rs:240`), which is why
/// the caller ties this to the same first-`Resumed` moment the Dock tile uses.
#[cfg(target_os = "macos")]
pub(crate) fn install_terminate_handler() {
    macos::install();
}

#[cfg(not(target_os = "macos"))]
pub(crate) fn install_terminate_handler() {}

#[cfg(target_os = "macos")]
mod macos {
    use std::sync::atomic::{AtomicBool, Ordering};

    use objc2::runtime::{AnyClass, AnyObject, Sel};
    use objc2::{msg_send, sel};

    static INSTALLED: AtomicBool = AtomicBool::new(false);

    /// `NSTerminateCancel` (`NSApplication.h`): stop the termination, leaving
    /// the app running so its own quit path can unwind normally.
    const NS_TERMINATE_CANCEL: usize = 0;

    /// `@encode` for `NSUInteger (*)(id, SEL, id)` — the signature of
    /// `-[NSObject applicationShouldTerminate:]`.
    const TYPES: &[u8] = b"L@:@\0";

    extern "C-unwind" fn should_terminate(
        _this: &AnyObject,
        _cmd: Sel,
        _sender: *mut AnyObject,
    ) -> usize {
        super::note_terminate_request();
        NS_TERMINATE_CANCEL
    }

    pub(super) fn install() {
        // AppKit is main-thread only, and off it there is no delegate to reach.
        let Some(main_thread) = objc2::MainThreadMarker::new() else {
            return;
        };
        if INSTALLED.swap(true, Ordering::SeqCst) {
            return;
        }
        let _ = main_thread;

        // SAFETY: every call below is a main-thread AppKit/runtime call, and
        // the method is *added* rather than replaced: winit's delegate class
        // does not implement `applicationShouldTerminate:`
        // (`app_state.rs:69-72` implements only `applicationWillTerminate:`),
        // so nothing is overridden and no existing behaviour is displaced. If
        // winit ever implements it, `class_addMethod` returns NO and leaves
        // winit's in place, which is the safe direction.
        unsafe {
            let application: *mut AnyObject =
                msg_send![objc2::class!(NSApplication), sharedApplication];
            if application.is_null() {
                return;
            }
            let delegate: *mut AnyObject = msg_send![application, delegate];
            if delegate.is_null() {
                // No delegate yet: this ran before winit installed one, and
                // the caller retries on a later turn.
                INSTALLED.store(false, Ordering::SeqCst);
                return;
            }
            let class: *const AnyClass = msg_send![delegate, class];
            objc2::ffi::class_addMethod(
                class as *mut AnyClass,
                sel!(applicationShouldTerminate:),
                std::mem::transmute::<
                    extern "C-unwind" fn(&AnyObject, Sel, *mut AnyObject) -> usize,
                    objc2::runtime::Imp,
                >(should_terminate),
                TYPES.as_ptr().cast(),
            );
        }
    }
}

#[cfg(all(
    test,
    any(not(feature = "app-test-shard-mode"), feature = "app-test-shard-5",),
))]
mod tests {
    use super::*;

    /// The latch is consume-once, so holding Cmd+Q down cannot queue several
    /// quits behind a league `End` that is still in flight.
    #[test]
    fn a_terminate_request_is_consumed_exactly_once() {
        // Leave the global as found: other tests in this binary share it.
        let _ = take_terminate_request();

        assert!(!take_terminate_request(), "nothing pending to begin with");
        note_terminate_request();
        note_terminate_request();
        assert!(take_terminate_request(), "the request is observed");
        assert!(!take_terminate_request(), "and only observed once");
    }
}
