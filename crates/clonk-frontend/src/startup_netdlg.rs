//! Pixel-parity renderer for the C++ `C4StartupNetDlg` startup dialog
//! ("Start Network Game"), mirroring the engine's first-shown state
//! (see `target/parity-specs/net.md`). Implemented against the
//! engine's F9 reference capture at 1280x720; owned by its
//! implementation agent.
//!
//! Geometry mirrors the C++ constructor math (C4StartupNetDlg.cpp:631-728)
//! with the fullscreen-dialog margins of C4GuiDialogs.cpp:813-822 and the
//! ComponentAligner of C4Gui.cpp:975-1057.

use std::time::Instant;

use crate::classic_gui::{
    blacken_transparent_pixels, draw_3d_frame, draw_clipped_text, draw_engine_box,
    draw_engine_line, draw_facet_stretch, ClassicButtonState, ClassicGuiSkin,
};
use crate::clonk_fonts::{expand_hotkey_markup, ClonkFontSet};
use crate::draw_scaled_caret;
use crate::message_dialog::break_message;
use crate::startup_main_menu::{IntRect, StartupTooltip};
use crate::{GuiPoint, ImageData, KeyCode};
use clonk_graphics::clonk_font::{ClonkFont, TextAlign};
use clonk_graphics::{GammaRamp, Surface};
use clonk_gui::Rect as GuiRect;

// Engine colors (C4Gui.h:52-103,163-165). Font colors are NORMAL-alpha RGBA
// (0xff = opaque); box/line colors are engine AARRGGBB with INVERTED alpha
// (0x00 = opaque).
/// C4GUI_FullscreenCaptionFontClr / C4GUI_Caption2FontClr / C4GUI_ButtonFontClr.
/// `C4GUI::TextWindow`'s constructor defaults, which `C4ChatControl::ChatSheet`
/// takes verbatim (src/C4Gui.h:1309; src/C4ChatDlg.cpp:194).
/// `C4GUI_ScrollArrowHgt` (src/C4Gui.h).
const CHAT_SCROLL_ARROW_EXTENT: i32 = 16;
const CHAT_TRANSCRIPT_MAX_LINES: usize = 100;
const CHAT_TRANSCRIPT_MAX_TEXT: usize = 4096;

const CLR_YELLOW: [u8; 4] = [0xff, 0xff, 0x00, 0xff];
/// C4GUI_CaptionFontClr / C4GUI_MessageFontClr.
const CLR_WHITE: [u8; 4] = [0xff, 0xff, 0xff, 0xff];
/// C4GUI_InactMessageFontClr.
const CLR_INACTIVE: [u8; 4] = [0xaf, 0xaf, 0xaf, 0xff];
/// C4GUI_NotifyFontClr.
const CLR_NOTIFY: [u8; 4] = [0xff, 0x00, 0x00, 0xff];
const CLR_DISABLED: [u8; 4] = [0xaf, 0xaf, 0xaf, 0xff];
const CLR_HYPERLINK: [u8; 4] = [0x80, 0x80, 0xff, 0xff];
/// C4GUI_ErrorFontClr.
const CLR_ERROR: [u8; 4] = [0xff, 0x1f, 0x1f, 0xff];
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
        let out = IntRect::new(
            self.area.x + self.mx + wdt.map_or(0, |w| (full_w - w) / 2),
            self.area.y + self.my,
            wdt.unwrap_or(full_w),
            hgt,
        );
        let d = hgt + self.my * 2;
        self.area.y += d;
        self.area.h -= d;
        out
    }

    /// GetFromLeft, full height (C4Gui.cpp:992-1008).
    fn get_from_left(&mut self, wdt: i32) -> IntRect {
        let out = IntRect::new(
            self.area.x + self.mx,
            self.area.y + self.my,
            wdt,
            self.area.h - self.my * 2,
        );
        let d = wdt + self.mx * 2;
        self.area.x += d;
        self.area.w -= d;
        out
    }

    /// GetFromRight, full height (C4Gui.cpp:1010-1024).
    fn get_from_right(&mut self, wdt: i32) -> IntRect {
        let out = IntRect::new(
            self.area.x + self.area.w - wdt - self.mx,
            self.area.y + self.my,
            wdt,
            self.area.h - self.my * 2,
        );
        self.area.w -= wdt + self.mx * 2;
        out
    }

    /// GetFromBottom, full width (C4Gui.cpp:1026-1041).
    fn get_from_bottom(&mut self, hgt: i32) -> IntRect {
        let out = IntRect::new(
            self.area.x + self.mx,
            self.area.y + self.area.h - hgt - self.my,
            self.area.w - self.mx * 2,
            hgt,
        );
        self.area.h -= hgt + self.my * 2;
        out
    }

    /// GetAll (C4Gui.cpp:1043-1049).
    fn all(&self) -> IntRect {
        IntRect::new(
            self.area.x + self.mx,
            self.area.y + self.my,
            self.area.w - self.mx * 2,
            self.area.h - self.my * 2,
        )
    }

    /// GetCentered (C4Gui.cpp:1051-1060; GetMiddleX/Y are x + Wdt/2).
    fn centered(&self, wdt: i32, hgt: i32) -> IntRect {
        IntRect::new(
            self.area.x + self.area.w / 2 - wdt / 2,
            self.area.y + self.area.h / 2 - hgt / 2,
            wdt,
            hgt,
        )
    }

    /// `ComponentAligner::ExpandTop`; negative values deliberately consume
    /// space and are used by the classic IRC login form to change from its
    /// 2px component margin to the 5px group gap.
    fn expand_top(&mut self, by: i32) {
        self.area.y -= by;
        self.area.h += by;
    }
}

/// Offsets `rect` by `(dx, dy)`.
fn offset(rect: IntRect, dx: i32, dy: i32) -> IntRect {
    rect.with_position(rect.x + dx, rect.y + dy)
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
    let client = IntRect::new(
        margin_x,
        margin_top,
        w - 2 * margin_x,
        h - margin_top - margin_y,
    );
    let at_client = |rect: IntRect| offset(rect, client.x, client.y);

    // Constructor constants (C4StartupNetDlg.cpp:633-637).
    let icon_size = 64; // C4GUI_IconExWdt
    let side_size = (w / 6).max(icon_size);
    let button_hgt = 32; // C4GUI_ButtonHgt
    let button_indent = w / 40;
    let button_wdt = metrics.caption_back_extent * 3;

    // Aligner stacking (C4StartupNetDlg.cpp:638-645); caMain is zero-based
    // over the client rect (fZeroAreaXY = true).
    let mut ca_main = Aligner::new(IntRect::new(0, 0, client.w, client.h), 0, 0);
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
    let mut ca_game_list = Aligner::new(IntRect::new(0, 0, tabular.w, tabular.h), 0, 0);
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
    let list_client = IntRect::new(
        game_list.x + 3,
        game_list.y + 3,
        game_list.w - 6,
        game_list.h - 6,
    );
    // Entry: iHeight = 2*22 + 4 = 48 after label restack
    // (C4StartupNetDlg.cpp:42-44,372-388).
    let entry_h = metrics.text_line_height * 2 + 4;
    let list_viewport = IntRect::new(
        list_client.x,
        list_client.y,
        list_client.w - SCROLLBAR_WIDTH,
        list_client.h,
    );
    let list_scrollbar = IntRect::new(
        list_viewport.x + list_viewport.w,
        list_viewport.y,
        SCROLLBAR_WIDTH,
        list_viewport.h,
    );
    let list_entry = IntRect::new(list_viewport.x, list_viewport.y, list_viewport.w, entry_h);
    // Aspect-fit of the 40x32 query-icon facet into the 48x48 icon bounds
    // (C4Facet.cpp:100-127): Hgt = 32*48/40, centered vertically.
    let icon_fit_h = 32 * entry_h / 40;
    let entry_icon = IntRect::new(
        list_entry.x,
        list_entry.y + (entry_h - icon_fit_h) / 2,
        entry_h,
        icon_fit_h,
    );
    // Labels at x = 48+3, y = 1/25, width = entryW - 51 - 1
    // (C4StartupNetDlg.cpp:64-76, UpdateText C4StartupNetDlg.cpp:400-410).
    let label_x = list_entry.x + entry_h + 3;
    let label_w = list_entry.w - (entry_h + 3) - 1;
    let entry_labels = [
        IntRect::new(label_x, list_entry.y + 1, label_w, metrics.text_line_height),
        IntRect::new(
            label_x,
            list_entry.y + 1 + metrics.text_line_height + 2,
            label_w,
            metrics.text_line_height,
        ),
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
    IntRect::new(
        rect.x + 4,
        rect.y + 2,
        (rect.w - 8).max(0),
        (rect.h - 4).max(0),
    )
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

/// Which half of the classic `C4ChatControl` is currently visible.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum NetDlgChatPage {
    #[default]
    Login,
    Chats,
}

/// Editable field on the IRC login page. The server is supplied by config,
/// just as in `C4ChatControl::OnConnectBtn`, and is therefore not an edit.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum NetDlgChatLoginField {
    #[default]
    Nick,
    Password,
    RealName,
    Channel,
}

impl NetDlgChatLoginField {
    const ALL: [Self; 4] = [Self::Nick, Self::Password, Self::RealName, Self::Channel];

    const fn index(self) -> usize {
        match self {
            Self::Nick => 0,
            Self::Password => 1,
            Self::RealName => 2,
            Self::Channel => 3,
        }
    }
}

/// Localized strings drawn by the dependency-free IRC frontend. The app
/// supplies these from the active resource table; English defaults keep
/// standalone tests and embedders source-compatible.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct NetDlgChatStrings {
    pub chat: String,
    pub not_connected: String,
    pub server: String,
    pub nick: String,
    pub password_optional: String,
    pub real_name: String,
    pub channel: String,
    pub connect: String,
    pub not_connected_error: String,
    /// Use `{command}` for the command name.
    pub insufficient_parameters: String,
    pub invalid_nick: String,
    /// Use `{command}` for the command name.
    pub unknown_command: String,
    pub not_on_channel: String,
    /// `IDS_NET_CONNECTING`. C4ChatControl substitutes the server address and
    /// an empty second argument (C4ChatDlg.cpp:643).
    pub connecting: String,
}

/// C4ResStrTable substitutes positional `%s`/`%d` arguments in template order.
/// A placeholder without a matching argument stays literal, which is what the
/// C++ `sprintf` fallback yields for a truncated language table.
fn substitute_resource_arguments(template: &str, arguments: &[&str]) -> String {
    let mut output = String::with_capacity(template.len());
    let mut remainder = template;
    let mut arguments = arguments.iter();
    while let Some(placeholder) = remainder.find('%') {
        output.push_str(&remainder[..placeholder]);
        let rest = &remainder[placeholder..];
        let mut characters = rest.chars();
        characters.next();
        match characters.next() {
            Some('s' | 'd' | 'i' | 'u') => match arguments.next() {
                Some(argument) => output.push_str(argument),
                None => output.push_str(&rest[..2]),
            },
            Some('%') => output.push('%'),
            Some(_) => output.push_str(&rest[..2]),
            None => {
                output.push('%');
                remainder = "";
                break;
            }
        }
        remainder = &rest[2..];
    }
    output.push_str(remainder);
    output
}

impl Default for NetDlgChatStrings {
    fn default() -> Self {
        Self {
            chat: "Chat".into(),
            not_connected: "Not connected".into(),
            server: "Server".into(),
            nick: "Nickname:".into(),
            password_optional: "Password (optional):".into(),
            real_name: "Real name:".into(),
            channel: "Channel:".into(),
            connect: "Connect".into(),
            not_connected_error: "Not connected to a server".into(),
            insufficient_parameters: "Insufficient parameters for /{command}".into(),
            invalid_nick: "/{command}: invalid nick name".into(),
            unknown_command: "Unknown command: {command}".into(),
            not_on_channel: "Not on a channel".into(),
            connecting: "Connecting to %s at %s".into(),
        }
    }
}

/// Exact validation category returned to the application, which owns the
/// localized modal and restores focus to [`NetDlgChatValidationError::field`].
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum NetDlgChatValidationError {
    InvalidNick,
    InvalidPassword,
    InvalidChannel,
}

impl NetDlgChatValidationError {
    pub const fn field(self) -> NetDlgChatLoginField {
        match self {
            Self::InvalidNick => NetDlgChatLoginField::Nick,
            Self::InvalidPassword => NetDlgChatLoginField::Password,
            Self::InvalidChannel => NetDlgChatLoginField::Channel,
        }
    }
}

/// Values submitted by the classic IRC login form. Passwords remain
/// transient; the application decides which other fields are persisted.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct NetDlgChatLogin {
    pub server: String,
    pub nick: String,
    pub password: String,
    pub real_name: String,
    pub channel: String,
}

impl Default for NetDlgChatLogin {
    fn default() -> Self {
        Self {
            server: "irc.euirc.net".to_string(),
            nick: String::new(),
            password: String::new(),
            real_name: String::new(),
            channel: "#clonken,#legacyclonk".to_string(),
        }
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum NetDlgChatConnectionState {
    #[default]
    Disconnected,
    Connecting,
    Connected,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum NetDlgChatMessageKind {
    Server,
    Status,
    Message,
    Notice,
    Action,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct NetDlgChatMessage {
    pub kind: NetDlgChatMessageKind,
    pub source: String,
    pub target: String,
    pub text: String,
    pub is_channel: bool,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct NetDlgChatUser {
    pub prefix: String,
    pub name: String,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct NetDlgChatChannel {
    pub name: String,
    pub topic: String,
    pub users: Vec<NetDlgChatUser>,
}

/// Dependency-free projection of `clonk_network::irc::IrcClientSnapshot`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct NetDlgChatSnapshot {
    pub connection_state: NetDlgChatConnectionState,
    pub server: String,
    pub nick: String,
    pub channels: Vec<NetDlgChatChannel>,
    pub messages: Vec<NetDlgChatMessage>,
    pub unread_index: usize,
    pub last_error: Option<String>,
}

impl Default for NetDlgChatSnapshot {
    fn default() -> Self {
        Self {
            connection_state: NetDlgChatConnectionState::Disconnected,
            server: NetDlgChatLogin::default().server,
            nick: String::new(),
            channels: Vec::new(),
            messages: Vec::new(),
            unread_index: 0,
            last_error: None,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum NetDlgChatSheetKind {
    Server,
    Channel,
    Query,
}

/// Native chat transcript color class. The renderer maps these to the exact
/// C4GUI message, inactive, notify and error colors.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum NetDlgChatLineKind {
    Server,
    Status,
    Message,
    Notice,
    Action,
    Error,
}

/// One *display* line of the transcript, which is the unit `C4LogBuffer`
/// stores: `AppendLines` splits a message on CR/LF and word-wraps each part at
/// the width in force when it arrived, then hands every resulting line to
/// `AppendSingleLine` separately (src/C4LogBuf.cpp:96-148,174-205).
/// `TextWindow::UpdateSize` only re-bounds the buffer control
/// (src/C4GuiLabels.cpp:490-500), so a later resize never re-flows what is
/// already stored.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct NetDlgChatLine {
    pub kind: NetDlgChatLineKind,
    pub text: String,
    /// True on the first display line produced by a source message or by a
    /// CR/LF-separated part of one, which is what spaces paragraphs apart.
    pub new_paragraph: bool,
}

impl PartialEq<&str> for NetDlgChatLine {
    fn eq(&self, other: &&str) -> bool {
        self.text == *other
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct NetDlgChatSheet {
    pub kind: NetDlgChatSheetKind,
    pub title: String,
    pub ident: String,
    pub topic: String,
    pub users: Vec<NetDlgChatUser>,
    pub lines: Vec<NetDlgChatLine>,
    pub unread: bool,
    /// Pixel displacement from the top of the wrapped transcript.
    pub transcript_scroll: i32,
    /// True while incoming lines keep the transcript pinned to its bottom.
    pub transcript_follow_bottom: bool,
    /// Retained `C4GUI::ScrollWindow` offset of the channel nick pane, which
    /// C++ builds as a scrollable `ListBox` (src/C4ChatDlg.cpp:226-238).
    /// Rows below the fold are reachable rather than dropped.
    pub user_scroll: i32,
}

/// Commands parsed from the classic slash-command language. These mirror the
/// network crate's transport commands while keeping `clonk-frontend` independent.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum NetDlgChatCommand {
    Quit { reason: String },
    Join { channel: String },
    Part { channel: String },
    Message { target: String, text: String },
    Notice { target: String, text: String },
    Action { target: String, text: String },
    Raw(String),
    ChangeNick { nick: String },
    OpenQuery { nick: String },
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
    Error,
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
    ChatConnect(NetDlgChatLogin),
    /// Requests the host-owned localized error modal. The controller has
    /// already restored focus to the invalid edit.
    ChatValidationFailed(NetDlgChatValidationError),
    ChatCommand(NetDlgChatCommand),
    /// Closing the server sheet asks the host to display the classic
    /// disconnect OK/Abort modal. Accepting it dispatches `ChatDisconnect`.
    ChatDisconnectConfirmationRequested,
    /// Closes only the singleton standalone window; the process-global IRC
    /// transport remains connected.
    ChatDialogCloseRequested,
    ChatDisconnect,
    /// Mirrors `C4MessageInput::StoreBackBuffer` so the app can synchronize
    /// IRC with its process-global lobby/running-chat history.
    ChatHistoryStored(String),
    ChatSelectSheet {
        kind: NetDlgChatSheetKind,
        ident: String,
    },
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
        (Instant::now()
            .checked_duration_since(self.last_input)
            .unwrap_or_default()
            .as_millis()
            / 500)
            .is_multiple_of(2)
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
    chat_strings: NetDlgChatStrings,
    /// Full outer bounds of a standalone IRC dialog. `None` uses the startup
    /// NetDlg's embedded chat group.
    chat_bounds_override: Option<IntRect>,
    chat_server: String,
    /// Sheet a still-unreported send was submitted from, so the transport's
    /// asynchronous error lands where `ProcessInput` would have put it.
    chat_send_error_origin: Option<(NetDlgChatSheetKind, String)>,
    chat_login_edits: [NetDlgEditState; 4],
    chat_login_field: NetDlgChatLoginField,
    chat_connect_focused: bool,
    chat_connection_state: NetDlgChatConnectionState,
    chat_page: NetDlgChatPage,
    chat_initial_messages_received: bool,
    chat_sheets: Vec<NetDlgChatSheet>,
    chat_active_sheet: usize,
    chat_edit: NetDlgEditState,
    chat_history: Vec<String>,
    chat_history_index: Option<usize>,
    chat_pressed: Option<NetDlgChatHit>,
    chat_dialog_drag: Option<NetDlgChatDialogDrag>,
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
    /// A held `C4GUI::ScrollBar` pin on the IRC transcript.
    chat_transcript_scrollbar_dragging: bool,
    scrollbar_arrow_captured: bool,
    scrollbar_arrow: i8,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum NetDlgChatHit {
    DialogClose,
    DialogCaption,
    LoginField(NetDlgChatLoginField),
    Connect,
    Tab(usize),
    TabClose(usize),
    Input,
    Transcript,
    /// `C4GUI::ScrollBar`'s three pointer regions
    /// (src/C4GuiContainers.cpp:477-623): the two arrow buttons step one line,
    /// the pin is draggable, and pressing the bare track pages.
    TranscriptScrollUp,
    TranscriptScrollDown,
    TranscriptScrollPin,
    TranscriptScrollTrack,
    User(usize),
}

#[derive(Clone, Copy, Debug)]
struct NetDlgChatDialogDrag {
    pointer: GuiPoint,
    bounds: IntRect,
}

#[derive(Clone, Copy, Debug)]
struct NetDlgChatTabLayout {
    rect: IntRect,
    close: IntRect,
}

#[derive(Clone, Debug)]
struct NetDlgChatLayout {
    group: IntRect,
    login_labels: [IntRect; 4],
    login_edits: [IntRect; 4],
    connect: IntRect,
    tabs: Vec<NetDlgChatTabLayout>,
    transcript: IntRect,
    transcript_viewport: IntRect,
    transcript_scrollbar: IntRect,
    users: Option<IntRect>,
    input_label: Option<IntRect>,
    input: IntRect,
}

#[derive(Clone, Debug)]
struct NetDlgWrappedChatLine {
    text: String,
    kind: NetDlgChatLineKind,
    new_paragraph: bool,
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
        let login = NetDlgChatLogin::default();
        let mut chat_login_edits: [NetDlgEditState; 4] =
            std::array::from_fn(|_| Default::default());
        chat_login_edits[NetDlgChatLoginField::Nick.index()].set_text(&login.nick);
        chat_login_edits[NetDlgChatLoginField::Password.index()].set_text(&login.password);
        chat_login_edits[NetDlgChatLoginField::RealName.index()].set_text(&login.real_name);
        chat_login_edits[NetDlgChatLoginField::Channel.index()].set_text(&login.channel);
        Self {
            metrics,
            text_font: None,
            width: 1,
            height: 1,
            config,
            mode: NetDlgMode::GameList,
            join_edit: NetDlgEditState::default(),
            chat_strings: NetDlgChatStrings::default(),
            chat_bounds_override: None,
            chat_server: login.server.clone(),
            chat_send_error_origin: None,
            chat_login_edits,
            chat_login_field: NetDlgChatLoginField::Nick,
            chat_connect_focused: false,
            chat_connection_state: NetDlgChatConnectionState::Disconnected,
            chat_page: NetDlgChatPage::Login,
            chat_initial_messages_received: false,
            chat_sheets: vec![NetDlgChatSheet {
                kind: NetDlgChatSheetKind::Server,
                title: "Server".to_string(),
                ident: login.server,
                topic: String::new(),
                users: Vec::new(),
                lines: Vec::new(),
                unread: false,
                transcript_scroll: 0,
                transcript_follow_bottom: true,
                user_scroll: 0,
            }],
            chat_active_sheet: 0,
            chat_edit: NetDlgEditState::default(),
            chat_history: Vec::new(),
            chat_history_index: None,
            chat_pressed: None,
            chat_dialog_drag: None,
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
            chat_transcript_scrollbar_dragging: false,
            scrollbar_arrow_captured: false,
            scrollbar_arrow: 0,
        }
    }

    pub fn resize(&mut self, width: i32, height: i32) {
        self.chat_dialog_drag = None;
        self.width = width.max(1);
        self.height = height.max(1);
        self.hovered = self
            .pointer_position
            .and_then(|point| self.hit_button(point));
        self.clamp_list_scroll();
        self.clamp_active_chat_scroll();
    }

    /// Supplies the live text font used by `CStdFont::BreakMessage` row
    /// wrapping. The constructor retains its metrics-only fallback so tests
    /// and headless callers that cannot load GUI fonts remain usable.
    pub fn set_text_font(&mut self, font: &ClonkFont) {
        self.text_font = Some(font.clone());
        self.clamp_list_scroll();
        self.clamp_active_chat_scroll();
    }

    pub const fn config(&self) -> NetDlgConfig {
        self.config
    }

    pub fn set_chat_strings(&mut self, strings: NetDlgChatStrings) {
        self.chat_strings = strings;
        if let Some(server) = self
            .chat_sheets
            .iter_mut()
            .find(|sheet| sheet.kind == NetDlgChatSheetKind::Server)
        {
            server.title.clone_from(&self.chat_strings.server);
        }
    }

    pub fn chat_strings(&self) -> &NetDlgChatStrings {
        &self.chat_strings
    }

    /// Whether the visible standalone chat page owns an Alt mnemonic through
    /// one of its classic GUI controls. The login page currently has the
    /// localized Connect button; chat sheets expose no mnemonic buttons.
    pub fn chat_dialog_has_hotkey(&self, character: char) -> bool {
        self.chat_page == NetDlgChatPage::Login
            && expand_hotkey_markup(&self.chat_strings.connect)
                .1
                .is_some_and(|hotkey| hotkey.eq_ignore_ascii_case(&character))
    }

    /// Overrides the full outer chat-dialog rectangle. This is used by the
    /// standalone `C4ChatDlg` surface; its top caption is carved from these
    /// bounds and all pointer hit testing follows the same geometry.
    pub fn set_chat_bounds_override(&mut self, bounds: Option<IntRect>) {
        self.chat_dialog_drag = None;
        self.chat_bounds_override =
            bounds.map(|bounds| IntRect::new(bounds.x, bounds.y, bounds.w.max(1), bounds.h.max(1)));
        self.clamp_active_chat_scroll();
    }

    pub const fn chat_bounds_override(&self) -> Option<IntRect> {
        self.chat_bounds_override
    }

    /// Whether the standalone title currently owns positional pointer drag.
    pub const fn chat_dialog_drag_active(&self) -> bool {
        self.chat_dialog_drag.is_some()
    }

    /// Native standalone placement: ten percent inset on every edge.
    pub const fn standalone_chat_bounds(width: i32, height: i32) -> IntRect {
        IntRect::new(width / 10, height / 10, width * 4 / 5, height * 4 / 5)
    }

    /// Makes this controller immediately usable as a standalone chat dialog,
    /// without synthesizing startup Games/Chat button actions.
    pub fn force_chat_mode_and_focus(&mut self) {
        self.mode = NetDlgMode::Chat;
        self.focus = NetDlgControl::ChatInput;
        self.join_edit.blur();
        self.chat_connect_focused = false;
        self.active_chat_edit_mut().focus();
    }

    /// Applies `C4ChatControl::GetDefaultControl` for a newly shown
    /// standalone dialog: Connect owns focus on the login page, while a live
    /// chat focuses the active sheet's message edit.
    pub fn force_chat_mode_and_default_focus(&mut self) {
        self.mode = NetDlgMode::Chat;
        self.focus = NetDlgControl::ChatInput;
        self.join_edit.blur();
        match self.chat_page {
            NetDlgChatPage::Login => {
                for edit in &mut self.chat_login_edits {
                    edit.blur();
                }
                self.chat_connect_focused = true;
            }
            NetDlgChatPage::Chats => {
                self.chat_connect_focused = false;
                self.chat_edit.focus();
            }
        }
    }

    /// Seeds the C++ process-global message backbuffer order: newest entry at
    /// index zero. Empty and duplicate entries are discarded; at most twenty
    /// remain. Local submissions still update this fallback immediately.
    pub fn set_chat_history(&mut self, history_newest_first: Vec<String>) {
        self.chat_history.clear();
        for entry in history_newest_first {
            if entry.is_empty() || self.chat_history.iter().any(|old| old == &entry) {
                continue;
            }
            self.chat_history.push(entry);
            if self.chat_history.len() == 20 {
                break;
            }
        }
        self.chat_history_index = None;
    }

    pub fn chat_history(&self) -> &[String] {
        &self.chat_history
    }

    pub fn set_chat_login(&mut self, login: NetDlgChatLogin) {
        self.chat_server = login.server;
        self.chat_login_edits[NetDlgChatLoginField::Nick.index()].set_text(&login.nick);
        self.chat_login_edits[NetDlgChatLoginField::Password.index()].set_text(&login.password);
        self.chat_login_edits[NetDlgChatLoginField::RealName.index()].set_text(&login.real_name);
        self.chat_login_edits[NetDlgChatLoginField::Channel.index()].set_text(&login.channel);
        if let Some(server) = self.chat_sheets.first_mut() {
            server.ident.clone_from(&self.chat_server);
        }
    }

    pub fn chat_login(&self) -> NetDlgChatLogin {
        NetDlgChatLogin {
            server: self.chat_server.clone(),
            nick: self.chat_login_edits[NetDlgChatLoginField::Nick.index()]
                .text
                .clone(),
            password: self.chat_login_edits[NetDlgChatLoginField::Password.index()]
                .text
                .clone(),
            real_name: self.chat_login_edits[NetDlgChatLoginField::RealName.index()]
                .text
                .clone(),
            channel: self.chat_login_edits[NetDlgChatLoginField::Channel.index()]
                .text
                .clone(),
        }
    }

    pub const fn chat_page(&self) -> NetDlgChatPage {
        self.chat_page
    }

    pub const fn chat_edit_is_focused(&self) -> bool {
        matches!(self.mode, NetDlgMode::Chat)
            && matches!(self.focus, NetDlgControl::ChatInput)
            && !self.chat_connect_focused
    }

    pub const fn chat_connection_state(&self) -> NetDlgChatConnectionState {
        self.chat_connection_state
    }

    pub const fn chat_login_field(&self) -> NetDlgChatLoginField {
        self.chat_login_field
    }

    pub fn chat_input(&self) -> &str {
        match self.chat_page() {
            NetDlgChatPage::Login => &self.chat_login_edits[self.chat_login_field.index()].text,
            NetDlgChatPage::Chats => &self.chat_edit.text,
        }
    }

    pub fn chat_sheets(&self) -> &[NetDlgChatSheet] {
        &self.chat_sheets
    }

    pub fn active_chat_sheet(&self) -> Option<&NetDlgChatSheet> {
        self.chat_sheets.get(self.chat_active_sheet)
    }

    /// Returns to the login form after an explicit user disconnect (or a
    /// cancelled host-side connection warning). Transport shutdown remains an
    /// application side effect.
    pub fn show_chat_login(&mut self) {
        self.chat_page = NetDlgChatPage::Login;
        self.chat_connection_state = NetDlgChatConnectionState::Disconnected;
        self.chat_initial_messages_received = false;
        self.chat_connect_focused = false;
    }

    pub fn request_chat_disconnect(&mut self) -> Vec<NetDlgAction> {
        self.show_chat_login();
        vec![NetDlgAction::ChatDisconnect]
    }

    pub fn close_active_chat_sheet(&mut self) -> Vec<NetDlgAction> {
        self.close_chat_sheet(self.chat_active_sheet)
    }

    pub fn close_chat_sheet(&mut self, index: usize) -> Vec<NetDlgAction> {
        let Some(sheet) = self.chat_sheets.get(index).cloned() else {
            return Vec::new();
        };
        match sheet.kind {
            NetDlgChatSheetKind::Server => {
                if self.chat_connection_state == NetDlgChatConnectionState::Disconnected {
                    self.request_chat_disconnect()
                } else {
                    vec![NetDlgAction::ChatDisconnectConfirmationRequested]
                }
            }
            NetDlgChatSheetKind::Channel => {
                vec![NetDlgAction::ChatCommand(NetDlgChatCommand::Part {
                    channel: sheet.ident,
                })]
            }
            NetDlgChatSheetKind::Query => {
                self.chat_sheets.remove(index);
                if index < self.chat_active_sheet {
                    self.chat_active_sheet = self.chat_active_sheet.saturating_sub(1);
                } else if index == self.chat_active_sheet {
                    self.chat_active_sheet = index.min(self.chat_sheets.len().saturating_sub(1));
                }
                if let Some(active) = self.chat_sheets.get_mut(self.chat_active_sheet) {
                    active.unread = false;
                }
                Vec::new()
            }
        }
    }

    /// Rebuilds the visible classic server/channel/query tabs from a backend
    /// snapshot. Existing manually opened query tabs and the active tab survive
    /// refreshes, while parted channels disappear as in `C4ChatControl::Update`.
    pub fn sync_chat_snapshot(&mut self, snapshot: NetDlgChatSnapshot) {
        let previous_page = self.chat_page;
        // Every line appended below wraps against the width in force now, the
        // way `AddTextLine` hands `AppendLines` the window's current extent.
        let wrap_width = self.chat_transcript_wrap_width();
        let has_error = snapshot
            .last_error
            .as_ref()
            .is_some_and(|error| !error.is_empty());
        let new_connection = self.chat_connection_state == NetDlgChatConnectionState::Disconnected
            && snapshot.connection_state != NetDlgChatConnectionState::Disconnected;
        let active = self
            .active_chat_sheet()
            .map(|sheet| (sheet.kind, sheet.ident.clone()));
        let old_sheets = self.chat_sheets.clone();
        let existing_channels = old_sheets
            .iter()
            .filter(|sheet| sheet.kind == NetDlgChatSheetKind::Channel)
            .map(|sheet| sheet.ident.clone())
            .collect::<Vec<_>>();
        if new_connection {
            self.chat_initial_messages_received = false;
            self.chat_page = NetDlgChatPage::Chats;
        } else if snapshot.connection_state != NetDlgChatConnectionState::Disconnected || has_error
        {
            self.chat_page = NetDlgChatPage::Chats;
        }
        if previous_page != NetDlgChatPage::Chats && self.chat_page == NetDlgChatPage::Chats {
            self.chat_connect_focused = false;
            for edit in &mut self.chat_login_edits {
                edit.blur();
            }
            if self.focus == NetDlgControl::ChatInput {
                self.chat_edit.focus();
            }
        }
        if !snapshot.server.is_empty() {
            self.chat_server.clone_from(&snapshot.server);
        }

        let mut server = old_sheets
            .iter()
            .find(|sheet| sheet.kind == NetDlgChatSheetKind::Server)
            .cloned()
            .unwrap_or_else(|| {
                Self::new_chat_sheet(
                    NetDlgChatSheetKind::Server,
                    self.chat_strings.server.clone(),
                    self.chat_server.clone(),
                )
            });
        server.title.clone_from(&self.chat_strings.server);
        server.ident.clone_from(&self.chat_server);
        server.topic.clone_from(&self.chat_server);
        server.users.clear();
        if new_connection {
            server.lines.clear();
            server.unread = false;
            server.transcript_scroll = 0;
            server.transcript_follow_bottom = true;
        }
        let mut sheets = vec![server];
        let mut newest_channel = None;
        for mut channel in snapshot.channels {
            Self::sort_chat_users(&mut channel.users);
            let old = old_sheets
                .iter()
                .find(|sheet| {
                    !new_connection
                        && sheet.kind == NetDlgChatSheetKind::Channel
                        && sheet.ident.eq_ignore_ascii_case(&channel.name)
                })
                .cloned();
            let mut sheet = old.unwrap_or_else(|| {
                Self::new_chat_sheet(
                    NetDlgChatSheetKind::Channel,
                    channel.name.clone(),
                    channel.name.clone(),
                )
            });
            sheet.title.clone_from(&channel.name);
            sheet.ident = channel.name;
            sheet.topic = channel.topic;
            sheet.users = channel.users;
            sheets.push(sheet);
            if !existing_channels
                .iter()
                .any(|name| name.eq_ignore_ascii_case(&sheets.last().unwrap().ident))
            {
                newest_channel = Some(sheets.len() - 1);
            }
        }
        if !new_connection {
            sheets.extend(
                old_sheets
                    .iter()
                    .filter(|sheet| sheet.kind == NetDlgChatSheetKind::Query)
                    .cloned(),
            );
        }
        if new_connection {
            Self::append_chat_line(
                &mut sheets[0],
                NetDlgChatLine {
                    kind: NetDlgChatLineKind::Status,
                    text: substitute_resource_arguments(
                        &self.chat_strings.connecting,
                        &[&self.chat_server, ""],
                    ),
                    new_paragraph: true,
                },
                true,
                self.text_font.as_ref(),
                wrap_width,
            );
        }

        // Newly discovered channels are selected immediately. Notice routing
        // below therefore sees the same active sheet as native Update().
        let mut selected = newest_channel.unwrap_or_else(|| {
            if new_connection {
                return 0;
            }
            active
                .as_ref()
                .and_then(|(kind, ident)| {
                    sheets.iter().position(|sheet| {
                        sheet.kind == *kind && sheet.ident.eq_ignore_ascii_case(ident)
                    })
                })
                .unwrap_or(0)
        });

        let message_start = if self.chat_initial_messages_received {
            snapshot.unread_index.min(snapshot.messages.len())
        } else {
            0
        };
        for message in snapshot.messages.into_iter().skip(message_start) {
            let source_nick = message
                .source
                .split_once('!')
                .map_or(message.source.as_str(), |(nick, _)| nick);
            let target = if matches!(message.kind, NetDlgChatMessageKind::Server) {
                Some(0)
            } else if message.is_channel {
                // C++ discards late messages for a channel whose sheet was
                // already removed; they must never leak into Server.
                sheets
                    .iter()
                    .position(|sheet| {
                        sheet.kind == NetDlgChatSheetKind::Channel
                            && sheet.ident.eq_ignore_ascii_case(&message.target)
                    })
                    .map(Some)
                    .unwrap_or(None)
            } else if Self::is_irc_service(source_nick) {
                Some(0)
            } else if matches!(message.kind, NetDlgChatMessageKind::Notice) {
                // Native `Update` tests `MSG_Notice` before the empty-source
                // fallback, so a source-less notice stays on the active sheet
                // (C4ChatDlg.cpp:742-747).
                Some(selected.min(sheets.len().saturating_sub(1)))
            } else if matches!(message.kind, NetDlgChatMessageKind::Status)
                || source_nick.is_empty()
            {
                Some(0)
            } else {
                let outgoing = source_nick.eq_ignore_ascii_case(&snapshot.nick);
                let query = if outgoing {
                    message.target.as_str()
                } else {
                    source_nick
                };
                if Self::is_irc_service(query) {
                    Some(0)
                } else {
                    // `SplitAtChar('!')` leaves the ident behind the nick, and
                    // `OpenQuery` matches on it, so a nick change reuses and
                    // retitles the sheet. An own message passes no ident at all
                    // (C4ChatDlg.cpp:753-770,834-854).
                    let ident = if outgoing {
                        ""
                    } else {
                        message
                            .source
                            .split_once('!')
                            .map_or(message.source.as_str(), |(_, ident)| ident)
                    };
                    let query_index = Self::ensure_query_sheet(&mut sheets, query, ident);
                    if outgoing {
                        sheets[query_index].topic = query.to_string();
                        selected = query_index;
                        sheets[query_index].unread = false;
                    } else if !message.source.is_empty() {
                        sheets[query_index].topic.clone_from(&message.source);
                    }
                    Some(query_index)
                }
            };
            let Some(target) = target else {
                continue;
            };
            let line = NetDlgChatLine {
                kind: Self::chat_line_kind(message.kind),
                text: Self::format_chat_message(&message, source_nick, &snapshot.nick),
                new_paragraph: true,
            };
            Self::append_chat_line(
                &mut sheets[target],
                line,
                target == selected,
                self.text_font.as_ref(),
                wrap_width,
            );
        }
        if let Some(error) = snapshot.last_error.filter(|error| !error.is_empty()) {
            let line = NetDlgChatLine {
                kind: NetDlgChatLineKind::Error,
                text: format!("Error: {error}"),
                new_paragraph: true,
            };
            let target = self
                .chat_send_error_origin
                .take()
                .and_then(|(kind, ident)| {
                    sheets.iter().position(|sheet| {
                        sheet.kind == kind && sheet.ident.eq_ignore_ascii_case(&ident)
                    })
                })
                .unwrap_or(0);
            // The same error must not be repeated when a snapshot re-reports
            // it. Now that the transcript holds display lines, the comparison
            // is against the whole wrapped tail rather than one stored line.
            let mut candidate = line.clone();
            candidate.text = Self::sanitize_chat_line(&candidate.text);
            let display = Self::chat_display_lines(&candidate, self.text_font.as_ref(), wrap_width);
            let stored = &sheets[target].lines;
            let repeated = !display.is_empty()
                && stored.len() >= display.len()
                && stored[stored.len() - display.len()..] == display[..];
            if !repeated {
                Self::append_chat_line(
                    &mut sheets[target],
                    line,
                    target == selected,
                    self.text_font.as_ref(),
                    wrap_width,
                );
            }
        }

        self.chat_initial_messages_received = true;
        self.chat_connection_state = snapshot.connection_state;
        self.chat_sheets = sheets;
        self.chat_active_sheet = selected.min(self.chat_sheets.len().saturating_sub(1));
        if let Some(active) = self.chat_sheets.get_mut(self.chat_active_sheet) {
            active.unread = false;
        }
        self.clamp_active_chat_scroll();
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
        if self.mode == NetDlgMode::Chat {
            let chat = self.chat_layout();
            if self.chat_bounds_override.is_some()
                && contains(
                    Self::chat_dialog_close_rect(self.chat_caption_and_group().0),
                    point,
                )
            {
                return Some(StartupTooltip::resource("IDS_MNU_CLOSE"));
            }
            if self.chat_page() == NetDlgChatPage::Chats {
                if contains(chat.input, point)
                    || chat.input_label.is_some_and(|label| contains(label, point))
                {
                    return Some(StartupTooltip::resource("IDS_DLGTIP_CHAT"));
                }
                if let Some(users) = chat.users.filter(|users| contains(*users, point)) {
                    let row = ((point.y.floor() as i32 - users.y)
                        / self.metrics.text_line_height.max(1))
                    .max(0);
                    if let Some(user) = usize::try_from(row)
                        .ok()
                        .and_then(|row| self.active_chat_sheet()?.users.get(row))
                    {
                        return Some(StartupTooltip::text(format!(
                            "{}{}",
                            user.prefix, user.name
                        )));
                    }
                }
            }
            if self.chat_bounds_override.is_some() {
                return None;
            }
        }
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
            for edit in &mut self.chat_login_edits {
                edit.drag_anchor = None;
            }
            self.chat_edit.drag_anchor = None;
            self.chat_pressed = None;
            self.chat_dialog_drag = None;
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

    /// The periodic masterserver re-query changes the icon and nothing else, so
    /// this deliberately skips `clamp_list_scroll`: the row keeps its line count
    /// and C++ reaches no `UpdateEntrySize` on that path
    /// (C4StartupNetDlg.cpp:191-207).
    pub fn set_masterserver_row_icon(&mut self, row_icon: NetDlgRowIcon) {
        self.masterserver.row_icon = row_icon;
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

    /// Whether the Chat tab is shown, which decides button visibility exactly
    /// like `C4StartupNetDlg::UpdateCollapsed`.
    pub fn is_chat_mode(&self) -> bool {
        self.mode == NetDlgMode::Chat
    }

    pub fn masterserver_signup(&self) -> bool {
        self.config.masterserver_signup
    }

    pub fn list_is_collapsed(&self) -> bool {
        self.list_is_collapsed_with_font(&self.layout(), self.text_font.as_ref())
    }

    /// Adds text received from the windowing layer while the IP edit owns
    /// focus. `KeyCode` intentionally contains navigation keys only, so text
    /// input is a separate operation just like C4GUI::Edit::CharIn.
    pub fn handle_text_input(&mut self, text: &str, font: &ClonkFont) -> Vec<NetDlgAction> {
        let editing_join =
            self.focus == NetDlgControl::JoinAddress && self.mode == NetDlgMode::GameList;
        let editing_chat = self.focus == NetDlgControl::ChatInput
            && self.mode == NetDlgMode::Chat
            && !self.chat_connect_focused;
        if !editing_join && !editing_chat {
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
        if editing_chat {
            let rect = self.active_chat_edit_rect();
            let edit = self.active_chat_edit_mut();
            edit.insert_raw_text(&filtered);
            edit.ensure_cursor_in_view(rect, font);
            Vec::new()
        } else {
            let changed = self.join_edit.insert_raw_text(&filtered);
            self.join_edit
                .ensure_cursor_in_view(self.layout().join_edit, font);
            changed
                .then(|| NetDlgAction::JoinAddressChanged(self.join_edit.text.clone()))
                .into_iter()
                .collect()
        }
    }

    pub fn handle_edit_key_down(
        &mut self,
        key: NetDlgEditKey,
        modifiers: NetDlgEditModifiers,
        font: &ClonkFont,
    ) -> NetDlgEditInputOutcome {
        let editing_join =
            self.focus == NetDlgControl::JoinAddress && self.mode == NetDlgMode::GameList;
        let editing_chat = self.focus == NetDlgControl::ChatInput
            && self.mode == NetDlgMode::Chat
            && !self.chat_connect_focused;
        if !editing_join && !editing_chat {
            return NetDlgEditInputOutcome::passed();
        }
        if editing_chat {
            let rect = self.active_chat_edit_rect();
            self.active_chat_edit_mut()
                .handle_key(key, modifiers, rect, font);
            return NetDlgEditInputOutcome::captured(Vec::new());
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
        let editing_join =
            self.focus == NetDlgControl::JoinAddress && self.mode == NetDlgMode::GameList;
        let editing_chat = self.focus == NetDlgControl::ChatInput
            && self.mode == NetDlgMode::Chat
            && !self.chat_connect_focused;
        if !editing_join && !editing_chat {
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
        let editing_chat = self.mode == NetDlgMode::Chat
            && self.focus == NetDlgControl::ChatInput
            && !self.chat_connect_focused;
        if editing_chat {
            let rect = self.active_chat_edit_rect();
            match command {
                NetDlgEditContextCommand::Copy => return self.begin_clipboard_transfer(false),
                NetDlgEditContextCommand::Cut => return self.begin_clipboard_transfer(true),
                NetDlgEditContextCommand::Paste => {
                    let Some(text) = clipboard_text.filter(|text| !text.is_empty()) else {
                        return Vec::new();
                    };
                    return self.paste_chat_text(text, rect, font);
                }
                NetDlgEditContextCommand::Clear => {
                    let edit = self.active_chat_edit_mut();
                    edit.pending_cut = None;
                    edit.delete_selection();
                    edit.ensure_cursor_in_view(rect, font);
                    return Vec::new();
                }
                NetDlgEditContextCommand::SelectAll => {
                    let edit = self.active_chat_edit_mut();
                    edit.pending_cut = None;
                    edit.caret = edit.text.len();
                    edit.selection = (!edit.text.is_empty()).then_some((0, edit.text.len()));
                    return Vec::new();
                }
            }
        }
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

    fn paste_chat_text(
        &mut self,
        text: &str,
        edit_rect: IntRect,
        font: &ClonkFont,
    ) -> Vec<NetDlgAction> {
        let mapped = text
            .chars()
            .filter(|character| {
                matches!(character, '\r' | '\n')
                    || (!character.is_control() && *character != '\u{7f}')
            })
            .map(|character| {
                if character == '|' {
                    '\u{a6}'
                } else {
                    character
                }
            })
            .collect::<String>();
        let mut remaining = mapped.as_str();
        let mut actions = Vec::new();
        loop {
            let line_break = remaining.char_indices().find_map(|(index, character)| {
                matches!(character, '\r' | '\n').then_some((index, character.len_utf8()))
            });
            let Some((index, break_len)) = line_break else {
                break;
            };
            if index == 0 {
                remaining = &remaining[break_len..];
                continue;
            }
            let line = &remaining[..index];
            {
                let edit = self.active_chat_edit_mut();
                edit.insert_raw_text(line);
                edit.ensure_cursor_in_view(edit_rect, font);
            }
            let (line_actions, abort) = self.submit_chat_input_with_paste_result();
            actions.extend(line_actions);
            remaining = &remaining[index + break_len..];
            if abort {
                return actions;
            }
        }
        if !remaining.is_empty() {
            let edit = self.active_chat_edit_mut();
            edit.insert_raw_text(remaining);
            edit.ensure_cursor_in_view(edit_rect, font);
        }
        actions
    }

    /// Completes a pending cut only after the host successfully wrote the
    /// matching selection to the native clipboard.
    pub fn confirm_clipboard_cut(&mut self, font: &ClonkFont) -> Vec<NetDlgAction> {
        if self.mode == NetDlgMode::Chat && self.focus == NetDlgControl::ChatInput {
            let rect = self.active_chat_edit_rect();
            let Some(pending) = self.active_chat_edit_mut().pending_cut.take() else {
                return Vec::new();
            };
            let edit = self.active_chat_edit_mut();
            if edit.selected_range() != Some(pending.range)
                || edit.text.get(pending.range.0..pending.range.1) != Some(pending.text.as_str())
                || !edit.delete_selection()
            {
                return Vec::new();
            }
            edit.ensure_cursor_in_view(rect, font);
            return Vec::new();
        }
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
        if self.mode == NetDlgMode::Chat {
            match self.chat_hit(point) {
                Some(NetDlgChatHit::LoginField(field)) => self.select_chat_login_field(field),
                Some(NetDlgChatHit::Input) => {}
                _ => return NetDlgEditInputOutcome::passed(),
            }
            self.chat_connect_focused = false;
            let mut actions = self.change_focus(NetDlgControl::ChatInput);
            let request =
                Self::edit_context_request(self.active_chat_edit_mut(), point, clipboard_has_text);
            actions.push(NetDlgAction::OpenJoinAddressContextMenu(request));
            return NetDlgEditInputOutcome::captured(actions);
        }
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
        if self.mode == NetDlgMode::Chat
            && self.focus == NetDlgControl::ChatInput
            && !self.chat_connect_focused
        {
            let edit = self.active_chat_edit_rect();
            let anchor = GuiPoint::new((edit.x + edit.w / 2) as f32, (edit.y + edit.h / 2) as f32);
            return NetDlgEditInputOutcome::captured(vec![
                NetDlgAction::OpenJoinAddressContextMenu(Self::edit_context_request(
                    self.active_chat_edit_mut(),
                    anchor,
                    clipboard_has_text,
                )),
            ]);
        }
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
        if self.mode == NetDlgMode::Chat {
            match self.chat_hit(point) {
                Some(NetDlgChatHit::LoginField(field)) => self.select_chat_login_field(field),
                Some(NetDlgChatHit::Input) => {}
                _ => return NetDlgEditInputOutcome::passed(),
            }
            self.chat_connect_focused = false;
            let actions = self.change_focus(NetDlgControl::ChatInput);
            let edit_rect = self.active_chat_edit_rect();
            let edit = self.active_chat_edit_mut();
            edit.pending_cut = None;
            edit.caret = edit.character_at(point.x, edit_rect, font);
            edit.selection = Some((edit.caret, edit.caret));
            if let Some(text) = primary_selection.filter(|text| !text.is_empty()) {
                edit.insert_raw_text(text);
            }
            edit.ensure_cursor_in_view(edit_rect, font);
            return NetDlgEditInputOutcome::captured(actions);
        }
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
        if self.chat_dialog_drag.is_some() {
            self.update_chat_dialog_drag(position);
            self.hovered = None;
            return Vec::new();
        }
        self.hovered = self.hit_button(position);
        let layout = self.layout();
        if self.join_edit.drag_anchor.is_some() {
            let character = self
                .join_edit
                .character_at(position.x, layout.join_edit, font);
            self.join_edit
                .drag_pointer_selection(character, layout.join_edit, font);
        }
        if self.mode == NetDlgMode::Chat && self.focus == NetDlgControl::ChatInput {
            let edit_rect = self.active_chat_edit_rect();
            if self.active_chat_edit_mut().drag_anchor.is_some() {
                let character = self
                    .active_chat_edit_mut()
                    .character_at(position.x, edit_rect, font);
                self.active_chat_edit_mut()
                    .drag_pointer_selection(character, edit_rect, font);
            }
        }
        if self.chat_transcript_scrollbar_dragging {
            self.set_chat_transcript_scroll_from_pointer(position);
            return Vec::new();
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

        if self.mode == NetDlgMode::Chat {
            self.chat_pressed = self.chat_hit(position);
            if let Some(hit) = self.chat_pressed {
                if hit == NetDlgChatHit::DialogCaption {
                    if let Some(bounds) = self.chat_bounds_override {
                        self.chat_dialog_drag = Some(NetDlgChatDialogDrag {
                            pointer: position,
                            bounds,
                        });
                    }
                    self.chat_pressed = None;
                    self.pointer_pressed = None;
                    return Vec::new();
                }
                if let NetDlgChatHit::TabClose(index) = hit {
                    self.chat_pressed = None;
                    self.pointer_pressed = None;
                    return self.close_chat_sheet(index);
                }
                if let NetDlgChatHit::Tab(index) = hit {
                    self.chat_pressed = None;
                    self.pointer_pressed = None;
                    return self.select_chat_sheet(index);
                }
                let mut actions = self.change_focus(NetDlgControl::ChatInput);
                match hit {
                    NetDlgChatHit::LoginField(field) => {
                        self.select_chat_login_field(field);
                        self.chat_connect_focused = false;
                        let edit_rect = self.active_chat_edit_rect();
                        let character = self
                            .active_chat_edit_mut()
                            .character_at(position.x, edit_rect, font);
                        self.active_chat_edit_mut()
                            .begin_pointer_selection(character, edit_rect, font);
                    }
                    NetDlgChatHit::Input => {
                        self.chat_connect_focused = false;
                        let edit_rect = self.active_chat_edit_rect();
                        let character = self
                            .active_chat_edit_mut()
                            .character_at(position.x, edit_rect, font);
                        self.active_chat_edit_mut()
                            .begin_pointer_selection(character, edit_rect, font);
                    }
                    NetDlgChatHit::Connect => self.chat_connect_focused = true,
                    NetDlgChatHit::TranscriptScrollUp => {
                        self.scroll_active_chat_by(-self.metrics.text_line_height.max(1));
                    }
                    NetDlgChatHit::TranscriptScrollDown => {
                        self.scroll_active_chat_by(self.metrics.text_line_height.max(1));
                    }
                    NetDlgChatHit::TranscriptScrollPin => {
                        self.chat_transcript_scrollbar_dragging = true;
                    }
                    NetDlgChatHit::TranscriptScrollTrack => {
                        self.set_chat_transcript_scroll_from_pointer(position);
                        self.chat_transcript_scrollbar_dragging = true;
                    }
                    NetDlgChatHit::DialogClose
                    | NetDlgChatHit::DialogCaption
                    | NetDlgChatHit::Tab(_)
                    | NetDlgChatHit::TabClose(_)
                    | NetDlgChatHit::Transcript
                    | NetDlgChatHit::User(_) => {}
                }
                return std::mem::take(&mut actions);
            }
        }

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
        if self.chat_dialog_drag.is_some() {
            self.update_chat_dialog_drag(position);
            self.chat_dialog_drag = None;
            self.chat_pressed = None;
            self.pointer_pressed = None;
            self.hovered = self.hit_button(position);
            return Vec::new();
        }
        self.hovered = self.hit_button(position);
        self.join_edit.drag_anchor = None;
        for edit in &mut self.chat_login_edits {
            edit.drag_anchor = None;
        }
        self.chat_edit.drag_anchor = None;
        if self.mode == NetDlgMode::Chat {
            if let Some(pressed) = self.chat_pressed.take() {
                if self.chat_hit(position) == Some(pressed) {
                    return match pressed {
                        NetDlgChatHit::DialogClose => {
                            vec![NetDlgAction::ChatDialogCloseRequested]
                        }
                        NetDlgChatHit::Connect => self.submit_chat_login(),
                        NetDlgChatHit::Tab(_) | NetDlgChatHit::TabClose(_) => Vec::new(),
                        NetDlgChatHit::LoginField(_)
                        | NetDlgChatHit::DialogCaption
                        | NetDlgChatHit::Input
                        | NetDlgChatHit::Transcript
                        | NetDlgChatHit::TranscriptScrollUp
                        | NetDlgChatHit::TranscriptScrollDown
                        | NetDlgChatHit::TranscriptScrollPin
                        | NetDlgChatHit::TranscriptScrollTrack
                        | NetDlgChatHit::User(_) => Vec::new(),
                    };
                }
            }
        }
        if self.chat_transcript_scrollbar_dragging {
            self.set_chat_transcript_scroll_from_pointer(position);
            self.chat_transcript_scrollbar_dragging = false;
            return Vec::new();
        }
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
        if self.mode == NetDlgMode::Chat {
            if let Some(NetDlgChatHit::User(index)) = self.chat_hit(position) {
                let Some(user) = self
                    .active_chat_sheet()
                    .and_then(|sheet| sheet.users.get(index))
                    .cloned()
                else {
                    return Vec::new();
                };
                let query = Self::ensure_query_sheet(&mut self.chat_sheets, &user.name, &user.name);
                self.chat_active_sheet = query;
                return vec![NetDlgAction::ChatCommand(NetDlgChatCommand::OpenQuery {
                    nick: user.name,
                })];
            }
            let Some(NetDlgChatHit::LoginField(_) | NetDlgChatHit::Input) = self.chat_hit(position)
            else {
                return Vec::new();
            };
            let edit_rect = self.active_chat_edit_rect();
            let character = self
                .active_chat_edit_mut()
                .character_at(position.x, edit_rect, font);
            self.active_chat_edit_mut()
                .select_word_at(character, edit_rect, font);
            return self.change_focus(NetDlgControl::ChatInput);
        }
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
        if self.mode == NetDlgMode::Chat && self.chat_page == NetDlgChatPage::Chats {
            let chat = self.chat_layout();
            if contains(chat.transcript, position) {
                self.scroll_active_chat_by(delta.saturating_neg());
                return Vec::new();
            }
            if chat.users.is_some_and(|users| contains(users, position)) {
                self.scroll_active_chat_users_by(delta.saturating_neg());
                return Vec::new();
            }
        }
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
        if self.mode == NetDlgMode::Chat && self.focus == NetDlgControl::ChatInput {
            match key {
                KeyCode::Tab if self.chat_page() == NetDlgChatPage::Login => {
                    if self.chat_connect_focused {
                        if backwards {
                            self.chat_connect_focused = false;
                            self.select_chat_login_field(NetDlgChatLoginField::Channel);
                            return Vec::new();
                        }
                        return self.move_focus(false);
                    }
                    let index = self.chat_login_field.index();
                    if backwards && index == 0 {
                        return self.move_focus(true);
                    }
                    if !backwards && index == NetDlgChatLoginField::ALL.len() - 1 {
                        self.chat_connect_focused = true;
                        self.chat_login_edits[index].blur();
                        return Vec::new();
                    }
                    let next = if backwards { index - 1 } else { index + 1 };
                    self.select_chat_login_field(NetDlgChatLoginField::ALL[next]);
                    return Vec::new();
                }
                KeyCode::Enter if self.chat_page() == NetDlgChatPage::Login => {
                    if self.chat_connect_focused {
                        return self.submit_chat_login();
                    }
                    let index = self.chat_login_field.index();
                    if index + 1 < NetDlgChatLoginField::ALL.len() {
                        self.select_chat_login_field(NetDlgChatLoginField::ALL[index + 1]);
                    } else {
                        self.chat_login_edits[index].blur();
                        self.chat_connect_focused = true;
                    }
                    return Vec::new();
                }
                KeyCode::Enter if self.chat_page() == NetDlgChatPage::Chats => {
                    return self.submit_chat_input();
                }
                KeyCode::Up if self.chat_page() == NetDlgChatPage::Chats => {
                    if self.chat_history.is_empty() {
                        return Vec::new();
                    }
                    let next = self.chat_history_index.map_or(0, |index| index + 1);
                    if let Some(text) = self.chat_history.get(next) {
                        self.chat_history_index = Some(next);
                        self.chat_edit.set_text(text);
                        self.chat_edit.selection = Some((0, self.chat_edit.text.len()));
                    } else {
                        self.chat_history_index = None;
                        self.chat_edit.set_text("");
                    }
                    return Vec::new();
                }
                KeyCode::Down if self.chat_page() == NetDlgChatPage::Chats => {
                    let Some(index) = self.chat_history_index else {
                        self.chat_edit.set_text("");
                        return Vec::new();
                    };
                    if index == 0 {
                        self.chat_history_index = None;
                        self.chat_edit.set_text("");
                    } else {
                        let next = index - 1;
                        self.chat_history_index = Some(next);
                        self.chat_edit.set_text(&self.chat_history[next]);
                        self.chat_edit.selection = Some((0, self.chat_edit.text.len()));
                    }
                    return Vec::new();
                }
                _ => {}
            }
        }
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
            KeyCode::Home if self.focus == NetDlgControl::GameList => {
                self.select_list_boundary(false)
            }
            KeyCode::End if self.focus == NetDlgControl::GameList => {
                self.select_list_boundary(true)
            }
            KeyCode::PageUp if self.focus == NetDlgControl::GameList => {
                self.page_list_selection(false)
            }
            KeyCode::PageDown if self.focus == NetDlgControl::GameList => {
                self.page_list_selection(true)
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
        let edit = if self.mode == NetDlgMode::Chat && self.focus == NetDlgControl::ChatInput {
            self.active_chat_edit_mut()
        } else {
            &mut self.join_edit
        };
        let Some((range, text)) = edit
            .selected_range()
            .zip(edit.selected_text().map(str::to_string))
        else {
            edit.pending_cut = None;
            return Vec::new();
        };
        edit.pending_cut = cut.then(|| PendingClipboardCut {
            range,
            text: text.clone(),
        });
        vec![NetDlgAction::ClipboardTransfer { text, cut }]
    }

    fn paste_join_address(&mut self, clipboard: &str, font: &ClonkFont) -> Vec<NetDlgAction> {
        let transformed = clipboard.replace('|', "\u{a6}");
        let mut rest = transformed.as_str();
        while let Some(line_break) = rest.find(['\r', '\n']) {
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
        Self::edit_context_request(&self.join_edit, anchor, clipboard_has_text)
    }

    fn edit_context_request(
        edit: &NetDlgEditState,
        anchor: GuiPoint,
        clipboard_has_text: bool,
    ) -> NetDlgEditContextRequest {
        let has_selection = edit.selected_range().is_some();
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
        let whole_text_selected = edit.selected_range() == Some((0, edit.text.len()));
        if !edit.text.is_empty() && !whole_text_selected {
            items.push(item(
                NetDlgEditContextCommand::SelectAll,
                "Select all",
                "Selects the complete text",
            ));
        }
        NetDlgEditContextRequest { anchor, items }
    }

    fn new_chat_sheet(kind: NetDlgChatSheetKind, title: String, ident: String) -> NetDlgChatSheet {
        NetDlgChatSheet {
            kind,
            title,
            ident,
            topic: String::new(),
            users: Vec::new(),
            lines: Vec::new(),
            unread: false,
            transcript_scroll: 0,
            transcript_follow_bottom: true,
            user_scroll: 0,
        }
    }

    fn chat_user_privilege(user: &NetDlgChatUser) -> u8 {
        match user.prefix.as_bytes().first().copied() {
            Some(b'!') => 4,
            Some(b'@') => 3,
            Some(b'%') => 2,
            Some(b'+') => 1,
            _ => 0,
        }
    }

    fn sort_chat_users(users: &mut [NetDlgChatUser]) {
        users.sort_by(|left, right| {
            Self::chat_user_privilege(right)
                .cmp(&Self::chat_user_privilege(left))
                .then_with(|| {
                    left.name
                        .to_ascii_lowercase()
                        .cmp(&right.name.to_ascii_lowercase())
                })
                .then_with(|| left.name.cmp(&right.name))
        });
    }

    fn sanitize_chat_line(text: &str) -> String {
        text.chars()
            .map(|character| {
                if ('\u{1}'..' ').contains(&character) {
                    ' '
                } else {
                    character
                }
            })
            .collect()
    }

    /// `C4ChatControl::ChatSheet` builds its transcript as a plain
    /// `C4GUI::TextWindow(rcDefault)` (src/C4ChatDlg.cpp:194), which takes the
    /// constructor's `iMaxLines = 100` / `iMaxTextLen = 4096` defaults
    /// (src/C4Gui.h:1309). Those reach a `C4LogBuffer` that evicts from the
    /// front - `DiscardFirstLine` on reaching the line cap, and again until an
    /// oversized message fits the character ring
    /// (src/C4LogBuf.cpp:96-148), so an IRC session cannot retain unbounded
    /// scrollback.
    fn bound_chat_transcript(sheet: &mut NetDlgChatSheet) {
        while sheet.lines.len() > CHAT_TRANSCRIPT_MAX_LINES {
            sheet.lines.remove(0);
        }
        // The native buffer counts the trailing NUL of every stored line.
        let mut budget = sheet
            .lines
            .iter()
            .map(|line| line.text.len() + 1)
            .sum::<usize>();
        while budget > CHAT_TRANSCRIPT_MAX_TEXT && !sheet.lines.is_empty() {
            budget -= sheet.lines[0].text.len() + 1;
            sheet.lines.remove(0);
        }
    }

    /// The display lines one incoming message becomes, in `AppendLines`
    /// order: split on CR/LF — `|` stays literal, because chat TextWindows are
    /// built with `fMarkup = false` (C4Gui.h:1309; C4LogBuf.cpp:174-205) —
    /// then word-wrapped at the width in force right now. Empty parts are
    /// dropped, matching `AppendSingleLine`'s "do not append empty line"
    /// guard (C4LogBuf.cpp:98).
    fn chat_display_lines(
        line: &NetDlgChatLine,
        font: Option<&ClonkFont>,
        width: i32,
    ) -> Vec<NetDlgChatLine> {
        let mut display = Vec::new();
        let mut first_physical = true;
        for paragraph in line.text.split(['\r', '\n']) {
            let broken = font.map_or_else(
                || paragraph.to_string(),
                |font| break_message(font, paragraph, width.max(1)),
            );
            for physical in broken.split('\n') {
                if physical.is_empty() {
                    continue;
                }
                display.push(NetDlgChatLine {
                    kind: line.kind,
                    text: physical.to_string(),
                    new_paragraph: first_physical,
                });
                first_physical = false;
            }
        }
        display
    }

    fn append_chat_line(
        sheet: &mut NetDlgChatSheet,
        mut line: NetDlgChatLine,
        active: bool,
        font: Option<&ClonkFont>,
        width: i32,
    ) {
        line.text = Self::sanitize_chat_line(&line.text);
        // `AppendSingleLine` applies the line and character caps to each
        // wrapped line as it is stored, so a message long enough to overflow
        // the transcript on its own loses its own leading lines
        // (C4LogBuf.cpp:100-104).
        for display in Self::chat_display_lines(&line, font, width) {
            sheet.lines.push(display);
            Self::bound_chat_transcript(sheet);
        }
        // TextWindow::AddTextLine always ScrollToBottom, even on an inactive
        // tab. The inactive caption is then marked unread.
        sheet.transcript_follow_bottom = true;
        sheet.unread = !active;
    }

    const fn chat_line_kind(kind: NetDlgChatMessageKind) -> NetDlgChatLineKind {
        match kind {
            NetDlgChatMessageKind::Server => NetDlgChatLineKind::Server,
            NetDlgChatMessageKind::Status => NetDlgChatLineKind::Status,
            NetDlgChatMessageKind::Message => NetDlgChatLineKind::Message,
            NetDlgChatMessageKind::Notice => NetDlgChatLineKind::Notice,
            NetDlgChatMessageKind::Action => NetDlgChatLineKind::Action,
        }
    }

    fn ensure_query_sheet(sheets: &mut Vec<NetDlgChatSheet>, title: &str, ident: &str) -> usize {
        if let Some(index) = sheets.iter().position(|sheet| {
            sheet.kind == NetDlgChatSheetKind::Query
                && (sheet.title.eq_ignore_ascii_case(title)
                    || (!ident.is_empty() && sheet.ident.eq_ignore_ascii_case(ident)))
        }) {
            sheets[index].title = title.to_string();
            if !ident.is_empty() {
                sheets[index].ident = ident.to_string();
            }
            return index;
        }
        let ident = if ident.is_empty() { title } else { ident };
        let mut sheet = Self::new_chat_sheet(
            NetDlgChatSheetKind::Query,
            title.to_string(),
            ident.to_string(),
        );
        sheet.topic = ident.to_string();
        sheets.push(sheet);
        sheets.len() - 1
    }

    fn is_irc_service(name: &str) -> bool {
        ["NickServ", "ChanServ", "MemoServ", "HelpServ", "Global"]
            .iter()
            .any(|service| service.eq_ignore_ascii_case(name))
    }

    fn valid_irc_nick(nick: &str) -> bool {
        let bytes = nick.as_bytes();
        let valid_first = |byte: u8| {
            byte.is_ascii_alphabetic() || matches!(byte, b'_' | b'^' | b'{' | b'[' | b']' | b'}')
        };
        let valid_rest =
            |byte: u8| valid_first(byte) || byte.is_ascii_digit() || matches!(byte, b'|' | b'-');
        (2..=30).contains(&bytes.len())
            && bytes.first().copied().is_some_and(valid_first)
            && bytes[1..].iter().copied().all(valid_rest)
            && !["NickServ", "ChanServ", "MemoServ", "OperServ", "HelpServ"]
                .iter()
                .any(|service| service.eq_ignore_ascii_case(nick))
    }

    fn valid_irc_password(password: &str) -> bool {
        if password.is_empty() {
            return true;
        }
        let Some(bytes) = clonk_resources::encode_legacy_script_text(password) else {
            return false;
        };
        (2..=31).contains(&bytes.len()) && !bytes.contains(&b' ')
    }

    fn valid_irc_channel(channel: &str) -> bool {
        if channel.is_empty() {
            return true;
        }
        let Some(bytes) = clonk_resources::encode_legacy_script_text(channel) else {
            return false;
        };
        (2..=32).contains(&bytes.len())
            && matches!(bytes.first(), Some(b'#' | b'+'))
            && !bytes.contains(&b' ')
    }

    fn format_chat_message(message: &NetDlgChatMessage, source: &str, own_nick: &str) -> String {
        match message.kind {
            NetDlgChatMessageKind::Server | NetDlgChatMessageKind::Status => {
                format!("- {}", message.text)
            }
            NetDlgChatMessageKind::Notice if source.is_empty() => {
                format!("* {}", message.text)
            }
            NetDlgChatMessageKind::Notice if source.eq_ignore_ascii_case(own_nick) => {
                format!("-> -{}- {}", message.target, message.text)
            }
            NetDlgChatMessageKind::Notice => format!("-{source}- {}", message.text),
            NetDlgChatMessageKind::Message if source.is_empty() => {
                format!("* {}", message.text)
            }
            NetDlgChatMessageKind::Message
                if source.eq_ignore_ascii_case(own_nick)
                    && Self::is_irc_service(&message.target) =>
            {
                format!("-> *{}* {}", message.target, message.text)
            }
            NetDlgChatMessageKind::Message if source.eq_ignore_ascii_case(own_nick) => {
                format!("<{source}> {}", message.text)
            }
            NetDlgChatMessageKind::Message => format!("<{source}> {}", message.text),
            NetDlgChatMessageKind::Action if source.is_empty() => {
                format!("* {}", message.text)
            }
            NetDlgChatMessageKind::Action => format!("* {source} {}", message.text),
        }
    }

    fn chat_title(&self) -> String {
        if self.chat_page() == NetDlgChatPage::Login {
            return self.chat_strings.not_connected.clone();
        }
        let Some(sheet) = self.active_chat_sheet() else {
            return String::new();
        };
        match sheet.kind {
            NetDlgChatSheetKind::Server => self.chat_server.clone(),
            NetDlgChatSheetKind::Channel if !sheet.topic.is_empty() => {
                format!("{}: {}", sheet.ident, sheet.topic)
            }
            NetDlgChatSheetKind::Channel | NetDlgChatSheetKind::Query => {
                if sheet.topic.is_empty() {
                    sheet.title.clone()
                } else {
                    sheet.topic.clone()
                }
            }
        }
    }

    fn embedded_chat_group(&self) -> IntRect {
        let layout = self.layout();
        IntRect::new(
            layout.game_list.x,
            layout.game_list.y,
            layout.game_list.w,
            layout.join_edit.y + layout.join_edit.h - layout.game_list.y,
        )
    }

    fn chat_caption_and_group(&self) -> (IntRect, IntRect) {
        if let Some(dialog) = self.chat_bounds_override {
            let caption_h = self.metrics.text_line_height.max(23).min(dialog.h);
            return (
                IntRect::new(dialog.x, dialog.y, dialog.w, caption_h),
                IntRect::new(
                    dialog.x,
                    dialog.y + caption_h,
                    dialog.w,
                    (dialog.h - caption_h).max(0),
                ),
            );
        }
        (self.layout().game_list_caption, self.embedded_chat_group())
    }

    fn chat_layout(&self) -> NetDlgChatLayout {
        self.chat_layout_in(self.chat_caption_and_group().1)
    }

    /// The width `AppendLines` word-wraps against: the transcript viewport the
    /// renderer draws into, less the same 6px the draw path subtracts.
    fn chat_transcript_wrap_width(&self) -> i32 {
        (self.chat_layout().transcript_viewport.w - 6).max(1)
    }

    fn chat_dialog_close_rect(caption: IntRect) -> IntRect {
        IntRect::new(caption.x + caption.w - 20, caption.y + 4, 16, 16)
    }

    fn chat_layout_in(&self, group: IntRect) -> NetDlgChatLayout {
        let inner = IntRect::new(
            group.x + 2,
            group.y + 2,
            (group.w - 4).max(0),
            (group.h - 4).max(0),
        );
        let label_h = self.metrics.text_line_height;
        let edit_h = (self.metrics.text_line_height + 3).max(23);
        let login_h = (label_h * 8 + 2 * 10 + 5 * 10 + 32 + 20).min(inner.h);
        let login_w = (login_h * 2 / 3).min(inner.w);
        let centered = Aligner::new(inner, 0, 0).centered(login_w, login_h);
        let mut login = Aligner::new(centered, 2, 2);
        let mut login_labels = [IntRect::default(); 4];
        let mut login_edits = [IntRect::default(); 4];
        for field in NetDlgChatLoginField::ALL {
            login_labels[field.index()] = login.get_from_top(label_h, None);
            login_edits[field.index()] = login.get_from_top(edit_h, None);
            login.expand_top(2 * (2 - 5));
        }
        let connect = login.get_from_top(32, Some(140.min(login_w.saturating_sub(4))));

        let tab_h = (self.metrics.text_line_height + 8).max(23);
        let tab_caption_h = self.metrics.text_line_height.max(1);
        let tab_close_size = (tab_caption_h - 2).max(1);
        let natural_widths = self
            .chat_sheets
            .iter()
            .map(|sheet| {
                self.text_font.as_ref().map_or_else(
                    || i32::try_from(sheet.title.chars().count()).unwrap_or(i32::MAX) * 8,
                    |font| font.measure(&sheet.title, true).0,
                ) + tab_caption_h
                    + 10
            })
            .map(|width| width.max(tab_h + tab_caption_h))
            .collect::<Vec<_>>();
        let natural_total = natural_widths.iter().copied().sum::<i32>();
        let squeezed = if natural_total > inner.w && !natural_widths.is_empty() {
            Some((inner.w / i32::try_from(natural_widths.len()).unwrap_or(1)).max(1))
        } else {
            None
        };
        let mut tab_x = inner.x;
        let tabs = natural_widths
            .into_iter()
            .map(|natural| {
                let width = squeezed.unwrap_or(natural);
                let rect = IntRect::new(tab_x, inner.y, width, tab_h);
                tab_x = tab_x.saturating_add(width);
                NetDlgChatTabLayout {
                    rect,
                    close: IntRect::new(
                        rect.x + rect.w - tab_caption_h - 4,
                        rect.y + 1,
                        tab_close_size,
                        tab_close_size,
                    ),
                }
            })
            .collect();
        let input_h = edit_h;
        let input_row = IntRect::new(inner.x, inner.y + inner.h - input_h, inner.w, input_h);
        let active_kind = self.active_chat_sheet().map(|sheet| sheet.kind);
        let users_w = if active_kind == Some(NetDlgChatSheetKind::Channel) {
            (inner.w / 5).max(100).min(inner.w)
        } else {
            0
        };
        let users = (users_w > 0).then_some(IntRect::new(
            inner.x + inner.w - users_w,
            inner.y + tab_h,
            users_w,
            (inner.h - tab_h - input_h).max(0),
        ));
        let input_label = (active_kind != Some(NetDlgChatSheetKind::Server))
            .then_some(IntRect::new(input_row.x, input_row.y, 40, input_row.h));
        let label_w = input_label.map_or(0, |label| label.w);
        let transcript = IntRect::new(
            inner.x,
            inner.y + tab_h,
            (inner.w - users_w).max(0),
            (inner.h - tab_h - input_h).max(0),
        );
        let scrollbar_w = SCROLLBAR_WIDTH.min(transcript.w.max(0));
        NetDlgChatLayout {
            group,
            login_labels,
            login_edits,
            connect,
            tabs,
            transcript,
            transcript_viewport: IntRect::new(
                transcript.x,
                transcript.y,
                (transcript.w - scrollbar_w).max(0),
                transcript.h,
            ),
            transcript_scrollbar: IntRect::new(
                transcript.x + transcript.w - scrollbar_w,
                transcript.y,
                scrollbar_w,
                transcript.h,
            ),
            users,
            input_label,
            input: IntRect::new(
                input_row.x + label_w,
                input_row.y,
                (input_row.w - label_w).max(0),
                input_row.h,
            ),
        }
    }

    /// The transcript is already stored as display lines, wrapped when each
    /// message arrived. `TextWindow::UpdateSize` re-bounds the buffer control
    /// without re-flowing its contents (src/C4GuiLabels.cpp:490-500), so a
    /// later width or font change must not re-wrap them here either.
    fn wrapped_chat_lines(sheet: &NetDlgChatSheet) -> Vec<NetDlgWrappedChatLine> {
        sheet
            .lines
            .iter()
            .map(|line| NetDlgWrappedChatLine {
                text: line.text.clone(),
                kind: line.kind,
                new_paragraph: line.new_paragraph,
            })
            .collect()
    }

    fn chat_transcript_content_height(&self, index: usize, _layout: &NetDlgChatLayout) -> i32 {
        let Some(sheet) = self.chat_sheets.get(index) else {
            return 0;
        };
        let line_h = self.metrics.text_line_height.max(1);
        Self::wrapped_chat_lines(sheet)
            .iter()
            .enumerate()
            .map(|(index, line)| {
                line_h
                    + if index > 0 && line.new_paragraph {
                        line_h / 3
                    } else {
                        0
                    }
            })
            .sum()
    }

    fn chat_transcript_max_scroll_for(&self, index: usize, layout: &NetDlgChatLayout) -> i32 {
        (self.chat_transcript_content_height(index, layout) - layout.transcript_viewport.h).max(0)
    }

    pub fn chat_transcript_scroll_offset(&self) -> i32 {
        self.active_chat_sheet()
            .map_or(0, |sheet| sheet.transcript_scroll)
    }

    pub fn chat_transcript_max_scroll(&self) -> i32 {
        self.chat_transcript_max_scroll_for(self.chat_active_sheet, &self.chat_layout())
    }

    pub fn chat_transcript_follows_bottom(&self) -> bool {
        self.active_chat_sheet()
            .is_none_or(|sheet| sheet.transcript_follow_bottom)
    }

    fn clamp_active_chat_scroll(&mut self) {
        if self.chat_sheets.is_empty() {
            return;
        }
        let layout = self.chat_layout();
        let max_scroll = self.chat_transcript_max_scroll_for(self.chat_active_sheet, &layout);
        if let Some(sheet) = self.chat_sheets.get_mut(self.chat_active_sheet) {
            if sheet.transcript_follow_bottom {
                sheet.transcript_scroll = max_scroll;
            } else {
                sheet.transcript_scroll = sheet.transcript_scroll.clamp(0, max_scroll);
                sheet.transcript_follow_bottom = sheet.transcript_scroll == max_scroll;
            }
        }
    }

    /// Maximum `ScrollWindow` offset of the nick `ListBox`: the rows that do
    /// not fit its viewport (src/C4GuiContainers.cpp:477-623).
    pub(crate) fn chat_users_max_scroll(&self, line_height: i32) -> i32 {
        let Some(users) = self.chat_layout().users else {
            return 0;
        };
        let Some(sheet) = self.chat_sheets.get(self.chat_active_sheet) else {
            return 0;
        };
        let content = i32::try_from(sheet.users.len())
            .unwrap_or(i32::MAX)
            .saturating_mul(line_height.max(1));
        (content - users.h).max(0)
    }

    fn scroll_active_chat_users_by(&mut self, delta: i32) {
        let line_height = self.metrics.text_line_height.max(1);
        let max_scroll = self.chat_users_max_scroll(line_height);
        if let Some(sheet) = self.chat_sheets.get_mut(self.chat_active_sheet) {
            sheet.user_scroll = sheet.user_scroll.saturating_add(delta).clamp(0, max_scroll);
        }
    }

    /// `C4GUI::ScrollBar`'s pointer regions: the two `C4GUI_ScrollArrowHgt`
    /// arrow buttons and, between them, the draggable pin over a pageable
    /// track (src/C4GuiContainers.cpp:477-623). The bar exists only while the
    /// transcript overflows.
    fn chat_transcript_scrollbar_hit(
        &self,
        layout: &NetDlgChatLayout,
        point: GuiPoint,
    ) -> Option<NetDlgChatHit> {
        let bar = layout.transcript_scrollbar;
        let max_scroll = self.chat_transcript_max_scroll_for(self.chat_active_sheet, layout);
        if max_scroll <= 0 || !contains(bar, point) {
            return None;
        }
        let arrow = CHAT_SCROLL_ARROW_EXTENT.min(bar.h / 2);
        let y = point.y.floor() as i32;
        if y < bar.y + arrow {
            return Some(NetDlgChatHit::TranscriptScrollUp);
        }
        if y >= bar.y + bar.h - arrow {
            return Some(NetDlgChatHit::TranscriptScrollDown);
        }
        let pin = self.chat_transcript_pin_rect(layout, max_scroll);
        Some(if contains(pin, point) {
            NetDlgChatHit::TranscriptScrollPin
        } else {
            NetDlgChatHit::TranscriptScrollTrack
        })
    }

    fn chat_transcript_pin_rect(&self, layout: &NetDlgChatLayout, max_scroll: i32) -> IntRect {
        let bar = layout.transcript_scrollbar;
        let arrow = CHAT_SCROLL_ARROW_EXTENT.min(bar.h / 2);
        let travel = (bar.h - 3 * arrow).max(0);
        let scroll = self
            .chat_sheets
            .get(self.chat_active_sheet)
            .map_or(0, |sheet| {
                if sheet.transcript_follow_bottom {
                    max_scroll
                } else {
                    sheet.transcript_scroll
                }
            });
        let offset = if max_scroll > 0 && travel > 0 {
            (i64::from(scroll) * i64::from(travel) / i64::from(max_scroll)) as i32
        } else {
            0
        };
        IntRect::new(bar.x, bar.y + arrow + offset, bar.w, arrow)
    }

    /// `ScrollBar::MouseInput`'s drag/page: centre the pin under the pointer
    /// and map that back to a scroll offset.
    fn set_chat_transcript_scroll_from_pointer(&mut self, point: GuiPoint) {
        let layout = self.chat_layout();
        let max_scroll = self.chat_transcript_max_scroll_for(self.chat_active_sheet, &layout);
        let bar = layout.transcript_scrollbar;
        let arrow = CHAT_SCROLL_ARROW_EXTENT.min(bar.h / 2);
        let travel = (bar.h - 3 * arrow).max(0);
        if max_scroll <= 0 || travel <= 0 {
            return;
        }
        let position = (point.y.floor() as i32 - bar.y - arrow - arrow / 2).clamp(0, travel);
        let scroll = (i64::from(position) * i64::from(max_scroll) / i64::from(travel)) as i32;
        if let Some(sheet) = self.chat_sheets.get_mut(self.chat_active_sheet) {
            sheet.transcript_scroll = scroll.clamp(0, max_scroll);
            sheet.transcript_follow_bottom = sheet.transcript_scroll == max_scroll;
        }
    }

    fn scroll_active_chat_by(&mut self, delta: i32) {
        let layout = self.chat_layout();
        let max_scroll = self.chat_transcript_max_scroll_for(self.chat_active_sheet, &layout);
        let Some(sheet) = self.chat_sheets.get_mut(self.chat_active_sheet) else {
            return;
        };
        let current = if sheet.transcript_follow_bottom {
            max_scroll
        } else {
            sheet.transcript_scroll
        };
        sheet.transcript_scroll = current.saturating_add(delta).clamp(0, max_scroll);
        sheet.transcript_follow_bottom = sheet.transcript_scroll == max_scroll;
    }

    fn update_chat_dialog_drag(&mut self, point: GuiPoint) {
        let Some(drag) = self.chat_dialog_drag else {
            return;
        };
        let Some(bounds) = self.chat_bounds_override.as_mut() else {
            self.chat_dialog_drag = None;
            return;
        };
        bounds.x = drag.bounds.x + (point.x - drag.pointer.x) as i32;
        bounds.y = drag.bounds.y + (point.y - drag.pointer.y) as i32;
    }

    fn chat_hit(&self, point: GuiPoint) -> Option<NetDlgChatHit> {
        if self.chat_bounds_override.is_some() {
            let caption = self.chat_caption_and_group().0;
            if contains(Self::chat_dialog_close_rect(caption), point) {
                return Some(NetDlgChatHit::DialogClose);
            }
            if contains(caption, point) {
                return Some(NetDlgChatHit::DialogCaption);
            }
        }
        let layout = self.chat_layout();
        match self.chat_page() {
            NetDlgChatPage::Login => {
                for field in NetDlgChatLoginField::ALL {
                    if contains(layout.login_edits[field.index()], point) {
                        return Some(NetDlgChatHit::LoginField(field));
                    }
                }
                contains(layout.connect, point).then_some(NetDlgChatHit::Connect)
            }
            NetDlgChatPage::Chats => layout
                .tabs
                .iter()
                .position(|tab| contains(tab.close, point))
                .map(NetDlgChatHit::TabClose)
                .or_else(|| {
                    layout
                        .tabs
                        .iter()
                        .position(|tab| contains(tab.rect, point))
                        .map(NetDlgChatHit::Tab)
                })
                .or_else(|| contains(layout.input, point).then_some(NetDlgChatHit::Input))
                .or_else(|| self.chat_transcript_scrollbar_hit(&layout, point))
                .or_else(|| {
                    contains(layout.transcript_viewport, point).then_some(NetDlgChatHit::Transcript)
                })
                .or_else(|| {
                    let users = layout.users?;
                    if !contains(users, point) {
                        return None;
                    }
                    let row = ((point.y.floor() as i32 - users.y)
                        / self.metrics.text_line_height.max(1))
                    .max(0);
                    usize::try_from(row).ok().map(NetDlgChatHit::User)
                }),
        }
    }

    fn active_chat_edit_rect(&self) -> IntRect {
        let layout = self.chat_layout();
        match self.chat_page() {
            NetDlgChatPage::Login => layout.login_edits[self.chat_login_field.index()],
            NetDlgChatPage::Chats => layout.input,
        }
    }

    fn active_chat_edit_mut(&mut self) -> &mut NetDlgEditState {
        match self.chat_page() {
            NetDlgChatPage::Login => &mut self.chat_login_edits[self.chat_login_field.index()],
            NetDlgChatPage::Chats => &mut self.chat_edit,
        }
    }

    fn select_chat_login_field(&mut self, field: NetDlgChatLoginField) {
        if self.chat_login_field == field {
            return;
        }
        self.chat_login_edits[self.chat_login_field.index()].blur();
        self.chat_login_field = field;
        self.chat_login_edits[field.index()].focus();
    }

    fn select_chat_sheet(&mut self, index: usize) -> Vec<NetDlgAction> {
        let Some(sheet) = self.chat_sheets.get_mut(index) else {
            return Vec::new();
        };
        self.chat_active_sheet = index;
        sheet.unread = false;
        vec![NetDlgAction::ChatSelectSheet {
            kind: sheet.kind,
            ident: sheet.ident.clone(),
        }]
    }

    /// Cycles native top tabs for the app's Ctrl+Tab / Ctrl+Shift+Tab route.
    pub fn cycle_chat_sheet(&mut self, backwards: bool) -> Vec<NetDlgAction> {
        if self.chat_page != NetDlgChatPage::Chats || self.chat_sheets.len() < 2 {
            return Vec::new();
        }
        let next = if backwards {
            self.chat_active_sheet
                .checked_sub(1)
                .unwrap_or(self.chat_sheets.len() - 1)
        } else {
            (self.chat_active_sheet + 1) % self.chat_sheets.len()
        };
        self.select_chat_sheet(next)
    }

    pub fn submit_chat_login(&mut self) -> Vec<NetDlgAction> {
        let login = self.chat_login();
        let invalid = if !Self::valid_irc_nick(&login.nick) {
            Some(NetDlgChatValidationError::InvalidNick)
        } else if !Self::valid_irc_password(&login.password) {
            Some(NetDlgChatValidationError::InvalidPassword)
        } else if !Self::valid_irc_channel(&login.channel) {
            Some(NetDlgChatValidationError::InvalidChannel)
        } else {
            None
        };
        if let Some(error) = invalid {
            self.chat_connect_focused = false;
            self.select_chat_login_field(error.field());
            return vec![NetDlgAction::ChatValidationFailed(error)];
        }
        vec![NetDlgAction::ChatConnect(login)]
    }

    fn chat_error(&mut self, error: impl Into<String>) -> Vec<NetDlgAction> {
        let wrap_width = self.chat_transcript_wrap_width();
        let font = self.text_font.clone();
        if let Some(sheet) = self.chat_sheets.get_mut(self.chat_active_sheet) {
            Self::append_chat_line(
                sheet,
                NetDlgChatLine {
                    kind: NetDlgChatLineKind::Error,
                    text: error.into(),
                    new_paragraph: true,
                },
                true,
                font.as_ref(),
                wrap_width,
            );
        }
        vec![NetDlgAction::GuiSound(NetDlgSound::Error)]
    }

    pub fn submit_chat_input(&mut self) -> Vec<NetDlgAction> {
        self.submit_chat_input_with_paste_result().0
    }

    fn store_chat_history(&mut self, input: &str) {
        if let Some(existing) = self.chat_history.iter().position(|entry| entry == input) {
            self.chat_history.remove(existing);
        }
        self.chat_history.insert(0, input.to_string());
        self.chat_history.truncate(20);
    }

    fn chat_string_command(template: &str, command: &str) -> String {
        template.replace("{command}", command)
    }

    /// Returns whether native `ProcessInput` asks a multiline paste to abort.
    fn submit_chat_input_with_paste_result(&mut self) -> (Vec<NetDlgAction>, bool) {
        let input = std::mem::take(&mut self.chat_edit.text);
        self.chat_edit.set_text("");
        self.chat_history_index = None;
        if input.is_empty() {
            // `OnChatInput` answers empty submission with `DoError(nullptr)`,
            // which sounds without adding a line and never reaches
            // `ProcessInput` or the back buffer (C4ChatDlg.cpp:254-259,329-336).
            return (vec![NetDlgAction::GuiSound(NetDlgSound::Error)], false);
        }
        self.store_chat_history(&input);
        let mut actions = vec![NetDlgAction::ChatHistoryStored(input.clone())];
        if self.chat_connection_state != NetDlgChatConnectionState::Connected {
            let error = self.chat_strings.not_connected_error.clone();
            actions.extend(self.chat_error(error));
            return (actions, false);
        }

        let active = self.active_chat_sheet().cloned();
        let parsed: Result<(NetDlgChatCommand, bool), String> =
            if input.starts_with('/') && !input[1..].to_ascii_lowercase().starts_with("me ") {
                let command_line = &input[1..];
                // `SplitAtChar` cuts at the first delimiter only and takes the
                // remainder verbatim, so parameter spacing survives
                // (StdBuf.h:579-588).
                let (name, parameter) = command_line.split_once(' ').unwrap_or((command_line, ""));
                match name.to_ascii_lowercase().as_str() {
                    "quit" => Ok((
                        NetDlgChatCommand::Quit {
                            reason: parameter.to_string(),
                        },
                        true,
                    )),
                    "part" => {
                        let implicit_current = parameter.is_empty()
                            && active
                                .as_ref()
                                .is_some_and(|sheet| sheet.kind == NetDlgChatSheetKind::Channel);
                        Ok((
                            NetDlgChatCommand::Part {
                                channel: if implicit_current {
                                    active.as_ref().unwrap().ident.clone()
                                } else {
                                    parameter.to_string()
                                },
                            },
                            implicit_current,
                        ))
                    }
                    "join" | "j" => Ok((
                        NetDlgChatCommand::Join {
                            channel: if parameter.is_empty() {
                                self.chat_login().channel
                            } else {
                                parameter.to_string()
                            },
                        },
                        false,
                    )),
                    "notice" | "msg" => {
                        let Some((target, text)) = parameter.split_once(' ') else {
                            let error = Self::chat_string_command(
                                &self.chat_strings.insufficient_parameters,
                                name,
                            );
                            actions.extend(self.chat_error(error));
                            return (actions, false);
                        };
                        if text.is_empty() {
                            let error = Self::chat_string_command(
                                &self.chat_strings.insufficient_parameters,
                                name,
                            );
                            actions.extend(self.chat_error(error));
                            return (actions, false);
                        }
                        if name.eq_ignore_ascii_case("msg") {
                            Ok((
                                NetDlgChatCommand::Message {
                                    target: target.to_string(),
                                    text: text.to_string(),
                                },
                                false,
                            ))
                        } else {
                            Ok((
                                NetDlgChatCommand::Notice {
                                    target: target.to_string(),
                                    text: text.to_string(),
                                },
                                false,
                            ))
                        }
                    }
                    "raw" if !parameter.is_empty() => {
                        Ok((NetDlgChatCommand::Raw(parameter.to_string()), false))
                    }
                    "raw" => Err(Self::chat_string_command(
                        &self.chat_strings.insufficient_parameters,
                        name,
                    )),
                    "ns" | "cs" | "ms" if !parameter.is_empty() => Ok((
                        NetDlgChatCommand::Message {
                            target: match name.to_ascii_lowercase().as_str() {
                                "ns" => "NickServ",
                                "cs" => "ChanServ",
                                _ => "MemoServ",
                            }
                            .to_string(),
                            text: parameter.to_string(),
                        },
                        false,
                    )),
                    "ns" | "cs" | "ms" => Err(Self::chat_string_command(
                        &self.chat_strings.insufficient_parameters,
                        name,
                    )),
                    "query" | "q" if !parameter.is_empty() => {
                        let index =
                            Self::ensure_query_sheet(&mut self.chat_sheets, parameter, parameter);
                        self.chat_active_sheet = index;
                        self.chat_sheets[index].unread = false;
                        Ok((
                            NetDlgChatCommand::OpenQuery {
                                nick: parameter.to_string(),
                            },
                            false,
                        ))
                    }
                    "query" | "q" => Err(Self::chat_string_command(
                        &self.chat_strings.insufficient_parameters,
                        name,
                    )),
                    "nick" if Self::valid_irc_nick(parameter) => Ok((
                        NetDlgChatCommand::ChangeNick {
                            nick: parameter.to_string(),
                        },
                        false,
                    )),
                    "nick" => Err(Self::chat_string_command(
                        &self.chat_strings.invalid_nick,
                        name,
                    )),
                    _ => Err(Self::chat_string_command(
                        &self.chat_strings.unknown_command,
                        name,
                    )),
                }
            } else {
                let Some(active) = active else {
                    actions.extend(self.chat_error(self.chat_strings.not_on_channel.clone()));
                    return (actions, false);
                };
                if active.kind == NetDlgChatSheetKind::Server {
                    actions.extend(self.chat_error(self.chat_strings.not_on_channel.clone()));
                    return (actions, false);
                }
                if input.to_ascii_lowercase().starts_with("/me ") {
                    Ok((
                        NetDlgChatCommand::Action {
                            target: active.title,
                            text: input[4..].to_string(),
                        },
                        false,
                    ))
                } else {
                    Ok((
                        NetDlgChatCommand::Message {
                            target: active.title,
                            text: input,
                        },
                        false,
                    ))
                }
            };
        match parsed {
            Ok((command, abort_paste)) => {
                // `ProcessInput` reports a failed send on the sheet it was
                // handed (C4ChatDlg.cpp:1014-1017). The port sends
                // asynchronously, so the origin is retained until the
                // transport's error reaches the next snapshot.
                self.chat_send_error_origin = self
                    .chat_sheets
                    .get(self.chat_active_sheet)
                    .map(|sheet| (sheet.kind, sheet.ident.clone()));
                actions.push(NetDlgAction::ChatCommand(command));
                (actions, abort_paste)
            }
            Err(error) => {
                actions.extend(self.chat_error(error));
                (actions, false)
            }
        }
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

    /// Captions the dialog advertises, in `C4StartupNetDlg`'s construction
    /// order (C4StartupNetDlg.cpp:651-728). `Back` carries no marker.
    const HOTKEY_CAPTIONS: [(NetDlgControl, &'static str); 8] = [
        (NetDlgControl::GamesButton, "&Games"),
        (NetDlgControl::ChatButton, "&Chat"),
        (NetDlgControl::Internet, "&Internet"),
        (NetDlgControl::Record, "&Record"),
        (NetDlgControl::Back, "Back"),
        (NetDlgControl::Refresh, "Reloa&d"),
        (NetDlgControl::JoinGame, "&Join game"),
        (NetDlgControl::CreateGame, "&New game"),
    ];

    /// `Dialog::OnHotkey` walks the visible elements and lets the first
    /// matching enabled `Button` press itself (C4GuiButton.cpp:73-79). Chat
    /// mode removes Internet, Record, Reload and Join from the dialog, so
    /// their markers cannot fire.
    pub fn handle_hotkey(&mut self, character: char) -> Option<Vec<NetDlgAction>> {
        let character = character.to_ascii_uppercase();
        if !character.is_ascii_alphanumeric() {
            return None;
        }
        Self::HOTKEY_CAPTIONS
            .into_iter()
            .filter(|(control, _)| self.control_is_shown(*control))
            .find(|(_, caption)| expand_hotkey_markup(caption).1 == Some(character))
            .map(|(control, _)| self.activate(control))
    }

    fn control_is_shown(&self, control: NetDlgControl) -> bool {
        self.mode == NetDlgMode::GameList
            || !matches!(
                control,
                NetDlgControl::Internet
                    | NetDlgControl::Record
                    | NetDlgControl::Refresh
                    | NetDlgControl::JoinGame
            )
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
        if self.focus == NetDlgControl::ChatInput {
            self.active_chat_edit_mut().blur();
        }
        self.focus = focus;
        if focus == NetDlgControl::JoinAddress {
            self.join_edit.focus();
        }
        if focus == NetDlgControl::ChatInput && !self.chat_connect_focused {
            self.active_chat_edit_mut().focus();
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

    fn change_list_selection(&mut self, selection: Option<NetDlgSelection>) -> Vec<NetDlgAction> {
        if self.selection == selection {
            return Vec::new();
        }
        self.selection = selection;
        let layout = self.layout();
        self.ensure_selection_visible(&layout);
        selection
            .map(|_| vec![NetDlgAction::GuiSound(NetDlgSound::Command)])
            .unwrap_or_default()
    }

    fn select_list_boundary(&mut self, last: bool) -> Vec<NetDlgAction> {
        let layout = self.layout();
        let rows = self.row_layouts(&layout);
        let selection = if last { rows.last() } else { rows.first() }.map(|row| row.selection);
        self.change_list_selection(selection)
    }

    fn list_row_fully_visible(row: &NetDlgRowLayout, scroll_y: i32, layout: &NetDlgLayout) -> bool {
        let top = row.rect.y - layout.list_viewport.y;
        scroll_y <= top
            && scroll_y.saturating_add(layout.list_viewport.h) >= top.saturating_add(row.rect.h)
    }

    /// Exact adjacent-first paging from `C4GUI::ListBox::KeyPageDown/KeyPageUp`.
    /// Network rows have live variable heights, so visibility is evaluated
    /// against their current bounds instead of a fixed rows-per-page count.
    fn page_list_selection(&mut self, forward: bool) -> Vec<NetDlgAction> {
        let layout = self.layout();
        let rows = self.row_layouts(&layout);
        if rows.is_empty() {
            return Vec::new();
        }
        let mut target = self
            .selection
            .and_then(|selection| rows.iter().position(|row| row.selection == selection))
            .unwrap_or(if forward { 0 } else { rows.len() - 1 });

        if forward {
            if target + 1 < rows.len() {
                target += 1;
                if Self::list_row_fully_visible(&rows[target], self.list_scroll_y, &layout) {
                    while target + 1 < rows.len()
                        && Self::list_row_fully_visible(
                            &rows[target + 1],
                            self.list_scroll_y,
                            &layout,
                        )
                    {
                        target += 1;
                    }
                } else {
                    self.scroll_list_by(layout.list_viewport.h, &layout);
                    target = rows.len() - 1;
                    while target > 0
                        && !Self::list_row_fully_visible(&rows[target], self.list_scroll_y, &layout)
                    {
                        target -= 1;
                    }
                }
            }
        } else if target > 0 {
            target -= 1;
            if Self::list_row_fully_visible(&rows[target], self.list_scroll_y, &layout) {
                while target > 0
                    && Self::list_row_fully_visible(&rows[target - 1], self.list_scroll_y, &layout)
                {
                    target -= 1;
                }
            } else {
                self.scroll_list_by(layout.list_viewport.h.saturating_neg(), &layout);
                target = 0;
                while target + 1 < rows.len()
                    && !Self::list_row_fully_visible(&rows[target], self.list_scroll_y, &layout)
                {
                    target += 1;
                }
            }
        }

        self.change_list_selection(Some(rows[target].selection))
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
                rect: IntRect::new(label_x, line_y, width, text_height),
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
            rect: IntRect::new(layout.list_entry.x, y, layout.list_entry.w, row_height),
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

    /// Draws only the singleton-style IRC dialog, leaving the existing frame
    /// untouched outside its bounds. `set_chat_bounds_override` supplies the
    /// hit-test/render rectangle; without one this uses C++'s 10% inset.
    pub fn render_standalone_chat_dialog(
        surface: &mut Surface,
        assets: &NetDlgAssets,
        fonts: &ClonkFontSet,
        gamma: Option<&GammaRamp>,
        controller: &NetDlgController,
        draw_focus: bool,
    ) {
        let (width, height) = (surface.width() as i32, surface.height() as i32);
        let outer = controller
            .chat_bounds_override
            .unwrap_or_else(|| NetDlgController::standalone_chat_bounds(width, height));
        let caption_h = controller.metrics.text_line_height.max(23).min(outer.h);
        let caption = IntRect::new(outer.x, outer.y, outer.w, caption_h);
        let group = IntRect::new(
            outer.x,
            outer.y + caption_h,
            outer.w,
            (outer.h - caption_h).max(0),
        );
        let highlight = blacken_transparent_pixels(&assets.gui_button_highlight);
        let skin = ClassicGuiSkin::new(
            &assets.gui_caption,
            &assets.gui_button,
            &assets.gui_button_down,
            Some(&highlight),
        );
        skin.draw_dialog(surface, outer, gamma);
        Self::draw_chat_sheet(
            surface,
            assets,
            fonts,
            &skin,
            Some(&highlight),
            gamma,
            caption,
            controller.chat_layout_in(group),
            true,
            controller,
            draw_focus,
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
                    let icon_rect = IntRect::new(
                        row_rect.x + row_rect.w
                            - size * (i32::try_from(index).unwrap_or(i32::MAX) + 1),
                        row_rect.y,
                        size,
                        size,
                    );
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
                let clip = IntRect::new(client.x - 2, client.y, client.w + 4, client.h + 1);
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
            if let Some(controller) = controller {
                let (caption, group) = controller.chat_caption_and_group();
                Self::draw_chat_sheet(
                    surface,
                    assets,
                    fonts,
                    &classic_skin,
                    button_highlight.as_ref(),
                    gamma,
                    caption,
                    controller.chat_layout_in(group),
                    false,
                    controller,
                    draw_focus,
                );
            }
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
                    &gui_rect(IntRect::new(
                        row.x,
                        row.y + (large_size - fitted_height) / 2,
                        large_size,
                        fitted_height,
                    )),
                    &phase,
                    gamma,
                );
            }
            NetDlgRowIcon::QueryStatic => {
                let phase = net_get_ref_phase(&assets.net_get_ref, 0);
                let fitted_height = 32 * large_size / 40;
                crate::draw_image_bilinear(
                    surface,
                    &gui_rect(IntRect::new(
                        row.x,
                        row.y + (large_size - fitted_height) / 2,
                        large_size,
                        fitted_height,
                    )),
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
                    IntRect::new(
                        row.x + (large_size - small_size) / 2,
                        row.y + (large_size - small_size) / 2,
                        small_size,
                        small_size,
                    ),
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
                // One row of square icons, so the cell is the sheet's height —
                // the same derivation `draw_scen_list_item` uses
                // (startup_scensel.rs:1176). Reading it rather than assuming 24
                // keeps a higher-resolution strip selecting the right icon.
                Self::draw_icon_phase(
                    surface,
                    &assets.scen_icons,
                    assets.scen_icons.height().max(1),
                    phase as u32,
                    IntRect::new(
                        row.x + (large_size - small_size) / 2,
                        row.y + (large_size - small_size) / 2,
                        small_size,
                        small_size,
                    ),
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

    fn draw_icon_phase_rgb_modulated(
        surface: &mut Surface,
        image: &ImageData,
        cell: u32,
        phase: u32,
        rect: IntRect,
        modulation: u8,
        gamma: Option<&GammaRamp>,
    ) {
        let columns = (image.width() / cell).max(1);
        let source_x = phase % columns * cell;
        let source_y = phase / columns * cell;
        let pixels = (0..cell)
            .flat_map(|y| {
                (0..cell).flat_map(move |x| {
                    let offset = (((source_y + y) * image.width() + source_x + x) * 4) as usize;
                    let pixel = image.pixels().get(offset..offset + 4).unwrap_or(&[0; 4]);
                    [
                        ((u16::from(pixel[0]) * u16::from(modulation)) / 255) as u8,
                        ((u16::from(pixel[1]) * u16::from(modulation)) / 255) as u8,
                        ((u16::from(pixel[2]) * u16::from(modulation)) / 255) as u8,
                        pixel[3],
                    ]
                })
            })
            .collect();
        crate::draw_image_bilinear(
            surface,
            &gui_rect(rect),
            &ImageData::new(cell, cell, pixels),
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
        draw_engine_line(surface, rect.x, y, rect.x + width, y, 0x0080_80ff, gamma);
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
        assets: &NetDlgAssets,
        fonts: &ClonkFontSet,
        classic_skin: &ClassicGuiSkin<'_>,
        button_highlight: Option<&ImageData>,
        gamma: Option<&GammaRamp>,
        caption: IntRect,
        chat_layout: NetDlgChatLayout,
        show_dialog_close: bool,
        controller: &NetDlgController,
        draw_focus: bool,
    ) {
        let title = format!(
            "{} - {}",
            controller.chat_strings.chat,
            controller.chat_title()
        );
        classic_skin.draw_caption_with_right_indent(
            surface,
            caption,
            &title,
            &fonts.text,
            CLR_YELLOW,
            TextAlign::Left,
            if show_dialog_close { 20 } else { 0 },
            gamma,
        );
        if show_dialog_close {
            let close = NetDlgController::chat_dialog_close_rect(caption);
            let hovered = controller.pointer_position.is_some_and(|point| {
                controller.chat_hit(point) == Some(NetDlgChatHit::DialogClose)
            });
            let pressed = hovered && controller.chat_pressed == Some(NetDlgChatHit::DialogClose);
            if hovered {
                if let Some(highlight) = button_highlight {
                    crate::draw_image_bilinear_additive(
                        surface,
                        &gui_rect(close),
                        highlight,
                        gamma,
                    );
                }
            }
            Self::draw_icon_phase(surface, &assets.gui_icons, 40, 34, close, gamma);
            if pressed {
                if let Some(highlight) = button_highlight {
                    crate::draw_image_bilinear_additive(
                        surface,
                        &gui_rect(close),
                        highlight,
                        gamma,
                    );
                }
            }
        }
        let chat = chat_layout.group;
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

        if controller.chat_page() == NetDlgChatPage::Login {
            let labels = [
                controller.chat_strings.nick.as_str(),
                controller.chat_strings.password_optional.as_str(),
                controller.chat_strings.real_name.as_str(),
                controller.chat_strings.channel.as_str(),
            ];
            for (index, label) in labels.into_iter().enumerate() {
                let label_rect = chat_layout.login_labels[index];
                fonts.text.draw_with_gamma(
                    surface,
                    label_rect.x,
                    label_rect.y,
                    label,
                    CLR_WHITE,
                    TextAlign::Left,
                    true,
                    gamma,
                );
                let field = NetDlgChatLoginField::ALL[index];
                let focused = draw_focus
                    && controller.focus == NetDlgControl::ChatInput
                    && controller.chat_login_field == field
                    && !controller.chat_connect_focused;
                Self::draw_chat_edit(
                    surface,
                    &fonts.text,
                    gamma,
                    chat_layout.login_edits[index],
                    &controller.chat_login_edits[index],
                    field == NetDlgChatLoginField::Password,
                    focused,
                );
            }
            classic_skin.draw_button(
                surface,
                chat_layout.connect,
                &controller.chat_strings.connect,
                fonts,
                ClassicButtonState {
                    pressed: controller.chat_pressed == Some(NetDlgChatHit::Connect),
                    highlighted: draw_focus
                        && controller.focus == NetDlgControl::ChatInput
                        && controller.chat_connect_focused,
                },
                gamma,
            );
            return;
        }

        for (index, tab) in chat_layout.tabs.iter().copied().enumerate() {
            let selected = index == controller.chat_active_sheet;
            let Some(sheet) = controller.chat_sheets.get(index) else {
                continue;
            };
            let color = if selected {
                CLR_WHITE
            } else if sheet.unread {
                CLR_YELLOW
            } else {
                CLR_INACTIVE
            };
            classic_skin.draw_caption(
                surface,
                tab.rect,
                "",
                &fonts.text,
                color,
                TextAlign::Center,
                gamma,
            );
            let title_rect = IntRect::new(
                tab.rect.x + 4,
                tab.rect.y,
                (tab.close.x - tab.rect.x - 6).max(0),
                tab.rect.h,
            );
            draw_clipped_text(
                surface,
                &fonts.text,
                title_rect.x + title_rect.w / 2,
                title_rect.y + (title_rect.h - fonts.text.line_height).max(0) / 2,
                &sheet.title,
                color,
                TextAlign::Center,
                gamma,
                title_rect,
            );
            let close_hovered = controller.pointer_position.is_some_and(|point| {
                controller.chat_hit(point) == Some(NetDlgChatHit::TabClose(index))
            });
            if close_hovered {
                Self::draw_icon_phase(surface, &assets.gui_icons, 40, 34, tab.close, gamma);
            } else {
                Self::draw_icon_phase_rgb_modulated(
                    surface,
                    &assets.gui_icons,
                    40,
                    34,
                    tab.close,
                    0x7f,
                    gamma,
                );
            }
        }

        draw_engine_box(
            surface,
            chat_layout.transcript.x,
            chat_layout.transcript.y,
            chat_layout.transcript.x + chat_layout.transcript.w - 1,
            chat_layout.transcript.y + chat_layout.transcript.h - 1,
            CLR_DARK_BG,
            gamma,
        );
        let line_h = fonts.text.line_height.max(1);
        if let Some(sheet) = controller.active_chat_sheet() {
            let lines = NetDlgController::wrapped_chat_lines(sheet);
            let content_height = lines
                .iter()
                .enumerate()
                .map(|(index, line)| {
                    line_h
                        + if index > 0 && line.new_paragraph {
                            line_h / 3
                        } else {
                            0
                        }
                })
                .sum::<i32>();
            let max_scroll = (content_height - chat_layout.transcript_viewport.h).max(0);
            let scroll = if sheet.transcript_follow_bottom {
                max_scroll
            } else {
                sheet.transcript_scroll.clamp(0, max_scroll)
            };
            let mut y = chat_layout.transcript_viewport.y - scroll;
            for (index, line) in lines.iter().enumerate() {
                if index > 0 && line.new_paragraph {
                    y += line_h / 3;
                }
                let row = IntRect::new(
                    chat_layout.transcript_viewport.x + 3,
                    y,
                    (chat_layout.transcript_viewport.w - 6).max(0),
                    line_h,
                );
                if row.y + row.h > chat_layout.transcript_viewport.y
                    && row.y < chat_layout.transcript_viewport.y + chat_layout.transcript_viewport.h
                {
                    draw_clipped_text(
                        surface,
                        &fonts.text,
                        row.x,
                        row.y,
                        &line.text,
                        Self::chat_line_color(line.kind),
                        TextAlign::Left,
                        gamma,
                        chat_layout.transcript_viewport,
                    );
                }
                y += line_h;
            }
            if max_scroll > 0 {
                Self::draw_chat_scrollbar(
                    surface,
                    assets,
                    chat_layout.transcript_scrollbar,
                    scroll,
                    max_scroll,
                    gamma,
                );
            }
            if let Some(users) = chat_layout.users {
                draw_engine_box(
                    surface,
                    users.x,
                    users.y,
                    users.x + users.w - 1,
                    users.y + users.h - 1,
                    CLR_DARK_BG,
                    gamma,
                );
                // ScrollWindow shows the rows under its retained offset; a
                // partially scrolled row exposes one more at the bottom.
                let first_row = usize::try_from(sheet.user_scroll / line_h).unwrap_or(0);
                let visible_rows = usize::try_from((users.h / line_h).max(0)).unwrap_or(0)
                    + usize::from(sheet.user_scroll % line_h != 0);
                let row_offset = sheet.user_scroll % line_h;
                for (row, user) in sheet
                    .users
                    .iter()
                    .skip(first_row)
                    .take(visible_rows)
                    .enumerate()
                {
                    let icon = match user.prefix.as_bytes().first().copied() {
                        Some(b'!') => 35,
                        Some(b'@') => 36,
                        Some(b'%') => 37,
                        Some(b'+') => 20,
                        _ => 9,
                    };
                    let row_y =
                        users.y + i32::try_from(row).unwrap_or(i32::MAX) * line_h - row_offset;
                    Self::draw_icon_phase(
                        surface,
                        &assets.gui_icons,
                        40,
                        icon,
                        IntRect::new(users.x + 1, row_y, line_h, line_h),
                        gamma,
                    );
                    fonts.text.draw_with_gamma(
                        surface,
                        users.x + line_h + 3,
                        row_y,
                        &user.name,
                        CLR_WHITE,
                        TextAlign::Left,
                        true,
                        gamma,
                    );
                }
                draw_3d_frame(surface, users, gamma);
            }
        }
        draw_3d_frame(surface, chat_layout.transcript, gamma);
        if let Some(label) = chat_layout.input_label {
            classic_skin.draw_caption(
                surface,
                label,
                &controller.chat_strings.chat,
                &fonts.text,
                CLR_WHITE,
                TextAlign::Center,
                gamma,
            );
        }
        Self::draw_chat_edit(
            surface,
            &fonts.text,
            gamma,
            chat_layout.input,
            &controller.chat_edit,
            false,
            draw_focus && controller.focus == NetDlgControl::ChatInput,
        );
    }

    const fn chat_line_color(kind: NetDlgChatLineKind) -> [u8; 4] {
        match kind {
            NetDlgChatLineKind::Status => CLR_INACTIVE,
            NetDlgChatLineKind::Notice => CLR_NOTIFY,
            NetDlgChatLineKind::Error => CLR_ERROR,
            NetDlgChatLineKind::Server
            | NetDlgChatLineKind::Message
            | NetDlgChatLineKind::Action => CLR_WHITE,
        }
    }

    fn draw_chat_scrollbar(
        surface: &mut Surface,
        assets: &NetDlgAssets,
        bar: IntRect,
        scroll: i32,
        max_scroll: i32,
        gamma: Option<&GammaRamp>,
    ) {
        if bar.w <= 0 || bar.h < 2 * SCROLLBAR_PART {
            return;
        }
        crate::draw_image_strip(
            surface,
            bar.x,
            bar.y,
            &assets.gui_scroll,
            0,
            0,
            16,
            16,
            gamma,
        );
        let mut y = SCROLLBAR_PART;
        while y < bar.h - SCROLLBAR_PART {
            let height = SCROLLBAR_PART.min(bar.h - SCROLLBAR_PART - y).max(0) as u32;
            if height == 0 {
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
                height,
                gamma,
            );
            y += SCROLLBAR_PART;
        }
        crate::draw_image_strip(
            surface,
            bar.x,
            bar.y + bar.h - SCROLLBAR_PART,
            &assets.gui_scroll,
            0,
            32,
            16,
            16,
            gamma,
        );
        if bar.h > 3 * SCROLLBAR_PART && max_scroll > 0 {
            let range = bar.h - 3 * SCROLLBAR_PART;
            let pin = range * scroll / max_scroll;
            crate::draw_image_strip(
                surface,
                bar.x,
                bar.y + SCROLLBAR_PART + pin,
                &assets.gui_scroll,
                16,
                16,
                16,
                16,
                gamma,
            );
        }
    }

    fn draw_chat_edit(
        surface: &mut Surface,
        font: &ClonkFont,
        gamma: Option<&GammaRamp>,
        rect: IntRect,
        state: &NetDlgEditState,
        password: bool,
        focused: bool,
    ) {
        let client = edit_client(rect);
        draw_engine_box(
            surface,
            rect.x,
            rect.y,
            rect.x + rect.w - 1,
            rect.y + rect.h - 1,
            CLR_DARK_BG,
            gamma,
        );
        draw_3d_frame(surface, rect, gamma);
        let display = if password {
            "*".repeat(state.text.chars().count())
        } else {
            state.text.clone()
        };
        let text_y = client.y + (client.h - font.line_height).max(0) / 2;
        draw_clipped_text(
            surface,
            font,
            client.x - state.horizontal_scroll,
            text_y,
            &display,
            CLR_WHITE,
            TextAlign::Left,
            gamma,
            client,
        );
        if focused && state.cursor_visible() {
            let caret_text = if password {
                "*".repeat(state.text[..state.caret].chars().count())
            } else {
                state.text[..state.caret].to_string()
            };
            let caret_x = client.x + font.measure(&caret_text, false).0 - state.horizontal_scroll;
            draw_scaled_caret(surface, font, caret_x, text_y, client, gamma);
        }
    }
}

#[cfg(test)]
mod tests {
    use std::sync::OnceLock;

    use super::*;
    use crate::test_support::endeavour_font_set;
    use clonk_graphics::{Color, PixelFormat};

    macro_rules! assert_no_actions {
        ($actual:expr $(, $($arg:tt)+)?) => {
            assert!($actual.is_empty() $(, $($arg)+)?);
        };
    }

    macro_rules! assert_same {
        ($actual:expr => $expected:expr $(, $($arg:tt)+)?) => {
            assert_eq!($actual, $expected $(, $($arg)+)?);
        };
    }

    macro_rules! assert_action {
        ($actual:expr => $expected:expr $(, $($arg:tt)+)?) => {
            assert_eq!($actual, vec![$expected] $(, $($arg)+)?);
        };
        ($actual:expr, $expected:expr $(, $($arg:tt)+)?) => {
            assert_eq!($actual, vec![$expected] $(, $($arg)+)?);
        };
    }

    macro_rules! assert_actions {
        ($actual:expr => [$($expected:expr),+ $(,)?] $(, $($arg:tt)+)?) => {
            assert_eq!($actual, vec![$($expected),+] $(, $($arg)+)?);
        };
        ($actual:expr, [$($expected:expr),+ $(,)?] $(, $($arg:tt)+)?) => {
            assert_eq!($actual, vec![$($expected),+] $(, $($arg)+)?);
        };
    }

    macro_rules! assert_focus {
        ($actual:expr => $expected:expr) => {
            assert_action!($actual => NetDlgAction::FocusChanged($expected));
        };
        ($actual:expr, $expected:expr) => {
            assert_action!($actual => NetDlgAction::FocusChanged($expected));
        };
    }

    macro_rules! assert_chat {
        ($actual:expr => $input:expr => $command:expr) => {
            assert_same!($actual => chat_actions([($input, $command)]));
        };
    }

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

        let rect = |x, y, w, h| IntRect::new(x, y, w, h);
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

    fn game_entry(
        title: impl Into<String>,
        details: impl Into<String>,
        address: Option<&str>,
        joinable: bool,
    ) -> NetDlgGameEntry {
        NetDlgGameEntry {
            title: title.into(),
            details: details.into(),
            address: address.map(str::to_owned),
            joinable,
            ..NetDlgGameEntry::default()
        }
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
        assert_no_actions!(controller.handle_pointer_down(point, text_font()));
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

    fn controller_with_config(config: NetDlgConfig) -> NetDlgController {
        let mut controller = NetDlgController::new(config, metrics());
        controller.resize(1280, 720);
        controller
    }

    fn controller() -> NetDlgController {
        controller_with_config(NetDlgConfig::default())
    }

    fn offline_controller() -> NetDlgController {
        controller_with_config(NetDlgConfig {
            masterserver_signup: false,
            ..NetDlgConfig::default()
        })
    }

    fn controller_with_font(config: NetDlgConfig, font: &ClonkFont) -> NetDlgController {
        let mut controller = controller_with_config(config);
        controller.set_text_font(font);
        controller
    }

    fn default_controller_with_font(font: &ClonkFont) -> NetDlgController {
        controller_with_font(NetDlgConfig::default(), font)
    }

    fn offline_controller_with_font(font: &ClonkFont) -> NetDlgController {
        controller_with_font(
            NetDlgConfig {
                masterserver_signup: false,
                ..NetDlgConfig::default()
            },
            font,
        )
    }

    fn rendered_controller(
        controller: &NetDlgController,
        assets: &NetDlgAssets,
        fonts: &ClonkFontSet,
        gamma: Option<&GammaRamp>,
        get_ref_phase: u32,
    ) -> Surface {
        let mut surface = Surface::new(1280, 720, PixelFormat::Rgba8888);
        NetDlgScreen::render_controller(
            &mut surface,
            assets,
            fonts,
            gamma,
            controller,
            get_ref_phase,
        );
        surface
    }

    fn captured_controller(
        controller: &NetDlgController,
        assets: &NetDlgAssets,
        fonts: &ClonkFontSet,
        gamma: Option<&GammaRamp>,
        get_ref_phase: u32,
    ) -> Surface {
        let mut surface = Surface::new(1280, 720, PixelFormat::Rgba8888);
        surface.begin_clonk_text_capture();
        NetDlgScreen::render_controller(
            &mut surface,
            assets,
            fonts,
            gamma,
            controller,
            get_ref_phase,
        );
        surface
    }

    fn rendered_texts(
        controller: &NetDlgController,
        assets: &NetDlgAssets,
        fonts: &ClonkFontSet,
        gamma: Option<&GammaRamp>,
        get_ref_phase: u32,
    ) -> Vec<String> {
        captured_controller(controller, assets, fonts, gamma, get_ref_phase)
            .take_clonk_text_capture()
            .into_iter()
            .map(|command| command.text)
            .collect()
    }

    fn masterserver_entry() -> NetDlgMasterserverEntry {
        NetDlgMasterserverEntry {
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
        }
    }

    #[test]
    fn network_refresh_phases_reuse_retained_texture_identity() {
        let assets = net_assets();
        let first = net_get_ref_phase(&assets.net_get_ref, 17);
        let second = net_get_ref_phase(&assets.net_get_ref, 17);
        assert_eq!(first.gpu_texture_id(), second.gpu_texture_id());
        assert_eq!(first.pixels(), second.pixels());
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
        let mut controller = offline_controller_with_font(&fonts.text);
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
        assert_same!(rows[0].lines[0].rect.w => layout.entry_labels[0].w - 7 * metrics().text_line_height);
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
        let mut surface = captured_controller(&controller, &assets, &fonts, None, 0);
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
        let mut controller = offline_controller_with_font(&fonts.text);
        controller.set_games((0..6).map(rich_game).collect());
        let layout = controller.layout();

        assert!(controller.list_is_collapsed());
        assert_same!(controller .row_layouts(&layout) .iter() .map(|row| row.rect.h) .collect::<Vec<_>>() => vec![48; 6]);
        assert_eq!(controller.list_max_scroll(), 0);

        assert!(controller.focus_game(2).is_empty());
        let rows = controller.row_layouts(&layout);
        assert_same!(rows.iter().map(|row| row.rect.h).collect::<Vec<_>>() => vec![48, 48, 120, 48, 48, 48]);
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
        assert_same!(controller.list_scroll_offset() => controller.list_max_scroll());

        let mut with_master = self::controller();
        with_master.set_text_font(&fonts.text);
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
        assert_same!(selected_rows[0].rect.h => 48, "selected game collapses master");
        assert_eq!(selected_rows[3].rect.h, 120, "selected game stays expanded");
    }

    #[test]
    fn native_row_top_spacing_drives_collapse_scroll_and_gap_hit_testing() {
        let mut controller = offline_controller();
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
        assert!(controller.handle_pointer_down(gap, text_font()).is_empty());
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
        let mut controller = default_controller_with_font(&fonts.text);
        controller.set_masterserver_entry(masterserver_entry());

        let layout = controller.layout();
        let rows = controller.row_layouts(&layout);
        assert_eq!(rows[0].rect.h, 96);
        assert_eq!(rows[0].lines.len(), 4);
        let link = rows[0].lines[3].rect;
        let link_point = GuiPoint::new((link.x + 2) as f32, (link.y + 2) as f32);
        assert_action!(controller.handle_pointer_down(link_point, text_font()) => NetDlgAction::OpenUrl("https://league.example/news".into()));

        let assets = net_assets();
        let captured = rendered_texts(&controller, &assets, &fonts, None, 0);
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
    fn hyperlink_uses_cpp_color_exact_underline_and_only_link_opens() {
        let fonts = endeavour_font_set();
        let mut controller = default_controller_with_font(&fonts.text);
        controller.set_masterserver_entry(masterserver_entry());
        controller.set_games(vec![game_entry(
            "Wrong version",
            "Engine version: 4.9.11.0 [363]",
            None,
            false,
        )]);

        let layout = controller.layout();
        let rows = controller.row_layouts(&layout);
        let link_rect = rows[0].lines[3].rect;
        let ordinary_rect = rows[1].lines[0].rect;
        let link_point = GuiPoint::new((link_rect.x + 2) as f32, (link_rect.y + 2) as f32);
        let ordinary_actions = controller.handle_pointer_down(
            GuiPoint::new((ordinary_rect.x + 2) as f32, (ordinary_rect.y + 2) as f32),
            text_font(),
        );
        assert!(!ordinary_actions
            .iter()
            .any(|action| matches!(action, NetDlgAction::OpenUrl(_))));
        assert_action!(controller.handle_pointer_down(link_point, text_font()) => NetDlgAction::OpenUrl("https://league.example/news".into()));

        let assets = net_assets();
        let mut surface = captured_controller(&controller, &assets, &fonts, None, 0);
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
        let render = |official: bool, selected: bool| {
            let mut controller = offline_controller_with_font(&fonts.text);
            let mut game = rich_game(0);
            if official {
                game.status_icons.push(NetDlgStatusIcon::OfficialServer);
            }
            controller.set_games(vec![game]);
            if selected {
                let _ = controller.focus_game(0);
            }
            rendered_controller(&controller, &assets, &fonts, None, 0)
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
        let render = |icon, phase| {
            let mut controller = offline_controller_with_font(&fonts.text);
            controller.set_games(vec![NetDlgGameEntry {
                title: "Direct join on example.test".into(),
                details: "Query status".into(),
                row_icon: icon,
                ..NetDlgGameEntry::default()
            }]);
            rendered_controller(&controller, &assets, &fonts, None, phase)
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
        assert_same!(icon_pixels(&scenario_default) => icon_pixels(&scenario_negative));
        assert_same!(icon_pixels(&scenario_default) => icon_pixels(&scenario_too_large));

        let small_icon = IntRect::new(icon.x + 8, icon.y + 8, 32, 32);
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

    // The scenario-icon strip is a single row of square icons, so its cell is
    // the sheet's own height — which is how the scenario-selection list already
    // reads it (`draw_scen_list_item`, startup_scensel.rs:1176). Deriving it
    // here too lets a higher-resolution strip drop in: a 3x sheet must still
    // select icon N, not a 24px sliver of icon N/3.
    #[test]
    fn scenario_row_icons_derive_the_cell_from_the_strip_height() {
        let fonts = endeavour_font_set();
        let render = |assets: &NetDlgAssets, phase: i32| {
            let mut controller = offline_controller_with_font(&fonts.text);
            controller.set_games(vec![NetDlgGameEntry {
                title: "Direct join on example.test".into(),
                details: "Query status".into(),
                row_icon: NetDlgRowIcon::Scenario(phase),
                ..NetDlgGameEntry::default()
            }]);
            rendered_controller(&controller, assets, &fonts, None, 0)
        };

        let native = net_assets();
        let upscaled = NetDlgAssets {
            scen_icons: {
                let src = &native.scen_icons;
                let (w, h, k) = (src.width(), src.height(), 3u32);
                let mut pixels = vec![0u8; (w * k * h * k * 4) as usize];
                for y in 0..h * k {
                    for x in 0..w * k {
                        let from = (((y / k) * w + x / k) * 4) as usize;
                        let to = ((y * w * k + x) * 4) as usize;
                        pixels[to..to + 4].copy_from_slice(&src.pixels()[from..from + 4]);
                    }
                }
                crate::ImageData::new(w * k, h * k, pixels)
            },
            ..net_assets()
        };

        let distance = |a: &Surface, b: &Surface| -> u64 {
            a.pixels()
                .iter()
                .zip(b.pixels())
                .map(|(left, right)| u64::from(left.abs_diff(*right)))
                .sum()
        };
        let natives = (0..4)
            .map(|phase| render(&native, phase))
            .collect::<Vec<_>>();
        for phase in 0..4 {
            let drawn = render(&upscaled, phase);
            let nearest = (0..4)
                .min_by_key(|candidate| distance(&drawn, &natives[*candidate as usize]))
                .unwrap();
            assert_same!(nearest => phase, "a 3x strip drew icon {nearest} where phase {phase} was asked for");
        }
    }

    #[test]
    fn disabled_reference_rows_use_native_inactive_message_color() {
        let fonts = endeavour_font_set();
        let assets = net_assets();
        let mut controller = offline_controller_with_font(&fonts.text);
        controller.set_games(vec![game_entry(
            "Wrong version",
            "Engine version: 4.9.11.0 [363]",
            None,
            false,
        )]);
        let mut surface = captured_controller(&controller, &assets, &fonts, None, 0);
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
        let mut controller = controller();
        let layout = net_dlg_layout(1280, 720, &metrics());

        assert_action!(click(&mut controller, layout.buttons[0]) => NetDlgAction::Back);
        assert_action!(click(&mut controller, layout.buttons[1]) => NetDlgAction::Refresh);

        controller.set_join_address(" 127.0.0.1:11111 ");
        assert_action!(click(&mut controller, layout.buttons[2]) => NetDlgAction::JoinGame {address: None }, "a Join button click must not reinterpret an unfocused edit as a direct query");
        assert_focus!(controller.handle_pointer_down(center(layout.join_edit), text_font()) => NetDlgControl::JoinAddress);
        assert_actions!(click(&mut controller, layout.buttons[2]) => [NetDlgAction::QueryAddress {address: " 127.0.0.1:11111 ".into() }, NetDlgAction::FocusChanged(NetDlgControl::GameList), ]);
        assert_action!(click(&mut controller, layout.buttons[3]) => NetDlgAction::CreateGame);

        assert_action!(click(&mut controller, layout.btn_internet) => NetDlgAction::MasterserverSignupChanged(false));
        assert!(!controller.config().masterserver_signup);
        assert_action!(click(&mut controller, layout.btn_record) => NetDlgAction::RecordingChanged(true));
        assert!(controller.config().record);
        assert_actions!(click(&mut controller, layout.btn_chat) => [NetDlgAction::ModeChanged(NetDlgMode::Chat), NetDlgAction::FocusChanged(NetDlgControl::ChatInput), ]);
        assert_actions!(click(&mut controller, layout.btn_game_list) => [NetDlgAction::ModeChanged(NetDlgMode::GameList), NetDlgAction::FocusChanged(NetDlgControl::GameList), ]);

        let outside = crate::GuiPoint::new(
            (layout.buttons[0].x + layout.buttons[0].w) as f32,
            layout.buttons[0].y as f32,
        );
        assert_no_actions!(controller.handle_pointer_down(outside, text_font()));
        assert_no_actions!(controller.handle_pointer_up(outside, text_font()));
    }

    #[test]
    fn mode_switch_preserves_standalone_button_focus_and_structural_tab_order() {
        let mut controller = controller();
        let layout = net_dlg_layout(1280, 720, &metrics());
        for _ in 0..9 {
            controller.handle_key_down(crate::KeyCode::Tab);
        }
        assert_eq!(controller.focused_control(), NetDlgControl::ChatButton);

        assert_no_actions!(controller.handle_key_down(crate::KeyCode::Space));
        assert_action!(controller.handle_key_up(crate::KeyCode::Space) => NetDlgAction::ModeChanged(NetDlgMode::Chat));
        assert_eq!(controller.focused_control(), NetDlgControl::ChatButton);

        let mut hidden_button_focus = self::controller();
        for _ in 0..5 {
            hidden_button_focus.handle_key_down(crate::KeyCode::Tab);
        }
        assert_same!(hidden_button_focus.focused_control() => NetDlgControl::Refresh);
        assert_action!(click(&mut hidden_button_focus, layout.btn_chat) => NetDlgAction::ModeChanged(NetDlgMode::Chat));
        assert_same!(hidden_button_focus.focused_control() => NetDlgControl::Refresh);
        assert_no_actions!(hidden_button_focus.handle_key_down(crate::KeyCode::Enter));
        assert_action!(hidden_button_focus.handle_key_up(crate::KeyCode::Enter) => NetDlgAction::Refresh);
        assert_no_actions!(hidden_button_focus.handle_key_down(crate::KeyCode::Space));
        assert_action!(hidden_button_focus.handle_key_up(crate::KeyCode::Space) => NetDlgAction::Refresh);
        assert_focus!(hidden_button_focus.handle_key_down(crate::KeyCode::Tab) => NetDlgControl::CreateGame);

        let mut internet_focus = self::controller();
        for _ in 0..2 {
            internet_focus.handle_key_down(crate::KeyCode::Tab);
        }
        assert_eq!(internet_focus.focused_control(), NetDlgControl::Internet);
        assert_action!(click(&mut internet_focus, layout.btn_chat) => NetDlgAction::ModeChanged(NetDlgMode::Chat));
        assert_focus!(internet_focus.handle_key_down(crate::KeyCode::Tab) => NetDlgControl::Back);

        let mut hidden_join_focus = self::controller();
        for _ in 0..6 {
            hidden_join_focus.handle_key_down(crate::KeyCode::Tab);
        }
        assert_eq!(hidden_join_focus.focused_control(), NetDlgControl::JoinGame);
        assert_action!(click(&mut hidden_join_focus, layout.btn_chat) => NetDlgAction::ModeChanged(NetDlgMode::Chat));
        assert_no_actions!(hidden_join_focus.handle_key_down(crate::KeyCode::Enter));
        assert_no_actions!(hidden_join_focus.handle_key_up(crate::KeyCode::Enter));
        assert_no_actions!(hidden_join_focus.handle_key_down(crate::KeyCode::Space));
        assert_no_actions!(hidden_join_focus.handle_key_up(crate::KeyCode::Space));
    }

    #[test]
    fn shift_tab_reverses_game_list_focus_order() {
        let mut controller = self::controller();
        assert_eq!(controller.focused_control(), NetDlgControl::GameList);
        assert_focus!(controller.handle_key_down_with_tab_direction(crate::KeyCode::Tab, true) => NetDlgControl::ChatButton);
        assert_focus!(controller.handle_key_down(KeyCode::Tab) => NetDlgControl::GameList);

        assert_focus!(controller.handle_key_down(crate::KeyCode::Tab) => NetDlgControl::JoinAddress);
        assert_focus!(controller.handle_key_down(crate::KeyCode::Tab) => NetDlgControl::Internet);
        assert_focus!(controller.handle_key_down_with_tab_direction(crate::KeyCode::Tab, true) => NetDlgControl::JoinAddress);
        assert_focus!(controller.handle_key_down(crate::KeyCode::Tab) => NetDlgControl::Internet);
    }

    #[test]
    fn direct_join_edit_is_a_two_step_query_then_selected_row_join() {
        let mut controller = controller();
        let layout = net_dlg_layout(1280, 720, &metrics());
        controller.set_join_address("   ");

        assert_action!(controller.handle_key_down(crate::KeyCode::Enter) => NetDlgAction::JoinGame {address: None });
        assert_action!(click(&mut controller, layout.buttons[2]) => NetDlgAction::JoinGame {address: None });
        assert_action!(controller.handle_pointer_down(center(layout.join_edit), text_font()) => NetDlgAction::FocusChanged(NetDlgControl::JoinAddress));
        assert_actions!(controller.handle_key_down(crate::KeyCode::Enter) => [NetDlgAction::QueryAddress {address: "   ".into() }, NetDlgAction::FocusChanged(NetDlgControl::GameList), ]);
        assert_eq!(controller.focused_control(), NetDlgControl::GameList);
        assert_eq!(controller.join_address(), "   ");

        // The application materializes and selects the direct-query row.
        controller.set_games(vec![game_entry(
            "Direct query",
            "Querying",
            Some("example.test"),
            false,
        )]);
        assert_no_actions!(controller.focus_game(0));
        assert_action!(controller.handle_key_down(crate::KeyCode::Enter) => NetDlgAction::JoinGame {address: Some("example.test".into()) });
    }

    // The game list owns initial focus; Tab advances into the IP edit, whose
    // Left key edits rather than firing StartupNetBack. A focused button uses
    // C4GUI::Button's down/up key pair (C4StartupNetDlg.cpp:624-629,734;
    // C4GuiDialogs.cpp:343-357,616-644; C4GuiButton.cpp:22-35,112-126).
    #[test]
    fn controller_matches_initial_focus_and_keyboard_activation() {
        let mut controller = controller();
        assert_eq!(controller.focused_control(), NetDlgControl::GameList);

        assert_action!(controller.handle_key_down(crate::KeyCode::Enter) => NetDlgAction::JoinGame {address: None });
        assert_action!(controller.handle_key_down(crate::KeyCode::Tab) => NetDlgAction::FocusChanged(NetDlgControl::JoinAddress));
        assert_no_actions!(controller.handle_key_down(crate::KeyCode::Left));

        assert_action!(controller.handle_key_down(crate::KeyCode::Tab) => NetDlgAction::FocusChanged(NetDlgControl::Internet));
        assert_no_actions!(controller.handle_key_down(crate::KeyCode::Space));
        assert_action!(controller.handle_key_up(crate::KeyCode::Space) => NetDlgAction::MasterserverSignupChanged(false));
        assert_action!(controller.handle_key_down(crate::KeyCode::Escape) => NetDlgAction::Back);
    }

    #[test]
    fn join_edit_keyboard_matches_cpp_caret_selection_words_and_char_in() {
        let mut controller = controller();
        controller.set_join_address("alpha beta");
        assert_action!(controller.handle_key_down(KeyCode::Tab) => NetDlgAction::FocusChanged(NetDlgControl::JoinAddress));
        assert_eq!(controller.join_address_selection(), Some((0, 10)));

        assert_action!(controller.handle_text_input("x|\n\u{7f}", text_font()) => NetDlgAction::JoinAddressChanged("x\u{a6}".into()));
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
        assert_same!(controller.join_address_caret() => 9, "underscore is a word char");
        key(&mut controller, NetDlgEditKey::Right, true, false);
        assert_eq!(controller.join_address_selection(), Some((9, 11)));
        assert_eq!(controller.join_edit.selected_text(), Some("é"));
        assert_action!(key(&mut controller, NetDlgEditKey::Delete, false, false).actions => NetDlgAction::JoinAddressChanged("one,_two lan".into()));

        controller.set_join_address("abcd");
        key(&mut controller, NetDlgEditKey::Left, true, false);
        assert_eq!(controller.join_address_selection(), Some((3, 4)));
        key(&mut controller, NetDlgEditKey::Left, false, false);
        assert_same!(controller.join_address_caret() => 2, "plain movement clears a selection and still moves from its caret");
        let unchanged = controller.join_address().to_string();
        key(&mut controller, NetDlgEditKey::Delete, true, false);
        assert_eq!(controller.join_address(), unchanged);
    }

    #[test]
    fn join_edit_pointer_midpoints_drag_and_double_click_match_cpp() {
        let mut controller = controller();
        controller.set_join_address("Wi");
        controller.handle_key_down(KeyCode::Tab);
        let edit = controller.layout().join_edit;
        let client = edit_client(edit);
        let first_width = text_font().measure("W", false).0;
        let midpoint = first_width - first_width / 2;
        let y = (edit.y + edit.h / 2) as f32;

        let tie = GuiPoint::new((client.x + midpoint) as f32, y);
        controller.handle_pointer_down(tie, text_font());
        assert_same!(controller.join_address_caret() => 0, "midpoint tie stays left");
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
        let mut controller = controller();
        controller.set_join_address("alpha beta gamma");
        controller.focus = NetDlgControl::JoinAddress;
        let edit = controller.layout().join_edit;
        let stale = Instant::now() - std::time::Duration::from_millis(750);
        controller.join_edit.last_input = stale;
        controller.join_edit.horizontal_scroll = 13;

        controller.apply_context_command(NetDlgEditContextCommand::SelectAll, None, text_font());
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
        let mut controller = controller();
        controller.set_join_address("copy me");
        controller.handle_key_down(KeyCode::Tab);

        let copy = controller.handle_clipboard_shortcut(
            NetDlgEditClipboardShortcut::Copy,
            None,
            text_font(),
        );
        assert!(copy.captured);
        assert_action!(copy.actions => NetDlgAction::ClipboardTransfer {text: "copy me".into(), cut: false, });
        let cut = controller.handle_clipboard_shortcut(
            NetDlgEditClipboardShortcut::Cut,
            None,
            text_font(),
        );
        assert_action!(cut.actions => NetDlgAction::ClipboardTransfer {text: "copy me".into(), cut: true, });
        assert_eq!(controller.join_address(), "copy me", "cut waits for host");
        assert_action!(controller.confirm_clipboard_cut(text_font()) => NetDlgAction::JoinAddressChanged(String::new()));

        controller.set_join_address("old");
        controller.apply_context_command(NetDlgEditContextCommand::SelectAll, None, text_font());
        assert_actions!(
            controller.apply_context_command(
                NetDlgEditContextCommand::Paste,
                Some("\r\nhost|name\nignored"),
                text_font(),
            ) => [
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
        let mut controller = controller();
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
        let mut controller = offline_controller();
        controller.set_games(vec![
            game_entry(
                "Joinable game",
                "Lobby — Host One",
                Some("203.0.113.10:11112"),
                true,
            ),
            game_entry(
                "Wrong version",
                "LegacyClonk 4.9.11.0 [363]",
                Some("203.0.113.11:11112"),
                false,
            ),
        ]);

        assert_eq!(controller.selected_game(), None);
        assert_no_actions!(controller.handle_key_down(crate::KeyCode::Down));
        assert_eq!(controller.selected_game(), Some(0));
        assert_action!(controller.handle_key_down(crate::KeyCode::Enter) => NetDlgAction::JoinGame {address: Some("203.0.113.10:11112".into()) });
        assert_no_actions!(controller.handle_key_down(crate::KeyCode::Down));
        assert_eq!(controller.selected_game(), Some(1));
        assert_action!(controller.handle_key_down(crate::KeyCode::Enter) => NetDlgAction::JoinGame {address: Some("203.0.113.11:11112".into()) });
    }

    #[test]
    fn list_double_click_selects_focuses_and_joins_the_row_under_pointer() {
        let mut controller = controller();
        controller.set_games(vec![
            game_entry("Joinable", "Lobby", Some("203.0.113.10:11112"), true),
            game_entry(
                "Runtime confirmation required",
                "Running",
                Some("203.0.113.11:11112"),
                false,
            ),
        ]);
        let layout = net_dlg_layout(1280, 720, &metrics());
        assert_action!(controller.handle_pointer_down(center(layout.join_edit), text_font()) => NetDlgAction::FocusChanged(NetDlgControl::JoinAddress));

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
        assert_actions!(
            controller.handle_pointer_double_click(point, text_font()) => [
                NetDlgAction::FocusChanged(NetDlgControl::GameList),
                NetDlgAction::JoinGame {
                    address: Some("203.0.113.11:11112".into()),
                },
            ]
        );
        assert_eq!(controller.selected_game(), Some(1));
        assert_eq!(controller.focused_control(), NetDlgControl::GameList);

        assert_same!(controller.game_index_at(center(layout.list_entry)) => None, "the masterserver query row is not a discovered game");
    }

    #[test]
    fn overflowing_game_list_wheel_scrolls_clamps_and_hit_tests_content() {
        let mut controller = offline_controller();
        controller.set_games(games(20));
        let layout = net_dlg_layout(1280, 720, &metrics());
        let point = GuiPoint::new(
            (layout.list_viewport.x + 4) as f32,
            (layout.list_viewport.y + 4) as f32,
        );

        assert_eq!(controller.list_max_scroll(), 565);
        assert_no_actions!(controller.handle_wheel(point, -60));
        assert_eq!(controller.list_scroll_offset(), 60);
        controller.handle_wheel(point, -10_000);
        assert_same!(controller.list_scroll_offset() => controller.list_max_scroll());
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
        assert_no_actions!(controller.handle_pointer_down(point, text_font()));
        assert_eq!(controller.selected_game(), Some(2));
    }

    #[test]
    fn keyboard_selection_scrolls_each_row_range_into_view() {
        let mut controller = offline_controller();
        controller.set_games(games(20));
        let layout = net_dlg_layout(1280, 720, &metrics());

        for index in 0..20 {
            assert_no_actions!(controller.handle_key_down(KeyCode::Down));
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
        assert_same!(controller.list_scroll_offset() => controller.list_max_scroll());

        for index in (0..19).rev() {
            assert_no_actions!(controller.handle_key_down(KeyCode::Up));
            assert_eq!(controller.selected_game(), Some(index));
        }
        assert_eq!(controller.list_scroll_offset(), 0);
    }

    #[test]
    fn home_end_and_pages_use_live_network_row_ranges() {
        let command = vec![NetDlgAction::GuiSound(NetDlgSound::Command)];
        let mut with_master = controller();
        with_master.set_games(games(3));
        assert_eq!(with_master.handle_key_down(KeyCode::Home), command);
        assert_eq!(with_master.selection, Some(NetDlgSelection::Masterserver));
        assert_eq!(with_master.handle_key_down(KeyCode::End), command);
        assert_eq!(with_master.selection, Some(NetDlgSelection::Game(2)));

        let mut controller = offline_controller();
        controller.set_games(games(20));

        assert_eq!(controller.handle_key_down(KeyCode::Home), command);
        assert_eq!(controller.selected_game(), Some(0));
        assert_eq!(controller.list_scroll_offset(), 0);
        assert_eq!(controller.handle_key_down(KeyCode::End), command);
        assert_eq!(controller.selected_game(), Some(19));
        assert_same!(controller.list_scroll_offset() => controller.list_max_scroll());
        assert_eq!(controller.handle_key_down(KeyCode::Home), command);
        assert_eq!(controller.selected_game(), Some(0));
        assert_eq!(controller.list_scroll_offset(), 0);

        for (key, expected_index, expected_scroll) in [
            (KeyCode::PageDown, 8, 0),
            (KeyCode::PageDown, 17, 490),
            (KeyCode::PageDown, 19, 570),
            (KeyCode::PageUp, 11, 570),
            (KeyCode::PageUp, 2, 80),
            (KeyCode::PageUp, 0, 0),
        ] {
            assert_eq!(controller.handle_key_down(key), command);
            assert_eq!(controller.selected_game(), Some(expected_index));
            assert_eq!(controller.list_scroll_offset(), expected_scroll);
        }

        assert_action!(controller.handle_key_down(KeyCode::Tab) => NetDlgAction::FocusChanged(NetDlgControl::JoinAddress));
        for key in [
            KeyCode::Home,
            KeyCode::End,
            KeyCode::PageUp,
            KeyCode::PageDown,
        ] {
            assert_no_actions!(controller.handle_key_down(key));
            assert_eq!(controller.selected_game(), Some(0));
            assert_eq!(controller.list_scroll_offset(), 0);
        }

        let mut unselected = offline_controller();
        unselected.set_games(games(20));
        assert_eq!(unselected.handle_key_down(KeyCode::PageDown), command);
        assert_eq!(unselected.selected_game(), Some(8));
        unselected.selection = None;
        assert_eq!(unselected.handle_key_down(KeyCode::PageUp), command);
        assert_eq!(unselected.selected_game(), Some(0));
    }

    #[test]
    fn network_rows_do_not_match_typed_characters() {
        let mut controller = offline_controller();
        controller.set_join_address("unchanged");
        controller.set_games(vec![
            NetDlgGameEntry {
                title: "Thomas".into(),
                ..NetDlgGameEntry::default()
            },
            NetDlgGameEntry {
                title: "tina".into(),
                ..NetDlgGameEntry::default()
            },
        ]);
        controller.focus_game(1);
        let scroll = controller.list_scroll_offset();

        assert_no_actions!(controller.handle_text_input("T", text_font()));
        assert_eq!(controller.focused_control(), NetDlgControl::GameList);
        assert_eq!(controller.selected_game(), Some(1));
        assert_eq!(controller.list_scroll_offset(), scroll);
        assert_eq!(controller.join_address(), "unchanged");
    }

    #[test]
    fn scrollbar_track_drag_and_held_arrows_match_fixed_pin_math() {
        let mut controller = offline_controller();
        controller.set_games(games(20));
        let layout = net_dlg_layout(1280, 720, &metrics());
        let track = GuiPoint::new(
            (layout.list_scrollbar.x + 8) as f32,
            (layout.list_scrollbar.y + layout.list_scrollbar.h / 2) as f32,
        );

        assert_action!(controller.handle_pointer_down(center(layout.join_edit), text_font()) => NetDlgAction::FocusChanged(NetDlgControl::JoinAddress));
        assert_actions!(controller.handle_pointer_down(track, text_font()) => [NetDlgAction::FocusChanged(NetDlgControl::GameList), NetDlgAction::GuiSound(NetDlgSound::Command), ]);
        assert!(controller.list_scroll_offset() > 0);
        let below = GuiPoint::new(track.x, (layout.list_scrollbar.y + 10_000) as f32);
        assert_no_actions!(controller.handle_pointer_move(below, text_font()));
        assert_same!(controller.list_scroll_offset() => controller.list_max_scroll());
        assert_no_actions!(controller.handle_pointer_up(below, text_font()));

        let viewport = GuiPoint::new(
            (layout.list_viewport.x + 4) as f32,
            (layout.list_viewport.y + 4) as f32,
        );
        controller.handle_wheel(viewport, 10_000);
        let bottom_arrow = GuiPoint::new(
            (layout.list_scrollbar.x + 8) as f32,
            (layout.list_scrollbar.y + layout.list_scrollbar.h - 8) as f32,
        );
        assert_action!(controller.handle_pointer_down(bottom_arrow, text_font()) => NetDlgAction::GuiSound(NetDlgSound::ArrowHit));
        assert_eq!(controller.list_scroll_pin, 0);
        assert!(controller.tick_scrollbar());
        assert_eq!(controller.list_scroll_pin, 1);
        assert!(controller.tick_scrollbar());
        assert_eq!(controller.list_scroll_pin, 2);
        assert_action!(controller.handle_pointer_move(track, text_font()) => NetDlgAction::GuiSound(NetDlgSound::ArrowHit));
        assert!(!controller.tick_scrollbar());
        assert_action!(controller.handle_pointer_move(bottom_arrow, text_font()) => NetDlgAction::GuiSound(NetDlgSound::ArrowHit));
        assert!(controller.tick_scrollbar());
        assert_eq!(controller.list_scroll_pin, 3);
        let after_held_frames = controller.list_scroll_offset();
        assert_action!(controller.handle_pointer_up(bottom_arrow, text_font()) => NetDlgAction::GuiSound(NetDlgSound::ArrowHit));
        assert!(!controller.tick_scrollbar());
        assert_eq!(controller.list_scroll_offset(), after_held_frames);

        controller.change_mode(NetDlgMode::Chat);
        let before_hidden_click = controller.list_scroll_offset();
        assert_no_actions!(controller.handle_pointer_down(bottom_arrow, text_font()));
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

        let mut controller = offline_controller();
        controller.resize(1280, height);
        controller.set_games(games(4));
        assert_eq!(controller.list_max_scroll(), 159);
        let bottom_arrow = GuiPoint::new(
            (layout.list_scrollbar.x + 8) as f32,
            (layout.list_scrollbar.y + layout.list_scrollbar.h - 8) as f32,
        );
        assert_action!(controller.handle_pointer_down(bottom_arrow, text_font()) => NetDlgAction::GuiSound(NetDlgSound::ArrowHit));
        assert!(controller.tick_scrollbar());
        assert_eq!(controller.list_scroll_pin, 1);
        assert_eq!(controller.list_scroll_offset(), 1);
    }

    #[test]
    fn gamepad_horizontal_traverses_without_firing_keyboard_back() {
        let mut controller = controller();
        assert_action!(controller.handle_gamepad_horizontal(true) => NetDlgAction::FocusChanged(NetDlgControl::ChatButton));
        assert_action!(controller.handle_gamepad_horizontal(false) => NetDlgAction::FocusChanged(NetDlgControl::GameList));
    }

    #[test]
    fn tooltip_targets_match_native_net_dialog_visibility_and_ip_parent_pair() {
        let mut controller = controller();
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
            assert_same!(controller.tooltip_at(center(rect)) => Some(StartupTooltip::resource(key)));
        }
        for rect in [layout.ip_label, layout.join_edit] {
            assert_same!(controller.tooltip_at(center(rect)) => Some(StartupTooltip::resource("IDS_NET_IP_DESC")));
        }
        assert_same!(controller.tooltip_at(center(layout.game_list_caption)) => None);

        assert_actions!(click(&mut controller, layout.btn_chat) => [NetDlgAction::ModeChanged(NetDlgMode::Chat), NetDlgAction::FocusChanged(NetDlgControl::ChatInput), ]);
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
        assert_same!(controller.tooltip_at(center(layout.buttons[0])) => Some(StartupTooltip::resource("IDS_DLGTIP_BACKMAIN")));
    }

    // OnShown calls UpdateMasterserver, which replaces the Internet icon and
    // creates/removes only the query row. The retained dialog itself must not
    // be reconstructed: its active sheet, focus, edit contents, Record value,
    // and even simultaneous pointer/key press latches survive the config
    // refresh (C4StartupNetDlg.cpp:771-781,851-867; C4GuiButton.cpp:241-244).
    #[test]
    fn masterserver_config_sync_preserves_all_retained_dialog_state() {
        use crate::test_support::standard_gamma;

        let assets = net_assets();
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
            assert_action!(controller.handle_key_down(KeyCode::Tab) => NetDlgAction::FocusChanged(expected));
        }
        assert!(controller.handle_key_down(KeyCode::Space).is_empty());
        assert_no_actions!(controller.handle_pointer_down(center(layout.btn_internet), text_font()));
        assert_eq!(controller.pointer_pressed, Some(NetDlgControl::Internet));
        assert_same!(controller.key_pressed => Some((NetDlgControl::Record, KeyCode::Space)));

        let before = rendered_controller(&controller, &assets, &fonts, Some(standard_gamma()), 0);
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

        assert_same!(controller.config() => NetDlgConfig {masterserver_signup: false, record: true, });
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

        let after = rendered_controller(&controller, &assets, &fonts, Some(standard_gamma()), 0);
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
        let restored = rendered_controller(&controller, &assets, &fonts, Some(standard_gamma()), 0);
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
        use crate::test_support::standard_gamma;
        let assets = net_assets();
        let fonts = endeavour_font_set();
        let mut controller = controller();

        let render = |controller: &NetDlgController| {
            rendered_controller(controller, &assets, &fonts, Some(standard_gamma()), 0)
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
        let assets = net_assets();
        let fonts = endeavour_font_set();
        let mut controller = controller();
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
        let clip = IntRect::new(client.x - 2, client.y, client.w + 4, client.h + 1);
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
        use crate::test_support::standard_gamma;
        let assets = net_assets();
        let fonts = endeavour_font_set();
        let mut controller = offline_controller();
        controller.set_games(games(20));
        let layout = net_dlg_layout(1280, 720, &metrics());
        let render = |controller: &NetDlgController| {
            rendered_controller(controller, &assets, &fonts, Some(standard_gamma()), 0)
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
                assert_same!(top_pixel => bottom_pixel, "scrolled row bled outside list client at pixel {index}");
            }
        }
        assert!(viewport_changed, "later rows must become visible");
    }

    #[test]
    fn ordered_native_game_row_text_retains_the_scrollwindow_clipper() {
        use crate::test_support::standard_gamma;
        let assets = net_assets();
        let fonts = endeavour_font_set();
        let mut controller = offline_controller();
        controller.set_games(games(20));
        let layout = net_dlg_layout(1280, 720, &metrics());
        controller.handle_wheel(
            GuiPoint::new(
                (layout.list_viewport.x + 4) as f32,
                (layout.list_viewport.y + 4) as f32,
            ),
            -10_000,
        );

        let mut surface =
            captured_controller(&controller, &assets, &fonts, Some(standard_gamma()), 0);
        let expected = clonk_graphics::Rect::new(
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

    fn chat_login() -> NetDlgChatLogin {
        NetDlgChatLogin {
            server: "irc.example.test".into(),
            nick: "Clonker".into(),
            password: "secret".into(),
            real_name: "Clonk Player".into(),
            channel: "#clonken".into(),
        }
    }

    fn chat_snapshot(
        state: NetDlgChatConnectionState,
        messages: Vec<NetDlgChatMessage>,
        unread_index: usize,
    ) -> NetDlgChatSnapshot {
        NetDlgChatSnapshot {
            connection_state: state,
            server: "irc.example.test".into(),
            nick: "Clonker".into(),
            channels: vec![NetDlgChatChannel {
                name: "#clonken".into(),
                topic: "Clonk Rust".into(),
                users: vec![NetDlgChatUser {
                    prefix: "@".into(),
                    name: "Keeper".into(),
                }],
            }],
            messages,
            unread_index,
            last_error: None,
        }
    }

    fn chat_message(
        kind: NetDlgChatMessageKind,
        source: impl Into<String>,
        target: impl Into<String>,
        text: impl Into<String>,
        is_channel: bool,
    ) -> NetDlgChatMessage {
        NetDlgChatMessage {
            kind,
            source: source.into(),
            target: target.into(),
            text: text.into(),
            is_channel,
        }
    }

    fn chat_user(prefix: &str, name: &str) -> NetDlgChatUser {
        NetDlgChatUser {
            prefix: prefix.into(),
            name: name.into(),
        }
    }

    fn message_command(text: impl Into<String>) -> NetDlgChatCommand {
        NetDlgChatCommand::Message {
            target: "#clonken".into(),
            text: text.into(),
        }
    }

    fn action_command(text: impl Into<String>) -> NetDlgChatCommand {
        NetDlgChatCommand::Action {
            target: "#clonken".into(),
            text: text.into(),
        }
    }

    fn chat_actions<const N: usize>(
        submissions: [(&str, NetDlgChatCommand); N],
    ) -> Vec<NetDlgAction> {
        submissions
            .into_iter()
            .flat_map(|(input, command)| {
                [
                    NetDlgAction::ChatHistoryStored(input.into()),
                    NetDlgAction::ChatCommand(command),
                ]
            })
            .collect()
    }

    fn channel_message(text: impl Into<String>) -> NetDlgChatMessage {
        chat_message(
            NetDlgChatMessageKind::Message,
            "Keeper!ident@example",
            "#clonken",
            text,
            true,
        )
    }

    fn chat_controller(messages: Vec<NetDlgChatMessage>, unread_index: usize) -> NetDlgController {
        let mut controller = controller();
        controller.sync_chat_snapshot(chat_snapshot(
            NetDlgChatConnectionState::Connected,
            messages,
            unread_index,
        ));
        controller
    }

    fn focused_chat_controller(
        messages: Vec<NetDlgChatMessage>,
        unread_index: usize,
    ) -> NetDlgController {
        let mut controller = chat_controller(messages, unread_index);
        controller.force_chat_mode_and_focus();
        controller
    }

    fn chat_sheet_index(
        controller: &NetDlgController,
        kind: NetDlgChatSheetKind,
        expectation: &str,
    ) -> usize {
        controller
            .chat_sheets()
            .iter()
            .position(|sheet| sheet.kind == kind)
            .expect(expectation)
    }

    fn chat_sheet<'a>(
        controller: &'a NetDlgController,
        kind: NetDlgChatSheetKind,
        expectation: &str,
    ) -> &'a NetDlgChatSheet {
        controller
            .chat_sheets()
            .iter()
            .find(|sheet| sheet.kind == kind)
            .expect(expectation)
    }

    #[test]
    fn chat_login_enter_traversal_validates_and_submits_without_game_controls() {
        let mut controller = controller();
        controller.set_chat_login(chat_login());
        let layout = net_dlg_layout(1280, 720, &metrics());
        assert_actions!(click(&mut controller, layout.btn_chat) => [NetDlgAction::ModeChanged(NetDlgMode::Chat), NetDlgAction::FocusChanged(NetDlgControl::ChatInput), ]);
        assert_eq!(controller.chat_page(), NetDlgChatPage::Login);
        assert_no_actions!(click(&mut controller, layout.btn_internet));
        assert_no_actions!(click(&mut controller, layout.buttons[1]));
        assert_no_actions!(click(&mut controller, layout.buttons[2]));

        for expected in [
            NetDlgChatLoginField::Password,
            NetDlgChatLoginField::RealName,
            NetDlgChatLoginField::Channel,
        ] {
            assert_no_actions!(controller.handle_key_down(KeyCode::Enter));
            assert_eq!(controller.chat_login_field(), expected);
        }
        assert_no_actions!(controller.handle_key_down(KeyCode::Enter));
        assert_action!(controller.handle_key_down(KeyCode::Enter) => NetDlgAction::ChatConnect(chat_login()));
        controller.sync_chat_snapshot(chat_snapshot(
            NetDlgChatConnectionState::Connected,
            Vec::new(),
            0,
        ));
        controller.handle_text_input("after connect", text_font());
        assert_eq!(controller.chat_input(), "after connect");
        assert_chat!(controller.handle_key_down(KeyCode::Enter) => "after connect" => message_command("after connect"));

        let mut invalid = chat_login();
        invalid.nick = "NickServ".into();
        controller.show_chat_login();
        controller.set_chat_login(invalid);
        assert_action!(controller.submit_chat_login() => NetDlgAction::ChatValidationFailed(NetDlgChatValidationError::InvalidNick));
        assert_eq!(controller.chat_login_field(), NetDlgChatLoginField::Nick);
    }

    // `C4ChatControl::ChatSheet`'s transcript is a plain
    // `C4GUI::TextWindow(rcDefault)`, so it takes the constructor's
    // `iMaxLines = 100` / `iMaxTextLen = 4096` defaults, and its `C4LogBuffer`
    // evicts from the front on both bounds. The channel nick pane is a real
    // scrollable `ListBox`, so rows below the fold stay reachable
    // (src/C4ChatDlg.cpp:194,226-238; src/C4Gui.h:1309;
    // src/C4LogBuf.cpp:96-148; src/C4GuiContainers.cpp:477-623).
    #[test]
    fn irc_chat_scroll_windows_match_cpp_pointer_overflow_and_limits() {
        // Far more than the native line cap, each line short enough that the
        // character budget is not what evicts.
        let many = (0..250)
            .map(|index| channel_message(format!("m{index}")))
            .collect();
        let controller = chat_controller(many, 0);
        let channel = chat_sheet(&controller, NetDlgChatSheetKind::Channel, "channel tab");
        assert_same!(channel.lines.len() => CHAT_TRANSCRIPT_MAX_LINES, "the transcript is bounded by iMaxLines");
        assert_same!(channel.lines.last().expect("newest line") => &"<Keeper> m249", "eviction is from the front");

        // Long lines hit the character budget before the line cap.
        let long = (0..40)
            .map(|index| channel_message(format!("{index:03}{}", "x".repeat(200))))
            .collect();
        let controller = chat_controller(long, 0);
        let channel = chat_sheet(&controller, NetDlgChatSheetKind::Channel, "channel tab");
        assert!(
            channel.lines.len() < CHAT_TRANSCRIPT_MAX_LINES,
            "the character budget evicts before the line cap"
        );
        let retained = channel
            .lines
            .iter()
            .map(|line| line.text.len() + 1)
            .sum::<usize>();
        assert!(
            retained <= CHAT_TRANSCRIPT_MAX_TEXT,
            "retained {retained} bytes exceeds iMaxTextLen"
        );
        assert!(
            channel
                .lines
                .last()
                .expect("newest line")
                .text
                .contains("039"),
            "the newest line always survives"
        );

        // The transcript bar's arrows step a line, its pin drags, and its bare
        // track pages, exactly like C4GUI::ScrollBar.
        let overflow = (0..90)
            .map(|index| channel_message(format!("line {index}")))
            .collect();
        let mut controller = chat_controller(overflow, 0);
        let channel_index =
            chat_sheet_index(&controller, NetDlgChatSheetKind::Channel, "channel tab");
        controller.select_chat_sheet(channel_index);
        controller.force_chat_mode_and_focus();
        let layout = controller.chat_layout();
        let bar = layout.transcript_scrollbar;
        let max_scroll =
            controller.chat_transcript_max_scroll_for(controller.chat_active_sheet, &layout);
        assert!(max_scroll > 0, "90 lines overflow the transcript viewport");
        assert_eq!(
            controller.chat_sheets()[channel_index].transcript_scroll,
            max_scroll,
            "AddTextLine leaves the transcript pinned to the bottom"
        );

        let at = |y: i32| GuiPoint::new((bar.x + bar.w / 2) as f32, y as f32);
        // The up arrow steps one line off the bottom.
        controller.handle_pointer_down(at(bar.y + 2), text_font());
        controller.handle_pointer_up(at(bar.y + 2), text_font());
        let stepped = controller.chat_sheets()[channel_index].transcript_scroll;
        assert_eq!(stepped, max_scroll - metrics().text_line_height.max(1));
        assert!(!controller.chat_sheets()[channel_index].transcript_follow_bottom);

        // The down arrow steps back and re-pins.
        controller.handle_pointer_down(at(bar.y + bar.h - 2), text_font());
        controller.handle_pointer_up(at(bar.y + bar.h - 2), text_font());
        assert_same!(controller.chat_sheets()[channel_index].transcript_scroll => max_scroll);
        assert!(controller.chat_sheets()[channel_index].transcript_follow_bottom);

        // Pressing the bare track pages to that position, and dragging the pin
        // tracks the pointer.
        controller.handle_pointer_down(at(bar.y + bar.h / 2), text_font());
        let paged = controller.chat_sheets()[channel_index].transcript_scroll;
        assert!(paged > 0 && paged < max_scroll, "paged to {paged}");
        controller.handle_pointer_move(at(bar.y + CHAT_SCROLL_ARROW_EXTENT), text_font());
        assert_eq!(controller.chat_sheets()[channel_index].transcript_scroll, 0);
        controller.handle_pointer_up(at(bar.y + bar.h), text_font());
        assert_eq!(
            controller.chat_sheets()[channel_index].transcript_scroll,
            max_scroll,
            "the release position is applied before the drag ends"
        );

        // The nick list retains a scroll offset instead of dropping rows.
        let mut controller = self::controller();
        let mut snapshot = chat_snapshot(NetDlgChatConnectionState::Connected, Vec::new(), 0);
        snapshot.channels[0].users = (0..500)
            .map(|index| NetDlgChatUser {
                prefix: String::new(),
                name: format!("User{index:02}"),
            })
            .collect();
        controller.sync_chat_snapshot(snapshot);
        let channel_index =
            chat_sheet_index(&controller, NetDlgChatSheetKind::Channel, "channel tab");
        controller.select_chat_sheet(channel_index);
        controller.force_chat_mode_and_focus();
        let line_height = metrics().text_line_height.max(1);
        let max_scroll = controller.chat_users_max_scroll(line_height);
        assert!(max_scroll > 0, "500 users overflow the nick pane");

        let users = controller.chat_layout().users.expect("channel nick pane");
        let inside = GuiPoint::new(
            (users.x + users.w / 2) as f32,
            (users.y + users.h / 2) as f32,
        );
        controller.handle_wheel(inside, -60);
        let scrolled = controller.chat_sheets()[channel_index].user_scroll;
        assert!(scrolled > 0, "the wheel moves the nick ListBox");
        controller.handle_wheel(inside, 60);
        assert_eq!(controller.chat_sheets()[channel_index].user_scroll, 0);
        for _ in 0..200 {
            controller.handle_wheel(inside, -60);
        }
        assert_same!(controller.chat_sheets()[channel_index].user_scroll => max_scroll, "the offset is bounded by the overflow");
    }

    // `AppendLines` word-wraps a message at the width in force and hands every
    // resulting line to `AppendSingleLine` on its own, and that is where the
    // `iMaxLines = 100` / `iMaxTextLen = 4096` caps evict from the front
    // (src/C4Gui.h:1309; src/C4LogBuf.cpp:96-148,174-205). The caps therefore
    // count *display* lines, so one message long enough to overflow the
    // transcript by itself loses its own leading lines instead of being
    // retained whole.
    #[test]
    fn the_transcript_caps_count_wrapped_display_lines() {
        let mut controller = controller();
        controller.set_text_font(text_font());
        let width = controller.chat_transcript_wrap_width();
        assert!(width > 0, "the transcript has a wrap width");

        // One message, long enough to overflow the transcript on its own, with
        // every word individually identifiable.
        let words = (0..1500)
            .map(|index| format!("w{index}"))
            .collect::<Vec<_>>();
        controller.sync_chat_snapshot(chat_snapshot(
            NetDlgChatConnectionState::Connected,
            vec![channel_message(words.join(" "))],
            0,
        ));
        let channel = chat_sheet(&controller, NetDlgChatSheetKind::Channel, "channel tab");

        assert!(
            channel.lines.len() > 1,
            "a single long message is stored as the display lines it wraps to, not one entry"
        );
        assert!(
            channel.lines.len() <= CHAT_TRANSCRIPT_MAX_LINES,
            "the line cap counts display lines: {} retained",
            channel.lines.len()
        );
        let retained = channel
            .lines
            .iter()
            .map(|line| line.text.len() + 1)
            .sum::<usize>();
        assert!(
            retained <= CHAT_TRANSCRIPT_MAX_TEXT,
            "retained {retained} bytes exceeds iMaxTextLen"
        );

        // Front eviction reaches inside the message: its opening words are
        // gone while its last word survives.
        let transcript = channel
            .lines
            .iter()
            .map(|line| line.text.as_str())
            .collect::<Vec<_>>()
            .join(" ");
        assert!(
            transcript.contains("w1499"),
            "the newest display line always survives"
        );
        assert!(
            !transcript.contains("<Keeper>"),
            "the message's own opening line is evicted, not preserved whole"
        );
        assert!(
            !channel
                .lines
                .first()
                .expect("a retained line")
                .new_paragraph,
            "the retained head is a continuation line, not the paragraph start"
        );
    }

    #[test]
    fn irc_login_validation_counts_legacy_bytes_and_rejects_unrepresentable_text() {
        let password_at_limit = format!("{}é", "a".repeat(30));
        assert_same!(clonk_resources::encode_legacy_script_text(&password_at_limit) .expect("Windows-1252 password") .len() => 31);
        assert!(NetDlgController::valid_irc_password(&password_at_limit));
        assert!(!NetDlgController::valid_irc_password(&format!(
            "{}é",
            "a".repeat(31)
        )));
        assert!(!NetDlgController::valid_irc_password("ok🙂"));
        assert!(!NetDlgController::valid_irc_password("has space"));

        let channel_at_limit = format!("#{}é", "a".repeat(30));
        assert_same!(clonk_resources::encode_legacy_script_text(&channel_at_limit) .expect("Windows-1252 channel") .len() => 32);
        assert!(NetDlgController::valid_irc_channel(&channel_at_limit));
        assert!(!NetDlgController::valid_irc_channel(&format!(
            "#{}é",
            "a".repeat(31)
        )));
        assert!(!NetDlgController::valid_irc_channel("#clonk🙂"));
        assert!(!NetDlgController::valid_irc_channel("clonken"));
        assert!(!NetDlgController::valid_irc_channel("#clonk en"));
    }

    #[test]
    fn chat_snapshot_builds_tabs_routes_messages_and_appends_only_unread_suffix() {
        let first = channel_message("Welcome");
        let second = chat_message(
            NetDlgChatMessageKind::Action,
            "Keeper!ident@example",
            "#clonken",
            "waves",
            true,
        );
        assert_eq!(
            NetDlgController::format_chat_message(
                &chat_message(
                    NetDlgChatMessageKind::Message,
                    "Clonker!ident@example",
                    "#clonken",
                    "local echo",
                    true,
                ),
                "Clonker",
                "Clonker",
            ),
            "<Clonker> local echo"
        );
        let mut controller = controller();
        controller.sync_chat_snapshot(chat_snapshot(
            NetDlgChatConnectionState::Connecting,
            Vec::new(),
            0,
        ));
        assert_eq!(controller.chat_page(), NetDlgChatPage::Chats);
        assert_same!(controller.chat_sheets()[0].lines => vec!["Connecting to irc.example.test at "]);

        controller.sync_chat_snapshot(chat_snapshot(
            NetDlgChatConnectionState::Connected,
            vec![first.clone()],
            0,
        ));
        let channel = controller.active_chat_sheet().expect("channel tab");
        assert_eq!(channel.kind, NetDlgChatSheetKind::Channel);
        assert_eq!(channel.topic, "Clonk Rust");
        assert_eq!(channel.users[0].name, "Keeper");
        assert_eq!(channel.lines, vec!["<Keeper> Welcome"]);

        controller.sync_chat_snapshot(chat_snapshot(
            NetDlgChatConnectionState::Connected,
            vec![first, second],
            1,
        ));
        assert_same!(controller.active_chat_sheet().unwrap().lines => vec!["<Keeper> Welcome", "* Keeper waves"]);
        assert_same!(controller.chat_sheets()[0].lines => vec!["Connecting to irc.example.test at "]);
    }

    #[test]
    fn connected_chat_enter_routes_messages_commands_and_history() {
        let mut controller = chat_controller(Vec::new(), 0);
        controller.mode = NetDlgMode::Chat;
        controller.focus = NetDlgControl::ChatInput;

        assert_no_actions!(controller.handle_text_input("hello channel", text_font()));
        assert_chat!(controller.handle_key_down(KeyCode::Enter) => "hello channel" => message_command("hello channel"));
        controller.handle_text_input("/me waves", text_font());
        assert_chat!(controller.handle_key_down(KeyCode::Enter) => "/me waves" => action_command("waves"));
        controller.handle_text_input("/part", text_font());
        assert_chat!(controller.handle_key_down(KeyCode::Enter) => "/part" => NetDlgChatCommand::Part { channel: "#clonken".into() });
        assert_no_actions!(controller.handle_key_down(KeyCode::Up));
        assert_eq!(controller.chat_input(), "/part");
        assert_no_actions!(controller.handle_key_down(KeyCode::Down));
        assert_eq!(controller.chat_input(), "");
        controller.handle_text_input("Grüße", text_font());
        assert_chat!(controller.handle_key_down(KeyCode::Enter) => "Grüße" => message_command("Grüße"));
    }

    #[test]
    fn chat_edit_keys_clipboard_and_rendered_tabs_are_functional() {
        let mut controller = chat_controller(vec![channel_message("Welcome")], 0);
        controller.mode = NetDlgMode::Chat;
        controller.focus = NetDlgControl::ChatInput;
        controller.handle_text_input("copy me", text_font());
        assert!(
            controller
                .handle_clipboard_shortcut(
                    NetDlgEditClipboardShortcut::SelectAll,
                    None,
                    text_font(),
                )
                .captured
        );
        assert_action!(controller .handle_clipboard_shortcut(NetDlgEditClipboardShortcut::Copy, None, text_font()) .actions => NetDlgAction::ClipboardTransfer {text: "copy me".into(), cut: false, });
        assert!(
            controller
                .handle_edit_key_down(
                    NetDlgEditKey::Backspace,
                    NetDlgEditModifiers::default(),
                    text_font(),
                )
                .captured
        );
        assert_eq!(controller.chat_input(), "");

        let fonts = endeavour_font_set();
        let assets = net_assets();
        let texts = rendered_texts(&controller, &assets, &fonts, None, 0);
        assert!(texts.iter().any(|text| text == "#clonken"));
        assert!(texts.iter().any(|text| text == "<Keeper> Welcome"));
        assert!(texts.iter().any(|text| text == "Keeper"));
        assert!(!texts.iter().any(|text| text == "X"));
    }

    #[test]
    fn chat_login_uses_cpp_centered_geometry_localized_strings_and_typed_validation() {
        let mut controller = controller();
        controller.force_chat_mode_and_focus();
        controller.set_chat_strings(NetDlgChatStrings {
            chat: "Gespräch".into(),
            not_connected: "Nicht verbunden".into(),
            nick: "Spitzname:".into(),
            connect: "Verbinden".into(),
            ..NetDlgChatStrings::default()
        });

        let layout = controller.chat_layout();
        let inner = IntRect::new(
            layout.group.x + 2,
            layout.group.y + 2,
            layout.group.w - 4,
            layout.group.h - 4,
        );
        let login_h = (metrics().text_line_height * 8 + 2 * 10 + 5 * 10 + 32 + 20).min(inner.h);
        let login_w = (login_h * 2 / 3).min(inner.w);
        let login = Aligner::new(inner, 0, 0).centered(login_w, login_h);
        assert_eq!(layout.login_labels[0].x, login.x + 2);
        assert_eq!(layout.login_labels[0].y, login.y + 2);
        assert_eq!(layout.login_labels[0].w, login.w - 4);
        assert_same!(layout.login_edits[0].h => (metrics().text_line_height + 3).max(23));
        assert_eq!(layout.connect.w, 140);
        assert_same!(layout.connect.x + layout.connect.w / 2 => login.x + login.w / 2);

        let mut login_values = chat_login();
        login_values.nick = "NickServ".into();
        controller.set_chat_login(login_values);
        assert_action!(controller.submit_chat_login() => NetDlgAction::ChatValidationFailed(NetDlgChatValidationError::InvalidNick));
        assert_eq!(controller.chat_login_field(), NetDlgChatLoginField::Nick);

        let fonts = endeavour_font_set();
        let assets = net_assets();
        let texts = rendered_texts(&controller, &assets, &fonts, None, 0);
        assert!(texts
            .iter()
            .any(|text| text == "Gespräch - Nicht verbunden"));
        assert!(texts.iter().any(|text| text == "Spitzname:"));
        assert!(texts.iter().any(|text| text == "Verbinden"));
        assert!(!texts.iter().any(|text| text.contains("Invalid nickname")));
    }

    #[test]
    fn chat_snapshot_sanitizes_colors_unread_routes_and_sorts_like_cpp() {
        let users = vec![
            chat_user("", "zulu"),
            chat_user("@", "beta"),
            chat_user("+", "alpha"),
            chat_user("!", "Owner"),
            chat_user("@", "Alpha"),
        ];
        let messages = vec![
            chat_message(
                NetDlgChatMessageKind::Message,
                "Ghost!gone@example",
                "#already-parted",
                "must be dropped",
                true,
            ),
            chat_message(
                NetDlgChatMessageKind::Notice,
                "Announcer!ident@example",
                "Clonker",
                "red\u{1}notice",
                false,
            ),
            chat_message(
                NetDlgChatMessageKind::Message,
                "Alice!ident@example",
                "Clonker",
                "hi\u{1}there",
                false,
            ),
            chat_message(
                NetDlgChatMessageKind::Message,
                "Clonker!own@example",
                "Bob",
                "outgoing",
                false,
            ),
        ];
        let mut controller = controller();
        let mut snapshot = chat_snapshot(NetDlgChatConnectionState::Connected, messages, 0);
        snapshot.channels[0].users = users;
        controller.sync_chat_snapshot(snapshot);

        assert!(!controller.chat_sheets()[0]
            .lines
            .iter()
            .any(|line| line.text.contains("must be dropped")));
        let channel = &controller.chat_sheets()[1];
        assert_eq!(
            channel
                .users
                .iter()
                .map(|user| (user.prefix.as_str(), user.name.as_str()))
                .collect::<Vec<_>>(),
            vec![
                ("!", "Owner"),
                ("@", "Alpha"),
                ("@", "beta"),
                ("+", "alpha"),
                ("", "zulu"),
            ]
        );
        assert_eq!(channel.lines[0].kind, NetDlgChatLineKind::Notice);
        assert_eq!(channel.lines[0].text, "-Announcer- red notice");
        assert_same!(NetDlgScreen::chat_line_color(channel.lines[0].kind) => CLR_NOTIFY);
        let alice = controller
            .chat_sheets()
            .iter()
            .find(|sheet| sheet.title == "Alice")
            .expect("incoming query");
        assert!(alice.unread);
        assert_eq!(alice.topic, "Alice!ident@example");
        assert_eq!(alice.lines[0].kind, NetDlgChatLineKind::Message);
        assert_eq!(alice.lines[0].text, "<Alice> hi there");
        let active = controller
            .active_chat_sheet()
            .expect("outgoing query selected");
        assert_eq!(active.title, "Bob");
        assert!(!active.unread);
        assert_eq!(active.lines[0].kind, NetDlgChatLineKind::Message);

        let alice_index = controller
            .chat_sheets()
            .iter()
            .position(|sheet| sheet.title == "Alice")
            .unwrap();
        controller.select_chat_sheet(alice_index);
        assert!(!controller.chat_sheets()[alice_index].unread);

        controller.select_chat_sheet(1);
        controller.force_chat_mode_and_focus();
        let users = controller.chat_layout().users.expect("channel nick list");
        assert_eq!(
            controller.tooltip_at(GuiPoint::new(
                (users.x + 2) as f32,
                (users.y + metrics().text_line_height / 2) as f32,
            )),
            Some(StartupTooltip::text("!Owner"))
        );
    }

    #[test]
    fn chat_tab_close_cycle_and_channel_part_match_native_sheet_types() {
        let mut controller = focused_chat_controller(Vec::new(), 0);
        assert_action!(controller.close_chat_sheet(0) => NetDlgAction::ChatDisconnectConfirmationRequested);
        assert_eq!(controller.chat_page(), NetDlgChatPage::Chats);
        assert_action!(controller.close_active_chat_sheet() => NetDlgAction::ChatCommand(NetDlgChatCommand::Part {channel: "#clonken".into(), }));

        controller.handle_text_input("/query Keeper", text_font());
        let query_actions = controller.submit_chat_input();
        assert_same!(query_actions[0] => NetDlgAction::ChatHistoryStored("/query Keeper".into()));
        let query_index = controller.chat_active_sheet;
        assert_same!(controller.chat_sheets()[query_index].kind => NetDlgChatSheetKind::Query);
        assert_no_actions!(controller.close_active_chat_sheet());
        assert!(!controller
            .chat_sheets()
            .iter()
            .any(|sheet| sheet.title == "Keeper"));

        assert_eq!(controller.chat_active_sheet, 1);
        assert_action!(controller.cycle_chat_sheet(false) => NetDlgAction::ChatSelectSheet {kind: NetDlgChatSheetKind::Server, ident: "irc.example.test".into(), });
        controller.chat_sheets[1].unread = true;
        let channel_tab = controller.chat_layout().tabs[1];
        let channel_point = GuiPoint::new(
            (channel_tab.rect.x + 3) as f32,
            (channel_tab.rect.y + channel_tab.rect.h / 2) as f32,
        );
        assert_action!(controller.handle_pointer_down(channel_point, text_font()) => NetDlgAction::ChatSelectSheet {kind: NetDlgChatSheetKind::Channel, ident: "#clonken".into(), });
        assert!(!controller.chat_sheets()[1].unread);
        assert_no_actions!(controller.handle_pointer_up(channel_point, text_font()));

        controller.select_chat_sheet(0);
        let close = controller.chat_layout().tabs[0].close;
        let close_point = center(close);
        assert_action!(controller.handle_pointer_down(close_point, text_font()) => NetDlgAction::ChatDisconnectConfirmationRequested);
        assert_no_actions!(controller.handle_pointer_up(close_point, text_font()));

        controller.show_chat_login();
        assert_action!(controller.close_chat_sheet(0) => NetDlgAction::ChatDisconnect);
        assert_eq!(controller.chat_page(), NetDlgChatPage::Login);
    }

    #[test]
    fn chat_multiline_paste_submits_lines_retains_tail_and_honors_abort() {
        let mut controller = focused_chat_controller(Vec::new(), 0);
        controller.set_text_font(text_font());
        let actions = controller.apply_context_command(
            NetDlgEditContextCommand::Paste,
            Some("one\r\n\r\ntwo\nfinal|tail"),
            text_font(),
        );
        assert_same!(actions => chat_actions([("one", message_command("one")), ("two", message_command("two"))]));
        assert_eq!(controller.chat_input(), "final¦tail");

        controller.chat_edit.set_text("");
        let actions = controller.apply_context_command(
            NetDlgEditContextCommand::Paste,
            Some("/quit bye\nignored\n"),
            text_font(),
        );
        assert_same!(actions => chat_actions([("/quit bye", NetDlgChatCommand::Quit { reason: "bye".into() })]));
        assert_eq!(controller.chat_input(), "");

        let actions = controller.apply_context_command(
            NetDlgEditContextCommand::Paste,
            Some("/part\nignored\n"),
            text_font(),
        );
        assert_same!(actions => chat_actions([("/part", NetDlgChatCommand::Part { channel: "#clonken".into() })]));
    }

    /// Chat `TextWindow`s are built with `fMarkup = false` (C4Gui.h:1309), so
    /// `C4LogBuffer::AppendLines` breaks on CR/LF only and leaves `|` literal
    /// (C4LogBuf.cpp:180-183). `Update` tests `MSG_Notice` before the
    /// empty-source Server fallback, and `OpenQuery` keys queries by the post-`!`
    /// ident so a nick change reuses and retitles the sheet
    /// (C4ChatDlg.cpp:729-773,834-854).
    #[test]
    fn irc_transcript_literal_pipes_and_query_routing_match_cpp() {
        let mut controller = controller();
        controller.set_text_font(text_font());
        controller.sync_chat_snapshot(chat_snapshot(
            NetDlgChatConnectionState::Connected,
            vec![channel_message("a|b")],
            0,
        ));
        controller.force_chat_mode_and_focus();
        let channel = chat_sheet_index(&controller, NetDlgChatSheetKind::Channel, "channel sheet");
        let lines = NetDlgController::wrapped_chat_lines(&controller.chat_sheets()[channel]);
        assert_same!(lines .iter() .map(|line| line.text.as_str()) .collect::<Vec<_>>() => vec!["<Keeper> a|b"]);

        // A source-less notice reaches the active sheet, not Server.
        controller.select_chat_sheet(channel);
        let mut snapshot = chat_snapshot(NetDlgChatConnectionState::Connected, Vec::new(), 0);
        snapshot.messages = vec![chat_message(
            NetDlgChatMessageKind::Notice,
            "",
            "Clonker",
            "server notice",
            false,
        )];
        snapshot.unread_index = 0;
        controller.sync_chat_snapshot(snapshot);
        let channel = chat_sheet_index(&controller, NetDlgChatSheetKind::Channel, "channel sheet");
        assert!(
            controller.chat_sheets()[channel]
                .lines
                .iter()
                .any(|line| line.text.contains("server notice")),
            "{:?}",
            controller.chat_sheets()[channel].lines
        );
        assert!(
            !controller.chat_sheets()[0]
                .lines
                .iter()
                .any(|line| line.text.contains("server notice")),
            "{:?}",
            controller.chat_sheets()[0].lines
        );

        // One query survives the sender's nick change because its ident did not.
        let mut snapshot = chat_snapshot(NetDlgChatConnectionState::Connected, Vec::new(), 0);
        snapshot.messages = vec![
            chat_message(
                NetDlgChatMessageKind::Message,
                "Keeper!ident@example",
                "Clonker",
                "first",
                false,
            ),
            chat_message(
                NetDlgChatMessageKind::Message,
                "Wache!ident@example",
                "Clonker",
                "second",
                false,
            ),
        ];
        snapshot.unread_index = 0;
        controller.sync_chat_snapshot(snapshot);
        let queries = controller
            .chat_sheets()
            .iter()
            .filter(|sheet| sheet.kind == NetDlgChatSheetKind::Query)
            .collect::<Vec<_>>();
        assert_eq!(queries.len(), 1, "{queries:?}");
        assert_eq!(queries[0].title, "Wache");
        assert_eq!(queries[0].ident, "ident@example");
        assert_eq!(
            queries[0]
                .lines
                .iter()
                .map(|line| line.text.as_str())
                .collect::<Vec<_>>(),
            vec!["<Keeper> first", "<Wache> second"]
        );
    }

    /// `OnChatInput` answers empty submission with `DoError(nullptr)`, which is
    /// sound-only (C4ChatDlg.cpp:250-273,329-336). `SplitAtChar` keeps every
    /// character after the first delimiter (StdBuf.h:579-588), the invalid-nick
    /// error carries its command token, and a failed send is reported on the
    /// originating sheet rather than Server (C4ChatDlg.cpp:899-1018).
    #[test]
    fn irc_process_input_errors_spacing_and_send_failure_match_cpp() {
        let mut controller = focused_chat_controller(Vec::new(), 0);
        let channel = chat_sheet_index(&controller, NetDlgChatSheetKind::Channel, "channel sheet");
        controller.select_chat_sheet(channel);

        // Empty submission sounds without a transcript line or a history entry.
        controller.handle_key_down(KeyCode::Up);
        let before = controller.chat_sheets()[channel].lines.clone();
        let actions = controller.submit_chat_input();
        assert_action!(actions => NetDlgAction::GuiSound(NetDlgSound::Error));
        assert_eq!(controller.chat_sheets()[channel].lines, before);
        assert!(controller.chat_history().is_empty());
        assert_eq!(controller.chat_input(), "");

        // Everything after the first delimiter survives verbatim.
        controller.handle_text_input("/ns  identify   secret ", text_font());
        let actions = controller.submit_chat_input();
        assert!(
            actions.contains(&NetDlgAction::ChatCommand(NetDlgChatCommand::Message {
                target: "NickServ".into(),
                text: " identify   secret ".into(),
            })),
            "{actions:?}"
        );
        controller.handle_text_input("/raw PRIVMSG #clonken  :two  spaces", text_font());
        let actions = controller.submit_chat_input();
        assert!(
            actions.contains(&NetDlgAction::ChatCommand(NetDlgChatCommand::Raw(
                "PRIVMSG #clonken  :two  spaces".into()
            ))),
            "{actions:?}"
        );

        // The invalid-nick error carries the command token like IDS_ERR_INVALIDNICKNAME2.
        controller.handle_text_input("/nick bad nick", text_font());
        controller.submit_chat_input();
        assert_same!(controller.chat_sheets()[channel] .lines .last() .map(|line| line.text.as_str()) => Some("/nick: invalid nick name"));

        // A queued send that fails reports on the sheet it was typed into.
        controller.handle_text_input("hello", text_font());
        controller.submit_chat_input();
        let mut snapshot = chat_snapshot(NetDlgChatConnectionState::Connected, Vec::new(), 0);
        snapshot.last_error = Some("Send failed".into());
        controller.sync_chat_snapshot(snapshot);
        let channel = chat_sheet_index(&controller, NetDlgChatSheetKind::Channel, "channel sheet");
        assert!(
            controller.chat_sheets()[channel]
                .lines
                .iter()
                .any(|line| line.text.contains("Send failed")),
            "{:?}",
            controller.chat_sheets()[channel].lines
        );
        assert!(
            !controller.chat_sheets()[0]
                .lines
                .iter()
                .any(|line| line.text.contains("Send failed")),
            "{:?}",
            controller.chat_sheets()[0].lines
        );
    }

    #[test]
    fn chat_history_is_shared_order_bounded_deduplicated_and_cycles_to_empty() {
        let mut controller = focused_chat_controller(Vec::new(), 0);
        let mut seeded = (0..22)
            .map(|index| format!("entry {index}"))
            .collect::<Vec<_>>();
        seeded.insert(2, "entry 0".into());
        controller.set_chat_history(seeded);
        assert_eq!(controller.chat_history().len(), 20);
        assert_eq!(controller.chat_history()[0], "entry 0");
        assert_eq!(controller.chat_history()[1], "entry 1");

        controller.handle_key_down(KeyCode::Up);
        assert_eq!(controller.chat_input(), "entry 0");
        controller.handle_key_down(KeyCode::Up);
        assert_eq!(controller.chat_input(), "entry 1");
        for _ in 0..19 {
            controller.handle_key_down(KeyCode::Up);
        }
        assert_eq!(controller.chat_input(), "");
        controller.handle_key_down(KeyCode::Up);
        assert_eq!(controller.chat_input(), "entry 0");
        controller.handle_key_down(KeyCode::Down);
        assert_eq!(controller.chat_input(), "");

        controller.handle_text_input("entry 5", text_font());
        let actions = controller.submit_chat_input();
        assert_same!(actions[0] => NetDlgAction::ChatHistoryStored("entry 5".into()));
        assert_eq!(controller.chat_history()[0], "entry 5");
        assert_same!(controller .chat_history() .iter() .filter(|entry| entry.as_str() == "entry 5") .count() => 1);
        assert_eq!(controller.chat_history().len(), 20);
    }

    #[test]
    fn chat_transcript_wheel_breaks_follow_and_new_line_restores_bottom() {
        let mut controller = controller();
        controller.resize(640, 480);
        controller.set_text_font(text_font());
        let messages = (0..60)
            .map(|index| {
                channel_message(format!(
                    "wrapped transcript line {index} with enough words to wrap"
                ))
            })
            .collect::<Vec<_>>();
        controller.sync_chat_snapshot(chat_snapshot(
            NetDlgChatConnectionState::Connected,
            messages.clone(),
            0,
        ));
        controller.force_chat_mode_and_focus();
        let max_before = controller.chat_transcript_max_scroll();
        assert!(max_before > 0);
        assert!(controller.chat_transcript_follows_bottom());
        let transcript = controller.chat_layout().transcript_viewport;
        controller.handle_wheel(center(transcript), 60);
        assert!(!controller.chat_transcript_follows_bottom());
        assert!(controller.chat_transcript_scroll_offset() < max_before);

        let mut updated = messages;
        updated.push(channel_message("fresh channel line"));
        controller.sync_chat_snapshot(chat_snapshot(
            NetDlgChatConnectionState::Connected,
            updated,
            60,
        ));
        assert!(controller.chat_transcript_follows_bottom());
        assert_same!(controller.chat_transcript_scroll_offset() => controller.chat_transcript_max_scroll());
    }

    #[test]
    fn standalone_chat_renderer_uses_override_and_draws_only_chat_dialog() {
        let mut controller = controller();
        controller.resize(1000, 800);
        let bounds = NetDlgController::standalone_chat_bounds(1000, 800);
        assert_same!(bounds => IntRect::new(100, 80, 800, 640));
        controller.set_chat_bounds_override(Some(bounds));
        controller.force_chat_mode_and_focus();
        controller.set_chat_strings(NetDlgChatStrings {
            chat: "IRC".into(),
            not_connected: "Offline".into(),
            ..NetDlgChatStrings::default()
        });
        let layout = controller.chat_layout();
        assert_same!(layout.group.y => bounds.y + metrics().text_line_height.max(23));
        assert_same!(layout.group.h => bounds.h - metrics().text_line_height.max(23));

        let fonts = endeavour_font_set();
        let assets = net_assets();
        let mut surface = Surface::new(1000, 800, PixelFormat::Rgba8888);
        surface.begin_clonk_text_capture();
        NetDlgScreen::render_standalone_chat_dialog(
            &mut surface,
            &assets,
            &fonts,
            None,
            &controller,
            true,
        );
        let texts = surface
            .take_clonk_text_capture()
            .into_iter()
            .map(|command| command.text)
            .collect::<Vec<_>>();
        assert!(texts.iter().any(|text| text == "IRC - Offline"));
        assert!(!texts.iter().any(|text| text == "Start Network Game"));
        assert!(!texts.iter().any(|text| text == "Games"));
        assert!(!texts.iter().any(|text| text == "X"));
        let close = NetDlgController::chat_dialog_close_rect(controller.chat_caption_and_group().0);
        assert_action!(click(&mut controller, close) => NetDlgAction::ChatDialogCloseRequested);
        assert_same!(controller.chat_connection_state() => NetDlgChatConnectionState::Disconnected);
    }

    #[test]
    fn chat_tooltips_match_native_controls_and_standalone_hides_startup_chrome() {
        let mut controller = controller();
        controller.resize(1000, 800);
        controller.sync_chat_snapshot(chat_snapshot(
            NetDlgChatConnectionState::Connected,
            Vec::new(),
            0,
        ));
        controller
            .set_chat_bounds_override(Some(NetDlgController::standalone_chat_bounds(1000, 800)));
        controller.force_chat_mode_and_default_focus();

        let chat = controller.chat_layout();
        assert_same!(controller.tooltip_at(center(chat.input)) => Some(StartupTooltip::resource("IDS_DLGTIP_CHAT")));
        assert_eq!(
            controller.tooltip_at(center(chat.input_label.expect("channel input label"))),
            Some(StartupTooltip::resource("IDS_DLGTIP_CHAT"))
        );
        let caption = controller.chat_caption_and_group().0;
        assert_eq!(
            controller.tooltip_at(center(NetDlgController::chat_dialog_close_rect(caption))),
            Some(StartupTooltip::resource("IDS_MNU_CLOSE"))
        );

        let hidden_startup_control = net_dlg_layout(1000, 800, &metrics()).buttons[0];
        assert!(!contains(
            controller
                .chat_bounds_override()
                .expect("standalone bounds"),
            center(hidden_startup_control),
        ));
        assert_eq!(controller.tooltip_at(center(hidden_startup_control)), None);

        controller.set_chat_bounds_override(None);
        assert_same!(controller.tooltip_at(center(controller.chat_layout().input)) => Some(StartupTooltip::resource("IDS_DLGTIP_CHAT")));
    }

    #[test]
    fn standalone_chat_caption_drag_moves_override_and_close_keeps_precedence() {
        let mut controller = controller();
        controller.resize(1000, 800);
        let initial = NetDlgController::standalone_chat_bounds(1000, 800);
        controller.set_chat_bounds_override(Some(initial));
        controller.force_chat_mode_and_default_focus();

        let caption = controller.chat_caption_and_group().0;
        let close = NetDlgController::chat_dialog_close_rect(caption);
        assert_no_actions!(controller.handle_pointer_down(center(close), text_font()));
        assert!(!controller.chat_dialog_drag_active());
        assert_action!(controller.handle_pointer_up(center(close), text_font()) => NetDlgAction::ChatDialogCloseRequested);
        assert_eq!(controller.chat_bounds_override(), Some(initial));

        let start = GuiPoint::new((caption.x + 12) as f32, (caption.y + 8) as f32);
        assert_no_actions!(controller.handle_pointer_down(start, text_font()));
        assert!(controller.chat_dialog_drag_active());
        let moved = GuiPoint::new(start.x - 325.0, start.y + 41.0);
        assert_no_actions!(controller.handle_pointer_move(moved, text_font()));
        assert_same!(controller.chat_bounds_override() => Some(initial.with_position(initial.x - 325, initial.y + 41)));

        let released = GuiPoint::new(moved.x - 7.0, moved.y + 3.0);
        assert_no_actions!(controller.handle_pointer_up(released, text_font()));
        assert_same!(controller.chat_bounds_override() => Some(initial.with_position(initial.x - 332, initial.y + 44)));
        assert!(!controller.chat_dialog_drag_active());
        let retained = controller.chat_bounds_override();
        controller.handle_pointer_move(GuiPoint::new(900.0, 700.0), text_font());
        assert_eq!(controller.chat_bounds_override(), retained);

        let mut embedded = self::controller();
        embedded.resize(1000, 800);
        embedded.force_chat_mode_and_default_focus();
        let embedded_caption = embedded.chat_caption_and_group().0;
        embedded.handle_pointer_down(center(embedded_caption), text_font());
        assert!(!embedded.chat_dialog_drag_active());
    }

    #[test]
    fn standalone_chat_drag_capture_is_cleared_by_leave_resize_and_cancel() {
        let mut controller = controller();
        controller.resize(1000, 800);
        controller.set_chat_bounds_override(Some(IntRect::new(100, 80, 800, 640)));
        controller.force_chat_mode_and_default_focus();

        let start_drag = |controller: &mut NetDlgController| {
            let caption = controller.chat_caption_and_group().0;
            let point = GuiPoint::new((caption.x + 10) as f32, (caption.y + 8) as f32);
            assert_no_actions!(controller.handle_pointer_down(point, text_font()));
            assert!(controller.chat_dialog_drag_active());
        };

        start_drag(&mut controller);
        let retained = controller.chat_bounds_override();
        controller.pointer_left();
        assert!(!controller.chat_dialog_drag_active());
        controller.handle_pointer_move(GuiPoint::new(0.0, 0.0), text_font());
        assert_eq!(controller.chat_bounds_override(), retained);

        start_drag(&mut controller);
        controller.resize(1200, 900);
        assert!(!controller.chat_dialog_drag_active());

        start_drag(&mut controller);
        controller.cancel_interaction();
        assert!(!controller.chat_dialog_drag_active());
    }

    #[test]
    fn standalone_chat_default_focus_uses_connect_then_live_message_input() {
        let mut controller = controller();
        controller.set_chat_login(chat_login());
        controller.force_chat_mode_and_default_focus();
        assert!(controller.chat_connect_focused);
        assert_action!(controller.handle_key_down(KeyCode::Enter) => NetDlgAction::ChatConnect(chat_login()));

        controller.sync_chat_snapshot(chat_snapshot(
            NetDlgChatConnectionState::Connected,
            Vec::new(),
            0,
        ));
        controller.force_chat_mode_and_default_focus();
        assert!(!controller.chat_connect_focused);
        assert_no_actions!(controller.handle_text_input("live input", text_font()));
        assert_chat!(controller.handle_key_down(KeyCode::Enter) => "live input" => message_command("live input"));
    }

    /// Renders the dialog at 1280x720 with the final whole-surface gamma pass
    /// (mirroring the app's render_startup_frame) and dumps the PPM artifact
    /// that is diffed offline against the C++ F9 reference. CI has no
    /// reference, so this test only checks coarse invariants.
    #[test]
    fn render_matches_reference() {
        use crate::test_support::{standard_gamma, write_ppm};
        let assets = net_assets();
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
