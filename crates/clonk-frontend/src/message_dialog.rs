//! Pixel-faithful reusable `C4GUI::MessageDialog` presentation and input.
//!
//! The C++ modal wrapper only runs a nested message loop. The visible object
//! is an ordinary classic dialog, so this renderer deliberately leaves the
//! underlying screen untouched and draws no dimming layer.

use crate::classic_gui::{
    draw_3d_frame, draw_facet_stretch, ClassicButtonState, ClassicGuiSkin, IntRect,
};
use crate::clonk_fonts::NativeClonkFont;
use crate::context_menu::draw_classic_tooltip;
use crate::hud::HudFont;
use crate::{expand_hotkey_markup, ClonkFontSet, GuiPoint, ImageData, KeyCode};
use anyhow::Result;
use clonk_graphics::clonk_font::{
    active_markup_fragments, font_image_lookup_tag, inline_image_token, scaled_font_image_width,
    skip_markup_tag, ClonkFont, FontImageProvider, TextAlign,
};
use clonk_graphics::{GammaRamp, Surface};
use clonk_gui::Rect as GuiRect;
use std::cell::Cell;
use std::ops::{BitOr, BitOrAssign};
use std::time::{Duration, Instant};

const REGULAR_WIDTH: i32 = 500;
const MEDIUM_WIDTH: i32 = 360;
const SMALL_WIDTH: i32 = 300;
const MIN_CAPTION_HEIGHT: i32 = 23;
const ICON_SIZE: i32 = 40;
const DIALOG_INDENT: i32 = 10;
const BUTTON_WIDTH: i32 = 120;
const BUTTON_HEIGHT: i32 = 32;
const BUTTON_GAP: i32 = 10;
const CLIENT_VERTICAL_ROOM: i32 = 80;
const PROGRESS_VERTICAL_ROOM: i32 = 150;
const PROGRESS_HEIGHT: i32 = 30;
const PROGRESS_BUTTON_AREA_HEIGHT: i32 = 40;
const PROGRESS_BUTTON_WIDTH: i32 = 140;
const CLOSE_ICON_PHASE: u16 = 34;
const TITLE_LEFT_INDENT: i32 = 5;
const TITLE_RIGHT_INDENT: i32 = 20;
const TITLE_SCROLL_DELAY: Duration = Duration::from_millis(3000);

/// C++ `MessageDialog::Buttons` bitmask.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct MessageDialogButtons(u8);

impl MessageDialogButtons {
    pub const NONE: Self = Self(0);
    pub const OK: Self = Self(1);
    pub const CANCEL: Self = Self(2);
    pub const YES: Self = Self(4);
    pub const NO: Self = Self(8);
    pub const RETRY: Self = Self(16);
    /// Abort-dialog callback button. The native generic message dialog leaves
    /// this bit unused; `C4AbortGameDialog` inserts Restart between Yes and No.
    pub const RESTART: Self = Self(32);
    pub const OK_CANCEL: Self = Self(Self::OK.0 | Self::CANCEL.0);
    pub const YES_NO: Self = Self(Self::YES.0 | Self::NO.0);
    pub const YES_RESTART_NO: Self = Self(Self::YES.0 | Self::RESTART.0 | Self::NO.0);
    pub const RETRY_CANCEL: Self = Self(Self::RETRY.0 | Self::CANCEL.0);

    pub const fn contains(self, button: MessageDialogButton) -> bool {
        self.0 & button.mask() != 0
    }

    fn ordered(self) -> Vec<MessageDialogButton> {
        MessageDialogButton::ORDER
            .into_iter()
            .filter(|button| self.contains(*button))
            .collect()
    }
}

impl BitOr for MessageDialogButtons {
    type Output = Self;

    fn bitor(self, rhs: Self) -> Self::Output {
        Self(self.0 | rhs.0)
    }
}

impl BitOrAssign for MessageDialogButtons {
    fn bitor_assign(&mut self, rhs: Self) {
        self.0 |= rhs.0;
    }
}

/// Canonical construction/render order from `C4GuiDialogs.cpp`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MessageDialogButton {
    Ok,
    Retry,
    Cancel,
    Yes,
    Restart,
    No,
}

impl MessageDialogButton {
    const ORDER: [Self; 6] = [
        Self::Ok,
        Self::Retry,
        Self::Cancel,
        Self::Yes,
        Self::Restart,
        Self::No,
    ];

    const fn mask(self) -> u8 {
        match self {
            Self::Ok => MessageDialogButtons::OK.0,
            Self::Retry => MessageDialogButtons::RETRY.0,
            Self::Cancel => MessageDialogButtons::CANCEL.0,
            Self::Yes => MessageDialogButtons::YES.0,
            Self::Restart => MessageDialogButtons::RESTART.0,
            Self::No => MessageDialogButtons::NO.0,
        }
    }

    pub const fn label(self) -> &'static str {
        match self {
            Self::Ok => "&OK",
            Self::Retry => "Retry",
            Self::Cancel => "Cancel",
            Self::Yes => "&Yes",
            Self::Restart => "Restart",
            Self::No => "&No",
        }
    }

    pub const fn result(self) -> MessageDialogResult {
        match self {
            Self::Ok => MessageDialogResult::Ok,
            Self::Retry => MessageDialogResult::Retry,
            Self::Cancel => MessageDialogResult::Cancel,
            Self::Yes => MessageDialogResult::Yes,
            Self::Restart => MessageDialogResult::Restart,
            Self::No => MessageDialogResult::No,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MessageDialogResult {
    Ok,
    Retry,
    Cancel,
    Yes,
    Restart,
    No,
    Dismissed,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MessageDialogSound {
    ArrowHit,
    Click,
}

impl MessageDialogResult {
    pub const fn is_positive(self) -> bool {
        matches!(self, Self::Ok | Self::Retry | Self::Yes | Self::Restart)
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum MessageDialogSize {
    #[default]
    Regular,
    Medium,
    Small,
    /// Exact native dialog width for custom dialogs such as
    /// `C4AbortGameDialog`'s 400px host/Film2 layout.
    Fixed(i32),
}

impl MessageDialogSize {
    pub const fn width(self) -> i32 {
        match self {
            Self::Regular => REGULAR_WIDTH,
            Self::Medium => MEDIUM_WIDTH,
            Self::Small => SMALL_WIDTH,
            Self::Fixed(width) => width,
        }
    }
}

/// A normal 40px `GUIIcons` phase or extended 64px `GUIIcons2` phase.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum MessageDialogIcon {
    #[default]
    None,
    Standard(u16),
    Extended(u16),
}

impl MessageDialogIcon {
    pub const NOTIFY: Self = Self::Standard(1);
    pub const PLAYER: Self = Self::Standard(9);
    pub const ERROR: Self = Self::Standard(11);
    pub const CONFIRM: Self = Self::Standard(18);
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum MessageDialogPlacement {
    #[default]
    Centered,
    /// Non-exclusive C++ screens place ordinary dialogs at the preferred
    /// viewport origin plus `(30,30)`.
    Preferred { x: i32, y: i32 },
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct MessageDialogButtonLayout {
    pub button: MessageDialogButton,
    pub rect: IntRect,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct MessageDialogCheckboxLayout {
    pub bounds: IntRect,
    pub square: IntRect,
    pub label_x: i32,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct MessageDialogLayout {
    pub bounds: IntRect,
    pub caption: Option<IntRect>,
    pub close_button: Option<IntRect>,
    pub icon: IntRect,
    pub message: IntRect,
    pub message_text: String,
    pub message_alignment: TextAlign,
    pub checkbox: Option<MessageDialogCheckboxLayout>,
    pub progress: Option<IntRect>,
    pub buttons: Vec<MessageDialogButtonLayout>,
}

#[derive(Clone, Debug, PartialEq)]
pub struct MessageDialogTooltip {
    pub pointer: GuiPoint,
    pub text: String,
}

/// Borrowed classic resources. Callers should decline to render rather than
/// construct this value with substitute assets.
#[derive(Clone, Copy)]
pub struct MessageDialogResources<'a> {
    pub skin: ClassicGuiSkin<'a>,
    pub fonts: &'a ClonkFontSet,
    pub tooltip_font: &'a ClonkFont,
    pub icons: &'a ImageData,
    pub icons_extended: &'a ImageData,
    pub button_highlight: &'a ImageData,
    pub checkbox: &'a ImageData,
    pub progress: &'a ImageData,
}

impl MessageDialogResources<'_> {
    pub fn validate(self) -> Result<()> {
        self.skin.validate_message_dialog_assets()?;
        validate_icon_sheet("GUIIcons.png", self.icons, 40)?;
        validate_icon_sheet("GUIIcons2.png", self.icons_extended, 64)?;
        validate_nonempty_image("GUIButtonHighlight.png", self.button_highlight)?;
        validate_checkbox_sheet("GUICheckbox.png", self.checkbox)?;
        validate_progress_image("GUIProgress.png", self.progress)?;
        anyhow::ensure!(
            self.tooltip_font.line_height > 0,
            "classic TooltipFont must have a positive line height"
        );
        Ok(())
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum DialogTarget {
    Close,
    Checkbox,
    Button(usize),
}

#[derive(Clone, Debug)]
struct MessageDialogCheckbox {
    raw_label: String,
    expanded_label: String,
    hotkey: Option<char>,
    checked: bool,
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

/// Pure frontend state for one message dialog.
#[derive(Clone, Debug)]
pub struct MessageDialogState {
    caption: String,
    close_tooltip: String,
    progress_tooltip: String,
    caption_scroll: Cell<CaptionScrollState>,
    message: String,
    buttons: MessageDialogButtons,
    button_labels: Vec<(MessageDialogButton, String)>,
    icon: MessageDialogIcon,
    size: MessageDialogSize,
    default_no: bool,
    force_centered_message: bool,
    checkbox: Option<MessageDialogCheckbox>,
    progress: Option<u8>,
    checkbox_changes: Vec<bool>,
    placement: MessageDialogPlacement,
    dialog_offset: (i32, i32),
    focus: Option<DialogTarget>,
    hovered: Option<DialogTarget>,
    pointer: Option<GuiPoint>,
    pointer_pressed: Option<DialogTarget>,
    title_drag: Option<TitleDrag>,
    key_pressed: Option<DialogTarget>,
    sound_events: Vec<MessageDialogSound>,
}

impl MessageDialogState {
    pub fn new(
        message: impl Into<String>,
        caption: impl Into<String>,
        buttons: MessageDialogButtons,
        icon: MessageDialogIcon,
        size: MessageDialogSize,
        default_no: bool,
    ) -> Self {
        let caption = caption.into();
        let ordered = buttons.ordered();
        let focus_button = initial_focus_button(&ordered, default_no).map(DialogTarget::Button);
        Self {
            caption,
            close_tooltip: "Close".into(),
            progress_tooltip: "Progress bar".into(),
            caption_scroll: Cell::new(CaptionScrollState::default()),
            message: message.into(),
            buttons,
            button_labels: Vec::new(),
            icon,
            size,
            default_no,
            force_centered_message: false,
            checkbox: None,
            progress: None,
            checkbox_changes: Vec::new(),
            placement: MessageDialogPlacement::Centered,
            dialog_offset: (0, 0),
            focus: focus_button,
            hovered: None,
            pointer: None,
            pointer_pressed: None,
            title_drag: None,
            key_pressed: None,
            sound_events: Vec::new(),
        }
    }

    pub fn regular_ok(
        message: impl Into<String>,
        caption: impl Into<String>,
        icon: MessageDialogIcon,
    ) -> Self {
        Self::new(
            message,
            caption,
            MessageDialogButtons::OK,
            icon,
            MessageDialogSize::Regular,
            false,
        )
    }

    pub fn with_placement(mut self, placement: MessageDialogPlacement) -> Self {
        self.placement = placement;
        self
    }

    pub fn with_fixed_width(mut self, width: i32) -> Self {
        self.size = MessageDialogSize::Fixed(width);
        self
    }

    pub fn without_focus(mut self) -> Self {
        self.focus = None;
        self
    }

    pub fn with_centered_message(mut self) -> Self {
        self.force_centered_message = true;
        self
    }

    pub fn with_checkbox(mut self, label: impl Into<String>, checked: bool) -> Self {
        let raw_label = label.into();
        let (expanded_label, hotkey) = expand_hotkey_markup(&raw_label);
        self.checkbox = Some(MessageDialogCheckbox {
            raw_label,
            expanded_label,
            hotkey,
            checked,
        });
        self
    }

    /// Selects the native `ProgressDialog` content layout. The value is a
    /// percentage, so network resource progress is clamped to 0..=100.
    pub fn with_progress(mut self, progress: u8) -> Self {
        self.progress = Some(progress.min(100));
        self
    }

    pub fn set_progress(&mut self, progress: u8) {
        if self.progress.is_some() {
            self.progress = Some(progress.min(100));
        }
    }

    pub const fn progress(&self) -> Option<u8> {
        self.progress
    }

    /// Overrides one stock button label with the active language resource.
    /// C++ constructs OK/Cancel/Yes/No from `IDS_DLG_*` and Retry from
    /// `IDS_BTN_RETRY`, so this lives on the dialog instance rather than in
    /// process-global frontend state. The raw `&` marker is retained for
    /// both underlined drawing and localized accelerator dispatch.
    pub fn set_button_label(&mut self, button: MessageDialogButton, label: impl Into<String>) {
        let label = label.into();
        if let Some((_, existing)) = self
            .button_labels
            .iter_mut()
            .find(|(candidate, _)| *candidate == button)
        {
            *existing = label;
        } else {
            self.button_labels.push((button, label));
        }
    }

    pub fn set_close_tooltip(&mut self, tooltip: impl Into<String>) {
        self.close_tooltip = tooltip.into();
    }

    pub fn set_progress_tooltip(&mut self, tooltip: impl Into<String>) {
        self.progress_tooltip = tooltip.into();
    }

    pub fn button_label(&self, button: MessageDialogButton) -> &str {
        self.button_labels
            .iter()
            .find_map(|(candidate, label)| (*candidate == button).then_some(label.as_str()))
            .unwrap_or_else(|| button.label())
    }

    pub fn with_us_dont_show_again(self, checked: bool) -> Self {
        self.with_checkbox("&Don't display this message in the future.", checked)
    }

    pub fn caption(&self) -> &str {
        &self.caption
    }

    pub fn message(&self) -> &str {
        &self.message
    }

    pub fn set_message(&mut self, message: impl Into<String>) {
        self.message = message.into();
    }

    pub const fn buttons(&self) -> MessageDialogButtons {
        self.buttons
    }

    pub const fn icon(&self) -> MessageDialogIcon {
        self.icon
    }

    pub const fn size(&self) -> MessageDialogSize {
        self.size
    }

    pub const fn default_no(&self) -> bool {
        self.default_no
    }

    pub const fn dialog_offset(&self) -> (i32, i32) {
        self.dialog_offset
    }

    pub fn checkbox_checked(&self) -> Option<bool> {
        self.checkbox.as_ref().map(|checkbox| checkbox.checked)
    }

    pub fn checkbox_focused(&self) -> bool {
        self.focus == Some(DialogTarget::Checkbox)
    }

    pub fn take_checkbox_changes(&mut self) -> Vec<bool> {
        std::mem::take(&mut self.checkbox_changes)
    }

    pub fn focused_button(&self) -> Option<MessageDialogButton> {
        let ordered = self.buttons.ordered();
        match self.focus {
            Some(DialogTarget::Button(index)) => ordered.get(index).copied(),
            _ => None,
        }
    }

    pub fn close_focused(&self) -> bool {
        self.focus == Some(DialogTarget::Close)
    }

    pub fn take_sound_events(&mut self) -> Vec<MessageDialogSound> {
        std::mem::take(&mut self.sound_events)
    }

    pub fn handle_hotkey(&mut self, character: char) -> Option<MessageDialogResult> {
        let character = character.to_ascii_uppercase();
        if self
            .checkbox
            .as_ref()
            .is_some_and(|checkbox| checkbox.hotkey == Some(character))
        {
            self.toggle_checkbox();
            return None;
        }
        self.buttons
            .ordered()
            .into_iter()
            .find(|button| expand_hotkey_markup(self.button_label(*button)).1 == Some(character))
            .map(MessageDialogButton::result)
    }

    pub fn has_hotkey(&self, character: char) -> bool {
        let character = character.to_ascii_uppercase();
        self.checkbox
            .as_ref()
            .is_some_and(|checkbox| checkbox.hotkey == Some(character))
            || self
                .buttons
                .ordered()
                .into_iter()
                .any(|button| expand_hotkey_markup(self.button_label(button)).1 == Some(character))
    }

    pub fn layout(
        &self,
        screen_width: i32,
        screen_height: i32,
        font: &ClonkFont,
    ) -> MessageDialogLayout {
        let width = self.size.width();
        let title_height = if self.caption.is_empty() {
            0
        } else {
            font.line_height.max(MIN_CAPTION_HEIGHT)
        };
        let (unbroken_width, unbroken_height) = font.measure(&self.message, true);
        let is_progress = self.progress.is_some();
        let centered = is_progress
            || self.force_centered_message
            || self.size != MessageDialogSize::Regular
            || (unbroken_width <= width - 140 && unbroken_height <= font.line_height);
        let message_width = if is_progress {
            width - 3 * DIALOG_INDENT - ICON_SIZE
        } else if centered {
            width - 140
        } else {
            width - 80
        };
        let message_text = break_message(font, &self.message, message_width);
        let (_, message_height) = font.measure(&message_text, true);
        let checkbox_size = self.checkbox.as_ref().map(|checkbox| {
            let (label_width, label_height) = font.measure(&checkbox.raw_label, true);
            (label_width + label_height + 4, label_height)
        });
        let (client_height, height) = if is_progress {
            let height = message_height.max(ICON_SIZE) + PROGRESS_VERTICAL_ROOM;
            (height - title_height, height)
        } else {
            let client_height = message_height
                + checkbox_size.map_or(CLIENT_VERTICAL_ROOM, |(_, height)| height + 100);
            (client_height, title_height + client_height)
        };
        let (base_x, base_y) = match self.placement {
            MessageDialogPlacement::Centered => {
                ((screen_width - width) / 2, (screen_height - height) / 2)
            }
            MessageDialogPlacement::Preferred { x, y } => (x + 30, y + 30),
        };
        let x = base_x + self.dialog_offset.0;
        let y = base_y + self.dialog_offset.1;
        let client_y = y + title_height;
        let caption = (title_height > 0).then_some(IntRect::new(x, y, width, title_height));
        let close_button =
            (title_height > 0).then_some(IntRect::new(x + width - 20, y + 4, 16, 16));
        let icon = IntRect::new(
            x + DIALOG_INDENT,
            client_y + DIALOG_INDENT,
            ICON_SIZE,
            ICON_SIZE,
        );
        let message = IntRect::new(
            if is_progress {
                x + 3 * DIALOG_INDENT + ICON_SIZE
            } else {
                x + 70
            },
            client_y + 10,
            if is_progress {
                width - 4 * DIALOG_INDENT - ICON_SIZE
            } else {
                message_width
            },
            message_height,
        );
        let checkbox = checkbox_size.map(|(checkbox_width, checkbox_height)| {
            let bounds = IntRect::new(
                message.x + (message.w - checkbox_width) / 2,
                client_y + message_height + 30,
                checkbox_width,
                checkbox_height,
            );
            MessageDialogCheckboxLayout {
                bounds,
                square: bounds.with_width(checkbox_height),
                label_x: bounds.x + checkbox_height + 4,
            }
        });
        let progress = is_progress.then_some(IntRect::new(
            x + DIALOG_INDENT,
            client_y + client_height
                - PROGRESS_BUTTON_AREA_HEIGHT
                - PROGRESS_HEIGHT
                - 3 * DIALOG_INDENT,
            width - 2 * DIALOG_INDENT,
            PROGRESS_HEIGHT,
        ));
        let ordered = self.buttons.ordered();
        let count = i32::try_from(ordered.len()).unwrap_or(i32::MAX);
        let button_width = if is_progress {
            PROGRESS_BUTTON_WIDTH
        } else {
            BUTTON_WIDTH
        };
        let group_width = if count == 0 {
            0
        } else {
            count * button_width + (count - 1) * BUTTON_GAP
        };
        let button_y = if is_progress {
            client_y + client_height - DIALOG_INDENT - PROGRESS_BUTTON_AREA_HEIGHT
                + (PROGRESS_BUTTON_AREA_HEIGHT - BUTTON_HEIGHT) / 2
        } else {
            client_y
                + message_height
                + checkbox_size.map_or(34, |(_, checkbox_height)| checkbox_height + 54)
        };
        let first_button_x = x + (width - group_width) / 2;
        let buttons = ordered
            .into_iter()
            .enumerate()
            .map(|(index, button)| MessageDialogButtonLayout {
                button,
                rect: IntRect::new(
                    first_button_x
                        + i32::try_from(index).unwrap_or(i32::MAX) * (button_width + BUTTON_GAP),
                    button_y,
                    button_width,
                    BUTTON_HEIGHT,
                ),
            })
            .collect();
        MessageDialogLayout {
            bounds: IntRect::new(x, y, width, height),
            caption,
            close_button,
            icon,
            message,
            message_text,
            message_alignment: if centered {
                TextAlign::Center
            } else {
                TextAlign::Left
            },
            checkbox,
            progress,
            buttons,
        }
    }

    pub fn handle_key_down(
        &mut self,
        key: KeyCode,
        backwards: bool,
    ) -> Option<MessageDialogResult> {
        match key {
            KeyCode::Escape => Some(MessageDialogResult::Dismissed),
            KeyCode::Tab => {
                self.advance_focus(backwards);
                None
            }
            KeyCode::Space if self.checkbox_focused() => {
                self.toggle_checkbox();
                None
            }
            KeyCode::Enter if self.focus.is_none() || self.checkbox_focused() => {
                self.positive_result()
            }
            KeyCode::Enter | KeyCode::Space => {
                if self.key_pressed.is_none() {
                    self.key_pressed = self.focus;
                    if self.key_pressed.is_some() {
                        self.sound_events.push(MessageDialogSound::ArrowHit);
                    }
                }
                None
            }
            _ => None,
        }
    }

    pub fn handle_key_up(&mut self, key: KeyCode) -> Option<MessageDialogResult> {
        if !matches!(key, KeyCode::Enter | KeyCode::Space) {
            return None;
        }
        let target = self.key_pressed.take()?;
        (Some(target) == self.focus).then(|| {
            self.sound_events.push(MessageDialogSound::Click);
            self.target_result(target)
        })
    }

    pub fn handle_gamepad_low_down(&mut self) -> Option<MessageDialogResult> {
        if self.checkbox_focused() {
            self.toggle_checkbox();
            None
        } else {
            self.handle_key_down(KeyCode::Enter, false)
        }
    }

    pub fn handle_gamepad_low_up(&mut self) -> Option<MessageDialogResult> {
        self.handle_key_up(KeyCode::Enter)
    }

    pub fn handle_pointer_move(&mut self, point: GuiPoint, layout: &MessageDialogLayout) {
        let was_down = self.pointer_target_is_down();
        self.note_pointer_input(point);
        if let Some(drag) = self.title_drag {
            self.dialog_offset = (
                drag.offset.0 + (point.x - drag.pointer.x) as i32,
                drag.offset.1 + (point.y - drag.pointer.y) as i32,
            );
            return;
        }
        self.hovered = hit_target(layout, point);
        if was_down != self.pointer_target_is_down() {
            self.sound_events.push(MessageDialogSound::ArrowHit);
        }
    }

    pub fn handle_pointer_down(&mut self, layout: &MessageDialogLayout) {
        let was_down = self.pointer_target_is_down();
        if let Some(point) = self.pointer {
            self.note_pointer_input(point);
        }
        let target = self.pointer.and_then(|point| hit_target(layout, point));
        if target == Some(DialogTarget::Checkbox) {
            self.pointer_pressed = None;
            return;
        }
        if target.is_none()
            && self
                .pointer
                .is_some_and(|point| caption_contains(layout, point))
        {
            self.pointer_pressed = None;
            self.title_drag = self.pointer.map(|pointer| TitleDrag {
                pointer,
                offset: self.dialog_offset,
            });
            return;
        }
        self.pointer_pressed = target;
        if !was_down && self.pointer_pressed.is_some() {
            self.sound_events.push(MessageDialogSound::ArrowHit);
        }
    }

    /// Route a button-down at its event coordinate. A dialog can be inserted
    /// beneath a stationary cursor, so its cached pointer position is not
    /// necessarily initialized before the first click reaches it.
    pub fn handle_pointer_down_at(&mut self, point: GuiPoint, layout: &MessageDialogLayout) {
        self.pointer = Some(point);
        self.hovered = hit_target(layout, point);
        self.handle_pointer_down(layout);
    }

    pub fn handle_pointer_up(
        &mut self,
        layout: &MessageDialogLayout,
    ) -> Option<MessageDialogResult> {
        let was_down = self.pointer_target_is_down();
        if let Some(point) = self.pointer {
            self.note_pointer_input(point);
            self.finish_title_drag_at(point);
        }
        let released = self.pointer.and_then(|point| hit_target(layout, point));
        self.finish_pointer_up(released, was_down)
    }

    /// Route a button-up at its event coordinate without synthesizing a
    /// preceding move. This matters after CMouse cleared `pDragElement`: a
    /// button left earlier must not re-arm merely because release hit it,
    /// while a checkbox still toggles from the LeftUp coordinate alone.
    pub fn handle_pointer_up_at(
        &mut self,
        point: GuiPoint,
        layout: &MessageDialogLayout,
    ) -> Option<MessageDialogResult> {
        let was_down = self.pointer_target_is_down();
        self.note_pointer_input(point);
        self.finish_title_drag_at(point);
        let released = hit_target(layout, point);
        self.hovered = released;
        self.finish_pointer_up(released, was_down)
    }

    /// Apply the retained title's final pointer delta before ordinary
    /// top-down release hit-testing, matching `Screen::MouseInput`.
    pub fn stop_pointer_drag_at(&mut self, point: GuiPoint) {
        self.note_pointer_input(point);
        self.finish_title_drag_at(point);
    }

    fn finish_pointer_up(
        &mut self,
        released: Option<DialogTarget>,
        was_down: bool,
    ) -> Option<MessageDialogResult> {
        if released == Some(DialogTarget::Checkbox) {
            self.pointer_pressed = None;
            self.toggle_checkbox();
            return None;
        }
        let pressed = self.pointer_pressed.take()?;
        (was_down && released == Some(pressed)).then(|| {
            self.sound_events.push(MessageDialogSound::Click);
            self.target_result(pressed)
        })
    }

    /// Whether a classic button/close-button press is retaining pointer moves.
    ///
    /// `C4GUI::Button` installs itself as `CMouse::pDragElement` on left-down,
    /// so a shared (non-exclusive) screen continues routing movement to that
    /// dialog after the pointer leaves its bounds. `CMouse` clears the drag
    /// element before hit-testing the matching release. Checkboxes deliberately
    /// do not retain the pointer at all.
    pub const fn has_pointer_capture(&self) -> bool {
        self.pointer_pressed.is_some() || self.title_drag.is_some()
    }

    pub const fn has_positional_pointer_drag(&self) -> bool {
        self.title_drag.is_some()
    }

    pub const fn has_pointer_hover(&self) -> bool {
        self.hovered.is_some()
    }

    /// Clear a retained button press when shared-screen release hit-testing
    /// selects a lower dialog or the game world.
    pub fn cancel_pointer_capture(&mut self) {
        let was_down = self.pointer_target_is_down();
        self.pointer = None;
        self.hovered = None;
        self.pointer_pressed = None;
        self.title_drag = None;
        if was_down {
            self.sound_events.push(MessageDialogSound::ArrowHit);
        }
    }

    pub fn pointer_left(&mut self) {
        let was_down = self.pointer_target_is_down();
        self.pointer = None;
        self.hovered = None;
        if was_down {
            self.sound_events.push(MessageDialogSound::ArrowHit);
        }
    }

    pub fn cancel_interaction(&mut self) {
        self.pointer = None;
        self.hovered = None;
        self.pointer_pressed = None;
        self.title_drag = None;
        self.key_pressed = None;
        self.sound_events.clear();
    }

    /// Global tooltip state for the title label installed by
    /// `Dialog::SetTitle`. The host supplies the pointer only after the
    /// process-level [`crate::context_menu::ClassicTooltipTracker`] reaches
    /// its shared 500ms threshold.
    pub fn tooltip_state(
        &self,
        eligible_pointer: Option<GuiPoint>,
        layout: &MessageDialogLayout,
    ) -> Option<MessageDialogTooltip> {
        let pointer = eligible_pointer?;
        let routed_pointer = self.pointer?;
        if routed_pointer.x as i32 != pointer.x as i32
            || routed_pointer.y as i32 != pointer.y as i32
        {
            return None;
        }
        let text = if layout
            .progress
            .is_some_and(|progress| rect_contains(progress, pointer))
        {
            &self.progress_tooltip
        } else {
            match hit_target(layout, pointer) {
                Some(DialogTarget::Close) => &self.close_tooltip,
                None if caption_contains(layout, pointer) => &self.caption,
                Some(DialogTarget::Checkbox | DialogTarget::Button(_)) | None => return None,
            }
        };
        (!text.is_empty()).then(|| MessageDialogTooltip {
            pointer,
            text: text.clone(),
        })
    }

    /// Draw the title tooltip in the host's screen-global final overlay pass.
    pub fn render_tooltip_at(
        &self,
        surface: &mut Surface,
        resources: MessageDialogResources<'_>,
        eligible_pointer: Option<GuiPoint>,
        gamma: Option<&GammaRamp>,
    ) -> Result<()> {
        resources.validate()?;
        let layout = self.layout(
            surface.width() as i32,
            surface.height() as i32,
            &resources.fonts.text,
        );
        if let Some(tooltip) = self.tooltip_state(eligible_pointer, &layout) {
            draw_classic_tooltip(
                surface,
                resources.tooltip_font,
                tooltip.pointer,
                &tooltip.text,
                gamma,
            );
        }
        Ok(())
    }

    pub fn render_tooltip(
        &self,
        surface: &mut Surface,
        resources: MessageDialogResources<'_>,
        eligible_pointer: Option<GuiPoint>,
        gamma: Option<&GammaRamp>,
    ) -> Result<()> {
        self.render_tooltip_at(surface, resources, eligible_pointer, gamma)
    }

    pub fn render(
        &self,
        surface: &mut Surface,
        resources: MessageDialogResources<'_>,
        keyboard_active: bool,
        mouse_active: bool,
        gamma: Option<&GammaRamp>,
    ) -> Result<()> {
        self.render_at(
            surface,
            resources,
            keyboard_active,
            mouse_active,
            gamma,
            Instant::now(),
        )
    }

    pub fn render_at(
        &self,
        surface: &mut Surface,
        resources: MessageDialogResources<'_>,
        keyboard_active: bool,
        mouse_active: bool,
        gamma: Option<&GammaRamp>,
        now: Instant,
    ) -> Result<()> {
        resources.validate()?;
        if self.progress.is_some() {
            validate_progress_image("GUIProgress.png", resources.progress)?;
        }
        let layout = self.layout(
            surface.width() as i32,
            surface.height() as i32,
            &resources.fonts.text,
        );
        resources.skin.draw_dialog(surface, layout.bounds, gamma);
        if let Some(caption) = layout.caption {
            let scroll = self.caption_scroll_offset_at(now, &resources.fonts.text);
            resources.skin.draw_caption_scrolled(
                surface,
                caption,
                &self.caption,
                &resources.fonts.text,
                [255, 255, 255, 255],
                TextAlign::Left,
                TITLE_RIGHT_INDENT,
                scroll,
                gamma,
            );
        }
        draw_dialog_icon(
            surface,
            layout.icon,
            self.icon,
            resources.icons,
            resources.icons_extended,
            gamma,
        )?;
        let message_x = match layout.message_alignment {
            TextAlign::Left => layout.message.x,
            TextAlign::Center => layout.message.x + layout.message.w / 2,
            TextAlign::Right => layout.message.x + layout.message.w,
        };
        resources.fonts.text.draw_with_gamma(
            surface,
            message_x,
            layout.message.y,
            &layout.message_text,
            [255, 255, 255, 255],
            layout.message_alignment,
            true,
            gamma,
        );
        if let (Some(progress_rect), Some(progress)) = (layout.progress, self.progress) {
            draw_progress_bar(
                surface,
                progress_rect,
                progress,
                resources.progress,
                &resources.fonts.text,
                gamma,
            );
        }
        if let (Some(checkbox_layout), Some(checkbox)) =
            (layout.checkbox.as_ref(), self.checkbox.as_ref())
        {
            draw_checkbox(
                surface,
                checkbox_layout.square,
                checkbox.checked,
                resources.checkbox,
                gamma,
            )?;
            resources.fonts.text.draw_with_gamma(
                surface,
                checkbox_layout.label_x,
                checkbox_layout.bounds.y
                    + (checkbox_layout.bounds.h - resources.fonts.text.line_height).max(0) / 2,
                &checkbox.expanded_label,
                [255, 255, 255, 255],
                TextAlign::Left,
                true,
                gamma,
            );
            if keyboard_active && self.focus == Some(DialogTarget::Checkbox)
                || mouse_active && self.hovered == Some(DialogTarget::Checkbox)
            {
                let highlight_size = checkbox_layout.square.h / 2;
                crate::draw_image_bilinear_additive(
                    surface,
                    &GuiRect::new(
                        (checkbox_layout.square.x + checkbox_layout.square.w / 4) as f32,
                        (checkbox_layout.square.y + checkbox_layout.square.h / 4) as f32,
                        highlight_size as f32,
                        highlight_size as f32,
                    ),
                    resources.button_highlight,
                    gamma,
                );
            }
        }
        for (index, button) in layout.buttons.iter().enumerate() {
            let target = DialogTarget::Button(index);
            resources.skin.draw_button(
                surface,
                button.rect,
                self.button_label(button.button),
                resources.fonts,
                ClassicButtonState {
                    pressed: self.target_pressed(target),
                    highlighted: keyboard_active && self.focus == Some(target)
                        || mouse_active && self.hovered == Some(target),
                },
                gamma,
            );
        }
        if let Some(close) = layout.close_button {
            let target = DialogTarget::Close;
            let highlighted = keyboard_active && self.focus == Some(target)
                || mouse_active && self.hovered == Some(target);
            if highlighted {
                crate::draw_image_bilinear_additive(
                    surface,
                    &GuiRect::new(
                        close.x as f32,
                        close.y as f32,
                        close.w as f32,
                        close.h as f32,
                    ),
                    resources.button_highlight,
                    gamma,
                );
            }
            draw_dialog_icon(
                surface,
                close,
                MessageDialogIcon::Standard(CLOSE_ICON_PHASE),
                resources.icons,
                resources.icons_extended,
                gamma,
            )?;
            if self.target_pressed(target) {
                crate::draw_image_bilinear_additive(
                    surface,
                    &GuiRect::new(
                        close.x as f32,
                        close.y as f32,
                        close.w as f32,
                        close.h as f32,
                    ),
                    resources.button_highlight,
                    gamma,
                );
            }
        }
        Ok(())
    }

    fn advance_focus(&mut self, backwards: bool) {
        let mut targets = Vec::new();
        if !self.caption.is_empty() {
            targets.push(DialogTarget::Close);
        }
        if self.checkbox.is_some() {
            targets.push(DialogTarget::Checkbox);
        }
        targets.extend(
            self.buttons
                .ordered()
                .iter()
                .enumerate()
                .map(|(index, _)| DialogTarget::Button(index)),
        );
        if targets.is_empty() {
            self.focus = None;
            return;
        }
        let current = self
            .focus
            .and_then(|focus| targets.iter().position(|target| *target == focus));
        let next = match (current, backwards) {
            (Some(0), true) | (None, true) => targets.len() - 1,
            (Some(index), true) => index - 1,
            (Some(index), false) => (index + 1) % targets.len(),
            (None, false) => 0,
        };
        self.focus = Some(targets[next]);
        self.key_pressed = None;
    }

    fn positive_result(&self) -> Option<MessageDialogResult> {
        self.buttons
            .ordered()
            .into_iter()
            .find(|button| matches!(button, MessageDialogButton::Ok))
            .or_else(|| {
                self.buttons
                    .ordered()
                    .into_iter()
                    .find(|button| matches!(button, MessageDialogButton::Yes))
            })
            .map(MessageDialogButton::result)
    }

    fn toggle_checkbox(&mut self) {
        let Some(checkbox) = self.checkbox.as_mut() else {
            return;
        };
        checkbox.checked = !checkbox.checked;
        self.checkbox_changes.push(checkbox.checked);
        self.sound_events.push(MessageDialogSound::ArrowHit);
    }

    fn target_result(&self, target: DialogTarget) -> MessageDialogResult {
        match target {
            DialogTarget::Close => MessageDialogResult::Dismissed,
            DialogTarget::Checkbox => MessageDialogResult::Dismissed,
            DialogTarget::Button(index) => self
                .buttons
                .ordered()
                .get(index)
                .copied()
                .map(MessageDialogButton::result)
                .unwrap_or(MessageDialogResult::Dismissed),
        }
    }

    fn target_pressed(&self, target: DialogTarget) -> bool {
        self.key_pressed == Some(target)
            || (self.pointer_pressed == Some(target) && self.hovered == Some(target))
    }

    fn pointer_target_is_down(&self) -> bool {
        self.pointer_pressed.is_some() && self.pointer_pressed == self.hovered
    }

    fn note_pointer_input(&mut self, point: GuiPoint) {
        self.pointer = Some(point);
    }

    fn finish_title_drag_at(&mut self, point: GuiPoint) {
        if let Some(drag) = self.title_drag.take() {
            self.dialog_offset = (
                drag.offset.0 + (point.x - drag.pointer.x) as i32,
                drag.offset.1 + (point.y - drag.pointer.y) as i32,
            );
        }
    }

    fn caption_scroll_offset_at(&self, now: Instant, font: &ClonkFont) -> i32 {
        if self.caption.is_empty() {
            return 0;
        }
        let max_scroll =
            (font.measure(&self.caption, true).0 + TITLE_LEFT_INDENT + TITLE_RIGHT_INDENT
                - self.size.width())
            .max(0);
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
}

fn initial_focus_button(buttons: &[MessageDialogButton], default_no: bool) -> Option<usize> {
    let find = |needle| buttons.iter().position(|button| *button == needle);
    if !default_no {
        if let Some(index) = find(MessageDialogButton::Ok) {
            return Some(index);
        }
    }
    if let Some(index) = find(MessageDialogButton::Retry) {
        return Some(index);
    }
    if let Some(index) = find(MessageDialogButton::Cancel) {
        return Some(index);
    }
    if !default_no {
        if let Some(index) = find(MessageDialogButton::Yes) {
            return Some(index);
        }
    }
    find(MessageDialogButton::No).or_else(|| find(MessageDialogButton::Restart))
}

fn hit_target(layout: &MessageDialogLayout, point: GuiPoint) -> Option<DialogTarget> {
    if layout
        .close_button
        .is_some_and(|rect| rect_contains(rect, point))
    {
        return Some(DialogTarget::Close);
    }
    if layout
        .checkbox
        .as_ref()
        .is_some_and(|checkbox| checkbox_contains(checkbox.square, point))
    {
        return Some(DialogTarget::Checkbox);
    }
    layout
        .buttons
        .iter()
        .position(|button| rect_contains(button.rect, point))
        .map(DialogTarget::Button)
}

fn caption_contains(layout: &MessageDialogLayout, point: GuiPoint) -> bool {
    layout
        .caption
        .is_some_and(|caption| rect_contains(caption, point))
}

fn rect_contains(rect: IntRect, point: GuiPoint) -> bool {
    point.x >= rect.x as f32
        && point.x < (rect.x + rect.w) as f32
        && point.y >= rect.y as f32
        && point.y < (rect.y + rect.h) as f32
}

fn checkbox_contains(rect: IntRect, point: GuiPoint) -> bool {
    point.x >= rect.x as f32
        && point.x <= (rect.x + rect.w) as f32
        && point.y >= rect.y as f32
        && point.y < (rect.y + rect.h) as f32
}

fn draw_dialog_icon(
    surface: &mut Surface,
    rect: IntRect,
    icon: MessageDialogIcon,
    icons: &ImageData,
    icons_extended: &ImageData,
    gamma: Option<&GammaRamp>,
) -> Result<()> {
    let (sheet, phase, cell) = match icon {
        MessageDialogIcon::None => return Ok(()),
        MessageDialogIcon::Standard(phase) => (icons, u32::from(phase), 40_u32),
        MessageDialogIcon::Extended(phase) => (icons_extended, u32::from(phase), 64_u32),
    };
    let columns = sheet.width() / cell;
    let src_x = (phase % columns) * cell;
    let src_y = (phase / columns) * cell;
    anyhow::ensure!(
        src_x + cell <= sheet.width() && src_y + cell <= sheet.height(),
        "classic message-dialog icon phase {phase} is outside the {}x{} sheet",
        sheet.width(),
        sheet.height()
    );
    draw_facet_stretch(
        surface,
        sheet,
        (src_x as f32, src_y as f32, cell as f32, cell as f32),
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
    let phase = u32::from(checked);
    let src_x = phase * cell;
    anyhow::ensure!(
        cell > 0 && src_x + cell <= sheet.width(),
        "GUICheckbox.png does not contain enabled phase {phase}"
    );
    draw_facet_stretch(
        surface,
        sheet,
        (src_x as f32, 0.0, cell as f32, cell as f32),
        (rect.x as f32, rect.y as f32, rect.w as f32, rect.h as f32),
        gamma,
    );
    Ok(())
}

fn draw_progress_bar(
    surface: &mut Surface,
    rect: IntRect,
    progress: u8,
    image: &ImageData,
    font: &ClonkFont,
    gamma: Option<&GammaRamp>,
) {
    draw_3d_frame(surface, rect, gamma);
    let fill_width = (rect.w - 4).max(0) * i32::from(progress) / 100;
    if fill_width > 0 && rect.h > 2 {
        draw_facet_stretch(
            surface,
            image,
            (
                1.0,
                0.0,
                image.width().saturating_sub(2) as f32,
                image.height() as f32,
            ),
            (
                (rect.x + 2) as f32,
                (rect.y + 2) as f32,
                fill_width as f32,
                (rect.h - 2) as f32,
            ),
            gamma,
        );
    }
    font.draw_with_gamma(
        surface,
        rect.x + rect.w / 2,
        rect.y + (rect.h - font.line_height) / 2 - 1,
        &format!("{progress}%"),
        [255, 255, 255, 255],
        TextAlign::Center,
        true,
        gamma,
    );
}

fn validate_icon_sheet(name: &str, image: &ImageData, cell: u32) -> Result<()> {
    anyhow::ensure!(
        image.width() >= cell && image.height() >= cell,
        "{name} must contain at least one {cell}x{cell} classic icon, got {}x{}",
        image.width(),
        image.height()
    );
    Ok(())
}

fn validate_nonempty_image(name: &str, image: &ImageData) -> Result<()> {
    anyhow::ensure!(
        image.width() > 0 && image.height() > 0,
        "{name} must not be empty for classic message dialogs"
    );
    Ok(())
}

fn validate_checkbox_sheet(name: &str, image: &ImageData) -> Result<()> {
    anyhow::ensure!(
        image.height() > 0 && image.width() >= image.height().saturating_mul(2),
        "{name} must contain the enabled unchecked/checked square phases, got {}x{}",
        image.width(),
        image.height()
    );
    Ok(())
}

fn validate_progress_image(name: &str, image: &ImageData) -> Result<()> {
    anyhow::ensure!(
        image.width() >= 3 && image.height() > 0,
        "{name} must contain the classic progress facet, got {}x{}",
        image.width(),
        image.height()
    );
    Ok(())
}

#[derive(Clone, Debug)]
enum MessageToken {
    Text {
        raw: String,
        width: f32,
        break_kind: Option<bool>,
        line_character: bool,
        source_end: usize,
    },
    HardBreak {
        delimiter: char,
        source_end: usize,
    },
}

impl MessageToken {
    fn width(&self) -> f32 {
        match self {
            Self::Text { width, .. } => *width,
            Self::HardBreak { .. } => 0.0,
        }
    }

    fn break_kind(&self) -> Option<bool> {
        match self {
            Self::Text { break_kind, .. } => *break_kind,
            Self::HardBreak { .. } => None,
        }
    }

    fn source_end(&self) -> usize {
        match self {
            Self::Text { source_end, .. } | Self::HardBreak { source_end, .. } => *source_end,
        }
    }

    fn append_to(&self, output: &mut String) {
        if let Self::Text { raw, .. } = self {
            output.push_str(raw);
        }
    }

    fn is_line_character(&self) -> bool {
        matches!(
            self,
            Self::Text {
                line_character: true,
                ..
            }
        )
    }
}

#[derive(Clone, Copy, Debug)]
enum MessageBreak {
    Automatic,
    Manual(char),
}

#[derive(Clone, Debug)]
struct MessageSegment {
    tokens: Vec<MessageToken>,
    break_after: Option<MessageBreak>,
}

/// Optional `CStdFont::BreakMessage` parameters. Existing helpers use these
/// defaults, matching C++'s `fZoom=1.0` and unlimited `maxLines=0`.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct BreakMessageOptions {
    pub zoom: f32,
    pub max_lines: usize,
    pub markup: bool,
}

impl Default for BreakMessageOptions {
    fn default() -> Self {
        Self {
            zoom: 1.0,
            max_lines: 0,
            markup: true,
        }
    }
}

/// `CStdFont::BreakMessage`'s character-level line breaking for ordinary GUI
/// labels. Automatic wraps close and reopen active markup so each physical
/// line is self-contained, while manual `\n` and `|` delimiters stay verbatim.
pub fn break_message(font: &ClonkFont, text: &str, max_width: i32) -> String {
    break_message_with_options(font, text, max_width, BreakMessageOptions::default())
}

pub fn break_message_with_options(
    font: &ClonkFont,
    text: &str,
    max_width: i32,
    options: BreakMessageOptions,
) -> String {
    break_message_impl(font, text, max_width, None, options)
}

/// [`CStdFont::BreakMessage`](../../../../src/StdFont.cpp) using the in-game
/// HUD font abstraction. Inline images have no provider on this path and
/// therefore occupy zero width, matching an unhooked C++ font image source.
pub fn break_hud_message(font: &HudFont<'_>, text: &str, max_width: i32) -> String {
    break_hud_message_max_lines(font, text, max_width, 0)
}

pub(crate) fn break_hud_message_max_lines(
    font: &HudFont<'_>,
    text: &str,
    max_width: i32,
    max_lines: usize,
) -> String {
    break_message_in_units(
        text,
        max_width as f32,
        max_lines,
        true,
        |character| {
            if character >= ' ' {
                font.character_advance(character) as f32
            } else {
                0.0
            }
        },
        |_| 0.0,
    )
}

pub fn break_message_with_images(
    font: &ClonkFont,
    text: &str,
    max_width: i32,
    images: &dyn FontImageProvider,
) -> String {
    break_message_with_images_and_options(
        font,
        text,
        max_width,
        images,
        BreakMessageOptions::default(),
    )
}

pub fn break_message_with_images_and_options(
    font: &ClonkFont,
    text: &str,
    max_width: i32,
    images: &dyn FontImageProvider,
    options: BreakMessageOptions,
) -> String {
    break_message_impl(font, text, max_width, Some(images), options)
}

fn break_message_impl(
    font: &ClonkFont,
    text: &str,
    max_width: i32,
    images: Option<&dyn FontImageProvider>,
    options: BreakMessageOptions,
) -> String {
    break_message_in_units(
        text,
        max_width as f32,
        options.max_lines,
        options.markup,
        |character| {
            if character >= ' ' {
                let advance = font.message_character_advance(character);
                options.zoom * (advance - font.h_space) as f32 + font.h_space as f32
            } else {
                0.0
            }
        },
        |tag| {
            images
                .and_then(|provider| provider.font_image(font_image_lookup_tag(tag)))
                .map_or(0, |image| scaled_font_image_width(font.cell_height, image))
                as f32
        },
    )
}

/// Scale-native `CStdFont::BreakMessage`. Character widths stay in physical
/// numerator units until the comparison against the GUI-unit width, matching
/// C++'s float accumulation without prematurely truncating each glyph.
pub fn break_native_message(font: &NativeClonkFont, text: &str, max_width: i32) -> String {
    break_native_message_with_options(font, text, max_width, BreakMessageOptions::default())
}

pub fn break_native_message_with_options(
    font: &NativeClonkFont,
    text: &str,
    max_width: i32,
    options: BreakMessageOptions,
) -> String {
    break_native_message_impl(font, text, max_width, None, options)
}

pub fn break_native_message_with_images(
    font: &NativeClonkFont,
    text: &str,
    max_width: i32,
    images: &dyn FontImageProvider,
) -> String {
    break_native_message_with_images_and_options(
        font,
        text,
        max_width,
        images,
        BreakMessageOptions::default(),
    )
}

pub fn break_native_message_with_images_and_options(
    font: &NativeClonkFont,
    text: &str,
    max_width: i32,
    images: &dyn FontImageProvider,
    options: BreakMessageOptions,
) -> String {
    break_native_message_impl(font, text, max_width, Some(images), options)
}

fn break_native_message_impl(
    font: &NativeClonkFont,
    text: &str,
    max_width: i32,
    images: Option<&dyn FontImageProvider>,
    options: BreakMessageOptions,
) -> String {
    let units_per_pixel = font.message_width_units_per_gui_pixel();
    let max_width_units = max_width.saturating_mul(units_per_pixel);
    let spacing_units = -units_per_pixel;
    break_message_in_units(
        text,
        max_width_units as f32,
        options.max_lines,
        options.markup,
        |character| {
            if character < ' ' {
                return 0.0;
            }
            let advance = font.message_character_advance_units(character);
            options.zoom * (advance - spacing_units) as f32 + spacing_units as f32
        },
        |tag| {
            images
                .and_then(|provider| provider.font_image(font_image_lookup_tag(tag)))
                .map_or(0, |image| font.message_image_advance_units(image)) as f32
        },
    )
}

fn break_message_in_units(
    text: &str,
    max_width: f32,
    max_lines: usize,
    markup: bool,
    mut character_width: impl FnMut(char) -> f32,
    mut image_width: impl FnMut(&str) -> f32,
) -> String {
    let mut tokens = Vec::new();
    let mut rest = text;
    while !rest.is_empty() {
        let source_start = text.len() - rest.len();
        if let Some(advance) = markup.then(|| skip_markup_tag(rest)).flatten() {
            tokens.push(MessageToken::Text {
                raw: rest[..advance].to_string(),
                width: 0.0,
                break_kind: None,
                line_character: false,
                source_end: source_start + advance,
            });
            rest = &rest[advance..];
            continue;
        }
        if let Some((tag, advance)) = markup.then(|| inline_image_token(rest)).flatten() {
            tokens.push(MessageToken::Text {
                raw: rest[..advance].to_string(),
                width: image_width(tag),
                break_kind: None,
                line_character: true,
                source_end: source_start + advance,
            });
            rest = &rest[advance..];
            continue;
        }
        let character = rest.chars().next().expect("non-empty message");
        rest = &rest[character.len_utf8()..];
        if character == '\n' || (markup && character == '|') {
            tokens.push(MessageToken::HardBreak {
                delimiter: character,
                source_end: source_start + character.len_utf8(),
            });
            continue;
        }
        let width = character_width(character);
        let break_kind = if character.is_ascii_whitespace() {
            Some(false)
        } else if character == '-' {
            Some(true)
        } else {
            None
        };
        tokens.push(MessageToken::Text {
            raw: character.to_string(),
            width,
            break_kind,
            line_character: true,
            source_end: source_start + character.len_utf8(),
        });
    }

    let mut segments = Vec::new();
    let mut line = Vec::new();
    let mut line_width = 0.0_f32;
    let mut last_break: Option<(usize, bool)> = None;
    let mut last_emergency_break = 0_usize;
    let mut first_line_character = true;
    let mut breaks_seen = 0_usize;
    let mut early_tail = None;
    for token in tokens {
        let token = match token {
            MessageToken::HardBreak {
                delimiter,
                source_end,
            } => {
                segments.push(MessageSegment {
                    tokens: std::mem::take(&mut line),
                    break_after: Some(MessageBreak::Manual(delimiter)),
                });
                breaks_seen += 1;
                if max_lines != 0 && breaks_seen == max_lines {
                    early_tail = Some(text[source_end..].to_string());
                    break;
                }
                line_width = 0.0;
                last_break = None;
                last_emergency_break = 0;
                first_line_character = true;
                continue;
            }
            token => token,
        };
        let source_end = token.source_end();
        let width = token.width();
        let token_break = token.break_kind();
        let line_character = token.is_line_character();
        line.push(token);
        if !line_character {
            continue;
        }
        line_width += width;
        let was_first_line_character = first_line_character;
        if line_width <= max_width || was_first_line_character {
            if let Some(include) = token_break {
                last_break = Some((line.len() - 1, include || was_first_line_character));
            }
            last_emergency_break = line.len();
            first_line_character = false;
            continue;
        }

        let current_is_space = token_break == Some(false);
        let (split_at, skip) = if current_is_space {
            (line.len() - 1, 1)
        } else if let Some((index, include)) = last_break {
            if include {
                (index + 1, 0)
            } else {
                (index, 1)
            }
        } else {
            (last_emergency_break, 0)
        };
        let mut remainder = line.split_off(split_at);
        if skip > 0 && !remainder.is_empty() {
            remainder.remove(0);
        }
        segments.push(MessageSegment {
            tokens: std::mem::take(&mut line),
            break_after: Some(MessageBreak::Automatic),
        });
        breaks_seen += 1;
        if max_lines != 0 && breaks_seen == max_lines {
            let mut tail = String::new();
            for token in &remainder {
                token.append_to(&mut tail);
            }
            tail.push_str(&text[source_end..]);
            early_tail = Some(tail);
            break;
        }
        line = remainder;
        line_width = line.iter().fold(0.0_f32, |sum, token| sum + token.width());
        // `CStdFont::BreakMessage` deliberately resets both the normal break
        // candidate and its first-character flag after an automatic split,
        // even when already-scanned remainder text occupies the new line.
        // Consequently the next scanned character is admitted regardless of
        // width and old spaces in the remainder are not reused as candidates.
        last_break = None;
        last_emergency_break = 0;
        first_line_character = true;
    }
    if early_tail.is_none() {
        segments.push(MessageSegment {
            tokens: line,
            break_after: None,
        });
    }
    let mut output = String::new();
    for segment in segments {
        for token in &segment.tokens {
            token.append_to(&mut output);
        }
        match segment.break_after {
            Some(MessageBreak::Automatic) => {
                if markup {
                    let current_line = output.rsplit('\n').next().unwrap_or(&output);
                    let (closing, reopening) = active_markup_fragments(current_line);
                    output.push_str(&closing);
                    output.push('\n');
                    output.push_str(&reopening);
                } else {
                    output.push('\n');
                }
            }
            Some(MessageBreak::Manual(delimiter)) => output.push(delimiter),
            None => {}
        }
    }
    if let Some(tail) = early_tail {
        output.push_str(&tail);
    }
    output
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::classic_gui::blacken_transparent_pixels;
    use crate::test_support::{endeavour_font_set, load_graphics_png, standard_gamma};
    use clonk_graphics::{Color, PixelFormat};

    struct TestFontImages {
        image: ImageData,
    }

    fn unit_width_font(characters: &str) -> ClonkFont {
        let mut font = ClonkFont::new(3);
        font.h_space = 0;
        for character in characters.chars() {
            font.add_glyph(
                character,
                clonk_graphics::clonk_font::GlyphCell {
                    width: 1,
                    pixels: vec![Color::opaque(255, 255, 255); 4],
                },
            );
        }
        font
    }

    impl FontImageProvider for TestFontImages {
        fn font_image(&self, tag: &str) -> Option<clonk_graphics::clonk_font::FontImageRef<'_>> {
            (tag == "FLAM").then_some(clonk_graphics::clonk_font::FontImageRef {
                width: self.image.width(),
                height: self.image.height(),
                rgba: self.image.pixels(),
            })
        }
    }

    fn ok_dialog(message: &str) -> MessageDialogState {
        MessageDialogState::regular_ok(message, "Cannot join game", MessageDialogIcon::ERROR)
    }

    #[test]
    fn geometry_matches_component_aligner_for_centered_and_wrapped_text() {
        let fonts = endeavour_font_set();
        let short = ok_dialog("Short message");
        let layout = short.layout(1280, 720, &fonts.text);
        assert_eq!(layout.bounds.w, 500);
        assert_eq!(layout.caption.expect("caption").h, 23);
        assert_eq!(layout.icon.x - layout.bounds.x, 10);
        assert_eq!(layout.icon.y - layout.bounds.y, 33);
        assert_eq!(layout.message.x - layout.bounds.x, 70);
        assert_eq!(layout.message.w, 360);
        assert_eq!(layout.message_alignment, TextAlign::Center);
        assert_eq!(layout.buttons[0].rect.w, 120);
        assert_eq!(layout.buttons[0].rect.h, 32);
        assert_eq!(layout.buttons[0].rect.x - layout.bounds.x, 190);
        assert_eq!(layout.bounds.h, 23 + layout.message.h + 80);

        let long = ok_dialog(
            "No reference selected. Select a game from the list or enter a direct join address below!",
        );
        let layout = long.layout(1280, 720, &fonts.text);
        assert_eq!(layout.message.w, 420);
        assert_eq!(layout.message_alignment, TextAlign::Left);
        assert!(layout.message_text.contains('\n'));
        assert_eq!(layout.bounds.h, 23 + layout.message.h + 80);
    }

    #[test]
    fn forced_centered_message_keeps_constructor_alignment_after_text_update() {
        // C4Network2::ReadyCheckDialog constructs TimedDialog with an empty
        // message, which fixes ACenter in MessageDialog; SetText later changes
        // the two-line text without changing that alignment
        // (src/C4Network2.cpp:129-149; src/C4GuiDialogs.cpp:891-924,1279-1309).
        let fonts = endeavour_font_set();
        let mut dialog = MessageDialogState::new(
            "",
            "Are you ready?",
            MessageDialogButtons::YES_NO,
            MessageDialogIcon::Standard(30),
            MessageDialogSize::Regular,
            false,
        )
        .with_centered_message();
        dialog.set_message("The host wants to know whether you're ready.|15 seconds remaining.");

        let layout = dialog.layout(1280, 720, &fonts.text);

        assert_eq!(layout.message_alignment, TextAlign::Center);
        assert_eq!(layout.message.w, 360);
        assert!(layout.message_text.contains('|'));
        assert!(!layout.message_text.contains('\n'));
    }

    #[test]
    fn all_sizes_and_button_order_match_cpp() {
        let fonts = endeavour_font_set();
        let buttons = MessageDialogButtons::OK
            | MessageDialogButtons::RETRY
            | MessageDialogButtons::CANCEL
            | MessageDialogButtons::YES
            | MessageDialogButtons::RESTART
            | MessageDialogButtons::NO;
        for (size, width) in [
            (MessageDialogSize::Regular, 500),
            (MessageDialogSize::Medium, 360),
            (MessageDialogSize::Small, 300),
        ] {
            let dialog = MessageDialogState::new(
                "message",
                "caption",
                buttons,
                MessageDialogIcon::None,
                size,
                false,
            );
            let layout = dialog.layout(1280, 720, &fonts.text);
            assert_eq!(layout.bounds.w, width);
            assert_eq!(
                layout
                    .buttons
                    .iter()
                    .map(|button| button.button)
                    .collect::<Vec<_>>(),
                MessageDialogButton::ORDER
            );
        }
    }

    #[test]
    fn abort_like_dialog_uses_fixed_width_restart_order_and_native_close_focus() {
        let fonts = endeavour_font_set();
        let mut dialog = MessageDialogState::new(
            "Abort round?",
            "Abort",
            MessageDialogButtons::YES_RESTART_NO,
            MessageDialogIcon::Standard(33),
            MessageDialogSize::Small,
            false,
        )
        .with_fixed_width(400)
        .with_centered_message();
        let layout = dialog.layout(1280, 720, &fonts.text);

        assert_eq!(dialog.size(), MessageDialogSize::Fixed(400));
        assert_eq!(layout.bounds.w, 400);
        assert!(layout.close_button.is_some());
        assert_eq!(
            layout
                .buttons
                .iter()
                .map(|button| button.button)
                .collect::<Vec<_>>(),
            [
                MessageDialogButton::Yes,
                MessageDialogButton::Restart,
                MessageDialogButton::No,
            ]
        );
        assert_eq!(layout.buttons[0].rect.x - layout.bounds.x, 10);
        assert_eq!(layout.buttons[1].rect.x - layout.bounds.x, 140);
        assert_eq!(layout.buttons[2].rect.x - layout.bounds.x, 270);
        assert_eq!(dialog.focused_button(), Some(MessageDialogButton::Yes));

        dialog.handle_key_down(KeyCode::Tab, false);
        assert_eq!(dialog.focused_button(), Some(MessageDialogButton::Restart));
        assert!(!dialog.close_focused());
        assert_eq!(dialog.handle_key_down(KeyCode::Enter, false), None);
        assert_eq!(
            dialog.handle_key_up(KeyCode::Enter),
            Some(MessageDialogResult::Restart)
        );
        assert!(MessageDialogResult::Restart.is_positive());

        dialog.handle_key_down(KeyCode::Tab, false);
        assert_eq!(dialog.focused_button(), Some(MessageDialogButton::No));
        dialog.handle_key_down(KeyCode::Tab, false);
        assert!(dialog.close_focused());
        dialog.handle_key_down(KeyCode::Tab, false);
        assert_eq!(dialog.focused_button(), Some(MessageDialogButton::Yes));
        dialog.handle_key_down(KeyCode::Tab, true);
        assert!(dialog.close_focused());
    }

    #[test]
    fn dont_show_again_geometry_focus_and_input_match_cpp() {
        let fonts = endeavour_font_set();
        let mut dialog = MessageDialogState::new(
            "message",
            "caption",
            MessageDialogButtons::YES_NO,
            MessageDialogIcon::CONFIRM,
            MessageDialogSize::Regular,
            false,
        )
        .with_us_dont_show_again(false);
        let layout = dialog.layout(1280, 720, &fonts.text);
        let checkbox = layout.checkbox.as_ref().expect("checkbox layout");
        let (label_width, label_height) = fonts
            .text
            .measure("&Don't display this message in the future.", true);
        assert_eq!(checkbox.bounds.h, label_height);
        assert_eq!(checkbox.bounds.w, label_width + label_height + 4);
        assert_eq!(checkbox.square.w, label_height);
        assert_eq!(checkbox.label_x, checkbox.bounds.x + label_height + 4);
        assert_eq!(
            checkbox.bounds.x,
            layout.message.x + (layout.message.w - checkbox.bounds.w) / 2
        );
        assert_eq!(checkbox.bounds.y, layout.message.y + layout.message.h + 20);
        assert_eq!(
            layout.buttons[0].rect.y,
            layout.message.y + layout.message.h + label_height + 44
        );

        assert_eq!(dialog.focused_button(), Some(MessageDialogButton::Yes));
        dialog.handle_key_down(KeyCode::Tab, true);
        assert!(dialog.checkbox_focused());
        assert_eq!(dialog.handle_key_down(KeyCode::Space, false), None);
        assert_eq!(dialog.checkbox_checked(), Some(true));
        assert_eq!(dialog.take_checkbox_changes(), vec![true]);
        assert_eq!(
            dialog.take_sound_events(),
            vec![MessageDialogSound::ArrowHit]
        );
        assert_eq!(dialog.handle_gamepad_low_down(), None);
        assert_eq!(dialog.handle_gamepad_low_up(), None);
        assert_eq!(dialog.checkbox_checked(), Some(false));
        assert_eq!(dialog.take_checkbox_changes(), vec![false]);
        assert_eq!(
            dialog.take_sound_events(),
            vec![MessageDialogSound::ArrowHit]
        );
        assert_eq!(
            dialog.handle_key_down(KeyCode::Enter, false),
            Some(MessageDialogResult::Yes),
            "Return accepts the dialog rather than toggling the checkbox"
        );

        assert_eq!(dialog.handle_hotkey('D'), None);
        assert_eq!(dialog.checkbox_checked(), Some(true));
        assert_eq!(dialog.take_checkbox_changes(), vec![true]);
        assert_eq!(
            dialog.take_sound_events(),
            vec![MessageDialogSound::ArrowHit]
        );

        dialog.handle_key_down(KeyCode::Tab, false);
        assert_eq!(dialog.focused_button(), Some(MessageDialogButton::Yes));

        dialog.handle_pointer_move(
            GuiPoint::new(
                (checkbox.square.x + checkbox.square.w) as f32,
                checkbox.square.y as f32,
            ),
            &layout,
        );
        assert_eq!(dialog.handle_pointer_up(&layout), None);
        assert_eq!(dialog.checkbox_checked(), Some(false));
        assert_eq!(dialog.take_checkbox_changes(), vec![false]);
        assert_eq!(
            dialog.take_sound_events(),
            vec![MessageDialogSound::ArrowHit]
        );
        assert_eq!(
            dialog.focused_button(),
            Some(MessageDialogButton::Yes),
            "checkbox clicks do not steal focus in C4GUI"
        );
        assert_eq!(dialog.handle_key_down(KeyCode::Space, false), None);
        assert_eq!(dialog.checkbox_checked(), Some(false));
        assert_eq!(
            dialog.handle_key_up(KeyCode::Space),
            Some(MessageDialogResult::Yes)
        );
    }

    #[test]
    fn default_focus_and_keyboard_results_match_cpp() {
        let mut yes = MessageDialogState::new(
            "message",
            "caption",
            MessageDialogButtons::YES_NO,
            MessageDialogIcon::CONFIRM,
            MessageDialogSize::Regular,
            false,
        );
        assert_eq!(yes.focused_button(), Some(MessageDialogButton::Yes));
        assert_eq!(yes.handle_key_down(KeyCode::Enter, false), None);
        assert_eq!(yes.take_sound_events(), vec![MessageDialogSound::ArrowHit]);
        assert_eq!(
            yes.handle_key_up(KeyCode::Enter),
            Some(MessageDialogResult::Yes)
        );
        assert_eq!(yes.take_sound_events(), vec![MessageDialogSound::Click]);

        let mut no = MessageDialogState::new(
            "message",
            "caption",
            MessageDialogButtons::YES_NO,
            MessageDialogIcon::CONFIRM,
            MessageDialogSize::Regular,
            true,
        );
        assert_eq!(no.focused_button(), Some(MessageDialogButton::No));
        assert_eq!(no.handle_key_down(KeyCode::Space, false), None);
        assert_eq!(no.take_sound_events(), vec![MessageDialogSound::ArrowHit]);
        assert_eq!(
            no.handle_key_up(KeyCode::Space),
            Some(MessageDialogResult::No)
        );
        assert_eq!(no.take_sound_events(), vec![MessageDialogSound::Click]);
        assert_eq!(
            no.handle_key_down(KeyCode::Escape, false),
            Some(MessageDialogResult::Dismissed)
        );
        assert_eq!(yes.handle_hotkey('y'), Some(MessageDialogResult::Yes));
        assert_eq!(yes.handle_hotkey('N'), Some(MessageDialogResult::No));
        assert_eq!(yes.handle_hotkey('X'), None);
    }

    #[test]
    fn button_labels_accept_per_dialog_language_resources() {
        let mut dialog = MessageDialogState::new(
            "message",
            "caption",
            MessageDialogButtons::OK_CANCEL,
            MessageDialogIcon::NOTIFY,
            MessageDialogSize::Regular,
            false,
        );
        dialog.set_button_label(MessageDialogButton::Ok, "&OK");
        dialog.set_button_label(MessageDialogButton::Cancel, "&Abbrechen");

        assert_eq!(dialog.button_label(MessageDialogButton::Ok), "&OK");
        assert_eq!(
            dialog.button_label(MessageDialogButton::Cancel),
            "&Abbrechen"
        );
        assert_eq!(dialog.handle_hotkey('A'), Some(MessageDialogResult::Cancel));
        assert_eq!(dialog.handle_hotkey('C'), None);
        assert_eq!(
            dialog.button_label(MessageDialogButton::Retry),
            MessageDialogButton::Retry.label()
        );
    }

    #[test]
    fn pointer_capture_uses_half_open_hits_and_restores_on_reentry() {
        let fonts = endeavour_font_set();
        let mut dialog = ok_dialog("message");
        let layout = dialog.layout(1280, 720, &fonts.text);
        let button = layout.buttons[0].rect;
        dialog.handle_pointer_move(
            GuiPoint::new((button.x + 1) as f32, (button.y + 1) as f32),
            &layout,
        );
        dialog.handle_pointer_down(&layout);
        dialog.handle_pointer_move(
            GuiPoint::new((button.x + button.w) as f32, button.y as f32),
            &layout,
        );
        assert_eq!(dialog.handle_pointer_up(&layout), None);

        dialog.handle_pointer_move(
            GuiPoint::new((button.x + 1) as f32, (button.y + 1) as f32),
            &layout,
        );
        dialog.handle_pointer_down(&layout);
        dialog.handle_pointer_move(GuiPoint::new(0.0, 0.0), &layout);
        assert_eq!(
            dialog.handle_pointer_up_at(
                GuiPoint::new((button.x + 1) as f32, (button.y + 1) as f32),
                &layout,
            ),
            None,
            "release hit-testing does not synthesize a re-entry move"
        );

        dialog.cancel_pointer_capture();
        dialog.handle_pointer_move(GuiPoint::new(0.0, 0.0), &layout);
        dialog.handle_pointer_down_at(
            GuiPoint::new((button.x + 1) as f32, (button.y + 1) as f32),
            &layout,
        );
        assert!(dialog.has_pointer_capture());
        assert_eq!(
            dialog.handle_pointer_up_at(
                GuiPoint::new((button.x + 1) as f32, (button.y + 1) as f32),
                &layout,
            ),
            Some(MessageDialogResult::Ok),
            "button-down uses the current event coordinate, not stale hover state"
        );

        dialog.handle_pointer_move(
            GuiPoint::new((button.x + 1) as f32, (button.y + 1) as f32),
            &layout,
        );
        dialog.handle_pointer_down(&layout);
        dialog.handle_pointer_move(GuiPoint::new(0.0, 0.0), &layout);
        dialog.handle_pointer_move(
            GuiPoint::new((button.x + 2) as f32, (button.y + 2) as f32),
            &layout,
        );
        assert_eq!(
            dialog.handle_pointer_up(&layout),
            Some(MessageDialogResult::Ok)
        );
    }

    #[test]
    fn title_drag_applies_live_and_final_delta_outside_dialog() {
        let fonts = endeavour_font_set();
        let mut dialog = ok_dialog("message");
        let layout = dialog.layout(1280, 720, &fonts.text);
        let caption = layout.caption.expect("caption");
        let start = GuiPoint::new((caption.x + 10) as f32, (caption.y + 10) as f32);

        dialog.handle_pointer_down_at(start, &layout);
        assert!(dialog.has_pointer_capture());
        assert!(dialog.has_positional_pointer_drag());

        let moved = GuiPoint::new(start.x - 500.0, start.y + 37.0);
        dialog.handle_pointer_move(moved, &layout);
        assert_eq!(dialog.dialog_offset(), (-500, 37));
        let moved_layout = dialog.layout(1280, 720, &fonts.text);
        assert_eq!(moved_layout.bounds.x, layout.bounds.x - 500);
        assert_eq!(moved_layout.bounds.y, layout.bounds.y + 37);

        let released = GuiPoint::new(moved.x - 9.0, moved.y + 4.0);
        assert_eq!(dialog.handle_pointer_up_at(released, &moved_layout), None);
        assert_eq!(dialog.dialog_offset(), (-509, 41));
        assert!(!dialog.has_pointer_capture());

        let retained = dialog.dialog_offset();
        let released_layout = dialog.layout(1280, 720, &fonts.text);
        dialog.handle_pointer_move(GuiPoint::new(1000.0, 700.0), &released_layout);
        assert_eq!(dialog.dialog_offset(), retained);

        let mut close = ok_dialog("message");
        let close_layout = close.layout(1280, 720, &fonts.text);
        let close_point = GuiPoint::new(
            (close_layout.close_button.expect("close").x + 1) as f32,
            (close_layout.close_button.expect("close").y + 1) as f32,
        );
        close.handle_pointer_down_at(close_point, &close_layout);
        assert!(close.has_pointer_capture());
        assert!(!close.has_positional_pointer_drag());
    }

    #[test]
    fn title_autoscroll_advances_per_draw_and_dwells_at_both_ends() {
        let font = unit_width_font("W");
        let caption = "W".repeat(278);
        let dialog = MessageDialogState::new(
            "message",
            caption,
            MessageDialogButtons::OK,
            MessageDialogIcon::None,
            MessageDialogSize::Small,
            false,
        );
        assert_eq!(font.measure(dialog.caption(), true).0 + 25 - SMALL_WIDTH, 3);
        let base = Instant::now();
        assert_eq!(dialog.caption_scroll_offset_at(base, &font), 0);
        assert_eq!(
            dialog.caption_scroll_offset_at(
                base + TITLE_SCROLL_DELAY - Duration::from_millis(1),
                &font,
            ),
            0
        );
        let outbound = base + TITLE_SCROLL_DELAY;
        assert_eq!(dialog.caption_scroll_offset_at(outbound, &font), 1);
        assert_eq!(dialog.caption_scroll_offset_at(outbound, &font), 2);
        assert_eq!(
            dialog.caption_scroll_offset_at(outbound, &font),
            2,
            "the attempted max-scroll frame reverses and immediately backs off"
        );
        assert_eq!(
            dialog.caption_scroll_offset_at(
                outbound + TITLE_SCROLL_DELAY - Duration::from_millis(1),
                &font,
            ),
            2
        );
        let returning = outbound + TITLE_SCROLL_DELAY;
        assert_eq!(dialog.caption_scroll_offset_at(returning, &font), 1);
        assert_eq!(dialog.caption_scroll_offset_at(returning, &font), 0);
        assert_eq!(
            dialog.caption_scroll_offset_at(returning, &font),
            0,
            "the attempted negative frame reverses and pauses at the start"
        );
        assert_eq!(
            dialog.caption_scroll_offset_at(returning + TITLE_SCROLL_DELAY, &font),
            1
        );

        for (width, expected) in [(276, 0), (275, 0)] {
            let short = MessageDialogState::new(
                "message",
                "W".repeat(width),
                MessageDialogButtons::OK,
                MessageDialogIcon::None,
                MessageDialogSize::Small,
                false,
            );
            assert_eq!(short.caption_scroll_offset_at(base, &font), 0);
            assert_eq!(
                short.caption_scroll_offset_at(base + TITLE_SCROLL_DELAY, &font),
                expected,
                "max scroll of one or zero has no visible offset"
            );
        }
    }

    #[test]
    fn title_tooltip_uses_shared_mouse_delay_and_close_wins_overlap() {
        use crate::context_menu::{ClassicTooltipTracker, CLASSIC_TOOLTIP_DELAY};

        let fonts = endeavour_font_set();
        let mut dialog = ok_dialog("message");
        dialog.set_close_tooltip("Schließen");
        let layout = dialog.layout(1280, 720, &fonts.text);
        let caption = layout.caption.expect("caption");
        let title_point = GuiPoint::new((caption.x + 10) as f32, (caption.y + 10) as f32);
        let base = Instant::now();
        let mut tracker = ClassicTooltipTracker::new_at(base);
        tracker.note_pointer_move_at(title_point, base);
        dialog.handle_pointer_move(title_point, &layout);

        assert!(dialog
            .tooltip_state(
                tracker
                    .eligible_pointer_at(base + CLASSIC_TOOLTIP_DELAY - Duration::from_millis(1)),
                &layout,
            )
            .is_none());
        assert_eq!(
            dialog
                .tooltip_state(
                    tracker.eligible_pointer_at(base + CLASSIC_TOOLTIP_DELAY),
                    &layout,
                )
                .expect("title tooltip")
                .text,
            dialog.caption()
        );

        tracker.note_non_pointer_input();
        assert!(!tracker.note_pointer_move_at(
            GuiPoint::new(title_point.x + 0.25, title_point.y + 0.25),
            base + Duration::from_secs(1),
        ));
        assert!(dialog
            .tooltip_state(
                tracker.eligible_pointer_at(base + Duration::from_secs(2)),
                &layout,
            )
            .is_none());

        let close = layout.close_button.expect("close");
        let close_point = GuiPoint::new((close.x + 1) as f32, (close.y + 1) as f32);
        let close_at = base + Duration::from_secs(3);
        tracker.note_pointer_move_at(close_point, close_at);
        dialog.handle_pointer_move(close_point, &layout);
        assert_eq!(
            dialog
                .tooltip_state(
                    tracker.eligible_pointer_at(close_at + CLASSIC_TOOLTIP_DELAY),
                    &layout,
                )
                .expect("localized close tooltip wins the title overlap")
                .text,
            "Schließen"
        );
    }

    #[test]
    fn rendering_overlays_without_dimming_the_screen() {
        let fonts = endeavour_font_set();
        let caption = load_graphics_png("GUICaption.png");
        let button = load_graphics_png("GUIButton.png");
        let button_down = load_graphics_png("GUIButtonDown.png");
        let highlight = blacken_transparent_pixels(&load_graphics_png("GUIButtonHighlight.png"));
        let icons = load_graphics_png("GUIIcons.png");
        let icons_extended = load_graphics_png("GUIIcons2.png");
        let checkbox = load_graphics_png("GUICheckbox.png");
        let progress = load_graphics_png("GUIProgress.png");
        let mut surface = Surface::new(800, 600, PixelFormat::Rgba8888);
        surface.fill(Color::opaque(17, 29, 43));
        let before = surface.get_pixel(0, 0).expect("corner");
        let resources = MessageDialogResources {
            skin: ClassicGuiSkin::new(&caption, &button, &button_down, Some(&highlight)),
            fonts: &fonts,
            tooltip_font: &fonts.text,
            icons: &icons,
            icons_extended: &icons_extended,
            button_highlight: &highlight,
            checkbox: &checkbox,
            progress: &progress,
        };
        let mut dialog = ok_dialog("message");
        dialog
            .render(&mut surface, resources, true, true, Some(standard_gamma()))
            .expect("render valid classic resources");
        assert_eq!(surface.get_pixel(0, 0).expect("corner"), before);
        assert_ne!(surface.get_pixel(400, 300).expect("dialog center"), before);

        let mut inactive = Surface::new(800, 600, PixelFormat::Rgba8888);
        inactive.fill(Color::opaque(17, 29, 43));
        dialog
            .render(
                &mut inactive,
                resources,
                false,
                false,
                Some(standard_gamma()),
            )
            .expect("render inactive stacked dialog");
        assert_ne!(
            surface.pixels(),
            inactive.pixels(),
            "only the active stack entry draws its focus highlight"
        );

        let layout = dialog.layout(800, 600, &fonts.text);
        let button = layout.buttons.first().expect("OK layout").rect;
        dialog.handle_pointer_move(
            GuiPoint::new((button.x + 2) as f32, (button.y + 2) as f32),
            &layout,
        );
        let mut shared_hover = Surface::new(800, 600, PixelFormat::Rgba8888);
        shared_hover.fill(Color::opaque(17, 29, 43));
        dialog
            .render(&mut shared_hover, resources, false, true, None)
            .expect("render mouse-active shared dialog");
        assert_ne!(
            shared_hover.pixels(),
            inactive.pixels(),
            "shared dialogs retain mouse hover without keyboard focus"
        );

        dialog.handle_pointer_down(&layout);
        let mut pressed_without_focus = Surface::new(800, 600, PixelFormat::Rgba8888);
        pressed_without_focus.fill(Color::opaque(17, 29, 43));
        dialog
            .render(&mut pressed_without_focus, resources, false, false, None)
            .expect("render pressed inactive dialog");
        assert_ne!(
            pressed_without_focus.pixels(),
            inactive.pixels(),
            "button down frame is independent of keyboard and mouse activity"
        );
    }

    #[test]
    fn automatic_wrap_preserves_cpp_first_character_quirk() {
        let fonts = endeavour_font_set();
        let (single_width, _) = fonts.text.measure("W", true);
        assert_eq!(break_message(&fonts.text, "WWW", single_width), "W\nWW");
    }

    #[test]
    fn break_message_uses_cpp_skip_mode_markup_validation() {
        let font = unit_width_font("AB<>/focz ");
        let max_width = font.measure("A", true).0;

        for (message, expected) in [("</foo>AB", "</foo>A\nB"), ("<c zz>AB", "<c zz>A\nB")] {
            assert_eq!(break_message(&font, message, max_width), expected);
        }
    }

    #[test]
    fn break_message_rewrites_a_zero_width_tab_break_candidate() {
        let font = unit_width_font("AB");
        assert_eq!(break_message(&font, "AAAA\tBBBB", 4), "AAAA\nBBBB");
    }

    #[test]
    fn break_message_uses_the_drawn_missing_glyph_width() {
        let mut font = ClonkFont::new(3);
        font.add_glyph(
            'A',
            clonk_graphics::clonk_font::GlyphCell {
                width: 5,
                pixels: vec![Color::opaque(255, 255, 255); 5 * 4],
            },
        );
        font.set_missing_glyph(clonk_graphics::clonk_font::GlyphCell {
            width: 5,
            pixels: vec![Color::opaque(255, 255, 255); 5 * 4],
        });

        let broken = break_message(&font, "A☃", 5);
        assert_eq!(broken, "A\n☃");
        assert!(
            broken.lines().all(|line| font.measure(line, true).0 <= 5),
            "every broken line fits when measured with the rendered fallback"
        );
    }

    #[test]
    fn break_message_matches_cpp_output_contract() {
        let mut font = ClonkFont::new(3);
        font.h_space = 0;
        font.add_glyph(
            'W',
            clonk_graphics::clonk_font::GlyphCell {
                width: 1,
                pixels: vec![Color::opaque(255, 255, 255); 4],
            },
        );

        assert_eq!(break_message(&font, "WWWW", 1), "W\nWW\nW");
        assert_eq!(
            break_message_with_options(
                &font,
                "WWWW",
                1,
                BreakMessageOptions {
                    max_lines: 1,
                    ..BreakMessageOptions::default()
                },
            ),
            "W\nWWW",
            "the suffix after the first break stays untouched"
        );
        assert_eq!(
            break_message(&font, "<c ff0000>WWW</c>", 1),
            "<c ff0000>W</c>\n<c ff0000>WW</c>"
        );
        assert_eq!(
            break_message(&font, "W<i>W</i>", 1),
            "W\n<i>W</i>",
            "markup skipped immediately before overflow moves to the continuation"
        );
        assert_eq!(
            break_message(&font, "<c RED>WW", 1),
            "<c RED>W\nW",
            "skip-mode markup can be widthless without entering the strict reopen stack"
        );
        assert_eq!(
            break_message_with_options(
                &font,
                "W|WWWW",
                1,
                BreakMessageOptions {
                    max_lines: 1,
                    ..BreakMessageOptions::default()
                },
            ),
            "W|WWWW",
            "a manual break also exhausts max_lines before the untouched suffix"
        );
        assert_eq!(break_message(&font, "left|right", i32::MAX), "left|right");
        assert_eq!(break_message(&font, "left\nright", i32::MAX), "left\nright");
    }

    #[test]
    fn break_message_zoom_scales_glyphs_but_not_spacing() {
        let mut font = ClonkFont::new(3);
        font.h_space = 0;
        font.add_glyph(
            'W',
            clonk_graphics::clonk_font::GlyphCell {
                width: 2,
                pixels: vec![Color::opaque(255, 255, 255); 8],
            },
        );

        assert_eq!(break_message(&font, "WW", 2), "W\nW");
        assert_eq!(
            break_message_with_options(
                &font,
                "WW",
                2,
                BreakMessageOptions {
                    zoom: 0.5,
                    ..BreakMessageOptions::default()
                },
            ),
            "WW"
        );
        assert_eq!(
            break_message_with_options(
                &font,
                "WW",
                0,
                BreakMessageOptions {
                    zoom: 0.5,
                    ..BreakMessageOptions::default()
                },
            ),
            "W\nW",
            "BreakMessage does not clamp a zero-width output area"
        );
    }

    #[test]
    fn hud_break_message_matches_clonk_adapter() {
        let fonts = endeavour_font_set();
        let hud = HudFont::Clonk(&fonts.text);
        let (single_width, _) = fonts.text.measure("W", true);
        let dash_width = "AAAA-"
            .chars()
            .map(|character| hud.character_advance(character))
            .sum();
        for (text, width) in [
            ("AAA   BBB", i32::MAX),
            ("AAAA-BBBB", dash_width),
            ("WWW", single_width),
            ("<i>AAA BBB</i>", single_width * 3),
        ] {
            assert_eq!(
                break_hud_message(&hud, text, width),
                break_message(&fonts.text, text, width),
                "{text:?} at width {width}"
            );
        }
        assert_eq!(break_hud_message(&hud, "AAA   BBB", i32::MAX), "AAA   BBB");
        assert_eq!(
            break_hud_message(&hud, "AAAA-BBBB", dash_width),
            "AAAA-\nBBBB"
        );
        assert_eq!(break_hud_message(&hud, "WWW", single_width), "W\nWW");
    }

    #[test]
    fn break_message_counts_inline_image_as_one_aspect_scaled_token() {
        let mut font = ClonkFont::new(3);
        font.add_glyph(
            'A',
            clonk_graphics::clonk_font::GlyphCell {
                width: 5,
                pixels: vec![Color::opaque(255, 255, 255); 5 * 4],
            },
        );
        font.add_glyph(
            'B',
            clonk_graphics::clonk_font::GlyphCell {
                width: 4,
                pixels: vec![Color::opaque(255, 255, 255); 4 * 4],
            },
        );
        let images = TestFontImages {
            image: ImageData::new(2, 1, vec![255; 2 * 4]),
        };

        assert_eq!(
            break_message_with_images(&font, "A{{FLAM}}B", 12, &images),
            "A{{FLAM}}\nB"
        );
        assert_eq!(
            break_message_with_images(&font, "A{{MISS}}B", 7, &images),
            "A{{MISS}}B",
            "an unresolved image is a zero-width atomic character"
        );
    }

    #[test]
    fn malformed_or_out_of_range_resources_fail_loudly() {
        let fonts = endeavour_font_set();
        let caption = load_graphics_png("GUICaption.png");
        let button = load_graphics_png("GUIButton.png");
        let button_down = load_graphics_png("GUIButtonDown.png");
        let highlight = blacken_transparent_pixels(&load_graphics_png("GUIButtonHighlight.png"));
        let icons_extended = load_graphics_png("GUIIcons2.png");
        let checkbox = load_graphics_png("GUICheckbox.png");
        let progress = load_graphics_png("GUIProgress.png");
        let malformed_icons = ImageData::new(39, 40, vec![0; 39 * 40 * 4]);
        let resources = MessageDialogResources {
            skin: ClassicGuiSkin::new(&caption, &button, &button_down, Some(&highlight)),
            fonts: &fonts,
            tooltip_font: &fonts.text,
            icons: &malformed_icons,
            icons_extended: &icons_extended,
            button_highlight: &highlight,
            checkbox: &checkbox,
            progress: &progress,
        };
        let mut surface = Surface::new(800, 600, PixelFormat::Rgba8888);
        let error = ok_dialog("message")
            .render(&mut surface, resources, true, true, None)
            .expect_err("undersized icon sheet must fail");
        assert!(error.to_string().contains("GUIIcons.png"));

        let icons = load_graphics_png("GUIIcons.png");
        let malformed_checkbox = ImageData::new(31, 32, vec![0; 31 * 32 * 4]);
        let resources = MessageDialogResources {
            skin: ClassicGuiSkin::new(&caption, &button, &button_down, Some(&highlight)),
            fonts: &fonts,
            tooltip_font: &fonts.text,
            icons: &icons,
            icons_extended: &icons_extended,
            button_highlight: &highlight,
            checkbox: &malformed_checkbox,
            progress: &progress,
        };
        let error = ok_dialog("message")
            .render(&mut surface, resources, true, true, None)
            .expect_err("malformed checkbox sheet must fail");
        assert!(error.to_string().contains("GUICheckbox.png"));

        let malformed_progress = ImageData::new(2, 1, vec![0; 2 * 4]);
        let resources = MessageDialogResources {
            skin: ClassicGuiSkin::new(&caption, &button, &button_down, Some(&highlight)),
            fonts: &fonts,
            tooltip_font: &fonts.text,
            icons: &icons,
            icons_extended: &icons_extended,
            button_highlight: &highlight,
            checkbox: &checkbox,
            progress: &malformed_progress,
        };
        let error = ok_dialog("message")
            .with_progress(50)
            .render(&mut surface, resources, true, true, None)
            .expect_err("malformed progress strip must fail");
        assert!(error.to_string().contains("GUIProgress.png"));

        let resources = MessageDialogResources {
            skin: ClassicGuiSkin::new(&caption, &button, &button_down, Some(&highlight)),
            fonts: &fonts,
            tooltip_font: &fonts.text,
            icons: &icons,
            icons_extended: &icons_extended,
            button_highlight: &highlight,
            checkbox: &checkbox,
            progress: &progress,
        };
        let dialog = MessageDialogState::regular_ok(
            "message",
            "caption",
            MessageDialogIcon::Standard(u16::MAX),
        );
        let error = dialog
            .render(&mut surface, resources, true, true, None)
            .expect_err("out-of-range phase must fail");
        assert!(error.to_string().contains("phase 65535"));
    }
}
