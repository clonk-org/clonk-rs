//! The Script / Title / Info component editor's surface.
//!
//! `C4Console::EditScript`, `EditTitle` and `EditInfo`
//! (`C4Console.cpp:1328-1351`) are three lines each: refuse in a network game,
//! then `ShowDialog(hWindow)` on the component's `C4ComponentHost` — and that
//! call is inside `#ifdef _WIN32`, so on the reference build all three refuse
//! or do nothing. The one statement that survives the macro is
//! `Game.ScriptEngine.ReLink(&Game.Defs)` at the end of `EditScript`, which
//! runs **whether or not** the dialog opened or was cancelled.
//!
//! The commit rules are already ported —
//! [`clonk_engine::developer_components::ComponentHost`] holds
//! `C4ComponentHost`'s accept/cancel/save behaviour — so what this module adds
//! is the text surface itself, which has no oracle at all: the Win32 dialog is
//! an `EDITTEXT` control and its behaviour is the toolkit's.

use clonk_frontend::classic_gui::IntRect;
use clonk_frontend::developer_chrome::{
    draw_fitted_text, draw_sunken, fill, CONTROL_BACKGROUND, CONTROL_TEXT, MID_EDGE,
    SMALL_FONT_SIZE, WINDOW_BACKGROUND,
};
use clonk_graphics::{Surface, TextFont};

/// The editor window's extent. Nothing to port — the Win32 dialog template is
/// not compiled here — so it is a comfortable size for a scenario script.
pub(crate) const EDITOR_WIDTH: u32 = 560;
pub(crate) const EDITOR_HEIGHT: u32 = 420;

const PADDING: i32 = 6;
const ROW_HEIGHT: i32 = 14;
const STATUS_HEIGHT: i32 = 18;
/// The inset `draw_fitted_text` starts a line at, which the caret has to share
/// or it would sit a few pixels off the text it points into.
const TEXT_PADDING: i32 = 3;

/// An open editor's text and caret.
///
/// The text is held as lines because that is how it is drawn, how the caret
/// moves, and how it scrolls; it is joined back with `\n` on commit.
#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct ComponentEditorText {
    lines: Vec<String>,
    /// Caret line, and caret column measured in `char`s — not bytes, so a
    /// multi-byte character moves the caret one step like every other.
    caret: (usize, usize),
    /// Where a shift-extended selection started, if one is open.
    ///
    /// The caret is the moving end, so the pair is not ordered — `selection`
    /// orders it. Holding the anchor rather than a sorted range is what lets a
    /// selection be extended back past its own start without inverting.
    anchor: Option<(usize, usize)>,
    first_visible: usize,
    modified: bool,
    /// States to step back to, oldest first. The opened text is never pushed,
    /// so undo stops there rather than emptying the editor.
    undone: Vec<EditorHistory>,
    /// States undone out of, newest last. Cleared by any fresh edit, so redo
    /// cannot splice a stale future onto a changed present.
    redone: Vec<EditorHistory>,
    /// Set while one user-level edit is running, so a paste that inserts a
    /// hundred characters records one step rather than a hundred.
    recording: bool,
}

/// One undo step: everything an edit can move.
#[derive(Clone, Debug, Eq, PartialEq)]
struct EditorHistory {
    lines: Vec<String>,
    caret: (usize, usize),
    modified: bool,
}

/// What a key did to the editor.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum ComponentEditorKey {
    Left,
    Right,
    Up,
    Down,
    Home,
    End,
    Backspace,
    Delete,
    Enter,
}

impl ComponentEditorText {
    /// Open on a component's bytes.
    ///
    /// `C4ComponentHost` holds raw bytes and the port keeps them raw, so the
    /// text is projected through the same Latin-1 conversion the rest of the
    /// engine's strings use rather than being assumed UTF-8 — a scenario
    /// script written on a German Windows is not.
    pub(crate) fn opened(data: &[u8]) -> Self {
        let text = clonk_script::c4_string_from_bytes(data);
        // A component that ends in a newline has a real empty last line, and
        // dropping it would move the caret somewhere the user did not put it.
        let lines = text
            .split('\n')
            .map(|line| line.trim_end_matches('\r').to_owned())
            .collect::<Vec<_>>();
        Self {
            lines: if lines.is_empty() {
                vec![String::new()]
            } else {
                lines
            },
            caret: (0, 0),
            anchor: None,
            first_visible: 0,
            modified: false,
            undone: Vec::new(),
            redone: Vec::new(),
            recording: false,
        }
    }

    /// The bytes to commit, back through the same projection.
    pub(crate) fn bytes(&self) -> Vec<u8> {
        clonk_script::c4_string_bytes(&self.lines.join("\n"))
    }

    pub(crate) fn modified(&self) -> bool {
        self.modified
    }

    pub(crate) fn caret(&self) -> (usize, usize) {
        self.caret
    }

    pub(crate) fn lines(&self) -> &[String] {
        &self.lines
    }

    fn line_length(&self, line: usize) -> usize {
        self.lines.get(line).map_or(0, |line| line.chars().count())
    }

    /// The byte offset of a character column, so an edit lands between
    /// characters rather than inside one.
    fn byte_offset(line: &str, column: usize) -> usize {
        line.char_indices()
            .nth(column)
            .map_or(line.len(), |(offset, _)| offset)
    }

    /// The open selection as an ordered `(start, end)` pair.
    ///
    /// `None` when nothing is selected *or* when the anchor sits on the caret,
    /// so callers never have to treat an empty range as a special case.
    pub(crate) fn selection(&self) -> Option<((usize, usize), (usize, usize))> {
        let anchor = self.anchor?;
        if anchor == self.caret {
            return None;
        }
        Some(if anchor <= self.caret {
            (anchor, self.caret)
        } else {
            (self.caret, anchor)
        })
    }

    /// The selected text, joined with newlines as it would be copied.
    pub(crate) fn selected_text(&self) -> Option<String> {
        let (start, end) = self.selection()?;
        if start.0 == end.0 {
            let line = self.lines.get(start.0)?;
            let from = Self::byte_offset(line, start.1);
            let to = Self::byte_offset(line, end.1);
            return Some(line[from..to].to_owned());
        }
        let first = self.lines.get(start.0)?;
        let mut text = first[Self::byte_offset(first, start.1)..].to_owned();
        for line in self.lines.get(start.0 + 1..end.0)? {
            text.push('\n');
            text.push_str(line);
        }
        let last = self.lines.get(end.0)?;
        text.push('\n');
        text.push_str(&last[..Self::byte_offset(last, end.1)]);
        Some(text)
    }

    /// Removes the selection, leaving the caret where it started.
    ///
    /// Returns whether anything was removed, so an edit can ask "did the
    /// selection already handle this?" before doing its own single-character
    /// work.
    fn delete_selection(&mut self) -> bool {
        let Some((start, end)) = self.selection() else {
            self.anchor = None;
            return false;
        };
        if start.0 == end.0 {
            if let Some(line) = self.lines.get_mut(start.0) {
                let from = Self::byte_offset(line, start.1);
                let to = Self::byte_offset(line, end.1);
                line.replace_range(from..to, "");
            }
        } else {
            let tail = self
                .lines
                .get(end.0)
                .map(|line| line[Self::byte_offset(line, end.1)..].to_owned())
                .unwrap_or_default();
            if let Some(line) = self.lines.get_mut(start.0) {
                let from = Self::byte_offset(line, start.1);
                line.truncate(from);
                line.push_str(&tail);
            }
            self.lines
                .drain(start.0 + 1..=end.0.min(self.lines.len() - 1));
        }
        self.caret = start;
        self.anchor = None;
        self.modified = true;
        true
    }

    /// Records the state one edit is about to leave behind.
    ///
    /// Re-entrant edits — `paste` driving `insert`, `insert` driving `key` —
    /// take the checkpoint of the outermost one only, which is what makes a
    /// paste undo as the single action the user performed.
    fn checkpoint(&mut self) -> bool {
        if self.recording {
            return false;
        }
        self.undone.push(EditorHistory {
            lines: self.lines.clone(),
            caret: self.caret,
            modified: self.modified,
        });
        self.redone.clear();
        self.recording = true;
        true
    }

    fn finish_edit(&mut self, owned: bool) {
        if owned {
            self.recording = false;
        }
    }

    /// Steps back one edit. `false` when there is nothing left to undo.
    pub(crate) fn undo(&mut self) -> bool {
        let Some(previous) = self.undone.pop() else {
            return false;
        };
        self.redone.push(EditorHistory {
            lines: std::mem::take(&mut self.lines),
            caret: self.caret,
            modified: self.modified,
        });
        self.lines = previous.lines;
        self.caret = previous.caret;
        self.modified = previous.modified;
        self.anchor = None;
        true
    }

    /// Steps forward again. `false` when nothing was undone, or when a fresh
    /// edit has dropped the trail.
    pub(crate) fn redo(&mut self) -> bool {
        let Some(next) = self.redone.pop() else {
            return false;
        };
        self.undone.push(EditorHistory {
            lines: std::mem::take(&mut self.lines),
            caret: self.caret,
            modified: self.modified,
        });
        self.lines = next.lines;
        self.caret = next.caret;
        self.modified = next.modified;
        self.anchor = None;
        true
    }

    /// The selected text, for the clipboard. Takes nothing and changes
    /// nothing.
    pub(crate) fn copy_selection(&self) -> Option<String> {
        self.selected_text()
    }

    /// The selected text, removing it. `None` — and no edit — when nothing is
    /// selected, rather than falling back to the line or the word.
    pub(crate) fn cut_selection(&mut self) -> Option<String> {
        let text = self.selected_text()?;
        let owned = self.checkpoint();
        self.delete_selection();
        self.finish_edit(owned);
        Some(text)
    }

    /// Inserts text at the caret, replacing any selection.
    ///
    /// Newlines split lines exactly as typing one would, so pasted script
    /// arrives as script rather than as one long line. Other control
    /// characters are dropped for the same reason `insert` drops them: they
    /// have no glyph and would corrupt the component on commit.
    pub(crate) fn paste(&mut self, text: &str) {
        if text.is_empty() {
            return;
        }
        let owned = self.checkpoint();
        self.delete_selection();
        for character in text.chars() {
            match character {
                '\n' => self.key(ComponentEditorKey::Enter),
                '\r' => {}
                character => self.insert(character),
            }
        }
        self.finish_edit(owned);
    }

    /// How far into a line a caret column sits, in pixels.
    ///
    /// Measured rather than counted: the editor draws in a proportional font,
    /// so a column index says nothing about where the gap between two glyphs
    /// actually is. A column past the end of the line clamps to its full
    /// advance, which is where the caret belongs after `End`.
    fn caret_offset(line: &str, column: usize, font: &dyn TextFont) -> i32 {
        if column == 0 {
            return 0;
        }
        let prefix: String = line.chars().take(column).collect();
        if prefix.is_empty() {
            return 0;
        }
        font.measure_text(&prefix, SMALL_FONT_SIZE).width as i32
    }

    /// Type one character.
    pub(crate) fn insert(&mut self, character: char) {
        if character == '\n' {
            self.key(ComponentEditorKey::Enter);
            return;
        }
        // A control character other than the newline has no glyph and would
        // corrupt the component if it were written back.
        if character.is_control() {
            return;
        }
        let owned = self.checkpoint();
        self.delete_selection();
        let (line, column) = self.caret;
        let Some(target) = self.lines.get_mut(line) else {
            self.finish_edit(owned);
            return;
        };
        let offset = Self::byte_offset(target, column);
        target.insert(offset, character);
        self.caret = (line, column + 1);
        self.modified = true;
        self.finish_edit(owned);
    }

    /// Apply one editing key, with no selection extension.
    pub(crate) fn key(&mut self, key: ComponentEditorKey) {
        self.key_extending(key, false);
    }

    /// Apply one editing key, extending the selection when shift is held.
    ///
    /// A movement with `extend` opens a selection from wherever the caret was;
    /// one without drops any open selection, which is what stops a stale range
    /// from being left highlighted behind a moved caret.
    pub(crate) fn key_extending(&mut self, key: ComponentEditorKey, extend: bool) {
        let movement = matches!(
            key,
            ComponentEditorKey::Left
                | ComponentEditorKey::Right
                | ComponentEditorKey::Up
                | ComponentEditorKey::Down
                | ComponentEditorKey::Home
                | ComponentEditorKey::End
        );
        let owned = if movement { false } else { self.checkpoint() };
        if movement {
            if extend {
                self.anchor.get_or_insert(self.caret);
            } else {
                self.anchor = None;
            }
        } else if matches!(
            key,
            ComponentEditorKey::Backspace | ComponentEditorKey::Delete | ComponentEditorKey::Enter
        ) && self.delete_selection()
            && !matches!(key, ComponentEditorKey::Enter)
        {
            // Backspace and delete are satisfied by the selection they took;
            // enter still has to split at the caret it left behind.
            self.finish_edit(owned);
            return;
        }
        let (line, column) = self.caret;
        match key {
            ComponentEditorKey::Left if column > 0 => self.caret = (line, column - 1),
            // Left at the start of a line wraps to the end of the one above,
            // which is where the character it would delete lives.
            ComponentEditorKey::Left if line > 0 => {
                self.caret = (line - 1, self.line_length(line - 1))
            }
            ComponentEditorKey::Left => {}
            ComponentEditorKey::Right if column < self.line_length(line) => {
                self.caret = (line, column + 1)
            }
            ComponentEditorKey::Right if line + 1 < self.lines.len() => self.caret = (line + 1, 0),
            ComponentEditorKey::Right => {}
            ComponentEditorKey::Up if line > 0 => {
                self.caret = (line - 1, column.min(self.line_length(line - 1)))
            }
            ComponentEditorKey::Up => {}
            ComponentEditorKey::Down if line + 1 < self.lines.len() => {
                self.caret = (line + 1, column.min(self.line_length(line + 1)))
            }
            ComponentEditorKey::Down => {}
            ComponentEditorKey::Home => self.caret = (line, 0),
            ComponentEditorKey::End => self.caret = (line, self.line_length(line)),
            ComponentEditorKey::Enter => {
                let offset = Self::byte_offset(&self.lines[line], column);
                let tail = self.lines[line].split_off(offset);
                self.lines.insert(line + 1, tail);
                self.caret = (line + 1, 0);
                self.modified = true;
            }
            ComponentEditorKey::Backspace if column > 0 => {
                let offset = Self::byte_offset(&self.lines[line], column - 1);
                self.lines[line].remove(offset);
                self.caret = (line, column - 1);
                self.modified = true;
            }
            // Backspace at the start of a line joins it to the one above.
            ComponentEditorKey::Backspace if line > 0 => {
                let tail = self.lines.remove(line);
                let column = self.line_length(line - 1);
                self.lines[line - 1].push_str(&tail);
                self.caret = (line - 1, column);
                self.modified = true;
            }
            ComponentEditorKey::Backspace => {}
            ComponentEditorKey::Delete if column < self.line_length(line) => {
                let offset = Self::byte_offset(&self.lines[line], column);
                self.lines[line].remove(offset);
                self.modified = true;
            }
            ComponentEditorKey::Delete if line + 1 < self.lines.len() => {
                let tail = self.lines.remove(line + 1);
                self.lines[line].push_str(&tail);
                self.modified = true;
            }
            ComponentEditorKey::Delete => {}
        }
        self.finish_edit(owned);
    }

    /// Scroll so the caret is on screen, given how many rows fit.
    fn follow_caret(&mut self, rows: usize) {
        let (line, _) = self.caret;
        if line < self.first_visible {
            self.first_visible = line;
        } else if rows > 0 && line >= self.first_visible + rows {
            self.first_visible = line + 1 - rows;
        }
    }

    /// Draw the editor.
    pub(crate) fn render(&mut self, surface: &mut Surface, font: &dyn TextFont, title: &str) {
        surface.fill(WINDOW_BACKGROUND);
        let (width, height) = (surface.width() as i32, surface.height() as i32);
        let text_area = IntRect {
            x: PADDING,
            y: PADDING,
            w: (width - PADDING * 2).max(1),
            h: (height - PADDING * 2 - STATUS_HEIGHT).max(1),
        };
        draw_sunken(surface, text_area, CONTROL_BACKGROUND);
        let rows = ((text_area.h - 2) / ROW_HEIGHT).max(0) as usize;
        self.follow_caret(rows);
        for (row, line) in self
            .lines
            .iter()
            .enumerate()
            .skip(self.first_visible)
            .take(rows)
        {
            let rect = IntRect {
                x: text_area.x + 1,
                y: text_area.y + 1 + (row - self.first_visible) as i32 * ROW_HEIGHT,
                w: (text_area.w - 2).max(1),
                h: ROW_HEIGHT,
            };
            if row == self.caret.0 {
                // Between glyphs, measured. `draw_fitted_text` starts the line
                // at `rect.x + padding` and truncates rather than rescaling, so
                // the same padding and font size place the caret on the gap the
                // text actually shows.
                let offset = Self::caret_offset(line, self.caret.1, font);
                fill(
                    surface,
                    IntRect {
                        x: (rect.x + TEXT_PADDING + offset).min(rect.x + rect.w - 1),
                        w: 1,
                        ..rect
                    },
                    MID_EDGE,
                );
            }
            draw_fitted_text(
                surface,
                font,
                rect,
                line,
                CONTROL_TEXT,
                SMALL_FONT_SIZE,
                TEXT_PADDING,
            );
        }
        draw_fitted_text(
            surface,
            font,
            IntRect {
                x: PADDING,
                y: height - STATUS_HEIGHT,
                w: (width - PADDING * 2).max(1),
                h: STATUS_HEIGHT,
            },
            title,
            CONTROL_TEXT,
            SMALL_FONT_SIZE,
            2,
        );
    }
}

#[cfg(all(
    test,
    any(not(feature = "app-test-shard-mode"), feature = "app-test-shard-5",),
))]
mod tests {
    use super::*;
    use clonk_graphics::{BitmapFont, PixelFormat};

    /// Undo steps back one edit at a time and redo returns them
    /// (clonk-org/clonk-rs#389).
    #[test]
    fn undo_steps_back_through_edits_and_redo_returns_them() {
        let mut editor = ComponentEditorText::opened(b"a");
        editor.key(ComponentEditorKey::End);
        editor.insert('b');
        editor.insert('c');
        assert_eq!(editor.lines(), ["abc"]);

        assert!(editor.undo());
        assert_eq!(editor.lines(), ["ab"]);
        assert!(editor.undo());
        assert_eq!(editor.lines(), ["a"]);
        assert!(
            !editor.undo(),
            "the opened text is the floor; undo cannot go behind it"
        );
        assert!(
            !editor.modified(),
            "undone all the way back, the component is unmodified again"
        );

        assert!(editor.redo());
        assert_eq!(editor.lines(), ["ab"]);
        assert!(editor.redo());
        assert_eq!(editor.lines(), ["abc"]);
        assert!(!editor.redo());
    }

    /// A paste is one edit, not one per character — undoing it a letter at a
    /// time would make pasting a script unusable.
    #[test]
    fn a_paste_undoes_as_a_single_step() {
        let mut editor = ComponentEditorText::opened(b"");
        editor.paste("one\ntwo");
        assert_eq!(editor.lines(), ["one", "two"]);

        assert!(editor.undo());
        assert_eq!(
            editor.lines(),
            [""],
            "the whole paste came back out at once"
        );
    }

    /// Editing after an undo drops what was undone, so redo cannot splice a
    /// stale future onto a changed present.
    #[test]
    fn an_edit_after_undo_drops_the_redo_trail() {
        let mut editor = ComponentEditorText::opened(b"a");
        editor.key(ComponentEditorKey::End);
        editor.insert('b');
        assert!(editor.undo());
        assert_eq!(editor.lines(), ["a"]);

        editor.insert('c');
        assert_eq!(editor.lines(), ["ac"]);
        assert!(
            !editor.redo(),
            "the `b` that was undone is not reachable after a different edit"
        );
    }

    /// Cut, copy and paste are pure string operations here on purpose: the
    /// window reaches the real clipboard, and a test that did would be reading
    /// and writing the developer's own (clonk-org/clonk-rs#389).
    #[test]
    fn cut_and_copy_take_the_selection_and_paste_puts_text_where_the_caret_is() {
        let mut editor = ComponentEditorText::opened(b"hello world");
        for _ in 0..5 {
            editor.key_extending(ComponentEditorKey::Right, true);
        }

        assert_eq!(editor.copy_selection().as_deref(), Some("hello"));
        assert_eq!(
            editor.lines(),
            ["hello world"],
            "copy leaves the text exactly as it was"
        );
        assert!(!editor.modified(), "and does not mark the component");

        assert_eq!(editor.cut_selection().as_deref(), Some("hello"));
        assert_eq!(editor.lines(), [" world"]);
        assert_eq!(editor.caret(), (0, 0));
        assert!(editor.modified());

        editor.paste("goodbye");
        assert_eq!(editor.lines(), ["goodbye world"]);
        assert_eq!(editor.caret(), (0, 7), "the caret follows the pasted text");
    }

    /// With nothing selected there is nothing to take, and neither operation
    /// may fall back to something surprising like the whole line.
    #[test]
    fn cut_and_copy_do_nothing_without_a_selection() {
        let mut editor = ComponentEditorText::opened(b"hello");
        assert_eq!(editor.copy_selection(), None);
        assert_eq!(editor.cut_selection(), None);
        assert_eq!(editor.lines(), ["hello"]);
        assert!(!editor.modified());
    }

    /// Pasted newlines split lines exactly as typing them would, and a paste
    /// over a selection replaces it.
    #[test]
    fn a_multi_line_paste_splits_lines_and_replaces_a_selection() {
        let mut editor = ComponentEditorText::opened(b"ab");
        editor.key(ComponentEditorKey::Right);
        editor.paste("X\nY");
        assert_eq!(editor.lines(), ["aX", "Yb"]);
        assert_eq!(editor.caret(), (1, 1));

        let mut editor = ComponentEditorText::opened(b"hello world");
        for _ in 0..5 {
            editor.key_extending(ComponentEditorKey::Right, true);
        }
        editor.paste("hi");
        assert_eq!(editor.lines(), ["hi world"]);
        assert_eq!(editor.selection(), None);
    }

    /// The caret sits between glyphs, which in a proportional font means
    /// measuring the text before it rather than counting columns
    /// (clonk-org/clonk-rs#389).
    #[test]
    fn the_caret_is_measured_between_glyphs_not_pinned_to_the_line_start() {
        let font = BitmapFont::new();
        let at = |column| ComponentEditorText::caret_offset("hello", column, &font);

        assert_eq!(at(0), 0, "column zero is the left edge of the text");
        assert!(at(3) > at(0), "three characters in is past the start");
        assert!(at(5) > at(3), "and five is past three");
        assert_eq!(
            at(5),
            font.measure_text("hello", SMALL_FONT_SIZE).width as i32,
            "the end of the line is the full measured advance of it"
        );
        assert_eq!(
            at(99),
            at(5),
            "a column past the end clamps rather than measuring nothing"
        );
    }

    /// Held shift extends a selection from where the caret was; an unshifted
    /// move drops it. There is no oracle for any of this — the Win32 dialog is
    /// an `EDITTEXT` — so it follows what every other text field does
    /// (clonk-org/clonk-rs#389).
    #[test]
    fn shift_extends_a_selection_and_an_unshifted_move_drops_it() {
        let mut editor = ComponentEditorText::opened(b"hello world");
        assert_eq!(editor.selection(), None, "a fresh editor has no selection");

        for _ in 0..5 {
            editor.key_extending(ComponentEditorKey::Right, true);
        }
        assert_eq!(editor.selection(), Some(((0, 0), (0, 5))));
        assert_eq!(editor.selected_text().as_deref(), Some("hello"));

        // Moving without shift collapses it rather than leaving a stale range
        // behind the caret.
        editor.key_extending(ComponentEditorKey::Right, false);
        assert_eq!(editor.selection(), None);
        assert_eq!(editor.selected_text(), None);
    }

    /// A selection made backwards reads the same as one made forwards: the
    /// range is ordered, so callers never have to sort it themselves.
    #[test]
    fn a_backwards_selection_reports_the_same_ordered_range() {
        let mut editor = ComponentEditorText::opened(b"hello world");
        for _ in 0..5 {
            editor.key_extending(ComponentEditorKey::Right, false);
        }
        for _ in 0..3 {
            editor.key_extending(ComponentEditorKey::Left, true);
        }
        assert_eq!(editor.selection(), Some(((0, 2), (0, 5))));
        assert_eq!(editor.selected_text().as_deref(), Some("llo"));
    }

    /// Typing over a selection replaces it, which is the behaviour that makes
    /// selection worth having at all.
    #[test]
    fn typing_over_a_selection_replaces_it() {
        let mut editor = ComponentEditorText::opened(b"hello world");
        for _ in 0..5 {
            editor.key_extending(ComponentEditorKey::Right, true);
        }
        editor.insert('H');
        assert_eq!(editor.lines(), ["H world"]);
        assert_eq!(editor.caret(), (0, 1));
        assert_eq!(editor.selection(), None);
        assert!(editor.modified());
    }

    /// Backspace and delete both take the selection when there is one, instead
    /// of eating one further character beside it.
    #[test]
    fn backspace_over_a_selection_takes_the_selection_and_nothing_more() {
        let mut editor = ComponentEditorText::opened(b"hello world");
        for _ in 0..5 {
            editor.key_extending(ComponentEditorKey::Right, true);
        }
        editor.key(ComponentEditorKey::Backspace);
        assert_eq!(editor.lines(), [" world"]);
        assert_eq!(editor.caret(), (0, 0));

        let mut editor = ComponentEditorText::opened(b"hello world");
        for _ in 0..5 {
            editor.key_extending(ComponentEditorKey::Right, true);
        }
        editor.key(ComponentEditorKey::Delete);
        assert_eq!(editor.lines(), [" world"]);
        assert_eq!(editor.caret(), (0, 0));
    }

    /// A selection that spans lines joins them, so the text left behind is the
    /// head of the first line and the tail of the last.
    #[test]
    fn a_selection_across_lines_joins_what_is_left_of_them() {
        let mut editor = ComponentEditorText::opened(b"first\nsecond\nthird");
        editor.key_extending(ComponentEditorKey::End, false);
        editor.key_extending(ComponentEditorKey::Down, true);
        editor.key_extending(ComponentEditorKey::Down, true);
        assert_eq!(editor.selection(), Some(((0, 5), (2, 5))));
        editor.insert('!');
        assert_eq!(editor.lines(), ["first!"]);
        assert_eq!(editor.caret(), (0, 6));
    }

    #[test]
    fn component_editor_opens_on_the_component_bytes_and_round_trips_them() {
        let editor = ComponentEditorText::opened(b"func Initialize()\n{\n}\n");
        assert_eq!(
            editor.lines(),
            ["func Initialize()", "{", "}", ""],
            "a trailing newline is a real empty last line"
        );
        assert!(!editor.modified());
        assert_eq!(
            editor.bytes(),
            b"func Initialize()\n{\n}\n",
            "an untouched component commits the bytes it opened with"
        );
        // CRLF is normalised on the way in, as every other component reader
        // in the engine does.
        assert_eq!(ComponentEditorText::opened(b"a\r\nb").lines(), ["a", "b"]);
        // An empty component still has one line to put the caret on.
        assert_eq!(ComponentEditorText::opened(b"").lines(), [""]);
    }

    #[test]
    fn component_editor_typing_splitting_and_joining_move_the_caret_with_them() {
        let mut editor = ComponentEditorText::opened(b"ab\ncd");
        assert_eq!(editor.caret(), (0, 0));

        editor.insert('X');
        assert_eq!(editor.lines()[0], "Xab");
        assert_eq!(editor.caret(), (0, 1));
        assert!(editor.modified());

        // Enter splits at the caret and carries the tail down.
        editor.key(ComponentEditorKey::Enter);
        assert_eq!(editor.lines(), ["X", "ab", "cd"]);
        assert_eq!(editor.caret(), (1, 0));

        // Backspace at column zero joins back onto the line above, and the
        // caret lands at the join.
        editor.key(ComponentEditorKey::Backspace);
        assert_eq!(editor.lines(), ["Xab", "cd"]);
        assert_eq!(editor.caret(), (0, 1));

        // Delete at the end of a line pulls the next one up.
        editor.key(ComponentEditorKey::End);
        editor.key(ComponentEditorKey::Delete);
        assert_eq!(editor.lines(), ["Xabcd"]);

        // Both edges hold: nothing moves past the start or the end.
        editor.key(ComponentEditorKey::Home);
        editor.key(ComponentEditorKey::Left);
        editor.key(ComponentEditorKey::Up);
        assert_eq!(editor.caret(), (0, 0));
        editor.key(ComponentEditorKey::Backspace);
        assert_eq!(editor.lines(), ["Xabcd"]);
        editor.key(ComponentEditorKey::End);
        editor.key(ComponentEditorKey::Right);
        editor.key(ComponentEditorKey::Down);
        assert_eq!(editor.caret(), (0, 5));
        editor.key(ComponentEditorKey::Delete);
        assert_eq!(editor.lines(), ["Xabcd"]);

        // A control character has no glyph and never reaches the component.
        let before = editor.bytes();
        editor.insert('\t');
        editor.insert('\u{7}');
        assert_eq!(editor.bytes(), before);
    }

    // A multi-byte character moves the caret one step and is deleted whole —
    // the caret is in characters, the edit in bytes.
    #[test]
    fn component_editor_steps_over_a_multi_byte_character_whole() {
        let mut editor = ComponentEditorText::opened("ä".as_bytes());
        editor.key(ComponentEditorKey::End);
        assert_eq!(editor.caret(), (0, 1));
        editor.key(ComponentEditorKey::Backspace);
        assert_eq!(editor.lines(), [""]);
        assert_eq!(editor.caret(), (0, 0));
    }

    #[test]
    fn component_editor_scrolls_to_its_caret_and_renders_at_any_extent() {
        let font = BitmapFont::new();
        let source = (0..200)
            .map(|line| format!("line {line}"))
            .collect::<Vec<_>>()
            .join("\n");
        let mut editor = ComponentEditorText::opened(source.as_bytes());
        for _ in 0..150 {
            editor.key(ComponentEditorKey::Down);
        }
        assert_eq!(editor.caret().0, 150);
        for (width, height) in [(EDITOR_WIDTH, EDITOR_HEIGHT), (1, 1), (80, 60)] {
            let mut surface = Surface::new(width, height, PixelFormat::Rgba8888);
            editor.render(&mut surface, &font, "Script");
        }
        // The caret's line is on screen after a render at a real extent.
        let rows = ((EDITOR_HEIGHT as i32 - PADDING * 2 - STATUS_HEIGHT - 2) / ROW_HEIGHT) as usize;
        assert!(editor.first_visible <= 150 && 150 < editor.first_visible + rows);
    }
}
