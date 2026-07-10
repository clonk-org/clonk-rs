use lc_graphics::{Color, Rect, Surface, TextFont};

const BACKDROP_COLOR: Color = Color::new(8, 12, 24, 210);
const PANEL_COLOR: Color = Color::new(22, 32, 52, 240);
const PANEL_BORDER: Color = Color::opaque(198, 210, 232);
const TITLE_COLOR: Color = Color::opaque(242, 246, 255);
const SUBTITLE_COLOR: Color = Color::opaque(200, 212, 236);
const HEADER_COLOR: Color = Color::opaque(188, 204, 230);
const TEXT_COLOR: Color = Color::opaque(226, 234, 248);
const MUTED_TEXT_COLOR: Color = Color::opaque(164, 176, 196);
const LOCAL_ROW_HIGHLIGHT: Color = Color::new(48, 72, 124, 185);
const HEADER_RULE_COLOR: Color = Color::opaque(84, 108, 156);
const COLOR_SWATCH_BORDER: Color = Color::opaque(28, 38, 58);
const BUTTON_COLOR: Color = Color::opaque(48, 62, 88);
const BUTTON_SELECTED_COLOR: Color = Color::opaque(70, 98, 152);
const BUTTON_BORDER_COLOR: Color = Color::opaque(154, 174, 208);

const PANEL_WIDTH: u32 = 760;
const PANEL_HEIGHT_MIN: u32 = 320;
const PANEL_PADDING: i32 = 28;
const TITLE_FONT_SIZE: f32 = 30.0;
const SUBTITLE_FONT_SIZE: f32 = 20.0;
const HEADER_FONT_SIZE: f32 = 16.0;
const ROW_FONT_SIZE: f32 = 18.0;
const FOOTER_FONT_SIZE: f32 = 14.0;
const BUTTON_FONT_SIZE: f32 = 16.0;
const ROW_HEIGHT: i32 = 40;
const GAP_AFTER_TITLE: i32 = 14;
const GAP_AFTER_SUBTITLE: i32 = 20;
const GAP_AFTER_HEADER: i32 = 12;
const GAP_BEFORE_FOOTER: i32 = 18;
const COLUMN_GAP: i32 = 14;
const OUTCOME_WIDTH: i32 = 130;
const STAT_COLUMN_WIDTH: i32 = 96;
const COLOR_SWATCH_SIZE: i32 = 16;
const BUTTON_HEIGHT: i32 = 32;
const BUTTON_GAP: i32 = 8;
const GAP_BEFORE_BUTTONS: i32 = 18;
const GAP_AFTER_BUTTONS: i32 = 10;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum GameOverOutcome {
    Victory,
    Defeat,
    Observer,
}

impl GameOverOutcome {
    fn sort_rank(self) -> u8 {
        match self {
            GameOverOutcome::Victory => 0,
            GameOverOutcome::Defeat => 1,
            GameOverOutcome::Observer => 2,
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            GameOverOutcome::Victory => "Victory",
            GameOverOutcome::Defeat => "Defeat",
            GameOverOutcome::Observer => "Observer",
        }
    }

    pub fn label_color(self) -> Color {
        match self {
            GameOverOutcome::Victory => Color::opaque(132, 216, 156),
            GameOverOutcome::Defeat => Color::opaque(232, 128, 128),
            GameOverOutcome::Observer => MUTED_TEXT_COLOR,
        }
    }

    pub fn summary(self) -> &'static str {
        match self {
            GameOverOutcome::Victory => "Victory!",
            GameOverOutcome::Defeat => "Defeat",
            GameOverOutcome::Observer => "Observer",
        }
    }
}

#[derive(Clone, Debug)]
pub struct GameOverEntry {
    #[allow(dead_code)]
    pub player_id: i32,
    pub name: String,
    pub outcome: GameOverOutcome,
    pub wealth: i32,
    pub score: i32,
    pub value: i32,
    pub is_local: bool,
    pub color: Option<Color>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum GameOverAction {
    End,
    Continue,
    Restart,
    NextMission,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct NextMissionButton {
    pub label: String,
    pub description: String,
}

#[derive(Debug)]
struct GameOverButton {
    action: GameOverAction,
    label: String,
    description: String,
}

#[derive(Debug)]
pub struct GameOverState {
    title: String,
    subtitle: String,
    entries: Vec<GameOverEntry>,
    buttons: Vec<GameOverButton>,
    selected_button: usize,
    pointer_position: Option<(f32, f32)>,
}

impl GameOverState {
    pub fn new(title: String, entries: Vec<GameOverEntry>) -> Self {
        Self::with_next_mission(title, entries, u32::MAX, None)
    }

    pub fn with_next_mission(
        title: String,
        mut entries: Vec<GameOverEntry>,
        screen_width: u32,
        next_mission: Option<NextMissionButton>,
    ) -> Self {
        entries.sort_by(|left, right| {
            left.outcome
                .sort_rank()
                .cmp(&right.outcome.sort_rank())
                .then(left.name.cmp(&right.name))
        });

        let subtitle = if let Some(local) = entries.iter().find(|entry| entry.is_local) {
            local.outcome.summary().to_string()
        } else if entries.is_empty() {
            "Game Over".to_string()
        } else if entries
            .iter()
            .any(|entry| matches!(entry.outcome, GameOverOutcome::Victory))
        {
            "Victory!".to_string()
        } else if entries
            .iter()
            .all(|entry| matches!(entry.outcome, GameOverOutcome::Observer))
        {
            "Observer".to_string()
        } else {
            "Defeat".to_string()
        };

        let mut buttons = vec![
            GameOverButton {
                action: GameOverAction::End,
                label: "&End game".to_string(),
                description: "End the round.".to_string(),
            },
            GameOverButton {
                action: GameOverAction::Continue,
                label: "&Continue playing".to_string(),
                description: "Continue playing this round (with no further evaluation)."
                    .to_string(),
            },
        ];
        if next_mission.is_none() || screen_width >= 1280 {
            buttons.push(GameOverButton {
                action: GameOverAction::Restart,
                label: "&Restart".to_string(),
                description: "Play this scenario again.".to_string(),
            });
        }
        if let Some(next_mission) = next_mission {
            buttons.push(GameOverButton {
                action: GameOverAction::NextMission,
                label: next_mission.label,
                description: next_mission.description,
            });
        }

        Self {
            title,
            subtitle,
            entries,
            buttons,
            selected_button: 0,
            pointer_position: None,
        }
    }

    pub fn subtitle(&self) -> &str {
        &self.subtitle
    }

    #[allow(dead_code)]
    pub fn entries(&self) -> &[GameOverEntry] {
        &self.entries
    }

    pub fn actions(&self) -> Vec<GameOverAction> {
        self.buttons.iter().map(|button| button.action).collect()
    }

    pub fn selected_description(&self) -> &str {
        self.buttons
            .get(self.selected_button)
            .map(|button| button.description.as_str())
            .unwrap_or("")
    }

    pub fn move_selection(&mut self, delta: i32) {
        let count = self.buttons.len() as i32;
        if count == 0 {
            return;
        }
        self.selected_button = (self.selected_button as i32 + delta).rem_euclid(count) as usize;
    }

    pub fn activate_selected(&self) -> Option<GameOverAction> {
        self.buttons
            .get(self.selected_button)
            .map(|button| button.action)
    }

    pub fn handle_pointer_move(&mut self, x: f32, y: f32, surface_width: u32, surface_height: u32) {
        self.pointer_position = Some((x, y));
        if let Some(index) = self
            .button_rects(surface_width, surface_height)
            .iter()
            .position(|rect| point_in_rect(x, y, *rect))
        {
            self.selected_button = index;
        }
    }

    pub fn pointer_left(&mut self) {
        self.pointer_position = None;
    }

    pub fn activate_pointer_position(
        &self,
        surface_width: u32,
        surface_height: u32,
    ) -> Option<GameOverAction> {
        self.pointer_position
            .and_then(|(x, y)| self.activate_pointer(x, y, surface_width, surface_height))
    }

    pub fn activate_pointer(
        &self,
        x: f32,
        y: f32,
        surface_width: u32,
        surface_height: u32,
    ) -> Option<GameOverAction> {
        self.button_rects(surface_width, surface_height)
            .iter()
            .position(|rect| point_in_rect(x, y, *rect))
            .and_then(|index| self.buttons.get(index))
            .map(|button| button.action)
    }

    fn panel_rect(&self, surface_width: u32, surface_height: u32) -> Rect {
        let min_width = 360.min(surface_width);
        let panel_width = PANEL_WIDTH.min(surface_width).max(min_width);
        let rows = self.entries.len().max(1) as i32;
        let title_height = TITLE_FONT_SIZE.ceil() as i32;
        let subtitle_height = if self.subtitle.is_empty() {
            0
        } else {
            SUBTITLE_FONT_SIZE.ceil() as i32
        };
        let header_height = HEADER_FONT_SIZE.ceil() as i32;
        let footer_height = FOOTER_FONT_SIZE.ceil() as i32;

        let mut panel_height = PANEL_PADDING * 2 + title_height + GAP_AFTER_TITLE;
        if subtitle_height > 0 {
            panel_height += subtitle_height + GAP_AFTER_SUBTITLE;
        }
        panel_height += header_height + GAP_AFTER_HEADER + rows * ROW_HEIGHT;
        panel_height += GAP_BEFORE_BUTTONS + BUTTON_HEIGHT + GAP_AFTER_BUTTONS + footer_height;
        let panel_height = panel_height.max(PANEL_HEIGHT_MIN as i32) as u32;
        Rect::new(
            ((surface_width as i32 - panel_width as i32) / 2).max(0),
            ((surface_height as i32 - panel_height as i32) / 2).max(0),
            panel_width,
            panel_height,
        )
    }

    fn button_rects(&self, surface_width: u32, surface_height: u32) -> Vec<Rect> {
        let panel_rect = self.panel_rect(surface_width, surface_height);
        let content_left = panel_rect.x + PANEL_PADDING;
        let content_right = panel_rect.x + panel_rect.width as i32 - PANEL_PADDING;
        let footer_height = FOOTER_FONT_SIZE.ceil() as i32;
        let buttons_y = panel_rect.y + panel_rect.height as i32
            - PANEL_PADDING
            - footer_height
            - GAP_AFTER_BUTTONS
            - BUTTON_HEIGHT;
        let button_count = self.buttons.len().max(1) as i32;
        let button_width = ((content_right - content_left - BUTTON_GAP * (button_count - 1))
            / button_count)
            .max(1);
        self.buttons
            .iter()
            .enumerate()
            .map(|(index, _)| {
                Rect::new(
                    content_left + index as i32 * (button_width + BUTTON_GAP),
                    buttons_y,
                    button_width as u32,
                    BUTTON_HEIGHT as u32,
                )
            })
            .collect()
    }

    pub fn render(&self, surface: &mut Surface, font: &dyn TextFont) {
        if surface.width() == 0 || surface.height() == 0 {
            return;
        }

        let surface_rect = Rect::new(0, 0, surface.width(), surface.height());
        fill_rect(surface, surface_rect, BACKDROP_COLOR);

        let title_height = TITLE_FONT_SIZE.ceil() as i32;
        let subtitle_height = if self.subtitle.is_empty() {
            0
        } else {
            SUBTITLE_FONT_SIZE.ceil() as i32
        };
        let header_height = HEADER_FONT_SIZE.ceil() as i32;
        let panel_rect = self.panel_rect(surface.width(), surface.height());
        fill_rect(surface, panel_rect, PANEL_COLOR);
        draw_border(surface, panel_rect, PANEL_BORDER);

        let content_left = panel_rect.x + PANEL_PADDING;
        let content_right = panel_rect.x + panel_rect.width as i32 - PANEL_PADDING;
        let mut cursor_y = panel_rect.y + PANEL_PADDING;

        draw_text_centered(
            surface,
            font,
            &self.title,
            TITLE_FONT_SIZE,
            TITLE_COLOR,
            content_left,
            content_right,
            cursor_y,
        );
        cursor_y += title_height + GAP_AFTER_TITLE;

        if subtitle_height > 0 {
            draw_text_centered(
                surface,
                font,
                &self.subtitle,
                SUBTITLE_FONT_SIZE,
                SUBTITLE_COLOR,
                content_left,
                content_right,
                cursor_y,
            );
            cursor_y += subtitle_height + GAP_AFTER_SUBTITLE;
        }

        let mut column_right = content_right;
        let value_column_x = column_right - STAT_COLUMN_WIDTH;
        column_right = value_column_x - COLUMN_GAP;
        let score_column_x = column_right - STAT_COLUMN_WIDTH;
        column_right = score_column_x - COLUMN_GAP;
        let wealth_column_x = column_right - STAT_COLUMN_WIDTH;
        column_right = wealth_column_x - COLUMN_GAP;
        let outcome_column_x = column_right - OUTCOME_WIDTH;
        let name_column_x = content_left;

        draw_header(
            surface,
            font,
            cursor_y,
            name_column_x,
            outcome_column_x,
            wealth_column_x,
            score_column_x,
            value_column_x,
        );
        cursor_y += header_height;

        let rule_rect = Rect::new(
            panel_rect.x + PANEL_PADDING,
            cursor_y,
            panel_rect.width - (PANEL_PADDING as u32 * 2),
            1,
        );
        fill_rect(surface, rule_rect, HEADER_RULE_COLOR);
        cursor_y += GAP_AFTER_HEADER;

        for entry in &self.entries {
            let row_top = cursor_y;
            cursor_y += ROW_HEIGHT;

            let row_rect = Rect::new(
                panel_rect.x + PANEL_PADDING,
                row_top,
                panel_rect.width - (PANEL_PADDING as u32 * 2),
                ROW_HEIGHT as u32,
            );
            if entry.is_local {
                fill_rect(surface, row_rect, LOCAL_ROW_HIGHLIGHT);
            }

            let text_y = row_top as f32 + (ROW_HEIGHT as f32 - ROW_FONT_SIZE) * 0.5;
            let mut name_x = name_column_x;
            if let Some(color) = entry.color {
                let size = COLOR_SWATCH_SIZE.min(ROW_HEIGHT - 4);
                let swatch_y = row_top + (ROW_HEIGHT - size) / 2;
                let swatch_rect = Rect::new(name_x, swatch_y, size as u32, size as u32);
                fill_rect(surface, swatch_rect, color);
                draw_border(surface, swatch_rect, COLOR_SWATCH_BORDER);
                name_x += size + 8;
            }

            font.draw_text(
                surface,
                name_x as f32,
                text_y,
                &entry.name,
                ROW_FONT_SIZE,
                if entry.outcome == GameOverOutcome::Observer {
                    MUTED_TEXT_COLOR
                } else {
                    TEXT_COLOR
                },
            );

            font.draw_text(
                surface,
                outcome_column_x as f32,
                text_y,
                entry.outcome.label(),
                ROW_FONT_SIZE,
                entry.outcome.label_color(),
            );

            draw_stat(surface, font, wealth_column_x, text_y, entry.wealth);
            draw_stat(surface, font, score_column_x, text_y, entry.score);
            draw_stat(surface, font, value_column_x, text_y, entry.value);
        }

        for (index, (button, rect)) in self
            .buttons
            .iter()
            .zip(self.button_rects(surface.width(), surface.height()))
            .enumerate()
        {
            fill_rect(
                surface,
                rect,
                if index == self.selected_button {
                    BUTTON_SELECTED_COLOR
                } else {
                    BUTTON_COLOR
                },
            );
            draw_border(surface, rect, BUTTON_BORDER_COLOR);
            draw_text_centered(
                surface,
                font,
                &button.label.replace('&', ""),
                BUTTON_FONT_SIZE,
                TEXT_COLOR,
                rect.x,
                rect.x + rect.width as i32,
                rect.y + (BUTTON_HEIGHT - BUTTON_FONT_SIZE.ceil() as i32) / 2,
            );
        }

        let footer_y = panel_rect.y as f32 + panel_rect.height as f32
            - PANEL_PADDING as f32
            - FOOTER_FONT_SIZE;
        draw_text_centered(
            surface,
            font,
            self.selected_description(),
            FOOTER_FONT_SIZE,
            MUTED_TEXT_COLOR,
            content_left,
            content_right,
            footer_y as i32,
        );
    }
}

fn point_in_rect(x: f32, y: f32, rect: Rect) -> bool {
    x >= rect.x as f32
        && y >= rect.y as f32
        && x < (rect.x + rect.width as i32) as f32
        && y < (rect.y + rect.height as i32) as f32
}

fn draw_header(
    surface: &mut Surface,
    font: &dyn TextFont,
    baseline_y: i32,
    name_x: i32,
    outcome_x: i32,
    wealth_x: i32,
    score_x: i32,
    value_x: i32,
) {
    let header_y = baseline_y as f32;
    font.draw_text(
        surface,
        name_x as f32,
        header_y,
        "Player",
        HEADER_FONT_SIZE,
        HEADER_COLOR,
    );
    font.draw_text(
        surface,
        outcome_x as f32,
        header_y,
        "Outcome",
        HEADER_FONT_SIZE,
        HEADER_COLOR,
    );
    draw_header_stat(surface, font, wealth_x, header_y, "Wealth");
    draw_header_stat(surface, font, score_x, header_y, "Score");
    draw_header_stat(surface, font, value_x, header_y, "Value");
}

fn draw_header_stat(
    surface: &mut Surface,
    font: &dyn TextFont,
    column_x: i32,
    baseline: f32,
    label: &str,
) {
    let metrics = font.measure_text(label, HEADER_FONT_SIZE);
    let x = column_x as f32 + STAT_COLUMN_WIDTH as f32 - metrics.width;
    font.draw_text(surface, x, baseline, label, HEADER_FONT_SIZE, HEADER_COLOR);
}

fn draw_stat(surface: &mut Surface, font: &dyn TextFont, column_x: i32, y: f32, value: i32) {
    let text = format!("{value}");
    let metrics = font.measure_text(&text, ROW_FONT_SIZE);
    let x = column_x as f32 + STAT_COLUMN_WIDTH as f32 - metrics.width;
    font.draw_text(surface, x, y, &text, ROW_FONT_SIZE, TEXT_COLOR);
}

fn draw_text_centered(
    surface: &mut Surface,
    font: &dyn TextFont,
    text: &str,
    size: f32,
    color: Color,
    left: i32,
    right: i32,
    baseline: i32,
) {
    let metrics = font.measure_text(text, size);
    let width = metrics.width;
    let x = (left as f32 + right as f32 - width) * 0.5;
    font.draw_text(surface, x, baseline as f32, text, size, color);
}

fn fill_rect(surface: &mut Surface, rect: Rect, color: Color) {
    if let Some(clipped) = rect.intersection(surface.bounds()) {
        for y in clipped.y..(clipped.y + clipped.height as i32) {
            for x in clipped.x..(clipped.x + clipped.width as i32) {
                let _ = surface.blend_pixel(x as u32, y as u32, color);
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

#[cfg(test)]
mod tests {
    use super::*;

    fn entry(id: i32, name: &str, outcome: GameOverOutcome, is_local: bool) -> GameOverEntry {
        GameOverEntry {
            player_id: id,
            name: name.to_string(),
            outcome,
            wealth: 0,
            score: 0,
            value: 0,
            is_local,
            color: None,
        }
    }

    #[test]
    fn subtitle_prefers_local_outcome() {
        let entries = vec![
            entry(1, "Observer", GameOverOutcome::Observer, false),
            entry(2, "Player", GameOverOutcome::Victory, true),
            entry(3, "Opponent", GameOverOutcome::Defeat, false),
        ];
        let state = GameOverState::new("Goldmine".into(), entries);
        assert_eq!(state.subtitle(), "Victory!");
    }

    #[test]
    fn next_mission_replaces_restart_on_narrow_screens_like_cpp() {
        // C4GameOverDlg hides Restart below 1280 px when a next mission is
        // available, leaving End/Continue/Next (C4GameOverDlg.cpp:125-139,
        // 232-258).
        let state = GameOverState::with_next_mission(
            "A Clonk".into(),
            Vec::new(),
            1279,
            Some(NextMissionButton {
                label: "Next tutorial".into(),
                description: "Continue learning".into(),
            }),
        );

        assert_eq!(
            state.actions(),
            &[
                GameOverAction::End,
                GameOverAction::Continue,
                GameOverAction::NextMission,
            ]
        );
        assert_eq!(state.selected_description(), "End the round.");
    }

    #[test]
    fn wide_game_over_keeps_restart_and_navigation_wraps() {
        let mut state = GameOverState::with_next_mission(
            "A Clonk".into(),
            Vec::new(),
            1280,
            Some(NextMissionButton {
                label: "Next tutorial".into(),
                description: "Continue learning".into(),
            }),
        );
        assert_eq!(
            state.actions(),
            &[
                GameOverAction::End,
                GameOverAction::Continue,
                GameOverAction::Restart,
                GameOverAction::NextMission,
            ]
        );

        state.move_selection(-1);
        assert_eq!(state.activate_selected(), Some(GameOverAction::NextMission));
        assert_eq!(state.selected_description(), "Continue learning");
    }

    #[test]
    fn subtitle_defaults_to_defeat_without_winners() {
        let entries = vec![
            entry(1, "Player", GameOverOutcome::Defeat, false),
            entry(2, "Opponent", GameOverOutcome::Defeat, false),
        ];
        let state = GameOverState::new("Goldmine".into(), entries);
        assert_eq!(state.subtitle(), "Defeat");
    }

    #[test]
    fn entries_sorted_by_outcome_and_name() {
        let entries = vec![
            entry(3, "Charlie", GameOverOutcome::Defeat, false),
            entry(1, "Alice", GameOverOutcome::Victory, false),
            entry(2, "Bravo", GameOverOutcome::Victory, true),
            entry(4, "Delta", GameOverOutcome::Observer, false),
        ];
        let state = GameOverState::new("Goldmine".into(), entries);
        let ids: Vec<i32> = state
            .entries()
            .iter()
            .map(|entry| entry.player_id)
            .collect();
        assert_eq!(ids, vec![1, 2, 3, 4]);
    }
}
