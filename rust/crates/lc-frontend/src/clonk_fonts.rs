//! The GUI font set, mirroring `C4GraphicsResource::InitFonts` /
//! `C4GUI::Resource` (C4GraphicsResource.cpp:144-169, C4GuiResource.h:48-57).
//!
//! All sizes derive from the base font size 14 (`Config.General.RXFontSize`,
//! C4Config.cpp:391) via C4Fonts.cpp:280-288: Log 12, MainSmall 13, Main 14,
//! Caption 16, Title 22.

use anyhow::{Context, Result};
use freetype::face::LoadFlag;
use freetype::Library;
use lc_graphics::clonk_font::{compose_glyph_cell, line_height_for, ClonkFont, GlyphCell};

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

/// Windows-1252 specials in 0x80..=0x9F; the rest of 0x80..=0xFF maps to the
/// same Unicode scalar. Mirrors the C++ iconv conversion of the legacy
/// charset (StdFont.cpp:386-401, default charset per C4Config).
fn cp1252_to_char(byte: u8) -> Option<char> {
    match byte {
        0x80 => Some('\u{20AC}'),
        0x82 => Some('\u{201A}'),
        0x83 => Some('\u{0192}'),
        0x84 => Some('\u{201E}'),
        0x85 => Some('\u{2026}'),
        0x86 => Some('\u{2020}'),
        0x87 => Some('\u{2021}'),
        0x88 => Some('\u{02C6}'),
        0x89 => Some('\u{2030}'),
        0x8A => Some('\u{0160}'),
        0x8B => Some('\u{2039}'),
        0x8C => Some('\u{0152}'),
        0x8E => Some('\u{017D}'),
        0x91 => Some('\u{2018}'),
        0x92 => Some('\u{2019}'),
        0x93 => Some('\u{201C}'),
        0x94 => Some('\u{201D}'),
        0x95 => Some('\u{2022}'),
        0x96 => Some('\u{2013}'),
        0x97 => Some('\u{2014}'),
        0x98 => Some('\u{02DC}'),
        0x99 => Some('\u{2122}'),
        0x9A => Some('\u{0161}'),
        0x9B => Some('\u{203A}'),
        0x9C => Some('\u{0153}'),
        0x9E => Some('\u{017E}'),
        0x9F => Some('\u{0178}'),
        0x81 | 0x8D | 0x8F | 0x90 | 0x9D => None,
        b if b >= 0x80 => Some(b as char),
        b => Some(b as char),
    }
}

/// Rasterizes one ClonkFont at `px_height` from `face`.
fn build_font(face: &freetype::Face, px_height: u32) -> Result<ClonkFont> {
    face.set_pixel_sizes(px_height, px_height)
        .context("FT_Set_Pixel_Sizes failed")?;

    let raw = face.raw();
    let units_per_em = i32::from(raw.units_per_EM);
    let (ascender, descender) = (i32::from(raw.ascender), i32::from(raw.descender));
    let line_height = line_height_for(ascender, descender, units_per_em, px_height);
    // iGfxLineHgt = iLineHgt + 1 for the shadow row (StdFont.cpp:352).
    let cell_height = (line_height + 1) as usize;
    // Baseline offset inside the cell (StdFont.cpp:221).
    let ascent_px = i64::from(px_height) * i64::from(ascender) / i64::from(units_per_em);

    let mut font = ClonkFont::new(line_height);
    for byte in 0x20u16..=0xFF {
        let Some(ch) = cp1252_to_char(byte as u8) else {
            continue;
        };
        if face
            .load_char(ch as usize, LoadFlag::RENDER | LoadFlag::NO_HINTING)
            .is_err()
        {
            // C++ skips characters the font cannot render (StdFont.cpp:203-208).
            continue;
        }
        let slot = face.glyph();
        let bitmap = slot.bitmap();
        if bitmap.rows() > 0 && bitmap.pixel_mode().ok() != Some(freetype::bitmap::PixelMode::Gray)
        {
            continue; // StdFont.cpp:211-216
        }

        let (cov_w, cov_h) = (bitmap.width() as usize, bitmap.rows() as usize);
        let pitch = bitmap.pitch();
        let buffer = bitmap.buffer();
        // Repack honoring the pitch (rows may be padded).
        let cov: Vec<u8> = (0..cov_h)
            .flat_map(|y| {
                let start = (y as i32 * pitch) as usize;
                buffer[start..start + cov_w].iter().copied()
            })
            .collect();

        // width = max(advance, bearing+width) + shadow (StdFont.cpp:218).
        let advance_px = (slot.advance().x >> 6) as i32;
        let bearing = slot.bitmap_left().max(0);
        let cell_w = (advance_px.max(bearing + cov_w as i32) + 1).max(1) as usize;
        let at_x = bearing as usize;
        let at_y = (ascent_px - i64::from(slot.bitmap_top())).max(0) as usize;

        let pixels = compose_glyph_cell(&cov, cov_w, cov_h, cell_w, cell_height, at_x, at_y);
        font.add_glyph(
            ch,
            GlyphCell {
                width: cell_w as i32,
                pixels,
            },
        );
    }
    Ok(font)
}

/// Builds the five GUI fonts from a TTF, sized from the base size 14 like
/// `C4Fonts.cpp:280-288` (Log 12, MainSmall 13, Main 14, Caption 16, Title 22).
pub fn build_font_set(ttf_bytes: &[u8]) -> Result<ClonkFontSet> {
    let library = Library::init().context("FreeType init failed")?;
    let face = library
        .new_memory_face(ttf_bytes.to_vec(), 0)
        .context("failed to load font face")?;
    Ok(ClonkFontSet {
        title: build_font(&face, 22)?,
        caption: build_font(&face, 16)?,
        text: build_font(&face, 14)?,
        main_small: build_font(&face, 13)?,
        mini: build_font(&face, 12)?,
    })
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
