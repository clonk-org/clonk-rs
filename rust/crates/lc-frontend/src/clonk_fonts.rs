//! The GUI font set, mirroring `C4GraphicsResource::InitFonts` /
//! `C4GUI::Resource` (C4GraphicsResource.cpp:144-169, C4GuiResource.h:48-57).
//!
//! All sizes derive from the base font size 14 (`Config.General.RXFontSize`,
//! C4Config.cpp:391) via C4Fonts.cpp:280-288: Log 12, MainSmall 13, Main 14,
//! Caption 16, Title 22.

use anyhow::{Context, Result};
use freetype::face::LoadFlag;
use freetype::Library;
use lc_graphics::clonk_font::{
    compose_glyph_cell, line_height_for, scaled_font_image_width, ClonkFont, FontImageProvider,
    FontImageRef, GlyphCell, TextAlign,
};
use lc_graphics::{Color, GammaRamp, Surface};
use std::collections::BTreeSet;

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

/// One CStdFont rasterized at the application's integer output scale.
///
/// C++ keeps the glyph atlas in physical pixels while all public metrics and
/// draw coordinates remain in GUI units (`StdFont.cpp:319-352,571-638,841-842,
/// 938`). The Rust renderer uses this type only for the physical overlay pass;
/// the ordinary [`ClonkFontSet`] remains the scale-1 logical renderer.
pub struct NativeClonkFont {
    raster: ClonkFont,
    scale: u32,
}

impl NativeClonkFont {
    /// Integer denominator used by CStdFont's scale-native GUI metrics.
    pub(crate) fn message_width_units_per_gui_pixel(&self) -> i32 {
        self.scale as i32
    }

    /// One `BreakMessage` character advance in physical numerator units.
    /// C++ accumulates `facet.Wdt / scale + iHSpace`, where the shadowed
    /// font's `iHSpace` remains -1 GUI pixel (`StdFont.cpp:640-760`). Keeping
    /// the numerator avoids losing the fractional width before the wrap test.
    pub(crate) fn message_character_advance_units(&self, character: char) -> i32 {
        if character < ' ' {
            return 0;
        }
        self.raster
            .measure(&character.to_string(), false)
            .0
            .saturating_add(self.raster.h_space)
    }

    pub(crate) fn message_image_advance_units(&self, image: FontImageRef<'_>) -> i32 {
        scaled_font_image_width(self.raster.cell_height, image)
    }

    /// CStdFont's internal `iLineHgt`, in physical atlas pixels.
    pub fn raster_line_height(&self) -> i32 {
        self.raster.line_height
    }

    /// CStdFont's internal `iGfxLineHgt`, in physical atlas pixels.
    pub fn raster_cell_height(&self) -> i32 {
        self.raster.cell_height
    }

    /// `CStdFont::GetLineHeight`: internal height divided by application scale.
    pub fn logical_line_height(&self) -> i32 {
        self.raster.line_height / self.scale as i32
    }

    pub fn glyph(&self, ch: char) -> Option<&GlyphCell> {
        self.raster.glyph(ch)
    }

    /// `CStdFont::GetTextExtent` in GUI units. Physical glyph widths include
    /// the scaled shadow and physical spacing; one final integer division is
    /// equivalent to C++'s per-glyph float division for an integer scale.
    pub fn measure(&self, text: &str, markup: bool) -> (i32, i32) {
        self.measure_impl(text, markup, None)
    }

    pub fn measure_with_images(
        &self,
        text: &str,
        markup: bool,
        images: &dyn FontImageProvider,
    ) -> (i32, i32) {
        self.measure_impl(text, markup, Some(images))
    }

    fn measure_impl(
        &self,
        text: &str,
        markup: bool,
        images: Option<&dyn FontImageProvider>,
    ) -> (i32, i32) {
        let (width, height) = images.map_or_else(
            || self.raster.measure(text, markup),
            |images| self.raster.measure_with_images(text, markup, images),
        );
        let scale = self.scale as i32;
        let lines = if self.raster.line_height > 0 {
            height / self.raster.line_height
        } else {
            0
        };
        (
            width / scale,
            lines.saturating_mul(self.raster.line_height.saturating_add(scale - 1) / scale),
        )
    }

    /// Draw a native-resolution glyph run onto a physical surface while
    /// accepting C++ GUI-unit coordinates.
    #[allow(clippy::too_many_arguments)]
    pub fn draw_to_physical_surface(
        &self,
        surface: &mut Surface,
        x: i32,
        y: i32,
        text: &str,
        color: [u8; 4],
        align: TextAlign,
        markup: bool,
        gamma: Option<&GammaRamp>,
    ) {
        self.draw_to_physical_surface_with_offset(
            surface,
            x,
            y,
            text,
            color,
            align,
            markup,
            (0, 0),
            gamma,
        );
    }

    /// Physical-surface draw with the framebuffer offset of C++'s GL
    /// viewport. A negative Y offset represents rows clipped from the top.
    #[allow(clippy::too_many_arguments)]
    pub fn draw_to_physical_surface_with_offset(
        &self,
        surface: &mut Surface,
        x: i32,
        y: i32,
        text: &str,
        color: [u8; 4],
        align: TextAlign,
        markup: bool,
        physical_offset: (i32, i32),
        gamma: Option<&GammaRamp>,
    ) {
        self.draw_to_physical_surface_with_offset_impl(
            surface,
            x,
            y,
            text,
            color,
            align,
            markup,
            physical_offset,
            gamma,
            None,
        );
    }

    /// [`Self::draw_to_physical_surface_with_offset`] with custom images.
    #[allow(clippy::too_many_arguments)]
    pub fn draw_to_physical_surface_with_offset_and_images(
        &self,
        surface: &mut Surface,
        x: i32,
        y: i32,
        text: &str,
        color: [u8; 4],
        align: TextAlign,
        markup: bool,
        physical_offset: (i32, i32),
        gamma: Option<&GammaRamp>,
        images: &dyn FontImageProvider,
    ) {
        self.draw_to_physical_surface_with_offset_impl(
            surface,
            x,
            y,
            text,
            color,
            align,
            markup,
            physical_offset,
            gamma,
            Some(images),
        );
    }

    #[allow(clippy::too_many_arguments)]
    fn draw_to_physical_surface_with_offset_impl(
        &self,
        surface: &mut Surface,
        x: i32,
        y: i32,
        text: &str,
        color: [u8; 4],
        align: TextAlign,
        markup: bool,
        physical_offset: (i32, i32),
        gamma: Option<&GammaRamp>,
        images: Option<&dyn FontImageProvider>,
    ) {
        let scale = self.scale as i32;
        let line_height = self.logical_line_height();
        let origins = text
            .split(|character: char| character == '\n' || (markup && character == '|'))
            .enumerate()
            .map(|(line_index, line)| {
                let logical_width = self.measure_impl(line, markup, images).0;
                let logical_left = x.saturating_sub(match align {
                    TextAlign::Left => 0,
                    TextAlign::Center => logical_width / 2,
                    TextAlign::Right => logical_width,
                });
                let line_index = i32::try_from(line_index).unwrap_or(i32::MAX);
                let logical_y = y.saturating_add(line_index.saturating_mul(line_height));
                (
                    logical_left
                        .saturating_mul(scale)
                        .saturating_add(physical_offset.0),
                    logical_y
                        .saturating_mul(scale)
                        .saturating_add(physical_offset.1),
                )
            })
            .collect::<Vec<_>>();
        if let Some(images) = images {
            self.raster.draw_lines_at_origins_with_gamma_and_images(
                surface, &origins, text, color, markup, gamma, images,
            );
        } else {
            self.raster
                .draw_lines_at_origins_with_gamma(surface, &origins, text, color, markup, gamma);
        }
    }

    /// `CStdDDraw::StringOut` variant. Alignment uses `GetTextExtent` (where
    /// newline and markup-enabled `|` are virtual breaks), but the one
    /// `CStdFont::DrawText` call ignores newline and draws `|` on the current
    /// row. This differs deliberately from [`Self::draw_to_physical_surface`]
    /// and is used by `C4LoaderScreen` title/progress strings.
    #[allow(clippy::too_many_arguments)]
    pub fn draw_string_to_physical_surface(
        &self,
        surface: &mut Surface,
        x: i32,
        y: i32,
        text: &str,
        color: [u8; 4],
        align: TextAlign,
        markup: bool,
        gamma: Option<&GammaRamp>,
    ) {
        self.draw_string_to_physical_surface_with_offset(
            surface,
            x,
            y,
            text,
            color,
            align,
            markup,
            (0, 0),
            gamma,
        );
    }

    /// [`Self::draw_string_to_physical_surface`] with the framebuffer offset
    /// of an oversized C++ GL viewport.
    #[allow(clippy::too_many_arguments)]
    pub fn draw_string_to_physical_surface_with_offset(
        &self,
        surface: &mut Surface,
        x: i32,
        y: i32,
        text: &str,
        color: [u8; 4],
        align: TextAlign,
        markup: bool,
        physical_offset: (i32, i32),
        gamma: Option<&GammaRamp>,
    ) {
        let (logical_width, _) = self.measure(text, markup);
        let logical_left = x.saturating_sub(match align {
            TextAlign::Left => 0,
            TextAlign::Center => logical_width / 2,
            TextAlign::Right => logical_width,
        });
        let scale = self.scale as i32;
        if !text.contains('\n') && (!markup || !text.contains('|')) {
            self.raster.draw_with_gamma(
                surface,
                logical_left
                    .saturating_mul(scale)
                    .saturating_add(physical_offset.0),
                y.saturating_mul(scale).saturating_add(physical_offset.1),
                text,
                color,
                TextAlign::Left,
                markup,
                gamma,
            );
            return;
        }

        let sentinel = ('\u{E000}'..='\u{F8FF}')
            .find(|candidate| !text.contains(*candidate))
            .unwrap_or('\u{10FFFD}');
        let mut raster = self.raster.clone();
        if let Some(pipe) = self.raster.glyph('|').cloned() {
            raster.add_glyph(sentinel, pipe);
        }
        let transformed: String = text
            .chars()
            .filter_map(|character| match character {
                '\n' => None,
                '|' if markup => Some(sentinel),
                other => Some(other),
            })
            .collect();
        raster.draw_with_gamma(
            surface,
            logical_left
                .saturating_mul(scale)
                .saturating_add(physical_offset.0),
            y.saturating_mul(scale).saturating_add(physical_offset.1),
            &transformed,
            color,
            TextAlign::Left,
            markup,
            gamma,
        );
    }
}

/// The five GUI fonts rasterized at the application's physical output scale.
pub struct NativeClonkFontSet {
    pub title: NativeClonkFont,
    pub caption: NativeClonkFont,
    pub text: NativeClonkFont,
    pub main_small: NativeClonkFont,
    pub mini: NativeClonkFont,
    scale: u32,
}

impl NativeClonkFontSet {
    pub fn scale(&self) -> u32 {
        self.scale
    }

    /// C4GUI::Button chooses the largest logical font fitting height - 2
    /// (`C4GuiButton.cpp:100-108`).
    pub fn button_font(&self, button_height: i32) -> &NativeClonkFont {
        let text_height = button_height - 2;
        if self.title.logical_line_height() <= text_height {
            &self.title
        } else if self.caption.logical_line_height() <= text_height {
            &self.caption
        } else {
            &self.text
        }
    }
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

/// Characters materialized into the Rust cell map.
///
/// C++ pre-renders the active single-byte charset. In UTF-8 mode it instead
/// renders Unicode characters lazily through `GetUnicodeCharacterFacet`
/// (`StdFont.cpp:307-315,386-430`). Rust cells are independent allocations,
/// so eagerly materializing the face's bounded charmap is equivalent while
/// retaining the complete CP1252 map used by the default language.
fn classic_font_characters(face: &freetype::Face) -> BTreeSet<char> {
    let mut characters = (0x20_u16..=0xff)
        .filter_map(|byte| cp1252_to_char(byte as u8))
        .collect::<BTreeSet<_>>();
    characters.extend(face.chars().filter_map(|(charcode, _)| {
        u32::try_from(charcode)
            .ok()
            .and_then(char::from_u32)
            .filter(|character| *character >= ' ')
    }));
    characters
}

/// Convert the glyph currently loaded in `face` into a scale-one shadowed
/// CStdFont cell.
fn loaded_glyph_cell(
    face: &freetype::Face,
    cell_height: usize,
    ascent_px: i64,
) -> Option<GlyphCell> {
    let slot = face.glyph();
    let bitmap = slot.bitmap();
    if bitmap.rows() > 0 && bitmap.pixel_mode().ok() != Some(freetype::bitmap::PixelMode::Gray) {
        return None; // StdFont.cpp:211-216
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
    Some(GlyphCell {
        width: cell_w as i32,
        pixels,
    })
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
    for ch in classic_font_characters(face) {
        if face
            .load_char(ch as usize, LoadFlag::RENDER | LoadFlag::NO_HINTING)
            .is_err()
        {
            // C++ skips characters the font cannot render (StdFont.cpp:203-208).
            continue;
        }
        if let Some(cell) = loaded_glyph_cell(face, cell_height, ascent_px) {
            font.add_glyph(ch, cell);
        }
    }
    // FT_Load_Char maps an absent UTF-8 scalar to glyph index zero before
    // loading it. Reuse that one `.notdef` cell instead of retaining a live
    // mutable FreeType face solely for C++'s on-demand cache behavior.
    let missing_glyph = face
        .load_glyph(0, LoadFlag::RENDER | LoadFlag::NO_HINTING)
        .ok()
        .and_then(|_| loaded_glyph_cell(face, cell_height, ascent_px));
    if let Some(cell) = missing_glyph {
        font.set_missing_glyph(cell);
    }
    Ok(font)
}

/// CStdFont::AddRenderedChar's glyph-cell compositor for an application
/// scale greater than one (`StdFont.cpp:184,218-258`). Unlike the scale-1
/// helper in lc-graphics, the shadow sample is offset by `round(scale)`
/// physical pixels.
#[allow(clippy::too_many_arguments)]
fn compose_scaled_glyph_cell(
    cov: &[u8],
    cov_w: usize,
    cov_h: usize,
    cell_w: usize,
    cell_h: usize,
    at_x: usize,
    at_y: usize,
    shadow_size: usize,
) -> Vec<Color> {
    let Some(len) = cell_w.checked_mul(cell_h) else {
        return Vec::new();
    };
    let mut cell = vec![Color::transparent(); len];
    let coverage = |x: usize, y: usize| -> u32 {
        (x < cov_w && y < cov_h)
            .then(|| {
                y.checked_mul(cov_w)
                    .and_then(|row| row.checked_add(x))
                    .and_then(|index| cov.get(index))
            })
            .flatten()
            .map_or(0, |&value| u32::from(value))
    };
    for y in 0..cov_h.saturating_add(shadow_size) {
        for x in 0..cov_w.saturating_add(shadow_size) {
            let alpha_inverted = if x < cov_w && y < cov_h {
                255 - coverage(x, y)
            } else {
                255
            };
            let (base_grey, shadow_alpha_inverted) = if shadow_size > 0
                && x >= shadow_size
                && y >= shadow_size
            {
                let lower = shadow_size - 1;
                let upper = shadow_size + 1;
                let shadow = [
                    (x < cov_w && y < cov_h).then(|| coverage(x - lower, y - lower)),
                    (x > shadow_size && y < cov_h).then(|| coverage(x - upper, y - lower)),
                    (x > lower && y < cov_h).then(|| coverage(x - shadow_size, y - lower)),
                    (x < cov_w && y > shadow_size).then(|| coverage(x - lower, y - upper)),
                    (x > shadow_size && y > shadow_size).then(|| coverage(x - upper, y - upper)),
                    (x > lower && y > shadow_size).then(|| coverage(x - shadow_size, y - upper)),
                    (x < cov_w && y > lower).then(|| coverage(x - lower, y - shadow_size)),
                    (x > shadow_size && y > lower).then(|| coverage(x - upper, y - shadow_size)),
                    (x > lower && y > lower)
                        .then(|| coverage(x - shadow_size, y - shadow_size) * 8),
                ]
                .into_iter()
                .flatten()
                .sum::<u32>();
                ((255 - alpha_inverted) / 2, 255 - shadow / 16)
            } else {
                (0, 255)
            };
            let (r, g, b, out_alpha_inverted) = if shadow_alpha_inverted == 255 {
                (255, 255, 255, alpha_inverted)
            } else {
                let source_alpha = 255 - alpha_inverted;
                let mix = |destination: u32| {
                    ((255 * source_alpha + destination * alpha_inverted) >> 8).min(255)
                };
                (
                    mix(base_grey),
                    mix(base_grey),
                    mix(base_grey),
                    shadow_alpha_inverted.saturating_sub(source_alpha),
                )
            };
            if let (Some(target_x), Some(target_y)) = (at_x.checked_add(x), at_y.checked_add(y)) {
                if target_x < cell_w && target_y < cell_h {
                    cell[target_y * cell_w + target_x] =
                        Color::new(r as u8, g as u8, b as u8, (255 - out_alpha_inverted) as u8);
                }
            }
        }
    }
    cell
}

/// Convert the glyph currently loaded in `face` into a native-resolution
/// shadowed CStdFont cell.
fn loaded_native_glyph_cell(
    face: &freetype::Face,
    cell_height: usize,
    ascent_px: i64,
    scale: u32,
) -> Option<GlyphCell> {
    let slot = face.glyph();
    let bitmap = slot.bitmap();
    if bitmap.rows() > 0 && bitmap.pixel_mode().ok() != Some(freetype::bitmap::PixelMode::Gray) {
        return None;
    }
    let (cov_w, cov_h) = (bitmap.width() as usize, bitmap.rows() as usize);
    let pitch = bitmap.pitch();
    let buffer = bitmap.buffer();
    let cov: Vec<u8> = (0..cov_h)
        .flat_map(|y| {
            let start = (y as i32 * pitch) as usize;
            buffer[start..start + cov_w].iter().copied()
        })
        .collect();
    let advance_px = (slot.advance().x >> 6) as i32;
    let bearing = slot.bitmap_left().max(0);
    let cell_width = (advance_px.max(bearing + cov_w as i32) + scale as i32).max(1) as usize;
    let at_x = bearing as usize;
    let at_y = (ascent_px - i64::from(slot.bitmap_top())).max(0) as usize;
    let pixels = compose_scaled_glyph_cell(
        &cov,
        cov_w,
        cov_h,
        cell_width,
        cell_height,
        at_x,
        at_y,
        scale as usize,
    );
    Some(GlyphCell {
        width: cell_width as i32,
        pixels,
    })
}

fn build_native_font(
    face: &freetype::Face,
    logical_height: u32,
    scale: u32,
) -> Result<NativeClonkFont> {
    let raster_height = logical_height
        .checked_mul(scale)
        .context("scaled font height overflow")?;
    face.set_pixel_sizes(raster_height, raster_height)
        .context("FT_Set_Pixel_Sizes failed")?;

    let raw = face.raw();
    let units_per_em = i32::from(raw.units_per_EM);
    let (ascender, descender) = (i32::from(raw.ascender), i32::from(raw.descender));
    let line_height = line_height_for(ascender, descender, units_per_em, raster_height);
    // C++ deliberately adds one atlas row here, even when shadowSize is 3
    // (`StdFont.cpp:351-352`); AddRenderedChar clips its wider shadow loop.
    let cell_height = (line_height + 1) as usize;
    let ascent_px = i64::from(raster_height) * i64::from(ascender) / i64::from(units_per_em);

    let mut font = ClonkFont::new(line_height);
    // iHSpace remains -1 GUI unit in C++; this renderer advances in physical
    // pixels, so the equivalent native spacing is -scale.
    font.h_space = -(scale as i32);
    for ch in classic_font_characters(face) {
        if face
            .load_char(ch as usize, LoadFlag::RENDER | LoadFlag::NO_HINTING)
            .is_err()
        {
            continue;
        }
        if let Some(cell) = loaded_native_glyph_cell(face, cell_height, ascent_px, scale) {
            font.add_glyph(ch, cell);
        }
    }
    let missing_glyph = face
        .load_glyph(0, LoadFlag::RENDER | LoadFlag::NO_HINTING)
        .ok()
        .and_then(|_| loaded_native_glyph_cell(face, cell_height, ascent_px, scale));
    if let Some(cell) = missing_glyph {
        font.set_missing_glyph(cell);
    }
    Ok(NativeClonkFont {
        raster: font,
        scale,
    })
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

/// Builds `C4GraphicsResource::FontTooltip`: the Main-14 RX face initialized
/// independently with `fDoShadow = false` (`C4GraphicsResource.cpp:165`).
/// It is deliberately not borrowed from the startup book-font bundle because
/// the process-global GUI resource owns a separate `CStdFont` instance.
pub fn build_tooltip_font(ttf_bytes: &[u8]) -> Result<ClonkFont> {
    let library = Library::init().context("FreeType init failed")?;
    let face = library
        .new_memory_face(ttf_bytes.to_vec(), 0)
        .context("failed to load font face")?;
    crate::startup_scensel::build_shadowless_font(&face, 14)
}

/// Build the five GUI fonts the way C++ does when `Graphics.Scale` is an
/// integer greater than one: native physical raster data with logical GUI
/// metrics (`C4Fonts.cpp:158-173`; `StdFont.cpp:319-352,571-638,938`).
pub fn build_native_font_set(ttf_bytes: &[u8], scale: u32) -> Result<NativeClonkFontSet> {
    anyhow::ensure!(scale > 0, "font scale must be positive");
    let library = Library::init().context("FreeType init failed")?;
    let face = library
        .new_memory_face(ttf_bytes.to_vec(), 0)
        .context("failed to load font face")?;
    Ok(NativeClonkFontSet {
        title: build_native_font(&face, 22, scale)?,
        caption: build_native_font(&face, 16, scale)?,
        text: build_native_font(&face, 14, scale)?,
        main_small: build_native_font(&face, 13, scale)?,
        mini: build_native_font(&face, 12, scale)?,
        scale,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn endeavour_bytes() -> Vec<u8> {
        let path = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("../../../planet/System.c4g/Endeavour.ttf");
        std::fs::read(path).expect("read Endeavour.ttf")
    }

    #[test]
    fn scale_three_fonts_use_native_raster_shadow_and_logical_metrics() {
        // C4FontLoader passes Application.GetScale() to CStdFont::Init
        // (C4Fonts.cpp:158-173). CStdFont rasterizes at height*scale, uses
        // round(scale) for the shadow, and divides metrics back to GUI units
        // (StdFont.cpp:184,319-352,571-638,938).
        let fonts = build_native_font_set(&endeavour_bytes(), 3).expect("build 3x fonts");

        assert_eq!(fonts.scale(), 3);
        assert_eq!(fonts.title.raster_line_height(), 103);
        assert_eq!(fonts.title.raster_cell_height(), 104);
        assert_eq!(fonts.title.logical_line_height(), 34);
        assert_eq!(fonts.caption.logical_line_height(), 25);
        assert_eq!(fonts.text.logical_line_height(), 22);
        assert_eq!(fonts.main_small.logical_line_height(), 20);
        assert_eq!(fonts.mini.logical_line_height(), 18);
        assert!(
            fonts.text.glyph('\u{0100}').is_some(),
            "native FontRegular must include Endeavour's Unicode charmap"
        );
        assert!(
            fonts.text.measure("\u{1f642}", false).0 > 0,
            "native FontRegular must measure an unmapped scalar as glyph zero"
        );

        let base = build_font_set(&endeavour_bytes()).expect("build 1x fonts");
        assert!(
            fonts.title.glyph('A').expect("native A").width
                > base.title.glyph('A').expect("base A").width * 2,
            "the 3x font must contain a newly rasterized glyph, not a scaled 1x cell"
        );

        let cell = compose_scaled_glyph_cell(&[255], 1, 1, 5, 5, 0, 0, 3);
        assert_eq!(cell[6].a, 0, "3x shadow is not a 1px shadow");
        assert_eq!(
            cell[3 * 5 + 3],
            lc_graphics::Color::new(0, 0, 0, 127),
            "round(scale)=3 places the C++ shadow three physical pixels away"
        );
    }

    #[test]
    fn vector_font_covers_unicode_charmap_and_missing_glyph() {
        let fonts = build_font_set(&endeavour_bytes()).expect("build GUI fonts");

        assert!(
            fonts.text.glyph('\u{0100}').is_some(),
            "U+0100 is present in Endeavour but outside Windows-1252"
        );
        assert!(fonts.text.measure("\u{0100}", false).0 > 0);

        assert!(
            fonts.text.glyph('\u{1f642}').is_none(),
            "an unmapped scalar must remain distinguishable from direct coverage"
        );
        assert!(
            fonts.text.measure("\u{1f642}", false).0 > 0,
            "FT_Load_Char resolves an unmapped scalar through glyph index zero"
        );
    }

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
