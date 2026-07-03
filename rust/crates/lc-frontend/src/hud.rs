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
//!   (src/C4Viewport.cpp:1446-1476) — keyboard graphic and player name in
//!   the player color.
//! - Message board: `C4MessageBoard::Draw` (src/C4MessageBoard.cpp:243-306)
//!   — one log line over the tiled background strip at the screen bottom.

use crate::{
    draw_image_bilinear, draw_image_strip, fill_rect, ClonkFontSet, HudGraphics, ImageData,
};
use lc_graphics::{clonk_font::TextAlign, Color, Rect as SurfaceRect, Surface, TextFont};

/// `C4UpperBoardHeight` (src/C4Constants.h:77): the screen strip reserved
/// above the viewports in `C4GraphicsSystem::RecalculateViewports`
/// (src/C4GraphicsSystem.cpp:345).
pub const UPPER_BOARD_HEIGHT: i32 = 50;
/// `C4SymbolSize` / `C4SymbolBorder` (src/C4Constants.h:75-76).
pub const SYMBOL_SIZE: i32 = 35;
pub const SYMBOL_BORDER: i32 = 5;
/// `DrawMessageOffset` (src/C4GameMessage.cpp:95).
pub const DRAW_MESSAGE_OFFSET: i32 = -35;
/// `fctKeyboard` cell inside Control.png (src/C4GraphicsResource.cpp:201).
const KEYBOARD_CELL: (u32, u32) = (80, 36);
/// White HUD text (`CStdDDraw::DEFAULT_MESSAGE_COLOR`, src/StdDDraw2.h:361).
const MESSAGE_COLOR: Color = Color::opaque(255, 255, 255);
/// FontRegular is the 14px main font (`C4Fonts.cpp:280-288`); the fallback
/// TextFont path draws at the same pixel size.
const FALLBACK_FONT_SIZE: f32 = 14.0;

/// `Game.GraphicsResource.FontRegular` for the HUD: the CStdFont-faithful
/// Clonk font when loaded, else the generic [`TextFont`] fallback.
pub enum HudFont<'a> {
    Clonk(&'a lc_graphics::clonk_font::ClonkFont),
    Fallback(&'a dyn TextFont),
}

impl HudFont<'_> {
    pub fn from_set<'a>(
        set: Option<&'a ClonkFontSet>,
        fallback: &'a dyn TextFont,
    ) -> HudFont<'a> {
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
        match self {
            HudFont::Clonk(font) => font.draw(
                surface,
                x,
                y,
                text,
                [color.r, color.g, color.b, color.a],
                align,
                false,
            ),
            HudFont::Fallback(font) => {
                let width = self.text_width(text);
                let origin = match align {
                    TextAlign::Left => x,
                    TextAlign::Center => x - width / 2,
                    TextAlign::Right => x - width,
                };
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
fn blit_tile(surface: &mut Surface, image: &ImageData, x: i32, y: i32, width: i32, height: i32) {
    let (tile_w, tile_h) = (image.width() as i32, image.height() as i32);
    if tile_w <= 0 || tile_h <= 0 {
        return;
    }
    let mut ty = 0;
    while ty < height {
        let src_h = tile_h.min(height - ty);
        let mut tx = 0;
        while tx < width {
            let src_w = tile_w.min(width - tx);
            draw_image_strip(
                surface,
                x + tx,
                y + ty,
                image,
                0,
                0,
                src_w as u32,
                src_h as u32,
                None,
            );
            tx += tile_w;
        }
        ty += tile_h;
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

fn draw_image_aspect(surface: &mut Surface, image: &ImageData, rect: SurfaceRect) {
    let target = aspect_fit(image.width() as i32, image.height() as i32, rect);
    draw_image_bilinear(
        surface,
        &lc_gui::Rect::new(
            target.x as f32,
            target.y as f32,
            target.width as f32,
            target.height as f32,
        ),
        image,
        None,
    );
}

/// `ClrByOwner` (src/C4Surface.cpp:236-287): blue-hued pixels — hue in
/// [145,175] of the 255-scaled HLS wheel (blue = 170), saturation > 100 —
/// are ColorByOwner pixels. Returns the gray value C++ keeps in the
/// overlay surface: the blue channel (`GetRValue(dwClr)` on the engine's
/// 0xAARRGGBB pixels, src/C4Surface.cpp:283-285).
fn clr_by_owner_gray(r: i32, g: i32, b: i32) -> Option<u8> {
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
        ((c_max - c_min) * HLSMAX + (2 * RGBMAX - c_max - c_min) / 2)
            / (2 * RGBMAX - c_max - c_min)
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

/// `C4FacetExSurface::CreateClrByOwner` + `DrawValue2Clr` combined
/// (src/C4Surface.cpp:288-318, src/C4Facet.cpp:151-157): blue ClrByOwner
/// pixels become the owner color modulated by their gray value.
pub fn colorize_by_owner(image: &ImageData, owner: Color) -> ImageData {
    let pixels = image.pixels();
    let mut out = Vec::with_capacity(pixels.len());
    for chunk in pixels.chunks_exact(4) {
        let (r, g, b, a) = (chunk[0], chunk[1], chunk[2], chunk[3]);
        match clr_by_owner_gray(r as i32, g as i32, b as i32) {
            Some(gray) => {
                let modulate = |c: u8| ((c as u16 * gray as u16) / 255) as u8;
                out.extend_from_slice(&[
                    modulate(owner.r),
                    modulate(owner.g),
                    modulate(owner.b),
                    a,
                ]);
            }
            None => out.extend_from_slice(&[r, g, b, a]),
        }
    }
    ImageData::new(image.width(), image.height(), out)
}

/// `C4UpperBoard::Draw` (src/C4UpperBoard.cpp:46-96) in `Full` mode.
pub fn draw_upper_board(
    surface: &mut Surface,
    font: &HudFont<'_>,
    hud: &HudGraphics,
    scenario_title: &str,
    game_time_seconds: u64,
) {
    let width = surface.width() as i32;
    // Output.Hgt = max(C4UpperBoardHeight, fctUpperBoard.Hgt)
    // (C4UpperBoard::Init, src/C4UpperBoard.cpp:117-120).
    let board_height = hud
        .upper_board
        .as_ref()
        .map(|image| (image.height() as i32).max(UPPER_BOARD_HEIGHT))
        .unwrap_or(UPPER_BOARD_HEIGHT);

    match hud.upper_board.as_ref() {
        Some(board) => blit_tile(surface, board, 0, 0, width, board_height),
        None => fill_rect(
            surface,
            &lc_gui::Rect::new(0.0, 0.0, width as f32, board_height as f32),
            Color::opaque(66, 44, 24),
        ),
    }

    // Logo (src/C4UpperBoard.cpp:54-71).
    if let Some(logo) = hud.logo.as_ref() {
        let (logo_w, logo_h) = (logo.width() as f32, logo.height() as f32);
        if logo_w > 0.0 && logo_h > 0.0 {
            let mut zoom = if logo_w / logo_h != 3.0 { 0.25 } else { 0.21 };
            zoom *= 960.0 / logo_w;
            let dst_w = (logo_w * zoom) as i32;
            let dst_h = (logo_h * zoom) as i32;
            let dst_x = (width as f32 / 2.0 - (logo_w / 2.0) * zoom) as i32;
            draw_image_bilinear(
                surface,
                &lc_gui::Rect::new(dst_x as f32, 0.0, dst_w as f32, dst_h as f32),
                logo,
                None,
            );
        }
    }

    // Text rows center on the reserved 50px strip, not the texture height
    // (TextYPosition, src/C4UpperBoard.cpp:126).
    let text_y = UPPER_BOARD_HEIGHT / 2 - font.line_height() / 2;
    let time_text = format_game_time(game_time_seconds);
    let time_width = font.text_width(&time_text);
    font.draw(
        surface,
        width - time_width - 10,
        text_y,
        &time_text,
        MESSAGE_COLOR,
        TextAlign::Left,
    );
    font.draw(
        surface,
        10,
        text_y,
        scenario_title,
        MESSAGE_COLOR,
        TextAlign::Left,
    );
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
) {
    if let Some(icon) = icon {
        draw_image_aspect(surface, icon, cgo);
    }
    font.draw(
        surface,
        cgo.x + cgo.width as i32 - 1,
        cgo.y + cgo.height as i32 - 1,
        text,
        MESSAGE_COLOR,
        TextAlign::Right,
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
    let (wdt, hgt) = (SYMBOL_SIZE, SYMBOL_SIZE / 2);
    let right = viewport.x + viewport.width as i32;
    let top = viewport.y + SYMBOL_BORDER;

    // Wealth (src/C4Viewport.cpp:1287-1296).
    let cgo = SurfaceRect::new(right - wdt - SYMBOL_BORDER, top, wdt as u32, hgt as u32);
    draw_value(surface, font, hud.wealth.as_ref(), &wealth.to_string(), cgo);

    // Value gain / score (src/C4Viewport.cpp:1299-1309).
    let cgo = SurfaceRect::new(
        right - 2 * wdt - 2 * SYMBOL_BORDER,
        top,
        wdt as u32,
        hgt as u32,
    );
    draw_value(surface, font, hud.score.as_ref(), &score.to_string(), cgo);

    // Crew (src/C4Viewport.cpp:1312-1321): fctCrewClr colored by the
    // player color, "SelectCount/ActiveCrewCount".
    let cgo = SurfaceRect::new(
        right - 3 * wdt - 3 * SYMBOL_BORDER,
        top,
        wdt as u32,
        hgt as u32,
    );
    let crew_icon = hud
        .crew
        .as_ref()
        .map(|icon| colorize_by_owner(icon, owner_color));
    draw_value(
        surface,
        font,
        crew_icon.as_ref(),
        &format!("{select_count}/{crew_count}"),
        cgo,
    );
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
    if let Some(portrait) = portrait {
        let rect = SurfaceRect::new(
            cgo.x + ix,
            cgo.y,
            (4 * SYMBOL_SIZE / 3 + 10) as u32,
            (SYMBOL_SIZE + 10) as u32,
        );
        draw_image_aspect(surface, portrait, rect);
        ix += 4 * SYMBOL_SIZE / 3;
    }

    // Rank symbol: C4RankSystem::DrawRankSymbol draws the phase cell 1:1
    // (src/C4RankSystem.cpp:305-307); cells are height-square
    // (C4FCT_Height load, src/C4GraphicsResource.cpp:215).
    let symbols = rank_symbols.or(hud.rank.as_ref());
    if let Some(symbols) = symbols {
        let cell = symbols.height();
        if cell > 0 && symbols.width() >= cell {
            let count = (symbols.width() / cell).max(1) as i32;
            let base_rank = rank.max(0) % count;
            draw_image_strip(
                surface,
                cgo.x + ix,
                cgo.y,
                symbols,
                base_rank as u32 * cell,
                0,
                cell,
                cell,
                None,
            );
            ix += cell as i32;
        }
    }

    // Name (src/C4ObjectInfo.cpp:353-370) — DEFAULT_MESSAGE_COLOR, left.
    if !name.is_empty() {
        font.draw(surface, cgo.x + ix, cgo.y, name, MESSAGE_COLOR, TextAlign::Left);
    }
}

/// `C4Object::DrawEnergy` → `C4Facet::DrawEnergyLevelEx`
/// (src/C4Viewport.cpp:921-945, src/C4Facet.cpp:334-389): the vertical
/// bar left of the viewport. `EnergyBars.png` is a 6x3 cell grid — column
/// `bar_idx*2` filled, `+1` empty; rows top cap / middle tile / bottom cap
/// (src/C4GraphicsResource.cpp:236-241).
pub fn draw_energy_bar(
    surface: &mut Surface,
    hud: &HudGraphics,
    viewport: SurfaceRect,
    energy_fraction: f32,
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
    // iYOff = 10 with portraits shown (src/C4Viewport.cpp:927).
    let y_off = 10;
    let x = viewport.x + SYMBOL_BORDER;
    let y = viewport.y + SYMBOL_SIZE + 2 * SYMBOL_BORDER + y_off;
    let height = vp_height - 3 * SYMBOL_BORDER - 2 * SYMBOL_SIZE - y_off;
    if height <= 0 {
        return;
    }

    // yBar = Hgt - level*Hgt/range (src/C4Facet.cpp:339).
    let fraction = energy_fraction.clamp(0.0, 1.0);
    let y_bar = height - (fraction * height as f32).round() as i32;
    let cell_h_i = cell_h as i32;
    for row in 0..height {
        let (vidx, dy) = if row >= height - cell_h_i {
            (2, row + cell_h_i - height)
        } else if row < cell_h_i {
            (0, row)
        } else {
            (1, row % cell_h_i)
        };
        let column = if row >= y_bar { 0 } else { 1 };
        draw_image_strip(
            surface,
            x,
            y + row,
            bars,
            column * cell_w,
            vidx as u32 * cell_h + dy as u32,
            cell_w,
            1,
            None,
        );
    }
}

/// `C4Viewport::DrawPlayerStartup` (src/C4Viewport.cpp:1446-1476):
/// keyboard graphic + player name in the player color.
pub fn draw_player_startup(
    surface: &mut Surface,
    font: &HudFont<'_>,
    hud: &HudGraphics,
    viewport: SurfaceRect,
    player_name: &str,
    player_color: Color,
) {
    let (cell_w, cell_h) = KEYBOARD_CELL;
    let mut name_height_off = 0;
    if let Some(control) = hud.control.as_ref() {
        if control.width() >= cell_w && control.height() >= cell_h {
            // fctKeyboard phase 0 = keyboard set 1
            // (src/C4Viewport.cpp:1461-1466).
            draw_image_strip(
                surface,
                viewport.x + (viewport.width as i32 - cell_w as i32) / 2,
                viewport.y + viewport.height as i32 * 2 / 3 + DRAW_MESSAGE_OFFSET,
                control,
                0,
                0,
                cell_w,
                cell_h,
                None,
            );
            name_height_off = cell_h as i32;
        }
    }
    // Name in ColorDw | 0xff000000, centered (src/C4Viewport.cpp:1471-1475).
    font.draw(
        surface,
        viewport.x + viewport.width as i32 / 2,
        viewport.y + viewport.height as i32 * 2 / 3 + name_height_off + DRAW_MESSAGE_OFFSET,
        player_name,
        Color::opaque(player_color.r, player_color.g, player_color.b),
        TextAlign::Center,
    );
}

/// `C4MessageBoard::Draw` in one-line mode (src/C4MessageBoard.cpp:243-306):
/// tiled `fctBackground` strip at the screen bottom with the current log
/// line at its top-left.
pub fn draw_message_board(
    surface: &mut Surface,
    font: &HudFont<'_>,
    hud: &HudGraphics,
    line: Option<&str>,
) {
    let height = font.line_height();
    let width = surface.width() as i32;
    let y = surface.height() as i32 - height;
    match hud.background.as_ref() {
        Some(background) => blit_tile(surface, background, 0, y, width, height),
        None => fill_rect(
            surface,
            &lc_gui::Rect::new(0.0, y as f32, width as f32, height as f32),
            Color::opaque(0, 0, 0),
        ),
    }
    if let Some(line) = line.filter(|line| !line.is_empty()) {
        // iMsgY = cgo.Y + (iMsg + iLines-1)*iLineHgt + Fader with the
        // current message at iMsg = -1, iLines = 2, Fader = 0
        // (src/C4MessageBoard.cpp:271-303).
        font.draw(surface, 0, y, line, MESSAGE_COLOR, TextAlign::Left);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use lc_graphics::PixelFormat;

    fn solid_image(width: u32, height: u32, color: [u8; 4]) -> ImageData {
        let pixels = color
            .iter()
            .copied()
            .cycle()
            .take((width * height * 4) as usize)
            .collect::<Vec<u8>>();
        ImageData::new(width, height, pixels)
    }

    fn surface(width: u32, height: u32) -> Surface {
        let mut surface = Surface::new(width, height, PixelFormat::Rgba8888);
        surface.fill(Color::opaque(0, 0, 0));
        surface
    }

    fn bitmap_font() -> lc_graphics::BitmapFont {
        lc_graphics::BitmapFont::new()
    }

    #[test]
    fn formats_game_time_like_upper_board_execute() {
        // C4UpperBoard::Execute (src/C4UpperBoard.cpp:41).
        assert_eq!(format_game_time(0), "00:00:00");
        assert_eq!(format_game_time(18), "00:00:18");
        assert_eq!(format_game_time(3600 + 2 * 60 + 3), "01:02:03");
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
        draw_upper_board(&mut target, &font, &hud, "", 0);
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
        draw_upper_board(&mut target, &font, &hud, "", 0);
        // dst x = 400/2 - 480*0.21 = 99.2 -> 99, width 201, height 67.
        assert_eq!(target.get_pixel(98, 30), Some(Color::opaque(120, 80, 40)));
        assert_eq!(target.get_pixel(100, 30), Some(Color::opaque(10, 200, 30)));
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
        // Non-blue pixels pass through untouched.
        let image = solid_image(1, 1, [200, 30, 30, 255]);
        let colored = colorize_by_owner(&image, Color::opaque(255, 0, 0));
        assert_eq!(&colored.pixels()[..4], &[200, 30, 30, 255]);
    }

    #[test]
    fn energy_bar_draws_filled_column_from_the_bottom() {
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
        draw_energy_bar(&mut target, &hud, viewport, 0.5);
        // Bar spans y = 55 .. 55 + (200 - 95) = 160; yBar at half.
        let bar_top = SYMBOL_SIZE + 2 * SYMBOL_BORDER + 10;
        let bar_height = 200 - 3 * SYMBOL_BORDER - 2 * SYMBOL_SIZE - 10;
        let x = SYMBOL_BORDER as u32;
        assert_eq!(
            target.get_pixel(x, (bar_top + 1) as u32),
            Some(Color::opaque(60, 60, 60)),
            "top of a half-full bar is empty"
        );
        assert_eq!(
            target.get_pixel(x, (bar_top + bar_height - 2) as u32),
            Some(Color::opaque(200, 0, 0)),
            "bottom of a half-full bar is filled"
        );
    }

    #[test]
    fn cursor_info_places_portrait_and_rank_symbol_like_c4objectinfo_draw() {
        // Portrait aspect-fit into (border, border, 4*35/3+10, 45) and the
        // rank cell 1:1 at iX = 4*35/3 (src/C4ObjectInfo.cpp:308-341).
        let mut target = surface(200, 200);
        let hud = HudGraphics::default();
        let portrait = solid_image(150, 150, [10, 200, 30, 255]);
        // Two 4x4 rank cells: rank 0 blue, rank 1 yellow.
        let mut rank_pixels = Vec::new();
        for _row in 0..4 {
            rank_pixels.extend(std::iter::repeat_n([0u8, 0, 220, 255], 4).flatten());
            rank_pixels.extend(std::iter::repeat_n([220u8, 220, 0, 255], 4).flatten());
        }
        let ranks = ImageData::new(8, 4, rank_pixels);
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
        draw_message_board(&mut target, &font, &hud, None);
        assert_eq!(
            target.get_pixel(50, 64 - 1),
            Some(Color::opaque(20, 24, 28))
        );
        assert_eq!(
            target.get_pixel(50, (64 - strip_height - 1) as u32),
            Some(Color::opaque(0, 0, 0))
        );
    }
}
