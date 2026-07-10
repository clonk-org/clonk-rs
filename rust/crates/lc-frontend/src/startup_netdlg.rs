//! Pixel-parity renderer for the C++ `C4StartupNetDlg` startup dialog
//! ("Start Network Game"), mirroring the engine's first-shown state
//! (see `rust/target/parity-specs/net.md`). Implemented against the
//! engine's F9 reference capture at 1280x720; owned by its
//! implementation agent.
//!
//! Geometry mirrors the C++ constructor math (C4StartupNetDlg.cpp:631-728)
//! with the fullscreen-dialog margins of C4GuiDialogs.cpp:813-822 and the
//! ComponentAligner of C4Gui.cpp:975-1057.

use crate::clonk_fonts::{expand_hotkey_markup, ClonkFontSet};
use crate::startup_main_menu::{draw_bar, IntRect};
use crate::{GuiPoint, ImageData, KeyCode};
use lc_graphics::clonk_font::{ClonkFont, TextAlign};
use lc_graphics::{Color, GammaRamp, PixelFormat, Surface};
use lc_gui::Rect as GuiRect;

// Engine colors (C4Gui.h:52-103,163-165). Font colors are NORMAL-alpha RGBA
// (0xff = opaque); box/line colors are engine AARRGGBB with INVERTED alpha
// (0x00 = opaque).
/// C4GUI_FullscreenCaptionFontClr / C4GUI_Caption2FontClr / C4GUI_ButtonFontClr.
const CLR_YELLOW: [u8; 4] = [0xff, 0xff, 0x00, 0xff];
/// C4GUI_CaptionFontClr / C4GUI_MessageFontClr.
const CLR_WHITE: [u8; 4] = [0xff, 0xff, 0xff, 0xff];
/// ListBox background / C4GUI_EditBGColor.
const CLR_DARK_BG: u32 = 0x7f00_0000;

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
    let mut ca_main = Aligner::new(IntRect { x: 0, y: 0, w: client.w, h: client.h }, 0, 0);
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
    let mut ca_game_list = Aligner::new(IntRect { x: 0, y: 0, w: tabular.w, h: tabular.h }, 0, 0);
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
    let list_entry = IntRect {
        x: list_client.x,
        y: list_client.y,
        w: list_client.w - 16,
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
        IntRect { x: label_x, y: list_entry.y + 1, w: label_w, h: metrics.text_line_height },
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
        list_entry,
        entry_icon,
        entry_labels,
        ip_label,
        join_edit,
        buttons,
    }
}

/// Draws a horizontal three-slice bar at native scale with an explicit
/// `border` slice width (DynBarFacet::SetHorizontal with iBorderWidth,
/// C4Gui.cpp:92-99), as used by the wooden caption bar (GUICaption.png,
/// border 32, height 23; C4Gui.cpp:1087-1088). Mirrors DrawBar's "exact bar"
/// branch including the narrow-bar overflow quirks: a truncated begin slice,
/// a middle loop that still starts at the full begin width, and an end slice
/// cropped from its left that overdraws right-aligned (C4Gui.cpp:283-311).
fn draw_bar_with_border(
    surface: &mut Surface,
    rect: IntRect,
    image: &ImageData,
    border: u32,
    gamma: Option<&GammaRamp>,
) {
    let h = image.height();
    let mid_w = image.width().saturating_sub(2 * border);
    let bar_w = rect.w;
    let end_show = (border / 3) as i32; // iRightShowLength (C4Gui.cpp:289)

    // begin slice, truncated when the bar is narrower (C4Gui.cpp:290-292)
    let begin_w = (border as i32).clamp(0, bar_w.max(0)) as u32;
    crate::draw_image_strip(surface, rect.x, rect.y, image, 0, 0, begin_w, h, gamma);

    // middle tiles; iX starts at the untruncated begin width (C4Gui.cpp:288,293-298)
    if mid_w > 0 {
        let mut ix = border as i32;
        while ix < bar_w - end_show {
            let tile_w = (mid_w as i32).min(bar_w - end_show - ix).max(0) as u32;
            crate::draw_image_strip(surface, rect.x + ix, rect.y, image, border, 0, tile_w, h, gamma);
            ix += mid_w as i32;
        }
    }

    // end slice, right-aligned and cropped from its left when overflowing
    // (fctEnd.X += wRight - Wdt; C4Gui.cpp:300-310), drawn last
    let end_w = (border as i32).clamp(0, bar_w.max(0)) as u32;
    let end_src_x = image.width() - border + (border - end_w);
    crate::draw_image_strip(
        surface,
        rect.x + bar_w - end_w as i32,
        rect.y,
        image,
        end_src_x,
        0,
        end_w,
        h,
        gamma,
    );
}

/// `CStdDDraw::DrawBoxDw` (StdDDraw2.cpp:1401-1404 -> CStdGL::DrawQuadDw,
/// StdGL.cpp:846-891): fills the INCLUSIVE rect (x1,y1)-(x2,y2) with an
/// engine AARRGGBB color whose alpha is inverted (0x00 = opaque). GL blends
/// `src*(1 - A/255) + dst*(A/255)` (glBlendFunc GL_ONE_MINUS_SRC_ALPHA /
/// GL_SRC_ALPHA on the raw engine color) with the source RGB gamma-encoded
/// by the blit shader's per-fragment lookup (StdGL.cpp:1082-1086).
fn draw_box_engine(
    surface: &mut Surface,
    x1: i32,
    y1: i32,
    x2: i32,
    y2: i32,
    clr: u32,
    gamma: Option<&GammaRamp>,
) {
    let a = (255 - ((clr >> 24) & 0xff)) as f32 / 255.0;
    if a <= 0.0 {
        return;
    }
    let enc = |c: u32| -> f32 {
        let v = (c & 0xff) as f32;
        gamma.map_or(v, |g| f32::from(g.encode_float(v)))
    };
    let (sr, sg, sb) = (enc(clr >> 16), enc(clr >> 8), enc(clr));
    let blend =
        |s: f32, d: u8| (s * a + f32::from(d) * (1.0 - a)).round().clamp(0.0, 255.0) as u8;
    for y in y1.max(0)..=y2.min(surface.height() as i32 - 1) {
        for x in x1.max(0)..=x2.min(surface.width() as i32 - 1) {
            let dst = surface.get_pixel(x as u32, y as u32).unwrap_or_default();
            let _ = surface.set_pixel(
                x as u32,
                y as u32,
                Color::new(blend(sr, dst.r), blend(sg, dst.g), blend(sb, dst.b), 255),
            );
        }
    }
}

/// `CStdGL::DrawLineDw` (StdGL.cpp:893-934) for the axis-aligned GUI frame
/// lines: a GL_LINES segment between the two pixel centers `+0.5`. By the GL
/// diamond-exit rule the segment never leaves the final pixel's diamond, so
/// the END pixel is not rasterized (verified against the F9 capture: frame
/// corners blend exactly once where the C++ lines meet, not twice). Blending
/// and gamma match DrawBoxDw — the engine inverts the alpha for the
/// SRC_ALPHA/ONE_MINUS_SRC_ALPHA path (StdGL.cpp:924), which is equivalent.
fn draw_line_engine(
    surface: &mut Surface,
    x1: i32,
    y1: i32,
    x2: i32,
    y2: i32,
    clr: u32,
    gamma: Option<&GammaRamp>,
) {
    // Axis-aligned only (all GUI 3D-frame lines are); drop the end pixel.
    if y1 == y2 && x2 > x1 {
        draw_box_engine(surface, x1, y1, x2 - 1, y2, clr, gamma);
    } else if x1 == x2 && y2 > y1 {
        draw_box_engine(surface, x1, y1, x2, y2 - 1, clr, gamma);
    } else {
        draw_box_engine(surface, x1, y1, x2, y2, clr, gamma);
    }
}

/// `C4GUI::Element::Draw3DFrame` (C4Gui.cpp:264-279) around `rect` with the
/// default border alpha 0xaf and the C4GUI_BorderColor1/2/3 palette
/// (C4Gui.h:97-100), drawn in the exact C++ order so pixels covered by two
/// lines blend twice.
fn draw_3d_frame(surface: &mut Surface, rect: IntRect, gamma: Option<&GammaRamp>) {
    const ALPHA: u32 = 0xaf << 24;
    const COLOR1: u32 = 0x0077_2200;
    const COLOR2: u32 = 0x0033_1100;
    const COLOR3: u32 = 0x00aa_4400;
    let (x0, y0) = (rect.x, rect.y);
    let (x1, y1) = (rect.x + rect.w - 1, rect.y + rect.h - 1);
    [
        (x0, y0, x1, y0, COLOR1),         // top outer
        (x0, y0, x0, y1, COLOR1),         // left outer
        (x0 + 1, y0 + 1, x1 - 1, y0 + 1, COLOR2), // top inner
        (x0 + 1, y0 + 1, x0 + 1, y1 - 1, COLOR2), // left inner
        (x0, y1, x1, y1, COLOR3),         // bottom outer
        (x1, y0, x1, y1, COLOR3),         // right outer
        (x0 + 1, y1 - 1, x1 - 1, y1 - 1, COLOR1), // bottom inner
        (x1 - 1, y0 + 1, x1 - 1, y1 - 1, COLOR1), // right inner
    ]
    .into_iter()
    .for_each(|(ax, ay, bx, by, c)| draw_line_engine(surface, ax, ay, bx, by, c | ALPHA, gamma));
}

/// Draws text clipped to a rect, mirroring the C++ primary clipper
/// (SetPrimaryClipper is INCLUSIVE of its x2/y2 corner; StdDDraw2.cpp:583-600).
/// `clip` is already in exclusive width/height form. Implemented by drawing
/// into a copy of the clipped region, which is blend-exact.
#[allow(clippy::too_many_arguments)]
fn draw_text_clipped(
    surface: &mut Surface,
    font: &ClonkFont,
    x: i32,
    y: i32,
    text: &str,
    color: [u8; 4],
    align: TextAlign,
    gamma: Option<&GammaRamp>,
    clip: IntRect,
) {
    let cx0 = clip.x.max(0);
    let cy0 = clip.y.max(0);
    let cx1 = (clip.x + clip.w).min(surface.width() as i32);
    let cy1 = (clip.y + clip.h).min(surface.height() as i32);
    if cx0 >= cx1 || cy0 >= cy1 {
        return;
    }
    let (cw, ch) = ((cx1 - cx0) as u32, (cy1 - cy0) as u32);
    let mut tmp = Surface::new(cw, ch, PixelFormat::Rgba8888);
    for ty in 0..ch {
        for tx in 0..cw {
            let src = surface.get_pixel(cx0 as u32 + tx, cy0 as u32 + ty).unwrap_or_default();
            let _ = tmp.set_pixel(tx, ty, src);
        }
    }
    font.draw_with_gamma(&mut tmp, x - cx0, y - cy0, text, color, align, true, gamma);
    for ty in 0..ch {
        for tx in 0..cw {
            if let Some(px) = tmp.get_pixel(tx, ty) {
                let _ = surface.set_pixel(cx0 as u32 + tx, cy0 as u32 + ty, px);
            }
        }
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
        }
    }

    pub fn resize(&mut self, width: i32, height: i32) {
        self.width = width.max(1);
        self.height = height.max(1);
        self.hovered = self.pointer_position.and_then(|point| self.hit_button(point));
    }

    pub const fn config(&self) -> NetDlgConfig {
        self.config
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
        }
    }

    pub fn pointer_left(&mut self) {
        self.set_pointer_position(None);
    }

    pub fn join_address(&self) -> &str {
        &self.join_address
    }

    pub fn set_join_address(&mut self, address: impl Into<String>) {
        self.join_address = address.into();
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
        Vec::new()
    }

    pub fn handle_pointer_down(&mut self, position: GuiPoint) -> Vec<NetDlgAction> {
        self.pointer_position = Some(position);
        self.hovered = self.hit_button(position);
        self.pointer_pressed = self.hovered;

        let hit = self.hit_control(position);
        match hit {
            Some(control @ (NetDlgControl::GameList | NetDlgControl::JoinAddress)) => {
                self.change_focus(control)
            }
            _ => Vec::new(),
        }
    }

    pub fn handle_pointer_up(&mut self, position: GuiPoint) -> Vec<NetDlgAction> {
        self.pointer_position = Some(position);
        self.hovered = self.hit_button(position);
        let Some(pressed) = self.pointer_pressed.take() else {
            return Vec::new();
        };
        if self.hit_button(position) != Some(pressed) {
            return Vec::new();
        }
        self.activate(pressed)
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
        let order = match self.mode {
            NetDlgMode::GameList => GAME_LIST_ORDER.as_slice(),
            NetDlgMode::Chat => CHAT_ORDER.as_slice(),
        };
        let index = order.iter().position(|control| *control == self.focus).unwrap_or(0);
        self.change_focus(order[(index + 1) % order.len()])
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
            NetDlgControl::JoinGame => self.join_action(),
            NetDlgControl::CreateGame => vec![NetDlgAction::CreateGame],
            NetDlgControl::GameList
            | NetDlgControl::JoinAddress
            | NetDlgControl::ChatInput => Vec::new(),
        }
    }

    fn change_mode(&mut self, mode: NetDlgMode) -> Vec<NetDlgAction> {
        self.mode = mode;
        let focus = match mode {
            NetDlgMode::GameList => NetDlgControl::GameList,
            NetDlgMode::Chat => NetDlgControl::ChatInput,
        };
        let mut actions = vec![NetDlgAction::ModeChanged(mode)];
        actions.extend(self.change_focus(focus));
        actions
    }

    fn join_action(&self) -> Vec<NetDlgAction> {
        let address = self.join_address.trim();
        vec![NetDlgAction::JoinGame {
            address: (!address.is_empty()).then(|| address.to_string()),
        }]
    }

    fn is_highlighted(&self, control: NetDlgControl) -> bool {
        self.focus == control || self.hovered == Some(control)
    }

    fn is_pressed(&self, control: NetDlgControl) -> bool {
        self.pointer_pressed == Some(control)
            || self.key_pressed.is_some_and(|(pressed, _)| pressed == control)
    }
}

impl NetDlgControl {
    const fn is_button(self) -> bool {
        !matches!(
            self,
            Self::GameList | Self::JoinAddress | Self::ChatInput
        )
    }
}

fn contains(rect: IntRect, point: GuiPoint) -> bool {
    point.x >= rect.x as f32
        && point.y >= rect.y as f32
        && point.x < (rect.x + rect.w) as f32
        && point.y < (rect.y + rect.h) as f32
}

fn blacken_transparent(image: &ImageData) -> ImageData {
    let pixels = image
        .pixels()
        .chunks_exact(4)
        .flat_map(|pixel| {
            if pixel[3] == 0 {
                [0, 0, 0, 0]
            } else {
                [pixel[0], pixel[1], pixel[2], pixel[3]]
            }
        })
        .collect();
    ImageData::new(image.width(), image.height(), pixels)
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
        let button_highlight = controller
            .map(|_| blacken_transparent(&assets.gui_button_highlight));

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
        draw_bar_with_border(surface, layout.game_list_caption, &assets.gui_caption, 32, gamma);
        let capt = layout.game_list_caption;
        draw_text_clipped(
            surface,
            &fonts.text,
            capt.x + 5,
            capt.y + (capt.h - fonts.text.line_height) / 2 - 1,
            "Running Games",
            CLR_YELLOW,
            TextAlign::Left,
            gamma,
            inclusive_clip(capt),
        );

        // Game list box (ListBox::DrawElement, C4GuiListBox.cpp:100-139):
        // dark background box over the inclusive bounds, then the 3D frame.
        // No selection bar, delimiters or scroll bar at t=0.
        let list = layout.game_list;
        draw_box_engine(
            surface,
            list.x,
            list.y,
            list.x + list.w - 1,
            list.y + list.h - 1,
            CLR_DARK_BG,
            gamma,
        );
        draw_3d_frame(surface, list, gamma);

        // The masterserver query entry: animated NetGetRef icon, aspect-fit
        // 40x32 -> 48x38 (Picture::DrawElement, C4GuiLabels.cpp:349-380;
        // C4Facet.cpp:100-127), then the two info labels (Label::DrawElement;
        // texts per SetRefQuery, C4StartupNetDlg.cpp:144-160).
        let phase = net_get_ref_phase(&assets.net_get_ref, get_ref_phase);
        crate::draw_image_bilinear(surface, &gui_rect(layout.entry_icon), &phase, gamma);
        let entry_texts = [
            "Internet server on league.clonkspot.org",
            "Querying game infos...",
        ];
        for (rect, text) in layout.entry_labels.iter().zip(entry_texts) {
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

        // "IP:" wooden label (C4StartupNetDlg.cpp:679-688): the 28px bar
        // exercises DrawBar's overflow quirk; ACenter text, top row clipped.
        draw_bar_with_border(surface, layout.ip_label, &assets.gui_caption, 32, gamma);
        let ip = layout.ip_label;
        draw_text_clipped(
            surface,
            &fonts.text,
            ip.x + ip.w / 2,
            ip.y + (ip.h - fonts.text.line_height) / 2 - 1,
            "IP:",
            CLR_YELLOW,
            TextAlign::Center,
            gamma,
            inclusive_clip(ip),
        );

        // Join-address edit, empty and unfocused (Edit::DrawElement,
        // C4GuiEdit.cpp:556-634): background box down to the client-rect
        // bottom (margins L4 R4 T2 B2, C4GuiEdit.h:102-105), default 3D
        // frame, no text, no caret.
        let edit = layout.join_edit;
        draw_box_engine(
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
            draw_text_clipped(
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
                draw_text_clipped(
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
            Self::draw_chat_sheet(surface, assets, fonts, gamma, layout);
        }

        // ⑥⑦ Right icon buttons, config-driven (C4StartupNetDlg.cpp:710-717)
        // on the 4-column grid: InternetOn Ex+7 (192,64) / InternetOff Ex+6
        // (128,64); RecordOn Ex+1 (64,0) / RecordOff Ex+0 (0,0).
        if mode == NetDlgMode::GameList {
            let internet_src = if config.masterserver_signup { (192, 64) } else { (128, 64) };
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
            Self::bottom_button(
                surface,
                assets,
                fonts,
                gamma,
                layout.buttons[index],
                label,
                button_highlight.as_ref(),
                controller.is_some_and(|state| state.is_highlighted(control)),
                controller.is_some_and(|state| state.is_pressed(control)),
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

    #[allow(clippy::too_many_arguments)]
    fn bottom_button(
        surface: &mut Surface,
        assets: &NetDlgAssets,
        fonts: &ClonkFontSet,
        gamma: Option<&GammaRamp>,
        rect: IntRect,
        label: &str,
        highlight_image: Option<&ImageData>,
        highlighted: bool,
        pressed: bool,
    ) {
        let plank = if pressed {
            &assets.gui_button_down
        } else {
            &assets.gui_button
        };
        draw_bar(surface, &gui_rect(rect), plank, gamma);
        if highlighted {
            if let Some(highlight) = highlight_image {
                crate::draw_image_bilinear_additive(
                    surface,
                    &GuiRect::new(
                        (rect.x + 5) as f32,
                        (rect.y + 3) as f32,
                        (rect.w - 10) as f32,
                        (rect.h - 6) as f32,
                    ),
                    highlight,
                    gamma,
                );
            }
        }
        let font = fonts.button_font(rect.h);
        let (text, _) = expand_hotkey_markup(label);
        let offset = i32::from(pressed);
        font.draw_with_gamma(
            surface,
            (rect.x + rect.x + rect.w - 1) / 2 + offset,
            (rect.y + rect.y + rect.h - 1 - font.line_height) / 2 + offset,
            &text,
            CLR_YELLOW,
            TextAlign::Center,
            true,
            gamma,
        );
    }

    fn draw_chat_sheet(
        surface: &mut Surface,
        assets: &NetDlgAssets,
        fonts: &ClonkFontSet,
        gamma: Option<&GammaRamp>,
        layout: NetDlgLayout,
    ) {
        draw_bar_with_border(
            surface,
            layout.game_list_caption,
            &assets.gui_caption,
            32,
            gamma,
        );
        let caption = layout.game_list_caption;
        draw_text_clipped(
            surface,
            &fonts.text,
            caption.x + 5,
            caption.y + (caption.h - fonts.text.line_height) / 2 - 1,
            "Chat",
            CLR_YELLOW,
            TextAlign::Left,
            gamma,
            inclusive_clip(caption),
        );
        let chat = IntRect {
            x: layout.game_list.x,
            y: layout.game_list.y,
            w: layout.game_list.w,
            h: layout.join_edit.y + layout.join_edit.h - layout.game_list.y,
        };
        draw_box_engine(
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

        assert_eq!(click(&mut controller, layout.buttons[0]), vec![NetDlgAction::Back]);
        assert_eq!(click(&mut controller, layout.buttons[1]), vec![NetDlgAction::Refresh]);

        controller.set_join_address(" 127.0.0.1:11111 ");
        assert_eq!(
            click(&mut controller, layout.buttons[2]),
            vec![NetDlgAction::JoinGame {
                address: Some("127.0.0.1:11111".into())
            }]
        );
        assert_eq!(click(&mut controller, layout.buttons[3]), vec![NetDlgAction::CreateGame]);

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

    /// Builds a `w`x`h` image whose pixel at column x is gray value `10*(x+1)`.
    fn column_coded_image(w: u32, h: u32) -> ImageData {
        let pixels = (0..h)
            .flat_map(|_| (0..w).flat_map(|x| [(10 * (x + 1)) as u8; 3].into_iter().chain([255u8])))
            .collect();
        ImageData::new(w, h, pixels)
    }

    fn column_values(surface: &Surface, y: u32, w: u32) -> Vec<u8> {
        (0..w)
            .map(|x| surface.get_pixel(x, y).map(|c| c.r).unwrap_or(0))
            .collect()
    }

    // DrawBar "exact bar" with a border narrower than implied by the texture
    // height (DynBarFacet::SetHorizontal with iBorderWidth, C4Gui.cpp:92-99,
    // 283-311): begin = left `border` columns, middle tiled until
    // `w - border/3`, end right-aligned and drawn last.
    #[test]
    fn caption_bar_tiles_middle_and_right_aligns_end() {
        // 8x1 texture, border 2: begin cols 0-1 (10,20), middle cols 2-5
        // (30,40,50,60), end cols 6-7 (70,80).
        let image = column_coded_image(8, 1);
        let mut surface = Surface::new(9, 1, PixelFormat::Rgba8888);
        let rect = IntRect { x: 0, y: 0, w: 9, h: 1 };
        draw_bar_with_border(&mut surface, rect, &image, 2, None);
        // iRightShowLength = 2/3 = 0; middle tiles at x=2 (full) and x=6
        // (cropped to 3); end drawn last right-aligned at x=7.
        assert_eq!(column_values(&surface, 0, 9), vec![10, 20, 30, 40, 50, 60, 30, 70, 80]);
    }

    // The IP-label overflow quirk (bar narrower than one border slice,
    // C4Gui.cpp:290-292,300-310): begin truncated, middle skipped, end
    // cropped from its left and overdrawing the begin at the same span.
    #[test]
    fn caption_bar_narrower_than_border_lets_end_overdraw_begin() {
        let image = column_coded_image(8, 1);
        let mut surface = Surface::new(1, 1, PixelFormat::Rgba8888);
        let rect = IntRect { x: 0, y: 0, w: 1, h: 1 };
        draw_bar_with_border(&mut surface, rect, &image, 2, None);
        // begin drew col 0 (10); end src X += 2-1 -> texture col 7 (80) wins.
        assert_eq!(column_values(&surface, 0, 1), vec![80]);
    }

    // DrawBoxDw via DrawQuadDw (StdGL.cpp:846-891): inclusive corners, engine
    // inverted alpha, `src*(1-A/255) + dst*(A/255)` blending.
    #[test]
    fn engine_box_blends_with_inverted_alpha_over_inclusive_rect() {
        let mut surface = Surface::new(3, 1, PixelFormat::Rgba8888);
        for x in 0..3 {
            let _ = surface.set_pixel(x, 0, Color::new(200, 100, 0, 255));
        }
        // 0x7f000000: black at opacity (255-127)/255 over cols 0..=1.
        draw_box_engine(&mut surface, 0, 0, 1, 0, 0x7f00_0000, None);
        // 200*(127/255) = 99.6 -> 100; 100*(127/255) = 49.8 -> 50.
        assert_eq!(column_values(&surface, 0, 3), vec![100, 100, 200]);
        assert_eq!(surface.get_pixel(0, 0).map(|c| c.g), Some(50));
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
            gui_icons_ex: load_graphics_png("GUIIcons2.png"),
        };
        let fonts = endeavour_font_set();
        let mut surface = Surface::new(1280, 720, PixelFormat::Rgba8888);
        // The capture machine had Config.General.Record enabled (the icons
        // are config-driven; the C++ default is false) — render to match the
        // reference.
        let config = NetDlgConfig { record: true, ..NetDlgConfig::default() };
        NetDlgScreen::render(&mut surface, &assets, &fonts, Some(standard_gamma()), config, 0);
        standard_gamma().apply_to_surface(&mut surface);

        // The opaque background must cover every pixel (no channel left at
        // the cleared 0 thanks to the gamma floor).
        assert!(surface.get_pixel(0, 0).map(|c| c.r >= 1).unwrap_or(false));

        let dir = std::path::Path::new("/tmp/menu-parity-net");
        std::fs::create_dir_all(dir).expect("create artifact dir");
        write_ppm(&surface, dir.join("out.ppm"));
    }
}
