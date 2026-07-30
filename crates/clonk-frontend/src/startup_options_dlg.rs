//! Pixel-parity renderer/controller for `C4StartupOptionsDlg`, including the
//! first-shown **Program** tab and the complete **Sound** tab, against the C++
//! engine's F9 reference capture
//! (`target/parity-specs/options.md`, `build/Screenshots/ref-options.png`).
//!
//! Geometry mirrors the C++ ctor `C4StartupOptionsDlg.cpp:609-985` in exact
//! integer math; widget rendering mirrors `C4GuiTabular.cpp` (tab strip),
//! `C4GuiComboBox.cpp:138-185`, `C4GuiCheckBox.cpp:110-137`,
//! `C4GuiContainers.cpp:446-473,633-677` (slider/group box) and
//! `C4StartupOptionsDlg.cpp:69-105` (`SmallButton`).
//!
//! Spec corrections (each verified against the C++ source and the reference
//! pixels in `ref-options.png`):
//! 1. The spec derives `caConfigArea`'s vertical margin as `632/200 = 3`,
//!    but the C++ uses `caMain.GetHeight()/200` *after* `GetFromBottom`
//!    consumed the button bar (C4StartupOptionsDlg.cpp:652, C4Gui.h:1900) —
//!    `542/200 = 2`. The tabular is therefore at (243,71,794,538) abs, paper
//!    (338,71,699,538), sheet client (356,108,644,462); group-frame pixels
//!    (x=417/418, x2=647, combo top y=139) confirm this.
//! 2. The active-tab focus highlight is 85 px wide, not `iMaxTabWidth-10 =
//!    105`: `DrawCaption` clamps `iMaxWdt = iTxtWdt` (95) whenever clip gfx
//!    are present (C4GuiTabular.cpp:393).
//! 3. The fair-crew group's client top margin uses the GUI CaptionFont line
//!    height (25), not the BookFont's: `SetTitle` (which relayouts) runs
//!    before `SetFont` (which does not), see `options_dlg_layout`.
//! 4. `DrawLineDw` is GL_LINES with the diamond-exit rule — the END pixel of
//!    each line is not rasterized (group-frame corner pixels prove it).
//! 5. Fully transparent texels sample as BLACK in stretched blits
//!    (`C4Surface::ReadPNG` rewrite, C4Surface.cpp:972), not as the PNG's
//!    hidden RGB; GL-tile padding outside the image is transparent WHITE
//!    (C4Surface.cpp:1113).

use crate::clonk_fonts::ClonkFontSet;
use crate::startup_main_menu::{draw_bar, IntRect, StartupTooltip};
use crate::startup_options_controls::{
    control_facets, control_sheet_hit_test, key_button_facets, ControlCaptureTarget, ControlDevice,
    ControlSheetHit, ControlSheetLayout, ControlSheetState, CONTROL_KEY_COUNT, CONTROL_KEY_LABELS,
};
use crate::startup_options_graphics::{
    graphics_hit_test, graphics_sheet_layout, GraphicsCheckboxId, GraphicsHitTarget,
    GraphicsSheetAction, GraphicsSheetLayout, GraphicsSheetState, GraphicsSliderId,
    GraphicsSliderPart, SpinboxDirection, MAX_GRAPHICS_SCALE_PERCENT, MAX_SMOKE_LEVEL,
    MIN_GRAPHICS_SCALE_PERCENT, MIN_SMOKE_LEVEL,
};
use crate::startup_options_network::{
    network_sheet_hit_test, NetworkCheckboxId, NetworkPortId, NetworkSheetHit, NetworkSheetLayout,
    NetworkSheetState, NetworkTextField,
};
use crate::{
    draw_image_bilinear, draw_image_bilinear_additive, draw_image_strip, GuiPoint, ImageData,
    KeyCode,
};
use anyhow::{Context, Result};
use clonk_graphics::clonk_font::{ClonkFont, ClonkFontRole, GlyphCell, TextAlign};
use clonk_graphics::{
    BlitSampling, Color, GammaRamp, Rect as SurfaceRect, Surface, SurfaceDrawTarget,
};
use clonk_gui::Rect as GuiRect;
use clonk_resources::LanguageInfo;
use freetype::face::LoadFlag;
use freetype::Library;
use std::cell::RefCell;
use std::collections::HashMap;

// ---------------------------------------------------------------------------
// Startup colors (C4Startup.h:28-34) and GUI constants. Engine box/line/quad
// colors are AARRGGBB with INVERTED alpha (0x00 = opaque).
// ---------------------------------------------------------------------------

/// `C4StartupEditBorderColor` (C4Startup.h:31).
const EDIT_BORDER_COLOR: u32 = 0x00a4_947a;
/// `C4StartupBtnBorderColor1` (C4Startup.h:33).
const BTN_BORDER_COLOR1: u32 = 0x00cc_c3b4;
/// `C4StartupBtnBorderColor2` (C4Startup.h:34).
const BTN_BORDER_COLOR2: u32 = 0x0094_846a;
/// `C4StartupFontClr` 0xff000000 as normal-alpha RGBA (text colors are
/// normal-alpha; `DrawText` inverts on entry, StdFont.cpp:819).
pub(crate) const STARTUP_FONT_RGBA: [u8; 4] = [0, 0, 0, 255];
/// `C4StartupBtnFontClr` 0xff202020 (C4Startup.h:32).
const BTN_FONT_RGBA: [u8; 4] = [0x20, 0x20, 0x20, 255];
/// `C4GUI_ButtonFontClr` / `C4GUI_FullscreenCaptionFontClr` 0xffffff00.
const YELLOW_FONT_RGBA: [u8; 4] = [255, 255, 0, 255];

/// Every string `C4StartupOptionsDlg` draws or measures, resolved through
/// `LoadResStr` at construction (C4StartupOptionsDlg.cpp:107-125,160-200,
/// 609-1033). `Default` reproduces the shipped `LanguageUS.txt` values, which
/// is what `C4ResStrTable` falls back to for a missing key.
///
/// Trailing punctuation that C++ appends in code rather than storing in the
/// table - the `':'` after the Language/Font/White Chat labels
/// (C4StartupOptionsDlg.cpp:687) - is added at the draw site, so these fields
/// hold the bare resource text.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct OptionsLabels {
    /// `IDS_DLG_OPTIONS`, with its mnemonic marker stripped for the caption.
    pub title: String,
    /// `IDS_DLG_PROGRAM`/`GRAPHICS`/`SOUND`/`KEYBOARD`/`GAMEPAD`/`NETWORK`.
    pub sheets: [String; 6],
    /// `IDS_BTN_BACK`.
    pub back: String,
    /// The twelve `IDS_CTL_*` action labels, in
    /// `startup_options_controls::CONTROL_KEY_LABEL_KEYS` order.
    pub control_keys: [String; CONTROL_KEY_COUNT],
    /// `IDS_CTL_LANGUAGE`.
    pub language: String,
    /// `IDS_CTL_FONT`.
    pub font: String,
    /// `IDS_MNU_WHITECHAT`.
    pub white_chat: String,
    /// `IDS_CTL_WHITECHAT_INGAME`.
    pub white_chat_ingame: String,
    /// `IDS_CTL_WHITECHAT_LOBBY`.
    pub white_chat_lobby: String,
    /// `IDS_CTL_TIMESTAMPS`.
    pub timestamps: String,
    /// `IDS_CTL_PRELOADING`.
    pub preloading: String,
    /// `IDS_BTN_RESETCONFIG`.
    pub reset_config: String,
    /// `IDS_DLG_ADVANCED_SETTINGS`.
    pub advanced_settings: String,
    /// `IDS_CTL_FAIRCREWSTRENGTH`.
    pub fair_crew_strength: String,
    /// `IDS_CTL_NOLANGINFO`.
    pub no_language_info: String,
    /// `IDS_CTL_MUSIC`.
    pub music: String,
    /// `IDS_CTL_SOUNDFX`.
    pub sound_effects: String,
    /// `IDS_CTL_DISPLAYMODE`.
    pub display_mode: String,
    /// `IDS_CTL_GRAPHICSSCALE`.
    pub graphics_scale: String,
    /// `IDS_CTL_SMOKELOW` / `IDS_CTL_SMOKEHI`.
    pub effects_low: String,
    pub effects_high: String,
    /// `IDS_BTN_TESTGRAPHICSSCALE`.
    pub apply_scale: String,
    /// `IDS_BTN_RESETKEYBOARD`.
    pub reset_keyboard: String,
    /// `IDS_NET_PORT_REFERENCE` / `IDS_NET_PORT_DISCOVERY`.
    pub port_reference: String,
    pub port_discovery: String,
    /// `IDS_CTL_ACTIVE`.
    pub active: String,
    /// `IDS_CTL_USEOTHERSERVER`.
    pub use_other_server: String,
    /// `IDS_CTL_AUTOMATICUPDATES`.
    pub automatic_updates: String,
    /// `IDS_CTL_UPNP`.
    pub upnp: String,
    /// `IDS_NET_COMPUTERNAME` / `IDS_NET_USERNAME`; both already carry `':'`.
    pub computer_name: String,
    pub chat_name: String,
}

impl Default for OptionsLabels {
    fn default() -> Self {
        Self {
            title: "Options".into(),
            control_keys: CONTROL_KEY_LABELS.map(Into::into),
            sheets: [
                "Program".into(),
                "Graphics".into(),
                "Sound".into(),
                "Keyboard".into(),
                "Gamepad".into(),
                "Network".into(),
            ],
            back: "Back".into(),
            language: "Language".into(),
            font: "Font".into(),
            white_chat: "White Chat".into(),
            white_chat_ingame: "Ingame".into(),
            white_chat_lobby: "Lobby".into(),
            timestamps: "Timestamps".into(),
            preloading: "Preload game data".into(),
            reset_config: "Reset configuration".into(),
            advanced_settings: "Advanced settings".into(),
            fair_crew_strength: "Strength of \"Fair Crew\"".into(),
            no_language_info: "Language pack not available.".into(),
            music: "Music".into(),
            sound_effects: "Sound effects".into(),
            display_mode: "Display mode".into(),
            graphics_scale: "Scale".into(),
            effects_low: "Low".into(),
            effects_high: "High".into(),
            apply_scale: "Apply".into(),
            reset_keyboard: "Reset all".into(),
            port_reference: "Reference port".into(),
            port_discovery: "Discovery port".into(),
            active: "Active".into(),
            use_other_server: "Use alternate internet server".into(),
            automatic_updates: "Enable automatic updates".into(),
            upnp: "Use UPnP".into(),
            computer_name: "Computer name:".into(),
            chat_name: "Chat name:".into(),
        }
    }
}

/// Sheet titles + icon phases, ctor order (C4StartupOptionsDlg.cpp:663-668;
/// LanguageUS.txt). Program and Sound are pixel-implemented; the remaining
/// sheets are data stubs for the tab strip.
pub const SHEET_TITLES: [&str; 6] = [
    "Program", "Graphics", "Sound", "Keyboard", "Gamepad", "Network",
];

/// Entries filled by `C4StartupOptionsDlg::OnFontFaceComboFill`, in native
/// menu order.
pub const PROGRAM_FONT_FACES: [&str; 4] =
    ["Arial Unicode MS", "Comic Sans MS", "Endeavour", "Verdana"];

/// Entries filled by `C4StartupOptionsDlg::OnFontSizeComboFill`, in native
/// menu order. The context-menu id is the displayed size itself.
pub const PROGRAM_FONT_SIZES: [i32; 9] = [8, 10, 12, 14, 16, 18, 20, 24, 28];

// ---------------------------------------------------------------------------
// Shadowless startup "book" fonts (C4StartupGraphics::InitFonts,
// C4Startup.cpp:93-116: BookFont = C4FT_Main 14px, BookSmallFont =
// C4FT_MainSmall 13px, fDoShadow = false).
// ---------------------------------------------------------------------------

/// The two shadowless Endeavour book fonts the options dialog draws with.
pub struct BookFonts {
    /// `C4StartupGraphics::BookFont` (14px, line height 22).
    pub book: ClonkFont,
    /// `C4StartupGraphics::BookSmallFont` (13px, line height 20).
    pub book_small: ClonkFont,
}

/// Builds the two book fonts from a TTF, mirroring `CStdFont::Init` with
/// `fDoShadow = false` (StdFont.cpp:319-358): `iHSpace = 0`, `iGfxLineHgt =
/// iLineHgt`, no shadow kernel; each atlas pixel is pure white with alpha =
/// FreeType coverage (StdFont.cpp:224-258 with shadowSize = 0).
pub fn build_book_fonts(ttf_bytes: &[u8]) -> Result<BookFonts> {
    let library = Library::init().context("FreeType init failed")?;
    let face = library
        .new_memory_face(ttf_bytes.to_vec(), 0)
        .context("failed to load font face")?;
    Ok(BookFonts {
        book: build_shadowless_font(&face, 14)?.with_role(ClonkFontRole::BookText),
        book_small: build_shadowless_font(&face, 13)?.with_role(ClonkFontRole::BookSmall),
    })
}

/// Rasterizes one shadowless `CStdFont` at `px_height`.
///
/// Differences from the shadowed GUI build (crate::clonk_fonts::build_font):
/// cell width has no `+1` shadow column (StdFont.cpp:218 with shadowSize=0),
/// cell height is `iLineHgt` (StdFont.cpp:352), `iHSpace = 0`
/// (StdFont.cpp:327), and the cell is white-with-coverage-alpha only
/// (the `BltAlpha` on a fully transparent base yields the source,
/// StdColors.h:122-126). Glyphs cover ASCII and Latin-1; the CP1252
/// specials 0x80-0x9F are skipped (unused by this dialog's strings).
fn build_shadowless_font(face: &freetype::Face, px_height: u32) -> Result<ClonkFont> {
    face.set_pixel_sizes(px_height, px_height)
        .context("FT_Set_Pixel_Sizes failed")?;

    let raw = face.raw();
    let units_per_em = i32::from(raw.units_per_EM);
    let (ascender, descender) = (i32::from(raw.ascender), i32::from(raw.descender));
    let line_height =
        clonk_graphics::clonk_font::line_height_for(ascender, descender, units_per_em, px_height);
    let ascent_px = i64::from(px_height) * i64::from(ascender) / i64::from(units_per_em);

    let mut font = ClonkFont::new(line_height);
    font.cell_height = line_height; // iGfxLineHgt = iLineHgt (no shadow row)
    font.h_space = 0; // StdFont.cpp:327

    let chars = (0x20u8..0x7f).chain(0xa0..=0xff).map(char::from);
    for ch in chars {
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

        // width = max(advance, bearing + width), no shadow (StdFont.cpp:218).
        let advance_px = (slot.advance().x >> 6) as i32;
        let bearing = slot.bitmap_left().max(0);
        let cell_w = advance_px.max(bearing + cov_w as i32).max(0) as usize;
        let cell_h = line_height.max(0) as usize;
        let at_x = bearing as usize;
        let at_y = (ascent_px - i64::from(slot.bitmap_top())).max(0) as usize;

        let mut pixels = vec![Color::transparent(); cell_w * cell_h];
        for y in 0..cov_h {
            for x in 0..cov_w {
                let coverage = buffer
                    .get((y as i32 * pitch) as usize + x)
                    .copied()
                    .unwrap_or(0);
                let (tx, ty) = (at_x + x, at_y + y);
                if tx < cell_w && ty < cell_h {
                    // White with alpha = coverage (StdFont.cpp:228-232,255).
                    pixels[ty * cell_w + tx] = Color::new(255, 255, 255, coverage);
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

// ---------------------------------------------------------------------------
// ComponentAligner (C4Gui.cpp:975-1090, C4Gui.h:1868-1912) — exact C++
// integer math; only the pieces this dialog uses.
// ---------------------------------------------------------------------------

#[derive(Clone, Copy)]
struct Aligner {
    area: IntRect,
    mx: i32,
    my: i32,
}

impl Aligner {
    fn new(area: IntRect, mx: i32, my: i32) -> Self {
        Self { area, mx, my }
    }

    fn width(&self) -> i32 {
        self.area.w
    }

    fn height(&self) -> i32 {
        self.area.h
    }

    fn inner_width(&self) -> i32 {
        self.area.w - 2 * self.mx
    }

    /// C4Gui.cpp:975-990.
    fn get_from_top(&mut self, hgt: i32) -> IntRect {
        let out = IntRect {
            x: self.area.x + self.mx,
            y: self.area.y + self.my,
            w: self.area.w - 2 * self.mx,
            h: hgt,
        };
        let d = hgt + self.my * 2;
        self.area.y += d;
        self.area.h -= d;
        out
    }

    /// C4Gui.cpp:992-1007 (`iHgt = -1` keeps the full height).
    fn get_from_left(&mut self, wdt: i32, hgt: i32) -> IntRect {
        let mut out = IntRect {
            x: self.area.x + self.mx,
            y: self.area.y + self.my,
            w: wdt,
            h: self.area.h - 2 * self.my,
        };
        let d = wdt + self.mx * 2;
        self.area.x += d;
        self.area.w -= d;
        if hgt >= 0 {
            out.y += (out.h - hgt) / 2;
            out.h = hgt;
        }
        out
    }

    /// C4Gui.cpp:1009-1023.
    fn get_from_right(&mut self, wdt: i32, hgt: i32) -> IntRect {
        let mut out = IntRect {
            x: self.area.x + self.area.w - wdt - self.mx,
            y: self.area.y + self.my,
            w: wdt,
            h: self.area.h - 2 * self.my,
        };
        self.area.w -= wdt + self.mx * 2;
        if hgt >= 0 {
            out.y += (out.h - hgt) / 2;
            out.h = hgt;
        }
        out
    }

    /// C4Gui.cpp:1025-1041.
    fn get_from_bottom(&mut self, hgt: i32) -> IntRect {
        let out = IntRect {
            x: self.area.x + self.mx,
            y: self.area.y + self.area.h - hgt - self.my,
            w: self.area.w - 2 * self.mx,
            h: hgt,
        };
        self.area.h -= hgt + self.my * 2;
        out
    }

    /// C4Gui.cpp:1043-1049.
    fn get_all(&self) -> IntRect {
        IntRect {
            x: self.area.x + self.mx,
            y: self.area.y + self.my,
            w: self.area.w - 2 * self.mx,
            h: self.area.h - 2 * self.my,
        }
    }

    /// C4Gui.cpp:1051-1060 (`GetMiddleX/Y` = origin + extent/2).
    fn get_centered(&self, wdt: i32, hgt: i32) -> IntRect {
        IntRect {
            x: self.area.x + self.area.w / 2 - wdt / 2,
            y: self.area.y + self.area.h / 2 - hgt / 2,
            w: wdt,
            h: hgt,
        }
    }

    /// C4Gui.h:1909 (`ExpandLeft`).
    fn expand_left(&mut self, by: i32) {
        self.area.x -= by;
        self.area.w += by;
    }

    /// C4Gui.cpp:1062-1085.
    #[allow(clippy::too_many_arguments)] // mirrors the C++ signature
    fn get_grid_cell(
        &self,
        sect_x: i32,
        sect_x_max: i32,
        sect_y: i32,
        sect_y_max: i32,
        size_x: i32,
        size_y: i32,
        center: bool,
        num_x: i32,
        num_y: i32,
    ) -> IntRect {
        let size_x_max = (self.area.w - self.mx) / sect_x_max - self.mx;
        let size_y_max = (self.area.h - self.my) / sect_y_max - self.my;
        let cell_w = if size_x < 0 || center {
            size_x_max
        } else {
            size_x.min(size_x_max)
        };
        let cell_h = if size_y < 0 || center {
            size_y_max
        } else {
            size_y.min(size_y_max)
        };
        let mut out = IntRect {
            x: sect_x * (cell_w + self.mx) + self.mx + self.area.x,
            y: sect_y * (cell_h + self.my) + self.my + self.area.y,
            w: cell_w * num_x + self.mx * (num_x - 1),
            h: cell_h * num_y + self.my * (num_y - 1),
        };
        if size_x >= 0 && center {
            out.x += (cell_w - size_x) / 2;
            out.w = size_x;
        }
        if size_y >= 0 && center {
            out.y += (cell_h - size_y) / 2;
            out.h = size_y;
        }
        out
    }
}

// ---------------------------------------------------------------------------
// Layout
// ---------------------------------------------------------------------------

/// Pixel-exact geometry for the Sound sheet, in screen coordinates.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SoundSheetLayout {
    pub frontend_group: IntRect,
    pub game_group: IntRect,
    pub volume_group: IntRect,
    pub checkboxes: [IntRect; 4],
    pub volume_headings: [IntRect; 2],
    pub quiet_labels: [IntRect; 2],
    pub loud_labels: [IntRect; 2],
    pub sliders: [IntRect; 2],
}

impl SoundSheetLayout {
    pub const fn checkbox(&self, id: SoundCheckboxId) -> IntRect {
        self.checkboxes[id.index()]
    }

    pub const fn slider(&self, id: SoundVolumeId) -> IntRect {
        self.sliders[id.index()]
    }
}

/// Pixel-exact `C4StartupOptionsDlg` geometry, in screen coordinates.
#[derive(Clone, Debug)]
pub struct OptionsDlgLayout {
    /// Fullscreen-dialog client rect (C4GuiDialogs.cpp:816-823).
    pub client: IntRect,
    /// "Options" title center anchor (FullscreenDialog::SetTitle,
    /// C4GuiDialogs.cpp:834-849).
    pub title_center: (i32, i32),
    /// Autosized fullscreen title label bounds. The generic C++
    /// `FullscreenDialog::SetTitle` installs the displayed title itself as
    /// this label's tooltip (C4GuiDialogs.cpp:834-849).
    pub title_label: IntRect,
    /// Back button (ctor 655-657).
    pub back_button: IntRect,
    /// Tabular bounds after `SetGfx` aspect correction
    /// (C4GuiTabular.cpp:637-668).
    pub tabular: IntRect,
    /// Paper `DrawX` dest (C4GuiTabular.cpp:455).
    pub paper: IntRect,
    /// Tab clip top-left per sheet (C4GuiTabular.cpp:436-441,59).
    pub tab_clips: [(i32, i32); 6],
    /// 32x32 icon top-left per sheet (C4GuiTabular.cpp:62-63).
    pub tab_icons: [(i32, i32); 6],
    /// Caption text center (ACenter anchor) per sheet (C4GuiTabular.cpp:64).
    pub tab_captions: [(i32, i32); 6],
    /// Focus highlight rect on the active tab (C4GuiTabular.cpp:67-72).
    pub focus_highlight: IntRect,
    /// Sheet client area (tabular margins, C4GuiTabular.h:108-111).
    pub sheet: IntRect,
    /// "Language:" label text position (ALeft anchor, rect top).
    pub language_label: (i32, i32),
    pub language_combo: IntRect,
    /// Language info label text position.
    pub language_info: (i32, i32),
    /// "Font:" label text position.
    pub font_label: (i32, i32),
    pub font_face_combo: IntRect,
    pub font_size_combo: IntRect,
    /// "White Chat:" label text position.
    pub white_chat_label: (i32, i32),
    /// CheckBox bounds; the box facet is `h`x`h` at the rect origin.
    pub ingame_check: IntRect,
    pub lobby_check: IntRect,
    pub timestamps_check: IntRect,
    pub preloading_check: IntRect,
    /// Fair-crew GroupBox bounds (ctor 762-768).
    pub group: IntRect,
    /// "weak" / "strong" labels (ACenter within rect, fAutosize=false).
    pub weak_label: IntRect,
    pub strong_label: IntRect,
    /// Fair-crew slider (horizontal ScrollBar) bounds.
    pub slider: IntRect,
    pub reset_button: IntRect,
    pub advanced_button: IntRect,
    /// Sound sheet groups and children (`C4StartupOptionsDlg.cpp:921-985`).
    pub sound: SoundSheetLayout,
    pub graphics: GraphicsSheetLayout,
    pub controls: ControlSheetLayout,
    pub network: NetworkSheetLayout,
}

/// Computes the dialog layout, mirroring C4StartupOptionsDlg.cpp:609-985.
/// `gui` provides the shadowed GUI fonts (caption measurements), `book` the
/// startup book fonts (all sheet text).
pub fn options_dlg_layout(
    w: i32,
    h: i32,
    gui: &ClonkFontSet,
    book: &BookFonts,
) -> OptionsDlgLayout {
    options_dlg_layout_with_labels(w, h, gui, book, &OptionsLabels::default())
}

/// The measured widths of the Program sheet's labels and buttons come from the
/// resolved resource text, so a longer translation widens its column exactly as
/// `GetTextExtent` does in C++ (C4StartupOptionsDlg.cpp:687-697).
pub fn options_dlg_layout_with_labels(
    w: i32,
    h: i32,
    gui: &ClonkFontSet,
    book: &BookFonts,
    labels: &OptionsLabels,
) -> OptionsDlgLayout {
    // FullscreenDialog margins (C4GuiDialogs.cpp:816-823).
    let margin_x = if w < 500 { 2 } else { w / 50 };
    let margin_y = if h < 320 { 2 } else { h * 2 / 75 };
    let client = IntRect {
        x: margin_x,
        y: 50 + margin_y,
        w: w - 2 * margin_x,
        h: h - (50 + margin_y) - margin_y,
    };
    let abs = |r: IntRect| IntRect {
        x: r.x + client.x,
        y: r.y + client.y,
        ..r
    };

    let f_small = client.w < 750;
    // Title label (C4GuiDialogs.cpp:843-845): centered, y = 25 - lh/2 - top.
    let title_center = (
        client.x + client.w / 2,
        client.y + 25 - gui.title.line_height / 2 - (50 + margin_y),
    );
    let (title_width, title_height) = gui.title.measure(&labels.title, true);
    let title_label = IntRect {
        x: title_center.0 - title_width / 2,
        y: title_center.1,
        w: title_width,
        h: title_height,
    };

    // Back button (ctor 627-629, 655-657): 3*w("<< BACK") @ CaptionFont.
    let back_w = 3 * gui.caption.measure("<< BACK", true).0;
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
    let button_area = ca_main.get_from_bottom(ca_main.height() / if f_small { 20 } else { 7 });
    let mut ca_buttons = Aligner::new(
        Aligner::new(button_area, 0, 0).get_centered(client.w * 7 / 8, 32),
        0,
        0,
    );
    let back_button = abs(ca_buttons.get_from_left(back_w, -1));

    // Tabular (ctor 652, 660-661): margins from caMain AFTER the button bar.
    let ca_config = Aligner::new(
        ca_main.get_all(),
        if f_small {
            0
        } else {
            ca_main.width() * 69 / 1730
        },
        if f_small { 0 } else { ca_main.height() / 200 },
    );
    let mut tab = ca_config.get_all();
    // SetGfx aspect correction (C4GuiTabular.cpp:637-668); left clip size =
    // 120*95/120 = 95 (C4GuiTabular.h:135, StartupTabClip.png is 120 wide).
    let left_size = 95;
    let (paper_w, paper_h) = (628, 483);
    let (eff_w, eff_h) = (tab.w - left_size, tab.h);
    if eff_w * paper_h > paper_w * eff_h {
        let oversize = eff_w - paper_w * eff_h / paper_h;
        tab.x += oversize / 2;
        tab.w -= oversize;
    } else {
        let oversize = eff_h - paper_h * eff_w / paper_w;
        tab.y += oversize;
        tab.h -= oversize;
    }
    let tabular = abs(tab);

    // Tab strip (C4GuiTabular.cpp:380-462): x0 at the paper's left edge,
    // iSheetOff=20, iSheetSpacing=-10, advance (80-10)+2 = 72.
    let x0 = tabular.x + left_size;
    let paper = IntRect {
        x: x0,
        y: tabular.y,
        w: tabular.x + tabular.w - x0,
        h: tabular.h,
    };
    let cpt_x = x0 - left_size + 10;
    let mut tab_clips = [(0, 0); 6];
    let mut tab_icons = [(0, 0); 6];
    let mut tab_captions = [(0, 0); 6];
    for i in 0..6 {
        let d = tabular.y + 20 + 72 * i;
        let y = d - 5; // iCptTextY = d + iSheetSpacing/2
        tab_clips[i as usize] = (cpt_x, y);
        // DrawCaption (C4GuiTabular.cpp:59-64): icon centered above the text.
        let y_top = y + 80 / 2 - (32 + 2 + book.book_small.line_height) / 2;
        tab_icons[i as usize] = (cpt_x + 95 / 2 - 16, y_top);
        tab_captions[i as usize] = (cpt_x + 95 / 2, y_top + 32 + 2);
    }
    // Spec correction: the spec derives the focus-highlight width from
    // iMaxTabWidth (115), but DrawCaption overrides `iMaxWdt = iTxtWdt` (95)
    // whenever clip gfx are present (C4GuiTabular.cpp:393) — so the additive
    // highlight is 95-10 = 85 wide (verified against ref-options.png).
    let focus_highlight = IntRect {
        x: cpt_x + 5,
        y: tab_clips[0].1 + 3,
        w: 95 - 10,
        h: 80 - 6,
    };

    // Sheet client (tabular margins, C4GuiTabular.h:108-111, iSheetMargin=4).
    let sheet = IntRect {
        x: tabular.x + 4 + left_size + (tab.w - left_size) * 13 / 628,
        y: tabular.y + 4 + tab.h * 30 / 483,
        w: tab.w
            - (4 + left_size + (tab.w - left_size) * 13 / 628)
            - (4 + (tab.w - left_size) * 30 / 628),
        h: tab.h - (4 + tab.h * 30 / 483) - (4 + tab.h * 32 / 483),
    };
    let sheet_abs = |r: IntRect| IntRect {
        x: r.x + sheet.x,
        y: r.y + sheet.y,
        ..r
    };

    // Program sheet (ctor 675-792): margins caMain w/20, h/20 (post-bar).
    let book_w = |s: &str| book.book.measure(s, true).0;
    let ca_sheet = Aligner::new(
        IntRect {
            x: 0,
            y: 0,
            w: sheet.w,
            h: sheet.h,
        },
        ca_main.width() / 20,
        ca_main.height() / 20,
    );

    // Language rows (678-698).
    let mut ca_language =
        Aligner::new(ca_sheet.get_grid_cell(0, 1, 0, 8, -1, -1, true, 1, 2), 0, 4);
    let mut ca_lang_box = Aligner::new(ca_language.get_from_top(26), 0, 0);
    let lang_label_rect =
        ca_lang_box.get_from_left(book_w(&format!("{}:", labels.language)) + 4, -1);
    let language_label = (sheet.x + lang_label_rect.x, sheet.y + lang_label_rect.y);
    let lang_combo_w = book_w("XX: Top Secret Language").min(ca_lang_box.width());
    let language_combo = sheet_abs(ca_lang_box.get_from_left(lang_combo_w, -1));
    let info_rect = ca_language.get_from_top(gui.text.line_height * 3);
    let language_info = (sheet.x + info_rect.x, sheet.y + info_rect.y);

    // Font row (700-723).
    let mut ca_font = Aligner::new(ca_sheet.get_grid_cell(0, 1, 2, 9, -1, 26, true, 1, 1), 0, 0);
    let font_label_rect = ca_font.get_from_left(book_w(&format!("{}:", labels.font)) + 4, -1);
    let font_label = (sheet.x + font_label_rect.x, sheet.y + font_label_rect.y);
    let comic_w = book_w("Comic Sans MS");
    let face_w = (ca_font.inner_width() * 3 / 4).min(comic_w * 3);
    let font_face_combo = sheet_abs(ca_font.get_from_left(face_w, -1));
    ca_font.expand_left(-4);
    let font_size_combo = sheet_abs(ca_font.get_from_left(ca_font.inner_width().min(comic_w), -1));

    // White chat row (726-747).
    let mut ca_chat = Aligner::new(ca_sheet.get_grid_cell(0, 1, 3, 9, -1, 26, true, 1, 1), 0, 0);
    let chat_label_rect =
        ca_chat.get_from_left(book_w(&format!("{}:", labels.white_chat)) + 4 + 26, -1);
    let white_chat_label = (sheet.x + chat_label_rect.x, sheet.y + chat_label_rect.y);
    let ingame_check =
        sheet_abs(ca_chat.get_from_left(book_w(&labels.white_chat_ingame) + 4 + 2 * 26, -1));
    let lobby_check =
        sheet_abs(ca_chat.get_from_left(book_w(&labels.white_chat_lobby) + 4 + 2 * 26, -1));

    // Timestamps / preloading (750-759); iCheckHgt = book line height.
    let check_h = book.book.line_height;
    let timestamps_check = sheet_abs(ca_sheet.get_grid_cell(0, 1, 4, 9, -1, check_h, true, 1, 1));
    let preloading_check = sheet_abs(ca_sheet.get_grid_cell(0, 1, 5, 9, -1, check_h, true, 1, 1));

    // Fair crew group (762-779): h = 2*lh + 2*iIndentY2 + 16.
    let indent_y2 = if f_small {
        1
    } else {
        1.max(client.h / 200 / 2)
    };
    let group_h = book.book.line_height * 2 + indent_y2 * 2 + 16;
    let group = sheet_abs(ca_sheet.get_grid_cell(0, 2, 6, 9, -1, group_h, true, 1, 2));
    // Group client: margins l/r/b = 4, top = 4 + title-font line height
    // (C4Gui.h:1008-1011). Spec correction: the title font here is the GUI
    // CaptionFont (25px), NOT the BookFont — the ctor calls SetTitle (which
    // recomputes the client rect via UpdateOwnPos with pFont still null,
    // C4Gui.h:1001) BEFORE SetFont, and SetFont never relayouts
    // (C4Gui.h:999); nothing re-runs UpdateOwnPos afterwards. The reference
    // confirms: weak/strong/slider sit 1px lower than a BookFont-margin
    // model would place them.
    let title_lh = gui.caption.line_height;
    let group_client = IntRect {
        x: group.x + 4,
        y: group.y + 4 + title_lh,
        w: group.w - 8,
        h: group.h - 8 - title_lh,
    };
    let mut ca_group = Aligner::new(
        IntRect {
            x: 0,
            y: 0,
            w: group_client.w,
            h: group_client.h,
        },
        1,
        0,
    );
    let group_abs = |r: IntRect| IntRect {
        x: r.x + group_client.x,
        y: r.y + group_client.y,
        ..r
    };
    let weak_label = group_abs(ca_group.get_from_left(book_w("weak"), check_h));
    let strong_label = group_abs(ca_group.get_from_right(book_w("strong"), check_h));
    let slider = group_abs(ca_group.get_centered(ca_group.inner_width(), 16));

    // Bottom small buttons (781-792): W = min(w@CaptionFont + lh*4, inner*2/5);
    // SmallButton height = 22*6/5 + 6 = 32 (C4StartupOptionsDlg.cpp:101-105).
    let small_btn_h = book.book.line_height * 6 / 5 + 6;
    let btn_w = |text: &str| {
        let (tw, _) = gui.caption.measure(text, true);
        (tw + gui.caption.line_height * 4).min(ca_sheet.inner_width() * 2 / 5)
    };
    let reset_button = sheet_abs(ca_sheet.get_grid_cell(
        1,
        2,
        8,
        9,
        btn_w(&labels.reset_config),
        small_btn_h,
        true,
        1,
        1,
    ));
    let advanced_button = sheet_abs(ca_sheet.get_grid_cell(
        0,
        2,
        8,
        9,
        btn_w(&labels.advanced_settings),
        small_btn_h,
        true,
        1,
        1,
    ));

    // Sound sheet (ctor 921-985). Its grid deliberately uses the dialog-wide
    // responsive indents, while all child coordinates are local to their
    // titled GroupBox client windows.
    let indent_x1 = if f_small { 20 } else { client.w / 40 };
    let indent_y1 = if f_small { 1 } else { client.h / 200 };
    let sound_sheet = Aligner::new(
        IntRect {
            x: 0,
            y: 0,
            w: sheet.w,
            h: sheet.h,
        },
        indent_x1,
        indent_y1,
    );
    let (lorem_w, sound_check_h) = book.book.measure("Lorem ipsum", true);
    let sound_check_w = lorem_w + sound_check_h + 4;
    let grid_w = sound_check_w * 2;
    let grid_h = sound_check_h * 5 / 2;
    let frontend_group =
        sheet_abs(sound_sheet.get_grid_cell(0, 2, 0, 5, grid_w, grid_h, false, 1, 2));
    let game_group = sheet_abs(sound_sheet.get_grid_cell(1, 2, 0, 5, grid_w, grid_h, false, 1, 2));
    let volume_group =
        sheet_abs(sound_sheet.get_grid_cell(0, 2, 2, 5, grid_w, grid_h, false, 2, 3));

    // SetTitle relayouts before SetFont, so every group's stored client top
    // margin uses GUI CaptionFont even though its title later draws in BookFont
    // (C4Gui.h:993-1011). This is the same quirk as the Program group above.
    let titled_client = |group: IntRect| IntRect {
        x: group.x + 4,
        y: group.y + 4 + title_lh,
        w: group.w - 8,
        h: group.h - 8 - title_lh,
    };
    let child_abs = |client: IntRect, rect: IntRect| IntRect {
        x: client.x + rect.x,
        y: client.y + rect.y,
        ..rect
    };
    let frontend_client = titled_client(frontend_group);
    let game_client = titled_client(game_group);
    let frontend_controls = Aligner::new(
        IntRect {
            x: 0,
            y: 0,
            w: frontend_client.w,
            h: frontend_client.h,
        },
        indent_x1,
        indent_y2,
    );
    let game_controls = Aligner::new(
        IntRect {
            x: 0,
            y: 0,
            w: game_client.w,
            h: game_client.h,
        },
        indent_x1,
        indent_y2,
    );
    let checkboxes = [
        child_abs(
            frontend_client,
            frontend_controls.get_grid_cell(0, 1, 0, 2, -1, sound_check_h, true, 1, 1),
        ),
        child_abs(
            frontend_client,
            frontend_controls.get_grid_cell(0, 1, 1, 2, -1, sound_check_h, true, 1, 1),
        ),
        child_abs(
            game_client,
            game_controls.get_grid_cell(0, 1, 0, 2, -1, sound_check_h, true, 1, 1),
        ),
        child_abs(
            game_client,
            game_controls.get_grid_cell(0, 1, 1, 2, -1, sound_check_h, true, 1, 1),
        ),
    ];

    let volume_client = titled_client(volume_group);
    let volume_controls = Aligner::new(
        IntRect {
            x: 0,
            y: 0,
            w: volume_client.w,
            h: volume_client.h,
        },
        indent_x1,
        indent_y2,
    );
    let mut volume_headings = [IntRect::default(); 2];
    let mut quiet_labels = [IntRect::default(); 2];
    let mut loud_labels = [IntRect::default(); 2];
    let mut sliders = [IntRect::default(); 2];
    for i in 0..2 {
        let row = volume_controls.get_grid_cell(
            0,
            1,
            i as i32,
            2,
            -1,
            book.book.line_height + indent_y2 * 2 + 16,
            true,
            1,
            1,
        );
        let mut row = Aligner::new(row, 1, 0);
        volume_headings[i] = child_abs(volume_client, row.get_from_top(book.book.line_height));
        let (quiet_w, quiet_h) = book.book.measure("quiet", true);
        quiet_labels[i] = child_abs(volume_client, row.get_from_left(quiet_w, quiet_h));
        let (loud_w, loud_h) = book.book.measure("loud", true);
        loud_labels[i] = child_abs(volume_client, row.get_from_right(loud_w, loud_h));
        sliders[i] = child_abs(volume_client, row.get_centered(row.inner_width(), 16));
    }
    let sound = SoundSheetLayout {
        frontend_group,
        game_group,
        volume_group,
        checkboxes,
        volume_headings,
        quiet_labels,
        loud_labels,
        sliders,
    };
    let graphics = graphics_sheet_layout(sheet);
    let controls = ControlSheetLayout::from_sheet(sheet, book.book.line_height);
    let network = NetworkSheetLayout::from_sheet(sheet, book.book.line_height);

    OptionsDlgLayout {
        client,
        title_center,
        title_label,
        back_button,
        tabular,
        paper,
        tab_clips,
        tab_icons,
        tab_captions,
        focus_highlight,
        sheet,
        language_label,
        language_combo,
        language_info,
        font_label,
        font_face_combo,
        font_size_combo,
        white_chat_label,
        ingame_check,
        lobby_check,
        timestamps_check,
        preloading_check,
        group,
        weak_label,
        strong_label,
        slider,
        reset_button,
        advanced_button,
        sound,
        graphics,
        controls,
        network,
    }
}

// ---------------------------------------------------------------------------
// Assets and state
// ---------------------------------------------------------------------------

/// Graphics.c4g assets used by the options dialog (C4Startup.cpp:58-81 and
/// C4GUI::Resource::Load, C4Gui.cpp:1090-1110).
pub struct OptionsDlgAssets {
    /// `LoaderGoldmine1.png` 3840x2880 — the startup loader background
    /// (C4Startup.h:25), stretched fullscreen by Screen::Draw
    /// (C4Gui.cpp:669-682).
    pub background: ImageData,
    /// `StartupDlgPaper.png` 628x483 — tabular paper, stretched.
    pub paper: ImageData,
    /// `StartupTabClip.png` 120x80 — tab background, drawn 1:1.
    pub tab_clip: ImageData,
    /// `StartupOptionIcons.png` 192x32 — six 32x32 tab icons.
    pub option_icons: ImageData,
    /// `StartupBookScroll.png` 48x48 — slider bar/arrow/pin facets
    /// (ScrollBarFacets::Set, C4Gui.cpp:109-121).
    pub book_scroll: ImageData,
    /// `StartupContext.png` 32x16 — combo side arrow, phase 0 = 16x16
    /// (C4Startup.cpp:64-65).
    pub context_arrow: ImageData,
    /// `GUICheckBox.png` 128x32 — four 32x32 check box phases.
    pub checkbox: ImageData,
    /// `GUIButtonHighlight.png` 16x16 — additive focus/hover overlay.
    pub button_highlight: ImageData,
    /// `GUIButton.png` 128x32 — 3-slice bar of the standard Back button.
    pub button: ImageData,
    /// `Control.png` — source of `fctKeyboard`, `fctCommand` and `fctKey`
    /// (`C4GraphicsResource.cpp:200-203`). Absent falls back to text buttons.
    pub control: Option<ImageData>,
    /// `Gamepad.png` — `fctGamepad`, its own image with an 80px phase width
    /// (`C4GraphicsResource.cpp:229`).
    pub gamepad: Option<ImageData>,
}

/// Mutable display state of the Program sheet. Defaults mirror a fresh US
/// config on macOS (C4Config.cpp:385-404) — the state the reference capture
/// shows.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ProgramSheetState {
    /// Verbatim `Config.General.Language` until the user selects a combo row.
    pub language: String,
    /// `Config.General.LanguageEx`, recomposed by `UpdateLanguage` for a known
    /// primary language.
    pub language_ex: String,
    /// `C4Language::Infos` in native local/pack discovery order.
    pub language_infos: Vec<LanguageInfo>,
    /// Lang combo text, `"{CC} - {Name}"` (UpdateLanguage, ctor 1196-1232).
    pub language_text: String,
    /// Language pack info line (`IDS_LANG_INFO` of the active pack).
    pub language_info: String,
    /// Active `IDS_CTL_NOLANGINFO`, retained when the configured code is not
    /// present in the catalog.
    pub no_language_info: String,
    /// `Config.General.RXFontName` (default "Endeavour").
    pub font_face: String,
    /// `Config.General.RXFontSize` (default "14").
    pub font_size: String,
    pub white_chat_ingame: bool,
    pub white_chat_lobby: bool,
    pub show_log_timestamps: bool,
    /// Default false on macOS (C4Config.cpp:400-404).
    pub preloading: bool,
    /// Slider value 0..=100; `FairCrewStrength2Slider(1000) = 9`
    /// (C4StartupOptionsDlg.cpp:1061-1065).
    pub fair_crew_slider: i32,
    /// Raw `Config.General.FairCrewStrength` (`DefCrewStrength` on disk).
    /// This is separate because the native conversions truncate and are
    /// intentionally not mathematical inverses.
    pub fair_crew_strength: i32,
}

/// One checkbox on the classic Sound sheet, in C++ construction/focus order
/// (`C4StartupOptionsDlg.cpp:932-959`).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum SoundCheckboxId {
    FrontendMusic,
    FrontendSoundEffects,
    GameMusic,
    GameSoundEffects,
}

impl SoundCheckboxId {
    pub const ALL: [Self; 4] = [
        Self::FrontendMusic,
        Self::FrontendSoundEffects,
        Self::GameMusic,
        Self::GameSoundEffects,
    ];

    const fn index(self) -> usize {
        match self {
            Self::FrontendMusic => 0,
            Self::FrontendSoundEffects => 1,
            Self::GameMusic => 2,
            Self::GameSoundEffects => 3,
        }
    }
}

/// One pointer-only volume slider on the classic Sound sheet, in construction
/// order (`C4StartupOptionsDlg.cpp:967-984`).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum SoundVolumeId {
    Music,
    SoundEffects,
}

impl SoundVolumeId {
    pub const ALL: [Self; 2] = [Self::Music, Self::SoundEffects];

    const fn index(self) -> usize {
        match self {
            Self::Music => 0,
            Self::SoundEffects => 1,
        }
    }
}

/// Sound names emitted separately from value mutations so the app can apply
/// C++ callback ordering exactly (notably the old `FESamples` value gates the
/// click which precedes a checkbox mutation).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum SoundSheetSound {
    ArrowHit,
    Command,
}

/// GUI feedback shared by non-Sound sheets. This alias keeps the existing
/// Sound-sheet API while giving top-level dialog actions a sheet-neutral type.
pub type GuiSound = SoundSheetSound;

/// Ordered effects produced by one Sound-sheet input operation.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SoundSheetAction {
    GuiSound(SoundSheetSound),
    CheckboxChanged { id: SoundCheckboxId, checked: bool },
    VolumeChanged { id: SoundVolumeId, value: u8 },
    TestSound(SoundSheetSound),
}

/// Live values displayed by the classic Sound sheet. Volumes use the C++
/// callback domain `0..=100`, not normalized mixer floats.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SoundSheetState {
    pub frontend_music: bool,
    pub frontend_sound_effects: bool,
    pub game_music: bool,
    pub game_sound_effects: bool,
    pub music_volume: u8,
    pub sound_effects_volume: u8,
}

impl SoundSheetState {
    pub fn new(
        frontend_music: bool,
        frontend_sound_effects: bool,
        game_music: bool,
        game_sound_effects: bool,
        music_volume: u8,
        sound_effects_volume: u8,
    ) -> Self {
        Self {
            frontend_music,
            frontend_sound_effects,
            game_music,
            game_sound_effects,
            music_volume: music_volume.min(100),
            sound_effects_volume: sound_effects_volume.min(100),
        }
    }

    pub const fn checkbox(&self, id: SoundCheckboxId) -> bool {
        match id {
            SoundCheckboxId::FrontendMusic => self.frontend_music,
            SoundCheckboxId::FrontendSoundEffects => self.frontend_sound_effects,
            SoundCheckboxId::GameMusic => self.game_music,
            SoundCheckboxId::GameSoundEffects => self.game_sound_effects,
        }
    }

    pub const fn volume(&self, id: SoundVolumeId) -> u8 {
        match id {
            SoundVolumeId::Music => self.music_volume,
            SoundVolumeId::SoundEffects => self.sound_effects_volume,
        }
    }

    fn set_checkbox(&mut self, id: SoundCheckboxId, checked: bool) {
        match id {
            SoundCheckboxId::FrontendMusic => self.frontend_music = checked,
            SoundCheckboxId::FrontendSoundEffects => self.frontend_sound_effects = checked,
            SoundCheckboxId::GameMusic => self.game_music = checked,
            SoundCheckboxId::GameSoundEffects => self.game_sound_effects = checked,
        }
    }

    fn set_volume(&mut self, id: SoundVolumeId, value: u8) {
        match id {
            SoundVolumeId::Music => self.music_volume = value.min(100),
            SoundVolumeId::SoundEffects => self.sound_effects_volume = value.min(100),
        }
    }
}

impl Default for SoundSheetState {
    fn default() -> Self {
        Self::new(true, true, true, true, 100, 100)
    }
}

impl Default for ProgramSheetState {
    fn default() -> Self {
        let mut state = Self {
            language: "US - English".into(),
            language_ex: String::new(),
            language_infos: vec![LanguageInfo {
                code_bytes: *b"US",
                code: "US".into(),
                name: "English".into(),
                info: "Original language pack by RedWolf Design.".into(),
                fallback: String::new(),
                charset: String::new(),
            }],
            language_text: String::new(),
            language_info: String::new(),
            no_language_info: "Language pack not available.".into(),
            font_face: "Endeavour".into(),
            font_size: "14".into(),
            white_chat_ingame: false,
            white_chat_lobby: false,
            show_log_timestamps: false,
            preloading: false,
            fair_crew_slider: 9,
            fair_crew_strength: 1_000,
        };
        state.refresh_language();
        state
    }
}

impl ProgramSheetState {
    /// Installs the process-global language catalog and applies the same
    /// constructor-time `UpdateLanguage` projection as the C++ dialog.
    pub fn set_language_catalog(
        &mut self,
        language: impl Into<String>,
        language_ex: impl Into<String>,
        infos: Vec<LanguageInfo>,
    ) {
        self.language = language.into();
        self.language_ex = language_ex.into();
        self.language_infos = infos;
        self.refresh_language();
    }

    /// Applies `OnLangComboSelChange`. Returns false for a stale code that no
    /// longer exists in the catalog.
    pub fn select_language(&mut self, code: &str) -> bool {
        let Some(info) = self
            .language_infos
            .iter()
            .find(|info| info.code == code || info.matches_code(code))
        else {
            return false;
        };
        self.language = info.code.clone();
        self.refresh_language();
        true
    }

    /// Refreshes both font combo captions after the app successfully applies
    /// the new game font and recreates the dialog.
    pub fn set_font(&mut self, face: impl Into<String>, size: i32) {
        self.font_face = face.into();
        self.font_size = size.to_string();
    }

    /// Loads the raw config strength and derives the constructor-time slider
    /// value through the native lossy inverse mapping.
    pub fn set_fair_crew_strength(&mut self, strength: i32) {
        self.fair_crew_strength = strength;
        self.fair_crew_slider = fair_crew_strength_to_slider(strength).clamp(0, 100);
    }

    fn refresh_language(&mut self) {
        let selected = self
            .language_infos
            .iter()
            .find(|info| info.code == self.language || info.matches_code(&self.language));
        if let Some(info) = selected {
            self.language_text = format!("{} - {}", info.code, info.name);
            self.language_info = info.info.clone();
            self.language_ex = compose_language_ex(info);
        } else {
            self.language_text = format!("unknown ({})", self.language);
            self.language_info = self.no_language_info.clone();
        }
    }
}

/// `C4StartupOptionsDlg::FairCrewSlider2Strength`.
pub fn fair_crew_slider_to_strength(slider: i32) -> i32 {
    ((f64::from(slider) / 9.5).powf(1.5) * 1_000.0) as i32
}

/// `C4StartupOptionsDlg::FairCrewStrength2Slider`.
pub fn fair_crew_strength_to_slider(strength: i32) -> i32 {
    ((f64::from(strength) / 1_000.0).powf(1.0 / 1.5) * 9.5) as i32
}

/// `C4StartupOptionsDlg::UpdateLanguage` fallback composition. Substring
/// checks and fallback casing intentionally retain the native quirks.
pub fn compose_language_ex(info: &LanguageInfo) -> String {
    let mut language_ex = info.code.clone();
    if !info.fallback.is_empty() {
        language_ex.push(',');
        let mut condensed = String::new();
        for segment in info.fallback.split(',') {
            let segment = segment.as_bytes();
            let start = segment
                .iter()
                .position(|byte| !matches!(*byte, b' ' | b'\t' | b'\r' | b'\n'))
                .unwrap_or(segment.len());
            let code = &segment[start..(start + 2).min(segment.len())];
            if code.is_empty() {
                continue;
            }
            if !condensed.is_empty() {
                condensed.push(',');
            }
            condensed.push_str(&String::from_utf8_lossy(code));
        }
        language_ex.push_str(&condensed);
    }
    for fallback in ["US", "DE"] {
        if !language_ex.contains(fallback) {
            if !language_ex.is_empty() {
                language_ex.push(',');
            }
            language_ex.push_str(fallback);
        }
    }
    language_ex
}

/// One `C4GUI::Tabular::Sheet` in constructor order
/// (`C4StartupOptionsDlg.cpp:663-668`).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum OptionsSheet {
    #[default]
    Program,
    Graphics,
    Sound,
    Keyboard,
    Gamepad,
    Network,
}

impl OptionsSheet {
    const ALL: [Self; 6] = [
        Self::Program,
        Self::Graphics,
        Self::Sound,
        Self::Keyboard,
        Self::Gamepad,
        Self::Network,
    ];

    const fn index(self) -> usize {
        match self {
            Self::Program => 0,
            Self::Graphics => 1,
            Self::Sound => 2,
            Self::Keyboard => 3,
            Self::Gamepad => 4,
            Self::Network => 5,
        }
    }

    fn wrapping_offset(self, delta: isize) -> Self {
        let len = Self::ALL.len() as isize;
        let index = (self.index() as isize + delta).rem_euclid(len) as usize;
        Self::ALL[index]
    }
}

/// Observable callbacks from the C++ options dialog chrome.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum OptionsDlgAction {
    /// `C4StartupOptionsDlg::DoBack`; the app owns validation/persistence.
    Back,
    /// User-selected tab (`Tabular::SelectionChanged(true)`).
    SheetChanged(OptionsSheet),
    /// `BoolConfig::OnCheckChange` updated `Config.General.ShowLogTimestamps`.
    ShowLogTimestampsChanged(bool),
    /// Open the language combo's classic context-menu list.
    OpenLanguageCombo,
    /// Open one of the two native fixed-entry font context menus.
    OpenFontFaceCombo,
    OpenFontSizeCombo,
    WhiteChatIngameChanged(bool),
    WhiteChatLobbyChanged(bool),
    PreloadingChanged(bool),
    /// Raw `Config.General.FairCrewStrength`, after the exponential mapping.
    FairCrewStrengthChanged(i32),
    /// GUI feedback from the Program sheet, ordered separately from value
    /// callbacks so the app can mirror native scrollbar dispatch.
    ProgramGuiSound(GuiSound),
    /// Open the native Yes/No reset confirmation.
    ResetConfiguration,
    /// Open the generic advanced configuration editor.
    OpenAdvancedSettings,
    /// One ordered callback/feedback effect from the fully implemented Sound
    /// sheet. Ordering inside the outer action vector is observable.
    Sound(SoundSheetAction),
    Graphics(GraphicsSheetAction),
    OpenGraphicsScaleText,
    BeginControlCapture(ControlCaptureTarget),
    ResetControlBindings(ControlDevice),
    /// A different gamepad control set became active. The app owns the
    /// corresponding device-open lifecycle.
    GamepadDeviceSelected(usize),
    GamepadGuiControlChanged(bool),
    OpenNetworkText(NetworkTextField),
    NetworkPortEnabledChanged {
        id: NetworkPortId,
        enabled: bool,
    },
    NetworkCheckboxChanged {
        id: NetworkCheckboxId,
        checked: bool,
    },
}

/// Focusable Program-sheet controls in exact nested construction order.
/// The fair-crew scrollbar inherits `Element`, not `Control`, and therefore is
/// deliberately absent from keyboard/gamepad traversal.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum OptionsProgramFocusTarget {
    LanguageCombo,
    FontFaceCombo,
    FontSizeCombo,
    WhiteChatIngame,
    WhiteChatLobby,
    ShowLogTimestamps,
    Preloading,
    ResetButton,
    AdvancedButton,
}

impl OptionsProgramFocusTarget {
    const ALL: [Self; 9] = [
        Self::LanguageCombo,
        Self::FontFaceCombo,
        Self::FontSizeCombo,
        Self::WhiteChatIngame,
        Self::WhiteChatLobby,
        Self::ShowLogTimestamps,
        Self::Preloading,
        Self::ResetButton,
        Self::AdvancedButton,
    ];

    const fn is_combo(self) -> bool {
        matches!(
            self,
            Self::LanguageCombo | Self::FontFaceCombo | Self::FontSizeCombo
        )
    }

    const fn is_checkbox(self) -> bool {
        matches!(
            self,
            Self::WhiteChatIngame
                | Self::WhiteChatLobby
                | Self::ShowLogTimestamps
                | Self::Preloading
        )
    }

    const fn is_button(self) -> bool {
        matches!(self, Self::ResetButton | Self::AdvancedButton)
    }
}

/// Focusable Graphics-sheet controls in exact nested construction order.
/// The scale and smoke scrollbars inherit `Element`, not `Control`, and the
/// spinbox arrows are part of the single edit control.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum OptionsGraphicsFocusTarget {
    DisplayModeCombo,
    ApplyScale,
    ScaleEdit,
    Checkbox(GraphicsCheckboxId),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum OptionsFocus {
    None,
    Back,
    Tabular,
    Program(OptionsProgramFocusTarget),
    Graphics(OptionsGraphicsFocusTarget),
    SoundCheckbox(SoundCheckboxId),
    Control(ControlSheetHit),
    Network(NetworkSheetHit),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum OptionsHit {
    Back,
    Tab(OptionsSheet),
    Program(OptionsProgramFocusTarget),
    FairCrewSlider(ProgramSliderPart),
    SoundCheckbox(SoundCheckboxId),
    SoundSlider(SoundVolumeId, SoundSliderPart),
    Graphics(GraphicsHitTarget),
    Control(ControlSheetHit),
    Network(NetworkSheetHit),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum SoundSliderPart {
    DecrementArrow,
    Track,
    IncrementArrow,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum SoundSliderDirection {
    Decrement,
    Increment,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum ProgramSliderPart {
    DecrementArrow,
    Track,
    IncrementArrow,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum ProgramSliderDirection {
    Decrement,
    Increment,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum GraphicsSliderDirection {
    Decrement,
    Increment,
}

/// Live interaction and presentation state for the pixel-parity options
/// dialog. It deliberately emits external work as actions instead of owning
/// configuration persistence.
pub struct OptionsDlgState {
    program: ProgramSheetState,
    sound: SoundSheetState,
    graphics: GraphicsSheetState,
    controls: ControlSheetState,
    network: NetworkSheetState,
    active_sheet: OptionsSheet,
    /// The C++ ctor explicitly focuses the tabular after adding all controls
    /// (`C4StartupOptionsDlg.cpp:1039`).
    focus: OptionsFocus,
    layout: Option<OptionsDlgLayout>,
    labels: OptionsLabels,
    pointer_position: Option<GuiPoint>,
    hovered: Option<OptionsHit>,
    pressed_back: bool,
    back_pointer_owned: bool,
    pointer_down: bool,
    captured_fair_crew_slider: bool,
    pressed_fair_crew_arrow: Option<ProgramSliderDirection>,
    fair_crew_slider_position: Option<i32>,
    pressed_program_button: Option<OptionsProgramFocusTarget>,
    captured_sound_slider: Option<SoundVolumeId>,
    pressed_sound_arrow: Option<(SoundVolumeId, SoundSliderDirection)>,
    sound_slider_positions: [Option<i32>; 2],
    captured_graphics_slider: Option<GraphicsSliderId>,
    pressed_graphics_arrow: Option<(GraphicsSliderId, GraphicsSliderDirection)>,
    graphics_slider_positions: [Option<i32>; 2],
    pressed_release_target: Option<OptionsHit>,
}

impl Default for OptionsDlgState {
    fn default() -> Self {
        Self::new(ProgramSheetState::default())
    }
}

impl OptionsDlgState {
    pub fn new(program: ProgramSheetState) -> Self {
        Self::with_sound(program, SoundSheetState::default())
    }

    /// Constructs the dialog from both directly bound option groups while
    /// preserving [`Self::new`] for existing Program-only callers.
    pub fn with_sound(program: ProgramSheetState, sound: SoundSheetState) -> Self {
        Self::with_all(
            program,
            sound,
            GraphicsSheetState::default(),
            ControlSheetState::default(),
            NetworkSheetState::default(),
        )
    }

    pub fn with_all(
        program: ProgramSheetState,
        sound: SoundSheetState,
        graphics: GraphicsSheetState,
        controls: ControlSheetState,
        network: NetworkSheetState,
    ) -> Self {
        Self {
            program,
            sound,
            graphics,
            controls,
            network,
            active_sheet: OptionsSheet::Program,
            focus: OptionsFocus::Tabular,
            layout: None,
            labels: OptionsLabels::default(),
            pointer_position: None,
            hovered: None,
            pressed_back: false,
            back_pointer_owned: false,
            pointer_down: false,
            captured_fair_crew_slider: false,
            pressed_fair_crew_arrow: None,
            fair_crew_slider_position: None,
            pressed_program_button: None,
            captured_sound_slider: None,
            pressed_sound_arrow: None,
            sound_slider_positions: [None; 2],
            captured_graphics_slider: None,
            pressed_graphics_arrow: None,
            graphics_slider_positions: [None; 2],
            pressed_release_target: None,
        }
    }

    /// Refreshes the cached C++ integer layout used for pointer hit-testing.
    /// Installs the active language table's Options strings. C++ resolves them
    /// once in the constructor, so the caller reinstalls them by rebuilding the
    /// dialog - which is exactly what a language change does.
    pub fn set_labels(&mut self, labels: OptionsLabels) {
        self.labels = labels;
    }

    pub fn labels(&self) -> &OptionsLabels {
        &self.labels
    }

    pub fn resize(&mut self, width: i32, height: i32, gui: &ClonkFontSet, book: &BookFonts) {
        self.layout = Some(options_dlg_layout_with_labels(
            width.max(1),
            height.max(1),
            gui,
            book,
            &self.labels,
        ));
        self.captured_fair_crew_slider = false;
        self.pressed_fair_crew_arrow = None;
        self.pressed_program_button = None;
        self.captured_sound_slider = None;
        self.pressed_sound_arrow = None;
        self.captured_graphics_slider = None;
        self.pressed_graphics_arrow = None;
        self.pressed_release_target = None;
        self.back_pointer_owned = false;
        self.pressed_back = false;
        self.fair_crew_slider_position = None;
        self.sound_slider_positions = [None; 2];
        self.graphics_slider_positions = [None; 2];
        self.sync_fair_crew_slider_position();
        self.sync_sound_slider_positions();
        self.sync_graphics_slider_positions();
        self.hovered = self.pointer_position.and_then(|point| {
            self.layout.as_ref().and_then(|layout| {
                options_hit_test(
                    layout,
                    self.active_sheet,
                    &self.controls,
                    &self.network,
                    point,
                )
            })
        });
    }

    pub const fn active_sheet(&self) -> OptionsSheet {
        self.active_sheet
    }

    /// Restores the tab that owned the advanced-settings dialog after the
    /// options screen is recreated. Native `RecreateDialog` reconstructs the
    /// controls, then the caller reselects the previously active sheet.
    pub fn restore_sheet(&mut self, sheet: OptionsSheet) {
        let _ = self.handle_pointer_left();
        self.active_sheet = sheet;
        self.focus = OptionsFocus::Tabular;
    }

    pub fn program(&self) -> &ProgramSheetState {
        &self.program
    }

    pub fn program_mut(&mut self) -> &mut ProgramSheetState {
        self.fair_crew_slider_position = None;
        &mut self.program
    }

    /// Applies one combo selection and mirrors `RecreateDialog(true)`'s
    /// reset to the Program sheet with the tabular focused.
    pub fn select_language(&mut self, code: &str) -> bool {
        if !self.program.select_language(code) {
            return false;
        }
        self.active_sheet = OptionsSheet::Program;
        self.focus = OptionsFocus::Tabular;
        self.pointer_down = false;
        self.back_pointer_owned = false;
        self.pressed_back = false;
        true
    }

    pub fn sound(&self) -> &SoundSheetState {
        &self.sound
    }

    pub fn graphics(&self) -> &GraphicsSheetState {
        &self.graphics
    }

    pub fn graphics_mut(&mut self) -> &mut GraphicsSheetState {
        self.graphics_slider_positions = [None; 2];
        &mut self.graphics
    }

    pub fn controls(&self) -> &ControlSheetState {
        &self.controls
    }

    pub fn controls_mut(&mut self) -> &mut ControlSheetState {
        &mut self.controls
    }

    pub fn network(&self) -> &NetworkSheetState {
        &self.network
    }

    pub fn network_mut(&mut self) -> &mut NetworkSheetState {
        &mut self.network
    }

    /// Replaces all values from authoritative app configuration. Slider pixel
    /// positions are re-derived on the next layout sync.
    pub fn set_sound_state(&mut self, sound: SoundSheetState) {
        self.sound = sound;
        self.sound_slider_positions = [None; 2];
        self.sync_sound_slider_positions();
    }

    /// `C4StartupOptionsDlg::KeyMusicToggle` uses `SetChecked`, which updates
    /// the FE Music visual without replaying the checkbox callback or click.
    /// Call this only when Options is the top key-owning dialog: a modal above
    /// Options lets global F3 mutate config while the hidden checkbox stays
    /// stale until that modal closes/recreation occurs.
    pub fn sync_frontend_music_from_f3(&mut self, enabled: bool) {
        self.sound.frontend_music = enabled;
    }

    pub const fn focused_sound_checkbox(&self) -> Option<SoundCheckboxId> {
        match self.focus {
            OptionsFocus::SoundCheckbox(id) => Some(id),
            _ => None,
        }
    }

    pub const fn focused_program_control(&self) -> Option<OptionsProgramFocusTarget> {
        match self.focus {
            OptionsFocus::Program(target) => Some(target),
            _ => None,
        }
    }

    pub const fn focused_graphics_control(&self) -> Option<OptionsGraphicsFocusTarget> {
        match self.focus {
            OptionsFocus::Graphics(target) => Some(target),
            _ => None,
        }
    }

    pub const fn focused_control_sheet_control(&self) -> Option<ControlSheetHit> {
        match self.focus {
            OptionsFocus::Control(target) => Some(target),
            _ => None,
        }
    }

    pub const fn focused_network_control(&self) -> Option<NetworkSheetHit> {
        match self.focus {
            OptionsFocus::Network(target) => Some(target),
            _ => None,
        }
    }

    pub const fn pointer_position(&self) -> Option<GuiPoint> {
        self.pointer_position
    }

    /// Returns the native tooltip target under `point` without resolving a
    /// language string. Tooltip hit regions deliberately differ from action
    /// hit regions: C++ attaches tips to complete elements (including a
    /// checkbox caption), and a child with no own tip inherits its nearest
    /// parent window's tip (`C4GUI::Element::GetToolTip`).
    pub fn tooltip_at(&self, point: GuiPoint, book: &BookFonts) -> Option<StartupTooltip> {
        let layout = self.layout.as_ref()?;
        options_tooltip_at(
            layout,
            self.active_sheet,
            &self.program,
            &self.controls,
            &self.labels,
            book,
            point,
        )
    }

    /// Resolves the tooltip target under the controller's current pointer.
    pub fn tooltip(&self, book: &BookFonts) -> Option<StartupTooltip> {
        self.tooltip_at(self.pointer_position?, book)
    }

    pub const fn language_combo_focused(&self) -> bool {
        matches!(
            self.focus,
            OptionsFocus::Program(OptionsProgramFocusTarget::LanguageCombo)
        )
    }

    pub fn language_combo_anchor(&self) -> Option<GuiPoint> {
        self.layout.as_ref().map(|layout| {
            GuiPoint::new(
                layout.language_combo.x as f32,
                (layout.language_combo.y + layout.language_combo.h) as f32,
            )
        })
    }

    pub fn font_face_combo_anchor(&self) -> Option<GuiPoint> {
        self.layout.as_ref().map(|layout| {
            GuiPoint::new(
                layout.font_face_combo.x as f32,
                (layout.font_face_combo.y + layout.font_face_combo.h) as f32,
            )
        })
    }

    pub fn font_size_combo_anchor(&self) -> Option<GuiPoint> {
        self.layout.as_ref().map(|layout| {
            GuiPoint::new(
                layout.font_size_combo.x as f32,
                (layout.font_size_combo.y + layout.font_size_combo.h) as f32,
            )
        })
    }

    pub fn graphics_display_combo_anchor(&self) -> Option<GuiPoint> {
        self.layout.as_ref().map(|layout| {
            GuiPoint::new(
                layout.graphics.display_mode_combo.x as f32,
                (layout.graphics.display_mode_combo.y + layout.graphics.display_mode_combo.h)
                    as f32,
            )
        })
    }

    pub fn set_pointer_position(&mut self, position: Option<GuiPoint>) {
        self.pointer_position = position;
        self.hovered = position.and_then(|point| {
            self.layout.as_ref().and_then(|layout| {
                options_hit_test(
                    layout,
                    self.active_sheet,
                    &self.controls,
                    &self.network,
                    point,
                )
            })
        });
        if position.is_none() {
            self.pressed_back = false;
        }
    }

    pub fn pointer_left(&mut self) {
        let _ = self.handle_pointer_left();
    }

    /// Pointer state loss. `ScrollBar::MouseLeave` clears held arrow state
    /// silently; the return type matches the other pointer entry points for
    /// callers that route their action vectors uniformly.
    pub fn handle_pointer_left(&mut self) -> Vec<OptionsDlgAction> {
        self.pointer_position = None;
        self.hovered = None;
        self.pressed_back = false;
        self.back_pointer_owned = false;
        self.pointer_down = false;
        self.captured_fair_crew_slider = false;
        self.pressed_fair_crew_arrow = None;
        self.pressed_program_button = None;
        self.captured_sound_slider = None;
        self.pressed_sound_arrow = None;
        self.captured_graphics_slider = None;
        self.pressed_graphics_arrow = None;
        self.pressed_release_target = None;
        Vec::new()
    }

    pub fn handle_pointer_move(&mut self, position: GuiPoint) -> Vec<OptionsDlgAction> {
        if self.back_pointer_owned {
            self.set_pointer_position(Some(position));
            self.pressed_back = self.hovered == Some(OptionsHit::Back);
            return Vec::new();
        }
        if self.captured_fair_crew_slider {
            self.pointer_position = Some(position);
            self.hovered = None;
            return self.update_fair_crew_slider_from_pointer(position);
        }
        if let Some(id) = self.captured_sound_slider {
            self.pointer_position = Some(position);
            self.hovered = None;
            return self.update_sound_slider_from_pointer(id, position);
        }
        if let Some(id) = self.captured_graphics_slider {
            self.pointer_position = Some(position);
            self.hovered = None;
            return self.update_graphics_slider_from_pointer(id, position);
        }
        let previous_fair_crew_arrow = self.pressed_fair_crew_arrow;
        let previous_arrow = self.pressed_sound_arrow;
        let previous_graphics_arrow = self.pressed_graphics_arrow;
        self.set_pointer_position(Some(position));
        if self.pointer_down
            && (previous_fair_crew_arrow.is_some()
                || matches!(self.hovered, Some(OptionsHit::FairCrewSlider(_))))
        {
            return self.update_held_fair_crew_arrow(position, previous_fair_crew_arrow);
        }
        if self.pointer_down
            && (previous_arrow.is_some()
                || matches!(self.hovered, Some(OptionsHit::SoundSlider(_, _))))
        {
            return self.update_held_sound_arrow(position, previous_arrow);
        }
        if self.pointer_down
            && (previous_graphics_arrow.is_some()
                || matches!(
                    self.hovered,
                    Some(OptionsHit::Graphics(GraphicsHitTarget::Slider { .. }))
                ))
        {
            return self.update_held_graphics_arrow(position, previous_graphics_arrow);
        }
        Vec::new()
    }

    pub fn handle_pointer_down(&mut self, position: GuiPoint) -> Vec<OptionsDlgAction> {
        self.pointer_down = true;
        self.back_pointer_owned = false;
        self.captured_fair_crew_slider = false;
        self.pressed_fair_crew_arrow = None;
        self.pressed_program_button = None;
        self.captured_sound_slider = None;
        self.pressed_sound_arrow = None;
        self.captured_graphics_slider = None;
        self.pressed_graphics_arrow = None;
        self.pressed_release_target = None;
        self.set_pointer_position(Some(position));
        match self.hovered {
            Some(OptionsHit::Back) => {
                self.back_pointer_owned = true;
                self.pressed_back = true;
                Vec::new()
            }
            Some(OptionsHit::Tab(sheet)) => {
                self.pressed_back = false;
                self.select_sheet(sheet)
            }
            Some(OptionsHit::Program(
                target @ (OptionsProgramFocusTarget::LanguageCombo
                | OptionsProgramFocusTarget::FontFaceCombo
                | OptionsProgramFocusTarget::FontSizeCombo),
            )) => {
                self.pressed_back = false;
                // The context menu captures the matching release. Do not
                // leave the underlying dialog in a held-pointer state when
                // that menu is cancelled instead of selecting an entry.
                self.pointer_down = false;
                self.activate_program_combo(target)
            }
            Some(OptionsHit::Program(
                OptionsProgramFocusTarget::WhiteChatIngame
                | OptionsProgramFocusTarget::WhiteChatLobby
                | OptionsProgramFocusTarget::ShowLogTimestamps
                | OptionsProgramFocusTarget::Preloading,
            )) => {
                self.pressed_back = false;
                Vec::new()
            }
            Some(OptionsHit::Program(
                OptionsProgramFocusTarget::ResetButton | OptionsProgramFocusTarget::AdvancedButton,
            )) => {
                self.pressed_back = false;
                self.pressed_release_target = self.hovered;
                Vec::new()
            }
            Some(OptionsHit::FairCrewSlider(part)) => {
                self.pressed_back = false;
                self.begin_fair_crew_slider_pointer(part, position)
            }
            Some(OptionsHit::SoundCheckbox(_)) => {
                self.pressed_back = false;
                Vec::new()
            }
            Some(OptionsHit::SoundSlider(id, part)) => {
                self.pressed_back = false;
                self.begin_sound_slider_pointer(id, part, position)
            }
            Some(OptionsHit::Graphics(hit)) => {
                self.pressed_back = false;
                match hit {
                    GraphicsHitTarget::Slider { id, part } => {
                        self.begin_graphics_slider_pointer(id, part, position)
                    }
                    GraphicsHitTarget::DisplayModeCombo | GraphicsHitTarget::ScaleEdit => {
                        // Like native ComboBox/Edit focus, the overlay opened by
                        // these controls consumes the matching release.
                        self.pointer_down = false;
                        self.activate_graphics_hit(hit)
                    }
                    GraphicsHitTarget::ScaleSpinbox(_) | GraphicsHitTarget::ApplyScale => {
                        self.pressed_release_target = self.hovered;
                        Vec::new()
                    }
                    GraphicsHitTarget::Checkbox(_) => Vec::new(),
                }
            }
            Some(OptionsHit::Control(hit)) => {
                self.pressed_back = false;
                if hit == ControlSheetHit::GamepadGui {
                    Vec::new()
                } else {
                    self.pressed_release_target = self.hovered;
                    Vec::new()
                }
            }
            Some(OptionsHit::Network(hit)) => {
                self.pressed_back = false;
                if matches!(hit, NetworkSheetHit::Text(_)) {
                    self.pointer_down = false;
                    self.activate_network_hit(hit)
                } else {
                    Vec::new()
                }
            }
            None => {
                self.pressed_back = false;
                Vec::new()
            }
        }
    }

    pub fn handle_pointer_up(&mut self, position: GuiPoint) -> Vec<OptionsDlgAction> {
        self.pointer_down = false;
        if self.captured_fair_crew_slider {
            self.captured_fair_crew_slider = false;
            let mut actions = self.update_fair_crew_slider_from_pointer(position);
            self.set_pointer_position(Some(position));
            self.pressed_fair_crew_arrow = None;
            self.back_pointer_owned = false;
            actions.extend(self.dispatch_pointer_up_target());
            return actions;
        }
        if let Some(id) = self.captured_sound_slider.take() {
            // Screen::MouseInput first calls StopDragging (and therefore the
            // forced final scrollbar callback), clears pDragElement, and only
            // then dispatches this same LeftUp to the element under the cursor.
            let mut actions = self.update_sound_slider_from_pointer(id, position);
            self.set_pointer_position(Some(position));
            self.pressed_sound_arrow = None;
            self.back_pointer_owned = false;
            actions.extend(self.dispatch_pointer_up_target());
            return actions;
        }
        if let Some(id) = self.captured_graphics_slider.take() {
            let mut actions = self.update_graphics_slider_from_pointer(id, position);
            self.set_pointer_position(Some(position));
            self.pressed_graphics_arrow = None;
            self.back_pointer_owned = false;
            actions.extend(self.dispatch_pointer_up_target());
            return actions;
        }
        self.set_pointer_position(Some(position));
        if self.pressed_fair_crew_arrow.take().is_some() {
            let released_inside_scrollbar = self
                .layout
                .as_ref()
                .is_some_and(|layout| rect_contains(&layout.slider, position));
            if released_inside_scrollbar {
                return vec![OptionsDlgAction::ProgramGuiSound(GuiSound::ArrowHit)];
            }
        }
        if let Some((id, _)) = self.pressed_sound_arrow.take() {
            let released_inside_scrollbar = self
                .layout
                .as_ref()
                .is_some_and(|layout| rect_contains(&layout.sound.slider(id), position));
            if released_inside_scrollbar {
                return vec![Self::sound_action(SoundSheetAction::GuiSound(
                    SoundSheetSound::ArrowHit,
                ))];
            }
        }
        self.pressed_graphics_arrow = None;
        if self.back_pointer_owned {
            self.back_pointer_owned = false;
            self.pressed_back = self.hovered == Some(OptionsHit::Back);
        }
        self.dispatch_pointer_up_target()
    }

    fn dispatch_pointer_up_target(&mut self) -> Vec<OptionsDlgAction> {
        let pressed_release_target = self.pressed_release_target.take();
        let activate_back = self.pressed_back && self.hovered == Some(OptionsHit::Back);
        self.pressed_back = false;
        if activate_back {
            return vec![OptionsDlgAction::Back];
        }
        if let Some(OptionsHit::Program(
            target @ (OptionsProgramFocusTarget::WhiteChatIngame
            | OptionsProgramFocusTarget::WhiteChatLobby
            | OptionsProgramFocusTarget::ShowLogTimestamps
            | OptionsProgramFocusTarget::Preloading),
        )) = self.hovered
        {
            return self.toggle_program_checkbox(target);
        }
        if let Some(OptionsHit::SoundCheckbox(id)) = self.hovered {
            return self.toggle_sound_checkbox(id);
        }
        if pressed_release_target == self.hovered {
            match pressed_release_target {
                Some(OptionsHit::Graphics(hit)) => return self.activate_graphics_hit(hit),
                Some(OptionsHit::Control(hit)) => return self.activate_control_hit(hit),
                Some(OptionsHit::Program(
                    target @ (OptionsProgramFocusTarget::ResetButton
                    | OptionsProgramFocusTarget::AdvancedButton),
                )) => return self.activate_program_button(target),
                _ => {}
            }
        }
        match self.hovered {
            Some(OptionsHit::Graphics(hit @ GraphicsHitTarget::Checkbox(_))) => {
                self.activate_graphics_hit(hit)
            }
            Some(OptionsHit::Control(ControlSheetHit::GamepadGui)) => {
                self.activate_control_hit(ControlSheetHit::GamepadGui)
            }
            Some(OptionsHit::Network(
                hit @ (NetworkSheetHit::PortCheck(_) | NetworkSheetHit::Checkbox(_)),
            )) => self.activate_network_hit(hit),
            _ => Vec::new(),
        }
    }

    pub fn handle_key_down(&mut self, key: KeyCode) -> Vec<OptionsDlgAction> {
        match key {
            // Dedicated options bindings: K_BACK + K_LEFT, plus the dialog's
            // OnEscape override (C4StartupOptionsDlg.cpp:615-620; header:37).
            KeyCode::Escape | KeyCode::Left => vec![OptionsDlgAction::Back],
            KeyCode::Up if self.focus == OptionsFocus::Tabular => {
                self.select_sheet(self.active_sheet.wrapping_offset(-1))
            }
            KeyCode::Down if self.focus == OptionsFocus::Tabular => {
                self.select_sheet(self.active_sheet.wrapping_offset(1))
            }
            KeyCode::Down | KeyCode::Space if matches!(self.focus, OptionsFocus::Program(target) if target.is_combo()) =>
            {
                let OptionsFocus::Program(target) = self.focus else {
                    unreachable!()
                };
                self.activate_program_combo(target)
            }
            KeyCode::Tab => self.handle_tab(false),
            KeyCode::Space if matches!(self.focus, OptionsFocus::Program(target) if target.is_checkbox()) =>
            {
                let OptionsFocus::Program(target) = self.focus else {
                    unreachable!()
                };
                self.toggle_program_checkbox(target)
            }
            KeyCode::Space if matches!(self.focus, OptionsFocus::SoundCheckbox(_)) => {
                let OptionsFocus::SoundCheckbox(id) = self.focus else {
                    unreachable!()
                };
                self.toggle_sound_checkbox(id)
            }
            KeyCode::Enter | KeyCode::Space if matches!(self.focus, OptionsFocus::Program(target) if target.is_button()) =>
            {
                let OptionsFocus::Program(target) = self.focus else {
                    unreachable!()
                };
                self.pressed_program_button = Some(target);
                Vec::new()
            }
            KeyCode::Enter | KeyCode::Space if self.focus == OptionsFocus::Back => {
                self.pressed_back = true;
                Vec::new()
            }
            _ => Vec::new(),
        }
    }

    /// Dispatches the digit mnemonics assigned to the visible Keyboard or
    /// Gamepad control-set buttons (`C4StartupOptionsDlg.cpp:251-278`).
    pub fn handle_hotkey(&mut self, character: char) -> Option<Vec<OptionsDlgAction>> {
        let set = "1234567890"
            .chars()
            .position(|hotkey| hotkey == character)?;
        let device = match self.active_sheet {
            OptionsSheet::Keyboard => ControlDevice::Keyboard,
            OptionsSheet::Gamepad => ControlDevice::Gamepad,
            _ => return None,
        };
        if set >= self.controls.visible_sets(device) {
            return None;
        }
        Some(self.activate_control_hit(ControlSheetHit::Set(set)))
    }

    pub fn handle_key_up(&mut self, key: KeyCode) -> Vec<OptionsDlgAction> {
        if matches!(key, KeyCode::Enter | KeyCode::Space) {
            if let Some(target) = self.pressed_program_button.take() {
                if self.focus == OptionsFocus::Program(target) {
                    return self.activate_program_button(target);
                }
            }
        }
        if matches!(key, KeyCode::Enter | KeyCode::Space)
            && self.focus == OptionsFocus::Back
            && self.pressed_back
        {
            self.pressed_back = false;
            return vec![OptionsDlgAction::Back];
        }
        Vec::new()
    }

    /// The generic `Dialog::AdvanceFocus` preorder flattened for the active
    /// Options sheet. Conditionally hidden controls remain in this raw order
    /// so a control hidden while focused is still the traversal anchor.
    fn focus_order(&self) -> Vec<OptionsFocus> {
        let mut order = vec![OptionsFocus::Back, OptionsFocus::Tabular];
        match self.active_sheet {
            OptionsSheet::Program => order.extend(
                OptionsProgramFocusTarget::ALL
                    .into_iter()
                    .map(OptionsFocus::Program),
            ),
            OptionsSheet::Graphics => {
                order.extend([
                    OptionsFocus::Graphics(OptionsGraphicsFocusTarget::DisplayModeCombo),
                    OptionsFocus::Graphics(OptionsGraphicsFocusTarget::ApplyScale),
                    OptionsFocus::Graphics(OptionsGraphicsFocusTarget::ScaleEdit),
                ]);
                order.extend(
                    GraphicsCheckboxId::ALL
                        .into_iter()
                        .map(|id| OptionsFocus::Graphics(OptionsGraphicsFocusTarget::Checkbox(id))),
                );
            }
            OptionsSheet::Sound => order.extend(
                SoundCheckboxId::ALL
                    .into_iter()
                    .map(OptionsFocus::SoundCheckbox),
            ),
            OptionsSheet::Keyboard | OptionsSheet::Gamepad => {
                let device = if self.active_sheet == OptionsSheet::Keyboard {
                    ControlDevice::Keyboard
                } else {
                    ControlDevice::Gamepad
                };
                order.extend(
                    (0..self.controls.visible_sets(device))
                        .map(|set| OptionsFocus::Control(ControlSheetHit::Set(set))),
                );
                order.extend(
                    (0..CONTROL_KEY_COUNT)
                        .map(|control| OptionsFocus::Control(ControlSheetHit::Key(control))),
                );
                if device == ControlDevice::Gamepad {
                    order.push(OptionsFocus::Control(ControlSheetHit::GamepadGui));
                }
                order.push(OptionsFocus::Control(ControlSheetHit::Reset));
            }
            OptionsSheet::Network => {
                for id in NetworkPortId::ALL {
                    order.push(OptionsFocus::Network(NetworkSheetHit::PortCheck(id)));
                    order.push(OptionsFocus::Network(NetworkSheetHit::Text(
                        NetworkTextField::Port(id),
                    )));
                }
                order.extend([
                    OptionsFocus::Network(NetworkSheetHit::Checkbox(
                        NetworkCheckboxId::UseAlternateServer,
                    )),
                    OptionsFocus::Network(NetworkSheetHit::Text(
                        NetworkTextField::AlternateServerAddress,
                    )),
                    OptionsFocus::Network(NetworkSheetHit::Checkbox(
                        NetworkCheckboxId::AutomaticUpdate,
                    )),
                    OptionsFocus::Network(NetworkSheetHit::Checkbox(NetworkCheckboxId::EnableUpnp)),
                    OptionsFocus::Network(NetworkSheetHit::Text(NetworkTextField::LocalName)),
                    OptionsFocus::Network(NetworkSheetHit::Text(NetworkTextField::Nick)),
                ]);
            }
        }
        order
    }

    fn focus_is_visible(&self, focus: OptionsFocus) -> bool {
        match focus {
            OptionsFocus::None => false,
            OptionsFocus::Back | OptionsFocus::Tabular => true,
            OptionsFocus::Program(_) => self.active_sheet == OptionsSheet::Program,
            OptionsFocus::Graphics(_) => self.active_sheet == OptionsSheet::Graphics,
            OptionsFocus::SoundCheckbox(_) => self.active_sheet == OptionsSheet::Sound,
            OptionsFocus::Control(target) => {
                let device = match self.active_sheet {
                    OptionsSheet::Keyboard => ControlDevice::Keyboard,
                    OptionsSheet::Gamepad => ControlDevice::Gamepad,
                    _ => return false,
                };
                match target {
                    ControlSheetHit::Set(set) => set < self.controls.visible_sets(device),
                    ControlSheetHit::Key(control) => control < CONTROL_KEY_COUNT,
                    ControlSheetHit::Reset => true,
                    ControlSheetHit::GamepadGui => {
                        device == ControlDevice::Gamepad
                            && self.controls.gamepad_gui_checkbox_visible()
                    }
                }
            }
            OptionsFocus::Network(target) => {
                if self.active_sheet != OptionsSheet::Network {
                    return false;
                }
                match target {
                    NetworkSheetHit::PortCheck(_) | NetworkSheetHit::Checkbox(_) => true,
                    NetworkSheetHit::Text(NetworkTextField::Port(id)) => {
                        self.network.port(id).enabled
                    }
                    NetworkSheetHit::Text(NetworkTextField::AlternateServerAddress) => {
                        self.network.use_alternate_server
                    }
                    NetworkSheetHit::Text(NetworkTextField::LocalName | NetworkTextField::Nick) => {
                        true
                    }
                }
            }
        }
    }

    /// Modifier-aware `Dialog::AdvanceFocus`: `backwards=true` is Shift+Tab
    /// (or gamepad Left), while false is Tab (or gamepad Right).
    pub fn handle_tab(&mut self, backwards: bool) -> Vec<OptionsDlgAction> {
        self.pressed_back = false;
        self.pressed_program_button = None;
        let order = self.focus_order();
        let len = order.len();
        let mut index = order
            .iter()
            .position(|candidate| *candidate == self.focus)
            .unwrap_or(if backwards { 0 } else { len - 1 });
        for _ in 0..len {
            index = if backwards {
                (index + len - 1) % len
            } else {
                (index + 1) % len
            };
            if self.focus_is_visible(order[index]) {
                self.focus = order[index];
                break;
            }
        }
        Vec::new()
    }

    /// `Ctrl+Tab`/`Ctrl+Shift+Tab` changes sheets at control priority and is
    /// independent of which child currently has focus.
    pub fn handle_ctrl_tab(&mut self, backwards: bool) -> Vec<OptionsDlgAction> {
        self.select_sheet(
            self.active_sheet
                .wrapping_offset(if backwards { -1 } else { 1 }),
        )
    }

    pub fn handle_gamepad_horizontal(&mut self, backwards: bool) -> Vec<OptionsDlgAction> {
        self.handle_tab(backwards)
    }

    /// A raw gamepad AnyLowButton DOWN. This must be called before any app
    /// aliasing to Enter: focused checkboxes toggle on DOWN at `PRIO_Ctrl`,
    /// whereas the dialog's lower-priority Enter handler returns false.
    pub fn handle_gamepad_low_down(&mut self) -> Vec<OptionsDlgAction> {
        match self.focus {
            OptionsFocus::SoundCheckbox(id) => self.toggle_sound_checkbox(id),
            OptionsFocus::Program(target) if target.is_combo() => {
                self.activate_program_combo(target)
            }
            OptionsFocus::Program(target) if target.is_checkbox() => {
                self.toggle_program_checkbox(target)
            }
            OptionsFocus::Program(target) if target.is_button() => {
                self.pressed_program_button = Some(target);
                Vec::new()
            }
            OptionsFocus::Program(_) => Vec::new(),
            OptionsFocus::Back => {
                self.pressed_back = true;
                Vec::new()
            }
            OptionsFocus::None
            | OptionsFocus::Tabular
            | OptionsFocus::Graphics(_)
            | OptionsFocus::Control(_)
            | OptionsFocus::Network(_) => Vec::new(),
        }
    }

    pub fn handle_gamepad_low_up(&mut self) -> Vec<OptionsDlgAction> {
        if let Some(target) = self.pressed_program_button.take() {
            if self.focus == OptionsFocus::Program(target) {
                return self.activate_program_button(target);
            }
        }
        if self.focus == OptionsFocus::Back && self.pressed_back {
            self.pressed_back = false;
            vec![OptionsDlgAction::Back]
        } else {
            Vec::new()
        }
    }

    pub fn handle_gamepad_high_down(&mut self) -> Vec<OptionsDlgAction> {
        vec![OptionsDlgAction::Back]
    }

    /// One rendered frame while a scrollbar arrow remains held. C++ advances
    /// by one *thumb pixel* from `ScrollBar::DrawElement`, then invokes the
    /// value callback even when integer range conversion repeats a value.
    pub fn advance_frame(&mut self) -> Vec<OptionsDlgAction> {
        if self.active_sheet == OptionsSheet::Program {
            let Some(direction) = self.pressed_fair_crew_arrow else {
                return Vec::new();
            };
            let Some(rect) = self.layout.as_ref().map(|layout| layout.slider) else {
                return Vec::new();
            };
            let max_scroll = program_slider_max_scroll(rect);
            let old = self.fair_crew_slider_position(rect);
            let new = match direction {
                ProgramSliderDirection::Decrement => old.saturating_sub(1),
                ProgramSliderDirection::Increment => (old + 1).min(max_scroll),
            };
            if new == old {
                return Vec::new();
            }
            return self.set_fair_crew_slider_position(rect, new);
        }
        if self.active_sheet == OptionsSheet::Sound {
            let Some((id, direction)) = self.pressed_sound_arrow else {
                return Vec::new();
            };
            let Some(rect) = self.layout.as_ref().map(|layout| layout.sound.slider(id)) else {
                return Vec::new();
            };
            let max_scroll = sound_slider_max_scroll(rect);
            let old = self.sound_slider_position(id, rect);
            let new = match direction {
                SoundSliderDirection::Decrement => old.saturating_sub(1),
                SoundSliderDirection::Increment => (old + 1).min(max_scroll),
            };
            if new == old {
                return Vec::new();
            }
            return self.set_sound_slider_position(id, rect, new);
        }
        if self.active_sheet == OptionsSheet::Graphics {
            let Some((id, direction)) = self.pressed_graphics_arrow else {
                return Vec::new();
            };
            let Some(rect) = self
                .layout
                .as_ref()
                .map(|layout| layout.graphics.slider(id))
            else {
                return Vec::new();
            };
            let max_scroll = graphics_slider_max_scroll(rect);
            let old = self.graphics_slider_position(id, rect);
            let new = match direction {
                GraphicsSliderDirection::Decrement => old.saturating_sub(1),
                GraphicsSliderDirection::Increment => (old + 1).min(max_scroll),
            };
            if new == old {
                return Vec::new();
            }
            return self.set_graphics_slider_position(id, rect, new);
        }
        Vec::new()
    }

    fn select_sheet(&mut self, sheet: OptionsSheet) -> Vec<OptionsDlgAction> {
        if self.active_sheet == sheet {
            return Vec::new();
        }
        if matches!(
            self.focus,
            OptionsFocus::Program(_)
                | OptionsFocus::Graphics(_)
                | OptionsFocus::SoundCheckbox(_)
                | OptionsFocus::Control(_)
                | OptionsFocus::Network(_)
        ) {
            self.focus = OptionsFocus::None;
        }
        self.captured_fair_crew_slider = false;
        self.pressed_fair_crew_arrow = None;
        self.pressed_program_button = None;
        self.captured_sound_slider = None;
        self.pressed_sound_arrow = None;
        self.captured_graphics_slider = None;
        self.pressed_graphics_arrow = None;
        self.pressed_release_target = None;
        self.pointer_down = false;
        self.active_sheet = sheet;
        self.set_pointer_position(self.pointer_position);
        vec![OptionsDlgAction::SheetChanged(sheet)]
    }

    const fn tabular_focused(&self) -> bool {
        matches!(self.focus, OptionsFocus::Tabular)
    }

    const fn back_highlighted(&self) -> bool {
        matches!(self.focus, OptionsFocus::Back) || matches!(self.hovered, Some(OptionsHit::Back))
    }

    fn program_control_highlighted(&self, target: OptionsProgramFocusTarget) -> bool {
        matches!(self.focus, OptionsFocus::Program(focused) if focused == target)
            || matches!(self.hovered, Some(OptionsHit::Program(hovered)) if hovered == target)
    }

    fn graphics_control_highlighted(&self, target: OptionsGraphicsFocusTarget) -> bool {
        matches!(self.focus, OptionsFocus::Graphics(focused) if focused == target)
            || match (target, self.hovered) {
                (
                    OptionsGraphicsFocusTarget::DisplayModeCombo,
                    Some(OptionsHit::Graphics(GraphicsHitTarget::DisplayModeCombo)),
                )
                | (
                    OptionsGraphicsFocusTarget::ApplyScale,
                    Some(OptionsHit::Graphics(GraphicsHitTarget::ApplyScale)),
                )
                | (
                    OptionsGraphicsFocusTarget::ScaleEdit,
                    Some(OptionsHit::Graphics(
                        GraphicsHitTarget::ScaleEdit | GraphicsHitTarget::ScaleSpinbox(_),
                    )),
                ) => true,
                (
                    OptionsGraphicsFocusTarget::Checkbox(focused),
                    Some(OptionsHit::Graphics(GraphicsHitTarget::Checkbox(hovered))),
                ) => focused == hovered,
                _ => false,
            }
    }

    fn control_sheet_control_highlighted(&self, target: ControlSheetHit) -> bool {
        matches!(self.focus, OptionsFocus::Control(focused) if focused == target)
            || matches!(self.hovered, Some(OptionsHit::Control(hovered)) if hovered == target)
    }

    fn network_control_highlighted(&self, target: NetworkSheetHit) -> bool {
        matches!(self.focus, OptionsFocus::Network(focused) if focused == target)
            || matches!(self.hovered, Some(OptionsHit::Network(hovered)) if hovered == target)
    }

    fn program_button_pressed(&self, target: OptionsProgramFocusTarget) -> bool {
        self.pressed_program_button == Some(target)
            || (self.pointer_down
                && self.pressed_release_target == Some(OptionsHit::Program(target))
                && self.hovered == Some(OptionsHit::Program(target)))
    }

    fn fair_crew_arrow_pressed(&self, direction: ProgramSliderDirection) -> bool {
        self.pressed_fair_crew_arrow == Some(direction)
    }

    fn sound_checkbox_highlighted(&self, id: SoundCheckboxId) -> bool {
        matches!(self.focus, OptionsFocus::SoundCheckbox(focused) if focused == id)
            || matches!(self.hovered, Some(OptionsHit::SoundCheckbox(hovered)) if hovered == id)
    }

    fn sound_arrow_pressed(&self, id: SoundVolumeId, direction: SoundSliderDirection) -> bool {
        matches!(self.pressed_sound_arrow, Some((pressed_id, pressed_direction)) if pressed_id == id && pressed_direction == direction)
    }

    fn graphics_arrow_pressed(
        &self,
        id: GraphicsSliderId,
        direction: GraphicsSliderDirection,
    ) -> bool {
        matches!(self.pressed_graphics_arrow, Some((pressed_id, pressed_direction)) if pressed_id == id && pressed_direction == direction)
    }

    const fn sound_action(action: SoundSheetAction) -> OptionsDlgAction {
        OptionsDlgAction::Sound(action)
    }

    fn activate_program_combo(&self, target: OptionsProgramFocusTarget) -> Vec<OptionsDlgAction> {
        match target {
            OptionsProgramFocusTarget::LanguageCombo => {
                vec![OptionsDlgAction::OpenLanguageCombo]
            }
            OptionsProgramFocusTarget::FontFaceCombo => {
                vec![OptionsDlgAction::OpenFontFaceCombo]
            }
            OptionsProgramFocusTarget::FontSizeCombo => {
                vec![OptionsDlgAction::OpenFontSizeCombo]
            }
            _ => Vec::new(),
        }
    }

    fn toggle_program_checkbox(
        &mut self,
        target: OptionsProgramFocusTarget,
    ) -> Vec<OptionsDlgAction> {
        let action = match target {
            OptionsProgramFocusTarget::WhiteChatIngame => {
                self.program.white_chat_ingame = !self.program.white_chat_ingame;
                OptionsDlgAction::WhiteChatIngameChanged(self.program.white_chat_ingame)
            }
            OptionsProgramFocusTarget::WhiteChatLobby => {
                self.program.white_chat_lobby = !self.program.white_chat_lobby;
                OptionsDlgAction::WhiteChatLobbyChanged(self.program.white_chat_lobby)
            }
            OptionsProgramFocusTarget::ShowLogTimestamps => {
                self.program.show_log_timestamps = !self.program.show_log_timestamps;
                OptionsDlgAction::ShowLogTimestampsChanged(self.program.show_log_timestamps)
            }
            OptionsProgramFocusTarget::Preloading => {
                self.program.preloading = !self.program.preloading;
                OptionsDlgAction::PreloadingChanged(self.program.preloading)
            }
            _ => return Vec::new(),
        };
        vec![action]
    }

    fn activate_program_button(&self, target: OptionsProgramFocusTarget) -> Vec<OptionsDlgAction> {
        match target {
            OptionsProgramFocusTarget::ResetButton => {
                vec![OptionsDlgAction::ResetConfiguration]
            }
            OptionsProgramFocusTarget::AdvancedButton => {
                vec![OptionsDlgAction::OpenAdvancedSettings]
            }
            _ => Vec::new(),
        }
    }

    fn begin_fair_crew_slider_pointer(
        &mut self,
        part: ProgramSliderPart,
        position: GuiPoint,
    ) -> Vec<OptionsDlgAction> {
        match part {
            ProgramSliderPart::DecrementArrow => {
                self.pressed_fair_crew_arrow = Some(ProgramSliderDirection::Decrement);
                vec![OptionsDlgAction::ProgramGuiSound(GuiSound::ArrowHit)]
            }
            ProgramSliderPart::IncrementArrow => {
                self.pressed_fair_crew_arrow = Some(ProgramSliderDirection::Increment);
                vec![OptionsDlgAction::ProgramGuiSound(GuiSound::ArrowHit)]
            }
            ProgramSliderPart::Track => {
                let mut actions = self.update_fair_crew_slider_from_pointer(position);
                self.captured_fair_crew_slider = true;
                actions.push(OptionsDlgAction::ProgramGuiSound(GuiSound::Command));
                actions
            }
        }
    }

    fn update_held_fair_crew_arrow(
        &mut self,
        position: GuiPoint,
        previous: Option<ProgramSliderDirection>,
    ) -> Vec<OptionsDlgAction> {
        let current = match self.hovered {
            Some(OptionsHit::FairCrewSlider(ProgramSliderPart::DecrementArrow)) => {
                Some(ProgramSliderDirection::Decrement)
            }
            Some(OptionsHit::FairCrewSlider(ProgramSliderPart::IncrementArrow)) => {
                Some(ProgramSliderDirection::Increment)
            }
            Some(OptionsHit::FairCrewSlider(ProgramSliderPart::Track)) => {
                self.pressed_fair_crew_arrow = None;
                let mut actions = self.update_fair_crew_slider_from_pointer(position);
                self.captured_fair_crew_slider = true;
                actions.push(OptionsDlgAction::ProgramGuiSound(GuiSound::Command));
                if previous.is_some() {
                    actions.push(OptionsDlgAction::ProgramGuiSound(GuiSound::ArrowHit));
                }
                return actions;
            }
            _ => None,
        };
        self.pressed_fair_crew_arrow = current;
        if current.is_some() && previous.is_none() {
            vec![OptionsDlgAction::ProgramGuiSound(GuiSound::ArrowHit)]
        } else {
            Vec::new()
        }
    }

    fn update_fair_crew_slider_from_pointer(
        &mut self,
        position: GuiPoint,
    ) -> Vec<OptionsDlgAction> {
        let Some(rect) = self.layout.as_ref().map(|layout| layout.slider) else {
            return Vec::new();
        };
        let max_scroll = program_slider_max_scroll(rect);
        let local_x = position.x.floor() as i32 - rect.x;
        let scroll_pos = (local_x - 16 - 8).clamp(0, max_scroll);
        self.set_fair_crew_slider_position(rect, scroll_pos)
    }

    fn set_fair_crew_slider_position(
        &mut self,
        rect: IntRect,
        scroll_pos: i32,
    ) -> Vec<OptionsDlgAction> {
        let max_scroll = program_slider_max_scroll(rect).max(1);
        let scroll_pos = scroll_pos.clamp(0, max_scroll);
        self.fair_crew_slider_position = Some(scroll_pos);
        let slider = (scroll_pos * 100 / max_scroll).clamp(0, 100);
        let strength = fair_crew_slider_to_strength(slider);
        self.program.fair_crew_slider = slider;
        self.program.fair_crew_strength = strength;
        vec![OptionsDlgAction::FairCrewStrengthChanged(strength)]
    }

    fn sync_fair_crew_slider_position(&mut self) {
        let Some(rect) = self.layout.as_ref().map(|layout| layout.slider) else {
            return;
        };
        if self.fair_crew_slider_position.is_none() {
            self.fair_crew_slider_position =
                Some(self.program.fair_crew_slider * program_slider_max_scroll(rect) / 100);
        }
    }

    fn fair_crew_slider_position(&self, rect: IntRect) -> i32 {
        self.fair_crew_slider_position.unwrap_or_else(|| {
            self.program.fair_crew_slider * program_slider_max_scroll(rect) / 100
        })
    }

    fn toggle_sound_checkbox(&mut self, id: SoundCheckboxId) -> Vec<OptionsDlgAction> {
        let checked = !self.sound.checkbox(id);
        self.sound.set_checkbox(id, checked);
        vec![
            Self::sound_action(SoundSheetAction::GuiSound(SoundSheetSound::ArrowHit)),
            Self::sound_action(SoundSheetAction::CheckboxChanged { id, checked }),
        ]
    }

    fn activate_graphics_hit(&mut self, hit: GraphicsHitTarget) -> Vec<OptionsDlgAction> {
        let action = match hit {
            GraphicsHitTarget::DisplayModeCombo => Some(GraphicsSheetAction::OpenDisplayModeCombo),
            GraphicsHitTarget::ScaleEdit => {
                return vec![OptionsDlgAction::OpenGraphicsScaleText];
            }
            GraphicsHitTarget::ScaleSpinbox(direction) => {
                let action = self.graphics.step_scale_spinbox(match direction {
                    SpinboxDirection::Increment => 1,
                    SpinboxDirection::Decrement => -1,
                });
                if action.is_some() {
                    self.graphics_slider_positions
                        [graphics_slider_index(GraphicsSliderId::Scale)] = None;
                }
                action
            }
            GraphicsHitTarget::ApplyScale => self.graphics.request_scale_test(),
            GraphicsHitTarget::Checkbox(id) => Some(self.graphics.toggle_checkbox(id)),
            GraphicsHitTarget::Slider { .. } => return Vec::new(),
        };
        action.map(OptionsDlgAction::Graphics).into_iter().collect()
    }

    fn begin_graphics_slider_pointer(
        &mut self,
        id: GraphicsSliderId,
        part: GraphicsSliderPart,
        position: GuiPoint,
    ) -> Vec<OptionsDlgAction> {
        match part {
            GraphicsSliderPart::DecrementArrow => {
                self.pressed_graphics_arrow = Some((id, GraphicsSliderDirection::Decrement));
                Vec::new()
            }
            GraphicsSliderPart::IncrementArrow => {
                self.pressed_graphics_arrow = Some((id, GraphicsSliderDirection::Increment));
                Vec::new()
            }
            GraphicsSliderPart::Track => {
                let actions = self.update_graphics_slider_from_pointer(id, position);
                self.captured_graphics_slider = Some(id);
                actions
            }
        }
    }

    fn update_held_graphics_arrow(
        &mut self,
        position: GuiPoint,
        _previous: Option<(GraphicsSliderId, GraphicsSliderDirection)>,
    ) -> Vec<OptionsDlgAction> {
        let current = match self.hovered {
            Some(OptionsHit::Graphics(GraphicsHitTarget::Slider {
                id,
                part: GraphicsSliderPart::DecrementArrow,
            })) => Some((id, GraphicsSliderDirection::Decrement)),
            Some(OptionsHit::Graphics(GraphicsHitTarget::Slider {
                id,
                part: GraphicsSliderPart::IncrementArrow,
            })) => Some((id, GraphicsSliderDirection::Increment)),
            Some(OptionsHit::Graphics(GraphicsHitTarget::Slider {
                id,
                part: GraphicsSliderPart::Track,
            })) => {
                self.pressed_graphics_arrow = None;
                let actions = self.update_graphics_slider_from_pointer(id, position);
                self.captured_graphics_slider = Some(id);
                return actions;
            }
            _ => None,
        };
        self.pressed_graphics_arrow = current;
        Vec::new()
    }

    fn update_graphics_slider_from_pointer(
        &mut self,
        id: GraphicsSliderId,
        position: GuiPoint,
    ) -> Vec<OptionsDlgAction> {
        let Some(rect) = self
            .layout
            .as_ref()
            .map(|layout| layout.graphics.slider(id))
        else {
            return Vec::new();
        };
        let max_scroll = graphics_slider_max_scroll(rect);
        let local_x = position.x.floor() as i32 - rect.x;
        let scroll_pos = (local_x - 16 - 8).clamp(0, max_scroll);
        self.set_graphics_slider_position(id, rect, scroll_pos)
    }

    fn set_graphics_slider_position(
        &mut self,
        id: GraphicsSliderId,
        rect: IntRect,
        scroll_pos: i32,
    ) -> Vec<OptionsDlgAction> {
        let max_scroll = graphics_slider_max_scroll(rect).max(1);
        let scroll_pos = scroll_pos.clamp(0, max_scroll);
        self.graphics_slider_positions[graphics_slider_index(id)] = Some(scroll_pos);
        let (minimum, maximum) = graphics_slider_range(id);
        let value = minimum + scroll_pos * (maximum - minimum) / max_scroll;
        let action = match id {
            GraphicsSliderId::Scale => self.graphics.set_scale_slider_value(value).unwrap_or(
                GraphicsSheetAction::ScaleProposalChanged(self.graphics.proposed_scale_percent),
            ),
            GraphicsSliderId::SmokeLevel => self.graphics.set_smoke_slider_value(value).unwrap_or(
                GraphicsSheetAction::SmokeLevelChanged(self.graphics.smoke_level),
            ),
        };
        vec![OptionsDlgAction::Graphics(action)]
    }

    fn sync_graphics_slider_positions(&mut self) {
        let Some(layout) = self.layout.as_ref() else {
            return;
        };
        for id in [GraphicsSliderId::Scale, GraphicsSliderId::SmokeLevel] {
            let index = graphics_slider_index(id);
            if self.graphics_slider_positions[index].is_none() {
                let rect = layout.graphics.slider(id);
                let max_scroll = graphics_slider_max_scroll(rect);
                let (minimum, maximum) = graphics_slider_range(id);
                let value = match id {
                    GraphicsSliderId::Scale => self.graphics.scale_slider_value(),
                    GraphicsSliderId::SmokeLevel => self.graphics.smoke_level,
                };
                self.graphics_slider_positions[index] =
                    Some((value - minimum) * max_scroll / (maximum - minimum));
            }
        }
    }

    fn graphics_slider_position(&self, id: GraphicsSliderId, rect: IntRect) -> i32 {
        self.graphics_slider_positions[graphics_slider_index(id)].unwrap_or_else(|| {
            let max_scroll = graphics_slider_max_scroll(rect);
            let (minimum, maximum) = graphics_slider_range(id);
            let value = match id {
                GraphicsSliderId::Scale => self.graphics.scale_slider_value(),
                GraphicsSliderId::SmokeLevel => self.graphics.smoke_level,
            };
            (value - minimum) * max_scroll / (maximum - minimum)
        })
    }

    fn activate_control_hit(&mut self, hit: ControlSheetHit) -> Vec<OptionsDlgAction> {
        let device = match self.active_sheet {
            OptionsSheet::Keyboard => ControlDevice::Keyboard,
            OptionsSheet::Gamepad => ControlDevice::Gamepad,
            _ => return Vec::new(),
        };
        match hit {
            ControlSheetHit::Set(set) => {
                let changed = self.controls.select_set(device, set);
                (changed && device == ControlDevice::Gamepad)
                    .then_some(OptionsDlgAction::GamepadDeviceSelected(set))
                    .into_iter()
                    .collect()
            }
            ControlSheetHit::Key(control) => self
                .controls
                .capture_target(device, control)
                .map(OptionsDlgAction::BeginControlCapture)
                .into_iter()
                .collect(),
            ControlSheetHit::Reset => vec![OptionsDlgAction::ResetControlBindings(device)],
            ControlSheetHit::GamepadGui if device == ControlDevice::Gamepad => {
                let enabled = !self.controls.gamepad_gui_control();
                self.controls.set_gamepad_gui_control(enabled);
                vec![OptionsDlgAction::GamepadGuiControlChanged(enabled)]
            }
            ControlSheetHit::GamepadGui => Vec::new(),
        }
    }

    fn activate_network_hit(&mut self, hit: NetworkSheetHit) -> Vec<OptionsDlgAction> {
        match hit {
            NetworkSheetHit::Text(field) => vec![OptionsDlgAction::OpenNetworkText(field)],
            NetworkSheetHit::PortCheck(id) => {
                let enabled = !self.network.port(id).enabled;
                self.network.port_mut(id).enabled = enabled;
                vec![OptionsDlgAction::NetworkPortEnabledChanged { id, enabled }]
            }
            NetworkSheetHit::Checkbox(id) => {
                let checked = match id {
                    NetworkCheckboxId::UseAlternateServer => {
                        self.network.use_alternate_server = !self.network.use_alternate_server;
                        self.network.use_alternate_server
                    }
                    NetworkCheckboxId::AutomaticUpdate => {
                        self.network.automatic_update = !self.network.automatic_update;
                        self.network.automatic_update
                    }
                    NetworkCheckboxId::EnableUpnp => {
                        self.network.enable_upnp = !self.network.enable_upnp;
                        self.network.enable_upnp
                    }
                };
                vec![OptionsDlgAction::NetworkCheckboxChanged { id, checked }]
            }
        }
    }

    fn begin_sound_slider_pointer(
        &mut self,
        id: SoundVolumeId,
        part: SoundSliderPart,
        position: GuiPoint,
    ) -> Vec<OptionsDlgAction> {
        match part {
            SoundSliderPart::DecrementArrow => {
                self.pressed_sound_arrow = Some((id, SoundSliderDirection::Decrement));
                vec![Self::sound_action(SoundSheetAction::GuiSound(
                    SoundSheetSound::ArrowHit,
                ))]
            }
            SoundSliderPart::IncrementArrow => {
                self.pressed_sound_arrow = Some((id, SoundSliderDirection::Increment));
                vec![Self::sound_action(SoundSheetAction::GuiSound(
                    SoundSheetSound::ArrowHit,
                ))]
            }
            SoundSliderPart::Track => {
                let mut actions = self.update_sound_slider_from_pointer(id, position);
                self.captured_sound_slider = Some(id);
                actions.push(Self::sound_action(SoundSheetAction::GuiSound(
                    SoundSheetSound::Command,
                )));
                actions
            }
        }
    }

    fn update_held_sound_arrow(
        &mut self,
        position: GuiPoint,
        previous: Option<(SoundVolumeId, SoundSliderDirection)>,
    ) -> Vec<OptionsDlgAction> {
        let current = match self.hovered {
            Some(OptionsHit::SoundSlider(id, SoundSliderPart::DecrementArrow)) => {
                Some((id, SoundSliderDirection::Decrement))
            }
            Some(OptionsHit::SoundSlider(id, SoundSliderPart::IncrementArrow)) => {
                Some((id, SoundSliderDirection::Increment))
            }
            Some(OptionsHit::SoundSlider(id, SoundSliderPart::Track)) => {
                self.pressed_sound_arrow = None;
                let mut actions = self.update_sound_slider_from_pointer(id, position);
                self.captured_sound_slider = Some(id);
                actions.push(Self::sound_action(SoundSheetAction::GuiSound(
                    SoundSheetSound::Command,
                )));
                if previous.is_some_and(|(previous_id, _)| previous_id == id) {
                    actions.push(Self::sound_action(SoundSheetAction::GuiSound(
                        SoundSheetSound::ArrowHit,
                    )));
                }
                return actions;
            }
            _ => None,
        };
        self.pressed_sound_arrow = current;
        if current.is_some_and(|(current_id, _)| {
            previous.is_none_or(|(previous_id, _)| previous_id != current_id)
        }) {
            vec![Self::sound_action(SoundSheetAction::GuiSound(
                SoundSheetSound::ArrowHit,
            ))]
        } else {
            Vec::new()
        }
    }

    fn update_sound_slider_from_pointer(
        &mut self,
        id: SoundVolumeId,
        position: GuiPoint,
    ) -> Vec<OptionsDlgAction> {
        let Some(rect) = self.layout.as_ref().map(|layout| layout.sound.slider(id)) else {
            return Vec::new();
        };
        let max_scroll = sound_slider_max_scroll(rect);
        let local_x = position.x.floor() as i32 - rect.x;
        let scroll_pos = (local_x - 16 - 8).clamp(0, max_scroll);
        self.set_sound_slider_position(id, rect, scroll_pos)
    }

    fn set_sound_slider_position(
        &mut self,
        id: SoundVolumeId,
        rect: IntRect,
        scroll_pos: i32,
    ) -> Vec<OptionsDlgAction> {
        let max_scroll = sound_slider_max_scroll(rect).max(1);
        let scroll_pos = scroll_pos.clamp(0, max_scroll);
        self.sound_slider_positions[id.index()] = Some(scroll_pos);
        let value = (scroll_pos * 100 / max_scroll).clamp(0, 100) as u8;
        self.sound.set_volume(id, value);
        let mut actions = vec![Self::sound_action(SoundSheetAction::VolumeChanged {
            id,
            value,
        })];
        if id == SoundVolumeId::SoundEffects {
            actions.push(Self::sound_action(SoundSheetAction::TestSound(
                SoundSheetSound::ArrowHit,
            )));
        }
        actions
    }

    fn sync_sound_slider_positions(&mut self) {
        let Some(layout) = self.layout.as_ref() else {
            return;
        };
        for id in SoundVolumeId::ALL {
            if self.sound_slider_positions[id.index()].is_none() {
                let rect = layout.sound.slider(id);
                let max_scroll = sound_slider_max_scroll(rect);
                self.sound_slider_positions[id.index()] =
                    Some(i32::from(self.sound.volume(id)) * max_scroll / 100);
            }
        }
    }

    fn sound_slider_position(&self, id: SoundVolumeId, rect: IntRect) -> i32 {
        self.sound_slider_positions[id.index()].unwrap_or_else(|| {
            i32::from(self.sound.volume(id)) * sound_slider_max_scroll(rect) / 100
        })
    }
}

fn rect_contains(rect: &IntRect, point: GuiPoint) -> bool {
    let (x, y) = (point.x.floor() as i32, point.y.floor() as i32);
    x >= rect.x && x < rect.x + rect.w && y >= rect.y && y < rect.y + rect.h
}

fn sound_slider_max_scroll(rect: IntRect) -> i32 {
    if rect.w > 48 {
        rect.w - 48
    } else {
        100
    }
}

fn program_slider_max_scroll(rect: IntRect) -> i32 {
    if rect.w > 48 {
        rect.w - 48
    } else {
        100
    }
}

fn graphics_slider_max_scroll(rect: IntRect) -> i32 {
    if rect.w > 48 {
        rect.w - 48
    } else {
        1
    }
}

const fn graphics_slider_index(id: GraphicsSliderId) -> usize {
    match id {
        GraphicsSliderId::Scale => 0,
        GraphicsSliderId::SmokeLevel => 1,
    }
}

const fn graphics_slider_range(id: GraphicsSliderId) -> (i32, i32) {
    match id {
        GraphicsSliderId::Scale => (0, MAX_GRAPHICS_SCALE_PERCENT - MIN_GRAPHICS_SCALE_PERCENT),
        GraphicsSliderId::SmokeLevel => (MIN_SMOKE_LEVEL, MAX_SMOKE_LEVEL),
    }
}

fn sound_checkbox_label(labels: &OptionsLabels, id: SoundCheckboxId) -> &str {
    match id {
        SoundCheckboxId::FrontendMusic | SoundCheckboxId::GameMusic => &labels.music,
        SoundCheckboxId::FrontendSoundEffects | SoundCheckboxId::GameSoundEffects => {
            &labels.sound_effects
        }
    }
}

/// The volume headings append `':'` in code, like the Program sheet's labels.
fn sound_volume_heading(labels: &OptionsLabels, id: SoundVolumeId) -> String {
    match id {
        SoundVolumeId::Music => format!("{}:", labels.music),
        SoundVolumeId::SoundEffects => format!("{}:", labels.sound_effects),
    }
}

const CONTROL_KEY_TOOLTIP_KEYS: [&str; CONTROL_KEY_COUNT] = [
    "IDS_CTL_SELECTLEFT",
    "IDS_CTL_SELECTTOGGLE",
    "IDS_CTL_SELECTRIGHT",
    "IDS_CTL_THROW",
    "IDS_CTL_UPJUMP",
    "IDS_CTL_DIG",
    "IDS_CTL_LEFT",
    "IDS_CTL_DOWNSTOP",
    "IDS_CTL_RIGHT",
    "IDS_CTL_PLAYERMENU",
    "IDS_CTL_SPECIAL1",
    "IDS_CTL_SPECIAL2",
];

const fn graphics_checkbox_tooltip_key(id: GraphicsCheckboxId) -> &'static str {
    match id {
        GraphicsCheckboxId::AddNewCrewPortraits => "IDS_MSG_ADDPORTRAITS_DESC",
        GraphicsCheckboxId::SaveDefaultPortraits => "IDS_DESC_STOREPORTRAITS",
        GraphicsCheckboxId::AutoFrameSkip => "IDS_DESC_AUTOFRAMESKIP",
        GraphicsCheckboxId::ShowFolderMaps => "IDS_DESC_SHOWFOLDERMAPS",
        GraphicsCheckboxId::DisableGamma => "IDS_MSG_DISABLEGAMMA_DESC",
        GraphicsCheckboxId::FireParticles => "IDS_MSG_FIREPARTICLES_DESC",
    }
}

const fn sound_checkbox_tooltip_key(id: SoundCheckboxId) -> &'static str {
    match id {
        SoundCheckboxId::FrontendMusic => "IDS_DESC_MENUMUSIC",
        SoundCheckboxId::FrontendSoundEffects => "IDS_DESC_MENUSOUND",
        SoundCheckboxId::GameMusic => "IDS_DESC_GAMEMUSIC",
        SoundCheckboxId::GameSoundEffects => "IDS_DESC_GAMESOUND",
    }
}

const fn sound_volume_tooltip_key(id: SoundVolumeId) -> &'static str {
    match id {
        SoundVolumeId::Music => "IDS_DESC_VOLUMEMUSIC",
        SoundVolumeId::SoundEffects => "IDS_DESC_VOLUMESOUND",
    }
}

const fn network_port_tooltip_key(id: NetworkPortId) -> &'static str {
    match id {
        NetworkPortId::Tcp => "IDS_NET_PORT_TCP_DESC",
        NetworkPortId::Udp => "IDS_NET_PORT_UDP_DESC",
        NetworkPortId::Reference => "IDS_NET_PORT_REFERENCE_DESC",
        NetworkPortId::Discovery => "IDS_NET_PORT_DISCOVERY_DESC",
    }
}

fn left_label_bounds(font: &ClonkFont, position: (i32, i32), text: &str) -> IntRect {
    let (width, height) = font.measure(text, true);
    IntRect {
        x: position.0,
        y: position.1,
        w: width,
        h: height,
    }
}

fn enclosing_rect(first: IntRect, second: IntRect) -> IntRect {
    let left = first.x.min(second.x);
    let top = first.y.min(second.y);
    let right = (first.x + first.w).max(second.x + second.w);
    let bottom = (first.y + first.h).max(second.y + second.h);
    IntRect {
        x: left,
        y: top,
        w: right - left,
        h: bottom - top,
    }
}

fn labeled_edit_bounds(edit: IntRect, label_line_height: i32) -> IntRect {
    IntRect {
        y: edit.y - label_line_height,
        h: edit.h + label_line_height,
        ..edit
    }
}

fn options_tooltip_at(
    layout: &OptionsDlgLayout,
    active_sheet: OptionsSheet,
    program: &ProgramSheetState,
    controls: &ControlSheetState,
    labels: &OptionsLabels,
    book: &BookFonts,
    point: GuiPoint,
) -> Option<StartupTooltip> {
    let resource = |key| Some(StartupTooltip::resource(key));

    // Back and the fullscreen title were added before the Tabular. Any point
    // inside that later sibling is owned by its active sheet (or by an
    // untipped Tabular child) and cannot fall through to either label/button.
    if !rect_contains(&layout.tabular, point) {
        if rect_contains(&layout.back_button, point) {
            return resource("IDS_DLGTIP_BACKMAIN");
        }
        if rect_contains(&layout.title_label, point) {
            return Some(StartupTooltip::text(&labels.title));
        }
        return None;
    }

    match active_sheet {
        OptionsSheet::Program => {
            let language_label = left_label_bounds(
                &book.book,
                layout.language_label,
                &format!("{}:", labels.language),
            );
            let language_info =
                left_label_bounds(&book.book, layout.language_info, &program.language_info);
            if rect_contains(&language_label, point)
                || rect_contains(&layout.language_combo, point)
                || rect_contains(&language_info, point)
            {
                return resource("IDS_MSG_SELECTLANG");
            }

            let font_label =
                left_label_bounds(&book.book, layout.font_label, &format!("{}:", labels.font));
            if rect_contains(&font_label, point) || rect_contains(&layout.font_face_combo, point) {
                return resource("IDS_DESC_SELECTFONT");
            }
            if rect_contains(&layout.font_size_combo, point) {
                return resource("IDS_DESC_FONTSIZE");
            }

            let white_chat_label = left_label_bounds(
                &book.book,
                layout.white_chat_label,
                &format!("{}:", labels.white_chat),
            );
            if rect_contains(&white_chat_label, point) {
                return resource("IDS_DESC_WHITECHAT");
            }
            for (bounds, key) in [
                (layout.ingame_check, "IDS_DESC_WHITECHAT_INGAME"),
                (layout.lobby_check, "IDS_DESC_WHITECHAT_LOBBY"),
                (layout.timestamps_check, "IDS_DESC_TIMESTAMPS"),
                (layout.preloading_check, "IDS_MSG_PRELOADING_DESC"),
            ] {
                if rect_contains(&bounds, point) {
                    return resource(key);
                }
            }

            // The tooltip is on the GroupBox, not its ScrollBar, so every
            // child and every otherwise-empty part of the frame inherits it.
            if rect_contains(&layout.group, point) {
                return resource("IDS_DESC_FAIRCREWSTRENGTH");
            }
            if rect_contains(&layout.reset_button, point) {
                return resource("IDS_DESC_RESETCONFIG");
            }
            if rect_contains(&layout.advanced_button, point) {
                return resource("IDS_DESC_ADVANCED_SETTINGS");
            }
        }
        OptionsSheet::Graphics => {
            // ScaleEdit is a SpinBox window. Its edit and both arrow children
            // inherit the parent's tip; the adjacent ScrollBar has the same
            // explicitly assigned resource.
            if rect_contains(&layout.graphics.scale_edit, point)
                || rect_contains(&layout.graphics.scale_slider, point)
            {
                return resource("IDS_DESC_GRAPHICSSCALE");
            }
            for id in GraphicsCheckboxId::ALL {
                if rect_contains(&layout.graphics.checkbox(id), point) {
                    return resource(graphics_checkbox_tooltip_key(id));
                }
            }
            if rect_contains(&layout.graphics.smoke_slider, point) {
                return resource("IDS_MSG_PARTICLES_DESC");
            }
        }
        OptionsSheet::Sound => {
            for id in SoundCheckboxId::ALL {
                if rect_contains(&layout.sound.checkbox(id), point) {
                    return resource(sound_checkbox_tooltip_key(id));
                }
            }
            for id in SoundVolumeId::ALL {
                if rect_contains(&layout.sound.slider(id), point) {
                    return resource(sound_volume_tooltip_key(id));
                }
            }
        }
        OptionsSheet::Keyboard | OptionsSheet::Gamepad => {
            let device = if active_sheet == OptionsSheet::Keyboard {
                ControlDevice::Keyboard
            } else {
                ControlDevice::Gamepad
            };
            for bounds in layout
                .controls
                .set_buttons
                .iter()
                .take(controls.visible_sets(device))
            {
                if rect_contains(bounds, point) {
                    return resource("IDS_MSG_SELECTKEYSET");
                }
            }
            for (index, bounds) in layout.controls.key_buttons.iter().enumerate() {
                if rect_contains(bounds, point) {
                    return resource(CONTROL_KEY_TOOLTIP_KEYS[index]);
                }
            }
            if active_sheet == OptionsSheet::Gamepad
                && controls.gamepad_gui_checkbox_visible()
                && rect_contains(&layout.controls.gamepad_gui_check, point)
            {
                return resource("IDS_DESC_GAMEPADFORMENU");
            }
            if rect_contains(&layout.controls.reset_button, point) {
                return resource("IDS_MSG_RESETKEYSETS");
            }
        }
        OptionsSheet::Network => {
            // EditConfig derives from LabeledEdit. SetToolTip is called on
            // that parent, so both its label and its Edit child are targets.
            // These are added last on the native sheet and therefore win any
            // edge overlap with an earlier checkbox.
            let local_name =
                labeled_edit_bounds(layout.network.local_name_edit, book.book.line_height);
            if rect_contains(&local_name, point) {
                return resource("IDS_NET_COMPUTERNAME_DESC");
            }
            let nick = labeled_edit_bounds(layout.network.nick_edit, book.book.line_height);
            if rect_contains(&nick, point) {
                return resource("IDS_NET_USERNAME_DESC");
            }

            if rect_contains(&layout.network.upnp_check, point) {
                return resource("IDS_DESC_UPNP");
            }
            if rect_contains(&layout.network.automatic_update_check, point) {
                return resource("IDS_DESC_AUTOMATICUPDATES");
            }

            // NetworkServerAddressConfig likewise owns one parent tooltip;
            // the checkbox remains visible when its Edit child is hidden.
            let alternate = enclosing_rect(
                layout.network.alternate_check,
                layout.network.alternate_edit,
            );
            if rect_contains(&alternate, point) {
                return resource("IDS_NET_MASTERSRV_DESC");
            }

            // NetworkPortConfig assigns the tip to the containing Window.
            // Its title, checkbox, edit and SpinBox arrows all inherit it,
            // including when the edit itself is hidden.
            for id in NetworkPortId::ALL {
                if rect_contains(&layout.network.port_controls[id.index()], point) {
                    return resource(network_port_tooltip_key(id));
                }
            }
        }
    }

    None
}

/// `Tabular::MouseInput` for a graphical left tab strip
/// (C4GuiTabular.cpp:464-534). `Inside` is inclusive for the internal
/// caption bands even though the surrounding `C4Rect` is half-open.
fn options_hit_test(
    layout: &OptionsDlgLayout,
    active_sheet: OptionsSheet,
    controls: &ControlSheetState,
    network: &NetworkSheetState,
    point: GuiPoint,
) -> Option<OptionsHit> {
    if rect_contains(&layout.back_button, point) {
        return Some(OptionsHit::Back);
    }
    if active_sheet == OptionsSheet::Program {
        for (bounds, target) in [
            (
                layout.language_combo,
                OptionsProgramFocusTarget::LanguageCombo,
            ),
            (
                layout.font_face_combo,
                OptionsProgramFocusTarget::FontFaceCombo,
            ),
            (
                layout.font_size_combo,
                OptionsProgramFocusTarget::FontSizeCombo,
            ),
        ] {
            if rect_contains(&bounds, point) {
                return Some(OptionsHit::Program(target));
            }
        }

        // CheckBox::MouseInput accepts only the square, with an inclusive
        // horizontal Hgt edge; parent dispatch keeps the vertical edge
        // half-open.
        for (bounds, target) in [
            (
                layout.ingame_check,
                OptionsProgramFocusTarget::WhiteChatIngame,
            ),
            (
                layout.lobby_check,
                OptionsProgramFocusTarget::WhiteChatLobby,
            ),
            (
                layout.timestamps_check,
                OptionsProgramFocusTarget::ShowLogTimestamps,
            ),
            (
                layout.preloading_check,
                OptionsProgramFocusTarget::Preloading,
            ),
        ] {
            let square = IntRect {
                w: bounds.h + 1,
                ..bounds
            };
            if rect_contains(&square, point) {
                return Some(OptionsHit::Program(target));
            }
        }

        if rect_contains(&layout.slider, point) {
            let local_x = point.x.floor() as i32 - layout.slider.x;
            let part = if local_x < 16 {
                ProgramSliderPart::DecrementArrow
            } else if local_x >= layout.slider.w - 16 {
                ProgramSliderPart::IncrementArrow
            } else if layout.slider.w > 48 {
                ProgramSliderPart::Track
            } else {
                return None;
            };
            return Some(OptionsHit::FairCrewSlider(part));
        }
        for (bounds, target) in [
            (layout.reset_button, OptionsProgramFocusTarget::ResetButton),
            (
                layout.advanced_button,
                OptionsProgramFocusTarget::AdvancedButton,
            ),
        ] {
            if rect_contains(&bounds, point) {
                return Some(OptionsHit::Program(target));
            }
        }
    }
    if active_sheet == OptionsSheet::Sound {
        for id in SoundCheckboxId::ALL {
            let bounds = layout.sound.checkbox(id);
            // CheckBox::MouseInput uses Inside(x, 0, Hgt), inclusive. Parent
            // dispatch still clips y to the control's half-open C4Rect.
            let square = IntRect {
                w: bounds.h + 1,
                ..bounds
            };
            if rect_contains(&square, point) {
                return Some(OptionsHit::SoundCheckbox(id));
            }
        }
        for id in SoundVolumeId::ALL {
            let slider = layout.sound.slider(id);
            if !rect_contains(&slider, point) {
                continue;
            }
            let local_x = point.x.floor() as i32 - slider.x;
            let part = if local_x < 16 {
                SoundSliderPart::DecrementArrow
            } else if local_x >= slider.w - 16 {
                SoundSliderPart::IncrementArrow
            } else if slider.w > 48 {
                SoundSliderPart::Track
            } else {
                continue;
            };
            return Some(OptionsHit::SoundSlider(id, part));
        }
    }
    if active_sheet == OptionsSheet::Graphics {
        if let Some(hit) = graphics_hit_test(&layout.graphics, point) {
            return Some(OptionsHit::Graphics(hit));
        }
    }
    let control_device = match active_sheet {
        OptionsSheet::Keyboard => Some(ControlDevice::Keyboard),
        OptionsSheet::Gamepad => Some(ControlDevice::Gamepad),
        _ => None,
    };
    if let Some(device) = control_device {
        if let Some(hit) = control_sheet_hit_test(&layout.controls, controls, device, point) {
            return Some(OptionsHit::Control(hit));
        }
    }
    if active_sheet == OptionsSheet::Network {
        if let Some(hit) = network_sheet_hit_test(&layout.network, network, point) {
            return Some(OptionsHit::Network(hit));
        }
    }
    if !rect_contains(&layout.tabular, point) {
        return None;
    }
    let x = point.x.floor() as i32 - layout.tabular.x;
    let y = point.y.floor() as i32 - layout.tabular.y;
    if !(0..=95).contains(&x) {
        return None;
    }
    OptionsSheet::ALL
        .iter()
        .copied()
        .find(|sheet| {
            let top = 20 + 72 * sheet.index() as i32;
            (top..=top + 70).contains(&y)
        })
        .map(OptionsHit::Tab)
}

// ---------------------------------------------------------------------------
// Engine draw primitives (boxes, lines, quads, facet blits)
// ---------------------------------------------------------------------------

/// Source-color gamma encode of the blit shader (StdGL.cpp:1082-1086).
fn encode(gamma: Option<&GammaRamp>, c: f32) -> f32 {
    gamma
        .map(|g| f32::from(g.encode_float(c)))
        .unwrap_or_else(|| c.round().clamp(0.0, 255.0))
}

/// Blends one engine-color fragment over the surface with `opacity` =
/// `(255-A)/255` of the inverted-alpha color (DrawQuadDw blend
/// `glBlendFunc(GL_ONE_MINUS_SRC_ALPHA, GL_SRC_ALPHA)`, StdGL.cpp:877).
fn blend_engine_fragment(
    surface: &mut Surface,
    x: i32,
    y: i32,
    rgb: [u8; 3],
    opacity: f32,
    gamma: Option<&GammaRamp>,
) {
    if x < 0 || y < 0 {
        return;
    }
    if surface.is_gpu_scene_capture_active() {
        // Fallback rasterization during capture must stay a painter-ordered
        // retained fragment instead of blending against stale CPU backing.
        let _ = surface.blend_fragment_over(
            x as u32,
            y as u32,
            [
                f32::from(rgb[0]),
                f32::from(rgb[1]),
                f32::from(rgb[2]),
                opacity * 255.0,
            ],
            gamma,
        );
        return;
    }
    let Some(dst) = surface.get_pixel(x as u32, y as u32) else {
        return;
    };
    let mix = |c: u8, d: u8| {
        (encode(gamma, f32::from(c)) * opacity + f32::from(d) * (1.0 - opacity)).round() as u8
    };
    let out = Color::new(
        mix(rgb[0], dst.r),
        mix(rgb[1], dst.g),
        mix(rgb[2], dst.b),
        255,
    );
    let _ = surface.set_pixel(x as u32, y as u32, out);
}

/// Byte-exact CPU sample blend of the options rasterizers: gamma-encoded
/// source over destination RGB with the framebuffer forced opaque. During
/// capture the fragment is retained painter-ordered instead.
fn blend_sampled_fragment(
    surface: &mut Surface,
    x: u32,
    y: u32,
    s: [f32; 4],
    gamma: Option<&GammaRamp>,
) {
    if surface.is_gpu_scene_capture_active() {
        let _ = surface.blend_fragment_over(x, y, s, gamma);
        return;
    }
    let af = (s[3] / 255.0).clamp(0.0, 1.0);
    let Some(dst) = surface.get_pixel(x, y) else {
        return;
    };
    let blend = |src: f32, d: u8| {
        (encode(gamma, src) * af + f32::from(d) * (1.0 - af))
            .round()
            .clamp(0.0, 255.0) as u8
    };
    let out = Color::new(
        blend(s[0], dst.r),
        blend(s[1], dst.g),
        blend(s[2], dst.b),
        255,
    );
    let _ = surface.set_pixel(x, y, out);
}

fn engine_rgb(clr: u32) -> [u8; 3] {
    [(clr >> 16) as u8, (clr >> 8) as u8, clr as u8]
}

fn engine_opacity(clr: u32) -> f32 {
    (255 - (clr >> 24).min(255)) as f32 / 255.0
}

/// `DrawBoxDw` (StdDDraw2.cpp:1401-1404): fills (x0,y0)-(x1,y1) INCLUSIVE.
pub(crate) fn fill_box_dw(
    surface: &mut Surface,
    x0: i32,
    y0: i32,
    x1: i32,
    y1: i32,
    clr: u32,
    gamma: Option<&GammaRamp>,
) {
    if crate::active_advanced_renderer_config()
        .is_some_and(|config| config.blit_offset != 0 || config.no_box_fades)
    {
        if x1 < x0 || y1 < y0 {
            return;
        }
        crate::draw_color_rect(
            surface,
            SurfaceRect::new(
                x0,
                y0,
                x1.saturating_sub(x0).saturating_add(1) as u32,
                y1.saturating_sub(y0).saturating_add(1) as u32,
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
    fill_box_dw_unconfigured(surface, x0, y0, x1, y1, clr, gamma);
}

/// Compatibility box rasterizer used when DrawBoxDw has no configured renderer.
fn fill_box_dw_unconfigured(
    surface: &mut Surface,
    x0: i32,
    y0: i32,
    x1: i32,
    y1: i32,
    clr: u32,
    gamma: Option<&GammaRamp>,
) {
    if surface.is_gpu_scene_capture_active() {
        if x1 < x0 || y1 < y0 {
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
                x0 as f32,
                y0 as f32,
                x1.saturating_add(1) as f32,
                y1.saturating_add(1) as f32,
            ),
            [color; 4],
            clonk_graphics::GpuBlend::Normal,
            clonk_graphics::GpuSolidStyle::with_gamma(
                gamma.is_some_and(|gamma| !gamma.is_passthrough()),
            ),
        );
        return;
    }
    let opacity = engine_opacity(clr);
    if opacity <= 0.0 {
        return;
    }
    let rgb = engine_rgb(clr);
    for y in y0..=y1 {
        for x in x0..=x1 {
            blend_engine_fragment(surface, x, y, rgb, opacity, gamma);
        }
    }
}

/// `DrawLineDw` (StdGL.cpp:893-934) for the axis-aligned 1px lines the GUI
/// draws: a GL_LINES segment between the pixel centers `+0.5`. By the GL
/// diamond-exit rule the segment never leaves the END pixel's diamond, so
/// the end pixel is NOT rasterized (proven against F9 captures by the
/// net/scensel dialogs and re-verified here on the group-frame corners).
fn draw_line_dw(
    surface: &mut Surface,
    x0: i32,
    y0: i32,
    x1: i32,
    y1: i32,
    clr: u32,
    gamma: Option<&GammaRamp>,
) {
    if surface.is_gpu_scene_capture_active() {
        // Preserve the directed GL_LINES primitive until presentation scale is
        // known. A logical box surrogate diverges after fractional viewport
        // projection and cannot reproduce the diamond-exit final endpoint.
        crate::classic_gui::draw_engine_line(surface, x0, y0, x1, y1, clr, gamma);
        return;
    }

    match (x1 - x0, y1 - y0) {
        (0, dy) if dy > 0 => fill_box_dw_unconfigured(surface, x0, y0, x1, y1 - 1, clr, gamma),
        (0, dy) if dy < 0 => fill_box_dw_unconfigured(surface, x0, y1 + 1, x1, y0, clr, gamma),
        (dx, 0) if dx > 0 => fill_box_dw_unconfigured(surface, x0, y0, x1 - 1, y1, clr, gamma),
        (dx, 0) if dx < 0 => fill_box_dw_unconfigured(surface, x1 + 1, y0, x0, y1, clr, gamma),
        // Degenerate/diagonal lines are not drawn by this dialog.
        _ => {}
    }
}

/// `DrawFrameDw` (StdDDraw2.cpp:1181-1187): outline of the inclusive rect.
fn draw_frame_dw(
    surface: &mut Surface,
    x0: i32,
    y0: i32,
    x1: i32,
    y1: i32,
    clr: u32,
    gamma: Option<&GammaRamp>,
) {
    draw_line_dw(surface, x0, y0, x1, y0, clr, gamma);
    draw_line_dw(surface, x1, y0, x1, y1, clr, gamma);
    draw_line_dw(surface, x1, y1, x0, y1, clr, gamma);
    draw_line_dw(surface, x0, y1, x0, y0, clr, gamma);
}

/// `DrawQuadDw` (StdGL.cpp:846-891): convex quad fill with pixel centers
/// inside the polygon (GL triangle-strip rasterization, blitOffset = 0).
fn fill_quad_dw(surface: &mut Surface, vtx: &[(i32, i32); 4], clr: u32, gamma: Option<&GammaRamp>) {
    if surface.is_gpu_scene_capture_active() {
        let renderer_config = crate::active_advanced_renderer_config()
            .unwrap_or(crate::AdvancedRendererConfig::DEFAULT);
        let mut color = Color::new(
            (clr >> 16) as u8,
            (clr >> 8) as u8,
            clr as u8,
            255 - (clr >> 24) as u8,
        );
        if renderer_config.no_box_fades {
            color = crate::normalize_quad_colors([color; 4]);
        }
        if color.a == 0 {
            return;
        }
        let offset = renderer_config.blit_offset as f32 / 100.0;
        let corner = |(x, y): (i32, i32)| clonk_graphics::GpuSolidVertex {
            position: [x as f32 + offset, y as f32 + offset, 1.0],
            color: crate::gpu_rgba(color),
            outer_modulation: clonk_graphics::GpuSolidOuterModulation::PackedC4,
        };
        let corners = vtx.map(corner);
        let _ = surface.push_gpu_command(clonk_graphics::GpuCommand::Solid {
            vertices: vec![
                corners[0], corners[1], corners[2], corners[2], corners[1], corners[3],
            ],
            topology: clonk_graphics::GpuPrimitiveTopology::TriangleList,
            alpha_mode: clonk_graphics::GpuSolidAlphaMode::SourceOver,
            clip: surface.clip(),
            blend: clonk_graphics::GpuBlend::Normal,
            style: clonk_graphics::GpuSolidStyle::with_gamma(
                gamma.is_some_and(|gamma| !gamma.is_passthrough()),
            ),
        });
        return;
    }
    if let Some(renderer_config) = crate::active_advanced_renderer_config()
        .filter(|config| config.blit_offset != 0 || config.no_box_fades)
    {
        let mut color = Color::new(
            (clr >> 16) as u8,
            (clr >> 8) as u8,
            clr as u8,
            255 - (clr >> 24) as u8,
        );
        if renderer_config.no_box_fades {
            color = crate::normalize_quad_colors([color; 4]);
        }
        if color.a == 0 {
            return;
        }
        let offset = renderer_config.blit_offset as f32 / 100.0;
        let vertices = vtx.map(|(x, y)| (x as f32 + offset, y as f32 + offset));
        let y_min = vertices.iter().map(|v| v.1).reduce(f32::min).unwrap_or(0.0);
        let y_max = vertices.iter().map(|v| v.1).reduce(f32::max).unwrap_or(0.0);
        let first_y = (y_min - 0.5).ceil() as i32;
        let last_y = (y_max - 0.5).ceil() as i32;
        let opacity = f32::from(color.a) / 255.0;
        for y in first_y..last_y {
            let yc = y as f32 + 0.5;
            let crossings: Vec<f32> = (0..4)
                .filter_map(|i| {
                    let (ax, ay) = vertices[i];
                    let (bx, by) = vertices[(i + 1) % 4];
                    ((ay <= yc) != (by <= yc)).then(|| ax + (yc - ay) / (by - ay) * (bx - ax))
                })
                .collect();
            let (Some(enter), Some(exit)) = (
                crossings.iter().copied().reduce(f32::min),
                crossings.iter().copied().reduce(f32::max),
            ) else {
                continue;
            };
            let mut x = (enter - 0.5).ceil() as i32;
            while (x as f32 + 0.5) < exit {
                blend_engine_fragment(surface, x, y, [color.r, color.g, color.b], opacity, gamma);
                x += 1;
            }
        }
        return;
    }
    let opacity = engine_opacity(clr);
    if opacity <= 0.0 {
        return;
    }
    let rgb = engine_rgb(clr);
    let y_min = vtx.iter().map(|v| v.1).min().unwrap_or(0);
    let y_max = vtx.iter().map(|v| v.1).max().unwrap_or(0);
    for y in y_min..y_max {
        let yc = y as f32 + 0.5;
        let crossings: Vec<f32> = (0..4)
            .filter_map(|i| {
                let (ax, ay) = (vtx[i].0 as f32, vtx[i].1 as f32);
                let (bx, by) = (vtx[(i + 1) % 4].0 as f32, vtx[(i + 1) % 4].1 as f32);
                ((ay <= yc) != (by <= yc)).then(|| ax + (yc - ay) / (by - ay) * (bx - ax))
            })
            .collect();
        let (Some(enter), Some(exit)) = (
            crossings.iter().copied().reduce(f32::min),
            crossings.iter().copied().reduce(f32::max),
        ) else {
            continue;
        };
        let mut x = (enter - 0.5).ceil() as i32;
        while (x as f32 + 0.5) < exit {
            blend_engine_fragment(surface, x, y, rgb, opacity, gamma);
            x += 1;
        }
    }
}

/// Reads an RGBA texel of `image`; texels inside a GL tile but outside the
/// image read transparent WHITE — the C++ texture buffers are
/// `memset(..., 0xff, ...)` (C4Surface.cpp:1113), i.e. white with inverted
/// alpha 0xff = transparent. Texels inside the image that are FULLY
/// transparent read transparent BLACK: `C4Surface::ReadPNG` rewrites them to
/// `0xff000000` (C4Surface.cpp:972). Both only matter to GL_LINEAR edge
/// mixing — their alpha contribution is zero.
fn texel_or_white(image: &ImageData, x: i32, y: i32) -> [f32; 4] {
    if x < 0 || y < 0 || x >= image.width() as i32 || y >= image.height() as i32 {
        return [255.0, 255.0, 255.0, 0.0];
    }
    let idx = ((y as u32 * image.width() + x as u32) * 4) as usize;
    image
        .pixels()
        .get(idx..idx + 4)
        .map(|p| match p[3] {
            0 => [0.0, 0.0, 0.0, 0.0],
            a => [
                f32::from(p[0]),
                f32::from(p[1]),
                f32::from(p[2]),
                f32::from(a),
            ],
        })
        .unwrap_or([255.0, 255.0, 255.0, 0.0])
}

/// Mirrors `C4Surface::ReadPNG`'s post-load fixup (C4Surface.cpp:972): every
/// fully transparent pixel becomes transparent BLACK in the GL texture, so
/// GL_LINEAR sampling near alpha edges mixes toward black, not toward the
/// PNG's hidden RGB (white in most GUI assets). Needed for every *stretched*
/// blit of an asset with fully transparent texels.
fn blacken_transparent(image: &ImageData) -> ImageData {
    let pixels = image
        .pixels()
        .chunks_exact(4)
        .flat_map(|p| {
            if p[3] == 0 {
                [0, 0, 0, 0]
            } else {
                [p[0], p[1], p[2], p[3]]
            }
        })
        .collect();
    ImageData::new(image.width(), image.height(), pixels)
}

thread_local! {
    /// Stable retained copies of PNG-backed GUI images after
    /// C4Surface::ReadPNG's alpha-zero RGB fixup. Keeping the generated
    /// ImageData identity avoids allocating and re-uploading the options
    /// paper on every frame.
    static RETAINED_BLACKENED_IMAGES: RefCell<HashMap<clonk_graphics::GpuTextureId, ImageData>> =
        RefCell::new(HashMap::new());
}

pub(crate) fn retained_blackened_image(image: &ImageData) -> ImageData {
    RETAINED_BLACKENED_IMAGES.with(|images| {
        if let Some(cached) = images.borrow().get(&image.gpu_texture_id()).cloned() {
            return cached;
        }
        let needs_fixup = image
            .pixels()
            .chunks_exact(4)
            .any(|pixel| pixel[3] == 0 && pixel[..3] != [0, 0, 0]);
        let blackened = if needs_fixup {
            blacken_transparent(image)
        } else {
            image.clone()
        };
        images
            .borrow_mut()
            .insert(image.gpu_texture_id(), blackened.clone());
        blackened
    })
}

/// `CStdDDraw::Blit` whole-image stretch (StdDDraw2.cpp:637-786) like
/// `crate::draw_image_bilinear`, but with the C++ texture padding color
/// (transparent white) so edge samples bleed toward white exactly like the
/// engine. Needed for the paper: its 628x483 image inside 512px tiles is
/// sampled fractionally at the right/bottom edges.
fn draw_image_bilinear_white_pad(
    surface: &mut Surface,
    rect: &GuiRect,
    image: &ImageData,
    gamma: Option<&GammaRamp>,
) {
    if rect.size.width <= 0.0
        || rect.size.height <= 0.0
        || image.width() == 0
        || image.height() == 0
    {
        return;
    }
    if surface.is_gpu_scene_capture_active() {
        let blackened = retained_blackened_image(image);
        if crate::capture_gpu_gui_image(
            surface,
            (
                rect.origin.x,
                rect.origin.y,
                rect.size.width,
                rect.size.height,
            ),
            &blackened,
            crate::FloatSourceRect {
                x: 0.0,
                y: 0.0,
                width: image.width() as f32,
                height: image.height() as f32,
            },
            clonk_graphics::GpuSampler::Linear,
            crate::BilinearBlend::AlphaOver,
            None,
            gamma,
        ) {
            return;
        }
    }
    if crate::draw_image_source_with_active_renderer_config(
        surface,
        rect,
        image,
        (0.0, 0.0, image.width() as f32, image.height() as f32),
        BlitSampling::Linear,
        gamma,
    ) {
        return;
    }
    let (fw, fh) = (image.width() as f32, image.height() as f32);
    let (tx, ty) = (rect.origin.x, rect.origin.y);
    let (scale_x, scale_y) = (rect.size.width / fw, rect.size.height / fh);
    // Tile size: next pow2 of min(W,H), capped 4096 (C4Surface.cpp:166-189).
    let ts = {
        let need = image.width().min(image.height()).max(1);
        let mut n = 1u32;
        while (1 << n) < need {
            n += 1;
        }
        ((1u32 << n).min(4096)) as i32
    };
    let tiles_x = (image.width() as i32 - 1) / ts + 1;
    let tiles_y = (image.height() as i32 - 1) / ts + 1;

    for tile_iy in 0..tiles_y {
        for tile_ix in 0..tiles_x {
            let (blit_x, blit_y) = (tile_ix * ts, tile_iy * ts);
            let s_right = ((blit_x + ts) as f32).min(fw);
            let s_bottom = ((blit_y + ts) as f32).min(fh);
            let t_left = blit_x as f32 * scale_x + tx;
            let t_top = blit_y as f32 * scale_y + ty;
            let t_right = s_right * scale_x + tx;
            let t_bottom = s_bottom * scale_y + ty;
            let py0 = ((t_top - 0.5).ceil() as i32).max(0);
            let px0 = ((t_left - 0.5).ceil() as i32).max(0);
            for py in py0..surface.height() as i32 {
                if (py as f32 + 0.5) >= t_bottom {
                    break;
                }
                for px in px0..surface.width() as i32 {
                    if (px as f32 + 0.5) >= t_right {
                        break;
                    }
                    // GL_LINEAR sample, GL_CLAMP_TO_EDGE within the tile.
                    let u = (px as f32 + 0.5 - tx) / scale_x - 0.5 - blit_x as f32;
                    let v = (py as f32 + 0.5 - ty) / scale_y - 0.5 - blit_y as f32;
                    let (x0, y0) = (u.floor() as i32, v.floor() as i32);
                    let (fx, fy) = (u - x0 as f32, v - y0 as f32);
                    let tap = |xr: i32, yr: i32| {
                        texel_or_white(
                            image,
                            blit_x + xr.clamp(0, ts - 1),
                            blit_y + yr.clamp(0, ts - 1),
                        )
                    };
                    let (p00, p10) = (tap(x0, y0), tap(x0 + 1, y0));
                    let (p01, p11) = (tap(x0, y0 + 1), tap(x0 + 1, y0 + 1));
                    let s: [f32; 4] = std::array::from_fn(|c| {
                        let top = p00[c] * (1.0 - fx) + p10[c] * fx;
                        let bottom = p01[c] * (1.0 - fx) + p11[c] * fx;
                        top * (1.0 - fy) + bottom * fy
                    });
                    if s[3] <= 0.0 {
                        continue;
                    }
                    blend_sampled_fragment(surface, px as u32, py as u32, s, gamma);
                }
            }
        }
    }
}

/// `ScrollBar::DrawElement` for a horizontal bar decorated by a book scroll
/// sheet (C4GuiContainers.cpp:446-479): the begin/middle/end cells of
/// `barScroll` through `DrawHBarByVGfx`, then `fctScrollPin` at
/// `x + C4GUI_ScrollArrowWdt + iScrollPos`. `pin_src` is the sheet cell chosen
/// by `ScrollBarFacets::Set` (C4Gui.cpp:203-215): `(16, 16)` for the default
/// pin, `(32, 16 * (index - 1))` for the R/G/B pins of `sfctBookScrollR/G/B`.
pub(crate) fn draw_horizontal_book_scrollbar(
    surface: &mut Surface,
    book_scroll: &ImageData,
    rect: &IntRect,
    scroll_pos: i32,
    decrement_pressed: bool,
    increment_pressed: bool,
    pin_src: (u32, u32),
    gamma: Option<&GammaRamp>,
) {
    draw_rotated_vfacet(
        surface,
        book_scroll,
        if decrement_pressed { 16 } else { 0 },
        0,
        16,
        rect.x,
        rect.y,
        gamma,
    );
    let mut iy = 16;
    while iy < rect.w - 5 {
        let h2 = 16.min(rect.w - 5 - iy);
        draw_rotated_vfacet(surface, book_scroll, 0, 16, h2, rect.x + iy, rect.y, gamma);
        iy += 16;
    }
    draw_rotated_vfacet(
        surface,
        book_scroll,
        if increment_pressed { 16 } else { 0 },
        32,
        16,
        rect.x + rect.w - 16,
        rect.y,
        gamma,
    );
    draw_image_strip(
        surface,
        rect.x + 16 + scroll_pos,
        rect.y,
        book_scroll,
        pin_src.0,
        pin_src.1,
        16,
        16,
        gamma,
    );
}

/// One vertical-gfx facet of `DrawHBarByVGfx` (C4Gui.cpp:347-361): the 16px
/// wide facet at `(src_x, src_y, 16, src_h)` rotated -90 degrees about the
/// bar's left end. Dest pixel `(dest_x + dx, dest_y + dy)` samples texel
/// `(src_x + 15 - dy, src_y + dx)` — the integer-aligned rotation lands
/// exactly on texel centers.
fn draw_rotated_vfacet(
    surface: &mut Surface,
    image: &ImageData,
    src_x: i32,
    src_y: i32,
    src_h: i32,
    dest_x: i32,
    dest_y: i32,
    gamma: Option<&GammaRamp>,
) {
    let renderer_config =
        crate::active_advanced_renderer_config().unwrap_or(crate::AdvancedRendererConfig::DEFAULT);
    if surface.is_gpu_scene_capture_active()
        || renderer_config.tex_indent != 0
        || renderer_config.blit_offset != 0
    {
        let transform = clonk_graphics::Transform::set_rotate(
            -90 * 100,
            dest_x as f32 + 8.0,
            dest_y as f32 + 8.0,
        );
        crate::draw_image_region_transformed_float_source(
            surface,
            (dest_x as f32, dest_y as f32, 16.0, src_h as f32),
            &transform,
            image,
            None,
            &crate::FloatSourceRect {
                x: src_x as f32,
                y: src_y as f32,
                width: 16.0,
                height: src_h as f32,
            },
            BlitSampling::Linear,
            false,
            None,
            crate::SpriteBlitState {
                mode: 0,
                modulation: None,
                fog_modulation: None,
                renderer_config,
            },
            gamma,
            None,
        );
        return;
    }
    for dx in 0..src_h {
        for dy in 0..16 {
            let s = texel_or_white(image, src_x + 15 - dy, src_y + dx);
            if s[3] <= 0.0 {
                continue;
            }
            let (x, y) = (dest_x + dx, dest_y + dy);
            if x < 0 || y < 0 {
                continue;
            }
            blend_sampled_fragment(surface, x as u32, y as u32, s, gamma);
        }
    }
}

/// Crops a sub-rect of `image` into its own `ImageData` (used to stretch one
/// facet phase: each 32x32 phase is its own GL texture tile in C++, so a
/// crop + whole-image stretch reproduces the engine's sampling exactly).
fn crop_image(image: &ImageData, x: u32, y: u32, w: u32, h: u32) -> ImageData {
    type CroppedImageCache = HashMap<(clonk_graphics::GpuTextureId, u32, u32, u32, u32), ImageData>;
    thread_local! {
        static CROPPED_IMAGES: RefCell<CroppedImageCache> = RefCell::new(HashMap::new());
    }
    CROPPED_IMAGES.with(|images| {
        let key = (image.gpu_texture_id(), x, y, w, h);
        if let Some(image) = images.borrow().get(&key).cloned() {
            return image;
        }
        let pixels = (0..h)
            .flat_map(|row| {
                let start = (((y + row) * image.width() + x) * 4) as usize;
                image.pixels()[start..start + (w * 4) as usize]
                    .iter()
                    .copied()
            })
            .collect();
        let cropped = ImageData::new(w, h, pixels);
        images.borrow_mut().insert(key, cropped.clone());
        cropped
    })
}

// ---------------------------------------------------------------------------
// Renderer
// ---------------------------------------------------------------------------

/// Renders the live options dialog chrome and the implemented active sheet.
pub struct OptionsDlgScreen;

impl OptionsDlgScreen {
    /// Source-compatible first-shown renderer. New callers with live input
    /// state should use [`Self::render_state`].
    pub fn render(
        surface: &mut Surface,
        assets: &OptionsDlgAssets,
        gui: &ClonkFontSet,
        book: &BookFonts,
        program: &ProgramSheetState,
        gamma: Option<&GammaRamp>,
    ) {
        let state = OptionsDlgState::new(program.clone());
        Self::render_state(surface, assets, gui, book, &state, gamma);
    }

    /// Draws one live steady-state frame in the C++ draw order (spec section
    /// 4). The caller applies the final whole-surface gamma pass.
    pub fn render_state(
        surface: &mut Surface,
        assets: &OptionsDlgAssets,
        gui: &ClonkFontSet,
        book: &BookFonts,
        state: &OptionsDlgState,
        gamma: Option<&GammaRamp>,
    ) {
        Self::render_state_with_draw_focus(surface, assets, gui, book, state, gamma, true);
    }

    /// Draws the same state while explicitly modeling whether this dialog owns
    /// C++ `HasDrawFocus`/active-dialog hover. Overlays pass `false`; the
    /// source-compatible [`Self::render_state`] assumes Options is active.
    pub fn render_state_with_draw_focus(
        surface: &mut Surface,
        assets: &OptionsDlgAssets,
        gui: &ClonkFontSet,
        book: &BookFonts,
        state: &OptionsDlgState,
        gamma: Option<&GammaRamp>,
        draw_focus: bool,
    ) {
        let (w, h) = (surface.width() as i32, surface.height() as i32);
        let layout = options_dlg_layout(w, h, gui, book);

        // 1. Loader background, stretched fullscreen (C4Gui.cpp:669-682).
        let full = GuiRect::new(0.0, 0.0, w as f32, h as f32);
        draw_image_bilinear(surface, &full, &assets.background, gamma);

        // 2. Title label, GUI TitleFont, yellow, centered.
        gui.title.draw_with_gamma(
            surface,
            layout.title_center.0,
            layout.title_center.1,
            "Options",
            YELLOW_FONT_RGBA,
            TextAlign::Center,
            true,
            gamma,
        );

        // 3. Back button: GUIButton 3-slice bar + CaptionFont caption
        // (Button::DrawElement, C4GuiButton.cpp:81-110).
        let b = layout.back_button;
        draw_bar(
            surface,
            &GuiRect::new(b.x as f32, b.y as f32, b.w as f32, b.h as f32),
            &assets.button,
            gamma,
        );
        if draw_focus && state.back_highlighted() {
            draw_image_bilinear_additive(
                surface,
                &GuiRect::new(
                    (b.x + 5) as f32,
                    (b.y + 3) as f32,
                    (b.w - 10) as f32,
                    (b.h - 6) as f32,
                ),
                &retained_blackened_image(&assets.button_highlight),
                gamma,
            );
        }
        let font = gui.button_font(b.h);
        let pressed_offset = i32::from(state.pressed_back);
        font.draw_with_gamma(
            surface,
            (b.x + b.x + b.w - 1) / 2 + pressed_offset,
            (b.y + b.y + b.h - 1 - font.line_height) / 2 + pressed_offset,
            &state.labels.back,
            YELLOW_FONT_RGBA,
            TextAlign::Center,
            true,
            gamma,
        );

        // 4. Tab strip: inactive captions, then paper, then the active
        // caption + focus highlight (Tabular::DrawElement, C4GuiTabular.cpp:
        // 388-458).
        let active_sheet = state.active_sheet().index();
        for i in 0..state.labels.sheets.len() {
            if i != active_sheet {
                Self::draw_tab_caption(surface, assets, book, &layout, &state.labels, i, gamma);
            }
        }
        let p = layout.paper;
        draw_image_bilinear_white_pad(
            surface,
            &GuiRect::new(p.x as f32, p.y as f32, p.w as f32, p.h as f32),
            &assets.paper,
            gamma,
        );
        Self::draw_tab_caption(
            surface,
            assets,
            book,
            &layout,
            &state.labels,
            active_sheet,
            gamma,
        );
        if draw_focus && state.tabular_focused() {
            let mut f = layout.focus_highlight;
            f.y += 72 * active_sheet as i32;
            draw_image_bilinear_additive(
                surface,
                &GuiRect::new(f.x as f32, f.y as f32, f.w as f32, f.h as f32),
                &retained_blackened_image(&assets.button_highlight),
                gamma,
            );
        }

        // Active sheet only (C4GuiTabular.cpp:258-267).
        match state.active_sheet() {
            OptionsSheet::Sound => {
                Self::draw_sound_sheet(
                    surface,
                    assets,
                    book,
                    &layout.sound,
                    state,
                    gamma,
                    draw_focus,
                );
                return;
            }
            OptionsSheet::Graphics => {
                Self::draw_graphics_sheet(
                    surface,
                    assets,
                    book,
                    &layout.graphics,
                    state,
                    gamma,
                    draw_focus,
                );
                return;
            }
            OptionsSheet::Keyboard => {
                Self::draw_control_sheet(
                    surface,
                    assets,
                    book,
                    &layout.controls,
                    state,
                    ControlDevice::Keyboard,
                    gamma,
                    draw_focus,
                );
                return;
            }
            OptionsSheet::Gamepad => {
                Self::draw_control_sheet(
                    surface,
                    assets,
                    book,
                    &layout.controls,
                    state,
                    ControlDevice::Gamepad,
                    gamma,
                    draw_focus,
                );
                return;
            }
            OptionsSheet::Network => {
                Self::draw_network_sheet(
                    surface,
                    assets,
                    book,
                    &layout.network,
                    state,
                    gamma,
                    draw_focus,
                );
                return;
            }
            OptionsSheet::Program => {}
        }

        // 5. Program sheet children, add order (ctor 675-792).
        let program = state.program();
        let black = STARTUP_FONT_RGBA;
        let draw_book_left = |surface: &mut Surface, pos: (i32, i32), text: &str| {
            book.book.draw_with_gamma(
                surface,
                pos.0,
                pos.1,
                text,
                black,
                TextAlign::Left,
                true,
                gamma,
            );
        };
        draw_book_left(
            surface,
            layout.language_label,
            &format!("{}:", state.labels.language),
        );
        Self::draw_combo(
            surface,
            assets,
            book,
            &layout.language_combo,
            &program.language_text,
            draw_focus
                && state.program_control_highlighted(OptionsProgramFocusTarget::LanguageCombo),
            gamma,
        );
        draw_book_left(surface, layout.language_info, &program.language_info);
        draw_book_left(
            surface,
            layout.font_label,
            &format!("{}:", state.labels.font),
        );
        Self::draw_combo(
            surface,
            assets,
            book,
            &layout.font_face_combo,
            &program.font_face,
            draw_focus
                && state.program_control_highlighted(OptionsProgramFocusTarget::FontFaceCombo),
            gamma,
        );
        Self::draw_combo(
            surface,
            assets,
            book,
            &layout.font_size_combo,
            &program.font_size,
            draw_focus
                && state.program_control_highlighted(OptionsProgramFocusTarget::FontSizeCombo),
            gamma,
        );
        draw_book_left(
            surface,
            layout.white_chat_label,
            &format!("{}:", state.labels.white_chat),
        );
        Self::draw_checkbox(
            surface,
            assets,
            book,
            &layout.ingame_check,
            &state.labels.white_chat_ingame,
            program.white_chat_ingame,
            draw_focus
                && state.program_control_highlighted(OptionsProgramFocusTarget::WhiteChatIngame),
            gamma,
        );
        Self::draw_checkbox(
            surface,
            assets,
            book,
            &layout.lobby_check,
            &state.labels.white_chat_lobby,
            program.white_chat_lobby,
            draw_focus
                && state.program_control_highlighted(OptionsProgramFocusTarget::WhiteChatLobby),
            gamma,
        );
        Self::draw_checkbox(
            surface,
            assets,
            book,
            &layout.timestamps_check,
            &state.labels.timestamps,
            program.show_log_timestamps,
            draw_focus
                && state.program_control_highlighted(OptionsProgramFocusTarget::ShowLogTimestamps),
            gamma,
        );
        Self::draw_checkbox(
            surface,
            assets,
            book,
            &layout.preloading_check,
            &state.labels.preloading,
            program.preloading,
            draw_focus && state.program_control_highlighted(OptionsProgramFocusTarget::Preloading),
            gamma,
        );
        Self::draw_fair_crew_group(surface, assets, book, &layout, state, gamma);
        Self::draw_small_button(
            surface,
            assets,
            book,
            &layout.reset_button,
            &state.labels.reset_config,
            draw_focus && state.program_control_highlighted(OptionsProgramFocusTarget::ResetButton),
            state.program_button_pressed(OptionsProgramFocusTarget::ResetButton),
            gamma,
        );
        Self::draw_small_button(
            surface,
            assets,
            book,
            &layout.advanced_button,
            &state.labels.advanced_settings,
            draw_focus
                && state.program_control_highlighted(OptionsProgramFocusTarget::AdvancedButton),
            state.program_button_pressed(OptionsProgramFocusTarget::AdvancedButton),
            gamma,
        );
    }

    /// Sheet::DrawCaption with clip gfx (C4GuiTabular.cpp:59-64): clip 1:1,
    /// icon phase 1:1, BookSmallFont title centered, all black.
    fn draw_tab_caption(
        surface: &mut Surface,
        assets: &OptionsDlgAssets,
        book: &BookFonts,
        layout: &OptionsDlgLayout,
        labels: &OptionsLabels,
        index: usize,
        gamma: Option<&GammaRamp>,
    ) {
        let (cx, cy) = layout.tab_clips[index];
        draw_image_strip(surface, cx, cy, &assets.tab_clip, 0, 0, 120, 80, gamma);
        let (ix, iy) = layout.tab_icons[index];
        draw_image_strip(
            surface,
            ix,
            iy,
            &assets.option_icons,
            32 * index as u32,
            0,
            32,
            32,
            gamma,
        );
        let (tx, ty) = layout.tab_captions[index];
        book.book_small.draw_with_gamma(
            surface,
            tx,
            ty,
            &labels.sheets[index],
            STARTUP_FONT_RGBA,
            TextAlign::Center,
            true,
            gamma,
        );
    }

    /// ComboBox::DrawElement (C4GuiComboBox.cpp:138-185): invisible bg box,
    /// two nested frames, side arrow phase 0, BookFont text.
    fn draw_combo(
        surface: &mut Surface,
        assets: &OptionsDlgAssets,
        book: &BookFonts,
        rect: &IntRect,
        text: &str,
        highlighted: bool,
        gamma: Option<&GammaRamp>,
    ) {
        // DrawBoxDw with C4StartupEditBGColor = 0xff000000 -> opacity 0, skip.
        let (x0, y0) = (rect.x, rect.y);
        let (x2, y2) = (x0 + rect.w, y0 + rect.h);
        draw_frame_dw(surface, x0, y0, x2, y2 - 1, EDIT_BORDER_COLOR, gamma);
        draw_frame_dw(
            surface,
            x0 + 1,
            y0 + 1,
            x2 - 1,
            y2 - 2,
            EDIT_BORDER_COLOR,
            gamma,
        );
        // Side arrow: startup fctContext phase 0, 16x16 (C4Startup.cpp:64-65).
        draw_image_strip(
            surface,
            x0 + rect.w - 16 - 1,
            y0 + (rect.h - 16) / 2,
            &assets.context_arrow,
            0,
            0,
            16,
            16,
            gamma,
        );
        book.book.draw_with_gamma(
            surface,
            x0 + 16 + 2,
            y0 + (rect.h - book.book.line_height) / 2,
            text,
            STARTUP_FONT_RGBA,
            TextAlign::Left,
            true,
            gamma,
        );
        if highlighted {
            draw_image_bilinear_additive(
                surface,
                &GuiRect::new(x0 as f32, y0 as f32, rect.w as f32, rect.h as f32),
                &retained_blackened_image(&assets.button_highlight),
                gamma,
            );
        }
    }

    /// CheckBox::DrawElement (C4GuiCheckBox.cpp:110-137): box facet phase
    /// `checked` stretched to H x H, BookFont label at x + H + 4.
    fn draw_checkbox(
        surface: &mut Surface,
        assets: &OptionsDlgAssets,
        book: &BookFonts,
        rect: &IntRect,
        label: &str,
        checked: bool,
        highlighted: bool,
        gamma: Option<&GammaRamp>,
    ) {
        let phase = retained_blackened_image(&crop_image(
            &assets.checkbox,
            if checked { 32 } else { 0 },
            0,
            32,
            32,
        ));
        draw_image_bilinear(
            surface,
            &GuiRect::new(rect.x as f32, rect.y as f32, rect.h as f32, rect.h as f32),
            &phase,
            gamma,
        );
        if highlighted {
            let size = rect.h / 2;
            draw_image_bilinear_additive(
                surface,
                &GuiRect::new(
                    (rect.x + rect.h / 4) as f32,
                    (rect.y + rect.h / 4) as f32,
                    size as f32,
                    size as f32,
                ),
                &retained_blackened_image(&assets.button_highlight),
                gamma,
            );
        }
        let y_off = (rect.h - book.book.line_height).max(0) / 2;
        book.book.draw_with_gamma(
            surface,
            rect.x + rect.h + 4,
            rect.y + y_off,
            label,
            STARTUP_FONT_RGBA,
            TextAlign::Left,
            true,
            gamma,
        );
    }

    /// Sound page construction/draw order from
    /// `C4StartupOptionsDlg.cpp:925-985`.
    fn draw_sound_sheet(
        surface: &mut Surface,
        assets: &OptionsDlgAssets,
        book: &BookFonts,
        layout: &SoundSheetLayout,
        state: &OptionsDlgState,
        gamma: Option<&GammaRamp>,
        draw_focus: bool,
    ) {
        Self::draw_group_box(surface, book, &layout.frontend_group, "Frontend", gamma);
        for id in [
            SoundCheckboxId::FrontendMusic,
            SoundCheckboxId::FrontendSoundEffects,
        ] {
            Self::draw_checkbox(
                surface,
                assets,
                book,
                &layout.checkbox(id),
                sound_checkbox_label(&state.labels, id),
                state.sound.checkbox(id),
                draw_focus && state.sound_checkbox_highlighted(id),
                gamma,
            );
        }

        Self::draw_group_box(surface, book, &layout.game_group, "Game", gamma);
        for id in [
            SoundCheckboxId::GameMusic,
            SoundCheckboxId::GameSoundEffects,
        ] {
            Self::draw_checkbox(
                surface,
                assets,
                book,
                &layout.checkbox(id),
                sound_checkbox_label(&state.labels, id),
                state.sound.checkbox(id),
                draw_focus && state.sound_checkbox_highlighted(id),
                gamma,
            );
        }

        Self::draw_group_box(surface, book, &layout.volume_group, "Volume control", gamma);
        for id in SoundVolumeId::ALL {
            let i = id.index();
            book.book.draw_with_gamma(
                surface,
                layout.volume_headings[i].x,
                layout.volume_headings[i].y,
                &sound_volume_heading(&state.labels, id),
                STARTUP_FONT_RGBA,
                TextAlign::Left,
                true,
                gamma,
            );
            for (rect, text) in [
                (&layout.quiet_labels[i], "quiet"),
                (&layout.loud_labels[i], "loud"),
            ] {
                book.book.draw_with_gamma(
                    surface,
                    rect.x + rect.w / 2,
                    rect.y,
                    text,
                    STARTUP_FONT_RGBA,
                    TextAlign::Center,
                    true,
                    gamma,
                );
            }
            let slider = layout.slider(id);
            Self::draw_book_scrollbar(
                surface,
                assets,
                &slider,
                state.sound_slider_position(id, slider),
                state.sound_arrow_pressed(id, SoundSliderDirection::Decrement),
                state.sound_arrow_pressed(id, SoundSliderDirection::Increment),
                gamma,
            );
        }
    }

    fn draw_graphics_sheet(
        surface: &mut Surface,
        assets: &OptionsDlgAssets,
        book: &BookFonts,
        layout: &GraphicsSheetLayout,
        state: &OptionsDlgState,
        gamma: Option<&GammaRamp>,
        draw_focus: bool,
    ) {
        let graphics = state.graphics();
        Self::draw_group_box(surface, book, &layout.display_group, "Display", gamma);
        Self::draw_group_box(surface, book, &layout.options_group, "Options", gamma);
        Self::draw_group_box(surface, book, &layout.effects_group, "Effects Level", gamma);
        for (rect, text) in [
            (
                &layout.display_mode_label,
                format!("{}:", state.labels.display_mode),
            ),
            (
                &layout.scale_label,
                format!("{}:", state.labels.graphics_scale),
            ),
            (&layout.percent_label, "%".to_string()),
            (&layout.low_label, state.labels.effects_low.clone()),
            (&layout.high_label, state.labels.effects_high.clone()),
        ] {
            book.book.draw_with_gamma(
                surface,
                rect.x,
                rect.y,
                &text,
                STARTUP_FONT_RGBA,
                TextAlign::Left,
                true,
                gamma,
            );
        }
        Self::draw_combo(
            surface,
            assets,
            book,
            &layout.display_mode_combo,
            graphics.display_mode.label(),
            draw_focus
                && state.graphics_control_highlighted(OptionsGraphicsFocusTarget::DisplayModeCombo),
            gamma,
        );
        Self::draw_edit(
            surface,
            book,
            &layout.scale_edit,
            &graphics.proposed_scale_percent.to_string(),
            draw_focus
                && matches!(
                    state.focus,
                    OptionsFocus::Graphics(OptionsGraphicsFocusTarget::ScaleEdit)
                ),
            gamma,
        );
        Self::draw_small_button(
            surface,
            assets,
            book,
            &layout.apply_button,
            &state.labels.apply_scale,
            draw_focus
                && state.graphics_control_highlighted(OptionsGraphicsFocusTarget::ApplyScale),
            false,
            gamma,
        );
        Self::draw_book_scrollbar(
            surface,
            assets,
            &layout.scale_slider,
            state.graphics_slider_position(GraphicsSliderId::Scale, layout.scale_slider),
            state.graphics_arrow_pressed(
                GraphicsSliderId::Scale,
                GraphicsSliderDirection::Decrement,
            ),
            state.graphics_arrow_pressed(
                GraphicsSliderId::Scale,
                GraphicsSliderDirection::Increment,
            ),
            gamma,
        );
        for id in GraphicsCheckboxId::ALL {
            Self::draw_checkbox(
                surface,
                assets,
                book,
                &layout.checkbox(id),
                id.label(),
                graphics.checkbox(id),
                draw_focus
                    && state.graphics_control_highlighted(OptionsGraphicsFocusTarget::Checkbox(id)),
                gamma,
            );
        }
        Self::draw_book_scrollbar(
            surface,
            assets,
            &layout.smoke_slider,
            state.graphics_slider_position(GraphicsSliderId::SmokeLevel, layout.smoke_slider),
            state.graphics_arrow_pressed(
                GraphicsSliderId::SmokeLevel,
                GraphicsSliderDirection::Decrement,
            ),
            state.graphics_arrow_pressed(
                GraphicsSliderId::SmokeLevel,
                GraphicsSliderDirection::Increment,
            ),
            gamma,
        );
    }

    fn draw_control_sheet(
        surface: &mut Surface,
        assets: &OptionsDlgAssets,
        book: &BookFonts,
        layout: &ControlSheetLayout,
        state: &OptionsDlgState,
        device: ControlDevice,
        gamma: Option<&GammaRamp>,
        draw_focus: bool,
    ) {
        for set in 0..state.controls().visible_sets(device) {
            let label = match device {
                ControlDevice::Keyboard => format!("Keyboard {}", set + 1),
                ControlDevice::Gamepad => format!("Gamepad {}", set + 1),
            };
            // `fctCtrlPic = fGamepad ? fctGamepad : fctKeyboard`, drawn with
            // the set index as its phase (C4StartupOptionsDlg.cpp:271).
            let selector = match device {
                ControlDevice::Keyboard => assets
                    .control
                    .as_ref()
                    .map(|image| (image, control_facets::KEYBOARD)),
                ControlDevice::Gamepad => assets.gamepad.as_ref().map(|image| {
                    (
                        image,
                        IntRect {
                            x: 0,
                            y: 0,
                            w: control_facets::GAMEPAD_PHASE_WIDTH,
                            h: image.height() as i32,
                        },
                    )
                }),
            };
            if let Some((image, cell)) = selector {
                let rect = layout.set_buttons[set];
                let source = control_facets::phase_rect(cell, set);
                crate::classic_gui::draw_facet_stretch(
                    surface,
                    image,
                    (
                        source.x as f32,
                        source.y as f32,
                        source.w as f32,
                        source.h as f32,
                    ),
                    (rect.x as f32, rect.y as f32, rect.w as f32, rect.h as f32),
                    gamma,
                );
                continue;
            }
            Self::draw_small_button(
                surface,
                assets,
                book,
                &layout.set_buttons[set],
                &label,
                draw_focus && state.control_sheet_control_highlighted(ControlSheetHit::Set(set)),
                false,
                gamma,
            );
        }
        for (control, label) in state
            .labels()
            .control_keys
            .iter()
            .enumerate()
            .take(CONTROL_KEY_COUNT)
        {
            let rect = layout.key_buttons[control];
            let binding = state
                .controls()
                .visible_label(device, control)
                .unwrap_or("Undefined");
            // `KeySelButton::DrawElement` blits fctKey then an inset fctCommand
            // (C4StartupOptionsDlg.cpp:215-240). Without the facet the sheet
            // keeps its text button, which is what a headless run gets.
            if let Some(control_facet) = assets.control.as_ref() {
                let facets = key_button_facets(rect, control, false);
                let key_cell = control_facets::phase_rect(control_facets::KEY, facets.key_phase);
                crate::classic_gui::draw_facet_stretch(
                    surface,
                    control_facet,
                    (
                        key_cell.x as f32,
                        key_cell.y as f32,
                        key_cell.w as f32,
                        key_cell.h as f32,
                    ),
                    (rect.x as f32, rect.y as f32, rect.w as f32, rect.h as f32),
                    gamma,
                );
                let command_cell =
                    control_facets::phase_rect(control_facets::COMMAND, facets.command_phase);
                let target = facets.command_rect;
                crate::classic_gui::draw_facet_stretch(
                    surface,
                    control_facet,
                    (
                        command_cell.x as f32,
                        command_cell.y as f32,
                        command_cell.w as f32,
                        command_cell.h as f32,
                    ),
                    (
                        target.x as f32,
                        target.y as f32,
                        target.w as f32,
                        target.h as f32,
                    ),
                    gamma,
                );
                continue;
            }
            Self::draw_small_button(
                surface,
                assets,
                book,
                &rect,
                binding,
                draw_focus
                    && state.control_sheet_control_highlighted(ControlSheetHit::Key(control)),
                false,
                gamma,
            );
            book.book_small.draw_with_gamma(
                surface,
                rect.x + rect.w / 2,
                rect.y - book.book_small.line_height,
                label,
                STARTUP_FONT_RGBA,
                TextAlign::Center,
                true,
                gamma,
            );
        }
        Self::draw_small_button(
            surface,
            assets,
            book,
            &layout.reset_button,
            &state.labels.reset_keyboard,
            draw_focus && state.control_sheet_control_highlighted(ControlSheetHit::Reset),
            false,
            gamma,
        );
        if device == ControlDevice::Gamepad && state.controls().gamepad_gui_checkbox_visible() {
            Self::draw_checkbox(
                surface,
                assets,
                book,
                &layout.gamepad_gui_check,
                "GUI control",
                state.controls().gamepad_gui_control(),
                draw_focus && state.control_sheet_control_highlighted(ControlSheetHit::GamepadGui),
                gamma,
            );
        }
    }

    fn draw_network_sheet(
        surface: &mut Surface,
        assets: &OptionsDlgAssets,
        book: &BookFonts,
        layout: &NetworkSheetLayout,
        state: &OptionsDlgState,
        gamma: Option<&GammaRamp>,
        draw_focus: bool,
    ) {
        let network = state.network();
        for id in NetworkPortId::ALL {
            let title = match id {
                NetworkPortId::Tcp => "TCP port",
                NetworkPortId::Udp => "UDP port",
                NetworkPortId::Reference => state.labels.port_reference.as_str(),
                NetworkPortId::Discovery => state.labels.port_discovery.as_str(),
            };
            let group = layout.port_controls[id.index()];
            Self::draw_group_box(surface, book, &group, title, gamma);
            Self::draw_checkbox(
                surface,
                assets,
                book,
                &layout.port_check(id),
                &state.labels.active,
                network.port(id).enabled,
                draw_focus && state.network_control_highlighted(NetworkSheetHit::PortCheck(id)),
                gamma,
            );
            if network.port(id).enabled {
                Self::draw_edit(
                    surface,
                    book,
                    &layout.port_edit(id),
                    &network.port(id).port.to_string(),
                    draw_focus
                        && matches!(
                            state.focus,
                            OptionsFocus::Network(NetworkSheetHit::Text(
                                NetworkTextField::Port(focused),
                            )) if focused == id
                        ),
                    gamma,
                );
            }
        }
        Self::draw_checkbox(
            surface,
            assets,
            book,
            &layout.alternate_check,
            &state.labels.use_other_server,
            network.use_alternate_server,
            draw_focus
                && state.network_control_highlighted(NetworkSheetHit::Checkbox(
                    NetworkCheckboxId::UseAlternateServer,
                )),
            gamma,
        );
        if network.use_alternate_server {
            Self::draw_edit(
                surface,
                book,
                &layout.alternate_edit,
                &network.alternate_server_address,
                draw_focus
                    && matches!(
                        state.focus,
                        OptionsFocus::Network(NetworkSheetHit::Text(
                            NetworkTextField::AlternateServerAddress,
                        ))
                    ),
                gamma,
            );
        }
        Self::draw_checkbox(
            surface,
            assets,
            book,
            &layout.automatic_update_check,
            &state.labels.automatic_updates,
            network.automatic_update,
            draw_focus
                && state.network_control_highlighted(NetworkSheetHit::Checkbox(
                    NetworkCheckboxId::AutomaticUpdate,
                )),
            gamma,
        );
        Self::draw_checkbox(
            surface,
            assets,
            book,
            &layout.upnp_check,
            &state.labels.upnp,
            network.enable_upnp,
            draw_focus
                && state.network_control_highlighted(NetworkSheetHit::Checkbox(
                    NetworkCheckboxId::EnableUpnp,
                )),
            gamma,
        );
        for (label, rect, value, field) in [
            (
                &state.labels.computer_name,
                &layout.local_name_edit,
                network.local_name.as_str(),
                NetworkTextField::LocalName,
            ),
            (
                &state.labels.chat_name,
                &layout.nick_edit,
                network.nick.as_str(),
                NetworkTextField::Nick,
            ),
        ] {
            book.book_small.draw_with_gamma(
                surface,
                rect.x,
                rect.y - book.book_small.line_height,
                label,
                STARTUP_FONT_RGBA,
                TextAlign::Left,
                true,
                gamma,
            );
            Self::draw_edit(
                surface,
                book,
                rect,
                value,
                draw_focus
                    && matches!(
                        state.focus,
                        OptionsFocus::Network(NetworkSheetHit::Text(focused))
                            if focused == field
                    ),
                gamma,
            );
        }
    }

    fn draw_edit(
        surface: &mut Surface,
        book: &BookFonts,
        rect: &IntRect,
        text: &str,
        focused: bool,
        gamma: Option<&GammaRamp>,
    ) {
        draw_frame_dw(
            surface,
            rect.x,
            rect.y,
            rect.x + rect.w,
            rect.y + rect.h - 1,
            EDIT_BORDER_COLOR,
            gamma,
        );
        let text_y = rect.y + (rect.h - book.book.line_height).max(0) / 2;
        if focused && !text.is_empty() {
            let text_width = book.book.measure(text, true).0;
            fill_box_dw(
                surface,
                rect.x + 4,
                text_y + 1,
                (rect.x + 3 + text_width).min(rect.x + rect.w - 2),
                text_y + book.book.line_height - 2,
                0x7f7f_7f00,
                gamma,
            );
        }
        book.book.draw_with_gamma(
            surface,
            rect.x + 4,
            text_y,
            text,
            STARTUP_FONT_RGBA,
            TextAlign::Left,
            true,
            gamma,
        );
    }

    /// Titled `GroupBox::DrawElement` branch
    /// (`C4GuiContainers.cpp:633-677`).
    fn draw_group_box(
        surface: &mut Surface,
        book: &BookFonts,
        group: &IntRect,
        title: &str,
        gamma: Option<&GammaRamp>,
    ) {
        book.book.draw_with_gamma(
            surface,
            group.x + 7 + 2,
            group.y,
            title,
            STARTUP_FONT_RGBA,
            TextAlign::Left,
            true,
            gamma,
        );
        let gap_w = book.book.measure(title, true).0 + 4;
        let (x1, y1) = (group.x, group.y + book.book.line_height / 2);
        let (x2, y2) = (group.x + group.w, y1 + group.h - book.book.line_height / 2);
        for i in 0..2 {
            draw_line_dw(
                surface,
                x1 + i,
                y1,
                x1 + i,
                y2 - 1,
                EDIT_BORDER_COLOR,
                gamma,
            );
            draw_line_dw(
                surface,
                x1 + 2,
                y1 + i,
                x1 + 7,
                y1 + i,
                EDIT_BORDER_COLOR,
                gamma,
            );
            draw_line_dw(
                surface,
                x1 + 7 + gap_w,
                y1 + i,
                x2 - 3,
                y1 + i,
                EDIT_BORDER_COLOR,
                gamma,
            );
            draw_line_dw(
                surface,
                x2 - 1 - i,
                y1,
                x2 - 1 - i,
                y2 - 1,
                EDIT_BORDER_COLOR,
                gamma,
            );
            draw_line_dw(
                surface,
                x1 + 2,
                y2 - 1 - i,
                x2 - 3,
                y2 - 1 - i,
                EDIT_BORDER_COLOR,
                gamma,
            );
        }
    }

    fn draw_book_scrollbar(
        surface: &mut Surface,
        assets: &OptionsDlgAssets,
        rect: &IntRect,
        scroll_pos: i32,
        decrement_pressed: bool,
        increment_pressed: bool,
        gamma: Option<&GammaRamp>,
    ) {
        draw_horizontal_book_scrollbar(
            surface,
            &assets.book_scroll,
            rect,
            scroll_pos,
            decrement_pressed,
            increment_pressed,
            (16, 16),
            gamma,
        );
    }

    /// GroupBox::DrawElement (C4GuiContainers.cpp:633-677, titled-frame
    /// branch) plus its children: weak/strong labels and the slider.
    fn draw_fair_crew_group(
        surface: &mut Surface,
        assets: &OptionsDlgAssets,
        book: &BookFonts,
        layout: &OptionsDlgLayout,
        state: &OptionsDlgState,
        gamma: Option<&GammaRamp>,
    ) {
        let g = layout.group;
        let title = state.labels.fair_crew_strength.as_str();
        book.book.draw_with_gamma(
            surface,
            g.x + 7 + 2,
            g.y,
            title,
            STARTUP_FONT_RGBA,
            TextAlign::Left,
            true,
            gamma,
        );
        let gap_w = book.book.measure(title, true).0 + 4;
        let (x1, y1) = (g.x, g.y + book.book.line_height / 2);
        let (x2, y2) = (g.x + g.w, y1 + g.h - book.book.line_height / 2);
        for i in 0..2 {
            draw_line_dw(
                surface,
                x1 + i,
                y1,
                x1 + i,
                y2 - 1,
                EDIT_BORDER_COLOR,
                gamma,
            ); // left
            draw_line_dw(
                surface,
                x1 + 2,
                y1 + i,
                x1 + 7,
                y1 + i,
                EDIT_BORDER_COLOR,
                gamma,
            ); // top-left
            draw_line_dw(
                surface,
                x1 + 7 + gap_w,
                y1 + i,
                x2 - 3,
                y1 + i,
                EDIT_BORDER_COLOR,
                gamma,
            ); // top-right
            draw_line_dw(
                surface,
                x2 - 1 - i,
                y1,
                x2 - 1 - i,
                y2 - 1,
                EDIT_BORDER_COLOR,
                gamma,
            ); // right
            draw_line_dw(
                surface,
                x1 + 2,
                y2 - 1 - i,
                x2 - 3,
                y2 - 1 - i,
                EDIT_BORDER_COLOR,
                gamma,
            ); // bottom
        }
        // Children: weak label, strong label, slider (add order, ctor 769-779).
        let center_label = |surface: &mut Surface, r: &IntRect, text: &str| {
            book.book.draw_with_gamma(
                surface,
                r.x + r.w / 2,
                r.y,
                text,
                STARTUP_FONT_RGBA,
                TextAlign::Center,
                true,
                gamma,
            );
        };
        center_label(surface, &layout.weak_label, "weak");
        center_label(surface, &layout.strong_label, "strong");

        Self::draw_book_scrollbar(
            surface,
            assets,
            &layout.slider,
            state.fair_crew_slider_position(layout.slider),
            state.fair_crew_arrow_pressed(ProgramSliderDirection::Decrement),
            state.fair_crew_arrow_pressed(ProgramSliderDirection::Increment),
            gamma,
        );
    }

    /// SmallButton::DrawElement (C4StartupOptionsDlg.cpp:69-98): four beveled
    /// border quads + centered BookFont caption.
    fn draw_small_button(
        surface: &mut Surface,
        assets: &OptionsDlgAssets,
        book: &BookFonts,
        rect: &IntRect,
        text: &str,
        highlighted: bool,
        pressed: bool,
        gamma: Option<&GammaRamp>,
    ) {
        let (x0, y0) = (rect.x, rect.y);
        let (x1, y1) = (rect.x + rect.w, rect.y + rect.h);
        let text_h = book.book.line_height;
        let i = ((rect.h - text_h) / 3).clamp(2, 5);
        let (high, low) = if pressed {
            (BTN_BORDER_COLOR2, BTN_BORDER_COLOR1)
        } else {
            (BTN_BORDER_COLOR1, BTN_BORDER_COLOR2)
        };
        fill_quad_dw(
            surface,
            &[(x0, y0), (x1, y0), (x1 - i, y0 + i), (x0, y0 + i)],
            high,
            gamma,
        );
        fill_quad_dw(
            surface,
            &[(x0, y0), (x0 + i, y0), (x0 + i, y1 - i), (x0, y1)],
            high,
            gamma,
        );
        fill_quad_dw(
            surface,
            &[(x1, y0), (x1, y1), (x1 - i, y1), (x1 - i, y0 + i)],
            low,
            gamma,
        );
        fill_quad_dw(
            surface,
            &[(x1, y1), (x0, y1), (x0 + i, y1 - i), (x1, y1 - i)],
            low,
            gamma,
        );
        let text_offset = if pressed { i } else { 0 };
        if highlighted {
            draw_image_bilinear_additive(
                surface,
                &GuiRect::new(
                    (x0 + 5 + text_offset) as f32,
                    (y0 + 3 + text_offset) as f32,
                    (rect.w - 10) as f32,
                    (rect.h - 6) as f32,
                ),
                &retained_blackened_image(&assets.button_highlight),
                gamma,
            );
        }
        book.book.draw_with_gamma(
            surface,
            (x0 + x1) / 2 + text_offset,
            (y0 + y1 - text_h) / 2 + text_offset,
            text,
            BTN_FONT_RGBA,
            TextAlign::Center,
            true,
            gamma,
        );
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use crate::startup_options_controls::CONTROL_SET_COUNT;
    use crate::test_support::{
        endeavour_font_set, load_graphics_png, repo_root, standard_gamma, write_ppm,
    };
    use clonk_graphics::PixelFormat;

    fn book_fonts() -> BookFonts {
        let path = repo_root().join("planet/System.c4g/Endeavour.ttf");
        let bytes = std::fs::read(&path).unwrap_or_else(|e| panic!("read {}: {e}", path.display()));
        build_book_fonts(&bytes).expect("build book fonts")
    }

    fn options_assets() -> OptionsDlgAssets {
        OptionsDlgAssets {
            background: load_graphics_png("LoaderGoldmine1.png"),
            paper: load_graphics_png("StartupDlgPaper.png"),
            tab_clip: load_graphics_png("StartupTabClip.png"),
            option_icons: load_graphics_png("StartupOptionIcons.png"),
            book_scroll: load_graphics_png("StartupBookScroll.png"),
            context_arrow: load_graphics_png("StartupContext.png"),
            checkbox: load_graphics_png("GUICheckBox.png"),
            button_highlight: load_graphics_png("GUIButtonHighlight.png"),
            button: load_graphics_png("GUIButton.png"),
            control: None,
            gamepad: None,
        }
    }

    // CStdFont::Init with fDoShadow=false (StdFont.cpp:319-358): BookFont
    // 14px lh 22, BookSmallFont 13px lh 20, iHSpace 0, iGfxLineHgt = iLineHgt.
    #[test]
    fn book_fonts_match_cpp_shadowless_metrics() {
        let fonts = book_fonts();
        assert_eq!(fonts.book.line_height, 22);
        assert_eq!(fonts.book.cell_height, 22);
        assert_eq!(fonts.book.h_space, 0);
        assert_eq!(fonts.book_small.line_height, 20);
        assert_eq!(fonts.book_small.cell_height, 20);
        assert_eq!(fonts.book_small.h_space, 0);
    }

    #[test]
    fn retained_options_paper_is_one_native_tiled_quad() {
        let mut pixels = [180, 160, 120, 255].repeat(64);
        pixels[0..4].copy_from_slice(&[255, 255, 255, 0]);
        let image = ImageData::new(8, 8, pixels);
        let mut surface = Surface::new(180, 120, PixelFormat::Rgba8888);
        surface.begin_gpu_scene_capture();
        draw_image_bilinear_white_pad(
            &mut surface,
            &GuiRect::new(4.0, 6.0, 170.0, 108.0),
            &image,
            None,
        );

        let scene = surface
            .take_gpu_scene_capture()
            .expect("capture remains active")
            .into_scene([180, 120], Color::transparent(), &GammaRamp::identity());
        assert_eq!(scene.textures.len(), 1);
        assert_eq!(&scene.textures[0].pixels[0..4], &[0, 0, 0, 0]);
        assert_eq!(scene.commands.len(), 1);
        let clonk_graphics::GpuCommand::Quad {
            vertices, sampler, ..
        } = &scene.commands[0]
        else {
            panic!("options paper was not retained as a texture quad");
        };
        assert_eq!(*sampler, clonk_graphics::GpuSampler::Linear);
        assert!(vertices.iter().all(|vertex| vertex.sample_tile[3] == 1.0));
    }

    #[test]
    fn retained_options_primitives_are_bounded_solid_commands() {
        let mut surface = Surface::new(100, 60, PixelFormat::Rgba8888);
        surface.begin_gpu_scene_capture();
        fill_box_dw(&mut surface, 2, 3, 80, 40, 0x7f20_4060, None);
        fill_quad_dw(
            &mut surface,
            &[(0, 0), (90, 0), (85, 5), (0, 5)],
            0x0040_6080,
            None,
        );

        let scene = surface
            .take_gpu_scene_capture()
            .expect("capture remains active")
            .into_scene([100, 60], Color::transparent(), &GammaRamp::identity());
        assert_eq!(scene.commands.len(), 1);
        let clonk_graphics::GpuCommand::Solid {
            vertices,
            topology,
            alpha_mode,
            ..
        } = &scene.commands[0]
        else {
            panic!("options primitives were not retained as solid geometry");
        };
        assert_eq!(
            *topology,
            clonk_graphics::GpuPrimitiveTopology::TriangleList
        );
        assert_eq!(*alpha_mode, clonk_graphics::GpuSolidAlphaMode::SourceOver);
        assert_eq!(vertices.len(), 12, "adjacent compatible quads coalesce");
    }

    #[test]
    fn retained_options_line_defers_scaled_diamond_rasterization() {
        let _renderer = crate::activate_advanced_renderer_config(crate::AdvancedRendererConfig {
            blit_offset: 100,
            ..crate::AdvancedRendererConfig::DEFAULT
        });
        let mut surface = Surface::new(8, 4, PixelFormat::Rgba8888);
        surface.begin_gpu_scene_capture();
        draw_line_dw(&mut surface, 5, 2, 1, 2, 0x7f20_4060, None);

        let scene = surface
            .take_gpu_scene_capture()
            .expect("capture remains active")
            .into_scene([8, 4], Color::transparent(), &GammaRamp::identity());
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
        assert_eq!(vertices[0].position, [5.5, 2.5, 1.0]);
        assert_eq!(vertices[1].position, [1.5, 2.5, 1.0]);
    }

    #[test]
    fn retained_rotated_vfacet_is_one_textured_command() {
        let image = ImageData::new(16, 16, [20, 40, 60, 255].repeat(256));
        let mut surface = Surface::new(64, 32, PixelFormat::Rgba8888);
        surface.begin_gpu_scene_capture();
        draw_rotated_vfacet(&mut surface, &image, 0, 0, 16, 4, 5, None);

        let scene = surface
            .take_gpu_scene_capture()
            .expect("capture remains active")
            .into_scene([64, 32], Color::transparent(), &GammaRamp::identity());
        assert_eq!(scene.commands.len(), 1);
        assert!(matches!(
            &scene.commands[0],
            clonk_graphics::GpuCommand::Quad { .. }
        ));
    }

    #[test]
    fn book_fonts_have_semantic_roles() {
        let fonts = book_fonts();
        assert_eq!(fonts.book.role(), Some(ClonkFontRole::BookText));
        assert_eq!(fonts.book_small.role(), Some(ClonkFontRole::BookSmall));
    }

    // Shadowless atlas pixels are pure white with alpha = coverage
    // (StdFont.cpp:228-232,255 with shadowSize = 0): no black shadow texels.
    #[test]
    fn book_glyphs_are_white_with_coverage_alpha() {
        let fonts = book_fonts();
        let cell = fonts.book.glyph('A').expect("glyph A");
        assert!(cell
            .pixels
            .iter()
            .all(|p| p.r == 255 && p.g == 255 && p.b == 255 || p.a == 0));
        assert!(cell.pixels.iter().any(|p| p.a == 255));
        // Shadowed GUI font of the same size has +1 shadow column.
        let gui = endeavour_font_set();
        let gui_cell = gui.text.glyph('A').expect("gui glyph A");
        assert_eq!(gui_cell.width, cell.width + 1);
    }

    // Pixel-exact geometry at 1280x720, derived from the C++ ctor
    // (C4StartupOptionsDlg.cpp:609-792) and verified against frame pixels in
    // build/Screenshots/ref-options.png (group frame x=417/418, x2=647;
    // language combo frame top y=139, left x=484; font combo top y=229).
    #[test]
    fn layout_matches_cpp_at_1280x720() {
        let gui = endeavour_font_set();
        let book = book_fonts();
        let l = options_dlg_layout(1280, 720, &gui, &book);

        assert_eq!(
            (l.client.x, l.client.y, l.client.w, l.client.h),
            (25, 69, 1230, 632)
        );
        assert_eq!(l.title_center, (640, 8));
        // Back button: (77,571) client -> (102,640); h = C4GUI_ButtonHgt.
        assert_eq!(
            (l.back_button.x, l.back_button.y, l.back_button.h),
            (102, 640, 32)
        );
        assert_eq!(l.back_button.w, 3 * gui.caption.measure("<< BACK", true).0);

        // Tabular after aspect fix: (218,2,794,538) client -> abs.
        assert_eq!(
            (l.tabular.x, l.tabular.y, l.tabular.w, l.tabular.h),
            (243, 71, 794, 538)
        );
        assert_eq!(
            (l.paper.x, l.paper.y, l.paper.w, l.paper.h),
            (338, 71, 699, 538)
        );
        for i in 0..6 {
            let i32i = i as i32;
            assert_eq!(l.tab_clips[i], (253, 86 + 72 * i32i), "clip {i}");
            assert_eq!(l.tab_icons[i], (284, 99 + 72 * i32i), "icon {i}");
            assert_eq!(l.tab_captions[i], (300, 133 + 72 * i32i), "caption {i}");
        }
        // Width 85: DrawCaption clamps iMaxWdt to the 95px clip caption size
        // (C4GuiTabular.cpp:393), not iMaxTabWidth = 115.
        assert_eq!(
            (
                l.focus_highlight.x,
                l.focus_highlight.y,
                l.focus_highlight.w,
                l.focus_highlight.h
            ),
            (258, 89, 85, 74)
        );
        // Sheet client: margins 113/37/37/39 (C4GuiTabular.h:108-111).
        assert_eq!(
            (l.sheet.x, l.sheet.y, l.sheet.w, l.sheet.h),
            (356, 108, 644, 462)
        );

        // Language rows: box at sheet-local (61,31); combo x pinned by the
        // reference frame pixels at x=484.
        assert_eq!(l.language_label, (417, 139));
        assert_eq!(
            (l.language_combo.x, l.language_combo.y, l.language_combo.h),
            (484, 139, 26)
        );
        assert_eq!(l.language_info, (417, 173));
        // Font row at local y=121.
        assert_eq!(l.font_label, (417, 229));
        assert_eq!(l.font_face_combo.y, 229);
        assert_eq!(l.font_size_combo.y, 229);
        assert_eq!(
            l.font_size_combo.x,
            l.font_face_combo.x + l.font_face_combo.w + 4
        );
        // White chat row at local y=169; checks are 26px boxes.
        assert_eq!(l.white_chat_label, (417, 277));
        assert_eq!(l.ingame_check.h, 26);
        assert_eq!(l.lobby_check.h, 26);
        // Timestamps local (61,219), preloading (61,267), 22px boxes.
        assert_eq!(
            (
                l.timestamps_check.x,
                l.timestamps_check.y,
                l.timestamps_check.h
            ),
            (417, 327, 22)
        );
        assert_eq!(
            (
                l.preloading_check.x,
                l.preloading_check.y,
                l.preloading_check.h
            ),
            (417, 375, 22)
        );
        // Fair crew group local (61,295,230,62) -> abs. Client top margin is
        // 4 + CaptionFont lh (25) = 29 (SetTitle-before-SetFont, see layout
        // fn), so the client is (421,432,222,29): weak/strong y = 432 +
        // (29-22)/2 = 435, slider y = 432 + (29-16)/2 = 438 — both verified
        // against ref-options.png rows.
        assert_eq!(
            (l.group.x, l.group.y, l.group.w, l.group.h),
            (417, 403, 230, 62)
        );
        assert_eq!(
            (l.weak_label.x, l.weak_label.y, l.weak_label.h),
            (422, 435, 22)
        );
        assert_eq!(l.slider.y, 438);
        assert_eq!(l.slider.h, 16);
        assert_eq!(
            l.slider.x + l.slider.w + 2 + l.strong_label.w + 1,
            l.group.x + 4 + 222,
        );
        // Buttons row local y=406, h = 22*6/5+6 = 32.
        assert_eq!((l.reset_button.y, l.reset_button.h), (514, 32));
        assert_eq!((l.advanced_button.y, l.advanced_button.h), (514, 32));
        let reset_w = (gui.caption.measure("Reset configuration", true).0 + 100).min(208);
        assert_eq!(l.reset_button.w, reset_w);
        assert_eq!(l.reset_button.x, 356 + 352 + (230 - reset_w) / 2);
    }

    fn tooltip_fixture() -> (OptionsDlgState, OptionsDlgLayout, BookFonts) {
        let gui = endeavour_font_set();
        let book = book_fonts();
        let mut state = OptionsDlgState::with_all(
            ProgramSheetState::default(),
            SoundSheetState::default(),
            GraphicsSheetState::default(),
            ControlSheetState::default(),
            NetworkSheetState::default(),
        );
        state.resize(1280, 720, &gui, &book);
        let layout = options_dlg_layout(1280, 720, &gui, &book);
        (state, layout, book)
    }

    fn assert_resource_tooltip(
        state: &OptionsDlgState,
        book: &BookFonts,
        point: GuiPoint,
        key: &'static str,
    ) {
        assert_eq!(
            state.tooltip_at(point, book),
            Some(StartupTooltip::resource(key)),
            "tooltip resource at ({}, {})",
            point.x,
            point.y,
        );
    }

    fn right_inside(rect: IntRect) -> GuiPoint {
        GuiPoint::new((rect.x + rect.w - 1) as f32, (rect.y + rect.h / 2) as f32)
    }

    #[test]
    fn program_tooltip_targets_cover_exact_labels_controls_and_parent_fallback() {
        let (mut state, layout, book) = tooltip_fixture();

        assert_eq!(
            state.tooltip_at(rect_center(layout.title_label), &book),
            Some(StartupTooltip::text("Options")),
        );
        assert_resource_tooltip(
            &state,
            &book,
            rect_center(layout.back_button),
            "IDS_DLGTIP_BACKMAIN",
        );

        let language_label = left_label_bounds(&book.book, layout.language_label, "Language:");
        let language_info = left_label_bounds(
            &book.book,
            layout.language_info,
            &state.program().language_info,
        );
        for point in [
            right_inside(language_label),
            rect_center(layout.language_combo),
            right_inside(language_info),
        ] {
            assert_resource_tooltip(&state, &book, point, "IDS_MSG_SELECTLANG");
        }

        let font_label = left_label_bounds(&book.book, layout.font_label, "Font:");
        for point in [
            right_inside(font_label),
            rect_center(layout.font_face_combo),
        ] {
            assert_resource_tooltip(&state, &book, point, "IDS_DESC_SELECTFONT");
        }
        assert_resource_tooltip(
            &state,
            &book,
            rect_center(layout.font_size_combo),
            "IDS_DESC_FONTSIZE",
        );

        let white_chat = left_label_bounds(&book.book, layout.white_chat_label, "White Chat:");
        assert_resource_tooltip(
            &state,
            &book,
            right_inside(white_chat),
            "IDS_DESC_WHITECHAT",
        );
        for (bounds, key) in [
            (layout.ingame_check, "IDS_DESC_WHITECHAT_INGAME"),
            (layout.lobby_check, "IDS_DESC_WHITECHAT_LOBBY"),
            (layout.timestamps_check, "IDS_DESC_TIMESTAMPS"),
            (layout.preloading_check, "IDS_MSG_PRELOADING_DESC"),
        ] {
            // The far caption edge is outside the action-only checkbox square
            // but remains inside the native tooltip Element.
            assert_resource_tooltip(&state, &book, right_inside(bounds), key);
        }

        for point in [
            GuiPoint::new((layout.group.x + 1) as f32, (layout.group.y + 1) as f32),
            rect_center(layout.weak_label),
            rect_center(layout.slider),
        ] {
            assert_resource_tooltip(&state, &book, point, "IDS_DESC_FAIRCREWSTRENGTH");
        }
        assert_resource_tooltip(
            &state,
            &book,
            rect_center(layout.reset_button),
            "IDS_DESC_RESETCONFIG",
        );
        assert_resource_tooltip(
            &state,
            &book,
            rect_center(layout.advanced_button),
            "IDS_DESC_ADVANCED_SETTINGS",
        );

        let active_tab = GuiPoint::new(layout.tab_icons[0].0 as f32, layout.tab_icons[0].1 as f32);
        assert_eq!(
            state.tooltip_at(active_tab, &book),
            None,
            "tabs have no C++ tip"
        );
        state.set_pointer_position(Some(rect_center(layout.language_combo)));
        assert_eq!(
            state.tooltip(&book),
            Some(StartupTooltip::resource("IDS_MSG_SELECTLANG"))
        );
    }

    #[test]
    fn graphics_and_sound_tooltip_targets_are_exhaustive_without_invented_tips() {
        let (mut state, layout, book) = tooltip_fixture();
        assert_eq!(
            state.handle_ctrl_tab(false),
            vec![OptionsDlgAction::SheetChanged(OptionsSheet::Graphics)]
        );

        for point in [
            rect_center(layout.graphics.scale_edit),
            rect_center(layout.graphics.scale_spin_increment),
            rect_center(layout.graphics.scale_spin_decrement),
            rect_center(layout.graphics.scale_slider),
        ] {
            assert_resource_tooltip(&state, &book, point, "IDS_DESC_GRAPHICSSCALE");
        }
        for id in GraphicsCheckboxId::ALL {
            assert_resource_tooltip(
                &state,
                &book,
                right_inside(layout.graphics.checkbox(id)),
                graphics_checkbox_tooltip_key(id),
            );
        }
        assert_resource_tooltip(
            &state,
            &book,
            rect_center(layout.graphics.smoke_slider),
            "IDS_MSG_PARTICLES_DESC",
        );
        assert_eq!(
            state.tooltip_at(rect_center(layout.graphics.display_mode_combo), &book),
            None,
            "C++ does not assign a display-mode tooltip",
        );
        assert_eq!(
            state.tooltip_at(rect_center(layout.graphics.apply_button), &book),
            None,
            "C++ does not assign an Apply tooltip",
        );

        assert_eq!(
            state.handle_ctrl_tab(false),
            vec![OptionsDlgAction::SheetChanged(OptionsSheet::Sound)]
        );
        for id in SoundCheckboxId::ALL {
            assert_resource_tooltip(
                &state,
                &book,
                right_inside(layout.sound.checkbox(id)),
                sound_checkbox_tooltip_key(id),
            );
        }
        for id in SoundVolumeId::ALL {
            assert_resource_tooltip(
                &state,
                &book,
                rect_center(layout.sound.slider(id)),
                sound_volume_tooltip_key(id),
            );
        }
        assert_eq!(
            state.tooltip_at(rect_center(layout.sound.volume_headings[0]), &book),
            None,
            "volume headings have no C++ tip",
        );
    }

    #[test]
    fn controls_and_network_tooltips_include_composite_parent_regions() {
        let (mut state, layout, book) = tooltip_fixture();
        for expected in [
            OptionsSheet::Graphics,
            OptionsSheet::Sound,
            OptionsSheet::Keyboard,
        ] {
            assert_eq!(
                state.handle_ctrl_tab(false),
                vec![OptionsDlgAction::SheetChanged(expected)]
            );
        }

        for set in 0..state.controls().visible_sets(ControlDevice::Keyboard) {
            assert_resource_tooltip(
                &state,
                &book,
                rect_center(layout.controls.set_buttons[set]),
                "IDS_MSG_SELECTKEYSET",
            );
        }
        for (index, bounds) in layout.controls.key_buttons.iter().copied().enumerate() {
            assert_resource_tooltip(
                &state,
                &book,
                rect_center(bounds),
                CONTROL_KEY_TOOLTIP_KEYS[index],
            );
        }
        assert_resource_tooltip(
            &state,
            &book,
            rect_center(layout.controls.reset_button),
            "IDS_MSG_RESETKEYSETS",
        );

        assert_eq!(
            state.handle_ctrl_tab(false),
            vec![OptionsDlgAction::SheetChanged(OptionsSheet::Gamepad)]
        );
        assert_resource_tooltip(
            &state,
            &book,
            rect_center(layout.controls.set_buttons[0]),
            "IDS_MSG_SELECTKEYSET",
        );
        assert_eq!(
            state.tooltip_at(rect_center(layout.controls.set_buttons[1]), &book),
            None,
            "disconnected gamepad selectors are not visible targets",
        );
        assert_resource_tooltip(
            &state,
            &book,
            right_inside(layout.controls.gamepad_gui_check),
            "IDS_DESC_GAMEPADFORMENU",
        );

        assert_eq!(
            state.handle_ctrl_tab(false),
            vec![OptionsDlgAction::SheetChanged(OptionsSheet::Network)]
        );
        for id in NetworkPortId::ALL {
            let parent = layout.network.port_controls[id.index()];
            // The group-title/frame corner is outside both interactive
            // children and proves recursive parent fallback.
            let parent_only = GuiPoint::new((parent.x + 1) as f32, (parent.y + 1) as f32);
            assert_resource_tooltip(&state, &book, parent_only, network_port_tooltip_key(id));
        }

        // AlternateServerAddress is hidden by default. The visible parent
        // checkbox still owns the whole composite tooltip region.
        assert!(!state.network().use_alternate_server);
        assert_resource_tooltip(
            &state,
            &book,
            rect_center(layout.network.alternate_edit),
            "IDS_NET_MASTERSRV_DESC",
        );
        assert_resource_tooltip(
            &state,
            &book,
            right_inside(layout.network.automatic_update_check),
            "IDS_DESC_AUTOMATICUPDATES",
        );
        assert_resource_tooltip(
            &state,
            &book,
            GuiPoint::new(
                (layout.network.upnp_check.x + layout.network.upnp_check.w - 1) as f32,
                (layout.network.upnp_check.y + 1) as f32,
            ),
            "IDS_DESC_UPNP",
        );

        let local_label =
            labeled_edit_bounds(layout.network.local_name_edit, book.book.line_height);
        let nick_label = labeled_edit_bounds(layout.network.nick_edit, book.book.line_height);
        for point in [
            GuiPoint::new((local_label.x + 1) as f32, (local_label.y + 1) as f32),
            rect_center(layout.network.local_name_edit),
        ] {
            assert_resource_tooltip(&state, &book, point, "IDS_NET_COMPUTERNAME_DESC");
        }
        for point in [
            GuiPoint::new((nick_label.x + 1) as f32, (nick_label.y + 1) as f32),
            rect_center(layout.network.nick_edit),
        ] {
            assert_resource_tooltip(&state, &book, point, "IDS_NET_USERNAME_DESC");
        }
    }

    #[test]
    fn later_tabular_occludes_small_screen_back_button_overlap() {
        let gui = endeavour_font_set();
        let book = book_fonts();
        let mut state = OptionsDlgState::with_all(
            ProgramSheetState::default(),
            SoundSheetState::default(),
            GraphicsSheetState::default(),
            ControlSheetState::default(),
            NetworkSheetState::default(),
        );
        state.resize(640, 480, &gui, &book);
        let layout = options_dlg_layout(640, 480, &gui, &book);
        let overlap = GuiPoint::new(
            layout.back_button.x.max(layout.tabular.x) as f32,
            layout.back_button.y.max(layout.tabular.y) as f32,
        );
        assert!(rect_contains(&layout.back_button, overlap));
        assert!(rect_contains(&layout.tabular, overlap));
        assert_eq!(state.tooltip_at(overlap, &book), None);

        let back_only = GuiPoint::new(
            (layout.back_button.x + layout.back_button.w / 2) as f32,
            (layout.back_button.y + layout.back_button.h - 1) as f32,
        );
        assert!(!rect_contains(&layout.tabular, back_only));
        assert_resource_tooltip(&state, &book, back_only, "IDS_DLGTIP_BACKMAIN");
    }

    fn live_sound_state(sound: SoundSheetState) -> (OptionsDlgState, OptionsDlgLayout) {
        let gui = endeavour_font_set();
        let book = book_fonts();
        let mut state = OptionsDlgState::with_sound(ProgramSheetState::default(), sound);
        state.resize(1280, 720, &gui, &book);
        assert_eq!(
            state.handle_ctrl_tab(false),
            vec![OptionsDlgAction::SheetChanged(OptionsSheet::Graphics)]
        );
        assert_eq!(
            state.handle_ctrl_tab(false),
            vec![OptionsDlgAction::SheetChanged(OptionsSheet::Sound)]
        );
        (state, options_dlg_layout(1280, 720, &gui, &book))
    }

    fn live_graphics_state() -> (OptionsDlgState, OptionsDlgLayout) {
        let gui = endeavour_font_set();
        let book = book_fonts();
        let mut state = OptionsDlgState::with_all(
            ProgramSheetState::default(),
            SoundSheetState::default(),
            GraphicsSheetState::default(),
            ControlSheetState::default(),
            NetworkSheetState::default(),
        );
        state.resize(1280, 720, &gui, &book);
        assert_eq!(
            state.handle_ctrl_tab(false),
            vec![OptionsDlgAction::SheetChanged(OptionsSheet::Graphics)]
        );
        (state, options_dlg_layout(1280, 720, &gui, &book))
    }

    fn rect_center(rect: IntRect) -> GuiPoint {
        GuiPoint::new((rect.x + rect.w / 2) as f32, (rect.y + rect.h / 2) as f32)
    }

    #[test]
    fn graphics_sliders_drag_and_held_arrows_advance_each_frame() {
        let (mut state, layout) = live_graphics_state();
        let scale = layout.graphics.scale_slider;
        let scale_travel = graphics_slider_max_scroll(scale);
        let midpoint = GuiPoint::new(
            (scale.x + 24 + scale_travel / 2) as f32,
            (scale.y + scale.h / 2) as f32,
        );
        let right = GuiPoint::new(
            (scale.x + 24 + scale_travel) as f32,
            (scale.y + scale.h / 2) as f32,
        );

        // The thumb maps linearly onto the whole spinbox domain
        // (`iNewScale = val + minScale`, C4StartupOptionsDlg.cpp:1105), so the
        // track midpoint and its right end follow the ceiling constant.
        let midpoint_percent = MIN_GRAPHICS_SCALE_PERCENT
            + (MAX_GRAPHICS_SCALE_PERCENT - MIN_GRAPHICS_SCALE_PERCENT) / 2;
        assert_eq!(
            state.handle_pointer_down(midpoint),
            vec![OptionsDlgAction::Graphics(
                GraphicsSheetAction::ScaleProposalChanged(midpoint_percent)
            )]
        );
        assert_eq!(
            state.handle_pointer_move(right),
            vec![OptionsDlgAction::Graphics(
                GraphicsSheetAction::ScaleProposalChanged(MAX_GRAPHICS_SCALE_PERCENT)
            )]
        );
        assert_eq!(
            state.handle_pointer_up(right),
            vec![OptionsDlgAction::Graphics(
                GraphicsSheetAction::ScaleProposalChanged(MAX_GRAPHICS_SCALE_PERCENT)
            )]
        );
        assert_eq!(
            state.graphics().proposed_scale_percent,
            MAX_GRAPHICS_SCALE_PERCENT
        );
        // The thumb round-trips at the raised ceiling: a full-travel drag
        // leaves the rendered position at the end of the track.
        assert_eq!(
            state.graphics_slider_position(GraphicsSliderId::Scale, scale),
            scale_travel
        );

        let smoke = layout.graphics.smoke_slider;
        let increment = GuiPoint::new(
            (smoke.x + smoke.w - 2) as f32,
            (smoke.y + smoke.h / 2) as f32,
        );
        assert_eq!(
            graphics_hit_test(&layout.graphics, increment),
            Some(GraphicsHitTarget::Slider {
                id: GraphicsSliderId::SmokeLevel,
                part: GraphicsSliderPart::IncrementArrow,
            })
        );
        assert!(state.handle_pointer_down(increment).is_empty());
        assert_eq!(
            state.pressed_graphics_arrow,
            Some((
                GraphicsSliderId::SmokeLevel,
                GraphicsSliderDirection::Increment
            ))
        );
        let before = state.graphics().smoke_level;
        let mut previous_position =
            state.graphics_slider_position(GraphicsSliderId::SmokeLevel, smoke);
        for _ in 0..4 {
            assert!(!state.advance_frame().is_empty());
            let position = state.graphics_slider_position(GraphicsSliderId::SmokeLevel, smoke);
            assert_eq!(position, previous_position + 1);
            previous_position = position;
        }
        assert!(state.graphics().smoke_level > before);
        assert!(state.handle_pointer_up(increment).is_empty());
        assert!(state.advance_frame().is_empty());
    }

    #[test]
    fn new_buttons_and_checkboxes_activate_on_release_and_cancel_on_drag_out() {
        let (mut state, layout) = live_graphics_state();
        let outside = GuiPoint::new(0.0, 0.0);
        let checkbox_rect = layout
            .graphics
            .checkbox(GraphicsCheckboxId::AddNewCrewPortraits);
        let checkbox = GuiPoint::new(
            (checkbox_rect.x + checkbox_rect.h / 2) as f32,
            (checkbox_rect.y + checkbox_rect.h / 2) as f32,
        );
        assert!(state.handle_pointer_down(checkbox).is_empty());
        assert!(state.graphics().add_new_crew_portraits);
        assert!(state.handle_pointer_move(outside).is_empty());
        assert!(state.handle_pointer_up(outside).is_empty());
        assert!(state.graphics().add_new_crew_portraits);
        assert!(state.handle_pointer_down(checkbox).is_empty());
        assert_eq!(
            state.handle_pointer_up(checkbox),
            vec![OptionsDlgAction::Graphics(
                GraphicsSheetAction::CheckboxChanged {
                    id: GraphicsCheckboxId::AddNewCrewPortraits,
                    checked: false,
                }
            )]
        );

        state.graphics_mut().set_scale_spinbox_value(150);
        let apply = rect_center(layout.graphics.apply_button);
        assert!(state.handle_pointer_down(apply).is_empty());
        assert!(state.handle_pointer_move(outside).is_empty());
        assert!(state.handle_pointer_up(outside).is_empty());
        assert!(state.handle_pointer_down(apply).is_empty());
        assert_eq!(
            state.handle_pointer_up(apply),
            vec![OptionsDlgAction::Graphics(GraphicsSheetAction::TestScale {
                old_percent: 100,
                new_percent: 150,
            })]
        );

        assert_eq!(
            state.handle_ctrl_tab(false),
            vec![OptionsDlgAction::SheetChanged(OptionsSheet::Sound)]
        );
        assert_eq!(
            state.handle_ctrl_tab(false),
            vec![OptionsDlgAction::SheetChanged(OptionsSheet::Keyboard)]
        );
        let key = rect_center(layout.controls.key_buttons[0]);
        assert!(state.handle_pointer_down(key).is_empty());
        assert!(state.handle_pointer_move(outside).is_empty());
        assert!(state.handle_pointer_up(outside).is_empty());
        assert!(state.handle_pointer_down(key).is_empty());
        assert_eq!(
            state.handle_pointer_up(key),
            vec![OptionsDlgAction::BeginControlCapture(
                ControlCaptureTarget {
                    device: ControlDevice::Keyboard,
                    set: 0,
                    control: 0,
                }
            )]
        );

        assert_eq!(
            state.handle_ctrl_tab(false),
            vec![OptionsDlgAction::SheetChanged(OptionsSheet::Gamepad)]
        );
        assert_eq!(
            state.handle_ctrl_tab(false),
            vec![OptionsDlgAction::SheetChanged(OptionsSheet::Network)]
        );
        let port_check = rect_center(layout.network.port_check(NetworkPortId::Tcp));
        assert!(state.handle_pointer_down(port_check).is_empty());
        assert!(state.network().port(NetworkPortId::Tcp).enabled);
        assert!(state.handle_pointer_move(outside).is_empty());
        assert!(state.handle_pointer_up(outside).is_empty());
        assert!(state.network().port(NetworkPortId::Tcp).enabled);
        assert!(state.handle_pointer_down(port_check).is_empty());
        assert_eq!(
            state.handle_pointer_up(port_check),
            vec![OptionsDlgAction::NetworkPortEnabledChanged {
                id: NetworkPortId::Tcp,
                enabled: false,
            }]
        );
        let port_edit = rect_center(layout.network.port_edit(NetworkPortId::Tcp));
        assert!(state.handle_pointer_down(port_edit).is_empty());
        assert!(state.handle_pointer_up(port_edit).is_empty());
    }

    #[test]
    fn gamepad_set_selection_emits_only_changed_device_actions() {
        let gui = endeavour_font_set();
        let book = book_fonts();
        let labels = || std::array::from_fn(|_| std::array::from_fn(|_| "Undefined".to_string()));
        let controls = ControlSheetState::new(labels(), labels(), 3, false);
        let mut state = OptionsDlgState::with_all(
            ProgramSheetState::default(),
            SoundSheetState::default(),
            GraphicsSheetState::default(),
            controls,
            NetworkSheetState::default(),
        );
        state.resize(1280, 720, &gui, &book);
        let layout = options_dlg_layout(1280, 720, &gui, &book);

        for expected in [
            OptionsSheet::Graphics,
            OptionsSheet::Sound,
            OptionsSheet::Keyboard,
        ] {
            assert_eq!(
                state.handle_ctrl_tab(false),
                vec![OptionsDlgAction::SheetChanged(expected)]
            );
        }
        let keyboard_two = rect_center(layout.controls.set_buttons[1]);
        assert!(state.handle_pointer_down(keyboard_two).is_empty());
        assert!(state.handle_pointer_up(keyboard_two).is_empty());

        assert_eq!(
            state.handle_ctrl_tab(false),
            vec![OptionsDlgAction::SheetChanged(OptionsSheet::Gamepad)]
        );
        for (set, expected) in [
            (2, vec![OptionsDlgAction::GamepadDeviceSelected(2)]),
            (1, vec![OptionsDlgAction::GamepadDeviceSelected(1)]),
            (1, Vec::new()),
        ] {
            let point = rect_center(layout.controls.set_buttons[set]);
            assert!(state.handle_pointer_down(point).is_empty());
            assert_eq!(state.handle_pointer_up(point), expected);
        }
        assert_eq!(state.controls().selected_set(ControlDevice::Gamepad), 1);
    }

    #[test]
    fn l079_control_set_hotkeys_follow_visible_button_order() {
        let labels = || std::array::from_fn(|_| std::array::from_fn(|_| "Undefined".to_string()));
        let controls = ControlSheetState::new(labels(), labels(), 3, false);
        let mut state = OptionsDlgState::with_all(
            ProgramSheetState::default(),
            SoundSheetState::default(),
            GraphicsSheetState::default(),
            controls,
            NetworkSheetState::default(),
        );

        assert_eq!(state.handle_hotkey('1'), None);
        state.restore_sheet(OptionsSheet::Network);
        assert_eq!(state.handle_hotkey('1'), None);
        state.restore_sheet(OptionsSheet::Keyboard);
        assert_eq!(state.handle_hotkey('4'), Some(Vec::new()));
        assert_eq!(state.controls().selected_set(ControlDevice::Keyboard), 3);
        assert_eq!(state.handle_hotkey('4'), Some(Vec::new()));
        assert_eq!(state.handle_hotkey('5'), None);

        state.restore_sheet(OptionsSheet::Gamepad);
        assert_eq!(
            state.handle_hotkey('3'),
            Some(vec![OptionsDlgAction::GamepadDeviceSelected(2)])
        );
        assert_eq!(state.controls().selected_set(ControlDevice::Gamepad), 2);
        assert_eq!(state.handle_hotkey('3'), Some(Vec::new()));
        assert_eq!(state.handle_hotkey('4'), None);
        assert_eq!(state.handle_hotkey('0'), None);
        assert_eq!(state.handle_hotkey('A'), None);
        assert_eq!(state.controls().selected_set(ControlDevice::Keyboard), 3);
    }

    #[test]
    fn sound_layout_uses_cpp_grid_math_and_caption_font_client_inset() {
        let gui = endeavour_font_set();
        let book = book_fonts();
        let layout = options_dlg_layout(1280, 720, &gui, &book);
        let sound = &layout.sound;
        let mx = layout.client.w / 40;
        let my = layout.client.h / 200;
        let (lorem_w, check_h) = book.book.measure("Lorem ipsum", true);
        let desired_w = 2 * (lorem_w + check_h + 4);
        let desired_h = check_h * 5 / 2;
        let cell_w = desired_w.min((layout.sheet.w - mx) / 2 - mx);
        let cell_h = desired_h.min((layout.sheet.h - my) / 5 - my);

        assert_eq!(
            sound.frontend_group,
            IntRect {
                x: layout.sheet.x + mx,
                y: layout.sheet.y + my,
                w: cell_w,
                h: 2 * cell_h + my,
            }
        );
        assert_eq!(
            sound.game_group,
            IntRect {
                x: layout.sheet.x + cell_w + 2 * mx,
                ..sound.frontend_group
            }
        );
        assert_eq!(
            sound.volume_group,
            IntRect {
                x: layout.sheet.x + mx,
                y: layout.sheet.y + 2 * cell_h + 3 * my,
                w: 2 * cell_w + mx,
                h: 3 * cell_h + 2 * my,
            }
        );

        // SetTitle runs before SetFont: controls are offset by CaptionFont's
        // stored client top, although the visible title uses BookFont.
        let group_client_top = sound.frontend_group.y + 4 + gui.caption.line_height;
        assert!(sound.checkbox(SoundCheckboxId::FrontendMusic).y >= group_client_top);
        assert_ne!(gui.caption.line_height, book.book.line_height);
        assert_eq!(sound.slider(SoundVolumeId::Music).h, 16);
        assert_eq!(sound.slider(SoundVolumeId::SoundEffects).h, 16);
    }

    fn assert_options_focus_cycle(state: &mut OptionsDlgState, controls: &[OptionsFocus]) {
        assert_eq!(state.focus, OptionsFocus::Tabular);
        for &expected in controls {
            assert!(state.handle_tab(false).is_empty());
            assert_eq!(state.focus, expected);
        }
        assert!(state.handle_tab(false).is_empty());
        assert_eq!(state.focus, OptionsFocus::Back);
        assert!(state.handle_tab(false).is_empty());
        assert_eq!(state.focus, OptionsFocus::Tabular);

        assert!(state.handle_tab(true).is_empty());
        assert_eq!(state.focus, OptionsFocus::Back);
        for &expected in controls.iter().rev() {
            assert!(state.handle_tab(true).is_empty());
            assert_eq!(state.focus, expected);
        }
        assert!(state.handle_tab(true).is_empty());
        assert_eq!(state.focus, OptionsFocus::Tabular);
    }

    #[test]
    fn l131_graphics_tab_and_shift_tab_cover_native_controls() {
        let mut state = OptionsDlgState::default();
        state.restore_sheet(OptionsSheet::Graphics);
        let mut controls = vec![
            OptionsFocus::Graphics(OptionsGraphicsFocusTarget::DisplayModeCombo),
            OptionsFocus::Graphics(OptionsGraphicsFocusTarget::ApplyScale),
            OptionsFocus::Graphics(OptionsGraphicsFocusTarget::ScaleEdit),
        ];
        controls.extend(
            GraphicsCheckboxId::ALL
                .into_iter()
                .map(|id| OptionsFocus::Graphics(OptionsGraphicsFocusTarget::Checkbox(id))),
        );

        assert_options_focus_cycle(&mut state, &controls);
        assert_eq!(state.focused_graphics_control(), None);
    }

    #[test]
    fn l131_keyboard_tab_and_shift_tab_cover_native_controls() {
        let mut state = OptionsDlgState::default();
        state.restore_sheet(OptionsSheet::Keyboard);
        let mut controls: Vec<_> = (0..CONTROL_SET_COUNT)
            .map(|set| OptionsFocus::Control(ControlSheetHit::Set(set)))
            .collect();
        controls.extend(
            (0..CONTROL_KEY_COUNT)
                .map(|control| OptionsFocus::Control(ControlSheetHit::Key(control))),
        );
        controls.push(OptionsFocus::Control(ControlSheetHit::Reset));

        assert_options_focus_cycle(&mut state, &controls);
        assert_eq!(state.focused_control_sheet_control(), None);
    }

    #[test]
    fn l131_gamepad_tab_and_shift_tab_follow_visible_controls() {
        let labels = || std::array::from_fn(|_| std::array::from_fn(|_| "Undefined".to_string()));
        let controls_state = ControlSheetState::new(labels(), labels(), 3, false);
        let mut state = OptionsDlgState::with_all(
            ProgramSheetState::default(),
            SoundSheetState::default(),
            GraphicsSheetState::default(),
            controls_state,
            NetworkSheetState::default(),
        );
        state.restore_sheet(OptionsSheet::Gamepad);
        let mut controls: Vec<_> = (0..3)
            .map(|set| OptionsFocus::Control(ControlSheetHit::Set(set)))
            .collect();
        controls.extend(
            (0..CONTROL_KEY_COUNT)
                .map(|control| OptionsFocus::Control(ControlSheetHit::Key(control))),
        );
        controls.push(OptionsFocus::Control(ControlSheetHit::GamepadGui));
        controls.push(OptionsFocus::Control(ControlSheetHit::Reset));

        assert_options_focus_cycle(&mut state, &controls);
    }

    #[test]
    fn l131_network_tab_and_shift_tab_follow_dynamic_visibility() {
        let network = NetworkSheetState::new(
            [11_112, 11_113, 11_111, 11_114],
            true,
            "league.openclonk.org".to_string(),
            true,
            true,
            "Local".to_string(),
            "Nick".to_string(),
            false,
        );
        let mut state = OptionsDlgState::with_all(
            ProgramSheetState::default(),
            SoundSheetState::default(),
            GraphicsSheetState::default(),
            ControlSheetState::default(),
            network,
        );
        state.restore_sheet(OptionsSheet::Network);
        let mut controls = Vec::new();
        for id in NetworkPortId::ALL {
            controls.push(OptionsFocus::Network(NetworkSheetHit::PortCheck(id)));
            controls.push(OptionsFocus::Network(NetworkSheetHit::Text(
                NetworkTextField::Port(id),
            )));
        }
        controls.extend([
            OptionsFocus::Network(NetworkSheetHit::Checkbox(
                NetworkCheckboxId::UseAlternateServer,
            )),
            OptionsFocus::Network(NetworkSheetHit::Text(
                NetworkTextField::AlternateServerAddress,
            )),
            OptionsFocus::Network(NetworkSheetHit::Checkbox(
                NetworkCheckboxId::AutomaticUpdate,
            )),
            OptionsFocus::Network(NetworkSheetHit::Checkbox(NetworkCheckboxId::EnableUpnp)),
            OptionsFocus::Network(NetworkSheetHit::Text(NetworkTextField::LocalName)),
            OptionsFocus::Network(NetworkSheetHit::Text(NetworkTextField::Nick)),
        ]);
        assert_options_focus_cycle(&mut state, &controls);

        assert!(state.handle_tab(false).is_empty());
        assert!(state.handle_tab(false).is_empty());
        assert_eq!(
            state.focused_network_control(),
            Some(NetworkSheetHit::Text(NetworkTextField::Port(
                NetworkPortId::Tcp,
            )))
        );
        state.network_mut().port_mut(NetworkPortId::Tcp).enabled = false;
        assert!(state.handle_tab(false).is_empty());
        assert_eq!(
            state.focused_network_control(),
            Some(NetworkSheetHit::PortCheck(NetworkPortId::Udp)),
            "a newly hidden edit remains the traversal anchor"
        );
    }

    #[test]
    fn sound_focus_cycle_and_raw_low_button_match_control_priority() {
        let (mut state, _) = live_sound_state(SoundSheetState::default());
        assert_eq!(state.focus, OptionsFocus::Tabular);

        for id in SoundCheckboxId::ALL {
            assert!(state.handle_tab(false).is_empty());
            assert_eq!(state.focused_sound_checkbox(), Some(id));
            assert!(state.handle_key_down(KeyCode::Enter).is_empty());
            assert!(state.handle_key_up(KeyCode::Enter).is_empty());
            assert!(state.sound().checkbox(id));
            assert_eq!(
                state.handle_gamepad_low_down(),
                vec![
                    OptionsDlgAction::Sound(SoundSheetAction::GuiSound(SoundSheetSound::ArrowHit,)),
                    OptionsDlgAction::Sound(SoundSheetAction::CheckboxChanged {
                        id,
                        checked: false,
                    }),
                ]
            );
            assert!(!state.sound().checkbox(id));
            assert!(state.handle_gamepad_low_up().is_empty());
        }
        assert!(state.handle_tab(false).is_empty());
        assert_eq!(state.focus, OptionsFocus::Back);
        assert!(state.handle_tab(false).is_empty());
        assert_eq!(state.focus, OptionsFocus::Tabular);
        assert!(state.handle_tab(true).is_empty());
        assert_eq!(state.focus, OptionsFocus::Back);
        assert!(state.handle_tab(true).is_empty());
        assert_eq!(
            state.focused_sound_checkbox(),
            Some(SoundCheckboxId::GameSoundEffects)
        );
        assert_eq!(
            state.handle_ctrl_tab(false),
            vec![OptionsDlgAction::SheetChanged(OptionsSheet::Keyboard)]
        );
        assert_eq!(state.focus, OptionsFocus::None);
        assert_eq!(
            state.handle_ctrl_tab(true),
            vec![OptionsDlgAction::SheetChanged(OptionsSheet::Sound)]
        );
        assert_eq!(state.focus, OptionsFocus::None);
    }

    #[test]
    fn sound_pointer_toggles_only_checkbox_square_without_changing_focus() {
        let (mut state, layout) = live_sound_state(SoundSheetState::default());
        let checkbox = layout.sound.checkbox(SoundCheckboxId::FrontendSoundEffects);
        let caption = GuiPoint::new(
            (checkbox.x + checkbox.h + 5) as f32,
            (checkbox.y + checkbox.h / 2) as f32,
        );
        let square = GuiPoint::new(
            (checkbox.x + checkbox.h / 2) as f32,
            (checkbox.y + checkbox.h / 2) as f32,
        );
        let inclusive_right_edge = GuiPoint::new(
            (checkbox.x + checkbox.h) as f32,
            (checkbox.y + checkbox.h / 2) as f32,
        );

        assert!(state.handle_pointer_down(caption).is_empty());
        assert!(state.handle_pointer_up(caption).is_empty());
        assert!(state.sound().frontend_sound_effects);
        assert_eq!(state.focus, OptionsFocus::Tabular);
        assert!(state.handle_pointer_down(square).is_empty());
        assert_eq!(
            state.handle_pointer_up(square),
            vec![
                OptionsDlgAction::Sound(SoundSheetAction::GuiSound(SoundSheetSound::ArrowHit,)),
                OptionsDlgAction::Sound(SoundSheetAction::CheckboxChanged {
                    id: SoundCheckboxId::FrontendSoundEffects,
                    checked: false,
                }),
            ]
        );
        assert_eq!(state.focus, OptionsFocus::Tabular);
        assert_eq!(
            state.handle_pointer_up(inclusive_right_edge),
            vec![
                OptionsDlgAction::Sound(SoundSheetAction::GuiSound(SoundSheetSound::ArrowHit,)),
                OptionsDlgAction::Sound(SoundSheetAction::CheckboxChanged {
                    id: SoundCheckboxId::FrontendSoundEffects,
                    checked: true,
                }),
            ],
            "Inside(x, 0, Hgt) includes the horizontal Hgt edge"
        );
    }

    #[test]
    fn back_pointer_ownership_blocks_sliders_and_rearms_only_over_back() {
        let (mut state, layout) = live_sound_state(SoundSheetState::default());
        let back = layout.back_button;
        let back_point = GuiPoint::new((back.x + back.w / 2) as f32, (back.y + back.h / 2) as f32);
        let slider = layout.sound.slider(SoundVolumeId::Music);
        let slider_track = GuiPoint::new(
            (slider.x + slider.w / 2) as f32,
            (slider.y + slider.h / 2) as f32,
        );

        assert!(state.handle_pointer_down(back_point).is_empty());
        assert!(state.back_pointer_owned);
        assert!(state.pressed_back);
        assert!(state.handle_pointer_move(slider_track).is_empty());
        assert!(state.back_pointer_owned);
        assert!(!state.pressed_back);
        assert_eq!(state.captured_sound_slider, None);
        assert_eq!(state.sound().music_volume, 100);
        assert!(state.handle_pointer_move(back_point).is_empty());
        assert!(
            state.pressed_back,
            "MouseEnter rearms the owned Back button"
        );
        assert_eq!(
            state.handle_pointer_up(back_point),
            vec![OptionsDlgAction::Back]
        );

        assert!(state.handle_pointer_down(back_point).is_empty());
        assert!(state.handle_pointer_move(slider_track).is_empty());
        assert!(
            state.handle_pointer_up(slider_track).is_empty(),
            "Back owns the drag, so entering a scrollbar cannot start it"
        );
        assert_eq!(state.sound().music_volume, 100);

        assert!(state.handle_pointer_down(back_point).is_empty());
        let checkbox = layout.sound.checkbox(SoundCheckboxId::FrontendMusic);
        let checkbox_square = GuiPoint::new(
            (checkbox.x + checkbox.h / 2) as f32,
            (checkbox.y + checkbox.h / 2) as f32,
        );
        assert_eq!(
            state.handle_pointer_up(checkbox_square),
            vec![
                OptionsDlgAction::Sound(SoundSheetAction::GuiSound(SoundSheetSound::ArrowHit,)),
                OptionsDlgAction::Sound(SoundSheetAction::CheckboxChanged {
                    id: SoundCheckboxId::FrontendMusic,
                    checked: false,
                }),
            ],
            "after Back drag ownership clears, the same LeftUp reaches its target"
        );
    }

    #[test]
    fn captured_slider_release_callbacks_before_checkbox_left_up() {
        let (mut state, layout) = live_sound_state(SoundSheetState::default());
        let slider = layout.sound.slider(SoundVolumeId::SoundEffects);
        let track = GuiPoint::new(
            (slider.x + slider.w / 2) as f32,
            (slider.y + slider.h / 2) as f32,
        );
        assert!(!state.handle_pointer_down(track).is_empty());
        let checkbox = layout.sound.checkbox(SoundCheckboxId::FrontendMusic);
        let release = GuiPoint::new(
            (checkbox.x + checkbox.h / 2) as f32,
            (checkbox.y + checkbox.h / 2) as f32,
        );
        let expected_pos = (release.x.floor() as i32 - slider.x - 16 - 8)
            .clamp(0, sound_slider_max_scroll(slider));
        let expected_value = (expected_pos * 100 / sound_slider_max_scroll(slider).max(1)) as u8;
        assert_eq!(
            state.handle_pointer_up(release),
            vec![
                OptionsDlgAction::Sound(SoundSheetAction::VolumeChanged {
                    id: SoundVolumeId::SoundEffects,
                    value: expected_value,
                }),
                OptionsDlgAction::Sound(SoundSheetAction::TestSound(SoundSheetSound::ArrowHit,)),
                OptionsDlgAction::Sound(SoundSheetAction::GuiSound(SoundSheetSound::ArrowHit,)),
                OptionsDlgAction::Sound(SoundSheetAction::CheckboxChanged {
                    id: SoundCheckboxId::FrontendMusic,
                    checked: false,
                }),
            ]
        );
        assert_eq!(state.captured_sound_slider, None);
        assert!(!state.sound().frontend_music);
    }

    #[test]
    fn sound_slider_track_drag_and_test_actions_are_strictly_ordered() {
        let (mut state, layout) = live_sound_state(SoundSheetState::default());
        let music = layout.sound.slider(SoundVolumeId::Music);
        let music_track = GuiPoint::new(
            (music.x + music.w / 2) as f32,
            (music.y + music.h / 2) as f32,
        );
        let max_scroll = sound_slider_max_scroll(music);
        let expected_pos = (music.w / 2 - 24).clamp(0, max_scroll);
        let expected_value = (expected_pos * 100 / max_scroll.max(1)) as u8;
        assert_eq!(
            state.handle_pointer_down(music_track),
            vec![
                OptionsDlgAction::Sound(SoundSheetAction::VolumeChanged {
                    id: SoundVolumeId::Music,
                    value: expected_value,
                }),
                OptionsDlgAction::Sound(SoundSheetAction::GuiSound(SoundSheetSound::Command,)),
            ]
        );
        // Track down captured the slider: dragging outside its rectangle still
        // invokes only the value callback, with no Command cue.
        assert_eq!(
            state.handle_pointer_move(GuiPoint::new((music.x - 100) as f32, music.y as f32)),
            vec![OptionsDlgAction::Sound(SoundSheetAction::VolumeChanged {
                id: SoundVolumeId::Music,
                value: 0,
            })]
        );
        assert_eq!(
            state.handle_pointer_up(GuiPoint::new((music.x - 100) as f32, music.y as f32)),
            vec![OptionsDlgAction::Sound(SoundSheetAction::VolumeChanged {
                id: SoundVolumeId::Music,
                value: 0,
            })],
            "captured LeftUp runs DoDragging/OnPosChanged even unchanged"
        );

        let effects = layout.sound.slider(SoundVolumeId::SoundEffects);
        let effects_track = GuiPoint::new(
            (effects.x + effects.w / 2) as f32,
            (effects.y + effects.h / 2) as f32,
        );
        let actions = state.handle_pointer_down(effects_track);
        assert!(matches!(
            actions.as_slice(),
            [
                OptionsDlgAction::Sound(SoundSheetAction::VolumeChanged {
                    id: SoundVolumeId::SoundEffects,
                    ..
                }),
                OptionsDlgAction::Sound(SoundSheetAction::TestSound(SoundSheetSound::ArrowHit)),
                OptionsDlgAction::Sound(SoundSheetAction::GuiSound(SoundSheetSound::Command)),
            ]
        ));
    }

    #[test]
    fn cross_slider_arrow_transitions_keep_mouseleave_and_bar_state_separate() {
        let (mut state, layout) =
            live_sound_state(SoundSheetState::new(true, true, true, true, 50, 50));
        let music = layout.sound.slider(SoundVolumeId::Music);
        let effects = layout.sound.slider(SoundVolumeId::SoundEffects);
        let music_arrow = GuiPoint::new((music.x + 2) as f32, (music.y + 2) as f32);
        let effects_arrow = GuiPoint::new((effects.x + 2) as f32, (effects.y + 2) as f32);
        let effects_track = GuiPoint::new(
            (effects.x + effects.w / 2) as f32,
            (effects.y + effects.h / 2) as f32,
        );

        assert_eq!(
            state.handle_pointer_down(music_arrow),
            vec![OptionsDlgAction::Sound(SoundSheetAction::GuiSound(
                SoundSheetSound::ArrowHit,
            ))]
        );
        assert_eq!(
            state.handle_pointer_move(effects_arrow),
            vec![OptionsDlgAction::Sound(SoundSheetAction::GuiSound(
                SoundSheetSound::ArrowHit,
            ))],
            "the old bar leaves silently and the different bar enters pressed"
        );
        assert!(matches!(
            state.handle_pointer_move(effects_track).as_slice(),
            [
                OptionsDlgAction::Sound(SoundSheetAction::VolumeChanged {
                    id: SoundVolumeId::SoundEffects,
                    ..
                }),
                OptionsDlgAction::Sound(SoundSheetAction::TestSound(SoundSheetSound::ArrowHit)),
                OptionsDlgAction::Sound(SoundSheetAction::GuiSound(SoundSheetSound::Command)),
                OptionsDlgAction::Sound(SoundSheetAction::GuiSound(SoundSheetSound::ArrowHit)),
            ]
        ));
        assert!(!state.handle_pointer_up(effects_track).is_empty());

        let (mut direct, _) =
            live_sound_state(SoundSheetState::new(true, true, true, true, 50, 50));
        assert!(!direct.handle_pointer_down(music_arrow).is_empty());
        assert!(matches!(
            direct.handle_pointer_move(effects_track).as_slice(),
            [
                OptionsDlgAction::Sound(SoundSheetAction::VolumeChanged {
                    id: SoundVolumeId::SoundEffects,
                    ..
                }),
                OptionsDlgAction::Sound(SoundSheetAction::TestSound(SoundSheetSound::ArrowHit)),
                OptionsDlgAction::Sound(SoundSheetAction::GuiSound(SoundSheetSound::Command)),
            ]
        ));
    }

    #[test]
    fn sound_slider_arrow_moves_on_frame_and_f3_sync_is_visual_only() {
        let sound = SoundSheetState::new(true, true, true, true, 50, 50);
        let (mut state, layout) = live_sound_state(sound);
        let effects = layout.sound.slider(SoundVolumeId::SoundEffects);
        let decrement = GuiPoint::new((effects.x + 2) as f32, (effects.y + 2) as f32);
        assert_eq!(
            state.handle_pointer_down(decrement),
            vec![OptionsDlgAction::Sound(SoundSheetAction::GuiSound(
                SoundSheetSound::ArrowHit,
            ))]
        );
        let held_actions = state.advance_frame();
        assert!(matches!(
            held_actions.as_slice(),
            [
                OptionsDlgAction::Sound(SoundSheetAction::VolumeChanged {
                    id: SoundVolumeId::SoundEffects,
                    ..
                }),
                OptionsDlgAction::Sound(SoundSheetAction::TestSound(SoundSheetSound::ArrowHit)),
            ]
        ));
        assert_eq!(
            state.handle_pointer_up(decrement),
            vec![OptionsDlgAction::Sound(SoundSheetAction::GuiSound(
                SoundSheetSound::ArrowHit,
            ))]
        );

        assert_eq!(
            state.handle_pointer_down(decrement),
            vec![OptionsDlgAction::Sound(SoundSheetAction::GuiSound(
                SoundSheetSound::ArrowHit,
            ))]
        );
        let outside = GuiPoint::new((effects.x - 10) as f32, decrement.y);
        assert!(
            state.handle_pointer_up(outside).is_empty(),
            "an outside LeftUp dispatches MouseLeave before the old scrollbar"
        );

        assert_eq!(
            state.handle_pointer_down(decrement),
            vec![OptionsDlgAction::Sound(SoundSheetAction::GuiSound(
                SoundSheetSound::ArrowHit,
            ))]
        );
        assert!(
            state.handle_pointer_move(outside).is_empty(),
            "MouseLeave clears a held arrow silently"
        );
        assert!(
            state.handle_pointer_up(outside).is_empty(),
            "LeftUp outside after MouseLeave has no release cue"
        );

        assert_eq!(
            state.handle_pointer_down(decrement),
            vec![OptionsDlgAction::Sound(SoundSheetAction::GuiSound(
                SoundSheetSound::ArrowHit,
            ))]
        );
        assert!(state.handle_pointer_move(outside).is_empty());
        let track = GuiPoint::new(
            (effects.x + effects.w / 2) as f32,
            (effects.y + effects.h / 2) as f32,
        );
        assert!(matches!(
            state.handle_pointer_move(track).as_slice(),
            [
                OptionsDlgAction::Sound(SoundSheetAction::VolumeChanged {
                    id: SoundVolumeId::SoundEffects,
                    ..
                }),
                OptionsDlgAction::Sound(SoundSheetAction::TestSound(SoundSheetSound::ArrowHit)),
                OptionsDlgAction::Sound(SoundSheetAction::GuiSound(SoundSheetSound::Command)),
            ]
        ));
        assert!(matches!(
            state.handle_pointer_up(track).as_slice(),
            [
                OptionsDlgAction::Sound(SoundSheetAction::VolumeChanged {
                    id: SoundVolumeId::SoundEffects,
                    ..
                }),
                OptionsDlgAction::Sound(SoundSheetAction::TestSound(SoundSheetSound::ArrowHit)),
            ]
        ));

        assert_eq!(
            state.handle_pointer_down(decrement),
            vec![OptionsDlgAction::Sound(SoundSheetAction::GuiSound(
                SoundSheetSound::ArrowHit,
            ))]
        );
        assert!(state.handle_pointer_move(outside).is_empty());
        assert_eq!(
            state.handle_pointer_move(decrement),
            vec![OptionsDlgAction::Sound(SoundSheetAction::GuiSound(
                SoundSheetSound::ArrowHit,
            ))],
            "an LDown pointer re-entering the arrow re-arms it"
        );
        assert_eq!(
            state.handle_pointer_up(decrement),
            vec![OptionsDlgAction::Sound(SoundSheetAction::GuiSound(
                SoundSheetSound::ArrowHit,
            ))]
        );

        let old_sound = state.sound().clone();
        state.sync_frontend_music_from_f3(false);
        assert!(!state.sound().frontend_music);
        assert_eq!(
            state.sound().frontend_sound_effects,
            old_sound.frontend_sound_effects
        );
        assert_eq!(
            state.sound().sound_effects_volume,
            old_sound.sound_effects_volume
        );
    }

    #[test]
    fn sound_checkbox_draw_focus_is_suppressed_under_an_overlay() {
        let assets = options_assets();
        let gui = endeavour_font_set();
        let book = book_fonts();
        let mut state = OptionsDlgState::default();
        state.resize(1280, 720, &gui, &book);
        assert_eq!(state.handle_ctrl_tab(false).len(), 1);
        assert_eq!(state.handle_ctrl_tab(false).len(), 1);
        assert!(state.handle_tab(false).is_empty());
        let layout = options_dlg_layout(1280, 720, &gui, &book);
        let checkbox = layout.sound.checkbox(SoundCheckboxId::FrontendMusic);
        let mut active = Surface::new(1280, 720, PixelFormat::Rgba8888);
        let mut covered = Surface::new(1280, 720, PixelFormat::Rgba8888);
        OptionsDlgScreen::render_state_with_draw_focus(
            &mut active,
            &assets,
            &gui,
            &book,
            &state,
            Some(standard_gamma()),
            true,
        );
        OptionsDlgScreen::render_state_with_draw_focus(
            &mut covered,
            &assets,
            &gui,
            &book,
            &state,
            Some(standard_gamma()),
            false,
        );
        assert!((checkbox.y..checkbox.y + checkbox.h).any(|y| {
            (checkbox.x..checkbox.x + checkbox.h).any(|x| {
                active.get_pixel(x as u32, y as u32) != covered.get_pixel(x as u32, y as u32)
            })
        }));
    }

    #[test]
    fn sound_renderer_draws_groups_checkbox_phases_and_0_50_100_pins() {
        let assets = options_assets();
        let gui = endeavour_font_set();
        let book = book_fonts();
        let gamma = standard_gamma();
        let layout = options_dlg_layout(1280, 720, &gui, &book);
        let render = |sound: SoundSheetState| {
            let mut state = OptionsDlgState::with_sound(ProgramSheetState::default(), sound);
            state.resize(1280, 720, &gui, &book);
            let _ = state.handle_ctrl_tab(false);
            let _ = state.handle_ctrl_tab(false);
            let mut surface = Surface::new(1280, 720, PixelFormat::Rgba8888);
            OptionsDlgScreen::render_state_with_draw_focus(
                &mut surface,
                &assets,
                &gui,
                &book,
                &state,
                Some(gamma),
                false,
            );
            surface
        };

        let at_zero = render(SoundSheetState::new(true, true, true, true, 0, 0));
        let at_half = render(SoundSheetState::new(true, true, true, true, 50, 50));
        let at_full = render(SoundSheetState::new(true, true, true, true, 100, 100));
        let unchecked = render(SoundSheetState::new(false, true, true, true, 50, 50));

        let frame_rgb = [0xa4_u8, 0x94, 0x7a].map(|channel| {
            encode(Some(gamma), f32::from(channel))
                .round()
                .clamp(0.0, 255.0) as u8
        });
        let group = layout.sound.frontend_group;
        assert_eq!(
            at_half.get_pixel(
                group.x as u32,
                (group.y + book.book.line_height / 2 + 3) as u32
            ),
            Some(Color::new(frame_rgb[0], frame_rgb[1], frame_rgb[2], 255)),
            "the two-pixel GroupBox frame uses C4StartupEditBorderColor"
        );

        let checkbox = layout.sound.checkbox(SoundCheckboxId::FrontendMusic);
        assert!(
            (checkbox.y..checkbox.y + checkbox.h).any(|y| {
                (checkbox.x..checkbox.x + checkbox.h).any(|x| {
                    at_half.get_pixel(x as u32, y as u32) != unchecked.get_pixel(x as u32, y as u32)
                })
            }),
            "checked and unchecked GUICheckbox phases must differ"
        );

        let pin_anchor = (0..16)
            .flat_map(|y| (0..16).map(move |x| (x, y)))
            .find(|&(x, y)| {
                let index = (((16 + y) * assets.book_scroll.width() + 16 + x) * 4) as usize;
                assets.book_scroll.pixels()[index + 3] == 255
            })
            .expect("StartupBookScroll pin has an opaque pixel");
        let source_index =
            (((16 + pin_anchor.1) * assets.book_scroll.width() + 16 + pin_anchor.0) * 4) as usize;
        let source = &assets.book_scroll.pixels()[source_index..source_index + 4];
        let expected_pin = Color::new(
            encode(Some(gamma), f32::from(source[0])).round() as u8,
            encode(Some(gamma), f32::from(source[1])).round() as u8,
            encode(Some(gamma), f32::from(source[2])).round() as u8,
            255,
        );
        let slider = layout.sound.slider(SoundVolumeId::Music);
        let max_scroll = sound_slider_max_scroll(slider);
        for (value, surface) in [(0, &at_zero), (50, &at_half), (100, &at_full)] {
            let pin_x = slider.x + 16 + value * max_scroll / 100 + pin_anchor.0 as i32;
            let pin_y = slider.y + pin_anchor.1 as i32;
            assert_eq!(
                surface.get_pixel(pin_x as u32, pin_y as u32),
                Some(expected_pin),
                "music pin anchor at volume {value}"
            );
        }
    }

    // C4GuiTabular.cpp:464-534 switches a left-hand sheet on mouse-down;
    // the caption hit bands are inclusive and advance by 72 px. The options
    // dialog starts with the tabular focused (C4StartupOptionsDlg.cpp:1039).
    #[test]
    fn live_state_switches_exact_tab_hit_bands_on_pointer_down() {
        let gui = endeavour_font_set();
        let book = book_fonts();
        let mut state = OptionsDlgState::default();
        state.resize(1280, 720, &gui, &book);
        let layout = options_dlg_layout(1280, 720, &gui, &book);

        let second_tab =
            crate::GuiPoint::new(layout.tabular.x as f32, (layout.tabular.y + 92) as f32);
        assert_eq!(
            state.handle_pointer_down(second_tab),
            vec![OptionsDlgAction::SheetChanged(OptionsSheet::Graphics)]
        );
        assert_eq!(state.active_sheet(), OptionsSheet::Graphics);
        assert!(state.handle_pointer_up(second_tab).is_empty());

        // One-pixel gap between first [20,90] and second [92,162] bands.
        let gap = crate::GuiPoint::new(layout.tabular.x as f32, (layout.tabular.y + 91) as f32);
        assert!(state.handle_pointer_down(gap).is_empty());
        assert_eq!(state.active_sheet(), OptionsSheet::Graphics);

        assert!(state.handle_tab(true).is_empty());
        assert_eq!(state.focus, OptionsFocus::Back);
        let third_tab =
            crate::GuiPoint::new(layout.tabular.x as f32, (layout.tabular.y + 164) as f32);
        assert_eq!(
            state.handle_pointer_down(third_tab),
            vec![OptionsDlgAction::SheetChanged(OptionsSheet::Sound)]
        );
        assert!(state.handle_key_down(crate::KeyCode::Enter).is_empty());
        assert_eq!(
            state.handle_key_up(crate::KeyCode::Enter),
            vec![OptionsDlgAction::Back],
            "Tabular::IsFocusOnClick is false, so a sheet click retains Back focus"
        );
    }

    // C4StartupOptionsDlg.h:38-51 and C4GuiTabular.cpp:222-239: Left and
    // Escape leave; Up/Down wrap the focused tabular. A focused GUI button
    // activates on key-up (C4GuiButton.cpp:112-128).
    #[test]
    fn live_state_routes_options_keys_and_back_button_like_cpp() {
        let gui = endeavour_font_set();
        let book = book_fonts();
        let mut state = OptionsDlgState::default();
        state.resize(1280, 720, &gui, &book);

        assert_eq!(
            state.handle_key_down(crate::KeyCode::Up),
            vec![OptionsDlgAction::SheetChanged(OptionsSheet::Network)]
        );
        assert_eq!(
            state.handle_key_down(crate::KeyCode::Down),
            vec![OptionsDlgAction::SheetChanged(OptionsSheet::Program)]
        );
        assert_eq!(
            state.handle_key_down(crate::KeyCode::Left),
            vec![OptionsDlgAction::Back]
        );
        assert_eq!(
            state.handle_key_down(crate::KeyCode::Escape),
            vec![OptionsDlgAction::Back]
        );

        assert!(state.handle_tab(true).is_empty());
        assert!(state.handle_key_down(crate::KeyCode::Enter).is_empty());
        assert_eq!(
            state.handle_key_up(crate::KeyCode::Enter),
            vec![OptionsDlgAction::Back]
        );
    }

    // Dialog::AdvanceFocus walks nested controls in construction order. The
    // fair-crew scrollbar is an Element rather than a Control, so it is not in
    // this sequence.
    #[test]
    fn program_focus_traverses_and_activates_every_native_control() {
        let mut state = OptionsDlgState::default();

        assert!(state.handle_gamepad_horizontal(false).is_empty());
        assert_eq!(
            state.focused_program_control(),
            Some(OptionsProgramFocusTarget::LanguageCombo)
        );
        assert_eq!(
            state.handle_gamepad_low_down(),
            vec![OptionsDlgAction::OpenLanguageCombo]
        );
        assert_eq!(
            state.handle_key_down(crate::KeyCode::Down),
            vec![OptionsDlgAction::OpenLanguageCombo]
        );
        assert_eq!(
            state.handle_key_down(crate::KeyCode::Space),
            vec![OptionsDlgAction::OpenLanguageCombo]
        );

        for (target, action) in [
            (
                OptionsProgramFocusTarget::FontFaceCombo,
                OptionsDlgAction::OpenFontFaceCombo,
            ),
            (
                OptionsProgramFocusTarget::FontSizeCombo,
                OptionsDlgAction::OpenFontSizeCombo,
            ),
        ] {
            assert!(state.handle_gamepad_horizontal(false).is_empty());
            assert_eq!(state.focused_program_control(), Some(target));
            assert_eq!(state.handle_gamepad_low_down(), vec![action]);
        }

        for (target, action) in [
            (
                OptionsProgramFocusTarget::WhiteChatIngame,
                OptionsDlgAction::WhiteChatIngameChanged(true),
            ),
            (
                OptionsProgramFocusTarget::WhiteChatLobby,
                OptionsDlgAction::WhiteChatLobbyChanged(true),
            ),
            (
                OptionsProgramFocusTarget::ShowLogTimestamps,
                OptionsDlgAction::ShowLogTimestampsChanged(true),
            ),
            (
                OptionsProgramFocusTarget::Preloading,
                OptionsDlgAction::PreloadingChanged(true),
            ),
        ] {
            assert!(state.handle_gamepad_horizontal(false).is_empty());
            assert_eq!(state.focused_program_control(), Some(target));
            assert_eq!(state.handle_gamepad_low_down(), vec![action]);
        }

        assert!(state.handle_gamepad_horizontal(false).is_empty());
        assert_eq!(
            state.focused_program_control(),
            Some(OptionsProgramFocusTarget::ResetButton)
        );
        assert!(state.handle_gamepad_low_down().is_empty());
        assert_eq!(
            state.handle_gamepad_low_up(),
            vec![OptionsDlgAction::ResetConfiguration]
        );

        assert!(state.handle_gamepad_horizontal(false).is_empty());
        assert_eq!(
            state.focused_program_control(),
            Some(OptionsProgramFocusTarget::AdvancedButton)
        );
        assert!(state.handle_key_down(crate::KeyCode::Enter).is_empty());
        assert_eq!(
            state.handle_key_up(crate::KeyCode::Enter),
            vec![OptionsDlgAction::OpenAdvancedSettings]
        );

        assert!(state.handle_gamepad_horizontal(false).is_empty());
        assert_eq!(state.focus, OptionsFocus::Back);
        assert!(state.handle_gamepad_horizontal(true).is_empty());
        assert_eq!(
            state.focused_program_control(),
            Some(OptionsProgramFocusTarget::AdvancedButton)
        );
        assert!(state.handle_gamepad_horizontal(true).is_empty());
        assert_eq!(
            state.focused_program_control(),
            Some(OptionsProgramFocusTarget::ResetButton)
        );
    }

    #[test]
    fn fair_crew_conversions_preserve_raw_strength_and_native_truncation() {
        assert_eq!(
            PROGRAM_FONT_FACES,
            ["Arial Unicode MS", "Comic Sans MS", "Endeavour", "Verdana",]
        );
        assert_eq!(PROGRAM_FONT_SIZES, [8, 10, 12, 14, 16, 18, 20, 24, 28]);

        assert_eq!(fair_crew_slider_to_strength(0), 0);
        assert_eq!(fair_crew_slider_to_strength(9), 922);
        assert_eq!(fair_crew_slider_to_strength(10), 1_079);
        assert_eq!(fair_crew_slider_to_strength(100), 34_151);
        assert_eq!(fair_crew_strength_to_slider(0), 0);
        assert_eq!(fair_crew_strength_to_slider(1_000), 9);
        assert_eq!(fair_crew_strength_to_slider(34_151), 99);

        let mut program = ProgramSheetState::default();
        program.set_fair_crew_strength(1_079);
        assert_eq!(program.fair_crew_strength, 1_079);
        assert_eq!(program.fair_crew_slider, 9);
        program.set_font("Verdana", 20);
        assert_eq!(program.font_face, "Verdana");
        assert_eq!(program.font_size, "20");
    }

    #[test]
    fn program_pointer_controls_emit_actions_and_fair_crew_drag_updates_raw_strength() {
        let gui = endeavour_font_set();
        let book = book_fonts();
        let mut state = OptionsDlgState::default();
        state.resize(1280, 720, &gui, &book);
        let layout = options_dlg_layout(1280, 720, &gui, &book);

        for (rect, action, anchor) in [
            (
                layout.font_face_combo,
                OptionsDlgAction::OpenFontFaceCombo,
                state.font_face_combo_anchor(),
            ),
            (
                layout.font_size_combo,
                OptionsDlgAction::OpenFontSizeCombo,
                state.font_size_combo_anchor(),
            ),
        ] {
            let point = GuiPoint::new(rect.x as f32, rect.y as f32);
            assert_eq!(state.handle_pointer_down(point), vec![action]);
            assert!(!state.pointer_down);
            assert_eq!(
                anchor,
                Some(GuiPoint::new(rect.x as f32, (rect.y + rect.h) as f32))
            );
        }

        for (rect, action) in [
            (
                layout.ingame_check,
                OptionsDlgAction::WhiteChatIngameChanged(true),
            ),
            (
                layout.lobby_check,
                OptionsDlgAction::WhiteChatLobbyChanged(true),
            ),
            (
                layout.preloading_check,
                OptionsDlgAction::PreloadingChanged(true),
            ),
        ] {
            let square = GuiPoint::new((rect.x + rect.h / 2) as f32, (rect.y + rect.h / 2) as f32);
            assert!(state.handle_pointer_down(square).is_empty());
            assert_eq!(state.handle_pointer_up(square), vec![action]);
        }

        let slider = layout.slider;
        let track = GuiPoint::new(
            (slider.x + slider.w / 2) as f32,
            (slider.y + slider.h / 2) as f32,
        );
        let max_scroll = program_slider_max_scroll(slider);
        let scroll_pos = (slider.w / 2 - 24).clamp(0, max_scroll);
        let slider_value = scroll_pos * 100 / max_scroll.max(1);
        let strength = fair_crew_slider_to_strength(slider_value);
        assert_eq!(
            state.handle_pointer_down(track),
            vec![
                OptionsDlgAction::FairCrewStrengthChanged(strength),
                OptionsDlgAction::ProgramGuiSound(GuiSound::Command),
            ]
        );
        assert!(state.captured_fair_crew_slider);
        assert_eq!(state.program().fair_crew_slider, slider_value);
        assert_eq!(state.program().fair_crew_strength, strength);
        assert_eq!(state.fair_crew_slider_position, Some(scroll_pos));

        let outside = GuiPoint::new((slider.x - 100) as f32, slider.y as f32);
        assert_eq!(
            state.handle_pointer_up(outside),
            vec![OptionsDlgAction::FairCrewStrengthChanged(0)]
        );
        assert!(!state.captured_fair_crew_slider);
        assert_eq!(state.program().fair_crew_strength, 0);

        let reset = GuiPoint::new(
            (layout.reset_button.x + layout.reset_button.w / 2) as f32,
            (layout.reset_button.y + layout.reset_button.h / 2) as f32,
        );
        assert!(state.handle_pointer_down(reset).is_empty());
        assert_eq!(
            state.handle_pointer_up(reset),
            vec![OptionsDlgAction::ResetConfiguration]
        );
        let advanced = GuiPoint::new(
            (layout.advanced_button.x + layout.advanced_button.w / 2) as f32,
            (layout.advanced_button.y + layout.advanced_button.h / 2) as f32,
        );
        assert!(state.handle_pointer_down(advanced).is_empty());
        assert_eq!(
            state.handle_pointer_up(advanced),
            vec![OptionsDlgAction::OpenAdvancedSettings]
        );
    }

    #[test]
    fn fair_crew_slider_gui_sounds_match_scrollbar_ordering() {
        let gui = endeavour_font_set();
        let book = book_fonts();
        let layout = options_dlg_layout(1280, 720, &gui, &book);
        let slider = layout.slider;
        let decrement = GuiPoint::new((slider.x + 2) as f32, (slider.y + 2) as f32);
        let track = GuiPoint::new(
            (slider.x + slider.w / 2) as f32,
            (slider.y + slider.h / 2) as f32,
        );
        let outside = GuiPoint::new((slider.x - 10) as f32, decrement.y);

        let mut state = OptionsDlgState::default();
        state.resize(1280, 720, &gui, &book);
        assert_eq!(
            state.handle_pointer_down(decrement),
            vec![OptionsDlgAction::ProgramGuiSound(GuiSound::ArrowHit)]
        );
        assert!(matches!(
            state.advance_frame().as_slice(),
            [OptionsDlgAction::FairCrewStrengthChanged(_)]
        ));
        assert_eq!(
            state.handle_pointer_up(decrement),
            vec![OptionsDlgAction::ProgramGuiSound(GuiSound::ArrowHit)]
        );

        assert_eq!(
            state.handle_pointer_down(decrement),
            vec![OptionsDlgAction::ProgramGuiSound(GuiSound::ArrowHit)]
        );
        assert!(state.handle_pointer_move(outside).is_empty());
        assert_eq!(
            state.handle_pointer_move(decrement),
            vec![OptionsDlgAction::ProgramGuiSound(GuiSound::ArrowHit)],
            "an LDown pointer re-entering the arrow re-arms it"
        );
        assert_eq!(
            state.handle_pointer_up(decrement),
            vec![OptionsDlgAction::ProgramGuiSound(GuiSound::ArrowHit)]
        );

        assert_eq!(
            state.handle_pointer_down(decrement),
            vec![OptionsDlgAction::ProgramGuiSound(GuiSound::ArrowHit)]
        );
        assert!(matches!(
            state.handle_pointer_move(track).as_slice(),
            [
                OptionsDlgAction::FairCrewStrengthChanged(_),
                OptionsDlgAction::ProgramGuiSound(GuiSound::Command),
                OptionsDlgAction::ProgramGuiSound(GuiSound::ArrowHit),
            ]
        ));
        assert!(matches!(
            state.handle_pointer_move(outside).as_slice(),
            [OptionsDlgAction::FairCrewStrengthChanged(_)]
        ));
        assert!(matches!(
            state.handle_pointer_up(outside).as_slice(),
            [OptionsDlgAction::FairCrewStrengthChanged(_)]
        ));

        let mut direct_track = OptionsDlgState::default();
        direct_track.resize(1280, 720, &gui, &book);
        assert!(matches!(
            direct_track.handle_pointer_down(track).as_slice(),
            [
                OptionsDlgAction::FairCrewStrengthChanged(_),
                OptionsDlgAction::ProgramGuiSound(GuiSound::Command),
            ]
        ));
    }

    #[test]
    fn language_catalog_projects_de_and_selection_recomposes_cpp_fallbacks() {
        let infos = vec![
            LanguageInfo {
                code_bytes: *b"US",
                code: "US".into(),
                name: "English".into(),
                info: "English info".into(),
                fallback: String::new(),
                charset: String::new(),
            },
            LanguageInfo {
                code_bytes: *b"DE",
                code: "DE".into(),
                name: "Deutsch".into(),
                info: "Deutsche Info".into(),
                fallback: "  us long, x, de".into(),
                charset: String::new(),
            },
        ];
        let mut program = ProgramSheetState::default();
        program.set_language_catalog("DE - Deutsch", "stale", infos.clone());
        assert_eq!(program.language_text, "DE - Deutsch");
        assert_eq!(program.language_info, "Deutsche Info");
        assert_eq!(program.language_ex, "DE,us,x,de,US");

        program.no_language_info = "Keine Sprachinfo".into();
        program.set_language_catalog("ZZ - Missing", "keep,this", infos.clone());
        assert_eq!(program.language_text, "unknown (ZZ - Missing)");
        assert_eq!(program.language_info, "Keine Sprachinfo");
        assert_eq!(program.language_ex, "keep,this");

        let mut dialog = OptionsDlgState::new(program);
        assert!(dialog.select_language("us"));
        assert_eq!(dialog.program().language, "US");
        assert_eq!(dialog.program().language_text, "US - English");
        assert_eq!(dialog.program().language_ex, "US,DE");
        assert!(!dialog.select_language("ZZ"));
    }

    #[test]
    fn language_combo_pointer_down_opens_at_the_program_control() {
        let gui = endeavour_font_set();
        let book = book_fonts();
        let mut state = OptionsDlgState::default();
        state.resize(1280, 720, &gui, &book);
        let combo = options_dlg_layout(1280, 720, &gui, &book).language_combo;
        let point = crate::GuiPoint::new(combo.x as f32, combo.y as f32);

        assert_eq!(
            state.handle_pointer_down(point),
            vec![OptionsDlgAction::OpenLanguageCombo]
        );
        assert!(!state.pointer_down);
        assert_eq!(
            state.language_combo_anchor(),
            Some(crate::GuiPoint::new(
                combo.x as f32,
                (combo.y + combo.h) as f32,
            ))
        );
    }

    // CallbackButton only fires if a left-down is released over the same
    // control (C4GuiButton.cpp:130-154). The exact C4Rect edge is half-open.
    #[test]
    fn live_state_back_hit_test_requires_matching_press_and_release() {
        let gui = endeavour_font_set();
        let book = book_fonts();
        let mut state = OptionsDlgState::default();
        state.resize(1280, 720, &gui, &book);
        let back = options_dlg_layout(1280, 720, &gui, &book).back_button;
        let inside = crate::GuiPoint::new(back.x as f32, back.y as f32);
        let outside = crate::GuiPoint::new((back.x + back.w) as f32, back.y as f32);

        assert!(state.handle_pointer_down(inside).is_empty());
        assert!(state.handle_pointer_up(outside).is_empty());
        assert_eq!(
            state.handle_key_down(crate::KeyCode::Down),
            vec![OptionsDlgAction::SheetChanged(OptionsSheet::Graphics)],
            "Button::IsFocusOnClick is false, so a cancelled Back click retains Tabular focus"
        );
        assert!(state.handle_pointer_down(inside).is_empty());
        assert_eq!(
            state.handle_pointer_up(inside),
            vec![OptionsDlgAction::Back]
        );
    }

    // The Program sheet binds this checkbox directly to
    // Config.General.ShowLogTimestamps (C4StartupOptionsDlg.cpp:749-753).
    // C4GUI::CheckBox toggles only on left-button-up over its square, not its
    // caption (C4GuiCheckBox.cpp:82-96), then BoolConfig updates the bound value
    // (C4StartupOptionsDlg.cpp:558-568).
    #[test]
    fn live_state_toggles_log_timestamps_only_over_checkbox_square() {
        let gui = endeavour_font_set();
        let book = book_fonts();
        let mut state = OptionsDlgState::default();
        state.resize(1280, 720, &gui, &book);
        let checkbox = options_dlg_layout(1280, 720, &gui, &book).timestamps_check;
        let square = crate::GuiPoint::new(
            (checkbox.x + checkbox.h / 2) as f32,
            (checkbox.y + checkbox.h / 2) as f32,
        );
        let caption = crate::GuiPoint::new(
            (checkbox.x + checkbox.h + 5) as f32,
            (checkbox.y + checkbox.h / 2) as f32,
        );

        assert!(!state.program().show_log_timestamps);
        assert!(state.handle_pointer_down(square).is_empty());
        assert_eq!(
            state.handle_pointer_up(square),
            vec![OptionsDlgAction::ShowLogTimestampsChanged(true)]
        );
        assert!(state.program().show_log_timestamps);

        assert!(state.handle_pointer_up(caption).is_empty());
        assert!(state.program().show_log_timestamps);
        assert_eq!(
            state.handle_pointer_up(square),
            vec![OptionsDlgAction::ShowLogTimestampsChanged(false)]
        );
        assert!(!state.program().show_log_timestamps);
    }

    // DrawLineDw is GL_LINES between pixel centers (StdGL.cpp:893-934): by
    // the diamond-exit rule the END pixel is not rasterized, in either
    // direction. Blending is (255-A)/255 of the inverted-alpha color.
    #[test]
    fn engine_line_drops_the_diamond_exit_end_pixel() {
        let mut sfc = Surface::new(8, 4, PixelFormat::Rgba8888);
        draw_line_dw(&mut sfc, 1, 1, 5, 1, 0x00ff0000, None);
        let red = |sfc: &Surface, x: u32, y: u32| sfc.get_pixel(x, y).unwrap().r;
        assert_eq!(red(&sfc, 0, 1), 0);
        assert_eq!(red(&sfc, 1, 1), 255); // start pixel drawn
        assert_eq!(red(&sfc, 4, 1), 255);
        assert_eq!(red(&sfc, 5, 1), 0); // end pixel dropped
        assert_eq!(red(&sfc, 1, 0), 0);
        assert_eq!(red(&sfc, 1, 2), 0);
        // Reversed (right-to-left): drops the LEFT end.
        draw_line_dw(&mut sfc, 5, 2, 1, 2, 0x00ff0000, None);
        assert_eq!(red(&sfc, 5, 2), 255);
        assert_eq!(red(&sfc, 2, 2), 255);
        assert_eq!(red(&sfc, 1, 2), 0);
        // 50% transparent (inverted alpha 0x7f): 255 * 128/255 = 128.
        draw_line_dw(&mut sfc, 0, 3, 2, 3, 0x7fff0000, None);
        assert_eq!(red(&sfc, 0, 3), 128);
        assert_eq!(red(&sfc, 2, 3), 0);
    }

    // DrawQuadDw rasterization (StdGL.cpp:879-889, blitOffset = 0): GL
    // triangle-strip with integer vertices — fragments whose centers lie
    // inside the polygon; centers exactly on a left (entering) edge are in,
    // on a right (exiting) edge are out. The SmallButton top bevel
    // (0,0),(12,0),(9,3),(0,3) therefore lights 11/10/9 pixels per row: the
    // diagonal crosses row 0 at x=11.5, exactly on pixel 11's center.
    #[test]
    fn engine_quad_fills_beveled_trapezoid() {
        let mut sfc = Surface::new(12, 6, PixelFormat::Rgba8888);
        fill_quad_dw(
            &mut sfc,
            &[(0, 0), (12, 0), (12 - 3, 3), (0, 3)],
            0x00ffffff,
            None,
        );
        let lit = |y: u32| {
            (0..12u32)
                .filter(|&x| sfc.get_pixel(x, y).unwrap().r > 0)
                .count()
        };
        assert_eq!(lit(0), 11); // right edge excludes the on-edge center
        assert_eq!(lit(1), 10); // slant eats one pixel per row
        assert_eq!(lit(2), 9);
        assert_eq!(lit(3), 0); // bottom edge exclusive

        // The adjoining right quad (12,0),(12,6),(9,6),(9,3) owns the seam:
        // its diagonal is a LEFT edge, so the on-edge center (11,0) is in.
        let mut sfc2 = Surface::new(12, 6, PixelFormat::Rgba8888);
        fill_quad_dw(
            &mut sfc2,
            &[(12, 0), (12, 6), (9, 6), (9, 3)],
            0x00ffffff,
            None,
        );
        assert_eq!(sfc2.get_pixel(11, 0).unwrap().r, 255);
        assert_eq!(sfc2.get_pixel(10, 0).unwrap().r, 0);
    }

    #[test]
    fn advanced_box_and_quad_shift_but_native_lines_do_not() {
        let _renderer = crate::activate_advanced_renderer_config(crate::AdvancedRendererConfig {
            blit_offset: 100,
            no_box_fades: true,
            ..crate::AdvancedRendererConfig::DEFAULT
        });
        let expected = crate::normalize_quad_colors([Color::opaque(64, 0, 0); 4]);

        let mut box_surface = Surface::new(8, 8, PixelFormat::Rgba8888);
        fill_box_dw(&mut box_surface, 0, 0, 0, 0, 0x0040_0000, None);
        assert_eq!(box_surface.get_pixel(1, 1), Some(expected));
        assert_eq!(box_surface.get_pixel(0, 0).unwrap().r, 0);

        let mut quad_surface = Surface::new(8, 8, PixelFormat::Rgba8888);
        fill_quad_dw(
            &mut quad_surface,
            &[(3, 0), (5, 0), (5, 2), (3, 2)],
            0x0040_0000,
            None,
        );
        assert_eq!(quad_surface.get_pixel(4, 1), Some(expected));
        assert_eq!(quad_surface.get_pixel(3, 0).unwrap().r, 0);

        let mut line_surface = Surface::new(8, 8, PixelFormat::Rgba8888);
        draw_line_dw(&mut line_surface, 1, 6, 3, 6, 0x0040_0000, None);
        assert_eq!(line_surface.get_pixel(1, 6).unwrap().r, 64);
        assert_eq!(line_surface.get_pixel(2, 6).unwrap().r, 64);
        assert_eq!(line_surface.get_pixel(2, 7).unwrap().r, 0);
    }

    // DrawHBarByVGfx rotation (C4Gui.cpp:347-361): dest (dx,dy) samples texel
    // (15-dy, dx) of the vertical facet.
    #[test]
    fn rotated_vfacet_transposes_texels() {
        // 16x16 image: texel (x,y) has r = x, g = y.
        let pixels = (0..16u32)
            .flat_map(|y| (0..16u32).flat_map(move |x| [x as u8, y as u8, 0, 255]))
            .collect();
        let image = ImageData::new(16, 16, pixels);
        let mut sfc = Surface::new(16, 16, PixelFormat::Rgba8888);
        draw_rotated_vfacet(&mut sfc, &image, 0, 0, 16, 0, 0, None);
        let px = |x: u32, y: u32| sfc.get_pixel(x, y).unwrap();
        assert_eq!((px(0, 0).r, px(0, 0).g), (15, 0)); // dest(0,0) <- texel(15,0)
        assert_eq!((px(0, 15).r, px(0, 15).g), (0, 0)); // dest(0,15) <- texel(0,0)
        assert_eq!((px(5, 3).r, px(5, 3).g), (12, 5)); // dest(5,3) <- texel(12,5)

        // BlitOffset is submitted before the -90 degree transform. A (+1,+1)
        // native offset therefore rotates to (+1,-1) around the fixed pivot.
        let _renderer = crate::activate_advanced_renderer_config(crate::AdvancedRendererConfig {
            blit_offset: 100,
            ..crate::AdvancedRendererConfig::DEFAULT
        });
        let mut shifted = Surface::new(20, 20, PixelFormat::Rgba8888);
        draw_rotated_vfacet(&mut shifted, &image, 0, 0, 16, 2, 2, None);
        assert_eq!(shifted.get_pixel(3, 1), Some(Color::new(15, 0, 0, 255)));
        assert_eq!(shifted.get_pixel(2, 2).unwrap().a, 0);
    }

    /// Renders the reference frame and dumps it for external diffing against
    /// build/Screenshots/ref-options.png (the CI box has no reference, so no
    /// assertion here).
    #[test]
    fn render_program_tab_reference_artifact() {
        let assets = options_assets();
        let gui = endeavour_font_set();
        let book = book_fonts();
        let state = OptionsDlgState::default();
        let mut surface = Surface::new(1280, 720, PixelFormat::Rgba8888);
        OptionsDlgScreen::render_state(
            &mut surface,
            &assets,
            &gui,
            &book,
            &state,
            Some(standard_gamma()),
        );
        // Final whole-surface gamma pass (mirrors render_startup_frame).
        standard_gamma().apply_to_surface(&mut surface);
        std::fs::create_dir_all("/tmp/menu-parity-options").expect("mkdir");
        write_ppm(&surface, "/tmp/menu-parity-options/out.ppm");
    }
}
