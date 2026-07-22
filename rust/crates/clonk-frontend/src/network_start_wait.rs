//! Isolated classic `C4Network2StartWaitDlg` presentation and input state.
//!
//! The application owns network status barriers and restart/abort side effects.
//! This module only retains the visible non-host client snapshot, renders the
//! classic dialog, and reports which of its two closing actions was requested.

use anyhow::{ensure, Result};
use clonk_graphics::clonk_font::TextAlign;
use clonk_graphics::{GammaRamp, Surface};
use clonk_gui::Rect as GuiRect;

use crate::classic_gui::{
    blacken_transparent_pixels, draw_3d_frame, draw_clipped_text_with_markup, draw_engine_box,
    draw_facet_stretch, ClassicButtonState, ClassicGuiSkin, IntRect,
};
use crate::{ClonkFontSet, GuiPoint, ImageData, KeyCode};

const SMALL_DIALOG_WIDTH: i32 = 250;
const LARGE_DIALOG_WIDTH: i32 = 500;
const SMALL_DIALOG_HEIGHT: i32 = 300;
const LARGE_DIALOG_HEIGHT: i32 = 600;
const LARGE_WIDTH_THRESHOLD: i32 = 800;
const LARGE_HEIGHT_THRESHOLD: i32 = 600;

const MIN_CAPTION_HEIGHT: i32 = 23;
const DIALOG_INDENT: i32 = 10;
const WAIT_LABEL_HEIGHT: i32 = 25;
const BUTTON_AREA_HEIGHT: i32 = 40;
const BUTTON_WIDTH: i32 = 120;
const BUTTON_HEIGHT: i32 = 32;
const BUTTON_GAP: i32 = 10;
const LIST_MARGIN: i32 = 3;
const ROW_VERTICAL_INDENT: i32 = 2;
const ICON_LABEL_SPACING: i32 = 2;

const STANDARD_ICON_CELL: u32 = 40;
const STANDARD_ICON_SHEET_WIDTH: u32 = 240;
const STANDARD_ICON_SHEET_HEIGHT: u32 = 360;
const BUTTON_HIGHLIGHT_WIDTH: u32 = 16;
const BUTTON_HIGHLIGHT_HEIGHT: u32 = 16;

const CLOSE_ICON_PHASE: u16 = 34;
const KICK_ICON_PHASE: u16 = 16;
const LIST_BACKGROUND_COLOR: u32 = 0x7f00_0000;
const WHITE: [u8; 4] = [255, 255, 255, 255];

/// C++ client status projected onto the three icons visible in the startup
/// client list (`Ico_Loading`, `Ico_Ready`, and `Ico_Kick`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum NetworkStartWaitClientStatus {
    Loading,
    Ready,
    Kick,
}

impl NetworkStartWaitClientStatus {
    /// Phase in the 6-column, 40px-cell `GUIIcons.png` sheet.
    pub const fn icon_phase(self) -> u16 {
        match self {
            Self::Loading => 17,
            Self::Ready => 47,
            Self::Kick => 16,
        }
    }
}

/// One client supplied by the application. Client zero is the network host
/// and is deliberately filtered by [`NetworkStartWaitState`].
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct NetworkStartWaitClient {
    pub client_id: i32,
    pub name: String,
    pub status: NetworkStartWaitClientStatus,
}

impl NetworkStartWaitClient {
    pub fn new(
        client_id: i32,
        name: impl Into<String>,
        status: NetworkStartWaitClientStatus,
    ) -> Self {
        Self {
            client_id,
            name: name.into(),
            status,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum NetworkStartWaitAction {
    Restart,
    Cancel,
    Kick(i32),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum NetworkStartWaitControl {
    Close,
    Restart,
    Cancel,
    Kick(i32),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum NetworkStartWaitSound {
    ArrowHit,
    Click,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct NetworkStartWaitLabels {
    pub caption: String,
    pub waiting: String,
    pub restart: String,
    pub cancel: String,
}

impl Default for NetworkStartWaitLabels {
    fn default() -> Self {
        Self {
            caption: "Network".into(),
            waiting: "Waiting for start...".into(),
            restart: "&Restart".into(),
            cancel: "Cancel".into(),
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct NetworkStartWaitClientLayout {
    pub client_id: i32,
    pub row: IntRect,
    pub icon: IntRect,
    pub label: IntRect,
    pub kick_button: Option<IntRect>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct NetworkStartWaitLayout {
    pub bounds: IntRect,
    pub caption: IntRect,
    pub close_button: IntRect,
    pub waiting_label: IntRect,
    pub client_list: IntRect,
    pub client_list_clip: IntRect,
    pub client_scroll: i32,
    pub client_scroll_max: i32,
    pub clients: Vec<NetworkStartWaitClientLayout>,
    pub restart_button: IntRect,
    pub cancel_button: IntRect,
}

/// Validated assets required by the classic dialog. No generic or synthesized
/// fallback graphics are accepted.
#[derive(Clone)]
pub struct NetworkStartWaitResources<'a> {
    skin: ClassicGuiSkin<'a>,
    fonts: &'a ClonkFontSet,
    icons: ImageData,
    button_highlight: ImageData,
}

impl<'a> NetworkStartWaitResources<'a> {
    pub fn new(
        skin: ClassicGuiSkin<'a>,
        fonts: &'a ClonkFontSet,
        icons: &ImageData,
        button_highlight: &ImageData,
    ) -> Result<Self> {
        let resources = Self {
            skin,
            fonts,
            icons: blacken_transparent_pixels(icons),
            button_highlight: blacken_transparent_pixels(button_highlight),
        };
        resources.validate()?;
        Ok(resources)
    }

    pub fn fonts(&self) -> &ClonkFontSet {
        self.fonts
    }

    fn validate(&self) -> Result<()> {
        self.skin.validate_message_dialog_assets()?;
        ensure!(
            (self.icons.width(), self.icons.height())
                == (STANDARD_ICON_SHEET_WIDTH, STANDARD_ICON_SHEET_HEIGHT),
            "GUIIcons.png must be the exact {}x{} classic sheet, got {}x{}",
            STANDARD_ICON_SHEET_WIDTH,
            STANDARD_ICON_SHEET_HEIGHT,
            self.icons.width(),
            self.icons.height()
        );
        ensure!(
            (
                self.button_highlight.width(),
                self.button_highlight.height()
            ) == (BUTTON_HIGHLIGHT_WIDTH, BUTTON_HIGHLIGHT_HEIGHT),
            "GUIButtonHighlight.png must be the exact {}x{} classic facet, got {}x{}",
            BUTTON_HIGHLIGHT_WIDTH,
            BUTTON_HIGHLIGHT_HEIGHT,
            self.button_highlight.width(),
            self.button_highlight.height()
        );
        ensure!(
            self.fonts.text.line_height > 0,
            "classic TextFont must have a positive line height"
        );
        Ok(())
    }
}

/// Pure interaction and presentation state for one start-wait dialog.
#[derive(Clone, Debug)]
pub struct NetworkStartWaitState {
    labels: NetworkStartWaitLabels,
    clients: Vec<NetworkStartWaitClient>,
    focus: Option<NetworkStartWaitControl>,
    hovered: Option<NetworkStartWaitControl>,
    pointer: Option<GuiPoint>,
    pointer_pressed: Option<NetworkStartWaitControl>,
    key_pressed: Option<(NetworkStartWaitControl, KeyCode)>,
    client_scroll: i32,
    sound_events: Vec<NetworkStartWaitSound>,
}

impl Default for NetworkStartWaitState {
    fn default() -> Self {
        Self {
            labels: NetworkStartWaitLabels::default(),
            clients: Vec::new(),
            focus: None,
            hovered: None,
            pointer: None,
            pointer_pressed: None,
            key_pressed: None,
            client_scroll: 0,
            sound_events: Vec::new(),
        }
    }
}

impl NetworkStartWaitState {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn with_clients(clients: impl IntoIterator<Item = NetworkStartWaitClient>) -> Self {
        let mut state = Self::new();
        state.replace_clients(clients);
        state
    }

    pub fn labels(&self) -> &NetworkStartWaitLabels {
        &self.labels
    }

    pub fn set_labels(&mut self, labels: NetworkStartWaitLabels) {
        self.labels = labels;
    }

    pub fn clients(&self) -> &[NetworkStartWaitClient] {
        &self.clients
    }

    pub const fn focused_control(&self) -> Option<NetworkStartWaitControl> {
        self.focus
    }

    /// Replaces the complete client snapshot. Host ID zero is omitted and the
    /// remaining rows are kept in the client-list's ascending ID order.
    pub fn replace_clients(&mut self, clients: impl IntoIterator<Item = NetworkStartWaitClient>) {
        self.clients.clear();
        self.client_scroll = 0;
        for client in clients {
            self.update_client(client);
        }
    }

    /// Inserts or replaces one client row. Updating host ID zero removes any
    /// stale row with that ID but never inserts a host into the startup list.
    pub fn update_client(&mut self, client: NetworkStartWaitClient) {
        match self
            .clients
            .binary_search_by_key(&client.client_id, |entry| entry.client_id)
        {
            Ok(index) if client.client_id == 0 => {
                self.clients.remove(index);
            }
            Ok(index) => self.clients[index] = client,
            Err(_) if client.client_id == 0 => {}
            Err(index) => self.clients.insert(index, client),
        }
    }

    pub fn update_client_status(
        &mut self,
        client_id: i32,
        status: NetworkStartWaitClientStatus,
    ) -> bool {
        let Ok(index) = self
            .clients
            .binary_search_by_key(&client_id, |entry| entry.client_id)
        else {
            return false;
        };
        if self.clients[index].status == status {
            return false;
        }
        self.clients[index].status = status;
        true
    }

    pub fn remove_client(&mut self, client_id: i32) -> bool {
        let Ok(index) = self
            .clients
            .binary_search_by_key(&client_id, |entry| entry.client_id)
        else {
            return false;
        };
        self.clients.remove(index);
        true
    }

    pub fn take_sound_events(&mut self) -> Vec<NetworkStartWaitSound> {
        std::mem::take(&mut self.sound_events)
    }

    pub fn layout(
        &self,
        screen_width: i32,
        screen_height: i32,
        fonts: &ClonkFontSet,
    ) -> NetworkStartWaitLayout {
        let width = if screen_width > LARGE_WIDTH_THRESHOLD {
            LARGE_DIALOG_WIDTH
        } else {
            SMALL_DIALOG_WIDTH
        };
        let height = if screen_height > LARGE_HEIGHT_THRESHOLD {
            LARGE_DIALOG_HEIGHT
        } else {
            SMALL_DIALOG_HEIGHT
        };
        let bounds = IntRect {
            x: (screen_width - width) / 2,
            y: (screen_height - height) / 2,
            w: width,
            h: height,
        };
        let caption_height = fonts.text.line_height.max(MIN_CAPTION_HEIGHT);
        let caption = IntRect {
            h: caption_height,
            ..bounds
        };
        let close_button = IntRect {
            x: caption.x + caption.w - 20,
            y: caption.y + 4,
            w: 16,
            h: 16,
        };
        let mut available = IntRect {
            x: bounds.x,
            y: bounds.y + caption_height,
            w: bounds.w,
            h: (bounds.h - caption_height).max(0),
        };

        let button_area = IntRect {
            x: available.x + DIALOG_INDENT,
            y: available.y + available.h - BUTTON_AREA_HEIGHT - DIALOG_INDENT,
            w: available.w - 2 * DIALOG_INDENT,
            h: BUTTON_AREA_HEIGHT,
        };
        available.h = (available.h - BUTTON_AREA_HEIGHT - 2 * DIALOG_INDENT).max(0);

        let waiting_label = IntRect {
            x: available.x + DIALOG_INDENT,
            y: available.y + DIALOG_INDENT,
            w: available.w - 2 * DIALOG_INDENT,
            h: WAIT_LABEL_HEIGHT,
        };
        available.y += WAIT_LABEL_HEIGHT + 2 * DIALOG_INDENT;
        available.h = (available.h - WAIT_LABEL_HEIGHT - 2 * DIALOG_INDENT).max(0);

        let client_list = IntRect {
            x: available.x + DIALOG_INDENT,
            y: available.y + DIALOG_INDENT,
            w: available.w - 2 * DIALOG_INDENT,
            h: (available.h - 2 * DIALOG_INDENT).max(0),
        };
        let client_list_clip = IntRect {
            x: client_list.x + LIST_MARGIN,
            y: client_list.y + LIST_MARGIN,
            w: (client_list.w - 2 * LIST_MARGIN).max(0),
            h: (client_list.h - 2 * LIST_MARGIN).max(0),
        };

        let icon_size = 2 * fonts.text.line_height;
        let row_height = icon_size + 2 * ROW_VERTICAL_INDENT;
        let client_scroll_max = i32::try_from(self.clients.len())
            .unwrap_or(i32::MAX)
            .saturating_mul(row_height)
            .saturating_sub(client_list_clip.h)
            .max(0);
        let client_scroll = self.client_scroll.clamp(0, client_scroll_max);
        let clients = self
            .clients
            .iter()
            .enumerate()
            .map(|(index, client)| {
                let row = IntRect {
                    x: client_list_clip.x,
                    y: client_list_clip.y + i32::try_from(index).unwrap_or(i32::MAX) * row_height
                        - client_scroll,
                    w: client_list_clip.w,
                    h: row_height,
                };
                let icon = IntRect {
                    x: row.x,
                    y: row.y + ROW_VERTICAL_INDENT,
                    w: icon_size,
                    h: icon_size,
                };
                let kick_size = icon_size.max(16);
                let kick_button = (client.client_id != 0).then_some(IntRect {
                    x: row.x + row.w - kick_size - 2,
                    y: row.y + 1,
                    w: kick_size,
                    h: kick_size,
                });
                let label = IntRect {
                    x: icon.x + icon.w + ICON_LABEL_SPACING,
                    y: row.y + ROW_VERTICAL_INDENT,
                    w: (kick_button.map_or(row.x + row.w, |button| button.x)
                        - icon.x
                        - icon.w
                        - ICON_LABEL_SPACING)
                        .max(0),
                    h: fonts.text.line_height,
                };
                NetworkStartWaitClientLayout {
                    client_id: client.client_id,
                    row,
                    icon,
                    label,
                    kick_button,
                }
            })
            .collect();

        let button_group_width = 2 * BUTTON_WIDTH + BUTTON_GAP;
        let first_button_x = button_area.x + (button_area.w - button_group_width) / 2;
        let button_y = button_area.y + (button_area.h - BUTTON_HEIGHT) / 2;
        let restart_button = IntRect {
            x: first_button_x,
            y: button_y,
            w: BUTTON_WIDTH,
            h: BUTTON_HEIGHT,
        };
        let cancel_button = IntRect {
            x: first_button_x + BUTTON_WIDTH + BUTTON_GAP,
            ..restart_button
        };

        NetworkStartWaitLayout {
            bounds,
            caption,
            close_button,
            waiting_label,
            client_list,
            client_list_clip,
            client_scroll,
            client_scroll_max,
            clients,
            restart_button,
            cancel_button,
        }
    }

    pub fn handle_pointer_move(&mut self, point: GuiPoint, layout: &NetworkStartWaitLayout) {
        let was_down = self.pointer_target_is_down();
        self.pointer = Some(point);
        self.hovered = hit_target(layout, point);
        if was_down != self.pointer_target_is_down() {
            self.sound_events.push(NetworkStartWaitSound::ArrowHit);
        }
    }

    /// `C4GUI::ScrollWindow` consumes the native wheel sign: positive moves
    /// toward the first row and negative moves toward later rows.
    pub fn handle_wheel(
        &mut self,
        point: GuiPoint,
        native_delta: i32,
        layout: &NetworkStartWaitLayout,
    ) -> bool {
        if !rect_contains(layout.client_list, point) || native_delta == 0 {
            return false;
        }
        let next = layout
            .client_scroll
            .saturating_sub(native_delta)
            .clamp(0, layout.client_scroll_max);
        if next == layout.client_scroll {
            return false;
        }
        self.client_scroll = next;
        true
    }

    pub fn handle_pointer_down(
        &mut self,
        point: GuiPoint,
        layout: &NetworkStartWaitLayout,
    ) -> Vec<NetworkStartWaitAction> {
        self.pointer = Some(point);
        self.hovered = hit_target(layout, point);
        self.pointer_pressed = self.hovered;
        if let Some(target) = self.pointer_pressed {
            self.focus = Some(target);
            self.key_pressed = None;
            self.sound_events.push(NetworkStartWaitSound::ArrowHit);
        }
        Vec::new()
    }

    pub fn handle_pointer_up(
        &mut self,
        point: GuiPoint,
        layout: &NetworkStartWaitLayout,
    ) -> Vec<NetworkStartWaitAction> {
        self.pointer = Some(point);
        self.hovered = hit_target(layout, point);
        let Some(pressed) = self.pointer_pressed.take() else {
            return Vec::new();
        };
        if self.hovered != Some(pressed) {
            return Vec::new();
        }
        self.sound_events.push(NetworkStartWaitSound::Click);
        vec![action_for(pressed)]
    }

    pub const fn has_pointer_capture(&self) -> bool {
        self.pointer_pressed.is_some()
    }

    pub fn cancel_pointer_capture(&mut self) {
        let was_down = self.pointer_target_is_down();
        self.pointer = None;
        self.hovered = None;
        self.pointer_pressed = None;
        if was_down {
            self.sound_events.push(NetworkStartWaitSound::ArrowHit);
        }
    }

    pub fn pointer_left(&mut self) {
        let was_down = self.pointer_target_is_down();
        self.pointer = None;
        self.hovered = None;
        if was_down {
            self.sound_events.push(NetworkStartWaitSound::ArrowHit);
        }
    }

    pub fn cancel_interaction(&mut self) {
        self.pointer = None;
        self.hovered = None;
        self.pointer_pressed = None;
        self.key_pressed = None;
        self.sound_events.clear();
    }

    pub fn handle_key_down(&mut self, key: KeyCode) -> Vec<NetworkStartWaitAction> {
        self.handle_key_down_with_tab_direction(key, false)
    }

    pub fn handle_key_down_with_tab_direction(
        &mut self,
        key: KeyCode,
        backwards: bool,
    ) -> Vec<NetworkStartWaitAction> {
        match key {
            KeyCode::Escape => vec![NetworkStartWaitAction::Cancel],
            KeyCode::Tab => {
                self.advance_focus(backwards);
                Vec::new()
            }
            KeyCode::Left | KeyCode::Up => {
                self.advance_focus(true);
                Vec::new()
            }
            KeyCode::Right | KeyCode::Down => {
                self.advance_focus(false);
                Vec::new()
            }
            KeyCode::Enter | KeyCode::Space => {
                if self.key_pressed.is_none() {
                    if let Some(focused) = self.focus {
                        self.key_pressed = Some((focused, key));
                        self.sound_events.push(NetworkStartWaitSound::ArrowHit);
                    }
                }
                Vec::new()
            }
            KeyCode::Home | KeyCode::End | KeyCode::PageUp | KeyCode::PageDown => Vec::new(),
        }
    }

    pub fn handle_key_up(&mut self, key: KeyCode) -> Vec<NetworkStartWaitAction> {
        let Some((pressed, pressed_key)) = self.key_pressed.take() else {
            return Vec::new();
        };
        if pressed_key != key || self.focus != Some(pressed) {
            return Vec::new();
        }
        self.sound_events.push(NetworkStartWaitSound::Click);
        vec![action_for(pressed)]
    }

    pub fn handle_hotkey(&mut self, character: char) -> Vec<NetworkStartWaitAction> {
        if character.eq_ignore_ascii_case(&'r') {
            vec![NetworkStartWaitAction::Restart]
        } else {
            Vec::new()
        }
    }

    pub fn handle_gamepad_horizontal(&mut self, backwards: bool) {
        self.advance_focus(backwards);
    }

    pub fn handle_gamepad_low_down(&mut self) -> Vec<NetworkStartWaitAction> {
        self.handle_key_down(KeyCode::Enter)
    }

    pub fn handle_gamepad_low_up(&mut self) -> Vec<NetworkStartWaitAction> {
        self.handle_key_up(KeyCode::Enter)
    }

    pub fn handle_gamepad_high_down(&mut self) -> Vec<NetworkStartWaitAction> {
        vec![NetworkStartWaitAction::Cancel]
    }

    pub fn render(
        &self,
        surface: &mut Surface,
        resources: &NetworkStartWaitResources<'_>,
        active: bool,
        gamma: Option<&GammaRamp>,
    ) -> Result<()> {
        resources.validate()?;
        let layout = self.layout(
            surface.width() as i32,
            surface.height() as i32,
            resources.fonts,
        );

        resources.skin.draw_dialog(surface, layout.bounds, gamma);
        resources.skin.draw_caption_with_right_indent(
            surface,
            layout.caption,
            &self.labels.caption,
            &resources.fonts.text,
            WHITE,
            TextAlign::Left,
            20,
            gamma,
        );
        resources.fonts.text.draw_with_gamma(
            surface,
            layout.waiting_label.x + layout.waiting_label.w / 2,
            layout.waiting_label.y
                + (layout.waiting_label.h - resources.fonts.text.line_height) / 2,
            &self.labels.waiting,
            WHITE,
            TextAlign::Center,
            true,
            gamma,
        );

        draw_engine_box(
            surface,
            layout.client_list.x,
            layout.client_list.y,
            layout.client_list.x + layout.client_list.w - 1,
            layout.client_list.y + layout.client_list.h - 1,
            LIST_BACKGROUND_COLOR,
            gamma,
        );
        draw_3d_frame(surface, layout.client_list, gamma);

        let previous_clip = surface.clip();
        if let Some(clip) = intersect_surface_clip(surface, layout.client_list_clip, previous_clip)
        {
            surface.set_clip(clip);
            for (client, row) in self.clients.iter().zip(&layout.clients) {
                draw_standard_icon(
                    surface,
                    row.icon,
                    &resources.icons,
                    client.status.icon_phase(),
                    gamma,
                )?;
                draw_clipped_text_with_markup(
                    surface,
                    &resources.fonts.text,
                    row.label.x,
                    row.label.y,
                    &client.name,
                    WHITE,
                    TextAlign::Left,
                    gamma,
                    layout.client_list_clip,
                    false,
                );
                if let Some(kick_button) = row.kick_button {
                    draw_icon_button(
                        surface,
                        resources,
                        kick_button,
                        KICK_ICON_PHASE,
                        NetworkStartWaitControl::Kick(client.client_id),
                        self,
                        active,
                        gamma,
                    )?;
                }
            }
        }
        match previous_clip {
            Some(clip) => surface.set_clip(clip),
            None => surface.clear_clip(),
        }

        draw_button(
            surface,
            resources,
            layout.restart_button,
            &self.labels.restart,
            NetworkStartWaitControl::Restart,
            self,
            active,
            gamma,
        );
        draw_button(
            surface,
            resources,
            layout.cancel_button,
            &self.labels.cancel,
            NetworkStartWaitControl::Cancel,
            self,
            active,
            gamma,
        );
        draw_close_button(surface, resources, &layout, self, active, gamma)?;
        Ok(())
    }

    fn advance_focus(&mut self, backwards: bool) {
        const ORDER: [NetworkStartWaitControl; 3] = [
            NetworkStartWaitControl::Close,
            NetworkStartWaitControl::Restart,
            NetworkStartWaitControl::Cancel,
        ];
        let Some(focused) = self.focus else {
            self.focus = Some(if backwards {
                NetworkStartWaitControl::Cancel
            } else {
                NetworkStartWaitControl::Restart
            });
            self.key_pressed = None;
            return;
        };
        let index = ORDER
            .iter()
            .position(|target| *target == focused)
            .unwrap_or(0);
        let next = if backwards {
            (index + ORDER.len() - 1) % ORDER.len()
        } else {
            (index + 1) % ORDER.len()
        };
        self.focus = Some(ORDER[next]);
        self.key_pressed = None;
    }

    fn target_pressed(&self, target: NetworkStartWaitControl) -> bool {
        self.key_pressed
            .is_some_and(|(pressed, _)| pressed == target)
            || (self.pointer_pressed == Some(target) && self.hovered == Some(target))
    }

    fn pointer_target_is_down(&self) -> bool {
        self.pointer_pressed.is_some() && self.pointer_pressed == self.hovered
    }
}

#[allow(clippy::too_many_arguments)]
fn draw_button(
    surface: &mut Surface,
    resources: &NetworkStartWaitResources<'_>,
    rect: IntRect,
    label: &str,
    target: NetworkStartWaitControl,
    state: &NetworkStartWaitState,
    active: bool,
    gamma: Option<&GammaRamp>,
) {
    resources.skin.draw_button(
        surface,
        rect,
        label,
        resources.fonts,
        ClassicButtonState {
            pressed: active && state.target_pressed(target),
            highlighted: active && (state.focus == Some(target) || state.hovered == Some(target)),
        },
        gamma,
    );
}

fn draw_close_button(
    surface: &mut Surface,
    resources: &NetworkStartWaitResources<'_>,
    layout: &NetworkStartWaitLayout,
    state: &NetworkStartWaitState,
    active: bool,
    gamma: Option<&GammaRamp>,
) -> Result<()> {
    draw_icon_button(
        surface,
        resources,
        layout.close_button,
        CLOSE_ICON_PHASE,
        NetworkStartWaitControl::Close,
        state,
        active,
        gamma,
    )
}

#[allow(clippy::too_many_arguments)]
fn draw_icon_button(
    surface: &mut Surface,
    resources: &NetworkStartWaitResources<'_>,
    rect: IntRect,
    phase: u16,
    target: NetworkStartWaitControl,
    state: &NetworkStartWaitState,
    active: bool,
    gamma: Option<&GammaRamp>,
) -> Result<()> {
    let highlighted = active && (state.focus == Some(target) || state.hovered == Some(target));
    if highlighted {
        crate::draw_image_bilinear_additive(
            surface,
            &gui_rect(rect),
            &resources.button_highlight,
            gamma,
        );
    }
    draw_standard_icon(surface, rect, &resources.icons, phase, gamma)?;
    if active && state.target_pressed(target) {
        crate::draw_image_bilinear_additive(
            surface,
            &gui_rect(rect),
            &resources.button_highlight,
            gamma,
        );
    }
    Ok(())
}

fn draw_standard_icon(
    surface: &mut Surface,
    rect: IntRect,
    icons: &ImageData,
    phase: u16,
    gamma: Option<&GammaRamp>,
) -> Result<()> {
    let columns = icons.width() / STANDARD_ICON_CELL;
    let phase = u32::from(phase);
    let source_x = phase % columns * STANDARD_ICON_CELL;
    let source_y = phase / columns * STANDARD_ICON_CELL;
    ensure!(
        source_x + STANDARD_ICON_CELL <= icons.width()
            && source_y + STANDARD_ICON_CELL <= icons.height(),
        "GUIIcons.png phase {phase} is outside the classic sheet"
    );
    draw_facet_stretch(
        surface,
        icons,
        (
            source_x as f32,
            source_y as f32,
            STANDARD_ICON_CELL as f32,
            STANDARD_ICON_CELL as f32,
        ),
        (rect.x as f32, rect.y as f32, rect.w as f32, rect.h as f32),
        gamma,
    );
    Ok(())
}

fn action_for(control: NetworkStartWaitControl) -> NetworkStartWaitAction {
    match control {
        NetworkStartWaitControl::Restart => NetworkStartWaitAction::Restart,
        NetworkStartWaitControl::Kick(client_id) => NetworkStartWaitAction::Kick(client_id),
        NetworkStartWaitControl::Close | NetworkStartWaitControl::Cancel => {
            NetworkStartWaitAction::Cancel
        }
    }
}

fn hit_target(layout: &NetworkStartWaitLayout, point: GuiPoint) -> Option<NetworkStartWaitControl> {
    if rect_contains(layout.client_list_clip, point) {
        if let Some(target) = layout.clients.iter().find_map(|row| {
            row.kick_button
                .filter(|rect| rect_contains(*rect, point))
                .map(|_| NetworkStartWaitControl::Kick(row.client_id))
        }) {
            return Some(target);
        }
    }
    [
        (NetworkStartWaitControl::Close, layout.close_button),
        (NetworkStartWaitControl::Restart, layout.restart_button),
        (NetworkStartWaitControl::Cancel, layout.cancel_button),
    ]
    .into_iter()
    .find_map(|(target, rect)| rect_contains(rect, point).then_some(target))
}

fn rect_contains(rect: IntRect, point: GuiPoint) -> bool {
    point.x >= rect.x as f32
        && point.x < (rect.x + rect.w) as f32
        && point.y >= rect.y as f32
        && point.y < (rect.y + rect.h) as f32
}

fn gui_rect(rect: IntRect) -> GuiRect {
    GuiRect::new(rect.x as f32, rect.y as f32, rect.w as f32, rect.h as f32)
}

fn intersect_surface_clip(
    surface: &Surface,
    requested: IntRect,
    existing: Option<clonk_graphics::Rect>,
) -> Option<clonk_graphics::Rect> {
    let mut left = requested.x.max(0);
    let mut top = requested.y.max(0);
    let mut right = (requested.x + requested.w).min(surface.width() as i32);
    let mut bottom = (requested.y + requested.h).min(surface.height() as i32);
    if let Some(existing) = existing {
        left = left.max(existing.x);
        top = top.max(existing.y);
        right = right.min(existing.x + existing.width as i32);
        bottom = bottom.min(existing.y + existing.height as i32);
    }
    (left < right && top < bottom)
        .then(|| clonk_graphics::Rect::new(left, top, (right - left) as u32, (bottom - top) as u32))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_support::endeavour_font_set;

    fn center(rect: IntRect) -> GuiPoint {
        GuiPoint::new((rect.x + rect.w / 2) as f32, (rect.y + rect.h / 2) as f32)
    }

    fn client(id: i32, name: &str, status: NetworkStartWaitClientStatus) -> NetworkStartWaitClient {
        NetworkStartWaitClient::new(id, name, status)
    }

    #[test]
    fn layout_applies_width_and_height_thresholds_independently() {
        let fonts = endeavour_font_set();
        let state = NetworkStartWaitState::new();
        for (screen, expected) in [
            ((800, 600), (250, 300)),
            ((801, 600), (500, 300)),
            ((800, 601), (250, 600)),
            ((801, 601), (500, 600)),
        ] {
            let layout = state.layout(screen.0, screen.1, &fonts);
            assert_eq!((layout.bounds.w, layout.bounds.h), expected);
            assert_eq!(layout.bounds.x, (screen.0 - expected.0) / 2);
            assert_eq!(layout.bounds.y, (screen.1 - expected.1) / 2);
            assert_eq!(layout.waiting_label.h, WAIT_LABEL_HEIGHT);
            assert_eq!(layout.restart_button.w, BUTTON_WIDTH);
            assert_eq!(layout.cancel_button.w, BUTTON_WIDTH);
            assert_eq!(
                layout.restart_button.x + BUTTON_WIDTH + BUTTON_GAP,
                layout.cancel_button.x
            );
            assert_eq!(
                layout.restart_button.x + layout.cancel_button.x + BUTTON_WIDTH,
                2 * layout.bounds.x + layout.bounds.w
            );
        }
    }

    #[test]
    fn client_snapshot_filters_host_sorts_rows_and_updates_in_place() {
        let fonts = endeavour_font_set();
        let mut state = NetworkStartWaitState::with_clients([
            client(9, "Ready client", NetworkStartWaitClientStatus::Ready),
            client(0, "Host", NetworkStartWaitClientStatus::Ready),
            client(4, "Loading client", NetworkStartWaitClientStatus::Loading),
        ]);
        assert_eq!(
            state
                .clients()
                .iter()
                .map(|client| client.client_id)
                .collect::<Vec<_>>(),
            [4, 9]
        );

        state.update_client(client(
            4,
            "Removed client",
            NetworkStartWaitClientStatus::Kick,
        ));
        state.update_client(client(
            7,
            "New client",
            NetworkStartWaitClientStatus::Loading,
        ));
        assert!(state.update_client_status(7, NetworkStartWaitClientStatus::Ready));
        assert!(!state.update_client_status(7, NetworkStartWaitClientStatus::Ready));
        assert_eq!(state.clients()[0].name, "Removed client");
        assert_eq!(state.clients()[0].status.icon_phase(), 16);
        assert_eq!(state.clients()[1].status.icon_phase(), 47);
        assert_eq!(state.clients()[2].status.icon_phase(), 47);

        let layout = state.layout(1280, 720, &fonts);
        assert_eq!(
            layout
                .clients
                .iter()
                .map(|row| row.client_id)
                .collect::<Vec<_>>(),
            [4, 7, 9]
        );
        assert!(layout
            .clients
            .windows(2)
            .all(|rows| rows[0].row.y < rows[1].row.y));
        assert!(state.remove_client(7));
        assert!(!state.remove_client(7));
    }

    #[test]
    fn non_host_kick_button_emits_the_client_id_and_obeys_list_clipping() {
        let fonts = endeavour_font_set();
        let mut state = NetworkStartWaitState::with_clients([
            client(0, "Host", NetworkStartWaitClientStatus::Ready),
            client(7, "Remote", NetworkStartWaitClientStatus::Loading),
        ]);
        let layout = state.layout(1280, 720, &fonts);
        assert!(layout.clients.iter().all(|row| row.client_id != 0));
        let remote = layout
            .clients
            .iter()
            .find(|row| row.client_id == 7)
            .expect("remote row");
        let kick = remote.kick_button.expect("remote kick button");
        assert_eq!(kick.x + kick.w, remote.row.x + remote.row.w - 2);
        assert_eq!(kick.y, remote.row.y + 1);
        assert!(remote.label.x + remote.label.w <= kick.x);

        assert!(state
            .handle_pointer_down(center(remote.icon), &layout)
            .is_empty());
        assert!(state
            .handle_pointer_up(center(remote.icon), &layout)
            .is_empty());
        assert!(state.handle_pointer_down(center(kick), &layout).is_empty());
        assert_eq!(
            state.handle_pointer_up(center(kick), &layout),
            [NetworkStartWaitAction::Kick(7)]
        );

        let mut clipped = layout.clone();
        clipped.client_list_clip.y = kick.y + kick.h;
        clipped.client_list_clip.h = 1;
        assert!(state.handle_pointer_down(center(kick), &clipped).is_empty());
        assert!(state.handle_pointer_up(center(kick), &clipped).is_empty());
    }

    #[test]
    fn overflowing_client_list_scrolls_to_later_rows() {
        let fonts = endeavour_font_set();
        let mut state = NetworkStartWaitState::with_clients((1..=12).map(|id| {
            client(
                id,
                &format!("Client {id}"),
                NetworkStartWaitClientStatus::Loading,
            )
        }));
        let initial = state.layout(640, 480, &fonts);
        assert!(initial.client_scroll_max > 0);
        assert!(
            initial.clients.last().unwrap().row.y
                >= initial.client_list_clip.y + initial.client_list_clip.h
        );

        assert!(state.handle_wheel(
            center(initial.client_list),
            -initial.client_scroll_max,
            &initial,
        ));
        let scrolled = state.layout(640, 480, &fonts);
        assert_eq!(scrolled.client_scroll, scrolled.client_scroll_max);
        assert!(
            scrolled.clients.last().unwrap().row.y
                < scrolled.client_list_clip.y + scrolled.client_list_clip.h
        );
        assert!(scrolled.clients.first().unwrap().row.y < scrolled.client_list_clip.y);
    }

    #[test]
    fn escape_pointer_keyboard_and_gamepad_report_typed_actions() {
        let fonts = endeavour_font_set();
        let mut state = NetworkStartWaitState::new();
        let layout = state.layout(1280, 720, &fonts);

        assert_eq!(state.focused_control(), None);
        assert!(state.handle_key_down(KeyCode::Enter).is_empty());
        assert!(state.handle_key_up(KeyCode::Enter).is_empty());
        assert!(state.handle_key_down(KeyCode::Space).is_empty());
        assert!(state.handle_key_up(KeyCode::Space).is_empty());
        assert!(state.handle_gamepad_low_down().is_empty());
        assert!(state.handle_gamepad_low_up().is_empty());
        assert!(state.take_sound_events().is_empty());

        let mut forward = NetworkStartWaitState::new();
        assert!(forward.handle_key_down(KeyCode::Tab).is_empty());
        assert_eq!(
            forward.focused_control(),
            Some(NetworkStartWaitControl::Restart)
        );

        let mut backward = NetworkStartWaitState::new();
        assert!(backward
            .handle_key_down_with_tab_direction(KeyCode::Tab, true)
            .is_empty());
        assert_eq!(
            backward.focused_control(),
            Some(NetworkStartWaitControl::Cancel)
        );

        assert_eq!(
            state.handle_key_down(KeyCode::Escape),
            [NetworkStartWaitAction::Cancel]
        );
        assert!(state
            .handle_pointer_down(center(layout.restart_button), &layout)
            .is_empty());
        assert_eq!(
            state.handle_pointer_up(center(layout.restart_button), &layout),
            [NetworkStartWaitAction::Restart]
        );

        assert!(state.handle_key_down(KeyCode::Right).is_empty());
        assert_eq!(
            state.focused_control(),
            Some(NetworkStartWaitControl::Cancel)
        );
        assert!(state.handle_key_down(KeyCode::Enter).is_empty());
        assert_eq!(
            state.handle_key_up(KeyCode::Enter),
            [NetworkStartWaitAction::Cancel]
        );

        state.handle_gamepad_horizontal(true);
        assert_eq!(
            state.focused_control(),
            Some(NetworkStartWaitControl::Restart)
        );
        assert!(state.handle_gamepad_low_down().is_empty());
        assert_eq!(
            state.handle_gamepad_low_up(),
            [NetworkStartWaitAction::Restart]
        );
        assert_eq!(
            state.handle_gamepad_high_down(),
            [NetworkStartWaitAction::Cancel]
        );
    }
}
