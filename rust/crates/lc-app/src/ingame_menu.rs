//! In-game player menu: a faithful port of `C4MainMenu`
//! (src/C4MainMenu.cpp) on the classic `C4Menu` context-style chassis
//! (src/C4Menu.cpp). Entry lists, order, captions (LanguageUS.txt), icons
//! (GfxR facets) and command semantics mirror the C++ oracle; the renderer
//! reproduces the classic menu furniture (semi-transparent black dialog with
//! a 3D frame, wooden caption bar, red selection box, symbol+text lines and
//! the bottom command-key bar).
//!
//! C++ opens this menu per player via `COM_PlayerMenu`
//! (C4Game.cpp:3593-3601 -> C4Player::ActivateMenuMain, C4Player.cpp:2327);
//! lc-app additionally opens it on Escape (C++ Escape shows the
//! C4AbortGameDialog instead — approximated here by the "Abort round?"
//! confirmation page).

use std::collections::HashMap;

use lc_engine::{CommandKind, ControlCommand};
use lc_frontend::{hud::HudFont, HudGraphics};
use lc_graphics::clonk_font::TextAlign;
use lc_graphics::{Color, GammaRamp, Rect, Surface};
use lc_gui::ImageData;

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
const MAX_TOOLTIP_WDT: i32 = 256;

/// `C4GUI::Icons` indices into GUIIcons.png (C4Gui.h:670-731), 40x40 cells
/// (C4Gui.cpp:1094-1095), 6 per row (C4GuiLabels.cpp:441-450).
pub const ICO_TEAM: u8 = 19;
pub const ICO_GAME_RUNNING: u8 = 30;
pub const ICO_EXIT: u8 = 33;
pub const ICO_SURRENDER: u8 = 45;
pub const ICO_DISCONNECT: u8 = 49;
pub const ICO_VIEW: u8 = 50;

/// GfxR facet references for menu symbols (C4GraphicsResource.cpp:199-227).
#[derive(Clone, Debug)]
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
    /// A definition picture (`pDef->Draw(fctSymbol)`, C4MainMenu.cpp:367).
    Definition(String),
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
    /// "ActivateMenu:Client" (C4MainMenu.cpp:752).
    ActivateClientDisconnect,
    /// "ActivateMenu:Host" (C4MainMenu.cpp:751).
    ActivateHostDisconnect,
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
    /// C4AbortGameDialog "Yes": `Game.Abort()` (C4GameDialogs.cpp:104-121).
    AbortConfirmed,
    /// C4AbortGameDialog "Restart": `Application.SetNextMission` + abort
    /// (C4GameDialogs.cpp:116-120).
    RestartRound,
    /// Items with an empty command (the "No" buttons, C4MainMenu.cpp:533).
    NoOp,
}

/// `C4Menu::Identification`-style page tag for the active menu.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MenuPage {
    Main,
    Goals,
    Rules,
    NewPlayer,
    Savegame,
    Options,
    Display,
    Surrender,
    ClientDisconnect,
    AbortConfirm,
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

/// Display toggles shown by `ActivateDisplay` (C4MainMenu.cpp:582-641),
/// defaults per C4Config.cpp:381,446-465.
#[derive(Clone, Copy, Debug)]
pub struct DisplayFlags {
    pub player_names: bool,
    pub clonk_names: bool,
    pub portraits: bool,
    pub show_commands: bool,
    pub show_command_keys: bool,
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
/// (C4MainMenu.cpp:332-405): definition id + name (+fulfilled for goals).
#[derive(Clone, Debug)]
pub struct GoalRuleEntry {
    pub definition_id: String,
    pub name: String,
    pub fulfilled: bool,
}

/// A player file entry for `ActivateNewPlayer` (C4MainMenu.cpp:59-122).
#[derive(Clone, Debug)]
pub struct NewPlayerEntry {
    pub file: String,
    pub name: String,
}

/// The active in-game menu: one `C4MainMenu` page (C4Menu state per
/// C4Menu.h:134-268 — caption, symbol, item list, selection, permanent flag
/// and close command).
pub struct IngameMenuState {
    page: MenuPage,
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
            page,
            caption: caption.into(),
            symbol,
            items,
            selection: 0,
            permanent,
            close_action,
            time_on_selection: 0,
        }
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
                MenuAction::NoOp, // team switch not ported; see PORT_STATUS
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

    /// `C4MainMenu::ActivateGoals` (C4MainMenu.cpp:332-380).
    pub fn goals_menu(goals: &[GoalRuleEntry]) -> Self {
        let items = goals
            .iter()
            .map(|goal| {
                MenuItem::new(
                    goal.name.clone(),
                    MenuSymbol::Definition(goal.definition_id.clone()),
                    MenuAction::GoalInfo(goal.definition_id.clone()),
                    None,
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
                    MenuSymbol::Definition(rule.definition_id.clone()),
                    MenuAction::RuleInfo(rule.definition_id.clone()),
                    None,
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

    /// Menu-shaped approximation of `C4AbortGameDialog`
    /// (C4GameDialogs.cpp:33-79): "Abort round?" with Yes / (Restart) / No;
    /// the restart button appears for the control host (local rounds).
    pub fn abort_confirm_menu(show_restart: bool) -> Self {
        let mut items = vec![MenuItem::new(
            "Yes",
            MenuSymbol::OkCancel(3, 0),
            MenuAction::AbortConfirmed,
            None,
        )];
        if show_restart {
            items.push(MenuItem::new(
                "Restart",
                MenuSymbol::GuiIcon(ICO_GAME_RUNNING),
                MenuAction::RestartRound,
                None,
            ));
        }
        items.push(MenuItem::new(
            "No",
            MenuSymbol::OkCancel(1, 0),
            MenuAction::NoOp,
            None,
        ));
        Self::new(
            MenuPage::AbortConfirm,
            "Abort round?",
            MenuSymbol::GuiIcon(ICO_EXIT),
            items,
            false,
            Some(MenuAction::ActivateMain),
        )
    }

    pub fn page(&self) -> MenuPage {
        self.page
    }

    pub fn caption(&self) -> &str {
        &self.caption
    }

    pub fn items(&self) -> &[MenuItem] {
        &self.items
    }

    pub fn selection(&self) -> usize {
        self.selection
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
    }

    /// Advances `TimeOnSelection` once per frame while the menu is shown
    /// (C4Menu::Draw, C4Menu.cpp:805).
    pub fn tick(&mut self) {
        self.time_on_selection = self.time_on_selection.saturating_add(1);
    }

    /// `C4Menu::Control` (C4Menu.cpp:433-484) for a one-column menu: all
    /// four directions move the selection by one with wrap-around; Enter
    /// activates; COM_MenuClose closes with the close command.
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
            ControlCommand::MenuUp | ControlCommand::MenuLeft => {
                self.move_selection(-1);
                None
            }
            ControlCommand::MenuDown | ControlCommand::MenuRight => {
                self.move_selection(1);
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
        self.selection = (self.selection as i32 + delta).rem_euclid(len) as usize;
        self.time_on_selection = 0;
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
        draw_menu(self, &layout, surface, font, tiny_font, gfx, gamma);
    }

    /// Menu geometry per `C4Menu::InitLocation`/`InitSize`
    /// (C4Menu.cpp:642-783) for `C4MN_Style_Context`, one column.
    fn layout(&self, area: Rect, font: &HudFont<'_>, gfx: &IngameMenuGraphics) -> MenuLayout {
        // ItemHeight = max(C4MN_SymbolSize, font line height) (C4Menu.cpp:650).
        let item_height = MN_SYMBOL_SIZE.max(font.line_height());
        // Caption contributes ItemHeight + 16 (C4Menu.cpp:652-655).
        let mut item_width = font.text_width(&self.caption) + item_height + 16;
        for item in &self.items {
            // symbol width == item height in context menus (C4Menu.cpp:137-139).
            item_width = item_width.max(font.text_width(&item.caption) + item_height);
        }
        item_width += 3; // (C4Menu.cpp:664)

        let area_w = area.width as i32;
        let area_h = area.height as i32;
        // Lines = item count clamped to the viewport (C4Menu.cpp:715-720).
        let lines = (self.items.len() as i32)
            .min(((area_h - 100) / item_height.max(1)).max(1))
            .max(1);

        // Margins: title bar on top (Dialog::GetMarginTop,
        // C4GuiDialogs.h:95), C4MN_FrameWidth left/right/bottom plus the
        // extra bar when menu controls are drawn (C4Menu.h:262-264).
        let title_height = font.line_height().max(MIN_WOOD_BAR_HGT);
        let extra_height = if gfx.show_commands { MN_SYMBOL_SIZE } else { 0 };
        let width = item_width + 2 * MN_FRAME_WIDTH;
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

        // Scroll the selection into view (C4Menu::AdjustPosition,
        // C4Menu.cpp:601-609).
        let visible = lines as usize;
        let scroll = if self.selection >= visible {
            self.selection + 1 - visible
        } else {
            0
        };

        MenuLayout {
            bounds: Rect::new(x, y, width as u32, height as u32),
            item_width,
            item_height,
            title_height,
            lines,
            scroll,
        }
    }
}

/// Computed menu geometry (see [`IngameMenuState::layout`]).
struct MenuLayout {
    bounds: Rect,
    item_width: i32,
    item_height: i32,
    title_height: i32,
    lines: i32,
    scroll: usize,
}

/// Graphics.c4g sheets and flags the renderer needs; missing sheets degrade
/// to text-only rendering (headless tests without game data).
#[derive(Default)]
pub struct IngameMenuGraphics {
    /// Shared HUD facets used by composite object-menu symbols.
    pub hud: HudGraphics,
    /// Runtime player colors keyed by C4Player number.
    pub owner_colors: HashMap<i32, Color>,
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
    /// `Config.Graphics.ShowCommands` (C4Config.cpp:449) — draws the bottom
    /// command bar (C4Menu.cpp:851-880).
    pub show_commands: bool,
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
            MenuSymbol::Definition(id) => self
                .definition_icons
                .get(id)
                .map(|img| (img, Rect::new(0, 0, img.width(), img.height()))),
        }
    }
}

fn draw_menu(
    menu: &IngameMenuState,
    layout: &MenuLayout,
    surface: &mut Surface,
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
    let title_rect = Rect::new(x0, y0, bounds.width, layout.title_height as u32);
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

    // Client area: items stacked vertically (C4Menu::UpdateElementPositions).
    let client_x = x0 + MN_FRAME_WIDTH;
    let client_y = y0 + layout.title_height;
    let visible = layout.lines as usize;
    for (row, (index, item)) in menu
        .items()
        .iter()
        .enumerate()
        .skip(layout.scroll)
        .take(visible)
        .enumerate()
    {
        let item_y = client_y + row as i32 * layout.item_height;
        let row_rect = Rect::new(
            client_x,
            item_y,
            layout.item_width as u32,
            layout.item_height as u32,
        );
        // Selection mark: filled red box (C4MenuItem::DrawElement,
        // C4Menu.cpp:152-154).
        if index == menu.selection() {
            fill_rect(surface, row_rect, SELECTION_COLOR, gamma);
        }
        // Symbol square at the left, width == item height (C4Menu.cpp:156-166).
        if let Some((image, src)) = gfx.symbol_source(&item.symbol) {
            draw_image_region_aspect(
                surface,
                image,
                src,
                Rect::new(client_x, item_y, layout.item_height as u32, row_rect.height),
                matches!(item.symbol, MenuSymbol::PlayerColor),
                gamma,
            );
        }
        // Caption (C4MN_Style_Context: FontRegular, left, C4Menu.cpp:170-172).
        font.draw_with_gamma(
            surface,
            client_x + layout.item_height,
            item_y,
            &item.caption,
            MESSAGE_COLOR,
            TextAlign::Left,
            gamma,
        );
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
        // divider frame in palette color 80 (#440000) (C4Menu.cpp:932-935).
        draw_rect_outline(surface, extra, EXTRA_FRAME_COLOR, gamma);
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
            let row = menu.selection().saturating_sub(layout.scroll);
            let item_x = client_x;
            let item_y = client_y + row as i32 * layout.item_height;
            draw_tooltip(surface, font, item_x, item_y, info, gamma);
        }
    }
}

/// `DrawCommandKey` (C4ObjectCom.cpp:930-945): key cap (fctKey, Control.png
/// (0,100) 64x64) + command symbol (fctCommand, Control.png (0,36) 32x32
/// phases) + the key name in the small font when ShowCommandKeys is set.
pub(crate) fn draw_command_key(
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
pub(crate) fn draw_ok_cancel(
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
pub(crate) fn draw_tooltip(
    surface: &mut Surface,
    font: &HudFont<'_>,
    x: i32,
    y: i32,
    text: &str,
    gamma: Option<&GammaRamp>,
) {
    let area_w = surface.width() as i32;
    let lines = break_message(font, text, MAX_TOOLTIP_WDT);
    let text_w = lines
        .iter()
        .map(|line| font.text_width(line))
        .max()
        .unwrap_or(0);
    let text_h = font.line_height() * lines.len() as i32;
    let w = text_w + 6;
    let h = text_h + 4;
    let ty = if y < h + 5 { y + 5 } else { y - h - 5 };
    let tx = (x - w / 2).clamp(0, (area_w - w).max(0));
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
    for (index, line) in lines.iter().enumerate() {
        font.draw_with_gamma(
            surface,
            tx + 3,
            ty + 1 + index as i32 * font.line_height(),
            line,
            TOOLTIP_TEXT_COLOR,
            TextAlign::Left,
            gamma,
        );
    }
}

/// Simple word wrap in the spirit of `CStdFont::BreakMessage`
/// (StdFont.cpp): greedy fill by word up to `max_width` pixels.
pub(crate) fn break_message(font: &HudFont<'_>, text: &str, max_width: i32) -> Vec<String> {
    let mut lines = Vec::new();
    for raw_line in text.split(['\n', '|']) {
        let mut current = String::new();
        for word in raw_line.split_whitespace() {
            let candidate = if current.is_empty() {
                word.to_string()
            } else {
                format!("{current} {word}")
            };
            if !current.is_empty() && font.text_width(&candidate) > max_width {
                lines.push(std::mem::take(&mut current));
                current = word.to_string();
            } else {
                current = candidate;
            }
        }
        lines.push(current);
    }
    if lines.is_empty() {
        lines.push(String::new());
    }
    lines
}

/// `C4GUI::Element::Draw3DFrame` (C4Gui.cpp:264-279) with the default border
/// colors at `C4GUI_BorderAlpha`.
pub(crate) fn draw_3d_frame(surface: &mut Surface, rect: Rect, gamma: Option<&GammaRamp>) {
    let x0 = rect.x;
    let y0 = rect.y;
    let x1 = rect.x + rect.width as i32 - 1;
    let y1 = rect.y + rect.height as i32 - 1;
    // DrawLineDw covers every pixel except the final one (GL diamond-exit).
    let hline = |surface: &mut Surface, x_start: i32, x_end: i32, y: i32, color: Color| {
        fill_rect(
            surface,
            Rect::new(x_start, y, (x_end - x_start).max(0) as u32, 1),
            color,
            gamma,
        );
    };
    let vline = |surface: &mut Surface, x: i32, y_start: i32, y_end: i32, color: Color| {
        fill_rect(
            surface,
            Rect::new(x, y_start, 1, (y_end - y_start).max(0) as u32),
            color,
            gamma,
        );
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
pub(crate) fn draw_caption_bar(
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
pub(crate) fn draw_image_region(
    surface: &mut Surface,
    image: &ImageData,
    src: Rect,
    dest: Rect,
    gamma: Option<&GammaRamp>,
) {
    if dest.width == 0 || dest.height == 0 || src.width == 0 || src.height == 0 {
        return;
    }
    let bounds = surface.bounds();
    let pixels = image.pixels();
    let img_w = image.width();
    let img_h = image.height();
    for dy in 0..dest.height {
        let target_y = dest.y + dy as i32;
        if target_y < bounds.y || target_y >= bounds.y + bounds.height as i32 {
            continue;
        }
        let sy = src.y + ((dy as u64 * src.height as u64) / dest.height as u64) as i32;
        if sy < 0 || sy >= img_h as i32 {
            continue;
        }
        for dx in 0..dest.width {
            let target_x = dest.x + dx as i32;
            if target_x < bounds.x || target_x >= bounds.x + bounds.width as i32 {
                continue;
            }
            let sx = src.x + ((dx as u64 * src.width as u64) / dest.width as u64) as i32;
            if sx < 0 || sx >= img_w as i32 {
                continue;
            }
            let idx = ((sy as u32 * img_w + sx as u32) * 4) as usize;
            if idx + 3 >= pixels.len() {
                continue;
            }
            let color = Color::new(pixels[idx], pixels[idx + 1], pixels[idx + 2], pixels[idx + 3]);
            if color.a == 0 {
                continue;
            }
            let result = if let Some(gamma) = gamma {
                let destination = surface
                    .get_pixel(target_x as u32, target_y as u32)
                    .unwrap_or_default();
                let output = if color.a == 255 {
                    lc_frontend::gamma_encode_fragment(color, gamma)
                } else {
                    lc_frontend::gamma_blend_fragment_over(color, destination, gamma)
                };
                surface.set_pixel(target_x as u32, target_y as u32, output)
            } else if color.a == 255 {
                surface.set_pixel(target_x as u32, target_y as u32, color)
            } else {
                surface.blend_pixel(target_x as u32, target_y as u32, color)
            };
            if result.is_err() {
                return;
            }
        }
    }
}

/// `C4Facet::Draw` with `fAspect=true`: scale to fit, keep the aspect ratio
/// and center in the target. `colorize` applies the default blue player
/// color like `fctPlayerClr.Surface->SetClr(0xff)` (C4MainMenu.cpp:69-70).
pub(crate) fn draw_image_region_aspect(
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
        let colored = lc_frontend::hud::colorize_by_owner(image, Color::opaque(0, 0, 0xff));
        draw_image_region(surface, &colored, src, fitted, gamma);
    } else {
        draw_image_region(surface, image, src, fitted, gamma);
    }
}

fn fill_rect(surface: &mut Surface, rect: Rect, color: Color, gamma: Option<&GammaRamp>) {
    lc_frontend::draw_color_rect(surface, rect, color, gamma);
}

fn draw_rect_outline(surface: &mut Surface, rect: Rect, color: Color, gamma: Option<&GammaRamp>) {
    if rect.width == 0 || rect.height == 0 {
        return;
    }
    fill_rect(
        surface,
        Rect::new(rect.x, rect.y, rect.width, 1),
        color,
        gamma,
    );
    fill_rect(
        surface,
        Rect::new(rect.x, rect.y + rect.height as i32 - 1, rect.width, 1),
        color,
        gamma,
    );
    fill_rect(
        surface,
        Rect::new(rect.x, rect.y, 1, rect.height),
        color,
        gamma,
    );
    fill_rect(
        surface,
        Rect::new(rect.x + rect.width as i32 - 1, rect.y, 1, rect.height),
        color,
        gamma,
    );
}

#[cfg(test)]
mod tests {
    use super::*;

    fn captions(menu: &IngameMenuState) -> Vec<&str> {
        menu.items()
            .iter()
            .map(|item| item.caption.as_str())
            .collect()
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

    // Abort confirmation mirrors C4AbortGameDialog: Yes/Restart/No for the
    // control host, Yes/No otherwise (C4GameDialogs.cpp:33-79).
    #[test]
    fn abort_confirm_offers_restart_for_control_host() {
        let menu = IngameMenuState::abort_confirm_menu(true);
        assert_eq!(captions(&menu), vec!["Yes", "Restart", "No"]);
        assert_eq!(menu.items()[1].action, MenuAction::RestartRound);
        let menu = IngameMenuState::abort_confirm_menu(false);
        assert_eq!(captions(&menu), vec!["Yes", "No"]);
    }

    #[test]
    fn goals_menu_uses_definition_symbols() {
        let goals = vec![GoalRuleEntry {
            definition_id: "GOLD".to_string(),
            name: "Gold Rush".to_string(),
            fulfilled: false,
        }];
        let menu = IngameMenuState::goals_menu(&goals);
        assert_eq!(captions(&menu), vec!["Gold Rush"]);
        assert!(matches!(&menu.items()[0].symbol, MenuSymbol::Definition(id) if id == "GOLD"));
        assert_eq!(
            menu.items()[0].action,
            MenuAction::GoalInfo("GOLD".to_string())
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

    // Layout mirrors C4Menu::InitLocation/InitSize (C4Menu.cpp:642-783).
    #[test]
    fn layout_aligns_left_bottom_with_symbol_margin() {
        use lc_graphics::BitmapFont;
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
        assert_eq!(layout.scroll, 0);
    }

    #[test]
    fn render_smoke_test_draws_selection_box() {
        use lc_graphics::BitmapFont;
        let menu = IngameMenuState::main_menu(&MainMenuConditions::default()).expect("menu");
        let font_backend = BitmapFont::new();
        let font = HudFont::Fallback(&font_backend);
        let gfx = IngameMenuGraphics::default();
        let mut surface = Surface::new(640, 480, lc_graphics::PixelFormat::Rgba8888);
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
        use lc_graphics::BitmapFont;

        let menu = IngameMenuState::main_menu(&MainMenuConditions::default()).expect("menu");
        let font_backend = BitmapFont::new();
        let font = HudFont::Fallback(&font_backend);
        let gfx = IngameMenuGraphics::default();
        let mut surface = Surface::new(640, 480, lc_graphics::PixelFormat::Rgba8888);
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
