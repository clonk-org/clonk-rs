use lc_engine::{CommandKind, ControlCommand};
use lc_graphics::{Color, Rect, Surface, TextFont};

const BACKDROP_COLOR: Color = Color::new(0, 0, 0, 160);
const PANEL_COLOR: Color = Color::new(18, 28, 48, 232);
const PANEL_BORDER: Color = Color::new(210, 224, 255, 220);
const HIGHLIGHT_COLOR: Color = Color::new(58, 92, 164, 220);
const TITLE_COLOR: Color = Color::opaque(240, 244, 255);
const TEXT_COLOR: Color = Color::opaque(214, 220, 235);
const DISABLED_TEXT_COLOR: Color = Color::opaque(140, 148, 160);
const HIGHLIGHT_TEXT_COLOR: Color = Color::opaque(255, 255, 255);

const PANEL_WIDTH_MIN: i32 = 340;
const PANEL_WIDTH_MAX: i32 = 720;
const PANEL_PADDING: i32 = 24;
const TITLE_GAP: i32 = 28;
const ITEM_HEIGHT: i32 = 46;
const ITEM_SPACING: i32 = 6;
const TITLE_FONT_SIZE: f32 = 22.0;
const ITEM_FONT_SIZE: f32 = 18.0;
const DESCRIPTION_FONT_SIZE: f32 = 14.0;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum IngameMenuAction {
    Resume,
    QuickSave,
    QuickLoad,
    SaveGame,
    LoadGame,
    AbortToMenu,
}

#[derive(Clone, Debug)]
struct IngameMenuItem {
    label: String,
    description: Option<String>,
    action: IngameMenuAction,
    enabled: bool,
}

impl IngameMenuItem {
    fn new(
        label: impl Into<String>,
        description: Option<impl Into<String>>,
        action: IngameMenuAction,
        enabled: bool,
    ) -> Self {
        Self {
            label: label.into(),
            description: description.map(Into::into),
            action,
            enabled,
        }
    }
}

pub struct IngameMenuState {
    items: Vec<IngameMenuItem>,
    selected: usize,
}

impl IngameMenuState {
    pub fn new(has_quick_save: bool, has_saved_games: bool) -> Self {
        let items = vec![
            IngameMenuItem::new(
                "Resume Game",
                Some("Close the menu and continue playing."),
                IngameMenuAction::Resume,
                true,
            ),
            IngameMenuItem::new(
                "Quick Save",
                Some("Store the current game state in the quick save slot."),
                IngameMenuAction::QuickSave,
                true,
            ),
            IngameMenuItem::new(
                "Quick Load",
                Some("Restore the most recent quick save."),
                IngameMenuAction::QuickLoad,
                has_quick_save,
            ),
            IngameMenuItem::new(
                "Save Game",
                Some("Create or overwrite a named save slot."),
                IngameMenuAction::SaveGame,
                true,
            ),
            IngameMenuItem::new(
                "Load Game",
                Some("Resume from an existing saved game."),
                IngameMenuAction::LoadGame,
                has_saved_games,
            ),
            IngameMenuItem::new(
                "Abort to Startup Menu",
                Some("Stop the current game and return to the startup browser."),
                IngameMenuAction::AbortToMenu,
                true,
            ),
        ];
        let selected = items
            .iter()
            .enumerate()
            .find(|(_, item)| item.enabled)
            .map(|(index, _)| index)
            .unwrap_or(0);
        Self { items, selected }
    }

    pub fn update_save_options(&mut self, quick_available: bool, load_available: bool) {
        for item in &mut self.items {
            match item.action {
                IngameMenuAction::QuickLoad => item.enabled = quick_available,
                IngameMenuAction::LoadGame => item.enabled = load_available,
                _ => {}
            }
        }
        if !self
            .items
            .get(self.selected)
            .map_or(false, |item| item.enabled)
        {
            self.advance_selection(1);
        }
    }

    pub fn handle_command(
        &mut self,
        command: ControlCommand,
        kind: CommandKind,
    ) -> Option<IngameMenuAction> {
        if !matches!(
            kind,
            CommandKind::Press | CommandKind::Single | CommandKind::Double
        ) {
            return None;
        }
        match command {
            ControlCommand::MenuUp | ControlCommand::MenuLeft => {
                self.advance_selection(-1);
                None
            }
            ControlCommand::MenuDown | ControlCommand::MenuRight => {
                self.advance_selection(1);
                None
            }
            ControlCommand::MenuSelect
            | ControlCommand::MenuEnter
            | ControlCommand::MenuEnterAll => self.activate_selected(),
            ControlCommand::MenuClose => Some(IngameMenuAction::Resume),
            ControlCommand::MenuShowText => None,
            _ => None,
        }
    }

    pub fn render(&self, surface: &mut Surface, font: &dyn TextFont) {
        let width = surface.width() as i32;
        let height = surface.height() as i32;
        if width <= 0 || height <= 0 {
            return;
        }

        fill_rect(surface, surface.bounds(), BACKDROP_COLOR);

        let mut panel_width = (width as f32 * 0.5).round() as i32;
        panel_width = panel_width.clamp(PANEL_WIDTH_MIN, PANEL_WIDTH_MAX);
        panel_width = panel_width
            .min(width - PANEL_PADDING * 2)
            .max(PANEL_WIDTH_MIN);
        let items_area_height =
            (self.items.len() as i32).saturating_mul(ITEM_HEIGHT + ITEM_SPACING) - ITEM_SPACING;
        let panel_height = (PANEL_PADDING * 2) + TITLE_GAP + items_area_height.max(ITEM_HEIGHT);
        let panel_x = (width - panel_width) / 2;
        let panel_y = (height - panel_height) / 2;

        let panel_rect = Rect::new(panel_x, panel_y, panel_width as u32, panel_height as u32);
        fill_rect(surface, panel_rect, PANEL_COLOR);
        draw_border(surface, panel_rect, PANEL_BORDER);

        let title_x = panel_x + PANEL_PADDING;
        let mut cursor_y = panel_y + PANEL_PADDING;
        font.draw_text(
            surface,
            title_x as f32,
            cursor_y as f32,
            "Player Menu",
            TITLE_FONT_SIZE,
            TITLE_COLOR,
        );

        cursor_y += TITLE_GAP;
        for (index, item) in self.items.iter().enumerate() {
            let row_rect = Rect::new(
                panel_x + PANEL_PADDING,
                cursor_y,
                (panel_width - PANEL_PADDING * 2) as u32,
                ITEM_HEIGHT as u32,
            );
            if index == self.selected {
                fill_rect(surface, row_rect, HIGHLIGHT_COLOR);
            }
            let label_color = if !item.enabled {
                DISABLED_TEXT_COLOR
            } else if index == self.selected {
                HIGHLIGHT_TEXT_COLOR
            } else {
                TEXT_COLOR
            };

            font.draw_text(
                surface,
                (row_rect.x + 12) as f32,
                (row_rect.y + 10) as f32,
                &item.label,
                ITEM_FONT_SIZE,
                label_color,
            );

            if let Some(desc) = item.description.as_deref() {
                let desc_color = if !item.enabled {
                    DISABLED_TEXT_COLOR
                } else {
                    TEXT_COLOR
                };
                font.draw_text(
                    surface,
                    (row_rect.x + 12) as f32,
                    (row_rect.y + 28) as f32,
                    desc,
                    DESCRIPTION_FONT_SIZE,
                    desc_color,
                );
            }

            cursor_y += ITEM_HEIGHT + ITEM_SPACING;
        }
    }

    pub fn activate_selected(&self) -> Option<IngameMenuAction> {
        self.items
            .get(self.selected)
            .filter(|item| item.enabled)
            .map(|item| item.action)
    }

    fn advance_selection(&mut self, delta: i32) {
        if self.items.is_empty() {
            self.selected = 0;
            return;
        }
        let len = self.items.len() as i32;
        let mut next = self.selected as i32;
        for _ in 0..len {
            next = (next + delta).rem_euclid(len);
            if self.items[next as usize].enabled {
                self.selected = next as usize;
                return;
            }
        }
        self.selected = next.rem_euclid(len) as usize;
    }
}

fn fill_rect(surface: &mut Surface, rect: Rect, color: Color) {
    if let Some(clipped) = rect.intersection(surface.bounds()) {
        for y in clipped.y..(clipped.y + clipped.height as i32) {
            for x in clipped.x..(clipped.x + clipped.width as i32) {
                let result = if color.a == 255 {
                    surface.set_pixel(x as u32, y as u32, color)
                } else {
                    surface.blend_pixel(x as u32, y as u32, color)
                };
                if result.is_err() {
                    break;
                }
            }
        }
    }
}

fn draw_border(surface: &mut Surface, rect: Rect, color: Color) {
    if rect.width == 0 || rect.height == 0 {
        return;
    }
    let top = Rect::new(rect.x, rect.y, rect.width, 1);
    let bottom = Rect::new(rect.x, rect.y + rect.height as i32 - 1, rect.width, 1);
    let left = Rect::new(rect.x, rect.y, 1, rect.height);
    let right = Rect::new(rect.x + rect.width as i32 - 1, rect.y, 1, rect.height);
    fill_rect(surface, top, color);
    fill_rect(surface, bottom, color);
    fill_rect(surface, left, color);
    fill_rect(surface, right, color);
}
