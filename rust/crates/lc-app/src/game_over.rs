use lc_frontend::classic_gui::{ClassicButtonState, ClassicGuiSkin, IntRect};
use lc_frontend::{ClonkFontSet, ImageData};
use lc_graphics::clonk_font::TextAlign;
use lc_graphics::{Color, Rect, Surface, TextFont};

const CLASSIC_DIALOG_TITLE: &str = "Evaluation";
const CLASSIC_MIN_CAPTION_HEIGHT: i32 = 23;
const CLASSIC_BUTTON_HEIGHT: i32 = 32;
const CLASSIC_INDENT_X: i32 = 10;
const CLASSIC_INDENT_Y: i32 = 6;
const CLASSIC_GOAL_SIZE: i32 = 64;
const CLASSIC_GOAL_MARGIN: i32 = 4;
const CLASSIC_PLAYER_LIST_TOP_INSET: i32 = 12;
const CLASSIC_PLAYER_ROW_HEIGHT: i32 = 54;
const CLASSIC_PLAYER_ROW_STEP: i32 = 55;
const CLASSIC_PLAYER_ROW_LEFT_INSET: i32 = 6;
const CLASSIC_PLAYER_ROW_RIGHT_INSET: i32 = 19;
const CLASSIC_PLAYER_ROW_TOP_INSET: i32 = 3;
const CLASSIC_PLAYER_LABEL_SPACING: i32 = 2;
pub const CLASSIC_FULFILLED_STAR_SOURCE: IntRect = IntRect {
    x: 0,
    y: 320,
    w: 40,
    h: 40,
};

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

/// One frozen goal row consumed by the classic evaluation dialog.
///
/// `picture` is the buffered definition picture created without invoking
/// scenario callbacks, just like `C4GoalDisplay::GoalPicture`
/// (`C4GameOverDlg.cpp:25-58`).
#[derive(Clone, Debug, PartialEq)]
pub struct EvaluationGoal {
    pub definition_id: String,
    pub fulfilled: bool,
    pub picture: Option<ImageData>,
}

/// Presentation data for one C4PlayerInfo evaluation row.
///
/// `player_info_id` deliberately is not the in-round C4Player number:
/// C4PlayerInfoListBox joins player infos to C4RoundResultsPlayer by info ID
/// (`C4PlayerInfoListBox.cpp:132-143,344-358`).
#[derive(Clone, Debug, PartialEq)]
pub struct EvaluationPlayer {
    pub player_info_id: i32,
    pub name: String,
    pub won: bool,
    pub color_dw: u32,
    pub total_playing_time: u32,
    pub score_old: i32,
    pub score_new: Option<i32>,
    pub custom_evaluation_strings: String,
    pub big_icon: Option<ImageData>,
}

/// Frozen presentation model for C4GameOverDlg.
///
/// Player order remains the order supplied by C4PlayerInfos; C++ does not sort
/// evaluation rows by numeric info ID. Lookups still use the info ID as the
/// sole join key.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct EvaluationViewModel {
    goals: Vec<EvaluationGoal>,
    players: Vec<EvaluationPlayer>,
}

impl EvaluationViewModel {
    pub fn new(goals: Vec<EvaluationGoal>, players: Vec<EvaluationPlayer>) -> Self {
        Self { goals, players }
    }

    pub fn goals(&self) -> &[EvaluationGoal] {
        &self.goals
    }

    pub fn players(&self) -> impl ExactSizeIterator<Item = &EvaluationPlayer> {
        self.players.iter()
    }

    pub fn player_by_info_id(&self, player_info_id: i32) -> Option<&EvaluationPlayer> {
        self.players
            .iter()
            .find(|player| player.player_info_id == player_info_id)
    }
}

#[derive(Clone, Copy)]
pub struct GameOverClassicResources<'a> {
    skin: ClassicGuiSkin<'a>,
    fonts: &'a ClonkFontSet,
    gui_icons: Option<&'a ImageData>,
    player: Option<&'a ImageData>,
    score: Option<&'a ImageData>,
}

impl<'a> GameOverClassicResources<'a> {
    pub const fn new(
        skin: ClassicGuiSkin<'a>,
        fonts: &'a ClonkFontSet,
        gui_icons: Option<&'a ImageData>,
        player: Option<&'a ImageData>,
        score: Option<&'a ImageData>,
    ) -> Self {
        Self {
            skin,
            fonts,
            gui_icons,
            player,
            score,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct ClassicGameOverLayout {
    dialog: IntRect,
    caption: IntRect,
    player_area: IntRect,
    buttons: Vec<IntRect>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ClassicEvaluationGoalLayout {
    pub picture: IntRect,
    pub fulfilled_star: IntRect,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ClassicEvaluationPlayerLayout {
    pub row: IntRect,
    pub icon: IntRect,
    pub name_anchor: (i32, i32),
    pub score_anchor: (i32, i32),
    pub time_anchor: (i32, i32),
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ClassicEvaluationLayout {
    pub goal_display: Option<IntRect>,
    pub goals: Vec<ClassicEvaluationGoalLayout>,
    pub player_list: IntRect,
    pub players: Vec<ClassicEvaluationPlayerLayout>,
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
    evaluation: EvaluationViewModel,
    buttons: Vec<GameOverButton>,
    selected_button: usize,
    pressed_button: Option<usize>,
    classic_button_width: Option<i32>,
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
            evaluation: EvaluationViewModel::default(),
            buttons,
            selected_button: 0,
            pressed_button: None,
            classic_button_width: None,
            pointer_position: None,
        }
    }

    pub fn configure_classic_fonts(&mut self, fonts: Option<&ClonkFontSet>) {
        self.classic_button_width = fonts.map(classic_button_width);
    }

    pub fn subtitle(&self) -> &str {
        &self.subtitle
    }

    pub fn set_evaluation(&mut self, evaluation: EvaluationViewModel) {
        self.evaluation = evaluation;
    }

    pub fn evaluation(&self) -> &EvaluationViewModel {
        &self.evaluation
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
        self.pressed_button = None;
    }

    pub fn handle_pointer_down(&mut self, surface_width: u32, surface_height: u32) {
        self.pressed_button = self.pointer_position.and_then(|(x, y)| {
            self.button_rects(surface_width, surface_height)
                .iter()
                .position(|rect| point_in_rect(x, y, *rect))
        });
    }

    pub fn handle_pointer_up(
        &mut self,
        surface_width: u32,
        surface_height: u32,
    ) -> Option<GameOverAction> {
        let pressed = self.pressed_button.take()?;
        let hovered = self.pointer_position.and_then(|(x, y)| {
            self.button_rects(surface_width, surface_height)
                .iter()
                .position(|rect| point_in_rect(x, y, *rect))
        });
        (hovered == Some(pressed))
            .then(|| self.buttons.get(pressed).map(|button| button.action))
            .flatten()
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
        if let Some(button_width) = self.classic_button_width {
            return self
                .classic_layout_with_button_width(surface_width, surface_height, button_width)
                .buttons
                .into_iter()
                .map(surface_rect)
                .collect();
        }
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

    fn classic_layout(
        &self,
        surface_width: u32,
        surface_height: u32,
        fonts: &ClonkFontSet,
    ) -> ClassicGameOverLayout {
        self.classic_layout_with_button_width(
            surface_width,
            surface_height,
            classic_button_width(fonts),
        )
    }

    fn classic_layout_with_button_width(
        &self,
        surface_width: u32,
        surface_height: u32,
        button_width: i32,
    ) -> ClassicGameOverLayout {
        let screen_width = surface_width as i32;
        let screen_height = surface_height as i32;
        let dialog_width = if screen_width < 1280 {
            screen_width - 10
        } else {
            (screen_width - 150).min(1280)
        }
        .max(1);
        let dialog_height = if screen_height < 720 {
            screen_height - 10
        } else {
            (screen_height - 150).min(720)
        }
        .max(1);
        let dialog = IntRect {
            x: (screen_width - dialog_width) / 2,
            y: (screen_height - dialog_height) / 2,
            w: dialog_width,
            h: dialog_height,
        };
        let caption_height = CLASSIC_MIN_CAPTION_HEIGHT.min(dialog.h);
        let caption = IntRect {
            h: caption_height,
            ..dialog
        };
        let client_height = (dialog.h - caption_height).max(0);

        // ComponentAligner caMain(GetClientRect(), 0, 6, true), followed by
        // GetFromBottom(0) and GetFromBottom(32 + 2*6).
        let after_bottom_padding = (client_height - 2 * CLASSIC_INDENT_Y).max(0);
        let button_area_height = CLASSIC_BUTTON_HEIGHT + 2 * CLASSIC_INDENT_Y;
        let button_area_y = (after_bottom_padding - button_area_height - CLASSIC_INDENT_Y).max(0);
        let remaining_height =
            (after_bottom_padding - button_area_height - 2 * CLASSIC_INDENT_Y).max(0);
        let player_area = IntRect {
            x: dialog.x + CLASSIC_INDENT_X,
            y: dialog.y + caption_height + CLASSIC_INDENT_Y,
            w: (dialog.w - 2 * CLASSIC_INDENT_X).max(0),
            h: (remaining_height - 2 * CLASSIC_INDENT_Y).max(0),
        };

        let count = self.buttons.len().max(1) as i32;
        let cell_width = ((dialog.w - CLASSIC_INDENT_X) / count - CLASSIC_INDENT_X).max(1);
        let actual_button_width = button_width.min(cell_width).max(1);
        let buttons = self
            .buttons
            .iter()
            .enumerate()
            .map(|(index, _)| IntRect {
                x: dialog.x
                    + CLASSIC_INDENT_X
                    + index as i32 * (cell_width + CLASSIC_INDENT_X)
                    + (cell_width - actual_button_width) / 2,
                y: dialog.y + caption_height + button_area_y,
                w: actual_button_width,
                h: button_area_height,
            })
            .collect();

        ClassicGameOverLayout {
            dialog,
            caption,
            player_area,
            buttons,
        }
    }

    fn classic_evaluation_layout(
        &self,
        surface_width: u32,
        surface_height: u32,
        fonts: &ClonkFontSet,
    ) -> ClassicEvaluationLayout {
        let chrome = self.classic_layout(surface_width, surface_height, fonts);
        let goal_area_height = CLASSIC_GOAL_SIZE + 2 * CLASSIC_GOAL_MARGIN;
        let goal_count = self.evaluation.goals().len() as i32;
        let goals_per_row = (chrome.dialog.w / goal_area_height).max(1);
        let goal_rows = if goal_count == 0 {
            0
        } else {
            (goal_count - 1) / goals_per_row + 1
        };
        let goal_display = (goal_rows > 0).then_some(IntRect {
            x: chrome.dialog.x,
            y: chrome.player_area.y,
            w: chrome.dialog.w,
            h: goal_rows * goal_area_height,
        });
        let goals = (0..goal_count)
            .map(|index| {
                let row = index / goals_per_row;
                let column = index % goals_per_row;
                let row_start = row * goals_per_row;
                let goals_in_row = (goal_count - row_start).min(goals_per_row);
                let group_width = goals_in_row * goal_area_height;
                let picture = IntRect {
                    x: chrome.dialog.x + (chrome.dialog.w - group_width) / 2
                        + column * goal_area_height
                        + CLASSIC_GOAL_MARGIN,
                    y: chrome.player_area.y
                        + row * goal_area_height
                        + CLASSIC_GOAL_MARGIN,
                    w: CLASSIC_GOAL_SIZE,
                    h: CLASSIC_GOAL_SIZE,
                };
                ClassicEvaluationGoalLayout {
                    fulfilled_star: IntRect {
                        x: picture.x + picture.w / 2,
                        y: picture.y + picture.h / 2,
                        w: picture.w / 2,
                        h: picture.h / 2,
                    },
                    picture,
                }
            })
            .collect();

        // C4PlayerInfoListBox's scroll window keeps the same top inset and
        // hidden-scrollbar column even when no scrollbar is visible
        // (C4GameOverDlg.cpp:210-220; C4Gui.cpp:1346-1384).
        let player_list_y = chrome.player_area.y
            + goal_rows * goal_area_height
            + CLASSIC_PLAYER_LIST_TOP_INSET;
        let player_list = IntRect {
            x: chrome.player_area.x,
            y: player_list_y,
            w: chrome.player_area.w,
            h: (chrome.player_area.y + chrome.player_area.h - player_list_y).max(0),
        };
        let row_width =
            (player_list.w - CLASSIC_PLAYER_ROW_LEFT_INSET - CLASSIC_PLAYER_ROW_RIGHT_INSET)
                .max(0);
        let players = self
            .evaluation
            .players()
            .enumerate()
            .map(|(index, _)| {
                let row = IntRect {
                    x: player_list.x + CLASSIC_PLAYER_ROW_LEFT_INSET,
                    y: player_list.y
                        + CLASSIC_PLAYER_ROW_TOP_INSET
                        + index as i32 * CLASSIC_PLAYER_ROW_STEP,
                    w: row_width,
                    h: CLASSIC_PLAYER_ROW_HEIGHT,
                };
                let right_anchor = row.x + row.w - CLASSIC_PLAYER_LABEL_SPACING;
                ClassicEvaluationPlayerLayout {
                    row,
                    icon: IntRect {
                        w: CLASSIC_PLAYER_ROW_HEIGHT,
                        h: CLASSIC_PLAYER_ROW_HEIGHT,
                        ..row
                    },
                    name_anchor: (
                        row.x + CLASSIC_PLAYER_ROW_HEIGHT + CLASSIC_PLAYER_LABEL_SPACING,
                        row.y + CLASSIC_PLAYER_LABEL_SPACING,
                    ),
                    score_anchor: (right_anchor, row.y + CLASSIC_PLAYER_LABEL_SPACING),
                    time_anchor: (right_anchor, row.y + 26),
                }
            })
            .collect();

        ClassicEvaluationLayout {
            goal_display,
            goals,
            player_list,
            players,
        }
    }

    pub fn render(
        &self,
        surface: &mut Surface,
        font: &dyn TextFont,
        classic: Option<GameOverClassicResources<'_>>,
    ) {
        if surface.width() == 0 || surface.height() == 0 {
            return;
        }

        if let Some(classic) = classic {
            self.render_classic(surface, classic);
        } else {
            self.render_fallback(surface, font);
        }
    }

    fn render_fallback(&self, surface: &mut Surface, font: &dyn TextFont) {
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

    fn render_classic(&self, surface: &mut Surface, resources: GameOverClassicResources<'_>) {
        let layout = self.classic_layout(surface.width(), surface.height(), resources.fonts);
        resources.skin.draw_dialog(surface, layout.dialog, None);
        resources.skin.draw_caption(
            surface,
            layout.caption,
            CLASSIC_DIALOG_TITLE,
            &resources.fonts.text,
            [0xff, 0xff, 0xff, 0xff],
            TextAlign::Left,
            None,
        );

        self.render_classic_entries(surface, resources.fonts, layout.player_area);
        for (index, (button, rect)) in self.buttons.iter().zip(layout.buttons).enumerate() {
            resources.skin.draw_button(
                surface,
                rect,
                &button.label,
                resources.fonts,
                ClassicButtonState {
                    pressed: self.pressed_button == Some(index),
                    highlighted: self.selected_button == index,
                },
                None,
            );
        }
    }

    fn render_classic_entries(&self, surface: &mut Surface, fonts: &ClonkFontSet, area: IntRect) {
        let content_left = area.x;
        let content_right = area.x + area.w;
        let mut cursor_y = area.y;
        let mut column_right = content_right;
        let value_column_x = column_right - STAT_COLUMN_WIDTH;
        column_right = value_column_x - COLUMN_GAP;
        let score_column_x = column_right - STAT_COLUMN_WIDTH;
        column_right = score_column_x - COLUMN_GAP;
        let wealth_column_x = column_right - STAT_COLUMN_WIDTH;
        column_right = wealth_column_x - COLUMN_GAP;
        let outcome_column_x = column_right - OUTCOME_WIDTH;

        draw_clonk_text(
            surface,
            &fonts.text,
            content_left,
            cursor_y,
            "Player",
            HEADER_COLOR,
            TextAlign::Left,
        );
        draw_clonk_text(
            surface,
            &fonts.text,
            outcome_column_x,
            cursor_y,
            "Outcome",
            HEADER_COLOR,
            TextAlign::Left,
        );
        draw_clonk_text(
            surface,
            &fonts.text,
            wealth_column_x + STAT_COLUMN_WIDTH,
            cursor_y,
            "Wealth",
            HEADER_COLOR,
            TextAlign::Right,
        );
        draw_clonk_text(
            surface,
            &fonts.text,
            score_column_x + STAT_COLUMN_WIDTH,
            cursor_y,
            "Score",
            HEADER_COLOR,
            TextAlign::Right,
        );
        draw_clonk_text(
            surface,
            &fonts.text,
            value_column_x + STAT_COLUMN_WIDTH,
            cursor_y,
            "Value",
            HEADER_COLOR,
            TextAlign::Right,
        );
        cursor_y += fonts.text.line_height + GAP_AFTER_HEADER;

        for entry in &self.entries {
            let row_top = cursor_y;
            cursor_y += ROW_HEIGHT;
            if row_top + ROW_HEIGHT > area.y + area.h {
                break;
            }
            if entry.is_local {
                fill_rect(
                    surface,
                    Rect::new(area.x, row_top, area.w.max(0) as u32, ROW_HEIGHT as u32),
                    LOCAL_ROW_HIGHLIGHT,
                );
            }
            let text_y = row_top + (ROW_HEIGHT - fonts.text.line_height) / 2;
            let mut name_x = content_left;
            if let Some(color) = entry.color {
                let size = COLOR_SWATCH_SIZE.min(ROW_HEIGHT - 4);
                let swatch_y = row_top + (ROW_HEIGHT - size) / 2;
                let swatch = Rect::new(name_x, swatch_y, size as u32, size as u32);
                fill_rect(surface, swatch, color);
                draw_border(surface, swatch, COLOR_SWATCH_BORDER);
                name_x += size + 8;
            }
            let name_color = if entry.outcome == GameOverOutcome::Observer {
                MUTED_TEXT_COLOR
            } else {
                TEXT_COLOR
            };
            draw_clonk_text(
                surface,
                &fonts.text,
                name_x,
                text_y,
                &entry.name,
                name_color,
                TextAlign::Left,
            );
            draw_clonk_text(
                surface,
                &fonts.text,
                outcome_column_x,
                text_y,
                entry.outcome.label(),
                entry.outcome.label_color(),
                TextAlign::Left,
            );
            draw_clonk_text(
                surface,
                &fonts.text,
                wealth_column_x + STAT_COLUMN_WIDTH,
                text_y,
                &entry.wealth.to_string(),
                TEXT_COLOR,
                TextAlign::Right,
            );
            draw_clonk_text(
                surface,
                &fonts.text,
                score_column_x + STAT_COLUMN_WIDTH,
                text_y,
                &entry.score.to_string(),
                TEXT_COLOR,
                TextAlign::Right,
            );
            draw_clonk_text(
                surface,
                &fonts.text,
                value_column_x + STAT_COLUMN_WIDTH,
                text_y,
                &entry.value.to_string(),
                TEXT_COLOR,
                TextAlign::Right,
            );
        }
    }
}

fn classic_button_width(fonts: &ClonkFontSet) -> i32 {
    fonts.caption.measure("Quit it, baby! And some.", true).0 * 13 / 10
}

fn surface_rect(rect: IntRect) -> Rect {
    Rect::new(rect.x, rect.y, rect.w.max(0) as u32, rect.h.max(0) as u32)
}

fn draw_clonk_text(
    surface: &mut Surface,
    font: &lc_graphics::clonk_font::ClonkFont,
    x: i32,
    y: i32,
    text: &str,
    color: Color,
    align: TextAlign,
) {
    font.draw_with_gamma(
        surface,
        x,
        y,
        text,
        [color.r, color.g, color.b, color.a],
        align,
        true,
        None,
    );
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

    fn endeavour_fonts() -> lc_frontend::ClonkFontSet {
        let path = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("../../../planet/System.c4g/Endeavour.ttf");
        let bytes = std::fs::read(path).expect("read Endeavour.ttf");
        crate::clonk_fonts::build_font_set(&bytes).expect("build Endeavour GUI fonts")
    }

    fn solid_image(width: u32, height: u32, color: [u8; 4]) -> lc_frontend::ImageData {
        lc_frontend::ImageData::new(
            width,
            height,
            std::iter::repeat_n(color, (width * height) as usize)
                .flatten()
                .collect(),
        )
    }

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
    fn classic_resources_carry_cpp_evaluation_icon_sheets() {
        // C4GoalDisplay uses GUIIcons for Ico_Star, while evaluation player
        // rows use GraphicsResource Player and Score facets
        // (C4GameOverDlg.cpp:68-71; C4PlayerInfoListBox.cpp:261-322,
        // 344-425; C4GraphicsResource.cpp:265-268).
        let caption = solid_image(96, 23, [1, 2, 3, 255]);
        let button = solid_image(128, 32, [4, 5, 6, 255]);
        let button_down = solid_image(128, 32, [7, 8, 9, 255]);
        let gui_icons = solid_image(240, 360, [10, 11, 12, 255]);
        let player = solid_image(48, 48, [13, 14, 15, 255]);
        let score = solid_image(60, 30, [16, 17, 18, 255]);
        let fonts = endeavour_fonts();
        let resources = GameOverClassicResources::new(
            ClassicGuiSkin::new(&caption, &button, &button_down, None),
            &fonts,
            Some(&gui_icons),
            Some(&player),
            Some(&score),
        );

        assert_eq!(resources.gui_icons.map(|image| (image.width(), image.height())), Some((240, 360)));
        assert_eq!(CLASSIC_FULFILLED_STAR_SOURCE, IntRect { x: 0, y: 320, w: 40, h: 40 });
        assert_eq!(resources.player.map(|image| (image.width(), image.height())), Some((48, 48)));
        assert_eq!(resources.score.map(|image| (image.width(), image.height())), Some((60, 30)));
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

    #[test]
    fn classic_host_layout_matches_cpp_at_1024x600() {
        // Local control is the host. C4GameOverDlg expands to screen-10 below
        // 1280x720, centers the dialog, titles it "Evaluation", and lays out
        // the bottom buttons through ComponentAligner (C4GameOverDlg.cpp:
        // 115-157,232-258; C4Gui.cpp:1025-1079).
        let fonts = endeavour_fonts();
        let state = GameOverState::with_next_mission(
            "A Clonk".into(),
            Vec::new(),
            1024,
            Some(NextMissionButton {
                label: "Next tutorial".into(),
                description: "Continue learning".into(),
            }),
        );
        let layout = state.classic_layout(1024, 600, &fonts);

        assert_eq!(CLASSIC_DIALOG_TITLE, "Evaluation");
        assert_eq!(
            layout.dialog,
            IntRect {
                x: 5,
                y: 5,
                w: 1014,
                h: 590
            }
        );
        assert_eq!(
            layout.caption,
            IntRect {
                x: 5,
                y: 5,
                w: 1014,
                h: 23
            }
        );
        assert_eq!(
            layout.player_area,
            IntRect {
                x: 15,
                y: 34,
                w: 994,
                h: 487
            }
        );
        assert_eq!(
            layout.buttons,
            vec![
                IntRect {
                    x: 65,
                    y: 533,
                    w: 224,
                    h: 44
                },
                IntRect {
                    x: 399,
                    y: 533,
                    w: 224,
                    h: 44
                },
                IntRect {
                    x: 733,
                    y: 533,
                    w: 224,
                    h: 44
                },
            ]
        );
    }

    fn evaluation_player(player_info_id: i32, name: &str) -> EvaluationPlayer {
        EvaluationPlayer {
            player_info_id,
            name: name.to_string(),
            won: true,
            color_dw: 0x00f4_0000,
            total_playing_time: 165,
            score_old: 0,
            score_new: Some(100),
            custom_evaluation_strings: String::new(),
            big_icon: None,
        }
    }

    fn evaluation_state(screen_width: u32) -> GameOverState {
        let evaluation = EvaluationViewModel::new(
            vec![EvaluationGoal {
                definition_id: "SCRG".to_string(),
                fulfilled: true,
                picture: None,
            }],
            vec![
                evaluation_player(41, "Player"),
                evaluation_player(7, "Second"),
            ],
        );
        let mut state = GameOverState::with_next_mission(
            "A Clonk".into(),
            Vec::new(),
            screen_width,
            Some(NextMissionButton {
                label: "Next tutorial".into(),
                description: "Continue learning".into(),
            }),
        );
        state.set_evaluation(evaluation);
        state
    }

    #[test]
    fn evaluation_players_are_keyed_by_cpp_player_info_id() {
        // C4PlayerInfoListBox looks up C4RoundResultsPlayer with the
        // C4PlayerInfo ID, never the runtime C4Player number
        // (C4PlayerInfoListBox.cpp:132-143,344-358).
        let state = evaluation_state(1024);

        assert_eq!(
            state
                .evaluation()
                .player_by_info_id(41)
                .map(|player| player.name.as_str()),
            Some("Player")
        );
        assert!(state.evaluation().player_by_info_id(0).is_none());
        assert_eq!(
            state
                .evaluation()
                .players()
                .map(|player| player.player_info_id)
                .collect::<Vec<_>>(),
            vec![41, 7],
            "C4PlayerInfo order must not become numeric info-ID order"
        );
    }

    #[test]
    fn classic_evaluation_content_layout_matches_cpp_at_audited_resolutions() {
        // C4GameOverDlg takes a 64px goal strip, C4GoalDisplay adds 4px
        // margins around each goal, and C4PlayerInfoListBox reserves its
        // hidden 16px scrollbar while positioning 54px evaluation rows
        // (C4GameOverDlg.cpp:145-220; C4PlayerInfoListBox.cpp:79-154,
        // 184-231; C4Gui.h:106-129).
        let fonts = endeavour_fonts();

        let state = evaluation_state(1024);
        let chrome = state.classic_layout(1024, 600, &fonts);
        let content = state.classic_evaluation_layout(1024, 600, &fonts);
        assert_eq!(chrome.buttons.iter().map(|rect| rect.x).collect::<Vec<_>>(), vec![65, 399, 733]);
        assert_eq!(content.goal_display, Some(IntRect { x: 5, y: 34, w: 1014, h: 72 }));
        assert_eq!(content.goals[0].picture, IntRect { x: 480, y: 38, w: 64, h: 64 });
        assert_eq!(content.goals[0].fulfilled_star, IntRect { x: 512, y: 70, w: 32, h: 32 });
        assert_eq!(content.player_list, IntRect { x: 15, y: 118, w: 994, h: 403 });
        assert_eq!(content.players[0].row, IntRect { x: 21, y: 121, w: 969, h: 54 });
        assert_eq!(content.players[0].icon, IntRect { x: 21, y: 121, w: 54, h: 54 });
        assert_eq!(content.players[0].name_anchor, (77, 123));
        assert_eq!(content.players[0].score_anchor, (988, 123));
        assert_eq!(content.players[0].time_anchor, (988, 147));

        let state = evaluation_state(1280);
        let chrome = state.classic_layout(1280, 720, &fonts);
        let content = state.classic_evaluation_layout(1280, 720, &fonts);
        assert_eq!(chrome.dialog, IntRect { x: 75, y: 75, w: 1130, h: 570 });
        assert_eq!(chrome.buttons.iter().map(|rect| rect.x).collect::<Vec<_>>(), vec![108, 388, 668, 948]);
        assert!(chrome.buttons.iter().all(|rect| rect.y == 583));
        assert_eq!(content.goals[0].picture, IntRect { x: 608, y: 108, w: 64, h: 64 });
        assert_eq!(content.player_list, IntRect { x: 85, y: 188, w: 1110, h: 383 });
        assert_eq!(content.players[0].row, IntRect { x: 91, y: 191, w: 1085, h: 54 });

        let state = evaluation_state(1920);
        let chrome = state.classic_layout(1920, 1080, &fonts);
        let content = state.classic_evaluation_layout(1920, 1080, &fonts);
        assert_eq!(chrome.dialog, IntRect { x: 320, y: 180, w: 1280, h: 720 });
        assert_eq!(chrome.buttons.iter().map(|rect| rect.x).collect::<Vec<_>>(), vec![371, 688, 1005, 1322]);
        assert!(chrome.buttons.iter().all(|rect| rect.y == 838));
        assert_eq!(content.goals[0].picture, IntRect { x: 928, y: 213, w: 64, h: 64 });
        assert_eq!(content.player_list, IntRect { x: 330, y: 293, w: 1260, h: 533 });
        assert_eq!(content.players[0].row, IntRect { x: 336, y: 296, w: 1235, h: 54 });
    }

    #[test]
    fn classic_render_uses_skin_without_scrim_or_footer() {
        // Dialog, WoodenLabel and Button draw the standard C4GUI skin; only
        // the dialog bounds receive the translucent background, and buttons
        // select GUIButton/Down plus the additive highlight
        // (C4GuiDialogs.cpp:537-550; C4GuiLabels.cpp:168-214;
        // C4GuiButton.cpp:81-109).
        let fonts = endeavour_fonts();
        let caption = solid_image(192, 23, [200, 0, 0, 255]);
        let button = solid_image(128, 32, [0, 120, 0, 255]);
        let button_down = solid_image(128, 32, [0, 0, 180, 255]);
        let highlight = solid_image(16, 16, [80, 0, 0, 255]);
        let gui_icons = solid_image(240, 360, [0, 0, 0, 0]);
        let player = solid_image(48, 48, [0, 0, 0, 0]);
        let score = solid_image(60, 30, [0, 0, 0, 0]);
        let skin = lc_frontend::classic_gui::ClassicGuiSkin::new(
            &caption,
            &button,
            &button_down,
            Some(&highlight),
        );
        let mut state = GameOverState::with_next_mission(
            "A Clonk".into(),
            vec![entry(1, "Player", GameOverOutcome::Victory, true)],
            1024,
            Some(NextMissionButton {
                label: "Next tutorial".into(),
                description: "Continue learning".into(),
            }),
        );
        state.pressed_button = Some(1);
        let layout = state.classic_layout(1024, 600, &fonts);
        let background = Color::opaque(11, 22, 33);
        let mut surface = Surface::new(1024, 600, lc_graphics::PixelFormat::Rgba8888);
        surface.fill(background);
        let fallback = lc_graphics::BitmapFont::new();

        state.render(
            &mut surface,
            &fallback,
            Some(GameOverClassicResources::new(
                skin,
                &fonts,
                Some(&gui_icons),
                Some(&player),
                Some(&score),
            )),
        );

        assert_eq!(
            surface.get_pixel(0, 0),
            Some(background),
            "no full-screen scrim"
        );
        assert_eq!(
            surface.get_pixel(
                (layout.caption.x + 900) as u32,
                (layout.caption.y + 10) as u32
            ),
            Some(Color::opaque(200, 0, 0)),
            "GUICaption is used"
        );
        let focused = layout.buttons[0];
        assert_eq!(
            surface.get_pixel((focused.x + 8) as u32, (focused.y + 5) as u32),
            Some(Color::opaque(80, 120, 0)),
            "focused button receives additive GUIButtonHighlight"
        );
        let pressed = layout.buttons[1];
        assert_eq!(
            surface.get_pixel((pressed.x + 8) as u32, (pressed.y + 5) as u32),
            Some(Color::opaque(0, 0, 180)),
            "pressed button uses GUIButtonDown"
        );
        let footer_probe_y = layout.dialog.y + layout.dialog.h - 8;
        assert_eq!(
            surface.get_pixel(
                (layout.dialog.x + layout.dialog.w / 2) as u32,
                footer_probe_y as u32
            ),
            surface.get_pixel((layout.dialog.x + 20) as u32, footer_probe_y as u32),
            "classic dialog has no permanent description footer"
        );
    }

    #[test]
    fn classic_pointer_press_uses_down_state_and_requires_same_release_target() {
        // C4GUI::Button captures on left-down and invokes OnPress only when
        // left-up lands on the same button (C4GuiButton.cpp:128-155).
        let fonts = endeavour_fonts();
        let mut state = GameOverState::new(
            "A Clonk".into(),
            vec![entry(1, "Player", GameOverOutcome::Victory, true)],
        );
        state.configure_classic_fonts(Some(&fonts));
        let first = state.classic_layout(1024, 600, &fonts).buttons[0];
        state.handle_pointer_move(
            (first.x + first.w / 2) as f32,
            (first.y + first.h / 2) as f32,
            1024,
            600,
        );
        state.handle_pointer_down(1024, 600);
        assert_eq!(state.pressed_button, Some(0));
        assert_eq!(
            state.handle_pointer_up(1024, 600),
            Some(GameOverAction::End)
        );

        state.handle_pointer_down(1024, 600);
        state.handle_pointer_move(0.0, 0.0, 1024, 600);
        assert_eq!(state.handle_pointer_up(1024, 600), None);
    }
}
