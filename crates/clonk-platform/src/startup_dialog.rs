//! Native reporting for failures that happen before a window exists.
//!
//! `C4WinMain.cpp:97-117` shows `MessageBox*(nullptr, message, STD_PRODUCT,
//! MB_ICONERROR)` for a COM or `CStdApp::StartupException` failure and returns
//! `C4XRV_Failure`. The Unix entry point writes the same message to stderr and,
//! in developer builds, opens a GTK error dialog (`C4WinMain.cpp:274-289`).
//!
//! Both paths keep the diagnostic on stderr and still fail; the dialog is an
//! addition, never a replacement. The sink is injected so a headless run — and
//! any platform without a dialog backend — stays deterministic.

use crate::paths::ENGINE_CAPTION;

/// What a startup failure reports. `caption` is `STD_PRODUCT`, which this port
/// already uses as its window title.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct StartupFailureDialog {
    pub caption: &'static str,
    pub message: String,
}

impl StartupFailureDialog {
    /// The dialog C++ would show for `message` (`C4WinMain.cpp:103,111`).
    pub fn new(message: impl Into<String>) -> Self {
        Self {
            caption: ENGINE_CAPTION,
            message: message.into(),
        }
    }
}

/// A platform backend. `present` reports whether a dialog was actually shown,
/// so a caller can tell "reported" from "silently unavailable" without either
/// outcome changing the exit status.
pub trait StartupDialogSink {
    fn present(&mut self, dialog: &StartupFailureDialog) -> bool;
}

/// The backend for platforms with no dialog available — and for headless runs.
/// C++'s Unix path without `WITH_DEVELOPER_MODE` behaves the same: stderr only.
#[derive(Clone, Copy, Debug, Default)]
pub struct NoStartupDialog;

impl StartupDialogSink for NoStartupDialog {
    fn present(&mut self, _dialog: &StartupFailureDialog) -> bool {
        false
    }
}

/// Set once the application window exists. C++ only shows these dialogs for
/// failures raised before that point (`C4WinMain.cpp:97-117`); afterwards the
/// running game reports its own errors.
static WINDOW_CREATED: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);

/// Records that the application window now exists.
pub fn note_window_created() {
    WINDOW_CREATED.store(true, std::sync::atomic::Ordering::SeqCst);
}

/// Whether the application window has been created.
pub fn window_was_created() -> bool {
    WINDOW_CREATED.load(std::sync::atomic::Ordering::SeqCst)
}

/// Whether a startup failure should try to open a dialog at all.
///
/// A headless run must not block on an acknowledgement, which is the
/// deterministic fallback the graphical path cannot assume.
pub fn should_present_startup_dialog(headless: bool) -> bool {
    !headless
}

/// Reports `message` natively when appropriate. The caller keeps its own stderr
/// and log output and its failing exit status either way — this only adds the
/// dialog, matching C++ printing *and* showing the same text.
///
/// Returns whether a dialog was shown.
pub fn report_startup_failure<S: StartupDialogSink>(
    sink: &mut S,
    headless: bool,
    message: &str,
) -> bool {
    if !should_present_startup_dialog(headless) {
        return false;
    }
    sink.present(&StartupFailureDialog::new(message))
}

#[cfg(windows)]
pub use windows_impl::NativeStartupDialog;

#[cfg(windows)]
mod windows_impl {
    use super::{StartupDialogSink, StartupFailureDialog};
    use windows_sys::Win32::UI::WindowsAndMessaging::{MessageBoxA, MB_ICONERROR, MB_OK};

    /// `MessageBoxA(nullptr, message, STD_PRODUCT, MB_ICONERROR)`
    /// (`C4WinMain.cpp:111`). `MB_OK` is the default button set, so the
    /// acknowledgement matches too.
    #[derive(Clone, Copy, Debug, Default)]
    pub struct NativeStartupDialog;

    impl StartupDialogSink for NativeStartupDialog {
        fn present(&mut self, dialog: &StartupFailureDialog) -> bool {
            let (Ok(message), Ok(caption)) = (
                std::ffi::CString::new(dialog.message.as_str()),
                std::ffi::CString::new(dialog.caption),
            ) else {
                // An interior NUL cannot be shown; the caller still logs and
                // fails, which is the point of the boolean.
                return false;
            };
            // SAFETY: both strings outlive the call, which owns no memory.
            unsafe {
                MessageBoxA(
                    0,
                    message.as_ptr().cast(),
                    caption.as_ptr().cast(),
                    MB_ICONERROR | MB_OK,
                )
            };
            true
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[derive(Debug, Default)]
    struct RecordingSink(Vec<StartupFailureDialog>);

    impl StartupDialogSink for RecordingSink {
        fn present(&mut self, dialog: &StartupFailureDialog) -> bool {
            self.0.push(dialog.clone());
            true
        }
    }

    // C4WinMain.cpp:97-117,274-289 — a pre-window failure is shown natively
    // under the product caption, and a headless run stays silent instead of
    // blocking on an acknowledgement.
    #[test]
    fn startup_failure_uses_native_error_dialog_before_window_exists() {
        let mut sink = RecordingSink::default();
        assert!(report_startup_failure(
            &mut sink,
            false,
            "failed to initialize COM: access denied"
        ));
        assert_eq!(
            sink.0,
            vec![StartupFailureDialog {
                // STD_PRODUCT, which this port already uses as its window title.
                caption: "LegacyClonk",
                message: "failed to initialize COM: access denied".to_owned(),
            }]
        );

        // Headless: nothing is presented and nothing blocks.
        let mut headless = RecordingSink::default();
        assert!(!report_startup_failure(&mut headless, true, "no display"));
        assert!(headless.0.is_empty());
        assert!(!should_present_startup_dialog(true));
        assert!(should_present_startup_dialog(false));

        // A platform with no backend reports "not shown" rather than failing;
        // the caller's stderr output and exit status are unaffected.
        assert!(!report_startup_failure(
            &mut NoStartupDialog,
            false,
            "no dialog backend"
        ));
    }
}
