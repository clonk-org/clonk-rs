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
//! - FreeType rasterization: callers supply the 8-bit coverage bitmap.

use crate::{Color, GammaRamp, Rect, Surface, SurfaceDrawTarget};
use std::cell::Cell;
use std::collections::HashMap;
use std::marker::PhantomData;
use std::rc::Rc;

/// CStdGL device switches that affect the textured blits submitted by
/// `CStdFont::DrawText`. AllowedBlitModes is retained for an exact device
/// snapshot, although ordinary font blits request mode zero and masking it is
/// therefore a no-op.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ClonkTextBlitConfig {
    pub no_alpha_add: bool,
    pub tex_indent: i32,
    pub blit_offset: i32,
    pub allowed_blit_modes: u32,
    pub shader: bool,
}

impl ClonkTextBlitConfig {
    fn texture_indent(self) -> f32 {
        self.tex_indent as f32 / 1000.0
    }

    fn destination_offset(self) -> f32 {
        self.blit_offset as f32 / 100.0
    }

    fn changes_geometry(self) -> bool {
        self.tex_indent != 0 || self.blit_offset != 0
    }

    fn disables_fixed_alpha_add(self) -> bool {
        !self.shader && self.no_alpha_add
    }
}

thread_local! {
    static ACTIVE_CLONK_TEXT_BLIT_CONFIG: Cell<Option<ClonkTextBlitConfig>> =
        const { Cell::new(None) };
}

/// Nest-safe activation guard for CStdFont's low-level textured glyph blits.
#[must_use = "the text blit configuration remains active only while the guard is alive"]
pub struct ClonkTextBlitConfigGuard {
    previous: Option<ClonkTextBlitConfig>,
    _not_send_or_sync: PhantomData<Rc<()>>,
}

impl Drop for ClonkTextBlitConfigGuard {
    fn drop(&mut self) {
        ACTIVE_CLONK_TEXT_BLIT_CONFIG.with(|active| active.set(self.previous));
    }
}

pub fn activate_clonk_text_blit_config(config: ClonkTextBlitConfig) -> ClonkTextBlitConfigGuard {
    let previous = ACTIVE_CLONK_TEXT_BLIT_CONFIG.with(|active| active.replace(Some(config)));
    ClonkTextBlitConfigGuard {
        previous,
        _not_send_or_sync: PhantomData,
    }
}

fn active_clonk_text_blit_config() -> Option<ClonkTextBlitConfig> {
    ACTIVE_CLONK_TEXT_BLIT_CONFIG.with(Cell::get)
}

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
                    (x < cov_w && y < cov_h).then(|| g(x, y)),     // :239
                    (x > 1 && y < cov_h).then(|| g(x - 2, y)),     // :240
                    (x > 0 && y < cov_h).then(|| g(x - 1, y)),     // :241
                    (x < cov_w && y > 1).then(|| g(x, y - 2)),     // :242
                    (x > 1 && y > 1).then(|| g(x - 2, y - 2)),     // :243
                    (x > 0 && y > 1).then(|| g(x - 1, y - 2)),     // :244
                    (x < cov_w && y > 0).then(|| g(x, y - 1)),     // :245
                    (x > 1 && y > 0).then(|| g(x - 2, y - 1)),     // :246
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

/// Borrowed RGBA image returned by a [`FontImageProvider`].
#[derive(Debug, Clone, Copy)]
pub struct FontImageRef<'a> {
    pub width: u32,
    pub height: u32,
    pub rgba: &'a [u8],
}

/// Dynamic source for `{{image spec}}` runs in `CStdFont` text.
///
/// The provider is deliberately supplied per operation rather than stored in
/// [`ClonkFont`]: C++ points FontRegular at the live definition list, while
/// Rust shares immutable font atlases across scenarios.
pub trait FontImageProvider {
    fn font_image(&self, tag: &str) -> Option<FontImageRef<'_>>;
}

/// Parse one C++ `{{image spec}}` token at the beginning of `text`.
///
/// The third opening brace suppresses recognition for this position, so
/// `{{{ID}}` is a literal `{` followed by the `{{ID}}` token on the next
/// parser step. Empty, malformed and unclosed tokens are ordinary text.
pub fn inline_image_token(text: &str) -> Option<(&str, usize)> {
    let after_open = text.strip_prefix("{{")?;
    if after_open.is_empty() || after_open.starts_with('{') {
        return None;
    }
    let close = after_open.find('}')?;
    if close == 0 || after_open.as_bytes().get(close + 1) != Some(&b'}') {
        return None;
    }
    Some((&after_open[..close], 2 + close + 2))
}

/// C++ copies at most 100 bytes of an inline image spec into its lookup
/// buffer while consuming the complete source token. Keep Rust strings valid
/// by backing up to the preceding UTF-8 boundary for non-legacy input.
pub fn font_image_lookup_tag(tag: &str) -> &str {
    let mut end = tag.len().min(100);
    while !tag.is_char_boundary(end) {
        end -= 1;
    }
    &tag[..end]
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

/// Semantic identity of a classic font face used by native-text replay.
///
/// GUI and book roles remain distinct even when they share a nominal point
/// size: C++ initializes the book faces without the ordinary GUI shadow.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ClonkFontRole {
    GuiTitle,
    GuiCaption,
    GuiText,
    GuiMainSmall,
    GuiMini,
    GuiTooltip,
    BookTitle,
    BookCaption,
    BookText,
    BookSmall,
}

/// One semantic CStdFont draw captured before logical glyph rasterization.
///
/// The command owns its text and gamma ramp so it may outlive the renderer's
/// borrowed arguments and be replayed later against a scale-native atlas.
#[derive(Debug, Clone, PartialEq)]
pub struct CapturedClonkText {
    pub role: ClonkFontRole,
    pub x: i32,
    pub y: i32,
    pub text: String,
    pub color: [u8; 4],
    pub align: TextAlign,
    pub markup: bool,
    pub clip: Option<Rect>,
    pub gamma: Option<GammaRamp>,
    pub images: Vec<CapturedFontImage>,
}

/// Owned inline image referenced by a captured `{{TextSpec}}` run.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CapturedFontImage {
    pub tag: String,
    pub width: u32,
    pub height: u32,
    pub rgba: Vec<u8>,
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
    /// FreeType glyph index zero (`.notdef`), used by UTF-8 vector fonts when
    /// the active charmap has no entry for a decoded scalar. C++ obtains this
    /// through `FT_Load_Char` in `GetUnicodeCharacterFacet`.
    missing_glyph: Option<GlyphCell>,
    /// Physical C4Surface texture dimension used by TexIndent's texture
    /// matrix. Vector fonts use 128px atlases through height 40 and 512px
    /// atlases above it (StdFont.cpp:331-337).
    texture_size: i32,
    /// Replay identity for engine-wide scale-native text capture. Untagged
    /// fonts retain the historical immediate-raster behavior.
    role: Option<ClonkFontRole>,
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
            missing_glyph: None,
            texture_size: 128,
            role: None,
        }
    }

    /// Override the physical font-atlas texture dimension selected by C++.
    /// Call this before drawing; it does not affect legacy zero-indent output.
    pub fn set_texture_size(&mut self, texture_size: u32) {
        self.texture_size = i32::try_from(texture_size.max(1)).unwrap_or(i32::MAX);
    }

    /// Physical C4Surface tile size backing this font's glyph facets.
    pub fn texture_size(&self) -> i32 {
        self.texture_size
    }

    /// The semantic replay role, if this font participates in capture.
    pub fn role(&self) -> Option<ClonkFontRole> {
        self.role
    }

    /// Assign or clear this font's semantic replay role.
    pub fn set_role(&mut self, role: Option<ClonkFontRole>) {
        self.role = role;
    }

    /// Builder-style counterpart of [`Self::set_role`].
    pub fn with_role(mut self, role: ClonkFontRole) -> Self {
        self.role = Some(role);
        self
    }

    /// Register the glyph cell for `ch` (mirrors the per-character facets
    /// stored by `CStdFont::AddRenderedChar`, `src/StdFont.cpp:260`).
    pub fn add_glyph(&mut self, ch: char, cell: GlyphCell) {
        self.cells.insert(ch, cell);
    }

    /// Look up the directly rendered glyph cell for `ch`.
    ///
    /// This deliberately does not return the missing-glyph fallback, so
    /// callers that validate an atlas can still distinguish real charmap
    /// coverage from FreeType's glyph-zero rendering.
    pub fn glyph(&self, ch: char) -> Option<&GlyphCell> {
        self.cells.get(&ch)
    }

    /// Install FreeType glyph index zero as the fallback for decoded Unicode
    /// scalars absent from the font's charmap.
    pub fn set_missing_glyph(&mut self, cell: GlyphCell) {
        self.missing_glyph = Some(cell);
    }

    /// Resolve the cell DrawText uses, including the installed FreeType
    /// glyph-zero fallback for a character absent from the direct map.
    pub fn rendered_glyph(&self, ch: char) -> Option<&GlyphCell> {
        self.cells.get(&ch).or(self.missing_glyph.as_ref())
    }

    /// One `CStdFont::BreakMessage` character advance at scale 1. The same
    /// rendered facet lookup is used by measurement and drawing.
    pub fn message_character_advance(&self, character: char) -> i32 {
        if character < ' ' {
            return 0;
        }
        self.rendered_glyph(character)
            .map_or(0, |glyph| glyph.width)
            .saturating_add(self.h_space)
    }

    /// Measure `text`, mirroring `CStdFont::GetTextExtent`
    /// (`src/StdFont.cpp:571-638`) with `scale = iFontZoom = 1`.
    ///
    /// Returns `(width, height)`. The width is the maximum over all rows of
    /// the per-character widths plus `h_space` after every character that has
    /// *any* remaining text in the whole string — including before `'\n'`
    /// (`if (*szText) iRowWdt += iHSpace`, `src/StdFont.cpp:630`). The height
    /// is `line_height` per row (`src/StdFont.cpp:583,596`). Characters
    /// without a direct glyph use the installed FreeType glyph-zero fallback;
    /// without one they contribute width 0 (empty facet). With `markup`,
    /// valid tags are skipped (`src/StdFont.cpp:590`) and `'|'` breaks lines
    /// (`src/StdFont.cpp:596`).
    pub fn measure(&self, text: &str, markup: bool) -> (i32, i32) {
        self.measure_impl(text, markup, None)
    }

    /// [`Self::measure`] with FontRegular's live custom-image provider.
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
        let mut rest = text;
        let mut row_width: i32 = 0;
        let mut width: i32 = 0;
        let mut height = self.line_height; // src/StdFont.cpp:583
        loop {
            if markup {
                rest = skip_tags(rest); // src/StdFont.cpp:590
            }
            if markup {
                if let Some((tag, advance)) = inline_image_token(rest) {
                    let image_width = images
                        .and_then(|provider| provider.font_image(font_image_lookup_tag(tag)))
                        .map_or(0, |image| scaled_font_image_width(self.cell_height, image));
                    row_width = row_width.saturating_add(image_width);
                    rest = &rest[advance..];
                    // GetTextExtent applies iHSpace after every recognized
                    // token with raw text remaining, even an unresolved image
                    // (StdFont.cpp:625-630).
                    if !rest.is_empty() {
                        row_width = row_width.saturating_add(self.h_space);
                    }
                    width = width.max(row_width);
                    continue;
                }
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
            // Character facet width (src/StdFont.cpp:627); a UTF-8 vector font
            // resolves an unmapped scalar through FreeType glyph index zero.
            row_width = row_width.saturating_add(self.rendered_glyph(c).map_or(0, |g| g.width));
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
    /// alpha uses `color`'s inverted-alpha blit addition only (tag alpha is
    /// ignored).
    /// `</c>` reverts. Each open `<i>` contributes the native centered `-0.3`
    /// horizontal shear. Invalid or unknown tags render literally
    /// (`src/StdFont.cpp:864-866`).
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

    /// [`Self::draw`] with FontRegular's live custom-image provider.
    #[allow(clippy::too_many_arguments)]
    pub fn draw_with_images(
        &self,
        surface: &mut Surface,
        x: i32,
        y: i32,
        text: &str,
        color: [u8; 4],
        align: TextAlign,
        markup: bool,
        images: &dyn FontImageProvider,
    ) {
        self.draw_with_gamma_and_images(surface, x, y, text, color, align, markup, None, images);
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
        self.draw_with_gamma_impl(surface, x, y, text, color, align, markup, gamma, None);
    }

    /// Target-generic counterpart of [`Self::draw_with_gamma`] used by
    /// zero-copy native framebuffer views.
    #[allow(clippy::too_many_arguments)]
    pub fn draw_with_gamma_to<T: SurfaceDrawTarget + ?Sized>(
        &self,
        surface: &mut T,
        x: i32,
        y: i32,
        text: &str,
        color: [u8; 4],
        align: TextAlign,
        markup: bool,
        gamma: Option<&crate::GammaRamp>,
    ) {
        self.draw_with_gamma_impl(surface, x, y, text, color, align, markup, gamma, None);
    }

    /// [`Self::draw_with_gamma`] with FontRegular's custom images.
    #[allow(clippy::too_many_arguments)]
    pub fn draw_with_gamma_and_images(
        &self,
        surface: &mut Surface,
        x: i32,
        y: i32,
        text: &str,
        color: [u8; 4],
        align: TextAlign,
        markup: bool,
        gamma: Option<&crate::GammaRamp>,
        images: &dyn FontImageProvider,
    ) {
        self.draw_with_gamma_impl(
            surface,
            x,
            y,
            text,
            color,
            align,
            markup,
            gamma,
            Some(images),
        );
    }

    /// Target-generic counterpart of [`Self::draw_with_gamma_and_images`].
    #[allow(clippy::too_many_arguments)]
    pub fn draw_with_gamma_and_images_to<T: SurfaceDrawTarget + ?Sized>(
        &self,
        surface: &mut T,
        x: i32,
        y: i32,
        text: &str,
        color: [u8; 4],
        align: TextAlign,
        markup: bool,
        gamma: Option<&crate::GammaRamp>,
        images: &dyn FontImageProvider,
    ) {
        self.draw_with_gamma_impl(
            surface,
            x,
            y,
            text,
            color,
            align,
            markup,
            gamma,
            Some(images),
        );
    }

    #[allow(clippy::too_many_arguments)]
    fn draw_with_gamma_impl<T: SurfaceDrawTarget + ?Sized>(
        &self,
        surface: &mut T,
        x: i32,
        y: i32,
        text: &str,
        color: [u8; 4],
        align: TextAlign,
        markup: bool,
        gamma: Option<&crate::GammaRamp>,
        images: Option<&dyn FontImageProvider>,
    ) {
        if let Some(role) = self.role {
            let mut captured_images = Vec::new();
            if let Some(provider) = images {
                let mut rest = text;
                while let Some(character) = rest.chars().next() {
                    if let Some((tag, advance)) = inline_image_token(rest) {
                        let lookup = font_image_lookup_tag(tag);
                        if !captured_images
                            .iter()
                            .any(|image: &CapturedFontImage| image.tag == lookup)
                        {
                            if let Some(image) = provider.font_image(lookup) {
                                captured_images.push(CapturedFontImage {
                                    tag: lookup.to_owned(),
                                    width: image.width,
                                    height: image.height,
                                    rgba: image.rgba.to_vec(),
                                });
                            }
                        }
                        rest = &rest[advance..];
                    } else {
                        rest = &rest[character.len_utf8()..];
                    }
                }
            }
            let command = CapturedClonkText {
                role,
                x,
                y,
                text: text.to_owned(),
                color,
                align,
                markup,
                clip: surface.clip(),
                gamma: gamma.cloned(),
                images: captured_images,
            };
            if surface.capture_clonk_text(command) {
                return;
            }
        }

        let mut stack: Vec<MarkupTag> = Vec::new(); // src/StdDDraw2.cpp:1037
        let mut line_y = y;
        for line in text.split(|c: char| c == '\n' || (markup && c == '|')) {
            self.draw_line(
                surface, x, line_y, line, color, align, markup, &mut stack, gamma, images,
            );
            // iTy += fZoom * GetLineHeight() per line (src/StdDDraw2.cpp:1039).
            line_y = line_y.saturating_add(self.line_height);
        }
    }

    /// Draw each logical line at a caller-supplied physical origin while
    /// retaining one markup stack across the complete text. Scale-native
    /// CStdFont rendering needs this because C++ advances line positions in
    /// GUI units before the graphics transform, which can differ from the
    /// raster atlas's physical `line_height` (`StdDDraw2.cpp:1035-1042`).
    #[allow(clippy::too_many_arguments)]
    pub fn draw_lines_at_origins_with_gamma(
        &self,
        surface: &mut Surface,
        origins: &[(i32, i32)],
        text: &str,
        color: [u8; 4],
        markup: bool,
        gamma: Option<&crate::GammaRamp>,
    ) {
        self.draw_lines_at_origins_with_gamma_impl(
            surface, origins, text, color, markup, gamma, None,
        );
    }

    /// Target-generic counterpart of
    /// [`Self::draw_lines_at_origins_with_gamma`].
    #[allow(clippy::too_many_arguments)]
    pub fn draw_lines_at_origins_with_gamma_to<T: SurfaceDrawTarget + ?Sized>(
        &self,
        surface: &mut T,
        origins: &[(i32, i32)],
        text: &str,
        color: [u8; 4],
        markup: bool,
        gamma: Option<&crate::GammaRamp>,
    ) {
        self.draw_lines_at_origins_with_gamma_impl(
            surface, origins, text, color, markup, gamma, None,
        );
    }

    /// [`Self::draw_lines_at_origins_with_gamma`] with custom images.
    #[allow(clippy::too_many_arguments)]
    pub fn draw_lines_at_origins_with_gamma_and_images(
        &self,
        surface: &mut Surface,
        origins: &[(i32, i32)],
        text: &str,
        color: [u8; 4],
        markup: bool,
        gamma: Option<&crate::GammaRamp>,
        images: &dyn FontImageProvider,
    ) {
        self.draw_lines_at_origins_with_gamma_impl(
            surface,
            origins,
            text,
            color,
            markup,
            gamma,
            Some(images),
        );
    }

    /// Target-generic counterpart of
    /// [`Self::draw_lines_at_origins_with_gamma_and_images`].
    #[allow(clippy::too_many_arguments)]
    pub fn draw_lines_at_origins_with_gamma_and_images_to<T: SurfaceDrawTarget + ?Sized>(
        &self,
        surface: &mut T,
        origins: &[(i32, i32)],
        text: &str,
        color: [u8; 4],
        markup: bool,
        gamma: Option<&crate::GammaRamp>,
        images: &dyn FontImageProvider,
    ) {
        self.draw_lines_at_origins_with_gamma_impl(
            surface,
            origins,
            text,
            color,
            markup,
            gamma,
            Some(images),
        );
    }

    #[allow(clippy::too_many_arguments)]
    fn draw_lines_at_origins_with_gamma_impl<T: SurfaceDrawTarget + ?Sized>(
        &self,
        surface: &mut T,
        origins: &[(i32, i32)],
        text: &str,
        color: [u8; 4],
        markup: bool,
        gamma: Option<&crate::GammaRamp>,
        images: Option<&dyn FontImageProvider>,
    ) {
        let mut stack: Vec<MarkupTag> = Vec::new();
        for ((x, y), line) in origins
            .iter()
            .copied()
            .zip(text.split(|character: char| character == '\n' || (markup && character == '|')))
        {
            self.draw_line(
                surface,
                x,
                y,
                line,
                color,
                TextAlign::Left,
                markup,
                &mut stack,
                gamma,
                images,
            );
        }
    }

    /// One `CStdFont::DrawText` call (`src/StdFont.cpp:814-934`).
    #[allow(clippy::too_many_arguments)]
    fn draw_line<T: SurfaceDrawTarget + ?Sized>(
        &self,
        surface: &mut T,
        x: i32,
        y: i32,
        line: &str,
        color: [u8; 4],
        align: TextAlign,
        markup: bool,
        stack: &mut Vec<MarkupTag>,
        gamma: Option<&crate::GammaRamp>,
        images: Option<&dyn FontImageProvider>,
    ) {
        // Alignment uses the markup-aware extent of this line
        // (src/StdFont.cpp:826-839); sx / 2 is integer division.
        let (sx, _) = self.measure_impl(line, markup, images);
        let mut pen_x = x - match align {
            TextAlign::Left => 0,
            TextAlign::Center => sx / 2, // src/StdFont.cpp:831
            TextAlign::Right => sx,      // src/StdFont.cpp:838
        };
        let mut rest = line;
        // DrawText keeps its transform pointer once markup was already open or
        // any valid tag was read, even if a later close empties the stack.
        let mut transform_active = !stack.is_empty();
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
                    transform_active = true;
                    rest = &rest[advance..];
                    continue;
                }
                // Invalid tag: fall through and render '<' as text.
            }
            if markup && c == '{' {
                if let Some((tag, advance)) = inline_image_token(rest) {
                    rest = &rest[advance..];
                    let Some(image) =
                        images.and_then(|provider| provider.font_image(font_image_lookup_tag(tag)))
                    else {
                        // DrawText consumes unresolved tags without blitting
                        // or advancing the pen (StdFont.cpp:884-892).
                        continue;
                    };
                    if image.height == 0 {
                        // A zero-height facet is the C++ provider's
                        // unresolved sentinel and neither blits nor advances.
                        continue;
                    }
                    let image_width = scaled_font_image_width(self.cell_height, image);
                    blit_font_image(
                        surface,
                        image,
                        image_width,
                        self.cell_height,
                        pen_x,
                        y,
                        image_modulation_rgb(stack, color, transform_active),
                        color[3],
                        gamma,
                        markup_shear(stack),
                        transform_active,
                    );
                    pen_x = pen_x
                        .saturating_add(image_width)
                        .saturating_add(self.h_space);
                    continue;
                }
            }
            rest = after;
            let cell = self.rendered_glyph(c);
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
                    markup_shear(stack),
                    transform_active,
                    self.texture_size,
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
    /// `<i>` — contributes one native `-0.3` horizontal shear.
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

/// `CMarkup::Apply` visits every open tag and each italic tag subtracts 0.3
/// from `CBltTransform::mat[1]` (`src/StdMarkup.cpp:24-28`).
fn markup_shear(stack: &[MarkupTag]) -> f32 {
    stack
        .iter()
        .filter(|tag| matches!(tag, MarkupTag::Italic))
        .fold(0.0_f32, |shear, _| shear - 0.3)
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

/// Consume one tag exactly as `CMarkup::Read(fSkip = true)` would.
///
/// The returned byte count is the native pointer advance (normally through
/// `<` and `>`; an overlong tag follows C++'s 49-byte truncation). Unknown
/// opening tags remain visible, while any parameterless closing tag and color
/// parameters of at most eight raw bytes are accepted in skip mode.
pub fn skip_markup_tag(text: &str) -> Option<usize> {
    read_tag(text, None)
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
            (None, _) => false,                   // :75
            (Some(p), _) if p.len() > 8 => false, // :76-79
            (Some(_), None) => true,              // skip mode: hex unchecked (:80)
            (Some(p), Some(stack)) => parse_color_tag(p)
                .map(|clr| stack.push(MarkupTag::TextColor(clr)))
                .is_some(),
        }
    } else {
        false // unknown tag (src/StdMarkup.cpp:99-100)
    };
    valid.then_some(advance)
}

/// Return the closing and reopening markup for the tags left active after
/// parsing `text` with `CMarkup::Read(..., false)`.
///
/// `CStdFont::BreakMessage` uses these fragments around each automatically
/// inserted newline so either physical line can be drawn independently.
pub fn active_markup_fragments(text: &str) -> (String, String) {
    let mut stack = Vec::new();
    let mut rest = text;
    while !rest.is_empty() {
        if rest.starts_with('<') {
            if let Some(advance) = read_tag(rest, Some(&mut stack)) {
                rest = &rest[advance..];
                continue;
            }
        }
        let character = rest.chars().next().expect("non-empty markup text");
        rest = &rest[character.len_utf8()..];
    }

    let mut closing = String::new();
    for tag in stack.iter().rev() {
        closing.push_str("</");
        closing.push_str(tag.name());
        closing.push('>');
    }
    let mut reopening = String::new();
    for tag in stack {
        match tag {
            MarkupTag::Italic => reopening.push_str("<i>"),
            MarkupTag::TextColor(color) => reopening.push_str(&format!("<c {color:x}>")),
        }
    }
    (closing, reopening)
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

/// Aspect-scaled width of an inline font image at `iGfxLineHgt`.
pub fn scaled_font_image_width(cell_height: i32, image: FontImageRef<'_>) -> i32 {
    if cell_height <= 0 || image.height == 0 {
        return 0;
    }
    (i64::from(image.width) * i64::from(cell_height) / i64::from(image.height))
        .try_into()
        .unwrap_or(i32::MAX)
}

fn image_modulation_rgb(stack: &[MarkupTag], color: [u8; 4], transform_active: bool) -> [u8; 3] {
    if stack
        .iter()
        .any(|tag| matches!(tag, MarkupTag::TextColor(_)))
    {
        modulation_rgb(stack, color)
    } else if transform_active {
        // Once DrawText has a transform pointer, its shared markup branch
        // restarts custom-image modulation from the base text color too.
        [color[0], color[1], color[2]]
    } else {
        // CStdFont disables ordinary text-color modulation for custom images;
        // only active markup or alpha fadeout affects them (StdFont.cpp:893-915).
        [255, 255, 255]
    }
}

#[allow(clippy::too_many_arguments)]
fn blit_font_image<T: SurfaceDrawTarget + ?Sized>(
    surface: &mut T,
    image: FontImageRef<'_>,
    width: i32,
    height: i32,
    x: i32,
    y: i32,
    mod_rgb: [u8; 3],
    color_alpha: u8,
    gamma: Option<&crate::GammaRamp>,
    shear: f32,
    transform_active: bool,
) {
    if width <= 0 || height <= 0 || image.width == 0 || image.height == 0 {
        return;
    }
    if let Some(config) = active_clonk_text_blit_config().filter(|config| config.changes_geometry())
    {
        blit_font_image_configured(
            surface,
            image,
            width,
            height,
            x,
            y,
            mod_rgb,
            color_alpha,
            gamma,
            shear,
            transform_active,
            config,
        );
        return;
    }
    if shear != 0.0 {
        let Some((x0, y0, x1, y1)) = sheared_raster_bounds(
            surface,
            x as f32,
            y as f32,
            width as f32,
            height as f32,
            shear,
        ) else {
            return;
        };
        for target_y in y0..y1 {
            for target_x in x0..x1 {
                let Some((sample_x, sample_y)) = inverse_sheared_sample(
                    target_x,
                    target_y,
                    x as f32,
                    y as f32,
                    width as f32,
                    height as f32,
                    image.width as f32,
                    image.height as f32,
                    shear,
                ) else {
                    continue;
                };
                blend_font_sample(
                    surface,
                    target_x as u32,
                    target_y as u32,
                    bilinear_font_image_sample(image, sample_x, sample_y),
                    mod_rgb,
                    color_alpha,
                    gamma,
                );
            }
        }
        return;
    }
    for row in 0..height as usize {
        for col in 0..width as usize {
            let sample_x = (col as f32 + 0.5) * image.width as f32 / width as f32 - 0.5;
            let sample_y = (row as f32 + 0.5) * image.height as f32 / height as f32 - 0.5;
            let (Some(dx), Some(dy)) = (offset_coord(x, col), offset_coord(y, row)) else {
                continue;
            };
            blend_font_sample(
                surface,
                dx,
                dy,
                bilinear_font_image_sample(image, sample_x, sample_y),
                mod_rgb,
                color_alpha,
                gamma,
            );
        }
    }
}

/// Configured CStdGL font-image blit. Font images remain isolated source
/// surfaces in Rust, so GL_CLAMP_TO_EDGE is reproduced at that source-facet
/// boundary while the texture matrix retains C++'s physical texture size.
#[allow(clippy::too_many_arguments)]
fn blit_font_image_configured<T: SurfaceDrawTarget + ?Sized>(
    surface: &mut T,
    image: FontImageRef<'_>,
    width: i32,
    height: i32,
    x: i32,
    y: i32,
    mod_rgb: [u8; 3],
    color_alpha: u8,
    gamma: Option<&crate::GammaRamp>,
    shear: f32,
    _transform_active: bool,
    config: ClonkTextBlitConfig,
) {
    let texture_size = font_image_texture_size(image);
    let Some(mapping) = ConfiguredFontBlit::new(
        x as f32,
        y as f32,
        width as f32,
        height as f32,
        image.width as f32,
        image.height as f32,
        texture_size,
        shear,
        config,
    ) else {
        return;
    };
    let Some((x0, y0, x1, y1)) = mapping.raster_bounds(surface) else {
        return;
    };
    for target_y in y0..y1 {
        for target_x in x0..x1 {
            let Some((sample_x, sample_y)) = mapping.bilinear_sample(target_x, target_y) else {
                continue;
            };
            blend_font_sample(
                surface,
                target_x as u32,
                target_y as u32,
                bilinear_font_image_sample(image, sample_x, sample_y),
                mod_rgb,
                color_alpha,
                gamma,
            );
        }
    }
}

#[allow(clippy::too_many_arguments)]
fn blend_font_sample<T: SurfaceDrawTarget + ?Sized>(
    surface: &mut T,
    x: u32,
    y: u32,
    sample: [f32; 4],
    mod_rgb: [u8; 3],
    color_alpha: u8,
    gamma: Option<&crate::GammaRamp>,
) {
    let out_a = font_sample_alpha(sample[3], color_alpha);
    if out_a <= 0.0 {
        return;
    }
    // Preserve the native inverted-alpha subtraction above while keeping the
    // unblended float fragment available to retained GPU targets. The default
    // SurfaceDrawTarget implementation performs the same gamma lookup and
    // source-alpha composition for CPU targets.
    let _ = surface.blend_fragment(
        x,
        y,
        [
            sample[0] * f32::from(mod_rgb[0]) / 255.0,
            sample[1] * f32::from(mod_rgb[1]) / 255.0,
            sample[2] * f32::from(mod_rgb[2]) / 255.0,
            out_a,
        ],
        gamma,
    );
}

fn font_sample_alpha(sample_alpha: f32, color_alpha: u8) -> f32 {
    if active_clonk_text_blit_config().is_some_and(ClonkTextBlitConfig::disables_fixed_alpha_add) {
        // Fixed-function NoAlphaAdd switches the texture environment from
        // GL_COMBINE/GL_ADD to GL_MODULATE with an opaque primary alpha.
        return sample_alpha.clamp(0.0, 255.0);
    }
    // The font texture's inverted alpha and the primary modulation's
    // inverted alpha are added in the C++ shader. In normal-alpha form this
    // subtracts the modulation transparency instead of multiplying opacity.
    (sample_alpha - f32::from(255 - color_alpha)).max(0.0)
}

fn font_image_texture_size(image: FontImageRef<'_>) -> i32 {
    // C4Surface::CreateTextures starts at the smaller surface dimension and
    // deliberately chooses 2 even for a one-pixel source.
    let required = image.width.min(image.height).max(1);
    required
        .max(2)
        .checked_next_power_of_two()
        .unwrap_or(4096)
        .min(4096) as i32
}

/// Destination and texture-matrix state shared by configured glyph/image
/// blits. CStdGL adds BlitOffset to vertices after CStdFont has centered its
/// markup transform, so `pivot_y` deliberately remains unshifted.
#[derive(Clone, Copy)]
struct ConfiguredFontBlit {
    x: f32,
    y: f32,
    width: f32,
    height: f32,
    source_width: f32,
    source_height: f32,
    texture_scale: f32,
    texture_indent: f32,
    shear: f32,
    pivot_y: f32,
}

impl ConfiguredFontBlit {
    #[allow(clippy::too_many_arguments)]
    fn new(
        x: f32,
        y: f32,
        width: f32,
        height: f32,
        source_width: f32,
        source_height: f32,
        texture_size: i32,
        shear: f32,
        config: ClonkTextBlitConfig,
    ) -> Option<Self> {
        let texture_size = texture_size as f32;
        let texture_indent = config.texture_indent();
        let texture_denominator = texture_size + texture_indent * 2.0;
        if width <= 0.0
            || height <= 0.0
            || source_width <= 0.0
            || source_height <= 0.0
            || texture_size <= 0.0
            || !x.is_finite()
            || !y.is_finite()
            || !width.is_finite()
            || !height.is_finite()
            || !source_width.is_finite()
            || !source_height.is_finite()
            || !texture_denominator.is_finite()
            || texture_denominator == 0.0
            || !shear.is_finite()
        {
            return None;
        }
        let destination_offset = config.destination_offset();
        Some(Self {
            x: x + destination_offset,
            y: y + destination_offset,
            width,
            height,
            source_width,
            source_height,
            texture_scale: texture_size / texture_denominator,
            texture_indent,
            shear,
            pivot_y: y + height / 2.0,
        })
    }

    fn raster_bounds<T: SurfaceDrawTarget + ?Sized>(
        self,
        surface: &T,
    ) -> Option<(i32, i32, i32, i32)> {
        transformed_raster_bounds(
            surface,
            self.x,
            self.y,
            self.width,
            self.height,
            self.shear,
            self.pivot_y,
        )
    }

    fn source_edge(self, target_x: i32, target_y: i32) -> Option<(f32, f32)> {
        let pixel_x = target_x as f32 + 0.5;
        let pixel_y = target_y as f32 + 0.5;
        let unsheared_x = pixel_x - self.shear * (pixel_y - self.pivot_y);
        let local_x = unsheared_x - self.x;
        let local_y = pixel_y - self.y;
        if local_x < 0.0 || local_y < 0.0 || local_x >= self.width || local_y >= self.height {
            return None;
        }
        Some((
            self.texture_indent + local_x * self.source_width / self.width * self.texture_scale,
            self.texture_indent + local_y * self.source_height / self.height * self.texture_scale,
        ))
    }

    fn bilinear_sample(self, target_x: i32, target_y: i32) -> Option<(f32, f32)> {
        self.source_edge(target_x, target_y)
            .map(|(x, y)| (x - 0.5, y - 0.5))
    }

    fn nearest_sample(self, target_x: i32, target_y: i32) -> Option<(i32, i32)> {
        self.source_edge(target_x, target_y)
            .map(|(x, y)| (x.floor() as i32, y.floor() as i32))
    }
}

fn bilinear_font_image_sample(image: FontImageRef<'_>, sample_x: f32, sample_y: f32) -> [f32; 4] {
    let texel = |x: i32, y: i32| {
        if image.width == 0 || image.height == 0 {
            return [0.0; 4];
        }
        // GL_CLAMP_TO_EDGE clamps the bilinear footprint at a facet's outer
        // edge rather than mixing it with transparent pixels.
        let x = x.clamp(0, image.width as i32 - 1) as u32;
        let y = y.clamp(0, image.height as i32 - 1) as u32;
        let index = ((y * image.width + x) * 4) as usize;
        image
            .rgba
            .get(index..index + 4)
            .map(|pixel| {
                [
                    f32::from(pixel[0]),
                    f32::from(pixel[1]),
                    f32::from(pixel[2]),
                    f32::from(pixel[3]),
                ]
            })
            .unwrap_or([0.0; 4])
    };
    let x0 = sample_x.floor() as i32;
    let y0 = sample_y.floor() as i32;
    let fraction_x = sample_x - x0 as f32;
    let fraction_y = sample_y - y0 as f32;
    let top_left = texel(x0, y0);
    let top_right = texel(x0 + 1, y0);
    let bottom_left = texel(x0, y0 + 1);
    let bottom_right = texel(x0 + 1, y0 + 1);
    std::array::from_fn(|channel| {
        let top = top_left[channel] * (1.0 - fraction_x) + top_right[channel] * fraction_x;
        let bottom = bottom_left[channel] * (1.0 - fraction_x) + bottom_right[channel] * fraction_x;
        top * (1.0 - fraction_y) + bottom * fraction_y
    })
}

fn bilinear_glyph_sample(
    cell: &GlyphCell,
    cell_height: i32,
    sample_x: f32,
    sample_y: f32,
) -> [f32; 4] {
    let texel = |x: i32, y: i32| {
        if x < 0 || y < 0 || x >= cell.width || y >= cell_height {
            return [0.0; 4];
        }
        cell.pixels
            .get(y as usize * cell.width as usize + x as usize)
            .map(|pixel| {
                [
                    f32::from(pixel.r),
                    f32::from(pixel.g),
                    f32::from(pixel.b),
                    f32::from(pixel.a),
                ]
            })
            .unwrap_or([0.0; 4])
    };
    let x0 = sample_x.floor() as i32;
    let y0 = sample_y.floor() as i32;
    let fraction_x = sample_x - x0 as f32;
    let fraction_y = sample_y - y0 as f32;
    let top_left = texel(x0, y0);
    let top_right = texel(x0 + 1, y0);
    let bottom_left = texel(x0, y0 + 1);
    let bottom_right = texel(x0 + 1, y0 + 1);
    std::array::from_fn(|channel| {
        let top = top_left[channel] * (1.0 - fraction_x) + top_right[channel] * fraction_x;
        let bottom = bottom_left[channel] * (1.0 - fraction_x) + bottom_right[channel] * fraction_x;
        top * (1.0 - fraction_y) + bottom * fraction_y
    })
}

fn sheared_raster_bounds<T: SurfaceDrawTarget + ?Sized>(
    surface: &T,
    x: f32,
    y: f32,
    width: f32,
    height: f32,
    shear: f32,
) -> Option<(i32, i32, i32, i32)> {
    transformed_raster_bounds(surface, x, y, width, height, shear, y + height / 2.0)
}

fn transformed_raster_bounds<T: SurfaceDrawTarget + ?Sized>(
    surface: &T,
    x: f32,
    y: f32,
    width: f32,
    height: f32,
    shear: f32,
    pivot_y: f32,
) -> Option<(i32, i32, i32, i32)> {
    if width <= 0.0
        || height <= 0.0
        || !x.is_finite()
        || !y.is_finite()
        || !width.is_finite()
        || !height.is_finite()
        || !shear.is_finite()
        || !pivot_y.is_finite()
        || surface.width() == 0
        || surface.height() == 0
    {
        return None;
    }
    let top_shift = shear * (y - pivot_y);
    let bottom_shift = shear * (y + height - pivot_y);
    let surface_width = i32::try_from(surface.width()).unwrap_or(i32::MAX);
    let surface_height = i32::try_from(surface.height()).unwrap_or(i32::MAX);
    let x0 = ((x + top_shift.min(bottom_shift) - 0.5).ceil() as i32).max(0);
    let x1 = ((x + width + top_shift.max(bottom_shift) - 0.5).ceil() as i32).min(surface_width);
    let y0 = ((y - 0.5).ceil() as i32).max(0);
    let y1 = ((y + height - 0.5).ceil() as i32).min(surface_height);
    (x0 < x1 && y0 < y1).then_some((x0, y0, x1, y1))
}

#[allow(clippy::too_many_arguments)]
fn inverse_sheared_sample(
    target_x: i32,
    target_y: i32,
    x: f32,
    y: f32,
    destination_width: f32,
    destination_height: f32,
    source_width: f32,
    source_height: f32,
    shear: f32,
) -> Option<(f32, f32)> {
    if destination_width <= 0.0
        || destination_height <= 0.0
        || source_width <= 0.0
        || source_height <= 0.0
    {
        return None;
    }
    let pixel_x = target_x as f32 + 0.5;
    let pixel_y = target_y as f32 + 0.5;
    let center_y = y + destination_height / 2.0;
    let unsheared_x = pixel_x - shear * (pixel_y - center_y);
    let local_x = unsheared_x - x;
    let local_y = pixel_y - y;
    if local_x < 0.0
        || local_y < 0.0
        || local_x >= destination_width
        || local_y >= destination_height
    {
        return None;
    }
    Some((
        local_x * source_width / destination_width - 0.5,
        local_y * source_height / destination_height - 0.5,
    ))
}

/// Blit one glyph cell at `(x, y)`, mirroring the GL character blit
/// (`src/StdFont.cpp:922-925`): texture RGBA modulated by the blit color
/// (`glColor` modulate, f32 round-to-nearest), then composited with
/// `glBlendFunc(GL_SRC_ALPHA, GL_ONE_MINUS_SRC_ALPHA)`. Clipped to the
/// surface; malformed `pixels` lengths are tolerated.
#[allow(clippy::too_many_arguments)]
fn blit_cell<T: SurfaceDrawTarget + ?Sized>(
    surface: &mut T,
    cell: &GlyphCell,
    cell_height: i32,
    x: i32,
    y: i32,
    mod_rgb: [u8; 3],
    color_alpha: u8,
    gamma: Option<&crate::GammaRamp>,
    shear: f32,
    transform_active: bool,
    texture_size: i32,
) {
    if let Some(config) = active_clonk_text_blit_config().filter(|config| config.changes_geometry())
    {
        blit_cell_configured(
            surface,
            cell,
            cell_height,
            x,
            y,
            mod_rgb,
            color_alpha,
            gamma,
            shear,
            transform_active,
            texture_size,
            config,
        );
        return;
    }
    if shear != 0.0 {
        let Some((x0, y0, x1, y1)) = sheared_raster_bounds(
            surface,
            x as f32,
            y as f32,
            cell.width as f32,
            cell_height as f32,
            shear,
        ) else {
            return;
        };
        for target_y in y0..y1 {
            for target_x in x0..x1 {
                let Some((sample_x, sample_y)) = inverse_sheared_sample(
                    target_x,
                    target_y,
                    x as f32,
                    y as f32,
                    cell.width as f32,
                    cell_height as f32,
                    cell.width as f32,
                    cell_height as f32,
                    shear,
                ) else {
                    continue;
                };
                blend_font_sample(
                    surface,
                    target_x as u32,
                    target_y as u32,
                    bilinear_glyph_sample(cell, cell_height, sample_x, sample_y),
                    mod_rgb,
                    color_alpha,
                    gamma,
                );
            }
        }
        return;
    }
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
            let out_a = font_sample_alpha(f32::from(px.a), color_alpha);
            if out_a <= 0.0 {
                continue; // fully transparent source leaves the surface unchanged
            }
            let (Some(dx), Some(dy)) = (offset_coord(x, col), offset_coord(y, row)) else {
                continue;
            };
            // Keep the unblended float source available to a retained GPU
            // target; CPU targets apply the same gamma and alpha composition
            // through SurfaceDrawTarget's reference implementation.
            let _ = surface.blend_fragment(
                dx,
                dy,
                [
                    f32::from(px.r) * f32::from(mod_rgb[0]) / 255.0,
                    f32::from(px.g) * f32::from(mod_rgb[1]) / 255.0,
                    f32::from(px.b) * f32::from(mod_rgb[2]) / 255.0,
                    out_a,
                ],
                gamma,
            );
        }
    }
}

#[allow(clippy::too_many_arguments)]
fn blit_cell_configured<T: SurfaceDrawTarget + ?Sized>(
    surface: &mut T,
    cell: &GlyphCell,
    cell_height: i32,
    x: i32,
    y: i32,
    mod_rgb: [u8; 3],
    color_alpha: u8,
    gamma: Option<&crate::GammaRamp>,
    shear: f32,
    transform_active: bool,
    texture_size: i32,
    config: ClonkTextBlitConfig,
) {
    let Some(mapping) = ConfiguredFontBlit::new(
        x as f32,
        y as f32,
        cell.width as f32,
        cell_height as f32,
        cell.width as f32,
        cell_height as f32,
        texture_size,
        shear,
        config,
    ) else {
        return;
    };
    let Some((x0, y0, x1, y1)) = mapping.raster_bounds(surface) else {
        return;
    };
    for target_y in y0..y1 {
        for target_x in x0..x1 {
            let sample = if transform_active {
                let Some((sample_x, sample_y)) = mapping.bilinear_sample(target_x, target_y) else {
                    continue;
                };
                bilinear_glyph_sample(cell, cell_height, sample_x, sample_y)
            } else {
                let Some((sample_x, sample_y)) = mapping.nearest_sample(target_x, target_y) else {
                    continue;
                };
                glyph_sample(cell, cell_height, sample_x, sample_y)
            };
            blend_font_sample(
                surface,
                target_x as u32,
                target_y as u32,
                sample,
                mod_rgb,
                color_alpha,
                gamma,
            );
        }
    }
}

fn glyph_sample(cell: &GlyphCell, cell_height: i32, x: i32, y: i32) -> [f32; 4] {
    if x < 0 || y < 0 || x >= cell.width || y >= cell_height {
        return [0.0; 4];
    }
    cell.pixels
        .get(y as usize * cell.width as usize + x as usize)
        .map(|pixel| {
            [
                f32::from(pixel.r),
                f32::from(pixel.g),
                f32::from(pixel.b),
                f32::from(pixel.a),
            ]
        })
        .unwrap_or([0.0; 4])
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

    struct TestImages {
        tag: &'static str,
        width: u32,
        height: u32,
        rgba: Vec<u8>,
    }

    impl FontImageProvider for TestImages {
        fn font_image(&self, tag: &str) -> Option<FontImageRef<'_>> {
            (tag == self.tag).then_some(FontImageRef {
                width: self.width,
                height: self.height,
                rgba: &self.rgba,
            })
        }
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
    fn active_markup_fragments_match_strict_cpp_stack_and_color_spelling() {
        assert_eq!(
            active_markup_fragments("<c 000001><i>text"),
            ("</i></c>".into(), "<c 1><i>".into())
        );
        assert_eq!(
            active_markup_fragments("<c 80ff0000>text"),
            ("</c>".into(), "<c 7fff0000>".into())
        );
        assert_eq!(
            active_markup_fragments("<c ff><i>text</c>"),
            ("</i></c>".into(), "<c ff><i>".into()),
            "a mismatched close does not mutate the active stack"
        );
    }

    #[test]
    fn inline_images_measure_missing_provider_aspect_and_escape_like_cpp() {
        let mut font = test_font();
        font.add_glyph(
            '{',
            GlyphCell {
                width: 2,
                pixels: vec![Color::opaque(255, 255, 255); 2 * 4],
            },
        );
        let images = TestImages {
            tag: "FLAM",
            width: 6,
            height: 3,
            rgba: vec![255; 6 * 3 * 4],
        };

        assert_eq!(font.measure_with_images("{{FLAM}}", true, &images), (8, 3));
        assert_eq!(
            font.measure_with_images("{{{FLAM}}", true, &images),
            (2 + font.h_space + 8, 3),
            "the first of three braces is literal"
        );
        assert_eq!(
            font.measure("A{{MISS}}B", true).0,
            font.measure("AB", true).0 + font.h_space,
            "GetTextExtent applies spacing even after an unresolved image"
        );

        // With spacing neutralized, the ticket's zero-width formulation is
        // visible directly: the missing token contributes no image width.
        let mut no_spacing = font;
        no_spacing.h_space = 0;
        assert_eq!(
            no_spacing.measure("A{{MISS}}B", true),
            no_spacing.measure("AB", true)
        );
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
    fn missing_unicode_glyph_uses_freetype_fallback_for_measure_and_draw() {
        let mut font = test_font();
        font.set_missing_glyph(GlyphCell {
            width: 3,
            pixels: vec![Color::opaque(255, 255, 255); 3 * 4],
        });

        assert!(
            font.glyph('\u{1f642}').is_none(),
            "fallback is not direct coverage"
        );
        assert_eq!(font.measure("\u{1f642}", false), (3, 3));

        let gamma = crate::GammaRamp::from_control_points([0x102030, 0x405060, 0x708090]);
        let mut sfc = surface();
        font.draw_with_gamma(
            &mut sfc,
            0,
            0,
            "<c 000000>\u{1f642}</c>",
            WHITE,
            TextAlign::Left,
            true,
            Some(&gamma),
        );

        assert_eq!(px(&sfc, 0, 0), Color::new(17, 33, 49, 255));
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

    fn text_blit_config() -> ClonkTextBlitConfig {
        ClonkTextBlitConfig {
            no_alpha_add: false,
            tex_indent: 0,
            blit_offset: 0,
            allowed_blit_modes: 15,
            shader: false,
        }
    }

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
    fn semantic_role_does_not_change_font_metrics() {
        let mut font = test_font();
        let before = (
            font.measure("AB", false),
            font.line_height,
            font.cell_height,
            font.h_space,
        );

        font.set_role(Some(ClonkFontRole::GuiText));

        assert_eq!(font.role(), Some(ClonkFontRole::GuiText));
        assert_eq!(
            (
                font.measure("AB", false),
                font.line_height,
                font.cell_height,
                font.h_space,
            ),
            before
        );
    }

    #[test]
    fn capture_suppresses_tagged_pixels_and_owns_draw_state() {
        let font = test_font().with_role(ClonkFontRole::GuiText);
        let gamma = crate::GammaRamp::from_control_points([0x102030, 0x405060, 0x708090]);
        let clip = Rect::new(1, 2, 7, 4);
        let mut sfc = surface();
        sfc.set_clip(clip);
        sfc.begin_clonk_text_capture();

        font.draw_with_gamma(
            &mut sfc,
            8,
            3,
            "<c ff0000>AB</c>",
            [11, 22, 33, 44],
            TextAlign::Center,
            true,
            Some(&gamma),
        );

        assert!(sfc.pixels().iter().all(|byte| *byte == 0));
        let commands = sfc.take_clonk_text_capture();
        assert_eq!(
            commands,
            vec![CapturedClonkText {
                role: ClonkFontRole::GuiText,
                x: 8,
                y: 3,
                text: "<c ff0000>AB</c>".to_string(),
                color: [11, 22, 33, 44],
                align: TextAlign::Center,
                markup: true,
                clip: Some(clip),
                gamma: Some(gamma),
                images: Vec::new(),
            }]
        );
        assert!(!sfc.is_clonk_text_capture_active());
    }

    #[test]
    fn capture_retains_draw_order() {
        let text = test_font().with_role(ClonkFontRole::GuiText);
        let caption = test_font().with_role(ClonkFontRole::GuiCaption);
        let mut sfc = surface();
        sfc.begin_clonk_text_capture();

        text.draw(&mut sfc, 1, 2, "first", WHITE, TextAlign::Left, false);
        caption.draw(&mut sfc, 3, 4, "second", WHITE, TextAlign::Right, false);

        let commands = sfc.take_clonk_text_capture();
        assert_eq!(commands.len(), 2);
        assert_eq!(commands[0].role, ClonkFontRole::GuiText);
        assert_eq!(commands[0].text, "first");
        assert_eq!(commands[1].role, ClonkFontRole::GuiCaption);
        assert_eq!(commands[1].text, "second");
    }

    #[test]
    fn capture_owns_inline_images_for_later_native_replay() {
        let font = test_font().with_role(ClonkFontRole::GuiText);
        let images = TestImages {
            tag: "FLAM",
            width: 2,
            height: 1,
            rgba: vec![255, 0, 0, 255, 0, 255, 0, 255],
        };
        let mut sfc = surface();
        sfc.begin_clonk_text_capture();
        font.draw_with_images(
            &mut sfc,
            0,
            0,
            "{{FLAM}}A{{FLAM}}",
            WHITE,
            TextAlign::Left,
            true,
            &images,
        );

        let commands = sfc.take_clonk_text_capture();
        assert_eq!(commands.len(), 1);
        assert_eq!(
            commands[0].images,
            vec![CapturedFontImage {
                tag: "FLAM".to_string(),
                width: 2,
                height: 1,
                rgba: images.rgba,
            }]
        );
    }

    #[test]
    fn untagged_font_still_draws_while_capture_is_active() {
        let font = test_font();
        let mut sfc = surface();
        sfc.begin_clonk_text_capture();

        font.draw(&mut sfc, 0, 0, "A", WHITE, TextAlign::Left, false);

        assert_eq!(px(&sfc, 0, 0), Color::opaque(255, 255, 255));
        assert!(sfc.take_clonk_text_capture().is_empty());
    }

    #[test]
    fn draw_inline_image_scales_to_gfx_height_and_advances_pen() {
        let font = test_font();
        let images = TestImages {
            tag: "FLAM",
            width: 2,
            height: 1,
            rgba: [255, 0, 0, 255, 255, 0, 0, 255].to_vec(),
        };
        let mut sfc = surface();
        font.draw_with_images(
            &mut sfc,
            0,
            0,
            "{{FLAM}}A",
            WHITE,
            TextAlign::Left,
            true,
            &images,
        );

        assert_eq!(px(&sfc, 0, 0), Color::opaque(255, 0, 0));
        assert_eq!(px(&sfc, 6, 0), Color::opaque(255, 0, 0));
        assert_eq!(px(&sfc, 7, 0), Color::opaque(255, 255, 255));
        assert_eq!(px(&sfc, 11, 0), Color::opaque(255, 255, 255));
        assert_eq!(px(&sfc, 12, 0).a, 0);
        assert!(px(&sfc, 0, 3).r > 0);
        assert_eq!(px(&sfc, 0, 4).a, 0);
    }

    #[test]
    fn italic_clonk_font_shears_glyphs_and_images_like_cpp() {
        fn changed_span(surface: &Surface, y: u32, background: Color) -> Option<(u32, u32)> {
            let changed = (0..surface.width())
                .filter(|x| surface.get_pixel(*x, y) != Some(background))
                .collect::<Vec<_>>();
            changed.first().copied().zip(changed.last().copied())
        }

        let mut font = ClonkFont::new(9);
        font.add_glyph(
            'X',
            GlyphCell {
                width: 6,
                pixels: vec![Color::opaque(255, 255, 255); 6 * 10],
            },
        );
        let images = TestImages {
            tag: "TEST",
            width: 6,
            height: 10,
            rgba: vec![255; 6 * 10 * 4],
        };
        let background = Color::opaque(3, 5, 7);
        let mut plain = Surface::new(48, 20, PixelFormat::Rgba8888);
        let mut italic = Surface::new(48, 20, PixelFormat::Rgba8888);
        let mut nested = Surface::new(48, 20, PixelFormat::Rgba8888);
        let mut image = Surface::new(48, 20, PixelFormat::Rgba8888);
        for surface in [&mut plain, &mut italic, &mut nested, &mut image] {
            surface.fill(background);
        }

        font.draw(&mut plain, 20, 4, "X", WHITE, TextAlign::Left, true);
        font.draw(&mut italic, 20, 4, "<i>X</i>", WHITE, TextAlign::Left, true);
        font.draw(
            &mut nested,
            20,
            4,
            "<i><i>X</i></i>",
            WHITE,
            TextAlign::Left,
            true,
        );
        font.draw_with_images(
            &mut image,
            20,
            4,
            "<i>{{TEST}}</i>",
            [100, 150, 200, 255],
            TextAlign::Left,
            true,
            &images,
        );

        // Keep the tag open so this isolates italic metrics from the native
        // trailing-close-tag h-space quirk pinned by the extent tests below.
        assert_eq!(font.measure("X", true), font.measure("<i>X", true));
        assert_eq!(
            font.measure_with_images("{{TEST}}", true, &images),
            font.measure_with_images("<i>{{TEST}}", true, &images),
        );
        let plain_top = changed_span(&plain, 4, background).expect("plain top row");
        let plain_bottom = changed_span(&plain, 13, background).expect("plain bottom row");
        let italic_top = changed_span(&italic, 4, background).expect("italic top row");
        let italic_bottom = changed_span(&italic, 13, background).expect("italic bottom row");
        let nested_top = changed_span(&nested, 4, background).expect("nested italic top row");
        let image_top = changed_span(&image, 4, background).expect("italic image top row");
        let image_bottom = changed_span(&image, 13, background).expect("italic image bottom row");
        assert!(italic_top.0 > plain_top.0, "top edge shears right");
        assert!(italic_bottom.0 < plain_bottom.0, "bottom edge shears left");
        assert!(nested_top.0 > italic_top.0, "nested italics accumulate");
        assert_eq!(image_top, italic_top);
        assert_eq!(image_bottom, italic_bottom);
        assert_eq!(image.get_pixel(22, 9), Some(Color::opaque(100, 150, 200)));
        assert_eq!(markup_shear(&[MarkupTag::Italic]), -0.3_f32);
        assert_eq!(
            markup_shear(&[MarkupTag::Italic, MarkupTag::Italic, MarkupTag::Italic,]),
            ((0.0_f32 - 0.3) - 0.3) - 0.3,
        );

        let mut plain_sequence = Surface::new(48, 20, PixelFormat::Rgba8888);
        let mut italic_sequence = Surface::new(48, 20, PixelFormat::Rgba8888);
        plain_sequence.fill(background);
        italic_sequence.fill(background);
        font.draw(
            &mut plain_sequence,
            20,
            4,
            "X<c ff0000>X</c>",
            WHITE,
            TextAlign::Left,
            true,
        );
        font.draw(
            &mut italic_sequence,
            20,
            4,
            "<i>X</i><c ff0000>X</c>",
            WHITE,
            TextAlign::Left,
            true,
        );
        let red_span = |surface: &Surface| {
            let pixels = (0..surface.width())
                .filter(|x| {
                    surface
                        .get_pixel(*x, 9)
                        .is_some_and(|pixel| pixel.r > pixel.g && pixel.r > pixel.b)
                })
                .collect::<Vec<_>>();
            pixels.first().copied().zip(pixels.last().copied())
        };
        assert_eq!(red_span(&italic_sequence), red_span(&plain_sequence));

        let mut clipped = Surface::new(48, 20, PixelFormat::Rgba8888);
        clipped.fill(background);
        clipped.set_clip(Rect::new(21, 4, 4, 10));
        font.draw(
            &mut clipped,
            20,
            4,
            "<i>X</i>",
            WHITE,
            TextAlign::Left,
            true,
        );
        assert_ne!(italic.get_pixel(25, 4), Some(background));
        assert_ne!(italic.get_pixel(20, 13), Some(background));
        assert_ne!(clipped.get_pixel(21, 4), Some(background));
        assert_eq!(clipped.get_pixel(25, 4), Some(background));
        assert_eq!(clipped.get_pixel(20, 13), Some(background));
        assert_eq!(clipped.get_pixel(25, 13), Some(background));
    }

    #[test]
    fn draw_zero_height_inline_image_is_unresolved_without_advance() {
        let font = test_font();
        let images = TestImages {
            tag: "FLAM",
            width: 2,
            height: 0,
            rgba: Vec::new(),
        };
        let mut sfc = surface();
        font.draw_with_images(
            &mut sfc,
            0,
            0,
            "{{FLAM}}A",
            WHITE,
            TextAlign::Left,
            true,
            &images,
        );

        assert_eq!(px(&sfc, 4, 0), Color::opaque(255, 255, 255));
        assert_eq!(px(&sfc, 5, 0).a, 0);
    }

    #[test]
    fn draw_gamma_samples_independent_rgb_tables() {
        // Font glyphs use the same three-channel blit shader as sprites
        // (StdFont.cpp:922-925; StdGL.cpp:1068-1087).
        let font = test_font();
        let gamma = crate::GammaRamp::from_control_points([0x102030, 0x405060, 0x708090]);
        let mut sfc = surface();
        font.draw_with_gamma(
            &mut sfc,
            0,
            0,
            "A",
            [0, 0, 0, 255],
            TextAlign::Left,
            false,
            Some(&gamma),
        );

        assert_eq!(px(&sfc, 0, 0), Color::new(17, 33, 49, 255));
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
        // Half-transparent red: native inverted-alpha addition produces
        // 255-(255-128)=128; framebuffer blending then yields rgb 128/0/0
        // and alpha round(128²/255)=64 over transparent black.
        font.draw(
            &mut sfc,
            0,
            0,
            "A",
            [255, 0, 0, 128],
            TextAlign::Left,
            false,
        );
        assert_eq!(px(&sfc, 0, 0), Color::new(128, 0, 0, 64));
    }

    #[test]
    fn draw_adds_inverted_modulation_alpha_instead_of_multiplying() {
        let mut font = test_font();
        let cell_height = font.cell_height as usize;
        font.add_glyph(
            'X',
            GlyphCell {
                width: 1,
                pixels: vec![Color::new(255, 0, 0, 200); cell_height],
            },
        );
        let mut sfc = surface();

        font.draw(
            &mut sfc,
            0,
            0,
            "X",
            [255, 255, 255, 128],
            TextAlign::Left,
            false,
        );

        // Filtered alpha 200 with opacity 128 becomes 200-(255-128)=73.
        // GL_SRC_ALPHA blending stores red 73 and alpha round(73²/255)=21.
        assert_eq!(px(&sfc, 0, 0), Color::new(73, 0, 0, 21));
    }

    #[test]
    fn configured_glyph_blit_applies_blit_offset_and_tex_indent() {
        let mut font = ClonkFont::new(3);
        font.cell_height = 4;
        font.h_space = 0;
        font.set_texture_size(128);
        let row = [
            Color::opaque(10, 0, 0),
            Color::opaque(100, 0, 0),
            Color::opaque(200, 0, 0),
        ];
        font.add_glyph(
            'X',
            GlyphCell {
                width: 3,
                pixels: row.repeat(4),
            },
        );

        let render = |tex_indent| {
            let mut surface = Surface::new(5, 6, PixelFormat::Rgba8888);
            let _config = activate_clonk_text_blit_config(ClonkTextBlitConfig {
                tex_indent,
                blit_offset: 100,
                ..text_blit_config()
            });
            font.draw(&mut surface, 0, 0, "X", WHITE, TextAlign::Left, false);
            surface
        };
        let unindented = render(0);
        let indented = render(1000);

        assert_eq!(px(&unindented, 0, 0).a, 0);
        assert_eq!(px(&unindented, 1, 1), Color::opaque(10, 0, 0));
        assert_eq!(px(&indented, 0, 0).a, 0);
        assert_eq!(px(&indented, 1, 1), Color::opaque(100, 0, 0));
    }

    #[test]
    fn configured_font_image_blit_applies_blit_offset_and_tex_indent() {
        let mut font = ClonkFont::new(3);
        font.cell_height = 4;
        font.h_space = 0;
        let row = [10, 0, 0, 255, 100, 0, 0, 255, 200, 0, 0, 255];
        let images = TestImages {
            tag: "TEST",
            width: 3,
            height: 4,
            rgba: row.repeat(4),
        };
        let render = |tex_indent| {
            let mut surface = Surface::new(5, 6, PixelFormat::Rgba8888);
            let _config = activate_clonk_text_blit_config(ClonkTextBlitConfig {
                tex_indent,
                blit_offset: 100,
                ..text_blit_config()
            });
            font.draw_with_images(
                &mut surface,
                0,
                0,
                "{{TEST}}",
                WHITE,
                TextAlign::Left,
                true,
                &images,
            );
            surface
        };
        let unindented = render(0);
        let indented = render(1000);

        assert_eq!(px(&unindented, 0, 0).a, 0);
        assert_eq!(px(&unindented, 1, 1), Color::opaque(10, 0, 0));
        assert_eq!(px(&indented, 0, 0).a, 0);
        assert!(px(&indented, 1, 1).r > px(&unindented, 1, 1).r);
    }

    #[test]
    fn fixed_no_alpha_add_preserves_glyph_and_font_image_opacity() {
        let mut font = ClonkFont::new(0);
        font.cell_height = 1;
        font.h_space = 0;
        font.add_glyph(
            'X',
            GlyphCell {
                width: 1,
                pixels: vec![Color::new(255, 0, 0, 200)],
            },
        );
        let images = TestImages {
            tag: "TEST",
            width: 1,
            height: 1,
            rgba: vec![255, 0, 0, 200],
        };
        let render = |shader, image| {
            let mut surface = Surface::new(2, 1, PixelFormat::Rgba8888);
            let _config = activate_clonk_text_blit_config(ClonkTextBlitConfig {
                no_alpha_add: true,
                shader,
                ..text_blit_config()
            });
            if image {
                font.draw_with_images(
                    &mut surface,
                    0,
                    0,
                    "{{TEST}}",
                    [255, 255, 255, 128],
                    TextAlign::Left,
                    true,
                    &images,
                );
            } else {
                font.draw(
                    &mut surface,
                    0,
                    0,
                    "X",
                    [255, 255, 255, 128],
                    TextAlign::Left,
                    false,
                );
            }
            px(&surface, 0, 0)
        };

        assert_eq!(render(false, false), Color::new(200, 0, 0, 157));
        assert_eq!(render(false, true), Color::new(200, 0, 0, 157));
        assert_eq!(render(true, false), Color::new(73, 0, 0, 21));
        assert_eq!(render(true, true), Color::new(73, 0, 0, 21));
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
