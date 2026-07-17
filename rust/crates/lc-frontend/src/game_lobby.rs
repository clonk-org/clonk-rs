//! Classic `C4GameLobby::MainDlg` frontend for the initially visible
//! Players/Clients sheet.
//!
//! The fullscreen lobby is a transparent overlay: the C++ dialog deliberately
//! leaves the loader/game background in place.  This module therefore draws
//! only classic GUI furniture and refuses incomplete or substituted resources.
//! Sheets and dialogs outside this bounded slice are emitted as typed requests.

use std::collections::HashMap;
use std::time::{Duration, Instant};

use anyhow::{ensure, Result};
use lc_graphics::clonk_font::TextAlign;
use lc_graphics::{GammaRamp, PixelFormat, Surface};
use lc_gui::Rect as GuiRect;

use crate::classic_gui::{
    blacken_transparent_pixels, draw_3d_frame, draw_clipped_text, draw_engine_box,
    draw_facet_stretch, ClassicButtonState, ClassicGuiSkin, IntRect, STANDARD_BACKGROUND_COLOR,
};
use crate::context_menu::draw_classic_tooltip;
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
const CLIENT_ROW_SPACING: i32 = 8;
const DEFAULT_ROW_SPACING: i32 = 1;
const PLAYER_ROW_INDENT: i32 = 3;
const ICON_LABEL_SPACING: i32 = 2;
const READY_COOLDOWN: Duration = Duration::from_secs(2);
const SOUND_ICON_SHOW_TIME: Duration = Duration::from_secs(1);
const TOOLTIP_DELAY: Duration = Duration::from_millis(500);

const STANDARD_ICON_WIDTH: u32 = 240;
const STANDARD_ICON_HEIGHT: u32 = 360;
const EXTENDED_ICON_WIDTH: u32 = 256;
const EXTENDED_ICON_HEIGHT: u32 = 320;
const CAPTION_WIDTH: u32 = 192;
const CAPTION_HEIGHT: u32 = 23;
const BUTTON_TEXTURE_WIDTH: u32 = 128;
const BUTTON_TEXTURE_HEIGHT: u32 = 32;
const HIGHLIGHT_EXTENT: u32 = 16;
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

/// A right-side sheet. Only [`Self::Players`] is rendered by this slice.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum LobbySheet {
    Players,
    Teams,
    Resources,
    Options,
    Scenario,
}

/// Localized resource strings used by the visible lobby slice. Templates use
/// named braces so the frontend never bakes an English word order into layout.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct LobbyLabels {
    pub lobby: String,
    pub scenario_lobby_template: String,
    pub players_template: String,
    pub chat: String,
    pub exit: String,
    pub start: String,
    pub cancel: String,
    pub ready: String,
    pub still_loading: String,
    pub countdown_template: String,
    pub countdown_short_template: String,
    pub start_aborted: String,
    pub tooltip_chat: String,
    pub tooltip_exit: String,
    pub tooltip_start: String,
    pub tooltip_ready: String,
    pub tooltip_ready_unavailable: String,
    pub tooltip_ping: String,
    pub tooltip_unassigned_savegame_players: String,
    pub tooltip_script_players: String,
    pub tooltip_replay_players: String,
}

impl Default for LobbyLabels {
    fn default() -> Self {
        Self {
            lobby: "Lobby".into(),
            scenario_lobby_template: "{scenario} - {lobby}".into(),
            players_template: "&Players ({active}/{maximum})".into(),
            chat: "Cha&t:".into(),
            exit: "E&xit".into(),
            start: "&Start".into(),
            cancel: "Cancel".into(),
            ready: "R&eady".into(),
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
            tooltip_ping: "Ping".into(),
            tooltip_unassigned_savegame_players: "Unassociated savegame players.".into(),
            tooltip_script_players: "Players controlled by computer.".into(),
            tooltip_replay_players: "Starring".into(),
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
                    kind: LobbyRosterHeader::ReplayPlayers,
                    ..
                })
        )
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct LobbyLogLine {
    pub text: String,
    pub color: [u8; 4],
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
    Exit,
    GameOption(GameOptionButton),
    Run,
    Ready,
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

#[derive(Clone, Debug, PartialEq, Eq)]
struct WrappedLobbyLogLine {
    text: String,
    color: [u8; 4],
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
        let rect = IntRect {
            x: self.area.x + self.margin_x,
            y: self.area.y + self.margin_y,
            w: self.area.w - 2 * self.margin_x,
            h: height,
        };
        let used = height + 2 * self.margin_y;
        self.area.y += used;
        self.area.h -= used;
        rect
    }

    fn get_from_bottom(&mut self, height: i32) -> IntRect {
        let rect = IntRect {
            x: self.area.x + self.margin_x,
            y: self.area.y + self.area.h - height - self.margin_y,
            w: self.area.w - 2 * self.margin_x,
            h: height,
        };
        self.area.h -= height + 2 * self.margin_y;
        rect
    }

    fn get_from_left(&mut self, width: i32) -> IntRect {
        let rect = IntRect {
            x: self.area.x + self.margin_x,
            y: self.area.y + self.margin_y,
            w: width,
            h: self.area.h - 2 * self.margin_y,
        };
        let used = width + 2 * self.margin_x;
        self.area.x += used;
        self.area.w -= used;
        rect
    }

    fn get_from_right(&mut self, width: i32) -> IntRect {
        let rect = IntRect {
            x: self.area.x + self.area.w - width - self.margin_x,
            y: self.area.y + self.margin_y,
            w: width,
            h: self.area.h - 2 * self.margin_y,
        };
        self.area.w -= width + 2 * self.margin_x;
        rect
    }

    const fn all(self) -> IntRect {
        IntRect {
            x: self.area.x + self.margin_x,
            y: self.area.y + self.margin_y,
            w: self.area.w - 2 * self.margin_x,
            h: self.area.h - 2 * self.margin_y,
        }
    }

    const fn inner_width(self) -> i32 {
        self.area.w - 2 * self.margin_x
    }

    const fn height(self) -> i32 {
        self.area.h
    }

    const fn centered(self, width: i32, height: i32) -> IntRect {
        IntRect {
            x: self.area.x + self.area.w / 2 - width / 2,
            y: self.area.y + self.area.h / 2 - height / 2,
            w: width,
            h: height,
        }
    }

    fn expand_top(&mut self, height: i32) {
        self.area.y -= height;
        self.area.h += height;
    }
}

fn offset(rect: IntRect, x: i32, y: i32) -> IntRect {
    IntRect {
        x: rect.x + x,
        y: rect.y + y,
        ..rect
    }
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
    let client = IntRect {
        x: margin_x,
        y: margin_top,
        w: screen_width - 2 * margin_x,
        h: screen_height - margin_top - margin_y,
    };
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

    let mut main = Aligner::new(
        IntRect {
            x: 0,
            y: 0,
            w: client.w,
            h: client.h,
        },
        0,
        0,
    );
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
    let roster = IntRect {
        x: right_tab.x + TAB_SHEET_MARGIN,
        y: right_tab.y + TAB_SHEET_MARGIN,
        w: right_tab.w - 2 * TAB_SHEET_MARGIN,
        h: right_tab.h - 2 * TAB_SHEET_MARGIN,
    };
    let roster_client = IntRect {
        w: roster.w - SCROLLBAR_EXTENT,
        ..roster
    };
    let roster_scrollbar = IntRect {
        x: roster_client.x + roster_client.w,
        y: roster.y,
        w: SCROLLBAR_EXTENT,
        h: roster.h,
    };

    let mut center = Aligner::new(main.all(), indent_x2, indent_y3);
    let edit_height = (text_line_height + 3).max(CAPTION_HEIGHT as i32);
    let chat_row = center.get_from_bottom(edit_height);
    let mut chat = Aligner::new(chat_row, 0, 0);
    let chat_label = absolute(chat.get_from_left(40));
    let chat_edit = absolute(chat.all());
    let chat_log = absolute(center.all());
    let chat_log_client = IntRect {
        x: chat_log.x + 10,
        y: chat_log.y + 8,
        w: chat_log.w - 10 - 5 - SCROLLBAR_EXTENT,
        h: chat_log.h - 16,
    };
    let chat_log_scrollbar = IntRect {
        x: chat_log.x + chat_log.w - 5 - SCROLLBAR_EXTENT,
        y: chat_log.y + 8,
        w: SCROLLBAR_EXTENT,
        h: chat_log.h - 16,
    };

    let count = 4 + usize::from(has_teams) + usize::from(has_external_chat);
    let mut next_index = count as i32;
    let mut tab_buttons = Vec::with_capacity(count);
    let mut add_tab = |control, sheet, icon, selected| {
        next_index -= 1;
        let rect = IntRect {
            x: right_caption.x + right_caption.w - (TAB_ICON_EXTENT + 4) * (next_index + 1),
            y: right_caption.y + 4,
            w: TAB_ICON_EXTENT,
            h: TAB_ICON_EXTENT,
        };
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
        ready_square: IntRect {
            w: ready_checkbox.h,
            ..ready_checkbox
        },
        game_option_strip,
        tab_buttons,
    }
}

/// Exact classic resources. `new` fails instead of allowing generic widgets.
#[derive(Clone)]
pub struct LobbyResources<'a> {
    fonts: &'a ClonkFontSet,
    tooltip_font: &'a lc_graphics::clonk_font::ClonkFont,
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
        tooltip_font: &'a lc_graphics::clonk_font::ClonkFont,
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
        validate_exact(
            "GUIButtonHighlight.png",
            &self.button_highlight,
            HIGHLIGHT_EXTENT,
            HIGHLIGHT_EXTENT,
        )?;
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
    AddPlayer(usize),
    Team(usize),
    RosterScrollTop,
    RosterScrollBottom,
    RosterScrollTrack,
    RosterScrollInert,
    Exit,
    GameOption(GameOptionButton),
    Run,
    Ready,
}

impl HitTarget {
    const fn button_control(self) -> Option<LobbyControl> {
        match self {
            Self::Tab(control) => Some(control),
            Self::Exit => Some(LobbyControl::Exit),
            Self::Run => Some(LobbyControl::Run),
            Self::AddPlayer(_) => Some(LobbyControl::RosterAddPlayer),
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
    active_players: i32,
    max_players: i32,
    has_teams: bool,
    has_external_chat: bool,
    resources_loaded: bool,
    ready: bool,
    configured_countdown_seconds: i32,
    league_mode: bool,
    countdown: LobbyCountdownState,
    rows: Vec<LobbyRosterRow>,
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
    pointer_pressed: Option<HitTarget>,
    pointer_inside_pressed: bool,
    key_pressed: Option<(LobbyControl, KeyCode)>,
    selected_row: Option<usize>,
    selected_roster_id: Option<LobbyRosterId>,
    roster_scroll: i32,
    roster_max_scroll: i32,
    roster_scroll_pin: i32,
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
            active_players,
            max_players,
            has_teams,
            has_external_chat,
            resources_loaded,
            ready,
            configured_countdown_seconds,
            league_mode: false,
            countdown: LobbyCountdownState::None,
            rows,
            client_sound_status: HashMap::new(),
            logs: Vec::new(),
            chat_edit: LobbyChatEditView::default(),
            chat_scroll: 0,
            chat_max_scroll: 0,
            chat_scroll_pin: 0,
            chat_follow_bottom: true,
            focus: LobbyControl::ChatInput,
            hovered: HitTarget::None,
            hover_since: Instant::now(),
            pointer: None,
            pointer_pressed: None,
            pointer_inside_pressed: false,
            key_pressed: None,
            selected_row: None,
            selected_roster_id: None,
            roster_scroll: 0,
            roster_max_scroll: 0,
            roster_scroll_pin: 0,
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

    pub const fn focus(&self) -> LobbyControl {
        self.focus
    }

    pub const fn ready(&self) -> bool {
        self.ready
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

    pub const fn chat_scroll(&self) -> i32 {
        self.chat_scroll
    }

    pub const fn roster_scroll(&self) -> i32 {
        self.roster_scroll
    }

    pub fn set_rows(&mut self, rows: Vec<LobbyRosterRow>) {
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
        self.selected_row = selected_id
            .as_ref()
            .and_then(|selected| self.rows.iter().position(|row| row.id() == *selected));
        self.selected_roster_id = self
            .selected_row
            .and_then(|index| self.rows.get(index))
            .map(LobbyRosterRow::id);
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
            self.focus = LobbyControl::Roster;
        }
    }

    pub fn set_player_count(&mut self, active: i32, maximum: i32) {
        self.active_players = active;
        self.max_players = maximum;
    }

    pub fn set_resources_loaded(&mut self, loaded: bool) -> Vec<LobbyAction> {
        self.resources_loaded = loaded;
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

    pub fn layout(&self, width: i32, height: i32, fonts: &ClonkFontSet) -> LobbyLayout {
        game_lobby_layout(
            width,
            height,
            fonts.title.line_height,
            fonts.text.line_height,
            self.role,
            self.has_teams,
            self.has_external_chat,
        )
    }

    pub fn roster_layout(
        &mut self,
        layout: &LobbyLayout,
        text_line_height: i32,
    ) -> LobbyRosterLayout {
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

    pub fn chat_scroll_metrics(
        &mut self,
        layout: &LobbyLayout,
        font: &lc_graphics::clonk_font::ClonkFont,
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
        font: &lc_graphics::clonk_font::ClonkFont,
    ) -> Vec<WrappedLobbyLogLine> {
        let mut wrapped = Vec::new();
        for line in &self.logs {
            for paragraph in line
                .text
                .split(['\r', '\n', '|'])
                .filter(|paragraph| !paragraph.is_empty())
            {
                let text = break_message(font, paragraph, layout.chat_log_client.w.max(1));
                for (physical_index, physical) in lobby_markup_lines(&text)
                    .into_iter()
                    .filter(|physical| !physical.is_empty())
                    .enumerate()
                {
                    wrapped.push(WrappedLobbyLogLine {
                        text: physical,
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
            let rect = IntRect {
                x: layout.roster_client.x + indent,
                y: layout.roster_client.y + y - self.roster_scroll,
                w: layout.roster_client.w - indent,
                h: height,
            };
            let icon = IntRect {
                x: rect.x,
                y: rect.y,
                w: height,
                h: height,
            };
            let add_player = match row {
                LobbyRosterRow::Client(client) if client.local => Some(IntRect {
                    x: rect.x + rect.w - height - 2,
                    y: rect.y,
                    w: height,
                    h: height,
                }),
                LobbyRosterRow::Header(header)
                    if header.kind == LobbyRosterHeader::ScriptPlayers && header.can_add_player =>
                {
                    Some(IntRect {
                        x: rect.x + rect.w - height - 2,
                        y: rect.y,
                        w: height,
                        h: height,
                    })
                }
                _ => None,
            };
            let (team, rank) = match row {
                LobbyRosterRow::Player(_) if !collapsed => {
                    let team_y = rect.y + height - (text_line_height + 4) - ICON_LABEL_SPACING;
                    let mut team_rect = IntRect {
                        x: rect.x + height + 2 * ICON_LABEL_SPACING + 2,
                        y: team_y,
                        w: rect.w - height - 4 * ICON_LABEL_SPACING,
                        h: text_line_height + 4,
                    };
                    let rank = self.league_mode.then(|| {
                        let rank_rect = IntRect {
                            x: team_rect.x + team_rect.w - team_rect.h,
                            y: team_rect.y,
                            w: team_rect.h,
                            h: team_rect.h,
                        };
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
            let now_inside = pressed == hit;
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
                } else if !self.pointer_inside_pressed
                    && now_inside
                    && previous_hover != hit
                {
                    let key_already_down = self
                        .key_pressed
                        .is_some_and(|(pressed_control, _)| pressed_control == control);
                    self.pointer_inside_pressed = true;
                    if !key_already_down {
                        self.sounds.push(LobbySound::ArrowHit);
                    }
                }
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
        let hit = self.hit_test(point, layout, roster);
        self.hovered = hit;
        self.hover_since = Instant::now();
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
                self.change_focus(LobbyControl::ChatInput, false);
                let mut actions = vec![LobbyAction::Chat(LobbyChatRequest::FocusInput)];
                if changed {
                    actions.insert(0, LobbyAction::FocusChanged(LobbyControl::ChatInput));
                }
                self.append_game_option_focus_clear(previous_focus, &mut actions);
                return actions;
            }
            HitTarget::RightCaption => {
                let changed = self.focus != LobbyControl::Roster;
                self.change_focus(LobbyControl::Roster, false);
                if changed {
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
                let changed = self.focus != LobbyControl::Roster;
                self.change_focus(LobbyControl::Roster, false);
                self.sounds.push(LobbySound::ArrowHit);
                self.pointer_pressed = Some(hit);
                if changed {
                    let mut actions = vec![LobbyAction::FocusChanged(LobbyControl::Roster)];
                    self.append_game_option_focus_clear(previous_focus, &mut actions);
                    return actions;
                }
            }
            HitTarget::RosterScrollBottom => {
                let changed = self.focus != LobbyControl::Roster;
                self.change_focus(LobbyControl::Roster, false);
                self.sounds.push(LobbySound::ArrowHit);
                self.pointer_pressed = Some(hit);
                if changed {
                    let mut actions = vec![LobbyAction::FocusChanged(LobbyControl::Roster)];
                    self.append_game_option_focus_clear(previous_focus, &mut actions);
                    return actions;
                }
            }
            HitTarget::RosterScrollTrack => {
                let changed = self.focus != LobbyControl::Roster;
                self.change_focus(LobbyControl::Roster, false);
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
            HitTarget::RosterScrollInert => {
                let changed = self.focus != LobbyControl::Roster;
                self.change_focus(LobbyControl::Roster, false);
                if changed {
                    let mut actions = vec![LobbyAction::FocusChanged(LobbyControl::Roster)];
                    self.append_game_option_focus_clear(previous_focus, &mut actions);
                    return actions;
                }
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
        let hit = self.hit_test(point, layout, roster);
        self.hovered = hit;
        self.scrollbar_drag = None;
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
        if pressed != Some(hit) && self.pointer_inside_pressed {
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
        if pressed != Some(hit) || !button_was_down {
            return Vec::new();
        }
        if pressed.is_some() {
            self.sounds.push(LobbySound::Click);
        }
        if let Some(control) = hit.button_control() {
            if self
                .key_pressed
                .is_some_and(|(pressed_control, _)| pressed_control == control)
            {
                self.key_pressed = None;
            }
        }
        self.activate_hit(hit)
    }

    pub fn pointer_secondary_down(
        &mut self,
        point: GuiPoint,
        layout: &LobbyLayout,
        roster: &LobbyRosterLayout,
    ) -> Vec<LobbyAction> {
        match self.hit_test(point, layout, roster) {
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
        if self.hit_test(point, layout, roster) == HitTarget::ChatInput {
            let previous = self.focus;
            let changed = self.focus != LobbyControl::ChatInput;
            self.change_focus(LobbyControl::ChatInput, false);
            let mut actions = vec![LobbyAction::Chat(LobbyChatRequest::PointerDoubleClick(
                point,
            ))];
            if changed {
                actions.insert(0, LobbyAction::FocusChanged(LobbyControl::ChatInput));
            }
            self.append_game_option_focus_clear(previous, &mut actions);
            actions
        } else {
            Vec::new()
        }
    }

    pub fn pointer_middle_down(
        &mut self,
        point: GuiPoint,
        layout: &LobbyLayout,
        roster: &LobbyRosterLayout,
    ) -> Vec<LobbyAction> {
        if self.hit_test(point, layout, roster) == HitTarget::ChatInput {
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
            self.roster_scroll = (self.roster_scroll - delta).clamp(0, roster.max_scroll);
            self.roster_scroll_pin = scroll_to_pin(
                self.roster_scroll,
                roster.max_scroll,
                scrollbar_max_pin(layout.roster_scrollbar),
            );
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
                _ => {}
            },
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
            | LobbyControl::Run)
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
            self.change_focus(LobbyControl::ChatInput, false);
            let mut actions = vec![LobbyAction::Chat(LobbyChatRequest::FocusInput)];
            if changed {
                actions.insert(0, LobbyAction::FocusChanged(LobbyControl::ChatInput));
            }
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
        if self.focus == LobbyControl::Roster && vertical != 0 {
            return self.move_selection(vertical.signum() as i32, layout, roster);
        }
        if horizontal != 0 || vertical != 0 {
            self.focus_next(horizontal < 0 || vertical < 0)
        } else {
            Vec::new()
        }
    }

    pub fn request_focused_context(&self, position: GuiPoint) -> Vec<LobbyAction> {
        if self.focus != LobbyControl::Roster {
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
                    color: [255, 31, 31, 255],
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
                    color: [255, 31, 31, 255],
                };
                self.logs.push(line.clone());
                self.chat_follow_bottom = true;
                self.sounds.push(LobbySound::Command);
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
                    if header.kind == LobbyRosterHeader::ScriptPlayers && header.can_add_player =>
                {
                    order.push(LobbyControl::RosterAddPlayer);
                }
                _ => {}
            }
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

    fn move_selection(
        &mut self,
        direction: i32,
        layout: &LobbyLayout,
        roster: &LobbyRosterLayout,
    ) -> Vec<LobbyAction> {
        if self.rows.is_empty() {
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

    fn select_row(
        &mut self,
        selected: Option<usize>,
        by_user: bool,
        layout: &LobbyLayout,
        roster: &LobbyRosterLayout,
    ) -> Vec<LobbyAction> {
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
            HitTarget::AddPlayer(index) => match self.rows.get(index) {
                Some(LobbyRosterRow::Client(client)) => {
                    vec![LobbyAction::AddPlayerRequested {
                        client_id: client.id,
                    }]
                }
                Some(LobbyRosterRow::Header(header))
                    if header.kind == LobbyRosterHeader::ScriptPlayers =>
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
            _ => Vec::new(),
        }
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
            LobbyControl::RosterTeam => self
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
            LobbyControl::RosterAddPlayer => self
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
        self.roster_scroll_pin = pin;
        self.roster_scroll = if max_pin == 0 {
            0
        } else {
            roster.max_scroll * pin / max_pin
        };
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
                self.roster_scroll_pin = (self.roster_scroll_pin - 1).max(0);
                self.roster_scroll = pin_to_scroll(
                    self.roster_scroll_pin,
                    roster_max_scroll,
                    scrollbar_max_pin(layout.roster_scrollbar),
                );
            }
            Some(HitTarget::RosterScrollBottom) => {
                let max_pin = scrollbar_max_pin(layout.roster_scrollbar);
                self.roster_scroll_pin = (self.roster_scroll_pin + 1).min(max_pin);
                self.roster_scroll =
                    pin_to_scroll(self.roster_scroll_pin, roster_max_scroll, max_pin);
            }
            _ => {}
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
        if contains(layout.roster_client, point) {
            return HitTarget::RosterBlank;
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
        if now
            .checked_duration_since(self.hover_since)
            .unwrap_or_default()
            < TOOLTIP_DELAY
        {
            return None;
        }
        let text = match self.hovered {
            HitTarget::ChatInput | HitTarget::ChatLabel => self.labels.tooltip_chat.clone(),
            HitTarget::Exit => self.labels.tooltip_exit.clone(),
            HitTarget::Run => self.labels.tooltip_start.clone(),
            HitTarget::Ready if self.resources_loaded => self.labels.tooltip_ready.clone(),
            HitTarget::Ready => self.labels.tooltip_ready_unavailable.clone(),
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
                    },
                    _ => return None,
                }
            }
            _ => return None,
        };
        let pointer = self.pointer?;
        (!text.is_empty()).then_some(LobbyTooltip { pointer, text })
    }

    pub fn tooltip_state_with_roster_at(
        &self,
        now: Instant,
        roster: &LobbyRosterLayout,
        font: &lc_graphics::clonk_font::ClonkFont,
    ) -> Option<LobbyTooltip> {
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
            let ping_rect = IntRect {
                x: row_layout.rect.x + row_layout.rect.w - width,
                y: row_layout.rect.y,
                w: width,
                h: height,
            };
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
        let roster = self.roster_layout(&layout, resources.fonts.text.line_height);
        let _ = self.chat_scroll_metrics(&layout, &resources.fonts.text);
        self.advance_held_scrollbars(&layout, roster.max_scroll);
        let roster = self.roster_layout(&layout, resources.fonts.text.line_height);
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

        let (players_title, _) = expand_hotkey_markup(&self.players_title());
        skin.draw_caption(
            surface,
            layout.right_caption,
            &players_title,
            &resources.fonts.text,
            COLOR_WHITE,
            TextAlign::Left,
            gamma,
        );
        for tab in &layout.tab_buttons {
            self.draw_tab_button(surface, *tab, resources, active, gamma);
        }

        draw_engine_box(
            surface,
            layout.right_tab.x,
            layout.right_tab.y,
            layout.right_tab.x + layout.right_tab.w - 1,
            layout.right_tab.y + layout.right_tab.h - 1,
            STANDARD_BACKGROUND_COLOR,
            gamma,
        );
        draw_3d_frame(surface, layout.right_tab, gamma);
        draw_engine_box(
            surface,
            layout.roster.x,
            layout.roster.y,
            layout.roster.x + layout.roster.w - 1,
            layout.roster.y + layout.roster.h - 1,
            DARK_BACKGROUND,
            gamma,
        );
        self.draw_roster(surface, &layout, &roster, resources, active, gamma)?;
        draw_scrollbar(
            surface,
            layout.roster_scrollbar,
            resources.scroll,
            self.roster_scroll_pin,
            roster.max_scroll,
            self.pointer_pressed == Some(HitTarget::RosterScrollTop),
            self.pointer_pressed == Some(HitTarget::RosterScrollBottom),
            gamma,
        );

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
        option_buttons.render_tooltip(surface, option_resources, active, gamma)?;
        if active {
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
                    draw_engine_box(
                        surface,
                        row_layout.rect.x + 10,
                        y,
                        row_layout.rect.x + layout.roster_client.w - 11,
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
            if selectable {
                draw_source_clipped(
                    surface,
                    resources.context,
                    (0, 0, 16, 16),
                    IntRect {
                        w: 16,
                        h: 16,
                        ..team_rect
                    },
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
                && (self.hovered == HitTarget::Team(row.index)
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
            let marker = IntRect {
                x: layout.ready_square.x + layout.ready_square.w / 4,
                y: layout.ready_square.y + layout.ready_square.h / 4,
                w: layout.ready_square.w / 2,
                h: layout.ready_square.h / 2,
            };
            draw_highlight(surface, marker, &resources.button_highlight, gamma);
        }
    }

    fn button_state(&self, control: LobbyControl, active: bool) -> ClassicButtonState {
        let target = match control {
            LobbyControl::Exit => HitTarget::Exit,
            LobbyControl::Run => HitTarget::Run,
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
    font: &lc_graphics::clonk_font::ClonkFont,
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
    let clip = IntRect {
        x: client.x - 2,
        y: client.y,
        w: client.w + 4,
        h: client.h + 1,
    };
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
fn draw_clipped_text_mode(
    surface: &mut Surface,
    font: &lc_graphics::clonk_font::ClonkFont,
    x: i32,
    y: i32,
    text: &str,
    color: [u8; 4],
    align: TextAlign,
    gamma: Option<&GammaRamp>,
    clip: IntRect,
    markup: bool,
) {
    if markup {
        draw_clipped_text(surface, font, x, y, text, color, align, gamma, clip);
        return;
    }
    let left = clip.x.max(0);
    let top = clip.y.max(0);
    let right = (clip.x + clip.w).min(surface.width() as i32);
    let bottom = (clip.y + clip.h).min(surface.height() as i32);
    if left >= right || top >= bottom {
        return;
    }
    let (width, height) = ((right - left) as u32, (bottom - top) as u32);
    let mut scratch = Surface::new(width, height, PixelFormat::Rgba8888);
    for target_y in 0..height {
        for target_x in 0..width {
            if let Some(pixel) = surface.get_pixel(left as u32 + target_x, top as u32 + target_y) {
                let _ = scratch.set_pixel(target_x, target_y, pixel);
            }
        }
    }
    font.draw_with_gamma(
        &mut scratch,
        x - left,
        y - top,
        text,
        color,
        align,
        false,
        gamma,
    );
    for target_y in 0..height {
        for target_x in 0..width {
            if let Some(pixel) = scratch.get_pixel(target_x, target_y) {
                let _ = surface.set_pixel(left as u32 + target_x, top as u32 + target_y, pixel);
            }
        }
    }
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
            draw_image_clipped(surface, image, rect, clip, gamma);
        }
    }
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
    IntRect {
        x: rect.x + 4,
        y: rect.y + 2,
        w: (rect.w - 8).max(0),
        h: (rect.h - 4).max(0),
    }
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

/// `CStdFont::BreakMessage(..., fCheckMarkup=true)` closes active markup at
/// an inserted wrap and reopens it on the continuation line. Lobby rows draw
/// physical lines independently, so retain that state explicitly.
fn lobby_markup_lines(text: &str) -> Vec<String> {
    let mut lines = Vec::new();
    let mut line = String::new();
    let mut open: Vec<(String, String)> = Vec::new();
    let mut rest = text;
    while !rest.is_empty() {
        if rest.starts_with('\n') {
            for (name, _) in open.iter().rev() {
                line.push_str("</");
                line.push_str(name);
                line.push('>');
            }
            lines.push(std::mem::take(&mut line));
            for (_, markup) in &open {
                line.push_str(markup);
            }
            rest = &rest[1..];
            continue;
        }
        if rest.starts_with('<') {
            if let Some(end) = rest.find('>') {
                let raw = &rest[..=end];
                let contents = &rest[1..end];
                let opening = if contents == "i" {
                    Some("i")
                } else if let Some(color) = contents.strip_prefix("c ") {
                    (color.len() <= 8
                        && color
                            .bytes()
                            .all(|byte| byte.is_ascii_digit() || matches!(byte, b'a'..=b'f')))
                    .then_some("c")
                } else {
                    None
                };
                let closing = match contents {
                    "/i" => Some("i"),
                    "/c" => Some("c"),
                    _ => None,
                };
                if let Some(name) = opening {
                    line.push_str(raw);
                    open.push((name.to_string(), raw.to_string()));
                    rest = &rest[end + 1..];
                    continue;
                }
                if closing.is_some_and(|name| {
                    open.last()
                        .is_some_and(|(open_name, _)| open_name == name)
                }) {
                    line.push_str(raw);
                    open.pop();
                    rest = &rest[end + 1..];
                    continue;
                }
            }
        }
        let character = rest.chars().next().expect("non-empty markup text");
        line.push(character);
        rest = &rest[character.len_utf8()..];
    }
    lines.push(line);
    lines
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

/// Edit draws its flashing broken-bar cursor through `TextOut(..., 1.5f)`.
fn draw_scaled_caret(
    surface: &mut Surface,
    font: &lc_graphics::clonk_font::ClonkFont,
    x: i32,
    y: i32,
    clip: IntRect,
    gamma: Option<&GammaRamp>,
) {
    const SCALE: f32 = 1.5;
    let Some(glyph) = font.glyph('\u{a6}') else {
        return;
    };
    let Ok(width) = u32::try_from(glyph.width) else {
        return;
    };
    let Ok(height) = u32::try_from(font.cell_height) else {
        return;
    };
    if width == 0 || height == 0 {
        return;
    }
    let mut pixels = Vec::with_capacity(glyph.pixels.len() * 4);
    for pixel in &glyph.pixels {
        let (red, green, blue) = if pixel.a == 0 {
            (255, 255, 255)
        } else {
            (pixel.r, pixel.g, pixel.b)
        };
        pixels.extend_from_slice(&[red, green, blue, pixel.a]);
    }
    if pixels.len() != width as usize * height as usize * 4 {
        return;
    }
    let image = ImageData::new(width, height, pixels);
    let destination = (
        x as f32,
        y as f32,
        width as f32 * SCALE,
        height as f32 * SCALE,
    );
    let left = destination.0.max(clip.x as f32);
    let top = destination.1.max(clip.y as f32);
    let right = (destination.0 + destination.2).min((clip.x + clip.w) as f32);
    let bottom = (destination.1 + destination.3).min((clip.y + clip.h) as f32);
    if left >= right || top >= bottom {
        return;
    }
    draw_facet_stretch(
        surface,
        &image,
        (
            (left - destination.0) / SCALE,
            (top - destination.1) / SCALE,
            (right - left) / SCALE,
            (bottom - top) / SCALE,
        ),
        (left, top, right - left, bottom - top),
        gamma,
    );
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
    IntRect {
        x,
        y,
        w: (right - x).max(0),
        h: (bottom - y).max(0),
    }
}

fn gui_rect(rect: IntRect) -> GuiRect {
    GuiRect::new(rect.x as f32, rect.y as f32, rect.w as f32, rect.h as f32)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_support::endeavour_font_set;

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
        assert_eq!(
            lobby_markup_lines("<c 123456>sender <i>long\ncontinuation</i></c>"),
            vec![
                "<c 123456>sender <i>long</i></c>",
                "<c 123456><i>continuation</i></c>",
            ]
        );
        assert_eq!(
            lobby_markup_lines("<c RED>literal\ncontinuation"),
            vec!["<c RED>literal", "continuation"]
        );
        assert_eq!(
            lobby_markup_lines("<c ff><i>x</c>\ny"),
            vec![
                "<c ff><i>x</c></i></c>",
                "<c ff><i>y",
            ]
        );
    }

    #[test]
    fn normal_host_layout_matches_constructor_math() {
        let layout = game_lobby_layout(1280, 720, 34, 22, LobbyRole::Host, false, false);
        assert_eq!(
            layout.client,
            IntRect {
                x: 25,
                y: 69,
                w: 1230,
                h: 632
            }
        );
        assert_eq!(layout.title_anchor, (640, 8));
        assert_eq!(
            layout.chat_log,
            IntRect {
                x: 45,
                y: 77,
                w: 780,
                h: 491
            }
        );
        assert_eq!(
            layout.chat_label,
            IntRect {
                x: 45,
                y: 584,
                w: 40,
                h: 25
            }
        );
        assert_eq!(
            layout.chat_edit,
            IntRect {
                x: 85,
                y: 584,
                w: 740,
                h: 25
            }
        );
        assert_eq!(
            layout.right_caption,
            IntRect {
                x: 850,
                y: 77,
                w: 400,
                h: 23
            }
        );
        assert_eq!(
            layout.right_tab,
            IntRect {
                x: 850,
                y: 99,
                w: 400,
                h: 510
            }
        );
        assert_eq!(
            layout.roster,
            IntRect {
                x: 854,
                y: 103,
                w: 392,
                h: 502
            }
        );
        assert_eq!(
            layout.exit_button,
            IntRect {
                x: 35,
                y: 633,
                w: 100,
                h: 32
            }
        );
        assert_eq!(
            layout.run_button,
            Some(IntRect {
                x: 1145,
                y: 633,
                w: 100,
                h: 32
            })
        );
        assert_eq!(
            layout.ready_checkbox,
            IntRect {
                x: 1015,
                y: 633,
                w: 110,
                h: 32
            }
        );
        assert_eq!(
            layout.game_option_strip,
            IntRect {
                x: 155,
                y: 617,
                w: 840,
                h: 64
            }
        );
        assert_eq!(
            layout.tab_buttons[0].rect,
            IntRect {
                x: 1170,
                y: 81,
                w: 16,
                h: 16
            }
        );
        assert_eq!(
            layout.tab_buttons[3].rect,
            IntRect {
                x: 1230,
                y: 81,
                w: 16,
                h: 16
            }
        );
    }

    #[test]
    fn client_variant_recenters_options_without_go_button() {
        let layout = game_lobby_layout(1280, 720, 34, 22, LobbyRole::Client, false, false);
        assert_eq!(layout.run_button, None);
        assert_eq!(
            layout.ready_checkbox,
            IntRect {
                x: 1135,
                y: 633,
                w: 110,
                h: 32
            }
        );
        assert_eq!(
            layout.game_option_strip,
            IntRect {
                x: 185,
                y: 617,
                w: 930,
                h: 64
            }
        );
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
        assert_eq!(lobby.take_sounds(), [LobbySound::Command]);
        let _ = lobby.apply_countdown_packet(LobbyCountdownPacket::Seconds(10));
        assert_eq!(
            lobby.take_sounds(),
            [
                LobbySound::Fuse,
                LobbySound::StartElevatorLoop,
                LobbySound::Command,
            ]
        );
        let actions = lobby.apply_countdown_packet(LobbyCountdownPacket::Seconds(9));
        assert!(matches!(
            actions.last(),
            Some(LobbyAction::AppendLog(LobbyLogLine { text, .. })) if text == "9..."
        ));
        let _ = lobby.apply_countdown_packet(LobbyCountdownPacket::Abort);
        assert_eq!(
            lobby.take_sounds(),
            [
                LobbySound::Command,
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
            let layout =
                game_lobby_layout(1280, 720, 34, 22, LobbyRole::Host, false, false);
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
            let layout =
                game_lobby_layout(1280, 720, 34, 22, LobbyRole::Host, false, false);
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

        let _ = lobby.apply_countdown_packet(LobbyCountdownPacket::Seconds(11));
        assert!(lobby
            .pointer_down(point, &layout, &roster)
            .contains(&LobbyAction::TeamSelectionRequested { player_id: 2 }));
        lobby.focus = LobbyControl::RosterTeam;
        let actions = lobby.apply_countdown_packet(LobbyCountdownPacket::Seconds(10));
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
        assert_eq!(
            roster.rows[0].team,
            Some(IntRect {
                x: 917,
                y: 129,
                w: 285,
                h: 26,
            }),
            "team 0 keeps the exact blank-combo geometry"
        );
        assert_eq!(
            roster.rows[0].rank,
            Some(IntRect {
                x: 1202,
                y: 129,
                w: 26,
                h: 26,
            }),
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
