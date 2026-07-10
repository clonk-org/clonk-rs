//! Pixel-parity renderer for one C++ startup dialog (see
//! `rust/target/parity-specs/`). Implemented against the engine's F9
//! reference captures; owned by its implementation agent.
//!
//! This file renders `C4StartupPlrSelDlg` (player selection) in its
//! first-shown state, mirroring `src/C4StartupPlrSelDlg.cpp` and the C4GUI
//! widgets it instantiates. All geometry uses C++ integer math; all blits go
//! through the CStdDDraw-faithful helpers in this crate.

use crate::clonk_fonts::ClonkFontSet;
use crate::startup_main_menu::{draw_bar, IntRect};
use crate::{GuiPoint, ImageData, KeyCode};
use anyhow::{Context, Result};
use freetype::face::LoadFlag;
use freetype::Library;
use lc_graphics::clonk_font::{line_height_for, ClonkFont, GlyphCell, TextAlign};
use lc_graphics::{Color, GammaRamp, Surface};
use lc_gui::Rect as GuiRect;

/// Pixel-exact C4StartupPlrSelDlg geometry, all in C++ integer math and
/// screen coordinates (dialog-client coordinates shifted by the client
/// origin).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct PlrSelLayout {
    /// Fullscreen-dialog client rect: margins X = w/50, Y = h*2/75
    /// (C4GuiDialogs.cpp:819-820) but top = h/7
    /// (C4StartupPlrSelDlg.h:221 `GetMarginTop`).
    pub client: IntRect,
    /// Player list box bounds (C4StartupPlrSelDlg.cpp:558).
    pub plr_list: IntRect,
    /// List box client area: 3px margins all sides (C4GuiListBox.h:120-123).
    pub list_client: IntRect,
    /// Width of the list scroll window client = item width
    /// (list client minus the 16px scrollbar, C4Gui.h:111).
    pub item_width: i32,
    /// Height of one player list item: BookFont line height 22 + 2*2
    /// (C4StartupPlrSelDlg.cpp:81-82).
    pub item_height: i32,
    /// Vertical pitch between items: item height + 1px spacing
    /// (C4GUI_DefaultListSpacing, C4Gui.h:129).
    pub item_pitch: i32,
    /// Selection info text window bounds (C4StartupPlrSelDlg.cpp:559).
    pub info_window: IntRect,
    /// Info text origin/extent: TextWindow margins T8 L10 R5 B8
    /// (C4Gui.h:1334-1337) minus the 16px scrollbar reservation.
    pub info_client: IntRect,
    /// Portrait picture area (C4StartupPlrSelDlg.cpp:560-562).
    pub picture_area: IntRect,
    /// The six bottom buttons (UpdateBottomButtons,
    /// C4StartupPlrSelDlg.cpp:651-656).
    pub buttons: [IntRect; 6],
    /// Centered anchor of the fullscreen title label
    /// (FullscreenDialog::SetTitle non-woodbar path, C4GuiDialogs.cpp:843-847).
    pub title_anchor: (i32, i32),
}

/// Line height of the GUI TitleFont (Endeavour 22px → 34,
/// StdFont.cpp:351); the title label's y position depends on it.
const TITLE_FONT_LINE_HEIGHT: i32 = 34;
/// Startup BookFont (14px shadowless) line height (StdFont.cpp:351).
const BOOK_FONT_LINE_HEIGHT: i32 = 22;

/// Computes the C4StartupPlrSelDlg layout for a `w`x`h` screen.
///
/// Mirrors C4StartupPlrSelDlg.cpp:550-562 (ctor geometry),
/// C4StartupPlrSelDlg.cpp:636-657 (bottom buttons via GetGridCell,
/// C4Gui.cpp:1059-1080), C4GuiDialogs.cpp:819-822 (fullscreen margins) and
/// C4GuiContainers.cpp:301-307 / C4GuiListBox.h:120-123 (client rects).
pub fn plrsel_layout(w: i32, h: i32) -> PlrSelLayout {
    // Fullscreen dialog margins (C4GuiDialogs.cpp:819-820); the top margin is
    // overridden to rcBounds.Hgt/7 (C4StartupPlrSelDlg.h:221).
    let margin_x = if w < 500 { 2 } else { w / 50 };
    let margin_y = if h < 320 { 2 } else { h * 2 / 75 };
    let margin_top = h / 7;
    let client = IntRect {
        x: margin_x,
        y: margin_top,
        w: w - 2 * margin_x,
        h: h - margin_top - margin_y,
    };

    // Ctor math over the zero-based client (C4StartupPlrSelDlg.cpp:550-562).
    let button_height = 32; // C4GUI_ButtonHgt (C4Gui.h:119)
    let button_x_spacing = if client.w > 700 { client.w / 58 } else { 2 };
    let button_area_h = (client.h / 15).max(button_height);
    // caButtonArea = GetFromBottom (C4Gui.cpp:1025-1043); rcBottomButtons =
    // GetCentered(clientWdt, 32) (C4Gui.cpp:1049-1057).
    let button_area_y = client.h - button_area_h;
    let bottom_buttons_y = button_area_y + button_area_h / 2 - button_height / 2;
    let bottom_button_w = (client.w - button_x_spacing * 5) / 6;
    // rcMain = caMain.GetAll() after GetFromBottom shrank it by the button
    // area (C4Gui.cpp:1031,1041-1047).
    let main_w = client.w;
    let main_h = client.h - button_area_h;

    let plr_list_rel = IntRect {
        x: main_w / 10,
        y: main_h * 10 / 36,
        w: main_w * 25 / 81,
        h: main_h * 2 / 3,
    };
    let info_rel = IntRect {
        x: main_w * 371 / 768,
        y: main_h * 197 / 451,
        w: main_w * 121 / 384,
        h: main_h * 242 / 451,
    };
    let picture_w = (main_w * 121 / 384).min(200);
    let picture_h = picture_w * 3 / 4;
    let picture_rel = IntRect {
        x: main_w * 613 / 768 - picture_w,
        y: main_h * 197 / 451 - picture_h,
        w: picture_w,
        h: picture_h,
    };

    let at_screen = |r: IntRect| IntRect {
        x: client.x + r.x,
        y: client.y + r.y,
        w: r.w,
        h: r.h,
    };
    let plr_list = at_screen(plr_list_rel);
    // ListBox client: 3px margins (C4GuiListBox.h:120-123).
    let list_client = IntRect {
        x: plr_list.x + 3,
        y: plr_list.y + 3,
        w: plr_list.w - 6,
        h: plr_list.h - 6,
    };
    let info_window = at_screen(info_rel);
    // TextWindow margins T8 L10 R5 B8 (C4Gui.h:1334-1337); the scroll window
    // reserves the 16px scrollbar (C4GuiContainers.cpp:477-491).
    let info_client = IntRect {
        x: info_window.x + 10,
        y: info_window.y + 8,
        w: info_window.w - 10 - 5 - 16,
        h: info_window.h - 8 - 8,
    };

    // GetGridCell(i,6,0,1,bw,32,centered) over rcBottomButtons
    // (C4Gui.cpp:1059-1080): sector w = clientWdt/6, centered shrink to bw.
    let cell_w = client.w / 6;
    let mut buttons = [IntRect::default(); 6];
    for (i, rect) in buttons.iter_mut().enumerate() {
        *rect = IntRect {
            x: client.x + cell_w * i as i32 + (cell_w - bottom_button_w) / 2,
            y: client.y + bottom_buttons_y,
            w: bottom_button_w,
            h: button_height,
        };
    }

    PlrSelLayout {
        client,
        plr_list,
        list_client,
        item_width: list_client.w - 16,
        item_height: BOOK_FONT_LINE_HEIGHT + 4,
        item_pitch: BOOK_FONT_LINE_HEIGHT + 4 + 1,
        info_window,
        info_client,
        picture_area: at_screen(picture_rel),
        buttons,
        // Title label: x0 = clientWdt/2 (ACenter), y = C4UpperBoardHeight/2 -
        // TitleFont.lh/2 - GetMarginTop() (C4GuiDialogs.cpp:843-847).
        title_anchor: (
            client.x + client.w / 2,
            client.y + 25 - TITLE_FONT_LINE_HEIGHT / 2 - margin_top,
        ),
    }
}

// ---------------------------------------------------------------------------
// Startup book fonts (shadowless)
// ---------------------------------------------------------------------------

/// The startup "book" fonts: `C4StartupGraphics::InitFonts` initializes them
/// with `fDoShadow = false` (C4Startup.cpp:94-116). The player selection
/// dialog uses `BookFontCapt` (16px) and `BookFont` (14px).
pub struct BookFontSet {
    /// `BookFontCapt` — C4FT_Caption, 16px, line height 25.
    pub caption: ClonkFont,
    /// `BookFont` — C4FT_Main, 14px, line height 22.
    pub text: ClonkFont,
}

/// Rasterizes one *shadowless* ClonkFont at `px_height`, mirroring
/// `CStdFont::Init`/`AddRenderedChar` with `fDoShadow = false`
/// (StdFont.cpp:319-358,182-258): `iHSpace = 0`, `iGfxLineHgt = iLineHgt`
/// (no shadow row), cell width without the +1 shadow column, and every atlas
/// pixel is pure white with alpha = FreeType coverage (the shadow kernel is
/// skipped, so `BltAlpha` reduces to the white source over a transparent
/// base).
fn build_book_font(face: &freetype::Face, px_height: u32) -> Result<ClonkFont> {
    face.set_pixel_sizes(px_height, px_height)
        .context("FT_Set_Pixel_Sizes failed")?;

    let raw = face.raw();
    let units_per_em = i32::from(raw.units_per_EM);
    let (ascender, descender) = (i32::from(raw.ascender), i32::from(raw.descender));
    let line_height = line_height_for(ascender, descender, units_per_em, px_height);
    let cell_height = line_height as usize; // iGfxLineHgt = iLineHgt + 0
    let ascent_px = i64::from(px_height) * i64::from(ascender) / i64::from(units_per_em);

    let mut font = ClonkFont::new(line_height);
    font.cell_height = line_height; // no vertical shadow (StdFont.cpp:352)
    font.h_space = 0; // iHSpace = 0 without shadow (StdFont.cpp:327)
    // The same CP1252 range the shadowed GUI fonts rasterize
    // (clonk_fonts::build_font; StdFont.cpp:361-380).
    for byte in 0x20u16..=0xFF {
        let Some(ch) = cp1252_to_char(byte as u8) else {
            continue;
        };
        if face
            .load_char(ch as usize, LoadFlag::RENDER | LoadFlag::NO_HINTING)
            .is_err()
        {
            continue; // StdFont.cpp:203-208
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

        // width = max(advance, bearing+width) + shadowSize(0) (StdFont.cpp:218).
        let advance_px = (slot.advance().x >> 6) as i32;
        let bearing = slot.bitmap_left().max(0);
        let cell_w = advance_px.max(bearing + cov_w as i32).max(0) as usize;
        let at_x = bearing as usize;
        // at_y may be negative for glyphs taller than the ascent; the C++
        // atlas write clips them (SetPixDw bounds check).
        let at_y = ascent_px - i64::from(slot.bitmap_top());

        let mut pixels = vec![Color::transparent(); cell_w * cell_height];
        for y in 0..cov_h {
            let ty = at_y + y as i64;
            if ty < 0 || ty >= cell_height as i64 {
                continue;
            }
            for x in 0..cov_w {
                let tx = at_x + x;
                if tx >= cell_w {
                    continue;
                }
                let coverage = buffer[(y as i32 * pitch) as usize + x];
                // bAlpha = 255 - coverage (inverted); dwPixVal = white |
                // bAlpha<<24 → normal alpha = coverage (StdFont.cpp:228-256).
                pixels[ty as usize * cell_w + tx] = Color::new(255, 255, 255, coverage);
            }
        }
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

/// Windows-1252 specials in 0x80..=0x9F (StdFont.cpp:386-401); identical to
/// the table in `clonk_fonts` (private there).
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
        b => Some(b as char),
    }
}

/// Builds the two book fonts from a TTF (C4Startup.cpp:94-105: Caption 16 and
/// Main 14 from `Config.General.RXFontSize` = 14, C4Fonts.cpp:280-288).
pub fn build_book_font_set(ttf_bytes: &[u8]) -> Result<BookFontSet> {
    let library = Library::init().context("FreeType init failed")?;
    let face = library
        .new_memory_face(ttf_bytes.to_vec(), 0)
        .context("failed to load font face")?;
    Ok(BookFontSet {
        caption: build_book_font(&face, 16)?,
        text: build_book_font(&face, 14)?,
    })
}

// ---------------------------------------------------------------------------
// Assets and player data
// ---------------------------------------------------------------------------

/// Graphics.c4g assets the player selection dialog draws first-shown.
pub struct PlrSelAssets {
    /// `StartupPlrSelBG.png` — 800x600 fullscreen background
    /// (C4Startup.cpp:44, C4StartupPlrSelDlg.cpp:630-634).
    pub background: ImageData,
    /// `GUICheckbox.png` — 128x32, four 32x32 phases (C4Gui.cpp:1103-1104).
    pub checkbox: ImageData,
    /// `GUIButton.png` — 128x32 three-slice button bar (C4Gui.cpp:1089-1090).
    pub button: ImageData,
    /// `GUIButtonDown.png` — pressed bottom-button plank.
    pub button_down: ImageData,
    /// `GUIButtonHighlight.png` — additive focus/hover overlay.
    pub button_highlight: ImageData,
    /// `Player.png` — 48x48 ColorByOwner source used for the default player
    /// icon and portrait (C4GraphicsResource.cpp:265-268,
    /// C4StartupPlrSelDlg.cpp:168-170,230-233).
    pub player: ImageData,
}

/// One entry of the player file list (data from the `.c4p`'s Player.txt and
/// images; C4StartupPlrSelDlg::PlayerListItem::Load, cpp:215-244).
#[derive(Clone)]
pub struct PlrSelPlayer {
    /// `Core.PrefName`.
    pub name: String,
    /// Checkbox state: `SIsModule(Config.General.Participants, file)`
    /// (cpp:703).
    pub activated: bool,
    /// `BigIcon.png` from the player file, if present (cpp:227-229).
    pub big_icon: Option<ImageData>,
    /// `Portrait.png` from the player file (un-colorized base), if present
    /// (cpp:144-156).
    pub portrait: Option<ImageData>,
    /// `Core.PrefColorDw` — ColorByOwner tint for icon/portrait.
    pub color_dw: u32,
    /// `Core.Score`.
    pub score: i32,
    /// `Core.Rounds`.
    pub rounds: i32,
    /// `Core.RoundsWon`.
    pub rounds_won: i32,
    /// `Core.RoundsLost`.
    pub rounds_lost: i32,
    /// `Core.TotalPlayingTime` in seconds.
    pub total_playing_time: i32,
    /// `Core.Comment`.
    pub comment: String,
}

// ---------------------------------------------------------------------------
// CStdDDraw-faithful private draw helpers
// ---------------------------------------------------------------------------

/// `ClrPlayerItem` (C4StartupPlrSelDlg.cpp:36): opaque black, normal alpha.
const CLR_PLAYER_ITEM: [u8; 4] = [0, 0, 0, 255];
/// `C4GUI_ButtonFontClr` / `C4GUI_FullscreenCaptionFontClr` (C4Gui.h:56,164).
const CLR_BUTTON_FONT: [u8; 4] = [0xff, 0xff, 0x00, 0xff];
/// `C4GUI_ListBoxSelColor` (C4Gui.h:76): focused selection bar, engine
/// AARRGGBB with inverted alpha.
const CLR_LIST_BOX_SEL: u32 = 0xafaf0000;
/// `C4GUI_ListBoxInactSelColor` when a bottom button owns focus.
const CLR_LIST_BOX_INACTIVE_SEL: u32 = 0xaf7f7f7f;

/// `CStdDDraw::DrawBoxDw` (StdDDraw2.cpp:1401-1404) → `CStdGL::DrawQuadDw`
/// (StdGL.cpp:846-894): solid quad whose color carries engine INVERTED alpha
/// (0x00 = opaque); rgb goes through the DummyShader gamma lookup
/// (StdGL.cpp:1188-1199), then `glBlendFunc(GL_ONE_MINUS_SRC_ALPHA,
/// GL_SRC_ALPHA)` blends with opacity `(255-A)/255`. `x2`/`y2` are INCLUSIVE.
fn draw_box_dw(surface: &mut Surface, x1: i32, y1: i32, x2: i32, y2: i32, clr: u32, gamma: Option<&GammaRamp>) {
    let a_inv = ((clr >> 24) & 0xff) as f32 / 255.0;
    let opacity = 1.0 - a_inv;
    let enc = |c: u8| -> f32 {
        gamma.map_or(f32::from(c), |g| f32::from(g.encode_float(f32::from(c))))
    };
    let rgb = [enc((clr >> 16) as u8), enc((clr >> 8) as u8), enc(clr as u8)];
    for y in y1.max(0)..=y2.min(surface.height() as i32 - 1) {
        for x in x1.max(0)..=x2.min(surface.width() as i32 - 1) {
            let Some(dst) = surface.get_pixel(x as u32, y as u32) else {
                continue;
            };
            let blend = |src: f32, dst: u8| (src * opacity + f32::from(dst) * a_inv).round() as u8;
            let _ = surface.set_pixel(
                x as u32,
                y as u32,
                Color::new(blend(rgb[0], dst.r), blend(rgb[1], dst.g), blend(rgb[2], dst.b), 255),
            );
        }
    }
}

/// `C4Surface::ReadPNG` (C4Surface.cpp:972,982): every fully transparent
/// texel is forced to BLACK on texture upload (`if (pPix[3] == 0xff) *pPix =
/// 0xff000000`, engine inverted alpha). PNGs store transparent texels as
/// white, which would bleed too bright through GL_LINEAR edge interpolation.
fn engine_png_texture(image: &ImageData) -> ImageData {
    let mut px = image.pixels().to_vec();
    px.chunks_exact_mut(4)
        .filter(|texel| texel[3] == 0)
        .for_each(|texel| texel[..3].fill(0));
    ImageData::new(image.width(), image.height(), px)
}

/// `ClrByOwner` (C4Surface.cpp:236-286): HLS-based blue detection with
/// `HLSMAX = RGBMAX = 255`; hue window 145..=175 and saturation > 100.
/// Returns the gray replacement value (the blue channel) for overlay pixels.
fn clr_by_owner_gray(r: u8, g: u8, b: u8) -> Option<u8> {
    let (rv, gv, bv) = (i32::from(r), i32::from(g), i32::from(b));
    let c_max = rv.max(gv).max(bv);
    let c_min = rv.min(gv).min(bv);
    if c_max == c_min {
        return None; // achromatic: S = 0 (C4Surface.cpp:251-254)
    }
    let l = ((c_max + c_min) * 255 + 255) / (2 * 255);
    let s = if l <= 255 / 2 {
        ((c_max - c_min) * 255 + (c_max + c_min) / 2) / (c_max + c_min)
    } else {
        ((c_max - c_min) * 255 + (2 * 255 - c_max - c_min) / 2) / (2 * 255 - c_max - c_min)
    };
    let delta = |c: i32| ((c_max - c) * (255 / 6) + (c_max - c_min) / 2) / (c_max - c_min);
    let h = if rv == c_max {
        delta(bv) - delta(gv)
    } else if gv == c_max {
        255 / 3 + delta(rv) - delta(bv)
    } else {
        2 * 255 / 3 + delta(gv) - delta(rv)
    };
    let h = if h < 0 { h + 255 } else if h > 255 { h - 255 } else { h };
    ((145..=175).contains(&h) && s > 100).then_some(b)
}

/// `C4Surface::CreateColorByOwner` (C4Surface.cpp:288-318): moves
/// ColorByOwner pixels into a gray overlay (gray = blue channel, alpha
/// preserved) and punches them out of the base via `SetPixDw(0xffffffff)`
/// (C4Surface.cpp:311) — which squashes the fully transparent write to
/// BLACK (`if (dwClr >> 24 == 0xff) dwClr = 0xff000000`, C4Surface.cpp:733).
/// Fresh overlay texels keep the texture-clear transparent WHITE (memset
/// 0xff, C4Surface.cpp:1113) — the rgb matters for bilinear edge bleed.
fn split_color_by_owner(image: &ImageData) -> (ImageData, ImageData) {
    let px = image.pixels();
    let mut base = px.to_vec();
    let mut overlay: Vec<u8> = px
        .chunks_exact(4)
        .flat_map(|_| [255u8, 255, 255, 0])
        .collect();
    for (i, chunk) in px.chunks_exact(4).enumerate() {
        if let Some(gray) = clr_by_owner_gray(chunk[0], chunk[1], chunk[2]) {
            let o = i * 4;
            overlay[o..o + 4].copy_from_slice(&[gray, gray, gray, chunk[3]]);
            base[o..o + 4].copy_from_slice(&[0, 0, 0, 0]);
        }
    }
    (
        ImageData::new(image.width(), image.height(), base),
        ImageData::new(image.width(), image.height(), overlay),
    )
}

/// 1:1 ColorByOwner overlay blit: `PerformBlt` with `dwModClr = ClrByOwnerClr`
/// (StdDDraw2.cpp:770-780). The blit shader (StdGL.cpp:1068-1088) computes
/// `fragColor.rgb = tex.rgb * mod.rgb`, `fragColor.a = tex.a + mod.a` (mod
/// alpha byte is `clr >> 24`, raw — 0 for PrefColorDw), gamma-encodes the rgb
/// and blends `GL_SRC_ALPHA, GL_ONE_MINUS_SRC_ALPHA`. Exact (1:1) blits use
/// GL_NEAREST (StdGL.cpp:530-535), i.e. a direct texel copy.
fn draw_image_strip_modulated(
    surface: &mut Surface,
    dest_x: i32,
    dest_y: i32,
    image: &ImageData,
    mod_clr: u32,
    gamma: Option<&GammaRamp>,
) {
    let mod_rgb = [(mod_clr >> 16) as u8, (mod_clr >> 8) as u8, mod_clr as u8];
    let mod_a = (mod_clr >> 24) as u8;
    let px = image.pixels();
    for sy in 0..image.height() {
        let ty = dest_y + sy as i32;
        if ty < 0 || ty >= surface.height() as i32 {
            continue;
        }
        for sx in 0..image.width() {
            let tx = dest_x + sx as i32;
            if tx < 0 || tx >= surface.width() as i32 {
                continue;
            }
            let idx = ((sy * image.width() + sx) * 4) as usize;
            let Some(rgba) = px.get(idx..idx + 4) else {
                continue;
            };
            let a = (f32::from(rgba[3]) + f32::from(mod_a)).min(255.0);
            if a <= 0.0 {
                continue;
            }
            let af = a / 255.0;
            let Some(dst) = surface.get_pixel(tx as u32, ty as u32) else {
                continue;
            };
            let blend = |src: u8, m: u8, dst: u8| -> u8 {
                let modulated = f32::from(src) * f32::from(m) / 255.0;
                let enc = gamma.map_or_else(
                    || modulated.round().clamp(0.0, 255.0),
                    |g| f32::from(g.encode_float(modulated)),
                );
                (enc * af + f32::from(dst) * (1.0 - af)).round().clamp(0.0, 255.0) as u8
            };
            let _ = surface.set_pixel(
                tx as u32,
                ty as u32,
                Color::new(
                    blend(rgba[0], mod_rgb[0], dst.r),
                    blend(rgba[1], mod_rgb[1], dst.g),
                    blend(rgba[2], mod_rgb[2], dst.b),
                    255,
                ),
            );
        }
    }
}

/// The GL texture tile size for an image: next power of two of min(W, H)
/// capped at 4096 (C4Surface::CreateTextures, C4Surface.cpp:166-189);
/// mirrors the private `cpp_tex_size` in lib.rs.
fn cpp_tex_size(width: u32, height: u32) -> u32 {
    let need = width.min(height).max(1);
    let mut n = 1u32;
    while (1 << n) < need {
        n += 1;
    }
    (1u32 << n).min(4096)
}

/// GL_LINEAR sample of one texture tile with GL_CLAMP_TO_EDGE; mirrors the
/// private `bilinear_sample_tile` in lib.rs (C4Surface.cpp:1102-1103).
fn bilinear_sample_tile(
    image: &ImageData,
    tile_x: i32,
    tile_y: i32,
    tile_size: i32,
    u_rel: f32,
    v_rel: f32,
) -> [f32; 4] {
    let pixels = image.pixels();
    let texel = |x_rel: i32, y_rel: i32| -> [f32; 4] {
        let x = tile_x + x_rel.clamp(0, tile_size - 1);
        let y = tile_y + y_rel.clamp(0, tile_size - 1);
        if x < 0 || y < 0 || x >= image.width() as i32 || y >= image.height() as i32 {
            return [0.0; 4]; // tile padding
        }
        let idx = ((y as u32 * image.width() + x as u32) * 4) as usize;
        pixels
            .get(idx..idx + 4)
            .map(|p| [p[0] as f32, p[1] as f32, p[2] as f32, p[3] as f32])
            .unwrap_or([0.0; 4])
    };
    let (x0, y0) = (u_rel.floor() as i32, v_rel.floor() as i32);
    let (fx, fy) = (u_rel - x0 as f32, v_rel - y0 as f32);
    let (p00, p10) = (texel(x0, y0), texel(x0 + 1, y0));
    let (p01, p11) = (texel(x0, y0 + 1), texel(x0 + 1, y0 + 1));
    std::array::from_fn(|c| {
        let top = p00[c] * (1.0 - fx) + p10[c] * fx;
        let bottom = p01[c] * (1.0 - fx) + p11[c] * fx;
        top * (1.0 - fy) + bottom * fy
    })
}

/// Stretched ColorByOwner overlay blit: `crate::draw_image_bilinear`'s
/// sampling/blending (StdDDraw2.cpp:637-786) plus the blit shader's color
/// modulation (StdGL.cpp:1068-1088): rgb scaled by `mod.rgb` before the
/// gamma lookup, alpha = tex.a + mod.a (raw byte, 0 for PrefColorDw).
fn draw_image_bilinear_modulated(
    surface: &mut Surface,
    rect: &GuiRect,
    image: &ImageData,
    mod_clr: u32,
    gamma: Option<&GammaRamp>,
) {
    if rect.size.width <= 0.0 || rect.size.height <= 0.0 || image.width() == 0 || image.height() == 0
    {
        return;
    }
    let mod_rgbf = [
        ((mod_clr >> 16) & 0xff) as f32 / 255.0,
        ((mod_clr >> 8) & 0xff) as f32 / 255.0,
        (mod_clr & 0xff) as f32 / 255.0,
    ];
    let mod_a = (mod_clr >> 24) as f32;
    let (fw, fh) = (image.width() as f32, image.height() as f32);
    let (tx, ty) = (rect.origin.x, rect.origin.y);
    let scale_x = rect.size.width / fw;
    let scale_y = rect.size.height / fh;
    let ts = cpp_tex_size(image.width(), image.height()) as i32;
    let tiles_x = (image.width() as i32 - 1) / ts + 1;
    let tiles_y = (image.height() as i32 - 1) / ts + 1;

    for tile_iy in 0..tiles_y {
        for tile_ix in 0..tiles_x {
            let (blit_x, blit_y) = (tile_ix * ts, tile_iy * ts);
            let s_left = blit_x as f32;
            let s_top = blit_y as f32;
            let s_right = ((blit_x + ts) as f32).min(fw);
            let s_bottom = ((blit_y + ts) as f32).min(fh);
            let t_left = s_left * scale_x + tx;
            let t_top = s_top * scale_y + ty;
            let t_right = s_right * scale_x + tx;
            let t_bottom = s_bottom * scale_y + ty;
            let px0 = (t_left - 0.5).ceil() as i32;
            let py0 = (t_top - 0.5).ceil() as i32;
            for py in py0.max(0)..surface.height() as i32 {
                if (py as f32 + 0.5) >= t_bottom {
                    break;
                }
                for px in px0.max(0)..surface.width() as i32 {
                    if (px as f32 + 0.5) >= t_right {
                        break;
                    }
                    let u_rel = (px as f32 + 0.5 - tx) / scale_x - 0.5 - blit_x as f32;
                    let v_rel = (py as f32 + 0.5 - ty) / scale_y - 0.5 - blit_y as f32;
                    let s = bilinear_sample_tile(image, blit_x, blit_y, ts, u_rel, v_rel);
                    let a = (s[3] + mod_a).min(255.0);
                    if a <= 0.0 {
                        continue;
                    }
                    let af = (a / 255.0).clamp(0.0, 1.0);
                    let Some(dst) = surface.get_pixel(px as u32, py as u32) else {
                        continue;
                    };
                    let blend = |src: f32, m: f32, dst: u8| -> u8 {
                        let modulated = (src * m).clamp(0.0, 255.0);
                        let enc = gamma.map_or_else(
                            || modulated.round(),
                            |g| f32::from(g.encode_float(modulated)),
                        );
                        (enc * af + f32::from(dst) * (1.0 - af)).round().clamp(0.0, 255.0) as u8
                    };
                    let _ = surface.set_pixel(
                        px as u32,
                        py as u32,
                        Color::new(
                            blend(s[0], mod_rgbf[0], dst.r),
                            blend(s[1], mod_rgbf[1], dst.g),
                            blend(s[2], mod_rgbf[2], dst.b),
                            255,
                        ),
                    );
                }
            }
        }
    }
}

/// Copies a `w`x`h` subregion out of `image` (a facet's source rect). The
/// GUI facets used here are exactly one GL tile each (GUICheckbox 128x32 →
/// 32px tiles), so sampling the standalone copy is bit-identical to sampling
/// the original tile with GL_CLAMP_TO_EDGE.
fn extract_region(image: &ImageData, x: u32, y: u32, w: u32, h: u32) -> ImageData {
    let mut out = Vec::with_capacity((w * h * 4) as usize);
    for sy in y..y + h {
        let start = ((sy * image.width() + x) * 4) as usize;
        out.extend_from_slice(&image.pixels()[start..start + (w * 4) as usize]);
    }
    ImageData::new(w, h, out)
}

/// Aspect-preserving centering of a `src_w`x`src_h` facet inside `dest`,
/// mirroring `C4Facet::Draw(fAspect=true)` (C4Facet.cpp:106-117) integer
/// math.
fn aspect_fit(src_w: i32, src_h: i32, dest: IntRect) -> IntRect {
    if src_w <= 0 || src_h <= 0 {
        return dest;
    }
    let mut out = dest;
    if 100 * dest.w / src_w < 100 * dest.h / src_h {
        out.h = src_h * dest.w / src_w;
        out.y += (dest.h - out.h) / 2;
    } else if 100 * dest.h / src_h < 100 * dest.w / src_w {
        out.w = src_w * dest.h / src_h;
        out.x += (dest.w - out.w) / 2;
    }
    out
}

/// `TimeString` (C4StartupPlrSelDlg.cpp:40-46): `hh:mm:ss`, zero-padded.
fn time_string(seconds: i32) -> String {
    let hours = seconds / 3600;
    let minutes = seconds % 3600 / 60;
    format!("{:02}:{:02}:{:02}", hours, minutes, seconds % 60)
}

fn gui_rect(r: IntRect) -> GuiRect {
    GuiRect::new(r.x as f32, r.y as f32, r.w as f32, r.h as f32)
}

/// Focusable controls in the player-selection dialog's player mode.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PlrSelControl {
    PlayerList,
    Back,
    NewPlayer,
    Activate,
    Delete,
    Properties,
    Crew,
}

/// Requests produced by [`PlrSelController`]. File creation, deletion and
/// persistence remain application responsibilities; activation is mirrored
/// locally so the renderer can update immediately.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum PlrSelAction {
    SelectionChanged(Option<usize>),
    FocusChanged(PlrSelControl),
    Back,
    NewPlayer,
    ActivationChanged { index: usize, activated: bool },
    DeletePlayer(usize),
    PlayerProperties(usize),
    ShowCrew(usize),
}

/// Live input/selection state for `C4StartupPlrSelDlg` player mode.
pub struct PlrSelController {
    width: i32,
    height: i32,
    activations: Vec<bool>,
    selected: Option<usize>,
    focus: PlrSelControl,
    pointer_position: Option<GuiPoint>,
    hovered: Option<PlrSelControl>,
    pointer_pressed: Option<PlrSelControl>,
    key_pressed: Option<(PlrSelControl, KeyCode)>,
}

impl PlrSelController {
    pub fn new(player_count: usize) -> Self {
        Self {
            width: 1,
            height: 1,
            activations: vec![false; player_count],
            // UpdatePlayerList selects the first available entry
            // (C4StartupPlrSelDlg.cpp:724-729).
            selected: (player_count > 0).then_some(0),
            focus: PlrSelControl::PlayerList,
            pointer_position: None,
            hovered: None,
            pointer_pressed: None,
            key_pressed: None,
        }
    }

    pub fn resize(&mut self, width: i32, height: i32) {
        self.width = width.max(1);
        self.height = height.max(1);
        self.hovered = self.pointer_position.and_then(|point| self.hit_button(point));
    }

    pub fn set_player_count(&mut self, player_count: usize) {
        self.activations.resize(player_count, false);
        self.normalize_selection();
    }

    /// Replaces the activation flags after player-file discovery. Like C++,
    /// the first activated player is selected, falling back to the first
    /// deactivated player (C4StartupPlrSelDlg.cpp:695-729).
    pub fn set_player_activations(&mut self, activations: Vec<bool>) {
        self.activations = activations;
        self.selected = self
            .activations
            .iter()
            .position(|activated| *activated)
            .or_else(|| (!self.activations.is_empty()).then_some(0));
    }

    pub fn player_activations(&self) -> &[bool] {
        &self.activations
    }

    pub fn is_player_activated(&self, index: usize) -> Option<bool> {
        self.activations.get(index).copied()
    }

    pub const fn selected_index(&self) -> Option<usize> {
        self.selected
    }

    pub fn set_selected_index(&mut self, selected: Option<usize>) {
        self.selected = selected.filter(|index| *index < self.activations.len());
    }

    pub const fn focused_control(&self) -> PlrSelControl {
        self.focus
    }

    pub fn pointer_position(&self) -> Option<GuiPoint> {
        self.pointer_position
    }

    pub fn set_pointer_position(&mut self, position: Option<GuiPoint>) {
        self.pointer_position = position;
        self.hovered = position.and_then(|point| self.hit_button(point));
        if position.is_none() {
            self.pointer_pressed = None;
        }
    }

    pub fn pointer_left(&mut self) {
        self.set_pointer_position(None);
    }

    pub fn handle_pointer_move(&mut self, position: GuiPoint) -> Vec<PlrSelAction> {
        self.pointer_position = Some(position);
        self.hovered = self.hit_button(position);
        Vec::new()
    }

    pub fn handle_pointer_down(&mut self, position: GuiPoint) -> Vec<PlrSelAction> {
        self.pointer_position = Some(position);
        self.hovered = self.hit_button(position);
        self.pointer_pressed = self.hovered;

        if let Some(button) = self.hovered {
            return self.change_focus(button);
        }

        let layout = self.layout();
        if contains_plrsel(layout.list_client, position) {
            let mut actions = self.change_focus(PlrSelControl::PlayerList);
            let selected = self.list_item_at(position);
            actions.extend(self.change_selection(selected));
            return actions;
        }
        Vec::new()
    }

    pub fn handle_pointer_up(&mut self, position: GuiPoint) -> Vec<PlrSelAction> {
        self.pointer_position = Some(position);
        self.hovered = self.hit_button(position);
        if let Some(index) = self.checkbox_at(position) {
            self.pointer_pressed = None;
            return self.toggle_activation(index);
        }
        let Some(pressed) = self.pointer_pressed.take() else {
            return Vec::new();
        };
        if self.hit_button(position) != Some(pressed) {
            return Vec::new();
        }
        self.activate(pressed)
    }

    pub fn handle_pointer_double_click(&mut self, position: GuiPoint) -> Vec<PlrSelAction> {
        self.pointer_position = Some(position);
        self.hovered = self.hit_button(position);
        self.pointer_pressed = None;
        let layout = self.layout();
        if !contains_plrsel(layout.list_client, position) {
            return Vec::new();
        }
        let selected = self.list_item_at(position);
        let mut actions = self.change_focus(PlrSelControl::PlayerList);
        actions.extend(self.change_selection(selected));
        actions.extend(selected.map(PlrSelAction::PlayerProperties));
        actions
    }

    pub fn handle_key_down(&mut self, key: KeyCode) -> Vec<PlrSelAction> {
        match key {
            // StartupPlrSelBack binds Back, Left and Escape at override
            // priority (C4StartupPlrSelDlg.cpp:596-605).
            KeyCode::Escape | KeyCode::Left => vec![PlrSelAction::Back],
            KeyCode::Tab => self.advance_focus(),
            KeyCode::Up if self.focus == PlrSelControl::PlayerList => self.move_selection(-1),
            KeyCode::Down if self.focus == PlrSelControl::PlayerList => self.move_selection(1),
            KeyCode::Right if self.focus == PlrSelControl::PlayerList => self
                .selected
                .map(PlrSelAction::ShowCrew)
                .into_iter()
                .collect(),
            KeyCode::Space if self.focus == PlrSelControl::PlayerList => {
                self.toggle_selected_activation()
            }
            KeyCode::Enter if self.focus == PlrSelControl::PlayerList => self
                .selected
                .map(PlrSelAction::PlayerProperties)
                .into_iter()
                .collect(),
            KeyCode::Enter | KeyCode::Space => {
                self.key_pressed = Some((self.focus, key));
                Vec::new()
            }
            _ => Vec::new(),
        }
    }

    pub fn handle_key_up(&mut self, key: KeyCode) -> Vec<PlrSelAction> {
        let Some((pressed, pressed_key)) = self.key_pressed.take() else {
            return Vec::new();
        };
        if pressed_key != key || pressed != self.focus {
            return Vec::new();
        }
        self.activate(pressed)
    }

    fn layout(&self) -> PlrSelLayout {
        plrsel_layout(self.width, self.height)
    }

    fn hit_button(&self, point: GuiPoint) -> Option<PlrSelControl> {
        const CONTROLS: [PlrSelControl; 6] = [
            PlrSelControl::Back,
            PlrSelControl::NewPlayer,
            PlrSelControl::Activate,
            PlrSelControl::Delete,
            PlrSelControl::Properties,
            PlrSelControl::Crew,
        ];
        self.layout()
            .buttons
            .iter()
            .zip(CONTROLS)
            .find_map(|(rect, control)| contains_plrsel(*rect, point).then_some(control))
    }

    fn list_item_at(&self, point: GuiPoint) -> Option<usize> {
        let layout = self.layout();
        if point.x >= (layout.list_client.x + layout.item_width) as f32 {
            return None;
        }
        let offset = point.y as i32 - layout.list_client.y;
        if offset < 0 || offset % layout.item_pitch >= layout.item_height {
            return None;
        }
        let index = (offset / layout.item_pitch) as usize;
        (index < self.activations.len()).then_some(index)
    }

    fn checkbox_at(&self, point: GuiPoint) -> Option<usize> {
        let layout = self.layout();
        let index = self.list_item_at(point)?;
        (point.x < (layout.list_client.x + layout.item_height) as f32).then_some(index)
    }

    fn advance_focus(&mut self) -> Vec<PlrSelAction> {
        const ORDER: [PlrSelControl; 7] = [
            PlrSelControl::PlayerList,
            PlrSelControl::Back,
            PlrSelControl::NewPlayer,
            PlrSelControl::Activate,
            PlrSelControl::Delete,
            PlrSelControl::Properties,
            PlrSelControl::Crew,
        ];
        let index = ORDER.iter().position(|control| *control == self.focus).unwrap_or(0);
        self.change_focus(ORDER[(index + 1) % ORDER.len()])
    }

    fn change_focus(&mut self, focus: PlrSelControl) -> Vec<PlrSelAction> {
        if self.focus == focus {
            return Vec::new();
        }
        self.focus = focus;
        self.key_pressed = None;
        vec![PlrSelAction::FocusChanged(focus)]
    }

    fn change_selection(&mut self, selected: Option<usize>) -> Vec<PlrSelAction> {
        if self.selected == selected {
            return Vec::new();
        }
        self.selected = selected;
        vec![PlrSelAction::SelectionChanged(selected)]
    }

    fn move_selection(&mut self, delta: i32) -> Vec<PlrSelAction> {
        if self.activations.is_empty() {
            return Vec::new();
        }
        let selected = match (self.selected, delta) {
            (None, value) if value < 0 => Some(self.activations.len() - 1),
            (None, _) => Some(0),
            (Some(index), value) if value < 0 => Some(index.saturating_sub(1)),
            (Some(index), _) => Some((index + 1).min(self.activations.len() - 1)),
        };
        self.change_selection(selected)
    }

    fn toggle_selected_activation(&mut self) -> Vec<PlrSelAction> {
        let Some(index) = self.selected else {
            return Vec::new();
        };
        self.toggle_activation(index)
    }

    fn toggle_activation(&mut self, index: usize) -> Vec<PlrSelAction> {
        let Some(activated) = self.activations.get_mut(index) else {
            return Vec::new();
        };
        *activated = !*activated;
        vec![PlrSelAction::ActivationChanged {
            index,
            activated: *activated,
        }]
    }

    fn activate(&mut self, control: PlrSelControl) -> Vec<PlrSelAction> {
        match control {
            PlrSelControl::PlayerList => Vec::new(),
            PlrSelControl::Back => vec![PlrSelAction::Back],
            PlrSelControl::NewPlayer => vec![PlrSelAction::NewPlayer],
            PlrSelControl::Activate => self.toggle_selected_activation(),
            PlrSelControl::Delete => self
                .selected
                .map(PlrSelAction::DeletePlayer)
                .into_iter()
                .collect(),
            PlrSelControl::Properties => self
                .selected
                .map(PlrSelAction::PlayerProperties)
                .into_iter()
                .collect(),
            PlrSelControl::Crew => self
                .selected
                .map(PlrSelAction::ShowCrew)
                .into_iter()
                .collect(),
        }
    }

    fn normalize_selection(&mut self) {
        self.selected = self
            .selected
            .filter(|index| *index < self.activations.len())
            .or_else(|| (!self.activations.is_empty()).then_some(0));
    }

    fn is_highlighted(&self, control: PlrSelControl) -> bool {
        self.focus == control || self.hovered == Some(control)
    }

    fn is_pressed(&self, control: PlrSelControl) -> bool {
        self.pointer_pressed == Some(control)
            || self.key_pressed.is_some_and(|(pressed, _)| pressed == control)
    }
}

fn contains_plrsel(rect: IntRect, point: GuiPoint) -> bool {
    point.x >= rect.x as f32
        && point.y >= rect.y as f32
        && point.x < (rect.x + rect.w) as f32
        && point.y < (rect.y + rect.h) as f32
}

// ---------------------------------------------------------------------------
// Renderer
// ---------------------------------------------------------------------------

/// Renders C4StartupPlrSelDlg's first-shown state in the exact draw order of
/// Dialog::Draw → child add-order (spec §9; C4GuiDialogs.cpp:483-526,
/// C4GuiContainers.cpp:273-294).
pub struct PlrSelScreen;

impl PlrSelScreen {
    /// `fonts` are the shadowed GUI fonts (title + button captions); `book`
    /// the shadowless startup book fonts (list items + info text). The
    /// listbox has keyboard focus first-shown (cpp:593), so the selection
    /// bar uses the focused color.
    pub fn render(
        surface: &mut Surface,
        assets: &PlrSelAssets,
        fonts: &ClonkFontSet,
        book: &BookFontSet,
        players: &[PlrSelPlayer],
        selected: Option<usize>,
        gamma: Option<&GammaRamp>,
    ) {
        Self::render_impl(surface, assets, fonts, book, players, selected, None, gamma);
    }

    /// Draws the live controller state, including activation flags and all
    /// list/button interaction visuals.
    pub fn render_controller(
        surface: &mut Surface,
        assets: &PlrSelAssets,
        fonts: &ClonkFontSet,
        book: &BookFontSet,
        players: &[PlrSelPlayer],
        controller: &PlrSelController,
        gamma: Option<&GammaRamp>,
    ) {
        Self::render_impl(
            surface,
            assets,
            fonts,
            book,
            players,
            controller.selected,
            Some(controller),
            gamma,
        );
    }

    #[allow(clippy::too_many_arguments)]
    fn render_impl(
        surface: &mut Surface,
        assets: &PlrSelAssets,
        fonts: &ClonkFontSet,
        book: &BookFontSet,
        players: &[PlrSelPlayer],
        selected: Option<usize>,
        controller: Option<&PlrSelController>,
        gamma: Option<&GammaRamp>,
    ) {
        let (w, h) = (surface.width() as i32, surface.height() as i32);
        let layout = plrsel_layout(w, h);
        // Engine texture upload: fully transparent PNG texels turn black
        // (C4Surface::ReadPNG, C4Surface.cpp:972).
        let assets = &PlrSelAssets {
            background: engine_png_texture(&assets.background),
            checkbox: engine_png_texture(&assets.checkbox),
            button: engine_png_texture(&assets.button),
            button_down: engine_png_texture(&assets.button_down),
            button_highlight: engine_png_texture(&assets.button_highlight),
            player: engine_png_texture(&assets.player),
        };

        // 1. Background stretched over screen bounds +1px ring
        //    (DrawBackground, C4GuiDialogs.cpp:878-887).
        crate::draw_image_bilinear(
            surface,
            &GuiRect::new(-1.0, -1.0, (w + 2) as f32, (h + 2) as f32),
            &assets.background,
            gamma,
        );

        // 2. List box: selection bar behind the items (ListBox::DrawElement,
        //    C4GuiListBox.cpp:100-124), then the items in add-order.
        if let Some(sel) = selected.filter(|&sel| sel < players.len()) {
            let y = layout.list_client.y + layout.item_pitch * sel as i32;
            let color = if controller
                .is_none_or(|state| state.focus == PlrSelControl::PlayerList)
            {
                CLR_LIST_BOX_SEL
            } else {
                CLR_LIST_BOX_INACTIVE_SEL
            };
            draw_box_dw(
                surface,
                layout.list_client.x,
                y,
                layout.list_client.x + layout.item_width - 1,
                y + layout.item_height - 1,
                color,
                gamma,
            );
        }
        for (i, player) in players.iter().enumerate() {
            let activated = controller
                .and_then(|state| state.activations.get(i).copied())
                .unwrap_or(player.activated);
            Self::render_list_item(
                surface,
                assets,
                book,
                &layout,
                player,
                activated,
                i as i32,
                gamma,
            );
        }

        // 3. Info panel text for the selected player
        //    (PlayerListItem::SetSelectionInfo, cpp:293-302).
        if let Some(player) = selected.and_then(|sel| players.get(sel)) {
            Self::render_selection_info(surface, book, &layout, player, gamma);
        }

        // 4. Portrait picture, ColorByOwner-tinted (cpp:798-801).
        if let Some(player) = selected.and_then(|sel| players.get(sel)) {
            Self::render_portrait(surface, assets, &layout, player, gamma);
        }

        // 5.-10. Bottom buttons (Button::DrawElement, C4GuiButton.cpp:80-111).
        let activate_label = selected
            .and_then(|sel| {
                controller
                    .and_then(|state| state.activations.get(sel).copied())
                    .or_else(|| players.get(sel).map(|player| player.activated))
            })
            .map_or("Activate", |activated| {
                if activated {
                    "Deactivate"
                } else {
                    "Activate"
                }
            });
        let buttons = [
            (PlrSelControl::Back, "Back"),
            (PlrSelControl::NewPlayer, "New"),
            (PlrSelControl::Activate, activate_label),
            (PlrSelControl::Delete, "Delete"),
            (PlrSelControl::Properties, "Properties"),
            (PlrSelControl::Crew, "Crew"),
        ];
        for (index, (control, label)) in buttons.into_iter().enumerate() {
            Self::render_button(
                surface,
                assets,
                fonts,
                layout.buttons[index],
                label,
                controller.is_some_and(|state| state.is_highlighted(control)),
                controller.is_some_and(|state| state.is_pressed(control)),
                gamma,
            );
        }

        // 11. Fullscreen title, drawn last (SetTitle re-adds the label at the
        //     list end, C4GuiDialogs.cpp:835-847; cpp:693).
        fonts.title.draw_with_gamma(
            surface,
            layout.title_anchor.0,
            layout.title_anchor.1,
            "Player Selection",
            CLR_BUTTON_FONT,
            TextAlign::Center,
            true,
            gamma,
        );
    }

    #[allow(clippy::too_many_arguments)]
    fn render_button(
        surface: &mut Surface,
        assets: &PlrSelAssets,
        fonts: &ClonkFontSet,
        rect: IntRect,
        label: &str,
        highlighted: bool,
        pressed: bool,
        gamma: Option<&GammaRamp>,
    ) {
        let plank = if pressed {
            &assets.button_down
        } else {
            &assets.button
        };
        draw_bar(surface, &gui_rect(rect), plank, gamma);
        if highlighted {
            crate::draw_image_bilinear_additive(
                surface,
                &GuiRect::new(
                    (rect.x + 5) as f32,
                    (rect.y + 3) as f32,
                    (rect.w - 10) as f32,
                    (rect.h - 6) as f32,
                ),
                &assets.button_highlight,
                gamma,
            );
        }
        // Button::DrawElement shifts the caption one pixel while held
        // (C4GuiButton.cpp:81-110).
        let offset = i32::from(pressed);
        let font = fonts.button_font(rect.h);
        let (x0, y0) = (rect.x, rect.y);
        let (x1, y1) = (rect.x + rect.w - 1, rect.y + rect.h - 1);
        font.draw_with_gamma(
            surface,
            (x0 + x1) / 2 + offset,
            (y0 + y1 - font.line_height) / 2 + offset,
            label,
            CLR_BUTTON_FONT,
            TextAlign::Center,
            true,
            gamma,
        );
    }

    /// One PlayerListItem: checkbox, icon, name label (cpp:76-103).
    fn render_list_item(
        surface: &mut Surface,
        assets: &PlrSelAssets,
        book: &BookFontSet,
        layout: &PlrSelLayout,
        player: &PlrSelPlayer,
        activated: bool,
        index: i32,
        gamma: Option<&GammaRamp>,
    ) {
        let item = IntRect {
            x: layout.list_client.x,
            y: layout.list_client.y + layout.item_pitch * index,
            w: layout.item_width,
            h: layout.item_height,
        };
        // Checkbox: phase = fChecked + 2*!fEnabled of the 32x32 facet,
        // stretched to Hgt x Hgt (CheckBox::DrawElement,
        // C4GuiCheckBox.cpp:110-115).
        let phase = u32::from(activated);
        let cb = extract_region(&assets.checkbox, phase * 32, 0, 32, 32);
        crate::draw_image_bilinear(
            surface,
            &gui_rect(IntRect { x: item.x, y: item.y, w: item.h, h: item.h }),
            &cb,
            gamma,
        );
        // Icon at x = iHeight + IconLabelSpacing (cpp:88), aspect-centered
        // (Picture::DrawElement, C4GuiLabels.cpp:348-378).
        let icon_box = IntRect { x: item.x + item.h + 2, y: item.y, w: item.h, h: item.h };
        match &player.big_icon {
            Some(icon) => {
                let icon = engine_png_texture(icon); // C4Surface.cpp:972
                let dest = aspect_fit(icon.width() as i32, icon.height() as i32, icon_box);
                crate::draw_image_bilinear(surface, &gui_rect(dest), &icon, gamma);
            }
            None => {
                // Default: Player.png colorized by PrefColorDw (cpp:230-233).
                // The C++ pre-renders this through the software Blit8 path at
                // load; approximated with the GL-faithful blit here (the
                // reference player ships a BigIcon, so this path is not
                // exercised by the parity capture).
                let dest = aspect_fit(
                    assets.player.width() as i32,
                    assets.player.height() as i32,
                    icon_box,
                );
                Self::draw_color_by_owner(surface, &assets.player, dest, player.color_dw, gamma);
            }
        }
        // Name label at x0 = (iHeight + IconLabelSpacing)*2, y =
        // IconLabelSpacing, BookFont, ClrPlayerItem, no markup (cpp:89-90).
        book.text.draw_with_gamma(
            surface,
            item.x + (item.h + 2) * 2,
            item.y + 2,
            &player.name,
            CLR_PLAYER_ITEM,
            TextAlign::Left,
            false,
            gamma,
        );
    }

    /// Info panel lines (SetSelectionInfo, cpp:293-302): name in
    /// BookFontCapt, then IDS_DESC_PLAYER in BookFont. Every CR/LF segment
    /// becomes its own paragraph (C4LogBuf.cpp:177-218, empty segments
    /// dropped), and MultilineLabel adds lineHgt/3 before each paragraph
    /// after the first (C4GuiLabels.cpp:248-262). All lines fit the 356px
    /// wrap width, so BreakMessage wrapping is not needed here.
    fn render_selection_info(
        surface: &mut Surface,
        book: &BookFontSet,
        layout: &PlrSelLayout,
        player: &PlrSelPlayer,
        gamma: Option<&GammaRamp>,
    ) {
        let desc = [
            format!("Score: {}", player.score),
            format!(
                "Rounds: {} ({} won {} lost)",
                player.rounds, player.rounds_won, player.rounds_lost
            ),
            format!("Playing time: {}", time_string(player.total_playing_time)),
            format!("Comment: {}", player.comment),
        ];
        let lines = std::iter::once((&book.caption, player.name.as_str()))
            .chain(desc.iter().map(|line| (&book.text, line.as_str())));
        let mut y = layout.info_client.y;
        for (index, (font, line)) in lines.enumerate() {
            if index > 0 {
                y += font.line_height / 3; // paragraph indent
            }
            font.draw_with_gamma(
                surface,
                layout.info_client.x,
                y,
                line,
                CLR_PLAYER_ITEM,
                TextAlign::Left,
                false,
                gamma,
            );
            y += font.line_height;
        }
    }

    /// Portrait picture: the player's Portrait.png (or Player.png default)
    /// split via CreateColorByOwner and drawn base + tinted overlay
    /// (cpp:144-170,798-801; Picture::DrawElement with fAspect).
    fn render_portrait(
        surface: &mut Surface,
        assets: &PlrSelAssets,
        layout: &PlrSelLayout,
        player: &PlrSelPlayer,
        gamma: Option<&GammaRamp>,
    ) {
        // C4Surface.cpp:972: ReadPNG squashes transparent texels to black
        // before CreateColorByOwner splits the surface.
        let source = engine_png_texture(player.portrait.as_ref().unwrap_or(&assets.player));
        let dest = aspect_fit(source.width() as i32, source.height() as i32, layout.picture_area);
        Self::draw_color_by_owner(surface, &source, dest, player.color_dw, gamma);
    }

    /// `C4Facet::DrawClr` (C4Facet.cpp:142-150): base surface blit followed
    /// by the overlay modulated with the owner color (StdDDraw2.cpp:770-780).
    /// 1:1 blits are "exact" (GL_NEAREST, StdDDraw2.cpp:668); stretches use
    /// GL_LINEAR.
    fn draw_color_by_owner(
        surface: &mut Surface,
        source: &ImageData,
        dest: IntRect,
        color_dw: u32,
        gamma: Option<&GammaRamp>,
    ) {
        let (base, overlay) = split_color_by_owner(source);
        if dest.w == source.width() as i32 && dest.h == source.height() as i32 {
            crate::draw_image_strip(
                surface,
                dest.x,
                dest.y,
                &base,
                0,
                0,
                base.width(),
                base.height(),
                gamma,
            );
            draw_image_strip_modulated(surface, dest.x, dest.y, &overlay, color_dw, gamma);
        } else {
            crate::draw_image_bilinear(surface, &gui_rect(dest), &base, gamma);
            draw_image_bilinear_modulated(surface, &gui_rect(dest), &overlay, color_dw, gamma);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // Pixel-exact C4StartupPlrSelDlg geometry at 1280x720, derived from
    // C4StartupPlrSelDlg.cpp:550-562/636-657, C4GuiDialogs.cpp:819-820 and
    // C4StartupPlrSelDlg.h:221, verified against an F9 screenshot of the C++
    // engine at 1280x720 (see rust/target/parity-specs/plrsel.md).
    #[test]
    fn layout_matches_cpp_plrsel_dlg_at_1280x720() {
        let l = plrsel_layout(1280, 720);

        // Client: margins x=1280/50=25, y=720*2/75=19, top=720/7=102.
        assert_eq!((l.client.x, l.client.y, l.client.w, l.client.h), (25, 102, 1230, 599));

        // Player list box (123,155,379,373) client-rel → screen.
        assert_eq!(
            (l.plr_list.x, l.plr_list.y, l.plr_list.w, l.plr_list.h),
            (148, 257, 379, 373)
        );
        // List client: +3px margins.
        assert_eq!(
            (l.list_client.x, l.list_client.y, l.list_client.w, l.list_client.h),
            (151, 260, 373, 367)
        );
        // Items: 357 wide (373-16 scrollbar), 26 high, 27px pitch.
        assert_eq!((l.item_width, l.item_height, l.item_pitch), (357, 26, 27));

        // Info window (594,244,387,300) client-rel → screen; text client
        // shrunk by margins 10/8/5/8 and the 16px scrollbar.
        assert_eq!(
            (l.info_window.x, l.info_window.y, l.info_window.w, l.info_window.h),
            (619, 346, 387, 300)
        );
        assert_eq!(
            (l.info_client.x, l.info_client.y, l.info_client.w, l.info_client.h),
            (629, 354, 356, 284)
        );

        // Portrait picture area (781,94,200,150) client-rel → screen.
        assert_eq!(
            (l.picture_area.x, l.picture_area.y, l.picture_area.w, l.picture_area.h),
            (806, 196, 200, 150)
        );

        // Bottom buttons: 187x32 at x=34+205*i, y=665.
        for (i, b) in l.buttons.iter().enumerate() {
            assert_eq!((b.x, b.y, b.w, b.h), (34 + 205 * i as i32, 665, 187, 32), "button {i}");
        }

        // Title label anchor: centered at x=640, y=8.
        assert_eq!(l.title_anchor, (640, 8));
    }

    fn center(rect: IntRect) -> crate::GuiPoint {
        crate::GuiPoint::new((rect.x + rect.w / 2) as f32, (rect.y + rect.h / 2) as f32)
    }

    fn click(controller: &mut PlrSelController, rect: IntRect) -> Vec<PlrSelAction> {
        let point = center(rect);
        let _ = controller.handle_pointer_down(point);
        controller.handle_pointer_up(point)
    }

    // The six C4StartupPlrSelDlg callback buttons keep the list selection and
    // dispatch their operation on release; selection-dependent buttons do
    // nothing without a selected item (C4StartupPlrSelDlg.cpp:575-584,
    // 840-869,912-962; C4GuiButton.cpp:128-155).
    #[test]
    fn controller_routes_all_player_buttons_and_tracks_activation() {
        let layout = plrsel_layout(1280, 720);
        let mut controller = PlrSelController::new(2);
        controller.set_player_activations(vec![true, false]);
        controller.resize(1280, 720);

        let second_row = crate::GuiPoint::new(
            (layout.list_client.x + layout.item_height * 3) as f32,
            (layout.list_client.y + layout.item_pitch + 4) as f32,
        );
        assert_eq!(
            controller.handle_pointer_down(second_row),
            vec![PlrSelAction::SelectionChanged(Some(1))]
        );
        assert!(controller.handle_pointer_up(second_row).is_empty());

        assert_eq!(click(&mut controller, layout.buttons[0]), vec![PlrSelAction::Back]);
        assert_eq!(click(&mut controller, layout.buttons[1]), vec![PlrSelAction::NewPlayer]);
        assert_eq!(
            click(&mut controller, layout.buttons[2]),
            vec![PlrSelAction::ActivationChanged {
                index: 1,
                activated: true,
            }]
        );
        assert_eq!(controller.is_player_activated(1), Some(true));
        assert_eq!(
            click(&mut controller, layout.buttons[3]),
            vec![PlrSelAction::DeletePlayer(1)]
        );
        assert_eq!(
            click(&mut controller, layout.buttons[4]),
            vec![PlrSelAction::PlayerProperties(1)]
        );
        assert_eq!(
            click(&mut controller, layout.buttons[5]),
            vec![PlrSelAction::ShowCrew(1)]
        );
    }

    // ListBox uses half-open item rects and one-pixel row spacing. Clicking
    // spacing clears selection; Up/Down do not wrap. Space toggles the
    // selected ListItem and Enter invokes its double-click/property callback
    // (C4GuiListBox.cpp:142-173,218-254,386-394;
    // C4StartupPlrSelDlg.cpp:65-72,568-569,596-613).
    #[test]
    fn controller_matches_list_hit_testing_and_keys() {
        let layout = plrsel_layout(1280, 720);
        let mut controller = PlrSelController::new(2);
        controller.resize(1280, 720);
        assert_eq!(controller.selected_index(), Some(0));
        assert_eq!(controller.focused_control(), PlrSelControl::PlayerList);

        assert_eq!(
            controller.handle_key_down(crate::KeyCode::Down),
            vec![PlrSelAction::SelectionChanged(Some(1))]
        );
        assert!(controller.handle_key_down(crate::KeyCode::Down).is_empty());
        assert_eq!(
            controller.handle_key_down(crate::KeyCode::Space),
            vec![PlrSelAction::ActivationChanged {
                index: 1,
                activated: true,
            }]
        );
        assert_eq!(
            controller.handle_key_down(crate::KeyCode::Enter),
            vec![PlrSelAction::PlayerProperties(1)]
        );
        assert_eq!(
            controller.handle_key_down(crate::KeyCode::Right),
            vec![PlrSelAction::ShowCrew(1)]
        );

        let spacing = crate::GuiPoint::new(
            (layout.list_client.x + 2) as f32,
            (layout.list_client.y + layout.item_height) as f32,
        );
        assert_eq!(
            controller.handle_pointer_down(spacing),
            vec![PlrSelAction::SelectionChanged(None)]
        );
        assert_eq!(
            controller.handle_key_down(crate::KeyCode::Up),
            vec![PlrSelAction::SelectionChanged(Some(1))]
        );
        assert_eq!(
            controller.handle_key_down(crate::KeyCode::Escape),
            vec![PlrSelAction::Back]
        );
    }

    // Mouse routing follows the nested C4GUI controls: button down transfers
    // focus, checkbox up toggles that row, and ListBox left-double selects the
    // row before invoking OnSelDblClick -> Properties
    // (C4GuiContainers.cpp:695-710; C4GuiCheckBox.cpp:78-94;
    // C4GuiListBox.cpp:142-173; C4StartupPlrSelDlg.cpp:568-569).
    #[test]
    fn controller_matches_button_checkbox_and_double_click_routing() {
        let layout = plrsel_layout(1280, 720);
        let mut controller = PlrSelController::new(2);
        controller.resize(1280, 720);

        let back = center(layout.buttons[0]);
        assert_eq!(
            controller.handle_pointer_down(back),
            vec![PlrSelAction::FocusChanged(PlrSelControl::Back)]
        );
        assert_eq!(controller.focused_control(), PlrSelControl::Back);
        assert_eq!(controller.handle_pointer_up(back), vec![PlrSelAction::Back]);

        let second_checkbox = crate::GuiPoint::new(
            (layout.list_client.x + layout.item_height / 2) as f32,
            (layout.list_client.y + layout.item_pitch + layout.item_height / 2) as f32,
        );
        assert_eq!(
            controller.handle_pointer_down(second_checkbox),
            vec![
                PlrSelAction::FocusChanged(PlrSelControl::PlayerList),
                PlrSelAction::SelectionChanged(Some(1)),
            ]
        );
        assert_eq!(
            controller.handle_pointer_up(second_checkbox),
            vec![PlrSelAction::ActivationChanged {
                index: 1,
                activated: true,
            }]
        );

        let first_row_name = crate::GuiPoint::new(
            (layout.list_client.x + layout.item_height * 3) as f32,
            (layout.list_client.y + layout.item_height / 2) as f32,
        );
        assert_eq!(
            controller.handle_pointer_double_click(first_row_name),
            vec![
                PlrSelAction::SelectionChanged(Some(0)),
                PlrSelAction::PlayerProperties(0),
            ]
        );
    }

    // Live rendering must consume the controller that receives input: list
    // selection/focus, activation flags and C4GUI::Button interaction cannot
    // remain frozen at the first-shown frame (C4GuiListBox.cpp:100-124;
    // C4GuiButton.cpp:81-110; C4StartupPlrSelDlg.cpp:772-802,840-849).
    #[test]
    fn live_renderer_reflects_player_controller_state() {
        use lc_graphics::PixelFormat;
        let assets = PlrSelAssets {
            background: crate::test_support::load_graphics_png("StartupPlrSelBG.png"),
            checkbox: crate::test_support::load_graphics_png("GUICheckbox.png"),
            button: crate::test_support::load_graphics_png("GUIButton.png"),
            button_down: crate::test_support::load_graphics_png("GUIButtonDown.png"),
            button_highlight: crate::test_support::load_graphics_png("GUIButtonHighlight.png"),
            player: crate::test_support::load_graphics_png("Player.png"),
        };
        let fonts = crate::test_support::endeavour_font_set();
        let book = book_fonts();
        let gamma = crate::test_support::standard_gamma();
        let mut second = tyler();
        second.name = "Second".into();
        let players = [tyler(), second];
        let mut controller = PlrSelController::new(players.len());
        controller.resize(1280, 720);

        let render = |controller: &PlrSelController| {
            let mut surface = Surface::new(1280, 720, PixelFormat::Rgba8888);
            PlrSelScreen::render_controller(
                &mut surface,
                &assets,
                &fonts,
                &book,
                &players,
                controller,
                Some(gamma),
            );
            surface
        };

        let first = render(&controller);
        controller.handle_key_down(crate::KeyCode::Down);
        let selected_second = render(&controller);
        assert_ne!(first.pixels(), selected_second.pixels());

        controller.handle_key_down(crate::KeyCode::Space);
        let activated = render(&controller);
        assert_ne!(selected_second.pixels(), activated.pixels());

        controller.handle_key_down(crate::KeyCode::Tab);
        let button_focused = render(&controller);
        assert_ne!(activated.pixels(), button_focused.pixels());

        let layout = plrsel_layout(1280, 720);
        controller.handle_pointer_move(center(layout.buttons[1]));
        let hovered = render(&controller);
        assert_ne!(button_focused.pixels(), hovered.pixels());
        controller.handle_pointer_down(center(layout.buttons[1]));
        let pressed = render(&controller);
        assert_ne!(hovered.pixels(), pressed.pixels());
    }

    fn book_fonts() -> BookFontSet {
        let path = crate::test_support::repo_root().join("planet/System.c4g/Endeavour.ttf");
        let bytes = std::fs::read(&path).expect("read Endeavour.ttf");
        build_book_font_set(&bytes).expect("build book fonts")
    }

    // Shadowless book fonts per CStdFont::Init(fDoShadow=false)
    // (StdFont.cpp:327,352): iHSpace=0, iGfxLineHgt=iLineHgt, and glyph
    // cells are pure white with alpha = coverage and no +1 shadow column
    // (StdFont.cpp:184,218,224-256 with shadowSize=0).
    #[test]
    fn book_fonts_are_shadowless_endeavour_metrics() {
        let book = book_fonts();
        assert_eq!(book.caption.line_height, 25);
        assert_eq!(book.caption.cell_height, 25);
        assert_eq!(book.caption.h_space, 0);
        assert_eq!(book.text.line_height, 22);
        assert_eq!(book.text.cell_height, 22);
        assert_eq!(book.text.h_space, 0);

        // Every rendered pixel is pure white (no gray shadow blur); coverage
        // only lives in the alpha channel.
        let glyph = book.text.glyph('T').expect("glyph T");
        assert!(glyph
            .pixels
            .iter()
            .all(|px| (px.r, px.g, px.b) == (255, 255, 255) || px.a == 0));
        assert!(glyph.pixels.iter().any(|px| px.a == 255));

        // No shadow column: the shadowed GUI font's cell is exactly one
        // pixel wider for the same character (StdFont.cpp:218).
        let shadowed = crate::test_support::endeavour_font_set();
        assert_eq!(
            glyph.width + 1,
            shadowed.text.glyph('T').expect("shadowed T").width
        );
    }

    // DrawBoxDw → DrawQuadDw (StdGL.cpp:846-894): inverted-alpha color,
    // gamma-encoded rgb, blended src*(255-A)/255 + dst*A/255, inclusive x2/y2.
    #[test]
    fn draw_box_dw_blends_with_inverted_alpha_and_gamma() {
        use lc_graphics::PixelFormat;
        let gamma = crate::test_support::standard_gamma();
        let mut sfc = Surface::new(4, 4, PixelFormat::Rgba8888);
        for y in 0..4 {
            for x in 0..4 {
                let _ = sfc.set_pixel(x, y, Color::new(100, 100, 100, 255));
            }
        }
        draw_box_dw(&mut sfc, 1, 1, 2, 2, CLR_LIST_BOX_SEL, Some(gamma));
        // 0xafaf0000: opacity 80/255; r = round((175*80 + 100*175)/255) = 124,
        // g/b = round((encode(0)=1)*80 + 100*175)/255) = 69.
        let px = sfc.get_pixel(1, 1).unwrap();
        assert_eq!((px.r, px.g, px.b), (124, 69, 69));
        // x2/y2 inclusive: (2,2) painted, (3,3) untouched.
        assert_eq!(sfc.get_pixel(2, 2).unwrap().r, 124);
        assert_eq!(sfc.get_pixel(3, 3).unwrap().r, 100);
        assert_eq!(sfc.get_pixel(0, 0).unwrap().r, 100);
    }

    // ClrByOwner (C4Surface.cpp:236-286): saturated blue (hue 145..=175,
    // S>100) becomes a gray of the blue channel; everything else stays.
    #[test]
    fn clr_by_owner_detects_saturated_blue_only() {
        // Pure blue: H = 2*255/3 = 170, S = 255 → overlay, gray = 255.
        assert_eq!(clr_by_owner_gray(0, 0, 255), Some(255));
        // Tyler's shirt blue from Portrait.png is in-window too.
        assert_eq!(clr_by_owner_gray(40, 50, 180), Some(180));
        // Achromatic, red and green stay on the base surface.
        assert_eq!(clr_by_owner_gray(128, 128, 128), None);
        assert_eq!(clr_by_owner_gray(200, 0, 0), None);
        assert_eq!(clr_by_owner_gray(0, 200, 0), None);
        // Desaturated blue (S <= 100) stays.
        assert_eq!(clr_by_owner_gray(120, 120, 160), None);
    }

    // C4Surface::ReadPNG forces fully transparent texels to black
    // (C4Surface.cpp:972,982: `if (pPix[3] == 0xff) *pPix = 0xff000000`,
    // inverted alpha); partially transparent texels keep their rgb. The
    // PNG's transparent-white texels otherwise bleed too bright through
    // GL_LINEAR edge interpolation (proven on the checkbox top/left edge).
    #[test]
    fn engine_png_texture_squashes_fully_transparent_texels_to_black() {
        let image = ImageData::new(
            2,
            1,
            vec![255, 255, 255, 0, 200, 100, 50, 1],
        );
        let out = engine_png_texture(&image);
        assert_eq!(out.pixels(), &[0, 0, 0, 0, 200, 100, 50, 1]);
    }

    // CreateColorByOwner clears base pixels via SetPixDw(0xffffffff)
    // (C4Surface.cpp:311), and SetPixDw squashes fully transparent writes to
    // black (C4Surface.cpp:733) — so punched base texels are transparent
    // BLACK; untouched overlay texels keep the texture-clear transparent
    // WHITE (memset 0xff, C4Surface.cpp:1113).
    #[test]
    fn split_color_by_owner_punches_base_black_and_clears_overlay_white() {
        let image = ImageData::new(
            2,
            1,
            // One ClrByOwner blue pixel, one red pixel.
            vec![0, 0, 255, 255, 200, 0, 0, 255],
        );
        let (base, overlay) = split_color_by_owner(&image);
        assert_eq!(base.pixels(), &[0, 0, 0, 0, 200, 0, 0, 255]);
        assert_eq!(overlay.pixels(), &[255, 255, 255, 255, 255, 255, 255, 0]);
    }

    #[test]
    fn time_string_is_zero_padded_hms() {
        assert_eq!(time_string(0), "00:00:00");
        assert_eq!(time_string(3600 + 23 * 60 + 5), "01:23:05");
    }

    // C4Facet::Draw aspect math (C4Facet.cpp:106-117).
    #[test]
    fn aspect_fit_centers_square_source_in_wide_box() {
        // 150x150 source in (806,196,200,150) → (831,196,150,150) (spec §6).
        let out = aspect_fit(150, 150, IntRect { x: 806, y: 196, w: 200, h: 150 });
        assert_eq!((out.x, out.y, out.w, out.h), (831, 196, 150, 150));
        // 64x64 into 26x26: ratios equal → unchanged.
        let out = aspect_fit(64, 64, IntRect { x: 179, y: 260, w: 26, h: 26 });
        assert_eq!((out.x, out.y, out.w, out.h), (179, 260, 26, 26));
    }

    /// Loads the images extracted from `build/Tyler.c4p` (packed c4group;
    /// extracted next to the parity artifacts). Present on the reference
    /// machine only — elsewhere the renderer falls back to the default
    /// Player.png paths and the artifact is not reference-comparable.
    fn tyler_image(name: &str) -> Option<ImageData> {
        let path = std::path::Path::new("/tmp/menu-parity-plrsel/tyler").join(name);
        let rgba = image::open(path).ok()?.into_rgba8();
        let (w, h) = rgba.dimensions();
        Some(ImageData::new(w, h, rgba.into_raw()))
    }

    /// The single player file of the reference capture: build/Tyler.c4p
    /// (Player.txt: Name=Tyler, Comment=I'm new., ColorDw=15990784; stats 0).
    fn tyler() -> PlrSelPlayer {
        PlrSelPlayer {
            name: "Tyler".into(),
            activated: false,
            big_icon: tyler_image("BigIcon.png"),
            portrait: tyler_image("Portrait.png"),
            color_dw: 15_990_784, // 0x00F40000
            score: 0,
            rounds: 0,
            rounds_won: 0,
            rounds_lost: 0,
            total_playing_time: 0,
            comment: "I'm new.".into(),
        }
    }

    // Renders the first-shown dialog at 1280x720 and dumps the artifact the
    // parity harness diffs against the C++ F9 capture
    // (build/Screenshots/ref-plrsel.png). Deliberately no assertion against
    // the reference: CI machines do not ship it.
    //
    // Measured parity (2026-06-10, ref rows 1..719 vs render rows 0..718 —
    // SavePNG off-by-one): ZERO pixels with channel delta > 1 outside the
    // single mask x 637..=657, y 357..=384 (the engine's red-arrow mouse
    // cursor at screen center, captured into the reference); 95.42%
    // bit-identical, remaining 4.5% at ±1 spread uniformly over the
    // 800x600→1282x722 background stretch (GPU bilinear rounding residual).
    #[test]
    fn render_writes_reference_artifact() {
        use lc_graphics::PixelFormat;
        let assets = PlrSelAssets {
            background: crate::test_support::load_graphics_png("StartupPlrSelBG.png"),
            checkbox: crate::test_support::load_graphics_png("GUICheckbox.png"),
            button: crate::test_support::load_graphics_png("GUIButton.png"),
            button_down: crate::test_support::load_graphics_png("GUIButtonDown.png"),
            button_highlight: crate::test_support::load_graphics_png("GUIButtonHighlight.png"),
            player: crate::test_support::load_graphics_png("Player.png"),
        };
        let fonts = crate::test_support::endeavour_font_set();
        let book = book_fonts();
        let gamma = crate::test_support::standard_gamma();
        let players = [tyler()];

        let mut surface = Surface::new(1280, 720, PixelFormat::Rgba8888);
        PlrSelScreen::render(&mut surface, &assets, &fonts, &book, &players, Some(0), Some(gamma));
        // The app's final whole-surface gamma pass (identity except 0 → 1).
        gamma.apply_to_surface(&mut surface);

        std::fs::create_dir_all("/tmp/menu-parity-plrsel").expect("artifact dir");
        crate::test_support::write_ppm(&surface, "/tmp/menu-parity-plrsel/out.ppm");
    }
}
