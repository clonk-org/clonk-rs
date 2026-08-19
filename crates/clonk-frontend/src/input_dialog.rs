//! Pixel-faithful reusable frontend state for `C4GUI::InputDialog`, including
//! its compact ordinary-chat layout.
//!
//! The controller owns only presentation and input state. Callers translate
//! [`InputDialogAction::Accepted`] into the C++ callback's side effect and
//! retain ownership of configuration, networking, and the modal stack.

use std::borrow::Cow;
use std::cell::Cell;
use std::time::{Duration, Instant};

use anyhow::{ensure, Result};
use clonk_graphics::clonk_font::{ClonkFont, TextAlign};
use clonk_graphics::{GammaRamp, Surface};
use clonk_gui::Rect as GuiRect;

use crate::classic_gui::{
    blacken_transparent_pixels, draw_3d_frame, draw_clipped_text, draw_engine_box,
    draw_facet_stretch, ClassicButtonState, ClassicGuiSkin, IntRect,
};
use crate::context_menu::{draw_classic_tooltip, ContextMenuEntry, ContextMenuIcon};
use crate::draw_scaled_caret;
use crate::expand_hotkey_markup;
use crate::message_dialog::break_message;
use crate::{ClonkFontSet, GuiPoint, ImageData, KeyCode};

const DIALOG_WIDTH: i32 = 300;
const DIALOG_VERTICAL_ROOM: i32 = 150;
const DIALOG_INDENT: i32 = 10;
const ICON_SIZE: i32 = 40;
const BUTTON_AREA_HEIGHT: i32 = 40;
const BUTTON_WIDTH: i32 = 120;
const BUTTON_HEIGHT: i32 = 32;
const BUTTON_GAP: i32 = 10;
const MIN_WOOD_BAR_HEIGHT: i32 = 23;
const STANDARD_ICON_CELL: u32 = 40;
const EXTENDED_ICON_CELL: u32 = 64;
const CLOSE_ICON_PHASE: u16 = 34;
const DEFAULT_MAX_TEXT: usize = 255;
const EDIT_SCROLL_OFFSET: i32 = 2;
const TOOLTIP_DELAY: Duration = Duration::from_millis(500);
const TITLE_SCROLL_DELAY: Duration = Duration::from_millis(3000);

const EDIT_BACKGROUND: u32 = 0x7f00_0000;
const EDIT_SELECTION: u32 = 0x7f7f_7f00;
/// The composition underline. Opaque white in `draw_engine_box`'s packed form,
/// matching the text it sits under.
const EDIT_COMPOSITION_UNDERLINE: u32 = 0x00ff_ffff;
const WHITE: [u8; 4] = [255, 255, 255, 255];

/// A normal 40px `GUIIcons.png` phase or extended 64px `GUIIcons2.png`
/// phase. `InputDialog` scales either source into its fixed 40px icon slot.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum InputDialogIcon {
    #[default]
    None,
    Standard(u16),
    Extended(u16),
}

impl InputDialogIcon {
    pub const OPTIONS: Self = Self::Standard(14);
    pub const LOCKED: Self = Self::Extended(11);
    pub const LOCKED_FRONTAL: Self = Self::Extended(13);
    pub const COMMENT: Self = Self::Extended(17);
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum InputDialogPlacement {
    #[default]
    Centered,
    /// Free-place chat dialogs use the centered position plus one third of
    /// the full screen height (`Screen::ShowDialog`).
    BottomThird,
    /// Shared-mode C++ screens place ordinary dialogs at the preferred
    /// viewport origin plus `(30, 30)`.
    Preferred { x: i32, y: i32 },
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum InputDialogControl {
    Close,
    Edit,
    Ok,
    Cancel,
}

#[derive(Clone, Debug, PartialEq)]
pub enum InputDialogAction {
    FocusChanged(InputDialogControl),
    TextChanged(String),
    /// One complete line from a multiline compact-chat paste. The dialog
    /// remains open for the remaining clipboard text.
    SubmittedLine(String),
    ClipboardWrite(String),
    OpenContextMenu(InputDialogContextMenuRequest),
    Accepted(String),
    Cancelled,
}

/// Capture metadata for hosts that must keep a closing key/gamepad release
/// from reaching the dialog underneath this modal.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct InputDialogInputOutcome {
    pub captured: bool,
    pub capture_release: bool,
    pub actions: Vec<InputDialogAction>,
}

impl InputDialogInputOutcome {
    fn passed() -> Self {
        Self::default()
    }

    fn captured(actions: Vec<InputDialogAction>) -> Self {
        Self {
            captured: true,
            capture_release: false,
            actions,
        }
    }

    fn captured_down(actions: Vec<InputDialogAction>) -> Self {
        Self {
            captured: true,
            capture_release: true,
            actions,
        }
    }
}

/// Localized labels used by the two standard close buttons.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct InputDialogButtonLabels {
    pub ok: String,
    pub cancel: String,
}

impl InputDialogButtonLabels {
    pub fn new(ok: impl Into<String>, cancel: impl Into<String>) -> Self {
        Self {
            ok: ok.into(),
            cancel: cancel.into(),
        }
    }
}

impl Default for InputDialogButtonLabels {
    fn default() -> Self {
        Self::new("&OK", "Cancel")
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum InputDialogContextCommand {
    Cut,
    Copy,
    Paste,
    Clear,
    SelectAll,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct InputDialogContextMenuItem {
    pub command: InputDialogContextCommand,
    pub label: String,
    pub tooltip: String,
}

#[derive(Clone, Debug, PartialEq)]
pub struct InputDialogContextMenuRequest {
    pub anchor: GuiPoint,
    pub items: Vec<InputDialogContextMenuItem>,
}

#[derive(Clone, Debug, PartialEq)]
pub struct InputDialogTooltip {
    pub pointer: GuiPoint,
    pub text: String,
}

impl InputDialogContextMenuRequest {
    /// Converts the typed request directly into the shared recursive classic
    /// context-menu chassis. Edit entries use `Ico_None` in C++.
    pub fn entries(&self) -> Vec<ContextMenuEntry<InputDialogContextCommand>> {
        self.items
            .iter()
            .map(|item| {
                ContextMenuEntry::new(&item.label)
                    .with_tooltip(&item.tooltip)
                    .with_icon(ContextMenuIcon::None)
                    .with_action(item.command)
            })
            .collect()
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct InputDialogContextLabels {
    pub cut: String,
    pub cut_tooltip: String,
    pub copy: String,
    pub copy_tooltip: String,
    pub paste: String,
    pub paste_tooltip: String,
    pub clear: String,
    pub clear_tooltip: String,
    pub select_all: String,
    pub select_all_tooltip: String,
}

impl Default for InputDialogContextLabels {
    fn default() -> Self {
        Self {
            cut: "Cut".into(),
            cut_tooltip: "Moves the selection to the clipboard.".into(),
            copy: "Copy".into(),
            copy_tooltip: "Copies the selection to the clipboard.".into(),
            paste: "Paste".into(),
            paste_tooltip: "Inserts the contents of the clipboard.".into(),
            clear: "Clear".into(),
            clear_tooltip: "Clears the selection.".into(),
            select_all: "Select all".into(),
            select_all_tooltip: "Selects the complete text".into(),
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum InputDialogClipboardShortcut {
    Copy,
    Cut,
    Paste,
    SelectAll,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum InputDialogSound {
    ArrowHit,
    Click,
}

/// Edit-only keys not represented by the frontend-wide [`KeyCode`].
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum InputDialogEditKey {
    Left,
    Right,
    Home,
    End,
    Backspace,
    Delete,
    SelectAll,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct InputDialogKeyModifiers {
    pub shift: bool,
    pub control: bool,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct InputDialogLayout {
    pub bounds: IntRect,
    pub caption: Option<IntRect>,
    pub client: IntRect,
    pub close_button: Option<IntRect>,
    pub icon: IntRect,
    /// Remaining `ComponentAligner` area used to anchor the centered label.
    pub message: IntRect,
    pub message_text: String,
    pub edit: IntRect,
    pub ok_button: IntRect,
    pub cancel_button: IntRect,
}

/// Validated classic assets. Bilinearly sampled facets are copied with RGB
/// cleared below fully transparent pixels, matching the engine texture path.
#[derive(Clone)]
pub struct InputDialogResources<'a> {
    skin: ClassicGuiSkin<'a>,
    fonts: &'a ClonkFontSet,
    tooltip_font: &'a ClonkFont,
    icons: ImageData,
    icons_extended: ImageData,
    button_highlight: ImageData,
}

impl<'a> InputDialogResources<'a> {
    pub fn new(
        skin: ClassicGuiSkin<'a>,
        fonts: &'a ClonkFontSet,
        tooltip_font: &'a ClonkFont,
        icons: &ImageData,
        icons_extended: &ImageData,
        button_highlight: &ImageData,
    ) -> Result<Self> {
        let resources = Self {
            skin,
            fonts,
            tooltip_font,
            icons: blacken_transparent_pixels(icons),
            icons_extended: blacken_transparent_pixels(icons_extended),
            button_highlight: blacken_transparent_pixels(button_highlight),
        };
        resources.validate()?;
        Ok(resources)
    }

    pub fn fonts(&self) -> &ClonkFontSet {
        self.fonts
    }

    fn validate(&self) -> Result<()> {
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
            self.button_highlight.width() > 0 && self.button_highlight.height() > 0,
            "GUIButtonHighlight.png must be a non-empty full-size classic facet, got {}x{}",
            self.button_highlight.width(),
            self.button_highlight.height()
        );
        ensure!(
            self.fonts.text.line_height > 0,
            "classic TextFont must have a positive line height"
        );
        ensure!(
            self.tooltip_font.line_height > 0,
            "classic TooltipFont must have a positive line height"
        );
        Ok(())
    }
}

#[derive(Clone, Copy, Debug, Default)]
struct CaptionScrollState {
    last_change: Option<Instant>,
    position: i32,
    direction: i8,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum ButtonTarget {
    Close,
    Ok,
    Cancel,
}

impl ButtonTarget {
    const fn control(self) -> InputDialogControl {
        match self {
            Self::Close => InputDialogControl::Close,
            Self::Ok => InputDialogControl::Ok,
            Self::Cancel => InputDialogControl::Cancel,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum HitTarget {
    Close,
    Edit,
    Ok,
    Cancel,
    Caption,
    Message,
    None,
}

impl HitTarget {
    const fn button(self) -> Option<ButtonTarget> {
        match self {
            Self::Close => Some(ButtonTarget::Close),
            Self::Ok => Some(ButtonTarget::Ok),
            Self::Cancel => Some(ButtonTarget::Cancel),
            Self::Edit | Self::Caption | Self::Message | Self::None => None,
        }
    }
}

#[derive(Clone, Copy, Debug)]
struct TitleDrag {
    pointer: GuiPoint,
    offset: (i32, i32),
}

/// An IME composition in progress, as `WindowEvent::Ime::Preedit` reports it.
///
/// Provisional text: it is drawn in the field so the user can see what they are
/// composing, and it never enters the committed text. Only `Ime::Commit`
/// reaches the ordinary input path, which is why enabling this changes nothing
/// about what the field finally submits.
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct ImeComposition {
    pub text: String,
    /// The IME's own cursor inside `text`, as a byte range. `None` means the
    /// IME reported no cursor, which winit documents as "hide the cursor"; the
    /// caret then sits after the whole composition.
    pub cursor: Option<(usize, usize)>,
}

impl ImeComposition {
    /// Where the caret belongs inside the composition, in bytes from its start.
    fn caret_offset(&self) -> usize {
        self.cursor
            .map_or(self.text.len(), |(start, _)| start.min(self.text.len()))
    }
}

/// Pure controller for one classic input dialog, including the compact chat layout.
#[derive(Clone, Debug)]
pub struct InputDialogController {
    message: String,
    caption: String,
    icon: InputDialogIcon,
    button_labels: InputDialogButtonLabels,
    close_tooltip: String,
    chat_tooltip: String,
    caption_scroll: Cell<CaptionScrollState>,
    placement: InputDialogPlacement,
    dialog_offset: (i32, i32),
    text: String,
    max_text: usize,
    caret: usize,
    selection: Option<(usize, usize)>,
    composition: Option<ImeComposition>,
    horizontal_scroll: i32,
    focus: InputDialogControl,
    pointer: Option<GuiPoint>,
    pointer_active: bool,
    tooltip_since: Instant,
    hovered: Option<ButtonTarget>,
    pointer_pressed: Option<ButtonTarget>,
    key_pressed: Option<(ButtonTarget, KeyCode)>,
    edit_drag_anchor: Option<usize>,
    title_drag: Option<TitleDrag>,
    chat_layout: bool,
    last_edit_input: Instant,
    sound_events: Vec<InputDialogSound>,
}

impl InputDialogController {
    pub fn new(
        message: impl Into<String>,
        caption: impl Into<String>,
        icon: InputDialogIcon,
    ) -> Self {
        Self {
            message: message.into(),
            caption: caption.into(),
            icon,
            button_labels: InputDialogButtonLabels::default(),
            close_tooltip: "Close".into(),
            chat_tooltip: String::new(),
            caption_scroll: Cell::new(CaptionScrollState::default()),
            placement: InputDialogPlacement::Centered,
            dialog_offset: (0, 0),
            text: String::new(),
            max_text: DEFAULT_MAX_TEXT,
            caret: 0,
            selection: None,
            composition: None,
            horizontal_scroll: 0,
            focus: InputDialogControl::Edit,
            pointer: None,
            pointer_active: false,
            tooltip_since: Instant::now(),
            hovered: None,
            pointer_pressed: None,
            key_pressed: None,
            edit_drag_anchor: None,
            title_drag: None,
            chat_layout: false,
            last_edit_input: Instant::now(),
            sound_events: Vec::new(),
        }
    }

    /// Ordinary `C4ChatInputDialog` passes a null title and selects
    /// `InputDialog`'s compact chat branch: an inline wooden label and edit,
    /// without title, close icon, or OK/Cancel buttons.
    pub fn new_chat(message: impl Into<String>, initial_text: &str) -> Self {
        let mut controller = Self::new(message, "", InputDialogIcon::None)
            .with_placement(InputDialogPlacement::BottomThird);
        controller.chat_layout = true;
        controller.text = truncate_utf8(initial_text, controller.payload_limit()).to_string();
        controller.caret = controller.text.len();
        controller.selection = None;
        controller
    }

    pub fn with_placement(mut self, placement: InputDialogPlacement) -> Self {
        self.placement = placement;
        self
    }

    pub fn with_button_labels(mut self, labels: InputDialogButtonLabels) -> Self {
        self.button_labels = labels;
        self
    }

    pub fn with_close_tooltip(mut self, tooltip: impl Into<String>) -> Self {
        self.close_tooltip = tooltip.into();
        self
    }

    pub fn with_chat_tooltip(mut self, tooltip: impl Into<String>) -> Self {
        self.chat_tooltip = tooltip.into();
        self
    }

    /// Mirrors `Edit::SetMaxText`. The C++ value includes room for the
    /// terminating NUL, so at most `max_text - 1` bytes are accepted.
    pub fn with_max_text(mut self, max_text: usize) -> Self {
        self.set_max_text(max_text);
        self
    }

    /// Mirrors `InputDialog::SetInputText`, including truncation and the
    /// final select-all operation.
    pub fn with_input_text(mut self, text: &str) -> Self {
        self.set_input_text(text);
        self
    }

    pub fn message(&self) -> &str {
        &self.message
    }

    pub fn caption(&self) -> &str {
        &self.caption
    }

    pub const fn icon(&self) -> InputDialogIcon {
        self.icon
    }

    pub fn button_labels(&self) -> &InputDialogButtonLabels {
        &self.button_labels
    }

    pub fn text(&self) -> &str {
        &self.text
    }

    pub const fn max_text(&self) -> usize {
        self.max_text
    }

    pub const fn focused_control(&self) -> InputDialogControl {
        self.focus
    }

    pub const fn caret(&self) -> usize {
        self.caret
    }

    /// Replaces the composition in progress. `None` ends it, which is what
    /// `Ime::Commit` and `Ime::Disabled` both mean.
    pub fn set_composition(&mut self, composition: Option<ImeComposition>) {
        self.composition = composition.filter(|composition| !composition.text.is_empty());
    }

    pub fn composition(&self) -> Option<&ImeComposition> {
        self.composition.as_ref()
    }

    /// The text the field draws: the committed text with any composition
    /// inserted at the caret.
    ///
    /// Borrowed when nothing is being composed, which is every frame outside an
    /// IME session.
    pub fn displayed_text(&self) -> Cow<'_, str> {
        match &self.composition {
            None => Cow::Borrowed(self.text.as_str()),
            Some(composition) => {
                let mut displayed = String::with_capacity(self.text.len() + composition.text.len());
                displayed.push_str(&self.text[..self.caret]);
                displayed.push_str(&composition.text);
                displayed.push_str(&self.text[self.caret..]);
                Cow::Owned(displayed)
            }
        }
    }

    /// The caret's byte offset within [`Self::displayed_text`].
    pub fn displayed_caret(&self) -> usize {
        self.caret
            + self
                .composition
                .as_ref()
                .map_or(0, ImeComposition::caret_offset)
    }

    /// The caret's rectangle inside the edit, for positioning the platform IME
    /// candidate window.
    ///
    /// The same arithmetic `render_edit` uses for the caret, so the candidate
    /// list appears under the character actually being composed rather than at
    /// the window origin.
    pub fn caret_area(&self, layout: &InputDialogLayout, font: &ClonkFont) -> IntRect {
        let client = edit_client(layout.edit);
        let displayed = self.displayed_text();
        let caret_x = client.x + font.measure(&displayed[..self.displayed_caret()], false).0
            - self.horizontal_scroll;
        let (text_y0, height) = if client.h <= font.line_height {
            (client.y, client.h)
        } else {
            (
                client.y + (client.h - font.line_height) / 2 + 1,
                font.line_height - 2,
            )
        };
        IntRect {
            x: caret_x.clamp(client.x, client.x + client.w),
            y: text_y0,
            w: 1,
            h: height.max(1),
        }
    }

    /// The composition's byte range within [`Self::displayed_text`], which is
    /// what the underline spans.
    pub fn displayed_composition_range(&self) -> Option<(usize, usize)> {
        self.composition
            .as_ref()
            .map(|composition| (self.caret, self.caret + composition.text.len()))
    }

    pub const fn selection(&self) -> Option<(usize, usize)> {
        self.selection
    }

    pub const fn horizontal_scroll(&self) -> i32 {
        self.horizontal_scroll
    }

    pub const fn dialog_offset(&self) -> (i32, i32) {
        self.dialog_offset
    }

    pub const fn is_chat_layout(&self) -> bool {
        self.chat_layout
    }

    pub fn set_max_text(&mut self, max_text: usize) {
        // C++ does not retroactively truncate an already populated edit.
        self.max_text = max_text;
    }

    pub fn set_input_text(&mut self, text: &str) {
        self.text = truncate_utf8(text, self.payload_limit()).to_string();
        self.caret = self.text.len();
        self.selection = (!self.text.is_empty()).then_some((0, self.text.len()));
        self.last_edit_input = Instant::now();
    }

    /// Replaces text through the focused Edit input path, which scrolls the
    /// new end caret into view before selecting the replacement.
    pub fn replace_edit_text(&mut self, text: &str, layout: &InputDialogLayout, font: &ClonkFont) {
        self.set_input_text(text);
        self.ensure_cursor_in_view(layout, font);
    }

    pub fn selected_text(&self) -> Option<&str> {
        let (start, end) = self.selected_range()?;
        self.text.get(start..end)
    }

    pub fn take_sound_events(&mut self) -> Vec<InputDialogSound> {
        std::mem::take(&mut self.sound_events)
    }

    pub fn layout(
        &self,
        screen_width: i32,
        screen_height: i32,
        font: &ClonkFont,
    ) -> InputDialogLayout {
        if self.chat_layout {
            let edit_height = (font.line_height + 3).max(MIN_WOOD_BAR_HEIGHT);
            let width = screen_width * 4 / 5;
            let height = edit_height + 2;
            let (base_x, base_y) = match self.placement {
                InputDialogPlacement::Centered => {
                    ((screen_width - width) / 2, (screen_height - height) / 2)
                }
                InputDialogPlacement::BottomThird => (
                    (screen_width - width) / 2,
                    (screen_height - height) / 2 + screen_height / 3,
                ),
                InputDialogPlacement::Preferred { x, y } => (x + 30, y + 30),
            };
            let x = base_x + self.dialog_offset.0;
            let y = base_y + self.dialog_offset.1;
            let label_width = font.measure(&self.message, true).0 + 4;
            let empty = IntRect { x, y, w: 0, h: 0 };
            return InputDialogLayout {
                bounds: IntRect {
                    x,
                    y,
                    w: width,
                    h: height,
                },
                caption: None,
                client: IntRect {
                    x,
                    y,
                    w: width,
                    h: height,
                },
                close_button: None,
                icon: empty,
                message: IntRect {
                    x: x + 1,
                    y: y + 1,
                    w: label_width,
                    h: edit_height,
                },
                message_text: self.message.clone(),
                edit: IntRect {
                    x: x + label_width + 1,
                    y: y + 1,
                    w: width - label_width - 2,
                    h: edit_height,
                },
                ok_button: empty,
                cancel_button: empty,
            };
        }
        let message_text = break_message(
            font,
            &self.message,
            DIALOG_WIDTH - 3 * DIALOG_INDENT - ICON_SIZE,
        );
        let message_height = font.measure(&message_text, true).1;
        let height = message_height.max(ICON_SIZE) + DIALOG_VERTICAL_ROOM;
        let title_height = if self.caption.is_empty() {
            0
        } else {
            font.line_height.max(MIN_WOOD_BAR_HEIGHT)
        };
        let (base_x, base_y) = match self.placement {
            InputDialogPlacement::Centered => (
                (screen_width - DIALOG_WIDTH) / 2,
                (screen_height - height) / 2,
            ),
            InputDialogPlacement::BottomThird => (
                (screen_width - DIALOG_WIDTH) / 2,
                (screen_height - height) / 2 + screen_height / 3,
            ),
            InputDialogPlacement::Preferred { x, y } => (x + 30, y + 30),
        };
        let x = base_x + self.dialog_offset.0;
        let y = base_y + self.dialog_offset.1;
        let client = IntRect {
            x,
            y: y + title_height,
            w: DIALOG_WIDTH,
            h: height - title_height,
        };

        // ComponentAligner(GetClientRect(), 10, 10, true).
        let mut remaining_height = client.h;
        let button_area = IntRect {
            x: client.x + DIALOG_INDENT,
            y: client.y + remaining_height - BUTTON_AREA_HEIGHT - DIALOG_INDENT,
            w: client.w - 2 * DIALOG_INDENT,
            h: BUTTON_AREA_HEIGHT,
        };
        remaining_height -= BUTTON_AREA_HEIGHT + 2 * DIALOG_INDENT;
        let edit_height = (font.line_height + 3).max(MIN_WOOD_BAR_HEIGHT);
        let edit = IntRect {
            x: client.x + DIALOG_INDENT,
            y: client.y + remaining_height - edit_height - DIALOG_INDENT,
            w: client.w - 2 * DIALOG_INDENT,
            h: edit_height,
        };
        remaining_height -= edit_height + 2 * DIALOG_INDENT;
        let icon = IntRect {
            x: client.x + DIALOG_INDENT,
            y: client.y + DIALOG_INDENT,
            w: ICON_SIZE,
            h: ICON_SIZE,
        };
        let message = IntRect {
            x: client.x + ICON_SIZE + 3 * DIALOG_INDENT,
            y: client.y + DIALOG_INDENT,
            w: client.w - ICON_SIZE - 4 * DIALOG_INDENT,
            h: remaining_height - 2 * DIALOG_INDENT,
        };
        let group_width = 2 * BUTTON_WIDTH + BUTTON_GAP;
        let first_button_x = button_area.x + button_area.w / 2 - group_width / 2;
        let button_y = button_area.y + button_area.h / 2 - BUTTON_HEIGHT / 2;

        InputDialogLayout {
            bounds: IntRect {
                x,
                y,
                w: DIALOG_WIDTH,
                h: height,
            },
            caption: (title_height > 0).then_some(IntRect {
                x,
                y,
                w: DIALOG_WIDTH,
                h: title_height,
            }),
            client,
            close_button: (title_height > 0).then_some(IntRect {
                x: x + DIALOG_WIDTH - 20,
                y: y + 4,
                w: 16,
                h: 16,
            }),
            icon,
            message,
            message_text,
            edit,
            ok_button: IntRect {
                x: first_button_x,
                y: button_y,
                w: BUTTON_WIDTH,
                h: BUTTON_HEIGHT,
            },
            cancel_button: IntRect {
                x: first_button_x + BUTTON_WIDTH + BUTTON_GAP,
                y: button_y,
                w: BUTTON_WIDTH,
                h: BUTTON_HEIGHT,
            },
        }
    }

    /// Routes the frontend-wide key set. `shift` distinguishes reverse Tab
    /// and extending edit selections.
    pub fn route_key_down(
        &mut self,
        key: KeyCode,
        shift: bool,
        layout: &InputDialogLayout,
        font: &ClonkFont,
    ) -> InputDialogInputOutcome {
        let focus = self.focus;
        let captured = match key {
            KeyCode::Escape | KeyCode::Tab | KeyCode::Enter => true,
            KeyCode::Space => focus != InputDialogControl::Edit,
            KeyCode::Left | KeyCode::Right => focus == InputDialogControl::Edit,
            KeyCode::Up
            | KeyCode::Down
            | KeyCode::Home
            | KeyCode::End
            | KeyCode::PageUp
            | KeyCode::PageDown => false,
        };
        let capture_release = matches!(key, KeyCode::Escape | KeyCode::Enter)
            || (key == KeyCode::Space && focus != InputDialogControl::Edit);
        let actions = self.handle_key_down(key, shift, layout, font);
        InputDialogInputOutcome {
            captured,
            capture_release: captured && capture_release,
            actions,
        }
    }

    pub fn handle_key_down(
        &mut self,
        key: KeyCode,
        shift: bool,
        layout: &InputDialogLayout,
        font: &ClonkFont,
    ) -> Vec<InputDialogAction> {
        self.pointer_active = false;
        match key {
            KeyCode::Escape => vec![InputDialogAction::Cancelled],
            KeyCode::Tab => self.advance_focus(shift),
            KeyCode::Enter if self.focus == InputDialogControl::Edit => {
                vec![InputDialogAction::Accepted(self.text.clone())]
            }
            KeyCode::Enter | KeyCode::Space => {
                let Some(target) = self.focus_button_target() else {
                    return Vec::new();
                };
                if self.key_pressed.is_none() {
                    self.key_pressed = Some((target, key));
                    self.sound_events.push(InputDialogSound::ArrowHit);
                }
                Vec::new()
            }
            KeyCode::Left if self.focus == InputDialogControl::Edit => self.handle_edit_key_down(
                InputDialogEditKey::Left,
                InputDialogKeyModifiers {
                    shift,
                    control: false,
                },
                layout,
                font,
            ),
            KeyCode::Right if self.focus == InputDialogControl::Edit => self.handle_edit_key_down(
                InputDialogEditKey::Right,
                InputDialogKeyModifiers {
                    shift,
                    control: false,
                },
                layout,
                font,
            ),
            KeyCode::Up
            | KeyCode::Down
            | KeyCode::Left
            | KeyCode::Right
            | KeyCode::Home
            | KeyCode::End
            | KeyCode::PageUp
            | KeyCode::PageDown => Vec::new(),
        }
    }

    pub fn handle_key_up(&mut self, key: KeyCode) -> Vec<InputDialogAction> {
        let Some((target, pressed_key)) = self.key_pressed else {
            return Vec::new();
        };
        if key != pressed_key {
            return Vec::new();
        }
        self.key_pressed = None;
        if self.focus_button_target() != Some(target) {
            return Vec::new();
        }
        self.sound_events.push(InputDialogSound::Click);
        self.activate_button(target)
    }

    pub fn route_key_up(&mut self, key: KeyCode) -> InputDialogInputOutcome {
        let captured = self
            .key_pressed
            .is_some_and(|(_, pressed_key)| pressed_key == key);
        let actions = self.handle_key_up(key);
        InputDialogInputOutcome {
            captured,
            capture_release: false,
            actions,
        }
    }

    pub fn handle_edit_key_down(
        &mut self,
        key: InputDialogEditKey,
        modifiers: InputDialogKeyModifiers,
        layout: &InputDialogLayout,
        font: &ClonkFont,
    ) -> Vec<InputDialogAction> {
        self.pointer_active = false;
        if self.focus != InputDialogControl::Edit {
            return Vec::new();
        }
        let old_text = self.text.clone();
        match key {
            InputDialogEditKey::SelectAll => {
                self.caret = self.text.len();
                self.selection = (!self.text.is_empty()).then_some((0, self.text.len()));
            }
            InputDialogEditKey::Backspace | InputDialogEditKey::Delete
                if self.delete_selection() => {}
            InputDialogEditKey::Backspace | InputDialogEditKey::Delete if modifiers.shift => {}
            InputDialogEditKey::Backspace => {
                let start = if modifiers.control {
                    self.word_boundary(-1)
                } else {
                    previous_boundary(&self.text, self.caret)
                };
                if start < self.caret {
                    self.text.replace_range(start..self.caret, "");
                    self.caret = start;
                }
                self.selection = None;
            }
            InputDialogEditKey::Delete => {
                let end = if modifiers.control {
                    self.word_boundary(1)
                } else {
                    next_boundary(&self.text, self.caret)
                };
                if end > self.caret {
                    self.text.replace_range(self.caret..end, "");
                }
                self.selection = None;
            }
            InputDialogEditKey::Left
            | InputDialogEditKey::Right
            | InputDialogEditKey::Home
            | InputDialogEditKey::End => {
                let destination = match key {
                    InputDialogEditKey::Left if modifiers.control => self.word_boundary(-1),
                    InputDialogEditKey::Right if modifiers.control => self.word_boundary(1),
                    InputDialogEditKey::Left => previous_boundary(&self.text, self.caret),
                    InputDialogEditKey::Right => next_boundary(&self.text, self.caret),
                    InputDialogEditKey::Home => 0,
                    InputDialogEditKey::End => self.text.len(),
                    _ => self.caret,
                };
                if modifiers.shift {
                    let anchor = self.selection.map_or(self.caret, |(anchor, _)| anchor);
                    self.caret = destination;
                    self.selection = (anchor != destination).then_some((anchor, destination));
                } else {
                    self.selection = None;
                    self.caret = destination;
                }
            }
        }
        self.last_edit_input = Instant::now();
        self.ensure_cursor_in_view(layout, font);
        if self.text != old_text {
            vec![InputDialogAction::TextChanged(self.text.clone())]
        } else {
            Vec::new()
        }
    }

    /// C4GUI sends printable text separately from navigation key events.
    pub fn handle_text_input(
        &mut self,
        input: &str,
        layout: &InputDialogLayout,
        font: &ClonkFont,
    ) -> Vec<InputDialogAction> {
        self.pointer_active = false;
        if self.focus != InputDialogControl::Edit {
            return Vec::new();
        }
        let filtered: String = input
            .chars()
            .filter(|character| !character.is_control() && *character != '\u{7f}')
            .map(|character| {
                if character == '|' {
                    '\u{a6}'
                } else {
                    character
                }
            })
            .collect();
        if filtered.is_empty() {
            return Vec::new();
        }
        let old_text = self.text.clone();
        self.delete_selection();
        let available = self.payload_limit().saturating_sub(self.text.len());
        let insert = truncate_utf8(&filtered, available);
        if !insert.is_empty() {
            self.text.insert_str(self.caret, insert);
            self.caret += insert.len();
        }
        self.selection = None;
        self.last_edit_input = Instant::now();
        self.ensure_cursor_in_view(layout, font);
        if self.text != old_text {
            vec![InputDialogAction::TextChanged(self.text.clone())]
        } else {
            Vec::new()
        }
    }

    pub fn handle_hotkey(&mut self, character: char) -> Vec<InputDialogAction> {
        self.pointer_active = false;
        if self.chat_layout {
            return Vec::new();
        }
        let character = character.to_ascii_uppercase();
        let ok_hotkey = expand_hotkey_markup(&self.button_labels.ok).1;
        let cancel_hotkey = expand_hotkey_markup(&self.button_labels.cancel).1;
        if ok_hotkey == Some(character) {
            vec![InputDialogAction::Accepted(self.text.clone())]
        } else if cancel_hotkey == Some(character) {
            vec![InputDialogAction::Cancelled]
        } else {
            Vec::new()
        }
    }

    pub fn has_hotkey(&self, character: char) -> bool {
        if self.chat_layout {
            return false;
        }
        let character = character.to_ascii_uppercase();
        expand_hotkey_markup(&self.button_labels.ok).1 == Some(character)
            || expand_hotkey_markup(&self.button_labels.cancel).1 == Some(character)
    }

    /// Opens the edit's recursive classic context menu for a right click.
    /// The host passes [`InputDialogContextMenuRequest::entries`] to
    /// `ClassicContextMenu::open` and returns the activated command through
    /// [`Self::apply_context_command`].
    pub fn request_context_menu_at(
        &mut self,
        point: GuiPoint,
        layout: &InputDialogLayout,
        clipboard_has_text: bool,
        labels: &InputDialogContextLabels,
    ) -> InputDialogInputOutcome {
        self.note_pointer_input(point, true);
        if hit_target(layout, point) != HitTarget::Edit {
            return InputDialogInputOutcome::passed();
        }
        InputDialogInputOutcome::captured(vec![InputDialogAction::OpenContextMenu(
            self.context_menu_request(point, clipboard_has_text, labels),
        )])
    }

    /// `K_MENU` is registered on every focused C4GUI control; Edit anchors
    /// its popup at the control center.
    pub fn request_context_menu_from_key(
        &mut self,
        layout: &InputDialogLayout,
        clipboard_has_text: bool,
        labels: &InputDialogContextLabels,
    ) -> InputDialogInputOutcome {
        self.pointer_active = false;
        if self.focus != InputDialogControl::Edit {
            return InputDialogInputOutcome::passed();
        }
        InputDialogInputOutcome::captured(vec![InputDialogAction::OpenContextMenu(
            self.context_menu_request(center(layout.edit), clipboard_has_text, labels),
        )])
    }

    pub fn handle_clipboard_shortcut(
        &mut self,
        shortcut: InputDialogClipboardShortcut,
        clipboard_text: Option<&str>,
        layout: &InputDialogLayout,
        font: &ClonkFont,
    ) -> InputDialogInputOutcome {
        self.pointer_active = false;
        if self.focus != InputDialogControl::Edit {
            return InputDialogInputOutcome::passed();
        }
        let command = match shortcut {
            InputDialogClipboardShortcut::Copy => InputDialogContextCommand::Copy,
            InputDialogClipboardShortcut::Cut => InputDialogContextCommand::Cut,
            InputDialogClipboardShortcut::Paste => InputDialogContextCommand::Paste,
            InputDialogClipboardShortcut::SelectAll => InputDialogContextCommand::SelectAll,
        };
        InputDialogInputOutcome::captured(self.apply_context_command(
            command,
            clipboard_text,
            layout,
            font,
        ))
    }

    pub fn apply_context_command(
        &mut self,
        command: InputDialogContextCommand,
        clipboard_text: Option<&str>,
        layout: &InputDialogLayout,
        font: &ClonkFont,
    ) -> Vec<InputDialogAction> {
        match command {
            InputDialogContextCommand::Copy => self
                .selected_text()
                .map(|text| vec![InputDialogAction::ClipboardWrite(text.to_string())])
                .unwrap_or_default(),
            InputDialogContextCommand::Cut => {
                let Some(copied) = self.selected_text().map(str::to_string) else {
                    return Vec::new();
                };
                self.delete_selection();
                self.last_edit_input = Instant::now();
                self.ensure_cursor_in_view(layout, font);
                vec![
                    InputDialogAction::ClipboardWrite(copied),
                    InputDialogAction::TextChanged(self.text.clone()),
                ]
            }
            InputDialogContextCommand::Paste => clipboard_text
                .filter(|text| !text.is_empty())
                .map(|text| self.paste_text(text, layout, font))
                .unwrap_or_default(),
            InputDialogContextCommand::Clear => {
                if !self.delete_selection() {
                    return Vec::new();
                }
                self.last_edit_input = Instant::now();
                self.ensure_cursor_in_view(layout, font);
                vec![InputDialogAction::TextChanged(self.text.clone())]
            }
            InputDialogContextCommand::SelectAll => {
                self.caret = self.text.len();
                self.selection = (!self.text.is_empty()).then_some((0, self.text.len()));
                self.last_edit_input = Instant::now();
                Vec::new()
            }
        }
    }

    /// Non-Windows C4GUI pastes the primary selection on middle-down after
    /// moving the caret, without changing keyboard focus.
    pub fn handle_pointer_middle_down(
        &mut self,
        point: GuiPoint,
        primary_selection: Option<&str>,
        layout: &InputDialogLayout,
        font: &ClonkFont,
    ) -> InputDialogInputOutcome {
        self.note_pointer_input(point, true);
        if hit_target(layout, point) != HitTarget::Edit {
            return InputDialogInputOutcome::passed();
        }
        self.caret = self.character_at(point.x, layout, font);
        self.selection = Some((self.caret, self.caret));
        self.last_edit_input = Instant::now();
        let actions = primary_selection
            .filter(|text| !text.is_empty())
            .map(|text| self.paste_text(text, layout, font))
            .unwrap_or_default();
        InputDialogInputOutcome::captured(actions)
    }

    pub fn handle_gamepad_direction(&mut self, right: bool) -> Vec<InputDialogAction> {
        self.pointer_active = false;
        self.advance_focus(!right)
    }

    /// Any low gamepad button is a dialog Enter. The edit has no gamepad
    /// control binding, so it accepts immediately on button-down; a focused
    /// standard button captures down/up like its keyboard binding.
    pub fn handle_gamepad_low_down(
        &mut self,
        layout: &InputDialogLayout,
        font: &ClonkFont,
    ) -> Vec<InputDialogAction> {
        self.handle_key_down(KeyCode::Enter, false, layout, font)
    }

    pub fn route_gamepad_low_down(
        &mut self,
        layout: &InputDialogLayout,
        font: &ClonkFont,
    ) -> InputDialogInputOutcome {
        self.route_key_down(KeyCode::Enter, false, layout, font)
    }

    pub fn handle_gamepad_low_up(&mut self) -> Vec<InputDialogAction> {
        self.handle_key_up(KeyCode::Enter)
    }

    pub fn route_gamepad_low_up(&mut self) -> InputDialogInputOutcome {
        self.route_key_up(KeyCode::Enter)
    }

    pub fn handle_gamepad_high_down(&mut self) -> Vec<InputDialogAction> {
        self.pointer_active = false;
        vec![InputDialogAction::Cancelled]
    }

    pub fn route_gamepad_high_down(&mut self) -> InputDialogInputOutcome {
        InputDialogInputOutcome::captured_down(self.handle_gamepad_high_down())
    }

    pub fn handle_pointer_move(
        &mut self,
        point: GuiPoint,
        layout: &InputDialogLayout,
        font: &ClonkFont,
    ) -> Vec<InputDialogAction> {
        let was_down = self.pointer_button_is_down();
        self.note_pointer_input(point, false);
        if let Some(drag) = self.title_drag {
            self.dialog_offset = (
                drag.offset.0 + (point.x - drag.pointer.x) as i32,
                drag.offset.1 + (point.y - drag.pointer.y) as i32,
            );
            return Vec::new();
        }
        if let Some(anchor) = self.edit_drag_anchor {
            self.caret = self.character_at(point.x, layout, font);
            self.selection = Some((anchor, self.caret));
            self.last_edit_input = Instant::now();
            self.ensure_cursor_in_view(layout, font);
        }
        self.hovered = hit_target(layout, point).button();
        if was_down != self.pointer_button_is_down() {
            self.sound_events.push(InputDialogSound::ArrowHit);
        }
        Vec::new()
    }

    pub fn handle_pointer_down(
        &mut self,
        point: GuiPoint,
        layout: &InputDialogLayout,
        font: &ClonkFont,
    ) -> Vec<InputDialogAction> {
        self.note_pointer_input(point, true);
        let hit = hit_target(layout, point);
        self.hovered = hit.button();
        match hit {
            HitTarget::Caption => {
                self.title_drag = Some(TitleDrag {
                    pointer: point,
                    offset: self.dialog_offset,
                });
                Vec::new()
            }
            HitTarget::Edit => {
                let actions = self.set_focus(InputDialogControl::Edit);
                self.caret = self.character_at(point.x, layout, font);
                self.selection = Some((self.caret, self.caret));
                self.edit_drag_anchor = Some(self.caret);
                self.last_edit_input = Instant::now();
                self.ensure_cursor_in_view(layout, font);
                actions
            }
            _ => {
                self.pointer_pressed = hit.button();
                if self.pointer_pressed.is_some() {
                    self.sound_events.push(InputDialogSound::ArrowHit);
                }
                Vec::new()
            }
        }
    }

    pub fn handle_pointer_up(
        &mut self,
        point: GuiPoint,
        layout: &InputDialogLayout,
        font: &ClonkFont,
    ) -> Vec<InputDialogAction> {
        self.stop_pointer_drag_at(point, layout, font);
        let released = hit_target(layout, point).button();
        self.hovered = released;
        let Some(pressed) = self.pointer_pressed.take() else {
            return Vec::new();
        };
        if released != Some(pressed) {
            return Vec::new();
        }
        self.sound_events.push(InputDialogSound::Click);
        self.activate_button(pressed)
    }

    /// Finish CMouse's retained drag at the button-up coordinate before the
    /// global drag element is cleared and ordinary screen hit-testing resumes.
    pub fn stop_pointer_drag_at(
        &mut self,
        point: GuiPoint,
        layout: &InputDialogLayout,
        font: &ClonkFont,
    ) {
        self.note_pointer_input(point, true);
        if let Some(drag) = self.title_drag {
            self.dialog_offset = (
                drag.offset.0 + (point.x - drag.pointer.x) as i32,
                drag.offset.1 + (point.y - drag.pointer.y) as i32,
            );
        }
        if let Some(anchor) = self.edit_drag_anchor {
            self.caret = self.character_at(point.x, layout, font);
            self.selection = Some((anchor, self.caret));
            self.last_edit_input = Instant::now();
            self.ensure_cursor_in_view(layout, font);
        }
        self.edit_drag_anchor = None;
        self.title_drag = None;
    }

    pub fn handle_pointer_double_click(
        &mut self,
        point: GuiPoint,
        layout: &InputDialogLayout,
        font: &ClonkFont,
    ) -> Vec<InputDialogAction> {
        self.note_pointer_input(point, true);
        if hit_target(layout, point) != HitTarget::Edit || self.text.is_empty() {
            return Vec::new();
        }
        let mut position = self.character_at(point.x, layout, font);
        if position == self.text.len() || is_word_spacer(char_at(&self.text, position)) {
            position = previous_boundary(&self.text, position);
            if is_word_spacer(char_at(&self.text, position)) {
                return Vec::new();
            }
        }
        let mut start = position;
        while start > 0 {
            let previous = previous_boundary(&self.text, start);
            if is_word_spacer(char_at(&self.text, previous)) {
                break;
            }
            start = previous;
        }
        let mut end = next_boundary(&self.text, position);
        while end < self.text.len() && !is_word_spacer(char_at(&self.text, end)) {
            end = next_boundary(&self.text, end);
        }
        self.caret = end;
        self.selection = Some((start, end));
        self.edit_drag_anchor = None;
        self.ensure_cursor_in_view(layout, font);
        Vec::new()
    }

    pub fn handle_touch_start(
        &mut self,
        point: GuiPoint,
        layout: &InputDialogLayout,
        font: &ClonkFont,
    ) -> Vec<InputDialogAction> {
        self.handle_pointer_down(point, layout, font)
    }

    pub fn handle_touch_move(
        &mut self,
        point: GuiPoint,
        layout: &InputDialogLayout,
        font: &ClonkFont,
    ) -> Vec<InputDialogAction> {
        self.handle_pointer_move(point, layout, font)
    }

    pub fn handle_touch_end(
        &mut self,
        point: GuiPoint,
        layout: &InputDialogLayout,
        font: &ClonkFont,
    ) -> Vec<InputDialogAction> {
        self.handle_pointer_up(point, layout, font)
    }

    pub fn handle_touch_cancel(&mut self) {
        self.pointer_active = false;
        self.pointer_pressed = None;
        self.edit_drag_anchor = None;
        self.title_drag = None;
    }

    /// Whether this controller currently owns CMouse's drag element.
    /// Labels and other inert client-area hits deliberately do not capture.
    pub const fn has_pointer_capture(&self) -> bool {
        self.pointer_pressed.is_some()
            || self.edit_drag_anchor.is_some()
            || self.title_drag.is_some()
    }

    pub const fn has_positional_pointer_drag(&self) -> bool {
        self.edit_drag_anchor.is_some() || self.title_drag.is_some()
    }

    pub fn pointer_left(&mut self) {
        let was_down = self.pointer_button_is_down();
        self.pointer = None;
        self.pointer_active = false;
        self.hovered = None;
        if was_down {
            self.sound_events.push(InputDialogSound::ArrowHit);
        }
    }

    /// Mirror `CMouse::ReleaseElements` without disturbing keyboard state.
    pub fn release_pointer_elements(&mut self) {
        let was_down = self.pointer_button_is_down();
        self.pointer = None;
        self.pointer_active = false;
        self.hovered = None;
        self.pointer_pressed = None;
        self.edit_drag_anchor = None;
        self.title_drag = None;
        if was_down {
            self.sound_events.push(InputDialogSound::ArrowHit);
        }
    }

    pub fn cancel_interaction(&mut self) {
        self.pointer = None;
        self.pointer_active = false;
        self.hovered = None;
        self.pointer_pressed = None;
        self.key_pressed = None;
        self.edit_drag_anchor = None;
        self.title_drag = None;
        self.sound_events.clear();
    }

    /// Delayed global tooltip state. Regular input dialogs assign tooltips
    /// only to the wooden title and its close icon. Compact chat assigns its
    /// localized chat tooltip to both the inline label and edit.
    pub fn tooltip_state_at(
        &self,
        now: Instant,
        layout: &InputDialogLayout,
        active: bool,
    ) -> Option<InputDialogTooltip> {
        if !active
            || !self.pointer_active
            || now
                .checked_duration_since(self.tooltip_since)
                .unwrap_or_default()
                < TOOLTIP_DELAY
        {
            return None;
        }
        let pointer = self.pointer?;
        let text = match hit_target(layout, pointer) {
            HitTarget::Close => &self.close_tooltip,
            HitTarget::Caption => &self.caption,
            HitTarget::Edit | HitTarget::Message if self.chat_layout => &self.chat_tooltip,
            HitTarget::Edit
            | HitTarget::Ok
            | HitTarget::Cancel
            | HitTarget::Message
            | HitTarget::None => return None,
        };
        (!text.is_empty()).then(|| InputDialogTooltip {
            pointer,
            text: text.clone(),
        })
    }

    /// Draws the tooltip in the host's screen-global final overlay pass.
    pub fn render_tooltip_at(
        &self,
        surface: &mut Surface,
        resources: &InputDialogResources<'_>,
        active: bool,
        gamma: Option<&GammaRamp>,
        now: Instant,
    ) -> Result<()> {
        resources.validate()?;
        let layout = self.layout(
            surface.width() as i32,
            surface.height() as i32,
            &resources.fonts.text,
        );
        if let Some(tooltip) = self.tooltip_state_at(now, &layout, active) {
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
        resources: &InputDialogResources<'_>,
        active: bool,
        gamma: Option<&GammaRamp>,
    ) -> Result<()> {
        self.render_tooltip_at(surface, resources, active, gamma, Instant::now())
    }

    pub fn render(
        &self,
        surface: &mut Surface,
        resources: &InputDialogResources<'_>,
        active: bool,
        gamma: Option<&GammaRamp>,
    ) -> Result<()> {
        self.render_with_activity(surface, resources, active, active, gamma)
    }

    pub fn render_with_activity(
        &self,
        surface: &mut Surface,
        resources: &InputDialogResources<'_>,
        keyboard_active: bool,
        mouse_active: bool,
        gamma: Option<&GammaRamp>,
    ) -> Result<()> {
        let now = Instant::now();
        let cursor_visible = (now
            .checked_duration_since(self.last_edit_input)
            .unwrap_or_default()
            .as_millis()
            / 500)
            .is_multiple_of(2);
        self.render_with_cursor_at_activity(
            surface,
            resources,
            keyboard_active,
            mouse_active,
            cursor_visible,
            gamma,
            now,
        )
    }

    /// Deterministic rendering entry point for cached frames and tests.
    pub fn render_with_cursor(
        &self,
        surface: &mut Surface,
        resources: &InputDialogResources<'_>,
        active: bool,
        cursor_visible: bool,
        gamma: Option<&GammaRamp>,
    ) -> Result<()> {
        self.render_with_cursor_at(
            surface,
            resources,
            active,
            cursor_visible,
            gamma,
            Instant::now(),
        )
    }

    pub fn render_with_cursor_at(
        &self,
        surface: &mut Surface,
        resources: &InputDialogResources<'_>,
        active: bool,
        cursor_visible: bool,
        gamma: Option<&GammaRamp>,
        now: Instant,
    ) -> Result<()> {
        self.render_with_cursor_at_activity(
            surface,
            resources,
            active,
            active,
            cursor_visible,
            gamma,
            now,
        )
    }

    #[allow(clippy::too_many_arguments)]
    fn render_with_cursor_at_activity(
        &self,
        surface: &mut Surface,
        resources: &InputDialogResources<'_>,
        keyboard_active: bool,
        mouse_active: bool,
        cursor_visible: bool,
        gamma: Option<&GammaRamp>,
        now: Instant,
    ) -> Result<()> {
        resources.validate()?;
        let layout = self.layout(
            surface.width() as i32,
            surface.height() as i32,
            &resources.fonts.text,
        );
        resources.skin.draw_dialog(surface, layout.bounds, gamma);
        if self.chat_layout {
            resources.skin.draw_caption(
                surface,
                layout.message,
                &layout.message_text,
                &resources.fonts.text,
                WHITE,
                TextAlign::Center,
                gamma,
            );
            self.render_edit(
                surface,
                layout.edit,
                &resources.fonts.text,
                keyboard_active,
                cursor_visible,
                gamma,
            );
            return Ok(());
        }
        if let Some(caption) = layout.caption {
            let scroll = self.caption_scroll_offset_at(now, &resources.fonts.text);
            resources.skin.draw_caption_scrolled(
                surface,
                caption,
                &self.caption,
                &resources.fonts.text,
                WHITE,
                TextAlign::Left,
                20,
                scroll,
                gamma,
            );
        }
        if let Some(close) = layout.close_button {
            self.render_close(
                surface,
                close,
                resources,
                keyboard_active,
                mouse_active,
                gamma,
            )?;
        }
        draw_icon(
            surface,
            layout.icon,
            self.icon,
            &resources.icons,
            &resources.icons_extended,
            gamma,
        )?;
        resources.fonts.text.draw_with_gamma(
            surface,
            layout.message.x + layout.message.w / 2,
            layout.message.y,
            &layout.message_text,
            WHITE,
            TextAlign::Center,
            true,
            gamma,
        );
        self.render_edit(
            surface,
            layout.edit,
            &resources.fonts.text,
            keyboard_active,
            cursor_visible,
            gamma,
        );
        resources.skin.draw_button_with_highlight(
            surface,
            layout.ok_button,
            &self.button_labels.ok,
            resources.fonts,
            self.button_state(ButtonTarget::Ok, keyboard_active, mouse_active),
            Some(&resources.button_highlight),
            gamma,
        );
        resources.skin.draw_button_with_highlight(
            surface,
            layout.cancel_button,
            &self.button_labels.cancel,
            resources.fonts,
            self.button_state(ButtonTarget::Cancel, keyboard_active, mouse_active),
            Some(&resources.button_highlight),
            gamma,
        );
        Ok(())
    }

    fn render_close(
        &self,
        surface: &mut Surface,
        rect: IntRect,
        resources: &InputDialogResources<'_>,
        keyboard_active: bool,
        mouse_active: bool,
        gamma: Option<&GammaRamp>,
    ) -> Result<()> {
        let state = self.button_state(ButtonTarget::Close, keyboard_active, mouse_active);
        if state.highlighted {
            draw_highlight(surface, rect, &resources.button_highlight, gamma);
        }
        draw_icon(
            surface,
            rect,
            InputDialogIcon::Standard(CLOSE_ICON_PHASE),
            &resources.icons,
            &resources.icons_extended,
            gamma,
        )?;
        if state.pressed {
            draw_highlight(surface, rect, &resources.button_highlight, gamma);
        }
        Ok(())
    }

    fn render_edit(
        &self,
        surface: &mut Surface,
        rect: IntRect,
        font: &ClonkFont,
        active: bool,
        cursor_visible: bool,
        gamma: Option<&GammaRamp>,
    ) {
        let client = edit_client(rect);
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

        let (text_y0, selection_height) = if client.h <= font.line_height {
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
        if let Some((start, end)) = self.selected_range() {
            let x1 = client.x + font.measure(&self.text[..start], false).0 - self.horizontal_scroll;
            let x2 = client.x + font.measure(&self.text[..end], false).0 - self.horizontal_scroll;
            let clipped_x1 = x1.max(clip.x);
            let clipped_x2 = (x2 - 1).min(clip.x + clip.w - 1);
            if clipped_x1 <= clipped_x2 {
                draw_engine_box(
                    surface,
                    clipped_x1,
                    text_y0,
                    clipped_x2,
                    text_y0 + selection_height - 1,
                    EDIT_SELECTION,
                    gamma,
                );
            }
        }
        // An IME composition is provisional text drawn at the caret; with none
        // in progress this is the committed text and the committed caret, so
        // the ordinary field is pixel-for-pixel what it was.
        let displayed = self.displayed_text();
        let displayed_caret = self.displayed_caret();
        draw_clipped_text(
            surface,
            font,
            client.x - self.horizontal_scroll,
            text_y0 - 1,
            &displayed,
            WHITE,
            TextAlign::Left,
            gamma,
            clip,
        );
        // The composition is underlined, which is how every platform marks
        // text the IME has not committed yet.
        if let Some((start, end)) = self.displayed_composition_range() {
            let x1 = client.x + font.measure(&displayed[..start], false).0 - self.horizontal_scroll;
            let x2 = client.x + font.measure(&displayed[..end], false).0 - self.horizontal_scroll;
            let underline_y = text_y0 + selection_height - 1;
            let clipped_x1 = x1.max(clip.x);
            let clipped_x2 = (x2 - 1).min(clip.x + clip.w - 1);
            if clipped_x1 <= clipped_x2 {
                draw_engine_box(
                    surface,
                    clipped_x1,
                    underline_y,
                    clipped_x2,
                    underline_y,
                    EDIT_COMPOSITION_UNDERLINE,
                    gamma,
                );
            }
        }
        if active && self.focus == InputDialogControl::Edit && cursor_visible {
            let caret_x = client.x + font.measure(&displayed[..displayed_caret], false).0
                - font.measure("\u{a6}", false).0 / 2
                - self.horizontal_scroll;
            draw_scaled_caret(
                surface,
                font,
                caret_x,
                text_y0 - font.line_height / 3,
                clip,
                gamma,
            );
        }
    }

    fn payload_limit(&self) -> usize {
        self.max_text.saturating_sub(1)
    }

    fn note_pointer_input(&mut self, point: GuiPoint, button_event: bool) {
        let moved = self
            .pointer
            .is_none_or(|old| old.x as i32 != point.x as i32 || old.y as i32 != point.y as i32);
        self.pointer = Some(point);
        self.pointer_active = true;
        if moved || button_event {
            self.tooltip_since = Instant::now();
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

    fn context_menu_request(
        &self,
        anchor: GuiPoint,
        clipboard_has_text: bool,
        labels: &InputDialogContextLabels,
    ) -> InputDialogContextMenuRequest {
        let has_selection = self.selected_range().is_some();
        let mut items = Vec::new();
        if has_selection {
            items.push(context_item(
                InputDialogContextCommand::Cut,
                &labels.cut,
                &labels.cut_tooltip,
            ));
            items.push(context_item(
                InputDialogContextCommand::Copy,
                &labels.copy,
                &labels.copy_tooltip,
            ));
        }
        if clipboard_has_text {
            items.push(context_item(
                InputDialogContextCommand::Paste,
                &labels.paste,
                &labels.paste_tooltip,
            ));
        }
        if has_selection {
            items.push(context_item(
                InputDialogContextCommand::Clear,
                &labels.clear,
                &labels.clear_tooltip,
            ));
        }
        let whole_text_selected = self.selected_range() == Some((0, self.text.len()));
        if !self.text.is_empty() && !whole_text_selected {
            items.push(context_item(
                InputDialogContextCommand::SelectAll,
                &labels.select_all,
                &labels.select_all_tooltip,
            ));
        }
        InputDialogContextMenuRequest { anchor, items }
    }

    fn paste_text(
        &mut self,
        clipboard: &str,
        layout: &InputDialogLayout,
        font: &ClonkFont,
    ) -> Vec<InputDialogAction> {
        let transformed = clipboard.replace('|', "\u{a6}");
        if self.chat_layout {
            return self.paste_chat_text(&transformed, layout, font);
        }
        let old_text = self.text.clone();
        let mut rest = transformed.as_str();
        while let Some(line_break) = rest.find(['\r', '\n']) {
            if line_break == 0 {
                let skip = rest.chars().next().map_or(0, char::len_utf8);
                rest = &rest[skip..];
                continue;
            }
            self.insert_raw_text(&rest[..line_break]);
            self.last_edit_input = Instant::now();
            self.ensure_cursor_in_view(layout, font);
            let mut actions = Vec::new();
            if self.text != old_text {
                actions.push(InputDialogAction::TextChanged(self.text.clone()));
            }
            // Edit::OnFinishInput returns IR_CloseDlg for the first
            // non-leading pasted line break.
            actions.push(InputDialogAction::Accepted(self.text.clone()));
            return actions;
        }
        if !rest.is_empty() {
            self.insert_raw_text(rest);
        }
        self.last_edit_input = Instant::now();
        self.ensure_cursor_in_view(layout, font);
        if self.text != old_text {
            vec![InputDialogAction::TextChanged(self.text.clone())]
        } else {
            Vec::new()
        }
    }

    fn paste_chat_text(
        &mut self,
        clipboard: &str,
        layout: &InputDialogLayout,
        font: &ClonkFont,
    ) -> Vec<InputDialogAction> {
        let mut actions = Vec::new();
        let mut rest = clipboard;
        while let Some(line_break) = rest.find(['\r', '\n']) {
            if line_break == 0 {
                let skip = rest.chars().next().map_or(0, char::len_utf8);
                rest = &rest[skip..];
                continue;
            }
            let old_text = self.text.clone();
            self.insert_raw_text(&rest[..line_break]);
            self.last_edit_input = Instant::now();
            self.ensure_cursor_in_view(layout, font);
            if self.text != old_text {
                actions.push(InputDialogAction::TextChanged(self.text.clone()));
            }
            rest = &rest[line_break + 1..];
            if rest.is_empty() {
                actions.push(InputDialogAction::Accepted(self.text.clone()));
                return actions;
            }
            actions.push(InputDialogAction::SubmittedLine(self.text.clone()));
            self.caret = self.text.len();
            self.selection = (!self.text.is_empty()).then_some((0, self.text.len()));
        }
        if !rest.is_empty() {
            let old_text = self.text.clone();
            self.insert_raw_text(rest);
            self.last_edit_input = Instant::now();
            self.ensure_cursor_in_view(layout, font);
            if self.text != old_text {
                actions.push(InputDialogAction::TextChanged(self.text.clone()));
            }
        }
        actions
    }

    fn insert_raw_text(&mut self, text: &str) {
        self.delete_selection();
        let available = self.payload_limit().saturating_sub(self.text.len());
        let insert = truncate_utf8(text, available);
        if !insert.is_empty() {
            self.text.insert_str(self.caret, insert);
            self.caret += insert.len();
        }
        self.selection = None;
    }

    fn selected_range(&self) -> Option<(usize, usize)> {
        let (start, end) = self.selection?;
        (start != end).then_some((start.min(end), start.max(end)))
    }

    fn delete_selection(&mut self) -> bool {
        let Some((start, end)) = self.selected_range() else {
            return false;
        };
        self.text.replace_range(start..end, "");
        self.caret = start;
        self.selection = None;
        true
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
                previous_boundary(&self.text, position)
            } else {
                if position >= self.text.len() {
                    break;
                }
                next_boundary(&self.text, position)
            };
            let sample = if direction < 0 { next } else { position };
            if is_word_spacer(char_at(&self.text, sample)) {
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

    fn ensure_cursor_in_view(&mut self, layout: &InputDialogLayout, font: &ClonkFont) {
        let client_width = edit_client(layout.edit).w;
        if client_width < 5 {
            return;
        }
        let mut width = font.measure(&self.text[..self.caret], false).0;
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

    fn character_at(&self, pointer_x: f32, layout: &InputDialogLayout, font: &ClonkFont) -> usize {
        let control_x =
            pointer_x.floor() as i32 - edit_client(layout.edit).x + self.horizontal_scroll;
        let mut previous_width = 0;
        for (index, character) in self.text.char_indices() {
            let end = index + character.len_utf8();
            let width = font.measure(&self.text[..end], false).0;
            if width - (width - previous_width) / 2 >= control_x {
                return index;
            }
            previous_width = width;
        }
        self.text.len()
    }

    fn set_focus(&mut self, control: InputDialogControl) -> Vec<InputDialogAction> {
        if control == self.focus {
            return Vec::new();
        }
        if self.focus == InputDialogControl::Edit {
            self.selection = None;
        }
        self.focus = control;
        if control == InputDialogControl::Edit {
            self.caret = self.text.len();
            self.selection = (!self.text.is_empty()).then_some((0, self.text.len()));
            self.last_edit_input = Instant::now();
        }
        self.key_pressed = None;
        vec![InputDialogAction::FocusChanged(control)]
    }

    fn advance_focus(&mut self, backwards: bool) -> Vec<InputDialogAction> {
        if self.chat_layout {
            return self.set_focus(InputDialogControl::Edit);
        }
        const WITH_CLOSE: [InputDialogControl; 4] = [
            InputDialogControl::Close,
            InputDialogControl::Edit,
            InputDialogControl::Ok,
            InputDialogControl::Cancel,
        ];
        const WITHOUT_CLOSE: [InputDialogControl; 3] = [
            InputDialogControl::Edit,
            InputDialogControl::Ok,
            InputDialogControl::Cancel,
        ];
        let controls = if self.caption.is_empty() {
            &WITHOUT_CLOSE[..]
        } else {
            &WITH_CLOSE[..]
        };
        let current = controls
            .iter()
            .position(|control| *control == self.focus)
            .unwrap_or(0);
        let next = if backwards {
            current.checked_sub(1).unwrap_or(controls.len() - 1)
        } else {
            (current + 1) % controls.len()
        };
        self.set_focus(controls[next])
    }

    fn focus_button_target(&self) -> Option<ButtonTarget> {
        match self.focus {
            InputDialogControl::Close => Some(ButtonTarget::Close),
            InputDialogControl::Ok => Some(ButtonTarget::Ok),
            InputDialogControl::Cancel => Some(ButtonTarget::Cancel),
            InputDialogControl::Edit => None,
        }
    }

    fn activate_button(&self, target: ButtonTarget) -> Vec<InputDialogAction> {
        match target {
            ButtonTarget::Ok => vec![InputDialogAction::Accepted(self.text.clone())],
            ButtonTarget::Close | ButtonTarget::Cancel => vec![InputDialogAction::Cancelled],
        }
    }

    fn pointer_button_is_down(&self) -> bool {
        self.pointer_pressed.is_some() && self.pointer_pressed == self.hovered
    }

    fn button_state(
        &self,
        target: ButtonTarget,
        keyboard_active: bool,
        mouse_active: bool,
    ) -> ClassicButtonState {
        let keyboard_pressed = keyboard_active
            && self
                .key_pressed
                .is_some_and(|(pressed, _)| pressed == target);
        let pointer_pressed =
            mouse_active && self.pointer_pressed == Some(target) && self.hovered == Some(target);
        ClassicButtonState {
            pressed: keyboard_pressed || pointer_pressed,
            highlighted: (keyboard_active && self.focus == target.control())
                || (mouse_active && self.hovered == Some(target)),
        }
    }
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

fn context_item(
    command: InputDialogContextCommand,
    label: &str,
    tooltip: &str,
) -> InputDialogContextMenuItem {
    InputDialogContextMenuItem {
        command,
        label: label.to_string(),
        tooltip: tooltip.to_string(),
    }
}

fn hit_target(layout: &InputDialogLayout, point: GuiPoint) -> HitTarget {
    if layout
        .close_button
        .is_some_and(|rect| contains(rect, point))
    {
        HitTarget::Close
    } else if contains(layout.edit, point) {
        HitTarget::Edit
    } else if contains(layout.ok_button, point) {
        HitTarget::Ok
    } else if contains(layout.cancel_button, point) {
        HitTarget::Cancel
    } else if layout.caption.is_some_and(|rect| contains(rect, point)) {
        HitTarget::Caption
    } else if contains(layout.message, point) {
        HitTarget::Message
    } else {
        HitTarget::None
    }
}

fn contains(rect: IntRect, point: GuiPoint) -> bool {
    point.x >= rect.x as f32
        && point.x < (rect.x + rect.w) as f32
        && point.y >= rect.y as f32
        && point.y < (rect.y + rect.h) as f32
}

fn truncate_utf8(text: &str, byte_limit: usize) -> &str {
    let mut end = text.len().min(byte_limit);
    while !text.is_char_boundary(end) {
        end -= 1;
    }
    &text[..end]
}

fn previous_boundary(text: &str, position: usize) -> usize {
    text[..position.min(text.len())]
        .char_indices()
        .next_back()
        .map_or(0, |(index, _)| index)
}

fn next_boundary(text: &str, position: usize) -> usize {
    if position >= text.len() {
        return text.len();
    }
    position + text[position..].chars().next().map_or(0, char::len_utf8)
}

fn char_at(text: &str, position: usize) -> char {
    text.get(position..)
        .and_then(|tail| tail.chars().next())
        .unwrap_or('\0')
}

fn is_word_spacer(character: char) -> bool {
    character.is_ascii() && !character.is_ascii_alphanumeric() && character != '_'
}

fn draw_icon(
    surface: &mut Surface,
    rect: IntRect,
    icon: InputDialogIcon,
    icons: &ImageData,
    icons_extended: &ImageData,
    gamma: Option<&GammaRamp>,
) -> Result<()> {
    let (sheet, phase, cell) = match icon {
        InputDialogIcon::None => return Ok(()),
        InputDialogIcon::Standard(phase) => (icons, u32::from(phase), STANDARD_ICON_CELL),
        InputDialogIcon::Extended(phase) => (icons_extended, u32::from(phase), EXTENDED_ICON_CELL),
    };
    let columns = sheet.width() / cell;
    let source_x = (phase % columns) * cell;
    let source_y = (phase / columns) * cell;
    ensure!(
        source_x + cell <= sheet.width() && source_y + cell <= sheet.height(),
        "classic input-dialog icon phase {phase} is outside the {}x{} sheet",
        sheet.width(),
        sheet.height()
    );
    draw_facet_stretch(
        surface,
        sheet,
        (source_x as f32, source_y as f32, cell as f32, cell as f32),
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_support::{endeavour_font_set, load_graphics_png, standard_gamma};
    use clonk_graphics::{Color, PixelFormat};

    fn controller() -> InputDialogController {
        InputDialogController::new(
            "Enter password:",
            "Password",
            InputDialogIcon::LOCKED_FRONTAL,
        )
    }

    /// An IME composition is *shown* at the caret without entering the
    /// committed text: `WindowEvent::Ime::Preedit` is provisional, and only
    /// `Ime::Commit` reaches the existing input path.
    #[test]
    fn a_composition_is_drawn_at_the_caret_without_entering_the_text() {
        let mut controller = controller();
        controller.set_input_text("ab");
        controller.set_composition(Some(ImeComposition {
            text: "\u{304b}".to_owned(),
            cursor: None,
        }));

        assert_eq!(
            controller.text(),
            "ab",
            "a composition never enters the committed text"
        );
        assert_eq!(controller.displayed_text(), "ab\u{304b}");
        assert_eq!(
            controller.displayed_caret(),
            "ab\u{304b}".len(),
            "with no IME cursor the caret sits at the end of the composition"
        );

        // Clearing it leaves the field exactly as it was.
        controller.set_composition(None);
        assert_eq!(controller.displayed_text(), "ab");
        assert_eq!(controller.displayed_caret(), controller.caret());
    }

    /// winit reports the IME's own cursor as a byte range inside the preedit,
    /// and `None` when the IME asks for it to be hidden.
    #[test]
    fn the_ime_cursor_places_the_caret_inside_the_composition() {
        let mut controller = controller();
        controller.set_input_text("ab");
        controller.set_composition(Some(ImeComposition {
            text: "\u{304b}\u{306a}".to_owned(),
            cursor: Some((0, 0)),
        }));

        assert_eq!(
            controller.displayed_caret(),
            "ab".len(),
            "a cursor at the composition's start keeps the caret before it"
        );
        assert_eq!(
            controller.displayed_composition_range(),
            Some(("ab".len(), "ab\u{304b}\u{306a}".len())),
            "the underline spans the whole composition regardless of the cursor"
        );

        // An empty preedit is how an IME cancels: it ends the composition
        // rather than leaving a zero-width one to underline.
        controller.set_composition(Some(ImeComposition::default()));
        assert!(controller.composition().is_none());
        assert_eq!(controller.displayed_composition_range(), None);
    }

    fn point_in(rect: IntRect) -> GuiPoint {
        GuiPoint::new((rect.x + rect.w / 2) as f32, (rect.y + rect.h / 2) as f32)
    }

    fn skin_assets() -> (ImageData, ImageData, ImageData, ImageData) {
        (
            load_graphics_png("GUICaption.png"),
            load_graphics_png("GUIButton.png"),
            load_graphics_png("GUIButtonDown.png"),
            load_graphics_png("GUIButtonHighlight.png"),
        )
    }

    #[test]
    fn full_size_scenario_button_highlight_is_valid() {
        let fonts = endeavour_font_set();
        let (caption, button, button_down, _) = skin_assets();
        let icons = load_graphics_png("GUIIcons.png");
        let icons_extended = load_graphics_png("GUIIcons2.png");
        let highlight = ImageData::new(30, 30, vec![0xff; 30 * 30 * 4]);
        let skin = ClassicGuiSkin::new(&caption, &button, &button_down, Some(&highlight));

        // C4GUI::Resource::Load passes C4FCT_Full for GUIButtonHighlight, so
        // C4FacetExSurface::Load retains the complete override dimensions
        // (src/C4Gui.cpp:1093; src/C4FacetEx.cpp:137-161).
        InputDialogResources::new(
            skin,
            &fonts,
            &fonts.text,
            &icons,
            &icons_extended,
            &highlight,
        )
        .expect("MarsClonk's 30x30 highlight is a valid full-size facet");
    }

    #[test]
    fn exact_regular_geometry_matches_component_aligner() {
        let fonts = endeavour_font_set();
        let layout = controller().layout(1280, 720, &fonts.text);
        assert_eq!(fonts.text.line_height, 22);
        assert_eq!(
            layout.bounds,
            IntRect {
                x: 490,
                y: 265,
                w: 300,
                h: 190
            }
        );
        assert_eq!(
            layout.caption,
            Some(IntRect {
                x: 490,
                y: 265,
                w: 300,
                h: 23
            })
        );
        assert_eq!(
            layout.client,
            IntRect {
                x: 490,
                y: 288,
                w: 300,
                h: 167
            }
        );
        assert_eq!(
            layout.close_button,
            Some(IntRect {
                x: 770,
                y: 269,
                w: 16,
                h: 16
            })
        );
        assert_eq!(
            layout.icon,
            IntRect {
                x: 500,
                y: 298,
                w: 40,
                h: 40
            }
        );
        assert_eq!(
            layout.message,
            IntRect {
                x: 560,
                y: 298,
                w: 220,
                h: 42
            }
        );
        assert_eq!(
            layout.edit,
            IntRect {
                x: 500,
                y: 360,
                w: 280,
                h: 25
            }
        );
        assert_eq!(
            layout.ok_button,
            IntRect {
                x: 515,
                y: 409,
                w: 120,
                h: 32
            }
        );
        assert_eq!(
            layout.cancel_button,
            IntRect {
                x: 645,
                y: 409,
                w: 120,
                h: 32
            }
        );
    }

    #[test]
    fn compact_chat_layout_uses_bottom_third_label_edit_and_no_title_or_buttons() {
        let fonts = endeavour_font_set();
        let mut state = InputDialogController::new_chat("Chat:", "alpha beta")
            .with_chat_tooltip("Enter chat messages here and send them with enter.");
        let layout = state.layout(1280, 720, &fonts.text);
        let edit_height = (fonts.text.line_height + 3).max(MIN_WOOD_BAR_HEIGHT);
        let label_width = fonts.text.measure("Chat:", true).0 + 4;

        assert!(state.is_chat_layout());
        assert!(!state.has_hotkey('C'));
        assert_eq!(state.focused_control(), InputDialogControl::Edit);
        assert_eq!(state.caret(), "alpha beta".len());
        assert_eq!(state.selection(), None);
        assert_eq!(
            layout.bounds,
            IntRect {
                x: 128,
                y: 586,
                w: 1024,
                h: edit_height + 2,
            }
        );
        assert_eq!(layout.caption, None);
        assert_eq!(layout.close_button, None);
        assert_eq!(
            layout.message,
            IntRect {
                x: 129,
                y: 587,
                w: label_width,
                h: edit_height,
            }
        );
        assert_eq!(
            layout.edit,
            IntRect {
                x: 129 + label_width,
                y: 587,
                w: 1024 - label_width - 2,
                h: edit_height,
            }
        );

        let label = point_in(layout.message);
        state.handle_pointer_down(label, &layout, &fonts.text);
        assert!(!state.has_pointer_capture());
        state.handle_pointer_move(
            GuiPoint::new(label.x + 20.0, label.y - 10.0),
            &layout,
            &fonts.text,
        );
        state.handle_pointer_up(label, &layout, &fonts.text);
        assert_eq!(
            state.dialog_offset(),
            (0, 0),
            "the inline label is not a title drag target"
        );
        assert_eq!(
            state
                .tooltip_state_at(state.tooltip_since + TOOLTIP_DELAY, &layout, true)
                .expect("chat label tooltip")
                .text,
            "Enter chat messages here and send them with enter."
        );
        assert!(state
            .handle_key_down(KeyCode::Tab, false, &layout, &fonts.text)
            .is_empty());
        assert_eq!(state.focused_control(), InputDialogControl::Edit);

        let start = GuiPoint::new((layout.edit.x + 5) as f32, point_in(layout.edit).y);
        let end = GuiPoint::new((layout.edit.x + 30) as f32, point_in(layout.edit).y);
        state.handle_pointer_move(start, &layout, &fonts.text);
        assert_eq!(
            state
                .tooltip_state_at(state.tooltip_since + TOOLTIP_DELAY, &layout, true)
                .expect("chat edit tooltip")
                .text,
            "Enter chat messages here and send them with enter."
        );
        state.handle_pointer_down(start, &layout, &fonts.text);
        assert!(state.has_pointer_capture());
        state.handle_pointer_up(end, &layout, &fonts.text);
        assert!(!state.has_pointer_capture());
        assert!(state.selected_text().is_some_and(|text| !text.is_empty()));
        let outcome =
            state.request_context_menu_at(end, &layout, true, &InputDialogContextLabels::default());
        let InputDialogAction::OpenContextMenu(request) = &outcome.actions[0] else {
            panic!("chat edit context menu request");
        };
        assert_eq!(
            request
                .items
                .iter()
                .map(|item| item.command)
                .collect::<Vec<_>>(),
            vec![
                InputDialogContextCommand::Cut,
                InputDialogContextCommand::Copy,
                InputDialogContextCommand::Paste,
                InputDialogContextCommand::Clear,
                InputDialogContextCommand::SelectAll,
            ]
        );
    }

    #[test]
    fn button_visuals_keep_keyboard_and_pointer_activity_independent() {
        let mut state = controller();
        state.focus = InputDialogControl::Ok;
        state.hovered = Some(ButtonTarget::Cancel);
        state.pointer_pressed = Some(ButtonTarget::Cancel);

        assert_eq!(
            state.button_state(ButtonTarget::Ok, true, false),
            ClassicButtonState {
                pressed: false,
                highlighted: true,
            }
        );
        assert_eq!(
            state.button_state(ButtonTarget::Cancel, true, false),
            ClassicButtonState::default(),
            "an inactive shared-mouse path must not leak a retained pointer press"
        );
        assert_eq!(
            state.button_state(ButtonTarget::Cancel, false, true),
            ClassicButtonState {
                pressed: true,
                highlighted: true,
            }
        );

        state.key_pressed = Some((ButtonTarget::Ok, KeyCode::Enter));
        assert_eq!(
            state.button_state(ButtonTarget::Ok, true, false),
            ClassicButtonState {
                pressed: true,
                highlighted: true,
            }
        );
        assert_eq!(
            state.button_state(ButtonTarget::Ok, false, true),
            ClassicButtonState::default(),
            "pointer activity alone must not paint keyboard focus or presses"
        );
    }

    #[test]
    fn compact_chat_multiline_paste_submits_complete_lines_and_keeps_remainder() {
        let fonts = endeavour_font_set();
        let mut state = InputDialogController::new_chat("Chat:", "");
        let layout = state.layout(1280, 720, &fonts.text);
        assert_eq!(
            state.apply_context_command(
                InputDialogContextCommand::Paste,
                Some("one\r\ntwo\nthree"),
                &layout,
                &fonts.text,
            ),
            vec![
                InputDialogAction::TextChanged("one".into()),
                InputDialogAction::SubmittedLine("one".into()),
                InputDialogAction::TextChanged("two".into()),
                InputDialogAction::SubmittedLine("two".into()),
                InputDialogAction::TextChanged("three".into()),
            ]
        );
        assert_eq!(state.text(), "three");
        assert_eq!(state.caret(), "three".len());
        assert_eq!(state.selection(), None);

        let mut trailing = InputDialogController::new_chat("Chat:", "");
        let layout = trailing.layout(1280, 720, &fonts.text);
        assert_eq!(
            trailing.apply_context_command(
                InputDialogContextCommand::Paste,
                Some("final\n"),
                &layout,
                &fonts.text,
            ),
            vec![
                InputDialogAction::TextChanged("final".into()),
                InputDialogAction::Accepted("final".into()),
            ]
        );
    }

    #[test]
    fn wrapping_grows_the_dialog_and_preferred_placement_uses_cpp_offset() {
        let fonts = endeavour_font_set();
        let state = InputDialogController::new(
            "Please enter a deliberately long message which must wrap over several lines in the narrow classic input dialog.",
            "Comment",
            InputDialogIcon::COMMENT,
        )
        .with_placement(InputDialogPlacement::Preferred { x: 100, y: 50 });
        let layout = state.layout(1280, 720, &fonts.text);
        assert_eq!((layout.bounds.x, layout.bounds.y), (130, 80));
        assert!(layout.message_text.contains('\n'));
        assert_eq!(
            layout.bounds.h,
            fonts.text.measure(&layout.message_text, true).1 + DIALOG_VERTICAL_ROOM
        );
    }

    #[test]
    fn set_input_text_selects_all_and_max_includes_the_nul_byte() {
        let fonts = endeavour_font_set();
        let mut state = controller().with_max_text(6).with_input_text("abcdefghi");
        assert_eq!(state.text(), "abcde");
        assert_eq!(state.selection(), Some((0, 5)));
        let layout = state.layout(1280, 720, &fonts.text);
        assert_eq!(
            state.handle_text_input("Z", &layout, &fonts.text),
            vec![InputDialogAction::TextChanged("Z".into())]
        );
        assert_eq!(state.text(), "Z");
        assert!(state
            .handle_text_input("\n\t", &layout, &fonts.text)
            .is_empty());
        state.handle_text_input("|123456", &layout, &fonts.text);
        assert_eq!(state.text(), "Z\u{a6}12");
        assert_eq!(state.text().len(), 5);
    }

    #[test]
    fn edit_navigation_focus_and_button_release_match_control_priority() {
        let fonts = endeavour_font_set();
        let mut state = controller().with_input_text("secret");
        let layout = state.layout(1280, 720, &fonts.text);
        assert_eq!(
            state.handle_key_down(KeyCode::Enter, false, &layout, &fonts.text),
            vec![InputDialogAction::Accepted("secret".into())]
        );
        assert_eq!(
            state.handle_key_down(KeyCode::Tab, false, &layout, &fonts.text),
            vec![InputDialogAction::FocusChanged(InputDialogControl::Ok)]
        );
        assert_eq!(state.selection(), None);
        assert!(state
            .handle_key_down(KeyCode::Space, false, &layout, &fonts.text)
            .is_empty());
        assert_eq!(state.take_sound_events(), vec![InputDialogSound::ArrowHit]);
        assert!(state.handle_key_up(KeyCode::Enter).is_empty());
        assert_eq!(
            state.handle_key_up(KeyCode::Space),
            vec![InputDialogAction::Accepted("secret".into())]
        );
        assert_eq!(state.take_sound_events(), vec![InputDialogSound::Click]);
        assert_eq!(
            state.handle_key_down(KeyCode::Escape, false, &layout, &fonts.text),
            vec![InputDialogAction::Cancelled]
        );
    }

    #[test]
    fn pointer_edit_selection_buttons_and_title_drag_are_recursive() {
        let fonts = endeavour_font_set();
        let mut state = controller().with_input_text("alpha beta");
        let layout = state.layout(1280, 720, &fonts.text);
        state.handle_pointer_down(point_in(layout.edit), &layout, &fonts.text);
        assert_eq!(state.focused_control(), InputDialogControl::Edit);
        assert!(state.selection().is_some());
        state.handle_pointer_double_click(point_in(layout.edit), &layout, &fonts.text);
        assert!(state.selected_text().is_some());
        state.handle_pointer_up(point_in(layout.edit), &layout, &fonts.text);

        state.handle_pointer_down(point_in(layout.cancel_button), &layout, &fonts.text);
        assert_eq!(state.take_sound_events(), vec![InputDialogSound::ArrowHit]);
        assert_eq!(
            state.handle_pointer_up(point_in(layout.cancel_button), &layout, &fonts.text),
            vec![InputDialogAction::Cancelled]
        );
        assert_eq!(state.take_sound_events(), vec![InputDialogSound::Click]);

        let caption = layout.caption.expect("caption");
        let start = GuiPoint::new((caption.x + 20) as f32, (caption.y + 10) as f32);
        state.handle_pointer_down(start, &layout, &fonts.text);
        state.handle_pointer_move(
            GuiPoint::new(start.x + 17.0, start.y - 9.0),
            &layout,
            &fonts.text,
        );
        assert_eq!(state.dialog_offset(), (17, -9));
    }

    #[test]
    fn gamepad_traversal_and_low_high_buttons_follow_dialog_bindings() {
        let fonts = endeavour_font_set();
        let mut state = controller().with_input_text("pw");
        let layout = state.layout(1280, 720, &fonts.text);
        assert_eq!(
            state.handle_gamepad_low_down(&layout, &fonts.text),
            vec![InputDialogAction::Accepted("pw".into())]
        );
        assert_eq!(
            state.handle_gamepad_direction(true),
            vec![InputDialogAction::FocusChanged(InputDialogControl::Ok)]
        );
        assert!(state
            .handle_gamepad_low_down(&layout, &fonts.text)
            .is_empty());
        assert_eq!(
            state.handle_gamepad_low_up(),
            vec![InputDialogAction::Accepted("pw".into())]
        );
        assert_eq!(
            state.handle_gamepad_high_down(),
            vec![InputDialogAction::Cancelled]
        );
    }

    #[test]
    fn localized_button_hotkeys_and_explicit_release_capture_are_exposed() {
        let fonts = endeavour_font_set();
        let mut state = controller()
            .with_button_labels(InputDialogButtonLabels::new("&Ja", "&Abbrechen"))
            .with_input_text("pw");
        let layout = state.layout(1280, 720, &fonts.text);
        assert!(state.has_hotkey('j'));
        assert!(state.has_hotkey('A'));
        assert!(!state.has_hotkey('c'));
        assert_eq!(
            state.handle_hotkey('j'),
            vec![InputDialogAction::Accepted("pw".into())]
        );
        assert_eq!(state.handle_hotkey('a'), vec![InputDialogAction::Cancelled]);

        let enter = state.route_key_down(KeyCode::Enter, false, &layout, &fonts.text);
        assert!(enter.captured);
        assert!(enter.capture_release);
        assert_eq!(
            enter.actions,
            vec![InputDialogAction::Accepted("pw".into())]
        );
        let tab = state.route_key_down(KeyCode::Tab, false, &layout, &fonts.text);
        assert!(tab.captured);
        assert!(!tab.capture_release);
        let space = state.route_key_down(KeyCode::Space, false, &layout, &fonts.text);
        assert!(space.captured && space.capture_release);
        assert!(!state.route_key_up(KeyCode::Enter).captured);
        let release = state.route_key_up(KeyCode::Space);
        assert!(release.captured);
        assert_eq!(
            release.actions,
            vec![InputDialogAction::Accepted("pw".into())]
        );
    }

    #[test]
    fn edit_context_menu_clipboard_and_multiline_paste_cover_recursive_paths() {
        let fonts = endeavour_font_set();
        let labels = InputDialogContextLabels::default();
        let mut state = controller().with_input_text("alpha beta");
        let layout = state.layout(1280, 720, &fonts.text);
        let outcome = state.request_context_menu_at(point_in(layout.edit), &layout, true, &labels);
        assert!(outcome.captured);
        let InputDialogAction::OpenContextMenu(request) = &outcome.actions[0] else {
            panic!("context-menu request");
        };
        assert_eq!(
            request
                .items
                .iter()
                .map(|item| item.command)
                .collect::<Vec<_>>(),
            vec![
                InputDialogContextCommand::Cut,
                InputDialogContextCommand::Copy,
                InputDialogContextCommand::Paste,
                InputDialogContextCommand::Clear,
            ]
        );
        assert_eq!(request.entries().len(), 4);
        assert_eq!(
            state.apply_context_command(
                InputDialogContextCommand::Copy,
                None,
                &layout,
                &fonts.text,
            ),
            vec![InputDialogAction::ClipboardWrite("alpha beta".into())]
        );
        assert_eq!(
            state
                .apply_context_command(InputDialogContextCommand::Cut, None, &layout, &fonts.text,),
            vec![
                InputDialogAction::ClipboardWrite("alpha beta".into()),
                InputDialogAction::TextChanged(String::new()),
            ]
        );
        assert_eq!(
            state.apply_context_command(
                InputDialogContextCommand::Paste,
                Some("one\ntwo"),
                &layout,
                &fonts.text,
            ),
            vec![
                InputDialogAction::TextChanged("one".into()),
                InputDialogAction::Accepted("one".into()),
            ]
        );
    }

    #[test]
    fn shifted_delete_without_selection_is_consumed_but_does_not_edit() {
        let fonts = endeavour_font_set();
        let mut state = controller().with_input_text("abc");
        let layout = state.layout(1280, 720, &fonts.text);
        state.handle_pointer_down(
            GuiPoint::new(
                (layout.edit.x + layout.edit.w - 5) as f32,
                layout.edit.y as f32 + 5.0,
            ),
            &layout,
            &fonts.text,
        );
        state.handle_pointer_up(point_in(layout.edit), &layout, &fonts.text);
        assert!(state
            .handle_edit_key_down(
                InputDialogEditKey::Backspace,
                InputDialogKeyModifiers {
                    shift: true,
                    control: false,
                },
                &layout,
                &fonts.text,
            )
            .is_empty());
        assert_eq!(state.text(), "abc");
    }

    #[test]
    fn set_input_text_preserves_the_oracle_edit_scroll_offset() {
        let fonts = endeavour_font_set();
        let mut state = controller()
            .with_max_text(1024)
            .with_input_text(&"long text ".repeat(50));
        let layout = state.layout(1280, 720, &fonts.text);
        state.handle_edit_key_down(
            InputDialogEditKey::End,
            InputDialogKeyModifiers::default(),
            &layout,
            &fonts.text,
        );
        let scrolled = state.horizontal_scroll;
        assert!(scrolled > 0);
        state.set_input_text("replacement");
        assert_eq!(state.horizontal_scroll, scrolled);
        assert_eq!(state.selection, Some((0, "replacement".len())));
        state.replace_edit_text("history", &layout, &fonts.text);
        assert_eq!(
            state.horizontal_scroll,
            fonts.text.measure("history", false).0 + fonts.text.measure("\u{a6}", false).0 / 2
                - EDIT_SCROLL_OFFSET
        );
        assert_eq!(state.selection, Some((0, "history".len())));
    }

    #[test]
    fn title_autoscroll_survives_drag_and_tooltips_match_only_oracle_assignments() {
        let fonts = endeavour_font_set();
        let caption = "A deliberately overlong localized input dialog caption that cannot fit";
        let mut state =
            InputDialogController::new("Enter password:", caption, InputDialogIcon::LOCKED_FRONTAL)
                .with_close_tooltip("Schließen");
        let layout = state.layout(1280, 720, &fonts.text);
        let base = Instant::now();
        assert_eq!(state.caption_scroll_offset_at(base, &fonts.text), 0);
        assert_eq!(
            state.caption_scroll_offset_at(
                base + TITLE_SCROLL_DELAY - Duration::from_millis(1),
                &fonts.text,
            ),
            0
        );
        assert_eq!(
            state.caption_scroll_offset_at(base + TITLE_SCROLL_DELAY, &fonts.text),
            1
        );

        let title = layout.caption.expect("caption");
        let drag_start = GuiPoint::new((title.x + 30) as f32, (title.y + 10) as f32);
        state.handle_pointer_down(drag_start, &layout, &fonts.text);
        state.handle_pointer_move(
            GuiPoint::new(drag_start.x + 12.0, drag_start.y + 7.0),
            &layout,
            &fonts.text,
        );
        assert_eq!(
            state.caption_scroll_offset_at(
                base + TITLE_SCROLL_DELAY + Duration::from_millis(1),
                &fonts.text,
            ),
            2,
            "dragging moves only the dialog and must not reset title scrolling"
        );

        state.title_drag = None;
        state.handle_pointer_move(drag_start, &layout, &fonts.text);
        let tooltip_start = state.tooltip_since;
        assert!(state
            .tooltip_state_at(
                tooltip_start + TOOLTIP_DELAY - Duration::from_millis(1),
                &layout,
                true,
            )
            .is_none());
        assert_eq!(
            state
                .tooltip_state_at(tooltip_start + TOOLTIP_DELAY, &layout, true)
                .expect("title tooltip")
                .text,
            caption
        );

        let close = layout.close_button.expect("close");
        state.handle_pointer_move(point_in(close), &layout, &fonts.text);
        let tooltip_start = state.tooltip_since;
        assert_eq!(
            state
                .tooltip_state_at(tooltip_start + TOOLTIP_DELAY, &layout, true)
                .expect("close tooltip")
                .text,
            "Schließen"
        );
        state.handle_pointer_move(point_in(layout.ok_button), &layout, &fonts.text);
        assert!(state
            .tooltip_state_at(state.tooltip_since + TOOLTIP_DELAY, &layout, true)
            .is_none());
        state.handle_pointer_move(point_in(layout.cancel_button), &layout, &fonts.text);
        assert!(state
            .tooltip_state_at(state.tooltip_since + TOOLTIP_DELAY, &layout, true)
            .is_none());
        state.handle_pointer_move(point_in(layout.edit), &layout, &fonts.text);
        assert!(state
            .tooltip_state_at(state.tooltip_since + TOOLTIP_DELAY, &layout, true)
            .is_none());
    }

    #[test]
    fn long_selection_pixels_are_clipped_to_the_cpp_edit_client() {
        let fonts = endeavour_font_set();
        let (caption, button, button_down, highlight) = skin_assets();
        let icons = load_graphics_png("GUIIcons.png");
        let icons_extended = load_graphics_png("GUIIcons2.png");
        let skin = ClassicGuiSkin::new(&caption, &button, &button_down, Some(&highlight));
        let resources = InputDialogResources::new(
            skin,
            &fonts,
            &fonts.text,
            &icons,
            &icons_extended,
            &highlight,
        )
        .expect("classic resources");
        let mut selected = controller()
            .with_max_text(1024)
            .with_input_text(&"Wide selection ".repeat(40));
        let layout = selected.layout(1280, 720, &fonts.text);
        selected.handle_edit_key_down(
            InputDialogEditKey::Home,
            InputDialogKeyModifiers::default(),
            &layout,
            &fonts.text,
        );
        selected.handle_edit_key_down(
            InputDialogEditKey::End,
            InputDialogKeyModifiers {
                shift: true,
                control: false,
            },
            &layout,
            &fonts.text,
        );
        assert!(selected.horizontal_scroll > 0);
        // Mid-scroll leaves the full selection beyond both client edges.
        selected.horizontal_scroll /= 2;
        let mut unselected = selected.clone();
        unselected.selection = None;
        let mut with_selection = Surface::new(1280, 720, PixelFormat::Rgba8888);
        let mut without_selection = Surface::new(1280, 720, PixelFormat::Rgba8888);
        let now = Instant::now();
        selected
            .render_with_cursor_at(&mut with_selection, &resources, false, false, None, now)
            .expect("selected render");
        unselected
            .render_with_cursor_at(&mut without_selection, &resources, false, false, None, now)
            .expect("unselected render");
        let edit = layout.edit;
        let clip = IntRect {
            x: edit.x + 2,
            y: edit.y + 2,
            w: edit.w - 4,
            h: edit.h - 3,
        };
        let mut inside_difference = false;
        let mut difference_x = (u32::MAX, 0_u32);
        for y in 0..720 {
            for x in 0..1280 {
                let selected_pixel = with_selection.get_pixel(x, y);
                let plain_pixel = without_selection.get_pixel(x, y);
                if x >= clip.x as u32
                    && x < (clip.x + clip.w) as u32
                    && y >= clip.y as u32
                    && y < (clip.y + clip.h) as u32
                {
                    if selected_pixel != plain_pixel {
                        inside_difference = true;
                        difference_x.0 = difference_x.0.min(x);
                        difference_x.1 = difference_x.1.max(x);
                    }
                } else {
                    assert_eq!(
                        selected_pixel, plain_pixel,
                        "selection escaped edit clip at ({x},{y})"
                    );
                }
            }
        }
        assert!(
            inside_difference,
            "selection must still render inside the edit"
        );
        assert_eq!(
            difference_x,
            (clip.x as u32, (clip.x + clip.w - 1) as u32),
            "mid-scroll selection must reach both exact horizontal clip edges"
        );
    }

    #[test]
    fn caret_on_off_edit_crop_has_pinned_one_point_five_scale_pixels() {
        let fonts = endeavour_font_set();
        let (caption, button, button_down, highlight) = skin_assets();
        let icons = load_graphics_png("GUIIcons.png");
        let icons_extended = load_graphics_png("GUIIcons2.png");
        let skin = ClassicGuiSkin::new(&caption, &button, &button_down, Some(&highlight));
        let resources = InputDialogResources::new(
            skin,
            &fonts,
            &fonts.text,
            &icons,
            &icons_extended,
            &highlight,
        )
        .expect("classic resources");
        let mut state = controller().with_input_text("secret");
        state.selection = None;
        let mut caret_on = Surface::new(1280, 720, PixelFormat::Rgba8888);
        let mut caret_off = Surface::new(1280, 720, PixelFormat::Rgba8888);
        let now = Instant::now();
        state
            .render_with_cursor_at(&mut caret_on, &resources, true, true, None, now)
            .expect("caret-on render");
        state
            .render_with_cursor_at(&mut caret_off, &resources, true, false, None, now)
            .expect("caret-off render");
        let edit = state.layout(1280, 720, &fonts.text).edit;
        let mut changed = 0_u32;
        let mut bounds = (u32::MAX, u32::MAX, 0_u32, 0_u32);
        let mut hash = 0xcbf2_9ce4_8422_2325_u64;
        for y in edit.y as u32..(edit.y + edit.h) as u32 {
            for x in edit.x as u32..(edit.x + edit.w) as u32 {
                let on = caret_on.get_pixel(x, y).expect("caret-on pixel");
                let off = caret_off.get_pixel(x, y).expect("caret-off pixel");
                if on != off {
                    changed += 1;
                    bounds.0 = bounds.0.min(x);
                    bounds.1 = bounds.1.min(y);
                    bounds.2 = bounds.2.max(x);
                    bounds.3 = bounds.3.max(y);
                    for byte in [on.r, on.g, on.b, on.a, off.r, off.g, off.b, off.a] {
                        hash = (hash ^ u64::from(byte)).wrapping_mul(0x0000_0100_0000_01b3);
                    }
                }
            }
        }
        assert_eq!(
            (changed, bounds, hash),
            // The dialog is rendered on transparent black here; ordered
            // layers now retain source-over alpha instead of forcing A=255.
            (114, (545, 363, 550, 382), 0xe279_9eba_5278_c371)
        );
    }

    #[test]
    fn retained_input_caret_reuses_texture_identity() {
        let fonts = endeavour_font_set();
        let capture = || {
            let mut surface = Surface::new(80, 40, PixelFormat::Rgba8888);
            surface.begin_gpu_scene_capture();
            draw_scaled_caret(
                &mut surface,
                &fonts.text,
                10,
                8,
                IntRect {
                    x: 0,
                    y: 0,
                    w: 80,
                    h: 40,
                },
                None,
            );
            surface
                .take_gpu_scene_capture()
                .expect("capture remains active")
                .into_scene(
                    [80, 40],
                    Color::transparent(),
                    &clonk_graphics::GammaRamp::identity(),
                )
        };

        let first = capture();
        let second = capture();
        assert_eq!(first.commands.len(), 1);
        assert_eq!(first.textures.len(), 1);
        assert_eq!(first.textures[0].id, second.textures[0].id);
    }

    #[test]
    fn real_assets_render_and_missing_classic_assets_fail_fast() {
        let fonts = endeavour_font_set();
        let (caption, button, button_down, highlight) = skin_assets();
        let icons = load_graphics_png("GUIIcons.png");
        let icons_extended = load_graphics_png("GUIIcons2.png");
        let skin = ClassicGuiSkin::new(&caption, &button, &button_down, Some(&highlight));
        let resources = InputDialogResources::new(
            skin,
            &fonts,
            &fonts.text,
            &icons,
            &icons_extended,
            &highlight,
        )
        .expect("classic resources");
        let state = controller().with_input_text("secret");
        let mut surface = Surface::new(1280, 720, PixelFormat::Rgba8888);
        for y in 0..surface.height() {
            for x in 0..surface.width() {
                surface
                    .set_pixel(x, y, Color::opaque(13, 17, 19))
                    .expect("surface coordinate");
            }
        }
        state
            .render_with_cursor(&mut surface, &resources, true, true, Some(standard_gamma()))
            .expect("render input dialog");
        assert_ne!(surface.get_pixel(490, 265), Some(Color::opaque(13, 17, 19)));
        assert_ne!(surface.get_pixel(520, 315), Some(Color::opaque(13, 17, 19)));

        let chat = InputDialogController::new_chat("Chat:", "hello");
        let chat_layout = chat.layout(1280, 720, &fonts.text);
        let mut chat_surface = Surface::new(1280, 720, PixelFormat::Rgba8888);
        for y in 0..chat_surface.height() {
            for x in 0..chat_surface.width() {
                chat_surface
                    .set_pixel(x, y, Color::opaque(13, 17, 19))
                    .expect("chat surface coordinate");
            }
        }
        chat.render_with_cursor(
            &mut chat_surface,
            &resources,
            true,
            true,
            Some(standard_gamma()),
        )
        .expect("render compact chat input dialog");
        assert_ne!(
            chat_surface.get_pixel(
                (chat_layout.message.x + chat_layout.message.w / 2) as u32,
                (chat_layout.message.y + chat_layout.message.h / 2) as u32,
            ),
            Some(Color::opaque(13, 17, 19))
        );
        assert_ne!(
            chat_surface.get_pixel(
                (chat_layout.edit.x + chat_layout.edit.w / 2) as u32,
                (chat_layout.edit.y + chat_layout.edit.h / 2) as u32,
            ),
            Some(Color::opaque(13, 17, 19))
        );
        let invalid_icons = ImageData::new(40, 40, vec![0; 40 * 40 * 4]);
        let error = InputDialogResources::new(
            skin,
            &fonts,
            &fonts.text,
            &invalid_icons,
            &icons_extended,
            &highlight,
        )
        .err()
        .expect("invalid icons must fail");
        assert!(error.to_string().contains("exact 240x360"));
    }
}
