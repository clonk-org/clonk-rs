//! The GUI font set, mirroring `C4GraphicsResource::InitFonts` /
//! `C4GUI::Resource` (C4GraphicsResource.cpp:144-169, C4GuiResource.h:48-57).
//!
//! All sizes derive from the base font size 14 (`Config.General.RXFontSize`,
//! C4Config.cpp:391) via C4Fonts.cpp:280-288: Log 12, MainSmall 13, Main 14,
//! Caption 16, Title 22.

use lc_graphics::clonk_font::ClonkFont;

/// The five GUI fonts the startup menus draw with.
pub struct ClonkFontSet {
    /// C4FT_Title (22px) — `C4GUI::Resource::TitleFont`.
    pub title: ClonkFont,
    /// C4FT_Caption (16px) — `CaptionFont`.
    pub caption: ClonkFont,
    /// C4FT_Main (14px) — `TextFont`.
    pub text: ClonkFont,
    /// C4FT_MainSmall (13px) — used by the startup book fonts.
    pub main_small: ClonkFont,
    /// C4FT_Log (12px) — `MiniFont`.
    pub mini: ClonkFont,
}

impl ClonkFontSet {
    /// Picks the caption font for a button of the given height: the largest
    /// of Title/Caption/Text whose line height fits `height - 2`
    /// (Button::DrawElement, C4GuiButton.cpp:100-108).
    pub fn button_font(&self, button_height: i32) -> &ClonkFont {
        let text_height = button_height - 2;
        if self.title.line_height <= text_height {
            &self.title
        } else if self.caption.line_height <= text_height {
            &self.caption
        } else {
            &self.text
        }
    }
}

/// Expands a `&x` hotkey marker into the C++ markup highlight
/// `<c ffffff7f>x</c>` and returns the expanded label plus the (uppercased)
/// hotkey character (C4GUI::ExpandHotkeyMarkup, C4Gui.cpp:39-69).
pub fn expand_hotkey_markup(label: &str) -> (String, Option<char>) {
    label
        .find('&')
        .and_then(|pos| {
            let hotkey = label[pos + 1..].chars().next()?;
            let expanded = format!(
                "{}<c ffffff7f>{}</c>{}",
                &label[..pos],
                hotkey,
                &label[pos + 1 + hotkey.len_utf8()..]
            );
            Some((expanded, Some(hotkey.to_ascii_uppercase())))
        })
        .unwrap_or_else(|| (label.to_string(), None))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn expands_hotkey_marker_to_color_markup() {
        assert_eq!(
            expand_hotkey_markup("&Start Game"),
            ("<c ffffff7f>S</c>tart Game".to_string(), Some('S'))
        );
        assert_eq!(
            expand_hotkey_markup("E&xit"),
            ("E<c ffffff7f>x</c>it".to_string(), Some('X'))
        );
        assert_eq!(
            expand_hotkey_markup("No Marker"),
            ("No Marker".to_string(), None)
        );
    }

    #[test]
    fn button_font_prefers_largest_fitting_font() {
        let set = ClonkFontSet {
            title: ClonkFont::new(34),
            caption: ClonkFont::new(25),
            text: ClonkFont::new(22),
            main_small: ClonkFont::new(20),
            mini: ClonkFont::new(18),
        };
        // 40px button: title (34) fits 38.
        assert_eq!(set.button_font(40).line_height, 34);
        // 32px button: title doesn't fit 30, caption (25) does.
        assert_eq!(set.button_font(32).line_height, 25);
        // tiny button: falls back to text font.
        assert_eq!(set.button_font(20).line_height, 22);
    }
}
