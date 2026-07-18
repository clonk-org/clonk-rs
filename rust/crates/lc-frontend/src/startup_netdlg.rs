//! Pixel-parity renderer for the C++ `C4StartupNetDlg` startup dialog
//! ("Start Network Game"), mirroring the engine's first-shown state
//! (see `rust/target/parity-specs/net.md`). Implemented against the
//! engine's F9 reference capture at 1280x720; owned by its
//! implementation agent.
//!
//! Geometry mirrors the C++ constructor math (C4StartupNetDlg.cpp:631-728)
//! with the fullscreen-dialog margins of C4GuiDialogs.cpp:813-822 and the
//! ComponentAligner of C4Gui.cpp:975-1057.

use crate::classic_gui::{
    blacken_transparent_pixels, draw_3d_frame, draw_clipped_text, draw_engine_box,
    ClassicButtonState, ClassicGuiSkin,
};
use crate::clonk_fonts::{expand_hotkey_markup, ClonkFontSet};
use crate::startup_main_menu::IntRect;
use crate::{GuiPoint, ImageData, KeyCode};
use lc_graphics::clonk_font::TextAlign;
use lc_graphics::{GammaRamp, Surface};
use lc_gui::Rect as GuiRect;

// Engine colors (C4Gui.h:52-103,163-165). Font colors are NORMAL-alpha RGBA
// (0xff = opaque); box/line colors are engine AARRGGBB with INVERTED alpha
// (0x00 = opaque).
/// C4GUI_FullscreenCaptionFontClr / C4GUI_Caption2FontClr / C4GUI_ButtonFontClr.
const CLR_YELLOW: [u8; 4] = [0xff, 0xff, 0x00, 0xff];
/// C4GUI_CaptionFontClr / C4GUI_MessageFontClr.
const CLR_WHITE: [u8; 4] = [0xff, 0xff, 0xff, 0xff];
const CLR_DISABLED: [u8; 4] = [0x7f, 0x7f, 0x7f, 0xff];
/// ListBox background / C4GUI_EditBGColor.
const CLR_DARK_BG: u32 = 0x7f00_0000;
const SCROLLBAR_WIDTH: i32 = 16;
const SCROLLBAR_PART: i32 = 16;

/// The `Graphics.c4g` images `C4StartupNetDlg` draws (C4Startup.cpp:48,82-83;
/// C4Gui.cpp:1087-1097).
pub struct NetDlgAssets {
    /// `StartupNetworkBG.png` (800x600): fullscreen background.
    pub background: ImageData,
    /// `StartupNetGetRef.png` (2000x32): 50-phase animated query icon.
    pub net_get_ref: ImageData,
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

/// One C++ inclusive-corner clipper covering `rect` (x2 = x + w, y2 = y + h;
/// the covered span is one pixel wider/taller than the rect).
fn inclusive_clip(rect: IntRect) -> IntRect {
    IntRect {
        x: rect.x,
        y: rect.y,
        w: rect.w + 1,
        h: rect.h + 1,
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

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct NetDlgGameEntry {
    pub title: String,
    pub details: String,
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

/// Requests produced by [`NetDlgController`]. The controller mutates only
/// presentation-local state; the application owns network/config side effects.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum NetDlgAction {
    FocusChanged(NetDlgControl),
    ModeChanged(NetDlgMode),
    Back,
    Refresh,
    JoinGame { address: Option<String> },
    CreateGame,
    MasterserverSignupChanged(bool),
    RecordingChanged(bool),
    JoinAddressChanged(String),
    GuiSound(NetDlgSound),
}

/// Live input state for the pixel-parity network dialog.
///
/// Pointer hits use the same half-open integer rectangles as `C4Rect::Contains`
/// (C4Rect.h:40-43). Buttons retain C++'s press-on-down, invoke-on-up model
/// (C4GuiButton.cpp:112-155), including keyboard activation.
pub struct NetDlgController {
    metrics: NetDlgFontMetrics,
    width: i32,
    height: i32,
    config: NetDlgConfig,
    mode: NetDlgMode,
    join_address: String,
    focus: NetDlgControl,
    pointer_position: Option<GuiPoint>,
    hovered: Option<NetDlgControl>,
    pointer_pressed: Option<NetDlgControl>,
    key_pressed: Option<(NetDlgControl, KeyCode)>,
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

impl NetDlgController {
    pub fn new(config: NetDlgConfig, metrics: NetDlgFontMetrics) -> Self {
        Self {
            metrics,
            width: 1,
            height: 1,
            config,
            mode: NetDlgMode::GameList,
            join_address: String::new(),
            // C4StartupNetDlg.cpp:734 / GetDlgModeFocusControl: game list.
            focus: NetDlgControl::GameList,
            pointer_position: None,
            hovered: None,
            pointer_pressed: None,
            key_pressed: None,
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

    pub fn join_address(&self) -> &str {
        &self.join_address
    }

    pub fn set_join_address(&mut self, address: impl Into<String>) {
        self.join_address = address.into();
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

    pub fn games(&self) -> &[NetDlgGameEntry] {
        &self.games
    }

    pub fn selected_game(&self) -> Option<usize> {
        match self.selection {
            Some(NetDlgSelection::Game(index)) => Some(index),
            Some(NetDlgSelection::Masterserver) | None => None,
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

    /// Adds text received from the windowing layer while the IP edit owns
    /// focus. `KeyCode` intentionally contains navigation keys only, so text
    /// input is a separate operation just like C4GUI::Edit::CharIn.
    pub fn handle_text_input(&mut self, text: &str) -> Vec<NetDlgAction> {
        if self.focus != NetDlgControl::JoinAddress || self.mode != NetDlgMode::GameList {
            return Vec::new();
        }
        self.join_address
            .extend(text.chars().filter(|character| !character.is_control()));
        vec![NetDlgAction::JoinAddressChanged(self.join_address.clone())]
    }

    pub fn delete_join_address_char(&mut self) -> Vec<NetDlgAction> {
        if self.focus != NetDlgControl::JoinAddress || self.mode != NetDlgMode::GameList {
            return Vec::new();
        }
        self.join_address.pop();
        vec![NetDlgAction::JoinAddressChanged(self.join_address.clone())]
    }

    pub fn handle_pointer_move(&mut self, position: GuiPoint) -> Vec<NetDlgAction> {
        self.pointer_position = Some(position);
        self.hovered = self.hit_button(position);
        let layout = self.layout();
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

    pub fn handle_pointer_down(&mut self, position: GuiPoint) -> Vec<NetDlgAction> {
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
            Some(control @ (NetDlgControl::GameList | NetDlgControl::JoinAddress)) => {
                if control == NetDlgControl::GameList {
                    self.select_list_row(position);
                }
                self.change_focus(control)
            }
            _ => Vec::new(),
        }
    }

    pub fn handle_pointer_up(&mut self, position: GuiPoint) -> Vec<NetDlgAction> {
        self.pointer_position = Some(position);
        self.hovered = self.hit_button(position);
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
            KeyCode::Tab => self.advance_focus(),
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

    fn advance_focus(&mut self) -> Vec<NetDlgAction> {
        self.move_focus(false)
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
        self.focus = focus;
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

    fn join_action(&self) -> Vec<NetDlgAction> {
        let selected_address = self
            .selected_game()
            .and_then(|index| self.games.get(index))
            .filter(|game| game.joinable)
            .and_then(|game| game.address.clone());
        vec![NetDlgAction::JoinGame {
            address: (self.focus == NetDlgControl::JoinAddress && !self.join_address.is_empty())
                .then(|| self.join_address.clone())
                .or(selected_address),
        }]
    }

    fn select_list_row(&mut self, position: GuiPoint) {
        let layout = self.layout();
        if !contains(layout.list_viewport, position) {
            return;
        }
        let previous = self.selection;
        let row = ((position.y as i32 - layout.list_viewport.y + self.list_scroll_y)
            / layout.list_entry.h) as usize;
        if self.config.masterserver_signup {
            self.selection = if row == 0 {
                Some(NetDlgSelection::Masterserver)
            } else {
                (row - 1 < self.games.len()).then_some(NetDlgSelection::Game(row - 1))
            };
        } else {
            self.selection = (row < self.games.len()).then_some(NetDlgSelection::Game(row));
        }
        if self.selection != previous {
            self.ensure_selection_visible(&layout);
        }
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

    fn list_row_count(&self) -> usize {
        self.games.len() + usize::from(self.config.masterserver_signup)
    }

    fn list_content_height(&self, layout: &NetDlgLayout) -> i32 {
        i32::try_from(self.list_row_count())
            .unwrap_or(i32::MAX)
            .saturating_mul(layout.list_entry.h)
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

    fn selected_row(&self) -> Option<usize> {
        match self.selection {
            Some(NetDlgSelection::Masterserver) => Some(0),
            Some(NetDlgSelection::Game(index)) => {
                Some(index + usize::from(self.config.masterserver_signup))
            }
            None => None,
        }
    }

    fn ensure_selection_visible(&mut self, layout: &NetDlgLayout) {
        let Some(row) = self.selected_row() else {
            return;
        };
        let top = i32::try_from(row)
            .unwrap_or(i32::MAX)
            .saturating_mul(layout.list_entry.h);
        let bottom = top.saturating_add(layout.list_entry.h);
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
        Self::render_impl(surface, assets, fonts, gamma, config, None, get_ref_phase);
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
        Self::render_impl(
            surface,
            assets,
            fonts,
            gamma,
            controller.config,
            Some(controller),
            get_ref_phase,
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
            let mut row = 0_i32;
            if config.masterserver_signup {
                let dy = -scroll_y;
                let row_rect = offset(layout.list_entry, 0, dy);
                if row_visible(row_rect) {
                    if controller
                        .is_some_and(|state| state.selection == Some(NetDlgSelection::Masterserver))
                    {
                        draw_engine_box(
                            surface,
                            row_rect.x,
                            row_rect.y,
                            row_rect.x + row_rect.w - 1,
                            row_rect.y + row_rect.h - 1,
                            0xafaf_0000,
                            gamma,
                        );
                    }
                    // The masterserver query entry: animated NetGetRef icon,
                    // aspect-fit 40x32 -> 48x38 (C4StartupNetDlg.cpp:144-160).
                    let phase = net_get_ref_phase(&assets.net_get_ref, get_ref_phase);
                    crate::draw_image_bilinear(
                        surface,
                        &gui_rect(offset(layout.entry_icon, 0, dy)),
                        &phase,
                        gamma,
                    );
                    let entry_texts = [
                        "Internet server on league.clonkspot.org",
                        "Querying game infos...",
                    ];
                    for (rect, text) in layout
                        .entry_labels
                        .iter()
                        .map(|rect| offset(*rect, 0, dy))
                        .zip(entry_texts)
                    {
                        fonts.text.draw_with_gamma(
                            surface,
                            rect.x,
                            rect.y,
                            text,
                            CLR_WHITE,
                            TextAlign::Left,
                            true,
                            gamma,
                        );
                    }
                }
                row += 1;
            }

            if let Some(controller) = controller {
                for (index, game) in controller.games().iter().enumerate() {
                    let dy = row * layout.list_entry.h - scroll_y;
                    let row_rect = offset(layout.list_entry, 0, dy);
                    if !row_visible(row_rect) {
                        row += 1;
                        continue;
                    }
                    if controller.selection == Some(NetDlgSelection::Game(index)) {
                        draw_engine_box(
                            surface,
                            row_rect.x,
                            row_rect.y,
                            row_rect.x + row_rect.w - 1,
                            row_rect.y + row_rect.h - 1,
                            0xafaf_0000,
                            gamma,
                        );
                    }
                    let color = if game.joinable {
                        CLR_WHITE
                    } else {
                        CLR_DISABLED
                    };
                    for (rect, text) in layout
                        .entry_labels
                        .iter()
                        .map(|rect| offset(*rect, 0, dy))
                        .zip([game.title.as_str(), game.details.as_str()])
                    {
                        fonts.text.draw_with_gamma(
                            surface,
                            rect.x,
                            rect.y,
                            text,
                            color,
                            TextAlign::Left,
                            true,
                            gamma,
                        );
                    }
                    row += 1;
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

            // Join-address edit, empty and unfocused (Edit::DrawElement,
            // C4GuiEdit.cpp:556-634): background box down to the client-rect
            // bottom (margins L4 R4 T2 B2, C4GuiEdit.h:102-105), default 3D
            // frame, no text, no caret.
            let edit = layout.join_edit;
            draw_engine_box(
                surface,
                edit.x,
                edit.y,
                edit.x + edit.w - 1,
                edit.y + 2 + (edit.h - 4),
                CLR_DARK_BG,
                gamma,
            );
            draw_3d_frame(surface, edit, gamma);
            if let Some(address) = controller.map(|state| state.join_address()) {
                draw_clipped_text(
                    surface,
                    &fonts.text,
                    edit.x + 4,
                    edit.y + 1,
                    address,
                    CLR_WHITE,
                    TextAlign::Left,
                    gamma,
                    inclusive_clip(edit),
                );
                if controller.is_some_and(|state| state.focus == NetDlgControl::JoinAddress) {
                    let caret_x = edit.x + 4 + fonts.text.measure(address, false).0;
                    draw_clipped_text(
                        surface,
                        &fonts.text,
                        caret_x,
                        edit.y - fonts.text.line_height / 3,
                        "¦",
                        CLR_WHITE,
                        TextAlign::Left,
                        gamma,
                        inclusive_clip(edit),
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
    use super::*;
    use crate::test_support::endeavour_font_set;
    use lc_graphics::PixelFormat;

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
            })
            .collect()
    }

    fn center(rect: IntRect) -> crate::GuiPoint {
        crate::GuiPoint::new((rect.x + rect.w / 2) as f32, (rect.y + rect.h / 2) as f32)
    }

    fn click(controller: &mut NetDlgController, rect: IntRect) -> Vec<NetDlgAction> {
        let point = center(rect);
        assert!(controller.handle_pointer_down(point).is_empty());
        controller.handle_pointer_up(point)
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
            controller.handle_pointer_down(center(layout.join_edit)),
            vec![NetDlgAction::FocusChanged(NetDlgControl::JoinAddress)]
        );
        assert_eq!(
            click(&mut controller, layout.buttons[2]),
            vec![NetDlgAction::JoinGame {
                address: Some(" 127.0.0.1:11111 ".into())
            }]
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
        assert!(controller.handle_pointer_down(outside).is_empty());
        assert!(controller.handle_pointer_up(outside).is_empty());
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
    fn direct_join_requires_edit_focus_and_preserves_raw_text() {
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
            controller.handle_pointer_down(center(layout.join_edit)),
            vec![NetDlgAction::FocusChanged(NetDlgControl::JoinAddress)]
        );
        assert_eq!(
            controller.handle_key_down(crate::KeyCode::Enter),
            vec![NetDlgAction::JoinGame {
                address: Some("   ".into())
            }]
        );
        assert_eq!(
            click(&mut controller, layout.buttons[2]),
            vec![NetDlgAction::JoinGame {
                address: Some("   ".into())
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
    fn discovered_rows_are_selectable_and_supply_the_reference_address() {
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
            },
            NetDlgGameEntry {
                title: "Wrong version".into(),
                details: "LegacyClonk 4.9.11.0 [363]".into(),
                address: Some("203.0.113.11:11112".into()),
                joinable: false,
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
            vec![NetDlgAction::JoinGame { address: None }]
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

        assert_eq!(controller.list_max_scroll(), 470);
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

        controller.handle_wheel(point, -96);
        assert!(controller.handle_pointer_down(point).is_empty());
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
            let top = index as i32 * layout.list_entry.h;
            let bottom = top + layout.list_entry.h;
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
            controller.handle_pointer_down(center(layout.join_edit)),
            vec![NetDlgAction::FocusChanged(NetDlgControl::JoinAddress)]
        );
        assert_eq!(
            controller.handle_pointer_down(track),
            vec![
                NetDlgAction::FocusChanged(NetDlgControl::GameList),
                NetDlgAction::GuiSound(NetDlgSound::Command),
            ]
        );
        assert!(controller.list_scroll_offset() > 0);
        let below = GuiPoint::new(track.x, (layout.list_scrollbar.y + 10_000) as f32);
        assert!(controller.handle_pointer_move(below).is_empty());
        assert_eq!(
            controller.list_scroll_offset(),
            controller.list_max_scroll()
        );
        assert!(controller.handle_pointer_up(below).is_empty());

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
            controller.handle_pointer_down(bottom_arrow),
            vec![NetDlgAction::GuiSound(NetDlgSound::ArrowHit)]
        );
        assert_eq!(controller.list_scroll_pin, 0);
        assert!(controller.tick_scrollbar());
        assert_eq!(controller.list_scroll_pin, 1);
        assert!(controller.tick_scrollbar());
        assert_eq!(controller.list_scroll_pin, 2);
        assert_eq!(
            controller.handle_pointer_move(track),
            vec![NetDlgAction::GuiSound(NetDlgSound::ArrowHit)]
        );
        assert!(!controller.tick_scrollbar());
        assert_eq!(
            controller.handle_pointer_move(bottom_arrow),
            vec![NetDlgAction::GuiSound(NetDlgSound::ArrowHit)]
        );
        assert!(controller.tick_scrollbar());
        assert_eq!(controller.list_scroll_pin, 3);
        let after_held_frames = controller.list_scroll_offset();
        assert_eq!(
            controller.handle_pointer_up(bottom_arrow),
            vec![NetDlgAction::GuiSound(NetDlgSound::ArrowHit)]
        );
        assert!(!controller.tick_scrollbar());
        assert_eq!(controller.list_scroll_offset(), after_held_frames);

        controller.change_mode(NetDlgMode::Chat);
        let before_hidden_click = controller.list_scroll_offset();
        assert!(controller.handle_pointer_down(bottom_arrow).is_empty());
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
        assert_eq!(controller.list_max_scroll(), 144);
        let bottom_arrow = GuiPoint::new(
            (layout.list_scrollbar.x + 8) as f32,
            (layout.list_scrollbar.y + layout.list_scrollbar.h - 8) as f32,
        );
        assert_eq!(
            controller.handle_pointer_down(bottom_arrow),
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
            gui_caption: load_graphics_png("GUICaption.png"),
            gui_button: load_graphics_png("GUIButton.png"),
            gui_button_down: load_graphics_png("GUIButtonDown.png"),
            gui_button_highlight: load_graphics_png("GUIButtonHighlight.png"),
            gui_scroll: load_graphics_png("GUIScroll.png"),
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
            .handle_pointer_down(center(layout.btn_internet))
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
            controller.join_address.clone(),
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
                controller.join_address.clone(),
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
            gui_caption: load_graphics_png("GUICaption.png"),
            gui_button: load_graphics_png("GUIButton.png"),
            gui_button_down: load_graphics_png("GUIButtonDown.png"),
            gui_button_highlight: load_graphics_png("GUIButtonHighlight.png"),
            gui_scroll: load_graphics_png("GUIScroll.png"),
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
        controller.handle_pointer_down(center(layout.join_edit));
        let with_address = render(&controller);
        assert_ne!(first_shown.pixels(), with_address.pixels());

        controller.handle_pointer_move(center(layout.buttons[0]));
        let hovered = render(&controller);
        assert_ne!(with_address.pixels(), hovered.pixels());
        controller.handle_pointer_down(center(layout.buttons[0]));
        let pressed = render(&controller);
        assert_ne!(hovered.pixels(), pressed.pixels());
        controller.handle_pointer_up(center(layout.buttons[0]));

        let actions = click(&mut controller, layout.btn_chat);
        assert!(actions.contains(&NetDlgAction::ModeChanged(NetDlgMode::Chat)));
        let chat = render(&controller);
        assert_ne!(with_address.pixels(), chat.pixels());
    }

    #[test]
    fn scrolled_rows_and_scrollbar_are_clipped_inside_the_list_client() {
        use crate::test_support::{load_graphics_png, standard_gamma};
        let assets = NetDlgAssets {
            background: load_graphics_png("StartupNetworkBG.png"),
            net_get_ref: load_graphics_png("StartupNetGetRef.png"),
            gui_caption: load_graphics_png("GUICaption.png"),
            gui_button: load_graphics_png("GUIButton.png"),
            gui_button_down: load_graphics_png("GUIButtonDown.png"),
            gui_button_highlight: load_graphics_png("GUIButtonHighlight.png"),
            gui_scroll: load_graphics_png("GUIScroll.png"),
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
            gui_caption: load_graphics_png("GUICaption.png"),
            gui_button: load_graphics_png("GUIButton.png"),
            gui_button_down: load_graphics_png("GUIButtonDown.png"),
            gui_button_highlight: load_graphics_png("GUIButtonHighlight.png"),
            gui_scroll: load_graphics_png("GUIScroll.png"),
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
            gui_caption: load_graphics_png("GUICaption.png"),
            gui_button: load_graphics_png("GUIButton.png"),
            gui_button_down: load_graphics_png("GUIButtonDown.png"),
            gui_button_highlight: load_graphics_png("GUIButtonHighlight.png"),
            gui_scroll: load_graphics_png("GUIScroll.png"),
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
