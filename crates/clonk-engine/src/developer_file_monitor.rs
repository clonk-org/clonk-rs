//! Arming the developer file monitor, and the external reload trigger.
//!
//! `C4Game` starts a monitor only when the developer key is set, the app is
//! **not** fullscreen, and no monitor is already running
//! (`C4Game.cpp:2414`):
//!
//! ```cpp
//! if (Config.Developer.AutoFileReload && !Application.isFullScreen && !FileMonitor)
//! ```
//!
//! The external trigger is a `WM_COPYDATA` message tagged `WM_USER_RELOADFILE`
//! whose payload must be NUL-terminated (`C4Console.cpp:241-250`):
//!
//! ```cpp
//! const char *szPath = reinterpret_cast<const char *>(pcds->lpData);
//! if (szPath[pcds->cbData - 1]) break;   // last byte not NUL: ignored
//! Game.ReloadFile(szPath);
//! ```
//!
//! Path-to-definition matching lives in [`crate::developer_reload`].

/// Whether a file monitor should be started (`C4Game.cpp:2414`).
///
/// All three must hold: the `Developer.AutoFileReload` key, a windowed app, and
/// no monitor already running. A fullscreen session never watches, however the
/// key is set.
pub fn should_arm_file_monitor(
    auto_file_reload: bool,
    fullscreen: bool,
    monitor_running: bool,
) -> bool {
    auto_file_reload && !fullscreen && !monitor_running
}

/// Why an external reload payload was rejected.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ReloadPayloadError {
    /// `cbData` was zero, so `szPath[cbData - 1]` has nothing to test.
    Empty,
    /// The final byte was not NUL — C++ `break`s without reloading.
    NotNulTerminated,
    /// The bytes before the terminator are not valid text.
    NotText,
}

/// Validates a `WM_USER_RELOADFILE` payload and returns the path it names
/// (`C4Console.cpp:243-249`).
///
/// The check is on the **last byte only**: an embedded NUL earlier in the
/// buffer still passes in C++, and the path simply ends at the first one — so
/// this truncates there rather than rejecting.
pub fn reload_payload_path(payload: &[u8]) -> Result<&str, ReloadPayloadError> {
    let Some((terminator, body)) = payload.split_last() else {
        return Err(ReloadPayloadError::Empty);
    };
    if *terminator != 0 {
        return Err(ReloadPayloadError::NotNulTerminated);
    }
    // C++ treats the buffer as a C string, so it stops at the first NUL.
    let end = body
        .iter()
        .position(|byte| *byte == 0)
        .unwrap_or(body.len());
    std::str::from_utf8(&body[..end]).map_err(|_| ReloadPayloadError::NotText)
}

#[cfg(test)]
mod tests {
    use super::*;

    // C4Game.cpp:2414 — all three conditions, and fullscreen never watches.
    #[test]
    fn console_auto_file_reload_watches_unpacked_sources_and_dispatches_paths() {
        assert!(should_arm_file_monitor(true, false, false));
        // The key off, fullscreen, or an existing monitor each veto it.
        assert!(!should_arm_file_monitor(false, false, false));
        assert!(
            !should_arm_file_monitor(true, true, false),
            "a fullscreen session never watches, however the key is set"
        );
        assert!(
            !should_arm_file_monitor(true, false, true),
            "a running monitor is not replaced"
        );
    }

    // C4Console.cpp:243-249 — the payload's last byte must be NUL.
    #[test]
    fn external_reload_trigger_validates_path_and_reload_particle_is_name_based() {
        assert_eq!(
            reload_payload_path(b"Objects.c4d\\Rock.c4d\\Script.c\0"),
            Ok("Objects.c4d\\Rock.c4d\\Script.c")
        );

        // Not terminated: C++ `break`s and never reloads.
        assert_eq!(
            reload_payload_path(b"Objects.c4d\\Rock.c4d"),
            Err(ReloadPayloadError::NotNulTerminated)
        );
        // An empty payload has no last byte to test.
        assert_eq!(reload_payload_path(b""), Err(ReloadPayloadError::Empty));

        // A lone terminator is a valid, empty path — C++ would hand
        // `ReloadFile` an empty string, which matches no definition.
        assert_eq!(reload_payload_path(b"\0"), Ok(""));

        // C++ reads the buffer as a C string, so an embedded NUL ends the path
        // while the trailing-byte check still governs acceptance.
        assert_eq!(reload_payload_path(b"Script.c\0ignored\0"), Ok("Script.c"));
        assert_eq!(
            reload_payload_path(b"Script.c\0ignored"),
            Err(ReloadPayloadError::NotNulTerminated),
            "the last byte is what is tested, not the first NUL"
        );

        // Non-text bytes are rejected rather than lossily converted.
        assert_eq!(
            reload_payload_path(&[0xff, 0xfe, 0x00]),
            Err(ReloadPayloadError::NotText)
        );
    }
}
