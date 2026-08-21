//! Reusable classic `C4DownloadDlg` transfer lifecycle.
//!
//! The application owns the HTTP client. This controller consumes its byte
//! progress/completion events, presents the existing classic progress dialog,
//! and emits the one side effect that belongs to the transport owner: aborting
//! an in-flight request.

use crate::message_dialog::{
    MessageDialogButton, MessageDialogIcon, MessageDialogResult, MessageDialogState,
};
use crate::progress_dialog::ProgressDialogState;

const NET_WAIT_ICON_PHASE: u16 = 3;

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum DownloadTransferEvent {
    ByteProgress {
        downloaded_bytes: u64,
        total_bytes: u64,
    },
    Completed,
    /// The public C++ wrapper formats the URL basename, transport error and
    /// optional not-found suffix before opening its separate error modal.
    Failed {
        display_message: String,
    },
}

/// Side effects that the HTTP-transfer owner must apply.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum DownloadDialogAction {
    AbortTransfer { reason: String },
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum DownloadDialogOutcome {
    Completed,
    Cancelled,
    Failed(String),
}

/// Pure frontend state for one URL download.
#[derive(Clone, Debug)]
pub struct DownloadDialogState {
    caption: String,
    cancel_reason: String,
    transfer_dialog: Option<ProgressDialogState>,
    error_dialog: Option<MessageDialogState>,
    outcome: Option<DownloadDialogOutcome>,
    /// Localized `IDS_PRC_DOWNLOADERROR` ("Error downloading the file %s.|%s.")
    /// and the filename its first argument names. Unset leaves an error
    /// message as the caller composed it.
    error_format: Option<(String, String)>,
    /// Appended after a `'|'` when the composed error mentions 404
    /// (`C4DownloadDlg.cpp:184-189`).
    not_found_message: Option<String>,
}

impl DownloadDialogState {
    /// Constructs a US-English dialog. Hosts with active language resources
    /// should use [`Self::new_localized`].
    pub fn new(message: impl Into<String>, caption: impl Into<String>) -> Self {
        Self::new_localized(message, caption, "Cancel", "User abort")
    }

    /// Constructs a dialog from already-localized strings.
    pub fn new_localized(
        message: impl Into<String>,
        caption: impl Into<String>,
        cancel_label: impl Into<String>,
        cancel_reason: impl Into<String>,
    ) -> Self {
        let caption = caption.into();
        let mut transfer_dialog = ProgressDialogState::new(
            message,
            caption.clone(),
            0,
            MessageDialogIcon::Standard(NET_WAIT_ICON_PHASE),
        );
        transfer_dialog
            .dialog_mut()
            .set_button_label(MessageDialogButton::Cancel, cancel_label);
        // Unlike C4GUI::ProgressDialog, C4DownloadDlg does not install a
        // tooltip on its progress bar.
        transfer_dialog.dialog_mut().set_progress_tooltip("");
        Self {
            caption,
            cancel_reason: cancel_reason.into(),
            transfer_dialog: Some(transfer_dialog),
            error_dialog: None,
            outcome: None,
            error_format: None,
            not_found_message: None,
        }
    }

    /// Installs the localized `IDS_PRC_DOWNLOADERROR` format and the filename
    /// the wrapper names in it (`C4DownloadDlg.cpp:182`).
    #[must_use]
    pub fn with_error_format(
        mut self,
        format: impl Into<String>,
        filename: impl Into<String>,
    ) -> Self {
        self.error_format = Some((format.into(), filename.into()));
        self
    }

    /// Installs the extended message a 404 appends
    /// (`C4DownloadDlg.cpp:184-189`).
    #[must_use]
    pub fn with_not_found_message(mut self, message: impl Into<String>) -> Self {
        self.not_found_message = Some(message.into());
        self
    }

    /// Composes the terminal modal text for an error exactly as
    /// `C4DownloadDlg::DownloadFile` does.
    fn compose_error(&self, error: &str) -> String {
        let mut composed = match self.error_format.as_ref() {
            Some((format, filename)) => format.replacen("%s", filename, 1).replacen("%s", error, 1),
            None => error.to_string(),
        };
        // C++ tests the *composed* string, not the raw error.
        if composed.contains("404") {
            if let Some(message) = self.not_found_message.as_deref() {
                composed.push('|');
                composed.push_str(message);
            }
        }
        composed
    }

    /// Ends the transfer in the download-error modal. Shared by a transport
    /// failure and by a user cancel, which C++ routes through the same wrapper
    /// path with `IDS_ERR_USERCANCEL` as the error text.
    fn open_error_dialog(&mut self, error: &str) {
        self.transfer_dialog = None;
        self.error_dialog = Some(MessageDialogState::regular_ok(
            self.compose_error(error),
            self.caption.clone(),
            MessageDialogIcon::ERROR,
        ));
    }

    /// Fails the transfer with `error`, opening the terminal modal.
    pub fn fail_with_error(&mut self, error: impl AsRef<str>) {
        if self.outcome.is_some() {
            return;
        }
        let error = error.as_ref();
        self.open_error_dialog(error);
        self.outcome = Some(DownloadDialogOutcome::Failed(self.compose_error(error)));
    }

    pub fn transfer_dialog(&self) -> Option<&MessageDialogState> {
        self.transfer_dialog
            .as_ref()
            .map(ProgressDialogState::dialog)
    }

    pub fn transfer_dialog_mut(&mut self) -> Option<&mut MessageDialogState> {
        self.transfer_dialog
            .as_mut()
            .map(ProgressDialogState::dialog_mut)
    }

    pub fn error_dialog(&self) -> Option<&MessageDialogState> {
        self.error_dialog.as_ref()
    }

    pub fn take_error_dialog(&mut self) -> Option<MessageDialogState> {
        self.error_dialog.take()
    }

    pub fn outcome(&self) -> Option<&DownloadDialogOutcome> {
        self.outcome.as_ref()
    }

    /// The displayed percentage, if a bar is showing at all.
    ///
    /// `None` covers both "no transfer dialog" and "a transfer whose length is
    /// unknown", which is what the caller needs to distinguish from a real
    /// zero per cent (clonk-org/clonk-rs#575).
    pub fn progress(&self) -> Option<u8> {
        self.transfer_dialog
            .as_ref()
            .and_then(|transfer| transfer.dialog().progress())
    }

    /// Applies a transport callback. Late callbacks after a terminal event are
    /// deliberately ignored.
    pub fn handle_transfer_event(&mut self, event: DownloadTransferEvent) {
        if self.outcome.is_some() {
            return;
        }
        match event {
            DownloadTransferEvent::ByteProgress {
                downloaded_bytes,
                total_bytes,
            } => self.on_byte_progress(downloaded_bytes, total_bytes),
            DownloadTransferEvent::Completed => {
                self.transfer_dialog = None;
                self.outcome = Some(DownloadDialogOutcome::Completed);
            }
            DownloadTransferEvent::Failed { display_message } => {
                self.open_error_dialog(&display_message);
                self.outcome = Some(DownloadDialogOutcome::Failed(
                    self.compose_error(&display_message),
                ));
            }
        }
    }

    /// Applies the HTTP byte-progress callback using overflow-safe integer
    /// floor division. It matches the C++ result for valid callbacks where
    /// downloaded bytes do not exceed the reported total and safely clamps a
    /// malformed over-total callback.
    pub fn on_byte_progress(&mut self, downloaded_bytes: u64, total_bytes: u64) {
        let Some(dialog) = self.transfer_dialog.as_mut() else {
            return;
        };
        if total_bytes == 0 {
            // A transfer that never reports a total has no percentage to
            // show. C++ leaves the bar out for these rather than drawing one
            // that cannot move; returning early here would instead freeze
            // whatever the last known-length callback left on screen
            // (clonk-org/clonk-rs#575).
            dialog.hide_progress();
            return;
        }
        let percent = ((u128::from(downloaded_bytes) * 100) / u128::from(total_bytes)).min(100);
        dialog.set_progress(percent as u8);
    }

    /// Handles Cancel, Escape, or title-close from the active transfer dialog.
    /// The returned action must be consumed by the transfer owner.
    pub fn handle_transfer_dialog_result(
        &mut self,
        result: MessageDialogResult,
    ) -> Option<DownloadDialogAction> {
        if !matches!(
            result,
            MessageDialogResult::Cancel | MessageDialogResult::Dismissed
        ) || self.transfer_dialog.is_none()
        {
            return None;
        }
        // `UserClose` does not simply dismiss: it fails the transfer with
        // `IDS_ERR_USERCANCEL`, and the wrapper shows the same terminal modal
        // it shows for a transport failure (`C4DownloadDlg.cpp:128-131,167-196`).
        let reason = self.cancel_reason.clone();
        self.open_error_dialog(&reason);
        self.outcome = Some(DownloadDialogOutcome::Cancelled);
        Some(DownloadDialogAction::AbortTransfer { reason })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::message_dialog::{MessageDialogButtons, MessageDialogIcon};
    use crate::test_support::endeavour_font_set;
    use crate::KeyCode;

    fn dialog() -> DownloadDialogState {
        DownloadDialogState::new_localized(
            "Downloading patch.c4u...",
            "Downloading Update",
            "Cancel",
            "User abort",
        )
    }

    /// A transfer whose total length is never reported has no percentage to
    /// show, so the bar is dropped rather than drawn stuck
    /// (clonk-org/clonk-rs#575).
    ///
    /// The interesting case is a transfer that *starts* with a known length
    /// and then reports zero — a server that stops sending `Content-Length`
    /// part way, or a redirect onto a chunked response. Returning early there
    /// left the last real percentage frozen on screen, which reads as a
    /// stalled download rather than an unmeasurable one.
    #[test]
    fn an_unknown_total_length_suppresses_the_progress_bar() {
        let mut state = dialog();
        state.on_byte_progress(25, 100);
        assert_eq!(state.progress(), Some(25), "a known length drives the bar");

        state.on_byte_progress(30, 0);
        assert_eq!(
            state.progress(),
            None,
            "an unknown total drops the bar instead of freezing it at 25"
        );

        // Once suppressed it stays suppressed: `set_progress` deliberately
        // cannot resurrect a bar the layout has stopped reserving space for.
        state.on_byte_progress(60, 100);
        assert_eq!(state.progress(), None);
    }

    #[test]
    fn byte_progress_callback_updates_displayed_percentage() {
        let mut state = dialog();
        let transfer = state.transfer_dialog().expect("active transfer dialog");
        assert_eq!(transfer.message(), "Downloading patch.c4u...");
        assert_eq!(transfer.icon(), MessageDialogIcon::Standard(3));
        assert_eq!(transfer.buttons(), MessageDialogButtons::CANCEL);

        let layout = transfer.layout(800, 600, &endeavour_font_set().text);
        assert!(layout.progress.is_some());
        assert_eq!(layout.buttons.len(), 1);
        assert_eq!(layout.buttons[0].button, MessageDialogButton::Cancel);

        state.handle_transfer_event(DownloadTransferEvent::ByteProgress {
            downloaded_bytes: 101,
            total_bytes: 400,
        });
        assert_eq!(state.progress(), Some(25));

        state.on_byte_progress(u64::MAX, u64::MAX - 1);
        assert_eq!(state.progress(), Some(100));
    }

    #[derive(Default)]
    struct SyntheticTransfer {
        aborted_with: Option<String>,
    }

    impl SyntheticTransfer {
        fn apply(&mut self, action: DownloadDialogAction) {
            let DownloadDialogAction::AbortTransfer { reason } = action;
            self.aborted_with = Some(reason);
        }
    }

    #[test]
    fn cancel_aborts_synthetic_transfer_and_closes_dialog() {
        let mut state = dialog();
        let result = {
            let transfer = state.transfer_dialog_mut().expect("active transfer dialog");
            assert_eq!(transfer.handle_key_down(KeyCode::Tab, false), None);
            assert_eq!(transfer.handle_key_down(KeyCode::Tab, false), None);
            assert_eq!(transfer.handle_key_down(KeyCode::Enter, false), None);
            transfer
                .handle_key_up(KeyCode::Enter)
                .expect("Cancel result")
        };
        assert_eq!(result, MessageDialogResult::Cancel);

        let action = state
            .handle_transfer_dialog_result(result)
            .expect("transport abort action");
        let mut transfer = SyntheticTransfer::default();
        transfer.apply(action);

        assert_eq!(transfer.aborted_with.as_deref(), Some("User abort"));
        assert!(state.transfer_dialog().is_none());
        assert_eq!(state.outcome(), Some(&DownloadDialogOutcome::Cancelled));
        assert!(state
            .handle_transfer_dialog_result(MessageDialogResult::Dismissed)
            .is_none());

        let mut title_closed = dialog();
        let result = title_closed
            .transfer_dialog_mut()
            .expect("active transfer dialog")
            .handle_key_down(KeyCode::Escape, false)
            .expect("title-close result");
        assert_eq!(result, MessageDialogResult::Dismissed);
        let action = title_closed
            .handle_transfer_dialog_result(result)
            .expect("title-close abort action");
        let mut transfer = SyntheticTransfer::default();
        transfer.apply(action);
        assert_eq!(transfer.aborted_with.as_deref(), Some("User abort"));
        assert!(title_closed.transfer_dialog().is_none());
        assert_eq!(
            title_closed.outcome(),
            Some(&DownloadDialogOutcome::Cancelled)
        );
    }

    #[test]
    fn completion_and_error_have_distinct_presentations() {
        let mut completed = dialog();
        completed.handle_transfer_event(DownloadTransferEvent::Completed);
        assert!(completed.transfer_dialog().is_none());
        assert!(completed.error_dialog().is_none());
        assert_eq!(completed.outcome(), Some(&DownloadDialogOutcome::Completed));

        let error = "Error downloading the file patch.c4u.|HTTP 404.";
        let mut failed = dialog();
        failed.handle_transfer_event(DownloadTransferEvent::Failed {
            display_message: error.into(),
        });
        assert!(failed.transfer_dialog().is_none());
        assert_eq!(
            failed.outcome(),
            Some(&DownloadDialogOutcome::Failed(error.into()))
        );
        let error_dialog = failed.error_dialog().expect("separate error dialog");
        assert_eq!(error_dialog.message(), error);
        assert_eq!(error_dialog.caption(), "Downloading Update");
        assert_eq!(error_dialog.icon(), MessageDialogIcon::ERROR);
        assert_eq!(error_dialog.buttons(), MessageDialogButtons::OK);
    }

    /// Cancelling still ends in the download-error modal
    /// (clonk-org/clonk-rs#575).
    ///
    /// This is the part that reads as "nothing happened" if you only follow
    /// the cancel path: `UserClose` does not close the dialog with a result,
    /// it *fails the transfer* with `IDS_ERR_USERCANCEL` as the error text
    /// (`C4DownloadDlg.cpp:128-131`). `ShowModal` then returns false and
    /// `DownloadFile` shows the same terminal modal it shows for a transport
    /// failure, with `IDS_PRC_DOWNLOADERROR` naming the file and the error
    /// (`:167-196`).
    ///
    /// So a user cancel and a network failure are the same shape to the
    /// player; only the error text differs. A port that treated Cancel as a
    /// plain dismissal would silently swallow the outcome.
    #[test]
    fn cancelling_ends_in_the_download_error_modal_naming_the_user_abort() {
        let mut dialog = DownloadDialogState::new_localized(
            "Downloading patch.c4u...",
            "Downloading Update",
            "Cancel",
            "User abort",
        )
        // IDS_PRC_DOWNLOADERROR, already localized, with the filename the
        // wrapper passes as its first argument.
        .with_error_format("Error downloading the file %s.|%s.", "patch.c4u");

        let action = dialog
            .handle_transfer_dialog_result(MessageDialogResult::Cancel)
            .expect("cancel must abort the transfer");
        assert_eq!(
            action,
            DownloadDialogAction::AbortTransfer {
                reason: "User abort".to_string()
            }
        );
        assert!(
            dialog.transfer_dialog().is_none(),
            "the progress dialog closes first"
        );
        assert_eq!(
            dialog.outcome(),
            Some(&DownloadDialogOutcome::Cancelled),
            "the outcome stays Cancelled — it is not reported as a transport failure"
        );

        let error = dialog
            .error_dialog()
            .expect("cancel ends in the same terminal modal as a failure");
        assert_eq!(
            error.message(),
            "Error downloading the file patch.c4u.|User abort."
        );
        assert_eq!(error.caption(), "Downloading Update");
        assert_eq!(error.icon(), MessageDialogIcon::ERROR);
        assert_eq!(error.buttons(), MessageDialogButtons::OK);
    }

    /// A 404 appends the caller's not-found message on its own line
    /// (clonk-org/clonk-rs#575).
    ///
    /// `DownloadFile` tests the *composed* error for `"404"` and appends
    /// `'|'` plus the supplied message (`C4DownloadDlg.cpp:184-189`). The
    /// separator is the classic line break, not a space, so the extra
    /// sentence lands on its own line in the modal.
    #[test]
    fn a_404_failure_appends_the_not_found_message_on_its_own_line() {
        let build = |error: &str, not_found: Option<&str>| {
            let mut dialog = DownloadDialogState::new_localized(
                "Downloading patch.c4u...",
                "Downloading Update",
                "Cancel",
                "User abort",
            )
            .with_error_format("Error downloading the file %s.|%s.", "patch.c4u");
            if let Some(message) = not_found {
                dialog = dialog.with_not_found_message(message);
            }
            dialog.fail_with_error(error);
            dialog
                .error_dialog()
                .expect("a failure opens the modal")
                .message()
                .to_string()
        };

        assert_eq!(
            build("404 Not Found", Some("Check the update server.")),
            "Error downloading the file patch.c4u.|404 Not Found.|Check the update server.",
        );
        // No 404 in the text: the extra message is not appended.
        assert_eq!(
            build("Connection reset", Some("Check the update server.")),
            "Error downloading the file patch.c4u.|Connection reset.",
        );
        // A 404 without a supplied message stays as it is.
        assert_eq!(
            build("404 Not Found", None),
            "Error downloading the file patch.c4u.|404 Not Found.",
        );
    }
}
