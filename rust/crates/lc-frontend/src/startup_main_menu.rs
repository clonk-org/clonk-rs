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
            // Labels match the C++ main menu (IDS_BTN_LOCALGAME="&Start Game",
            // IDS_DLG_NETSTART="Start Network Game" in System.c4g/LanguageUS.txt).
            MenuButton::new("Start Game", MainMenuItem::LocalGame),
            MenuButton::new("Start Network Game", MainMenuItem::NetworkGame),
            MenuButton::new("Player Selection", MainMenuItem::PlayerSelection),
            MenuButton::new("Options", MainMenuItem::Options),
            MenuButton::new("About", MainMenuItem::About),
            MenuButton::new("Exit", MainMenuItem::Quit),
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

        // C++ C4StartupMainDlg draws the buttons directly over the loader
        // background — there is no panel backdrop or footer box.
        for (index, rect) in self.layout.iter().enumerate() {
            let state = ButtonVisualState::from_indices(
                index,
                self.selected_index,
                self.pressed_index,
                self.buttons[index].enabled,
            );
            self.draw_button(surface, rect, &self.buttons[index], state);
        }

        // Participants label: plain white, right-aligned near the bottom-right
        // (C++ Label at Wdt*39/40, Hgt*9/10, ARight; C4StartupMainDlg.cpp:69).
        let width = self.size.width.max(1.0);
        let height = self.size.height.max(1.0);
        let font_size = (height * 0.03).clamp(14.0, 30.0);
        let metrics = self.font.measure_text(participants_label, font_size);
        let label_rect = GuiRect::new(
            (width * 39.0 / 40.0 - metrics.width).max(0.0),
            height * 9.0 / 10.0,
            metrics.width,
            font_size,
        );
        draw_text(
            surface,
            &label_rect,
            participants_label,
            Color::new(255, 255, 255, 255),
            font_size,
            0.0,
            self.font.as_ref(),
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

        // C++ C4GUI button captions use C4GUI_ButtonFontClr = 0xffffff00 (yellow)
        // when active and C4GUI_InactCaptionFontClr = 0xffafafaf when disabled
        // (src/C4Gui.h:53-56, drawn at C4GuiButton.cpp:109).
        let text_color = if button.enabled {
            Color::new(0xff, 0xff, 0x00, 0xff)
        } else {
            Color::new(0xaf, 0xaf, 0xaf, 0xff)
        };
        // C++ renders the button caption centred within the plank; centre both
        // axes using the measured text extent.
        let font_size = (rect.size.height * 0.48).clamp(16.0, 32.0);
        let metrics = self.font.measure_text(button.label, font_size);
        let text_rect = GuiRect::new(
            rect.origin.x + ((rect.size.width - metrics.width) * 0.5).max(0.0),
            rect.origin.y + ((rect.size.height - font_size) * 0.5).max(0.0),
            metrics.width,
            font_size,
        );
        draw_text(
            surface,
            &text_rect,
            button.label,
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

        let width = self.size.width.max(1.0);
        let height = self.size.height.max(1.0);
        // Mirror C++ C4StartupMainDlg (C4StartupMainDlg.cpp:44-65): the buttons
        // occupy the right 2/5 of the width (`caMain.GetFromRight(Wdt*2/5)`),
        // inset by `Wdt/26` horizontally and `40 + Hgt/8` vertically, stacked from
        // the top at `C4GUI_BigButtonHgt` (40) each with `iButtonPadding` (2)
        // between. The fixed C++ logical sizes are scaled to the render height so
        // the layout matches across resolutions; the full-width planks float over
        // the loader background (no panel backdrop).
        let panel_x = width * 3.0 / 5.0;
        let panel_w = width * 2.0 / 5.0;
        let hmargin = width / 26.0;
        let button_x = panel_x + hmargin;
        let button_w = (panel_w - hmargin * 2.0).max(1.0);
        let button_height = (height * 0.062).clamp(34.0, 60.0);
        let padding = (button_height * 0.05).max(2.0);
        let top_margin = height / 8.0 + button_height;

        let mut rects = Vec::with_capacity(self.buttons.len());
        for (idx, _) in self.buttons.iter().enumerate() {
            let y = top_margin + idx as f32 * (button_height + padding);
            rects.push(GuiRect::new(button_x, y, button_w, button_height));
        }
        rects
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
