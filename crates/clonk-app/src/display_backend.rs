//! Winit backend selection for the Wayland/X11 split on Linux and the BSDs.
//!
//! Steam Input drives a Steam Controller through its *desktop* configuration by
//! synthesising X11 input with the XTEST extension. Under a Wayland session
//! those events stop inside Xwayland — wlroots compositors do not route XTEST
//! back out to Wayland clients — so an X11 client sees the controller and a
//! native Wayland client sees nothing at all (clonk-org/clonk-rs#238).
//!
//! Winit picks Wayland whenever `WAYLAND_DISPLAY` is set, which is exactly the
//! case that loses the input, so the choice is made here instead.

use std::ffi::OsString;
use std::path::Path;

/// What the user asked for on the command line.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, clap::ValueEnum)]
pub(crate) enum DisplayServerPreference {
    /// Winit's own order, except where Steam Input needs X11.
    #[default]
    Auto,
    /// Force X11, running under Xwayland on a Wayland session.
    X11,
    /// Force Wayland.
    Wayland,
}

/// The backend the event loop is built with.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum DisplayBackend {
    /// Leave winit's own Wayland-then-X11 order alone.
    PlatformDefault,
    X11,
    Wayland,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum DisplayBackendReason {
    PlatformDefault,
    Requested,
    SteamInputXtest,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct DisplayBackendSelection {
    pub(crate) backend: DisplayBackend,
    pub(crate) reason: DisplayBackendReason,
}

/// The two display-server variables winit reads to order its backends.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub(crate) struct DisplayServerEnvironment {
    pub(crate) wayland_display: Option<OsString>,
    pub(crate) display: Option<OsString>,
}

impl DisplayServerEnvironment {
    pub(crate) fn from_env() -> Self {
        Self {
            wayland_display: std::env::var_os("WAYLAND_DISPLAY"),
            display: std::env::var_os("DISPLAY"),
        }
    }

    fn is_wayland_session(&self) -> bool {
        is_set(self.wayland_display.as_ref())
    }

    fn has_x_display(&self) -> bool {
        is_set(self.display.as_ref())
    }
}

/// An exported-but-empty display variable is `Some("")` to `env::var_os` and
/// unusable to every display server client, winit's backend order included.
fn is_set(value: Option<&OsString>) -> bool {
    value.is_some_and(|value| !value.is_empty())
}

/// The evdev names Steam publishes while it is managing a controller: the
/// virtual pad it feeds a Steam-launched game, and the Steam Controller's own
/// nodes. Steam only creates the virtual pad while it runs, so its presence
/// under `/sys/devices/virtual/input` is the signal that Steam Input — and with
/// it the XTEST desktop mode — is live.
const STEAM_INPUT_DEVICE_NAMES: [&str; 3] =
    ["steam virtual gamepad", "x-box 360 pad", "steam controller"];

/// Apply the selection to the event loop under construction. Only the free-unix
/// backends can be chosen; every other platform builds exactly one.
pub(crate) fn apply_display_backend<T>(
    builder: &mut winit::event_loop::EventLoopBuilder<T>,
    selection: DisplayBackendSelection,
) {
    #[cfg(any(
        target_os = "linux",
        target_os = "dragonfly",
        target_os = "freebsd",
        target_os = "netbsd",
        target_os = "openbsd"
    ))]
    {
        use winit::platform::wayland::EventLoopBuilderExtWayland;
        use winit::platform::x11::EventLoopBuilderExtX11;

        match selection.backend {
            DisplayBackend::PlatformDefault => {}
            DisplayBackend::X11 => {
                builder.with_x11();
            }
            DisplayBackend::Wayland => {
                builder.with_wayland();
            }
        }
    }
    #[cfg(not(any(
        target_os = "linux",
        target_os = "dragonfly",
        target_os = "freebsd",
        target_os = "netbsd",
        target_os = "openbsd"
    )))]
    let _ = builder;
    match selection.reason {
        DisplayBackendReason::PlatformDefault => {}
        DisplayBackendReason::Requested => tracing::info!(
            backend = ?selection.backend,
            "using the requested display server backend"
        ),
        DisplayBackendReason::SteamInputXtest => tracing::info!(
            "Steam Input is running on a Wayland session; using the X11 backend so its \
             synthesised controller input reaches the window"
        ),
    }
}

pub(crate) fn steam_input_pad_present() -> bool {
    cfg!(target_os = "linux") && steam_input_pad_present_in(Path::new("/sys"))
}

/// Scan the uinput devices under `sysfs_root`. A real controller of the same
/// name hangs off its USB port instead, so restricting the walk to the virtual
/// tree keeps a natively working pad on the Wayland backend.
fn steam_input_pad_present_in(sysfs_root: &Path) -> bool {
    let Ok(entries) = std::fs::read_dir(sysfs_root.join("devices/virtual/input")) else {
        return false;
    };
    entries
        .flatten()
        .filter_map(|entry| std::fs::read_to_string(entry.path().join("name")).ok())
        .any(|name| is_steam_input_device_name(&name))
}

fn is_steam_input_device_name(name: &str) -> bool {
    let name = name.trim().to_ascii_lowercase();
    STEAM_INPUT_DEVICE_NAMES
        .iter()
        .any(|candidate| name.contains(candidate))
}

pub(crate) fn select_display_backend(
    preference: DisplayServerPreference,
    environment: &DisplayServerEnvironment,
    steam_input_pad_present: bool,
) -> DisplayBackendSelection {
    let requested = match preference {
        DisplayServerPreference::Auto => None,
        DisplayServerPreference::X11 => Some(DisplayBackend::X11),
        DisplayServerPreference::Wayland => Some(DisplayBackend::Wayland),
    };
    if let Some(backend) = requested {
        return DisplayBackendSelection {
            backend,
            reason: DisplayBackendReason::Requested,
        };
    }
    if steam_input_pad_present && environment.is_wayland_session() && environment.has_x_display() {
        return DisplayBackendSelection {
            backend: DisplayBackend::X11,
            reason: DisplayBackendReason::SteamInputXtest,
        };
    }
    DisplayBackendSelection {
        backend: DisplayBackend::PlatformDefault,
        reason: DisplayBackendReason::PlatformDefault,
    }
}

#[cfg(all(
    test,
    any(not(feature = "app-test-shard-mode"), feature = "app-test-shard-5",),
))]
mod tests {
    use std::fs;
    use std::path::Path;

    use super::*;

    fn write_virtual_input_device(root: &Path, input: &str, name: &str) {
        let directory = root.join("devices/virtual/input").join(input);
        fs::create_dir_all(&directory).expect("create the sysfs input directory");
        fs::write(directory.join("name"), format!("{name}\n")).expect("write the device name");
    }

    #[test]
    fn a_virtual_x_box_pad_is_recognised_as_steam_input() {
        // What Steam Input publishes for a Steam Controller through uinput; the
        // trailing newline is sysfs' own.
        let root = tempfile::tempdir().expect("create a temporary sysfs root");
        write_virtual_input_device(root.path(), "input75", "Microsoft X-Box 360 pad 0");
        assert!(steam_input_pad_present_in(root.path()));
    }

    #[test]
    fn an_unrelated_virtual_device_is_not_steam_input() {
        let root = tempfile::tempdir().expect("create a temporary sysfs root");
        write_virtual_input_device(root.path(), "input3", "PC Speaker");
        assert!(!steam_input_pad_present_in(root.path()));
    }

    #[test]
    fn a_sysfs_root_without_virtual_inputs_reports_no_steam_input() {
        let root = tempfile::tempdir().expect("create a temporary sysfs root");
        assert!(!steam_input_pad_present_in(root.path()));
    }

    fn wayland_session_with_xwayland() -> DisplayServerEnvironment {
        DisplayServerEnvironment {
            wayland_display: Some(OsString::from("wayland-1")),
            display: Some(OsString::from(":1")),
        }
    }

    #[test]
    fn an_explicit_wayland_preference_survives_steam_input_detection() {
        assert_eq!(
            select_display_backend(
                DisplayServerPreference::Wayland,
                &wayland_session_with_xwayland(),
                true,
            ),
            DisplayBackendSelection {
                backend: DisplayBackend::Wayland,
                reason: DisplayBackendReason::Requested,
            }
        );
    }

    #[test]
    fn an_explicit_x11_preference_needs_no_detection() {
        assert_eq!(
            select_display_backend(
                DisplayServerPreference::X11,
                &wayland_session_with_xwayland(),
                false,
            ),
            DisplayBackendSelection {
                backend: DisplayBackend::X11,
                reason: DisplayBackendReason::Requested,
            }
        );
    }

    fn platform_default() -> DisplayBackendSelection {
        DisplayBackendSelection {
            backend: DisplayBackend::PlatformDefault,
            reason: DisplayBackendReason::PlatformDefault,
        }
    }

    #[test]
    fn a_steam_input_pad_without_an_x_display_keeps_the_platform_default() {
        // Nothing to fall back to: forcing X11 here would only fail the event
        // loop build, and the Wayland window at least keeps keyboard and mouse.
        let environment = DisplayServerEnvironment {
            wayland_display: Some(OsString::from("wayland-1")),
            display: None,
        };
        assert_eq!(
            select_display_backend(DisplayServerPreference::Auto, &environment, true),
            platform_default()
        );
    }

    #[test]
    fn a_steam_input_pad_outside_a_wayland_session_keeps_the_platform_default() {
        // Winit already picks X11 when `WAYLAND_DISPLAY` is unset, and the
        // classic X session never lost the XTEST events in the first place.
        let environment = DisplayServerEnvironment {
            wayland_display: None,
            display: Some(OsString::from(":0")),
        };
        assert_eq!(
            select_display_backend(DisplayServerPreference::Auto, &environment, true),
            platform_default()
        );
    }

    #[test]
    fn an_empty_display_variable_counts_as_unset() {
        // `env::var_os` reports an exported-but-empty variable as `Some("")`,
        // which neither winit nor Xlib treats as a usable display.
        let environment = DisplayServerEnvironment {
            wayland_display: Some(OsString::from("wayland-1")),
            display: Some(OsString::new()),
        };
        assert_eq!(
            select_display_backend(DisplayServerPreference::Auto, &environment, true),
            platform_default()
        );
    }

    #[test]
    fn a_wayland_session_without_steam_input_keeps_the_platform_default() {
        assert_eq!(
            select_display_backend(
                DisplayServerPreference::Auto,
                &wayland_session_with_xwayland(),
                false,
            ),
            platform_default()
        );
    }

    #[test]
    fn steam_input_pad_on_a_wayland_session_selects_x11() {
        assert_eq!(
            select_display_backend(
                DisplayServerPreference::Auto,
                &wayland_session_with_xwayland(),
                true,
            ),
            DisplayBackendSelection {
                backend: DisplayBackend::X11,
                reason: DisplayBackendReason::SteamInputXtest,
            }
        );
    }
}
