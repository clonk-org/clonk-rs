use std::path::PathBuf;

use clonk_engine::{CommandKind, ControlCommand};
use clonk_graphics::{Color, GammaRamp, Rect, Surface, TextFont};
use clonk_gui::ImageData;

const BACKDROP_COLOR: Color = Color::new(0, 0, 0, 180);
const PANEL_COLOR: Color = Color::new(18, 28, 48, 235);
const PANEL_BORDER: Color = Color::opaque(208, 220, 240);
const HIGHLIGHT_COLOR: Color = Color::new(58, 92, 164, 220);
const TITLE_COLOR: Color = Color::opaque(240, 244, 255);
const TEXT_COLOR: Color = Color::opaque(214, 222, 236);
const MUTED_TEXT_COLOR: Color = Color::opaque(144, 154, 170);
const HIGHLIGHT_TEXT_COLOR: Color = Color::opaque(255, 255, 255);

const PANEL_WIDTH: i32 = 760;
const PANEL_HEIGHT_MIN: i32 = 360;
const PANEL_PADDING: i32 = 24;
const TITLE_GAP: i32 = 28;
const ITEM_HEIGHT: i32 = 48;
const ITEM_SPACING: i32 = 6;
const TITLE_FONT_SIZE: f32 = 26.0;
const ITEM_FONT_SIZE: f32 = 18.0;
const DETAIL_FONT_SIZE: f32 = 14.0;
const PREVIEW_WIDTH: i32 = 320;
const PREVIEW_HEIGHT: i32 = 200;
const PREVIEW_GAP: i32 = 18;

#[derive(Clone, Debug)]
pub struct SaveEntry {
    pub display_name: String,
    pub scenario_title: String,
    pub saved_at_seconds: u64,
    pub saved_label: String,
    pub path: PathBuf,
    pub thumbnail: Option<ImageData>,
}

#[derive(Clone, Debug)]
enum SaveMenuItem {
    NewSlot { label: String },
    Entry(SaveEntry),
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum SaveBrowserMode {
    Save { suggested_label: String },
    Load,
}

#[derive(Clone, Debug)]
pub enum SaveBrowserAction {
    Close,
    SaveNew { label: String },
    SaveExisting { entry: SaveEntry },
    Load { entry: SaveEntry },
}

pub struct SaveBrowserState {
    mode: SaveBrowserMode,
    items: Vec<SaveMenuItem>,
    selected: Option<usize>,
}

impl SaveBrowserState {
    pub fn new(mode: SaveBrowserMode, mut entries: Vec<SaveEntry>) -> Self {
        entries.sort_by(|a, b| {
            b.saved_at_seconds
                .cmp(&a.saved_at_seconds)
                .then_with(|| a.display_name.cmp(&b.display_name))
        });

        let mut items = Vec::new();
        let mut selected = None;
        match mode {
            SaveBrowserMode::Save {
                ref suggested_label,
            } => {
                items.push(SaveMenuItem::NewSlot {
                    label: suggested_label.clone(),
                });
                selected = Some(0);
            }
            SaveBrowserMode::Load => {}
        }
        let start_index = items.len();
        for entry in entries {
            items.push(SaveMenuItem::Entry(entry));
        }
        if selected.is_none() && !items.is_empty() {
            selected = Some(start_index);
        }
        Self {
            mode,
            items,
            selected,
        }
    }

    pub fn mode(&self) -> &SaveBrowserMode {
        &self.mode
    }

    pub fn handle_command(
        &mut self,
        command: ControlCommand,
        kind: CommandKind,
    ) -> Option<SaveBrowserAction> {
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
            | ControlCommand::MenuEnterAll => self.activate(),
            ControlCommand::MenuClose => Some(SaveBrowserAction::Close),
            ControlCommand::MenuShowText => None,
            _ => None,
        }
    }

    pub fn render(&self, surface: &mut Surface, font: &dyn TextFont) {
        self.render_with_gamma(surface, font, None);
    }

    pub fn render_with_gamma(
        &self,
        surface: &mut Surface,
        font: &dyn TextFont,
        gamma: Option<&GammaRamp>,
    ) {
        let width = surface.width() as i32;
        let height = surface.height() as i32;
        if width <= 0 || height <= 0 {
            return;
        }

        fill_rect(surface, surface.bounds(), BACKDROP_COLOR, gamma);

        let panel_width = PANEL_WIDTH
            .min(width - PANEL_PADDING * 2)
            .max(PANEL_WIDTH / 2);
        let mut panel_height = PANEL_HEIGHT_MIN;
        let item_count = self.items.len() as i32;
        if item_count > 0 {
            let list_height = item_count * (ITEM_HEIGHT + ITEM_SPACING) - ITEM_SPACING;
            panel_height = panel_height.max(list_height + PANEL_PADDING * 2 + TITLE_GAP);
        }
        panel_height = panel_height
            .min(height - PANEL_PADDING * 2)
            .max(PANEL_HEIGHT_MIN);

        let panel_x = (width - panel_width) / 2;
        let panel_y = (height - panel_height) / 2;
        let panel_rect = Rect::new(panel_x, panel_y, panel_width as u32, panel_height as u32);
        fill_rect(surface, panel_rect, PANEL_COLOR, gamma);
        draw_border(surface, panel_rect, PANEL_BORDER, gamma);

        let title = match self.mode {
            SaveBrowserMode::Save { .. } => "Save Game",
            SaveBrowserMode::Load => "Load Game",
        };

        let title_x = panel_x + PANEL_PADDING;
        let mut cursor_y = panel_y + PANEL_PADDING;
        clonk_frontend::draw_text_with_gamma(
            font,
            surface,
            title_x as f32,
            cursor_y as f32,
            title,
            TITLE_FONT_SIZE,
            TITLE_COLOR,
            gamma,
        );

        cursor_y += TITLE_GAP;

        let list_width = panel_width - PANEL_PADDING * 2 - (PREVIEW_WIDTH + PREVIEW_GAP);
        let list_width = list_width.max(200);
        let mut preview: Option<&SaveEntry> = None;
        for (index, item) in self.items.iter().enumerate() {
            let item_rect = Rect::new(
                panel_x + PANEL_PADDING,
                cursor_y,
                list_width as u32,
                ITEM_HEIGHT as u32,
            );
            let is_selected = self.selected == Some(index);
            if is_selected {
                fill_rect(surface, item_rect, HIGHLIGHT_COLOR, gamma);
            }

            match item {
                SaveMenuItem::NewSlot { label } => {
                    let title = format!("Create New Save ({label})");
                    clonk_frontend::draw_text_with_gamma(
                        font,
                        surface,
                        (item_rect.x + 12) as f32,
                        (item_rect.y + 10) as f32,
                        &title,
                        ITEM_FONT_SIZE,
                        HIGHLIGHT_TEXT_COLOR,
                        gamma,
                    );
                    let description = "Create a new save file with the suggested name.";
                    clonk_frontend::draw_text_with_gamma(
                        font,
                        surface,
                        (item_rect.x + 12) as f32,
                        (item_rect.y + 28) as f32,
                        description,
                        DETAIL_FONT_SIZE,
                        MUTED_TEXT_COLOR,
                        gamma,
                    );
                }
                SaveMenuItem::Entry(entry) => {
                    let label_color = if is_selected {
                        HIGHLIGHT_TEXT_COLOR
                    } else {
                        TEXT_COLOR
                    };
                    clonk_frontend::draw_text_with_gamma(
                        font,
                        surface,
                        (item_rect.x + 12) as f32,
                        (item_rect.y + 8) as f32,
                        &entry.display_name,
                        ITEM_FONT_SIZE,
                        label_color,
                        gamma,
                    );
                    let details = format!("{} • {}", entry.scenario_title, entry.saved_label);
                    clonk_frontend::draw_text_with_gamma(
                        font,
                        surface,
                        (item_rect.x + 12) as f32,
                        (item_rect.y + 28) as f32,
                        &details,
                        DETAIL_FONT_SIZE,
                        MUTED_TEXT_COLOR,
                        gamma,
                    );
                    if is_selected {
                        preview = Some(entry);
                    }
                }
            }

            cursor_y += ITEM_HEIGHT + ITEM_SPACING;
        }

        if self.items.is_empty() {
            let message = match self.mode {
                SaveBrowserMode::Save { .. } => "No save slots available.",
                SaveBrowserMode::Load => "No saved games found.",
            };
            let text_x = (panel_x + PANEL_PADDING + 12) as f32;
            let text_y = (panel_y + PANEL_PADDING + TITLE_GAP + 12) as f32;
            clonk_frontend::draw_text_with_gamma(
                font,
                surface,
                text_x,
                text_y,
                message,
                ITEM_FONT_SIZE,
                MUTED_TEXT_COLOR,
                gamma,
            );
        }

        if let Some(entry) = preview {
            let preview_x = panel_x + panel_width - PANEL_PADDING - PREVIEW_WIDTH;
            let preview_y = panel_y + PANEL_PADDING;
            let preview_rect = Rect::new(
                preview_x,
                preview_y,
                PREVIEW_WIDTH as u32,
                PREVIEW_HEIGHT as u32,
            );
            fill_rect(surface, preview_rect, Color::new(12, 24, 40, 220), gamma);
            draw_border(surface, preview_rect, PANEL_BORDER, gamma);

            if let Some(thumbnail) = entry.thumbnail.as_ref() {
                blit_image(surface, preview_rect, thumbnail, gamma);
            } else {
                let message = "No thumbnail available";
                let metrics = font.measure_text(message, DETAIL_FONT_SIZE);
                let text_x = (preview_rect.x as f32 + PREVIEW_WIDTH as f32 / 2.0
                    - metrics.width / 2.0)
                    .max(preview_rect.x as f32 + 8.0);
                let text_y = preview_rect.y as f32 + PREVIEW_HEIGHT as f32 / 2.0 - 8.0;
                clonk_frontend::draw_text_with_gamma(
                    font,
                    surface,
                    text_x,
                    text_y,
                    message,
                    DETAIL_FONT_SIZE,
                    MUTED_TEXT_COLOR,
                    gamma,
                );
            }
        }
    }

    fn advance_selection(&mut self, delta: i32) {
        let Some(current) = self.selected else {
            if !self.items.is_empty() {
                self.selected = Some(0);
            }
            return;
        };
        if self.items.is_empty() {
            self.selected = None;
            return;
        }
        let len = self.items.len() as i32;
        let mut next = current as i32;
        for _ in 0..len {
            next = (next + delta).rem_euclid(len);
            if self.items.get(next as usize).is_some() {
                self.selected = Some(next as usize);
                return;
            }
        }
        self.selected = Some(((current as i32 + delta).rem_euclid(len)) as usize);
    }

    fn activate(&self) -> Option<SaveBrowserAction> {
        let index = self.selected?;
        match (&self.mode, self.items.get(index)?) {
            (SaveBrowserMode::Save { .. }, SaveMenuItem::NewSlot { label }) => {
                Some(SaveBrowserAction::SaveNew {
                    label: label.clone(),
                })
            }
            (_, SaveMenuItem::Entry(entry)) => match self.mode {
                SaveBrowserMode::Save { .. } => Some(SaveBrowserAction::SaveExisting {
                    entry: entry.clone(),
                }),
                SaveBrowserMode::Load => Some(SaveBrowserAction::Load {
                    entry: entry.clone(),
                }),
            },
            _ => None,
        }
    }
}

fn fill_rect(surface: &mut Surface, rect: Rect, color: Color, gamma: Option<&GammaRamp>) {
    clonk_frontend::draw_color_rect(surface, rect, color, gamma);
}

fn draw_border(surface: &mut Surface, rect: Rect, color: Color, gamma: Option<&GammaRamp>) {
    if rect.width == 0 || rect.height == 0 {
        return;
    }
    let top = Rect::new(rect.x, rect.y, rect.width, 1);
    let bottom = Rect::new(rect.x, rect.y + rect.height as i32 - 1, rect.width, 1);
    let left = Rect::new(rect.x, rect.y, 1, rect.height);
    let right = Rect::new(rect.x + rect.width as i32 - 1, rect.y, 1, rect.height);
    fill_rect(surface, top, color, gamma);
    fill_rect(surface, bottom, color, gamma);
    fill_rect(surface, left, color, gamma);
    fill_rect(surface, right, color, gamma);
}

fn blit_image(surface: &mut Surface, rect: Rect, image: &ImageData, gamma: Option<&GammaRamp>) {
    clonk_frontend::draw_image_with_gamma(
        surface,
        &clonk_gui::Rect::new(
            rect.x as f32,
            rect.y as f32,
            rect.width as f32,
            rect.height as f32,
        ),
        image,
        gamma,
    );
}
