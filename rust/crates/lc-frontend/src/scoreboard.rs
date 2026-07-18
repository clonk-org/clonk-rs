//! C++-faithful `C4ScoreboardDlg` layout and rendering.
//!
//! The deterministic scoreboard matrix and show-count lifecycle live in
//! `lc-engine`. This module owns only the presentation performed by
//! `C4ScoreboardDlg` (`src/C4Scoreboard.cpp:292-372`) and the standard
//! `C4GUI::Dialog`/`WoodenLabel` furniture beneath it.

use std::collections::HashMap;

use anyhow::{ensure, Result};
use lc_engine::ScoreboardState;
use lc_graphics::clonk_font::{markup_blit_color, ClonkFont, GlyphCell, TextAlign};
use lc_graphics::gamma::GammaChannel;
use lc_graphics::Color;
#[cfg(test)]
use lc_graphics::PixelFormat;
use lc_graphics::{GammaRamp, Surface};

use crate::classic_gui::{
    draw_3d_frame, draw_bar, draw_engine_box, draw_facet_stretch, IntRect,
    STANDARD_BACKGROUND_COLOR,
};
use crate::{ClonkFontSet, ImageData};

fn presentation_text(text: &str) -> String {
    crate::c4_presentation_text(text)
}

const X_INDENT: i32 = 4;
const Y_INDENT: i32 = 4;
const X_MARGIN: i32 = 3;
const Y_MARGIN: i32 = 3;
const TITLE_EXTRA_WIDTH: i32 = 40;
const MIN_WOOD_BAR_HEIGHT: i32 = 23;
const PLACEMENT_RIGHT_INSET: i32 = 20;
const PLACEMENT_TOP_INSET: i32 = 38;
const ICON_CELL: u32 = 40;
const PLAYER_ICON_PHASE: u32 = 9;
const CLOSE_ICON_PHASE: u32 = 34;
const CAPTION_RIGHT_INDENT: i32 = 20;
const CAPTION_ICON_INSET: i32 = 1;
const CLOSE_BUTTON_SIZE: i32 = 16;
const CLOSE_BUTTON_INSET: i32 = 4;
/// `CMarkupTagItalic::Apply`: each open `<i>` subtracts 0.3 from the
/// destination-space x/y matrix term (src/StdMarkup.cpp:24-28).
const ITALIC_SHEAR: f32 = -0.3;

/// The text anchor and alignment used for one scoreboard matrix cell.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ScoreboardTextAnchor {
    pub x: i32,
    pub y: i32,
    pub align: TextAlign,
}

/// Fully resolved `C4ScoreboardDlg` geometry in screen coordinates.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ScoreboardLayout {
    pub bounds: IntRect,
    pub caption: Option<IntRect>,
    pub client: IntRect,
    pub title_icon: Option<IntRect>,
    pub close_button: Option<IntRect>,
    pub column_widths: Vec<i32>,
    pub row_height: i32,
    rows: usize,
    columns: usize,
}

impl ScoreboardLayout {
    /// `C4ScoreboardDlg::DrawElement`: column zero is left-aligned; every
    /// other column is centered in its calculated width.
    pub fn cell_text_anchor(&self, row: usize, column: usize) -> Option<ScoreboardTextAnchor> {
        if row >= self.rows || column >= self.columns {
            return None;
        }
        let column_x = self
            .column_widths
            .iter()
            .take(column)
            .fold(self.client.x + X_MARGIN, |x, width| {
                x.saturating_add(*width)
            });
        Some(ScoreboardTextAnchor {
            x: if column == 0 {
                column_x
            } else {
                column_x + self.column_widths[column] / 2
            },
            y: self.client.y + Y_MARGIN + row as i32 * self.row_height,
            align: if column == 0 {
                TextAlign::Left
            } else {
                TextAlign::Center
            },
        })
    }
}

/// Validated process-global assets used by `C4ScoreboardDlg`.
///
/// `font_images` mirrors `FontRegular.SetCustomImages(&Game.Defs)`. Callers
/// may use [`scoreboard_inline_image_specs`] to resolve only the definitions
/// referenced by the live matrix; layout and rendering fail when any such
/// token is absent or resolves to an empty image.
#[derive(Clone, Copy)]
pub struct ScoreboardResources<'a> {
    caption: &'a ImageData,
    icons: &'a ImageData,
    fonts: &'a ClonkFontSet,
    font_images: Option<&'a HashMap<String, ImageData>>,
}

impl<'a> ScoreboardResources<'a> {
    pub fn new(
        caption: &'a ImageData,
        icons: &'a ImageData,
        fonts: &'a ClonkFontSet,
    ) -> Result<Self> {
        let resources = Self {
            caption,
            icons,
            fonts,
            font_images: None,
        };
        resources.validate()?;
        Ok(resources)
    }

    pub fn with_font_images(mut self, images: &'a HashMap<String, ImageData>) -> Self {
        self.font_images = Some(images);
        self
    }

    pub fn fonts(&self) -> &ClonkFontSet {
        self.fonts
    }

    fn font_images(&self) -> &HashMap<String, ImageData> {
        static EMPTY: std::sync::OnceLock<HashMap<String, ImageData>> = std::sync::OnceLock::new();
        self.font_images
            .unwrap_or_else(|| EMPTY.get_or_init(HashMap::new))
    }

    fn validate(&self) -> Result<()> {
        ensure!(
            (self.caption.width(), self.caption.height()) == (192, 23),
            "GUICaption.png must be the exact 192x23 classic sheet, got {}x{}",
            self.caption.width(),
            self.caption.height()
        );
        ensure!(
            (self.icons.width(), self.icons.height()) == (240, 360),
            "GUIIcons.png must be the exact 240x360 classic sheet, got {}x{}",
            self.icons.width(),
            self.icons.height()
        );
        ensure!(
            self.fonts.text.line_height > 0 && self.fonts.text.cell_height > 0,
            "classic FontRegular must have positive line and cell heights"
        );
        Ok(())
    }
}

/// Return every `{{image spec}}` used by the matrix in C++ row-major order,
/// de-duplicated by first occurrence.
pub fn scoreboard_inline_image_specs(scoreboard: &ScoreboardState) -> Vec<String> {
    let mut specs = Vec::new();
    for row in 0..scoreboard.row_count() {
        for column in 0..scoreboard.column_count() {
            if let Some(text) = scoreboard.cell(row, column).and_then(|cell| cell.text()) {
                collect_inline_image_specs(&presentation_text(text), &mut specs);
            }
        }
    }
    specs
}

/// Calculate the dialog exactly as `C4ScoreboardDlg::Update` and
/// `DoPlacement` do. `preferred` is `C4GUI::Screen::GetPreferredDlgRect()`.
pub fn scoreboard_layout(
    preferred: IntRect,
    scoreboard: &ScoreboardState,
    resources: &ScoreboardResources<'_>,
) -> Result<ScoreboardLayout> {
    resources.validate()?;
    ensure!(
        preferred.w >= 0 && preferred.h >= 0,
        "invalid preferred dialog rectangle"
    );

    let rows = scoreboard.row_count();
    let columns = scoreboard.column_count();
    ensure!(
        rows != 0 && columns != 0,
        "C4ScoreboardDlg requires a non-empty rectangular matrix"
    );
    for row in 0..rows {
        for column in 0..columns {
            ensure!(
                scoreboard.cell(row, column).is_some(),
                "scoreboard matrix is not rectangular at row {row}, column {column}"
            );
        }
    }

    let font = &resources.fonts.text;
    let images = resources.font_images();
    for spec in scoreboard_inline_image_specs(scoreboard) {
        ensure!(
            font_image(images, &spec).is_some(),
            "scoreboard FontRegular image token `{{{{{spec}}}}}` is unresolved"
        );
    }
    let mut column_widths = Vec::with_capacity(columns);
    for column in 0..columns {
        let mut width = X_INDENT;
        for row in 0..rows {
            let cell = scoreboard
                .cell(row, column)
                .expect("matrix shape was validated above");
            // The title corner is never part of the spreadsheet width scan.
            // An allocated empty StdStrBuf still participates, unlike null.
            if (row != 0 || column != 0) && cell.text().is_some() {
                let text = presentation_text(cell.text().unwrap_or_default());
                width = width.max(
                    scoreboard_text_width(&text, font, images).saturating_add(X_INDENT),
                );
            }
        }
        column_widths.push(width);
    }

    let columns_width = column_widths
        .iter()
        .copied()
        .fold(0_i32, i32::saturating_add);
    let mut width = X_MARGIN.saturating_mul(2).saturating_add(columns_width);
    let title = scoreboard.cell(0, 0).and_then(|cell| cell.text());
    // C++ tests the StdStrBuf pointer here, not whether the string is empty.
    if let Some(title) = title {
        let title = presentation_text(title);
        width =
            width.max(scoreboard_text_width(&title, font, images).saturating_add(TITLE_EXTRA_WIDTH));
    }

    // The constructor initially installs "nops". For an allocated-but-empty
    // (0,0), Update therefore computes this margin before final SetTitle("")
    // removes the visible WoodenLabel without updating rcClientRect again.
    let title_margin_height = title
        .is_some()
        .then(|| font.line_height.max(MIN_WOOD_BAR_HEIGHT));
    let visible_caption_height = title
        .is_some_and(|title| !title.is_empty())
        .then(|| font.line_height.max(MIN_WOOD_BAR_HEIGHT));
    let row_height = font.line_height.saturating_add(Y_INDENT);
    let client_height = Y_MARGIN
        .saturating_mul(2)
        .saturating_add((rows as i32).saturating_mul(row_height));
    let height = client_height.saturating_add(title_margin_height.unwrap_or(0));
    ensure!(
        width > 0 && height > 0,
        "scoreboard layout overflowed its bounds"
    );

    let x = preferred
        .x
        .saturating_add(preferred.w)
        .saturating_sub(width)
        .saturating_sub(PLACEMENT_RIGHT_INSET);
    let y = preferred.y.saturating_add(PLACEMENT_TOP_INSET);
    let bounds = IntRect {
        x,
        y,
        w: width,
        h: height,
    };
    let caption = visible_caption_height.map(|title_height| IntRect {
        x,
        y,
        w: width,
        h: title_height,
    });
    let client = IntRect {
        x,
        y: y + title_margin_height.unwrap_or(0),
        w: width,
        h: client_height,
    };
    let title_icon = caption.map(|caption| IntRect {
        x: caption.x + CAPTION_ICON_INSET,
        y: caption.y + CAPTION_ICON_INSET,
        w: caption.h - 2 * CAPTION_ICON_INSET,
        h: caption.h - 2 * CAPTION_ICON_INSET,
    });
    let close_button = caption.map(|caption| IntRect {
        x: caption.x + caption.w - CLOSE_BUTTON_SIZE - CLOSE_BUTTON_INSET,
        y: caption.y + CLOSE_BUTTON_INSET,
        w: CLOSE_BUTTON_SIZE,
        h: CLOSE_BUTTON_SIZE,
    });

    Ok(ScoreboardLayout {
        bounds,
        caption,
        client,
        title_icon,
        close_button,
        column_widths,
        row_height,
        rows,
        columns,
    })
}

/// Draw a visible scoreboard dialog. Lifecycle and input remain app-owned;
/// this function performs no state mutation.
pub fn render_scoreboard(
    surface: &mut Surface,
    preferred: IntRect,
    scoreboard: &ScoreboardState,
    resources: &ScoreboardResources<'_>,
    gamma: Option<&GammaRamp>,
) -> Result<()> {
    let layout = scoreboard_layout(preferred, scoreboard, resources)?;
    render_scoreboard_body_with_layout(surface, scoreboard, resources, &layout, gamma);
    render_scoreboard_caption_with_layout(surface, scoreboard, resources, &layout, gamma)
}

/// Draw the dialog background/frame and spreadsheet cells, stopping before
/// the caption child. Ordered presentation commits captured cell text here so
/// the later caption chrome can cover it exactly as `C4GUI::Window::Draw` does.
pub fn render_scoreboard_body(
    surface: &mut Surface,
    preferred: IntRect,
    scoreboard: &ScoreboardState,
    resources: &ScoreboardResources<'_>,
    gamma: Option<&GammaRamp>,
) -> Result<()> {
    let layout = scoreboard_layout(preferred, scoreboard, resources)?;
    render_scoreboard_body_with_layout(surface, scoreboard, resources, &layout, gamma);
    Ok(())
}

/// Draw the caption bar, icon, markup-aware title, and close button after
/// [`render_scoreboard_body`].
pub fn render_scoreboard_caption(
    surface: &mut Surface,
    preferred: IntRect,
    scoreboard: &ScoreboardState,
    resources: &ScoreboardResources<'_>,
    gamma: Option<&GammaRamp>,
) -> Result<()> {
    let layout = scoreboard_layout(preferred, scoreboard, resources)?;
    render_scoreboard_caption_with_layout(surface, scoreboard, resources, &layout, gamma)
}

fn render_scoreboard_body_with_layout(
    surface: &mut Surface,
    scoreboard: &ScoreboardState,
    resources: &ScoreboardResources<'_>,
    layout: &ScoreboardLayout,
    gamma: Option<&GammaRamp>,
) {
    draw_engine_box(
        surface,
        layout.bounds.x,
        layout.bounds.y,
        layout.bounds.x + layout.bounds.w - 1,
        layout.bounds.y + layout.bounds.h - 1,
        STANDARD_BACKGROUND_COLOR,
        gamma,
    );
    draw_3d_frame(surface, layout.bounds, gamma);

    // C4ScoreboardDlg paints the spreadsheet from DrawElement itself, before
    // Window::Draw narrows the clipper for child controls. Deliberately keep
    // multiline and italic overflow outside rcClientRect visible.
    for row in 0..scoreboard.row_count() {
        for column in 0..scoreboard.column_count() {
            if row == 0 && column == 0 {
                continue;
            }
            let Some(text) = scoreboard
                .cell(row, column)
                .and_then(|cell| cell.text())
                .filter(|text| !text.is_empty())
            else {
                continue;
            };
            let text = presentation_text(text);
            let anchor = layout
                .cell_text_anchor(row, column)
                .expect("layout and scoreboard dimensions match");
            draw_scoreboard_text(
                surface,
                anchor.x,
                anchor.y,
                &text,
                &resources.fonts.text,
                resources.font_images(),
                anchor.align,
                gamma,
            );
        }
    }
}

fn render_scoreboard_caption_with_layout(
    surface: &mut Surface,
    scoreboard: &ScoreboardState,
    resources: &ScoreboardResources<'_>,
    layout: &ScoreboardLayout,
    gamma: Option<&GammaRamp>,
) -> Result<()> {
    if let (Some(caption), Some(title)) = (
        layout.caption,
        scoreboard.cell(0, 0).and_then(|cell| cell.text()),
    ) {
        let title = presentation_text(title);
        draw_bar(surface, caption, resources.caption, 32, gamma);
        if let Some(icon) = layout.title_icon {
            draw_icon_phase(surface, resources.icons, PLAYER_ICON_PHASE, icon, gamma)?;
        }

        // WoodenLabel clips after its icon indent and before its 20px close
        // control, while keeping the left text offset at +5.
        let text_clip = IntRect {
            x: caption.x + caption.h,
            y: caption.y,
            w: (caption.w - caption.h - CAPTION_RIGHT_INDENT + 1).max(0),
            h: caption.h + 1,
        };
        let text_y = caption.y + (caption.h - resources.fonts.text.line_height) / 2 - 1;
        with_surface_clip(surface, text_clip, |caption_surface| {
            // WoodenLabel stores fMarkup=false here, but its DrawElement
            // override omits that argument and therefore calls markup-enabled
            // TextOut. Preserve that C++ quirk for colors, italics and images.
            draw_scoreboard_text(
                caption_surface,
                caption.x + caption.h + 5,
                text_y,
                &title,
                &resources.fonts.text,
                resources.font_images(),
                TextAlign::Left,
                gamma,
            );
        });
        if let Some(close) = layout.close_button {
            draw_icon_phase(surface, resources.icons, CLOSE_ICON_PHASE, close, gamma)?;
        }
    }
    Ok(())
}

fn draw_icon_phase(
    surface: &mut Surface,
    icons: &ImageData,
    phase: u32,
    destination: IntRect,
    gamma: Option<&GammaRamp>,
) -> Result<()> {
    let columns = icons.width() / ICON_CELL;
    ensure!(columns != 0, "GUIIcons.png has no complete icon columns");
    let source_x = (phase % columns) * ICON_CELL;
    let source_y = (phase / columns) * ICON_CELL;
    ensure!(
        source_x + ICON_CELL <= icons.width() && source_y + ICON_CELL <= icons.height(),
        "GUIIcons.png does not contain classic icon phase {phase}"
    );
    draw_facet_stretch(
        surface,
        icons,
        (
            source_x as f32,
            source_y as f32,
            ICON_CELL as f32,
            ICON_CELL as f32,
        ),
        (
            destination.x as f32,
            destination.y as f32,
            destination.w as f32,
            destination.h as f32,
        ),
        gamma,
    );
    Ok(())
}

fn with_surface_clip(surface: &mut Surface, clip: IntRect, draw: impl FnOnce(&mut Surface)) {
    let previous = surface.clip();
    let mut left = i64::from(clip.x).max(0);
    let mut top = i64::from(clip.y).max(0);
    let mut right = (i64::from(clip.x) + i64::from(clip.w.max(0)))
        .min(i64::from(surface.width().min(i32::MAX as u32)));
    let mut bottom = (i64::from(clip.y) + i64::from(clip.h.max(0)))
        .min(i64::from(surface.height().min(i32::MAX as u32)));
    if let Some(existing) = previous {
        left = left.max(i64::from(existing.x));
        top = top.max(i64::from(existing.y));
        right = right.min(i64::from(existing.x) + i64::from(existing.width));
        bottom = bottom.min(i64::from(existing.y) + i64::from(existing.height));
    }
    if left < right && top < bottom {
        surface.set_clip(lc_graphics::Rect::new(
            left as i32,
            top as i32,
            (right - left) as u32,
            (bottom - top) as u32,
        ));
        draw(surface);
    }
    if let Some(existing) = previous {
        surface.set_clip(existing);
    } else {
        surface.clear_clip();
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum ScoreboardMarkupTag {
    Italic,
    TextColor(u32),
}

impl ScoreboardMarkupTag {
    const fn name(self) -> &'static str {
        match self {
            Self::Italic => "i",
            Self::TextColor(_) => "c",
        }
    }
}

fn scoreboard_text_width(text: &str, font: &ClonkFont, images: &HashMap<String, ImageData>) -> i32 {
    // CStdFont::GetTextExtent walks the unsplit source. It resets the row at
    // `|`/newline, but the preceding glyph has already received iHSpace
    // because raw source (the separator) remained. With FontRegular's -1
    // spacing, layout can therefore be one pixel narrower than TextOut's
    // independently aligned line. Preserve that deliberate mismatch.
    let mut maximum = 0_i32;
    let mut row_width = 0_i32;
    let mut rest = text;
    while !rest.is_empty() {
        while rest.starts_with('<') {
            let Some(advance) = skip_markup_tag(rest) else {
                break;
            };
            rest = &rest[advance..];
        }
        if rest.is_empty() {
            break;
        }
        if let Some((spec, advance)) = inline_image_token(rest) {
            if let Some(image) = font_image(images, spec) {
                row_width = row_width.saturating_add(scaled_font_image_width(font, image));
            }
            rest = &rest[advance..];
            if !rest.is_empty() {
                row_width = row_width.saturating_add(font.h_space);
            }
            maximum = maximum.max(row_width);
            continue;
        }
        let character = rest.chars().next().expect("non-empty text");
        rest = &rest[character.len_utf8()..];
        if character == '\n' || character == '|' {
            row_width = 0;
            continue;
        }
        if character < ' ' {
            continue;
        }
        row_width = row_width.saturating_add(font.glyph(character).map_or(0, |glyph| glyph.width));
        if !rest.is_empty() {
            row_width = row_width.saturating_add(font.h_space);
        }
        maximum = maximum.max(row_width);
    }
    maximum
}

fn scoreboard_line_width(
    mut text: &str,
    font: &ClonkFont,
    images: &HashMap<String, ImageData>,
) -> i32 {
    let mut width = 0_i32;
    while !text.is_empty() {
        while text.starts_with('<') {
            let Some(advance) = skip_markup_tag(text) else {
                break;
            };
            text = &text[advance..];
        }
        if text.is_empty() {
            break;
        }
        if let Some((spec, advance)) = inline_image_token(text) {
            if let Some(image) = font_image(images, spec) {
                width = width.saturating_add(scaled_font_image_width(font, image));
            }
            text = &text[advance..];
            if !text.is_empty() {
                width = width.saturating_add(font.h_space);
            }
            continue;
        }
        let character = text.chars().next().expect("non-empty text");
        text = &text[character.len_utf8()..];
        if character < ' ' {
            continue;
        }
        width = width.saturating_add(font.glyph(character).map_or(0, |glyph| glyph.width));
        if !text.is_empty() {
            width = width.saturating_add(font.h_space);
        }
    }
    width
}

#[allow(clippy::too_many_arguments)]
fn draw_scoreboard_text(
    surface: &mut Surface,
    x: i32,
    y: i32,
    text: &str,
    font: &ClonkFont,
    images: &HashMap<String, ImageData>,
    align: TextAlign,
    gamma: Option<&GammaRamp>,
) {
    let mut markup = Vec::new();
    for (line_index, line) in text.split(['\n', '|']).enumerate() {
        let line_width = scoreboard_line_width(line, font, images);
        let mut pen_x = x.saturating_sub(match align {
            TextAlign::Left => 0,
            TextAlign::Center => line_width / 2,
            TextAlign::Right => line_width,
        });
        let line_y = y.saturating_add(line_index as i32 * font.line_height);
        let mut rest = line;
        while !rest.is_empty() {
            if rest.starts_with('<') {
                if let Some(advance) = read_markup_tag(rest, &mut markup) {
                    rest = &rest[advance..];
                    continue;
                }
            }
            if let Some((spec, advance)) = inline_image_token(rest) {
                rest = &rest[advance..];
                let Some(image) = font_image(images, spec) else {
                    // CStdFont ignores an unresolved custom image without
                    // consuming horizontal spacing in the draw pass.
                    continue;
                };
                let width = scaled_font_image_width(font, image);
                if width > 0 {
                    let modulation = markup_rgba(&markup);
                    draw_scoreboard_font_image(
                        surface,
                        image,
                        pen_x,
                        line_y,
                        width,
                        font.cell_height,
                        modulation,
                        markup_shear(&markup),
                        gamma,
                    );
                    pen_x = pen_x.saturating_add(width).saturating_add(font.h_space);
                }
                continue;
            }

            let character = rest.chars().next().expect("non-empty text");
            rest = &rest[character.len_utf8()..];
            if character < ' ' {
                continue;
            }
            let color = markup_rgba(&markup);
            if let Some(glyph) = font.glyph(character) {
                let shear = markup_shear(&markup);
                let native_capture =
                    font.role().is_some() && surface.is_clonk_text_capture_active();
                if native_capture || (shear == 0.0 && color[3] == 255) {
                    font.draw_with_gamma(
                        surface,
                        pen_x,
                        line_y,
                        &character.to_string(),
                        color,
                        TextAlign::Left,
                        false,
                        gamma,
                    );
                } else {
                    draw_sheared_glyph(
                        surface,
                        glyph,
                        font.cell_height,
                        pen_x,
                        line_y,
                        color,
                        shear,
                        gamma,
                    );
                }
            }
            pen_x = pen_x
                .saturating_add(font.glyph(character).map_or(0, |glyph| glyph.width))
                .saturating_add(font.h_space);
        }
    }
}

fn markup_rgba(stack: &[ScoreboardMarkupTag]) -> [u8; 4] {
    stack
        .iter()
        .rev()
        .find_map(|tag| match tag {
            ScoreboardMarkupTag::TextColor(color) => Some(*color),
            ScoreboardMarkupTag::Italic => None,
        })
        .map(|color| {
            // Opaque white is the base DrawText color. C++ skips
            // ModulateClrA when a tag resolves to that exact value, avoiding
            // the otherwise observable 255 -> 254 channel quirk.
            if color == 0x00ff_ffff {
                return [255, 255, 255, 255];
            }
            let rgb = markup_blit_color([
                (color >> 16) as u8,
                (color >> 8) as u8,
                color as u8,
            ]);
            [rgb[0], rgb[1], rgb[2], 255 - (color >> 24) as u8]
        })
        .unwrap_or([255, 255, 255, 255])
}

fn markup_shear(stack: &[ScoreboardMarkupTag]) -> f32 {
    stack
        .iter()
        .filter(|tag| matches!(tag, ScoreboardMarkupTag::Italic))
        .count() as f32
        * ITALIC_SHEAR
}

fn markup_sample_alpha(sample_alpha: f32, tag_alpha: u8) -> f32 {
    // Default OpenGL path uses GL_ADD for the font texture's alpha and the
    // primary modulation's *inverted* alpha. Converted back to normal
    // opacity this is max(sample - (255 - tag), 0), not multiplication.
    (sample_alpha - f32::from(255 - tag_alpha)).max(0.0)
}

#[allow(clippy::too_many_arguments)]
fn draw_scoreboard_font_image(
    surface: &mut Surface,
    image: &ImageData,
    x: i32,
    y: i32,
    width: i32,
    height: i32,
    modulation: [u8; 4],
    shear: f32,
    gamma: Option<&GammaRamp>,
) {
    if modulation == [255, 255, 255, 255] {
        if shear == 0.0 {
            draw_facet_stretch(
                surface,
                image,
                (0.0, 0.0, image.width() as f32, image.height() as f32),
                (x as f32, y as f32, width as f32, height as f32),
                gamma,
            );
        } else {
            draw_sheared_font_image(surface, image, x, y, width, height, shear, gamma);
        }
    } else {
        let tinted = modulate_font_image(image, modulation);
        if shear == 0.0 {
            draw_facet_stretch(
                surface,
                &tinted,
                (0.0, 0.0, tinted.width() as f32, tinted.height() as f32),
                (x as f32, y as f32, width as f32, height as f32),
                gamma,
            );
        } else {
            draw_sheared_font_image(surface, &tinted, x, y, width, height, shear, gamma);
        }
    }
}

/// Rasterize one italic glyph through the same destination-space transform as
/// `CStdFont::DrawText`: the source rectangle is placed at `(x, y)`, then the
/// x/y shear is centered on that glyph's own destination rectangle
/// (`src/StdFont.cpp:906-925`). The pen advance intentionally remains the
/// unsheared facet width.
#[allow(clippy::too_many_arguments)]
fn draw_sheared_glyph(
    surface: &mut Surface,
    glyph: &GlyphCell,
    height: i32,
    x: i32,
    y: i32,
    modulation: [u8; 4],
    shear: f32,
    gamma: Option<&GammaRamp>,
) {
    let width = glyph.width;
    let Some((x0, y0, x1, y1)) = sheared_raster_bounds(surface, x, y, width, height, shear) else {
        return;
    };
    for target_y in y0..y1 {
        for target_x in x0..x1 {
            let Some((sample_x, sample_y)) = inverse_sheared_sample(
                target_x, target_y, x, y, width, height, width, height, shear,
            ) else {
                continue;
            };
            let sample = bilinear_sample_glyph(glyph, height, sample_x, sample_y);
            if sample[3] <= 0.0 {
                continue;
            }
            let destination = surface
                .get_pixel(target_x as u32, target_y as u32)
                .unwrap_or_default();
            let source_alpha = markup_sample_alpha(sample[3], modulation[3]);
            let alpha = (source_alpha / 255.0).clamp(0.0, 1.0);
            let source_channel = |channel: GammaChannel, value: f32, tint: u8| {
                let value = value * f32::from(tint) / 255.0;
                gamma.map_or(value, |ramp| ramp.sample_channel_float(channel, value))
            };
            let blend = |source: f32, destination: u8| {
                store_sample(source * alpha + f32::from(destination) * (1.0 - alpha))
            };
            let _ = surface.set_pixel(
                target_x as u32,
                target_y as u32,
                Color::new(
                    blend(
                        source_channel(GammaChannel::Red, sample[0], modulation[0]),
                        destination.r,
                    ),
                    blend(
                        source_channel(GammaChannel::Green, sample[1], modulation[1]),
                        destination.g,
                    ),
                    blend(
                        source_channel(GammaChannel::Blue, sample[2], modulation[2]),
                        destination.b,
                    ),
                    blend(source_alpha, destination.a),
                ),
            );
        }
    }
}

/// Italic custom font images use the identical per-facet centered transform as
/// glyphs. Image scaling happens before the markup transform in C++ (`w2/h2`
/// are established at src/StdFont.cpp:868-885, then transformed at 906-925).
#[allow(clippy::too_many_arguments)]
fn draw_sheared_font_image(
    surface: &mut Surface,
    image: &ImageData,
    x: i32,
    y: i32,
    width: i32,
    height: i32,
    shear: f32,
    gamma: Option<&GammaRamp>,
) {
    let Some((x0, y0, x1, y1)) = sheared_raster_bounds(surface, x, y, width, height, shear) else {
        return;
    };
    for target_y in y0..y1 {
        for target_x in x0..x1 {
            let Some((sample_x, sample_y)) = inverse_sheared_sample(
                target_x,
                target_y,
                x,
                y,
                width,
                height,
                image.width() as i32,
                image.height() as i32,
                shear,
            ) else {
                continue;
            };
            let sample = bilinear_sample_image(image, sample_x, sample_y);
            if sample[3] <= 0.0 {
                continue;
            }
            let destination = surface
                .get_pixel(target_x as u32, target_y as u32)
                .unwrap_or_default();
            let alpha = (sample[3] / 255.0).clamp(0.0, 1.0);
            let encode = |value: f32| {
                gamma.map_or_else(
                    || value.round().clamp(0.0, 255.0),
                    |ramp| f32::from(ramp.encode_float(value)),
                )
            };
            let blend = |source: f32, destination: u8| {
                store_sample(encode(source) * alpha + f32::from(destination) * (1.0 - alpha))
            };
            let _ = surface.set_pixel(
                target_x as u32,
                target_y as u32,
                Color::new(
                    blend(sample[0], destination.r),
                    blend(sample[1], destination.g),
                    blend(sample[2], destination.b),
                    blend(sample[3], destination.a),
                ),
            );
        }
    }
}

fn sheared_raster_bounds(
    surface: &Surface,
    x: i32,
    y: i32,
    width: i32,
    height: i32,
    shear: f32,
) -> Option<(i32, i32, i32, i32)> {
    if width <= 0 || height <= 0 || surface.width() == 0 || surface.height() == 0 {
        return None;
    }
    let half_height = height as f32 / 2.0;
    let top_shift = shear * -half_height;
    let bottom_shift = shear * half_height;
    let min_shift = top_shift.min(bottom_shift);
    let max_shift = top_shift.max(bottom_shift);
    // A destination pixel participates when its center lies inside the
    // transformed quad. This is the same half-pixel convention used by the
    // classic stretch blitter in `classic_gui`.
    let x0 = ((x as f32 + min_shift - 0.5).ceil() as i32).max(0);
    let x1 =
        ((x as f32 + width as f32 + max_shift - 0.5).ceil() as i32).min(surface.width() as i32);
    let y0 = y.max(0);
    let y1 = y.saturating_add(height).min(surface.height() as i32);
    (x0 < x1 && y0 < y1).then_some((x0, y0, x1, y1))
}

#[allow(clippy::too_many_arguments)]
fn inverse_sheared_sample(
    target_x: i32,
    target_y: i32,
    x: i32,
    y: i32,
    destination_width: i32,
    destination_height: i32,
    source_width: i32,
    source_height: i32,
    shear: f32,
) -> Option<(f32, f32)> {
    if destination_width <= 0 || destination_height <= 0 || source_width <= 0 || source_height <= 0
    {
        return None;
    }
    let pixel_x = target_x as f32 + 0.5;
    let pixel_y = target_y as f32 + 0.5;
    let center_y = y as f32 + destination_height as f32 / 2.0;
    // Forward: x' = x + shear * (y - center_y). Undo that term to
    // inverse-map the destination pixel center into the unsheared quad.
    let unsheared_x = pixel_x - shear * (pixel_y - center_y);
    let local_x = unsheared_x - x as f32;
    let local_y = pixel_y - y as f32;
    if local_x < 0.0
        || local_y < 0.0
        || local_x >= destination_width as f32
        || local_y >= destination_height as f32
    {
        return None;
    }
    Some((
        local_x * source_width as f32 / destination_width as f32 - 0.5,
        local_y * source_height as f32 / destination_height as f32 - 0.5,
    ))
}

fn bilinear_sample_glyph(glyph: &GlyphCell, height: i32, sample_x: f32, sample_y: f32) -> [f32; 4] {
    bilinear_sample(sample_x, sample_y, |x, y| {
        if x < 0 || y < 0 || x >= glyph.width || y >= height {
            return [0.0; 4];
        }
        glyph
            .pixels
            .get((y as usize).saturating_mul(glyph.width as usize) + x as usize)
            .map(|pixel| {
                [
                    f32::from(pixel.r),
                    f32::from(pixel.g),
                    f32::from(pixel.b),
                    f32::from(pixel.a),
                ]
            })
            .unwrap_or([0.0; 4])
    })
}

fn bilinear_sample_image(image: &ImageData, sample_x: f32, sample_y: f32) -> [f32; 4] {
    bilinear_sample(sample_x, sample_y, |x, y| {
        if x < 0 || y < 0 || x >= image.width() as i32 || y >= image.height() as i32 {
            return [0.0; 4];
        }
        let index = ((y as u32 * image.width() + x as u32) * 4) as usize;
        image
            .pixels()
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
    })
}

fn bilinear_sample(sample_x: f32, sample_y: f32, texel: impl Fn(i32, i32) -> [f32; 4]) -> [f32; 4] {
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

fn store_sample(value: f32) -> u8 {
    value.round().clamp(0.0, 255.0) as u8
}

fn modulate_font_image(image: &ImageData, rgba: [u8; 4]) -> ImageData {
    let pixels = image
        .pixels()
        .chunks_exact(4)
        .flat_map(|pixel| {
            [
                ((u16::from(pixel[0]) * u16::from(rgba[0])) / 255) as u8,
                ((u16::from(pixel[1]) * u16::from(rgba[1])) / 255) as u8,
                ((u16::from(pixel[2]) * u16::from(rgba[2])) / 255) as u8,
                pixel[3].saturating_sub(255 - rgba[3]),
            ]
        })
        .collect();
    ImageData::new(image.width(), image.height(), pixels)
}

fn scaled_font_image_width(font: &ClonkFont, image: &ImageData) -> i32 {
    if image.height() == 0 {
        return 0;
    }
    ((i64::from(image.width()) * i64::from(font.cell_height)) / i64::from(image.height()))
        .try_into()
        .unwrap_or(i32::MAX)
}

fn font_image<'a>(images: &'a HashMap<String, ImageData>, spec: &str) -> Option<&'a ImageData> {
    let key = truncate_utf8_bytes(spec, 100);
    images
        .get(key)
        .filter(|image| image.width() > 0 && image.height() > 0)
}

fn truncate_utf8_bytes(text: &str, max: usize) -> &str {
    let mut end = text.len().min(max);
    while !text.is_char_boundary(end) {
        end -= 1;
    }
    &text[..end]
}

fn inline_image_token(text: &str) -> Option<(&str, usize)> {
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

fn collect_inline_image_specs(text: &str, specs: &mut Vec<String>) {
    let mut rest = text;
    while !rest.is_empty() {
        if let Some((spec, advance)) = inline_image_token(rest) {
            let spec = truncate_utf8_bytes(spec, 100);
            if !specs.iter().any(|old| old == spec) {
                specs.push(spec.to_string());
            }
            rest = &rest[advance..];
            continue;
        }
        let character = rest.chars().next().expect("non-empty text");
        rest = &rest[character.len_utf8()..];
    }
}

fn markup_tag_parts(text: &str) -> Option<(usize, &str, Option<&str>)> {
    let inner = text.strip_prefix('<')?;
    let close = inner.find('>')?;
    let full = &inner[..close];
    let mut tag_len = full.len().min(49);
    while !full.is_char_boundary(tag_len) {
        tag_len -= 1;
    }
    let tag = &full[..tag_len];
    let mut advance = (tag_len + 2).min(text.len());
    while !text.is_char_boundary(advance) {
        advance += 1;
    }
    let (name, parameters) = match tag.find(' ') {
        Some(index) => (&tag[..index], Some(&tag[index + 1..])),
        None => (tag, None),
    };
    Some((advance, name, parameters))
}

fn skip_markup_tag(text: &str) -> Option<usize> {
    let (advance, name, parameters) = markup_tag_parts(text)?;
    let valid = if name.starts_with('/') || name == "i" {
        parameters.is_none()
    } else if name == "c" {
        parameters.is_some_and(|parameters| parameters.len() <= 8)
    } else {
        false
    };
    valid.then_some(advance)
}

fn read_markup_tag(text: &str, stack: &mut Vec<ScoreboardMarkupTag>) -> Option<usize> {
    let (advance, name, parameters) = markup_tag_parts(text)?;
    let valid = if let Some(closing) = name.strip_prefix('/') {
        parameters.is_none() && stack.last().is_some_and(|tag| tag.name() == closing) && {
            stack.pop();
            true
        }
    } else if name == "i" {
        parameters.is_none() && {
            stack.push(ScoreboardMarkupTag::Italic);
            true
        }
    } else if name == "c" {
        parameters
            .and_then(parse_color_tag)
            .map(|color| stack.push(ScoreboardMarkupTag::TextColor(color)))
            .is_some()
    } else {
        false
    };
    valid.then_some(advance)
}

fn parse_color_tag(parameters: &str) -> Option<u32> {
    let length = parameters.len();
    (length <= 8)
        .then_some(())
        .and_then(|()| {
            parameters
                .bytes()
                .enumerate()
                .try_fold(0_u32, |color, (index, byte)| {
                    let digit = match byte {
                        b'0'..=b'9' => byte - b'0',
                        b'a'..=b'f' => byte - b'a' + 10,
                        _ => return None,
                    };
                    Some(color | ((digit as u32) << ((length - index - 1) * 4)))
                })
        })
        .map(|color| {
            if length <= 6 {
                color | 0xff00_0000
            } else {
                color
            }
        })
        .map(|color| (color & 0x00ff_ffff) | ((255 - (color >> 24)) << 24))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_support::{endeavour_font_set, load_graphics_png};

    fn scoreboard(rows: serde_json::Value) -> ScoreboardState {
        serde_json::from_value(serde_json::json!({
            "rows": rows,
            "show_count": 1
        }))
        .expect("scoreboard fixture")
    }

    fn preferred() -> IntRect {
        IntRect {
            x: 40,
            y: 20,
            w: 560,
            h: 400,
        }
    }

    fn solid_test_font() -> ClonkFont {
        let mut font = ClonkFont::new(9);
        font.add_glyph(
            'X',
            GlyphCell {
                width: 6,
                pixels: vec![Color::opaque(255, 255, 255); 6 * 10],
            },
        );
        font
    }

    fn changed_row_span(surface: &Surface, y: u32, background: Color) -> Option<(u32, u32)> {
        let changed = (0..surface.width())
            .filter(|x| surface.get_pixel(*x, y) != Some(background))
            .collect::<Vec<_>>();
        changed.first().copied().zip(changed.last().copied())
    }

    #[test]
    fn layout_matches_cpp_column_rows_and_top_right_placement() {
        let fonts = endeavour_font_set();
        let caption = load_graphics_png("GUICaption.png");
        let icons = load_graphics_png("GUIIcons.png");
        let resources =
            ScoreboardResources::new(&caption, &icons, fonts.as_ref()).expect("resources");
        let board = scoreboard(serde_json::json!([
            [
                {"text":"Scores","value":-1},
                {"text":"Points","value":1},
                {"text":"Time","value":2}
            ],
            [
                {"text":"Alice","value":7},
                {"text":"42","value":42},
                {"text":"01:23","value":83}
            ],
            [
                {"text":"Bob","value":8},
                {"text":"7","value":7},
                {"text":"00:09","value":9}
            ]
        ]));

        let layout = scoreboard_layout(preferred(), &board, &resources).expect("layout");
        let expected_columns = (0..3)
            .map(|column| {
                (0..3)
                    .filter(|row| *row != 0 || column != 0)
                    .filter_map(|row| board.cell(row, column).and_then(|cell| cell.text()))
                    .map(|text| fonts.text.measure(text, true).0 + X_INDENT)
                    .fold(X_INDENT, i32::max)
            })
            .collect::<Vec<_>>();
        assert_eq!(layout.column_widths, expected_columns);
        assert_eq!(layout.row_height, fonts.text.line_height + Y_INDENT);
        assert_eq!(
            layout.client.h,
            2 * Y_MARGIN + 3 * (fonts.text.line_height + Y_INDENT)
        );
        assert_eq!(
            layout.bounds.x + layout.bounds.w,
            preferred().x + preferred().w - PLACEMENT_RIGHT_INSET
        );
        assert_eq!(layout.bounds.y, preferred().y + PLACEMENT_TOP_INSET);
        assert_eq!(
            layout.caption.expect("title").h,
            23.max(fonts.text.line_height)
        );

        let first = layout.cell_text_anchor(1, 0).expect("first column");
        let numeric = layout.cell_text_anchor(1, 1).expect("numeric column");
        assert_eq!(first.align, TextAlign::Left);
        assert_eq!(numeric.align, TextAlign::Center);
        assert_eq!(
            numeric.x,
            first.x + layout.column_widths[0] + layout.column_widths[1] / 2
        );
    }

    #[test]
    fn null_empty_and_nonempty_titles_retain_distinct_cpp_semantics() {
        let fonts = endeavour_font_set();
        let caption = load_graphics_png("GUICaption.png");
        let icons = load_graphics_png("GUIIcons.png");
        let resources =
            ScoreboardResources::new(&caption, &icons, fonts.as_ref()).expect("resources");
        let null_title = scoreboard(serde_json::json!([[{"value":-1}]]));
        let empty_title = scoreboard(serde_json::json!([[{"text":"","value":-1}]]));
        let title = scoreboard(serde_json::json!([[{"text":"Scores","value":-1}]]));

        let null = scoreboard_layout(preferred(), &null_title, &resources).expect("null title");
        let empty = scoreboard_layout(preferred(), &empty_title, &resources).expect("empty title");
        let titled = scoreboard_layout(preferred(), &title, &resources).expect("title");
        assert!(null.caption.is_none());
        assert!(empty.caption.is_none());
        assert!(titled.caption.is_some());
        assert!(empty.title_icon.is_none());
        assert!(empty.close_button.is_none());
        assert_eq!(null.bounds.w, 2 * X_MARGIN + X_INDENT);
        assert_eq!(empty.bounds.w, TITLE_EXTRA_WIDTH);
        let stale_margin = MIN_WOOD_BAR_HEIGHT.max(fonts.text.line_height);
        assert_eq!(empty.bounds.h - null.bounds.h, stale_margin);
        assert_eq!(null.client.y, null.bounds.y);
        assert_eq!(empty.client.y - empty.bounds.y, stale_margin);
        assert_eq!(titled.bounds.h - titled.client.h, stale_margin);
    }

    #[test]
    fn live_matrix_growth_remeasures_and_keeps_the_right_edge_fixed() {
        let fonts = endeavour_font_set();
        let caption = load_graphics_png("GUICaption.png");
        let icons = load_graphics_png("GUIIcons.png");
        let resources =
            ScoreboardResources::new(&caption, &icons, fonts.as_ref()).expect("resources");
        let short = scoreboard(serde_json::json!([
            [{"text":"Scores","value":-1},{"text":"P","value":1}],
            [{"text":"A","value":2},{"text":"1","value":1}]
        ]));
        let long = scoreboard(serde_json::json!([
            [{"text":"Scores","value":-1},{"text":"Points","value":1}],
            [{"text":"A very long player name","value":2},{"text":"100000","value":1}],
            [{"text":"Second","value":3},{"text":"2","value":2}]
        ]));
        let before = scoreboard_layout(preferred(), &short, &resources).expect("short");
        let after = scoreboard_layout(preferred(), &long, &resources).expect("long");
        assert!(after.bounds.w > before.bounds.w);
        assert!(after.bounds.h > before.bounds.h);
        assert_eq!(
            before.bounds.x + before.bounds.w,
            after.bounds.x + after.bounds.w
        );
    }

    #[test]
    fn markup_and_custom_font_images_participate_in_metrics_and_discovery() {
        let fonts = endeavour_font_set();
        let caption = load_graphics_png("GUICaption.png");
        let icons = load_graphics_png("GUIIcons.png");
        let image = ImageData::new(10, 5, vec![255; 10 * 5 * 4]);
        let images = HashMap::from([("TEST:2".to_string(), image)]);
        let resources = ScoreboardResources::new(&caption, &icons, fonts.as_ref())
            .expect("resources")
            .with_font_images(&images);
        let board = scoreboard(serde_json::json!([
            [{"text":"Scores","value":-1},{"text":"Value","value":1}],
            [{"text":"<c ff0000>A</c>{{TEST:2}}B","value":2},{"text":"1","value":1}],
            [{"text":"{{TEST:2}}","value":3},{"text":"2","value":2}]
        ]));
        assert_eq!(scoreboard_inline_image_specs(&board), vec!["TEST:2"]);
        let layout = scoreboard_layout(preferred(), &board, &resources).expect("layout");
        let with_image = scoreboard_text_width("<c ff0000>A</c>{{TEST:2}}B", &fonts.text, &images);
        let without_image = scoreboard_text_width("<c ff0000>A</c>B", &fonts.text, &images);
        assert!(with_image > without_image);
        assert!(layout.column_widths[0] >= with_image + X_INDENT);
    }

    #[test]
    fn native_scoreboard_bytes_decode_at_layout_and_render_boundaries() {
        let fonts = endeavour_font_set();
        let caption = load_graphics_png("GUICaption.png");
        let icons = load_graphics_png("GUIIcons.png");
        let resources =
            ScoreboardResources::new(&caption, &icons, fonts.as_ref()).expect("resources");
        let raw = scoreboard(serde_json::json!([
            [{"text":{"c4_bytes":[83,99,246,114,101,115]},"value":-1},{"text":"Points","value":1}],
            [{"text":{"c4_bytes":[65,110,100,114,233]},"value":2},{"text":"7","value":7}]
        ]));
        let presented = scoreboard(serde_json::json!([
            [{"text":"Sc\u{f6}res","value":-1},{"text":"Points","value":1}],
            [{"text":"Andr\u{e9}","value":2},{"text":"7","value":7}]
        ]));

        assert_eq!(
            scoreboard_layout(preferred(), &raw, &resources).expect("raw layout"),
            scoreboard_layout(preferred(), &presented, &resources).expect("presented layout")
        );
        let render = |board: &ScoreboardState| {
            let mut surface = Surface::new(640, 480, PixelFormat::Rgba8888);
            render_scoreboard(&mut surface, preferred(), board, &resources, None)
                .expect("scoreboard renders");
            surface.snapshot()
        };
        assert_eq!(render(&raw), render(&presented));
        assert_eq!(
            lc_script::c4_string_bytes(
                raw.cell(1, 0)
                    .and_then(|cell| cell.text())
                    .expect("raw player cell")
            ),
            b"Andr\xe9"
        );
    }

    #[test]
    fn layout_measurement_keeps_cpp_separator_spacing_quirk() {
        let font = solid_test_font();
        let image = ImageData::new(6, 10, vec![255; 6 * 10 * 4]);
        let images = HashMap::from([("TEST".to_string(), image)]);

        assert_eq!(scoreboard_text_width("XX|X", &font, &images), 10);
        assert_eq!(scoreboard_text_width("XX|X", &font, &images), font.measure("XX|X", true).0);
        assert_eq!(scoreboard_line_width("XX", &font, &images), 11);
        assert_eq!(scoreboard_text_width("{{TEST}}|", &font, &images), 5);
        assert_eq!(scoreboard_line_width("{{TEST}}", &font, &images), 6);
    }

    #[test]
    fn unclosed_shipped_style_italic_shears_each_glyph_and_persists_across_lines() {
        let font = solid_test_font();
        let images = HashMap::new();
        let background = Color::opaque(3, 5, 7);
        let mut plain = Surface::new(64, 40, PixelFormat::Rgba8888);
        let mut italic = Surface::new(64, 40, PixelFormat::Rgba8888);
        plain.fill(background);
        italic.fill(background);

        draw_scoreboard_text(
            &mut plain,
            20,
            6,
            "X|X",
            &font,
            &images,
            TextAlign::Left,
            None,
        );
        // CaptureTheFlag.c4s deliberately emits `Format("<i>%s", name)`
        // without a closing tag. CStdDDraw keeps one CMarkup stack for every
        // `|`-split TextOut line, so that open italic remains in force.
        draw_scoreboard_text(
            &mut italic,
            20,
            6,
            "<i>X|X",
            &font,
            &images,
            TextAlign::Left,
            None,
        );

        assert_eq!(
            scoreboard_text_width("<i>X", &font, &images),
            scoreboard_text_width("X", &font, &images),
            "italic changes pixels, never the pen advance"
        );
        // The glyph cell is one pixel taller than line_height, so adjacent
        // lines overlap at y=15. Sample just inside each line at the seam.
        for (top_y, bottom_y) in [(6_u32, 14_u32), (16, 24)] {
            let plain_top = changed_row_span(&plain, top_y, background).expect("plain top row");
            let italic_top = changed_row_span(&italic, top_y, background).expect("italic top row");
            let plain_bottom =
                changed_row_span(&plain, bottom_y, background).expect("plain bottom row");
            let italic_bottom =
                changed_row_span(&italic, bottom_y, background).expect("italic bottom row");
            assert!(italic_top.0 > plain_top.0, "top edge shears right");
            assert!(italic_bottom.0 < plain_bottom.0, "bottom edge shears left");
        }
    }

    #[test]
    fn italic_stack_and_inverse_transform_pin_the_cpp_coefficient() {
        let mut stack = Vec::new();
        assert_eq!(read_markup_tag("<i>", &mut stack), Some(3));
        assert_eq!(markup_shear(&stack), -0.3);
        assert_eq!(read_markup_tag("<i>", &mut stack), Some(3));
        assert_eq!(markup_shear(&stack), -0.6);
        assert_eq!(read_markup_tag("</i>", &mut stack), Some(4));
        assert_eq!(markup_shear(&stack), -0.3);
        assert_eq!(read_markup_tag("</i>", &mut stack), Some(4));
        assert_eq!(markup_shear(&stack), 0.0);

        let surface = Surface::new(100, 40, PixelFormat::Rgba8888);
        assert_eq!(
            sheared_raster_bounds(&surface, 20, 5, 6, 10, -0.3),
            Some((18, 5, 27, 15))
        );
        let (sample_x, sample_y) =
            inverse_sheared_sample(22, 5, 20, 5, 6, 10, 6, 10, -0.3)
                .expect("sample inside transformed quad");
        assert!((sample_x - 0.65).abs() < 0.0001);
        assert!(sample_y.abs() < 0.0001);
    }

    #[test]
    fn italic_shears_custom_font_images_without_losing_color_markup() {
        let font = solid_test_font();
        let image = ImageData::new(6, 10, vec![255; 6 * 10 * 4]);
        let images = HashMap::from([("TEST".to_string(), image)]);
        let background = Color::opaque(3, 5, 7);
        let mut plain = Surface::new(64, 30, PixelFormat::Rgba8888);
        let mut italic = Surface::new(64, 30, PixelFormat::Rgba8888);
        plain.fill(background);
        italic.fill(background);

        draw_scoreboard_text(
            &mut plain,
            20,
            6,
            "<c ff0000>{{TEST}}",
            &font,
            &images,
            TextAlign::Left,
            None,
        );
        draw_scoreboard_text(
            &mut italic,
            20,
            6,
            "<c ff0000><i>{{TEST}}",
            &font,
            &images,
            TextAlign::Left,
            None,
        );

        let plain_top = changed_row_span(&plain, 6, background).expect("plain image top");
        let italic_top = changed_row_span(&italic, 6, background).expect("italic image top");
        let plain_bottom = changed_row_span(&plain, 15, background).expect("plain image bottom");
        let italic_bottom = changed_row_span(&italic, 15, background).expect("italic image bottom");
        assert!(italic_top.0 > plain_top.0);
        assert!(italic_bottom.0 < plain_bottom.0);
        let tinted = italic
            .get_pixel(italic_top.0, 6)
            .expect("tinted sheared image pixel");
        assert!(tinted.r > tinted.g && tinted.r > tinted.b);
    }

    #[test]
    fn color_markup_preserves_alpha_and_exact_base_white() {
        let font = solid_test_font();
        let images = HashMap::new();
        let mut translucent = Surface::new(32, 16, PixelFormat::Rgba8888);
        draw_scoreboard_text(
            &mut translucent,
            0,
            0,
            "<c 80ff0000>X",
            &font,
            &images,
            TextAlign::Left,
            None,
        );
        let red = translucent.get_pixel(0, 0).expect("translucent red glyph");
        assert!(red.r > red.g && red.r > red.b);
        assert!((63..=64).contains(&red.a));
        assert_eq!(markup_sample_alpha(128.0, 128), 1.0);

        let mut white = Surface::new(32, 16, PixelFormat::Rgba8888);
        draw_scoreboard_text(
            &mut white,
            0,
            0,
            "<c ffffff>X",
            &font,
            &images,
            TextAlign::Left,
            None,
        );
        assert_eq!(white.get_pixel(0, 0), Some(Color::opaque(255, 255, 255)));
    }

    #[test]
    fn multiline_cell_overflow_is_not_clipped_to_the_client() {
        let fonts = endeavour_font_set();
        let caption = load_graphics_png("GUICaption.png");
        let icons = load_graphics_png("GUIIcons.png");
        let resources =
            ScoreboardResources::new(&caption, &icons, fonts.as_ref()).expect("resources");
        let board = scoreboard(serde_json::json!([[
            {"text":"Scores","value":-1},
            {"text":"X|X","value":1}
        ]]));
        let background = Color::opaque(3, 5, 7);
        let mut surface = Surface::new(640, 480, PixelFormat::Rgba8888);
        surface.fill(background);
        let layout = scoreboard_layout(preferred(), &board, &resources).expect("layout");
        render_scoreboard(&mut surface, preferred(), &board, &resources, None)
            .expect("render scoreboard");

        let first_outside_y = (layout.bounds.y + layout.bounds.h).max(0) as u32;
        assert!(
            (first_outside_y..surface.height()).any(|y| {
                (0..surface.width())
                    .any(|x| surface.get_pixel(x, y) != Some(background))
            }),
            "C4ScoreboardDlg::DrawElement paints before Window installs its client clip"
        );
    }

    #[test]
    fn renderer_draws_dialog_caption_icons_and_cells() {
        let fonts = endeavour_font_set();
        let caption = load_graphics_png("GUICaption.png");
        let icons = load_graphics_png("GUIIcons.png");
        let resources =
            ScoreboardResources::new(&caption, &icons, fonts.as_ref()).expect("resources");
        let board = scoreboard(serde_json::json!([
            [{"text":"Scores","value":-1},{"text":"Points","value":1}],
            [{"text":"Alice","value":2},{"text":"42","value":42}]
        ]));
        let mut surface = Surface::new(640, 480, PixelFormat::Rgba8888);
        surface.fill(Color::opaque(3, 5, 7));
        let layout = scoreboard_layout(preferred(), &board, &resources).expect("layout");
        render_scoreboard(&mut surface, preferred(), &board, &resources, None)
            .expect("render scoreboard");

        let changed = |rect: IntRect| {
            (rect.y.max(0)..(rect.y + rect.h).min(surface.height() as i32)).any(|y| {
                (rect.x.max(0)..(rect.x + rect.w).min(surface.width() as i32))
                    .any(|x| surface.get_pixel(x as u32, y as u32) != Some(Color::opaque(3, 5, 7)))
            })
        };
        assert!(changed(layout.bounds));
        assert!(changed(layout.title_icon.expect("player icon")));
        assert!(changed(layout.close_button.expect("close icon")));
    }

    #[test]
    fn body_and_caption_form_distinct_native_capture_phases() {
        let fonts = endeavour_font_set();
        let caption = load_graphics_png("GUICaption.png");
        let icons = load_graphics_png("GUIIcons.png");
        let resources =
            ScoreboardResources::new(&caption, &icons, fonts.as_ref()).expect("resources");
        let board = scoreboard(serde_json::json!([
            [{"text":"TITLE","value":-1},{"text":"","value":1}],
            [{"text":"BODY","value":2},{"text":"","value":3}]
        ]));

        let mut captured = Surface::new(640, 480, PixelFormat::Rgba8888);
        captured.begin_clonk_text_capture();
        render_scoreboard_body(&mut captured, preferred(), &board, &resources, None).expect("body");
        let body_commands = captured.take_clonk_text_capture();
        let body_text = body_commands
            .iter()
            .map(|command| command.text.as_str())
            .collect::<String>();
        assert!(body_text.contains("BODY"));
        assert!(!body_text.contains("TITLE"));

        captured.begin_clonk_text_capture();
        render_scoreboard_caption(&mut captured, preferred(), &board, &resources, None)
            .expect("caption");
        let caption_commands = captured.take_clonk_text_capture();
        assert_eq!(
            caption_commands
                .iter()
                .map(|command| command.text.as_str())
                .collect::<String>(),
            "TITLE"
        );
        let layout = scoreboard_layout(preferred(), &board, &resources).expect("layout");
        let caption_bounds = layout.caption.expect("caption");
        let expected_clip = lc_graphics::Rect::new(
            caption_bounds.x + caption_bounds.h,
            caption_bounds.y,
            (caption_bounds.w - caption_bounds.h - CAPTION_RIGHT_INDENT + 1) as u32,
            (caption_bounds.h + 1) as u32,
        );
        assert!(caption_commands
            .iter()
            .all(|command| command.clip == Some(expected_clip)));

        let mut combined = Surface::new(640, 480, PixelFormat::Rgba8888);
        let mut staged = Surface::new(640, 480, PixelFormat::Rgba8888);
        render_scoreboard(&mut combined, preferred(), &board, &resources, None).expect("combined");
        render_scoreboard_body(&mut staged, preferred(), &board, &resources, None)
            .expect("staged body");
        render_scoreboard_caption(&mut staged, preferred(), &board, &resources, None)
            .expect("staged caption");
        assert_eq!(combined.pixels(), staged.pixels());
    }

    #[test]
    fn italic_translucent_scoreboard_glyphs_still_enter_native_capture() {
        let fonts = endeavour_font_set();
        let caption = load_graphics_png("GUICaption.png");
        let icons = load_graphics_png("GUIIcons.png");
        let resources =
            ScoreboardResources::new(&caption, &icons, fonts.as_ref()).expect("resources");
        let board = scoreboard(serde_json::json!([
            [{"text":"TITLE","value":-1}],
            [{"text":"<i><c 80ff0000>X</c></i>","value":1}]
        ]));

        let mut surface = Surface::new(640, 480, PixelFormat::Rgba8888);
        surface.begin_clonk_text_capture();
        render_scoreboard_body(&mut surface, preferred(), &board, &resources, None)
            .expect("body");
        let commands = surface.take_clonk_text_capture();
        let glyph = commands
            .iter()
            .find(|command| command.text == "X")
            .expect("resolved italic glyph remains a semantic native command");
        assert_eq!(glyph.color, [254, 0, 0, 128]);
        assert!(!glyph.markup);
    }

    #[test]
    fn renderer_keeps_wooden_title_markup_enabled() {
        let fonts = endeavour_font_set();
        let caption = load_graphics_png("GUICaption.png");
        let icons = load_graphics_png("GUIIcons.png");
        let resources =
            ScoreboardResources::new(&caption, &icons, fonts.as_ref()).expect("resources");
        let tagged = scoreboard(serde_json::json!([[
            {"text":"<c ff0000>X</c>","value":-1}
        ]]));
        let plain = scoreboard(serde_json::json!([[{"text":"X","value":-1}]]));
        let mut tagged_surface = Surface::new(640, 480, PixelFormat::Rgba8888);
        let mut plain_surface = Surface::new(640, 480, PixelFormat::Rgba8888);
        let background = Color::opaque(3, 5, 7);
        tagged_surface.fill(background);
        plain_surface.fill(background);
        render_scoreboard(&mut tagged_surface, preferred(), &tagged, &resources, None)
            .expect("render tagged title");
        render_scoreboard(&mut plain_surface, preferred(), &plain, &resources, None)
            .expect("render plain title");
        let layout = scoreboard_layout(preferred(), &tagged, &resources).expect("layout");
        let caption = layout.caption.expect("caption");
        assert!((caption.y..caption.y + caption.h).any(|y| {
            (caption.x..caption.x + caption.w).any(|x| {
                let tagged = tagged_surface
                    .get_pixel(x as u32, y as u32)
                    .unwrap_or_default();
                let plain = plain_surface
                    .get_pixel(x as u32, y as u32)
                    .unwrap_or_default();
                tagged != plain
                    && tagged.r.saturating_add(2) >= plain.r
                    && tagged.g.saturating_add(20) < plain.g
                    && tagged.b.saturating_add(20) < plain.b
            })
        }));
    }

    #[test]
    fn malformed_required_sheets_and_empty_matrices_fail_visibly() {
        let fonts = endeavour_font_set();
        let caption = load_graphics_png("GUICaption.png");
        let icons = load_graphics_png("GUIIcons.png");
        let malformed = ImageData::new(1, 1, vec![0, 0, 0, 0]);
        assert!(ScoreboardResources::new(&malformed, &icons, fonts.as_ref()).is_err());
        assert!(ScoreboardResources::new(&caption, &malformed, fonts.as_ref()).is_err());

        let resources =
            ScoreboardResources::new(&caption, &icons, fonts.as_ref()).expect("resources");
        assert!(scoreboard_layout(preferred(), &ScoreboardState::default(), &resources).is_err());
    }

    #[test]
    fn unresolved_or_empty_custom_images_fail_before_layout() {
        let fonts = endeavour_font_set();
        let caption = load_graphics_png("GUICaption.png");
        let icons = load_graphics_png("GUIIcons.png");
        let board = scoreboard(serde_json::json!([
            [{"text":"Scores","value":-1}],
            [{"text":"{{MISSING}}","value":1}]
        ]));
        let resources =
            ScoreboardResources::new(&caption, &icons, fonts.as_ref()).expect("resources");
        let error = scoreboard_layout(preferred(), &board, &resources)
            .expect_err("unresolved image must fail");
        assert!(error.to_string().contains("{{MISSING}}"));

        let empty = ImageData::new(0, 0, Vec::new());
        let images = HashMap::from([("MISSING".to_string(), empty)]);
        let resources = resources.with_font_images(&images);
        assert!(scoreboard_layout(preferred(), &board, &resources).is_err());
    }
}
