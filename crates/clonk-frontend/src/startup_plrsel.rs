//! Pixel-parity renderer for one C++ startup dialog (see
//! `target/parity-specs/`). Implemented against the engine's F9
//! reference captures; owned by its implementation agent.
//!
//! This file renders `C4StartupPlrSelDlg` (player selection) in its
//! first-shown state, mirroring `src/C4StartupPlrSelDlg.cpp` and the C4GUI
//! widgets it instantiates. All geometry uses C++ integer math; all blits go
//! through the CStdDDraw-faithful helpers in this crate.

use crate::clonk_fonts::{expand_hotkey_markup, ClonkFontSet};
use crate::rename_edit::RenameEdit;
use crate::startup_main_menu::{draw_bar, IntRect, StartupTooltip};
use crate::{GuiPoint, ImageData, KeyCode};
use anyhow::{Context, Result};
use clonk_graphics::clonk_font::{line_height_for, ClonkFont, ClonkFontRole, GlyphCell, TextAlign};
use clonk_graphics::{
    BlitSampling, Color, GammaRamp, Rect as SurfaceRect, Surface, SurfaceDrawTarget,
};
use clonk_gui::Rect as GuiRect;
use clonk_resources::{PhysicalInfo, C4_MAX_PHYSICAL};
use freetype::face::LoadFlag;
use freetype::Library;
use std::{cell::RefCell, collections::HashMap};

const SCROLLBAR_WIDTH: i32 = 16;
const SCROLLBAR_PART: i32 = 16;

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
    /// ScrollWindow viewport. The 16px scrollbar remains reserved even while
    /// auto-hide makes the scrollbar itself invisible.
    pub list_viewport: IntRect,
    /// Fixed-width book scrollbar beside [`Self::list_viewport`].
    pub list_scrollbar: IntRect,
    /// Width of the list scroll window client = item width
    /// (list client minus the 16px scrollbar, C4Gui.h:111).
    pub item_width: i32,
    /// Height of one player list item: current BookFont line height + 2*2
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
    /// Crew mode's four visible bottom buttons. C++ retains the width that
    /// was calculated for the six-button player row, then centers those
    /// planks in a four-column grid (C4StartupPlrSelDlg.cpp:668-671).
    pub crew_buttons: [IntRect; 4],
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
/// This compatibility entry point uses the default Endeavour metrics. Runtime
/// callers with configured fonts should use [`plrsel_layout_with_fonts`].
///
/// Mirrors C4StartupPlrSelDlg.cpp:550-562 (ctor geometry),
/// C4StartupPlrSelDlg.cpp:636-657 (bottom buttons via GetGridCell,
/// C4Gui.cpp:1059-1080), C4GuiDialogs.cpp:819-822 (fullscreen margins) and
/// C4GuiContainers.cpp:301-307 / C4GuiListBox.h:120-123 (client rects).
pub fn plrsel_layout(w: i32, h: i32) -> PlrSelLayout {
    plrsel_layout_with_line_heights(w, h, TITLE_FONT_LINE_HEIGHT, BOOK_FONT_LINE_HEIGHT)
}

/// Computes player-selection geometry from the active GUI TitleFont and
/// startup BookFont. C++ creates the title and each list item from these live
/// font metrics (C4GuiDialogs.cpp:842-845; C4StartupPlrSelDlg.cpp:79-82).
pub fn plrsel_layout_with_fonts(
    w: i32,
    h: i32,
    fonts: &ClonkFontSet,
    book: &BookFontSet,
) -> PlrSelLayout {
    plrsel_layout_with_line_heights(w, h, fonts.title.line_height, book.text.line_height)
}

fn plrsel_layout_with_line_heights(
    w: i32,
    h: i32,
    title_font_line_height: i32,
    book_font_line_height: i32,
) -> PlrSelLayout {
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
    let list_viewport = IntRect {
        x: list_client.x,
        y: list_client.y,
        w: list_client.w - SCROLLBAR_WIDTH,
        h: list_client.h,
    };
    let list_scrollbar = IntRect {
        x: list_viewport.x + list_viewport.w,
        y: list_viewport.y,
        w: SCROLLBAR_WIDTH,
        h: list_viewport.h,
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
    // Crew mode reuses `bottom_button_w`; only the number of grid sectors
    // changes from six to four (UpdateBottomButtons, cpp:668-671).
    let crew_cell_w = client.w / 4;
    let mut crew_buttons = [IntRect::default(); 4];
    for (i, rect) in crew_buttons.iter_mut().enumerate() {
        *rect = IntRect {
            x: client.x + crew_cell_w * i as i32 + (crew_cell_w - bottom_button_w) / 2,
            y: client.y + bottom_buttons_y,
            w: bottom_button_w,
            h: button_height,
        };
    }

    PlrSelLayout {
        client,
        plr_list,
        list_client,
        list_viewport,
        list_scrollbar,
        item_width: list_viewport.w,
        item_height: book_font_line_height + 4,
        item_pitch: book_font_line_height + 4 + 1,
        info_window,
        info_client,
        picture_area: at_screen(picture_rel),
        buttons,
        crew_buttons,
        // Title label: x0 = clientWdt/2 (ACenter), y = C4UpperBoardHeight/2 -
        // TitleFont.lh/2 - GetMarginTop() (C4GuiDialogs.cpp:843-847).
        title_anchor: (
            client.x + client.w / 2,
            client.y + 25 - title_font_line_height / 2 - margin_top,
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
        caption: build_book_font(&face, 16)?.with_role(ClonkFontRole::BookCaption),
        text: build_book_font(&face, 14)?.with_role(ClonkFontRole::BookText),
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
    /// `StartupBookScroll.png` — 16px up/track/down/pin facets used by the
    /// auto-hiding player-list scrollbar.
    pub book_scroll: ImageData,
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

/// Promotion text shown in a crew member's detail pane. The app resolves
/// this from `C4ObjectInfoCore::GetNextRankInfo`, because the fallback rank
/// system is owned outside the startup renderer.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PlrSelCrewPromotion {
    pub rank_name: String,
    pub experience: i32,
}

/// Presentation data for one direct `*.c4i` child of the selected player.
/// Source filenames and writable group handles intentionally stay in the
/// app/data layer; this model contains only what the dialog draws.
#[derive(Clone)]
pub struct PlrSelCrew {
    pub name: String,
    /// `Core.Participation != 0`, mirrored by the controller for immediate
    /// checkbox feedback while the owning app persists `ObjectInfo.txt`.
    pub participating: bool,
    /// Fully resolved rank symbol. This may come from the crew group's own
    /// Rank image or a phase cut from the global rank sheet.
    pub rank_icon: Option<ImageData>,
    /// Custom/resolved crew portrait. Unlike players, C++ leaves the picture
    /// blank when no crew portrait can be resolved.
    pub portrait: Option<ImageData>,
    /// Parent player's `PrefColorDw`, applied to the crew portrait overlay.
    pub color_dw: u32,
    pub rank: i32,
    pub rank_name: String,
    pub type_name: String,
    pub experience: i32,
    pub rounds: i32,
    pub death_count: i32,
    pub total_playing_time: i32,
    /// Already localized/formatted equivalent of C++ `DateString(Birthday)`.
    pub birthday: String,
    pub next_rank: Option<PlrSelCrewPromotion>,
    pub physical: PhysicalInfo,
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
fn draw_box_dw(
    surface: &mut Surface,
    x1: i32,
    y1: i32,
    x2: i32,
    y2: i32,
    clr: u32,
    gamma: Option<&GammaRamp>,
) {
    if surface.is_gpu_scene_capture_active()
        || crate::active_advanced_renderer_config()
            .is_some_and(|config| config.blit_offset != 0 || config.no_box_fades)
    {
        if x2 < x1 || y2 < y1 {
            return;
        }
        crate::draw_color_rect(
            surface,
            SurfaceRect::new(
                x1,
                y1,
                x2.saturating_sub(x1).saturating_add(1) as u32,
                y2.saturating_sub(y1).saturating_add(1) as u32,
            ),
            Color::new(
                (clr >> 16) as u8,
                (clr >> 8) as u8,
                clr as u8,
                255 - (clr >> 24) as u8,
            ),
            gamma,
        );
        return;
    }
    let a_inv = ((clr >> 24) & 0xff) as f32 / 255.0;
    let opacity = 1.0 - a_inv;
    let enc =
        |c: u8| -> f32 { gamma.map_or(f32::from(c), |g| f32::from(g.encode_float(f32::from(c)))) };
    let rgb = [
        enc((clr >> 16) as u8),
        enc((clr >> 8) as u8),
        enc(clr as u8),
    ];
    for y in y1.max(0)..=y2.min(surface.height() as i32 - 1) {
        for x in x1.max(0)..=x2.min(surface.width() as i32 - 1) {
            let Some(dst) = surface.get_pixel(x as u32, y as u32) else {
                continue;
            };
            let blend = |src: f32, dst: u8| (src * opacity + f32::from(dst) * a_inv).round() as u8;
            let _ = surface.set_pixel(
                x as u32,
                y as u32,
                Color::new(
                    blend(rgb[0], dst.r),
                    blend(rgb[1], dst.g),
                    blend(rgb[2], dst.b),
                    blend_surface_alpha(opacity, dst.a),
                ),
            );
        }
    }
}

fn blend_surface_alpha(opacity: f32, destination: u8) -> u8 {
    (255.0 * opacity + f32::from(destination) * (1.0 - opacity))
        .round()
        .clamp(0.0, 255.0) as u8
}

/// `C4Surface::ReadPNG` (C4Surface.cpp:972,982): every fully transparent
/// texel is forced to BLACK on texture upload (`if (pPix[3] == 0xff) *pPix =
/// 0xff000000`, engine inverted alpha). PNGs store transparent texels as
/// white, which would bleed too bright through GL_LINEAR edge interpolation.
fn engine_png_texture(image: &ImageData) -> ImageData {
    thread_local! {
        static ENGINE_PNG_TEXTURES: RefCell<HashMap<clonk_graphics::GpuTextureId, ImageData>> =
            RefCell::new(HashMap::new());
    }
    ENGINE_PNG_TEXTURES.with(|textures| {
        if let Some(image) = textures.borrow().get(&image.gpu_texture_id()).cloned() {
            return image;
        }
        let needs_fixup = image
            .pixels()
            .chunks_exact(4)
            .any(|pixel| pixel[3] == 0 && pixel[..3] != [0, 0, 0]);
        let canonical = if needs_fixup {
            let mut pixels = image.pixels().to_vec();
            pixels
                .chunks_exact_mut(4)
                .filter(|texel| texel[3] == 0)
                .for_each(|texel| texel[..3].fill(0));
            ImageData::new(image.width(), image.height(), pixels)
        } else {
            image.clone()
        };
        textures
            .borrow_mut()
            .insert(image.gpu_texture_id(), canonical.clone());
        canonical
    })
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
    let h = if h < 0 {
        h + 255
    } else if h > 255 {
        h - 255
    } else {
        h
    };
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
    thread_local! {
        static OWNER_LAYERS: RefCell<
            HashMap<clonk_graphics::GpuTextureId, (ImageData, ImageData)>,
        > = RefCell::new(HashMap::new());
    }
    OWNER_LAYERS.with(|layers| {
        if let Some(layer) = layers.borrow().get(&image.gpu_texture_id()).cloned() {
            return layer;
        }
        let pixels = image.pixels();
        let mut base = pixels.to_vec();
        let mut overlay: Vec<u8> = pixels
            .chunks_exact(4)
            .flat_map(|_| [255u8, 255, 255, 0])
            .collect();
        for (index, pixel) in pixels.chunks_exact(4).enumerate() {
            if let Some(gray) = clr_by_owner_gray(pixel[0], pixel[1], pixel[2]) {
                let offset = index * 4;
                overlay[offset..offset + 4].copy_from_slice(&[gray, gray, gray, pixel[3]]);
                base[offset..offset + 4].copy_from_slice(&[0, 0, 0, 0]);
            }
        }
        let layer = (
            ImageData::new(image.width(), image.height(), base),
            ImageData::new(image.width(), image.height(), overlay),
        );
        layers
            .borrow_mut()
            .insert(image.gpu_texture_id(), layer.clone());
        layer
    })
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
    if crate::draw_image_source_modulated_with_active_renderer_config(
        surface,
        &GuiRect::new(
            dest_x as f32,
            dest_y as f32,
            image.width() as f32,
            image.height() as f32,
        ),
        image,
        (0.0, 0.0, image.width() as f32, image.height() as f32),
        BlitSampling::Nearest,
        mod_clr,
        gamma,
    ) {
        return;
    }
    if crate::capture_gpu_gui_image(
        surface,
        (
            dest_x as f32,
            dest_y as f32,
            image.width() as f32,
            image.height() as f32,
        ),
        image,
        crate::FloatSourceRect {
            x: 0.0,
            y: 0.0,
            width: image.width() as f32,
            height: image.height() as f32,
        },
        clonk_graphics::GpuSampler::Nearest,
        crate::BilinearBlend::AlphaOver,
        Some(if mod_clr == 0 { 0xff } else { mod_clr }),
        gamma,
    ) {
        return;
    }
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
            if surface.is_gpu_scene_capture_active() {
                // Fallback rasterization during capture must stay a
                // painter-ordered retained fragment instead of blending
                // against stale CPU backing.
                let _ = surface.blend_fragment_over(
                    tx as u32,
                    ty as u32,
                    [
                        f32::from(rgba[0]) * f32::from(mod_rgb[0]) / 255.0,
                        f32::from(rgba[1]) * f32::from(mod_rgb[1]) / 255.0,
                        f32::from(rgba[2]) * f32::from(mod_rgb[2]) / 255.0,
                        a,
                    ],
                    gamma,
                );
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
                (enc * af + f32::from(dst) * (1.0 - af))
                    .round()
                    .clamp(0.0, 255.0) as u8
            };
            let _ = surface.set_pixel(
                tx as u32,
                ty as u32,
                Color::new(
                    blend(rgba[0], mod_rgb[0], dst.r),
                    blend(rgba[1], mod_rgb[1], dst.g),
                    blend(rgba[2], mod_rgb[2], dst.b),
                    blend_surface_alpha(af, dst.a),
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
    if crate::draw_image_source_modulated_with_active_renderer_config(
        surface,
        rect,
        image,
        (0.0, 0.0, image.width() as f32, image.height() as f32),
        BlitSampling::Linear,
        mod_clr,
        gamma,
    ) {
        return;
    }
    if rect.size.width <= 0.0
        || rect.size.height <= 0.0
        || image.width() == 0
        || image.height() == 0
    {
        return;
    }
    if crate::capture_gpu_gui_image(
        surface,
        (
            rect.origin.x,
            rect.origin.y,
            rect.size.width,
            rect.size.height,
        ),
        image,
        crate::FloatSourceRect {
            x: 0.0,
            y: 0.0,
            width: image.width() as f32,
            height: image.height() as f32,
        },
        clonk_graphics::GpuSampler::Linear,
        crate::BilinearBlend::AlphaOver,
        Some(if mod_clr == 0 { 0xff } else { mod_clr }),
        gamma,
    ) {
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
                    if surface.is_gpu_scene_capture_active() {
                        // Fallback rasterization during capture must stay a
                        // painter-ordered retained fragment instead of
                        // blending against stale CPU backing.
                        let _ = surface.blend_fragment_over(
                            px as u32,
                            py as u32,
                            [
                                s[0] * mod_rgbf[0],
                                s[1] * mod_rgbf[1],
                                s[2] * mod_rgbf[2],
                                a,
                            ],
                            gamma,
                        );
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
                        (enc * af + f32::from(dst) * (1.0 - af))
                            .round()
                            .clamp(0.0, 255.0) as u8
                    };
                    let _ = surface.set_pixel(
                        px as u32,
                        py as u32,
                        Color::new(
                            blend(s[0], mod_rgbf[0], dst.r),
                            blend(s[1], mod_rgbf[1], dst.g),
                            blend(s[2], mod_rgbf[2], dst.b),
                            blend_surface_alpha(af, dst.a),
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
    type ExtractedRegionCache =
        HashMap<(clonk_graphics::GpuTextureId, u32, u32, u32, u32), ImageData>;
    thread_local! {
        static EXTRACTED_REGIONS: RefCell<ExtractedRegionCache> = RefCell::new(HashMap::new());
    }
    EXTRACTED_REGIONS.with(|regions| {
        let key = (image.gpu_texture_id(), x, y, w, h);
        if let Some(region) = regions.borrow().get(&key).cloned() {
            return region;
        }
        let mut pixels = Vec::with_capacity((w * h * 4) as usize);
        for source_y in y..y + h {
            let start = ((source_y * image.width() + x) * 4) as usize;
            pixels.extend_from_slice(&image.pixels()[start..start + (w * 4) as usize]);
        }
        let region = ImageData::new(w, h, pixels);
        regions.borrow_mut().insert(key, region.clone());
        region
    })
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

/// Builds `PlayerListItem::GetDelWarning`'s confirmation text
/// (C4StartupPlrSelDlg.cpp:304-311).
pub fn player_delete_warning(player: &PlrSelPlayer) -> String {
    let mut warning = format!("Do you really want to delete player {}?", player.name);
    if player.total_playing_time > 10 * 60 * 60 {
        warning.push_str(&format!(
            " - this player has a total playing time of {}!",
            time_string(player.total_playing_time)
        ));
    }
    warning
}

/// Builds `CrewListItem::GetDelWarning`'s confirmation text
/// (C4StartupPlrSelDlg.cpp:509-516).
pub fn crew_delete_warning(crew: &PlrSelCrew) -> String {
    let mut warning = format!(
        "Do you really want to delete {} {}?",
        crew.rank_name, crew.name
    );
    if crew.total_playing_time > 10 * 60 * 60 {
        warning.push_str(&format!(
            " - this Clonk has a total playing time of {}!",
            time_string(crew.total_playing_time)
        ));
    }
    warning
}

fn gui_rect(r: IntRect) -> GuiRect {
    GuiRect::new(r.x as f32, r.y as f32, r.w as f32, r.h as f32)
}

/// Active list projection of the shared player-selection dialog.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum PlrSelMode {
    Player,
    Crew {
        player_index: usize,
        player_name: String,
    },
}

impl PlrSelMode {
    pub fn title(&self) -> String {
        match self {
            Self::Player => "Player Selection".to_string(),
            // LanguageUS IDS_CTL_CREW is exactly "Crew:".
            Self::Crew { player_name, .. } => format!("Crew: {player_name}"),
        }
    }
}

/// Focusable controls in the mode-aware player-selection dialog.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PlrSelControl {
    PlayerList,
    Back,
    NewPlayer,
    Activate,
    Delete,
    Properties,
    Crew,
    Rename,
}

/// Classic GUI samples emitted by player-selection controls.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PlrSelSound {
    Command,
    ArrowHit,
    Click,
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
    LeaveCrew,
    CrewParticipationChanged { index: usize, participating: bool },
    DeleteCrew(usize),
    RenameCrew(usize),
    SetCrewDeathMessage(usize),
}

/// Commands contributed by one player row to the startup context menu.
/// The player index is captured when the menu opens, matching the C++
/// `PlayerListItem` handler's non-owning target.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PlrSelPlayerContextCommand {
    PlayerProperties(usize),
    DeletePlayer(usize),
}

impl From<PlrSelPlayerContextCommand> for PlrSelAction {
    fn from(command: PlrSelPlayerContextCommand) -> Self {
        match command {
            PlrSelPlayerContextCommand::PlayerProperties(player_index) => {
                Self::PlayerProperties(player_index)
            }
            PlrSelPlayerContextCommand::DeletePlayer(player_index) => {
                Self::DeletePlayer(player_index)
            }
        }
    }
}

/// C++ context-menu icon choice for a player-row entry. Both current
/// commands use `Ico_None`, but keeping it explicit makes the model directly
/// adaptable to the shared context-menu entry type.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PlrSelPlayerContextIcon {
    None,
}

/// One immutable player-row context-menu entry.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct PlrSelPlayerContextEntry {
    pub label: &'static str,
    pub tooltip: Option<&'static str>,
    pub icon: PlrSelPlayerContextIcon,
    pub hotkey: Option<char>,
    pub command: PlrSelPlayerContextCommand,
}

/// The exact two-entry context menu built by
/// `C4StartupPlrSelDlg::PlayerListItem::OnContext`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct PlrSelPlayerContextMenu {
    pub entries: [PlrSelPlayerContextEntry; 2],
    /// C++ does not preselect either entry when the popup opens.
    pub initial_selection: Option<usize>,
}

impl PlrSelPlayerContextMenu {
    pub const fn for_player(player_index: usize) -> Self {
        Self {
            entries: [
                PlrSelPlayerContextEntry {
                    label: "Properties",
                    tooltip: Some("Change player color and preferred controls."),
                    icon: PlrSelPlayerContextIcon::None,
                    hotkey: None,
                    command: PlrSelPlayerContextCommand::PlayerProperties(player_index),
                },
                PlrSelPlayerContextEntry {
                    label: "Delete",
                    tooltip: Some("Delete the selected player file."),
                    icon: PlrSelPlayerContextIcon::None,
                    hotkey: None,
                    command: PlrSelPlayerContextCommand::DeletePlayer(player_index),
                },
            ],
            initial_selection: None,
        }
    }
}

/// Commands contributed by a crew row to the startup context menu.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PlrSelCrewContextCommand {
    RenameCrew(usize),
    DeleteCrew(usize),
    SetCrewDeathMessage(usize),
}

impl From<PlrSelCrewContextCommand> for PlrSelAction {
    fn from(command: PlrSelCrewContextCommand) -> Self {
        match command {
            PlrSelCrewContextCommand::RenameCrew(index) => Self::RenameCrew(index),
            PlrSelCrewContextCommand::DeleteCrew(index) => Self::DeleteCrew(index),
            PlrSelCrewContextCommand::SetCrewDeathMessage(index) => {
                Self::SetCrewDeathMessage(index)
            }
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PlrSelCrewContextIcon {
    None,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct PlrSelCrewContextEntry {
    pub label: &'static str,
    pub tooltip: Option<&'static str>,
    pub icon: PlrSelCrewContextIcon,
    pub hotkey: Option<char>,
    pub command: PlrSelCrewContextCommand,
}

/// The exact three-entry context menu built by `CrewListItem::ContextMenu`
/// (C4StartupPlrSelDlg.cpp:375-385).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct PlrSelCrewContextMenu {
    pub entries: [PlrSelCrewContextEntry; 3],
    pub initial_selection: Option<usize>,
}

impl PlrSelCrewContextMenu {
    pub const fn for_crew(crew_index: usize) -> Self {
        Self {
            entries: [
                PlrSelCrewContextEntry {
                    label: "Rename",
                    tooltip: Some("Rename the selected crew member."),
                    icon: PlrSelCrewContextIcon::None,
                    hotkey: None,
                    command: PlrSelCrewContextCommand::RenameCrew(crew_index),
                },
                PlrSelCrewContextEntry {
                    label: "Delete",
                    tooltip: Some("Delete the selected crew member."),
                    icon: PlrSelCrewContextIcon::None,
                    hotkey: None,
                    command: PlrSelCrewContextCommand::DeleteCrew(crew_index),
                },
                PlrSelCrewContextEntry {
                    label: "Set death message",
                    tooltip: Some("Set the message that appear when this clonk dies."),
                    icon: PlrSelCrewContextIcon::None,
                    hotkey: None,
                    command: PlrSelCrewContextCommand::SetCrewDeathMessage(crew_index),
                },
            ],
            initial_selection: None,
        }
    }
}

/// Live input/selection state for both projections of
/// `C4StartupPlrSelDlg`.
pub struct PlrSelController {
    width: i32,
    height: i32,
    title_font_line_height: i32,
    book_font_line_height: i32,
    mode: PlrSelMode,
    player_activations: Vec<bool>,
    crew_participations: Vec<bool>,
    saved_player_selection: Option<usize>,
    selected: Option<usize>,
    focus: PlrSelControl,
    pointer_position: Option<GuiPoint>,
    hovered: Option<PlrSelControl>,
    pointer_pressed: Option<PlrSelControl>,
    key_pressed: Option<(PlrSelControl, KeyCode)>,
    list_scroll_y: i32,
    list_scroll_pin: i32,
    scrollbar_dragging: bool,
    scrollbar_arrow_captured: bool,
    scrollbar_arrow: i8,
    sound_events: Vec<PlrSelSound>,
}

impl PlrSelController {
    pub fn new(player_count: usize) -> Self {
        Self {
            width: 1,
            height: 1,
            title_font_line_height: TITLE_FONT_LINE_HEIGHT,
            book_font_line_height: BOOK_FONT_LINE_HEIGHT,
            mode: PlrSelMode::Player,
            player_activations: vec![false; player_count],
            crew_participations: Vec::new(),
            saved_player_selection: None,
            // UpdatePlayerList selects the first available entry
            // (C4StartupPlrSelDlg.cpp:724-729).
            selected: (player_count > 0).then_some(0),
            focus: PlrSelControl::PlayerList,
            pointer_position: None,
            hovered: None,
            pointer_pressed: None,
            key_pressed: None,
            list_scroll_y: 0,
            list_scroll_pin: 0,
            scrollbar_dragging: false,
            scrollbar_arrow_captured: false,
            scrollbar_arrow: 0,
            sound_events: Vec::new(),
        }
    }

    pub const fn mode(&self) -> &PlrSelMode {
        &self.mode
    }

    pub fn dialog_title(&self) -> String {
        self.mode.title()
    }

    pub const fn is_crew_mode(&self) -> bool {
        matches!(&self.mode, PlrSelMode::Crew { .. })
    }

    /// Switches the existing dialog into crew mode after the app has
    /// successfully opened a selected player and found at least one direct
    /// crew entry. The selected player index is retained for the return trip.
    pub fn enter_crew_mode(
        &mut self,
        player_index: usize,
        player_name: impl Into<String>,
        participations: Vec<bool>,
    ) -> bool {
        if self.is_crew_mode()
            || player_index >= self.player_activations.len()
            || participations.is_empty()
        {
            return false;
        }
        self.saved_player_selection = Some(player_index);
        self.mode = PlrSelMode::Crew {
            player_index,
            player_name: player_name.into(),
        };
        self.crew_participations = participations;
        self.selected = (!self.crew_participations.is_empty()).then_some(0);
        self.reset_list_scroll();
        self.focus = PlrSelControl::PlayerList;
        self.pointer_pressed = None;
        self.key_pressed = None;
        self.hovered = self
            .pointer_position
            .and_then(|point| self.hit_button(point));
        true
    }

    /// Restores player mode and the player that owned the displayed crew.
    /// Returns that still-valid player index for app-side stable-path restore.
    pub fn leave_crew_mode(&mut self) -> Option<usize> {
        if !self.is_crew_mode() {
            return None;
        }
        let restored = self
            .saved_player_selection
            .take()
            .filter(|index| *index < self.player_activations.len());
        self.mode = PlrSelMode::Player;
        self.crew_participations.clear();
        self.selected = restored.or_else(|| (!self.player_activations.is_empty()).then_some(0));
        self.reset_list_scroll();
        let layout = self.layout();
        self.ensure_selection_visible(&layout);
        self.focus = PlrSelControl::PlayerList;
        self.pointer_pressed = None;
        self.key_pressed = None;
        self.hovered = self
            .pointer_position
            .and_then(|point| self.hit_button(point));
        restored
    }

    pub fn resize(&mut self, width: i32, height: i32) {
        self.width = width.max(1);
        self.height = height.max(1);
        self.reflow_layout();
    }

    /// Resizes and configures geometry from the active player-selection
    /// fonts in one pass. Subsequent [`Self::resize`] calls retain the font
    /// metrics.
    pub fn resize_with_fonts(
        &mut self,
        width: i32,
        height: i32,
        fonts: &ClonkFontSet,
        book: &BookFontSet,
    ) {
        self.width = width.max(1);
        self.height = height.max(1);
        self.title_font_line_height = fonts.title.line_height;
        self.book_font_line_height = book.text.line_height;
        self.reflow_layout();
    }

    /// Reflows controller hit-testing and scrolling after a runtime font
    /// change without altering the current surface dimensions.
    pub fn set_layout_fonts(&mut self, fonts: &ClonkFontSet, book: &BookFontSet) {
        self.title_font_line_height = fonts.title.line_height;
        self.book_font_line_height = book.text.line_height;
        self.reflow_layout();
    }

    fn reflow_layout(&mut self) {
        self.hovered = self
            .pointer_position
            .and_then(|point| self.hit_button(point));
        self.clamp_list_scroll();
        let layout = self.layout();
        self.ensure_selection_visible(&layout);
    }

    pub fn set_player_count(&mut self, player_count: usize) {
        self.player_activations.resize(player_count, false);
        self.saved_player_selection = self
            .saved_player_selection
            .filter(|index| *index < player_count);
        if !self.is_crew_mode() {
            self.normalize_selection();
        }
        self.clamp_list_scroll();
    }

    /// Replaces the activation flags after player-file discovery. Like C++,
    /// the first activated player is selected, falling back to the first
    /// deactivated player (C4StartupPlrSelDlg.cpp:695-729).
    pub fn set_player_activations(&mut self, activations: Vec<bool>) {
        self.player_activations = activations;
        self.saved_player_selection = self
            .saved_player_selection
            .filter(|index| *index < self.player_activations.len());
        if !self.is_crew_mode() {
            self.selected = self
                .player_activations
                .iter()
                .position(|activated| *activated)
                .or_else(|| (!self.player_activations.is_empty()).then_some(0));
            self.reset_list_scroll();
            let layout = self.layout();
            self.ensure_selection_visible(&layout);
        }
    }

    pub fn player_activations(&self) -> &[bool] {
        &self.player_activations
    }

    pub fn is_player_activated(&self, index: usize) -> Option<bool> {
        self.player_activations.get(index).copied()
    }

    /// Updates one checkbox without rebuilding the list, changing selection,
    /// or resetting its ScrollWindow offset. This is the controller analogue
    /// of C++ `PlayerListItem::SetActivated`, whose checkbox setter does not
    /// invoke the activation callback.
    pub fn set_player_activation(&mut self, index: usize, activated: bool) -> bool {
        let Some(current) = self.player_activations.get_mut(index) else {
            return false;
        };
        *current = activated;
        true
    }

    pub fn set_crew_participations(&mut self, participations: Vec<bool>) -> bool {
        if !self.is_crew_mode() {
            return false;
        }
        self.crew_participations = participations;
        self.normalize_selection();
        self.reset_list_scroll();
        let layout = self.layout();
        self.ensure_selection_visible(&layout);
        true
    }

    pub fn crew_participations(&self) -> &[bool] {
        &self.crew_participations
    }

    pub fn is_crew_participating(&self, index: usize) -> Option<bool> {
        self.crew_participations.get(index).copied()
    }

    pub const fn selected_index(&self) -> Option<usize> {
        self.selected
    }

    pub fn set_selected_index(&mut self, selected: Option<usize>) {
        let selected = selected.filter(|index| *index < self.row_count());
        if self.selected != selected {
            self.selected = selected;
            let layout = self.layout();
            self.ensure_selection_visible(&layout);
        }
    }

    /// Returns the player row under a context-menu press. The whole item
    /// rectangle is a target because C++ child controls inherit the row's
    /// context handler; the one-pixel spacing and scrollbar are not.
    pub fn player_context_index_at(&self, position: GuiPoint) -> Option<usize> {
        if self.is_crew_mode() {
            return None;
        }
        let layout = self.layout();
        if !contains_plrsel(layout.list_client, position) {
            return None;
        }
        self.list_item_at(position)
    }

    /// Selects the context target without transferring keyboard focus or
    /// emitting normal list-selection actions. Popup opening owns any sound.
    pub fn select_player_for_context(&mut self, index: usize) -> bool {
        if self.is_crew_mode() || index >= self.player_activations.len() {
            return false;
        }
        self.selected = Some(index);
        true
    }

    pub fn crew_context_index_at(&self, position: GuiPoint) -> Option<usize> {
        if !self.is_crew_mode() {
            return None;
        }
        let layout = self.layout();
        if !contains_plrsel(layout.list_client, position) {
            return None;
        }
        self.list_item_at(position)
    }

    pub fn select_crew_for_context(&mut self, index: usize) -> bool {
        if !self.is_crew_mode() || index >= self.crew_participations.len() {
            return false;
        }
        self.selected = Some(index);
        true
    }

    /// Mode-aware row target used by the app's shared context-menu router.
    pub fn context_index_at(&self, position: GuiPoint) -> Option<usize> {
        if self.is_crew_mode() {
            self.crew_context_index_at(position)
        } else {
            self.player_context_index_at(position)
        }
    }

    /// Selects a mode-appropriate popup target without changing focus.
    pub fn select_for_context(&mut self, index: usize) -> bool {
        if self.is_crew_mode() {
            self.select_crew_for_context(index)
        } else {
            self.select_player_for_context(index)
        }
    }

    /// Returns the selected row and the screen-space point used by C++
    /// `Element::DoContext` for the Menu/Apps key. Keyboard context menus are
    /// anchored at the row's center and are available only while the list has
    /// draw focus; the last pointer position is deliberately irrelevant.
    pub fn keyboard_context_target(&self) -> Option<(usize, GuiPoint)> {
        if self.focus != PlrSelControl::PlayerList {
            return None;
        }
        let index = self.selected.filter(|index| *index < self.row_count())?;
        let layout = self.layout();
        Some((
            index,
            GuiPoint::new(
                (layout.list_viewport.x + layout.item_width / 2) as f32,
                (layout.list_viewport.y + index as i32 * layout.item_pitch - self.list_scroll_y
                    + layout.item_height / 2) as f32,
            ),
        ))
    }

    pub const fn focused_control(&self) -> PlrSelControl {
        self.focus
    }

    pub fn restore_focus(&mut self, focus: PlrSelControl) {
        self.focus = focus;
        self.key_pressed = None;
    }

    pub fn pointer_position(&self) -> Option<GuiPoint> {
        self.pointer_position
    }

    pub fn take_sound_events(&mut self) -> Vec<PlrSelSound> {
        std::mem::take(&mut self.sound_events)
    }

    /// Current vertical ScrollWindow displacement in logical pixels.
    pub const fn list_scroll_offset(&self) -> i32 {
        self.list_scroll_y
    }

    /// Maximum displacement for the current rows and viewport.
    pub fn list_max_scroll(&self) -> i32 {
        self.max_list_scroll(&self.layout())
    }

    /// Whether the next pointer release belongs to the book scrollbar rather
    /// than to a row under that release position.
    pub const fn scrollbar_pointer_captured(&self) -> bool {
        self.scrollbar_dragging || self.scrollbar_arrow_captured
    }

    /// Resolves the native tooltip target at `point`. `row_names` must follow
    /// the currently displayed player/crew order; row tooltips contain the
    /// live name, and the activation button formats that same name into its
    /// localized participate/deactivate description.
    pub fn tooltip_at<'a>(
        &self,
        point: GuiPoint,
        row_names: impl IntoIterator<Item = &'a str>,
    ) -> Option<StartupTooltip> {
        if let Some(control) = self.hit_button(point) {
            return match (self.is_crew_mode(), control) {
                (false, PlrSelControl::Back) => {
                    Some(StartupTooltip::resource("IDS_DLGTIP_BACKMAIN"))
                }
                (false, PlrSelControl::NewPlayer) => {
                    Some(StartupTooltip::resource("IDS_DLGTIP_NEWPLAYER"))
                }
                (false, PlrSelControl::Delete) => {
                    Some(StartupTooltip::resource("IDS_DLGTIP_PLAYERDELETE"))
                }
                (false, PlrSelControl::Properties) => {
                    Some(StartupTooltip::resource("IDS_DLGTIP_PLAYERPROPERTIES"))
                }
                (false, PlrSelControl::Crew) => {
                    Some(StartupTooltip::resource("IDS_DLGTIP_PLAYERCREW"))
                }
                (true, PlrSelControl::Back) => {
                    Some(StartupTooltip::resource("IDS_MSG_BACKTOPLAYERDLG"))
                }
                (true, PlrSelControl::Delete) => {
                    Some(StartupTooltip::resource("IDS_MSG_DELETECLONK_DESC"))
                }
                (true, PlrSelControl::Rename) => {
                    Some(StartupTooltip::resource("IDS_DESC_CREWRENAME"))
                }
                (_, PlrSelControl::Activate) => {
                    let selected = self.selected;
                    let name = selected
                        .and_then(|index| row_names.into_iter().nth(index))
                        .unwrap_or_default();
                    let active = selected.is_some_and(|index| {
                        if self.is_crew_mode() {
                            self.crew_participations
                                .get(index)
                                .copied()
                                .unwrap_or(false)
                        } else {
                            self.player_activations.get(index).copied().unwrap_or(false)
                        }
                    });
                    Some(StartupTooltip::formatted_resource(
                        if active {
                            "IDS_MSG_NOPARTICIPATE_DESC"
                        } else {
                            "IDS_MSG_PARTICIPATE_DESC"
                        },
                        [name],
                    ))
                }
                _ => None,
            };
        }

        let layout = self.layout();
        if !contains_plrsel(layout.plr_list, point) {
            return None;
        }
        if let Some(index) = self.list_item_at(point) {
            return row_names.into_iter().nth(index).map(StartupTooltip::text);
        }
        Some(StartupTooltip::resource("IDS_DLGTIP_PLAYERFILES"))
    }

    pub fn tooltip<'a>(
        &self,
        row_names: impl IntoIterator<Item = &'a str>,
    ) -> Option<StartupTooltip> {
        self.tooltip_at(self.pointer_position?, row_names)
    }

    pub fn set_pointer_position(&mut self, position: Option<GuiPoint>) {
        self.pointer_position = position;
        self.hovered = position.and_then(|point| self.hit_button(point));
        if position.is_none() {
            self.pointer_pressed = None;
            self.scrollbar_dragging = false;
            self.scrollbar_arrow_captured = false;
            self.scrollbar_arrow = 0;
        }
    }

    pub fn pointer_left(&mut self) {
        self.set_pointer_position(None);
    }

    pub fn cancel_interaction(&mut self) {
        self.set_pointer_position(None);
        self.key_pressed = None;
    }

    pub fn handle_pointer_move(&mut self, position: GuiPoint) -> Vec<PlrSelAction> {
        let button_was_down = self.pointer_button_is_down();
        self.pointer_position = Some(position);
        self.hovered = self.hit_button(position);
        let layout = self.layout();
        if self.scrollbar_dragging {
            self.set_scroll_from_pointer(position, &layout);
        } else if self.scrollbar_arrow_captured {
            self.scrollbar_arrow = self.scrollbar_arrow_at(position, &layout);
        }
        if button_was_down != self.pointer_button_is_down() {
            self.sound_events.push(PlrSelSound::ArrowHit);
        }
        Vec::new()
    }

    pub fn handle_pointer_down(&mut self, position: GuiPoint) -> Vec<PlrSelAction> {
        let button_was_down = self.pointer_button_is_down();
        self.pointer_position = Some(position);
        self.hovered = self.hit_button(position);
        self.pointer_pressed = self.hovered;

        if !button_was_down && self.pointer_button_is_down() {
            self.sound_events.push(PlrSelSound::ArrowHit);
        }

        let layout = self.layout();
        if self.max_list_scroll(&layout) > 0 && contains_plrsel(layout.list_scrollbar, position) {
            self.pointer_pressed = None;
            let actions = self.change_focus(PlrSelControl::PlayerList);
            self.begin_scrollbar_pointer(position, &layout);
            return actions;
        }

        if self.hovered.is_some() {
            // C4GUI::Button::IsFocusOnClick is false. Bottom-button clicks
            // retain the list (or previously tabbed) keyboard focus.
            return Vec::new();
        }

        if contains_plrsel(layout.list_viewport, position) {
            let mut actions = self.change_focus(PlrSelControl::PlayerList);
            let selected = self.list_item_at(position);
            actions.extend(self.change_selection(selected));
            return actions;
        }
        Vec::new()
    }

    pub fn handle_pointer_up(&mut self, position: GuiPoint) -> Vec<PlrSelAction> {
        let button_was_down = self.pointer_button_is_down();
        self.pointer_position = Some(position);
        self.hovered = self.hit_button(position);
        if self.scrollbar_dragging {
            let layout = self.layout();
            self.set_scroll_from_pointer(position, &layout);
            self.scrollbar_dragging = false;
            return Vec::new();
        }
        if self.scrollbar_arrow_captured {
            self.scrollbar_arrow_captured = false;
            self.scrollbar_arrow = 0;
            return Vec::new();
        }
        if let Some(pressed) = self.pointer_pressed.take() {
            if !button_was_down || self.hit_button(position) != Some(pressed) {
                if button_was_down {
                    self.sound_events.push(PlrSelSound::ArrowHit);
                }
                return Vec::new();
            }
            self.sound_events.push(PlrSelSound::Click);
            return self.activate(pressed);
        }
        if let Some(index) = self.checkbox_at(position) {
            self.sound_events.push(PlrSelSound::ArrowHit);
            return self.toggle_activation(index);
        }
        Vec::new()
    }

    pub fn handle_pointer_double_click(&mut self, position: GuiPoint) -> Vec<PlrSelAction> {
        self.pointer_position = Some(position);
        self.hovered = self.hit_button(position);
        self.pointer_pressed = None;
        let layout = self.layout();
        if !contains_plrsel(layout.list_viewport, position) {
            return Vec::new();
        }
        let selected = self.list_item_at(position);
        let mut actions = self.change_focus(PlrSelControl::PlayerList);
        actions.extend(self.change_selection(selected));
        let edit = self.selected_edit_action();
        if !edit.is_empty() {
            self.sound_events.push(PlrSelSound::Click);
        }
        actions.extend(edit);
        actions
    }

    /// Routes the native signed wheel delta over the ScrollWindow viewport.
    /// C4FullScreen supplies +60 for one notch up; ScrollWindow negates it.
    pub fn handle_wheel(&mut self, position: GuiPoint, delta: i32) -> Vec<PlrSelAction> {
        self.pointer_position = Some(position);
        self.hovered = self.hit_button(position);
        let layout = self.layout();
        if contains_plrsel(layout.list_viewport, position) {
            self.scroll_list_by(delta.saturating_neg(), &layout);
        }
        Vec::new()
    }

    /// Advances a held arrow by one fixed thumb pixel, matching
    /// `C4GUI::ScrollBar::DrawElement`.
    pub fn tick_scrollbar(&mut self) -> bool {
        if self.scrollbar_arrow == 0 {
            return false;
        }
        let layout = self.layout();
        let max_scroll = self.max_list_scroll(&layout);
        let max_pin = Self::scrollbar_range(&layout);
        if max_scroll == 0 {
            return false;
        }
        let previous_pin = self.list_scroll_pin;
        self.list_scroll_pin =
            (self.list_scroll_pin + i32::from(self.scrollbar_arrow)).clamp(0, max_pin);
        self.list_scroll_y = max_scroll * self.list_scroll_pin / max_pin;
        self.list_scroll_pin != previous_pin
    }

    pub fn handle_key_down(&mut self, key: KeyCode) -> Vec<PlrSelAction> {
        self.handle_key_down_with_tab_direction(key, false)
    }

    /// Mirrors `C4GUI::ListBox::CharIn`: one typed ASCII byte searches from
    /// the row after the current selection, wrapping once, and selects the
    /// next display name whose first byte matches case-insensitively. There
    /// is no prefix buffer or timeout.
    pub fn handle_character<'a>(
        &mut self,
        character: char,
        row_names: impl IntoIterator<Item = &'a str>,
    ) -> Vec<PlrSelAction> {
        if self.focus != PlrSelControl::PlayerList || !character.is_ascii() {
            return Vec::new();
        }
        let names = row_names
            .into_iter()
            .take(self.row_count())
            .collect::<Vec<_>>();
        if names.is_empty() {
            return Vec::new();
        }

        let selected = self.selected.filter(|index| *index < names.len());
        let start = selected.map_or(0, |index| (index + 1) % names.len());
        let candidates = if selected.is_some() {
            names.len().saturating_sub(1)
        } else {
            names.len()
        };
        let input = character as u8;
        for offset in 0..candidates {
            let index = (start + offset) % names.len();
            if names[index]
                .as_bytes()
                .first()
                .is_some_and(|first| first.eq_ignore_ascii_case(&input))
            {
                return self.change_selection(Some(index));
            }
        }
        Vec::new()
    }

    /// Dispatches a caption mnemonic through the currently visible bottom
    /// buttons. The bundled C++ LanguageUS captions contain no `&` markers
    /// here, so the current controller returns `None` for every
    /// alphanumeric key instead of inventing first-letter shortcuts.
    pub fn handle_hotkey(&mut self, character: char) -> Option<Vec<PlrSelAction>> {
        let character = character.to_ascii_uppercase();
        if !character.is_ascii_alphanumeric() {
            return None;
        }
        let activated = self.selected.and_then(|index| {
            if self.is_crew_mode() {
                self.crew_participations.get(index).copied()
            } else {
                self.player_activations.get(index).copied()
            }
        });
        let activate_label = if activated == Some(true) {
            "Deactivate"
        } else {
            "Activate"
        };
        let player_buttons = [
            (PlrSelControl::Back, "Back"),
            (PlrSelControl::NewPlayer, "New"),
            (PlrSelControl::Activate, activate_label),
            (PlrSelControl::Delete, "Delete"),
            (PlrSelControl::Properties, "Properties"),
            (PlrSelControl::Crew, "Crew"),
        ];
        let crew_buttons = [
            (PlrSelControl::Back, "Back"),
            (PlrSelControl::Activate, activate_label),
            (PlrSelControl::Delete, "Delete"),
            (PlrSelControl::Rename, "Rename"),
        ];
        let buttons: &[(PlrSelControl, &str)] = if self.is_crew_mode() {
            &crew_buttons
        } else {
            &player_buttons
        };
        buttons
            .iter()
            .find(|(_, label)| expand_hotkey_markup(label).1 == Some(character))
            .map(|(control, _)| self.activate(*control))
    }

    pub fn handle_key_down_with_tab_direction(
        &mut self,
        key: KeyCode,
        backwards: bool,
    ) -> Vec<PlrSelAction> {
        match key {
            // StartupPlrSelBack binds Back, Left and Escape at override
            // priority (C4StartupPlrSelDlg.cpp:596-605).
            KeyCode::Escape | KeyCode::Left => {
                if self.is_crew_mode() {
                    vec![PlrSelAction::LeaveCrew]
                } else {
                    vec![PlrSelAction::Back]
                }
            }
            KeyCode::Tab => self.move_focus(backwards),
            KeyCode::Up if self.focus == PlrSelControl::PlayerList => self.move_selection(-1),
            KeyCode::Down if self.focus == PlrSelControl::PlayerList => self.move_selection(1),
            KeyCode::Home if self.focus == PlrSelControl::PlayerList => {
                self.select_list_boundary(false)
            }
            KeyCode::End if self.focus == PlrSelControl::PlayerList => {
                self.select_list_boundary(true)
            }
            KeyCode::PageUp if self.focus == PlrSelControl::PlayerList => {
                self.page_list_selection(false)
            }
            KeyCode::PageDown if self.focus == PlrSelControl::PlayerList => {
                self.page_list_selection(true)
            }
            KeyCode::Right if !self.is_crew_mode() => self
                .selected
                .map(PlrSelAction::ShowCrew)
                .into_iter()
                .collect(),
            KeyCode::Space if self.focus == PlrSelControl::PlayerList => {
                let actions = self.toggle_selected_activation();
                if !actions.is_empty() {
                    self.sound_events.push(PlrSelSound::ArrowHit);
                }
                actions
            }
            KeyCode::Enter if self.focus == PlrSelControl::PlayerList => {
                let actions = self.selected_edit_action();
                if !actions.is_empty() {
                    self.sound_events.push(PlrSelSound::Click);
                }
                actions
            }
            KeyCode::Enter | KeyCode::Space => {
                if self.key_pressed.is_none() {
                    self.key_pressed = Some((self.focus, key));
                    self.sound_events.push(PlrSelSound::ArrowHit);
                }
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
        self.sound_events.push(PlrSelSound::Click);
        self.activate(pressed)
    }

    /// Mode-aware equivalent of the dialog's F2/edit shortcut: player mode
    /// opens properties, crew mode begins renaming the selected member.
    pub fn handle_edit_shortcut(&self) -> Vec<PlrSelAction> {
        self.selected_edit_action()
    }

    /// Current controller geometry, including configured font metrics.
    pub fn layout(&self) -> PlrSelLayout {
        plrsel_layout_with_line_heights(
            self.width,
            self.height,
            self.title_font_line_height,
            self.book_font_line_height,
        )
    }

    pub fn row_count(&self) -> usize {
        if self.is_crew_mode() {
            self.crew_participations.len()
        } else {
            self.player_activations.len()
        }
    }

    fn hit_button(&self, point: GuiPoint) -> Option<PlrSelControl> {
        const PLAYER_CONTROLS: [PlrSelControl; 6] = [
            PlrSelControl::Back,
            PlrSelControl::NewPlayer,
            PlrSelControl::Activate,
            PlrSelControl::Delete,
            PlrSelControl::Properties,
            PlrSelControl::Crew,
        ];
        const CREW_CONTROLS: [PlrSelControl; 4] = [
            PlrSelControl::Back,
            PlrSelControl::Activate,
            PlrSelControl::Delete,
            PlrSelControl::Rename,
        ];
        let layout = self.layout();
        if self.is_crew_mode() {
            layout
                .crew_buttons
                .iter()
                .zip(CREW_CONTROLS)
                .find_map(|(rect, control)| contains_plrsel(*rect, point).then_some(control))
        } else {
            layout
                .buttons
                .iter()
                .zip(PLAYER_CONTROLS)
                .find_map(|(rect, control)| contains_plrsel(*rect, point).then_some(control))
        }
    }

    fn list_item_at(&self, point: GuiPoint) -> Option<usize> {
        let layout = self.layout();
        if !contains_plrsel(layout.list_viewport, point) {
            return None;
        }
        let offset = point.y as i32 - layout.list_viewport.y + self.list_scroll_y;
        if offset % layout.item_pitch >= layout.item_height {
            return None;
        }
        let index = (offset / layout.item_pitch) as usize;
        (index < self.row_count()).then_some(index)
    }

    fn checkbox_at(&self, point: GuiPoint) -> Option<usize> {
        let layout = self.layout();
        let index = self.list_item_at(point)?;
        (point.x < (layout.list_client.x + layout.item_height) as f32).then_some(index)
    }

    pub fn handle_gamepad_horizontal(&mut self, backwards: bool) -> Vec<PlrSelAction> {
        self.move_focus(backwards)
    }

    fn move_focus(&mut self, backwards: bool) -> Vec<PlrSelAction> {
        const PLAYER_ORDER: [PlrSelControl; 7] = [
            PlrSelControl::PlayerList,
            PlrSelControl::Back,
            PlrSelControl::NewPlayer,
            PlrSelControl::Activate,
            PlrSelControl::Delete,
            PlrSelControl::Properties,
            PlrSelControl::Crew,
        ];
        const CREW_ORDER: [PlrSelControl; 5] = [
            PlrSelControl::PlayerList,
            PlrSelControl::Back,
            PlrSelControl::Activate,
            PlrSelControl::Delete,
            PlrSelControl::Rename,
        ];
        let order: &[PlrSelControl] = if self.is_crew_mode() {
            &CREW_ORDER
        } else {
            &PLAYER_ORDER
        };
        let index = order
            .iter()
            .position(|control| *control == self.focus)
            .unwrap_or(0);
        let next = if backwards {
            (index + order.len() - 1) % order.len()
        } else {
            (index + 1) % order.len()
        };
        self.change_focus(order[next])
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
        let layout = self.layout();
        self.ensure_selection_visible(&layout);
        if selected.is_some() {
            self.sound_events.push(PlrSelSound::Command);
        }
        vec![PlrSelAction::SelectionChanged(selected)]
    }

    fn move_selection(&mut self, delta: i32) -> Vec<PlrSelAction> {
        let row_count = self.row_count();
        if row_count == 0 {
            return Vec::new();
        }
        let selected = match (self.selected, delta) {
            (None, value) if value < 0 => Some(row_count - 1),
            (None, _) => Some(0),
            (Some(index), value) if value < 0 => Some(index.saturating_sub(1)),
            (Some(index), _) => Some((index + 1).min(row_count - 1)),
        };
        self.change_selection(selected)
    }

    fn select_list_boundary(&mut self, last: bool) -> Vec<PlrSelAction> {
        let selected = if last {
            self.row_count().checked_sub(1)
        } else {
            (self.row_count() > 0).then_some(0)
        };
        self.change_selection(selected)
    }

    fn list_item_fully_visible(&self, index: usize, layout: &PlrSelLayout) -> bool {
        let top = i32::try_from(index)
            .unwrap_or(i32::MAX)
            .saturating_mul(layout.item_pitch);
        self.list_scroll_y <= top
            && self.list_scroll_y.saturating_add(layout.list_viewport.h)
                >= top.saturating_add(layout.item_height)
    }

    /// Exact adjacent-first paging from `C4GUI::ListBox::KeyPageDown/KeyPageUp`:
    /// walk through fully visible rows, or scroll one viewport and scan inward
    /// from the global list boundary.
    fn page_list_selection(&mut self, forward: bool) -> Vec<PlrSelAction> {
        let row_count = self.row_count();
        if row_count == 0 {
            return Vec::new();
        }
        let layout = self.layout();
        let mut target = self
            .selected
            .filter(|index| *index < row_count)
            .unwrap_or(if forward { 0 } else { row_count - 1 });

        if forward {
            if target + 1 < row_count {
                target += 1;
                if self.list_item_fully_visible(target, &layout) {
                    while target + 1 < row_count
                        && self.list_item_fully_visible(target + 1, &layout)
                    {
                        target += 1;
                    }
                } else {
                    self.scroll_list_by(layout.list_viewport.h, &layout);
                    target = row_count - 1;
                    while target > 0 && !self.list_item_fully_visible(target, &layout) {
                        target -= 1;
                    }
                }
            }
        } else if target > 0 {
            target -= 1;
            if self.list_item_fully_visible(target, &layout) {
                while target > 0 && self.list_item_fully_visible(target - 1, &layout) {
                    target -= 1;
                }
            } else {
                self.scroll_list_by(layout.list_viewport.h.saturating_neg(), &layout);
                target = 0;
                while target + 1 < row_count && !self.list_item_fully_visible(target, &layout) {
                    target += 1;
                }
            }
        }

        self.change_selection(Some(target))
    }

    fn toggle_selected_activation(&mut self) -> Vec<PlrSelAction> {
        let Some(index) = self.selected else {
            return Vec::new();
        };
        self.toggle_activation(index)
    }

    fn toggle_activation(&mut self, index: usize) -> Vec<PlrSelAction> {
        if self.is_crew_mode() {
            let Some(participating) = self.crew_participations.get_mut(index) else {
                return Vec::new();
            };
            *participating = !*participating;
            vec![PlrSelAction::CrewParticipationChanged {
                index,
                participating: *participating,
            }]
        } else {
            let Some(activated) = self.player_activations.get_mut(index) else {
                return Vec::new();
            };
            *activated = !*activated;
            vec![PlrSelAction::ActivationChanged {
                index,
                activated: *activated,
            }]
        }
    }

    fn activate(&mut self, control: PlrSelControl) -> Vec<PlrSelAction> {
        match control {
            PlrSelControl::PlayerList => Vec::new(),
            PlrSelControl::Back if self.is_crew_mode() => vec![PlrSelAction::LeaveCrew],
            PlrSelControl::Back => vec![PlrSelAction::Back],
            PlrSelControl::NewPlayer if !self.is_crew_mode() => vec![PlrSelAction::NewPlayer],
            PlrSelControl::NewPlayer => Vec::new(),
            PlrSelControl::Activate => self.toggle_selected_activation(),
            PlrSelControl::Delete if self.is_crew_mode() => self
                .selected
                .map(PlrSelAction::DeleteCrew)
                .into_iter()
                .collect(),
            PlrSelControl::Delete => self
                .selected
                .map(PlrSelAction::DeletePlayer)
                .into_iter()
                .collect(),
            PlrSelControl::Properties if !self.is_crew_mode() => self
                .selected
                .map(PlrSelAction::PlayerProperties)
                .into_iter()
                .collect(),
            PlrSelControl::Properties => Vec::new(),
            PlrSelControl::Crew if !self.is_crew_mode() => self
                .selected
                .map(PlrSelAction::ShowCrew)
                .into_iter()
                .collect(),
            PlrSelControl::Crew => Vec::new(),
            PlrSelControl::Rename if self.is_crew_mode() => self
                .selected
                .map(PlrSelAction::RenameCrew)
                .into_iter()
                .collect(),
            PlrSelControl::Rename => Vec::new(),
        }
    }

    fn selected_edit_action(&self) -> Vec<PlrSelAction> {
        self.selected
            .map(|index| {
                if self.is_crew_mode() {
                    PlrSelAction::RenameCrew(index)
                } else {
                    PlrSelAction::PlayerProperties(index)
                }
            })
            .into_iter()
            .collect()
    }

    fn normalize_selection(&mut self) {
        let row_count = self.row_count();
        self.selected = self
            .selected
            .filter(|index| *index < row_count)
            .or_else(|| (row_count > 0).then_some(0));
        self.clamp_list_scroll();
        let layout = self.layout();
        self.ensure_selection_visible(&layout);
    }

    fn list_content_height(&self, layout: &PlrSelLayout) -> i32 {
        if self.row_count() == 0 {
            return 0;
        }
        i32::try_from(self.row_count())
            .unwrap_or(i32::MAX)
            .saturating_mul(layout.item_pitch)
            .saturating_sub(1)
    }

    fn max_list_scroll(&self, layout: &PlrSelLayout) -> i32 {
        self.list_content_height(layout)
            .saturating_sub(layout.list_viewport.h)
            .max(0)
    }

    fn scrollbar_has_pin(layout: &PlrSelLayout) -> bool {
        layout.list_scrollbar.h > 3 * SCROLLBAR_PART
    }

    fn scrollbar_range(layout: &PlrSelLayout) -> i32 {
        if Self::scrollbar_has_pin(layout) {
            layout.list_scrollbar.h - 3 * SCROLLBAR_PART
        } else {
            // C4GUI::ScrollBar uses a synthetic range when the viewport is
            // too short for its fixed pin. The arrows remain usable.
            100
        }
    }

    fn reset_list_scroll(&mut self) {
        self.list_scroll_y = 0;
        self.list_scroll_pin = 0;
        self.scrollbar_dragging = false;
        self.scrollbar_arrow_captured = false;
        self.scrollbar_arrow = 0;
    }

    fn clamp_list_scroll(&mut self) {
        let layout = self.layout();
        let max_scroll = self.max_list_scroll(&layout);
        self.list_scroll_y = self.list_scroll_y.clamp(0, max_scroll);
        self.sync_pin_from_scroll(&layout);
        if max_scroll == 0 {
            self.scrollbar_dragging = false;
            self.scrollbar_arrow_captured = false;
            self.scrollbar_arrow = 0;
        }
    }

    fn sync_pin_from_scroll(&mut self, layout: &PlrSelLayout) {
        let max_scroll = self.max_list_scroll(layout);
        self.list_scroll_pin = if max_scroll == 0 || !Self::scrollbar_has_pin(layout) {
            0
        } else {
            Self::scrollbar_range(layout) * self.list_scroll_y / max_scroll
        };
    }

    fn scroll_list_by(&mut self, amount: i32, layout: &PlrSelLayout) {
        self.list_scroll_y = self
            .list_scroll_y
            .saturating_add(amount)
            .clamp(0, self.max_list_scroll(layout));
        self.sync_pin_from_scroll(layout);
    }

    fn set_scroll_from_pointer(&mut self, point: GuiPoint, layout: &PlrSelLayout) {
        let max_pin = Self::scrollbar_range(layout);
        self.list_scroll_pin =
            (point.y as i32 - layout.list_scrollbar.y - SCROLLBAR_PART - SCROLLBAR_PART / 2)
                .clamp(0, max_pin);
        self.list_scroll_y = self.max_list_scroll(layout) * self.list_scroll_pin / max_pin.max(1);
    }

    fn scrollbar_arrow_at(&self, point: GuiPoint, layout: &PlrSelLayout) -> i8 {
        if !contains_plrsel(layout.list_scrollbar, point) {
            return 0;
        }
        let local_y = point.y as i32 - layout.list_scrollbar.y;
        if local_y < SCROLLBAR_PART {
            -1
        } else if local_y >= layout.list_scrollbar.h - SCROLLBAR_PART {
            1
        } else {
            0
        }
    }

    fn begin_scrollbar_pointer(&mut self, point: GuiPoint, layout: &PlrSelLayout) {
        let arrow = self.scrollbar_arrow_at(point, layout);
        if arrow != 0 {
            self.scrollbar_arrow_captured = true;
            self.scrollbar_arrow = arrow;
        } else if Self::scrollbar_has_pin(layout) {
            self.scrollbar_arrow_captured = false;
            self.set_scroll_from_pointer(point, layout);
            self.scrollbar_dragging = true;
        }
    }

    fn ensure_selection_visible(&mut self, layout: &PlrSelLayout) {
        if layout.list_viewport.h <= 0 {
            self.list_scroll_y = 0;
            self.list_scroll_pin = 0;
            return;
        }
        let Some(index) = self.selected else {
            return;
        };
        let top = i32::try_from(index)
            .unwrap_or(i32::MAX)
            .saturating_mul(layout.item_pitch);
        let bottom = top.saturating_add(layout.item_height);
        if self.list_scroll_y > top {
            self.list_scroll_y = top;
        } else if self.list_scroll_y + layout.list_viewport.h < bottom {
            self.list_scroll_y = bottom - layout.list_viewport.h;
        }
        self.list_scroll_y = self.list_scroll_y.clamp(0, self.max_list_scroll(layout));
        self.sync_pin_from_scroll(layout);
    }

    fn is_highlighted(&self, control: PlrSelControl, draw_focus: bool) -> bool {
        (draw_focus && self.focus == control) || self.hovered == Some(control)
    }

    fn pointer_button_is_down(&self) -> bool {
        self.pointer_pressed
            .is_some_and(|pressed| self.hovered == Some(pressed))
    }

    fn is_pressed(&self, control: PlrSelControl) -> bool {
        (self.pointer_pressed == Some(control) && self.hovered == Some(control))
            || self
                .key_pressed
                .is_some_and(|(pressed, _)| pressed == control)
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
        Self::render_impl(
            surface,
            assets,
            fonts,
            book,
            players,
            &[],
            selected,
            None,
            None,
            true,
            gamma,
        );
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
            &[],
            controller.selected,
            Some(controller),
            None,
            true,
            gamma,
        );
    }

    /// Draws live controller state while allowing an owning popup to suppress
    /// the retained control's focus visuals, as `Control::HasDrawFocus()` does
    /// while a C++ screen context menu is open. Pointer hover remains visible.
    #[allow(clippy::too_many_arguments)]
    pub fn render_controller_with_draw_focus(
        surface: &mut Surface,
        assets: &PlrSelAssets,
        fonts: &ClonkFontSet,
        book: &BookFontSet,
        players: &[PlrSelPlayer],
        controller: &PlrSelController,
        draw_focus: bool,
        gamma: Option<&GammaRamp>,
    ) {
        Self::render_impl(
            surface,
            assets,
            fonts,
            book,
            players,
            &[],
            controller.selected,
            Some(controller),
            None,
            draw_focus,
            gamma,
        );
    }

    /// Mode-aware live renderer. The controller selects which slice is
    /// visible; keeping both available lets the app retain player rows while
    /// a crew session is open and restore them without reconstructing UI.
    #[allow(clippy::too_many_arguments)]
    pub fn render_controller_with_crew(
        surface: &mut Surface,
        assets: &PlrSelAssets,
        fonts: &ClonkFontSet,
        book: &BookFontSet,
        players: &[PlrSelPlayer],
        crew: &[PlrSelCrew],
        controller: &PlrSelController,
        gamma: Option<&GammaRamp>,
    ) {
        Self::render_impl(
            surface,
            assets,
            fonts,
            book,
            players,
            crew,
            controller.selected,
            Some(controller),
            None,
            true,
            gamma,
        );
    }

    /// Mode-aware renderer variant used while an owning popup suppresses the
    /// retained control's keyboard-focus highlight.
    #[allow(clippy::too_many_arguments)]
    pub fn render_controller_with_crew_and_draw_focus(
        surface: &mut Surface,
        assets: &PlrSelAssets,
        fonts: &ClonkFontSet,
        book: &BookFontSet,
        players: &[PlrSelPlayer],
        crew: &[PlrSelCrew],
        controller: &PlrSelController,
        draw_focus: bool,
        gamma: Option<&GammaRamp>,
    ) {
        Self::render_impl(
            surface,
            assets,
            fonts,
            book,
            players,
            crew,
            controller.selected,
            Some(controller),
            None,
            draw_focus,
            gamma,
        );
    }

    /// Mode-aware renderer variant with an inline replacement for one crew
    /// name label, matching `C4GUI::RenameEdit` ownership and draw order.
    #[allow(clippy::too_many_arguments)]
    pub fn render_controller_with_crew_rename_and_draw_focus(
        surface: &mut Surface,
        assets: &PlrSelAssets,
        fonts: &ClonkFontSet,
        book: &BookFontSet,
        players: &[PlrSelPlayer],
        crew: &[PlrSelCrew],
        controller: &PlrSelController,
        crew_rename: Option<(usize, &mut RenameEdit<PlrSelControl>)>,
        draw_focus: bool,
        gamma: Option<&GammaRamp>,
    ) {
        Self::render_impl(
            surface,
            assets,
            fonts,
            book,
            players,
            crew,
            controller.selected,
            Some(controller),
            crew_rename,
            draw_focus,
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
        crew: &[PlrSelCrew],
        selected: Option<usize>,
        controller: Option<&PlrSelController>,
        mut crew_rename: Option<(usize, &mut RenameEdit<PlrSelControl>)>,
        draw_focus: bool,
        gamma: Option<&GammaRamp>,
    ) {
        let (w, h) = (surface.width() as i32, surface.height() as i32);
        let layout = plrsel_layout_with_fonts(w, h, fonts, book);
        // Engine texture upload: fully transparent PNG texels turn black
        // (C4Surface::ReadPNG, C4Surface.cpp:972).
        let assets = &PlrSelAssets {
            background: engine_png_texture(&assets.background),
            checkbox: engine_png_texture(&assets.checkbox),
            button: engine_png_texture(&assets.button),
            button_down: engine_png_texture(&assets.button_down),
            button_highlight: engine_png_texture(&assets.button_highlight),
            book_scroll: engine_png_texture(&assets.book_scroll),
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

        let crew_mode = controller.is_some_and(PlrSelController::is_crew_mode);
        let row_count = if crew_mode { crew.len() } else { players.len() };
        let scroll_y = controller.map_or(0, PlrSelController::list_scroll_offset);

        // 2. List box: selection bar behind the items (ListBox::DrawElement,
        //    C4GuiListBox.cpp:100-124), then the items in add-order. Its
        //    ScrollWindow shifts children and clips them to the viewport.
        let viewport = layout.list_viewport;
        let saved_clip = surface.clip();
        let viewport_clip = clonk_graphics::Rect::new(
            viewport.x,
            viewport.y,
            viewport.w.max(0) as u32,
            viewport.h.max(0) as u32,
        );
        let active_clip = saved_clip
            .and_then(|clip| clip.intersection(viewport_clip))
            .unwrap_or_else(|| {
                if saved_clip.is_some() {
                    clonk_graphics::Rect::new(viewport.x, viewport.y, 0, 0)
                } else {
                    viewport_clip
                }
            });
        surface.set_clip(active_clip);

        if let Some(sel) = selected.filter(|&sel| sel < row_count) {
            let y = layout.list_viewport.y + layout.item_pitch * sel as i32 - scroll_y;
            let color = if crew_rename.is_none()
                && controller
                    .is_none_or(|state| draw_focus && state.focus == PlrSelControl::PlayerList)
            {
                CLR_LIST_BOX_SEL
            } else {
                CLR_LIST_BOX_INACTIVE_SEL
            };
            draw_box_dw(
                surface,
                layout.list_viewport.x,
                y,
                layout.list_viewport.x + layout.item_width - 1,
                y + layout.item_height - 1,
                color,
                gamma,
            );
        }
        if crew_mode {
            for (i, member) in crew.iter().enumerate() {
                let participating = controller
                    .and_then(|state| state.crew_participations.get(i).copied())
                    .unwrap_or(member.participating);
                let is_renaming = crew_rename
                    .as_ref()
                    .is_some_and(|(rename_index, _)| *rename_index == i);
                Self::render_crew_list_item(
                    surface,
                    assets,
                    book,
                    &layout,
                    member,
                    participating,
                    i as i32,
                    scroll_y,
                    !is_renaming,
                    gamma,
                );
                if is_renaming {
                    let item = IntRect {
                        x: layout.list_viewport.x,
                        y: layout.list_viewport.y + layout.item_pitch * i as i32 - scroll_y,
                        w: layout.item_width,
                        h: layout.item_height,
                    };
                    let edit_x = item.x + (item.h + 2) * 2;
                    if let Some((_, edit)) = crew_rename.as_mut() {
                        edit.render_with_draw_focus(
                            surface,
                            &fonts.text,
                            IntRect {
                                x: edit_x,
                                y: item.y + 2,
                                w: (item.x + item.w - edit_x - 2).max(1),
                                h: (item.h - 4).max(1),
                            },
                            draw_focus,
                            gamma,
                        );
                    }
                }
            }
        } else {
            for (i, player) in players.iter().enumerate() {
                let activated = controller
                    .and_then(|state| state.player_activations.get(i).copied())
                    .unwrap_or(player.activated);
                Self::render_list_item(
                    surface, assets, book, &layout, player, activated, i as i32, scroll_y, gamma,
                );
            }
        }

        if let Some(saved) = saved_clip {
            surface.set_clip(saved);
        } else {
            surface.clear_clip();
        }

        if let Some(controller) = controller {
            if controller.max_list_scroll(&layout) > 0 {
                Self::draw_scrollbar(surface, assets, controller, &layout, gamma);
            }
        }

        if crew_mode {
            if let Some(member) = selected.and_then(|sel| crew.get(sel)) {
                Self::render_crew_selection_info(surface, book, &layout, member, gamma);
                Self::render_crew_portrait(surface, &layout, member, gamma);
            }
        } else {
            // 3. Info panel text for the selected player
            //    (PlayerListItem::SetSelectionInfo, cpp:293-302).
            if let Some(player) = selected.and_then(|sel| players.get(sel)) {
                Self::render_selection_info(surface, book, &layout, player, gamma);
            }

            // 4. Portrait picture, ColorByOwner-tinted (cpp:798-801).
            if let Some(player) = selected.and_then(|sel| players.get(sel)) {
                Self::render_portrait(surface, assets, &layout, player, gamma);
            }
        }

        // 5.-10. Bottom buttons (Button::DrawElement, C4GuiButton.cpp:80-111).
        let activate_label = selected
            .and_then(|sel| {
                if crew_mode {
                    controller
                        .and_then(|state| state.crew_participations.get(sel).copied())
                        .or_else(|| crew.get(sel).map(|member| member.participating))
                } else {
                    controller
                        .and_then(|state| state.player_activations.get(sel).copied())
                        .or_else(|| players.get(sel).map(|player| player.activated))
                }
            })
            .map_or("Activate", |activated| {
                if activated {
                    "Deactivate"
                } else {
                    "Activate"
                }
            });
        if crew_mode {
            let buttons = [
                (PlrSelControl::Back, "Back"),
                (PlrSelControl::Activate, activate_label),
                (PlrSelControl::Delete, "Delete"),
                (PlrSelControl::Rename, "Rename"),
            ];
            for (rect, (control, label)) in layout.crew_buttons.into_iter().zip(buttons) {
                Self::render_button(
                    surface,
                    assets,
                    fonts,
                    rect,
                    label,
                    controller.is_some_and(|state| state.is_highlighted(control, draw_focus)),
                    controller.is_some_and(|state| state.is_pressed(control)),
                    gamma,
                );
            }
        } else {
            let buttons = [
                (PlrSelControl::Back, "Back"),
                (PlrSelControl::NewPlayer, "New"),
                (PlrSelControl::Activate, activate_label),
                (PlrSelControl::Delete, "Delete"),
                (PlrSelControl::Properties, "Properties"),
                (PlrSelControl::Crew, "Crew"),
            ];
            for (rect, (control, label)) in layout.buttons.into_iter().zip(buttons) {
                Self::render_button(
                    surface,
                    assets,
                    fonts,
                    rect,
                    label,
                    controller.is_some_and(|state| state.is_highlighted(control, draw_focus)),
                    controller.is_some_and(|state| state.is_pressed(control)),
                    gamma,
                );
            }
        }

        // 11. Fullscreen title, drawn last (SetTitle re-adds the label at the
        //     list end, C4GuiDialogs.cpp:835-847; cpp:693).
        let title = controller.map_or_else(
            || "Player Selection".to_string(),
            |state| state.dialog_title(),
        );
        fonts.title.draw_with_gamma(
            surface,
            layout.title_anchor.0,
            layout.title_anchor.1,
            &title,
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

    fn draw_scrollbar(
        surface: &mut Surface,
        assets: &PlrSelAssets,
        controller: &PlrSelController,
        layout: &PlrSelLayout,
        gamma: Option<&GammaRamp>,
    ) {
        let bar = layout.list_scrollbar;
        let top_x = if controller.scrollbar_arrow < 0 {
            16
        } else {
            0
        };
        let bottom_x = if controller.scrollbar_arrow > 0 {
            16
        } else {
            0
        };
        crate::draw_image_strip(
            surface,
            bar.x,
            bar.y,
            &assets.book_scroll,
            top_x,
            0,
            16,
            16,
            gamma,
        );
        let mut y = SCROLLBAR_PART;
        while y < bar.h - 5 {
            let tile_height = SCROLLBAR_PART.min(bar.h - 5 - y).max(0) as u32;
            if tile_height == 0 {
                break;
            }
            crate::draw_image_strip(
                surface,
                bar.x,
                bar.y + y,
                &assets.book_scroll,
                0,
                16,
                16,
                tile_height,
                gamma,
            );
            y += SCROLLBAR_PART;
        }
        crate::draw_image_strip(
            surface,
            bar.x,
            bar.y + bar.h - SCROLLBAR_PART,
            &assets.book_scroll,
            bottom_x,
            32,
            16,
            16,
            gamma,
        );
        if PlrSelController::scrollbar_has_pin(layout) {
            crate::draw_image_strip(
                surface,
                bar.x,
                bar.y + SCROLLBAR_PART + controller.list_scroll_pin,
                &assets.book_scroll,
                16,
                16,
                16,
                16,
                gamma,
            );
        }
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
        scroll_y: i32,
        gamma: Option<&GammaRamp>,
    ) {
        let item = IntRect {
            x: layout.list_viewport.x,
            y: layout.list_viewport.y + layout.item_pitch * index - scroll_y,
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
            &gui_rect(IntRect {
                x: item.x,
                y: item.y,
                w: item.h,
                h: item.h,
            }),
            &cb,
            gamma,
        );
        // Icon at x = iHeight + IconLabelSpacing (cpp:88), aspect-centered
        // (Picture::DrawElement, C4GuiLabels.cpp:348-378).
        let icon_box = IntRect {
            x: item.x + item.h + 2,
            y: item.y,
            w: item.h,
            h: item.h,
        };
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

    /// One CrewListItem: participation checkbox, resolved rank icon and name
    /// (C4StartupPlrSelDlg.cpp:341-373).
    #[allow(clippy::too_many_arguments)]
    fn render_crew_list_item(
        surface: &mut Surface,
        assets: &PlrSelAssets,
        book: &BookFontSet,
        layout: &PlrSelLayout,
        crew: &PlrSelCrew,
        participating: bool,
        index: i32,
        scroll_y: i32,
        draw_name: bool,
        gamma: Option<&GammaRamp>,
    ) {
        let item = IntRect {
            x: layout.list_viewport.x,
            y: layout.list_viewport.y + layout.item_pitch * index - scroll_y,
            w: layout.item_width,
            h: layout.item_height,
        };
        let phase = u32::from(participating);
        let checkbox = extract_region(&assets.checkbox, phase * 32, 0, 32, 32);
        crate::draw_image_bilinear(
            surface,
            &gui_rect(IntRect {
                x: item.x,
                y: item.y,
                w: item.h,
                h: item.h,
            }),
            &checkbox,
            gamma,
        );

        let icon_box = IntRect {
            x: item.x + item.h + 2,
            y: item.y,
            w: item.h,
            h: item.h,
        };
        if let Some(icon) = &crew.rank_icon {
            let icon = engine_png_texture(icon);
            let dest = aspect_fit(icon.width() as i32, icon.height() as i32, icon_box);
            crate::draw_image_bilinear(surface, &gui_rect(dest), &icon, gamma);
        }
        if draw_name {
            book.text.draw_with_gamma(
                surface,
                item.x + (item.h + 2) * 2,
                item.y + 2,
                &crew.name,
                CLR_PLAYER_ITEM,
                TextAlign::Left,
                false,
                gamma,
            );
        }
    }

    /// Crew detail text from `CrewListItem::SetSelectionInfo`
    /// (C4StartupPlrSelDlg.cpp:467-507).
    fn render_crew_selection_info(
        surface: &mut Surface,
        book: &BookFontSet,
        layout: &PlrSelLayout,
        crew: &PlrSelCrew,
        gamma: Option<&GammaRamp>,
    ) {
        let caption = format!("{} {}", crew.rank_name, crew.name);
        let mut lines = vec![
            format!("Type: {}", crew.type_name),
            format!("Experience: {}", crew.experience),
            format!("Rounds: {}", crew.rounds),
            format!("Died: {} x", crew.death_count),
        ];
        if let Some(next_rank) = &crew.next_rank {
            lines.push(format!("Promotion to {}", next_rank.rank_name));
            lines.push(format!("at: {}", next_rank.experience));
        } else {
            lines.push("No further promotions.".to_string());
        }
        lines.push(format!(
            "Playing time: {}",
            time_string(crew.total_playing_time)
        ));
        lines.push(format!("Birthday: {}", crew.birthday));

        let physical = crew.physical;
        lines.extend([
            Self::physical_text_line("Energy:", physical.energy),
            Self::physical_text_line("Breath:", physical.breath),
            Self::physical_text_line("Walk:", physical.walk),
            Self::physical_text_line("Jump:", physical.jump),
        ]);
        if physical.can_scale != 0 {
            lines.push(Self::physical_text_line("Scale:", physical.scale));
        }
        if physical.can_hangle != 0 {
            lines.push(Self::physical_text_line("Hangle:", physical.hangle));
        }
        lines.extend([
            Self::physical_text_line("Dig:", physical.dig),
            Self::physical_text_line("Swim:", physical.swim),
            Self::physical_text_line("Throw:", physical.throw),
            Self::physical_text_line("Push:", physical.push),
            Self::physical_text_line("Fight:", physical.fight),
        ]);
        if physical.magic != 0 {
            lines.push(Self::physical_text_line("Magic:", physical.magic));
        }

        let mut y = layout.info_client.y;
        book.caption.draw_with_gamma(
            surface,
            layout.info_client.x,
            y,
            &caption,
            CLR_PLAYER_ITEM,
            TextAlign::Left,
            false,
            gamma,
        );
        y += book.caption.line_height;
        let bottom = layout.info_client.y + layout.info_client.h;
        for line in lines {
            y += book.text.line_height / 3;
            if y + book.text.line_height > bottom {
                break;
            }
            book.text.draw_with_gamma(
                surface,
                layout.info_client.x,
                y,
                &line,
                CLR_PLAYER_ITEM,
                TextAlign::Left,
                false,
                gamma,
            );
            y += book.text.line_height;
        }
    }

    fn physical_text_line(label: &str, value: i32) -> String {
        let bars = (10_i64 * i64::from(value) / i64::from(C4_MAX_PHYSICAL)).max(0) as usize;
        format!("{label} {}", "\u{b7}".repeat(bars))
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
        let dest = aspect_fit(
            source.width() as i32,
            source.height() as i32,
            layout.picture_area,
        );
        Self::draw_color_by_owner(surface, &source, dest, player.color_dw, gamma);
    }

    /// Crew portraits do not use the player's default icon as fallback;
    /// `LoadPortrait(..., false)` leaves the picture blank when unresolved.
    fn render_crew_portrait(
        surface: &mut Surface,
        layout: &PlrSelLayout,
        crew: &PlrSelCrew,
        gamma: Option<&GammaRamp>,
    ) {
        let Some(portrait) = &crew.portrait else {
            return;
        };
        let source = engine_png_texture(portrait);
        let dest = aspect_fit(
            source.width() as i32,
            source.height() as i32,
            layout.picture_area,
        );
        Self::draw_color_by_owner(surface, &source, dest, crew.color_dw, gamma);
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

    fn size_sixteen_layout_fonts() -> (ClonkFontSet, BookFontSet) {
        let defaults = crate::test_support::endeavour_font_set();
        let mut fonts = ClonkFontSet {
            title: defaults.title.clone(),
            caption: defaults.caption.clone(),
            text: defaults.text.clone(),
            main_small: defaults.main_small.clone(),
            mini: defaults.mini.clone(),
        };
        let mut book = book_fonts();
        // Configured base size 16 produces these line heights in the current
        // classic font bundles. Only layout metrics matter to these tests.
        fonts.title.line_height = 39;
        book.text.line_height = 25;
        (fonts, book)
    }

    #[test]
    fn retained_player_box_and_owner_portrait_are_two_commands() {
        let image = ImageData::new(2, 2, [120, 120, 120, 200].repeat(4));
        let mut surface = Surface::new(200, 120, clonk_graphics::PixelFormat::Rgba8888);
        surface.begin_gpu_scene_capture();
        draw_box_dw(&mut surface, 1, 2, 180, 100, 0x7f20_4060, None);
        draw_image_bilinear_modulated(
            &mut surface,
            &GuiRect::new(20.0, 10.0, 160.0, 100.0),
            &image,
            0x0011_2233,
            None,
        );

        let scene = surface
            .take_gpu_scene_capture()
            .expect("capture remains active")
            .into_scene([200, 120], Color::transparent(), &GammaRamp::identity());
        assert_eq!(scene.commands.len(), 2);
        assert!(matches!(
            &scene.commands[0],
            clonk_graphics::GpuCommand::Solid { .. }
        ));
        let clonk_graphics::GpuCommand::Quad { sampler, .. } = &scene.commands[1] else {
            panic!("owner portrait overlay was not retained as a texture quad");
        };
        assert_eq!(*sampler, clonk_graphics::GpuSampler::Linear);
    }

    // Pixel-exact C4StartupPlrSelDlg geometry at 1280x720, derived from
    // C4StartupPlrSelDlg.cpp:550-562/636-657, C4GuiDialogs.cpp:819-820 and
    // C4StartupPlrSelDlg.h:221, verified against an F9 screenshot of the C++
    // engine at 1280x720 (see target/parity-specs/plrsel.md).
    #[test]
    fn layout_matches_cpp_plrsel_dlg_at_1280x720() {
        let l = plrsel_layout(1280, 720);

        // Client: margins x=1280/50=25, y=720*2/75=19, top=720/7=102.
        assert_eq!(
            (l.client.x, l.client.y, l.client.w, l.client.h),
            (25, 102, 1230, 599)
        );

        // Player list box (123,155,379,373) client-rel → screen.
        assert_eq!(
            (l.plr_list.x, l.plr_list.y, l.plr_list.w, l.plr_list.h),
            (148, 257, 379, 373)
        );
        // List client: +3px margins.
        assert_eq!(
            (
                l.list_client.x,
                l.list_client.y,
                l.list_client.w,
                l.list_client.h
            ),
            (151, 260, 373, 367)
        );
        assert_eq!(
            l.list_viewport,
            IntRect {
                x: 151,
                y: 260,
                w: 357,
                h: 367
            }
        );
        assert_eq!(
            l.list_scrollbar,
            IntRect {
                x: 508,
                y: 260,
                w: 16,
                h: 367
            }
        );
        // Items: 357 wide (373-16 scrollbar), 26 high, 27px pitch.
        assert_eq!((l.item_width, l.item_height, l.item_pitch), (357, 26, 27));

        // Info window (594,244,387,300) client-rel → screen; text client
        // shrunk by margins 10/8/5/8 and the 16px scrollbar.
        assert_eq!(
            (
                l.info_window.x,
                l.info_window.y,
                l.info_window.w,
                l.info_window.h
            ),
            (619, 346, 387, 300)
        );
        assert_eq!(
            (
                l.info_client.x,
                l.info_client.y,
                l.info_client.w,
                l.info_client.h
            ),
            (629, 354, 356, 284)
        );

        // Portrait picture area (781,94,200,150) client-rel → screen.
        assert_eq!(
            (
                l.picture_area.x,
                l.picture_area.y,
                l.picture_area.w,
                l.picture_area.h
            ),
            (806, 196, 200, 150)
        );

        // Bottom buttons: 187x32 at x=34+205*i, y=665.
        for (i, b) in l.buttons.iter().enumerate() {
            assert_eq!(
                (b.x, b.y, b.w, b.h),
                (34 + 205 * i as i32, 665, 187, 32),
                "button {i}"
            );
        }
        // Crew uses four 307px grid sectors while retaining the 187px plank
        // width calculated for the six-button player row.
        for (i, b) in l.crew_buttons.iter().enumerate() {
            assert_eq!(
                (b.x, b.y, b.w, b.h),
                (85 + 307 * i as i32, 665, 187, 32),
                "crew button {i}"
            );
        }

        // Title label anchor: centered at x=640, y=8.
        assert_eq!(l.title_anchor, (640, 8));
    }

    #[test]
    fn size_sixteen_font_metrics_drive_player_selection_layout() {
        let (fonts, book) = size_sixteen_layout_fonts();

        let layout = plrsel_layout_with_fonts(1280, 720, &fonts, &book);
        assert_eq!((layout.item_height, layout.item_pitch), (29, 30));
        assert_eq!(layout.title_anchor, (640, 6));

        // The compatibility wrapper remains pinned to the reference-capture
        // defaults used by existing callers and tests.
        let default_layout = plrsel_layout(1280, 720);
        assert_eq!(
            (default_layout.item_height, default_layout.item_pitch),
            (26, 27)
        );
        assert_eq!(default_layout.title_anchor, (640, 8));
    }

    #[test]
    fn size_sixteen_font_metrics_reflow_controller_hits_and_scroll() {
        let (fonts, book) = size_sixteen_layout_fonts();
        let layout = plrsel_layout_with_fonts(1280, 720, &fonts, &book);

        let mut controller = PlrSelController::new(20);
        controller.resize(1280, 720);
        controller.set_selected_index(Some(19));
        assert_eq!(controller.list_scroll_offset(), 172);

        controller.set_layout_fonts(&fonts, &book);
        assert_eq!(controller.layout(), layout);
        assert_eq!(controller.list_max_scroll(), 232);
        assert_eq!(controller.list_scroll_offset(), 232);

        controller.set_selected_index(Some(0));
        let name_x = (layout.list_client.x + layout.item_height * 2) as f32;
        assert_eq!(
            controller.context_index_at(GuiPoint::new(
                name_x,
                (layout.list_viewport.y + layout.item_height - 1) as f32,
            )),
            Some(0),
        );
        assert_eq!(
            controller.context_index_at(GuiPoint::new(
                name_x,
                (layout.list_viewport.y + layout.item_height) as f32,
            )),
            None,
            "the one-pixel C4GUI list spacing is not a row target",
        );
        assert_eq!(
            controller.context_index_at(GuiPoint::new(
                name_x,
                (layout.list_viewport.y + layout.item_pitch) as f32,
            )),
            Some(1),
        );
    }

    fn center(rect: IntRect) -> crate::GuiPoint {
        crate::GuiPoint::new((rect.x + rect.w / 2) as f32, (rect.y + rect.h / 2) as f32)
    }

    fn click(controller: &mut PlrSelController, rect: IntRect) -> Vec<PlrSelAction> {
        let point = center(rect);
        let _ = controller.handle_pointer_down(point);
        controller.handle_pointer_up(point)
    }

    #[test]
    fn player_context_menu_matches_cpp_entries_and_commands() {
        let menu = PlrSelPlayerContextMenu::for_player(3);
        assert_eq!(menu.initial_selection, None);
        assert_eq!(
            menu.entries,
            [
                PlrSelPlayerContextEntry {
                    label: "Properties",
                    tooltip: Some("Change player color and preferred controls."),
                    icon: PlrSelPlayerContextIcon::None,
                    hotkey: None,
                    command: PlrSelPlayerContextCommand::PlayerProperties(3),
                },
                PlrSelPlayerContextEntry {
                    label: "Delete",
                    tooltip: Some("Delete the selected player file."),
                    icon: PlrSelPlayerContextIcon::None,
                    hotkey: None,
                    command: PlrSelPlayerContextCommand::DeletePlayer(3),
                },
            ]
        );
        assert_eq!(
            PlrSelAction::from(menu.entries[0].command),
            PlrSelAction::PlayerProperties(3)
        );
        assert_eq!(
            PlrSelAction::from(menu.entries[1].command),
            PlrSelAction::DeletePlayer(3)
        );
    }

    #[test]
    fn crew_context_menu_matches_cpp_entries_and_commands() {
        let menu = PlrSelCrewContextMenu::for_crew(4);
        assert_eq!(menu.initial_selection, None);
        assert_eq!(
            menu.entries
                .iter()
                .map(|entry| entry.label)
                .collect::<Vec<_>>(),
            ["Rename", "Delete", "Set death message"]
        );
        assert_eq!(
            menu.entries.map(|entry| PlrSelAction::from(entry.command)),
            [
                PlrSelAction::RenameCrew(4),
                PlrSelAction::DeleteCrew(4),
                PlrSelAction::SetCrewDeathMessage(4),
            ]
        );
    }

    #[test]
    fn controller_enters_and_leaves_crew_with_exact_title_and_player_restore() {
        let mut controller = PlrSelController::new(3);
        controller.set_selected_index(Some(2));

        assert!(!controller.enter_crew_mode(2, "Ada", Vec::new()));
        assert_eq!(controller.mode(), &PlrSelMode::Player);
        assert_eq!(controller.dialog_title(), "Player Selection");
        assert_eq!(
            controller.handle_key_down(crate::KeyCode::Right),
            vec![PlrSelAction::ShowCrew(2)]
        );

        assert!(controller.enter_crew_mode(2, "Ada", vec![true, false]));
        assert_eq!(
            controller.mode(),
            &PlrSelMode::Crew {
                player_index: 2,
                player_name: "Ada".to_string(),
            }
        );
        assert_eq!(controller.dialog_title(), "Crew: Ada");
        assert_eq!(controller.row_count(), 2);
        assert_eq!(controller.selected_index(), Some(0));
        assert!(controller.handle_key_down(crate::KeyCode::Right).is_empty());
        assert_eq!(
            controller.handle_key_down(crate::KeyCode::Left),
            vec![PlrSelAction::LeaveCrew]
        );

        assert_eq!(controller.leave_crew_mode(), Some(2));
        assert_eq!(controller.mode(), &PlrSelMode::Player);
        assert_eq!(controller.selected_index(), Some(2));
        assert_eq!(controller.row_count(), 3);
    }

    #[test]
    fn tooltip_targets_follow_rows_participation_and_mode_specific_buttons() {
        let layout = plrsel_layout(1280, 720);
        let names = ["Ada", "Grace"];
        let mut controller = PlrSelController::new(names.len());
        controller.resize(1280, 720);

        for (rect, key) in [
            (layout.buttons[0], "IDS_DLGTIP_BACKMAIN"),
            (layout.buttons[1], "IDS_DLGTIP_NEWPLAYER"),
            (layout.buttons[3], "IDS_DLGTIP_PLAYERDELETE"),
            (layout.buttons[4], "IDS_DLGTIP_PLAYERPROPERTIES"),
            (layout.buttons[5], "IDS_DLGTIP_PLAYERCREW"),
        ] {
            assert_eq!(
                controller.tooltip_at(center(rect), names),
                Some(StartupTooltip::resource(key))
            );
        }
        assert_eq!(
            controller.tooltip_at(center(layout.buttons[2]), names),
            Some(StartupTooltip::formatted_resource(
                "IDS_MSG_PARTICIPATE_DESC",
                ["Ada"]
            ))
        );

        controller.set_player_activations(vec![true, false]);
        assert_eq!(
            controller.tooltip_at(center(layout.buttons[2]), names),
            Some(StartupTooltip::formatted_resource(
                "IDS_MSG_NOPARTICIPATE_DESC",
                ["Ada"]
            ))
        );
        let first_row = GuiPoint::new(
            (layout.list_client.x + layout.item_height * 2) as f32,
            (layout.list_client.y + layout.item_height / 2) as f32,
        );
        assert_eq!(
            controller.tooltip_at(first_row, names),
            Some(StartupTooltip::text("Ada"))
        );
        let row_gap = GuiPoint::new(
            (layout.list_client.x + 1) as f32,
            (layout.list_client.y + layout.item_height) as f32,
        );
        assert_eq!(
            controller.tooltip_at(row_gap, names),
            Some(StartupTooltip::resource("IDS_DLGTIP_PLAYERFILES"))
        );

        assert!(controller.enter_crew_mode(0, "Ada", vec![true, false]));
        for (rect, key) in [
            (layout.crew_buttons[0], "IDS_MSG_BACKTOPLAYERDLG"),
            (layout.crew_buttons[2], "IDS_MSG_DELETECLONK_DESC"),
            (layout.crew_buttons[3], "IDS_DESC_CREWRENAME"),
        ] {
            assert_eq!(
                controller.tooltip_at(center(rect), names),
                Some(StartupTooltip::resource(key))
            );
        }
        assert_eq!(
            controller.tooltip_at(center(layout.crew_buttons[1]), names),
            Some(StartupTooltip::formatted_resource(
                "IDS_MSG_NOPARTICIPATE_DESC",
                ["Ada"]
            ))
        );
    }

    #[test]
    fn tooltip_targets_follow_scrolled_player_rows() {
        let layout = plrsel_layout(1280, 720);
        let names: Vec<String> = (0..20).map(|index| format!("Player {index}")).collect();
        let mut controller = PlrSelController::new(names.len());
        controller.resize(1280, 720);
        let viewport_point = GuiPoint::new(
            (layout.list_viewport.x + 4) as f32,
            (layout.list_viewport.y + 4) as f32,
        );

        controller.handle_wheel(viewport_point, -60);

        assert_eq!(controller.list_scroll_offset(), 60);
        assert_eq!(
            controller.tooltip_at(viewport_point, names.iter().map(String::as_str)),
            Some(StartupTooltip::text("Player 2"))
        );
    }

    #[test]
    fn crew_controls_route_participation_delete_and_rename() {
        let layout = plrsel_layout(1280, 720);
        let mut controller = PlrSelController::new(2);
        controller.resize(1280, 720);
        controller.set_selected_index(Some(1));
        assert!(controller.enter_crew_mode(1, "Ada", vec![true, false]));

        assert_eq!(
            click(&mut controller, layout.crew_buttons[0]),
            vec![PlrSelAction::LeaveCrew]
        );
        assert_eq!(
            click(&mut controller, layout.crew_buttons[1]),
            vec![PlrSelAction::CrewParticipationChanged {
                index: 0,
                participating: false,
            }]
        );
        assert_eq!(controller.is_crew_participating(0), Some(false));
        assert_eq!(
            click(&mut controller, layout.crew_buttons[2]),
            vec![PlrSelAction::DeleteCrew(0)]
        );
        assert_eq!(
            click(&mut controller, layout.crew_buttons[3]),
            vec![PlrSelAction::RenameCrew(0)]
        );

        let second_row_name = crate::GuiPoint::new(
            (layout.list_client.x + layout.item_height * 3) as f32,
            (layout.list_client.y + layout.item_pitch + layout.item_height / 2) as f32,
        );
        assert_eq!(
            controller.handle_pointer_double_click(second_row_name),
            vec![
                PlrSelAction::SelectionChanged(Some(1)),
                PlrSelAction::RenameCrew(1),
            ]
        );
        assert_eq!(
            controller.handle_key_down(crate::KeyCode::Enter),
            vec![PlrSelAction::RenameCrew(1)]
        );
        assert_eq!(
            controller.handle_edit_shortcut(),
            vec![PlrSelAction::RenameCrew(1)]
        );
    }

    #[test]
    fn mode_aware_context_target_uses_current_row_set_without_changing_focus() {
        let layout = plrsel_layout(1280, 720);
        let point = crate::GuiPoint::new(
            (layout.list_client.x + layout.item_height * 3) as f32,
            (layout.list_client.y + layout.item_pitch + 2) as f32,
        );
        let mut controller = PlrSelController::new(3);
        controller.resize(1280, 720);
        assert_eq!(controller.context_index_at(point), Some(1));
        assert!(controller.select_for_context(1));
        assert_eq!(controller.selected_index(), Some(1));

        assert!(controller.enter_crew_mode(1, "Ada", vec![true, true]));
        assert_eq!(controller.player_context_index_at(point), None);
        assert_eq!(controller.crew_context_index_at(point), Some(1));
        assert_eq!(controller.context_index_at(point), Some(1));
        assert!(controller.select_for_context(1));
        assert!(!controller.select_for_context(2));
        assert_eq!(controller.focused_control(), PlrSelControl::PlayerList);
    }

    #[test]
    fn player_context_targets_the_whole_row_without_changing_focus() {
        let layout = plrsel_layout(1280, 720);
        let mut controller = PlrSelController::new(2);
        controller.resize(1280, 720);
        assert_eq!(
            controller.handle_key_down(crate::KeyCode::Tab),
            vec![PlrSelAction::FocusChanged(PlrSelControl::Back)]
        );
        assert_eq!(controller.focused_control(), PlrSelControl::Back);

        let second_y = layout.list_client.y + layout.item_pitch;
        for x in [
            layout.list_client.x,
            layout.list_client.x + layout.item_height * 3,
            layout.list_client.x + layout.item_width - 1,
        ] {
            assert_eq!(
                controller.player_context_index_at(crate::GuiPoint::new(
                    x as f32,
                    (second_y + layout.item_height / 2) as f32,
                )),
                Some(1),
                "row descendant at x={x}"
            );
        }
        assert_eq!(
            controller.player_context_index_at(crate::GuiPoint::new(
                (layout.list_client.x + 1) as f32,
                (layout.list_client.y + layout.item_height) as f32,
            )),
            None,
            "one-pixel item spacing is not a row"
        );
        assert_eq!(
            controller.player_context_index_at(crate::GuiPoint::new(
                (layout.list_client.x + layout.item_width) as f32,
                (second_y + 1) as f32,
            )),
            None,
            "the scrollbar is not part of a player row"
        );

        assert!(controller.select_player_for_context(1));
        assert_eq!(controller.selected_index(), Some(1));
        assert_eq!(controller.focused_control(), PlrSelControl::Back);
        assert!(!controller.select_player_for_context(2));
        assert_eq!(controller.selected_index(), Some(1));
        assert_eq!(controller.focused_control(), PlrSelControl::Back);
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

        assert_eq!(
            click(&mut controller, layout.buttons[0]),
            vec![PlrSelAction::Back]
        );
        assert_eq!(
            click(&mut controller, layout.buttons[1]),
            vec![PlrSelAction::NewPlayer]
        );
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

    #[test]
    fn l046_player_buttons_do_not_invent_absent_cpp_mnemonics() {
        let mut controller = PlrSelController::new(1);
        for character in ['N', 'A', 'D', 'P', 'C'] {
            assert_eq!(
                controller.handle_hotkey(character),
                None,
                "LanguageUS has no '&' marker for {character}"
            );
        }
        controller.set_player_activations(vec![true]);
        assert_eq!(
            controller.handle_hotkey('A'),
            None,
            "Deactivate is unmarked too"
        );
        assert_eq!(controller.handle_hotkey('-'), None);
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

    #[test]
    fn l047_typeahead_cycles_matching_rows_and_requires_list_focus() {
        let names = ["Thomas", "Ada", "tina", "Tori"];
        let mut controller = PlrSelController::new(names.len());
        controller.resize(1280, 720);

        for (character, expected) in [('T', 2), ('T', 3), ('T', 0), ('t', 2)] {
            assert_eq!(
                controller.handle_character(character, names),
                vec![PlrSelAction::SelectionChanged(Some(expected))]
            );
            assert_eq!(controller.selected_index(), Some(expected));
        }

        controller.set_selected_index(None);
        assert_eq!(
            controller.handle_character('t', names),
            vec![PlrSelAction::SelectionChanged(Some(0))],
            "an unselected list starts its search at the first row"
        );
        assert!(controller.handle_character('x', names).is_empty());
        assert_eq!(controller.selected_index(), Some(0));

        assert_eq!(
            controller.handle_key_down(crate::KeyCode::Tab),
            vec![PlrSelAction::FocusChanged(PlrSelControl::Back)]
        );
        assert!(controller.handle_character('T', names).is_empty());
        assert_eq!(controller.selected_index(), Some(0));
        assert_eq!(controller.keyboard_context_target(), None);
    }

    #[test]
    fn l047_keyboard_context_target_is_selected_row_center_not_pointer() {
        let layout = plrsel_layout(1280, 720);
        let mut controller = PlrSelController::new(4);
        controller.resize(1280, 720);
        controller.set_selected_index(Some(3));
        controller.set_pointer_position(Some(GuiPoint::new(1.0, 2.0)));

        assert_eq!(
            controller.keyboard_context_target(),
            Some((
                3,
                GuiPoint::new(
                    (layout.list_viewport.x + layout.item_width / 2) as f32,
                    (layout.list_viewport.y + 3 * layout.item_pitch + layout.item_height / 2)
                        as f32,
                ),
            ))
        );
    }

    #[test]
    fn l021_overflow_wheel_scrollbar_and_scrolled_hits_reach_every_row() {
        let layout = plrsel_layout(1280, 720);
        let mut controller = PlrSelController::new(20);
        controller.resize(1280, 720);

        assert_eq!(controller.list_max_scroll(), 172);
        let viewport_point = crate::GuiPoint::new(
            (layout.list_viewport.x + 4) as f32,
            (layout.list_viewport.y + 4) as f32,
        );
        controller.handle_wheel(viewport_point, -60);
        assert_eq!(controller.list_scroll_offset(), 60);
        assert_eq!(controller.context_index_at(viewport_point), Some(2));

        controller.handle_wheel(viewport_point, -10_000);
        assert_eq!(controller.list_scroll_offset(), 172);
        let last_checkbox = crate::GuiPoint::new(
            (layout.list_viewport.x + layout.item_height / 2) as f32,
            (layout.list_viewport.y + 19 * layout.item_pitch - 172 + layout.item_height / 2) as f32,
        );
        assert_eq!(
            controller.handle_pointer_down(last_checkbox),
            vec![PlrSelAction::SelectionChanged(Some(19))]
        );
        assert_eq!(
            controller.handle_pointer_up(last_checkbox),
            vec![PlrSelAction::ActivationChanged {
                index: 19,
                activated: true,
            }]
        );
        assert!(controller.player_activations()[..19]
            .iter()
            .all(|activated| !activated));
        assert!(controller.player_activations()[19]);

        let last_name = crate::GuiPoint::new(
            (layout.list_viewport.x + layout.item_height * 3) as f32,
            last_checkbox.y,
        );
        assert_eq!(controller.context_index_at(last_name), Some(19));
        assert_eq!(
            controller.handle_pointer_double_click(last_name),
            vec![PlrSelAction::PlayerProperties(19)]
        );
        assert_eq!(
            click(&mut controller, layout.buttons[3]),
            vec![PlrSelAction::DeletePlayer(19)]
        );

        let over_bar = crate::GuiPoint::new(
            (layout.list_scrollbar.x + 8) as f32,
            (layout.list_scrollbar.y + 8) as f32,
        );
        controller.handle_wheel(over_bar, 10_000);
        assert_eq!(controller.list_scroll_offset(), 172);

        controller.handle_wheel(viewport_point, 10_000);
        assert_eq!(controller.list_scroll_offset(), 0);
        let middle_track = crate::GuiPoint::new(
            (layout.list_scrollbar.x + 8) as f32,
            (layout.list_scrollbar.y + layout.list_scrollbar.h / 2) as f32,
        );
        assert!(controller.handle_pointer_down(middle_track).is_empty());
        assert_eq!(controller.list_scroll_offset(), 85);
        let bottom_track = crate::GuiPoint::new(
            middle_track.x,
            (layout.list_scrollbar.y + layout.list_scrollbar.h - 17) as f32,
        );
        controller.handle_pointer_move(bottom_track);
        assert_eq!(controller.list_scroll_offset(), 172);
        controller.handle_pointer_up(bottom_track);
        assert_eq!(controller.list_scroll_offset(), 172);
    }

    #[test]
    fn l021_keyboard_selection_scrolls_each_row_into_view() {
        let layout = plrsel_layout(1280, 720);
        let mut controller = PlrSelController::new(20);
        controller.resize(1280, 720);

        for index in 1..20 {
            assert_eq!(
                controller.handle_key_down(crate::KeyCode::Down),
                vec![PlrSelAction::SelectionChanged(Some(index))]
            );
            let expected = (index as i32 * layout.item_pitch + layout.item_height
                - layout.list_viewport.h)
                .max(0);
            assert_eq!(controller.list_scroll_offset(), expected);
            let top = index as i32 * layout.item_pitch;
            assert!(controller.list_scroll_offset() <= top);
            assert!(
                controller.list_scroll_offset() + layout.list_viewport.h
                    >= top + layout.item_height
            );
        }
        assert_eq!(controller.list_scroll_offset(), 172);

        for index in (0..19).rev() {
            assert_eq!(
                controller.handle_key_down(crate::KeyCode::Up),
                vec![PlrSelAction::SelectionChanged(Some(index))]
            );
            assert_eq!(
                controller.list_scroll_offset(),
                (index as i32 * layout.item_pitch).min(172)
            );
        }
        assert_eq!(controller.list_scroll_offset(), 0);

        let mut initial = PlrSelController::new(20);
        let mut activations = vec![false; 20];
        activations[19] = true;
        initial.set_player_activations(activations);
        initial.resize(1280, 720);
        assert_eq!(initial.selected_index(), Some(19));
        assert_eq!(initial.list_scroll_offset(), 172);
    }

    #[test]
    fn l057_home_end_and_pages_use_fully_visible_player_rows() {
        let mut controller = PlrSelController::new(20);
        controller.resize(1280, 720);

        assert!(controller.handle_key_down(KeyCode::Home).is_empty());
        assert_eq!(controller.selected_index(), Some(0));
        assert_eq!(controller.list_scroll_offset(), 0);
        assert_eq!(
            controller.handle_key_down(KeyCode::End),
            vec![PlrSelAction::SelectionChanged(Some(19))]
        );
        assert_eq!(controller.list_scroll_offset(), 172);
        assert_eq!(
            controller.handle_key_down(KeyCode::Home),
            vec![PlrSelAction::SelectionChanged(Some(0))]
        );
        assert_eq!(controller.list_scroll_offset(), 0);

        for (key, expected_index, expected_scroll) in [
            (KeyCode::PageDown, 12, 0),
            (KeyCode::PageDown, 19, 172),
            (KeyCode::PageUp, 7, 172),
            (KeyCode::PageUp, 0, 0),
        ] {
            assert_eq!(
                controller.handle_key_down(key),
                vec![PlrSelAction::SelectionChanged(Some(expected_index))]
            );
            assert_eq!(controller.selected_index(), Some(expected_index));
            assert_eq!(controller.list_scroll_offset(), expected_scroll);
        }
        assert!(controller.handle_key_down(KeyCode::PageUp).is_empty());

        assert_eq!(
            controller.handle_key_down(KeyCode::Tab),
            vec![PlrSelAction::FocusChanged(PlrSelControl::Back)]
        );
        for key in [
            KeyCode::Home,
            KeyCode::End,
            KeyCode::PageUp,
            KeyCode::PageDown,
        ] {
            assert!(controller.handle_key_down(key).is_empty());
            assert_eq!(controller.selected_index(), Some(0));
            assert_eq!(controller.list_scroll_offset(), 0);
        }

        let mut unselected = PlrSelController::new(20);
        unselected.resize(1280, 720);
        unselected.set_selected_index(None);
        assert_eq!(
            unselected.handle_key_down(KeyCode::PageDown),
            vec![PlrSelAction::SelectionChanged(Some(12))]
        );
        unselected.set_selected_index(None);
        assert_eq!(
            unselected.handle_key_down(KeyCode::PageUp),
            vec![PlrSelAction::SelectionChanged(Some(0))]
        );
    }

    #[test]
    fn l021_scrolled_rows_and_selection_are_clipped_to_list_viewport() {
        use clonk_graphics::PixelFormat;

        let assets = PlrSelAssets {
            background: crate::test_support::load_graphics_png("StartupPlrSelBG.png"),
            checkbox: crate::test_support::load_graphics_png("GUICheckbox.png"),
            button: crate::test_support::load_graphics_png("GUIButton.png"),
            button_down: crate::test_support::load_graphics_png("GUIButtonDown.png"),
            button_highlight: crate::test_support::load_graphics_png("GUIButtonHighlight.png"),
            book_scroll: crate::test_support::load_graphics_png("StartupBookScroll.png"),
            player: crate::test_support::load_graphics_png("Player.png"),
        };
        let fonts = crate::test_support::endeavour_font_set();
        let book = book_fonts();
        let gamma = crate::test_support::standard_gamma();
        let players = (0..20)
            .map(|index| {
                let mut player = tyler();
                player.name = format!("Player {index:02}");
                player
            })
            .collect::<Vec<_>>();
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
        let top = render(&controller);
        let layout = plrsel_layout(1280, 720);
        controller.handle_wheel(
            crate::GuiPoint::new(
                (layout.list_viewport.x + 4) as f32,
                (layout.list_viewport.y + 4) as f32,
            ),
            -10_000,
        );
        let bottom = render(&controller);

        let mut inside_differences = 0;
        let mut viewport_differences = 0;
        for y in 0..720_u32 {
            for x in 0..1280_u32 {
                let differs = top.get_pixel(x, y) != bottom.get_pixel(x, y);
                let inside_client = x >= layout.list_client.x as u32
                    && x < (layout.list_client.x + layout.list_client.w) as u32
                    && y >= layout.list_client.y as u32
                    && y < (layout.list_client.y + layout.list_client.h) as u32;
                if inside_client {
                    inside_differences += usize::from(differs);
                    let inside_viewport = x >= layout.list_viewport.x as u32
                        && x < (layout.list_viewport.x + layout.list_viewport.w) as u32
                        && y >= layout.list_viewport.y as u32
                        && y < (layout.list_viewport.y + layout.list_viewport.h) as u32;
                    if inside_viewport {
                        viewport_differences += usize::from(differs);
                    }
                } else {
                    assert!(!differs, "scrolled list leaked to pixel ({x},{y})");
                }
            }
        }
        assert!(inside_differences > 0);
        assert!(
            viewport_differences > 0,
            "rows did not move inside viewport"
        );
    }

    #[test]
    fn inline_crew_rename_intersects_outer_clip_and_hides_caret_without_draw_focus() {
        use clonk_graphics::PixelFormat;

        let assets = PlrSelAssets {
            background: crate::test_support::load_graphics_png("StartupPlrSelBG.png"),
            checkbox: crate::test_support::load_graphics_png("GUICheckbox.png"),
            button: crate::test_support::load_graphics_png("GUIButton.png"),
            button_down: crate::test_support::load_graphics_png("GUIButtonDown.png"),
            button_highlight: crate::test_support::load_graphics_png("GUIButtonHighlight.png"),
            book_scroll: crate::test_support::load_graphics_png("StartupBookScroll.png"),
            player: crate::test_support::load_graphics_png("Player.png"),
        };
        let fonts = crate::test_support::endeavour_font_set();
        let book = book_fonts();
        let gamma = crate::test_support::standard_gamma();
        let crew = [PlrSelCrew {
            name: "Alpha".to_string(),
            participating: true,
            rank_icon: None,
            portrait: None,
            color_dw: 0xff,
            rank: 0,
            rank_name: "Clonk".to_string(),
            type_name: "Clonk".to_string(),
            experience: 0,
            rounds: 0,
            death_count: 0,
            total_playing_time: 0,
            birthday: String::new(),
            next_rank: None,
            physical: PhysicalInfo::default(),
        }];
        let mut controller = PlrSelController::new(1);
        controller.resize(1280, 720);
        assert!(controller.enter_crew_mode(0, "Ada", vec![true]));

        let render = |draw_focus| {
            let mut surface = Surface::new(1280, 720, PixelFormat::Rgba8888);
            let mut edit = RenameEdit::new("Alpha", Some(PlrSelControl::PlayerList));
            PlrSelScreen::render_controller_with_crew_rename_and_draw_focus(
                &mut surface,
                &assets,
                &fonts,
                &book,
                &[],
                &crew,
                &controller,
                Some((0, &mut edit)),
                draw_focus,
                Some(gamma),
            );
            surface
        };
        assert_ne!(render(true).pixels(), render(false).pixels());

        let untouched = Color::new(17, 31, 47, 255);
        let mut clipped = Surface::new(1280, 720, PixelFormat::Rgba8888);
        clipped.fill(untouched);
        clipped.set_clip(clonk_graphics::Rect::new(0, 0, 1, 1));
        let mut edit = RenameEdit::new("Alpha", Some(PlrSelControl::PlayerList));
        PlrSelScreen::render_controller_with_crew_rename_and_draw_focus(
            &mut clipped,
            &assets,
            &fonts,
            &book,
            &[],
            &crew,
            &controller,
            Some((0, &mut edit)),
            true,
            Some(gamma),
        );
        for y in 0..clipped.height() {
            for x in 0..clipped.width() {
                if x != 0 || y != 0 {
                    assert_eq!(clipped.get_pixel(x, y), Some(untouched));
                }
            }
        }
    }

    #[test]
    fn keyboard_right_is_crew_but_gamepad_horizontal_only_moves_focus() {
        let mut controller = PlrSelController::new(2);
        controller.resize(1280, 720);
        assert_eq!(
            controller.handle_key_down(crate::KeyCode::Tab),
            vec![PlrSelAction::FocusChanged(PlrSelControl::Back)]
        );
        assert_eq!(
            controller.handle_key_down(crate::KeyCode::Right),
            vec![PlrSelAction::ShowCrew(0)],
            "the dialog-level keyboard Crew binding is independent of bottom-button focus"
        );
        assert_eq!(
            controller.handle_gamepad_horizontal(true),
            vec![PlrSelAction::FocusChanged(PlrSelControl::PlayerList)]
        );
        assert_eq!(
            controller.handle_gamepad_horizontal(false),
            vec![PlrSelAction::FocusChanged(PlrSelControl::Back)]
        );
    }

    #[test]
    fn shift_tab_reverses_player_and_crew_focus_order() {
        let mut controller = PlrSelController::new(1);
        assert_eq!(
            controller.handle_key_down_with_tab_direction(crate::KeyCode::Tab, true),
            vec![PlrSelAction::FocusChanged(PlrSelControl::Crew)]
        );
        assert_eq!(
            controller.handle_key_down(crate::KeyCode::Tab),
            vec![PlrSelAction::FocusChanged(PlrSelControl::PlayerList)]
        );

        assert!(controller.enter_crew_mode(0, "Ada", vec![true]));
        assert_eq!(controller.focused_control(), PlrSelControl::PlayerList);
        assert_eq!(
            controller.handle_key_down_with_tab_direction(crate::KeyCode::Tab, true),
            vec![PlrSelAction::FocusChanged(PlrSelControl::Rename)]
        );
        assert_eq!(
            controller.handle_key_down(crate::KeyCode::Tab),
            vec![PlrSelAction::FocusChanged(PlrSelControl::PlayerList)]
        );
    }

    // Mouse routing follows the nested C4GUI controls: bottom buttons retain
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
        assert!(controller.handle_pointer_down(back).is_empty());
        assert_eq!(controller.focused_control(), PlrSelControl::PlayerList);
        assert_eq!(controller.handle_pointer_up(back), vec![PlrSelAction::Back]);

        let second_checkbox = crate::GuiPoint::new(
            (layout.list_client.x + layout.item_height / 2) as f32,
            (layout.list_client.y + layout.item_pitch + layout.item_height / 2) as f32,
        );
        assert_eq!(
            controller.handle_pointer_down(second_checkbox),
            vec![PlrSelAction::SelectionChanged(Some(1))]
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

    #[test]
    fn l061_list_and_checkbox_sounds_follow_the_user_input_source() {
        let layout = plrsel_layout(1280, 720);
        let mut controller = PlrSelController::new(2);
        controller.resize(1280, 720);

        assert_eq!(
            controller.handle_key_down(crate::KeyCode::Down),
            vec![PlrSelAction::SelectionChanged(Some(1))]
        );
        assert_eq!(controller.take_sound_events(), [PlrSelSound::Command]);
        assert!(controller.handle_key_down(crate::KeyCode::Down).is_empty());
        assert!(controller.take_sound_events().is_empty());

        let first_checkbox = crate::GuiPoint::new(
            (layout.list_client.x + layout.item_height / 2) as f32,
            (layout.list_client.y + layout.item_height / 2) as f32,
        );
        assert_eq!(
            controller.handle_pointer_down(first_checkbox),
            vec![PlrSelAction::SelectionChanged(Some(0))]
        );
        assert_eq!(controller.take_sound_events(), [PlrSelSound::Command]);
        assert_eq!(
            controller.handle_pointer_up(first_checkbox),
            vec![PlrSelAction::ActivationChanged {
                index: 0,
                activated: true,
            }]
        );
        assert_eq!(controller.take_sound_events(), [PlrSelSound::ArrowHit]);

        assert_eq!(
            controller.handle_key_down(crate::KeyCode::Space),
            vec![PlrSelAction::ActivationChanged {
                index: 0,
                activated: false,
            }]
        );
        assert_eq!(controller.take_sound_events(), [PlrSelSound::ArrowHit]);

        let second_row_name = crate::GuiPoint::new(
            (layout.list_client.x + layout.item_height * 3) as f32,
            (layout.list_client.y + layout.item_pitch + layout.item_height / 2) as f32,
        );
        assert_eq!(
            controller.handle_pointer_double_click(second_row_name),
            vec![
                PlrSelAction::SelectionChanged(Some(1)),
                PlrSelAction::PlayerProperties(1),
            ]
        );
        assert_eq!(
            controller.take_sound_events(),
            [PlrSelSound::Command, PlrSelSound::Click]
        );
        assert_eq!(
            controller.handle_key_down(crate::KeyCode::Enter),
            vec![PlrSelAction::PlayerProperties(1)]
        );
        assert_eq!(controller.take_sound_events(), [PlrSelSound::Click]);
        assert_eq!(
            controller.handle_edit_shortcut(),
            vec![PlrSelAction::PlayerProperties(1)]
        );
        assert!(controller.take_sound_events().is_empty());
    }

    #[test]
    fn l061_button_sounds_follow_down_cancel_reentry_and_keyboard_paths() {
        let layout = plrsel_layout(1280, 720);
        let back = center(layout.buttons[0]);
        let outside = crate::GuiPoint::new(0.0, 0.0);
        let mut controller = PlrSelController::new(1);
        controller.resize(1280, 720);

        assert!(controller.handle_pointer_down(back).is_empty());
        assert_eq!(controller.take_sound_events(), [PlrSelSound::ArrowHit]);
        assert!(controller.handle_pointer_down(back).is_empty());
        assert!(controller.take_sound_events().is_empty());
        assert!(controller.handle_pointer_up(outside).is_empty());
        assert_eq!(controller.take_sound_events(), [PlrSelSound::ArrowHit]);

        assert!(controller.handle_pointer_down(back).is_empty());
        assert_eq!(controller.take_sound_events(), [PlrSelSound::ArrowHit]);
        assert!(controller.handle_pointer_move(outside).is_empty());
        assert_eq!(controller.take_sound_events(), [PlrSelSound::ArrowHit]);
        assert!(controller.handle_pointer_move(back).is_empty());
        assert_eq!(controller.take_sound_events(), [PlrSelSound::ArrowHit]);
        assert_eq!(controller.handle_pointer_up(back), vec![PlrSelAction::Back]);
        assert_eq!(controller.take_sound_events(), [PlrSelSound::Click]);

        assert_eq!(
            controller.handle_key_down(crate::KeyCode::Tab),
            vec![PlrSelAction::FocusChanged(PlrSelControl::Back)]
        );
        assert!(controller.take_sound_events().is_empty());
        assert!(controller.handle_key_down(crate::KeyCode::Enter).is_empty());
        assert_eq!(controller.take_sound_events(), [PlrSelSound::ArrowHit]);
        assert!(controller.handle_key_down(crate::KeyCode::Enter).is_empty());
        assert!(controller.take_sound_events().is_empty());
        assert_eq!(
            controller.handle_key_up(crate::KeyCode::Enter),
            vec![PlrSelAction::Back]
        );
        assert_eq!(controller.take_sound_events(), [PlrSelSound::Click]);

        let activate = center(layout.buttons[2]);
        assert!(controller.handle_pointer_down(activate).is_empty());
        assert_eq!(controller.take_sound_events(), [PlrSelSound::ArrowHit]);
        assert_eq!(
            controller.handle_pointer_up(activate),
            vec![PlrSelAction::ActivationChanged {
                index: 0,
                activated: true,
            }]
        );
        assert_eq!(controller.take_sound_events(), [PlrSelSound::Click]);
    }

    #[test]
    fn bottom_button_pointer_down_retains_the_previous_focus() {
        let layout = plrsel_layout(1280, 720);
        for button in layout.buttons {
            let mut controller = PlrSelController::new(2);
            controller.resize(1280, 720);
            assert_eq!(
                controller.handle_key_down(crate::KeyCode::Tab),
                vec![PlrSelAction::FocusChanged(PlrSelControl::Back)]
            );

            assert!(controller.handle_pointer_down(center(button)).is_empty());
            assert_eq!(controller.focused_control(), PlrSelControl::Back);
        }
    }

    // Live rendering must consume the controller that receives input: list
    // selection/focus, activation flags and C4GUI::Button interaction cannot
    // remain frozen at the first-shown frame (C4GuiListBox.cpp:100-124;
    // C4GuiButton.cpp:81-110; C4StartupPlrSelDlg.cpp:772-802,840-849).
    #[test]
    fn live_renderer_reflects_player_controller_state() {
        use clonk_graphics::PixelFormat;
        let assets = PlrSelAssets {
            background: crate::test_support::load_graphics_png("StartupPlrSelBG.png"),
            checkbox: crate::test_support::load_graphics_png("GUICheckbox.png"),
            button: crate::test_support::load_graphics_png("GUIButton.png"),
            button_down: crate::test_support::load_graphics_png("GUIButtonDown.png"),
            button_highlight: crate::test_support::load_graphics_png("GUIButtonHighlight.png"),
            book_scroll: crate::test_support::load_graphics_png("StartupBookScroll.png"),
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
        let render_without_draw_focus = |controller: &PlrSelController| {
            let mut surface = Surface::new(1280, 720, PixelFormat::Rgba8888);
            PlrSelScreen::render_controller_with_draw_focus(
                &mut surface,
                &assets,
                &fonts,
                &book,
                &players,
                controller,
                false,
                Some(gamma),
            );
            surface
        };

        let first = render(&controller);
        let list_focus_suppressed = render_without_draw_focus(&controller);
        assert_ne!(first.pixels(), list_focus_suppressed.pixels());
        controller.handle_key_down(crate::KeyCode::Down);
        let selected_second = render(&controller);
        assert_ne!(first.pixels(), selected_second.pixels());

        controller.handle_key_down(crate::KeyCode::Space);
        let activated = render(&controller);
        assert_ne!(selected_second.pixels(), activated.pixels());

        controller.handle_key_down(crate::KeyCode::Tab);
        let button_focused = render(&controller);
        assert_ne!(activated.pixels(), button_focused.pixels());
        let button_focus_suppressed = render_without_draw_focus(&controller);
        assert_ne!(button_focused.pixels(), button_focus_suppressed.pixels());

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
        assert_eq!(book.caption.role(), Some(ClonkFontRole::BookCaption));
        assert_eq!(book.text.role(), Some(ClonkFontRole::BookText));
    }

    #[test]
    fn transparent_player_selection_box_keeps_layer_alpha() {
        use clonk_graphics::PixelFormat;
        let mut surface = Surface::new(1, 1, PixelFormat::Rgba8888);
        draw_box_dw(&mut surface, 0, 0, 0, 0, CLR_LIST_BOX_SEL, None);
        assert_eq!(surface.get_pixel(0, 0).map(|pixel| pixel.a), Some(80));
    }

    // DrawBoxDw → DrawQuadDw (StdGL.cpp:846-894): inverted-alpha color,
    // gamma-encoded rgb, blended src*(255-A)/255 + dst*A/255, inclusive x2/y2.
    #[test]
    fn draw_box_dw_blends_with_inverted_alpha_and_gamma() {
        use clonk_graphics::PixelFormat;
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
        let image = ImageData::new(2, 1, vec![255, 255, 255, 0, 200, 100, 50, 1]);
        let out = engine_png_texture(&image);
        assert_eq!(out.pixels(), &[0, 0, 0, 0, 200, 100, 50, 1]);
        assert_eq!(
            out.gpu_texture_id(),
            engine_png_texture(&image).gpu_texture_id()
        );
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
        let (base_again, overlay_again) = split_color_by_owner(&image);
        assert_eq!(base.gpu_texture_id(), base_again.gpu_texture_id());
        assert_eq!(overlay.gpu_texture_id(), overlay_again.gpu_texture_id());
    }

    #[test]
    fn extracted_checkbox_region_reuses_texture_identity() {
        let image = ImageData::new(4, 2, [10, 20, 30, 255].repeat(8));
        let first = extract_region(&image, 1, 0, 2, 2);
        let second = extract_region(&image, 1, 0, 2, 2);
        assert_eq!(first.gpu_texture_id(), second.gpu_texture_id());
    }

    #[test]
    fn time_string_is_zero_padded_hms() {
        assert_eq!(time_string(0), "00:00:00");
        assert_eq!(time_string(3600 + 23 * 60 + 5), "01:23:05");
    }

    #[test]
    fn player_delete_warning_uses_strict_ten_hour_boundary() {
        let mut player = tyler();
        player.name = "Ada".into();
        player.total_playing_time = 36_000;
        assert_eq!(
            player_delete_warning(&player),
            "Do you really want to delete player Ada?"
        );

        player.total_playing_time = 36_001;
        assert_eq!(
            player_delete_warning(&player),
            "Do you really want to delete player Ada? - this player has a total playing time of 10:00:01!"
        );
    }

    // C4Facet::Draw aspect math (C4Facet.cpp:106-117).
    #[test]
    fn aspect_fit_centers_square_source_in_wide_box() {
        // 150x150 source in (806,196,200,150) → (831,196,150,150) (spec §6).
        let out = aspect_fit(
            150,
            150,
            IntRect {
                x: 806,
                y: 196,
                w: 200,
                h: 150,
            },
        );
        assert_eq!((out.x, out.y, out.w, out.h), (831, 196, 150, 150));
        // 64x64 into 26x26: ratios equal → unchanged.
        let out = aspect_fit(
            64,
            64,
            IntRect {
                x: 179,
                y: 260,
                w: 26,
                h: 26,
            },
        );
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
        use clonk_graphics::PixelFormat;
        let assets = PlrSelAssets {
            background: crate::test_support::load_graphics_png("StartupPlrSelBG.png"),
            checkbox: crate::test_support::load_graphics_png("GUICheckbox.png"),
            button: crate::test_support::load_graphics_png("GUIButton.png"),
            button_down: crate::test_support::load_graphics_png("GUIButtonDown.png"),
            button_highlight: crate::test_support::load_graphics_png("GUIButtonHighlight.png"),
            book_scroll: crate::test_support::load_graphics_png("StartupBookScroll.png"),
            player: crate::test_support::load_graphics_png("Player.png"),
        };
        let fonts = crate::test_support::endeavour_font_set();
        let book = book_fonts();
        let gamma = crate::test_support::standard_gamma();
        let players = [tyler()];

        let mut surface = Surface::new(1280, 720, PixelFormat::Rgba8888);
        PlrSelScreen::render(
            &mut surface,
            &assets,
            &fonts,
            &book,
            &players,
            Some(0),
            Some(gamma),
        );
        // The app's final whole-surface gamma pass (identity except 0 → 1).
        gamma.apply_to_surface(&mut surface);

        std::fs::create_dir_all("/tmp/menu-parity-plrsel").expect("artifact dir");
        crate::test_support::write_ppm(&surface, "/tmp/menu-parity-plrsel/out.ppm");
    }
}
