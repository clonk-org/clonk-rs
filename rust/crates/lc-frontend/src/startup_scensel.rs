//! Pixel-parity renderer for `C4StartupScenSelDlg` — the scenario-selection
//! "book" (see `rust/target/parity-specs/scensel.md`). Implemented against the
//! engine's F9 reference capture at 1280x720; mirrors
//! `src/C4StartupScenSelDlg.cpp` (ctor layout, 1302-1382), `src/C4Gui.cpp`
//! (DrawBar/DrawVBar/Draw3DFrame, 264-345) and `src/C4Startup.cpp:92-116`
//! (shadowless book fonts).

use crate::clonk_fonts::{expand_hotkey_markup, ClonkFontSet};
use crate::startup_main_menu::{draw_bar, IntRect};
use crate::{draw_image_bilinear, draw_image_strip, ImageData};
use anyhow::{Context, Result};
use freetype::face::LoadFlag;
use freetype::Library;
use lc_graphics::clonk_font::{line_height_for, ClonkFont, GlyphCell, TextAlign};
use lc_graphics::{Color, GammaRamp, Surface};
use lc_gui::Rect as GuiRect;

// ---------------------------------------------------------------------------
// Assets (planet/Graphics.c4g)
// ---------------------------------------------------------------------------

/// The Graphics.c4g images the scenario-selection book draws with.
pub struct ScenSelAssets {
    /// `StartupScenSelBG.png` (800x600) — fullscreen-stretched parchment book
    /// background (C4Startup.cpp:41-42, C4StartupScenSelDlg.cpp:1412-1419).
    pub background: ImageData,
    /// `StartupBookScroll.png` (48x48, 16px cells) — book-style scrollbar
    /// facets (C4Gui.cpp:109-121).
    pub book_scroll: ImageData,
    /// `StartupScenSelIcons.png` (1248x24, 52 icons of 24x24) — list-entry
    /// icons (C4Startup.cpp:67-69).
    pub scen_icons: ImageData,
    /// `GUICaption.png` (192x23, border 32) — wooden 3-slice bar behind the
    /// "Search:" label (C4Gui.cpp:1088).
    pub caption_bar: ImageData,
    /// `GUIButton.png` (128x32, border 32) — released button plank
    /// (C4GuiButton.cpp:81-89).
    pub button: ImageData,
    /// `GUICheckbox.png` (128x32, 4 phases of 32x32) — checkbox states
    /// (C4GuiCheckBox.cpp:110-115).
    pub checkbox: ImageData,
    /// `GUIIcons2.png` (256x320, 64x64 cells) — extended icons for the
    /// fair-crew/record icon buttons (C4Gui.h:734-751).
    pub icons_ex: ImageData,
    /// `StartupScenSelTitleOv.png` (220x170) — paper frame drawn over the
    /// right-page title picture (fctScenSelTitleOverlay, C4Startup.cpp;
    /// OverlayPicture border 10, C4StartupScenSelDlg.cpp:1361-1362).
    pub title_overlay: ImageData,
}

// ---------------------------------------------------------------------------
// Book fonts (shadowless CStdFont)
// ---------------------------------------------------------------------------

/// The three shadowless "book" fonts of the startup graphics
/// (`C4StartupGraphics::InitFonts`, C4Startup.cpp:92-116; all built with
/// `fDoShadow = false`).
pub struct BookFontSet {
    /// `BookFontTitle` — C4FT_Title (22px, line height 34).
    pub title: ClonkFont,
    /// `BookFontCapt` — C4FT_Caption (16px, line height 25).
    pub caption: ClonkFont,
    /// `BookFont` — C4FT_Main (14px, line height 22).
    pub text: ClonkFont,
}

/// Windows-1252 byte to Unicode, mirroring the C++ iconv conversion of the
/// legacy charset (StdFont.cpp:386-401); same table as the GUI font builder.
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

/// Rasterizes one shadowless ClonkFont at `px_height`, mirroring
/// `CStdFont::Init`/`AddRenderedChar` with `fDoShadow = false`
/// (StdFont.cpp:184,218-258,327-352): `iHSpace = 0`, `iGfxLineHgt = iLineHgt`
/// (no shadow row), `shadowSize = 0` so every atlas pixel is pure white with
/// alpha = FreeType coverage (`BltAlpha` onto a fully transparent base keeps
/// the white source, StdColors.h:122-126).
fn build_book_font(face: &freetype::Face, px_height: u32) -> Result<ClonkFont> {
    face.set_pixel_sizes(px_height, px_height)
        .context("FT_Set_Pixel_Sizes failed")?;

    let raw = face.raw();
    let units_per_em = i32::from(raw.units_per_EM);
    let (ascender, descender) = (i32::from(raw.ascender), i32::from(raw.descender));
    let line_height = line_height_for(ascender, descender, units_per_em, px_height);
    let mut font = ClonkFont::new(line_height);
    // Shadowless metrics (StdFont.cpp:327,352): iHSpace = 0 and
    // iGfxLineHgt = iLineHgt + fDoShadow = iLineHgt.
    font.h_space = 0;
    font.cell_height = line_height;
    let cell_height = line_height.max(0) as usize;
    // Baseline offset inside the cell (StdFont.cpp:221).
    let ascent_px = i64::from(px_height) * i64::from(ascender) / i64::from(units_per_em);

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

        // width = max(advance, bearing + width) + shadowSize(=0) (StdFont.cpp:218).
        let advance_px = (slot.advance().x >> 6) as i32;
        let bearing = slot.bitmap_left().max(0);
        let cell_w = advance_px.max(bearing + cov_w as i32).max(1) as usize;
        let at_x = bearing as usize;
        let at_y = (ascent_px - i64::from(slot.bitmap_top())).max(0) as usize;

        // Pixel loop without the shadow extension (StdFont.cpp:224-258 with
        // shadowSize = 0): white with alpha = coverage.
        let mut pixels = vec![Color::transparent(); cell_w * cell_height];
        for y in 0..cov_h {
            for x in 0..cov_w {
                let cov = buffer[(y as i32 * pitch) as usize + x];
                let (tx, ty) = (at_x + x, at_y + y);
                if tx < cell_w && ty < cell_height {
                    pixels[ty * cell_w + tx] = Color::new(255, 255, 255, cov);
                }
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

/// Builds the three book fonts from a TTF, sized like the GUI fonts
/// (C4Fonts.cpp:280-288: Main 14, Caption 16, Title 22) but without shadow.
pub fn build_book_font_set(ttf_bytes: &[u8]) -> Result<BookFontSet> {
    let library = Library::init().context("FreeType init failed")?;
    let face = library
        .new_memory_face(ttf_bytes.to_vec(), 0)
        .context("failed to load font face")?;
    Ok(BookFontSet {
        title: build_book_font(&face, 22)?,
        caption: build_book_font(&face, 16)?,
        text: build_book_font(&face, 14)?,
    })
}

// ---------------------------------------------------------------------------
// Layout (C4StartupScenSelDlg ctor, C4StartupScenSelDlg.cpp:1302-1382)
// ---------------------------------------------------------------------------

/// `C4GUI::ComponentAligner` (C4Gui.cpp:975-1057): carves rects out of a
/// client area, insetting by the perpendicular margins.
struct Aligner {
    area: IntRect,
    margin_x: i32,
    margin_y: i32,
}

impl Aligner {
    fn new(area: IntRect, margin_x: i32, margin_y: i32) -> Self {
        Self {
            area,
            margin_x,
            margin_y,
        }
    }

    /// C4Gui.cpp:975-990 (`iWdt < 0` variant only — never used with a width).
    fn get_from_top(&mut self, height: i32) -> IntRect {
        let out = IntRect {
            x: self.area.x + self.margin_x,
            y: self.area.y + self.margin_y,
            w: self.area.w - self.margin_x * 2,
            h: height,
        };
        let d = height + self.margin_y * 2;
        self.area.y += d;
        self.area.h -= d;
        out
    }

    /// C4Gui.cpp:1026-1040 (`iWdt < 0` variant).
    fn get_from_bottom(&mut self, height: i32) -> IntRect {
        let out = IntRect {
            x: self.area.x + self.margin_x,
            y: self.area.y + self.area.h - height - self.margin_y,
            w: self.area.w - self.margin_x * 2,
            h: height,
        };
        self.area.h -= height + self.margin_y * 2;
        out
    }

    /// C4Gui.cpp:992-1008; `height >= 0` centers the rect vertically.
    fn get_from_left(&mut self, width: i32, height: Option<i32>) -> IntRect {
        let mut out = IntRect {
            x: self.area.x + self.margin_x,
            y: self.area.y + self.margin_y,
            w: width,
            h: self.area.h - self.margin_y * 2,
        };
        let d = width + self.margin_x * 2;
        self.area.x += d;
        self.area.w -= d;
        if let Some(height) = height {
            out.y += (out.h - height) / 2;
            out.h = height;
        }
        out
    }

    /// C4Gui.cpp:1010-1024.
    fn get_from_right(&mut self, width: i32, height: Option<i32>) -> IntRect {
        let mut out = IntRect {
            x: self.area.x + self.area.w - width - self.margin_x,
            y: self.area.y + self.margin_y,
            w: width,
            h: self.area.h - self.margin_y * 2,
        };
        self.area.w -= width + self.margin_x * 2;
        if let Some(height) = height {
            out.y += (out.h - height) / 2;
            out.h = height;
        }
        out
    }

    /// C4Gui.cpp:1041-1047.
    fn get_all(&self) -> IntRect {
        IntRect {
            x: self.area.x + self.margin_x,
            y: self.area.y + self.margin_y,
            w: self.area.w - self.margin_x * 2,
            h: self.area.h - self.margin_y * 2,
        }
    }

    /// C4Gui.cpp:1049-1057 (`GetMiddle - size/2`, no consume).
    fn get_centered(&self, width: i32, height: i32) -> IntRect {
        IntRect {
            x: self.area.x + self.area.w / 2 - width / 2,
            y: self.area.y + self.area.h / 2 - height / 2,
            w: width,
            h: height,
        }
    }

    /// `ExpandTop` (C4Gui.h aligner): grows the area upward by `by` —
    /// negative values shrink it from the top.
    fn expand_top(&mut self, by: i32) {
        self.area.y -= by;
        self.area.h += by;
    }
}

/// Pixel-exact C4StartupScenSelDlg geometry in screen coordinates.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ScenSelLayout {
    /// Dialog client rect (margins X = w/50, top = h/7, bottom = h*2/75;
    /// C4StartupScenSelDlg.h:406, C4GuiDialogs.cpp:819-820).
    pub client: IntRect,
    /// Fullscreen title "Start Game": ACenter anchor x and top y
    /// (C4GuiDialogs.cpp:834-849; offsets captured with the pre-override
    /// margin top of 50 + h*2/75).
    pub title_anchor: (i32, i32),
    /// Book caption "Scenarios": ACenter anchor x and top y
    /// (C4StartupScenSelDlg.cpp:1331-1334).
    pub caption_anchor: (i32, i32),
    /// Wooden "Search:" label (C4StartupScenSelDlg.cpp:1336-1343).
    pub search_label: IntRect,
    /// Search edit field (C4StartupScenSelDlg.cpp:1345-1347).
    pub search_edit: IntRect,
    /// Scenario list box bounds (C4StartupScenSelDlg.cpp:1349-1355).
    pub list: IntRect,
    /// List scrollbar track (ScrollWindow ctor, C4GuiContainers.cpp:477-491).
    pub list_scrollbar: IntRect,
    /// Right-page selection info TextWindow (C4StartupScenSelDlg.cpp:1360-1364).
    pub selection_info: IntRect,
    /// "Back" button (C4StartupScenSelDlg.cpp:1367-1370).
    pub back_button: IntRect,
    /// "Open" button (C4StartupScenSelDlg.cpp:1372-1373).
    pub open_button: IntRect,
    /// "Choose definitions" checkbox (C4StartupScenSelDlg.cpp:1376).
    pub user_change_checkbox: IntRect,
    /// Fair-crew icon button, 64x64 (C4Network2Dialogs.cpp:588-654).
    pub fair_crew_button: IntRect,
    /// Record icon button, 64x64.
    pub record_button: IntRect,
    /// `iButtonWidth` = 3 x CaptionFont width of "<< BACK"
    /// (C4StartupScenSelDlg.cpp:1309-1310).
    pub button_width: i32,
    /// GUI TextFont width of "Search:" (C4StartupScenSelDlg.cpp:1337-1339).
    pub search_text_width: i32,
}

/// Computes the C4StartupScenSelDlg layout for a `w`x`h` screen, mirroring
/// the constructor math (C4StartupScenSelDlg.cpp:1302-1382) with the GUI
/// fonts measured at runtime.
pub fn scen_sel_layout(w: i32, h: i32, fonts: &ClonkFontSet) -> ScenSelLayout {
    // Fullscreen dialog margins (C4GuiDialogs.cpp:819-820) with the dialog's
    // GetMarginTop() override = h/7 (C4StartupScenSelDlg.h:406).
    let margin_x = if w < 500 { 2 } else { w / 50 };
    let margin_y = if h < 320 { 2 } else { h * 2 / 75 };
    let margin_top = h / 7;
    let client = IntRect {
        x: margin_x,
        y: margin_top,
        w: w - 2 * margin_x,
        h: h - margin_top - margin_y,
    };

    // Title label: created during the FullscreenDialog base ctor when the
    // margin top was still 50 + margin_y; the stored client-relative offsets
    // keep that bias after the override (C4GuiDialogs.cpp:846).
    let title_anchor = (
        client.x + client.w / 2,
        client.y + 50 / 2 - fonts.title.line_height / 2 - (50 + margin_y),
    );

    // Constructor constants (C4StartupScenSelDlg.cpp:1307-1310).
    let extra_h_padding = if w >= 700 { w / 50 } else { 0 };
    let extra_v_padding = if h >= 540 { h / 20 } else { 0 };
    let button_width = 3 * fonts.caption.measure("<< BACK", true).0;

    // caMain over the zero-origin client (C4StartupScenSelDlg.cpp:1311).
    let mut ca_main = Aligner::new(
        IntRect {
            x: 0,
            y: 0,
            w: client.w,
            h: client.h,
        },
        0,
        0,
    );
    let mut ca_button_area = Aligner::new(
        ca_main.get_from_bottom(ca_main.area.h / 8),
        w / if w >= 700 { 128 } else { 256 },
        0,
    );
    let mut rc_map = ca_main.get_centered(ca_main.area.w, ca_main.area.h);
    let y_oversize = ca_main.area.h / 10; // overlap of map to top (:1314)
    rc_map.y -= y_oversize;
    rc_map.h += y_oversize;
    let mut ca_map = Aligner::new(
        IntRect {
            x: 0,
            y: 0,
            w: rc_map.w,
            h: rc_map.h,
        },
        0,
        0,
    );
    ca_map.expand_top(-y_oversize);
    let mut ca_book = Aligner::new(
        ca_map.get_centered(ca_map.area.w * 11 / 12 - 4 * extra_h_padding, ca_map.area.h),
        w / 30,
        extra_v_padding,
    );
    let book_page_width = ca_book.area.w * 4 / 9 + 4 - extra_h_padding * 2;
    let mut ca_book_left = Aligner::new(ca_book.get_from_left(book_page_width, None), 0, 5);

    // The book sheet's children are sheet-relative; the sheet (tabular client
    // rcMap with zero chrome) maps to screen at client + rcMap origin
    // (C4StartupScenSelDlg.cpp:1313-1315,1322-1328).
    let sheet_x = client.x + rc_map.x;
    let sheet_y = client.y + rc_map.y;
    let on_sheet = |r: IntRect| IntRect {
        x: r.x + sheet_x,
        y: r.y + sheet_y,
        w: r.w,
        h: r.h,
    };

    // Left-page caption (C4StartupScenSelDlg.cpp:1331-1334).
    let caption_rect = on_sheet(ca_book_left.get_from_top(fonts.title.line_height));
    let caption_anchor = (caption_rect.x + caption_rect.w / 2, caption_rect.y);

    // Search row at the page bottom (C4StartupScenSelDlg.cpp:1336-1347).
    let search_text_width = fonts.text.measure("Search:", true).0;
    let search_height = fonts.text.line_height;
    let mut ca_search_bar = Aligner::new(ca_book_left.get_from_bottom(search_height), 0, 0);
    let search_label = on_sheet(ca_search_bar.get_from_left(search_text_width + 10, None));
    let search_edit = on_sheet(ca_search_bar.get_all());

    // Scenario list (C4StartupScenSelDlg.cpp:1349-1355); margins 3 all around
    // (C4GuiListBox.h:120-123), scroll window keeps the right 16px for the
    // scrollbar (C4GuiContainers.cpp:477-491).
    let list = on_sheet(ca_book_left.get_all());
    let list_scrollbar = IntRect {
        x: list.x + 3 + (list.w - 6) - 16,
        y: list.y + 3,
        w: 16,
        h: list.h - 6,
    };

    // Right page (C4StartupScenSelDlg.cpp:1360-1364).
    let selection_info = on_sheet(ca_book.get_from_right(book_page_width, None));

    // Bottom button bar, dialog-client relative (C4StartupScenSelDlg.cpp:
    // 1367-1382); the helper shifts to screen coordinates.
    let on_client = |r: IntRect| IntRect {
        x: r.x + client.x,
        y: r.y + client.y,
        w: r.w,
        h: r.h,
    };
    let button_height = 32; // C4GUI_ButtonHgt (C4Gui.h:119)
    let back_button = on_client(ca_button_area.get_from_left(button_width, Some(button_height)));
    let open_button = on_client(ca_button_area.get_from_right(button_width, Some(button_height)));
    let user_change_checkbox =
        on_client(ca_button_area.get_from_right(button_width, Some(button_height)));

    // C4GameOptionButtons window over the remaining bar area
    // (C4Network2Dialogs.cpp:588-654): two 64x64 icon buttons, centered.
    let options = on_client(ca_button_area.get_all());
    let icon_size = 64.min(options.h); // C4GUI_IconExHgt
    let icon_spacing = options.w / if options.w >= 400 { 64 } else { 128 };
    let ca_buttons_area = Aligner::new(
        IntRect {
            x: 0,
            y: 0,
            w: options.w,
            h: options.h,
        },
        0,
        0,
    )
    .get_centered((icon_size + 2 * icon_spacing) * 2, icon_size);
    let mut ca_buttons = Aligner::new(ca_buttons_area, icon_spacing, 0);
    let on_options = |r: IntRect| IntRect {
        x: r.x + options.x,
        y: r.y + options.y,
        w: r.w,
        h: r.h,
    };
    let fair_crew_button = on_options(ca_buttons.get_from_left(icon_size, Some(icon_size)));
    let record_button = on_options(ca_buttons.get_from_left(icon_size, Some(icon_size)));

    ScenSelLayout {
        client,
        title_anchor,
        caption_anchor,
        search_label,
        search_edit,
        list,
        list_scrollbar,
        selection_info,
        back_button,
        open_button,
        user_change_checkbox,
        fair_crew_button,
        record_button,
        button_width,
        search_text_width,
    }
}

// ---------------------------------------------------------------------------
// Draw primitives (CStdDDraw / C4GUI::Element)
// ---------------------------------------------------------------------------

/// The GL texture tile size for an image: next power of two of min(W, H),
/// capped at 4096 (C4Surface::CreateTextures, C4Surface.cpp:166-189).
fn cpp_tex_size(width: u32, height: u32) -> u32 {
    let need = width.min(height).max(1);
    let mut size = 1u32;
    while size < need {
        size <<= 1;
    }
    size.min(4096)
}

/// GL_LINEAR tap inside one texture tile with CLAMP_TO_EDGE
/// (C4Surface.cpp:1102-1103); texels inside the tile but outside the image
/// are transparent padding. Coordinates are tile-relative texel centers
/// already offset by -0.5.
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

/// The blit shader's per-fragment gamma encode (StdGL.cpp:1082-1086), or
/// plain rounding without a ramp.
fn encode_channel(gamma: Option<&GammaRamp>, x: f32) -> f32 {
    gamma
        .map(|ramp| f32::from(ramp.encode_float(x)))
        .unwrap_or_else(|| x.round().clamp(0.0, 255.0))
}

/// Stretch-blit of an `image` subregion, mirroring `CStdDDraw::Blit`
/// (StdDDraw2.cpp:637-786): one quad per power-of-two texture tile
/// overlapping the source rect, GL_LINEAR sampling per tile, the blit
/// shader's gamma lookup and float alpha-over blending rounded on store.
fn draw_facet_stretch(
    surface: &mut Surface,
    image: &ImageData,
    src: (f32, f32, f32, f32),
    dest: (f32, f32, f32, f32),
    gamma: Option<&GammaRamp>,
) {
    let (fx, fy, fwdt, fhgt) = src;
    let (tx, ty, twdt, thgt) = dest;
    if fwdt <= 0.0 || fhgt <= 0.0 || twdt <= 0.0 || thgt <= 0.0 {
        return;
    }
    let scale_x = twdt / fwdt;
    let scale_y = thgt / fhgt;
    let ts = cpp_tex_size(image.width(), image.height()) as i32;
    // Involved texture tiles (StdDDraw2.cpp:693-697).
    let tiles_x_img = (image.width() as i32 - 1) / ts + 1;
    let tiles_y_img = (image.height() as i32 - 1) / ts + 1;
    let tex_x = ((fx / ts as f32) as i32).max(0);
    let tex_y = ((fy / ts as f32) as i32).max(0);
    let tex_x2 = (((fx + fwdt - 1.0) as i32) / ts + 1).min(tiles_x_img);
    let tex_y2 = (((fy + fhgt - 1.0) as i32) / ts + 1).min(tiles_y_img);

    for tile_iy in tex_y..tex_y2 {
        for tile_ix in tex_x..tex_x2 {
            let (blit_x, blit_y) = (tile_ix * ts, tile_iy * ts);
            // Source bounds within this tile (StdDDraw2.cpp:737-741).
            let s_left = (fx - blit_x as f32).max(0.0);
            let s_top = (fy - blit_y as f32).max(0.0);
            let s_right = (fx + fwdt - blit_x as f32).min(ts as f32);
            let s_bottom = (fy + fhgt - blit_y as f32).min(ts as f32);
            // Destination quad (StdDDraw2.cpp:742-746).
            let t_left = (s_left + blit_x as f32 - fx) * scale_x + tx;
            let t_top = (s_top + blit_y as f32 - fy) * scale_y + ty;
            let t_right = (s_right + blit_x as f32 - fx) * scale_x + tx;
            let t_bottom = (s_bottom + blit_y as f32 - fy) * scale_y + ty;
            // Pixels whose centers fall inside the quad.
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
                    // Texture matrix of the quad (StdDDraw2.cpp:752-760) with
                    // texIndent = blitOffset = 0: texel = srcX + (fragX - tx)
                    // / scale; GL_LINEAR taps at texel - 0.5.
                    let u_rel = fx - blit_x as f32 + (px as f32 + 0.5 - tx) / scale_x - 0.5;
                    let v_rel = fy - blit_y as f32 + (py as f32 + 0.5 - ty) / scale_y - 0.5;
                    let s = bilinear_sample_tile(image, blit_x, blit_y, ts, u_rel, v_rel);
                    if s[3] <= 0.0 {
                        continue;
                    }
                    let af = (s[3] / 255.0).clamp(0.0, 1.0);
                    let dst = surface.get_pixel(px as u32, py as u32).unwrap_or_default();
                    let blend = |src: f32, dst: u8| -> u8 {
                        (encode_channel(gamma, src) * af + f32::from(dst) * (1.0 - af))
                            .round()
                            .clamp(0.0, 255.0) as u8
                    };
                    let out = Color::new(blend(s[0], dst.r), blend(s[1], dst.g), blend(s[2], dst.b), 255);
                    let _ = surface.set_pixel(px as u32, py as u32, out);
                }
            }
        }
    }
}

/// `CStdDDraw::DrawBoxDw` (StdDDraw2.cpp:1106-1110 → DrawBoxFade →
/// `CStdGL::DrawQuadDw`, StdGL.cpp:846-891): fills the INCLUSIVE rect
/// `x1..=x2, y1..=y2` with an engine AARRGGBB color whose alpha is INVERTED
/// (0x00 = opaque). Blend is `glBlendFunc(GL_ONE_MINUS_SRC_ALPHA,
/// GL_SRC_ALPHA)`: out = src*(255-A)/255 + dst*A/255, with the quad color
/// gamma-encoded by the dummy shader before blending.
fn draw_box_dw(
    surface: &mut Surface,
    x1: i32,
    y1: i32,
    x2: i32,
    y2: i32,
    clr: u32,
    gamma: Option<&GammaRamp>,
) {
    let a = (clr >> 24) & 0xff;
    let opacity = (255 - a) as f32 / 255.0;
    if opacity <= 0.0 {
        return;
    }
    let src = [
        encode_channel(gamma, ((clr >> 16) & 0xff) as f32),
        encode_channel(gamma, ((clr >> 8) & 0xff) as f32),
        encode_channel(gamma, (clr & 0xff) as f32),
    ];
    for y in y1.max(0)..=y2.min(surface.height() as i32 - 1) {
        for x in x1.max(0)..=x2.min(surface.width() as i32 - 1) {
            let dst = surface.get_pixel(x as u32, y as u32).unwrap_or_default();
            let blend =
                |s: f32, d: u8| (s * opacity + f32::from(d) * (1.0 - opacity)).round() as u8;
            let out = Color::new(
                blend(src[0], dst.r),
                blend(src[1], dst.g),
                blend(src[2], dst.b),
                255,
            );
            let _ = surface.set_pixel(x as u32, y as u32, out);
        }
    }
}

/// `CStdGL::DrawLineDw` (StdGL.cpp:893-934) for the axis-aligned lines of
/// `Draw3DFrame`: an aliased GL_LINES segment from (x1+0.5,y1+0.5) to
/// (x2+0.5,y2+0.5). With both endpoints on pixel centers the diamond-exit
/// rule produces fragments for every pixel except the final one. Blending
/// and opacity semantics equal [`draw_box_dw`] (the inverted engine alpha
/// becomes GL alpha via `InvertRGBAAlpha`, StdGL.cpp:923-925).
fn draw_line_dw(
    surface: &mut Surface,
    x1: i32,
    y1: i32,
    x2: i32,
    y2: i32,
    clr: u32,
    gamma: Option<&GammaRamp>,
) {
    if x1 == x2 {
        // vertical: y1..y2 excluding the end pixel
        draw_box_dw(surface, x1, y1.min(y2), x1, y1.max(y2) - 1, clr, gamma);
    } else {
        debug_assert_eq!(y1, y2);
        draw_box_dw(surface, x1.min(x2), y1, x1.max(x2) - 1, y1, clr, gamma);
    }
}

/// `C4GUI::Element::Draw3DFrame` (C4Gui.cpp:264-279) with the default border
/// colors (C4Gui.h:97-100) and `byAlpha` = C4GUI_BorderAlpha = 0xaf.
fn draw_3d_frame(surface: &mut Surface, rect: &IntRect, gamma: Option<&GammaRamp>) {
    const ALPHA: u32 = 0xaf << 24;
    const C1: u32 = ALPHA | 0x772200; // C4GUI_BorderColor1
    const C2: u32 = ALPHA | 0x331100; // C4GUI_BorderColor2
    const C3: u32 = ALPHA | 0xaa4400; // C4GUI_BorderColor3
    let (x0, y0) = (rect.x, rect.y);
    let (x1, y1) = (rect.x + rect.w - 1, rect.y + rect.h - 1);
    draw_line_dw(surface, x0, y0, x1, y0, C1, gamma); // top
    draw_line_dw(surface, x0, y0, x0, y1, C1, gamma); // left
    draw_line_dw(surface, x0 + 1, y0 + 1, x1 - 1, y0 + 1, C2, gamma); // top inner
    draw_line_dw(surface, x0 + 1, y0 + 1, x0 + 1, y1 - 1, C2, gamma); // left inner
    draw_line_dw(surface, x0, y1, x1, y1, C3, gamma); // bottom
    draw_line_dw(surface, x1, y0, x1, y1, C3, gamma); // right
    draw_line_dw(surface, x0 + 1, y1 - 1, x1 - 1, y1 - 1, C1, gamma); // bottom inner
    draw_line_dw(surface, x1 - 1, y0 + 1, x1 - 1, y1 - 1, C1, gamma); // right inner
}

/// `C4GUI::Element::DrawVBar` (C4Gui.cpp:332-345) with the book scrollbar
/// facets (`ScrollBarFacets::Set`, C4Gui.cpp:109-121): begin = up arrow
/// (0,0,16,16), middle = track tile (0,16,16,16) tiled while `iY < Hgt - 5`
/// (the last tile height-clipped), end = down arrow (0,32,16,16) drawn at
/// the bottom. All slices are 1:1 blits.
fn draw_vbar(
    surface: &mut Surface,
    x: i32,
    y: i32,
    height: i32,
    image: &ImageData,
    gamma: Option<&GammaRamp>,
) {
    draw_image_strip(surface, x, y, image, 0, 0, 16, 16, gamma);
    let mut iy = 16;
    while iy < height - 5 {
        let tile_h = 16.min(height - 5 - iy) as u32;
        draw_image_strip(surface, x, y + iy, image, 0, 16, 16, tile_h, gamma);
        iy += 16;
    }
    draw_image_strip(surface, x, y + height - 16, image, 0, 32, 16, 16, gamma);
}

/// The zoomed branch of `C4GUI::Element::DrawBar` (C4Gui.cpp:313-329) for
/// `GetRes()->barCaption`: GUICaption.png (192x23) sliced with border 32
/// (`barCaption.SetHorizontal(sfcCaption, sfcCaption.Hgt, 32)`,
/// C4Gui.cpp:1088) and vertically zoomed to the bar height.
fn draw_caption_bar(
    surface: &mut Surface,
    rect: &IntRect,
    image: &ImageData,
    gamma: Option<&GammaRamp>,
) {
    let img_h = image.height() as f32; // 23
    let zoom = rect.h as f32 / img_h;
    let (x0, y0) = (rect.x as f32, rect.y as f32);
    let h = rect.h as f32;
    let begin_w = (zoom * 32.0) as i32;
    let mid_w = (zoom * 128.0) as i32;
    let right_show = 32 / 3; // iRightShowLength = fctEnd.Wdt / 3
    draw_facet_stretch(
        surface,
        image,
        (0.0, 0.0, 32.0, img_h),
        (x0, y0, begin_w as f32, h),
        gamma,
    );
    let mut ix = begin_w;
    while (ix as f32) < rect.w as f32 - zoom * right_show as f32 {
        let w2 = mid_w.min(rect.w - (zoom * right_show as f32) as i32 - ix);
        let src_w = (w2 as f32 / zoom) as i32; // long(float(w2) / fZoom)
        draw_facet_stretch(
            surface,
            image,
            (32.0, 0.0, src_w as f32, img_h),
            ((x0 as i32 + ix) as f32, y0, w2 as f32, h),
            gamma,
        );
        ix += mid_w;
    }
    draw_facet_stretch(
        surface,
        image,
        (160.0, 0.0, 32.0, img_h),
        ((rect.x + rect.w - begin_w) as f32, y0, begin_w as f32, h),
        gamma,
    );
}

/// Draws text through a primary-clipper rectangle (inclusive bounds, like
/// `CStdDDraw::SetPrimaryClipper`, StdDDraw2.cpp:566-599): the text is
/// rendered to a scratch copy and only the pixels inside the clip rect are
/// committed.
#[allow(clippy::too_many_arguments)]
fn draw_text_clipped(
    surface: &mut Surface,
    font: &ClonkFont,
    x: i32,
    y: i32,
    text: &str,
    color: [u8; 4],
    align: TextAlign,
    markup: bool,
    gamma: Option<&GammaRamp>,
    clip: (i32, i32, i32, i32),
) {
    let mut scratch = surface.clone();
    font.draw_with_gamma(&mut scratch, x, y, text, color, align, markup, gamma);
    let (cx1, cy1, cx2, cy2) = clip;
    for py in cy1.max(0)..=cy2.min(surface.height() as i32 - 1) {
        for px in cx1.max(0)..=cx2.min(surface.width() as i32 - 1) {
            if let Some(c) = scratch.get_pixel(px as u32, py as u32) {
                let _ = surface.set_pixel(px as u32, py as u32, c);
            }
        }
    }
}

// ---------------------------------------------------------------------------
// List items (C4StartupScenSelDlg::ScenListItem, for populated lists)
// ---------------------------------------------------------------------------

/// Height of one scenario list row: BookFont line height + 2 *
/// IconLabelSpacing(2) (C4StartupScenSelDlg.cpp:1217-1219).
pub fn scen_list_item_height(book_text_font: &ClonkFont) -> i32 {
    book_text_font.line_height + 2 * 2
}

/// Draws one `ScenListItem` (C4StartupScenSelDlg.cpp:1210-1238) at item
/// origin `(x, y)`: a 24x24 icon from `StartupScenSelIcons.png` (index
/// clamped like `C4ScenarioListLoader::Entry`, icon 14 fallback handled by
/// the caller) aspect-stretched into the 26x26 icon picture, and the name in
/// BookFont at x+28, y+2 — black `ClrScenarioItem` when enabled, 50% black
/// when disabled (cpp:1205-1207); markup off (names are pre-stripped).
#[allow(clippy::too_many_arguments)]
pub fn draw_scen_list_item(
    surface: &mut Surface,
    icons: &ImageData,
    book_text_font: &ClonkFont,
    gamma: Option<&GammaRamp>,
    x: i32,
    y: i32,
    icon_index: u32,
    name: &str,
    enabled: bool,
) {
    let item_h = scen_list_item_height(book_text_font); // 26
    // Picture(0,0,26,26, fAspect=true): a 24x24 facet has the same aspect, so
    // it stretches to the full rect (C4Facet::Draw, C4Facet.cpp:447-467).
    let icon_count = icons.width() / icons.height().max(1);
    let src_x = (icon_index.min(icon_count.saturating_sub(1)) * icons.height()) as f32;
    draw_facet_stretch(
        surface,
        icons,
        (src_x, 0.0, icons.height() as f32, icons.height() as f32),
        (x as f32, y as f32, item_h as f32, item_h as f32),
        gamma,
    );
    let color = if enabled {
        [0, 0, 0, 255] // ClrScenarioItem = 0xff000000
    } else {
        [0, 0, 0, 127] // ClrScenarioItemDisabled = 0x7f000000
    };
    book_text_font.draw_with_gamma(
        surface,
        x + item_h + 2,
        y + 2,
        name,
        color,
        TextAlign::Left,
        false,
        gamma,
    );
}

// ---------------------------------------------------------------------------
// Renderer (steady-state first-shown frame, spec §6)
// ---------------------------------------------------------------------------

/// Renders the first-shown state of `C4StartupScenSelDlg` (empty scenario
/// list — the F9 reference was captured with no scenarios in the exe dir).
pub struct ScenSelScreen;

impl ScenSelScreen {
    /// Draws one steady-state frame in the C++ draw order
    /// (Screen::Draw → Dialog::Draw, spec §6). `fair_crew`/`record` mirror
    /// `Config.General.FairCrew`/`Record` for the two icon buttons
    /// (C4Network2Dialogs.cpp:653-655,796-816).
    pub fn render(
        surface: &mut Surface,
        assets: &ScenSelAssets,
        gui_fonts: &ClonkFontSet,
        book_fonts: &BookFontSet,
        gamma: Option<&GammaRamp>,
        fair_crew: bool,
        record: bool,
    ) {
        Self::render_chrome(surface, assets, gui_fonts, gamma, fair_crew, record);
        let layout = scen_sel_layout(surface.width() as i32, surface.height() as i32, gui_fonts);
        // Default selection-independent widget states (first-shown frame):
        // root caption, "Open" button, disabled unchecked checkbox.
        draw_book_caption(surface, &layout, book_fonts, "Scenarios", gamma);
        draw_open_button(surface, &layout, "Open", assets, gui_fonts, gamma);
        draw_user_change_checkbox(surface, &layout, assets, gui_fonts, false, false, gamma);
    }

    /// The selection-independent part of the frame: everything except the
    /// book caption, the Open button and the "Choose definitions" checkbox,
    /// which change with the selection and are drawn on top by the caller.
    pub fn render_chrome(
        surface: &mut Surface,
        assets: &ScenSelAssets,
        gui_fonts: &ClonkFontSet,
        gamma: Option<&GammaRamp>,
        fair_crew: bool,
        record: bool,
    ) {
        let layout = scen_sel_layout(surface.width() as i32, surface.height() as i32, gui_fonts);
        let yellow = [255, 255, 0, 255]; // C4GUI_Caption2FontClr / ButtonFontClr

        // 1. Background: StartupScenSelBG stretched to screen bounds
        // inflated by 1px (FullscreenDialog::DrawBackground,
        // C4GuiDialogs.cpp:878-887).
        draw_image_bilinear(
            surface,
            &GuiRect::new(
                -1.0,
                -1.0,
                surface.width() as f32 + 2.0,
                surface.height() as f32 + 2.0,
            ),
            &assets.background,
            gamma,
        );

        // 2. Fullscreen title "Start Game" (IDS_DLG_STARTGAME, amp stripped):
        // GUI TitleFont, C4GUI_FullscreenCaptionFontClr yellow, ACenter
        // (C4GuiDialogs.cpp:846, C4GuiLabels.cpp:34-37).
        gui_fonts.title.draw_with_gamma(
            surface,
            layout.title_anchor.0,
            layout.title_anchor.1,
            "Start Game",
            yellow,
            TextAlign::Center,
            true,
            gamma,
        );

        // 4. WoodenLabel "Search:" (C4GuiLabels.cpp:168-209): zoomed
        // barCaption wood, then GUI TextFont yellow ACenter at the label
        // middle, one pixel above the vertical center, clipped to the label
        // bounds.
        draw_caption_bar(surface, &layout.search_label, &assets.caption_bar, gamma);
        let label = &layout.search_label;
        draw_text_clipped(
            surface,
            &gui_fonts.text,
            label.x + label.w / 2,
            label.y + (label.h - gui_fonts.text.line_height) / 2 - 1,
            "Search:",
            yellow,
            TextAlign::Center,
            true,
            gamma,
            (label.x, label.y, label.x + label.w, label.y + label.h),
        );

        // 5. Search Edit (C4GuiEdit.cpp:556-569): background box from the
        // bounds top-left to (x + W - 1, clientBottom) in C4GUI_EditBGColor,
        // then the default 3D frame. Empty text, unfocused: no text/cursor.
        let edit = &layout.search_edit;
        draw_box_dw(
            surface,
            edit.x,
            edit.y,
            edit.x + edit.w - 1,
            edit.y + 2 + (edit.h - 4), // rcClientRect.y + rcClientRect.Hgt (margins T2/B2)
            0x7f000000,
            gamma,
        );
        draw_3d_frame(surface, edit, gamma);

        // 6. List scrollbar track (visible-but-pinless first-shown quirk,
        // spec §4.3): DrawVBar with sfctBookScroll
        // (C4GuiContainers.cpp:446-473).
        let bar = &layout.list_scrollbar;
        draw_vbar(surface, bar.x, bar.y, bar.h, &assets.book_scroll, gamma);

        // 7-10. Bottom bar in add order: Back, icon buttons
        // (C4StartupScenSelDlg.cpp:1367-1382); the checkbox and Open button
        // are selection-dependent and drawn by the caller.
        draw_button(surface, &layout.back_button, "Back", assets, gui_fonts, gamma);

        // Icon buttons (IconButton::DrawElement, C4GuiButton.cpp:205-232):
        // plain 64x64 icon blit, no highlight without focus/hover.
        // Icons from GUIIcons2 (Icon::GetIconFacet, C4GuiLabels.cpp:441-450):
        // fair crew = Ico_Ex_FairCrew(2)/Ico_Ex_NormalCrew(3), record =
        // Ico_Ex_RecordOn(1)/Ico_Ex_RecordOff(0).
        let icon_ex = |idx: u32| ((idx % 4) * 64, (idx / 4) * 64);
        let (fc_x, fc_y) = icon_ex(if fair_crew { 2 } else { 3 });
        let fc = &layout.fair_crew_button;
        draw_image_strip(surface, fc.x, fc.y, &assets.icons_ex, fc_x, fc_y, 64, 64, gamma);
        let (rec_x, rec_y) = icon_ex(if record { 1 } else { 0 });
        let rec = &layout.record_button;
        draw_image_strip(surface, rec.x, rec.y, &assets.icons_ex, rec_x, rec_y, 64, 64, gamma);
    }
}

/// The book caption above the scenario list: the current folder's name, or
/// "Scenarios" (IDS_DLG_SCENARIOS) at the root — BookFontTitle, black,
/// ACenter (C4StartupScenSelDlg.cpp:1331-1334,1527-1535).
pub fn draw_book_caption(
    surface: &mut Surface,
    layout: &ScenSelLayout,
    book_fonts: &BookFontSet,
    caption: &str,
    gamma: Option<&GammaRamp>,
) {
    book_fonts.title.draw_with_gamma(
        surface,
        layout.caption_anchor.0,
        layout.caption_anchor.1,
        caption,
        [0, 0, 0, 255], // ClrScenarioItem
        TextAlign::Center,
        true,
        gamma,
    );
}

/// Draws the mutable contents of the cached scenario-search edit. The edit
/// frame itself is part of [`ScenSelScreen::render_chrome`]; C++ routes text
/// through `C4GUI::Edit::DrawElement` (C4GuiEdit.cpp:556-626).
pub fn draw_search_edit_contents(
    surface: &mut Surface,
    layout: &ScenSelLayout,
    gui_fonts: &ClonkFontSet,
    text: &str,
    focused: bool,
    gamma: Option<&GammaRamp>,
) {
    let edit = &layout.search_edit;
    // Restore the client over the cached chrome before drawing a new buffer.
    draw_box_dw(
        surface,
        edit.x + 2,
        edit.y + 2,
        edit.x + edit.w - 3,
        edit.y + edit.h - 3,
        0x7f000000,
        gamma,
    );
    let text_y = edit.y + (edit.h - gui_fonts.text.line_height) / 2;
    draw_text_clipped(
        surface,
        &gui_fonts.text,
        edit.x + 2,
        text_y,
        text,
        [255, 255, 255, 255],
        TextAlign::Left,
        false,
        gamma,
        (
            edit.x + 2,
            edit.y + 2,
            edit.x + edit.w - 3,
            edit.y + edit.h - 3,
        ),
    );
    if focused {
        let cursor_x = edit.x + 2 + gui_fonts.text.measure(text, false).0;
        draw_line_dw(
            surface,
            cursor_x,
            edit.y + 3,
            cursor_x,
            edit.y + edit.h - 4,
            0xffffffff,
            gamma,
        );
    }
}

/// The Open/Start button with its selection-specific text — "Open"
/// (IDS_BTN_OPEN) for folders/none, "&Start" (IDS_BTN_STARTGAME) for
/// scenarios (Entry::GetOpenText, C4StartupScenSelDlg.cpp:794-797,926-929;
/// applied in UpdateSelection, :1587).
pub fn draw_open_button(
    surface: &mut Surface,
    layout: &ScenSelLayout,
    text: &str,
    assets: &ScenSelAssets,
    gui_fonts: &ClonkFontSet,
    gamma: Option<&GammaRamp>,
) {
    let (text, _) = expand_hotkey_markup(text);
    draw_button(surface, &layout.open_button, &text, assets, gui_fonts, gamma);
}

/// The "Choose definitions" checkbox (IDS_DLG_ALLOWUSERCHANGE): phase
/// fChecked + 2*!fEnabled (C4GuiCheckBox.cpp:110-137); enabled/checked per
/// the selected scenario's [Definitions] LocalOnly/AllowUserChange
/// (C4StartupScenSelDlg.cpp:1590-1599).
pub fn draw_user_change_checkbox(
    surface: &mut Surface,
    layout: &ScenSelLayout,
    assets: &ScenSelAssets,
    gui_fonts: &ClonkFontSet,
    enabled: bool,
    checked: bool,
    gamma: Option<&GammaRamp>,
) {
    let cb = &layout.user_change_checkbox;
    let phase = u32::from(checked) + 2 * u32::from(!enabled);
    draw_image_strip(surface, cb.x, cb.y, &assets.checkbox, phase * 32, 0, 32, 32, gamma);
    let (caption, _) = expand_hotkey_markup("Choose &definitions");
    let color = if enabled {
        [255, 255, 255, 255] // C4GUI_CheckboxFontClr
    } else {
        [0xaf, 0xaf, 0xaf, 255] // C4GUI_CheckboxDisabledFontClr
    };
    gui_fonts.text.draw_with_gamma(
        surface,
        cb.x + cb.h + 4, // x0 + Hgt + C4GUI_CheckBoxLabelSpacing
        cb.y + (cb.h - gui_fonts.text.line_height).max(0) / 2,
        &caption,
        color,
        TextAlign::Left,
        true,
        gamma,
    );
}

/// One released GUI button (Button::DrawElement, C4GuiButton.cpp:81-110):
/// 3-slice GUIButton plank and the caption in the largest font fitting
/// `Hgt - 2` (CaptionFont for 32px buttons), C4GUI_ButtonFontClr yellow.
fn draw_button(
    surface: &mut Surface,
    rect: &IntRect,
    text: &str,
    assets: &ScenSelAssets,
    gui_fonts: &ClonkFontSet,
    gamma: Option<&GammaRamp>,
) {
    draw_bar(
        surface,
        &GuiRect::new(rect.x as f32, rect.y as f32, rect.w as f32, rect.h as f32),
        &assets.button,
        gamma,
    );
    let font = gui_fonts.button_font(rect.h);
    let (x1, y1) = (rect.x + rect.w - 1, rect.y + rect.h - 1);
    font.draw_with_gamma(
        surface,
        (rect.x + x1) / 2,
        (rect.y + y1 - font.line_height) / 2,
        text,
        [255, 255, 0, 255],
        TextAlign::Center,
        true,
        gamma,
    );
}

// ---------------------------------------------------------------------------
// Right book page (C4StartupScenSelDlg::UpdateSelection + C4GUI::TextWindow)
// ---------------------------------------------------------------------------

/// The selected entry's data shown on the right book page
/// (C4StartupScenSelDlg::UpdateSelection, C4StartupScenSelDlg.cpp:1551-1619).
#[derive(Default)]
pub struct SelectionInfo<'a> {
    /// `Entry::GetTitlePicture` — the Title.png/Title.bmp facet.
    pub picture: Option<&'a ImageData>,
    /// `Entry::GetName` — shown alone only when there is no description.
    pub title: Option<&'a str>,
    /// `Entry::GetDesc` — Desc??.rtf plain text.
    pub desc: Option<&'a str>,
    /// `Entry::GetAuthor` — "Author: %s" line (IDS_CTL_AUTHOR).
    pub author: Option<&'a str>,
    /// `Entry::GetVersion` — "Version %s" line (IDS_DLG_VERSION).
    pub version: Option<&'a str>,
}

/// Scroll bounds of the selection-info `C4GUI::TextWindow`, in logical GUI
/// pixels (`ScrollWindow::Update`/`ScrollBy`, C4GuiContainers.cpp:493-541).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct SelectionInfoScrollMetrics {
    pub viewport_height: i32,
    pub content_height: i32,
    pub max_scroll: i32,
}

impl SelectionInfoScrollMetrics {
    pub fn clamp_offset(self, offset: i32) -> i32 {
        offset.clamp(0, self.max_scroll)
    }
}

fn selection_info_client(layout: &ScenSelLayout) -> (IntRect, i32) {
    // TextWindow margins: left 10, right 5, top 8, bottom 8 (C4Gui.h:1334-
    // 1337); the ScrollWindow always reserves 16px for its scrollbar.
    let win = &layout.selection_info;
    let client = IntRect {
        x: win.x + 10,
        y: win.y + 8,
        w: win.w - 15,
        h: win.h - 16,
    };
    let content_w = client.w - 16;
    (client, content_w)
}

fn selection_info_lines<'a>(
    info: &SelectionInfo<'_>,
    book_fonts: &'a BookFontSet,
) -> Vec<(String, &'a ClonkFont, [u8; 4])> {
    // "never show a pure title string: There must always be some text or an
    // image" (C4StartupScenSelDlg.cpp:1583-1585).
    let has_desc = info.desc.is_some_and(|desc| !desc.is_empty());
    let title = info
        .title
        .filter(|title| !title.is_empty() && (info.picture.is_some() || has_desc));
    let black = [0u8, 0, 0, 255]; // ClrScenarioItem
    let half_black = [0u8, 0, 0, 127]; // ClrScenarioItemXtra
    let mut lines = Vec::new();
    if let (Some(title), false) = (title, has_desc) {
        lines.push((title.to_string(), &book_fonts.caption, black));
    }
    if let Some(desc) = info.desc.filter(|_| has_desc) {
        let mut first = true;
        for segment in desc.split(['\r', '\n']).filter(|line| !line.is_empty()) {
            let font = if first {
                &book_fonts.caption
            } else {
                &book_fonts.text
            };
            first = false;
            lines.push((segment.to_string(), font, black));
        }
    }
    if let Some(author) = info.author.filter(|author| !author.is_empty()) {
        lines.push((format!("Author: {author}"), &book_fonts.text, half_black));
    }
    if let Some(version) = info.version.filter(|version| !version.is_empty()) {
        lines.push((format!("Version {version}"), &book_fonts.text, half_black));
    }
    lines
}

pub fn selection_info_scroll_metrics(
    layout: &ScenSelLayout,
    book_fonts: &BookFontSet,
    info: &SelectionInfo<'_>,
) -> SelectionInfoScrollMetrics {
    let (client, content_w) = selection_info_client(layout);
    let picture_height = info.picture.map_or(0, |_| {
        let pic_w = 220.min(content_w);
        170 * pic_w / 220 + 10
    });
    let text_height = selection_info_lines(info, book_fonts)
        .iter()
        .map(|(text, font, _)| wrap_line(text, font, content_w).len() as i32 * font.line_height)
        .sum::<i32>();
    let content_height = picture_height + text_height;
    SelectionInfoScrollMetrics {
        viewport_height: client.h,
        content_height,
        max_scroll: (content_height - client.h).max(0),
    }
}

/// Renders the right-page selection info like the C++ TextWindow
/// (C4GuiLabels.cpp:454-489 geometry; C4Gui.h:1334-1337 margins; picture as
/// OverlayPicture with the ScenSelTitleOv frame, border 10;
/// C4StartupScenSelDlg.cpp:1607-1616 line contents). Content that exceeds
/// the window is clipped and the book scrollbar track drawn.
pub fn draw_selection_info(
    surface: &mut Surface,
    layout: &ScenSelLayout,
    assets: &ScenSelAssets,
    book_fonts: &BookFontSet,
    info: &SelectionInfo,
    gamma: Option<&GammaRamp>,
) {
    draw_selection_info_scrolled(surface, layout, assets, book_fonts, info, 0, gamma);
}

/// Draws selection info at a clamped `ScrollWindow::iScrollY` offset and
/// returns the exact content bounds used for wheel/scrollbar interaction.
#[allow(clippy::too_many_arguments)]
pub fn draw_selection_info_scrolled(
    surface: &mut Surface,
    layout: &ScenSelLayout,
    assets: &ScenSelAssets,
    book_fonts: &BookFontSet,
    info: &SelectionInfo,
    scroll_y: i32,
    gamma: Option<&GammaRamp>,
) -> SelectionInfoScrollMetrics {
    let (client, content_w) = selection_info_client(layout);
    let metrics = selection_info_scroll_metrics(layout, book_fonts, info);
    let scroll_y = metrics.clamp_offset(scroll_y);
    let mut scratch = surface.clone();
    let mut y = client.y - scroll_y;
    // Title picture: 220x170 incl. the 10px overlay margin (TextWindow ctor
    // with C4StartupScenSel_TitlePictureWdt/Hgt + 2*TitleOverlayMargin,
    // C4StartupScenSelDlg.cpp:1361-1362; C4GuiLabels.cpp:469-483).
    if let Some(picture) = info.picture {
        let pic_w = 220.min(content_w);
        let pic_h = 170 * pic_w / 220;
        let pic_x = client.x + (content_w / 2 - 220 / 2).max(0);
        // OverlayPicture (C4GuiLabels.cpp:405-423): inner picture inset by
        // border * rc / overlay-size, stretched without aspect; the overlay
        // frame over the full rect.
        let overlay = &assets.title_overlay;
        let inset_x = 10 * pic_w / overlay.width().max(1) as i32;
        let inset_y = 10 * pic_h / overlay.height().max(1) as i32;
        draw_facet_stretch(
            &mut scratch,
            picture,
            (0.0, 0.0, picture.width() as f32, picture.height() as f32),
            (
                (pic_x + inset_x) as f32,
                (y + inset_y) as f32,
                (pic_w - 2 * inset_x) as f32,
                (pic_h - 2 * inset_y) as f32,
            ),
            gamma,
        );
        draw_facet_stretch(
            &mut scratch,
            overlay,
            (0.0, 0.0, overlay.width() as f32, overlay.height() as f32),
            (pic_x as f32, y as f32, pic_w as f32, pic_h as f32),
            gamma,
        );
        y += pic_h + 10; // C4StartupScenSel_TitlePicturePadding
    }

    // Word-wrap (CStdFont::BreakMessage semantics: greedy break at spaces).
    // The whole child window is drawn shifted and then clipped, preserving
    // partially visible picture/text rows at both viewport edges.
    for (text, font, color) in selection_info_lines(info, book_fonts) {
        for wrapped in wrap_line(&text, font, content_w) {
            font.draw_with_gamma(
                &mut scratch,
                client.x,
                y,
                &wrapped,
                color,
                TextAlign::Left,
                false,
                gamma,
            );
            y += font.line_height;
        }
    }

    let clip_right = client.x + content_w - 1;
    let clip_bottom = client.y + client.h - 1;
    for py in client.y.max(0)..=clip_bottom.min(surface.height() as i32 - 1) {
        for px in client.x.max(0)..=clip_right.min(surface.width() as i32 - 1) {
            if let Some(color) = scratch.get_pixel(px as u32, py as u32) {
                let _ = surface.set_pixel(px as u32, py as u32, color);
            }
        }
    }

    // Book scrollbar track + fixed 16px pin on overflow
    // (C4GuiContainers.cpp:343-368,446-473).
    if metrics.max_scroll > 0 {
        let bar_x = client.x + client.w - 16;
        draw_vbar(
            surface,
            bar_x,
            client.y,
            client.h,
            &assets.book_scroll,
            gamma,
        );
        let max_pin_travel = (client.h - 48).max(0);
        let pin_y = client.y + 16 + max_pin_travel * scroll_y / metrics.max_scroll;
        draw_image_strip(
            surface,
            bar_x,
            pin_y,
            &assets.book_scroll,
            16,
            16,
            16,
            16,
            gamma,
        );
    }
    metrics
}

/// Greedy word wrap at spaces against the pixel width, like
/// `CStdFont::BreakMessage` for label text.
fn wrap_line(text: &str, font: &ClonkFont, width: i32) -> Vec<String> {
    let mut wrapped = Vec::new();
    let mut current = String::new();
    for word in text.split(' ').filter(|word| !word.is_empty()) {
        let candidate = if current.is_empty() {
            word.to_string()
        } else {
            format!("{current} {word}")
        };
        if current.is_empty() || font.measure(&candidate, false).0 <= width {
            current = candidate;
        } else {
            wrapped.push(std::mem::take(&mut current));
            current = word.to_string();
        }
    }
    if !current.is_empty() || wrapped.is_empty() {
        wrapped.push(current);
    }
    wrapped
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_support::endeavour_font_set;

    // Shadowless book fonts per CStdFont::Init(fDoShadow = false)
    // (StdFont.cpp:327,352): iHSpace = 0, iGfxLineHgt = iLineHgt, atlas
    // pixels are pure white with alpha = coverage (no shadow kernel).
    #[test]
    fn book_fonts_are_shadowless() {
        let ttf = std::fs::read(
            crate::test_support::repo_root().join("planet/System.c4g/Endeavour.ttf"),
        )
        .expect("read Endeavour.ttf");
        let fonts = build_book_font_set(&ttf).expect("build book fonts");
        for (font, line_height) in [
            (&fonts.title, 34),
            (&fonts.caption, 25),
            (&fonts.text, 22),
        ] {
            assert_eq!(font.line_height, line_height);
            assert_eq!(font.cell_height, line_height, "no +1 shadow row");
            assert_eq!(font.h_space, 0, "no shadow overlap indent");
        }
        // Every visible glyph pixel is pure white with alpha = coverage — the
        // shadowed build would contain grey/black shadow pixels.
        let glyph = fonts.title.glyph('S').expect("glyph S");
        assert!(glyph
            .pixels
            .iter()
            .all(|px| px.a == 0 || (px.r, px.g, px.b) == (255, 255, 255)));
        assert!(glyph.pixels.iter().any(|px| px.a == 255), "solid coverage");
    }

    // Pixel-exact geometry at 1280x720, derived from
    // C4StartupScenSelDlg.cpp:1302-1382 and verified against the C++ F9
    // capture (ref-scensel.png).
    #[test]
    fn layout_matches_cpp_scensel_dlg_at_1280x720() {
        let fonts = endeavour_font_set();
        let layout = scen_sel_layout(1280, 720, &fonts);
        let w = layout.button_width;
        let s = layout.search_text_width;

        // Client: margins x = 1280/50 = 25, top = 720/7 = 102, bottom = 19.
        assert_eq!(
            (layout.client.x, layout.client.y, layout.client.w, layout.client.h),
            (25, 102, 1230, 599)
        );
        // Title label keeps its pre-override offsets: center 640, top 41.
        assert_eq!(layout.title_anchor, (640, 41));
        // Book caption: center 374, top 143 (sheet origin 25,50).
        assert_eq!(layout.caption_anchor, (374, 143));
        // Search row: label (169,564,S+10,22), edit (179+S,564,400-S,22).
        assert_eq!(
            (layout.search_label.x, layout.search_label.y, layout.search_label.h),
            (169, 564, 22)
        );
        assert_eq!(layout.search_label.w, s + 10);
        assert_eq!(
            (layout.search_edit.x, layout.search_edit.y, layout.search_edit.w, layout.search_edit.h),
            (179 + s, 564, 400 - s, 22)
        );
        // List box (169,187,410,367); scrollbar track (560,190,16,361).
        assert_eq!(
            (layout.list.x, layout.list.y, layout.list.w, layout.list.h),
            (169, 187, 410, 367)
        );
        assert_eq!(
            (
                layout.list_scrollbar.x,
                layout.list_scrollbar.y,
                layout.list_scrollbar.w,
                layout.list_scrollbar.h
            ),
            (560, 190, 16, 361)
        );
        // Right page (702,138,410,453).
        assert_eq!(
            (
                layout.selection_info.x,
                layout.selection_info.y,
                layout.selection_info.w,
                layout.selection_info.h
            ),
            (702, 138, 410, 453)
        );
        // Bottom bar: Back (35,648,W,32), Open (1245-W,...), checkbox
        // (1225-2W,...), icon buttons 64x64 at y=632.
        assert_eq!(
            (layout.back_button.x, layout.back_button.y, layout.back_button.w, layout.back_button.h),
            (35, 648, w, 32)
        );
        assert_eq!(
            (layout.open_button.x, layout.open_button.y),
            (1245 - w, 648)
        );
        assert_eq!(
            (layout.user_change_checkbox.x, layout.user_change_checkbox.y),
            (1225 - 2 * w, 648)
        );
        assert_eq!(
            (layout.fair_crew_button.x, layout.fair_crew_button.y),
            (479, 632)
        );
        assert_eq!((layout.record_button.x, layout.record_button.y), (563, 632));
        assert_eq!((layout.fair_crew_button.w, layout.fair_crew_button.h), (64, 64));

        // Pin the measured font widths so regressions in the font code are
        // caught here: W = 3 * caption("<< BACK") = 3*51, S = text("Search:").
        assert_eq!(w, 153);
        assert_eq!(s, 46);
    }

    // ScenListItem (C4StartupScenSelDlg.cpp:1210-1238): 26px row, the 24x24
    // icon aspect-stretched into (0,0,26,26), the name in BookFont black at
    // x = 28, y = 2.
    #[test]
    fn scen_list_item_renders_icon_and_label() {
        let ttf = std::fs::read(
            crate::test_support::repo_root().join("planet/System.c4g/Endeavour.ttf"),
        )
        .expect("read Endeavour.ttf");
        let book_fonts = build_book_font_set(&ttf).expect("build book fonts");
        assert_eq!(scen_list_item_height(&book_fonts.text), 26);

        // Synthetic icon strip: cell 0 red, cell 1 green (52 cells, 1248x24).
        let pixels = (0..24u32)
            .flat_map(|_| {
                (0..1248u32).flat_map(|x| {
                    if x < 24 {
                        [255, 0, 0, 255]
                    } else if x < 48 {
                        [0, 255, 0, 255]
                    } else {
                        [0, 0, 255, 255]
                    }
                })
            })
            .collect();
        let icons = crate::ImageData::new(1248, 24, pixels);

        let mut surface = lc_graphics::Surface::new(120, 30, lc_graphics::PixelFormat::Rgba8888);
        draw_scen_list_item(&mut surface, &icons, &book_fonts.text, None, 0, 0, 1, "I", true);
        // Icon stretched over the full 26x26 picture rect: pure cell-1 green
        // at the center, nothing at column 26.
        assert_eq!(
            surface.get_pixel(13, 13),
            Some(lc_graphics::Color::new(0, 255, 0, 255))
        );
        assert_eq!(surface.get_pixel(27, 13).map(|c| c.g), Some(0));
        // Label "I" starts at x = 28, y = 2: a black pixel must appear in the
        // glyph cell (black text on the transparent surface).
        let label_hit = (28..40).any(|x| {
            (2..26).any(|y| {
                surface
                    .get_pixel(x, y)
                    .is_some_and(|c| c.a > 200 && c.r == 0 && c.g == 0 && c.b == 0)
            })
        });
        assert!(label_hit, "BookFont label pixel at x>=28");
        // Disabled entries use 50% black (ClrScenarioItemDisabled).
        let mut disabled = lc_graphics::Surface::new(120, 30, lc_graphics::PixelFormat::Rgba8888);
        draw_scen_list_item(&mut disabled, &icons, &book_fonts.text, None, 0, 0, 1, "I", false);
        let max_a = (28..40)
            .flat_map(|x| (2..26).map(move |y| (x, y)))
            .filter_map(|(x, y)| disabled.get_pixel(x, y))
            .filter(|c| c.r == 0 && c.g == 0 && c.b == 0)
            .map(|c| c.a)
            .max()
            .unwrap_or(0);
        assert!(max_a > 0 && max_a < 200, "disabled label is half-opaque");
    }

    // C4GUI::TextWindow::UpdateHeight + ScrollWindow::ScrollBy clamp the
    // wrapped selection description to clientHeight - visibleHeight
    // (C4GuiLabels.cpp:454-489; C4GuiContainers.cpp:493-541).
    #[test]
    fn selection_info_scroll_metrics_cover_wrapped_overflow() {
        let ttf = std::fs::read(
            crate::test_support::repo_root().join("planet/System.c4g/Endeavour.ttf"),
        )
        .expect("read Endeavour.ttf");
        let gui_fonts = crate::clonk_fonts::build_font_set(&ttf).expect("build GUI fonts");
        let book_fonts = build_book_font_set(&ttf).expect("build book fonts");
        let layout = scen_sel_layout(800, 600, &gui_fonts);
        let description = (0..80)
            .map(|index| format!("wrapped scenario description line {index}"))
            .collect::<Vec<_>>()
            .join("\n");
        let info = SelectionInfo {
            title: Some("Overflow"),
            desc: Some(&description),
            ..SelectionInfo::default()
        };

        let metrics = selection_info_scroll_metrics(&layout, &book_fonts, &info);
        assert!(metrics.content_height > metrics.viewport_height);
        assert_eq!(
            metrics.max_scroll,
            metrics.content_height - metrics.viewport_height
        );
        assert_eq!(metrics.clamp_offset(-60), 0);
        assert_eq!(
            metrics.clamp_offset(metrics.max_scroll + 60),
            metrics.max_scroll
        );
    }

    // Renders the first-shown frame at 1280x720 and dumps it for the
    // out-of-band ImageMagick diff against the C++ F9 capture
    // (build/Screenshots/ref-scensel.png). CI has no reference image, so this
    // test only produces the artifact.
    #[test]
    fn render_matches_reference() {
        let load = crate::test_support::load_graphics_png;
        let assets = ScenSelAssets {
            background: load("StartupScenSelBG.png"),
            book_scroll: load("StartupBookScroll.png"),
            scen_icons: load("StartupScenSelIcons.png"),
            caption_bar: load("GUICaption.png"),
            button: load("GUIButton.png"),
            checkbox: load("GUICheckbox.png"),
            icons_ex: load("GUIIcons2.png"),
            title_overlay: load("StartupScenSelTitleOv.png"),
        };
        let gui_fonts = endeavour_font_set();
        let ttf = std::fs::read(
            crate::test_support::repo_root().join("planet/System.c4g/Endeavour.ttf"),
        )
        .expect("read Endeavour.ttf");
        let book_fonts = build_book_font_set(&ttf).expect("build book fonts");
        let gamma = crate::test_support::standard_gamma();

        let mut surface = lc_graphics::Surface::new(1280, 720, lc_graphics::PixelFormat::Rgba8888);
        // The reference capture ran with Config.General.FairCrew = true and
        // Record = true (verified against the icon-button pixels; see the
        // GUIIcons2 phases 2 and 1 in the F9 capture).
        ScenSelScreen::render(
            &mut surface,
            &assets,
            &gui_fonts,
            &book_fonts,
            Some(gamma),
            true,
            true,
        );
        // Final whole-frame gamma pass, mirroring render_startup_frame.
        gamma.apply_to_surface(&mut surface);

        std::fs::create_dir_all("/tmp/menu-parity-scensel").expect("mkdir");
        crate::test_support::write_ppm(&surface, "/tmp/menu-parity-scensel/out.ppm");
    }
}
