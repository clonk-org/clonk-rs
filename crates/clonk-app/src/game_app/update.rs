//! `impl GameApp` — the in-app update check.
//!
//! Ports `C4UpdateDlg::CheckForUpdates` (`C4UpdateDlg.cpp:262-405`) and the
//! three entry points `C4StartupMainDlg::OnShown` drives it from
//! (`C4StartupMainDlg.cpp:257-277`): an incoming package, a check requested on
//! the command line, and the once-a-day automatic check.
//!
//! Two deliberate divergences from C++ are recorded here rather than in a
//! comment somewhere downstream:
//!
//! * **An incoming `.c4u` is refused, never applied.** C++'s `ApplyUpdate`
//!   (`C4UpdateDlg.cpp:171-215`) opens the group, extracts the update program
//!   out of it and executes that program — running an executable that arrived
//!   as a file argument. This port will not do that, so a handed-in package is
//!   reported with `IDS_MSG_UPDATEFAILED`.
//! * **An engine change is not "no update available".** C++ hides an engine
//!   mismatch behind `IsValidUpdate` and reports it as nothing to do
//!   (`C4UpdateDlg.cpp:248`). Here it gets its own message, because the release
//!   does exist and the user has to install it by hand.
//! * **A finished check names the version it found rather than denying one.**
//!   `IDS_MSG_NOUPDATEAVAILABLEFORTHISV` ("No update available for this
//!   version") reads as *an update exists, just not for you*, which is the
//!   message above and not this one, so the up-to-date case states the running
//!   version instead (`IDS_MSG_UPTODATE`).

use super::*;
use crate::update_check::{spawn_update_check, PendingUpdateCheck, UpdateCheckOutcome};
use crate::update_download::{
    launch_update_applier, spawn_update_download, total_component_size, UpdateDownloadEvent,
};
use clonk_update::should_check_for_updates;
use clonk_update_net::DEFAULT_UPDATE_BASE_URL;

/// `C4GUI::Ico_Ex_Update`, the GUIIcons2 phase every update dialog uses
/// (`C4UpdateDlg.cpp:277`, `:384`).
const UPDATE_ICON: clonk_frontend::message_dialog::MessageDialogIcon =
    clonk_frontend::message_dialog::MessageDialogIcon::Extended(14);

/// The LegacyClonk version server.
///
/// It answers `?action=version` with an INI document, which is not the manifest
/// this port fetches, so a config still carrying it is a stale default rather
/// than a mirror somebody chose. Anything *else* is honoured, which is what
/// makes `Network.UpdateServerAddress` usable to point a build at a mirror.
const LEGACY_UPDATE_SERVER: &str = "update.clonkspot.org";

impl GameApp {
    /// Where this build looks for a manifest.
    pub(crate) fn update_server_address(&self) -> String {
        self.app_paths
            .as_ref()
            .and_then(|paths| Config::load(paths.config_file()).ok())
            .and_then(|config| {
                config
                    .get_in(Some("Network"), "UpdateServerAddress")
                    .map(str::trim)
                    .filter(|address| {
                        !address.is_empty() && !address.contains(LEGACY_UPDATE_SERVER)
                    })
                    .map(str::to_string)
            })
            .unwrap_or_else(|| DEFAULT_UPDATE_BASE_URL.to_string())
    }

    /// The caption every dialog the check raises carries.
    ///
    /// C++ titles all of them with `Config.Network.UpdateServerAddress`
    /// (`C4UpdateDlg.cpp:277`, `:317`, `:384`, `:399`). There that was a bare
    /// hostname; here the default is a full release-download URL, which is a
    /// title bar's worth of address and names no command. The button that
    /// opened the dialog does, so it supplies the caption instead.
    pub(crate) fn update_check_caption(&self) -> String {
        self.runtime_resource_text("IDS_DLG_CHECKFORUPDATES", "Check for Updates")
    }

    /// `Config.Network.AutomaticUpdate` (`C4StartupMainDlg.cpp:273`), which
    /// this port spells `EnableAutomaticUpdate` and defaults to on.
    pub(crate) fn automatic_update_enabled(&self) -> bool {
        self.app_paths
            .as_ref()
            .and_then(|paths| Config::load(paths.config_file()).ok())
            .and_then(|config| {
                config
                    .get_in(Some("Network"), "EnableAutomaticUpdate")
                    .map(parse_config_bool)
            })
            .unwrap_or(true)
    }

    fn last_update_time(&self) -> i64 {
        self.app_paths
            .as_ref()
            .and_then(|paths| Config::load(paths.config_file()).ok())
            .and_then(|config| {
                config
                    .get_in(Some("Network"), "LastUpdateTime")
                    .and_then(|value| value.trim().parse::<i64>().ok())
            })
            .unwrap_or(0)
    }

    /// Stores the time of this check, successful or not, exactly as
    /// `C4UpdateDlg.cpp:266-267` does before it queries anything.
    fn store_last_update_time(&mut self, now: i64) {
        let Some(paths) = self.app_paths.as_ref() else {
            return;
        };
        let path = paths.config_file();
        let stored = Config::load(&path).map(|mut config| {
            config.set_in(Some("Network"), "LastUpdateTime", now.to_string());
            (config, path.clone())
        });
        match stored {
            Ok((config, path)) => {
                if let Err(error) =
                    save_config_preserving_native_general_booleans(&config, &path, None, None)
                {
                    tracing::warn!(%error, "failed to store the update check time");
                }
            }
            Err(error) => {
                tracing::warn!(%error, "failed to read the config before an update check")
            }
        }
    }

    /// `C4UpdateDlg::CheckForUpdates` (`C4UpdateDlg.cpp:262`).
    ///
    /// An automatic check is throttled to once a day and stays silent when it
    /// finds nothing; a manual one ignores the throttle and always reports.
    pub(crate) fn check_for_updates(&mut self, automatic: bool) -> Result<(), EngineError> {
        self.check_for_updates_at(
            automatic,
            i64::try_from(current_unix_timestamp()).unwrap_or(0),
        )
    }

    /// [`Self::check_for_updates`] against an explicit clock, so the daily gate
    /// is testable without waiting a day.
    pub(crate) fn check_for_updates_at(
        &mut self,
        automatic: bool,
        now: i64,
    ) -> Result<(), EngineError> {
        if self.update_check.is_some() || self.update_download.is_some() {
            // C++ cannot reach this: its check blocks the message loop. Here a
            // second request while one is in flight is possible, and the honest
            // answer is the one the resource table already ships.
            return match automatic {
                true => Ok(()),
                false => self.show_update_notice(
                    self.runtime_resource_text(
                        "IDS_MSG_UPDATEINPROGRESS",
                        "Update still in progress. Please wait.",
                    ),
                    self.update_check_caption(),
                ),
            };
        }
        if !should_check_for_updates(automatic, now, self.last_update_time()) {
            return Ok(());
        }
        self.store_last_update_time(now);

        let server = self.update_server_address();
        let install_root = self
            .app_paths
            .as_ref()
            .map(|paths| paths.install_root().to_path_buf());
        self.update_check = Some(PendingUpdateCheck {
            receiver: spawn_update_check(server, install_root),
            automatic,
        });
        self.push_message_dialog(
            clonk_frontend::message_dialog::MessageDialogState::new(
                self.runtime_resource_text("IDS_MSG_LOOKINGFORUPDATES", "Checking for updates..."),
                self.update_check_caption(),
                clonk_frontend::message_dialog::MessageDialogButtons::CANCEL,
                UPDATE_ICON,
                clonk_frontend::message_dialog::MessageDialogSize::Regular,
                false,
            ),
            MessageDialogContinuation::UpdateCheckWait,
        )
    }

    /// Runs a check on the calling thread against a supplied transport.
    ///
    /// The seam tests use: the presentation below is identical either way, and
    /// only the worker thread is skipped.
    #[cfg(test)]
    pub(crate) fn check_for_updates_with(
        &mut self,
        automatic: bool,
        transport: &dyn clonk_update_net::UpdateTransport,
    ) -> Result<(), EngineError> {
        let server = self.update_server_address();
        let install_root = self
            .app_paths
            .as_ref()
            .map(|paths| paths.install_root().to_path_buf());
        let outcome = crate::update_check::check_for_updates(
            transport,
            &server,
            install_root.as_deref(),
            crate::update_check::TARGET_TRIPLE,
        );
        self.finish_update_check(outcome, automatic)
    }

    /// Drains a finished check. Called from the ordinary application pass, so
    /// the wait dialog stays interactive while the worker runs.
    pub(crate) fn poll_update_check(&mut self) -> Result<(), EngineError> {
        let Some(pending) = self.update_check.as_ref() else {
            return Ok(());
        };
        let outcome = match pending.receiver.try_recv() {
            Ok(outcome) => outcome,
            Err(mpsc::TryRecvError::Empty) => return Ok(()),
            // The worker died without answering, which is a failed check
            // rather than a wait that never ends.
            Err(mpsc::TryRecvError::Disconnected) => UpdateCheckOutcome::Failed {
                detail: "the update check ended without a result".to_string(),
            },
        };
        let Some(pending) = self.update_check.take() else {
            return Ok(());
        };
        self.finish_update_check(outcome, pending.automatic)
    }

    /// Abandons the check behind the wait dialog the user just closed
    /// (`C4UpdateDlg.cpp:294-296`, which treats a closed `pWaitDlg` as an
    /// abort and reports nothing).
    pub(crate) fn abort_update_check(&mut self) {
        // Dropping the receiver is the whole cancellation: the worker's send
        // fails, it exits, and its verdict is never presented. The request
        // already in flight is left to time out on its own.
        self.update_check = None;
    }

    fn finish_update_check(
        &mut self,
        outcome: UpdateCheckOutcome,
        automatic: bool,
    ) -> Result<(), EngineError> {
        self.update_check = None;
        self.close_update_check_wait_dialog()?;
        let caption = self.update_check_caption();
        match outcome {
            UpdateCheckOutcome::Available {
                manifest_base_url,
                version,
                components,
            } => {
                let total = total_component_size(&components);
                tracing::info!(
                    %version,
                    components = components.len(),
                    bytes = total,
                    "an update is available"
                );
                let message = format_resource_string(
                    self.runtime_resource_text(
                        "IDS_MSG_ANUPDATETOVERSIONISAVAILA",
                        "An update to version %s is available. \
                         Do you want to download and install this update?",
                    ),
                    &[&version],
                );
                self.push_message_dialog(
                    clonk_frontend::message_dialog::MessageDialogState::new(
                        message,
                        caption,
                        clonk_frontend::message_dialog::MessageDialogButtons::YES_NO,
                        UPDATE_ICON,
                        clonk_frontend::message_dialog::MessageDialogSize::Regular,
                        false,
                    ),
                    MessageDialogContinuation::UpdatePrompt {
                        manifest_base_url,
                        version,
                        components,
                    },
                )
            }
            // `C4UpdateDlg.cpp:396-400`: an automatic check that finds nothing
            // says nothing.
            UpdateCheckOutcome::UpToDate if automatic => Ok(()),
            UpdateCheckOutcome::UpToDate => {
                let message = format_resource_string(
                    self.runtime_resource_text(
                        "IDS_MSG_UPTODATE",
                        "Clonk Rust %s is the latest version.",
                    ),
                    &[clonk_core::version::PORT_VERSION],
                );
                self.show_update_notice(message, caption)
            }
            UpdateCheckOutcome::EngineChanged { version } => {
                self.show_manual_install_notice(&version, caption)
            }
            UpdateCheckOutcome::Failed { detail } => {
                tracing::warn!(%detail, "the update check failed");
                let message = format!(
                    "{}: {detail}",
                    self.runtime_resource_text("IDS_MSG_UPDATEFAILED", "Update failed.")
                );
                self.show_update_notice(message, caption)
            }
        }
    }

    /// Starts the component transfer the user accepted in the update prompt.
    pub(crate) fn start_update_download(
        &mut self,
        manifest_base_url: String,
        version: String,
        components: Vec<clonk_update::PlannedComponent>,
    ) -> Result<(), EngineError> {
        let caption = self.update_check_caption();
        let Some(paths) = self.app_paths.clone() else {
            return self.show_update_notice(
                format!(
                    "{}: the installation directory could not be located",
                    self.runtime_resource_text("IDS_MSG_UPDATEFAILED", "Update failed.")
                ),
                caption,
            );
        };
        self.update_download = Some(spawn_update_download(
            manifest_base_url,
            version.clone(),
            components,
            paths,
        ));
        let message = format_resource_string(
            self.runtime_resource_text("IDS_MSG_DOWNLOADINGUPDATE", "Downloading update %s..."),
            &[&version],
        );
        self.push_message_dialog(
            clonk_frontend::message_dialog::MessageDialogState::new(
                message,
                caption,
                clonk_frontend::message_dialog::MessageDialogButtons::CANCEL,
                UPDATE_ICON,
                clonk_frontend::message_dialog::MessageDialogSize::Regular,
                false,
            )
            .with_progress(0),
            MessageDialogContinuation::UpdateDownloadWait,
        )
    }

    /// Applies download progress and launches the detached applier once every
    /// component has passed its size and digest checks.
    pub(crate) fn poll_update_download(&mut self) -> Result<(), EngineError> {
        let (progress, terminal) = {
            let Some(pending) = self.update_download.as_ref() else {
                return Ok(());
            };
            let mut progress = None;
            let mut terminal = None;
            loop {
                match pending.receiver.try_recv() {
                    Ok(UpdateDownloadEvent::Progress { downloaded, total }) => {
                        progress = Some((downloaded, total));
                    }
                    Ok(event @ UpdateDownloadEvent::Prepared { .. })
                    | Ok(event @ UpdateDownloadEvent::Failed { .. }) => {
                        terminal = Some(event);
                        break;
                    }
                    Err(mpsc::TryRecvError::Empty) => break,
                    Err(mpsc::TryRecvError::Disconnected) => {
                        terminal = Some(UpdateDownloadEvent::Failed {
                            detail: "the update download ended without a result".to_string(),
                        });
                        break;
                    }
                }
            }
            (progress, terminal)
        };
        if let Some((downloaded, total)) = progress {
            let percent = if total == 0 {
                0
            } else {
                ((u128::from(downloaded) * 100) / u128::from(total)).min(100) as u8
            };
            self.update_update_download_progress(percent);
        }
        let Some(terminal) = terminal else {
            return Ok(());
        };
        self.update_download = None;
        self.close_update_download_dialog();
        match terminal {
            UpdateDownloadEvent::Prepared { update } => {
                let launched = self
                    .app_paths
                    .as_ref()
                    .context("the installation directory disappeared before update apply")
                    .and_then(|paths| launch_update_applier(paths, update.plan_path()));
                match launched {
                    Ok(()) => {
                        update.hand_off();
                        self.request_exit();
                    }
                    Err(error) => {
                        let message = format!(
                            "{}: {error:#}",
                            self.runtime_resource_text("IDS_MSG_UPDATEFAILED", "Update failed.")
                        );
                        self.show_update_notice(message, self.update_check_caption())?;
                    }
                }
            }
            UpdateDownloadEvent::Failed { detail } => {
                tracing::warn!(%detail, "the update download failed");
                let message = format!(
                    "{}: {detail}",
                    self.runtime_resource_text("IDS_MSG_UPDATEFAILED", "Update failed.")
                );
                self.show_update_notice(message, self.update_check_caption())?;
            }
            UpdateDownloadEvent::Progress { .. } => {}
        }
        Ok(())
    }

    fn update_update_download_progress(&mut self, percent: u8) {
        if let Some(dialog) = self.message_dialogs.iter_mut().find(|dialog| {
            matches!(
                dialog.continuation,
                MessageDialogContinuation::UpdateDownloadWait
            )
        }) {
            dialog.state.set_progress(percent);
        }
    }

    fn close_update_download_dialog(&mut self) {
        let Some(index) = self.message_dialogs.iter().position(|dialog| {
            matches!(
                dialog.continuation,
                MessageDialogContinuation::UpdateDownloadWait
            )
        }) else {
            return;
        };
        self.remove_message_dialog_at(index);
    }

    pub(crate) fn abort_update_download(&mut self) {
        if let Some(pending) = self.update_download.take() {
            pending.cancel();
        }
    }

    /// The release cannot be installed from inside the game.
    ///
    /// The caller is a release built against another engine tuple, whose
    /// content cannot safely be installed under the running engine.
    pub(crate) fn show_manual_install_notice(
        &mut self,
        version: &str,
        caption: String,
    ) -> Result<(), EngineError> {
        let message = format_resource_string(
            self.runtime_resource_text(
                "IDS_MSG_UPDATEINSTALLMANUALLY",
                "Version %s cannot be installed from within the game. \
                 Please install it manually.",
            ),
            &[version],
        );
        self.show_update_notice(message, caption)
    }

    /// `C4GUI::Screen::ShowMessage` with the update icon: one OK button, no
    /// continuation (`C4UpdateDlg.cpp:317`, `:388`, `:399`).
    pub(crate) fn show_update_notice(
        &mut self,
        message: String,
        caption: String,
    ) -> Result<(), EngineError> {
        self.push_message_dialog(
            clonk_frontend::message_dialog::MessageDialogState::regular_ok(
                message,
                caption,
                UPDATE_ICON,
            ),
            MessageDialogContinuation::UpdateNotice,
        )
    }

    fn close_update_check_wait_dialog(&mut self) -> Result<(), EngineError> {
        let Some(index) = self.message_dialogs.iter().position(|dialog| {
            matches!(
                dialog.continuation,
                MessageDialogContinuation::UpdateCheckWait
            )
        }) else {
            return Ok(());
        };
        // Removed rather than finished: `finish_message_dialog_at` would route
        // straight back into `abort_update_check`.
        self.remove_message_dialog_at(index);
        Ok(())
    }

    /// A `.c4u` handed to the process on the command line.
    ///
    /// C++ applies it (`C4StartupMainDlg.cpp:259-263`). This port refuses it:
    /// see the module documentation.
    pub(crate) fn refuse_incoming_update(&mut self, package: &Path) -> Result<(), EngineError> {
        tracing::warn!(
            package = %package.display(),
            "refusing an incoming update package: this build never runs a program out of one"
        );
        let caption = self.runtime_resource_text("IDS_TYPE_UPDATE", "Update");
        let message = self.runtime_resource_text("IDS_MSG_UPDATEFAILED", "Update failed.");
        self.show_update_notice(message, caption)
    }
}
