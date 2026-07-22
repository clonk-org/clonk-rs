//! In-game player menu: a faithful port of `C4MainMenu`
//! (src/C4MainMenu.cpp) on the classic `C4Menu` context-style chassis
//! (src/C4Menu.cpp). Entry lists, order, captions (LanguageUS.txt), icons
//! (GfxR facets) and command semantics mirror the C++ oracle; the renderer
//! reproduces the classic menu furniture (semi-transparent black dialog with
//! a 3D frame, wooden caption bar, red selection box, symbol+text lines and
//! the bottom command-key bar).
//!
//! C++ opens this menu per player via `COM_PlayerMenu`
//! (C4Game.cpp:3593-3601 -> C4Player::ActivateMenuMain, C4Player.cpp:2327).
//! Bare Escape and the Abort command leave this menu layer and open the
//! standalone `C4AbortGameDialog` port owned by clonk-app.

use std::cell::{Cell, RefCell};
use std::collections::HashMap;

use clonk_engine::{CommandKind, ControlCommand};
use clonk_frontend::{hud::HudFont, message_dialog::break_hud_message, GuiPoint, HudGraphics};
use clonk_graphics::clonk_font::TextAlign;
use clonk_graphics::{Color, GammaRamp, Rect, Surface};
use clonk_gui::ImageData;

/// `C4MN_SymbolSize` (C4Menu.h:34): min item height and extra-bar height.
const MN_SYMBOL_SIZE: i32 = 16;
/// `C4MN_FrameWidth` (C4Menu.h:35).
const MN_FRAME_WIDTH: i32 = 2;
/// `C4SymbolSize` (C4Constants.h:75): menu alignment margin.
const SYMBOL_SIZE: i32 = 35;
/// `C4MN_InfoCaption_Delay` (C4Menu.cpp:37): frames before the tooltip shows.
const INFO_CAPTION_DELAY: u32 = 90;
/// `C4GUI_MinWoodBarHgt` (C4Gui.h:161) — `WoodenLabel::GetDefaultHeight`.
const MIN_WOOD_BAR_HGT: i32 = 23;

/// `C4GUI_StandardBGColor` 0x5f000000 (C4Gui.h:80); engine alpha is inverted
/// (0x00 = opaque), so the dialog background is black at (255-0x5f)/255.
const STANDARD_BG_ALPHA: u8 = 255 - 0x5f;
/// `C4GUI_BorderColor1..3` at `C4GUI_BorderAlpha` 0xaf (C4Gui.h:97-100).
const BORDER_ALPHA: u8 = 255 - 0xaf;
const BORDER_COLOR_1: Color = Color::new(0x77, 0x22, 0x00, BORDER_ALPHA);
const BORDER_COLOR_2: Color = Color::new(0x33, 0x11, 0x00, BORDER_ALPHA);
const BORDER_COLOR_3: Color = Color::new(0xaa, 0x44, 0x00, BORDER_ALPHA);
/// Selection mark: `DrawBox(..., CRed)` (C4Menu.cpp:154) — palette entry 10
/// of Graphics.c4g/C4.PAL is #c80000.
const SELECTION_COLOR: Color = Color::opaque(0xc8, 0x00, 0x00);
/// Extra-bar divider: `DrawFrame(..., 80)` (C4Menu.cpp:934) — palette entry
/// 80 is #440000.
const EXTRA_FRAME_COLOR: Color = Color::opaque(0x44, 0x00, 0x00);
/// `CStdDDraw::DEFAULT_MESSAGE_COLOR` (StdDDraw2.h:361).
const MESSAGE_COLOR: Color = Color::opaque(0xff, 0xff, 0xff);
/// `C4GUI_CaptionFontClr` (C4Gui.h:53).
const CAPTION_COLOR: Color = Color::opaque(0xff, 0xff, 0xff);
/// Tooltip colors (C4Gui.h:86-88): bg 0x00F1EA78 (inverted alpha: opaque),
/// frame 0x7f000000, text 0xFF483222 (font path: plain RGB).
const TOOLTIP_BG_COLOR: Color = Color::opaque(0xf1, 0xea, 0x78);
const TOOLTIP_FRAME_ALPHA: u8 = 255 - 0x7f;
const TOOLTIP_TEXT_COLOR: Color = Color::opaque(0x48, 0x32, 0x22);
/// `C4GUI_MaxToolTipWdt` (C4Gui.h): maximum tooltip line width.
const MAX_TOOLTIP_WDT: i32 = 500;

/// `C4GUI::Icons` indices into GUIIcons.png (C4Gui.h:670-731), 40x40 cells
/// (C4Gui.cpp:1094-1095), 6 per row (C4GuiLabels.cpp:441-450).
pub const ICO_HOST: u8 = 4;
pub const ICO_CLIENT: u8 = 5;
pub const ICO_OBSERVER_CLIENT: u8 = 8;
pub const ICO_TEAM: u8 = 19;
pub const ICO_GAME_RUNNING: u8 = 30;
pub const ICO_EXIT: u8 = 33;
pub const ICO_CLOSE: u8 = 34;
pub const ICO_SURRENDER: u8 = 45;
pub const ICO_STAR: u8 = 48;
pub const ICO_DISCONNECT: u8 = 49;
pub const ICO_VIEW: u8 = 50;

/// GfxR facet references for menu symbols (C4GraphicsResource.cpp:199-227).
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum MenuSymbol {
    /// `GfxR->fctMenu.GetPhase(x)`: Menu.png row 0, 35x35 cells
    /// (C4GraphicsResource.cpp:219).
    Menu(u8),
    /// `GfxR->fctMenu.GetPhase(slot - 1, free ? 2 : 1)`: savegame slot icons
    /// (C4MainMenu.cpp:493).
    SaveSlot { slot: u8, free: bool },
    /// `GfxR->fctOptions.GetPhase(x)`: Options.png 35x35 cells
    /// (C4GraphicsResource.cpp:224).
    Options(u8),
    /// `GfxR->fctOKCancel` phase (x, y): Control.png (128,100) + 32x32 grid
    /// (C4GraphicsResource.cpp:204).
    OkCancel(u8, u8),
    /// `C4GUI::Icon::GetIconFacet(index)`: GUIIcons.png (C4GuiLabels.cpp:441).
    GuiIcon(u8),
    /// `GfxR->fctPlayerClr` (Player.png) with the default blue overlay
    /// (C4MainMenu.cpp:69-70, 686).
    PlayerColor,
    /// `C4Player::DrawHostility`: the opponent's owner-colored crew image,
    /// overlaid with Menu.png phase 7 while the menu owner attacks them
    /// (C4Player.cpp:1149-1165).
    Hostility { opponent: i32, hostile: bool },
    /// A definition picture (`pDef->Draw(fctSymbol)`, C4MainMenu.cpp:367),
    /// optionally overlaid with the Captain facet for a fulfilled goal
    /// (C4MainMenu.cpp:368-372).
    Definition { id: String, fulfilled: bool },
    /// One `C4MN_TeamSelection` / `C4MN_TeamSwitch` row. Rendering resolves
    /// `IconSpec` first, then uses the team's owner-colored Crew facet when
    /// occupied, and finally the generic team GUI icon (C4MainMenu.cpp:200-212).
    Team {
        id: i32,
        icon_spec: Option<String>,
        color: u32,
        has_participants: bool,
    },
}

/// Display submenu toggles ("Display:*" commands, C4MainMenu.cpp:855-884).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DisplayToggle {
    UpperBoard,
    Fps,
    PlayerNames,
    ClonkNames,
    Portraits,
    ShowCommands,
    ShowCommandKeys,
    Clock,
    WhiteChat,
}

/// Target selected by `C4MN_Observer`'s `Observe:*` commands
/// (C4MainMenu.cpp:235-273,920-945).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ObserverTarget {
    Free,
    Player(i32),
}

/// One visible runtime player row in the observer target menu.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ObserverPlayerEntry {
    pub id: i32,
    pub name: String,
}

/// Menu commands, mirroring the strings dispatched by
/// `C4MainMenu::MenuCommand` (C4MainMenu.cpp:734-948).
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum MenuAction {
    /// "ActivateMenu:Main" (C4MainMenu.cpp:742).
    ActivateMain,
    /// "ActivateMenu:Goals" (C4MainMenu.cpp:745).
    ActivateGoals,
    /// "ActivateMenu:Rules" (C4MainMenu.cpp:750).
    ActivateRules,
    /// "ActivateMenu:Hostility" (C4MainMenu.cpp:743).
    ActivateHostility,
    /// "SetHostility:<player>" queues `CID_ToggleHostility`
    /// (C4MainMenu.cpp:773-783).
    ToggleHostility(i32),
    /// "ActivateMenu:NewPlayer" (C4MainMenu.cpp:744).
    ActivateNewPlayer,
    /// "ActivateMenu:Options" (C4MainMenu.cpp:753).
    ActivateOptions,
    /// "ActivateMenu:Display" (C4MainMenu.cpp:754).
    ActivateDisplay,
    /// "ActivateMenu:Save:Game" (C4MainMenu.cpp:755).
    ActivateSavegame,
    /// "ActivateMenu:Surrender" (C4MainMenu.cpp:757).
    ActivateSurrender,
    /// "ActivateMenu:Observer" (C4MainMenu.cpp:758).
    ActivateObserver,
    /// "Observe:Free" / "Observe:<player>" (C4MainMenu.cpp:920-945).
    Observe(ObserverTarget),
    /// "ActivateMenu:TeamSel" (C4MainMenu.cpp:756).
    ActivateTeamSelection,
    /// "ActivateMenu:Client" (C4MainMenu.cpp:752).
    ActivateClientDisconnect,
    /// "ActivateMenu:Host" (C4MainMenu.cpp:751).
    ActivateHostDisconnect,
    /// "Host:Kick:<id>" (C4MainMenu.cpp:805-819).
    KickClient(i32),
    /// "Abort" -> `FullScreen.ShowAbortDlg()` (C4MainMenu.cpp:785-789).
    Abort,
    /// "Surrender" -> queues `CID_SurrenderPlayer` (C4MainMenu.cpp:791-795).
    Surrender,
    /// "Part": client leaves the network game (C4MainMenu.cpp:821-832).
    Part,
    /// "Save:Game:<file>:<title>" -> `Game.QuickSave` then reopen
    /// (C4MainMenu.cpp:797-804). Slot is 1-based.
    SaveSlot(u8),
    /// "Options:Sound" (C4MainMenu.cpp:842-845).
    ToggleSound,
    /// "Options:Music" (C4MainMenu.cpp:837-840).
    ToggleMusic,
    /// "Options:Mouse" (C4MainMenu.cpp:847-849).
    ToggleMouseControl,
    /// "Display:*" (C4MainMenu.cpp:855-884).
    Display(DisplayToggle),
    /// "Player:Goal:<id>" (C4MainMenu.cpp:886-897).
    GoalInfo(String),
    /// "Player:Rule:<id>" (C4MainMenu.cpp:886-897).
    RuleInfo(String),
    /// "JoinPlayer:<file>" (C4MainMenu.cpp:761-772).
    JoinPlayer(String),
    /// "TeamSel:<id>" (C4MainMenu.cpp:899-908).
    SelectTeam(i32),
    /// "TeamSwitch:<id>" queues `CID_SetPlayerTeam`
    /// (C4MainMenu.cpp:909-918).
    SwitchTeam(i32),
    /// Items with an empty command (the "No" buttons, C4MainMenu.cpp:533).
    NoOp,
}

/// `C4Menu::Identification`-style page tag for the active menu.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MenuPage {
    Main,
    Hostility,
    Observer,
    TeamSelection,
    Goals,
    Rules,
    NewPlayer,
    Savegame,
    Options,
    Display,
    Surrender,
    ClientDisconnect,
    HostDisconnect,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum MenuStyle {
    Normal,
    Context,
}

/// One `C4MenuItem` (C4Menu.h:76-132) as used by C4MainMenu: caption,
/// symbol, command and info caption; all main-menu items are selectable and
/// use `C4MN_Item_NoCount`.
#[derive(Clone, Debug)]
pub struct MenuItem {
    pub caption: String,
    pub symbol: MenuSymbol,
    pub action: MenuAction,
    pub info_caption: Option<String>,
}

impl MenuItem {
    fn new(
        caption: impl Into<String>,
        symbol: MenuSymbol,
        action: MenuAction,
        info_caption: Option<&str>,
    ) -> Self {
        Self {
            caption: caption.into(),
            symbol,
            action,
            info_caption: info_caption.map(str::to_string),
        }
    }
}

/// Result of a menu control com (C4Menu::Control, C4Menu.cpp:433-484).
#[derive(Clone, Debug)]
pub enum MenuOutcome {
    /// Enter on the selected item (C4Menu::Enter, C4Menu.cpp:498-521): a
    /// non-permanent menu closes before its command runs.
    Action { action: MenuAction, close_menu: bool },
    /// COM_MenuClose (C4Menu::TryClose, C4Menu.cpp:317-334): the menu closes
    /// and the close command (if any) runs after.
    Closed { close_action: Option<MenuAction> },
}

/// Conditions from `C4MainMenu::ActivateMain` (C4MainMenu.cpp:643-715).
#[derive(Clone, Debug)]
pub struct MainMenuConditions {
    /// `Game.Players.Get(iPlayer)` non-null.
    pub has_player: bool,
    /// `Game.Players.GetCount()`.
    pub player_count: usize,
    /// `Game.Parameters.MaxPlayers`.
    pub max_players: usize,
    /// `Game.Parameters.isLeague()`.
    pub is_league: bool,
    /// `Game.Network.isEnabled()`.
    pub network_enabled: bool,
    /// `Game.Network.isHost()`.
    pub network_host: bool,
    /// `Game.Clients.getClient(nullptr)` non-null (any remote client).
    pub network_has_clients: bool,
    /// `Application.isFullScreen`.
    pub is_fullscreen: bool,
    /// `Game.Teams.IsTeamSwitchAllowed()`.
    pub team_switch_allowed: bool,
}

impl Default for MainMenuConditions {
    /// A local fullscreen single-player round.
    fn default() -> Self {
        Self {
            has_player: true,
            player_count: 1,
            max_players: 12,
            is_league: false,
            network_enabled: false,
            network_host: false,
            network_has_clients: false,
            is_fullscreen: true,
            team_switch_allowed: false,
        }
    }
}

/// Option toggles shown by `ActivateOptions` (C4MainMenu.cpp:553-580).
#[derive(Clone, Copy, Debug)]
pub struct OptionFlags {
    /// `Config.Sound.RXSound`.
    pub sound: bool,
    /// `Config.Sound.RXMusic`.
    pub music: bool,
    /// Whether the mouse entry is shown (`pPlr && !DisableMouse` and mouse
    /// control available, C4MainMenu.cpp:564-571).
    pub mouse_shown: bool,
    /// `pPlr->MouseControl`.
    pub mouse: bool,
}

/// `Config.Graphics.UpperBoard` (C4Config.cpp:455-462; C4UpperBoard modes).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum UpperBoardMode {
    Hide,
    Full,
    Small,
    Mini,
}

impl UpperBoardMode {
    /// `++Config.Graphics.UpperBoard` with wrap (C4MainMenu.cpp:858-864).
    pub fn next(self) -> Self {
        match self {
            UpperBoardMode::Hide => UpperBoardMode::Full,
            UpperBoardMode::Full => UpperBoardMode::Small,
            UpperBoardMode::Small => UpperBoardMode::Mini,
            UpperBoardMode::Mini => UpperBoardMode::Hide,
        }
    }

    /// `IDS_MNU_UPPERBOARD_*` (C4MainMenu.cpp:606-623).
    fn label(self) -> &'static str {
        match self {
            UpperBoardMode::Hide => "Off",
            UpperBoardMode::Full => "Normal",
            UpperBoardMode::Small => "Small",
            UpperBoardMode::Mini => "Minimal at bottom",
        }
    }
}

/// Display toggles and renderer flags, with defaults from
/// C4Config.cpp:381,446-465. The interactive subset is shown by
/// `ActivateDisplay` (C4MainMenu.cpp:582-641).
#[derive(Clone, Copy, Debug)]
pub struct DisplayFlags {
    pub player_names: bool,
    pub clonk_names: bool,
    pub portraits: bool,
    pub show_commands: bool,
    pub show_command_keys: bool,
    pub show_player_hud_always: bool,
    pub splitscreen_dividers: bool,
    pub fire_particles: bool,
    pub upper_board: UpperBoardMode,
    pub fps: bool,
    pub clock: bool,
    pub white_chat: bool,
    pub is_fullscreen: bool,
}

impl Default for DisplayFlags {
    fn default() -> Self {
        Self {
            player_names: true,
            clonk_names: true,
            portraits: true,
            show_commands: true,
            show_command_keys: true,
            show_player_hud_always: true,
            splitscreen_dividers: true,
            fire_particles: true,
            upper_board: UpperBoardMode::Full,
            fps: false,
            clock: false,
            white_chat: false,
            is_fullscreen: true,
        }
    }
}

impl DisplayFlags {
    /// Applies a toggle like `C4MainMenu::MenuCommand`'s "Display:" branch
    /// (C4MainMenu.cpp:855-884).
    pub fn toggle(&mut self, toggle: DisplayToggle) {
        match toggle {
            DisplayToggle::UpperBoard => self.upper_board = self.upper_board.next(),
            DisplayToggle::Fps => self.fps = !self.fps,
            DisplayToggle::PlayerNames => self.player_names = !self.player_names,
            DisplayToggle::ClonkNames => self.clonk_names = !self.clonk_names,
            DisplayToggle::Portraits => self.portraits = !self.portraits,
            DisplayToggle::ShowCommands => self.show_commands = !self.show_commands,
            DisplayToggle::ShowCommandKeys => self.show_command_keys = !self.show_command_keys,
            DisplayToggle::Clock => self.clock = !self.clock,
            DisplayToggle::WhiteChat => self.white_chat = !self.white_chat,
        }
    }
}

/// One savegame slot's state for `ActivateSavegame` (C4MainMenu.cpp:483-494).
#[derive(Clone, Copy, Debug)]
pub struct SaveSlotState {
    pub free: bool,
}

/// A goal or rule entry for `ActivateGoals`/`ActivateRules`
/// (C4MainMenu.cpp:332-405): definition id, name, description and fulfillment
/// state (the latter is used only for goals).
#[derive(Clone, Debug)]
pub struct GoalRuleEntry {
    pub definition_id: String,
    pub name: String,
    pub description: Option<String>,
    pub fulfilled: bool,
}

/// A player file entry for `ActivateNewPlayer` (C4MainMenu.cpp:59-122).
#[derive(Clone, Debug)]
pub struct NewPlayerEntry {
    pub file: String,
    pub name: String,
}

/// One native-order hostility row (`C4MainMenu::DoRefillInternal`,
/// C4MainMenu.cpp:138-168). Hostility is directional: both declarations are
/// needed to reproduce the row caption and its tooltip.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct HostilityEntry {
    pub opponent: i32,
    pub name: String,
    pub hostile: bool,
    pub opponent_hostile: bool,
}

/// One ordered `C4TeamList` row as displayed by the initial team-selection
/// menu (`C4MainMenu.cpp:175-232`). `caption` is the already composed
/// `C4Team::GetNameWithParticipants()` text (or "New Team").
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TeamSelectionEntry {
    pub id: i32,
    pub caption: String,
    pub icon_spec: Option<String>,
    pub color: u32,
    pub has_participants: bool,
}

impl TeamSelectionEntry {
    pub fn symbol(&self) -> MenuSymbol {
        MenuSymbol::Team {
            id: self.id,
            icon_spec: self.icon_spec.clone(),
            color: self.color,
            has_participants: self.has_participants,
        }
    }
}

/// One ordered `Game.Network.Clients` row displayed by
/// `C4MainMenu::ActivateHost` (C4MainMenu.cpp:502-518).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct HostDisconnectClientEntry {
    pub client_id: i32,
    pub caption: String,
    pub activated: bool,
}

/// The active in-game menu: one `C4MainMenu` page (C4Menu state per
/// C4Menu.h:134-268 — caption, symbol, item list, selection, permanent flag
/// and close command).
pub struct IngameMenuState {
    /// `C4MainMenu::Player`: the player whose controls operate this menu.
    /// `None` mirrors the pre-`Init` `NO_OWNER` state.
    player: Option<i32>,
    page: MenuPage,
    style: MenuStyle,
    caption: String,
    symbol: MenuSymbol,
    items: Vec<MenuItem>,
    selection: usize,
    /// `C4Menu::SetPermanent` — permanent menus stay open on Enter.
    permanent: bool,
    /// `C4Menu::SetCloseCommand`.
    close_action: Option<MenuAction>,
    /// `C4Menu::TimeOnSelection` for the tooltip delay (C4Menu.cpp:804-821).
    time_on_selection: u32,
    /// Presentation-only `C4GUI::ScrollWindow::iScrollY`. Unlike the
    /// synchronized selection, this survives draws and wheel input locally.
    scroll_y: Cell<i32>,
    /// Selection for which `ScrollRangeInView` was last applied. Layout is
    /// computed from `&self`, so this marker lets it perform the native
    /// one-shot adjustment without undoing a later wheel scroll every frame.
    scroll_selection: Cell<Option<usize>>,
    /// Absolute screen position installed by dragging the wooden title.
    /// `None` restores C4MainMenu's normal Left|Bottom viewport anchor.
    location: Cell<Option<(i32, i32)>>,
    /// Last owning viewport used to initialize the external dialog. Native
    /// sets `ResetMenuPositions` whenever a viewport output rect changes,
    /// including split-screen relayouts that do not resize the OS window.
    last_area: Cell<Option<Rect>>,
    /// Normal menus keep their initialized row count when a refill shrinks;
    /// C4Menu only invalidates `LocationSet` on growth (C4Menu.cpp:961-968).
    normal_lines: Cell<Option<i32>>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum IngameMenuPointerTarget {
    Item(usize),
    Close,
    Title,
    Background,
}

impl IngameMenuState {
    fn new(
        page: MenuPage,
        caption: impl Into<String>,
        symbol: MenuSymbol,
        items: Vec<MenuItem>,
        permanent: bool,
        close_action: Option<MenuAction>,
    ) -> Self {
        Self {
            player: None,
            page,
            style: MenuStyle::Context,
            caption: caption.into(),
            symbol,
            items,
            selection: 0,
            permanent,
            close_action,
            time_on_selection: 0,
            scroll_y: Cell::new(0),
            scroll_selection: Cell::new(None),
            location: Cell::new(None),
            last_area: Cell::new(None),
            normal_lines: Cell::new(None),
        }
    }

    /// `C4MainMenu::Init` / `InitRefSym` records the player number on every
    /// menu instance (C4MainMenu.cpp:45-57).
    pub fn for_player(mut self, player: i32) -> Self {
        self.player = Some(player);
        self
    }

    pub fn set_player(&mut self, player: i32) {
        self.player = Some(player);
    }

    pub fn player(&self) -> Option<i32> {
        self.player
    }

    fn team_menu(
        teams: &[TeamSelectionEntry],
        switching: bool,
        return_to_main: bool,
    ) -> Self {
        let items = teams
            .iter()
            .map(|team| {
                MenuItem::new(
                    team.caption.clone(),
                    team.symbol(),
                    if switching {
                        MenuAction::SwitchTeam(team.id)
                    } else {
                        MenuAction::SelectTeam(team.id)
                    },
                    Some(&format!("Join team {}", team.caption)),
                )
            })
            .collect();
        Self::new(
            MenuPage::TeamSelection,
            "Select team",
            MenuSymbol::GuiIcon(ICO_TEAM),
            items,
            false,
            return_to_main.then_some(MenuAction::ActivateMain),
        )
    }

    /// Initial `C4Player::ActivateMenuTeamSelection(false)` and
    /// `C4MainMenu::Refill` for `C4MN_TeamSelection`
    /// (C4Player.cpp:1762-1771; C4MainMenu.cpp:175-232).
    ///
    /// C++ resolves each team's `IconSpec`, then falls back to a colorized
    /// crew for occupied teams or the team GUI icon for empty teams.
    pub fn team_selection_menu(teams: &[TeamSelectionEntry]) -> Self {
        Self::team_menu(teams, false, false)
    }

    /// `ActivateMenuTeamSelection(true)` while the player is still in
    /// `PS_TeamSelection`: keep `TeamSel:<id>` actions, but install the
    /// back-to-main close command (C4Player.cpp:1762-1771).
    pub fn team_selection_menu_from_main(teams: &[TeamSelectionEntry]) -> Self {
        Self::team_menu(teams, false, true)
    }

    /// Mid-round `C4Player::ActivateMenuTeamSelection(true)`: the same team
    /// rows dispatch `TeamSwitch:<id>`, and closing returns to the main page
    /// (C4Player.cpp:1762-1771; C4MainMenu.cpp:175-232,909-918).
    pub fn team_switch_menu(teams: &[TeamSelectionEntry]) -> Self {
        Self::team_menu(teams, true, true)
    }

    /// `C4MainMenu::ActivateMain` (C4MainMenu.cpp:643-715). Returns `None`
    /// when no entry applies (`if (GetItemCount() == 0) Close(false)`).
    pub fn main_menu(cond: &MainMenuConditions) -> Option<Self> {
        let mut items = Vec::new();
        // Goals + Rules (player menu only, C4MainMenu.cpp:659-665)
        if cond.has_player {
            items.push(MenuItem::new(
                "Goals",
                MenuSymbol::Menu(4),
                MenuAction::ActivateGoals,
                Some("The round ends when all goals are fulfilled."),
            ));
            items.push(MenuItem::new(
                "Rules",
                MenuSymbol::Menu(5),
                MenuAction::ActivateRules,
                Some("Rules for this round."),
            ));
        }
        // Observer menu in free viewport (C4MainMenu.cpp:666-670)
        if !cond.has_player {
            items.push(MenuItem::new(
                "View",
                MenuSymbol::GuiIcon(ICO_VIEW),
                MenuAction::ActivateObserver,
                Some("Select view mode."),
            ));
        }
        // Hostility (C4MainMenu.cpp:671-676)
        if cond.has_player && cond.player_count > 1 {
            items.push(MenuItem::new(
                "Attack",
                MenuSymbol::Menu(7),
                MenuAction::ActivateHostility,
                Some("Order your clonks to attack other players."),
            ));
        }
        // Team change (C4MainMenu.cpp:677-682)
        if cond.has_player && cond.team_switch_allowed {
            items.push(MenuItem::new(
                "Select team",
                MenuSymbol::GuiIcon(ICO_TEAM),
                MenuAction::ActivateTeamSelection,
                Some("Allows you to join a different team."),
            ));
        }
        // Player join (C4MainMenu.cpp:683-687)
        if cond.player_count < cond.max_players && !cond.is_league {
            items.push(MenuItem::new(
                "Join player",
                MenuSymbol::PlayerColor,
                MenuAction::ActivateNewPlayer,
                Some("Have another player join the game (player files from the working directory)."),
            ));
        }
        // Save game (C4MainMenu.cpp:688-692)
        if cond.has_player && (!cond.network_enabled || cond.network_host) {
            items.push(MenuItem::new(
                "Save game",
                MenuSymbol::Menu(0),
                MenuAction::ActivateSavegame,
                Some("Save this game so it can be resumed later."),
            ));
        }
        // Options (C4MainMenu.cpp:693-694)
        items.push(MenuItem::new(
            "Options",
            MenuSymbol::Options(0),
            MenuAction::ActivateOptions,
            Some("Change program options."),
        ));
        // Disconnect (C4MainMenu.cpp:695-704)
        if cond.network_enabled {
            if cond.network_host && cond.network_has_clients {
                items.push(MenuItem::new(
                    "Disconnect",
                    MenuSymbol::GuiIcon(ICO_DISCONNECT),
                    MenuAction::ActivateHostDisconnect,
                    Some("Kick certain clients from the game."),
                ));
            }
            if !cond.network_host {
                items.push(MenuItem::new(
                    "Disconnect",
                    MenuSymbol::GuiIcon(ICO_DISCONNECT),
                    MenuAction::ActivateClientDisconnect,
                    Some("Disconnect the game from the host."),
                ));
            }
        }
        // Surrender (C4MainMenu.cpp:705-707)
        if cond.has_player {
            items.push(MenuItem::new(
                "Surrender",
                MenuSymbol::GuiIcon(ICO_SURRENDER),
                MenuAction::ActivateSurrender,
                Some("Leave the game with evaluation."),
            ));
        }
        // Abort (C4MainMenu.cpp:708-710)
        if cond.is_fullscreen {
            items.push(MenuItem::new(
                "Abort round",
                MenuSymbol::GuiIcon(ICO_EXIT),
                MenuAction::Abort,
                Some("Abort the round without evaluation."),
            ));
        }
        // No empty menus (C4MainMenu.cpp:711-712)
        if items.is_empty() {
            return None;
        }
        // Caption IDS_MENU_CPMAIN / IDS_MENU_OBSERVER, symbol fctOKCancel
        // phase (1,1) (C4MainMenu.cpp:649-653).
        Some(Self::new(
            MenuPage::Main,
            if cond.has_player {
                "Player Menu"
            } else {
                "Observer Menu"
            },
            MenuSymbol::OkCancel(1, 1),
            items,
            false,
            None,
        ))
    }

    fn hostility_items(entries: &[HostilityEntry]) -> Vec<MenuItem> {
        entries
            .iter()
            .map(|entry| {
                let caption = if entry.hostile {
                    format!("Attack {}", entry.name)
                } else {
                    format!("Don't attack {}", entry.name)
                };
                let relation = if entry.opponent_hostile {
                    "hostile"
                } else {
                    "friendly"
                };
                let not_attacked = if entry.hostile { "" } else { "not " };
                MenuItem::new(
                    caption,
                    MenuSymbol::Hostility {
                        opponent: entry.opponent,
                        hostile: entry.hostile,
                    },
                    MenuAction::ToggleHostility(entry.opponent),
                    Some(&format!(
                        "{} is currently {} and will {}be attacked.",
                        entry.name, relation, not_attacked
                    )),
                )
            })
            .collect()
    }

    /// `C4MainMenu::ActivateHostility` and its first refill
    /// (C4MainMenu.cpp:138-168,717-732).
    pub fn hostility_menu(entries: &[HostilityEntry]) -> Self {
        let mut menu = Self::new(
            MenuPage::Hostility,
            "Attack",
            MenuSymbol::Menu(7),
            Self::hostility_items(entries),
            true,
            Some(MenuAction::ActivateMain),
        );
        menu.style = MenuStyle::Normal;
        menu
    }

    /// Native `ClearItems(false)` + `AdjustSelection` refill. Keeping the
    /// menu instance preserves its permanent state, dragged position,
    /// selection timer and numeric selection across the Tick35 rebuild.
    pub fn refill_hostility(&mut self, entries: &[HostilityEntry]) {
        debug_assert_eq!(self.page, MenuPage::Hostility);
        let previous_count = self.items.len();
        self.items = Self::hostility_items(entries);
        if self.selection >= self.items.len() {
            self.selection = self.items.len().saturating_sub(1);
            self.time_on_selection = 0;
            self.scroll_selection.set(None);
        } else if previous_count == 0 && !self.items.is_empty() {
            // Native empty menus retain Selection=-1. AdjustSelection picks
            // row zero when the first item appears and resets its tooltip age.
            self.time_on_selection = 0;
        }
        if self.items.len() > previous_count {
            self.location.set(None);
            self.last_area.set(None);
            self.normal_lines.set(None);
            self.scroll_selection.set(None);
        }
    }

    /// `C4MainMenu::ActivateObserver` and the `C4MN_Observer` refill
    /// (C4MainMenu.cpp:235-273,950-961): Free first, then each visible
    /// runtime player in player-list order. The page is non-permanent, so
    /// Enter closes it before dispatching the selected `Observe:*` action.
    pub fn observer_menu(
        players: &[ObserverPlayerEntry],
        current_target: ObserverTarget,
    ) -> Self {
        let mut items = Vec::with_capacity(players.len() + 1);
        items.push(MenuItem::new(
            "free view",
            MenuSymbol::GuiIcon(ICO_STAR),
            MenuAction::Observe(ObserverTarget::Free),
            Some("Freely scroll around the map."),
        ));
        items.extend(players.iter().map(|player| {
            MenuItem::new(
                player.name.clone(),
                MenuSymbol::PlayerColor,
                MenuAction::Observe(ObserverTarget::Player(player.id)),
                Some(&format!("Follow view of player {}.", player.name)),
            )
        }));
        let mut menu = Self::new(
            MenuPage::Observer,
            "View",
            MenuSymbol::GuiIcon(ICO_VIEW),
            items,
            false,
            Some(MenuAction::ActivateMain),
        );
        let selection = menu
            .items
            .iter()
            .position(|item| item.action == MenuAction::Observe(current_target))
            .unwrap_or(0);
        menu.set_selection(selection);
        menu
    }

    /// `C4MainMenu::ActivateOptions` (C4MainMenu.cpp:553-580).
    pub fn options_menu(flags: &OptionFlags, selection: usize) -> Self {
        let mut items = vec![
            MenuItem::new(
                "Sound",
                MenuSymbol::Options(17 + u8::from(flags.sound)),
                MenuAction::ToggleSound,
                None,
            ),
            MenuItem::new(
                "Music",
                MenuSymbol::Options(1 + u8::from(flags.music)),
                MenuAction::ToggleMusic,
                None,
            ),
        ];
        if flags.mouse_shown {
            items.push(MenuItem::new(
                "Mouse control",
                MenuSymbol::Options(11 + u8::from(flags.mouse)),
                MenuAction::ToggleMouseControl,
                None,
            ));
        }
        items.push(MenuItem::new(
            "Display",
            MenuSymbol::Menu(8),
            MenuAction::ActivateDisplay,
            None,
        ));
        let mut menu = Self::new(
            MenuPage::Options,
            "Options",
            MenuSymbol::Options(0),
            items,
            true,
            Some(MenuAction::ActivateMain),
        );
        menu.set_selection(selection);
        menu
    }

    /// `C4MainMenu::ActivateDisplay` (C4MainMenu.cpp:582-641).
    pub fn display_menu(flags: &DisplayFlags, selection: usize) -> Self {
        let mut items = vec![
            MenuItem::new(
                "Player names",
                MenuSymbol::Options(7 + u8::from(flags.player_names)),
                MenuAction::Display(DisplayToggle::PlayerNames),
                Some("Displays player names above enemy clonks."),
            ),
            MenuItem::new(
                "Clonk names",
                MenuSymbol::Options(9 + u8::from(flags.clonk_names)),
                MenuAction::Display(DisplayToggle::ClonkNames),
                Some("Displays clonk names above enemy clonks."),
            ),
            MenuItem::new(
                "Portraits",
                MenuSymbol::Options(13 + u8::from(flags.portraits)),
                MenuAction::Display(DisplayToggle::Portraits),
                None,
            ),
            MenuItem::new(
                "Commands",
                MenuSymbol::Options(19 + u8::from(flags.show_commands)),
                MenuAction::Display(DisplayToggle::ShowCommands),
                None,
            ),
            MenuItem::new(
                "Keys",
                MenuSymbol::Options(21 + u8::from(flags.show_command_keys)),
                MenuAction::Display(DisplayToggle::ShowCommandKeys),
                None,
            ),
        ];
        if flags.is_fullscreen {
            items.push(MenuItem::new(
                format!("Title board: {}", flags.upper_board.label()),
                MenuSymbol::Options(3 + u8::from(flags.upper_board != UpperBoardMode::Hide)),
                MenuAction::Display(DisplayToggle::UpperBoard),
                None,
            ));
            items.push(MenuItem::new(
                "FPS Display",
                MenuSymbol::Options(5 + u8::from(flags.fps)),
                MenuAction::Display(DisplayToggle::Fps),
                None,
            ));
            items.push(MenuItem::new(
                "Clock",
                MenuSymbol::Options(15 + u8::from(flags.clock)),
                MenuAction::Display(DisplayToggle::Clock),
                None,
            ));
            items.push(MenuItem::new(
                "White Chat",
                MenuSymbol::Options(3 + u8::from(flags.white_chat)),
                MenuAction::Display(DisplayToggle::WhiteChat),
                Some("Displays messages in the ingame chat in white and only the sender in player color.\nMay improve readability."),
            ));
        }
        let mut menu = Self::new(
            MenuPage::Display,
            "Display",
            MenuSymbol::Menu(8),
            items,
            true,
            Some(MenuAction::ActivateOptions),
        );
        menu.set_selection(selection);
        menu
    }

    /// `C4MainMenu::ActivateSavegame` (C4MainMenu.cpp:422-500): ten slots,
    /// each captioned IDS_MENU_CPSAVEGAME with the slot-number icon.
    pub fn savegame_menu(slots: &[SaveSlotState; 10]) -> Self {
        let items = slots
            .iter()
            .enumerate()
            .map(|(index, slot)| {
                let number = (index + 1) as u8;
                MenuItem::new(
                    "Save game",
                    MenuSymbol::SaveSlot {
                        slot: number,
                        free: slot.free,
                    },
                    MenuAction::SaveSlot(number),
                    Some("Save this game so it can be resumed later."),
                )
            })
            .collect();
        Self::new(
            MenuPage::Savegame,
            "Save game",
            MenuSymbol::Menu(0),
            items,
            true,
            Some(MenuAction::ActivateMain),
        )
    }

    /// `C4MainMenu::ActivateSurrender` (C4MainMenu.cpp:538-551).
    pub fn surrender_menu() -> Self {
        Self::new(
            MenuPage::Surrender,
            "Surrender",
            MenuSymbol::GuiIcon(ICO_SURRENDER),
            vec![
                MenuItem::new("Yes", MenuSymbol::OkCancel(3, 0), MenuAction::Surrender, None),
                MenuItem::new("No", MenuSymbol::OkCancel(1, 0), MenuAction::NoOp, None),
            ],
            false,
            Some(MenuAction::ActivateMain),
        )
    }

    /// `C4MainMenu::ActivateClient` (C4MainMenu.cpp:522-536).
    pub fn client_disconnect_menu() -> Self {
        Self::new(
            MenuPage::ClientDisconnect,
            "Disconnect from server",
            MenuSymbol::GuiIcon(ICO_DISCONNECT),
            vec![
                MenuItem::new("Yes", MenuSymbol::OkCancel(3, 0), MenuAction::Part, None),
                MenuItem::new("No", MenuSymbol::OkCancel(1, 0), MenuAction::NoOp, None),
            ],
            false,
            Some(MenuAction::ActivateMain),
        )
    }

    /// `C4MainMenu::ActivateHost` (C4MainMenu.cpp:502-518). Every registered
    /// client is selectable, including host ID zero (whose command is a
    /// deliberate no-op), and the permanent page stays open after commands.
    pub fn host_disconnect_menu(clients: &[HostDisconnectClientEntry]) -> Self {
        let items = clients
            .iter()
            .map(|client| {
                let icon = if client.client_id == 0 {
                    ICO_HOST
                } else if client.activated {
                    ICO_CLIENT
                } else {
                    ICO_OBSERVER_CLIENT
                };
                MenuItem::new(
                    client.caption.clone(),
                    MenuSymbol::GuiIcon(icon),
                    MenuAction::KickClient(client.client_id),
                    None,
                )
            })
            .collect();
        Self::new(
            MenuPage::HostDisconnect,
            "Disconnect client",
            MenuSymbol::GuiIcon(ICO_DISCONNECT),
            items,
            true,
            Some(MenuAction::ActivateMain),
        )
    }

    /// `C4MainMenu::ActivateGoals` (C4MainMenu.cpp:332-380).
    pub fn goals_menu(goals: &[GoalRuleEntry]) -> Self {
        let items = goals
            .iter()
            .map(|goal| {
                MenuItem::new(
                    goal.name.clone(),
                    MenuSymbol::Definition {
                        id: goal.definition_id.clone(),
                        fulfilled: goal.fulfilled,
                    },
                    MenuAction::GoalInfo(goal.definition_id.clone()),
                    goal.description.as_deref(),
                )
            })
            .collect();
        Self::new(
            MenuPage::Goals,
            "Goals",
            MenuSymbol::Menu(4),
            items,
            false,
            Some(MenuAction::ActivateMain),
        )
    }

    /// `C4MainMenu::ActivateRules` (C4MainMenu.cpp:382-405).
    pub fn rules_menu(rules: &[GoalRuleEntry]) -> Self {
        let items = rules
            .iter()
            .map(|rule| {
                MenuItem::new(
                    rule.name.clone(),
                    MenuSymbol::Definition {
                        id: rule.definition_id.clone(),
                        fulfilled: false,
                    },
                    MenuAction::RuleInfo(rule.definition_id.clone()),
                    rule.description.as_deref(),
                )
            })
            .collect();
        Self::new(
            MenuPage::Rules,
            "Rules",
            MenuSymbol::Menu(5),
            items,
            false,
            Some(MenuAction::ActivateMain),
        )
    }

    /// `C4MainMenu::ActivateNewPlayer` (C4MainMenu.cpp:59-122). The menu
    /// caption is IDS_MENU_NOPLRFILES, which doubles as the empty-menu text.
    pub fn new_player_menu(players: &[NewPlayerEntry]) -> Self {
        let items = players
            .iter()
            .map(|player| {
                MenuItem::new(
                    format!("Join player: {}", player.name),
                    MenuSymbol::PlayerColor,
                    MenuAction::JoinPlayer(player.file.clone()),
                    None,
                )
            })
            .collect();
        Self::new(
            MenuPage::NewPlayer,
            "No additional player files available.",
            MenuSymbol::PlayerColor,
            items,
            false,
            Some(MenuAction::ActivateMain),
        )
    }

    pub fn page(&self) -> MenuPage {
        self.page
    }

    pub fn caption(&self) -> &str {
        if self.style == MenuStyle::Normal {
            self.items
                .get(self.selection)
                .map(|item| item.caption.as_str())
                .filter(|caption| !caption.is_empty())
                .unwrap_or(&self.caption)
        } else {
            &self.caption
        }
    }

    pub fn items(&self) -> &[MenuItem] {
        &self.items
    }

    pub fn selection(&self) -> usize {
        self.selection
    }

    /// The highlighted `Observe:*` target when this is the observer page.
    pub fn selected_observer_target(&self) -> Option<ObserverTarget> {
        if self.page != MenuPage::Observer {
            return None;
        }
        match self.items.get(self.selection).map(|item| &item.action) {
            Some(MenuAction::Observe(target)) => Some(*target),
            _ => None,
        }
    }

    pub fn is_permanent(&self) -> bool {
        self.permanent
    }

    pub fn close_action(&self) -> Option<&MenuAction> {
        self.close_action.as_ref()
    }

    /// `C4Menu::SetSelection` (used to restore the cursor when option pages
    /// reopen, C4MainMenu.cpp:575,636).
    pub fn set_selection(&mut self, selection: usize) {
        self.selection = selection.min(self.items.len().saturating_sub(1));
        self.time_on_selection = 0;
        // SetSelection calls AdjustPosition even when the same item is
        // selected again (C4Menu.cpp:560-592).
        self.scroll_selection.set(None);
    }

    /// Current logical-pixel `C4GUI::ScrollWindow` displacement.
    pub fn scroll_y(&self) -> i32 {
        self.scroll_y.get()
    }

    /// `C4GUI::ScrollWindow::ScrollBy`: clamp a logical-pixel displacement
    /// without changing the menu selection.
    pub fn scroll_by(
        &self,
        amount: i32,
        area: Rect,
        font: &HudFont<'_>,
        gfx: &IngameMenuGraphics,
    ) -> bool {
        let layout = self.layout(area, font, gfx);
        let old = self.scroll_y.get();
        let new = old.saturating_add(amount).clamp(0, layout.max_scroll);
        if new == old {
            return false;
        }
        self.scroll_y.set(new);
        true
    }

    /// Restore C4MainMenu's anchored placement. Reinitializing a location
    /// also reruns `AdjustPosition` for the current selection.
    pub fn reset_location(&mut self) {
        self.location.set(None);
        self.last_area.set(None);
        self.normal_lines.set(None);
        self.scroll_selection.set(None);
    }

    /// Install an absolute top-left position from title dragging. Native
    /// `Element::DoDragging` does not clamp each motion to the viewport.
    pub fn set_location(&mut self, location: (i32, i32)) {
        self.location.set(Some(location));
    }

    /// Current externally drawn dialog bounds in screen coordinates.
    pub fn bounds(&self, area: Rect, font: &HudFont<'_>, gfx: &IngameMenuGraphics) -> Rect {
        self.layout(area, font, gfx).bounds
    }

    /// Whether a point lies in the visible `ScrollWindow` client. The title,
    /// frame and optional command strip deliberately do not receive wheel
    /// input through this helper.
    pub fn client_contains(
        &self,
        area: Rect,
        font: &HudFont<'_>,
        gfx: &IngameMenuGraphics,
        point: GuiPoint,
    ) -> bool {
        rect_contains_point(area, point)
            && rect_contains_point(self.layout(area, font, gfx).client_rect(), point)
    }

    /// Advances `TimeOnSelection` once per frame while the menu is shown
    /// (C4Menu::Draw, C4Menu.cpp:805).
    pub fn tick(&mut self) {
        self.time_on_selection = self.time_on_selection.saturating_add(1);
    }

    /// `C4Menu::Control` (C4Menu.cpp:433-484): left/right move one cell and
    /// up/down move one row (`Columns`, five for normal icon menus), with the
    /// native incomplete-grid wrapping behavior.
    pub fn handle_command(
        &mut self,
        command: ControlCommand,
        kind: CommandKind,
    ) -> Option<MenuOutcome> {
        if !matches!(
            kind,
            CommandKind::Press | CommandKind::Single | CommandKind::Double
        ) {
            return None;
        }
        match command {
            ControlCommand::MenuLeft => {
                self.move_selection(-1);
                None
            }
            ControlCommand::MenuRight => {
                self.move_selection(1);
                None
            }
            ControlCommand::MenuUp => {
                self.move_selection_vertical(-1);
                None
            }
            ControlCommand::MenuDown => {
                self.move_selection_vertical(1);
                None
            }
            ControlCommand::MenuSelect
            | ControlCommand::MenuEnter
            | ControlCommand::MenuEnterAll => self.enter(),
            ControlCommand::MenuClose => Some(MenuOutcome::Closed {
                close_action: self.close_action.clone(),
            }),
            _ => None,
        }
    }

    /// `C4Menu::Enter` (C4Menu.cpp:498-521).
    fn enter(&self) -> Option<MenuOutcome> {
        self.items
            .get(self.selection)
            .map(|item| MenuOutcome::Action {
                action: item.action.clone(),
                close_menu: !self.permanent,
            })
    }

    /// `C4Menu::MoveSelection` with wrap (C4Menu.cpp:439-461, 535-555).
    fn move_selection(&mut self, delta: i32) {
        if self.items.is_empty() {
            return;
        }
        let len = self.items.len() as i32;
        let selection = (self.selection as i32 + delta).rem_euclid(len) as usize;
        if selection != self.selection {
            self.selection = selection;
            self.time_on_selection = 0;
            self.scroll_selection.set(None);
        }
    }

    fn move_selection_vertical(&mut self, direction: i32) {
        if self.items.is_empty() {
            return;
        }
        let columns = match self.style {
            MenuStyle::Normal => 5,
            MenuStyle::Context => 1,
        };
        let selection = self.selection as i32;
        let count = self.items.len() as i32;
        let mut delta = direction * columns;
        if delta < 0 && selection + delta < 0 {
            while selection + delta + columns < count {
                delta += columns;
            }
        } else if delta > 0 && selection + delta >= count {
            while selection + delta - columns >= 0 {
                delta -= columns;
            }
        }
        self.move_selection(delta);
    }

    /// Draws the menu with the classic C4Menu context-style furniture. The
    /// `area` is the viewport rect the menu aligns in (C4Menu::InitLocation,
    /// C4Menu.cpp:642-753; alignment Left|Bottom, C4MainMenu.cpp:654).
    pub fn render(
        &self,
        surface: &mut Surface,
        area: Rect,
        font: &HudFont<'_>,
        tiny_font: Option<&HudFont<'_>>,
        gfx: &IngameMenuGraphics,
    ) {
        self.render_with_gamma(surface, area, font, tiny_font, gfx, None);
    }

    pub fn render_with_gamma(
        &self,
        surface: &mut Surface,
        area: Rect,
        font: &HudFont<'_>,
        tiny_font: Option<&HudFont<'_>>,
        gfx: &IngameMenuGraphics,
        gamma: Option<&GammaRamp>,
    ) {
        let layout = self.layout(area, font, gfx);
        let previous_clip = surface.clip();
        let viewport_clip = previous_clip
            .map(|clip| clip.intersection(area).unwrap_or(Rect::new(0, 0, 0, 0)))
            .unwrap_or(area);
        surface.set_clip(viewport_clip);
        draw_menu(self, &layout, surface, area, font, tiny_font, gfx, gamma);
        match previous_clip {
            Some(clip) => surface.set_clip(clip),
            None => surface.clear_clip(),
        }
    }

    /// Hit-tests the externally drawn dialog inside its associated viewport.
    /// `C4GUI::Screen::MouseInput` first filters to `pForVP`, clips external
    /// dialogs to that viewport's output rect, and only then forwards to the
    /// menu elements (C4GUI.cpp:802-845).
    pub fn pointer_target(
        &self,
        area: Rect,
        font: &HudFont<'_>,
        gfx: &IngameMenuGraphics,
        point: GuiPoint,
    ) -> Option<IngameMenuPointerTarget> {
        if !rect_contains_point(area, point) {
            return None;
        }
        let layout = self.layout(area, font, gfx);
        if gfx.show_close_button && rect_contains_point(layout.close_button_rect(), point) {
            return Some(IngameMenuPointerTarget::Close);
        }
        if rect_contains_point(layout.title_rect(), point) {
            return Some(IngameMenuPointerTarget::Title);
        }
        if rect_contains_point(layout.client_rect(), point) {
            if let Some(index) = self.items.iter().enumerate().find_map(|(index, _)| {
                layout
                    .item_rect(index)
                    .filter(|rect| rect_contains_point(*rect, point))
                    .map(|_| index)
            }) {
                return Some(IngameMenuPointerTarget::Item(index));
            }
        }
        rect_contains_point(layout.bounds, point).then_some(IngameMenuPointerTarget::Background)
    }

    pub fn close_button_rect(
        &self,
        area: Rect,
        font: &HudFont<'_>,
        gfx: &IngameMenuGraphics,
    ) -> Rect {
        self.layout(area, font, gfx).close_button_rect()
    }

    /// Menu geometry per `C4Menu::InitLocation`/`InitSize`
    /// (C4Menu.cpp:642-783), including the five-column 35px normal grid.
    fn layout(&self, area: Rect, font: &HudFont<'_>, gfx: &IngameMenuGraphics) -> MenuLayout {
        if self
            .last_area
            .replace(Some(area))
            .is_some_and(|previous| previous != area)
        {
            self.location.set(None);
            self.normal_lines.set(None);
            self.scroll_selection.set(None);
        }
        let (columns, cell_width, item_height, client_width) = match self.style {
            MenuStyle::Normal => (5, SYMBOL_SIZE, SYMBOL_SIZE, 5 * SYMBOL_SIZE),
            MenuStyle::Context => {
                // ItemHeight = max(C4MN_SymbolSize, font line height).
                let item_height = MN_SYMBOL_SIZE.max(font.line_height());
                // Caption contributes ItemHeight + 16 (C4Menu.cpp:652-655).
                let mut item_width = font.text_width(&self.caption) + item_height + 16;
                for item in &self.items {
                    item_width = item_width.max(font.text_width(&item.caption) + item_height);
                }
                item_width += 3; // keep text off the right border
                (1, item_width, item_height, item_width)
            }
        };

        let area_w = area.width as i32;
        let area_h = area.height as i32;
        let item_count = i32::try_from(self.items.len()).unwrap_or(i32::MAX);
        let row_count = item_count / columns + i32::from(item_count % columns != 0);
        // Lines = row count clamped to the viewport (C4Menu.cpp:715-720).
        let computed_lines = row_count
            .min(((area_h - 100) / item_height.max(1)).max(1))
            .max(1);
        let lines = if self.style == MenuStyle::Normal {
            self.normal_lines.get().unwrap_or_else(|| {
                self.normal_lines.set(Some(computed_lines));
                computed_lines
            })
        } else {
            computed_lines
        };

        // Margins: title bar on top (Dialog::GetMarginTop,
        // C4GuiDialogs.h:95), C4MN_FrameWidth left/right/bottom plus the
        // extra bar when menu controls are drawn (C4Menu.h:262-264).
        let title_height = font.line_height().max(MIN_WOOD_BAR_HGT);
        let extra_height = if gfx.show_commands { MN_SYMBOL_SIZE } else { 0 };
        let width = client_width + 2 * MN_FRAME_WIDTH;
        let height = lines * item_height + title_height + extra_height + MN_FRAME_WIDTH;

        // Alignment Left|Bottom (C4Menu.cpp:734-745): X = C4SymbolSize,
        // Y = areaH - C4SymbolSize - height, centered when oversized.
        let mut x = SYMBOL_SIZE;
        let mut y = area_h - SYMBOL_SIZE - height;
        if width > area_w - 2 * SYMBOL_SIZE {
            x = (area_w - width) / 2;
        }
        if height > area_h - 2 * SYMBOL_SIZE {
            y = (area_h - height) / 2;
        }
        x += area.x;
        y += area.y;
        if let Some((location_x, location_y)) = self.location.get() {
            x = location_x;
            y = location_y;
        }

        let client_height = lines.saturating_mul(item_height);
        let content_height = row_count.saturating_mul(item_height);
        let max_scroll = content_height.saturating_sub(client_height).max(0);
        let mut scroll_y = self.scroll_y.get().clamp(0, max_scroll);

        // `AdjustPosition` runs after selection changes (and InitLocation),
        // not on every draw. Preserve wheel displacement while selection is
        // unchanged, and minimally reveal only the newly selected row.
        if self.scroll_selection.get() != Some(self.selection) {
            if lines > 1 && !self.items.is_empty() {
                let selection_y = i32::try_from(self.selection)
                    .unwrap_or(i32::MAX)
                    .checked_div(columns)
                    .unwrap_or_default()
                    .saturating_mul(item_height);
                scroll_y = scroll_range_in_view(
                    scroll_y,
                    selection_y,
                    item_height,
                    client_height,
                    max_scroll,
                );
            }
            self.scroll_selection.set(Some(self.selection));
        }
        self.scroll_y.set(scroll_y);

        MenuLayout {
            bounds: Rect::new(x, y, width as u32, height as u32),
            client_width,
            cell_width,
            item_height,
            title_height,
            columns,
            lines,
            scroll_y,
            max_scroll,
        }
    }
}

fn scroll_range_in_view(
    scroll_y: i32,
    range_y: i32,
    range_height: i32,
    viewport_height: i32,
    max_scroll: i32,
) -> i32 {
    let mut scroll_y = scroll_y.clamp(0, max_scroll);
    if scroll_y > range_y {
        scroll_y = range_y;
    } else if scroll_y.saturating_add(viewport_height) < range_y.saturating_add(range_height) {
        scroll_y = range_y
            .saturating_add(range_height)
            .saturating_sub(viewport_height);
    }
    scroll_y.clamp(0, max_scroll)
}

/// Computed menu geometry (see [`IngameMenuState::layout`]).
struct MenuLayout {
    bounds: Rect,
    client_width: i32,
    cell_width: i32,
    item_height: i32,
    title_height: i32,
    columns: i32,
    lines: i32,
    scroll_y: i32,
    max_scroll: i32,
}

impl MenuLayout {
    fn title_rect(&self) -> Rect {
        Rect::new(
            self.bounds.x,
            self.bounds.y,
            self.bounds.width,
            self.title_height as u32,
        )
    }

    fn client_rect(&self) -> Rect {
        Rect::new(
            self.bounds.x + MN_FRAME_WIDTH,
            self.bounds.y + self.title_height,
            self.client_width as u32,
            self.lines.saturating_mul(self.item_height) as u32,
        )
    }

    fn close_button_rect(&self) -> Rect {
        // Dialog::SetTitle uses GetToprightCornerRect(16,16,4,4,0).
        Rect::new(
            self.bounds.x + self.bounds.width as i32 - 20,
            self.bounds.y + 4,
            16,
            16,
        )
    }

    fn item_rect(&self, index: usize) -> Option<Rect> {
        let index = i32::try_from(index).unwrap_or(i32::MAX);
        let column_x = index
            .rem_euclid(self.columns)
            .saturating_mul(self.cell_width);
        let row_y = index
            .checked_div(self.columns)
            .unwrap_or_default()
            .saturating_mul(self.item_height)
            .saturating_sub(self.scroll_y);
        let client = self.client_rect();
        let x = client.x.saturating_add(column_x);
        let y = client.y.saturating_add(row_y);
        let bottom = y.saturating_add(self.item_height);
        let client_bottom = client.y.saturating_add(client.height as i32);
        (bottom > client.y && y < client_bottom)
            .then(|| Rect::new(x, y, self.cell_width as u32, self.item_height as u32))
    }
}

fn rect_contains_point(rect: Rect, point: GuiPoint) -> bool {
    point.x >= rect.x as f32
        && point.y >= rect.y as f32
        && point.x < (rect.x + rect.width as i32) as f32
        && point.y < (rect.y + rect.height as i32) as f32
}

/// Graphics.c4g sheets and flags the renderer needs; missing sheets degrade
/// to text-only rendering (headless tests without game data).
#[derive(Default)]
pub struct IngameMenuGraphics {
    /// Shared HUD facets used by composite object-menu symbols.
    pub hud: HudGraphics,
    /// Runtime player colors keyed by C4Player number.
    pub owner_colors: HashMap<i32, Color>,
    /// Resource-backed `C4Player::BigIcon` surfaces keyed by player number.
    pub hostility_big_icons: HashMap<i32, ImageData>,
    /// Menu.png (35x35 phases, C4GraphicsResource.cpp:219).
    pub menu: Option<ImageData>,
    /// Options.png (35x35 phases, C4GraphicsResource.cpp:224).
    pub options: Option<ImageData>,
    /// Control.png (C4GraphicsResource.cpp:200-205).
    pub control: Option<ImageData>,
    /// GUIIcons.png (40x40 cells, C4Gui.cpp:1094-1095).
    pub gui_icons: Option<ImageData>,
    /// Player.png (`fctPlayerClr`).
    pub player: Option<ImageData>,
    /// GUICaption.png (192x23, 3-slice border 32, C4Gui.cpp:1088).
    pub caption_bar: Option<ImageData>,
    /// Definition pictures for [`MenuSymbol::Definition`] items.
    pub definition_icons: HashMap<String, ImageData>,
    /// Successfully resolved team `IconSpec` pictures keyed by team ID.
    /// Missing entries deliberately select the classic fallback chain.
    pub team_icons: HashMap<i32, ImageData>,
    /// `CStdFont::SetCustomImages(Game.Defs)` results for `{{TextSpec}}`
    /// tokens embedded in classic Info/Dialog menu text.
    pub font_images: HashMap<String, ImageData>,
    /// Full primary sprite sheet for the active script menu's captured
    /// FrameDecoration source definition.
    pub frame_decoration: Option<ImageData>,
    /// Absolute viewport location for `C4MN_Align_Free` object menus.
    pub menu_location: Option<(i32, i32)>,
    /// Presentation-only logical-pixel scroll displacement for the active
    /// script menu's client `ScrollWindow`.
    pub menu_scroll_y: i32,
    /// `Config.Graphics.ShowCommands` (C4Config.cpp:449) — draws the bottom
    /// command bar (C4Menu.cpp:851-880).
    pub show_commands: bool,
    /// `Config.Graphics.ShowPortraits`, used by `C4Player::DrawHostility`.
    pub show_portraits: bool,
    /// `C4Menu::HasMouse()` passed to `Dialog::SetTitle`: reserves and draws
    /// the title-bar close button only for the controlling mouse player.
    pub show_close_button: bool,
    /// `Config.Graphics.ShowCommandKeys` (C4Config.cpp:450) — key names on
    /// the command keys (C4ObjectCom.cpp:942-944).
    pub show_command_keys: bool,
    /// `PlrControlKeyName(..., CON_Throw)` — the menu-enter key label.
    pub throw_key: String,
    /// `PlrControlKeyName(..., CON_Special2)` — the menu-enter-all label.
    pub special2_key: String,
    /// `PlrControlKeyName(..., CON_Dig)` — the menu-close key label.
    pub dig_key: String,
}

impl IngameMenuGraphics {
    fn symbol_source(&self, symbol: &MenuSymbol) -> Option<(&ImageData, Rect)> {
        match symbol {
            // fctMenu phases: 35x35 grid (C4GraphicsResource.cpp:219).
            MenuSymbol::Menu(phase) => self
                .menu
                .as_ref()
                .map(|img| (img, Rect::new(35 * i32::from(*phase), 0, 35, 35))),
            // Save slots: row 1 = used, row 2 = free (C4MainMenu.cpp:493).
            MenuSymbol::SaveSlot { slot, free } => self.menu.as_ref().map(|img| {
                let row = if *free { 2 } else { 1 };
                (
                    img,
                    Rect::new(35 * (i32::from(*slot) - 1), 35 * row, 35, 35),
                )
            }),
            // fctOptions phases: 35x35 (C4GraphicsResource.cpp:224).
            MenuSymbol::Options(phase) => self
                .options
                .as_ref()
                .map(|img| (img, Rect::new(35 * i32::from(*phase), 0, 35, 35))),
            // fctOKCancel: base (128,100), 32x32 grid (C4GraphicsResource.cpp:204).
            MenuSymbol::OkCancel(px, py) => self.control.as_ref().map(|img| {
                (
                    img,
                    Rect::new(128 + 32 * i32::from(*px), 100 + 32 * i32::from(*py), 32, 32),
                )
            }),
            // GUIIcons.png: 40x40 cells, 6 per row (C4GuiLabels.cpp:441-450).
            MenuSymbol::GuiIcon(index) => self.gui_icons.as_ref().map(|img| {
                let x = i32::from(*index % 6) * 40;
                let y = i32::from(*index / 6) * 40;
                (img, Rect::new(x, y, 40, 40))
            }),
            MenuSymbol::PlayerColor => self
                .player
                .as_ref()
                .map(|img| (img, Rect::new(0, 0, img.width(), img.height()))),
            MenuSymbol::Hostility { .. } => None,
            MenuSymbol::Definition { id, .. } => self
                .definition_icons
                .get(id)
                .map(|img| (img, Rect::new(0, 0, img.width(), img.height()))),
            MenuSymbol::Team { .. } => None,
        }
    }

    fn draw_definition_symbol(
        &self,
        surface: &mut Surface,
        id: &str,
        fulfilled: bool,
        dest: Rect,
        gamma: Option<&GammaRamp>,
    ) {
        let side = SYMBOL_SIZE as u32;
        type DefinitionSymbolKey = (
            String,
            bool,
            Option<clonk_graphics::GpuTextureId>,
            Option<clonk_graphics::GpuTextureId>,
        );
        thread_local! {
            /// C++ caches this software-composed 35px facet. Reuse the same
            /// retained texture identity instead of growing the GPU cache on
            /// every open-menu frame.
            static DEFINITION_SYMBOLS: RefCell<HashMap<DefinitionSymbolKey, ImageData>> =
                RefCell::new(HashMap::new());
        }
        let definition = self.definition_icons.get(id);
        let captain = fulfilled.then(|| self.hud.captain.as_ref()).flatten();
        let key = (
            id.to_owned(),
            fulfilled,
            definition.map(ImageData::gpu_texture_id),
            captain.map(ImageData::gpu_texture_id),
        );
        let composed = DEFINITION_SYMBOLS.with(|symbols| {
            if let Some(image) = symbols.borrow().get(&key).cloned() {
                return image;
            }
            let mut surface = Surface::new(side, side, clonk_graphics::PixelFormat::Rgba8888);
            if let Some(image) = definition {
                let _ =
                    crate::copy_menu_image_aspect(&mut surface, image, Rect::new(0, 0, side, side));
            }
            if let Some(captain) = captain {
                // ActivateGoals first software-composites the Captain facet at
                // (17,2) on the 35x35 definition symbol. Preserve BltAlpha's
                // /256 cache quirk, then scale and gamma-draw that symbol once.
                let _ = crate::software_blit_menu_image(
                    &mut surface,
                    captain,
                    Rect::new(
                        SYMBOL_SIZE - captain.width() as i32 - 2,
                        2,
                        captain.width(),
                        captain.height(),
                    ),
                    clonk_graphics::BlitMode::Normal,
                );
            }
            let image = ImageData::new(side, side, surface.pixels().to_vec());
            symbols.borrow_mut().insert(key.clone(), image.clone());
            image
        });
        draw_image_region(surface, &composed, Rect::new(0, 0, side, side), dest, gamma);
    }

    fn draw_hostility_symbol(
        &self,
        surface: &mut Surface,
        opponent: i32,
        hostile: bool,
        dest: Rect,
        gamma: Option<&GammaRamp>,
    ) {
        if let Some(big_icon) = self
            .show_portraits
            .then(|| self.hostility_big_icons.get(&opponent))
            .flatten()
        {
            draw_image_region_aspect(
                surface,
                big_icon,
                Rect::new(0, 0, big_icon.width(), big_icon.height()),
                dest,
                false,
                gamma,
            );
        } else if let Some(crew) = self.hud.crew.as_ref() {
            let owner = self
                .owner_colors
                .get(&opponent)
                .copied()
                .filter(|color| color.r != 0 || color.g != 0 || color.b != 0)
                .unwrap_or_else(|| Color::opaque(0, 0, 0xff));
            let colored = clonk_frontend::hud::colorize_by_owner(crew, owner);
            draw_image_region_aspect(
                surface,
                &colored,
                Rect::new(0, 0, colored.width(), colored.height()),
                dest,
                false,
                gamma,
            );
        }
        if hostile {
            if let Some(menu) = self.menu.as_ref() {
                draw_image_region(
                    surface,
                    menu,
                    Rect::new(35 * 7, 0, 35, 35),
                    dest,
                    gamma,
                );
            }
        }
    }

    fn draw_team_symbol(
        &self,
        surface: &mut Surface,
        id: i32,
        color: u32,
        has_participants: bool,
        dest: Rect,
        gamma: Option<&GammaRamp>,
    ) {
        if let Some(image) = self.team_icons.get(&id) {
            draw_image_region_aspect(
                surface,
                image,
                Rect::new(0, 0, image.width(), image.height()),
                dest,
                false,
                gamma,
            );
            return;
        }
        if has_participants {
            if let Some(crew) = self.hud.crew.as_ref() {
                // C4Surface::SetClr maps zero to the default blue 0xff.
                let color = if color == 0 { 0xff } else { color };
                let owner = Color::opaque(
                    ((color >> 16) & 0xff) as u8,
                    ((color >> 8) & 0xff) as u8,
                    (color & 0xff) as u8,
                );
                let colored = clonk_frontend::hud::colorize_by_owner(crew, owner);
                draw_image_region_aspect(
                    surface,
                    &colored,
                    Rect::new(0, 0, colored.width(), colored.height()),
                    dest,
                    false,
                    gamma,
                );
            }
            return;
        }
        if let Some((image, src)) = self.symbol_source(&MenuSymbol::GuiIcon(ICO_TEAM)) {
            draw_image_region_aspect(surface, image, src, dest, false, gamma);
        }
    }
}

fn draw_menu(
    menu: &IngameMenuState,
    layout: &MenuLayout,
    surface: &mut Surface,
    area: Rect,
    font: &HudFont<'_>,
    tiny_font: Option<&HudFont<'_>>,
    gfx: &IngameMenuGraphics,
    gamma: Option<&GammaRamp>,
) {
    let bounds = layout.bounds;
    let (x0, y0) = (bounds.x, bounds.y);
    let (w, h) = (bounds.width as i32, bounds.height as i32);

    // Dialog background + 3D frame (C4GUI::Dialog::DrawElement,
    // C4GuiDialogs.cpp:537-550).
    fill_rect(
        surface,
        bounds,
        Color::new(0, 0, 0, STANDARD_BG_ALPHA),
        gamma,
    );
    draw_3d_frame(surface, bounds, gamma);

    // Wooden caption bar with the menu symbol and title
    // (WoodenLabel::DrawElement, C4GuiLabels.cpp:168-213).
    let title_rect = layout.title_rect();
    if let Some(caption) = gfx.caption_bar.as_ref() {
        draw_caption_bar(surface, title_rect, caption, gamma);
    }
    let mut icon_indent = 0;
    if let Some((image, src)) = gfx.symbol_source(&menu.symbol) {
        // icon square: (x+1, y+1, hgt-2, hgt-2) (C4GuiLabels.cpp:175-178)
        let side = (layout.title_height - 2) as u32;
        draw_image_region(
            surface,
            image,
            src,
            Rect::new(x0 + 1, y0 + 1, side, side),
            gamma,
        );
        // GetLeftIndent = bar height when an icon is set (C4Gui.h:560).
        icon_indent = layout.title_height;
    }
    // WoodenLabel reserves 20px for the close button and clips its caption
    // before Dialog draws Ico_Close (C4GuiDialogs.cpp:386-421).
    let close_rect = layout.close_button_rect();
    let text_right = if gfx.show_close_button {
        close_rect.x
    } else {
        title_rect.x + title_rect.width as i32
    };
    let previous_clip = surface.clip();
    let text_left = title_rect.x + icon_indent;
    let title_clip = Rect::new(
        text_left,
        title_rect.y,
        text_right.saturating_sub(text_left).max(0) as u32,
        title_rect.height,
    );
    let nested_clip = previous_clip
        .map(|clip| {
            clip.intersection(title_clip)
                .unwrap_or(Rect::new(0, 0, 0, 0))
        })
        .unwrap_or(title_clip);
    surface.set_clip(nested_clip);
    // ALeft x offset +5, vertically centered -1 (C4GuiLabels.cpp:183-212).
    font.draw_with_gamma(
        surface,
        x0 + icon_indent + 5,
        y0 + (layout.title_height - font.line_height()) / 2 - 1,
        menu.caption(),
        CAPTION_COLOR,
        TextAlign::Left,
        gamma,
    );
    match previous_clip {
        Some(clip) => surface.set_clip(clip),
        None => surface.clear_clip(),
    }
    if gfx.show_close_button {
        if let Some(gui_icons) = gfx.gui_icons.as_ref() {
            let source_x = i32::from(ICO_CLOSE % 6) * 40;
            let source_y = i32::from(ICO_CLOSE / 6) * 40;
            draw_image_region_aspect(
                surface,
                gui_icons,
                Rect::new(source_x, source_y, 40, 40),
                close_rect,
                false,
                gamma,
            );
        }
    }

    // Client area: items are translated by the ScrollWindow's logical-pixel
    // offset and clipped, including a partially visible first/last row.
    let client = layout.client_rect();
    let previous_clip = surface.clip();
    let client_clip = previous_clip
        .map(|clip| clip.intersection(client).unwrap_or(Rect::new(0, 0, 0, 0)))
        .unwrap_or(client);
    surface.set_clip(client_clip);
    let first_row = usize::try_from(layout.scroll_y / layout.item_height).unwrap_or_default();
    let visible_rows = layout.lines as usize
        + usize::from(layout.scroll_y % layout.item_height != 0);
    let columns = usize::try_from(layout.columns).unwrap_or(1);
    let first = first_row.saturating_mul(columns);
    let visible = visible_rows.saturating_mul(columns);
    for (index, item) in menu.items().iter().enumerate().skip(first).take(visible) {
        let Some(row_rect) = layout.item_rect(index) else {
            continue;
        };
        let item_y = row_rect.y;
        // Selection mark: filled red box (C4MenuItem::DrawElement,
        // C4Menu.cpp:152-154).
        if index == menu.selection() {
            fill_rect(surface, row_rect, SELECTION_COLOR, gamma);
        }
        // Symbol square at the left, width == item height (C4Menu.cpp:156-166).
        let symbol_rect = Rect::new(
            row_rect.x,
            item_y,
            layout.item_height as u32,
            row_rect.height,
        );
        match &item.symbol {
            MenuSymbol::Hostility { opponent, hostile } => {
                gfx.draw_hostility_symbol(surface, *opponent, *hostile, symbol_rect, gamma);
            }
            MenuSymbol::Definition { id, fulfilled } => {
                gfx.draw_definition_symbol(surface, id, *fulfilled, symbol_rect, gamma);
            }
            MenuSymbol::Team {
                id,
                icon_spec: _,
                color,
                has_participants,
            } => {
                gfx.draw_team_symbol(
                    surface,
                    *id,
                    *color,
                    *has_participants,
                    symbol_rect,
                    gamma,
                );
            }
            _ => {
                if let Some((image, src)) = gfx.symbol_source(&item.symbol) {
                    draw_image_region_aspect(
                        surface,
                        image,
                        src,
                        symbol_rect,
                        matches!(item.symbol, MenuSymbol::PlayerColor),
                        gamma,
                    );
                }
            }
        }
        // Normal menus are icon-only; their selected caption is installed in
        // the title. Context menus draw the caption alongside the symbol.
        if menu.style == MenuStyle::Context {
            font.draw_with_gamma(
                surface,
                row_rect.x + layout.item_height,
                item_y,
                &item.caption,
                MESSAGE_COLOR,
                TextAlign::Left,
                gamma,
            );
        }
    }
    match previous_clip {
        Some(clip) => surface.set_clip(clip),
        None => surface.clear_clip(),
    }

    // Bottom bar with the menu controls (C4Menu::DrawElement,
    // C4Menu.cpp:823-880).
    if gfx.show_commands {
        let extra = Rect::new(
            x0 + 1,
            y0 + h - MN_SYMBOL_SIZE - 1,
            (w - 2) as u32,
            MN_SYMBOL_SIZE as u32,
        );
        // divider frame in palette color 80 (#440000) (C4Menu.cpp:932-935);
        // CStdDDraw::DrawFrame never rasterizes the bottom-right corner
        // (capture: Drachenfels divider (1208,662) stays background).
        draw_hv_frame(surface, extra, EXTRA_FRAME_COLOR, gamma);
        let cell = extra.height;
        let mut cx = extra.x;
        let tiny = tiny_font.unwrap_or(font);
        // Enter: key cap with the Throw command + OK symbol
        // (C4Menu.cpp:857-864).
        draw_command_key(
            surface,
            gfx,
            tiny,
            cx,
            extra.y,
            cell,
            3,
            &gfx.throw_key,
            gamma,
        );
        cx += cell as i32;
        draw_ok_cancel(surface, gfx, cx, extra.y, cell, 0, 0, gamma);
        cx += cell as i32;
        // Close: key cap with the Dig command + cancel symbol
        // (C4Menu.cpp:874-880).
        draw_command_key(
            surface,
            gfx,
            tiny,
            cx,
            extra.y,
            cell,
            5,
            &gfx.dig_key,
            gamma,
        );
        cx += cell as i32;
        draw_ok_cancel(surface, gfx, cx, extra.y, cell, 1, 0, gamma);
    }

    // Tooltip with the info caption after the selection has rested
    // (C4Menu::Draw, C4Menu.cpp:804-821).
    if menu.time_on_selection >= INFO_CAPTION_DELAY {
        if let Some(info) = menu
            .items()
            .get(menu.selection())
            .and_then(|item| item.info_caption.as_deref())
        {
            if let Some(item) = layout.item_rect(menu.selection()) {
                draw_tooltip(surface, font, area, item.x, item.y, info, gamma);
            }
        }
    }
}

/// `DrawCommandKey` (C4ObjectCom.cpp:930-945): key cap (fctKey, Control.png
/// (0,100) 64x64) + command symbol (fctCommand, Control.png (0,36) 32x32
/// phases) + the key name in the small font when ShowCommandKeys is set.
pub fn draw_command_key(
    surface: &mut Surface,
    gfx: &IngameMenuGraphics,
    tiny_font: &HudFont<'_>,
    x: i32,
    y: i32,
    size: u32,
    control_index: i32,
    key_name: &str,
    gamma: Option<&GammaRamp>,
) {
    if let Some(control) = gfx.control.as_ref() {
        let dest = Rect::new(x, y, size, size);
        draw_image_region(surface, control, Rect::new(0, 100, 64, 64), dest, gamma);
        draw_image_region(
            surface,
            control,
            Rect::new(32 * control_index, 36, 32, 32),
            dest,
            gamma,
        );
    }
    if gfx.show_command_keys && !key_name.is_empty() {
        tiny_font.draw_with_gamma(
            surface,
            x + size as i32 / 2,
            y + size as i32 - tiny_font.line_height() - 2,
            key_name,
            MESSAGE_COLOR,
            TextAlign::Center,
            gamma,
        );
    }
}

/// `GfxR->fctOKCancel.Draw(cgo, true, px, py)` (C4Menu.cpp:860,880).
pub fn draw_ok_cancel(
    surface: &mut Surface,
    gfx: &IngameMenuGraphics,
    x: i32,
    y: i32,
    size: u32,
    px: i32,
    py: i32,
    gamma: Option<&GammaRamp>,
) {
    if let Some(control) = gfx.control.as_ref() {
        draw_image_region(
            surface,
            control,
            Rect::new(128 + 32 * px, 100 + 32 * py, 32, 32),
            Rect::new(x, y, size, size),
            gamma,
        );
    }
}

/// `C4GUI::Screen::DrawToolTip` (C4Gui.cpp:907-928).
pub fn draw_tooltip(
    surface: &mut Surface,
    font: &HudFont<'_>,
    facet: Rect,
    x: i32,
    y: i32,
    text: &str,
    gamma: Option<&GammaRamp>,
) {
    let broken = break_hud_message(font, text, tooltip_wrap_width(facet));
    let text_w = font.text_width_markup(&broken);
    let text_h = font.line_height() * tooltip_line_count(&broken) as i32;
    let w = text_w + 6;
    let h = text_h + 4;
    let (tx, ty) = tooltip_position(facet, x, y, w, h);
    fill_rect(
        surface,
        Rect::new(tx, ty, w as u32, (h - 1) as u32),
        TOOLTIP_BG_COLOR,
        gamma,
    );
    draw_rect_outline(
        surface,
        Rect::new(tx, ty, w as u32, h as u32),
        Color::new(0, 0, 0, TOOLTIP_FRAME_ALPHA),
        gamma,
    );
    font.draw_markup_with_gamma(
        surface,
        tx + 3,
        ty + 1,
        &broken,
        TOOLTIP_TEXT_COLOR,
        TextAlign::Left,
        gamma,
    );
}

pub fn tooltip_wrap_width(facet: Rect) -> i32 {
    MAX_TOOLTIP_WDT.min((facet.width as i32).max(50))
}

fn tooltip_line_count(text: &str) -> usize {
    text.split(['\n', '|']).count()
}

pub fn tooltip_position(facet: Rect, x: i32, y: i32, width: i32, height: i32) -> (i32, i32) {
    let bottom = facet.y.saturating_add(facet.height as i32);
    let tooltip_y = if y < facet.y.saturating_add(height).saturating_add(5) {
        (y + 5).min(bottom - height)
    } else {
        y - height - 5
    };
    let right = facet.x.saturating_add(facet.width as i32);
    let max_x = (right - width).max(facet.x);
    ((x - width / 2).clamp(facet.x, max_x), tooltip_y)
}

/// `C4GUI::Element::Draw3DFrame` (C4Gui.cpp:264-279) with the default border
/// colors at `C4GUI_BorderAlpha`.
pub fn draw_3d_frame(surface: &mut Surface, rect: Rect, gamma: Option<&GammaRamp>) {
    let x0 = rect.x;
    let y0 = rect.y;
    let x1 = rect.x + rect.width as i32 - 1;
    let y1 = rect.y + rect.height as i32 - 1;
    // DrawLineDw covers every pixel except the final one (GL diamond-exit).
    // Retained capture keeps the native line primitives so backends replay
    // DrawLineDw geometry; the end-exclusive strips below rasterize the same
    // pixel set with the same rounded blend, capture-verified 2026-07-21
    // (Drachenfels menu border: single-blend edges (38,11,1)/(17,6,1)).
    let capture = surface.is_gpu_scene_capture_active();
    let hline = |surface: &mut Surface, x_start: i32, x_end: i32, y: i32, color: Color| {
        if capture {
            draw_color_line(surface, x_start, y, x_end, y, color, gamma);
        } else {
            fill_rect(
                surface,
                Rect::new(x_start, y, (x_end - x_start).max(0) as u32, 1),
                color,
                gamma,
            );
        }
    };
    let vline = |surface: &mut Surface, x: i32, y_start: i32, y_end: i32, color: Color| {
        if capture {
            draw_color_line(surface, x, y_start, x, y_end, color, gamma);
        } else {
            fill_rect(
                surface,
                Rect::new(x, y_start, 1, (y_end - y_start).max(0) as u32),
                color,
                gamma,
            );
        }
    };
    hline(surface, x0, x1, y0, BORDER_COLOR_1);
    vline(surface, x0, y0, y1, BORDER_COLOR_1);
    hline(surface, x0 + 1, x1 - 1, y0 + 1, BORDER_COLOR_2);
    vline(surface, x0 + 1, y0 + 1, y1 - 1, BORDER_COLOR_2);
    hline(surface, x0, x1, y1, BORDER_COLOR_3);
    vline(surface, x1, y0, y1, BORDER_COLOR_3);
    hline(surface, x0 + 1, x1 - 1, y1 - 1, BORDER_COLOR_1);
    vline(surface, x1 - 1, y0 + 1, y1 - 1, BORDER_COLOR_1);
}

/// The zoomed branch of `C4GUI::Element::DrawBar` (C4Gui.cpp:313-329) for
/// `GetRes()->barCaption`: GUICaption.png sliced 32/128/32 horizontally.
pub fn draw_caption_bar(
    surface: &mut Surface,
    rect: Rect,
    image: &ImageData,
    gamma: Option<&GammaRamp>,
) {
    let img_h = image.height() as i32;
    if img_h <= 0 || rect.height == 0 {
        return;
    }
    let zoom = rect.height as f32 / img_h as f32;
    let begin_w = (zoom * 32.0) as i32;
    let mid_w = (zoom * 128.0) as i32;
    let right_show = (zoom * (32 / 3) as f32) as i32;
    let w = rect.width as i32;
    draw_image_region(
        surface,
        image,
        Rect::new(0, 0, 32, img_h as u32),
        Rect::new(rect.x, rect.y, begin_w as u32, rect.height),
        gamma,
    );
    let mut ix = begin_w;
    while ix < w - right_show {
        let w2 = mid_w.min(w - right_show - ix);
        let src_w = ((w2 as f32) / zoom) as i32;
        if w2 <= 0 || src_w <= 0 {
            break;
        }
        draw_image_region(
            surface,
            image,
            Rect::new(32, 0, src_w as u32, img_h as u32),
            Rect::new(rect.x + ix, rect.y, w2 as u32, rect.height),
            gamma,
        );
        ix += mid_w;
    }
    draw_image_region(
        surface,
        image,
        Rect::new(160, 0, 32, img_h as u32),
        Rect::new(rect.x + w - begin_w, rect.y, begin_w as u32, rect.height),
        gamma,
    );
}

/// Nearest-neighbour stretch blit of `src` (source-pixel rect) into `dest`
/// with alpha blending — the software analogue of `C4Facet::DrawX`.
pub fn draw_image_region(
    surface: &mut Surface,
    image: &ImageData,
    src: Rect,
    dest: Rect,
    gamma: Option<&GammaRamp>,
) {
    clonk_frontend::classic_gui::draw_facet_nearest(surface, image, src, dest, gamma);
}

/// `C4Facet::Draw` with `fAspect=true`: scale to fit, keep the aspect ratio
/// and center in the target. `colorize` applies the default blue player
/// color like `fctPlayerClr.Surface->SetClr(0xff)` (C4MainMenu.cpp:69-70).
pub fn draw_image_region_aspect(
    surface: &mut Surface,
    image: &ImageData,
    src: Rect,
    dest: Rect,
    colorize: bool,
    gamma: Option<&GammaRamp>,
) {
    if src.width == 0 || src.height == 0 {
        return;
    }
    let scale = (dest.width as f32 / src.width as f32)
        .min(dest.height as f32 / src.height as f32);
    let w = ((src.width as f32 * scale) as u32).max(1);
    let h = ((src.height as f32 * scale) as u32).max(1);
    let fitted = Rect::new(
        dest.x + (dest.width as i32 - w as i32) / 2,
        dest.y + (dest.height as i32 - h as i32) / 2,
        w,
        h,
    );
    if colorize {
        let colored = clonk_frontend::hud::colorize_by_owner(image, Color::opaque(0, 0, 0xff));
        draw_image_region(surface, &colored, src, fitted, gamma);
    } else {
        draw_image_region(surface, image, src, fitted, gamma);
    }
}

fn fill_rect(surface: &mut Surface, rect: Rect, color: Color, gamma: Option<&GammaRamp>) {
    clonk_frontend::draw_color_rect(surface, rect, color, gamma);
}

#[allow(clippy::too_many_arguments)]
fn draw_color_line(
    surface: &mut Surface,
    x1: i32,
    y1: i32,
    x2: i32,
    y2: i32,
    color: Color,
    gamma: Option<&GammaRamp>,
) {
    let packed = (u32::from(255 - color.a) << 24)
        | (u32::from(color.r) << 16)
        | (u32::from(color.g) << 8)
        | u32::from(color.b);
    clonk_frontend::classic_gui::draw_engine_line(surface, x1, y1, x2, y2, packed, gamma);
}

fn pack_engine_color(color: Color) -> u32 {
    (u32::from(255 - color.a) << 24)
        | (u32::from(color.r) << 16)
        | (u32::from(color.g) << 8)
        | u32::from(color.b)
}

/// `CStdDDraw::DrawFrameDw` (StdDDraw2.cpp:1181-1187): the directed line loop
/// covers every corner exactly once. Verified against a real C++ GL capture
/// (Drachenfels tooltip frame, 2026-07-21): all frame pixels over the opaque
/// fill read one blend — the former full-length strips double-blended the
/// corners.
fn draw_rect_outline(surface: &mut Surface, rect: Rect, color: Color, gamma: Option<&GammaRamp>) {
    if rect.width == 0 || rect.height == 0 {
        return;
    }
    clonk_frontend::classic_gui::draw_engine_frame(
        surface,
        rect.x,
        rect.y,
        rect.x + rect.width as i32 - 1,
        rect.y + rect.height as i32 - 1,
        pack_engine_color(color),
        gamma,
    );
}

/// `CStdDDraw::DrawFrame` (StdDDraw2.cpp:1173-1179) as reached from
/// `C4Menu::DrawFrame` (C4Menu.cpp:932-935): two horizontals plus two
/// verticals whose shared excluded endpoint leaves the bottom-right corner
/// unpainted on render targets.
fn draw_hv_frame(surface: &mut Surface, rect: Rect, color: Color, gamma: Option<&GammaRamp>) {
    if rect.width == 0 || rect.height == 0 {
        return;
    }
    clonk_frontend::classic_gui::draw_engine_frame_hv(
        surface,
        rect.x,
        rect.y,
        rect.x + rect.width as i32 - 1,
        rect.y + rect.height as i32 - 1,
        pack_engine_color(color),
        gamma,
    );
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn retained_ingame_frames_keep_draw_line_alpha_provenance() {
        let mut surface = Surface::new(8, 8, clonk_graphics::PixelFormat::Rgba8888);
        surface.begin_gpu_scene_capture();
        draw_3d_frame(&mut surface, Rect::new(1, 1, 6, 6), None);

        let scene = surface
            .take_gpu_scene_capture()
            .expect("capture remains active")
            .into_scene([8, 8], Color::transparent(), &GammaRamp::identity());
        assert!(!scene.commands.is_empty());
        for command in &scene.commands {
            let clonk_graphics::GpuCommand::Solid {
                topology,
                alpha_mode,
                ..
            } = command
            else {
                panic!("3D frame did not remain solid painter commands");
            };
            assert_eq!(*topology, clonk_graphics::GpuPrimitiveTopology::LineList);
            assert_eq!(
                *alpha_mode,
                clonk_graphics::GpuSolidAlphaMode::SourceOver,
                "C4GUI frame rectangles retain DrawLineDw alpha provenance"
            );
        }
    }

    // C++ GL capture oracle (M06-P3-L034, 2026-07-21, Drachenfels choice-menu
    // tooltip at (942,580) 182x26 in Screenshot001.png): every frame pixel
    // over the opaque #F1EA78 fill — corners (942,580)/(1123,580) included —
    // reads (121,117,60): exactly one DrawLineDw blend of 0x7f000000 with GL
    // round-to-nearest and the gamma shader's black floor (0 -> 1). The
    // former full-length strips double-blended the corners to (61,59,30).
    #[test]
    fn m06_l034_tooltip_frame_corners_blend_once_per_cpp_capture() {
        let gamma = GammaRamp::from_control_points([0x000000, 0x808080, 0xffffff]);
        let mut surface = Surface::new(16, 12, clonk_graphics::PixelFormat::Rgba8888);
        surface.fill(Color::opaque(241, 234, 120));
        draw_rect_outline(
            &mut surface,
            Rect::new(2, 2, 12, 8),
            Color::new(0, 0, 0, TOOLTIP_FRAME_ALPHA),
            Some(&gamma),
        );
        let frame = Some(Color::opaque(121, 117, 60));
        for (x, y) in [(2, 2), (13, 2), (2, 9), (13, 9)] {
            assert_eq!(
                surface.get_pixel(x, y),
                frame,
                "corner ({x},{y}) must blend once like the C++ capture"
            );
        }
        for x in 3..13 {
            assert_eq!(surface.get_pixel(x, 2), frame);
            assert_eq!(surface.get_pixel(x, 9), frame);
        }
        for y in 3..9 {
            assert_eq!(surface.get_pixel(2, y), frame);
            assert_eq!(surface.get_pixel(13, y), frame);
        }
        assert_eq!(surface.get_pixel(3, 3), Some(Color::opaque(241, 234, 120)));
    }

    // C++ GL capture oracle (M06-P3-L034, 2026-07-21, Drachenfels extra-bar
    // divider (1032,647)-(1208,662) in Screenshot001.png): palette color 80
    // renders (68,1,1) — black-floored — on the top and left edges and the
    // three reachable corners, while the bottom-right corner (1208,662)
    // stays at the (1,1,1) background because both `CStdDDraw::DrawFrame`
    // lines exclude it as their endpoint.
    #[test]
    fn m06_l034_extra_divider_skips_bottom_right_corner_per_cpp_capture() {
        let gamma = GammaRamp::from_control_points([0x000000, 0x808080, 0xffffff]);
        let mut surface = Surface::new(16, 12, clonk_graphics::PixelFormat::Rgba8888);
        surface.fill(Color::opaque(1, 1, 1));
        draw_hv_frame(
            &mut surface,
            Rect::new(2, 2, 12, 8),
            EXTRA_FRAME_COLOR,
            Some(&gamma),
        );
        let divider = Some(Color::opaque(68, 1, 1));
        assert_eq!(surface.get_pixel(2, 2), divider);
        assert_eq!(surface.get_pixel(13, 2), divider);
        assert_eq!(surface.get_pixel(2, 9), divider);
        assert_eq!(
            surface.get_pixel(13, 9),
            Some(Color::opaque(1, 1, 1)),
            "CStdDDraw::DrawFrame never rasterizes the shared bottom-right endpoint"
        );
        for x in 3..13 {
            assert_eq!(surface.get_pixel(x, 2), divider);
            assert_eq!(surface.get_pixel(x, 9), divider);
        }
        for y in 3..9 {
            assert_eq!(surface.get_pixel(2, y), divider);
            assert_eq!(surface.get_pixel(13, y), divider);
        }
    }

    fn captions(menu: &IngameMenuState) -> Vec<&str> {
        menu.items()
            .iter()
            .map(|item| item.caption.as_str())
            .collect()
    }

    // Initial team selection preserves `C4TeamList` order and dispatches the
    // selected team ID through `TeamSel:<id>`; the menu remains non-permanent
    // and has no main-menu close command (C4Player.cpp:1762-1771;
    // C4MainMenu.cpp:175-232, 899-908).
    #[test]
    fn initial_team_selection_matches_cpp_entries_and_close_semantics() {
        let teams = vec![
            TeamSelectionEntry {
                id: 7,
                caption: "Blue Team (Clonko)".to_string(),
                icon_spec: None,
                color: 0x0000_00ff,
                has_participants: true,
            },
            TeamSelectionEntry {
                id: 3,
                caption: "Red Team".to_string(),
                icon_spec: None,
                color: 0x00ff_0000,
                has_participants: false,
            },
        ];
        let mut menu = IngameMenuState::team_selection_menu(&teams);

        assert_eq!(menu.page(), MenuPage::TeamSelection);
        assert_eq!(menu.caption(), "Select team");
        assert_eq!(captions(&menu), vec!["Blue Team (Clonko)", "Red Team"]);
        assert_eq!(menu.items()[0].action, MenuAction::SelectTeam(7));
        assert_eq!(menu.items()[1].action, MenuAction::SelectTeam(3));
        assert_eq!(
            menu.items()[0].info_caption.as_deref(),
            Some("Join team Blue Team (Clonko)")
        );
        assert!(!menu.is_permanent());
        assert!(menu.close_action().is_none());

        menu.set_selection(1);
        let outcome = menu
            .handle_command(ControlCommand::MenuEnter, CommandKind::Press)
            .expect("team selection outcome");
        assert!(matches!(
            outcome,
            MenuOutcome::Action {
                action: MenuAction::SelectTeam(3),
                close_menu: true
            }
        ));
    }

    #[test]
    fn l135_team_selection_menu_uses_declared_icon_spec_and_cpp_fallbacks() {
        let teams = vec![
            TeamSelectionEntry {
                id: 1,
                caption: "Resolved".to_string(),
                icon_spec: Some("ICON".to_string()),
                color: 0x0011_2233,
                has_participants: true,
            },
            TeamSelectionEntry {
                id: 2,
                caption: "Unresolved".to_string(),
                icon_spec: Some("MISS".to_string()),
                color: 0x0024_68ac,
                has_participants: true,
            },
            TeamSelectionEntry {
                id: 3,
                caption: "Crew".to_string(),
                icon_spec: None,
                color: 0x0012_3456,
                has_participants: true,
            },
            TeamSelectionEntry {
                id: 4,
                caption: "Empty".to_string(),
                icon_spec: None,
                color: 0,
                has_participants: false,
            },
        ];
        let menu = IngameMenuState::team_selection_menu(&teams);
        let declared = Color::opaque(0xe1, 0x22, 0x33);
        let generic = Color::opaque(0xf0, 0xd1, 0x12);
        let gfx = IngameMenuGraphics {
            hud: HudGraphics {
                crew: Some(ImageData::new(1, 1, vec![0, 0, 0xff, 0xff])),
                ..HudGraphics::default()
            },
            gui_icons: Some(ImageData::new(
                40 * 6,
                40 * 4,
                [generic.r, generic.g, generic.b, generic.a].repeat(40 * 6 * 40 * 4),
            )),
            team_icons: HashMap::from([(
                1,
                ImageData::new(1, 1, vec![declared.r, declared.g, declared.b, declared.a]),
            )]),
            ..IngameMenuGraphics::default()
        };
        let draw = |symbol: &MenuSymbol| {
            let MenuSymbol::Team {
                id,
                color,
                has_participants,
                ..
            } = symbol
            else {
                panic!("team row must retain a semantic team symbol");
            };
            let mut surface = Surface::new(1, 1, clonk_graphics::PixelFormat::Rgba8888);
            gfx.draw_team_symbol(
                &mut surface,
                *id,
                *color,
                *has_participants,
                Rect::new(0, 0, 1, 1),
                None,
            );
            surface.get_pixel(0, 0)
        };

        assert_eq!(draw(&menu.items()[0].symbol), Some(declared));
        assert_eq!(
            draw(&menu.items()[1].symbol),
            Some(Color::opaque(0x24, 0x68, 0xac)),
            "an unresolved IconSpec must fall through to the occupied-team crew"
        );
        assert_eq!(
            draw(&menu.items()[2].symbol),
            Some(Color::opaque(0x12, 0x34, 0x56))
        );
        assert_eq!(draw(&menu.items()[3].symbol), Some(generic));
    }

    // C4MainMenu::ActivateMain for a local fullscreen single-player round
    // (C4MainMenu.cpp:643-715).
    #[test]
    fn main_menu_local_single_player_matches_cpp_entry_list() {
        let menu = IngameMenuState::main_menu(&MainMenuConditions::default()).expect("menu");
        assert_eq!(menu.caption(), "Player Menu"); // IDS_MENU_CPMAIN
        assert_eq!(
            captions(&menu),
            vec![
                "Goals",
                "Rules",
                "Join player",
                "Save game",
                "Options",
                "Surrender",
                "Abort round",
            ]
        );
        assert_eq!(menu.selection(), 0);
        assert!(!menu.is_permanent());
        assert!(menu.close_action().is_none());
    }

    #[test]
    fn main_menu_actions_mirror_cpp_commands() {
        let menu = IngameMenuState::main_menu(&MainMenuConditions::default()).expect("menu");
        let actions: Vec<&MenuAction> = menu.items().iter().map(|item| &item.action).collect();
        assert_eq!(
            actions,
            vec![
                &MenuAction::ActivateGoals,
                &MenuAction::ActivateRules,
                &MenuAction::ActivateNewPlayer,
                &MenuAction::ActivateSavegame,
                &MenuAction::ActivateOptions,
                &MenuAction::ActivateSurrender,
                &MenuAction::Abort,
            ]
        );
    }

    // Observer menu: no player => View entry, no Goals/Rules/Save/Surrender
    // (C4MainMenu.cpp:653, 666-670).
    #[test]
    fn main_menu_observer_shows_view_and_hides_player_entries() {
        let cond = MainMenuConditions {
            has_player: false,
            player_count: 0,
            ..MainMenuConditions::default()
        };
        let menu = IngameMenuState::main_menu(&cond).expect("menu");
        assert_eq!(menu.caption(), "Observer Menu"); // IDS_MENU_OBSERVER
        assert_eq!(
            captions(&menu),
            vec!["View", "Join player", "Options", "Abort round"]
        );
    }

    // Hostility entry appears for >1 players (C4MainMenu.cpp:672).
    #[test]
    fn main_menu_multiplayer_adds_attack_entry() {
        let cond = MainMenuConditions {
            player_count: 2,
            ..MainMenuConditions::default()
        };
        let menu = IngameMenuState::main_menu(&cond).expect("menu");
        assert_eq!(captions(&menu)[2], "Attack");
    }

    #[test]
    fn hostility_menu_matches_directional_cpp_rows_and_refill() {
        let mut menu = IngameMenuState::hostility_menu(&[
            HostilityEntry {
                opponent: 7,
                name: "Ada".to_string(),
                hostile: false,
                opponent_hostile: true,
            },
            HostilityEntry {
                opponent: 9,
                name: "Bob".to_string(),
                hostile: true,
                opponent_hostile: false,
            },
        ]);

        assert_eq!(menu.page(), MenuPage::Hostility);
        assert_eq!(menu.caption(), "Don't attack Ada");
        assert_eq!(captions(&menu), vec!["Don't attack Ada", "Attack Bob"]);
        assert_eq!(menu.items()[0].action, MenuAction::ToggleHostility(7));
        assert_eq!(
            menu.items()[0].info_caption.as_deref(),
            Some("Ada is currently hostile and will not be attacked.")
        );
        assert_eq!(
            menu.items()[1].info_caption.as_deref(),
            Some("Bob is currently friendly and will be attacked.")
        );
        assert!(matches!(
            menu.items()[1].symbol,
            MenuSymbol::Hostility {
                opponent: 9,
                hostile: true
            }
        ));
        assert!(menu.is_permanent());
        assert_eq!(menu.close_action(), Some(&MenuAction::ActivateMain));

        menu.set_selection(1);
        assert_eq!(menu.caption(), "Attack Bob");
        menu.refill_hostility(&[HostilityEntry {
            opponent: 7,
            name: "Ada".to_string(),
            hostile: true,
            opponent_hostile: true,
        }]);
        assert_eq!(captions(&menu), vec!["Attack Ada"]);
        assert_eq!(menu.caption(), "Attack Ada");
        assert_eq!(menu.selection(), 0);

        let entries = (0..7)
            .map(|opponent| HostilityEntry {
                opponent,
                name: opponent.to_string(),
                hostile: false,
                opponent_hostile: false,
            })
            .collect::<Vec<_>>();
        let mut grid = IngameMenuState::hostility_menu(&entries);
        grid.handle_command(ControlCommand::MenuUp, CommandKind::Press);
        assert_eq!(grid.selection(), 5, "up wraps to the last row in column zero");
        grid.handle_command(ControlCommand::MenuDown, CommandKind::Press);
        assert_eq!(grid.selection(), 0);
        grid.set_selection(2);
        grid.handle_command(ControlCommand::MenuUp, CommandKind::Press);
        assert_eq!(grid.selection(), 2, "an incomplete column has no second row");
    }

    #[test]
    fn hostility_symbol_renderer_matches_cpp_portrait_color_and_overlay_layers() {
        let opponent = 7;
        let portrait = Color::opaque(0xe0, 0x10, 0x20);
        let owner = Color::opaque(0x10, 0xc0, 0x20);
        let attack = Color::opaque(0xf0, 0xd0, 0x10);
        let mut gfx = IngameMenuGraphics {
            hud: HudGraphics {
                crew: Some(ImageData::new(1, 1, vec![0, 0, 0xff, 0xff])),
                ..HudGraphics::default()
            },
            owner_colors: HashMap::from([(opponent, owner)]),
            hostility_big_icons: HashMap::from([(
                opponent,
                ImageData::new(1, 1, vec![portrait.r, portrait.g, portrait.b, portrait.a]),
            )]),
            menu: Some(ImageData::new(
                35 * 8,
                35,
                [attack.r, attack.g, attack.b, attack.a].repeat(35 * 8 * 35),
            )),
            show_portraits: true,
            ..IngameMenuGraphics::default()
        };
        let draw = |gfx: &IngameMenuGraphics, hostile| {
            let mut surface = Surface::new(1, 1, clonk_graphics::PixelFormat::Rgba8888);
            gfx.draw_hostility_symbol(
                &mut surface,
                opponent,
                hostile,
                Rect::new(0, 0, 1, 1),
                None,
            );
            surface.get_pixel(0, 0)
        };

        assert_eq!(draw(&gfx, false), Some(portrait));
        gfx.show_portraits = false;
        assert_eq!(draw(&gfx, false), Some(owner));
        assert_eq!(draw(&gfx, true), Some(attack));
    }

    #[test]
    fn hostility_normal_grid_keeps_initialized_height_until_growth() {
        use clonk_graphics::BitmapFont;

        let entries = |count| {
            (0..count)
                .map(|opponent| HostilityEntry {
                    opponent,
                    name: opponent.to_string(),
                    hostile: false,
                    opponent_hostile: false,
                })
                .collect::<Vec<_>>()
        };
        let font_backend = BitmapFont::new();
        let font = HudFont::Fallback(&font_backend);
        let gfx = IngameMenuGraphics::default();
        let area = Rect::new(0, 0, 640, 480);
        let mut menu = IngameMenuState::hostility_menu(&entries(7));

        assert_eq!(menu.layout(area, &font, &gfx).lines, 2);
        menu.refill_hostility(&entries(1));
        assert_eq!(
            menu.layout(area, &font, &gfx).lines,
            2,
            "normal refill shrink retains C4Menu's initialized client height"
        );
        menu.refill_hostility(&entries(11));
        assert_eq!(
            menu.layout(area, &font, &gfx).lines,
            3,
            "growth invalidates LocationSet and recomputes the normal grid"
        );
    }

    #[test]
    fn team_switch_page_uses_switch_actions_and_returns_to_main() {
        let cond = MainMenuConditions {
            team_switch_allowed: true,
            ..MainMenuConditions::default()
        };
        let menu = IngameMenuState::main_menu(&cond).expect("menu");
        let team = menu
            .items()
            .iter()
            .find(|item| item.caption == "Select team")
            .expect("team entry");
        assert_eq!(team.action, MenuAction::ActivateTeamSelection);

        let teams = vec![
            TeamSelectionEntry {
                id: 7,
                caption: "Blue Team".to_string(),
                icon_spec: None,
                color: 0x0000_00ff,
                has_participants: false,
            },
            TeamSelectionEntry {
                id: -1,
                caption: "New Team".to_string(),
                icon_spec: None,
                color: 0,
                has_participants: false,
            },
        ];
        let initial_from_main = IngameMenuState::team_selection_menu_from_main(&teams);
        assert_eq!(
            initial_from_main.items()[0].action,
            MenuAction::SelectTeam(7)
        );
        assert_eq!(
            initial_from_main.close_action(),
            Some(&MenuAction::ActivateMain)
        );

        let mut menu = IngameMenuState::team_switch_menu(&teams);
        assert_eq!(menu.page(), MenuPage::TeamSelection);
        assert!(!menu.is_permanent());
        assert_eq!(menu.close_action(), Some(&MenuAction::ActivateMain));
        assert_eq!(menu.items()[0].action, MenuAction::SwitchTeam(7));
        assert_eq!(menu.items()[1].action, MenuAction::SwitchTeam(-1));

        menu.set_selection(1);
        assert!(matches!(
            menu.handle_command(ControlCommand::MenuEnter, CommandKind::Press),
            Some(MenuOutcome::Action {
                action: MenuAction::SwitchTeam(-1),
                close_menu: true,
            })
        ));
    }

    // Network client: no Save game (not host), Disconnect entry
    // (C4MainMenu.cpp:689, 702-703).
    #[test]
    fn main_menu_network_client_hides_save_and_offers_disconnect() {
        let cond = MainMenuConditions {
            network_enabled: true,
            network_host: false,
            player_count: 2,
            ..MainMenuConditions::default()
        };
        let menu = IngameMenuState::main_menu(&cond).expect("menu");
        let names = captions(&menu);
        assert!(!names.contains(&"Save game"));
        assert!(names.contains(&"Disconnect"));
    }

    // League games hide the player-join entry (C4MainMenu.cpp:684).
    #[test]
    fn main_menu_league_hides_join_player() {
        let cond = MainMenuConditions {
            is_league: true,
            ..MainMenuConditions::default()
        };
        let menu = IngameMenuState::main_menu(&cond).expect("menu");
        assert!(!captions(&menu).contains(&"Join player"));
    }

    // C4Menu::Control wrap-around navigation (C4Menu.cpp:439-461).
    #[test]
    fn navigation_wraps_in_both_directions() {
        let mut menu = IngameMenuState::main_menu(&MainMenuConditions::default()).expect("menu");
        let count = menu.items().len();
        menu.handle_command(ControlCommand::MenuUp, CommandKind::Press);
        assert_eq!(menu.selection(), count - 1);
        menu.handle_command(ControlCommand::MenuDown, CommandKind::Press);
        assert_eq!(menu.selection(), 0);
        menu.handle_command(ControlCommand::MenuRight, CommandKind::Press);
        assert_eq!(menu.selection(), 1);
        menu.handle_command(ControlCommand::MenuLeft, CommandKind::Press);
        assert_eq!(menu.selection(), 0);
    }

    #[test]
    fn hostility_refill_resets_tooltip_age_when_first_row_appears() {
        let mut menu = IngameMenuState::hostility_menu(&[]);
        for _ in 0..INFO_CAPTION_DELAY {
            menu.tick();
        }
        assert_eq!(menu.time_on_selection, INFO_CAPTION_DELAY);

        menu.refill_hostility(&[HostilityEntry {
            opponent: 7,
            name: "Ada".to_string(),
            hostile: true,
            opponent_hostile: false,
        }]);

        assert_eq!(menu.selection(), 0);
        assert_eq!(menu.time_on_selection, 0);
    }

    // C4Menu::Enter on a non-permanent menu closes it before the command
    // runs (C4Menu.cpp:512-518).
    #[test]
    fn enter_on_main_menu_closes_and_returns_action() {
        let mut menu = IngameMenuState::main_menu(&MainMenuConditions::default()).expect("menu");
        let outcome = menu
            .handle_command(ControlCommand::MenuEnter, CommandKind::Press)
            .expect("outcome");
        match outcome {
            MenuOutcome::Action { action, close_menu } => {
                assert_eq!(action, MenuAction::ActivateGoals);
                assert!(close_menu);
            }
            other => panic!("expected action, got {other:?}"),
        }
    }

    // Key releases do not activate items (menu coms act on press).
    #[test]
    fn release_events_are_ignored() {
        let mut menu = IngameMenuState::main_menu(&MainMenuConditions::default()).expect("menu");
        assert!(menu
            .handle_command(ControlCommand::MenuEnter, CommandKind::Release)
            .is_none());
        menu.handle_command(ControlCommand::MenuDown, CommandKind::Release);
        assert_eq!(menu.selection(), 0);
    }

    // COM_MenuClose on the main menu closes without a follow-up command
    // (close command unset, C4MainMenu.cpp:643-715).
    #[test]
    fn close_on_main_menu_has_no_close_action() {
        let mut menu = IngameMenuState::main_menu(&MainMenuConditions::default()).expect("menu");
        let outcome = menu
            .handle_command(ControlCommand::MenuClose, CommandKind::Press)
            .expect("outcome");
        match outcome {
            MenuOutcome::Closed { close_action } => assert!(close_action.is_none()),
            other => panic!("expected close, got {other:?}"),
        }
    }

    // Submenus set "ActivateMenu:Main" as close command
    // (C4MainMenu.cpp:496-497, 577).
    #[test]
    fn close_on_submenu_returns_to_main() {
        let flags = OptionFlags {
            sound: true,
            music: true,
            mouse_shown: true,
            mouse: true,
        };
        let mut menu = IngameMenuState::options_menu(&flags, 0);
        let outcome = menu
            .handle_command(ControlCommand::MenuClose, CommandKind::Press)
            .expect("outcome");
        match outcome {
            MenuOutcome::Closed { close_action } => {
                assert_eq!(close_action, Some(MenuAction::ActivateMain));
            }
            other => panic!("expected close, got {other:?}"),
        }
    }

    // ActivateOptions: Sound, Music, Mouse control, Display with the
    // on/off icon phases (C4MainMenu.cpp:553-580).
    #[test]
    fn options_menu_lists_sound_music_mouse_display() {
        let flags = OptionFlags {
            sound: true,
            music: false,
            mouse_shown: true,
            mouse: true,
        };
        let menu = IngameMenuState::options_menu(&flags, 2);
        assert_eq!(
            captions(&menu),
            vec!["Sound", "Music", "Mouse control", "Display"]
        );
        assert!(menu.is_permanent());
        assert_eq!(menu.selection(), 2);
        assert!(matches!(menu.items()[0].symbol, MenuSymbol::Options(18)));
        assert!(matches!(menu.items()[1].symbol, MenuSymbol::Options(1)));
        assert!(matches!(menu.items()[2].symbol, MenuSymbol::Options(12)));
        assert!(matches!(menu.items()[3].symbol, MenuSymbol::Menu(8)));
    }

    // Enter on a permanent menu keeps it open (C4Menu.cpp:512-513).
    #[test]
    fn enter_on_permanent_menu_does_not_close() {
        let flags = OptionFlags {
            sound: true,
            music: true,
            mouse_shown: false,
            mouse: false,
        };
        let mut menu = IngameMenuState::options_menu(&flags, 0);
        let outcome = menu
            .handle_command(ControlCommand::MenuEnter, CommandKind::Press)
            .expect("outcome");
        match outcome {
            MenuOutcome::Action { action, close_menu } => {
                assert_eq!(action, MenuAction::ToggleSound);
                assert!(!close_menu);
            }
            other => panic!("expected action, got {other:?}"),
        }
    }

    // ActivateDisplay fullscreen entry list (C4MainMenu.cpp:582-641).
    #[test]
    fn display_menu_fullscreen_lists_all_entries() {
        let menu = IngameMenuState::display_menu(&DisplayFlags::default(), 0);
        assert_eq!(
            captions(&menu),
            vec![
                "Player names",
                "Clonk names",
                "Portraits",
                "Commands",
                "Keys",
                "Title board: Normal",
                "FPS Display",
                "Clock",
                "White Chat",
            ]
        );
        assert_eq!(
            menu.close_action(),
            Some(&MenuAction::ActivateOptions)
        );
    }

    #[test]
    fn display_menu_windowed_hides_fullscreen_only_entries() {
        let flags = DisplayFlags {
            is_fullscreen: false,
            ..DisplayFlags::default()
        };
        let menu = IngameMenuState::display_menu(&flags, 0);
        assert_eq!(
            captions(&menu),
            vec!["Player names", "Clonk names", "Portraits", "Commands", "Keys"]
        );
    }

    // Upper board cycles Off -> Normal -> Small -> Minimal
    // (C4MainMenu.cpp:858-864; C4Config.cpp:455-460).
    #[test]
    fn upper_board_mode_cycles() {
        assert_eq!(UpperBoardMode::Hide.next(), UpperBoardMode::Full);
        assert_eq!(UpperBoardMode::Mini.next(), UpperBoardMode::Hide);
    }

    // ActivateSavegame: ten "Save game" slots with free/used icons
    // (C4MainMenu.cpp:483-494).
    #[test]
    fn savegame_menu_lists_ten_slots() {
        let mut slots = [SaveSlotState { free: true }; 10];
        slots[0].free = false;
        let menu = IngameMenuState::savegame_menu(&slots);
        assert_eq!(menu.items().len(), 10);
        assert!(menu.is_permanent());
        assert!(menu
            .items()
            .iter()
            .all(|item| item.caption == "Save game"));
        assert!(matches!(
            menu.items()[0].symbol,
            MenuSymbol::SaveSlot { slot: 1, free: false }
        ));
        assert!(matches!(
            menu.items()[9].symbol,
            MenuSymbol::SaveSlot { slot: 10, free: true }
        ));
        assert_eq!(menu.items()[4].action, MenuAction::SaveSlot(5));
    }

    // ActivateSurrender: Yes/No with the OKCancel symbols
    // (C4MainMenu.cpp:538-551).
    #[test]
    fn surrender_menu_is_yes_no() {
        let menu = IngameMenuState::surrender_menu();
        assert_eq!(captions(&menu), vec!["Yes", "No"]);
        assert_eq!(menu.items()[0].action, MenuAction::Surrender);
        assert_eq!(menu.items()[1].action, MenuAction::NoOp);
        assert!(!menu.is_permanent());
    }

    #[test]
    fn goals_menu_uses_definition_description_as_tooltip() {
        let goals = vec![GoalRuleEntry {
            definition_id: "GOLD".to_string(),
            name: "Gold Rush".to_string(),
            description: Some("Collect all the gold.".to_string()),
            fulfilled: false,
        }];
        let menu = IngameMenuState::goals_menu(&goals);
        assert_eq!(captions(&menu), vec!["Gold Rush"]);
        assert_eq!(
            menu.items()[0].info_caption.as_deref(),
            Some("Collect all the gold.")
        );
        assert!(matches!(
            &menu.items()[0].symbol,
            MenuSymbol::Definition {
                id,
                fulfilled: false,
            } if id == "GOLD"
        ));
        assert_eq!(
            menu.items()[0].action,
            MenuAction::GoalInfo("GOLD".to_string())
        );

        let rules = vec![GoalRuleEntry {
            definition_id: "RULE".to_string(),
            name: "Rule".to_string(),
            description: Some("Keep to the rule.".to_string()),
            fulfilled: false,
        }];
        let menu = IngameMenuState::rules_menu(&rules);
        assert_eq!(
            menu.items()[0].info_caption.as_deref(),
            Some("Keep to the rule.")
        );
    }

    #[test]
    fn goals_menu_marks_fulfilled_goal() {
        use clonk_graphics::BitmapFont;

        let goals = vec![GoalRuleEntry {
            definition_id: "GOLD".to_string(),
            name: "Gold Rush".to_string(),
            description: None,
            fulfilled: true,
        }];
        let menu = IngameMenuState::goals_menu(&goals);
        assert!(matches!(
            &menu.items()[0].symbol,
            MenuSymbol::Definition {
                id,
                fulfilled: true,
            } if id == "GOLD"
        ));

        let definition = Color::opaque(200, 30, 20);
        let captain = Color::opaque(20, 220, 40);
        let mut captain_pixels = [captain.r, captain.g, captain.b, captain.a].repeat(16 * 16);
        captain_pixels[..4].copy_from_slice(&[0, 0, 255, 128]);
        captain_pixels[8..12].copy_from_slice(&[250, 0, 0, 0]);
        let gfx = IngameMenuGraphics {
            hud: HudGraphics {
                captain: Some(ImageData::new(16, 16, captain_pixels)),
                ..HudGraphics::default()
            },
            definition_icons: HashMap::from([(
                "GOLD".to_string(),
                ImageData::new(
                    35,
                    35,
                    [definition.r, definition.g, definition.b, definition.a].repeat(35 * 35),
                ),
            )]),
            ..IngameMenuGraphics::default()
        };
        let font_backend = BitmapFont::new();
        let font = HudFont::Fallback(&font_backend);
        let area = Rect::new(0, 0, 320, 200);
        let symbol = menu
            .layout(area, &font, &gfx)
            .item_rect(0)
            .expect("goal row");
        let mut surface = Surface::new(320, 200, clonk_graphics::PixelFormat::Rgba8888);
        menu.render(&mut surface, area, &font, None, &gfx);
        let scaled_offset = |offset: i32| (offset * symbol.height as i32 + 34) / 35;
        let marker_x = symbol.x + scaled_offset(35 - 16 - 2);
        let marker_y = symbol.y + scaled_offset(2);

        assert_eq!(
            surface.get_pixel(marker_x as u32, marker_y as u32),
            Some(Color::opaque(99, 14, 137)),
            "the Captain marker is software-composited at (17, 2) before row scaling",
        );
        assert_eq!(
            surface.get_pixel((marker_x + 1) as u32, marker_y as u32),
            Some(Color::opaque(199, 29, 19)),
            "transparent Captain pixels retain software BltAlpha's /256 coverage quirk",
        );
        assert_eq!(
            surface.get_pixel((marker_x + 2) as u32, marker_y as u32),
            Some(Color::opaque(19, 219, 39)),
            "opaque Captain pixels retain software BltAlpha's /256 quirk",
        );
        assert_eq!(
            surface.get_pixel(symbol.x as u32, symbol.y as u32),
            Some(definition),
            "the fulfilled marker remains an overlay on the definition picture",
        );
    }

    #[test]
    fn new_player_menu_formats_join_captions() {
        let players = vec![NewPlayerEntry {
            file: "Player2.c4p".to_string(),
            name: "Twonk".to_string(),
        }];
        let menu = IngameMenuState::new_player_menu(&players);
        // IDS_MENU_NEWPLAYER = "Join player: %s"
        assert_eq!(captions(&menu), vec!["Join player: Twonk"]);
        // IDS_MENU_NOPLRFILES doubles as menu caption (C4MainMenu.cpp:71).
        assert_eq!(menu.caption(), "No additional player files available.");
    }

    fn l065_long_menu(count: usize) -> IngameMenuState {
        let players = (0..count)
            .map(|index| NewPlayerEntry {
                file: format!("Player{index}.c4p"),
                name: format!("Player {index}"),
            })
            .collect::<Vec<_>>();
        IngameMenuState::new_player_menu(&players)
    }

    #[test]
    fn l065_selection_scroll_is_persistent_and_minimal() {
        use clonk_graphics::BitmapFont;

        let mut menu = l065_long_menu(8);
        let font_backend = BitmapFont::new();
        let font = HudFont::Fallback(&font_backend);
        let gfx = IngameMenuGraphics::default();
        // Fallback ItemHeight is 16, so this exposes exactly three rows.
        let area = Rect::new(0, 0, 640, 148);
        let initial = menu.layout(area, &font, &gfx);
        assert_eq!(initial.lines, 3);
        assert_eq!(menu.scroll_y(), 0);

        menu.set_selection(4);
        menu.layout(area, &font, &gfx);
        assert_eq!(menu.scroll_y(), 2 * initial.item_height);

        // Row three is already inside rows two through four. Native
        // ScrollRangeInView keeps the existing displacement.
        menu.set_selection(3);
        menu.layout(area, &font, &gfx);
        assert_eq!(menu.scroll_y(), 2 * initial.item_height);

        menu.set_selection(1);
        menu.layout(area, &font, &gfx);
        assert_eq!(menu.scroll_y(), initial.item_height);

        let mut one_line = l065_long_menu(8);
        one_line.set_selection(4);
        assert_eq!(one_line.layout(Rect::new(0, 0, 320, 116), &font, &gfx).lines, 1);
        assert_eq!(
            one_line.scroll_y(),
            0,
            "C4Menu::AdjustPosition is intentionally disabled for one-line menus",
        );
    }

    #[test]
    fn l065_wheel_scroll_survives_layout_and_clips_partial_rows() {
        use clonk_graphics::BitmapFont;

        let menu = l065_long_menu(8);
        let font_backend = BitmapFont::new();
        let font = HudFont::Fallback(&font_backend);
        let gfx = IngameMenuGraphics::default();
        let area = Rect::new(0, 0, 640, 148);
        menu.layout(area, &font, &gfx);
        let selection = menu.selection();

        assert!(menu.scroll_by(7, area, &font, &gfx));
        assert_eq!(menu.selection(), selection);
        assert_eq!(menu.scroll_y(), 7);
        // Rendering/hit-test layout must not reveal the unchanged selection
        // and thereby erase a wheel displacement.
        let layout = menu.layout(area, &font, &gfx);
        assert_eq!(menu.scroll_y(), 7);
        let client = layout.client_rect();
        assert!(menu.client_contains(
            area,
            &font,
            &gfx,
            GuiPoint::new(client.x as f32, client.y as f32),
        ));
        assert_eq!(
            menu.pointer_target(
                area,
                &font,
                &gfx,
                GuiPoint::new(client.x as f32 + 1.0, client.y as f32),
            ),
            Some(IngameMenuPointerTarget::Item(0)),
        );
        assert_eq!(
            menu.pointer_target(
                area,
                &font,
                &gfx,
                GuiPoint::new(
                    client.x as f32 + 1.0,
                    (client.y + client.height as i32) as f32 - 0.5,
                ),
            ),
            Some(IngameMenuPointerTarget::Item(3)),
        );

        let mut surface = Surface::new(640, 148, clonk_graphics::PixelFormat::Rgba8888);
        menu.render(&mut surface, area, &font, None, &gfx);
        assert_eq!(
            surface.get_pixel((client.x + 1) as u32, client.y as u32),
            Some(SELECTION_COLOR),
            "the partially visible selected row starts exactly at the client clip",
        );
        let red = (0..surface.height())
            .flat_map(|y| (0..surface.width()).map(move |x| (x, y)))
            .filter(|&(x, y)| surface.get_pixel(x, y) == Some(SELECTION_COLOR))
            .collect::<Vec<_>>();
        assert!(red.iter().all(|&(_, y)| {
            y >= client.y as u32 && y < (client.y + client.height as i32) as u32
        }));

        assert!(menu.scroll_by(i32::MAX, area, &font, &gfx));
        assert_eq!(menu.scroll_y(), layout.max_scroll);
        assert!(!menu.scroll_by(1, area, &font, &gfx));
        assert_eq!(menu.selection(), selection);
    }

    #[test]
    fn l065_title_location_persists_raw_until_reset() {
        use clonk_graphics::BitmapFont;

        let mut menu = l065_long_menu(8);
        let font_backend = BitmapFont::new();
        let font = HudFont::Fallback(&font_backend);
        let gfx = IngameMenuGraphics {
            show_close_button: true,
            ..IngameMenuGraphics::default()
        };
        let area = Rect::new(50, 20, 640, 148);
        let anchored = menu.bounds(area, &font, &gfx);
        let title = menu.layout(area, &font, &gfx).title_rect();
        assert_eq!(
            menu.pointer_target(
                area,
                &font,
                &gfx,
                GuiPoint::new(title.x as f32 + 2.0, title.y as f32 + 2.0),
            ),
            Some(IngameMenuPointerTarget::Title),
        );

        menu.set_location((-41, 377));
        assert_eq!(menu.bounds(area, &font, &gfx).x, -41);
        assert_eq!(menu.bounds(area, &font, &gfx).y, 377);
        let relaid_area = Rect::new(0, 0, 320, 148);
        let relaid_expected = l065_long_menu(8).bounds(relaid_area, &font, &gfx);
        assert_eq!(
            menu.bounds(relaid_area, &font, &gfx),
            relaid_expected,
            "a split-screen output-rect change resets the dragged location without an OS resize",
        );

        menu.set_location((-41, 377));
        assert!(menu.scroll_by(i32::MAX, area, &font, &gfx));
        menu.reset_location();
        assert_eq!(menu.bounds(area, &font, &gfx), anchored);
        assert_eq!(
            menu.scroll_y(),
            0,
            "InitLocation reruns AdjustPosition for the selected first row",
        );

        menu.set_location((0, 30));
        let mut surface = Surface::new(720, 200, clonk_graphics::PixelFormat::Rgba8888);
        menu.render(&mut surface, area, &font, None, &gfx);
        assert!(
            (0..surface.height()).all(|y| (0..surface.width()).all(|x| {
                let inside = x >= area.x as u32
                    && y >= area.y as u32
                    && x < (area.x + area.width as i32) as u32
                    && y < (area.y + area.height as i32) as u32;
                inside || surface.get_pixel(x, y) == Some(Color::transparent())
            })),
            "a dragged external dialog must remain clipped to its owning viewport",
        );
    }

    // Layout mirrors C4Menu::InitLocation/InitSize (C4Menu.cpp:642-783).
    #[test]
    fn layout_aligns_left_bottom_with_symbol_margin() {
        use clonk_graphics::BitmapFont;
        let menu = IngameMenuState::main_menu(&MainMenuConditions::default()).expect("menu");
        let font_backend = BitmapFont::new();
        let font = HudFont::Fallback(&font_backend);
        let gfx = IngameMenuGraphics {
            show_commands: true,
            ..IngameMenuGraphics::default()
        };
        let area = Rect::new(0, 0, 640, 480);
        let layout = menu.layout(area, &font, &gfx);
        // X = C4SymbolSize (C4Menu.cpp:739).
        assert_eq!(layout.bounds.x, 35);
        // Y = area height - C4SymbolSize - menu height (C4Menu.cpp:742).
        assert_eq!(
            layout.bounds.y,
            480 - 35 - layout.bounds.height as i32
        );
        // Height = lines*itemHeight + title bar + extra bar + frame.
        let expected_height = 7 * layout.item_height + layout.title_height + 16 + 2;
        assert_eq!(layout.bounds.height as i32, expected_height);
        assert_eq!(layout.lines, 7);
        assert_eq!(layout.scroll_y, 0);
    }

    #[test]
    fn mouse_close_button_uses_classic_title_geometry_and_icon_phase() {
        // Dialog::SetTitle reserves GetToprightCornerRect(16,16,4,4) and
        // assigns GUIIcons Ico_Close (phase 34) only when C4Menu::HasMouse
        // (C4GuiDialogs.cpp:386-425; C4Menu.cpp:1270-1276).
        use clonk_graphics::BitmapFont;

        let menu = IngameMenuState::main_menu(&MainMenuConditions::default()).expect("menu");
        let font_backend = BitmapFont::new();
        let font = HudFont::Fallback(&font_backend);
        let area = Rect::new(0, 0, 640, 480);
        let mut gui_icons = vec![0_u8; 240 * 240 * 4];
        for y in 200..240 {
            for x in 160..200 {
                let offset = (y * 240 + x) * 4;
                gui_icons[offset..offset + 4].copy_from_slice(&[17, 238, 51, 255]);
            }
        }
        let gui_icons = ImageData::new(240, 240, gui_icons);
        let gfx = IngameMenuGraphics {
            gui_icons: Some(gui_icons.clone()),
            show_close_button: true,
            ..IngameMenuGraphics::default()
        };
        let layout = menu.layout(area, &font, &gfx);
        let close = menu.close_button_rect(area, &font, &gfx);
        assert_eq!(
            close,
            Rect::new(
                layout.bounds.x + layout.bounds.width as i32 - 20,
                layout.bounds.y + 4,
                16,
                16,
            )
        );
        for point in [
            GuiPoint::new(close.x as f32, close.y as f32),
            GuiPoint::new(
                (close.x + close.width as i32) as f32 - 0.5,
                (close.y + close.height as i32) as f32 - 0.5,
            ),
        ] {
            assert_eq!(
                menu.pointer_target(area, &font, &gfx, point),
                Some(IngameMenuPointerTarget::Close)
            );
        }
        for point in [
            GuiPoint::new((close.x + close.width as i32) as f32, close.y as f32),
            GuiPoint::new(close.x as f32, (close.y + close.height as i32) as f32),
        ] {
            assert_eq!(
                menu.pointer_target(area, &font, &gfx, point),
                Some(IngameMenuPointerTarget::Title),
                "the 16x16 close hitbox is half-open"
            );
        }

        let center = (
            (close.x + close.width as i32 / 2) as u32,
            (close.y + close.height as i32 / 2) as u32,
        );
        let mut visible = Surface::new(640, 480, clonk_graphics::PixelFormat::Rgba8888);
        menu.render(&mut visible, area, &font, None, &gfx);
        let close_pixels = (0..visible.height())
            .flat_map(|y| (0..visible.width()).map(move |x| (x, y)))
            .filter(|&(x, y)| visible.get_pixel(x, y) == Some(Color::opaque(17, 238, 51)))
            .collect::<Vec<_>>();
        assert_eq!(close_pixels.len(), 16 * 16);
        assert!(close_pixels.iter().all(|&(x, y)| {
            x >= close.x as u32
                && x < (close.x + close.width as i32) as u32
                && y >= close.y as u32
                && y < (close.y + close.height as i32) as u32
        }));
        assert_eq!(
            visible.get_pixel(center.0, center.1),
            Some(Color::opaque(17, 238, 51)),
            "Ico_Close is phase 34 in the six-column 40px GUIIcons atlas"
        );

        let hidden_gfx = IngameMenuGraphics {
            gui_icons: Some(gui_icons),
            show_close_button: false,
            ..IngameMenuGraphics::default()
        };
        let mut hidden = Surface::new(640, 480, clonk_graphics::PixelFormat::Rgba8888);
        menu.render(&mut hidden, area, &font, None, &hidden_gfx);
        assert!(
            (0..hidden.height())
                .flat_map(|y| (0..hidden.width()).map(move |x| (x, y)))
                .all(|(x, y)| hidden.get_pixel(x, y) != Some(Color::opaque(17, 238, 51))),
            "the same GUIIcons atlas must remain hidden when HasMouse=false"
        );
        assert_eq!(
            menu.pointer_target(
                area,
                &font,
                &hidden_gfx,
                GuiPoint::new(center.0 as f32, center.1 as f32),
            ),
            Some(IngameMenuPointerTarget::Title),
            "HasMouse=false removes the close control instead of leaving an invisible hitbox"
        );
    }

    #[test]
    fn tooltip_geometry_uses_facet_width_and_bottom_clamp() {
        assert_eq!(tooltip_wrap_width(Rect::new(0, 0, 20, 100)), 50);
        assert_eq!(tooltip_wrap_width(Rect::new(0, 0, 320, 100)), 320);
        assert_eq!(tooltip_wrap_width(Rect::new(0, 0, 800, 100)), 500);
        assert_eq!(tooltip_line_count("first|second\nthird"), 3);

        let facet = Rect::new(100, 20, 320, 50);
        assert_eq!(tooltip_position(facet, 200, 40, 80, 40), (160, 30));
        assert_eq!(
            tooltip_position(facet, 200, facet.y + 40 + 5, 80, 40),
            (160, 20),
            "equality uses the above-pointer branch"
        );
    }

    #[test]
    fn render_smoke_test_draws_selection_box() {
        use clonk_graphics::BitmapFont;
        let menu = IngameMenuState::main_menu(&MainMenuConditions::default()).expect("menu");
        let font_backend = BitmapFont::new();
        let font = HudFont::Fallback(&font_backend);
        let gfx = IngameMenuGraphics::default();
        let mut surface = Surface::new(640, 480, clonk_graphics::PixelFormat::Rgba8888);
        menu.render(
            &mut surface,
            Rect::new(0, 0, 640, 480),
            &font,
            None,
            &gfx,
        );
        // The selected row (first item) is marked with the CRed box
        // (#c80000, C4Menu.cpp:152-154 + C4.PAL entry 10).
        let area = Rect::new(0, 0, 640, 480);
        let layout = menu.layout(area, &font, &gfx);
        // Probe inside the symbol square of the selected row (no icon sheets
        // are loaded in this test, so the red selection box shows through).
        let probe_x = (layout.bounds.x + MN_FRAME_WIDTH + 2) as u32;
        let probe_y = (layout.bounds.y + layout.title_height + layout.item_height / 2) as u32;
        let pixel = surface.get_pixel(probe_x, probe_y).expect("pixel");
        assert_eq!((pixel.r, pixel.g, pixel.b), (0xc8, 0x00, 0x00));
    }

    #[test]
    fn tutorial_seven_gamma_encodes_the_ingame_menu_selection_fragment() {
        // C4GraphicsSystem draws C4GUI before applying a pending ramp at the
        // tail of Execute (C4GraphicsSystem.cpp:167-199). Tutorial07's active
        // ramp therefore samples the CRed selection fragment emitted by
        // C4MenuItem::DrawElement (C4Menu.cpp:152-154).
        use clonk_graphics::BitmapFont;

        let menu = IngameMenuState::main_menu(&MainMenuConditions::default()).expect("menu");
        let font_backend = BitmapFont::new();
        let font = HudFont::Fallback(&font_backend);
        let gfx = IngameMenuGraphics::default();
        let mut surface = Surface::new(640, 480, clonk_graphics::PixelFormat::Rgba8888);
        let area = Rect::new(0, 0, 640, 480);
        let gamma = crate::tutorial_seven_gamma();
        menu.render_with_gamma(&mut surface, area, &font, None, &gfx, Some(&gamma));

        let layout = menu.layout(area, &font, &gfx);
        let probe_x = (layout.bounds.x + MN_FRAME_WIDTH + 2) as u32;
        let probe_y = (layout.bounds.y + layout.title_height + layout.item_height / 2) as u32;
        assert_eq!(
            surface.get_pixel(probe_x, probe_y),
            Some(crate::tutorial_seven_gamma_color(SELECTION_COLOR)),
        );
    }

}
