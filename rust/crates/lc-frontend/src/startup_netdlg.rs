//! Pixel-parity renderer for the C++ `C4StartupNetDlg` startup dialog
//! ("Start Network Game"), mirroring the engine's first-shown state
//! (see `rust/target/parity-specs/net.md`). Implemented against the
//! engine's F9 reference capture at 1280x720; owned by its
//! implementation agent.
//!
//! Geometry mirrors the C++ constructor math (C4StartupNetDlg.cpp:631-728)
//! with the fullscreen-dialog margins of C4GuiDialogs.cpp:813-822 and the
//! ComponentAligner of C4Gui.cpp:975-1057.

use std::time::Instant;

use crate::classic_gui::{
    blacken_transparent_pixels, draw_3d_frame, draw_clipped_text, draw_engine_box,
    draw_facet_stretch, ClassicButtonState, ClassicGuiSkin,
};
use crate::clonk_fonts::{expand_hotkey_markup, ClonkFontSet};
use crate::message_dialog::break_message;
use crate::startup_main_menu::{IntRect, StartupTooltip};
use crate::{GuiPoint, ImageData, KeyCode};
use lc_graphics::clonk_font::{ClonkFont, TextAlign};
use lc_graphics::{GammaRamp, Surface};
use lc_gui::Rect as GuiRect;

// Engine colors (C4Gui.h:52-103,163-165). Font colors are NORMAL-alpha RGBA
// (0xff = opaque); box/line colors are engine AARRGGBB with INVERTED alpha
// (0x00 = opaque).
/// C4GUI_FullscreenCaptionFontClr / C4GUI_Caption2FontClr / C4GUI_ButtonFontClr.
const CLR_YELLOW: [u8; 4] = [0xff, 0xff, 0x00, 0xff];
/// C4GUI_CaptionFontClr / C4GUI_MessageFontClr.
const CLR_WHITE: [u8; 4] = [0xff, 0xff, 0xff, 0xff];
const CLR_DISABLED: [u8; 4] = [0xaf, 0xaf, 0xaf, 0xff];
const CLR_HYPERLINK: [u8; 4] = [0x80, 0x80, 0xff, 0xff];
/// ListBox background / C4GUI_EditBGColor.
const CLR_DARK_BG: u32 = 0x7f00_0000;
const CLR_EDIT_SELECTION: u32 = 0x7f7f_7f00;
const CLR_IMPORTANT_BG: u32 = 0xcf00_007f;
const CLR_SELECTION: u32 = 0xafaf_0000;
const SCROLLBAR_WIDTH: i32 = 16;
const SCROLLBAR_PART: i32 = 16;
const JOIN_EDIT_MAX_PAYLOAD: usize = 254;
const EDIT_SCROLL_OFFSET: i32 = 2;
const MAX_INFO_ICONS: usize = 10;
const SCENARIO_ICON_COUNT: i32 = 52;
const DEFAULT_SCENARIO_ICON_PHASE: i32 = 14;
const EXPANDED_ROW_TOP_SPACING: i32 = 10;
const COLLAPSED_ROW_TOP_SPACING: i32 = 5;

/// The `Graphics.c4g` images `C4StartupNetDlg` draws (C4Startup.cpp:48,82-83;
/// C4Gui.cpp:1087-1097).
pub struct NetDlgAssets {
    /// `StartupNetworkBG.png` (800x600): fullscreen background.
    pub background: ImageData,
    /// `StartupNetGetRef.png` (2000x32): 50-phase animated query icon.
    pub net_get_ref: ImageData,
    /// `StartupScenSelIcons.png` (1248x24): 52 scenario-icon phases.
    pub scen_icons: ImageData,
    /// `GUICaption.png` (192x23): wooden caption bar, 3-slice border 32.
    pub gui_caption: ImageData,
    /// `GUIButton.png` (128x32): bottom button planks, 3-slice border 32.
    pub gui_button: ImageData,
    /// `GUIButtonDown.png`: pressed bottom-button plank.
    pub gui_button_down: ImageData,
    /// `GUIButtonHighlight.png`: additive focus/hover/pressed overlay.
    pub gui_button_highlight: ImageData,
    /// `GUIScroll.png` (32x48): classic vertical scrollbar facets.
    pub gui_scroll: ImageData,
    /// `GUIIcons.png` (240x360): standard 40x40 icon grid, 6 columns.
    pub gui_icons: ImageData,
    /// `GUIIcons2.png` (256x320): extended 64x64 icon grid, 4 columns.
    pub gui_icons_ex: ImageData,
}

/// Font-derived inputs of the layout (the C++ reads them from the live GUI
/// fonts; C4StartupNetDlg.cpp:636,686, C4GuiLabels.cpp:211-215).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct NetDlgFontMetrics {
    /// CaptionFont extent of `"<< BACK"` with markup (C4StartupNetDlg.cpp:636).
    pub caption_back_extent: i32,
    /// TextFont extent of `"IP:"` with markup (C4StartupNetDlg.cpp:685-686).
    pub text_ip_extent: i32,
    /// TextFont line height (22 for Endeavour 14px).
    pub text_line_height: i32,
    /// CaptionFont line height (25 for Endeavour 16px).
    pub caption_line_height: i32,
    /// TitleFont line height (34 for Endeavour 22px).
    pub title_line_height: i32,
}

impl NetDlgFontMetrics {
    /// Measures the metrics from the live GUI font set.
    pub fn from_fonts(fonts: &ClonkFontSet) -> Self {
        Self {
            caption_back_extent: fonts.caption.measure("<< BACK", true).0,
            text_ip_extent: fonts.text.measure("IP:", true).0,
            text_line_height: fonts.text.line_height,
            caption_line_height: fonts.caption.line_height,
            title_line_height: fonts.title.line_height,
        }
    }
}

/// Pixel-exact `C4StartupNetDlg` geometry, all in C++ integer math and
/// absolute screen coordinates.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct NetDlgLayout {
    /// Fullscreen-dialog client rect (C4GuiDialogs.cpp:813-822,858-862).
    pub client: IntRect,
    /// Centered anchor of the big-font title (C4GuiDialogs.cpp:843-845).
    pub title_anchor: (i32, i32),
    /// Left icon button "Games" (C4StartupNetDlg.cpp:650-653).
    pub btn_game_list: IntRect,
    /// Left icon button "Chat" (C4StartupNetDlg.cpp:654-657).
    pub btn_chat: IntRect,
    /// Right icon button "Internet" (C4StartupNetDlg.cpp:710-713).
    pub btn_internet: IntRect,
    /// Right icon button "Record" (C4StartupNetDlg.cpp:714-717).
    pub btn_record: IntRect,
    /// "Running Games" wooden caption (C4StartupNetDlg.cpp:670-672).
    pub game_list_caption: IntRect,
    /// Game list box bounds (C4StartupNetDlg.cpp:673-678).
    pub game_list: IntRect,
    /// List box client area, margins 3 (C4GuiListBox.h:120-123).
    pub list_client: IntRect,
    /// ScrollWindow viewport. The 16px scrollbar remains reserved even while
    /// its auto-hide mode makes it invisible (C4GuiContainers.cpp:477-491).
    pub list_viewport: IntRect,
    /// Fixed-width vertical scrollbar beside [`Self::list_viewport`].
    pub list_scrollbar: IntRect,
    /// The masterserver query entry (width minus scroll bar reserve;
    /// C4GuiContainers.cpp:477-491, C4StartupNetDlg.cpp:39-81).
    pub list_entry: IntRect,
    /// Aspect-fit destination of the animated NetGetRef icon
    /// (C4Facet.cpp:100-127: 40x32 fit into 48x48 -> 48x38, centered).
    pub entry_icon: IntRect,
    /// The two entry info labels (C4StartupNetDlg.cpp:64-76,372-430).
    pub entry_labels: [IntRect; 2],
    /// "IP:" wooden label (C4StartupNetDlg.cpp:679-688).
    pub ip_label: IntRect,
    /// Join-address edit field (C4StartupNetDlg.cpp:689-690).
    pub join_edit: IntRect,
    /// Bottom buttons: Back, Reload, Join game, New game
    /// (C4StartupNetDlg.cpp:719-728).
    pub buttons: [IntRect; 4],
}

/// `C4GUI::ComponentAligner` (C4Gui.cpp:975-1057, C4Gui.h:1868-1912): doles
/// out sub-rects of a client area with per-component margins.
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
        self.area.w // GetWidth (C4Gui.h:1901)
    }

    fn height(&self) -> i32 {
        self.area.h // GetHeight (C4Gui.h:1902)
    }

    /// GetFromTop with optional horizontal centering (C4Gui.cpp:975-990).
    fn get_from_top(&mut self, hgt: i32, wdt: Option<i32>) -> IntRect {
        let full_w = self.area.w - self.mx * 2;
        let out = IntRect {
            x: self.area.x + self.mx + wdt.map_or(0, |w| (full_w - w) / 2),
            y: self.area.y + self.my,
            w: wdt.unwrap_or(full_w),
            h: hgt,
        };
        let d = hgt + self.my * 2;
        self.area.y += d;
        self.area.h -= d;
        out
    }

    /// GetFromLeft, full height (C4Gui.cpp:992-1008).
    fn get_from_left(&mut self, wdt: i32) -> IntRect {
        let out = IntRect {
            x: self.area.x + self.mx,
            y: self.area.y + self.my,
            w: wdt,
            h: self.area.h - self.my * 2,
        };
        let d = wdt + self.mx * 2;
        self.area.x += d;
        self.area.w -= d;
        out
    }

    /// GetFromRight, full height (C4Gui.cpp:1010-1024).
    fn get_from_right(&mut self, wdt: i32) -> IntRect {
        let out = IntRect {
            x: self.area.x + self.area.w - wdt - self.mx,
            y: self.area.y + self.my,
            w: wdt,
            h: self.area.h - self.my * 2,
        };
        self.area.w -= wdt + self.mx * 2;
        out
    }

    /// GetFromBottom, full width (C4Gui.cpp:1026-1041).
    fn get_from_bottom(&mut self, hgt: i32) -> IntRect {
        let out = IntRect {
            x: self.area.x + self.mx,
            y: self.area.y + self.area.h - hgt - self.my,
            w: self.area.w - self.mx * 2,
            h: hgt,
        };
        self.area.h -= hgt + self.my * 2;
        out
    }

    /// GetAll (C4Gui.cpp:1043-1049).
    fn all(&self) -> IntRect {
        IntRect {
            x: self.area.x + self.mx,
            y: self.area.y + self.my,
            w: self.area.w - self.mx * 2,
            h: self.area.h - self.my * 2,
        }
    }

    /// GetCentered (C4Gui.cpp:1051-1060; GetMiddleX/Y are x + Wdt/2).
    fn centered(&self, wdt: i32, hgt: i32) -> IntRect {
        IntRect {
            x: self.area.x + self.area.w / 2 - wdt / 2,
            y: self.area.y + self.area.h / 2 - hgt / 2,
            w: wdt,
            h: hgt,
        }
    }
}

/// Offsets `rect` by `(dx, dy)`.
fn offset(rect: IntRect, dx: i32, dy: i32) -> IntRect {
    IntRect {
        x: rect.x + dx,
        y: rect.y + dy,
        ..rect
    }
}

/// Computes the `C4StartupNetDlg` layout for a `w`x`h` screen, mirroring
/// C4StartupNetDlg.cpp:631-728 in C++ integer math.
pub fn net_dlg_layout(w: i32, h: i32, metrics: &NetDlgFontMetrics) -> NetDlgLayout {
    // Fullscreen dialog margins (C4GuiDialogs.cpp:813-822): X = w/50, Y =
    // h*2/75 (2 below 500/320 px); the top margin adds the 50px title strip
    // (C4GUI_FullscreenDlg_TitleHeight = C4UpperBoardHeight, C4Gui.h:163).
    let margin_x = if w < 500 { 2 } else { w / 50 };
    let margin_y = if h < 320 { 2 } else { h * 2 / 75 };
    let margin_top = 50 + margin_y;
    let client = IntRect {
        x: margin_x,
        y: margin_top,
        w: w - 2 * margin_x,
        h: h - margin_top - margin_y,
    };
    let at_client = |rect: IntRect| offset(rect, client.x, client.y);

    // Constructor constants (C4StartupNetDlg.cpp:633-637).
    let icon_size = 64; // C4GUI_IconExWdt
    let side_size = (w / 6).max(icon_size);
    let button_hgt = 32; // C4GUI_ButtonHgt
    let button_indent = w / 40;
    let button_wdt = metrics.caption_back_extent * 3;

    // Aligner stacking (C4StartupNetDlg.cpp:638-645); caMain is zero-based
    // over the client rect (fZeroAreaXY = true).
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
    let ca_button_area_rect = ca_main.get_from_bottom(ca_main.height() / 7);
    let ca_button_area = Aligner::new(ca_button_area_rect, 0, 0);
    let button_area_wdt = ca_button_area.width() * 7 / 8;
    let button_wdt = button_wdt.min((button_area_wdt - 8 * button_indent) / 4);
    let button_indent = (button_area_wdt - 4 * button_wdt) / 8;
    let mut ca_buttons = Aligner::new(
        ca_button_area.centered(button_area_wdt, button_hgt),
        button_indent,
        0,
    );

    // Left/right icon areas. The mutating GetFromLeft/Right is evaluated
    // before GetWidth (clang arg order); per the spec the centered result is
    // invariant under gcc's order too (C4StartupNetDlg.cpp:644-645).
    let left_area = ca_main.get_from_left(side_size);
    let left_mx = (ca_main.width() / 20).min((side_size - icon_size) / 2);
    let left_my = ca_main.height() / 40;
    let mut ca_left = Aligner::new(left_area, left_mx, left_my);
    let btn_game_list = at_client(ca_left.get_from_top(icon_size, Some(icon_size)));
    let btn_chat = at_client(ca_left.get_from_top(icon_size, Some(icon_size)));

    let right_area = ca_main.get_from_right(side_size);
    let right_mx = (ca_main.width() / 20).min((side_size - icon_size) / 2);
    let right_my = ca_main.height() / 40;
    let mut ca_config = Aligner::new(right_area, right_mx, right_my);
    let btn_internet = at_client(ca_config.get_from_top(icon_size, Some(icon_size)));
    let btn_record = at_client(ca_config.get_from_top(icon_size, Some(icon_size)));

    // Tabular sheet content (zero chrome; C4StartupNetDlg.cpp:663-690).
    let tabular = ca_main.all();
    let at_sheet = |rect: IntRect| at_client(offset(rect, tabular.x, tabular.y));
    let mut ca_game_list = Aligner::new(
        IntRect {
            x: 0,
            y: 0,
            w: tabular.w,
            h: tabular.h,
        },
        0,
        0,
    );
    // iCaptHgt = max(TextFont line height, C4GUI_MinWoodBarHgt = 23)
    // (C4GuiLabels.cpp:211-215).
    let capt_hgt = metrics.text_line_height.max(23);
    let game_list_caption = at_sheet(ca_game_list.get_from_top(capt_hgt, None));
    let game_list = at_sheet(ca_game_list.get_from_top(ca_game_list.height() - capt_hgt, None));
    let mut ca_ip = Aligner::new(ca_game_list.all(), 0, 0);
    let ip_label = at_sheet(ca_ip.get_from_left(metrics.text_ip_extent + 10));
    let join_edit = at_sheet(ca_ip.all());

    // List box internals: margins 3 (C4GuiListBox.h:120-123); the scroll
    // window reserves C4GUI_ScrollBarWdt = 16 (C4GuiContainers.cpp:477-491).
    let list_client = IntRect {
        x: game_list.x + 3,
        y: game_list.y + 3,
        w: game_list.w - 6,
        h: game_list.h - 6,
    };
    // Entry: iHeight = 2*22 + 4 = 48 after label restack
    // (C4StartupNetDlg.cpp:42-44,372-388).
    let entry_h = metrics.text_line_height * 2 + 4;
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
    let list_entry = IntRect {
        x: list_viewport.x,
        y: list_viewport.y,
        w: list_viewport.w,
        h: entry_h,
    };
    // Aspect-fit of the 40x32 query-icon facet into the 48x48 icon bounds
    // (C4Facet.cpp:100-127): Hgt = 32*48/40, centered vertically.
    let icon_fit_h = 32 * entry_h / 40;
    let entry_icon = IntRect {
        x: list_entry.x,
        y: list_entry.y + (entry_h - icon_fit_h) / 2,
        w: entry_h,
        h: icon_fit_h,
    };
    // Labels at x = 48+3, y = 1/25, width = entryW - 51 - 1
    // (C4StartupNetDlg.cpp:64-76, UpdateText C4StartupNetDlg.cpp:400-410).
    let label_x = list_entry.x + entry_h + 3;
    let label_w = list_entry.w - (entry_h + 3) - 1;
    let entry_labels = [
        IntRect {
            x: label_x,
            y: list_entry.y + 1,
            w: label_w,
            h: metrics.text_line_height,
        },
        IntRect {
            x: label_x,
            y: list_entry.y + 1 + metrics.text_line_height + 2,
            w: label_w,
            h: metrics.text_line_height,
        },
    ];

    // Bottom buttons (C4StartupNetDlg.cpp:719-728).
    let buttons = [(); 4].map(|()| at_client(ca_buttons.get_from_left(button_wdt)));

    NetDlgLayout {
        client,
        // Centered big-font title (C4GuiDialogs.cpp:843-845): y = 50/2 -
        // titleLH/2 - marginTop, relative to the client origin.
        title_anchor: (
            client.x + client.w / 2,
            client.y + 25 - metrics.title_line_height / 2 - margin_top,
        ),
        btn_game_list,
        btn_chat,
        btn_internet,
        btn_record,
        game_list_caption,
        game_list,
        list_client,
        list_viewport,
        list_scrollbar,
        list_entry,
        entry_icon,
        entry_labels,
        ip_label,
        join_edit,
        buttons,
    }
}

/// Extracts one 40px-wide animation phase of `StartupNetGetRef.png` as its
/// own image (C4Facet::GetPhase; facet Wdt = 40, C4Startup.cpp:82-83).
fn net_get_ref_phase(image: &ImageData, phase: u32) -> ImageData {
    let pw = 40u32.min(image.width());
    let ph = image.height();
    let phases = (image.width() / pw.max(1)).max(1);
    let sx = (phase % phases) * pw;
    let pixels = (0..ph)
        .flat_map(|y| {
            let start = ((y * image.width() + sx) * 4) as usize;
            image.pixels()[start..start + (pw * 4) as usize].to_vec()
        })
        .collect();
    ImageData::new(pw, ph, pixels)
}

/// Converts an [`IntRect`] to the float rect the stretch blitters take.
fn gui_rect(rect: IntRect) -> GuiRect {
    GuiRect::new(rect.x as f32, rect.y as f32, rect.w as f32, rect.h as f32)
}

fn edit_client(rect: IntRect) -> IntRect {
    IntRect {
        x: rect.x + 4,
        y: rect.y + 2,
        w: (rect.w - 8).max(0),
        h: (rect.h - 4).max(0),
    }
}

/// Config-driven state of the two right icon buttons
/// (C4StartupNetDlg.cpp:710-717).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct NetDlgConfig {
    /// `Config.Network.MasterServerSignUp` (default true, C4Config.cpp:545):
    /// picks Ico_Ex_InternetOn (Ex+7) over Ico_Ex_InternetOff (Ex+6).
    pub masterserver_signup: bool,
    /// `Config.General.Record` (default false, C4Config.cpp:382): picks
    /// Ico_Ex_RecordOn (Ex+1) over Ico_Ex_RecordOff (Ex+0).
    pub record: bool,
}

impl Default for NetDlgConfig {
    fn default() -> Self {
        Self {
            masterserver_signup: true,
            record: false,
        }
    }
}

/// Top-right reference-state icons in native insertion order
/// (`C4StartupNetListEntry::SetReference`, C4StartupNetDlg.cpp:467-490).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum NetDlgStatusIcon {
    PasswordNeeded,
    League,
    LobbyActive,
    Running,
    RuntimeJoin,
    FairCrew,
    OfficialServer,
}

impl NetDlgStatusIcon {
    /// `(extended_sheet, phase)` from `C4GUI::Icons`.
    const fn source(self) -> (bool, u32) {
        match self {
            Self::PasswordNeeded => (true, 13),
            Self::League => (true, 8),
            Self::LobbyActive => (false, 31),
            Self::Running => (false, 30),
            Self::RuntimeJoin => (false, 32),
            Self::FairCrew => (true, 2),
            Self::OfficialServer => (false, 44),
        }
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum NetDlgRowIcon {
    #[default]
    None,
    /// Animated `StartupNetGetRef` reference query.
    Query,
    /// Static base frame of `StartupNetGetRef` for a completed query.
    QueryStatic,
    /// Standard `Ico_Close` (GUIIcons phase 34).
    Error,
    /// Raw `C4Network2Reference::Icon` phase from `StartupScenSelIcons`.
    Scenario(i32),
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum NetDlgTextLine {
    Plain(String),
    Hyperlink { label: String, url: String },
}

impl NetDlgTextLine {
    fn text(&self) -> &str {
        match self {
            Self::Plain(text) | Self::Hyperlink { label: text, .. } => text,
        }
    }

    fn hyperlink(&self) -> Option<&str> {
        match self {
            Self::Plain(_) => None,
            Self::Hyperlink { url, .. } => Some(url),
        }
    }
}

/// Stateful masterserver query row. Response metadata extends the two compact
/// query lines without being flattened into the discovered-game list.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct NetDlgMasterserverEntry {
    pub title: String,
    pub details: String,
    pub extra_lines: Vec<NetDlgTextLine>,
    pub row_icon: NetDlgRowIcon,
}

impl Default for NetDlgMasterserverEntry {
    fn default() -> Self {
        Self {
            title: "Internet server on league.clonkspot.org".to_string(),
            details: "Querying game infos...".to_string(),
            extra_lines: Vec::new(),
            row_icon: NetDlgRowIcon::Query,
        }
    }
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct NetDlgGameEntry {
    pub title: String,
    pub details: String,
    /// Version, comment and player-name labels for resolved references.
    pub extra_lines: Vec<String>,
    /// At most the first ten icons are displayed, matching the native slots.
    pub status_icons: Vec<NetDlgStatusIcon>,
    pub row_icon: NetDlgRowIcon,
    pub address: Option<String>,
    pub joinable: bool,
}

/// The two sheets of `C4StartupNetDlg` (C4StartupNetDlg.h:133,
/// C4StartupNetDlg.cpp:814-836).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum NetDlgMode {
    GameList,
    Chat,
}

/// Focusable controls and callback buttons in C++ traversal order.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum NetDlgControl {
    GameList,
    JoinAddress,
    ChatInput,
    GamesButton,
    ChatButton,
    Internet,
    Record,
    Back,
    Refresh,
    JoinGame,
    CreateGame,
}

/// Classic GUI sounds produced by the net-dialog scrollbar.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum NetDlgSound {
    ArrowHit,
    Command,
}

/// Cursor operations registered by `C4GUI::Edit` for every modifier state.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum NetDlgEditKey {
    Left,
    Right,
    Home,
    End,
    Backspace,
    Delete,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct NetDlgEditModifiers {
    pub shift: bool,
    pub control: bool,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum NetDlgEditClipboardShortcut {
    Copy,
    Cut,
    Paste,
    SelectAll,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum NetDlgEditContextCommand {
    Cut,
    Copy,
    Paste,
    Clear,
    SelectAll,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct NetDlgEditContextItem {
    pub command: NetDlgEditContextCommand,
    pub label: String,
    pub tooltip: String,
}

#[derive(Clone, Debug, PartialEq)]
pub struct NetDlgEditContextRequest {
    pub anchor: GuiPoint,
    pub items: Vec<NetDlgEditContextItem>,
}

/// Requests produced by [`NetDlgController`]. The controller mutates only
/// presentation-local state; the application owns network/config side effects.
#[derive(Clone, Debug, PartialEq)]
pub enum NetDlgAction {
    FocusChanged(NetDlgControl),
    ModeChanged(NetDlgMode),
    Back,
    Refresh,
    QueryAddress {
        address: String,
    },
    JoinGame {
        address: Option<String>,
    },
    CreateGame,
    MasterserverSignupChanged(bool),
    RecordingChanged(bool),
    JoinAddressChanged(String),
    OpenJoinAddressContextMenu(NetDlgEditContextRequest),
    /// The host writes `text` to the native clipboard. For a cut, it calls
    /// [`NetDlgController::confirm_clipboard_cut`] only after that write
    /// succeeds, so a failed clipboard operation never destroys user text.
    ClipboardTransfer {
        text: String,
        cut: bool,
    },
    OpenUrl(String),
    GuiSound(NetDlgSound),
}

/// Capture metadata for edit-only routes that otherwise fall through to the
/// startup dialog or application key bindings.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct NetDlgEditInputOutcome {
    pub captured: bool,
    pub actions: Vec<NetDlgAction>,
}

impl NetDlgEditInputOutcome {
    fn passed() -> Self {
        Self::default()
    }

    fn captured(actions: Vec<NetDlgAction>) -> Self {
        Self {
            captured: true,
            actions,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct PendingClipboardCut {
    range: (usize, usize),
    text: String,
}

#[derive(Clone, Debug)]
struct NetDlgEditState {
    text: String,
    caret: usize,
    selection: Option<(usize, usize)>,
    horizontal_scroll: i32,
    drag_anchor: Option<usize>,
    last_input: Instant,
    last_reported_cursor_visible: bool,
    pending_cut: Option<PendingClipboardCut>,
}

impl Default for NetDlgEditState {
    fn default() -> Self {
        Self {
            text: String::new(),
            caret: 0,
            selection: None,
            horizontal_scroll: 0,
            drag_anchor: None,
            last_input: Instant::now(),
            last_reported_cursor_visible: false,
            pending_cut: None,
        }
    }
}

impl NetDlgEditState {
    fn set_text(&mut self, text: &str) {
        self.text = truncate_utf8(text, JOIN_EDIT_MAX_PAYLOAD).to_string();
        self.caret = self.text.len();
        self.selection = None;
        self.horizontal_scroll = 0;
        self.drag_anchor = None;
        self.pending_cut = None;
        self.last_input = Instant::now();
    }

    fn selected_range(&self) -> Option<(usize, usize)> {
        let (anchor, caret) = self.selection?;
        (anchor != caret).then_some((anchor.min(caret), anchor.max(caret)))
    }

    fn selected_text(&self) -> Option<&str> {
        let (start, end) = self.selected_range()?;
        self.text.get(start..end)
    }

    fn focus(&mut self) {
        self.caret = self.text.len();
        self.selection = (!self.text.is_empty()).then_some((0, self.text.len()));
        self.drag_anchor = None;
        self.pending_cut = None;
        self.last_input = Instant::now();
    }

    fn blur(&mut self) {
        self.selection = None;
        self.drag_anchor = None;
        self.pending_cut = None;
    }

    fn delete_selection(&mut self) -> bool {
        let Some((start, end)) = self.selected_range() else {
            return false;
        };
        self.text.replace_range(start..end, "");
        self.caret = start;
        self.selection = None;
        self.pending_cut = None;
        self.last_input = Instant::now();
        true
    }

    /// `Edit::InsertText`: selection deletion happens before the capacity
    /// check, and this path performs no CharIn/Paste transformations.
    fn insert_raw_text(&mut self, text: &str) -> bool {
        let old_text = self.text.clone();
        self.delete_selection();
        let available = JOIN_EDIT_MAX_PAYLOAD.saturating_sub(self.text.len());
        let insert = truncate_utf8(text, available);
        if !insert.is_empty() {
            self.text.insert_str(self.caret, insert);
            self.caret += insert.len();
            self.last_input = Instant::now();
        }
        self.selection = None;
        self.pending_cut = None;
        self.text != old_text
    }

    fn handle_key(
        &mut self,
        key: NetDlgEditKey,
        modifiers: NetDlgEditModifiers,
        rect: IntRect,
        font: &ClonkFont,
    ) -> bool {
        let old_text = self.text.clone();
        self.pending_cut = None;

        if matches!(key, NetDlgEditKey::Backspace | NetDlgEditKey::Delete)
            && self.delete_selection()
        {
            self.ensure_cursor_in_view(rect, font);
            return self.text != old_text;
        }
        if self.selected_range().is_some() && !modifiers.shift {
            self.selection = None;
        }

        match key {
            NetDlgEditKey::Backspace | NetDlgEditKey::Delete if modifiers.shift => {}
            NetDlgEditKey::Backspace => {
                let start = if modifiers.control {
                    self.word_boundary(-1)
                } else {
                    previous_boundary(&self.text, self.caret)
                };
                if start < self.caret {
                    self.text.replace_range(start..self.caret, "");
                    self.caret = start;
                }
                self.selection = None;
            }
            NetDlgEditKey::Delete => {
                let end = if modifiers.control {
                    self.word_boundary(1)
                } else {
                    next_boundary(&self.text, self.caret)
                };
                if end > self.caret {
                    self.text.replace_range(self.caret..end, "");
                }
                self.selection = None;
            }
            NetDlgEditKey::Left
            | NetDlgEditKey::Right
            | NetDlgEditKey::Home
            | NetDlgEditKey::End => {
                let old_caret = self.caret;
                let destination = match key {
                    NetDlgEditKey::Left if modifiers.control => self.word_boundary(-1),
                    NetDlgEditKey::Right if modifiers.control => self.word_boundary(1),
                    NetDlgEditKey::Left => previous_boundary(&self.text, self.caret),
                    NetDlgEditKey::Right => next_boundary(&self.text, self.caret),
                    NetDlgEditKey::Home => 0,
                    NetDlgEditKey::End => self.text.len(),
                    _ => self.caret,
                };
                self.caret = destination;
                if modifiers.shift {
                    let anchor = self.selection.map_or(old_caret, |(anchor, _)| anchor);
                    self.selection = (anchor != destination).then_some((anchor, destination));
                } else {
                    self.selection = None;
                }
            }
        }

        self.last_input = Instant::now();
        self.ensure_cursor_in_view(rect, font);
        self.text != old_text
    }

    fn word_boundary(&self, direction: i8) -> usize {
        let mut position = self.caret;
        let mut nonspace_found = false;
        let mut space_found = false;
        loop {
            let next = if direction < 0 {
                if position == 0 {
                    break;
                }
                previous_boundary(&self.text, position)
            } else {
                if position >= self.text.len() {
                    break;
                }
                next_boundary(&self.text, position)
            };
            let sample = if direction < 0 { next } else { position };
            if is_word_spacer(char_at(&self.text, sample)) {
                if nonspace_found && direction < 0 {
                    break;
                }
                space_found = true;
            } else {
                if space_found && direction > 0 {
                    break;
                }
                nonspace_found = true;
            }
            position = next;
        }
        position
    }

    fn ensure_cursor_in_view(&mut self, rect: IntRect, font: &ClonkFont) {
        let client_width = edit_client(rect).w;
        if client_width < 5 {
            return;
        }
        let mut width = font.measure(&self.text[..self.caret], false).0;
        width += font.measure("\u{a6}", false).0 / 2;
        if width < self.horizontal_scroll && self.horizontal_scroll > 0 {
            self.horizontal_scroll = (width - EDIT_SCROLL_OFFSET).max(0);
        }
        if width > self.horizontal_scroll && width > client_width + self.horizontal_scroll {
            self.horizontal_scroll = width - client_width
                + if self.caret < self.text.len() {
                    EDIT_SCROLL_OFFSET
                } else {
                    0
                };
        }
    }

    fn character_at(&self, pointer_x: f32, rect: IntRect, font: &ClonkFont) -> usize {
        let control_x = pointer_x.floor() as i32 - edit_client(rect).x + self.horizontal_scroll;
        let mut previous_width = 0;
        for (index, character) in self.text.char_indices() {
            let end = index + character.len_utf8();
            let width = font.measure(&self.text[..end], false).0;
            // C4GuiEdit.cpp:207-213: a midpoint tie belongs to the character
            // on the left, i.e. the caret before this character.
            if width - (width - previous_width) / 2 >= control_x {
                return index;
            }
            previous_width = width;
        }
        self.text.len()
    }

    fn begin_pointer_selection(&mut self, position: usize, rect: IntRect, font: &ClonkFont) {
        self.pending_cut = None;
        self.caret = position.min(self.text.len());
        self.selection = Some((self.caret, self.caret));
        self.drag_anchor = Some(self.caret);
        self.ensure_cursor_in_view(rect, font);
    }

    fn drag_pointer_selection(&mut self, position: usize, rect: IntRect, font: &ClonkFont) {
        let Some(anchor) = self.drag_anchor else {
            return;
        };
        self.pending_cut = None;
        self.caret = position.min(self.text.len());
        self.selection = Some((anchor, self.caret));
        self.ensure_cursor_in_view(rect, font);
    }

    fn select_word_at(&mut self, mut position: usize, rect: IntRect, font: &ClonkFont) {
        position = position.min(self.text.len());
        if is_word_spacer(char_at(&self.text, position)) {
            if position == 0 {
                self.drag_anchor = None;
                return;
            }
            position = previous_boundary(&self.text, position);
            if is_word_spacer(char_at(&self.text, position)) {
                self.drag_anchor = None;
                return;
            }
        }
        let mut start = position;
        while start > 0 {
            let previous = previous_boundary(&self.text, start);
            if is_word_spacer(char_at(&self.text, previous)) {
                break;
            }
            start = previous;
        }
        let mut end = next_boundary(&self.text, position);
        while end < self.text.len() && !is_word_spacer(char_at(&self.text, end)) {
            end = next_boundary(&self.text, end);
        }
        self.caret = end;
        self.selection = Some((start, end));
        self.drag_anchor = None;
        self.pending_cut = None;
        self.ensure_cursor_in_view(rect, font);
    }

    fn cursor_visible(&self) -> bool {
        Instant::now()
            .checked_duration_since(self.last_input)
            .unwrap_or_default()
            .as_millis()
            / 500
            % 2
            == 0
    }
}

/// Live input state for the pixel-parity network dialog.
///
/// Pointer hits use the same half-open integer rectangles as `C4Rect::Contains`
/// (C4Rect.h:40-43). Buttons retain C++'s press-on-down, invoke-on-up model
/// (C4GuiButton.cpp:112-155), including keyboard activation.
pub struct NetDlgController {
    metrics: NetDlgFontMetrics,
    text_font: Option<ClonkFont>,
    width: i32,
    height: i32,
    config: NetDlgConfig,
    mode: NetDlgMode,
    join_edit: NetDlgEditState,
    focus: NetDlgControl,
    pointer_position: Option<GuiPoint>,
    hovered: Option<NetDlgControl>,
    pointer_pressed: Option<NetDlgControl>,
    key_pressed: Option<(NetDlgControl, KeyCode)>,
    masterserver: NetDlgMasterserverEntry,
    games: Vec<NetDlgGameEntry>,
    selection: Option<NetDlgSelection>,
    list_scroll_y: i32,
    list_scroll_pin: i32,
    scrollbar_dragging: bool,
    scrollbar_arrow_captured: bool,
    scrollbar_arrow: i8,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum NetDlgSelection {
    Masterserver,
    Game(usize),
}

#[derive(Clone, Debug)]
struct NetDlgLineLayout {
    rect: IntRect,
    text: String,
    hyperlink: Option<String>,
}

#[derive(Clone, Debug)]
struct NetDlgRowLayout {
    selection: NetDlgSelection,
    rect: IntRect,
    lines: Vec<NetDlgLineLayout>,
    status_icons: Vec<NetDlgStatusIcon>,
    row_icon: NetDlgRowIcon,
    joinable: bool,
}

impl NetDlgController {
    pub fn new(config: NetDlgConfig, metrics: NetDlgFontMetrics) -> Self {
        Self {
            metrics,
            text_font: None,
            width: 1,
            height: 1,
            config,
            mode: NetDlgMode::GameList,
            join_edit: NetDlgEditState::default(),
            // C4StartupNetDlg.cpp:734 / GetDlgModeFocusControl: game list.
            focus: NetDlgControl::GameList,
            pointer_position: None,
            hovered: None,
            pointer_pressed: None,
            key_pressed: None,
            masterserver: NetDlgMasterserverEntry::default(),
            games: Vec::new(),
            selection: None,
            list_scroll_y: 0,
            list_scroll_pin: 0,
            scrollbar_dragging: false,
            scrollbar_arrow_captured: false,
            scrollbar_arrow: 0,
        }
    }

    pub fn resize(&mut self, width: i32, height: i32) {
        self.width = width.max(1);
        self.height = height.max(1);
        self.hovered = self
            .pointer_position
            .and_then(|point| self.hit_button(point));
        self.clamp_list_scroll();
    }

    /// Supplies the live text font used by `CStdFont::BreakMessage` row
    /// wrapping. The constructor retains its metrics-only fallback so tests
    /// and headless callers that cannot load GUI fonts remain usable.
    pub fn set_text_font(&mut self, font: &ClonkFont) {
        self.text_font = Some(font.clone());
        self.clamp_list_scroll();
    }

    pub const fn config(&self) -> NetDlgConfig {
        self.config
    }

    /// Mirrors the config-facing part of `C4StartupNetDlg::OnShown` calling
    /// `UpdateMasterserver`: refresh the Internet icon and masterserver query
    /// row presence from `Config.Network.MasterServerSignUp` without
    /// reconstructing the dialog.
    ///
    /// C++ `IconButton::SetIcon` only replaces the icon facet, so even an
    /// in-progress Internet-button press, along with all other interaction and
    /// presentation state, remains untouched. Record is deliberately not
    /// synchronized here because `UpdateMasterserver` does not read it.
    pub fn sync_masterserver_signup_from_config(&mut self, masterserver_signup: bool) {
        self.config.masterserver_signup = masterserver_signup;
        if !masterserver_signup && self.selection == Some(NetDlgSelection::Masterserver) {
            self.selection = None;
        }
        self.clamp_list_scroll();
    }

    pub const fn mode(&self) -> NetDlgMode {
        self.mode
    }

    pub const fn focused_control(&self) -> NetDlgControl {
        self.focus
    }

    pub fn pointer_position(&self) -> Option<GuiPoint> {
        self.pointer_position
    }

    /// Returns the C++ `SetToolTip` target at `point`. Timing and keyboard
    /// suppression belong to the screen-wide classic mouse tracker.
    pub fn tooltip_at(&self, point: GuiPoint) -> Option<StartupTooltip> {
        let layout = self.layout();
        let controls = [
            (
                layout.btn_game_list,
                "IDS_DESC_SHOWSAVAILABLENETWORKGAME",
                true,
            ),
            (layout.btn_chat, "IDS_DESC_CONNECTSTOANIRCCHATSERVER", true),
            (
                layout.btn_internet,
                "IDS_DLGTIP_SEARCHINTERNETGAME",
                self.mode == NetDlgMode::GameList,
            ),
            (
                layout.btn_record,
                "IDS_DLGTIP_RECORD",
                self.mode == NetDlgMode::GameList,
            ),
            (layout.buttons[0], "IDS_DLGTIP_BACKMAIN", true),
            (
                layout.buttons[1],
                "IDS_NET_RELOAD_DESC",
                self.mode == NetDlgMode::GameList,
            ),
            (
                layout.buttons[2],
                "IDS_NET_JOINGAME_DESC",
                self.mode == NetDlgMode::GameList,
            ),
            (layout.buttons[3], "IDS_NET_NEWGAME_DESC", true),
        ];
        if let Some((_, key, _)) = controls
            .into_iter()
            .find(|(rect, _, visible)| *visible && contains(*rect, point))
        {
            return Some(StartupTooltip::resource(key));
        }
        (self.mode == NetDlgMode::GameList
            && (contains(layout.ip_label, point) || contains(layout.join_edit, point)))
        .then(|| StartupTooltip::resource("IDS_NET_IP_DESC"))
    }

    pub fn tooltip(&self) -> Option<StartupTooltip> {
        self.tooltip_at(self.pointer_position?)
    }

    pub fn set_pointer_position(&mut self, position: Option<GuiPoint>) {
        self.pointer_position = position;
        self.hovered = position.and_then(|point| self.hit_button(point));
        if position.is_none() {
            self.pointer_pressed = None;
            self.join_edit.drag_anchor = None;
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

    pub fn join_address(&self) -> &str {
        &self.join_edit.text
    }

    pub const fn join_address_caret(&self) -> usize {
        self.join_edit.caret
    }

    pub fn join_address_selection(&self) -> Option<(usize, usize)> {
        self.join_edit.selected_range()
    }

    pub const fn join_address_horizontal_scroll(&self) -> i32 {
        self.join_edit.horizontal_scroll
    }

    pub fn join_address_contains(&self, point: GuiPoint) -> bool {
        self.mode == NetDlgMode::GameList && contains(self.layout().join_edit, point)
    }

    pub fn join_address_cursor_visible(&self) -> bool {
        self.mode == NetDlgMode::GameList
            && self.focus == NetDlgControl::JoinAddress
            && self.join_edit.cursor_visible()
    }

    /// Returns true only when the effective 500ms C4GUI cursor phase changed.
    pub fn tick_join_address_cursor(&mut self) -> bool {
        let visible = self.join_address_cursor_visible();
        let changed = visible != self.join_edit.last_reported_cursor_visible;
        self.join_edit.last_reported_cursor_visible = visible;
        changed
    }

    pub fn set_join_address(&mut self, address: impl Into<String>) {
        self.join_edit.set_text(&address.into());
    }

    pub fn set_games(&mut self, games: Vec<NetDlgGameEntry>) {
        self.games = games;
        if self
            .selected_game()
            .is_some_and(|index| index >= self.games.len())
        {
            self.selection = None;
        }
        self.clamp_list_scroll();
    }

    pub fn set_masterserver_entry(&mut self, entry: NetDlgMasterserverEntry) {
        self.masterserver = entry;
        self.clamp_list_scroll();
    }

    pub fn masterserver_entry(&self) -> &NetDlgMasterserverEntry {
        &self.masterserver
    }

    pub fn games(&self) -> &[NetDlgGameEntry] {
        &self.games
    }

    pub fn selected_game(&self) -> Option<usize> {
        match self.selection {
            Some(NetDlgSelection::Game(index)) => Some(index),
            Some(NetDlgSelection::Masterserver) | None => None,
        }
    }

    /// Selects one discovered game and transfers keyboard focus to the list.
    /// The application uses this after a direct reference query materializes
    /// its row, mirroring `AddReferenceQuery` followed by `SetFocus`.
    pub fn focus_game(&mut self, index: usize) -> Vec<NetDlgAction> {
        if index >= self.games.len() {
            return Vec::new();
        }
        self.selection = Some(NetDlgSelection::Game(index));
        let layout = self.layout();
        self.ensure_selection_visible(&layout);
        self.change_focus(NetDlgControl::GameList)
    }

    /// Returns the discovered-game index under a visible list point. The
    /// masterserver query row and blank space are deliberately not games.
    pub fn game_index_at(&self, position: GuiPoint) -> Option<usize> {
        match self.list_selection_at(position)? {
            NetDlgSelection::Game(index) => Some(index),
            NetDlgSelection::Masterserver => None,
        }
    }

    /// Current vertical ScrollWindow displacement in logical pixels.
    pub const fn list_scroll_offset(&self) -> i32 {
        self.list_scroll_y
    }

    /// Maximum displacement for the current rows and viewport.
    pub fn list_max_scroll(&self) -> i32 {
        self.max_list_scroll(&self.layout())
    }

    pub fn list_is_collapsed(&self) -> bool {
        self.list_is_collapsed_with_font(&self.layout(), self.text_font.as_ref())
    }

    /// Adds text received from the windowing layer while the IP edit owns
    /// focus. `KeyCode` intentionally contains navigation keys only, so text
    /// input is a separate operation just like C4GUI::Edit::CharIn.
    pub fn handle_text_input(&mut self, text: &str, font: &ClonkFont) -> Vec<NetDlgAction> {
        if self.focus != NetDlgControl::JoinAddress || self.mode != NetDlgMode::GameList {
            return Vec::new();
        }
        let filtered: String = text
            .chars()
            .filter(|character| !character.is_control() && *character != '\u{7f}')
            .map(|character| {
                if character == '|' {
                    '\u{a6}'
                } else {
                    character
                }
            })
            .collect();
        if filtered.is_empty() {
            return Vec::new();
        }
        let changed = self.join_edit.insert_raw_text(&filtered);
        self.join_edit
            .ensure_cursor_in_view(self.layout().join_edit, font);
        changed
            .then(|| NetDlgAction::JoinAddressChanged(self.join_edit.text.clone()))
            .into_iter()
            .collect()
    }

    pub fn handle_edit_key_down(
        &mut self,
        key: NetDlgEditKey,
        modifiers: NetDlgEditModifiers,
        font: &ClonkFont,
    ) -> NetDlgEditInputOutcome {
        if self.focus != NetDlgControl::JoinAddress || self.mode != NetDlgMode::GameList {
            return NetDlgEditInputOutcome::passed();
        }
        let changed = self
            .join_edit
            .handle_key(key, modifiers, self.layout().join_edit, font);
        let actions = changed
            .then(|| NetDlgAction::JoinAddressChanged(self.join_edit.text.clone()))
            .into_iter()
            .collect();
        NetDlgEditInputOutcome::captured(actions)
    }

    /// Compatibility convenience for callers that route Backspace separately.
    pub fn delete_join_address_char(&mut self, font: &ClonkFont) -> Vec<NetDlgAction> {
        self.handle_edit_key_down(
            NetDlgEditKey::Backspace,
            NetDlgEditModifiers::default(),
            font,
        )
        .actions
    }

    pub fn handle_clipboard_shortcut(
        &mut self,
        shortcut: NetDlgEditClipboardShortcut,
        clipboard_text: Option<&str>,
        font: &ClonkFont,
    ) -> NetDlgEditInputOutcome {
        if self.focus != NetDlgControl::JoinAddress || self.mode != NetDlgMode::GameList {
            return NetDlgEditInputOutcome::passed();
        }
        let command = match shortcut {
            NetDlgEditClipboardShortcut::Copy => NetDlgEditContextCommand::Copy,
            NetDlgEditClipboardShortcut::Cut => NetDlgEditContextCommand::Cut,
            NetDlgEditClipboardShortcut::Paste => NetDlgEditContextCommand::Paste,
            NetDlgEditClipboardShortcut::SelectAll => NetDlgEditContextCommand::SelectAll,
        };
        NetDlgEditInputOutcome::captured(self.apply_context_command(command, clipboard_text, font))
    }

    pub fn apply_context_command(
        &mut self,
        command: NetDlgEditContextCommand,
        clipboard_text: Option<&str>,
        font: &ClonkFont,
    ) -> Vec<NetDlgAction> {
        if self.mode != NetDlgMode::GameList {
            return Vec::new();
        }
        match command {
            NetDlgEditContextCommand::Copy => self.begin_clipboard_transfer(false),
            NetDlgEditContextCommand::Cut => self.begin_clipboard_transfer(true),
            NetDlgEditContextCommand::Paste => clipboard_text
                .filter(|text| !text.is_empty())
                .map(|text| self.paste_join_address(text, font))
                .unwrap_or_default(),
            NetDlgEditContextCommand::Clear => {
                self.join_edit.pending_cut = None;
                if !self.join_edit.delete_selection() {
                    return Vec::new();
                }
                self.join_edit
                    .ensure_cursor_in_view(self.layout().join_edit, font);
                vec![NetDlgAction::JoinAddressChanged(
                    self.join_edit.text.clone(),
                )]
            }
            NetDlgEditContextCommand::SelectAll => {
                self.join_edit.pending_cut = None;
                self.join_edit.caret = self.join_edit.text.len();
                self.join_edit.selection =
                    (!self.join_edit.text.is_empty()).then_some((0, self.join_edit.text.len()));
                Vec::new()
            }
        }
    }

    /// Completes a pending cut only after the host successfully wrote the
    /// matching selection to the native clipboard.
    pub fn confirm_clipboard_cut(&mut self, font: &ClonkFont) -> Vec<NetDlgAction> {
        let Some(pending) = self.join_edit.pending_cut.take() else {
            return Vec::new();
        };
        if self.join_edit.selected_range() != Some(pending.range)
            || self.join_edit.text.get(pending.range.0..pending.range.1)
                != Some(pending.text.as_str())
        {
            return Vec::new();
        }
        if !self.join_edit.delete_selection() {
            return Vec::new();
        }
        self.join_edit
            .ensure_cursor_in_view(self.layout().join_edit, font);
        vec![NetDlgAction::JoinAddressChanged(
            self.join_edit.text.clone(),
        )]
    }

    pub fn request_context_menu_at(
        &mut self,
        point: GuiPoint,
        clipboard_has_text: bool,
    ) -> NetDlgEditInputOutcome {
        self.pointer_position = Some(point);
        self.hovered = self.hit_button(point);
        let layout = self.layout();
        if self.mode != NetDlgMode::GameList || !contains(layout.join_edit, point) {
            return NetDlgEditInputOutcome::passed();
        }
        NetDlgEditInputOutcome::captured(vec![NetDlgAction::OpenJoinAddressContextMenu(
            self.join_address_context_request(point, clipboard_has_text),
        )])
    }

    pub fn request_context_menu_from_key(
        &mut self,
        clipboard_has_text: bool,
    ) -> NetDlgEditInputOutcome {
        if self.mode != NetDlgMode::GameList || self.focus != NetDlgControl::JoinAddress {
            return NetDlgEditInputOutcome::passed();
        }
        let edit = self.layout().join_edit;
        let anchor = GuiPoint::new((edit.x + edit.w / 2) as f32, (edit.y + edit.h / 2) as f32);
        NetDlgEditInputOutcome::captured(vec![NetDlgAction::OpenJoinAddressContextMenu(
            self.join_address_context_request(anchor, clipboard_has_text),
        )])
    }

    /// Non-Windows C4GUI inserts the primary selection through InsertText,
    /// not Paste: no `|` mapping and no line-break callbacks occur here.
    pub fn handle_pointer_middle_down(
        &mut self,
        point: GuiPoint,
        primary_selection: Option<&str>,
        font: &ClonkFont,
    ) -> NetDlgEditInputOutcome {
        self.pointer_position = Some(point);
        self.hovered = self.hit_button(point);
        let edit = self.layout().join_edit;
        if self.mode != NetDlgMode::GameList || !contains(edit, point) {
            return NetDlgEditInputOutcome::passed();
        }
        self.join_edit.pending_cut = None;
        self.join_edit.caret = self.join_edit.character_at(point.x, edit, font);
        self.join_edit.selection = Some((self.join_edit.caret, self.join_edit.caret));
        let changed = primary_selection
            .filter(|text| !text.is_empty())
            .is_some_and(|text| self.join_edit.insert_raw_text(text));
        self.join_edit.ensure_cursor_in_view(edit, font);
        let actions = changed
            .then(|| NetDlgAction::JoinAddressChanged(self.join_edit.text.clone()))
            .into_iter()
            .collect();
        NetDlgEditInputOutcome::captured(actions)
    }

    pub fn handle_pointer_move(
        &mut self,
        position: GuiPoint,
        font: &ClonkFont,
    ) -> Vec<NetDlgAction> {
        self.pointer_position = Some(position);
        self.hovered = self.hit_button(position);
        let layout = self.layout();
        if self.join_edit.drag_anchor.is_some() {
            let character = self
                .join_edit
                .character_at(position.x, layout.join_edit, font);
            self.join_edit
                .drag_pointer_selection(character, layout.join_edit, font);
        }
        if self.scrollbar_dragging {
            self.set_scroll_from_pointer(position, &layout);
        } else if self.scrollbar_arrow_captured {
            let was_down = self.scrollbar_arrow != 0;
            let inside_bar = contains(layout.list_scrollbar, position);
            self.scrollbar_arrow = self.scrollbar_arrow_at(position, &layout);
            if inside_bar && was_down != (self.scrollbar_arrow != 0) {
                return vec![NetDlgAction::GuiSound(NetDlgSound::ArrowHit)];
            }
        }
        Vec::new()
    }

    pub fn handle_pointer_down(
        &mut self,
        position: GuiPoint,
        font: &ClonkFont,
    ) -> Vec<NetDlgAction> {
        self.pointer_position = Some(position);
        self.hovered = self.hit_button(position);
        self.pointer_pressed = self.hovered;

        let layout = self.layout();
        if self.mode == NetDlgMode::GameList
            && self.max_list_scroll(&layout) > 0
            && contains(layout.list_scrollbar, position)
        {
            self.pointer_pressed = None;
            let mut actions = self.change_focus(NetDlgControl::GameList);
            actions.extend(self.begin_scrollbar_pointer(position, &layout));
            return actions;
        }

        let hit = self.hit_control(position);
        match hit {
            Some(NetDlgControl::JoinAddress) => {
                let actions = self.change_focus(NetDlgControl::JoinAddress);
                let character = self
                    .join_edit
                    .character_at(position.x, layout.join_edit, font);
                self.join_edit
                    .begin_pointer_selection(character, layout.join_edit, font);
                actions
            }
            Some(NetDlgControl::GameList) => {
                let hyperlink = self.hyperlink_at(position, &layout);
                self.select_list_row(position);
                let mut actions = self.change_focus(NetDlgControl::GameList);
                if let Some(url) = hyperlink {
                    actions.push(NetDlgAction::OpenUrl(url));
                }
                actions
            }
            _ => Vec::new(),
        }
    }

    pub fn handle_pointer_up(
        &mut self,
        position: GuiPoint,
        _font: &ClonkFont,
    ) -> Vec<NetDlgAction> {
        self.pointer_position = Some(position);
        self.hovered = self.hit_button(position);
        self.join_edit.drag_anchor = None;
        if self.scrollbar_dragging {
            let layout = self.layout();
            self.set_scroll_from_pointer(position, &layout);
            self.scrollbar_dragging = false;
            return Vec::new();
        }
        if self.scrollbar_arrow_captured {
            let was_down = self.scrollbar_arrow != 0;
            self.scrollbar_arrow_captured = false;
            self.scrollbar_arrow = 0;
            return if was_down {
                vec![NetDlgAction::GuiSound(NetDlgSound::ArrowHit)]
            } else {
                Vec::new()
            };
        }
        let Some(pressed) = self.pointer_pressed.take() else {
            return Vec::new();
        };
        if self.hit_button(position) != Some(pressed) {
            return Vec::new();
        }
        self.activate(pressed)
    }

    /// A native list double-click selects and focuses the row before invoking
    /// the same callback as Return/Join. Only concrete game rows carry a join
    /// target; double-clicking the masterserver status row has no activation.
    pub fn handle_pointer_double_click(
        &mut self,
        position: GuiPoint,
        font: &ClonkFont,
    ) -> Vec<NetDlgAction> {
        self.pointer_position = Some(position);
        self.hovered = self.hit_button(position);
        self.pointer_pressed = None;
        let layout = self.layout();
        if self.mode == NetDlgMode::GameList && contains(layout.join_edit, position) {
            let actions = self.change_focus(NetDlgControl::JoinAddress);
            let character = self
                .join_edit
                .character_at(position.x, layout.join_edit, font);
            self.join_edit
                .select_word_at(character, layout.join_edit, font);
            return actions;
        }
        let Some(index) = self.game_index_at(position) else {
            return Vec::new();
        };
        let mut actions = self.focus_game(index);
        actions.extend(self.join_action());
        actions
    }

    /// Routes the native signed wheel delta over the ScrollWindow viewport.
    /// C4FullScreen supplies +60 for one notch up; ScrollWindow negates it.
    pub fn handle_wheel(&mut self, position: GuiPoint, delta: i32) -> Vec<NetDlgAction> {
        self.pointer_position = Some(position);
        self.hovered = self.hit_button(position);
        let layout = self.layout();
        if self.mode == NetDlgMode::GameList && contains(layout.list_viewport, position) {
            self.scroll_list_by(delta.saturating_neg(), &layout);
        }
        Vec::new()
    }

    /// Advances a held arrow by one fixed thumb pixel, matching
    /// `C4GUI::ScrollBar::DrawElement`. The mapped content offset may remain
    /// unchanged for a frame because both conversions truncate integers.
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

    pub fn handle_key_down(&mut self, key: KeyCode) -> Vec<NetDlgAction> {
        self.handle_key_down_with_tab_direction(key, false)
    }

    pub fn handle_key_down_with_tab_direction(
        &mut self,
        key: KeyCode,
        backwards: bool,
    ) -> Vec<NetDlgAction> {
        match key {
            KeyCode::Escape => vec![NetDlgAction::Back],
            // StartupNetBack binds Left, but an edit/chat input consumes it
            // first at control priority (C4StartupNetDlg.cpp:624-627).
            KeyCode::Left
                if !matches!(
                    self.focus,
                    NetDlgControl::JoinAddress | NetDlgControl::ChatInput
                ) =>
            {
                vec![NetDlgAction::Back]
            }
            KeyCode::Tab => self.move_focus(backwards),
            KeyCode::Up if self.focus == NetDlgControl::GameList => {
                self.move_game_selection(false);
                Vec::new()
            }
            KeyCode::Down if self.focus == NetDlgControl::GameList => {
                self.move_game_selection(true);
                Vec::new()
            }
            KeyCode::Enter | KeyCode::Space if self.focus.is_button() => {
                self.key_pressed = Some((self.focus, key));
                Vec::new()
            }
            KeyCode::Enter
                if matches!(
                    self.focus,
                    NetDlgControl::GameList | NetDlgControl::JoinAddress
                ) =>
            {
                self.join_action()
            }
            _ => Vec::new(),
        }
    }

    pub fn handle_key_up(&mut self, key: KeyCode) -> Vec<NetDlgAction> {
        let Some((pressed, pressed_key)) = self.key_pressed.take() else {
            return Vec::new();
        };
        if pressed_key != key || pressed != self.focus {
            return Vec::new();
        }
        self.activate(pressed)
    }

    fn begin_clipboard_transfer(&mut self, cut: bool) -> Vec<NetDlgAction> {
        let Some((range, text)) = self
            .join_edit
            .selected_range()
            .zip(self.join_edit.selected_text().map(str::to_string))
        else {
            self.join_edit.pending_cut = None;
            return Vec::new();
        };
        self.join_edit.pending_cut = cut.then(|| PendingClipboardCut {
            range,
            text: text.clone(),
        });
        vec![NetDlgAction::ClipboardTransfer { text, cut }]
    }

    fn paste_join_address(&mut self, clipboard: &str, font: &ClonkFont) -> Vec<NetDlgAction> {
        let transformed = clipboard.replace('|', "\u{a6}");
        let mut rest = transformed.as_str();
        loop {
            let Some(line_break) = rest.find(['\r', '\n']) else {
                break;
            };
            if line_break == 0 {
                let skip = rest.chars().next().map_or(0, char::len_utf8);
                rest = &rest[skip..];
                continue;
            }

            let changed = self.join_edit.insert_raw_text(&rest[..line_break]);
            self.join_edit
                .ensure_cursor_in_view(self.layout().join_edit, font);
            let mut actions = Vec::new();
            if changed {
                actions.push(NetDlgAction::JoinAddressChanged(
                    self.join_edit.text.clone(),
                ));
            }
            // CallbackEdit::OnJoinAddressEnter returns IR_Abort, so the first
            // non-leading pasted line break invokes DoOK and discards the tail.
            actions.extend(self.join_action());
            return actions;
        }

        if rest.is_empty() {
            return Vec::new();
        }
        let changed = self.join_edit.insert_raw_text(rest);
        self.join_edit
            .ensure_cursor_in_view(self.layout().join_edit, font);
        changed
            .then(|| NetDlgAction::JoinAddressChanged(self.join_edit.text.clone()))
            .into_iter()
            .collect()
    }

    fn join_address_context_request(
        &self,
        anchor: GuiPoint,
        clipboard_has_text: bool,
    ) -> NetDlgEditContextRequest {
        let has_selection = self.join_edit.selected_range().is_some();
        let item = |command, label: &str, tooltip: &str| NetDlgEditContextItem {
            command,
            label: label.to_string(),
            tooltip: tooltip.to_string(),
        };
        let mut items = Vec::new();
        if has_selection {
            items.push(item(
                NetDlgEditContextCommand::Cut,
                "Cut",
                "Moves the selection to the clipboard.",
            ));
            items.push(item(
                NetDlgEditContextCommand::Copy,
                "Copy",
                "Copies the selection to the clipboard.",
            ));
        }
        if clipboard_has_text {
            items.push(item(
                NetDlgEditContextCommand::Paste,
                "Paste",
                "Inserts the contents of the clipboard.",
            ));
        }
        if has_selection {
            items.push(item(
                NetDlgEditContextCommand::Clear,
                "Clear",
                "Clears the selection.",
            ));
        }
        let whole_text_selected =
            self.join_edit.selected_range() == Some((0, self.join_edit.text.len()));
        if !self.join_edit.text.is_empty() && !whole_text_selected {
            items.push(item(
                NetDlgEditContextCommand::SelectAll,
                "Select all",
                "Selects the complete text",
            ));
        }
        NetDlgEditContextRequest { anchor, items }
    }

    fn layout(&self) -> NetDlgLayout {
        net_dlg_layout(self.width, self.height, &self.metrics)
    }

    fn hit_control(&self, point: GuiPoint) -> Option<NetDlgControl> {
        self.hit_button(point).or_else(|| {
            let layout = self.layout();
            match self.mode {
                NetDlgMode::GameList if contains(layout.join_edit, point) => {
                    Some(NetDlgControl::JoinAddress)
                }
                NetDlgMode::GameList if contains(layout.game_list, point) => {
                    Some(NetDlgControl::GameList)
                }
                _ => None,
            }
        })
    }

    fn hit_button(&self, point: GuiPoint) -> Option<NetDlgControl> {
        let layout = self.layout();
        let mut buttons = vec![
            (NetDlgControl::GamesButton, layout.btn_game_list),
            (NetDlgControl::ChatButton, layout.btn_chat),
            (NetDlgControl::Back, layout.buttons[0]),
            (NetDlgControl::CreateGame, layout.buttons[3]),
        ];
        if self.mode == NetDlgMode::GameList {
            buttons.extend([
                (NetDlgControl::Internet, layout.btn_internet),
                (NetDlgControl::Record, layout.btn_record),
                (NetDlgControl::Refresh, layout.buttons[1]),
                (NetDlgControl::JoinGame, layout.buttons[2]),
            ]);
        }
        buttons
            .into_iter()
            .find_map(|(control, rect)| contains(rect, point).then_some(control))
    }

    pub fn handle_gamepad_horizontal(&mut self, backwards: bool) -> Vec<NetDlgAction> {
        self.move_focus(backwards)
    }

    fn move_focus(&mut self, backwards: bool) -> Vec<NetDlgAction> {
        const GAME_LIST_ORDER: [NetDlgControl; 10] = [
            NetDlgControl::GameList,
            NetDlgControl::JoinAddress,
            NetDlgControl::Internet,
            NetDlgControl::Record,
            NetDlgControl::Back,
            NetDlgControl::Refresh,
            NetDlgControl::JoinGame,
            NetDlgControl::CreateGame,
            NetDlgControl::GamesButton,
            NetDlgControl::ChatButton,
        ];
        const CHAT_ORDER: [NetDlgControl; 5] = [
            NetDlgControl::ChatInput,
            NetDlgControl::Back,
            NetDlgControl::CreateGame,
            NetDlgControl::GamesButton,
            NetDlgControl::ChatButton,
        ];
        if self.mode == NetDlgMode::Chat {
            match (self.focus, backwards) {
                (NetDlgControl::Internet | NetDlgControl::Record, false) => {
                    return self.change_focus(NetDlgControl::Back);
                }
                (NetDlgControl::Internet | NetDlgControl::Record, true) => {
                    return self.change_focus(NetDlgControl::ChatInput);
                }
                (NetDlgControl::Refresh | NetDlgControl::JoinGame, false) => {
                    return self.change_focus(NetDlgControl::CreateGame);
                }
                (NetDlgControl::Refresh | NetDlgControl::JoinGame, true) => {
                    return self.change_focus(NetDlgControl::Back);
                }
                _ => {}
            }
        }
        let order = match self.mode {
            NetDlgMode::GameList => GAME_LIST_ORDER.as_slice(),
            NetDlgMode::Chat => CHAT_ORDER.as_slice(),
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

    fn change_focus(&mut self, focus: NetDlgControl) -> Vec<NetDlgAction> {
        if self.focus == focus {
            return Vec::new();
        }
        if self.focus == NetDlgControl::JoinAddress {
            self.join_edit.blur();
        }
        self.focus = focus;
        if focus == NetDlgControl::JoinAddress {
            self.join_edit.focus();
        }
        self.key_pressed = None;
        vec![NetDlgAction::FocusChanged(focus)]
    }

    fn activate(&mut self, control: NetDlgControl) -> Vec<NetDlgAction> {
        match control {
            NetDlgControl::GamesButton => self.change_mode(NetDlgMode::GameList),
            NetDlgControl::ChatButton => {
                let mode = match self.mode {
                    NetDlgMode::GameList => NetDlgMode::Chat,
                    NetDlgMode::Chat => NetDlgMode::GameList,
                };
                self.change_mode(mode)
            }
            NetDlgControl::Internet => {
                self.config.masterserver_signup = !self.config.masterserver_signup;
                if !self.config.masterserver_signup
                    && self.selection == Some(NetDlgSelection::Masterserver)
                {
                    self.selection = None;
                }
                self.clamp_list_scroll();
                vec![NetDlgAction::MasterserverSignupChanged(
                    self.config.masterserver_signup,
                )]
            }
            NetDlgControl::Record => {
                self.config.record = !self.config.record;
                vec![NetDlgAction::RecordingChanged(self.config.record)]
            }
            NetDlgControl::Back => vec![NetDlgAction::Back],
            NetDlgControl::Refresh => vec![NetDlgAction::Refresh],
            NetDlgControl::JoinGame if self.mode == NetDlgMode::Chat => Vec::new(),
            NetDlgControl::JoinGame => self.join_action(),
            NetDlgControl::CreateGame => vec![NetDlgAction::CreateGame],
            NetDlgControl::GameList | NetDlgControl::JoinAddress | NetDlgControl::ChatInput => {
                Vec::new()
            }
        }
    }

    fn change_mode(&mut self, mode: NetDlgMode) -> Vec<NetDlgAction> {
        self.mode = mode;
        self.scrollbar_dragging = false;
        self.scrollbar_arrow_captured = false;
        self.scrollbar_arrow = 0;
        let replacement_focus = match (mode, self.focus) {
            (NetDlgMode::Chat, NetDlgControl::GameList | NetDlgControl::JoinAddress) => {
                Some(NetDlgControl::ChatInput)
            }
            (NetDlgMode::GameList, NetDlgControl::ChatInput) => Some(NetDlgControl::GameList),
            _ => None,
        };
        let mut actions = vec![NetDlgAction::ModeChanged(mode)];
        if let Some(focus) = replacement_focus {
            actions.extend(self.change_focus(focus));
        }
        actions
    }

    fn join_action(&mut self) -> Vec<NetDlgAction> {
        if self.focus == NetDlgControl::JoinAddress && !self.join_edit.text.is_empty() {
            let mut actions = vec![NetDlgAction::QueryAddress {
                address: self.join_edit.text.clone(),
            }];
            actions.extend(self.change_focus(NetDlgControl::GameList));
            return actions;
        }
        let selected_address = self
            .selected_game()
            .and_then(|index| self.games.get(index))
            .and_then(|game| game.address.clone());
        vec![NetDlgAction::JoinGame {
            address: selected_address,
        }]
    }

    fn select_list_row(&mut self, position: GuiPoint) {
        let previous = self.selection;
        self.selection = self.list_selection_at(position);
        if self.selection != previous {
            let layout = self.layout();
            self.ensure_selection_visible(&layout);
        }
    }

    fn list_selection_at(&self, position: GuiPoint) -> Option<NetDlgSelection> {
        let layout = self.layout();
        if !contains(layout.list_viewport, position) {
            return None;
        }
        let content_y = position.y as i32 - layout.list_viewport.y + self.list_scroll_y;
        self.row_layouts(&layout)
            .into_iter()
            .find(|row| {
                let top = row.rect.y - layout.list_viewport.y;
                content_y >= top && content_y < top.saturating_add(row.rect.h)
            })
            .map(|row| row.selection)
    }

    fn hyperlink_at(&self, position: GuiPoint, layout: &NetDlgLayout) -> Option<String> {
        if !contains(layout.list_viewport, position) {
            return None;
        }
        self.row_layouts(layout)
            .into_iter()
            .flat_map(|row| row.lines)
            .find_map(|line| {
                let hyperlink = line.hyperlink?;
                contains(offset(line.rect, 0, -self.list_scroll_y), position).then_some(hyperlink)
            })
    }

    fn move_game_selection(&mut self, forward: bool) {
        let previous = self.selection;
        let master = self.config.masterserver_signup;
        self.selection = match (self.selection, forward) {
            (None, true) if master => Some(NetDlgSelection::Masterserver),
            (None, true) => (!self.games.is_empty()).then_some(NetDlgSelection::Game(0)),
            (None, false) => self
                .games
                .len()
                .checked_sub(1)
                .map(NetDlgSelection::Game)
                .or(master.then_some(NetDlgSelection::Masterserver)),
            (Some(NetDlgSelection::Masterserver), true) => (!self.games.is_empty())
                .then_some(NetDlgSelection::Game(0))
                .or(self.selection),
            (Some(NetDlgSelection::Masterserver), false) => self.selection,
            (Some(NetDlgSelection::Game(index)), true) => (index + 1 < self.games.len())
                .then_some(NetDlgSelection::Game(index + 1))
                .or(self.selection),
            (Some(NetDlgSelection::Game(0)), false) if master => {
                Some(NetDlgSelection::Masterserver)
            }
            (Some(NetDlgSelection::Game(index)), false) => index
                .checked_sub(1)
                .map(NetDlgSelection::Game)
                .or(self.selection),
        };
        if self.selection != previous {
            let layout = self.layout();
            self.ensure_selection_visible(&layout);
        }
    }

    fn list_content_height(&self, layout: &NetDlgLayout) -> i32 {
        self.row_layouts(layout)
            .last()
            .map_or(0, |row| row.rect.y + row.rect.h - layout.list_viewport.y)
    }

    fn row_layouts(&self, layout: &NetDlgLayout) -> Vec<NetDlgRowLayout> {
        self.row_layouts_with_font(layout, self.text_font.as_ref())
    }

    fn list_is_collapsed_with_font(&self, layout: &NetDlgLayout, font: Option<&ClonkFont>) -> bool {
        let mut expanded_height = 0_i32;
        let mut has_previous = false;
        if self.config.masterserver_signup {
            expanded_height = expanded_height.saturating_add(
                self.layout_row(
                    layout,
                    font,
                    NetDlgSelection::Masterserver,
                    layout.list_viewport.y,
                    self.masterserver_lines(),
                    &[],
                    self.masterserver.row_icon,
                    true,
                    false,
                )
                .rect
                .h,
            );
            has_previous = true;
        }
        for (index, game) in self.games.iter().enumerate() {
            if has_previous {
                expanded_height = expanded_height.saturating_add(EXPANDED_ROW_TOP_SPACING);
            }
            expanded_height = expanded_height.saturating_add(
                self.layout_row(
                    layout,
                    font,
                    NetDlgSelection::Game(index),
                    layout.list_viewport.y,
                    Self::game_lines(game),
                    &game.status_icons,
                    game.row_icon,
                    game.joinable,
                    false,
                )
                .rect
                .h,
            );
            has_previous = true;
        }
        expanded_height > layout.list_viewport.h
    }

    fn row_layouts_with_font(
        &self,
        layout: &NetDlgLayout,
        font: Option<&ClonkFont>,
    ) -> Vec<NetDlgRowLayout> {
        let collapsed = self.list_is_collapsed_with_font(layout, font);
        let mut rows =
            Vec::with_capacity(self.games.len() + usize::from(self.config.masterserver_signup));
        let mut y = layout.list_viewport.y;
        let mut has_previous = false;
        if self.config.masterserver_signup {
            let master_selection = NetDlgSelection::Masterserver;
            let master_collapsed =
                collapsed && self.selection.is_some() && self.selection != Some(master_selection);
            let row = self.layout_row(
                layout,
                font,
                master_selection,
                y,
                self.masterserver_lines(),
                &[],
                self.masterserver.row_icon,
                true,
                master_collapsed,
            );
            y = y.saturating_add(row.rect.h);
            rows.push(row);
            has_previous = true;
        }
        for (index, game) in self.games.iter().enumerate() {
            let selection = NetDlgSelection::Game(index);
            let row_collapsed = collapsed && self.selection != Some(selection);
            if has_previous {
                y = y.saturating_add(if row_collapsed {
                    COLLAPSED_ROW_TOP_SPACING
                } else {
                    EXPANDED_ROW_TOP_SPACING
                });
            }
            let row = self.layout_row(
                layout,
                font,
                selection,
                y,
                Self::game_lines(game),
                &game.status_icons,
                game.row_icon,
                game.joinable,
                row_collapsed,
            );
            y = y.saturating_add(row.rect.h);
            rows.push(row);
            has_previous = true;
        }
        rows
    }

    #[allow(clippy::too_many_arguments)]
    fn layout_row(
        &self,
        layout: &NetDlgLayout,
        font: Option<&ClonkFont>,
        selection: NetDlgSelection,
        y: i32,
        mut lines: Vec<NetDlgTextLine>,
        status_icons: &[NetDlgStatusIcon],
        row_icon: NetDlgRowIcon,
        joinable: bool,
        collapsed: bool,
    ) -> NetDlgRowLayout {
        lines.truncate(if collapsed { 2 } else { 5 });
        let status_icons = status_icons
            .iter()
            .copied()
            .take(MAX_INFO_ICONS)
            .collect::<Vec<_>>();
        let label_x = layout.entry_labels[0].x;
        let label_width = layout.entry_labels[0].w;
        let mut line_y = y.saturating_add(1);
        let mut line_layouts = Vec::with_capacity(lines.len());
        for (index, line) in lines.into_iter().enumerate() {
            let width = if index == 0 {
                label_width
                    .saturating_sub(
                        self.metrics
                            .text_line_height
                            .saturating_mul(i32::try_from(status_icons.len()).unwrap_or(i32::MAX)),
                    )
                    .max(1)
            } else {
                label_width.max(1)
            };
            let text = font.map_or_else(
                || line.text().to_string(),
                |font| break_message(font, line.text(), width),
            );
            let text_height = font
                .map(|font| font.measure(&text, true).1)
                .unwrap_or(self.metrics.text_line_height)
                .max(self.metrics.text_line_height);
            line_layouts.push(NetDlgLineLayout {
                rect: IntRect {
                    x: label_x,
                    y: line_y,
                    w: width,
                    h: text_height,
                },
                text,
                hyperlink: line.hyperlink().map(ToOwned::to_owned),
            });
            line_y = line_y.saturating_add(text_height).saturating_add(2);
        }
        let row_height = line_y
            .saturating_sub(y)
            .saturating_sub(1)
            .max(layout.list_entry.h);
        NetDlgRowLayout {
            selection,
            rect: IntRect {
                x: layout.list_entry.x,
                y,
                w: layout.list_entry.w,
                h: row_height,
            },
            lines: line_layouts,
            status_icons,
            row_icon,
            joinable,
        }
    }

    fn masterserver_lines(&self) -> Vec<NetDlgTextLine> {
        let mut lines = vec![
            NetDlgTextLine::Plain(self.masterserver.title.clone()),
            NetDlgTextLine::Plain(self.masterserver.details.clone()),
        ];
        lines.extend(self.masterserver.extra_lines.iter().cloned());
        lines
    }

    fn game_lines(game: &NetDlgGameEntry) -> Vec<NetDlgTextLine> {
        let mut lines = vec![
            NetDlgTextLine::Plain(game.title.clone()),
            NetDlgTextLine::Plain(game.details.clone()),
        ];
        lines.extend(game.extra_lines.iter().cloned().map(NetDlgTextLine::Plain));
        lines
    }

    fn max_list_scroll(&self, layout: &NetDlgLayout) -> i32 {
        self.list_content_height(layout)
            .saturating_sub(layout.list_viewport.h)
            .max(0)
    }

    fn scrollbar_has_pin(layout: &NetDlgLayout) -> bool {
        layout.list_scrollbar.h > 3 * SCROLLBAR_PART
    }

    fn scrollbar_range(layout: &NetDlgLayout) -> i32 {
        if Self::scrollbar_has_pin(layout) {
            layout.list_scrollbar.h - 3 * SCROLLBAR_PART
        } else {
            // C4GUI::ScrollBar::GetMaxScroll uses a synthetic range when the
            // viewport is too short to display a thumb. Arrows remain usable.
            100
        }
    }

    fn clamp_list_scroll(&mut self) {
        let layout = self.layout();
        self.list_scroll_y = self.list_scroll_y.clamp(0, self.max_list_scroll(&layout));
        self.sync_pin_from_scroll(&layout);
        if self.max_list_scroll(&layout) == 0 {
            self.scrollbar_dragging = false;
            self.scrollbar_arrow_captured = false;
            self.scrollbar_arrow = 0;
        }
    }

    fn sync_pin_from_scroll(&mut self, layout: &NetDlgLayout) {
        let max_scroll = self.max_list_scroll(layout);
        self.list_scroll_pin = if max_scroll == 0 || !Self::scrollbar_has_pin(layout) {
            0
        } else {
            Self::scrollbar_range(layout) * self.list_scroll_y / max_scroll
        };
    }

    fn scroll_list_by(&mut self, amount: i32, layout: &NetDlgLayout) {
        self.list_scroll_y = self
            .list_scroll_y
            .saturating_add(amount)
            .clamp(0, self.max_list_scroll(layout));
        self.sync_pin_from_scroll(layout);
    }

    fn set_scroll_from_pointer(&mut self, point: GuiPoint, layout: &NetDlgLayout) {
        let max_pin = Self::scrollbar_range(layout);
        self.list_scroll_pin =
            (point.y as i32 - layout.list_scrollbar.y - SCROLLBAR_PART - SCROLLBAR_PART / 2)
                .clamp(0, max_pin);
        self.list_scroll_y = self.max_list_scroll(layout) * self.list_scroll_pin / max_pin.max(1);
    }

    fn scrollbar_arrow_at(&self, point: GuiPoint, layout: &NetDlgLayout) -> i8 {
        if !contains(layout.list_scrollbar, point) {
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

    fn begin_scrollbar_pointer(
        &mut self,
        point: GuiPoint,
        layout: &NetDlgLayout,
    ) -> Vec<NetDlgAction> {
        let arrow = self.scrollbar_arrow_at(point, layout);
        if arrow != 0 {
            self.scrollbar_arrow_captured = true;
            self.scrollbar_arrow = arrow;
            vec![NetDlgAction::GuiSound(NetDlgSound::ArrowHit)]
        } else if Self::scrollbar_has_pin(layout) {
            self.scrollbar_arrow_captured = false;
            self.set_scroll_from_pointer(point, layout);
            self.scrollbar_dragging = true;
            vec![NetDlgAction::GuiSound(NetDlgSound::Command)]
        } else {
            Vec::new()
        }
    }

    fn ensure_selection_visible(&mut self, layout: &NetDlgLayout) {
        let Some(selection) = self.selection else {
            return;
        };
        let Some(row) = self
            .row_layouts(layout)
            .into_iter()
            .find(|row| row.selection == selection)
        else {
            return;
        };
        let top = row.rect.y - layout.list_viewport.y;
        let bottom = top.saturating_add(row.rect.h);
        if self.list_scroll_y > top {
            self.list_scroll_y = top;
        } else if self.list_scroll_y + layout.list_viewport.h < bottom {
            self.list_scroll_y = bottom - layout.list_viewport.h;
        }
        self.list_scroll_y = self.list_scroll_y.clamp(0, self.max_list_scroll(layout));
        self.sync_pin_from_scroll(layout);
    }

    fn is_highlighted(&self, control: NetDlgControl) -> bool {
        self.focus == control || self.hovered == Some(control)
    }

    fn is_pressed(&self, control: NetDlgControl) -> bool {
        self.pointer_pressed == Some(control)
            || self
                .key_pressed
                .is_some_and(|(pressed, _)| pressed == control)
    }
}

impl NetDlgControl {
    const fn is_button(self) -> bool {
        !matches!(self, Self::GameList | Self::JoinAddress | Self::ChatInput)
    }
}

fn contains(rect: IntRect, point: GuiPoint) -> bool {
    point.x >= rect.x as f32
        && point.y >= rect.y as f32
        && point.x < (rect.x + rect.w) as f32
        && point.y < (rect.y + rect.h) as f32
}

fn truncate_utf8(text: &str, byte_limit: usize) -> &str {
    let mut end = text.len().min(byte_limit);
    while !text.is_char_boundary(end) {
        end -= 1;
    }
    &text[..end]
}

fn previous_boundary(text: &str, position: usize) -> usize {
    text[..position.min(text.len())]
        .char_indices()
        .next_back()
        .map_or(0, |(index, _)| index)
}

fn next_boundary(text: &str, position: usize) -> usize {
    if position >= text.len() {
        return text.len();
    }
    position + text[position..].chars().next().map_or(0, char::len_utf8)
}

fn char_at(text: &str, position: usize) -> char {
    text.get(position..)
        .and_then(|tail| tail.chars().next())
        .unwrap_or('\0')
}

fn is_word_spacer(character: char) -> bool {
    character.is_ascii() && !character.is_ascii_alphanumeric() && character != '_'
}

/// `Edit::DrawElement` renders its broken-bar cursor at scale 1.5 while the
/// ordinary edit text remains at scale one.
fn draw_scaled_caret(
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

/// Renders `C4StartupNetDlg`'s deterministic first-shown state.
pub struct NetDlgScreen;

impl NetDlgScreen {
    /// Draws the full dialog in the exact C++ draw order (spec "Draw order";
    /// Container::Draw insertion order, C4GuiContainers.cpp:273-294).
    /// `get_ref_phase` selects the animated query-icon phase, which is
    /// frame-count driven (nondeterministic) in C++ and masked when diffing.
    pub fn render(
        surface: &mut Surface,
        assets: &NetDlgAssets,
        fonts: &ClonkFontSet,
        gamma: Option<&GammaRamp>,
        config: NetDlgConfig,
        get_ref_phase: u32,
    ) {
        Self::render_impl(
            surface,
            assets,
            fonts,
            gamma,
            config,
            None,
            get_ref_phase,
            false,
            false,
        );
    }

    /// Draws the live controller state. Unlike [`Self::render`], this path
    /// reflects the active game/chat sheet, edited join address, config
    /// toggles and button focus/hover/press state.
    pub fn render_controller(
        surface: &mut Surface,
        assets: &NetDlgAssets,
        fonts: &ClonkFontSet,
        gamma: Option<&GammaRamp>,
        controller: &NetDlgController,
        get_ref_phase: u32,
    ) {
        Self::render_controller_with_draw_focus(
            surface,
            assets,
            fonts,
            gamma,
            controller,
            get_ref_phase,
            true,
        );
    }

    /// Live rendering with the screen-level `HasDrawFocus` gate. An open
    /// context menu leaves the edit selected but suppresses its flashing caret.
    #[allow(clippy::too_many_arguments)]
    pub fn render_controller_with_draw_focus(
        surface: &mut Surface,
        assets: &NetDlgAssets,
        fonts: &ClonkFontSet,
        gamma: Option<&GammaRamp>,
        controller: &NetDlgController,
        get_ref_phase: u32,
        draw_focus: bool,
    ) {
        Self::render_impl(
            surface,
            assets,
            fonts,
            gamma,
            controller.config,
            Some(controller),
            get_ref_phase,
            draw_focus,
            controller.join_edit.cursor_visible(),
        );
    }

    #[allow(clippy::too_many_arguments)]
    fn render_impl(
        surface: &mut Surface,
        assets: &NetDlgAssets,
        fonts: &ClonkFontSet,
        gamma: Option<&GammaRamp>,
        config: NetDlgConfig,
        controller: Option<&NetDlgController>,
        get_ref_phase: u32,
        draw_focus: bool,
        cursor_visible: bool,
    ) {
        let (w, h) = (surface.width() as i32, surface.height() as i32);
        let metrics = NetDlgFontMetrics::from_fonts(fonts);
        let layout = net_dlg_layout(w, h, &metrics);
        let mode = controller.map_or(NetDlgMode::GameList, |state| state.mode);
        let button_highlight =
            controller.map(|_| blacken_transparent_pixels(&assets.gui_button_highlight));
        let classic_skin = ClassicGuiSkin::new(
            &assets.gui_caption,
            &assets.gui_button,
            &assets.gui_button_down,
            button_highlight.as_ref(),
        );

        // ① Background: StartupNetworkBG plain-stretched one pixel past every
        // screen edge (FullscreenDialog::DrawBackground, C4GuiDialogs.cpp:878-887).
        crate::draw_image_bilinear(
            surface,
            &GuiRect::new(-1.0, -1.0, (w + 2) as f32, (h + 2) as f32),
            &assets.background,
            gamma,
        );

        // ② Title, centered big font (C4GuiDialogs.cpp:843-845; Label draw
        // C4GuiLabels.cpp:34-50).
        fonts.title.draw_with_gamma(
            surface,
            layout.title_anchor.0,
            layout.title_anchor.1,
            "Start Network Game",
            CLR_YELLOW,
            TextAlign::Center,
            true,
            gamma,
        );

        // ③④ Left icon buttons (IconButton::DrawElement, C4GuiButton.cpp:205-232).
        // GUIIcons2 sources: Ico_Ex_GameList = Ex+16, Ico_Ex_Chat = Ex+15 on a
        // 4-column 64x64 grid (C4GuiLabels.cpp:441-450).
        Self::icon_button(
            surface,
            assets,
            fonts,
            gamma,
            layout.btn_game_list,
            (0, 256),
            "&Games",
            button_highlight.as_ref(),
            controller.is_some_and(|state| state.is_highlighted(NetDlgControl::GamesButton)),
            controller.is_some_and(|state| state.is_pressed(NetDlgControl::GamesButton)),
        );
        Self::icon_button(
            surface,
            assets,
            fonts,
            gamma,
            layout.btn_chat,
            (192, 192),
            "&Chat",
            button_highlight.as_ref(),
            controller.is_some_and(|state| state.is_highlighted(NetDlgControl::ChatButton)),
            controller.is_some_and(|state| state.is_pressed(NetDlgControl::ChatButton)),
        );

        if mode == NetDlgMode::GameList {
            // ⑤ Tabular sheet 0 (zero chrome; C4GuiTabular.cpp:362-364):
            // "Running Games" wooden caption (WoodenLabel::DrawElement,
            // C4GuiLabels.cpp:168-209): bar, then ALeft text at x+5, vertically
            // centered minus one, clipped to the label's inclusive bounds.
            let capt = layout.game_list_caption;
            classic_skin.draw_caption(
                surface,
                capt,
                "Running Games",
                &fonts.text,
                CLR_YELLOW,
                TextAlign::Left,
                gamma,
            );

            // Game list box (ListBox::DrawElement, C4GuiListBox.cpp:100-139):
            // dark background box over the inclusive bounds, then the 3D frame.
            // No selection bar, delimiters or scroll bar at t=0.
            let list = layout.game_list;
            draw_engine_box(
                surface,
                list.x,
                list.y,
                list.x + list.w - 1,
                list.y + list.h - 1,
                CLR_DARK_BG,
                gamma,
            );
            draw_3d_frame(surface, list, gamma);

            // ScrollWindow draws its children at y=-scroll and installs its
            // viewport as the primary clipper. Keep the clip on the original
            // surface so semantic/native font capture retains it as well.
            let viewport = layout.list_viewport;
            let saved_clip = surface.clip();
            let viewport_clip = lc_graphics::Rect::new(
                viewport.x,
                viewport.y,
                viewport.w.max(0) as u32,
                viewport.h.max(0) as u32,
            );
            let active_clip = saved_clip
                .and_then(|clip| clip.intersection(viewport_clip))
                .unwrap_or_else(|| {
                    if saved_clip.is_some() {
                        lc_graphics::Rect::new(viewport.x, viewport.y, 0, 0)
                    } else {
                        viewport_clip
                    }
                });
            surface.set_clip(active_clip);

            let scroll_y = controller.map_or(0, |state| state.list_scroll_y);
            let row_visible =
                |rect: IntRect| rect.y < viewport.y + viewport.h && rect.y + rect.h > viewport.y;
            let fallback_controller;
            let row_state = match controller {
                Some(controller) => controller,
                None => {
                    fallback_controller = NetDlgController::new(config, metrics);
                    &fallback_controller
                }
            };
            for row in row_state.row_layouts_with_font(&layout, Some(&fonts.text)) {
                let row_rect = offset(row.rect, 0, -scroll_y);
                if !row_visible(row_rect) {
                    continue;
                }
                let selected = controller
                    .is_some_and(|controller| controller.selection == Some(row.selection));
                let official = row.status_icons.contains(&NetDlgStatusIcon::OfficialServer);
                if selected {
                    draw_engine_box(
                        surface,
                        row_rect.x,
                        row_rect.y,
                        row_rect.x + row_rect.w - 1,
                        row_rect.y + row_rect.h - 1,
                        CLR_SELECTION,
                        gamma,
                    );
                } else if official {
                    // C4StartupNetListEntry::DrawElement uses the native
                    // inclusive endpoint quirk (`x + Wdt`, `y + Hgt`).
                    draw_engine_box(
                        surface,
                        row_rect.x,
                        row_rect.y,
                        row_rect.x + row_rect.w,
                        row_rect.y + row_rect.h,
                        CLR_IMPORTANT_BG,
                        gamma,
                    );
                }

                Self::draw_row_icon(
                    surface,
                    assets,
                    row_rect,
                    row.row_icon,
                    get_ref_phase,
                    gamma,
                );
                for (index, icon) in row.status_icons.iter().copied().enumerate() {
                    let size = metrics.text_line_height;
                    let icon_rect = IntRect {
                        x: row_rect.x + row_rect.w
                            - size * (i32::try_from(index).unwrap_or(i32::MAX) + 1),
                        y: row_rect.y,
                        w: size,
                        h: size,
                    };
                    Self::draw_status_icon(surface, assets, icon_rect, icon, gamma);
                }

                let ordinary_color = if row.joinable {
                    CLR_WHITE
                } else {
                    CLR_DISABLED
                };
                for line in row.lines {
                    let rect = offset(line.rect, 0, -scroll_y);
                    let color = if line.hyperlink.is_some() {
                        CLR_HYPERLINK
                    } else {
                        ordinary_color
                    };
                    fonts.text.draw_with_gamma(
                        surface,
                        rect.x,
                        rect.y,
                        &line.text,
                        color,
                        TextAlign::Left,
                        true,
                        gamma,
                    );
                    if line.hyperlink.is_some() {
                        Self::draw_hyperlink_underline(
                            surface,
                            &fonts.text,
                            rect,
                            &line.text,
                            gamma,
                        );
                    }
                }
            }

            if let Some(saved) = saved_clip {
                surface.set_clip(saved);
            } else {
                surface.clear_clip();
            }

            if let Some(controller) = controller {
                if controller.max_list_scroll(&layout) > 0 {
                    Self::draw_scrollbar(surface, assets, controller, layout, gamma);
                }
            }

            // "IP:" wooden label (C4StartupNetDlg.cpp:679-688): the 28px bar
            // exercises DrawBar's overflow quirk; ACenter text, top row clipped.
            let ip = layout.ip_label;
            classic_skin.draw_caption(
                surface,
                ip,
                "IP:",
                &fonts.text,
                CLR_YELLOW,
                TextAlign::Center,
                gamma,
            );

            // Join-address `C4GUI::Edit` (C4GuiEdit.cpp:556-634).
            let edit = layout.join_edit;
            let client = edit_client(edit);
            draw_engine_box(
                surface,
                edit.x,
                edit.y,
                edit.x + edit.w - 1,
                client.y + client.h,
                CLR_DARK_BG,
                gamma,
            );
            draw_3d_frame(surface, edit, gamma);

            if let Some(controller) = controller {
                let state = &controller.join_edit;
                let (text_y0, selection_height) = if client.h <= fonts.text.line_height {
                    (client.y, client.h)
                } else {
                    (
                        client.y + (client.h - fonts.text.line_height) / 2 + 1,
                        fonts.text.line_height - 2,
                    )
                };
                let clip = IntRect {
                    x: client.x - 2,
                    y: client.y,
                    w: client.w + 4,
                    h: client.h + 1,
                };
                if let Some((start, end)) = state.selected_range() {
                    let x1 = client.x + fonts.text.measure(&state.text[..start], false).0
                        - state.horizontal_scroll;
                    let x2 = client.x + fonts.text.measure(&state.text[..end], false).0
                        - state.horizontal_scroll;
                    let clipped_x1 = x1.max(clip.x);
                    let clipped_x2 = (x2 - 1).min(clip.x + clip.w - 1);
                    if clipped_x1 <= clipped_x2 {
                        draw_engine_box(
                            surface,
                            clipped_x1,
                            text_y0,
                            clipped_x2,
                            text_y0 + selection_height - 1,
                            CLR_EDIT_SELECTION,
                            gamma,
                        );
                    }
                }
                draw_clipped_text(
                    surface,
                    &fonts.text,
                    client.x - state.horizontal_scroll,
                    text_y0 - 1,
                    &state.text,
                    CLR_WHITE,
                    TextAlign::Left,
                    gamma,
                    clip,
                );
                if draw_focus && cursor_visible && controller.focus == NetDlgControl::JoinAddress {
                    let caret_x = client.x
                        + fonts.text.measure(&state.text[..state.caret], false).0
                        - fonts.text.measure("\u{a6}", false).0 / 2
                        - state.horizontal_scroll;
                    draw_scaled_caret(
                        surface,
                        &fonts.text,
                        caret_x,
                        text_y0 - fonts.text.line_height / 3,
                        clip,
                        gamma,
                    );
                }
            }
        } else {
            Self::draw_chat_sheet(surface, fonts, &classic_skin, gamma, layout);
        }

        // ⑥⑦ Right icon buttons, config-driven (C4StartupNetDlg.cpp:710-717)
        // on the 4-column grid: InternetOn Ex+7 (192,64) / InternetOff Ex+6
        // (128,64); RecordOn Ex+1 (64,0) / RecordOff Ex+0 (0,0).
        if mode == NetDlgMode::GameList {
            let internet_src = if config.masterserver_signup {
                (192, 64)
            } else {
                (128, 64)
            };
            let record_src = if config.record { (64, 0) } else { (0, 0) };
            Self::icon_button(
                surface,
                assets,
                fonts,
                gamma,
                layout.btn_internet,
                internet_src,
                "&Internet",
                button_highlight.as_ref(),
                controller.is_some_and(|state| state.is_highlighted(NetDlgControl::Internet)),
                controller.is_some_and(|state| state.is_pressed(NetDlgControl::Internet)),
            );
            Self::icon_button(
                surface,
                assets,
                fonts,
                gamma,
                layout.btn_record,
                record_src,
                "&Record",
                button_highlight.as_ref(),
                controller.is_some_and(|state| state.is_highlighted(NetDlgControl::Record)),
                controller.is_some_and(|state| state.is_pressed(NetDlgControl::Record)),
            );
        }

        // ⑧⑨⑩ Bottom wooden buttons (Button::DrawElement, C4GuiButton.cpp:81-110):
        // GUIButton 3-slice plank, then the markup caption centered at
        // ((x0+x1)/2, (y0+y1-LH)/2) in the largest font fitting Hgt-2.
        let buttons = [
            (NetDlgControl::Back, "Back"),
            (NetDlgControl::Refresh, "Reloa&d"),
            (NetDlgControl::JoinGame, "&Join game"),
            (NetDlgControl::CreateGame, "&New game"),
        ];
        for (index, (control, label)) in buttons.into_iter().enumerate() {
            if mode == NetDlgMode::Chat
                && matches!(control, NetDlgControl::Refresh | NetDlgControl::JoinGame)
            {
                continue;
            }
            classic_skin.draw_button(
                surface,
                layout.buttons[index],
                label,
                fonts,
                ClassicButtonState {
                    pressed: controller.is_some_and(|state| state.is_pressed(control)),
                    highlighted: controller.is_some_and(|state| state.is_highlighted(control)),
                },
                gamma,
            );
        }
    }

    fn draw_row_icon(
        surface: &mut Surface,
        assets: &NetDlgAssets,
        row: IntRect,
        icon: NetDlgRowIcon,
        get_ref_phase: u32,
        gamma: Option<&GammaRamp>,
    ) {
        let large_size = row.h.min(48).max(0);
        match icon {
            NetDlgRowIcon::None => {}
            NetDlgRowIcon::Query => {
                let phase = net_get_ref_phase(&assets.net_get_ref, get_ref_phase);
                let fitted_height = 32 * large_size / 40;
                crate::draw_image_bilinear(
                    surface,
                    &gui_rect(IntRect {
                        x: row.x,
                        y: row.y + (large_size - fitted_height) / 2,
                        w: large_size,
                        h: fitted_height,
                    }),
                    &phase,
                    gamma,
                );
            }
            NetDlgRowIcon::QueryStatic => {
                let phase = net_get_ref_phase(&assets.net_get_ref, 0);
                let fitted_height = 32 * large_size / 40;
                crate::draw_image_bilinear(
                    surface,
                    &gui_rect(IntRect {
                        x: row.x,
                        y: row.y + (large_size - fitted_height) / 2,
                        w: large_size,
                        h: fitted_height,
                    }),
                    &phase,
                    gamma,
                );
            }
            NetDlgRowIcon::Error => {
                let small_size = large_size * 2 / 3;
                Self::draw_icon_phase(
                    surface,
                    &assets.gui_icons,
                    40,
                    34,
                    IntRect {
                        x: row.x + (large_size - small_size) / 2,
                        y: row.y + (large_size - small_size) / 2,
                        w: small_size,
                        h: small_size,
                    },
                    gamma,
                );
            }
            NetDlgRowIcon::Scenario(raw_phase) => {
                let phase = if (0..SCENARIO_ICON_COUNT).contains(&raw_phase) {
                    raw_phase
                } else {
                    DEFAULT_SCENARIO_ICON_PHASE
                };
                let small_size = large_size * 2 / 3;
                Self::draw_icon_phase(
                    surface,
                    &assets.scen_icons,
                    24,
                    phase as u32,
                    IntRect {
                        x: row.x + (large_size - small_size) / 2,
                        y: row.y + (large_size - small_size) / 2,
                        w: small_size,
                        h: small_size,
                    },
                    gamma,
                );
            }
        }
    }

    fn draw_status_icon(
        surface: &mut Surface,
        assets: &NetDlgAssets,
        rect: IntRect,
        icon: NetDlgStatusIcon,
        gamma: Option<&GammaRamp>,
    ) {
        let (extended, phase) = icon.source();
        let (image, cell) = if extended {
            (&assets.gui_icons_ex, 64)
        } else {
            (&assets.gui_icons, 40)
        };
        Self::draw_icon_phase(surface, image, cell, phase, rect, gamma);
    }

    fn draw_icon_phase(
        surface: &mut Surface,
        image: &ImageData,
        cell: u32,
        phase: u32,
        rect: IntRect,
        gamma: Option<&GammaRamp>,
    ) {
        let columns = (image.width() / cell).max(1);
        let source_x = phase % columns * cell;
        let source_y = phase / columns * cell;
        draw_facet_stretch(
            surface,
            image,
            (source_x as f32, source_y as f32, cell as f32, cell as f32),
            (rect.x as f32, rect.y as f32, rect.w as f32, rect.h as f32),
            gamma,
        );
    }

    fn draw_hyperlink_underline(
        surface: &mut Surface,
        font: &ClonkFont,
        rect: IntRect,
        text: &str,
        gamma: Option<&GammaRamp>,
    ) {
        let (width, height) = font.measure(text, true);
        if width <= 0 || height <= 0 {
            return;
        }
        let y = rect.y + height - 2;
        draw_engine_box(
            surface,
            rect.x,
            y,
            rect.x + width - 1,
            y,
            0x0080_80ff,
            gamma,
        );
    }

    fn draw_scrollbar(
        surface: &mut Surface,
        assets: &NetDlgAssets,
        controller: &NetDlgController,
        layout: NetDlgLayout,
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
            &assets.gui_scroll,
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
                &assets.gui_scroll,
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
            &assets.gui_scroll,
            bottom_x,
            32,
            16,
            16,
            gamma,
        );
        if NetDlgController::scrollbar_has_pin(&layout) {
            crate::draw_image_strip(
                surface,
                bar.x,
                bar.y + SCROLLBAR_PART + controller.list_scroll_pin,
                &assets.gui_scroll,
                16,
                16,
                16,
                16,
                gamma,
            );
        }
    }

    /// One `C4GUI::IconButton`: focus/hover highlight behind the icon and a
    /// second additive pass over a pressed icon (C4GuiButton.cpp:205-232).
    fn icon_button(
        surface: &mut Surface,
        assets: &NetDlgAssets,
        fonts: &ClonkFontSet,
        gamma: Option<&GammaRamp>,
        rect: IntRect,
        src: (u32, u32),
        label: &str,
        highlight_image: Option<&ImageData>,
        highlighted: bool,
        pressed: bool,
    ) {
        if highlighted {
            if let Some(highlight) = highlight_image {
                crate::draw_image_bilinear_additive(surface, &gui_rect(rect), highlight, gamma);
            }
        }
        crate::draw_image_strip(
            surface,
            rect.x,
            rect.y,
            &assets.gui_icons_ex,
            src.0,
            src.1,
            64,
            64,
            gamma,
        );
        if pressed {
            if let Some(highlight) = highlight_image {
                crate::draw_image_bilinear_additive(surface, &gui_rect(rect), highlight, gamma);
            }
        }
        let (text, _) = expand_hotkey_markup(label);
        fonts.text.draw_with_gamma(
            surface,
            rect.x + rect.w / 2,
            rect.y + rect.h - fonts.text.line_height * 4 / 5,
            &text,
            CLR_WHITE,
            TextAlign::Center,
            true,
            gamma,
        );
    }

    fn draw_chat_sheet(
        surface: &mut Surface,
        fonts: &ClonkFontSet,
        classic_skin: &ClassicGuiSkin<'_>,
        gamma: Option<&GammaRamp>,
        layout: NetDlgLayout,
    ) {
        classic_skin.draw_caption(
            surface,
            layout.game_list_caption,
            "Chat",
            &fonts.text,
            CLR_YELLOW,
            TextAlign::Left,
            gamma,
        );
        let chat = IntRect {
            x: layout.game_list.x,
            y: layout.game_list.y,
            w: layout.game_list.w,
            h: layout.join_edit.y + layout.join_edit.h - layout.game_list.y,
        };
        draw_engine_box(
            surface,
            chat.x,
            chat.y,
            chat.x + chat.w - 1,
            chat.y + chat.h - 1,
            CLR_DARK_BG,
            gamma,
        );
        draw_3d_frame(surface, chat, gamma);
    }
}

#[cfg(test)]
mod tests {
    use std::sync::OnceLock;

    use super::*;
    use crate::test_support::endeavour_font_set;
    use lc_graphics::{Color, PixelFormat};

    /// The two text extents the C++ constructor measures from the live fonts
    /// (C4StartupNetDlg.cpp:636,685-686), pinned to the spec's values.
    #[test]
    fn font_metrics_match_cpp_measured_extents() {
        let metrics = NetDlgFontMetrics::from_fonts(&endeavour_font_set());
        assert_eq!(
            metrics,
            NetDlgFontMetrics {
                caption_back_extent: 51,
                text_ip_extent: 18,
                text_line_height: 22,
                caption_line_height: 25,
                title_line_height: 34,
            }
        );
    }

    /// Pixel-exact C4StartupNetDlg geometry at 1280x720, derived from
    /// C4StartupNetDlg.cpp:631-728 (ComponentAligner stacking),
    /// C4GuiDialogs.cpp:813-822,858-862 (fullscreen margins),
    /// C4GuiListBox.h:120-123 + C4GuiContainers.cpp:477-491 (list internals)
    /// and C4StartupNetDlg.cpp:39-81 (entry geometry); all rects verified
    /// against an F9 screenshot of the C++ engine.
    #[test]
    fn layout_matches_cpp_net_dlg_at_1280x720() {
        let metrics = NetDlgFontMetrics {
            caption_back_extent: 51,
            text_ip_extent: 18,
            text_line_height: 22,
            caption_line_height: 25,
            title_line_height: 34,
        };
        let layout = net_dlg_layout(1280, 720, &metrics);

        let rect = |x, y, w, h| IntRect { x, y, w, h };
        assert_eq!(layout.client, rect(25, 69, 1230, 632));
        assert_eq!(layout.title_anchor, (640, 8));
        assert_eq!(layout.btn_game_list, rect(99, 82, 64, 64));
        assert_eq!(layout.btn_chat, rect(99, 172, 64, 64));
        assert_eq!(layout.btn_internet, rect(1116, 82, 64, 64));
        assert_eq!(layout.btn_record, rect(1116, 172, 64, 64));
        assert_eq!(layout.game_list_caption, rect(238, 69, 804, 23));
        assert_eq!(layout.game_list, rect(238, 92, 804, 496));
        assert_eq!(layout.list_client, rect(241, 95, 798, 490));
        assert_eq!(layout.list_viewport, rect(241, 95, 782, 490));
        assert_eq!(layout.list_scrollbar, rect(1023, 95, 16, 490));
        assert_eq!(layout.list_entry, rect(241, 95, 782, 48));
        assert_eq!(layout.entry_icon, rect(241, 100, 48, 38));
        assert_eq!(layout.entry_labels[0], rect(292, 96, 730, 22));
        assert_eq!(layout.entry_labels[1], rect(292, 120, 730, 22));
        assert_eq!(layout.ip_label, rect(238, 588, 28, 23));
        assert_eq!(layout.join_edit, rect(266, 588, 776, 23));
        let xs = [160, 429, 698, 967];
        for (i, &x) in xs.iter().enumerate() {
            assert_eq!(layout.buttons[i], rect(x, 640, 153, 32), "button {i}");
        }
    }

    fn metrics() -> NetDlgFontMetrics {
        NetDlgFontMetrics {
            caption_back_extent: 51,
            text_ip_extent: 18,
            text_line_height: 22,
            caption_line_height: 25,
            title_line_height: 34,
        }
    }

    fn games(count: usize) -> Vec<NetDlgGameEntry> {
        (0..count)
            .map(|index| NetDlgGameEntry {
                title: format!("Game {index:02}"),
                details: format!("Lobby {index:02} — Host"),
                address: Some(format!("203.0.113.{index}:11112")),
                joinable: true,
                ..NetDlgGameEntry::default()
            })
            .collect()
    }

    fn center(rect: IntRect) -> crate::GuiPoint {
        crate::GuiPoint::new((rect.x + rect.w / 2) as f32, (rect.y + rect.h / 2) as f32)
    }

    fn text_font() -> &'static ClonkFont {
        static FONT: OnceLock<ClonkFont> = OnceLock::new();
        FONT.get_or_init(|| endeavour_font_set().text.clone())
    }

    fn click(controller: &mut NetDlgController, rect: IntRect) -> Vec<NetDlgAction> {
        let point = center(rect);
        assert!(controller
            .handle_pointer_down(point, text_font())
            .is_empty());
        controller.handle_pointer_up(point, text_font())
    }

    fn net_assets() -> NetDlgAssets {
        let load = crate::test_support::load_graphics_png;
        NetDlgAssets {
            background: load("StartupNetworkBG.png"),
            net_get_ref: load("StartupNetGetRef.png"),
            scen_icons: load("StartupScenSelIcons.png"),
            gui_caption: load("GUICaption.png"),
            gui_button: load("GUIButton.png"),
            gui_button_down: load("GUIButtonDown.png"),
            gui_button_highlight: load("GUIButtonHighlight.png"),
            gui_scroll: load("GUIScroll.png"),
            gui_icons: load("GUIIcons.png"),
            gui_icons_ex: load("GUIIcons2.png"),
        }
    }

    fn rich_game(index: usize) -> NetDlgGameEntry {
        NetDlgGameEntry {
            title: format!("Round {index} on Host"),
            details: "2/4 players - Capture the flag - Running - 01:02:03".into(),
            extra_lines: vec![
                "Engine version: 4.9.11.0 [362]".into(),
                "Comment: Exact reference presentation".into(),
                "Player: Alice, Bob".into(),
            ],
            address: Some(format!("203.0.113.{index}:11112")),
            joinable: true,
            ..NetDlgGameEntry::default()
        }
    }

    #[test]
    fn resolved_row_wraps_five_info_lines_and_maps_every_status_icon() {
        let fonts = endeavour_font_set();
        let mut controller = NetDlgController::new(
            NetDlgConfig {
                masterserver_signup: false,
                ..NetDlgConfig::default()
            },
            metrics(),
        );
        controller.set_text_font(&fonts.text);
        controller.resize(1280, 720);
        let statuses = vec![
            NetDlgStatusIcon::PasswordNeeded,
            NetDlgStatusIcon::League,
            NetDlgStatusIcon::LobbyActive,
            NetDlgStatusIcon::Running,
            NetDlgStatusIcon::RuntimeJoin,
            NetDlgStatusIcon::FairCrew,
            NetDlgStatusIcon::OfficialServer,
        ];
        let mut game = rich_game(0);
        game.extra_lines[2] = format!("Player: {}", "Long player name ".repeat(80));
        game.status_icons = statuses.clone();
        controller.set_games(vec![game]);

        let layout = controller.layout();
        let rows = controller.row_layouts(&layout);
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].lines.len(), 5);
        assert!(rows[0].lines[4].rect.h > metrics().text_line_height);
        assert_eq!(rows[0].status_icons, statuses);
        assert_eq!(
            rows[0].lines[0].rect.w,
            layout.entry_labels[0].w - 7 * metrics().text_line_height
        );
        assert_eq!(
            rows[0]
                .status_icons
                .iter()
                .copied()
                .map(NetDlgStatusIcon::source)
                .collect::<Vec<_>>(),
            vec![
                (true, 13),
                (true, 8),
                (false, 31),
                (false, 30),
                (false, 32),
                (true, 2),
                (false, 44),
            ]
        );

        let assets = net_assets();
        let mut surface = Surface::new(1280, 720, PixelFormat::Rgba8888);
        surface.begin_clonk_text_capture();
        NetDlgScreen::render_controller(&mut surface, &assets, &fonts, None, &controller, 0);
        let row_text = surface
            .take_clonk_text_capture()
            .into_iter()
            .map(|command| command.text)
            .filter(|text| {
                text.starts_with("Round ")
                    || text.starts_with("2/4 players")
                    || text.starts_with("Engine version")
                    || text.starts_with("Comment:")
                    || text.starts_with("Player:")
            })
            .collect::<Vec<_>>();
        assert_eq!(row_text.len(), 5);
        assert!(row_text[4].contains('\n'));
    }

    #[test]
    fn overflowing_expanded_rows_collapse_except_selection_and_remain_hittable() {
        let fonts = endeavour_font_set();
        let mut controller = NetDlgController::new(
            NetDlgConfig {
                masterserver_signup: false,
                ..NetDlgConfig::default()
            },
            metrics(),
        );
        controller.set_text_font(&fonts.text);
        controller.resize(1280, 720);
        controller.set_games((0..6).map(rich_game).collect());
        let layout = controller.layout();

        assert!(controller.list_is_collapsed());
        assert_eq!(
            controller
                .row_layouts(&layout)
                .iter()
                .map(|row| row.rect.h)
                .collect::<Vec<_>>(),
            vec![48; 6]
        );
        assert_eq!(controller.list_max_scroll(), 0);

        assert!(controller.focus_game(2).is_empty());
        let rows = controller.row_layouts(&layout);
        assert_eq!(
            rows.iter().map(|row| row.rect.h).collect::<Vec<_>>(),
            vec![48, 48, 120, 48, 48, 48]
        );
        for (index, row) in rows.iter().enumerate() {
            assert_eq!(
                controller.game_index_at(GuiPoint::new(
                    (row.rect.x + 2) as f32,
                    (row.rect.y + row.rect.h - 2) as f32,
                )),
                Some(index)
            );
        }

        controller.set_games((0..10).map(rich_game).collect());
        assert!(controller.list_max_scroll() > 0);
        assert!(controller.focus_game(9).is_empty());
        assert_eq!(controller.selected_game(), Some(9));
        assert_eq!(
            controller.list_scroll_offset(),
            controller.list_max_scroll()
        );

        let mut with_master = NetDlgController::new(NetDlgConfig::default(), metrics());
        with_master.set_text_font(&fonts.text);
        with_master.resize(1280, 720);
        with_master.set_masterserver_entry(NetDlgMasterserverEntry {
            title: "Masterserver".into(),
            details: "Six games".into(),
            extra_lines: vec![
                NetDlgTextLine::Plain("MOTD".into()),
                NetDlgTextLine::Plain("News".into()),
                NetDlgTextLine::Plain("Status".into()),
            ],
            row_icon: NetDlgRowIcon::None,
        });
        with_master.set_games((0..6).map(rich_game).collect());
        let master_layout = with_master.layout();
        assert_eq!(with_master.row_layouts(&master_layout)[0].rect.h, 120);
        assert!(with_master.focus_game(2).is_empty());
        let selected_rows = with_master.row_layouts(&master_layout);
        assert_eq!(
            selected_rows[0].rect.h, 48,
            "selected game collapses master"
        );
        assert_eq!(selected_rows[3].rect.h, 120, "selected game stays expanded");
    }

    #[test]
    fn native_row_top_spacing_drives_collapse_scroll_and_gap_hit_testing() {
        let mut controller = NetDlgController::new(
            NetDlgConfig {
                masterserver_signup: false,
                ..NetDlgConfig::default()
            },
            metrics(),
        );
        controller.resize(1280, 720);
        controller.set_games(games(10));
        let layout = controller.layout();

        // Fully expanded this is 10*48 + 9*10 = 570, so a 490px viewport
        // enters collapsed mode. Collapsed it is 10*48 + 9*5 = 525.
        assert_eq!(layout.list_viewport.h, 490);
        assert!(controller.list_is_collapsed());
        assert_eq!(controller.list_max_scroll(), 35);
        let rows = controller.row_layouts(&layout);
        assert_eq!(rows[1].rect.y - (rows[0].rect.y + rows[0].rect.h), 5);

        let gap = GuiPoint::new(
            (layout.list_viewport.x + 4) as f32,
            (rows[0].rect.y + rows[0].rect.h + 2) as f32,
        );
        assert_eq!(controller.game_index_at(gap), None);
        assert!(controller
            .handle_pointer_down(gap, text_font())
            .is_empty());
        assert_eq!(controller.selected_game(), None);

        assert!(controller.focus_game(1).is_empty());
        let selected_rows = controller.row_layouts(&layout);
        assert_eq!(
            selected_rows[1].rect.y - (selected_rows[0].rect.y + selected_rows[0].rect.h),
            10,
            "an expanded selected row uses the native 10px top spacing"
        );
        assert_eq!(
            selected_rows[2].rect.y - (selected_rows[1].rect.y + selected_rows[1].rect.h),
            5,
            "the following collapsed row uses the native 5px top spacing"
        );
    }

    #[test]
    fn masterserver_reply_renders_count_motd_and_link_and_link_activates() {
        let fonts = endeavour_font_set();
        let mut controller = NetDlgController::new(NetDlgConfig::default(), metrics());
        controller.set_text_font(&fonts.text);
        controller.resize(1280, 720);
        controller.set_masterserver_entry(NetDlgMasterserverEntry {
            title: "Internet server on league.example".into(),
            details: "3 game(s) found, 7 players online.".into(),
            extra_lines: vec![
                NetDlgTextLine::Plain("Message of the day: Welcome".into()),
                NetDlgTextLine::Hyperlink {
                    label: "https://league.example/news".into(),
                    url: "https://league.example/news".into(),
                },
            ],
            row_icon: NetDlgRowIcon::None,
        });

        let layout = controller.layout();
        let rows = controller.row_layouts(&layout);
        assert_eq!(rows[0].rect.h, 96);
        assert_eq!(rows[0].lines.len(), 4);
        let link = rows[0].lines[3].rect;
        assert_eq!(
            controller.handle_pointer_down(
                GuiPoint::new((link.x + 2) as f32, (link.y + 2) as f32),
                text_font(),
            ),
            vec![NetDlgAction::OpenUrl("https://league.example/news".into())]
        );

        let assets = net_assets();
        let mut surface = Surface::new(1280, 720, PixelFormat::Rgba8888);
        surface.begin_clonk_text_capture();
        NetDlgScreen::render_controller(&mut surface, &assets, &fonts, None, &controller, 0);
        let captured = surface
            .take_clonk_text_capture()
            .into_iter()
            .map(|command| command.text)
            .collect::<Vec<_>>();
        for expected in [
            "Internet server on league.example",
            "3 game(s) found, 7 players online.",
            "Message of the day: Welcome",
            "https://league.example/news",
        ] {
            assert!(captured.iter().any(|text| text == expected), "{expected}");
        }
    }

    #[test]
    fn l044_hyperlink_uses_cpp_color_exact_underline_and_only_link_opens() {
        let fonts = endeavour_font_set();
        let mut controller = NetDlgController::new(NetDlgConfig::default(), metrics());
        controller.set_text_font(&fonts.text);
        controller.resize(1280, 720);
        controller.set_masterserver_entry(NetDlgMasterserverEntry {
            title: "Internet server on league.example".into(),
            details: "3 game(s) found, 7 players online.".into(),
            extra_lines: vec![
                NetDlgTextLine::Plain("Message of the day: Welcome".into()),
                NetDlgTextLine::Hyperlink {
                    label: "https://league.example/news".into(),
                    url: "https://league.example/news".into(),
                },
            ],
            row_icon: NetDlgRowIcon::None,
        });
        controller.set_games(vec![NetDlgGameEntry {
            title: "Wrong version".into(),
            details: "Engine version: 4.9.11.0 [363]".into(),
            joinable: false,
            ..NetDlgGameEntry::default()
        }]);

        let layout = controller.layout();
        let rows = controller.row_layouts(&layout);
        let link_rect = rows[0].lines[3].rect;
        let ordinary_rect = rows[1].lines[0].rect;
        let ordinary_actions = controller.handle_pointer_down(
            GuiPoint::new((ordinary_rect.x + 2) as f32, (ordinary_rect.y + 2) as f32),
            text_font(),
        );
        assert!(!ordinary_actions
            .iter()
            .any(|action| matches!(action, NetDlgAction::OpenUrl(_))));
        assert_eq!(
            controller.handle_pointer_down(
                GuiPoint::new((link_rect.x + 2) as f32, (link_rect.y + 2) as f32),
                text_font(),
            ),
            vec![NetDlgAction::OpenUrl("https://league.example/news".into())]
        );

        let assets = net_assets();
        let mut surface = Surface::new(1280, 720, PixelFormat::Rgba8888);
        surface.begin_clonk_text_capture();
        NetDlgScreen::render_controller(&mut surface, &assets, &fonts, None, &controller, 0);
        let captured = surface.take_clonk_text_capture();
        let color_for = |text: &str| {
            captured
                .iter()
                .find(|command| command.text == text)
                .unwrap_or_else(|| panic!("missing captured text: {text}"))
                .color
        };
        for text in [
            "Internet server on league.example",
            "3 game(s) found, 7 players online.",
            "Message of the day: Welcome",
        ] {
            assert_eq!(color_for(text), CLR_WHITE, "{text}");
        }
        assert_eq!(color_for("https://league.example/news"), CLR_HYPERLINK);
        for text in ["Wrong version", "Engine version: 4.9.11.0 [363]"] {
            assert_eq!(color_for(text), CLR_DISABLED, "{text}");
        }

        let (width, height) = fonts.text.measure("https://league.example/news", true);
        let underline_y = link_rect.y + height - 2;
        let hyperlink_pixel = Some(Color::opaque(0x80, 0x80, 0xff));
        assert!((link_rect.x..link_rect.x + width)
            .all(|x| { surface.get_pixel(x as u32, underline_y as u32) == hyperlink_pixel }));
        assert_ne!(
            surface.get_pixel((link_rect.x + width) as u32, underline_y as u32),
            hyperlink_pixel,
            "the engine line endpoint is excluded, so the underline is exactly text-width pixels"
        );
    }

    #[test]
    fn official_unselected_row_uses_important_background_but_selection_wins() {
        let fonts = endeavour_font_set();
        let assets = net_assets();
        let config = NetDlgConfig {
            masterserver_signup: false,
            ..NetDlgConfig::default()
        };
        let render = |official: bool, selected: bool| {
            let mut controller = NetDlgController::new(config, metrics());
            controller.set_text_font(&fonts.text);
            controller.resize(1280, 720);
            let mut game = rich_game(0);
            if official {
                game.status_icons.push(NetDlgStatusIcon::OfficialServer);
            }
            controller.set_games(vec![game]);
            if selected {
                let _ = controller.focus_game(0);
            }
            let mut surface = Surface::new(1280, 720, PixelFormat::Rgba8888);
            NetDlgScreen::render_controller(&mut surface, &assets, &fonts, None, &controller, 0);
            surface
        };
        let layout = net_dlg_layout(1280, 720, &metrics());
        let sample = (layout.list_entry.x + 2, layout.list_entry.y + 2);
        let ordinary = render(false, false);
        let official = render(true, false);
        assert_ne!(
            ordinary.get_pixel(sample.0 as u32, sample.1 as u32),
            official.get_pixel(sample.0 as u32, sample.1 as u32)
        );
        let ordinary_selected = render(false, true);
        let official_selected = render(true, true);
        assert_eq!(
            ordinary_selected.get_pixel(sample.0 as u32, sample.1 as u32),
            official_selected.get_pixel(sample.0 as u32, sample.1 as u32)
        );
    }

    #[test]
    fn row_icons_use_native_query_error_and_scenario_sources() {
        let fonts = endeavour_font_set();
        let assets = net_assets();
        let config = NetDlgConfig {
            masterserver_signup: false,
            ..NetDlgConfig::default()
        };
        let render = |icon, phase| {
            let mut controller = NetDlgController::new(config, metrics());
            controller.set_text_font(&fonts.text);
            controller.resize(1280, 720);
            controller.set_games(vec![NetDlgGameEntry {
                title: "Direct join on example.test".into(),
                details: "Query status".into(),
                row_icon: icon,
                ..NetDlgGameEntry::default()
            }]);
            let mut surface = Surface::new(1280, 720, PixelFormat::Rgba8888);
            NetDlgScreen::render_controller(
                &mut surface,
                &assets,
                &fonts,
                None,
                &controller,
                phase,
            );
            surface
        };
        let query_0 = render(NetDlgRowIcon::Query, 0);
        let query_1 = render(NetDlgRowIcon::Query, 1);
        let query_static_0 = render(NetDlgRowIcon::QueryStatic, 0);
        let query_static_1 = render(NetDlgRowIcon::QueryStatic, 1);
        let error_0 = render(NetDlgRowIcon::Error, 0);
        let error_1 = render(NetDlgRowIcon::Error, 1);
        let scenario_0 = render(NetDlgRowIcon::Scenario(0), 0);
        let scenario_0_later = render(NetDlgRowIcon::Scenario(0), 1);
        let scenario_1 = render(NetDlgRowIcon::Scenario(1), 0);
        let scenario_default = render(NetDlgRowIcon::Scenario(14), 0);
        let scenario_negative = render(NetDlgRowIcon::Scenario(-1), 0);
        let scenario_too_large = render(NetDlgRowIcon::Scenario(52), 0);
        let no_icon = render(NetDlgRowIcon::None, 0);
        let icon = net_dlg_layout(1280, 720, &metrics()).list_entry;
        let icon_pixels = |surface: &Surface| {
            (icon.y..icon.y + icon.h)
                .flat_map(|y| (icon.x..icon.x + icon.h).map(move |x| (x, y)))
                .map(|(x, y)| surface.get_pixel(x as u32, y as u32))
                .collect::<Vec<_>>()
        };
        assert_ne!(icon_pixels(&query_0), icon_pixels(&query_1));
        assert_eq!(icon_pixels(&query_static_0), icon_pixels(&query_0));
        assert_eq!(icon_pixels(&query_static_0), icon_pixels(&query_static_1));
        assert_ne!(icon_pixels(&query_0), icon_pixels(&error_0));
        assert_eq!(icon_pixels(&error_0), icon_pixels(&error_1));
        assert_ne!(icon_pixels(&scenario_0), icon_pixels(&scenario_1));
        assert_eq!(icon_pixels(&scenario_0), icon_pixels(&scenario_0_later));
        assert_eq!(
            icon_pixels(&scenario_default),
            icon_pixels(&scenario_negative)
        );
        assert_eq!(
            icon_pixels(&scenario_default),
            icon_pixels(&scenario_too_large)
        );

        let small_icon = IntRect {
            x: icon.x + 8,
            y: icon.y + 8,
            w: 32,
            h: 32,
        };
        let changed = no_icon
            .pixels()
            .chunks_exact(4)
            .zip(scenario_0.pixels().chunks_exact(4))
            .enumerate()
            .filter_map(|(index, (none, scenario))| (none != scenario).then_some(index))
            .collect::<Vec<_>>();
        assert!(!changed.is_empty());
        assert!(changed.into_iter().all(|index| {
            let x = i32::try_from(index % 1280).unwrap();
            let y = i32::try_from(index / 1280).unwrap();
            x >= small_icon.x
                && x < small_icon.x + small_icon.w
                && y >= small_icon.y
                && y < small_icon.y + small_icon.h
        }));
    }

    #[test]
    fn disabled_reference_rows_use_native_inactive_message_color() {
        let fonts = endeavour_font_set();
        let assets = net_assets();
        let mut controller = NetDlgController::new(
            NetDlgConfig {
                masterserver_signup: false,
                ..NetDlgConfig::default()
            },
            metrics(),
        );
        controller.set_text_font(&fonts.text);
        controller.resize(1280, 720);
        controller.set_games(vec![NetDlgGameEntry {
            title: "Wrong version".into(),
            details: "Engine version: 4.9.11.0 [363]".into(),
            joinable: false,
            ..NetDlgGameEntry::default()
        }]);
        let mut surface = Surface::new(1280, 720, PixelFormat::Rgba8888);
        surface.begin_clonk_text_capture();
        NetDlgScreen::render_controller(&mut surface, &assets, &fonts, None, &controller, 0);
        let rows = surface
            .take_clonk_text_capture()
            .into_iter()
            .filter(|command| {
                command.text == "Wrong version" || command.text == "Engine version: 4.9.11.0 [363]"
            })
            .collect::<Vec<_>>();
        assert_eq!(rows.len(), 2);
        assert!(rows.iter().all(|command| command.color == CLR_DISABLED));
        assert_eq!(CLR_DISABLED, [0xaf, 0xaf, 0xaf, 0xff]);
    }

    // Callback buttons invoke their C4StartupNetDlg handlers only after a
    // left-down/up pair inside the same half-open C4Rect
    // (C4GuiButton.cpp:128-155; C4Rect.h:40-43).
    #[test]
    fn controller_routes_every_visible_button_and_uses_half_open_hits() {
        let mut controller = NetDlgController::new(NetDlgConfig::default(), metrics());
        controller.resize(1280, 720);
        let layout = net_dlg_layout(1280, 720, &metrics());

        assert_eq!(
            click(&mut controller, layout.buttons[0]),
            vec![NetDlgAction::Back]
        );
        assert_eq!(
            click(&mut controller, layout.buttons[1]),
            vec![NetDlgAction::Refresh]
        );

        controller.set_join_address(" 127.0.0.1:11111 ");
        assert_eq!(
            click(&mut controller, layout.buttons[2]),
            vec![NetDlgAction::JoinGame { address: None }],
            "a Join button click must not reinterpret an unfocused edit as a direct query"
        );
        assert_eq!(
            controller.handle_pointer_down(center(layout.join_edit), text_font()),
            vec![NetDlgAction::FocusChanged(NetDlgControl::JoinAddress)]
        );
        assert_eq!(
            click(&mut controller, layout.buttons[2]),
            vec![
                NetDlgAction::QueryAddress {
                    address: " 127.0.0.1:11111 ".into()
                },
                NetDlgAction::FocusChanged(NetDlgControl::GameList),
            ]
        );
        assert_eq!(
            click(&mut controller, layout.buttons[3]),
            vec![NetDlgAction::CreateGame]
        );

        assert_eq!(
            click(&mut controller, layout.btn_internet),
            vec![NetDlgAction::MasterserverSignupChanged(false)]
        );
        assert!(!controller.config().masterserver_signup);
        assert_eq!(
            click(&mut controller, layout.btn_record),
            vec![NetDlgAction::RecordingChanged(true)]
        );
        assert!(controller.config().record);
        assert_eq!(
            click(&mut controller, layout.btn_chat),
            vec![
                NetDlgAction::ModeChanged(NetDlgMode::Chat),
                NetDlgAction::FocusChanged(NetDlgControl::ChatInput),
            ]
        );
        assert_eq!(
            click(&mut controller, layout.btn_game_list),
            vec![
                NetDlgAction::ModeChanged(NetDlgMode::GameList),
                NetDlgAction::FocusChanged(NetDlgControl::GameList),
            ]
        );

        let outside = crate::GuiPoint::new(
            (layout.buttons[0].x + layout.buttons[0].w) as f32,
            layout.buttons[0].y as f32,
        );
        assert!(controller
            .handle_pointer_down(outside, text_font())
            .is_empty());
        assert!(controller
            .handle_pointer_up(outside, text_font())
            .is_empty());
    }

    #[test]
    fn mode_switch_preserves_standalone_button_focus_and_structural_tab_order() {
        let mut controller = NetDlgController::new(NetDlgConfig::default(), metrics());
        controller.resize(1280, 720);
        let layout = net_dlg_layout(1280, 720, &metrics());
        for _ in 0..9 {
            controller.handle_key_down(crate::KeyCode::Tab);
        }
        assert_eq!(controller.focused_control(), NetDlgControl::ChatButton);

        assert!(controller.handle_key_down(crate::KeyCode::Space).is_empty());
        assert_eq!(
            controller.handle_key_up(crate::KeyCode::Space),
            vec![NetDlgAction::ModeChanged(NetDlgMode::Chat)]
        );
        assert_eq!(controller.focused_control(), NetDlgControl::ChatButton);

        let mut hidden_button_focus = NetDlgController::new(NetDlgConfig::default(), metrics());
        hidden_button_focus.resize(1280, 720);
        for _ in 0..5 {
            hidden_button_focus.handle_key_down(crate::KeyCode::Tab);
        }
        assert_eq!(
            hidden_button_focus.focused_control(),
            NetDlgControl::Refresh
        );
        assert_eq!(
            click(&mut hidden_button_focus, layout.btn_chat),
            vec![NetDlgAction::ModeChanged(NetDlgMode::Chat)]
        );
        assert_eq!(
            hidden_button_focus.focused_control(),
            NetDlgControl::Refresh
        );
        assert!(hidden_button_focus
            .handle_key_down(crate::KeyCode::Enter)
            .is_empty());
        assert_eq!(
            hidden_button_focus.handle_key_up(crate::KeyCode::Enter),
            vec![NetDlgAction::Refresh]
        );
        assert!(hidden_button_focus
            .handle_key_down(crate::KeyCode::Space)
            .is_empty());
        assert_eq!(
            hidden_button_focus.handle_key_up(crate::KeyCode::Space),
            vec![NetDlgAction::Refresh]
        );
        assert_eq!(
            hidden_button_focus.handle_key_down(crate::KeyCode::Tab),
            vec![NetDlgAction::FocusChanged(NetDlgControl::CreateGame)]
        );

        let mut internet_focus = NetDlgController::new(NetDlgConfig::default(), metrics());
        internet_focus.resize(1280, 720);
        for _ in 0..2 {
            internet_focus.handle_key_down(crate::KeyCode::Tab);
        }
        assert_eq!(internet_focus.focused_control(), NetDlgControl::Internet);
        assert_eq!(
            click(&mut internet_focus, layout.btn_chat),
            vec![NetDlgAction::ModeChanged(NetDlgMode::Chat)]
        );
        assert_eq!(
            internet_focus.handle_key_down(crate::KeyCode::Tab),
            vec![NetDlgAction::FocusChanged(NetDlgControl::Back)]
        );

        let mut hidden_join_focus = NetDlgController::new(NetDlgConfig::default(), metrics());
        hidden_join_focus.resize(1280, 720);
        for _ in 0..6 {
            hidden_join_focus.handle_key_down(crate::KeyCode::Tab);
        }
        assert_eq!(hidden_join_focus.focused_control(), NetDlgControl::JoinGame);
        assert_eq!(
            click(&mut hidden_join_focus, layout.btn_chat),
            vec![NetDlgAction::ModeChanged(NetDlgMode::Chat)]
        );
        assert!(hidden_join_focus
            .handle_key_down(crate::KeyCode::Enter)
            .is_empty());
        assert!(hidden_join_focus
            .handle_key_up(crate::KeyCode::Enter)
            .is_empty());
        assert!(hidden_join_focus
            .handle_key_down(crate::KeyCode::Space)
            .is_empty());
        assert!(hidden_join_focus
            .handle_key_up(crate::KeyCode::Space)
            .is_empty());
    }

    #[test]
    fn shift_tab_reverses_game_list_focus_order() {
        let mut controller = NetDlgController::new(NetDlgConfig::default(), metrics());
        assert_eq!(controller.focused_control(), NetDlgControl::GameList);
        assert_eq!(
            controller.handle_key_down_with_tab_direction(crate::KeyCode::Tab, true),
            vec![NetDlgAction::FocusChanged(NetDlgControl::ChatButton)]
        );
        assert_eq!(
            controller.handle_key_down(crate::KeyCode::Tab),
            vec![NetDlgAction::FocusChanged(NetDlgControl::GameList)]
        );

        assert_eq!(
            controller.handle_key_down(crate::KeyCode::Tab),
            vec![NetDlgAction::FocusChanged(NetDlgControl::JoinAddress)]
        );
        assert_eq!(
            controller.handle_key_down(crate::KeyCode::Tab),
            vec![NetDlgAction::FocusChanged(NetDlgControl::Internet)]
        );
        assert_eq!(
            controller.handle_key_down_with_tab_direction(crate::KeyCode::Tab, true),
            vec![NetDlgAction::FocusChanged(NetDlgControl::JoinAddress)]
        );
        assert_eq!(
            controller.handle_key_down(crate::KeyCode::Tab),
            vec![NetDlgAction::FocusChanged(NetDlgControl::Internet)]
        );
    }

    #[test]
    fn direct_join_edit_is_a_two_step_query_then_selected_row_join() {
        let mut controller = NetDlgController::new(NetDlgConfig::default(), metrics());
        controller.resize(1280, 720);
        let layout = net_dlg_layout(1280, 720, &metrics());
        controller.set_join_address("   ");

        assert_eq!(
            controller.handle_key_down(crate::KeyCode::Enter),
            vec![NetDlgAction::JoinGame { address: None }]
        );
        assert_eq!(
            click(&mut controller, layout.buttons[2]),
            vec![NetDlgAction::JoinGame { address: None }]
        );
        assert_eq!(
            controller.handle_pointer_down(center(layout.join_edit), text_font()),
            vec![NetDlgAction::FocusChanged(NetDlgControl::JoinAddress)]
        );
        assert_eq!(
            controller.handle_key_down(crate::KeyCode::Enter),
            vec![
                NetDlgAction::QueryAddress {
                    address: "   ".into()
                },
                NetDlgAction::FocusChanged(NetDlgControl::GameList),
            ]
        );
        assert_eq!(controller.focused_control(), NetDlgControl::GameList);
        assert_eq!(controller.join_address(), "   ");

        // The application materializes and selects the direct-query row.
        controller.set_games(vec![NetDlgGameEntry {
            title: "Direct query".into(),
            details: "Querying".into(),
            address: Some("example.test".into()),
            joinable: false,
            ..NetDlgGameEntry::default()
        }]);
        assert!(controller.focus_game(0).is_empty());
        assert_eq!(
            controller.handle_key_down(crate::KeyCode::Enter),
            vec![NetDlgAction::JoinGame {
                address: Some("example.test".into())
            }]
        );
    }

    // The game list owns initial focus; Tab advances into the IP edit, whose
    // Left key edits rather than firing StartupNetBack. A focused button uses
    // C4GUI::Button's down/up key pair (C4StartupNetDlg.cpp:624-629,734;
    // C4GuiDialogs.cpp:343-357,616-644; C4GuiButton.cpp:22-35,112-126).
    #[test]
    fn controller_matches_initial_focus_and_keyboard_activation() {
        let mut controller = NetDlgController::new(NetDlgConfig::default(), metrics());
        controller.resize(1280, 720);
        assert_eq!(controller.focused_control(), NetDlgControl::GameList);

        assert_eq!(
            controller.handle_key_down(crate::KeyCode::Enter),
            vec![NetDlgAction::JoinGame { address: None }]
        );
        assert_eq!(
            controller.handle_key_down(crate::KeyCode::Tab),
            vec![NetDlgAction::FocusChanged(NetDlgControl::JoinAddress)]
        );
        assert!(controller.handle_key_down(crate::KeyCode::Left).is_empty());

        assert_eq!(
            controller.handle_key_down(crate::KeyCode::Tab),
            vec![NetDlgAction::FocusChanged(NetDlgControl::Internet)]
        );
        assert!(controller.handle_key_down(crate::KeyCode::Space).is_empty());
        assert_eq!(
            controller.handle_key_up(crate::KeyCode::Space),
            vec![NetDlgAction::MasterserverSignupChanged(false)]
        );
        assert_eq!(
            controller.handle_key_down(crate::KeyCode::Escape),
            vec![NetDlgAction::Back]
        );
    }

    #[test]
    fn join_edit_keyboard_matches_cpp_caret_selection_words_and_char_in() {
        let mut controller = NetDlgController::new(NetDlgConfig::default(), metrics());
        controller.resize(1280, 720);
        controller.set_join_address("alpha beta");
        assert_eq!(
            controller.handle_key_down(KeyCode::Tab),
            vec![NetDlgAction::FocusChanged(NetDlgControl::JoinAddress)]
        );
        assert_eq!(controller.join_address_selection(), Some((0, 10)));

        assert_eq!(
            controller.handle_text_input("x|\n\u{7f}", text_font()),
            vec![NetDlgAction::JoinAddressChanged("x\u{a6}".into())]
        );
        assert_eq!(controller.join_address(), "x\u{a6}");
        assert_eq!(controller.join_address_caret(), "x\u{a6}".len());
        assert_eq!(controller.join_address_selection(), None);

        controller.set_join_address("one,_two élan");
        let key = |controller: &mut NetDlgController, key, shift, control| {
            controller.handle_edit_key_down(
                key,
                NetDlgEditModifiers { shift, control },
                text_font(),
            )
        };
        assert!(key(&mut controller, NetDlgEditKey::Home, false, false).captured);
        assert_eq!(controller.join_address_caret(), 0);
        key(&mut controller, NetDlgEditKey::Right, false, true);
        assert_eq!(controller.join_address_caret(), 4, "Ctrl+Right skips comma");
        key(&mut controller, NetDlgEditKey::Right, false, true);
        assert_eq!(
            controller.join_address_caret(),
            9,
            "underscore is a word char"
        );
        key(&mut controller, NetDlgEditKey::Right, true, false);
        assert_eq!(controller.join_address_selection(), Some((9, 11)));
        assert_eq!(controller.join_edit.selected_text(), Some("é"));
        assert_eq!(
            key(&mut controller, NetDlgEditKey::Delete, false, false).actions,
            vec![NetDlgAction::JoinAddressChanged("one,_two lan".into())]
        );

        controller.set_join_address("abcd");
        key(&mut controller, NetDlgEditKey::Left, true, false);
        assert_eq!(controller.join_address_selection(), Some((3, 4)));
        key(&mut controller, NetDlgEditKey::Left, false, false);
        assert_eq!(
            controller.join_address_caret(),
            2,
            "plain movement clears a selection and still moves from its caret"
        );
        let unchanged = controller.join_address().to_string();
        key(&mut controller, NetDlgEditKey::Delete, true, false);
        assert_eq!(controller.join_address(), unchanged);
    }

    #[test]
    fn join_edit_pointer_midpoints_drag_and_double_click_match_cpp() {
        let mut controller = NetDlgController::new(NetDlgConfig::default(), metrics());
        controller.resize(1280, 720);
        controller.set_join_address("Wi");
        controller.handle_key_down(KeyCode::Tab);
        let edit = controller.layout().join_edit;
        let client = edit_client(edit);
        let first_width = text_font().measure("W", false).0;
        let midpoint = first_width - first_width / 2;
        let y = (edit.y + edit.h / 2) as f32;

        let tie = GuiPoint::new((client.x + midpoint) as f32, y);
        controller.handle_pointer_down(tie, text_font());
        assert_eq!(
            controller.join_address_caret(),
            0,
            "midpoint tie stays left"
        );
        controller.handle_pointer_up(tie, text_font());

        let right_half = GuiPoint::new((client.x + midpoint + 1) as f32, y);
        controller.handle_pointer_down(right_half, text_font());
        assert_eq!(controller.join_address_caret(), 1);
        controller.handle_pointer_up(right_half, text_font());

        let far_left = GuiPoint::new((edit.x - 100) as f32, y);
        let far_right = GuiPoint::new((edit.x + edit.w + 100) as f32, y);
        controller.handle_pointer_down(GuiPoint::new(client.x as f32, y), text_font());
        controller.handle_pointer_move(far_right, text_font());
        assert_eq!(controller.join_address_selection(), Some((0, 2)));
        controller.pointer_left();
        assert_eq!(controller.join_edit.drag_anchor, None);
        assert!(!controller.join_address_contains(far_left));
        assert!(controller.join_address_contains(tie));

        controller.set_join_address("alpha beta");
        let beta_x = client.x
            + text_font().measure("alpha ", false).0
            + text_font().measure("b", false).0 / 2;
        controller.handle_pointer_double_click(GuiPoint::new(beta_x as f32, y), text_font());
        assert_eq!(controller.join_edit.selected_text(), Some("beta"));
    }

    #[test]
    fn join_edit_mouse_selection_and_select_all_preserve_cpp_blink_phase() {
        let mut controller = NetDlgController::new(NetDlgConfig::default(), metrics());
        controller.resize(1280, 720);
        controller.set_join_address("alpha beta gamma");
        controller.focus = NetDlgControl::JoinAddress;
        let edit = controller.layout().join_edit;
        let stale = Instant::now() - std::time::Duration::from_millis(750);
        controller.join_edit.last_input = stale;
        controller.join_edit.horizontal_scroll = 13;

        controller.apply_context_command(
            NetDlgEditContextCommand::SelectAll,
            None,
            text_font(),
        );
        assert_eq!(controller.join_edit.last_input, stale);
        assert_eq!(controller.join_edit.horizontal_scroll, 13);

        let first = GuiPoint::new((edit_client(edit).x + 2) as f32, center(edit).y);
        controller.handle_pointer_down(first, text_font());
        assert_eq!(controller.join_edit.last_input, stale);
        controller.handle_pointer_move(
            GuiPoint::new((edit_client(edit).x + 80) as f32, center(edit).y),
            text_font(),
        );
        assert_eq!(controller.join_edit.last_input, stale);
        controller.handle_pointer_double_click(first, text_font());
        assert_eq!(controller.join_edit.last_input, stale);
    }

    #[test]
    fn join_edit_clipboard_context_middle_paste_and_multiline_abort_match_cpp() {
        let mut controller = NetDlgController::new(NetDlgConfig::default(), metrics());
        controller.resize(1280, 720);
        controller.set_join_address("copy me");
        controller.handle_key_down(KeyCode::Tab);

        let copy = controller.handle_clipboard_shortcut(
            NetDlgEditClipboardShortcut::Copy,
            None,
            text_font(),
        );
        assert!(copy.captured);
        assert_eq!(
            copy.actions,
            vec![NetDlgAction::ClipboardTransfer {
                text: "copy me".into(),
                cut: false,
            }]
        );
        let cut = controller.handle_clipboard_shortcut(
            NetDlgEditClipboardShortcut::Cut,
            None,
            text_font(),
        );
        assert_eq!(
            cut.actions,
            vec![NetDlgAction::ClipboardTransfer {
                text: "copy me".into(),
                cut: true,
            }]
        );
        assert_eq!(controller.join_address(), "copy me", "cut waits for host");
        assert_eq!(
            controller.confirm_clipboard_cut(text_font()),
            vec![NetDlgAction::JoinAddressChanged(String::new())]
        );

        controller.set_join_address("old");
        controller.apply_context_command(NetDlgEditContextCommand::SelectAll, None, text_font());
        assert_eq!(
            controller.apply_context_command(
                NetDlgEditContextCommand::Paste,
                Some("\r\nhost|name\nignored"),
                text_font(),
            ),
            vec![
                NetDlgAction::JoinAddressChanged("host\u{a6}name".into()),
                NetDlgAction::QueryAddress {
                    address: "host\u{a6}name".into(),
                },
                NetDlgAction::FocusChanged(NetDlgControl::GameList),
            ]
        );
        assert_eq!(controller.join_address(), "host\u{a6}name");

        controller.set_join_address("");
        let edit = controller.layout().join_edit;
        let middle = controller.handle_pointer_middle_down(
            center(edit),
            Some("raw|primary\ntext"),
            text_font(),
        );
        assert!(middle.captured);
        assert_eq!(controller.join_address(), "raw|primary\ntext");
        assert_eq!(controller.focused_control(), NetDlgControl::GameList);

        let context = controller.request_context_menu_at(center(edit), true);
        let [NetDlgAction::OpenJoinAddressContextMenu(request)] = context.actions.as_slice() else {
            panic!("join edit context request");
        };
        assert_eq!(
            request
                .items
                .iter()
                .map(|item| item.command)
                .collect::<Vec<_>>(),
            vec![
                NetDlgEditContextCommand::Paste,
                NetDlgEditContextCommand::SelectAll,
            ]
        );
        assert!(!controller.request_context_menu_from_key(true).captured);
    }

    #[test]
    fn join_edit_capacity_scroll_and_blink_transition_are_bounded() {
        let mut controller = NetDlgController::new(NetDlgConfig::default(), metrics());
        controller.resize(320, 240);
        controller.set_join_address("W".repeat(300));
        assert_eq!(controller.join_address().len(), JOIN_EDIT_MAX_PAYLOAD);
        controller.handle_key_down(KeyCode::Tab);
        controller.apply_context_command(NetDlgEditContextCommand::SelectAll, None, text_font());
        controller.handle_edit_key_down(
            NetDlgEditKey::End,
            NetDlgEditModifiers::default(),
            text_font(),
        );
        assert!(controller.join_address_horizontal_scroll() > 0);
        assert!(controller.tick_join_address_cursor());
        assert!(!controller.tick_join_address_cursor());
        controller.join_edit.last_input = Instant::now() - std::time::Duration::from_millis(501);
        assert!(controller.tick_join_address_cursor());
        assert!(!controller.join_address_cursor_visible());
    }

    #[test]
    fn discovered_rows_are_selectable_and_disabled_rows_remain_actionable() {
        let mut controller = NetDlgController::new(
            NetDlgConfig {
                masterserver_signup: false,
                ..NetDlgConfig::default()
            },
            metrics(),
        );
        controller.resize(1280, 720);
        controller.set_games(vec![
            NetDlgGameEntry {
                title: "Joinable game".into(),
                details: "Lobby — Host One".into(),
                address: Some("203.0.113.10:11112".into()),
                joinable: true,
                ..NetDlgGameEntry::default()
            },
            NetDlgGameEntry {
                title: "Wrong version".into(),
                details: "LegacyClonk 4.9.11.0 [363]".into(),
                address: Some("203.0.113.11:11112".into()),
                joinable: false,
                ..NetDlgGameEntry::default()
            },
        ]);

        assert_eq!(controller.selected_game(), None);
        assert!(controller.handle_key_down(crate::KeyCode::Down).is_empty());
        assert_eq!(controller.selected_game(), Some(0));
        assert_eq!(
            controller.handle_key_down(crate::KeyCode::Enter),
            vec![NetDlgAction::JoinGame {
                address: Some("203.0.113.10:11112".into())
            }]
        );
        assert!(controller.handle_key_down(crate::KeyCode::Down).is_empty());
        assert_eq!(controller.selected_game(), Some(1));
        assert_eq!(
            controller.handle_key_down(crate::KeyCode::Enter),
            vec![NetDlgAction::JoinGame {
                address: Some("203.0.113.11:11112".into())
            }]
        );
    }

    #[test]
    fn list_double_click_selects_focuses_and_joins_the_row_under_pointer() {
        let mut controller = NetDlgController::new(NetDlgConfig::default(), metrics());
        controller.resize(1280, 720);
        controller.set_games(vec![
            NetDlgGameEntry {
                title: "Joinable".into(),
                details: "Lobby".into(),
                address: Some("203.0.113.10:11112".into()),
                joinable: true,
                ..NetDlgGameEntry::default()
            },
            NetDlgGameEntry {
                title: "Runtime confirmation required".into(),
                details: "Running".into(),
                address: Some("203.0.113.11:11112".into()),
                joinable: false,
                ..NetDlgGameEntry::default()
            },
        ]);
        let layout = net_dlg_layout(1280, 720, &metrics());
        assert_eq!(
            controller.handle_pointer_down(center(layout.join_edit), text_font()),
            vec![NetDlgAction::FocusChanged(NetDlgControl::JoinAddress)]
        );

        // Row zero is the masterserver query; the second discovered game is
        // therefore visual row two.
        let second_game = controller
            .row_layouts(&layout)
            .into_iter()
            .find(|row| row.selection == NetDlgSelection::Game(1))
            .expect("second discovered game row");
        let point = GuiPoint::new(
            (second_game.rect.x + 4) as f32,
            (second_game.rect.y + 4) as f32,
        );
        assert_eq!(controller.game_index_at(point), Some(1));
        assert_eq!(
            controller.handle_pointer_double_click(point, text_font()),
            vec![
                NetDlgAction::FocusChanged(NetDlgControl::GameList),
                NetDlgAction::JoinGame {
                    address: Some("203.0.113.11:11112".into())
                },
            ]
        );
        assert_eq!(controller.selected_game(), Some(1));
        assert_eq!(controller.focused_control(), NetDlgControl::GameList);

        assert_eq!(
            controller.game_index_at(center(layout.list_entry)),
            None,
            "the masterserver query row is not a discovered game"
        );
    }

    #[test]
    fn overflowing_game_list_wheel_scrolls_clamps_and_hit_tests_content() {
        let mut controller = NetDlgController::new(
            NetDlgConfig {
                masterserver_signup: false,
                ..NetDlgConfig::default()
            },
            metrics(),
        );
        controller.resize(1280, 720);
        controller.set_games(games(20));
        let layout = net_dlg_layout(1280, 720, &metrics());
        let point = GuiPoint::new(
            (layout.list_viewport.x + 4) as f32,
            (layout.list_viewport.y + 4) as f32,
        );

        assert_eq!(controller.list_max_scroll(), 565);
        assert!(controller.handle_wheel(point, -60).is_empty());
        assert_eq!(controller.list_scroll_offset(), 60);
        controller.handle_wheel(point, -10_000);
        assert_eq!(
            controller.list_scroll_offset(),
            controller.list_max_scroll()
        );
        controller.handle_wheel(point, 10_000);
        assert_eq!(controller.list_scroll_offset(), 0);

        let scrollbar_point = GuiPoint::new(
            (layout.list_scrollbar.x + 8) as f32,
            (layout.list_scrollbar.y + 40) as f32,
        );
        controller.handle_wheel(scrollbar_point, -60);
        assert_eq!(controller.list_scroll_offset(), 0);

        let third_row_top = controller.row_layouts(&layout)[2].rect.y - layout.list_viewport.y;
        controller.handle_wheel(point, -third_row_top);
        assert!(controller
            .handle_pointer_down(point, text_font())
            .is_empty());
        assert_eq!(controller.selected_game(), Some(2));
    }

    #[test]
    fn keyboard_selection_scrolls_each_row_range_into_view() {
        let mut controller = NetDlgController::new(
            NetDlgConfig {
                masterserver_signup: false,
                ..NetDlgConfig::default()
            },
            metrics(),
        );
        controller.resize(1280, 720);
        controller.set_games(games(20));
        let layout = net_dlg_layout(1280, 720, &metrics());

        for index in 0..20 {
            assert!(controller.handle_key_down(KeyCode::Down).is_empty());
            assert_eq!(controller.selected_game(), Some(index));
            let row = controller
                .row_layouts(&layout)
                .into_iter()
                .find(|row| row.selection == NetDlgSelection::Game(index))
                .expect("selected row layout");
            let top = row.rect.y - layout.list_viewport.y;
            let bottom = top + row.rect.h;
            assert!(controller.list_scroll_offset() <= top);
            assert!(controller.list_scroll_offset() + layout.list_viewport.h >= bottom);
        }
        assert_eq!(
            controller.list_scroll_offset(),
            controller.list_max_scroll()
        );

        for index in (0..19).rev() {
            assert!(controller.handle_key_down(KeyCode::Up).is_empty());
            assert_eq!(controller.selected_game(), Some(index));
        }
        assert_eq!(controller.list_scroll_offset(), 0);
    }

    #[test]
    fn scrollbar_track_drag_and_held_arrows_match_fixed_pin_math() {
        let mut controller = NetDlgController::new(
            NetDlgConfig {
                masterserver_signup: false,
                ..NetDlgConfig::default()
            },
            metrics(),
        );
        controller.resize(1280, 720);
        controller.set_games(games(20));
        let layout = net_dlg_layout(1280, 720, &metrics());
        let track = GuiPoint::new(
            (layout.list_scrollbar.x + 8) as f32,
            (layout.list_scrollbar.y + layout.list_scrollbar.h / 2) as f32,
        );

        assert_eq!(
            controller.handle_pointer_down(center(layout.join_edit), text_font()),
            vec![NetDlgAction::FocusChanged(NetDlgControl::JoinAddress)]
        );
        assert_eq!(
            controller.handle_pointer_down(track, text_font()),
            vec![
                NetDlgAction::FocusChanged(NetDlgControl::GameList),
                NetDlgAction::GuiSound(NetDlgSound::Command),
            ]
        );
        assert!(controller.list_scroll_offset() > 0);
        let below = GuiPoint::new(track.x, (layout.list_scrollbar.y + 10_000) as f32);
        assert!(controller
            .handle_pointer_move(below, text_font())
            .is_empty());
        assert_eq!(
            controller.list_scroll_offset(),
            controller.list_max_scroll()
        );
        assert!(controller.handle_pointer_up(below, text_font()).is_empty());

        let viewport = GuiPoint::new(
            (layout.list_viewport.x + 4) as f32,
            (layout.list_viewport.y + 4) as f32,
        );
        controller.handle_wheel(viewport, 10_000);
        let bottom_arrow = GuiPoint::new(
            (layout.list_scrollbar.x + 8) as f32,
            (layout.list_scrollbar.y + layout.list_scrollbar.h - 8) as f32,
        );
        assert_eq!(
            controller.handle_pointer_down(bottom_arrow, text_font()),
            vec![NetDlgAction::GuiSound(NetDlgSound::ArrowHit)]
        );
        assert_eq!(controller.list_scroll_pin, 0);
        assert!(controller.tick_scrollbar());
        assert_eq!(controller.list_scroll_pin, 1);
        assert!(controller.tick_scrollbar());
        assert_eq!(controller.list_scroll_pin, 2);
        assert_eq!(
            controller.handle_pointer_move(track, text_font()),
            vec![NetDlgAction::GuiSound(NetDlgSound::ArrowHit)]
        );
        assert!(!controller.tick_scrollbar());
        assert_eq!(
            controller.handle_pointer_move(bottom_arrow, text_font()),
            vec![NetDlgAction::GuiSound(NetDlgSound::ArrowHit)]
        );
        assert!(controller.tick_scrollbar());
        assert_eq!(controller.list_scroll_pin, 3);
        let after_held_frames = controller.list_scroll_offset();
        assert_eq!(
            controller.handle_pointer_up(bottom_arrow, text_font()),
            vec![NetDlgAction::GuiSound(NetDlgSound::ArrowHit)]
        );
        assert!(!controller.tick_scrollbar());
        assert_eq!(controller.list_scroll_offset(), after_held_frames);

        controller.change_mode(NetDlgMode::Chat);
        let before_hidden_click = controller.list_scroll_offset();
        assert!(controller
            .handle_pointer_down(bottom_arrow, text_font())
            .is_empty());
        assert!(!controller.scrollbar_arrow_captured);
        assert!(!controller.tick_scrollbar());
        assert_eq!(controller.list_scroll_offset(), before_hidden_click);
    }

    #[test]
    fn tiny_scrollbar_without_thumb_keeps_native_synthetic_arrow_range() {
        let height = (100..500)
            .find(|height| net_dlg_layout(1280, *height, &metrics()).list_viewport.h == 48)
            .expect("a screen height with an exact 48px list viewport");
        let layout = net_dlg_layout(1280, height, &metrics());
        assert!(!NetDlgController::scrollbar_has_pin(&layout));

        let mut controller = NetDlgController::new(
            NetDlgConfig {
                masterserver_signup: false,
                ..NetDlgConfig::default()
            },
            metrics(),
        );
        controller.resize(1280, height);
        controller.set_games(games(4));
        assert_eq!(controller.list_max_scroll(), 159);
        let bottom_arrow = GuiPoint::new(
            (layout.list_scrollbar.x + 8) as f32,
            (layout.list_scrollbar.y + layout.list_scrollbar.h - 8) as f32,
        );
        assert_eq!(
            controller.handle_pointer_down(bottom_arrow, text_font()),
            vec![NetDlgAction::GuiSound(NetDlgSound::ArrowHit)]
        );
        assert!(controller.tick_scrollbar());
        assert_eq!(controller.list_scroll_pin, 1);
        assert_eq!(controller.list_scroll_offset(), 1);
    }

    #[test]
    fn gamepad_horizontal_traverses_without_firing_keyboard_back() {
        let mut controller = NetDlgController::new(NetDlgConfig::default(), metrics());
        controller.resize(1280, 720);
        assert_eq!(
            controller.handle_gamepad_horizontal(true),
            vec![NetDlgAction::FocusChanged(NetDlgControl::ChatButton)]
        );
        assert_eq!(
            controller.handle_gamepad_horizontal(false),
            vec![NetDlgAction::FocusChanged(NetDlgControl::GameList)]
        );
    }

    #[test]
    fn tooltip_targets_match_native_net_dialog_visibility_and_ip_parent_pair() {
        let mut controller = NetDlgController::new(NetDlgConfig::default(), metrics());
        controller.resize(1280, 720);
        let layout = net_dlg_layout(1280, 720, &metrics());
        for (rect, key) in [
            (layout.btn_game_list, "IDS_DESC_SHOWSAVAILABLENETWORKGAME"),
            (layout.btn_chat, "IDS_DESC_CONNECTSTOANIRCCHATSERVER"),
            (layout.btn_internet, "IDS_DLGTIP_SEARCHINTERNETGAME"),
            (layout.btn_record, "IDS_DLGTIP_RECORD"),
            (layout.buttons[0], "IDS_DLGTIP_BACKMAIN"),
            (layout.buttons[1], "IDS_NET_RELOAD_DESC"),
            (layout.buttons[2], "IDS_NET_JOINGAME_DESC"),
            (layout.buttons[3], "IDS_NET_NEWGAME_DESC"),
        ] {
            assert_eq!(
                controller.tooltip_at(center(rect)),
                Some(StartupTooltip::resource(key))
            );
        }
        for rect in [layout.ip_label, layout.join_edit] {
            assert_eq!(
                controller.tooltip_at(center(rect)),
                Some(StartupTooltip::resource("IDS_NET_IP_DESC"))
            );
        }
        assert_eq!(
            controller.tooltip_at(center(layout.game_list_caption)),
            None
        );

        assert_eq!(
            click(&mut controller, layout.btn_chat),
            vec![
                NetDlgAction::ModeChanged(NetDlgMode::Chat),
                NetDlgAction::FocusChanged(NetDlgControl::ChatInput),
            ]
        );
        for rect in [
            layout.btn_internet,
            layout.btn_record,
            layout.buttons[1],
            layout.buttons[2],
            layout.ip_label,
            layout.join_edit,
        ] {
            assert_eq!(controller.tooltip_at(center(rect)), None);
        }
        assert_eq!(
            controller.tooltip_at(center(layout.buttons[0])),
            Some(StartupTooltip::resource("IDS_DLGTIP_BACKMAIN"))
        );
    }

    // OnShown calls UpdateMasterserver, which replaces the Internet icon and
    // creates/removes only the query row. The retained dialog itself must not
    // be reconstructed: its active sheet, focus, edit contents, Record value,
    // and even simultaneous pointer/key press latches survive the config
    // refresh (C4StartupNetDlg.cpp:771-781,851-867; C4GuiButton.cpp:241-244).
    #[test]
    fn masterserver_config_sync_preserves_all_retained_dialog_state() {
        use crate::test_support::{load_graphics_png, standard_gamma};

        let assets = NetDlgAssets {
            background: load_graphics_png("StartupNetworkBG.png"),
            net_get_ref: load_graphics_png("StartupNetGetRef.png"),
            scen_icons: load_graphics_png("StartupScenSelIcons.png"),
            gui_caption: load_graphics_png("GUICaption.png"),
            gui_button: load_graphics_png("GUIButton.png"),
            gui_button_down: load_graphics_png("GUIButtonDown.png"),
            gui_button_highlight: load_graphics_png("GUIButtonHighlight.png"),
            gui_scroll: load_graphics_png("GUIScroll.png"),
            gui_icons: load_graphics_png("GUIIcons.png"),
            gui_icons_ex: load_graphics_png("GUIIcons2.png"),
        };
        let fonts = endeavour_font_set();
        let mut controller = NetDlgController::new(
            NetDlgConfig {
                masterserver_signup: true,
                record: true,
            },
            metrics(),
        );
        controller.resize(1280, 720);
        controller.set_join_address("remembered.example:11112");
        let layout = net_dlg_layout(1280, 720, &metrics());

        for expected in [
            NetDlgControl::JoinAddress,
            NetDlgControl::Internet,
            NetDlgControl::Record,
        ] {
            assert_eq!(
                controller.handle_key_down(KeyCode::Tab),
                vec![NetDlgAction::FocusChanged(expected)]
            );
        }
        assert!(controller.handle_key_down(KeyCode::Space).is_empty());
        assert!(controller
            .handle_pointer_down(center(layout.btn_internet), text_font())
            .is_empty());
        assert_eq!(controller.pointer_pressed, Some(NetDlgControl::Internet));
        assert_eq!(
            controller.key_pressed,
            Some((NetDlgControl::Record, KeyCode::Space))
        );

        let mut before = Surface::new(1280, 720, PixelFormat::Rgba8888);
        NetDlgScreen::render_controller(
            &mut before,
            &assets,
            &fonts,
            Some(standard_gamma()),
            &controller,
            0,
        );
        let retained = (
            controller.metrics,
            controller.width,
            controller.height,
            controller.mode,
            controller.join_address().to_string(),
            controller.focus,
            controller.pointer_position,
            controller.hovered,
            controller.pointer_pressed,
            controller.key_pressed,
        );

        controller.sync_masterserver_signup_from_config(false);

        assert_eq!(
            controller.config(),
            NetDlgConfig {
                masterserver_signup: false,
                record: true,
            }
        );
        assert_eq!(
            (
                controller.metrics,
                controller.width,
                controller.height,
                controller.mode,
                controller.join_address().to_string(),
                controller.focus,
                controller.pointer_position,
                controller.hovered,
                controller.pointer_pressed,
                controller.key_pressed,
            ),
            retained
        );

        let mut after = Surface::new(1280, 720, PixelFormat::Rgba8888);
        NetDlgScreen::render_controller(
            &mut after,
            &assets,
            &fonts,
            Some(standard_gamma()),
            &controller,
            0,
        );
        let changed_pixels = before
            .pixels()
            .chunks_exact(4)
            .zip(after.pixels().chunks_exact(4))
            .enumerate()
            .filter_map(|(index, (before, after))| (before != after).then_some(index))
            .collect::<Vec<_>>();
        assert!(
            !changed_pixels.is_empty(),
            "Internet icon and row must change"
        );
        let mut icon_changed = false;
        let mut row_changed = false;
        let mut query_icon_changed = false;
        let mut query_text_changed = false;
        assert!(changed_pixels.into_iter().all(|index| {
            let point = GuiPoint::new((index % 1280) as f32, (index / 1280) as f32);
            let in_icon = contains(layout.btn_internet, point);
            let in_row = contains(layout.list_entry, point);
            icon_changed |= in_icon;
            row_changed |= in_row;
            query_icon_changed |= contains(layout.entry_icon, point);
            query_text_changed |= layout
                .entry_labels
                .iter()
                .any(|rect| contains(*rect, point));
            in_icon || in_row
        }));
        assert!(icon_changed, "Internet icon must change");
        assert!(row_changed, "disabling Internet must remove the query row");
        assert!(query_icon_changed, "query animation must disappear");
        assert!(query_text_changed, "query labels must disappear");

        controller.sync_masterserver_signup_from_config(true);
        let mut restored = Surface::new(1280, 720, PixelFormat::Rgba8888);
        NetDlgScreen::render_controller(
            &mut restored,
            &assets,
            &fonts,
            Some(standard_gamma()),
            &controller,
            0,
        );
        assert!(
            (layout.list_entry.y..layout.list_entry.y + layout.list_entry.h).all(|y| {
                (layout.list_entry.x..layout.list_entry.x + layout.list_entry.w).all(|x| {
                    before.get_pixel(x as u32, y as u32) == restored.get_pixel(x as u32, y as u32)
                })
            })
        );
    }

    // The live app must render the same state that receives input: edit text,
    // toggled config icons, button interaction and the active sheet may not
    // remain stuck at the first-shown snapshot (C4StartupNetDlg.cpp:814-964;
    // C4GuiButton.cpp:81-110,205-232; C4GuiEdit.cpp:556-634).
    #[test]
    fn live_renderer_reflects_controller_state() {
        use crate::test_support::{load_graphics_png, standard_gamma};
        let assets = NetDlgAssets {
            background: load_graphics_png("StartupNetworkBG.png"),
            net_get_ref: load_graphics_png("StartupNetGetRef.png"),
            scen_icons: load_graphics_png("StartupScenSelIcons.png"),
            gui_caption: load_graphics_png("GUICaption.png"),
            gui_button: load_graphics_png("GUIButton.png"),
            gui_button_down: load_graphics_png("GUIButtonDown.png"),
            gui_button_highlight: load_graphics_png("GUIButtonHighlight.png"),
            gui_scroll: load_graphics_png("GUIScroll.png"),
            gui_icons: load_graphics_png("GUIIcons.png"),
            gui_icons_ex: load_graphics_png("GUIIcons2.png"),
        };
        let fonts = endeavour_font_set();
        let mut controller = NetDlgController::new(NetDlgConfig::default(), metrics());
        controller.resize(1280, 720);

        let render = |controller: &NetDlgController| {
            let mut surface = Surface::new(1280, 720, PixelFormat::Rgba8888);
            NetDlgScreen::render_controller(
                &mut surface,
                &assets,
                &fonts,
                Some(standard_gamma()),
                controller,
                0,
            );
            surface
        };
        let first_shown = render(&controller);

        controller.set_join_address("127.0.0.1:11111");
        let layout = net_dlg_layout(1280, 720, &metrics());
        controller.handle_pointer_down(center(layout.join_edit), text_font());
        let with_address = render(&controller);
        assert_ne!(first_shown.pixels(), with_address.pixels());

        controller.handle_pointer_move(center(layout.buttons[0]), text_font());
        let hovered = render(&controller);
        assert_ne!(with_address.pixels(), hovered.pixels());
        controller.handle_pointer_down(center(layout.buttons[0]), text_font());
        let pressed = render(&controller);
        assert_ne!(hovered.pixels(), pressed.pixels());
        controller.handle_pointer_up(center(layout.buttons[0]), text_font());

        let actions = click(&mut controller, layout.btn_chat);
        assert!(actions.contains(&NetDlgAction::ModeChanged(NetDlgMode::Chat)));
        let chat = render(&controller);
        assert_ne!(with_address.pixels(), chat.pixels());
    }

    #[test]
    fn join_edit_renderer_draws_and_clips_selection_and_caret() {
        use crate::test_support::load_graphics_png;

        let assets = NetDlgAssets {
            background: load_graphics_png("StartupNetworkBG.png"),
            net_get_ref: load_graphics_png("StartupNetGetRef.png"),
            scen_icons: load_graphics_png("StartupScenSelIcons.png"),
            gui_caption: load_graphics_png("GUICaption.png"),
            gui_button: load_graphics_png("GUIButton.png"),
            gui_button_down: load_graphics_png("GUIButtonDown.png"),
            gui_button_highlight: load_graphics_png("GUIButtonHighlight.png"),
            gui_scroll: load_graphics_png("GUIScroll.png"),
            gui_icons: load_graphics_png("GUIIcons.png"),
            gui_icons_ex: load_graphics_png("GUIIcons2.png"),
        };
        let fonts = endeavour_font_set();
        let mut controller = NetDlgController::new(NetDlgConfig::default(), metrics());
        controller.resize(1280, 720);
        controller.set_join_address("alpha beta");
        controller.focus = NetDlgControl::JoinAddress;
        controller.join_edit.caret = 5;
        controller.join_edit.selection = None;

        let render = |controller: &NetDlgController, draw_focus| {
            let mut surface = Surface::new(1280, 720, PixelFormat::Rgba8888);
            NetDlgScreen::render_controller_with_draw_focus(
                &mut surface,
                &assets,
                &fonts,
                None,
                controller,
                0,
                draw_focus,
            );
            surface
        };
        let unselected = render(&controller, false);
        controller.join_edit.selection = Some((0, 5));
        let selected = render(&controller, false);
        controller.join_edit.last_input = Instant::now();
        let with_caret = render(&controller, true);

        let client = edit_client(controller.layout().join_edit);
        let clip = IntRect {
            x: client.x - 2,
            y: client.y,
            w: client.w + 4,
            h: client.h + 1,
        };
        let changed_inside_clip = |before: &Surface, after: &Surface| {
            let changed = before
                .pixels()
                .chunks_exact(4)
                .zip(after.pixels().chunks_exact(4))
                .enumerate()
                .filter_map(|(index, (before, after))| (before != after).then_some(index))
                .collect::<Vec<_>>();
            assert!(!changed.is_empty());
            assert!(changed.into_iter().all(|index| {
                contains(
                    clip,
                    GuiPoint::new((index % 1280) as f32, (index / 1280) as f32),
                )
            }));
        };
        changed_inside_clip(&unselected, &selected);
        changed_inside_clip(&selected, &with_caret);
    }

    #[test]
    fn scrolled_rows_and_scrollbar_are_clipped_inside_the_list_client() {
        use crate::test_support::{load_graphics_png, standard_gamma};
        let assets = NetDlgAssets {
            background: load_graphics_png("StartupNetworkBG.png"),
            net_get_ref: load_graphics_png("StartupNetGetRef.png"),
            scen_icons: load_graphics_png("StartupScenSelIcons.png"),
            gui_caption: load_graphics_png("GUICaption.png"),
            gui_button: load_graphics_png("GUIButton.png"),
            gui_button_down: load_graphics_png("GUIButtonDown.png"),
            gui_button_highlight: load_graphics_png("GUIButtonHighlight.png"),
            gui_scroll: load_graphics_png("GUIScroll.png"),
            gui_icons: load_graphics_png("GUIIcons.png"),
            gui_icons_ex: load_graphics_png("GUIIcons2.png"),
        };
        let fonts = endeavour_font_set();
        let config = NetDlgConfig {
            masterserver_signup: false,
            ..NetDlgConfig::default()
        };
        let mut controller = NetDlgController::new(config, metrics());
        controller.resize(1280, 720);
        controller.set_games(games(20));
        let layout = net_dlg_layout(1280, 720, &metrics());
        let render = |controller: &NetDlgController| {
            let mut surface = Surface::new(1280, 720, PixelFormat::Rgba8888);
            NetDlgScreen::render_controller(
                &mut surface,
                &assets,
                &fonts,
                Some(standard_gamma()),
                controller,
                0,
            );
            surface
        };

        let top = render(&controller);
        controller.handle_wheel(
            GuiPoint::new(
                (layout.list_viewport.x + 4) as f32,
                (layout.list_viewport.y + 4) as f32,
            ),
            -10_000,
        );
        let bottom = render(&controller);

        let mut viewport_changed = false;
        for (index, (top_pixel, bottom_pixel)) in top
            .pixels()
            .chunks_exact(4)
            .zip(bottom.pixels().chunks_exact(4))
            .enumerate()
        {
            let point = GuiPoint::new((index % 1280) as f32, (index / 1280) as f32);
            if contains(layout.list_viewport, point) {
                viewport_changed |= top_pixel != bottom_pixel;
            } else if !contains(layout.list_client, point) {
                assert_eq!(
                    top_pixel, bottom_pixel,
                    "scrolled row bled outside list client at pixel {index}"
                );
            }
        }
        assert!(viewport_changed, "later rows must become visible");
    }

    #[test]
    fn ordered_native_game_row_text_retains_the_scrollwindow_clipper() {
        use crate::test_support::{load_graphics_png, standard_gamma};
        let assets = NetDlgAssets {
            background: load_graphics_png("StartupNetworkBG.png"),
            net_get_ref: load_graphics_png("StartupNetGetRef.png"),
            scen_icons: load_graphics_png("StartupScenSelIcons.png"),
            gui_caption: load_graphics_png("GUICaption.png"),
            gui_button: load_graphics_png("GUIButton.png"),
            gui_button_down: load_graphics_png("GUIButtonDown.png"),
            gui_button_highlight: load_graphics_png("GUIButtonHighlight.png"),
            gui_scroll: load_graphics_png("GUIScroll.png"),
            gui_icons: load_graphics_png("GUIIcons.png"),
            gui_icons_ex: load_graphics_png("GUIIcons2.png"),
        };
        let fonts = endeavour_font_set();
        let mut controller = NetDlgController::new(
            NetDlgConfig {
                masterserver_signup: false,
                ..NetDlgConfig::default()
            },
            metrics(),
        );
        controller.resize(1280, 720);
        controller.set_games(games(20));
        let layout = net_dlg_layout(1280, 720, &metrics());
        controller.handle_wheel(
            GuiPoint::new(
                (layout.list_viewport.x + 4) as f32,
                (layout.list_viewport.y + 4) as f32,
            ),
            -10_000,
        );

        let mut surface = Surface::new(1280, 720, PixelFormat::Rgba8888);
        surface.begin_clonk_text_capture();
        NetDlgScreen::render_controller(
            &mut surface,
            &assets,
            &fonts,
            Some(standard_gamma()),
            &controller,
            0,
        );
        let expected = lc_graphics::Rect::new(
            layout.list_viewport.x,
            layout.list_viewport.y,
            layout.list_viewport.w as u32,
            layout.list_viewport.h as u32,
        );
        let row_commands = surface
            .take_clonk_text_capture()
            .into_iter()
            .filter(|command| {
                command.text.starts_with("Game ") || command.text.starts_with("Lobby ")
            })
            .collect::<Vec<_>>();
        assert!(!row_commands.is_empty());
        assert!(
            row_commands
                .iter()
                .all(|command| command.clip == Some(expected)),
            "every native game-row text command must retain the viewport clip"
        );
    }

    /// Renders the dialog at 1280x720 with the final whole-surface gamma pass
    /// (mirroring the app's render_startup_frame) and dumps the PPM artifact
    /// that is diffed offline against the C++ F9 reference. CI has no
    /// reference, so this test only checks coarse invariants.
    #[test]
    fn render_matches_reference() {
        use crate::test_support::{load_graphics_png, standard_gamma, write_ppm};
        let assets = NetDlgAssets {
            background: load_graphics_png("StartupNetworkBG.png"),
            net_get_ref: load_graphics_png("StartupNetGetRef.png"),
            scen_icons: load_graphics_png("StartupScenSelIcons.png"),
            gui_caption: load_graphics_png("GUICaption.png"),
            gui_button: load_graphics_png("GUIButton.png"),
            gui_button_down: load_graphics_png("GUIButtonDown.png"),
            gui_button_highlight: load_graphics_png("GUIButtonHighlight.png"),
            gui_scroll: load_graphics_png("GUIScroll.png"),
            gui_icons: load_graphics_png("GUIIcons.png"),
            gui_icons_ex: load_graphics_png("GUIIcons2.png"),
        };
        let fonts = endeavour_font_set();
        let mut surface = Surface::new(1280, 720, PixelFormat::Rgba8888);
        // The capture machine had Config.General.Record enabled (the icons
        // are config-driven; the C++ default is false) — render to match the
        // reference.
        let config = NetDlgConfig {
            record: true,
            ..NetDlgConfig::default()
        };
        NetDlgScreen::render(
            &mut surface,
            &assets,
            &fonts,
            Some(standard_gamma()),
            config,
            0,
        );
        standard_gamma().apply_to_surface(&mut surface);

        // The opaque background must cover every pixel (no channel left at
        // the cleared 0 thanks to the gamma floor).
        assert!(surface.get_pixel(0, 0).map(|c| c.r >= 1).unwrap_or(false));

        let dir = std::path::Path::new("/tmp/menu-parity-net");
        std::fs::create_dir_all(dir).expect("create artifact dir");
        write_ppm(&surface, dir.join("out.ppm"));
    }
}
