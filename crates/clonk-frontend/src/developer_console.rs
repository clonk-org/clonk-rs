//! Platform-neutral model and software presentation for the classic
//! `C4Console` window (`src/C4Console.cpp`).
//!
//! The native implementations use Win32 or GTK widgets.  Keeping the state,
//! menu commands, and hit testing here lets `clonk-app` host the same console in
//! a winit window without coupling this crate to a windowing toolkit.  File
//! actions expose both a software path-entry prompt and a request token that a
//! platform host may satisfy with a native file chooser.

use std::path::PathBuf;

use clonk_graphics::{Color, Surface, TextFont};
use clonk_gui::Rect as GuiRect;

use crate::classic_gui::IntRect;
use crate::{fill_rect, GuiPoint};

pub const CONSOLE_LOG_CHARACTER_CAP: usize = 60_000;

const MENU_BAR_HEIGHT: i32 = 24;
const MENU_ITEM_HEIGHT: i32 = 22;
const MENU_SEPARATOR_HEIGHT: i32 = 8;
const STATUS_BAR_HEIGHT: i32 = 24;
const TOOLBAR_HEIGHT: i32 = 34;
const INPUT_HEIGHT: i32 = 26;
const WINDOW_PADDING: i32 = 5;
const FONT_SIZE: f32 = 13.0;
const SMALL_FONT_SIZE: f32 = 11.0;

const WINDOW_BACKGROUND: Color = Color::opaque(0xd4, 0xd0, 0xc8);
const CONTROL_BACKGROUND: Color = Color::opaque(0xff, 0xff, 0xff);
const CONTROL_TEXT: Color = Color::opaque(0x10, 0x10, 0x10);
const DISABLED_TEXT: Color = Color::opaque(0x78, 0x78, 0x78);
const SELECTED_BACKGROUND: Color = Color::opaque(0x31, 0x6a, 0xc5);
const SELECTED_TEXT: Color = Color::opaque(0xff, 0xff, 0xff);
const LIGHT_EDGE: Color = Color::opaque(0xff, 0xff, 0xff);
const DARK_EDGE: Color = Color::opaque(0x60, 0x60, 0x60);
const MID_EDGE: Color = Color::opaque(0x9a, 0x9a, 0x9a);

/// App-owned presentation strings for the native `C4Console` shell.
///
/// Resource-backed defaults match `LanguageUS.txt`; strings that C++ renders
/// literally (the status words, `?`, GTK file-dialog titles, and the software
/// host's text substitutes for bitmap toolbar buttons) keep those native
/// literals here. The app can replace the resource-backed members from the
/// active language table without teaching the frontend about C4Group files.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ConsoleStrings {
    pub default_caption: String,
    pub menu_file: String,
    pub menu_components: String,
    pub menu_player: String,
    pub menu_viewport: String,
    pub menu_net: String,
    pub menu_help: String,
    pub file_open: String,
    pub file_open_with_players: String,
    pub file_save_scenario: String,
    pub file_save_scenario_as: String,
    pub file_save_game: String,
    pub file_save_game_as: String,
    pub file_record: String,
    pub file_close: String,
    pub file_quit: String,
    pub component_objects: String,
    pub component_script: String,
    pub component_title: String,
    pub component_info: String,
    pub player_join: String,
    pub viewport_new: String,
    pub help_about: String,
    pub tool_play: String,
    pub tool_halt: String,
    pub tool_mode_play: String,
    pub tool_mode_edit: String,
    pub tool_mode_draw: String,
    pub status_frame_prefix: String,
    pub status_script_prefix: String,
    pub status_fps_suffix: String,
    pub path_load_title: String,
    pub path_save_title: String,
    pub path_scenario_prompt: String,
    pub path_players_prompt: String,
    pub path_target_prompt: String,
    pub path_scenario_filter: String,
    pub path_player_filter: String,
    pub path_entry_hint: String,
}

impl Default for ConsoleStrings {
    fn default() -> Self {
        Self {
            default_caption: "Console".to_owned(),
            menu_file: "File".to_owned(),
            menu_components: "Components".to_owned(),
            menu_player: "Player".to_owned(),
            menu_viewport: "Viewport".to_owned(),
            menu_net: "Host".to_owned(),
            menu_help: "?".to_owned(),
            file_open: "Open...".to_owned(),
            file_open_with_players: "Open with players...".to_owned(),
            file_save_scenario: "Save scenario".to_owned(),
            file_save_scenario_as: "Save scenario as...".to_owned(),
            file_save_game: "Save game".to_owned(),
            file_save_game_as: "Save game as...".to_owned(),
            file_record: "Record".to_owned(),
            file_close: "Close".to_owned(),
            file_quit: "Quit".to_owned(),
            component_objects: "Objects".to_owned(),
            component_script: "Script".to_owned(),
            component_title: "Title".to_owned(),
            component_info: "Info".to_owned(),
            player_join: "Join".to_owned(),
            viewport_new: "New".to_owned(),
            help_about: "About...".to_owned(),
            tool_play: "Play".to_owned(),
            tool_halt: "Halt".to_owned(),
            tool_mode_play: "Mouse".to_owned(),
            tool_mode_edit: "Edit".to_owned(),
            tool_mode_draw: "Draw".to_owned(),
            status_frame_prefix: "Frame".to_owned(),
            status_script_prefix: "Script".to_owned(),
            status_fps_suffix: "FPS".to_owned(),
            path_load_title: "Load file...".to_owned(),
            path_save_title: "Save file...".to_owned(),
            path_scenario_prompt: "Scenario path".to_owned(),
            path_players_prompt: "Player path(s)".to_owned(),
            path_target_prompt: "Target scenario path".to_owned(),
            path_scenario_filter: "Clonk 4 Scenario".to_owned(),
            path_player_filter: "Clonk 4 Player".to_owned(),
            path_entry_hint: "Enter: accept    Esc: cancel    Separate multiple paths with ;"
                .to_owned(),
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum ConsoleMenu {
    File,
    Components,
    Player,
    Viewport,
    Net,
    Help,
}

impl ConsoleMenu {
    pub fn label(self, strings: &ConsoleStrings) -> &str {
        match self {
            Self::File => &strings.menu_file,
            Self::Components => &strings.menu_components,
            Self::Player => &strings.menu_player,
            Self::Viewport => &strings.menu_viewport,
            Self::Net => &strings.menu_net,
            Self::Help => &strings.menu_help,
        }
    }

    fn tab_width(self, strings: &ConsoleStrings) -> i32 {
        (self.label(strings).chars().count() as i32 * 8 + 16).clamp(34, 180)
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub enum ConsoleEditMode {
    #[default]
    Play,
    Edit,
    Draw,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum ConsoleSaveKind {
    Scenario,
    Savegame,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum ConsolePathPurpose {
    OpenScenario,
    OpenScenarioWithPlayersScenario,
    OpenScenarioWithPlayersPlayers,
    SaveScenarioAs,
    SaveGameAs,
    JoinPlayers,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ConsolePathRequest {
    pub token: u64,
    pub purpose: ConsolePathPurpose,
    pub title: String,
    pub prompt: String,
    pub allow_multiple: bool,
    pub save: bool,
    pub suggested_path: Option<PathBuf>,
    pub filter_label: String,
    pub extensions: Vec<&'static str>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum DeveloperConsoleAction {
    RequestPath(ConsolePathRequest),
    OpenGame {
        scenario: PathBuf,
        player_files: Vec<PathBuf>,
    },
    Save {
        kind: ConsoleSaveKind,
        target: Option<PathBuf>,
    },
    RequestRuntimeRecord,
    CloseGame,
    QuitApplication,
    Play,
    Halt,
    TogglePause,
    SetEditMode(ConsoleEditMode),
    SubmitInput(String),
    EditObjects,
    EditScript,
    EditTitle,
    EditInfo,
    JoinPlayers(Vec<PathBuf>),
    EliminatePlayer(i32),
    NewViewport(Option<i32>),
    KickClient(i32),
    ShowAbout,
}

#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub enum ConsoleMenuItemId {
    Open,
    OpenWithPlayers,
    SaveScenario,
    SaveScenarioAs,
    SaveGame,
    SaveGameAs,
    Record,
    Close,
    Quit,
    Objects,
    Script,
    Title,
    Info,
    JoinPlayer,
    QuitPlayer(i32),
    NewViewport,
    NewPlayerViewport(i32),
    NetClient(i32),
    About,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ConsoleMenuItem {
    pub id: ConsoleMenuItemId,
    pub label: String,
    pub enabled: bool,
    pub checked: bool,
}

impl ConsoleMenuItem {
    fn new(id: ConsoleMenuItemId, label: impl Into<String>, enabled: bool) -> Self {
        Self {
            id,
            label: label.into(),
            enabled,
            checked: false,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ConsoleMenuEntry {
    Item(ConsoleMenuItem),
    Separator,
}

impl ConsoleMenuEntry {
    pub fn item(&self) -> Option<&ConsoleMenuItem> {
        match self {
            Self::Item(item) => Some(item),
            Self::Separator => None,
        }
    }
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct ConsolePlayerRow {
    pub number: i32,
    /// Fully localized `IDS_CNS_PLRQUIT`/`IDS_CNS_PLRQUITNET` caption.
    pub quit_label: String,
    pub quit_enabled: bool,
    /// Fully localized `IDS_CNS_NEWPLRVIEWPORT` caption.
    pub viewport_label: String,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct ConsoleClientRow {
    pub id: i32,
    /// Fully localized host/client/deactivated caption from `UpdateNetMenu`.
    pub menu_label: String,
    pub menu_enabled: bool,
}

/// Complete app-owned projection consumed by the shell.  It deliberately
/// contains values rather than references so the frontend never observes a
/// half-updated player/client menu while synchronized controls execute.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ConsoleViewModel {
    pub strings: ConsoleStrings,
    pub caption: String,
    pub current_scenario_path: Option<PathBuf>,
    pub game_open: bool,
    pub lobby_active: bool,
    pub editing: bool,
    pub halted: bool,
    pub runtime_record_possible: bool,
    pub network_enabled: bool,
    pub network_host: bool,
    pub players: Vec<ConsolePlayerRow>,
    pub clients: Vec<ConsoleClientRow>,
    /// GTK-order public engine functions followed by scenario functions.
    pub completions: Vec<String>,
    pub edit_mode: ConsoleEditMode,
    pub cursor_text: String,
    pub frame: u64,
    pub script_counter: i32,
    pub time_seconds: i32,
    pub frames_per_second: i32,
}

impl Default for ConsoleViewModel {
    fn default() -> Self {
        let strings = ConsoleStrings::default();
        Self {
            caption: strings.default_caption.clone(),
            strings,
            current_scenario_path: None,
            game_open: false,
            lobby_active: false,
            editing: true,
            halted: true,
            runtime_record_possible: false,
            network_enabled: false,
            network_host: false,
            players: Vec::new(),
            clients: Vec::new(),
            completions: Vec::new(),
            edit_mode: ConsoleEditMode::Play,
            cursor_text: String::new(),
            frame: 0,
            script_counter: 0,
            time_seconds: 0,
            frames_per_second: 0,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ConsoleStatusBars {
    frame: u64,
    script: i32,
    time: i32,
    fps: i32,
    frame_prefix: String,
    script_prefix: String,
    fps_suffix: String,
    frame_text: String,
    script_text: String,
    time_text: String,
    revision: u64,
}

impl Default for ConsoleStatusBars {
    fn default() -> Self {
        let strings = ConsoleStrings::default();
        Self {
            frame: 0,
            script: 0,
            time: 0,
            fps: 0,
            frame_prefix: strings.status_frame_prefix.clone(),
            script_prefix: strings.status_script_prefix.clone(),
            fps_suffix: strings.status_fps_suffix.clone(),
            frame_text: format!("{}: 0", strings.status_frame_prefix),
            script_text: format!("{}: 0", strings.status_script_prefix),
            time_text: format!("00:00:00 (0 {})", strings.status_fps_suffix),
            revision: 0,
        }
    }
}

impl ConsoleStatusBars {
    /// Mirrors `C4Console::UpdateStatusBars`: each label is replaced only when
    /// one of the source values belonging to that label changes.
    pub fn update(
        &mut self,
        frame: u64,
        script: i32,
        time: i32,
        fps: i32,
        strings: &ConsoleStrings,
    ) -> bool {
        let mut changed = false;
        if self.frame != frame || self.frame_prefix != strings.status_frame_prefix {
            self.frame = frame;
            self.frame_prefix.clone_from(&strings.status_frame_prefix);
            self.frame_text = format!("{}: {frame}", self.frame_prefix);
            changed = true;
        }
        if self.script != script || self.script_prefix != strings.status_script_prefix {
            self.script = script;
            self.script_prefix.clone_from(&strings.status_script_prefix);
            self.script_text = format!("{}: {script}", self.script_prefix);
            changed = true;
        }
        if self.time != time || self.fps != fps || self.fps_suffix != strings.status_fps_suffix {
            self.time = time;
            self.fps = fps;
            self.fps_suffix.clone_from(&strings.status_fps_suffix);
            self.time_text = format!(
                "{:02}:{:02}:{:02} ({} {})",
                time / 3_600,
                (time % 3_600) / 60,
                time % 60,
                fps,
                self.fps_suffix,
            );
            changed = true;
        }
        if changed {
            self.revision = self.revision.wrapping_add(1);
        }
        changed
    }

    pub fn frame_text(&self) -> &str {
        &self.frame_text
    }

    pub fn script_text(&self) -> &str {
        &self.script_text
    }

    pub fn time_text(&self) -> &str {
        &self.time_text
    }

    pub const fn revision(&self) -> u64 {
        self.revision
    }
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct ConsoleLogBuffer {
    text: String,
    characters: usize,
    revision: u64,
}

impl ConsoleLogBuffer {
    pub fn out(&mut self, text: &str) -> bool {
        if text.is_empty() {
            return false;
        }
        self.text.push_str(text);
        self.characters = self.characters.saturating_add(text.chars().count());
        if !text.ends_with('\n') {
            self.text.push('\n');
            self.characters = self.characters.saturating_add(1);
        }
        if self.characters > CONSOLE_LOG_CHARACTER_CAP {
            let discard = self.characters - CONSOLE_LOG_CHARACTER_CAP;
            let byte = self
                .text
                .char_indices()
                .nth(discard)
                .map_or(self.text.len(), |(byte, _)| byte);
            self.text.drain(..byte);
            self.characters = CONSOLE_LOG_CHARACTER_CAP;
        }
        self.revision = self.revision.wrapping_add(1);
        true
    }

    pub fn clear(&mut self) -> bool {
        if self.text.is_empty() {
            return false;
        }
        self.text.clear();
        self.characters = 0;
        self.revision = self.revision.wrapping_add(1);
        true
    }

    pub fn text(&self) -> &str {
        &self.text
    }

    pub const fn character_count(&self) -> usize {
        self.characters
    }

    pub const fn revision(&self) -> u64 {
        self.revision
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DeveloperConsoleKey {
    Enter,
    Tab,
    Escape,
    Backspace,
    Delete,
    Left,
    Right,
    Home,
    End,
    Up,
    Down,
    PageUp,
    PageDown,
    Pause,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ConsolePathEntryView {
    pub request: ConsolePathRequest,
    pub text: String,
    pub caret: usize,
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct TextEdit {
    text: String,
    caret: usize,
    select_all: bool,
}

impl TextEdit {
    fn new(text: String) -> Self {
        let caret = text.chars().count();
        Self {
            text,
            caret,
            select_all: false,
        }
    }

    fn byte_at(&self, character: usize) -> usize {
        self.text
            .char_indices()
            .nth(character)
            .map_or(self.text.len(), |(byte, _)| byte)
    }

    fn prepare_edit(&mut self) {
        if self.select_all {
            self.text.clear();
            self.caret = 0;
            self.select_all = false;
        }
    }

    fn insert(&mut self, character: char) {
        self.prepare_edit();
        let byte = self.byte_at(self.caret);
        self.text.insert(byte, character);
        self.caret += 1;
    }

    fn backspace(&mut self) {
        if self.select_all {
            self.text.clear();
            self.caret = 0;
            self.select_all = false;
        } else if self.caret > 0 {
            let end = self.byte_at(self.caret);
            let start = self.byte_at(self.caret - 1);
            self.text.replace_range(start..end, "");
            self.caret -= 1;
        }
    }

    fn delete(&mut self) {
        if self.select_all {
            self.text.clear();
            self.caret = 0;
            self.select_all = false;
        } else if self.caret < self.text.chars().count() {
            let start = self.byte_at(self.caret);
            let end = self.byte_at(self.caret + 1);
            self.text.replace_range(start..end, "");
        }
    }

    fn move_left(&mut self) {
        if self.select_all {
            self.caret = 0;
            self.select_all = false;
        } else {
            self.caret = self.caret.saturating_sub(1);
        }
    }

    fn move_right(&mut self) {
        let end = self.text.chars().count();
        if self.select_all {
            self.caret = end;
            self.select_all = false;
        } else {
            self.caret = self.caret.saturating_add(1).min(end);
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct PathEntryState {
    request: ConsolePathRequest,
    edit: TextEdit,
    pending_scenario: Option<PathBuf>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ConsoleMenuTabLayout {
    pub menu: ConsoleMenu,
    pub rect: IntRect,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ConsoleMenuEntryLayout {
    pub index: usize,
    pub rect: IntRect,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ConsoleCompletionEntryLayout {
    /// Index into `ConsoleViewModel::completions`.
    pub model_index: usize,
    pub rect: IntRect,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DeveloperConsoleLayout {
    pub menu_bar: IntRect,
    pub menu_tabs: Vec<ConsoleMenuTabLayout>,
    pub log: IntRect,
    pub input: IntRect,
    pub cursor: IntRect,
    pub play: IntRect,
    pub halt: IntRect,
    pub mode_play: IntRect,
    pub mode_edit: IntRect,
    pub mode_draw: IntRect,
    pub status_frame: IntRect,
    pub status_script: IntRect,
    pub status_time: IntRect,
    pub dropdown: Vec<ConsoleMenuEntryLayout>,
    pub completion_dropdown: Vec<ConsoleCompletionEntryLayout>,
    pub path_dialog: Option<IntRect>,
    pub path_input: Option<IntRect>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
enum HitTarget {
    MenuTab(ConsoleMenu),
    MenuItem(usize),
    CompletionItem(usize),
    Play,
    Halt,
    Mode(ConsoleEditMode),
    CommandInput,
    PathInput,
    Log,
}

#[derive(Clone, Debug)]
pub struct DeveloperConsole {
    view: ConsoleViewModel,
    status: ConsoleStatusBars,
    log: ConsoleLogBuffer,
    command: TextEdit,
    path_entry: Option<PathEntryState>,
    open_menu: Option<ConsoleMenu>,
    menu_selection: Option<usize>,
    completion_selection: Option<usize>,
    pointer: Option<GuiPoint>,
    pressed: Option<HitTarget>,
    command_focused: bool,
    log_scroll_lines: usize,
    next_path_token: u64,
    revision: u64,
}

impl Default for DeveloperConsole {
    fn default() -> Self {
        Self::new()
    }
}

impl DeveloperConsole {
    pub fn new() -> Self {
        Self {
            view: ConsoleViewModel::default(),
            status: ConsoleStatusBars::default(),
            log: ConsoleLogBuffer::default(),
            command: TextEdit::new(String::new()),
            path_entry: None,
            open_menu: None,
            menu_selection: None,
            completion_selection: None,
            pointer: None,
            pressed: None,
            command_focused: true,
            log_scroll_lines: 0,
            next_path_token: 1,
            revision: 0,
        }
    }

    pub fn view_model(&self) -> &ConsoleViewModel {
        &self.view
    }

    pub fn set_view_model(&mut self, view: ConsoleViewModel) -> bool {
        let status_changed = self.status.update(
            view.frame,
            view.script_counter,
            view.time_seconds,
            view.frames_per_second,
            &view.strings,
        );
        let view_changed = self.view != view;
        self.view = view;
        if self
            .completion_selection
            .is_some_and(|selection| !self.completion_match_indices().contains(&selection))
        {
            self.completion_selection = None;
        }
        if self.open_menu == Some(ConsoleMenu::Net) && !self.net_menu_visible() {
            self.open_menu = None;
            self.menu_selection = None;
        }
        if status_changed || view_changed {
            self.bump_revision();
            true
        } else {
            false
        }
    }

    pub fn status(&self) -> &ConsoleStatusBars {
        &self.status
    }

    pub fn log(&self) -> &ConsoleLogBuffer {
        &self.log
    }

    pub fn out(&mut self, text: &str) -> bool {
        if self.log.out(text) {
            self.log_scroll_lines = 0;
            self.bump_revision();
            true
        } else {
            false
        }
    }

    pub fn clear_log(&mut self) -> bool {
        if self.log.clear() {
            self.log_scroll_lines = 0;
            self.bump_revision();
            true
        } else {
            false
        }
    }

    pub const fn revision(&self) -> u64 {
        self.revision
    }

    pub fn command_text(&self) -> &str {
        &self.command.text
    }

    /// Native GTK enables the script entry for either an open round or the
    /// network lobby. The app projection may report both for a lobby, but the
    /// explicit lobby bit keeps the frontend contract faithful on its own.
    pub fn command_input_enabled(&self) -> bool {
        self.view.game_open || self.view.lobby_active
    }

    pub fn set_command_text(&mut self, text: impl Into<String>) {
        self.command = TextEdit::new(text.into());
        self.completion_selection = None;
        self.command_focused = true;
        self.bump_revision();
    }

    /// Completion model indices matching the current entry, in the exact
    /// app-projected GTK ordering. GTK's default entry completion is an
    /// ASCII-case-insensitive prefix match for C4 identifier names.
    pub fn completion_match_indices(&self) -> Vec<usize> {
        if !self.command_input_enabled()
            || !self.command_focused
            || self.path_entry.is_some()
            || self.open_menu.is_some()
            || self.command.select_all
            || self.command.text.is_empty()
        {
            return Vec::new();
        }
        self.view
            .completions
            .iter()
            .enumerate()
            .filter_map(|(index, completion)| {
                ascii_prefix_matches(completion, &self.command.text).then_some(index)
            })
            .collect()
    }

    pub fn selected_completion(&self) -> Option<&str> {
        self.completion_selection
            .and_then(|index| self.view.completions.get(index))
            .map(String::as_str)
    }

    fn visible_completion_indices(&self) -> Vec<usize> {
        const MAX_VISIBLE: usize = 7;
        let matches = self.completion_match_indices();
        if matches.len() <= MAX_VISIBLE {
            return matches;
        }
        let selected_position = self
            .completion_selection
            .and_then(|selection| matches.iter().position(|index| *index == selection))
            .unwrap_or(0);
        let start = selected_position
            .saturating_sub(MAX_VISIBLE - 1)
            .min(matches.len() - MAX_VISIBLE);
        matches[start..start + MAX_VISIBLE].to_vec()
    }

    fn move_completion_selection(&mut self, delta: isize) -> bool {
        let matches = self.completion_match_indices();
        if matches.is_empty() {
            self.completion_selection = None;
            return false;
        }
        let next = match self
            .completion_selection
            .and_then(|selection| matches.iter().position(|index| *index == selection))
        {
            Some(position) => (position as isize + delta)
                .clamp(0, matches.len().saturating_sub(1) as isize)
                as usize,
            None if delta < 0 => matches.len() - 1,
            None => 0,
        };
        self.completion_selection = Some(matches[next]);
        self.bump_revision();
        true
    }

    fn accept_completion(&mut self, fallback_to_first: bool) -> bool {
        let selection = self.completion_selection.or_else(|| {
            fallback_to_first
                .then(|| self.completion_match_indices().into_iter().next())
                .flatten()
        });
        let Some(completion) = selection
            .and_then(|index| self.view.completions.get(index))
            .cloned()
        else {
            return false;
        };
        self.command = TextEdit::new(completion);
        self.completion_selection = None;
        self.command_focused = true;
        self.bump_revision();
        true
    }

    pub fn path_entry(&self) -> Option<ConsolePathEntryView> {
        self.path_entry.as_ref().map(|entry| ConsolePathEntryView {
            request: entry.request.clone(),
            text: entry.edit.text.clone(),
            caret: entry.edit.caret,
        })
    }

    pub fn open_menu(&self) -> Option<ConsoleMenu> {
        self.open_menu
    }

    pub fn menu_order(&self) -> Vec<ConsoleMenu> {
        let mut menus = vec![
            ConsoleMenu::File,
            ConsoleMenu::Components,
            ConsoleMenu::Player,
            ConsoleMenu::Viewport,
        ];
        if self.net_menu_visible() {
            menus.push(ConsoleMenu::Net);
        }
        menus.push(ConsoleMenu::Help);
        menus
    }

    fn net_menu_visible(&self) -> bool {
        self.view.network_enabled && self.view.network_host
    }

    pub fn menu_entries(&self, menu: ConsoleMenu) -> Vec<ConsoleMenuEntry> {
        let enabled = self.view.game_open;
        let editing = enabled && self.view.editing;
        match menu {
            ConsoleMenu::File => vec![
                item(
                    ConsoleMenuItemId::Open,
                    self.view.strings.file_open.clone(),
                    true,
                ),
                item(
                    ConsoleMenuItemId::OpenWithPlayers,
                    self.view.strings.file_open_with_players.clone(),
                    true,
                ),
                ConsoleMenuEntry::Separator,
                item(
                    ConsoleMenuItemId::SaveScenario,
                    self.view.strings.file_save_scenario.clone(),
                    enabled,
                ),
                item(
                    ConsoleMenuItemId::SaveScenarioAs,
                    self.view.strings.file_save_scenario_as.clone(),
                    enabled,
                ),
                item(
                    ConsoleMenuItemId::SaveGame,
                    self.view.strings.file_save_game.clone(),
                    enabled && !self.view.players.is_empty(),
                ),
                item(
                    ConsoleMenuItemId::SaveGameAs,
                    self.view.strings.file_save_game_as.clone(),
                    enabled && !self.view.players.is_empty(),
                ),
                item(
                    ConsoleMenuItemId::Record,
                    self.view.strings.file_record.clone(),
                    enabled && self.view.runtime_record_possible,
                ),
                ConsoleMenuEntry::Separator,
                item(
                    ConsoleMenuItemId::Close,
                    self.view.strings.file_close.clone(),
                    enabled,
                ),
                ConsoleMenuEntry::Separator,
                item(
                    ConsoleMenuItemId::Quit,
                    self.view.strings.file_quit.clone(),
                    true,
                ),
            ],
            ConsoleMenu::Components => vec![
                item(
                    ConsoleMenuItemId::Objects,
                    self.view.strings.component_objects.clone(),
                    editing,
                ),
                item(
                    ConsoleMenuItemId::Script,
                    self.view.strings.component_script.clone(),
                    editing,
                ),
                item(
                    ConsoleMenuItemId::Title,
                    self.view.strings.component_title.clone(),
                    editing,
                ),
                item(
                    ConsoleMenuItemId::Info,
                    self.view.strings.component_info.clone(),
                    editing,
                ),
            ],
            ConsoleMenu::Player => {
                let mut rows = vec![item(
                    ConsoleMenuItemId::JoinPlayer,
                    self.view.strings.player_join.clone(),
                    editing,
                )];
                rows.extend(self.view.players.iter().map(|player| {
                    item(
                        ConsoleMenuItemId::QuitPlayer(player.number),
                        player.quit_label.clone(),
                        editing && player.quit_enabled,
                    )
                }));
                rows
            }
            ConsoleMenu::Viewport => {
                let mut rows = vec![item(
                    ConsoleMenuItemId::NewViewport,
                    self.view.strings.viewport_new.clone(),
                    enabled,
                )];
                rows.extend(self.view.players.iter().map(|player| {
                    item(
                        ConsoleMenuItemId::NewPlayerViewport(player.number),
                        player.viewport_label.clone(),
                        enabled,
                    )
                }));
                rows
            }
            ConsoleMenu::Net => {
                if !self.net_menu_visible() {
                    return Vec::new();
                }
                self.view
                    .clients
                    .iter()
                    .map(|client| {
                        item(
                            ConsoleMenuItemId::NetClient(client.id),
                            client.menu_label.clone(),
                            client.menu_enabled,
                        )
                    })
                    .collect()
            }
            ConsoleMenu::Help => vec![item(
                ConsoleMenuItemId::About,
                self.view.strings.help_about.clone(),
                true,
            )],
        }
    }

    pub fn activate_menu_item(&mut self, id: &ConsoleMenuItemId) -> Vec<DeveloperConsoleAction> {
        let enabled = self
            .menu_order()
            .into_iter()
            .flat_map(|menu| self.menu_entries(menu))
            .filter_map(|entry| match entry {
                ConsoleMenuEntry::Item(item) => Some(item),
                ConsoleMenuEntry::Separator => None,
            })
            .find(|item| &item.id == id)
            .is_some_and(|item| item.enabled);
        if !enabled {
            return Vec::new();
        }
        self.open_menu = None;
        self.menu_selection = None;
        self.completion_selection = None;
        self.bump_revision();
        match id {
            ConsoleMenuItemId::Open => {
                self.begin_path_request(ConsolePathPurpose::OpenScenario, None)
            }
            ConsoleMenuItemId::OpenWithPlayers => {
                self.begin_path_request(ConsolePathPurpose::OpenScenarioWithPlayersScenario, None)
            }
            ConsoleMenuItemId::SaveScenario => vec![DeveloperConsoleAction::Save {
                kind: ConsoleSaveKind::Scenario,
                target: None,
            }],
            ConsoleMenuItemId::SaveScenarioAs => self.begin_path_request(
                ConsolePathPurpose::SaveScenarioAs,
                self.view.current_scenario_path.clone(),
            ),
            ConsoleMenuItemId::SaveGame => vec![DeveloperConsoleAction::Save {
                kind: ConsoleSaveKind::Savegame,
                target: None,
            }],
            ConsoleMenuItemId::SaveGameAs => self.begin_path_request(
                ConsolePathPurpose::SaveGameAs,
                self.view.current_scenario_path.clone(),
            ),
            ConsoleMenuItemId::Record => vec![DeveloperConsoleAction::RequestRuntimeRecord],
            ConsoleMenuItemId::Close => vec![DeveloperConsoleAction::CloseGame],
            ConsoleMenuItemId::Quit => vec![DeveloperConsoleAction::QuitApplication],
            ConsoleMenuItemId::Objects => vec![DeveloperConsoleAction::EditObjects],
            ConsoleMenuItemId::Script => vec![DeveloperConsoleAction::EditScript],
            ConsoleMenuItemId::Title => vec![DeveloperConsoleAction::EditTitle],
            ConsoleMenuItemId::Info => vec![DeveloperConsoleAction::EditInfo],
            ConsoleMenuItemId::JoinPlayer => {
                self.begin_path_request(ConsolePathPurpose::JoinPlayers, None)
            }
            ConsoleMenuItemId::QuitPlayer(player) => {
                vec![DeveloperConsoleAction::EliminatePlayer(*player)]
            }
            ConsoleMenuItemId::NewViewport => {
                vec![DeveloperConsoleAction::NewViewport(None)]
            }
            ConsoleMenuItemId::NewPlayerViewport(player) => {
                vec![DeveloperConsoleAction::NewViewport(Some(*player))]
            }
            ConsoleMenuItemId::NetClient(client) => {
                vec![DeveloperConsoleAction::KickClient(*client)]
            }
            ConsoleMenuItemId::About => vec![DeveloperConsoleAction::ShowAbout],
        }
    }

    fn next_request_token(&mut self) -> u64 {
        let token = self.next_path_token.max(1);
        self.next_path_token = token.wrapping_add(1).max(1);
        token
    }

    fn begin_path_request(
        &mut self,
        purpose: ConsolePathPurpose,
        suggested_path: Option<PathBuf>,
    ) -> Vec<DeveloperConsoleAction> {
        let token = self.next_request_token();
        let (title, prompt, filter_label, allow_multiple, save, extensions) =
            path_request_description(purpose, &self.view.strings);
        let request = ConsolePathRequest {
            token,
            purpose,
            title: title.to_owned(),
            prompt: prompt.to_owned(),
            allow_multiple,
            save,
            suggested_path: suggested_path.clone(),
            filter_label: filter_label.to_owned(),
            extensions,
        };
        let text = suggested_path
            .as_ref()
            .map(|path| path.to_string_lossy().into_owned())
            .unwrap_or_default();
        let mut edit = TextEdit::new(text);
        edit.select_all = suggested_path.is_some();
        self.path_entry = Some(PathEntryState {
            request: request.clone(),
            edit,
            pending_scenario: None,
        });
        self.completion_selection = None;
        self.command_focused = false;
        self.bump_revision();
        vec![DeveloperConsoleAction::RequestPath(request)]
    }

    pub fn respond_path_request(
        &mut self,
        token: u64,
        paths: Vec<PathBuf>,
    ) -> Vec<DeveloperConsoleAction> {
        let Some(entry) = self.path_entry.as_ref() else {
            return Vec::new();
        };
        if entry.request.token != token {
            return Vec::new();
        }
        // `rfd` only exposes extension filters, so the native picker must use
        // `txt` to keep the classic exact `Scenario.txt` choice discoverable.
        // Reject every other text file before it can close the current round.
        if !paths.is_empty() && !path_response_is_valid(entry.request.purpose, &paths) {
            return Vec::new();
        }
        let entry = self
            .path_entry
            .take()
            .expect("validated developer-console path request remains active");
        self.command_focused = true;
        self.bump_revision();
        if paths.is_empty() {
            return Vec::new();
        }
        match entry.request.purpose {
            ConsolePathPurpose::OpenScenario => vec![DeveloperConsoleAction::OpenGame {
                scenario: paths[0].clone(),
                player_files: Vec::new(),
            }],
            ConsolePathPurpose::OpenScenarioWithPlayersScenario => {
                let scenario = paths[0].clone();
                let actions = self
                    .begin_path_request(ConsolePathPurpose::OpenScenarioWithPlayersPlayers, None);
                if let Some(next) = self.path_entry.as_mut() {
                    next.pending_scenario = Some(scenario);
                }
                actions
            }
            ConsolePathPurpose::OpenScenarioWithPlayersPlayers => {
                let Some(scenario) = entry.pending_scenario else {
                    return Vec::new();
                };
                vec![DeveloperConsoleAction::OpenGame {
                    scenario,
                    player_files: paths,
                }]
            }
            ConsolePathPurpose::SaveScenarioAs => vec![DeveloperConsoleAction::Save {
                kind: ConsoleSaveKind::Scenario,
                target: Some(paths[0].clone()),
            }],
            ConsolePathPurpose::SaveGameAs => vec![DeveloperConsoleAction::Save {
                kind: ConsoleSaveKind::Savegame,
                target: Some(paths[0].clone()),
            }],
            ConsolePathPurpose::JoinPlayers => {
                vec![DeveloperConsoleAction::JoinPlayers(paths)]
            }
        }
    }

    pub fn cancel_path_request(&mut self, token: u64) -> bool {
        if self
            .path_entry
            .as_ref()
            .is_some_and(|entry| entry.request.token == token)
        {
            self.path_entry = None;
            self.command_focused = true;
            self.bump_revision();
            true
        } else {
            false
        }
    }

    fn submit_path_entry(&mut self) -> Vec<DeveloperConsoleAction> {
        let Some(entry) = self.path_entry.as_ref() else {
            return Vec::new();
        };
        let token = entry.request.token;
        let paths = parse_path_list(&entry.edit.text);
        if paths.is_empty() {
            return Vec::new();
        }
        self.respond_path_request(token, paths)
    }

    pub fn handle_character(&mut self, character: char) -> bool {
        if character.is_control() {
            return false;
        }
        if let Some(entry) = self.path_entry.as_mut() {
            entry.edit.insert(character);
        } else if self.command_input_enabled() {
            self.command.insert(character);
            self.completion_selection = None;
            self.command_focused = true;
        } else {
            return false;
        }
        self.bump_revision();
        true
    }

    pub fn handle_key(
        &mut self,
        key: DeveloperConsoleKey,
        pressed: bool,
    ) -> Vec<DeveloperConsoleAction> {
        if !pressed {
            return Vec::new();
        }
        if key == DeveloperConsoleKey::Pause && self.path_entry.is_none() && self.view.game_open {
            return vec![DeveloperConsoleAction::TogglePause];
        }
        if self.path_entry.is_some() {
            match key {
                DeveloperConsoleKey::Enter => return self.submit_path_entry(),
                DeveloperConsoleKey::Escape => {
                    let token = self.path_entry.as_ref().unwrap().request.token;
                    self.cancel_path_request(token);
                }
                DeveloperConsoleKey::Backspace => {
                    self.path_entry.as_mut().unwrap().edit.backspace();
                    self.bump_revision();
                }
                DeveloperConsoleKey::Delete => {
                    self.path_entry.as_mut().unwrap().edit.delete();
                    self.bump_revision();
                }
                DeveloperConsoleKey::Left => {
                    self.path_entry.as_mut().unwrap().edit.move_left();
                    self.bump_revision();
                }
                DeveloperConsoleKey::Right => {
                    self.path_entry.as_mut().unwrap().edit.move_right();
                    self.bump_revision();
                }
                DeveloperConsoleKey::Home => {
                    self.path_entry.as_mut().unwrap().edit.caret = 0;
                    self.bump_revision();
                }
                DeveloperConsoleKey::End => {
                    let entry = self.path_entry.as_mut().unwrap();
                    entry.edit.caret = entry.edit.text.chars().count();
                    self.bump_revision();
                }
                _ => {}
            }
            return Vec::new();
        }

        if self.open_menu.is_some() {
            match key {
                DeveloperConsoleKey::Escape => {
                    self.open_menu = None;
                    self.menu_selection = None;
                    self.bump_revision();
                    return Vec::new();
                }
                DeveloperConsoleKey::Up => {
                    self.move_menu_selection(-1);
                    return Vec::new();
                }
                DeveloperConsoleKey::Down => {
                    self.move_menu_selection(1);
                    return Vec::new();
                }
                DeveloperConsoleKey::Left => {
                    self.move_open_menu(-1);
                    return Vec::new();
                }
                DeveloperConsoleKey::Right => {
                    self.move_open_menu(1);
                    return Vec::new();
                }
                DeveloperConsoleKey::Enter => return self.activate_selected_menu_item(),
                _ => {}
            }
        }

        if !self.command_input_enabled() {
            match key {
                DeveloperConsoleKey::PageUp | DeveloperConsoleKey::Up => self.scroll_log(3),
                DeveloperConsoleKey::PageDown | DeveloperConsoleKey::Down => self.scroll_log(-3),
                _ => {}
            }
            return Vec::new();
        }

        match key {
            DeveloperConsoleKey::Enter if self.completion_selection.is_some() => {
                self.accept_completion(false);
                return Vec::new();
            }
            DeveloperConsoleKey::Tab if self.accept_completion(true) => return Vec::new(),
            DeveloperConsoleKey::Up if self.move_completion_selection(-1) => return Vec::new(),
            DeveloperConsoleKey::Down if self.move_completion_selection(1) => return Vec::new(),
            _ => {}
        }

        match key {
            DeveloperConsoleKey::Enter => {
                if self.command.text.is_empty() {
                    Vec::new()
                } else {
                    let command = self.command.text.clone();
                    self.command.select_all = true;
                    self.completion_selection = None;
                    self.bump_revision();
                    vec![DeveloperConsoleAction::SubmitInput(command)]
                }
            }
            DeveloperConsoleKey::Backspace => {
                self.command.backspace();
                self.completion_selection = None;
                self.bump_revision();
                Vec::new()
            }
            DeveloperConsoleKey::Delete => {
                self.command.delete();
                self.completion_selection = None;
                self.bump_revision();
                Vec::new()
            }
            DeveloperConsoleKey::Left => {
                self.command.move_left();
                self.completion_selection = None;
                self.bump_revision();
                Vec::new()
            }
            DeveloperConsoleKey::Right => {
                self.command.move_right();
                self.completion_selection = None;
                self.bump_revision();
                Vec::new()
            }
            DeveloperConsoleKey::Home => {
                self.command.caret = 0;
                self.command.select_all = false;
                self.completion_selection = None;
                self.bump_revision();
                Vec::new()
            }
            DeveloperConsoleKey::End => {
                self.command.caret = self.command.text.chars().count();
                self.command.select_all = false;
                self.completion_selection = None;
                self.bump_revision();
                Vec::new()
            }
            DeveloperConsoleKey::PageUp | DeveloperConsoleKey::Up => {
                self.scroll_log(3);
                Vec::new()
            }
            DeveloperConsoleKey::PageDown | DeveloperConsoleKey::Down => {
                self.scroll_log(-3);
                Vec::new()
            }
            _ => Vec::new(),
        }
    }

    pub fn handle_menu_mnemonic(&mut self, character: char) -> bool {
        let target = match character.to_ascii_lowercase() {
            'f' => Some(ConsoleMenu::File),
            'c' => Some(ConsoleMenu::Components),
            'p' => Some(ConsoleMenu::Player),
            'v' => Some(ConsoleMenu::Viewport),
            'n' if self.net_menu_visible() => Some(ConsoleMenu::Net),
            'h' => Some(ConsoleMenu::Help),
            _ => None,
        };
        if let Some(menu) = target {
            self.open_menu = Some(menu);
            self.menu_selection = first_enabled_index(&self.menu_entries(menu));
            self.completion_selection = None;
            self.bump_revision();
            true
        } else {
            false
        }
    }

    fn move_menu_selection(&mut self, delta: isize) {
        let Some(menu) = self.open_menu else {
            return;
        };
        let entries = self.menu_entries(menu);
        let enabled = entries
            .iter()
            .enumerate()
            .filter_map(|(index, entry)| entry.item().filter(|item| item.enabled).map(|_| index))
            .collect::<Vec<_>>();
        if enabled.is_empty() {
            self.menu_selection = None;
            return;
        }
        let current = self
            .menu_selection
            .and_then(|selection| enabled.iter().position(|index| *index == selection));
        let next = match current {
            None if delta < 0 => enabled.len() - 1,
            None => 0,
            Some(current) => (current as isize + delta).rem_euclid(enabled.len() as isize) as usize,
        };
        self.menu_selection = Some(enabled[next]);
        self.bump_revision();
    }

    fn move_open_menu(&mut self, delta: isize) {
        let Some(menu) = self.open_menu else {
            return;
        };
        let menus = self.menu_order();
        let current = menus
            .iter()
            .position(|candidate| *candidate == menu)
            .unwrap_or(0);
        let next = (current as isize + delta).rem_euclid(menus.len() as isize) as usize;
        self.open_menu = Some(menus[next]);
        self.menu_selection = first_enabled_index(&self.menu_entries(menus[next]));
        self.bump_revision();
    }

    fn activate_selected_menu_item(&mut self) -> Vec<DeveloperConsoleAction> {
        let (Some(menu), Some(index)) = (self.open_menu, self.menu_selection) else {
            return Vec::new();
        };
        let Some(id) = self
            .menu_entries(menu)
            .get(index)
            .and_then(ConsoleMenuEntry::item)
            .filter(|item| item.enabled)
            .map(|item| item.id.clone())
        else {
            return Vec::new();
        };
        self.activate_menu_item(&id)
    }

    pub fn scroll_log(&mut self, lines_from_bottom_delta: i32) {
        let total = self.log.text.lines().count();
        if lines_from_bottom_delta >= 0 {
            self.log_scroll_lines = self
                .log_scroll_lines
                .saturating_add(lines_from_bottom_delta as usize)
                .min(total.saturating_sub(1));
        } else {
            self.log_scroll_lines = self
                .log_scroll_lines
                .saturating_sub(lines_from_bottom_delta.unsigned_abs() as usize);
        }
        self.bump_revision();
    }

    pub fn layout(&self, width: u32, height: u32) -> DeveloperConsoleLayout {
        let width = i32::try_from(width).unwrap_or(i32::MAX).max(1);
        let height = i32::try_from(height).unwrap_or(i32::MAX).max(1);
        let menu_bar = IntRect {
            x: 0,
            y: 0,
            w: width,
            h: MENU_BAR_HEIGHT.min(height),
        };
        let mut x = 2;
        let menu_tabs = self
            .menu_order()
            .into_iter()
            .map(|menu| {
                let rect = IntRect {
                    x,
                    y: 1,
                    w: menu.tab_width(&self.view.strings),
                    h: MENU_BAR_HEIGHT - 2,
                };
                x += rect.w;
                ConsoleMenuTabLayout { menu, rect }
            })
            .collect::<Vec<_>>();

        let status_y = (height - STATUS_BAR_HEIGHT).max(MENU_BAR_HEIGHT);
        let toolbar_y = (status_y - TOOLBAR_HEIGHT).max(MENU_BAR_HEIGHT);
        let input_y = (toolbar_y - INPUT_HEIGHT).max(MENU_BAR_HEIGHT);
        let log_y = MENU_BAR_HEIGHT + WINDOW_PADDING;
        let log = IntRect {
            x: WINDOW_PADDING,
            y: log_y,
            w: (width - WINDOW_PADDING * 2).max(1),
            h: (input_y - log_y - WINDOW_PADDING).max(1),
        };
        let input = IntRect {
            x: WINDOW_PADDING,
            y: input_y + 2,
            w: (width - WINDOW_PADDING * 2).max(1),
            h: (INPUT_HEIGHT - 4).max(1),
        };

        let button_gap = 3;
        let button_width = ((width - 2 * WINDOW_PADDING - 4 * button_gap) / 6).clamp(34, 54);
        let total_buttons = button_width * 5 + button_gap * 4;
        let buttons_x = (width - WINDOW_PADDING - total_buttons).max(WINDOW_PADDING);
        let button_y = toolbar_y + 4;
        let button_height = (TOOLBAR_HEIGHT - 8).max(1);
        let button = |index: i32| IntRect {
            x: buttons_x + index * (button_width + button_gap),
            y: button_y,
            w: button_width,
            h: button_height,
        };
        let cursor = IntRect {
            x: WINDOW_PADDING,
            y: button_y,
            w: (buttons_x - WINDOW_PADDING * 2).max(1),
            h: button_height,
        };

        let status_width = width / 3;
        let status_frame = IntRect {
            x: 0,
            y: status_y,
            w: status_width,
            h: height - status_y,
        };
        let status_script = IntRect {
            x: status_width,
            y: status_y,
            w: status_width,
            h: height - status_y,
        };
        let status_time = IntRect {
            x: status_width * 2,
            y: status_y,
            w: width - status_width * 2,
            h: height - status_y,
        };

        let mut dropdown = Vec::new();
        if let Some(menu) = self.open_menu {
            let entries = self.menu_entries(menu);
            let max_chars = entries
                .iter()
                .filter_map(ConsoleMenuEntry::item)
                .map(|item| item.label.chars().count())
                .max()
                .unwrap_or(12);
            let dropdown_width = (max_chars as i32 * 8 + 30).clamp(180, width);
            let tab_x = menu_tabs
                .iter()
                .find(|tab| tab.menu == menu)
                .map_or(0, |tab| tab.rect.x);
            let dropdown_x = tab_x.min((width - dropdown_width).max(0));
            let mut y = MENU_BAR_HEIGHT;
            for (index, entry) in entries.iter().enumerate() {
                let h = if matches!(entry, ConsoleMenuEntry::Separator) {
                    MENU_SEPARATOR_HEIGHT
                } else {
                    MENU_ITEM_HEIGHT
                };
                dropdown.push(ConsoleMenuEntryLayout {
                    index,
                    rect: IntRect {
                        x: dropdown_x,
                        y,
                        w: dropdown_width,
                        h,
                    },
                });
                y += h;
            }
        }

        let completion_indices = self.visible_completion_indices();
        let completion_height = completion_indices.len() as i32 * MENU_ITEM_HEIGHT;
        let completion_y = (input.y - completion_height).max(MENU_BAR_HEIGHT);
        let completion_dropdown = completion_indices
            .into_iter()
            .enumerate()
            .map(|(row, model_index)| ConsoleCompletionEntryLayout {
                model_index,
                rect: IntRect {
                    x: input.x,
                    y: completion_y + row as i32 * MENU_ITEM_HEIGHT,
                    w: input.w,
                    h: MENU_ITEM_HEIGHT,
                },
            })
            .collect();

        let (path_dialog, path_input) = if self.path_entry.is_some() {
            let dialog_width = (width - 24).clamp(220, 520).min(width);
            let dialog_height = 118.min(height);
            let dialog = IntRect {
                x: (width - dialog_width) / 2,
                y: (height - dialog_height) / 2,
                w: dialog_width,
                h: dialog_height,
            };
            let input = IntRect {
                x: dialog.x + 12,
                y: dialog.y + 51,
                w: (dialog.w - 24).max(1),
                h: 25,
            };
            (Some(dialog), Some(input))
        } else {
            (None, None)
        };

        DeveloperConsoleLayout {
            menu_bar,
            menu_tabs,
            log,
            input,
            cursor,
            play: button(0),
            halt: button(1),
            mode_play: button(2),
            mode_edit: button(3),
            mode_draw: button(4),
            status_frame,
            status_script,
            status_time,
            dropdown,
            completion_dropdown,
            path_dialog,
            path_input,
        }
    }

    pub fn handle_pointer_move(&mut self, position: GuiPoint) {
        self.pointer = Some(position);
        self.bump_revision();
    }

    pub fn handle_pointer_down(&mut self, position: GuiPoint, width: u32, height: u32) {
        self.pointer = Some(position);
        self.pressed = self.hit_test(position, width, height);
        if matches!(self.pressed, Some(HitTarget::CommandInput)) {
            self.command_focused = true;
        } else if matches!(self.pressed, Some(HitTarget::Log)) {
            self.command_focused = false;
            self.completion_selection = None;
        }
        self.bump_revision();
    }

    pub fn handle_pointer_up(
        &mut self,
        position: GuiPoint,
        width: u32,
        height: u32,
    ) -> Vec<DeveloperConsoleAction> {
        self.pointer = Some(position);
        let released = self.hit_test(position, width, height);
        let pressed = self.pressed.take();
        if pressed != released {
            self.bump_revision();
            return Vec::new();
        }
        let actions = match released {
            Some(HitTarget::MenuTab(menu)) => {
                self.completion_selection = None;
                if self.open_menu == Some(menu) {
                    self.open_menu = None;
                    self.menu_selection = None;
                } else {
                    self.open_menu = Some(menu);
                    self.menu_selection = first_enabled_index(&self.menu_entries(menu));
                }
                Vec::new()
            }
            Some(HitTarget::MenuItem(index)) => {
                let Some(menu) = self.open_menu else {
                    return Vec::new();
                };
                let id = self
                    .menu_entries(menu)
                    .get(index)
                    .and_then(ConsoleMenuEntry::item)
                    .filter(|item| item.enabled)
                    .map(|item| item.id.clone());
                id.map_or_else(Vec::new, |id| self.activate_menu_item(&id))
            }
            Some(HitTarget::CompletionItem(model_index)) => {
                self.completion_selection = Some(model_index);
                self.accept_completion(false);
                Vec::new()
            }
            Some(HitTarget::Play) if self.play_controls_enabled() => {
                vec![DeveloperConsoleAction::Play]
            }
            Some(HitTarget::Halt) if self.play_controls_enabled() => {
                vec![DeveloperConsoleAction::Halt]
            }
            Some(HitTarget::Mode(mode)) if self.mode_enabled(mode) => {
                vec![DeveloperConsoleAction::SetEditMode(mode)]
            }
            Some(HitTarget::CommandInput) | Some(HitTarget::PathInput) | Some(HitTarget::Log) => {
                Vec::new()
            }
            None => {
                if self.path_entry.is_none() {
                    self.open_menu = None;
                    self.menu_selection = None;
                }
                Vec::new()
            }
            _ => Vec::new(),
        };
        self.bump_revision();
        actions
    }

    fn hit_test(&self, point: GuiPoint, width: u32, height: u32) -> Option<HitTarget> {
        let layout = self.layout(width, height);
        if self.path_entry.is_some() {
            return layout
                .path_input
                .filter(|rect| contains(*rect, point))
                .map(|_| HitTarget::PathInput);
        }
        for row in &layout.completion_dropdown {
            if contains(row.rect, point) {
                return Some(HitTarget::CompletionItem(row.model_index));
            }
        }
        for row in &layout.dropdown {
            if contains(row.rect, point) {
                return Some(HitTarget::MenuItem(row.index));
            }
        }
        for tab in &layout.menu_tabs {
            if contains(tab.rect, point) {
                return Some(HitTarget::MenuTab(tab.menu));
            }
        }
        for (rect, target) in [
            (layout.play, HitTarget::Play),
            (layout.halt, HitTarget::Halt),
            (layout.mode_play, HitTarget::Mode(ConsoleEditMode::Play)),
            (layout.mode_edit, HitTarget::Mode(ConsoleEditMode::Edit)),
            (layout.mode_draw, HitTarget::Mode(ConsoleEditMode::Draw)),
        ] {
            if contains(rect, point) {
                return Some(target);
            }
        }
        if self.command_input_enabled() && contains(layout.input, point) {
            Some(HitTarget::CommandInput)
        } else if contains(layout.log, point) {
            Some(HitTarget::Log)
        } else {
            None
        }
    }

    fn play_controls_enabled(&self) -> bool {
        self.view.game_open || self.view.lobby_active
    }

    fn mode_enabled(&self, mode: ConsoleEditMode) -> bool {
        // The GTK console explicitly enables all three mode buttons while a
        // network lobby is active, even though `fGameOpen` is still false.
        if self.view.lobby_active {
            return true;
        }
        match mode {
            ConsoleEditMode::Play => self.view.game_open,
            ConsoleEditMode::Edit | ConsoleEditMode::Draw => {
                self.view.game_open && self.view.editing
            }
        }
    }

    pub fn render(&self, surface: &mut Surface, font: &dyn TextFont) {
        surface.fill(WINDOW_BACKGROUND);
        let layout = self.layout(surface.width(), surface.height());
        fill(surface, layout.menu_bar, WINDOW_BACKGROUND);
        draw_bottom_line(surface, layout.menu_bar, MID_EDGE);

        for tab in &layout.menu_tabs {
            let selected = self.open_menu == Some(tab.menu);
            if selected || self.hovered(HitTarget::MenuTab(tab.menu), surface) {
                fill(
                    surface,
                    tab.rect,
                    if selected {
                        SELECTED_BACKGROUND
                    } else {
                        LIGHT_EDGE
                    },
                );
            }
            draw_fitted_text(
                surface,
                font,
                tab.rect,
                tab.menu.label(&self.view.strings),
                if selected {
                    SELECTED_TEXT
                } else {
                    CONTROL_TEXT
                },
                FONT_SIZE,
                6,
            );
        }

        draw_sunken(surface, layout.log, CONTROL_BACKGROUND);
        self.render_log(surface, font, layout.log);
        draw_sunken(
            surface,
            layout.input,
            if self.command_input_enabled() {
                CONTROL_BACKGROUND
            } else {
                WINDOW_BACKGROUND
            },
        );
        self.render_text_edit(
            surface,
            font,
            layout.input,
            &self.command,
            self.command_focused && self.path_entry.is_none() && self.command_input_enabled(),
            self.command_input_enabled(),
        );

        draw_fitted_text(
            surface,
            font,
            layout.cursor,
            &self.view.cursor_text,
            CONTROL_TEXT,
            FONT_SIZE,
            2,
        );
        self.render_button(
            surface,
            font,
            layout.play,
            &self.view.strings.tool_play,
            !self.view.halted,
            self.play_controls_enabled(),
            HitTarget::Play,
        );
        self.render_button(
            surface,
            font,
            layout.halt,
            &self.view.strings.tool_halt,
            self.view.halted,
            self.play_controls_enabled(),
            HitTarget::Halt,
        );
        for (mode, rect, label) in [
            (
                ConsoleEditMode::Play,
                layout.mode_play,
                self.view.strings.tool_mode_play.as_str(),
            ),
            (
                ConsoleEditMode::Edit,
                layout.mode_edit,
                self.view.strings.tool_mode_edit.as_str(),
            ),
            (
                ConsoleEditMode::Draw,
                layout.mode_draw,
                self.view.strings.tool_mode_draw.as_str(),
            ),
        ] {
            self.render_button(
                surface,
                font,
                rect,
                label,
                self.view.edit_mode == mode,
                self.mode_enabled(mode),
                HitTarget::Mode(mode),
            );
        }

        for (rect, text) in [
            (layout.status_frame, self.status.frame_text()),
            (layout.status_script, self.status.script_text()),
            (layout.status_time, self.status.time_text()),
        ] {
            draw_sunken(surface, rect, WINDOW_BACKGROUND);
            draw_fitted_text(surface, font, rect, text, CONTROL_TEXT, SMALL_FONT_SIZE, 5);
        }

        self.render_completion_dropdown(surface, font, &layout.completion_dropdown);
        if let Some(menu) = self.open_menu {
            self.render_dropdown(surface, font, menu, &layout.dropdown);
        }
        if let (Some(entry), Some(dialog), Some(input)) = (
            self.path_entry.as_ref(),
            layout.path_dialog,
            layout.path_input,
        ) {
            draw_raised(surface, dialog, WINDOW_BACKGROUND);
            draw_fitted_text(
                surface,
                font,
                IntRect {
                    x: dialog.x + 10,
                    y: dialog.y + 8,
                    w: dialog.w - 20,
                    h: 20,
                },
                &entry.request.title,
                CONTROL_TEXT,
                FONT_SIZE,
                1,
            );
            draw_fitted_text(
                surface,
                font,
                IntRect {
                    x: dialog.x + 10,
                    y: dialog.y + 29,
                    w: dialog.w - 20,
                    h: 18,
                },
                &entry.request.prompt,
                CONTROL_TEXT,
                SMALL_FONT_SIZE,
                1,
            );
            draw_sunken(surface, input, CONTROL_BACKGROUND);
            self.render_text_edit(surface, font, input, &entry.edit, true, true);
            draw_fitted_text(
                surface,
                font,
                IntRect {
                    x: dialog.x + 10,
                    y: dialog.y + 82,
                    w: dialog.w - 20,
                    h: 24,
                },
                &self.view.strings.path_entry_hint,
                DISABLED_TEXT,
                SMALL_FONT_SIZE,
                1,
            );
        }
    }

    fn render_log(&self, surface: &mut Surface, font: &dyn TextFont, rect: IntRect) {
        let line_height = 15;
        let max_lines = ((rect.h - 6) / line_height).max(0) as usize;
        let lines = self
            .log
            .text()
            .lines()
            .map(|line| line.trim_end_matches('\r'))
            .collect::<Vec<_>>();
        let end = lines.len().saturating_sub(self.log_scroll_lines);
        let start = end.saturating_sub(max_lines);
        for (row, line) in lines[start..end].iter().enumerate() {
            draw_fitted_text(
                surface,
                font,
                IntRect {
                    x: rect.x + 3,
                    y: rect.y + 3 + row as i32 * line_height,
                    w: rect.w - 6,
                    h: line_height,
                },
                line,
                CONTROL_TEXT,
                SMALL_FONT_SIZE,
                1,
            );
        }
    }

    fn render_text_edit(
        &self,
        surface: &mut Surface,
        font: &dyn TextFont,
        rect: IntRect,
        edit: &TextEdit,
        focused: bool,
        enabled: bool,
    ) {
        if edit.select_all && focused {
            fill(
                surface,
                IntRect {
                    x: rect.x + 2,
                    y: rect.y + 2,
                    w: rect.w - 4,
                    h: rect.h - 4,
                },
                SELECTED_BACKGROUND,
            );
        }
        draw_fitted_text(
            surface,
            font,
            rect,
            &edit.text,
            if !enabled {
                DISABLED_TEXT
            } else if edit.select_all && focused {
                SELECTED_TEXT
            } else {
                CONTROL_TEXT
            },
            FONT_SIZE,
            4,
        );
        if focused && !edit.select_all {
            let prefix = edit.text.chars().take(edit.caret).collect::<String>();
            let caret_x = rect.x + 4 + font.measure_text(&prefix, FONT_SIZE).width.round() as i32;
            fill(
                surface,
                IntRect {
                    x: caret_x.min(rect.x + rect.w - 3),
                    y: rect.y + 4,
                    w: 1,
                    h: (rect.h - 8).max(1),
                },
                CONTROL_TEXT,
            );
        }
    }

    #[allow(clippy::too_many_arguments)]
    fn render_button(
        &self,
        surface: &mut Surface,
        font: &dyn TextFont,
        rect: IntRect,
        label: &str,
        selected: bool,
        enabled: bool,
        target: HitTarget,
    ) {
        let pressed = self.pressed.as_ref() == Some(&target) || selected;
        if pressed {
            draw_sunken(surface, rect, WINDOW_BACKGROUND);
        } else {
            draw_raised(surface, rect, WINDOW_BACKGROUND);
        }
        if self.hovered(target, surface) && enabled && !pressed {
            fill(
                surface,
                IntRect {
                    x: rect.x + 2,
                    y: rect.y + 2,
                    w: rect.w - 4,
                    h: rect.h - 4,
                },
                LIGHT_EDGE,
            );
        }
        draw_fitted_text(
            surface,
            font,
            rect,
            label,
            if enabled { CONTROL_TEXT } else { DISABLED_TEXT },
            SMALL_FONT_SIZE,
            4 + i32::from(pressed),
        );
    }

    fn render_dropdown(
        &self,
        surface: &mut Surface,
        font: &dyn TextFont,
        menu: ConsoleMenu,
        layout: &[ConsoleMenuEntryLayout],
    ) {
        let entries = self.menu_entries(menu);
        if let (Some(first), Some(last)) = (layout.first(), layout.last()) {
            draw_raised(
                surface,
                IntRect {
                    x: first.rect.x,
                    y: first.rect.y,
                    w: first.rect.w,
                    h: last.rect.y + last.rect.h - first.rect.y,
                },
                WINDOW_BACKGROUND,
            );
        }
        for row in layout {
            match &entries[row.index] {
                ConsoleMenuEntry::Separator => {
                    fill(
                        surface,
                        IntRect {
                            x: row.rect.x + 4,
                            y: row.rect.y + row.rect.h / 2,
                            w: row.rect.w - 8,
                            h: 1,
                        },
                        MID_EDGE,
                    );
                }
                ConsoleMenuEntry::Item(item) => {
                    let highlighted = self.menu_selection == Some(row.index)
                        || self.pointer.is_some_and(|point| contains(row.rect, point));
                    if highlighted && item.enabled {
                        fill(surface, row.rect, SELECTED_BACKGROUND);
                    }
                    let label = if item.checked {
                        format!("[x] {}", item.label)
                    } else {
                        item.label.clone()
                    };
                    draw_fitted_text(
                        surface,
                        font,
                        row.rect,
                        &label,
                        if !item.enabled {
                            DISABLED_TEXT
                        } else if highlighted {
                            SELECTED_TEXT
                        } else {
                            CONTROL_TEXT
                        },
                        SMALL_FONT_SIZE,
                        8,
                    );
                }
            }
        }
    }

    fn render_completion_dropdown(
        &self,
        surface: &mut Surface,
        font: &dyn TextFont,
        layout: &[ConsoleCompletionEntryLayout],
    ) {
        let (Some(first), Some(last)) = (layout.first(), layout.last()) else {
            return;
        };
        draw_raised(
            surface,
            IntRect {
                x: first.rect.x,
                y: first.rect.y,
                w: first.rect.w,
                h: last.rect.y + last.rect.h - first.rect.y,
            },
            CONTROL_BACKGROUND,
        );
        for row in layout {
            let selected = self.completion_selection == Some(row.model_index)
                || self.pointer.is_some_and(|point| contains(row.rect, point));
            if selected {
                fill(surface, row.rect, SELECTED_BACKGROUND);
            }
            if let Some(text) = self.view.completions.get(row.model_index) {
                draw_fitted_text(
                    surface,
                    font,
                    row.rect,
                    text,
                    if selected {
                        SELECTED_TEXT
                    } else {
                        CONTROL_TEXT
                    },
                    SMALL_FONT_SIZE,
                    6,
                );
            }
        }
    }

    fn hovered(&self, target: HitTarget, surface: &Surface) -> bool {
        self.pointer.is_some_and(|point| {
            self.hit_test(point, surface.width(), surface.height()) == Some(target)
        })
    }

    fn bump_revision(&mut self) {
        self.revision = self.revision.wrapping_add(1);
    }
}

fn item(id: ConsoleMenuItemId, label: impl Into<String>, enabled: bool) -> ConsoleMenuEntry {
    ConsoleMenuEntry::Item(ConsoleMenuItem::new(id, label, enabled))
}

fn first_enabled_index(entries: &[ConsoleMenuEntry]) -> Option<usize> {
    entries
        .iter()
        .position(|entry| entry.item().is_some_and(|item| item.enabled))
}

fn path_request_description(
    purpose: ConsolePathPurpose,
    strings: &ConsoleStrings,
) -> (&str, &str, &str, bool, bool, Vec<&'static str>) {
    match purpose {
        ConsolePathPurpose::OpenScenario => (
            &strings.path_load_title,
            &strings.path_scenario_prompt,
            &strings.path_scenario_filter,
            false,
            false,
            vec!["c4s", "c4f", "txt"],
        ),
        ConsolePathPurpose::OpenScenarioWithPlayersScenario => (
            &strings.path_load_title,
            &strings.path_scenario_prompt,
            &strings.path_scenario_filter,
            false,
            false,
            vec!["c4s", "c4f"],
        ),
        ConsolePathPurpose::OpenScenarioWithPlayersPlayers => (
            &strings.path_load_title,
            &strings.path_players_prompt,
            &strings.path_player_filter,
            true,
            false,
            vec!["c4p"],
        ),
        ConsolePathPurpose::SaveScenarioAs => (
            &strings.path_save_title,
            &strings.path_target_prompt,
            &strings.path_scenario_filter,
            false,
            true,
            vec!["c4s"],
        ),
        ConsolePathPurpose::SaveGameAs => (
            &strings.path_save_title,
            &strings.path_target_prompt,
            &strings.path_scenario_filter,
            false,
            true,
            vec!["c4s"],
        ),
        ConsolePathPurpose::JoinPlayers => (
            &strings.path_load_title,
            &strings.path_players_prompt,
            &strings.path_player_filter,
            true,
            false,
            vec!["c4p"],
        ),
    }
}

fn path_response_is_valid(purpose: ConsolePathPurpose, paths: &[PathBuf]) -> bool {
    let Some(path) = paths.first() else {
        return true;
    };
    match purpose {
        ConsolePathPurpose::OpenScenario => is_scenario_path(path, true),
        ConsolePathPurpose::OpenScenarioWithPlayersScenario => is_scenario_path(path, false),
        _ => true,
    }
}

fn is_scenario_path(path: &std::path::Path, allow_scenario_txt: bool) -> bool {
    let group_extension = path
        .extension()
        .and_then(|extension| extension.to_str())
        .is_some_and(|extension| {
            extension.eq_ignore_ascii_case("c4s") || extension.eq_ignore_ascii_case("c4f")
        });
    group_extension
        || (allow_scenario_txt
            && path
                .file_name()
                .and_then(|name| name.to_str())
                .is_some_and(|name| name.eq_ignore_ascii_case("Scenario.txt")))
}

fn parse_path_list(input: &str) -> Vec<PathBuf> {
    let mut paths = Vec::new();
    let mut current = String::new();
    let mut quoted = false;
    let push = |current: &mut String, paths: &mut Vec<PathBuf>| {
        let value = current.trim();
        if !value.is_empty() {
            paths.push(PathBuf::from(value));
        }
        current.clear();
    };
    for character in input.chars() {
        match character {
            '"' => quoted = !quoted,
            ';' | '\n' if !quoted => push(&mut current, &mut paths),
            _ => current.push(character),
        }
    }
    push(&mut current, &mut paths);
    paths
}

fn ascii_prefix_matches(candidate: &str, prefix: &str) -> bool {
    candidate
        .get(..prefix.len())
        .is_some_and(|head| head.eq_ignore_ascii_case(prefix))
}

fn contains(rect: IntRect, point: GuiPoint) -> bool {
    point.x >= rect.x as f32
        && point.y >= rect.y as f32
        && point.x < rect.x.saturating_add(rect.w) as f32
        && point.y < rect.y.saturating_add(rect.h) as f32
}

fn gui_rect(rect: IntRect) -> GuiRect {
    GuiRect::new(
        rect.x as f32,
        rect.y as f32,
        rect.w.max(0) as f32,
        rect.h.max(0) as f32,
    )
}

fn fill(surface: &mut Surface, rect: IntRect, color: Color) {
    if rect.w > 0 && rect.h > 0 {
        fill_rect(surface, &gui_rect(rect), color);
    }
}

fn draw_bottom_line(surface: &mut Surface, rect: IntRect, color: Color) {
    fill(
        surface,
        IntRect {
            x: rect.x,
            y: rect.y + rect.h - 1,
            w: rect.w,
            h: 1,
        },
        color,
    );
}

fn draw_raised(surface: &mut Surface, rect: IntRect, color: Color) {
    fill(surface, rect, color);
    fill(
        surface,
        IntRect {
            x: rect.x,
            y: rect.y,
            w: rect.w,
            h: 1,
        },
        LIGHT_EDGE,
    );
    fill(
        surface,
        IntRect {
            x: rect.x,
            y: rect.y,
            w: 1,
            h: rect.h,
        },
        LIGHT_EDGE,
    );
    fill(
        surface,
        IntRect {
            x: rect.x,
            y: rect.y + rect.h - 1,
            w: rect.w,
            h: 1,
        },
        DARK_EDGE,
    );
    fill(
        surface,
        IntRect {
            x: rect.x + rect.w - 1,
            y: rect.y,
            w: 1,
            h: rect.h,
        },
        DARK_EDGE,
    );
}

fn draw_sunken(surface: &mut Surface, rect: IntRect, color: Color) {
    fill(surface, rect, color);
    fill(
        surface,
        IntRect {
            x: rect.x,
            y: rect.y,
            w: rect.w,
            h: 1,
        },
        DARK_EDGE,
    );
    fill(
        surface,
        IntRect {
            x: rect.x,
            y: rect.y,
            w: 1,
            h: rect.h,
        },
        DARK_EDGE,
    );
    fill(
        surface,
        IntRect {
            x: rect.x,
            y: rect.y + rect.h - 1,
            w: rect.w,
            h: 1,
        },
        LIGHT_EDGE,
    );
    fill(
        surface,
        IntRect {
            x: rect.x + rect.w - 1,
            y: rect.y,
            w: 1,
            h: rect.h,
        },
        LIGHT_EDGE,
    );
}

fn draw_fitted_text(
    surface: &mut Surface,
    font: &dyn TextFont,
    rect: IntRect,
    text: &str,
    color: Color,
    size: f32,
    padding: i32,
) {
    if rect.w <= padding * 2 || rect.h <= 0 {
        return;
    }
    let available = (rect.w - padding * 2) as f32;
    let mut fitted = String::new();
    for character in text.chars() {
        let mut candidate = fitted.clone();
        candidate.push(character);
        if font.measure_text(&candidate, size).width > available {
            break;
        }
        fitted.push(character);
    }
    font.draw_text(
        surface,
        (rect.x + padding) as f32,
        (rect.y + ((rect.h as f32 - size) / 2.0).max(1.0) as i32) as f32,
        &fitted,
        size,
        color,
    );
}

#[cfg(test)]
mod tests {
    use super::*;
    use clonk_graphics::{BitmapFont, PixelFormat};

    fn running_view() -> ConsoleViewModel {
        ConsoleViewModel {
            strings: ConsoleStrings::default(),
            caption: "Round.c4s".to_owned(),
            current_scenario_path: Some(PathBuf::from("Round.c4s")),
            game_open: true,
            halted: false,
            runtime_record_possible: true,
            players: vec![ConsolePlayerRow {
                number: 3,
                quit_label: "Remove Ada".to_owned(),
                quit_enabled: true,
                viewport_label: "New for Ada".to_owned(),
            }],
            edit_mode: ConsoleEditMode::Edit,
            cursor_text: "X: 12 Y: 34".to_owned(),
            frame: 123,
            script_counter: 7,
            time_seconds: 3_661,
            frames_per_second: 36,
            ..Default::default()
        }
    }

    fn lobby_view() -> ConsoleViewModel {
        let mut view = running_view();
        view.caption = view.strings.default_caption.clone();
        view.current_scenario_path = None;
        view.game_open = false;
        view.lobby_active = true;
        view.editing = false;
        view.halted = true;
        view
    }

    fn menu_item_enabled(
        console: &DeveloperConsole,
        menu: ConsoleMenu,
        id: &ConsoleMenuItemId,
    ) -> bool {
        console
            .menu_entries(menu)
            .into_iter()
            .filter_map(|entry| entry.item().cloned())
            .find(|item| &item.id == id)
            .expect("developer-console menu item")
            .enabled
    }

    fn click_console_control(
        console: &mut DeveloperConsole,
        rect: IntRect,
    ) -> Vec<DeveloperConsoleAction> {
        let point = GuiPoint::new((rect.x + rect.w / 2) as f32, (rect.y + rect.h / 2) as f32);
        console.handle_pointer_down(point, 640, 360);
        console.handle_pointer_up(point, 640, 360)
    }

    fn item_ids(entries: &[ConsoleMenuEntry]) -> Vec<ConsoleMenuItemId> {
        entries
            .iter()
            .filter_map(ConsoleMenuEntry::item)
            .map(|item| item.id.clone())
            .collect()
    }

    #[test]
    fn menu_structure_and_dynamic_net_insertion_match_console_order() {
        let mut console = DeveloperConsole::new();
        assert_eq!(
            console.menu_order(),
            vec![
                ConsoleMenu::File,
                ConsoleMenu::Components,
                ConsoleMenu::Player,
                ConsoleMenu::Viewport,
                ConsoleMenu::Help,
            ]
        );
        assert_eq!(
            item_ids(&console.menu_entries(ConsoleMenu::File)),
            vec![
                ConsoleMenuItemId::Open,
                ConsoleMenuItemId::OpenWithPlayers,
                ConsoleMenuItemId::SaveScenario,
                ConsoleMenuItemId::SaveScenarioAs,
                ConsoleMenuItemId::SaveGame,
                ConsoleMenuItemId::SaveGameAs,
                ConsoleMenuItemId::Record,
                ConsoleMenuItemId::Close,
                ConsoleMenuItemId::Quit,
            ]
        );

        let mut view = running_view();
        view.network_enabled = true;
        view.network_host = true;
        view.clients = vec![
            ConsoleClientRow {
                id: 0,
                menu_label: "Host Host (0)".to_owned(),
                menu_enabled: true,
            },
            ConsoleClientRow {
                id: 8,
                menu_label: "Client Guest (8) deactivated".to_owned(),
                menu_enabled: false,
            },
        ];
        console.set_view_model(view);
        assert_eq!(
            console.menu_order(),
            vec![
                ConsoleMenu::File,
                ConsoleMenu::Components,
                ConsoleMenu::Player,
                ConsoleMenu::Viewport,
                ConsoleMenu::Net,
                ConsoleMenu::Help,
            ]
        );
        assert_eq!(
            item_ids(&console.menu_entries(ConsoleMenu::Net)),
            vec![
                ConsoleMenuItemId::NetClient(0),
                ConsoleMenuItemId::NetClient(8)
            ]
        );
        let net = console.menu_entries(ConsoleMenu::Net);
        let net_items = net
            .iter()
            .filter_map(ConsoleMenuEntry::item)
            .collect::<Vec<_>>();
        assert_eq!(net_items[0].label, "Host Host (0)");
        assert!(net_items[0].enabled);
        assert_eq!(net_items[1].label, "Client Guest (8) deactivated");
        assert!(!net_items[1].enabled);

        let player = console.menu_entries(ConsoleMenu::Player);
        let quit = player
            .iter()
            .filter_map(ConsoleMenuEntry::item)
            .find(|item| item.id == ConsoleMenuItemId::QuitPlayer(3))
            .expect("projected quit row");
        assert_eq!(quit.label, "Remove Ada");
        assert!(quit.enabled);
        let viewport = console.menu_entries(ConsoleMenu::Viewport);
        let viewport = viewport
            .iter()
            .filter_map(ConsoleMenuEntry::item)
            .find(|item| item.id == ConsoleMenuItemId::NewPlayerViewport(3))
            .expect("projected viewport row");
        assert_eq!(viewport.label, "New for Ada");
    }

    #[test]
    fn enable_controls_matches_game_editing_player_and_record_gates() {
        let mut console = DeveloperConsole::new();
        let file = console.menu_entries(ConsoleMenu::File);
        let enabled = |id: ConsoleMenuItemId, rows: &[ConsoleMenuEntry]| {
            rows.iter()
                .filter_map(ConsoleMenuEntry::item)
                .find(|item| item.id == id)
                .unwrap()
                .enabled
        };
        assert!(enabled(ConsoleMenuItemId::Open, &file));
        assert!(!enabled(ConsoleMenuItemId::SaveScenario, &file));
        assert!(!enabled(ConsoleMenuItemId::Record, &file));

        let mut view = running_view();
        view.editing = false;
        console.set_view_model(view);
        let file = console.menu_entries(ConsoleMenu::File);
        assert!(enabled(ConsoleMenuItemId::SaveScenario, &file));
        assert!(enabled(ConsoleMenuItemId::SaveGame, &file));
        assert!(enabled(ConsoleMenuItemId::Record, &file));
        assert!(!enabled(
            ConsoleMenuItemId::JoinPlayer,
            &console.menu_entries(ConsoleMenu::Player)
        ));
        assert!(!enabled(
            ConsoleMenuItemId::Script,
            &console.menu_entries(ConsoleMenu::Components)
        ));
    }

    #[test]
    fn lobby_enables_only_the_native_gtk_lobby_controls() {
        let mut console = DeveloperConsole::new();
        console.set_view_model(lobby_view());

        assert!(menu_item_enabled(
            &console,
            ConsoleMenu::File,
            &ConsoleMenuItemId::Open
        ));
        assert!(menu_item_enabled(
            &console,
            ConsoleMenu::File,
            &ConsoleMenuItemId::OpenWithPlayers
        ));
        assert!(menu_item_enabled(
            &console,
            ConsoleMenu::File,
            &ConsoleMenuItemId::Quit
        ));
        for id in [
            ConsoleMenuItemId::SaveScenario,
            ConsoleMenuItemId::SaveScenarioAs,
            ConsoleMenuItemId::SaveGame,
            ConsoleMenuItemId::SaveGameAs,
            ConsoleMenuItemId::Record,
            ConsoleMenuItemId::Close,
        ] {
            assert!(!menu_item_enabled(&console, ConsoleMenu::File, &id));
        }
        for (menu, ids) in [
            (
                ConsoleMenu::Components,
                vec![
                    ConsoleMenuItemId::Objects,
                    ConsoleMenuItemId::Script,
                    ConsoleMenuItemId::Title,
                    ConsoleMenuItemId::Info,
                ],
            ),
            (
                ConsoleMenu::Player,
                vec![
                    ConsoleMenuItemId::JoinPlayer,
                    ConsoleMenuItemId::QuitPlayer(3),
                ],
            ),
            (
                ConsoleMenu::Viewport,
                vec![
                    ConsoleMenuItemId::NewViewport,
                    ConsoleMenuItemId::NewPlayerViewport(3),
                ],
            ),
        ] {
            for id in ids {
                assert!(!menu_item_enabled(&console, menu, &id));
            }
        }

        assert!(console.command_input_enabled());
        console.set_command_text("Log(42)");
        assert_eq!(
            console.handle_key(DeveloperConsoleKey::Enter, true),
            vec![DeveloperConsoleAction::SubmitInput("Log(42)".to_owned())]
        );
        assert!(console
            .handle_key(DeveloperConsoleKey::Pause, true)
            .is_empty());

        let layout = console.layout(640, 360);
        for (rect, action) in [
            (layout.play, DeveloperConsoleAction::Play),
            (layout.halt, DeveloperConsoleAction::Halt),
            (
                layout.mode_play,
                DeveloperConsoleAction::SetEditMode(ConsoleEditMode::Play),
            ),
            (
                layout.mode_edit,
                DeveloperConsoleAction::SetEditMode(ConsoleEditMode::Edit),
            ),
            (
                layout.mode_draw,
                DeveloperConsoleAction::SetEditMode(ConsoleEditMode::Draw),
            ),
        ] {
            assert_eq!(click_console_control(&mut console, rect), vec![action]);
        }
    }

    #[test]
    fn static_shell_strings_and_path_prompts_come_from_the_app_projection() {
        let mut view = running_view();
        view.strings.menu_file = "Datei".to_owned();
        view.strings.file_open = "Oeffnen...".to_owned();
        view.strings.player_join = "Beitritt".to_owned();
        view.strings.path_load_title = "Datei laden...".to_owned();
        view.strings.path_scenario_prompt = "Szenariopfad".to_owned();
        let mut console = DeveloperConsole::new();
        console.set_view_model(view);

        assert_eq!(
            ConsoleMenu::File.label(&console.view_model().strings),
            "Datei"
        );
        let open = console
            .menu_entries(ConsoleMenu::File)
            .into_iter()
            .filter_map(|entry| match entry {
                ConsoleMenuEntry::Item(item) => Some(item),
                ConsoleMenuEntry::Separator => None,
            })
            .find(|item| item.id == ConsoleMenuItemId::Open)
            .expect("localized open row");
        assert_eq!(open.label, "Oeffnen...");
        assert_eq!(
            console
                .menu_entries(ConsoleMenu::Player)
                .first()
                .and_then(ConsoleMenuEntry::item)
                .map(|item| item.label.as_str()),
            Some("Beitritt")
        );

        let request = match console.activate_menu_item(&ConsoleMenuItemId::Open).pop() {
            Some(DeveloperConsoleAction::RequestPath(request)) => request,
            _ => panic!("localized path request"),
        };
        assert_eq!(request.title, "Datei laden...");
        assert_eq!(request.prompt, "Szenariopfad");
    }

    #[test]
    fn status_labels_change_only_with_sources_and_keep_native_format() {
        let mut status = ConsoleStatusBars::default();
        let mut strings = ConsoleStrings::default();
        assert!(!status.update(0, 0, 0, 0, &strings));
        assert_eq!(status.revision(), 0);
        assert!(status.update(42, 9, 360_061, 61, &strings));
        assert_eq!(status.frame_text(), "Frame: 42");
        assert_eq!(status.script_text(), "Script: 9");
        assert_eq!(status.time_text(), "100:01:01 (61 FPS)");
        let revision = status.revision();
        assert!(!status.update(42, 9, 360_061, 61, &strings));
        assert_eq!(status.revision(), revision);
        assert!(status.update(42, 9, 360_061, 62, &strings));
        assert_eq!(status.time_text(), "100:01:01 (62 FPS)");

        strings.status_frame_prefix = "Bild".to_owned();
        strings.status_script_prefix = "Skript".to_owned();
        strings.status_fps_suffix = "BPS".to_owned();
        assert!(status.update(42, 9, 360_061, 62, &strings));
        assert_eq!(status.frame_text(), "Bild: 42");
        assert_eq!(status.script_text(), "Skript: 9");
        assert_eq!(status.time_text(), "100:01:01 (62 BPS)");
    }

    #[test]
    fn log_out_newline_clear_and_unicode_head_truncation_are_bounded() {
        let mut log = ConsoleLogBuffer::default();
        assert!(!log.out(""));
        assert!(log.out("first"));
        assert!(log.out("second\n"));
        assert_eq!(log.text(), "first\nsecond\n");

        let payload = "é".repeat(CONSOLE_LOG_CHARACTER_CAP + 11);
        log.out(&payload);
        assert_eq!(log.character_count(), CONSOLE_LOG_CHARACTER_CAP);
        assert!(log.text().is_char_boundary(0));
        assert_eq!(log.text().chars().count(), CONSOLE_LOG_CHARACTER_CAP);
        assert!(log.clear());
        assert_eq!(log.text(), "");
        assert!(!log.clear());
    }

    #[test]
    fn open_with_players_is_a_two_stage_tokenized_path_flow() {
        let mut console = DeveloperConsole::new();
        let scenario_request = match console
            .activate_menu_item(&ConsoleMenuItemId::OpenWithPlayers)
            .pop()
        {
            Some(DeveloperConsoleAction::RequestPath(request)) => request,
            _ => panic!("scenario chooser request"),
        };
        assert_eq!(
            scenario_request.purpose,
            ConsolePathPurpose::OpenScenarioWithPlayersScenario
        );
        let player_request = match console
            .respond_path_request(
                scenario_request.token,
                vec![PathBuf::from("Missions/Test Round.c4s")],
            )
            .pop()
        {
            Some(DeveloperConsoleAction::RequestPath(request)) => request,
            _ => panic!("player chooser request"),
        };
        let actions = console.respond_path_request(
            player_request.token,
            vec![PathBuf::from("Ada.c4p"), PathBuf::from("Bob.c4p")],
        );
        assert_eq!(
            actions,
            vec![DeveloperConsoleAction::OpenGame {
                scenario: PathBuf::from("Missions/Test Round.c4s"),
                player_files: vec![PathBuf::from("Ada.c4p"), PathBuf::from("Bob.c4p")],
            }]
        );
    }

    #[test]
    fn open_scenario_accepts_only_group_paths_or_exact_scenario_txt() {
        let mut console = DeveloperConsole::new();
        let request = match console.activate_menu_item(&ConsoleMenuItemId::Open).pop() {
            Some(DeveloperConsoleAction::RequestPath(request)) => request,
            _ => panic!("scenario chooser request"),
        };
        // rfd cannot express an exact basename, so `txt` remains a broad
        // chooser hint; the response validator supplies the native boundary.
        assert_eq!(request.extensions, vec!["c4s", "c4f", "txt"]);
        assert!(console
            .respond_path_request(request.token, vec![PathBuf::from("Readme.txt")])
            .is_empty());
        assert_eq!(
            console.path_entry().map(|entry| entry.request.token),
            Some(request.token),
            "an invalid chooser response must not consume the pending request"
        );
        assert_eq!(
            console
                .respond_path_request(request.token, vec![PathBuf::from("Missions/SCENARIO.TXT")],),
            vec![DeveloperConsoleAction::OpenGame {
                scenario: PathBuf::from("Missions/SCENARIO.TXT"),
                player_files: Vec::new(),
            }]
        );
    }

    #[test]
    fn open_with_players_does_not_accept_scenario_txt() {
        let mut console = DeveloperConsole::new();
        let request = match console
            .activate_menu_item(&ConsoleMenuItemId::OpenWithPlayers)
            .pop()
        {
            Some(DeveloperConsoleAction::RequestPath(request)) => request,
            _ => panic!("scenario chooser request"),
        };
        assert!(console
            .respond_path_request(request.token, vec![PathBuf::from("Scenario.txt")])
            .is_empty());
        assert_eq!(
            console.path_entry().map(|entry| entry.request.token),
            Some(request.token)
        );
        assert!(matches!(
            console
                .respond_path_request(request.token, vec![PathBuf::from("Missions.c4f")])
                .as_slice(),
            [DeveloperConsoleAction::RequestPath(ConsolePathRequest {
                purpose: ConsolePathPurpose::OpenScenarioWithPlayersPlayers,
                ..
            })]
        ));
    }

    #[test]
    fn software_path_entry_accepts_quoted_semicolon_lists() {
        let mut console = DeveloperConsole::new();
        let request = match console
            .begin_path_request(ConsolePathPurpose::JoinPlayers, None)
            .pop()
        {
            Some(DeveloperConsoleAction::RequestPath(request)) => request,
            _ => panic!("path request"),
        };
        assert_eq!(console.path_entry().unwrap().request.token, request.token);
        for character in "\"Players/A;da.c4p\"; Players/Bob.c4p".chars() {
            console.handle_character(character);
        }
        assert_eq!(
            console.handle_key(DeveloperConsoleKey::Enter, true),
            vec![DeveloperConsoleAction::JoinPlayers(vec![
                PathBuf::from("Players/A;da.c4p"),
                PathBuf::from("Players/Bob.c4p"),
            ])]
        );
    }

    #[test]
    fn command_entry_selects_submitted_text_and_pause_is_running_scoped() {
        let mut console = DeveloperConsole::new();
        assert!(!console.command_input_enabled());
        assert!(!console.handle_character('x'));
        assert!(console
            .handle_key(DeveloperConsoleKey::Enter, true)
            .is_empty());
        assert!(console
            .handle_key(DeveloperConsoleKey::Pause, true)
            .is_empty());
        console.set_view_model(running_view());
        assert!(console.command_input_enabled());
        console.set_command_text("Log(42)");
        assert_eq!(
            console.handle_key(DeveloperConsoleKey::Enter, true),
            vec![DeveloperConsoleAction::SubmitInput("Log(42)".to_owned())]
        );
        console.handle_character('N');
        assert_eq!(console.command_text(), "N");
        assert_eq!(
            console.handle_key(DeveloperConsoleKey::Pause, true),
            vec![DeveloperConsoleAction::TogglePause]
        );
        assert!(console
            .handle_key(DeveloperConsoleKey::Pause, false)
            .is_empty());
    }

    #[test]
    fn gtk_completion_order_prefix_navigation_and_acceptance_are_reachable() {
        let mut view = running_view();
        view.completions = vec![
            "GetAlpha".to_owned(),
            "getBeta".to_owned(),
            "ScenarioFunction".to_owned(),
        ];
        let mut console = DeveloperConsole::new();
        console.set_view_model(view);

        console.handle_character('g');
        assert_eq!(console.completion_match_indices(), vec![0, 1]);
        assert_eq!(console.selected_completion(), None);
        console.handle_key(DeveloperConsoleKey::Down, true);
        assert_eq!(console.selected_completion(), Some("GetAlpha"));
        console.handle_key(DeveloperConsoleKey::Down, true);
        assert_eq!(console.selected_completion(), Some("getBeta"));
        assert!(console
            .handle_key(DeveloperConsoleKey::Enter, true)
            .is_empty());
        assert_eq!(console.command_text(), "getBeta");
        assert_eq!(
            console.handle_key(DeveloperConsoleKey::Enter, true),
            vec![DeveloperConsoleAction::SubmitInput("getBeta".to_owned())]
        );

        console.set_command_text("");
        console.handle_character('s');
        assert!(console
            .handle_key(DeveloperConsoleKey::Tab, true)
            .is_empty());
        assert_eq!(console.command_text(), "ScenarioFunction");

        console.set_command_text("g");
        let row = console.layout(640, 360).completion_dropdown[0].rect;
        let point = GuiPoint::new((row.x + 4) as f32, (row.y + 4) as f32);
        console.handle_pointer_down(point, 640, 360);
        assert!(console.handle_pointer_up(point, 640, 360).is_empty());
        assert_eq!(console.command_text(), "GetAlpha");
    }

    #[test]
    fn pointer_play_halt_and_mode_buttons_emit_host_actions() {
        let mut console = DeveloperConsole::new();
        console.set_view_model(running_view());
        let layout = console.layout(640, 360);
        let center = |rect: IntRect| {
            GuiPoint::new((rect.x + rect.w / 2) as f32, (rect.y + rect.h / 2) as f32)
        };
        let point = center(layout.halt);
        console.handle_pointer_down(point, 640, 360);
        assert_eq!(
            console.handle_pointer_up(point, 640, 360),
            vec![DeveloperConsoleAction::Halt]
        );
        let point = center(layout.mode_draw);
        console.handle_pointer_down(point, 640, 360);
        assert_eq!(
            console.handle_pointer_up(point, 640, 360),
            vec![DeveloperConsoleAction::SetEditMode(ConsoleEditMode::Draw)]
        );
    }

    #[test]
    fn menu_keyboard_navigation_skips_separators_and_disabled_items() {
        let mut console = DeveloperConsole::new();
        assert!(console.handle_menu_mnemonic('f'));
        assert_eq!(console.open_menu(), Some(ConsoleMenu::File));
        console.handle_key(DeveloperConsoleKey::Down, true);
        assert_eq!(
            console.handle_key(DeveloperConsoleKey::Enter, true),
            vec![DeveloperConsoleAction::RequestPath(
                console
                    .path_entry()
                    .expect("open-with-players prompt")
                    .request
            )]
        );
    }

    #[test]
    fn render_draws_shell_status_log_and_modal_without_panicking() {
        let mut console = DeveloperConsole::new();
        console.set_view_model(running_view());
        console.out("Console ready");
        console.handle_menu_mnemonic('p');
        let mut surface = Surface::new(640, 360, PixelFormat::Rgba8888);
        console.render(&mut surface, &BitmapFont::new());
        assert!(surface.pixels().chunks_exact(4).any(|pixel| pixel[3] != 0));

        console.begin_path_request(ConsolePathPurpose::OpenScenario, None);
        console.render(&mut surface, &BitmapFont::new());
        assert!(console.layout(640, 360).path_dialog.is_some());
    }
}
