//! Classic `C4LeagueSignupDialog` presentation and input state.
//!
//! The controller is intentionally independent of league/network ownership:
//! callers decide when C++ would open the modal and translate a validated
//! [`LeagueSignupSubmission`] into the existing authentication request.

use std::cell::Cell;
use std::time::{Duration, Instant};

use anyhow::{ensure, Result};
use clonk_graphics::clonk_font::{ClonkFont, TextAlign};
use clonk_graphics::{GammaRamp, Surface};
use clonk_gui::Rect as GuiRect;

use crate::classic_gui::{
    draw_3d_frame, draw_clipped_text_with_markup, draw_engine_box, draw_facet_stretch,
    ClassicButtonState, ClassicGuiSkin, IntRect,
};
use crate::message_dialog::break_message;
use crate::{expand_hotkey_markup, ClonkFontSet, GuiPoint, ImageData, KeyCode};

const DIALOG_WIDTH: i32 = 500;
const DIALOG_INDENT: i32 = 10;
const ICON_SIZE: i32 = 40;
const FORM_LEFT_RESERVE: i32 = ICON_SIZE + 2 * DIALOG_INDENT;
const FORM_RIGHT_RESERVE: i32 = ICON_SIZE / 2 + 2 * DIALOG_INDENT;
const BUTTON_AREA_HEIGHT: i32 = 40;
const BUTTON_WIDTH: i32 = 120;
const BUTTON_HEIGHT: i32 = 32;
const BUTTON_GAP: i32 = 10;
const MIN_WOOD_BAR_HEIGHT: i32 = 23;
const EXTENDED_ICON_CELL: u32 = 64;
const LEAGUE_ICON_PHASE: u32 = 8;
const STANDARD_ICON_CELL: u32 = 40;
const CLOSE_ICON_PHASE: u32 = 34;
const EDIT_MAX_BYTES: usize = 254;
const EDIT_SCROLL_OFFSET: i32 = 2;
const TITLE_SCROLL_DELAY: Duration = Duration::from_millis(3000);
const EDIT_BACKGROUND: u32 = 0x7f00_0000;
const EDIT_SELECTION: u32 = 0x7f7f_7f00;
const WHITE: [u8; 4] = [255, 255, 255, 255];

/// `C4LeagueSignupDialog`'s two constructor branches.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum LeagueSignupMode {
    /// Authenticate an existing account. Password entry is mandatory.
    Login,
    /// Register a new account. A custom password is optional.
    Registration,
}

/// Runtime values passed to the classic dialog constructor.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct LeagueSignupConfig {
    pub player_name: String,
    pub server_name: String,
    pub account_preference: String,
    pub password_preference: String,
    pub mode: LeagueSignupMode,
}

impl LeagueSignupConfig {
    pub fn new(
        player_name: impl Into<String>,
        server_name: impl Into<String>,
        mode: LeagueSignupMode,
    ) -> Self {
        Self {
            player_name: player_name.into(),
            server_name: server_name.into(),
            account_preference: String::new(),
            password_preference: String::new(),
            mode,
        }
    }

    pub fn with_preferences(
        mut self,
        account: impl Into<String>,
        password: impl Into<String>,
    ) -> Self {
        self.account_preference = account.into();
        self.password_preference = password.into();
        self
    }
}

/// Localized strings consumed by the pure frontend controller.
///
/// The three `%s` values match the corresponding `LoadResStr` templates. A
/// host with a loaded language table should replace these defaults verbatim.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct LeagueSignupStrings {
    pub caption_on_server: String,
    pub login_message: String,
    pub registration_message: String,
    pub account_label: String,
    pub password_checkbox: String,
    pub password_checkbox_tooltip: String,
    pub password_label: String,
    pub password_confirmation_label: String,
    pub ok: String,
    pub cancel: String,
    pub close_tooltip: String,
    pub invalid_entry_caption: String,
    pub missing_account: String,
    pub invalid_account: String,
    pub account_too_short: String,
    pub missing_password: String,
    pub password_mismatch: String,
    pub cancelled_caption: String,
    pub cancelled_message: String,
}

impl Default for LeagueSignupStrings {
    fn default() -> Self {
        Self {
            caption_on_server: "League Login on %s".into(),
            login_message: "League login for player %s:".into(),
            registration_message: "Player %s: This is your first login at the league. Your can specify your desired league user name and league password below.".into(),
            account_label: "League user name:".into(),
            password_checkbox: "Specify league password".into(),
            password_checkbox_tooltip: "Enable to enter your own password. If you do not enter a password of your own, the personal WebCode will be used which is already stored on this system.".into(),
            password_label: "League password:".into(),
            password_confirmation_label: "League password (repeat):".into(),
            ok: "&OK".into(),
            cancel: "Cancel".into(),
            close_tooltip: "Close".into(),
            invalid_entry_caption: "Invalid Entry".into(),
            missing_account: "Please enter a user name!".into(),
            invalid_account: "The user name contains invalid characters.".into(),
            account_too_short: "The user name is too short.".into(),
            missing_password: "Please enter a password!".into(),
            password_mismatch: "Repeated password mismatch. Please re-enter password!".into(),
            cancelled_caption: "League Login".into(),
            cancelled_message: "League login for player %s cancelled. Without login this player can not take part in this round!".into(),
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum LeagueSignupField {
    Account,
    Password,
    PasswordConfirmation,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum LeagueSignupControl {
    Close,
    Account,
    PasswordCheckbox,
    Password,
    PasswordConfirmation,
    Ok,
    Cancel,
}

impl LeagueSignupControl {
    const fn field(self) -> Option<LeagueSignupField> {
        match self {
            Self::Account => Some(LeagueSignupField::Account),
            Self::Password => Some(LeagueSignupField::Password),
            Self::PasswordConfirmation => Some(LeagueSignupField::PasswordConfirmation),
            Self::Close | Self::PasswordCheckbox | Self::Ok | Self::Cancel => None,
        }
    }
}

/// Validation failures in native `UserClose(true)` order.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum LeagueSignupValidationError {
    MissingAccount,
    InvalidAccountCharacters,
    AccountTooShort,
    MissingPassword,
    PasswordMismatch,
}

impl LeagueSignupValidationError {
    pub const fn offending_control(self) -> LeagueSignupControl {
        match self {
            Self::MissingAccount | Self::InvalidAccountCharacters | Self::AccountTooShort => {
                LeagueSignupControl::Account
            }
            Self::MissingPassword => LeagueSignupControl::Password,
            Self::PasswordMismatch => LeagueSignupControl::PasswordConfirmation,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct LeagueSignupValidationFailure {
    pub error: LeagueSignupValidationError,
    pub caption: String,
    pub message: String,
}

/// Native-byte submission. Both fields are already encoded as Windows-1252,
/// the system charset used by `StdStrBuf` and the league request serializer.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct LeagueSignupSubmission {
    pub account: Vec<u8>,
    /// `None` is registration with the optional password checkbox unchecked.
    pub password: Option<Vec<u8>>,
}

#[derive(Clone, Debug, PartialEq)]
pub enum LeagueSignupAction {
    FocusChanged(LeagueSignupControl),
    TextChanged {
        field: LeagueSignupField,
        text: String,
    },
    PasswordEnabledChanged(bool),
    OpenEditContextMenu(LeagueSignupEditContextRequest),
    ClipboardTransfer {
        field: LeagueSignupField,
        text: String,
        cut: bool,
    },
    ValidationFailed(LeagueSignupValidationFailure),
    Submitted(LeagueSignupSubmission),
    Aborted {
        caption: String,
        message: String,
    },
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum LeagueSignupEditKey {
    Left,
    Right,
    Home,
    End,
    Backspace,
    Delete,
    SelectAll,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum LeagueSignupEditClipboardShortcut {
    Copy,
    Cut,
    Paste,
    SelectAll,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum LeagueSignupEditContextCommand {
    Cut,
    Copy,
    Paste,
    Clear,
    SelectAll,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum LeagueSignupSound {
    ArrowHit,
    Click,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct LeagueSignupEditContextItem {
    pub command: LeagueSignupEditContextCommand,
    pub label: String,
    pub tooltip: String,
}

#[derive(Clone, Debug, PartialEq)]
pub struct LeagueSignupEditContextRequest {
    pub field: LeagueSignupField,
    pub anchor: GuiPoint,
    pub items: Vec<LeagueSignupEditContextItem>,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct LeagueSignupKeyModifiers {
    pub shift: bool,
    pub control: bool,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct LeagueSignupEditLayout {
    pub bounds: IntRect,
    pub label: IntRect,
    pub edit: IntRect,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct LeagueSignupCheckboxLayout {
    pub bounds: IntRect,
    pub square: IntRect,
    pub label_x: i32,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct LeagueSignupLayout {
    pub bounds: IntRect,
    pub caption: IntRect,
    pub close_button: IntRect,
    pub client: IntRect,
    pub icon: IntRect,
    pub message: IntRect,
    pub message_text: String,
    pub account: LeagueSignupEditLayout,
    pub password_checkbox: Option<LeagueSignupCheckboxLayout>,
    pub password: Option<LeagueSignupEditLayout>,
    pub password_confirmation: Option<LeagueSignupEditLayout>,
    pub ok_button: IntRect,
    pub cancel_button: IntRect,
    /// Height added by `OnChkPassword` for the two registration edits.
    pub password_row_space: i32,
}

/// Borrowed, validated assets used by the classic renderer.
#[derive(Clone, Copy)]
pub struct LeagueSignupResources<'a> {
    pub skin: ClassicGuiSkin<'a>,
    pub fonts: &'a ClonkFontSet,
    pub icons: &'a ImageData,
    pub icons_extended: &'a ImageData,
    pub checkbox: &'a ImageData,
    pub button_highlight: &'a ImageData,
}

impl LeagueSignupResources<'_> {
    pub fn validate(self) -> Result<()> {
        self.skin.validate_message_dialog_assets()?;
        ensure!(
            (self.icons.width(), self.icons.height()) == (240, 360),
            "GUIIcons.png must be the exact 240x360 classic sheet, got {}x{}",
            self.icons.width(),
            self.icons.height()
        );
        ensure!(
            (self.icons_extended.width(), self.icons_extended.height()) == (256, 320),
            "GUIIcons2.png must be the exact 256x320 classic sheet, got {}x{}",
            self.icons_extended.width(),
            self.icons_extended.height()
        );
        ensure!(
            self.checkbox.height() > 0
                && self.checkbox.width() >= self.checkbox.height().saturating_mul(2),
            "GUICheckbox.png must contain enabled unchecked/checked phases, got {}x{}",
            self.checkbox.width(),
            self.checkbox.height()
        );
        ensure!(
            self.button_highlight.width() > 0 && self.button_highlight.height() > 0,
            "GUIButtonHighlight.png must not be empty"
        );
        ensure!(
            self.fonts.text.line_height > 0,
            "classic TextFont must have a positive line height"
        );
        Ok(())
    }
}

#[derive(Clone, Debug)]
struct EditState {
    text: String,
    caret: usize,
    anchor: usize,
    horizontal_scroll: i32,
    drag_anchor: Option<usize>,
    last_input: Instant,
    pending_cut: Option<PendingClipboardCut>,
}

#[derive(Clone, Debug)]
struct PendingClipboardCut {
    range: (usize, usize),
    text: String,
}

#[derive(Clone, Copy, Debug, Default)]
struct CaptionScrollState {
    last_change: Option<Instant>,
    position: i32,
    direction: i8,
}

#[derive(Clone, Copy, Debug)]
struct TitleDrag {
    pointer: GuiPoint,
    offset: (i32, i32),
}

impl EditState {
    fn new(text: &str) -> Self {
        let text = truncate_legacy_text(text, EDIT_MAX_BYTES);
        let end = text.len();
        Self {
            text,
            caret: end,
            anchor: end,
            horizontal_scroll: 0,
            drag_anchor: None,
            last_input: Instant::now(),
            pending_cut: None,
        }
    }

    fn set_text(&mut self, text: &str) {
        self.text = truncate_legacy_text(text, EDIT_MAX_BYTES);
        self.caret = self.text.len();
        self.anchor = self.caret;
        self.horizontal_scroll = 0;
        self.drag_anchor = None;
        self.last_input = Instant::now();
        self.pending_cut = None;
    }

    fn select_all(&mut self) {
        self.anchor = 0;
        self.caret = self.text.len();
        self.pending_cut = None;
    }

    fn deselect(&mut self) {
        self.anchor = self.caret;
        self.pending_cut = None;
    }

    fn selection(&self) -> Option<(usize, usize)> {
        (self.anchor != self.caret)
            .then_some((self.anchor.min(self.caret), self.anchor.max(self.caret)))
    }

    fn selected_text(&self) -> Option<&str> {
        let (start, end) = self.selection()?;
        self.text.get(start..end)
    }

    fn delete_selection(&mut self) -> bool {
        let Some((start, end)) = self.selection() else {
            return false;
        };
        self.text.replace_range(start..end, "");
        self.caret = start;
        self.anchor = start;
        self.last_input = Instant::now();
        self.pending_cut = None;
        true
    }

    fn insert_user_text(&mut self, input: &str) -> bool {
        let old = self.text.clone();
        self.pending_cut = None;
        self.delete_selection();
        for character in input.chars() {
            let character = if character == '|' {
                '\u{a6}'
            } else {
                character
            };
            let Some(encoded) = clonk_resources::encode_legacy_script_text(&character.to_string())
            else {
                continue;
            };
            if encoded
                .first()
                .is_some_and(|byte| *byte < b' ' || *byte == 0x7f)
            {
                continue;
            }
            let current = legacy_bytes(&self.text).map_or(EDIT_MAX_BYTES, |bytes| bytes.len());
            if current + encoded.len() > EDIT_MAX_BYTES {
                break;
            }
            self.text.insert(self.caret, character);
            self.caret += character.len_utf8();
            self.anchor = self.caret;
        }
        let changed = self.text != old;
        if changed {
            self.last_input = Instant::now();
        }
        changed
    }

    /// Native `Edit::InsertText`: used by clipboard and primary-selection
    /// insertion after their caller-specific transformations.
    fn insert_raw_text(&mut self, input: &str) -> bool {
        let old = self.text.clone();
        self.pending_cut = None;
        self.delete_selection();
        for character in input.chars() {
            let Some(encoded) = clonk_resources::encode_legacy_script_text(&character.to_string())
            else {
                continue;
            };
            let current = legacy_bytes(&self.text).map_or(EDIT_MAX_BYTES, |bytes| bytes.len());
            if current + encoded.len() > EDIT_MAX_BYTES {
                break;
            }
            self.text.insert(self.caret, character);
            self.caret += character.len_utf8();
            self.anchor = self.caret;
        }
        let changed = self.text != old;
        if changed {
            self.last_input = Instant::now();
        }
        changed
    }

    fn previous_boundary(&self, at: usize) -> usize {
        self.text[..at]
            .char_indices()
            .next_back()
            .map_or(0, |(index, _)| index)
    }

    fn next_boundary(&self, at: usize) -> usize {
        if at >= self.text.len() {
            self.text.len()
        } else {
            at + self.text[at..].chars().next().map_or(0, char::len_utf8)
        }
    }

    fn word_boundary(&self, direction: i8) -> usize {
        let mut position = self.caret;
        let mut nonspace_found = false;
        let mut space_found = false;
        loop {
            let next = if direction < 0 {
                if position == 0 {
                    break;
                }
                self.previous_boundary(position)
            } else {
                if position >= self.text.len() {
                    break;
                }
                self.next_boundary(position)
            };
            let sample = if direction < 0 { next } else { position };
            let character = self.text[sample..].chars().next().unwrap_or('\0');
            if character.is_ascii() && !character.is_ascii_alphanumeric() && character != '_' {
                if nonspace_found && direction < 0 {
                    break;
                }
                space_found = true;
            } else {
                if space_found && direction > 0 {
                    break;
                }
                nonspace_found = true;
            }
            position = next;
        }
        position
    }

    fn move_caret(&mut self, target: usize, shift: bool) {
        if !shift {
            self.anchor = target;
        }
        self.caret = target;
    }

    fn handle_key(
        &mut self,
        key: LeagueSignupEditKey,
        modifiers: LeagueSignupKeyModifiers,
    ) -> bool {
        let old = self.text.clone();
        self.pending_cut = None;
        match key {
            LeagueSignupEditKey::SelectAll => self.select_all(),
            LeagueSignupEditKey::Home => self.move_caret(0, modifiers.shift),
            LeagueSignupEditKey::End => self.move_caret(self.text.len(), modifiers.shift),
            LeagueSignupEditKey::Left => {
                let target = if modifiers.control {
                    self.word_boundary(-1)
                } else {
                    self.previous_boundary(self.caret)
                };
                self.move_caret(target, modifiers.shift);
            }
            LeagueSignupEditKey::Right => {
                let target = if modifiers.control {
                    self.word_boundary(1)
                } else {
                    self.next_boundary(self.caret)
                };
                self.move_caret(target, modifiers.shift);
            }
            LeagueSignupEditKey::Backspace => {
                if !self.delete_selection() && self.caret > 0 && !modifiers.shift {
                    let previous = if modifiers.control {
                        self.word_boundary(-1)
                    } else {
                        self.previous_boundary(self.caret)
                    };
                    self.text.replace_range(previous..self.caret, "");
                    self.caret = previous;
                    self.anchor = previous;
                }
            }
            LeagueSignupEditKey::Delete => {
                if !self.delete_selection() && self.caret < self.text.len() && !modifiers.shift {
                    let next = if modifiers.control {
                        self.word_boundary(1)
                    } else {
                        self.next_boundary(self.caret)
                    };
                    self.text.replace_range(self.caret..next, "");
                    self.anchor = self.caret;
                }
            }
        }
        self.last_input = Instant::now();
        self.text != old
    }

    fn displayed_prefix(&self, byte_index: usize, password: bool) -> String {
        if password {
            "*".repeat(self.text[..byte_index].chars().count())
        } else {
            self.text[..byte_index].to_owned()
        }
    }

    fn ensure_cursor_in_view(&mut self, rect: IntRect, font: &ClonkFont, password: bool) {
        let client_width = edit_client(rect).w;
        if client_width < 5 {
            return;
        }
        let mut width = font
            .measure(&self.displayed_prefix(self.caret, password), false)
            .0;
        width += font.measure("\u{a6}", false).0 / 2;
        if width < self.horizontal_scroll && self.horizontal_scroll > 0 {
            self.horizontal_scroll = (width - EDIT_SCROLL_OFFSET).max(0);
        }
        if width > self.horizontal_scroll && width > client_width + self.horizontal_scroll {
            self.horizontal_scroll = width - client_width
                + if self.caret < self.text.len() {
                    EDIT_SCROLL_OFFSET
                } else {
                    0
                };
        }
    }

    fn character_at(
        &self,
        pointer_x: f32,
        rect: IntRect,
        font: &ClonkFont,
        password: bool,
    ) -> usize {
        let control_x = pointer_x.floor() as i32 - edit_client(rect).x + self.horizontal_scroll;
        let mut previous_width = 0;
        for (index, character) in self.text.char_indices() {
            let end = index + character.len_utf8();
            let width = font.measure(&self.displayed_prefix(end, password), false).0;
            if width - (width - previous_width) / 2 >= control_x {
                return index;
            }
            previous_width = width;
        }
        self.text.len()
    }

    fn begin_pointer_selection(
        &mut self,
        position: usize,
        rect: IntRect,
        font: &ClonkFont,
        password: bool,
    ) {
        self.caret = position.min(self.text.len());
        self.anchor = self.caret;
        self.drag_anchor = Some(self.caret);
        self.pending_cut = None;
        self.ensure_cursor_in_view(rect, font, password);
    }

    fn drag_pointer_selection(
        &mut self,
        position: usize,
        rect: IntRect,
        font: &ClonkFont,
        password: bool,
    ) {
        let Some(anchor) = self.drag_anchor else {
            return;
        };
        self.caret = position.min(self.text.len());
        self.anchor = anchor;
        self.pending_cut = None;
        self.ensure_cursor_in_view(rect, font, password);
    }

    fn select_word_at(
        &mut self,
        mut position: usize,
        rect: IntRect,
        font: &ClonkFont,
        password: bool,
    ) {
        position = position.min(self.text.len());
        if is_word_spacer(self.text[position..].chars().next()) {
            if position == 0 {
                self.drag_anchor = None;
                return;
            }
            position = self.previous_boundary(position);
            if is_word_spacer(self.text[position..].chars().next()) {
                self.drag_anchor = None;
                return;
            }
        }
        let mut start = position;
        while start > 0 {
            let previous = self.previous_boundary(start);
            if is_word_spacer(self.text[previous..].chars().next()) {
                break;
            }
            start = previous;
        }
        let mut end = self.next_boundary(position);
        while end < self.text.len() && !is_word_spacer(self.text[end..].chars().next()) {
            end = self.next_boundary(end);
        }
        self.anchor = start;
        self.caret = end;
        self.drag_anchor = None;
        self.pending_cut = None;
        self.ensure_cursor_in_view(rect, font, password);
    }

    fn cursor_visible(&self) -> bool {
        (Instant::now()
            .checked_duration_since(self.last_input)
            .unwrap_or_default()
            .as_millis()
            / 500)
            .is_multiple_of(2)
    }
}

/// Pure state machine for one `C4LeagueSignupDialog`.
#[derive(Clone, Debug)]
pub struct LeagueSignupController {
    config: LeagueSignupConfig,
    strings: LeagueSignupStrings,
    caption: String,
    message: String,
    account: EditState,
    password: EditState,
    password_confirmation: EditState,
    password_enabled: bool,
    caption_scroll: Cell<CaptionScrollState>,
    dialog_offset: (i32, i32),
    title_drag: Option<TitleDrag>,
    focus: Option<LeagueSignupControl>,
    hovered: Option<LeagueSignupControl>,
    pointer_pressed: Option<LeagueSignupControl>,
    key_pressed: Option<(LeagueSignupControl, KeyCode)>,
    sound_events: Vec<LeagueSignupSound>,
    closed: bool,
}

impl LeagueSignupController {
    pub fn new(config: LeagueSignupConfig, strings: LeagueSignupStrings) -> Self {
        let caption = replace_first_placeholder(&strings.caption_on_server, &config.server_name);
        let message_template = match config.mode {
            LeagueSignupMode::Login => &strings.login_message,
            LeagueSignupMode::Registration => &strings.registration_message,
        };
        let message = replace_first_placeholder(message_template, &config.player_name);
        let mut account_preference = config.account_preference.clone();
        if config.mode == LeagueSignupMode::Registration && account_preference.is_empty() {
            account_preference.clone_from(&config.player_name);
        }
        let focus = match config.mode {
            LeagueSignupMode::Login => LeagueSignupControl::Password,
            LeagueSignupMode::Registration => LeagueSignupControl::Account,
        };
        let mut account = EditState::new(&account_preference);
        let mut password = EditState::new(&config.password_preference);
        let password_confirmation = EditState::new(&config.password_preference);
        match focus {
            LeagueSignupControl::Account => account.select_all(),
            LeagueSignupControl::Password => password.select_all(),
            _ => {}
        }
        Self {
            config,
            strings,
            caption,
            message,
            account,
            password,
            password_confirmation,
            password_enabled: false,
            caption_scroll: Cell::new(CaptionScrollState::default()),
            dialog_offset: (0, 0),
            title_drag: None,
            focus: Some(focus),
            hovered: None,
            pointer_pressed: None,
            key_pressed: None,
            sound_events: Vec::new(),
            closed: false,
        }
    }

    pub fn mode(&self) -> LeagueSignupMode {
        self.config.mode
    }

    pub fn player_name(&self) -> &str {
        &self.config.player_name
    }

    pub fn server_name(&self) -> &str {
        &self.config.server_name
    }

    pub fn caption(&self) -> &str {
        &self.caption
    }

    pub fn message(&self) -> &str {
        &self.message
    }

    pub fn strings(&self) -> &LeagueSignupStrings {
        &self.strings
    }

    pub fn focused_control(&self) -> Option<LeagueSignupControl> {
        self.focus
    }

    pub fn hovered_control(&self) -> Option<LeagueSignupControl> {
        self.hovered
    }

    pub const fn dialog_offset(&self) -> (i32, i32) {
        self.dialog_offset
    }

    pub fn reset_location(&mut self) {
        self.dialog_offset = (0, 0);
        self.title_drag = None;
    }

    pub fn take_sound_events(&mut self) -> Vec<LeagueSignupSound> {
        std::mem::take(&mut self.sound_events)
    }

    pub fn is_closed(&self) -> bool {
        self.closed
    }

    pub fn password_enabled(&self) -> bool {
        self.mode() == LeagueSignupMode::Login || self.password_enabled
    }

    pub fn has_password(&self) -> bool {
        self.password_enabled()
    }

    pub fn field_visible(&self, field: LeagueSignupField) -> bool {
        match field {
            LeagueSignupField::Account => true,
            LeagueSignupField::Password => self.password_enabled(),
            LeagueSignupField::PasswordConfirmation => {
                self.mode() == LeagueSignupMode::Registration && self.password_enabled
            }
        }
    }

    pub fn field_text(&self, field: LeagueSignupField) -> &str {
        &self.edit(field).text
    }

    pub fn field_selection(&self, field: LeagueSignupField) -> Option<(usize, usize)> {
        self.edit(field).selection()
    }

    pub fn field_horizontal_scroll(&self, field: LeagueSignupField) -> i32 {
        self.edit(field).horizontal_scroll
    }

    pub fn account(&self) -> &str {
        &self.account.text
    }

    pub fn password(&self) -> &str {
        &self.password.text
    }

    pub fn password_confirmation(&self) -> &str {
        &self.password_confirmation.text
    }

    pub fn set_field_text(&mut self, field: LeagueSignupField, text: &str) {
        self.edit_mut(field).set_text(text);
    }

    pub fn set_account(&mut self, text: &str) {
        self.set_field_text(LeagueSignupField::Account, text);
    }

    pub fn set_password(&mut self, text: &str) {
        self.set_field_text(LeagueSignupField::Password, text);
    }

    pub fn set_password_confirmation(&mut self, text: &str) {
        self.set_field_text(LeagueSignupField::PasswordConfirmation, text);
    }

    pub fn set_focus(&mut self, control: LeagueSignupControl) -> bool {
        if !self.control_visible(control) || self.focus == Some(control) {
            return false;
        }
        if let Some(field) = self.focus.and_then(LeagueSignupControl::field) {
            self.edit_mut(field).deselect();
        }
        self.focus = Some(control);
        if let Some(field) = control.field() {
            let edit = self.edit_mut(field);
            edit.select_all();
            // `Edit::OnGetFocus` restarts the caret flash after selecting all.
            edit.last_input = Instant::now();
        }
        self.key_pressed = None;
        true
    }

    fn clear_focus(&mut self) {
        if let Some(field) = self.focus.and_then(LeagueSignupControl::field) {
            self.edit_mut(field).deselect();
        }
        self.focus = None;
        self.key_pressed = None;
    }

    pub fn set_password_enabled(&mut self, enabled: bool) -> bool {
        if self.mode() != LeagueSignupMode::Registration || self.password_enabled == enabled {
            return false;
        }
        self.password_enabled = enabled;
        if !enabled
            && matches!(
                self.focus,
                Some(LeagueSignupControl::Password | LeagueSignupControl::PasswordConfirmation)
            )
        {
            // Hiding a native Container clears focus from a contained Edit;
            // it does not transfer focus to the checkbox that caused the hide.
            self.clear_focus();
        }
        self.pointer_pressed = None;
        self.key_pressed = None;
        true
    }

    pub fn cancel_interaction(&mut self) {
        self.hovered = None;
        self.pointer_pressed = None;
        self.key_pressed = None;
        self.title_drag = None;
        self.account.drag_anchor = None;
        self.password.drag_anchor = None;
        self.password_confirmation.drag_anchor = None;
    }

    /// Release classic mouse ownership after the pointer leaves the window.
    ///
    /// `CMouse::ReleaseElements` calls `Button::MouseLeave` before dropping
    /// `pDragElement`, so a visibly depressed button is raised with the
    /// native `ArrowHit` sound. Non-pointer teardown remains silent through
    /// [`Self::cancel_interaction`].
    pub fn pointer_left(&mut self) {
        let button_was_down = self.pointer_button_is_down();
        self.hovered = None;
        self.pointer_pressed = None;
        self.title_drag = None;
        self.account.drag_anchor = None;
        self.password.drag_anchor = None;
        self.password_confirmation.drag_anchor = None;
        if button_was_down {
            self.sound_events.push(LeagueSignupSound::ArrowHit);
        }
    }

    pub fn toggle_password_enabled(&mut self) -> LeagueSignupAction {
        let enabled = !self.password_enabled;
        self.set_password_enabled(enabled);
        self.sound_events.push(LeagueSignupSound::ArrowHit);
        LeagueSignupAction::PasswordEnabledChanged(enabled)
    }

    pub fn layout(
        &self,
        screen_width: i32,
        screen_height: i32,
        font: &ClonkFont,
    ) -> LeagueSignupLayout {
        let form_x = FORM_LEFT_RESERVE + DIALOG_INDENT;
        let form_width = DIALOG_WIDTH - form_x - FORM_RIGHT_RESERVE - DIALOG_INDENT;
        let message_text = break_message(font, &self.message, form_width);
        let message_height = font.measure(&message_text, true).1;
        let edit_height = (font.line_height + 3).max(MIN_WOOD_BAR_HEIGHT);
        let row_height = |label: &str| font.measure(label, true).1 + edit_height + 2;
        let account_height = row_height(&self.strings.account_label);
        let password_height = row_height(&self.strings.password_label);
        // C++ constructs both password LabeledEdits with the one iCtrlHeight
        // measured from IDS_CTL_LEAGUEPASSWORD.
        let confirmation_height = password_height;
        let checkbox_height = font
            .measure(
                &expand_hotkey_markup(&self.strings.password_checkbox).0,
                true,
            )
            .1;
        let password_row_space = password_height + confirmation_height + 4 * DIALOG_INDENT;

        let mut client_height = message_height + 2 * DIALOG_INDENT;
        client_height += account_height + 2 * DIALOG_INDENT;
        match self.mode() {
            LeagueSignupMode::Login => {
                client_height += password_height + 2 * DIALOG_INDENT;
            }
            LeagueSignupMode::Registration => {
                client_height += checkbox_height + 2 * DIALOG_INDENT;
                if self.password_enabled {
                    client_height += password_row_space;
                }
            }
        }
        client_height += BUTTON_AREA_HEIGHT + 2 * DIALOG_INDENT;
        let caption_height = font.line_height.max(MIN_WOOD_BAR_HEIGHT);
        let total_height = caption_height + client_height;
        let x = (screen_width - DIALOG_WIDTH) / 2 + self.dialog_offset.0;
        // Dialog::GetMarginTop includes the WoodenLabel title. SetClientSize
        // adds that margin to rcBounds, so Screen::ShowDialog centers and
        // Dialog::DrawElement frames the combined caption + client bounds.
        let y = (screen_height - total_height) / 2 + self.dialog_offset.1;
        let caption = IntRect {
            x,
            y,
            w: DIALOG_WIDTH,
            h: caption_height,
        };
        let close_button = IntRect {
            x: caption.x + caption.w - 20,
            y: caption.y + 4,
            w: 16,
            h: 16,
        };
        let client = IntRect {
            x,
            y: y + caption_height,
            w: DIALOG_WIDTH,
            h: client_height,
        };
        let icon = IntRect {
            x: client.x + DIALOG_INDENT,
            y: client.y + DIALOG_INDENT,
            w: ICON_SIZE,
            h: ICON_SIZE,
        };
        let mut cursor_y = client.y;
        let message = IntRect {
            x: client.x + form_x,
            y: cursor_y + DIALOG_INDENT,
            w: form_width,
            h: message_height,
        };
        cursor_y += message_height + 2 * DIALOG_INDENT;
        let account_bounds = IntRect {
            x: client.x + form_x,
            y: cursor_y + DIALOG_INDENT,
            w: form_width,
            h: account_height,
        };
        let account = labeled_edit_layout(
            account_bounds,
            font.measure(&self.strings.account_label, true).1,
        );
        cursor_y += account_height + 2 * DIALOG_INDENT;

        let mut password_checkbox = None;
        let mut password = None;
        let mut password_confirmation = None;
        match self.mode() {
            LeagueSignupMode::Login => {
                let bounds = IntRect {
                    x: client.x + form_x,
                    y: cursor_y + DIALOG_INDENT,
                    w: form_width,
                    h: password_height,
                };
                password = Some(labeled_edit_layout(
                    bounds,
                    font.measure(&self.strings.password_label, true).1,
                ));
                cursor_y += password_height + 2 * DIALOG_INDENT;
            }
            LeagueSignupMode::Registration => {
                let bounds = IntRect {
                    x: client.x + form_x,
                    y: cursor_y + DIALOG_INDENT,
                    w: form_width,
                    h: checkbox_height,
                };
                password_checkbox = Some(LeagueSignupCheckboxLayout {
                    bounds,
                    square: IntRect {
                        x: bounds.x,
                        y: bounds.y,
                        w: bounds.h,
                        h: bounds.h,
                    },
                    label_x: bounds.x + bounds.h + 4,
                });
                cursor_y += checkbox_height + 2 * DIALOG_INDENT;
                if self.password_enabled {
                    let bounds = IntRect {
                        x: client.x + form_x,
                        y: cursor_y + DIALOG_INDENT,
                        w: form_width,
                        h: password_height,
                    };
                    password = Some(labeled_edit_layout(
                        bounds,
                        font.measure(&self.strings.password_label, true).1,
                    ));
                    cursor_y += password_height + 2 * DIALOG_INDENT;
                    let bounds = IntRect {
                        x: client.x + form_x,
                        y: cursor_y + DIALOG_INDENT,
                        w: form_width,
                        h: confirmation_height,
                    };
                    password_confirmation = Some(labeled_edit_layout(
                        bounds,
                        font.measure(&self.strings.password_confirmation_label, true)
                            .1,
                    ));
                    cursor_y += confirmation_height + 2 * DIALOG_INDENT;
                }
            }
        }

        let button_area = IntRect {
            x: client.x + form_x,
            y: cursor_y + DIALOG_INDENT,
            w: form_width,
            h: BUTTON_AREA_HEIGHT,
        };
        let group_width = 2 * BUTTON_WIDTH + BUTTON_GAP;
        let button_x = button_area.x + (button_area.w - group_width) / 2;
        let button_y = button_area.y + (button_area.h - BUTTON_HEIGHT) / 2;
        let ok_button = IntRect {
            x: button_x,
            y: button_y,
            w: BUTTON_WIDTH,
            h: BUTTON_HEIGHT,
        };
        let cancel_button = IntRect {
            x: button_x + BUTTON_WIDTH + BUTTON_GAP,
            ..ok_button
        };
        LeagueSignupLayout {
            bounds: IntRect {
                x,
                y,
                w: DIALOG_WIDTH,
                h: total_height,
            },
            caption,
            close_button,
            client,
            icon,
            message,
            message_text,
            account,
            password_checkbox,
            password,
            password_confirmation,
            ok_button,
            cancel_button,
            password_row_space,
        }
    }

    pub fn validate(&self) -> Result<LeagueSignupSubmission, LeagueSignupValidationError> {
        let account = legacy_bytes(&self.account.text)
            .ok_or(LeagueSignupValidationError::InvalidAccountCharacters)?;
        if account.is_empty() {
            return Err(LeagueSignupValidationError::MissingAccount);
        }
        if !account.iter().all(|byte| valid_account_byte(*byte)) {
            return Err(LeagueSignupValidationError::InvalidAccountCharacters);
        }
        if account.len() < 3 {
            return Err(LeagueSignupValidationError::AccountTooShort);
        }
        if self.password_enabled() {
            let password = legacy_bytes(&self.password.text).unwrap_or_default();
            if password.is_empty() {
                return Err(LeagueSignupValidationError::MissingPassword);
            }
            if self.mode() == LeagueSignupMode::Registration
                && password != legacy_bytes(&self.password_confirmation.text).unwrap_or_default()
            {
                return Err(LeagueSignupValidationError::PasswordMismatch);
            }
            Ok(LeagueSignupSubmission {
                account,
                password: Some(password),
            })
        } else {
            Ok(LeagueSignupSubmission {
                account,
                password: None,
            })
        }
    }

    pub fn submit(&mut self) -> LeagueSignupAction {
        match self.validate() {
            Ok(submission) => {
                self.closed = true;
                LeagueSignupAction::Submitted(submission)
            }
            Err(error) => {
                let control = error.offending_control();
                self.set_focus(control);
                if error == LeagueSignupValidationError::PasswordMismatch {
                    // Native focuses the confirmation edit first, then
                    // `SetText("", false)` clears only that edit.
                    self.password_confirmation.set_text("");
                }
                self.pointer_pressed = None;
                self.key_pressed = None;
                LeagueSignupAction::ValidationFailed(self.validation_failure(error))
            }
        }
    }

    pub fn abort(&mut self) -> LeagueSignupAction {
        self.closed = true;
        LeagueSignupAction::Aborted {
            caption: self.strings.cancelled_caption.clone(),
            message: replace_first_placeholder(
                &self.strings.cancelled_message,
                &self.config.player_name,
            ),
        }
    }

    pub fn handle_text_input(&mut self, text: &str) -> Vec<LeagueSignupAction> {
        let Some(field) = self.focus.and_then(LeagueSignupControl::field) else {
            return Vec::new();
        };
        if !self.field_visible(field) {
            return Vec::new();
        }
        if self.edit_mut(field).insert_user_text(text) {
            vec![LeagueSignupAction::TextChanged {
                field,
                text: self.field_text(field).to_owned(),
            }]
        } else {
            Vec::new()
        }
    }

    pub fn handle_text_input_with_layout(
        &mut self,
        text: &str,
        layout: &LeagueSignupLayout,
        font: &ClonkFont,
    ) -> Vec<LeagueSignupAction> {
        let actions = self.handle_text_input(text);
        self.ensure_focused_edit_in_view(layout, font);
        actions
    }

    pub fn handle_edit_key(
        &mut self,
        key: LeagueSignupEditKey,
        modifiers: LeagueSignupKeyModifiers,
    ) -> Vec<LeagueSignupAction> {
        let Some(field) = self.focus.and_then(LeagueSignupControl::field) else {
            return Vec::new();
        };
        if self.edit_mut(field).handle_key(key, modifiers) {
            vec![LeagueSignupAction::TextChanged {
                field,
                text: self.field_text(field).to_owned(),
            }]
        } else {
            Vec::new()
        }
    }

    pub fn handle_edit_key_with_layout(
        &mut self,
        key: LeagueSignupEditKey,
        modifiers: LeagueSignupKeyModifiers,
        layout: &LeagueSignupLayout,
        font: &ClonkFont,
    ) -> Vec<LeagueSignupAction> {
        let actions = self.handle_edit_key(key, modifiers);
        self.ensure_focused_edit_in_view(layout, font);
        actions
    }

    pub fn handle_clipboard_shortcut(
        &mut self,
        shortcut: LeagueSignupEditClipboardShortcut,
        clipboard_text: Option<&str>,
        layout: &LeagueSignupLayout,
        font: &ClonkFont,
    ) -> Vec<LeagueSignupAction> {
        let Some(field) = self.focus.and_then(LeagueSignupControl::field) else {
            return Vec::new();
        };
        let command = match shortcut {
            LeagueSignupEditClipboardShortcut::Copy => LeagueSignupEditContextCommand::Copy,
            LeagueSignupEditClipboardShortcut::Cut => LeagueSignupEditContextCommand::Cut,
            LeagueSignupEditClipboardShortcut::Paste => LeagueSignupEditContextCommand::Paste,
            LeagueSignupEditClipboardShortcut::SelectAll => {
                LeagueSignupEditContextCommand::SelectAll
            }
        };
        self.apply_edit_context_command(field, command, clipboard_text, layout, font)
    }

    pub fn apply_edit_context_command(
        &mut self,
        field: LeagueSignupField,
        command: LeagueSignupEditContextCommand,
        clipboard_text: Option<&str>,
        layout: &LeagueSignupLayout,
        font: &ClonkFont,
    ) -> Vec<LeagueSignupAction> {
        if !self.field_visible(field) {
            return Vec::new();
        }
        match command {
            LeagueSignupEditContextCommand::Copy => self.begin_clipboard_transfer(field, false),
            LeagueSignupEditContextCommand::Cut => self.begin_clipboard_transfer(field, true),
            LeagueSignupEditContextCommand::Paste => clipboard_text
                .filter(|text| !text.is_empty())
                .map(|text| self.paste_field_text(field, text, layout, font))
                .unwrap_or_default(),
            LeagueSignupEditContextCommand::Clear => {
                if !self.edit_mut(field).delete_selection() {
                    return Vec::new();
                }
                let rect = layout_for_field(layout, field).edit;
                self.edit_mut(field).ensure_cursor_in_view(
                    rect,
                    font,
                    field != LeagueSignupField::Account,
                );
                vec![self.text_changed_action(field)]
            }
            LeagueSignupEditContextCommand::SelectAll => {
                self.edit_mut(field).select_all();
                let rect = layout_for_field(layout, field).edit;
                self.edit_mut(field).ensure_cursor_in_view(
                    rect,
                    font,
                    field != LeagueSignupField::Account,
                );
                Vec::new()
            }
        }
    }

    pub fn confirm_clipboard_cut(
        &mut self,
        field: LeagueSignupField,
        layout: &LeagueSignupLayout,
        font: &ClonkFont,
    ) -> Vec<LeagueSignupAction> {
        let Some(pending) = self.edit_mut(field).pending_cut.take() else {
            return Vec::new();
        };
        let edit = self.edit_mut(field);
        if edit.selection() != Some(pending.range)
            || edit.text.get(pending.range.0..pending.range.1) != Some(pending.text.as_str())
            || !edit.delete_selection()
        {
            return Vec::new();
        }
        let rect = layout_for_field(layout, field).edit;
        edit.ensure_cursor_in_view(rect, font, field != LeagueSignupField::Account);
        vec![self.text_changed_action(field)]
    }

    pub fn request_context_menu_at(
        &mut self,
        point: GuiPoint,
        clipboard_has_text: bool,
        layout: &LeagueSignupLayout,
    ) -> Vec<LeagueSignupAction> {
        let Some(field) = field_at_point(layout, point) else {
            return Vec::new();
        };
        let request = self.edit_context_request(field, point, clipboard_has_text);
        (!request.items.is_empty())
            .then_some(LeagueSignupAction::OpenEditContextMenu(request))
            .into_iter()
            .collect()
    }

    pub fn request_context_menu_from_key(
        &self,
        clipboard_has_text: bool,
        layout: &LeagueSignupLayout,
    ) -> Vec<LeagueSignupAction> {
        let Some(field) = self.focus.and_then(LeagueSignupControl::field) else {
            return Vec::new();
        };
        let edit = layout_for_field(layout, field).edit;
        let anchor = GuiPoint::new((edit.x + edit.w / 2) as f32, (edit.y + edit.h / 2) as f32);
        let request = self.edit_context_request(field, anchor, clipboard_has_text);
        (!request.items.is_empty())
            .then_some(LeagueSignupAction::OpenEditContextMenu(request))
            .into_iter()
            .collect()
    }

    pub fn handle_pointer_middle_down(
        &mut self,
        point: GuiPoint,
        primary_selection: Option<&str>,
        layout: &LeagueSignupLayout,
        font: &ClonkFont,
    ) -> Vec<LeagueSignupAction> {
        let Some(field) = field_at_point(layout, point) else {
            return Vec::new();
        };
        let mut actions = Vec::new();
        // `Control::MouseInput` focuses only on LeftDown. A middle-click
        // edits the target field without transferring dialog focus.
        let rect = layout_for_field(layout, field).edit;
        let password = field != LeagueSignupField::Account;
        self.edit_mut(field)
            .ensure_cursor_in_view(rect, font, password);
        let position = self.edit(field).character_at(point.x, rect, font, password);
        {
            let edit = self.edit_mut(field);
            edit.caret = position;
            edit.anchor = position;
            edit.drag_anchor = None;
            edit.pending_cut = None;
        }
        let changed = primary_selection
            .filter(|text| !text.is_empty())
            .is_some_and(|text| self.edit_mut(field).insert_raw_text(text));
        self.edit_mut(field)
            .ensure_cursor_in_view(rect, font, password);
        if changed {
            actions.push(self.text_changed_action(field));
        }
        actions
    }

    pub fn handle_key_down(&mut self, key: KeyCode, shift: bool) -> Vec<LeagueSignupAction> {
        match key {
            KeyCode::Escape => vec![self.abort()],
            KeyCode::Enter
                if matches!(
                    self.focus,
                    Some(
                        LeagueSignupControl::Close
                            | LeagueSignupControl::Ok
                            | LeagueSignupControl::Cancel
                    )
                ) =>
            {
                if self.key_pressed.is_none() {
                    self.key_pressed = Some((self.focus.expect("matched focused control"), key));
                    self.sound_events.push(LeagueSignupSound::ArrowHit);
                }
                Vec::new()
            }
            KeyCode::Enter => vec![self.submit()],
            KeyCode::Tab => self.advance_focus(shift),
            KeyCode::Space if self.focus == Some(LeagueSignupControl::PasswordCheckbox) => {
                vec![self.toggle_password_enabled()]
            }
            KeyCode::Space
                if matches!(
                    self.focus,
                    Some(
                        LeagueSignupControl::Close
                            | LeagueSignupControl::Ok
                            | LeagueSignupControl::Cancel
                    )
                ) =>
            {
                if self.key_pressed.is_none() {
                    self.key_pressed = Some((self.focus.expect("matched focused control"), key));
                    self.sound_events.push(LeagueSignupSound::ArrowHit);
                }
                Vec::new()
            }
            KeyCode::Left => self.handle_edit_key(
                LeagueSignupEditKey::Left,
                LeagueSignupKeyModifiers {
                    shift,
                    control: false,
                },
            ),
            KeyCode::Right => self.handle_edit_key(
                LeagueSignupEditKey::Right,
                LeagueSignupKeyModifiers {
                    shift,
                    control: false,
                },
            ),
            KeyCode::Home => self.handle_edit_key(
                LeagueSignupEditKey::Home,
                LeagueSignupKeyModifiers {
                    shift,
                    control: false,
                },
            ),
            KeyCode::End => self.handle_edit_key(
                LeagueSignupEditKey::End,
                LeagueSignupKeyModifiers {
                    shift,
                    control: false,
                },
            ),
            KeyCode::Space | KeyCode::Up | KeyCode::Down | KeyCode::PageUp | KeyCode::PageDown => {
                Vec::new()
            }
        }
    }

    /// Dialog-level Alt mnemonic dispatch in C++ child insertion order.
    pub fn handle_hotkey(&mut self, character: char) -> Vec<LeagueSignupAction> {
        let character = character.to_ascii_uppercase();
        if self.mode() == LeagueSignupMode::Registration
            && expand_hotkey_markup(&self.strings.password_checkbox).1 == Some(character)
        {
            vec![self.toggle_password_enabled()]
        } else if expand_hotkey_markup(&self.strings.ok).1 == Some(character) {
            vec![self.submit()]
        } else if expand_hotkey_markup(&self.strings.cancel).1 == Some(character) {
            vec![self.abort()]
        } else {
            Vec::new()
        }
    }

    pub fn handle_key_up(&mut self, key: KeyCode) -> Vec<LeagueSignupAction> {
        let Some((control, pressed_key)) = self.key_pressed else {
            return Vec::new();
        };
        if key != pressed_key {
            return Vec::new();
        }
        self.key_pressed = None;
        if self.focus != Some(control) {
            return Vec::new();
        }
        self.sound_events.push(LeagueSignupSound::Click);
        vec![self.activate(control)]
    }

    pub fn handle_pointer_move(
        &mut self,
        point: GuiPoint,
        layout: &LeagueSignupLayout,
        font: &ClonkFont,
    ) -> Vec<LeagueSignupAction> {
        if let Some(drag) = self.title_drag {
            self.dialog_offset = (
                drag.offset.0 + (point.x - drag.pointer.x) as i32,
                drag.offset.1 + (point.y - drag.pointer.y) as i32,
            );
            self.hovered = None;
            return Vec::new();
        }
        let was_down = self.pointer_button_is_down();
        self.hovered = hit_control(layout, point);
        if was_down != self.pointer_button_is_down() {
            self.sound_events.push(LeagueSignupSound::ArrowHit);
        }
        let Some(field) = self.focus.and_then(LeagueSignupControl::field) else {
            return Vec::new();
        };
        if self.edit(field).drag_anchor.is_some() {
            let rect = layout_for_field(layout, field).edit;
            let password = field != LeagueSignupField::Account;
            let position = self.edit(field).character_at(point.x, rect, font, password);
            self.edit_mut(field)
                .drag_pointer_selection(position, rect, font, password);
        }
        Vec::new()
    }

    pub fn handle_pointer_down(
        &mut self,
        point: GuiPoint,
        layout: &LeagueSignupLayout,
        font: &ClonkFont,
    ) -> Vec<LeagueSignupAction> {
        self.hovered = hit_control(layout, point);
        if self.hovered.is_none() && contains(layout.caption, point) {
            self.pointer_pressed = None;
            self.title_drag = Some(TitleDrag {
                pointer: point,
                offset: self.dialog_offset,
            });
            return Vec::new();
        }
        let Some(control) = self.hovered else {
            self.pointer_pressed = None;
            return Vec::new();
        };
        if control == LeagueSignupControl::PasswordCheckbox {
            // CheckBox::IsFocusOnClick is false and LeftDown never captures;
            // the square toggles solely from the eventual LeftUp location.
            self.pointer_pressed = None;
            return Vec::new();
        }
        self.pointer_pressed = Some(control);
        if is_button_control(control) {
            self.sound_events.push(LeagueSignupSound::ArrowHit);
        }
        let mut actions = Vec::new();
        if let Some(field) = control.field() {
            if self.set_focus(control) {
                actions.push(LeagueSignupAction::FocusChanged(control));
            }
            let edit_rect = layout_for_field(layout, field).edit;
            let password = field != LeagueSignupField::Account;
            self.edit_mut(field)
                .ensure_cursor_in_view(edit_rect, font, password);
            let position = self
                .edit(field)
                .character_at(point.x, edit_rect, font, password);
            self.edit_mut(field)
                .begin_pointer_selection(position, edit_rect, font, password);
        }
        actions
    }

    pub fn handle_pointer_up(
        &mut self,
        point: GuiPoint,
        layout: &LeagueSignupLayout,
        font: &ClonkFont,
    ) -> Vec<LeagueSignupAction> {
        if let Some(drag) = self.title_drag.take() {
            self.dialog_offset = (
                drag.offset.0 + (point.x - drag.pointer.x) as i32,
                drag.offset.1 + (point.y - drag.pointer.y) as i32,
            );
            self.hovered = None;
            return Vec::new();
        }
        // `CMouse` invokes the retained Edit's StopDragging at the release
        // coordinate before normal LeftUp hit-testing.
        if let Some(field) = [
            LeagueSignupField::Account,
            LeagueSignupField::Password,
            LeagueSignupField::PasswordConfirmation,
        ]
        .into_iter()
        .find(|field| self.field_visible(*field) && self.edit(*field).drag_anchor.is_some())
        {
            let rect = layout_for_field(layout, field).edit;
            let password = field != LeagueSignupField::Account;
            let position = self.edit(field).character_at(point.x, rect, font, password);
            self.edit_mut(field)
                .drag_pointer_selection(position, rect, font, password);
        }
        // Preserve whether Button::MouseLeave had already called SetUp(false)
        // before replacing hover ownership with the release hit.
        let button_was_down = self.pointer_button_is_down();
        self.hovered = hit_control(layout, point);
        if button_was_down && !self.pointer_button_is_down() {
            self.sound_events.push(LeagueSignupSound::ArrowHit);
        }
        self.account.drag_anchor = None;
        self.password.drag_anchor = None;
        self.password_confirmation.drag_anchor = None;
        let pressed = self.pointer_pressed.take();
        if layout
            .password_checkbox
            .as_ref()
            .is_some_and(|checkbox| contains(checkbox.square, point))
        {
            return vec![self.toggle_password_enabled()];
        }
        let Some(pressed) = pressed else {
            return Vec::new();
        };
        if self.hovered != Some(pressed) || (is_button_control(pressed) && !button_was_down) {
            return Vec::new();
        }
        if is_button_control(pressed) {
            self.sound_events.push(LeagueSignupSound::Click);
        }
        match pressed {
            LeagueSignupControl::Close
            | LeagueSignupControl::PasswordCheckbox
            | LeagueSignupControl::Ok
            | LeagueSignupControl::Cancel => vec![self.activate(pressed)],
            LeagueSignupControl::Account
            | LeagueSignupControl::Password
            | LeagueSignupControl::PasswordConfirmation => Vec::new(),
        }
    }

    pub fn handle_pointer_double_click(
        &mut self,
        point: GuiPoint,
        layout: &LeagueSignupLayout,
        font: &ClonkFont,
    ) -> Vec<LeagueSignupAction> {
        self.hovered = hit_control(layout, point);
        self.pointer_pressed = None;
        let Some(control) = self.hovered else {
            return Vec::new();
        };
        let Some(field) = control.field() else {
            return Vec::new();
        };
        // Control::MouseInput transfers focus only for LeftDown. The platform
        // emits LeftDouble instead of the second LeftDown, so cross-edit word
        // selection leaves the previously focused control unchanged.
        let rect = layout_for_field(layout, field).edit;
        let password = field != LeagueSignupField::Account;
        self.edit_mut(field)
            .ensure_cursor_in_view(rect, font, password);
        let position = self.edit(field).character_at(point.x, rect, font, password);
        self.edit_mut(field)
            .select_word_at(position, rect, font, password);
        Vec::new()
    }

    pub fn render(
        &self,
        surface: &mut Surface,
        resources: LeagueSignupResources<'_>,
        active: bool,
        gamma: Option<&GammaRamp>,
    ) -> Result<()> {
        let cursor_visible = self
            .focus
            .and_then(LeagueSignupControl::field)
            .is_some_and(|field| self.edit(field).cursor_visible());
        self.render_with_cursor(surface, resources, active, cursor_visible, gamma)
    }

    /// Deterministic rendering entry point for cached frames and tests.
    pub fn render_with_cursor(
        &self,
        surface: &mut Surface,
        resources: LeagueSignupResources<'_>,
        active: bool,
        cursor_visible: bool,
        gamma: Option<&GammaRamp>,
    ) -> Result<()> {
        resources.validate()?;
        let layout = self.layout(
            surface.width() as i32,
            surface.height() as i32,
            &resources.fonts.text,
        );
        resources.skin.draw_dialog(surface, layout.bounds, gamma);
        resources.skin.draw_caption_scrolled(
            surface,
            layout.caption,
            &self.caption,
            &resources.fonts.text,
            WHITE,
            TextAlign::Left,
            20,
            self.caption_scroll_offset_at(Instant::now(), &resources.fonts.text),
            gamma,
        );
        if active
            && (self.focus == Some(LeagueSignupControl::Close)
                || self.hovered == Some(LeagueSignupControl::Close))
        {
            draw_highlight(
                surface,
                layout.close_button,
                resources.button_highlight,
                gamma,
            );
        }
        draw_standard_icon(
            surface,
            layout.close_button,
            resources.icons,
            CLOSE_ICON_PHASE,
            gamma,
        )?;
        if active
            && self
                .button_state(LeagueSignupControl::Close, active)
                .pressed
        {
            draw_highlight(
                surface,
                layout.close_button,
                resources.button_highlight,
                gamma,
            );
        }
        draw_league_icon(surface, layout.icon, resources.icons_extended, gamma)?;
        resources.fonts.text.draw_with_gamma(
            surface,
            layout.message.x,
            layout.message.y,
            &layout.message_text,
            WHITE,
            TextAlign::Left,
            true,
            gamma,
        );
        self.render_labeled_edit(
            surface,
            &layout.account,
            &self.strings.account_label,
            LeagueSignupField::Account,
            false,
            resources.fonts,
            active,
            cursor_visible,
            gamma,
        );
        if let Some(checkbox) = &layout.password_checkbox {
            draw_checkbox(
                surface,
                checkbox.square,
                self.password_enabled,
                resources.checkbox,
                gamma,
            )?;
            let (label, _) = expand_hotkey_markup(&self.strings.password_checkbox);
            resources.fonts.text.draw_with_gamma(
                surface,
                checkbox.label_x,
                checkbox.bounds.y
                    + (checkbox.bounds.h - resources.fonts.text.line_height).max(0) / 2,
                &label,
                WHITE,
                TextAlign::Left,
                true,
                gamma,
            );
            if active
                && (self.focus == Some(LeagueSignupControl::PasswordCheckbox)
                    || self.hovered == Some(LeagueSignupControl::PasswordCheckbox))
            {
                draw_highlight(
                    surface,
                    IntRect {
                        x: checkbox.square.x + checkbox.square.w / 4,
                        y: checkbox.square.y + checkbox.square.h / 4,
                        w: checkbox.square.w / 2,
                        h: checkbox.square.h / 2,
                    },
                    resources.button_highlight,
                    gamma,
                );
            }
        }
        if let Some(password) = &layout.password {
            self.render_labeled_edit(
                surface,
                password,
                &self.strings.password_label,
                LeagueSignupField::Password,
                true,
                resources.fonts,
                active,
                cursor_visible,
                gamma,
            );
        }
        if let Some(confirmation) = &layout.password_confirmation {
            self.render_labeled_edit(
                surface,
                confirmation,
                &self.strings.password_confirmation_label,
                LeagueSignupField::PasswordConfirmation,
                true,
                resources.fonts,
                active,
                cursor_visible,
                gamma,
            );
        }
        resources.skin.draw_button_with_highlight(
            surface,
            layout.ok_button,
            &self.strings.ok,
            resources.fonts,
            self.button_state(LeagueSignupControl::Ok, active),
            Some(resources.button_highlight),
            gamma,
        );
        resources.skin.draw_button_with_highlight(
            surface,
            layout.cancel_button,
            &self.strings.cancel,
            resources.fonts,
            self.button_state(LeagueSignupControl::Cancel, active),
            Some(resources.button_highlight),
            gamma,
        );
        Ok(())
    }

    #[allow(clippy::too_many_arguments)]
    fn render_labeled_edit(
        &self,
        surface: &mut Surface,
        layout: &LeagueSignupEditLayout,
        label: &str,
        field: LeagueSignupField,
        password: bool,
        fonts: &ClonkFontSet,
        active: bool,
        cursor_visible: bool,
        gamma: Option<&GammaRamp>,
    ) {
        fonts.text.draw_with_gamma(
            surface,
            layout.label.x,
            layout.label.y,
            label,
            WHITE,
            TextAlign::Left,
            false,
            gamma,
        );
        let state = self.edit(field);
        render_edit(
            surface,
            layout.edit,
            state,
            password,
            &fonts.text,
            active && self.focus.and_then(LeagueSignupControl::field) == Some(field),
            cursor_visible,
            gamma,
        );
    }

    fn validation_failure(
        &self,
        error: LeagueSignupValidationError,
    ) -> LeagueSignupValidationFailure {
        let message = match error {
            LeagueSignupValidationError::MissingAccount => &self.strings.missing_account,
            LeagueSignupValidationError::InvalidAccountCharacters => &self.strings.invalid_account,
            LeagueSignupValidationError::AccountTooShort => &self.strings.account_too_short,
            LeagueSignupValidationError::MissingPassword => &self.strings.missing_password,
            LeagueSignupValidationError::PasswordMismatch => &self.strings.password_mismatch,
        };
        LeagueSignupValidationFailure {
            error,
            caption: self.strings.invalid_entry_caption.clone(),
            message: message.clone(),
        }
    }

    fn edit(&self, field: LeagueSignupField) -> &EditState {
        match field {
            LeagueSignupField::Account => &self.account,
            LeagueSignupField::Password => &self.password,
            LeagueSignupField::PasswordConfirmation => &self.password_confirmation,
        }
    }

    fn edit_mut(&mut self, field: LeagueSignupField) -> &mut EditState {
        match field {
            LeagueSignupField::Account => &mut self.account,
            LeagueSignupField::Password => &mut self.password,
            LeagueSignupField::PasswordConfirmation => &mut self.password_confirmation,
        }
    }

    fn control_visible(&self, control: LeagueSignupControl) -> bool {
        control
            .field()
            .is_none_or(|field| self.field_visible(field))
            && (control != LeagueSignupControl::PasswordCheckbox
                || self.mode() == LeagueSignupMode::Registration)
    }

    fn focus_order(&self) -> Vec<LeagueSignupControl> {
        let mut controls = vec![LeagueSignupControl::Close, LeagueSignupControl::Account];
        if self.mode() == LeagueSignupMode::Registration {
            controls.push(LeagueSignupControl::PasswordCheckbox);
        }
        if self.password_enabled() {
            controls.push(LeagueSignupControl::Password);
        }
        if self.mode() == LeagueSignupMode::Registration && self.password_enabled {
            controls.push(LeagueSignupControl::PasswordConfirmation);
        }
        controls.extend([LeagueSignupControl::Ok, LeagueSignupControl::Cancel]);
        controls
    }

    fn text_changed_action(&self, field: LeagueSignupField) -> LeagueSignupAction {
        LeagueSignupAction::TextChanged {
            field,
            text: self.field_text(field).to_owned(),
        }
    }

    fn caption_scroll_offset_at(&self, now: Instant, font: &ClonkFont) -> i32 {
        if self.caption.is_empty() {
            return 0;
        }
        let max_scroll = (font.measure(&self.caption, true).0 + 5 + 20 - DIALOG_WIDTH).max(0);
        let mut state = self.caption_scroll.get();
        let Some(last_change) = state.last_change else {
            state.last_change = Some(now);
            self.caption_scroll.set(state);
            return 0;
        };
        if now.checked_duration_since(last_change).unwrap_or_default() >= TITLE_SCROLL_DELAY {
            if state.direction == 0 {
                state.direction = 1;
            }
            if max_scroll > 0 {
                state.position += i32::from(state.direction);
                if state.position >= max_scroll || state.position < 0 {
                    state.direction = -state.direction;
                    state.position += i32::from(state.direction);
                    state.last_change = Some(now);
                }
            }
        }
        self.caption_scroll.set(state);
        state.position
    }

    fn begin_clipboard_transfer(
        &mut self,
        field: LeagueSignupField,
        cut: bool,
    ) -> Vec<LeagueSignupAction> {
        let edit = self.edit_mut(field);
        let Some((range, text)) = edit
            .selection()
            .zip(edit.selected_text().map(str::to_owned))
        else {
            edit.pending_cut = None;
            return Vec::new();
        };
        edit.pending_cut = cut.then(|| PendingClipboardCut {
            range,
            text: text.clone(),
        });
        vec![LeagueSignupAction::ClipboardTransfer { field, text, cut }]
    }

    fn paste_field_text(
        &mut self,
        field: LeagueSignupField,
        clipboard: &str,
        layout: &LeagueSignupLayout,
        font: &ClonkFont,
    ) -> Vec<LeagueSignupAction> {
        let transformed = clipboard.replace('|', "\u{a6}");
        let mut rest = transformed.as_str();
        while let Some(line_break) = rest.find(['\r', '\n']) {
            if line_break == 0 {
                let skip = rest.chars().next().map_or(0, char::len_utf8);
                rest = &rest[skip..];
                continue;
            }
            let changed = self.edit_mut(field).insert_raw_text(&rest[..line_break]);
            let rect = layout_for_field(layout, field).edit;
            self.edit_mut(field).ensure_cursor_in_view(
                rect,
                font,
                field != LeagueSignupField::Account,
            );
            let mut actions = Vec::new();
            if changed {
                actions.push(self.text_changed_action(field));
            }
            actions.push(self.submit());
            return actions;
        }
        if rest.is_empty() {
            return Vec::new();
        }
        let changed = self.edit_mut(field).insert_raw_text(rest);
        let rect = layout_for_field(layout, field).edit;
        self.edit_mut(field)
            .ensure_cursor_in_view(rect, font, field != LeagueSignupField::Account);
        changed
            .then(|| self.text_changed_action(field))
            .into_iter()
            .collect()
    }

    fn edit_context_request(
        &self,
        field: LeagueSignupField,
        anchor: GuiPoint,
        clipboard_has_text: bool,
    ) -> LeagueSignupEditContextRequest {
        let edit = self.edit(field);
        let item = |command, label: &str, tooltip: &str| LeagueSignupEditContextItem {
            command,
            label: label.to_owned(),
            tooltip: tooltip.to_owned(),
        };
        let mut items = Vec::new();
        if edit.selection().is_some() {
            items.push(item(
                LeagueSignupEditContextCommand::Cut,
                "Cut",
                "Moves the selection to the clipboard.",
            ));
            items.push(item(
                LeagueSignupEditContextCommand::Copy,
                "Copy",
                "Copies the selection to the clipboard.",
            ));
        }
        if clipboard_has_text {
            items.push(item(
                LeagueSignupEditContextCommand::Paste,
                "Paste",
                "Inserts the contents of the clipboard.",
            ));
        }
        if edit.selection().is_some() {
            items.push(item(
                LeagueSignupEditContextCommand::Clear,
                "Clear",
                "Clears the selection.",
            ));
        }
        if !edit.text.is_empty() && edit.selection() != Some((0, edit.text.len())) {
            items.push(item(
                LeagueSignupEditContextCommand::SelectAll,
                "Select all",
                "Selects the complete text",
            ));
        }
        LeagueSignupEditContextRequest {
            field,
            anchor,
            items,
        }
    }

    fn ensure_focused_edit_in_view(&mut self, layout: &LeagueSignupLayout, font: &ClonkFont) {
        let Some(field) = self.focus.and_then(LeagueSignupControl::field) else {
            return;
        };
        let rect = layout_for_field(layout, field).edit;
        let password = field != LeagueSignupField::Account;
        self.edit_mut(field)
            .ensure_cursor_in_view(rect, font, password);
    }

    fn advance_focus(&mut self, backwards: bool) -> Vec<LeagueSignupAction> {
        let controls = self.focus_order();
        let current = self
            .focus
            .and_then(|focused| controls.iter().position(|control| *control == focused));
        let next = match (current, backwards) {
            (None, false) => 0,
            (None, true) => controls.len() - 1,
            (Some(current), true) => current.checked_sub(1).unwrap_or(controls.len() - 1),
            (Some(current), false) => (current + 1) % controls.len(),
        };
        let control = controls[next];
        self.set_focus(control);
        vec![LeagueSignupAction::FocusChanged(control)]
    }

    fn activate(&mut self, control: LeagueSignupControl) -> LeagueSignupAction {
        match control {
            LeagueSignupControl::Close => self.abort(),
            LeagueSignupControl::PasswordCheckbox => self.toggle_password_enabled(),
            LeagueSignupControl::Ok => self.submit(),
            LeagueSignupControl::Cancel => self.abort(),
            LeagueSignupControl::Account
            | LeagueSignupControl::Password
            | LeagueSignupControl::PasswordConfirmation => {
                LeagueSignupAction::FocusChanged(control)
            }
        }
    }

    fn button_state(&self, control: LeagueSignupControl, active: bool) -> ClassicButtonState {
        ClassicButtonState {
            pressed: (self.pointer_pressed == Some(control) && self.hovered == Some(control))
                || self
                    .key_pressed
                    .is_some_and(|(pressed, _)| pressed == control),
            highlighted: active && (self.focus == Some(control) || self.hovered == Some(control)),
        }
    }

    fn pointer_button_is_down(&self) -> bool {
        self.pointer_pressed
            .is_some_and(|pressed| is_button_control(pressed) && self.hovered == Some(pressed))
    }

    /// Tooltip ownership follows the native child element, even though only
    /// the checkbox square toggles on left-up.
    pub fn tooltip_at<'a>(
        &'a self,
        point: GuiPoint,
        layout: &LeagueSignupLayout,
    ) -> Option<&'a str> {
        if contains(layout.close_button, point) {
            Some(&self.strings.close_tooltip)
        } else if layout
            .password_checkbox
            .as_ref()
            .is_some_and(|checkbox| contains(checkbox.bounds, point))
        {
            Some(&self.strings.password_checkbox_tooltip)
        } else if contains(layout.caption, point) {
            Some(&self.caption)
        } else {
            None
        }
    }
}

fn replace_first_placeholder(template: &str, value: &str) -> String {
    template.find("%s").map_or_else(
        || template.to_owned(),
        |index| {
            let mut output = String::with_capacity(template.len() + value.len());
            output.push_str(&template[..index]);
            output.push_str(value);
            output.push_str(&template[index + 2..]);
            output
        },
    )
}

fn legacy_bytes(text: &str) -> Option<Vec<u8>> {
    clonk_resources::encode_legacy_script_text(text)
}

fn truncate_legacy_text(text: &str, limit: usize) -> String {
    let mut output = String::new();
    let mut length = 0usize;
    for character in text.chars() {
        let Some(encoded) = clonk_resources::encode_legacy_script_text(&character.to_string())
        else {
            continue;
        };
        if length + encoded.len() > limit {
            break;
        }
        output.push(character);
        length += encoded.len();
    }
    output
}

/// Literal `C4League_Name_Valid_Characters` from `src/C4League.h`.
fn valid_account_byte(byte: u8) -> bool {
    byte.is_ascii_alphanumeric()
        || matches!(byte, b' ' | b'.' | b'-' | b'_')
        || matches!(byte, 0xc0..=0xd6 | 0xd9..=0xdd | 0xdf..=0xf6 | 0xf8..=0xff)
}

fn labeled_edit_layout(bounds: IntRect, label_height: i32) -> LeagueSignupEditLayout {
    LeagueSignupEditLayout {
        bounds,
        label: IntRect {
            x: bounds.x,
            y: bounds.y,
            w: bounds.w,
            h: label_height,
        },
        // `ExpandLeft(-2); ExpandTop(-2)` after taking the label.
        edit: IntRect {
            x: bounds.x + 2,
            y: bounds.y + label_height + 2,
            w: (bounds.w - 2).max(0),
            h: (bounds.h - label_height - 2).max(0),
        },
    }
}

fn layout_for_field(
    layout: &LeagueSignupLayout,
    field: LeagueSignupField,
) -> &LeagueSignupEditLayout {
    match field {
        LeagueSignupField::Account => &layout.account,
        LeagueSignupField::Password => layout.password.as_ref().expect("visible password layout"),
        LeagueSignupField::PasswordConfirmation => layout
            .password_confirmation
            .as_ref()
            .expect("visible confirmation layout"),
    }
}

fn hit_control(layout: &LeagueSignupLayout, point: GuiPoint) -> Option<LeagueSignupControl> {
    if contains(layout.close_button, point) {
        Some(LeagueSignupControl::Close)
    } else if contains(layout.account.edit, point) {
        Some(LeagueSignupControl::Account)
    } else if layout
        .password_checkbox
        .as_ref()
        .is_some_and(|checkbox| contains(checkbox.square, point))
    {
        Some(LeagueSignupControl::PasswordCheckbox)
    } else if layout
        .password
        .as_ref()
        .is_some_and(|password| contains(password.edit, point))
    {
        Some(LeagueSignupControl::Password)
    } else if layout
        .password_confirmation
        .as_ref()
        .is_some_and(|confirmation| contains(confirmation.edit, point))
    {
        Some(LeagueSignupControl::PasswordConfirmation)
    } else if contains(layout.ok_button, point) {
        Some(LeagueSignupControl::Ok)
    } else if contains(layout.cancel_button, point) {
        Some(LeagueSignupControl::Cancel)
    } else {
        None
    }
}

fn field_at_point(layout: &LeagueSignupLayout, point: GuiPoint) -> Option<LeagueSignupField> {
    if contains(layout.account.edit, point) {
        Some(LeagueSignupField::Account)
    } else if layout
        .password
        .as_ref()
        .is_some_and(|password| contains(password.edit, point))
    {
        Some(LeagueSignupField::Password)
    } else if layout
        .password_confirmation
        .as_ref()
        .is_some_and(|confirmation| contains(confirmation.edit, point))
    {
        Some(LeagueSignupField::PasswordConfirmation)
    } else {
        None
    }
}

const fn is_button_control(control: LeagueSignupControl) -> bool {
    matches!(
        control,
        LeagueSignupControl::Close | LeagueSignupControl::Ok | LeagueSignupControl::Cancel
    )
}

fn contains(rect: IntRect, point: GuiPoint) -> bool {
    point.x >= rect.x as f32
        && point.x < (rect.x + rect.w) as f32
        && point.y >= rect.y as f32
        && point.y < (rect.y + rect.h) as f32
}

fn edit_client(rect: IntRect) -> IntRect {
    IntRect {
        x: rect.x + 4,
        y: rect.y + 2,
        w: (rect.w - 8).max(0),
        h: (rect.h - 4).max(0),
    }
}

fn is_word_spacer(character: Option<char>) -> bool {
    let character = character.unwrap_or('\0');
    character.is_ascii() && !character.is_ascii_alphanumeric() && character != '_'
}

fn draw_league_icon(
    surface: &mut Surface,
    rect: IntRect,
    icons_extended: &ImageData,
    gamma: Option<&GammaRamp>,
) -> Result<()> {
    let columns = icons_extended.width() / EXTENDED_ICON_CELL;
    let source_x = (LEAGUE_ICON_PHASE % columns) * EXTENDED_ICON_CELL;
    let source_y = (LEAGUE_ICON_PHASE / columns) * EXTENDED_ICON_CELL;
    ensure!(
        source_x + EXTENDED_ICON_CELL <= icons_extended.width()
            && source_y + EXTENDED_ICON_CELL <= icons_extended.height(),
        "classic extended league icon phase is outside GUIIcons2.png"
    );
    draw_facet_stretch(
        surface,
        icons_extended,
        (
            source_x as f32,
            source_y as f32,
            EXTENDED_ICON_CELL as f32,
            EXTENDED_ICON_CELL as f32,
        ),
        (rect.x as f32, rect.y as f32, rect.w as f32, rect.h as f32),
        gamma,
    );
    Ok(())
}

fn draw_standard_icon(
    surface: &mut Surface,
    rect: IntRect,
    icons: &ImageData,
    phase: u32,
    gamma: Option<&GammaRamp>,
) -> Result<()> {
    let columns = icons.width() / STANDARD_ICON_CELL;
    let source_x = phase % columns * STANDARD_ICON_CELL;
    let source_y = phase / columns * STANDARD_ICON_CELL;
    ensure!(
        source_x + STANDARD_ICON_CELL <= icons.width()
            && source_y + STANDARD_ICON_CELL <= icons.height(),
        "GUIIcons.png phase {phase} is outside the classic sheet"
    );
    draw_facet_stretch(
        surface,
        icons,
        (
            source_x as f32,
            source_y as f32,
            STANDARD_ICON_CELL as f32,
            STANDARD_ICON_CELL as f32,
        ),
        (rect.x as f32, rect.y as f32, rect.w as f32, rect.h as f32),
        gamma,
    );
    Ok(())
}

fn draw_checkbox(
    surface: &mut Surface,
    rect: IntRect,
    checked: bool,
    sheet: &ImageData,
    gamma: Option<&GammaRamp>,
) -> Result<()> {
    let cell = sheet.height();
    let source_x = u32::from(checked) * cell;
    ensure!(
        cell > 0 && source_x + cell <= sheet.width(),
        "GUICheckbox.png does not contain enabled checkbox phase"
    );
    draw_facet_stretch(
        surface,
        sheet,
        (source_x as f32, 0.0, cell as f32, cell as f32),
        (rect.x as f32, rect.y as f32, rect.w as f32, rect.h as f32),
        gamma,
    );
    Ok(())
}

fn draw_highlight(
    surface: &mut Surface,
    rect: IntRect,
    highlight: &ImageData,
    gamma: Option<&GammaRamp>,
) {
    crate::draw_image_bilinear_additive(
        surface,
        &GuiRect::new(rect.x as f32, rect.y as f32, rect.w as f32, rect.h as f32),
        highlight,
        gamma,
    );
}

#[allow(clippy::too_many_arguments)]
fn render_edit(
    surface: &mut Surface,
    rect: IntRect,
    state: &EditState,
    password: bool,
    font: &ClonkFont,
    focused: bool,
    cursor_visible: bool,
    gamma: Option<&GammaRamp>,
) {
    let client = edit_client(rect);
    let horizontal_scroll = {
        let mut snapshot = state.clone();
        snapshot.ensure_cursor_in_view(rect, font, password);
        snapshot.horizontal_scroll
    };
    draw_engine_box(
        surface,
        rect.x,
        rect.y,
        rect.x + rect.w - 1,
        client.y + client.h,
        EDIT_BACKGROUND,
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
    let displayed = if password {
        "*".repeat(state.text.chars().count())
    } else {
        state.text.clone()
    };
    if let Some((start, end)) = state.selection() {
        let x1 = client.x
            + font
                .measure(&state.displayed_prefix(start, password), false)
                .0
            - horizontal_scroll;
        let x2 = client.x
            + font
                .measure(&state.displayed_prefix(end, password), false)
                .0
            - horizontal_scroll;
        if x2 > x1 {
            let left = x1.max(clip.x);
            let right = (x2 - 1).min(clip.x + clip.w - 1);
            if left <= right {
                draw_engine_box(
                    surface,
                    left,
                    text_y,
                    right,
                    text_y + selection_height - 1,
                    EDIT_SELECTION,
                    gamma,
                );
            }
        }
    }
    draw_clipped_text_with_markup(
        surface,
        font,
        client.x - horizontal_scroll,
        text_y - 1,
        &displayed,
        WHITE,
        TextAlign::Left,
        gamma,
        clip,
        false,
    );
    if focused && cursor_visible {
        let caret_x = client.x
            + font
                .measure(&state.displayed_prefix(state.caret, password), false)
                .0
            - horizontal_scroll
            - font.measure("\u{a6}", false).0 / 2;
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

fn draw_scaled_caret(
    surface: &mut Surface,
    font: &ClonkFont,
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
    if width == 0 || height == 0 || glyph.pixels.len() != width as usize * height as usize {
        return;
    }
    let atlas_width = width.max(height).next_power_of_two();
    let mut pixels = vec![255_u8; atlas_width as usize * height as usize * 4];
    for pixel in pixels.chunks_exact_mut(4) {
        pixel[3] = 0;
    }
    for source_y in 0..height as usize {
        for source_x in 0..width as usize {
            let pixel = glyph.pixels[source_y * width as usize + source_x];
            let destination = (source_y * atlas_width as usize + source_x) * 4;
            let (red, green, blue) = if pixel.a == 0 {
                (255, 255, 255)
            } else {
                (pixel.r, pixel.g, pixel.b)
            };
            pixels[destination..destination + 4].copy_from_slice(&[red, green, blue, pixel.a]);
        }
    }
    let image = ImageData::new(atlas_width, height, pixels);
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

#[cfg(test)]
mod tests {
    use super::*;

    fn font() -> ClonkFont {
        ClonkFont::new(22)
    }

    fn registration() -> LeagueSignupController {
        LeagueSignupController::new(
            LeagueSignupConfig::new(
                "Andr\u{e9}",
                "league.example",
                LeagueSignupMode::Registration,
            ),
            LeagueSignupStrings::default(),
        )
    }

    fn login() -> LeagueSignupController {
        LeagueSignupController::new(
            LeagueSignupConfig::new("Andr\u{e9}", "league.example", LeagueSignupMode::Login)
                .with_preferences("account", "secret"),
            LeagueSignupStrings::default(),
        )
    }

    #[test]
    fn league_signup_forms_and_layout_match_cpp() {
        let font = font();
        let mut registration = registration();
        assert_eq!(registration.account(), "Andr\u{e9}");
        assert_eq!(
            registration.focused_control(),
            Some(LeagueSignupControl::Account)
        );
        assert!(!registration.password_enabled());
        assert!(!registration.field_visible(LeagueSignupField::Password));
        assert!(!registration.field_visible(LeagueSignupField::PasswordConfirmation));
        let collapsed = registration.layout(1280, 720, &font);
        assert_eq!(collapsed.bounds.w, 500);
        assert_eq!(collapsed.bounds.y, (720 - collapsed.bounds.h) / 2);
        assert_eq!(collapsed.bounds.h, collapsed.caption.h + collapsed.client.h);
        assert_eq!(
            collapsed.caption.y + collapsed.caption.h,
            collapsed.client.y
        );
        assert_eq!(collapsed.close_button.w, 16);
        assert_eq!(collapsed.close_button.x, collapsed.caption.x + 480);
        assert_eq!(collapsed.icon.w, 40);
        assert_eq!(collapsed.message.w, 380);
        assert_eq!(collapsed.account.bounds.w, 380);
        assert!(collapsed.password_checkbox.is_some());
        assert!(collapsed.password.is_none());
        assert!(collapsed.password_confirmation.is_none());
        assert_eq!(collapsed.account.label.y, collapsed.account.bounds.y);
        assert!(collapsed.account.edit.y > collapsed.account.label.y);

        assert!(registration.set_password_enabled(true));
        let expanded = registration.layout(1280, 720, &font);
        assert_eq!(
            expanded.client.h - collapsed.client.h,
            expanded.password_row_space
        );
        assert_eq!(
            (expanded.ok_button.y - expanded.client.y)
                - (collapsed.ok_button.y - collapsed.client.y),
            expanded.password_row_space
        );
        assert!(expanded.password.is_some());
        assert!(expanded.password_confirmation.is_some());
        assert_eq!(
            expanded.password.as_ref().map(|row| row.bounds.h),
            expanded
                .password_confirmation
                .as_ref()
                .map(|row| row.bounds.h)
        );
        assert!(expanded
            .password
            .as_ref()
            .is_some_and(|row| row.label.y < row.edit.y));

        let login = login();
        let layout = login.layout(1280, 720, &font);
        assert_eq!(login.focused_control(), Some(LeagueSignupControl::Password));
        assert!(login.password_enabled());
        assert!(layout.password_checkbox.is_none());
        assert!(layout.password.is_some());
        assert!(layout.password_confirmation.is_none());
    }

    fn failure(action: LeagueSignupAction) -> LeagueSignupValidationError {
        match action {
            LeagueSignupAction::ValidationFailed(failure) => failure.error,
            other => panic!("expected validation failure, got {other:?}"),
        }
    }

    #[test]
    fn league_signup_validation_order_and_focus_match_cpp() {
        let mut form = login();
        form.set_account("");
        form.set_password("");
        assert_eq!(
            failure(form.submit()),
            LeagueSignupValidationError::MissingAccount
        );
        assert_eq!(form.focused_control(), Some(LeagueSignupControl::Account));

        form.set_account("ab!");
        assert_eq!(
            failure(form.submit()),
            LeagueSignupValidationError::InvalidAccountCharacters
        );
        form.set_account("A\u{de}z");
        assert_eq!(
            failure(form.submit()),
            LeagueSignupValidationError::InvalidAccountCharacters,
            "C4League_Name_Valid_Characters deliberately omits byte DE"
        );
        form.set_account("ab");
        assert_eq!(
            failure(form.submit()),
            LeagueSignupValidationError::AccountTooShort
        );
        form.set_account("Andr\u{e9}");
        assert_eq!(
            failure(form.submit()),
            LeagueSignupValidationError::MissingPassword
        );
        assert_eq!(form.focused_control(), Some(LeagueSignupControl::Password));

        let mut registration = registration();
        registration.set_password_enabled(true);
        registration.set_password("first");
        registration.set_password_confirmation("second");
        assert_eq!(
            failure(registration.submit()),
            LeagueSignupValidationError::PasswordMismatch
        );
        assert_eq!(
            registration.focused_control(),
            Some(LeagueSignupControl::PasswordConfirmation)
        );
        assert_eq!(registration.password(), "first");
        assert_eq!(registration.password_confirmation(), "");

        registration.set_password_confirmation("first");
        match registration.submit() {
            LeagueSignupAction::Submitted(submission) => {
                assert_eq!(submission.account, b"Andr\xe9");
                assert_eq!(submission.password.as_deref(), Some(b"first".as_slice()));
            }
            other => panic!("expected submission, got {other:?}"),
        }
    }

    #[test]
    fn league_signup_abort_never_submits() {
        let mut form = login();
        let action = form.handle_key_down(KeyCode::Escape, false);
        assert_eq!(action.len(), 1);
        assert!(matches!(
            &action[0],
            LeagueSignupAction::Aborted { message, .. }
                if message.contains("Andr\u{e9}")
        ));
        assert!(form.is_closed());
        assert!(!action
            .iter()
            .any(|action| matches!(action, LeagueSignupAction::Submitted(_))));

        let mut form = login();
        let layout = form.layout(1280, 720, &font());
        let close_point = GuiPoint::new(
            (layout.close_button.x + 8) as f32,
            (layout.close_button.y + 8) as f32,
        );
        form.handle_pointer_down(close_point, &layout, &font());
        assert!(matches!(
            form.handle_pointer_up(close_point, &layout, &font())
                .as_slice(),
            [LeagueSignupAction::Aborted { .. }]
        ));
        assert_eq!(
            form.tooltip_at(close_point, &layout),
            Some(form.strings().close_tooltip.as_str())
        );
    }

    #[test]
    fn league_signup_edit_focus_and_word_keys_match_cpp() {
        let mut form = login();
        assert!(form.password.selection().is_some());
        let stale_input = Instant::now() - Duration::from_secs(2);
        form.account.last_input = stale_input;
        assert!(form.set_focus(LeagueSignupControl::Account));
        assert!(form.account.last_input > stale_input);
        assert!(form.account.cursor_visible());
        assert!(form.password.selection().is_none());
        assert!(form.account.selection().is_some());

        form.set_account("one two");
        assert!(form
            .handle_edit_key(
                LeagueSignupEditKey::Left,
                LeagueSignupKeyModifiers {
                    shift: false,
                    control: true,
                },
            )
            .is_empty());
        assert_eq!(form.account.caret, 4);
        assert!(form
            .handle_edit_key(
                LeagueSignupEditKey::Backspace,
                LeagueSignupKeyModifiers {
                    shift: true,
                    control: false,
                },
            )
            .is_empty());
        assert_eq!(form.account(), "one two");
        assert!(matches!(
            form.handle_edit_key(
                LeagueSignupEditKey::Backspace,
                LeagueSignupKeyModifiers {
                    shift: false,
                    control: true,
                },
            )
            .as_slice(),
            [LeagueSignupAction::TextChanged { .. }]
        ));
        assert_eq!(form.account(), "two");

        assert!(matches!(
            form.handle_key_down(KeyCode::Tab, true).as_slice(),
            [LeagueSignupAction::FocusChanged(LeagueSignupControl::Close)]
        ));
        assert!(form.handle_key_down(KeyCode::Enter, false).is_empty());
        assert!(form.handle_key_up(KeyCode::Space).is_empty());
        assert!(matches!(
            form.handle_key_up(KeyCode::Enter).as_slice(),
            [LeagueSignupAction::Aborted { .. }]
        ));

        let mut form = login();
        assert!(form.set_focus(LeagueSignupControl::Cancel));
        assert!(form.handle_key_down(KeyCode::Enter, false).is_empty());
        assert!(matches!(
            form.handle_key_up(KeyCode::Enter).as_slice(),
            [LeagueSignupAction::Aborted { .. }]
        ));

        let mut form = login();
        assert!(matches!(
            form.handle_hotkey('o').as_slice(),
            [LeagueSignupAction::Submitted(_)]
        ));
    }

    #[test]
    fn league_signup_checkbox_pointer_uses_only_square_and_keeps_focus() {
        let font = font();
        let mut form = registration();
        let layout = form.layout(1280, 720, &font);
        let checkbox = layout.password_checkbox.as_ref().unwrap();
        let label_point = GuiPoint::new(
            (checkbox.label_x + 2) as f32,
            (checkbox.bounds.y + checkbox.bounds.h / 2) as f32,
        );
        form.handle_pointer_down(label_point, &layout, &font);
        assert!(form
            .handle_pointer_up(label_point, &layout, &font)
            .is_empty());
        assert!(!form.password_enabled());

        let square_point = GuiPoint::new(
            (checkbox.square.x + checkbox.square.w / 2) as f32,
            (checkbox.square.y + checkbox.square.h / 2) as f32,
        );
        assert!(matches!(
            form.handle_pointer_up(square_point, &layout, &font)
                .as_slice(),
            [LeagueSignupAction::PasswordEnabledChanged(true)]
        ));
        assert_eq!(form.focused_control(), Some(LeagueSignupControl::Account));

        let mut dragged = registration();
        let dragged_layout = dragged.layout(1280, 720, &font);
        let ok_point = GuiPoint::new(
            (dragged_layout.ok_button.x + 2) as f32,
            (dragged_layout.ok_button.y + 2) as f32,
        );
        dragged.handle_pointer_down(ok_point, &dragged_layout, &font);
        assert!(matches!(
            dragged
                .handle_pointer_up(square_point, &dragged_layout, &font)
                .as_slice(),
            [LeagueSignupAction::PasswordEnabledChanged(true)]
        ));
        assert_eq!(
            dragged.take_sound_events(),
            vec![
                LeagueSignupSound::ArrowHit,
                LeagueSignupSound::ArrowHit,
                LeagueSignupSound::ArrowHit,
            ]
        );
    }

    #[test]
    fn league_signup_middle_paste_keeps_focus_and_drag_release_uses_final_point() {
        let font = font();
        let mut form = login();
        let layout = form.layout(1280, 720, &font);
        let account_client = edit_client(layout.account.edit);
        let account_left = GuiPoint::new(
            account_client.x as f32,
            (account_client.y + account_client.h / 2) as f32,
        );

        assert_eq!(form.focused_control(), Some(LeagueSignupControl::Password));
        assert!(matches!(
            form.handle_pointer_middle_down(account_left, Some("|primary"), &layout, &font,)
                .as_slice(),
            [LeagueSignupAction::TextChanged {
                field: LeagueSignupField::Account,
                ..
            }]
        ));
        assert_eq!(
            form.focused_control(),
            Some(LeagueSignupControl::Password),
            "MiddleDown is not a Control focus gesture"
        );
        assert!(form.account().contains("|primary"));

        assert!(form.set_focus(LeagueSignupControl::Account));
        form.set_account("drag target");
        let layout = form.layout(1280, 720, &font);
        let client = edit_client(layout.account.edit);
        let left = GuiPoint::new(client.x as f32, (client.y + client.h / 2) as f32);
        let right = GuiPoint::new(
            (client.x + client.w - 1) as f32,
            (client.y + client.h / 2) as f32,
        );
        form.handle_pointer_down(left, &layout, &font);
        assert!(form.handle_pointer_up(right, &layout, &font).is_empty());
        assert_eq!(
            form.field_selection(LeagueSignupField::Account),
            Some((0, 11))
        );
    }

    #[test]
    fn league_signup_hidden_edit_clears_focus_and_pointer_buttons_rearm() {
        let font = font();
        let mut form = registration();
        assert!(form.set_password_enabled(true));
        assert!(form.set_focus(LeagueSignupControl::Password));
        assert!(form.set_password_enabled(false));
        assert_eq!(form.focused_control(), None);
        assert!(form.handle_key_down(KeyCode::Space, false).is_empty());
        assert!(matches!(
            form.handle_key_down(KeyCode::Tab, false).as_slice(),
            [LeagueSignupAction::FocusChanged(LeagueSignupControl::Close)]
        ));

        let mut button_form = login();
        let layout = button_form.layout(1280, 720, &font);
        let cancel = GuiPoint::new(
            (layout.cancel_button.x + 2) as f32,
            (layout.cancel_button.y + 2) as f32,
        );
        let outside = GuiPoint::new(layout.bounds.x as f32, layout.bounds.y as f32);
        button_form.handle_pointer_down(cancel, &layout, &font);
        assert!(
            button_form
                .button_state(LeagueSignupControl::Cancel, true)
                .pressed
        );
        button_form.handle_pointer_move(outside, &layout, &font);
        assert!(
            !button_form
                .button_state(LeagueSignupControl::Cancel, true)
                .pressed
        );
        button_form.handle_pointer_move(cancel, &layout, &font);
        assert!(
            button_form
                .button_state(LeagueSignupControl::Cancel, true)
                .pressed
        );
        assert!(matches!(
            button_form
                .handle_pointer_up(cancel, &layout, &font)
                .as_slice(),
            [LeagueSignupAction::Aborted { .. }]
        ));

        let mut released_outside = login();
        let layout = released_outside.layout(1280, 720, &font);
        released_outside.handle_pointer_down(cancel, &layout, &font);
        released_outside.take_sound_events();
        assert!(released_outside
            .handle_pointer_up(outside, &layout, &font)
            .is_empty());
        assert_eq!(
            released_outside.take_sound_events(),
            vec![LeagueSignupSound::ArrowHit]
        );

        let mut disarmed = login();
        let layout = disarmed.layout(1280, 720, &font);
        disarmed.handle_pointer_down(cancel, &layout, &font);
        assert_eq!(
            disarmed.take_sound_events(),
            vec![LeagueSignupSound::ArrowHit]
        );
        disarmed.handle_pointer_move(outside, &layout, &font);
        assert_eq!(
            disarmed.take_sound_events(),
            vec![LeagueSignupSound::ArrowHit]
        );
        assert!(disarmed
            .handle_pointer_up(cancel, &layout, &font)
            .is_empty());
        assert!(disarmed.take_sound_events().is_empty());
        assert!(!disarmed.is_closed());

        let mut pointer_left = login();
        let layout = pointer_left.layout(1280, 720, &font);
        pointer_left.handle_pointer_down(cancel, &layout, &font);
        pointer_left.take_sound_events();
        pointer_left.pointer_left();
        assert_eq!(
            pointer_left.take_sound_events(),
            vec![LeagueSignupSound::ArrowHit]
        );
        assert!(pointer_left
            .handle_pointer_up(cancel, &layout, &font)
            .is_empty());

        let mut keyboard_latch = login();
        assert!(keyboard_latch.set_focus(LeagueSignupControl::Cancel));
        assert!(keyboard_latch
            .handle_key_down(KeyCode::Enter, false)
            .is_empty());
        keyboard_latch.pointer_left();
        assert!(matches!(
            keyboard_latch.handle_key_up(KeyCode::Enter).as_slice(),
            [LeagueSignupAction::Aborted { .. }]
        ));
    }
}
