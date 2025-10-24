use crate::{draw_image, draw_text, fill_rect, GuiPoint, KeyCode};
use lc_graphics::{Color, Surface, TextFont};
use lc_gui::{ButtonTextures, Rect as GuiRect, Size as GuiSize};
use std::sync::Arc;

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
    textures: Option<ButtonTextures>,
    buttons: Vec<MenuButton>,
    pointer_position: Option<GuiPoint>,
    pressed_index: Option<usize>,
    selected_index: Option<usize>,
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
            MenuButton::new("Local Game", MainMenuItem::LocalGame),
            MenuButton::new("Network Game", MainMenuItem::NetworkGame),
            MenuButton::new("Player Selection", MainMenuItem::PlayerSelection),
            MenuButton::new("Options", MainMenuItem::Options),
            MenuButton::new("About", MainMenuItem::About),
            MenuButton::new("Quit", MainMenuItem::Quit),
        ];
        Self {
            font,
            textures,
            buttons,
            pointer_position: None,
            pressed_index: None,
            selected_index: None,
            layout: Vec::new(),
            size: GuiSize::new(0.0, 0.0),
        }
    }

    pub fn resize(&mut self, width: f32, height: f32) {
        self.size = GuiSize::new(width.max(1.0), height.max(1.0));
        self.layout = self.compute_layout();
    }

    pub fn pointer_position(&self) -> Option<GuiPoint> {
        self.pointer_position
    }

    pub fn set_pointer_position(&mut self, position: Option<GuiPoint>) {
        self.pointer_position = position;
        if let Some(point) = position {
            if let Some(index) = self.hit_test(point) {
                self.selected_index = Some(index);
            }
        }
    }

    pub fn pointer_left(&mut self) {
        self.pointer_position = None;
        self.pressed_index = None;
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
            }
        }
    }

    pub fn handle_pointer_move(&mut self, position: GuiPoint) -> Vec<MainMenuAction> {
        self.pointer_position = Some(position);
        let mut actions = Vec::new();
        if let Some(index) = self.hit_test(position) {
            if self.selected_index != Some(index) {
                self.selected_index = Some(index);
                let item = self.buttons[index].item;
                actions.push(MainMenuAction::SelectionChanged(item));
            }
        } else if self.selected_index.is_some() {
            self.selected_index = None;
        }
        actions
    }

    pub fn handle_pointer_down(&mut self, position: GuiPoint) -> Vec<MainMenuAction> {
        if let Some(index) = self.hit_test(position) {
            if self.buttons[index].enabled {
                self.pressed_index = Some(index);
            }
        }
        Vec::new()
    }

    pub fn handle_pointer_up(&mut self, position: GuiPoint) -> Vec<MainMenuAction> {
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
            KeyCode::Up | KeyCode::Left => self.move_selection(-1),
            KeyCode::Down | KeyCode::Right => self.move_selection(1),
            KeyCode::Enter | KeyCode::Space => {
                if let Some(index) = self.selected_index {
                    if self.buttons[index].enabled {
                        return vec![MainMenuAction::Activate(self.buttons[index].item)];
                    }
                }
                Vec::new()
            }
            _ => Vec::new(),
        }
    }

    pub fn handle_key_up(&mut self, _key: KeyCode) -> Vec<MainMenuAction> {
        Vec::new()
    }

    pub fn render(&mut self, surface: &mut Surface, participants_label: &str) {
        if self.layout.is_empty() {
            self.layout = self.compute_layout();
        }

        let panel_rect = self.panel_rect();
        if let Some(rect) = panel_rect {
            let backdrop = GuiRect::new(
                rect.origin.x + 4.0,
                rect.origin.y + 4.0,
                rect.size.width,
                rect.size.height,
            );
            fill_rect(surface, &backdrop, Color::new(0, 0, 0, 120));
            fill_rect(surface, &rect, Color::new(16, 28, 52, 235));
        }

        for (index, rect) in self.layout.iter().enumerate() {
            let state = ButtonVisualState::from_indices(
                index,
                self.selected_index,
                self.pressed_index,
                self.buttons[index].enabled,
            );
            self.draw_button(surface, rect, &self.buttons[index], state);
        }

        if let Some(rect) = panel_rect {
            let footer_height = 28.0_f32.min(rect.size.height * 0.18);
            let footer_rect = GuiRect::new(
                rect.origin.x + 12.0,
                rect.origin.y + rect.size.height - footer_height - 12.0,
                rect.size.width - 24.0,
                footer_height,
            );
            fill_rect(surface, &footer_rect, Color::new(8, 14, 28, 210));
            draw_text(
                surface,
                &footer_rect,
                participants_label,
                Color::new(220, 220, 240, 255),
                footer_rect.size.height * 0.55,
                8.0,
                self.font.as_ref(),
            );
        }
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
    ) {
        if let Some(textures) = self.textures.as_ref() {
            let image = match state {
                ButtonVisualState::Disabled => {
                    textures.disabled.as_ref().unwrap_or(&textures.normal)
                }
                ButtonVisualState::Pressed => &textures.pressed,
                ButtonVisualState::Selected => &textures.selected,
                ButtonVisualState::Normal => &textures.normal,
            };
            draw_image(surface, rect, image);
        } else {
            let color = match state {
                ButtonVisualState::Disabled => Color::new(50, 60, 72, 220),
                ButtonVisualState::Pressed => Color::new(44, 70, 120, 240),
                ButtonVisualState::Selected => Color::new(54, 90, 160, 240),
                ButtonVisualState::Normal => Color::new(36, 62, 104, 230),
            };
            fill_rect(surface, rect, color);
        }

        let text_color = if button.enabled {
            Color::new(236, 242, 255, 255)
        } else {
            Color::new(164, 172, 192, 255)
        };
        draw_text(
            surface,
            rect,
            button.label,
            text_color,
            (rect.size.height * 0.48).clamp(18.0, 32.0),
            rect.size.height * 0.22,
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

        let width = self.size.width.max(1.0);
        let height = self.size.height.max(1.0);
        let panel_margin = (width * 0.04).clamp(16.0, 48.0);
        let panel_width_nominal = (width * 0.34).clamp(280.0, 420.0);
        let available_width = (width - panel_margin * 2.0).max(200.0);
        let panel_width = panel_width_nominal
            .min(available_width)
            .max(220.0f32.min(available_width));
        let button_height = (height * 0.075).clamp(46.0, 72.0);
        let spacing = button_height * 0.18;

        let total_buttons_height =
            button_height * self.buttons.len() as f32 + spacing * (self.buttons.len() as f32 - 1.0);
        let available_height = (height - panel_margin * 2.0).max(button_height * 2.5);
        let min_panel_height = (height * 0.55)
            .max(button_height * 2.5)
            .min(available_height);
        let desired_height = total_buttons_height + button_height * 0.85;
        let panel_height = desired_height.max(min_panel_height).min(available_height);

        let panel_x = width - panel_width - panel_margin;
        let panel_y = height / 2.0 - panel_height / 2.0;

        let mut rects = Vec::with_capacity(self.buttons.len());
        let start_y = panel_y + (panel_height - total_buttons_height) * 0.5;
        for (idx, _) in self.buttons.iter().enumerate() {
            let y = start_y + idx as f32 * (button_height + spacing);
            rects.push(GuiRect::new(
                panel_x + 24.0,
                y,
                panel_width - 48.0,
                button_height,
            ));
        }
        rects
    }

    fn panel_rect(&self) -> Option<GuiRect> {
        if self.layout.is_empty() {
            return None;
        }
        let first = self.layout.first()?;
        let last = self.layout.last()?;
        let top = first.origin.y - (first.size.height * 0.5);
        let bottom = last.origin.y + last.size.height + (last.size.height * 0.85);
        let height = (bottom - top).max(0.0);
        Some(GuiRect::new(
            first.origin.x - 24.0,
            top,
            first.size.width + 48.0,
            height,
        ))
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
