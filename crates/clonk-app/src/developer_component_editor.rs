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
    first_visible: usize,
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
            first_visible: 0,
            modified: false,
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
        let (line, column) = self.caret;
        let Some(target) = self.lines.get_mut(line) else {
            return;
        };
        let offset = Self::byte_offset(target, column);
        target.insert(offset, character);
        self.caret = (line, column + 1);
        self.modified = true;
    }

    /// Apply one editing key.
    pub(crate) fn key(&mut self, key: ComponentEditorKey) {
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
                // The caret is a filled column at the start of its line rather
                // than between glyphs: measuring a proportional font per
                // character to place it exactly would cost more than it shows.
                fill(
                    surface,
                    IntRect {
                        w: 1,
                        ..IntRect {
                            x: rect.x + 1,
                            ..rect
                        }
                    },
                    MID_EDGE,
                );
            }
            draw_fitted_text(surface, font, rect, line, CONTROL_TEXT, SMALL_FONT_SIZE, 3);
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
