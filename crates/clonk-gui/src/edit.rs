//! Shared C4GUI::Edit cursor and selection operations.
//!
//! Widgets retain their own focus, drag anchors, clocks and paste callbacks.
//! This view borrows their text state so those independent lifecycles do not
//! need to share a renderer or a generic widget implementation.

use std::ops::Range;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CursorOperation {
    Left,
    Right,
    Home,
    End,
}

pub fn selection_range(anchor: usize, caret: usize) -> Option<Range<usize>> {
    (anchor != caret).then(|| anchor.min(caret)..anchor.max(caret))
}

pub struct EditBuffer<'a> {
    text: &'a mut String,
    caret: &'a mut usize,
    anchor: &'a mut usize,
}

impl<'a> EditBuffer<'a> {
    /// Positions must be UTF-8 boundaries inside `text`, as in the owning edit.
    pub fn new(text: &'a mut String, caret: &'a mut usize, anchor: &'a mut usize) -> Self {
        Self {
            text,
            caret,
            anchor,
        }
    }

    pub fn delete_selection(&mut self) -> bool {
        let Some(range) = selection_range(*self.anchor, *self.caret) else {
            return false;
        };
        *self.caret = range.start;
        *self.anchor = range.start;
        self.text.replace_range(range, "");
        true
    }

    /// CharIn/Paste sanitation; selection deletion and its result belong to
    /// the caller, because rename and search report an empty insertion differently.
    pub fn insert_sanitized(&mut self, text: &str, max_bytes: usize) -> bool {
        let available = max_bytes.saturating_sub(self.text.len());
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
        self.text.insert_str(*self.caret, &sanitized);
        *self.caret += sanitized.len();
        *self.anchor = *self.caret;
        true
    }

    pub fn move_cursor(&mut self, operation: CursorOperation, ctrl: bool, shift: bool) {
        if !shift {
            *self.anchor = *self.caret;
        }
        let target = match operation {
            CursorOperation::Left if ctrl => word_boundary(self.text, *self.caret, -1),
            CursorOperation::Left => previous_boundary(self.text, *self.caret),
            CursorOperation::Right if ctrl => word_boundary(self.text, *self.caret, 1),
            CursorOperation::Right => next_boundary(self.text, *self.caret),
            CursorOperation::Home => 0,
            CursorOperation::End => self.text.len(),
        };
        *self.caret = target;
        if !shift {
            *self.anchor = target;
        }
    }

    /// Deletes without a selection. The owner first handles selection deletion
    /// so it can preserve the native hidden drag anchor separately.
    pub fn erase(&mut self, backwards: bool, ctrl: bool, shift: bool) -> bool {
        if shift
            || (backwards && *self.caret == 0)
            || (!backwards && *self.caret == self.text.len())
        {
            return false;
        }
        let target = if ctrl {
            word_boundary(self.text, *self.caret, if backwards { -1 } else { 1 })
        } else if backwards {
            previous_boundary(self.text, *self.caret)
        } else {
            next_boundary(self.text, *self.caret)
        };
        let start = target.min(*self.caret);
        let end = target.max(*self.caret);
        self.text.replace_range(start..end, "");
        *self.caret = start;
        *self.anchor = start;
        true
    }
}

pub fn truncate_utf8(text: &str, byte_limit: usize) -> &str {
    let mut end = text.len().min(byte_limit);
    while !text.is_char_boundary(end) {
        end -= 1;
    }
    &text[..end]
}

pub fn previous_boundary(text: &str, position: usize) -> usize {
    text[..position.min(text.len())]
        .char_indices()
        .next_back()
        .map_or(0, |(index, _)| index)
}

pub fn next_boundary(text: &str, position: usize) -> usize {
    if position >= text.len() {
        return text.len();
    }
    position + text[position..].chars().next().map_or(0, char::len_utf8)
}

pub fn char_at(text: &str, position: usize) -> char {
    text.get(position..)
        .and_then(|tail| tail.chars().next())
        .unwrap_or('\0')
}

pub fn is_word_spacer(character: char) -> bool {
    character.is_ascii() && !character.is_ascii_alphanumeric() && character != '_'
}

pub fn word_boundary(text: &str, caret: usize, direction: i8) -> usize {
    let mut position = caret;
    let mut nonspace_found = false;
    let mut space_found = false;
    loop {
        let next = if direction < 0 {
            if position == 0 {
                break;
            }
            previous_boundary(text, position)
        } else {
            if position >= text.len() {
                break;
            }
            next_boundary(text, position)
        };
        let sample = if direction < 0 { next } else { position };
        if is_word_spacer(char_at(text, sample)) {
            if nonspace_found && direction < 0 {
                break;
            }
            space_found = true;
        } else {
            if space_found && direction > 0 {
                break;
            }
            nonspace_found = true;
        }
        position = next;
    }
    position
}

pub fn word_selection(text: &str, mut position: usize) -> Option<Range<usize>> {
    position = position.min(text.len());
    if position < text.len() {
        let next = next_boundary(text, position);
        let character = text[position..next]
            .chars()
            .next()
            .expect("non-empty character slice");
        if is_word_spacer(character) {
            if position == 0 {
                return None;
            }
            let previous = previous_boundary(text, position);
            let character = text[previous..position]
                .chars()
                .next()
                .expect("non-empty character slice");
            if is_word_spacer(character) {
                return None;
            }
            position = previous;
        }
    } else if position > 0 {
        let previous = previous_boundary(text, position);
        let character = text[previous..position]
            .chars()
            .next()
            .expect("non-empty character slice");
        if is_word_spacer(character) {
            return None;
        }
        position = previous;
    } else {
        return None;
    }
    let mut start = position;
    while start > 0 {
        let previous = previous_boundary(text, start);
        let character = text[previous..start]
            .chars()
            .next()
            .expect("non-empty character slice");
        if is_word_spacer(character) {
            break;
        }
        start = previous;
    }
    let mut end = next_boundary(text, position);
    while end < text.len() {
        let next = next_boundary(text, end);
        let character = text[end..next]
            .chars()
            .next()
            .expect("non-empty character slice");
        if is_word_spacer(character) {
            break;
        }
        end = next;
    }
    Some(start..end)
}
