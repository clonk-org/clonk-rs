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

/// `C4D_Load_*` — which parts of a definition a load or reload covers
/// (`C4Def.h:119-130`).
pub mod load_what {
    pub const NONE: u32 = 0;
    pub const PICTURE: u32 = 1;
    pub const BITMAP: u32 = 2;
    pub const SCRIPT: u32 = 4;
    pub const DESC: u32 = 8;
    pub const ACT_MAP: u32 = 16;
    pub const IMAGE: u32 = 32;
    pub const SOUNDS: u32 = 64;
    pub const CLONK_NAMES: u32 = 128;
    pub const RANK_NAMES: u32 = 256;
    pub const RANK_FACES: u32 = 512;

    /// `C4D_Load_RX` — the default `C4Game::ReloadDef` uses
    /// (`C4Game.h:225`).
    ///
    /// Note what it leaves out: **`PICTURE` and `IMAGE` are not included**, so a
    /// console reload deliberately does not rebuild the definition's picture or
    /// image facets. Reloading them as well would look like a harmless
    /// completeness fix and would diverge.
    pub const RX: u32 =
        BITMAP | SCRIPT | CLONK_NAMES | DESC | ACT_MAP | SOUNDS | RANK_NAMES | RANK_FACES;
}

/// One step of `C4DefList::Reload` (`C4Def.cpp`), in call order.
///
/// The sequence is load-bearing in three places a rewrite tends to reorder:
///
/// - `SortByID` rebuilds the quick-access table **before** the relink, so the
///   relink sees the definition at its final position.
/// - `ReLink` runs **before** graphics are restored, and it "will also do
///   include callbacks" — so a script that inspects graphics during an include
///   callback sees the *backed-up* set, not the reloaded one.
/// - Graphics are restored last, by `C4DefGraphicsPtrBackup::AssignUpdate`,
///   which remaps live pointers rather than reassigning wholesale.
///
/// Failure is guarded by the backup's destructor: if the group cannot be opened
/// or `Load` fails, every graphic is reset to default on the way out. And
/// `Clear` deliberately **keeps the filename** ("Assume filename is being
/// kept"), which is what lets the reload re-open the same group it came from.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum DefinitionReloadStep {
    /// `C4DefGraphicsPtrBackup GfxBackup(&pDef->Graphics)`.
    BackUpGraphics,
    /// `pDef->Clear()` — the filename survives.
    ClearKeepingFilename,
    /// `hGroup.Open(pDef->Filename)`.
    OpenGroup,
    /// `pDef->Load(hGroup, dwLoadWhat, szLanguage, pSoundSystem)`.
    Load,
    /// `SortByID()`.
    SortById,
    /// `Game.ScriptEngine.ReLink(this)`, include callbacks and all.
    RelinkScripts,
    /// `GfxBackup.AssignUpdate(&pDef->Graphics)`.
    RestoreGraphics,
}

/// `C4DefList::Reload`'s full sequence.
pub const DEFINITION_RELOAD_STEPS: [DefinitionReloadStep; 7] = [
    DefinitionReloadStep::BackUpGraphics,
    DefinitionReloadStep::ClearKeepingFilename,
    DefinitionReloadStep::OpenGroup,
    DefinitionReloadStep::Load,
    DefinitionReloadStep::SortById,
    DefinitionReloadStep::RelinkScripts,
    DefinitionReloadStep::RestoreGraphics,
];

/// The steps that actually run, given where the reload stopped.
///
/// `opened` is `hGroup.Open`, `loaded` is `pDef->Load`. Either failing returns
/// early — the remaining steps do not run, and the graphics backup's destructor
/// resets every graphic to default.
pub fn definition_reload_steps(opened: bool, loaded: bool) -> Vec<DefinitionReloadStep> {
    let taken = match (opened, loaded) {
        (false, _) => 3,
        (true, false) => 4,
        (true, true) => DEFINITION_RELOAD_STEPS.len(),
    };
    DEFINITION_RELOAD_STEPS[..taken].to_vec()
}

/// Whether a stopped reload must reset graphics to default — the backup
/// destructor's job on every early return.
pub fn definition_reload_resets_graphics(opened: bool, loaded: bool) -> bool {
    !(opened && loaded)
}

/// Where a changed file's reload goes (`C4Game::ReloadFile`, `C4Game.cpp:2306`).
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum ChangedFileRoute {
    /// `if (Network.isEnabled()) return;` — the first line again. A network
    /// game ignores the watcher entirely.
    RefusedInNetwork,
    /// `Defs.GetByPath` matched: reload that definition.
    Definition { definition: String },
    /// No definition owns the path, so it goes to the generic script host —
    /// `ScriptEngine.ReloadScript(relativePath, &Defs)`. Note this is the
    /// *fallback*, not a separate branch of the match: an unmatched path is
    /// always offered to the script engine.
    Script { relative_path: String },
}

/// `C4Game::ReloadFile` (`C4Game.cpp:2306-2319`).
///
/// `relative_path` must already be `Config.AtExeRelativePath(path)` — C++
/// converts before matching, so an absolute watcher path never reaches
/// `GetByPath`. `definition_for_path` is
/// [`find_definition_by_path`] over the loaded definitions.
pub fn changed_file_route(
    network_game: bool,
    relative_path: &str,
    definition_for_path: impl FnOnce(&str) -> Option<String>,
) -> ChangedFileRoute {
    if network_game {
        return ChangedFileRoute::RefusedInNetwork;
    }
    match definition_for_path(relative_path) {
        Some(definition) => ChangedFileRoute::Definition { definition },
        None => ChangedFileRoute::Script {
            relative_path: relative_path.to_owned(),
        },
    }
}

/// `C4Game::ReloadParticle` (`C4Game.cpp`).
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum ParticleReloadOutcome {
    /// Network games refuse, like every other reload path.
    RefusedInNetwork,
    /// `Particles.GetDef(szName)` found nothing. Nothing is reloaded and
    /// nothing is cleared.
    UnknownParticle,
    /// `pDef->Reload()` succeeded.
    Reloaded,
    /// `pDef->Reload()` failed. C++ is blunt about it: **every particle in the
    /// system is cleared**, not just this definition's, and the definition is
    /// deleted.
    Failed {
        clear_all_particles: bool,
        remove_definition: bool,
    },
}

/// `C4Game::ReloadParticle`. `reload` runs only when the name is known, and
/// reports whether `C4ParticleDef::Reload` succeeded.
pub fn particle_reload_outcome(
    network_game: bool,
    particle_known: bool,
    reload: impl FnOnce() -> bool,
) -> ParticleReloadOutcome {
    if network_game {
        return ParticleReloadOutcome::RefusedInNetwork;
    }
    if !particle_known {
        return ParticleReloadOutcome::UnknownParticle;
    }
    if reload() {
        return ParticleReloadOutcome::Reloaded;
    }
    ParticleReloadOutcome::Failed {
        clear_all_particles: true,
        remove_definition: true,
    }
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

    // C4DefList::Reload — the order, and the failure guard.
    #[test]
    fn definition_reload_relinks_before_restoring_graphics() {
        use DefinitionReloadStep::*;
        let full = definition_reload_steps(true, true);
        assert_eq!(full, DEFINITION_RELOAD_STEPS.to_vec());

        let at = |step: DefinitionReloadStep| full.iter().position(|held| *held == step).unwrap();
        // The table is rebuilt before the relink, so the relink sees the
        // definition at its final position...
        assert!(at(SortById) < at(RelinkScripts));
        // ...and the relink — which also runs include callbacks — happens
        // BEFORE graphics are restored, so those callbacks see the backed-up
        // graphics, not the reloaded ones.
        assert!(at(RelinkScripts) < at(RestoreGraphics));
        // Clear keeps the filename, which is what the re-open depends on.
        assert!(at(ClearKeepingFilename) < at(OpenGroup));
        assert!(at(BackUpGraphics) < at(ClearKeepingFilename));

        // A group that will not open stops after Clear; nothing is sorted,
        // relinked or restored, and graphics fall back to default.
        assert_eq!(
            definition_reload_steps(false, false),
            vec![BackUpGraphics, ClearKeepingFilename, OpenGroup]
        );
        assert!(definition_reload_resets_graphics(false, false));

        // A failed Load stops one step later, with the same guarantee — the
        // definition is left cleared, which is why the caller removes it.
        assert_eq!(
            definition_reload_steps(true, false),
            vec![BackUpGraphics, ClearKeepingFilename, OpenGroup, Load]
        );
        assert!(definition_reload_resets_graphics(true, false));

        // Only a complete reload keeps the reloaded graphics.
        assert!(!definition_reload_resets_graphics(true, true));

        // C4Def.h:119-130 / C4Game.h:225 — the console reload's flag set.
        use load_what::*;
        assert_eq!(RX, 2 | 4 | 128 | 8 | 16 | 64 | 256 | 512);
        assert_eq!(RX, 990);
        // The omissions are the point: a console reload rebuilds neither the
        // picture nor the image facet.
        assert_eq!(RX & PICTURE, 0, "C4D_Load_RX excludes Picture");
        assert_eq!(RX & IMAGE, 0, "C4D_Load_RX excludes Image");
        // Everything else is in.
        for included in [
            BITMAP,
            SCRIPT,
            DESC,
            ACT_MAP,
            SOUNDS,
            CLONK_NAMES,
            RANK_NAMES,
            RANK_FACES,
        ] {
            assert_eq!(RX & included, included);
        }
        assert_eq!(NONE, 0);
    }

    // C4Game::ReloadFile and C4Game::ReloadParticle — the watcher's dispatch
    // and the particle path's blunt failure policy.
    #[test]
    fn external_reload_routes_by_definition_and_clears_particles_on_failure() {
        let matches = |path: &str| (path == "Objects.c4d\\Rock.c4d").then(|| "ROCK".to_owned());

        // Network games ignore the watcher entirely — the first line again.
        assert_eq!(
            changed_file_route(true, "Objects.c4d\\Rock.c4d", matches),
            ChangedFileRoute::RefusedInNetwork
        );

        // A matched path reloads that definition.
        assert_eq!(
            changed_file_route(false, "Objects.c4d\\Rock.c4d", matches),
            ChangedFileRoute::Definition {
                definition: "ROCK".to_owned()
            }
        );

        // An unmatched path is not dropped: it always falls through to the
        // generic script host.
        assert_eq!(
            changed_file_route(false, "System.c4g\\Helper.c", matches),
            ChangedFileRoute::Script {
                relative_path: "System.c4g\\Helper.c".to_owned()
            }
        );

        // Particles: refusal, unknown name, success, and the failure sweep.
        assert_eq!(
            particle_reload_outcome(true, true, || unreachable!("no reload in a network game")),
            ParticleReloadOutcome::RefusedInNetwork
        );
        assert_eq!(
            particle_reload_outcome(false, false, || unreachable!(
                "an unknown particle is never reloaded"
            )),
            ParticleReloadOutcome::UnknownParticle
        );
        assert_eq!(
            particle_reload_outcome(false, true, || true),
            ParticleReloadOutcome::Reloaded
        );
        // Failure clears **every** particle in the system, not just this
        // definition's, and deletes the definition.
        assert_eq!(
            particle_reload_outcome(false, true, || false),
            ParticleReloadOutcome::Failed {
                clear_all_particles: true,
                remove_definition: true,
            }
        );
    }
}
