//! Scenario-search edit state, independent of the application event loop.

pub const SEARCH_EDIT_MAX_BYTES: usize = 254;

pub use crate::edit::CursorOperation as SearchCursorOperation;

#[derive(Clone, Debug, Default)]
pub struct SearchEditState {
    pub text: String,
    pub caret: usize,
    pub anchor: usize,
    pub focused: bool,
    pub horizontal_scroll: i32,
    pub dragging: bool,
    /// C++ retains `iSelectionStart` independently from the visible caret,
    /// even when the selection is collapsed; an active drag reuses it.
    pub drag_anchor: usize,
    pub blink_ticks: u32,
    /// The IME composition in progress, drawn at the caret and never entered
    /// into `text` — only `Ime::Commit` reaches `insert_text`.
    pub composition: Option<crate::ime::ImeComposition>,
}

impl SearchEditState {
    pub fn text(&self) -> &str {
        &self.text
    }

    pub fn composition(&self) -> Option<&crate::ime::ImeComposition> {
        self.composition.as_ref()
    }

    /// Replaces the composition in progress. `None` ends it, which is what
    /// `Ime::Commit` and `Ime::Disabled` both mean.
    pub fn set_composition(&mut self, composition: Option<crate::ime::ImeComposition>) {
        self.composition = composition.filter(|composition| !composition.text.is_empty());
    }

    pub fn caret(&self) -> usize {
        self.caret
    }

    pub fn set_text(&mut self, text: impl Into<String>) {
        let mut text = text.into();
        if text.len() > SEARCH_EDIT_MAX_BYTES {
            let mut end = SEARCH_EDIT_MAX_BYTES;
            while !text.is_char_boundary(end) {
                end -= 1;
            }
            text.truncate(end);
        }
        self.text = text;
        self.caret = self.text.len();
        self.anchor = self.caret;
        self.horizontal_scroll = 0;
        self.drag_anchor = 0;
        self.blink_ticks = 0;
    }

    pub fn focus(&mut self) {
        if self.focused {
            return;
        }
        self.focused = true;
        self.anchor = 0;
        self.caret = self.text.len();
        self.drag_anchor = 0;
        self.blink_ticks = 0;
    }

    pub fn blur(&mut self) {
        self.focused = false;
        self.anchor = self.caret;
        self.dragging = false;
        self.drag_anchor = 0;
        self.blink_ticks = 0;
    }

    pub fn is_focused(&self) -> bool {
        self.focused
    }

    pub fn selection_range(&self) -> Option<std::ops::Range<usize>> {
        crate::edit::selection_range(self.anchor, self.caret)
    }

    pub fn selected_text(&self) -> Option<&str> {
        self.selection_range().map(|range| &self.text[range])
    }

    pub fn select_all(&mut self) {
        self.anchor = 0;
        self.caret = self.text.len();
        self.drag_anchor = 0;
        self.blink_ticks = 0;
    }

    pub fn delete_selection(&mut self) -> bool {
        if !crate::edit::EditBuffer::new(&mut self.text, &mut self.caret, &mut self.anchor)
            .delete_selection()
        {
            return false;
        }
        self.drag_anchor = self.caret;
        self.blink_ticks = 0;
        true
    }

    pub fn insert_text(&mut self, text: &str) -> bool {
        let selection_deleted = self.delete_selection();
        let inserted =
            crate::edit::EditBuffer::new(&mut self.text, &mut self.caret, &mut self.anchor)
                .insert_sanitized(text, SEARCH_EDIT_MAX_BYTES);
        if inserted {
            self.blink_ticks = 0;
        }
        selection_deleted || inserted
    }

    /// `C4GUI::Edit::InsertText`: unlike keyboard input and ordinary Paste,
    /// the middle-button PRIMARY path inserts bytes without mapping `|` or
    /// treating line breaks as submit callbacks.
    pub fn insert_raw_text(&mut self, text: &str) -> bool {
        let old_text = self.text.clone();
        self.delete_selection();
        let available = SEARCH_EDIT_MAX_BYTES.saturating_sub(self.text.len());
        let mut end = text.len().min(available);
        while !text.is_char_boundary(end) {
            end -= 1;
        }
        if end > 0 {
            self.text.insert_str(self.caret, &text[..end]);
            self.caret += end;
            self.blink_ticks = 0;
        }
        self.anchor = self.caret;
        self.text != old_text
    }

    pub fn move_cursor(&mut self, operation: SearchCursorOperation, ctrl: bool, shift: bool) {
        let old_caret = self.caret;
        let had_selection = self.selection_range().is_some();
        if had_selection && !shift {
            self.drag_anchor = 0;
        }
        crate::edit::EditBuffer::new(&mut self.text, &mut self.caret, &mut self.anchor)
            .move_cursor(operation, ctrl, shift);
        if shift && self.caret != old_caret && !had_selection {
            self.drag_anchor = old_caret;
        }
        self.blink_ticks = 0;
    }

    pub fn backspace(&mut self, ctrl: bool, shift: bool) -> bool {
        if self.delete_selection() {
            return true;
        }
        let changed =
            crate::edit::EditBuffer::new(&mut self.text, &mut self.caret, &mut self.anchor)
                .erase(true, ctrl, shift);
        if changed {
            self.blink_ticks = 0;
        }
        changed
    }

    pub fn delete(&mut self, ctrl: bool, shift: bool) -> bool {
        if self.delete_selection() {
            return true;
        }
        let changed =
            crate::edit::EditBuffer::new(&mut self.text, &mut self.caret, &mut self.anchor)
                .erase(false, ctrl, shift);
        if changed {
            self.blink_ticks = 0;
        }
        changed
    }

    pub fn scroll_cursor_in_view(&mut self, cursor_x: i32, client_width: i32, cursor_half: i32) {
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
        self.drag_anchor = position;
        self.blink_ticks = 0;
    }

    pub fn drag_pointer_selection(&mut self, position: usize) {
        if !self.dragging {
            return;
        }
        self.anchor = self.drag_anchor.min(self.text.len());
        self.caret = position.min(self.text.len());
        self.blink_ticks = 0;
    }

    pub fn end_pointer_selection(&mut self, position: usize) {
        if self.dragging {
            self.anchor = self.drag_anchor.min(self.text.len());
            self.caret = position.min(self.text.len());
            self.dragging = false;
            self.blink_ticks = 0;
        }
    }

    pub fn select_word_at(&mut self, position: usize) {
        let Some(range) = crate::edit::word_selection(&self.text, position) else {
            return;
        };
        self.anchor = range.start;
        self.caret = range.end;
        self.dragging = false;
        self.drag_anchor = range.start;
        self.blink_ticks = 0;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn scensel_search_edit_matches_selection_word_and_length_rules() {
        let mut edit = SearchEditState::default();
        edit.set_text("Alpha beta");
        edit.focus();
        assert_eq!(edit.selected_text(), Some("Alpha beta"));
        edit.insert_text("Z");
        assert_eq!(edit.text(), "Z", "typing replaces Ctrl+F select-all");

        edit.set_text("one  two_three!");
        edit.move_cursor(SearchCursorOperation::End, false, false);
        edit.move_cursor(SearchCursorOperation::Left, false, false);
        edit.move_cursor(SearchCursorOperation::Left, true, false);
        assert_eq!(edit.caret(), 5, "Ctrl+Left stops at the final word start");
        edit.backspace(true, false);
        assert_eq!(edit.text(), "two_three!", "Ctrl+Backspace removes one word");
        edit.move_cursor(SearchCursorOperation::Home, false, false);
        edit.move_cursor(SearchCursorOperation::Right, true, true);
        assert_eq!(edit.selected_text(), Some("two_three!"));
        edit.delete(false, false);
        assert_eq!(edit.text(), "");

        edit.set_text("");
        edit.insert_text(&"a".repeat(300));
        assert_eq!(edit.text().len(), SEARCH_EDIT_MAX_BYTES);
        edit.set_text("");
        edit.insert_text("left|right");
        assert_eq!(edit.text(), "left¦right");
        edit.set_text("éé");
        edit.move_cursor(SearchCursorOperation::Left, false, false);
        assert_eq!(edit.caret(), "é".len(), "caret stays on UTF-8 boundaries");
        edit.backspace(false, false);
        assert_eq!(edit.text(), "é");

        edit.set_text("alpha beta");
        edit.select_word_at(8);
        assert_eq!(edit.selected_text(), Some("beta"));
        edit.begin_pointer_selection(0);
        edit.drag_pointer_selection(edit.text().len());
        edit.end_pointer_selection(edit.text().len());
        assert_eq!(edit.selected_text(), Some("alpha beta"));

        edit.set_text("abcdef");
        edit.begin_pointer_selection(5);
        edit.drag_pointer_selection(2);
        assert_eq!(edit.selected_text(), Some("cde"));
        assert!(edit.backspace(false, false));
        assert_eq!(edit.text(), "abf");
        edit.drag_pointer_selection(edit.text().len());
        assert_eq!(
            edit.selected_text(),
            Some("f"),
            "selection deletion updates the still-active physical drag anchor"
        );
        edit.end_pointer_selection(edit.text().len());

        edit.set_text("abcdef");
        edit.begin_pointer_selection(5);
        assert!(edit.backspace(false, false));
        assert_eq!(edit.text(), "abcdf");
        assert_eq!(edit.caret(), 4);
        edit.drag_pointer_selection(2);
        assert_eq!(
            edit.selected_text(),
            Some("cdf"),
            "collapsed cursor deletion preserves C++'s hidden drag anchor"
        );
        edit.end_pointer_selection(2);

        edit.set_text("W".repeat(100));
        edit.scroll_cursor_in_view(500, 100, 3);
        assert!(edit.horizontal_scroll > 0);
        edit.move_cursor(SearchCursorOperation::Home, false, false);
        edit.scroll_cursor_in_view(0, 100, 3);
        assert_eq!(edit.horizontal_scroll, 1);
        assert!(edit.cursor_visible());
        for _ in 0..18 {
            edit.tick_blink();
        }
        assert!(!edit.cursor_visible());
    }

    // C++ notifies text change while deleting the selection before a
    // replacement that cannot fit (src/C4GuiEdit.cpp:145-190). The Rust edit
    // must likewise report that mutation so live results are refreshed.
    #[test]
    fn scensel_search_edit_reports_selection_deletion_when_replacement_does_not_fit() {
        let mut edit = SearchEditState::default();
        edit.set_text("a".repeat(SEARCH_EDIT_MAX_BYTES));
        edit.anchor = SEARCH_EDIT_MAX_BYTES - 1;
        edit.caret = SEARCH_EDIT_MAX_BYTES;

        let changed = edit.insert_text("é");

        assert!(changed);
        assert_eq!(edit.text().len(), SEARCH_EDIT_MAX_BYTES - 1);
    }
}
