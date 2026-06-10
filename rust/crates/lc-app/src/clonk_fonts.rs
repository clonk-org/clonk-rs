//! FreeType rasterization for the CStdFont-faithful GUI fonts.
//!
//! Mirrors `CStdFont::Init` + `AddRenderedChar` (src/StdFont.cpp:182-267,
//! 319-446): unhinted FreeType rendering (`FT_LOAD_RENDER |
//! FT_LOAD_NO_HINTING`) at `FT_Set_Pixel_Sizes(h, h)`, composed into glyph
//! cells with the baked drop shadow by
//! `lc_graphics::clonk_font::compose_glyph_cell`.

use anyhow::{Context, Result};
use freetype::face::LoadFlag;
use freetype::Library;
use lc_frontend::ClonkFontSet;
use lc_graphics::clonk_font::{compose_glyph_cell, line_height_for, ClonkFont, GlyphCell};

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
    use std::path::PathBuf;

    fn endeavour_bytes() -> Vec<u8> {
        let path = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("../../../planet/System.c4g/Endeavour.ttf");
        std::fs::read(path).expect("read Endeavour.ttf")
    }

    #[test]
    fn font_set_matches_cpp_line_heights() {
        let set = build_font_set(&endeavour_bytes()).expect("build font set");
        // (1303 - (-308)) * size / 1024 (StdFont.cpp:351).
        assert_eq!(set.title.line_height, 34);
        assert_eq!(set.caption.line_height, 25);
        assert_eq!(set.text.line_height, 22);
        assert_eq!(set.main_small.line_height, 20);
        assert_eq!(set.mini.line_height, 18);
        assert_eq!(set.title.cell_height, 35);
        assert_eq!(set.title.h_space, -1);
    }

    #[test]
    fn glyph_cells_have_shadowed_white_cores() {
        let set = build_font_set(&endeavour_bytes()).expect("build font set");
        let cell = set.title.glyph('A').expect("glyph A");
        assert!(cell.width > 5, "A should be wider than 5px");
        // The glyph core bakes to 254 (BltAlpha >>8 quirk) or 255 (pure src),
        // with full alpha; verify some near-white fully-opaque pixel exists.
        assert!(
            cell.pixels
                .iter()
                .any(|p| p.r >= 254 && p.g >= 254 && p.b >= 254 && p.a == 255),
            "expected an opaque white core pixel in 'A'"
        );
        // And the shadow: some dark, partially transparent pixel.
        assert!(
            cell.pixels
                .iter()
                .any(|p| p.r == 0 && p.a > 0 && p.a < 255),
            "expected a translucent black shadow pixel in 'A'"
        );
    }
}
