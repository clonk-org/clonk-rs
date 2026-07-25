//! The in-game player HUD, mirroring the C++ fullscreen overlay:
//!
//! - Upper board: `C4UpperBoard::Draw` (src/C4UpperBoard.cpp:46-96) — the
//!   tiled wooden bar with the logo centered, the scenario title left and
//!   the elapsed game time right.
//! - Player fixed items: `C4Viewport::DrawPlayerInfo`
//!   (src/C4Viewport.cpp:1281-1326) — wealth / score / crew value displays
//!   right-aligned at the viewport top.
//! - Cursor info: `C4Viewport::DrawCursorInfo` (src/C4Viewport.cpp:884-965)
//!   with `C4ObjectInfo::Draw` (src/C4ObjectInfo.cpp:302-371) — portrait,
//!   rank symbol, crew name and the vertical energy bar.
//! - Player startup hint: `C4Viewport::DrawPlayerStartup`
//!   (src/C4Viewport.cpp:1446-1476) — keyboard/gamepad graphic, optional
//!   mouse symbol, and player name in the player color.
//! - Message board: `C4MessageBoard::Draw` (src/C4MessageBoard.cpp:243-306)
//!   — hidden, fading single-line, and continuous multi-line layouts over the
//!   tiled background strip at the screen bottom.

#[cfg(test)]
use crate::fill_rect;
use crate::{
    draw_image_bilinear, draw_image_bilinear_additive, draw_image_bilinear_owner, draw_image_strip,
    ClonkFontSet, HudGraphics, ImageData, InventoryOverlay, MessageBoardMode, MessageBoardOverlay,
};
use clonk_graphics::{
    clonk_font::TextAlign, Color, GammaRamp, Rect as SurfaceRect, Surface, SurfaceDrawTarget,
    TextFont,
};
use std::{cell::RefCell, collections::HashMap};

/// `C4UpperBoardHeight` (src/C4Constants.h:77): the screen strip reserved
/// above the viewports in `C4GraphicsSystem::RecalculateViewports`
/// (src/C4GraphicsSystem.cpp:345).
pub const UPPER_BOARD_HEIGHT: i32 = 50;

/// The vertical footprint of the classic 960x320 logo at C++'s 0.21 zoom.
///
/// Product logos may use a taller aspect ratio, but should not cover more of
/// the viewport than the classic upper-board artwork did.
const UPPER_BOARD_LOGO_MAX_HEIGHT: f32 = 320.0 * 0.21;

/// `C4UpperBoard::DisplayMode` (`src/C4UpperBoard.h:26-33`).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum UpperBoardMode {
    Hide,
    #[default]
    Full,
    Small,
    Mini,
}

/// The strip removed from the viewport area by `C4UpperBoard::Height`.
pub const fn upper_board_reserved_height(mode: UpperBoardMode) -> i32 {
    match mode {
        UpperBoardMode::Hide | UpperBoardMode::Mini => 0,
        UpperBoardMode::Full => UPPER_BOARD_HEIGHT,
        UpperBoardMode::Small => UPPER_BOARD_HEIGHT / 2,
    }
}

/// The actual top-board raster height established by `C4UpperBoard::Init`.
/// This may overlap the viewport when the loaded texture is taller than the
/// fixed strip returned by [`upper_board_reserved_height`].
pub fn upper_board_output_height(mode: UpperBoardMode, hud: &HudGraphics) -> i32 {
    let reserved = upper_board_reserved_height(mode);
    match mode {
        UpperBoardMode::Hide | UpperBoardMode::Mini => 0,
        UpperBoardMode::Full => hud
            .upper_board
            .as_ref()
            .map(|image| (image.height() as i32).max(reserved))
            .unwrap_or(reserved),
        UpperBoardMode::Small => hud
            .upper_board
            .as_ref()
            .map(|image| (image.height() as i32 / 2).max(reserved))
            .unwrap_or(reserved),
    }
}

/// `C4SymbolSize` / `C4SymbolBorder` (src/C4Constants.h:75-76).
pub const SYMBOL_SIZE: i32 = 35;
pub const SYMBOL_BORDER: i32 = 5;
/// `C4Viewport::DrawMouseButtons` uses two-thirds-size command-key cells.
pub const VIEWPORT_BUTTON_SIZE: i32 = 2 * SYMBOL_SIZE / 3;
/// `DrawMessageOffset` (src/C4GameMessage.cpp:95).
pub const DRAW_MESSAGE_OFFSET: i32 = -35;
/// `fctKeyboard` cell inside Control.png (src/C4GraphicsResource.cpp:201).
const KEYBOARD_CELL: (u32, u32) = (80, 36);
/// `fctMouse` inside Control.png (src/C4GraphicsResource.cpp:205).
const MOUSE_SOURCE: (u32, u32, u32, u32) = (198, 100, 32, 32);
/// `fctGamepad` is loaded with an 80px phase width and the image's full
/// height (src/C4GraphicsResource.cpp:229).
const GAMEPAD_CELL_WIDTH: u32 = 80;
/// White HUD text (`CStdDDraw::DEFAULT_MESSAGE_COLOR`, src/StdDDraw2.h:361).
const MESSAGE_COLOR: Color = Color::opaque(255, 255, 255);
/// `C4MSGB_MaxMsgFading` (src/C4MessageBoard.h:21).
pub const MESSAGE_BOARD_MAX_FADING_LINES: i32 = 6;
/// `0xfaFF0000`, the red caption color passed by C4MouseControl::Draw.
pub const MOUSE_CAPTION_COLOR: Color = Color::new(255, 0, 0, 0xfa);
/// FontRegular is the 14px main font (`C4Fonts.cpp:280-288`); the fallback
/// TextFont path draws at the same pixel size.
const FALLBACK_FONT_SIZE: f32 = 14.0;

/// `Game.GraphicsResource.FontRegular` for the HUD: the CStdFont-faithful
/// Clonk font when loaded, else the generic [`TextFont`] fallback.
pub enum HudFont<'a> {
    Clonk(&'a clonk_graphics::clonk_font::ClonkFont),
    Fallback(&'a dyn TextFont),
}

impl HudFont<'_> {
    pub fn from_set<'a>(set: Option<&'a ClonkFontSet>, fallback: &'a dyn TextFont) -> HudFont<'a> {
        set.map(|fonts| HudFont::Clonk(&fonts.text))
            .unwrap_or(HudFont::Fallback(fallback))
    }

    /// `CStdFont::GetLineHeight`.
    pub fn line_height(&self) -> i32 {
        match self {
            HudFont::Clonk(font) => font.line_height,
            HudFont::Fallback(font) => font
                .measure_text("0", FALLBACK_FONT_SIZE)
                .height
                .ceil()
                .max(1.0) as i32,
        }
    }

    /// `CStdFont::GetTextWidth`.
    pub fn text_width(&self, text: &str) -> i32 {
        match self {
            HudFont::Clonk(font) => font.measure(text, false).0,
            HudFont::Fallback(font) => {
                font.measure_text(text, FALLBACK_FONT_SIZE).width.ceil() as i32
            }
        }
    }

    /// Markup-aware `CStdFont::GetTextExtent(..., true)` used by classic
    /// Info/Dialog menus.
    pub fn text_width_markup(&self, text: &str) -> i32 {
        match self {
            HudFont::Clonk(font) => font.measure(text, true).0,
            HudFont::Fallback(font) => {
                let plain = strip_font_markup(text);
                font.measure_text(&plain, FALLBACK_FONT_SIZE).width.ceil() as i32
            }
        }
    }

    /// Markup-aware `CStdFont::GetTextExtent(..., true)` including pipe line
    /// breaks, as used by C4MouseControl captions.
    pub fn text_extent_markup(&self, text: &str) -> (i32, i32) {
        match self {
            HudFont::Clonk(font) => font.measure(text, true),
            HudFont::Fallback(font) => {
                let (width, lines) =
                    text.split('|')
                        .fold((0_i32, 0_i32), |(width, lines), line| {
                            let plain = strip_font_markup(line);
                            let line_width =
                                font.measure_text(&plain, FALLBACK_FONT_SIZE).width.ceil() as i32;
                            (width.max(line_width), lines + 1)
                        });
                (width, self.line_height().saturating_mul(lines.max(1)))
            }
        }
    }

    /// Per-character advance used by `CStdFont::BreakMessage`: glyph width
    /// plus `iHSpace`, including on the last character considered.
    pub fn character_advance(&self, character: char) -> i32 {
        match self {
            HudFont::Clonk(font) => font.message_character_advance(character),
            HudFont::Fallback(font) => font
                .measure_text(&character.to_string(), FALLBACK_FONT_SIZE)
                .width
                .ceil() as i32,
        }
    }

    /// `iGfxLineHgt`, the height used for `{{TextSpec}}` inline images.
    pub fn graphics_line_height(&self) -> i32 {
        match self {
            HudFont::Clonk(font) => font.cell_height,
            HudFont::Fallback(_) => self.line_height(),
        }
    }

    /// `CStdDDraw::TextOut` — `x` is the anchor for `align`.
    pub fn draw(
        &self,
        surface: &mut Surface,
        x: i32,
        y: i32,
        text: &str,
        color: Color,
        align: TextAlign,
    ) {
        self.draw_with_gamma(surface, x, y, text, color, align, None);
    }

    /// In-game `CStdDDraw::TextOut` with the active per-fragment gamma ramp.
    pub fn draw_with_gamma(
        &self,
        surface: &mut Surface,
        x: i32,
        y: i32,
        text: &str,
        color: Color,
        align: TextAlign,
        gamma: Option<&GammaRamp>,
    ) {
        match self {
            HudFont::Clonk(font) => font.draw_with_gamma(
                surface,
                x,
                y,
                text,
                [color.r, color.g, color.b, color.a],
                align,
                false,
                gamma,
            ),
            HudFont::Fallback(font) => {
                let width = self.text_width(text);
                let origin = match align {
                    TextAlign::Left => x,
                    TextAlign::Center => x - width / 2,
                    TextAlign::Right => x - width,
                };
                if let Some(gamma) = gamma {
                    draw_fallback_text_with_gamma(
                        *font,
                        surface,
                        origin as f32,
                        y as f32,
                        text,
                        color,
                        gamma,
                    );
                } else {
                    font.draw_text(
                        surface,
                        origin as f32,
                        y as f32,
                        text,
                        FALLBACK_FONT_SIZE,
                        color,
                    );
                }
            }
        }
    }

    /// Markup-aware `CStdDDraw::TextOut` used by Info/Dialog menu text.
    #[allow(clippy::too_many_arguments)]
    pub fn draw_markup_with_gamma(
        &self,
        surface: &mut Surface,
        x: i32,
        y: i32,
        text: &str,
        color: Color,
        align: TextAlign,
        gamma: Option<&GammaRamp>,
    ) {
        match self {
            HudFont::Clonk(font) => font.draw_with_gamma(
                surface,
                x,
                y,
                text,
                [color.r, color.g, color.b, color.a],
                align,
                true,
                gamma,
            ),
            HudFont::Fallback(_) => {
                let plain = strip_font_markup(text);
                self.draw_with_gamma(surface, x, y, &plain, color, align, gamma);
            }
        }
    }
}

fn strip_font_markup(text: &str) -> String {
    let mut plain = String::with_capacity(text.len());
    let mut rest = text;
    while let Some(character) = rest.chars().next() {
        if character == '<' {
            if let Some(end) = rest.find('>') {
                let tag = &rest[1..end];
                let name = tag.split_once(' ').map_or(tag, |(name, _)| name);
                if matches!(name, "i" | "/i" | "c" | "/c") {
                    rest = &rest[end + 1..];
                    continue;
                }
            }
        }
        plain.push(if character == '|' { '\n' } else { character });
        rest = &rest[character.len_utf8()..];
    }
    plain
}

fn draw_fallback_text_with_gamma(
    font: &dyn TextFont,
    surface: &mut Surface,
    origin_x: f32,
    origin_y: f32,
    text: &str,
    color: Color,
    gamma: &GammaRamp,
) {
    crate::draw_text_with_gamma(
        font,
        surface,
        origin_x,
        origin_y,
        text,
        FALLBACK_FONT_SIZE,
        color,
        Some(gamma),
    );
}

#[cfg(test)]
fn fill_hud_rect(
    surface: &mut Surface,
    rect: &clonk_gui::Rect,
    color: Color,
    gamma: Option<&GammaRamp>,
) {
    let color = gamma.map_or(color, |gamma| crate::gamma_encode_fragment(color, gamma));
    fill_rect(surface, rect, color);
}

#[allow(clippy::too_many_arguments)]
fn draw_hud_image_strip(
    surface: &mut Surface,
    dest_x: i32,
    dest_y: i32,
    image: &ImageData,
    src_x: u32,
    src_y: u32,
    src_w: u32,
    src_h: u32,
    gamma: Option<&GammaRamp>,
) {
    draw_image_strip(
        surface, dest_x, dest_y, image, src_x, src_y, src_w, src_h, gamma,
    );
}

fn draw_hud_image_bilinear(
    surface: &mut Surface,
    rect: &clonk_gui::Rect,
    image: &ImageData,
    gamma: Option<&GammaRamp>,
) {
    draw_image_bilinear(surface, rect, image, gamma);
}

fn draw_hud_image_bilinear_owner(
    surface: &mut Surface,
    rect: &clonk_gui::Rect,
    image: &ImageData,
    owner_color: u32,
    gamma: Option<&GammaRamp>,
) {
    draw_image_bilinear_owner(surface, rect, image, owner_color, gamma);
}

/// `{:02}:{:02}:{:02}` of `Game.Time` (C4UpperBoard::Execute,
/// src/C4UpperBoard.cpp:41).
pub fn format_game_time(seconds: u64) -> String {
    format!(
        "{:02}:{:02}:{:02}",
        seconds / 3600,
        (seconds % 3600) / 60,
        seconds % 60
    )
}

/// `CStdDDraw::BlitSurfaceTile`: unscaled tiling of `image` across the
/// target rect (upper board / message board backgrounds).
fn blit_tile(
    surface: &mut Surface,
    image: &ImageData,
    x: i32,
    y: i32,
    width: i32,
    height: i32,
    offset_x: i32,
    offset_y: i32,
    gamma: Option<&GammaRamp>,
) {
    let (tile_w, tile_h) = (image.width() as i32, image.height() as i32);
    if tile_w <= 0 || tile_h <= 0 {
        return;
    }
    let mut tile_y = y.saturating_add(offset_y % tile_h);
    while tile_y < y.saturating_add(height) {
        let src_y = y.saturating_sub(tile_y).max(0);
        let src_h = tile_h
            .min(y.saturating_add(height).saturating_sub(tile_y))
            .saturating_sub(src_y);
        let mut tile_x = x.saturating_add(offset_x % tile_w);
        while tile_x < x.saturating_add(width) {
            let src_x = x.saturating_sub(tile_x).max(0);
            let src_w = tile_w
                .min(x.saturating_add(width).saturating_sub(tile_x))
                .saturating_sub(src_x);
            draw_hud_image_strip(
                surface,
                tile_x.saturating_add(src_x),
                tile_y.saturating_add(src_y),
                image,
                src_x as u32,
                src_y as u32,
                src_w as u32,
                src_h as u32,
                gamma,
            );
            tile_x = tile_x.saturating_add(tile_w);
        }
        tile_y = tile_y.saturating_add(tile_h);
    }
}

/// `CStdDDraw::BlitSurfaceTile(..., scale)`: tile after applying the same
/// geometric scale to each source tile. Small upper boards pass `0.5f` here;
/// despite stale ticket wording, this parameter is not an opacity value.
#[allow(clippy::too_many_arguments)]
fn blit_tile_scaled(
    surface: &mut Surface,
    image: &ImageData,
    x: i32,
    y: i32,
    width: i32,
    height: i32,
    scale: f32,
    gamma: Option<&GammaRamp>,
) {
    if width <= 0 || height <= 0 || !scale.is_finite() || scale <= 0.0 {
        return;
    }
    if scale == 1.0 {
        blit_tile(surface, image, x, y, width, height, 0, 0, gamma);
        return;
    }
    let tile_width = image.width() as f32 * scale;
    let tile_height = image.height() as f32 * scale;
    if tile_width <= 0.0 || tile_height <= 0.0 {
        return;
    }

    let previous_clip = surface.clip();
    let target_clip = SurfaceRect::new(x, y, width as u32, height as u32);
    let effective_clip = previous_clip
        .and_then(|clip| clip.intersection(target_clip))
        .unwrap_or_else(|| {
            if previous_clip.is_some() {
                SurfaceRect::new(x, y, 0, 0)
            } else {
                target_clip
            }
        });
    surface.set_clip(effective_clip);

    let mut tile_y = y as f32;
    while tile_y < (y + height) as f32 {
        let mut tile_x = x as f32;
        while tile_x < (x + width) as f32 {
            draw_hud_image_bilinear(
                surface,
                &clonk_gui::Rect::new(tile_x, tile_y, tile_width, tile_height),
                image,
                gamma,
            );
            tile_x += tile_width;
        }
        tile_y += tile_height;
    }

    if let Some(clip) = previous_clip {
        surface.set_clip(clip);
    } else {
        surface.clear_clip();
    }
}

/// `C4Facet::Draw(cgo, fAspect=true)` (src/C4Facet.cpp:100-128): fit the
/// whole `image` into `rect` preserving aspect (integer math like C++),
/// returning the target rect actually covered.
fn aspect_fit(image_w: i32, image_h: i32, rect: SurfaceRect) -> SurfaceRect {
    let (cgo_w, cgo_h) = (rect.width as i32, rect.height as i32);
    if image_w <= 0 || image_h <= 0 || cgo_w <= 0 || cgo_h <= 0 {
        return rect;
    }
    let mut out = rect;
    if 100 * cgo_w / image_w < 100 * cgo_h / image_h {
        // By height (src/C4Facet.cpp:110-113).
        let new_h = image_h * cgo_w / image_w;
        out.y += (cgo_h - new_h) / 2;
        out.height = new_h.max(0) as u32;
    } else if 100 * cgo_h / image_h < 100 * cgo_w / image_w {
        // By width (src/C4Facet.cpp:115-119).
        let new_w = image_w * cgo_h / image_h;
        out.x += (cgo_w - new_w) / 2;
        out.width = new_w.max(0) as u32;
    }
    out
}

fn draw_image_aspect(
    surface: &mut Surface,
    image: &ImageData,
    rect: SurfaceRect,
    gamma: Option<&GammaRamp>,
) {
    let target = aspect_fit(image.width() as i32, image.height() as i32, rect);
    draw_hud_image_bilinear(
        surface,
        &clonk_gui::Rect::new(
            target.x as f32,
            target.y as f32,
            target.width as f32,
            target.height as f32,
        ),
        image,
        gamma,
    );
}

fn draw_image_aspect_owner(
    surface: &mut Surface,
    image: &ImageData,
    rect: SurfaceRect,
    owner_color: u32,
    gamma: Option<&GammaRamp>,
) {
    let target = aspect_fit(image.width() as i32, image.height() as i32, rect);
    draw_hud_image_bilinear_owner(
        surface,
        &clonk_gui::Rect::new(
            target.x as f32,
            target.y as f32,
            target.width as f32,
            target.height as f32,
        ),
        image,
        owner_color,
        gamma,
    );
}

/// `ClrByOwner` (src/C4Surface.cpp:236-287): blue-hued pixels — hue in
/// [145,175] of the 255-scaled HLS wheel (blue = 170), saturation > 100 —
/// are ColorByOwner pixels. Returns the gray value C++ keeps in the
/// overlay surface: the blue channel (`GetRValue(dwClr)` on the engine's
/// 0xAARRGGBB pixels, src/C4Surface.cpp:283-285).
pub(crate) fn clr_by_owner_gray(r: i32, g: i32, b: i32) -> Option<u8> {
    const HLSMAX: i32 = 255;
    const RGBMAX: i32 = 255;
    let c_max = r.max(g).max(b);
    let c_min = r.min(g).min(b);
    // Achromatic pixels never hit the saturation gate (S = 0).
    if c_max == c_min {
        return None;
    }
    let l = ((c_max + c_min) * HLSMAX + RGBMAX) / (2 * RGBMAX);
    let s = if l <= HLSMAX / 2 {
        ((c_max - c_min) * HLSMAX + (c_max + c_min) / 2) / (c_max + c_min)
    } else {
        ((c_max - c_min) * HLSMAX + (2 * RGBMAX - c_max - c_min) / 2) / (2 * RGBMAX - c_max - c_min)
    };
    let rdelta = ((c_max - r) * (HLSMAX / 6) + (c_max - c_min) / 2) / (c_max - c_min);
    let gdelta = ((c_max - g) * (HLSMAX / 6) + (c_max - c_min) / 2) / (c_max - c_min);
    let bdelta = ((c_max - b) * (HLSMAX / 6) + (c_max - c_min) / 2) / (c_max - c_min);
    let mut hue = if r == c_max {
        bdelta - gdelta
    } else if g == c_max {
        HLSMAX / 3 + rdelta - bdelta
    } else {
        2 * HLSMAX / 3 + gdelta - rdelta
    };
    if hue < 0 {
        hue += HLSMAX;
    }
    if hue > HLSMAX {
        hue -= HLSMAX;
    }
    ((145..=175).contains(&hue) && s > 100).then_some(b as u8)
}

fn colorize_by_owner_with(
    image: &ImageData,
    owner: Color,
    modulate: impl Fn(u8, u8) -> u8,
) -> ImageData {
    let pixels = image.pixels();
    let mut out = Vec::with_capacity(pixels.len());
    for chunk in pixels.chunks_exact(4) {
        let (r, g, b, a) = (chunk[0], chunk[1], chunk[2], chunk[3]);
        match clr_by_owner_gray(r as i32, g as i32, b as i32) {
            Some(gray) => {
                out.extend_from_slice(&[
                    modulate(owner.r, gray),
                    modulate(owner.g, gray),
                    modulate(owner.b, gray),
                    a,
                ]);
            }
            None => out.extend_from_slice(&[r, g, b, a]),
        }
    }
    ImageData::new(image.width(), image.height(), out)
}

/// `C4FacetExSurface::CreateClrByOwner` followed by normalized texture
/// modulation: blue ClrByOwner pixels become the owner color modulated by
/// their gray value. This is the display/GPU form where full white remains
/// 255 (`src/C4Surface.cpp:288-318`, `src/StdGL.cpp:488-503`).
pub fn colorize_by_owner(image: &ImageData, owner: Color) -> ImageData {
    thread_local! {
        static OWNER_IMAGES: RefCell<
            HashMap<(clonk_graphics::GpuTextureId, Color), ImageData>,
        > = RefCell::new(HashMap::new());
    }
    OWNER_IMAGES.with(|images| {
        let key = (image.gpu_texture_id(), owner);
        if let Some(image) = images.borrow().get(&key).cloned() {
            return image;
        }
        let colored = colorize_by_owner_with(image, owner, |channel, gray| {
            (u16::from(channel) * u16::from(gray) / 255) as u8
        });
        images.borrow_mut().insert(key, colored.clone());
        colored
    })
}

/// Software `C4Surface::GetPixDw(..., true)` owner modulation used when an
/// offscreen target sends `CStdDDraw::Blit` through `Blit8`. Legacy
/// `ModulateClr` divides RGB by 256, so even 255×255 becomes 254
/// (`src/C4Surface.cpp:673-700`, `src/StdColors.h:159-169`).
pub fn colorize_by_owner_software(image: &ImageData, owner: Color) -> ImageData {
    colorize_by_owner_with(image, owner, |channel, gray| {
        ((u16::from(channel) * u16::from(gray)) >> 8) as u8
    })
}

/// Width reserved at the right edge of the message board by Mini mode.
pub fn upper_board_text_strip_width(font: &HudFont<'_>, game_time_seconds: u64) -> i32 {
    upper_board_text_strip_width_for_text_width(
        font.text_width(&format_game_time(game_time_seconds)),
    )
}

/// Mini's fixed three-column strip after `C4UpperBoard::Init` has latched
/// `TextWidth` (src/C4UpperBoard.cpp:104-109).
pub fn upper_board_text_strip_width_for_text_width(text_width: i32) -> i32 {
    text_width.max(0).saturating_mul(3).saturating_add(30)
}

/// Logical width left to `C4MessageBoard` after `C4UpperBoard::Init`.
pub fn message_board_available_width(
    surface_width: i32,
    font: &HudFont<'_>,
    mode: UpperBoardMode,
    game_time_seconds: u64,
) -> i32 {
    message_board_available_width_for_text_width(
        surface_width,
        mode,
        font.text_width(&format_game_time(game_time_seconds)),
    )
}

/// Logical message-board width using the `TextWidth` cached by
/// `C4UpperBoard::Init`.
pub fn message_board_available_width_for_text_width(
    surface_width: i32,
    mode: UpperBoardMode,
    text_width: i32,
) -> i32 {
    match mode {
        UpperBoardMode::Mini => surface_width
            .saturating_sub(upper_board_text_strip_width_for_text_width(text_width))
            .max(0),
        _ => surface_width.max(0),
    }
}

/// `C4UpperBoard::Draw` (src/C4UpperBoard.cpp:46-96).
pub fn draw_upper_board(
    surface: &mut Surface,
    font: &HudFont<'_>,
    hud: &HudGraphics,
    mode: UpperBoardMode,
    scenario_title: &str,
    game_time_seconds: u64,
) {
    draw_upper_board_with_gamma(
        surface,
        font,
        hud,
        mode,
        scenario_title,
        game_time_seconds,
        None,
        None,
        None,
    );
}

pub(crate) fn draw_upper_board_with_gamma(
    surface: &mut Surface,
    font: &HudFont<'_>,
    hud: &HudGraphics,
    mode: UpperBoardMode,
    scenario_title: &str,
    game_time_seconds: u64,
    clock_text: Option<&str>,
    frames_per_second: Option<i32>,
    gamma: Option<&GammaRamp>,
) {
    let text_width = font.text_width(&format_game_time(game_time_seconds));
    draw_upper_board_with_initialized_text_width(
        surface,
        font,
        hud,
        mode,
        scenario_title,
        game_time_seconds,
        text_width,
        clock_text,
        frames_per_second,
        gamma,
    );
}

#[allow(clippy::too_many_arguments)]
pub(crate) fn draw_upper_board_with_initialized_text_width(
    surface: &mut Surface,
    font: &HudFont<'_>,
    hud: &HudGraphics,
    mode: UpperBoardMode,
    scenario_title: &str,
    game_time_seconds: u64,
    text_width: i32,
    clock_text: Option<&str>,
    frames_per_second: Option<i32>,
    gamma: Option<&GammaRamp>,
) {
    if mode == UpperBoardMode::Hide {
        return;
    }
    let width = surface.width() as i32;

    if mode != UpperBoardMode::Mini {
        let board_height = upper_board_output_height(mode, hud);
        let Some(board) = hud.upper_board.as_ref() else {
            return;
        };
        blit_tile_scaled(
            surface,
            board,
            0,
            0,
            width,
            board_height,
            if mode == UpperBoardMode::Small {
                0.5
            } else {
                1.0
            },
            gamma,
        );

        // Logo (src/C4UpperBoard.cpp:54-71).
        if let Some(logo) = hud.logo.as_ref() {
            let (logo_w, logo_h) = (logo.width() as f32, logo.height() as f32);
            if logo_w > 0.0 && logo_h > 0.0 {
                let mut zoom = if logo_w / logo_h != 3.0 { 0.25 } else { 0.21 };
                zoom *= 960.0 / logo_w;
                zoom = zoom.min(UPPER_BOARD_LOGO_MAX_HEIGHT / logo_h);
                if mode == UpperBoardMode::Small {
                    zoom *= 8.0 / 15.0;
                }
                let dst_w = (logo_w * zoom) as i32;
                let dst_h = (logo_h * zoom) as i32;
                let dst_x = (width as f32 / 2.0 - (logo_w / 2.0) * zoom) as i32;
                draw_hud_image_bilinear(
                    surface,
                    &clonk_gui::Rect::new(dst_x as f32, 0.0, dst_w as f32, dst_h as f32),
                    logo,
                    gamma,
                );
            }
        }
    }

    // Mini's Output facet is the right side of the bottom message strip.
    // Other modes center on their fixed reserved height, not texture height.
    let text_y = if mode == UpperBoardMode::Mini {
        surface.height() as i32 - font.line_height()
    } else {
        upper_board_reserved_height(mode) / 2 - font.line_height() / 2
    };
    let time_text = format_game_time(game_time_seconds);
    let mut right_offset = 1;
    font.draw_with_gamma(
        surface,
        width - right_offset * text_width - 10,
        text_y,
        &time_text,
        MESSAGE_COLOR,
        TextAlign::Left,
        gamma,
    );
    right_offset += 1;
    if let Some(clock_text) = clock_text {
        font.draw_with_gamma(
            surface,
            width - right_offset * text_width - 30,
            text_y,
            clock_text,
            MESSAGE_COLOR,
            TextAlign::Left,
            gamma,
        );
        right_offset += 1;
    }
    if let Some(frames_per_second) = frames_per_second {
        let fps_text = format!("{frames_per_second} FPS");
        font.draw_with_gamma(
            surface,
            width - right_offset * text_width - 30,
            text_y,
            &fps_text,
            MESSAGE_COLOR,
            TextAlign::Left,
            gamma,
        );
    }
    if mode != UpperBoardMode::Mini {
        let title_y = upper_board_output_height(mode, hud) / 2 - font.line_height() / 2;
        font.draw_with_gamma(
            surface,
            10,
            title_y,
            scenario_title,
            MESSAGE_COLOR,
            TextAlign::Left,
            gamma,
        );
    }
}

/// `C4Facet::DrawValue` with `C4FCT_Center` (src/C4Facet.cpp:240-250):
/// icon aspect-fit in `cgo`, value right-aligned hanging off the
/// bottom-right corner.
fn draw_value(
    surface: &mut Surface,
    font: &HudFont<'_>,
    icon: Option<&ImageData>,
    text: &str,
    cgo: SurfaceRect,
    gamma: Option<&GammaRamp>,
) {
    if let Some(icon) = icon {
        draw_image_aspect(surface, icon, cgo, gamma);
    }
    font.draw_with_gamma(
        surface,
        cgo.x + cgo.width as i32 - 1,
        cgo.y + cgo.height as i32 - 1,
        text,
        MESSAGE_COLOR,
        TextAlign::Right,
        gamma,
    );
}

/// The wealth / score / crew fixed items of `C4Viewport::DrawPlayerInfo`
/// (src/C4Viewport.cpp:1281-1322), `Config.Graphics.ShowPlayerHUDAlways`
/// defaults on (src/C4Config.cpp:445).
#[allow(clippy::too_many_arguments)]
pub fn draw_player_fixed_items(
    surface: &mut Surface,
    font: &HudFont<'_>,
    hud: &HudGraphics,
    viewport: SurfaceRect,
    wealth: i32,
    score: i32,
    select_count: i32,
    crew_count: i32,
    owner_color: Color,
) {
    draw_player_fixed_items_with_gamma(
        surface,
        font,
        hud,
        viewport,
        wealth,
        score,
        select_count,
        crew_count,
        owner_color,
        true,
        true,
        true,
        None,
    );
}

#[allow(clippy::too_many_arguments)]
pub(crate) fn draw_player_fixed_items_with_gamma(
    surface: &mut Surface,
    font: &HudFont<'_>,
    hud: &HudGraphics,
    viewport: SurfaceRect,
    wealth: i32,
    score: i32,
    select_count: i32,
    crew_count: i32,
    owner_color: Color,
    show_wealth: bool,
    show_score: bool,
    show_crew: bool,
    gamma: Option<&GammaRamp>,
) {
    let crew_icon = show_crew
        .then(|| {
            hud.crew
                .as_ref()
                .map(|icon| colorize_by_owner(icon, owner_color))
        })
        .flatten();
    draw_player_fixed_items_with_colored_crew_gamma(
        surface,
        font,
        hud,
        viewport,
        wealth,
        score,
        select_count,
        crew_count,
        crew_icon.as_ref(),
        show_wealth,
        show_score,
        show_crew,
        gamma,
    );
}

#[allow(clippy::too_many_arguments)]
pub(crate) fn draw_player_fixed_items_with_colored_crew_gamma(
    surface: &mut Surface,
    font: &HudFont<'_>,
    hud: &HudGraphics,
    viewport: SurfaceRect,
    wealth: i32,
    score: i32,
    select_count: i32,
    crew_count: i32,
    crew_icon: Option<&ImageData>,
    show_wealth: bool,
    show_score: bool,
    show_crew: bool,
    gamma: Option<&GammaRamp>,
) {
    let (wdt, hgt) = (SYMBOL_SIZE, SYMBOL_SIZE / 2);
    let right = viewport.x + viewport.width as i32;
    let top = viewport.y + SYMBOL_BORDER;

    if show_wealth {
        // Wealth (src/C4Viewport.cpp:1287-1296).
        let cgo = SurfaceRect::new(right - wdt - SYMBOL_BORDER, top, wdt as u32, hgt as u32);
        draw_value(
            surface,
            font,
            hud.wealth.as_ref(),
            &wealth.to_string(),
            cgo,
            gamma,
        );
    }

    if show_score {
        // Value gain / score (src/C4Viewport.cpp:1299-1309).
        let cgo = SurfaceRect::new(
            right - 2 * wdt - 2 * SYMBOL_BORDER,
            top,
            wdt as u32,
            hgt as u32,
        );
        draw_value(
            surface,
            font,
            hud.score.as_ref(),
            &score.to_string(),
            cgo,
            gamma,
        );
    }

    if show_crew {
        // Crew (src/C4Viewport.cpp:1312-1321): fctCrewClr colored by the
        // player color, "SelectCount/ActiveCrewCount".
        let cgo = SurfaceRect::new(
            right - 3 * wdt - 3 * SYMBOL_BORDER,
            top,
            wdt as u32,
            hgt as u32,
        );
        draw_value(
            surface,
            font,
            crew_icon,
            &format!("{select_count}/{crew_count}"),
            cgo,
            gamma,
        );
    }
}

/// `C4ObjectInfo::Draw` (src/C4ObjectInfo.cpp:302-371) inside the
/// `DrawCursorInfo` info facet (src/C4Viewport.cpp:904).
#[allow(clippy::too_many_arguments)]
pub fn draw_cursor_info(
    surface: &mut Surface,
    font: &HudFont<'_>,
    hud: &HudGraphics,
    viewport: SurfaceRect,
    name: &str,
    rank: i32,
    portrait: Option<&ImageData>,
    rank_symbols: Option<&ImageData>,
) {
    draw_cursor_info_with_gamma(
        surface,
        font,
        hud,
        viewport,
        name,
        rank,
        None,
        portrait,
        None,
        u32::MAX,
        rank_symbols,
        None,
        false,
        0,
        None,
    );
}

#[allow(clippy::too_many_arguments)]
pub(crate) fn draw_cursor_info_with_gamma(
    surface: &mut Surface,
    font: &HudFont<'_>,
    hud: &HudGraphics,
    viewport: SurfaceRect,
    name: &str,
    rank: i32,
    rank_name: Option<&str>,
    portrait: Option<&ImageData>,
    portrait_owner_overlay: Option<&ImageData>,
    portrait_owner_color: u32,
    rank_symbols: Option<&ImageData>,
    rank_symbol_count: Option<u32>,
    is_captain: bool,
    hide_hud_elements: i32,
    gamma: Option<&GammaRamp>,
) {
    // ccgo = (border, border, 3*C4SymbolSize, C4SymbolSize)
    // (src/C4Viewport.cpp:904).
    let cgo = SurfaceRect::new(
        viewport.x + SYMBOL_BORDER,
        viewport.y + SYMBOL_BORDER,
        (3 * SYMBOL_SIZE) as u32,
        SYMBOL_SIZE as u32,
    );
    let mut ix = 0;

    // Portrait: 4*Hgt/3+10 x Hgt+10 (src/C4ObjectInfo.cpp:308-320).
    if hide_hud_elements & clonk_engine::HIDE_HUD_ELEMENT_PORTRAIT == 0 {
        if let Some(portrait) = portrait {
            let rect = SurfaceRect::new(
                cgo.x + ix,
                cgo.y,
                (4 * SYMBOL_SIZE / 3 + 10) as u32,
                (SYMBOL_SIZE + 10) as u32,
            );
            draw_image_aspect(surface, portrait, rect, gamma);
            if let Some(owner_overlay) = portrait_owner_overlay {
                draw_image_aspect_owner(surface, owner_overlay, rect, portrait_owner_color, gamma);
            }
            ix += 4 * SYMBOL_SIZE / 3;
        }
    }

    // HH_Captain gates this status symbol independently of the extended-rank
    // star below (src/C4ObjectInfo.cpp:323-328).
    if is_captain && hide_hud_elements & clonk_engine::HIDE_HUD_ELEMENT_CAPTAIN == 0 {
        if let Some(captain) = hud.captain.as_ref() {
            draw_hud_image_strip(
                surface,
                cgo.x + ix,
                cgo.y,
                captain,
                0,
                0,
                captain.width(),
                captain.height(),
                gamma,
            );
            ix += captain.width() as i32;
        }
    }

    // Rank symbol: C4RankSystem::DrawRankSymbol draws the phase cell 1:1
    // (src/C4RankSystem.cpp:305-307); cells are height-square
    // (C4FCT_Height load, src/C4GraphicsResource.cpp:215).
    if hide_hud_elements & clonk_engine::HIDE_HUD_ELEMENT_RANK_IMAGE == 0 {
        let custom_symbols = rank_symbols.is_some();
        let symbols = rank_symbols.or(hud.rank.as_ref());
        if let Some(symbols) = symbols {
            let cell = symbols.height();
            if cell > 0 && symbols.width() >= cell {
                let total_count = (symbols.width() / cell).max(1);
                let base_count = if custom_symbols {
                    rank_symbol_count
                        .unwrap_or(total_count)
                        .clamp(1, total_count)
                } else {
                    total_count
                };
                let rank = rank.max(0) as u32;
                let mut base_rank = rank % base_count;
                let extension_level = rank / base_count;
                let extension_phase =
                    (extension_level > 0 && total_count > base_count).then(|| {
                        let requested = base_count + extension_level - 1;
                        if requested >= total_count {
                            base_rank = base_count - 1;
                            total_count - 1
                        } else {
                            requested
                        }
                    });
                draw_hud_image_strip(
                    surface,
                    cgo.x + ix,
                    cgo.y,
                    symbols,
                    base_rank * cell,
                    0,
                    cell,
                    cell,
                    gamma,
                );
                if let Some(extension_phase) = extension_phase {
                    draw_hud_image_strip(
                        surface,
                        cgo.x + ix - 4,
                        cgo.y - 3,
                        symbols,
                        extension_phase * cell,
                        0,
                        cell,
                        cell,
                        gamma,
                    );
                } else if extension_level > 0 {
                    if let Some(captain) = hud.captain.as_ref() {
                        draw_hud_image_strip(
                            surface,
                            cgo.x + ix - 4,
                            cgo.y - 3,
                            captain,
                            0,
                            0,
                            captain.width(),
                            captain.height(),
                            gamma,
                        );
                    }
                }
            }
        }
        // C++ advances by the global fctRank width even when a definition
        // supplies an invalid/missing custom strip or DrawRankSymbol fails.
        ix += hud.rank.as_ref().map(ImageData::height).unwrap_or(0) as i32;
    }

    // Rank and name (src/C4ObjectInfo.cpp:353-370) — the C++ `|` separator
    // stacks the rank name above the crew name.
    let rank_name = rank_name.filter(|rank_name| {
        hide_hud_elements & clonk_engine::HIDE_HUD_ELEMENT_RANK == 0
            && rank > 0
            && !rank_name.is_empty()
    });
    let name = if hide_hud_elements & clonk_engine::HIDE_HUD_ELEMENT_NAME == 0 {
        name
    } else {
        ""
    };
    if let Some(rank_name) = rank_name {
        font.draw_with_gamma(
            surface,
            cgo.x + ix,
            cgo.y,
            rank_name,
            MESSAGE_COLOR,
            TextAlign::Left,
            gamma,
        );
        if !name.is_empty() {
            font.draw_with_gamma(
                surface,
                cgo.x + ix,
                cgo.y + font.line_height(),
                name,
                MESSAGE_COLOR,
                TextAlign::Left,
                gamma,
            );
        }
    } else if !name.is_empty() {
        font.draw_with_gamma(
            surface,
            cgo.x + ix,
            cgo.y,
            name,
            MESSAGE_COLOR,
            TextAlign::Left,
            gamma,
        );
    }
}

/// The cursor contents row from `C4Viewport::DrawCursorInfo`
/// (src/C4Viewport.cpp:911-917). `C4ObjectList::DrawIDList` advances through
/// successive height-square sections of the 7-symbol-wide facet
/// (src/C4ObjectList.cpp:343-372; src/C4Facet.cpp:44-48).
pub fn draw_inventory(
    surface: &mut Surface,
    font: &HudFont<'_>,
    viewport: SurfaceRect,
    inventory: &[InventoryOverlay],
) {
    draw_inventory_with_gamma(surface, font, viewport, inventory, None);
}

pub(crate) fn draw_inventory_with_gamma(
    surface: &mut Surface,
    font: &HudFont<'_>,
    viewport: SurfaceRect,
    inventory: &[InventoryOverlay],
    gamma: Option<&GammaRamp>,
) {
    let origin_x = viewport.x + SYMBOL_BORDER;
    let origin_y = viewport.y + viewport.height as i32 - SYMBOL_BORDER - SYMBOL_SIZE;
    for (section, item) in inventory.iter().enumerate() {
        let cell = SurfaceRect::new(
            origin_x + section as i32 * SYMBOL_SIZE,
            origin_y,
            SYMBOL_SIZE as u32,
            SYMBOL_SIZE as u32,
        );
        let draw_picture = |surface: &mut Surface, picture: &ImageData, additive: bool| {
            let target = aspect_fit(picture.width() as i32, picture.height() as i32, cell);
            let rect = clonk_gui::Rect::new(
                target.x as f32,
                target.y as f32,
                target.width as f32,
                target.height as f32,
            );
            if additive {
                draw_image_bilinear_additive(surface, &rect, picture, gamma);
            } else {
                draw_hud_image_bilinear(surface, &rect, picture, gamma);
            }
        };
        if let Some(picture) = item.picture.as_ref() {
            draw_picture(surface, picture, item.additive);
        }
        for overlay in &item.picture_overlays {
            draw_picture(surface, &overlay.picture, overlay.additive);
        }
        // DrawIDList writes "{count}x" at the section's bottom-right for
        // every stack except a single item (C4ObjectList.cpp:343-368).
        if item.count != 1 {
            font.draw_with_gamma(
                surface,
                cell.x + cell.width as i32 - 1,
                cell.y + cell.height as i32 - 1 - font.line_height(),
                &format!("{}x", item.count),
                MESSAGE_COLOR,
                TextAlign::Right,
                gamma,
            );
        }
    }
}

/// C4RegionList hit test for the grouped cursor-inventory cells registered by
/// C4ObjectList::DrawIDList (C4Viewport.cpp:911-917; C4Region.cpp:87-94).
pub fn inventory_region_index(
    viewport: SurfaceRect,
    point: clonk_gui::Point,
    item_count: usize,
) -> Option<usize> {
    let origin_x = viewport.x + SYMBOL_BORDER;
    let origin_y = viewport.y + viewport.height as i32 - SYMBOL_BORDER - SYMBOL_SIZE;
    let x = point.x.floor() as i32 - origin_x;
    let y = point.y.floor() as i32 - origin_y;
    if x < 0 || !(0..SYMBOL_SIZE).contains(&y) {
        return None;
    }
    let section = usize::try_from(x / SYMBOL_SIZE).ok()?;
    (section < item_count).then_some(section)
}

/// Global output rectangle of one grouped cursor-inventory C4Region.
pub fn inventory_region_rect(
    viewport: SurfaceRect,
    section: usize,
    item_count: usize,
) -> Option<SurfaceRect> {
    if section >= item_count {
        return None;
    }
    Some(SurfaceRect::new(
        viewport.x + SYMBOL_BORDER + i32::try_from(section).ok()?.saturating_mul(SYMBOL_SIZE),
        viewport.y + viewport.height as i32 - SYMBOL_BORDER - SYMBOL_SIZE,
        SYMBOL_SIZE as u32,
        SYMBOL_SIZE as u32,
    ))
}

/// Measure C4MouseControl's red caption and apply its viewport-local clamp.
/// `caption_bottom_y` is the global Y of a hovered region's top edge.
pub fn mouse_caption_rect(
    font: &HudFont<'_>,
    viewport: SurfaceRect,
    pointer: clonk_gui::Point,
    caption_bottom_y: Option<i32>,
    text: &str,
) -> Option<SurfaceRect> {
    if text.is_empty() {
        return None;
    }
    let (width, height) = font.text_extent_markup(text);
    let width = width.max(1);
    let height = height.max(1);
    let viewport_right = viewport.x.saturating_add(viewport.width as i32);
    let minimum_center = viewport.x.saturating_add(width / 2).saturating_add(1);
    let maximum_center = viewport_right.saturating_sub(width / 2).saturating_sub(1);
    let pointer_x = pointer.x.floor() as i32;
    let center_x = if pointer_x < minimum_center {
        minimum_center
    } else if pointer_x > maximum_center {
        maximum_center
    } else {
        pointer_x
    };
    let requested_y = caption_bottom_y
        .map(|bottom| bottom.saturating_sub(height).saturating_sub(1))
        .unwrap_or_else(|| (pointer.y.floor() as i32).saturating_add(13));
    let y = requested_y.min(
        viewport
            .y
            .saturating_add(viewport.height as i32)
            .saturating_sub(height),
    );
    Some(SurfaceRect::new(
        center_x.saturating_sub(width / 2),
        y,
        width as u32,
        height as u32,
    ))
}

/// Draw C4MouseControl's non-help caption in FontRegular. Pipes delimit the
/// native two-line hover hints.
pub fn draw_mouse_caption(
    surface: &mut Surface,
    font: &HudFont<'_>,
    viewport: SurfaceRect,
    pointer: clonk_gui::Point,
    caption_bottom_y: Option<i32>,
    text: &str,
    gamma: Option<&GammaRamp>,
) -> Option<SurfaceRect> {
    let rect = mouse_caption_rect(font, viewport, pointer, caption_bottom_y, text)?;
    let center_x = rect.x.saturating_add(rect.width as i32 / 2);
    let previous_clip = surface.clip();
    let caption_clip = previous_clip.map_or(viewport, |clip| {
        clip.intersection(viewport)
            .unwrap_or_else(|| SurfaceRect::new(viewport.x, viewport.y, 0, 0))
    });
    surface.set_clip(caption_clip);
    match font {
        HudFont::Clonk(_) => font.draw_markup_with_gamma(
            surface,
            center_x,
            rect.y,
            text,
            MOUSE_CAPTION_COLOR,
            TextAlign::Center,
            gamma,
        ),
        HudFont::Fallback(_) => {
            let line_height = font.line_height().max(1);
            for (index, line) in text.split('|').enumerate() {
                font.draw_markup_with_gamma(
                    surface,
                    center_x,
                    rect.y.saturating_add(
                        i32::try_from(index)
                            .unwrap_or(i32::MAX)
                            .saturating_mul(line_height),
                    ),
                    line,
                    MOUSE_CAPTION_COLOR,
                    TextAlign::Center,
                    gamma,
                );
            }
        }
    }
    if let Some(clip) = previous_clip {
        surface.set_clip(clip);
    } else {
        surface.clear_clip();
    }
    Some(rect)
}

/// One contextual command entry of the C4Viewport::DrawCursorInfo command
/// rows (src/C4Viewport.cpp:947-962): a C4Object::DrawCommand pair — key
/// cell (fctKey cap + fctCommand symbol + key name) and image cell
/// (src/C4Object.cpp:4018-4078) — resolved to presentation data by the app.
#[derive(Clone, Debug, PartialEq)]
pub struct CommandIcon {
    /// The COM_* code incl. the COM_Double bit (src/C4Constants.h:173-235):
    /// picks the fctCommand phase via Com2Control and the double row.
    pub com: u8,
    /// `PlrControlKeyName(iPlayer, Com2Control(iCom), true)`
    /// (src/C4Object.cpp:4071-4073); empty = no label.
    pub key_label: String,
    /// Secondary (right side) area — self activation & specials
    /// (src/C4Object.cpp:3083-3098); bottom area otherwise.
    pub side: bool,
    /// C4Region caption installed over the paired key/image cells.
    pub caption: String,
    pub image: CommandImage,
}

/// What fills a command's image cell.
#[derive(Clone, Debug, PartialEq)]
pub enum CommandImage {
    /// Def picture aspect-fit into the cell (DrawPicture / pDescImageDef,
    /// src/C4Object.cpp:4053-4068).
    Picture(Option<ImageData>),
    /// Picture in GetFraction(85,85,Right,Top) + facet icon in
    /// GetFraction(85,85,Left,Bottom) (src/C4Object.cpp:2960-2996,3040-3068).
    Composite {
        picture: Option<ImageData>,
        icon: CommandOverlayIcon,
    },
    /// `fctExit.Draw(ccgo)` — the contained exit command
    /// (src/C4Object.cpp:3013-3017).
    Exit,
    /// `DrawMenuSymbol(C4MN_Buy, ...)` (src/C4Menu.cpp:61-65).
    BuyMenu { owner_color: Color },
    /// `DrawMenuSymbol(C4MN_Sell, ...)` (src/C4Menu.cpp:66-70).
    SellMenu { owner_color: Color },
    /// Target picture at left-bottom plus OKCancel phase (0,1) at
    /// right-top (src/C4ObjectMenu.cpp:405-414).
    InfoMenu { picture: Option<ImageData> },
}

/// The overlay icon of a composite image cell.
#[derive(Clone, Debug, PartialEq)]
pub enum CommandOverlayIcon {
    /// `fctBuild` (src/C4Object.cpp:2962).
    Build,
    /// `fctHand` phase — 0 put, 1 get, 6 ungrab
    /// (src/C4Object.cpp:2978-2994).
    Hand(i32),
}

/// `Com2Control` (src/C4ObjectCom.cpp:857-877) plus the COM_Double bit:
/// the fctCommand sheet phase (x = control index, y = double row).
fn com_control_index(com: i32) -> (i32, bool) {
    let double = com & 128 != 0; // COM_Double
    let control = match com & !(64 | 128) {
        12 => 0, // COM_CursorLeft   -> CON_CursorLeft
        14 => 1, // COM_CursorToggle -> CON_CursorToggle
        13 => 2, // COM_CursorRight  -> CON_CursorRight
        5 => 3,  // COM_Throw        -> CON_Throw
        3 => 4,  // COM_Up           -> CON_Up
        6 => 5,  // COM_Dig          -> CON_Dig
        1 => 6,  // COM_Left         -> CON_Left
        4 => 7,  // COM_Down         -> CON_Down
        2 => 8,  // COM_Right        -> CON_Right
        7 => 10, // COM_Special      -> CON_Special
        8 => 11, // COM_Special2     -> CON_Special2
        _ => 9,  // default          -> CON_Menu
    };
    (control, double)
}

/// Nearest-neighbour scale of an `image` subregion into `dest` — the
/// unfiltered C4Facet blit the command cells use.
fn draw_scaled_region(
    surface: &mut Surface,
    image: &ImageData,
    src: SurfaceRect,
    dest: SurfaceRect,
    gamma: Option<&GammaRamp>,
) {
    if src.width == 0 || src.height == 0 || dest.width == 0 || dest.height == 0 {
        return;
    }
    if surface.is_gpu_scene_capture_active()
        && src.x >= 0
        && src.y >= 0
        && i64::from(src.x) + i64::from(src.width) <= i64::from(image.width())
        && i64::from(src.y) + i64::from(src.height) <= i64::from(image.height())
    {
        if let (Ok(source_width), Ok(source_height)) =
            (i32::try_from(src.width), i32::try_from(src.height))
        {
            crate::draw_image_region(
                surface,
                &clonk_gui::Rect::new(
                    dest.x as f32,
                    dest.y as f32,
                    dest.width as f32,
                    dest.height as f32,
                ),
                image,
                None,
                &crate::SourceRect::new(src.x, src.y, source_width, source_height),
                false,
                None,
                crate::SpriteBlitState::normal(),
                gamma,
                None,
            );
            return;
        }
    }
    let pixels = image.pixels();
    let (img_w, img_h) = (image.width() as i32, image.height() as i32);
    for dy in 0..dest.height as i32 {
        let sy = src.y + (dy as i64 * src.height as i64 / dest.height as i64) as i32;
        if !(0..img_h).contains(&sy) {
            continue;
        }
        for dx in 0..dest.width as i32 {
            let sx = src.x + (dx as i64 * src.width as i64 / dest.width as i64) as i32;
            if !(0..img_w).contains(&sx) {
                continue;
            }
            let idx = ((sy * img_w + sx) * 4) as usize;
            let color = Color::new(
                pixels[idx],
                pixels[idx + 1],
                pixels[idx + 2],
                pixels[idx + 3],
            );
            if color.a == 0 {
                continue;
            }
            let (tx, ty) = (dest.x + dx, dest.y + dy);
            if tx < 0 || ty < 0 {
                continue;
            }
            let result = if surface.is_gpu_scene_capture_active() {
                // Invalid/partially out-of-bounds source rectangles cannot
                // become one retained texture quad. Keep their recovery
                // fragments ordered in the GPU scene without changing the
                // byte-exact software oracle below.
                surface.blend_fragment(
                    tx as u32,
                    ty as u32,
                    [
                        f32::from(color.r),
                        f32::from(color.g),
                        f32::from(color.b),
                        f32::from(color.a),
                    ],
                    gamma,
                )
            } else if let Some(gamma) = gamma {
                let destination = surface.get_pixel(tx as u32, ty as u32).unwrap_or_default();
                let output = if color.a == 255 {
                    crate::gamma_encode_fragment(color, gamma)
                } else {
                    crate::gamma_blend_fragment_over(color, destination, gamma)
                };
                surface.set_pixel(tx as u32, ty as u32, output)
            } else if color.a == 255 {
                surface.set_pixel(tx as u32, ty as u32, color)
            } else {
                surface.blend_pixel(tx as u32, ty as u32, color)
            };
            if result.is_err() {
                return;
            }
        }
    }
}

/// Aspect-fit an image subregion into `dest` (C4Facet::Draw fAspect,
/// src/C4Facet.cpp:99-130): scale preserving ratio, centered.
fn draw_scaled_region_aspect(
    surface: &mut Surface,
    image: &ImageData,
    src: SurfaceRect,
    dest: SurfaceRect,
    gamma: Option<&GammaRamp>,
) {
    if src.width == 0 || src.height == 0 {
        return;
    }
    let scale = (dest.width as f32 / src.width as f32).min(dest.height as f32 / src.height as f32);
    let w = ((src.width as f32 * scale) as u32).max(1);
    let h = ((src.height as f32 * scale) as u32).max(1);
    let fitted = SurfaceRect::new(
        dest.x + (dest.width as i32 - w as i32) / 2,
        dest.y + (dest.height as i32 - h as i32) / 2,
        w,
        h,
    );
    draw_scaled_region(surface, image, src, fitted, gamma);
}

/// The whole image aspect-fit into `dest`.
fn draw_image_aspect_fit(
    surface: &mut Surface,
    image: &ImageData,
    dest: SurfaceRect,
    gamma: Option<&GammaRamp>,
) {
    let src = SurfaceRect::new(0, 0, image.width(), image.height());
    draw_scaled_region_aspect(surface, image, src, dest, gamma);
}

/// `C4Facet::GetFraction` (src/C4Facet.cpp:459-474) over a square cell.
fn get_fraction(
    cell: SurfaceRect,
    percent_wdt: i32,
    percent_hgt: i32,
    align_right: bool,
    align_bottom: bool,
    align_center_y: bool,
) -> SurfaceRect {
    let wdt = (cell.width as i32 * percent_wdt / 100).max(1);
    let hgt = (cell.height as i32 * percent_hgt / 100).max(1);
    let mut x = cell.x;
    let mut y = cell.y;
    if align_right {
        x += cell.width as i32 - wdt;
    }
    if align_bottom {
        y += cell.height as i32 - hgt;
    }
    if align_center_y {
        y += cell.height as i32 / 2 - hgt / 2;
    }
    SurfaceRect::new(x, y, wdt as u32, hgt as u32)
}

/// A square-by-height sheet cell (C4FCT_Height loads,
/// src/C4GraphicsResource.cpp:228-233).
fn sheet_cell(image: &ImageData, phase: i32) -> SurfaceRect {
    let cell = image.height() as i32;
    SurfaceRect::new(phase * cell, 0, cell as u32, cell as u32)
}

/// `DrawCommandKey` (src/C4ObjectCom.cpp:930-944): fctKey cap (Control.png
/// (0,100) 64x64, phase 0 unpressed), fctCommand symbol (Control.png (0,36)
/// 32x32 phases; y phase 1 = double coms), key name in the small font when
/// ShowCommandKeys is set (23px cells <= C4MN_SymbolSize pick FontTiny).
fn draw_command_key_cell(
    surface: &mut Surface,
    font: &HudFont<'_>,
    hud: &HudGraphics,
    cell: SurfaceRect,
    com: u8,
    key_label: &str,
    show_command_keys: bool,
    gamma: Option<&GammaRamp>,
) {
    if let Some(control) = hud.control.as_ref() {
        draw_scaled_region(
            surface,
            control,
            SurfaceRect::new(0, 100, 64, 64),
            cell,
            gamma,
        );
        let (control_index, double) = com_control_index(i32::from(com));
        draw_scaled_region(
            surface,
            control,
            SurfaceRect::new(32 * control_index, 36 + 32 * i32::from(double), 32, 32),
            cell,
            gamma,
        );
    }
    if show_command_keys && !key_label.is_empty() {
        font.draw_with_gamma(
            surface,
            cell.x + cell.width as i32 / 2,
            cell.y + cell.height as i32 - font.line_height() - 2,
            key_label,
            MESSAGE_COLOR,
            TextAlign::Center,
            gamma,
        );
    }
}

/// The local controls registered by `C4Viewport::DrawMouseButtons`
/// (src/C4Viewport.cpp:1511-1533). These controls are handled by the mouse
/// layer and are never synchronized player commands.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ViewportButton {
    Help,
    PlayerMenu,
    Chat,
}

impl ViewportButton {
    pub const fn command(self) -> u8 {
        match self {
            Self::Help => clonk_engine::COM_HELP,
            Self::PlayerMenu => clonk_engine::COM_PLAYER_MENU,
            Self::Chat => clonk_engine::COM_CHAT,
        }
    }
}

/// Exact 23px viewport-relative cell used by `DrawMouseButtons`. The menu
/// hint deliberately remains in the second slot for keyboard viewports.
pub fn viewport_button_rect(viewport: SurfaceRect, button: ViewportButton) -> SurfaceRect {
    let row = match button {
        ViewportButton::Help => 0,
        ViewportButton::PlayerMenu => 1,
        ViewportButton::Chat => 2,
    };
    SurfaceRect::new(
        viewport.x + viewport.width as i32 - VIEWPORT_BUTTON_SIZE,
        viewport.y + SYMBOL_SIZE + 2 * SYMBOL_BORDER + row * VIEWPORT_BUTTON_SIZE,
        VIEWPORT_BUTTON_SIZE as u32,
        VIEWPORT_BUTTON_SIZE as u32,
    )
}

/// Hit-test for the clickable mouse-viewport stack. Keyboard/gamepad
/// viewports draw the menu hint but C++ does not register a region for it.
pub fn viewport_button_region(
    viewport: SurfaceRect,
    point: clonk_gui::Point,
    show_commands: bool,
    mouse_viewport: bool,
    chat_active: bool,
) -> Option<ViewportButton> {
    if !show_commands || !mouse_viewport {
        return None;
    }
    let px = point.x.floor() as i32;
    let py = point.y.floor() as i32;
    [ViewportButton::Help, ViewportButton::PlayerMenu]
        .into_iter()
        .chain(chat_active.then_some(ViewportButton::Chat))
        .find(|button| {
            let rect = viewport_button_rect(viewport, *button);
            px >= rect.x
                && px < rect.x + rect.width as i32
                && py >= rect.y
                && py < rect.y + rect.height as i32
        })
}

fn draw_viewport_button_cell(
    surface: &mut Surface,
    font: &HudFont<'_>,
    hud: &HudGraphics,
    gui_icons2: Option<&ImageData>,
    cell: SurfaceRect,
    button: ViewportButton,
    menu_key_label: &str,
    show_command_keys: bool,
    gamma: Option<&GammaRamp>,
) {
    if let Some(control) = hud.control.as_ref() {
        draw_scaled_region(
            surface,
            control,
            SurfaceRect::new(0, 100, 64, 64),
            cell,
            gamma,
        );
        let symbol = match button {
            // fctOKCancel phases (0,1) and (1,1).
            ViewportButton::Help => Some(SurfaceRect::new(128, 132, 32, 32)),
            ViewportButton::PlayerMenu => Some(SurfaceRect::new(160, 132, 32, 32)),
            ViewportButton::Chat => None,
        };
        if let Some(symbol) = symbol {
            draw_scaled_region(surface, control, symbol, cell, gamma);
        }
    }
    if button == ViewportButton::Chat {
        if let Some(gui_icons2) = gui_icons2 {
            // Ico_Ex_Chat, extended icon phase 15 in the 4-column sheet.
            draw_scaled_region(
                surface,
                gui_icons2,
                SurfaceRect::new(192, 192, 64, 64),
                cell,
                gamma,
            );
        }
    }
    if button == ViewportButton::PlayerMenu && show_command_keys && !menu_key_label.is_empty() {
        font.draw_with_gamma(
            surface,
            cell.x + cell.width as i32 / 2,
            cell.y + cell.height as i32 - font.line_height() - 2,
            menu_key_label,
            MESSAGE_COLOR,
            TextAlign::Center,
            gamma,
        );
    }
}

/// Draw the post-message viewport control layer from
/// `C4Viewport::DrawOverlay` (src/C4Viewport.cpp:863-880,1511-1533).
#[allow(clippy::too_many_arguments)]
pub fn draw_viewport_buttons_with_gamma(
    surface: &mut Surface,
    font: &HudFont<'_>,
    hud: &HudGraphics,
    gui_icons2: Option<&ImageData>,
    viewport: SurfaceRect,
    show_commands: bool,
    show_command_keys: bool,
    mouse_viewport: bool,
    chat_active: bool,
    menu_key_label: &str,
    gamma: Option<&GammaRamp>,
) {
    if !show_commands {
        return;
    }
    let buttons: &[ViewportButton] = if mouse_viewport {
        if chat_active {
            &[
                ViewportButton::Help,
                ViewportButton::PlayerMenu,
                ViewportButton::Chat,
            ]
        } else {
            &[ViewportButton::Help, ViewportButton::PlayerMenu]
        }
    } else {
        &[ViewportButton::PlayerMenu]
    };
    for &button in buttons {
        draw_viewport_button_cell(
            surface,
            font,
            hud,
            gui_icons2,
            viewport_button_rect(viewport, button),
            button,
            menu_key_label,
            show_command_keys,
            gamma,
        );
    }
}

/// The image cell of a command (src/C4Object.cpp:4050-4068 plus the
/// caller-drawn composites).
pub fn draw_command_image_cell(
    surface: &mut Surface,
    hud: &HudGraphics,
    cell: SurfaceRect,
    image: &CommandImage,
) {
    draw_command_image_cell_with_gamma(surface, hud, cell, image, None);
}

pub fn draw_command_image_cell_with_gamma(
    surface: &mut Surface,
    hud: &HudGraphics,
    cell: SurfaceRect,
    image: &CommandImage,
    gamma: Option<&GammaRamp>,
) {
    match image {
        CommandImage::Picture(picture) => {
            if let Some(picture) = picture {
                draw_image_aspect_fit(surface, picture, cell, gamma);
            }
        }
        CommandImage::Composite { picture, icon } => {
            if let Some(picture) = picture {
                let frac = get_fraction(cell, 85, 85, true, false, false);
                draw_image_aspect_fit(surface, picture, frac, gamma);
            }
            let frac = get_fraction(cell, 85, 85, false, true, false);
            match icon {
                CommandOverlayIcon::Build => {
                    if let Some(build) = hud.build.as_ref() {
                        draw_image_aspect_fit(surface, build, frac, gamma);
                    }
                }
                CommandOverlayIcon::Hand(phase) => {
                    if let Some(hand) = hud.hand.as_ref() {
                        draw_scaled_region_aspect(
                            surface,
                            hand,
                            sheet_cell(hand, *phase),
                            frac,
                            gamma,
                        );
                    }
                }
            }
        }
        CommandImage::Exit => {
            if let Some(exit) = hud.exit.as_ref() {
                draw_image_aspect_fit(surface, exit, cell, gamma);
            }
        }
        CommandImage::BuyMenu { owner_color } | CommandImage::SellMenu { owner_color } => {
            // DrawMenuSymbol (src/C4Menu.cpp:59-70): owner-colored flag at
            // GetFraction(75,75), wealth at (100,50,Left,Bottom), arrow
            // phase 0 buy / 1 sell at (70,70,Right,Center).
            if let Some(flag) = hud.flag.as_ref() {
                let colored = colorize_by_owner(flag, *owner_color);
                draw_image_aspect_fit(
                    surface,
                    &colored,
                    get_fraction(cell, 75, 75, false, false, false),
                    gamma,
                );
            }
            if let Some(wealth) = hud.wealth.as_ref() {
                draw_image_aspect_fit(
                    surface,
                    wealth,
                    get_fraction(cell, 100, 50, false, true, false),
                    gamma,
                );
            }
            if let Some(arrow) = hud.arrow.as_ref() {
                let phase = i32::from(matches!(image, CommandImage::SellMenu { .. }));
                draw_scaled_region_aspect(
                    surface,
                    arrow,
                    sheet_cell(arrow, phase),
                    get_fraction(cell, 70, 70, true, false, true),
                    gamma,
                );
            }
        }
        CommandImage::InfoMenu { picture } => {
            if let Some(picture) = picture {
                draw_image_aspect_fit(
                    surface,
                    picture,
                    get_fraction(cell, 85, 85, false, true, false),
                    gamma,
                );
            }
            if let Some(control) = hud.control.as_ref() {
                draw_scaled_region_aspect(
                    surface,
                    control,
                    SurfaceRect::new(128, 132, 32, 32),
                    get_fraction(cell, 85, 85, true, false, false),
                    gamma,
                );
            }
        }
    }
}

/// The DrawCursorInfo command rows (src/C4Viewport.cpp:947-962): bottom bar
/// consumed right-to-left, side strip consumed bottom-to-top, both in
/// `iSize = 2*C4SymbolSize/3` squares via C4Facet::TruncateSection
/// (src/C4Facet.cpp:182-215). Gated by the caller on
/// Config.Graphics.ShowCommands.
pub fn draw_commands(
    surface: &mut Surface,
    key_font: &HudFont<'_>,
    hud: &HudGraphics,
    viewport: SurfaceRect,
    icons: &[CommandIcon],
    show_command_keys: bool,
) {
    draw_commands_with_gamma(
        surface,
        key_font,
        hud,
        viewport,
        icons,
        show_command_keys,
        0,
        0,
        None,
    );
}

/// C4RegionList hit test for the paired key/image regions emitted by
/// C4Object::DrawCommand. Later-drawn icons win if malformed data overlaps,
/// matching C4RegionList::Add's prepend order (C4Object.cpp:4033-4092;
/// C4Region.cpp:49-57,87-94).
pub fn command_region_hit(
    viewport: SurfaceRect,
    point: clonk_gui::Point,
    icons: &[CommandIcon],
) -> Option<(usize, SurfaceRect)> {
    if viewport.height as i32 <= SYMBOL_SIZE {
        return None;
    }
    let px = point.x.floor() as i32;
    let py = point.y.floor() as i32;
    let size = 2 * SYMBOL_SIZE / 3;
    let size2 = 2 * size;
    let bottom_y = viewport.y + viewport.height as i32 - size;
    let mut bottom_wdt = viewport.width as i32;
    let side_x = viewport.x + viewport.width as i32 - size2;
    let mut side_hgt = viewport.height as i32 - size - 5;
    let mut found = None;

    for (index, icon) in icons.iter().enumerate() {
        let pair = if icon.side {
            if side_hgt < size || size2 > viewport.width as i32 {
                continue;
            }
            side_hgt -= size;
            SurfaceRect::new(side_x, viewport.y + side_hgt, size2 as u32, size as u32)
        } else {
            if bottom_wdt < size {
                continue;
            }
            bottom_wdt -= size;
            if bottom_wdt < size {
                continue;
            }
            bottom_wdt -= size;
            SurfaceRect::new(viewport.x + bottom_wdt, bottom_y, size2 as u32, size as u32)
        };
        if px >= pair.x
            && px < pair.x + pair.width as i32
            && py >= pair.y
            && py < pair.y + pair.height as i32
        {
            found = Some((index, pair));
        }
    }
    found
}

pub fn command_region_index(
    viewport: SurfaceRect,
    point: clonk_gui::Point,
    icons: &[CommandIcon],
) -> Option<usize> {
    command_region_hit(viewport, point, icons).map(|(index, _)| index)
}

#[allow(clippy::too_many_arguments)]
pub(crate) fn draw_commands_with_gamma(
    surface: &mut Surface,
    key_font: &HudFont<'_>,
    hud: &HudGraphics,
    viewport: SurfaceRect,
    icons: &[CommandIcon],
    show_command_keys: bool,
    flash_command: i32,
    frame: u64,
    gamma: Option<&GammaRamp>,
) {
    // `if (cgo.Hgt > C4SymbolSize)` (src/C4Viewport.cpp:950).
    if viewport.height as i32 <= SYMBOL_SIZE {
        return;
    }
    let size = 2 * SYMBOL_SIZE / 3;
    let size2 = 2 * size;

    // Primary area (bottom, src/C4Viewport.cpp:956).
    let bottom_y = viewport.y + viewport.height as i32 - size;
    let mut bottom_wdt = viewport.width as i32;
    // Secondary area (side, src/C4Viewport.cpp:958).
    let side_x = viewport.x + viewport.width as i32 - size2;
    let mut side_hgt = viewport.height as i32 - size - 5;

    for icon in icons {
        let (key_cell, image_cell) = if icon.side {
            // TruncateSection(C4FCT_Bottom|C4FCT_Half) -> 2*iSize x iSize
            // slice off the strip bottom; then (C4FCT_Left) splits it.
            if side_hgt < size || size2 > viewport.width as i32 {
                continue;
            }
            side_hgt -= size;
            let pair_y = viewport.y + side_hgt;
            (
                SurfaceRect::new(side_x, pair_y, size as u32, size as u32),
                SurfaceRect::new(side_x + size, pair_y, size as u32, size as u32),
            )
        } else {
            // Two TruncateSection(C4FCT_Right) squares: image cell first
            // (rightmost), key cell next (src/C4Object.cpp:4043-4048).
            if bottom_wdt < size {
                continue;
            }
            bottom_wdt -= size;
            let image_x = viewport.x + bottom_wdt;
            if bottom_wdt < size {
                continue;
            }
            bottom_wdt -= size;
            let key_x = viewport.x + bottom_wdt;
            (
                SurfaceRect::new(key_x, bottom_y, size as u32, size as u32),
                SurfaceRect::new(image_x, bottom_y, size as u32, size as u32),
            )
        };

        draw_command_image_cell_with_gamma(surface, hud, image_cell, &icon.image, gamma);
        // C4Object::DrawCommand keeps the image and region present, but the
        // exact FlashCom key cell blinks off for Tick35 0..=15.
        if i32::from(icon.com) != flash_command || frame % 35 > 15 {
            draw_command_key_cell(
                surface,
                key_font,
                hud,
                key_cell,
                icon.com,
                &icon.key_label,
                show_command_keys,
                gamma,
            );
        }
    }
}

/// `C4Object::DrawEnergy` → `C4Facet::DrawEnergyLevelEx`
/// (src/C4Viewport.cpp:921-945, src/C4Facet.cpp:334-389): the vertical
/// bar left of the viewport. `EnergyBars.png` is a 6x3 cell grid — column
/// `bar_idx*2` filled, `+1` empty; rows top cap / middle tile / bottom cap
/// (src/C4GraphicsResource.cpp:236-241).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum HudBarKind {
    Energy = 0,
    Magic = 1,
    Breath = 2,
}

/// `C4Facet::DrawEnergyLevelEx`. `slot` is the compact left-to-right position
/// after applying C++'s optional-bar gates; `kind` selects the corresponding
/// filled/empty pair in EnergyBars.png.
pub fn draw_level_bar(
    surface: &mut Surface,
    hud: &HudGraphics,
    viewport: SurfaceRect,
    kind: HudBarKind,
    slot: u32,
    level: i32,
    range: i32,
    show_portraits: bool,
) {
    draw_level_bar_with_gamma(
        surface,
        hud,
        viewport,
        kind,
        slot,
        level,
        range,
        show_portraits,
        None,
    );
}

#[allow(clippy::too_many_arguments)]
pub(crate) fn draw_level_bar_with_gamma(
    surface: &mut Surface,
    hud: &HudGraphics,
    viewport: SurfaceRect,
    kind: HudBarKind,
    slot: u32,
    level: i32,
    range: i32,
    show_portraits: bool,
    gamma: Option<&GammaRamp>,
) {
    draw_bar(
        surface,
        hud,
        viewport,
        kind,
        slot,
        show_portraits,
        |height| level_bar_y(height, level, range),
        gamma,
    );
}

/// Native `BoundBy(level, 0, range) * height / max(range, 1)` arithmetic.
/// LegacyClonk performs the intermediate operations in signed 32-bit values;
/// wrapping keeps script-set overflow deterministic while matching every
/// defined C++ input.
fn level_bar_y(height: i32, level: i32, range: i32) -> i32 {
    let bounded = if level < 0 {
        0
    } else if level > range {
        range
    } else {
        level
    };
    height.wrapping_sub(bounded.wrapping_mul(height) / range.max(1))
}

fn draw_bar(
    surface: &mut Surface,
    hud: &HudGraphics,
    viewport: SurfaceRect,
    kind: HudBarKind,
    slot: u32,
    show_portraits: bool,
    y_bar_for_height: impl FnOnce(i32) -> i32,
    gamma: Option<&GammaRamp>,
) {
    let Some(bars) = hud.energy_bars.as_ref() else {
        return;
    };
    let cell_w = bars.width() / 6;
    let cell_h = bars.height() / 3;
    if cell_w == 0 || cell_h == 0 {
        return;
    }
    let vp_height = viewport.height as i32;
    // Gate: cgo.Hgt > 2*C4SymbolSize + 2*C4SymbolBorder
    // (src/C4Viewport.cpp:922).
    if vp_height <= 2 * SYMBOL_SIZE + 2 * SYMBOL_BORDER {
        return;
    }
    // iYOff = Config.Graphics.ShowPortraits ? 10 : 0
    // (src/C4Viewport.cpp:927).
    let y_off = if show_portraits { 10 } else { 0 };
    let x = viewport.x + SYMBOL_BORDER + slot as i32 * (cell_w as i32 + 1);
    let y = viewport.y + SYMBOL_SIZE + 2 * SYMBOL_BORDER + y_off;
    let height = vp_height - 3 * SYMBOL_BORDER - 2 * SYMBOL_SIZE - y_off;
    if height <= 0 {
        return;
    }

    // yBar = Hgt - level*Hgt/range (src/C4Facet.cpp:339).
    let y_bar = y_bar_for_height(height);
    let cell_h_i = cell_h as i32;
    for row in 0..height {
        let (vidx, dy) = if row >= height - cell_h_i {
            (2, row + cell_h_i - height)
        } else if row < cell_h_i {
            (0, row)
        } else {
            (1, row % cell_h_i)
        };
        let column = kind as u32 * 2 + u32::from(row < y_bar);
        draw_hud_image_strip(
            surface,
            x,
            y + row,
            bars,
            column * cell_w,
            vidx as u32 * cell_h + dy as u32,
            cell_w,
            1,
            gamma,
        );
    }
}

/// `C4Viewport::DrawPlayerStartup` (src/C4Viewport.cpp:1446-1476):
/// keyboard/gamepad graphic, optional mouse symbol, and player name.
pub fn draw_player_startup(
    surface: &mut Surface,
    font: &HudFont<'_>,
    hud: &HudGraphics,
    viewport: SurfaceRect,
    player_name: &str,
    player_color: Color,
    control_set: i32,
    mouse_control: bool,
) {
    draw_player_startup_with_gamma(
        surface,
        font,
        hud,
        viewport,
        player_name,
        player_color,
        control_set,
        mouse_control,
        None,
    );
}

#[allow(clippy::too_many_arguments)]
pub(crate) fn draw_player_startup_with_gamma(
    surface: &mut Surface,
    font: &HudFont<'_>,
    hud: &HudGraphics,
    viewport: SurfaceRect,
    player_name: &str,
    player_color: Color,
    control_set: i32,
    mouse_control: bool,
    gamma: Option<&GammaRamp>,
) {
    let (cell_w, cell_h) = KEYBOARD_CELL;
    let dest_x = viewport.x + (viewport.width as i32 - cell_w as i32) / 2;
    let dest_y = viewport.y + viewport.height as i32 * 2 / 3 + DRAW_MESSAGE_OFFSET;
    let mut name_height_off = 0;

    // MouseControl is independent of the keyboard/gamepad branch and draws
    // first, so the later control facet wins in their overlap.
    if mouse_control {
        if let Some(control) = hud.control.as_ref() {
            let (src_x, src_y, src_w, src_h) = MOUSE_SOURCE;
            draw_hud_image_strip(
                surface,
                dest_x + 55,
                dest_y - 10,
                control,
                src_x,
                src_y,
                src_w,
                src_h,
                gamma,
            );
        }
    }

    if (0..=3).contains(&control_set) {
        if let Some(control) = hud.control.as_ref() {
            draw_hud_image_strip(
                surface,
                dest_x,
                dest_y,
                control,
                control_set as u32 * cell_w,
                0,
                cell_w,
                cell_h,
                gamma,
            );
        }
        name_height_off = cell_h as i32;
    } else if (4..=7).contains(&control_set) {
        if let Some(gamepad) = hud.gamepad.as_ref() {
            let gamepad_height = gamepad.height();
            draw_hud_image_strip(
                surface,
                dest_x,
                dest_y,
                gamepad,
                (control_set as u32 - 4) * GAMEPAD_CELL_WIDTH,
                0,
                GAMEPAD_CELL_WIDTH,
                gamepad_height,
                gamma,
            );
            name_height_off = gamepad_height as i32;
        }
    }

    // Name in ColorDw | 0xff000000, centered (src/C4Viewport.cpp:1471-1475).
    font.draw_with_gamma(
        surface,
        viewport.x + viewport.width as i32 / 2,
        dest_y + name_height_off,
        player_name,
        Color::opaque(player_color.r, player_color.g, player_color.b),
        TextAlign::Center,
        gamma,
    );
}

/// `C4Viewport::DrawPlayerControls` (src/C4Viewport.cpp:1394-1441): the
/// tutorial-selected command-key grid, including optional and blinking labels.
#[allow(clippy::too_many_arguments)]
pub fn draw_player_controls(
    surface: &mut Surface,
    regular_font: &HudFont<'_>,
    tiny_font: &HudFont<'_>,
    hud: &HudGraphics,
    viewport: SurfaceRect,
    show_control: i32,
    show_control_position: i32,
    last_com: i32,
    key_labels: &[String],
    frame: u64,
) {
    draw_player_controls_with_gamma(
        surface,
        regular_font,
        tiny_font,
        hud,
        viewport,
        show_control,
        show_control_position,
        last_com,
        key_labels,
        frame,
        None,
    );
}

#[allow(clippy::too_many_arguments)]
pub(crate) fn draw_player_controls_with_gamma(
    surface: &mut Surface,
    regular_font: &HudFont<'_>,
    tiny_font: &HudFont<'_>,
    hud: &HudGraphics,
    viewport: SurfaceRect,
    show_control: i32,
    show_control_position: i32,
    last_com: i32,
    key_labels: &[String],
    frame: u64,
    gamma: Option<&GammaRamp>,
) {
    if show_control == 0 {
        return;
    }
    let size = ((viewport.width as i32) / 3).min(7 * viewport.height as i32 / 24);
    if size <= 0 {
        return;
    }
    let (tx, ty) = match show_control_position {
        1 => (
            viewport.x + viewport.width as i32 * 3 / 4 - size / 2,
            viewport.y + viewport.height as i32 / 2 - size / 2,
        ),
        2 => (
            viewport.x + viewport.width as i32 / 4 - size / 2,
            viewport.y + viewport.height as i32 / 2 - size / 2,
        ),
        3 => (
            viewport.x + viewport.width as i32 / 4 - size / 2,
            viewport.y + 15,
        ),
        4 => (
            viewport.x + viewport.width as i32 * 3 / 4 - size / 2,
            viewport.y + 15,
        ),
        _ => (
            viewport.x + viewport.width as i32 / 2 - size / 2,
            viewport.y + 15,
        ),
    };
    let cell_width = size / 3;
    let cell_height = size / 4;
    if cell_width <= 0 || cell_height <= 0 {
        return;
    }
    let last_control = com_control_index(last_com).0;
    let tick35 = (frame % 35) as i32;

    for control in 0..10 {
        if show_control & (1 << control) == 0 {
            continue;
        }
        let mut show_text = show_control & (1 << (control + 10)) != 0;
        if show_control & (1 << (control + 20)) != 0 && tick35 > 18 {
            show_text = false;
        }
        let cell = SurfaceRect::new(
            tx + cell_width * (control % 3),
            ty + cell_height * (control / 3),
            cell_width as u32,
            cell_height as u32,
        );
        if let Some(control_sheet) = hud.control.as_ref() {
            let pressed = i32::from(last_control == control);
            draw_scaled_region(
                surface,
                control_sheet,
                SurfaceRect::new(64 * pressed, 100, 64, 64),
                cell,
                gamma,
            );
            draw_scaled_region_aspect(
                surface,
                control_sheet,
                SurfaceRect::new(32 * control, 36, 32, 32),
                cell,
                gamma,
            );
        }
        if show_text {
            if let Some(label) = key_labels
                .get(control as usize)
                .filter(|label| !label.is_empty())
            {
                let font = if cell_height <= SYMBOL_SIZE {
                    regular_font
                } else {
                    tiny_font
                };
                font.draw_with_gamma(
                    surface,
                    cell.x + cell.width as i32 / 2,
                    cell.y + cell.height as i32 - font.line_height() - 2,
                    label,
                    MESSAGE_COLOR,
                    TextAlign::Center,
                    gamma,
                );
            }
        }
    }
}

/// `C4MessageBoard::Draw` (src/C4MessageBoard.cpp:243-306): an opaque tiled
/// `fctBackground` output strip followed by the exact line-selection,
/// vertical-fader and text-alpha loop.
pub fn draw_message_board(
    surface: &mut Surface,
    font: &HudFont<'_>,
    hud: &HudGraphics,
    board: &MessageBoardOverlay,
) {
    draw_message_board_with_gamma(surface, font, hud, board, None);
}

/// Draws one physical line already admitted to `C4LogBuffer`.
pub(crate) fn draw_message_board_with_gamma(
    surface: &mut Surface,
    font: &HudFont<'_>,
    hud: &HudGraphics,
    board: &MessageBoardOverlay,
    gamma: Option<&GammaRamp>,
) {
    let line_height = font.line_height().max(1);
    let width = surface.width() as i32;
    let height = board.output_height(line_height);
    let y = surface.height() as i32 - height;
    let Some(background) = hud.background.as_ref() else {
        return;
    };
    blit_tile(surface, background, 0, y, width, height, 0, -y, gamma);

    if board.mode == MessageBoardMode::Hidden && !board.type_in {
        return;
    }

    let mut message_fader = if board.mode == MessageBoardMode::Continuous {
        0
    } else {
        MESSAGE_BOARD_MAX_FADING_LINES
    };
    let maximum_screen_fader = MESSAGE_BOARD_MAX_FADING_LINES.saturating_mul(line_height);
    let screen_fader = if board.screen_fader >= maximum_screen_fader {
        message_fader = 0;
        maximum_screen_fader
    } else {
        board.screen_fader
    };
    let lines = if board.mode == MessageBoardMode::Continuous {
        board.line_count.max(2)
    } else {
        2
    };
    let first_message = -message_fader - lines;
    for message_index in first_message..0 {
        let log_index = message_index.saturating_sub(board.back_scroll);
        if log_index >= 0 {
            break;
        }
        let Some(from_end) = log_index
            .checked_neg()
            .and_then(|index| usize::try_from(index).ok())
        else {
            continue;
        };
        let Some(line) = board
            .log_lines
            .len()
            .checked_sub(from_end)
            .and_then(|index| board.log_lines.get(index))
            .filter(|line| !line.is_empty())
        else {
            continue;
        };

        // `iMsgY = cgo.Y + (iMsg + (iLines - 1)) * iLineHgt + Fader`.
        let message_y = y
            .saturating_add(
                message_index
                    .saturating_add(lines.saturating_sub(1))
                    .saturating_mul(line_height),
            )
            .saturating_add(board.fader);
        let alpha = if message_y < y {
            // Preserve C++ integer-operation order:
            // `(distance * 256 / max(iMsgFader, 1) / iLineHgt)`.
            let distance = y
                .saturating_sub(message_y)
                .saturating_add(screen_fader.max(0));
            let fade = i64::from(distance).saturating_mul(256)
                / i64::from(message_fader.max(1))
                / i64::from(line_height);
            255_i64.saturating_sub(fade.clamp(0, 255)) as u8
        } else {
            255
        };
        font.draw_markup_with_gamma(
            surface,
            0,
            message_y,
            line,
            Color::new(MESSAGE_COLOR.r, MESSAGE_COLOR.g, MESSAGE_COLOR.b, alpha),
            TextAlign::Left,
            gamma,
        );
    }
}

/// `C4LogBuffer::AppendLines` wraps each message to `LBWidth` before adding
/// its physical lines to message-board history. Continuations use the
/// buffer's two-space indent (src/C4LogBuf.cpp:174-254).
fn message_board_tail_line(font: &HudFont<'_>, line: &str, width: i32) -> String {
    message_board_physical_lines(font, line, width)
        .pop()
        .unwrap_or_default()
}

pub(crate) fn message_board_physical_lines(
    font: &HudFont<'_>,
    line: &str,
    width: i32,
) -> Vec<String> {
    if line.is_empty() {
        return vec![String::new()];
    }
    let continuation_width = width.saturating_sub(font.text_width_markup("  "));
    let mut lines = Vec::new();
    for paragraph in line
        .split(['\r', '\n', '|'])
        .filter(|part| !part.is_empty())
    {
        let first_pass =
            crate::message_dialog::break_hud_message_max_lines(font, paragraph, width, 1);
        let Some((first, remainder)) = first_pass.split_once('\n') else {
            lines.push(first_pass);
            continue;
        };
        lines.push(first.to_string());
        lines.extend(
            crate::message_dialog::break_hud_message(font, remainder, continuation_width)
                .split('\n')
                .filter(|line| !line.is_empty())
                .map(|line| format!("  {line}")),
        );
    }
    lines
}

#[cfg(test)]
mod tests {
    use super::*;
    use clonk_graphics::{FontMetrics, GpuBlend, GpuCommand, GpuSampler, PixelFormat, TextFont};

    struct MarkerFont;

    impl TextFont for MarkerFont {
        fn measure_text(&self, _text: &str, _font_size: f32) -> FontMetrics {
            FontMetrics {
                width: 1.0,
                height: 1.0,
                lines: 1,
            }
        }

        fn draw_text(
            &self,
            surface: &mut Surface,
            origin_x: f32,
            origin_y: f32,
            _text: &str,
            _font_size: f32,
            color: Color,
        ) {
            if origin_x >= 0.0 && origin_y >= 0.0 {
                let _ = surface.set_pixel(origin_x as u32, origin_y as u32, color);
            }
        }
    }

    struct FixedWidthMarkerFont;

    impl TextFont for FixedWidthMarkerFont {
        fn measure_text(&self, text: &str, _font_size: f32) -> FontMetrics {
            FontMetrics {
                width: (text.chars().count() * 4) as f32,
                height: 6.0,
                lines: 1,
            }
        }

        fn draw_text(
            &self,
            surface: &mut Surface,
            origin_x: f32,
            origin_y: f32,
            text: &str,
            _font_size: f32,
            color: Color,
        ) {
            if origin_x >= 0.0 && origin_y >= 0.0 {
                for x in 0..text.chars().count() * 4 {
                    let _ = surface.set_pixel(origin_x as u32 + x as u32, origin_y as u32, color);
                }
            }
        }
    }

    fn solid_image(width: u32, height: u32, color: [u8; 4]) -> ImageData {
        let pixels = color
            .iter()
            .copied()
            .cycle()
            .take((width * height * 4) as usize)
            .collect::<Vec<u8>>();
        ImageData::new(width, height, pixels)
    }

    fn horizontal_cell_strip(cell: u32, colors: &[[u8; 4]]) -> ImageData {
        let mut pixels = Vec::with_capacity((cell * cell * colors.len() as u32 * 4) as usize);
        for _ in 0..cell {
            for color in colors {
                pixels.extend(std::iter::repeat_n(*color, cell as usize).flatten());
            }
        }
        ImageData::new(cell * colors.len() as u32, cell, pixels)
    }

    fn surface(width: u32, height: u32) -> Surface {
        let mut surface = Surface::new(width, height, PixelFormat::Rgba8888);
        surface.fill(Color::opaque(0, 0, 0));
        surface
    }

    #[test]
    fn gpu_capture_lowers_scaled_command_region_to_nearest_gamma_quad() {
        let image = solid_image(4, 2, [64, 128, 192, 128]);
        let gamma = GammaRamp::standard();
        let mut surface = surface(8, 4);
        surface.begin_gpu_scene_capture();

        draw_scaled_region(
            &mut surface,
            &image,
            SurfaceRect::new(1, 0, 2, 2),
            SurfaceRect::new(2, 1, 4, 2),
            Some(&gamma),
        );

        let scene = surface
            .take_gpu_scene_capture()
            .expect("GPU capture remains active")
            .into_scene([8, 4], Color::transparent(), &gamma);
        assert_eq!(scene.textures.len(), 1);
        assert_eq!(scene.commands.len(), 1);
        let GpuCommand::Quad {
            vertices,
            sampler,
            blend,
            gamma,
            ..
        } = &scene.commands[0]
        else {
            panic!("scaled command region did not lower to a textured quad");
        };
        assert_eq!(*sampler, GpuSampler::Nearest);
        assert_eq!(*blend, GpuBlend::Normal);
        assert!(*gamma);
        assert_eq!(vertices[0].position, [2.0, 1.0, 1.0]);
        assert_eq!(vertices[3].position, [6.0, 3.0, 1.0]);
        assert_eq!(vertices[0].uv, [0.25, 0.0]);
        assert_eq!(vertices[3].uv, [0.75, 1.0]);
    }

    #[test]
    fn scaled_command_region_preserves_legacy_cpu_truncation() {
        let image = ImageData::new(1, 1, vec![1, 3, 5, 128]);
        let mut surface = Surface::new(1, 1, PixelFormat::Rgba8888);
        draw_scaled_region(
            &mut surface,
            &image,
            SurfaceRect::new(0, 0, 1, 1),
            SurfaceRect::new(0, 0, 1, 1),
            None,
        );
        assert_eq!(surface.get_pixel(0, 0), Some(Color::new(0, 1, 2, 128)));
    }

    fn paint_rect(
        pixels: &mut [u8],
        width: u32,
        x: u32,
        y: u32,
        rect_width: u32,
        rect_height: u32,
        color: [u8; 4],
    ) {
        for py in y..y + rect_height {
            for px in x..x + rect_width {
                let index = ((py * width + px) * 4) as usize;
                pixels[index..index + 4].copy_from_slice(&color);
            }
        }
    }

    fn bitmap_font() -> clonk_graphics::BitmapFont {
        clonk_graphics::BitmapFont::new()
    }

    fn startup_control_sheet() -> ImageData {
        let width = 400u32;
        let height = 164u32;
        let mut pixels = vec![0u8; (width * height * 4) as usize];
        let keyboard = [
            [200, 10, 10, 255],
            [10, 200, 10, 255],
            [10, 10, 200, 255],
            [200, 200, 10, 255],
        ];
        for (phase, color) in keyboard.into_iter().enumerate() {
            let left = phase as u32 * KEYBOARD_CELL.0;
            for y in 0..KEYBOARD_CELL.1 {
                for x in left..left + KEYBOARD_CELL.0 {
                    let index = ((y * width + x) * 4) as usize;
                    pixels[index..index + 4].copy_from_slice(&color);
                }
            }
        }
        for y in MOUSE_SOURCE.1..MOUSE_SOURCE.1 + MOUSE_SOURCE.3 {
            for x in MOUSE_SOURCE.0..MOUSE_SOURCE.0 + MOUSE_SOURCE.2 {
                let index = ((y * width + x) * 4) as usize;
                pixels[index..index + 4].copy_from_slice(&[200, 10, 200, 255]);
            }
        }
        ImageData::new(width, height, pixels)
    }

    fn startup_gamepad_sheet(height: u32) -> ImageData {
        let colors = [
            [10, 100, 100, 255],
            [100, 10, 100, 255],
            [100, 100, 10, 255],
            [180, 80, 20, 255],
        ];
        let width = GAMEPAD_CELL_WIDTH * colors.len() as u32;
        let mut pixels = Vec::with_capacity((width * height * 4) as usize);
        for _ in 0..height {
            for color in colors {
                pixels.extend(std::iter::repeat_n(color, GAMEPAD_CELL_WIDTH as usize).flatten());
            }
        }
        ImageData::new(width, height, pixels)
    }

    #[test]
    fn player_startup_uses_keyboard_mouse_and_gamepad_facets() {
        let viewport = SurfaceRect::new(10, 20, 240, 180);
        let dest_x = 90;
        let dest_y = 105;
        let text_x = 130;
        let text_color = Color::opaque(7, 8, 9);
        let hud = HudGraphics {
            control: Some(startup_control_sheet()),
            // A non-shipped height proves that the name offset comes from the
            // loaded fctGamepad height rather than the keyboard constant.
            gamepad: Some(startup_gamepad_sheet(20)),
            ..HudGraphics::default()
        };
        let marker = MarkerFont;
        let font = HudFont::Fallback(&marker);
        let render = |control_set, mouse_control| {
            let mut target = surface(280, 220);
            draw_player_startup(
                &mut target,
                &font,
                &hud,
                viewport,
                "P",
                text_color,
                control_set,
                mouse_control,
            );
            target
        };

        let keyboard_colors = [
            Color::opaque(200, 10, 10),
            Color::opaque(10, 200, 10),
            Color::opaque(10, 10, 200),
            Color::opaque(200, 200, 10),
        ];
        for (control_set, expected) in keyboard_colors.into_iter().enumerate() {
            let target = render(control_set as i32, false);
            assert_eq!(target.get_pixel(dest_x, dest_y), Some(expected));
            assert_eq!(target.get_pixel(text_x, dest_y + 36), Some(text_color));
        }

        let with_mouse = render(0, true);
        assert_eq!(
            with_mouse.get_pixel(dest_x + 55, dest_y - 10),
            Some(Color::opaque(200, 10, 200)),
            "fctMouse uses the +55/-10 destination offset"
        );
        assert_eq!(
            with_mouse.get_pixel(dest_x + 55, dest_y),
            Some(keyboard_colors[0]),
            "the keyboard facet draws after and over the mouse overlap"
        );

        let gamepad_colors = [
            Color::opaque(10, 100, 100),
            Color::opaque(100, 10, 100),
            Color::opaque(100, 100, 10),
            Color::opaque(180, 80, 20),
        ];
        for (phase, expected) in gamepad_colors.into_iter().enumerate() {
            let target = render(phase as i32 + 4, false);
            assert_eq!(target.get_pixel(dest_x, dest_y), Some(expected));
            assert_eq!(
                target.get_pixel(text_x, dest_y + 20),
                Some(text_color),
                "name follows the loaded gamepad height"
            );
        }
    }

    #[test]
    fn hud_clonk_character_advance_uses_the_rendered_missing_glyph() {
        let mut font = clonk_graphics::clonk_font::ClonkFont::new(3);
        font.set_missing_glyph(clonk_graphics::clonk_font::GlyphCell {
            width: 5,
            pixels: vec![Color::opaque(255, 255, 255); 5 * 4],
        });
        let font = HudFont::Clonk(&font);

        assert_eq!(font.character_advance('☃'), 4);
        assert_eq!(font.character_advance('\t'), 0);
    }

    /// Control.png stand-in sized like the C++ sheet regions
    /// (src/C4GraphicsResource.cpp:200-205): key cap cell (0,100,64,64)
    /// solid blue, fctCommand single row (y=36) transparent, double row
    /// (y=68) solid green.
    fn control_sheet() -> ImageData {
        let width = 512u32;
        let height = 164u32;
        let mut pixels = vec![0u8; (width * height * 4) as usize];
        for y in 0..height {
            for x in 0..width {
                let idx = ((y * width + x) * 4) as usize;
                let color: [u8; 4] = if (100..164).contains(&y) && x < 64 {
                    [10, 10, 200, 255] // fctKey phase 0
                } else if (68..100).contains(&y) {
                    [10, 200, 10, 255] // fctCommand double row
                } else {
                    [0, 0, 0, 0]
                };
                pixels[idx..idx + 4].copy_from_slice(&color);
            }
        }
        ImageData::new(width, height, pixels)
    }

    fn viewport_button_control_sheet() -> ImageData {
        let width = 256;
        let height = 164;
        let mut pixels = vec![0; (width * height * 4) as usize];
        paint_rect(&mut pixels, width, 0, 100, 64, 64, [10, 10, 200, 255]);
        // Leave each symbol's left half transparent so the blue key cap
        // proves the base facet was drawn before the symbol facet.
        paint_rect(&mut pixels, width, 144, 132, 16, 32, [200, 20, 20, 255]);
        paint_rect(&mut pixels, width, 176, 132, 16, 32, [20, 200, 20, 255]);
        ImageData::new(width, height, pixels)
    }

    fn viewport_button_gui_icons2_sheet() -> ImageData {
        let width = 256;
        let height = 256;
        let mut pixels = vec![0; (width * height * 4) as usize];
        paint_rect(&mut pixels, width, 224, 192, 32, 64, [200, 200, 20, 255]);
        ImageData::new(width, height, pixels)
    }

    #[test]
    fn viewport_button_regions_match_cpp_slots_and_mouse_gating() {
        let viewport = SurfaceRect::new(10, 20, 200, 150);
        assert_eq!(
            viewport_button_rect(viewport, ViewportButton::Help),
            SurfaceRect::new(187, 65, 23, 23)
        );
        assert_eq!(
            viewport_button_rect(viewport, ViewportButton::PlayerMenu),
            SurfaceRect::new(187, 88, 23, 23)
        );
        assert_eq!(
            viewport_button_rect(viewport, ViewportButton::Chat),
            SurfaceRect::new(187, 111, 23, 23)
        );

        let at = |x, y, mouse, chat| {
            viewport_button_region(viewport, clonk_gui::Point::new(x, y), true, mouse, chat)
        };
        assert_eq!(at(187.0, 65.0, true, false), Some(ViewportButton::Help));
        assert_eq!(
            at(209.9, 110.9, true, false),
            Some(ViewportButton::PlayerMenu),
            "the final integer pixel remains inside the half-open cell"
        );
        assert_eq!(at(210.0, 88.0, true, false), None);
        assert_eq!(at(187.0, 111.0, true, false), None);
        assert_eq!(at(187.0, 111.0, true, true), Some(ViewportButton::Chat));
        assert_eq!(at(187.0, 88.0, false, true), None);
        assert_eq!(
            viewport_button_region(
                viewport,
                clonk_gui::Point::new(187.0, 65.0),
                false,
                true,
                true,
            ),
            None
        );
    }

    #[test]
    fn viewport_button_stack_uses_cpp_facets_and_keyboard_menu_only() {
        let viewport = SurfaceRect::new(10, 20, 200, 150);
        let hud = HudGraphics {
            control: Some(viewport_button_control_sheet()),
            ..HudGraphics::default()
        };
        let gui_icons2 = viewport_button_gui_icons2_sheet();
        let marker = MarkerFont;
        let font = HudFont::Fallback(&marker);
        let render = |show_commands, mouse_viewport, chat_active| {
            let mut target = surface(220, 180);
            draw_viewport_buttons_with_gamma(
                &mut target,
                &font,
                &hud,
                Some(&gui_icons2),
                viewport,
                show_commands,
                true,
                mouse_viewport,
                chat_active,
                "M",
                None,
            );
            target
        };

        let mouse = render(true, true, true);
        for y in [65, 88, 111] {
            assert_eq!(
                mouse.get_pixel(189, y + 11),
                Some(Color::opaque(10, 10, 200)),
                "transparent symbol half preserves the key cap in row {y}"
            );
        }
        assert_eq!(mouse.get_pixel(207, 76), Some(Color::opaque(200, 20, 20)));
        assert_eq!(mouse.get_pixel(207, 99), Some(Color::opaque(20, 200, 20)));
        assert_eq!(mouse.get_pixel(207, 122), Some(Color::opaque(200, 200, 20)));

        let keyboard = render(true, false, true);
        assert_eq!(keyboard.get_pixel(207, 76), Some(Color::opaque(0, 0, 0)));
        assert_eq!(
            keyboard.get_pixel(207, 99),
            Some(Color::opaque(20, 200, 20))
        );
        assert_eq!(keyboard.get_pixel(207, 122), Some(Color::opaque(0, 0, 0)));
        assert_eq!(
            keyboard.get_pixel(198, 108),
            Some(MESSAGE_COLOR),
            "the keyboard PlayerMenu variant includes its configured key hint"
        );

        let hidden = render(false, true, true);
        assert_eq!(hidden.get_pixel(207, 76), Some(Color::opaque(0, 0, 0)));
        assert_eq!(hidden.get_pixel(207, 99), Some(Color::opaque(0, 0, 0)));
        assert_eq!(hidden.get_pixel(207, 122), Some(Color::opaque(0, 0, 0)));
    }

    /// Transparent command symbols over blue unpressed and red pressed key
    /// caps, using the exact Control.png source rectangles.
    fn tutorial_control_sheet() -> ImageData {
        let width = 320u32;
        let height = 164u32;
        let mut pixels = vec![0u8; (width * height * 4) as usize];
        for y in 100..164 {
            for x in 0..128 {
                let idx = ((y * width + x) * 4) as usize;
                let color: [u8; 4] = if x < 64 {
                    [10, 10, 200, 255]
                } else {
                    [200, 10, 10, 255]
                };
                pixels[idx..idx + 4].copy_from_slice(&color);
            }
        }
        ImageData::new(width, height, pixels)
    }

    #[test]
    fn player_controls_follow_cpp_mask_grid_and_pressed_phase() {
        // DrawPlayerControls chooses size=min(Wdt/3,7*Hgt/24), position 2's
        // left/middle origin, and a 3x4 grid (src/C4Viewport.cpp:1397-1439).
        // DrawControlKey selects fctKey phase 1 for LastCom's control
        // (src/C4ObjectCom.cpp:946-958).
        let mut target = surface(300, 240);
        let hud = HudGraphics {
            control: Some(tutorial_control_sheet()),
            ..HudGraphics::default()
        };
        let font = bitmap_font();
        let font = HudFont::Fallback(&font);
        draw_player_controls(
            &mut target,
            &font,
            &font,
            &hud,
            SurfaceRect::new(0, 0, 300, 240),
            (1 << 0) | (1 << 3),
            2,
            12, // COM_CursorLeft -> CON_CursorLeft
            &[],
            0,
        );

        // size=70, origin=(40,85), cell=23x17. Control 0 is pressed/red;
        // control 3 is present but unpressed/blue; control 1 is absent.
        assert_eq!(target.get_pixel(45, 90), Some(Color::opaque(200, 10, 10)));
        assert_eq!(target.get_pixel(45, 107), Some(Color::opaque(10, 10, 200)));
        assert_eq!(target.get_pixel(70, 90), Some(Color::opaque(0, 0, 0)));
    }

    #[test]
    fn player_control_hint_does_not_truncate_full_width_last_com() {
        // Com2Control accepts C4Player::LastCom as int32_t. High bits must
        // therefore keep an otherwise-valid low byte in the default Menu
        // bucket instead of being narrowed to a visible movement control.
        assert_eq!(com_control_index(0x101), (9, false));
        assert_eq!(com_control_index(1), (6, false));
    }

    #[test]
    fn player_control_label_layers_follow_tick35_blinking() {
        // Layer two enables key text; layer three suppresses that text while
        // Tick35 > 18 (src/C4Viewport.cpp:1431-1439).
        let render = |mask: i32, frame: u64| {
            let mut target = surface(300, 240);
            let hud = HudGraphics {
                control: Some(tutorial_control_sheet()),
                ..HudGraphics::default()
            };
            let font = bitmap_font();
            let font = HudFont::Fallback(&font);
            draw_player_controls(
                &mut target,
                &font,
                &font,
                &hud,
                SurfaceRect::new(0, 0, 300, 240),
                mask,
                0,
                0,
                &["K".to_string()],
                frame,
            );
            target
        };
        let key_only = render(1, 18);
        let visible_label = render(1 | (1 << 10) | (1 << 20), 18);
        let hidden_label = render(1 | (1 << 10) | (1 << 20), 19);

        assert_ne!(visible_label.pixels(), key_only.pixels());
        assert_eq!(hidden_label.pixels(), key_only.pixels());
    }

    #[test]
    fn real_tutorial_one_guide_uses_clean_keyboard_one_labels() {
        // Tutorial01 Script21 enables the Up/Left/Down/Right cells, their
        // labels and their blinking layer. StringBitEval preserves each
        // character position (C4Script.cpp:209-216), so use the shipped
        // script string rather than reconstructing the mask.
        let script_path =
            crate::test_support::repo_root().join("content/Tutorial.c4f/Tutorial01.c4s/Script.c");
        let script = std::fs::read_to_string(&script_path)
            .unwrap_or_else(|error| panic!("read {}: {error}", script_path.display()));
        let script21 = script
            .split("func Script21()")
            .nth(1)
            .and_then(|tail| tail.split("func Script50()").next())
            .expect("Tutorial01 Script21 block");
        let controls = script21
            .split("SetPlrShowControl(0,\"")
            .nth(1)
            .and_then(|tail| tail.split('"').next())
            .expect("Tutorial01 Script21 control string");
        let show_control = controls
            .bytes()
            .enumerate()
            .filter(|(_, byte)| !matches!(byte, b'_' | b' '))
            .fold(0_i32, |mask, (position, _)| mask | (1_i32 << position));
        let movement_mask = (1 << 4) | (1 << 6) | (1 << 7) | (1 << 8);
        assert_eq!(
            show_control,
            movement_mask | (movement_mask << 10) | (movement_mask << 20)
        );

        // C4ConfigControls keyboard set one is Q/W/E/A/S/D/Z/X/C/R in
        // CON_* order (C4Config.cpp:624-633). PlrControlKeyName returns those
        // short configured names; in the guide's spatial order this is S
        // above Z/X/C, with no arrow-key aliases.
        let labels = ["Q", "W", "E", "A", "S", "D", "Z", "X", "C", "R"].map(str::to_string);
        assert_eq!(
            [
                labels[6].as_str(),
                labels[4].as_str(),
                labels[7].as_str(),
                labels[8].as_str()
            ],
            ["Z", "S", "X", "C"]
        );
        assert!(labels.iter().all(|label| !label.contains("Arrow")));

        let viewport = SurfaceRect::new(0, 0, 1068, 780);
        let hud = HudGraphics {
            control: Some(crate::test_support::load_graphics_png("Control.png")),
            ..HudGraphics::default()
        };
        let fonts = crate::test_support::endeavour_font_set();
        let render = |key_labels: &[String], frame: u64| {
            let mut target = Surface::new(1068, 780, PixelFormat::Rgba8888);
            target.fill(Color::opaque(12, 24, 40));
            draw_player_controls_with_gamma(
                &mut target,
                &HudFont::Clonk(&fonts.text),
                &HudFont::Clonk(&fonts.mini),
                &hud,
                viewport,
                show_control,
                3,
                0,
                key_labels,
                frame,
                Some(crate::test_support::standard_gamma()),
            );
            target
        };
        let no_labels = vec![String::new(); 10];
        let unlabeled = render(&no_labels, 18);
        let guide = render(&labels, 18);
        assert_eq!(guide.snapshot().to_string(), "1068x780#54938995");

        // At the user's high-resolution viewport size, prove each glyph's
        // changed pixels remain inside its own C++ 3x4 grid cell. This catches
        // the former UpArrow/RightArrow spill and split labels directly.
        let size = (viewport.width as i32 / 3).min(7 * viewport.height as i32 / 24);
        let tx = viewport.x + viewport.width as i32 / 4 - size / 2;
        let ty = viewport.y + 15;
        let cell_width = size / 3;
        let cell_height = size / 4;
        for control in [4, 6, 7, 8] {
            let mut one_label = no_labels.clone();
            one_label[control] = labels[control].clone();
            let rendered = render(&one_label, 18);
            let cell = SurfaceRect::new(
                tx + cell_width * (control as i32 % 3),
                ty + cell_height * (control as i32 / 3),
                cell_width as u32,
                cell_height as u32,
            );
            let changed = rendered
                .pixels()
                .chunks_exact(4)
                .zip(unlabeled.pixels().chunks_exact(4))
                .enumerate()
                .filter(|(_, (actual, base))| actual != base)
                .map(|(index, _)| {
                    (
                        (index % viewport.width as usize) as i32,
                        (index / viewport.width as usize) as i32,
                    )
                })
                .collect::<Vec<_>>();
            assert!(!changed.is_empty(), "control {control} label renders");
            assert!(changed.iter().all(|&(x, y)| {
                x >= cell.x
                    && x < cell.x + cell.width as i32
                    && y >= cell.y
                    && y < cell.y + cell.height as i32
            }));
        }
        assert_eq!(render(&labels, 19).pixels(), unlabeled.pixels());
    }

    #[test]
    fn bottom_command_pair_sits_right_aligned_in_23px_cells() {
        // C4Viewport::DrawCursorInfo (src/C4Viewport.cpp:948-961):
        // iSize = 2*C4SymbolSize/3 = 23; the bottom bar spans the viewport
        // bottom and each C4Object::DrawCommand truncates TWO squares from
        // its right end — image cell rightmost, key cell left of it
        // (src/C4Object.cpp:4043-4048).
        let mut target = surface(200, 100);
        let hud = HudGraphics {
            control: Some(control_sheet()),
            ..HudGraphics::default()
        };
        let font = bitmap_font();
        let icons = vec![CommandIcon {
            com: 5, // COM_Throw
            key_label: String::new(),
            side: false,
            caption: String::new(),
            image: CommandImage::Picture(Some(solid_image(8, 8, [200, 20, 20, 255]))),
        }];
        draw_commands(
            &mut target,
            &HudFont::Fallback(&font),
            &hud,
            SurfaceRect::new(0, 0, 200, 100),
            &icons,
            false,
        );
        // Image cell: [177,200) x [77,100), def picture aspect-fit fills it.
        assert_eq!(
            target.get_pixel(188, 88),
            Some(Color::opaque(200, 20, 20)),
            "picture centered in the rightmost 23px cell"
        );
        // Key cell: [154,177) — key cap blue (single-row symbol transparent).
        assert_eq!(
            target.get_pixel(160, 88),
            Some(Color::opaque(10, 10, 200)),
            "key cap fills the second-from-right 23px cell"
        );
        // Nothing further left.
        assert_eq!(target.get_pixel(140, 88), Some(Color::opaque(0, 0, 0)));
        assert_eq!(
            command_region_index(
                SurfaceRect::new(0, 0, 200, 100),
                clonk_gui::Point::new(154.0, 77.0),
                &icons,
            ),
            Some(0),
            "the key cell's first pixel belongs to the paired C4Region"
        );
        assert_eq!(
            command_region_hit(
                SurfaceRect::new(0, 0, 200, 100),
                clonk_gui::Point::new(154.0, 77.0),
                &icons,
            ),
            Some((0, SurfaceRect::new(154, 77, 46, 23))),
            "the hit exposes the exact pair geometry used by CaptionBottomY"
        );
        assert_eq!(
            command_region_index(
                SurfaceRect::new(0, 0, 200, 100),
                clonk_gui::Point::new(199.0, 99.0),
                &icons,
            ),
            Some(0),
            "the image cell's final integer pixel remains inside"
        );
        assert_eq!(
            command_region_index(
                SurfaceRect::new(0, 0, 200, 100),
                clonk_gui::Point::new(153.0, 88.0),
                &icons,
            ),
            None
        );
    }

    #[test]
    fn flash_command_blinks_only_the_matching_key_cell() {
        // C4Object::DrawCommand keeps the command image and hit region while
        // hiding the exact FlashCom key for Tick35 0..=15, then drawing it
        // for 16..=34 (src/C4Object.cpp:4043-4047,4084-4091).
        let hud = HudGraphics {
            control: Some(control_sheet()),
            ..HudGraphics::default()
        };
        let font = bitmap_font();
        let icons = vec![CommandIcon {
            com: 5,
            key_label: String::new(),
            side: false,
            caption: String::new(),
            image: CommandImage::Picture(Some(solid_image(8, 8, [200, 20, 20, 255]))),
        }];
        let render = |frame| {
            let mut target = surface(200, 100);
            draw_commands_with_gamma(
                &mut target,
                &HudFont::Fallback(&font),
                &hud,
                SurfaceRect::new(0, 0, 200, 100),
                &icons,
                false,
                5,
                frame,
                None,
            );
            target
        };

        for frame in [0, 15] {
            let target = render(frame);
            assert_eq!(target.get_pixel(160, 88), Some(Color::opaque(0, 0, 0)));
            assert_eq!(
                target.get_pixel(188, 88),
                Some(Color::opaque(200, 20, 20)),
                "command image remains visible at frame {frame}"
            );
        }
        assert_eq!(
            render(16).get_pixel(160, 88),
            Some(Color::opaque(10, 10, 200)),
            "matching key returns for Tick35 16"
        );
        assert_eq!(
            command_region_index(
                SurfaceRect::new(0, 0, 200, 100),
                clonk_gui::Point::new(160.0, 88.0),
                &icons,
            ),
            Some(0),
            "blinking never removes the command region"
        );
    }

    #[test]
    fn double_com_key_uses_second_fctcommand_row() {
        // DrawCommandKey (src/C4ObjectCom.cpp:938): fctCommand.Draw(...,
        // Com2Control(iCom), (iCom & COM_Double) != 0) — the double row is
        // one cell height below the single row.
        let mut target = surface(200, 100);
        let hud = HudGraphics {
            control: Some(control_sheet()),
            ..HudGraphics::default()
        };
        let font = bitmap_font();
        let icons = vec![CommandIcon {
            com: 4 | 128, // COM_Down_D
            key_label: String::new(),
            side: false,
            caption: String::new(),
            image: CommandImage::Picture(None),
        }];
        draw_commands(
            &mut target,
            &HudFont::Fallback(&font),
            &hud,
            SurfaceRect::new(0, 0, 200, 100),
            &icons,
            false,
        );
        // Key cell shows the green double-row symbol over the blue cap.
        assert_eq!(
            target.get_pixel(160, 88),
            Some(Color::opaque(10, 200, 10)),
            "double-row fctCommand phase over the key cap"
        );
    }

    #[test]
    fn side_command_pairs_stack_upward_above_the_bottom_row() {
        // Secondary area (src/C4Viewport.cpp:958): right side strip of
        // width 2*iSize, height cgo.Hgt - iSize - 5; DrawCommand with
        // C4FCT_Bottom|C4FCT_Half takes a 2*iSize x iSize slice from the
        // strip BOTTOM, key cell left, image cell right
        // (src/C4Facet.cpp:182-215, src/C4Object.cpp:4044-4047).
        let mut target = surface(200, 150);
        let hud = HudGraphics {
            control: Some(control_sheet()),
            ..HudGraphics::default()
        };
        let font = bitmap_font();
        let icon = |color: [u8; 4]| CommandIcon {
            com: 7, // COM_Special
            key_label: String::new(),
            side: true,
            caption: String::new(),
            image: CommandImage::Picture(Some(solid_image(8, 8, color))),
        };
        let icons = vec![icon([200, 20, 20, 255]), icon([20, 20, 200, 255])];
        draw_commands(
            &mut target,
            &HudFont::Fallback(&font),
            &hud,
            SurfaceRect::new(0, 0, 200, 150),
            &icons,
            false,
        );
        // Strip: x in [154,200), bottom at y = 150 - 23 - 5 = 122.
        // First pair occupies y [99,122): key at x[154,177), image x[177,200).
        assert_eq!(
            target.get_pixel(188, 110),
            Some(Color::opaque(200, 20, 20)),
            "first side image cell at the strip bottom"
        );
        assert_eq!(
            target.get_pixel(160, 110),
            Some(Color::opaque(10, 10, 200)),
            "first side key cell left of the image cell"
        );
        // Second pair stacks above: y [76,99).
        assert_eq!(
            target.get_pixel(188, 87),
            Some(Color::opaque(20, 20, 200)),
            "second side image cell above the first"
        );
    }

    #[test]
    fn command_rows_need_viewport_taller_than_symbol_size() {
        // `if (cgo.Hgt > C4SymbolSize)` (src/C4Viewport.cpp:950).
        let mut target = surface(200, 35);
        let hud = HudGraphics {
            control: Some(control_sheet()),
            ..HudGraphics::default()
        };
        let font = bitmap_font();
        let icons = vec![CommandIcon {
            com: 5,
            key_label: String::new(),
            side: false,
            caption: String::new(),
            image: CommandImage::Picture(Some(solid_image(8, 8, [200, 20, 20, 255]))),
        }];
        draw_commands(
            &mut target,
            &HudFont::Fallback(&font),
            &hud,
            SurfaceRect::new(0, 0, 200, 35),
            &icons,
            false,
        );
        assert!(
            target
                .pixels()
                .chunks_exact(4)
                .all(|chunk| chunk == [0, 0, 0, 255]),
            "35px-high viewports draw no command rows"
        );
    }

    #[test]
    fn composite_image_cell_draws_picture_right_top_and_hand_left_bottom() {
        // Put/Get/UnGrab image cells (src/C4Object.cpp:2976-2995): def
        // picture in GetFraction(85,85,Right,Top), fctHand phase in
        // GetFraction(85,85,Left,Bottom) — 19px fractions of the 23px cell
        // (23*85/100 = 19, src/C4Facet.cpp:459-474).
        let mut target = surface(200, 100);
        // Hand.png: two square cells — phase 0 yellow, phase 1 cyan.
        let mut hand_pixels = Vec::new();
        for _y in 0..8 {
            for x in 0..16 {
                hand_pixels.extend_from_slice(if x < 8 {
                    &[200u8, 200, 20, 255]
                } else {
                    &[20u8, 200, 200, 255]
                });
            }
        }
        let hud = HudGraphics {
            control: Some(control_sheet()),
            hand: Some(ImageData::new(16, 8, hand_pixels)),
            ..HudGraphics::default()
        };
        let font = bitmap_font();
        let icons = vec![CommandIcon {
            com: 5,
            key_label: String::new(),
            side: false,
            caption: String::new(),
            image: CommandImage::Composite {
                picture: Some(solid_image(8, 8, [200, 20, 20, 255])),
                icon: CommandOverlayIcon::Hand(1),
            },
        }];
        draw_commands(
            &mut target,
            &HudFont::Fallback(&font),
            &hud,
            SurfaceRect::new(0, 0, 200, 100),
            &icons,
            false,
        );
        // Image cell [177,200) x [77,100): picture fraction right-top
        // (x 181..200, y 77..96), hand fraction left-bottom (177..196, 81..100).
        assert_eq!(
            target.get_pixel(197, 79),
            Some(Color::opaque(200, 20, 20)),
            "picture in the right-top 85% fraction"
        );
        assert_eq!(
            target.get_pixel(178, 97),
            Some(Color::opaque(20, 200, 200)),
            "fctHand phase 1 in the left-bottom 85% fraction"
        );
    }

    #[test]
    fn formats_game_time_like_upper_board_execute() {
        // C4UpperBoard::Execute (src/C4UpperBoard.cpp:41).
        assert_eq!(format_game_time(0), "00:00:00");
        assert_eq!(format_game_time(18), "00:00:18");
        assert_eq!(format_game_time(3600 + 2 * 60 + 3), "01:02:03");
    }

    #[test]
    fn upper_board_modes_match_cpp_reserved_and_output_geometry() {
        let hud = HudGraphics {
            upper_board: Some(solid_image(8, 55, [120, 80, 40, 255])),
            ..HudGraphics::default()
        };
        assert_eq!(upper_board_reserved_height(UpperBoardMode::Hide), 0);
        assert_eq!(upper_board_reserved_height(UpperBoardMode::Full), 50);
        assert_eq!(upper_board_reserved_height(UpperBoardMode::Small), 25);
        assert_eq!(upper_board_reserved_height(UpperBoardMode::Mini), 0);
        assert_eq!(upper_board_output_height(UpperBoardMode::Hide, &hud), 0);
        assert_eq!(upper_board_output_height(UpperBoardMode::Full, &hud), 55);
        assert_eq!(upper_board_output_height(UpperBoardMode::Small, &hud), 27);
        assert_eq!(upper_board_output_height(UpperBoardMode::Mini, &hud), 0);

        let marker = FixedWidthMarkerFont;
        let font = HudFont::Fallback(&marker);
        let mut hidden = surface(400, 60);
        hidden.fill(Color::opaque(7, 11, 13));
        let before = hidden.pixels().to_vec();
        draw_upper_board_with_gamma(
            &mut hidden,
            &font,
            &hud,
            UpperBoardMode::Hide,
            "Hidden title",
            0,
            Some("[12:34:56]"),
            Some(42),
            None,
        );
        assert_eq!(hidden.pixels(), before);
    }

    #[test]
    fn small_upper_board_scales_tiles_logo_and_text_row() {
        let mut tile_pixels = Vec::new();
        for _ in 0..2 {
            for x in 0..4 {
                tile_pixels.extend_from_slice(if x < 2 {
                    &[220, 20, 20, 255]
                } else {
                    &[20, 20, 220, 255]
                });
            }
        }
        let hud = HudGraphics {
            upper_board: Some(ImageData::new(4, 2, tile_pixels)),
            ..HudGraphics::default()
        };
        let marker = FixedWidthMarkerFont;
        let font = HudFont::Fallback(&marker);
        let mut tiled = surface(16, 30);
        draw_upper_board(&mut tiled, &font, &hud, UpperBoardMode::Small, "", 0);
        assert_eq!(tiled.get_pixel(0, 0), tiled.get_pixel(2, 0));
        assert_eq!(tiled.get_pixel(1, 0), tiled.get_pixel(3, 0));
        assert_ne!(tiled.get_pixel(0, 0), tiled.get_pixel(1, 0));
        assert_eq!(tiled.get_pixel(0, 0).expect("scaled tile").a, 255);

        let hud = HudGraphics {
            upper_board: Some(solid_image(8, 55, [120, 80, 40, 255])),
            logo: Some(solid_image(960, 320, [10, 200, 30, 255])),
            ..HudGraphics::default()
        };
        let mut target = surface(400, 60);
        draw_upper_board_with_gamma(
            &mut target,
            &font,
            &hud,
            UpperBoardMode::Small,
            "T",
            0,
            None,
            None,
            None,
        );
        assert_eq!(target.get_pixel(145, 10), Some(Color::opaque(120, 80, 40)));
        assert_eq!(target.get_pixel(146, 10), Some(Color::opaque(10, 200, 30)));
        assert_eq!(target.get_pixel(252, 10), Some(Color::opaque(10, 200, 30)));
        assert_eq!(target.get_pixel(253, 10), Some(Color::opaque(120, 80, 40)));
        assert_eq!(target.get_pixel(10, 10), Some(MESSAGE_COLOR));
        assert_eq!(target.get_pixel(358, 9), Some(MESSAGE_COLOR));
        assert_eq!(target.get_pixel(20, 26), Some(Color::opaque(120, 80, 40)));
        assert_eq!(target.get_pixel(20, 27), Some(Color::opaque(0, 0, 0)));
    }

    #[test]
    fn mini_upper_board_carves_message_width_and_draws_only_bottom_text() {
        let marker = FixedWidthMarkerFont;
        let font = HudFont::Fallback(&marker);
        assert_eq!(upper_board_text_strip_width(&font, 0), 126);
        assert_eq!(
            message_board_available_width(400, &font, UpperBoardMode::Mini, 0),
            274
        );

        let hud = HudGraphics {
            upper_board: Some(solid_image(8, 50, [120, 80, 40, 255])),
            background: Some(solid_image(8, 8, [30, 50, 70, 255])),
            logo: Some(solid_image(960, 320, [10, 200, 30, 255])),
            ..HudGraphics::default()
        };
        let mut target = surface(400, 60);
        let long_line = "X".repeat(136);
        assert_eq!(message_board_tail_line(&font, &long_line, 274), "  XX");
        assert_eq!(
            message_board_physical_lines(&font, &format!("{} ", "X".repeat(68)), 274),
            vec!["X".repeat(68)]
        );
        let tail = message_board_tail_line(&font, &long_line, 274);
        let board = MessageBoardOverlay {
            log_lines: vec![tail],
            back_scroll: 0,
            ..MessageBoardOverlay::default()
        };
        draw_message_board_with_gamma(&mut target, &font, &hud, &board, None);
        assert_eq!(target.get_pixel(0, 54), Some(MESSAGE_COLOR));
        assert_eq!(target.get_pixel(273, 54), Some(Color::opaque(30, 50, 70)));
        assert_eq!(target.get_pixel(274, 54), Some(Color::opaque(30, 50, 70)));

        let mut unclipped = surface(400, 60);
        let unclipped_board = MessageBoardOverlay {
            log_lines: vec!["X".repeat(70)],
            back_scroll: 0,
            ..MessageBoardOverlay::default()
        };
        draw_message_board_with_gamma(&mut unclipped, &font, &hud, &unclipped_board, None);
        assert_eq!(
            unclipped.get_pixel(275, 54),
            Some(MESSAGE_COLOR),
            "the shortened facet wraps at append time but does not clip StringOut"
        );
        draw_upper_board_with_gamma(
            &mut target,
            &font,
            &hud,
            UpperBoardMode::Mini,
            "Must not draw",
            0,
            Some("[12:34:56]"),
            Some(42),
            None,
        );

        assert_eq!(target.get_pixel(10, 10), Some(Color::opaque(0, 0, 0)));
        assert_eq!(target.get_pixel(100, 54), Some(Color::opaque(30, 50, 70)));
        assert_eq!(target.get_pixel(358, 54), Some(MESSAGE_COLOR));
        assert_eq!(target.get_pixel(306, 54), Some(MESSAGE_COLOR));
        assert_eq!(target.get_pixel(274, 54), Some(MESSAGE_COLOR));
    }

    #[test]
    fn upper_board_draws_clock_and_fps_rows_when_enabled() {
        // iRightOff advances once for each enabled right-hand section:
        // playing time at 1*TextWidth-10, then Clock/FPS at successive
        // TextWidth slots with the 30px inset (src/C4UpperBoard.cpp:74-88).
        let mut target = surface(400, 60);
        let hud = HudGraphics {
            upper_board: Some(solid_image(8, 50, [120, 80, 40, 255])),
            ..HudGraphics::default()
        };
        let marker = FixedWidthMarkerFont;
        let font = HudFont::Fallback(&marker);
        draw_upper_board_with_gamma(
            &mut target,
            &font,
            &hud,
            UpperBoardMode::Full,
            "",
            0,
            Some("[12:34:56]"),
            Some(42),
            None,
        );

        let text_y = (UPPER_BOARD_HEIGHT / 2 - font.line_height() / 2) as u32;
        assert_eq!(
            target.get_pixel(306, text_y),
            Some(MESSAGE_COLOR),
            "clock starts at width - 2*TextWidth - 30"
        );
        assert_eq!(
            target.get_pixel(274, text_y),
            Some(MESSAGE_COLOR),
            "FPS starts at width - 3*TextWidth - 30 when Clock is present"
        );

        let mut disabled = surface(400, 60);
        draw_upper_board_with_gamma(
            &mut disabled,
            &font,
            &hud,
            UpperBoardMode::Full,
            "",
            0,
            None,
            None,
            None,
        );
        assert_eq!(
            disabled.get_pixel(306, text_y),
            Some(Color::opaque(120, 80, 40))
        );
        assert_eq!(
            disabled.get_pixel(274, text_y),
            Some(Color::opaque(120, 80, 40))
        );
    }

    #[test]
    fn upper_board_tiles_texture_across_full_texture_height() {
        // BlitSurfaceTile over Output.Wdt x Output.Hgt where Output.Hgt =
        // max(50, texture height) (src/C4UpperBoard.cpp:52,117-120).
        let mut target = surface(64, 80);
        let hud = HudGraphics {
            upper_board: Some(solid_image(16, 55, [120, 80, 40, 255])),
            ..HudGraphics::default()
        };
        let font = bitmap_font();
        let font = HudFont::Fallback(&font);
        draw_upper_board(&mut target, &font, &hud, UpperBoardMode::Full, "", 0);
        // Tiles cover x beyond one tile width and the full 55px height...
        assert_eq!(target.get_pixel(40, 54), Some(Color::opaque(120, 80, 40)));
        // ...but not below the texture height.
        assert_eq!(target.get_pixel(40, 55), Some(Color::opaque(0, 0, 0)));
    }

    #[test]
    fn upper_board_logo_is_centered_with_021_zoom_for_3_to_1_logo() {
        // fLogoZoom = 0.21 * 960/Wdt for the 3:1 logo
        // (src/C4UpperBoard.cpp:56-68).
        let mut target = surface(400, 80);
        let hud = HudGraphics {
            upper_board: Some(solid_image(16, 50, [120, 80, 40, 255])),
            logo: Some(solid_image(960, 320, [10, 200, 30, 255])),
            ..HudGraphics::default()
        };
        let font = bitmap_font();
        let font = HudFont::Fallback(&font);
        draw_upper_board(&mut target, &font, &hud, UpperBoardMode::Full, "", 0);
        // dst x = 400/2 - 480*0.21 = 99.2 -> 99, width 201, height 67.
        assert_eq!(target.get_pixel(98, 30), Some(Color::opaque(120, 80, 40)));
        assert_eq!(target.get_pixel(100, 30), Some(Color::opaque(10, 200, 30)));
        assert_eq!(target.get_pixel(200, 66), Some(Color::opaque(10, 200, 30)));
        assert_eq!(target.get_pixel(200, 68), Some(Color::opaque(0, 0, 0)));
    }

    #[test]
    fn two_line_product_logo_keeps_the_classic_upper_board_height() {
        let mut target = surface(400, 120);
        let hud = HudGraphics {
            upper_board: Some(solid_image(16, 50, [120, 80, 40, 255])),
            logo: Some(solid_image(972, 440, [10, 200, 30, 255])),
            ..HudGraphics::default()
        };
        let font = bitmap_font();
        let font = HudFont::Fallback(&font);
        draw_upper_board(&mut target, &font, &hud, UpperBoardMode::Full, "", 0);

        assert_eq!(target.get_pixel(200, 66), Some(Color::opaque(10, 200, 30)));
        assert_eq!(target.get_pixel(200, 68), Some(Color::opaque(0, 0, 0)));
    }

    #[test]
    fn aspect_fit_matches_c4facet_integer_math() {
        // 150x150 portrait into the 56x45 facet -> 45x45 at x+5
        // (src/C4Facet.cpp:100-121, src/C4ObjectInfo.cpp:313).
        let fitted = aspect_fit(150, 150, SurfaceRect::new(5, 5, 56, 45));
        assert_eq!((fitted.x, fitted.y), (10, 5));
        assert_eq!((fitted.width, fitted.height), (45, 45));
        // 60x30 wealth icon into 35x17 -> 34x17 at x+0.
        let fitted = aspect_fit(60, 30, SurfaceRect::new(0, 0, 35, 17));
        assert_eq!((fitted.x, fitted.y), (0, 0));
        assert_eq!((fitted.width, fitted.height), (34, 17));
    }

    #[test]
    fn colorize_by_owner_turns_blue_pixels_into_owner_color() {
        // ClrByOwner detects the pure-blue pixel and modulates the owner
        // color by its gray value (src/C4Surface.cpp:236-287).
        let image = solid_image(1, 1, [0, 0, 255, 255]);
        let colored = colorize_by_owner(&image, Color::opaque(255, 0, 0));
        assert_eq!(&colored.pixels()[..4], &[255, 0, 0, 255]);
        assert_eq!(
            colored.gpu_texture_id(),
            colorize_by_owner(&image, Color::opaque(255, 0, 0)).gpu_texture_id()
        );
        // Non-blue pixels pass through untouched.
        let image = solid_image(1, 1, [200, 30, 30, 255]);
        let colored = colorize_by_owner(&image, Color::opaque(255, 0, 0));
        assert_eq!(&colored.pixels()[..4], &[200, 30, 30, 255]);
    }

    #[test]
    fn resource_auto_mask_detector_matches_hud_over_sampled_colors() {
        const CHANNELS: [u8; 16] = [
            0, 1, 15, 31, 42, 63, 100, 101, 127, 128, 145, 170, 175, 223, 254, 255,
        ];
        let side = CHANNELS.len() as u32;
        let width = side * side;
        let mut source = image::RgbaImage::new(width, side);
        for (r_index, &r) in CHANNELS.iter().enumerate() {
            for (g_index, &g) in CHANNELS.iter().enumerate() {
                for (b_index, &b) in CHANNELS.iter().enumerate() {
                    source.put_pixel(
                        r_index as u32 * side + g_index as u32,
                        b_index as u32,
                        image::Rgba([r, g, b, 255]),
                    );
                }
            }
        }

        let temp = tempfile::Builder::new()
            .prefix("lc-hud-owner-mask-")
            .tempdir()
            .expect("tempdir");
        let definition_dir = temp.path().join("Sweep.c4d");
        std::fs::create_dir(&definition_dir).expect("definition directory");
        std::fs::write(
            definition_dir.join("DefCore.txt"),
            b"[DefCore]\nid=SWEP\nColorByOwner=1\n",
        )
        .expect("DefCore");
        source
            .save(definition_dir.join("Graphics.png"))
            .expect("graphics");
        let group = clonk_resources::Group::open(&definition_dir).expect("open definition");
        let definition =
            clonk_resources::ResourceDefinition::load(&group).expect("load definition");
        let mask = definition
            .color_by_owner_mask
            .expect("sample sweep contains blue shades");

        for (r_index, &r) in CHANNELS.iter().enumerate() {
            for (g_index, &g) in CHANNELS.iter().enumerate() {
                for (b_index, &b) in CHANNELS.iter().enumerate() {
                    let x = r_index as u32 * side + g_index as u32;
                    let y = b_index as u32;
                    assert_eq!(
                        mask.pixels[(y * width + x) as usize],
                        clr_by_owner_gray(i32::from(r), i32::from(g), i32::from(b)).unwrap_or(0),
                        "sample rgb({r}, {g}, {b})"
                    );
                }
            }
        }
    }

    #[test]
    fn energy_bar_uses_effective_physical_range_and_cpp_integer_threshold() {
        // DrawEnergyLevelEx: rows below yBar sample the filled column 0,
        // rows above the empty column 1 (src/C4Facet.cpp:334-389).
        let mut target = surface(40, 200);
        // 6x3 grid of 1x1 cells: filled column red, empty column gray.
        let mut pixels = Vec::new();
        for _row in 0..3 {
            pixels.extend_from_slice(&[200, 0, 0, 255]); // col 0: filled
            pixels.extend_from_slice(&[60, 60, 60, 255]); // col 1: empty
            pixels.extend_from_slice(&[0, 0, 200, 255, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0]);
        }
        let hud = HudGraphics {
            energy_bars: Some(ImageData::new(6, 3, pixels)),
            ..HudGraphics::default()
        };
        let viewport = SurfaceRect::new(0, 0, 40, 200);
        draw_level_bar(
            &mut target,
            &hud,
            viewport,
            HudBarKind::Energy,
            0,
            1,
            2,
            true,
        );
        // Bar spans y=55..160 with height 105. Native integer division fills
        // 1*105/2=52 rows, so local row 52 stays empty and row 53 is filled.
        // The removed float path rounded 52.5 up and filled one row too many.
        let bar_top = SYMBOL_SIZE + 2 * SYMBOL_BORDER + 10;
        let bar_height = 200 - 3 * SYMBOL_BORDER - 2 * SYMBOL_SIZE - 10;
        let x = SYMBOL_BORDER as u32;
        assert_eq!(bar_height, 105);
        assert_eq!(level_bar_y(bar_height, 1, 2), 53);
        assert_eq!(
            target.get_pixel(x, (bar_top + 52) as u32),
            Some(Color::opaque(60, 60, 60)),
            "the last C++-empty fractional boundary row remains empty"
        );
        assert_eq!(
            target.get_pixel(x, (bar_top + 53) as u32),
            Some(Color::opaque(200, 0, 0)),
            "the native threshold row begins the filled half"
        );
        assert_eq!(
            target.get_pixel(x, (bar_top + bar_height - 2) as u32),
            Some(Color::opaque(200, 0, 0)),
            "bottom of a half-full bar is filled"
        );

        let mut without_portraits = surface(40, 200);
        draw_level_bar(
            &mut without_portraits,
            &hud,
            viewport,
            HudBarKind::Energy,
            0,
            1,
            2,
            false,
        );
        let portraitless_top = SYMBOL_SIZE + 2 * SYMBOL_BORDER;
        let portraitless_height = 200 - 3 * SYMBOL_BORDER - 2 * SYMBOL_SIZE;
        assert_eq!(
            without_portraits.get_pixel(x, (portraitless_top - 1) as u32),
            Some(Color::opaque(0, 0, 0)),
            "the row above the portraitless bar stays untouched"
        );
        assert_eq!(
            without_portraits.get_pixel(x, portraitless_top as u32),
            Some(Color::opaque(60, 60, 60)),
            "disabling portraits moves the bar top up ten pixels"
        );
        assert_eq!(
            without_portraits.get_pixel(x, (portraitless_top + portraitless_height - 1) as u32),
            Some(Color::opaque(200, 0, 0)),
            "the portraitless bar keeps the portraits-on bottom edge"
        );
        assert_eq!(
            without_portraits.get_pixel(x, (portraitless_top + portraitless_height) as u32),
            Some(Color::opaque(0, 0, 0)),
            "the row below the bar stays untouched"
        );
        assert_eq!(portraitless_height, bar_height + 10);
        assert_eq!(portraitless_top + portraitless_height, bar_top + bar_height);

        // BoundBy tests the lower bound before the inverted upper bound.
        assert_eq!(level_bar_y(105, -1, -2), 105);
        assert_eq!(level_bar_y(105, 0, 0), 105);
        assert_eq!(level_bar_y(105, 0, -2), 315);
    }

    #[test]
    fn cursor_info_places_portrait_and_rank_symbol_like_c4objectinfo_draw() {
        // Portrait aspect-fit into (border, border, 4*35/3+10, 45) and the
        // rank cell 1:1 at iX = 4*35/3 (src/C4ObjectInfo.cpp:308-341).
        let mut target = surface(200, 200);
        let portrait = solid_image(150, 150, [10, 200, 30, 255]);
        // Two 4x4 rank cells: rank 0 blue, rank 1 yellow.
        let mut rank_pixels = Vec::new();
        for _row in 0..4 {
            rank_pixels.extend(std::iter::repeat_n([0u8, 0, 220, 255], 4).flatten());
            rank_pixels.extend(std::iter::repeat_n([220u8, 220, 0, 255], 4).flatten());
        }
        let ranks = ImageData::new(8, 4, rank_pixels);
        let hud = HudGraphics {
            rank: Some(ranks.clone()),
            ..HudGraphics::default()
        };
        let font = bitmap_font();
        let font = HudFont::Fallback(&font);
        let viewport = SurfaceRect::new(0, 0, 200, 200);
        draw_cursor_info(
            &mut target,
            &font,
            &hud,
            viewport,
            "William",
            0,
            Some(&portrait),
            Some(&ranks),
        );
        // Portrait: (5,5,56,45) aspect-fit -> (10,5,45,45).
        assert_eq!(target.get_pixel(9, 20), Some(Color::opaque(0, 0, 0)));
        assert_eq!(target.get_pixel(11, 20), Some(Color::opaque(10, 200, 30)));
        assert_eq!(target.get_pixel(54, 20), Some(Color::opaque(10, 200, 30)));
        assert_eq!(target.get_pixel(56, 20), Some(Color::opaque(0, 0, 0)));
        // Rank 0 cell at (5 + 46, 5), 4x4, blue — drawn over the portrait's
        // right edge, exactly like C++ (iX advances 4*Hgt/3 while the
        // portrait facet is 4*Hgt/3+10 wide, src/C4ObjectInfo.cpp:313-320).
        assert_eq!(target.get_pixel(51, 6), Some(Color::opaque(0, 0, 220)));
        assert_eq!(target.get_pixel(51, 10), Some(Color::opaque(10, 200, 30)));
    }

    #[test]
    fn cursor_info_without_portrait_keeps_rank_in_the_first_column() {
        // A null Portrait.GetGfx() skips both DrawClr and the iX increment
        // (src/C4ObjectInfo.cpp:308-320).
        let ranks = solid_image(4, 4, [220, 30, 20, 255]);
        let mut target = surface(100, 60);
        let font = bitmap_font();
        draw_cursor_info(
            &mut target,
            &HudFont::Fallback(&font),
            &HudGraphics::default(),
            SurfaceRect::new(0, 0, 100, 60),
            "",
            0,
            None,
            Some(&ranks),
        );

        assert_eq!(target.get_pixel(5, 5), Some(Color::opaque(220, 30, 20)));
        assert_eq!(target.get_pixel(51, 5), Some(Color::opaque(0, 0, 0)));
    }

    #[test]
    fn cursor_info_draws_owner_overlay_as_the_second_aspect_pass() {
        let base = solid_image(1, 1, [20, 40, 60, 255]);
        let owner_overlay = solid_image(1, 1, [147, 0, 0, 255]);
        let mut target = surface(80, 60);
        let font = bitmap_font();
        draw_cursor_info_with_gamma(
            &mut target,
            &HudFont::Fallback(&font),
            &HudGraphics::default(),
            SurfaceRect::new(0, 0, 80, 60),
            "",
            0,
            None,
            Some(&base),
            Some(&owner_overlay),
            0x00e8_0000,
            None,
            None,
            false,
            0,
            None,
        );

        assert_eq!(target.get_pixel(20, 20), Some(Color::opaque(134, 0, 0)));

        let base = solid_image(1, 1, [10, 20, 30, 255]);
        let owner_overlay = solid_image(1, 1, [64, 128, 192, 128]);
        let mut target = surface(80, 60);
        draw_cursor_info_with_gamma(
            &mut target,
            &HudFont::Fallback(&font),
            &HudGraphics::default(),
            SurfaceRect::new(0, 0, 80, 60),
            "",
            0,
            None,
            Some(&base),
            Some(&owner_overlay),
            0x00e8_8040,
            None,
            None,
            false,
            0,
            None,
        );

        assert_eq!(target.get_pixel(20, 20), Some(Color::opaque(34, 42, 39)));
    }

    #[test]
    fn cursor_info_hide_bits_gate_and_compact_columns() {
        let portrait = solid_image(150, 150, [10, 200, 30, 255]);
        let captain = solid_image(6, 6, [220, 30, 20, 255]);
        let global_ranks = solid_image(7, 7, [90, 90, 90, 255]);
        let mut rank_pixels = Vec::new();
        for _row in 0..4 {
            rank_pixels.extend(std::iter::repeat_n([0u8, 0, 220, 255], 4).flatten());
            rank_pixels.extend(std::iter::repeat_n([220u8, 220, 0, 255], 4).flatten());
        }
        let ranks = ImageData::new(8, 4, rank_pixels);
        let hud = HudGraphics {
            captain: Some(captain),
            rank: Some(global_ranks),
            ..HudGraphics::default()
        };
        let font = bitmap_font();
        let render = |hide_hud_elements| {
            let mut target = surface(160, 80);
            draw_cursor_info_with_gamma(
                &mut target,
                &HudFont::Fallback(&font),
                &hud,
                SurfaceRect::new(0, 0, 160, 80),
                "WW",
                1,
                Some("I"),
                Some(&portrait),
                None,
                u32::MAX,
                Some(&ranks),
                None,
                true,
                hide_hud_elements,
                None,
            );
            target
        };
        let white_pixels = |surface: &Surface| {
            surface
                .pixels()
                .chunks_exact(4)
                .enumerate()
                .filter(|(_, pixel)| *pixel == [255, 255, 255, 255])
                .map(|(index, _)| {
                    (
                        index % surface.width() as usize,
                        index / surface.width() as usize,
                    )
                })
                .collect::<Vec<_>>()
        };

        let baseline = render(0);
        let baseline_white = white_pixels(&baseline);
        let baseline_min_x = baseline_white.iter().map(|(x, _)| *x).min().unwrap();
        assert_eq!(baseline.get_pixel(11, 20), Some(Color::opaque(10, 200, 30)));
        assert_eq!(baseline.get_pixel(51, 6), Some(Color::opaque(220, 30, 20)));
        assert_eq!(baseline.get_pixel(57, 6), Some(Color::opaque(220, 220, 0)));

        let hidden_portrait = render(clonk_engine::HIDE_HUD_ELEMENT_PORTRAIT);
        assert_ne!(
            hidden_portrait.get_pixel(11, 20),
            Some(Color::opaque(10, 200, 30))
        );
        assert_eq!(
            hidden_portrait.get_pixel(5, 6),
            Some(Color::opaque(220, 30, 20))
        );
        assert_eq!(
            hidden_portrait.get_pixel(11, 6),
            Some(Color::opaque(220, 220, 0))
        );
        assert_eq!(
            white_pixels(&hidden_portrait)
                .iter()
                .map(|(x, _)| *x)
                .min()
                .unwrap(),
            baseline_min_x - 46
        );

        let hidden_captain = render(clonk_engine::HIDE_HUD_ELEMENT_CAPTAIN);
        assert_eq!(
            hidden_captain.get_pixel(51, 6),
            Some(Color::opaque(220, 220, 0))
        );
        assert_eq!(
            white_pixels(&hidden_captain)
                .iter()
                .map(|(x, _)| *x)
                .min()
                .unwrap(),
            baseline_min_x - 6
        );

        let hidden_rank_image = render(clonk_engine::HIDE_HUD_ELEMENT_RANK_IMAGE);
        assert_ne!(
            hidden_rank_image.get_pixel(57, 6),
            Some(Color::opaque(220, 220, 0))
        );
        assert_eq!(
            white_pixels(&hidden_rank_image)
                .iter()
                .map(|(x, _)| *x)
                .min()
                .unwrap(),
            baseline_min_x - 7
        );

        let hidden_rank = white_pixels(&render(clonk_engine::HIDE_HUD_ELEMENT_RANK));
        let hidden_name = white_pixels(&render(clonk_engine::HIDE_HUD_ELEMENT_NAME));
        let hidden_text = white_pixels(&render(
            clonk_engine::HIDE_HUD_ELEMENT_RANK | clonk_engine::HIDE_HUD_ELEMENT_NAME,
        ));
        let second_line = (SYMBOL_BORDER + HudFont::Fallback(&font).line_height()) as usize;
        assert!(hidden_rank.iter().all(|(_, y)| *y < second_line));
        assert!(!hidden_name.is_empty(), "rank title survives HH_Name");
        assert!(hidden_name.iter().all(|(_, y)| *y < second_line));
        assert!(
            hidden_rank.len() > hidden_name.len(),
            "WW remains while I is hidden"
        );
        assert!(hidden_text.is_empty());
    }

    #[test]
    fn cursor_info_uses_definition_base_count_and_direct_extension_offset() {
        let mut colors = vec![[0, 0, 0, 255]; 29];
        colors[1] = [20, 40, 220, 255];
        colors[24] = [220, 40, 20, 255];
        let ranks = horizontal_cell_strip(6, &colors);
        let mut target = surface(40, 40);
        let font = bitmap_font();

        draw_cursor_info_with_gamma(
            &mut target,
            &HudFont::Fallback(&font),
            &HudGraphics::default(),
            SurfaceRect::new(0, 0, 40, 40),
            "",
            25,
            None,
            None,
            None,
            u32::MAX,
            Some(&ranks),
            Some(24),
            false,
            0,
            None,
        );

        // Base phase 1 starts at (5,5). Extension phase 24 is drawn at its
        // native 6x6 size at (1,2), the direct C++ HUD offset (-4,-3).
        assert_eq!(target.get_pixel(1, 2), Some(Color::opaque(220, 40, 20)));
        assert_eq!(target.get_pixel(6, 2), Some(Color::opaque(220, 40, 20)));
        assert_eq!(target.get_pixel(7, 2), Some(Color::opaque(0, 0, 0)));
        assert_eq!(target.get_pixel(10, 8), Some(Color::opaque(20, 40, 220)));
    }

    #[test]
    fn cursor_info_saturates_past_the_last_definition_extension() {
        let mut colors = vec![[0, 0, 0, 255]; 29];
        colors[23] = [20, 220, 40, 255];
        colors[28] = [220, 20, 200, 255];
        let ranks = horizontal_cell_strip(6, &colors);
        let mut target = surface(40, 40);
        let font = bitmap_font();

        draw_cursor_info_with_gamma(
            &mut target,
            &HudFont::Fallback(&font),
            &HudGraphics::default(),
            SurfaceRect::new(0, 0, 40, 40),
            "",
            144,
            None,
            None,
            None,
            u32::MAX,
            Some(&ranks),
            Some(24),
            false,
            0,
            None,
        );

        assert_eq!(target.get_pixel(1, 2), Some(Color::opaque(220, 20, 200)));
        assert_eq!(target.get_pixel(10, 8), Some(Color::opaque(20, 220, 40)));
    }

    #[test]
    fn cursor_info_global_rank_strip_uses_captain_for_extensions() {
        let ranks = horizontal_cell_strip(6, &[[20, 40, 220, 255], [220, 220, 20, 255]]);
        let hud = HudGraphics {
            rank: Some(ranks),
            captain: Some(solid_image(6, 6, [220, 40, 20, 255])),
            ..HudGraphics::default()
        };
        let mut target = surface(40, 40);
        let font = bitmap_font();

        draw_cursor_info_with_gamma(
            &mut target,
            &HudFont::Fallback(&font),
            &hud,
            SurfaceRect::new(0, 0, 40, 40),
            "",
            2,
            None,
            None,
            None,
            u32::MAX,
            None,
            Some(1),
            false,
            clonk_engine::HIDE_HUD_ELEMENT_CAPTAIN,
            None,
        );

        // HH_Captain gates only the standalone status column. This Captain
        // texture is the extended-rank overlay and remains part of RankImage.
        assert_eq!(target.get_pixel(1, 2), Some(Color::opaque(220, 40, 20)));
        assert_eq!(target.get_pixel(10, 8), Some(Color::opaque(20, 40, 220)));
    }

    #[test]
    fn cursor_info_rank_name_stacks_only_above_positive_rank_name() {
        let font = bitmap_font();
        let font = HudFont::Fallback(&font);
        let viewport = SurfaceRect::new(0, 0, 120, 80);
        let mut unranked = surface(120, 80);
        draw_cursor_info_with_gamma(
            &mut unranked,
            &font,
            &HudGraphics::default(),
            viewport,
            "Joe",
            0,
            Some("Captain"),
            None,
            None,
            u32::MAX,
            None,
            None,
            false,
            0,
            None,
        );
        let mut ranked = surface(120, 80);
        draw_cursor_info_with_gamma(
            &mut ranked,
            &font,
            &HudGraphics::default(),
            viewport,
            "Joe",
            1,
            Some("Captain"),
            None,
            None,
            u32::MAX,
            None,
            None,
            false,
            0,
            None,
        );

        let white_rows = |surface: &Surface| {
            surface
                .pixels()
                .chunks_exact(4)
                .enumerate()
                .filter(|(_, pixel)| *pixel == [255, 255, 255, 255])
                .map(|(index, _)| index / surface.width() as usize)
                .collect::<Vec<_>>()
        };
        let unranked_rows = white_rows(&unranked);
        let ranked_rows = white_rows(&ranked);
        let second_line = (SYMBOL_BORDER + font.line_height()) as usize;
        assert!(unranked_rows.iter().all(|row| *row < second_line));
        assert!(ranked_rows.iter().any(|row| *row < second_line));
        assert!(ranked_rows.iter().any(|row| *row >= second_line));
    }

    #[test]
    fn inventory_sections_start_at_cpp_bottom_left_origin_and_advance_by_35() {
        // DrawCursorInfo sets the contents facet to
        // (X+5, Y+Hgt-5-35, 7*35, 35), then DrawIDList advances through
        // successive height-square sections (src/C4Viewport.cpp:911-917;
        // src/C4ObjectList.cpp:343-372; src/C4Facet.cpp:44-48).
        let mut target = surface(160, 140);
        let viewport = SurfaceRect::new(10, 20, 130, 100);
        let inventory = vec![
            crate::InventoryOverlay {
                object_id: clonk_engine::ObjectId::new(1),
                definition_id: "FLAG".to_string(),
                picture: Some(solid_image(4, 4, [220, 10, 10, 255])),
                additive: false,
                picture_overlays: Vec::new(),
                count: 1,
            },
            crate::InventoryOverlay {
                object_id: clonk_engine::ObjectId::new(2),
                definition_id: "ROCK".to_string(),
                picture: Some(solid_image(4, 4, [10, 220, 10, 255])),
                additive: false,
                picture_overlays: Vec::new(),
                count: 1,
            },
        ];

        let font = bitmap_font();
        draw_inventory(&mut target, &HudFont::Fallback(&font), viewport, &inventory);

        // Origin = (10+5, 20+100-5-35) = (15,80); the second section starts
        // at x=50. Pixels immediately above/left remain untouched.
        assert_eq!(target.get_pixel(15, 80), Some(Color::opaque(220, 10, 10)));
        assert_eq!(target.get_pixel(49, 114), Some(Color::opaque(220, 10, 10)));
        assert_eq!(target.get_pixel(50, 80), Some(Color::opaque(10, 220, 10)));
        assert_eq!(target.get_pixel(84, 114), Some(Color::opaque(10, 220, 10)));
        assert_eq!(target.get_pixel(14, 80), Some(Color::opaque(0, 0, 0)));
        assert_eq!(target.get_pixel(15, 79), Some(Color::opaque(0, 0, 0)));
    }

    #[test]
    fn inventory_stack_draws_cpp_count_suffix_at_bottom_right() {
        let render = |count| {
            let mut target = surface(60, 60);
            let font = bitmap_font();
            let inventory = [crate::InventoryOverlay {
                object_id: clonk_engine::ObjectId::new(1),
                definition_id: "ROCK".to_string(),
                picture: None,
                additive: false,
                picture_overlays: Vec::new(),
                count,
            }];
            draw_inventory(
                &mut target,
                &HudFont::Fallback(&font),
                SurfaceRect::new(0, 0, 60, 60),
                &inventory,
            );
            target
        };

        let single = render(1);
        let stack = render(2);
        assert!(
            single
                .pixels()
                .chunks_exact(4)
                .all(|pixel| pixel == [0, 0, 0, 255]),
            "DrawIDList suppresses the count for exactly one item"
        );
        assert_ne!(
            stack.pixels(),
            single.pixels(),
            "a stack draws the C++ `2x` suffix"
        );
    }

    #[test]
    fn inventory_region_hit_test_uses_cpp_cell_edges() {
        // DrawIDList registers one height-square C4Region per inventory
        // section; C4RegionList::Find treats both integer edges as inside
        // (C4Viewport.cpp:911-917; C4ObjectList.cpp:343-372;
        // C4Region.cpp:87-94).
        let viewport = SurfaceRect::new(10, 20, 320, 200);
        let top = 20 + 200 - SYMBOL_BORDER - SYMBOL_SIZE;
        assert_eq!(
            inventory_region_index(viewport, clonk_gui::Point::new(15.0, top as f32), 2),
            Some(0)
        );
        assert_eq!(
            inventory_region_index(
                viewport,
                clonk_gui::Point::new(
                    (15 + SYMBOL_SIZE - 1) as f32,
                    (top + SYMBOL_SIZE - 1) as f32
                ),
                2,
            ),
            Some(0),
            "last integer pixel remains inside the first C4Region"
        );
        assert_eq!(
            inventory_region_index(
                viewport,
                clonk_gui::Point::new((15 + SYMBOL_SIZE) as f32, top as f32),
                2,
            ),
            Some(1)
        );
        assert_eq!(
            inventory_region_index(
                viewport,
                clonk_gui::Point::new((15 + 2 * SYMBOL_SIZE) as f32, top as f32),
                2,
            ),
            None
        );
        assert_eq!(
            inventory_region_rect(viewport, 1, 2),
            Some(SurfaceRect::new(50, top, 35, 35))
        );
        assert_eq!(inventory_region_rect(viewport, 2, 2), None);
    }

    #[test]
    fn mouse_caption_clamps_red_multiline_text_and_honors_region_bottom() {
        let mut target = surface(120, 100);
        let previous_clip = SurfaceRect::new(0, 0, 100, 90);
        target.set_clip(previous_clip);
        let font = HudFont::Fallback(&FixedWidthMarkerFont);
        let viewport = SurfaceRect::new(10, 20, 80, 70);
        let text = "Grab Wagon.|(Double click)";
        let rect = draw_mouse_caption(
            &mut target,
            &font,
            viewport,
            clonk_gui::Point::new(10.0, 40.0),
            Some(80),
            text,
            None,
        )
        .expect("nonempty caption draws");

        assert_eq!(rect.x, viewport.x + 1, "left edge is clamped inside");
        assert!(
            rect.x + (rect.width as i32) < viewport.x + viewport.width as i32,
            "right edge remains inside the viewport"
        );
        assert_eq!(
            rect.y + rect.height as i32,
            79,
            "CaptionBottomY leaves the native one-pixel gap"
        );
        assert_eq!(
            target.get_pixel(
                (rect.x + rect.width as i32 / 2 - font.text_width("Grab Wagon.") / 2) as u32,
                rect.y as u32,
            ),
            Some(MOUSE_CAPTION_COLOR),
            "the first centered line uses 0xfaFF0000"
        );
        assert_eq!(
            target.clip(),
            Some(previous_clip),
            "caption drawing restores its caller's clip"
        );

        let oversized = draw_mouse_caption(
            &mut target,
            &font,
            viewport,
            clonk_gui::Point::new(50.0, 40.0),
            None,
            "This caption is wider than the viewport",
            None,
        )
        .expect("oversized caption still draws with native geometry");
        assert_eq!(
            oversized.x,
            viewport.x + 1,
            "BoundBy chooses its left bound even when that exceeds the right bound"
        );
        assert_eq!(
            target.get_pixel((viewport.x + viewport.width as i32 - 1) as u32, 53),
            Some(MOUSE_CAPTION_COLOR),
            "caption reaches the last pixel inside its viewport clip"
        );
        assert_eq!(
            target.get_pixel((viewport.x + viewport.width as i32) as u32, 53),
            Some(Color::opaque(0, 0, 0)),
            "caption pixels cannot leak into an adjacent viewport"
        );
    }

    #[test]
    fn fixed_items_sit_right_aligned_with_symbol_spacing() {
        // Wealth at right - (35+5), score at right - 2*(35+5), crew at
        // right - 3*(35+5), all at y = border (src/C4Viewport.cpp:1287-1321).
        let mut target = surface(200, 100);
        let hud = HudGraphics {
            wealth: Some(solid_image(60, 30, [220, 180, 0, 255])),
            score: Some(solid_image(60, 30, [180, 90, 0, 255])),
            crew: Some(solid_image(60, 30, [0, 0, 255, 255])),
            ..HudGraphics::default()
        };
        let font = bitmap_font();
        let font = HudFont::Fallback(&font);
        let viewport = SurfaceRect::new(0, 0, 200, 100);
        draw_player_fixed_items(
            &mut target,
            &font,
            &hud,
            viewport,
            7,
            3,
            1,
            1,
            Color::opaque(255, 0, 0),
        );
        // Wealth icon: cgo (160,5,35,17), aspect-fit 34x17 at x 160.
        assert_eq!(target.get_pixel(161, 10), Some(Color::opaque(220, 180, 0)));
        // Score icon: cgo (120,5,...).
        assert_eq!(target.get_pixel(121, 10), Some(Color::opaque(180, 90, 0)));
        // Crew icon: cgo (80,5,...) — pure blue is ClrByOwner, so it takes
        // the red owner color (src/C4Viewport.cpp:1320, C4Surface.cpp:236).
        assert_eq!(target.get_pixel(81, 10), Some(Color::opaque(255, 0, 0)));
    }

    #[test]
    fn l049_fixed_items_respect_individual_visibility() {
        let hud = HudGraphics {
            wealth: Some(solid_image(60, 30, [220, 180, 0, 255])),
            score: Some(solid_image(60, 30, [180, 90, 0, 255])),
            crew: Some(solid_image(60, 30, [0, 0, 255, 255])),
            ..HudGraphics::default()
        };
        let font = bitmap_font();
        let font = HudFont::Fallback(&font);
        let viewport = SurfaceRect::new(0, 0, 200, 100);
        let black = Color::opaque(0, 0, 0);

        let mut wealth_only = surface(200, 100);
        draw_player_fixed_items_with_gamma(
            &mut wealth_only,
            &font,
            &hud,
            viewport,
            7,
            3,
            1,
            1,
            Color::opaque(255, 0, 0),
            true,
            false,
            false,
            None,
        );
        assert_eq!(
            wealth_only.get_pixel(161, 10),
            Some(Color::opaque(220, 180, 0))
        );
        assert_eq!(wealth_only.get_pixel(121, 10), Some(black));
        assert_eq!(wealth_only.get_pixel(81, 10), Some(black));

        let mut score_and_crew = surface(200, 100);
        draw_player_fixed_items_with_gamma(
            &mut score_and_crew,
            &font,
            &hud,
            viewport,
            7,
            3,
            1,
            1,
            Color::opaque(255, 0, 0),
            false,
            true,
            true,
            None,
        );
        assert_eq!(score_and_crew.get_pixel(161, 10), Some(black));
        assert_eq!(
            score_and_crew.get_pixel(121, 10),
            Some(Color::opaque(180, 90, 0))
        );
        assert_eq!(
            score_and_crew.get_pixel(81, 10),
            Some(Color::opaque(255, 0, 0))
        );
    }

    #[test]
    fn scaled_portrait_edge_blends_toward_black_not_hidden_white() {
        // C4Surface clears RGB under exact transparency before GL_LINEAR
        // samples a scaled portrait. At the 25%-opaque tap, transparent
        // black produces 191 over white; hidden white would produce 239.
        let canonical = ImageData::new(2, 1, vec![0, 0, 0, 255, 0, 0, 0, 0]);
        let hidden_white = ImageData::new(2, 1, vec![0, 0, 0, 255, 255, 255, 255, 0]);
        let mut canonical_target = surface(4, 2);
        canonical_target.fill(Color::opaque(255, 255, 255));
        let mut dirty_target = surface(4, 2);
        dirty_target.fill(Color::opaque(255, 255, 255));

        let rect = SurfaceRect::new(0, 0, 4, 2);
        draw_image_aspect(&mut canonical_target, &canonical, rect, None);
        draw_image_aspect(&mut dirty_target, &hidden_white, rect, None);

        assert_eq!(
            canonical_target.get_pixel(2, 0),
            Some(Color::opaque(191, 191, 191))
        );
        assert_eq!(
            dirty_target.get_pixel(2, 0),
            Some(Color::opaque(239, 239, 239))
        );
    }

    #[test]
    fn message_board_fills_bottom_strip_with_background_tile() {
        // C4MessageBoard::Draw background blit (src/C4MessageBoard.cpp:258).
        let mut target = surface(64, 64);
        let hud = HudGraphics {
            background: Some(solid_image(8, 8, [20, 24, 28, 255])),
            ..HudGraphics::default()
        };
        let font = bitmap_font();
        let font = HudFont::Fallback(&font);
        let strip_height = font.line_height();
        draw_message_board(&mut target, &font, &hud, &MessageBoardOverlay::default());
        assert_eq!(
            target.get_pixel(50, 64 - 1),
            Some(Color::opaque(20, 24, 28))
        );
        assert_eq!(
            target.get_pixel(50, (64 - strip_height - 1) as u32),
            Some(Color::opaque(0, 0, 0))
        );
    }

    #[test]
    fn l120_message_board_background_tile_is_screen_anchored() {
        let mut target = surface(8, 10);
        let hud = HudGraphics {
            background: Some(ImageData::new(
                1,
                3,
                vec![200, 0, 0, 255, 0, 200, 0, 255, 0, 0, 200, 255],
            )),
            ..HudGraphics::default()
        };
        let fallback = FixedWidthMarkerFont;
        let font = HudFont::Fallback(&fallback);

        draw_message_board(&mut target, &font, &hud, &MessageBoardOverlay::default());

        // Output.Y is 4, so the first board row continues screen row 4 of
        // the three-row tile instead of restarting at source row zero.
        assert_eq!(target.get_pixel(7, 4), Some(Color::opaque(0, 200, 0)));
        assert_eq!(target.get_pixel(7, 5), Some(Color::opaque(0, 0, 200)));
        assert_eq!(target.get_pixel(7, 6), Some(Color::opaque(200, 0, 0)));
    }

    #[test]
    fn l120_single_line_fader_offsets_and_alpha_fades_older_text() {
        let mut target = surface(64, 24);
        let hud = HudGraphics {
            background: Some(solid_image(2, 2, [0, 0, 0, 255])),
            ..HudGraphics::default()
        };
        let fallback = FixedWidthMarkerFont;
        let font = HudFont::Fallback(&fallback);
        let line_height = font.line_height();
        let output_y = target.height() as i32 - line_height;
        let board = MessageBoardOverlay {
            log_lines: vec!["old".to_string(), "new".to_string()],
            back_scroll: 0,
            fader: 3,
            ..MessageBoardOverlay::default()
        };

        draw_message_board(&mut target, &font, &hud, &board);

        assert_eq!(
            target.get_pixel(0, (output_y + 3) as u32),
            Some(Color::opaque(255, 255, 255)),
            "Fader vertically offsets the current line"
        );
        assert_eq!(
            target.get_pixel(0, (output_y - 3) as u32),
            Some(Color::new(255, 255, 255, 234)),
            "the preceding line keeps native integer alpha fade ordering"
        );
    }

    #[test]
    fn l120_continuous_message_board_draws_multiple_lines_over_opaque_dynamic_strip() {
        let mut target = surface(96, 96);
        let background_color = Color::opaque(20, 24, 28);
        let hud = HudGraphics {
            background: Some(solid_image(8, 8, [20, 24, 28, 255])),
            ..HudGraphics::default()
        };
        let fallback = bitmap_font();
        let font = HudFont::Fallback(&fallback);
        let line_height = font.line_height();
        let board = MessageBoardOverlay {
            mode: MessageBoardMode::Continuous,
            line_count: 3,
            log_lines: vec![
                "Alpha".to_string(),
                "Bravo".to_string(),
                "Charlie".to_string(),
            ],
            back_scroll: 0,
            ..MessageBoardOverlay::default()
        };
        let output_height = board.output_height(line_height);
        let output_y = target.height() as i32 - output_height;

        draw_message_board(&mut target, &font, &hud, &board);

        assert_eq!(output_height, 4 * line_height);
        assert_eq!(
            target.get_pixel(90, 95),
            Some(background_color),
            "continuous mode tiles the complete (iLines + 1)-line output"
        );
        assert_eq!(
            target.get_pixel(90, (output_y - 1) as u32),
            Some(Color::opaque(0, 0, 0)),
            "the opaque backdrop begins at MessageBoard.Output.Y"
        );

        let has_opaque_text_in_band = |start_y: i32| {
            (start_y..start_y + line_height).any(|y| {
                (0..target.width()).any(|x| {
                    target
                        .get_pixel(x, y as u32)
                        .is_some_and(|pixel| pixel == Color::opaque(255, 255, 255))
                })
            })
        };
        assert!(has_opaque_text_in_band(output_y));
        assert!(has_opaque_text_in_band(output_y + line_height));
        assert!(
            !(output_y - line_height..output_y).any(|y| {
                (0..target.width()).any(|x| {
                    target
                        .get_pixel(x, y as u32)
                        .is_some_and(|pixel| pixel == Color::opaque(255, 255, 255))
                })
            }),
            "the loop's extra line above cgo.Y reaches zero alpha exactly"
        );
    }

    #[test]
    fn hud_image_leaves_gamma_sample_before_translucent_blending() {
        let mut target = surface(12, 2);
        target.fill(Color::opaque(200, 200, 200));
        let source = Color::new(64, 128, 192, 128);
        let image = solid_image(1, 1, [source.r, source.g, source.b, source.a]);
        let gamma = clonk_graphics::GammaRamp::from_control_points([0x000000, 0x646464, 0xc8c8c8]);

        fill_hud_rect(
            &mut target,
            &clonk_gui::Rect::new(0.0, 0.0, 2.0, 2.0),
            Color::opaque(source.r, source.g, source.b),
            Some(&gamma),
        );
        draw_hud_image_strip(&mut target, 3, 0, &image, 0, 0, 1, 1, Some(&gamma));
        draw_hud_image_bilinear(
            &mut target,
            &clonk_gui::Rect::new(6.0, 0.0, 1.0, 1.0),
            &image,
            Some(&gamma),
        );
        draw_scaled_region(
            &mut target,
            &image,
            SurfaceRect::new(0, 0, 1, 1),
            SurfaceRect::new(9, 0, 1, 1),
            Some(&gamma),
        );

        assert_eq!(target.get_pixel(0, 0), Some(Color::new(50, 100, 150, 255)));
        let expected = Some(Color::new(125, 150, 175, 255));
        for x in [3, 6, 9] {
            assert_eq!(target.get_pixel(x, 0), expected, "HUD leaf at x={x}");
        }
    }

    #[test]
    fn hud_clonk_and_fallback_text_use_independent_gamma_channels() {
        let gamma = clonk_graphics::GammaRamp::from_control_points([0x102030, 0x405060, 0x708090]);
        let encoded = [17, 33, 49, 255];

        let mut fallback_surface = surface(24, 16);
        fallback_surface.fill(Color::opaque(200, 200, 200));
        let fallback = bitmap_font();
        HudFont::Fallback(&fallback).draw_with_gamma(
            &mut fallback_surface,
            0,
            0,
            "X",
            Color::opaque(0, 0, 0),
            TextAlign::Left,
            Some(&gamma),
        );
        assert!(fallback_surface
            .pixels()
            .chunks_exact(4)
            .any(|pixel| pixel == encoded));

        let mut clonk = clonk_graphics::clonk_font::ClonkFont::new(1);
        clonk.add_glyph(
            'X',
            clonk_graphics::clonk_font::GlyphCell {
                width: 1,
                pixels: vec![Color::opaque(255, 255, 255), Color::transparent()],
            },
        );
        let mut clonk_surface = surface(2, 2);
        clonk_surface.fill(Color::opaque(200, 200, 200));
        HudFont::Clonk(&clonk).draw_with_gamma(
            &mut clonk_surface,
            0,
            0,
            "X",
            Color::opaque(0, 0, 0),
            TextAlign::Left,
            Some(&gamma),
        );
        assert_eq!(
            clonk_surface.get_pixel(0, 0),
            Some(Color::new(17, 33, 49, 255))
        );
    }

    #[test]
    fn tutorial_six_controls_compose_with_the_scenario_gamma() {
        // Tutorial06/Script.c:23-24 selects top controls 3..8 with labels;
        // Initialize applies the grey 0/100/200 gamma curve at Script.c:13.
        let controls = "___345678_   345678 __________";
        let show_control = controls
            .bytes()
            .enumerate()
            .filter(|(_, byte)| !matches!(byte, b'_' | b' '))
            .fold(0i32, |mask, (position, _)| mask | (1i32 << position));
        let hud = HudGraphics {
            control: Some(crate::test_support::load_graphics_png("Control.png")),
            ..HudGraphics::default()
        };
        let fonts = crate::test_support::endeavour_font_set();
        let labels = ["Z", "S", "X", "C", "A", "D", "Q", "W", "E", "R"].map(str::to_string);
        let gamma = clonk_graphics::GammaRamp::from_control_points([0x000000, 0x646464, 0xc8c8c8]);
        let render = |gamma| {
            let mut target = surface(320, 240);
            target.fill(Color::opaque(200, 200, 200));
            draw_player_controls_with_gamma(
                &mut target,
                &HudFont::Clonk(&fonts.text),
                &HudFont::Clonk(&fonts.mini),
                &hud,
                SurfaceRect::new(0, 0, 320, 240),
                show_control,
                1,
                0,
                &labels,
                0,
                gamma,
            );
            target.snapshot().checksum()
        };

        assert_eq!(
            (render(None), render(Some(&gamma))),
            (727_770_473, 366_450_976)
        );
    }
}
