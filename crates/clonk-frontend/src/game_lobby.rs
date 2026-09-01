//! Classic `C4GameLobby::MainDlg` frontend, including its roster, resources,
//! core Options, and scenario-description sheets.
//!
//! The fullscreen lobby is a transparent overlay: the C++ dialog deliberately
//! leaves the loader/game background in place.  This module therefore draws
//! only classic GUI furniture and refuses incomplete or substituted resources.
//! Sheets and dialogs outside this bounded slice are emitted as typed requests.

use std::collections::HashMap;
use std::time::{Duration, Instant};

use anyhow::{ensure, Result};
use clonk_graphics::clonk_font::TextAlign;
use clonk_graphics::{Color, GammaRamp, PixelFormat, Surface};
use clonk_gui::Rect as GuiRect;

use crate::classic_gui::{
    blacken_transparent_pixels, draw_3d_frame, draw_engine_box, draw_engine_line,
    draw_facet_stretch, with_surface_clip, ClassicButtonState, ClassicGuiSkin, IntRect,
    STANDARD_BACKGROUND_COLOR,
};
use crate::context_menu::draw_classic_tooltip;
use crate::draw_scaled_caret;
use crate::game_option_buttons::{
    game_option_buttons_layout, GameOptionButton, GameOptionButtonResources, GameOptionButtons,
    GameOptionContext,
};
use crate::message_dialog::break_message;
use crate::{expand_hotkey_markup, ClonkFontSet, GuiPoint, ImageData, KeyCode};

const BUTTON_HEIGHT: i32 = 32;
const ICON_EXTENT: i32 = 64;
const TAB_ICON_EXTENT: i32 = 16;
const SCROLLBAR_EXTENT: i32 = 16;
const TAB_SHEET_MARGIN: i32 = 4;
const LIST_BOX_MARGIN: i32 = 3;
const SCENARIO_TEXT_LEFT_MARGIN: i32 = 10;
const SCENARIO_TEXT_RIGHT_MARGIN: i32 = 5;
const SCENARIO_TEXT_VERTICAL_MARGIN: i32 = 8;
const CLIENT_ROW_SPACING: i32 = 8;
const DEFAULT_ROW_SPACING: i32 = 1;
const PLAYER_ROW_INDENT: i32 = 3;
const ICON_LABEL_SPACING: i32 = 2;
const READY_COOLDOWN: Duration = Duration::from_secs(2);
const SOUND_ICON_SHOW_TIME: Duration = Duration::from_secs(1);
const TOOLTIP_DELAY: Duration = Duration::from_millis(500);

const CLASSIC_ROSTER_ICON_EXTENT: u32 = 40;

const STANDARD_ICON_WIDTH: u32 = 240;
const STANDARD_ICON_HEIGHT: u32 = 360;
const EXTENDED_ICON_WIDTH: u32 = 256;
const EXTENDED_ICON_HEIGHT: u32 = 320;
const CAPTION_WIDTH: u32 = 192;
const CAPTION_HEIGHT: u32 = 23;
const BUTTON_TEXTURE_WIDTH: u32 = 128;
const BUTTON_TEXTURE_HEIGHT: u32 = 32;
const CHECKBOX_WIDTH: u32 = 128;
const CHECKBOX_HEIGHT: u32 = 32;
const SCROLL_WIDTH: u32 = 32;
const SCROLL_HEIGHT: u32 = 48;
const CONTEXT_WIDTH: u32 = 32;
const CONTEXT_HEIGHT: u32 = 16;

const COLOR_YELLOW: [u8; 4] = [255, 255, 0, 255];
const COLOR_WHITE: [u8; 4] = [255, 255, 255, 255];
const COLOR_GRAY: [u8; 4] = [175, 175, 175, 255];
const DARK_BACKGROUND: u32 = 0x7f00_0000;
const LIST_SELECTION: u32 = 0xafaf_0000;
const LIST_INACTIVE_SELECTION: u32 = 0xaf7f_7f7f;
const LIST_SEPARATOR: u32 = 0x7f77_2200;
const EDIT_SELECTION: u32 = 0x7f7f_7f00;

/// The two constructor variants of `C4GameLobby::MainDlg`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum LobbyRole {
    Host,
    Client,
}

impl LobbyRole {
    pub const fn game_option_context(self) -> GameOptionContext {
        match self {
            Self::Host => GameOptionContext::LobbyHost,
            Self::Client => GameOptionContext::LobbyClient,
        }
    }
}

/// A right-side sheet.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum LobbySheet {
    Players,
    Teams,
    Resources,
    Options,
    Scenario,
}

/// Plain-text presentation installed into the lobby's `ScenDesc` window.
/// Resource loading and RTF conversion remain app-owned.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum LobbyScenarioText {
    /// Loading text or the native hard-coded scenario-file error.
    Message(String),
    /// A successfully loaded `Desc{}.rtf` component.
    Description(String),
    /// Scenario-title fallback when no nonempty description exists.
    Title(String),
}

impl Default for LobbyScenarioText {
    fn default() -> Self {
        Self::Message(String::new())
    }
}

impl LobbyScenarioText {
    pub fn text(&self) -> &str {
        match self {
            Self::Message(text) | Self::Description(text) | Self::Title(text) => text,
        }
    }
}

/// Semantic identity of one row in the lobby Options sheet.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum LobbyOptionKind {
    ControlMode,
    ControlRate,
    RuntimeJoin,
    TeamDistribution,
    TeamColors,
    RandomTeamCount,
}

/// One selectable value supplied to the app-owned classic context menu.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct LobbyOptionChoice {
    pub id: i32,
    pub label: String,
    pub tooltip: String,
}

/// Localized strings needed to construct the lobby options list.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct LobbyOptionLabels {
    pub control_mode: String,
    pub control_mode_tooltip: String,
    pub control_mode_central: String,
    pub control_mode_decentral: String,
    pub control_mode_async: String,
    pub control_mode_none: String,
    pub control_rate: String,
    pub control_rate_tooltip: String,
    pub runtime_join: String,
    pub runtime_join_tooltip: String,
    pub runtime_join_barred: String,
    pub runtime_join_free: String,
    pub team_distribution: String,
    pub team_distribution_tooltip: String,
    pub team_distribution_free: String,
    pub team_distribution_host: String,
    pub team_distribution_none: String,
    pub team_distribution_random: String,
    pub team_distribution_random_invisible: String,
    pub team_colors: String,
    pub team_colors_tooltip: String,
    pub enabled: String,
    pub disabled: String,
    pub random_team_count: String,
    pub random_team_count_tooltip: String,
    pub automatic: String,
    pub automatic_tooltip: String,
    /// Named-brace or native `%s` template used by context-menu entries.
    pub select_template: String,
}

impl Default for LobbyOptionLabels {
    fn default() -> Self {
        Self {
            control_mode: "Control mode".into(),
            control_mode_tooltip:
                "Changes the way control data is exchanged between network clients.".into(),
            control_mode_central: "Central control".into(),
            control_mode_decentral: "Decentral control".into(),
            control_mode_async: "[!]Asynchroner Netzwerkmodus (experimentell!)".into(),
            control_mode_none: "No control mode".into(),
            control_rate: "Control rate".into(),
            control_rate_tooltip:
                "Specifies the time interval in frames, at which control data is being exchanged via network"
                    .into(),
            runtime_join: "Runtime join".into(),
            runtime_join_tooltip:
                "Specifies whether additional computers may connect to the game after start.".into(),
            runtime_join_barred: "Runtime join prohibited".into(),
            runtime_join_free: "Runtime join allowed".into(),
            team_distribution: "Team distribution".into(),
            team_distribution_tooltip: "Specifies how players are distributed among teams"
                .into(),
            team_distribution_free: "Free".into(),
            team_distribution_host: "by Host".into(),
            team_distribution_none: "none".into(),
            team_distribution_random: "random".into(),
            team_distribution_random_invisible: "surprise random!".into(),
            team_colors: "Team colors".into(),
            team_colors_tooltip: "Specifies whether all players of a team have the same color, or individual colors are assigned for each team-member."
                .into(),
            enabled: "enabled".into(),
            disabled: "disabled".into(),
            random_team_count: "Team count".into(),
            random_team_count_tooltip:
                "Specifies how many teams should be filled by the random team distribution."
                    .into(),
            automatic: "Automatic".into(),
            automatic_tooltip: "If teams are predefined all of them are filled.|If teams are automatically generated only two are filled."
                .into(),
            select_template: "Select {value}".into(),
        }
    }
}

impl LobbyOptionLabels {
    fn select_tooltip(&self, value: &str) -> String {
        if self.select_template.contains("{value}") {
            self.select_template.replace("{value}", value)
        } else {
            self.select_template.replacen("%s", value, 1)
        }
    }
}

/// Renderable state for one `C4GameOptionsList::OptionDropdown`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct LobbyOptionRow {
    pub kind: LobbyOptionKind,
    pub caption: String,
    pub value: String,
    pub tooltip: String,
    pub editable: bool,
    pub choices: Vec<LobbyOptionChoice>,
}

/// Live C4TeamList inputs needed by the non-runtime lobby options.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct LobbyTeamOptionState {
    pub active: bool,
    pub auto_generate_teams: bool,
    pub distribution: i32,
    pub team_colors: bool,
    pub random_team_count: i32,
    pub active_player_count: i32,
    pub team_count: i32,
}

/// Construct the core non-runtime options in native add order.
///
/// Control mode is always read-only in the lobby. Control rate is editable
/// only for the control host, and Runtime join exists only for the host.
pub fn core_lobby_option_rows(
    role: LobbyRole,
    labels: &LobbyOptionLabels,
    control_mode: i32,
    control_rate: i32,
    runtime_join_allowed: bool,
) -> Vec<LobbyOptionRow> {
    let control_mode = match control_mode {
        0 => labels.control_mode_decentral.clone(),
        1 => labels.control_mode_central.clone(),
        2 => labels.control_mode_async.clone(),
        _ => labels.control_mode_none.clone(),
    };
    let rate_choices = (1..10)
        .map(|rate| {
            let label = rate.to_string();
            LobbyOptionChoice {
                id: rate,
                tooltip: labels.select_tooltip(&label),
                label,
            }
        })
        .collect();
    let mut rows = vec![
        LobbyOptionRow {
            kind: LobbyOptionKind::ControlMode,
            caption: labels.control_mode.clone(),
            value: control_mode,
            tooltip: labels.control_mode_tooltip.clone(),
            editable: false,
            choices: Vec::new(),
        },
        LobbyOptionRow {
            kind: LobbyOptionKind::ControlRate,
            caption: labels.control_rate.clone(),
            value: control_rate.to_string(),
            tooltip: labels.control_rate_tooltip.clone(),
            editable: role == LobbyRole::Host,
            choices: rate_choices,
        },
    ];
    if role == LobbyRole::Host {
        let choices = [
            (0, labels.runtime_join_barred.clone()),
            (1, labels.runtime_join_free.clone()),
        ]
        .into_iter()
        .map(|(id, label)| LobbyOptionChoice {
            id,
            tooltip: labels.select_tooltip(&label),
            label,
        })
        .collect();
        rows.push(LobbyOptionRow {
            kind: LobbyOptionKind::RuntimeJoin,
            caption: labels.runtime_join.clone(),
            value: if runtime_join_allowed {
                labels.runtime_join_free.clone()
            } else {
                labels.runtime_join_barred.clone()
            },
            tooltip: labels.runtime_join_tooltip.clone(),
            editable: true,
            choices,
        });
    }
    rows
}

/// Construct the core runtime `C4GameOptionsList` rows in native add order.
///
/// Unlike the lobby variant, a control host may change the control mode while
/// the game is running. Runtime join is owned by the network host and is not
/// present at all for clients.
pub fn core_runtime_option_rows(
    control_host: bool,
    network_host: bool,
    league: bool,
    labels: &LobbyOptionLabels,
    control_mode: i32,
    control_rate: i32,
    runtime_join_allowed: bool,
) -> Vec<LobbyOptionRow> {
    let control_mode_value = match control_mode {
        0 => labels.control_mode_decentral.clone(),
        1 => labels.control_mode_central.clone(),
        2 => labels.control_mode_async.clone(),
        _ => labels.control_mode_none.clone(),
    };
    let mut control_mode_choices = vec![
        LobbyOptionChoice {
            id: 1,
            label: labels.control_mode_central.clone(),
            tooltip: labels.select_tooltip(&labels.control_mode_central),
        },
        LobbyOptionChoice {
            id: 0,
            label: labels.control_mode_decentral.clone(),
            tooltip: labels.select_tooltip(&labels.control_mode_decentral),
        },
    ];
    if !league {
        control_mode_choices.push(LobbyOptionChoice {
            id: 2,
            label: labels.control_mode_async.clone(),
            tooltip: labels.select_tooltip(&labels.control_mode_async),
        });
    }
    let rate_choices = (1..10)
        .map(|rate| {
            let label = rate.to_string();
            LobbyOptionChoice {
                id: rate,
                tooltip: labels.select_tooltip(&label),
                label,
            }
        })
        .collect();
    let mut rows = vec![
        LobbyOptionRow {
            kind: LobbyOptionKind::ControlMode,
            caption: labels.control_mode.clone(),
            value: control_mode_value,
            tooltip: labels.control_mode_tooltip.clone(),
            editable: control_host,
            choices: control_mode_choices,
        },
        LobbyOptionRow {
            kind: LobbyOptionKind::ControlRate,
            caption: labels.control_rate.clone(),
            value: control_rate.to_string(),
            tooltip: labels.control_rate_tooltip.clone(),
            editable: control_host,
            choices: rate_choices,
        },
    ];
    if network_host {
        let choices = [
            (0, labels.runtime_join_barred.clone()),
            (1, labels.runtime_join_free.clone()),
        ]
        .into_iter()
        .map(|(id, label)| LobbyOptionChoice {
            id,
            tooltip: labels.select_tooltip(&label),
            label,
        })
        .collect();
        rows.push(LobbyOptionRow {
            kind: LobbyOptionKind::RuntimeJoin,
            caption: labels.runtime_join.clone(),
            value: if runtime_join_allowed {
                labels.runtime_join_free.clone()
            } else {
                labels.runtime_join_barred.clone()
            },
            tooltip: labels.runtime_join_tooltip.clone(),
            editable: true,
            choices,
        });
    }
    rows
}

/// Construct the scenario-gated team rows in native add order.
pub fn team_lobby_option_rows(
    role: LobbyRole,
    labels: &LobbyOptionLabels,
    state: LobbyTeamOptionState,
) -> Vec<LobbyOptionRow> {
    if !state.active {
        return Vec::new();
    }

    let distribution_labels = [
        labels.team_distribution_free.clone(),
        labels.team_distribution_host.clone(),
        labels.team_distribution_none.clone(),
        labels.team_distribution_random.clone(),
        labels.team_distribution_random_invisible.clone(),
    ];
    let distribution_value = usize::try_from(state.distribution)
        .ok()
        .and_then(|index| distribution_labels.get(index))
        .cloned()
        .unwrap_or_else(|| format!("TEAMDIST_undefined({})", state.distribution));
    let distribution_choices = [0, 1, 2, 3, 4]
        .into_iter()
        .filter(|id| *id != 2 || state.auto_generate_teams)
        .map(|id| {
            let label = distribution_labels[id as usize].clone();
            LobbyOptionChoice {
                id,
                tooltip: labels.select_tooltip(&label),
                label,
            }
        })
        .collect();
    let color_choices = [(1, labels.enabled.clone()), (0, labels.disabled.clone())]
        .into_iter()
        .map(|(id, label)| LobbyOptionChoice {
            id,
            tooltip: labels.select_tooltip(&label),
            label,
        })
        .collect();
    let mut rows = vec![
        LobbyOptionRow {
            kind: LobbyOptionKind::TeamDistribution,
            caption: labels.team_distribution.clone(),
            value: distribution_value,
            tooltip: labels.team_distribution_tooltip.clone(),
            editable: role == LobbyRole::Host,
            choices: distribution_choices,
        },
        LobbyOptionRow {
            kind: LobbyOptionKind::TeamColors,
            caption: labels.team_colors.clone(),
            value: if state.team_colors {
                labels.enabled.clone()
            } else {
                labels.disabled.clone()
            },
            tooltip: labels.team_colors_tooltip.clone(),
            editable: role == LobbyRole::Host,
            choices: color_choices,
        },
    ];

    if role == LobbyRole::Host && matches!(state.distribution, 3 | 4) {
        let maximum = if state.auto_generate_teams {
            state.active_player_count
        } else {
            state.team_count
        };
        let mut choices = vec![LobbyOptionChoice {
            id: 0,
            label: labels.automatic.clone(),
            tooltip: labels.automatic_tooltip.clone(),
        }];
        choices.extend((2..=maximum).map(|count| {
            let label = count.to_string();
            LobbyOptionChoice {
                id: count,
                tooltip: labels.select_tooltip(&label),
                label,
            }
        }));
        rows.push(LobbyOptionRow {
            kind: LobbyOptionKind::RandomTeamCount,
            caption: labels.random_team_count.clone(),
            value: if state.random_team_count > 1 {
                state.random_team_count.to_string()
            } else {
                labels.automatic.clone()
            },
            tooltip: labels.random_team_count_tooltip.clone(),
            editable: true,
            choices,
        });
    }

    rows
}

impl LobbySheet {
    pub const fn is_roster(self) -> bool {
        matches!(self, Self::Players | Self::Teams)
    }
}

/// Localized resource strings used by the visible lobby slice. Templates use
/// named braces so the frontend never bakes an English word order into layout.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct LobbyLabels {
    pub lobby: String,
    pub scenario_lobby_template: String,
    pub players_template: String,
    pub resources: String,
    pub options: String,
    pub scenario: String,
    pub chat: String,
    pub exit: String,
    pub start: String,
    pub cancel: String,
    pub ready: String,
    pub preload: String,
    pub still_loading: String,
    pub countdown_template: String,
    pub countdown_short_template: String,
    pub start_aborted: String,
    pub tooltip_chat: String,
    pub tooltip_exit: String,
    pub tooltip_start: String,
    pub tooltip_ready: String,
    pub tooltip_ready_unavailable: String,
    pub tooltip_preload: String,
    pub tooltip_ping: String,
    pub tooltip_unassigned_savegame_players: String,
    pub tooltip_script_players: String,
    pub tooltip_replay_players: String,
    pub tooltip_team_template: String,
}

impl Default for LobbyLabels {
    fn default() -> Self {
        Self {
            lobby: "Lobby".into(),
            scenario_lobby_template: "{scenario} - {lobby}".into(),
            players_template: "&Players ({active}/{maximum})".into(),
            resources: "&Resources".into(),
            options: "&Options".into(),
            scenario: "&Scenario".into(),
            chat: "Cha&t:".into(),
            exit: "E&xit".into(),
            start: "&Start".into(),
            cancel: "Cancel".into(),
            ready: "R&eady".into(),
            preload: "Preload".into(),
            still_loading: "Still loading".into(),
            countdown_template: "The game will start in {seconds} seconds.".into(),
            countdown_short_template: "{seconds}...".into(),
            start_aborted: "Game start aborted.".into(),
            tooltip_chat: "Enter chat messages here and send them with enter.".into(),
            tooltip_exit: "End the program.".into(),
            tooltip_start: "Starts the game.".into(),
            tooltip_ready: "Set yourself as ready to play.".into(),
            tooltip_ready_unavailable:
                "In order to set yourself as ready to play, all network resources have to be loaded completely."
                    .into(),
            tooltip_preload: "Preload game data".into(),
            tooltip_ping: "Ping".into(),
            tooltip_unassigned_savegame_players: "Unassociated savegame players.".into(),
            tooltip_script_players: "Players controlled by computer.".into(),
            tooltip_replay_players: "Starring".into(),
            tooltip_team_template: "Team {team}".into(),
        }
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct LobbyTooltip {
    pub pointer: GuiPoint,
    pub text: String,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum LobbyChatEditKey {
    Left,
    Right,
    Home,
    End,
    Backspace,
    Delete,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct LobbyChatKeyModifiers {
    pub shift: bool,
    pub control: bool,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum LobbyChatClipboardShortcut {
    Copy,
    Cut,
    Paste,
    SelectAll,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum LobbyChatContextCommand {
    Cut,
    Copy,
    Paste,
    Clear,
    SelectAll,
}

/// App-owned edit snapshot used to render the real C4GUI edit state without
/// inventing a generic text box. Byte offsets must be UTF-8 boundaries.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct LobbyChatEditView {
    pub text: String,
    pub caret: usize,
    pub selection: Option<(usize, usize)>,
    pub horizontal_scroll: i32,
    pub cursor_visible: bool,
}

impl LobbyChatEditView {
    fn normalized(mut self) -> Self {
        self.caret = valid_boundary_at_or_before(&self.text, self.caret.min(self.text.len()));
        self.selection = self.selection.and_then(|(anchor, caret)| {
            let anchor = valid_boundary_at_or_before(&self.text, anchor.min(self.text.len()));
            let caret = valid_boundary_at_or_before(&self.text, caret.min(self.text.len()));
            (anchor != caret).then_some((anchor, caret))
        });
        self.horizontal_scroll = self.horizontal_scroll.max(0);
        self
    }
}

/// Semantic identity retained when a recursive row menu is requested.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub enum LobbyRosterId {
    Client(i32),
    Player(i32),
    Header(LobbyRosterHeader),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum LobbyRosterHeader {
    Team(i32),
    RandomTeam,
    ScriptPlayers,
    ReplayPlayers,
    UnassignedSavegamePlayers,
}

/// Standard `GUIIcons.png` status phases used by client rows.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum LobbyClientStatus {
    Host,
    Client,
    Unknown,
    Observer,
    Ready,
    Sound,
    MutedSound,
}

impl LobbyClientStatus {
    const fn icon_phase(self) -> u16 {
        match self {
            Self::Host => 4,
            Self::Client => 5,
            Self::Unknown => 6,
            Self::Observer => 8,
            Self::Sound => 23,
            Self::Ready => 47,
            Self::MutedSound => 52,
        }
    }
}

/// An explicit row icon. No generic or guessed fallback is synthesized.
#[derive(Clone, Debug, PartialEq)]
pub enum LobbyRosterIcon {
    Standard(u16),
    Raster(ImageData),
}

/// Compose the colored 40x40 fallback surface created by
/// `C4PlayerInfoListBox::PlayerListItem::UpdateIcon` when a complete player
/// resource has no usable `BigIcon.png`.
///
/// Returning the composed surface, rather than the native `Player` raster,
/// also keeps a later savegame-join overlay in the classic 40-pixel source
/// coordinate system.
pub fn compose_classic_lobby_player_fallback_icon(
    player: &ImageData,
    owner_color: Color,
) -> Result<ImageData> {
    ensure!(
        player.width() > 0 && player.height() > 0,
        "active Player raster must not be empty"
    );
    let player = blacken_transparent_pixels(player);
    let colored = crate::hud::colorize_by_owner_software(&player, owner_color);
    let extent = i32::try_from(CLASSIC_ROSTER_ICON_EXTENT).expect("40 fits i32");
    let bounds = IntRect::new(0, 0, extent, extent);
    let fitted = aspect_fit_roster_raster(colored.width(), colored.height(), bounds);
    let source = Surface::from_bytes(
        colored.width(),
        colored.height(),
        PixelFormat::Rgba8888,
        colored.pixels().to_vec(),
    )?;
    let mut surface = Surface::new(
        CLASSIC_ROSTER_ICON_EXTENT,
        CLASSIC_ROSTER_ICON_EXTENT,
        PixelFormat::Rgba8888,
    );
    surface.fill(Color::new(255, 255, 255, 0));
    ensure!(
        clonk_graphics::compositing::copy_stretched(
            &source,
            clonk_graphics::Rect::new(0, 0, colored.width(), colored.height()),
            &mut surface,
            clonk_graphics::Rect::new(
                fitted.x,
                fitted.y,
                fitted.w.max(0) as u32,
                fitted.h.max(0) as u32,
            ),
        )
        .is_some(),
        "active Player raster must fit the fallback surface"
    );
    Ok(ImageData::new(
        CLASSIC_ROSTER_ICON_EXTENT,
        CLASSIC_ROSTER_ICON_EXTENT,
        surface.pixels().to_vec(),
    ))
}

/// Savegame player joined by this lobby player. The classic lobby composites
/// the owner-colored crew graphic into the lower-left half of the base icon.
#[derive(Clone, Debug, PartialEq)]
pub struct LobbyJoinedPlayerOverlay {
    /// Uncolored `Crew.png`/`fctCrewClr` raster with ClrByOwner pixels.
    pub crew: ImageData,
    /// Final joined-player lobby color as RGBA.
    pub color: [u8; 4],
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct LobbyTeamValue {
    pub id: i32,
    pub name: String,
    pub selectable: bool,
}

#[derive(Clone, Debug, PartialEq)]
pub struct LobbyClientRow {
    pub id: i32,
    pub name: String,
    pub nick: String,
    /// Final readable RGBA color, after the C++ chat-color adjustment.
    pub color: [u8; 4],
    pub status: LobbyClientStatus,
    pub local: bool,
    pub connected: bool,
    /// Remote resource progress. Local clients never display this prefix.
    pub resource_progress: Option<u8>,
    pub ping_ms: Option<i32>,
}

impl LobbyClientRow {
    fn display_name(&self) -> String {
        let name = crate::c4_presentation_text(&self.name);
        if !self.local && self.connected {
            if let Some(progress) = self.resource_progress {
                return format!("({}%) {}", progress.min(100), name);
            }
        }
        name
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct LobbyPlayerRow {
    pub id: i32,
    pub client_id: i32,
    pub name: String,
    /// Final lobby-name RGBA color.
    pub color: [u8; 4],
    pub icon: LobbyRosterIcon,
    pub joined_player_overlay: Option<LobbyJoinedPlayerOverlay>,
    pub team: Option<LobbyTeamValue>,
    pub league_score: Option<String>,
    /// One through nine, matching `Ico_Rank1..Ico_Rank9`.
    pub league_rank: Option<u8>,
}

#[derive(Clone, Debug, PartialEq)]
pub struct LobbyHeaderRow {
    pub kind: LobbyRosterHeader,
    pub label: String,
    pub icon: LobbyRosterIcon,
    pub can_add_player: bool,
}

/// Rows are supplied in the exact C++ display order (savegame/script headers,
/// then clients and each client's players).
#[derive(Clone, Debug, PartialEq)]
pub enum LobbyRosterRow {
    Client(LobbyClientRow),
    Player(LobbyPlayerRow),
    Header(LobbyHeaderRow),
}

impl LobbyRosterRow {
    pub fn id(&self) -> LobbyRosterId {
        match self {
            Self::Client(row) => LobbyRosterId::Client(row.id),
            Self::Player(row) => LobbyRosterId::Player(row.id),
            Self::Header(row) => LobbyRosterId::Header(row.kind),
        }
    }

    const fn indent(&self) -> i32 {
        if matches!(self, Self::Player(_)) {
            PLAYER_ROW_INDENT
        } else {
            0
        }
    }

    const fn top_spacing(&self) -> i32 {
        match self {
            Self::Client(_) | Self::Header(_) => CLIENT_ROW_SPACING,
            Self::Player(_) => DEFAULT_ROW_SPACING,
        }
    }

    const fn has_spacing_bar(&self) -> bool {
        matches!(
            self,
            Self::Client(_)
                | Self::Header(LobbyHeaderRow {
                    kind: LobbyRosterHeader::ReplayPlayers
                        | LobbyRosterHeader::Team(_)
                        | LobbyRosterHeader::RandomTeam,
                    ..
                })
        )
    }
}

/// One `C4Network2ResDlg::ListItem`. The source filename remains available to
/// app code; presentation uses only its final path component like
/// `GetFilename(C4Network2ResCore::getFileName())`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct LobbyResourceRow {
    pub id: i32,
    pub filename: String,
    pub present_percent: u8,
    /// Exact `C4Network2ResDlg::ListItem::IsSavePossible` projection. The app
    /// owns locality, transfer state, type and configuration; the frontend
    /// owns only the conditional `Ico_Save` button.
    pub save_possible: bool,
}

impl LobbyResourceRow {
    pub fn basename(&self) -> &str {
        self.filename
            .rsplit(['/', '\\'])
            .next()
            .unwrap_or(self.filename.as_str())
    }

    pub fn progress_label(&self) -> Option<String> {
        (self.present_percent < 100).then(|| format!("{}%", self.present_percent))
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct LobbyLogLine {
    pub text: String,
    pub color: [u8; 4],
}

/// Raise a GUI text color to the native lightness floor used for labels on
/// black backgrounds (`C4GUI::MakeColorReadableOnBlack`, C4Gui.cpp:71-89).
pub fn make_color_readable_on_black(color: u32) -> [u8; 4] {
    let red = (color >> 16) & 0xff;
    let green = (color >> 8) & 0xff;
    let blue = color & 0xff;
    let lightness = red * 50 + green * 87 + blue * 27;
    let increment = if lightness < 16_575 {
        (16_575 - lightness) / 164
    } else {
        0
    };
    [
        (red + increment).min(255) as u8,
        (green + increment).min(255) as u8,
        (blue + increment).min(255) as u8,
        0xff,
    ]
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum LobbyCountdownState {
    #[default]
    None,
    Long {
        seconds: i32,
    },
    Final {
        seconds: i32,
    },
    Start,
}

impl LobbyCountdownState {
    pub const fn is_any(self) -> bool {
        !matches!(self, Self::None)
    }

    /// Team selection and fair-crew changes lock only at ten seconds or less.
    pub const fn is_locked(self) -> bool {
        matches!(self, Self::Final { .. } | Self::Start)
    }

    const fn same_phase(self, other: Self) -> bool {
        matches!(
            (self, other),
            (Self::None, Self::None)
                | (Self::Long { .. }, Self::Long { .. })
                | (Self::Final { .. }, Self::Final { .. })
                | (Self::Start, Self::Start)
        )
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum LobbyCountdownPacket {
    Abort,
    Seconds(i32),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum LobbySound {
    ArrowHit,
    Click,
    Command,
    /// `MainDlg::OnCountdownPacket` calls StartSoundEffect directly rather
    /// than routing this notification through C4GUI::GUISound.
    CountdownCommand,
    Fuse,
    StartElevatorLoop,
    StopElevatorLoop,
    Pshshsh,
    Blast3,
}

#[derive(Clone, Debug, PartialEq)]
pub enum LobbyChatRequest {
    FocusInput,
    InsertText(String),
    RefocusAndInsert(String),
    EditKey {
        key: LobbyChatEditKey,
        modifiers: LobbyChatKeyModifiers,
    },
    Clipboard {
        shortcut: LobbyChatClipboardShortcut,
    },
    OpenContextMenu {
        anchor: GuiPoint,
    },
    ContextCommand(LobbyChatContextCommand),
    PointerDown(GuiPoint),
    PointerMove(GuiPoint),
    PointerUp(GuiPoint),
    PointerDoubleClick(GuiPoint),
    PointerMiddleDown(GuiPoint),
    TouchCancel,
    Submit(String),
    History {
        older: bool,
    },
    OpenExternalDialog,
}

#[derive(Clone, Debug, PartialEq)]
pub enum LobbyGameOptionInput {
    PointerMove(GuiPoint),
    PointerDown(GuiPoint),
    PointerUp(GuiPoint),
    MouseLeave,
    TouchCancel,
    Focus(GameOptionButton),
    ClearFocus,
    KeyDown { key: KeyCode, shift: bool },
    KeyUp(KeyCode),
    Hotkey(char),
    GamepadLowDown,
    GamepadLowUp,
    GamepadDirection { horizontal: i8, vertical: i8 },
}

/// Every non-local effect is explicit; no generic Rust pane is requested.
#[derive(Clone, Debug, PartialEq)]
pub enum LobbyAction {
    ExitRequested,
    /// App-owned `MainDlg::Start` contract. The app must run the league-rule
    /// check first, show the unassociated-savegame warning when requested,
    /// and then either start directly at zero or start the network countdown.
    StartRequested {
        countdown_seconds: i32,
        check_league_rules: bool,
        confirm_unassociated_savegame_players: bool,
    },
    /// App-owned `Game.Network.AbortLobbyCountdown()` request.
    AbortCountdownRequested,
    /// App-owned `C4Game::Preload()` request from the Resources-sheet button.
    PreloadRequested,
    ReadyChanged(bool),
    SheetRequested(LobbySheet),
    TabContextRequested {
        position: GuiPoint,
    },
    RosterContextRequested {
        row: LobbyRosterId,
        position: GuiPoint,
    },
    AddPlayerRequested {
        client_id: i32,
    },
    AddScriptPlayerRequested,
    TeamSelectionRequested {
        player_id: i32,
    },
    MoveLocalPlayersIntoTeamRequested {
        team_id: i32,
    },
    /// Open the app-owned context menu used as this ComboBox's dropdown.
    OptionSelectionRequested {
        option: LobbyOptionKind,
        anchor: GuiPoint,
        minimum_width: i32,
    },
    SaveResourceRequested {
        resource_id: i32,
    },
    Chat(LobbyChatRequest),
    GameOptions(LobbyGameOptionInput),
    FocusChanged(LobbyControl),
    RosterSelectionChanged(Option<LobbyRosterId>),
    CountdownChanged(LobbyCountdownState),
    NotifyUserIfInactive,
    AppendLog(LobbyLogLine),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum LobbyControl {
    ChatInput,
    TeamsTab,
    PlayersTab,
    ResourcesTab,
    OptionsTab,
    ScenarioTab,
    ChatDialog,
    Roster,
    RosterTeam,
    RosterAddPlayer,
    ResourceSave(i32),
    OptionsList,
    Option(LobbyOptionKind),
    Exit,
    GameOption(GameOptionButton),
    Run,
    Ready,
    Preload,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum LobbyTabIcon {
    Standard(u16),
    Extended(u16),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct LobbyTabButtonLayout {
    pub control: LobbyControl,
    pub sheet: Option<LobbySheet>,
    pub icon: LobbyTabIcon,
    pub rect: IntRect,
    pub selected: bool,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct LobbyLayout {
    pub client: IntRect,
    pub title_anchor: (i32, i32),
    pub chat_log: IntRect,
    pub chat_log_client: IntRect,
    pub chat_log_scrollbar: IntRect,
    pub chat_label: IntRect,
    pub chat_edit: IntRect,
    pub right_caption: IntRect,
    pub right_tab: IntRect,
    pub roster: IntRect,
    pub roster_client: IntRect,
    pub roster_scrollbar: IntRect,
    pub exit_button: IntRect,
    pub run_button: Option<IntRect>,
    pub ready_checkbox: IntRect,
    pub ready_square: IntRect,
    /// Visible manual preload button. The Resources list may still reserve
    /// its 32-pixel strip while this is `None` and preloading is ineligible.
    pub preload_button: Option<IntRect>,
    /// Bounds passed verbatim to `C4GameOptionButtons`.
    pub game_option_strip: IntRect,
    pub tab_buttons: Vec<LobbyTabButtonLayout>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct LobbyRosterRowLayout {
    pub index: usize,
    pub rect: IntRect,
    pub icon: IntRect,
    pub add_player: Option<IntRect>,
    pub team: Option<IntRect>,
    /// Conditional 16x16 `Ico_Save` button on a Resources-sheet row.
    pub save: Option<IntRect>,
    /// Stacked ComboBox bounds for an Options-sheet row.
    pub option_value: Option<IntRect>,
    pub rank: Option<IntRect>,
    pub collapsed: bool,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct LobbyRosterLayout {
    pub rows: Vec<LobbyRosterRowLayout>,
    pub content_height: i32,
    pub max_scroll: i32,
    pub collapsed: bool,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct LobbyChatScrollMetrics {
    pub content_height: i32,
    pub max_scroll: i32,
    pub scroll: i32,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct LobbyScenarioScrollMetrics {
    pub content_height: i32,
    pub max_scroll: i32,
    pub scroll: i32,
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct WrappedLobbyLogLine {
    text: String,
    color: [u8; 4],
    new_paragraph: bool,
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct WrappedLobbyScenarioLine {
    text: String,
    title_font: bool,
    new_paragraph: bool,
}

#[derive(Clone, Copy, Debug)]
struct Aligner {
    area: IntRect,
    margin_x: i32,
    margin_y: i32,
}

impl Aligner {
    const fn new(area: IntRect, margin_x: i32, margin_y: i32) -> Self {
        Self {
            area,
            margin_x,
            margin_y,
        }
    }

    fn get_from_top(&mut self, height: i32) -> IntRect {
        let rect = IntRect::new(
            self.area.x + self.margin_x,
            self.area.y + self.margin_y,
            self.area.w - 2 * self.margin_x,
            height,
        );
        let used = height + 2 * self.margin_y;
        self.area.y += used;
        self.area.h -= used;
        rect
    }

    fn get_from_bottom(&mut self, height: i32) -> IntRect {
        let rect = IntRect::new(
            self.area.x + self.margin_x,
            self.area.y + self.area.h - height - self.margin_y,
            self.area.w - 2 * self.margin_x,
            height,
        );
        self.area.h -= height + 2 * self.margin_y;
        rect
    }

    fn get_from_left(&mut self, width: i32) -> IntRect {
        let rect = IntRect::new(
            self.area.x + self.margin_x,
            self.area.y + self.margin_y,
            width,
            self.area.h - 2 * self.margin_y,
        );
        let used = width + 2 * self.margin_x;
        self.area.x += used;
        self.area.w -= used;
        rect
    }

    fn get_from_right(&mut self, width: i32) -> IntRect {
        let rect = IntRect::new(
            self.area.x + self.area.w - width - self.margin_x,
            self.area.y + self.margin_y,
            width,
            self.area.h - 2 * self.margin_y,
        );
        self.area.w -= width + 2 * self.margin_x;
        rect
    }

    const fn all(self) -> IntRect {
        IntRect::new(
            self.area.x + self.margin_x,
            self.area.y + self.margin_y,
            self.area.w - 2 * self.margin_x,
            self.area.h - 2 * self.margin_y,
        )
    }

    const fn inner_width(self) -> i32 {
        self.area.w - 2 * self.margin_x
    }

    const fn height(self) -> i32 {
        self.area.h
    }

    const fn centered(self, width: i32, height: i32) -> IntRect {
        IntRect::new(
            self.area.x + self.area.w / 2 - width / 2,
            self.area.y + self.area.h / 2 - height / 2,
            width,
            height,
        )
    }

    fn expand_top(&mut self, height: i32) {
        self.area.y -= height;
        self.area.h += height;
    }
}

fn offset(rect: IntRect, x: i32, y: i32) -> IntRect {
    rect.with_position(rect.x + x, rect.y + y)
}

/// Constructor-exact geometry for the initial Players/Clients lobby sheet.
pub fn game_lobby_layout(
    screen_width: i32,
    screen_height: i32,
    title_line_height: i32,
    text_line_height: i32,
    role: LobbyRole,
    has_teams: bool,
    has_external_chat: bool,
) -> LobbyLayout {
    let margin_x = if screen_width < 500 {
        2
    } else {
        screen_width / 50
    };
    let margin_y = if screen_height < 320 {
        2
    } else {
        screen_height * 2 / 75
    };
    let margin_top = 50 + margin_y;
    let client = IntRect::new(
        margin_x,
        margin_top,
        screen_width - 2 * margin_x,
        screen_height - margin_top - margin_y,
    );
    let absolute = |rect| offset(rect, client.x, client.y);

    let normal_width = client.w > 500;
    let normal_height = client.h > 320;
    let (indent_x1, indent_x2, indent_x3, client_list_width) = if normal_width {
        (10, 20, 5, client.w / 3)
    } else {
        (2, 2, 1, client.w / 2)
    };
    let (indent_y1, indent_y2, indent_y3, indent_y4) = if normal_height {
        (16, 20, 8, 8)
    } else {
        (2, 2, 1, 1)
    };

    let mut main = Aligner::new(IntRect::new(0, 0, client.w, client.h), 0, 0);
    let _status_offset = main.get_from_bottom(indent_y2);
    let bottom_rect = main.get_from_bottom(BUTTON_HEIGHT + indent_y1 * 2);
    let mut bottom = Aligner::new(bottom_rect, indent_x1, indent_y1);
    let exit_button = absolute(bottom.get_from_left(100));
    let run_button = if role == LobbyRole::Host {
        Some(absolute(bottom.get_from_right(100)))
    } else {
        None
    };
    let ready_checkbox = absolute(bottom.get_from_right(110));
    if role == LobbyRole::Client {
        let _centering_gap = bottom.get_from_left(10);
    }
    let game_option_strip =
        absolute(bottom.centered(bottom.inner_width(), ICON_EXTENT.min(bottom.height())));

    let right_rect = main.get_from_right(client_list_width);
    let mut right = Aligner::new(right_rect, indent_x3, indent_y4);
    let right_caption = absolute(right.get_from_top(text_line_height.max(CAPTION_HEIGHT as i32)));
    right.expand_top(indent_y4 * 2 + 1);
    let right_tab = absolute(right.all());
    let roster = IntRect::new(
        right_tab.x + TAB_SHEET_MARGIN,
        right_tab.y + TAB_SHEET_MARGIN,
        right_tab.w - 2 * TAB_SHEET_MARGIN,
        right_tab.h - 2 * TAB_SHEET_MARGIN,
    );
    let roster_client = IntRect::new(
        roster.x + LIST_BOX_MARGIN,
        roster.y + LIST_BOX_MARGIN,
        (roster.w - 2 * LIST_BOX_MARGIN - SCROLLBAR_EXTENT).max(0),
        (roster.h - 2 * LIST_BOX_MARGIN).max(0),
    );
    let roster_scrollbar = IntRect::new(
        roster_client.x + roster_client.w,
        roster_client.y,
        SCROLLBAR_EXTENT,
        roster_client.h,
    );

    let mut center = Aligner::new(main.all(), indent_x2, indent_y3);
    let edit_height = (text_line_height + 3).max(CAPTION_HEIGHT as i32);
    let chat_row = center.get_from_bottom(edit_height);
    let mut chat = Aligner::new(chat_row, 0, 0);
    let chat_label = absolute(chat.get_from_left(40));
    let chat_edit = absolute(chat.all());
    let chat_log = absolute(center.all());
    let chat_log_client = IntRect::new(
        chat_log.x + 10,
        chat_log.y + 8,
        chat_log.w - 10 - 5 - SCROLLBAR_EXTENT,
        chat_log.h - 16,
    );
    let chat_log_scrollbar = IntRect::new(
        chat_log.x + chat_log.w - 5 - SCROLLBAR_EXTENT,
        chat_log.y + 8,
        SCROLLBAR_EXTENT,
        chat_log.h - 16,
    );

    let count = 4 + usize::from(has_teams) + usize::from(has_external_chat);
    let mut next_index = count as i32;
    let mut tab_buttons = Vec::with_capacity(count);
    let mut add_tab = |control, sheet, icon, selected| {
        next_index -= 1;
        let rect = IntRect::new(
            right_caption.x + right_caption.w - (TAB_ICON_EXTENT + 4) * (next_index + 1),
            right_caption.y + 4,
            TAB_ICON_EXTENT,
            TAB_ICON_EXTENT,
        );
        tab_buttons.push(LobbyTabButtonLayout {
            control,
            sheet,
            icon,
            rect,
            selected,
        });
    };
    if has_teams {
        add_tab(
            LobbyControl::TeamsTab,
            Some(LobbySheet::Teams),
            LobbyTabIcon::Standard(19),
            false,
        );
    }
    add_tab(
        LobbyControl::PlayersTab,
        Some(LobbySheet::Players),
        LobbyTabIcon::Standard(9),
        true,
    );
    add_tab(
        LobbyControl::ResourcesTab,
        Some(LobbySheet::Resources),
        LobbyTabIcon::Standard(10),
        false,
    );
    add_tab(
        LobbyControl::OptionsTab,
        Some(LobbySheet::Options),
        LobbyTabIcon::Standard(14),
        false,
    );
    add_tab(
        LobbyControl::ScenarioTab,
        Some(LobbySheet::Scenario),
        LobbyTabIcon::Standard(22),
        false,
    );
    if has_external_chat {
        add_tab(
            LobbyControl::ChatDialog,
            None,
            LobbyTabIcon::Extended(15),
            false,
        );
    }

    LobbyLayout {
        client,
        title_anchor: (
            client.x + client.w / 2,
            client.y + 25 - title_line_height / 2 - margin_top,
        ),
        chat_log,
        chat_log_client,
        chat_log_scrollbar,
        chat_label,
        chat_edit,
        right_caption,
        right_tab,
        roster,
        roster_client,
        roster_scrollbar,
        exit_button,
        run_button,
        ready_checkbox,
        ready_square: ready_checkbox.with_width(ready_checkbox.h),
        preload_button: None,
        game_option_strip,
        tab_buttons,
    }
}

/// Exact classic resources. `new` fails instead of allowing generic widgets.
#[derive(Clone)]
pub struct LobbyResources<'a> {
    fonts: &'a ClonkFontSet,
    tooltip_font: &'a clonk_graphics::clonk_font::ClonkFont,
    caption: &'a ImageData,
    button: &'a ImageData,
    button_down: &'a ImageData,
    icons: ImageData,
    icons_extended: ImageData,
    button_highlight: ImageData,
    checkbox: &'a ImageData,
    scroll: &'a ImageData,
    context: &'a ImageData,
}

impl<'a> LobbyResources<'a> {
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        fonts: &'a ClonkFontSet,
        tooltip_font: &'a clonk_graphics::clonk_font::ClonkFont,
        caption: &'a ImageData,
        button: &'a ImageData,
        button_down: &'a ImageData,
        icons: &'a ImageData,
        icons_extended: &'a ImageData,
        button_highlight: &'a ImageData,
        checkbox: &'a ImageData,
        scroll: &'a ImageData,
        context: &'a ImageData,
    ) -> Result<Self> {
        let resources = Self {
            fonts,
            tooltip_font,
            caption,
            button,
            button_down,
            icons: blacken_transparent_pixels(icons),
            icons_extended: blacken_transparent_pixels(icons_extended),
            button_highlight: blacken_transparent_pixels(button_highlight),
            checkbox,
            scroll,
            context,
        };
        resources.validate()?;
        Ok(resources)
    }

    fn validate(&self) -> Result<()> {
        validate_exact(
            "GUICaption.png",
            self.caption,
            CAPTION_WIDTH,
            CAPTION_HEIGHT,
        )?;
        validate_exact(
            "GUIButton.png",
            self.button,
            BUTTON_TEXTURE_WIDTH,
            BUTTON_TEXTURE_HEIGHT,
        )?;
        validate_exact(
            "GUIButtonDown.png",
            self.button_down,
            BUTTON_TEXTURE_WIDTH,
            BUTTON_TEXTURE_HEIGHT,
        )?;
        validate_exact(
            "GUIIcons.png",
            &self.icons,
            STANDARD_ICON_WIDTH,
            STANDARD_ICON_HEIGHT,
        )?;
        validate_exact(
            "GUIIcons2.png",
            &self.icons_extended,
            EXTENDED_ICON_WIDTH,
            EXTENDED_ICON_HEIGHT,
        )?;
        ensure!(
            self.button_highlight.width() > 0 && self.button_highlight.height() > 0,
            "GUIButtonHighlight.png must be a non-empty full-size classic facet: got {}x{}",
            self.button_highlight.width(),
            self.button_highlight.height()
        );
        validate_exact(
            "GUICheckBox.png",
            self.checkbox,
            CHECKBOX_WIDTH,
            CHECKBOX_HEIGHT,
        )?;
        validate_exact("GUIScroll.png", self.scroll, SCROLL_WIDTH, SCROLL_HEIGHT)?;
        validate_exact(
            "GUIContext.png",
            self.context,
            CONTEXT_WIDTH,
            CONTEXT_HEIGHT,
        )?;
        ensure!(
            self.fonts.text.line_height > 0
                && self.fonts.title.line_height > 0
                && self.fonts.caption.line_height > 0
                && self.tooltip_font.line_height > 0,
            "classic lobby fonts must have positive line heights"
        );
        Ok(())
    }

    fn skin(&self) -> ClassicGuiSkin<'_> {
        ClassicGuiSkin::new(
            self.caption,
            self.button,
            self.button_down,
            Some(&self.button_highlight),
        )
    }
}

fn validate_exact(name: &str, image: &ImageData, width: u32, height: u32) -> Result<()> {
    ensure!(
        (image.width(), image.height()) == (width, height),
        "{name} must be the exact {width}x{height} classic resource: got {}x{}",
        image.width(),
        image.height()
    );
    Ok(())
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum HitTarget {
    None,
    ChatInput,
    ChatLabel,
    ChatScrollTop,
    ChatScrollBottom,
    ChatScrollTrack,
    ChatScrollInert,
    RightCaption,
    Tab(LobbyControl),
    RosterRow(usize),
    RosterBlank,
    RightListInert,
    ResourceSave(i32),
    AddPlayer(usize),
    Team(usize),
    OptionRow(usize),
    OptionValue(usize),
    RosterScrollTop,
    RosterScrollBottom,
    RosterScrollTrack,
    RosterScrollInert,
    Exit,
    GameOption(GameOptionButton),
    Run,
    Ready,
    Preload,
}

impl HitTarget {
    const fn button_control(self) -> Option<LobbyControl> {
        match self {
            Self::Tab(control) => Some(control),
            Self::Exit => Some(LobbyControl::Exit),
            Self::Run => Some(LobbyControl::Run),
            Self::Preload => Some(LobbyControl::Preload),
            Self::AddPlayer(_) => Some(LobbyControl::RosterAddPlayer),
            Self::ResourceSave(resource_id) => Some(LobbyControl::ResourceSave(resource_id)),
            _ => None,
        }
    }

    const fn is_scroll_arrow(self) -> bool {
        matches!(
            self,
            Self::ChatScrollTop
                | Self::ChatScrollBottom
                | Self::RosterScrollTop
                | Self::RosterScrollBottom
        )
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum ScrollbarDrag {
    Chat,
    Roster,
}

/// Pure state/controller for one lobby overlay.
#[derive(Clone, Debug)]
pub struct GameLobby {
    role: LobbyRole,
    labels: LobbyLabels,
    scenario_title: String,
    active_sheet: LobbySheet,
    active_players: i32,
    max_players: i32,
    has_teams: bool,
    has_external_chat: bool,
    resources_loaded: bool,
    /// Manual preload button lifetime is separate from visibility: native
    /// reserves its strip from construction until a successful click.
    preload_button_present: bool,
    preload_eligible: bool,
    ready: bool,
    configured_countdown_seconds: i32,
    league_mode: bool,
    countdown: LobbyCountdownState,
    rows: Vec<LobbyRosterRow>,
    resource_rows: Vec<LobbyResourceRow>,
    option_rows: Vec<LobbyOptionRow>,
    client_sound_status: HashMap<i32, (bool, Instant)>,
    logs: Vec<LobbyLogLine>,
    chat_edit: LobbyChatEditView,
    chat_scroll: i32,
    chat_max_scroll: i32,
    chat_scroll_pin: i32,
    chat_follow_bottom: bool,
    focus: LobbyControl,
    hovered: HitTarget,
    hover_since: Instant,
    pointer: Option<GuiPoint>,
    tooltip_pointer_active: bool,
    pointer_pressed: Option<HitTarget>,
    pointer_pressed_roster_id: Option<LobbyRosterId>,
    pointer_inside_pressed: bool,
    key_pressed: Option<(LobbyControl, KeyCode)>,
    selected_row: Option<usize>,
    selected_roster_id: Option<LobbyRosterId>,
    selected_option: Option<LobbyOptionKind>,
    open_team_combo_player: Option<i32>,
    open_option_combo: Option<LobbyOptionKind>,
    roster_scroll: i32,
    roster_max_scroll: i32,
    roster_scroll_pin: i32,
    resource_scroll: i32,
    resource_max_scroll: i32,
    resource_scroll_pin: i32,
    scenario_text: LobbyScenarioText,
    scenario_scroll: i32,
    scenario_max_scroll: i32,
    scenario_scroll_pin: i32,
    option_scroll: i32,
    option_max_scroll: i32,
    option_scroll_pin: i32,
    scrollbar_drag: Option<ScrollbarDrag>,
    collapsed_roster: bool,
    collapse_player_limit: usize,
    ready_last_change: Option<Instant>,
    sounds: Vec<LobbySound>,
}

impl GameLobby {
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        role: LobbyRole,
        scenario_title: impl Into<String>,
        active_players: i32,
        max_players: i32,
        has_teams: bool,
        has_external_chat: bool,
        resources_loaded: bool,
        ready: bool,
        configured_countdown_seconds: i32,
        rows: Vec<LobbyRosterRow>,
    ) -> Self {
        Self {
            role,
            labels: LobbyLabels::default(),
            scenario_title: scenario_title.into(),
            active_sheet: LobbySheet::Players,
            active_players,
            max_players,
            has_teams,
            has_external_chat,
            resources_loaded,
            preload_button_present: false,
            preload_eligible: false,
            ready,
            configured_countdown_seconds,
            league_mode: false,
            countdown: LobbyCountdownState::None,
            rows,
            resource_rows: Vec::new(),
            option_rows: Vec::new(),
            client_sound_status: HashMap::new(),
            logs: Vec::new(),
            chat_edit: LobbyChatEditView {
                cursor_visible: true,
                ..LobbyChatEditView::default()
            },
            chat_scroll: 0,
            chat_max_scroll: 0,
            chat_scroll_pin: 0,
            chat_follow_bottom: true,
            focus: LobbyControl::ChatInput,
            hovered: HitTarget::None,
            hover_since: Instant::now(),
            pointer: None,
            tooltip_pointer_active: false,
            pointer_pressed: None,
            pointer_pressed_roster_id: None,
            pointer_inside_pressed: false,
            key_pressed: None,
            selected_row: None,
            selected_roster_id: None,
            selected_option: None,
            open_team_combo_player: None,
            open_option_combo: None,
            roster_scroll: 0,
            roster_max_scroll: 0,
            roster_scroll_pin: 0,
            resource_scroll: 0,
            resource_max_scroll: 0,
            resource_scroll_pin: 0,
            scenario_text: LobbyScenarioText::default(),
            scenario_scroll: 0,
            scenario_max_scroll: 0,
            scenario_scroll_pin: 0,
            option_scroll: 0,
            option_max_scroll: 0,
            option_scroll_pin: 0,
            scrollbar_drag: None,
            collapsed_roster: false,
            collapse_player_limit: usize::MAX,
            ready_last_change: None,
            sounds: Vec::new(),
        }
    }

    pub const fn role(&self) -> LobbyRole {
        self.role
    }

    pub fn set_scenario_title(&mut self, scenario_title: impl Into<String>) {
        self.scenario_title = scenario_title.into();
    }

    /// Updates whether the optional Teams sheet and per-player team controls
    /// exist, clearing interaction state that would otherwise point at a
    /// removed control.
    pub fn set_has_teams(&mut self, has_teams: bool) {
        if self.has_teams == has_teams {
            return;
        }
        self.has_teams = has_teams;
        if has_teams {
            return;
        }

        self.open_team_combo_player = None;
        if self.active_sheet == LobbySheet::Teams {
            self.set_active_sheet(LobbySheet::Players);
        }
        self.focus = match self.focus {
            LobbyControl::TeamsTab => LobbyControl::PlayersTab,
            LobbyControl::RosterTeam if self.active_sheet.is_roster() => LobbyControl::Roster,
            LobbyControl::RosterTeam => LobbyControl::ChatInput,
            focus => focus,
        };
        if self.key_pressed.is_some_and(|(control, _)| {
            matches!(control, LobbyControl::TeamsTab | LobbyControl::RosterTeam)
        }) {
            self.key_pressed = None;
        }
        if matches!(
            self.hovered,
            HitTarget::Tab(LobbyControl::TeamsTab) | HitTarget::Team(_)
        ) {
            self.hovered = HitTarget::None;
        }
        if matches!(
            self.pointer_pressed,
            Some(HitTarget::Tab(LobbyControl::TeamsTab) | HitTarget::Team(_))
        ) {
            self.pointer_pressed = None;
            self.pointer_pressed_roster_id = None;
            self.pointer_inside_pressed = false;
        }
    }

    /// Updates whether the optional external-chat button exists, clearing
    /// interaction state that would otherwise point at the removed button.
    pub fn set_has_external_chat(&mut self, has_external_chat: bool) {
        if self.has_external_chat == has_external_chat {
            return;
        }
        self.has_external_chat = has_external_chat;
        if has_external_chat {
            return;
        }

        if self.focus == LobbyControl::ChatDialog {
            self.focus = LobbyControl::ChatInput;
        }
        if self
            .key_pressed
            .is_some_and(|(control, _)| control == LobbyControl::ChatDialog)
        {
            self.key_pressed = None;
        }
        if self.hovered == HitTarget::Tab(LobbyControl::ChatDialog) {
            self.hovered = HitTarget::None;
        }
        if self.pointer_pressed == Some(HitTarget::Tab(LobbyControl::ChatDialog)) {
            self.pointer_pressed = None;
            self.pointer_inside_pressed = false;
        }
    }

    pub const fn active_sheet(&self) -> LobbySheet {
        self.active_sheet
    }

    pub const fn resource_sheet_active(&self) -> bool {
        matches!(self.active_sheet, LobbySheet::Resources)
    }

    pub const fn option_sheet_active(&self) -> bool {
        matches!(self.active_sheet, LobbySheet::Options)
    }

    pub fn set_active_sheet(&mut self, sheet: LobbySheet) {
        if self.active_sheet == sheet {
            return;
        }
        self.active_sheet = sheet;
        self.open_team_combo_player = None;
        self.open_option_combo = None;
        self.scrollbar_drag = None;
        if matches!(
            self.hovered,
            HitTarget::RosterRow(_)
                | HitTarget::RosterBlank
                | HitTarget::RightListInert
                | HitTarget::ResourceSave(_)
                | HitTarget::AddPlayer(_)
                | HitTarget::Team(_)
                | HitTarget::OptionRow(_)
                | HitTarget::OptionValue(_)
                | HitTarget::RosterScrollTop
                | HitTarget::RosterScrollBottom
                | HitTarget::RosterScrollTrack
                | HitTarget::RosterScrollInert
                | HitTarget::Preload
        ) {
            self.hovered = HitTarget::None;
        }
        if matches!(
            self.pointer_pressed,
            Some(
                HitTarget::RosterRow(_)
                    | HitTarget::RosterBlank
                    | HitTarget::RightListInert
                    | HitTarget::ResourceSave(_)
                    | HitTarget::AddPlayer(_)
                    | HitTarget::Team(_)
                    | HitTarget::OptionRow(_)
                    | HitTarget::OptionValue(_)
                    | HitTarget::RosterScrollTop
                    | HitTarget::RosterScrollBottom
                    | HitTarget::RosterScrollTrack
                    | HitTarget::RosterScrollInert
                    | HitTarget::Preload
            )
        ) {
            self.pointer_pressed = None;
            self.pointer_pressed_roster_id = None;
            self.pointer_inside_pressed = false;
        }
        if !sheet.is_roster()
            && matches!(
                self.focus,
                LobbyControl::Roster | LobbyControl::RosterTeam | LobbyControl::RosterAddPlayer
            )
        {
            self.focus = LobbyControl::ChatInput;
            self.key_pressed = None;
        }
        if sheet != LobbySheet::Options
            && matches!(
                self.focus,
                LobbyControl::OptionsList | LobbyControl::Option(_)
            )
        {
            self.focus = LobbyControl::ChatInput;
            self.key_pressed = None;
        }
        if sheet != LobbySheet::Resources && self.focus == LobbyControl::Preload {
            self.focus = LobbyControl::ChatInput;
            self.key_pressed = None;
        }
    }

    pub const fn focus(&self) -> LobbyControl {
        self.focus
    }

    pub const fn ready(&self) -> bool {
        self.ready
    }

    pub const fn resources_loaded(&self) -> bool {
        self.resources_loaded
    }

    pub const fn preload_button_present(&self) -> bool {
        self.preload_button_present
    }

    pub const fn preload_button_visible(&self) -> bool {
        self.preload_button_present && self.preload_eligible && self.resources_loaded
    }

    /// Restores the app-owned `Game.CanPreload()` projection. `present`
    /// models the lifetime of the manual button object; `eligible` supplies
    /// the caller-owned half of its native Show/Enable gate. Resource
    /// completion is enforced independently by this controller.
    pub fn set_preload_button_state(&mut self, present: bool, eligible: bool) {
        self.preload_button_present = present;
        self.preload_eligible = present && eligible;
        self.clear_invalid_preload_interaction();
    }

    fn clear_invalid_preload_interaction(&mut self) {
        if !self.preload_button_visible() {
            if self.focus == LobbyControl::Preload {
                self.focus = LobbyControl::ResourcesTab;
            }
            if self
                .key_pressed
                .is_some_and(|(control, _)| control == LobbyControl::Preload)
            {
                self.key_pressed = None;
            }
            if matches!(self.hovered, HitTarget::Preload) {
                self.hovered = HitTarget::None;
            }
            if matches!(self.pointer_pressed, Some(HitTarget::Preload)) {
                self.pointer_pressed = None;
                self.pointer_inside_pressed = false;
            }
        }
    }

    pub const fn countdown(&self) -> LobbyCountdownState {
        self.countdown
    }

    pub const fn league_mode(&self) -> bool {
        self.league_mode
    }

    pub fn rows(&self) -> &[LobbyRosterRow] {
        &self.rows
    }

    pub fn resource_rows(&self) -> &[LobbyResourceRow] {
        &self.resource_rows
    }

    pub fn option_rows(&self) -> &[LobbyOptionRow] {
        &self.option_rows
    }

    /// Installs a live options snapshot while retaining semantic selection,
    /// focus and open state when the same row still exists.
    pub fn set_option_rows(&mut self, mut rows: Vec<LobbyOptionRow>) -> bool {
        let mut seen = Vec::with_capacity(rows.len());
        rows.retain(|row| {
            if seen.contains(&row.kind) {
                false
            } else {
                seen.push(row.kind);
                true
            }
        });
        let changed = self.option_rows != rows;
        self.option_rows = rows;
        self.open_option_combo = self.open_option_combo.filter(|kind| {
            self.option_rows
                .iter()
                .any(|row| row.kind == *kind && row.editable)
        });
        self.selected_option = self
            .selected_option
            .filter(|kind| self.option_rows.iter().any(|row| row.kind == *kind));
        if let LobbyControl::Option(kind) = self.focus {
            if !self.option_rows.iter().any(|row| row.kind == kind) {
                self.focus = LobbyControl::OptionsList;
                self.key_pressed = None;
            }
        }
        changed
    }

    pub fn set_open_option_combo(&mut self, option: Option<LobbyOptionKind>) {
        self.open_option_combo = option.filter(|kind| {
            self.option_rows
                .iter()
                .any(|row| row.kind == *kind && row.editable)
        });
    }

    pub const fn open_option_combo(&self) -> Option<LobbyOptionKind> {
        self.open_option_combo
    }

    /// Installs the current resource-list snapshot. Native lookup begins at
    /// ID zero, and its reconciliation loop is ordered by resource ID.
    pub fn set_resource_rows(&mut self, mut rows: Vec<LobbyResourceRow>) {
        rows.retain(|row| row.id >= 0);
        rows.iter_mut()
            .for_each(|row| row.present_percent = row.present_percent.min(100));
        rows.sort_by_key(|row| row.id);
        rows.dedup_by_key(|row| row.id);
        self.resource_rows = rows;
    }

    pub fn set_resource_progress(&mut self, resource_id: i32, present_percent: u8) -> bool {
        let Some(row) = self
            .resource_rows
            .iter_mut()
            .find(|row| row.id == resource_id)
        else {
            return false;
        };
        row.present_percent = present_percent.min(100);
        true
    }

    pub fn remove_resource_row(&mut self, resource_id: i32) -> bool {
        let previous_len = self.resource_rows.len();
        self.resource_rows.retain(|row| row.id != resource_id);
        self.resource_rows.len() != previous_len
    }

    pub fn set_labels(&mut self, labels: LobbyLabels) {
        self.labels = labels;
    }

    pub fn set_league_mode(&mut self, league_mode: bool) {
        self.league_mode = league_mode;
    }

    pub fn labels(&self) -> &LobbyLabels {
        &self.labels
    }

    pub fn selected_roster_id(&self) -> Option<&LobbyRosterId> {
        self.selected_roster_id.as_ref()
    }

    pub fn accepted_roster_click_id(
        &self,
        point: GuiPoint,
        _layout: &LobbyLayout,
        _roster: &LobbyRosterLayout,
    ) -> Option<LobbyRosterId> {
        let pressed_id = self.pointer_pressed_roster_id.as_ref()?;
        let last_point = self.pointer?;
        if !matches!(self.pointer_pressed, Some(HitTarget::RosterRow(_)))
            || !self.pointer_inside_pressed
            || last_point.x as i32 != point.x as i32
            || last_point.y as i32 != point.y as i32
        {
            return None;
        }
        self.rows
            .iter()
            .any(|row| &row.id() == pressed_id)
            .then(|| pressed_id.clone())
            .filter(|id| self.selected_roster_id.as_ref() == Some(id))
    }

    /// Mirrors the per-row C4GUI::ComboBox `iOpenMenu` presentation state.
    /// Menu ownership remains app-side because the shared context-menu tree
    /// is rendered above the lobby.
    pub fn set_open_team_combo_player(&mut self, player_id: Option<i32>) {
        self.open_team_combo_player = player_id.filter(|player_id| {
            self.rows.iter().any(|row| {
                matches!(
                    row,
                    LobbyRosterRow::Player(player)
                        if player.id == *player_id
                            && player.team.as_ref().is_some_and(|team| team.selectable)
                )
            })
        });
    }

    pub const fn open_team_combo_player(&self) -> Option<i32> {
        self.open_team_combo_player
    }

    pub const fn chat_scroll(&self) -> i32 {
        self.chat_scroll
    }

    pub const fn roster_scroll(&self) -> i32 {
        self.roster_scroll
    }

    pub const fn resource_scroll(&self) -> i32 {
        self.resource_scroll
    }

    pub const fn scenario_scroll(&self) -> i32 {
        self.scenario_scroll
    }

    pub const fn option_scroll(&self) -> i32 {
        self.option_scroll
    }

    pub fn scenario_text(&self) -> &LobbyScenarioText {
        &self.scenario_text
    }

    pub fn set_scenario_text(&mut self, text: LobbyScenarioText) {
        if self.scenario_text == text {
            return;
        }
        self.scenario_text = text;
        self.scenario_scroll = 0;
        self.scenario_max_scroll = 0;
        self.scenario_scroll_pin = 0;
    }

    /// Restores app-owned scroll state before a transient controller is
    /// laid out. `roster_layout` applies the current content clamp and pin.
    pub fn set_resource_scroll(&mut self, scroll: i32) {
        self.resource_scroll = scroll.max(0);
    }

    pub fn set_scenario_scroll(&mut self, scroll: i32) {
        self.scenario_scroll = scroll.max(0);
    }

    pub fn set_option_scroll(&mut self, scroll: i32) {
        self.option_scroll = scroll.max(0);
    }

    fn remapped_roster_hit(&self, target: HitTarget, id: &LobbyRosterId) -> Option<HitTarget> {
        let index = self.rows.iter().position(|row| row.id() == *id)?;
        match (target, self.rows.get(index)) {
            (HitTarget::RosterRow(_), Some(_)) => Some(HitTarget::RosterRow(index)),
            (
                HitTarget::AddPlayer(_),
                Some(
                    LobbyRosterRow::Client(LobbyClientRow { local: true, .. })
                    | LobbyRosterRow::Header(LobbyHeaderRow {
                        kind: LobbyRosterHeader::ScriptPlayers,
                        can_add_player: true,
                        ..
                    }),
                ),
            ) => Some(HitTarget::AddPlayer(index)),
            (HitTarget::Team(_), Some(LobbyRosterRow::Player(_))) if self.has_teams => {
                Some(HitTarget::Team(index))
            }
            _ => None,
        }
    }

    pub fn set_rows(&mut self, rows: Vec<LobbyRosterRow>) {
        let hovered_roster = match self.hovered {
            target @ (HitTarget::RosterRow(index)
            | HitTarget::AddPlayer(index)
            | HitTarget::Team(index)) => self.rows.get(index).map(|row| (target, row.id())),
            _ => None,
        };
        let selected_id = self.selected_roster_id.clone().or_else(|| {
            self.selected_row
                .and_then(|index| self.rows.get(index))
                .map(LobbyRosterRow::id)
        });
        let player_count = rows
            .iter()
            .filter(|row| matches!(row, LobbyRosterRow::Player(_)))
            .count();
        if self.collapsed_roster && player_count <= self.collapse_player_limit {
            self.collapsed_roster = false;
        }
        self.rows = rows;
        if let Some((target, id)) = hovered_roster {
            self.hovered = self
                .remapped_roster_hit(target, &id)
                .unwrap_or(HitTarget::None);
            if self.hovered == HitTarget::None {
                self.hover_since = Instant::now();
            }
        }
        if let (Some(target), Some(id)) =
            (self.pointer_pressed, self.pointer_pressed_roster_id.clone())
        {
            if matches!(
                target,
                HitTarget::RosterRow(_) | HitTarget::AddPlayer(_) | HitTarget::Team(_)
            ) {
                self.pointer_pressed = self.remapped_roster_hit(target, &id);
                if self.pointer_pressed.is_none() {
                    if self.pointer_inside_pressed && target.button_control().is_some() {
                        self.sounds.push(LobbySound::ArrowHit);
                    }
                    self.pointer_pressed_roster_id = None;
                    self.pointer_inside_pressed = false;
                }
            }
        }
        self.selected_row = selected_id
            .as_ref()
            .and_then(|selected| self.rows.iter().position(|row| row.id() == *selected));
        self.selected_roster_id = self
            .selected_row
            .and_then(|index| self.rows.get(index))
            .map(LobbyRosterRow::id);
        self.set_open_team_combo_player(self.open_team_combo_player);
        let selected = self.selected_row.and_then(|index| self.rows.get(index));
        let child_focus_valid = match self.focus {
            LobbyControl::RosterTeam => matches!(
                selected,
                Some(LobbyRosterRow::Player(player))
                    if self.has_teams
                        && !self.countdown.is_locked()
                        && player.team.as_ref().is_some_and(|team| team.selectable)
            ),
            LobbyControl::RosterAddPlayer => matches!(
                selected,
                Some(LobbyRosterRow::Client(LobbyClientRow { local: true, .. }))
                    | Some(LobbyRosterRow::Header(LobbyHeaderRow {
                        kind: LobbyRosterHeader::ScriptPlayers,
                        can_add_player: true,
                        ..
                    }))
            ),
            _ => true,
        };
        if !child_focus_valid {
            if self.key_pressed.is_some_and(|(control, _)| {
                matches!(
                    control,
                    LobbyControl::RosterTeam | LobbyControl::RosterAddPlayer
                )
            }) {
                self.key_pressed = None;
            }
            self.focus = if self.active_sheet.is_roster() {
                LobbyControl::Roster
            } else {
                LobbyControl::ChatInput
            };
        }
    }

    pub fn set_player_count(&mut self, active: i32, maximum: i32) {
        self.active_players = active;
        self.max_players = maximum;
    }

    pub fn set_resources_loaded(&mut self, loaded: bool) -> Vec<LobbyAction> {
        self.resources_loaded = loaded;
        self.clear_invalid_preload_interaction();
        if !loaded && self.focus == LobbyControl::Ready {
            let replacement = if self.role == LobbyRole::Host {
                LobbyControl::Run
            } else {
                self.role
                    .game_option_context()
                    .buttons()
                    .last()
                    .copied()
                    .map(LobbyControl::GameOption)
                    .unwrap_or(LobbyControl::Exit)
            };
            self.focus = replacement;
            self.key_pressed = None;
            let mut actions = vec![LobbyAction::FocusChanged(replacement)];
            if let LobbyControl::GameOption(button) = replacement {
                actions.push(LobbyAction::GameOptions(LobbyGameOptionInput::Focus(
                    button,
                )));
            }
            return actions;
        }
        Vec::new()
    }

    pub fn set_ready(&mut self, ready: bool) {
        self.ready = ready;
    }

    pub fn set_chat_draft(&mut self, draft: impl Into<String>) {
        let text = draft.into();
        self.chat_edit = LobbyChatEditView {
            caret: text.len(),
            text,
            ..LobbyChatEditView::default()
        };
    }

    pub fn set_chat_edit_view(&mut self, view: LobbyChatEditView) {
        self.chat_edit = view.normalized();
    }

    pub fn chat_edit_view(&self) -> &LobbyChatEditView {
        &self.chat_edit
    }

    pub fn set_logs(&mut self, logs: Vec<LobbyLogLine>) {
        self.logs = logs;
        self.chat_follow_bottom = true;
    }

    pub fn push_log(&mut self, line: LobbyLogLine) {
        self.logs.push(line);
        self.chat_follow_bottom = true;
    }

    pub fn logs(&self) -> &[LobbyLogLine] {
        &self.logs
    }

    /// `C4GameLobby::MainDlg::OnClientSound`: expose an accepted `/sound`
    /// command on the sender row, distinguishing the configured mute icon.
    pub fn note_client_sound(&mut self, client_id: i32, muted: bool) {
        self.note_client_sound_at(client_id, muted, Instant::now());
    }

    fn note_client_sound_at(&mut self, client_id: i32, muted: bool, now: Instant) {
        if self
            .rows
            .iter()
            .any(|row| matches!(row, LobbyRosterRow::Client(client) if client.id == client_id))
        {
            self.client_sound_status.insert(client_id, (muted, now));
        }
    }

    fn client_status_at(&self, client: &LobbyClientRow, now: Instant) -> LobbyClientStatus {
        self.client_sound_status
            .get(&client.id)
            .filter(|(_, started)| {
                now.checked_duration_since(*started)
                    .is_none_or(|elapsed| elapsed < SOUND_ICON_SHOW_TIME)
            })
            .map(|(muted, _)| {
                if *muted {
                    LobbyClientStatus::MutedSound
                } else {
                    LobbyClientStatus::Sound
                }
            })
            .unwrap_or(client.status)
    }

    pub fn take_sounds(&mut self) -> Vec<LobbySound> {
        std::mem::take(&mut self.sounds)
    }

    pub fn title(&self) -> String {
        if self.scenario_title.is_empty() {
            self.labels.lobby.clone()
        } else {
            self.labels
                .scenario_lobby_template
                .replace("{scenario}", &self.scenario_title)
                .replace("{lobby}", &self.labels.lobby)
        }
    }

    pub fn players_title(&self) -> String {
        self.labels
            .players_template
            .replace("{active}", &self.active_players.to_string())
            .replace("{maximum}", &self.max_players.to_string())
    }

    pub fn right_title(&self) -> String {
        match self.active_sheet {
            LobbySheet::Players | LobbySheet::Teams => self.players_title(),
            LobbySheet::Resources => self.labels.resources.clone(),
            LobbySheet::Options => self.labels.options.clone(),
            LobbySheet::Scenario => self.labels.scenario.clone(),
        }
    }

    pub fn layout(&self, width: i32, height: i32, fonts: &ClonkFontSet) -> LobbyLayout {
        let mut layout = game_lobby_layout(
            width,
            height,
            fonts.title.line_height,
            fonts.text.line_height,
            self.role,
            self.has_teams,
            self.has_external_chat,
        );
        layout
            .tab_buttons
            .iter_mut()
            .for_each(|tab| tab.selected = tab.sheet == Some(self.active_sheet));
        if self.active_sheet == LobbySheet::Scenario {
            let bounds = layout.roster;
            layout.roster_client = IntRect::new(
                bounds.x + SCENARIO_TEXT_LEFT_MARGIN,
                bounds.y + SCENARIO_TEXT_VERTICAL_MARGIN,
                (bounds.w
                    - SCENARIO_TEXT_LEFT_MARGIN
                    - SCENARIO_TEXT_RIGHT_MARGIN
                    - SCROLLBAR_EXTENT)
                    .max(0),
                (bounds.h - 2 * SCENARIO_TEXT_VERTICAL_MARGIN).max(0),
            );
            layout.roster_scrollbar = IntRect::new(
                bounds.x + bounds.w - SCENARIO_TEXT_RIGHT_MARGIN - SCROLLBAR_EXTENT,
                bounds.y + SCENARIO_TEXT_VERTICAL_MARGIN,
                SCROLLBAR_EXTENT,
                (bounds.h - 2 * SCENARIO_TEXT_VERTICAL_MARGIN).max(0),
            );
        }
        if self.active_sheet == LobbySheet::Resources && self.preload_button_present {
            let button = IntRect::new(
                layout.roster.x,
                layout.roster.y + (layout.roster.h - BUTTON_HEIGHT).max(0),
                layout.roster.w,
                BUTTON_HEIGHT.min(layout.roster.h),
            );
            layout.roster.h = (layout.roster.h - BUTTON_HEIGHT).max(0);
            layout.roster_client.h = (layout.roster_client.h - BUTTON_HEIGHT).max(0);
            layout.roster_scrollbar.h = (layout.roster_scrollbar.h - BUTTON_HEIGHT).max(0);
            layout.preload_button = self.preload_button_visible().then_some(button);
        }
        layout
    }

    pub fn right_list_layout(
        &mut self,
        layout: &LobbyLayout,
        fonts: &ClonkFontSet,
    ) -> LobbyRosterLayout {
        if self.active_sheet == LobbySheet::Scenario {
            let _ = self.scenario_scroll_metrics(layout, fonts);
        }
        self.roster_layout(layout, fonts.text.line_height)
    }

    pub fn roster_layout(
        &mut self,
        layout: &LobbyLayout,
        text_line_height: i32,
    ) -> LobbyRosterLayout {
        if self.active_sheet == LobbySheet::Resources {
            return self.resource_list_layout(layout, text_line_height);
        }
        if self.active_sheet == LobbySheet::Options {
            return self.option_list_layout(layout, text_line_height);
        }
        if self.active_sheet == LobbySheet::Scenario {
            return LobbyRosterLayout {
                rows: Vec::new(),
                content_height: layout.roster_client.h + self.scenario_max_scroll,
                max_scroll: self.scenario_max_scroll,
                collapsed: false,
            };
        }
        if !self.active_sheet.is_roster() {
            return LobbyRosterLayout {
                rows: Vec::new(),
                content_height: 0,
                max_scroll: 0,
                collapsed: false,
            };
        }
        let expanded_height = self.stack_rows(layout, text_line_height, false).1;
        if !self.collapsed_roster && expanded_height > layout.roster_client.h {
            let player_count = self
                .rows
                .iter()
                .filter(|row| matches!(row, LobbyRosterRow::Player(_)))
                .count();
            self.collapse_player_limit = player_count.saturating_sub(1);
            self.collapsed_roster = true;
        }
        let (rows, content_height) =
            self.stack_rows(layout, text_line_height, self.collapsed_roster);
        let max_scroll = (content_height - layout.roster_client.h).max(0);
        self.roster_scroll = self.roster_scroll.clamp(0, max_scroll);
        if max_scroll != self.roster_max_scroll {
            self.roster_scroll_pin = scroll_to_pin(
                self.roster_scroll,
                max_scroll,
                scrollbar_max_pin(layout.roster_scrollbar),
            );
            self.roster_max_scroll = max_scroll;
        }
        LobbyRosterLayout {
            rows,
            content_height,
            max_scroll,
            collapsed: self.collapsed_roster,
        }
    }

    fn resource_list_layout(
        &mut self,
        layout: &LobbyLayout,
        text_line_height: i32,
    ) -> LobbyRosterLayout {
        let icon_size = text_line_height.max(1);
        let row_height = icon_size.saturating_add(4);
        let content_height = i32::try_from(self.resource_rows.len())
            .unwrap_or(i32::MAX)
            .saturating_mul(row_height);
        let max_scroll = (content_height - layout.roster_client.h).max(0);
        self.resource_scroll = self.resource_scroll.clamp(0, max_scroll);
        if max_scroll != self.resource_max_scroll {
            self.resource_scroll_pin = scroll_to_pin(
                self.resource_scroll,
                max_scroll,
                scrollbar_max_pin(layout.roster_scrollbar),
            );
            self.resource_max_scroll = max_scroll;
        }
        let rows = self
            .resource_rows
            .iter()
            .enumerate()
            .map(|(index, row)| {
                let y = i32::try_from(index)
                    .unwrap_or(i32::MAX)
                    .saturating_mul(row_height);
                let rect = IntRect::new(
                    layout.roster_client.x,
                    layout.roster_client.y + y - self.resource_scroll,
                    layout.roster_client.w,
                    row_height,
                );
                LobbyRosterRowLayout {
                    index,
                    rect,
                    icon: IntRect::new(rect.x, rect.y + 2, icon_size, icon_size),
                    add_player: None,
                    team: None,
                    save: row.save_possible.then_some(IntRect::new(
                        rect.x + rect.w - 18,
                        rect.y + 1,
                        16,
                        16,
                    )),
                    option_value: None,
                    rank: None,
                    collapsed: false,
                }
            })
            .collect();
        LobbyRosterLayout {
            rows,
            content_height,
            max_scroll,
            collapsed: false,
        }
    }

    fn option_list_layout(
        &mut self,
        layout: &LobbyLayout,
        text_line_height: i32,
    ) -> LobbyRosterLayout {
        // OptionDropdown's non-tabular constructor uses a caption line, the
        // TextFont+4 ComboBox height and one-pixel margins around each part.
        let caption_height = text_line_height.max(1);
        let combo_height = caption_height.saturating_add(4);
        let row_height = caption_height
            .saturating_add(combo_height)
            .saturating_add(4);
        let row_count = i32::try_from(self.option_rows.len()).unwrap_or(i32::MAX);
        let row_pitch = row_height.saturating_add(DEFAULT_ROW_SPACING);
        let content_height = row_count.saturating_mul(row_height).saturating_add(
            row_count
                .saturating_sub(1)
                .max(0)
                .saturating_mul(DEFAULT_ROW_SPACING),
        );
        let max_scroll = (content_height - layout.roster_client.h).max(0);
        self.option_scroll = self.option_scroll.clamp(0, max_scroll);
        if max_scroll != self.option_max_scroll {
            self.option_scroll_pin = scroll_to_pin(
                self.option_scroll,
                max_scroll,
                scrollbar_max_pin(layout.roster_scrollbar),
            );
            self.option_max_scroll = max_scroll;
        }
        let rows = self
            .option_rows
            .iter()
            .enumerate()
            .map(|(index, _)| {
                let y = i32::try_from(index)
                    .unwrap_or(i32::MAX)
                    .saturating_mul(row_pitch);
                let rect = IntRect::new(
                    layout.roster_client.x,
                    layout.roster_client.y + y - self.option_scroll,
                    layout.roster_client.w,
                    row_height,
                );
                LobbyRosterRowLayout {
                    index,
                    rect,
                    icon: IntRect::new(rect.x, rect.y, 0, 0),
                    add_player: None,
                    team: None,
                    save: None,
                    option_value: Some(IntRect::new(
                        rect.x + 6,
                        rect.y + caption_height + 3,
                        (rect.w - 7).max(0),
                        combo_height,
                    )),
                    rank: None,
                    collapsed: false,
                }
            })
            .collect();
        LobbyRosterLayout {
            rows,
            content_height,
            max_scroll,
            collapsed: false,
        }
    }

    pub fn chat_scroll_metrics(
        &mut self,
        layout: &LobbyLayout,
        font: &clonk_graphics::clonk_font::ClonkFont,
    ) -> LobbyChatScrollMetrics {
        let lines = self.wrapped_chat_lines(layout, font);
        let content_height = lines
            .iter()
            .enumerate()
            .map(|(index, line)| {
                font.line_height
                    + if index > 0 && line.new_paragraph {
                        font.line_height / 3
                    } else {
                        0
                    }
            })
            .sum::<i32>()
            .max(5);
        let max_scroll = (content_height - layout.chat_log_client.h).max(0);
        let max_pin = scrollbar_max_pin(layout.chat_log_scrollbar);
        if self.chat_follow_bottom {
            self.chat_scroll = max_scroll;
            self.chat_scroll_pin = max_pin;
        } else {
            self.chat_scroll = self.chat_scroll.clamp(0, max_scroll);
            if max_scroll != self.chat_max_scroll {
                self.chat_scroll_pin = scroll_to_pin(self.chat_scroll, max_scroll, max_pin);
            }
        }
        self.chat_max_scroll = max_scroll;
        LobbyChatScrollMetrics {
            content_height,
            max_scroll,
            scroll: self.chat_scroll,
        }
    }

    fn wrapped_chat_lines(
        &self,
        layout: &LobbyLayout,
        font: &clonk_graphics::clonk_font::ClonkFont,
    ) -> Vec<WrappedLobbyLogLine> {
        let mut wrapped = Vec::new();
        for line in &self.logs {
            for paragraph in line
                .text
                .split(['\r', '\n', '|'])
                .filter(|paragraph| !paragraph.is_empty())
            {
                let text = break_message(font, paragraph, layout.chat_log_client.w.max(1));
                for (physical_index, physical) in text
                    .split('\n')
                    .filter(|physical| !physical.is_empty())
                    .enumerate()
                {
                    wrapped.push(WrappedLobbyLogLine {
                        text: physical.to_string(),
                        color: line.color,
                        new_paragraph: physical_index == 0,
                    });
                    while wrapped.len() > 100
                        || wrapped
                            .iter()
                            .map(|line| line.text.len() + 1)
                            .sum::<usize>()
                            > 4096
                    {
                        wrapped.remove(0);
                    }
                }
            }
        }
        wrapped
    }

    pub fn scenario_scroll_metrics(
        &mut self,
        layout: &LobbyLayout,
        fonts: &ClonkFontSet,
    ) -> LobbyScenarioScrollMetrics {
        let lines = self.wrapped_scenario_lines(layout, fonts);
        let content_height = lines
            .iter()
            .enumerate()
            .map(|(index, line)| {
                let font = if line.title_font {
                    &fonts.caption
                } else {
                    &fonts.text
                };
                font.line_height
                    + if index > 0 && line.new_paragraph {
                        font.line_height / 3
                    } else {
                        0
                    }
            })
            .sum::<i32>();
        let max_scroll = (content_height - layout.roster_client.h).max(0);
        self.scenario_scroll = self.scenario_scroll.clamp(0, max_scroll);
        if max_scroll != self.scenario_max_scroll {
            self.scenario_scroll_pin = scroll_to_pin(
                self.scenario_scroll,
                max_scroll,
                scrollbar_max_pin(layout.roster_scrollbar),
            );
            self.scenario_max_scroll = max_scroll;
        }
        LobbyScenarioScrollMetrics {
            content_height,
            max_scroll,
            scroll: self.scenario_scroll,
        }
    }

    fn wrapped_scenario_lines(
        &self,
        layout: &LobbyLayout,
        fonts: &ClonkFontSet,
    ) -> Vec<WrappedLobbyScenarioLine> {
        let text = self.scenario_text.text();
        let title_only = matches!(self.scenario_text, LobbyScenarioText::Title(_));
        let first_description_line_is_title =
            matches!(self.scenario_text, LobbyScenarioText::Description(_))
                && (text.contains('\r') || text.contains('\n'));
        let mut wrapped = Vec::new();
        for (paragraph_index, paragraph) in text
            .split(['\r', '\n'])
            .filter(|paragraph| !paragraph.is_empty())
            .enumerate()
        {
            let title_font = title_only || first_description_line_is_title && paragraph_index == 0;
            let font = if title_font {
                &fonts.caption
            } else {
                &fonts.text
            };
            let physical_lines = break_message(font, paragraph, layout.roster_client.w.max(1));
            for (physical_index, physical) in physical_lines
                .split('\n')
                .filter(|physical| !physical.is_empty())
                .enumerate()
            {
                wrapped.push(WrappedLobbyScenarioLine {
                    text: physical.to_string(),
                    title_font,
                    new_paragraph: physical_index == 0,
                });
            }
        }
        wrapped
    }

    fn stack_rows(
        &self,
        layout: &LobbyLayout,
        text_line_height: i32,
        collapse: bool,
    ) -> (Vec<LobbyRosterRowLayout>, i32) {
        let mut y = 0;
        let mut rows = Vec::with_capacity(self.rows.len());
        for (index, row) in self.rows.iter().enumerate() {
            if index != 0 {
                y += row.top_spacing();
            }
            let collapsed = collapse
                && matches!(row, LobbyRosterRow::Player(_))
                && self.selected_row != Some(index);
            let height = match row {
                LobbyRosterRow::Player(_) if !collapsed => text_line_height * 2 + 10,
                LobbyRosterRow::Player(_) => text_line_height + 2 * ICON_LABEL_SPACING,
                LobbyRosterRow::Client(_) | LobbyRosterRow::Header(_) => text_line_height,
            };
            let indent = row.indent();
            let rect = IntRect::new(
                layout.roster_client.x + indent,
                layout.roster_client.y + y - self.roster_scroll,
                layout.roster_client.w - indent,
                height,
            );
            let icon = IntRect::new(rect.x, rect.y, height, height);
            let add_player = match row {
                LobbyRosterRow::Client(client) if client.local => Some(IntRect::new(
                    rect.x + rect.w - height - 2,
                    rect.y,
                    height,
                    height,
                )),
                LobbyRosterRow::Header(header)
                    if header.kind == LobbyRosterHeader::ScriptPlayers && header.can_add_player =>
                {
                    Some(IntRect::new(
                        rect.x + rect.w - height - 2,
                        rect.y,
                        height,
                        height,
                    ))
                }
                _ => None,
            };
            let (team, rank) = match row {
                LobbyRosterRow::Player(_) if !collapsed => {
                    let team_y = rect.y + height - (text_line_height + 4) - ICON_LABEL_SPACING;
                    let mut team_rect = IntRect::new(
                        rect.x + height + 2 * ICON_LABEL_SPACING + 2,
                        team_y,
                        rect.w - height - 4 * ICON_LABEL_SPACING,
                        text_line_height + 4,
                    );
                    let rank = self.league_mode.then(|| {
                        let rank_rect = IntRect::new(
                            team_rect.x + team_rect.w - team_rect.h,
                            team_rect.y,
                            team_rect.h,
                            team_rect.h,
                        );
                        team_rect.w -= rank_rect.w;
                        rank_rect
                    });
                    (self.has_teams.then_some(team_rect), rank)
                }
                _ => (None, None),
            };
            rows.push(LobbyRosterRowLayout {
                index,
                rect,
                icon,
                add_player,
                team,
                save: None,
                option_value: None,
                rank,
                collapsed,
            });
            y += height;
        }
        (rows, y)
    }

    pub fn pointer_move(
        &mut self,
        point: GuiPoint,
        layout: &LobbyLayout,
        roster: &LobbyRosterLayout,
    ) -> Vec<LobbyAction> {
        let pointer_moved = self.pointer.is_none_or(|previous| {
            previous.x as i32 != point.x as i32 || previous.y as i32 != point.y as i32
        });
        self.pointer = Some(point);
        if pointer_moved {
            self.tooltip_pointer_active = true;
        }
        let previous_hover = self.hovered;
        let hit = self.hit_test(point, layout, roster);
        self.hovered = hit;
        if previous_hover != hit || pointer_moved {
            self.hover_since = Instant::now();
        }
        match self.scrollbar_drag {
            Some(ScrollbarDrag::Chat) => self.set_chat_scroll_from_pointer(point, layout),
            Some(ScrollbarDrag::Roster) => self.set_scroll_from_pointer(point, layout, roster),
            None => {}
        }
        if let Some(pressed) = self.pointer_pressed {
            if pressed.is_scroll_arrow() && pressed != hit {
                self.pointer_pressed = None;
                self.pointer_inside_pressed = false;
                self.sounds.push(LobbySound::ArrowHit);
            }
            let now_inside = match (self.pointer_pressed_roster_id.as_ref(), pressed, hit) {
                (Some(pressed_id), HitTarget::RosterRow(_), HitTarget::RosterRow(index))
                | (Some(pressed_id), HitTarget::AddPlayer(_), HitTarget::AddPlayer(index))
                | (Some(pressed_id), HitTarget::Team(_), HitTarget::Team(index)) => self
                    .rows
                    .get(index)
                    .is_some_and(|row| &row.id() == pressed_id),
                (Some(_), _, _) => false,
                (None, _, _) => pressed == hit,
            };
            if let Some(control) = pressed.button_control() {
                if self.pointer_inside_pressed && !now_inside {
                    self.pointer_inside_pressed = false;
                    self.sounds.push(LobbySound::ArrowHit);
                    if self
                        .key_pressed
                        .is_some_and(|(pressed_control, _)| pressed_control == control)
                    {
                        self.key_pressed = None;
                    }
                } else if !self.pointer_inside_pressed && now_inside && previous_hover != hit {
                    let key_already_down = self
                        .key_pressed
                        .is_some_and(|(pressed_control, _)| pressed_control == control);
                    self.pointer_inside_pressed = true;
                    if !key_already_down {
                        self.sounds.push(LobbySound::ArrowHit);
                    }
                }
            } else if self.pointer_pressed_roster_id.is_some() {
                self.pointer_inside_pressed = now_inside;
            }
        }
        let mut actions = Vec::new();
        if matches!(previous_hover, HitTarget::GameOption(_))
            && !matches!(hit, HitTarget::GameOption(_))
        {
            actions.push(LobbyAction::GameOptions(LobbyGameOptionInput::MouseLeave));
        }
        if matches!(hit, HitTarget::GameOption(_))
            || matches!(self.pointer_pressed, Some(HitTarget::GameOption(_)))
        {
            actions.push(LobbyAction::GameOptions(LobbyGameOptionInput::PointerMove(
                point,
            )));
        }
        if self.pointer_pressed == Some(HitTarget::ChatInput) {
            actions.push(LobbyAction::Chat(LobbyChatRequest::PointerMove(point)));
        }
        actions
    }

    pub fn pointer_down(
        &mut self,
        point: GuiPoint,
        layout: &LobbyLayout,
        roster: &LobbyRosterLayout,
    ) -> Vec<LobbyAction> {
        let previous_focus = self.focus;
        self.pointer = Some(point);
        self.tooltip_pointer_active = true;
        let hit = self.hit_test(point, layout, roster);
        self.hovered = hit;
        self.hover_since = Instant::now();
        self.pointer_pressed_roster_id = None;
        match hit {
            HitTarget::GameOption(_) => {
                // Buttons deliberately preserve chat focus on mouse clicks.
                self.pointer_pressed = Some(hit);
                self.pointer_inside_pressed = true;
                return vec![LobbyAction::GameOptions(LobbyGameOptionInput::PointerDown(
                    point,
                ))];
            }
            HitTarget::RosterRow(index) => {
                let changed = self.focus != LobbyControl::Roster;
                self.change_focus(LobbyControl::Roster, false);
                let mut actions = self.select_row(Some(index), true, layout, roster);
                self.pointer_pressed = Some(hit);
                self.pointer_pressed_roster_id = self.rows.get(index).map(LobbyRosterRow::id);
                self.pointer_inside_pressed = true;
                if changed {
                    actions.insert(0, LobbyAction::FocusChanged(LobbyControl::Roster));
                }
                self.append_game_option_focus_clear(previous_focus, &mut actions);
                return actions;
            }
            HitTarget::RosterBlank => {
                let changed = self.focus != LobbyControl::Roster;
                self.change_focus(LobbyControl::Roster, false);
                let mut actions = self.select_row(None, true, layout, roster);
                if changed {
                    actions.insert(0, LobbyAction::FocusChanged(LobbyControl::Roster));
                }
                self.append_game_option_focus_clear(previous_focus, &mut actions);
                return actions;
            }
            HitTarget::AddPlayer(index) => {
                let changed = self.focus != LobbyControl::Roster;
                self.change_focus(LobbyControl::Roster, false);
                let mut actions = self.select_row(Some(index), true, layout, roster);
                if changed {
                    actions.insert(0, LobbyAction::FocusChanged(LobbyControl::Roster));
                }
                let already_down = self.button_is_down(LobbyControl::RosterAddPlayer);
                if !already_down {
                    self.sounds.push(LobbySound::ArrowHit);
                }
                self.pointer_pressed = Some(hit);
                self.pointer_pressed_roster_id = self.rows.get(index).map(LobbyRosterRow::id);
                self.pointer_inside_pressed = true;
                self.append_game_option_focus_clear(previous_focus, &mut actions);
                return actions;
            }
            HitTarget::Team(index) => {
                let changed = self.focus != LobbyControl::Roster;
                self.change_focus(LobbyControl::Roster, false);
                let mut actions = self.select_row(Some(index), true, layout, roster);
                if changed {
                    actions.insert(0, LobbyAction::FocusChanged(LobbyControl::Roster));
                }
                if let Some(LobbyRosterRow::Player(player)) = self.rows.get(index) {
                    if player.team.as_ref().is_some_and(|team| team.selectable)
                        && !self.countdown.is_locked()
                    {
                        actions.push(LobbyAction::TeamSelectionRequested {
                            player_id: player.id,
                        });
                    }
                }
                self.append_game_option_focus_clear(previous_focus, &mut actions);
                return actions;
            }
            HitTarget::OptionRow(index) | HitTarget::OptionValue(index) => {
                let Some(kind) = self.option_rows.get(index).map(|row| row.kind) else {
                    return Vec::new();
                };
                let control = LobbyControl::Option(kind);
                let changed = self.focus != control;
                self.change_focus(control, false);
                self.select_option(Some(kind), true, layout, roster);
                let mut actions = if hit == HitTarget::OptionValue(index) {
                    self.option_selection_request_by_index(index, roster)
                } else {
                    Vec::new()
                };
                if changed {
                    actions.insert(0, LobbyAction::FocusChanged(control));
                }
                self.append_game_option_focus_clear(previous_focus, &mut actions);
                return actions;
            }
            HitTarget::RightListInert if self.active_sheet == LobbySheet::Options => {
                let changed = self.focus != LobbyControl::OptionsList;
                self.change_focus(LobbyControl::OptionsList, false);
                self.select_option(None, true, layout, roster);
                let mut actions = Vec::new();
                if changed {
                    actions.push(LobbyAction::FocusChanged(LobbyControl::OptionsList));
                }
                self.append_game_option_focus_clear(previous_focus, &mut actions);
                return actions;
            }
            HitTarget::ChatInput => {
                let changed = self.focus != LobbyControl::ChatInput;
                self.change_focus(LobbyControl::ChatInput, false);
                self.pointer_pressed = Some(hit);
                let mut actions = vec![LobbyAction::Chat(LobbyChatRequest::PointerDown(point))];
                if changed {
                    actions.insert(0, LobbyAction::FocusChanged(LobbyControl::ChatInput));
                }
                self.append_game_option_focus_clear(previous_focus, &mut actions);
                return actions;
            }
            HitTarget::ChatLabel => {
                let changed = self.focus != LobbyControl::ChatInput;
                if !changed {
                    return Vec::new();
                }
                self.change_focus(LobbyControl::ChatInput, false);
                let mut actions = vec![
                    LobbyAction::FocusChanged(LobbyControl::ChatInput),
                    LobbyAction::Chat(LobbyChatRequest::FocusInput),
                ];
                self.append_game_option_focus_clear(previous_focus, &mut actions);
                return actions;
            }
            HitTarget::RightCaption => {
                if self.active_sheet.is_roster() && self.focus != LobbyControl::Roster {
                    self.change_focus(LobbyControl::Roster, false);
                    let mut actions = vec![LobbyAction::FocusChanged(LobbyControl::Roster)];
                    self.append_game_option_focus_clear(previous_focus, &mut actions);
                    return actions;
                }
            }
            HitTarget::ChatScrollTop => {
                self.sounds.push(LobbySound::ArrowHit);
                self.pointer_pressed = Some(hit);
                self.chat_follow_bottom = false;
            }
            HitTarget::ChatScrollBottom => {
                self.sounds.push(LobbySound::ArrowHit);
                self.pointer_pressed = Some(hit);
                self.chat_follow_bottom = false;
            }
            HitTarget::ChatScrollTrack => {
                if self.chat_max_scroll > 0 {
                    self.sounds.push(LobbySound::Command);
                    self.chat_follow_bottom = false;
                    self.scrollbar_drag = Some(ScrollbarDrag::Chat);
                    self.pointer_pressed = Some(hit);
                    self.set_chat_scroll_from_pointer(point, layout);
                }
            }
            HitTarget::RosterScrollTop => {
                let changed = self.active_sheet.is_roster() && self.focus != LobbyControl::Roster;
                if self.active_sheet.is_roster() {
                    self.change_focus(LobbyControl::Roster, false);
                }
                self.sounds.push(LobbySound::ArrowHit);
                self.pointer_pressed = Some(hit);
                if changed {
                    let mut actions = vec![LobbyAction::FocusChanged(LobbyControl::Roster)];
                    self.append_game_option_focus_clear(previous_focus, &mut actions);
                    return actions;
                }
            }
            HitTarget::RosterScrollBottom => {
                let changed = self.active_sheet.is_roster() && self.focus != LobbyControl::Roster;
                if self.active_sheet.is_roster() {
                    self.change_focus(LobbyControl::Roster, false);
                }
                self.sounds.push(LobbySound::ArrowHit);
                self.pointer_pressed = Some(hit);
                if changed {
                    let mut actions = vec![LobbyAction::FocusChanged(LobbyControl::Roster)];
                    self.append_game_option_focus_clear(previous_focus, &mut actions);
                    return actions;
                }
            }
            HitTarget::RosterScrollTrack => {
                let changed = self.active_sheet.is_roster() && self.focus != LobbyControl::Roster;
                if self.active_sheet.is_roster() {
                    self.change_focus(LobbyControl::Roster, false);
                }
                if roster.max_scroll > 0 {
                    self.sounds.push(LobbySound::Command);
                    self.scrollbar_drag = Some(ScrollbarDrag::Roster);
                    self.pointer_pressed = Some(hit);
                    self.set_scroll_from_pointer(point, layout, roster);
                }
                if changed {
                    let mut actions = vec![LobbyAction::FocusChanged(LobbyControl::Roster)];
                    self.append_game_option_focus_clear(previous_focus, &mut actions);
                    return actions;
                }
            }
            HitTarget::RosterScrollInert
                if self.active_sheet.is_roster() && self.focus != LobbyControl::Roster =>
            {
                self.change_focus(LobbyControl::Roster, false);
                let mut actions = vec![LobbyAction::FocusChanged(LobbyControl::Roster)];
                self.append_game_option_focus_clear(previous_focus, &mut actions);
                return actions;
            }
            _ => {}
        }
        if let Some(control) = hit.button_control() {
            let already_down = self.button_is_down(control);
            if !already_down {
                self.sounds.push(LobbySound::ArrowHit);
            }
            self.pointer_pressed = Some(hit);
            self.pointer_inside_pressed = true;
        }
        Vec::new()
    }

    pub fn pointer_up(
        &mut self,
        point: GuiPoint,
        layout: &LobbyLayout,
        roster: &LobbyRosterLayout,
        now: Instant,
    ) -> Vec<LobbyAction> {
        self.pointer = Some(point);
        self.tooltip_pointer_active = true;
        let hit = self.hit_test(point, layout, roster);
        self.hovered = hit;
        self.hover_since = Instant::now();
        self.scrollbar_drag = None;
        let pressed_roster_id = self.pointer_pressed_roster_id.take();
        if matches!(self.pointer_pressed, Some(HitTarget::GameOption(_))) {
            self.pointer_pressed = None;
            self.pointer_inside_pressed = false;
            return vec![LobbyAction::GameOptions(LobbyGameOptionInput::PointerUp(
                point,
            ))];
        }
        if self.pointer_pressed == Some(HitTarget::ChatInput) {
            self.pointer_pressed = None;
            return vec![LobbyAction::Chat(LobbyChatRequest::PointerUp(point))];
        }
        if matches!(
            self.pointer_pressed,
            Some(
                HitTarget::ChatScrollTop
                    | HitTarget::ChatScrollBottom
                    | HitTarget::ChatScrollTrack
                    | HitTarget::RosterScrollTop
                    | HitTarget::RosterScrollBottom
                    | HitTarget::RosterScrollTrack
            )
        ) {
            let pressed = self.pointer_pressed.take();
            self.pointer_inside_pressed = false;
            if pressed.is_some_and(HitTarget::is_scroll_arrow) {
                self.sounds.push(LobbySound::ArrowHit);
            }
            return Vec::new();
        }
        if hit == HitTarget::Ready && self.resources_loaded {
            return self.try_toggle_ready(now);
        }
        let pressed = self.pointer_pressed;
        let semantic_child_hit = match (pressed, pressed_roster_id.as_ref()) {
            (Some(target @ (HitTarget::AddPlayer(_) | HitTarget::Team(_))), Some(pressed_id))
                if self.pointer_inside_pressed =>
            {
                self.remapped_roster_hit(target, pressed_id)
                    .filter(|remapped| *remapped == hit)
            }
            (Some(HitTarget::AddPlayer(_) | HitTarget::Team(_)), Some(_)) => None,
            _ => Some(hit),
        };
        let pressed_matches = match (pressed, pressed_roster_id.as_ref()) {
            (Some(HitTarget::AddPlayer(_) | HitTarget::Team(_)), Some(_)) => {
                semantic_child_hit.is_some()
            }
            _ => pressed == Some(hit),
        };
        if !pressed_matches && self.pointer_inside_pressed {
            if let Some(control) = pressed.and_then(HitTarget::button_control) {
                self.pointer_inside_pressed = false;
                self.sounds.push(LobbySound::ArrowHit);
                if self
                    .key_pressed
                    .is_some_and(|(pressed_control, _)| pressed_control == control)
                {
                    self.key_pressed = None;
                }
            }
        }
        let button_was_down = pressed
            .and_then(HitTarget::button_control)
            .is_some_and(|control| self.button_is_down(control));
        self.pointer_pressed = None;
        self.pointer_inside_pressed = false;
        if !pressed_matches || !button_was_down {
            return Vec::new();
        }
        if pressed.is_some() {
            self.sounds.push(LobbySound::Click);
        }
        let activated = semantic_child_hit.unwrap_or(hit);
        if let Some(control) = activated.button_control() {
            if self
                .key_pressed
                .is_some_and(|(pressed_control, _)| pressed_control == control)
            {
                self.key_pressed = None;
            }
        }
        self.activate_hit(activated)
    }

    pub fn pointer_secondary_down(
        &mut self,
        point: GuiPoint,
        layout: &LobbyLayout,
        roster: &LobbyRosterLayout,
    ) -> Vec<LobbyAction> {
        self.note_pointer_button(point, layout, roster);
        let hit = self.hovered;
        match hit {
            HitTarget::ChatInput => vec![LobbyAction::Chat(LobbyChatRequest::OpenContextMenu {
                anchor: point,
            })],
            HitTarget::RightCaption => {
                vec![LobbyAction::TabContextRequested { position: point }]
            }
            HitTarget::RosterRow(index) | HitTarget::AddPlayer(index) | HitTarget::Team(index) => {
                match self.rows.get(index) {
                    Some(row @ (LobbyRosterRow::Client(_) | LobbyRosterRow::Player(_))) => {
                        vec![LobbyAction::RosterContextRequested {
                            row: row.id(),
                            position: point,
                        }]
                    }
                    Some(LobbyRosterRow::Header(_)) | None => Vec::new(),
                }
            }
            _ => Vec::new(),
        }
    }

    pub fn pointer_double_click(
        &mut self,
        point: GuiPoint,
        layout: &LobbyLayout,
        roster: &LobbyRosterLayout,
    ) -> Vec<LobbyAction> {
        self.note_pointer_button(point, layout, roster);
        let hit = self.hovered;
        if hit == HitTarget::ChatInput {
            vec![LobbyAction::Chat(LobbyChatRequest::PointerDoubleClick(
                point,
            ))]
        } else if let HitTarget::RosterRow(index) = hit {
            let row = self.rows.get(index).map(LobbyRosterRow::id);
            row.as_ref()
                .map(|row| self.roster_double_click(row))
                .unwrap_or_default()
        } else {
            Vec::new()
        }
    }

    /// Dispatches a roster double-click through the semantic row captured on
    /// pointer-down. Collapsing the previously selected player can move rows
    /// before button-up; native list items retain their identity across that
    /// reflow rather than re-targeting the release by its new coordinates.
    pub fn roster_double_click(&self, row: &LobbyRosterId) -> Vec<LobbyAction> {
        match row {
            LobbyRosterId::Header(LobbyRosterHeader::Team(team_id)) => {
                vec![LobbyAction::MoveLocalPlayersIntoTeamRequested { team_id: *team_id }]
            }
            LobbyRosterId::Client(_) | LobbyRosterId::Player(_) | LobbyRosterId::Header(_) => {
                Vec::new()
            }
        }
    }

    pub fn pointer_middle_down(
        &mut self,
        point: GuiPoint,
        layout: &LobbyLayout,
        roster: &LobbyRosterLayout,
    ) -> Vec<LobbyAction> {
        self.note_pointer_button(point, layout, roster);
        let hit = self.hovered;
        if hit == HitTarget::ChatInput {
            vec![LobbyAction::Chat(LobbyChatRequest::PointerMiddleDown(
                point,
            ))]
        } else {
            Vec::new()
        }
    }

    pub fn touch_start(
        &mut self,
        point: GuiPoint,
        layout: &LobbyLayout,
        roster: &LobbyRosterLayout,
    ) -> Vec<LobbyAction> {
        self.pointer_down(point, layout, roster)
    }

    pub fn touch_move(
        &mut self,
        point: GuiPoint,
        layout: &LobbyLayout,
        roster: &LobbyRosterLayout,
    ) -> Vec<LobbyAction> {
        self.pointer_move(point, layout, roster)
    }

    pub fn touch_end(
        &mut self,
        point: GuiPoint,
        layout: &LobbyLayout,
        roster: &LobbyRosterLayout,
        now: Instant,
    ) -> Vec<LobbyAction> {
        self.pointer_up(point, layout, roster, now)
    }

    pub fn touch_cancel(&mut self) -> Vec<LobbyAction> {
        let pressed = self.pointer_pressed.take();
        self.pointer_pressed_roster_id = None;
        let was_inside = self.pointer_inside_pressed;
        self.scrollbar_drag = None;
        self.pointer_inside_pressed = false;
        if matches!(pressed, Some(HitTarget::GameOption(_))) {
            vec![LobbyAction::GameOptions(LobbyGameOptionInput::TouchCancel)]
        } else if pressed == Some(HitTarget::ChatInput) {
            vec![LobbyAction::Chat(LobbyChatRequest::TouchCancel)]
        } else {
            if was_inside
                && (pressed.and_then(HitTarget::button_control).is_some()
                    || pressed.is_some_and(HitTarget::is_scroll_arrow))
            {
                self.sounds.push(LobbySound::ArrowHit);
            }
            Vec::new()
        }
    }

    /// Applies an ordinary OS cursor leave. Mouse hover/drag state is cleared,
    /// but a keyboard/gamepad `fDown` latch remains active until key-up, as in
    /// `C4GUI::Button::MouseLeave`.
    pub fn pointer_left(&mut self) {
        self.pointer = None;
        self.tooltip_pointer_active = false;
        self.hovered = HitTarget::None;
        self.hover_since = Instant::now();
        if let Some(pressed) = self.pointer_pressed {
            if pressed.is_scroll_arrow() {
                self.pointer_pressed = None;
            } else if self.pointer_inside_pressed {
                if let Some(control) = pressed.button_control() {
                    self.sounds.push(LobbySound::ArrowHit);
                    if self
                        .key_pressed
                        .is_some_and(|(pressed_control, _)| pressed_control == control)
                    {
                        self.key_pressed = None;
                    }
                }
            }
        }
        self.pointer_inside_pressed = false;
    }

    /// Clears every local input latch when the OS/controller cancels an
    /// interaction. This is deliberately side-effect free: app-owned nested
    /// controls are cancelled by their owners, while pending local sounds stay
    /// available through [`Self::take_sounds`].
    pub fn cancel_interaction(&mut self) {
        let _ = self.touch_cancel();
        self.key_pressed = None;
        self.pointer_left();
    }

    /// Mirrors `CMouse::ResetActiveInput()`: non-pointer input hides an
    /// existing tooltip without discarding the last cursor position. A
    /// same-pixel motion event therefore stays inactive; only actual integer
    /// pointer motion (or a new pointer press) reactivates tooltip timing.
    pub fn note_non_pointer_input(&mut self) {
        self.tooltip_pointer_active = false;
    }

    pub fn note_pointer_wheel(&mut self) {
        self.tooltip_pointer_active = self.pointer.is_some();
        self.hover_since = Instant::now();
    }

    pub fn note_pointer_button(
        &mut self,
        point: GuiPoint,
        layout: &LobbyLayout,
        roster: &LobbyRosterLayout,
    ) {
        self.pointer = Some(point);
        self.tooltip_pointer_active = true;
        self.hover_since = Instant::now();
        self.hovered = self.hit_test(point, layout, roster);
    }

    pub fn wheel(
        &mut self,
        point: GuiPoint,
        delta: i32,
        layout: &LobbyLayout,
        roster: &LobbyRosterLayout,
    ) -> bool {
        if delta == 0 {
            return false;
        }
        if contains(layout.chat_log, point) && self.chat_max_scroll > 0 {
            self.chat_follow_bottom = false;
            self.chat_scroll = (self.chat_scroll - delta).clamp(0, self.chat_max_scroll);
            self.chat_scroll_pin = scroll_to_pin(
                self.chat_scroll,
                self.chat_max_scroll,
                scrollbar_max_pin(layout.chat_log_scrollbar),
            );
            return true;
        }
        if contains(layout.roster, point) && roster.max_scroll > 0 {
            let max_pin = scrollbar_max_pin(layout.roster_scrollbar);
            if self.active_sheet == LobbySheet::Resources {
                self.resource_scroll = (self.resource_scroll - delta).clamp(0, roster.max_scroll);
                self.resource_scroll_pin =
                    scroll_to_pin(self.resource_scroll, roster.max_scroll, max_pin);
            } else if self.active_sheet == LobbySheet::Scenario {
                self.scenario_scroll = (self.scenario_scroll - delta).clamp(0, roster.max_scroll);
                self.scenario_scroll_pin =
                    scroll_to_pin(self.scenario_scroll, roster.max_scroll, max_pin);
            } else if self.active_sheet == LobbySheet::Options {
                self.option_scroll = (self.option_scroll - delta).clamp(0, roster.max_scroll);
                self.option_scroll_pin =
                    scroll_to_pin(self.option_scroll, roster.max_scroll, max_pin);
            } else if self.active_sheet.is_roster() {
                self.roster_scroll = (self.roster_scroll - delta).clamp(0, roster.max_scroll);
                self.roster_scroll_pin =
                    scroll_to_pin(self.roster_scroll, roster.max_scroll, max_pin);
            } else {
                return false;
            }
            return true;
        }
        false
    }

    pub fn key_down(
        &mut self,
        key: KeyCode,
        shift: bool,
        layout: &LobbyLayout,
        roster: &LobbyRosterLayout,
        now: Instant,
    ) -> Vec<LobbyAction> {
        match key {
            KeyCode::Escape => return vec![LobbyAction::ExitRequested],
            KeyCode::Tab => return self.focus_next(shift),
            _ => {}
        }
        match self.focus {
            LobbyControl::ChatInput => match key {
                KeyCode::Enter => {
                    return vec![LobbyAction::Chat(LobbyChatRequest::Submit(
                        self.chat_edit.text.clone(),
                    ))]
                }
                KeyCode::Up => {
                    return vec![LobbyAction::Chat(LobbyChatRequest::History { older: true })]
                }
                KeyCode::Down => {
                    return vec![LobbyAction::Chat(LobbyChatRequest::History {
                        older: false,
                    })]
                }
                KeyCode::Left => {
                    return vec![LobbyAction::Chat(LobbyChatRequest::EditKey {
                        key: LobbyChatEditKey::Left,
                        modifiers: LobbyChatKeyModifiers {
                            shift,
                            control: false,
                        },
                    })]
                }
                KeyCode::Right => {
                    return vec![LobbyAction::Chat(LobbyChatRequest::EditKey {
                        key: LobbyChatEditKey::Right,
                        modifiers: LobbyChatKeyModifiers {
                            shift,
                            control: false,
                        },
                    })]
                }
                _ => {}
            },
            LobbyControl::Roster => match key {
                KeyCode::Up => return self.move_selection(-1, layout, roster),
                KeyCode::Down => return self.move_selection(1, layout, roster),
                KeyCode::Home => return self.edge_selection(false, layout, roster),
                KeyCode::End => return self.edge_selection(true, layout, roster),
                KeyCode::PageUp => return self.page_selection(false, layout, roster),
                KeyCode::PageDown => return self.page_selection(true, layout, roster),
                _ => {}
            },
            LobbyControl::OptionsList => match key {
                KeyCode::Up => return self.move_option_selection(-1, layout, roster),
                KeyCode::Down => return self.move_option_selection(1, layout, roster),
                KeyCode::Space => return Vec::new(),
                _ => {}
            },
            LobbyControl::Option(option) if matches!(key, KeyCode::Down | KeyCode::Space) => {
                let actions = self.option_selection_request(option, roster);
                if !actions.is_empty() || key == KeyCode::Space {
                    return actions;
                }
                return self.move_option_selection(1, layout, roster);
            }
            LobbyControl::Option(_) if key == KeyCode::Up => {
                return self.move_option_selection(-1, layout, roster)
            }
            LobbyControl::Ready if key == KeyCode::Space => return self.try_toggle_ready(now),
            LobbyControl::GameOption(_) => {
                return vec![LobbyAction::GameOptions(LobbyGameOptionInput::KeyDown {
                    key,
                    shift,
                })]
            }
            LobbyControl::RosterTeam if matches!(key, KeyCode::Down | KeyCode::Space) => {
                return self.activate_control(LobbyControl::RosterTeam)
            }
            LobbyControl::RosterAddPlayer if matches!(key, KeyCode::Enter | KeyCode::Space) => {
                if self.key_pressed.is_none() {
                    let already_down = self.button_is_down(LobbyControl::RosterAddPlayer);
                    self.key_pressed = Some((LobbyControl::RosterAddPlayer, key));
                    if self.pointer_pressed.and_then(HitTarget::button_control)
                        == Some(LobbyControl::RosterAddPlayer)
                        && self.hovered == self.pointer_pressed.unwrap_or(HitTarget::None)
                    {
                        self.pointer_inside_pressed = true;
                    }
                    if !already_down {
                        self.sounds.push(LobbySound::ArrowHit);
                    }
                }
                return Vec::new();
            }
            control @ (LobbyControl::TeamsTab
            | LobbyControl::PlayersTab
            | LobbyControl::ResourcesTab
            | LobbyControl::OptionsTab
            | LobbyControl::ScenarioTab
            | LobbyControl::ChatDialog
            | LobbyControl::Exit
            | LobbyControl::Run
            | LobbyControl::Preload)
                if matches!(key, KeyCode::Enter | KeyCode::Space) =>
            {
                if self.key_pressed.is_none() {
                    let already_down = self.button_is_down(control);
                    self.key_pressed = Some((control, key));
                    if self.pointer_pressed.and_then(HitTarget::button_control) == Some(control)
                        && self.hovered == self.pointer_pressed.unwrap_or(HitTarget::None)
                    {
                        self.pointer_inside_pressed = true;
                    }
                    if !already_down {
                        self.sounds.push(LobbySound::ArrowHit);
                    }
                }
                return Vec::new();
            }
            _ => {}
        }
        if self.focus != LobbyControl::ChatInput {
            self.change_focus(LobbyControl::ChatInput, true);
            return vec![
                LobbyAction::FocusChanged(LobbyControl::ChatInput),
                LobbyAction::Chat(LobbyChatRequest::FocusInput),
            ];
        }
        Vec::new()
    }

    pub fn key_up(&mut self, key: KeyCode) -> Vec<LobbyAction> {
        if matches!(self.focus, LobbyControl::GameOption(_)) {
            return vec![LobbyAction::GameOptions(LobbyGameOptionInput::KeyUp(key))];
        }
        let Some((control, pressed_key)) = self.key_pressed else {
            return Vec::new();
        };
        if pressed_key != key {
            return Vec::new();
        }
        self.key_pressed = None;
        if self.pointer_pressed.and_then(HitTarget::button_control) == Some(control) {
            self.pointer_inside_pressed = false;
        }
        self.sounds.push(LobbySound::Click);
        self.activate_control(control)
    }

    pub fn hotkey(&mut self, hotkey: char, now: Instant) -> Vec<LobbyAction> {
        let hotkey = hotkey.to_ascii_uppercase();
        if hotkey == 'T' {
            let previous = self.focus;
            let changed = self.focus != LobbyControl::ChatInput;
            if !changed {
                return Vec::new();
            }
            self.change_focus(LobbyControl::ChatInput, false);
            let mut actions = vec![
                LobbyAction::FocusChanged(LobbyControl::ChatInput),
                LobbyAction::Chat(LobbyChatRequest::FocusInput),
            ];
            self.append_game_option_focus_clear(previous, &mut actions);
            return actions;
        }
        // Construction/add order: Teams precedes Players and both inherit P.
        if hotkey == 'P' {
            return self.activate_control(if self.has_teams {
                LobbyControl::TeamsTab
            } else {
                LobbyControl::PlayersTab
            });
        }
        if hotkey == 'O' {
            return self.activate_control(LobbyControl::OptionsTab);
        }
        if hotkey == 'X' {
            return vec![LobbyAction::ExitRequested];
        }
        if hotkey == 'S' && self.role == LobbyRole::Host {
            return self.activate_control(LobbyControl::Run);
        }
        if hotkey == 'E' {
            return self.try_toggle_ready(now);
        }
        if matches!(hotkey, 'I' | 'L' | 'M' | 'F' | 'R') {
            return vec![LobbyAction::GameOptions(LobbyGameOptionInput::Hotkey(
                hotkey,
            ))];
        }
        Vec::new()
    }

    pub fn text_input(&mut self, text: impl Into<String>) -> Vec<LobbyAction> {
        let text = text.into();
        if self.focus == LobbyControl::ChatInput {
            vec![LobbyAction::Chat(LobbyChatRequest::InsertText(text))]
        } else if text.starts_with(' ') || text.is_empty() {
            Vec::new()
        } else {
            let previous = self.focus;
            self.change_focus(LobbyControl::ChatInput, true);
            let mut actions = vec![
                LobbyAction::FocusChanged(LobbyControl::ChatInput),
                LobbyAction::Chat(LobbyChatRequest::RefocusAndInsert(text)),
            ];
            self.append_game_option_focus_clear(previous, &mut actions);
            actions
        }
    }

    pub fn focus_chat_input(&mut self) -> Vec<LobbyAction> {
        if self.focus == LobbyControl::ChatInput {
            return Vec::new();
        }
        self.change_focus(LobbyControl::ChatInput, true);
        vec![
            LobbyAction::FocusChanged(LobbyControl::ChatInput),
            LobbyAction::Chat(LobbyChatRequest::FocusInput),
        ]
    }

    pub fn chat_edit_key(
        &self,
        key: LobbyChatEditKey,
        modifiers: LobbyChatKeyModifiers,
    ) -> Vec<LobbyAction> {
        (self.focus == LobbyControl::ChatInput)
            .then_some(LobbyAction::Chat(LobbyChatRequest::EditKey {
                key,
                modifiers,
            }))
            .into_iter()
            .collect()
    }

    pub fn chat_clipboard(&self, shortcut: LobbyChatClipboardShortcut) -> Vec<LobbyAction> {
        (self.focus == LobbyControl::ChatInput)
            .then_some(LobbyAction::Chat(LobbyChatRequest::Clipboard { shortcut }))
            .into_iter()
            .collect()
    }

    pub fn chat_context_command(&self, command: LobbyChatContextCommand) -> Vec<LobbyAction> {
        (self.focus == LobbyControl::ChatInput)
            .then_some(LobbyAction::Chat(LobbyChatRequest::ContextCommand(command)))
            .into_iter()
            .collect()
    }

    pub fn chat_context_from_key(&self, layout: &LobbyLayout) -> Vec<LobbyAction> {
        if self.focus == LobbyControl::ChatInput {
            vec![LobbyAction::Chat(LobbyChatRequest::OpenContextMenu {
                anchor: center(layout.chat_edit),
            })]
        } else {
            Vec::new()
        }
    }

    pub fn gamepad_low_down(
        &mut self,
        now: Instant,
        layout: &LobbyLayout,
        roster: &LobbyRosterLayout,
    ) -> Vec<LobbyAction> {
        if matches!(self.focus, LobbyControl::GameOption(_)) {
            return vec![LobbyAction::GameOptions(
                LobbyGameOptionInput::GamepadLowDown,
            )];
        }
        self.key_down(KeyCode::Space, false, layout, roster, now)
    }

    pub fn gamepad_low_up(&mut self) -> Vec<LobbyAction> {
        if matches!(self.focus, LobbyControl::GameOption(_)) {
            return vec![LobbyAction::GameOptions(LobbyGameOptionInput::GamepadLowUp)];
        }
        self.key_up(KeyCode::Space)
    }

    pub fn gamepad_high_down(&self) -> Vec<LobbyAction> {
        vec![LobbyAction::ExitRequested]
    }

    /// Feeds an internal option-strip focus change back into the enclosing
    /// recursive dialog after the app routes [`LobbyGameOptionInput`].
    pub fn game_option_focus_changed(
        &mut self,
        button: GameOptionButton,
    ) -> Result<Vec<LobbyAction>> {
        ensure!(
            self.role.game_option_context().buttons().contains(&button),
            "{button:?} is not in the {:?} lobby option strip",
            self.role
        );
        let control = LobbyControl::GameOption(button);
        if self.focus == control {
            return Ok(Vec::new());
        }
        self.change_focus(control, true);
        Ok(vec![LobbyAction::FocusChanged(control)])
    }

    /// Completes the option strip's boundary
    /// `FocusTraversalRequested { backwards }` in the enclosing dialog.
    pub fn game_option_focus_traversal_requested(&mut self, backwards: bool) -> Vec<LobbyAction> {
        if matches!(self.focus, LobbyControl::GameOption(_)) {
            self.focus_next(backwards)
        } else {
            Vec::new()
        }
    }

    /// Applies `Dialog::KeyFocusDefault` after an option button reports an
    /// uncaptured key (notably Enter/Space on a disabled option).
    pub fn game_option_input_unhandled(&mut self) -> Vec<LobbyAction> {
        if !matches!(self.focus, LobbyControl::GameOption(_)) {
            return Vec::new();
        }
        self.change_focus(LobbyControl::ChatInput, true);
        vec![
            LobbyAction::FocusChanged(LobbyControl::ChatInput),
            LobbyAction::GameOptions(LobbyGameOptionInput::ClearFocus),
            LobbyAction::Chat(LobbyChatRequest::FocusInput),
        ]
    }

    pub fn gamepad_direction(
        &mut self,
        horizontal: i8,
        vertical: i8,
        layout: &LobbyLayout,
        roster: &LobbyRosterLayout,
    ) -> Vec<LobbyAction> {
        if matches!(self.focus, LobbyControl::GameOption(_)) {
            return vec![LobbyAction::GameOptions(
                LobbyGameOptionInput::GamepadDirection {
                    horizontal,
                    vertical,
                },
            )];
        }
        if self.focus == LobbyControl::OptionsList && vertical != 0 {
            return self.move_option_selection(vertical.signum() as i32, layout, roster);
        }
        if let LobbyControl::Option(option) = self.focus {
            if vertical > 0 {
                let actions = self.option_selection_request(option, roster);
                if !actions.is_empty() {
                    return actions;
                }
                return self.move_option_selection(1, layout, roster);
            }
            if vertical < 0 {
                return self.move_option_selection(-1, layout, roster);
            }
        }
        if self.active_sheet.is_roster() && self.focus == LobbyControl::Roster && vertical != 0 {
            return self.move_selection(vertical.signum() as i32, layout, roster);
        }
        if horizontal != 0 || vertical != 0 {
            self.focus_next(horizontal < 0 || vertical < 0)
        } else {
            Vec::new()
        }
    }

    pub fn request_focused_context(&self, position: GuiPoint) -> Vec<LobbyAction> {
        if !self.active_sheet.is_roster() || self.focus != LobbyControl::Roster {
            return Vec::new();
        }
        self.selected_row
            .and_then(|index| self.rows.get(index))
            .and_then(|row| match row {
                LobbyRosterRow::Client(_) | LobbyRosterRow::Player(_) => {
                    Some(LobbyAction::RosterContextRequested {
                        row: row.id(),
                        position,
                    })
                }
                LobbyRosterRow::Header(_) => None,
            })
            .into_iter()
            .collect()
    }

    /// Applies the synchronized packet and emits all C++-visible effects.
    pub fn apply_countdown_packet(&mut self, packet: LobbyCountdownPacket) -> Vec<LobbyAction> {
        let previous = self.countdown;
        let next = match packet {
            LobbyCountdownPacket::Abort => LobbyCountdownState::None,
            LobbyCountdownPacket::Seconds(0) => LobbyCountdownState::Start,
            LobbyCountdownPacket::Seconds(seconds) if seconds <= 10 => {
                LobbyCountdownState::Final { seconds }
            }
            LobbyCountdownPacket::Seconds(seconds) => LobbyCountdownState::Long { seconds },
        };
        let phase_changed = !next.same_phase(previous);
        if phase_changed {
            if matches!(previous, LobbyCountdownState::Final { .. }) {
                self.sounds.push(LobbySound::StopElevatorLoop);
                if next != LobbyCountdownState::Start {
                    self.sounds.push(LobbySound::Pshshsh);
                }
            }
            if next == LobbyCountdownState::Start {
                self.sounds.push(LobbySound::Blast3);
            } else if matches!(next, LobbyCountdownState::Final { .. }) {
                self.sounds.push(LobbySound::Fuse);
                self.sounds.push(LobbySound::StartElevatorLoop);
            }
        }
        self.countdown = next;
        if next.is_locked() {
            self.open_team_combo_player = None;
        }

        let mut actions = vec![LobbyAction::CountdownChanged(next)];
        if next.is_locked() && self.focus == LobbyControl::RosterTeam {
            self.change_focus(LobbyControl::Roster, true);
            actions.push(LobbyAction::FocusChanged(LobbyControl::Roster));
        }
        if phase_changed
            && matches!(
                next,
                LobbyCountdownState::Long { .. } | LobbyCountdownState::Final { .. }
            )
        {
            actions.push(LobbyAction::NotifyUserIfInactive);
        }
        match packet {
            LobbyCountdownPacket::Abort if previous.is_any() => {
                let line = LobbyLogLine {
                    text: self.labels.start_aborted.clone(),
                    color: make_color_readable_on_black(0x00ff_1f1f),
                };
                self.logs.push(line.clone());
                self.chat_follow_bottom = true;
                actions.push(LobbyAction::AppendLog(line));
            }
            LobbyCountdownPacket::Seconds(seconds) if seconds > 0 => {
                let initial = !previous.is_any();
                let text = if seconds < 10 && !initial {
                    self.labels
                        .countdown_short_template
                        .replace("{seconds}", &seconds.to_string())
                } else {
                    self.labels
                        .countdown_template
                        .replace("{seconds}", &seconds.to_string())
                };
                let line = LobbyLogLine {
                    text,
                    color: make_color_readable_on_black(0x00ff_1f1f),
                };
                self.logs.push(line.clone());
                self.chat_follow_bottom = true;
                self.sounds.push(LobbySound::CountdownCommand);
                actions.push(LobbyAction::AppendLog(line));
            }
            _ => {}
        }
        actions
    }

    fn try_toggle_ready(&mut self, now: Instant) -> Vec<LobbyAction> {
        if !self.resources_loaded {
            return Vec::new();
        }
        // CheckBox plays before the callback; a cooldown rejection restores
        // the prior value but does not retract the click sound.
        self.sounds.push(LobbySound::ArrowHit);
        if self
            .ready_last_change
            .is_some_and(|previous| now.saturating_duration_since(previous) < READY_COOLDOWN)
        {
            return Vec::new();
        }
        self.ready_last_change = Some(now);
        self.ready = !self.ready;
        vec![LobbyAction::ReadyChanged(self.ready)]
    }

    fn focus_order(&self) -> Vec<LobbyControl> {
        let mut order = vec![LobbyControl::ChatInput];
        if self.has_teams {
            order.push(LobbyControl::TeamsTab);
        }
        order.extend([
            LobbyControl::PlayersTab,
            LobbyControl::ResourcesTab,
            LobbyControl::OptionsTab,
            LobbyControl::ScenarioTab,
        ]);
        if self.has_external_chat {
            order.push(LobbyControl::ChatDialog);
        }
        if self.active_sheet.is_roster() {
            order.push(LobbyControl::Roster);
            if let Some(selected) = self.selected_row.and_then(|index| self.rows.get(index)) {
                match selected {
                    LobbyRosterRow::Player(player)
                        if self.has_teams
                            && !self.countdown.is_locked()
                            && player.team.as_ref().is_some_and(|team| team.selectable) =>
                    {
                        order.push(LobbyControl::RosterTeam);
                    }
                    LobbyRosterRow::Client(client) if client.local => {
                        order.push(LobbyControl::RosterAddPlayer);
                    }
                    LobbyRosterRow::Header(header)
                        if header.kind == LobbyRosterHeader::ScriptPlayers
                            && header.can_add_player =>
                    {
                        order.push(LobbyControl::RosterAddPlayer);
                    }
                    _ => {}
                }
            }
        }
        if self.active_sheet == LobbySheet::Options {
            order.push(LobbyControl::OptionsList);
            if let Some(kind) = self
                .selected_option
                .filter(|kind| self.option_rows.iter().any(|row| row.kind == *kind))
            {
                order.push(LobbyControl::Option(kind));
            }
        }
        if self.active_sheet == LobbySheet::Resources && self.preload_button_visible() {
            order.push(LobbyControl::Preload);
        }
        order.push(LobbyControl::Exit);
        order.extend(
            self.role
                .game_option_context()
                .buttons()
                .iter()
                .copied()
                .map(LobbyControl::GameOption),
        );
        if self.role == LobbyRole::Host {
            order.push(LobbyControl::Run);
        }
        if self.resources_loaded {
            order.push(LobbyControl::Ready);
        }
        order
    }

    fn focus_next(&mut self, backwards: bool) -> Vec<LobbyAction> {
        let order = self.focus_order();
        let current = order
            .iter()
            .position(|control| *control == self.focus)
            .unwrap_or(0);
        let next = if backwards {
            (current + order.len() - 1) % order.len()
        } else {
            (current + 1) % order.len()
        };
        let control = order[next];
        let previous = self.focus;
        self.change_focus(control, true);
        let mut selected_on_focus = None;
        if control == LobbyControl::Roster && self.selected_row.is_none() && !self.rows.is_empty() {
            // ListBox::OnGetFocus selects the first item only for keyboard
            // focus traversal, never for mouse focus.
            self.selected_row = Some(0);
            self.selected_roster_id = self.rows.first().map(LobbyRosterRow::id);
            selected_on_focus = self.selected_roster_id.clone();
        }
        if control == LobbyControl::OptionsList
            && self.selected_option.is_none()
            && !self.option_rows.is_empty()
        {
            // ListBox::OnGetFocus selects and scrolls the first item only for
            // keyboard focus. Pointer selection is handled by MouseInput.
            self.selected_option = self.option_rows.first().map(|row| row.kind);
            self.option_scroll = 0;
            self.option_scroll_pin = 0;
        }
        let mut actions = vec![LobbyAction::FocusChanged(control)];
        if let Some(selected) = selected_on_focus {
            actions.push(LobbyAction::RosterSelectionChanged(Some(selected)));
        }
        if matches!(previous, LobbyControl::GameOption(_))
            && !matches!(control, LobbyControl::GameOption(_))
        {
            actions.push(LobbyAction::GameOptions(LobbyGameOptionInput::ClearFocus));
        }
        if let LobbyControl::GameOption(button) = control {
            actions.push(LobbyAction::GameOptions(LobbyGameOptionInput::Focus(
                button,
            )));
        }
        actions
    }

    fn change_focus(&mut self, control: LobbyControl, clear_key: bool) {
        let changed = self.focus != control;
        self.focus = control;
        if clear_key || changed {
            self.key_pressed = None;
        }
    }

    fn append_game_option_focus_clear(
        &self,
        previous: LobbyControl,
        actions: &mut Vec<LobbyAction>,
    ) {
        if matches!(previous, LobbyControl::GameOption(_))
            && !matches!(self.focus, LobbyControl::GameOption(_))
        {
            actions.push(LobbyAction::GameOptions(LobbyGameOptionInput::ClearFocus));
        }
    }

    fn select_option(
        &mut self,
        selected: Option<LobbyOptionKind>,
        by_user: bool,
        layout: &LobbyLayout,
        roster: &LobbyRosterLayout,
    ) {
        if self.active_sheet != LobbySheet::Options || self.selected_option == selected {
            return;
        }
        self.selected_option = selected;
        if by_user && selected.is_some() {
            self.sounds.push(LobbySound::Command);
        }
        let Some(index) =
            selected.and_then(|kind| self.option_rows.iter().position(|row| row.kind == kind))
        else {
            return;
        };
        if let Some(row) = roster.rows.iter().find(|row| row.index == index) {
            let content_top = row.rect.y - layout.roster_client.y + self.option_scroll;
            if content_top < self.option_scroll {
                self.option_scroll = content_top;
            } else if content_top + row.rect.h > self.option_scroll + layout.roster_client.h {
                self.option_scroll = content_top + row.rect.h - layout.roster_client.h;
            }
            self.option_scroll = self.option_scroll.clamp(0, roster.max_scroll);
            self.option_scroll_pin = scroll_to_pin(
                self.option_scroll,
                roster.max_scroll,
                scrollbar_max_pin(layout.roster_scrollbar),
            );
        }
    }

    fn move_option_selection(
        &mut self,
        direction: i32,
        layout: &LobbyLayout,
        roster: &LobbyRosterLayout,
    ) -> Vec<LobbyAction> {
        if self.active_sheet != LobbySheet::Options || self.option_rows.is_empty() {
            return Vec::new();
        }
        let current = self
            .selected_option
            .and_then(|kind| self.option_rows.iter().position(|row| row.kind == kind));
        let next = match current {
            Some(current) if direction < 0 => current.saturating_sub(1),
            Some(current) => (current + 1).min(self.option_rows.len() - 1),
            None if direction < 0 => self.option_rows.len() - 1,
            None => 0,
        };
        let kind = self.option_rows[next].kind;
        self.select_option(Some(kind), true, layout, roster);
        if matches!(self.focus, LobbyControl::Option(_)) {
            self.change_focus(LobbyControl::OptionsList, false);
            vec![LobbyAction::FocusChanged(LobbyControl::OptionsList)]
        } else {
            Vec::new()
        }
    }

    fn move_selection(
        &mut self,
        direction: i32,
        layout: &LobbyLayout,
        roster: &LobbyRosterLayout,
    ) -> Vec<LobbyAction> {
        if !self.active_sheet.is_roster() || self.rows.is_empty() {
            return Vec::new();
        }
        let current = self.selected_row.unwrap_or(if direction < 0 {
            self.rows.len() - 1
        } else {
            0
        });
        let next = if direction < 0 {
            current.saturating_sub(1)
        } else {
            (current + 1).min(self.rows.len() - 1)
        };
        self.select_row(Some(next), true, layout, roster)
    }

    fn edge_selection(
        &mut self,
        end: bool,
        layout: &LobbyLayout,
        roster: &LobbyRosterLayout,
    ) -> Vec<LobbyAction> {
        if !self.active_sheet.is_roster() || self.rows.is_empty() {
            return Vec::new();
        }
        let index = if end { self.rows.len() - 1 } else { 0 };
        self.select_row(Some(index), true, layout, roster)
    }

    fn page_selection(
        &mut self,
        down: bool,
        layout: &LobbyLayout,
        roster: &LobbyRosterLayout,
    ) -> Vec<LobbyAction> {
        if !self.active_sheet.is_roster() || self.rows.is_empty() {
            return Vec::new();
        }

        let old_scroll = self.roster_scroll;
        let viewport_height = layout.roster_client.h.max(0);
        let content_range = |index: usize| {
            roster
                .rows
                .iter()
                .find(|row| row.index == index)
                .map(|row| {
                    let top = row.rect.y - layout.roster_client.y + old_scroll;
                    (top, top + row.rect.h)
                })
        };
        let fully_visible_at = |index: usize, scroll: i32| {
            content_range(index)
                .is_some_and(|(top, bottom)| top >= scroll && bottom <= scroll + viewport_height)
        };

        let current = self
            .selected_row
            .unwrap_or(if down { 0 } else { self.rows.len() - 1 });
        let adjacent = if down {
            current
                .checked_add(1)
                .filter(|index| *index < self.rows.len())
        } else {
            current.checked_sub(1)
        };
        let Some(mut target) = adjacent else {
            return if self.selected_row.is_none() {
                self.select_row(Some(current), true, layout, roster)
            } else {
                Vec::new()
            };
        };

        let mut page_scroll = None;
        if fully_visible_at(target, old_scroll) {
            loop {
                let candidate = if down {
                    target
                        .checked_add(1)
                        .filter(|index| *index < self.rows.len())
                } else {
                    target.checked_sub(1)
                };
                let Some(candidate) =
                    candidate.filter(|index| fully_visible_at(*index, old_scroll))
                else {
                    break;
                };
                target = candidate;
            }
        } else {
            let scroll = if down {
                old_scroll.saturating_add(viewport_height)
            } else {
                old_scroll.saturating_sub(viewport_height)
            }
            .clamp(0, roster.max_scroll);
            page_scroll = Some(scroll);
            target = if down {
                (0..self.rows.len())
                    .rev()
                    .find(|index| fully_visible_at(*index, scroll))
                    .unwrap_or(0)
            } else {
                (0..self.rows.len())
                    .find(|index| fully_visible_at(*index, scroll))
                    .unwrap_or(self.rows.len() - 1)
            };
        }

        let actions = self.select_row(Some(target), true, layout, roster);
        if let Some(scroll) = page_scroll {
            self.roster_scroll = scroll;
            self.roster_scroll_pin = scroll_to_pin(
                scroll,
                roster.max_scroll,
                scrollbar_max_pin(layout.roster_scrollbar),
            );
        }
        actions
    }

    fn select_row(
        &mut self,
        selected: Option<usize>,
        by_user: bool,
        layout: &LobbyLayout,
        roster: &LobbyRosterLayout,
    ) -> Vec<LobbyAction> {
        if !self.active_sheet.is_roster() {
            return Vec::new();
        }
        if self.selected_row == selected {
            return Vec::new();
        }
        self.selected_row = selected;
        self.selected_roster_id = selected
            .and_then(|index| self.rows.get(index))
            .map(LobbyRosterRow::id);
        if by_user && selected.is_some() {
            self.sounds.push(LobbySound::Command);
        }
        if let Some(index) = selected {
            if let Some(row) = roster.rows.iter().find(|row| row.index == index) {
                let content_top = row.rect.y - layout.roster_client.y + self.roster_scroll;
                if content_top < self.roster_scroll {
                    self.roster_scroll = content_top;
                } else if content_top + row.rect.h > self.roster_scroll + layout.roster_client.h {
                    self.roster_scroll = content_top + row.rect.h - layout.roster_client.h;
                }
                self.roster_scroll = self.roster_scroll.clamp(0, roster.max_scroll);
                self.roster_scroll_pin = scroll_to_pin(
                    self.roster_scroll,
                    roster.max_scroll,
                    scrollbar_max_pin(layout.roster_scrollbar),
                );
            }
        }
        let mut actions = vec![LobbyAction::RosterSelectionChanged(
            self.selected_roster_id.clone(),
        )];
        if matches!(
            self.focus,
            LobbyControl::RosterTeam | LobbyControl::RosterAddPlayer
        ) {
            self.focus = LobbyControl::Roster;
            actions.push(LobbyAction::FocusChanged(LobbyControl::Roster));
        }
        actions
    }

    fn activate_hit(&mut self, hit: HitTarget) -> Vec<LobbyAction> {
        match hit {
            HitTarget::Tab(control) => self.activate_control(control),
            HitTarget::Exit => vec![LobbyAction::ExitRequested],
            HitTarget::Run => self.activate_control(LobbyControl::Run),
            HitTarget::Preload => self.activate_control(LobbyControl::Preload),
            HitTarget::AddPlayer(index) => match self.rows.get(index) {
                Some(LobbyRosterRow::Client(client)) if client.local => {
                    vec![LobbyAction::AddPlayerRequested {
                        client_id: client.id,
                    }]
                }
                Some(LobbyRosterRow::Header(header))
                    if header.kind == LobbyRosterHeader::ScriptPlayers && header.can_add_player =>
                {
                    vec![LobbyAction::AddScriptPlayerRequested]
                }
                _ => Vec::new(),
            },
            HitTarget::Team(index) => match self.rows.get(index) {
                Some(LobbyRosterRow::Player(player))
                    if player.team.as_ref().is_some_and(|team| team.selectable)
                        && !self.countdown.is_locked() =>
                {
                    vec![LobbyAction::TeamSelectionRequested {
                        player_id: player.id,
                    }]
                }
                _ => Vec::new(),
            },
            HitTarget::ResourceSave(resource_id) => self
                .resource_rows
                .iter()
                .find(|row| row.id == resource_id && row.save_possible)
                .map(|_| LobbyAction::SaveResourceRequested { resource_id })
                .into_iter()
                .collect(),
            _ => Vec::new(),
        }
    }

    fn option_selection_request_by_index(
        &self,
        index: usize,
        layout: &LobbyRosterLayout,
    ) -> Vec<LobbyAction> {
        let Some(row) = self.option_rows.get(index) else {
            return Vec::new();
        };
        self.option_selection_request(row.kind, layout)
    }

    fn option_selection_request(
        &self,
        option: LobbyOptionKind,
        layout: &LobbyRosterLayout,
    ) -> Vec<LobbyAction> {
        if self.active_sheet != LobbySheet::Options {
            return Vec::new();
        }
        let Some((index, row)) = self
            .option_rows
            .iter()
            .enumerate()
            .find(|(_, row)| row.kind == option && row.editable && !row.choices.is_empty())
        else {
            return Vec::new();
        };
        let Some(combo) = layout
            .rows
            .iter()
            .find(|row| row.index == index)
            .and_then(|row| row.option_value)
        else {
            return Vec::new();
        };
        vec![LobbyAction::OptionSelectionRequested {
            option: row.kind,
            anchor: GuiPoint::new(combo.x as f32, (combo.y + combo.h) as f32),
            minimum_width: combo.w,
        }]
    }

    fn activate_control(&mut self, control: LobbyControl) -> Vec<LobbyAction> {
        match control {
            LobbyControl::TeamsTab => {
                self.sounds.push(LobbySound::Command);
                vec![LobbyAction::SheetRequested(LobbySheet::Teams)]
            }
            LobbyControl::PlayersTab => {
                self.sounds.push(LobbySound::Command);
                vec![LobbyAction::SheetRequested(LobbySheet::Players)]
            }
            LobbyControl::ResourcesTab => {
                self.sounds.push(LobbySound::Command);
                vec![LobbyAction::SheetRequested(LobbySheet::Resources)]
            }
            LobbyControl::OptionsTab => {
                self.sounds.push(LobbySound::Command);
                vec![LobbyAction::SheetRequested(LobbySheet::Options)]
            }
            LobbyControl::ScenarioTab => {
                self.sounds.push(LobbySound::Command);
                vec![LobbyAction::SheetRequested(LobbySheet::Scenario)]
            }
            LobbyControl::ChatDialog => {
                vec![LobbyAction::Chat(LobbyChatRequest::OpenExternalDialog)]
            }
            LobbyControl::Exit => vec![LobbyAction::ExitRequested],
            LobbyControl::Preload
                if self.active_sheet == LobbySheet::Resources && self.preload_button_visible() =>
            {
                vec![LobbyAction::PreloadRequested]
            }
            LobbyControl::RosterTeam if self.active_sheet.is_roster() => self
                .selected_row
                .and_then(|index| self.rows.get(index))
                .and_then(|row| match row {
                    LobbyRosterRow::Player(player)
                        if player.team.as_ref().is_some_and(|team| team.selectable)
                            && !self.countdown.is_locked() =>
                    {
                        Some(LobbyAction::TeamSelectionRequested {
                            player_id: player.id,
                        })
                    }
                    _ => None,
                })
                .into_iter()
                .collect(),
            LobbyControl::RosterAddPlayer if self.active_sheet.is_roster() => self
                .selected_row
                .and_then(|index| self.rows.get(index))
                .and_then(|row| match row {
                    LobbyRosterRow::Client(client) if client.local => {
                        Some(LobbyAction::AddPlayerRequested {
                            client_id: client.id,
                        })
                    }
                    LobbyRosterRow::Header(header)
                        if header.kind == LobbyRosterHeader::ScriptPlayers
                            && header.can_add_player =>
                    {
                        Some(LobbyAction::AddScriptPlayerRequested)
                    }
                    _ => None,
                })
                .into_iter()
                .collect(),
            LobbyControl::Run if self.role == LobbyRole::Host => {
                if self.countdown.is_any() {
                    vec![LobbyAction::AbortCountdownRequested]
                } else {
                    vec![LobbyAction::StartRequested {
                        countdown_seconds: self.validated_countdown_seconds(),
                        check_league_rules: true,
                        confirm_unassociated_savegame_players: self.rows.iter().any(|row| {
                            matches!(
                                row,
                                LobbyRosterRow::Header(LobbyHeaderRow {
                                    kind: LobbyRosterHeader::UnassignedSavegamePlayers,
                                    ..
                                })
                            )
                        }),
                    }]
                }
            }
            _ => Vec::new(),
        }
    }

    fn validated_countdown_seconds(&self) -> i32 {
        let seconds = if self.configured_countdown_seconds < 0 {
            5
        } else {
            self.configured_countdown_seconds
        };
        if self.league_mode {
            seconds.max(5)
        } else {
            seconds
        }
    }

    fn set_scroll_from_pointer(
        &mut self,
        point: GuiPoint,
        layout: &LobbyLayout,
        roster: &LobbyRosterLayout,
    ) {
        let max_pin = scrollbar_max_pin(layout.roster_scrollbar);
        let pin =
            (point.y as i32 - layout.roster_scrollbar.y - SCROLLBAR_EXTENT - SCROLLBAR_EXTENT / 2)
                .clamp(0, max_pin);
        let scroll = if max_pin == 0 {
            0
        } else {
            roster.max_scroll * pin / max_pin
        };
        if self.active_sheet == LobbySheet::Resources {
            self.resource_scroll_pin = pin;
            self.resource_scroll = scroll;
        } else if self.active_sheet == LobbySheet::Scenario {
            self.scenario_scroll_pin = pin;
            self.scenario_scroll = scroll;
        } else if self.active_sheet == LobbySheet::Options {
            self.option_scroll_pin = pin;
            self.option_scroll = scroll;
        } else if self.active_sheet.is_roster() {
            self.roster_scroll_pin = pin;
            self.roster_scroll = scroll;
        }
    }

    fn set_chat_scroll_from_pointer(&mut self, point: GuiPoint, layout: &LobbyLayout) {
        let max_pin = scrollbar_max_pin(layout.chat_log_scrollbar);
        let pin = (point.y as i32
            - layout.chat_log_scrollbar.y
            - SCROLLBAR_EXTENT
            - SCROLLBAR_EXTENT / 2)
            .clamp(0, max_pin);
        self.chat_scroll_pin = pin;
        self.chat_scroll = if max_pin == 0 {
            0
        } else {
            self.chat_max_scroll * pin / max_pin
        };
    }

    fn advance_held_scrollbars(&mut self, layout: &LobbyLayout, roster_max_scroll: i32) {
        match self.pointer_pressed {
            Some(HitTarget::ChatScrollTop) => {
                self.chat_scroll_pin = (self.chat_scroll_pin - 1).max(0);
                self.chat_scroll = pin_to_scroll(
                    self.chat_scroll_pin,
                    self.chat_max_scroll,
                    scrollbar_max_pin(layout.chat_log_scrollbar),
                );
            }
            Some(HitTarget::ChatScrollBottom) => {
                let max_pin = scrollbar_max_pin(layout.chat_log_scrollbar);
                self.chat_scroll_pin = (self.chat_scroll_pin + 1).min(max_pin);
                self.chat_scroll =
                    pin_to_scroll(self.chat_scroll_pin, self.chat_max_scroll, max_pin);
            }
            Some(HitTarget::RosterScrollTop) => {
                let max_pin = scrollbar_max_pin(layout.roster_scrollbar);
                if self.active_sheet == LobbySheet::Resources {
                    self.resource_scroll_pin = (self.resource_scroll_pin - 1).max(0);
                    self.resource_scroll =
                        pin_to_scroll(self.resource_scroll_pin, roster_max_scroll, max_pin);
                } else if self.active_sheet == LobbySheet::Scenario {
                    self.scenario_scroll_pin = (self.scenario_scroll_pin - 1).max(0);
                    self.scenario_scroll =
                        pin_to_scroll(self.scenario_scroll_pin, roster_max_scroll, max_pin);
                } else if self.active_sheet == LobbySheet::Options {
                    self.option_scroll_pin = (self.option_scroll_pin - 1).max(0);
                    self.option_scroll =
                        pin_to_scroll(self.option_scroll_pin, roster_max_scroll, max_pin);
                } else if self.active_sheet.is_roster() {
                    self.roster_scroll_pin = (self.roster_scroll_pin - 1).max(0);
                    self.roster_scroll =
                        pin_to_scroll(self.roster_scroll_pin, roster_max_scroll, max_pin);
                }
            }
            Some(HitTarget::RosterScrollBottom) => {
                let max_pin = scrollbar_max_pin(layout.roster_scrollbar);
                if self.active_sheet == LobbySheet::Resources {
                    self.resource_scroll_pin = (self.resource_scroll_pin + 1).min(max_pin);
                    self.resource_scroll =
                        pin_to_scroll(self.resource_scroll_pin, roster_max_scroll, max_pin);
                } else if self.active_sheet == LobbySheet::Scenario {
                    self.scenario_scroll_pin = (self.scenario_scroll_pin + 1).min(max_pin);
                    self.scenario_scroll =
                        pin_to_scroll(self.scenario_scroll_pin, roster_max_scroll, max_pin);
                } else if self.active_sheet == LobbySheet::Options {
                    self.option_scroll_pin = (self.option_scroll_pin + 1).min(max_pin);
                    self.option_scroll =
                        pin_to_scroll(self.option_scroll_pin, roster_max_scroll, max_pin);
                } else if self.active_sheet.is_roster() {
                    self.roster_scroll_pin = (self.roster_scroll_pin + 1).min(max_pin);
                    self.roster_scroll =
                        pin_to_scroll(self.roster_scroll_pin, roster_max_scroll, max_pin);
                }
            }
            _ => {}
        }
    }

    const fn right_list_scroll_pin(&self) -> i32 {
        match self.active_sheet {
            LobbySheet::Resources => self.resource_scroll_pin,
            LobbySheet::Options => self.option_scroll_pin,
            LobbySheet::Players | LobbySheet::Teams => self.roster_scroll_pin,
            LobbySheet::Scenario => self.scenario_scroll_pin,
        }
    }

    fn button_is_down(&self, control: LobbyControl) -> bool {
        self.pointer_inside_pressed
            && self.pointer_pressed.and_then(HitTarget::button_control) == Some(control)
            || self
                .key_pressed
                .is_some_and(|(pressed_control, _)| pressed_control == control)
    }

    fn hit_test(
        &self,
        point: GuiPoint,
        layout: &LobbyLayout,
        roster: &LobbyRosterLayout,
    ) -> HitTarget {
        for tab in &layout.tab_buttons {
            if contains(tab.rect, point) {
                return HitTarget::Tab(tab.control);
            }
        }
        if contains(layout.chat_log_scrollbar, point) && self.chat_max_scroll > 0 {
            if point.y < (layout.chat_log_scrollbar.y + SCROLLBAR_EXTENT) as f32 {
                return HitTarget::ChatScrollTop;
            }
            if point.y
                >= (layout.chat_log_scrollbar.y + layout.chat_log_scrollbar.h - SCROLLBAR_EXTENT)
                    as f32
            {
                return HitTarget::ChatScrollBottom;
            }
            return HitTarget::ChatScrollTrack;
        }
        if contains(layout.chat_log_scrollbar, point) {
            return HitTarget::ChatScrollInert;
        }
        if self.active_sheet.is_roster() {
            for row in &roster.rows {
                if contains(layout.roster_client, point)
                    && row.add_player.is_some_and(|rect| contains(rect, point))
                {
                    return HitTarget::AddPlayer(row.index);
                }
                if contains(layout.roster_client, point)
                    && row.team.is_some_and(|rect| contains(rect, point))
                {
                    return HitTarget::Team(row.index);
                }
                if contains(layout.roster_client, point) && contains(row.rect, point) {
                    return HitTarget::RosterRow(row.index);
                }
            }
        }
        if self.active_sheet == LobbySheet::Options {
            for row_layout in &roster.rows {
                if !contains(layout.roster_client, point) {
                    break;
                }
                let Some(row) = self.option_rows.get(row_layout.index) else {
                    continue;
                };
                if row.editable
                    && row_layout
                        .option_value
                        .is_some_and(|rect| contains(rect, point))
                {
                    return HitTarget::OptionValue(row_layout.index);
                }
                if contains(row_layout.rect, point) {
                    return HitTarget::OptionRow(row_layout.index);
                }
            }
        }
        if self.active_sheet == LobbySheet::Resources && contains(layout.roster_client, point) {
            for row_layout in &roster.rows {
                let Some(row) = self.resource_rows.get(row_layout.index) else {
                    continue;
                };
                if row_layout.save.is_some_and(|rect| contains(rect, point)) {
                    return HitTarget::ResourceSave(row.id);
                }
            }
        }
        if contains(layout.roster_client, point) {
            return if self.active_sheet.is_roster() {
                HitTarget::RosterBlank
            } else {
                HitTarget::RightListInert
            };
        }
        if contains(layout.roster_scrollbar, point) && roster.max_scroll > 0 {
            if point.y < (layout.roster_scrollbar.y + SCROLLBAR_EXTENT) as f32 {
                return HitTarget::RosterScrollTop;
            }
            if point.y
                >= (layout.roster_scrollbar.y + layout.roster_scrollbar.h - SCROLLBAR_EXTENT) as f32
            {
                return HitTarget::RosterScrollBottom;
            }
            return HitTarget::RosterScrollTrack;
        }
        if contains(layout.roster_scrollbar, point) {
            return HitTarget::RosterScrollInert;
        }
        if layout
            .preload_button
            .is_some_and(|rect| contains(rect, point))
        {
            return HitTarget::Preload;
        }
        if contains(layout.ready_square, point) {
            return HitTarget::Ready;
        }
        if layout.run_button.is_some_and(|rect| contains(rect, point)) {
            return HitTarget::Run;
        }
        if contains(layout.exit_button, point) {
            return HitTarget::Exit;
        }
        for option in
            game_option_buttons_layout(layout.game_option_strip, self.role.game_option_context())
                .buttons
        {
            if contains(option.rect, point) {
                return HitTarget::GameOption(option.button);
            }
        }
        if contains(layout.chat_edit, point) {
            return HitTarget::ChatInput;
        }
        if contains(layout.chat_label, point) {
            return HitTarget::ChatLabel;
        }
        if contains(layout.right_caption, point) {
            return HitTarget::RightCaption;
        }
        HitTarget::None
    }

    pub fn tooltip_state_at(&self, now: Instant) -> Option<LobbyTooltip> {
        if !self.tooltip_pointer_active {
            return None;
        }
        if now
            .checked_duration_since(self.hover_since)
            .unwrap_or_default()
            < TOOLTIP_DELAY
        {
            return None;
        }
        let text = self.tooltip_text()?;
        let pointer = self.pointer?;
        Some(LobbyTooltip { pointer, text })
    }

    fn tooltip_text(&self) -> Option<String> {
        let text = match self.hovered {
            HitTarget::ChatInput | HitTarget::ChatLabel => self.labels.tooltip_chat.clone(),
            HitTarget::Exit => self.labels.tooltip_exit.clone(),
            HitTarget::Run => self.labels.tooltip_start.clone(),
            HitTarget::Ready if self.resources_loaded => self.labels.tooltip_ready.clone(),
            HitTarget::Ready => self.labels.tooltip_ready_unavailable.clone(),
            HitTarget::Preload => self.labels.tooltip_preload.clone(),
            HitTarget::RosterRow(index) | HitTarget::AddPlayer(index) | HitTarget::Team(index) => {
                match self.rows.get(index) {
                    Some(LobbyRosterRow::Client(client)) => {
                        format!(
                            "Client {} ({})",
                            crate::c4_presentation_text(&client.name),
                            crate::c4_presentation_text(&client.nick)
                        )
                    }
                    Some(LobbyRosterRow::Header(header)) => match header.kind {
                        LobbyRosterHeader::UnassignedSavegamePlayers => {
                            self.labels.tooltip_unassigned_savegame_players.clone()
                        }
                        LobbyRosterHeader::ScriptPlayers => {
                            self.labels.tooltip_script_players.clone()
                        }
                        LobbyRosterHeader::ReplayPlayers => {
                            self.labels.tooltip_replay_players.clone()
                        }
                        LobbyRosterHeader::Team(_) | LobbyRosterHeader::RandomTeam => self
                            .labels
                            .tooltip_team_template
                            .replace("{team}", &crate::c4_presentation_text(&header.label)),
                    },
                    _ => return None,
                }
            }
            HitTarget::OptionRow(index) | HitTarget::OptionValue(index) => {
                self.option_rows.get(index).map(|row| row.tooltip.clone())?
            }
            _ => return None,
        };
        (!text.is_empty()).then_some(text)
    }

    pub fn tooltip_state_with_roster_at(
        &self,
        now: Instant,
        roster: &LobbyRosterLayout,
        font: &clonk_graphics::clonk_font::ClonkFont,
    ) -> Option<LobbyTooltip> {
        if !self.tooltip_pointer_active {
            return None;
        }
        if now
            .checked_duration_since(self.hover_since)
            .unwrap_or_default()
            < TOOLTIP_DELAY
        {
            return None;
        }
        let pointer = self.pointer?;
        for row_layout in &roster.rows {
            if self.hovered != HitTarget::RosterRow(row_layout.index) {
                continue;
            }
            let Some(LobbyRosterRow::Client(client)) = self.rows.get(row_layout.index) else {
                continue;
            };
            let Some(ping) = client.ping_ms else {
                continue;
            };
            let text = format!("{ping} ms");
            let (width, height) = font.measure(&text, true);
            let ping_rect = IntRect::new(
                row_layout.rect.x + row_layout.rect.w - width,
                row_layout.rect.y,
                width,
                height,
            );
            if contains(ping_rect, pointer) {
                return Some(LobbyTooltip {
                    pointer,
                    text: self.labels.tooltip_ping.clone(),
                });
            }
        }
        self.tooltip_state_at(now)
    }

    #[allow(clippy::too_many_arguments)]
    pub fn render(
        &mut self,
        surface: &mut Surface,
        resources: &LobbyResources<'_>,
        option_buttons: &GameOptionButtons,
        option_resources: &GameOptionButtonResources<'_>,
        active: bool,
        gamma: Option<&GammaRamp>,
    ) -> Result<()> {
        self.render_without_tooltips(
            surface,
            resources,
            option_buttons,
            option_resources,
            active,
            gamma,
        )?;
        self.render_tooltips(
            surface,
            resources,
            option_buttons,
            option_resources,
            active,
            gamma,
        )
    }

    /// Draw the complete lobby except its final option/lobby tooltip pass.
    /// Ordered native-text presentation commits this phase before drawing a
    /// tooltip so the tooltip chrome can occlude all earlier text like C++.
    #[allow(clippy::too_many_arguments)]
    pub fn render_without_tooltips(
        &mut self,
        surface: &mut Surface,
        resources: &LobbyResources<'_>,
        option_buttons: &GameOptionButtons,
        option_resources: &GameOptionButtonResources<'_>,
        active: bool,
        gamma: Option<&GammaRamp>,
    ) -> Result<()> {
        let now = Instant::now();
        self.client_sound_status.retain(|_, (_, started)| {
            now.checked_duration_since(*started)
                .is_none_or(|elapsed| elapsed < SOUND_ICON_SHOW_TIME)
        });
        resources.validate()?;
        let layout = self.layout(
            i32::try_from(surface.width()).unwrap_or(i32::MAX),
            i32::try_from(surface.height()).unwrap_or(i32::MAX),
            resources.fonts,
        );
        self.league_mode = option_buttons.values().lobby_is_league;
        let roster = self.right_list_layout(&layout, resources.fonts);
        let _ = self.chat_scroll_metrics(&layout, &resources.fonts.text);
        self.advance_held_scrollbars(&layout, roster.max_scroll);
        let roster = self.right_list_layout(&layout, resources.fonts);
        let chat_metrics = self.chat_scroll_metrics(&layout, &resources.fonts.text);
        ensure!(
            option_buttons.context() == self.role.game_option_context(),
            "lobby game-option context does not match {:?}",
            self.role
        );
        ensure!(
            option_buttons.layout().bounds == layout.game_option_strip,
            "lobby game-option strip must use exact bounds {:?}; got {:?}",
            layout.game_option_strip,
            option_buttons.layout().bounds
        );
        ensure!(
            option_buttons.values().countdown == self.countdown.is_locked(),
            "lobby game-option countdown lock must match the lobby countdown phase"
        );
        let expected_option_focus = match self.focus {
            LobbyControl::GameOption(button) => Some(button),
            _ => None,
        };
        ensure!(
            option_buttons.focused_button() == expected_option_focus,
            "lobby game-option recursive focus must match the enclosing lobby"
        );

        let skin = resources.skin();
        // FullscreenDialog has no background and no generic dialog pane.
        resources.fonts.title.draw_with_gamma(
            surface,
            layout.title_anchor.0,
            layout.title_anchor.1,
            &self.title(),
            COLOR_YELLOW,
            TextAlign::Center,
            true,
            gamma,
        );

        draw_text_window(surface, layout.chat_log, gamma);
        draw_scrollbar(
            surface,
            layout.chat_log_scrollbar,
            resources.scroll,
            self.chat_scroll_pin,
            chat_metrics.max_scroll,
            self.pointer_pressed == Some(HitTarget::ChatScrollTop),
            self.pointer_pressed == Some(HitTarget::ChatScrollBottom),
            gamma,
        );
        self.draw_logs(surface, &layout, resources, gamma);
        let (chat_label, _) = expand_hotkey_markup(&self.labels.chat);
        skin.draw_caption(
            surface,
            layout.chat_label,
            &chat_label,
            &resources.fonts.text,
            COLOR_WHITE,
            TextAlign::Center,
            gamma,
        );
        draw_edit(
            surface,
            layout.chat_edit,
            &self.chat_edit,
            &resources.fonts.text,
            active && self.focus == LobbyControl::ChatInput,
            gamma,
        );

        let (right_title, _) = expand_hotkey_markup(&self.right_title());
        skin.draw_caption(
            surface,
            layout.right_caption,
            &right_title,
            &resources.fonts.text,
            COLOR_WHITE,
            TextAlign::Left,
            gamma,
        );
        for tab in &layout.tab_buttons {
            self.draw_tab_button(surface, *tab, resources, active, gamma);
        }

        draw_3d_frame(surface, layout.right_tab, gamma);
        draw_engine_box(
            surface,
            layout.right_tab.x,
            layout.right_tab.y,
            layout.right_tab.x + layout.right_tab.w - 1,
            layout.right_tab.y + layout.right_tab.h - 1,
            STANDARD_BACKGROUND_COLOR,
            gamma,
        );
        if self.active_sheet != LobbySheet::Scenario {
            draw_engine_box(
                surface,
                layout.roster.x,
                layout.roster.y,
                layout.roster.x + layout.roster.w - 1,
                layout.roster.y + layout.roster.h - 1,
                DARK_BACKGROUND,
                gamma,
            );
        }
        match self.active_sheet {
            LobbySheet::Players | LobbySheet::Teams => {
                self.draw_roster(surface, &layout, &roster, resources, active, gamma)?
            }
            LobbySheet::Resources => {
                self.draw_resource_rows(surface, &layout, &roster, resources, gamma)
            }
            LobbySheet::Options => {
                self.draw_option_rows(surface, &layout, &roster, resources, active, gamma)
            }
            LobbySheet::Scenario => self.draw_scenario_text(surface, &layout, resources, gamma),
        }
        if self.active_sheet != LobbySheet::Scenario || roster.max_scroll > 0 {
            draw_scrollbar(
                surface,
                layout.roster_scrollbar,
                resources.scroll,
                self.right_list_scroll_pin(),
                roster.max_scroll,
                self.pointer_pressed == Some(HitTarget::RosterScrollTop),
                self.pointer_pressed == Some(HitTarget::RosterScrollBottom),
                gamma,
            );
        }

        if let Some(preload) = layout.preload_button {
            skin.draw_button(
                surface,
                preload,
                &self.labels.preload,
                resources.fonts,
                self.button_state(LobbyControl::Preload, active),
                gamma,
            );
        }

        skin.draw_button(
            surface,
            layout.exit_button,
            &self.labels.exit,
            resources.fonts,
            self.button_state(LobbyControl::Exit, active),
            gamma,
        );
        option_buttons.render(surface, option_resources, active, gamma)?;
        if let Some(run) = layout.run_button {
            skin.draw_button(
                surface,
                run,
                if self.countdown.is_any() {
                    &self.labels.cancel
                } else {
                    &self.labels.start
                },
                resources.fonts,
                self.button_state(LobbyControl::Run, active),
                gamma,
            );
        }
        self.draw_ready(surface, &layout, resources, active, gamma);
        Ok(())
    }

    /// Draw the final tooltip pass after [`Self::render_without_tooltips`].
    #[allow(clippy::too_many_arguments)]
    pub fn render_tooltips(
        &mut self,
        surface: &mut Surface,
        resources: &LobbyResources<'_>,
        option_buttons: &GameOptionButtons,
        option_resources: &GameOptionButtonResources<'_>,
        active: bool,
        gamma: Option<&GammaRamp>,
    ) -> Result<()> {
        resources.validate()?;
        let layout = self.layout(
            i32::try_from(surface.width()).unwrap_or(i32::MAX),
            i32::try_from(surface.height()).unwrap_or(i32::MAX),
            resources.fonts,
        );
        ensure!(
            option_buttons.context() == self.role.game_option_context(),
            "lobby game-option context does not match {:?}",
            self.role
        );
        ensure!(
            option_buttons.layout().bounds == layout.game_option_strip,
            "lobby game-option strip must use exact bounds {:?}; got {:?}",
            layout.game_option_strip,
            option_buttons.layout().bounds
        );
        option_buttons.render_tooltip(surface, option_resources, active, gamma)?;
        if active {
            let roster = self.right_list_layout(&layout, resources.fonts);
            if let Some(tooltip) =
                self.tooltip_state_with_roster_at(Instant::now(), &roster, &resources.fonts.text)
            {
                draw_classic_tooltip(
                    surface,
                    resources.tooltip_font,
                    tooltip.pointer,
                    &tooltip.text,
                    gamma,
                );
            }
        }
        Ok(())
    }

    fn draw_logs(
        &self,
        surface: &mut Surface,
        layout: &LobbyLayout,
        resources: &LobbyResources<'_>,
        gamma: Option<&GammaRamp>,
    ) {
        let lines = self.wrapped_chat_lines(layout, &resources.fonts.text);
        let mut y = layout.chat_log_client.y - self.chat_scroll;
        for (line_index, line) in lines.iter().enumerate() {
            if line_index > 0 && line.new_paragraph {
                y += resources.fonts.text.line_height / 3;
            }
            draw_clipped_text(
                surface,
                &resources.fonts.text,
                layout.chat_log_client.x,
                y,
                &line.text,
                line.color,
                TextAlign::Left,
                gamma,
                layout.chat_log_client,
            );
            y += resources.fonts.text.line_height;
        }
    }

    fn draw_scenario_text(
        &self,
        surface: &mut Surface,
        layout: &LobbyLayout,
        resources: &LobbyResources<'_>,
        gamma: Option<&GammaRamp>,
    ) {
        let lines = self.wrapped_scenario_lines(layout, resources.fonts);
        let mut y = layout.roster_client.y - self.scenario_scroll;
        for (line_index, line) in lines.iter().enumerate() {
            let font = if line.title_font {
                &resources.fonts.caption
            } else {
                &resources.fonts.text
            };
            if line_index > 0 && line.new_paragraph {
                y += font.line_height / 3;
            }
            draw_clipped_text_mode(
                surface,
                font,
                layout.roster_client.x,
                y,
                &line.text,
                COLOR_WHITE,
                TextAlign::Left,
                gamma,
                layout.roster_client,
                false,
            );
            y += font.line_height;
        }
    }

    fn draw_tab_button(
        &self,
        surface: &mut Surface,
        tab: LobbyTabButtonLayout,
        resources: &LobbyResources<'_>,
        active: bool,
        gamma: Option<&GammaRamp>,
    ) {
        let highlighted = tab.selected
            || active && (self.focus == tab.control || self.hovered == HitTarget::Tab(tab.control));
        let pressed = self.pointer_pressed == Some(HitTarget::Tab(tab.control))
            && self.pointer_inside_pressed
            || self
                .key_pressed
                .is_some_and(|(control, _)| control == tab.control);
        if highlighted {
            draw_highlight(surface, tab.rect, &resources.button_highlight, gamma);
        }
        match tab.icon {
            LobbyTabIcon::Standard(phase) => {
                draw_standard_icon(surface, tab.rect, &resources.icons, phase, gamma)
            }
            LobbyTabIcon::Extended(phase) => {
                draw_extended_icon(surface, tab.rect, &resources.icons_extended, phase, gamma)
            }
        }
        if tab.selected || active && pressed {
            draw_highlight(surface, tab.rect, &resources.button_highlight, gamma);
        }
    }

    fn draw_roster(
        &self,
        surface: &mut Surface,
        layout: &LobbyLayout,
        roster: &LobbyRosterLayout,
        resources: &LobbyResources<'_>,
        active: bool,
        gamma: Option<&GammaRamp>,
    ) -> Result<()> {
        for row_layout in &roster.rows {
            if !intersects(row_layout.rect, layout.roster_client) {
                continue;
            }
            let Some(row) = self.rows.get(row_layout.index) else {
                continue;
            };
            if self.selected_row == Some(row_layout.index) {
                let selected = intersection(row_layout.rect, layout.roster_client);
                draw_engine_box(
                    surface,
                    selected.x,
                    selected.y,
                    selected.x + selected.w - 1,
                    selected.y + selected.h - 1,
                    if active && self.focus == LobbyControl::Roster {
                        LIST_SELECTION
                    } else {
                        LIST_INACTIVE_SELECTION
                    },
                    gamma,
                );
            }
            if row_layout.index != 0 && row.has_spacing_bar() {
                let y = row_layout.rect.y - row.top_spacing() / 2;
                if y >= layout.roster_client.y
                    && y < layout.roster_client.y + layout.roster_client.h
                {
                    draw_engine_line(
                        surface,
                        row_layout.rect.x + 10,
                        y,
                        row_layout.rect.x + layout.roster_client.w - 10,
                        y,
                        LIST_SEPARATOR,
                        gamma,
                    );
                }
            }
            match row {
                LobbyRosterRow::Client(client) => self.draw_client_row(
                    surface,
                    client,
                    *row_layout,
                    layout,
                    resources,
                    active,
                    gamma,
                ),
                LobbyRosterRow::Player(player) => self.draw_player_row(
                    surface,
                    player,
                    *row_layout,
                    layout,
                    resources,
                    active,
                    gamma,
                )?,
                LobbyRosterRow::Header(header) => self.draw_header_row(
                    surface,
                    header,
                    *row_layout,
                    layout,
                    resources,
                    active,
                    gamma,
                )?,
            }
        }
        Ok(())
    }

    fn draw_resource_rows(
        &self,
        surface: &mut Surface,
        layout: &LobbyLayout,
        resource_layout: &LobbyRosterLayout,
        resources: &LobbyResources<'_>,
        gamma: Option<&GammaRamp>,
    ) {
        for row_layout in &resource_layout.rows {
            if !intersects(row_layout.rect, layout.roster_client) {
                continue;
            }
            let Some(row) = self.resource_rows.get(row_layout.index) else {
                continue;
            };
            draw_standard_icon_clipped(
                surface,
                row_layout.icon,
                layout.roster_client,
                &resources.icons,
                10,
                gamma,
            );
            draw_clipped_text(
                surface,
                &resources.fonts.text,
                row_layout.icon.x + row_layout.icon.w + ICON_LABEL_SPACING,
                row_layout.rect.y + 2,
                &crate::c4_presentation_text(row.basename()),
                COLOR_WHITE,
                TextAlign::Left,
                gamma,
                layout.roster_client,
            );
            if let Some(progress) = row.progress_label() {
                draw_clipped_text(
                    surface,
                    &resources.fonts.text,
                    row_layout.rect.x + row_layout.rect.w - ICON_LABEL_SPACING,
                    row_layout.rect.y + 2,
                    &progress,
                    COLOR_WHITE,
                    TextAlign::Right,
                    gamma,
                    layout.roster_client,
                );
            }
            if let Some(save) = row_layout.save {
                let target = HitTarget::ResourceSave(row.id);
                if self.hovered == target {
                    draw_highlight_clipped(
                        surface,
                        save,
                        layout.roster_client,
                        &resources.button_highlight,
                        gamma,
                    );
                }
                draw_standard_icon_clipped(
                    surface,
                    save,
                    layout.roster_client,
                    &resources.icons,
                    13,
                    gamma,
                );
                if self.pointer_pressed == Some(target) && self.pointer_inside_pressed {
                    draw_highlight_clipped(
                        surface,
                        save,
                        layout.roster_client,
                        &resources.button_highlight,
                        gamma,
                    );
                }
            }
        }
    }

    fn draw_option_rows(
        &self,
        surface: &mut Surface,
        layout: &LobbyLayout,
        option_layout: &LobbyRosterLayout,
        resources: &LobbyResources<'_>,
        active: bool,
        gamma: Option<&GammaRamp>,
    ) {
        for row_layout in &option_layout.rows {
            if !intersects(row_layout.rect, layout.roster_client) {
                continue;
            }
            let Some(row) = self.option_rows.get(row_layout.index) else {
                continue;
            };
            if self.selected_option == Some(row.kind) {
                let selected = intersection(row_layout.rect, layout.roster_client);
                draw_engine_box(
                    surface,
                    selected.x,
                    selected.y,
                    selected.x + selected.w - 1,
                    selected.y + selected.h - 1,
                    if active && self.focus == LobbyControl::OptionsList {
                        LIST_SELECTION
                    } else {
                        LIST_INACTIVE_SELECTION
                    },
                    gamma,
                );
            }
            let caption = format!("{}:", crate::c4_presentation_text(&row.caption));
            draw_clipped_text(
                surface,
                &resources.fonts.text,
                row_layout.rect.x + 1,
                row_layout.rect.y + 1,
                &caption,
                COLOR_WHITE,
                TextAlign::Left,
                gamma,
                layout.roster_client,
            );
            let Some(combo) = row_layout.option_value else {
                continue;
            };
            let arrow_x = combo.x + combo.w - CONTEXT_HEIGHT as i32 - 1;
            let open = self.open_option_combo == Some(row.kind);
            if row.editable {
                with_surface_clip(surface, layout.roster_client, |surface| {
                    draw_engine_box(
                        surface,
                        combo.x,
                        combo.y,
                        combo.x + combo.w - 1,
                        combo.y + combo.h - 1,
                        STANDARD_BACKGROUND_COLOR,
                        gamma,
                    );
                    draw_3d_frame(surface, combo, gamma);
                });
                draw_source_clipped(
                    surface,
                    resources.context,
                    (
                        u32::from(open) * CONTEXT_HEIGHT,
                        0,
                        CONTEXT_HEIGHT,
                        CONTEXT_HEIGHT,
                    ),
                    IntRect::new(
                        arrow_x,
                        combo.y + (combo.h - CONTEXT_HEIGHT as i32) / 2,
                        CONTEXT_HEIGHT as i32,
                        CONTEXT_HEIGHT as i32,
                    ),
                    layout.roster_client,
                    gamma,
                );
            }
            draw_clipped_text(
                surface,
                &resources.fonts.text,
                combo.x + CONTEXT_HEIGHT as i32 + 2,
                combo.y + (combo.h - resources.fonts.text.line_height) / 2,
                &crate::c4_presentation_text(&row.value),
                COLOR_WHITE,
                TextAlign::Left,
                gamma,
                intersection(
                    layout.roster_client,
                    IntRect::new(combo.x, combo.y, (arrow_x - combo.x).max(0), combo.h),
                ),
            );
            if active
                && row.editable
                && (open
                    || self.hovered == HitTarget::OptionValue(row_layout.index)
                    || self.focus == LobbyControl::Option(row.kind))
            {
                with_surface_clip(surface, layout.roster_client, |surface| {
                    draw_highlight(surface, combo, &resources.button_highlight, gamma);
                });
            }
        }
    }

    #[allow(clippy::too_many_arguments)]
    fn draw_client_row(
        &self,
        surface: &mut Surface,
        client: &LobbyClientRow,
        row: LobbyRosterRowLayout,
        layout: &LobbyLayout,
        resources: &LobbyResources<'_>,
        active: bool,
        gamma: Option<&GammaRamp>,
    ) {
        draw_standard_icon_clipped(
            surface,
            row.icon,
            layout.roster_client,
            &resources.icons,
            self.client_status_at(client, Instant::now()).icon_phase(),
            gamma,
        );
        let label_x = row.rect.x + row.rect.h + ICON_LABEL_SPACING;
        draw_clipped_text_mode(
            surface,
            &resources.fonts.text,
            label_x,
            row.rect.y,
            &client.display_name(),
            client.color,
            TextAlign::Left,
            gamma,
            layout.roster_client,
            false,
        );
        if let Some(ping) = client.ping_ms {
            draw_clipped_text(
                surface,
                &resources.fonts.text,
                row.rect.x + row.rect.w,
                row.rect.y,
                &format!("{ping} ms"),
                COLOR_WHITE,
                TextAlign::Right,
                gamma,
                layout.roster_client,
            );
        }
        if let Some(add) = row.add_player {
            self.draw_small_button(
                surface,
                add,
                20,
                HitTarget::AddPlayer(row.index),
                layout.roster_client,
                resources,
                active,
                gamma,
            );
        }
    }

    #[allow(clippy::too_many_arguments)]
    fn draw_player_row(
        &self,
        surface: &mut Surface,
        player: &LobbyPlayerRow,
        row: LobbyRosterRowLayout,
        layout: &LobbyLayout,
        resources: &LobbyResources<'_>,
        active: bool,
        gamma: Option<&GammaRamp>,
    ) -> Result<()> {
        draw_roster_icon(
            surface,
            row.icon,
            layout.roster_client,
            &player.icon,
            resources,
            gamma,
        )?;
        if let Some(overlay) = &player.joined_player_overlay {
            draw_joined_player_overlay(
                surface,
                row.icon,
                layout.roster_client,
                &player.icon,
                overlay,
                gamma,
            )?;
        }
        let player_name = crate::c4_presentation_text(&player.name);
        draw_clipped_text(
            surface,
            &resources.fonts.text,
            row.rect.x + row.rect.h + ICON_LABEL_SPACING,
            row.rect.y + ICON_LABEL_SPACING,
            &player_name,
            player.color,
            TextAlign::Left,
            gamma,
            layout.roster_client,
        );
        if let Some(score) = &player.league_score {
            draw_clipped_text(
                surface,
                &resources.fonts.text,
                row.rect.x + row.rect.w - ICON_LABEL_SPACING,
                row.rect.y + ICON_LABEL_SPACING,
                score,
                COLOR_WHITE,
                TextAlign::Right,
                gamma,
                layout.roster_client,
            );
        }
        if let Some(team_rect) = row.team {
            let team = player.team.as_ref();
            let team_name = team
                .map(|team| crate::c4_presentation_text(&team.name))
                .unwrap_or_default();
            let selectable =
                team.is_some_and(|team| team.selectable) && !self.countdown.is_locked();
            let combo_open = self.open_team_combo_player == Some(player.id);
            if selectable {
                draw_source_clipped(
                    surface,
                    resources.context,
                    (u32::from(combo_open) * 16, 0, 16, 16),
                    team_rect.with_size(16, 16),
                    layout.roster_client,
                    gamma,
                );
            }
            draw_clipped_text(
                surface,
                &resources.fonts.text,
                team_rect.x + 18,
                team_rect.y + (team_rect.h - resources.fonts.text.line_height) / 2,
                &team_name,
                COLOR_WHITE,
                TextAlign::Left,
                gamma,
                intersection(team_rect, layout.roster_client),
            );
            if active
                && selectable
                && (combo_open
                    || self.hovered == HitTarget::Team(row.index)
                    || self.focus == LobbyControl::RosterTeam
                        && self.selected_row == Some(row.index))
            {
                draw_highlight_clipped(
                    surface,
                    team_rect,
                    layout.roster_client,
                    &resources.button_highlight,
                    gamma,
                );
            }
        }
        if let (Some(rank), Some(rank_rect)) = (player.league_rank, row.rank) {
            let phase = 35 + u16::from(rank.clamp(1, 9) - 1);
            draw_standard_icon_clipped(
                surface,
                rank_rect,
                layout.roster_client,
                &resources.icons,
                phase,
                gamma,
            );
        }
        Ok(())
    }

    #[allow(clippy::too_many_arguments)]
    fn draw_header_row(
        &self,
        surface: &mut Surface,
        header: &LobbyHeaderRow,
        row: LobbyRosterRowLayout,
        layout: &LobbyLayout,
        resources: &LobbyResources<'_>,
        active: bool,
        gamma: Option<&GammaRamp>,
    ) -> Result<()> {
        draw_roster_icon(
            surface,
            row.icon,
            layout.roster_client,
            &header.icon,
            resources,
            gamma,
        )?;
        draw_clipped_text(
            surface,
            &resources.fonts.text,
            row.rect.x + row.rect.h + ICON_LABEL_SPACING,
            row.rect.y,
            &header.label,
            COLOR_WHITE,
            TextAlign::Left,
            gamma,
            layout.roster_client,
        );
        if let Some(add) = row.add_player {
            self.draw_small_button(
                surface,
                add,
                20,
                HitTarget::AddPlayer(row.index),
                layout.roster_client,
                resources,
                active,
                gamma,
            );
        }
        Ok(())
    }

    #[allow(clippy::too_many_arguments)]
    fn draw_small_button(
        &self,
        surface: &mut Surface,
        rect: IntRect,
        phase: u16,
        target: HitTarget,
        clip: IntRect,
        resources: &LobbyResources<'_>,
        active: bool,
        gamma: Option<&GammaRamp>,
    ) {
        let highlighted = self.small_button_highlighted(target, active);
        if highlighted {
            draw_highlight_clipped(surface, rect, clip, &resources.button_highlight, gamma);
        }
        draw_standard_icon_clipped(surface, rect, clip, &resources.icons, phase, gamma);
        if active
            && (self.pointer_pressed == Some(target) && self.pointer_inside_pressed
                || target.button_control().is_some_and(|control| {
                    self.key_pressed
                        .is_some_and(|(pressed_control, _)| pressed_control == control)
                }))
        {
            draw_highlight_clipped(surface, rect, clip, &resources.button_highlight, gamma);
        }
    }

    fn small_button_highlighted(&self, target: HitTarget, active: bool) -> bool {
        active
            && (self.hovered == target
                || target.button_control().is_some_and(|control| {
                    self.focus == control
                        && match target {
                            HitTarget::AddPlayer(index) => self.selected_row == Some(index),
                            _ => true,
                        }
                }))
    }

    fn draw_ready(
        &self,
        surface: &mut Surface,
        layout: &LobbyLayout,
        resources: &LobbyResources<'_>,
        active: bool,
        gamma: Option<&GammaRamp>,
    ) {
        let phase = u16::from(self.ready) + 2 * u16::from(!self.resources_loaded);
        draw_phase(
            surface,
            layout.ready_square,
            resources.checkbox,
            phase,
            CHECKBOX_HEIGHT,
            gamma,
        );
        let label = if self.resources_loaded {
            &self.labels.ready
        } else {
            &self.labels.still_loading
        };
        let (label, _) = expand_hotkey_markup(label);
        resources.fonts.text.draw_with_gamma(
            surface,
            layout.ready_square.x + layout.ready_square.w + 4,
            layout.ready_checkbox.y
                + (layout.ready_checkbox.h - resources.fonts.text.line_height) / 2,
            &label,
            if self.resources_loaded {
                COLOR_WHITE
            } else {
                COLOR_GRAY
            },
            TextAlign::Left,
            true,
            gamma,
        );
        if active
            && (self.focus == LobbyControl::Ready
                || self.resources_loaded && self.hovered == HitTarget::Ready)
        {
            let marker = IntRect::new(
                layout.ready_square.x + layout.ready_square.w / 4,
                layout.ready_square.y + layout.ready_square.h / 4,
                layout.ready_square.w / 2,
                layout.ready_square.h / 2,
            );
            draw_highlight(surface, marker, &resources.button_highlight, gamma);
        }
    }

    fn button_state(&self, control: LobbyControl, active: bool) -> ClassicButtonState {
        let target = match control {
            LobbyControl::Exit => HitTarget::Exit,
            LobbyControl::Run => HitTarget::Run,
            LobbyControl::Preload => HitTarget::Preload,
            _ => HitTarget::None,
        };
        ClassicButtonState {
            pressed: active
                && (self.pointer_pressed == Some(target) && self.pointer_inside_pressed
                    || self
                        .key_pressed
                        .is_some_and(|(pressed, _)| pressed == control)),
            highlighted: active && (self.focus == control || self.hovered == target),
        }
    }
}

fn draw_text_window(surface: &mut Surface, rect: IntRect, gamma: Option<&GammaRamp>) {
    draw_engine_box(
        surface,
        rect.x,
        rect.y,
        rect.x + rect.w - 1,
        rect.y + rect.h - 1,
        DARK_BACKGROUND,
        gamma,
    );
    draw_3d_frame(surface, rect, gamma);
}

fn draw_edit(
    surface: &mut Surface,
    rect: IntRect,
    view: &LobbyChatEditView,
    font: &clonk_graphics::clonk_font::ClonkFont,
    active: bool,
    gamma: Option<&GammaRamp>,
) {
    let client = edit_client(rect);
    draw_engine_box(
        surface,
        rect.x,
        rect.y,
        rect.x + rect.w - 1,
        client.y + client.h,
        DARK_BACKGROUND,
        gamma,
    );
    draw_3d_frame(surface, rect, gamma);
    let (text_y, selection_height) = if client.h <= font.line_height {
        (client.y, client.h)
    } else {
        (
            client.y + (client.h - font.line_height) / 2 + 1,
            font.line_height - 2,
        )
    };
    let clip = IntRect::new(client.x - 2, client.y, client.w + 4, client.h + 1);
    if let Some((anchor, caret)) = view.selection {
        let (start, end) = if anchor <= caret {
            (anchor, caret)
        } else {
            (caret, anchor)
        };
        let x1 = client.x + font.measure(&view.text[..start], false).0 - view.horizontal_scroll;
        let x2 = client.x + font.measure(&view.text[..end], false).0 - view.horizontal_scroll;
        let clipped_x1 = x1.max(clip.x);
        let clipped_x2 = (x2 - 1).min(clip.x + clip.w - 1);
        if clipped_x1 <= clipped_x2 {
            draw_engine_box(
                surface,
                clipped_x1,
                text_y,
                clipped_x2,
                text_y + selection_height - 1,
                EDIT_SELECTION,
                gamma,
            );
        }
    }
    draw_clipped_text(
        surface,
        font,
        client.x - view.horizontal_scroll,
        text_y - 1,
        &view.text,
        COLOR_WHITE,
        TextAlign::Left,
        gamma,
        clip,
    );
    if active && view.cursor_visible {
        let caret_x = client.x + font.measure(&view.text[..view.caret], false).0
            - font.measure("\u{a6}", false).0 / 2
            - view.horizontal_scroll;
        draw_scaled_caret(
            surface,
            font,
            caret_x,
            text_y - font.line_height / 3,
            clip,
            gamma,
        );
    }
}

#[allow(clippy::too_many_arguments)]
fn draw_clipped_text(
    surface: &mut Surface,
    font: &clonk_graphics::clonk_font::ClonkFont,
    x: i32,
    y: i32,
    text: &str,
    color: [u8; 4],
    align: TextAlign,
    gamma: Option<&GammaRamp>,
    clip: IntRect,
) {
    draw_clipped_text_mode(surface, font, x, y, text, color, align, gamma, clip, true);
}

#[allow(clippy::too_many_arguments)]
fn draw_clipped_text_mode(
    surface: &mut Surface,
    font: &clonk_graphics::clonk_font::ClonkFont,
    x: i32,
    y: i32,
    text: &str,
    color: [u8; 4],
    align: TextAlign,
    gamma: Option<&GammaRamp>,
    clip: IntRect,
    markup: bool,
) {
    with_surface_clip(surface, clip, |surface| {
        font.draw_with_gamma(surface, x, y, text, color, align, markup, gamma);
    });
}

#[allow(clippy::too_many_arguments)]
fn draw_scrollbar(
    surface: &mut Surface,
    rect: IntRect,
    scroll: &ImageData,
    pin: i32,
    max_scroll: i32,
    top_down: bool,
    bottom_down: bool,
    gamma: Option<&GammaRamp>,
) {
    let top_x = if top_down { 16.0 } else { 0.0 };
    let bottom_x = if bottom_down { 16.0 } else { 0.0 };
    draw_facet_stretch(
        surface,
        scroll,
        (top_x, 0.0, 16.0, 16.0),
        (rect.x as f32, rect.y as f32, 16.0, 16.0),
        gamma,
    );
    let mut y = 16;
    while y < rect.h - 5 {
        let height = 16.min(rect.h - 5 - y);
        if height <= 0 {
            break;
        }
        draw_facet_stretch(
            surface,
            scroll,
            (0.0, 16.0, 16.0, height as f32),
            (rect.x as f32, (rect.y + y) as f32, 16.0, height as f32),
            gamma,
        );
        y += 16;
    }
    draw_facet_stretch(
        surface,
        scroll,
        (bottom_x, 32.0, 16.0, 16.0),
        (rect.x as f32, (rect.y + rect.h - 16) as f32, 16.0, 16.0),
        gamma,
    );
    if max_scroll > 0 && rect.h >= 48 {
        draw_facet_stretch(
            surface,
            scroll,
            (16.0, 16.0, 16.0, 16.0),
            (rect.x as f32, (rect.y + 16 + pin) as f32, 16.0, 16.0),
            gamma,
        );
    }
}

fn draw_roster_icon(
    surface: &mut Surface,
    rect: IntRect,
    clip: IntRect,
    icon: &LobbyRosterIcon,
    resources: &LobbyResources<'_>,
    gamma: Option<&GammaRamp>,
) -> Result<()> {
    match icon {
        LobbyRosterIcon::Standard(phase) => {
            ensure!(
                *phase < 54,
                "GUIIcons.png phase {phase} is outside the classic 54-phase sheet"
            );
            draw_standard_icon_clipped(surface, rect, clip, &resources.icons, *phase, gamma);
        }
        LobbyRosterIcon::Raster(image) => {
            ensure!(
                image.width() > 0 && image.height() > 0,
                "lobby roster raster icon must not be empty"
            );
            let fitted = aspect_fit_roster_raster(image.width(), image.height(), rect);
            draw_image_clipped(surface, image, fitted, clip, gamma);
        }
    }
    Ok(())
}

/// `C4GUI::Icon` is a `Picture(..., fAspect=true)`, so custom player
/// rasters retain their aspect ratio inside the square roster-icon bounds.
/// Keep the integer comparisons and centering used by `C4Facet::Draw`.
fn aspect_fit_roster_raster(source_width: u32, source_height: u32, bounds: IntRect) -> IntRect {
    if source_width == 0 || source_height == 0 || bounds.w <= 0 || bounds.h <= 0 {
        return bounds;
    }
    let source_width = i64::from(source_width);
    let source_height = i64::from(source_height);
    let bounds_width = i64::from(bounds.w);
    let bounds_height = i64::from(bounds.h);
    let mut fitted = bounds;
    if 100 * bounds_width / source_width < 100 * bounds_height / source_height {
        let height = source_height * bounds_width / source_width;
        fitted.h = i32::try_from(height).unwrap_or(bounds.h);
        fitted.y += (bounds.h - fitted.h) / 2;
    } else if 100 * bounds_height / source_height < 100 * bounds_width / source_width {
        let width = source_width * bounds_height / source_height;
        fitted.w = i32::try_from(width).unwrap_or(bounds.w);
        fitted.x += (bounds.w - fitted.w) / 2;
    }
    fitted
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct JoinedPlayerOverlayLayout {
    shadow: IntRect,
    colored: IntRect,
    clip: IntRect,
}

/// `C4PlayerInfoListBox::PlayerListItem::UpdateIcon` draws into the base
/// icon's source surface before `C4GUI::Picture` aspect-fits that surface.
/// Calculate in source coordinates first, then map into the fitted row icon.
fn joined_player_overlay_layout(
    icon: &LobbyRosterIcon,
    crew: &ImageData,
    bounds: IntRect,
    clip: IntRect,
) -> Option<JoinedPlayerOverlayLayout> {
    let (source_width, source_height, fitted) = match icon {
        LobbyRosterIcon::Standard(_) => (
            CLASSIC_ROSTER_ICON_EXTENT,
            CLASSIC_ROSTER_ICON_EXTENT,
            bounds,
        ),
        LobbyRosterIcon::Raster(image) => (
            image.width(),
            image.height(),
            aspect_fit_roster_raster(image.width(), image.height(), bounds),
        ),
    };
    if source_width == 0
        || source_height == 0
        || crew.width() == 0
        || crew.height() == 0
        || fitted.w <= 0
        || fitted.h <= 0
    {
        return None;
    }
    let size_max = source_width.max(source_height);
    let crew_height = size_max / 2;
    if crew_height >= source_height {
        return None;
    }
    let overlay_bounds = IntRect::new(
        0,
        i32::try_from(crew_height).ok()?,
        i32::try_from(size_max / 2).ok()?,
        i32::try_from(source_height - crew_height).ok()?,
    );
    let colored_source = aspect_fit_roster_raster(crew.width(), crew.height(), overlay_bounds);
    let shadow_source =
        aspect_fit_roster_raster(crew.width(), crew.height(), overlay_bounds.with_x(2));

    let map = |source: IntRect| {
        let map_axis = |start: i32, extent: i32, source_start: i32, source_extent: u32| {
            start
                + i32::try_from(
                    i64::from(source_start) * i64::from(extent) / i64::from(source_extent),
                )
                .unwrap_or_default()
        };
        let left = map_axis(fitted.x, fitted.w, source.x, source_width);
        let top = map_axis(fitted.y, fitted.h, source.y, source_height);
        let right = map_axis(fitted.x, fitted.w, source.x + source.w, source_width);
        let bottom = map_axis(fitted.y, fitted.h, source.y + source.h, source_height);
        IntRect::new(left, top, right - left, bottom - top)
    };

    Some(JoinedPlayerOverlayLayout {
        shadow: map(shadow_source),
        colored: map(colored_source),
        clip: intersection(clip, fitted),
    })
}
fn draw_joined_player_overlay(
    surface: &mut Surface,
    bounds: IntRect,
    clip: IntRect,
    icon: &LobbyRosterIcon,
    overlay: &LobbyJoinedPlayerOverlay,
    gamma: Option<&GammaRamp>,
) -> Result<()> {
    ensure!(
        overlay.crew.width() > 0 && overlay.crew.height() > 0,
        "joined-player crew raster must not be empty"
    );
    let Some(layout) = joined_player_overlay_layout(icon, &overlay.crew, bounds, clip) else {
        return Ok(());
    };
    let [red, green, blue, alpha] = overlay.color;
    let colored = crate::hud::colorize_by_owner(&overlay.crew, Color::new(red, green, blue, alpha));
    let mut shadow_pixels = Vec::with_capacity(colored.pixels().len());
    for pixel in colored.pixels().chunks_exact(4) {
        shadow_pixels.extend_from_slice(&[0, 0, 0, pixel[3]]);
    }
    let shadow = ImageData::new(colored.width(), colored.height(), shadow_pixels);
    draw_image_clipped(surface, &shadow, layout.shadow, layout.clip, gamma);
    draw_image_clipped(surface, &colored, layout.colored, layout.clip, gamma);
    Ok(())
}

fn draw_standard_icon(
    surface: &mut Surface,
    rect: IntRect,
    icons: &ImageData,
    phase: u16,
    gamma: Option<&GammaRamp>,
) {
    draw_phase(surface, rect, icons, phase, 40, gamma);
}

fn draw_extended_icon(
    surface: &mut Surface,
    rect: IntRect,
    icons: &ImageData,
    phase: u16,
    gamma: Option<&GammaRamp>,
) {
    draw_phase(surface, rect, icons, phase, 64, gamma);
}

fn draw_standard_icon_clipped(
    surface: &mut Surface,
    rect: IntRect,
    clip: IntRect,
    icons: &ImageData,
    phase: u16,
    gamma: Option<&GammaRamp>,
) {
    let columns = (icons.width() / 40).max(1);
    let source = (
        u32::from(phase) % columns * 40,
        u32::from(phase) / columns * 40,
        40,
        40,
    );
    draw_source_clipped(surface, icons, source, rect, clip, gamma);
}

fn draw_phase(
    surface: &mut Surface,
    rect: IntRect,
    image: &ImageData,
    phase: u16,
    cell: u32,
    gamma: Option<&GammaRamp>,
) {
    let columns = (image.width() / cell).max(1);
    let x = u32::from(phase) % columns * cell;
    let y = u32::from(phase) / columns * cell;
    draw_facet_stretch(
        surface,
        image,
        (x as f32, y as f32, cell as f32, cell as f32),
        (rect.x as f32, rect.y as f32, rect.w as f32, rect.h as f32),
        gamma,
    );
}

fn draw_highlight(
    surface: &mut Surface,
    rect: IntRect,
    highlight: &ImageData,
    gamma: Option<&GammaRamp>,
) {
    crate::draw_image_bilinear_additive(surface, &gui_rect(rect), highlight, gamma);
}

fn draw_highlight_clipped(
    surface: &mut Surface,
    rect: IntRect,
    clip: IntRect,
    highlight: &ImageData,
    gamma: Option<&GammaRamp>,
) {
    // All small-button highlights are fully in the roster for normal layouts.
    // Guard partial rows rather than bleeding into the scrollbar/caption.
    if intersection(rect, clip) == rect {
        draw_highlight(surface, rect, highlight, gamma);
    }
}

fn draw_image_clipped(
    surface: &mut Surface,
    image: &ImageData,
    rect: IntRect,
    clip: IntRect,
    gamma: Option<&GammaRamp>,
) {
    draw_source_clipped(
        surface,
        image,
        (0, 0, image.width(), image.height()),
        rect,
        clip,
        gamma,
    );
}

fn draw_source_clipped(
    surface: &mut Surface,
    image: &ImageData,
    source: (u32, u32, u32, u32),
    destination: IntRect,
    clip: IntRect,
    gamma: Option<&GammaRamp>,
) {
    let visible = intersection(destination, clip);
    if visible.w <= 0 || visible.h <= 0 || destination.w <= 0 || destination.h <= 0 {
        return;
    }
    let scale_x = source.2 as f32 / destination.w as f32;
    let scale_y = source.3 as f32 / destination.h as f32;
    let source_x = source.0 as f32 + (visible.x - destination.x) as f32 * scale_x;
    let source_y = source.1 as f32 + (visible.y - destination.y) as f32 * scale_y;
    draw_facet_stretch(
        surface,
        image,
        (
            source_x,
            source_y,
            visible.w as f32 * scale_x,
            visible.h as f32 * scale_y,
        ),
        (
            visible.x as f32,
            visible.y as f32,
            visible.w as f32,
            visible.h as f32,
        ),
        gamma,
    );
}

fn edit_client(rect: IntRect) -> IntRect {
    IntRect::new(
        rect.x + 4,
        rect.y + 2,
        (rect.w - 8).max(0),
        (rect.h - 4).max(0),
    )
}

fn center(rect: IntRect) -> GuiPoint {
    GuiPoint::new((rect.x + rect.w / 2) as f32, (rect.y + rect.h / 2) as f32)
}

fn valid_boundary_at_or_before(text: &str, mut position: usize) -> usize {
    position = position.min(text.len());
    while position > 0 && !text.is_char_boundary(position) {
        position -= 1;
    }
    position
}

fn scrollbar_max_pin(rect: IntRect) -> i32 {
    (rect.h - 3 * SCROLLBAR_EXTENT).max(0)
}

fn scroll_to_pin(scroll: i32, max_scroll: i32, max_pin: i32) -> i32 {
    if max_scroll <= 0 || max_pin <= 0 {
        0
    } else {
        (max_pin * scroll / max_scroll).clamp(0, max_pin)
    }
}

fn pin_to_scroll(pin: i32, max_scroll: i32, max_pin: i32) -> i32 {
    if max_scroll <= 0 || max_pin <= 0 {
        0
    } else {
        (max_scroll * pin / max_pin).clamp(0, max_scroll)
    }
}

fn contains(rect: IntRect, point: GuiPoint) -> bool {
    point.x >= rect.x as f32
        && point.y >= rect.y as f32
        && point.x < (rect.x + rect.w) as f32
        && point.y < (rect.y + rect.h) as f32
}

fn intersects(a: IntRect, b: IntRect) -> bool {
    a.x < b.x + b.w && a.x + a.w > b.x && a.y < b.y + b.h && a.y + a.h > b.y
}

fn intersection(a: IntRect, b: IntRect) -> IntRect {
    let x = a.x.max(b.x);
    let y = a.y.max(b.y);
    let right = (a.x + a.w).min(b.x + b.w);
    let bottom = (a.y + a.h).min(b.y + b.h);
    IntRect::new(x, y, (right - x).max(0), (bottom - y).max(0))
}

fn gui_rect(rect: IntRect) -> GuiRect {
    GuiRect::new(rect.x as f32, rect.y as f32, rect.w as f32, rect.h as f32)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_support::{endeavour_font_set, load_graphics_png};

    fn client(id: i32, local: bool) -> LobbyRosterRow {
        LobbyRosterRow::Client(LobbyClientRow {
            id,
            name: format!("Client {id}"),
            nick: format!("Nick {id}"),
            color: COLOR_WHITE,
            status: if id == 1 {
                LobbyClientStatus::Host
            } else {
                LobbyClientStatus::Client
            },
            local,
            connected: true,
            resource_progress: None,
            ping_ms: None,
        })
    }

    fn player(id: i32) -> LobbyRosterRow {
        LobbyRosterRow::Player(LobbyPlayerRow {
            id,
            client_id: 1,
            name: format!("Player {id}"),
            color: COLOR_WHITE,
            icon: LobbyRosterIcon::Standard(7),
            joined_player_overlay: None,
            team: None,
            league_score: None,
            league_rank: None,
        })
    }

    fn team_player(id: i32) -> LobbyRosterRow {
        let mut row = match player(id) {
            LobbyRosterRow::Player(row) => row,
            _ => unreachable!(),
        };
        row.team = Some(LobbyTeamValue {
            id: 7,
            name: "Blue".to_string(),
            selectable: true,
        });
        LobbyRosterRow::Player(row)
    }

    fn lobby(role: LobbyRole, rows: Vec<LobbyRosterRow>) -> GameLobby {
        GameLobby::new(role, "Gold Mine", 1, 4, false, false, true, false, 5, rows)
    }

    fn header(kind: LobbyRosterHeader, can_add_player: bool) -> LobbyRosterRow {
        LobbyRosterRow::Header(LobbyHeaderRow {
            kind,
            label: format!("{kind:?}"),
            icon: LobbyRosterIcon::Standard(7),
            can_add_player,
        })
    }

    #[test]
    fn roster_raster_aspect_fit_matches_c4facet_integer_math() {
        let wide = aspect_fit_roster_raster(60, 30, IntRect::new(4, 8, 20, 20));
        assert_eq!(wide, IntRect::new(4, 13, 20, 10));

        let tall = aspect_fit_roster_raster(30, 60, IntRect::new(4, 8, 20, 20));
        assert_eq!(tall, IntRect::new(9, 8, 10, 20));

        let square = aspect_fit_roster_raster(64, 64, IntRect::new(4, 8, 20, 20));
        assert_eq!(square, IntRect::new(4, 8, 20, 20));
    }

    #[test]
    fn classic_player_fallback_composes_to_40px_before_join_overlay() {
        // C4Surface::Create initializes the texture backing store to 0xff
        // (C4Surface.cpp:1110-1113), so aspect-fit bars remain transparent
        // white around the pixels Blit8 writes.
        let player = ImageData::new(2, 1, vec![0, 0, 255, 255, 200, 20, 10, 255]);
        let icon = compose_classic_lobby_player_fallback_icon(&player, Color::opaque(12, 34, 56))
            .expect("non-empty active Player raster");
        assert_eq!((icon.width(), icon.height()), (40, 40));
        let pixel = |x: u32, y: u32| {
            let start = ((y * icon.width() + x) * 4) as usize;
            &icon.pixels()[start..start + 4]
        };
        assert_eq!(pixel(5, 9), [255, 255, 255, 0]);
        // The offscreen Blit8 path applies StdColors.h:159-169 modulation.
        assert_eq!(pixel(5, 20), [11, 33, 55, 255]);
        assert_eq!(pixel(30, 20), [200, 20, 10, 255]);
        assert_eq!(pixel(5, 30), [255, 255, 255, 0]);

        let crew = ImageData::new(1, 1, vec![0, 0, 255, 255]);
        let bounds = IntRect::new(0, 0, 40, 40);
        let layout =
            joined_player_overlay_layout(&LobbyRosterIcon::Raster(icon), &crew, bounds, bounds)
                .expect("40x40 fallback accepts a joined-player overlay");
        assert_eq!(
            layout,
            JoinedPlayerOverlayLayout {
                shadow: IntRect::new(2, 20, 20, 20),
                colored: IntRect::new(0, 20, 20, 20),
                clip: bounds,
            }
        );
    }

    #[test]
    fn classic_player_fallback_uses_offscreen_blit8_nearest_sampling() {
        // C4PlayerInfoListBox.cpp:293-294 creates a non-render-target 40x40
        // surface, so StdDDraw2.cpp:644-645,846-872 selects Blit8's integer
        // nearest-neighbour source coordinate for the fallback player icon.
        let player = ImageData::new(
            3,
            1,
            vec![10, 10, 10, 255, 100, 100, 100, 255, 200, 200, 200, 255],
        );
        let icon = compose_classic_lobby_player_fallback_icon(&player, Color::opaque(1, 2, 3))
            .expect("non-empty active Player raster");
        let pixel = |x: u32, y: u32| {
            let start = ((y * icon.width() + x) * 4) as usize;
            &icon.pixels()[start..start + 4]
        };

        assert_eq!(pixel(13, 20), [10, 10, 10, 255]);
        assert_eq!(pixel(14, 20), [100, 100, 100, 255]);
        assert_eq!(pixel(26, 20), [100, 100, 100, 255]);
        assert_eq!(pixel(27, 20), [200, 200, 200, 255]);
    }

    #[test]
    fn classic_player_fallback_uses_blit8_owner_modulation() {
        // Blit8 asks C4Surface::GetPixDw(..., true) for each source pixel
        // (C4Surface.cpp:742-755); its ClrByOwner path uses ModulateClr's
        // divide-by-256 RGB products (C4Surface.cpp:672-700;
        // StdColors.h:159-169), including 255*255 -> 254.
        let player = ImageData::new(1, 1, vec![0, 0, 255, 255]);
        let icon =
            compose_classic_lobby_player_fallback_icon(&player, Color::opaque(255, 255, 255))
                .expect("non-empty active Player raster");
        let center = ((20 * icon.width() + 20) * 4) as usize;

        assert_eq!(&icon.pixels()[center..center + 4], [254, 254, 254, 255]);
    }

    #[test]
    fn classic_player_fallback_preserves_partial_alpha_rgb_in_empty_cache() {
        // Blit8 reaches C4Surface::BltPix (C4Surface.cpp:742-755). The empty
        // destination has inverted alpha 0xff, so BltAlpha assigns the source
        // pixel verbatim instead of premultiplying its RGB
        // (StdColors.h:120-126).
        let player = ImageData::new(1, 1, vec![40, 40, 40, 128]);
        let icon = compose_classic_lobby_player_fallback_icon(&player, Color::opaque(12, 34, 56))
            .expect("partial-alpha Player raster creates the fallback surface");
        let center = ((20 * icon.width() + 20) * 4) as usize;

        assert_eq!(&icon.pixels()[center..center + 4], [40, 40, 40, 128]);
    }

    #[test]
    fn joined_player_overlay_matches_cpp_lower_half_shadow_aspect_and_clip() {
        // Keep the 2:1 aspect ratio while filling C4Surface's minimum 2px
        // texture height, so this layout/clip test does not also exercise the
        // transparent padding row (C4Surface.cpp:182-205,955-991).
        let crew = ImageData::new(4, 2, [0, 0, 255, 255].repeat(8));
        let icon = LobbyRosterIcon::Standard(7);
        let bounds = IntRect::new(4, 3, 40, 40);
        let clip = IntRect::new(4, 3, 22, 34);
        let layout = joined_player_overlay_layout(&icon, &crew, bounds, clip)
            .expect("non-empty crew overlay");
        assert_eq!(
            layout,
            JoinedPlayerOverlayLayout {
                shadow: IntRect::new(6, 28, 20, 10),
                colored: IntRect::new(4, 28, 20, 10),
                clip,
            }
        );

        let mut surface = Surface::new(48, 48, PixelFormat::Rgba8888);
        draw_joined_player_overlay(
            &mut surface,
            bounds,
            clip,
            &icon,
            &LobbyJoinedPlayerOverlay {
                crew,
                color: [255, 0, 0, 255],
            },
            None,
        )
        .expect("overlay render");

        assert_eq!(surface.get_pixel(4, 27), Some(Color::transparent()));
        assert_eq!(surface.get_pixel(4, 28), Some(Color::opaque(255, 0, 0)));
        assert_eq!(surface.get_pixel(23, 36), Some(Color::opaque(255, 0, 0)));
        assert_eq!(surface.get_pixel(24, 28), Some(Color::opaque(0, 0, 0)));
        assert_eq!(surface.get_pixel(25, 28), Some(Color::opaque(0, 0, 0)));
        assert_eq!(surface.get_pixel(26, 28), Some(Color::transparent()));
        assert_eq!(surface.get_pixel(4, 37), Some(Color::transparent()));
    }

    #[test]
    fn clipped_lobby_text_capture_keeps_global_coordinates_and_effective_clip() {
        let fonts = endeavour_font_set();
        let mut surface = Surface::new(40, 30, PixelFormat::Rgba8888);
        let outer = clonk_graphics::Rect::new(5, 2, 20, 20);
        surface.set_clip(outer);
        surface.begin_clonk_text_capture();

        draw_clipped_text_mode(
            &mut surface,
            &fonts.text,
            7,
            6,
            "Lobby",
            COLOR_WHITE,
            TextAlign::Left,
            None,
            IntRect::new(2, 4, 10, 10),
            false,
        );

        assert_eq!(surface.clip(), Some(outer));
        assert!(surface.pixels().iter().all(|byte| *byte == 0));
        let commands = surface.take_clonk_text_capture();
        assert_eq!(commands.len(), 1);
        assert_eq!(commands[0].x, 7);
        assert_eq!(commands[0].y, 6);
        assert!(!commands[0].markup);
        assert_eq!(
            commands[0].clip,
            Some(clonk_graphics::Rect::new(5, 4, 7, 10))
        );
    }

    #[test]
    fn client_sound_icon_expires_to_the_underlying_roster_status_after_one_second() {
        let start = Instant::now();
        let mut lobby = lobby(LobbyRole::Host, vec![client(1, true), client(7, false)]);
        let remote = match lobby.rows()[1].clone() {
            LobbyRosterRow::Client(client) => client,
            _ => unreachable!(),
        };

        lobby.note_client_sound_at(7, true, start);
        assert_eq!(
            lobby.client_status_at(&remote, start + Duration::from_millis(999)),
            LobbyClientStatus::MutedSound
        );
        assert_eq!(
            lobby.client_status_at(&remote, start + Duration::from_secs(1)),
            LobbyClientStatus::Client
        );

        lobby.note_client_sound_at(7, false, start + Duration::from_secs(2));
        assert_eq!(
            lobby.client_status_at(&remote, start + Duration::from_secs(2)),
            LobbyClientStatus::Sound
        );
    }

    #[test]
    fn independently_drawn_wrapped_lobby_lines_close_and_reopen_markup() {
        let fonts = endeavour_font_set();
        let mut lobby = lobby(LobbyRole::Host, vec![]);
        let mut layout = game_lobby_layout(
            640,
            300,
            fonts.title.line_height,
            fonts.text.line_height,
            LobbyRole::Host,
            false,
            false,
        );
        layout.chat_log_client.w = fonts.text.measure("W", true).0;
        lobby.set_logs(vec![LobbyLogLine {
            text: "<c 123456>WWW</c>".into(),
            color: COLOR_WHITE,
        }]);

        let wrapped = lobby.wrapped_chat_lines(&layout, &fonts.text);
        assert_eq!(
            wrapped
                .iter()
                .map(|line| line.text.as_str())
                .collect::<Vec<_>>(),
            ["<c 123456>W</c>", "<c 123456>WW</c>"]
        );
        assert_eq!(
            wrapped
                .iter()
                .map(|line| line.new_paragraph)
                .collect::<Vec<_>>(),
            [true, false]
        );
    }

    #[test]
    fn normal_host_layout_matches_constructor_math() {
        let layout = game_lobby_layout(1280, 720, 34, 22, LobbyRole::Host, false, false);
        assert_eq!(layout.client, IntRect::new(25, 69, 1230, 632));
        assert_eq!(layout.title_anchor, (640, 8));
        assert_eq!(layout.chat_log, IntRect::new(45, 77, 780, 491));
        assert_eq!(layout.chat_label, IntRect::new(45, 584, 40, 25));
        assert_eq!(layout.chat_edit, IntRect::new(85, 584, 740, 25));
        assert_eq!(layout.right_caption, IntRect::new(850, 77, 400, 23));
        assert_eq!(layout.right_tab, IntRect::new(850, 99, 400, 510));
        assert_eq!(layout.roster, IntRect::new(854, 103, 392, 502));
        assert_eq!(layout.exit_button, IntRect::new(35, 633, 100, 32));
        assert_eq!(layout.run_button, Some(IntRect::new(1145, 633, 100, 32)));
        assert_eq!(layout.ready_checkbox, IntRect::new(1015, 633, 110, 32));
        assert_eq!(layout.game_option_strip, IntRect::new(155, 617, 840, 64));
        assert_eq!(layout.tab_buttons[0].rect, IntRect::new(1170, 81, 16, 16));
        assert_eq!(layout.tab_buttons[3].rect, IntRect::new(1230, 81, 16, 16));
    }

    #[test]
    fn players_sheet_applies_the_native_list_box_client_margins() {
        // C4GUI::ListBox gives its ScrollWindow three-pixel margins before
        // reserving the scrollbar (src/C4GuiListBox.h:121-125;
        // src/C4GuiContainers.cpp:477-486).
        let fonts = endeavour_font_set();
        let lobby = lobby(LobbyRole::Host, vec![]);

        let layout = lobby.layout(1280, 720, &fonts);

        assert_eq!(layout.roster, IntRect::new(854, 103, 392, 502));
        assert_eq!(layout.roster_client, IntRect::new(857, 106, 370, 496));
        assert_eq!(layout.roster_scrollbar, IntRect::new(1227, 106, 16, 496));
    }

    #[test]
    fn client_variant_recenters_options_without_go_button() {
        let layout = game_lobby_layout(1280, 720, 34, 22, LobbyRole::Client, false, false);
        assert_eq!(layout.run_button, None);
        assert_eq!(layout.ready_checkbox, IntRect::new(1135, 633, 110, 32));
        assert_eq!(layout.game_option_strip, IntRect::new(185, 617, 930, 64));
    }

    #[test]
    fn optional_team_and_chat_buttons_preserve_cpp_add_order() {
        let layout = game_lobby_layout(1280, 720, 34, 22, LobbyRole::Host, true, true);
        let controls: Vec<_> = layout.tab_buttons.iter().map(|tab| tab.control).collect();
        assert_eq!(
            controls,
            [
                LobbyControl::TeamsTab,
                LobbyControl::PlayersTab,
                LobbyControl::ResourcesTab,
                LobbyControl::OptionsTab,
                LobbyControl::ScenarioTab,
                LobbyControl::ChatDialog,
            ]
        );
        assert_eq!(layout.tab_buttons[0].rect.x, 1130);
        assert_eq!(layout.tab_buttons[5].rect.x, 1230);
    }

    #[test]
    fn live_lobby_chrome_updates_clear_removed_control_interactions() {
        let fonts = endeavour_font_set();
        let mut lobby = GameLobby::new(
            LobbyRole::Client,
            "Old Title",
            1,
            4,
            true,
            true,
            true,
            false,
            5,
            vec![team_player(2)],
        );

        lobby.set_scenario_title("New Title");
        assert_eq!(lobby.title(), "New Title - Lobby");

        lobby.set_active_sheet(LobbySheet::Teams);
        lobby.set_open_team_combo_player(Some(2));
        lobby.focus = LobbyControl::RosterTeam;
        lobby.hovered = HitTarget::Team(0);
        lobby.pointer_pressed = Some(HitTarget::Team(0));
        lobby.pointer_inside_pressed = true;
        lobby.key_pressed = Some((LobbyControl::RosterTeam, KeyCode::Space));
        lobby.set_has_teams(false);
        assert_eq!(lobby.active_sheet(), LobbySheet::Players);
        assert_eq!(lobby.focus(), LobbyControl::Roster);
        assert_eq!(lobby.open_team_combo_player(), None);
        assert_eq!(lobby.hovered, HitTarget::None);
        assert_eq!(lobby.pointer_pressed, None);
        assert!(!lobby.pointer_inside_pressed);
        assert_eq!(lobby.key_pressed, None);
        assert!(!lobby
            .layout(1280, 720, &fonts)
            .tab_buttons
            .iter()
            .any(|tab| tab.control == LobbyControl::TeamsTab));

        lobby.focus = LobbyControl::ChatDialog;
        lobby.hovered = HitTarget::Tab(LobbyControl::ChatDialog);
        lobby.pointer_pressed = Some(HitTarget::Tab(LobbyControl::ChatDialog));
        lobby.pointer_inside_pressed = true;
        lobby.key_pressed = Some((LobbyControl::ChatDialog, KeyCode::Enter));
        lobby.set_has_external_chat(false);
        assert_eq!(lobby.focus(), LobbyControl::ChatInput);
        assert_eq!(lobby.hovered, HitTarget::None);
        assert_eq!(lobby.pointer_pressed, None);
        assert!(!lobby.pointer_inside_pressed);
        assert_eq!(lobby.key_pressed, None);
        assert!(!lobby
            .layout(1280, 720, &fonts)
            .tab_buttons
            .iter()
            .any(|tab| tab.control == LobbyControl::ChatDialog));
    }

    #[test]
    fn resource_rows_use_cpp_id_order_basename_progress_and_geometry() {
        let fonts = endeavour_font_set();
        let mut lobby = lobby(LobbyRole::Host, vec![client(1, true)]);
        lobby.set_resource_rows(vec![
            LobbyResourceRow {
                id: 9,
                filename: r"Network\Definitions.c4d".into(),
                present_percent: u8::MAX,
                save_possible: false,
            },
            LobbyResourceRow {
                id: -2,
                filename: "anonymous.c4s".into(),
                present_percent: 5,
                save_possible: false,
            },
            LobbyResourceRow {
                id: 2,
                filename: "Network/Scenario.c4s".into(),
                present_percent: 42,
                save_possible: false,
            },
        ]);

        assert_eq!(
            lobby
                .resource_rows()
                .iter()
                .map(|row| row.id)
                .collect::<Vec<_>>(),
            [2, 9]
        );
        assert_eq!(lobby.resource_rows()[0].basename(), "Scenario.c4s");
        assert_eq!(
            lobby.resource_rows()[0].progress_label().as_deref(),
            Some("42%")
        );
        assert_eq!(lobby.resource_rows()[1].basename(), "Definitions.c4d");
        assert_eq!(lobby.resource_rows()[1].present_percent, 100);
        assert_eq!(lobby.resource_rows()[1].progress_label(), None);

        lobby.set_active_sheet(LobbySheet::Resources);
        let layout = lobby.layout(1280, 720, &fonts);
        let resources = lobby.roster_layout(&layout, fonts.text.line_height);
        let row_height = fonts.text.line_height + 4;
        assert_eq!(resources.content_height, 2 * row_height);
        assert_eq!(resources.rows[0].rect.h, row_height);
        assert_eq!(resources.rows[0].icon.y, resources.rows[0].rect.y + 2);
        assert_eq!(resources.rows[0].icon.w, fonts.text.line_height);
        assert_eq!(resources.rows[0].icon.h, fonts.text.line_height);
        assert!(resources
            .rows
            .iter()
            .all(|row| { row.add_player.is_none() && row.team.is_none() && row.rank.is_none() }));

        assert!(lobby.set_resource_progress(2, 99));
        assert_eq!(
            lobby.resource_rows()[0].progress_label().as_deref(),
            Some("99%")
        );
        assert!(lobby.set_resource_progress(2, 101));
        assert_eq!(lobby.resource_rows()[0].progress_label(), None);
        assert!(!lobby.set_resource_progress(77, 10));
        assert!(lobby.remove_resource_row(9));
        assert!(!lobby.remove_resource_row(9));
    }

    #[test]
    fn resource_save_button_uses_native_geometry_and_emits_the_exact_row_id() {
        let fonts = endeavour_font_set();
        let mut lobby = lobby(LobbyRole::Client, vec![client(1, true)]);
        lobby.set_resource_rows(vec![
            LobbyResourceRow {
                id: 7,
                filename: "Network/Scenario.c4s".into(),
                present_percent: 100,
                save_possible: true,
            },
            LobbyResourceRow {
                id: 9,
                filename: "Network/System.c4g".into(),
                present_percent: 100,
                save_possible: false,
            },
        ]);
        lobby.set_active_sheet(LobbySheet::Resources);
        let layout = lobby.layout(1280, 720, &fonts);
        let roster = lobby.roster_layout(&layout, fonts.text.line_height);
        let save = roster.rows[0].save.expect("eligible row has Save");
        assert_eq!(save.x, roster.rows[0].rect.x + roster.rows[0].rect.w - 18);
        assert_eq!(save.y, roster.rows[0].rect.y + 1);
        assert_eq!((save.w, save.h), (16, 16));
        assert_eq!(roster.rows[1].save, None);

        let point = GuiPoint::new((save.x + 8) as f32, (save.y + 8) as f32);
        assert!(lobby.pointer_down(point, &layout, &roster).is_empty());
        assert_eq!(
            lobby.pointer_up(point, &layout, &roster, Instant::now()),
            vec![LobbyAction::SaveResourceRequested { resource_id: 7 }]
        );
    }

    #[test]
    fn manual_preload_reserves_strip_until_success_and_activates_only_when_eligible() {
        let fonts = endeavour_font_set();
        let mut lobby = lobby(LobbyRole::Client, vec![]);
        lobby.set_resource_rows(
            (0..40)
                .map(|id| LobbyResourceRow {
                    id,
                    filename: format!("Network/Resource{id}.c4d"),
                    present_percent: 100,
                    save_possible: false,
                })
                .collect(),
        );
        lobby.set_active_sheet(LobbySheet::Resources);
        let full = lobby.layout(1280, 720, &fonts);
        let full_resources = lobby.roster_layout(&full, fonts.text.line_height);
        assert!(full_resources.max_scroll > 0);

        lobby.set_preload_button_state(true, false);
        let hidden = lobby.layout(1280, 720, &fonts);
        let hidden_resources = lobby.roster_layout(&hidden, fonts.text.line_height);
        assert_eq!(full.roster.h - hidden.roster.h, BUTTON_HEIGHT);
        assert_eq!(full.roster_client.h - hidden.roster_client.h, BUTTON_HEIGHT);
        assert_eq!(
            full.roster_scrollbar.h - hidden.roster_scrollbar.h,
            BUTTON_HEIGHT
        );
        assert_eq!(
            hidden_resources.max_scroll - full_resources.max_scroll,
            BUTTON_HEIGHT
        );
        assert_eq!(
            hidden_resources.content_height,
            full_resources.content_height
        );
        assert_eq!(hidden.preload_button, None);
        assert!(!lobby.focus_order().contains(&LobbyControl::Preload));

        lobby.set_preload_button_state(true, true);
        let visible = lobby.layout(1280, 720, &fonts);
        let button = visible.preload_button.expect("eligible preload button");
        assert_eq!(visible.roster.h, hidden.roster.h);
        assert_eq!(button.y, visible.roster.y + visible.roster.h);
        assert!(lobby.focus_order().contains(&LobbyControl::Preload));
        let roster = lobby.roster_layout(&visible, fonts.text.line_height);
        let point = GuiPoint::new((button.x + 2) as f32, (button.y + 2) as f32);
        assert!(lobby.pointer_move(point, &visible, &roster).is_empty());
        assert_eq!(
            lobby
                .tooltip_state_at(lobby.hover_since + TOOLTIP_DELAY)
                .map(|tooltip| tooltip.text),
            Some("Preload game data".into())
        );
        assert!(lobby.pointer_down(point, &visible, &roster).is_empty());
        assert_eq!(
            lobby.pointer_up(point, &visible, &roster, Instant::now()),
            [LobbyAction::PreloadRequested]
        );
        lobby.focus = LobbyControl::Preload;
        assert!(lobby
            .key_down(KeyCode::Enter, false, &visible, &roster, Instant::now())
            .is_empty());
        assert_eq!(
            lobby.key_up(KeyCode::Enter),
            [LobbyAction::PreloadRequested]
        );

        // A failed app-side request restores this same presentation.
        lobby.set_preload_button_state(true, true);
        assert!(lobby.preload_button_visible());
        assert_eq!(lobby.layout(1280, 720, &fonts).roster.h, hidden.roster.h);

        // Losing caller eligibility cancels every in-flight interaction.
        lobby.focus = LobbyControl::Preload;
        assert!(lobby
            .key_down(KeyCode::Space, false, &visible, &roster, Instant::now())
            .is_empty());
        assert!(lobby.pointer_down(point, &visible, &roster).is_empty());
        assert!(lobby.key_pressed.is_some());
        assert_eq!(lobby.pointer_pressed, Some(HitTarget::Preload));
        lobby.set_preload_button_state(true, false);
        assert_eq!(lobby.focus(), LobbyControl::ResourcesTab);
        assert!(lobby.key_pressed.is_none());
        assert!(lobby.pointer_pressed.is_none());
        assert!(!lobby.pointer_inside_pressed);
        assert_eq!(lobby.hovered, HitTarget::None);
        assert!(lobby.key_up(KeyCode::Space).is_empty());

        // Resource completion is an independent half of the same gate, even
        // when the app's CanPreload projection remains true.
        lobby.set_preload_button_state(true, true);
        lobby.focus = LobbyControl::Preload;
        assert!(lobby
            .key_down(KeyCode::Enter, false, &visible, &roster, Instant::now())
            .is_empty());
        assert!(lobby.pointer_down(point, &visible, &roster).is_empty());
        assert!(lobby.set_resources_loaded(false).is_empty());
        assert!(!lobby.preload_button_visible());
        assert_eq!(lobby.focus(), LobbyControl::ResourcesTab);
        assert!(lobby.key_pressed.is_none());
        assert!(lobby.pointer_pressed.is_none());
        assert_eq!(lobby.hovered, HitTarget::None);
        assert!(!lobby.focus_order().contains(&LobbyControl::Preload));
        assert!(lobby.activate_control(LobbyControl::Preload).is_empty());
        let incomplete = lobby.layout(1280, 720, &fonts);
        assert_eq!(incomplete.preload_button, None);
        assert_eq!(incomplete.roster.h, hidden.roster.h);

        // Success deletes the native button object and returns its strip.
        let _ = lobby.set_resources_loaded(true);
        lobby.set_preload_button_state(false, false);
        let consumed = lobby.layout(1280, 720, &fonts);
        let consumed_resources = lobby.roster_layout(&consumed, fonts.text.line_height);
        assert_eq!(consumed.roster.h, full.roster.h);
        assert_eq!(consumed.roster_client.h, full.roster_client.h);
        assert_eq!(consumed.roster_scrollbar.h, full.roster_scrollbar.h);
        assert_eq!(consumed_resources.max_scroll, full_resources.max_scroll);
        assert_eq!(consumed.preload_button, None);
    }

    #[test]
    fn core_option_rows_follow_host_client_gates_and_choices() {
        let labels = LobbyOptionLabels {
            select_template: "Choose %s".into(),
            ..LobbyOptionLabels::default()
        };
        let host = core_lobby_option_rows(LobbyRole::Host, &labels, 1, 7, true);
        assert_eq!(
            host.iter().map(|row| row.kind).collect::<Vec<_>>(),
            [
                LobbyOptionKind::ControlMode,
                LobbyOptionKind::ControlRate,
                LobbyOptionKind::RuntimeJoin,
            ]
        );
        assert_eq!(host[0].value, labels.control_mode_central);
        assert!(!host[0].editable);
        assert!(host[0].choices.is_empty());
        assert_eq!(host[1].value, "7");
        assert!(host[1].editable);
        assert_eq!(
            host[1]
                .choices
                .iter()
                .map(|choice| choice.id)
                .collect::<Vec<_>>(),
            (1..10).collect::<Vec<_>>()
        );
        assert_eq!(host[1].choices[3].tooltip, "Choose 4");
        assert_eq!(host[2].value, labels.runtime_join_free);
        assert!(host[2].editable);
        assert_eq!(
            host[2]
                .choices
                .iter()
                .map(|choice| (choice.id, choice.label.as_str()))
                .collect::<Vec<_>>(),
            [
                (0, labels.runtime_join_barred.as_str()),
                (1, labels.runtime_join_free.as_str()),
            ]
        );

        let client = core_lobby_option_rows(LobbyRole::Client, &labels, 2, 20, false);
        assert_eq!(client.len(), 2);
        assert_eq!(client[0].value, labels.control_mode_async);
        assert!(client.iter().all(|row| !row.editable));
        assert!(!client
            .iter()
            .any(|row| row.kind == LobbyOptionKind::RuntimeJoin));
        assert_eq!(client[1].value, "20");
    }

    #[test]
    fn runtime_option_rows_follow_control_and_network_host_gates() {
        let labels = LobbyOptionLabels::default();
        let host = core_runtime_option_rows(true, true, false, &labels, 1, 4, true);
        assert_eq!(
            host.iter().map(|row| row.kind).collect::<Vec<_>>(),
            [
                LobbyOptionKind::ControlMode,
                LobbyOptionKind::ControlRate,
                LobbyOptionKind::RuntimeJoin,
            ]
        );
        assert!(host.iter().all(|row| row.editable));
        assert_eq!(
            host[0]
                .choices
                .iter()
                .map(|choice| choice.id)
                .collect::<Vec<_>>(),
            [1, 0, 2]
        );
        assert_eq!(
            host[1]
                .choices
                .iter()
                .map(|choice| choice.id)
                .collect::<Vec<_>>(),
            (1..10).collect::<Vec<_>>()
        );
        assert_eq!(host[2].value, labels.runtime_join_free);

        let league = core_runtime_option_rows(true, true, true, &labels, 2, 20, false);
        assert_eq!(
            league[0]
                .choices
                .iter()
                .map(|choice| choice.id)
                .collect::<Vec<_>>(),
            [1, 0]
        );
        assert_eq!(league[0].value, labels.control_mode_async);
        assert_eq!(league[1].value, "20");

        let client = core_runtime_option_rows(false, false, false, &labels, 0, 3, false);
        assert_eq!(client.len(), 2);
        assert!(client.iter().all(|row| !row.editable));
        assert!(!client
            .iter()
            .any(|row| row.kind == LobbyOptionKind::RuntimeJoin));

        let control_host_client =
            core_runtime_option_rows(true, false, false, &labels, 1, 4, false);
        assert!(control_host_client.iter().all(|row| row.editable));
        assert!(!control_host_client
            .iter()
            .any(|row| row.kind == LobbyOptionKind::RuntimeJoin));
    }

    #[test]
    fn team_option_rows_follow_scenario_role_and_choice_gates() {
        let labels = LobbyOptionLabels::default();
        let mut state = LobbyTeamOptionState {
            active: false,
            auto_generate_teams: false,
            distribution: 1,
            team_colors: true,
            random_team_count: 0,
            active_player_count: 2,
            team_count: 4,
        };
        assert!(team_lobby_option_rows(LobbyRole::Host, &labels, state).is_empty());

        state.active = true;
        let host = team_lobby_option_rows(LobbyRole::Host, &labels, state);
        assert_eq!(
            host.iter().map(|row| row.kind).collect::<Vec<_>>(),
            [
                LobbyOptionKind::TeamDistribution,
                LobbyOptionKind::TeamColors,
            ]
        );
        assert!(host.iter().all(|row| row.editable));
        assert_eq!(host[0].value, labels.team_distribution_host);
        assert_eq!(
            host[0]
                .choices
                .iter()
                .map(|choice| choice.id)
                .collect::<Vec<_>>(),
            [0, 1, 3, 4]
        );
        assert_eq!(
            host[1]
                .choices
                .iter()
                .map(|choice| choice.id)
                .collect::<Vec<_>>(),
            [1, 0]
        );
        assert_eq!(host[1].value, labels.enabled);

        state.distribution = 3;
        let client = team_lobby_option_rows(LobbyRole::Client, &labels, state);
        assert_eq!(client.len(), 2);
        assert!(client.iter().all(|row| !row.editable));
        assert!(!client
            .iter()
            .any(|row| row.kind == LobbyOptionKind::RandomTeamCount));
    }

    #[test]
    fn random_team_count_row_tracks_both_random_modes_and_native_ranges() {
        let labels = LobbyOptionLabels::default();
        let mut state = LobbyTeamOptionState {
            active: true,
            auto_generate_teams: false,
            distribution: 3,
            team_colors: false,
            random_team_count: 0,
            active_player_count: 2,
            team_count: 4,
        };
        let fixed = team_lobby_option_rows(LobbyRole::Host, &labels, state);
        let random = fixed
            .iter()
            .find(|row| row.kind == LobbyOptionKind::RandomTeamCount)
            .expect("Random distribution appends the team-count row");
        assert_eq!(random.value, labels.automatic);
        assert_eq!(
            random
                .choices
                .iter()
                .map(|choice| choice.id)
                .collect::<Vec<_>>(),
            [0, 2, 3, 4]
        );
        assert_eq!(random.choices[0].tooltip, labels.automatic_tooltip);

        state.auto_generate_teams = true;
        state.distribution = 4;
        state.random_team_count = 3;
        state.active_player_count = 3;
        state.team_count = 8;
        let generated = team_lobby_option_rows(LobbyRole::Host, &labels, state);
        assert_eq!(
            generated[0]
                .choices
                .iter()
                .map(|choice| choice.id)
                .collect::<Vec<_>>(),
            [0, 1, 2, 3, 4]
        );
        let random = generated
            .iter()
            .find(|row| row.kind == LobbyOptionKind::RandomTeamCount)
            .expect("surprise-random is also a random team mode");
        assert_eq!(random.value, "3");
        assert_eq!(
            random
                .choices
                .iter()
                .map(|choice| choice.id)
                .collect::<Vec<_>>(),
            [0, 2, 3]
        );

        for distribution in [0, 1, 2] {
            state.distribution = distribution;
            assert!(!team_lobby_option_rows(LobbyRole::Host, &labels, state)
                .iter()
                .any(|row| row.kind == LobbyOptionKind::RandomTeamCount));
        }
        state.active = false;
        state.distribution = 3;
        assert!(team_lobby_option_rows(LobbyRole::Host, &labels, state).is_empty());
    }

    #[test]
    fn options_sheet_uses_stacked_scrollable_layout() {
        let fonts = endeavour_font_set();
        let mut empty = lobby(LobbyRole::Host, vec![]);
        empty.set_active_sheet(LobbySheet::Options);
        let empty_layout = empty.layout(1280, 720, &fonts);
        let empty_options = empty.roster_layout(&empty_layout, fonts.text.line_height);
        assert_eq!(empty_options.content_height, 0);
        assert_eq!(empty_options.max_scroll, 0);
        assert!(empty.focus_order().contains(&LobbyControl::OptionsList));
        empty.focus = LobbyControl::ScenarioTab;
        assert_eq!(
            empty.focus_next(false),
            [LobbyAction::FocusChanged(LobbyControl::OptionsList)]
        );
        assert_eq!(empty.selected_option, None);

        let mut lobby = lobby(LobbyRole::Host, vec![]);
        lobby.set_option_rows(core_lobby_option_rows(
            LobbyRole::Host,
            &LobbyOptionLabels::default(),
            0,
            3,
            false,
        ));
        lobby.set_active_sheet(LobbySheet::Options);
        assert!(lobby.option_sheet_active());

        let line_height = fonts.text.line_height;
        let row_height = 2 * line_height + 8;
        let mut layout = lobby.layout(1280, 720, &fonts);
        assert_eq!(layout.roster_client.x, layout.roster.x + LIST_BOX_MARGIN);
        assert_eq!(layout.roster_client.y, layout.roster.y + LIST_BOX_MARGIN);
        assert_eq!(
            layout.roster_client.w + layout.roster_scrollbar.w,
            layout.roster.w - 2 * LIST_BOX_MARGIN
        );
        assert_eq!(
            layout.roster_scrollbar.x + layout.roster_scrollbar.w,
            layout.roster.x + layout.roster.w - LIST_BOX_MARGIN
        );
        layout.roster_client.h = 2 * row_height;
        let options = lobby.roster_layout(&layout, line_height);
        assert_eq!(options.content_height, 3 * row_height + 2);
        assert_eq!(options.max_scroll, row_height + 2);
        assert_eq!(options.rows[0].rect.h, row_height);
        assert_eq!(
            options.rows[1].rect.y,
            options.rows[0].rect.y + row_height + 1
        );
        let combo = options.rows[0].option_value.expect("stacked ComboBox");
        assert_eq!(combo.x, options.rows[0].rect.x + 6);
        assert_eq!(combo.y, options.rows[0].rect.y + line_height + 3);
        assert_eq!(combo.w, options.rows[0].rect.w - 7);
        assert_eq!(combo.h, line_height + 4);
        assert!(options.rows.iter().all(|row| {
            row.icon.w == 0
                && row.add_player.is_none()
                && row.team.is_none()
                && row.option_value.is_some()
                && row.rank.is_none()
        }));

        lobby.selected_option = Some(LobbyOptionKind::ControlRate);
        lobby.focus = LobbyControl::OptionsList;
        assert!(lobby.move_option_selection(1, &layout, &options).is_empty());
        assert_eq!(lobby.selected_option, Some(LobbyOptionKind::RuntimeJoin));
        assert_eq!(lobby.option_scroll(), options.max_scroll);
        lobby.set_option_scroll(0);
        let options = lobby.roster_layout(&layout, line_height);

        let point = GuiPoint::new(
            (layout.roster_client.x + 1) as f32,
            (layout.roster_client.y + 1) as f32,
        );
        assert!(lobby.wheel(point, -7, &layout, &options));
        assert_eq!(lobby.option_scroll(), 7);
        assert_eq!(lobby.resource_scroll(), 0);
        assert_eq!(lobby.roster_scroll(), 0);
    }

    #[test]
    fn option_combo_pointer_keyboard_gamepad_and_open_presentation() {
        let fonts = endeavour_font_set();
        let labels = LobbyOptionLabels::default();
        let mut lobby = lobby(LobbyRole::Host, vec![]);
        lobby.set_option_rows(core_lobby_option_rows(
            LobbyRole::Host,
            &labels,
            0,
            3,
            false,
        ));
        lobby.set_active_sheet(LobbySheet::Options);
        let layout = lobby.layout(1280, 720, &fonts);
        let options = lobby.roster_layout(&layout, fonts.text.line_height);
        let order = lobby.focus_order();
        assert!(order.contains(&LobbyControl::OptionsList));
        assert!(!order
            .iter()
            .any(|control| matches!(control, LobbyControl::Option(_))));
        lobby.focus = LobbyControl::ScenarioTab;
        assert_eq!(
            lobby.focus_next(false),
            [LobbyAction::FocusChanged(LobbyControl::OptionsList)]
        );
        assert_eq!(lobby.selected_option, Some(LobbyOptionKind::ControlMode));
        let order = lobby.focus_order();
        assert!(order.contains(&LobbyControl::OptionsList));
        assert!(order.contains(&LobbyControl::Option(LobbyOptionKind::ControlMode)));
        assert!(!order.contains(&LobbyControl::Option(LobbyOptionKind::ControlRate)));
        assert!(!order.contains(&LobbyControl::Option(LobbyOptionKind::RuntimeJoin)));
        lobby.focus = LobbyControl::Option(LobbyOptionKind::ControlMode);
        assert_eq!(
            lobby.key_down(KeyCode::Down, false, &layout, &options, Instant::now()),
            [LobbyAction::FocusChanged(LobbyControl::OptionsList)]
        );
        assert_eq!(lobby.selected_option, Some(LobbyOptionKind::ControlRate));

        let mode_combo = options.rows[0].option_value.expect("mode value");
        let mode_point = GuiPoint::new((mode_combo.x + 1) as f32, (mode_combo.y + 1) as f32);
        assert_eq!(
            lobby.pointer_down(mode_point, &layout, &options),
            [LobbyAction::FocusChanged(LobbyControl::Option(
                LobbyOptionKind::ControlMode
            ))]
        );
        assert_eq!(lobby.selected_option, Some(LobbyOptionKind::ControlMode));

        let rate_combo = options.rows[1].option_value.expect("rate combo");
        let expected = LobbyAction::OptionSelectionRequested {
            option: LobbyOptionKind::ControlRate,
            anchor: GuiPoint::new(rate_combo.x as f32, (rate_combo.y + rate_combo.h) as f32),
            minimum_width: rate_combo.w,
        };
        let rate_point = GuiPoint::new((rate_combo.x + 1) as f32, (rate_combo.y + 1) as f32);
        assert_eq!(
            lobby.pointer_down(rate_point, &layout, &options),
            [
                LobbyAction::FocusChanged(LobbyControl::Option(LobbyOptionKind::ControlRate)),
                expected.clone(),
            ]
        );
        // ComboBox itself declines focus-on-click, but the parent Option
        // control and ListBox selection retain focus for the active row.
        assert_eq!(
            lobby.focus(),
            LobbyControl::Option(LobbyOptionKind::ControlRate)
        );
        assert_eq!(lobby.selected_option, Some(LobbyOptionKind::ControlRate));
        lobby.change_focus(LobbyControl::ChatInput, false);
        assert_eq!(lobby.selected_option, Some(LobbyOptionKind::ControlRate));

        let order = lobby.focus_order();
        assert!(order.contains(&LobbyControl::OptionsList));
        assert!(!order.contains(&LobbyControl::Option(LobbyOptionKind::ControlMode)));
        assert!(order.contains(&LobbyControl::Option(LobbyOptionKind::ControlRate)));
        assert!(!order.contains(&LobbyControl::Option(LobbyOptionKind::RuntimeJoin)));
        lobby.focus = LobbyControl::Option(LobbyOptionKind::ControlRate);
        assert_eq!(
            lobby.key_down(KeyCode::Down, false, &layout, &options, Instant::now()),
            std::slice::from_ref(&expected)
        );
        assert_eq!(
            lobby.gamepad_low_down(Instant::now(), &layout, &options),
            std::slice::from_ref(&expected)
        );
        assert_eq!(lobby.gamepad_direction(0, 1, &layout, &options), [expected]);

        lobby.set_open_option_combo(Some(LobbyOptionKind::ControlRate));
        assert_eq!(
            lobby.open_option_combo(),
            Some(LobbyOptionKind::ControlRate)
        );
        lobby.set_open_option_combo(Some(LobbyOptionKind::ControlMode));
        assert_eq!(lobby.open_option_combo(), None);
        lobby.set_open_option_combo(Some(LobbyOptionKind::RuntimeJoin));
        lobby.set_active_sheet(LobbySheet::Players);
        assert_eq!(lobby.open_option_combo(), None);

        lobby.set_active_sheet(LobbySheet::Options);
        let layout = lobby.layout(1280, 720, &fonts);
        let options = lobby.roster_layout(&layout, fonts.text.line_height);
        let caption_point = GuiPoint::new(
            (options.rows[2].rect.x + 1) as f32,
            (options.rows[2].rect.y + 1) as f32,
        );
        assert_eq!(
            lobby.pointer_down(caption_point, &layout, &options),
            [LobbyAction::FocusChanged(LobbyControl::Option(
                LobbyOptionKind::RuntimeJoin
            ))]
        );
        assert_eq!(lobby.selected_option, Some(LobbyOptionKind::RuntimeJoin));
        let _ = lobby.pointer_move(caption_point, &layout, &options);
        assert_eq!(
            lobby
                .tooltip_state_at(lobby.hover_since + TOOLTIP_DELAY)
                .map(|tooltip| tooltip.text),
            Some(labels.runtime_join_tooltip)
        );
        let blank_point = GuiPoint::new(
            (layout.roster_client.x + 1) as f32,
            (layout.roster_client.y + layout.roster_client.h - 1) as f32,
        );
        assert_eq!(
            lobby.pointer_down(blank_point, &layout, &options),
            [LobbyAction::FocusChanged(LobbyControl::OptionsList)]
        );
        assert_eq!(lobby.selected_option, None);
    }

    #[test]
    fn active_sheet_drives_caption_tab_highlight_and_resource_activation() {
        let fonts = endeavour_font_set();
        let mut lobby = GameLobby::new(
            LobbyRole::Host,
            "Gold Mine",
            1,
            4,
            true,
            false,
            true,
            false,
            5,
            vec![],
        );
        assert_eq!(lobby.active_sheet(), LobbySheet::Players);
        assert!(!lobby.resource_sheet_active());
        assert_eq!(lobby.right_title(), "&Players (1/4)");

        lobby.set_active_sheet(LobbySheet::Resources);
        assert!(lobby.resource_sheet_active());
        assert_eq!(lobby.right_title(), "&Resources");
        let layout = lobby.layout(1280, 720, &fonts);
        assert_eq!(
            layout
                .tab_buttons
                .iter()
                .filter(|tab| tab.selected)
                .map(|tab| tab.sheet)
                .collect::<Vec<_>>(),
            [Some(LobbySheet::Resources)]
        );

        lobby.set_active_sheet(LobbySheet::Options);
        assert!(!lobby.resource_sheet_active());
        assert_eq!(lobby.right_title(), "&Options");
        lobby.set_active_sheet(LobbySheet::Scenario);
        assert_eq!(lobby.right_title(), "&Scenario");
        lobby.set_active_sheet(LobbySheet::Teams);
        assert_eq!(lobby.right_title(), "&Players (1/4)");
        let layout = lobby.layout(1280, 720, &fonts);
        assert_eq!(
            layout
                .tab_buttons
                .iter()
                .filter(|tab| tab.selected)
                .map(|tab| tab.sheet)
                .collect::<Vec<_>>(),
            [Some(LobbySheet::Teams)]
        );
    }

    #[test]
    fn scenario_sheet_uses_text_window_geometry_and_scrolls_overflow() {
        let fonts = endeavour_font_set();
        let mut lobby = lobby(LobbyRole::Host, vec![]);
        lobby.set_active_sheet(LobbySheet::Scenario);
        lobby.set_scenario_text(LobbyScenarioText::Description(
            (0..80)
                .map(|index| format!("Paragraph {index}"))
                .collect::<Vec<_>>()
                .join("\n"),
        ));

        let layout = lobby.layout(640, 300, &fonts);
        assert_eq!(layout.roster_client.x, layout.roster.x + 10);
        assert_eq!(layout.roster_client.y, layout.roster.y + 8);
        assert_eq!(
            layout.roster_client.w,
            layout.roster.w - 10 - 5 - SCROLLBAR_EXTENT
        );
        assert_eq!(layout.roster_client.h, layout.roster.h - 16);
        assert_eq!(
            layout.roster_scrollbar.x,
            layout.roster.x + layout.roster.w - 5 - SCROLLBAR_EXTENT
        );
        assert_eq!(layout.roster_scrollbar.w, SCROLLBAR_EXTENT);

        let scenario = lobby.right_list_layout(&layout, &fonts);
        assert!(scenario.rows.is_empty());
        assert!(scenario.max_scroll > 0);
        let point = GuiPoint::new(
            (layout.roster_client.x + 1) as f32,
            (layout.roster_client.y + 1) as f32,
        );
        assert!(lobby.wheel(point, -10, &layout, &scenario));
        assert_eq!(lobby.scenario_scroll(), 10);

        lobby.set_scenario_text(LobbyScenarioText::Title("Gold Mine".to_string()));
        let scenario = lobby.right_list_layout(&layout, &fonts);
        assert_eq!(scenario.max_scroll, 0);
        assert_eq!(lobby.scenario_scroll(), 0);
        assert!(lobby.wrapped_scenario_lines(&layout, &fonts)[0].title_font);

        lobby.set_scenario_text(LobbyScenarioText::Description(
            "One-line description".to_string(),
        ));
        assert!(!lobby.wrapped_scenario_lines(&layout, &fonts)[0].title_font);
    }

    #[test]
    fn team_header_double_click_requests_one_bulk_move_and_random_header_is_inert() {
        let fonts = endeavour_font_set();
        let mut lobby = GameLobby::new(
            LobbyRole::Host,
            "Gold Mine",
            2,
            4,
            true,
            false,
            true,
            false,
            5,
            vec![header(LobbyRosterHeader::Team(7), false), player(2)],
        );
        lobby.set_active_sheet(LobbySheet::Teams);
        let layout = lobby.layout(1280, 720, &fonts);
        let roster = lobby.roster_layout(&layout, fonts.text.line_height);
        let point = GuiPoint::new(
            (roster.rows[0].rect.x + 1) as f32,
            (roster.rows[0].rect.y + 1) as f32,
        );
        lobby.pointer_down(point, &layout, &roster);
        assert_eq!(
            lobby.accepted_roster_click_id(point, &layout, &roster),
            Some(LobbyRosterId::Header(LobbyRosterHeader::Team(7)))
        );
        assert_eq!(
            lobby.pointer_double_click(point, &layout, &roster),
            [LobbyAction::MoveLocalPlayersIntoTeamRequested { team_id: 7 }]
        );

        lobby.set_rows(vec![header(LobbyRosterHeader::Team(8), false)]);
        assert_eq!(
            lobby.accepted_roster_click_id(point, &layout, &roster),
            None,
            "a row replacement between down and up must invalidate the semantic click"
        );
        lobby.cancel_interaction();

        lobby.set_rows(vec![header(LobbyRosterHeader::RandomTeam, false)]);
        let roster = lobby.roster_layout(&layout, fonts.text.line_height);
        let point = GuiPoint::new(
            (roster.rows[0].rect.x + 1) as f32,
            (roster.rows[0].rect.y + 1) as f32,
        );
        assert!(lobby
            .pointer_double_click(point, &layout, &roster)
            .is_empty());
    }

    #[test]
    fn semantic_roster_capture_survives_selection_induced_collapse_reflow() {
        let fonts = endeavour_font_set();
        let mut lobby = GameLobby::new(
            LobbyRole::Client,
            "Gold Mine",
            1,
            4,
            true,
            false,
            true,
            false,
            5,
            vec![player(2), header(LobbyRosterHeader::Team(7), false)],
        );
        lobby.set_active_sheet(LobbySheet::Teams);
        lobby.collapsed_roster = true;
        lobby.collapse_player_limit = 0;
        lobby.selected_row = Some(0);
        lobby.selected_roster_id = Some(LobbyRosterId::Player(2));

        let layout = lobby.layout(640, 480, &fonts);
        let roster = lobby.roster_layout(&layout, fonts.text.line_height);
        let team = roster.rows[1].rect;
        let point = GuiPoint::new((team.x + 1) as f32, (team.y + 1) as f32);
        lobby.pointer_down(point, &layout, &roster);
        assert_eq!(
            lobby.selected_roster_id(),
            Some(&LobbyRosterId::Header(LobbyRosterHeader::Team(7)))
        );

        let reflowed = lobby.roster_layout(&layout, fonts.text.line_height);
        let clicked = lobby
            .accepted_roster_click_id(point, &layout, &reflowed)
            .expect("captured semantic team survives the selected-player collapse");
        assert_eq!(clicked, LobbyRosterId::Header(LobbyRosterHeader::Team(7)));
        assert_eq!(
            lobby.roster_double_click(&clicked),
            [LobbyAction::MoveLocalPlayersIntoTeamRequested { team_id: 7 }]
        );
    }

    #[test]
    fn semantic_add_capture_and_hover_survive_authoritative_reordering() {
        let fonts = endeavour_font_set();
        let mut lobby = lobby(LobbyRole::Host, vec![client(1, true), client(2, true)]);
        let layout = lobby.layout(640, 480, &fonts);
        let roster = lobby.roster_layout(&layout, fonts.text.line_height);
        let add = roster.rows[0].add_player.expect("first Add Player button");
        let point = GuiPoint::new((add.x + 1) as f32, (add.y + 1) as f32);

        lobby.pointer_move(point, &layout, &roster);
        lobby.pointer_down(point, &layout, &roster);
        let hover_started = lobby.hover_since;
        lobby.set_rows(vec![client(2, true), client(1, true)]);
        assert_eq!(lobby.hovered, HitTarget::AddPlayer(1));
        assert_eq!(lobby.pointer_pressed, Some(HitTarget::AddPlayer(1)));
        assert_eq!(lobby.hover_since, hover_started);
        assert!(lobby
            .tooltip_state_at(hover_started + TOOLTIP_DELAY)
            .is_some_and(|tooltip| tooltip.text.contains("Client 1")));

        let reordered = lobby.roster_layout(&layout, fonts.text.line_height);
        assert!(lobby
            .pointer_up(point, &layout, &reordered, Instant::now())
            .is_empty());
        let moved_add = reordered.rows[1]
            .add_player
            .expect("moved semantic Add Player button");
        let moved_point = GuiPoint::new((moved_add.x + 1) as f32, (moved_add.y + 1) as f32);
        lobby.pointer_down(moved_point, &layout, &reordered);
        assert_eq!(
            lobby.pointer_up(moved_point, &layout, &reordered, Instant::now()),
            [LobbyAction::AddPlayerRequested { client_id: 1 }]
        );

        lobby.set_rows(vec![client(1, true), client(2, true)]);
        let roster = lobby.roster_layout(&layout, fonts.text.line_height);
        let add = roster.rows[0]
            .add_player
            .expect("restored Add Player button");
        let point = GuiPoint::new((add.x + 1) as f32, (add.y + 1) as f32);
        lobby.pointer_down(point, &layout, &roster);
        let _ = lobby.take_sounds();
        lobby.pointer_move(GuiPoint::new(0.0, 0.0), &layout, &roster);
        assert_eq!(lobby.take_sounds(), [LobbySound::ArrowHit]);
        lobby.pointer_move(point, &layout, &roster);
        assert_eq!(lobby.take_sounds(), [LobbySound::ArrowHit]);
        lobby.set_rows(vec![client(2, true)]);
        assert_eq!(lobby.take_sounds(), [LobbySound::ArrowHit]);
        let removed = lobby.roster_layout(&layout, fonts.text.line_height);
        assert!(lobby
            .pointer_up(point, &layout, &removed, Instant::now())
            .is_empty());
    }

    #[test]
    fn authoritative_refresh_clears_removed_roster_child_key_latch() {
        let fonts = endeavour_font_set();
        let mut lobby = lobby(LobbyRole::Host, vec![client(1, true)]);
        lobby.selected_row = Some(0);
        lobby.selected_roster_id = Some(LobbyRosterId::Client(1));
        lobby.focus = LobbyControl::RosterAddPlayer;
        let layout = lobby.layout(640, 480, &fonts);
        let roster = lobby.roster_layout(&layout, fonts.text.line_height);

        assert!(lobby
            .key_down(KeyCode::Space, false, &layout, &roster, Instant::now())
            .is_empty());
        assert_eq!(
            lobby.key_pressed,
            Some((LobbyControl::RosterAddPlayer, KeyCode::Space))
        );

        lobby.set_rows(vec![client(1, false)]);
        assert_eq!(lobby.focus(), LobbyControl::Roster);
        assert_eq!(lobby.key_pressed, None);
        assert!(lobby.key_up(KeyCode::Space).is_empty());
    }

    #[test]
    fn non_roster_sheets_cannot_select_or_open_roster_rows() {
        let fonts = endeavour_font_set();
        let mut lobby = lobby(LobbyRole::Host, vec![client(1, true), player(2)]);
        let layout = lobby.layout(640, 300, &fonts);
        let roster = lobby.roster_layout(&layout, fonts.text.line_height);
        let roster_point = GuiPoint::new(
            (roster.rows[0].rect.x + 1) as f32,
            (roster.rows[0].rect.y + 1) as f32,
        );
        assert!(lobby
            .pointer_down(roster_point, &layout, &roster)
            .iter()
            .any(|action| matches!(action, LobbyAction::RosterSelectionChanged(_))));
        assert_eq!(lobby.selected_roster_id(), Some(&LobbyRosterId::Client(1)));

        lobby.set_resource_rows(
            (0..40)
                .map(|id| LobbyResourceRow {
                    id,
                    filename: format!("Network/Resource{id}.c4d"),
                    present_percent: 50,
                    save_possible: false,
                })
                .collect(),
        );
        lobby.set_active_sheet(LobbySheet::Resources);
        assert_eq!(lobby.focus(), LobbyControl::ChatInput);
        let layout = lobby.layout(640, 300, &fonts);
        let resources = lobby.roster_layout(&layout, fonts.text.line_height);
        assert!(resources.max_scroll > 0);
        let resource_point = GuiPoint::new(
            (resources.rows[0].rect.x + 1) as f32,
            (resources.rows[0].rect.y + 1) as f32,
        );
        assert!(lobby
            .pointer_down(resource_point, &layout, &resources)
            .is_empty());
        assert!(lobby
            .pointer_secondary_down(resource_point, &layout, &resources)
            .is_empty());
        assert_eq!(lobby.selected_roster_id(), Some(&LobbyRosterId::Client(1)));
        assert_eq!(lobby.focus(), LobbyControl::ChatInput);
        assert!(lobby.wheel(resource_point, -10, &layout, &resources));
        assert_eq!(lobby.resource_scroll(), 10);
        assert_eq!(lobby.roster_scroll(), 0);

        lobby.set_active_sheet(LobbySheet::Options);
        let layout = lobby.layout(640, 300, &fonts);
        let empty = lobby.roster_layout(&layout, fonts.text.line_height);
        assert!(empty.rows.is_empty());
        assert_eq!(
            lobby.pointer_down(resource_point, &layout, &empty),
            [LobbyAction::FocusChanged(LobbyControl::OptionsList)]
        );
        assert!(lobby.request_focused_context(resource_point).is_empty());
        assert_eq!(lobby.selected_roster_id(), Some(&LobbyRosterId::Client(1)));
    }

    #[test]
    fn overflowing_roster_collapses_only_unselected_players() {
        let rows = std::iter::once(client(1, true))
            .chain((0..20).map(player))
            .collect();
        let mut lobby = lobby(LobbyRole::Host, rows);
        let layout = game_lobby_layout(640, 300, 34, 22, LobbyRole::Host, false, false);
        let roster = lobby.roster_layout(&layout, 22);
        assert!(roster.collapsed);
        assert!(roster.rows[1..].iter().all(|row| row.collapsed));
        lobby.selected_row = Some(5);
        let roster = lobby.roster_layout(&layout, 22);
        assert!(!roster.rows[5].collapsed);
        assert!(roster.rows[4].collapsed);
    }

    #[test]
    fn recursive_tabs_and_contexts_are_typed_requests() {
        let mut lobby = lobby(LobbyRole::Host, vec![client(1, true)]);
        let layout = game_lobby_layout(1280, 720, 34, 22, LobbyRole::Host, false, false);
        let roster = lobby.roster_layout(&layout, 22);
        let options = layout.tab_buttons[2].rect;
        let point = GuiPoint::new((options.x + 2) as f32, (options.y + 2) as f32);
        let _ = lobby.pointer_down(point, &layout, &roster);
        assert_eq!(
            lobby.pointer_up(point, &layout, &roster, Instant::now()),
            [LobbyAction::SheetRequested(LobbySheet::Options)]
        );
        let row_point = GuiPoint::new(layout.roster_client.x as f32, layout.roster_client.y as f32);
        assert!(matches!(
            lobby
                .pointer_secondary_down(row_point, &layout, &roster)
                .as_slice(),
            [LobbyAction::RosterContextRequested {
                row: LobbyRosterId::Client(1),
                ..
            }]
        ));
    }

    #[test]
    fn team_combo_and_option_strip_preserve_typed_child_routing() {
        let mut lobby = GameLobby::new(
            LobbyRole::Host,
            "Gold Mine",
            1,
            4,
            true,
            false,
            true,
            false,
            5,
            vec![client(1, true), team_player(2)],
        );
        let layout = game_lobby_layout(1280, 720, 34, 22, LobbyRole::Host, true, false);
        let roster = lobby.roster_layout(&layout, 22);
        let team = roster.rows[1].team.expect("expanded team combo");
        let team_point = GuiPoint::new((team.x + 1) as f32, (team.y + 1) as f32);
        assert_eq!(
            lobby.pointer_down(team_point, &layout, &roster),
            [
                LobbyAction::FocusChanged(LobbyControl::Roster),
                LobbyAction::RosterSelectionChanged(Some(LobbyRosterId::Player(2))),
                LobbyAction::TeamSelectionRequested { player_id: 2 }
            ]
        );

        let option = game_option_buttons_layout(
            layout.game_option_strip,
            LobbyRole::Host.game_option_context(),
        )
        .buttons[0]
            .rect;
        let option_point = GuiPoint::new((option.x + 1) as f32, (option.y + 1) as f32);
        assert!(matches!(
            lobby
                .pointer_down(option_point, &layout, &roster)
                .as_slice(),
            [LobbyAction::GameOptions(LobbyGameOptionInput::PointerDown(
                _
            ))]
        ));
        let outside = GuiPoint::new(0.0, 0.0);
        assert!(matches!(
            lobby.pointer_move(outside, &layout, &roster).as_slice(),
            [
                LobbyAction::GameOptions(LobbyGameOptionInput::MouseLeave),
                LobbyAction::GameOptions(LobbyGameOptionInput::PointerMove(_))
            ]
        ));
        assert!(matches!(
            lobby
                .pointer_up(outside, &layout, &roster, Instant::now())
                .as_slice(),
            [LobbyAction::GameOptions(LobbyGameOptionInput::PointerUp(_))]
        ));
    }

    #[test]
    fn ready_is_square_only_disabled_while_loading_and_rate_limited() {
        let mut lobby = lobby(LobbyRole::Client, vec![]);
        let _ = lobby.set_resources_loaded(false);
        let layout = game_lobby_layout(1280, 720, 34, 22, LobbyRole::Client, false, false);
        let roster = lobby.roster_layout(&layout, 22);
        let square = GuiPoint::new(layout.ready_square.x as f32, layout.ready_square.y as f32);
        assert!(lobby
            .pointer_up(square, &layout, &roster, Instant::now())
            .is_empty());
        let _ = lobby.set_resources_loaded(true);
        let now = Instant::now();
        let _ = lobby.pointer_down(square, &layout, &roster);
        assert_eq!(
            lobby.pointer_up(square, &layout, &roster, now),
            [LobbyAction::ReadyChanged(true)]
        );
        let _ = lobby.pointer_down(square, &layout, &roster);
        assert!(lobby.pointer_up(square, &layout, &roster, now).is_empty());
        let label = GuiPoint::new(
            (layout.ready_square.x + layout.ready_square.w + 8) as f32,
            layout.ready_square.y as f32,
        );
        assert!(lobby
            .pointer_up(label, &layout, &roster, now + READY_COOLDOWN)
            .is_empty());
    }

    #[test]
    fn countdown_transitions_match_messages_and_sound_edges() {
        let mut lobby = lobby(LobbyRole::Host, vec![]);
        let actions = lobby.apply_countdown_packet(LobbyCountdownPacket::Seconds(12));
        assert!(actions.contains(&LobbyAction::NotifyUserIfInactive));
        assert_eq!(lobby.take_sounds(), [LobbySound::CountdownCommand]);
        let _ = lobby.apply_countdown_packet(LobbyCountdownPacket::Seconds(10));
        assert_eq!(
            lobby.take_sounds(),
            [
                LobbySound::Fuse,
                LobbySound::StartElevatorLoop,
                LobbySound::CountdownCommand,
            ]
        );
        let actions = lobby.apply_countdown_packet(LobbyCountdownPacket::Seconds(9));
        // Lobby countdown logs enter MainDlg::OnLog, whose AddTextLine call
        // enables readable-on-black conversion (src/C4GameLobby.cpp:738-753;
        // src/C4GuiLabels.cpp:293-299).
        assert!(matches!(
            actions.last(),
            Some(LobbyAction::AppendLog(LobbyLogLine { text, color }))
                if text == "9..." && *color == [255, 32, 32, 255]
        ));
        let _ = lobby.apply_countdown_packet(LobbyCountdownPacket::Abort);
        assert_eq!(
            lobby.take_sounds(),
            [
                LobbySound::CountdownCommand,
                LobbySound::StopElevatorLoop,
                LobbySound::Pshshsh
            ]
        );

        let logs_before = lobby.logs.len();
        let idle_abort = lobby.apply_countdown_packet(LobbyCountdownPacket::Abort);
        assert_eq!(
            idle_abort,
            [LobbyAction::CountdownChanged(LobbyCountdownState::None)]
        );
        assert_eq!(lobby.logs.len(), logs_before);
    }

    #[test]
    fn exact_resource_shape_validation_rejects_substitutes() {
        let substitute = ImageData::new(1, 1, vec![0, 0, 0, 0]);
        let error = validate_exact("GUIIcons.png", &substitute, 240, 360)
            .expect_err("generic icon sheet must fail");
        assert!(error.to_string().contains("exact 240x360"));
    }

    #[test]
    fn full_size_scenario_button_highlight_is_valid() {
        let fonts = endeavour_font_set();
        let caption = load_graphics_png("GUICaption.png");
        let button = load_graphics_png("GUIButton.png");
        let button_down = load_graphics_png("GUIButtonDown.png");
        let icons = load_graphics_png("GUIIcons.png");
        let icons_extended = load_graphics_png("GUIIcons2.png");
        let checkbox = load_graphics_png("GUICheckbox.png");
        let scroll = load_graphics_png("GUIScroll.png");
        let context = load_graphics_png("GUIContext.png");
        let highlight = ImageData::new(30, 30, vec![0xff; 30 * 30 * 4]);

        // C4GUI::Resource::Load passes C4FCT_Full for GUIButtonHighlight, so
        // C4FacetExSurface::Load retains the complete override dimensions
        // (src/C4Gui.cpp:1093; src/C4FacetEx.cpp:137-161).
        LobbyResources::new(
            &fonts,
            &fonts.text,
            &caption,
            &button,
            &button_down,
            &icons,
            &icons_extended,
            &highlight,
            &checkbox,
            &scroll,
            &context,
        )
        .expect("MarsClonk's 30x30 highlight is a valid full-size facet");
    }

    #[test]
    fn full_cancel_clears_key_pointer_hover_and_scroll_latches() {
        let mut lobby = lobby(LobbyRole::Host, vec![client(0, true)]);
        let layout = game_lobby_layout(640, 480, 34, 22, LobbyRole::Host, false, false);
        let roster = lobby.roster_layout(&layout, 22);
        lobby.focus = LobbyControl::Exit;
        assert!(lobby
            .key_down(KeyCode::Enter, false, &layout, &roster, Instant::now())
            .is_empty());
        assert!(lobby.key_pressed.is_some());
        let _ = lobby.take_sounds();
        lobby.pointer = Some(GuiPoint::new(10.0, 10.0));
        lobby.hovered = HitTarget::Exit;
        lobby.pointer_pressed = Some(HitTarget::Exit);
        lobby.pointer_inside_pressed = true;
        lobby.scrollbar_drag = Some(ScrollbarDrag::Chat);

        lobby.cancel_interaction();

        assert!(lobby.key_pressed.is_none());
        assert!(lobby.pointer.is_none());
        assert_eq!(lobby.hovered, HitTarget::None);
        assert!(lobby.pointer_pressed.is_none());
        assert!(!lobby.pointer_inside_pressed);
        assert!(lobby.scrollbar_drag.is_none());
        assert!(lobby.key_up(KeyCode::Enter).is_empty());
        assert_eq!(lobby.take_sounds(), [LobbySound::ArrowHit]);
    }

    #[test]
    fn ordinary_pointer_leave_preserves_keyboard_button_latch() {
        let mut lobby = lobby(LobbyRole::Host, vec![client(0, true)]);
        let layout = game_lobby_layout(640, 480, 34, 22, LobbyRole::Host, false, false);
        let roster = lobby.roster_layout(&layout, 22);
        lobby.focus = LobbyControl::Exit;
        assert!(lobby
            .key_down(KeyCode::Enter, false, &layout, &roster, Instant::now())
            .is_empty());
        lobby.pointer = Some(GuiPoint::new(10.0, 10.0));
        lobby.hovered = HitTarget::Exit;
        lobby.pointer_left();

        assert!(lobby.pointer.is_none());
        assert_eq!(lobby.hovered, HitTarget::None);
        assert!(lobby.key_pressed.is_some());
        assert_eq!(lobby.key_up(KeyCode::Enter), [LobbyAction::ExitRequested]);
    }

    #[test]
    fn pointer_leave_releases_shared_button_and_scroll_arrows_but_preserves_thumb_drag() {
        let mut lobby = lobby(LobbyRole::Host, vec![client(0, true)]);
        let layout = game_lobby_layout(640, 480, 34, 22, LobbyRole::Host, false, false);
        let roster = lobby.roster_layout(&layout, 22);
        lobby.focus = LobbyControl::Exit;
        let _ = lobby.key_down(KeyCode::Enter, false, &layout, &roster, Instant::now());
        let _ = lobby.take_sounds();
        lobby.pointer_pressed = Some(HitTarget::Exit);
        lobby.pointer_inside_pressed = true;
        lobby.pointer_left();
        assert!(lobby.key_pressed.is_none());
        assert_eq!(lobby.pointer_pressed, Some(HitTarget::Exit));
        assert!(!lobby.pointer_inside_pressed);
        assert_eq!(lobby.take_sounds(), [LobbySound::ArrowHit]);

        lobby.pointer_pressed = Some(HitTarget::ChatScrollTop);
        lobby.pointer_left();
        assert!(lobby.pointer_pressed.is_none());

        lobby.pointer_pressed = Some(HitTarget::RosterScrollTrack);
        lobby.scrollbar_drag = Some(ScrollbarDrag::Roster);
        lobby.pointer_left();
        assert_eq!(lobby.pointer_pressed, Some(HitTarget::RosterScrollTrack));
        assert_eq!(lobby.scrollbar_drag, Some(ScrollbarDrag::Roster));
    }

    #[test]
    fn game_option_contract_uses_role_specific_context_and_exact_bounds() {
        for role in [LobbyRole::Host, LobbyRole::Client] {
            let layout = game_lobby_layout(1280, 720, 34, 22, role, false, false);
            let option_layout = crate::game_option_buttons::game_option_buttons_layout(
                layout.game_option_strip,
                role.game_option_context(),
            );
            assert_eq!(option_layout.bounds, layout.game_option_strip);
            assert_eq!(
                option_layout.buttons.len(),
                role.game_option_context().button_count()
            );
        }
    }

    #[test]
    fn fresh_focused_lobby_starts_with_the_native_visible_chat_caret() {
        // MainDlg focuses pEdtChat immediately after constructing it, and a
        // fresh Edit focus shows the caret (src/C4GameLobby.cpp:305-306;
        // src/C4GuiEdit.cpp:538-546,614-620).
        let lobby = lobby(LobbyRole::Host, vec![]);

        assert_eq!(lobby.focus(), LobbyControl::ChatInput);
        assert!(lobby.chat_edit_view().cursor_visible);
    }

    #[test]
    fn chat_wraps_caps_scrolls_and_new_messages_force_bottom() {
        let fonts = endeavour_font_set();
        let mut lobby = lobby(LobbyRole::Host, vec![]);
        let layout = game_lobby_layout(
            640,
            300,
            fonts.title.line_height,
            fonts.text.line_height,
            LobbyRole::Host,
            false,
            false,
        );
        lobby.set_logs(vec![LobbyLogLine {
            text: "first paragraph|second paragraph".into(),
            color: COLOR_WHITE,
        }]);
        let wrapped = lobby.wrapped_chat_lines(&layout, &fonts.text);
        assert!(wrapped.len() >= 2);
        assert!(wrapped[0].new_paragraph);
        assert!(wrapped.iter().skip(1).any(|line| line.new_paragraph));

        lobby.set_logs(
            (0..150)
                .map(|index| LobbyLogLine {
                    text: format!("message {index}: this is deliberately long enough to wrap"),
                    color: COLOR_WHITE,
                })
                .collect(),
        );
        let wrapped = lobby.wrapped_chat_lines(&layout, &fonts.text);
        assert!(wrapped.len() <= 100);
        assert!(
            wrapped
                .iter()
                .map(|line| line.text.len() + 1)
                .sum::<usize>()
                <= 4096
        );
        let metrics = lobby.chat_scroll_metrics(&layout, &fonts.text);
        assert!(metrics.max_scroll > 0);
        assert_eq!(metrics.scroll, metrics.max_scroll);

        let roster = lobby.roster_layout(&layout, fonts.text.line_height);
        let point = GuiPoint::new(
            (layout.chat_log_client.x + 1) as f32,
            (layout.chat_log_client.y + 1) as f32,
        );
        assert!(lobby.wheel(point, 10, &layout, &roster));
        assert!(lobby.chat_scroll() < metrics.max_scroll);
        lobby.push_log(LobbyLogLine {
            text: "new".into(),
            color: COLOR_WHITE,
        });
        let metrics = lobby.chat_scroll_metrics(&layout, &fonts.text);
        assert_eq!(metrics.scroll, metrics.max_scroll);
    }

    #[test]
    fn chat_edit_contract_covers_utf8_navigation_clipboard_context_and_refocus() {
        let mut lobby = lobby(LobbyRole::Host, vec![]);
        lobby.set_chat_edit_view(LobbyChatEditView {
            text: "aéz".into(),
            caret: 2,
            selection: Some((4, 2)),
            horizontal_scroll: -7,
            cursor_visible: true,
        });
        assert_eq!(lobby.chat_edit.caret, 1);
        assert_eq!(lobby.chat_edit.selection, Some((4, 1)));
        assert_eq!(lobby.chat_edit.horizontal_scroll, 0);
        assert!(matches!(
            lobby
                .chat_edit_key(
                    LobbyChatEditKey::Home,
                    LobbyChatKeyModifiers {
                        shift: true,
                        control: true,
                    }
                )
                .as_slice(),
            [LobbyAction::Chat(LobbyChatRequest::EditKey {
                key: LobbyChatEditKey::Home,
                ..
            })]
        ));
        assert_eq!(
            lobby.chat_clipboard(LobbyChatClipboardShortcut::SelectAll),
            [LobbyAction::Chat(LobbyChatRequest::Clipboard {
                shortcut: LobbyChatClipboardShortcut::SelectAll
            })]
        );
        assert_eq!(
            lobby.chat_context_command(LobbyChatContextCommand::Clear),
            [LobbyAction::Chat(LobbyChatRequest::ContextCommand(
                LobbyChatContextCommand::Clear
            ))]
        );
        let layout = game_lobby_layout(1280, 720, 34, 22, LobbyRole::Host, false, false);
        assert!(matches!(
            lobby.chat_context_from_key(&layout).as_slice(),
            [LobbyAction::Chat(LobbyChatRequest::OpenContextMenu { .. })]
        ));

        lobby.focus = LobbyControl::Exit;
        assert_eq!(
            lobby.text_input("x"),
            [
                LobbyAction::FocusChanged(LobbyControl::ChatInput),
                LobbyAction::Chat(LobbyChatRequest::RefocusAndInsert("x".into()))
            ]
        );
        lobby.focus = LobbyControl::Exit;
        assert!(lobby.text_input(" ").is_empty());
        assert_eq!(lobby.focus(), LobbyControl::Exit);
    }

    #[test]
    fn chat_label_and_double_click_preserve_cpp_focus_semantics() {
        let mut lobby = lobby(LobbyRole::Host, vec![]);
        let layout = game_lobby_layout(1280, 720, 34, 22, LobbyRole::Host, false, false);
        let roster = lobby.roster_layout(&layout, 22);
        let label = GuiPoint::new(
            (layout.chat_label.x + 1) as f32,
            (layout.chat_label.y + 1) as f32,
        );
        let edit = GuiPoint::new(
            (layout.chat_edit.x + 1) as f32,
            (layout.chat_edit.y + 1) as f32,
        );

        lobby.set_chat_edit_view(LobbyChatEditView {
            text: "word".into(),
            caret: 3,
            selection: Some((0, 3)),
            horizontal_scroll: 0,
            cursor_visible: true,
        });
        let edit_view = lobby.chat_edit.clone();
        assert_eq!(lobby.focus(), LobbyControl::ChatInput);
        assert!(lobby.pointer_down(label, &layout, &roster).is_empty());
        assert!(lobby.hotkey('T', Instant::now()).is_empty());
        assert_eq!(lobby.chat_edit, edit_view);

        lobby.focus = LobbyControl::Exit;
        assert_eq!(
            lobby.pointer_down(label, &layout, &roster),
            [
                LobbyAction::FocusChanged(LobbyControl::ChatInput),
                LobbyAction::Chat(LobbyChatRequest::FocusInput)
            ]
        );
        assert_eq!(lobby.focus(), LobbyControl::ChatInput);

        lobby.focus = LobbyControl::Roster;
        assert_eq!(
            lobby.pointer_double_click(edit, &layout, &roster),
            [LobbyAction::Chat(LobbyChatRequest::PointerDoubleClick(
                edit
            ))]
        );
        assert_eq!(lobby.focus(), LobbyControl::Roster);
    }

    #[test]
    fn recursive_focus_visits_selected_children_and_each_option_button() {
        let mut lobby = GameLobby::new(
            LobbyRole::Host,
            "",
            1,
            4,
            true,
            true,
            true,
            false,
            5,
            vec![client(1, true), team_player(2)],
        );
        lobby.selected_row = Some(1);
        lobby.selected_roster_id = Some(LobbyRosterId::Player(2));
        let order = lobby.focus_order();
        assert!(order
            .windows(2)
            .any(|pair| { pair == [LobbyControl::Roster, LobbyControl::RosterTeam] }));
        let option_controls: Vec<_> = order
            .iter()
            .filter_map(|control| match control {
                LobbyControl::GameOption(button) => Some(*button),
                _ => None,
            })
            .collect();
        assert_eq!(
            option_controls,
            LobbyRole::Host.game_option_context().buttons()
        );
        assert_eq!(order.last(), Some(&LobbyControl::Ready));

        if let LobbyRosterRow::Player(player) = &mut lobby.rows[1] {
            player.team.as_mut().expect("team").selectable = false;
        }
        assert!(!lobby.focus_order().contains(&LobbyControl::RosterTeam));

        lobby.focus = LobbyControl::Ready;
        let actions = lobby.set_resources_loaded(false);
        assert_eq!(lobby.focus(), LobbyControl::Run);
        assert_eq!(actions, [LobbyAction::FocusChanged(LobbyControl::Run)]);
        assert!(!lobby.focus_order().contains(&LobbyControl::Ready));
    }

    #[test]
    fn roster_listbox_edges_pages_and_recursive_tab_directions_match_native() {
        let rows = (1..=40).map(|id| client(id, id == 1)).collect::<Vec<_>>();
        let mut lobby = lobby(LobbyRole::Client, rows);
        let layout = game_lobby_layout(640, 360, 34, 22, LobbyRole::Client, false, false);
        let mut roster = lobby.roster_layout(&layout, 22);
        assert!(roster.max_scroll > 0);
        lobby.focus = LobbyControl::Roster;

        lobby.key_down(KeyCode::PageDown, false, &layout, &roster, Instant::now());
        let first_page = lobby
            .selected_row
            .expect("last fully visible first-page row");
        assert!(first_page > 0);
        assert_eq!(lobby.roster_scroll(), 0);

        roster = lobby.roster_layout(&layout, 22);
        lobby.key_down(KeyCode::PageDown, false, &layout, &roster, Instant::now());
        assert!(lobby.selected_row.unwrap() > first_page);
        assert!(lobby.roster_scroll() > 0);

        roster = lobby.roster_layout(&layout, 22);
        lobby.key_down(KeyCode::End, false, &layout, &roster, Instant::now());
        assert_eq!(lobby.selected_row, Some(39));
        assert_eq!(lobby.roster_scroll(), roster.max_scroll);

        roster = lobby.roster_layout(&layout, 22);
        lobby.key_down(KeyCode::Home, false, &layout, &roster, Instant::now());
        assert_eq!(lobby.selected_row, Some(0));
        assert_eq!(lobby.roster_scroll(), 0);

        roster = lobby.roster_layout(&layout, 22);
        lobby.key_down(KeyCode::Tab, false, &layout, &roster, Instant::now());
        assert_eq!(lobby.focus(), LobbyControl::RosterAddPlayer);
        lobby.key_down(KeyCode::Tab, false, &layout, &roster, Instant::now());
        assert_eq!(lobby.focus(), LobbyControl::Exit);
        lobby.focus = LobbyControl::Roster;
        lobby.key_down(KeyCode::Tab, true, &layout, &roster, Instant::now());
        assert_eq!(lobby.focus(), LobbyControl::ScenarioTab);
    }

    #[test]
    fn disabled_client_ready_moves_focus_to_last_recursive_option() {
        let mut lobby = lobby(LobbyRole::Client, vec![]);
        lobby.focus = LobbyControl::Ready;
        assert_eq!(
            lobby.set_resources_loaded(false),
            [
                LobbyAction::FocusChanged(LobbyControl::GameOption(GameOptionButton::Record)),
                LobbyAction::GameOptions(LobbyGameOptionInput::Focus(GameOptionButton::Record))
            ]
        );
    }

    #[test]
    fn option_child_focus_results_rejoin_the_recursive_dialog() {
        let mut lobby = lobby(LobbyRole::Host, vec![]);
        let layout = game_lobby_layout(1280, 720, 34, 22, LobbyRole::Host, false, false);
        let roster = lobby.roster_layout(&layout, 22);
        lobby.focus = LobbyControl::GameOption(GameOptionButton::Internet);
        assert_eq!(
            lobby.key_down(KeyCode::Tab, true, &layout, &roster, Instant::now()),
            [
                LobbyAction::FocusChanged(LobbyControl::Exit),
                LobbyAction::GameOptions(LobbyGameOptionInput::ClearFocus)
            ]
        );

        lobby.focus = LobbyControl::GameOption(GameOptionButton::Internet);
        assert_eq!(
            lobby
                .game_option_focus_changed(GameOptionButton::League)
                .expect("valid child"),
            [LobbyAction::FocusChanged(LobbyControl::GameOption(
                GameOptionButton::League
            ))]
        );
        assert_eq!(
            lobby.game_option_input_unhandled(),
            [
                LobbyAction::FocusChanged(LobbyControl::ChatInput),
                LobbyAction::GameOptions(LobbyGameOptionInput::ClearFocus),
                LobbyAction::Chat(LobbyChatRequest::FocusInput)
            ]
        );

        lobby.focus = LobbyControl::GameOption(GameOptionButton::Record);
        assert_eq!(
            lobby.game_option_focus_traversal_requested(false),
            [
                LobbyAction::FocusChanged(LobbyControl::Run),
                LobbyAction::GameOptions(LobbyGameOptionInput::ClearFocus)
            ]
        );
    }

    #[test]
    fn enter_and_space_share_button_latch_and_mouse_preserves_chat_focus() {
        let mut lobby = lobby(LobbyRole::Host, vec![]);
        let layout = game_lobby_layout(1280, 720, 34, 22, LobbyRole::Host, false, false);
        let roster = lobby.roster_layout(&layout, 22);

        lobby.focus = LobbyControl::Exit;
        assert!(lobby
            .key_down(KeyCode::Enter, false, &layout, &roster, Instant::now())
            .is_empty());
        let _ = lobby.key_down(KeyCode::Space, false, &layout, &roster, Instant::now());
        assert!(lobby.key_up(KeyCode::Space).is_empty());
        assert_eq!(lobby.key_up(KeyCode::Enter), [LobbyAction::ExitRequested]);

        lobby.focus = LobbyControl::Roster;
        assert_eq!(
            lobby.key_down(KeyCode::Enter, false, &layout, &roster, Instant::now()),
            [
                LobbyAction::FocusChanged(LobbyControl::ChatInput),
                LobbyAction::Chat(LobbyChatRequest::FocusInput)
            ]
        );

        lobby.focus = LobbyControl::ChatInput;
        let exit = GuiPoint::new(
            (layout.exit_button.x + 1) as f32,
            (layout.exit_button.y + 1) as f32,
        );
        let _ = lobby.pointer_down(exit, &layout, &roster);
        assert_eq!(lobby.focus(), LobbyControl::ChatInput);
        assert_eq!(
            lobby.pointer_up(exit, &layout, &roster, Instant::now()),
            [LobbyAction::ExitRequested]
        );
        assert_eq!(lobby.focus(), LobbyControl::ChatInput);
    }

    #[test]
    fn outer_button_shared_down_preserves_drag_and_orders_releases_like_cpp() {
        let setup = || {
            let mut lobby = lobby(LobbyRole::Host, vec![]);
            let layout = game_lobby_layout(1280, 720, 34, 22, LobbyRole::Host, false, false);
            let roster = lobby.roster_layout(&layout, 22);
            let exit = GuiPoint::new(
                (layout.exit_button.x + 1) as f32,
                (layout.exit_button.y + 1) as f32,
            );
            lobby.focus = LobbyControl::Exit;
            assert!(lobby.pointer_down(exit, &layout, &roster).is_empty());
            assert_eq!(lobby.take_sounds(), [LobbySound::ArrowHit]);
            assert!(lobby
                .key_down(KeyCode::Enter, false, &layout, &roster, Instant::now())
                .is_empty());
            assert!(lobby.take_sounds().is_empty());
            (lobby, layout, roster, exit)
        };

        let (mut pointer_first, layout, roster, exit) = setup();
        assert_eq!(
            pointer_first.pointer_up(exit, &layout, &roster, Instant::now()),
            [LobbyAction::ExitRequested]
        );
        assert_eq!(pointer_first.take_sounds(), [LobbySound::Click]);
        assert!(pointer_first.key_up(KeyCode::Enter).is_empty());

        let (mut key_first, layout, roster, exit) = setup();
        assert_eq!(
            key_first.key_up(KeyCode::Enter),
            [LobbyAction::ExitRequested]
        );
        assert_eq!(key_first.take_sounds(), [LobbySound::Click]);
        assert_eq!(key_first.pointer_pressed, Some(HitTarget::Exit));
        assert!(!key_first.pointer_inside_pressed);
        assert!(key_first.pointer_move(exit, &layout, &roster).is_empty());
        assert!(key_first.take_sounds().is_empty());
        assert!(key_first
            .pointer_up(exit, &layout, &roster, Instant::now())
            .is_empty());
        assert!(key_first.take_sounds().is_empty());

        let (mut rearm, layout, roster, exit) = setup();
        assert_eq!(rearm.key_up(KeyCode::Enter), [LobbyAction::ExitRequested]);
        let _ = rearm.take_sounds();
        assert!(rearm
            .pointer_move(GuiPoint::new(0.0, 0.0), &layout, &roster)
            .is_empty());
        assert!(rearm.take_sounds().is_empty());
        assert!(rearm.pointer_move(exit, &layout, &roster).is_empty());
        assert_eq!(rearm.take_sounds(), [LobbySound::ArrowHit]);
        assert_eq!(
            rearm.pointer_up(exit, &layout, &roster, Instant::now()),
            [LobbyAction::ExitRequested]
        );
        assert_eq!(rearm.take_sounds(), [LobbySound::Click]);
    }

    #[test]
    fn outer_button_mouse_up_outside_preserves_key_and_key_owned_reentry_is_silent() {
        let setup_outside_key = || {
            let mut lobby = lobby(LobbyRole::Host, vec![]);
            let layout = game_lobby_layout(1280, 720, 34, 22, LobbyRole::Host, false, false);
            let roster = lobby.roster_layout(&layout, 22);
            let exit = GuiPoint::new(
                (layout.exit_button.x + 1) as f32,
                (layout.exit_button.y + 1) as f32,
            );
            let outside = GuiPoint::new(0.0, 0.0);
            lobby.focus = LobbyControl::Exit;
            assert!(lobby.pointer_down(exit, &layout, &roster).is_empty());
            assert_eq!(lobby.take_sounds(), [LobbySound::ArrowHit]);
            assert!(lobby.pointer_move(outside, &layout, &roster).is_empty());
            assert_eq!(lobby.take_sounds(), [LobbySound::ArrowHit]);
            assert!(lobby
                .key_down(KeyCode::Enter, false, &layout, &roster, Instant::now())
                .is_empty());
            assert_eq!(lobby.take_sounds(), [LobbySound::ArrowHit]);
            (lobby, layout, roster, exit, outside)
        };

        let (mut released_outside, layout, roster, _exit, outside) = setup_outside_key();
        assert!(released_outside
            .pointer_up(outside, &layout, &roster, Instant::now())
            .is_empty());
        assert!(released_outside.take_sounds().is_empty());
        assert_eq!(
            released_outside.key_up(KeyCode::Enter),
            [LobbyAction::ExitRequested]
        );
        assert_eq!(released_outside.take_sounds(), [LobbySound::Click]);

        let (mut reentered, layout, roster, exit, _outside) = setup_outside_key();
        assert!(reentered.pointer_move(exit, &layout, &roster).is_empty());
        assert!(reentered.take_sounds().is_empty());
        assert_eq!(
            reentered.pointer_up(exit, &layout, &roster, Instant::now()),
            [LobbyAction::ExitRequested]
        );
        assert_eq!(reentered.take_sounds(), [LobbySound::Click]);
        assert!(reentered.key_up(KeyCode::Enter).is_empty());
    }

    #[test]
    fn roster_scrollbar_drag_repeat_and_clipped_child_hits_match_listbox() {
        let rows = (0..40)
            .map(player)
            .chain(std::iter::once(client(99, true)))
            .collect();
        let mut lobby = lobby(LobbyRole::Host, rows);
        let layout = game_lobby_layout(640, 300, 34, 22, LobbyRole::Host, false, false);
        let roster = lobby.roster_layout(&layout, 22);
        assert!(roster.max_scroll > 0);

        let track = GuiPoint::new(
            (layout.roster_scrollbar.x + 1) as f32,
            (layout.roster_scrollbar.y + layout.roster_scrollbar.h / 2) as f32,
        );
        let _ = lobby.pointer_down(track, &layout, &roster);
        assert_eq!(lobby.scrollbar_drag, Some(ScrollbarDrag::Roster));
        assert!(lobby.roster_scroll() > 0);
        let _ = lobby.pointer_up(track, &layout, &roster, Instant::now());

        lobby.roster_scroll = 0;
        lobby.roster_scroll_pin = 0;
        lobby.pointer_pressed = Some(HitTarget::RosterScrollBottom);
        lobby.advance_held_scrollbars(&layout, roster.max_scroll);
        lobby.advance_held_scrollbars(&layout, roster.max_scroll);
        assert_eq!(lobby.roster_scroll_pin, 2);
        let _ = lobby.pointer_move(GuiPoint::new(0.0, 0.0), &layout, &roster);
        assert_eq!(lobby.pointer_pressed, None);

        let offscreen = roster.rows.last().expect("last row");
        let add = offscreen.add_player.expect("local add button");
        let point = GuiPoint::new((add.x + 1) as f32, (add.y + 1) as f32);
        assert_ne!(
            lobby.hit_test(point, &layout, &roster),
            HitTarget::AddPlayer(offscreen.index)
        );
    }

    #[test]
    fn roster_selection_survives_reordering_by_semantic_id() {
        let mut lobby = lobby(LobbyRole::Host, vec![client(1, true), player(2), player(3)]);
        lobby.selected_row = Some(1);
        lobby.selected_roster_id = Some(LobbyRosterId::Player(2));
        lobby.set_rows(vec![player(3), client(1, true), player(2)]);
        assert_eq!(lobby.selected_row, Some(2));
        assert_eq!(lobby.selected_roster_id(), Some(&LobbyRosterId::Player(2)));
    }

    #[test]
    fn final_countdown_but_not_long_countdown_locks_team_combo() {
        let mut lobby = GameLobby::new(
            LobbyRole::Host,
            "",
            1,
            4,
            true,
            false,
            true,
            false,
            5,
            vec![team_player(2)],
        );
        lobby.selected_row = Some(0);
        lobby.selected_roster_id = Some(LobbyRosterId::Player(2));
        let layout = game_lobby_layout(1280, 720, 34, 22, LobbyRole::Host, true, false);
        let roster = lobby.roster_layout(&layout, 22);
        let combo = roster.rows[0].team.expect("team combo");
        let point = GuiPoint::new((combo.x + 1) as f32, (combo.y + 1) as f32);
        lobby.set_open_team_combo_player(Some(2));

        let _ = lobby.apply_countdown_packet(LobbyCountdownPacket::Seconds(11));
        assert_eq!(lobby.open_team_combo_player(), Some(2));
        assert!(lobby
            .pointer_down(point, &layout, &roster)
            .contains(&LobbyAction::TeamSelectionRequested { player_id: 2 }));
        lobby.focus = LobbyControl::RosterTeam;
        let actions = lobby.apply_countdown_packet(LobbyCountdownPacket::Seconds(10));
        assert_eq!(lobby.open_team_combo_player(), None);
        assert!(actions.contains(&LobbyAction::FocusChanged(LobbyControl::Roster)));
        assert!(!lobby.focus_order().contains(&LobbyControl::RosterTeam));
        assert!(!lobby
            .pointer_down(point, &layout, &roster)
            .contains(&LobbyAction::TeamSelectionRequested { player_id: 2 }));
    }

    #[test]
    fn option_strip_emits_mouse_leave_without_a_pressed_child() {
        let mut lobby = lobby(LobbyRole::Host, vec![]);
        let layout = game_lobby_layout(1280, 720, 34, 22, LobbyRole::Host, false, false);
        let roster = lobby.roster_layout(&layout, 22);
        let option = game_option_buttons_layout(
            layout.game_option_strip,
            LobbyRole::Host.game_option_context(),
        )
        .buttons[0]
            .rect;
        let inside = GuiPoint::new((option.x + 1) as f32, (option.y + 1) as f32);
        assert!(matches!(
            lobby.pointer_move(inside, &layout, &roster).as_slice(),
            [LobbyAction::GameOptions(LobbyGameOptionInput::PointerMove(
                _
            ))]
        ));
        assert_eq!(
            lobby.pointer_move(GuiPoint::new(0.0, 0.0), &layout, &roster),
            [LobbyAction::GameOptions(LobbyGameOptionInput::MouseLeave)]
        );
    }

    #[test]
    fn roster_spacing_combo_rank_and_add_highlights_are_row_exact() {
        let free = header(LobbyRosterHeader::UnassignedSavegamePlayers, false);
        let script = header(LobbyRosterHeader::ScriptPlayers, true);
        let replay = header(LobbyRosterHeader::ReplayPlayers, false);
        assert_eq!(free.top_spacing(), CLIENT_ROW_SPACING);
        assert_eq!(script.top_spacing(), CLIENT_ROW_SPACING);
        assert!(!free.has_spacing_bar());
        assert!(!script.has_spacing_bar());
        assert!(replay.has_spacing_bar());
        assert_eq!(player(1).top_spacing(), DEFAULT_ROW_SPACING);

        let mut league_lobby = GameLobby::new(
            LobbyRole::Host,
            "",
            1,
            4,
            true,
            false,
            true,
            false,
            5,
            vec![player(1)],
        );
        league_lobby.set_league_mode(true);
        let layout = game_lobby_layout(1280, 720, 34, 22, LobbyRole::Host, true, false);
        let roster = league_lobby.roster_layout(&layout, 22);
        assert_eq!(roster.rows[0].rect.h, 54);
        // C4GUI::ListBox applies its three-pixel client margin before laying
        // out the team and rank columns (src/C4GuiListBox.h:121-125).
        assert_eq!(
            roster.rows[0].team,
            Some(IntRect::new(920, 132, 279, 26)),
            "team 0 keeps the exact blank-combo geometry"
        );
        assert_eq!(
            roster.rows[0].rank,
            Some(IntRect::new(1199, 132, 26, 26)),
            "league reserves rank width even without a rank symbol"
        );

        let mut lobby = lobby(LobbyRole::Host, vec![client(1, true), client(2, true)]);
        lobby.selected_row = Some(0);
        lobby.focus = LobbyControl::RosterAddPlayer;
        assert!(lobby.small_button_highlighted(HitTarget::AddPlayer(0), true));
        assert!(!lobby.small_button_highlighted(HitTarget::AddPlayer(1), true));
        lobby.hovered = HitTarget::AddPlayer(1);
        assert!(lobby.small_button_highlighted(HitTarget::AddPlayer(1), true));
    }

    #[test]
    fn localized_labels_tooltips_and_context_ownership_are_explicit() {
        let mut lobby = lobby(
            LobbyRole::Host,
            vec![
                header(LobbyRosterHeader::ScriptPlayers, true),
                client(1, true),
            ],
        );
        let labels = LobbyLabels {
            lobby: "Vestíbulo".into(),
            tooltip_chat: "Mensaje".into(),
            ..LobbyLabels::default()
        };
        lobby.set_labels(labels);
        lobby.scenario_title.clear();
        assert_eq!(lobby.title(), "Vestíbulo");

        let layout = game_lobby_layout(1280, 720, 34, 22, LobbyRole::Host, false, false);
        let roster = lobby.roster_layout(&layout, 22);
        let chat = GuiPoint::new(
            (layout.chat_edit.x + 1) as f32,
            (layout.chat_edit.y + 1) as f32,
        );
        let _ = lobby.pointer_move(chat, &layout, &roster);
        let now = Instant::now();
        lobby.hover_since = now - TOOLTIP_DELAY;
        assert_eq!(
            lobby.tooltip_state_at(now).map(|tooltip| tooltip.text),
            Some("Mensaje".into())
        );

        let header_point = GuiPoint::new(
            (roster.rows[0].rect.x + 1) as f32,
            (roster.rows[0].rect.y + 1) as f32,
        );
        assert!(lobby
            .pointer_secondary_down(header_point, &layout, &roster)
            .is_empty());
        let _ = lobby.pointer_move(header_point, &layout, &roster);
        lobby.hover_since = now - TOOLTIP_DELAY;
        assert_eq!(
            lobby.tooltip_state_at(now).map(|tooltip| tooltip.text),
            Some("Players controlled by computer.".into())
        );
        lobby.selected_row = Some(0);
        assert!(lobby
            .request_focused_context(GuiPoint::new(1.0, 1.0))
            .is_empty());

        let fonts = endeavour_font_set();
        let mut ping_client = match client(2, false) {
            LobbyRosterRow::Client(client) => client,
            _ => unreachable!(),
        };
        ping_client.ping_ms = Some(42);
        let mut ping_lobby = GameLobby::new(
            LobbyRole::Host,
            "Gold Mine",
            1,
            4,
            false,
            false,
            true,
            false,
            5,
            vec![LobbyRosterRow::Client(ping_client)],
        );
        let ping_roster = ping_lobby.roster_layout(&layout, fonts.text.line_height);
        let ping_text_width = fonts.text.measure("42 ms", true).0;
        let ping_point = GuiPoint::new(
            (ping_roster.rows[0].rect.x + ping_roster.rows[0].rect.w - ping_text_width + 1) as f32,
            (ping_roster.rows[0].rect.y + 1) as f32,
        );
        let _ = ping_lobby.pointer_move(ping_point, &layout, &ping_roster);
        ping_lobby.hover_since = now - TOOLTIP_DELAY;
        assert_eq!(
            ping_lobby
                .tooltip_state_with_roster_at(now, &ping_roster, &fonts.text)
                .map(|tooltip| tooltip.text),
            Some("Ping".into())
        );
    }

    #[test]
    fn non_pointer_input_suppresses_tooltip_until_actual_integer_pointer_motion() {
        let mut lobby = lobby(LobbyRole::Host, vec![]);
        let layout = game_lobby_layout(1280, 720, 34, 22, LobbyRole::Host, false, false);
        let roster = lobby.roster_layout(&layout, 22);
        let point = GuiPoint::new(
            (layout.chat_edit.x + 2) as f32,
            (layout.chat_edit.y + 2) as f32,
        );
        let _ = lobby.pointer_move(point, &layout, &roster);
        let after_delay = Instant::now() + TOOLTIP_DELAY;
        assert!(lobby.tooltip_state_at(after_delay).is_some());

        lobby.note_non_pointer_input();
        assert!(lobby.tooltip_state_at(after_delay).is_none());

        let _ = lobby.pointer_move(point, &layout, &roster);
        assert!(
            lobby.tooltip_state_at(after_delay).is_none(),
            "a synthesized same-pixel motion must not reactivate CMouse"
        );

        let moved = GuiPoint::new(point.x + 1.0, point.y);
        let _ = lobby.pointer_move(moved, &layout, &roster);
        assert!(
            lobby
                .tooltip_state_at(Instant::now() + TOOLTIP_DELAY)
                .is_some(),
            "actual integer motion starts a fresh tooltip delay"
        );

        lobby.hover_since = Instant::now() - TOOLTIP_DELAY;
        let _ = lobby.pointer_up(moved, &layout, &roster, Instant::now());
        let release_time = lobby.hover_since;
        assert_eq!(
            lobby.tooltip_state_at(release_time + TOOLTIP_DELAY - Duration::from_millis(1)),
            None,
            "mouse-up resets the tooltip clock"
        );
        assert!(
            lobby
                .tooltip_state_at(release_time + TOOLTIP_DELAY)
                .is_some(),
            "the inclusive 500ms boundary is restored after mouse-up"
        );
    }

    #[test]
    fn start_request_encodes_league_validation_and_savegame_confirmation() {
        let mut lobby = GameLobby::new(
            LobbyRole::Host,
            "",
            1,
            4,
            false,
            false,
            true,
            false,
            -1,
            vec![header(LobbyRosterHeader::UnassignedSavegamePlayers, false)],
        );
        lobby.set_league_mode(true);
        assert_eq!(
            lobby.activate_control(LobbyControl::Run),
            [LobbyAction::StartRequested {
                countdown_seconds: 5,
                check_league_rules: true,
                confirm_unassociated_savegame_players: true,
            }]
        );
        lobby.countdown = LobbyCountdownState::Long { seconds: 20 };
        assert_eq!(
            lobby.activate_control(LobbyControl::Run),
            [LobbyAction::AbortCountdownRequested]
        );
    }

    #[test]
    fn unrelated_key_up_does_not_cancel_held_run_button() {
        let mut lobby = lobby(LobbyRole::Host, vec![]);
        lobby.focus = LobbyControl::Run;
        let layout = game_lobby_layout(1280, 720, 34, 22, LobbyRole::Host, false, false);
        let roster = lobby.roster_layout(&layout, 22);
        let _ = lobby.key_down(KeyCode::Space, false, &layout, &roster, Instant::now());
        assert!(lobby.key_up(KeyCode::Enter).is_empty());
        assert_eq!(
            lobby.key_up(KeyCode::Space),
            [LobbyAction::StartRequested {
                countdown_seconds: 5,
                check_league_rules: true,
                confirm_unassociated_savegame_players: false,
            }]
        );
    }
}
