use lc_frontend::classic_gui::{ClassicButtonState, ClassicGuiSkin, IntRect};
use lc_frontend::{ClonkFontSet, ImageData};
use lc_graphics::clonk_font::TextAlign;
use lc_graphics::{Color, GammaRamp, Rect, Surface, TextFont};

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
const CLASSIC_CLOSE_BUTTON_SIZE: i32 = 16;
const CLASSIC_CLOSE_BUTTON_INSET: i32 = 4;
pub const CLASSIC_FULFILLED_STAR_SOURCE: IntRect = IntRect {
    x: 0,
    y: 320,
    w: 40,
    h: 40,
};
pub const CLASSIC_CLOSE_ICON_SOURCE: IntRect = IntRect {
    x: 160,
    y: 200,
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
    icon_button_highlight: Option<&'a ImageData>,
    gui_icons: Option<&'a ImageData>,
    player: Option<&'a ImageData>,
    score: Option<&'a ImageData>,
}

impl<'a> GameOverClassicResources<'a> {
    pub const fn new(
        skin: ClassicGuiSkin<'a>,
        fonts: &'a ClonkFontSet,
        icon_button_highlight: Option<&'a ImageData>,
        gui_icons: Option<&'a ImageData>,
        player: Option<&'a ImageData>,
        score: Option<&'a ImageData>,
    ) -> Self {
        Self {
            skin,
            fonts,
            icon_button_highlight,
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
    close_button: IntRect,
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
    hovered_button: Option<usize>,
    pressed_button: Option<usize>,
    close_pressed: bool,
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
            hovered_button: None,
            pressed_button: None,
            close_pressed: false,
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

    pub fn hovered_description(&self) -> &str {
        self.hovered_button
            .and_then(|index| self.buttons.get(index))
            .map(|button| button.description.as_str())
            .unwrap_or("")
    }

    pub fn hovered_action(&self) -> Option<GameOverAction> {
        self.hovered_button
            .and_then(|index| self.buttons.get(index))
            .map(|button| button.action)
    }

    pub fn handle_pointer_move(&mut self, x: f32, y: f32, surface_width: u32, surface_height: u32) {
        self.pointer_position = Some((x, y));
        self.hovered_button = self
            .button_rects(surface_width, surface_height)
            .iter()
            .position(|rect| point_in_rect(x, y, *rect));
    }

    pub fn pointer_left(&mut self) {
        self.pointer_position = None;
        self.hovered_button = None;
        self.pressed_button = None;
        self.close_pressed = false;
    }

    pub fn handle_pointer_down(&mut self, surface_width: u32, surface_height: u32) {
        self.close_pressed = self.pointer_position.is_some_and(|(x, y)| {
            self.classic_close_button_rect(surface_width, surface_height)
                .is_some_and(|rect| point_in_rect(x, y, rect))
        });
        if self.close_pressed {
            self.pressed_button = None;
            return;
        }
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
        if std::mem::take(&mut self.close_pressed) {
            return self
                .pointer_position
                .filter(|(x, y)| {
                    self.classic_close_button_rect(surface_width, surface_height)
                        .is_some_and(|rect| point_in_rect(*x, *y, rect))
                })
                .map(|_| GameOverAction::End);
        }
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
        if self
            .classic_close_button_rect(surface_width, surface_height)
            .is_some_and(|rect| point_in_rect(x, y, rect))
        {
            return Some(GameOverAction::End);
        }
        self.button_rects(surface_width, surface_height)
            .iter()
            .position(|rect| point_in_rect(x, y, *rect))
            .and_then(|index| self.buttons.get(index))
            .map(|button| button.action)
    }

    fn classic_close_button_rect(&self, surface_width: u32, surface_height: u32) -> Option<Rect> {
        self.classic_button_width.map(|button_width| {
            surface_rect(
                self.classic_layout_with_button_width(surface_width, surface_height, button_width)
                    .close_button,
            )
        })
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
        let close_button = IntRect {
            x: caption.x + caption.w - CLASSIC_CLOSE_BUTTON_SIZE - CLASSIC_CLOSE_BUTTON_INSET,
            y: caption.y + CLASSIC_CLOSE_BUTTON_INSET,
            w: CLASSIC_CLOSE_BUTTON_SIZE,
            h: CLASSIC_CLOSE_BUTTON_SIZE,
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
            close_button,
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

        // GetFromTop plus the resized goal display leaves this extra inset;
        // without goals caPlayerArea receives caMain.GetAll() directly
        // (C4GameOverDlg.cpp:151-168,214-229; C4Gui.cpp:975-989,1041-1047).
        let post_goal_inset = if goal_rows > 0 {
            CLASSIC_PLAYER_LIST_TOP_INSET
        } else {
            0
        };
        let player_list_y = chrome.player_area.y
            + goal_rows * goal_area_height
            + post_goal_inset;
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
        self.render_with_gamma(surface, font, classic, None);
    }

    pub fn render_with_gamma(
        &self,
        surface: &mut Surface,
        font: &dyn TextFont,
        classic: Option<GameOverClassicResources<'_>>,
        gamma: Option<&GammaRamp>,
    ) {
        if surface.width() == 0 || surface.height() == 0 {
            return;
        }

        if let Some(classic) = classic {
            self.render_classic(surface, classic, gamma);
        } else {
            self.render_fallback(surface, font, gamma);
        }
    }

    fn render_fallback(
        &self,
        surface: &mut Surface,
        font: &dyn TextFont,
        gamma: Option<&GammaRamp>,
    ) {
        let surface_rect = Rect::new(0, 0, surface.width(), surface.height());
        fill_rect(surface, surface_rect, BACKDROP_COLOR, gamma);

        let title_height = TITLE_FONT_SIZE.ceil() as i32;
        let subtitle_height = if self.subtitle.is_empty() {
            0
        } else {
            SUBTITLE_FONT_SIZE.ceil() as i32
        };
        let header_height = HEADER_FONT_SIZE.ceil() as i32;
        let panel_rect = self.panel_rect(surface.width(), surface.height());
        fill_rect(surface, panel_rect, PANEL_COLOR, gamma);
        draw_border(surface, panel_rect, PANEL_BORDER, gamma);

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
            gamma,
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
                gamma,
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
            gamma,
        );
        cursor_y += header_height;

        let rule_rect = Rect::new(
            panel_rect.x + PANEL_PADDING,
            cursor_y,
            panel_rect.width - (PANEL_PADDING as u32 * 2),
            1,
        );
        fill_rect(surface, rule_rect, HEADER_RULE_COLOR, gamma);
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
                fill_rect(surface, row_rect, LOCAL_ROW_HIGHLIGHT, gamma);
            }

            let text_y = row_top as f32 + (ROW_HEIGHT as f32 - ROW_FONT_SIZE) * 0.5;
            let mut name_x = name_column_x;
            if let Some(color) = entry.color {
                let size = COLOR_SWATCH_SIZE.min(ROW_HEIGHT - 4);
                let swatch_y = row_top + (ROW_HEIGHT - size) / 2;
                let swatch_rect = Rect::new(name_x, swatch_y, size as u32, size as u32);
                fill_rect(surface, swatch_rect, color, gamma);
                draw_border(surface, swatch_rect, COLOR_SWATCH_BORDER, gamma);
                name_x += size + 8;
            }

            lc_frontend::draw_text_with_gamma(
                font,
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
                gamma,
            );

            lc_frontend::draw_text_with_gamma(
                font,
                surface,
                outcome_column_x as f32,
                text_y,
                entry.outcome.label(),
                ROW_FONT_SIZE,
                entry.outcome.label_color(),
                gamma,
            );

            draw_stat(surface, font, wealth_column_x, text_y, entry.wealth, gamma);
            draw_stat(surface, font, score_column_x, text_y, entry.score, gamma);
            draw_stat(surface, font, value_column_x, text_y, entry.value, gamma);
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
                if self.hovered_button == Some(index) {
                    BUTTON_SELECTED_COLOR
                } else {
                    BUTTON_COLOR
                },
                gamma,
            );
            draw_border(surface, rect, BUTTON_BORDER_COLOR, gamma);
            draw_text_centered(
                surface,
                font,
                &button.label.replace('&', ""),
                BUTTON_FONT_SIZE,
                TEXT_COLOR,
                rect.x,
                rect.x + rect.width as i32,
                rect.y + (BUTTON_HEIGHT - BUTTON_FONT_SIZE.ceil() as i32) / 2,
                gamma,
            );
        }

        let footer_y = panel_rect.y as f32 + panel_rect.height as f32
            - PANEL_PADDING as f32
            - FOOTER_FONT_SIZE;
        draw_text_centered(
            surface,
            font,
            self.hovered_description(),
            FOOTER_FONT_SIZE,
            MUTED_TEXT_COLOR,
            content_left,
            content_right,
            footer_y as i32,
            gamma,
        );
    }

    fn render_classic(
        &self,
        surface: &mut Surface,
        resources: GameOverClassicResources<'_>,
        gamma: Option<&GammaRamp>,
    ) {
        let layout = self.classic_layout(surface.width(), surface.height(), resources.fonts);
        resources.skin.draw_dialog(surface, layout.dialog, gamma);
        resources.skin.draw_caption(
            surface,
            layout.caption,
            CLASSIC_DIALOG_TITLE,
            &resources.fonts.text,
            [0xff, 0xff, 0xff, 0xff],
            TextAlign::Left,
            gamma,
        );
        self.render_classic_close_button(surface, resources, layout.close_button, gamma);

        self.render_classic_evaluation(surface, resources, gamma);
        for (index, (button, rect)) in self.buttons.iter().zip(layout.buttons).enumerate() {
            resources.skin.draw_button(
                surface,
                rect,
                &button.label,
                resources.fonts,
                ClassicButtonState {
                    pressed: self.pressed_button == Some(index)
                        && self.hovered_button == Some(index),
                    highlighted: self.hovered_button == Some(index),
                },
                gamma,
            );
        }
    }

    fn render_classic_close_button(
        &self,
        surface: &mut Surface,
        resources: GameOverClassicResources<'_>,
        rect: IntRect,
        gamma: Option<&GammaRamp>,
    ) {
        let hovered = self
            .pointer_position
            .is_some_and(|(x, y)| point_in_rect(x, y, surface_rect(rect)));
        let draw_highlight = |surface: &mut Surface| {
            if let Some(highlight) = resources.icon_button_highlight {
                lc_frontend::draw_image_bilinear_additive(
                    surface,
                    &lc_gui::Rect::new(rect.x as f32, rect.y as f32, rect.w as f32, rect.h as f32),
                    highlight,
                    gamma,
                );
            }
        };
        if hovered {
            draw_highlight(surface);
        }
        let icon = resources
            .gui_icons
            .and_then(|icons| crop_image(icons, CLASSIC_CLOSE_ICON_SOURCE))
            .map(|icon| lc_frontend::classic_gui::blacken_transparent_pixels(&icon));
        if let Some(icon) = icon.as_ref() {
            draw_classic_image(surface, icon, rect, gamma);
        }
        if hovered && self.close_pressed {
            draw_highlight(surface);
        }
    }

    fn render_classic_evaluation(
        &self,
        surface: &mut Surface,
        resources: GameOverClassicResources<'_>,
        gamma: Option<&GammaRamp>,
    ) {
        let layout =
            self.classic_evaluation_layout(surface.width(), surface.height(), resources.fonts);
        for (goal, goal_layout) in self.evaluation.goals().iter().zip(&layout.goals) {
            if let Some(picture) = goal.picture.as_ref() {
                let grayscale = (!goal.fulfilled).then(|| grayscale_image(picture, 30));
                draw_classic_image(
                    surface,
                    grayscale.as_ref().unwrap_or(picture),
                    goal_layout.picture,
                    gamma,
                );
            }
            if goal.fulfilled {
                let star = resources
                    .gui_icons
                    .and_then(|icons| crop_image(icons, CLASSIC_FULFILLED_STAR_SOURCE))
                    .map(|star| lc_frontend::classic_gui::blacken_transparent_pixels(&star));
                if let Some(star) = star.as_ref() {
                    draw_classic_image(surface, star, goal_layout.fulfilled_star, gamma);
                }
            }
        }

        for (player, player_layout) in self.evaluation.players().zip(&layout.players) {
            lc_frontend::classic_gui::draw_engine_box(
                surface,
                player_layout.row.x,
                player_layout.row.y,
                player_layout.row.x + player_layout.row.w - 1,
                player_layout.row.y + player_layout.row.h - 1,
                if player.won {
                    0x4faf_7a00
                } else {
                    0x7faf_afaf
                },
                gamma,
            );

            let owner_color = Color::opaque(
                ((player.color_dw >> 16) & 0xff) as u8,
                ((player.color_dw >> 8) & 0xff) as u8,
                (player.color_dw & 0xff) as u8,
            );
            let fallback_icon = resources
                .player
                .map(|image| lc_frontend::hud::colorize_by_owner(image, owner_color));
            if let Some(icon) = player.big_icon.as_ref().or(fallback_icon.as_ref()) {
                draw_classic_image(surface, icon, player_layout.icon, gamma);
            }

            draw_clonk_text(
                surface,
                &resources.fonts.text,
                player_layout.name_anchor.0,
                player_layout.name_anchor.1,
                &format!(
                    "{} ({})",
                    player.name,
                    if player.won { "won" } else { "lost" }
                ),
                if player.won {
                    Color::opaque(0xff, 0xdf, 0x00)
                } else {
                    Color::opaque(0xff, 0xff, 0xff)
                },
                TextAlign::Left,
                gamma,
            );

            if player.score_old >= 0 {
                render_settlement_score(
                    surface,
                    resources.fonts,
                    resources.score,
                    player_layout.score_anchor,
                    player.score_old,
                    player.score_new,
                    gamma,
                );
            }
            let total = player.total_playing_time;
            draw_clonk_text(
                surface,
                &resources.fonts.text,
                player_layout.time_anchor.0,
                player_layout.time_anchor.1,
                &format!(
                    "Total playing time: {:02}:{:02}:{:02}",
                    total / 3_600,
                    (total / 60) % 60,
                    total % 60
                ),
                Color::opaque(0xff, 0xff, 0xff),
                TextAlign::Right,
                gamma,
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

fn draw_classic_image(
    surface: &mut Surface,
    image: &ImageData,
    rect: IntRect,
    gamma: Option<&GammaRamp>,
) {
    lc_frontend::draw_image_bilinear(
        surface,
        &lc_gui::Rect::new(
            rect.x as f32,
            rect.y as f32,
            rect.w.max(0) as f32,
            rect.h.max(0) as f32,
        ),
        image,
        gamma,
    );
}

fn crop_image(image: &ImageData, source: IntRect) -> Option<ImageData> {
    let x = u32::try_from(source.x).ok()?;
    let y = u32::try_from(source.y).ok()?;
    let width = u32::try_from(source.w).ok()?;
    let height = u32::try_from(source.h).ok()?;
    (width > 0
        && height > 0
        && x.checked_add(width)? <= image.width()
        && y.checked_add(height)? <= image.height())
    .then(|| {
        let source_pixels = image.pixels();
        let mut pixels = Vec::with_capacity((width * height * 4) as usize);
        for row in y..y + height {
            let start = ((row * image.width() + x) * 4) as usize;
            pixels.extend_from_slice(&source_pixels[start..start + (width * 4) as usize]);
        }
        ImageData::new(width, height, pixels)
    })
}

fn grayscale_image(image: &ImageData, offset: i32) -> ImageData {
    // CStdDDraw::Grayscale averages the three channels, adds 30 for goal
    // pictures and clamps, preserving alpha (StdDDraw2.cpp:1241-1260).
    let mut pixels = image.pixels().to_vec();
    for pixel in pixels.chunks_exact_mut(4) {
        let gray = ((i32::from(pixel[0]) + i32::from(pixel[1]) + i32::from(pixel[2])) / 3
            + offset)
            .clamp(0, 255) as u8;
        pixel[..3].fill(gray);
    }
    ImageData::new(image.width(), image.height(), pixels)
}

fn render_settlement_score(
    surface: &mut Surface,
    fonts: &ClonkFontSet,
    score_icon: Option<&ImageData>,
    anchor: (i32, i32),
    score_old: i32,
    score_new: Option<i32>,
    gamma: Option<&GammaRamp>,
) {
    // C4PlayerInfoListBox::UpdateScoreLabel emits Ico:Settlement followed by
    // gray old/gain and white new score. CStdFont scales the inline image to
    // iGfxLineHgt while preserving aspect (C4PlayerInfoListBox.cpp:404-413;
    // StdFont.cpp:845-896).
    let text = score_new.map_or_else(
        || format!("<c afafaf>({score_old})</c> Score"),
        |score_new| {
            format!(
                "<c afafaf>{score_old} ({:+})</c> {score_new} Score",
                score_new - score_old
            )
        },
    );
    let text_width = fonts.text.measure(&text, true).0;
    let (icon_width, icon_height) = score_icon.map_or((0, 0), |icon| {
        let height = fonts.text.cell_height;
        (
            (icon.width() as i32 * height / icon.height().max(1) as i32).max(1),
            height,
        )
    });
    let icon_advance = if icon_width > 0 {
        icon_width + fonts.text.h_space
    } else {
        0
    };
    let x = anchor.0 - text_width - icon_advance;
    if let Some(icon) = score_icon {
        draw_classic_image(
            surface,
            icon,
            IntRect {
                x,
                y: anchor.1,
                w: icon_width,
                h: icon_height,
            },
            gamma,
        );
    }
    draw_clonk_text(
        surface,
        &fonts.text,
        x + icon_advance,
        anchor.1,
        &text,
        Color::opaque(0xff, 0xff, 0xff),
        TextAlign::Left,
        gamma,
    );
}

fn draw_clonk_text(
    surface: &mut Surface,
    font: &lc_graphics::clonk_font::ClonkFont,
    x: i32,
    y: i32,
    text: &str,
    color: Color,
    align: TextAlign,
    gamma: Option<&GammaRamp>,
) {
    font.draw_with_gamma(
        surface,
        x,
        y,
        text,
        [color.r, color.g, color.b, color.a],
        align,
        true,
        gamma,
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
    gamma: Option<&GammaRamp>,
) {
    let header_y = baseline_y as f32;
    lc_frontend::draw_text_with_gamma(
        font,
        surface,
        name_x as f32,
        header_y,
        "Player",
        HEADER_FONT_SIZE,
        HEADER_COLOR,
        gamma,
    );
    lc_frontend::draw_text_with_gamma(
        font,
        surface,
        outcome_x as f32,
        header_y,
        "Outcome",
        HEADER_FONT_SIZE,
        HEADER_COLOR,
        gamma,
    );
    draw_header_stat(surface, font, wealth_x, header_y, "Wealth", gamma);
    draw_header_stat(surface, font, score_x, header_y, "Score", gamma);
    draw_header_stat(surface, font, value_x, header_y, "Value", gamma);
}

fn draw_header_stat(
    surface: &mut Surface,
    font: &dyn TextFont,
    column_x: i32,
    baseline: f32,
    label: &str,
    gamma: Option<&GammaRamp>,
) {
    let metrics = font.measure_text(label, HEADER_FONT_SIZE);
    let x = column_x as f32 + STAT_COLUMN_WIDTH as f32 - metrics.width;
    lc_frontend::draw_text_with_gamma(
        font,
        surface,
        x,
        baseline,
        label,
        HEADER_FONT_SIZE,
        HEADER_COLOR,
        gamma,
    );
}

fn draw_stat(
    surface: &mut Surface,
    font: &dyn TextFont,
    column_x: i32,
    y: f32,
    value: i32,
    gamma: Option<&GammaRamp>,
) {
    let text = format!("{value}");
    let metrics = font.measure_text(&text, ROW_FONT_SIZE);
    let x = column_x as f32 + STAT_COLUMN_WIDTH as f32 - metrics.width;
    lc_frontend::draw_text_with_gamma(font, surface, x, y, &text, ROW_FONT_SIZE, TEXT_COLOR, gamma);
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
    gamma: Option<&GammaRamp>,
) {
    let metrics = font.measure_text(text, size);
    let width = metrics.width;
    let x = (left as f32 + right as f32 - width) * 0.5;
    lc_frontend::draw_text_with_gamma(font, surface, x, baseline as f32, text, size, color, gamma);
}

fn fill_rect(surface: &mut Surface, rect: Rect, color: Color, gamma: Option<&GammaRamp>) {
    lc_frontend::draw_color_rect(surface, rect, color, gamma);
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
    fn tutorial_seven_gamma_encodes_the_game_over_button_fragment() {
        // C4GUI::Screen renders the evaluation dialog before the gamma latch
        // at the end of C4GraphicsSystem::Execute (C4GraphicsSystem.cpp:
        // 187-199), so even fallback solid GUI fragments use Tutorial07's
        // already-active scenario ramp.
        let mut state = GameOverState::new(
            "Tutorial 7".to_string(),
            vec![entry(1, "Player", GameOverOutcome::Victory, true)],
        );
        let mut surface = Surface::new(1024, 600, lc_graphics::PixelFormat::Rgba8888);
        let gamma = crate::tutorial_seven_gamma();
        state.render_with_gamma(
            &mut surface,
            &lc_graphics::BitmapFont::new(),
            None,
            Some(&gamma),
        );

        let first = state.button_rects(surface.width(), surface.height())[0];
        let probe = (first.x as u32 + 2, first.y as u32 + 2);
        assert_eq!(
            surface.get_pixel(probe.0, probe.1),
            Some(crate::tutorial_seven_gamma_color(BUTTON_COLOR)),
        );

        state.handle_pointer_move(
            (first.x + first.width as i32 / 2) as f32,
            (first.y + first.height as i32 / 2) as f32,
            surface.width(),
            surface.height(),
        );
        state.render_with_gamma(
            &mut surface,
            &lc_graphics::BitmapFont::new(),
            None,
            Some(&gamma),
        );
        assert_eq!(
            surface.get_pixel(probe.0, probe.1),
            Some(crate::tutorial_seven_gamma_color(BUTTON_SELECTED_COLOR)),
        );
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
            None,
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
        assert_eq!(state.hovered_action(), None);
        assert_eq!(state.hovered_description(), "");
    }

    #[test]
    fn wide_game_over_keeps_restart_without_inventing_initial_focus() {
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

        assert_eq!(state.hovered_action(), None);
        let next = state.button_rects(1280, 720)[3];
        state.handle_pointer_move(
            (next.x + next.width as i32 / 2) as f32,
            (next.y + next.height as i32 / 2) as f32,
            1280,
            720,
        );
        assert_eq!(state.hovered_action(), Some(GameOverAction::NextMission));
        assert_eq!(state.hovered_description(), "Continue learning");
        state.handle_pointer_move(0.0, 0.0, 1280, 720);
        assert_eq!(state.hovered_action(), None);
        assert_eq!(state.hovered_description(), "");
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
    fn classic_zero_goal_evaluation_starts_player_list_at_main_area_top() {
        // With no goals, C4GameOverDlg does not call GetFromTop/ExpandTop;
        // caPlayerArea therefore receives caMain.GetAll() directly. The list
        // itself contributes only its 3px client margin before the first row
        // (C4GameOverDlg.cpp:151-168,214-229; C4Gui.cpp:1041-1047;
        // C4GuiListBox.h:108-122; C4GuiListBox.cpp:405-459,532-544).
        let fonts = endeavour_fonts();
        let mut state = GameOverState::with_next_mission(
            "A Clonk".into(),
            Vec::new(),
            1024,
            Some(NextMissionButton {
                label: "Next tutorial".into(),
                description: "Continue learning".into(),
            }),
        );
        state.set_evaluation(EvaluationViewModel::new(
            Vec::new(),
            vec![evaluation_player(41, "Player")],
        ));

        let layout = state.classic_evaluation_layout(1024, 600, &fonts);
        assert_eq!(layout.goal_display, None);
        assert_eq!(layout.player_list, IntRect { x: 15, y: 34, w: 994, h: 487 });
        assert_eq!(layout.players[0].row, IntRect { x: 21, y: 37, w: 969, h: 54 });

        let caption = solid_image(192, 23, [200, 0, 0, 255]);
        let button = solid_image(128, 32, [0, 120, 0, 255]);
        let button_down = solid_image(128, 32, [0, 0, 180, 255]);
        let skin = ClassicGuiSkin::new(&caption, &button, &button_down, None);
        let background = Color::opaque(11, 22, 33);
        let mut surface = Surface::new(1024, 600, lc_graphics::PixelFormat::Rgba8888);
        surface.fill(background);
        state.render(
            &mut surface,
            &lc_graphics::BitmapFont::new(),
            Some(GameOverClassicResources::new(
                skin, &fonts, None, None, None, None,
            )),
        );

        let mut expected = Surface::new(1, 1, lc_graphics::PixelFormat::Rgba8888);
        expected.fill(background);
        lc_frontend::classic_gui::draw_engine_box(
            &mut expected,
            0,
            0,
            0,
            0,
            lc_frontend::classic_gui::STANDARD_BACKGROUND_COLOR,
            None,
        );
        lc_frontend::classic_gui::draw_engine_box(
            &mut expected,
            0,
            0,
            0,
            0,
            0x4faf_7a00,
            None,
        );
        assert_eq!(
            surface.get_pixel(500, 37),
            expected.get_pixel(0, 0),
            "the zero-goal winner row begins at the C++ list client top"
        );
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
        state.hovered_button = Some(1);
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
                Some(&highlight),
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
        let idle = layout.buttons[0];
        assert_eq!(
            surface.get_pixel((idle.x + 8) as u32, (idle.y + 5) as u32),
            Some(Color::opaque(0, 120, 0)),
            "the initial no-focus button uses GUIButton without a highlight"
        );
        let pressed = layout.buttons[1];
        assert_eq!(
            surface.get_pixel((pressed.x + 8) as u32, (pressed.y + 5) as u32),
            Some(Color::opaque(80, 0, 180)),
            "hovered pressed button uses GUIButtonDown plus highlight"
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
    fn classic_render_draws_cpp_goal_and_winner_row_instead_of_the_invented_table() {
        // C4GoalDisplay draws each buffered 64x64 goal and overlays Ico_Star
        // in its lower-right half. The evaluation PlayerListItem is 54px
        // high and uses the winning engine box/name treatment, player icon,
        // cumulative time and settlement-score line
        // (C4GameOverDlg.cpp:25-78,145-220; C4PlayerInfoListBox.cpp:72-154,
        // 344-425,651-680).
        let fonts = endeavour_fonts();
        let caption = solid_image(192, 23, [200, 0, 0, 255]);
        let button = solid_image(128, 32, [0, 120, 0, 255]);
        let button_down = solid_image(128, 32, [0, 0, 180, 255]);
        let highlight = solid_image(16, 16, [80, 0, 0, 255]);
        let goal_picture = solid_image(64, 64, [220, 10, 20, 255]);
        let mut gui_icon_pixels = vec![0; 240 * 360 * 4];
        for y in 320..360 {
            for x in 0..40 {
                let offset = ((y * 240 + x) * 4) as usize;
                gui_icon_pixels[offset..offset + 4].copy_from_slice(&[0, 230, 40, 255]);
            }
        }
        let gui_icons = ImageData::new(240, 360, gui_icon_pixels);
        let player = solid_image(48, 48, [0, 0, 255, 255]);
        let score = solid_image(60, 30, [0, 210, 240, 255]);
        let skin = ClassicGuiSkin::new(&caption, &button, &button_down, Some(&highlight));

        let mut state = GameOverState::with_next_mission(
            "decoy scenario title".into(),
            vec![entry(99, "decoy table row", GameOverOutcome::Victory, true)],
            1024,
            Some(NextMissionButton {
                label: "Next tutorial".into(),
                description: "Continue learning".into(),
            }),
        );
        state.set_evaluation(EvaluationViewModel::new(
            vec![EvaluationGoal {
                definition_id: "SCRG".into(),
                fulfilled: true,
                picture: Some(goal_picture),
            }],
            vec![EvaluationPlayer {
                player_info_id: 41,
                name: "Player".into(),
                won: true,
                color_dw: 0x00e8_0000,
                total_playing_time: 3_661,
                score_old: 10,
                score_new: Some(110),
                custom_evaluation_strings: String::new(),
                big_icon: None,
            }],
        ));
        let layout = state.classic_evaluation_layout(1024, 600, &fonts);
        let background = Color::opaque(11, 22, 33);
        let mut surface = Surface::new(1024, 600, lc_graphics::PixelFormat::Rgba8888);
        surface.fill(background);
        state.render(
            &mut surface,
            &lc_graphics::BitmapFont::new(),
            Some(GameOverClassicResources::new(
                skin,
                &fonts,
                Some(&highlight),
                Some(&gui_icons),
                Some(&player),
                Some(&score),
            )),
        );

        let goal = layout.goals[0];
        assert_eq!(
            surface.get_pixel((goal.picture.x + 12) as u32, (goal.picture.y + 12) as u32),
            Some(Color::opaque(220, 10, 20)),
            "the SCRG definition picture fills the classic 64px goal facet"
        );
        assert_eq!(
            surface.get_pixel(
                (goal.fulfilled_star.x + 16) as u32,
                (goal.fulfilled_star.y + 16) as u32
            ),
            Some(Color::opaque(0, 230, 40)),
            "fulfilled SCRG receives GUIIcons Ico_Star in its lower-right half"
        );

        let row = layout.players[0].row;
        let mut expected = Surface::new(1, 1, lc_graphics::PixelFormat::Rgba8888);
        expected.fill(background);
        lc_frontend::classic_gui::draw_engine_box(
            &mut expected,
            0,
            0,
            0,
            0,
            lc_frontend::classic_gui::STANDARD_BACKGROUND_COLOR,
            None,
        );
        lc_frontend::classic_gui::draw_engine_box(
            &mut expected,
            0,
            0,
            0,
            0,
            0x4faf_7a00,
            None,
        );
        assert_eq!(
            surface.get_pixel((row.x + 400) as u32, (row.y + row.h / 2) as u32),
            expected.get_pixel(0, 0),
            "winner row uses C4GUI_WinningBackgroundColor"
        );
        assert_eq!(
            surface.get_pixel(
                (layout.players[0].icon.x + 20) as u32,
                (layout.players[0].icon.y + 20) as u32
            ),
            Some(Color::opaque(232, 0, 0)),
            "the default Player.png icon is ColorByOwner-tinted"
        );
        assert!(
            (row.y..row.y + fonts.text.cell_height).any(|y| {
                (row.x + row.w - 300..row.x + row.w).any(|x| {
                    surface.get_pixel(x as u32, y as u32)
                        == Some(Color::opaque(0, 210, 240))
                })
            }),
            "the settlement score line contains the Score.png inline icon"
        );
        assert_eq!(
            surface.get_pixel(100, 75),
            surface.get_pixel(100, 110),
            "the fabricated Player/Outcome/Wealth/Score/Value table is gone"
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

    #[test]
    fn classic_buttons_start_unhighlighted_and_only_pointer_hover_highlights() {
        // C4GUI keeps keyboard focus null when the evaluation dialog opens.
        // Pointer hover is a separate visual state and disappears on leave.
        let fonts = endeavour_fonts();
        let caption = solid_image(192, 23, [20, 30, 40, 255]);
        let button = solid_image(128, 32, [0, 120, 0, 255]);
        let button_down = solid_image(128, 32, [0, 0, 180, 255]);
        let highlight = solid_image(16, 16, [40, 50, 60, 255]);
        let skin = ClassicGuiSkin::new(&caption, &button, &button_down, Some(&highlight));
        let resources = GameOverClassicResources::new(
            skin,
            &fonts,
            Some(&highlight),
            None,
            None,
            None,
        );
        let mut state = GameOverState::new(
            "A Clonk".into(),
            vec![entry(1, "Player", GameOverOutcome::Victory, true)],
        );
        state.configure_classic_fonts(Some(&fonts));
        let layout = state.classic_layout(1024, 600, &fonts);
        let first = layout.buttons[0];
        let second = layout.buttons[1];
        let render = |state: &GameOverState| {
            let mut surface = Surface::new(1024, 600, lc_graphics::PixelFormat::Rgba8888);
            surface.fill(Color::opaque(3, 4, 5));
            state.render(
                &mut surface,
                &lc_graphics::BitmapFont::new(),
                Some(resources),
            );
            surface
        };

        assert_eq!(state.hovered_action(), None);
        let idle = render(&state);
        state.handle_pointer_move(
            (first.x + first.w / 2) as f32,
            (first.y + first.h / 2) as f32,
            1024,
            600,
        );
        assert_eq!(state.hovered_action(), Some(GameOverAction::End));
        let hovered = render(&state);
        let first_probe = ((first.x + 8) as u32, (first.y + 5) as u32);
        let second_probe = ((second.x + 8) as u32, (second.y + 5) as u32);
        assert_ne!(
            hovered.get_pixel(first_probe.0, first_probe.1),
            idle.get_pixel(first_probe.0, first_probe.1),
            "pointer hover adds the classic button highlight"
        );
        assert_eq!(
            hovered.get_pixel(second_probe.0, second_probe.1),
            idle.get_pixel(second_probe.0, second_probe.1),
            "hover does not invent focus on another button"
        );

        state.handle_pointer_down(1024, 600);
        let pressed = render(&state);
        assert_eq!(state.pressed_button, Some(0));
        assert_ne!(
            pressed.get_pixel(first_probe.0, first_probe.1),
            hovered.get_pixel(first_probe.0, first_probe.1),
            "pointer down selects GUIButtonDown while still over the button"
        );
        state.handle_pointer_move(0.0, 0.0, 1024, 600);
        let dragged_out = render(&state);
        assert_eq!(
            state.pressed_button,
            Some(0),
            "drag-out retains the same-target release latch"
        );
        assert_eq!(
            dragged_out.get_pixel(first_probe.0, first_probe.1),
            idle.get_pixel(first_probe.0, first_probe.1),
            "drag-out raises the latched button"
        );
        state.handle_pointer_move(
            (first.x + first.w / 2) as f32,
            (first.y + first.h / 2) as f32,
            1024,
            600,
        );
        let dragged_back = render(&state);
        assert_eq!(
            dragged_back.get_pixel(first_probe.0, first_probe.1),
            pressed.get_pixel(first_probe.0, first_probe.1),
            "re-entry depresses the still-latched button again"
        );
        assert_eq!(
            state.handle_pointer_up(1024, 600),
            Some(GameOverAction::End)
        );

        state.pointer_left();
        assert_eq!(state.hovered_action(), None);
        let left = render(&state);
        assert_eq!(left.pixels(), idle.pixels(), "pointer leave restores idle pixels");
    }

    #[test]
    fn classic_caption_close_matches_cpp_icon_button_and_ends_round() {
        // Dialog::SetTitle places a 16x16 Ico_Close four pixels from the
        // caption's top-right corner. IconButton draws hover highlight, then
        // GUIIcons phase 34, and a second additive highlight while pressed;
        // releasing it closes C4GameOverDlg as an unsuccessful (End) result
        // (C4GuiDialogs.cpp:397-421; C4Gui.cpp:363-370;
        // C4GuiLabels.cpp:441-450; C4GuiButton.cpp:203-225;
        // C4GameOverDlg.cpp:360-388).
        let fonts = endeavour_fonts();
        let caption = solid_image(192, 23, [20, 30, 40, 255]);
        let button = solid_image(128, 32, [0, 120, 0, 255]);
        let button_down = solid_image(128, 32, [0, 0, 180, 255]);
        let highlight = solid_image(16, 16, [40, 50, 60, 255]);
        let mut gui_icon_pixels = vec![0; 240 * 360 * 4];
        for pixel in gui_icon_pixels.chunks_exact_mut(4) {
            pixel.copy_from_slice(&[220, 0, 0, 255]);
        }
        for y in 200..240 {
            for x in 160..200 {
                let offset = ((y * 240 + x) * 4) as usize;
                let color = if (168..192).contains(&x) && (208..232).contains(&y) {
                    [10, 220, 30, 255]
                } else {
                    [0, 0, 0, 0]
                };
                gui_icon_pixels[offset..offset + 4].copy_from_slice(&color);
            }
        }
        let gui_icons = ImageData::new(240, 360, gui_icon_pixels);
        let skin = ClassicGuiSkin::new(&caption, &button, &button_down, Some(&highlight));
        let mut state = GameOverState::new(
            "A Clonk".into(),
            vec![entry(1, "Player", GameOverOutcome::Victory, true)],
        );
        state.configure_classic_fonts(Some(&fonts));
        let layout = state.classic_layout(1024, 600, &fonts);

        assert_eq!(
            CLASSIC_CLOSE_ICON_SOURCE,
            IntRect {
                x: 160,
                y: 200,
                w: 40,
                h: 40,
            },
            "Ico_Close is GUIIcons phase 34 in the six-column 40px atlas"
        );
        assert_eq!(
            layout.close_button,
            IntRect {
                x: layout.caption.x + layout.caption.w - 20,
                y: layout.caption.y + 4,
                w: 16,
                h: 16,
            }
        );

        let resources = GameOverClassicResources::new(
            skin,
            &fonts,
            Some(&highlight),
            Some(&gui_icons),
            None,
            None,
        );
        let background = Color::opaque(3, 4, 5);
        let render = |state: &GameOverState| {
            let mut surface = Surface::new(1024, 600, lc_graphics::PixelFormat::Rgba8888);
            surface.fill(background);
            state.render(
                &mut surface,
                &lc_graphics::BitmapFont::new(),
                Some(resources),
            );
            surface
        };
        let close = layout.close_button;
        let center = (close.x + close.w / 2, close.y + close.h / 2);
        let idle = render(&state);
        assert_eq!(
            idle.get_pixel(center.0 as u32, center.1 as u32),
            Some(Color::opaque(10, 220, 30)),
            "the close control crops GUIIcons phase 34"
        );

        state.handle_pointer_move((close.x + 1) as f32, (close.y + 1) as f32, 1024, 600);
        let hovered = render(&state);
        assert_ne!(
            hovered.get_pixel((close.x + 1) as u32, (close.y + 1) as u32),
            idle.get_pixel((close.x + 1) as u32, (close.y + 1) as u32),
            "hover draws GUIButtonHighlight behind the transparent icon edge"
        );

        state.handle_pointer_move(center.0 as f32, center.1 as f32, 1024, 600);
        state.handle_pointer_down(1024, 600);
        let pressed = render(&state);
        assert_ne!(
            pressed.get_pixel(center.0 as u32, center.1 as u32),
            idle.get_pixel(center.0 as u32, center.1 as u32),
            "pressed IconButton adds a second highlight over the icon"
        );
        assert_eq!(
            state.handle_pointer_up(1024, 600),
            Some(GameOverAction::End)
        );
    }
}
