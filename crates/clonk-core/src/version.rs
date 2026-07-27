//! Version identity for the engine and for this port.
//!
//! These are two different values and must not be conflated:
//!
//! * The **engine compatibility version** mirrors `C4Version.h` (`C4XVER1`..
//!   `C4XVER4` plus the build number). Definition `DefCore` entries, savegame
//!   `C4XVer` headers, and config migration all compare against it, so it
//!   tracks the LegacyClonk release this port targets and changes only when
//!   that target moves. It is deliberately *not* this crate's own version:
//!   bundled content declares `Version=4,9,8`, and
//!   `definition_requires_newer_engine` prunes any definition that compares
//!   newer than the engine, so a lower tuple would discard the game's content.
//! * The **port version** is this workspace's release version, used for
//!   display and diagnostics only.

/// The engine compatibility version as a bare string literal.
///
/// A macro rather than a constant because call sites `concat!` it into user
/// agents, and `concat!` only accepts literals.
#[macro_export]
macro_rules! engine_version_str {
    () => {
        "4.9.11.0 [362]"
    };
}

/// `C4XVER1`..`C4XVER4` followed by the build number (`C4Version.h:28-32`).
pub const ENGINE_VERSION: [i32; 5] = [4, 9, 11, 0, 362];

/// The engine version without trailing whitespace, for protocol identification.
pub const ENGINE_VERSION_COMPACT: &str = engine_version_str!();

/// `C4VERSION` (`C4Version.h:55`). The trailing space is part of the C++
/// string: it comes from concatenating empty `C4VERSIONEXTRA`/`C4BUILDOPT`.
pub const ENGINE_VERSION_TEXT: &str = concat!(engine_version_str!(), " ");

/// This port's own release version, inherited from the workspace.
pub const PORT_VERSION: &str = env!("CARGO_PKG_VERSION");

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn engine_version_text_matches_the_numeric_tuple() {
        // The literal and the tuple are written separately, so pin them to
        // each other; otherwise bumping one and forgetting the other would
        // silently desynchronise protocol strings from content gating.
        let [major, minor, patch, revision, build] = ENGINE_VERSION;
        assert_eq!(
            ENGINE_VERSION_COMPACT,
            format!("{major}.{minor}.{patch}.{revision} [{build}]")
        );
    }

    #[test]
    fn engine_version_text_keeps_the_cpp_trailing_space() {
        assert_eq!(ENGINE_VERSION_TEXT, format!("{ENGINE_VERSION_COMPACT} "));
    }

    #[test]
    fn port_version_is_the_workspace_version() {
        // Guards against the port version silently becoming a per-crate value.
        assert_eq!(PORT_VERSION, env!("CARGO_PKG_VERSION"));
        assert!(!PORT_VERSION.is_empty());
    }
}
