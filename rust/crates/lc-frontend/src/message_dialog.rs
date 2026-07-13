//! Pixel-faithful reusable `C4GUI::MessageDialog` presentation and input.
//!
//! The C++ modal wrapper only runs a nested message loop. The visible object
//! is an ordinary classic dialog, so this renderer deliberately leaves the
//! underlying screen untouched and draws no dimming layer.

use crate::classic_gui::{draw_facet_stretch, ClassicButtonState, ClassicGuiSkin, IntRect};
use crate::{expand_hotkey_markup, ClonkFontSet, GuiPoint, ImageData, KeyCode};
use anyhow::Result;
use lc_graphics::clonk_font::{ClonkFont, TextAlign};
use lc_graphics::{GammaRamp, Surface};
use lc_gui::Rect as GuiRect;
use std::ops::{BitOr, BitOrAssign};

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
const CLOSE_ICON_PHASE: u16 = 34;

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
    pub const OK_CANCEL: Self = Self(Self::OK.0 | Self::CANCEL.0);
    pub const YES_NO: Self = Self(Self::YES.0 | Self::NO.0);
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
    No,
}

impl MessageDialogButton {
    const ORDER: [Self; 5] = [Self::Ok, Self::Retry, Self::Cancel, Self::Yes, Self::No];

    const fn mask(self) -> u8 {
        match self {
            Self::Ok => MessageDialogButtons::OK.0,
            Self::Retry => MessageDialogButtons::RETRY.0,
            Self::Cancel => MessageDialogButtons::CANCEL.0,
            Self::Yes => MessageDialogButtons::YES.0,
            Self::No => MessageDialogButtons::NO.0,
        }
    }

    pub const fn label(self) -> &'static str {
        match self {
            Self::Ok => "&OK",
            Self::Retry => "Retry",
            Self::Cancel => "Cancel",
            Self::Yes => "&Yes",
            Self::No => "&No",
        }
    }

    pub const fn result(self) -> MessageDialogResult {
        match self {
            Self::Ok => MessageDialogResult::Ok,
            Self::Retry => MessageDialogResult::Retry,
            Self::Cancel => MessageDialogResult::Cancel,
            Self::Yes => MessageDialogResult::Yes,
            Self::No => MessageDialogResult::No,
        }
    }

    pub const fn hotkey(self) -> Option<char> {
        match self {
            Self::Ok => Some('O'),
            Self::Yes => Some('Y'),
            Self::No => Some('N'),
            Self::Retry | Self::Cancel => None,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MessageDialogResult {
    Ok,
    Retry,
    Cancel,
    Yes,
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
        matches!(self, Self::Ok | Self::Retry | Self::Yes)
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum MessageDialogSize {
    #[default]
    Regular,
    Medium,
    Small,
}

impl MessageDialogSize {
    pub const fn width(self) -> i32 {
        match self {
            Self::Regular => REGULAR_WIDTH,
            Self::Medium => MEDIUM_WIDTH,
            Self::Small => SMALL_WIDTH,
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
    pub const ERROR: Self = Self::Standard(11);
    pub const CONFIRM: Self = Self::Standard(18);
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MessageDialogPlacement {
    Centered,
    /// Non-exclusive C++ screens place ordinary dialogs at the preferred
    /// viewport origin plus `(30,30)`.
    Preferred {
        x: i32,
        y: i32,
    },
}

impl Default for MessageDialogPlacement {
    fn default() -> Self {
        Self::Centered
    }
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
    pub buttons: Vec<MessageDialogButtonLayout>,
}

/// Borrowed classic resources. Callers should decline to render rather than
/// construct this value with substitute assets.
#[derive(Clone, Copy)]
pub struct MessageDialogResources<'a> {
    pub skin: ClassicGuiSkin<'a>,
    pub fonts: &'a ClonkFontSet,
    pub icons: &'a ImageData,
    pub icons_extended: &'a ImageData,
    pub button_highlight: &'a ImageData,
    pub checkbox: &'a ImageData,
}

impl MessageDialogResources<'_> {
    pub fn validate(self) -> Result<()> {
        self.skin.validate_message_dialog_assets()?;
        validate_icon_sheet("GUIIcons.png", self.icons, 40)?;
        validate_icon_sheet("GUIIcons2.png", self.icons_extended, 64)?;
        validate_nonempty_image("GUIButtonHighlight.png", self.button_highlight)?;
        validate_checkbox_sheet("GUICheckbox.png", self.checkbox)
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

/// Pure frontend state for one message dialog.
#[derive(Clone, Debug)]
pub struct MessageDialogState {
    caption: String,
    message: String,
    buttons: MessageDialogButtons,
    icon: MessageDialogIcon,
    size: MessageDialogSize,
    default_no: bool,
    checkbox: Option<MessageDialogCheckbox>,
    checkbox_changes: Vec<bool>,
    placement: MessageDialogPlacement,
    focus: Option<DialogTarget>,
    hovered: Option<DialogTarget>,
    pointer: Option<GuiPoint>,
    pointer_pressed: Option<DialogTarget>,
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
            message: message.into(),
            buttons,
            icon,
            size,
            default_no,
            checkbox: None,
            checkbox_changes: Vec::new(),
            placement: MessageDialogPlacement::Centered,
            focus: focus_button,
            hovered: None,
            pointer: None,
            pointer_pressed: None,
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

    pub fn with_us_dont_show_again(self, checked: bool) -> Self {
        self.with_checkbox("&Don't display this message in the future.", checked)
    }

    pub fn caption(&self) -> &str {
        &self.caption
    }

    pub fn message(&self) -> &str {
        &self.message
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
            .find(|button| button.hotkey() == Some(character))
            .map(MessageDialogButton::result)
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
        let centered = self.size != MessageDialogSize::Regular
            || (unbroken_width <= width - 140 && unbroken_height <= font.line_height);
        let message_width = if centered { width - 140 } else { width - 80 };
        let message_text = break_message(font, &self.message, message_width);
        let (_, message_height) = font.measure(&message_text, true);
        let checkbox_size = self.checkbox.as_ref().map(|checkbox| {
            let (label_width, label_height) = font.measure(&checkbox.raw_label, true);
            (label_width + label_height + 4, label_height)
        });
        let client_height =
            message_height + checkbox_size.map_or(CLIENT_VERTICAL_ROOM, |(_, height)| height + 100);
        let height = title_height + client_height;
        let (x, y) = match self.placement {
            MessageDialogPlacement::Centered => {
                ((screen_width - width) / 2, (screen_height - height) / 2)
            }
            MessageDialogPlacement::Preferred { x, y } => (x + 30, y + 30),
        };
        let client_y = y + title_height;
        let caption = (title_height > 0).then_some(IntRect {
            x,
            y,
            w: width,
            h: title_height,
        });
        let close_button = (title_height > 0).then_some(IntRect {
            x: x + width - 20,
            y: y + 4,
            w: 16,
            h: 16,
        });
        let icon = IntRect {
            x: x + DIALOG_INDENT,
            y: client_y + DIALOG_INDENT,
            w: ICON_SIZE,
            h: ICON_SIZE,
        };
        let message = IntRect {
            x: x + 70,
            y: client_y + 10,
            w: message_width,
            h: message_height,
        };
        let checkbox = checkbox_size.map(|(checkbox_width, checkbox_height)| {
            let bounds = IntRect {
                x: message.x + (message.w - checkbox_width) / 2,
                y: client_y + message_height + 30,
                w: checkbox_width,
                h: checkbox_height,
            };
            MessageDialogCheckboxLayout {
                bounds,
                square: IntRect {
                    w: checkbox_height,
                    ..bounds
                },
                label_x: bounds.x + checkbox_height + 4,
            }
        });
        let ordered = self.buttons.ordered();
        let count = i32::try_from(ordered.len()).unwrap_or(i32::MAX);
        let group_width = if count == 0 {
            0
        } else {
            count * BUTTON_WIDTH + (count - 1) * BUTTON_GAP
        };
        let button_y = client_y
            + message_height
            + checkbox_size.map_or(34, |(_, checkbox_height)| checkbox_height + 54);
        let first_button_x = x + (width - group_width) / 2;
        let buttons = ordered
            .into_iter()
            .enumerate()
            .map(|(index, button)| MessageDialogButtonLayout {
                button,
                rect: IntRect {
                    x: first_button_x
                        + i32::try_from(index).unwrap_or(i32::MAX) * (BUTTON_WIDTH + BUTTON_GAP),
                    y: button_y,
                    w: BUTTON_WIDTH,
                    h: BUTTON_HEIGHT,
                },
            })
            .collect();
        MessageDialogLayout {
            bounds: IntRect {
                x,
                y,
                w: width,
                h: height,
            },
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
        self.pointer = Some(point);
        self.hovered = hit_target(layout, point);
        if was_down != self.pointer_target_is_down() {
            self.sound_events.push(MessageDialogSound::ArrowHit);
        }
    }

    pub fn handle_pointer_down(&mut self, layout: &MessageDialogLayout) {
        let was_down = self.pointer_target_is_down();
        let target = self.pointer.and_then(|point| hit_target(layout, point));
        if target == Some(DialogTarget::Checkbox) {
            self.pointer_pressed = None;
            return;
        }
        self.pointer_pressed = target;
        if !was_down && self.pointer_pressed.is_some() {
            self.sound_events.push(MessageDialogSound::ArrowHit);
        }
    }

    pub fn handle_pointer_up(
        &mut self,
        layout: &MessageDialogLayout,
    ) -> Option<MessageDialogResult> {
        let released = self.pointer.and_then(|point| hit_target(layout, point));
        if released == Some(DialogTarget::Checkbox) {
            self.pointer_pressed = None;
            self.toggle_checkbox();
            return None;
        }
        let pressed = self.pointer_pressed.take()?;
        (released == Some(pressed)).then(|| {
            self.sound_events.push(MessageDialogSound::Click);
            self.target_result(pressed)
        })
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
        self.key_pressed = None;
        self.sound_events.clear();
    }

    pub fn render(
        &self,
        surface: &mut Surface,
        resources: MessageDialogResources<'_>,
        active: bool,
        gamma: Option<&GammaRamp>,
    ) -> Result<()> {
        resources.validate()?;
        let layout = self.layout(
            surface.width() as i32,
            surface.height() as i32,
            &resources.fonts.text,
        );
        resources.skin.draw_dialog(surface, layout.bounds, gamma);
        if let Some(caption) = layout.caption {
            resources.skin.draw_caption_with_right_indent(
                surface,
                caption,
                &self.caption,
                &resources.fonts.text,
                [255, 255, 255, 255],
                TextAlign::Left,
                20,
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
            if active
                && (self.focus == Some(DialogTarget::Checkbox)
                    || self.hovered == Some(DialogTarget::Checkbox))
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
                button.button.label(),
                resources.fonts,
                ClassicButtonState {
                    pressed: active && self.target_pressed(target),
                    highlighted: active
                        && (self.focus == Some(target) || self.hovered == Some(target)),
                },
                gamma,
            );
        }
        if let Some(close) = layout.close_button {
            let target = DialogTarget::Close;
            let highlighted =
                active && (self.focus == Some(target) || self.hovered == Some(target));
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
            if active && self.target_pressed(target) {
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
    find(MessageDialogButton::No)
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

#[derive(Clone, Debug)]
enum MessageToken {
    Text {
        raw: String,
        width: i32,
        break_kind: Option<bool>,
    },
    HardBreak,
}

impl MessageToken {
    fn width(&self) -> i32 {
        match self {
            Self::Text { width, .. } => *width,
            Self::HardBreak => 0,
        }
    }

    fn break_kind(&self) -> Option<bool> {
        match self {
            Self::Text { break_kind, .. } => *break_kind,
            Self::HardBreak => None,
        }
    }

    fn append_to(&self, output: &mut String) {
        if let Self::Text { raw, .. } = self {
            output.push_str(raw);
        }
    }
}

/// `CStdFont::BreakMessage`'s character-level line breaking for ordinary GUI
/// labels. Valid color/italic tags are widthless and their state naturally
/// persists across the inserted newline in `ClonkFont::draw_with_gamma`.
pub(crate) fn break_message(font: &ClonkFont, text: &str, max_width: i32) -> String {
    let max_width = max_width.max(1);
    let mut tokens = Vec::new();
    let mut rest = text;
    while !rest.is_empty() {
        if rest.starts_with('<') {
            if let Some(end) = rest.find('>') {
                let raw = &rest[..=end];
                let contents = &rest[1..end];
                if valid_message_markup(contents) {
                    tokens.push(MessageToken::Text {
                        raw: raw.to_string(),
                        width: 0,
                        break_kind: None,
                    });
                    rest = &rest[end + 1..];
                    continue;
                }
            }
        }
        let character = rest.chars().next().expect("non-empty message");
        rest = &rest[character.len_utf8()..];
        if character == '\n' || character == '|' {
            tokens.push(MessageToken::HardBreak);
            continue;
        }
        let width = if character >= ' ' {
            font.glyph(character)
                .map_or(0, |glyph| glyph.width)
                .saturating_add(font.h_space)
        } else {
            0
        };
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
        });
    }

    let mut lines: Vec<Vec<MessageToken>> = Vec::new();
    let mut line = Vec::new();
    let mut line_width = 0_i32;
    let mut last_break: Option<(usize, bool)> = None;
    let mut first_line_character = true;
    for token in tokens {
        if matches!(token, MessageToken::HardBreak) {
            lines.push(std::mem::take(&mut line));
            line_width = 0;
            last_break = None;
            first_line_character = true;
            continue;
        }
        let width = token.width();
        let token_break = token.break_kind();
        line.push(token);
        if width == 0 {
            continue;
        }
        line_width = line_width.saturating_add(width);
        let was_first_line_character = first_line_character;
        if line_width <= max_width || was_first_line_character {
            if let Some(include) = token_break {
                last_break = Some((line.len() - 1, include || was_first_line_character));
            }
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
            (line.len() - 1, 0)
        };
        let mut remainder = line.split_off(split_at);
        if skip > 0 && !remainder.is_empty() {
            remainder.remove(0);
        }
        lines.push(std::mem::take(&mut line));
        line = remainder;
        line_width = line
            .iter()
            .fold(0_i32, |sum, token| sum.saturating_add(token.width()));
        // `CStdFont::BreakMessage` deliberately resets both the normal break
        // candidate and its first-character flag after an automatic split,
        // even when already-scanned remainder text occupies the new line.
        // Consequently the next scanned character is admitted regardless of
        // width and old spaces in the remainder are not reused as candidates.
        last_break = None;
        first_line_character = true;
    }
    lines.push(line);
    let mut output = String::new();
    for (line_index, line) in lines.iter().enumerate() {
        if line_index > 0 {
            output.push('\n');
        }
        for token in line {
            token.append_to(&mut output);
        }
    }
    output
}

fn valid_message_markup(contents: &str) -> bool {
    matches!(contents, "i" | "/i" | "/c")
        || contents.strip_prefix("c ").is_some_and(|parameters| {
            let parameters = parameters.trim();
            !parameters.is_empty()
                && parameters.len() <= 8
                && parameters
                    .chars()
                    .all(|character| character.is_ascii_hexdigit())
        })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::classic_gui::blacken_transparent_pixels;
    use crate::test_support::{endeavour_font_set, load_graphics_png, standard_gamma};
    use lc_graphics::{Color, PixelFormat};

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
    fn all_sizes_and_button_order_match_cpp() {
        let fonts = endeavour_font_set();
        let buttons = MessageDialogButtons::OK
            | MessageDialogButtons::RETRY
            | MessageDialogButtons::CANCEL
            | MessageDialogButtons::YES
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
    fn rendering_overlays_without_dimming_the_screen() {
        let fonts = endeavour_font_set();
        let caption = load_graphics_png("GUICaption.png");
        let button = load_graphics_png("GUIButton.png");
        let button_down = load_graphics_png("GUIButtonDown.png");
        let highlight = blacken_transparent_pixels(&load_graphics_png("GUIButtonHighlight.png"));
        let icons = load_graphics_png("GUIIcons.png");
        let icons_extended = load_graphics_png("GUIIcons2.png");
        let checkbox = load_graphics_png("GUICheckbox.png");
        let mut surface = Surface::new(800, 600, PixelFormat::Rgba8888);
        surface.fill(Color::opaque(17, 29, 43));
        let before = surface.get_pixel(0, 0).expect("corner");
        let resources = MessageDialogResources {
            skin: ClassicGuiSkin::new(&caption, &button, &button_down, Some(&highlight)),
            fonts: &fonts,
            icons: &icons,
            icons_extended: &icons_extended,
            button_highlight: &highlight,
            checkbox: &checkbox,
        };
        let dialog = ok_dialog("message");
        dialog
            .render(&mut surface, resources, true, Some(standard_gamma()))
            .expect("render valid classic resources");
        assert_eq!(surface.get_pixel(0, 0).expect("corner"), before);
        assert_ne!(surface.get_pixel(400, 300).expect("dialog center"), before);

        let mut inactive = Surface::new(800, 600, PixelFormat::Rgba8888);
        inactive.fill(Color::opaque(17, 29, 43));
        dialog
            .render(&mut inactive, resources, false, Some(standard_gamma()))
            .expect("render inactive stacked dialog");
        assert_ne!(
            surface.pixels(),
            inactive.pixels(),
            "only the active stack entry draws its focus highlight"
        );
    }

    #[test]
    fn automatic_wrap_preserves_cpp_first_character_quirk() {
        let fonts = endeavour_font_set();
        let (single_width, _) = fonts.text.measure("W", true);
        assert_eq!(break_message(&fonts.text, "WWW", single_width), "W\nWW");
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
        let malformed_icons = ImageData::new(39, 40, vec![0; 39 * 40 * 4]);
        let resources = MessageDialogResources {
            skin: ClassicGuiSkin::new(&caption, &button, &button_down, Some(&highlight)),
            fonts: &fonts,
            icons: &malformed_icons,
            icons_extended: &icons_extended,
            button_highlight: &highlight,
            checkbox: &checkbox,
        };
        let mut surface = Surface::new(800, 600, PixelFormat::Rgba8888);
        let error = ok_dialog("message")
            .render(&mut surface, resources, true, None)
            .expect_err("undersized icon sheet must fail");
        assert!(error.to_string().contains("GUIIcons.png"));

        let icons = load_graphics_png("GUIIcons.png");
        let malformed_checkbox = ImageData::new(31, 32, vec![0; 31 * 32 * 4]);
        let resources = MessageDialogResources {
            skin: ClassicGuiSkin::new(&caption, &button, &button_down, Some(&highlight)),
            fonts: &fonts,
            icons: &icons,
            icons_extended: &icons_extended,
            button_highlight: &highlight,
            checkbox: &malformed_checkbox,
        };
        let error = ok_dialog("message")
            .render(&mut surface, resources, true, None)
            .expect_err("malformed checkbox sheet must fail");
        assert!(error.to_string().contains("GUICheckbox.png"));

        let resources = MessageDialogResources {
            skin: ClassicGuiSkin::new(&caption, &button, &button_down, Some(&highlight)),
            fonts: &fonts,
            icons: &icons,
            icons_extended: &icons_extended,
            button_highlight: &highlight,
            checkbox: &checkbox,
        };
        let dialog = MessageDialogState::regular_ok(
            "message",
            "caption",
            MessageDialogIcon::Standard(u16::MAX),
        );
        let error = dialog
            .render(&mut surface, resources, true, None)
            .expect_err("out-of-range phase must fail");
        assert!(error.to_string().contains("phase 65535"));
    }
}
