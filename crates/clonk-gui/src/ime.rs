//! IME composition: the provisional text an input method shows before it
//! commits anything.
//!
//! `WindowEvent::Ime::Preedit` carries text the user is still composing. It is
//! drawn in the focused field so they can see it, and it never enters that
//! field's own text — only `Ime::Commit` does, through the ordinary input path.
//! Every field that takes text therefore renders *its* text with the
//! composition spliced in at the caret, which is what [`compose`] produces.

use std::borrow::Cow;

/// An IME composition in progress, as `WindowEvent::Ime::Preedit` reports it.
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct ImeComposition {
    pub text: String,
    /// The IME's own cursor inside `text`, as a byte range. `None` is winit's
    /// "hide the cursor", and puts the caret after the whole composition.
    pub cursor: Option<(usize, usize)>,
}

impl ImeComposition {
    /// Where the caret belongs inside the composition, in bytes from its start.
    fn caret_offset(&self) -> usize {
        self.cursor
            .map_or(self.text.len(), |(start, _)| start.min(self.text.len()))
    }
}

/// A field's text with any composition spliced in at its caret.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ComposedText<'a> {
    /// What the field draws. Borrowed when nothing is being composed, which is
    /// every frame outside an IME session.
    pub text: Cow<'a, str>,
    /// The caret's byte offset within [`Self::text`].
    pub caret: usize,
    /// The composition's byte range within [`Self::text`] — what the underline
    /// spans — or `None` when nothing is being composed.
    pub composition: Option<(usize, usize)>,
}

/// Splices `composition` into `text` at `caret`.
///
/// With no composition this is `text` and `caret` unchanged, so a field that
/// always routes its drawing through here is pixel-for-pixel what it was
/// outside an IME session.
pub fn compose<'a>(
    text: &'a str,
    caret: usize,
    composition: Option<&ImeComposition>,
) -> ComposedText<'a> {
    let caret = caret.min(text.len());
    match composition.filter(|composition| !composition.text.is_empty()) {
        None => ComposedText {
            text: Cow::Borrowed(text),
            caret,
            composition: None,
        },
        Some(composition) => {
            // A caret the field reports mid-character would split a code point;
            // fall back to the field start rather than panicking on a slice.
            let caret = if text.is_char_boundary(caret) {
                caret
            } else {
                0
            };
            let mut composed = String::with_capacity(text.len() + composition.text.len());
            composed.push_str(&text[..caret]);
            composed.push_str(&composition.text);
            composed.push_str(&text[caret..]);
            ComposedText {
                text: Cow::Owned(composed),
                caret: caret + composition.caret_offset(),
                composition: Some((caret, caret + composition.text.len())),
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn nothing_composing_borrows_the_field_text_unchanged() {
        let composed = compose("ab", 1, None);

        assert_eq!(composed.text, Cow::Borrowed("ab"));
        assert!(matches!(composed.text, Cow::Borrowed(_)), "no allocation");
        assert_eq!(composed.caret, 1);
        assert_eq!(composed.composition, None);
    }

    #[test]
    fn a_composition_is_spliced_in_at_the_caret() {
        let composed = compose(
            "ab",
            1,
            Some(&ImeComposition {
                text: "\u{304b}".to_owned(),
                cursor: None,
            }),
        );

        assert_eq!(composed.text, "a\u{304b}b");
        assert_eq!(
            composed.caret,
            "a\u{304b}".len(),
            "with no IME cursor the caret sits after the composition"
        );
        assert_eq!(composed.composition, Some((1, 1 + "\u{304b}".len())));
    }

    #[test]
    fn the_ime_cursor_places_the_caret_inside_the_composition() {
        let composed = compose(
            "ab",
            2,
            Some(&ImeComposition {
                text: "\u{304b}\u{306a}".to_owned(),
                cursor: Some((0, 0)),
            }),
        );

        assert_eq!(composed.caret, 2, "a cursor at the start keeps it before");
        assert_eq!(
            composed.composition,
            Some((2, 2 + "\u{304b}\u{306a}".len())),
            "the underline spans the whole composition regardless of the cursor"
        );
    }

    /// An empty preedit is how an IME cancels; it must not leave a zero-width
    /// span for the underline to draw.
    #[test]
    fn an_empty_composition_reads_as_none() {
        let composed = compose("ab", 2, Some(&ImeComposition::default()));

        assert_eq!(composed.text, "ab");
        assert_eq!(composed.composition, None);
    }

    /// A caret past the end, or inside a multi-byte character, must not panic:
    /// both are reachable from a field whose text changed under the IME.
    #[test]
    fn an_impossible_caret_falls_back_rather_than_slicing_a_code_point() {
        let composing = ImeComposition {
            text: "x".to_owned(),
            cursor: None,
        };

        assert_eq!(compose("ab", 99, Some(&composing)).text, "abx");
        assert_eq!(compose("\u{304b}", 1, Some(&composing)).text, "x\u{304b}");
    }
}
