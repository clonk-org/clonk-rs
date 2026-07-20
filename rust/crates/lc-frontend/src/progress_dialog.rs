//! Reusable classic `C4GUI::ProgressDialog` construction.
//!
//! The retained input and dialog furniture live in `message_dialog`; this
//! wrapper selects the native progress layout, installs the cancel button and
//! leaves the initial default Enter action inert (`ProgressDialog::OnEnter`).

use crate::message_dialog::{
    MessageDialogButtons, MessageDialogIcon, MessageDialogSize, MessageDialogState,
};

/// Pure frontend state for a classic percent-progress dialog with Cancel.
#[derive(Clone, Debug)]
pub struct ProgressDialogState {
    dialog: MessageDialogState,
}

impl ProgressDialogState {
    pub fn new(
        message: impl Into<String>,
        caption: impl Into<String>,
        progress: u8,
        icon: MessageDialogIcon,
    ) -> Self {
        Self {
            dialog: MessageDialogState::new(
                message,
                caption,
                MessageDialogButtons::CANCEL,
                icon,
                MessageDialogSize::Regular,
                false,
            )
            .with_progress(progress)
            .without_focus(),
        }
    }

    pub fn progress(&self) -> u8 {
        self.dialog.progress().unwrap_or_default()
    }

    pub fn set_progress(&mut self, progress: u8) {
        self.dialog.set_progress(progress);
    }

    pub fn dialog(&self) -> &MessageDialogState {
        &self.dialog
    }

    pub fn dialog_mut(&mut self) -> &mut MessageDialogState {
        &mut self.dialog
    }

    pub fn into_message_dialog(self) -> MessageDialogState {
        self.dialog
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::message_dialog::{MessageDialogButton, MessageDialogResult};
    use crate::test_support::endeavour_font_set;
    use crate::{GuiPoint, KeyCode};

    #[test]
    fn progress_tracks_percent_and_enter_does_not_abort() {
        let mut progress = ProgressDialogState::new(
            "Waiting for Scenario...",
            "Network",
            17,
            MessageDialogIcon::Standard(3),
        );
        assert_eq!(progress.progress(), 17);

        progress.set_progress(63);
        assert_eq!(progress.progress(), 63);
        assert_eq!(
            progress.dialog_mut().handle_key_down(KeyCode::Enter, false),
            None
        );
        assert_eq!(progress.dialog_mut().handle_key_up(KeyCode::Enter), None);

        let fonts = endeavour_font_set();
        let layout = progress.dialog().layout(800, 600, &fonts.text);
        let bounds = layout.bounds;
        let progress_bar = layout.progress.expect("progress bar");
        assert_eq!(bounds.h, 190);
        assert_eq!(progress_bar.h, 30);
        assert_eq!(progress_bar.y, bounds.y + 90);
        assert_eq!(layout.message.x + layout.message.w / 2, bounds.x + 280);
        assert_eq!(layout.buttons.len(), 1);
        assert_eq!(layout.buttons[0].button, MessageDialogButton::Cancel);
        assert_eq!(layout.buttons[0].rect.w, 140);
        assert_eq!(layout.buttons[0].rect.y, bounds.y + 144);
        let progress_point = GuiPoint::new(
            (progress_bar.x + 1) as f32,
            (progress_bar.y + 1) as f32,
        );
        progress
            .dialog_mut()
            .handle_pointer_move(progress_point, &layout);
        assert_eq!(
            progress
                .dialog()
                .tooltip_state(Some(progress_point), &layout)
                .expect("progress tooltip")
                .text,
            "Progress bar"
        );

        assert_eq!(
            progress.dialog_mut().handle_key_down(KeyCode::Space, false),
            None
        );
        assert_eq!(progress.dialog_mut().handle_key_up(KeyCode::Space), None);
        assert_eq!(
            progress.dialog_mut().handle_key_down(KeyCode::Tab, false),
            None
        );
        assert_eq!(
            progress.dialog_mut().handle_key_down(KeyCode::Tab, false),
            None
        );
        assert_eq!(
            progress.dialog_mut().handle_key_down(KeyCode::Enter, false),
            None
        );
        assert_eq!(
            progress.dialog_mut().handle_key_up(KeyCode::Enter),
            Some(MessageDialogResult::Cancel)
        );
    }
}
