//! Whether to update, and to what.
//!
//! This is the port of the gating C++ performs in
//! `C4UpdateDlg::IsValidUpdate` (`C4UpdateDlg.cpp:246-260`) plus the
//! once-a-day throttle from `C4UpdateDlg::CheckForUpdates`
//! (`C4UpdateDlg.cpp:262-268`). It is pure: no clock of its own, no filesystem,
//! no network, so every rule below is directly testable.
//!
//! # The engine gate
//!
//! C++ refuses an update whose `iVer[0]`/`iVer[1]` differ from the running
//! engine (`C4UpdateDlg.cpp:248`). This port widens that to the first four
//! slots of the C4XVer tuple, and the reason is specific rather than
//! defensive: C++ shipped a `.c4u` that an *update program* migrated, whereas
//! this port ships `content` as a prebuilt component. `clonk-core::version`
//! documents that `definition_requires_newer_engine` **silently prunes**
//! definitions declaring a newer engine — so installing content built against a
//! different engine would not raise an error, it would quietly delete the
//! game's definitions. The build number (slot 4) stays outside the gate, since
//! C++ treats a higher build as a valid update on its own
//! (`C4UpdateDlg.cpp:256`).
//!
//! # Timing
//!
//! Validity has no time component: with no signature and no manifest expiry,
//! *when* the client looks changes nothing about whether the release is
//! acceptable. The only clock C++ consults is the automatic-check throttle,
//! which is why it lives in [`should_check_for_updates`] — a separate function
//! taking `now` explicitly — rather than in [`decide`].

use crate::manifest::{ArchiveSource, Manifest, TargetArchive, SUPPORTED_SCHEMA};
use crate::state::InstalledState;
use semver::Version;
use std::fmt;
use std::path::PathBuf;

/// `60 * 60 * 24`, the automatic-check interval (`C4UpdateDlg.cpp:265`).
pub const SECONDS_PER_DAY: i64 = 60 * 60 * 24;

/// A component this client has decided to fetch and install.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PlannedComponent {
    pub name: String,
    /// Release-asset file name; the fetching layer turns it into a URL.
    pub archive: String,
    /// Expected SHA-256 of the archive, verified before anything is unpacked.
    pub sha256: String,
    pub size: u64,
    /// Where the archive unpacks, relative to the install root. Empty means
    /// the install root itself, which is how the engine component — whose
    /// archive already carries `bin/…` or `Contents/…` — is expressed.
    pub destination: PathBuf,
    /// Which release holds the archive, when it is not this repository's own.
    ///
    /// `content` is built and published by the content repository, so its
    /// archive lives in a different release entirely. Dropping this on the way
    /// into the plan would send the fetcher to a clonk-rs release that has no
    /// `content.zip` in it.
    pub source: Option<ArchiveSource>,
}

/// Why an otherwise well-formed manifest will not be acted on.
///
/// A typed reason rather than a message: the caller picks the localized string,
/// and a refusal is never a free-text surprise.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RefusalReason {
    UnsupportedSchema {
        found: u32,
    },
    UnreadableVersion {
        version: String,
    },
    UnreadableInstalledVersion {
        version: String,
    },
    EngineMismatch {
        offered: [i32; 5],
        running: [i32; 5],
    },
    UnsafeDestination {
        component: String,
        destination: String,
    },
}

impl fmt::Display for RefusalReason {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::UnsupportedSchema { found } => write!(
                formatter,
                "manifest schema {found} is newer than the {SUPPORTED_SCHEMA} this build reads"
            ),
            Self::UnreadableVersion { version } => {
                write!(formatter, "offered version {version:?} is not a version")
            }
            Self::UnreadableInstalledVersion { version } => {
                write!(formatter, "installed version {version:?} is not a version")
            }
            Self::EngineMismatch { offered, running } => write!(
                formatter,
                "offered engine {offered:?} does not match the running engine {running:?}"
            ),
            Self::UnsafeDestination {
                component,
                destination,
            } => write!(
                formatter,
                "component {component} wants to install to {destination:?}, \
                 which is not inside the install root"
            ),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Decision {
    /// Nothing to do. Covers "already current", "the offer is older", and "this
    /// release has nothing for our platform" — none of which is a failure.
    UpToDate,
    /// The manifest is understood but must not be acted on.
    Refused { reason: RefusalReason },
    Update {
        version: String,
        components: Vec<PlannedComponent>,
    },
}

/// Turns a publisher-supplied destination into a path, or rejects it.
///
/// Manifest destinations are always `/`-separated relative paths, and the empty
/// string means the install root. Everything else — a root, a drive letter, a
/// `..`, even a `.` — is refused rather than normalised, because normalising
/// publisher text into a write path is how directory traversal happens. The
/// check is deliberately platform-independent: a destination that is relative
/// on Linux but absolute on Windows must be refused on both.
fn safe_destination(destination: &str) -> Option<PathBuf> {
    if destination.is_empty() {
        return Some(PathBuf::new());
    }
    let rejected = |segment: &&str| {
        segment.is_empty()
            || matches!(*segment, "." | "..")
            || segment.contains('\\')
            || segment.contains(':')
    };
    (!destination.split('/').any(|segment| rejected(&segment)))
        .then(|| destination.split('/').collect::<PathBuf>())
}

/// Whether the archive offered for our triple differs from what is installed.
///
/// With a recorded state the digests decide, which is what lets a code-only
/// release skip 299 MB of content. Without one — an install that predates the
/// updater — there is nothing to compare, so the release version is the only
/// evidence available and the component is refreshed.
fn needs_install(name: &str, target: &TargetArchive, installed: &Option<InstalledState>) -> bool {
    installed
        .as_ref()
        .and_then(|state| state.component(name))
        .is_none_or(|component| !component.sha256.eq_ignore_ascii_case(&target.sha256))
}

/// Ports `C4UpdateDlg::IsValidUpdate` (`C4UpdateDlg.cpp:246-260`) and turns the
/// verdict into the concrete list of components to fetch.
pub fn decide(
    manifest: &Manifest,
    installed: &Option<InstalledState>,
    installed_version: &str,
    engine_version: [i32; 5],
    target_triple: &str,
) -> Decision {
    let refuse = |reason| Decision::Refused { reason };

    if manifest.schema != SUPPORTED_SCHEMA {
        return refuse(RefusalReason::UnsupportedSchema {
            found: manifest.schema,
        });
    }

    // Engine compatibility first, exactly as C++ orders it
    // (`C4UpdateDlg.cpp:248` precedes the version comparisons).
    if manifest.engine_version[..4] != engine_version[..4] {
        return refuse(RefusalReason::EngineMismatch {
            offered: manifest.engine_version,
            running: engine_version,
        });
    }

    let offered = match Version::parse(&manifest.version) {
        Ok(version) => version,
        Err(_) => {
            return refuse(RefusalReason::UnreadableVersion {
                version: manifest.version.clone(),
            })
        }
    };
    let current = match Version::parse(installed_version) {
        Ok(version) => version,
        Err(_) => {
            return refuse(RefusalReason::UnreadableInstalledVersion {
                version: installed_version.to_string(),
            })
        }
    };
    // Strictly newer, per `C4UpdateDlg.cpp:250-258`: equal is nothing to do,
    // and older is a downgrade this client will not perform.
    if offered <= current {
        return Decision::UpToDate;
    }

    let mut components = Vec::new();
    for entry in &manifest.components {
        // A component this release did not build for our platform simply is
        // not offered; that is an absent target, not a broken manifest.
        let Some(target) = entry.target_for(target_triple) else {
            continue;
        };
        let Some(destination) = safe_destination(&target.install) else {
            return refuse(RefusalReason::UnsafeDestination {
                component: entry.name.clone(),
                destination: target.install.clone(),
            });
        };
        if !needs_install(&entry.name, target, installed) {
            continue;
        }
        components.push(PlannedComponent {
            name: entry.name.clone(),
            archive: target.archive.clone(),
            sha256: target.sha256.clone(),
            size: target.size,
            destination,
            source: target.source.clone(),
        });
    }

    if components.is_empty() {
        return Decision::UpToDate;
    }
    Decision::Update {
        version: manifest.version.clone(),
        components,
    }
}

/// [`decide`] with this build's own identity filled in.
///
/// The port version gates the release comparison and the engine tuple gates
/// compatibility; they are different values and `clonk-core::version` exists
/// largely to stop them being conflated, so the wiring is written once here
/// rather than at every call site.
pub fn decide_for_this_build(
    manifest: &Manifest,
    installed: &Option<InstalledState>,
    target_triple: &str,
) -> Decision {
    decide(
        manifest,
        installed,
        clonk_core::version::PORT_VERSION,
        clonk_core::version::ENGINE_VERSION,
        target_triple,
    )
}

/// Ports the automatic-check throttle at `C4UpdateDlg.cpp:262-268`.
///
/// A manual check always runs; an automatic one runs only once a day. Both
/// timestamps are Unix seconds, matching the `LastUpdateTime` config key that
/// C++ stores with `time(nullptr)`.
pub fn should_check_for_updates(automatic: bool, now: i64, last_update_time: i64) -> bool {
    !automatic || now.saturating_sub(last_update_time) >= SECONDS_PER_DAY
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::manifest::{ComponentEntry, SUPPORTED_SCHEMA};
    use std::collections::BTreeMap;

    const LINUX: &str = "x86_64-unknown-linux-gnu";
    const MACOS: &str = "aarch64-apple-darwin";
    const ENGINE: [i32; 5] = [4, 9, 11, 0, 362];

    fn target(name: &str, sha: &str, install: &str) -> TargetArchive {
        TargetArchive {
            archive: format!("{name}-{sha}.zip"),
            // Where a component is published does not enter the decision: what
            // to fetch is decided by the digest, not by who hosts it.
            source: None,
            sha256: sha.repeat(32),
            size: 1024,
            install: install.to_string(),
        }
    }

    /// A shared component: identical bytes under both triples, differing only
    /// in where they land.
    fn entry(name: &str, sha: &str, linux: &str, macos: &str) -> ComponentEntry {
        ComponentEntry {
            name: name.to_string(),
            targets: BTreeMap::from([
                (LINUX.to_string(), target(name, sha, linux)),
                (MACOS.to_string(), target(name, sha, macos)),
            ]),
        }
    }

    fn manifest() -> Manifest {
        Manifest {
            schema: SUPPORTED_SCHEMA,
            version: "0.4.0".to_string(),
            engine_version: ENGINE,
            released_at: "2026-07-28T10:00:00Z".to_string(),
            components: vec![
                entry("content", "cc", "content", "Contents/Resources/content"),
                entry("planet", "dd", "planet", "Contents/Resources/planet"),
                entry("engine", "ee", "", ""),
            ],
        }
    }

    #[test]
    fn a_components_source_release_survives_into_the_plan() {
        // `content` is built and published by the content repository, so its
        // archive is in a different release. If the plan drops that, the
        // fetcher resolves the name against a clonk-rs release which has no
        // `content.zip` in it, and every content update 404s.
        let mut manifest = manifest();
        let content = manifest
            .components
            .iter_mut()
            .find(|entry| entry.name == "content")
            .expect("content entry");
        for target in content.targets.values_mut() {
            target.source = Some(ArchiveSource {
                repo: "syb0rg/clonk-rs-content".to_string(),
                tag: "content-d34d385".to_string(),
            });
        }

        let Decision::Update { components, .. } = decide(&manifest, &None, "0.3.0", ENGINE, LINUX)
        else {
            panic!("a newer release with no recorded state is an update");
        };

        let planned = components
            .iter()
            .find(|component| component.name == "content")
            .expect("content is planned");
        let source = planned
            .source
            .as_ref()
            .expect("the content source reached the plan");
        assert_eq!(source.repo, "syb0rg/clonk-rs-content");
        assert_eq!(source.tag, "content-d34d385");

        // Components published by this repository carry no source at all.
        let engine = components
            .iter()
            .find(|component| component.name == "engine")
            .expect("engine is planned");
        assert!(engine.source.is_none());
    }

    fn installed_at(version: &str, content_sha: &str) -> InstalledState {
        let mut state = InstalledState::default();
        state.record("content", version, &content_sha.repeat(32));
        state.record("planet", version, &"dd".repeat(32));
        state.record("engine", version, &"aa".repeat(32));
        state
    }

    fn planned(decision: &Decision) -> Vec<String> {
        match decision {
            Decision::Update { components, .. } => {
                components.iter().map(|c| c.name.clone()).collect()
            }
            other => panic!("expected an update, got {other:?}"),
        }
    }

    #[test]
    fn a_newer_release_plans_its_components_data_first_and_engine_last() {
        // Manifest order is apply order: an interrupted apply must leave the
        // old binary in place, able to retry, not a new binary beside stale
        // data.
        let decision = decide(&manifest(), &None, "0.3.0", ENGINE, LINUX);
        assert_eq!(planned(&decision), ["content", "planet", "engine"]);
        let Decision::Update {
            version,
            components,
        } = &decision
        else {
            panic!("expected an update");
        };
        assert_eq!(version, "0.4.0");
        assert_eq!(components[0].destination, PathBuf::from("content"));
        assert_eq!(components[0].sha256, "cc".repeat(32));
        assert_eq!(components[0].archive, "content-cc.zip");
        assert_eq!(components[0].size, 1024);
        // The engine archive already carries `bin/…` or `Contents/…`, so it
        // unpacks at the install root itself.
        assert_eq!(components[2].destination, PathBuf::new());
    }

    #[test]
    fn macos_lands_shared_components_inside_the_bundle() {
        let decision = decide(&manifest(), &None, "0.3.0", ENGINE, MACOS);
        let Decision::Update { components, .. } = &decision else {
            panic!("expected an update");
        };
        assert_eq!(
            components[0].destination,
            PathBuf::from("Contents/Resources/content")
        );
    }

    #[test]
    fn the_installed_release_is_up_to_date() {
        let state = Some(installed_at("0.4.0", "cc"));
        assert_eq!(
            decide(&manifest(), &state, "0.4.0", ENGINE, LINUX),
            Decision::UpToDate
        );
    }

    #[test]
    fn an_older_release_is_never_installed() {
        // C++ requires the offered version to be strictly higher before it
        // will fetch anything (`C4UpdateDlg.cpp:250-258`); anything else is a
        // downgrade, and a stale manifest replayed at a client must not roll
        // it backwards.
        assert_eq!(
            decide(&manifest(), &None, "0.5.0", ENGINE, LINUX),
            Decision::UpToDate
        );
    }

    #[test]
    fn a_differing_engine_tuple_is_refused() {
        // Mirrors the engine/game version mismatch rejection at
        // `C4UpdateDlg.cpp:248`, widened to the whole C4XVer tuple because our
        // `content` component ships prebuilt rather than being migrated:
        // `definition_requires_newer_engine` *prunes* definitions that declare
        // a newer engine, so a mismatched content component would silently
        // delete game content instead of failing.
        let mut manifest = manifest();
        manifest.engine_version = [4, 9, 12, 0, 362];
        assert!(matches!(
            decide(&manifest, &None, "0.3.0", ENGINE, LINUX),
            Decision::Refused {
                reason: RefusalReason::EngineMismatch { .. }
            }
        ));
    }

    #[test]
    fn a_differing_build_number_alone_still_updates() {
        // C++ treats a higher build as a valid update on its own
        // (`C4UpdateDlg.cpp:256`), so the build slot is outside the gate.
        let mut manifest = manifest();
        manifest.engine_version = [4, 9, 11, 0, 999];
        assert!(matches!(
            decide(&manifest, &None, "0.3.0", ENGINE, LINUX),
            Decision::Update { .. }
        ));
    }

    #[test]
    fn a_release_without_our_triple_is_up_to_date_not_an_error() {
        // A platform this release did not build for has no update available.
        // Reporting that as a failure would put an error in front of every
        // user of that platform once a day, forever.
        assert_eq!(
            decide(
                &manifest(),
                &None,
                "0.3.0",
                ENGINE,
                "riscv64gc-unknown-linux-gnu"
            ),
            Decision::UpToDate
        );
    }

    #[test]
    fn a_component_whose_digest_is_unchanged_is_not_downloaded_again() {
        // The whole point of splitting the release: `content` is 299 MB and
        // changes far more rarely than the code.
        let state = Some(installed_at("0.3.0", "cc"));
        let decision = decide(&manifest(), &state, "0.3.0", ENGINE, LINUX);
        assert_eq!(planned(&decision), ["engine"]);
    }

    #[test]
    fn without_recorded_state_every_component_is_planned() {
        // Upgrading from a build that predates the updater: there is no
        // per-component record to compare, so the release version is all the
        // evidence there is and everything is refreshed.
        let decision = decide(&manifest(), &None, "0.3.0", ENGINE, LINUX);
        assert_eq!(planned(&decision), ["content", "planet", "engine"]);
    }

    #[test]
    fn a_component_missing_from_the_recorded_state_is_planned() {
        let mut state = installed_at("0.3.0", "cc");
        state.components.remove("planet");
        let decision = decide(&manifest(), &Some(state), "0.3.0", ENGINE, LINUX);
        assert_eq!(planned(&decision), ["planet", "engine"]);
    }

    #[test]
    fn an_install_destination_outside_the_install_root_is_refused() {
        // The destination is publisher-supplied text that becomes a path we
        // write into, so it is checked before a single byte is fetched.
        for destination in [
            "..",
            "../evil",
            "/etc",
            "planet/../../evil",
            "planet\\..\\evil",
            "C:\\Windows",
            "./planet",
        ] {
            let mut manifest = manifest();
            manifest.components[0]
                .targets
                .insert(LINUX.to_string(), target("content", "cc", destination));
            assert!(
                matches!(
                    decide(&manifest, &None, "0.3.0", ENGINE, LINUX),
                    Decision::Refused {
                        reason: RefusalReason::UnsafeDestination { .. }
                    }
                ),
                "{destination:?} should not be accepted as an install destination"
            );
        }
    }

    #[test]
    fn a_manifest_that_cannot_be_compared_is_refused() {
        let unknown_schema = Manifest {
            schema: SUPPORTED_SCHEMA + 1,
            ..manifest()
        };
        assert!(matches!(
            decide(&unknown_schema, &None, "0.3.0", ENGINE, LINUX),
            Decision::Refused {
                reason: RefusalReason::UnsupportedSchema { .. }
            }
        ));

        let unreadable_version = Manifest {
            version: "tuesday".to_string(),
            ..manifest()
        };
        assert!(matches!(
            decide(&unreadable_version, &None, "0.3.0", ENGINE, LINUX),
            Decision::Refused {
                reason: RefusalReason::UnreadableVersion { .. }
            }
        ));

        assert!(matches!(
            decide(&manifest(), &None, "not-a-version", ENGINE, LINUX),
            Decision::Refused {
                reason: RefusalReason::UnreadableInstalledVersion { .. }
            }
        ));
    }

    #[test]
    fn a_rebuild_with_identical_payloads_is_up_to_date() {
        // A newer version number whose components all match byte-for-byte
        // leaves nothing to download.
        let mut state = installed_at("0.3.0", "cc");
        state.record("engine", "0.3.0", &"ee".repeat(32));
        assert_eq!(
            decide(&manifest(), &Some(state), "0.3.0", ENGINE, LINUX),
            Decision::UpToDate
        );
    }

    #[test]
    fn this_build_compares_against_its_own_port_and_engine_versions() {
        // The two are different values and conflating them is the failure mode
        // `clonk-core::version` warns about, so the convenience wrapper is
        // pinned to which one it feeds to which gate.
        let current = Manifest {
            version: clonk_core::version::PORT_VERSION.to_string(),
            engine_version: clonk_core::version::ENGINE_VERSION,
            ..manifest()
        };
        assert_eq!(
            decide_for_this_build(&current, &None, LINUX),
            Decision::UpToDate
        );

        let foreign_engine = Manifest {
            version: "999.0.0".to_string(),
            engine_version: [9, 9, 9, 9, 9],
            ..current
        };
        assert!(matches!(
            decide_for_this_build(&foreign_engine, &None, LINUX),
            Decision::Refused {
                reason: RefusalReason::EngineMismatch { .. }
            }
        ));
    }

    #[test]
    fn automatic_checks_are_skipped_within_twenty_four_hours() {
        // `C4UpdateDlg::CheckForUpdates` returns immediately when an automatic
        // check runs less than a day after the last one
        // (`C4UpdateDlg.cpp:264-266`).
        let last = 1_700_000_000;
        assert!(!should_check_for_updates(
            true,
            last + SECONDS_PER_DAY - 1,
            last
        ));
        assert!(should_check_for_updates(true, last + SECONDS_PER_DAY, last));
    }

    #[test]
    fn a_manual_check_ignores_the_daily_gate() {
        // The gate sits inside `if (fAutomatic)` (`C4UpdateDlg.cpp:264`), so a
        // user pressing the button always gets a check.
        let last = 1_700_000_000;
        assert!(should_check_for_updates(false, last, last));
    }
}
