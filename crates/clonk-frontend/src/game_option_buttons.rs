//! Reusable classic `C4GameOptionButtons` controller and renderer.
//!
//! This mirrors `src/C4Network2Dialogs.cpp:586-816`: the scenario selector
//! and lobby embed the same icon-button strip with different network/host
//! flags. Configuration, network control packets, logging, and child input
//! dialogs remain application-owned and are represented by typed actions.

use std::time::{Duration, Instant};

use anyhow::{ensure, Result};
use clonk_graphics::clonk_font::ClonkFont;
use clonk_graphics::{GammaRamp, Surface};
use clonk_gui::Rect as GuiRect;

use crate::classic_gui::{blacken_transparent_pixels, draw_facet_stretch, IntRect};
use crate::context_menu::draw_classic_tooltip;
use crate::{draw_image_bilinear_additive, GuiPoint, ImageData, KeyCode};

pub const INTERNET_TOOLTIP: &str =
    "Internet game: other players can see this round on the internet.";
pub const LEAGUE_TOOLTIP: &str = "League game: this round will be evaluated in the league.";
pub const PASSWORD_TOOLTIP: &str =
    "Password protection: other players can only join with a password.";
pub const COMMENT_TOOLTIP: &str =
    "Comment: description for this round which can be seen by other players.";
pub const FAIR_CREW_TOOLTIP: &str = "Fair clonks: All Clonks have the same strength.";
pub const NORMAL_CREW_TOOLTIP: &str =
    "Trained Clonks: Clonks have different strength according to their rank.";
pub const RECORD_TOOLTIP: &str = "Record game: the round is recorded for later playback.";
pub const COMMENT_CHANGED_LOG: &str = "Network game comment adjusted.";

pub const PASSWORD_INPUT_MESSAGE: &str = "Enter password:";
pub const PASSWORD_INPUT_CAPTION: &str = "Password";
pub const COMMENT_INPUT_MESSAGE: &str = "Please enter the desired comment for this game:";
pub const COMMENT_INPUT_CAPTION: &str = "Comment";
pub const PASSWORD_MAX_TEXT: usize = 1024;
pub const COMMENT_MAX_TEXT: usize = 256;

const ICON_CELL: u32 = 64;
const ICON_COLUMNS: u32 = 4;
const ICON_SHEET_WIDTH: u32 = 256;
const ICON_SHEET_HEIGHT: u32 = 320;
#[cfg(test)]
const HIGHLIGHT_WIDTH: u32 = 16;
#[cfg(test)]
const HIGHLIGHT_HEIGHT: u32 = 16;
const TOOLTIP_DELAY: Duration = Duration::from_millis(500);

/// The four actual constructor contexts used by startup and the lobby.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum GameOptionContext {
    /// `C4StartupScenSelDlg(..., fNetwork=false)`: Fair Crew, Record.
    LocalSelector,
    /// `C4StartupScenSelDlg(..., fNetwork=true)`: all six buttons.
    NetworkHostSelector,
    /// Network lobby owned by the host: all six buttons.
    LobbyHost,
    /// Network lobby joined as a client: League, Fair Crew, Record.
    LobbyClient,
}

impl GameOptionContext {
    pub const fn is_network(self) -> bool {
        !matches!(self, Self::LocalSelector)
    }

    pub const fn is_host(self) -> bool {
        !matches!(self, Self::LobbyClient)
    }

    pub const fn is_lobby(self) -> bool {
        matches!(self, Self::LobbyHost | Self::LobbyClient)
    }

    pub const fn button_count(self) -> usize {
        if !self.is_network() {
            2
        } else if self.is_host() {
            6
        } else {
            3
        }
    }

    pub const fn buttons(self) -> &'static [GameOptionButton] {
        const LOCAL: [GameOptionButton; 2] = [GameOptionButton::FairCrew, GameOptionButton::Record];
        const NETWORK_HOST: [GameOptionButton; 6] = [
            GameOptionButton::Internet,
            GameOptionButton::League,
            GameOptionButton::Password,
            GameOptionButton::Comment,
            GameOptionButton::FairCrew,
            GameOptionButton::Record,
        ];
        const LOBBY_CLIENT: [GameOptionButton; 3] = [
            GameOptionButton::League,
            GameOptionButton::FairCrew,
            GameOptionButton::Record,
        ];
        match self {
            Self::LocalSelector => &LOCAL,
            Self::NetworkHostSelector | Self::LobbyHost => &NETWORK_HOST,
            Self::LobbyClient => &LOBBY_CLIENT,
        }
    }
}

/// Construction/add order and mnemonic of each icon button.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum GameOptionButton {
    Internet,
    League,
    Password,
    Comment,
    FairCrew,
    Record,
}

impl GameOptionButton {
    pub const fn hotkey(self) -> char {
        match self {
            Self::Internet => 'I',
            Self::League => 'L',
            Self::Password => 'P',
            Self::Comment => 'M',
            Self::FairCrew => 'F',
            Self::Record => 'R',
        }
    }
}

/// `GUIIcons2.png` phase numbers from `C4GUI::Icons`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[repr(u16)]
pub enum GameOptionIcon {
    RecordOff = 0,
    RecordOn = 1,
    FairCrew = 2,
    NormalCrew = 3,
    LeagueOff = 4,
    LeagueOn = 5,
    InternetOff = 6,
    InternetOn = 7,
    FairCrewGray = 9,
    NormalCrewGray = 10,
    Locked = 11,
    Unlocked = 12,
    LockedFrontal = 13,
    Comment = 17,
}

impl GameOptionIcon {
    pub const fn phase(self) -> u16 {
        self as u16
    }
}

/// Scenario-side `C4SForceFairCrew` state.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum FairCrewConstraint {
    #[default]
    Free,
    ForceFair,
    ForceNormal,
}

/// External values sampled by the C++ constructor and update methods.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct GameOptionValues {
    pub master_server_signup: bool,
    pub league_server_signup: bool,
    /// `Game.Parameters.isLeague()`, used only in lobby contexts.
    pub lobby_is_league: bool,
    pub password: String,
    pub last_password: String,
    pub comment: String,
    /// Selector: `Config.General.FairCrew`; lobby: `Parameters.UseFairCrew`.
    pub fair_crew: bool,
    /// Selector-only scenario constraint.
    pub selector_fair_crew_constraint: FairCrewConstraint,
    /// Lobby-only `Parameters.FairCrewForced`.
    pub lobby_fair_crew_forced: bool,
    pub fair_crew_strength: i32,
    pub record: bool,
    pub countdown: bool,
}

impl Default for GameOptionValues {
    fn default() -> Self {
        Self {
            master_server_signup: false,
            league_server_signup: false,
            lobby_is_league: false,
            password: String::new(),
            last_password: String::new(),
            comment: String::new(),
            fair_crew: false,
            selector_fair_crew_constraint: FairCrewConstraint::Free,
            lobby_fair_crew_forced: false,
            fair_crew_strength: 0,
            record: false,
            countdown: false,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum GameOptionInputKind {
    Password,
    Comment,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum GameOptionInputDialogResult {
    Submitted(String),
    Cancelled,
}

/// Exact app-owned `C4GUI::InputDialog` request.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct GameOptionInputDialogRequest {
    pub kind: GameOptionInputKind,
    pub message: &'static str,
    pub caption: &'static str,
    pub icon: GameOptionIcon,
    pub max_text: usize,
    pub initial_text: String,
    /// Both C++ calls pass `false` to `InputDialog`'s chat-layout flag.
    pub chat_layout: bool,
}

/// App-owned effects requested by the frontend controller.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum GameOptionAction {
    /// The strip reached its first/last child during the enclosing dialog's
    /// recursive focus traversal. The host must focus the preceding/following
    /// control outside this embedded window.
    FocusTraversalRequested {
        backwards: bool,
    },
    /// Update `Config.Network.MasterServerSignUp`. In a lobby the app must
    /// also call `LeagueSignupEnable/Disable`; feed a failed enable back via
    /// [`GameOptionButtons::apply_lobby_internet_result`].
    InternetSignupChanged {
        enabled: bool,
        live_lobby: bool,
    },
    LeagueSignupChanged(bool),
    ShowInputDialog(GameOptionInputDialogRequest),
    /// Set/clear `Game.Network`'s password. Non-empty submissions are also
    /// copied to `Config.Network.LastPassword`.
    PasswordChanged {
        password: String,
        remember_for_next_round: Option<String>,
    },
    /// Copy-validate the comment, invalidate the network reference, log
    /// [`COMMENT_CHANGED_LOG`], and play the queued Connect sound.
    CommentChanged(String),
    FairCrewPreferenceChanged(bool),
    /// Synchronous `C4ControlSet(C4CVT_FairCrew, value)`: `-1` disables fair
    /// crew; `FairCrewStrength` enables it.
    SendLobbyFairCrewControl {
        value: i32,
    },
    RecordPreferenceChanged(bool),
}

/// Result of routing a keyboard or low gamepad-button event through the
/// focused option button.
///
/// `captured == false` is significant: the C++ button binding returns false
/// for disabled buttons, allowing lower-priority bindings on the enclosing
/// dialog (notably Enter) to handle the same event.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct GameOptionKeyOutcome {
    pub captured: bool,
    pub actions: Vec<GameOptionAction>,
}

impl GameOptionKeyOutcome {
    const fn passed() -> Self {
        Self {
            captured: false,
            actions: Vec::new(),
        }
    }

    fn captured(actions: Vec<GameOptionAction>) -> Self {
        Self {
            captured: true,
            actions,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum GameOptionSound {
    ArrowHit,
    Click,
    Connect,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum GameOptionGamepadDirection {
    Left,
    Right,
    Up,
    Down,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct GameOptionButtonLayout {
    pub button: GameOptionButton,
    pub rect: IntRect,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct GameOptionButtonsLayout {
    pub bounds: IntRect,
    pub icon_size: i32,
    pub icon_spacing: i32,
    pub buttons: Vec<GameOptionButtonLayout>,
}

impl GameOptionButtonsLayout {
    pub fn rect(&self, button: GameOptionButton) -> Option<IntRect> {
        self.buttons
            .iter()
            .find(|entry| entry.button == button)
            .map(|entry| entry.rect)
    }
}

/// Exact `ComponentAligner` constructor geometry from
/// `C4GameOptionButtons::C4GameOptionButtons`.
pub fn game_option_buttons_layout(
    bounds: IntRect,
    context: GameOptionContext,
) -> GameOptionButtonsLayout {
    let count = context.button_count() as i32;
    let mut icon_size = 64.min(bounds.h);
    let mut icon_spacing = bounds.w / if bounds.w >= 400 { 64 } else { 128 };
    if (icon_size + icon_spacing * 2) * count > bounds.w {
        if icon_size * count <= bounds.w {
            icon_spacing = 0.max((bounds.w - icon_size * count) / (count * 2) - 1);
        } else {
            icon_spacing = 0;
            icon_size = bounds.w / count;
        }
    }
    let group_width = (icon_size + icon_spacing * 2) * count;
    let group_x = bounds.x + bounds.w / 2 - group_width / 2;
    let group_y = bounds.y + bounds.h / 2 - icon_size / 2;
    let pitch = icon_size + icon_spacing * 2;
    let buttons = context
        .buttons()
        .iter()
        .copied()
        .enumerate()
        .map(|(index, button)| GameOptionButtonLayout {
            button,
            rect: IntRect {
                x: group_x + icon_spacing + index as i32 * pitch,
                y: group_y,
                w: icon_size,
                h: icon_size,
            },
        })
        .collect();
    GameOptionButtonsLayout {
        bounds,
        icon_size,
        icon_spacing,
        buttons,
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct GameOptionButtonView {
    pub button: GameOptionButton,
    pub rect: IntRect,
    pub icon: GameOptionIcon,
    pub enabled: bool,
    pub tooltip: &'static str,
}

/// Delayed tooltip state for the host's final, screen-global overlay pass.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct GameOptionTooltip {
    pub pointer: GuiPoint,
    pub text: &'static str,
}

/// Exact classic resources. Construction fails instead of substituting a
/// generic pane or icon sheet.
#[derive(Clone)]
pub struct GameOptionButtonResources<'a> {
    icons_extended: ImageData,
    button_highlight: ImageData,
    tooltip_font: &'a ClonkFont,
}

impl<'a> GameOptionButtonResources<'a> {
    pub fn new(
        icons_extended: &ImageData,
        button_highlight: &ImageData,
        tooltip_font: &'a ClonkFont,
    ) -> Result<Self> {
        ensure!(
            (icons_extended.width(), icons_extended.height())
                == (ICON_SHEET_WIDTH, ICON_SHEET_HEIGHT),
            "GUIIcons2.png must be the exact 256x320 classic sheet: got {}x{}",
            icons_extended.width(),
            icons_extended.height()
        );
        ensure!(
            button_highlight.width() > 0 && button_highlight.height() > 0,
            "GUIButtonHighlight.png must be a non-empty full-size classic facet: got {}x{}",
            button_highlight.width(),
            button_highlight.height()
        );
        ensure!(
            tooltip_font.line_height > 0,
            "classic tooltip font has no line height"
        );
        Ok(Self {
            icons_extended: blacken_transparent_pixels(icons_extended),
            button_highlight: blacken_transparent_pixels(button_highlight),
            tooltip_font,
        })
    }

    fn validate(&self) -> Result<()> {
        ensure!(
            (self.icons_extended.width(), self.icons_extended.height())
                == (ICON_SHEET_WIDTH, ICON_SHEET_HEIGHT),
            "GUIIcons2.png must be the exact 256x320 classic sheet: got {}x{}",
            self.icons_extended.width(),
            self.icons_extended.height()
        );
        ensure!(
            self.button_highlight.width() > 0 && self.button_highlight.height() > 0,
            "GUIButtonHighlight.png must be a non-empty full-size classic facet: got {}x{}",
            self.button_highlight.width(),
            self.button_highlight.height()
        );
        ensure!(
            self.tooltip_font.line_height > 0,
            "classic tooltip font has no line height"
        );
        Ok(())
    }
}

/// Pure presentation/input state for one embedded option-button strip.
#[derive(Clone)]
pub struct GameOptionButtons {
    context: GameOptionContext,
    values: GameOptionValues,
    bounds: IntRect,
    focused: Option<GameOptionButton>,
    pointer: Option<GuiPoint>,
    hovered: Option<GameOptionButton>,
    pointer_pressed: Option<GameOptionButton>,
    pointer_down_visual: bool,
    key_pressed: Option<(GameOptionButton, KeyCode)>,
    pointer_active: bool,
    tooltip_since: Instant,
    sounds: Vec<GameOptionSound>,
}

impl GameOptionButtons {
    pub fn new(context: GameOptionContext, values: GameOptionValues) -> Self {
        Self {
            context,
            values,
            bounds: IntRect {
                x: 0,
                y: 0,
                w: 1,
                h: 1,
            },
            focused: None,
            pointer: None,
            hovered: None,
            pointer_pressed: None,
            pointer_down_visual: false,
            key_pressed: None,
            pointer_active: false,
            tooltip_since: Instant::now(),
            sounds: Vec::new(),
        }
    }

    pub const fn context(&self) -> GameOptionContext {
        self.context
    }

    pub fn values(&self) -> &GameOptionValues {
        &self.values
    }

    pub fn set_bounds(&mut self, bounds: IntRect) {
        let previous_hover = self.hovered;
        self.bounds = bounds;
        self.hovered = self.pointer.and_then(|point| self.hit_test(point));
        self.refresh_pointer_down_visual(previous_hover != self.hovered);
    }

    pub fn layout(&self) -> GameOptionButtonsLayout {
        game_option_buttons_layout(self.bounds, self.context)
    }

    pub const fn focused_button(&self) -> Option<GameOptionButton> {
        self.focused
    }

    pub fn set_focused_button(&mut self, button: Option<GameOptionButton>) {
        self.focused = button.filter(|candidate| self.context.buttons().contains(candidate));
        self.key_pressed = None;
    }

    pub const fn hovered_button(&self) -> Option<GameOptionButton> {
        self.hovered
    }

    pub const fn pointer_position(&self) -> Option<GuiPoint> {
        self.pointer
    }

    pub fn view(&self, button: GameOptionButton) -> Option<GameOptionButtonView> {
        let rect = self.layout().rect(button)?;
        let league = self.league_on();
        let (fair, fair_free) = self.fair_crew_state();
        let (icon, enabled, tooltip) = match button {
            GameOptionButton::Internet => (
                if self.internet_on() {
                    GameOptionIcon::InternetOn
                } else {
                    GameOptionIcon::InternetOff
                },
                self.context.is_network()
                    && self.context.is_host()
                    && !(self.context.is_lobby() && league),
                INTERNET_TOOLTIP,
            ),
            GameOptionButton::League => (
                if league {
                    GameOptionIcon::LeagueOn
                } else {
                    GameOptionIcon::LeagueOff
                },
                self.context.is_network() && self.context.is_host() && !self.context.is_lobby(),
                LEAGUE_TOOLTIP,
            ),
            GameOptionButton::Password => (
                if self.values.password.is_empty() {
                    GameOptionIcon::Unlocked
                } else {
                    GameOptionIcon::Locked
                },
                self.context.is_network() && self.context.is_host(),
                PASSWORD_TOOLTIP,
            ),
            GameOptionButton::Comment => (
                GameOptionIcon::Comment,
                self.context.is_network() && self.context.is_host(),
                COMMENT_TOOLTIP,
            ),
            GameOptionButton::FairCrew => (
                match (fair, fair_free) {
                    (true, true) => GameOptionIcon::FairCrew,
                    (false, true) => GameOptionIcon::NormalCrew,
                    (true, false) => GameOptionIcon::FairCrewGray,
                    (false, false) => GameOptionIcon::NormalCrewGray,
                },
                fair_free,
                if fair {
                    FAIR_CREW_TOOLTIP
                } else {
                    NORMAL_CREW_TOOLTIP
                },
            ),
            GameOptionButton::Record => (
                if self.values.record || league {
                    GameOptionIcon::RecordOn
                } else {
                    GameOptionIcon::RecordOff
                },
                !league,
                RECORD_TOOLTIP,
            ),
        };
        Some(GameOptionButtonView {
            button,
            rect,
            icon,
            enabled,
            tooltip,
        })
    }

    pub fn views(&self) -> Vec<GameOptionButtonView> {
        self.context
            .buttons()
            .iter()
            .filter_map(|button| self.view(*button))
            .collect()
    }

    pub fn set_selector_fair_crew_constraint(&mut self, constraint: FairCrewConstraint) {
        self.values.selector_fair_crew_constraint = constraint;
        self.cancel_disabled_presses();
    }

    pub fn set_countdown(&mut self, countdown: bool) {
        self.values.countdown = countdown;
        self.cancel_disabled_presses();
    }

    pub fn set_lobby_fair_crew(&mut self, fair: bool, forced: bool) {
        self.set_lobby_fair_crew_state(fair, self.values.fair_crew_strength, forced);
    }

    /// Refreshes the synchronized lobby Fair Crew parameters in place so a
    /// retained strip keeps its pointer, focus, and tooltip state.
    pub fn set_lobby_fair_crew_state(&mut self, fair: bool, strength: i32, forced: bool) {
        self.values.fair_crew = fair;
        self.values.fair_crew_strength = strength;
        self.values.lobby_fair_crew_forced = forced;
        self.cancel_disabled_presses();
    }

    /// Applies process-local command-line signup overrides without emitting
    /// UI callbacks or persisting them into configuration.
    pub fn set_server_signup(&mut self, master_server: Option<bool>, league_server: Option<bool>) {
        if let Some(enabled) = master_server {
            self.values.master_server_signup = enabled;
        }
        if let Some(enabled) = league_server {
            self.values.league_server_signup = enabled;
        }
    }

    pub fn set_password(&mut self, password: impl Into<String>) {
        self.values.password = password.into();
    }

    pub fn set_comment(&mut self, comment: impl Into<String>) {
        self.values.comment = comment.into();
    }

    /// Commit a live lobby password only after the application has updated
    /// host admission and the advertised reference successfully.
    pub fn apply_lobby_password_result(
        &mut self,
        password: impl Into<String>,
        remember_for_next_round: Option<String>,
    ) {
        debug_assert!(self.context.is_lobby());
        self.values.password = password.into();
        if let Some(password) = remember_for_next_round {
            self.values.last_password = password;
        }
        self.sounds.push(GameOptionSound::Connect);
    }

    /// Commit a validated live lobby comment and queue its callback sound.
    pub fn apply_lobby_comment_result(&mut self, comment: impl Into<String>) {
        debug_assert!(self.context.is_lobby());
        self.values.comment = comment.into();
        self.sounds.push(GameOptionSound::Connect);
    }

    pub fn set_lobby_league(&mut self, league: bool) {
        self.values.lobby_is_league = league;
        if league {
            self.values.master_server_signup = true;
        }
        self.cancel_disabled_presses();
    }

    /// Applies `LeagueSignupEnable()`'s synchronous success/failure result.
    pub fn apply_lobby_internet_result(&mut self, enabled: bool) {
        if self.context.is_lobby() {
            self.values.master_server_signup = enabled;
            if !enabled {
                self.values.league_server_signup = false;
            }
        }
    }

    pub fn take_sound_events(&mut self) -> Vec<GameOptionSound> {
        std::mem::take(&mut self.sounds)
    }

    pub fn tooltip_for_button(&self, button: GameOptionButton) -> Option<&'static str> {
        self.view(button).map(|view| view.tooltip)
    }

    /// Runtime language-table key assigned by C4GameOptionButtons for a
    /// visible button. Fair Crew switches keys with its current state.
    pub fn tooltip_resource_key_for_button(
        &self,
        button: GameOptionButton,
    ) -> Option<&'static str> {
        self.view(button)?;
        Some(match button {
            GameOptionButton::Internet => "IDS_DLGTIP_STARTINTERNETGAME",
            GameOptionButton::League => "IDS_DLGTIP_STARTLEAGUEGAME",
            GameOptionButton::Password => "IDS_NET_PASSWORD_DESC",
            GameOptionButton::Comment => "IDS_DESC_COMMENTDESCRIPTIONFORTHIS",
            GameOptionButton::FairCrew => {
                if self.fair_crew_state().0 {
                    "IDS_CTL_FAIRCREW_DESC"
                } else {
                    "IDS_CTL_NORMALCREW_DESC"
                }
            }
            GameOptionButton::Record => "IDS_DLGTIP_RECORD",
        })
    }

    pub fn tooltip_resource_key_at(&self, point: GuiPoint) -> Option<&'static str> {
        self.tooltip_resource_key_for_button(self.hit_test(point)?)
    }

    /// Resolves the native tooltip target at `point` without consulting or
    /// mutating the legacy delayed-tooltip clock. Disabled buttons retain
    /// their descriptions, while buttons absent from this context have no
    /// hit-test bounds and therefore no tooltip target.
    pub fn tooltip_at(&self, point: GuiPoint) -> Option<&'static str> {
        self.hit_test(point)
            .and_then(|button| self.tooltip_for_button(button))
    }

    pub fn hovered_tooltip_at(&self, now: Instant) -> Option<&'static str> {
        if !self.pointer_active
            || now
                .checked_duration_since(self.tooltip_since)
                .unwrap_or_default()
                < TOOLTIP_DELAY
        {
            return None;
        }
        self.hovered
            .and_then(|button| self.tooltip_for_button(button))
    }

    pub fn tooltip_state_at(&self, now: Instant) -> Option<GameOptionTooltip> {
        Some(GameOptionTooltip {
            pointer: self.pointer?,
            text: self.hovered_tooltip_at(now)?,
        })
    }

    pub fn handle_pointer_move(&mut self, position: GuiPoint) -> Vec<GameOptionAction> {
        let moved = self.pointer.is_none_or(|old| {
            old.x as i32 != position.x as i32 || old.y as i32 != position.y as i32
        });
        self.pointer = Some(position);
        if moved {
            self.pointer_active = true;
            self.tooltip_since = Instant::now();
        }
        let previous_hover = self.hovered;
        self.hovered = self.hit_test(position);
        self.refresh_pointer_down_visual(previous_hover != self.hovered);
        Vec::new()
    }

    pub fn handle_pointer_down(&mut self, position: GuiPoint) -> Vec<GameOptionAction> {
        self.handle_pointer_move(position);
        self.pointer_active = true;
        self.tooltip_since = Instant::now();
        let Some(button) = self.hovered else {
            return Vec::new();
        };
        if !self.view(button).is_some_and(|view| view.enabled) {
            return Vec::new();
        }
        let already_down = self.button_is_down(button);
        self.pointer_pressed = Some(button);
        self.pointer_down_visual = true;
        if !already_down {
            self.sounds.push(GameOptionSound::ArrowHit);
        }
        Vec::new()
    }

    pub fn handle_pointer_up(&mut self, position: GuiPoint) -> Vec<GameOptionAction> {
        self.handle_pointer_move(position);
        self.pointer_active = true;
        self.tooltip_since = Instant::now();
        let pressed = self.pointer_pressed;
        let activate = pressed.is_some()
            && pressed == self.hovered
            && pressed.is_some_and(|button| self.button_is_down(button))
            && pressed
                .and_then(|button| self.view(button))
                .is_some_and(|view| view.enabled);
        self.pointer_pressed = None;
        self.pointer_down_visual = false;
        if activate
            && pressed.is_some_and(|button| {
                self.key_pressed
                    .is_some_and(|(pressed_button, _)| pressed_button == button)
            })
        {
            self.key_pressed = None;
        }
        if !activate {
            return Vec::new();
        }
        self.sounds.push(GameOptionSound::Click);
        self.activate(pressed.expect("checked Some above"))
    }

    /// Touch is routed through the same C++ button down/up state machine.
    pub fn handle_touch_start(&mut self, position: GuiPoint) -> Vec<GameOptionAction> {
        self.handle_pointer_down(position)
    }

    pub fn handle_touch_move(&mut self, position: GuiPoint) -> Vec<GameOptionAction> {
        self.handle_pointer_move(position)
    }

    pub fn handle_touch_end(&mut self, position: GuiPoint) -> Vec<GameOptionAction> {
        self.handle_pointer_up(position)
    }

    pub fn handle_touch_cancel(&mut self) {
        self.pointer_left();
        self.pointer_pressed = None;
        self.pointer_down_visual = false;
        self.key_pressed = None;
    }

    pub fn pointer_left(&mut self) {
        self.pointer = None;
        self.pointer_active = false;
        self.hovered = None;
        self.refresh_pointer_down_visual(false);
    }

    pub fn cancel_interaction(&mut self) {
        self.pointer = None;
        self.pointer_active = false;
        self.hovered = None;
        self.pointer_pressed = None;
        self.pointer_down_visual = false;
        self.key_pressed = None;
    }

    /// Mirrors `CMouse::ResetActiveInput()` without discarding the retained
    /// position used to reject synthesized same-pixel motion.
    pub fn note_non_pointer_input(&mut self) {
        self.pointer_active = false;
    }

    pub fn note_pointer_wheel(&mut self) {
        self.pointer_active = self.pointer.is_some();
        self.tooltip_since = Instant::now();
    }

    pub fn note_pointer_button(&mut self) {
        self.pointer_active = self.pointer.is_some();
        self.tooltip_since = Instant::now();
    }

    pub fn handle_key_down(&mut self, key: KeyCode) -> GameOptionKeyOutcome {
        self.handle_key_down_with_tab_direction(key, false)
    }

    /// `backwards=true` is Shift+Tab. Gamepad Left uses the same C++ dialog
    /// focus traversal through [`handle_gamepad_direction`](Self::handle_gamepad_direction).
    pub fn handle_key_down_with_tab_direction(
        &mut self,
        key: KeyCode,
        backwards: bool,
    ) -> GameOptionKeyOutcome {
        self.pointer_active = false;
        match key {
            KeyCode::Tab => {
                GameOptionKeyOutcome::captured(self.advance_focus(backwards).into_iter().collect())
            }
            KeyCode::Enter | KeyCode::Space => {
                let Some(button) = self.focused else {
                    return GameOptionKeyOutcome::passed();
                };
                if !self.view(button).is_some_and(|view| view.enabled) {
                    return GameOptionKeyOutcome::passed();
                }
                if self.key_pressed.is_none() {
                    let already_down = self.button_is_down(button);
                    self.key_pressed = Some((button, key));
                    if self.pointer_pressed == Some(button) && self.hovered == Some(button) {
                        self.pointer_down_visual = true;
                    }
                    if !already_down {
                        self.sounds.push(GameOptionSound::ArrowHit);
                    }
                }
                GameOptionKeyOutcome::captured(Vec::new())
            }
            _ => GameOptionKeyOutcome::passed(),
        }
    }

    pub fn handle_key_up(&mut self, key: KeyCode) -> GameOptionKeyOutcome {
        let Some((button, pressed_key)) = self.key_pressed else {
            return GameOptionKeyOutcome::passed();
        };
        if pressed_key != key {
            return GameOptionKeyOutcome::passed();
        }
        self.key_pressed = None;
        if self.pointer_pressed == Some(button) {
            self.pointer_down_visual = false;
        }
        if self.focused != Some(button) || !self.view(button).is_some_and(|view| view.enabled) {
            return GameOptionKeyOutcome::passed();
        }
        self.sounds.push(GameOptionSound::Click);
        GameOptionKeyOutcome::captured(self.activate(button))
    }

    /// Dialog Alt-hotkeys call `OnPress` immediately and do not run the
    /// button's down/up sound path.
    pub fn handle_hotkey(&mut self, hotkey: char) -> Vec<GameOptionAction> {
        self.pointer_active = false;
        let hotkey = hotkey.to_ascii_uppercase();
        let Some(button) = self
            .context
            .buttons()
            .iter()
            .copied()
            .find(|button| button.hotkey() == hotkey)
        else {
            return Vec::new();
        };
        if !self.view(button).is_some_and(|view| view.enabled) {
            return Vec::new();
        }
        self.activate(button)
    }

    /// Left/Right are the dialog's focus bindings and are always captured,
    /// including when no boundary action is needed. Up/Down have no C++ GUI
    /// binding and pass through to lower-priority handlers.
    pub fn handle_gamepad_direction(
        &mut self,
        direction: GameOptionGamepadDirection,
    ) -> GameOptionKeyOutcome {
        self.pointer_active = false;
        match direction {
            GameOptionGamepadDirection::Left => {
                GameOptionKeyOutcome::captured(self.advance_focus(true).into_iter().collect())
            }
            GameOptionGamepadDirection::Right => {
                GameOptionKeyOutcome::captured(self.advance_focus(false).into_iter().collect())
            }
            GameOptionGamepadDirection::Up | GameOptionGamepadDirection::Down => {
                GameOptionKeyOutcome::passed()
            }
        }
    }

    pub fn handle_gamepad_low_down(&mut self) -> GameOptionKeyOutcome {
        self.handle_key_down(KeyCode::Enter)
    }

    pub fn handle_gamepad_low_up(&mut self) -> GameOptionKeyOutcome {
        self.handle_key_up(KeyCode::Enter)
    }

    /// Resolves an app-owned child dialog and recursively applies its callback
    /// (`OnPasswordSet` / `OnCommentSet`) only for an OK submission.
    pub fn resolve_input_dialog(
        &mut self,
        kind: GameOptionInputKind,
        result: GameOptionInputDialogResult,
    ) -> Vec<GameOptionAction> {
        match result {
            GameOptionInputDialogResult::Submitted(text) => self.submit_input_dialog(kind, text),
            GameOptionInputDialogResult::Cancelled => Vec::new(),
        }
    }

    /// Convenience wrapper for an accepted app-owned child input dialog.
    pub fn submit_input_dialog(
        &mut self,
        kind: GameOptionInputKind,
        text: impl Into<String>,
    ) -> Vec<GameOptionAction> {
        let text = text.into();
        match kind {
            GameOptionInputKind::Password => {
                let remember_for_next_round = (!text.is_empty()).then(|| text.clone());
                if self.context.is_lobby() {
                    return vec![GameOptionAction::PasswordChanged {
                        password: text,
                        remember_for_next_round,
                    }];
                }
                self.values.password = text.clone();
                if let Some(password) = remember_for_next_round.as_ref() {
                    self.values.last_password = password.clone();
                }
                self.sounds.push(GameOptionSound::Connect);
                vec![GameOptionAction::PasswordChanged {
                    password: text,
                    remember_for_next_round,
                }]
            }
            GameOptionInputKind::Comment => {
                if text == self.values.comment {
                    return Vec::new();
                }
                if self.context.is_lobby() {
                    return vec![GameOptionAction::CommentChanged(text)];
                }
                self.values.comment = text.clone();
                self.sounds.push(GameOptionSound::Connect);
                vec![GameOptionAction::CommentChanged(text)]
            }
        }
    }

    pub fn render(
        &self,
        surface: &mut Surface,
        resources: &GameOptionButtonResources<'_>,
        active: bool,
        gamma: Option<&GammaRamp>,
    ) -> Result<()> {
        resources.validate()?;
        for view in self.views() {
            let focused = self.focused == Some(view.button);
            let hovered = self.hovered == Some(view.button);
            if active && view.enabled && (focused || hovered) {
                draw_highlight(surface, view.rect, &resources.button_highlight, gamma);
            }
            draw_extended_icon(
                surface,
                view.rect,
                &resources.icons_extended,
                view.icon,
                gamma,
            );
            let pressed = self.pointer_down_visual && self.pointer_pressed == Some(view.button)
                || self
                    .key_pressed
                    .is_some_and(|(button, _)| button == view.button);
            if active && view.enabled && pressed {
                draw_highlight(surface, view.rect, &resources.button_highlight, gamma);
            }
        }
        Ok(())
    }

    /// Draws the delayed tooltip in the host's final screen-global overlay
    /// pass, after every ordinary child and popup has been rendered.
    pub fn render_tooltip(
        &self,
        surface: &mut Surface,
        resources: &GameOptionButtonResources<'_>,
        active: bool,
        gamma: Option<&GammaRamp>,
    ) -> Result<()> {
        self.render_tooltip_at(surface, resources, active, gamma, Instant::now())
    }

    pub fn render_tooltip_at(
        &self,
        surface: &mut Surface,
        resources: &GameOptionButtonResources<'_>,
        active: bool,
        gamma: Option<&GammaRamp>,
        now: Instant,
    ) -> Result<()> {
        resources.validate()?;
        if active {
            if let Some(tooltip) = self.tooltip_state_at(now) {
                draw_classic_tooltip(
                    surface,
                    resources.tooltip_font,
                    tooltip.pointer,
                    tooltip.text,
                    gamma,
                );
            }
        }
        Ok(())
    }

    fn internet_on(&self) -> bool {
        self.values.master_server_signup || (self.context.is_lobby() && self.values.lobby_is_league)
    }

    fn league_on(&self) -> bool {
        if self.context.is_lobby() {
            self.values.lobby_is_league
        } else {
            self.values.league_server_signup
        }
    }

    fn fair_crew_state(&self) -> (bool, bool) {
        if self.context.is_lobby() {
            (
                self.values.fair_crew,
                !self.values.countdown
                    && self.context.is_host()
                    && !self.values.lobby_fair_crew_forced,
            )
        } else {
            match self.values.selector_fair_crew_constraint {
                FairCrewConstraint::Free => (self.values.fair_crew, true),
                FairCrewConstraint::ForceFair => (true, false),
                FairCrewConstraint::ForceNormal => (false, false),
            }
        }
    }

    fn hit_test(&self, position: GuiPoint) -> Option<GameOptionButton> {
        self.layout()
            .buttons
            .into_iter()
            .find_map(|entry| contains(entry.rect, position).then_some(entry.button))
    }

    fn button_is_down(&self, button: GameOptionButton) -> bool {
        self.pointer_down_visual && self.pointer_pressed == Some(button)
            || self
                .key_pressed
                .is_some_and(|(pressed_button, _)| pressed_button == button)
    }

    fn refresh_pointer_down_visual(&mut self, allow_rise: bool) {
        let Some(pointer_button) = self.pointer_pressed else {
            self.pointer_down_visual = false;
            return;
        };
        let inside = self.hovered == Some(pointer_button)
            && self.view(pointer_button).is_some_and(|view| view.enabled);
        if self.pointer_down_visual && !inside {
            self.pointer_down_visual = false;
            self.sounds.push(GameOptionSound::ArrowHit);
            if self
                .key_pressed
                .is_some_and(|(pressed_button, _)| pressed_button == pointer_button)
            {
                self.key_pressed = None;
            }
        } else if !self.pointer_down_visual && inside && allow_rise {
            let key_already_down = self
                .key_pressed
                .is_some_and(|(pressed_button, _)| pressed_button == pointer_button);
            self.pointer_down_visual = true;
            if !key_already_down {
                self.sounds.push(GameOptionSound::ArrowHit);
            }
        }
    }

    /// `Button::SetEnabled(false)` clears C++'s held-down flag immediately
    /// and without a sound. Clear both pending input paths at the same state
    /// transition so a later re-enable cannot turn the old release into a
    /// click and pointer-up cannot synthesize a cancellation sound.
    fn cancel_disabled_presses(&mut self) {
        if self
            .key_pressed
            .is_some_and(|(button, _)| !self.view(button).is_some_and(|view| view.enabled))
        {
            self.key_pressed = None;
        }
        if self
            .pointer_pressed
            .is_some_and(|button| !self.view(button).is_some_and(|view| view.enabled))
        {
            self.pointer_pressed = None;
            self.pointer_down_visual = false;
        }
    }

    fn advance_focus(&mut self, backwards: bool) -> Option<GameOptionAction> {
        let buttons = self.context.buttons();
        if buttons.is_empty() {
            self.focused = None;
            self.key_pressed = None;
            return Some(GameOptionAction::FocusTraversalRequested { backwards });
        }
        let index = self
            .focused
            .and_then(|focused| buttons.iter().position(|button| *button == focused));
        let next = match (index, backwards) {
            (None, false) => 0,
            (None, true) => buttons.len() - 1,
            (Some(0), true) => {
                self.focused = None;
                self.key_pressed = None;
                return Some(GameOptionAction::FocusTraversalRequested { backwards: true });
            }
            (Some(index), true) => index - 1,
            (Some(index), false) if index + 1 == buttons.len() => {
                self.focused = None;
                self.key_pressed = None;
                return Some(GameOptionAction::FocusTraversalRequested { backwards: false });
            }
            (Some(index), false) => index + 1,
        };
        self.focused = Some(buttons[next]);
        self.key_pressed = None;
        None
    }

    fn activate(&mut self, button: GameOptionButton) -> Vec<GameOptionAction> {
        match button {
            GameOptionButton::Internet => self.activate_internet(),
            GameOptionButton::League => self.activate_league(),
            GameOptionButton::Password => self.activate_password(),
            GameOptionButton::Comment => vec![GameOptionAction::ShowInputDialog(
                GameOptionInputDialogRequest {
                    kind: GameOptionInputKind::Comment,
                    message: COMMENT_INPUT_MESSAGE,
                    caption: COMMENT_INPUT_CAPTION,
                    icon: GameOptionIcon::Comment,
                    max_text: COMMENT_MAX_TEXT,
                    initial_text: self.values.comment.clone(),
                    chat_layout: false,
                },
            )],
            GameOptionButton::FairCrew => {
                if self.context.is_lobby() {
                    vec![GameOptionAction::SendLobbyFairCrewControl {
                        value: if self.values.fair_crew {
                            -1
                        } else {
                            self.values.fair_crew_strength
                        },
                    }]
                } else {
                    self.values.fair_crew = !self.values.fair_crew;
                    vec![GameOptionAction::FairCrewPreferenceChanged(
                        self.values.fair_crew,
                    )]
                }
            }
            GameOptionButton::Record => {
                self.values.record = !self.values.record;
                vec![GameOptionAction::RecordPreferenceChanged(
                    self.values.record,
                )]
            }
        }
    }

    fn activate_internet(&mut self) -> Vec<GameOptionAction> {
        let enabled = !self.values.master_server_signup;
        self.values.master_server_signup = enabled;
        let mut actions = vec![GameOptionAction::InternetSignupChanged {
            enabled,
            live_lobby: self.context.is_lobby(),
        }];
        if !enabled {
            self.values.league_server_signup = false;
            actions.push(GameOptionAction::LeagueSignupChanged(false));
        }
        actions
    }

    fn activate_league(&mut self) -> Vec<GameOptionAction> {
        let enabled = !self.values.league_server_signup;
        self.values.league_server_signup = enabled;
        let mut actions = vec![GameOptionAction::LeagueSignupChanged(enabled)];
        if enabled && !self.values.master_server_signup {
            self.values.master_server_signup = true;
            actions.push(GameOptionAction::InternetSignupChanged {
                enabled: true,
                live_lobby: false,
            });
        }
        self.cancel_disabled_presses();
        actions
    }

    fn activate_password(&mut self) -> Vec<GameOptionAction> {
        if !self.values.password.is_empty() {
            if !self.context.is_lobby() {
                self.values.password.clear();
                self.sounds.push(GameOptionSound::Connect);
            }
            return vec![GameOptionAction::PasswordChanged {
                password: String::new(),
                remember_for_next_round: None,
            }];
        }
        vec![GameOptionAction::ShowInputDialog(
            GameOptionInputDialogRequest {
                kind: GameOptionInputKind::Password,
                message: PASSWORD_INPUT_MESSAGE,
                caption: PASSWORD_INPUT_CAPTION,
                icon: GameOptionIcon::LockedFrontal,
                max_text: PASSWORD_MAX_TEXT,
                initial_text: self.values.last_password.clone(),
                chat_layout: false,
            },
        )]
    }
}

fn contains(rect: IntRect, point: GuiPoint) -> bool {
    let x = point.x.floor() as i32;
    let y = point.y.floor() as i32;
    x >= rect.x && x < rect.x + rect.w && y >= rect.y && y < rect.y + rect.h
}

fn draw_extended_icon(
    surface: &mut Surface,
    rect: IntRect,
    icons: &ImageData,
    icon: GameOptionIcon,
    gamma: Option<&GammaRamp>,
) {
    let phase = u32::from(icon.phase());
    let source_x = (phase % ICON_COLUMNS) * ICON_CELL;
    let source_y = (phase / ICON_COLUMNS) * ICON_CELL;
    draw_facet_stretch(
        surface,
        icons,
        (
            source_x as f32,
            source_y as f32,
            ICON_CELL as f32,
            ICON_CELL as f32,
        ),
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
    draw_image_bilinear_additive(
        surface,
        &GuiRect::new(rect.x as f32, rect.y as f32, rect.w as f32, rect.h as f32),
        highlight,
        gamma,
    );
}

#[cfg(test)]
mod tests {
    use super::*;
    use clonk_graphics::{Color, PixelFormat};

    fn bounds() -> IntRect {
        IntRect {
            x: 10,
            y: 20,
            w: 600,
            h: 64,
        }
    }

    fn point(rect: IntRect) -> GuiPoint {
        GuiPoint::new((rect.x + rect.w / 2) as f32, (rect.y + rect.h / 2) as f32)
    }

    fn controller(context: GameOptionContext) -> GameOptionButtons {
        let mut controller = GameOptionButtons::new(
            context,
            GameOptionValues {
                fair_crew_strength: 42,
                ..GameOptionValues::default()
            },
        );
        controller.set_bounds(bounds());
        controller
    }

    #[test]
    fn exact_layout_and_order_cover_all_four_contexts() {
        let local = game_option_buttons_layout(bounds(), GameOptionContext::LocalSelector);
        assert_eq!(local.icon_size, 64);
        assert_eq!(local.icon_spacing, 9);
        assert_eq!(
            local.buttons,
            vec![
                GameOptionButtonLayout {
                    button: GameOptionButton::FairCrew,
                    rect: IntRect {
                        x: 237,
                        y: 20,
                        w: 64,
                        h: 64
                    },
                },
                GameOptionButtonLayout {
                    button: GameOptionButton::Record,
                    rect: IntRect {
                        x: 319,
                        y: 20,
                        w: 64,
                        h: 64
                    },
                },
            ]
        );

        let host = game_option_buttons_layout(bounds(), GameOptionContext::NetworkHostSelector);
        assert_eq!(
            host.buttons
                .iter()
                .map(|entry| entry.button)
                .collect::<Vec<_>>(),
            [
                GameOptionButton::Internet,
                GameOptionButton::League,
                GameOptionButton::Password,
                GameOptionButton::Comment,
                GameOptionButton::FairCrew,
                GameOptionButton::Record,
            ]
        );
        assert_eq!(
            host.buttons
                .iter()
                .map(|entry| entry.rect.x)
                .collect::<Vec<_>>(),
            [73, 155, 237, 319, 401, 483]
        );

        let lobby_host = game_option_buttons_layout(bounds(), GameOptionContext::LobbyHost);
        assert_eq!(lobby_host, host);
        let client = game_option_buttons_layout(bounds(), GameOptionContext::LobbyClient);
        assert_eq!(
            client
                .buttons
                .iter()
                .map(|entry| entry.button)
                .collect::<Vec<_>>(),
            [
                GameOptionButton::League,
                GameOptionButton::FairCrew,
                GameOptionButton::Record,
            ]
        );
        assert_eq!(
            client
                .buttons
                .iter()
                .map(|entry| entry.rect.x)
                .collect::<Vec<_>>(),
            [196, 278, 360]
        );

        let narrow = game_option_buttons_layout(
            IntRect {
                x: 10,
                y: 20,
                w: 350,
                h: 64,
            },
            GameOptionContext::NetworkHostSelector,
        );
        assert_eq!((narrow.icon_size, narrow.icon_spacing), (58, 0));
        assert_eq!(
            narrow
                .buttons
                .iter()
                .map(|entry| entry.rect.x)
                .collect::<Vec<_>>(),
            [11, 69, 127, 185, 243, 301]
        );
        assert!(narrow.buttons.iter().all(|entry| entry.rect.y == 23));
    }

    #[test]
    fn views_cover_visibility_icons_enabled_rules_and_tooltips() {
        let local = controller(GameOptionContext::LocalSelector);
        assert_eq!(
            local
                .views()
                .iter()
                .map(|view| view.button)
                .collect::<Vec<_>>(),
            [GameOptionButton::FairCrew, GameOptionButton::Record]
        );
        assert_eq!(
            local.view(GameOptionButton::FairCrew).unwrap().icon,
            GameOptionIcon::NormalCrew
        );
        assert_eq!(
            local.tooltip_for_button(GameOptionButton::FairCrew),
            Some(NORMAL_CREW_TOOLTIP)
        );

        let network = controller(GameOptionContext::NetworkHostSelector);
        assert!(network.views().iter().all(|view| view.enabled));
        assert_eq!(
            network.view(GameOptionButton::Internet).unwrap().tooltip,
            INTERNET_TOOLTIP
        );
        assert_eq!(
            network.view(GameOptionButton::Password).unwrap().icon,
            GameOptionIcon::Unlocked
        );
        assert_eq!(
            network.view(GameOptionButton::Comment).unwrap().icon,
            GameOptionIcon::Comment
        );

        let mut host_values = GameOptionValues {
            lobby_is_league: true,
            fair_crew: true,
            record: false,
            ..GameOptionValues::default()
        };
        let mut lobby_host =
            GameOptionButtons::new(GameOptionContext::LobbyHost, host_values.clone());
        lobby_host.set_bounds(bounds());
        assert!(!lobby_host.view(GameOptionButton::Internet).unwrap().enabled);
        assert_eq!(
            lobby_host.view(GameOptionButton::Internet).unwrap().icon,
            GameOptionIcon::InternetOn
        );
        assert!(!lobby_host.view(GameOptionButton::League).unwrap().enabled);
        assert!(!lobby_host.view(GameOptionButton::Record).unwrap().enabled);
        assert_eq!(
            lobby_host.view(GameOptionButton::Record).unwrap().icon,
            GameOptionIcon::RecordOn
        );

        host_values.lobby_is_league = false;
        let mut client = GameOptionButtons::new(GameOptionContext::LobbyClient, host_values);
        client.set_bounds(bounds());
        assert!(!client.view(GameOptionButton::League).unwrap().enabled);
        assert!(!client.view(GameOptionButton::FairCrew).unwrap().enabled);
        assert_eq!(
            client.view(GameOptionButton::FairCrew).unwrap().icon,
            GameOptionIcon::FairCrewGray
        );
        assert!(client.view(GameOptionButton::Record).unwrap().enabled);
    }

    #[test]
    fn pure_tooltip_target_covers_visible_disabled_buttons_and_rejects_hidden_space() {
        let values = GameOptionValues {
            lobby_is_league: true,
            ..GameOptionValues::default()
        };
        let mut lobby_host = GameOptionButtons::new(GameOptionContext::LobbyHost, values);
        lobby_host.set_bounds(bounds());

        let views = lobby_host.views();
        assert!(views.iter().any(|view| !view.enabled));
        for view in views {
            assert_eq!(
                lobby_host.tooltip_at(point(view.rect)),
                Some(view.tooltip),
                "visible {:?} button must retain its native tooltip",
                view.button
            );
            let expected_key = match view.button {
                GameOptionButton::Internet => "IDS_DLGTIP_STARTINTERNETGAME",
                GameOptionButton::League => "IDS_DLGTIP_STARTLEAGUEGAME",
                GameOptionButton::Password => "IDS_NET_PASSWORD_DESC",
                GameOptionButton::Comment => "IDS_DESC_COMMENTDESCRIPTIONFORTHIS",
                GameOptionButton::FairCrew => "IDS_CTL_NORMALCREW_DESC",
                GameOptionButton::Record => "IDS_DLGTIP_RECORD",
            };
            assert_eq!(
                lobby_host.tooltip_resource_key_at(point(view.rect)),
                Some(expected_key),
                "visible {:?} button must retain its language-table key",
                view.button
            );
        }

        let mut fair = controller(GameOptionContext::LocalSelector);
        fair.values.fair_crew = true;
        let fair_rect = fair
            .layout()
            .rect(GameOptionButton::FairCrew)
            .expect("local selector exposes Fair Crew");
        assert_eq!(
            fair.tooltip_resource_key_at(point(fair_rect)),
            Some("IDS_CTL_FAIRCREW_DESC")
        );

        let local = controller(GameOptionContext::LocalSelector);
        let hidden_internet =
            game_option_buttons_layout(bounds(), GameOptionContext::NetworkHostSelector)
                .rect(GameOptionButton::Internet)
                .expect("network selector exposes Internet");
        assert_eq!(local.tooltip_at(point(hidden_internet)), None);
        assert_eq!(local.tooltip_resource_key_at(point(hidden_internet)), None);
        assert_eq!(local.tooltip_at(GuiPoint::new(0.0, 0.0)), None);
    }

    #[test]
    fn local_selector_pointer_touch_and_forced_fair_crew_are_exact() {
        let mut state = controller(GameOptionContext::LocalSelector);
        let fair = state.layout().rect(GameOptionButton::FairCrew).unwrap();
        assert!(state.handle_pointer_down(point(fair)).is_empty());
        assert_eq!(state.take_sound_events(), [GameOptionSound::ArrowHit]);
        assert_eq!(
            state.handle_pointer_up(point(fair)),
            [GameOptionAction::FairCrewPreferenceChanged(true)]
        );
        assert_eq!(state.take_sound_events(), [GameOptionSound::Click]);
        assert_eq!(
            state.view(GameOptionButton::FairCrew).unwrap().icon,
            GameOptionIcon::FairCrew
        );

        let record = state.layout().rect(GameOptionButton::Record).unwrap();
        state.handle_touch_start(point(record));
        assert_eq!(
            state.handle_touch_end(point(record)),
            [GameOptionAction::RecordPreferenceChanged(true)]
        );
        assert_eq!(
            state.take_sound_events(),
            [GameOptionSound::ArrowHit, GameOptionSound::Click]
        );
        state.handle_touch_start(point(record));
        assert_eq!(state.take_sound_events(), [GameOptionSound::ArrowHit]);
        state.handle_touch_cancel();
        assert_eq!(state.take_sound_events(), [GameOptionSound::ArrowHit]);
        assert!(state.handle_touch_end(point(record)).is_empty());
        state.set_selector_fair_crew_constraint(FairCrewConstraint::ForceNormal);
        let fair_view = state.view(GameOptionButton::FairCrew).unwrap();
        assert!(!fair_view.enabled);
        assert_eq!(fair_view.icon, GameOptionIcon::NormalCrewGray);
        assert!(state.handle_hotkey('f').is_empty());
    }

    #[test]
    fn network_selector_toggles_internet_league_and_record_dependency() {
        let mut state = controller(GameOptionContext::NetworkHostSelector);
        assert_eq!(
            state.handle_hotkey('l'),
            [
                GameOptionAction::LeagueSignupChanged(true),
                GameOptionAction::InternetSignupChanged {
                    enabled: true,
                    live_lobby: false
                },
            ]
        );
        assert!(!state.view(GameOptionButton::Record).unwrap().enabled);
        assert_eq!(
            state.view(GameOptionButton::Record).unwrap().icon,
            GameOptionIcon::RecordOn
        );
        assert_eq!(
            state.handle_hotkey('i'),
            [
                GameOptionAction::InternetSignupChanged {
                    enabled: false,
                    live_lobby: false
                },
                GameOptionAction::LeagueSignupChanged(false),
            ]
        );
        assert!(state.view(GameOptionButton::Record).unwrap().enabled);
        assert_eq!(
            state.view(GameOptionButton::League).unwrap().icon,
            GameOptionIcon::LeagueOff
        );
    }

    #[test]
    fn password_child_request_submit_clear_and_cancel_contract() {
        let values = GameOptionValues {
            last_password: "remembered".into(),
            ..GameOptionValues::default()
        };
        let mut state = GameOptionButtons::new(GameOptionContext::NetworkHostSelector, values);
        state.set_bounds(bounds());
        assert_eq!(
            state.handle_hotkey('p'),
            [GameOptionAction::ShowInputDialog(
                GameOptionInputDialogRequest {
                    kind: GameOptionInputKind::Password,
                    message: PASSWORD_INPUT_MESSAGE,
                    caption: PASSWORD_INPUT_CAPTION,
                    icon: GameOptionIcon::LockedFrontal,
                    max_text: PASSWORD_MAX_TEXT,
                    initial_text: "remembered".into(),
                    chat_layout: false,
                }
            )]
        );
        assert!(state
            .resolve_input_dialog(
                GameOptionInputKind::Password,
                GameOptionInputDialogResult::Cancelled,
            )
            .is_empty());
        // Cancel leaves the callback and state untouched.
        assert!(state.values().password.is_empty());
        assert_eq!(
            state.submit_input_dialog(GameOptionInputKind::Password, "secret"),
            [GameOptionAction::PasswordChanged {
                password: "secret".into(),
                remember_for_next_round: Some("secret".into()),
            }]
        );
        assert_eq!(state.take_sound_events(), [GameOptionSound::Connect]);
        assert_eq!(
            state.view(GameOptionButton::Password).unwrap().icon,
            GameOptionIcon::Locked
        );
        assert_eq!(
            state.handle_hotkey('P'),
            [GameOptionAction::PasswordChanged {
                password: String::new(),
                remember_for_next_round: None,
            }]
        );
        assert_eq!(state.take_sound_events(), [GameOptionSound::Connect]);
        assert_eq!(state.values().last_password, "secret");
    }

    #[test]
    fn comment_child_request_submit_unchanged_and_changed_contract() {
        let values = GameOptionValues {
            comment: "old".into(),
            ..GameOptionValues::default()
        };
        let mut state = GameOptionButtons::new(GameOptionContext::NetworkHostSelector, values);
        state.set_bounds(bounds());
        assert_eq!(
            state.handle_hotkey('m'),
            [GameOptionAction::ShowInputDialog(
                GameOptionInputDialogRequest {
                    kind: GameOptionInputKind::Comment,
                    message: COMMENT_INPUT_MESSAGE,
                    caption: COMMENT_INPUT_CAPTION,
                    icon: GameOptionIcon::Comment,
                    max_text: COMMENT_MAX_TEXT,
                    initial_text: "old".into(),
                    chat_layout: false,
                }
            )]
        );
        assert!(state
            .submit_input_dialog(GameOptionInputKind::Comment, "old")
            .is_empty());
        assert!(state.take_sound_events().is_empty());
        assert_eq!(
            state.submit_input_dialog(GameOptionInputKind::Comment, "new"),
            [GameOptionAction::CommentChanged("new".into())]
        );
        assert_eq!(state.take_sound_events(), [GameOptionSound::Connect]);
        assert_eq!(COMMENT_CHANGED_LOG, "Network game comment adjusted.");
    }

    #[test]
    fn lobby_password_and_comment_wait_for_the_application_to_commit() {
        let values = GameOptionValues {
            password: "old password".into(),
            last_password: "remembered password".into(),
            comment: "old comment".into(),
            ..GameOptionValues::default()
        };
        let mut state = GameOptionButtons::new(GameOptionContext::LobbyHost, values);
        state.set_bounds(bounds());

        assert_eq!(
            state.handle_hotkey('P'),
            [GameOptionAction::PasswordChanged {
                password: String::new(),
                remember_for_next_round: None,
            }]
        );
        assert_eq!(state.values().password, "old password");
        assert!(state.take_sound_events().is_empty());
        state.apply_lobby_password_result(String::new(), None);
        assert!(state.values().password.is_empty());
        assert_eq!(state.values().last_password, "remembered password");
        assert_eq!(state.take_sound_events(), [GameOptionSound::Connect]);

        assert_eq!(
            state.submit_input_dialog(GameOptionInputKind::Password, "new password"),
            [GameOptionAction::PasswordChanged {
                password: "new password".into(),
                remember_for_next_round: Some("new password".into()),
            }]
        );
        assert!(state.values().password.is_empty());
        assert_eq!(state.values().last_password, "remembered password");
        assert!(state.take_sound_events().is_empty());
        state.apply_lobby_password_result("new password", Some("new password".to_string()));
        assert_eq!(state.values().password, "new password");
        assert_eq!(state.values().last_password, "new password");
        assert_eq!(state.take_sound_events(), [GameOptionSound::Connect]);

        assert_eq!(
            state.submit_input_dialog(GameOptionInputKind::Comment, "new comment"),
            [GameOptionAction::CommentChanged("new comment".into())]
        );
        assert_eq!(state.values().comment, "old comment");
        assert!(state.take_sound_events().is_empty());
        state.apply_lobby_comment_result("new comment");
        assert_eq!(state.values().comment, "new comment");
        assert_eq!(state.take_sound_events(), [GameOptionSound::Connect]);
    }

    #[test]
    fn lobby_host_emits_live_signup_and_synchronous_fair_crew_control() {
        let values = GameOptionValues {
            fair_crew_strength: 75,
            ..GameOptionValues::default()
        };
        let mut state = GameOptionButtons::new(GameOptionContext::LobbyHost, values);
        state.set_bounds(bounds());
        assert_eq!(
            state.handle_hotkey('i'),
            [GameOptionAction::InternetSignupChanged {
                enabled: true,
                live_lobby: true
            }]
        );
        state.apply_lobby_internet_result(false);
        assert_eq!(
            state.view(GameOptionButton::Internet).unwrap().icon,
            GameOptionIcon::InternetOff
        );
        assert_eq!(
            state.handle_hotkey('f'),
            [GameOptionAction::SendLobbyFairCrewControl { value: 75 }]
        );
        // The control response, not the click, updates the displayed state.
        assert_eq!(
            state.view(GameOptionButton::FairCrew).unwrap().icon,
            GameOptionIcon::NormalCrew
        );
        state.set_lobby_fair_crew(true, false);
        assert_eq!(
            state.handle_hotkey('f'),
            [GameOptionAction::SendLobbyFairCrewControl { value: -1 }]
        );
        state.set_countdown(true);
        assert!(!state.view(GameOptionButton::FairCrew).unwrap().enabled);
        assert_eq!(
            state.view(GameOptionButton::FairCrew).unwrap().icon,
            GameOptionIcon::FairCrewGray
        );
    }

    #[test]
    fn lobby_client_can_only_toggle_local_record_when_not_league() {
        let mut state = controller(GameOptionContext::LobbyClient);
        assert!(state.handle_hotkey('l').is_empty());
        assert!(state.handle_hotkey('f').is_empty());
        assert_eq!(
            state.handle_hotkey('r'),
            [GameOptionAction::RecordPreferenceChanged(true)]
        );
        state.set_lobby_league(true);
        assert!(!state.view(GameOptionButton::Record).unwrap().enabled);
        assert!(state.handle_hotkey('r').is_empty());
    }

    #[test]
    fn keyboard_gamepad_and_pointer_drag_follow_button_down_up_rules() {
        let mut state = controller(GameOptionContext::NetworkHostSelector);
        assert_eq!(
            state.handle_key_down(KeyCode::Tab),
            GameOptionKeyOutcome {
                captured: true,
                actions: Vec::new(),
            }
        );
        assert_eq!(state.focused_button(), Some(GameOptionButton::Internet));
        assert_eq!(
            state.handle_key_down_with_tab_direction(KeyCode::Tab, true),
            GameOptionKeyOutcome {
                captured: true,
                actions: vec![GameOptionAction::FocusTraversalRequested { backwards: true }],
            }
        );
        assert_eq!(state.focused_button(), None);
        assert_eq!(
            state.handle_key_down_with_tab_direction(KeyCode::Tab, true),
            GameOptionKeyOutcome {
                captured: true,
                actions: Vec::new(),
            }
        );
        assert_eq!(state.focused_button(), Some(GameOptionButton::Record));
        assert_eq!(
            state.handle_gamepad_direction(GameOptionGamepadDirection::Right),
            GameOptionKeyOutcome {
                captured: true,
                actions: vec![GameOptionAction::FocusTraversalRequested { backwards: false }],
            }
        );
        assert_eq!(state.focused_button(), None);
        assert_eq!(
            state.handle_gamepad_direction(GameOptionGamepadDirection::Right),
            GameOptionKeyOutcome {
                captured: true,
                actions: Vec::new(),
            }
        );
        assert_eq!(state.focused_button(), Some(GameOptionButton::Internet));
        assert_eq!(
            state.handle_gamepad_direction(GameOptionGamepadDirection::Right),
            GameOptionKeyOutcome {
                captured: true,
                actions: Vec::new(),
            }
        );
        assert_eq!(state.focused_button(), Some(GameOptionButton::League));
        assert_eq!(
            state.handle_gamepad_low_down(),
            GameOptionKeyOutcome {
                captured: true,
                actions: Vec::new(),
            }
        );
        // An unrelated key-up is not delivered to C++'s Enter binding and
        // must not cancel the held activation.
        assert_eq!(
            state.handle_key_up(KeyCode::Space),
            GameOptionKeyOutcome {
                captured: false,
                actions: Vec::new(),
            }
        );
        assert_eq!(state.take_sound_events(), [GameOptionSound::ArrowHit]);
        assert_eq!(
            state.handle_gamepad_low_up(),
            GameOptionKeyOutcome {
                captured: true,
                actions: vec![
                    GameOptionAction::LeagueSignupChanged(true),
                    GameOptionAction::InternetSignupChanged {
                        enabled: true,
                        live_lobby: false
                    }
                ],
            }
        );
        assert_eq!(state.take_sound_events(), [GameOptionSound::Click]);

        // Turn league back off so Internet is enabled, then verify drag-out /
        // re-entry sounds and activation.
        state.handle_hotkey('l');
        let internet = state.layout().rect(GameOptionButton::Internet).unwrap();
        state.handle_pointer_down(point(internet));
        state.take_sound_events();
        state.handle_pointer_move(GuiPoint::new(0.0, 0.0));
        assert_eq!(state.take_sound_events(), [GameOptionSound::ArrowHit]);
        state.handle_pointer_move(point(internet));
        assert_eq!(state.take_sound_events(), [GameOptionSound::ArrowHit]);
        assert_eq!(
            state.handle_pointer_up(point(internet)),
            [
                GameOptionAction::InternetSignupChanged {
                    enabled: false,
                    live_lobby: false
                },
                GameOptionAction::LeagueSignupChanged(false)
            ]
        );
        assert_eq!(state.take_sound_events(), [GameOptionSound::Click]);
    }

    #[test]
    fn gamepad_direction_reports_captured_left_right_and_unhandled_up_down() {
        let mut state = controller(GameOptionContext::LocalSelector);
        for direction in [
            GameOptionGamepadDirection::Right,
            GameOptionGamepadDirection::Left,
        ] {
            assert!(state.handle_gamepad_direction(direction).captured);
        }
        for direction in [
            GameOptionGamepadDirection::Up,
            GameOptionGamepadDirection::Down,
        ] {
            assert_eq!(
                state.handle_gamepad_direction(direction),
                GameOptionKeyOutcome {
                    captured: false,
                    actions: Vec::new(),
                }
            );
        }
    }

    #[test]
    fn disabling_a_held_button_silently_cancels_key_and_pointer_release() {
        let mut key = controller(GameOptionContext::LocalSelector);
        key.set_focused_button(Some(GameOptionButton::FairCrew));
        assert!(key.handle_key_down(KeyCode::Space).captured);
        assert_eq!(key.take_sound_events(), [GameOptionSound::ArrowHit]);
        key.set_selector_fair_crew_constraint(FairCrewConstraint::ForceNormal);
        assert!(key.take_sound_events().is_empty());
        key.set_selector_fair_crew_constraint(FairCrewConstraint::Free);
        assert_eq!(
            key.handle_key_up(KeyCode::Space),
            GameOptionKeyOutcome::passed()
        );
        assert!(!key.values().fair_crew);
        assert!(key.take_sound_events().is_empty());

        let mut pointer = controller(GameOptionContext::LobbyHost);
        let fair = pointer.layout().rect(GameOptionButton::FairCrew).unwrap();
        pointer.handle_pointer_down(point(fair));
        assert_eq!(pointer.take_sound_events(), [GameOptionSound::ArrowHit]);
        pointer.set_countdown(true);
        assert!(pointer.take_sound_events().is_empty());
        pointer.set_countdown(false);
        assert!(pointer.handle_pointer_up(point(fair)).is_empty());
        assert!(!pointer.values().fair_crew);
        assert!(pointer.take_sound_events().is_empty());
    }

    #[test]
    fn shared_button_down_state_allows_only_first_release_and_leave_cancels_matching_key() {
        let exercise = || {
            let mut state = controller(GameOptionContext::LocalSelector);
            state.set_focused_button(Some(GameOptionButton::FairCrew));
            let fair = state.layout().rect(GameOptionButton::FairCrew).unwrap();
            assert!(state.handle_key_down(KeyCode::Enter).captured);
            assert_eq!(state.take_sound_events(), [GameOptionSound::ArrowHit]);
            state.handle_pointer_down(point(fair));
            assert!(state.take_sound_events().is_empty());
            (state, fair)
        };

        let (mut pointer_first, fair) = exercise();
        assert_eq!(
            pointer_first.handle_pointer_up(point(fair)),
            [GameOptionAction::FairCrewPreferenceChanged(true)]
        );
        assert_eq!(pointer_first.take_sound_events(), [GameOptionSound::Click]);
        assert_eq!(
            pointer_first.handle_key_up(KeyCode::Enter),
            GameOptionKeyOutcome::passed()
        );
        assert!(pointer_first.take_sound_events().is_empty());

        let (mut key_first, fair) = exercise();
        assert_eq!(
            key_first.handle_key_up(KeyCode::Enter),
            GameOptionKeyOutcome::captured(vec![GameOptionAction::FairCrewPreferenceChanged(true)])
        );
        assert_eq!(key_first.take_sound_events(), [GameOptionSound::Click]);
        assert_eq!(key_first.pointer_pressed, Some(GameOptionButton::FairCrew));
        assert!(!key_first.pointer_down_visual);
        key_first.handle_pointer_move(point(fair));
        assert!(key_first.take_sound_events().is_empty());
        assert!(key_first.handle_pointer_up(point(fair)).is_empty());
        assert!(key_first.take_sound_events().is_empty());

        let (mut leave, fair) = exercise();
        leave.pointer_left();
        assert_eq!(leave.take_sound_events(), [GameOptionSound::ArrowHit]);
        assert_eq!(
            leave.handle_key_up(KeyCode::Enter),
            GameOptionKeyOutcome::passed()
        );
        assert_eq!(leave.pointer_pressed, Some(GameOptionButton::FairCrew));
        leave.handle_pointer_move(point(fair));
        assert_eq!(leave.take_sound_events(), [GameOptionSound::ArrowHit]);
        assert_eq!(
            leave.handle_pointer_up(point(fair)),
            [GameOptionAction::FairCrewPreferenceChanged(true)]
        );
        assert_eq!(leave.take_sound_events(), [GameOptionSound::Click]);
    }

    #[test]
    fn shared_button_drag_preserves_key_outside_and_reentry_suppresses_duplicate_down_sound() {
        let setup_outside_key = || {
            let mut state = controller(GameOptionContext::LocalSelector);
            state.set_focused_button(Some(GameOptionButton::FairCrew));
            let fair = state.layout().rect(GameOptionButton::FairCrew).unwrap();
            state.handle_pointer_down(point(fair));
            assert_eq!(state.take_sound_events(), [GameOptionSound::ArrowHit]);
            state.handle_pointer_move(GuiPoint::new(0.0, 0.0));
            assert_eq!(state.take_sound_events(), [GameOptionSound::ArrowHit]);
            assert!(state.handle_key_down(KeyCode::Enter).captured);
            assert_eq!(state.take_sound_events(), [GameOptionSound::ArrowHit]);
            (state, fair)
        };

        let (mut released_outside, _fair) = setup_outside_key();
        assert!(released_outside
            .handle_pointer_up(GuiPoint::new(0.0, 0.0))
            .is_empty());
        assert!(released_outside.take_sound_events().is_empty());
        assert_eq!(
            released_outside.handle_key_up(KeyCode::Enter),
            GameOptionKeyOutcome::captured(vec![GameOptionAction::FairCrewPreferenceChanged(true)])
        );
        assert_eq!(
            released_outside.take_sound_events(),
            [GameOptionSound::Click]
        );

        let (mut reentered, fair) = setup_outside_key();
        reentered.handle_pointer_move(point(fair));
        assert!(reentered.take_sound_events().is_empty());
        assert_eq!(
            reentered.handle_pointer_up(point(fair)),
            [GameOptionAction::FairCrewPreferenceChanged(true)]
        );
        assert_eq!(reentered.take_sound_events(), [GameOptionSound::Click]);
        assert_eq!(
            reentered.handle_key_up(KeyCode::Enter),
            GameOptionKeyOutcome::passed()
        );
    }

    #[test]
    fn every_enablement_transition_cancels_its_held_button() {
        let mut forced = controller(GameOptionContext::LobbyHost);
        forced.set_focused_button(Some(GameOptionButton::FairCrew));
        assert!(forced.handle_key_down(KeyCode::Enter).captured);
        forced.take_sound_events();
        forced.set_lobby_fair_crew(false, true);
        forced.set_lobby_fair_crew(false, false);
        assert!(!forced.handle_key_up(KeyCode::Enter).captured);
        assert!(forced.take_sound_events().is_empty());

        let mut league_state = controller(GameOptionContext::LobbyClient);
        let record = league_state
            .layout()
            .rect(GameOptionButton::Record)
            .unwrap();
        league_state.handle_pointer_down(point(record));
        league_state.take_sound_events();
        league_state.set_lobby_league(true);
        league_state.set_lobby_league(false);
        assert!(league_state.handle_pointer_up(point(record)).is_empty());
        assert!(league_state.take_sound_events().is_empty());

        let mut league_click = controller(GameOptionContext::NetworkHostSelector);
        let record = league_click
            .layout()
            .rect(GameOptionButton::Record)
            .unwrap();
        league_click.handle_pointer_down(point(record));
        league_click.take_sound_events();
        league_click.handle_hotkey('l');
        league_click.handle_hotkey('l');
        assert!(league_click.handle_pointer_up(point(record)).is_empty());
        assert!(!league_click.values().record);
        assert!(league_click.take_sound_events().is_empty());
    }

    #[test]
    fn focused_disabled_record_and_fair_crew_pass_through() {
        let mut record = GameOptionButtons::new(
            GameOptionContext::NetworkHostSelector,
            GameOptionValues {
                league_server_signup: true,
                ..GameOptionValues::default()
            },
        );
        record.set_bounds(bounds());
        record.set_focused_button(Some(GameOptionButton::Record));
        assert!(!record.view(GameOptionButton::Record).unwrap().enabled);
        assert_eq!(
            record.handle_key_down(KeyCode::Enter),
            GameOptionKeyOutcome {
                captured: false,
                actions: Vec::new(),
            }
        );

        let mut fair_crew = controller(GameOptionContext::LocalSelector);
        fair_crew.set_selector_fair_crew_constraint(FairCrewConstraint::ForceNormal);
        fair_crew.set_focused_button(Some(GameOptionButton::FairCrew));
        assert!(!fair_crew.view(GameOptionButton::FairCrew).unwrap().enabled);
        assert_eq!(
            fair_crew.handle_key_down(KeyCode::Space),
            GameOptionKeyOutcome {
                captured: false,
                actions: Vec::new(),
            }
        );
        assert!(record.take_sound_events().is_empty());
        assert!(fair_crew.take_sound_events().is_empty());
    }

    #[test]
    fn enabled_key_press_and_release_are_both_captured() {
        let mut state = controller(GameOptionContext::LocalSelector);
        state.set_focused_button(Some(GameOptionButton::Record));
        assert_eq!(
            state.handle_key_down(KeyCode::Space),
            GameOptionKeyOutcome {
                captured: true,
                actions: Vec::new(),
            }
        );
        assert_eq!(state.take_sound_events(), [GameOptionSound::ArrowHit]);
        assert_eq!(
            state.handle_key_up(KeyCode::Space),
            GameOptionKeyOutcome {
                captured: true,
                actions: vec![GameOptionAction::RecordPreferenceChanged(true)],
            }
        );
        assert_eq!(state.take_sound_events(), [GameOptionSound::Click]);
    }

    fn icon_sheet() -> ImageData {
        let mut pixels = vec![0; (ICON_SHEET_WIDTH * ICON_SHEET_HEIGHT * 4) as usize];
        for phase in 0..(ICON_SHEET_WIDTH / ICON_CELL * ICON_SHEET_HEIGHT / ICON_CELL) {
            let phase_x = phase % ICON_COLUMNS;
            let phase_y = phase / ICON_COLUMNS;
            let color = [phase as u8 + 1, 0, 0, 255];
            for y in 0..ICON_CELL {
                for x in 0..ICON_CELL {
                    let index =
                        ((((phase_y * ICON_CELL + y) * ICON_SHEET_WIDTH) + phase_x * ICON_CELL + x)
                            * 4) as usize;
                    pixels[index..index + 4].copy_from_slice(&color);
                }
            }
        }
        ImageData::new(ICON_SHEET_WIDTH, ICON_SHEET_HEIGHT, pixels)
    }

    #[test]
    fn full_size_scenario_button_highlight_is_valid() {
        let icons = icon_sheet();
        let highlight = ImageData::new(30, 30, vec![0xff; 30 * 30 * 4]);
        let font = ClonkFont::new(12);

        // C4GUI::Resource::Load passes C4FCT_Full for GUIButtonHighlight, so
        // C4FacetExSurface::Load retains the complete override dimensions
        // (src/C4Gui.cpp:1093; src/C4FacetEx.cpp:137-161).
        let resources = GameOptionButtonResources::new(&icons, &highlight, &font)
            .expect("MarsClonk's 30x30 highlight is a valid full-size facet");
        resources
            .validate()
            .expect("the full-size facet remains valid at render time");
    }

    #[test]
    fn renderer_draws_every_context_phase_and_rejects_nonclassic_assets() {
        let icons = icon_sheet();
        let transparent_highlight = ImageData::new(
            HIGHLIGHT_WIDTH,
            HIGHLIGHT_HEIGHT,
            vec![0; (HIGHLIGHT_WIDTH * HIGHLIGHT_HEIGHT * 4) as usize],
        );
        let font = ClonkFont::new(12);
        let resources = GameOptionButtonResources::new(&icons, &transparent_highlight, &font)
            .expect("classic resources");
        for context in [
            GameOptionContext::LocalSelector,
            GameOptionContext::NetworkHostSelector,
            GameOptionContext::LobbyHost,
            GameOptionContext::LobbyClient,
        ] {
            let state = controller(context);
            let mut surface = Surface::new(640, 100, PixelFormat::Rgba8888);
            surface.fill(Color::transparent());
            state
                .render(&mut surface, &resources, true, None)
                .expect("render");
            for view in state.views() {
                assert_eq!(
                    surface.get_pixel(
                        (view.rect.x + view.rect.w / 2) as u32,
                        (view.rect.y + view.rect.h / 2) as u32
                    ),
                    Some(Color::new(view.icon.phase() as u8 + 1, 0, 0, 255)),
                    "wrong phase for {context:?} / {:?}",
                    view.button
                );
            }
        }
        let bad = ImageData::new(64, 64, vec![255; 64 * 64 * 4]);
        assert!(GameOptionButtonResources::new(&bad, &transparent_highlight, &font).is_err());
    }

    #[test]
    fn renderer_applies_two_additive_highlights_to_pressed_focused_icon() {
        let icons = icon_sheet();
        let highlight = ImageData::new(
            HIGHLIGHT_WIDTH,
            HIGHLIGHT_HEIGHT,
            vec![32; (HIGHLIGHT_WIDTH * HIGHLIGHT_HEIGHT * 4) as usize],
        );
        let font = ClonkFont::new(12);
        let resources = GameOptionButtonResources::new(&icons, &highlight, &font).unwrap();
        let mut state = controller(GameOptionContext::LocalSelector);
        state.set_focused_button(Some(GameOptionButton::Record));
        state.handle_key_down(KeyCode::Space);
        let record = state.layout().rect(GameOptionButton::Record).unwrap();
        let mut surface = Surface::new(640, 100, PixelFormat::Rgba8888);
        state.render(&mut surface, &resources, true, None).unwrap();
        let pixel = surface
            .get_pixel(
                (record.x + record.w / 2) as u32,
                (record.y + record.h / 2) as u32,
            )
            .unwrap();
        assert!(pixel.r > GameOptionIcon::RecordOff.phase() as u8 + 1);
    }

    #[test]
    fn tooltip_delay_and_disabled_button_tooltip_match_screen_behavior() {
        let values = GameOptionValues {
            lobby_is_league: true,
            ..GameOptionValues::default()
        };
        let mut state = GameOptionButtons::new(GameOptionContext::LobbyHost, values);
        state.set_bounds(bounds());
        let internet = state.layout().rect(GameOptionButton::Internet).unwrap();
        state.handle_pointer_move(point(internet));
        let before = state.tooltip_since + Duration::from_millis(499);
        let after = state.tooltip_since + Duration::from_millis(500);
        assert_eq!(state.hovered_tooltip_at(before), None);
        assert_eq!(state.hovered_tooltip_at(after), Some(INTERNET_TOOLTIP));
        assert_eq!(
            state.tooltip_state_at(after),
            Some(GameOptionTooltip {
                pointer: point(internet),
                text: INTERNET_TOOLTIP,
            })
        );
        assert!(!state.view(GameOptionButton::Internet).unwrap().enabled);

        let icons = icon_sheet();
        let highlight = ImageData::new(
            HIGHLIGHT_WIDTH,
            HIGHLIGHT_HEIGHT,
            vec![0; (HIGHLIGHT_WIDTH * HIGHLIGHT_HEIGHT * 4) as usize],
        );
        let font = ClonkFont::new(12);
        let resources = GameOptionButtonResources::new(&icons, &highlight, &font).unwrap();
        let mut without_tooltip = Surface::new(640, 100, PixelFormat::Rgba8888);
        let mut with_tooltip = Surface::new(640, 100, PixelFormat::Rgba8888);
        state
            .render(&mut without_tooltip, &resources, true, None)
            .unwrap();
        state
            .render(&mut with_tooltip, &resources, true, None)
            .unwrap();
        assert_eq!(without_tooltip.pixels(), with_tooltip.pixels());
        state
            .render_tooltip_at(&mut with_tooltip, &resources, true, None, after)
            .unwrap();
        assert_ne!(without_tooltip.pixels(), with_tooltip.pixels());

        state.note_non_pointer_input();
        assert_eq!(state.hovered_tooltip_at(after), None);
        state.handle_pointer_move(point(internet));
        assert_eq!(
            state.hovered_tooltip_at(Instant::now() + Duration::from_secs(1)),
            None,
            "same-pixel motion does not restore CMouse ownership"
        );
        state.note_pointer_wheel();
        assert_eq!(
            state.hovered_tooltip_at(Instant::now() + Duration::from_secs(1)),
            Some(INTERNET_TOOLTIP),
            "an unconsumed wheel is a fresh pointer event"
        );
        state.note_pointer_button();
        let button_time = state.tooltip_since;
        assert_eq!(
            state.hovered_tooltip_at(button_time + TOOLTIP_DELAY - Duration::from_millis(1)),
            None,
            "every mouse-button edge resets the tooltip clock"
        );
        assert_eq!(
            state.hovered_tooltip_at(button_time + TOOLTIP_DELAY),
            Some(INTERNET_TOOLTIP)
        );
    }

    #[test]
    fn resource_constructor_blackens_hidden_png_rgb_before_filtering() {
        let mut icon_pixels = icon_sheet().pixels().to_vec();
        icon_pixels[..4].copy_from_slice(&[255, 127, 63, 0]);
        let icons = ImageData::new(ICON_SHEET_WIDTH, ICON_SHEET_HEIGHT, icon_pixels);
        let highlight = ImageData::new(
            HIGHLIGHT_WIDTH,
            HIGHLIGHT_HEIGHT,
            vec![255; (HIGHLIGHT_WIDTH * HIGHLIGHT_HEIGHT * 4) as usize],
        );
        let mut highlight_pixels = highlight.pixels().to_vec();
        highlight_pixels[..4].copy_from_slice(&[255, 127, 63, 0]);
        let highlight = ImageData::new(HIGHLIGHT_WIDTH, HIGHLIGHT_HEIGHT, highlight_pixels);
        let font = ClonkFont::new(12);
        let resources = GameOptionButtonResources::new(&icons, &highlight, &font).unwrap();
        assert_eq!(&resources.icons_extended.pixels()[..4], &[0, 0, 0, 0]);
        assert_eq!(&resources.button_highlight.pixels()[..4], &[0, 0, 0, 0]);
    }
}
