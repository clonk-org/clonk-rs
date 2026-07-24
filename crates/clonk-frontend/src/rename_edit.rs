//! Reusable inline rename edit lifecycle and renderer.
//!
//! This mirrors `C4GUI::RenameEdit` (`src/C4GuiEdit.cpp:686-775`): the
//! replaced label is hidden while the edit is active, its text is prefilled
//! and selected, invalid submissions refocus and reselect the text, and a
//! host-provided focus token is returned when editing finishes.

use std::ops::Range;

use clonk_graphics::clonk_font::{ClonkFont, TextAlign};
use clonk_graphics::{GammaRamp, Rect, Surface};

use crate::classic_gui::{draw_3d_frame, draw_engine_box, IntRect};

/// C++ `C4GUI::Edit` defaults to a 255-byte buffer and reserves one byte for
/// the terminator (`src/C4GuiEdit.cpp:49,170-171`).
pub const RENAME_EDIT_MAX_BYTES: usize = 254;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RenameEditCursorOperation {
    Left,
    Right,
    Home,
    End,
}

/// Request emitted when Enter or focus loss finishes the current input.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum RenameEditAction {
    /// Empty input has the same meaning as Escape.
    Cancelled,
    Submit(String),
}

/// Result returned by the host's rename callback.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RenameEditResult {
    Invalid,
    Accepted,
    /// The callback rebuilt or deleted the label and edit itself.
    Deleted,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RenameEditResolution {
    KeepEditing,
    Finished,
}

/// Inline editor state parameterized by the host's focus identity.
#[derive(Clone, Debug)]
pub struct RenameEdit<Focus> {
    label_text: String,
    text: String,
    caret: usize,
    anchor: usize,
    focused: bool,
    active: bool,
    horizontal_scroll: i32,
    dragging: bool,
    blink_ticks: u32,
    previous_focus: Option<Focus>,
}

impl<Focus> RenameEdit<Focus> {
    /// Replaces a label with a focused edit containing a full selection.
    pub fn new(label_text: impl Into<String>, previous_focus: Option<Focus>) -> Self {
        let mut edit = Self {
            label_text: label_text.into(),
            text: String::new(),
            caret: 0,
            anchor: 0,
            focused: false,
            active: true,
            horizontal_scroll: 0,
            dragging: false,
            blink_ticks: 0,
            previous_focus,
        };
        edit.set_text(edit.label_text.clone());
        edit.focus();
        edit
    }

    pub fn label_text(&self) -> &str {
        &self.label_text
    }

    pub fn label_visible(&self) -> bool {
        !self.active
    }

    pub fn is_active(&self) -> bool {
        self.active
    }

    pub fn text(&self) -> &str {
        &self.text
    }

    pub fn caret(&self) -> usize {
        self.caret
    }

    pub fn horizontal_scroll(&self) -> i32 {
        self.horizontal_scroll
    }

    /// Maps a window-space x coordinate to the nearest UTF-8 byte boundary,
    /// matching the classic edit's half-glyph caret hit test.
    pub fn character_at_x(&self, x: f32, rect: IntRect, font: &ClonkFont) -> usize {
        let target = (x - (rect.x + 2) as f32 + self.horizontal_scroll as f32).max(0.0);
        let mut previous_width = 0_i32;
        for (index, character) in self.text.char_indices() {
            let end = index + character.len_utf8();
            let width = font.measure(&self.text[..end], false).0;
            if target < (previous_width + width) as f32 / 2.0 {
                return index;
            }
            previous_width = width;
        }
        self.text.len()
    }

    pub fn is_focused(&self) -> bool {
        self.focused
    }

    pub fn is_dragging(&self) -> bool {
        self.dragging
    }

    pub fn previous_focus(&self) -> Option<&Focus> {
        self.previous_focus.as_ref()
    }

    pub fn take_previous_focus(&mut self) -> Option<Focus> {
        self.previous_focus.take()
    }

    pub fn set_text(&mut self, text: impl Into<String>) {
        let mut text = text.into();
        if text.len() > RENAME_EDIT_MAX_BYTES {
            let mut end = RENAME_EDIT_MAX_BYTES;
            while !text.is_char_boundary(end) {
                end -= 1;
            }
            text.truncate(end);
        }
        self.text = text;
        self.caret = self.text.len();
        self.anchor = self.caret;
        self.horizontal_scroll = 0;
        self.blink_ticks = 0;
    }

    pub fn focus(&mut self) {
        if self.focused {
            return;
        }
        self.focused = true;
        self.select_all();
        self.blink_ticks = 0;
    }

    pub fn blur(&mut self) {
        self.focused = false;
        self.anchor = self.caret;
        self.dragging = false;
        self.blink_ticks = 0;
    }

    pub fn selection_range(&self) -> Option<Range<usize>> {
        (self.anchor != self.caret).then(|| {
            let start = self.anchor.min(self.caret);
            let end = self.anchor.max(self.caret);
            start..end
        })
    }

    pub fn selected_text(&self) -> Option<&str> {
        self.selection_range().map(|range| &self.text[range])
    }

    pub fn select_all(&mut self) {
        self.anchor = 0;
        self.caret = self.text.len();
        self.blink_ticks = 0;
    }

    pub fn delete_selection(&mut self) -> bool {
        let Some(range) = self.selection_range() else {
            return false;
        };
        let start = range.start;
        self.text.replace_range(range, "");
        self.caret = start;
        self.anchor = start;
        self.blink_ticks = 0;
        true
    }

    pub fn insert_text(&mut self, text: &str) -> bool {
        self.delete_selection();
        let available = RENAME_EDIT_MAX_BYTES.saturating_sub(self.text.len());
        let mut sanitized = String::new();
        for character in text.chars() {
            if character.is_control() {
                continue;
            }
            let character = if character == '|' { '¦' } else { character };
            if sanitized.len() + character.len_utf8() > available {
                break;
            }
            sanitized.push(character);
        }
        if sanitized.is_empty() {
            return false;
        }
        self.text.insert_str(self.caret, &sanitized);
        self.caret += sanitized.len();
        self.anchor = self.caret;
        self.blink_ticks = 0;
        true
    }

    fn previous_boundary(&self, at: usize) -> usize {
        self.text[..at]
            .char_indices()
            .next_back()
            .map(|(index, _)| index)
            .unwrap_or(0)
    }

    fn next_boundary(&self, at: usize) -> usize {
        self.text[at..]
            .chars()
            .next()
            .map(|character| at + character.len_utf8())
            .unwrap_or(self.text.len())
    }

    fn is_word_spacer(character: char) -> bool {
        character.is_ascii() && !character.is_ascii_alphanumeric() && character != '_'
    }

    fn word_target(&self, direction: i32) -> usize {
        if direction < 0 {
            let mut cursor = self.caret;
            let mut nonspace_found = false;
            while cursor > 0 {
                let previous = self.previous_boundary(cursor);
                let character = self.text[previous..cursor]
                    .chars()
                    .next()
                    .expect("non-empty character slice");
                if Self::is_word_spacer(character) {
                    if nonspace_found {
                        break;
                    }
                } else {
                    nonspace_found = true;
                }
                cursor = previous;
            }
            cursor
        } else {
            let mut cursor = self.caret;
            let mut space_found = false;
            while cursor < self.text.len() {
                let next = self.next_boundary(cursor);
                let character = self.text[cursor..next]
                    .chars()
                    .next()
                    .expect("non-empty character slice");
                if Self::is_word_spacer(character) {
                    space_found = true;
                } else if space_found {
                    break;
                }
                cursor = next;
            }
            cursor
        }
    }

    pub fn move_cursor(&mut self, operation: RenameEditCursorOperation, ctrl: bool, shift: bool) {
        if self.selection_range().is_some() && !shift {
            self.anchor = self.caret;
        }
        let old_caret = self.caret;
        let target = match operation {
            RenameEditCursorOperation::Left => {
                if ctrl {
                    self.word_target(-1)
                } else {
                    self.previous_boundary(self.caret)
                }
            }
            RenameEditCursorOperation::Right => {
                if ctrl {
                    self.word_target(1)
                } else {
                    self.next_boundary(self.caret)
                }
            }
            RenameEditCursorOperation::Home => 0,
            RenameEditCursorOperation::End => self.text.len(),
        };
        if shift {
            if self.selection_range().is_none() {
                self.anchor = old_caret;
            }
            self.caret = target;
        } else {
            self.caret = target;
            self.anchor = target;
        }
        self.blink_ticks = 0;
    }

    pub fn backspace(&mut self, ctrl: bool, shift: bool) -> bool {
        if self.delete_selection() {
            return true;
        }
        if shift || self.caret == 0 {
            return false;
        }
        let start = if ctrl {
            self.word_target(-1)
        } else {
            self.previous_boundary(self.caret)
        };
        self.text.replace_range(start..self.caret, "");
        self.caret = start;
        self.anchor = start;
        self.blink_ticks = 0;
        true
    }

    pub fn delete(&mut self, ctrl: bool, shift: bool) -> bool {
        if self.delete_selection() {
            return true;
        }
        if shift || self.caret == self.text.len() {
            return false;
        }
        let end = if ctrl {
            self.word_target(1)
        } else {
            self.next_boundary(self.caret)
        };
        self.text.replace_range(self.caret..end, "");
        self.anchor = self.caret;
        self.blink_ticks = 0;
        true
    }

    pub fn tick_blink(&mut self) -> bool {
        if !self.focused {
            return false;
        }
        const BLINK_TICKS: u32 = 18;
        let before = (self.blink_ticks / BLINK_TICKS) % 2;
        self.blink_ticks = self.blink_ticks.wrapping_add(1);
        before != (self.blink_ticks / BLINK_TICKS) % 2
    }

    pub fn cursor_visible(&self) -> bool {
        self.focused && (self.blink_ticks / 18).is_multiple_of(2)
    }

    pub fn begin_pointer_selection(&mut self, position: usize) {
        let position = position.min(self.text.len());
        self.focus();
        self.anchor = position;
        self.caret = position;
        self.dragging = true;
        self.blink_ticks = 0;
    }

    pub fn drag_pointer_selection(&mut self, position: usize) {
        if !self.dragging {
            return;
        }
        self.caret = position.min(self.text.len());
        self.blink_ticks = 0;
    }

    pub fn end_pointer_selection(&mut self, position: usize) {
        if self.dragging {
            self.caret = position.min(self.text.len());
            self.dragging = false;
            self.blink_ticks = 0;
        }
    }

    pub fn cancel_pointer_selection(&mut self) {
        self.dragging = false;
    }

    pub fn select_word_at(&mut self, mut position: usize) {
        position = position.min(self.text.len());
        if position < self.text.len() {
            let next = self.next_boundary(position);
            let character = self.text[position..next]
                .chars()
                .next()
                .expect("non-empty character slice");
            if Self::is_word_spacer(character) {
                if position == 0 {
                    return;
                }
                let previous = self.previous_boundary(position);
                let character = self.text[previous..position]
                    .chars()
                    .next()
                    .expect("non-empty character slice");
                if Self::is_word_spacer(character) {
                    return;
                }
                position = previous;
            }
        } else if position > 0 {
            let previous = self.previous_boundary(position);
            let character = self.text[previous..position]
                .chars()
                .next()
                .expect("non-empty character slice");
            if Self::is_word_spacer(character) {
                return;
            }
            position = previous;
        } else {
            return;
        }
        let mut start = position;
        while start > 0 {
            let previous = self.previous_boundary(start);
            let character = self.text[previous..start]
                .chars()
                .next()
                .expect("non-empty character slice");
            if Self::is_word_spacer(character) {
                break;
            }
            start = previous;
        }
        let mut end = self.next_boundary(position);
        while end < self.text.len() {
            let next = self.next_boundary(end);
            let character = self.text[end..next]
                .chars()
                .next()
                .expect("non-empty character slice");
            if Self::is_word_spacer(character) {
                break;
            }
            end = next;
        }
        self.anchor = start;
        self.caret = end;
        self.dragging = false;
        self.blink_ticks = 0;
    }

    /// Enter and `OnLooseFocus` share this finish path. Empty input cancels.
    pub fn finish_input(&mut self) -> RenameEditAction {
        if self.text.is_empty() {
            self.finish();
            RenameEditAction::Cancelled
        } else {
            RenameEditAction::Submit(self.text.clone())
        }
    }

    pub fn focus_lost(&mut self) -> RenameEditAction {
        self.blur();
        self.finish_input()
    }

    pub fn abort(&mut self) -> RenameEditAction {
        self.finish();
        RenameEditAction::Cancelled
    }

    pub fn handle_gamepad_high(&mut self) -> RenameEditAction {
        self.abort()
    }

    pub fn resolve(&mut self, result: RenameEditResult) -> RenameEditResolution {
        match result {
            RenameEditResult::Invalid => {
                self.active = true;
                self.focus();
                self.select_all();
                RenameEditResolution::KeepEditing
            }
            RenameEditResult::Accepted => {
                self.finish();
                RenameEditResolution::Finished
            }
            RenameEditResult::Deleted => {
                // RR_Deleted means the callback already rebuilt/deleted the
                // host label and chose its replacement focus. Do not let a
                // later generic finish restore the stale saved control.
                self.finish();
                self.previous_focus = None;
                RenameEditResolution::Finished
            }
        }
    }

    fn finish(&mut self) {
        self.active = false;
        self.blur();
    }

    fn scroll_cursor_in_view(&mut self, cursor_x: i32, client_width: i32, cursor_half: i32) {
        if client_width < 5 {
            return;
        }
        let cursor_x = cursor_x.saturating_add(cursor_half);
        if cursor_x < self.horizontal_scroll && self.horizontal_scroll > 0 {
            self.horizontal_scroll = cursor_x.saturating_sub(2).max(0);
        }
        if cursor_x > self.horizontal_scroll
            && cursor_x > client_width.saturating_add(self.horizontal_scroll)
        {
            self.horizontal_scroll =
                cursor_x.saturating_sub(client_width) + i32::from(self.caret < self.text.len()) * 2;
        }
    }

    /// Draws the replacement edit in an arbitrary label rectangle.
    pub fn render(
        &mut self,
        surface: &mut Surface,
        font: &ClonkFont,
        rect: IntRect,
        gamma: Option<&GammaRamp>,
    ) {
        self.render_with_draw_focus(surface, font, rect, true, gamma);
    }

    /// Renders the active edit while allowing the owning screen to suppress
    /// only the caret when a context menu or modal dialog owns draw focus.
    pub fn render_with_draw_focus(
        &mut self,
        surface: &mut Surface,
        font: &ClonkFont,
        rect: IntRect,
        draw_focus: bool,
        gamma: Option<&GammaRamp>,
    ) {
        if rect.w <= 0 || rect.h <= 0 {
            return;
        }
        let cursor_x = font.measure(&self.text[..self.caret], false).0;
        let cursor_half = font.measure("¦", false).0 / 2;
        self.scroll_cursor_in_view(cursor_x, rect.w - 4, cursor_half);
        draw_engine_box(
            surface,
            rect.x,
            rect.y,
            rect.x + rect.w - 1,
            rect.y + rect.h - 1,
            0x7f000000,
            gamma,
        );
        draw_3d_frame(surface, rect, gamma);
        let client_left = rect.x + 2;
        let client_right = (rect.x + rect.w - 3).max(client_left);
        if let Some(selection) = self.selection_range() {
            let x1 = client_left + font.measure(&self.text[..selection.start], false).0
                - self.horizontal_scroll;
            let x2 = client_left + font.measure(&self.text[..selection.end], false).0
                - self.horizontal_scroll;
            if x2 > x1 {
                draw_engine_box(
                    surface,
                    x1.clamp(client_left, client_right),
                    rect.y + 1,
                    (x2 - 1).clamp(client_left, client_right),
                    rect.y + rect.h - 2,
                    0x7f7f7f00,
                    gamma,
                );
            }
        }
        let previous_clip = surface.clip();
        let edit_clip = Rect::new(rect.x, rect.y, rect.w.max(1) as u32, rect.h.max(1) as u32);
        let active_clip = previous_clip
            .and_then(|clip| clip.intersection(edit_clip))
            .unwrap_or_else(|| {
                if previous_clip.is_some() {
                    Rect::new(rect.x, rect.y, 0, 0)
                } else {
                    edit_clip
                }
            });
        surface.set_clip(active_clip);
        font.draw_with_gamma(
            surface,
            client_left - self.horizontal_scroll,
            rect.y,
            &self.text,
            [255, 255, 255, 255],
            TextAlign::Left,
            false,
            gamma,
        );
        if draw_focus && self.cursor_visible() {
            let x = client_left + cursor_x - self.horizontal_scroll;
            if (client_left..=client_right).contains(&x) {
                draw_engine_box(
                    surface,
                    x,
                    rect.y + 1,
                    x,
                    rect.y + rect.h - 2,
                    0x00ffffff,
                    gamma,
                );
            }
        }
        match previous_clip {
            Some(clip) => surface.set_clip(clip),
            None => surface.clear_clip(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[derive(Clone, Copy, Debug, PartialEq, Eq)]
    enum Focus {
        Search,
        List,
    }

    #[test]
    fn constructor_hides_label_selects_text_and_abort_restores_focus_token() {
        let mut edit = RenameEdit::new("Scenario", Some(Focus::Search));
        assert!(!edit.label_visible());
        assert!(edit.is_active());
        assert!(edit.is_focused());
        assert_eq!(edit.text(), "Scenario");
        assert_eq!(edit.selected_text(), Some("Scenario"));

        assert_eq!(edit.abort(), RenameEditAction::Cancelled);
        assert!(edit.label_visible());
        assert!(!edit.is_active());
        assert!(!edit.is_focused());
        assert_eq!(edit.take_previous_focus(), Some(Focus::Search));
    }

    #[test]
    fn empty_input_cancels_and_invalid_input_refocuses_with_full_selection() {
        let mut empty = RenameEdit::new("Scenario", Some(Focus::List));
        empty.delete_selection();
        assert_eq!(empty.finish_input(), RenameEditAction::Cancelled);
        assert!(empty.label_visible());

        let mut edit = RenameEdit::new("Scenario", Some(Focus::List));
        edit.insert_text("Taken");
        assert_eq!(
            edit.focus_lost(),
            RenameEditAction::Submit("Taken".to_string())
        );
        assert!(!edit.is_focused());
        assert_eq!(
            edit.resolve(RenameEditResult::Invalid),
            RenameEditResolution::KeepEditing
        );
        assert!(edit.is_active());
        assert!(edit.is_focused());
        assert_eq!(edit.selected_text(), Some("Taken"));
        assert_eq!(
            edit.resolve(RenameEditResult::Accepted),
            RenameEditResolution::Finished
        );
        assert!(edit.label_visible());

        let mut deleted = RenameEdit::new("Scenario", Some(Focus::Search));
        assert_eq!(
            deleted.resolve(RenameEditResult::Deleted),
            RenameEditResolution::Finished
        );
        assert_eq!(deleted.take_previous_focus(), None);
    }

    #[test]
    fn gamepad_high_uses_the_escape_abort_lifecycle() {
        let mut edit = RenameEdit::new("Scenario", None::<Focus>);
        edit.insert_text("Changed");
        assert_eq!(edit.handle_gamepad_high(), RenameEditAction::Cancelled);
        assert!(!edit.is_active());
        assert!(edit.label_visible());
    }
}
