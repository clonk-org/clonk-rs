//! Pixel-parity renderer for `C4StartupScenSelDlg` — the scenario-selection
//! "book" (see `rust/target/parity-specs/scensel.md`). Implemented against the
//! engine's F9 reference capture at 1280x720; mirrors
//! `src/C4StartupScenSelDlg.cpp` (ctor layout, 1302-1382), `src/C4Gui.cpp`
//! (DrawBar/DrawVBar/Draw3DFrame, 264-345) and `src/C4Startup.cpp:92-116`
//! (shadowless book fonts).

use crate::clonk_fonts::{expand_hotkey_markup, ClonkFontSet};
use crate::classic_gui::{ClassicButtonState, ClassicGuiSkin};
use crate::startup_main_menu::{
    centered_label_tooltip_at, draw_bar, IntRect, StartupTooltip,
};
use crate::{draw_image_bilinear, draw_image_strip, ImageData};
use anyhow::{ensure, Context, Result};
use freetype::face::LoadFlag;
use freetype::Library;
use clonk_graphics::clonk_font::{line_height_for, ClonkFont, ClonkFontRole, GlyphCell, TextAlign};
use clonk_graphics::{
    BlitSampling, Color, GammaRamp, Rect as SurfaceRect, Surface, SurfaceDrawTarget,
};
use clonk_gui::Rect as GuiRect;
use std::{cell::RefCell, collections::HashMap};

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
    /// `GUIButtonHighlight.png` — additive half-size focus/hover marker on
    /// the checkbox square (C4GuiCheckBox.cpp:128-134).
    pub button_highlight: ImageData,
    /// `GUIIcons2.png` (256x320, 64x64 cells) — extended icons for the
    /// fair-crew/record icon buttons (C4Gui.h:734-751).
    pub icons_ex: ImageData,
    /// `StartupScenSelTitleOv.png` (220x170) — paper frame drawn over the
    /// right-page title picture (fctScenSelTitleOverlay, C4Startup.cpp;
    /// OverlayPicture border 10, C4StartupScenSelDlg.cpp:1361-1362).
    pub title_overlay: ImageData,
}

/// The two independent visual flags used by the scenario book's standard
/// `C4GUI::CallbackButton` controls. `highlighted` combines keyboard focus
/// and active-dialog pointer hover; `pressed` selects `GUIButtonDown.png`
/// and offsets the caption by one pixel.
pub type ScenSelButtonState = ClassicButtonState;

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
pub(crate) fn build_shadowless_font(
    face: &freetype::Face,
    px_height: u32,
) -> Result<ClonkFont> {
    face.set_pixel_sizes(px_height, px_height)
        .context("FT_Set_Pixel_Sizes failed")?;

    let raw = face.raw();
    let units_per_em = i32::from(raw.units_per_EM);
    let (ascender, descender) = (i32::from(raw.ascender), i32::from(raw.descender));
    let line_height = line_height_for(ascender, descender, units_per_em, px_height);
    let mut font = ClonkFont::new(line_height);
    font.set_texture_size(if px_height > 40 { 512 } else { 128 });
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
        title: build_shadowless_font(&face, 22)?.with_role(ClonkFontRole::BookTitle),
        caption: build_shadowless_font(&face, 16)?.with_role(ClonkFontRole::BookCaption),
        text: build_shadowless_font(&face, 14)?.with_role(ClonkFontRole::BookText),
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
    /// Client bounds of the zero-chrome tabular sheet that hosts either the
    /// normal book or a `FolderMap.txt` map (C4StartupScenSelDlg.cpp:
    /// 1313-1328).
    pub map_sheet: IntRect,
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

impl ScenSelLayout {
    /// Bounds passed to the embedded `C4GameOptionButtons` window after the
    /// Back, Open and Choose-definitions controls have been carved from the
    /// dialog's bottom aligner. Exposing the parent bounds (rather than only
    /// the two local-selector child rects) also supports the six-button
    /// network-host variant without duplicating `ComponentAligner` math in
    /// the application.
    pub fn game_option_bounds(&self) -> IntRect {
        let horizontal_margin = self.back_button.x - self.client.x;
        let x = self.back_button.x + self.back_button.w + 2 * horizontal_margin;
        let right = self.user_change_checkbox.x - 2 * horizontal_margin;
        let h = self.client.h / 8;
        IntRect {
            x,
            y: self.client.y + self.client.h - h,
            w: (right - x).max(0),
            h,
        }
    }
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
        map_sheet: IntRect {
            x: client.x + rc_map.x,
            y: client.y + rc_map.y,
            w: rc_map.w,
            h: rc_map.h,
        },
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

fn tooltip_rect_contains(rect: IntRect, point: crate::GuiPoint) -> bool {
    point.x >= rect.x as f32
        && point.y >= rect.y as f32
        && point.x < (rect.x + rect.w) as f32
        && point.y < (rect.y + rect.h) as f32
}

fn tooltip_gui_rect_contains(rect: &GuiRect, point: crate::GuiPoint) -> bool {
    // C4Rect(FLOAT_RECT) truncates the origin but sizes to the enclosing
    // floor(left)/ceil(right) span before regular half-open C4Rect hits.
    let bounds = IntRect {
        x: rect.origin.x as i32,
        y: rect.origin.y as i32,
        w: ((rect.origin.x + rect.size.width).ceil() - rect.origin.x.floor()) as i32,
        h: ((rect.origin.y + rect.size.height).ceil() - rect.origin.y.floor()) as i32,
    };
    tooltip_rect_contains(bounds, point)
}

/// Resolves one normal-book scenario selector target. `row_names` follows
/// the visible C++ list order, while `list_scroll_y` is the logical
/// ScrollWindow displacement. Rows override the list box's resource tooltip
/// with their live, unstripped scenario/folder name.
pub fn scen_sel_book_tooltip_at<'a>(
    layout: &ScenSelLayout,
    point: crate::GuiPoint,
    caption_extent: (i32, i32),
    list_scroll_y: i32,
    item_height: i32,
    row_names: impl IntoIterator<Item = &'a str>,
) -> Option<StartupTooltip> {
    if let Some(tooltip) = centered_label_tooltip_at(
        point,
        layout.caption_anchor,
        caption_extent,
        StartupTooltip::resource("IDS_DLGTIP_SELECTSCENARIO"),
    ) {
        return Some(tooltip);
    }
    for rect in [layout.search_label, layout.search_edit] {
        if tooltip_rect_contains(rect, point) {
            return Some(StartupTooltip::resource("IDS_DLGTIP_SEARCHLIST"));
        }
    }
    if tooltip_rect_contains(layout.back_button, point) {
        return Some(StartupTooltip::resource("IDS_DLGTIP_BACKMAIN"));
    }
    if tooltip_rect_contains(layout.open_button, point) {
        return Some(StartupTooltip::resource("IDS_DLGTIP_SCENSELNEXT"));
    }
    if !tooltip_rect_contains(layout.list, point) {
        return None;
    }

    let viewport = IntRect {
        x: layout.list.x + 3,
        y: layout.list.y + 3,
        w: layout.list.w - 6 - 16,
        h: layout.list.h - 6,
    };
    if tooltip_rect_contains(viewport, point) {
        let item_height = item_height.max(1);
        let pitch = item_height + 1;
        let local_y = point.y.floor() as i32 - viewport.y + list_scroll_y.max(0);
        if local_y >= 0 && local_y % pitch < item_height {
            let index = (local_y / pitch) as usize;
            if let Some(name) = row_names.into_iter().nth(index) {
                return Some(StartupTooltip::text(name));
            }
        }
    }
    Some(StartupTooltip::resource("IDS_DLGTIP_SELECTSCENARIO"))
}

/// One topmost map scenario button. Buttons without a resolved entry carry
/// no tooltip and still occlude a `MapPic` underneath, matching GUI hit-test
/// recursion rather than sibling fall-through.
#[derive(Clone, Debug)]
pub struct ScenSelMapScenarioTooltip<'a> {
    pub bounds: GuiRect,
    pub scenario_name: Option<&'a str>,
}

/// Resolves scenario-map targets. `untipped_foreground_bounds` contains the
/// selection TextWindow added after all map controls. `picture_bounds`
/// contains actual `MapPic` elements (the non-fullscreen background and all
/// authorized access overlays, including empty facets); a fullscreen dialog
/// background is deliberately omitted.
pub fn scen_sel_map_tooltip_at<'a>(
    layout: &ScenSelLayout,
    point: crate::GuiPoint,
    untipped_foreground_bounds: impl IntoIterator<Item = GuiRect>,
    picture_bounds: impl IntoIterator<Item = GuiRect>,
    scenario_buttons: impl IntoIterator<Item = ScenSelMapScenarioTooltip<'a>>,
) -> Option<StartupTooltip> {
    if tooltip_rect_contains(layout.back_button, point) {
        return Some(StartupTooltip::resource("IDS_DLGTIP_BACKMAIN"));
    }
    if tooltip_rect_contains(layout.open_button, point) {
        return Some(StartupTooltip::resource("IDS_DLGTIP_SCENSELNEXT"));
    }
    // The active Tabular sheet clips and owns every configured map child;
    // FLOAT_RECT areas outside rcMap cannot receive Screen::MouseInput.
    if !tooltip_rect_contains(layout.map_sheet, point) {
        return None;
    }
    if untipped_foreground_bounds
        .into_iter()
        .any(|bounds| tooltip_gui_rect_contains(&bounds, point))
    {
        return None;
    }
    for button in scenario_buttons {
        if tooltip_gui_rect_contains(&button.bounds, point) {
            return button.scenario_name.map(|name| {
                StartupTooltip::formatted_resource("IDS_MSG_MAP_STARTSCEN", [name])
            });
        }
    }
    picture_bounds
        .into_iter()
        .any(|bounds| tooltip_gui_rect_contains(&bounds, point))
        .then(|| StartupTooltip::resource("IDS_MSG_MAP_DESC"))
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
    if crate::draw_image_source_with_active_renderer_config(
        surface,
        &GuiRect::new(tx, ty, twdt, thgt),
        image,
        src,
        BlitSampling::Linear,
        gamma,
    ) {
        return;
    }
    if fwdt <= 0.0 || fhgt <= 0.0 || twdt <= 0.0 || thgt <= 0.0 {
        return;
    }
    if crate::capture_gpu_gui_image(
        surface,
        (tx, ty, twdt, thgt),
        image,
        crate::FloatSourceRect {
            x: fx,
            y: fy,
            width: fwdt,
            height: fhgt,
        },
        clonk_graphics::GpuSampler::Linear,
        crate::BilinearBlend::AlphaOver,
        None,
        gamma,
    ) {
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
                    if surface.is_gpu_scene_capture_active() {
                        // Fallback rasterization during capture must stay a
                        // painter-ordered retained fragment instead of
                        // blending against stale CPU backing.
                        let _ = surface.blend_fragment_over(px as u32, py as u32, s, gamma);
                        continue;
                    }
                    let af = (s[3] / 255.0).clamp(0.0, 1.0);
                    let dst = surface.get_pixel(px as u32, py as u32).unwrap_or_default();
                    let blend = |src: f32, dst: u8| -> u8 {
                        (encode_channel(gamma, src) * af + f32::from(dst) * (1.0 - af))
                            .round()
                            .clamp(0.0, 255.0) as u8
                    };
                    let out = Color::new(
                        blend(s[0], dst.r),
                        blend(s[1], dst.g),
                        blend(s[2], dst.b),
                        blend_surface_alpha(af, dst.a),
                    );
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
    if crate::active_advanced_renderer_config()
        .is_some_and(|config| config.blit_offset != 0 || config.no_box_fades)
    {
        if x2 < x1 || y2 < y1 {
            return;
        }
        let width = x2.saturating_sub(x1).saturating_add(1) as u32;
        let height = y2.saturating_sub(y1).saturating_add(1) as u32;
        crate::draw_color_rect(
            surface,
            SurfaceRect::new(x1, y1, width, height),
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
    draw_box_dw_unconfigured(surface, x1, y1, x2, y2, clr, gamma);
}

/// Compatibility box rasterizer used outside a configured renderer scope.
fn draw_box_dw_unconfigured(
    surface: &mut Surface,
    x1: i32,
    y1: i32,
    x2: i32,
    y2: i32,
    clr: u32,
    gamma: Option<&GammaRamp>,
) {
    if surface.is_gpu_scene_capture_active() {
        if x2 < x1 || y2 < y1 {
            return;
        }
        let color = Color::new(
            (clr >> 16) as u8,
            (clr >> 8) as u8,
            clr as u8,
            255 - (clr >> 24) as u8,
        );
        if color.a == 0 {
            return;
        }
        crate::record_gpu_solid_quad(
            surface,
            (
                x1 as f32,
                y1 as f32,
                x2.saturating_add(1) as f32,
                y2.saturating_add(1) as f32,
            ),
            [color; 4],
            clonk_graphics::GpuBlend::Normal,
            gamma.is_some_and(|gamma| !gamma.is_passthrough()),
        );
        return;
    }
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
                blend_surface_alpha(opacity, dst.a),
            );
            let _ = surface.set_pixel(x as u32, y as u32, out);
        }
    }
}

fn encode_channel(gamma: Option<&GammaRamp>, x: f32) -> f32 {
    gamma
        .map(|ramp| f32::from(ramp.encode_float(x)))
        .unwrap_or_else(|| x.round().clamp(0.0, 255.0))
}

fn blend_surface_alpha(opacity: f32, destination: u8) -> u8 {
    (255.0 * opacity + f32::from(destination) * (1.0 - opacity))
        .round()
        .clamp(0.0, 255.0) as u8
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
    if surface.is_gpu_scene_capture_active() {
        // Keep native line geometry until the backend knows the physical
        // viewport. Projecting a one-logical-pixel box is not equivalent to
        // glLineWidth(Application.GetScale()) at fractional scales.
        crate::classic_gui::draw_engine_line(surface, x1, y1, x2, y2, clr, gamma);
        return;
    }

    if x1 == x2 {
        // vertical: y1..y2 excluding the end pixel
        draw_box_dw_unconfigured(surface, x1, y1.min(y2), x1, y1.max(y2) - 1, clr, gamma);
    } else {
        debug_assert_eq!(y1, y2);
        draw_box_dw_unconfigured(surface, x1.min(x2), y1, x1.max(x2) - 1, y1, clr, gamma);
    }
}

fn with_surface_clip(
    surface: &mut Surface,
    requested: (i64, i64, i64, i64),
    draw: impl FnOnce(&mut Surface),
) {
    let previous = surface.clip();
    let mut left = requested.0.max(0);
    let mut top = requested.1.max(0);
    let mut right = requested
        .2
        .min(i64::from(surface.width().min(i32::MAX as u32)));
    let mut bottom = requested
        .3
        .min(i64::from(surface.height().min(i32::MAX as u32)));
    if let Some(clip) = previous {
        left = left.max(i64::from(clip.x));
        top = top.max(i64::from(clip.y));
        right = right.min(i64::from(clip.x) + i64::from(clip.width));
        bottom = bottom.min(i64::from(clip.y) + i64::from(clip.height));
    }
    if left < right && top < bottom {
        surface.set_clip(clonk_graphics::Rect::new(
            left as i32,
            top as i32,
            (right - left) as u32,
            (bottom - top) as u32,
        ));
        draw(surface);
    }
    match previous {
        Some(clip) => surface.set_clip(clip),
        None => surface.clear_clip(),
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
/// rendered under the requested primary clipper.
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
    let (cx1, cy1, cx2, cy2) = clip;
    if cx2 < cx1 || cy2 < cy1 {
        return;
    }
    with_surface_clip(
        surface,
        (
            i64::from(cx1),
            i64::from(cy1),
            i64::from(cx2) + 1,
            i64::from(cy2) + 1,
        ),
        |surface| font.draw_with_gamma(surface, x, y, text, color, align, markup, gamma),
    );
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
        draw_user_change_checkbox(surface, &layout, assets, gui_fonts, false, false, false, gamma);
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
        Self::render_chrome_impl(
            surface,
            assets,
            gui_fonts,
            Some("Start Game"),
            true,
            Some((fair_crew, record)),
            gamma,
        );
    }

    /// Selection-independent chrome for an application that owns the
    /// recursive bottom controls. Unlike [`Self::render_chrome`], this does
    /// not bake a released Back plank or the Fair Crew/Record icon bases into
    /// the cached backdrop. The caller must draw Back once with
    /// [`draw_back_button_with_state`] and render its `C4GameOptionButtons`
    /// once after restoring the backdrop. `title` supports both "Start Game"
    /// and the network selector's "Start Network Game" without repainting a
    /// rectangle from the background.
    pub fn render_chrome_without_game_options(
        surface: &mut Surface,
        assets: &ScenSelAssets,
        gui_fonts: &ClonkFontSet,
        title: &str,
        gamma: Option<&GammaRamp>,
    ) {
        Self::render_chrome_impl(surface, assets, gui_fonts, Some(title), false, None, gamma);
    }

    /// Raster-only selection-independent chrome for the application's static
    /// backdrop cache. CStdFont commands are intentionally excluded because
    /// scale-native capture stores them outside the logical pixel surface;
    /// callers must emit them every frame with [`Self::draw_chrome_text`].
    pub fn render_backdrop_without_game_options(
        surface: &mut Surface,
        assets: &ScenSelAssets,
        gui_fonts: &ClonkFontSet,
        gamma: Option<&GammaRamp>,
    ) {
        Self::render_chrome_impl(surface, assets, gui_fonts, None, false, None, gamma);
    }

    /// Draws the fullscreen base title and the static CStdFont label owned by
    /// C4StartupScenSelDlg. They remain outside the raster backdrop cache so
    /// native-scale capture receives one title and wooden-search-label command
    /// on every frame.
    pub fn draw_chrome_text(
        surface: &mut Surface,
        gui_fonts: &ClonkFontSet,
        title: &str,
        gamma: Option<&GammaRamp>,
    ) {
        let layout = scen_sel_layout(surface.width() as i32, surface.height() as i32, gui_fonts);
        let yellow = [255, 255, 0, 255]; // C4GUI_Caption2FontClr / ButtonFontClr

        // Fullscreen title (IDS_DLG_STARTGAME/IDS_DLG_STARTNETWORKGAME):
        // GUI TitleFont, C4GUI_FullscreenCaptionFontClr yellow, ACenter
        // (C4GuiDialogs.cpp:846, C4GuiLabels.cpp:34-37).
        gui_fonts.title.draw_with_gamma(
            surface,
            layout.title_anchor.0,
            layout.title_anchor.1,
            title,
            yellow,
            TextAlign::Center,
            true,
            gamma,
        );

        // WoodenLabel text (C4GuiLabels.cpp:168-209): GUI TextFont yellow,
        // centered one pixel above the vertical midpoint and label-clipped.
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
    }

    fn render_chrome_impl(
        surface: &mut Surface,
        assets: &ScenSelAssets,
        gui_fonts: &ClonkFontSet,
        static_title: Option<&str>,
        draw_back: bool,
        game_options: Option<(bool, bool)>,
        gamma: Option<&GammaRamp>,
    ) {
        let layout = scen_sel_layout(surface.width() as i32, surface.height() as i32, gui_fonts);

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

        // 4. WoodenLabel raster (C4GuiLabels.cpp:168-209): zoomed
        // barCaption wood. Its scale-native text is emitted outside this
        // cacheable layer by `draw_chrome_text`.
        draw_caption_bar(surface, &layout.search_label, &assets.caption_bar, gamma);
        if let Some(title) = static_title {
            // Preserve the public renderer's C++ text order: title/search
            // precede the edit, list and recursive bottom controls. The app's
            // cached path passes `None` and emits the same pair after restore.
            Self::draw_chrome_text(surface, gui_fonts, title, gamma);
        }

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
        if draw_back {
            draw_button(surface, &layout.back_button, "Back", assets, gui_fonts, gamma);
        }

        // Icon buttons (IconButton::DrawElement, C4GuiButton.cpp:205-232):
        // plain 64x64 icon blit, no highlight without focus/hover.
        // Icons from GUIIcons2 (Icon::GetIconFacet, C4GuiLabels.cpp:441-450):
        // fair crew = Ico_Ex_FairCrew(2)/Ico_Ex_NormalCrew(3), record =
        // Ico_Ex_RecordOn(1)/Ico_Ex_RecordOff(0).
        if let Some((fair_crew, record)) = game_options {
            let icon_ex = |idx: u32| ((idx % 4) * 64, (idx / 4) * 64);
            let (fc_x, fc_y) = icon_ex(if fair_crew { 2 } else { 3 });
            let fc = &layout.fair_crew_button;
            draw_image_strip(surface, fc.x, fc.y, &assets.icons_ex, fc_x, fc_y, 64, 64, gamma);
            let (rec_x, rec_y) = icon_ex(if record { 1 } else { 0 });
            let rec = &layout.record_button;
            draw_image_strip(surface, rec.x, rec.y, &assets.icons_ex, rec_x, rec_y, 64, 64, gamma);
        }
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

/// Draws `C4StartupScenSelDlg::pScenSelProgressLabel` while recursive
/// scenario discovery owns the book. Native anchors the label at the list's
/// horizontal midpoint and (quirkily) derives its y coordinate from that
/// same midpoint (`C4StartupScenSelDlg.cpp:1357`).
pub fn draw_loading_label(
    surface: &mut Surface,
    layout: &ScenSelLayout,
    gui_fonts: &ClonkFontSet,
    book_fonts: &BookFontSet,
    text: &str,
    gamma: Option<&GammaRamp>,
) {
    let middle_x = layout.list.x + layout.list.w / 2;
    let local_middle_x = layout.list.x - layout.map_sheet.x + layout.list.w / 2;
    book_fonts.caption.draw_with_gamma(
        surface,
        middle_x,
        layout.map_sheet.y + local_middle_x - gui_fonts.caption.line_height / 2,
        text,
        [0, 0, 0, 255],
        TextAlign::Center,
        false,
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
    caret: usize,
    selection: Option<(usize, usize)>,
    horizontal_scroll: i32,
    cursor_visible: bool,
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
    let client = IntRect {
        x: edit.x + 4,
        y: edit.y + 2,
        w: (edit.w - 8).max(0),
        h: (edit.h - 4).max(0),
    };
    let clip = IntRect {
        x: client.x - 2,
        y: client.y,
        w: client.w + 4,
        h: client.h + 1,
    };
    let (text_y0, selection_height) = if client.h <= gui_fonts.text.line_height {
        (client.y, client.h)
    } else {
        (
            client.y + (client.h - gui_fonts.text.line_height) / 2 + 1,
            gui_fonts.text.line_height - 2,
        )
    };
    if let Some((selection_start, selection_end)) = selection {
        let selection_start = selection_start.min(text.len());
        let selection_end = selection_end.min(text.len());
        if selection_start < selection_end
            && text.is_char_boundary(selection_start)
            && text.is_char_boundary(selection_end)
        {
            let x1 = client.x
                + gui_fonts.text.measure(&text[..selection_start], false).0
                - horizontal_scroll;
            let x2 = client.x
                + gui_fonts.text.measure(&text[..selection_end], false).0
                - horizontal_scroll;
            let clipped_x1 = x1.max(clip.x);
            let clipped_x2 = (x2 - 1).min(clip.x + clip.w - 1);
            if clipped_x1 <= clipped_x2 {
                draw_box_dw(
                    surface,
                    clipped_x1,
                    text_y0,
                    clipped_x2,
                    text_y0 + selection_height - 1,
                    0x7f7f7f00,
                    gamma,
                );
            }
        }
    }
    draw_text_clipped(
        surface,
        &gui_fonts.text,
        client.x - horizontal_scroll,
        text_y0 - 1,
        text,
        [255, 255, 255, 255],
        TextAlign::Left,
        false,
        gamma,
        (
            clip.x,
            clip.y,
            clip.x + clip.w - 1,
            clip.y + clip.h - 1,
        ),
    );
    if cursor_visible {
        let caret = caret.min(text.len());
        let caret = if text.is_char_boundary(caret) { caret } else { 0 };
        let cursor_x = client.x + gui_fonts.text.measure(&text[..caret], false).0
            - gui_fonts.text.measure("\u{a6}", false).0 / 2
            - horizontal_scroll;
        draw_scaled_search_caret(
            surface,
            &gui_fonts.text,
            cursor_x,
            text_y0 - gui_fonts.text.line_height / 3,
            clip,
            gamma,
        );
    }
}

/// `Edit::DrawElement` renders the reserved broken-bar glyph through
/// `TextOut(..., 1.5f)`. Keep the glyph in a padded atlas tile so the shared
/// facet blitter reproduces the native linear filtering at that scale.
fn draw_scaled_search_caret(
    surface: &mut Surface,
    font: &ClonkFont,
    x: i32,
    y: i32,
    clip: IntRect,
    gamma: Option<&GammaRamp>,
) {
    const SCALE: f32 = 1.5;
    let Some(glyph) = font.glyph('\u{a6}') else {
        return;
    };
    let Ok(width) = u32::try_from(glyph.width) else {
        return;
    };
    let Ok(height) = u32::try_from(font.cell_height) else {
        return;
    };
    if width == 0 || height == 0 || glyph.pixels.len() != width as usize * height as usize {
        return;
    }

    let atlas_width = width.max(height).next_power_of_two();
    let mut glyph_hash = 0xcbf2_9ce4_8422_2325_u64;
    for pixel in &glyph.pixels {
        for byte in [pixel.r, pixel.g, pixel.b, pixel.a] {
            glyph_hash = (glyph_hash ^ u64::from(byte)).wrapping_mul(0x0000_0100_0000_01b3);
        }
    }
    thread_local! {
        /// The caret atlas is immutable for a given rasterized font. Reusing
        /// its ImageData identity keeps the retained renderer from allocating
        /// a fresh GPU texture cache entry on every blinking frame.
        static CARET_ATLASES: RefCell<HashMap<(u32, u32, u64), ImageData>> =
            RefCell::new(HashMap::new());
    }
    let image = CARET_ATLASES.with(|atlases| {
        let key = (width, height, glyph_hash);
        if let Some(image) = atlases.borrow().get(&key).cloned() {
            return image;
        }
        let mut pixels = vec![255_u8; atlas_width as usize * height as usize * 4];
        for pixel in pixels.chunks_exact_mut(4) {
            pixel[3] = 0;
        }
        for row in 0..height as usize {
            for column in 0..width as usize {
                let pixel = glyph.pixels[row * width as usize + column];
                let destination = (row * atlas_width as usize + column) * 4;
                let (red, green, blue) = if pixel.a == 0 {
                    (255, 255, 255)
                } else {
                    (pixel.r, pixel.g, pixel.b)
                };
                pixels[destination..destination + 4].copy_from_slice(&[red, green, blue, pixel.a]);
            }
        }
        let image = ImageData::new(atlas_width, height, pixels);
        atlases.borrow_mut().insert(key, image.clone());
        image
    });
    let destination = (
        x as f32,
        y as f32,
        width as f32 * SCALE,
        height as f32 * SCALE,
    );
    let left = destination.0.max(clip.x as f32);
    let top = destination.1.max(clip.y as f32);
    let right = (destination.0 + destination.2).min((clip.x + clip.w) as f32);
    let bottom = (destination.1 + destination.3).min((clip.y + clip.h) as f32);
    if left >= right || top >= bottom {
        return;
    }
    draw_facet_stretch(
        surface,
        &image,
        (
            (left - destination.0) / SCALE,
            (top - destination.1) / SCALE,
            (right - left) / SCALE,
            (bottom - top) / SCALE,
        ),
        (left, top, right - left, bottom - top),
        gamma,
    );
}

/// The Open/Start button with its selection-specific text — "Open"
/// (IDS_BTN_OPEN) for folders/none, "&Start" (IDS_BTN_STARTGAME) for
/// scenarios (Entry::GetOpenText, C4StartupScenSelDlg.cpp:794-797,926-929;
/// applied in UpdateSelection, :1587).
///
/// This compatibility helper renders the released, unhighlighted state. New
/// interactive callers should use [`draw_open_button_with_state`].
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

/// Validates the three exact classic resources needed to render a dynamic
/// Back/Open `C4GUI::CallbackButton`. The down plank is deliberately passed
/// separately so adding dynamic state does not break existing
/// [`ScenSelAssets`] struct literals and cached reference renderers.
pub fn validate_scensel_button_assets(
    assets: &ScenSelAssets,
    button_down: &ImageData,
) -> Result<()> {
    ensure!(
        (assets.button.width(), assets.button.height()) == (128, 32),
        "GUIButton.png must be the exact 128x32 classic plank: got {}x{}",
        assets.button.width(),
        assets.button.height()
    );
    ensure!(
        (button_down.width(), button_down.height()) == (128, 32),
        "GUIButtonDown.png must be the exact 128x32 classic plank: got {}x{}",
        button_down.width(),
        button_down.height()
    );
    ensure!(
        (
            assets.button_highlight.width(),
            assets.button_highlight.height()
        ) == (16, 16),
        "GUIButtonHighlight.png must be the exact 16x16 classic facet: got {}x{}",
        assets.button_highlight.width(),
        assets.button_highlight.height()
    );
    Ok(())
}

#[allow(clippy::too_many_arguments)]
fn draw_callback_button_with_state(
    surface: &mut Surface,
    rect: IntRect,
    text: &str,
    assets: &ScenSelAssets,
    button_down: &ImageData,
    gui_fonts: &ClonkFontSet,
    state: ScenSelButtonState,
    gamma: Option<&GammaRamp>,
) -> Result<()> {
    validate_scensel_button_assets(assets, button_down)?;
    ClassicGuiSkin::new(
        &assets.caption_bar,
        &assets.button,
        button_down,
        Some(&assets.button_highlight),
    )
    .draw_button(surface, rect, text, gui_fonts, state, gamma);
    Ok(())
}

/// Draws the Back callback button with exact released/down plank, additive
/// focus-or-hover highlight and one-pixel pressed caption offset.
#[allow(clippy::too_many_arguments)]
pub fn draw_back_button_with_state(
    surface: &mut Surface,
    layout: &ScenSelLayout,
    text: &str,
    assets: &ScenSelAssets,
    button_down: &ImageData,
    gui_fonts: &ClonkFontSet,
    state: ScenSelButtonState,
    gamma: Option<&GammaRamp>,
) -> Result<()> {
    draw_callback_button_with_state(
        surface,
        layout.back_button,
        text,
        assets,
        button_down,
        gui_fonts,
        state,
        gamma,
    )
}

/// Draws the selection-specific Open/Start callback button with exact
/// released/down plank, additive focus-or-hover highlight and one-pixel
/// pressed caption offset. Hotkey markup in `text` is expanded by the shared
/// classic button renderer.
#[allow(clippy::too_many_arguments)]
pub fn draw_open_button_with_state(
    surface: &mut Surface,
    layout: &ScenSelLayout,
    text: &str,
    assets: &ScenSelAssets,
    button_down: &ImageData,
    gui_fonts: &ClonkFontSet,
    state: ScenSelButtonState,
    gamma: Option<&GammaRamp>,
) -> Result<()> {
    draw_callback_button_with_state(
        surface,
        layout.open_button,
        text,
        assets,
        button_down,
        gui_fonts,
        state,
        gamma,
    )
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
    highlighted: bool,
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
    if highlighted {
        let highlight_size = cb.h / 2;
        crate::draw_image_bilinear_additive(
            surface,
            &GuiRect::new(
                (cb.x + cb.h / 4) as f32,
                (cb.y + cb.h / 4) as f32,
                highlight_size as f32,
                highlight_size as f32,
            ),
            &assets.button_highlight,
            gamma,
        );
    }
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

/// The `C4GUI::ScrollBar` rectangle owned by the right-page TextWindow.
/// It is always reserved by the ScrollWindow and only drawn/hit-tested when
/// the content exceeds the visible client height.
pub fn selection_info_scrollbar_rect(layout: &ScenSelLayout) -> IntRect {
    let (client, _) = selection_info_client(layout);
    IntRect {
        x: client.x + client.w - 16,
        y: client.y,
        w: 16,
        h: client.h,
    }
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
    let mut y = client.y - scroll_y;
    with_surface_clip(
        surface,
        (
            i64::from(client.x),
            i64::from(client.y),
            i64::from(client.x) + i64::from(content_w),
            i64::from(client.y) + i64::from(client.h),
        ),
        |surface| {
            if let Some(picture) = info.picture {
                let pic_w = 220.min(content_w);
                let pic_h = 170 * pic_w / 220;
                let pic_x = client.x + (content_w / 2 - 220 / 2).max(0);
                let overlay = &assets.title_overlay;
                let inset_x = 10 * pic_w / overlay.width().max(1) as i32;
                let inset_y = 10 * pic_h / overlay.height().max(1) as i32;
                draw_facet_stretch(
                    surface,
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
                    surface,
                    overlay,
                    (0.0, 0.0, overlay.width() as f32, overlay.height() as f32),
                    (pic_x as f32, y as f32, pic_w as f32, pic_h as f32),
                    gamma,
                );
                y += pic_h + 10;
            }

            for (text, font, color) in selection_info_lines(info, book_fonts) {
                for wrapped in wrap_line(&text, font, content_w) {
                    font.draw_with_gamma(
                        surface,
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
        },
    );

    // Book scrollbar track + fixed 16px pin on overflow
    // (C4GuiContainers.cpp:343-368,446-473).
    if metrics.max_scroll > 0 {
        let bar_x = selection_info_scrollbar_rect(layout).x;
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

    fn test_assets() -> ScenSelAssets {
        let load = crate::test_support::load_graphics_png;
        ScenSelAssets {
            background: load("StartupScenSelBG.png"),
            book_scroll: load("StartupBookScroll.png"),
            scen_icons: load("StartupScenSelIcons.png"),
            caption_bar: load("GUICaption.png"),
            button: load("GUIButton.png"),
            checkbox: load("GUICheckbox.png"),
            button_highlight: load("GUIButtonHighlight.png"),
            icons_ex: load("GUIIcons2.png"),
            title_overlay: load("StartupScenSelTitleOv.png"),
        }
    }

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
        assert_eq!(fonts.title.role(), Some(ClonkFontRole::BookTitle));
        assert_eq!(fonts.caption.role(), Some(ClonkFontRole::BookCaption));
        assert_eq!(fonts.text.role(), Some(ClonkFontRole::BookText));
    }

    #[test]
    fn retained_scenario_image_and_box_are_two_commands() {
        let image = ImageData::new(4, 4, [30, 60, 90, 255].repeat(16));
        let mut surface = Surface::new(220, 160, clonk_graphics::PixelFormat::Rgba8888);
        surface.begin_gpu_scene_capture();
        draw_facet_stretch(
            &mut surface,
            &image,
            (0.0, 0.0, 4.0, 4.0),
            (10.0, 12.0, 200.0, 130.0),
            None,
        );
        draw_box_dw(&mut surface, 8, 10, 211, 143, 0x7f20_4060, None);

        let scene = surface
            .take_gpu_scene_capture()
            .expect("capture remains active")
            .into_scene([220, 160], Color::transparent(), &GammaRamp::identity());
        assert_eq!(scene.commands.len(), 2);
        let clonk_graphics::GpuCommand::Quad { sampler, .. } = &scene.commands[0] else {
            panic!("scenario picture was not retained as a texture quad");
        };
        assert_eq!(*sampler, clonk_graphics::GpuSampler::Linear);
        assert!(matches!(
            &scene.commands[1],
            clonk_graphics::GpuCommand::Solid { .. }
        ));
    }

    #[test]
    fn retained_scensel_line_defers_scaled_diamond_rasterization() {
        let _renderer = crate::activate_advanced_renderer_config(crate::AdvancedRendererConfig {
            blit_offset: 100,
            ..crate::AdvancedRendererConfig::DEFAULT
        });
        let mut surface = Surface::new(8, 8, clonk_graphics::PixelFormat::Rgba8888);
        surface.begin_gpu_scene_capture();
        draw_line_dw(&mut surface, 3, 6, 3, 1, 0x7f20_4060, None);

        let scene = surface
            .take_gpu_scene_capture()
            .expect("capture remains active")
            .into_scene([8, 8], Color::transparent(), &GammaRamp::identity());
        let [clonk_graphics::GpuCommand::Solid {
            vertices,
            topology,
            alpha_mode,
            ..
        }] = scene.commands.as_slice()
        else {
            panic!("DrawLineDw was lowered before presentation scale was known");
        };
        assert_eq!(*topology, clonk_graphics::GpuPrimitiveTopology::LineList);
        assert_eq!(*alpha_mode, clonk_graphics::GpuSolidAlphaMode::SourceOver);
        assert_eq!(vertices.len(), 2);
        assert_eq!(vertices[0].position, [3.5, 6.5, 1.0]);
        assert_eq!(vertices[1].position, [3.5, 1.5, 1.0]);
    }

    #[test]
    fn retained_search_caret_reuses_texture_identity() {
        let fonts = endeavour_font_set();
        let render = || {
            let mut surface = Surface::new(80, 40, clonk_graphics::PixelFormat::Rgba8888);
            surface.begin_gpu_scene_capture();
            draw_scaled_search_caret(
                &mut surface,
                &fonts.text,
                10,
                8,
                IntRect {
                    x: 0,
                    y: 0,
                    w: 80,
                    h: 40,
                },
                None,
            );
            surface
                .take_gpu_scene_capture()
                .expect("capture remains active")
                .into_scene([80, 40], Color::transparent(), &GammaRamp::identity())
        };
        let first = render();
        let second = render();
        assert_eq!(first.commands.len(), 1);
        assert_eq!(second.commands.len(), 1);
        assert_eq!(first.textures[0].id, second.textures[0].id);
    }

    #[test]
    fn transparent_startup_primitives_preserve_source_over_alpha() {
        let mut surface = Surface::new(1, 1, clonk_graphics::PixelFormat::Rgba8888);
        draw_box_dw(&mut surface, 0, 0, 0, 0, 0x7f80_4020, None);
        assert_eq!(surface.get_pixel(0, 0), Some(Color::new(64, 32, 16, 128)));
    }

    #[test]
    fn clipped_book_text_capture_keeps_global_coordinates_and_clip() {
        let ttf =
            std::fs::read(crate::test_support::repo_root().join("planet/System.c4g/Endeavour.ttf"))
                .expect("read Endeavour.ttf");
        let fonts = build_book_font_set(&ttf).expect("book fonts");
        let mut surface = Surface::new(40, 30, clonk_graphics::PixelFormat::Rgba8888);
        let outer = clonk_graphics::Rect::new(5, 2, 20, 20);
        surface.set_clip(outer);
        surface.begin_clonk_text_capture();
        draw_text_clipped(
            &mut surface,
            &fonts.text,
            7,
            6,
            "Book",
            [0, 0, 0, 255],
            TextAlign::Left,
            true,
            None,
            (2, 4, 11, 13),
        );

        assert_eq!(surface.clip(), Some(outer));
        let commands = surface.take_clonk_text_capture();
        assert_eq!(commands.len(), 1);
        assert_eq!((commands[0].x, commands[0].y), (7, 6));
        assert_eq!(commands[0].clip, Some(clonk_graphics::Rect::new(5, 4, 7, 10)));
    }

    #[test]
    fn loading_label_uses_the_classic_book_caption_anchor() {
        let ttf = std::fs::read(
            crate::test_support::repo_root().join("planet/System.c4g/Endeavour.ttf"),
        )
        .expect("read Endeavour.ttf");
        let book_fonts = build_book_font_set(&ttf).expect("book fonts");
        let fonts = endeavour_font_set();
        let layout = scen_sel_layout(1280, 720, &fonts);
        let mut surface = Surface::new(1280, 720, clonk_graphics::PixelFormat::Rgba8888);
        surface.begin_clonk_text_capture();

        draw_loading_label(
            &mut surface,
            &layout,
            &fonts,
            &book_fonts,
            "Loading... (37%)",
            None,
        );

        let commands = surface.take_clonk_text_capture();
        assert_eq!(commands.len(), 1);
        assert_eq!(commands[0].text, "Loading... (37%)");
        assert_eq!((commands[0].x, commands[0].y), (374, 387));
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
        assert_eq!(
            layout.game_option_bounds(),
            IntRect {
                x: 208,
                y: 627,
                w: 691,
                h: 74,
            }
        );

        // Pin the measured font widths so regressions in the font code are
        // caught here: W = 3 * caption("<< BACK") = 3*51, S = text("Search:").
        assert_eq!(w, 153);
        assert_eq!(s, 46);
    }

    #[test]
    fn book_tooltip_targets_preserve_dynamic_rows_and_parent_fallback() {
        let fonts = endeavour_font_set();
        let layout = scen_sel_layout(1280, 720, &fonts);
        let caption_extent = (
            fonts.title.measure("Scenarios", true).0,
            fonts.title.line_height,
        );
        let rows = ["<c ff0000>Mission</c>", "Folder"];
        let center = |rect: IntRect| {
            crate::GuiPoint::new(
                (rect.x + rect.w / 2) as f32,
                (rect.y + rect.h / 2) as f32,
            )
        };

        assert_eq!(
            scen_sel_book_tooltip_at(
                &layout,
                crate::GuiPoint::new(
                    layout.caption_anchor.0 as f32,
                    layout.caption_anchor.1 as f32
                ),
                caption_extent,
                0,
                26,
                rows,
            ),
            Some(StartupTooltip::resource("IDS_DLGTIP_SELECTSCENARIO"))
        );
        for rect in [layout.search_label, layout.search_edit] {
            assert_eq!(
                scen_sel_book_tooltip_at(
                    &layout,
                    center(rect),
                    caption_extent,
                    0,
                    26,
                    rows,
                ),
                Some(StartupTooltip::resource("IDS_DLGTIP_SEARCHLIST"))
            );
        }
        let first_row = crate::GuiPoint::new(
            (layout.list.x + 10) as f32,
            (layout.list.y + 3 + 13) as f32,
        );
        assert_eq!(
            scen_sel_book_tooltip_at(
                &layout,
                first_row,
                caption_extent,
                0,
                26,
                rows,
            ),
            Some(StartupTooltip::text("<c ff0000>Mission</c>"))
        );
        assert_eq!(
            scen_sel_book_tooltip_at(
                &layout,
                first_row,
                caption_extent,
                27,
                26,
                rows,
            ),
            Some(StartupTooltip::text("Folder"))
        );
        let gap = crate::GuiPoint::new(
            (layout.list.x + 10) as f32,
            (layout.list.y + 3 + 26) as f32,
        );
        for point in [gap, center(layout.list_scrollbar)] {
            assert_eq!(
                scen_sel_book_tooltip_at(
                    &layout,
                    point,
                    caption_extent,
                    0,
                    26,
                    rows,
                ),
                Some(StartupTooltip::resource("IDS_DLGTIP_SELECTSCENARIO"))
            );
        }
        let blank_item_band = crate::GuiPoint::new(
            (layout.list.x + 10) as f32,
            (layout.list.y + 3 + 2 * 27 + 13) as f32,
        );
        assert_eq!(
            scen_sel_book_tooltip_at(
                &layout,
                blank_item_band,
                caption_extent,
                0,
                26,
                rows,
            ),
            Some(StartupTooltip::resource("IDS_DLGTIP_SELECTSCENARIO"))
        );
        assert_eq!(
            scen_sel_book_tooltip_at(
                &layout,
                center(layout.back_button),
                caption_extent,
                0,
                26,
                rows,
            ),
            Some(StartupTooltip::resource("IDS_DLGTIP_BACKMAIN"))
        );
        assert_eq!(
            scen_sel_book_tooltip_at(
                &layout,
                center(layout.open_button),
                caption_extent,
                0,
                26,
                rows,
            ),
            Some(StartupTooltip::resource("IDS_DLGTIP_SCENSELNEXT"))
        );
        assert_eq!(
            scen_sel_book_tooltip_at(
                &layout,
                center(layout.user_change_checkbox),
                caption_extent,
                0,
                26,
                rows,
            ),
            None
        );
    }

    #[test]
    fn map_tooltip_targets_respect_button_occlusion_and_dynamic_name() {
        let fonts = endeavour_font_set();
        let layout = scen_sel_layout(1280, 720, &fonts);
        let picture = GuiRect::new(100.0, 100.0, 100.0, 100.0);
        let button = GuiRect::new(120.0, 120.0, 30.0, 30.0);
        let picture_point = crate::GuiPoint::new(105.0, 105.0);
        let button_point = crate::GuiPoint::new(125.0, 125.0);
        assert_eq!(
            scen_sel_map_tooltip_at(
                &layout,
                picture_point,
                std::iter::empty(),
                [picture],
                std::iter::empty(),
            ),
            Some(StartupTooltip::resource("IDS_MSG_MAP_DESC"))
        );
        assert_eq!(
            scen_sel_map_tooltip_at(
                &layout,
                button_point,
                std::iter::empty(),
                [picture],
                [ScenSelMapScenarioTooltip {
                    bounds: button,
                    scenario_name: Some("The Mine"),
                }],
            ),
            Some(StartupTooltip::formatted_resource(
                "IDS_MSG_MAP_STARTSCEN",
                ["The Mine"]
            ))
        );
        assert_eq!(
            scen_sel_map_tooltip_at(
                &layout,
                button_point,
                std::iter::empty(),
                [picture],
                [ScenSelMapScenarioTooltip {
                    bounds: button,
                    scenario_name: None,
                }],
            ),
            None,
            "an untipped button blocks the MapPic sibling beneath it"
        );
        assert_eq!(
            scen_sel_map_tooltip_at(
                &layout,
                button_point,
                [button],
                [picture],
                [ScenSelMapScenarioTooltip {
                    bounds: button,
                    scenario_name: Some("The Mine"),
                }],
            ),
            None,
            "the selection TextWindow is added last and blocks map siblings"
        );
        assert_eq!(
            scen_sel_map_tooltip_at(
                &layout,
                crate::GuiPoint::new(10.0, 10.0),
                std::iter::empty(),
                std::iter::empty(),
                std::iter::empty(),
            ),
            None,
            "a fullscreen map background is not a MapPic tooltip target"
        );
        assert_eq!(
            scen_sel_map_tooltip_at(
                &layout,
                crate::GuiPoint::new(
                    (layout.map_sheet.x - 1) as f32,
                    (layout.map_sheet.y + 11) as f32,
                ),
                std::iter::empty(),
                [GuiRect::new(
                    (layout.map_sheet.x - 5) as f32,
                    (layout.map_sheet.y + 10) as f32,
                    10.0,
                    10.0,
                )],
                std::iter::empty(),
            ),
            None,
            "the active Tabular sheet clips every map descendant"
        );

        let fractional = GuiRect::new(
            layout.map_sheet.x as f32 + 10.75,
            layout.map_sheet.y as f32 + 20.25,
            2.1,
            3.1,
        );
        assert_eq!(
            scen_sel_map_tooltip_at(
                &layout,
                crate::GuiPoint::new(
                    (layout.map_sheet.x + 10) as f32,
                    (layout.map_sheet.y + 20) as f32,
                ),
                std::iter::empty(),
                [fractional],
                std::iter::empty(),
            ),
            Some(StartupTooltip::resource("IDS_MSG_MAP_DESC")),
            "FLOAT_RECT conversion truncates the origin"
        );
        assert_eq!(
            scen_sel_map_tooltip_at(
                &layout,
                crate::GuiPoint::new(
                    (layout.map_sheet.x + 13) as f32,
                    (layout.map_sheet.y + 20) as f32,
                ),
                std::iter::empty(),
                [fractional],
                std::iter::empty(),
            ),
            None,
            "the ceil(right)-floor(left) width remains half-open"
        );
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

        let mut surface = clonk_graphics::Surface::new(120, 30, clonk_graphics::PixelFormat::Rgba8888);
        draw_scen_list_item(&mut surface, &icons, &book_fonts.text, None, 0, 0, 1, "I", true);
        // Icon stretched over the full 26x26 picture rect: pure cell-1 green
        // at the center, nothing at column 26.
        assert_eq!(
            surface.get_pixel(13, 13),
            Some(clonk_graphics::Color::new(0, 255, 0, 255))
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
        let mut disabled = clonk_graphics::Surface::new(120, 30, clonk_graphics::PixelFormat::Rgba8888);
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

    #[test]
    fn search_edit_render_tracks_selection_caret_and_horizontal_scroll() {
        let fonts = endeavour_font_set();
        let layout = scen_sel_layout(800, 600, &fonts);
        let text = "alpha beta";
        let render = |selection, caret, horizontal_scroll, cursor_visible| {
            let mut surface = Surface::new(800, 600, clonk_graphics::PixelFormat::Rgba8888);
            draw_search_edit_contents(
                &mut surface,
                &layout,
                &fonts,
                text,
                caret,
                selection,
                horizontal_scroll,
                cursor_visible,
                None,
            );
            surface
        };
        let plain = render(None, text.len(), 0, false);
        assert_ne!(
            render(Some((0, 5)), text.len(), 0, false).snapshot(),
            plain.snapshot()
        );
        let caret = render(None, 0, 0, true);
        assert_ne!(caret.snapshot(), plain.snapshot());
        let edit = layout.search_edit;
        let mut changed = 0_u32;
        let mut bounds = (u32::MAX, u32::MAX, 0_u32, 0_u32);
        let mut hash = 0xcbf2_9ce4_8422_2325_u64;
        for y in edit.y as u32..(edit.y + edit.h) as u32 {
            for x in edit.x as u32..(edit.x + edit.w) as u32 {
                let on = caret.get_pixel(x, y).expect("caret-on pixel");
                let off = plain.get_pixel(x, y).expect("caret-off pixel");
                if on != off {
                    changed += 1;
                    bounds.0 = bounds.0.min(x);
                    bounds.1 = bounds.1.min(y);
                    bounds.2 = bounds.2.max(x);
                    bounds.3 = bounds.3.max(y);
                    for byte in [on.r, on.g, on.b, on.a, off.r, off.g, off.b, off.a] {
                        hash = (hash ^ u64::from(byte)).wrapping_mul(0x0000_0100_0000_01b3);
                    }
                }
            }
        }
        assert_eq!(
            (changed, bounds, hash),
            (104, (165, 468, 170, 485), 0x4ec9_97b4_06dd_f222)
        );
        assert_ne!(
            render(None, text.len(), 20, false).snapshot(),
            plain.snapshot()
        );
    }

    #[test]
    fn app_owned_game_options_get_icon_free_chrome_and_bounded_composition() {
        use crate::game_option_buttons::{
            GameOptionButtonResources, GameOptionButtons, GameOptionContext, GameOptionValues,
        };

        let assets = test_assets();
        let button_down = crate::test_support::load_graphics_png("GUIButtonDown.png");
        let fonts = endeavour_font_set();
        let mut legacy = Surface::new(1280, 720, clonk_graphics::PixelFormat::Rgba8888);
        ScenSelScreen::render_chrome(&mut legacy, &assets, &fonts, None, true, true);

        let mut composed = Surface::new(1280, 720, clonk_graphics::PixelFormat::Rgba8888);
        ScenSelScreen::render_chrome_without_game_options(
            &mut composed,
            &assets,
            &fonts,
            "Start Game",
            None,
        );
        let icon_free_chrome = composed.snapshot();
        let mut poison_assets = test_assets();
        poison_assets.icons_ex = ImageData::new(256, 320, vec![0xff; 256 * 320 * 4]);
        let mut poison_chrome = Surface::new(1280, 720, clonk_graphics::PixelFormat::Rgba8888);
        ScenSelScreen::render_chrome_without_game_options(
            &mut poison_chrome,
            &poison_assets,
            &fonts,
            "Start Game",
            None,
        );
        assert_eq!(
            poison_chrome.snapshot(),
            icon_free_chrome,
            "the app-owned chrome path must not sample even a deliberately poisoned icon sheet"
        );
        let layout = scen_sel_layout(1280, 720, &fonts);
        draw_back_button_with_state(
            &mut composed,
            &layout,
            "Back",
            &assets,
            &button_down,
            &fonts,
            ScenSelButtonState::default(),
            None,
        )
        .expect("classic Back resources");

        let mut options = GameOptionButtons::new(
            GameOptionContext::LocalSelector,
            GameOptionValues {
                fair_crew: true,
                record: true,
                ..GameOptionValues::default()
            },
        );
        options.set_bounds(layout.game_option_bounds());
        let resources = GameOptionButtonResources::new(
            &assets.icons_ex,
            &assets.button_highlight,
            &fonts.text,
        )
        .expect("classic option resources");
        let before_options = composed.snapshot();
        options
            .render(&mut composed, &resources, false, None)
            .expect("render app-owned option strip");
        assert_ne!(
            composed.snapshot(),
            before_options,
            "the app-owned strip must contribute the two icon bases exactly here"
        );

        let mut difference_count = 0usize;
        for y in 0..legacy.height() {
            for x in 0..legacy.width() {
                if legacy.get_pixel(x, y) != composed.get_pixel(x, y) {
                    difference_count += 1;
                    let inside = |rect: IntRect| {
                        (x as i32) >= rect.x
                            && (x as i32) < rect.x + rect.w
                            && (y as i32) >= rect.y
                            && (y as i32) < rect.y + rect.h
                    };
                    assert!(
                        inside(layout.fair_crew_button) || inside(layout.record_button),
                        "dynamic composition changed a non-option pixel at ({x}, {y})"
                    );
                }
            }
        }
        assert!(
            difference_count < (layout.fair_crew_button.w * layout.fair_crew_button.h * 2) as usize,
            "only filtered sampling inside the two app-owned icon facets may differ from the compatibility renderer"
        );
    }

    #[test]
    fn focused_and_pressed_back_open_match_classic_button_composition() {
        let assets = test_assets();
        let button_down = crate::test_support::load_graphics_png("GUIButtonDown.png");
        let fonts = endeavour_font_set();
        let layout = scen_sel_layout(800, 600, &fonts);
        let skin = ClassicGuiSkin::new(
            &assets.caption_bar,
            &assets.button,
            &button_down,
            Some(&assets.button_highlight),
        );
        let states = [
            ScenSelButtonState::default(),
            ScenSelButtonState {
                highlighted: true,
                pressed: false,
            },
            ScenSelButtonState {
                highlighted: true,
                pressed: true,
            },
        ];

        for (rect, label, draw) in [
            (
                layout.back_button,
                "&Back",
                draw_back_button_with_state as fn(
                    &mut Surface,
                    &ScenSelLayout,
                    &str,
                    &ScenSelAssets,
                    &ImageData,
                    &ClonkFontSet,
                    ScenSelButtonState,
                    Option<&GammaRamp>,
                ) -> Result<()>,
            ),
            (layout.open_button, "&Start", draw_open_button_with_state),
        ] {
            let mut snapshots = Vec::new();
            for state in states {
                let mut actual = Surface::new(800, 600, clonk_graphics::PixelFormat::Rgba8888);
                draw(
                    &mut actual,
                    &layout,
                    label,
                    &assets,
                    &button_down,
                    &fonts,
                    state,
                    None,
                )
                .expect("validated scenario button assets");

                let mut expected = Surface::new(800, 600, clonk_graphics::PixelFormat::Rgba8888);
                skin.draw_button(&mut expected, rect, label, &fonts, state, None);
                assert_eq!(actual.snapshot(), expected.snapshot());
                snapshots.push(actual.snapshot());
            }
            assert_ne!(snapshots[0], snapshots[1], "focus must add the highlight facet");
            assert_ne!(snapshots[1], snapshots[2], "press must select GUIButtonDown");
        }
    }

    #[test]
    fn dynamic_button_resources_fail_closed_on_nonclassic_facets() {
        let mut assets = test_assets();
        let button_down = crate::test_support::load_graphics_png("GUIButtonDown.png");
        let invalid_down = ImageData::new(1, 1, vec![0, 0, 0, 0]);
        let error = validate_scensel_button_assets(&assets, &invalid_down)
            .expect_err("a substitute down plank must be rejected");
        assert!(error.to_string().contains("GUIButtonDown.png"));

        assets.button = ImageData::new(64, 32, vec![0; 64 * 32 * 4]);
        let error = validate_scensel_button_assets(&assets, &button_down)
            .expect_err("a substitute released plank must be rejected");
        assert!(error.to_string().contains("GUIButton.png"));

        let mut assets = test_assets();
        assets.button_highlight = ImageData::new(32, 32, vec![0; 32 * 32 * 4]);
        let error = validate_scensel_button_assets(&assets, &button_down)
            .expect_err("a substitute highlight must be rejected");
        assert!(error.to_string().contains("GUIButtonHighlight.png"));
    }

    // Renders the first-shown frame at 1280x720 and dumps it for the
    // out-of-band ImageMagick diff against the C++ F9 capture
    // (build/Screenshots/ref-scensel.png). CI has no reference image, so this
    // test only produces the artifact.
    #[test]
    fn render_matches_reference() {
        let assets = test_assets();
        let gui_fonts = endeavour_font_set();
        let ttf = std::fs::read(
            crate::test_support::repo_root().join("planet/System.c4g/Endeavour.ttf"),
        )
        .expect("read Endeavour.ttf");
        let book_fonts = build_book_font_set(&ttf).expect("build book fonts");
        let gamma = crate::test_support::standard_gamma();

        let mut surface = clonk_graphics::Surface::new(1280, 720, clonk_graphics::PixelFormat::Rgba8888);
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
