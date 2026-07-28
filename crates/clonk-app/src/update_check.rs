//! Looking for a published update, off the main thread.
//!
//! This is the query half of `C4UpdateDlg::CheckForUpdates`
//! (`C4UpdateDlg.cpp:262-345`): fetch what the server publishes, compare it
//! against this install, and report one verdict. Presenting that verdict is the
//! caller's job, so nothing here touches a dialog.
//!
//! The comparison itself lives in `clonk-update`; this module only supplies the
//! three things that are the *application's* knowledge — where to fetch from,
//! which install is being compared, and which target triple this binary is.

use clonk_update::{decide_for_this_build, Decision, InstalledState, Manifest, RefusalReason};
#[cfg(not(test))]
use clonk_update_net::HttpTransport;
use clonk_update_net::{manifest_url, UpdateTransport};
use std::path::{Path, PathBuf};
use std::sync::mpsc::{self, Receiver};

/// The triple this binary was built for, captured by `build.rs`.
///
/// A manifest keys every component by triple, so a build that cannot name its
/// own triple would match nothing and silently report that no update exists.
pub(crate) const TARGET_TRIPLE: &str = env!("CLONK_TARGET_TRIPLE");

/// What a completed check found.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum UpdateCheckOutcome {
    /// A newer release this build can install, and what it would fetch.
    Available {
        version: String,
        components: Vec<clonk_update::PlannedComponent>,
    },
    /// Nothing to do — `C4UpdateDlg::IsValidUpdate` returning false
    /// (`C4UpdateDlg.cpp:246-260`).
    UpToDate,
    /// A release built against a different engine tuple.
    ///
    /// Held apart from [`Self::UpToDate`] deliberately: an engine bump is not
    /// "no update available", it is an update that this port cannot perform in
    /// place, because `clonk-core::version` prunes definitions that declare a
    /// newer engine and installing the offered content would delete the game's
    /// definitions rather than fail loudly.
    EngineChanged { version: String },
    /// The check did not produce a verdict — `C4UpdateDlg.cpp:308-322`, which
    /// shows `IDS_MSG_UPDATEFAILED` followed by the transport's own message.
    Failed { detail: String },
}

/// One check the application is waiting on.
pub(crate) struct PendingUpdateCheck {
    pub(crate) receiver: Receiver<UpdateCheckOutcome>,
    /// C++'s `fAutomatic`: it decides only whether "no update available" is
    /// worth a dialog (`C4UpdateDlg.cpp:396-400`).
    pub(crate) automatic: bool,
    /// `Config.Network.UpdateServerAddress`, which C++ uses as the caption of
    /// every dialog this check raises.
    pub(crate) server: String,
}

/// Runs one check against an already-built transport.
///
/// Separate from [`spawn_update_check`] so the whole decision path is testable
/// without a thread or a network.
pub(crate) fn check_for_updates(
    transport: &dyn UpdateTransport,
    base_url: &str,
    install_root: Option<&Path>,
    target_triple: &str,
) -> UpdateCheckOutcome {
    let failed = |detail: String| UpdateCheckOutcome::Failed { detail };

    let bytes = match transport.fetch_manifest(&manifest_url(base_url)) {
        Ok(bytes) => bytes,
        Err(error) => return failed(error.to_string()),
    };
    let manifest = match Manifest::parse(&bytes) {
        Ok(manifest) => manifest,
        Err(error) => return failed(error.to_string()),
    };
    // An install that predates the updater has no state file, which is absence
    // rather than failure; an unreadable one is reported, because treating it
    // as absence would silently re-download every component forever.
    let installed = match install_root.map(InstalledState::load).transpose() {
        Ok(state) => state.flatten(),
        Err(error) => return failed(error.to_string()),
    };

    match decide_for_this_build(&manifest, &installed, target_triple) {
        Decision::Update {
            version,
            components,
        } => UpdateCheckOutcome::Available {
            version,
            components,
        },
        Decision::UpToDate => UpdateCheckOutcome::UpToDate,
        Decision::Refused {
            reason: RefusalReason::EngineMismatch { .. },
        } => UpdateCheckOutcome::EngineChanged {
            version: manifest.version,
        },
        Decision::Refused { reason } => failed(reason.to_string()),
    }
}

/// Starts a check on its own thread, reporting the single verdict over a
/// channel the caller polls.
///
/// The thread owns the blocking transport, which builds and drops a
/// current-thread `tokio` runtime per request; that is only sound off an async
/// context, which is exactly what a plain worker thread is.
#[cfg(not(test))]
pub(crate) fn spawn_update_check(
    base_url: String,
    install_root: Option<PathBuf>,
) -> Receiver<UpdateCheckOutcome> {
    let (sender, receiver) = mpsc::channel();
    std::thread::spawn(move || {
        let outcome = match HttpTransport::new() {
            Ok(transport) => check_for_updates(
                &transport,
                &base_url,
                install_root.as_deref(),
                TARGET_TRIPLE,
            ),
            Err(error) => UpdateCheckOutcome::Failed {
                detail: error.to_string(),
            },
        };
        // A dropped receiver is an aborted check, not a failure.
        let _ = sender.send(outcome);
    });
    receiver
}

/// The test build's transport cut-off.
///
/// No test may reach the network, so no worker is started and the check simply
/// never answers — which is what a slow server looks like, and leaves the wait
/// dialog exactly as a test wants to find it. The sender is parked rather than
/// dropped so the caller sees a pending check instead of a disconnected one.
/// Tests that want a verdict supply their own transport to
/// [`check_for_updates`].
#[cfg(test)]
pub(crate) fn spawn_update_check(
    _base_url: String,
    _install_root: Option<PathBuf>,
) -> Receiver<UpdateCheckOutcome> {
    use std::sync::mpsc::Sender;
    use std::sync::Mutex;

    static PARKED: Mutex<Vec<Sender<UpdateCheckOutcome>>> = Mutex::new(Vec::new());

    let (sender, receiver) = mpsc::channel();
    if let Ok(mut parked) = PARKED.lock() {
        parked.push(sender);
    }
    receiver
}

/// Fakes shared by this module's tests and the application-level ones.
#[cfg(test)]
pub(crate) mod test_support {
    use super::*;
    use clonk_update_net::TransportError;

    /// A transport that answers from memory, so every branch is decided by the
    /// manifest rather than by a network.
    pub(crate) struct FakeTransport {
        manifest: Result<Vec<u8>, String>,
    }

    impl FakeTransport {
        pub(crate) fn serving(manifest: &str) -> Self {
            Self {
                manifest: Ok(manifest.as_bytes().to_vec()),
            }
        }

        pub(crate) fn failing(detail: &str) -> Self {
            Self {
                manifest: Err(detail.to_string()),
            }
        }
    }

    impl UpdateTransport for FakeTransport {
        fn fetch_manifest(&self, _url: &str) -> Result<Vec<u8>, TransportError> {
            self.manifest
                .clone()
                .map_err(|detail| TransportError::Status {
                    url: detail,
                    status: 503,
                })
        }

        fn download(
            &self,
            _url: &str,
            _into: &Path,
            _progress: &mut dyn FnMut(u64, u64) -> bool,
        ) -> Result<u64, TransportError> {
            unreachable!("a check never downloads a component")
        }
    }

    /// A manifest offering one `content` archive for every supported triple,
    /// so a test never depends on which host it runs on.
    pub(crate) fn manifest_for(version: &str, engine: [i32; 5]) -> String {
        let [a, b, c, d, e] = engine;
        let targets = [
            ("x86_64-unknown-linux-gnu", "content"),
            ("x86_64-pc-windows-gnu", "content"),
            ("aarch64-apple-darwin", "Contents/Resources/content"),
            ("x86_64-apple-darwin", "Contents/Resources/content"),
        ]
        .map(|(triple, install)| {
            format!(
                r#""{triple}": {{
                  "archive": "content-{version}.zip",
                  "sha256": "{}",
                  "size": 1024,
                  "install": "{install}"
                }}"#,
                "cc".repeat(32)
            )
        })
        .join(",\n");
        format!(
            r#"{{
              "schema": 1,
              "version": "{version}",
              "engine_version": [{a}, {b}, {c}, {d}, {e}],
              "released_at": "2026-07-28T10:00:00Z",
              "components": [
                {{ "name": "content", "targets": {{ {targets} }} }}
              ]
            }}"#
        )
    }

    /// A release far enough ahead of any port version this build could carry
    /// that a test never goes stale as the port version moves.
    pub(crate) const OFFERED_VERSION: &str = "99.0.0";
}

#[cfg(test)]
mod tests {
    use super::test_support::{manifest_for, FakeTransport, OFFERED_VERSION};
    use super::*;

    const MACOS: &str = "aarch64-apple-darwin";

    fn next_port_version() -> String {
        OFFERED_VERSION.to_string()
    }

    #[test]
    fn a_newer_release_for_this_platform_is_offered() {
        let version = next_port_version();
        let transport =
            FakeTransport::serving(&manifest_for(&version, clonk_core::version::ENGINE_VERSION));

        let outcome = check_for_updates(&transport, "https://example.invalid/u", None, MACOS);

        let UpdateCheckOutcome::Available {
            version: offered,
            components,
        } = outcome
        else {
            panic!("a newer release must be offered: {outcome:?}");
        };
        assert_eq!(offered, version);
        assert_eq!(components.len(), 1);
        assert_eq!(components[0].name, "content");
    }

    #[test]
    fn a_release_needing_another_engine_is_not_reported_as_having_no_update() {
        // `IDS_MSG_NOUPDATEAVAILABLEFORTHISV` would be a lie here: the release
        // exists and is newer, it just cannot be installed in place.
        let version = next_port_version();
        let [major, minor, objects, revision, build] = clonk_core::version::ENGINE_VERSION;
        let transport = FakeTransport::serving(&manifest_for(
            &version,
            [major, minor, objects + 1, revision, build],
        ));

        assert_eq!(
            check_for_updates(&transport, "https://example.invalid/u", None, MACOS),
            UpdateCheckOutcome::EngineChanged { version }
        );
    }

    #[test]
    fn a_release_this_build_already_has_is_up_to_date() {
        let transport = FakeTransport::serving(&manifest_for(
            clonk_core::version::PORT_VERSION,
            clonk_core::version::ENGINE_VERSION,
        ));

        assert_eq!(
            check_for_updates(&transport, "https://example.invalid/u", None, MACOS),
            UpdateCheckOutcome::UpToDate
        );
    }

    #[test]
    fn a_release_without_an_archive_for_this_platform_is_up_to_date() {
        // An absent target is a release that did not build for us, which is
        // nothing to report rather than a broken manifest.
        let transport = FakeTransport::serving(&manifest_for(
            &next_port_version(),
            clonk_core::version::ENGINE_VERSION,
        ));

        assert_eq!(
            check_for_updates(
                &transport,
                "https://example.invalid/u",
                None,
                "riscv64gc-unknown-linux-gnu"
            ),
            UpdateCheckOutcome::UpToDate
        );
    }

    #[test]
    fn a_transport_failure_is_reported_rather_than_read_as_up_to_date() {
        let transport = FakeTransport::failing("https://example.invalid/u/manifest.json");

        let UpdateCheckOutcome::Failed { detail } =
            check_for_updates(&transport, "https://example.invalid/u", None, MACOS)
        else {
            panic!("a failed fetch must not look like a successful check");
        };
        assert!(
            detail.contains("503"),
            "the transport's own message must survive: {detail}"
        );
    }

    #[test]
    fn an_unreadable_manifest_is_a_failure() {
        let transport = FakeTransport::serving("{ not json");

        assert!(matches!(
            check_for_updates(&transport, "https://example.invalid/u", None, MACOS),
            UpdateCheckOutcome::Failed { .. }
        ));
    }

    #[test]
    fn this_build_knows_which_target_triple_it_is() {
        // Without it every component lookup misses and every check would
        // answer "no update available" forever.
        assert!(TARGET_TRIPLE.contains('-'), "{TARGET_TRIPLE:?}");
    }
}
