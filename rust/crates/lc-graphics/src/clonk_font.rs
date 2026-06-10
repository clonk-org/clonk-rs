//! Pixel-exact reimplementation of the C++ `CStdFont` text renderer.
//!
//! Mirrors `src/StdFont.cpp` (glyph cell composition, text measurement and
//! drawing), `src/StdColors.h` (`BltAlpha`, `ModulateClrA`, `InvertRGBAAlpha`)
//! and `src/StdMarkup.cpp` (`CMarkup::Read`/`SkipTags`) for the engine's
//! standard configuration: shadow enabled (`fDoShadow`, shadowSize = 1),
//! `scale` = 1 and `iFontZoom` = 1 (`src/StdFont.cpp:129`).
//!
//! The C++ pipeline works with INVERTED alpha (0 = opaque, 255 = transparent);
//! every public boundary of this module uses NORMAL alpha (255 = opaque) and
//! conversions are performed exactly where the C++ does them.
//!
//! Out of scope (documented deviations):
//! - `<i>`/`</i>` italics: the C++ applies a shear transform
//!   (`src/StdMarkup.cpp:24-28`); we track the tag for nesting/closing
//!   semantics but render unsheared (no transform support).
//! - `{{id}}` inline images (`src/StdFont.cpp:601-622,870-897`): need a font
//!   image provider which is wired elsewhere; such sequences render literally.
//! - FreeType rasterization: callers supply the 8-bit coverage bitmap.

use crate::{Color, Surface};
use std::collections::HashMap;

/// Line height in pixels for a vector font, mirroring `CStdFont::Init`:
/// `iLineHgt = (ascender - descender) * dwHeight / units_per_EM` with C++
/// integer (truncating) division (`src/StdFont.cpp:351`).
///
/// Returns 0 for degenerate input (`units_per_em == 0` or an out-of-range
/// result) instead of panicking.
pub fn line_height_for(ascender: i32, descender: i32, units_per_em: i32, px_height: u32) -> i32 {
    (units_per_em != 0)
        .then(|| (ascender as i64 - descender as i64) * px_height as i64 / units_per_em as i64)
        .and_then(|height| i32::try_from(height).ok())
        .unwrap_or(0)
}

/// RGB modulation applied to a `<c ...>` markup tag color, mirroring
/// `ModulateClrA(dwBlitClr, dwAlphaMod)` in `CStdFont::DrawText`
/// (`src/StdFont.cpp:914`) with `dwAlphaMod = 0x00ffffff`
/// (`src/StdFont.cpp:824`, opaque base color): each channel becomes
/// `(c * 255) >> 8` (`src/StdColors.h:171-181`), so 255 → 254 and 127 → 126.
pub fn markup_blit_color(tag_rgb: [u8; 3]) -> [u8; 3] {
    tag_rgb.map(|c| ((c as u16 * 255) >> 8) as u8)
}

/// Compose one glyph cell from a FreeType-style 8-bit coverage bitmap,
/// mirroring the pixel loop of `CStdFont::AddRenderedChar`
/// (`src/StdFont.cpp:224-258`) with shadow enabled and shadowSize = 1.
///
/// `cov` is row-major, `cov_w`×`cov_h`, values 0..=255 (255 = full coverage).
/// `(at_x, at_y)` places the coverage's top-left corner inside the
/// `cell_w`×`cell_h` cell (the caller computes `max(bitmap_left, 0)` and
/// `ascent - bitmap_top`, `src/StdFont.cpp:221-222`).
///
/// Returns `cell_w * cell_h` pixels in NORMAL alpha over a transparent
/// background. (The C++ font texture background is transparent *white* —
/// `memset 0xff`, `src/C4Surface.cpp:1113` — but with alpha 0 the RGB never
/// contributes; we use transparent black.) Writes that fall outside the cell
/// and short/oversized `cov` slices are handled gracefully (missing coverage
/// reads as 0).
pub fn compose_glyph_cell(
    cov: &[u8],
    cov_w: usize,
    cov_h: usize,
    cell_w: usize,
    cell_h: usize,
    at_x: usize,
    at_y: usize,
) -> Vec<Color> {
    let Some(len) = cell_w.checked_mul(cell_h) else {
        return Vec::new();
    };
    let mut cell = vec![Color::transparent(); len];
    // Coverage lookup; out-of-range reads as 0 (no panic on short slices).
    let g = |x: usize, y: usize| -> u32 {
        (x < cov_w && y < cov_h)
            .then(|| {
                y.checked_mul(cov_w)
                    .and_then(|row| row.checked_add(x))
                    .and_then(|i| cov.get(i))
            })
            .flatten()
            .map_or(0, |&v| v as u32)
    };
    // Loop extends by shadowSize = 1 in both directions (src/StdFont.cpp:224-226).
    for y in 0..=cov_h {
        for x in 0..=cov_w {
            // Inverted text alpha: 255 - coverage inside the bitmap, else 255
            // (src/StdFont.cpp:228-232).
            let b_alpha = if x < cov_w && y < cov_h {
                255 - g(x, y)
            } else {
                255
            };
            // Base pixel under the glyph: a blurred shadow when inside the
            // shadow region (src/StdFont.cpp:236-254), else transparent black
            // (dwPixVal = 0, bAlphaShadow = 255; src/StdFont.cpp:234-235,254).
            let (base_grey, base_a_inv) = if x >= 1 && y >= 1 {
                // Shadow of the upper-left pixel blurred with its eight
                // neighbors, exactly src/StdFont.cpp:238-247 (shadowSize = 1).
                let i_shadow = [
                    (x < cov_w && y < cov_h).then(|| g(x, y)), // :239
                    (x > 1 && y < cov_h).then(|| g(x - 2, y)), // :240
                    (x > 0 && y < cov_h).then(|| g(x - 1, y)), // :241
                    (x < cov_w && y > 1).then(|| g(x, y - 2)), // :242
                    (x > 1 && y > 1).then(|| g(x - 2, y - 2)), // :243
                    (x > 0 && y > 1).then(|| g(x - 1, y - 2)), // :244
                    (x < cov_w && y > 0).then(|| g(x, y - 1)), // :245
                    (x > 1 && y > 0).then(|| g(x - 2, y - 1)), // :246
                    (x > 0 && y > 0).then(|| g(x - 1, y - 1) * 8), // :247
                ]
                .into_iter()
                .flatten()
                .sum::<u32>();
                // bAlphaShadow = 255 - iShadow / 16 (src/StdFont.cpp:248).
                // Shadow luminosity as if blitting on 50% grey:
                // cBack = 255 - bAlpha; RGB(cBack/2, ...) (src/StdFont.cpp:251-252).
                ((255 - b_alpha) / 2, 255 - i_shadow / 16)
            } else {
                (0, 255)
            };
            // BltAlpha(base, white | bAlpha<<24) in inverted alpha
            // (src/StdFont.cpp:255, src/StdColors.h:120-138).
            let (r, gr, b, a_inv) = if base_a_inv == 255 {
                // Fully transparent destination: result is the source
                // (src/StdColors.h:122-126).
                (255, 255, 255, b_alpha)
            } else {
                let a_dst = b_alpha; // source's inverted alpha (src/StdColors.h:128)
                let a_src = 255 - a_dst; // src/StdColors.h:129
                // out = min((src*aSrc + dst*aDst) >> 8, 255) with src = white
                // (src/StdColors.h:130-133).
                let mix = |dst_c: u32| ((255 * a_src + dst_c * a_dst) >> 8).min(255);
                (
                    mix(base_grey),
                    mix(base_grey),
                    mix(base_grey),
                    // out alpha = max(dst_a - aSrc, 0) (src/StdColors.h:134-137).
                    base_a_inv.saturating_sub(a_src),
                )
            };
            // Store at (at_x + x, at_y + y), inverted → normal alpha
            // (src/StdFont.cpp:256); writes outside the cell are skipped.
            if let (Some(tx), Some(ty)) = (at_x.checked_add(x), at_y.checked_add(y)) {
                if tx < cell_w && ty < cell_h {
                    cell[ty * cell_w + tx] =
                        Color::new(r as u8, gr as u8, b as u8, (255 - a_inv) as u8);
                }
            }
        }
    }
    cell
}

/// One pre-rendered glyph cell. `width` is the advance/blit width in pixels
/// (`pfctTarget->Wdt`, `src/StdFont.cpp:218,260`); the height is the owning
/// font's [`ClonkFont::cell_height`]. `pixels` is row-major,
/// `width * cell_height` NORMAL-alpha pixels (typically produced by
/// [`compose_glyph_cell`]).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GlyphCell {
    /// Blit width and horizontal advance basis of this glyph.
    pub width: i32,
    /// Row-major cell pixels (`width * cell_height`), NORMAL alpha.
    pub pixels: Vec<Color>,
}

/// Horizontal alignment for [`ClonkFont::draw`], mirroring
/// `STDFONT_CENTERED`/`STDFONT_RIGHTALGN` (`src/StdFont.h:30-31`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TextAlign {
    /// Pen starts at `x` (default C++ behavior).
    Left,
    /// `x -= sx / 2` with integer division (`src/StdFont.cpp:826-832`).
    Center,
    /// `x -= sx` (`src/StdFont.cpp:833-839`).
    Right,
}

/// A bitmap font equivalent to an initialized shadowed `CStdFont`
/// (`CStdFont::Init`, `src/StdFont.cpp:319-358`, `fDoShadow = true`).
#[derive(Debug, Clone)]
pub struct ClonkFont {
    /// `iLineHgt` (`src/StdFont.cpp:351`): vertical advance per text line.
    pub line_height: i32,
    /// `iGfxLineHgt = iLineHgt + 1` (`src/StdFont.cpp:352`): glyph cell height
    /// including the one-pixel vertical shadow.
    pub cell_height: i32,
    /// `iHSpace = -1` (`src/StdFont.cpp:327`): horizontal indent between
    /// characters (negative so adjacent shadows overlap).
    pub h_space: i32,
    cells: HashMap<char, GlyphCell>,
}

impl ClonkFont {
    /// Create an empty font with the given `iLineHgt`; `cell_height` and
    /// `h_space` follow the shadowed-font rules (`src/StdFont.cpp:327,352`).
    pub fn new(line_height: i32) -> Self {
        Self {
            line_height,
            cell_height: line_height.saturating_add(1),
            h_space: -1,
            cells: HashMap::new(),
        }
    }

    /// Register the glyph cell for `ch` (mirrors the per-character facets
    /// stored by `CStdFont::AddRenderedChar`, `src/StdFont.cpp:260`).
    pub fn add_glyph(&mut self, ch: char, cell: GlyphCell) {
        self.cells.insert(ch, cell);
    }

    /// Look up the glyph cell for `ch`, if rendered.
    pub fn glyph(&self, ch: char) -> Option<&GlyphCell> {
        self.cells.get(&ch)
    }

    /// Measure `text`, mirroring `CStdFont::GetTextExtent`
    /// (`src/StdFont.cpp:571-638`) with `scale = iFontZoom = 1`.
    ///
    /// Returns `(width, height)`. The width is the maximum over all rows of
    /// the per-character widths plus `h_space` after every character that has
    /// *any* remaining text in the whole string — including before `'\n'`
    /// (`if (*szText) iRowWdt += iHSpace`, `src/StdFont.cpp:630`). The height
    /// is `line_height` per row (`src/StdFont.cpp:583,596`). Characters
    /// without a glyph contribute width 0 (empty facet). With `markup`, valid
    /// tags are skipped (`src/StdFont.cpp:590`) and `'|'` breaks lines
    /// (`src/StdFont.cpp:596`).
    pub fn measure(&self, text: &str, markup: bool) -> (i32, i32) {
        let mut rest = text;
        let mut row_width: i32 = 0;
        let mut width: i32 = 0;
        let mut height = self.line_height; // src/StdFont.cpp:583
        loop {
            if markup {
                rest = skip_tags(rest); // src/StdFont.cpp:590
            }
            let mut chars = rest.chars();
            let Some(c) = chars.next() else { break }; // src/StdFont.cpp:592-594
            rest = chars.as_str();
            // Line break (src/StdFont.cpp:596).
            if c == '\n' || (markup && c == '|') {
                row_width = 0;
                height = height.saturating_add(self.line_height);
                continue;
            }
            // Ignore system characters (src/StdFont.cpp:598).
            if c < ' ' {
                continue;
            }
            // Character facet width (src/StdFont.cpp:627); missing glyph = 0.
            row_width = row_width.saturating_add(self.cells.get(&c).map_or(0, |g| g.width));
            // Horizontal indent for all but the last char of the whole string
            // (src/StdFont.cpp:630).
            if !rest.is_empty() {
                row_width = row_width.saturating_add(self.h_space);
            }
            width = width.max(row_width); // src/StdFont.cpp:632
        }
        (width, height)
    }

    /// Draw `text` onto `surface`, mirroring `CStdDDraw::TextOut`
    /// (`src/StdDDraw2.cpp:1035-1042`) + `CStdFont::DrawText`
    /// (`src/StdFont.cpp:814-934`) with `fZoom = 1`.
    ///
    /// `color` is NORMAL-alpha RGBA. Lines are split on `'\n'` (and `'|'` when
    /// `markup`, `src/StdDDraw2.cpp:1039`), each line is aligned independently
    /// and advances `y` by [`Self::line_height`]; markup state persists across
    /// lines (one `CMarkup` per `TextOut`, `src/StdDDraw2.cpp:1037`).
    ///
    /// Markup: `<c hex>` modulates glyph RGB by [`markup_blit_color`] of the
    /// tag RGB — but only when the tag color differs from the base color
    /// (`if (dwBlitClr != dwColor)`, `src/StdFont.cpp:910-915`); the glyph
    /// alpha stays modulated by `color`'s alpha only (tag alpha is ignored).
    /// `</c>` reverts. `<i>`/`</i>` are consumed but render unsheared. Invalid
    /// or unknown tags render literally (`src/StdFont.cpp:864-866`).
    #[allow(clippy::too_many_arguments)] // signature mirrors CStdFont::DrawText
    pub fn draw(
        &self,
        surface: &mut Surface,
        x: i32,
        y: i32,
        text: &str,
        color: [u8; 4],
        align: TextAlign,
        markup: bool,
    ) {
        self.draw_with_gamma(surface, x, y, text, color, align, markup, None);
    }

    /// [`ClonkFont::draw`] with the blit shader's per-fragment gamma lookup
    /// (StdGL.cpp:1082-1086) applied to the modulated glyph color before
    /// blending, exactly like the C++ GL pipeline.
    #[allow(clippy::too_many_arguments)]
    pub fn draw_with_gamma(
        &self,
        surface: &mut Surface,
        x: i32,
        y: i32,
        text: &str,
        color: [u8; 4],
        align: TextAlign,
        markup: bool,
        gamma: Option<&crate::GammaRamp>,
    ) {
        let mut stack: Vec<MarkupTag> = Vec::new(); // src/StdDDraw2.cpp:1037
        let mut line_y = y;
        for line in text.split(|c: char| c == '\n' || (markup && c == '|')) {
            self.draw_line(surface, x, line_y, line, color, align, markup, &mut stack, gamma);
            // iTy += fZoom * GetLineHeight() per line (src/StdDDraw2.cpp:1039).
            line_y = line_y.saturating_add(self.line_height);
        }
    }

    /// One `CStdFont::DrawText` call (`src/StdFont.cpp:814-934`).
    #[allow(clippy::too_many_arguments)]
    fn draw_line(
        &self,
        surface: &mut Surface,
        x: i32,
        y: i32,
        line: &str,
        color: [u8; 4],
        align: TextAlign,
        markup: bool,
        stack: &mut Vec<MarkupTag>,
        gamma: Option<&crate::GammaRamp>,
    ) {
        // Alignment uses the markup-aware extent of this line
        // (src/StdFont.cpp:826-839); sx / 2 is integer division.
        let (sx, _) = self.measure(line, markup);
        let mut pen_x = x - match align {
            TextAlign::Left => 0,
            TextAlign::Center => sx / 2, // src/StdFont.cpp:831
            TextAlign::Right => sx,      // src/StdFont.cpp:838
        };
        let mut rest = line;
        while let Some(c) = rest.chars().next() {
            let after = &rest[c.len_utf8()..];
            // Ignore system characters (src/StdFont.cpp:851).
            if c < ' ' {
                rest = after;
                continue;
            }
            // Markup tag (src/StdFont.cpp:853-866).
            if markup && c == '<' {
                if let Some(advance) = read_tag(rest, Some(stack)) {
                    rest = &rest[advance..];
                    continue;
                }
                // Invalid tag: fall through and render '<' as text.
            }
            rest = after;
            let cell = self.cells.get(&c);
            if let Some(cell) = cell {
                blit_cell(
                    surface,
                    cell,
                    self.cell_height,
                    pen_x,
                    y,
                    modulation_rgb(stack, color),
                    color[3],
                    gamma,
                );
            }
            // x += w2 + iHSpace (src/StdFont.cpp:927); empty facet → width 0.
            pen_x = pen_x
                .saturating_add(cell.map_or(0, |g| g.width))
                .saturating_add(self.h_space);
        }
    }
}

/// An open markup tag (`CMarkupTag`, `src/StdMarkup.h:30-66`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum MarkupTag {
    /// `<i>` — tracked for close-tag matching only (shear unsupported).
    Italic,
    /// `<c ...>` — color in inverted-alpha ARGB, as stored by
    /// `CMarkupTagColor` after `InvertRGBAAlpha` (`src/StdMarkup.cpp:94-96`).
    TextColor(u32),
}

impl MarkupTag {
    /// `CMarkupTag::TagName` (`src/StdMarkup.h:50,63`).
    fn name(self) -> &'static str {
        match self {
            MarkupTag::Italic => "i",
            MarkupTag::TextColor(_) => "c",
        }
    }
}

/// `InvertRGBAAlpha` (`src/StdColors.h:295-298`) applied to a NORMAL-alpha
/// RGBA color, as `DrawText` does on entry (`src/StdFont.cpp:819`); returns
/// inverted-alpha ARGB.
fn inverted_argb(color: [u8; 4]) -> u32 {
    ((255 - color[3] as u32) << 24)
        | ((color[0] as u32) << 16)
        | ((color[1] as u32) << 8)
        | color[2] as u32
}

/// Effective RGB blit modulation for the current markup state, mirroring
/// `src/StdFont.cpp:910-915`: `CMarkup::Apply` leaves the innermost color
/// tag's color (`src/StdMarkup.h:95-98`, applied first-to-last so the last
/// pushed wins), and `ModulateClrA` (the `(c*255)>>8` quirk) runs only when
/// that color differs from the base `dwColor`.
fn modulation_rgb(stack: &[MarkupTag], color: [u8; 4]) -> [u8; 3] {
    let base = inverted_argb(color);
    stack
        .iter()
        .rev()
        .find_map(|tag| match tag {
            MarkupTag::TextColor(clr) => Some(*clr),
            MarkupTag::Italic => None,
        })
        .filter(|&clr| clr != base) // src/StdFont.cpp:914
        .map(|clr| markup_blit_color([(clr >> 16) as u8, (clr >> 8) as u8, clr as u8]))
        .unwrap_or([color[0], color[1], color[2]])
}

/// `CMarkup::SkipTags` (`src/StdMarkup.cpp:109-115`): consume consecutive
/// valid tags in skip mode; stop at the first invalid one.
fn skip_tags(mut text: &str) -> &str {
    while text.starts_with('<') {
        match read_tag(text, None) {
            Some(advance) => text = &text[advance..],
            None => break,
        }
    }
    text
}

/// `CMarkup::Read` (`src/StdMarkup.cpp:36-107`). `text` must start at `'<'`.
/// `stack = None` is skip mode (`fSkip = true`): close tags match anything and
/// `<c ...>` parameters are not hex-validated (`src/StdMarkup.cpp:54,80`).
/// Returns the byte length to skip past the tag (`iTagLen + 2`,
/// `src/StdMarkup.cpp:104`), or `None` for an invalid tag (rendered as text).
fn read_tag(text: &str, stack: Option<&mut Vec<MarkupTag>>) -> Option<usize> {
    let inner = text.strip_prefix('<')?;
    // SCopyEnclosed: tag runs to the first '>' (src/C4Strings.cpp:425-433).
    let close = inner.find('>')?;
    let full = &inner[..close];
    // The C++ tag buffer holds 49 chars (src/StdMarkup.cpp:38,40); longer
    // contents are truncated. (Deviation: C++ truncates at byte 49 even inside
    // a UTF-8 sequence; we back up to the previous char boundary, which only
    // matters for >49-byte tags — all of which are invalid in draw mode.)
    let mut tag_len = full.len().min(49);
    while !full.is_char_boundary(tag_len) {
        tag_len -= 1;
    }
    let tag = &full[..tag_len];
    // *ppText += iTagLen + 2 (src/StdMarkup.cpp:104); for truncated tags this
    // may land mid-character in C++ — we round up to the next boundary.
    let mut advance = (tag_len + 2).min(text.len());
    while !text.is_char_boundary(advance) {
        advance += 1;
    }
    // Split into name and parameters at the first space (src/StdMarkup.cpp:44-48).
    let (name, pars) = match tag.find(' ') {
        Some(i) => (&tag[..i], Some(&tag[i + 1..])),
        None => (tag, None),
    };
    let valid = if let Some(closing) = name.strip_prefix('/') {
        // Closing tag: no parameters allowed (src/StdMarkup.cpp:50-53); in
        // skip mode the name/stack check is bypassed (src/StdMarkup.cpp:54).
        pars.is_none()
            && match stack {
                None => true,
                Some(stack) => match stack.last() {
                    // Must close the innermost open tag (src/StdMarkup.cpp:57-60).
                    Some(tag) if tag.name() == closing => {
                        stack.pop();
                        true
                    }
                    _ => false,
                },
            }
    } else if name == "i" {
        // Italic: no parameters (src/StdMarkup.cpp:64-70).
        pars.is_none() && {
            if let Some(stack) = stack {
                stack.push(MarkupTag::Italic);
            }
            true
        }
    } else if name == "c" {
        // Color (src/StdMarkup.cpp:72-98).
        match (pars, stack) {
            (None, _) => false,                      // :75
            (Some(p), _) if p.len() > 8 => false,    // :76-79
            (Some(_), None) => true,                 // skip mode: hex unchecked (:80)
            (Some(p), Some(stack)) => parse_color_tag(p)
                .map(|clr| stack.push(MarkupTag::TextColor(clr)))
                .is_some(),
        }
    } else {
        false // unknown tag (src/StdMarkup.cpp:99-100)
    };
    valid.then_some(advance)
}

/// Parse `<c ...>` parameters (`src/StdMarkup.cpp:83-94`): lowercase hex
/// digits only (`src/StdMarkup.cpp:87-89`, uppercase makes the tag invalid),
/// accumulated big-endian into a u32 — so 8 digits are **AARRGGBB** — with
/// alpha defaulting to 0xff for ≤6 digits (`src/StdMarkup.cpp:93`), then
/// `InvertRGBAAlpha` (`src/StdMarkup.cpp:94`). Returns inverted-alpha ARGB.
fn parse_color_tag(pars: &str) -> Option<u32> {
    let len = pars.len();
    pars.bytes()
        .enumerate()
        .try_fold(0u32, |clr, (i, b)| {
            let digit = match b {
                b'0'..=b'9' => b - b'0',
                b'a'..=b'f' => b - b'a' + 10,
                _ => return None,
            };
            Some(clr | ((digit as u32) << ((len - i - 1) * 4)))
        })
        .map(|clr| if len <= 6 { clr | 0xff00_0000 } else { clr })
        .map(|clr| (clr & 0x00ff_ffff) | ((255 - (clr >> 24)) << 24))
}

/// Blit one glyph cell at `(x, y)`, mirroring the GL character blit
/// (`src/StdFont.cpp:922-925`): texture RGBA modulated by the blit color
/// (`glColor` modulate, f32 round-to-nearest), then composited with
/// `glBlendFunc(GL_SRC_ALPHA, GL_ONE_MINUS_SRC_ALPHA)`. Clipped to the
/// surface; malformed `pixels` lengths are tolerated.
#[allow(clippy::too_many_arguments)]
fn blit_cell(
    surface: &mut Surface,
    cell: &GlyphCell,
    cell_height: i32,
    x: i32,
    y: i32,
    mod_rgb: [u8; 3],
    color_alpha: u8,
    gamma: Option<&crate::GammaRamp>,
) {
    let width = usize::try_from(cell.width).unwrap_or(0);
    let height = usize::try_from(cell_height).unwrap_or(0);
    for row in 0..height {
        for col in 0..width {
            let Some(&px) = row
                .checked_mul(width)
                .and_then(|o| o.checked_add(col))
                .and_then(|i| cell.pixels.get(i))
            else {
                continue;
            };
            // Glyph alpha modulated by the draw color's alpha only.
            let out_a = (px.a as f32 * color_alpha as f32 / 255.0).round();
            if out_a <= 0.0 {
                continue; // fully transparent source leaves the surface unchanged
            }
            let (Some(dx), Some(dy)) = (offset_coord(x, col), offset_coord(y, row)) else {
                continue;
            };
            let Some(dst) = surface.get_pixel(dx, dy) else {
                continue; // clipped
            };
            let af = out_a / 255.0;
            // Modulate in float; with a gamma ramp the result goes through
            // the shader's gamma texel lookup, else round like before.
            let modulate = |c: u8, m: u8| -> f32 {
                let v = c as f32 * m as f32 / 255.0;
                gamma.map_or_else(|| v.round(), |g| f32::from(g.encode_float(v)))
            };
            let blend = |src: f32, dst: u8| (src * af + dst as f32 * (1.0 - af)).round() as u8;
            let blended = Color::new(
                blend(modulate(px.r, mod_rgb[0]), dst.r),
                blend(modulate(px.g, mod_rgb[1]), dst.g),
                blend(modulate(px.b, mod_rgb[2]), dst.b),
                blend(out_a, dst.a),
            );
            let _ = surface.set_pixel(dx, dy, blended);
        }
    }
}

/// `base + offset` as a surface coordinate; `None` when negative/overflowing.
fn offset_coord(base: i32, offset: usize) -> Option<u32> {
    i32::try_from(offset)
        .ok()
        .and_then(|offset| base.checked_add(offset))
        .and_then(|v| u32::try_from(v).ok())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::PixelFormat;

    // ---- metrics (CStdFont::Init, src/StdFont.cpp:351) ----

    #[test]
    fn line_height_matches_endeavour_metrics() {
        // Endeavour.ttf: ascender 1303, descender -308, 1024 units/em.
        let height = |px| line_height_for(1303, -308, 1024, px);
        assert_eq!(height(22), 34);
        assert_eq!(height(16), 25);
        assert_eq!(height(14), 22);
        assert_eq!(height(13), 20);
        assert_eq!(height(12), 18);
    }

    #[test]
    fn line_height_handles_degenerate_input() {
        assert_eq!(line_height_for(1303, -308, 0, 22), 0);
    }

    // ---- markup color quirk (src/StdFont.cpp:914, src/StdColors.h:171-181) ----

    #[test]
    fn markup_blit_color_applies_shr8_quirk() {
        assert_eq!(markup_blit_color([255, 255, 127]), [254, 254, 126]);
        assert_eq!(markup_blit_color([0, 1, 128]), [0, 0, 127]);
    }

    // ---- glyph cell composition (src/StdFont.cpp:224-258) ----

    #[test]
    fn compose_full_coverage_matches_hand_computation() {
        // cov = [[255]], cell 2x2 at (0,0).
        let cell = compose_glyph_cell(&[255], 1, 1, 2, 2, 0, 0);
        // (0,0): bAlpha 0, base transparent → result = opaque white source.
        assert_eq!(cell[0], Color::new(255, 255, 255, 255));
        // (1,0)/(0,1): no shadow (needs x>=1 && y>=1), bAlpha 255 →
        // transparent white.
        assert_eq!(cell[1], Color::new(255, 255, 255, 0));
        assert_eq!(cell[2], Color::new(255, 255, 255, 0));
        // (1,1): iShadow = 8*255 = 2040 → /16 = 127 → inverted shadow alpha
        // 128; BltAlpha with fully transparent source keeps the black shadow:
        // normal alpha 127.
        assert_eq!(cell[3], Color::new(0, 0, 0, 127));
    }

    #[test]
    fn compose_half_coverage_matches_hand_computation() {
        // cov = [[128]], cell 2x2 at (0,0).
        let cell = compose_glyph_cell(&[128], 1, 1, 2, 2, 0, 0);
        // (0,0): bAlpha 127, base transparent → white with normal alpha 128.
        assert_eq!(cell[0], Color::new(255, 255, 255, 128));
        assert_eq!(cell[1], Color::new(255, 255, 255, 0));
        assert_eq!(cell[2], Color::new(255, 255, 255, 0));
        // (1,1): iShadow = 8*128 = 1024 → /16 = 64 → inverted alpha 191 →
        // black shadow with normal alpha 64.
        assert_eq!(cell[3], Color::new(0, 0, 0, 64));
    }

    #[test]
    fn compose_shadow_region_full_coverage_gets_254_quirk() {
        // cov 2x2 all 255, cell 3x3: the interior pixel (1,1) lies in the
        // shadow region; BltAlpha's >>8 yields 254, not 255.
        let cell = compose_glyph_cell(&[255; 4], 2, 2, 3, 3, 0, 0);
        assert_eq!(cell[0], Color::new(255, 255, 255, 255)); // (0,0)
        assert_eq!(cell[1], Color::new(255, 255, 255, 255)); // (1,0): y=0, no shadow
        assert_eq!(cell[3 + 1], Color::new(254, 254, 254, 255)); // (1,1)
        // (2,1)/(1,2)/(2,2): iShadow = 3*255 + 8*255 = 2805 → /16 = 175 →
        // black shadow, normal alpha 175.
        assert_eq!(cell[3 + 2], Color::new(0, 0, 0, 175)); // (2,1)
        assert_eq!(cell[6 + 1], Color::new(0, 0, 0, 175)); // (1,2)
        assert_eq!(cell[6 + 2], Color::new(0, 0, 0, 175)); // (2,2)
    }

    #[test]
    fn compose_respects_placement_and_clips_to_cell() {
        // 1x1 coverage placed at (1,1) in a 2x2 cell: the shadow pixels fall
        // outside and are dropped; untouched pixels stay transparent black.
        let cell = compose_glyph_cell(&[255], 1, 1, 2, 2, 1, 1);
        assert_eq!(cell[0], Color::transparent());
        assert_eq!(cell[1], Color::transparent());
        assert_eq!(cell[2], Color::transparent());
        assert_eq!(cell[3], Color::new(255, 255, 255, 255));
    }

    #[test]
    fn compose_tolerates_short_coverage_slice() {
        // Missing coverage reads as 0: everything fully transparent, no panic.
        let cell = compose_glyph_cell(&[], 2, 2, 3, 3, 0, 0);
        assert_eq!(cell.len(), 9);
        assert!(cell.iter().all(|px| px.a == 0));
    }

    // ---- measure (CStdFont::GetTextExtent, src/StdFont.cpp:571-638) ----

    fn test_font() -> ClonkFont {
        let mut font = ClonkFont::new(3);
        font.add_glyph(
            'A',
            GlyphCell {
                width: 5,
                pixels: vec![Color::opaque(255, 255, 255); 5 * 4],
            },
        );
        font.add_glyph(
            'B',
            GlyphCell {
                width: 4,
                pixels: vec![Color::opaque(255, 255, 255); 4 * 4],
            },
        );
        font
    }

    #[test]
    fn font_metrics_follow_shadow_rules() {
        let font = test_font();
        assert_eq!(font.cell_height, 4); // iGfxLineHgt = iLineHgt + 1
        assert_eq!(font.h_space, -1); // iHSpace = -1 with shadow
    }

    #[test]
    fn measure_sums_widths_with_h_space() {
        let font = test_font();
        assert_eq!(font.measure("AB", false), (8, 3)); // 5 - 1 + 4
        assert_eq!(font.measure("A", false), (5, 3));
        assert_eq!(font.measure("", false), (0, 3));
    }

    #[test]
    fn measure_adds_h_space_before_newline() {
        // h_space is added whenever more text follows — even a '\n'
        // (src/StdFont.cpp:630): row 1 measures 5 - 1 + 4 - 1 = 7.
        let font = test_font();
        assert_eq!(font.measure("AB\nA", false), (7, 6));
    }

    #[test]
    fn measure_markup_skips_tags() {
        let font = test_font();
        assert_eq!(font.measure("<c ffffff7f>A</c>B", true), (8, 3));
        // Without markup, tag characters are unknown glyphs (width 0) whose
        // h_space pulls the row negative; the max stays 0.
        assert_eq!(font.measure("<c ffffff7f>A</c>B", false), (0, 3));
    }

    #[test]
    fn measure_markup_pipe_breaks_lines() {
        let font = test_font();
        assert_eq!(font.measure("A|A", true), (5, 6));
        // Without markup '|' is a regular (unknown) character.
        assert_eq!(font.measure("A|A", false), (8, 3));
    }

    #[test]
    fn measure_skips_control_chars_entirely() {
        let font = test_font();
        assert_eq!(font.measure("A\tB", false), font.measure("AB", false));
    }

    #[test]
    fn measure_trailing_tag_keeps_h_space_quirk() {
        // 'A' is followed by tag text, so h_space applies (src/StdFont.cpp:630)
        // even though nothing visible follows: width 5 - 1 = 4.
        let font = test_font();
        assert_eq!(font.measure("A</c>", true), (4, 3));
    }

    #[test]
    fn measure_skip_mode_does_not_validate_hex() {
        // fSkip bypasses hex validation (src/StdMarkup.cpp:80): uppercase hex
        // is skipped by GetTextExtent even though DrawText renders it
        // literally.
        let font = test_font();
        assert_eq!(font.measure("<c FFFFFF>A", true), (5, 3));
        // Skip mode also matches any closing tag (src/StdMarkup.cpp:54).
        assert_eq!(font.measure("</foo>A", true), (5, 3));
    }

    // ---- draw (CStdFont::DrawText + CStdDDraw::TextOut) ----

    const WHITE: [u8; 4] = [255, 255, 255, 255];

    fn surface() -> Surface {
        Surface::new(16, 8, PixelFormat::Rgba8888)
    }

    fn px(surface: &Surface, x: u32, y: u32) -> Color {
        surface.get_pixel(x, y).expect("pixel in bounds")
    }

    #[test]
    fn draw_left_blits_glyph_cells() {
        let font = test_font();
        let mut sfc = surface();
        font.draw(&mut sfc, 0, 0, "AB", WHITE, TextAlign::Left, false);
        // 'A' covers x 0..5; pen advances 5 - 1 = 4; 'B' covers x 4..8.
        assert_eq!(px(&sfc, 0, 0), Color::new(255, 255, 255, 255));
        assert_eq!(px(&sfc, 7, 0), Color::new(255, 255, 255, 255));
        assert_eq!(px(&sfc, 8, 0).a, 0);
        assert_eq!(px(&sfc, 0, 3), Color::new(255, 255, 255, 255)); // cell row 3
        assert_eq!(px(&sfc, 0, 4).a, 0); // below cell_height
    }

    #[test]
    fn draw_center_subtracts_half_extent() {
        let font = test_font();
        let mut sfc = surface();
        // sx = 8 → x -= 8 / 2 → pen 4; glyphs cover x 4..12.
        font.draw(&mut sfc, 8, 0, "AB", WHITE, TextAlign::Center, false);
        assert_eq!(px(&sfc, 3, 0).a, 0);
        assert_eq!(px(&sfc, 4, 0), Color::new(255, 255, 255, 255));
        assert_eq!(px(&sfc, 11, 0), Color::new(255, 255, 255, 255));
        assert_eq!(px(&sfc, 12, 0).a, 0);

        // Odd width: sx = 5 → x -= 5 / 2 = 2 (integer division) → pen 6.
        let mut sfc = surface();
        font.draw(&mut sfc, 8, 0, "A", WHITE, TextAlign::Center, false);
        assert_eq!(px(&sfc, 5, 0).a, 0);
        assert_eq!(px(&sfc, 6, 0), Color::new(255, 255, 255, 255));
        assert_eq!(px(&sfc, 10, 0), Color::new(255, 255, 255, 255));
        assert_eq!(px(&sfc, 11, 0).a, 0);
    }

    #[test]
    fn draw_right_subtracts_extent() {
        let font = test_font();
        let mut sfc = surface();
        // sx = 8 → pen 12 - 8 = 4; glyphs cover x 4..12.
        font.draw(&mut sfc, 12, 0, "AB", WHITE, TextAlign::Right, false);
        assert_eq!(px(&sfc, 3, 0).a, 0);
        assert_eq!(px(&sfc, 4, 0), Color::new(255, 255, 255, 255));
        assert_eq!(px(&sfc, 11, 0), Color::new(255, 255, 255, 255));
        assert_eq!(px(&sfc, 12, 0).a, 0);
    }

    #[test]
    fn draw_markup_color_tag_modulates_glyph() {
        let font = test_font();
        let mut sfc = surface();
        font.draw(
            &mut sfc,
            0,
            0,
            "<c ffffff7f>A</c>B",
            WHITE,
            TextAlign::Left,
            true,
        );
        // Tag "ffffff7f" is AARRGGBB → rgb (255,255,127), modulated to
        // (254,254,126); the glyph stays opaque.
        assert_eq!(px(&sfc, 0, 0), Color::new(254, 254, 126, 255));
        assert_eq!(px(&sfc, 3, 0), Color::new(254, 254, 126, 255));
        // 'B' (pen 4, after </c>) reverts to the base color.
        assert_eq!(px(&sfc, 5, 0), Color::new(255, 255, 255, 255));
    }

    #[test]
    fn draw_markup_tag_equal_to_base_color_skips_modulation() {
        // dwBlitClr == dwColor → ModulateClrA is skipped (src/StdFont.cpp:914):
        // <c ffffff> over a white base renders 255, not 254.
        let font = test_font();
        let mut sfc = surface();
        font.draw(&mut sfc, 0, 0, "<c ffffff>A", WHITE, TextAlign::Left, true);
        assert_eq!(px(&sfc, 0, 0), Color::new(255, 255, 255, 255));

        // Same tag over a different base color does modulate.
        let mut sfc = surface();
        font.draw(
            &mut sfc,
            0,
            0,
            "<c ffffff>A",
            [0, 255, 0, 255],
            TextAlign::Left,
            true,
        );
        assert_eq!(px(&sfc, 0, 0), Color::new(254, 254, 254, 255));
    }

    #[test]
    fn draw_invalid_tag_renders_literally() {
        let font = test_font();
        let mut sfc = surface();
        // "<x>" is unknown (src/StdMarkup.cpp:99-100): '<', 'x', '>' render as
        // unknown glyphs advancing h_space each → 'A' starts at pen -3.
        font.draw(&mut sfc, 0, 0, "<x>A", WHITE, TextAlign::Left, true);
        assert_eq!(px(&sfc, 1, 0), Color::new(255, 255, 255, 255));
        assert_eq!(px(&sfc, 2, 0).a, 0);
    }

    #[test]
    fn draw_uppercase_hex_tag_renders_literally() {
        // Hex digits are lowercase-only (src/StdMarkup.cpp:87-89): the tag is
        // invalid in draw mode, renders as 10 unknown glyphs (pen -10) and
        // pushes 'A' fully off-surface.
        let font = test_font();
        let mut sfc = surface();
        font.draw(&mut sfc, 0, 0, "<c FFFFFF>A", WHITE, TextAlign::Left, true);
        assert_eq!(px(&sfc, 0, 0).a, 0);
    }

    #[test]
    fn draw_unknown_chars_advance_h_space_only() {
        let font = test_font();
        let mut sfc = surface();
        // '?' has no cell: width 0 + h_space → 'A' starts at pen -1.
        font.draw(&mut sfc, 0, 0, "?A", WHITE, TextAlign::Left, false);
        assert_eq!(px(&sfc, 0, 0), Color::new(255, 255, 255, 255));
        assert_eq!(px(&sfc, 3, 0), Color::new(255, 255, 255, 255));
        assert_eq!(px(&sfc, 4, 0).a, 0);
    }

    #[test]
    fn draw_modulates_alpha_by_color_alpha() {
        let font = test_font();
        let mut sfc = surface();
        // Half-transparent red: out = (255,0,0) with alpha round(255*128/255)
        // = 128; over transparent black: rgb 128/0/0, alpha round(128²/255) = 64.
        font.draw(&mut sfc, 0, 0, "A", [255, 0, 0, 128], TextAlign::Left, false);
        assert_eq!(px(&sfc, 0, 0), Color::new(128, 0, 0, 64));
    }

    #[test]
    fn draw_aligns_each_line_independently() {
        let font = test_font();
        let mut sfc = surface();
        // Line 1 "A": pen 8 - 2 = 6, rows 0..4. Line 2 "AB": pen 8 - 4 = 4,
        // rows 3..7 (y advances by line_height = 3, like TextOut).
        font.draw(&mut sfc, 8, 0, "A\nAB", WHITE, TextAlign::Center, false);
        assert_eq!(px(&sfc, 6, 0), Color::new(255, 255, 255, 255));
        assert_eq!(px(&sfc, 5, 0).a, 0);
        assert_eq!(px(&sfc, 4, 4), Color::new(255, 255, 255, 255));
        assert_eq!(px(&sfc, 3, 4).a, 0);
        assert_eq!(px(&sfc, 11, 4), Color::new(255, 255, 255, 255));
        assert_eq!(px(&sfc, 12, 4).a, 0);
    }

    #[test]
    fn draw_markup_pipe_splits_lines() {
        let font = test_font();
        let mut sfc = surface();
        // Markup mode splits on '|' (src/StdDDraw2.cpp:1039): 'B' lands on
        // line 2 (rows 3..7, cols 0..4).
        font.draw(&mut sfc, 0, 0, "A|B", WHITE, TextAlign::Left, true);
        assert_eq!(px(&sfc, 4, 1), Color::new(255, 255, 255, 255)); // A only
        assert_eq!(px(&sfc, 0, 6), Color::new(255, 255, 255, 255)); // B row
        assert_eq!(px(&sfc, 4, 6).a, 0); // beyond B's width

        // Without markup '|' is a regular char; everything stays on line 1.
        let mut sfc = surface();
        font.draw(&mut sfc, 0, 0, "A|B", WHITE, TextAlign::Left, false);
        assert_eq!(px(&sfc, 0, 6).a, 0);
    }

    #[test]
    fn draw_markup_color_persists_across_lines() {
        // One CMarkup per TextOut (src/StdDDraw2.cpp:1037): an unclosed color
        // tag keeps modulating on the next line.
        let font = test_font();
        let mut sfc = surface();
        font.draw(
            &mut sfc,
            0,
            0,
            "<c ffffff7f>A|A",
            WHITE,
            TextAlign::Left,
            true,
        );
        assert_eq!(px(&sfc, 0, 0), Color::new(254, 254, 126, 255));
        assert_eq!(px(&sfc, 0, 6), Color::new(254, 254, 126, 255));
    }

    #[test]
    fn draw_handles_clipping_and_bad_input_without_panicking() {
        let font = test_font();
        let mut sfc = surface();
        font.draw(&mut sfc, -3, -2, "AB", WHITE, TextAlign::Left, false);
        font.draw(&mut sfc, 100, 100, "AB", WHITE, TextAlign::Right, false);
        font.draw(&mut sfc, 0, 0, "<c", WHITE, TextAlign::Left, true);
        font.draw(&mut sfc, 0, 0, "</i>x<i>", WHITE, TextAlign::Center, true);
        let _ = font.measure("<", true);
        let _ = font.measure("\u{1F600}<c 12>|", true);
    }
}
