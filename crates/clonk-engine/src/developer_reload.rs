//! Matching a changed file path to a loaded definition.
//!
//! `C4DefList::GetByPath` (`C4Def.cpp:1137-1152`) decides whether a path names
//! a definition for reload purposes:
//!
//! ```cpp
//! const auto defPath = Config.AtExeRelativePath(def->Filename);
//! if (defPath && SEqual2NoCase(szPath, defPath))
//!     return !szPath[SLen(defPath)]
//!         || (szPath[SLen(defPath)] == '\\' && !strchr(szPath + SLen(defPath) + 1, '\\'));
//! ```
//!
//! So a path matches only the definition **root itself** or **one immediate
//! child** — never a grandchild. Anything else falls through to the generic
//! script-host reload path (`C4ScriptHost.cpp:135-149`).
//!
//! Comparison is case-insensitive, matching `SEqual2NoCase`.

/// The native path separator in a stored `C4Def::Filename`. C++ compares
/// against `'\\'` literally, so a caller normalising to `/` must say so.
pub const DEFINITION_PATH_SEPARATOR: char = '\\';

/// How `changed` relates to a definition rooted at `definition_path`.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum DefinitionPathMatch {
    /// The definition group itself.
    Root,
    /// Exactly one component below the root — a component file such as
    /// `Script.c` or `DefCore.txt`.
    ImmediateChild,
    /// Not this definition: a different path, or nested deeper than one level.
    None,
}

/// `C4DefList::GetByPath`'s per-definition test (`C4Def.cpp:1139-1147`).
///
/// `separator` is the separator used by both paths — `'\\'` for native stored
/// filenames, `'/'` for a caller that has already normalised.
pub fn definition_path_match(
    changed: &str,
    definition_path: &str,
    separator: char,
) -> DefinitionPathMatch {
    if definition_path.is_empty() {
        return DefinitionPathMatch::None;
    }
    // `SEqual2NoCase(szPath, defPath)` — a case-insensitive prefix test.
    let Some(rest) = strip_prefix_ignore_case(changed, definition_path) else {
        return DefinitionPathMatch::None;
    };
    // `!szPath[SLen(defPath)]` — the definition group itself.
    if rest.is_empty() {
        return DefinitionPathMatch::Root;
    }
    // `szPath[SLen(defPath)] == '\\' && !strchr(... + 1, '\\')` — exactly one
    // component below, with no further separator.
    let Some(child) = rest.strip_prefix(separator) else {
        // The prefix matched mid-component (`Rock.c4d` vs `Rock.c4dx`), which
        // C++ rejects because the next byte is neither NUL nor a separator.
        return DefinitionPathMatch::None;
    };
    if child.is_empty() || child.contains(separator) {
        return DefinitionPathMatch::None;
    }
    DefinitionPathMatch::ImmediateChild
}

/// The first definition whose path matches, in list order — `GetByPath` returns
/// the first `find_if` hit (`C4Def.cpp:1139-1150`).
pub fn find_definition_by_path<'a>(
    changed: &str,
    definition_paths: impl IntoIterator<Item = &'a str>,
    separator: char,
) -> Option<(&'a str, DefinitionPathMatch)> {
    definition_paths.into_iter().find_map(|path| {
        match definition_path_match(changed, path, separator) {
            DefinitionPathMatch::None => None,
            matched => Some((path, matched)),
        }
    })
}

fn strip_prefix_ignore_case<'a>(value: &'a str, prefix: &str) -> Option<&'a str> {
    let bytes = value.as_bytes();
    let prefix_bytes = prefix.as_bytes();
    if bytes.len() < prefix_bytes.len() {
        return None;
    }
    bytes[..prefix_bytes.len()]
        .iter()
        .zip(prefix_bytes)
        .all(|(left, right)| left.eq_ignore_ascii_case(right))
        .then(|| &value[prefix.len()..])
}

/// What `C4Game::ReloadDef` does before it touches anything (`C4Game.cpp`).
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum DefinitionReloadPlan {
    /// `if (Network.isEnabled()) return false;` — the very first line. A
    /// network game never reloads a definition, whatever changed on disk.
    RefusedInNetwork,
    /// `Defs.ID2Def(id)` found nothing, so there is nothing to reload. Note
    /// this check happens *after* `Synchronize(false)`, which has already run.
    UnknownDefinition,
    /// Reload it. `synchronize` mirrors `Synchronize(false)`: the game is
    /// synchronised so menus holding dead surfaces are closed, but player files
    /// are deliberately **not** written back.
    Reload { synchronize: bool },
}

/// `C4Game::ReloadDef`'s two outcomes, both of which touch every object of the
/// reloaded type.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum DefinitionReloadOutcome {
    /// `Defs.Reload` succeeded: every object of that id gets
    /// `UpdateFace(true)`. C++ refreshes them all rather than trying to work
    /// out which are affected — objects can use another definition's graphics,
    /// so "better update everything".
    Reloaded { refresh_faces: Vec<crate::ObjectId> },
    /// `Defs.Reload` failed: every object of that id is removed
    /// (`AssignRemoval`), any running script profile is aborted, and the
    /// definition itself is dropped from the list.
    Failed {
        remove_objects: Vec<crate::ObjectId>,
        abort_profiler: bool,
        remove_definition: bool,
    },
}

/// The gate `C4Game::ReloadDef` applies before reloading.
pub fn definition_reload_plan(network_game: bool, definition_known: bool) -> DefinitionReloadPlan {
    if network_game {
        return DefinitionReloadPlan::RefusedInNetwork;
    }
    if !definition_known {
        return DefinitionReloadPlan::UnknownDefinition;
    }
    DefinitionReloadPlan::Reload { synchronize: true }
}

/// What to do with live objects once `Defs.Reload` has answered.
///
/// `objects_of_definition` is every object whose `id` matches, in
/// `Game.Objects` order — C++ walks the master list First -> Next in both arms.
/// Whichever arm runs, `Messages.UpdateDef(id)` follows it.
pub fn definition_reload_outcome(
    reloaded: bool,
    objects_of_definition: &[crate::ObjectId],
) -> DefinitionReloadOutcome {
    if reloaded {
        return DefinitionReloadOutcome::Reloaded {
            refresh_faces: objects_of_definition.to_vec(),
        };
    }
    DefinitionReloadOutcome::Failed {
        remove_objects: objects_of_definition.to_vec(),
        abort_profiler: true,
        remove_definition: true,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const SEP: char = DEFINITION_PATH_SEPARATOR;
    const ROOT: &str = "Objects.c4d\\Rock.c4d";

    // C4Def.cpp:1137-1152 — the root or exactly one child, case-insensitively,
    // and nothing deeper.
    #[test]
    fn definition_path_matches_only_the_root_or_one_immediate_child() {
        assert_eq!(
            definition_path_match(ROOT, ROOT, SEP),
            DefinitionPathMatch::Root
        );
        assert_eq!(
            definition_path_match("Objects.c4d\\Rock.c4d\\Script.c", ROOT, SEP),
            DefinitionPathMatch::ImmediateChild
        );
        assert_eq!(
            definition_path_match("Objects.c4d\\Rock.c4d\\DefCore.txt", ROOT, SEP),
            DefinitionPathMatch::ImmediateChild
        );

        // A grandchild is not this definition — `strchr` finds a second
        // separator, so C++ falls through to the generic host path (:1144).
        assert_eq!(
            definition_path_match("Objects.c4d\\Rock.c4d\\Graphics\\Overlay.png", ROOT, SEP),
            DefinitionPathMatch::None
        );

        // `SEqual2NoCase` is case-insensitive.
        assert_eq!(
            definition_path_match("objects.C4D\\ROCK.c4d\\script.C", ROOT, SEP),
            DefinitionPathMatch::ImmediateChild
        );

        // A prefix that stops mid-component is rejected: the byte after the
        // prefix is neither NUL nor a separator.
        assert_eq!(
            definition_path_match("Objects.c4d\\Rock.c4dx", ROOT, SEP),
            DefinitionPathMatch::None
        );
        assert_eq!(
            definition_path_match("Objects.c4d\\RockSolid.c4d\\Script.c", ROOT, SEP),
            DefinitionPathMatch::None
        );

        // A trailing separator names no child.
        assert_eq!(
            definition_path_match("Objects.c4d\\Rock.c4d\\", ROOT, SEP),
            DefinitionPathMatch::None
        );

        // An unrelated path, a shorter path, and an empty definition path.
        assert_eq!(
            definition_path_match("Objects.c4d\\Wood.c4d\\Script.c", ROOT, SEP),
            DefinitionPathMatch::None
        );
        assert_eq!(
            definition_path_match("Objects.c4d", ROOT, SEP),
            DefinitionPathMatch::None
        );
        assert_eq!(
            definition_path_match(ROOT, "", SEP),
            DefinitionPathMatch::None
        );

        // The same rules hold for a caller that normalised to `/`.
        assert_eq!(
            definition_path_match("Objects.c4d/Rock.c4d/Script.c", "Objects.c4d/Rock.c4d", '/'),
            DefinitionPathMatch::ImmediateChild
        );
    }

    // `GetByPath` returns the first matching definition in list order.
    #[test]
    fn definition_lookup_returns_the_first_match_in_list_order() {
        let paths = [
            "Objects.c4d\\Wood.c4d",
            "Objects.c4d\\Rock.c4d",
            "Objects.c4d\\Rock.c4d",
        ];
        assert_eq!(
            find_definition_by_path("Objects.c4d\\Rock.c4d\\Script.c", paths, SEP),
            Some(("Objects.c4d\\Rock.c4d", DefinitionPathMatch::ImmediateChild))
        );
        assert_eq!(
            find_definition_by_path("Objects.c4d\\Metal.c4d\\Script.c", paths, SEP),
            None,
            "an unmatched path falls through to the generic script-host reload"
        );
    }

    // C4Game::ReloadDef — the network refusal, the no-player-writeback sync,
    // and the two symmetric object sweeps.
    #[test]
    fn console_definition_reload_refuses_network_and_sweeps_every_matching_object() {
        use crate::ObjectId;

        // The network check is the very first line: nothing else is attempted.
        assert_eq!(
            definition_reload_plan(true, true),
            DefinitionReloadPlan::RefusedInNetwork
        );
        assert_eq!(
            definition_reload_plan(true, false),
            DefinitionReloadPlan::RefusedInNetwork
        );
        // An unknown id stops after the synchronise, which has already run.
        assert_eq!(
            definition_reload_plan(false, false),
            DefinitionReloadPlan::UnknownDefinition
        );
        // Otherwise: Synchronize(false) — sync, but do not write player files.
        assert_eq!(
            definition_reload_plan(false, true),
            DefinitionReloadPlan::Reload { synchronize: true }
        );

        let objects = [ObjectId(4), ObjectId(9), ObjectId(2)];

        // Success refreshes the face of *every* object of that type, in master
        // list order. C++ does not try to work out which are affected — an
        // object can use another definition's graphics.
        assert_eq!(
            definition_reload_outcome(true, &objects),
            DefinitionReloadOutcome::Reloaded {
                refresh_faces: objects.to_vec()
            }
        );

        // Failure is the harsh arm: every object of that type is removed, the
        // profiler is aborted, and the definition itself is dropped.
        assert_eq!(
            definition_reload_outcome(false, &objects),
            DefinitionReloadOutcome::Failed {
                remove_objects: objects.to_vec(),
                abort_profiler: true,
                remove_definition: true,
            }
        );

        // With no live objects both arms are empty sweeps, but the failure arm
        // still removes the definition.
        assert_eq!(
            definition_reload_outcome(true, &[]),
            DefinitionReloadOutcome::Reloaded {
                refresh_faces: Vec::new()
            }
        );
        assert_eq!(
            definition_reload_outcome(false, &[]),
            DefinitionReloadOutcome::Failed {
                remove_objects: Vec::new(),
                abort_profiler: true,
                remove_definition: true,
            }
        );
    }
}
