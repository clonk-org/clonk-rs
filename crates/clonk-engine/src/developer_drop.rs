//! Dropping a definition file onto a console viewport.
//!
//! `C4Viewport::DropFiles` (`C4Viewport.cpp:225-240`) is the only way the
//! editor creates an object without typing script: the Win32 window accepts
//! `WM_DROPFILES`, converts the drop point through that viewport's `ViewX`/
//! `ViewY`, and hands each path to `C4Game::DropFile` (`C4Game.cpp:1641-1660`),
//! which resolves a definition and enqueues `CID_EMDropDef` through
//! `C4Game::DropDef` (`:1662-1676`).
//!
//! Both the window handler and `WM_USER_DROPDEF` beside it are `_WIN32`-only,
//! so on the reference build nothing reaches `DropDef` at all — its executor
//! and wire codec are live, and no caller exists. What *is* platform-neutral
//! is the decision this module keeps: which files are considered, in what
//! order a definition is resolved, and which of the two failure texts is
//! shown.

use std::path::Path;

use crate::DefinitionId;

/// The extension `DropFile` tests for, case-insensitively (`SEqualNoCase`).
pub const DEFINITION_FILE_EXTENSION: &str = "c4d";

/// What a drop should do.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum DropOutcome {
    /// `Console.Editing` is clear: `DropFiles` reports `IDS_CNS_NONETEDIT` and
    /// drops **every** file in the batch, not just the first
    /// (`C4Viewport.cpp:227`).
    Refused,
    /// Not a definition file. `DropFile` returns false without a word — the
    /// failure text lives inside the `.c4d` branch, so a dropped scenario or
    /// player says nothing at all (`C4Game.cpp:1641-1657`).
    Ignored,
    /// Enqueue `CID_EMDropDef` for this definition.
    Drop(DefinitionId),
    /// `IDS_CNS_DROPNODEF`, formatted with this text.
    NoDefinition(String),
}

/// `C4Game::DropFile` for one path (`C4Game.cpp:1641-1660`).
///
/// The resolution order is C++'s and is load-bearing: the id is read from the
/// file, then an **already loaded** definition wins, and only if there is none
/// is the definition loaded from that file and looked up a second time. A file
/// whose `DefCore` will not parse fails the same way a file whose definition
/// cannot be loaded does.
///
/// Both failures report the **file name**, not the id — `GetFilename(
/// szFilename)` (`:1655`). That matters: a `.c4d` whose `DefCore` is unreadable
/// has no id to name.
pub fn drop_file(
    editing: bool,
    path: &Path,
    id_of_file: impl FnOnce(&Path) -> Option<DefinitionId>,
    is_loaded: impl Fn(&str) -> bool,
    load_definition: impl FnOnce(&Path) -> bool,
) -> DropOutcome {
    // `if (!Console.Editing) { Console.Message(IDS_CNS_NONETEDIT); return false; }`
    // — and it is asked once for the whole batch, before any file is looked at.
    if !editing {
        return DropOutcome::Refused;
    }
    if !is_definition_file(path) {
        return DropOutcome::Ignored;
    }
    let name = dropped_file_name(path);
    let Some(id) = id_of_file(path) else {
        return DropOutcome::NoDefinition(name);
    };
    if is_loaded(&id) {
        // `C4Id2Def(c_id)` succeeded, so `DropDef` cannot fail its own check.
        return DropOutcome::Drop(id);
    }
    // `Defs.Load(szFilename, …) && (cdef = C4Id2Def(c_id))` — the load has to
    // succeed *and* the id has to resolve afterwards; C++ tests both.
    if load_definition(path) && is_loaded(&id) {
        return DropOutcome::Drop(id);
    }
    DropOutcome::NoDefinition(name)
}

/// `C4Game::DropDef` (`C4Game.cpp:1662-1676`) — the direct entry, which has no
/// file and reports the **id** instead (`C4IdText(id)`).
pub fn drop_definition(id: &str, is_loaded: impl Fn(&str) -> bool) -> DropOutcome {
    if is_loaded(id) {
        return DropOutcome::Drop(id.to_owned());
    }
    DropOutcome::NoDefinition(clonk_script::c4_id_text(id))
}

/// `SEqualNoCase(GetExtension(szFilename), "c4d")`.
pub fn is_definition_file(path: &Path) -> bool {
    path.extension()
        .and_then(|extension| extension.to_str())
        .is_some_and(|extension| extension.eq_ignore_ascii_case(DEFINITION_FILE_EXTENSION))
}

/// `GetFilename(szFilename)` — the last path component, which is what the
/// failure text names.
fn dropped_file_name(path: &Path) -> String {
    path.file_name()
        .and_then(|name| name.to_str())
        .unwrap_or_default()
        .to_owned()
}

/// The world position a drop lands at (`C4Viewport.cpp:112,236`).
///
/// `local` is the drop point in the viewport's **own** coordinates, already
/// divided by the application scale. The two C++ entry points disagree about
/// that division: `WM_USER_DROPDEF` divides (`:112`) and `DropFiles` does not
/// (`:236`), so at a scale other than 1 the same screen point drops an object
/// in two different places depending on which message carried it. The port
/// takes the dividing one for both, because it is the conversion every other
/// pointer path in a viewport window already performs and the one that puts
/// the object under the cursor. This cannot affect determinism: the position
/// is decided locally and then travels in the control.
pub fn drop_world_position(view_x: i32, view_y: i32, local: (i32, i32)) -> (i32, i32) {
    (
        view_x.saturating_add(local.0),
        view_y.saturating_add(local.1),
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    fn path(name: &str) -> PathBuf {
        PathBuf::from("/content/Objects.c4d").with_file_name(name)
    }

    // C4Viewport.cpp:225-240 and C4Game.cpp:1641-1676 — the drop's gate, its
    // extension filter, its two-step resolution and its two failure texts.
    #[test]
    fn drop_file_resolves_a_loaded_definition_before_loading_the_dropped_one() {
        let never = |_: &Path| -> Option<DefinitionId> { panic!("not reached") };
        let no_load = |_: &Path| panic!("not reached");

        // The console must be able to edit, and the check comes first — a
        // refusal never even looks at the extension.
        assert_eq!(
            drop_file(false, &path("Rock.c4d"), never, |_| true, no_load),
            DropOutcome::Refused
        );

        // Only `.c4d` is considered, and anything else is ignored *silently*:
        // the failure text sits inside that branch.
        for name in ["Round.c4s", "Ada.c4p", "Rock"] {
            assert_eq!(
                drop_file(true, &path(name), never, |_| true, no_load),
                DropOutcome::Ignored,
                "{name} is not a definition file"
            );
        }
        // `SEqualNoCase` — the extension test ignores case.
        assert_eq!(
            drop_file(
                true,
                &path("Rock.C4D"),
                |_| Some("ROCK".to_owned()),
                |_| true,
                no_load
            ),
            DropOutcome::Drop("ROCK".to_owned())
        );

        // A `.c4d` whose DefCore will not parse has no id to name, so the
        // failure reports the file.
        assert_eq!(
            drop_file(true, &path("Broken.c4d"), |_| None, |_| true, no_load),
            DropOutcome::NoDefinition("Broken.c4d".to_owned())
        );

        // An id that is already loaded drops without touching the file again.
        assert_eq!(
            drop_file(
                true,
                &path("Rock.c4d"),
                |_| Some("ROCK".to_owned()),
                |id| id == "ROCK",
                no_load
            ),
            DropOutcome::Drop("ROCK".to_owned())
        );

        // An unloaded id loads the definition from the dropped file and looks
        // it up *again*; C++ tests both, so a load that reports success but
        // leaves the id unresolved is still a failure.
        let loaded = std::cell::Cell::new(false);
        assert_eq!(
            drop_file(
                true,
                &path("Rock.c4d"),
                |_| Some("ROCK".to_owned()),
                |id| id == "ROCK" && loaded.get(),
                |_| {
                    loaded.set(true);
                    true
                }
            ),
            DropOutcome::Drop("ROCK".to_owned())
        );
        assert_eq!(
            drop_file(
                true,
                &path("Rock.c4d"),
                |_| Some("ROCK".to_owned()),
                |_| false,
                |_| true
            ),
            DropOutcome::NoDefinition("Rock.c4d".to_owned()),
            "the load succeeded but the id still does not resolve"
        );
        assert_eq!(
            drop_file(
                true,
                &path("Rock.c4d"),
                |_| Some("ROCK".to_owned()),
                |_| false,
                |_| false
            ),
            DropOutcome::NoDefinition("Rock.c4d".to_owned())
        );
    }

    // C4Game.cpp:1662-1676 — the direct entry reports the *id*, where the file
    // entry reports the file name.
    #[test]
    fn drop_definition_reports_the_id_where_drop_file_reports_the_file() {
        assert_eq!(
            drop_definition("ROCK", |id| id == "ROCK"),
            DropOutcome::Drop("ROCK".to_owned())
        );
        assert_eq!(
            drop_definition("ROCK", |_| false),
            DropOutcome::NoDefinition("ROCK".to_owned())
        );
    }

    // C4Viewport.cpp:112,236 — the drop point is added to the view origin.
    // The caller has already divided by the application scale, which is what
    // `WM_USER_DROPDEF` does and `DropFiles` does not; the port takes the
    // dividing one for both, so this function only offsets.
    #[test]
    fn drop_world_position_offsets_the_view_origin() {
        assert_eq!(drop_world_position(100, 40, (7, 3)), (107, 43));
        assert_eq!(drop_world_position(0, 0, (0, 0)), (0, 0));
        assert_eq!(drop_world_position(-20, -5, (4, 4)), (-16, -1));
        // A degenerate origin saturates rather than wrapping into a position
        // on the far side of the landscape.
        assert_eq!(
            drop_world_position(i32::MAX, i32::MAX, (1, 1)),
            (i32::MAX, i32::MAX)
        );
    }
}
