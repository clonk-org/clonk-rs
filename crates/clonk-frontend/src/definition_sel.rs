//! Pixel-faithful frontend state for the classic file-selector family.
//!
//! Directory enumeration deliberately lives outside this module. Callers pass
//! the immediate matching entries in their native order and rebuild the rows
//! after a refresh request. [`FileSelMode::Definitions`] retains the checked
//! multi-selection specialization; [`FileSelMode::Player`] models the
//! single-selection `C4PlayerSelDlg` specialization.

use crate::caption_scroll::{advance_caption_scroll, CaptionScrollState};
use crate::classic_gui::{
    draw_3d_frame, draw_clipped_text, draw_engine_box, draw_facet_stretch, ClassicButtonState,
    IntRect,
};
use crate::{ClonkFontSet, GuiPoint, ImageData, KeyCode, StartupTooltip};
use anyhow::{ensure, Result};
use clonk_graphics::clonk_font::{ClonkFont, TextAlign};
use clonk_graphics::{GammaRamp, PixelFormat, Point, Surface};
use clonk_gui::Rect as GuiRect;
use std::cell::Cell;
use std::time::{Duration, Instant};

const MIN_WIDTH: i32 = 300;
const MAX_WIDTH: i32 = 600;
const MIN_HEIGHT: i32 = 220;
const MAX_HEIGHT: i32 = 500;
const MIN_CAPTION_HEIGHT: i32 = 23;
const BUTTON_WIDTH: i32 = 120;
const BUTTON_HEIGHT: i32 = 32;
const SCROLLBAR_WIDTH: i32 = 16;
const ROW_SPACING: i32 = 1;
const PLAYER_ICON_PHASE: u32 = 9;
const DEFINITION_ICON_PHASE: u32 = 29;
const TITLE_LEFT_INDENT: i32 = 5;
const TITLE_RIGHT_INDENT: i32 = 20;
const TITLE_SCROLL_DELAY: Duration = Duration::from_millis(3000);

/// The two `C4FileSelDlg` specializations exposed by this controller.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FileSelMode {
    /// `C4DefinitionSelDlg`: checked multi-selection with fixed rows.
    Definitions,
    /// `C4PlayerSelDlg`: one selected `*.c4p` path and no checkboxes.
    Player,
}

impl FileSelMode {
    const fn is_multi_selection(self) -> bool {
        matches!(self, Self::Definitions)
    }

    const fn icon_phase(self) -> u32 {
        match self {
            Self::Definitions => DEFINITION_ICON_PHASE,
            Self::Player => PLAYER_ICON_PHASE,
        }
    }

    const fn title(self) -> &'static str {
        match self {
            Self::Definitions => "Select Object Definitions",
            Self::Player => "Select player...",
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DefinitionSelEntry {
    pub full_path: String,
    pub filename: String,
}

impl DefinitionSelEntry {
    pub fn new(full_path: impl Into<String>, filename: impl Into<String>) -> Self {
        Self {
            full_path: full_path.into(),
            filename: filename.into(),
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DefinitionSelRow {
    full_path: String,
    filename: String,
    label: String,
    fixed: bool,
    checked: bool,
}

impl DefinitionSelRow {
    pub fn full_path(&self) -> &str {
        &self.full_path
    }
    pub fn filename(&self) -> &str {
        &self.filename
    }
    pub fn label(&self) -> &str {
        &self.label
    }
    pub const fn is_fixed(&self) -> bool {
        self.fixed
    }
    pub const fn is_checked(&self) -> bool {
        self.checked
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DefinitionSelControl {
    Close,
    FileList,
    RowCheckbox(usize),
    Ok,
    Cancel,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DefinitionSelKey {
    Enter,
    Escape,
    Space,
    Tab,
    Up,
    Down,
    Left,
    Right,
    PageUp,
    PageDown,
    Home,
    End,
    Refresh,
}

impl From<KeyCode> for DefinitionSelKey {
    fn from(value: KeyCode) -> Self {
        match value {
            KeyCode::Enter => Self::Enter,
            KeyCode::Escape => Self::Escape,
            KeyCode::Space => Self::Space,
            KeyCode::Tab => Self::Tab,
            KeyCode::Up => Self::Up,
            KeyCode::Down => Self::Down,
            KeyCode::Left => Self::Left,
            KeyCode::Right => Self::Right,
            KeyCode::Home => Self::Home,
            KeyCode::End => Self::End,
            KeyCode::PageUp => Self::PageUp,
            KeyCode::PageDown => Self::PageDown,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum DefinitionSelAction {
    FocusChanged(DefinitionSelControl),
    SelectionChanged(Option<usize>),
    CheckedChanged { index: usize, checked: bool },
    RefreshRequested,
    PleaseSelectFile,
    Accepted(Vec<String>),
    Cancelled,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DefinitionSelSound {
    Command,
    ArrowHit,
    Click,
}

#[derive(Clone, Copy)]
pub struct DefinitionSelResources<'a> {
    pub skin: crate::classic_gui::ClassicGuiSkin<'a>,
    pub fonts: &'a ClonkFontSet,
    pub icons: &'a ImageData,
    pub checkbox: &'a ImageData,
    pub scroll: &'a ImageData,
    pub button_highlight: &'a ImageData,
}

impl DefinitionSelResources<'_> {
    /// Validates the resource set used by the definition specialization.
    /// Kept as the compatibility entry point for existing callers.
    pub fn validate(self) -> Result<()> {
        self.validate_for_mode(FileSelMode::Definitions)
    }

    /// Validates only the sheets consumed by the selected specialization.
    /// `C4PlayerSelDlg` has no checkbox controls and therefore does not depend
    /// on a usable `GUICheckbox` strip.
    pub fn validate_for_mode(self, mode: FileSelMode) -> Result<()> {
        self.skin.validate_message_dialog_assets()?;
        ensure!(
            self.icons.width() >= 40
                && self.icons.height() >= 40
                && self.icons.width().is_multiple_of(40)
                && self.icons.height().is_multiple_of(40),
            "GUIIcons.png must be a grid of 40x40 classic icons, got {}x{}",
            self.icons.width(),
            self.icons.height()
        );
        let icon_columns = self.icons.width() / 40;
        ensure!(
            34 / icon_columns < self.icons.height() / 40,
            "GUIIcons.png does not contain required file-selector and close phases"
        );
        if mode.is_multi_selection() {
            ensure!(
                self.checkbox.height() > 0
                    && self.checkbox.width() >= self.checkbox.height().saturating_mul(4),
                "GUICheckbox.png must contain all four enabled/disabled phases, got {}x{}",
                self.checkbox.width(),
                self.checkbox.height()
            );
        }
        ensure!(
            self.scroll.width() >= 32 && self.scroll.height() >= 48,
            "GUIScroll.png must contain the 32x48 classic scrollbar facets, got {}x{}",
            self.scroll.width(),
            self.scroll.height()
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

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DefinitionSelLayout {
    pub bounds: IntRect,
    pub caption: IntRect,
    pub client: IntRect,
    pub close_button: IntRect,
    pub file_list: IntRect,
    pub list_client: IntRect,
    pub list_scrollbar: IntRect,
    pub preview: IntRect,
    pub preview_client: IntRect,
    pub ok_button: IntRect,
    pub cancel_button: IntRect,
    pub row_height: i32,
    pub row_pitch: i32,
}

impl DefinitionSelLayout {
    pub fn row_rect(&self, index: usize, scroll_y: i32) -> IntRect {
        IntRect::new(
            self.list_client.x,
            self.list_client.y + index as i32 * self.row_pitch - scroll_y,
            self.list_client.w,
            self.row_height,
        )
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum ButtonTarget {
    Close,
    Ok,
    Cancel,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum HitTarget {
    Close,
    Ok,
    Cancel,
    Caption,
    Checkbox(usize),
    Row(usize),
    ListBlank,
    Scrollbar,
    None,
}

#[derive(Clone, Copy, Debug)]
struct TitleDrag {
    pointer_x: f32,
    pointer_y: f32,
    offset_x: i32,
    offset_y: i32,
}

#[derive(Clone, Debug)]
pub struct DefinitionSelController {
    mode: FileSelMode,
    root_path: String,
    fixed_selection: Vec<String>,
    rows: Vec<DefinitionSelRow>,
    selected: Option<usize>,
    focus: DefinitionSelControl,
    scroll_y: i32,
    scroll_pin: i32,
    dialog_offset: (i32, i32),
    caption_scroll: Cell<CaptionScrollState>,
    pointer: Option<GuiPoint>,
    pointer_pressed: Option<ButtonTarget>,
    key_pressed: Option<ButtonTarget>,
    key_checkbox_pressed: Option<usize>,
    title_drag: Option<TitleDrag>,
    scrollbar_dragging: bool,
    scrollbar_arrow: i8,
    sound_events: Vec<DefinitionSelSound>,
}

impl DefinitionSelController {
    /// Constructs the checked, multi-selection `C4DefinitionSelDlg` variant.
    pub fn new(
        root_path: impl Into<String>,
        fixed_selection: Vec<String>,
        entries: Vec<DefinitionSelEntry>,
    ) -> Self {
        Self::with_mode(
            FileSelMode::Definitions,
            root_path,
            fixed_selection,
            entries,
        )
    }

    /// Constructs the single-selection `C4PlayerSelDlg` variant.
    pub fn new_player(root_path: impl Into<String>, entries: Vec<DefinitionSelEntry>) -> Self {
        Self::with_mode(FileSelMode::Player, root_path, Vec::new(), entries)
    }

    fn with_mode(
        mode: FileSelMode,
        root_path: impl Into<String>,
        fixed_selection: Vec<String>,
        entries: Vec<DefinitionSelEntry>,
    ) -> Self {
        let rows = build_rows(&entries, &fixed_selection, true);
        Self {
            mode,
            root_path: root_path.into(),
            fixed_selection,
            rows,
            selected: None,
            focus: DefinitionSelControl::FileList,
            scroll_y: 0,
            scroll_pin: 0,
            dialog_offset: (0, 0),
            caption_scroll: Cell::new(CaptionScrollState::default()),
            pointer: None,
            pointer_pressed: None,
            key_pressed: None,
            key_checkbox_pressed: None,
            title_drag: None,
            scrollbar_dragging: false,
            scrollbar_arrow: 0,
            sound_events: Vec::new(),
        }
    }

    pub const fn mode(&self) -> FileSelMode {
        self.mode
    }

    pub const fn is_multi_selection(&self) -> bool {
        self.mode.is_multi_selection()
    }

    pub fn root_path(&self) -> &str {
        &self.root_path
    }
    pub fn rows(&self) -> &[DefinitionSelRow] {
        &self.rows
    }
    pub const fn selected_index(&self) -> Option<usize> {
        self.selected
    }
    pub fn selected_row(&self) -> Option<&DefinitionSelRow> {
        self.selected.and_then(|index| self.rows.get(index))
    }
    pub fn selected_full_path(&self) -> Option<&str> {
        self.selected_row().map(DefinitionSelRow::full_path)
    }
    pub const fn focus(&self) -> DefinitionSelControl {
        self.focus
    }
    pub const fn scroll_y(&self) -> i32 {
        self.scroll_y
    }
    pub const fn dialog_offset(&self) -> (i32, i32) {
        self.dialog_offset
    }
    pub const fn pointer_position(&self) -> Option<GuiPoint> {
        self.pointer
    }

    pub fn caption(&self) -> String {
        let title = self.mode.title();
        if self.mode == FileSelMode::Player || self.root_path.is_empty() {
            title.to_owned()
        } else {
            format!("{title} [{}]", self.root_path)
        }
    }

    /// Returns the native tooltip target at `point`, without applying the
    /// screen-global `CMouse` hover delay. The close icon wins its overlap
    /// with the enclosing wooden caption, matching top-down element routing.
    pub fn tooltip_at(
        &self,
        point: GuiPoint,
        layout: &DefinitionSelLayout,
    ) -> Option<StartupTooltip> {
        let routed_pointer = self.pointer?;
        if routed_pointer.x as i32 != point.x as i32 || routed_pointer.y as i32 != point.y as i32 {
            return None;
        }
        match self.hit_target(point, layout) {
            HitTarget::Close => Some(StartupTooltip::resource("IDS_MNU_CLOSE")),
            HitTarget::Caption => Some(StartupTooltip::text(self.caption())),
            HitTarget::Ok
            | HitTarget::Cancel
            | HitTarget::Checkbox(_)
            | HitTarget::Row(_)
            | HitTarget::ListBlank
            | HitTarget::Scrollbar
            | HitTarget::None => None,
        }
    }

    pub fn accepted_selection(&self) -> Vec<String> {
        if self.mode == FileSelMode::Player {
            return self
                .selected_row()
                .map(|row| vec![row.full_path.clone()])
                .unwrap_or_default();
        }
        let mut result = self.fixed_selection.clone();
        for row in &self.rows {
            if row.checked && !result.iter().any(|value| value == &row.filename) {
                result.push(row.filename.clone());
            }
        }
        result
    }

    pub fn rebuild_rows_after_refresh(&mut self, entries: Vec<DefinitionSelEntry>) {
        self.rows = build_rows(&entries, &self.fixed_selection, false);
        self.selected = None;
        if matches!(self.focus, DefinitionSelControl::RowCheckbox(_)) {
            self.focus = DefinitionSelControl::FileList;
        }
        self.scroll_y = 0;
        self.scroll_pin = 0;
        self.pointer_pressed = None;
        self.key_pressed = None;
        self.key_checkbox_pressed = None;
        self.scrollbar_dragging = false;
        self.scrollbar_arrow = 0;
    }

    pub fn take_sound_events(&mut self) -> Vec<DefinitionSelSound> {
        std::mem::take(&mut self.sound_events)
    }

    pub fn layout(
        &self,
        screen_width: i32,
        screen_height: i32,
        font: &ClonkFont,
    ) -> DefinitionSelLayout {
        definition_sel_layout(
            screen_width,
            screen_height,
            font.line_height,
            self.dialog_offset,
        )
    }

    pub fn handle_key_down(
        &mut self,
        key: DefinitionSelKey,
        backwards: bool,
        layout: &DefinitionSelLayout,
    ) -> Vec<DefinitionSelAction> {
        let mut actions = Vec::new();
        match key {
            DefinitionSelKey::Enter => self.try_accept(&mut actions),
            DefinitionSelKey::Escape => actions.push(DefinitionSelAction::Cancelled),
            DefinitionSelKey::Refresh => actions.push(DefinitionSelAction::RefreshRequested),
            DefinitionSelKey::Tab => self.advance_focus(backwards, layout, &mut actions),
            DefinitionSelKey::Space
                if self.focus == DefinitionSelControl::FileList
                    && self.mode.is_multi_selection() =>
            {
                if let Some(index) = self.selected {
                    self.toggle_row(index, &mut actions);
                }
            }
            DefinitionSelKey::Space => {
                if let DefinitionSelControl::RowCheckbox(index) = self.focus {
                    if self.rows.get(index).is_some_and(|row| !row.fixed) {
                        self.key_checkbox_pressed = Some(index);
                        self.sound_events.push(DefinitionSelSound::ArrowHit);
                    }
                } else if self.key_pressed.is_none() {
                    self.key_pressed = button_for_control(self.focus);
                    if self.key_pressed.is_some() {
                        self.sound_events.push(DefinitionSelSound::ArrowHit);
                    }
                }
            }
            DefinitionSelKey::Up if self.focus == DefinitionSelControl::FileList => {
                let next = self.selected.map_or_else(
                    || self.rows.len().checked_sub(1),
                    |index| index.checked_sub(1).or(Some(index)),
                );
                self.set_selection(next, true, layout, &mut actions);
            }
            DefinitionSelKey::Down if self.focus == DefinitionSelControl::FileList => {
                let next = self.selected.map_or_else(
                    || (!self.rows.is_empty()).then_some(0),
                    |index| Some((index + 1).min(self.rows.len().saturating_sub(1))),
                );
                self.set_selection(next, true, layout, &mut actions);
            }
            DefinitionSelKey::Home if self.focus == DefinitionSelControl::FileList => {
                self.set_selection(
                    (!self.rows.is_empty()).then_some(0),
                    true,
                    layout,
                    &mut actions,
                );
            }
            DefinitionSelKey::End if self.focus == DefinitionSelControl::FileList => {
                self.set_selection(self.rows.len().checked_sub(1), true, layout, &mut actions);
            }
            DefinitionSelKey::PageUp if self.focus == DefinitionSelControl::FileList => {
                self.page_selection(false, layout, &mut actions);
            }
            DefinitionSelKey::PageDown if self.focus == DefinitionSelControl::FileList => {
                self.page_selection(true, layout, &mut actions);
            }
            DefinitionSelKey::Left
            | DefinitionSelKey::Right
            | DefinitionSelKey::Up
            | DefinitionSelKey::Down
            | DefinitionSelKey::Home
            | DefinitionSelKey::End
            | DefinitionSelKey::PageUp
            | DefinitionSelKey::PageDown => {}
        }
        actions
    }

    pub fn handle_key_up(&mut self, key: DefinitionSelKey) -> Vec<DefinitionSelAction> {
        if key != DefinitionSelKey::Space {
            return Vec::new();
        }
        if let Some(index) = self.key_checkbox_pressed.take() {
            if self.focus == DefinitionSelControl::RowCheckbox(index) {
                let mut actions = Vec::new();
                self.sound_events.push(DefinitionSelSound::Click);
                self.toggle_row(index, &mut actions);
                return actions;
            }
        }
        let Some(target) = self.key_pressed.take() else {
            return Vec::new();
        };
        if button_for_control(self.focus) != Some(target) {
            return Vec::new();
        }
        self.sound_events.push(DefinitionSelSound::Click);
        self.activate_button(target)
    }

    pub fn handle_hotkey(&mut self, character: char) -> Vec<DefinitionSelAction> {
        if character.eq_ignore_ascii_case(&'o') {
            let mut actions = Vec::new();
            self.try_accept(&mut actions);
            actions
        } else {
            Vec::new()
        }
    }

    /// Primary gamepad action. On the list this matches `ListBox::KeyActivate`:
    /// definitions toggle their check, while a player selection accepts.
    /// Buttons follow their ordinary down/up interaction.
    pub fn handle_gamepad_low_down(
        &mut self,
        layout: &DefinitionSelLayout,
    ) -> Vec<DefinitionSelAction> {
        if self.focus == DefinitionSelControl::FileList {
            let mut actions = Vec::new();
            if self.mode.is_multi_selection() {
                if let Some(index) = self.selected {
                    self.toggle_row(index, &mut actions);
                } else {
                    self.try_accept(&mut actions);
                }
            } else {
                self.try_accept(&mut actions);
            }
            actions
        } else {
            self.handle_key_down(DefinitionSelKey::Space, false, layout)
        }
    }

    pub fn handle_gamepad_low_up(&mut self) -> Vec<DefinitionSelAction> {
        self.handle_key_up(DefinitionSelKey::Space)
    }

    pub fn handle_gamepad_high_down(&mut self) -> Vec<DefinitionSelAction> {
        vec![DefinitionSelAction::Cancelled]
    }

    pub fn handle_gamepad_up(&mut self, layout: &DefinitionSelLayout) -> Vec<DefinitionSelAction> {
        self.handle_key_down(DefinitionSelKey::Up, false, layout)
    }

    pub fn handle_gamepad_down(
        &mut self,
        layout: &DefinitionSelLayout,
    ) -> Vec<DefinitionSelAction> {
        self.handle_key_down(DefinitionSelKey::Down, false, layout)
    }

    pub fn handle_gamepad_left(
        &mut self,
        layout: &DefinitionSelLayout,
    ) -> Vec<DefinitionSelAction> {
        self.handle_key_down(DefinitionSelKey::Tab, true, layout)
    }

    pub fn handle_gamepad_right(
        &mut self,
        layout: &DefinitionSelLayout,
    ) -> Vec<DefinitionSelAction> {
        self.handle_key_down(DefinitionSelKey::Tab, false, layout)
    }

    pub fn handle_pointer_move(
        &mut self,
        point: GuiPoint,
        layout: &DefinitionSelLayout,
    ) -> Vec<DefinitionSelAction> {
        let was_button_down = self.pointer_button_is_down(layout);
        let had_arrow_down = self.scrollbar_arrow != 0;
        self.pointer = Some(point);

        if let Some(drag) = self.title_drag {
            self.dialog_offset = (
                drag.offset_x + (point.x - drag.pointer_x) as i32,
                drag.offset_y + (point.y - drag.pointer_y) as i32,
            );
        }
        if self.scrollbar_dragging {
            self.set_scroll_from_pointer(point, layout);
        } else if self.scrollbar_arrow != 0 {
            self.scrollbar_arrow = scrollbar_arrow_at(point, layout);
        }

        if was_button_down != self.pointer_button_is_down(layout) {
            self.sound_events.push(DefinitionSelSound::ArrowHit);
        }
        if had_arrow_down != (self.scrollbar_arrow != 0) {
            self.sound_events.push(DefinitionSelSound::ArrowHit);
        }
        Vec::new()
    }

    pub fn handle_pointer_down(
        &mut self,
        point: GuiPoint,
        layout: &DefinitionSelLayout,
    ) -> Vec<DefinitionSelAction> {
        self.pointer = Some(point);
        let mut actions = Vec::new();
        match self.hit_target(point, layout) {
            HitTarget::Close => {
                self.press_pointer_button(ButtonTarget::Close, layout, &mut actions)
            }
            HitTarget::Ok => self.press_pointer_button(ButtonTarget::Ok, layout, &mut actions),
            HitTarget::Cancel => {
                self.press_pointer_button(ButtonTarget::Cancel, layout, &mut actions)
            }
            HitTarget::Caption => {
                self.title_drag = Some(TitleDrag {
                    pointer_x: point.x,
                    pointer_y: point.y,
                    offset_x: self.dialog_offset.0,
                    offset_y: self.dialog_offset.1,
                });
            }
            HitTarget::Checkbox(index) | HitTarget::Row(index) => {
                self.set_focus(DefinitionSelControl::FileList, true, layout, &mut actions);
                self.set_selection(Some(index), true, layout, &mut actions);
            }
            HitTarget::ListBlank => {
                self.set_focus(DefinitionSelControl::FileList, true, layout, &mut actions);
                self.set_selection(None, true, layout, &mut actions);
            }
            HitTarget::Scrollbar => self.begin_scrollbar_pointer(point, layout),
            HitTarget::None => {}
        }
        actions
    }

    pub fn handle_pointer_up(
        &mut self,
        point: GuiPoint,
        layout: &DefinitionSelLayout,
    ) -> Vec<DefinitionSelAction> {
        let was_button_down = self.pointer_button_is_down(layout);
        self.pointer = Some(point);
        let released = self.hit_target(point, layout);
        let pressed = self.pointer_pressed.take();
        self.title_drag = None;
        self.scrollbar_dragging = false;
        if self.scrollbar_arrow != 0 {
            self.scrollbar_arrow = 0;
            self.sound_events.push(DefinitionSelSound::ArrowHit);
        }

        if let Some(target) = pressed {
            if released == hit_for_button(target) && was_button_down {
                self.sound_events.push(DefinitionSelSound::Click);
                return self.activate_button(target);
            }
            if was_button_down {
                self.sound_events.push(DefinitionSelSound::ArrowHit);
            }
        }

        let mut actions = Vec::new();
        if let HitTarget::Checkbox(index) = released {
            self.toggle_row(index, &mut actions);
        }
        actions
    }

    pub fn handle_pointer_double_click(
        &mut self,
        point: GuiPoint,
        layout: &DefinitionSelLayout,
    ) -> Vec<DefinitionSelAction> {
        self.pointer = Some(point);
        let mut actions = Vec::new();
        match self.hit_target(point, layout) {
            HitTarget::Checkbox(index) | HitTarget::Row(index) => {
                self.set_focus(DefinitionSelControl::FileList, true, layout, &mut actions);
                self.set_selection(Some(index), true, layout, &mut actions);
                if self.mode.is_multi_selection() {
                    self.toggle_row(index, &mut actions);
                } else {
                    self.try_accept(&mut actions);
                }
            }
            HitTarget::ListBlank => {
                self.set_focus(DefinitionSelControl::FileList, true, layout, &mut actions);
                self.set_selection(None, true, layout, &mut actions);
            }
            _ => {}
        }
        actions
    }

    pub fn handle_wheel(
        &mut self,
        point: GuiPoint,
        delta: i32,
        layout: &DefinitionSelLayout,
    ) -> Vec<DefinitionSelAction> {
        self.pointer = Some(point);
        if rect_contains(layout.list_client, point) {
            self.scroll_by(-delta, layout);
        }
        Vec::new()
    }

    pub fn handle_touch_start(
        &mut self,
        point: GuiPoint,
        layout: &DefinitionSelLayout,
    ) -> Vec<DefinitionSelAction> {
        self.handle_pointer_down(point, layout)
    }

    pub fn handle_touch_move(
        &mut self,
        point: GuiPoint,
        layout: &DefinitionSelLayout,
    ) -> Vec<DefinitionSelAction> {
        self.handle_pointer_move(point, layout)
    }

    pub fn handle_touch_end(
        &mut self,
        point: GuiPoint,
        layout: &DefinitionSelLayout,
    ) -> Vec<DefinitionSelAction> {
        self.handle_pointer_up(point, layout)
    }

    pub fn handle_touch_cancel(&mut self) {
        self.cancel_interaction();
    }

    pub fn scroll_by(&mut self, amount: i32, layout: &DefinitionSelLayout) {
        let maximum = self.max_scroll(layout);
        self.scroll_y = self.scroll_y.saturating_add(amount).clamp(0, maximum);
        self.sync_pin_from_scroll(layout);
    }

    /// Advances a held classic scrollbar arrow by one thumb pixel, matching
    /// `C4GUI::ScrollBar::DrawElement`'s per-frame behavior.
    pub fn tick_scrollbar(&mut self, layout: &DefinitionSelLayout) {
        if self.scrollbar_arrow == 0 || self.max_scroll(layout) == 0 {
            return;
        }
        let max_pin = max_scroll_pin(layout);
        self.scroll_pin = (self.scroll_pin + i32::from(self.scrollbar_arrow)).clamp(0, max_pin);
        self.scroll_y = self.max_scroll(layout) * self.scroll_pin / max_pin.max(1);
    }

    pub fn pointer_left(&mut self, layout: &DefinitionSelLayout) {
        if self.pointer_button_is_down(layout) || self.scrollbar_arrow != 0 {
            self.sound_events.push(DefinitionSelSound::ArrowHit);
        }
        self.pointer = None;
        self.scrollbar_arrow = 0;
    }

    pub fn cancel_interaction(&mut self) {
        self.pointer = None;
        self.pointer_pressed = None;
        self.key_pressed = None;
        self.key_checkbox_pressed = None;
        self.title_drag = None;
        self.scrollbar_dragging = false;
        self.scrollbar_arrow = 0;
        self.sound_events.clear();
    }

    fn try_accept(&self, actions: &mut Vec<DefinitionSelAction>) {
        if self.selected.is_some() {
            actions.push(DefinitionSelAction::Accepted(self.accepted_selection()));
        } else {
            actions.push(DefinitionSelAction::PleaseSelectFile);
        }
    }

    fn activate_button(&self, target: ButtonTarget) -> Vec<DefinitionSelAction> {
        match target {
            ButtonTarget::Close | ButtonTarget::Cancel => vec![DefinitionSelAction::Cancelled],
            ButtonTarget::Ok => {
                let mut actions = Vec::new();
                self.try_accept(&mut actions);
                actions
            }
        }
    }

    fn advance_focus(
        &mut self,
        backwards: bool,
        layout: &DefinitionSelLayout,
        actions: &mut Vec<DefinitionSelAction>,
    ) {
        let mut order = vec![DefinitionSelControl::Close, DefinitionSelControl::FileList];
        if let Some(index) = self.selected.filter(|index| {
            self.mode.is_multi_selection() && self.rows.get(*index).is_some_and(|row| !row.fixed)
        }) {
            order.push(DefinitionSelControl::RowCheckbox(index));
        }
        order.extend([DefinitionSelControl::Ok, DefinitionSelControl::Cancel]);
        let current = order
            .iter()
            .position(|control| *control == self.focus)
            .unwrap_or(0);
        let next = if backwards {
            (current + order.len() - 1) % order.len()
        } else {
            (current + 1) % order.len()
        };
        self.set_focus(order[next], false, layout, actions);
    }

    fn set_focus(
        &mut self,
        focus: DefinitionSelControl,
        by_mouse: bool,
        layout: &DefinitionSelLayout,
        actions: &mut Vec<DefinitionSelAction>,
    ) {
        if self.focus == focus {
            return;
        }
        self.focus = focus;
        self.key_pressed = None;
        self.key_checkbox_pressed = None;
        actions.push(DefinitionSelAction::FocusChanged(focus));
        if focus == DefinitionSelControl::FileList
            && !by_mouse
            && self.selected.is_none()
            && !self.rows.is_empty()
        {
            self.set_selection(Some(0), false, layout, actions);
        }
    }

    fn set_selection(
        &mut self,
        selection: Option<usize>,
        by_user: bool,
        layout: &DefinitionSelLayout,
        actions: &mut Vec<DefinitionSelAction>,
    ) {
        let selection = selection.filter(|index| *index < self.rows.len());
        if self.selected == selection {
            return;
        }
        self.selected = selection;
        if by_user && selection.is_some() {
            self.sound_events.push(DefinitionSelSound::Command);
        }
        actions.push(DefinitionSelAction::SelectionChanged(selection));
        if let Some(index) = selection {
            self.ensure_row_visible(index, layout);
        }
    }

    fn toggle_row(&mut self, index: usize, actions: &mut Vec<DefinitionSelAction>) {
        if !self.mode.is_multi_selection() {
            return;
        }
        let Some(row) = self.rows.get_mut(index) else {
            return;
        };
        if row.fixed {
            return;
        }
        row.checked = !row.checked;
        self.sound_events.push(DefinitionSelSound::ArrowHit);
        actions.push(DefinitionSelAction::CheckedChanged {
            index,
            checked: row.checked,
        });
    }

    fn page_selection(
        &mut self,
        down: bool,
        layout: &DefinitionSelLayout,
        actions: &mut Vec<DefinitionSelAction>,
    ) {
        if self.rows.is_empty() {
            return;
        }
        let mut next = self
            .selected
            .unwrap_or(if down { 0 } else { self.rows.len() - 1 });
        if down {
            if next + 1 < self.rows.len() {
                next += 1;
                if self.row_is_fully_visible(next, layout) {
                    while next + 1 < self.rows.len() && self.row_is_fully_visible(next + 1, layout)
                    {
                        next += 1;
                    }
                } else {
                    self.scroll_by(layout.list_client.h, layout);
                    next = self.rows.len() - 1;
                    while next > 0 && !self.row_is_fully_visible(next, layout) {
                        next -= 1;
                    }
                }
            }
        } else if next > 0 {
            next -= 1;
            if self.row_is_fully_visible(next, layout) {
                while next > 0 && self.row_is_fully_visible(next - 1, layout) {
                    next -= 1;
                }
            } else {
                self.scroll_by(-layout.list_client.h, layout);
                next = 0;
                while next + 1 < self.rows.len() && !self.row_is_fully_visible(next, layout) {
                    next += 1;
                }
            }
        }
        self.set_selection(Some(next), true, layout, actions);
    }

    fn row_is_fully_visible(&self, index: usize, layout: &DefinitionSelLayout) -> bool {
        let top = index as i32 * layout.row_pitch;
        self.scroll_y <= top && self.scroll_y + layout.list_client.h >= top + layout.row_height
    }

    fn ensure_row_visible(&mut self, index: usize, layout: &DefinitionSelLayout) {
        let top = index as i32 * layout.row_pitch;
        let bottom = top + layout.row_height;
        if self.scroll_y > top {
            self.scroll_y = top;
        } else if self.scroll_y + layout.list_client.h < bottom {
            self.scroll_y = bottom - layout.list_client.h;
        }
        self.scroll_y = self.scroll_y.clamp(0, self.max_scroll(layout));
        self.sync_pin_from_scroll(layout);
    }

    fn content_height(&self, layout: &DefinitionSelLayout) -> i32 {
        self.rows.len() as i32 * layout.row_pitch - i32::from(!self.rows.is_empty()) * ROW_SPACING
    }

    fn max_scroll(&self, layout: &DefinitionSelLayout) -> i32 {
        (self.content_height(layout) - layout.list_client.h).max(0)
    }

    fn sync_pin_from_scroll(&mut self, layout: &DefinitionSelLayout) {
        let max_scroll = self.max_scroll(layout);
        self.scroll_pin = if max_scroll == 0 {
            0
        } else {
            max_scroll_pin(layout) * self.scroll_y / max_scroll
        };
    }

    fn set_scroll_from_pointer(&mut self, point: GuiPoint, layout: &DefinitionSelLayout) {
        let max_pin = max_scroll_pin(layout);
        self.scroll_pin = ((point.y as i32 - layout.list_scrollbar.y - 16) - 8).clamp(0, max_pin);
        self.scroll_y = self.max_scroll(layout) * self.scroll_pin / max_pin.max(1);
    }

    fn begin_scrollbar_pointer(&mut self, point: GuiPoint, layout: &DefinitionSelLayout) {
        if self.max_scroll(layout) == 0 {
            return;
        }
        let arrow = scrollbar_arrow_at(point, layout);
        if arrow != 0 {
            self.scrollbar_arrow = arrow;
            self.sound_events.push(DefinitionSelSound::ArrowHit);
        } else if layout.list_scrollbar.h > 48 {
            self.set_scroll_from_pointer(point, layout);
            self.scrollbar_dragging = true;
            self.sound_events.push(DefinitionSelSound::Command);
        }
    }

    fn press_pointer_button(
        &mut self,
        target: ButtonTarget,
        layout: &DefinitionSelLayout,
        actions: &mut Vec<DefinitionSelAction>,
    ) {
        self.set_focus(control_for_button(target), true, layout, actions);
        if self.pointer_pressed.is_none() {
            self.pointer_pressed = Some(target);
            self.sound_events.push(DefinitionSelSound::ArrowHit);
        }
    }

    fn pointer_button_is_down(&self, layout: &DefinitionSelLayout) -> bool {
        self.pointer_pressed.is_some_and(|target| {
            self.pointer
                .is_some_and(|point| self.hit_target(point, layout) == hit_for_button(target))
        })
    }

    fn hit_target(&self, point: GuiPoint, layout: &DefinitionSelLayout) -> HitTarget {
        if rect_contains(layout.close_button, point) {
            return HitTarget::Close;
        }
        if rect_contains(layout.ok_button, point) {
            return HitTarget::Ok;
        }
        if rect_contains(layout.cancel_button, point) {
            return HitTarget::Cancel;
        }
        if rect_contains(layout.list_scrollbar, point) {
            return HitTarget::Scrollbar;
        }
        if rect_contains(layout.list_client, point) {
            let content_y = point.y as i32 - layout.list_client.y + self.scroll_y;
            if content_y >= 0 {
                let index = (content_y / layout.row_pitch) as usize;
                let within = content_y % layout.row_pitch;
                if index < self.rows.len() && within < layout.row_height {
                    if self.mode.is_multi_selection()
                        && point.x < (layout.list_client.x + layout.row_height) as f32
                    {
                        return HitTarget::Checkbox(index);
                    }
                    return HitTarget::Row(index);
                }
            }
            return HitTarget::ListBlank;
        }
        if rect_contains(layout.caption, point) {
            return HitTarget::Caption;
        }
        HitTarget::None
    }
}

impl DefinitionSelController {
    pub fn render(
        &self,
        surface: &mut Surface,
        resources: DefinitionSelResources<'_>,
        active: bool,
        gamma: Option<&GammaRamp>,
    ) -> Result<()> {
        resources.validate_for_mode(self.mode)?;
        let layout = self.layout(
            i32::try_from(surface.width()).unwrap_or(i32::MAX),
            i32::try_from(surface.height()).unwrap_or(i32::MAX),
            &resources.fonts.text,
        );

        resources.skin.draw_dialog(surface, layout.bounds, gamma);
        let caption = self.caption();
        let caption_scroll = self.caption_scroll_offset_at(
            Instant::now(),
            &resources.fonts.text,
            layout.caption.w,
            &caption,
        );
        resources.skin.draw_caption_scrolled(
            surface,
            layout.caption,
            &caption,
            &resources.fonts.text,
            [255, 255, 255, 255],
            TextAlign::Left,
            TITLE_RIGHT_INDENT,
            caption_scroll,
            gamma,
        );
        self.draw_close_button(surface, &layout, resources, active, gamma);

        draw_engine_box(
            surface,
            layout.file_list.x,
            layout.file_list.y,
            layout.file_list.x + layout.file_list.w - 1,
            layout.file_list.y + layout.file_list.h - 1,
            0x7f00_0000,
            gamma,
        );
        self.draw_list_contents(surface, &layout, resources, active, gamma)?;
        self.draw_scrollbar(surface, &layout, resources.scroll, gamma);

        draw_engine_box(
            surface,
            layout.preview.x,
            layout.preview.y,
            layout.preview.x + layout.preview.w - 1,
            layout.preview.y + layout.preview.h - 1,
            0x7f00_0000,
            gamma,
        );
        draw_3d_frame(surface, layout.preview, gamma);
        if let Some(row) = self.selected.and_then(|index| self.rows.get(index)) {
            let wrapped = crate::message_dialog::break_message(
                &resources.fonts.text,
                &row.full_path,
                layout.preview_client.w,
            );
            draw_clipped_text(
                surface,
                &resources.fonts.text,
                layout.preview_client.x,
                layout.preview_client.y,
                &wrapped,
                [255, 255, 255, 255],
                TextAlign::Left,
                gamma,
                layout.preview_client,
            );
        }

        for (target, rect, label) in [
            (ButtonTarget::Ok, layout.ok_button, "&OK"),
            (ButtonTarget::Cancel, layout.cancel_button, "Cancel"),
        ] {
            resources.skin.draw_button(
                surface,
                rect,
                label,
                resources.fonts,
                ClassicButtonState {
                    pressed: active && self.button_is_pressed(target, &layout),
                    highlighted: active && self.button_is_highlighted(target, &layout),
                },
                gamma,
            );
        }
        Ok(())
    }

    fn caption_scroll_offset_at(
        &self,
        now: Instant,
        font: &ClonkFont,
        caption_width: i32,
        caption: &str,
    ) -> i32 {
        if caption.is_empty() {
            return 0;
        }
        let max_scroll = (font.measure(caption, true).0 + TITLE_LEFT_INDENT + TITLE_RIGHT_INDENT
            - caption_width)
            .max(0);
        advance_caption_scroll(&self.caption_scroll, now, max_scroll, TITLE_SCROLL_DELAY)
    }

    fn draw_close_button(
        &self,
        surface: &mut Surface,
        layout: &DefinitionSelLayout,
        resources: DefinitionSelResources<'_>,
        active: bool,
        gamma: Option<&GammaRamp>,
    ) {
        let highlighted = active && self.button_is_highlighted(ButtonTarget::Close, layout);
        let pressed = active && self.button_is_pressed(ButtonTarget::Close, layout);
        if highlighted {
            draw_highlight(
                surface,
                layout.close_button,
                resources.button_highlight,
                gamma,
            );
        }
        draw_icon_phase(surface, layout.close_button, resources.icons, 34, gamma);
        if pressed {
            draw_highlight(
                surface,
                layout.close_button,
                resources.button_highlight,
                gamma,
            );
        }
    }

    fn draw_list_contents(
        &self,
        surface: &mut Surface,
        layout: &DefinitionSelLayout,
        resources: DefinitionSelResources<'_>,
        active: bool,
        gamma: Option<&GammaRamp>,
    ) -> Result<()> {
        if layout.list_client.w <= 0 || layout.list_client.h <= 0 {
            return Ok(());
        }
        let mut clipped = Surface::new(
            layout.list_client.w as u32,
            layout.list_client.h as u32,
            PixelFormat::Rgba8888,
        );
        let capture_gpu_scene = surface.is_gpu_scene_capture_active();
        let capture_clonk_text = surface.is_clonk_text_capture_active();
        if capture_gpu_scene {
            clipped.begin_gpu_scene_capture();
        } else {
            copy_surface_region(surface, &mut clipped, layout.list_client, false);
        }
        if capture_clonk_text {
            clipped.begin_clonk_text_capture();
        }

        if let Some(index) = self.selected.filter(|index| *index < self.rows.len()) {
            let y = index as i32 * layout.row_pitch - self.scroll_y;
            let color = if active && self.focus == DefinitionSelControl::FileList {
                0xafaf_0000
            } else {
                0xaf7f_7f7f
            };
            draw_engine_box(
                &mut clipped,
                0,
                y,
                layout.list_client.w - 1,
                y + layout.row_height - 1,
                color,
                gamma,
            );
        }

        for (index, row) in self.rows.iter().enumerate() {
            let y = index as i32 * layout.row_pitch - self.scroll_y;
            if y >= layout.list_client.h || y + layout.row_height <= 0 {
                continue;
            }
            let row_cell = IntRect::new(0, y, layout.row_height, layout.row_height);
            let icon_x = if self.mode.is_multi_selection() {
                let phase = u32::from(row.checked) + 2 * u32::from(row.fixed);
                let cell = resources.checkbox.height();
                draw_facet_stretch(
                    &mut clipped,
                    resources.checkbox,
                    ((phase * cell) as f32, 0.0, cell as f32, cell as f32),
                    (
                        row_cell.x as f32,
                        row_cell.y as f32,
                        row_cell.w as f32,
                        row_cell.h as f32,
                    ),
                    gamma,
                );
                layout.row_height
            } else {
                0
            };
            let icon_rect = row_cell.with_x(icon_x);
            draw_icon_phase(
                &mut clipped,
                icon_rect,
                resources.icons,
                self.mode.icon_phase(),
                gamma,
            );
            let label_x = icon_x + layout.row_height;
            draw_clipped_text(
                &mut clipped,
                &resources.fonts.text,
                label_x,
                y,
                &row.label,
                if row.fixed {
                    [175, 175, 175, 255]
                } else {
                    [255, 255, 255, 255]
                },
                TextAlign::Left,
                gamma,
                IntRect::new(
                    label_x,
                    y.max(0),
                    (layout.list_client.w - label_x).max(0),
                    layout
                        .row_height
                        .min(layout.list_client.h - y.max(0))
                        .max(0),
                ),
            );

            if self.mode.is_multi_selection()
                && active
                && !row.fixed
                && (self.focus == DefinitionSelControl::RowCheckbox(index)
                    || self.pointer.is_some_and(|point| {
                        self.hit_target(point, layout) == HitTarget::Checkbox(index)
                    }))
            {
                let size = layout.row_height / 2;
                draw_highlight(
                    &mut clipped,
                    IntRect::new(layout.row_height / 4, y + layout.row_height / 4, size, size),
                    resources.button_highlight,
                    gamma,
                );
            }
        }
        let offset = Point::new(layout.list_client.x, layout.list_client.y);
        if capture_gpu_scene {
            let _ = surface.append_gpu_scene_from_mut(&mut clipped, offset);
        } else {
            copy_surface_region(surface, &mut clipped, layout.list_client, true);
        }
        if capture_clonk_text {
            let _ = surface.extend_clonk_text_capture_from(&mut clipped, offset);
        }
        Ok(())
    }

    fn draw_scrollbar(
        &self,
        surface: &mut Surface,
        layout: &DefinitionSelLayout,
        scroll: &ImageData,
        gamma: Option<&GammaRamp>,
    ) {
        let bar = layout.list_scrollbar;
        let top_x = if self.scrollbar_arrow < 0 { 16 } else { 0 };
        let bottom_x = if self.scrollbar_arrow > 0 { 16 } else { 0 };
        crate::draw_image_strip(surface, bar.x, bar.y, scroll, top_x, 0, 16, 16, gamma);
        let mut y = 16;
        while y < bar.h - 5 {
            let tile_height = 16.min(bar.h - 5 - y).max(0) as u32;
            if tile_height == 0 {
                break;
            }
            crate::draw_image_strip(
                surface,
                bar.x,
                bar.y + y,
                scroll,
                0,
                16,
                16,
                tile_height,
                gamma,
            );
            y += 16;
        }
        crate::draw_image_strip(
            surface,
            bar.x,
            bar.y + bar.h - 16,
            scroll,
            bottom_x,
            32,
            16,
            16,
            gamma,
        );
        if self.max_scroll(layout) > 0 && bar.h > 48 {
            crate::draw_image_strip(
                surface,
                bar.x,
                bar.y + 16 + self.scroll_pin,
                scroll,
                16,
                16,
                16,
                16,
                gamma,
            );
        }
    }

    fn button_is_pressed(&self, target: ButtonTarget, layout: &DefinitionSelLayout) -> bool {
        self.key_pressed == Some(target)
            || (self.pointer_pressed == Some(target)
                && self
                    .pointer
                    .is_some_and(|point| self.hit_target(point, layout) == hit_for_button(target)))
    }

    fn button_is_highlighted(&self, target: ButtonTarget, layout: &DefinitionSelLayout) -> bool {
        self.focus == control_for_button(target)
            || self
                .pointer
                .is_some_and(|point| self.hit_target(point, layout) == hit_for_button(target))
    }
}

fn draw_highlight(
    surface: &mut Surface,
    rect: IntRect,
    highlight: &ImageData,
    gamma: Option<&GammaRamp>,
) {
    if rect.w <= 0 || rect.h <= 0 {
        return;
    }
    crate::draw_image_bilinear_additive(
        surface,
        &GuiRect::new(rect.x as f32, rect.y as f32, rect.w as f32, rect.h as f32),
        highlight,
        gamma,
    );
}

fn draw_icon_phase(
    surface: &mut Surface,
    rect: IntRect,
    icons: &ImageData,
    phase: u32,
    gamma: Option<&GammaRamp>,
) {
    let columns = icons.width() / 40;
    let source_x = phase % columns * 40;
    let source_y = phase / columns * 40;
    draw_facet_stretch(
        surface,
        icons,
        (source_x as f32, source_y as f32, 40.0, 40.0),
        (rect.x as f32, rect.y as f32, rect.w as f32, rect.h as f32),
        gamma,
    );
}

fn copy_surface_region(source: &mut Surface, scratch: &mut Surface, region: IntRect, back: bool) {
    for y in 0..scratch.height() {
        for x in 0..scratch.width() {
            let screen_x = region.x + x as i32;
            let screen_y = region.y + y as i32;
            if screen_x < 0 || screen_y < 0 {
                continue;
            }
            if back {
                if let Some(pixel) = scratch.get_pixel(x, y) {
                    let _ = source.set_pixel(screen_x as u32, screen_y as u32, pixel);
                }
            } else if let Some(pixel) = source.get_pixel(screen_x as u32, screen_y as u32) {
                let _ = scratch.set_pixel(x, y, pixel);
            }
        }
    }
}

fn button_for_control(control: DefinitionSelControl) -> Option<ButtonTarget> {
    match control {
        DefinitionSelControl::Close => Some(ButtonTarget::Close),
        DefinitionSelControl::Ok => Some(ButtonTarget::Ok),
        DefinitionSelControl::Cancel => Some(ButtonTarget::Cancel),
        DefinitionSelControl::FileList | DefinitionSelControl::RowCheckbox(_) => None,
    }
}

fn control_for_button(target: ButtonTarget) -> DefinitionSelControl {
    match target {
        ButtonTarget::Close => DefinitionSelControl::Close,
        ButtonTarget::Ok => DefinitionSelControl::Ok,
        ButtonTarget::Cancel => DefinitionSelControl::Cancel,
    }
}

fn hit_for_button(target: ButtonTarget) -> HitTarget {
    match target {
        ButtonTarget::Close => HitTarget::Close,
        ButtonTarget::Ok => HitTarget::Ok,
        ButtonTarget::Cancel => HitTarget::Cancel,
    }
}

fn rect_contains(rect: IntRect, point: GuiPoint) -> bool {
    point.x >= rect.x as f32
        && point.y >= rect.y as f32
        && point.x < (rect.x + rect.w) as f32
        && point.y < (rect.y + rect.h) as f32
}

fn scrollbar_arrow_at(point: GuiPoint, layout: &DefinitionSelLayout) -> i8 {
    if !rect_contains(layout.list_scrollbar, point) {
        return 0;
    }
    let y = point.y as i32 - layout.list_scrollbar.y;
    if y < 16 {
        -1
    } else if y >= layout.list_scrollbar.h - 16 {
        1
    } else {
        0
    }
}

fn max_scroll_pin(layout: &DefinitionSelLayout) -> i32 {
    (layout.list_scrollbar.h - 48).max(0)
}

pub fn definition_sel_layout(
    screen_width: i32,
    screen_height: i32,
    text_line_height: i32,
    dialog_offset: (i32, i32),
) -> DefinitionSelLayout {
    let width = (screen_width * 2 / 3 + 10).clamp(MIN_WIDTH, MAX_WIDTH);
    let height = (screen_height * 2 / 3 + 10).clamp(MIN_HEIGHT, MAX_HEIGHT);
    let title_height = text_line_height.max(MIN_CAPTION_HEIGHT);
    let x = (screen_width - width) / 2 + dialog_offset.0;
    let y = (screen_height - height) / 2 + dialog_offset.1;
    let client = IntRect::new(x, y + title_height, width, height - title_height);
    let upper_height = client.h - 40;
    let file_list = IntRect::new(client.x + 10, client.y + 10, width / 2, upper_height - 20);
    let list_client = IntRect::new(
        file_list.x + 3,
        file_list.y + 3,
        file_list.w - 6 - SCROLLBAR_WIDTH,
        file_list.h - 6,
    );
    let preview = IntRect::new(
        client.x + width / 2 + 20,
        client.y + 10,
        width - width / 2 - 30,
        upper_height - 20,
    );
    let button_group_x = client.x + (width - 280) / 2;
    DefinitionSelLayout {
        bounds: IntRect::new(x, y, width, height),
        caption: IntRect::new(x, y, width, title_height),
        client,
        close_button: IntRect::new(x + width - 20, y + 4, 16, 16),
        file_list,
        list_client,
        list_scrollbar: IntRect::new(
            list_client.x + list_client.w,
            list_client.y,
            16,
            list_client.h,
        ),
        preview,
        preview_client: IntRect::new(
            preview.x + 10,
            preview.y + 8,
            preview.w - 15 - 16,
            preview.h - 16,
        ),
        ok_button: IntRect::new(
            button_group_x + 10,
            client.y + client.h - 36,
            BUTTON_WIDTH,
            BUTTON_HEIGHT,
        ),
        cancel_button: IntRect::new(
            button_group_x + 150,
            client.y + client.h - 36,
            BUTTON_WIDTH,
            BUTTON_HEIGHT,
        ),
        row_height: text_line_height,
        row_pitch: text_line_height + ROW_SPACING,
    }
}

fn build_rows(
    entries: &[DefinitionSelEntry],
    fixed: &[String],
    initial_checks: bool,
) -> Vec<DefinitionSelRow> {
    entries
        .iter()
        .map(|entry| {
            let is_fixed = fixed.iter().any(|value| value == &entry.filename);
            DefinitionSelRow {
                full_path: entry.full_path.clone(),
                filename: entry.filename.clone(),
                label: remove_final_extension(&entry.filename),
                fixed: is_fixed,
                checked: initial_checks && is_fixed,
            }
        })
        .collect()
}

fn remove_final_extension(filename: &str) -> String {
    let separator = filename.rfind(['/', '\\']).map_or(0, |index| index + 1);
    filename
        .rfind('.')
        .filter(|&index| index >= separator && index + 1 < filename.len())
        .map_or_else(|| filename.to_owned(), |index| filename[..index].to_owned())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::classic_gui::ClassicGuiSkin;
    use crate::test_support::{endeavour_font_set, load_graphics_png, standard_gamma};
    use clonk_graphics::Color;

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

    fn entries(count: usize) -> Vec<DefinitionSelEntry> {
        (0..count)
            .map(|index| {
                DefinitionSelEntry::new(
                    format!("/Definitions/Pack{index}.c4d"),
                    format!("Pack{index}.c4d"),
                )
            })
            .collect()
    }

    fn player_entries(names: &[&str]) -> Vec<DefinitionSelEntry> {
        names
            .iter()
            .map(|name| {
                DefinitionSelEntry::new(format!("/Players/{name}.c4p"), format!("{name}.c4p"))
            })
            .collect()
    }

    fn center(rect: IntRect) -> GuiPoint {
        GuiPoint::new((rect.x + rect.w / 2) as f32, (rect.y + rect.h / 2) as f32)
    }

    #[test]
    fn geometry_matches_the_classic_component_aligners() {
        let fonts = endeavour_font_set();
        let controller = DefinitionSelController::new("/Definitions", Vec::new(), Vec::new());
        let layout = controller.layout(1280, 720, &fonts.text);
        assert_eq!(layout.bounds, IntRect::new(340, 115, 600, 490));
        assert_eq!(layout.caption, IntRect::new(340, 115, 600, 23));
        assert_eq!(layout.client, IntRect::new(340, 138, 600, 467));
        assert_eq!(layout.close_button, IntRect::new(920, 119, 16, 16));
        assert_eq!(layout.file_list, IntRect::new(350, 148, 300, 407));
        assert_eq!(layout.list_client, IntRect::new(353, 151, 278, 401));
        assert_eq!(layout.list_scrollbar, IntRect::new(631, 151, 16, 401));
        assert_eq!(layout.preview, IntRect::new(660, 148, 270, 407));
        assert_eq!(layout.preview_client, IntRect::new(670, 156, 239, 391));
        assert_eq!(layout.ok_button, IntRect::new(510, 569, 120, 32));
        assert_eq!(layout.cancel_button, IntRect::new(650, 569, 120, 32));
        assert_eq!((layout.row_height, layout.row_pitch), (22, 23));
    }

    #[test]
    fn fixed_rows_initial_output_and_refresh_quirk_are_exact() {
        let mut controller = DefinitionSelController::new(
            "/Definitions",
            vec!["Pack1.c4d".into(), "Pack1.c4d".into()],
            entries(3),
        );
        assert_eq!(controller.selected_index(), None);
        assert!(!controller.rows()[0].is_checked());
        assert!(controller.rows()[1].is_fixed());
        assert!(controller.rows()[1].is_checked());
        assert_eq!(controller.rows()[1].label(), "Pack1");
        assert_eq!(
            controller.accepted_selection(),
            vec!["Pack1.c4d".to_owned(), "Pack1.c4d".to_owned()]
        );

        controller.rows[0].checked = true;
        assert_eq!(
            controller.accepted_selection(),
            vec![
                "Pack1.c4d".to_owned(),
                "Pack1.c4d".to_owned(),
                "Pack0.c4d".to_owned()
            ]
        );

        controller.rebuild_rows_after_refresh(entries(3));
        assert_eq!(controller.selected_index(), None);
        assert!(controller.rows()[1].is_fixed());
        assert!(!controller.rows()[1].is_checked());
        assert_eq!(
            controller.accepted_selection(),
            vec!["Pack1.c4d".to_owned(), "Pack1.c4d".to_owned()]
        );
    }

    #[test]
    fn keyboard_focus_toggle_accept_cancel_and_refresh_contract() {
        let fonts = endeavour_font_set();
        let mut controller =
            DefinitionSelController::new("/Definitions", vec!["Pack2.c4d".into()], entries(4));
        let layout = controller.layout(1280, 720, &fonts.text);
        assert_eq!(
            controller.handle_gamepad_low_down(&layout),
            vec![DefinitionSelAction::PleaseSelectFile]
        );
        assert!(controller.handle_gamepad_low_up().is_empty());
        assert_eq!(
            controller.handle_key_down(DefinitionSelKey::Enter, false, &layout),
            vec![DefinitionSelAction::PleaseSelectFile]
        );
        assert_eq!(
            controller.handle_key_down(DefinitionSelKey::Down, false, &layout),
            vec![DefinitionSelAction::SelectionChanged(Some(0))]
        );
        assert_eq!(
            controller.handle_key_down(DefinitionSelKey::Space, false, &layout),
            vec![DefinitionSelAction::CheckedChanged {
                index: 0,
                checked: true
            }]
        );
        assert_eq!(
            controller.handle_key_down(DefinitionSelKey::End, false, &layout),
            vec![DefinitionSelAction::SelectionChanged(Some(3))]
        );
        assert_eq!(
            controller.handle_key_down(DefinitionSelKey::Tab, false, &layout),
            vec![DefinitionSelAction::FocusChanged(
                DefinitionSelControl::RowCheckbox(3)
            )]
        );
        assert_eq!(
            controller.handle_key_down(DefinitionSelKey::Tab, false, &layout),
            vec![DefinitionSelAction::FocusChanged(DefinitionSelControl::Ok)]
        );
        assert_eq!(
            controller.handle_key_down(DefinitionSelKey::Enter, false, &layout),
            vec![DefinitionSelAction::Accepted(vec![
                "Pack2.c4d".to_owned(),
                "Pack0.c4d".to_owned(),
            ])]
        );
        assert_eq!(
            controller.handle_key_down(DefinitionSelKey::Refresh, false, &layout),
            vec![DefinitionSelAction::RefreshRequested]
        );
        assert_eq!(
            controller.handle_key_down(DefinitionSelKey::Escape, false, &layout),
            vec![DefinitionSelAction::Cancelled]
        );
    }

    #[test]
    fn player_mode_is_single_selection_and_accepts_the_full_path() {
        let fonts = endeavour_font_set();
        let mut controller =
            DefinitionSelController::new_player("/Players", player_entries(&["Alice", "Bob"]));
        let layout = controller.layout(1280, 720, &fonts.text);

        assert_eq!(controller.mode(), FileSelMode::Player);
        assert!(!controller.is_multi_selection());
        assert_eq!(controller.caption(), "Select player...");
        assert!(controller
            .rows()
            .iter()
            .all(|row| !row.is_fixed() && !row.is_checked()));
        assert_eq!(
            controller.handle_key_down(DefinitionSelKey::Enter, false, &layout),
            vec![DefinitionSelAction::PleaseSelectFile]
        );
        assert_eq!(
            controller.handle_key_down(DefinitionSelKey::Down, false, &layout),
            vec![DefinitionSelAction::SelectionChanged(Some(0))]
        );
        assert_eq!(controller.selected_full_path(), Some("/Players/Alice.c4p"));
        assert!(controller
            .handle_key_down(DefinitionSelKey::Space, false, &layout)
            .is_empty());
        assert_eq!(
            controller.handle_key_down(DefinitionSelKey::Enter, false, &layout),
            vec![DefinitionSelAction::Accepted(vec![
                "/Players/Alice.c4p".to_owned()
            ])]
        );
        assert_eq!(
            controller.handle_gamepad_low_down(&layout),
            vec![DefinitionSelAction::Accepted(vec![
                "/Players/Alice.c4p".to_owned()
            ])]
        );
        assert_eq!(
            controller.handle_key_down(DefinitionSelKey::Tab, false, &layout),
            vec![DefinitionSelAction::FocusChanged(DefinitionSelControl::Ok)],
            "single-selection rows never insert a checkbox into tab order"
        );
        assert_eq!(
            controller.handle_key_down(DefinitionSelKey::Refresh, false, &layout),
            vec![DefinitionSelAction::RefreshRequested]
        );

        controller.rebuild_rows_after_refresh(player_entries(&["Carol"]));
        assert_eq!(controller.selected_index(), None);
        assert_eq!(controller.selected_full_path(), None);
        assert!(controller.accepted_selection().is_empty());
        assert_eq!(controller.rows()[0].label(), "Carol");
        assert_eq!(
            DefinitionSelController::new_player("", Vec::new()).caption(),
            "Select player..."
        );
    }

    #[test]
    fn player_icon_cell_selects_and_double_click_accepts_without_a_checkbox() {
        let fonts = endeavour_font_set();
        let mut controller =
            DefinitionSelController::new_player("/Players", player_entries(&["Alice", "Bob"]));
        let layout = controller.layout(1280, 720, &fonts.text);
        let first_icon = GuiPoint::new(
            (layout.list_client.x + 5) as f32,
            (layout.list_client.y + 5) as f32,
        );

        assert_eq!(controller.mode.icon_phase(), PLAYER_ICON_PHASE);
        assert_eq!(
            controller.hit_target(first_icon, &layout),
            HitTarget::Row(0)
        );
        assert_eq!(
            controller.handle_pointer_double_click(first_icon, &layout),
            vec![
                DefinitionSelAction::SelectionChanged(Some(0)),
                DefinitionSelAction::Accepted(vec!["/Players/Alice.c4p".to_owned()]),
            ]
        );
        assert!(controller.rows().iter().all(|row| !row.is_checked()));
    }

    #[test]
    fn pointer_touch_wheel_scrollbar_and_title_drag_are_exposed() {
        let fonts = endeavour_font_set();
        let mut controller = DefinitionSelController::new("/Definitions", Vec::new(), entries(30));
        let layout = controller.layout(1280, 720, &fonts.text);
        let first_label = GuiPoint::new(
            (layout.list_client.x + 60) as f32,
            (layout.list_client.y + 10) as f32,
        );
        assert_eq!(
            controller.handle_pointer_down(first_label, &layout),
            vec![DefinitionSelAction::SelectionChanged(Some(0))]
        );
        let second_label = GuiPoint::new(
            (layout.list_client.x + 60) as f32,
            (layout.list_client.y + 33) as f32,
        );
        assert_eq!(
            controller.handle_pointer_double_click(second_label, &layout),
            vec![
                DefinitionSelAction::SelectionChanged(Some(1)),
                DefinitionSelAction::CheckedChanged {
                    index: 1,
                    checked: true
                },
            ]
        );
        let third_checkbox = GuiPoint::new(
            (layout.list_client.x + 5) as f32,
            (layout.list_client.y + 51) as f32,
        );
        assert_eq!(
            controller.handle_touch_end(third_checkbox, &layout),
            vec![DefinitionSelAction::CheckedChanged {
                index: 2,
                checked: true
            }]
        );

        controller.handle_wheel(first_label, -100, &layout);
        assert_eq!(controller.scroll_y(), 100);
        let track = GuiPoint::new(
            (layout.list_scrollbar.x + 8) as f32,
            (layout.list_scrollbar.y + layout.list_scrollbar.h / 2) as f32,
        );
        controller.handle_pointer_down(track, &layout);
        assert!(controller.scrollbar_dragging);
        assert!(controller.scroll_y() > 0);
        controller.handle_pointer_up(track, &layout);

        let title = GuiPoint::new(
            (layout.caption.x + 100) as f32,
            (layout.caption.y + 10) as f32,
        );
        controller.handle_pointer_down(title, &layout);
        controller.handle_pointer_move(GuiPoint::new(title.x + 20.0, title.y + 10.0), &layout);
        assert_eq!(controller.dialog_offset(), (20, 10));
    }

    #[test]
    fn caption_tooltips_expose_title_and_localized_close_resource() {
        let fonts = endeavour_font_set();
        let mut controller = DefinitionSelController::new("/Definitions", Vec::new(), Vec::new());
        let layout = controller.layout(1280, 720, &fonts.text);
        let title_point = GuiPoint::new(
            (layout.caption.x + 10) as f32,
            (layout.caption.y + 10) as f32,
        );
        let _ = controller.handle_pointer_move(title_point, &layout);
        assert_eq!(
            controller.tooltip_at(title_point, &layout),
            Some(StartupTooltip::text(controller.caption()))
        );
        assert_eq!(
            controller.tooltip_at(center(layout.close_button), &layout),
            None,
            "an unrouted overlapping control cannot claim the shared timer"
        );
        let _ = controller.handle_pointer_move(center(layout.close_button), &layout);
        assert_eq!(
            controller.tooltip_at(center(layout.close_button), &layout),
            Some(StartupTooltip::resource("IDS_MNU_CLOSE")),
            "the close icon wins its overlap with the caption"
        );
        let _ = controller.handle_pointer_move(center(layout.client), &layout);
        assert_eq!(controller.tooltip_at(center(layout.client), &layout), None);
    }

    #[test]
    fn caption_autoscroll_advances_per_frame_and_dwells_at_both_ends() {
        const CAPTION_WIDTH: i32 = 300;
        const TARGET_TEXT_WIDTH: usize = 278;
        let fixed_characters = "Select Object Definitions []".chars().count();
        let root = "W".repeat(TARGET_TEXT_WIDTH - fixed_characters);
        let controller = DefinitionSelController::new(root, Vec::new(), Vec::new());
        let caption = controller.caption();
        let font = unit_width_font(&caption);
        assert_eq!(
            font.measure(&caption, true).0 + TITLE_LEFT_INDENT + TITLE_RIGHT_INDENT - CAPTION_WIDTH,
            3
        );

        let base = Instant::now();
        assert_eq!(
            controller.caption_scroll_offset_at(base, &font, CAPTION_WIDTH, &caption),
            0
        );
        assert_eq!(
            controller.caption_scroll_offset_at(
                base + TITLE_SCROLL_DELAY - Duration::from_millis(1),
                &font,
                CAPTION_WIDTH,
                &caption,
            ),
            0
        );

        let outbound = base + TITLE_SCROLL_DELAY;
        assert_eq!(
            controller.caption_scroll_offset_at(outbound, &font, CAPTION_WIDTH, &caption),
            1
        );
        assert_eq!(
            controller.caption_scroll_offset_at(outbound, &font, CAPTION_WIDTH, &caption),
            2
        );
        assert_eq!(
            controller.caption_scroll_offset_at(outbound, &font, CAPTION_WIDTH, &caption),
            2,
            "the attempted max-scroll frame reverses and immediately backs off"
        );
        assert_eq!(
            controller.caption_scroll_offset_at(
                outbound + TITLE_SCROLL_DELAY - Duration::from_millis(1),
                &font,
                CAPTION_WIDTH,
                &caption,
            ),
            2
        );

        let returning = outbound + TITLE_SCROLL_DELAY;
        assert_eq!(
            controller.caption_scroll_offset_at(returning, &font, CAPTION_WIDTH, &caption),
            1
        );
        assert_eq!(
            controller.caption_scroll_offset_at(returning, &font, CAPTION_WIDTH, &caption),
            0
        );
        assert_eq!(
            controller.caption_scroll_offset_at(returning, &font, CAPTION_WIDTH, &caption),
            0,
            "the attempted negative frame reverses and pauses at the start"
        );
    }

    #[test]
    fn render_uses_classic_assets_and_rejects_missing_phases() {
        let fonts = endeavour_font_set();
        let caption = load_graphics_png("GUICaption.png");
        let button = load_graphics_png("GUIButton.png");
        let button_down = load_graphics_png("GUIButtonDown.png");
        let highlight = load_graphics_png("GUIButtonHighlight.png");
        let icons = load_graphics_png("GUIIcons.png");
        let checkbox = load_graphics_png("GUICheckbox.png");
        let scroll = load_graphics_png("GUIScroll.png");
        let skin = ClassicGuiSkin::new(&caption, &button, &button_down, Some(&highlight));
        let resources = DefinitionSelResources {
            skin,
            fonts: &fonts,
            icons: &icons,
            checkbox: &checkbox,
            scroll: &scroll,
            button_highlight: &highlight,
        };
        let mut controller = DefinitionSelController::new("/Definitions", Vec::new(), entries(20));
        let layout = controller.layout(1280, 720, &fonts.text);
        controller.handle_key_down(DefinitionSelKey::Down, false, &layout);
        let mut active = Surface::new(1280, 720, PixelFormat::Rgba8888);
        let before = active.pixels().to_vec();
        controller
            .render(&mut active, resources, true, Some(standard_gamma()))
            .unwrap();
        assert_ne!(active.pixels(), before.as_slice());

        let mut inactive = Surface::new(1280, 720, PixelFormat::Rgba8888);
        controller
            .render(&mut inactive, resources, false, Some(standard_gamma()))
            .unwrap();
        let sample_x = (layout.list_client.x + layout.list_client.w - 3) as u32;
        let sample_y = (layout.list_client.y + 10) as u32;
        assert_ne!(
            active.get_pixel(sample_x, sample_y),
            inactive.get_pixel(sample_x, sample_y)
        );

        let bad_checkbox = ImageData::new(1, 1, vec![0, 0, 0, 0]);
        let bad_resources = DefinitionSelResources {
            checkbox: &bad_checkbox,
            ..resources
        };
        let error = controller
            .render(&mut inactive, bad_resources, true, None)
            .unwrap_err();
        assert!(error.to_string().contains("all four"));

        let player = DefinitionSelController::new_player("/Players", player_entries(&["Alice"]));
        let mut player_surface = Surface::new(1280, 720, PixelFormat::Rgba8888);
        player
            .render(&mut player_surface, bad_resources, true, None)
            .expect("single-selection player rows do not consume GUICheckbox.png");
    }
}
