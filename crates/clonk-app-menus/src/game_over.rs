use clonk_engine::RoundResultsNetworkResult;
use clonk_frontend::classic_gui::{ClassicButtonState, ClassicGuiSkin, IntRect};
use clonk_frontend::{expand_hotkey_markup, ClonkFontSet, ImageData};
use clonk_graphics::clonk_font::TextAlign;
use clonk_graphics::{Color, GammaRamp, Rect, Surface, TextFont};

const CLASSIC_DIALOG_TITLE: &str = "Evaluation";
const CLASSIC_MIN_CAPTION_HEIGHT: i32 = 23;
const CLASSIC_BUTTON_HEIGHT: i32 = 32;
const CLASSIC_INDENT_X: i32 = 10;
/// `C4SymbolSize`, the evaluation `TeamListItem` icon extent
/// (src/C4PlayerInfoListBox.cpp:1020-1025).
const CLASSIC_TEAM_HEADER_ICON: i32 = 35;
/// `C4GUI_ScrollBarWdt` (src/C4Gui.h) — the column `ScrollWindow` reserves and
/// the width of every `GUIScroll.png` facet cell.
use crate::scrollbar::{
    draw_classic_scrollbar, pin_offset as scrollbar_pin_offset_for,
    SCROLLBAR_EXTENT as CLASSIC_SCROLLBAR_EXTENT,
};
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

/// The real `C4GUI::Dialog` focus owner for the evaluation dialog.
///
/// The caption close icon is constructed first, followed by the (selection-
/// disabled, but still focusable) player list and then the visible buttons.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum GameOverFocus {
    Close,
    PlayerList(usize),
    Button(usize),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum GameOverActivationKey {
    Confirm,
    Space,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum GameOverSound {
    ArrowHit,
    Click,
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
    /// `GoalPicture::SetToolTip`: the localized fulfilled/unfulfilled text
    /// frozen with the definition name and description.
    pub tooltip: String,
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
    pub team_id: Option<i32>,
    pub name: String,
    pub won: bool,
    pub color_dw: u32,
    pub total_playing_time: u32,
    pub score_old: i32,
    pub score_new: Option<i32>,
    pub custom_evaluation_strings: String,
    pub big_icon: Option<ImageData>,
    /// `C4PlayerInfo::getLeagueScore()` — the league score the player carried
    /// into this round. `None` is C++'s falsy zero.
    pub league_score_old: Option<i32>,
    /// `C4RoundResultsPlayer::GetLeagueScoreGain()`.
    pub league_score_gain: Option<i32>,
    /// `C4RoundResultsPlayer::GetLeagueScoreNew()`; `None` is
    /// `!IsLeagueScoreNewValid()`.
    pub league_score_new: Option<i32>,
    /// `GetJoinedInfo()`'s lobby colour: the row's own colour when this *is* a
    /// free savegame player, otherwise the colour of the savegame player it
    /// took over, resolved through `Game.RestorePlayerInfos`
    /// (src/C4PlayerInfoListBox.cpp:701-716). `None` means "not joined", which
    /// suppresses the Crew overlay entirely.
    pub joined_color_dw: Option<u32>,
    /// One through nine, matching `Ico_Rank1..Ico_Rank9`. Resolved the way
    /// `UpdateScoreLabel` does: the frozen result's
    /// `GetLeagueRankSymbolNew()` when its league score is valid, otherwise
    /// the live `C4PlayerInfo::getLeagueRankSymbol()`; zero hides the icon
    /// (src/C4PlayerInfoListBox.cpp:439-456).
    pub league_rank_symbol: Option<u8>,
}

/// Which inline icon `UpdateScoreLabel` prefixes the score with
/// (src/C4PlayerInfoListBox.cpp:380-413).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum EvaluationScoreIcon {
    League,
    Settlement,
}

/// `C4PlayerInfoListBox::PlayerListItem::UpdateScoreLabel`'s evaluation branch
/// (src/C4PlayerInfoListBox.cpp:376-418). League state wins over settlement
/// state, and a settlement score is suppressed entirely once
/// `C4RoundResults::SettlementScoreIsHidden()`.
///
/// `score_text` is the localized `IDS_TEXT_SCORE` word appended to every
/// variant.
pub fn evaluation_score_label(
    player: &EvaluationPlayer,
    score_text: &str,
) -> Option<(EvaluationScoreIcon, String)> {
    let league_old = player.league_score_old.unwrap_or(0);
    if player.league_score_old.is_some() || player.league_score_new.is_some() {
        let text = match (player.league_score_new, player.league_score_gain) {
            (Some(new), gain) => {
                let gain = gain.unwrap_or(0);
                // The league server normally guarantees old + gain == new; a
                // discrepancy means an admin intervened and is shown in red.
                let discrepancy = new - (league_old + gain);
                if discrepancy == 0 {
                    format!("<c afafaf>{league_old} ({gain:+})</c> {new} {score_text}")
                } else {
                    format!(
                        "<c afafaf>{league_old} ({gain:+})</c><c ff0000>({discrepancy:+})</c> {new} {score_text}"
                    )
                }
            }
            (None, _) => format!("<c afafaf>({league_old})</c> {score_text}"),
        };
        return Some((EvaluationScoreIcon::League, text));
    }
    // A hidden settlement score reaches this projection as `score_old < 0`.
    if player.score_old < 0 {
        return None;
    }
    let text = match player.score_new {
        Some(new) => format!(
            "<c afafaf>{} ({:+})</c> {new} {score_text}",
            player.score_old,
            new - player.score_old
        ),
        None => format!("<c afafaf>({})</c> {score_text}", player.score_old),
    };
    Some((EvaluationScoreIcon::Settlement, text))
}

/// One `C4PlayerInfoListBox::TeamListItem` header
/// (src/C4PlayerInfoListBox.cpp:996-1035,1100-1115). During evaluation the
/// icon is the team's `IconSpec` drawn in the team colour when one is
/// declared, otherwise the generic `Ico_Team`, and the label takes the
/// winning or losing text colour from `C4Team::HasWon()`.
#[derive(Clone, Debug, PartialEq)]
pub struct EvaluationTeam {
    pub id: i32,
    pub name: String,
    pub color_dw: u32,
    /// The team's `IconSpec` image already resolved and colourized, mirroring
    /// `DrawTextSpecImage(pIcon->GetMFacet(), IconSpec, GetColor())` at
    /// construction time. `None` falls back to the generic `Ico_Team` facet.
    pub icon: Option<ImageData>,
    pub won: bool,
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
    custom_evaluation_strings: String,
    separate_team_ids: Option<[i32; 2]>,
    team_order: Vec<i32>,
    teams: Vec<EvaluationTeam>,
    /// `Game.Parameters.isLeague()` — the only condition under which a
    /// `PlayerListItem` gets a rank icon at all
    /// (src/C4PlayerInfoListBox.cpp:94-95).
    league: bool,
}

impl EvaluationViewModel {
    pub fn new(goals: Vec<EvaluationGoal>, players: Vec<EvaluationPlayer>) -> Self {
        Self {
            goals,
            players,
            custom_evaluation_strings: String::new(),
            separate_team_ids: None,
            team_order: Vec::new(),
            teams: Vec::new(),
            league: false,
        }
    }

    pub fn with_dialog_context(
        mut self,
        custom_evaluation_strings: String,
        separate_team_ids: Option<[i32; 2]>,
    ) -> Self {
        self.custom_evaluation_strings = custom_evaluation_strings;
        self.separate_team_ids = separate_team_ids;
        self
    }

    pub fn with_team_order(mut self, team_order: impl IntoIterator<Item = i32>) -> Self {
        self.team_order = team_order.into_iter().collect();
        self
    }

    pub fn with_teams(mut self, teams: impl IntoIterator<Item = EvaluationTeam>) -> Self {
        self.teams = teams.into_iter().collect();
        self
    }

    pub fn team(&self, id: i32) -> Option<&EvaluationTeam> {
        self.teams.iter().find(|team| team.id == id)
    }

    pub fn with_league(mut self, league: bool) -> Self {
        self.league = league;
        self
    }

    pub const fn is_league(&self) -> bool {
        self.league
    }

    pub fn goals(&self) -> &[EvaluationGoal] {
        &self.goals
    }

    pub fn players(&self) -> impl ExactSizeIterator<Item = &EvaluationPlayer> {
        self.players.iter()
    }

    pub fn custom_evaluation_strings(&self) -> &str {
        &self.custom_evaluation_strings
    }

    pub fn separate_team_ids(&self) -> Option<[i32; 2]> {
        self.separate_team_ids
    }

    fn player(&self, index: usize) -> Option<&EvaluationPlayer> {
        self.players.get(index)
    }

    fn player_list_count(&self) -> usize {
        usize::from(self.separate_team_ids.is_some()) + 1
    }

    fn player_indices_in_team_order(&self) -> Vec<usize> {
        let mut indices = self
            .team_order
            .iter()
            .flat_map(|team_id| {
                self.players
                    .iter()
                    .enumerate()
                    .filter_map(move |(index, player)| {
                        (player.team_id == Some(*team_id)).then_some(index)
                    })
            })
            .collect::<Vec<_>>();
        indices.extend(
            self.players
                .iter()
                .enumerate()
                .filter_map(|(index, player)| {
                    player
                        .team_id
                        .is_none_or(|team_id| !self.team_order.contains(&team_id))
                        .then_some(index)
                }),
        );
        indices
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
    scroll: Option<&'a ImageData>,
    crew: Option<&'a ImageData>,
}

impl<'a> GameOverClassicResources<'a> {
    #[allow(clippy::too_many_arguments)]
    pub const fn new(
        skin: ClassicGuiSkin<'a>,
        fonts: &'a ClonkFontSet,
        icon_button_highlight: Option<&'a ImageData>,
        gui_icons: Option<&'a ImageData>,
        player: Option<&'a ImageData>,
        score: Option<&'a ImageData>,
        scroll: Option<&'a ImageData>,
        crew: Option<&'a ImageData>,
    ) -> Self {
        Self {
            skin,
            fonts,
            icon_button_highlight,
            gui_icons,
            player,
            score,
            scroll,
            crew,
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
    pub player_index: usize,
    pub player_list_index: usize,
    pub row: IntRect,
    pub icon: IntRect,
    pub name_anchor: (i32, i32),
    pub score_anchor: (i32, i32),
    pub time_anchor: (i32, i32),
    pub custom_evaluation_anchor: Option<(i32, i32)>,
    /// The league rank icon's own right-hand column during evaluation
    /// (`caBounds.GetFromRight(caBounds.GetInnerHeight())`,
    /// src/C4PlayerInfoListBox.cpp:199-206). Present only in a league game.
    pub rank_icon: Option<IntRect>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ClassicEvaluationTextLayout {
    pub area: IntRect,
    pub viewport: IntRect,
    pub content_height: i32,
    pub scrollable: bool,
    /// `C4GUI::ScrollWindow`'s reserved `C4GUI_ScrollBarWdt` column. Present
    /// only for the overflowing `TextWindow` variant, which is the only one
    /// C++ gives a bar (src/C4GameOverDlg.cpp:196-206).
    pub scrollbar: Option<IntRect>,
}

/// One `TeamListItem` row at the top of a team-filtered evaluation list box
/// (src/C4PlayerInfoListBox.cpp:1536-1541).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ClassicEvaluationTeamHeaderLayout {
    pub player_list_index: usize,
    pub team_id: i32,
    pub icon: IntRect,
    pub name_anchor: (i32, i32),
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ClassicEvaluationLayout {
    pub goal_display: Option<IntRect>,
    pub goals: Vec<ClassicEvaluationGoalLayout>,
    pub network_result: Option<IntRect>,
    pub custom_evaluation: Option<ClassicEvaluationTextLayout>,
    pub player_lists: Vec<IntRect>,
    pub team_headers: Vec<ClassicEvaluationTeamHeaderLayout>,
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
    host_or_cinematic_film: bool,
    hovered_goal: Option<usize>,
    hovered_button: Option<usize>,
    pressed_button: Option<usize>,
    close_pressed: bool,
    focused: Option<GameOverFocus>,
    down_controls: Vec<GameOverFocus>,
    sounds: Vec<GameOverSound>,
    classic_button_width: Option<i32>,
    classic_text_font: Option<clonk_graphics::clonk_font::ClonkFont>,
    network_result_label: Option<String>,
    is_net_done: bool,
    quit_buttons_visible: bool,
    quit_allowed: bool,
    show_winners: bool,
    custom_evaluation_scroll: i32,
    pointer_position: Option<(f32, f32)>,
    pointer_surface_size: Option<(u32, u32)>,
}

impl GameOverState {
    pub fn new(title: String, entries: Vec<GameOverEntry>, host_or_cinematic_film: bool) -> Self {
        Self::with_next_mission(title, entries, u32::MAX, None, host_or_cinematic_film)
    }

    pub fn with_next_mission(
        title: String,
        mut entries: Vec<GameOverEntry>,
        screen_width: u32,
        next_mission: Option<NextMissionButton>,
        host_or_cinematic_film: bool,
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
        // C4GameOverDlg constructs these controls only for the control host
        // or a cinematic film (Head.Film == 2). The same predicate also owns
        // the dialog's expanded 1280x720 chrome below.
        if host_or_cinematic_film {
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
        }

        Self {
            title,
            subtitle,
            entries,
            evaluation: EvaluationViewModel::default(),
            buttons,
            host_or_cinematic_film,
            hovered_goal: None,
            hovered_button: None,
            pressed_button: None,
            close_pressed: false,
            focused: None,
            down_controls: Vec::new(),
            sounds: Vec::new(),
            classic_button_width: None,
            classic_text_font: None,
            network_result_label: None,
            is_net_done: true,
            quit_buttons_visible: true,
            quit_allowed: true,
            show_winners: true,
            custom_evaluation_scroll: 0,
            pointer_position: None,
            pointer_surface_size: None,
        }
    }

    pub fn configure_classic_fonts(&mut self, fonts: Option<&ClonkFontSet>) {
        self.classic_button_width = fonts.map(classic_button_width);
        self.classic_text_font = fonts.map(|fonts| fonts.text.clone());
    }

    pub fn subtitle(&self) -> &str {
        &self.subtitle
    }

    pub fn set_evaluation(&mut self, evaluation: EvaluationViewModel) {
        self.evaluation = evaluation;
        self.hovered_goal = None;
        self.custom_evaluation_scroll = 0;
    }

    pub fn evaluation(&self) -> &EvaluationViewModel {
        &self.evaluation
    }

    /// Construct the network-result child and run the dialog's first
    /// `Update()`. C++ creates this label only for a league game or a result
    /// that already exists; a result arriving later cannot add it.
    pub fn initialize_network_result(
        &mut self,
        present: bool,
        is_host: bool,
        result_text: &str,
        result: Option<RoundResultsNetworkResult>,
        pending_stream_data: usize,
        is_streaming: bool,
    ) {
        self.network_result_label = present.then(String::new);
        self.is_net_done = !present;
        self.quit_buttons_visible = true;
        self.quit_allowed = false;
        self.show_winners = true;
        self.update_network_result(
            is_host,
            result_text,
            result,
            pending_stream_data,
            is_streaming,
        );
    }

    /// Mirror `C4GameOverDlg::Update`/`SetNetResult`, including its literal
    /// initial visibility latch: controls are born visible, so an initially
    /// pending host still has clickable End/Continue buttons while Escape is
    /// ignored until the result and record stream are both complete.
    pub fn update_network_result(
        &mut self,
        is_host: bool,
        result_text: &str,
        result: Option<RoundResultsNetworkResult>,
        pending_stream_data: usize,
        is_streaming: bool,
    ) -> bool {
        let before = (
            self.network_result_label.clone(),
            self.is_net_done,
            self.quit_buttons_visible,
            self.quit_allowed,
            self.show_winners,
        );
        if let Some(label) = self.network_result_label.as_mut() {
            label.clear();
            label.push_str(result_text);
            if is_streaming {
                label.push_str("|[!]Transmitting record to league server... (");
                label.push_str(&(pending_stream_data / 1024).to_string());
                label.push_str(" kb remaining)");
            }
            if result.is_some() && !is_streaming {
                self.is_net_done = true;
            }
            if result == Some(RoundResultsNetworkResult::NetworkError) {
                self.show_winners = false;
            }
        }

        let quit_allowed = self.is_net_done || !is_host;
        if quit_allowed != self.quit_allowed {
            self.quit_allowed = quit_allowed;
            self.quit_buttons_visible = quit_allowed;
            if !quit_allowed {
                let buttons = &self.buttons;
                let is_visible = |index: usize| {
                    buttons.get(index).is_some_and(|button| {
                        !matches!(
                            button.action,
                            GameOverAction::End | GameOverAction::Continue
                        )
                    })
                };
                self.hovered_button = self.hovered_button.filter(|index| is_visible(*index));
                self.pressed_button = self.pressed_button.filter(|index| is_visible(*index));
                self.down_controls.retain(|focus| match *focus {
                    GameOverFocus::Button(index) => is_visible(index),
                    _ => true,
                });
            }
        }
        before
            != (
                self.network_result_label.clone(),
                self.is_net_done,
                self.quit_buttons_visible,
                self.quit_allowed,
                self.show_winners,
            )
    }

    pub fn network_result_label(&self) -> Option<&str> {
        self.network_result_label.as_deref()
    }

    pub fn is_net_done(&self) -> bool {
        self.is_net_done
    }

    pub fn allows_escape_close(&self) -> bool {
        self.quit_allowed
    }

    #[cfg(test)]
    fn shows_winners(&self) -> bool {
        self.show_winners
    }

    pub fn custom_evaluation_scroll(&self) -> i32 {
        self.custom_evaluation_scroll
    }

    #[allow(dead_code)]
    pub fn entries(&self) -> &[GameOverEntry] {
        &self.entries
    }

    pub fn actions(&self) -> Vec<GameOverAction> {
        self.buttons
            .iter()
            .enumerate()
            .filter(|(index, _)| self.button_is_visible(*index))
            .map(|(_, button)| button.action)
            .collect()
    }

    pub fn set_button_content(
        &mut self,
        action: GameOverAction,
        label: String,
        description: String,
    ) {
        if let Some(button) = self
            .buttons
            .iter_mut()
            .find(|button| button.action == action)
        {
            button.label = label;
            button.description = description;
        }
    }

    pub fn focused(&self) -> Option<GameOverFocus> {
        self.focused
    }

    pub fn focused_action(&self) -> Option<GameOverAction> {
        self.focused.and_then(|focus| self.action_for_focus(focus))
    }

    pub fn advance_focus(&mut self, backwards: bool) -> GameOverFocus {
        let player_list_count = self.evaluation.player_list_count();
        let mut order = Vec::with_capacity(self.buttons.len() + player_list_count + 1);
        order.push(GameOverFocus::Close);
        order.extend((0..player_list_count).map(GameOverFocus::PlayerList));
        order.extend(
            (0..self.buttons.len())
                .filter(|index| self.button_is_visible(*index))
                .map(GameOverFocus::Button),
        );
        let focus_count = order.len();
        let current = self
            .focused
            .and_then(|focus| order.iter().position(|candidate| *candidate == focus));
        let index = match current {
            Some(0) if backwards => focus_count - 1,
            Some(index) if backwards => index - 1,
            Some(index) => (index + 1) % focus_count,
            None if backwards => focus_count - 1,
            None => 0,
        };
        let focus = order[index];
        self.focused = Some(focus);
        focus
    }

    /// Mirrors the focused `C4GUI::Button` key binding. Space, Return and
    /// gamepad Low all mutate the focused button's one native `fDown` latch.
    pub fn handle_activation_down(&mut self, _key: GameOverActivationKey) -> bool {
        let Some(focus) = self
            .focused
            .filter(|focus| self.action_for_focus(*focus).is_some())
        else {
            return false;
        };
        self.set_down(focus);
        true
    }

    pub fn handle_activation_up(&mut self, _key: GameOverActivationKey) -> Option<GameOverAction> {
        let focus = self.focused?;
        let action = self.action_for_focus(focus)?;
        self.set_up(focus, true).then_some(action)
    }

    /// Dialog mnemonics invoke `OnPress` directly and therefore do not alter
    /// focus/down state or emit the button's ArrowHit/Click sounds.
    pub fn hotkey_action(&self, hotkey: char) -> Option<GameOverAction> {
        let hotkey = hotkey.to_ascii_uppercase();
        self.buttons
            .iter()
            .enumerate()
            .filter(|(index, _)| self.button_is_visible(*index))
            .find_map(|(_, button)| {
                (expand_hotkey_markup(&button.label).1 == Some(hotkey)).then_some(button.action)
            })
    }

    pub fn cancel_interaction(&mut self) {
        self.pressed_button = None;
        self.close_pressed = false;
        self.down_controls.clear();
    }

    pub fn take_sound_events(&mut self) -> Vec<GameOverSound> {
        std::mem::take(&mut self.sounds)
    }

    pub fn hovered_description(&self) -> &str {
        self.hovered_button
            .and_then(|index| self.buttons.get(index))
            .map(|button| button.description.as_str())
            .or_else(|| {
                self.hovered_goal
                    .and_then(|index| self.evaluation.goals().get(index))
                    .map(|goal| goal.tooltip.as_str())
            })
            .unwrap_or("")
    }

    /// Resolve the tooltip target from the process-global pointer position.
    ///
    /// Higher dialogs can consume pointer motion without forwarding it to
    /// this dialog, so rendering must not trust the cached hover left by the
    /// last event that did reach the evaluation screen.
    pub fn tooltip_at(&self, x: f32, y: f32, surface_width: u32, surface_height: u32) -> &str {
        if let Some(index) = self
            .button_rects(surface_width, surface_height)
            .iter()
            .enumerate()
            .find_map(|(index, rect)| {
                (self.button_is_visible(index) && point_in_rect(x, y, *rect)).then_some(index)
            })
        {
            return &self.buttons[index].description;
        }
        let Some((button_width, text_font)) = self
            .classic_button_width
            .zip(self.classic_text_font.as_ref())
        else {
            return "";
        };
        self.classic_evaluation_layout_with_metrics(
            surface_width,
            surface_height,
            button_width,
            text_font,
        )
        .goals
        .iter()
        .position(|goal| point_in_rect(x, y, surface_rect(goal.picture)))
        .and_then(|index| self.evaluation.goals().get(index))
        .map(|goal| goal.tooltip.as_str())
        .unwrap_or("")
    }

    pub fn hovered_action(&self) -> Option<GameOverAction> {
        self.hovered_button
            .and_then(|index| self.buttons.get(index))
            .map(|button| button.action)
    }

    pub fn handle_pointer_move(&mut self, x: f32, y: f32, surface_width: u32, surface_height: u32) {
        self.pointer_surface_size = Some((surface_width, surface_height));
        let pressed = self.pointer_pressed_focus();
        self.pointer_position = Some((x, y));
        self.hovered_button = self
            .button_rects(surface_width, surface_height)
            .iter()
            .enumerate()
            .find_map(|(index, rect)| {
                (self.button_is_visible(index) && point_in_rect(x, y, *rect)).then_some(index)
            });
        self.hovered_goal = match (self.classic_button_width, self.classic_text_font.as_ref()) {
            (Some(button_width), Some(text_font)) => self
                .classic_evaluation_layout_with_metrics(
                    surface_width,
                    surface_height,
                    button_width,
                    text_font,
                )
                .goals
                .iter()
                .position(|goal| point_in_rect(x, y, surface_rect(goal.picture))),
            _ => None,
        };
        if let Some(focus) = pressed {
            if self.pointer_is_over_focus(focus) {
                self.set_down(focus);
            } else {
                self.set_up(focus, false);
            }
        }
    }

    pub fn pointer_left(&mut self) {
        let pressed = self.pointer_pressed_focus();
        self.pointer_position = None;
        self.hovered_goal = None;
        self.hovered_button = None;
        self.pressed_button = None;
        self.close_pressed = false;
        if let Some(focus) = pressed {
            self.set_up(focus, false);
        }
    }

    /// Hit-test the actual evaluation dialog chassis. The running Screen may
    /// keep an active exclusive dialog fullscreen-blocking, but lower z=0
    /// dialogs must use these bounds so uncovered screen areas fall through.
    pub fn classic_dialog_contains_point(
        &self,
        x: f32,
        y: f32,
        surface_width: u32,
        surface_height: u32,
    ) -> bool {
        // The chassis dimensions do not depend on the measured button width,
        // so this remains exact even before the first resource-backed render.
        let rect = surface_rect(
            self.classic_layout_with_button_width(
                surface_width,
                surface_height,
                self.classic_button_width.unwrap_or(1),
            )
            .dialog,
        );
        point_in_rect(x, y, rect)
    }

    pub fn has_pointer_capture(&self) -> bool {
        self.pointer_pressed_focus().is_some()
    }

    pub fn handle_pointer_down(&mut self, surface_width: u32, surface_height: u32) {
        self.pointer_surface_size = Some((surface_width, surface_height));
        if let Some(index) = self.pointer_position.and_then(|(x, y)| {
            self.classic_player_list_rects(surface_width, surface_height)
                .iter()
                .position(|rect| point_in_rect(x, y, *rect))
        }) {
            self.focused = Some(GameOverFocus::PlayerList(index));
        }
        self.close_pressed = self.pointer_position.is_some_and(|(x, y)| {
            self.classic_close_button_rect(surface_width, surface_height)
                .is_some_and(|rect| point_in_rect(x, y, rect))
        });
        if self.close_pressed {
            self.pressed_button = None;
            self.set_down(GameOverFocus::Close);
            return;
        }
        self.pressed_button = self.pointer_position.and_then(|(x, y)| {
            self.button_rects(surface_width, surface_height)
                .iter()
                .enumerate()
                .find_map(|(index, rect)| {
                    (self.button_is_visible(index) && point_in_rect(x, y, *rect)).then_some(index)
                })
        });
        if let Some(index) = self.pressed_button {
            self.set_down(GameOverFocus::Button(index));
        }
    }

    pub fn handle_pointer_up(
        &mut self,
        surface_width: u32,
        surface_height: u32,
    ) -> Option<GameOverAction> {
        self.pointer_surface_size = Some((surface_width, surface_height));
        if std::mem::take(&mut self.close_pressed) {
            return self
                .set_up(GameOverFocus::Close, true)
                .then_some(GameOverAction::End);
        }
        let pressed = self.pressed_button.take()?;
        let focus = GameOverFocus::Button(pressed);
        let action = self.buttons.get(pressed).map(|button| button.action)?;
        self.set_up(focus, true).then_some(action)
    }

    fn action_for_focus(&self, focus: GameOverFocus) -> Option<GameOverAction> {
        match focus {
            GameOverFocus::Close => Some(GameOverAction::End),
            GameOverFocus::PlayerList(_) => None,
            GameOverFocus::Button(index) if self.button_is_visible(index) => {
                self.buttons.get(index).map(|button| button.action)
            }
            GameOverFocus::Button(_) => None,
        }
    }

    fn button_is_visible(&self, index: usize) -> bool {
        self.buttons.get(index).is_some_and(|button| {
            self.quit_buttons_visible
                || !matches!(
                    button.action,
                    GameOverAction::End | GameOverAction::Continue
                )
        })
    }

    fn pointer_pressed_focus(&self) -> Option<GameOverFocus> {
        if self.close_pressed {
            Some(GameOverFocus::Close)
        } else {
            self.pressed_button.map(GameOverFocus::Button)
        }
    }

    fn focus_is_down(&self, focus: GameOverFocus) -> bool {
        self.down_controls.contains(&focus)
    }

    fn set_down(&mut self, focus: GameOverFocus) {
        if !self.focus_is_down(focus) {
            self.down_controls.push(focus);
            self.sounds.push(GameOverSound::ArrowHit);
        }
    }

    fn set_up(&mut self, focus: GameOverFocus, pressed: bool) -> bool {
        let Some(index) = self.down_controls.iter().position(|down| *down == focus) else {
            return false;
        };
        self.down_controls.swap_remove(index);
        self.sounds.push(if pressed {
            GameOverSound::Click
        } else {
            GameOverSound::ArrowHit
        });
        true
    }

    fn pointer_is_over_focus(&self, focus: GameOverFocus) -> bool {
        match focus {
            GameOverFocus::Close => self.pointer_position.is_some_and(|(x, y)| {
                self.classic_close_button_rect_for_pointer()
                    .is_some_and(|rect| point_in_rect(x, y, rect))
            }),
            GameOverFocus::PlayerList(_) => false,
            GameOverFocus::Button(index) => self.hovered_button == Some(index),
        }
    }

    fn classic_close_button_rect_for_pointer(&self) -> Option<Rect> {
        let (width, height) = self.pointer_surface_size?;
        self.classic_close_button_rect(width, height)
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
            .enumerate()
            .find_map(|(index, rect)| {
                (self.button_is_visible(index) && point_in_rect(x, y, *rect)).then_some(index)
            })
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

    fn classic_player_list_rects(&self, surface_width: u32, surface_height: u32) -> Vec<Rect> {
        let (Some(button_width), Some(text_font)) =
            (self.classic_button_width, self.classic_text_font.as_ref())
        else {
            return Vec::new();
        };
        self.classic_evaluation_layout_with_metrics(
            surface_width,
            surface_height,
            button_width,
            text_font,
        )
        .player_lists
        .into_iter()
        .map(surface_rect)
        .collect()
    }

    pub fn handle_wheel(&mut self, delta: i32, surface_width: u32, surface_height: u32) -> bool {
        if delta == 0 {
            return false;
        }
        let (Some(button_width), Some(text_font), Some((x, y))) = (
            self.classic_button_width,
            self.classic_text_font.as_ref(),
            self.pointer_position,
        ) else {
            return false;
        };
        let layout = self.classic_evaluation_layout_with_metrics(
            surface_width,
            surface_height,
            button_width,
            text_font,
        );
        let Some(text) = layout
            .custom_evaluation
            .filter(|text| text.scrollable && point_in_rect(x, y, surface_rect(text.viewport)))
        else {
            return false;
        };
        let maximum = (text.content_height - text.viewport.h).max(0);
        let before = self.custom_evaluation_scroll;
        self.custom_evaluation_scroll = self
            .custom_evaluation_scroll
            .saturating_sub(delta)
            .clamp(0, maximum);
        self.custom_evaluation_scroll != before
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
        let (width_threshold, width_cap, height_threshold, height_cap) =
            if self.host_or_cinematic_film {
                (1280, 1280, 720, 720)
            } else {
                (800, 800, 600, 600)
            };
        let dialog_width = if screen_width < width_threshold {
            screen_width - 10
        } else {
            (screen_width - 150).min(width_cap)
        }
        .max(1);
        let dialog_height = if screen_height < height_threshold {
            screen_height - 10
        } else {
            (screen_height - 150).min(height_cap)
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

    pub fn classic_evaluation_layout(
        &self,
        surface_width: u32,
        surface_height: u32,
        fonts: &ClonkFontSet,
    ) -> ClassicEvaluationLayout {
        self.classic_evaluation_layout_with_metrics(
            surface_width,
            surface_height,
            classic_button_width(fonts),
            &fonts.text,
        )
    }

    fn classic_evaluation_layout_with_metrics(
        &self,
        surface_width: u32,
        surface_height: u32,
        button_width: i32,
        text_font: &clonk_graphics::clonk_font::ClonkFont,
    ) -> ClassicEvaluationLayout {
        let chrome =
            self.classic_layout_with_button_width(surface_width, surface_height, button_width);
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
                    x: chrome.dialog.x
                        + (chrome.dialog.w - group_width) / 2
                        + column * goal_area_height
                        + CLASSIC_GOAL_MARGIN,
                    y: chrome.player_area.y + row * goal_area_height + CLASSIC_GOAL_MARGIN,
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
        let mut player_list_y =
            chrome.player_area.y + goal_rows * goal_area_height + post_goal_inset;
        let player_area_bottom = chrome.player_area.y + chrome.player_area.h;
        let network_result = self.network_result_label.as_ref().map(|_| {
            let width = (chrome.dialog.w - 6 * CLASSIC_INDENT_X).max(0);
            let area = IntRect {
                x: chrome.dialog.x + (chrome.dialog.w - width) / 2,
                y: player_list_y,
                w: width,
                h: text_font.line_height * 2,
            };
            player_list_y += area.h + 2 * CLASSIC_INDENT_Y;
            area
        });
        let custom_evaluation =
            (!self.evaluation.custom_evaluation_strings().is_empty()).then(|| {
                let width = (chrome.dialog.w - 6 * CLASSIC_INDENT_X).max(0);
                let area_x = chrome.dialog.x + (chrome.dialog.w - width) / 2;
                let measured_lines = classic_multiline_label_lines(
                    text_font,
                    self.evaluation.custom_evaluation_strings(),
                    width.max(1),
                );
                let measured_height = classic_multiline_label_height(&measured_lines, text_font);
                let maximum_height = (player_area_bottom - player_list_y).max(0) / 3;
                let scrollable = measured_height > maximum_height;
                let area = IntRect {
                    x: area_x,
                    y: player_list_y,
                    w: width,
                    h: if scrollable {
                        maximum_height
                    } else {
                        measured_height
                    },
                };
                if scrollable {
                    // TextWindow keeps its 10/5px horizontal and 8px vertical
                    // client margins and always reserves the 16px ScrollBar.
                    let viewport = IntRect {
                        x: area.x + 10,
                        y: area.y + 8,
                        w: (area.w - 10 - 5 - 16).max(0),
                        h: (area.h - 16).max(0),
                    };
                    let lines = classic_multiline_label_lines(
                        text_font,
                        self.evaluation.custom_evaluation_strings(),
                        viewport.w.max(1),
                    );
                    ClassicEvaluationTextLayout {
                        area,
                        viewport,
                        content_height: classic_multiline_label_height(&lines, text_font),
                        scrollable: true,
                        scrollbar: Some(IntRect {
                            x: area.x + area.w - CLASSIC_SCROLLBAR_EXTENT,
                            y: area.y,
                            w: CLASSIC_SCROLLBAR_EXTENT,
                            h: area.h,
                        }),
                    }
                } else {
                    ClassicEvaluationTextLayout {
                        area,
                        viewport: area,
                        content_height: measured_height,
                        scrollable: false,
                        scrollbar: None,
                    }
                }
            });
        if let Some(custom_evaluation) = custom_evaluation {
            // GetFromTop(0, width) consumes caMain's 6px margins above and
            // below before ExpandTop reserves the label's measured height.
            player_list_y += custom_evaluation.area.h + 2 * CLASSIC_INDENT_Y;
        }

        let player_list_height = (player_area_bottom - player_list_y).max(0);
        let player_lists = if self.evaluation.separate_team_ids.is_some() {
            let cell_width = ((chrome.dialog.w - CLASSIC_INDENT_X) / 2 - CLASSIC_INDENT_X).max(0);
            (0..2)
                .map(|index| IntRect {
                    x: chrome.dialog.x + CLASSIC_INDENT_X + index * (cell_width + CLASSIC_INDENT_X),
                    y: player_list_y,
                    w: cell_width,
                    h: player_list_height,
                })
                .collect::<Vec<_>>()
        } else {
            vec![IntRect {
                x: chrome.player_area.x,
                y: player_list_y,
                w: chrome.player_area.w,
                h: player_list_height,
            }]
        };

        let split = self.evaluation.separate_team_ids;
        let mut players = Vec::new();
        let mut team_headers = Vec::new();
        for (player_list_index, player_list) in player_lists.iter().copied().enumerate() {
            let player_indices = if let Some(team_ids) = split {
                self.evaluation
                    .players()
                    .enumerate()
                    .filter_map(|(index, player)| {
                        (player.team_id == Some(team_ids[player_list_index])).then_some(index)
                    })
                    .collect::<Vec<_>>()
            } else if self.show_winners {
                [true, false]
                    .into_iter()
                    .flat_map(|won| {
                        self.evaluation
                            .players()
                            .enumerate()
                            .filter_map(move |(index, player)| (player.won == won).then_some(index))
                    })
                    .collect::<Vec<_>>()
            } else {
                self.evaluation.player_indices_in_team_order()
            };
            let row_width =
                (player_list.w - CLASSIC_PLAYER_ROW_LEFT_INSET - CLASSIC_PLAYER_ROW_RIGHT_INSET)
                    .max(0);
            let mut row_y = player_list.y + CLASSIC_PLAYER_ROW_TOP_INSET;
            // A team-filtered evaluation list emits one TeamListItem before its
            // players (src/C4PlayerInfoListBox.cpp:1536-1541). Its icon is a
            // C4SymbolSize square and the label is centred against it
            // (`:1014-1027`).
            if let Some(team_ids) = split {
                let icon = IntRect {
                    x: player_list.x + CLASSIC_PLAYER_ROW_LEFT_INSET,
                    y: row_y,
                    w: CLASSIC_TEAM_HEADER_ICON,
                    h: CLASSIC_TEAM_HEADER_ICON,
                };
                team_headers.push(ClassicEvaluationTeamHeaderLayout {
                    player_list_index,
                    team_id: team_ids[player_list_index],
                    icon,
                    name_anchor: (
                        icon.x + icon.w + CLASSIC_PLAYER_LABEL_SPACING,
                        icon.y + (icon.h - text_font.line_height) / 2,
                    ),
                });
                row_y += CLASSIC_TEAM_HEADER_ICON + CLASSIC_PLAYER_LABEL_SPACING;
            }
            for player_index in player_indices {
                let player = self
                    .evaluation
                    .player(player_index)
                    .expect("evaluation layout index remains valid");
                let has_custom_evaluation = !player.custom_evaluation_strings.is_empty();
                let row_height = CLASSIC_PLAYER_ROW_HEIGHT
                    + i32::from(has_custom_evaluation) * text_font.line_height;
                let row = IntRect {
                    x: player_list.x + CLASSIC_PLAYER_ROW_LEFT_INSET,
                    y: row_y,
                    w: row_width,
                    h: row_height,
                };
                // In a league game the rank icon claims a square column at
                // the row's right edge, and the score label's right edge is
                // measured against it instead of the row width
                // (src/C4PlayerInfoListBox.cpp:199-206,352).
                let rank_icon = self.evaluation.is_league().then(|| IntRect {
                    x: row.x + row.w - row.h,
                    y: row.y,
                    w: row.h,
                    h: row.h,
                });
                let right_anchor =
                    rank_icon.map_or(row.x + row.w, |rank| rank.x) - CLASSIC_PLAYER_LABEL_SPACING;
                let score_y = if split.is_some() {
                    row.y + row.h
                        - (text_font.line_height + 4) * (1 + i32::from(has_custom_evaluation))
                        - CLASSIC_PLAYER_LABEL_SPACING
                } else {
                    row.y + CLASSIC_PLAYER_LABEL_SPACING
                };
                players.push(ClassicEvaluationPlayerLayout {
                    player_index,
                    player_list_index,
                    row,
                    icon: IntRect {
                        w: CLASSIC_PLAYER_ROW_HEIGHT,
                        h: CLASSIC_PLAYER_ROW_HEIGHT,
                        ..row
                    },
                    name_anchor: (
                        row.x + row.h + CLASSIC_PLAYER_LABEL_SPACING,
                        row.y + CLASSIC_PLAYER_LABEL_SPACING,
                    ),
                    score_anchor: (right_anchor, score_y),
                    time_anchor: (
                        right_anchor,
                        row.y + text_font.line_height + if has_custom_evaluation { 0 } else { 4 },
                    ),
                    rank_icon,
                    custom_evaluation_anchor: has_custom_evaluation.then_some((
                        right_anchor,
                        row.y + row.h - text_font.line_height - CLASSIC_PLAYER_LABEL_SPACING,
                    )),
                });
                row_y += row_height + (CLASSIC_PLAYER_ROW_STEP - CLASSIC_PLAYER_ROW_HEIGHT);
            }
        }

        ClassicEvaluationLayout {
            goal_display,
            goals,
            network_result,
            custom_evaluation,
            player_lists,
            team_headers,
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
        self.render_with_gamma_active(surface, font, classic, gamma, true, true);
    }

    pub fn render_with_gamma_active(
        &self,
        surface: &mut Surface,
        font: &dyn TextFont,
        classic: Option<GameOverClassicResources<'_>>,
        gamma: Option<&GammaRamp>,
        focus_active: bool,
        mouse_active: bool,
    ) {
        if surface.width() == 0 || surface.height() == 0 {
            return;
        }

        if let Some(classic) = classic {
            self.render_classic(surface, classic, gamma, focus_active, mouse_active);
        } else {
            self.render_fallback(surface, font, gamma, focus_active, mouse_active);
        }
    }

    fn render_fallback(
        &self,
        surface: &mut Surface,
        font: &dyn TextFont,
        gamma: Option<&GammaRamp>,
        focus_active: bool,
        mouse_active: bool,
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

            clonk_frontend::draw_text_with_gamma(
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

            clonk_frontend::draw_text_with_gamma(
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
            if !self.button_is_visible(index) {
                continue;
            }
            fill_rect(
                surface,
                rect,
                if (mouse_active && self.hovered_button == Some(index))
                    || (focus_active && self.focused == Some(GameOverFocus::Button(index)))
                {
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
        focus_active: bool,
        mouse_active: bool,
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
        self.render_classic_close_button(
            surface,
            resources,
            layout.close_button,
            gamma,
            focus_active,
            mouse_active,
        );

        self.render_classic_evaluation(surface, resources, gamma);
        for (index, (button, rect)) in self.buttons.iter().zip(layout.buttons).enumerate() {
            if !self.button_is_visible(index) {
                continue;
            }
            resources.skin.draw_button(
                surface,
                rect,
                &button.label,
                resources.fonts,
                ClassicButtonState {
                    pressed: self.focus_is_down(GameOverFocus::Button(index)),
                    highlighted: (mouse_active && self.hovered_button == Some(index))
                        || (focus_active && self.focused == Some(GameOverFocus::Button(index))),
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
        focus_active: bool,
        mouse_active: bool,
    ) {
        let hovered = self
            .pointer_position
            .is_some_and(|(x, y)| point_in_rect(x, y, surface_rect(rect)));
        let draw_highlight = |surface: &mut Surface| {
            if let Some(highlight) = resources.icon_button_highlight {
                clonk_frontend::draw_image_bilinear_additive(
                    surface,
                    &clonk_gui::Rect::new(
                        rect.x as f32,
                        rect.y as f32,
                        rect.w as f32,
                        rect.h as f32,
                    ),
                    highlight,
                    gamma,
                );
            }
        };
        if (mouse_active && hovered) || (focus_active && self.focused == Some(GameOverFocus::Close))
        {
            draw_highlight(surface);
        }
        let icon = resources
            .gui_icons
            .and_then(|icons| crop_image(icons, CLASSIC_CLOSE_ICON_SOURCE))
            .map(|icon| clonk_frontend::classic_gui::blacken_transparent_pixels(&icon));
        if let Some(icon) = icon.as_ref() {
            draw_classic_image(surface, icon, rect, gamma);
        }
        if self.focus_is_down(GameOverFocus::Close) {
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
                    .map(|star| clonk_frontend::classic_gui::blacken_transparent_pixels(&star));
                if let Some(star) = star.as_ref() {
                    draw_classic_image(surface, star, goal_layout.fulfilled_star, gamma);
                }
            }
        }

        if let (Some(result_layout), Some(result)) =
            (layout.network_result, self.network_result_label.as_deref())
        {
            let broken = clonk_frontend::message_dialog::break_message(
                &resources.fonts.text,
                result,
                result_layout.w.max(1),
            );
            draw_clonk_text(
                surface,
                &resources.fonts.text,
                result_layout.x + result_layout.w / 2,
                result_layout.y,
                &broken,
                Color::opaque(0xff, 0xff, 0x00),
                TextAlign::Center,
                gamma,
            );
        }

        if let Some(text_layout) = layout.custom_evaluation {
            let lines = classic_multiline_label_lines(
                &resources.fonts.text,
                self.evaluation.custom_evaluation_strings(),
                text_layout.viewport.w.max(1),
            );
            let maximum_scroll = (text_layout.content_height - text_layout.viewport.h).max(0);
            let scroll = if text_layout.scrollable {
                self.custom_evaluation_scroll.min(maximum_scroll)
            } else {
                0
            };
            let previous_clip = surface.clip();
            let viewport = surface_rect(text_layout.viewport);
            let active_clip = previous_clip
                .and_then(|clip| clip.intersection(viewport))
                .unwrap_or_else(|| {
                    if previous_clip.is_some() {
                        Rect::new(viewport.x, viewport.y, 0, 0)
                    } else {
                        viewport
                    }
                });
            surface.set_clip(active_clip);
            let mut y = text_layout.viewport.y - scroll;
            for (index, line) in lines.iter().enumerate() {
                if index > 0 && line.starts_paragraph {
                    y += resources.fonts.text.line_height / 3;
                }
                draw_clonk_text(
                    surface,
                    &resources.fonts.text,
                    text_layout.viewport.x,
                    y,
                    &line.text,
                    Color::opaque(0xff, 0xff, 0xff),
                    TextAlign::Left,
                    gamma,
                );
                y += resources.fonts.text.line_height;
            }
            match previous_clip {
                Some(clip) => surface.set_clip(clip),
                None => surface.clear_clip(),
            }
            // The overflowing variant is a real C4GUI::TextWindow, whose
            // ScrollWindow reserves and operates a ScrollBar
            // (src/C4GameOverDlg.cpp:196-206).
            if let (Some(bar), Some(scroll_facet)) = (text_layout.scrollbar, resources.scroll) {
                draw_classic_scrollbar(
                    surface,
                    bar,
                    scroll_facet,
                    scrollbar_pin_offset(bar, scroll, maximum_scroll),
                    maximum_scroll,
                    gamma,
                );
            }
        }

        // TeamListItem headers precede their team's rows and take the winning
        // or losing text colour from C4Team::HasWon()
        // (src/C4PlayerInfoListBox.cpp:996-1035,1100-1115).
        for header in &layout.team_headers {
            let Some(team) = self.evaluation.team(header.team_id) else {
                continue;
            };
            let team_color = Color::opaque(
                ((team.color_dw >> 16) & 0xff) as u8,
                ((team.color_dw >> 8) & 0xff) as u8,
                (team.color_dw & 0xff) as u8,
            );
            // `DrawTextSpecImage` colours a declared IconSpec by the team
            // colour; without one the generic Ico_Team facet is drawn as-is.
            let fallback = resources
                .player
                .map(|image| clonk_frontend::hud::colorize_by_owner(image, team_color));
            if let Some(icon) = team.icon.as_ref().or(fallback.as_ref()) {
                draw_classic_image(surface, icon, header.icon, gamma);
            }
            draw_clonk_text(
                surface,
                &resources.fonts.text,
                header.name_anchor.0,
                header.name_anchor.1,
                &team.name,
                if team.won {
                    Color::opaque(0xff, 0xdf, 0x00)
                } else {
                    Color::opaque(0xff, 0xff, 0xff)
                },
                TextAlign::Left,
                gamma,
            );
        }

        let split_player_lists = layout.player_lists.len() == 2;
        for player_layout in &layout.players {
            let player = self
                .evaluation
                .player(player_layout.player_index)
                .expect("evaluation render index remains valid");
            clonk_frontend::classic_gui::draw_engine_box(
                surface,
                player_layout.row.x,
                player_layout.row.y,
                player_layout.row.x + player_layout.row.w - 1,
                player_layout.row.y + player_layout.row.h - 1,
                if self.show_winners && player.won {
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
                .map(|image| clonk_frontend::hud::colorize_by_owner(image, owner_color));
            if let Some(icon) = player.big_icon.as_ref().or(fallback_icon.as_ref()) {
                draw_classic_image(surface, icon, player_layout.icon, gamma);
                if let (Some(joined), Some(crew)) = (player.joined_color_dw, resources.crew) {
                    draw_joined_savegame_crew_overlay(
                        surface,
                        crew,
                        player_layout.icon,
                        joined,
                        gamma,
                    );
                }
            }

            let name = if self.show_winners {
                format!(
                    "{} ({})",
                    player.name,
                    if player.won { "won" } else { "lost" }
                )
            } else {
                player.name.clone()
            };
            draw_clonk_text(
                surface,
                &resources.fonts.text,
                player_layout.name_anchor.0,
                player_layout.name_anchor.1,
                &name,
                if self.show_winners && player.won {
                    Color::opaque(0xff, 0xdf, 0x00)
                } else {
                    Color::opaque(0xff, 0xff, 0xff)
                },
                TextAlign::Left,
                gamma,
            );

            // Ico_Rank1..Ico_Rank9 are GUIIcons.png phases 35..43
            // (src/C4PlayerInfoListBox.cpp:446-451).
            if let (Some(rank), Some(rect), Some(icons)) = (
                player.league_rank_symbol,
                player_layout.rank_icon,
                resources.gui_icons,
            ) {
                let phase = 35 + u32::from(rank.clamp(1, 9) - 1);
                let columns = 6;
                let cell = icons.width() as i32 / columns;
                if let Some(icon) = crop_image(
                    icons,
                    IntRect {
                        x: (phase as i32 % columns) * cell,
                        y: (phase as i32 / columns) * cell,
                        w: cell,
                        h: cell,
                    },
                ) {
                    draw_classic_image(surface, &icon, rect, gamma);
                }
            }
            if let Some((_icon, text)) = evaluation_score_label(player, "Score") {
                render_evaluation_score(
                    surface,
                    resources.fonts,
                    // `{{Ico:League}}` and `{{Ico:Settlement}}` are separate
                    // GUI icons in C++; only the settlement facet is in the
                    // validated evaluation resource set today, so both
                    // variants draw it.
                    resources.score,
                    player_layout.score_anchor,
                    text.as_str(),
                    gamma,
                );
            }
            if !split_player_lists {
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
            if let Some(anchor) = player_layout.custom_evaluation_anchor {
                draw_clonk_text(
                    surface,
                    &resources.fonts.text,
                    anchor.0,
                    anchor.1,
                    &player.custom_evaluation_strings,
                    Color::opaque(0xff, 0xff, 0xff),
                    TextAlign::Right,
                    gamma,
                );
            }
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct ClassicMultilineLabelLine {
    text: String,
    starts_paragraph: bool,
}

fn classic_multiline_label_lines(
    font: &clonk_graphics::clonk_font::ClonkFont,
    text: &str,
    width: i32,
) -> Vec<ClassicMultilineLabelLine> {
    const INDENT: &str = "    ";

    let indent_width = font.measure(INDENT, true).0;
    let mut lines = Vec::new();
    for paragraph in text.split(['\r', '\n', '|']) {
        if paragraph.is_empty() {
            continue;
        }
        let first = clonk_frontend::message_dialog::break_message_with_options(
            font,
            paragraph,
            width,
            clonk_frontend::message_dialog::BreakMessageOptions {
                max_lines: 1,
                ..clonk_frontend::message_dialog::BreakMessageOptions::default()
            },
        );
        let (first_line, remainder) = first
            .split_once('\n')
            .map_or((first.as_str(), None), |(line, rest)| (line, Some(rest)));
        lines.push(ClassicMultilineLabelLine {
            text: first_line.to_string(),
            starts_paragraph: true,
        });
        let Some(remainder) = remainder.filter(|remainder| !remainder.is_empty()) else {
            continue;
        };
        let continuation = clonk_frontend::message_dialog::break_message(
            font,
            remainder,
            width.saturating_sub(indent_width),
        );
        lines.extend(
            continuation
                .split('\n')
                .map(|line| ClassicMultilineLabelLine {
                    text: format!("{INDENT}{line}"),
                    starts_paragraph: false,
                }),
        );
    }
    lines
}

fn classic_multiline_label_height(
    lines: &[ClassicMultilineLabelLine],
    font: &clonk_graphics::clonk_font::ClonkFont,
) -> i32 {
    let mut height = 0_i32;
    for (index, line) in lines.iter().enumerate() {
        if index > 0 && line.starts_paragraph {
            height = height.saturating_add(font.line_height / 3);
        }
        height = height.saturating_add(font.line_height);
    }
    height.max(5)
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
    clonk_frontend::draw_image_bilinear(
        surface,
        &clonk_gui::Rect::new(
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
        let gray = ((i32::from(pixel[0]) + i32::from(pixel[1]) + i32::from(pixel[2])) / 3 + offset)
            .clamp(0, 255) as u8;
        pixel[..3].fill(gray);
    }
    ImageData::new(image.width(), image.height(), pixels)
}

/// `C4GUI::ScrollBar::Update` for a bar rectangle. The arithmetic lives in
/// `crate::scrollbar`; this keeps the rect-shaped call sites here unchanged.
fn scrollbar_pin_offset(rect: IntRect, scroll: i32, max_scroll: i32) -> i32 {
    scrollbar_pin_offset_for(rect.h, scroll, max_scroll)
}

/// `C4PlayerInfoListBox::PlayerListItem::UpdateIcon`'s joined-savegame overlay
/// (src/C4PlayerInfoListBox.cpp:302-321): a half-size `fctCrewClr` in the
/// joined player's lobby colour, drawn twice in the icon's lower-left - once
/// two pixels right under blit modulation 1, which is the shadow, and once
/// flush at the left edge.
fn draw_joined_savegame_crew_overlay(
    surface: &mut Surface,
    crew: &ImageData,
    icon: IntRect,
    joined_color_dw: u32,
    gamma: Option<&GammaRamp>,
) {
    let size_max = icon.w.max(icon.h);
    let crew_height = size_max / 2;
    let width = size_max / 2;
    let height = icon.h - crew_height;
    if width <= 0 || height <= 0 {
        return;
    }
    let y = icon.y + crew_height;
    let color = Color::opaque(
        ((joined_color_dw >> 16) & 0xff) as u8,
        ((joined_color_dw >> 8) & 0xff) as u8,
        (joined_color_dw & 0xff) as u8,
    );
    // The shadow pass runs under `ActivateBlitModulation(1)`, i.e. an almost
    // fully darkened blit, and is offset two pixels to the right; the visible
    // pass is flush at the icon's left edge in the joined player's colour.
    let shadow = clonk_frontend::hud::colorize_by_owner(crew, Color::opaque(0, 0, 1));
    let colored = clonk_frontend::hud::colorize_by_owner(crew, color);
    for (offset, image) in [(2, &shadow), (0, &colored)] {
        draw_classic_image(
            surface,
            image,
            IntRect {
                x: icon.x + offset,
                y,
                w: width,
                h: height,
            },
            gamma,
        );
    }
}

fn render_evaluation_score(
    surface: &mut Surface,
    fonts: &ClonkFontSet,
    score_icon: Option<&ImageData>,
    anchor: (i32, i32),
    text: &str,
    gamma: Option<&GammaRamp>,
) {
    // C4PlayerInfoListBox::UpdateScoreLabel emits the score icon followed by
    // gray old/gain and white new score. CStdFont scales the inline image to
    // iGfxLineHgt while preserving aspect (C4PlayerInfoListBox.cpp:376-418;
    // StdFont.cpp:845-896).
    let text_width = fonts.text.measure(text, true).0;
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
        text,
        Color::opaque(0xff, 0xff, 0xff),
        TextAlign::Left,
        gamma,
    );
}

// Keep the classic font draw inputs explicit so parity call sites can be
// compared directly with C4Font::DrawText.
#[allow(clippy::too_many_arguments)]
fn draw_clonk_text(
    surface: &mut Surface,
    font: &clonk_graphics::clonk_font::ClonkFont,
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

// Each x coordinate is a distinct classic result-table column; bundling them
// would hide the column mapping at the only call site.
#[allow(clippy::too_many_arguments)]
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
    clonk_frontend::draw_text_with_gamma(
        font,
        surface,
        name_x as f32,
        header_y,
        "Player",
        HEADER_FONT_SIZE,
        HEADER_COLOR,
        gamma,
    );
    clonk_frontend::draw_text_with_gamma(
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
    clonk_frontend::draw_text_with_gamma(
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
    clonk_frontend::draw_text_with_gamma(
        font,
        surface,
        x,
        y,
        &text,
        ROW_FONT_SIZE,
        TEXT_COLOR,
        gamma,
    );
}

// This small raster helper deliberately exposes the complete text and target
// span at each call site, matching the other fallback text draw helpers.
#[allow(clippy::too_many_arguments)]
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
    clonk_frontend::draw_text_with_gamma(
        font,
        surface,
        x,
        baseline as f32,
        text,
        size,
        color,
        gamma,
    );
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

#[cfg(test)]
mod tests {
    use super::*;

    fn endeavour_fonts() -> clonk_frontend::ClonkFontSet {
        let path = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("../../planet/System.c4g/Endeavour.ttf");
        let bytes = std::fs::read(path).expect("read Endeavour.ttf");
        crate::clonk_fonts::build_font_set(&bytes).expect("build Endeavour GUI fonts")
    }

    fn solid_image(width: u32, height: u32, color: [u8; 4]) -> clonk_frontend::ImageData {
        clonk_frontend::ImageData::new(
            width,
            height,
            std::iter::repeat_n(color, (width * height) as usize)
                .flatten()
                .collect(),
        )
    }

    // C4PlayerInfoListBox::UpdateScoreLabel's evaluation branch has four exact
    // variants and a strict precedence: any league state wins over settlement
    // state, and a hidden settlement score suppresses the label entirely
    // (src/C4PlayerInfoListBox.cpp:376-418).
    // C4GameOverDlg replaces the plain MultilineLabel with a real
    // C4GUI::TextWindow once the custom evaluation text exceeds a third of the
    // dialog height, and that TextWindow's ScrollWindow reserves and operates a
    // C4GUI_ScrollBarWdt bar (src/C4GameOverDlg.cpp:188-213).
    // UpdateIcon overlays a half-size fctCrewClr in the joined savegame
    // player's lobby colour over the lower-left of the row icon, drawn twice:
    // a two-pixel-right shadow under blit modulation 1, then the coloured pass
    // flush at the left edge (src/C4PlayerInfoListBox.cpp:302-321).
    // A team-filtered evaluation list box emits exactly one TeamListItem
    // before its players, and only when the dialog is in the two-team layout
    // (src/C4PlayerInfoListBox.cpp:1536-1541; src/C4GameOverDlg.cpp:214-230).
    // A PlayerListItem only gets a rank icon in a league game, and during
    // evaluation that icon takes its own square right-hand column, against
    // which the score label's right edge is measured
    // (src/C4PlayerInfoListBox.cpp:94-95,199-206,352,439-456).
    #[test]
    fn l183_league_rank_icons_claim_their_own_column_and_move_the_score_edge() {
        let player = EvaluationPlayer {
            player_info_id: 1,
            team_id: None,
            name: "Player".into(),
            won: true,
            color_dw: 0,
            total_playing_time: 0,
            score_old: 0,
            score_new: Some(10),
            custom_evaluation_strings: String::new(),
            big_icon: None,
            league_score_old: Some(1200),
            league_score_gain: Some(30),
            league_score_new: Some(1230),
            joined_color_dw: None,
            league_rank_symbol: Some(4),
        };
        let fonts = endeavour_fonts();

        let layout_for = |league: bool| {
            let evaluation =
                EvaluationViewModel::new(Vec::new(), vec![player.clone()]).with_league(league);
            let mut state = GameOverState::new("Round over".into(), Vec::new(), false);
            state.set_evaluation(evaluation);
            state.classic_evaluation_layout(640, 480, &fonts)
        };

        let plain = layout_for(false);
        let plain_row = plain.players[0];
        assert!(
            plain_row.rank_icon.is_none(),
            "a non-league round has no rank icon at all"
        );

        let league = layout_for(true);
        let row = league.players[0];
        let rank = row.rank_icon.expect("a league round reserves the column");
        assert_eq!(rank.w, rank.h, "the column is square");
        assert_eq!(rank.h, row.row.h, "sized by the row's inner height");
        assert_eq!(
            rank.x + rank.w,
            row.row.x + row.row.w,
            "flush with the row's right edge"
        );
        assert_eq!(rank.y, row.row.y);
        assert_eq!(
            row.score_anchor.0,
            rank.x - CLASSIC_PLAYER_LABEL_SPACING,
            "the score label's right edge follows the rank icon"
        );
        assert_eq!(
            plain_row.score_anchor.0,
            plain_row.row.x + plain_row.row.w - CLASSIC_PLAYER_LABEL_SPACING,
            "without a rank icon it follows the row width"
        );
        assert_eq!(
            row.time_anchor.0, row.score_anchor.0,
            "the playing-time label shares that right edge"
        );
    }

    #[test]
    fn l183_two_team_evaluation_lists_lead_with_one_native_team_header() {
        let player = |info_id: i32, team_id: i32, won: bool| EvaluationPlayer {
            player_info_id: info_id,
            team_id: Some(team_id),
            name: format!("Player {info_id}"),
            won,
            color_dw: 0,
            total_playing_time: 0,
            score_old: 0,
            score_new: None,
            custom_evaluation_strings: String::new(),
            big_icon: None,
            league_score_old: None,
            league_score_gain: None,
            league_score_new: None,
            joined_color_dw: None,
            league_rank_symbol: None,
        };
        let team = |id: i32, won: bool| EvaluationTeam {
            id,
            name: format!("Team {id}"),
            color_dw: 0x0011_2233,
            icon: None,
            won,
        };
        let evaluation =
            EvaluationViewModel::new(Vec::new(), vec![player(1, 7, true), player(2, 8, false)])
                .with_dialog_context(String::new(), Some([7, 8]))
                .with_team_order([7, 8])
                .with_teams([team(7, true), team(8, false)]);

        assert_eq!(evaluation.team(7).map(|team| team.won), Some(true));
        assert_eq!(evaluation.team(8).map(|team| team.won), Some(false));
        assert!(evaluation.team(9).is_none());

        let fonts = endeavour_fonts();
        let mut state = GameOverState::new("Round over".into(), Vec::new(), false);
        state.set_evaluation(evaluation);
        let layout = state.classic_evaluation_layout(640, 480, &fonts);

        assert_eq!(layout.player_lists.len(), 2);
        assert_eq!(
            layout
                .team_headers
                .iter()
                .map(|header| (header.player_list_index, header.team_id))
                .collect::<Vec<_>>(),
            [(0, 7), (1, 8)],
            "one header per filtered list, in team order"
        );
        for header in &layout.team_headers {
            let list = layout.player_lists[header.player_list_index];
            assert_eq!(header.icon.w, header.icon.h, "C4SymbolSize square");
            assert!(header.icon.x >= list.x && header.icon.y >= list.y);
            assert!(
                header.name_anchor.0 > header.icon.x + header.icon.w,
                "the caption sits right of its icon"
            );
            // Every row of that list starts below its header.
            for row in layout
                .players
                .iter()
                .filter(|row| row.player_list_index == header.player_list_index)
            {
                assert!(row.row.y >= header.icon.y + header.icon.h);
            }
        }
    }

    #[test]
    fn l183_joined_savegame_crew_overlay_uses_the_native_half_size_geometry() {
        // The C++ arithmetic, isolated: iSizeMax = max(Wdt, Hgt),
        // iCrewClrHgt = iSizeMax / 2, then Hgt -= iCrewClrHgt, Y += iCrewClrHgt
        // and Wdt = iSizeMax / 2.
        let geometry = |icon: IntRect| {
            let size_max = icon.w.max(icon.h);
            let crew_height = size_max / 2;
            IntRect {
                x: icon.x,
                y: icon.y + crew_height,
                w: size_max / 2,
                h: icon.h - crew_height,
            }
        };

        let square = IntRect {
            x: 40,
            y: 60,
            w: 40,
            h: 40,
        };
        assert_eq!(
            geometry(square),
            IntRect {
                x: 40,
                y: 80,
                w: 20,
                h: 20
            },
            "a square icon yields a quarter-area lower-left quad"
        );

        // A wide icon takes its half-size from the *larger* axis, so the quad
        // can be taller than half the icon.
        let wide = IntRect {
            x: 0,
            y: 0,
            w: 64,
            h: 32,
        };
        assert_eq!(
            geometry(wide),
            IntRect {
                x: 0,
                y: 32,
                w: 32,
                h: 0
            }
        );

        let tall = IntRect {
            x: 0,
            y: 0,
            w: 32,
            h: 64,
        };
        assert_eq!(
            geometry(tall),
            IntRect {
                x: 0,
                y: 32,
                w: 32,
                h: 32
            }
        );

        // Only the shadow pass is offset, and only by two pixels.
        assert_eq!(square.x + 2 - square.x, 2);
    }

    #[test]
    fn l183_overflowing_custom_evaluation_text_reserves_and_travels_the_native_scrollbar() {
        let short = ClassicEvaluationTextLayout {
            area: IntRect {
                x: 0,
                y: 0,
                w: 200,
                h: 40,
            },
            viewport: IntRect {
                x: 0,
                y: 0,
                w: 200,
                h: 40,
            },
            content_height: 40,
            scrollable: false,
            scrollbar: None,
        };
        assert!(
            short.scrollbar.is_none(),
            "a fitting MultilineLabel has no bar at all"
        );

        let area = IntRect {
            x: 10,
            y: 20,
            w: 200,
            h: 96,
        };
        let tall = ClassicEvaluationTextLayout {
            area,
            viewport: IntRect {
                x: area.x + 10,
                y: area.y + 8,
                w: area.w - 10 - 5 - 16,
                h: area.h - 16,
            },
            content_height: 400,
            scrollable: true,
            scrollbar: Some(IntRect {
                x: area.x + area.w - 16,
                y: area.y,
                w: 16,
                h: area.h,
            }),
        };
        let bar = tall.scrollbar.expect("overflowing text reserves a bar");
        assert_eq!(bar.w, 16, "C4GUI_ScrollBarWdt");
        assert_eq!(bar.x + bar.w, tall.area.x + tall.area.w);
        assert_eq!((bar.y, bar.h), (tall.area.y, tall.area.h));
        assert_eq!(
            tall.viewport.x + tall.viewport.w,
            bar.x - 5,
            "the text viewport stops before the reserved bar column"
        );

        // The pin travels the shaft between the two arrow buttons.
        let maximum = tall.content_height - tall.viewport.h;
        assert_eq!(scrollbar_pin_offset(bar, 0, maximum), 0);
        let travel = bar.h - 3 * 16;
        assert_eq!(scrollbar_pin_offset(bar, maximum, maximum), travel);
        assert_eq!(scrollbar_pin_offset(bar, maximum / 2, maximum), travel / 2);
        assert_eq!(
            scrollbar_pin_offset(bar, 10, 0),
            0,
            "an unscrollable window pins to the top"
        );
        let squat = IntRect { h: 3 * 16, ..bar };
        assert_eq!(
            scrollbar_pin_offset(squat, maximum, maximum),
            0,
            "a bar with no shaft left cannot travel"
        );
    }

    #[test]
    fn l183_evaluation_score_label_matches_the_native_league_and_settlement_variants() {
        let base = EvaluationPlayer {
            player_info_id: 1,
            team_id: None,
            name: "Player".into(),
            won: false,
            color_dw: 0,
            total_playing_time: 0,
            score_old: 10,
            score_new: Some(35),
            custom_evaluation_strings: String::new(),
            big_icon: None,
            league_score_old: None,
            league_score_gain: None,
            league_score_new: None,
            joined_color_dw: None,
            league_rank_symbol: None,
        };

        // Settlement, new score known.
        assert_eq!(
            evaluation_score_label(&base, "Score"),
            Some((
                EvaluationScoreIcon::Settlement,
                "<c afafaf>10 (+25)</c> 35 Score".to_string()
            ))
        );

        // Settlement, only the old score known (disconnected player).
        let old_only = EvaluationPlayer {
            score_new: None,
            ..base.clone()
        };
        assert_eq!(
            evaluation_score_label(&old_only, "Score"),
            Some((
                EvaluationScoreIcon::Settlement,
                "<c afafaf>(10)</c> Score".to_string()
            ))
        );

        // A hidden settlement score reaches the projection as score_old < 0.
        let hidden = EvaluationPlayer {
            score_old: -1,
            score_new: None,
            ..base.clone()
        };
        assert_eq!(evaluation_score_label(&hidden, "Score"), None);

        // League, old + gain == new.
        let league = EvaluationPlayer {
            league_score_old: Some(1200),
            league_score_gain: Some(30),
            league_score_new: Some(1230),
            joined_color_dw: None,
            league_rank_symbol: None,
            ..base.clone()
        };
        assert_eq!(
            evaluation_score_label(&league, "Score"),
            Some((
                EvaluationScoreIcon::League,
                "<c afafaf>1200 (+30)</c> 1230 Score".to_string()
            )),
            "league state wins over the settlement score"
        );

        // League with an admin discrepancy, shown in red.
        let discrepancy = EvaluationPlayer {
            league_score_new: Some(1300),
            joined_color_dw: None,
            league_rank_symbol: None,
            ..league.clone()
        };
        assert_eq!(
            evaluation_score_label(&discrepancy, "Score"),
            Some((
                EvaluationScoreIcon::League,
                "<c afafaf>1200 (+30)</c><c ff0000>(+70)</c> 1300 Score".to_string()
            ))
        );
        let negative = EvaluationPlayer {
            league_score_gain: Some(-30),
            league_score_new: Some(1100),
            joined_color_dw: None,
            league_rank_symbol: None,
            ..league.clone()
        };
        assert_eq!(
            evaluation_score_label(&negative, "Score"),
            Some((
                EvaluationScoreIcon::League,
                "<c afafaf>1200 (-30)</c><c ff0000>(-70)</c> 1100 Score".to_string()
            ))
        );

        // League score carried in but no new score: old score only.
        let old_league = EvaluationPlayer {
            league_score_new: None,
            joined_color_dw: None,
            league_rank_symbol: None,
            league_score_gain: None,
            ..league.clone()
        };
        assert_eq!(
            evaluation_score_label(&old_league, "Score"),
            Some((
                EvaluationScoreIcon::League,
                "<c afafaf>(1200)</c> Score".to_string()
            ))
        );

        // A zero league score is falsy in C++, so it does not select the
        // league branch; the projection maps it to None.
        let zero_league = EvaluationPlayer {
            league_score_old: None,
            league_score_gain: None,
            league_score_new: None,
            joined_color_dw: None,
            league_rank_symbol: None,
            ..base.clone()
        };
        assert_eq!(
            evaluation_score_label(&zero_league, "Score").map(|(icon, _)| icon),
            Some(EvaluationScoreIcon::Settlement)
        );

        // The IDS_TEXT_SCORE word is a runtime resource.
        assert!(evaluation_score_label(&base, "Punkte")
            .expect("settlement label")
            .1
            .ends_with(" Punkte"));
    }

    #[test]
    fn derived_game_over_facets_reuse_retained_texture_identity() {
        let source = solid_image(4, 4, [120, 80, 40, 255]);
        let rect = IntRect {
            x: 1,
            y: 1,
            w: 2,
            h: 2,
        };
        let first_crop = crop_image(&source, rect).expect("valid crop");
        let second_crop = crop_image(&source, rect).expect("valid crop");
        assert_eq!(first_crop.gpu_texture_id(), second_crop.gpu_texture_id());

        let first_gray = grayscale_image(&source, 30);
        let second_gray = grayscale_image(&source, 30);
        assert_eq!(first_gray.gpu_texture_id(), second_gray.gpu_texture_id());
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
    fn network_result_matches_cpp_pending_streaming_and_done_latches() {
        let mut host = GameOverState::new("Evaluation".into(), Vec::new(), true);
        host.initialize_network_result(true, true, "", None, 0, false);
        assert_eq!(host.network_result_label(), Some(""));
        assert!(!host.is_net_done());
        assert!(!host.allows_escape_close());
        assert!(host.actions().contains(&GameOverAction::End));
        assert!(host.actions().contains(&GameOverAction::Continue));
        assert_eq!(host.hotkey_action('e'), Some(GameOverAction::End));

        assert!(host.update_network_result(
            true,
            "evaluated",
            Some(RoundResultsNetworkResult::LeagueOk),
            2_047,
            true,
        ));
        assert_eq!(
            host.network_result_label(),
            Some("evaluated|[!]Transmitting record to league server... (1 kb remaining)")
        );
        assert!(!host.is_net_done());
        assert!(!host.allows_escape_close());

        assert!(host.update_network_result(
            true,
            "evaluated",
            Some(RoundResultsNetworkResult::LeagueOk),
            0,
            false,
        ));
        assert_eq!(host.network_result_label(), Some("evaluated"));
        assert!(host.is_net_done());
        assert!(host.allows_escape_close());

        let mut client = GameOverState::new("Evaluation".into(), Vec::new(), false);
        client.initialize_network_result(true, false, "", None, 0, false);
        assert!(!client.is_net_done());
        assert!(client.allows_escape_close());
        assert!(client.actions().contains(&GameOverAction::End));

        let mut local = GameOverState::new("Evaluation".into(), Vec::new(), true);
        local.initialize_network_result(false, true, "ignored", None, 0, false);
        assert_eq!(local.network_result_label(), None);
        assert!(local.is_net_done());
        assert!(local.allows_escape_close());
    }

    #[test]
    fn network_result_label_reserves_and_renders_the_native_two_line_area() {
        let fonts = endeavour_fonts();
        let mut state = GameOverState::new("Evaluation".into(), Vec::new(), true);
        state.initialize_network_result(
            true,
            true,
            "evaluated",
            Some(RoundResultsNetworkResult::LeagueOk),
            1_023,
            true,
        );
        let layout = state.classic_evaluation_layout(1024, 600, &fonts);
        let result = layout.network_result.expect("network result label");
        assert_eq!(
            result,
            IntRect {
                x: 35,
                y: 34,
                w: 954,
                h: fonts.text.line_height * 2,
            }
        );
        assert_eq!(
            layout.player_lists[0].y,
            result.y + result.h + 2 * CLASSIC_INDENT_Y
        );

        let caption = solid_image(192, 23, [20, 30, 40, 255]);
        let button = solid_image(128, 32, [0, 120, 0, 255]);
        let button_down = solid_image(128, 32, [0, 0, 180, 255]);
        let skin = ClassicGuiSkin::new(&caption, &button, &button_down, None);
        let mut surface = Surface::new(1024, 600, clonk_graphics::PixelFormat::Rgba8888);
        surface.begin_clonk_text_capture();
        state.render(
            &mut surface,
            &clonk_graphics::BitmapFont::new(),
            Some(GameOverClassicResources::new(
                skin, &fonts, None, None, None, None, None, None,
            )),
        );
        let command = surface
            .take_clonk_text_capture()
            .into_iter()
            .find(|command| command.text.starts_with("evaluated|[!]Transmitting"))
            .expect("network result text draw");
        assert_eq!(
            command.text,
            "evaluated|[!]Transmitting record to league server... (0 kb remaining)"
        );
        assert_eq!((command.x, command.y), (result.x + result.w / 2, result.y));
        assert_eq!(command.align, TextAlign::Center);
        assert_eq!(command.color, [0xff, 0xff, 0x00, 0xff]);
        assert!(command.markup);
    }

    #[test]
    fn native_focus_order_includes_close_player_list_and_visible_buttons() {
        let next = Some(NextMissionButton {
            label: "Next mission".to_string(),
            description: "Continue the campaign".to_string(),
        });
        let mut narrow = GameOverState::with_next_mission(
            "Evaluation".to_string(),
            Vec::new(),
            1024,
            next.clone(),
            true,
        );
        assert_eq!(narrow.focused(), None);
        for expected in [
            GameOverFocus::Close,
            GameOverFocus::PlayerList(0),
            GameOverFocus::Button(0),
            GameOverFocus::Button(1),
            GameOverFocus::Button(2),
            GameOverFocus::Close,
        ] {
            assert_eq!(narrow.advance_focus(false), expected);
        }

        let mut wide = GameOverState::with_next_mission(
            "Evaluation".to_string(),
            Vec::new(),
            1280,
            next,
            true,
        );
        assert_eq!(wide.advance_focus(true), GameOverFocus::Button(3));
        assert_eq!(wide.focused_action(), Some(GameOverAction::NextMission));
        assert_eq!(wide.advance_focus(false), GameOverFocus::Close);
    }

    #[test]
    fn focused_controls_use_native_down_up_sounds_and_direct_mnemonics() {
        let mut state = GameOverState::new("Evaluation".to_string(), Vec::new(), true);
        state.advance_focus(false);
        state.advance_focus(false);
        state.advance_focus(false);
        assert_eq!(state.focused_action(), Some(GameOverAction::End));
        assert!(state.handle_activation_down(GameOverActivationKey::Confirm));
        assert!(state.handle_activation_down(GameOverActivationKey::Space));
        assert_eq!(state.take_sound_events(), [GameOverSound::ArrowHit]);
        assert_eq!(
            state.handle_activation_up(GameOverActivationKey::Space),
            Some(GameOverAction::End)
        );
        assert_eq!(state.take_sound_events(), [GameOverSound::Click]);

        assert!(state.handle_activation_down(GameOverActivationKey::Confirm));
        assert_eq!(state.take_sound_events(), [GameOverSound::ArrowHit]);
        state.advance_focus(false);
        assert_eq!(state.focused_action(), Some(GameOverAction::Continue));
        assert_eq!(
            state.handle_activation_up(GameOverActivationKey::Space),
            None
        );
        assert!(state.take_sound_events().is_empty());
        assert!(state.handle_activation_down(GameOverActivationKey::Space));
        assert_eq!(state.take_sound_events(), [GameOverSound::ArrowHit]);
        state.advance_focus(true);
        assert_eq!(
            state.handle_activation_up(GameOverActivationKey::Confirm),
            Some(GameOverAction::End)
        );
        assert_eq!(state.take_sound_events(), [GameOverSound::Click]);
        assert!(
            state.focus_is_down(GameOverFocus::Button(1)),
            "each native Button retains its own fDown latch"
        );
        state.cancel_interaction();
        assert!(!state.focus_is_down(GameOverFocus::Button(1)));
        assert_eq!(
            state.handle_activation_up(GameOverActivationKey::Space),
            None
        );
        assert!(state.take_sound_events().is_empty());

        state.set_button_content(
            GameOverAction::End,
            "Runde &beenden".to_string(),
            String::new(),
        );
        state.set_button_content(
            GameOverAction::Continue,
            "&Weiterspielen".to_string(),
            String::new(),
        );
        state.set_button_content(
            GameOverAction::Restart,
            "Neusta&rt".to_string(),
            String::new(),
        );
        assert_eq!(state.hotkey_action('b'), Some(GameOverAction::End));
        assert_eq!(state.hotkey_action('w'), Some(GameOverAction::Continue));
        assert_eq!(state.hotkey_action('r'), Some(GameOverAction::Restart));
        assert_eq!(state.hotkey_action('e'), None);
        assert!(state.take_sound_events().is_empty());
    }

    #[test]
    fn selection_disabled_player_list_can_take_pointer_focus_but_not_activate() {
        let fonts = endeavour_fonts();
        let mut state = GameOverState::new("Evaluation".to_string(), Vec::new(), true);
        state.configure_classic_fonts(Some(&fonts));
        let list = state
            .classic_player_list_rects(1024, 600)
            .into_iter()
            .next()
            .expect("classic player list");
        state.handle_pointer_move((list.x + 1) as f32, (list.y + 1) as f32, 1024, 600);
        state.handle_pointer_down(1024, 600);
        assert_eq!(state.focused(), Some(GameOverFocus::PlayerList(0)));
        assert!(!state.handle_activation_down(GameOverActivationKey::Confirm));
        assert!(!state.handle_activation_down(GameOverActivationKey::Space));
        assert!(state.take_sound_events().is_empty());
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
            true,
        );
        let mut surface = Surface::new(1024, 600, clonk_graphics::PixelFormat::Rgba8888);
        let gamma = crate::tutorial_seven_gamma();
        state.render_with_gamma(
            &mut surface,
            &clonk_graphics::BitmapFont::new(),
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
            &clonk_graphics::BitmapFont::new(),
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
            None,
            None,
        );

        assert_eq!(
            resources
                .gui_icons
                .map(|image| (image.width(), image.height())),
            Some((240, 360))
        );
        assert_eq!(
            CLASSIC_FULFILLED_STAR_SOURCE,
            IntRect {
                x: 0,
                y: 320,
                w: 40,
                h: 40
            }
        );
        assert_eq!(
            resources
                .player
                .map(|image| (image.width(), image.height())),
            Some((48, 48))
        );
        assert_eq!(
            resources.score.map(|image| (image.width(), image.height())),
            Some((60, 30))
        );
    }

    #[test]
    fn subtitle_prefers_local_outcome() {
        let entries = vec![
            entry(1, "Observer", GameOverOutcome::Observer, false),
            entry(2, "Player", GameOverOutcome::Victory, true),
            entry(3, "Opponent", GameOverOutcome::Defeat, false),
        ];
        let state = GameOverState::new("Goldmine".into(), entries, true);
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
            true,
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
    fn classic_client_layout_keeps_base_chrome_and_omits_privileged_actions() {
        let fonts = endeavour_fonts();
        let state = GameOverState::with_next_mission(
            "A Clonk".into(),
            Vec::new(),
            1024,
            Some(NextMissionButton {
                label: "Next tutorial".into(),
                description: "Continue learning".into(),
            }),
            false,
        );
        let layout = state.classic_layout(1024, 600, &fonts);

        assert_eq!(
            state.actions(),
            &[GameOverAction::End, GameOverAction::Continue]
        );
        assert_eq!(
            GameOverState::new("A Clonk".into(), Vec::new(), false).actions(),
            &[GameOverAction::End, GameOverAction::Continue],
            "an ineligible client must not gain Restart when Next Mission is empty"
        );
        assert_eq!(
            GameOverState::new("A Clonk".into(), Vec::new(), true).actions(),
            &[
                GameOverAction::End,
                GameOverAction::Continue,
                GameOverAction::Restart,
            ],
            "an eligible host or Film 2 retains Restart without Next Mission"
        );
        assert_eq!(
            layout.dialog,
            IntRect {
                x: 112,
                y: 75,
                w: 800,
                h: 450,
            }
        );
        assert_eq!(layout.buttons.len(), 2);
    }

    #[test]
    fn wide_game_over_keeps_restart_without_inventing_initial_focus() {
        let fonts = endeavour_fonts();
        let mut state = GameOverState::with_next_mission(
            "A Clonk".into(),
            Vec::new(),
            1280,
            Some(NextMissionButton {
                label: "Next tutorial".into(),
                description: "Continue learning".into(),
            }),
            true,
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
        let layout = state.classic_layout(1280, 720, &fonts);
        assert_eq!(
            layout.dialog,
            IntRect {
                x: 75,
                y: 75,
                w: 1130,
                h: 570,
            }
        );
        assert_eq!(layout.buttons.len(), 4);
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
    fn goal_hover_reports_exact_fulfilled_and_unfulfilled_tooltips() {
        let fonts = endeavour_fonts();
        let mut state = GameOverState::new("A Clonk".into(), Vec::new(), true);
        state.set_evaluation(EvaluationViewModel::new(
            vec![
                EvaluationGoal {
                    definition_id: "DONE".into(),
                    fulfilled: true,
                    tooltip: "Goal Build the bridge fulfilled: Reach the other side".into(),
                    picture: None,
                },
                EvaluationGoal {
                    definition_id: "OPEN".into(),
                    fulfilled: false,
                    tooltip: "Goal Find the gold not fulfilled: Recover the treasure".into(),
                    picture: None,
                },
            ],
            Vec::new(),
        ));
        state.configure_classic_fonts(Some(&fonts));
        let layout = state.classic_evaluation_layout(1024, 600, &fonts);

        for (index, expected) in [
            "Goal Build the bridge fulfilled: Reach the other side",
            "Goal Find the gold not fulfilled: Recover the treasure",
        ]
        .into_iter()
        .enumerate()
        {
            let picture = layout.goals[index].picture;
            state.handle_pointer_move(
                (picture.x + picture.w / 2) as f32,
                (picture.y + picture.h / 2) as f32,
                1024,
                600,
            );
            assert_eq!(state.hovered_description(), expected);
        }

        let first = layout.goals[0].picture;
        state.handle_pointer_move(
            (first.x + first.w / 2) as f32,
            (first.y - 1) as f32,
            1024,
            600,
        );
        assert_eq!(
            state.hovered_description(),
            "",
            "the 4px margin is not a GoalPicture"
        );
        state.handle_pointer_move(
            (first.x + first.w / 2) as f32,
            (first.y + first.h / 2) as f32,
            1024,
            600,
        );
        state.pointer_left();
        assert_eq!(state.hovered_description(), "");
    }

    #[test]
    fn subtitle_defaults_to_defeat_without_winners() {
        let entries = vec![
            entry(1, "Player", GameOverOutcome::Defeat, false),
            entry(2, "Opponent", GameOverOutcome::Defeat, false),
        ];
        let state = GameOverState::new("Goldmine".into(), entries, true);
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
        let state = GameOverState::new("Goldmine".into(), entries, true);
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
            true,
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
            team_id: None,
            name: name.to_string(),
            won: true,
            color_dw: 0x00f4_0000,
            total_playing_time: 165,
            score_old: 0,
            score_new: Some(100),
            custom_evaluation_strings: String::new(),
            big_icon: None,
            league_score_old: None,
            league_score_gain: None,
            league_score_new: None,
            joined_color_dw: None,
            league_rank_symbol: None,
        }
    }

    #[test]
    fn network_error_uses_no_winners_team_order_names_and_losing_style() {
        let fonts = endeavour_fonts();
        let mut players = vec![
            evaluation_player(20, "Blue winner"),
            evaluation_player(10, "Red loser"),
            evaluation_player(30, "Teamless winner"),
            evaluation_player(11, "Red winner"),
        ];
        players[0].team_id = Some(2);
        players[1].team_id = Some(1);
        players[1].won = false;
        players[2].team_id = None;
        players[3].team_id = Some(1);
        let evaluation = EvaluationViewModel::new(Vec::new(), players).with_team_order([1, 2]);
        let mut state = GameOverState::new("Evaluation".into(), Vec::new(), true);
        state.set_evaluation(evaluation);
        state.initialize_network_result(
            true,
            false,
            "network failed",
            Some(RoundResultsNetworkResult::NetworkError),
            0,
            false,
        );
        assert!(!state.shows_winners());
        let layout = state.classic_evaluation_layout(1024, 600, &fonts);
        assert_eq!(
            layout
                .players
                .iter()
                .map(|player| player.player_index)
                .collect::<Vec<_>>(),
            vec![1, 3, 0, 2]
        );

        let caption = solid_image(192, 23, [20, 30, 40, 255]);
        let button = solid_image(128, 32, [0, 120, 0, 255]);
        let button_down = solid_image(128, 32, [0, 0, 180, 255]);
        let skin = ClassicGuiSkin::new(&caption, &button, &button_down, None);
        let background = Color::opaque(11, 22, 33);
        let mut surface = Surface::new(1024, 600, clonk_graphics::PixelFormat::Rgba8888);
        surface.fill(background);
        surface.begin_clonk_text_capture();
        state.render(
            &mut surface,
            &clonk_graphics::BitmapFont::new(),
            Some(GameOverClassicResources::new(
                skin, &fonts, None, None, None, None, None, None,
            )),
        );
        let commands = surface.take_clonk_text_capture();
        for name in ["Blue winner", "Red loser", "Teamless winner", "Red winner"] {
            let command = commands
                .iter()
                .find(|command| command.text == name)
                .expect("raw no-winners player name");
            assert_eq!(command.color, [0xff, 0xff, 0xff, 0xff]);
        }
        assert!(!commands
            .iter()
            .any(|command| command.text.ends_with("(won)") || command.text.ends_with("(lost)")));

        let mut expected = Surface::new(1, 1, clonk_graphics::PixelFormat::Rgba8888);
        expected.fill(background);
        clonk_frontend::classic_gui::draw_engine_box(
            &mut expected,
            0,
            0,
            0,
            0,
            clonk_frontend::classic_gui::STANDARD_BACKGROUND_COLOR,
            None,
        );
        clonk_frontend::classic_gui::draw_engine_box(&mut expected, 0, 0, 0, 0, 0x7faf_afaf, None);
        for player in &layout.players {
            assert_eq!(
                surface.get_pixel(
                    (player.row.x + player.row.w - 40) as u32,
                    (player.row.y + player.row.h / 2) as u32,
                ),
                expected.get_pixel(0, 0),
            );
        }
    }

    fn evaluation_state(screen_width: u32) -> GameOverState {
        let evaluation = EvaluationViewModel::new(
            vec![EvaluationGoal {
                definition_id: "SCRG".to_string(),
                fulfilled: true,
                tooltip: "Goal Scenario goal fulfilled: Complete the scenario".to_string(),
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
            true,
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
    fn fixed_two_team_evaluation_uses_distinct_lists_and_team_local_order() {
        // C4GameOverDlg creates exactly two grid cells for two predefined
        // teams. Each filtered C4PlayerInfoListBox retains player-info order
        // inside its own team instead of interleaving the columns by outcome
        // (C4GameOverDlg.cpp:214-229; C4PlayerInfoListBox.cpp:1529-1589).
        let fonts = endeavour_fonts();
        let mut players = vec![
            evaluation_player(20, "Blue first"),
            evaluation_player(10, "Red first"),
            evaluation_player(11, "Red second"),
            evaluation_player(21, "Blue second"),
        ];
        players[0].team_id = Some(2);
        players[0].won = false;
        players[1].team_id = Some(1);
        players[1].custom_evaluation_strings = "Red note".into();
        players[2].team_id = Some(1);
        players[3].team_id = Some(2);
        players[3].won = false;
        let evaluation = EvaluationViewModel::new(Vec::new(), players)
            .with_dialog_context(String::new(), Some([1, 2]));
        let mut state = GameOverState::new("Evaluation".into(), Vec::new(), true);
        state.set_evaluation(evaluation);
        state.configure_classic_fonts(Some(&fonts));

        let layout = state.classic_evaluation_layout(1024, 600, &fonts);
        assert_eq!(
            layout.player_lists,
            vec![
                IntRect {
                    x: 15,
                    y: 34,
                    w: 492,
                    h: 487,
                },
                IntRect {
                    x: 517,
                    y: 34,
                    w: 492,
                    h: 487,
                },
            ]
        );
        assert_eq!(
            layout
                .players
                .iter()
                .map(|player| (player.player_list_index, player.player_index))
                .collect::<Vec<_>>(),
            vec![(0, 1), (0, 2), (1, 0), (1, 3)]
        );
        assert_eq!(layout.players[0].row.h, 54 + fonts.text.line_height);
        assert_eq!(
            layout.players[1].row.y,
            layout.players[0].row.y + layout.players[0].row.h + 1,
            "the custom row reserves one text line before its team peer"
        );

        for expected in [
            GameOverFocus::Close,
            GameOverFocus::PlayerList(0),
            GameOverFocus::PlayerList(1),
            GameOverFocus::Button(0),
        ] {
            assert_eq!(state.advance_focus(false), expected);
        }
        assert_eq!(
            state.advance_focus(true),
            GameOverFocus::PlayerList(1),
            "backward traversal returns through the second team list"
        );
        let first_custom_anchor = layout.players[0]
            .custom_evaluation_anchor
            .expect("first red player custom line");
        assert!(layout.players[0].score_anchor.1 + fonts.text.line_height <= first_custom_anchor.1);
        let second = surface_rect(layout.player_lists[1]);
        state.handle_pointer_move((second.x + 1) as f32, (second.y + 1) as f32, 1024, 600);
        state.handle_pointer_down(1024, 600);
        assert_eq!(state.focused(), Some(GameOverFocus::PlayerList(1)));

        let caption = solid_image(192, 23, [20, 30, 40, 255]);
        let button = solid_image(128, 32, [0, 120, 0, 255]);
        let button_down = solid_image(128, 32, [0, 0, 180, 255]);
        let skin = ClassicGuiSkin::new(&caption, &button, &button_down, None);
        let mut surface = Surface::new(1024, 600, clonk_graphics::PixelFormat::Rgba8888);
        surface.begin_clonk_text_capture();
        state.render(
            &mut surface,
            &clonk_graphics::BitmapFont::new(),
            Some(GameOverClassicResources::new(
                skin, &fonts, None, None, None, None, None, None,
            )),
        );
        let commands = surface.take_clonk_text_capture();
        for player_layout in &layout.players {
            let player = state
                .evaluation()
                .player(player_layout.player_index)
                .expect("rendered split player");
            let label = format!(
                "{} ({})",
                player.name,
                if player.won { "won" } else { "lost" }
            );
            let command = commands
                .iter()
                .find(|command| command.text == label)
                .expect("split player name draw");
            assert_eq!((command.x, command.y), player_layout.name_anchor);
            assert!(player_layout.row.x >= layout.player_lists[player_layout.player_list_index].x);
            assert!(
                player_layout.row.x + player_layout.row.w
                    <= layout.player_lists[player_layout.player_list_index].x
                        + layout.player_lists[player_layout.player_list_index].w
            );
        }
    }

    #[test]
    fn custom_evaluation_long_paragraph_uses_cpp_continuation_indent() {
        let fonts = endeavour_fonts();
        let lines = classic_multiline_label_lines(
            &fonts.text,
            "Alpha beta gamma delta epsilon zeta eta theta iota kappa lambda mu nu xi omicron",
            120,
        );

        assert!(lines.len() > 2, "fixture must wrap automatically");
        assert!(lines[0].starts_paragraph);
        assert!(lines[1..]
            .iter()
            .all(|line| !line.starts_paragraph && line.text.starts_with("    ")));
        assert_eq!(
            classic_multiline_label_height(&lines, &fonts.text),
            i32::try_from(lines.len()).expect("line count") * fonts.text.line_height
        );
    }

    #[test]
    fn custom_evaluation_text_and_player_note_reserve_and_render_regions() {
        let fonts = endeavour_fonts();
        let mut player = evaluation_player(41, "Player");
        player.custom_evaluation_strings = "Personal note".into();
        let evaluation = EvaluationViewModel::new(Vec::new(), vec![player])
            .with_dialog_context("Global one|Global two".into(), None);
        let mut state = GameOverState::new("Evaluation".into(), Vec::new(), true);
        state.set_evaluation(evaluation);
        let layout = state.classic_evaluation_layout(1024, 600, &fonts);
        let custom = layout.custom_evaluation.expect("global custom text layout");
        assert!(!custom.scrollable);
        assert_eq!(
            custom.area.h,
            fonts.text.line_height * 2 + fonts.text.line_height / 3
        );
        assert_eq!(layout.players[0].row.h, 54 + fonts.text.line_height);
        assert_eq!(
            layout.players[0].time_anchor.1,
            layout.players[0].row.y + fonts.text.line_height,
            "the extra line shifts the unified-list time row four pixels upward"
        );
        assert_eq!(
            layout.player_lists[0].y,
            custom.area.y + custom.area.h + 2 * CLASSIC_INDENT_Y,
            "the custom block consumes caMain's vertical margins before the player list"
        );
        assert_eq!(
            layout.players[0].custom_evaluation_anchor,
            Some((
                layout.players[0].row.x + layout.players[0].row.w - 2,
                layout.players[0].row.y + layout.players[0].row.h - fonts.text.line_height - 2,
            ))
        );

        let caption = solid_image(192, 23, [20, 30, 40, 255]);
        let button = solid_image(128, 32, [0, 120, 0, 255]);
        let button_down = solid_image(128, 32, [0, 0, 180, 255]);
        let skin = ClassicGuiSkin::new(&caption, &button, &button_down, None);
        let mut surface = Surface::new(1024, 600, clonk_graphics::PixelFormat::Rgba8888);
        surface.begin_clonk_text_capture();
        state.render(
            &mut surface,
            &clonk_graphics::BitmapFont::new(),
            Some(GameOverClassicResources::new(
                skin, &fonts, None, None, None, None, None, None,
            )),
        );
        let commands = surface.take_clonk_text_capture();
        let global = commands
            .iter()
            .filter(|command| matches!(command.text.as_str(), "Global one" | "Global two"))
            .collect::<Vec<_>>();
        assert_eq!(global.len(), 2);
        assert!(global
            .iter()
            .all(|command| command.clip == Some(surface_rect(custom.viewport))));
        let personal = commands
            .iter()
            .find(|command| command.text == "Personal note")
            .expect("per-player custom evaluation draw");
        assert_eq!(personal.align, TextAlign::Right);
        assert_eq!(
            (personal.x, personal.y),
            layout.players[0]
                .custom_evaluation_anchor
                .expect("custom row anchor")
        );
    }

    #[test]
    fn oversized_custom_evaluation_is_clipped_to_one_third_and_wheel_scrolls() {
        let fonts = endeavour_fonts();
        let text = (0..40)
            .map(|index| format!("Line {index}"))
            .collect::<Vec<_>>()
            .join("|");
        let evaluation =
            EvaluationViewModel::new(Vec::new(), Vec::new()).with_dialog_context(text, None);
        let mut state = GameOverState::new("Evaluation".into(), Vec::new(), true);
        state.set_evaluation(evaluation);
        state.configure_classic_fonts(Some(&fonts));
        let layout = state.classic_evaluation_layout(1024, 600, &fonts);
        let custom = layout.custom_evaluation.expect("overflow layout");
        assert!(custom.scrollable);
        assert_eq!(custom.area.h, 487 / 3);
        assert!(custom.content_height > custom.viewport.h);

        state.handle_pointer_move(
            (custom.viewport.x + 1) as f32,
            (custom.viewport.y + 1) as f32,
            1024,
            600,
        );
        assert!(state.handle_wheel(-60, 1024, 600));
        assert_eq!(state.custom_evaluation_scroll, 60);

        let caption = solid_image(192, 23, [20, 30, 40, 255]);
        let button = solid_image(128, 32, [0, 120, 0, 255]);
        let button_down = solid_image(128, 32, [0, 0, 180, 255]);
        let skin = ClassicGuiSkin::new(&caption, &button, &button_down, None);
        let mut surface = Surface::new(1024, 600, clonk_graphics::PixelFormat::Rgba8888);
        surface.begin_clonk_text_capture();
        state.render(
            &mut surface,
            &clonk_graphics::BitmapFont::new(),
            Some(GameOverClassicResources::new(
                skin, &fonts, None, None, None, None, None, None,
            )),
        );
        let first = surface
            .take_clonk_text_capture()
            .into_iter()
            .find(|command| command.text == "Line 0")
            .expect("first overflow line draw");
        assert_eq!(first.y, custom.viewport.y - 60);
        assert_eq!(first.clip, Some(surface_rect(custom.viewport)));
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
        assert_eq!(
            chrome.buttons.iter().map(|rect| rect.x).collect::<Vec<_>>(),
            vec![65, 399, 733]
        );
        assert_eq!(
            content.goal_display,
            Some(IntRect {
                x: 5,
                y: 34,
                w: 1014,
                h: 72
            })
        );
        assert_eq!(
            content.goals[0].picture,
            IntRect {
                x: 480,
                y: 38,
                w: 64,
                h: 64
            }
        );
        assert_eq!(
            content.goals[0].fulfilled_star,
            IntRect {
                x: 512,
                y: 70,
                w: 32,
                h: 32
            }
        );
        assert_eq!(
            content.player_lists[0],
            IntRect {
                x: 15,
                y: 118,
                w: 994,
                h: 403
            }
        );
        assert_eq!(
            content.players[0].row,
            IntRect {
                x: 21,
                y: 121,
                w: 969,
                h: 54
            }
        );
        assert_eq!(
            content.players[0].icon,
            IntRect {
                x: 21,
                y: 121,
                w: 54,
                h: 54
            }
        );
        assert_eq!(content.players[0].name_anchor, (77, 123));
        assert_eq!(content.players[0].score_anchor, (988, 123));
        assert_eq!(content.players[0].time_anchor, (988, 147));

        let state = evaluation_state(1280);
        let chrome = state.classic_layout(1280, 720, &fonts);
        let content = state.classic_evaluation_layout(1280, 720, &fonts);
        assert_eq!(
            chrome.dialog,
            IntRect {
                x: 75,
                y: 75,
                w: 1130,
                h: 570
            }
        );
        assert_eq!(
            chrome.buttons.iter().map(|rect| rect.x).collect::<Vec<_>>(),
            vec![108, 388, 668, 948]
        );
        assert!(chrome.buttons.iter().all(|rect| rect.y == 583));
        assert_eq!(
            content.goals[0].picture,
            IntRect {
                x: 608,
                y: 108,
                w: 64,
                h: 64
            }
        );
        assert_eq!(
            content.player_lists[0],
            IntRect {
                x: 85,
                y: 188,
                w: 1110,
                h: 383
            }
        );
        assert_eq!(
            content.players[0].row,
            IntRect {
                x: 91,
                y: 191,
                w: 1085,
                h: 54
            }
        );

        let state = evaluation_state(1920);
        let chrome = state.classic_layout(1920, 1080, &fonts);
        let content = state.classic_evaluation_layout(1920, 1080, &fonts);
        assert_eq!(
            chrome.dialog,
            IntRect {
                x: 320,
                y: 180,
                w: 1280,
                h: 720
            }
        );
        assert_eq!(
            chrome.buttons.iter().map(|rect| rect.x).collect::<Vec<_>>(),
            vec![371, 688, 1005, 1322]
        );
        assert!(chrome.buttons.iter().all(|rect| rect.y == 838));
        assert_eq!(
            content.goals[0].picture,
            IntRect {
                x: 928,
                y: 213,
                w: 64,
                h: 64
            }
        );
        assert_eq!(
            content.player_lists[0],
            IntRect {
                x: 330,
                y: 293,
                w: 1260,
                h: 533
            }
        );
        assert_eq!(
            content.players[0].row,
            IntRect {
                x: 336,
                y: 296,
                w: 1235,
                h: 54
            }
        );
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
            true,
        );
        state.set_evaluation(EvaluationViewModel::new(
            Vec::new(),
            vec![evaluation_player(41, "Player")],
        ));

        let layout = state.classic_evaluation_layout(1024, 600, &fonts);
        assert_eq!(layout.goal_display, None);
        assert_eq!(
            layout.player_lists[0],
            IntRect {
                x: 15,
                y: 34,
                w: 994,
                h: 487
            }
        );
        assert_eq!(
            layout.players[0].row,
            IntRect {
                x: 21,
                y: 37,
                w: 969,
                h: 54
            }
        );

        let caption = solid_image(192, 23, [200, 0, 0, 255]);
        let button = solid_image(128, 32, [0, 120, 0, 255]);
        let button_down = solid_image(128, 32, [0, 0, 180, 255]);
        let skin = ClassicGuiSkin::new(&caption, &button, &button_down, None);
        let background = Color::opaque(11, 22, 33);
        let mut surface = Surface::new(1024, 600, clonk_graphics::PixelFormat::Rgba8888);
        surface.fill(background);
        state.render(
            &mut surface,
            &clonk_graphics::BitmapFont::new(),
            Some(GameOverClassicResources::new(
                skin, &fonts, None, None, None, None, None, None,
            )),
        );

        let mut expected = Surface::new(1, 1, clonk_graphics::PixelFormat::Rgba8888);
        expected.fill(background);
        clonk_frontend::classic_gui::draw_engine_box(
            &mut expected,
            0,
            0,
            0,
            0,
            clonk_frontend::classic_gui::STANDARD_BACKGROUND_COLOR,
            None,
        );
        clonk_frontend::classic_gui::draw_engine_box(&mut expected, 0, 0, 0, 0, 0x4faf_7a00, None);
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
        let skin = clonk_frontend::classic_gui::ClassicGuiSkin::new(
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
            true,
        );
        state.pressed_button = Some(1);
        state.down_controls.push(GameOverFocus::Button(1));
        state.hovered_button = Some(1);
        let layout = state.classic_layout(1024, 600, &fonts);
        let background = Color::opaque(11, 22, 33);
        let mut surface = Surface::new(1024, 600, clonk_graphics::PixelFormat::Rgba8888);
        surface.fill(background);
        let fallback = clonk_graphics::BitmapFont::new();

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
                None,
                None,
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
            true,
        );
        state.set_evaluation(EvaluationViewModel::new(
            vec![EvaluationGoal {
                definition_id: "SCRG".into(),
                fulfilled: true,
                tooltip: "Goal Scenario goal fulfilled: Complete the scenario".into(),
                picture: Some(goal_picture),
            }],
            vec![EvaluationPlayer {
                player_info_id: 41,
                team_id: None,
                name: "Player".into(),
                won: true,
                color_dw: 0x00e8_0000,
                total_playing_time: 3_661,
                score_old: 10,
                score_new: Some(110),
                custom_evaluation_strings: String::new(),
                big_icon: None,
                league_score_old: None,
                league_score_gain: None,
                league_score_new: None,
                joined_color_dw: None,
                league_rank_symbol: None,
            }],
        ));
        let layout = state.classic_evaluation_layout(1024, 600, &fonts);
        let background = Color::opaque(11, 22, 33);
        let mut surface = Surface::new(1024, 600, clonk_graphics::PixelFormat::Rgba8888);
        surface.fill(background);
        state.render(
            &mut surface,
            &clonk_graphics::BitmapFont::new(),
            Some(GameOverClassicResources::new(
                skin,
                &fonts,
                Some(&highlight),
                Some(&gui_icons),
                Some(&player),
                Some(&score),
                None,
                None,
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
        let mut expected = Surface::new(1, 1, clonk_graphics::PixelFormat::Rgba8888);
        expected.fill(background);
        clonk_frontend::classic_gui::draw_engine_box(
            &mut expected,
            0,
            0,
            0,
            0,
            clonk_frontend::classic_gui::STANDARD_BACKGROUND_COLOR,
            None,
        );
        clonk_frontend::classic_gui::draw_engine_box(&mut expected, 0, 0, 0, 0, 0x4faf_7a00, None);
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
                    surface.get_pixel(x as u32, y as u32) == Some(Color::opaque(0, 210, 240))
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
            true,
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
            state.focused(),
            None,
            "C4GUI::Button::IsFocusOnClick is false"
        );
        assert_eq!(
            state.handle_pointer_up(1024, 600),
            Some(GameOverAction::End)
        );

        state.handle_pointer_down(1024, 600);
        state.handle_pointer_move(0.0, 0.0, 1024, 600);
        assert_eq!(state.handle_pointer_up(1024, 600), None);
    }

    #[test]
    fn classic_button_down_latch_is_shared_by_pointer_and_activation_keys() {
        let fonts = endeavour_fonts();
        let mut state = GameOverState::new("Evaluation".to_string(), Vec::new(), true);
        state.configure_classic_fonts(Some(&fonts));
        let first = state.classic_layout(1024, 600, &fonts).buttons[0];
        state.handle_pointer_move(
            (first.x + first.w / 2) as f32,
            (first.y + first.h / 2) as f32,
            1024,
            600,
        );
        state.handle_pointer_down(1024, 600);
        state.handle_pointer_down(1024, 600);
        assert_eq!(state.take_sound_events(), [GameOverSound::ArrowHit]);
        assert_eq!(
            state.handle_pointer_up(1024, 600),
            Some(GameOverAction::End)
        );
        assert_eq!(state.take_sound_events(), [GameOverSound::Click]);

        for _ in 0..3 {
            state.advance_focus(false);
        }
        assert!(state.handle_activation_down(GameOverActivationKey::Confirm));
        assert_eq!(state.take_sound_events(), [GameOverSound::ArrowHit]);
        state.handle_pointer_down(1024, 600);
        assert!(state.take_sound_events().is_empty());
        assert_eq!(
            state.handle_pointer_up(1024, 600),
            Some(GameOverAction::End)
        );
        assert_eq!(state.take_sound_events(), [GameOverSound::Click]);
        assert_eq!(
            state.handle_activation_up(GameOverActivationKey::Space),
            None,
            "pointer-up already raised the shared native latch"
        );
        assert!(state.take_sound_events().is_empty());
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
            None,
            None,
        );
        let mut state = GameOverState::new(
            "A Clonk".into(),
            vec![entry(1, "Player", GameOverOutcome::Victory, true)],
            true,
        );
        state.configure_classic_fonts(Some(&fonts));
        let layout = state.classic_layout(1024, 600, &fonts);
        let first = layout.buttons[0];
        let second = layout.buttons[1];
        let render = |state: &GameOverState, focus_active, mouse_active| {
            let mut surface = Surface::new(1024, 600, clonk_graphics::PixelFormat::Rgba8888);
            surface.fill(Color::opaque(3, 4, 5));
            state.render_with_gamma_active(
                &mut surface,
                &clonk_graphics::BitmapFont::new(),
                Some(resources),
                None,
                focus_active,
                mouse_active,
            );
            surface
        };

        assert_eq!(state.hovered_action(), None);
        let idle = render(&state, true, true);
        state.handle_pointer_move(
            (first.x + first.w / 2) as f32,
            (first.y + first.h / 2) as f32,
            1024,
            600,
        );
        assert_eq!(state.hovered_action(), Some(GameOverAction::End));
        let hovered = render(&state, true, true);
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
        assert_eq!(
            render(&state, false, true).pixels(),
            hovered.pixels(),
            "a context suppresses keyboard focus but retains mouse hover"
        );
        assert_eq!(
            render(&state, false, false).pixels(),
            idle.pixels(),
            "a higher child dialog suppresses both focus and mouse hover"
        );

        state.handle_pointer_down(1024, 600);
        let pressed = render(&state, true, true);
        assert_eq!(state.pressed_button, Some(0));
        assert_ne!(
            pressed.get_pixel(first_probe.0, first_probe.1),
            hovered.get_pixel(first_probe.0, first_probe.1),
            "pointer down selects GUIButtonDown while still over the button"
        );
        state.handle_pointer_move(0.0, 0.0, 1024, 600);
        let dragged_out = render(&state, true, true);
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
        let dragged_back = render(&state, true, true);
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
        let left = render(&state, true, true);
        assert_eq!(
            left.pixels(),
            idle.pixels(),
            "pointer leave restores idle pixels"
        );

        for _ in 0..3 {
            state.advance_focus(false);
        }
        assert_eq!(state.focused(), Some(GameOverFocus::Button(0)));
        let focused = render(&state, true, true);
        assert_ne!(
            focused.get_pixel(first_probe.0, first_probe.1),
            idle.get_pixel(first_probe.0, first_probe.1),
            "active keyboard focus draws the same additive highlight"
        );
        let inactive = render(&state, false, true);
        assert_eq!(
            inactive.pixels(),
            idle.pixels(),
            "a retained focus is not drawn below a child dialog or context"
        );
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
            true,
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
            None,
            None,
        );
        let background = Color::opaque(3, 4, 5);
        let render = |state: &GameOverState| {
            let mut surface = Surface::new(1024, 600, clonk_graphics::PixelFormat::Rgba8888);
            surface.fill(background);
            state.render(
                &mut surface,
                &clonk_graphics::BitmapFont::new(),
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
        assert_eq!(
            state.focused(),
            None,
            "caption IconButton does not steal focus"
        );
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
