use crate::{draw_text, fill_rect, GuiPoint, KeyCode};
use clonk_graphics::{Color, Surface, TextFont};
use clonk_gui::{Rect as GuiRect, Size as GuiSize};
use std::sync::Arc;

#[derive(Clone, Debug)]
pub struct ControlOptionItem {
    pub label: String,
    pub key_label: String,
    pub is_default: bool,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ControlOptionsAction {
    SelectionChanged(usize),
    RequestRebind(usize),
    ResetAll,
    Close,
}

pub struct ControlOptionsView {
    font: Arc<dyn TextFont>,
    items: Vec<ControlOptionItem>,
    size: GuiSize,
    layout: Option<Layout>,
    pointer_position: Option<GuiPoint>,
    selected_index: Option<usize>,
    pressed_entry: Option<usize>,
    waiting_for_rebind: Option<usize>,
    hovered_footer: Option<FooterButtonKind>,
    pressed_footer: Option<FooterButtonKind>,
}

struct Layout {
    panel: GuiRect,
    content_left: f32,
    content_width: f32,
    item_rects: Vec<GuiRect>,
    instructions_rect: GuiRect,
    footer_buttons: Vec<FooterButton>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum FooterButtonKind {
    ResetAll,
    Back,
}

#[derive(Clone, Copy, Debug)]
struct FooterButton {
    rect: GuiRect,
    kind: FooterButtonKind,
    label: &'static str,
}

impl FooterButton {
    const fn new(rect: GuiRect, kind: FooterButtonKind) -> Self {
        let label = match kind {
            FooterButtonKind::ResetAll => "Reset Defaults",
            FooterButtonKind::Back => "Back",
        };
        Self { rect, kind, label }
    }
}

const PANEL_SHADOW_COLOR: Color = Color::new(0, 0, 0, 120);
const PANEL_BACKGROUND_COLOR: Color = Color::new(16, 28, 52, 235);
const ROW_BACKGROUND_COLOR: Color = Color::new(26, 46, 78, 220);
const ROW_SELECTED_COLOR: Color = Color::new(40, 68, 112, 235);
const ROW_WAITING_COLOR: Color = Color::new(112, 76, 28, 235);
const KEY_BACKGROUND_COLOR: Color = Color::new(15, 30, 52, 220);
const KEY_CUSTOM_COLOR: Color = Color::new(54, 36, 88, 230);
const FOOTER_BACKGROUND_COLOR: Color = Color::new(10, 20, 36, 220);
const FOOTER_BUTTON_COLOR: Color = Color::new(28, 48, 80, 230);
const FOOTER_BUTTON_HOVER_COLOR: Color = Color::new(40, 66, 110, 245);
const TEXT_PRIMARY_COLOR: Color = Color::new(232, 238, 255, 255);
const TEXT_SECONDARY_COLOR: Color = Color::new(196, 206, 226, 255);
const TEXT_WARNING_COLOR: Color = Color::new(252, 216, 160, 255);

const TITLE_FONT_SIZE: f32 = 26.0;
const ROW_FONT_SIZE: f32 = 18.0;
const KEY_FONT_SIZE: f32 = 17.0;
const INSTRUCTION_FONT_SIZE: f32 = 15.5;
const FOOTER_FONT_SIZE: f32 = 17.0;

const ROW_HEIGHT: f32 = 34.0;
const ROW_SPACING: f32 = 6.0;
const PANEL_HORIZONTAL_PADDING: f32 = 28.0;
const PANEL_TOP_PADDING: f32 = 60.0;
const TITLE_BOTTOM_MARGIN: f32 = 20.0;
const FOOTER_HEIGHT: f32 = 56.0;
const INSTRUCTION_HEIGHT: f32 = 44.0;
const KEY_COLUMN_WIDTH: f32 = 150.0;

impl ControlOptionsView {
    pub fn new(font: Arc<dyn TextFont>) -> Self {
        Self {
            font,
            items: Vec::new(),
            size: GuiSize::new(0.0, 0.0),
            layout: None,
            pointer_position: None,
            selected_index: None,
            pressed_entry: None,
            waiting_for_rebind: None,
            hovered_footer: None,
            pressed_footer: None,
        }
    }

    pub fn resize(&mut self, width: f32, height: f32) {
        let clamped_width = width.max(1.0);
        let clamped_height = height.max(1.0);
        if (self.size.width - clamped_width).abs() > f32::EPSILON
            || (self.size.height - clamped_height).abs() > f32::EPSILON
        {
            self.size = GuiSize::new(clamped_width, clamped_height);
            self.layout = None;
        }
    }

    pub fn set_items(&mut self, items: Vec<ControlOptionItem>) {
        self.items = items;
        if let Some(selected) = self.selected_index {
            if selected >= self.items.len() {
                self.selected_index = self.items.is_empty().then_some(0);
            }
        }
        self.layout = None;
    }

    pub fn update_item(&mut self, index: usize, item: ControlOptionItem) {
        if index < self.items.len() {
            self.items[index] = item;
        }
    }

    pub fn set_pointer_position(&mut self, position: Option<GuiPoint>) {
        self.pointer_position = position;
        if position.is_none() {
            self.pressed_entry = None;
            self.hovered_footer = None;
        }
    }

    pub fn pointer_position(&self) -> Option<GuiPoint> {
        self.pointer_position
    }

    pub fn set_waiting_for_rebind(&mut self, entry: Option<usize>) {
        self.waiting_for_rebind = entry;
    }

    pub fn waiting_for_rebind(&self) -> Option<usize> {
        self.waiting_for_rebind
    }

    pub fn selected_index(&self) -> Option<usize> {
        self.selected_index
    }

    pub fn set_selected_index(&mut self, index: Option<usize>) {
        self.selected_index = index.filter(|&idx| idx < self.items.len());
    }

    pub fn handle_pointer_move(&mut self, position: GuiPoint) -> Vec<ControlOptionsAction> {
        self.pointer_position = Some(position);
        let mut actions = Vec::new();
        self.ensure_layout();
        let hovered = {
            let layout = self.layout();
            footer_button_at(layout, position)
        };
        self.hovered_footer = hovered;
        if let Some(index) = {
            let layout = self.layout();
            entry_at(layout, position)
        } {
            if self.selected_index != Some(index) {
                self.selected_index = Some(index);
                actions.push(ControlOptionsAction::SelectionChanged(index));
            }
        }
        actions
    }

    pub fn handle_pointer_down(&mut self, position: GuiPoint) -> Vec<ControlOptionsAction> {
        self.pointer_position = Some(position);
        self.ensure_layout();
        let hovered = {
            let layout = self.layout();
            footer_button_at(layout, position)
        };
        self.hovered_footer = hovered;
        self.pressed_footer = hovered;
        self.pressed_entry = {
            let layout = self.layout();
            entry_at(layout, position)
        };
        Vec::new()
    }

    pub fn handle_pointer_up(&mut self, position: GuiPoint) -> Vec<ControlOptionsAction> {
        self.ensure_layout();
        let mut actions = Vec::new();
        let release_entry = {
            let layout = self.layout();
            entry_at(layout, position)
        };
        if let Some(pressed) = self.pressed_entry.take() {
            if Some(pressed) == release_entry {
                actions.push(ControlOptionsAction::RequestRebind(pressed));
            }
        }

        let release_button = {
            let layout = self.layout();
            footer_button_at(layout, position)
        };
        if let Some(pressed) = self.pressed_footer.take() {
            if Some(pressed) == release_button {
                match pressed {
                    FooterButtonKind::ResetAll => actions.push(ControlOptionsAction::ResetAll),
                    FooterButtonKind::Back => actions.push(ControlOptionsAction::Close),
                }
            }
        }
        actions
    }

    pub fn handle_key_down(&mut self, key: KeyCode) -> Vec<ControlOptionsAction> {
        match key {
            KeyCode::Up => self
                .move_selection(-1)
                .map(|idx| vec![ControlOptionsAction::SelectionChanged(idx)])
                .unwrap_or_default(),
            KeyCode::Down => self
                .move_selection(1)
                .map(|idx| vec![ControlOptionsAction::SelectionChanged(idx)])
                .unwrap_or_default(),
            KeyCode::Enter | KeyCode::Space => self
                .selected_index
                .map(|idx| vec![ControlOptionsAction::RequestRebind(idx)])
                .unwrap_or_default(),
            KeyCode::Escape => vec![ControlOptionsAction::Close],
            _ => Vec::new(),
        }
    }

    pub fn handle_key_up(&mut self, _key: KeyCode) -> Vec<ControlOptionsAction> {
        Vec::new()
    }

    pub fn render(&mut self, surface: &mut Surface) {
        let instructions = self.instructions_text();
        let waiting = self.waiting_for_rebind.is_some();
        self.ensure_layout();
        let layout = self.layout();
        draw_panel(surface, layout);

        let title_rect = GuiRect::new(
            layout.content_left,
            layout.panel.origin.y + PANEL_TOP_PADDING - 38.0,
            layout.content_width,
            32.0,
        );
        draw_text(
            surface,
            &title_rect,
            "Controls",
            TEXT_PRIMARY_COLOR,
            TITLE_FONT_SIZE,
            0.0,
            self.font.as_ref(),
        );

        for (index, rect) in layout.item_rects.iter().enumerate() {
            let mut background = ROW_BACKGROUND_COLOR;
            if Some(index) == self.waiting_for_rebind {
                background = ROW_WAITING_COLOR;
            } else if Some(index) == self.selected_index {
                background = ROW_SELECTED_COLOR;
            }
            fill_rect(surface, rect, background);

            let text_rect = rect.inset_by(12.0, 6.0);
            let label = self
                .items
                .get(index)
                .map(|item| item.label.as_str())
                .unwrap_or("");
            draw_text(
                surface,
                &text_rect,
                label,
                TEXT_PRIMARY_COLOR,
                ROW_FONT_SIZE,
                0.0,
                self.font.as_ref(),
            );

            let key_rect = GuiRect::new(
                rect.origin.x + rect.size.width - KEY_COLUMN_WIDTH,
                rect.origin.y + 4.0,
                KEY_COLUMN_WIDTH - 12.0,
                rect.size.height - 8.0,
            );

            let key_color = self
                .items
                .get(index)
                .map(|item| {
                    if item.is_default {
                        KEY_BACKGROUND_COLOR
                    } else {
                        KEY_CUSTOM_COLOR
                    }
                })
                .unwrap_or(KEY_BACKGROUND_COLOR);

            fill_rect(surface, &key_rect, key_color);

            if let Some(item) = self.items.get(index) {
                draw_text(
                    surface,
                    &key_rect,
                    &item.key_label,
                    TEXT_PRIMARY_COLOR,
                    KEY_FONT_SIZE,
                    6.0,
                    self.font.as_ref(),
                );
            }
        }

        fill_rect(surface, &layout.instructions_rect, FOOTER_BACKGROUND_COLOR);
        let instruction_color = if waiting {
            TEXT_WARNING_COLOR
        } else {
            TEXT_SECONDARY_COLOR
        };
        draw_text(
            surface,
            &layout.instructions_rect,
            &instructions,
            instruction_color,
            INSTRUCTION_FONT_SIZE,
            6.0,
            self.font.as_ref(),
        );

        let hovered_footer = self.hovered_footer;
        for button in &layout.footer_buttons {
            let mut color = FOOTER_BUTTON_COLOR;
            if Some(button.kind) == hovered_footer {
                color = FOOTER_BUTTON_HOVER_COLOR;
            }
            fill_rect(surface, &button.rect, color);
            draw_text(
                surface,
                &button.rect,
                button.label,
                TEXT_PRIMARY_COLOR,
                FOOTER_FONT_SIZE,
                8.0,
                self.font.as_ref(),
            );
        }
    }

    fn move_selection(&mut self, delta: isize) -> Option<usize> {
        if self.items.is_empty() {
            return None;
        }
        let len = self.items.len();
        let mut index = self.selected_index.unwrap_or(0);
        let mut attempts = 0usize;
        loop {
            let raw = index as isize + delta;
            if raw < 0 {
                index = len.saturating_sub(1);
            } else {
                index = (raw as usize) % len;
            }
            attempts += 1;
            if attempts >= len || self.items.get(index).is_some() {
                break;
            }
        }
        self.selected_index = Some(index);
        Some(index)
    }

    fn ensure_layout(&mut self) {
        if self.layout.is_none() {
            let width = self.size.width.max(360.0);
            let height = self.size.height.max(320.0);

            let max_panel_width = (width - 40.0).max(360.0);
            let preferred_width = width * 0.72;
            let panel_width = preferred_width.clamp(380.0, max_panel_width);

            let rows_height = if self.items.is_empty() {
                0.0
            } else {
                let count = self.items.len() as f32;
                count * ROW_HEIGHT + (count - 1.0) * ROW_SPACING
            };

            let min_panel_height = 320.0;
            let content_height = PANEL_TOP_PADDING
                + TITLE_BOTTOM_MARGIN
                + rows_height
                + INSTRUCTION_HEIGHT
                + FOOTER_HEIGHT
                + 16.0;
            let panel_height = content_height.clamp(min_panel_height, height - 24.0);

            let panel_origin_x = ((width - panel_width).max(0.0)) / 2.0;
            let panel_origin_y = ((height - panel_height).max(0.0)) / 2.0;
            let panel = GuiRect::new(panel_origin_x, panel_origin_y, panel_width, panel_height);

            let content_left = panel.origin.x + PANEL_HORIZONTAL_PADDING;
            let content_width = panel.size.width - PANEL_HORIZONTAL_PADDING * 2.0;

            let mut current_y = panel.origin.y + PANEL_TOP_PADDING + TITLE_BOTTOM_MARGIN;
            let mut item_rects = Vec::with_capacity(self.items.len());
            for _ in &self.items {
                let rect = GuiRect::new(content_left, current_y, content_width, ROW_HEIGHT);
                item_rects.push(rect);
                current_y += ROW_HEIGHT + ROW_SPACING;
            }

            let instructions_rect = GuiRect::new(
                content_left,
                panel.origin.y + panel.size.height - FOOTER_HEIGHT - INSTRUCTION_HEIGHT - 8.0,
                content_width,
                INSTRUCTION_HEIGHT,
            );

            let button_width = (content_width / 2.0 - 12.0).clamp(120.0, 220.0);
            let button_height = 36.0;
            let button_y = panel.origin.y + panel.size.height - FOOTER_HEIGHT
                + (FOOTER_HEIGHT - button_height) / 2.0;
            let reset_rect = GuiRect::new(content_left, button_y, button_width, button_height);
            let back_rect = GuiRect::new(
                content_left + content_width - button_width,
                button_y,
                button_width,
                button_height,
            );

            let footer_buttons = vec![
                FooterButton::new(reset_rect, FooterButtonKind::ResetAll),
                FooterButton::new(back_rect, FooterButtonKind::Back),
            ];

            self.layout = Some(Layout {
                panel,
                content_left,
                content_width,
                item_rects,
                instructions_rect,
                footer_buttons,
            });
        }
    }

    fn layout(&self) -> &Layout {
        self.layout
            .as_ref()
            .expect("control options layout should be computed")
    }

    fn instructions_text(&self) -> String {
        if let Some(index) = self.waiting_for_rebind {
            let label = self
                .items
                .get(index)
                .map(|item| item.label.as_str())
                .unwrap_or("control");
            format!("Press a key for “{}”… (Escape to cancel)", label)
        } else {
            "Enter/Click: Rebind    Backspace: Reset binding    R: Reset all    Escape: Back"
                .to_string()
        }
    }
}

fn draw_panel(surface: &mut Surface, layout: &Layout) {
    let shadow = GuiRect::new(
        layout.panel.origin.x + 4.0,
        layout.panel.origin.y + 4.0,
        layout.panel.size.width,
        layout.panel.size.height,
    );
    fill_rect(surface, &shadow, PANEL_SHADOW_COLOR);
    fill_rect(surface, &layout.panel, PANEL_BACKGROUND_COLOR);
}

fn entry_at(layout: &Layout, point: GuiPoint) -> Option<usize> {
    layout
        .item_rects
        .iter()
        .position(|rect| rect.contains(point))
}

fn footer_button_at(layout: &Layout, point: GuiPoint) -> Option<FooterButtonKind> {
    layout
        .footer_buttons
        .iter()
        .find(|button| button.rect.contains(point))
        .map(|button| button.kind)
}
