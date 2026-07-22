use crate::clonk_fonts::{expand_hotkey_markup, ClonkFontSet, NativeClonkFontSet};
use crate::{draw_text, fill_rect, GuiPoint, ImageData, KeyCode};
use clonk_graphics::clonk_font::TextAlign;
use clonk_graphics::{Color, Surface, TextFont};
use clonk_gui::{ButtonTextures, Rect as GuiRect, Size as GuiSize};
use std::sync::Arc;

const TRADEMARK_TEXT: &str = "LegacyClonk is a fan project based on Clonk Rage.   \
                              'Clonk' is a registered trademark of Matthes Bender.";

/// Text attached to one classic startup-dialog tooltip target.
///
/// Resource variants retain the language-table key so the application can
/// resolve the active runtime language and preserve C++'s undefined-resource
/// behavior. Text is used for live content such as scenario and player names.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum StartupTooltip {
    Resource {
        key: &'static str,
    },
    FormattedResource {
        key: &'static str,
        arguments: Vec<String>,
    },
    Text(String),
}

impl StartupTooltip {
    pub const fn resource(key: &'static str) -> Self {
        Self::Resource { key }
    }

    pub fn formatted_resource(
        key: &'static str,
        arguments: impl IntoIterator<Item = impl Into<String>>,
    ) -> Self {
        Self::FormattedResource {
            key,
            arguments: arguments.into_iter().map(Into::into).collect(),
        }
    }

    pub fn text(text: impl Into<String>) -> Self {
        Self::Text(text.into())
    }
}

/// Integer rectangle in screen pixels (mirrors C++ `C4Rect`).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct IntRect {
    pub x: i32,
    pub y: i32,
    pub w: i32,
    pub h: i32,
}

/// Autosized `C4GUI::Label` bounds for an `ACenter` anchor. Fullscreen
/// startup titles and the scenario-book caption use this exact geometry.
pub fn centered_label_rect(anchor: (i32, i32), extent: (i32, i32)) -> IntRect {
    IntRect {
        x: anchor.0 - extent.0 / 2,
        y: anchor.1,
        w: extent.0,
        h: extent.1,
    }
}

/// Resolves an autosized centered label's own tooltip. Callers supply the
/// measured live-font extent and the text target assigned by C++.
pub fn centered_label_tooltip_at(
    point: GuiPoint,
    anchor: (i32, i32),
    extent: (i32, i32),
    tooltip: StartupTooltip,
) -> Option<StartupTooltip> {
    let rect = centered_label_rect(anchor, extent);
    (point.x >= rect.x as f32
        && point.y >= rect.y as f32
        && point.x < (rect.x + rect.w) as f32
        && point.y < (rect.y + rect.h) as f32)
        .then_some(tooltip)
}

/// Pixel-exact C4StartupMainDlg geometry, all in C++ integer math.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct MainMenuLayout {
    /// Fullscreen-dialog client rect; child elements are offset by its origin
    /// (C4GuiContainers.cpp:273-308).
    pub client: IntRect,
    /// The six big buttons, in screen coordinates.
    pub buttons: [IntRect; 6],
    /// Right-aligned anchor of the participants label (C4StartupMainDlg.cpp:69).
    pub participants_anchor: (i32, i32),
    /// Right-aligned x anchor of the trademark line (C4StartupMainDlg.cpp:72-74);
    /// its y depends on the mini font's line height.
    pub trademark_anchor_x: i32,
}

/// Computes the C4StartupMainDlg layout for a `w`x`h` screen.
///
/// Mirrors C4StartupMainDlg.cpp:42-46 (ComponentAligner stacking),
/// C4GuiDialogs.cpp:813-822,858-862 (fullscreen dialog margins) and
/// C4Gui.cpp:975-990,1041-1047 (aligner margin handling).
pub fn main_menu_layout(w: i32, h: i32) -> MainMenuLayout {
    // Fullscreen dialog margins: X = w/50 (2 below 500px), Y = h*2/75 (2 below
    // 320px); the top margin adds the 50px reserved title strip
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

    // Button column: right 2/5 of the dialog bounds, inset Wdt/26 horizontally
    // and 40 + Hgt/8 vertically; each button takes 40px (C4GUI_BigButtonHgt)
    // plus a 2px aligner margin above and below (44px pitch).
    let panel_w = w * 2 / 5;
    let inset_x = w / 26;
    let inset_y = 40 + h / 8;
    let col_x = (w - panel_w) + inset_x;
    let col_w = panel_w - 2 * inset_x;

    let button_h = 40;
    let padding = 2;
    let mut buttons = [IntRect::default(); 6];
    for (i, rect) in buttons.iter_mut().enumerate() {
        *rect = IntRect {
            x: client.x + col_x,
            y: client.y + inset_y + padding + (i as i32) * (button_h + 2 * padding),
            w: col_w,
            h: button_h,
        };
    }

    MainMenuLayout {
        client,
        buttons,
        participants_anchor: (
            client.x + client.w * 39 / 40,
            client.y + client.h * 9 / 10,
        ),
        trademark_anchor_x: client.x + client.w,
    }
}

/// Draws a horizontal three-slice bar from `image` into `rect` at native
/// (1:1) pixel scale, mirroring C4GUI::Element::DrawBar's "exact bar" branch
/// (C4Gui.cpp:283-311) with DynBarFacet slices: begin = left `border` columns,
/// middle = the remainder tiled, end = right `border` columns drawn last,
/// where `border` = texture height (C4Gui.cpp:101-107).
pub fn draw_bar(
    surface: &mut Surface,
    rect: &GuiRect,
    image: &ImageData,
    gamma: Option<&clonk_graphics::GammaRamp>,
) {
    let border = image.height();
    let mid_w = image.width().saturating_sub(2 * border);
    let (x0, y0) = (rect.origin.x as i32, rect.origin.y as i32);
    let bar_w = rect.size.width as i32;
    let end_show = (border / 3) as i32; // iRightShowLength (C4Gui.cpp:289)

    // begin slice (clipped if the bar is narrower than the slice)
    let begin_w = (border as i32).min(bar_w).max(0) as u32;
    crate::draw_image_strip(surface, x0, y0, image, 0, 0, begin_w, border, gamma);

    // middle tiles: advance by the full middle width even when clipped
    if mid_w > 0 {
        let mut ix = border as i32;
        while ix < bar_w - end_show {
            let tile_w = (mid_w as i32).min(bar_w - end_show - ix).max(0) as u32;
            crate::draw_image_strip(surface, x0 + ix, y0, image, border, 0, tile_w, border, gamma);
            ix += mid_w as i32;
        }
    }

    // end slice, right-aligned, drawn last (overdraws middle tiles)
    let end_w = (border as i32).min(bar_w).max(0) as u32;
    let end_src_x = image.width() - border + (border - end_w);
    crate::draw_image_strip(
        surface,
        x0 + bar_w - end_w as i32,
        y0,
        image,
        end_src_x,
        0,
        end_w,
        border,
        gamma,
    );
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MainMenuItem {
    LocalGame,
    NetworkGame,
    PlayerSelection,
    Options,
    About,
    Quit,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum MainMenuAction {
    SelectionChanged(MainMenuItem),
    Activate(MainMenuItem),
}

#[derive(Clone)]
pub struct StartupMainMenu {
    font: Arc<dyn TextFont>,
    /// CStdFont-faithful fonts; when set, all menu text renders through them
    /// for pixel parity with the C++ engine.
    clonk_fonts: Option<Arc<ClonkFontSet>>,
    textures: Option<ButtonTextures>,
    /// GUIButtonHighlight, blitted additively over focused/hovered buttons
    /// (C4GuiButton.cpp:94-98).
    highlight: Option<ImageData>,
    /// Per-fragment gamma ramp of the C++ blit shader.
    gamma: Option<Arc<clonk_graphics::GammaRamp>>,
    buttons: Vec<MenuButton>,
    pointer_position: Option<GuiPoint>,
    pressed_index: Option<usize>,
    /// Focused button armed by Enter/Space. C++ sets `Button::fDown` on key
    /// down and invokes the callback only on the matching key up
    /// (C4GuiButton.cpp:112-127).
    key_pressed: Option<(usize, KeyCode)>,
    /// Keyboard focus; the C++ dialog focuses the start button on first show
    /// (C4StartupMainDlg.cpp:305-310).
    selected_index: Option<usize>,
    /// Button under the mouse; highlights like focus but doesn't move it.
    hover_index: Option<usize>,
    layout: Vec<GuiRect>,
    size: GuiSize,
}

#[derive(Clone)]
struct MenuButton {
    label: &'static str,
    item: MainMenuItem,
    enabled: bool,
}

impl StartupMainMenu {
    pub fn new(font: Arc<dyn TextFont>, textures: Option<ButtonTextures>) -> Self {
        let buttons = vec![
            // Labels match the C++ main menu, including the `&` hotkey markers
            // (IDS_BTN_LOCALGAME etc. in System.c4g/LanguageUS.txt:24-537).
            MenuButton::new("&Start Game", MainMenuItem::LocalGame),
            MenuButton::new("Start &Network Game", MainMenuItem::NetworkGame),
            MenuButton::new("&Player Selection", MainMenuItem::PlayerSelection),
            MenuButton::new("&Options", MainMenuItem::Options),
            MenuButton::new("&About", MainMenuItem::About),
            MenuButton::new("E&xit", MainMenuItem::Quit),
        ];
        Self {
            font,
            clonk_fonts: None,
            textures,
            highlight: None,
            gamma: None,
            buttons,
            pointer_position: None,
            pressed_index: None,
            key_pressed: None,
            selected_index: Some(0),
            hover_index: None,
            layout: Vec::new(),
            size: GuiSize::new(0.0, 0.0),
        }
    }

    /// Sets the GUIButtonHighlight texture for the focus/hover overlay.
    pub fn set_highlight_texture(&mut self, highlight: Option<ImageData>) {
        self.highlight = highlight;
    }

    /// Sets the CStdFont-faithful font set used for pixel-parity text.
    pub fn set_clonk_fonts(&mut self, fonts: Option<Arc<ClonkFontSet>>) {
        self.clonk_fonts = fonts;
    }

    /// Sets the gamma ramp the highlight overlay is encoded through.
    pub fn set_gamma_ramp(&mut self, gamma: Option<Arc<clonk_graphics::GammaRamp>>) {
        self.gamma = gamma;
    }

    pub fn resize(&mut self, width: f32, height: f32) {
        self.size = GuiSize::new(width.max(1.0), height.max(1.0));
        self.layout = self.compute_layout();
    }

    pub fn pointer_position(&self) -> Option<GuiPoint> {
        self.pointer_position
    }

    /// Autosized bounds of the right-aligned participants label. C++ attaches
    /// the Add/Remove context handler to this exact Label element.
    pub fn participants_rect(&self, participants_label: &str) -> IntRect {
        let layout = main_menu_layout(
            self.size.width.max(1.0) as i32,
            self.size.height.max(1.0) as i32,
        );
        let (expanded_label, _) = expand_hotkey_markup(participants_label);
        let (width, height) = self.clonk_fonts.as_ref().map_or_else(
            || {
                let metrics = self
                    .font
                    .measure_text(&participants_label.replacen('&', "", 1), 22.0);
                (metrics.width.round() as i32, 34)
            },
            |fonts| fonts.title.measure(&expanded_label, true),
        );
        IntRect {
            x: layout.participants_anchor.0 - width,
            y: layout.participants_anchor.1,
            w: width,
            h: height,
        }
    }

    pub fn participants_contains(&self, participants_label: &str, point: GuiPoint) -> bool {
        let rect = self.participants_rect(participants_label);
        point.x >= rect.x as f32
            && point.y >= rect.y as f32
            && point.x < (rect.x + rect.w) as f32
            && point.y < (rect.y + rect.h) as f32
    }

    fn trademark_contains(&self, point: GuiPoint) -> bool {
        let layout = main_menu_layout(
            self.size.width.max(1.0) as i32,
            self.size.height.max(1.0) as i32,
        );
        let (width, line_height) = self.clonk_fonts.as_ref().map_or_else(
            || {
                (
                    self.font.measure_text(TRADEMARK_TEXT, 12.0).width.round() as i32,
                    18,
                )
            },
            |fonts| {
                (
                    fonts.mini.measure(TRADEMARK_TEXT, false).0,
                    fonts.mini.line_height,
                )
            },
        );
        let rect = IntRect {
            x: layout.trademark_anchor_x - width,
            y: layout.client.y + layout.client.h - line_height / 2,
            w: width,
            h: line_height,
        };
        point.x >= rect.x as f32
            && point.y >= rect.y as f32
            && point.x < (rect.x + rect.w) as f32
            && point.y < (rect.y + rect.h) as f32
    }

    /// Returns the native tooltip target at `point`, without applying the
    /// screen-wide CMouse delay.
    pub fn tooltip_at(
        &self,
        participants_label: &str,
        point: GuiPoint,
    ) -> Option<StartupTooltip> {
        if self.trademark_contains(point) {
            return None;
        }
        if self.participants_contains(participants_label, point) {
            return Some(StartupTooltip::resource("IDS_DLGTIP_SELECTEDPLAYERS"));
        }
        let item = self.buttons.get(self.hit_test(point)?)?.item;
        Some(StartupTooltip::resource(match item {
            MainMenuItem::LocalGame => "IDS_DLGTIP_STARTGAME",
            MainMenuItem::NetworkGame => "IDS_DLGTIP_NETWORKGAME",
            MainMenuItem::PlayerSelection => "IDS_DLGTIP_PLAYERSELECTION",
            MainMenuItem::Options => "IDS_DLGTIP_OPTIONS",
            MainMenuItem::About => "IDS_DLGTIP_ABOUT",
            MainMenuItem::Quit => "IDS_DLGTIP_EXIT",
        }))
    }

    pub fn tooltip(&self, participants_label: &str) -> Option<StartupTooltip> {
        self.tooltip_at(participants_label, self.pointer_position?)
    }

    pub fn set_pointer_position(&mut self, position: Option<GuiPoint>) {
        self.pointer_position = position;
        self.hover_index = position.and_then(|point| self.hit_test(point));
    }

    pub fn pointer_left(&mut self) {
        self.pointer_position = None;
        self.pressed_index = None;
        self.key_pressed = None;
        self.hover_index = None;
    }

    pub fn set_item_enabled(&mut self, item: MainMenuItem, enabled: bool) {
        if let Some(button) = self.buttons.iter_mut().find(|button| button.item == item) {
            button.enabled = enabled;
            if !enabled {
                if let Some(selected) = self.selected_index {
                    if self.buttons[selected].item == item {
                        self.selected_index = None;
                    }
                }
                if let Some(pressed) = self.pressed_index {
                    if self.buttons[pressed].item == item {
                        self.pressed_index = None;
                    }
                }
                if let Some((pressed, _)) = self.key_pressed {
                    if self.buttons[pressed].item == item {
                        self.key_pressed = None;
                    }
                }
            }
        }
    }

    /// Dispatches a dialog mnemonic through the visible button order.
    ///
    /// C++ expands each button caption's first `&` marker when the text is
    /// assigned, then `Container::OnHotkey` visits the buttons in insertion
    /// order and skips disabled matches (`C4GuiContainers.cpp:194-202`,
    /// `C4GuiButton.cpp:55-78`). `Some` means the key was consumed.
    pub fn handle_hotkey(&mut self, character: char) -> Option<Vec<MainMenuAction>> {
        let character = character.to_ascii_uppercase();
        if !character.is_ascii_alphanumeric() {
            return None;
        }
        self.buttons
            .iter()
            .find(|button| {
                button.enabled && expand_hotkey_markup(button.label).1 == Some(character)
            })
            .map(|button| vec![MainMenuAction::Activate(button.item)])
    }

    pub fn handle_pointer_move(&mut self, position: GuiPoint) -> Vec<MainMenuAction> {
        self.pointer_position = Some(position);
        // The mouse only hovers; it does not move the keyboard focus
        // (C++ Button::MouseEnter sets fMouseOver, C4GuiButton.cpp:160-181).
        let mut actions = Vec::new();
        let hover = self.hit_test(position);
        if hover != self.hover_index {
            self.hover_index = hover;
            if let Some(index) = hover {
                actions.push(MainMenuAction::SelectionChanged(self.buttons[index].item));
            }
        }
        actions
    }

    pub fn handle_pointer_down(&mut self, position: GuiPoint) -> Vec<MainMenuAction> {
        self.pointer_position = Some(position);
        self.hover_index = self.hit_test(position);
        self.key_pressed = None;
        if let Some(index) = self.hit_test(position) {
            if self.buttons[index].enabled {
                self.pressed_index = Some(index);
            }
        }
        Vec::new()
    }

    pub fn handle_pointer_up(&mut self, position: GuiPoint) -> Vec<MainMenuAction> {
        self.pointer_position = Some(position);
        self.hover_index = self.hit_test(position);
        let mut actions = Vec::new();
        let pressed = self.pressed_index.take();
        let Some(pressed_index) = pressed else {
            return actions;
        };
        if !self.buttons[pressed_index].enabled {
            return actions;
        }
        if let Some(index) = self.hit_test(position) {
            if index == pressed_index {
                actions.push(MainMenuAction::Activate(self.buttons[index].item));
            }
        }
        actions
    }

    pub fn handle_key_down(&mut self, key: KeyCode) -> Vec<MainMenuAction> {
        match key {
            KeyCode::Up | KeyCode::Left => {
                self.key_pressed = None;
                self.move_selection(-1)
            }
            KeyCode::Down | KeyCode::Right => {
                self.key_pressed = None;
                self.move_selection(1)
            }
            KeyCode::Enter | KeyCode::Space => {
                self.pressed_index = None;
                if let Some(index) = self.selected_index {
                    if self.buttons[index].enabled {
                        self.key_pressed = Some((index, key));
                    }
                }
                Vec::new()
            }
            _ => Vec::new(),
        }
    }

    pub fn handle_key_up(&mut self, key: KeyCode) -> Vec<MainMenuAction> {
        let Some((pressed_index, pressed_key)) = self.key_pressed else {
            return Vec::new();
        };
        if key != pressed_key {
            return Vec::new();
        }
        self.key_pressed = None;
        if self.selected_index == Some(pressed_index) && self.buttons[pressed_index].enabled {
            vec![MainMenuAction::Activate(self.buttons[pressed_index].item)]
        } else {
            Vec::new()
        }
    }

    pub fn render(&mut self, surface: &mut Surface, participants_label: &str) {
        self.render_with_draw_focus(surface, participants_label, true);
    }

    pub fn render_with_draw_focus(
        &mut self,
        surface: &mut Surface,
        participants_label: &str,
        draw_focus: bool,
    ) {
        self.render_base(surface, participants_label, true, draw_focus);
    }

    /// Draw the filterable startup background layer without any text. C++
    /// rasterizes fonts at `Application.GetScale()` and the GL viewport maps
    /// their logical coordinates to physical pixels, so captions cannot be
    /// part of Rust's bilinear scale-1 base frame (`C4Fonts.cpp:158-173`;
    /// `StdFont.cpp:319-352,841-842`).
    pub fn render_chrome(&mut self, surface: &mut Surface) {
        self.render_base(surface, "", false, true);
    }

    fn render_base(
        &mut self,
        surface: &mut Surface,
        participants_label: &str,
        include_text: bool,
        draw_focus: bool,
    ) {
        if self.layout.is_empty() {
            self.layout = self.compute_layout();
        }

        // C++ C4StartupMainDlg draws the buttons directly over the loader
        // background — there is no panel backdrop or footer box.
        let pressed_index = self
            .pressed_index
            .or_else(|| self.key_pressed.map(|(index, _)| index));
        for (index, rect) in self.layout.iter().enumerate() {
            let state = ButtonVisualState::from_indices(
                index,
                if draw_focus { self.selected_index } else { None },
                pressed_index,
                self.buttons[index].enabled,
            );
            let highlighted = self.buttons[index].enabled
                && ((draw_focus && self.selected_index == Some(index))
                    || self.hover_index == Some(index));
            self.draw_button(
                surface,
                rect,
                &self.buttons[index],
                state,
                highlighted,
                include_text,
            );
        }

        if !include_text {
            return;
        }

        let layout = main_menu_layout(
            self.size.width.max(1.0) as i32,
            self.size.height.max(1.0) as i32,
        );
        let white = Color::new(255, 255, 255, 255);

        // Participants label: white TitleFont (22px), right-aligned at
        // client*(39/40, 9/10) (C4StartupMainDlg.cpp:69-70).
        // Trademark line: white MiniFont (12px), right-aligned at the client
        // rect's right edge, half a line above its bottom
        // (C4StartupMainDlg.cpp:72-74; FANPROJECTTEXT/TRADEMARKTEXT,
        // C4Version.h:21-22).
        let trademark = TRADEMARK_TEXT;
        let (anchor_x, anchor_y) = layout.participants_anchor;
        if let Some(fonts) = self.clonk_fonts.as_ref() {
            let (expanded_label, _) = expand_hotkey_markup(participants_label);
            fonts.title.draw_with_gamma(
                surface,
                anchor_x,
                anchor_y,
                &expanded_label,
                [255, 255, 255, 255],
                TextAlign::Right,
                true,
                self.gamma.as_deref(),
            );
            fonts.mini.draw_with_gamma(
                surface,
                layout.trademark_anchor_x,
                layout.client.y + layout.client.h - fonts.mini.line_height / 2,
                trademark,
                [255, 255, 255, 255],
                TextAlign::Right,
                false,
                self.gamma.as_deref(),
            );
            return;
        }

        let font_size = 22.0;
        let participants_label = participants_label.replacen('&', "", 1);
        let metrics = self.font.measure_text(&participants_label, font_size);
        let label_rect = GuiRect::new(
            (anchor_x as f32 - metrics.width).max(0.0),
            anchor_y as f32,
            metrics.width,
            font_size,
        );
        draw_text(
            surface,
            &label_rect,
            &participants_label,
            white,
            font_size,
            0.0,
            self.font.as_ref(),
        );

        let mini_size = 12.0;
        let mini_line_height = 18; // Endeavour 12px: (1303+308)*12/1024 (StdFont.cpp:351)
        let metrics = self.font.measure_text(trademark, mini_size);
        let label_rect = GuiRect::new(
            (layout.trademark_anchor_x as f32 - metrics.width).max(0.0),
            (layout.client.y + layout.client.h - mini_line_height / 2) as f32,
            metrics.width,
            mini_size,
        );
        draw_text(
            surface,
            &label_rect,
            trademark,
            white,
            mini_size,
            0.0,
            self.font.as_ref(),
        );
    }

    /// Draw the main-menu captions from a scale-native CStdFont atlas directly
    /// into the physical output buffer. Coordinates and layout stay in GUI
    /// units; [`NativeClonkFontSet`] performs the C++ scale conversion.
    pub fn render_native_text(
        &self,
        surface: &mut Surface,
        fonts: &NativeClonkFontSet,
        participants_label: &str,
        gamma: Option<&clonk_graphics::GammaRamp>,
    ) {
        self.render_native_text_with_offset(surface, fonts, participants_label, (0, 0), gamma);
    }

    /// Native caption pass with the physical offset of C++'s GL viewport.
    pub fn render_native_text_with_offset(
        &self,
        surface: &mut Surface,
        fonts: &NativeClonkFontSet,
        participants_label: &str,
        physical_offset: (i32, i32),
        gamma: Option<&clonk_graphics::GammaRamp>,
    ) {
        let layout = main_menu_layout(
            self.size.width.max(1.0) as i32,
            self.size.height.max(1.0) as i32,
        );
        let pressed_index = self
            .pressed_index
            .or_else(|| self.key_pressed.map(|(index, _)| index));
        for (index, rect) in layout.buttons.iter().enumerate() {
            let button = &self.buttons[index];
            let pressed = pressed_index == Some(index) && button.enabled;
            let offset = i32::from(pressed);
            let font = fonts.button_font(rect.h);
            let (expanded, _) = expand_hotkey_markup(button.label);
            let color = if button.enabled {
                [0xff, 0xff, 0x00, 0xff]
            } else {
                [0xaf, 0xaf, 0xaf, 0xff]
            };
            let x1 = rect.x + rect.w - 1;
            let y1 = rect.y + rect.h - 1;
            font.draw_to_physical_surface_with_offset(
                surface,
                (rect.x + x1) / 2 + offset,
                (rect.y + y1 - font.logical_line_height()) / 2 + offset,
                &expanded,
                color,
                TextAlign::Center,
                true,
                physical_offset,
                gamma,
            );
        }

        let (anchor_x, anchor_y) = layout.participants_anchor;
        let (expanded_label, _) = expand_hotkey_markup(participants_label);
        fonts.title.draw_to_physical_surface_with_offset(
            surface,
            anchor_x,
            anchor_y,
            &expanded_label,
            [255, 255, 255, 255],
            TextAlign::Right,
            true,
            physical_offset,
            gamma,
        );
        let trademark = TRADEMARK_TEXT;
        fonts.mini.draw_to_physical_surface_with_offset(
            surface,
            layout.trademark_anchor_x,
            layout.client.y + layout.client.h - fonts.mini.logical_line_height() / 2,
            trademark,
            [255, 255, 255, 255],
            TextAlign::Right,
            false,
            physical_offset,
            gamma,
        );
    }

    fn move_selection(&mut self, delta: isize) -> Vec<MainMenuAction> {
        if self.buttons.is_empty() {
            return Vec::new();
        }
        let mut index = self.selected_index.unwrap_or(0);
        let len = self.buttons.len();
        let mut attempts = 0usize;
        loop {
            let raw = index as isize + delta;
            if raw < 0 {
                index = len.saturating_sub(1);
            } else {
                index = (raw as usize) % len;
            }
            attempts += 1;
            if self.buttons[index].enabled || attempts >= len {
                break;
            }
        }
        self.selected_index = Some(index);
        vec![MainMenuAction::SelectionChanged(self.buttons[index].item)]
    }

    fn draw_button(
        &self,
        surface: &mut Surface,
        rect: &GuiRect,
        button: &MenuButton,
        state: ButtonVisualState,
        highlighted: bool,
        include_text: bool,
    ) {
        // Plank: 3-slice bar of StartupBigButton(Down) at native scale
        // (Button::DrawElement, C4GuiButton.cpp:81-89). The down state swaps
        // the texture; disabled/selected do NOT change the plank in C++.
        let pressed = state == ButtonVisualState::Pressed;
        if let Some(textures) = self.textures.as_ref() {
            let image = if pressed {
                &textures.pressed
            } else {
                &textures.normal
            };
            draw_bar(surface, rect, image, self.gamma.as_deref());
        } else {
            let color = match state {
                ButtonVisualState::Disabled => Color::new(50, 60, 72, 220),
                ButtonVisualState::Pressed => Color::new(44, 70, 120, 240),
                ButtonVisualState::Selected => Color::new(54, 90, 160, 240),
                ButtonVisualState::Normal => Color::new(36, 62, 104, 230),
            };
            fill_rect(surface, rect, color);
        }

        // Focus/hover highlight: additive blit of GUIButtonHighlight stretched
        // into (x0+5, y0+3, w-10, h-6) (C4GuiButton.cpp:94-98).
        if highlighted {
            if let Some(highlight) = self.highlight.as_ref() {
                let overlay = GuiRect::new(
                    rect.origin.x + 5.0,
                    rect.origin.y + 3.0,
                    rect.size.width - 10.0,
                    rect.size.height - 6.0,
                );
                crate::draw_image_bilinear_additive(surface, &overlay, highlight, self.gamma.as_deref());
            }
        }

        if !include_text {
            return;
        }

        // C++ C4GUI button captions use C4GUI_ButtonFontClr = 0xffffff00 (yellow)
        // when active and C4GUI_InactCaptionFontClr = 0xffafafaf when disabled
        // (src/C4Gui.h:53-56, drawn at C4GuiButton.cpp:109).
        let text_color = if button.enabled {
            Color::new(0xff, 0xff, 0x00, 0xff)
        } else {
            Color::new(0xaf, 0xaf, 0xaf, 0xff)
        };
        // Caption centred at ((x0+x1)/2, (y0+y1-textHgt)/2), shifted +1,+1 when
        // pressed (C4GuiButton.cpp:90-109).
        let txt_off: i32 = if pressed { 1 } else { 0 };
        if let Some(fonts) = self.clonk_fonts.as_ref() {
            let font = fonts.button_font(rect.size.height as i32);
            let (x0, y0) = (rect.origin.x as i32, rect.origin.y as i32);
            let x1 = x0 + rect.size.width as i32 - 1;
            let y1 = y0 + rect.size.height as i32 - 1;
            let (expanded, _) = expand_hotkey_markup(button.label);
            font.draw_with_gamma(
                surface,
                (x0 + x1) / 2 + txt_off,
                (y0 + y1 - font.line_height) / 2 + txt_off,
                &expanded,
                [text_color.r, text_color.g, text_color.b, text_color.a],
                TextAlign::Center,
                true,
                self.gamma.as_deref(),
            );
            return;
        }
        let label = button.label.replace('&', "");
        let font_size = (rect.size.height * 0.48).clamp(16.0, 32.0);
        let metrics = self.font.measure_text(&label, font_size);
        let text_rect = GuiRect::new(
            rect.origin.x + ((rect.size.width - metrics.width) * 0.5).max(0.0) + txt_off as f32,
            rect.origin.y + ((rect.size.height - font_size) * 0.5).max(0.0) + txt_off as f32,
            metrics.width,
            font_size,
        );
        draw_text(
            surface,
            &text_rect,
            &label,
            text_color,
            font_size,
            0.0,
            self.font.as_ref(),
        );
    }

    fn hit_test(&self, point: GuiPoint) -> Option<usize> {
        self.layout.iter().position(|rect| {
            point.x >= rect.origin.x
                && point.y >= rect.origin.y
                && point.x < rect.origin.x + rect.size.width
                && point.y < rect.origin.y + rect.size.height
        })
    }

    fn compute_layout(&self) -> Vec<GuiRect> {
        if self.buttons.is_empty() {
            return Vec::new();
        }

        let layout = main_menu_layout(self.size.width.max(1.0) as i32, self.size.height.max(1.0) as i32);
        layout
            .buttons
            .iter()
            .map(|r| GuiRect::new(r.x as f32, r.y as f32, r.w as f32, r.h as f32))
            .collect()
    }
}

impl MenuButton {
    const fn new(label: &'static str, item: MainMenuItem) -> Self {
        Self {
            label,
            item,
            enabled: true,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum ButtonVisualState {
    Normal,
    Selected,
    Pressed,
    Disabled,
}

impl ButtonVisualState {
    fn from_indices(
        index: usize,
        selected_index: Option<usize>,
        pressed_index: Option<usize>,
        enabled: bool,
    ) -> Self {
        if !enabled {
            return Self::Disabled;
        }
        if pressed_index == Some(index) {
            return Self::Pressed;
        }
        if selected_index == Some(index) {
            return Self::Selected;
        }
        Self::Normal
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_support::endeavour_font_set;
    use clonk_graphics::{BitmapFont, PixelFormat};

    fn main_menu() -> StartupMainMenu {
        let mut menu = StartupMainMenu::new(Arc::new(BitmapFont::new()), None);
        menu.resize(1280.0, 720.0);
        menu
    }

    fn button_center(index: usize) -> GuiPoint {
        let rect = main_menu_layout(1280, 720).buttons[index];
        GuiPoint::new((rect.x + rect.w / 2) as f32, (rect.y + rect.h / 2) as f32)
    }

    /// Builds a `w`x`h` image whose pixel at column x is gray value `10*(x+1)`.
    fn column_coded_image(w: u32, h: u32) -> crate::ImageData {
        let pixels = (0..h)
            .flat_map(|_| (0..w).flat_map(|x| [(10 * (x + 1)) as u8; 3].into_iter().chain([255u8])))
            .collect();
        crate::ImageData::new(w, h, pixels)
    }

    fn column_values(surface: &clonk_graphics::Surface, y: u32, w: u32) -> Vec<u8> {
        (0..w)
            .map(|x| surface.get_pixel(x, y).map(|c| c.r).unwrap_or(0))
            .collect()
    }

    // C4GUI::Element::DrawBar "exact bar" (C4Gui.cpp:283-311): begin slice 1:1,
    // middle tiled until `w - end_w/3`, end slice right-aligned drawn last.
    #[test]
    fn draw_bar_three_slices_tiles_and_right_aligns_end() {
        // 6x2 texture, border = height = 2: begin = cols 0-1 (10,20),
        // middle = cols 2-3 (30,40), end = cols 4-5 (50,60).
        let image = column_coded_image(6, 2);
        let mut surface = clonk_graphics::Surface::new(7, 2, PixelFormat::Rgba8888);
        draw_bar(&mut surface, &GuiRect::new(0.0, 0.0, 7.0, 2.0), &image, None);
        // iRightShowLength = 2/3 = 0; tiles at x=2 (30,40), x=4 (30,40), x=6 (30);
        // end drawn last right-aligned at x=5 -> overwrites cols 5,6 with 50,60.
        assert_eq!(column_values(&surface, 0, 7), vec![10, 20, 30, 40, 30, 50, 60]);
    }

    // Pixel-exact C4StartupMainDlg geometry at 1280x720, derived from
    // C4StartupMainDlg.cpp:42-46 (ComponentAligner math),
    // C4GuiDialogs.cpp:813-822,858-862 (fullscreen dialog margins) and
    // C4GuiContainers.cpp:301-308 (client rect = children offset), verified
    // against an F9 screenshot of the C++ engine at 1280x720.
    #[test]
    fn layout_matches_cpp_main_dlg_at_1280x720() {
        let layout = main_menu_layout(1280, 720);

        // Fullscreen dialog client rect: margins x=1280/50=25, top=50+720*2/75=69.
        assert_eq!(
            (
                layout.client.x,
                layout.client.y,
                layout.client.w,
                layout.client.h
            ),
            (25, 69, 1230, 632)
        );

        // Buttons: right 2/5 panel inset by Wdt/26 and 40+Hgt/8, stacked at
        // 40px height on a 44px pitch starting +2, offset by the client origin.
        for (i, rect) in layout.buttons.iter().enumerate() {
            assert_eq!(
                (rect.x, rect.y, rect.w, rect.h),
                (842, 201 + 44 * (i as i32), 414, 40),
                "button {i}"
            );
        }

        // Participants label anchor (right-aligned): client*(39/40, 9/10) + origin.
        assert_eq!(layout.participants_anchor, (1224, 637));

        // Trademark label anchor: right edge of the client rect.
        assert_eq!(layout.trademark_anchor_x, 1255);
        assert_eq!(layout.client.y + layout.client.h, 701);
    }

    #[test]
    fn participants_context_hit_uses_the_autosized_title_label() {
        let fonts = endeavour_font_set();
        let mut menu = main_menu();
        menu.set_clonk_fonts(Some(Arc::clone(&fonts)));
        let label = "Players: Ada, Bob";
        let rect = menu.participants_rect(label);
        let (expanded, _) = expand_hotkey_markup(label);
        let (width, height) = fonts.title.measure(&expanded, true);
        assert_eq!(
            rect,
            IntRect {
                x: 1224 - width,
                y: 637,
                w: width,
                h: height,
            }
        );
        assert!(menu.participants_contains(
            label,
            GuiPoint::new(rect.x as f32, rect.y as f32),
        ));
        assert!(!menu.participants_contains(
            label,
            GuiPoint::new((rect.x + rect.w) as f32, rect.y as f32),
        ));

        let marked = "Players: R&alf";
        let marked_rect = menu.participants_rect(marked);
        let (expanded, hotkey) = expand_hotkey_markup(marked);
        let (width, height) = fonts.title.measure(&expanded, true);
        assert_eq!(hotkey, Some('A'));
        assert_eq!((marked_rect.w, marked_rect.h), (width, height));
    }

    #[test]
    fn tooltip_targets_cover_every_native_main_control() {
        let menu = main_menu();
        let label = "Players: Ada";
        let expected = [
            "IDS_DLGTIP_STARTGAME",
            "IDS_DLGTIP_NETWORKGAME",
            "IDS_DLGTIP_PLAYERSELECTION",
            "IDS_DLGTIP_OPTIONS",
            "IDS_DLGTIP_ABOUT",
            "IDS_DLGTIP_EXIT",
        ];
        for (index, key) in expected.into_iter().enumerate() {
            assert_eq!(
                menu.tooltip_at(label, button_center(index)),
                Some(StartupTooltip::resource(key))
            );
        }

        let participants = menu.participants_rect(label);
        assert_eq!(
            menu.tooltip_at(
                label,
                GuiPoint::new(participants.x as f32, participants.y as f32)
            ),
            Some(StartupTooltip::resource("IDS_DLGTIP_SELECTEDPLAYERS"))
        );
        assert_eq!(menu.tooltip_at(label, GuiPoint::new(0.0, 0.0)), None);
    }

    #[test]
    fn later_trademark_label_occludes_participants_tooltip_overlap() {
        let fonts = endeavour_font_set();
        let mut menu = StartupMainMenu::new(Arc::new(BitmapFont::new()), None);
        menu.set_clonk_fonts(Some(Arc::clone(&fonts)));
        menu.resize(640.0, 480.0);
        let label = "Players: Ada";
        let participants = menu.participants_rect(label);
        let layout = main_menu_layout(640, 480);
        let trademark_y = layout.client.y + layout.client.h - fonts.mini.line_height / 2;
        let point = GuiPoint::new(
            (participants.x + participants.w - 1) as f32,
            trademark_y.max(participants.y) as f32,
        );
        assert!(menu.participants_contains(label, point));
        assert!(menu.trademark_contains(point));
        assert_eq!(menu.tooltip_at(label, point), None);
    }

    #[test]
    fn centered_title_tooltip_uses_autosized_label_bounds() {
        let target = StartupTooltip::text("&Player Selection");
        assert_eq!(
            centered_label_rect((100, 8), (21, 34)),
            IntRect {
                x: 90,
                y: 8,
                w: 21,
                h: 34,
            }
        );
        assert_eq!(
            centered_label_tooltip_at(
                GuiPoint::new(90.0, 8.0),
                (100, 8),
                (21, 34),
                target.clone(),
            ),
            Some(target)
        );
        assert_eq!(
            centered_label_tooltip_at(
                GuiPoint::new(111.0, 8.0),
                (100, 8),
                (21, 34),
                StartupTooltip::text("outside"),
            ),
            None
        );
    }

    #[test]
    fn context_focus_suppression_keeps_pointer_hover_only() {
        let mut menu = main_menu();
        menu.set_highlight_texture(Some(ImageData::new(
            2,
            2,
            vec![0xff; 2 * 2 * 4],
        )));
        assert_eq!(menu.selected_index, Some(0));
        assert_eq!(menu.hover_index, None);

        let mut focused = menu.clone();
        let mut suppressed = menu.clone();
        let mut focused_surface = Surface::new(1280, 720, PixelFormat::Rgba8888);
        let mut suppressed_surface = Surface::new(1280, 720, PixelFormat::Rgba8888);
        focused.render_with_draw_focus(&mut focused_surface, "Players: none selected", true);
        suppressed.render_with_draw_focus(
            &mut suppressed_surface,
            "Players: none selected",
            false,
        );
        assert!(focused_surface.pixels() != suppressed_surface.pixels());

        menu.handle_pointer_move(button_center(1));
        assert_eq!(menu.selected_index, Some(0), "logical focus is retained");
        assert_eq!(menu.hover_index, Some(1), "pointer hover remains live");

        let focused = ButtonVisualState::from_indices(0, menu.selected_index, None, true);
        let suppressed = ButtonVisualState::from_indices(0, None, None, true);
        assert_eq!(focused, ButtonVisualState::Selected);
        assert_eq!(suppressed, ButtonVisualState::Normal);
        assert!(menu.hover_index == Some(1));

        let mut hovered_surface = Surface::new(1280, 720, PixelFormat::Rgba8888);
        menu.render_with_draw_focus(&mut hovered_surface, "Players: none selected", false);
        assert!(hovered_surface.pixels() != suppressed_surface.pixels());
    }

    #[test]
    fn keyboard_activation_arms_on_down_and_fires_on_up_at_every_focus_position() {
        // Button key-down only sets fDown; matching key-up invokes OnPress
        // (C4GuiButton.cpp:112-127). Main-dialog arrows move focus and Enter is
        // re-sent as Space down/up (C4StartupMainDlg.cpp:80-97,245-255).
        let items = [
            MainMenuItem::LocalGame,
            MainMenuItem::NetworkGame,
            MainMenuItem::PlayerSelection,
            MainMenuItem::Options,
            MainMenuItem::About,
            MainMenuItem::Quit,
        ];

        for (index, item) in items.into_iter().enumerate() {
            for key in [KeyCode::Enter, KeyCode::Space] {
                let mut menu = main_menu();
                for _ in 0..index {
                    let _ = menu.handle_key_down(KeyCode::Down);
                }

                assert!(
                    menu.handle_key_down(key).is_empty(),
                    "{item:?} {key:?} down"
                );
                assert_eq!(menu.key_pressed, Some((index, key)));
                assert_eq!(
                    menu.handle_key_up(key),
                    vec![MainMenuAction::Activate(item)],
                    "{item:?} {key:?} up"
                );
                assert_eq!(menu.key_pressed, None);
            }
        }
    }

    #[test]
    fn keyboard_activation_is_cancelled_when_focus_moves_or_input_leaves() {
        let mut menu = main_menu();
        assert!(menu.handle_key_down(KeyCode::Space).is_empty());
        assert_eq!(
            menu.handle_key_down(KeyCode::Down),
            vec![MainMenuAction::SelectionChanged(MainMenuItem::NetworkGame)]
        );
        assert_eq!(menu.key_pressed, None);
        assert!(menu.handle_key_up(KeyCode::Space).is_empty());

        let mut menu = main_menu();
        assert!(menu.handle_key_down(KeyCode::Enter).is_empty());
        menu.pointer_left();
        assert!(menu.handle_key_up(KeyCode::Enter).is_empty());
    }

    #[test]
    fn l046_main_menu_mnemonics_follow_caption_markers_and_enabled_state() {
        let cases = [
            ('S', MainMenuItem::LocalGame),
            ('N', MainMenuItem::NetworkGame),
            ('P', MainMenuItem::PlayerSelection),
            ('O', MainMenuItem::Options),
            ('A', MainMenuItem::About),
            ('X', MainMenuItem::Quit),
        ];
        for (character, item) in cases {
            let mut menu = main_menu();
            assert_eq!(
                menu.handle_hotkey(character.to_ascii_lowercase()),
                Some(vec![MainMenuAction::Activate(item)]),
                "{character} must dispatch the marked caption"
            );
        }

        let mut menu = main_menu();
        menu.set_item_enabled(MainMenuItem::LocalGame, false);
        assert_eq!(menu.handle_hotkey('S'), None);
        assert_eq!(menu.handle_hotkey('-'), None);
        assert_eq!(menu.handle_hotkey('Q'), None);
    }

    #[test]
    fn pointer_activation_still_requires_release_on_the_pressed_button() {
        let mut menu = main_menu();
        let network = button_center(1);
        let player = button_center(2);

        assert!(menu.handle_pointer_down(network).is_empty());
        assert_eq!(menu.pointer_position(), Some(network));
        assert_eq!(menu.hover_index, Some(1));
        assert!(menu.handle_pointer_up(player).is_empty());
        assert_eq!(menu.pointer_position(), Some(player));
        assert_eq!(menu.hover_index, Some(2));
        assert!(menu.handle_pointer_down(network).is_empty());
        assert_eq!(
            menu.handle_pointer_up(network),
            vec![MainMenuAction::Activate(MainMenuItem::NetworkGame)]
        );
    }

    #[test]
    fn changing_input_source_cancels_the_previous_press() {
        let mut menu = main_menu();
        let network = button_center(1);

        assert!(menu.handle_pointer_down(network).is_empty());
        assert!(menu.handle_key_down(KeyCode::Enter).is_empty());
        assert!(menu.handle_pointer_up(network).is_empty());
        assert_eq!(
            menu.handle_key_up(KeyCode::Enter),
            vec![MainMenuAction::Activate(MainMenuItem::LocalGame)]
        );

        assert!(menu.handle_key_down(KeyCode::Space).is_empty());
        assert!(menu.handle_pointer_down(network).is_empty());
        assert!(menu.handle_key_up(KeyCode::Space).is_empty());
        assert_eq!(
            menu.handle_pointer_up(network),
            vec![MainMenuAction::Activate(MainMenuItem::NetworkGame)]
        );
    }

    #[test]
    fn main_menu_chrome_defers_captions_to_native_physical_pass() {
        // C++ keeps GUI geometry logical but draws the scale-3 CStdFont atlas
        // into the scale-3 framebuffer (C4GuiButton.cpp:100-109;
        // StdFont.cpp:319-352,841-842). The base image may be filtered, but a
        // scale-1 caption must never be baked into that filtered image.
        let bytes = std::fs::read(
            std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"))
                .join("../../../planet/System.c4g/Endeavour.ttf"),
        )
        .expect("read Endeavour.ttf");
        let fonts =
            crate::clonk_fonts::build_native_font_set(&bytes, 3).expect("build native GUI fonts");
        let mut menu = main_menu();
        let mut logical = Surface::new(1280, 720, PixelFormat::Rgba8888);
        menu.render_chrome(&mut logical);
        assert!(
            !logical
                .pixels()
                .chunks_exact(4)
                .any(|pixel| pixel[0] > 240 && pixel[1] > 240 && pixel[2] < 20),
            "the bilinear base must not contain the yellow scale-1 caption"
        );

        let mut physical = Surface::new(3840, 2160, PixelFormat::Rgba8888);
        physical.pixels_mut().fill(30);
        menu.render_native_text(&mut physical, &fonts, "Player", None);
        assert!(
            physical
                .pixels()
                .chunks_exact(4)
                .any(|pixel| pixel[0] > 240 && pixel[1] > 240 && pixel[2] < 20),
            "button captions must be rasterized directly into physical pixels"
        );
    }
}
