//! macOS capture authorization, requested only by the microphone worker.

use crate::{VoiceCaptureControl, VoiceCaptureError};
use std::sync::mpsc::{RecvTimeoutError, Sender};
use std::time::Duration;

enum AuthorizationStatus {
    NotDetermined,
    Authorized,
    Denied,
}

/// The running process's main bundle, as macOS's privacy check sees it.
enum MainBundle {
    /// A bare executable, such as a terminal run: macOS attributes its
    /// requests to the app that launched it.
    Unbundled,
    App {
        declares_microphone_use: bool,
    },
}

/// macOS terminates an app that asks for the microphone without
/// `NSMicrophoneUsageDescription` rather than denying it. The in-app updater
/// keeps an install's original `Info.plist`, so one installed before the key
/// shipped never gains it.
fn require_declared_microphone_use(bundle: MainBundle) -> Result<(), VoiceCaptureError> {
    match bundle {
        MainBundle::App {
            declares_microphone_use: false,
        } => Err(VoiceCaptureError::UndeclaredMicrophoneUse),
        MainBundle::App { .. } | MainBundle::Unbundled => Ok(()),
    }
}

fn permission_denied() -> VoiceCaptureError {
    VoiceCaptureError::PermissionDenied(
        "allow microphone access in System Settings > Privacy & Security > Microphone".into(),
    )
}

fn authorize(
    control: &VoiceCaptureControl,
    status: AuthorizationStatus,
    request: impl FnOnce(Sender<bool>),
) -> Result<(), VoiceCaptureError> {
    if !control.is_recording() {
        return Err(VoiceCaptureError::Cancelled);
    }
    match status {
        AuthorizationStatus::Authorized => return Ok(()),
        AuthorizationStatus::Denied => return Err(permission_denied()),
        AuthorizationStatus::NotDetermined => {}
    }
    let (reply, response) = std::sync::mpsc::channel();
    request(reply);
    loop {
        // Only the device worker waits. Dropping capture or releasing PTT must
        // stop it even when the user leaves the system dialog unanswered.
        let result = response.recv_timeout(Duration::from_millis(20));
        if !control.is_recording() {
            return Err(VoiceCaptureError::Cancelled);
        }
        match result {
            Ok(true) => return Ok(()),
            Ok(false) => return Err(permission_denied()),
            Err(RecvTimeoutError::Timeout) => {}
            Err(RecvTimeoutError::Disconnected) => {
                return Err(VoiceCaptureError::Stream(
                    "microphone authorization completed without a response".into(),
                ));
            }
        }
    }
}

#[cfg(all(target_os = "macos", feature = "cpal"))]
fn main_bundle() -> MainBundle {
    use objc2_foundation::{ns_string, NSBundle, NSString};

    let bundle = NSBundle::mainBundle();
    if !bundle
        .bundlePath()
        .pathExtension()
        .to_string()
        .eq_ignore_ascii_case("app")
    {
        return MainBundle::Unbundled;
    }
    let declares_microphone_use = bundle
        .objectForInfoDictionaryKey(ns_string!("NSMicrophoneUsageDescription"))
        .and_then(|value| value.downcast::<NSString>().ok())
        .is_some_and(|description| !description.to_string().trim().is_empty());
    MainBundle::App {
        declares_microphone_use,
    }
}

#[cfg(all(target_os = "macos", feature = "cpal"))]
pub(crate) fn request_microphone_access(
    control: &VoiceCaptureControl,
) -> Result<(), VoiceCaptureError> {
    use objc2_av_foundation::{AVAuthorizationStatus, AVCaptureDevice, AVMediaTypeAudio};

    // Before anything reaches AVFoundation or the device: asking is what
    // macOS punishes (clonk-org/clonk-rs#1849).
    require_declared_microphone_use(main_bundle())?;

    // SAFETY: the framework owns this constant for the process lifetime.
    let media_type = unsafe { AVMediaTypeAudio }.ok_or_else(|| {
        VoiceCaptureError::Stream("macOS audio authorization is unavailable".into())
    })?;
    // SAFETY: this is a supported media type, and these class methods may run
    // on the capture worker.
    let status = unsafe { AVCaptureDevice::authorizationStatusForMediaType(media_type) };
    let status = match status {
        AVAuthorizationStatus::NotDetermined => AuthorizationStatus::NotDetermined,
        AVAuthorizationStatus::Authorized => AuthorizationStatus::Authorized,
        // Restricted and unknown statuses cannot authorize capture either.
        _ => AuthorizationStatus::Denied,
    };
    authorize(control, status, |reply| {
        let completion = crate::voice_permission_block::completion(reply);
        // SAFETY: the block owns its sender and has the framework's BOOL ABI.
        // AVFoundation copies the block for its asynchronous completion.
        unsafe {
            AVCaptureDevice::requestAccessForMediaType_completionHandler(media_type, &completion);
        }
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::VoiceCaptureControl;

    /// clonk-org/clonk-rs#1849: macOS ends an app that asks for the microphone
    /// without `NSMicrophoneUsageDescription`, and an install updated in place
    /// from before v0.11.0 still has such an `Info.plist`.
    #[test]
    fn an_app_that_does_not_declare_microphone_use_never_asks_macos() {
        let result = require_declared_microphone_use(MainBundle::App {
            declares_microphone_use: false,
        });
        assert!(matches!(
            result,
            Err(VoiceCaptureError::UndeclaredMicrophoneUse)
        ));
    }

    #[test]
    fn a_declaring_app_or_a_bare_executable_may_ask_macos() {
        for bundle in [
            MainBundle::Unbundled,
            MainBundle::App {
                declares_microphone_use: true,
            },
        ] {
            assert!(require_declared_microphone_use(bundle).is_ok());
        }
    }

    /// A terminal or `cargo` run is a bare executable whose requests macOS
    /// attributes to the terminal, so the guard must not refuse it.
    #[cfg(all(target_os = "macos", feature = "cpal"))]
    #[test]
    fn a_bare_executable_is_not_mistaken_for_an_app_bundle() {
        assert!(matches!(main_bundle(), MainBundle::Unbundled));
    }

    #[test]
    fn undetermined_microphone_access_requests_authorization() {
        let mut requested = false;
        authorize(
            &VoiceCaptureControl::default(),
            AuthorizationStatus::NotDetermined,
            |reply| {
                requested = true;
                reply.send(true).unwrap();
            },
        )
        .unwrap();
        assert!(requested);
    }

    #[test]
    fn decided_microphone_permission_never_prompts_again() {
        for (status, granted) in [
            (AuthorizationStatus::Authorized, true),
            (AuthorizationStatus::Denied, false),
        ] {
            let result = authorize(&VoiceCaptureControl::default(), status, |_| {
                panic!("a decided permission must not prompt again");
            });
            if granted {
                result.unwrap();
            } else {
                assert!(matches!(
                    result,
                    Err(VoiceCaptureError::PermissionDenied(_))
                ));
            }
        }
    }

    #[test]
    fn denied_prompt_reports_permission_denied_instead_of_silent_capture() {
        let result = authorize(
            &VoiceCaptureControl::default(),
            AuthorizationStatus::NotDetermined,
            |reply| reply.send(false).unwrap(),
        );
        assert!(matches!(
            result,
            Err(VoiceCaptureError::PermissionDenied(_))
        ));
    }

    #[test]
    fn cancelled_capture_never_requests_permission() {
        let control = VoiceCaptureControl::default();
        control.abort();
        let result = authorize(&control, AuthorizationStatus::NotDetermined, |_| {
            panic!("cancelled capture must not show a prompt");
        });
        assert!(matches!(result, Err(VoiceCaptureError::Cancelled)));
    }

    #[test]
    fn late_permission_grant_cannot_revive_released_capture() {
        let control = VoiceCaptureControl::default();
        let result = authorize(&control, AuthorizationStatus::NotDetermined, |reply| {
            control.finish_at(std::time::Instant::now());
            reply.send(true).unwrap();
        });
        assert!(matches!(result, Err(VoiceCaptureError::Cancelled)));
    }

    #[test]
    fn cancelling_capture_does_not_wait_for_the_system_prompt() {
        let control = VoiceCaptureControl::default();
        let worker_control = control.clone();
        let (prompt, prompted) = std::sync::mpsc::channel();
        let (done, finished) = std::sync::mpsc::channel();
        let worker = std::thread::spawn(move || {
            let result = authorize(
                &worker_control,
                AuthorizationStatus::NotDetermined,
                |reply| prompt.send(reply).unwrap(),
            );
            done.send(result).unwrap();
        });
        let reply = prompted
            .recv_timeout(std::time::Duration::from_secs(1))
            .unwrap();
        control.abort();
        let result = finished.recv_timeout(std::time::Duration::from_secs(1));
        // Also release the worker when testing a broken, uncancellable wait.
        let _ = reply.send(true);
        worker.join().unwrap();
        assert!(matches!(result, Ok(Err(VoiceCaptureError::Cancelled))));
    }
}
