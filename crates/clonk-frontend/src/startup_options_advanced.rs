//! Standalone classic modal used by the startup Options dialog's advanced
//! configuration editor.
//!
//! The host owns the typed configuration schema and persistence. This module
//! deliberately edits a private draft and reports changed values as strings;
//! cancelling therefore cannot mutate the live configuration accidentally.

use crate::caption_scroll::{advance_caption_scroll, CaptionScrollState};
use crate::classic_gui::{
    draw_3d_frame, draw_clipped_text, draw_engine_box, draw_engine_frame, draw_facet_stretch,
    ClassicButtonState, ClassicGuiSkin, IntRect,
};
use crate::rename_edit::{RenameEdit, RenameEditCursorOperation};
use crate::startup_main_menu::StartupTooltip;
use crate::{expand_hotkey_markup, ClonkFontSet, GuiPoint, ImageData, KeyCode};
use anyhow::{ensure, Result};
use clonk_graphics::clonk_font::{ClonkFont, TextAlign};
use clonk_graphics::{GammaRamp, Rect, Surface};
use std::borrow::Cow;
use std::cell::Cell;
use std::time::{Duration, Instant};

const DEFAULT_WIDTH: i32 = 800;
const DEFAULT_HEIGHT: i32 = 600;
const CAPTION_HEIGHT: i32 = 32;
const OUTER_MARGIN: i32 = 10;
const TAB_GAP: i32 = 8;
const TAB_HEIGHT: i32 = 30;
const ROW_HEIGHT: i32 = 26;
const ROW_PITCH: i32 = 29;
const SCROLLBAR_WIDTH: i32 = 14;
const BUTTON_HEIGHT: i32 = 32;
const BUTTON_GAP: i32 = 10;
const MAX_BUTTON_WIDTH: i32 = 200;
const MAX_EDIT_BYTES: usize = 254;
const TITLE_LEFT_INDENT: i32 = 5;
const TITLE_RIGHT_INDENT: i32 = 20;
const TITLE_SCROLL_DELAY: Duration = Duration::from_millis(3000);

/// One stable persisted value and its user-facing label in a choice row.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct AdvancedConfigChoice {
    pub value: String,
    pub label: String,
}

/// Typed value displayed by one advanced setting row.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum AdvancedConfigValue {
    Bool(bool),
    Integer {
        value: i128,
        min: i128,
        max: i128,
    },
    /// A finite set of user-facing labels backed by stable persisted values.
    /// `value` is deliberately independent of the label and may be absent
    /// from `choices` when a saved device or plugin is temporarily missing.
    Choice {
        value: String,
        choices: Vec<AdvancedConfigChoice>,
    },
    Text(String),
    /// A raw or explicitly protected setting. It is displayed but cannot be
    /// changed by either controller input or [`AdvancedConfigController::set_value`].
    ReadOnly(String),
}

impl AdvancedConfigValue {
    /// INI-compatible representation returned by
    /// [`AdvancedConfigController::changes`]. Classic boolean fields compile
    /// as integer `0`/`1` values.
    pub fn serialized(&self) -> String {
        match self {
            Self::Bool(value) => i32::from(*value).to_string(),
            Self::Integer { value, .. } => value.to_string(),
            Self::Choice { value, .. } | Self::Text(value) | Self::ReadOnly(value) => value.clone(),
        }
    }

    fn display_text(&self) -> Cow<'_, str> {
        match self {
            Self::Bool(value) => Cow::Borrowed(if *value { "1" } else { "0" }),
            Self::Integer { value, .. } => Cow::Owned(value.to_string()),
            Self::Choice { value, choices } => choices
                .iter()
                .find(|choice| choice.value == *value)
                .map_or_else(
                    || Cow::Borrowed(value.as_str()),
                    |choice| Cow::Borrowed(choice.label.as_str()),
                ),
            Self::Text(value) | Self::ReadOnly(value) => Cow::Borrowed(value),
        }
    }

    pub const fn is_editable(&self) -> bool {
        !matches!(self, Self::ReadOnly(_))
    }

    fn normalize(&mut self) {
        if let Self::Integer { value, min, max } = self {
            if *min > *max {
                std::mem::swap(min, max);
            }
            *value = (*value).clamp(*min, *max);
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct AdvancedConfigRow {
    pub name: String,
    pub value: AdvancedConfigValue,
}

impl AdvancedConfigRow {
    pub fn new(name: impl Into<String>, value: AdvancedConfigValue) -> Self {
        Self {
            name: name.into(),
            value,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct AdvancedConfigSection {
    pub name: String,
    pub rows: Vec<AdvancedConfigRow>,
}

impl AdvancedConfigSection {
    pub fn new(name: impl Into<String>, rows: Vec<AdvancedConfigRow>) -> Self {
        Self {
            name: name.into(),
            rows,
        }
    }
}

/// One changed value ready for the host to write to its config store.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct AdvancedConfigChange {
    pub section: String,
    pub key: String,
    pub value: String,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum AdvancedConfigAction {
    Save,
    Cancel,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum AdvancedConfigSound {
    ArrowHit,
    Click,
    Command,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct AdvancedConfigLabels {
    pub caption: String,
    pub save: String,
    pub cancel: String,
}

impl Default for AdvancedConfigLabels {
    fn default() -> Self {
        Self {
            caption: "Advanced settings".into(),
            save: "&Save".into(),
            cancel: "Cancel".into(),
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum AdvancedConfigFocus {
    SectionTabs,
    Row(usize),
    Save,
    Cancel,
    Close,
}

/// Public hit-test result for app-level input and geometry tests.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum AdvancedConfigHit {
    Close,
    Caption,
    Section(usize),
    Row(usize),
    Checkbox(usize),
    Edit(usize),
    Decrement(usize),
    Increment(usize),
    Scrollbar,
    Save,
    Cancel,
    None,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct AdvancedConfigRowLayout {
    pub index: usize,
    pub bounds: IntRect,
    pub label: IntRect,
    pub control: IntRect,
    pub checkbox: Option<IntRect>,
    pub edit: Option<IntRect>,
    pub decrement_button: Option<IntRect>,
    pub increment_button: Option<IntRect>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct AdvancedConfigLayout {
    pub bounds: IntRect,
    pub caption: IntRect,
    pub close_button: IntRect,
    pub client: IntRect,
    pub section_list: IntRect,
    pub section_tabs: Vec<IntRect>,
    pub settings_list: IntRect,
    pub list_client: IntRect,
    pub scrollbar: IntRect,
    pub scrollbar_thumb: IntRect,
    pub rows: Vec<AdvancedConfigRowLayout>,
    pub save_button: IntRect,
    pub cancel_button: IntRect,
    pub row_height: i32,
    pub row_pitch: i32,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum PressTarget {
    Close,
    Checkbox(usize),
    Decrement(usize),
    Increment(usize),
    Save,
    Cancel,
}

#[derive(Clone, Debug)]
struct ActiveEdit {
    section: usize,
    row: usize,
    editor: RenameEdit<()>,
}

#[derive(Clone, Copy, Debug)]
struct CaptionDrag {
    pointer: GuiPoint,
    offset: (i32, i32),
}

/// Mutable draft and modal input controller. All pointer methods use the size
/// last supplied to [`Self::resize`].
#[derive(Clone, Debug)]
pub struct AdvancedConfigController {
    width: i32,
    height: i32,
    dialog_offset: (i32, i32),
    sections: Vec<AdvancedConfigSection>,
    original_values: Vec<Vec<String>>,
    labels: AdvancedConfigLabels,
    current_section: usize,
    scroll_y: i32,
    section_scroll_y: Vec<i32>,
    focus: AdvancedConfigFocus,
    pointer: Option<GuiPoint>,
    pointer_pressed: Option<PressTarget>,
    key_pressed: Option<PressTarget>,
    scrollbar_drag_offset: Option<i32>,
    caption_drag: Option<CaptionDrag>,
    caption_scroll: Cell<CaptionScrollState>,
    active_edit: Option<ActiveEdit>,
    sound_events: Vec<AdvancedConfigSound>,
}

impl AdvancedConfigController {
    pub fn new(mut sections: Vec<AdvancedConfigSection>) -> Self {
        for section in &mut sections {
            for row in &mut section.rows {
                row.value.normalize();
            }
        }
        let original_values = sections
            .iter()
            .map(|section| {
                section
                    .rows
                    .iter()
                    .map(|row| row.value.serialized())
                    .collect()
            })
            .collect();
        let section_scroll_y = vec![0; sections.len()];
        Self {
            width: DEFAULT_WIDTH,
            height: DEFAULT_HEIGHT,
            dialog_offset: (0, 0),
            sections,
            original_values,
            labels: AdvancedConfigLabels::default(),
            current_section: 0,
            scroll_y: 0,
            section_scroll_y,
            focus: AdvancedConfigFocus::SectionTabs,
            pointer: None,
            pointer_pressed: None,
            key_pressed: None,
            scrollbar_drag_offset: None,
            caption_drag: None,
            caption_scroll: Cell::new(CaptionScrollState::default()),
            active_edit: None,
            sound_events: Vec::new(),
        }
    }

    pub fn resize(&mut self, width: i32, height: i32) {
        self.width = width.max(1);
        self.height = height.max(1);
        self.clamp_scroll();
    }

    pub fn sections(&self) -> &[AdvancedConfigSection] {
        &self.sections
    }

    pub fn labels(&self) -> &AdvancedConfigLabels {
        &self.labels
    }

    pub fn set_labels(&mut self, labels: AdvancedConfigLabels) {
        if self.labels.caption != labels.caption {
            self.caption_scroll.set(CaptionScrollState::default());
        }
        self.labels = labels;
    }

    pub fn take_sound_events(&mut self) -> Vec<AdvancedConfigSound> {
        std::mem::take(&mut self.sound_events)
    }

    pub fn tick_edit_blink(&mut self) -> bool {
        self.active_edit
            .as_mut()
            .is_some_and(|active| active.editor.tick_blink())
    }

    pub const fn current_section_index(&self) -> usize {
        self.current_section
    }

    pub fn current_section(&self) -> Option<&AdvancedConfigSection> {
        self.sections.get(self.current_section)
    }

    pub const fn focus(&self) -> AdvancedConfigFocus {
        self.focus
    }

    pub const fn scroll_y(&self) -> i32 {
        self.scroll_y
    }

    pub const fn pointer_position(&self) -> Option<GuiPoint> {
        self.pointer
    }

    pub const fn dialog_offset(&self) -> (i32, i32) {
        self.dialog_offset
    }

    pub const fn has_positional_pointer_drag(&self) -> bool {
        self.caption_drag.is_some()
    }

    pub fn layout(&self) -> AdvancedConfigLayout {
        advanced_config_layout_with_offset(
            self.width,
            self.height,
            &self.sections,
            self.current_section,
            self.scroll_y,
            self.dialog_offset,
        )
    }

    pub fn select_section(&mut self, index: usize) -> bool {
        if index >= self.sections.len() {
            return false;
        }
        self.finish_edit();
        if let Some(scroll) = self.section_scroll_y.get_mut(self.current_section) {
            *scroll = self.scroll_y;
        }
        let changed = self.current_section != index;
        self.current_section = index;
        self.scroll_y = self.section_scroll_y.get(index).copied().unwrap_or(0);
        self.clamp_scroll();
        if matches!(self.focus, AdvancedConfigFocus::Row(_)) {
            self.focus = AdvancedConfigFocus::SectionTabs;
        }
        if changed {
            self.sound_events.push(AdvancedConfigSound::Command);
        }
        changed
    }

    pub fn select_section_named(&mut self, name: &str) -> bool {
        let Some(index) = self
            .sections
            .iter()
            .position(|section| section.name == name)
        else {
            return false;
        };
        self.select_section(index)
    }

    pub fn select_relative_section(&mut self, backwards: bool) -> bool {
        if self.sections.is_empty() {
            return false;
        }
        let index = if backwards {
            (self.current_section + self.sections.len() - 1) % self.sections.len()
        } else {
            (self.current_section + 1) % self.sections.len()
        };
        self.focus = AdvancedConfigFocus::SectionTabs;
        self.select_section(index)
    }

    pub fn value(&self, section: &str, key: &str) -> Option<&AdvancedConfigValue> {
        self.sections
            .iter()
            .find(|candidate| candidate.name == section)?
            .rows
            .iter()
            .find(|row| row.name == key)
            .map(|row| &row.value)
    }

    /// Changes a draft value while preserving the schema's original type and
    /// integer bounds. Returns false for unknown, type-mismatched or read-only
    /// settings.
    pub fn set_value(&mut self, section: &str, key: &str, value: AdvancedConfigValue) -> bool {
        self.finish_edit();
        let Some(section_index) = self
            .sections
            .iter()
            .position(|candidate| candidate.name == section)
        else {
            return false;
        };
        let Some(row_index) = self.sections[section_index]
            .rows
            .iter()
            .position(|row| row.name == key)
        else {
            return false;
        };
        let current = &mut self.sections[section_index].rows[row_index].value;
        match (current, value) {
            (AdvancedConfigValue::Bool(current), AdvancedConfigValue::Bool(value)) => {
                *current = value;
                true
            }
            (
                AdvancedConfigValue::Integer {
                    value: current,
                    min,
                    max,
                },
                AdvancedConfigValue::Integer { value, .. },
            ) => {
                *current = value.clamp(*min, *max);
                true
            }
            (AdvancedConfigValue::Text(current), AdvancedConfigValue::Text(value)) => {
                *current = truncate_utf8(value, MAX_EDIT_BYTES);
                true
            }
            (
                AdvancedConfigValue::Choice {
                    value: current,
                    choices,
                },
                AdvancedConfigValue::Choice { value, .. },
            ) if choices.iter().any(|choice| choice.value == value) => {
                *current = value;
                true
            }
            _ => false,
        }
    }

    /// Returns only editable values whose serialized representation differs
    /// from the initial draft.
    pub fn changes(&self) -> Vec<AdvancedConfigChange> {
        let mut changes = Vec::new();
        for (section_index, section) in self.sections.iter().enumerate() {
            for (row_index, row) in section.rows.iter().enumerate() {
                if !row.value.is_editable() {
                    continue;
                }
                let value = row.value.serialized();
                if self
                    .original_values
                    .get(section_index)
                    .and_then(|rows| rows.get(row_index))
                    != Some(&value)
                {
                    changes.push(AdvancedConfigChange {
                        section: section.name.clone(),
                        key: row.name.clone(),
                        value,
                    });
                }
            }
        }
        changes
    }

    pub fn hit_test(&self, point: GuiPoint) -> AdvancedConfigHit {
        let layout = self.layout();
        if rect_contains(layout.close_button, point) {
            return AdvancedConfigHit::Close;
        }
        if rect_contains(layout.caption, point) {
            return AdvancedConfigHit::Caption;
        }
        if rect_contains(layout.save_button, point) {
            return AdvancedConfigHit::Save;
        }
        if rect_contains(layout.cancel_button, point) {
            return AdvancedConfigHit::Cancel;
        }
        if let Some(index) = layout
            .section_tabs
            .iter()
            .position(|rect| rect_contains(*rect, point))
        {
            return AdvancedConfigHit::Section(index);
        }
        if rect_contains(layout.scrollbar, point) {
            return AdvancedConfigHit::Scrollbar;
        }
        if !rect_contains(layout.list_client, point) {
            return AdvancedConfigHit::None;
        }
        for row in &layout.rows {
            if !rect_contains(row.bounds, point) {
                continue;
            }
            if row.checkbox.is_some_and(|rect| rect_contains(rect, point)) {
                return AdvancedConfigHit::Checkbox(row.index);
            }
            if row
                .decrement_button
                .is_some_and(|rect| rect_contains(rect, point))
            {
                return AdvancedConfigHit::Decrement(row.index);
            }
            if row
                .increment_button
                .is_some_and(|rect| rect_contains(rect, point))
            {
                return AdvancedConfigHit::Increment(row.index);
            }
            if row.edit.is_some_and(|rect| rect_contains(rect, point)) {
                return AdvancedConfigHit::Edit(row.index);
            }
            return AdvancedConfigHit::Row(row.index);
        }
        AdvancedConfigHit::None
    }

    /// Returns the tooltip assigned by `C4GUI::Dialog::SetTitle` at `point`.
    /// The application owns the screen-global 500ms hover clock and resolves
    /// the close resource in the active language.
    pub fn tooltip_at(&self, point: GuiPoint) -> Option<StartupTooltip> {
        let routed_pointer = self.pointer?;
        if routed_pointer.x as i32 != point.x as i32 || routed_pointer.y as i32 != point.y as i32 {
            return None;
        }
        match self.hit_test(point) {
            AdvancedConfigHit::Close => Some(StartupTooltip::resource("IDS_MNU_CLOSE")),
            AdvancedConfigHit::Caption => (!self.labels.caption.is_empty())
                .then(|| StartupTooltip::text(&self.labels.caption)),
            AdvancedConfigHit::Section(_)
            | AdvancedConfigHit::Row(_)
            | AdvancedConfigHit::Checkbox(_)
            | AdvancedConfigHit::Edit(_)
            | AdvancedConfigHit::Decrement(_)
            | AdvancedConfigHit::Increment(_)
            | AdvancedConfigHit::Scrollbar
            | AdvancedConfigHit::Save
            | AdvancedConfigHit::Cancel
            | AdvancedConfigHit::None => None,
        }
    }

    pub fn tooltip(&self) -> Option<StartupTooltip> {
        self.tooltip_at(self.pointer?)
    }

    pub fn handle_pointer_move(&mut self, point: GuiPoint) -> Vec<AdvancedConfigAction> {
        self.handle_pointer_move_inner(point, None)
    }

    pub fn handle_pointer_move_with_font(
        &mut self,
        point: GuiPoint,
        font: &clonk_graphics::clonk_font::ClonkFont,
    ) -> Vec<AdvancedConfigAction> {
        self.handle_pointer_move_inner(point, Some(font))
    }

    fn handle_pointer_move_inner(
        &mut self,
        point: GuiPoint,
        font: Option<&clonk_graphics::clonk_font::ClonkFont>,
    ) -> Vec<AdvancedConfigAction> {
        self.pointer = Some(point);
        if let Some(drag) = self.caption_drag {
            self.dialog_offset = (
                drag.offset.0 + (point.x - drag.pointer.x) as i32,
                drag.offset.1 + (point.y - drag.pointer.y) as i32,
            );
            return Vec::new();
        }
        if let Some(offset) = self.scrollbar_drag_offset {
            self.set_scroll_from_thumb(point.y as i32 - offset);
        }
        let caret = self.active_edit_caret_at(point.x, font);
        if let (Some(active), Some(caret)) = (self.active_edit.as_mut(), caret) {
            active.editor.drag_pointer_selection(caret);
        }
        Vec::new()
    }

    pub fn handle_pointer_down(&mut self, point: GuiPoint) -> Vec<AdvancedConfigAction> {
        self.handle_pointer_down_inner(point, None)
    }

    pub fn handle_pointer_down_with_font(
        &mut self,
        point: GuiPoint,
        font: &clonk_graphics::clonk_font::ClonkFont,
    ) -> Vec<AdvancedConfigAction> {
        self.handle_pointer_down_inner(point, Some(font))
    }

    fn handle_pointer_down_inner(
        &mut self,
        point: GuiPoint,
        font: Option<&clonk_graphics::clonk_font::ClonkFont>,
    ) -> Vec<AdvancedConfigAction> {
        self.pointer = Some(point);
        let hit = self.hit_test(point);
        match hit {
            AdvancedConfigHit::Close => {
                self.finish_edit();
                self.focus = AdvancedConfigFocus::Close;
                self.pointer_pressed = Some(PressTarget::Close);
                self.sound_events.push(AdvancedConfigSound::ArrowHit);
            }
            AdvancedConfigHit::Caption => {
                self.finish_edit();
                self.pointer_pressed = None;
                self.scrollbar_drag_offset = None;
                self.caption_drag = Some(CaptionDrag {
                    pointer: point,
                    offset: self.dialog_offset,
                });
            }
            AdvancedConfigHit::Section(index) => {
                self.focus = AdvancedConfigFocus::SectionTabs;
                self.select_section(index);
            }
            AdvancedConfigHit::Checkbox(index) => {
                self.finish_edit();
                self.focus = AdvancedConfigFocus::Row(index);
                if self
                    .row(index)
                    .is_some_and(|row| matches!(row.value, AdvancedConfigValue::Bool(_)))
                {
                    self.pointer_pressed = Some(PressTarget::Checkbox(index));
                }
            }
            AdvancedConfigHit::Edit(index) => {
                self.focus = AdvancedConfigFocus::Row(index);
                self.begin_edit(index);
                let caret = self.active_edit_caret_at(point.x, font);
                if let (Some(active), Some(caret)) = (self.active_edit.as_mut(), caret) {
                    active.editor.begin_pointer_selection(caret);
                }
            }
            AdvancedConfigHit::Decrement(index) => {
                self.focus = AdvancedConfigFocus::Row(index);
                self.pointer_pressed = Some(PressTarget::Decrement(index));
                self.step_value_and_keep_focus(index, -1);
                self.sound_events.push(AdvancedConfigSound::ArrowHit);
            }
            AdvancedConfigHit::Increment(index) => {
                self.focus = AdvancedConfigFocus::Row(index);
                self.pointer_pressed = Some(PressTarget::Increment(index));
                self.step_value_and_keep_focus(index, 1);
                self.sound_events.push(AdvancedConfigSound::ArrowHit);
            }
            AdvancedConfigHit::Row(index) => {
                self.finish_edit();
                self.focus = AdvancedConfigFocus::Row(index);
            }
            AdvancedConfigHit::Scrollbar => {
                self.finish_edit();
                let layout = self.layout();
                if rect_contains(layout.scrollbar_thumb, point) {
                    self.scrollbar_drag_offset = Some(point.y as i32 - layout.scrollbar_thumb.y);
                } else {
                    self.set_scroll_from_thumb(point.y as i32 - layout.scrollbar_thumb.h / 2);
                }
            }
            AdvancedConfigHit::Save => {
                self.finish_edit();
                self.focus = AdvancedConfigFocus::Save;
                self.pointer_pressed = Some(PressTarget::Save);
                self.sound_events.push(AdvancedConfigSound::ArrowHit);
            }
            AdvancedConfigHit::Cancel => {
                self.finish_edit();
                self.focus = AdvancedConfigFocus::Cancel;
                self.pointer_pressed = Some(PressTarget::Cancel);
                self.sound_events.push(AdvancedConfigSound::ArrowHit);
            }
            AdvancedConfigHit::None => self.finish_edit(),
        }
        Vec::new()
    }

    pub fn handle_pointer_up(&mut self, point: GuiPoint) -> Vec<AdvancedConfigAction> {
        self.handle_pointer_up_inner(point, None)
    }

    pub fn handle_pointer_up_with_font(
        &mut self,
        point: GuiPoint,
        font: &clonk_graphics::clonk_font::ClonkFont,
    ) -> Vec<AdvancedConfigAction> {
        self.handle_pointer_up_inner(point, Some(font))
    }

    fn handle_pointer_up_inner(
        &mut self,
        point: GuiPoint,
        font: Option<&clonk_graphics::clonk_font::ClonkFont>,
    ) -> Vec<AdvancedConfigAction> {
        self.pointer = Some(point);
        self.scrollbar_drag_offset = None;
        if let Some(drag) = self.caption_drag.take() {
            self.dialog_offset = (
                drag.offset.0 + (point.x - drag.pointer.x) as i32,
                drag.offset.1 + (point.y - drag.pointer.y) as i32,
            );
            return Vec::new();
        }
        let caret = self.active_edit_caret_at(point.x, font);
        if let (Some(active), Some(caret)) = (self.active_edit.as_mut(), caret) {
            active.editor.end_pointer_selection(caret);
        }
        let hit = self.hit_test(point);
        let Some(pressed) = self.pointer_pressed.take() else {
            return Vec::new();
        };
        if matches!(
            pressed,
            PressTarget::Decrement(_) | PressTarget::Increment(_)
        ) {
            self.sound_events.push(AdvancedConfigSound::ArrowHit);
            return Vec::new();
        }
        if !press_matches_hit(pressed, hit) {
            return Vec::new();
        }
        self.activate_press(pressed)
    }

    pub fn pointer_left(&mut self) {
        self.pointer = None;
        self.pointer_pressed = None;
        self.scrollbar_drag_offset = None;
        self.caption_drag = None;
        if let Some(active) = self.active_edit.as_mut() {
            active.editor.cancel_pointer_selection();
        }
    }

    /// Scrolls under the last pointer position and reports whether the scroll
    /// offset changed. Hosts update that position through
    /// [`Self::handle_pointer_move`] before forwarding native wheel input.
    pub fn handle_wheel(&mut self, delta: i32) -> bool {
        let Some(point) = self.pointer else {
            return false;
        };
        if delta == 0 {
            return false;
        }
        let layout = self.layout();
        if rect_contains(layout.settings_list, point) {
            let before = self.scroll_y;
            self.scroll_by(delta.saturating_neg());
            return self.scroll_y != before;
        }
        false
    }

    pub fn scroll_by(&mut self, amount: i32) {
        let max = self.max_scroll();
        self.scroll_y = self.scroll_y.saturating_add(amount).clamp(0, max);
        self.remember_scroll();
    }

    /// Inserts printable text into the focused text/integer row. A newly
    /// focused edit begins with its complete value selected, matching the
    /// classic inline edit behavior.
    pub fn handle_text_input(&mut self, text: &str) -> bool {
        let Some(active) = self.active_edit.as_ref() else {
            return false;
        };
        let Some(value) = self
            .sections
            .get(active.section)
            .and_then(|section| section.rows.get(active.row))
            .map(|row| &row.value)
        else {
            return false;
        };
        let is_integer = matches!(value, AdvancedConfigValue::Integer { .. });
        let signed = matches!(value, AdvancedConfigValue::Integer { min, .. } if *min < 0);
        let mut insert = String::new();
        if is_integer {
            let selection = active.editor.selection_range();
            let insertion_point = selection
                .as_ref()
                .map_or(active.editor.caret(), |range| range.start);
            let selection_covers_minus = selection
                .is_some_and(|range| range.start == 0 && active.editor.text().starts_with('-'));
            let mut can_insert_minus = signed
                && insertion_point == 0
                && (!active.editor.text().starts_with('-') || selection_covers_minus);
            for character in text.chars() {
                if character.is_ascii_digit()
                    || (character == '-' && can_insert_minus && insert.is_empty())
                {
                    insert.push(character);
                    can_insert_minus = false;
                }
            }
        } else {
            for mut character in text.chars() {
                if character.is_control() {
                    continue;
                }
                if character == '|' {
                    character = '¦';
                }
                if insert.len() + character.len_utf8() > MAX_EDIT_BYTES {
                    break;
                }
                insert.push(character);
            }
        }
        if insert.is_empty() {
            if is_integer && text.chars().any(|character| !character.is_control()) {
                let changed = self
                    .active_edit
                    .as_mut()
                    .is_some_and(|active| active.editor.delete_selection());
                if changed {
                    self.sync_active_edit_value(false);
                }
                return changed;
            }
            return false;
        }
        let changed = self
            .active_edit
            .as_mut()
            .is_some_and(|active| active.editor.insert_text(&insert));
        if changed {
            self.sync_active_edit_value(false);
        }
        changed
    }

    pub fn handle_backspace(&mut self) -> bool {
        self.handle_backspace_with_modifiers(false, false)
    }

    pub fn handle_backspace_with_modifiers(&mut self, ctrl: bool, shift: bool) -> bool {
        let changed = self
            .active_edit
            .as_mut()
            .is_some_and(|active| active.editor.backspace(ctrl, shift));
        if changed {
            self.sync_active_edit_value(false);
        }
        changed
    }

    pub fn handle_delete(&mut self, ctrl: bool, shift: bool) -> bool {
        let changed = self
            .active_edit
            .as_mut()
            .is_some_and(|active| active.editor.delete(ctrl, shift));
        if changed {
            self.sync_active_edit_value(false);
        }
        changed
    }

    pub fn move_edit_cursor(
        &mut self,
        operation: RenameEditCursorOperation,
        ctrl: bool,
        shift: bool,
    ) -> bool {
        let Some(active) = self.active_edit.as_mut() else {
            return false;
        };
        active.editor.move_cursor(operation, ctrl, shift);
        true
    }

    pub fn select_all_edit_text(&mut self) -> bool {
        let Some(active) = self.active_edit.as_mut() else {
            return false;
        };
        active.editor.select_all();
        true
    }

    pub fn selected_edit_text(&self) -> Option<&str> {
        self.active_edit.as_ref()?.editor.selected_text()
    }

    /// Dialog-level Alt mnemonics invoke the button immediately without
    /// moving focus or manufacturing a key-down/key-up press pair.
    pub fn handle_hotkey(&mut self, character: char) -> Vec<AdvancedConfigAction> {
        let character = character.to_ascii_uppercase();
        let action = if expand_hotkey_markup(&self.labels.save).1 == Some(character) {
            Some(AdvancedConfigAction::Save)
        } else if expand_hotkey_markup(&self.labels.cancel).1 == Some(character) {
            Some(AdvancedConfigAction::Cancel)
        } else {
            None
        };
        if action.is_some() {
            self.finish_edit();
        }
        action.into_iter().collect()
    }

    pub fn delete_edit_selection(&mut self) -> bool {
        let changed = self
            .active_edit
            .as_mut()
            .is_some_and(|active| active.editor.delete_selection());
        if changed {
            self.sync_active_edit_value(false);
        }
        changed
    }

    pub fn handle_key_down(&mut self, key: KeyCode) -> Vec<AdvancedConfigAction> {
        match key {
            KeyCode::Escape => {
                self.finish_edit();
                self.key_pressed = None;
                vec![AdvancedConfigAction::Cancel]
            }
            // C4StartupOptionsAdvancedConfigDialog::OnEnter returns false:
            // Enter never accepts the dialog globally, but a focused Button
            // consumes it at control priority just like Space.
            KeyCode::Enter => {
                if matches!(
                    self.focus,
                    AdvancedConfigFocus::Save
                        | AdvancedConfigFocus::Cancel
                        | AdvancedConfigFocus::Close
                ) {
                    self.begin_keyboard_press();
                } else {
                    self.commit_active_edit();
                }
                Vec::new()
            }
            KeyCode::Tab => {
                self.handle_focus_step(false);
                Vec::new()
            }
            KeyCode::Space => {
                self.begin_keyboard_press();
                Vec::new()
            }
            KeyCode::Up => {
                self.handle_direction(-1);
                Vec::new()
            }
            KeyCode::Down => {
                self.handle_direction(1);
                Vec::new()
            }
            KeyCode::Left => {
                self.move_edit_cursor(RenameEditCursorOperation::Left, false, false);
                Vec::new()
            }
            KeyCode::Right => {
                self.move_edit_cursor(RenameEditCursorOperation::Right, false, false);
                Vec::new()
            }
            KeyCode::Home | KeyCode::End | KeyCode::PageUp | KeyCode::PageDown => Vec::new(),
        }
    }

    pub fn handle_key_up(&mut self, key: KeyCode) -> Vec<AdvancedConfigAction> {
        if !matches!(key, KeyCode::Space | KeyCode::Enter) {
            return Vec::new();
        }
        let Some(pressed) = self.key_pressed.take() else {
            return Vec::new();
        };
        if focus_for_press(pressed) != self.focus {
            return Vec::new();
        }
        self.activate_press(pressed)
    }

    pub fn handle_focus_step(&mut self, backwards: bool) {
        self.finish_edit();
        self.advance_focus(backwards);
    }

    pub fn handle_integer_page_step(&mut self, delta: i128) -> bool {
        let AdvancedConfigFocus::Row(index) = self.focus else {
            return false;
        };
        if !self
            .row(index)
            .is_some_and(|row| matches!(row.value, AdvancedConfigValue::Integer { .. }))
        {
            return false;
        }
        let before = self.row(index).map(|row| row.value.serialized());
        self.step_value_and_keep_focus(index, delta);
        before != self.row(index).map(|row| row.value.serialized())
    }

    pub fn cancel_interaction(&mut self) {
        self.pointer = None;
        self.pointer_pressed = None;
        self.key_pressed = None;
        self.scrollbar_drag_offset = None;
        self.caption_drag = None;
        if let Some(active) = self.active_edit.as_mut() {
            active.editor.cancel_pointer_selection();
        }
    }

    fn row(&self, index: usize) -> Option<&AdvancedConfigRow> {
        self.current_section()?.rows.get(index)
    }

    fn row_mut(&mut self, index: usize) -> Option<&mut AdvancedConfigRow> {
        self.sections
            .get_mut(self.current_section)?
            .rows
            .get_mut(index)
    }

    fn begin_edit(&mut self, row: usize) {
        self.finish_edit();
        let Some(value) = self.row(row).map(|row| &row.value) else {
            return;
        };
        let text = match value {
            AdvancedConfigValue::Integer { value, .. } => value.to_string(),
            AdvancedConfigValue::Text(value) => value.clone(),
            AdvancedConfigValue::Bool(_)
            | AdvancedConfigValue::Choice { .. }
            | AdvancedConfigValue::ReadOnly(_) => return,
        };
        self.active_edit = Some(ActiveEdit {
            section: self.current_section,
            row,
            editor: RenameEdit::new(text, None),
        });
    }

    fn finish_edit(&mut self) {
        self.sync_active_edit_value(true);
        self.active_edit = None;
    }

    /// Runs the edit control's Enter/finish-input hook without moving focus.
    /// Plain text stays untouched; spinboxes rewrite their text to the parsed,
    /// clamped value just like `C4GUI::SpinBox::OnFinishInput`.
    fn commit_active_edit(&mut self) {
        let Some((section, row)) = self
            .active_edit
            .as_ref()
            .map(|edit| (edit.section, edit.row))
        else {
            return;
        };
        self.sync_active_edit_value(true);
        let normalized_integer = self
            .sections
            .get(section)
            .and_then(|section| section.rows.get(row))
            .and_then(|row| match &row.value {
                AdvancedConfigValue::Integer { .. } => Some(row.value.serialized()),
                _ => None,
            });
        if let (Some(active), Some(normalized)) = (self.active_edit.as_mut(), normalized_integer) {
            active.editor.set_text(normalized);
        }
    }

    fn sync_active_edit_value(&mut self, finalize: bool) {
        let Some(edit) = self.active_edit.as_ref() else {
            return;
        };
        let (section, row, text) = (edit.section, edit.row, edit.editor.text().to_string());
        let Some(value) = self
            .sections
            .get_mut(section)
            .and_then(|section| section.rows.get_mut(row))
            .map(|row| &mut row.value)
        else {
            return;
        };
        match value {
            AdvancedConfigValue::Integer { value, min, max } => {
                let parsed = match text.parse::<i128>() {
                    Ok(parsed) => Some(parsed),
                    Err(_) if !finalize => None,
                    Err(_) if !text.bytes().any(|byte| byte.is_ascii_digit()) => Some(0),
                    Err(_) if text.starts_with('-') => Some(i128::MIN),
                    Err(_) => Some(i128::MAX),
                };
                if let Some(parsed) = parsed {
                    *value = parsed.clamp(*min, *max);
                }
            }
            AdvancedConfigValue::Text(value) => *value = text,
            AdvancedConfigValue::Bool(_)
            | AdvancedConfigValue::Choice { .. }
            | AdvancedConfigValue::ReadOnly(_) => {}
        }
    }

    fn active_edit_text(&self, row: usize) -> Option<&str> {
        self.active_edit
            .as_ref()
            .filter(|edit| edit.section == self.current_section && edit.row == row)
            .map(|edit| edit.editor.text())
    }

    fn active_edit_caret_at(
        &self,
        x: f32,
        font: Option<&clonk_graphics::clonk_font::ClonkFont>,
    ) -> Option<usize> {
        let active = self.active_edit.as_ref()?;
        let edit = self.layout().rows.get(active.row)?.edit?;
        Some(match font {
            Some(font) => active.editor.character_at_x(x, edit, font),
            None => character_position_at(active.editor.text(), edit, x),
        })
    }

    fn activate_press(&mut self, pressed: PressTarget) -> Vec<AdvancedConfigAction> {
        match pressed {
            PressTarget::Close | PressTarget::Cancel => {
                self.sound_events.push(AdvancedConfigSound::Click);
                vec![AdvancedConfigAction::Cancel]
            }
            PressTarget::Save => {
                self.sound_events.push(AdvancedConfigSound::Click);
                vec![AdvancedConfigAction::Save]
            }
            PressTarget::Checkbox(index) => {
                if let Some(AdvancedConfigRow {
                    value: AdvancedConfigValue::Bool(value),
                    ..
                }) = self.row_mut(index)
                {
                    *value = !*value;
                    self.sound_events.push(AdvancedConfigSound::ArrowHit);
                }
                Vec::new()
            }
            PressTarget::Decrement(index) => {
                self.step_value(index, -1);
                Vec::new()
            }
            PressTarget::Increment(index) => {
                self.step_value(index, 1);
                Vec::new()
            }
        }
    }

    fn step_value(&mut self, index: usize, delta: i128) {
        let Some(row) = self.row_mut(index) else {
            return;
        };
        match &mut row.value {
            AdvancedConfigValue::Integer { value, min, max } => {
                *value = value.saturating_add(delta).clamp(*min, *max);
            }
            AdvancedConfigValue::Choice { value, choices } if !choices.is_empty() => {
                let current = choices.iter().position(|choice| choice.value == *value);
                let next = match current {
                    Some(current) if delta < 0 => (current + choices.len() - 1) % choices.len(),
                    Some(current) => (current + 1) % choices.len(),
                    None if delta < 0 => choices.len() - 1,
                    None => 0,
                };
                value.clone_from(&choices[next].value);
            }
            AdvancedConfigValue::Bool(_)
            | AdvancedConfigValue::Choice { .. }
            | AdvancedConfigValue::Text(_)
            | AdvancedConfigValue::ReadOnly(_) => {}
        }
    }

    fn step_value_and_keep_focus(&mut self, index: usize, delta: i128) {
        self.sync_active_edit_value(true);
        self.step_value(index, delta);
        if self.focus != AdvancedConfigFocus::Row(index) {
            return;
        }
        let Some(text) = self.row(index).and_then(|row| match &row.value {
            AdvancedConfigValue::Integer { .. } => Some(row.value.serialized()),
            _ => None,
        }) else {
            return;
        };
        let current_section = self.current_section;
        if let Some(active) = self
            .active_edit
            .as_mut()
            .filter(|active| active.section == current_section && active.row == index)
        {
            active.editor.set_text(text);
            return;
        }
        let mut editor = RenameEdit::new(text.clone(), None);
        editor.set_text(text);
        self.active_edit = Some(ActiveEdit {
            section: current_section,
            row: index,
            editor,
        });
    }

    fn begin_keyboard_press(&mut self) {
        if let AdvancedConfigFocus::Row(index) = self.focus {
            if let Some(AdvancedConfigRow {
                value: AdvancedConfigValue::Bool(value),
                ..
            }) = self.row_mut(index)
            {
                *value = !*value;
                self.key_pressed = None;
                self.sound_events.push(AdvancedConfigSound::ArrowHit);
                return;
            }
        }
        let pressed = match self.focus {
            AdvancedConfigFocus::Save => Some(PressTarget::Save),
            AdvancedConfigFocus::Cancel => Some(PressTarget::Cancel),
            AdvancedConfigFocus::Close => Some(PressTarget::Close),
            AdvancedConfigFocus::SectionTabs | AdvancedConfigFocus::Row(_) => None,
        };
        self.key_pressed = pressed;
        if pressed.is_some() {
            self.sound_events.push(AdvancedConfigSound::ArrowHit);
        }
    }

    fn handle_direction(&mut self, direction: i32) {
        match self.focus {
            AdvancedConfigFocus::SectionTabs => {
                if self.sections.is_empty() {
                    return;
                }
                let next = if direction < 0 {
                    (self.current_section + self.sections.len() - 1) % self.sections.len()
                } else {
                    (self.current_section + 1) % self.sections.len()
                };
                self.select_section(next);
            }
            AdvancedConfigFocus::Row(index)
                if self.row(index).is_some_and(|row| {
                    matches!(
                        row.value,
                        AdvancedConfigValue::Integer { .. } | AdvancedConfigValue::Choice { .. }
                    )
                }) =>
            {
                self.step_value_and_keep_focus(index, -i128::from(direction));
            }
            AdvancedConfigFocus::Row(_) => {}
            AdvancedConfigFocus::Save
            | AdvancedConfigFocus::Cancel
            | AdvancedConfigFocus::Close => {}
        }
    }

    fn advance_focus(&mut self, backwards: bool) {
        let focusable_rows = self
            .current_section()
            .map(|section| {
                section
                    .rows
                    .iter()
                    .enumerate()
                    .filter_map(|(index, row)| row.value.is_editable().then_some(index))
                    .collect::<Vec<_>>()
            })
            .unwrap_or_default();
        let mut order = Vec::with_capacity(focusable_rows.len() + 4);
        order.push(AdvancedConfigFocus::SectionTabs);
        order.extend(focusable_rows.into_iter().map(AdvancedConfigFocus::Row));
        order.extend([
            AdvancedConfigFocus::Save,
            AdvancedConfigFocus::Cancel,
            AdvancedConfigFocus::Close,
        ]);
        let current = order
            .iter()
            .position(|focus| *focus == self.focus)
            .unwrap_or(0);
        let next = if backwards {
            (current + order.len() - 1) % order.len()
        } else {
            (current + 1) % order.len()
        };
        self.focus = order[next];
        if let AdvancedConfigFocus::Row(index) = self.focus {
            self.ensure_row_visible(index);
            self.begin_edit(index);
        }
    }

    fn ensure_row_visible(&mut self, index: usize) {
        let layout = self.layout();
        let top = index as i32 * ROW_PITCH;
        let bottom = top + ROW_HEIGHT;
        if top < self.scroll_y {
            self.scroll_y = top;
        } else if bottom > self.scroll_y + layout.list_client.h {
            self.scroll_y = bottom - layout.list_client.h;
        }
        self.clamp_scroll();
    }

    fn max_scroll(&self) -> i32 {
        let layout = self.layout();
        max_scroll_for(self.current_section(), &layout)
    }

    fn clamp_scroll(&mut self) {
        let max = self.max_scroll();
        self.scroll_y = self.scroll_y.clamp(0, max);
        self.remember_scroll();
    }

    fn remember_scroll(&mut self) {
        if let Some(scroll) = self.section_scroll_y.get_mut(self.current_section) {
            *scroll = self.scroll_y;
        }
    }

    fn set_scroll_from_thumb(&mut self, thumb_top: i32) {
        let layout = self.layout();
        let travel = (layout.scrollbar.h - layout.scrollbar_thumb.h).max(0);
        let pin = (thumb_top - layout.scrollbar.y).clamp(0, travel);
        let max_scroll = max_scroll_for(self.current_section(), &layout);
        self.scroll_y = if travel == 0 {
            0
        } else {
            max_scroll * pin / travel
        };
        self.remember_scroll();
    }

    fn caption_scroll_offset_at(&self, now: Instant, font: &ClonkFont) -> i32 {
        if self.labels.caption.is_empty() {
            return 0;
        }
        let layout = self.layout();
        let max_scroll =
            (font.measure(&self.labels.caption, true).0 + TITLE_LEFT_INDENT + TITLE_RIGHT_INDENT
                - layout.caption.w)
                .max(0);
        advance_caption_scroll(&self.caption_scroll, now, max_scroll, TITLE_SCROLL_DELAY)
    }
}

/// Owned image bundle used by [`AdvancedConfigScreen`].
pub struct AdvancedConfigAssets {
    pub caption: ImageData,
    pub button: ImageData,
    pub button_down: ImageData,
    pub button_highlight: ImageData,
    pub checkbox: ImageData,
}

impl AdvancedConfigAssets {
    pub fn validate(&self) -> Result<()> {
        let skin = ClassicGuiSkin::new(
            &self.caption,
            &self.button,
            &self.button_down,
            Some(&self.button_highlight),
        );
        skin.validate_message_dialog_assets()?;
        ensure!(
            self.checkbox.height() > 0
                && self.checkbox.width() >= self.checkbox.height().saturating_mul(2),
            "GUICheckbox.png must contain unchecked and checked phases, got {}x{}",
            self.checkbox.width(),
            self.checkbox.height()
        );
        Ok(())
    }
}

/// Classic renderer for the advanced configuration controller.
pub struct AdvancedConfigScreen;

impl AdvancedConfigScreen {
    pub fn render(
        surface: &mut Surface,
        assets: &AdvancedConfigAssets,
        fonts: &ClonkFontSet,
        controller: &mut AdvancedConfigController,
        active: bool,
        gamma: Option<&GammaRamp>,
    ) -> Result<()> {
        Self::render_at(
            surface,
            assets,
            fonts,
            controller,
            active,
            gamma,
            Instant::now(),
        )
    }

    pub fn render_at(
        surface: &mut Surface,
        assets: &AdvancedConfigAssets,
        fonts: &ClonkFontSet,
        controller: &mut AdvancedConfigController,
        active: bool,
        gamma: Option<&GammaRamp>,
        now: Instant,
    ) -> Result<()> {
        assets.validate()?;
        let layout = controller.layout();
        let skin = ClassicGuiSkin::new(
            &assets.caption,
            &assets.button,
            &assets.button_down,
            Some(&assets.button_highlight),
        );
        skin.draw_dialog(surface, layout.bounds, gamma);
        skin.draw_caption_scrolled(
            surface,
            layout.caption,
            &controller.labels.caption,
            &fonts.text,
            [255, 255, 255, 255],
            TextAlign::Left,
            TITLE_RIGHT_INDENT,
            controller.caption_scroll_offset_at(now, &fonts.text),
            gamma,
        );

        skin.draw_button(
            surface,
            layout.close_button,
            "X",
            fonts,
            controller.button_state(PressTarget::Close, active),
            gamma,
        );

        draw_engine_box(
            surface,
            layout.section_list.x,
            layout.section_list.y,
            layout.section_list.x + layout.section_list.w - 1,
            layout.section_list.y + layout.section_list.h - 1,
            0x7f00_0000,
            gamma,
        );
        draw_3d_frame(surface, layout.section_list, gamma);
        for (index, rect) in layout.section_tabs.iter().copied().enumerate() {
            if index == controller.current_section {
                draw_engine_box(
                    surface,
                    rect.x + 1,
                    rect.y + 1,
                    rect.x + rect.w - 2,
                    rect.y + rect.h - 2,
                    0x7f77_2200,
                    gamma,
                );
            }
            draw_engine_frame(
                surface,
                rect.x,
                rect.y,
                rect.x + rect.w - 1,
                rect.y + rect.h - 1,
                if index == controller.current_section {
                    0x5fff_cc00
                } else {
                    0xaf77_4422
                },
                gamma,
            );
            if let Some(section) = controller.sections.get(index) {
                draw_clipped_text(
                    surface,
                    &fonts.text,
                    rect.x + 5,
                    rect.y + (rect.h - fonts.text.line_height) / 2,
                    &section.name,
                    if index == controller.current_section {
                        [255, 255, 0, 255]
                    } else {
                        [255, 255, 255, 255]
                    },
                    TextAlign::Left,
                    gamma,
                    IntRect::new(rect.x + 3, rect.y, (rect.w - 6).max(0), rect.h),
                );
            }
        }

        draw_engine_box(
            surface,
            layout.settings_list.x,
            layout.settings_list.y,
            layout.settings_list.x + layout.settings_list.w - 1,
            layout.settings_list.y + layout.settings_list.h - 1,
            0x7f00_0000,
            gamma,
        );
        draw_3d_frame(surface, layout.settings_list, gamma);
        Self::render_rows(surface, assets, fonts, controller, &layout, active, gamma)?;
        Self::render_scrollbar(surface, controller, &layout, gamma);

        skin.draw_button(
            surface,
            layout.save_button,
            &controller.labels.save,
            fonts,
            controller.button_state(PressTarget::Save, active),
            gamma,
        );
        skin.draw_button(
            surface,
            layout.cancel_button,
            &controller.labels.cancel,
            fonts,
            controller.button_state(PressTarget::Cancel, active),
            gamma,
        );
        Ok(())
    }

    #[allow(clippy::too_many_arguments)]
    fn render_rows(
        surface: &mut Surface,
        assets: &AdvancedConfigAssets,
        fonts: &ClonkFontSet,
        controller: &mut AdvancedConfigController,
        layout: &AdvancedConfigLayout,
        active: bool,
        gamma: Option<&GammaRamp>,
    ) -> Result<()> {
        let saved_clip = surface.clip();
        if layout.list_client.w > 0 && layout.list_client.h > 0 {
            surface.set_clip(Rect::new(
                layout.list_client.x,
                layout.list_client.y,
                layout.list_client.w as u32,
                layout.list_client.h as u32,
            ));
        }
        if controller.current_section().is_some() {
            let current_section_index = controller.current_section;
            for row_layout in &layout.rows {
                if row_layout.bounds.y >= layout.list_client.y + layout.list_client.h
                    || row_layout.bounds.y + row_layout.bounds.h <= layout.list_client.y
                {
                    continue;
                }
                let Some(row) = controller
                    .current_section()
                    .and_then(|section| section.rows.get(row_layout.index))
                    .cloned()
                else {
                    continue;
                };
                if active && controller.focus == AdvancedConfigFocus::Row(row_layout.index) {
                    draw_engine_box(
                        surface,
                        row_layout.bounds.x,
                        row_layout.bounds.y,
                        row_layout.bounds.x + row_layout.bounds.w - 1,
                        row_layout.bounds.y + row_layout.bounds.h - 1,
                        0xaf55_3300,
                        gamma,
                    );
                }
                draw_clipped_text(
                    surface,
                    &fonts.text,
                    row_layout.label.x,
                    row_layout.label.y + (row_layout.label.h - fonts.text.line_height) / 2,
                    &row.name,
                    [255, 255, 255, 255],
                    TextAlign::Left,
                    gamma,
                    intersect_rect(row_layout.label, layout.list_client),
                );
                match &row.value {
                    AdvancedConfigValue::Bool(checked) => {
                        if let Some(rect) = row_layout.checkbox {
                            draw_checkbox(surface, rect, *checked, &assets.checkbox, gamma)?;
                        }
                    }
                    AdvancedConfigValue::Integer { value, .. } => {
                        if let Some(edit) = row_layout.edit {
                            let rendered_active = active
                                && controller.active_edit.as_mut().is_some_and(|active_edit| {
                                    if active_edit.section == current_section_index
                                        && active_edit.row == row_layout.index
                                    {
                                        active_edit.editor.render(
                                            surface,
                                            &fonts.text,
                                            edit,
                                            gamma,
                                        );
                                        true
                                    } else {
                                        false
                                    }
                                });
                            if !rendered_active {
                                draw_edit_box(
                                    surface,
                                    edit,
                                    controller.active_edit_text(row_layout.index).unwrap_or(""),
                                    if controller.active_edit_text(row_layout.index).is_some() {
                                        None
                                    } else {
                                        Some(value.to_string())
                                    },
                                    &fonts.text,
                                    [255, 255, 255, 255],
                                    gamma,
                                );
                            }
                        }
                        Self::render_step_buttons(
                            surface, fonts, controller, row_layout, active, gamma,
                        );
                    }
                    AdvancedConfigValue::Choice { .. } => {
                        if let Some(edit) = row_layout.edit {
                            let display = row.value.display_text();
                            draw_edit_box(
                                surface,
                                edit,
                                display.as_ref(),
                                None,
                                &fonts.text,
                                [255, 255, 255, 255],
                                gamma,
                            );
                        }
                        Self::render_step_buttons(
                            surface, fonts, controller, row_layout, active, gamma,
                        );
                    }
                    AdvancedConfigValue::Text(value) => {
                        if let Some(edit) = row_layout.edit {
                            let rendered_active = active
                                && controller.active_edit.as_mut().is_some_and(|active_edit| {
                                    if active_edit.section == current_section_index
                                        && active_edit.row == row_layout.index
                                    {
                                        active_edit.editor.render(
                                            surface,
                                            &fonts.text,
                                            edit,
                                            gamma,
                                        );
                                        true
                                    } else {
                                        false
                                    }
                                });
                            if !rendered_active {
                                draw_edit_box(
                                    surface,
                                    edit,
                                    controller
                                        .active_edit_text(row_layout.index)
                                        .unwrap_or(value),
                                    None,
                                    &fonts.text,
                                    [255, 255, 255, 255],
                                    gamma,
                                );
                            }
                        }
                    }
                    AdvancedConfigValue::ReadOnly(value) => {
                        draw_clipped_text(
                            surface,
                            &fonts.text,
                            row_layout.control.x,
                            row_layout.control.y
                                + (row_layout.control.h - fonts.text.line_height) / 2,
                            value,
                            [255, 255, 255, 255],
                            TextAlign::Left,
                            gamma,
                            intersect_rect(row_layout.control, layout.list_client),
                        );
                    }
                }
            }
        }
        match saved_clip {
            Some(clip) => surface.set_clip(clip),
            None => surface.clear_clip(),
        }
        Ok(())
    }

    fn render_step_buttons(
        surface: &mut Surface,
        fonts: &ClonkFontSet,
        controller: &AdvancedConfigController,
        row: &AdvancedConfigRowLayout,
        active: bool,
        gamma: Option<&GammaRamp>,
    ) {
        for (rect, label, target) in [
            (row.decrement_button, "-", PressTarget::Decrement(row.index)),
            (row.increment_button, "+", PressTarget::Increment(row.index)),
        ] {
            let Some(rect) = rect else { continue };
            let state = controller.button_state(target, active);
            draw_engine_box(
                surface,
                rect.x,
                rect.y,
                rect.x + rect.w - 1,
                rect.y + rect.h - 1,
                if state.pressed {
                    0x5f33_1100
                } else {
                    0x7f66_3311
                },
                gamma,
            );
            draw_3d_frame(surface, rect, gamma);
            fonts.text.draw_with_gamma(
                surface,
                rect.x + rect.w / 2 + i32::from(state.pressed),
                rect.y + (rect.h - fonts.text.line_height) / 2 + i32::from(state.pressed),
                label,
                [255, 255, 0, 255],
                TextAlign::Center,
                false,
                gamma,
            );
        }
    }

    fn render_scrollbar(
        surface: &mut Surface,
        controller: &AdvancedConfigController,
        layout: &AdvancedConfigLayout,
        gamma: Option<&GammaRamp>,
    ) {
        draw_engine_box(
            surface,
            layout.scrollbar.x,
            layout.scrollbar.y,
            layout.scrollbar.x + layout.scrollbar.w - 1,
            layout.scrollbar.y + layout.scrollbar.h - 1,
            0xaf22_1100,
            gamma,
        );
        draw_engine_frame(
            surface,
            layout.scrollbar.x,
            layout.scrollbar.y,
            layout.scrollbar.x + layout.scrollbar.w - 1,
            layout.scrollbar.y + layout.scrollbar.h - 1,
            0x7faa_6622,
            gamma,
        );
        let hovered = controller
            .pointer
            .is_some_and(|point| rect_contains(layout.scrollbar_thumb, point));
        draw_engine_box(
            surface,
            layout.scrollbar_thumb.x + 1,
            layout.scrollbar_thumb.y + 1,
            layout.scrollbar_thumb.x + layout.scrollbar_thumb.w - 2,
            layout.scrollbar_thumb.y + layout.scrollbar_thumb.h - 2,
            if hovered { 0x5faa_6622 } else { 0x7f77_4411 },
            gamma,
        );
        draw_3d_frame(surface, layout.scrollbar_thumb, gamma);
    }
}

impl AdvancedConfigController {
    fn button_state(&self, target: PressTarget, active: bool) -> ClassicButtonState {
        let hovered = self
            .pointer
            .is_some_and(|point| press_matches_hit(target, self.hit_test(point)));
        ClassicButtonState {
            pressed: active
                && (self.key_pressed == Some(target)
                    || self.pointer_pressed == Some(target) && hovered),
            highlighted: active && (hovered || self.focus == focus_for_press(target)),
        }
    }
}

/// Computes the centered three-quarter-screen modal and the active section's
/// public row geometry.
pub fn advanced_config_layout(
    screen_width: i32,
    screen_height: i32,
    sections: &[AdvancedConfigSection],
    current_section: usize,
    scroll_y: i32,
) -> AdvancedConfigLayout {
    advanced_config_layout_with_offset(
        screen_width,
        screen_height,
        sections,
        current_section,
        scroll_y,
        (0, 0),
    )
}

fn advanced_config_layout_with_offset(
    screen_width: i32,
    screen_height: i32,
    sections: &[AdvancedConfigSection],
    current_section: usize,
    scroll_y: i32,
    dialog_offset: (i32, i32),
) -> AdvancedConfigLayout {
    let screen_width = screen_width.max(1);
    let screen_height = screen_height.max(1);
    let width = (screen_width * 3 / 4).max(1);
    let height = (screen_height * 3 / 4).max(1);
    let bounds = IntRect::new(
        (screen_width - width) / 2 + dialog_offset.0,
        (screen_height - height) / 2 + dialog_offset.1,
        width,
        height,
    );
    let caption = IntRect::new(
        bounds.x + 2,
        bounds.y + 2,
        (bounds.w - 4).max(1),
        CAPTION_HEIGHT.min((bounds.h - 4).max(1)),
    );
    let close_size = 16.min((caption.w - 8).max(1)).min((caption.h - 8).max(1));
    let close_button = IntRect::new(
        caption.x + caption.w - close_size - 4,
        caption.y + 4,
        close_size,
        close_size,
    );
    let client = IntRect::new(
        bounds.x + OUTER_MARGIN,
        caption.y + caption.h + OUTER_MARGIN,
        (bounds.w - OUTER_MARGIN * 2).max(1),
        (bounds.y + bounds.h - OUTER_MARGIN - (caption.y + caption.h + OUTER_MARGIN)).max(1),
    );
    let button_y = client.y + client.h - BUTTON_HEIGHT;
    let list_height = (button_y - BUTTON_GAP - client.y).max(1);
    let section_width = (client.w / 4)
        .clamp(1, 180)
        .min((client.w - TAB_GAP - 1).max(1));
    let section_list = IntRect::new(client.x, client.y, section_width, list_height);
    let settings_list = IntRect::new(
        section_list.x + section_list.w + TAB_GAP,
        client.y,
        (client.x + client.w - (section_list.x + section_list.w + TAB_GAP)).max(1),
        list_height,
    );
    let scrollbar = IntRect::new(
        settings_list.x + settings_list.w - SCROLLBAR_WIDTH - 2,
        settings_list.y + 2,
        SCROLLBAR_WIDTH.min((settings_list.w - 4).max(1)),
        (settings_list.h - 4).max(1),
    );
    let list_client = IntRect::new(
        settings_list.x + 3,
        settings_list.y + 3,
        (scrollbar.x - 3 - (settings_list.x + 3)).max(1),
        (settings_list.h - 6).max(1),
    );
    let section_pitch = if sections.is_empty() {
        TAB_HEIGHT
    } else {
        ((section_list.h - 4).max(1) / sections.len() as i32).clamp(1, TAB_HEIGHT)
    };
    let section_tabs = sections
        .iter()
        .enumerate()
        .map(|(index, _)| {
            IntRect::new(
                section_list.x + 2,
                section_list.y + 2 + index as i32 * section_pitch,
                (section_list.w - 4).max(1),
                (section_pitch - 1).max(1),
            )
        })
        .collect();
    let rows = sections
        .get(current_section)
        .map(|section| {
            section
                .rows
                .iter()
                .enumerate()
                .map(|(index, row)| row_layout(index, row, list_client, scroll_y))
                .collect()
        })
        .unwrap_or_default();
    let content_height = sections
        .get(current_section)
        .map_or(0, |section| section.rows.len() as i32 * ROW_PITCH);
    let maximum_scroll = (content_height - list_client.h).max(0);
    let thumb_height = if maximum_scroll == 0 {
        scrollbar.h
    } else {
        (scrollbar.h * list_client.h / content_height.max(1)).clamp(18, scrollbar.h)
    };
    let travel = (scrollbar.h - thumb_height).max(0);
    let thumb_y = scrollbar.y
        + if maximum_scroll == 0 {
            0
        } else {
            travel * scroll_y.clamp(0, maximum_scroll) / maximum_scroll
        };
    let scrollbar_thumb = IntRect::new(scrollbar.x, thumb_y, scrollbar.w, thumb_height);
    let available_buttons = settings_list.w.max(1);
    let button_width = ((available_buttons - BUTTON_GAP) / 2)
        .max(1)
        .min(MAX_BUTTON_WIDTH);
    let buttons_width = button_width * 2 + BUTTON_GAP;
    let button_x = settings_list.x + (settings_list.w - buttons_width) / 2;
    let save_button = IntRect::new(
        button_x,
        button_y,
        button_width,
        BUTTON_HEIGHT.min(client.h.max(1)),
    );
    let cancel_button = save_button.with_x(button_x + button_width + BUTTON_GAP);
    AdvancedConfigLayout {
        bounds,
        caption,
        close_button,
        client,
        section_list,
        section_tabs,
        settings_list,
        list_client,
        scrollbar,
        scrollbar_thumb,
        rows,
        save_button,
        cancel_button,
        row_height: ROW_HEIGHT,
        row_pitch: ROW_PITCH,
    }
}

fn row_layout(
    index: usize,
    row: &AdvancedConfigRow,
    list_client: IntRect,
    scroll_y: i32,
) -> AdvancedConfigRowLayout {
    let bounds = IntRect::new(
        list_client.x,
        list_client.y + index as i32 * ROW_PITCH - scroll_y,
        list_client.w,
        ROW_HEIGHT,
    );
    let label_width = (bounds.w * 42 / 100).max(1);
    let label = IntRect::new(bounds.x + 3, bounds.y, (label_width - 6).max(1), bounds.h);
    let control = IntRect::new(
        bounds.x + label_width,
        bounds.y + 2,
        (bounds.w - label_width - 3).max(1),
        (bounds.h - 4).max(1),
    );
    let checkbox = matches!(row.value, AdvancedConfigValue::Bool(_)).then(|| {
        let size = control.h.min(control.w).max(1);
        IntRect::new(control.x, control.y, size, size)
    });
    let has_step_buttons = matches!(row.value, AdvancedConfigValue::Integer { .. })
        || matches!(
            row.value,
            AdvancedConfigValue::Choice {
                ref choices,
                ..
            } if !choices.is_empty()
        );
    let is_edit = matches!(
        row.value,
        AdvancedConfigValue::Integer { .. }
            | AdvancedConfigValue::Choice { .. }
            | AdvancedConfigValue::Text(_)
    );
    let step_width = if has_step_buttons {
        control.h.min(20)
    } else {
        0
    };
    let upper_step_height = (control.h / 2).max(1);
    let decrement_button = has_step_buttons.then_some(IntRect::new(
        control.x + control.w - step_width,
        control.y + upper_step_height,
        step_width,
        (control.h - upper_step_height).max(1),
    ));
    let increment_button = has_step_buttons.then_some(IntRect::new(
        control.x + control.w - step_width,
        control.y,
        step_width,
        upper_step_height,
    ));
    let edit = is_edit.then_some(IntRect::new(
        control.x,
        control.y,
        (control.w - step_width).max(1),
        control.h,
    ));
    AdvancedConfigRowLayout {
        index,
        bounds,
        label,
        control,
        checkbox,
        edit,
        decrement_button,
        increment_button,
    }
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
    ensure!(
        cell > 0 && (phase + 1) * cell <= sheet.width(),
        "GUICheckbox.png does not contain enabled phase {phase}"
    );
    draw_facet_stretch(
        surface,
        sheet,
        ((phase * cell) as f32, 0.0, cell as f32, cell as f32),
        (rect.x as f32, rect.y as f32, rect.w as f32, rect.h as f32),
        gamma,
    );
    Ok(())
}

#[allow(clippy::too_many_arguments)]
fn draw_edit_box(
    surface: &mut Surface,
    rect: IntRect,
    text: &str,
    owned_text: Option<String>,
    font: &clonk_graphics::clonk_font::ClonkFont,
    color: [u8; 4],
    gamma: Option<&GammaRamp>,
) {
    draw_engine_box(
        surface,
        rect.x,
        rect.y,
        rect.x + rect.w - 1,
        rect.y + rect.h - 1,
        0x7f00_0000,
        gamma,
    );
    draw_3d_frame(surface, rect, gamma);
    let display = owned_text.as_deref().unwrap_or(text);
    draw_clipped_text(
        surface,
        font,
        rect.x + 3,
        rect.y + (rect.h - font.line_height) / 2,
        display,
        color,
        TextAlign::Left,
        gamma,
        IntRect::new(
            rect.x + 2,
            rect.y + 1,
            (rect.w - 4).max(0),
            (rect.h - 2).max(0),
        ),
    );
}

fn max_scroll_for(section: Option<&AdvancedConfigSection>, layout: &AdvancedConfigLayout) -> i32 {
    let content_height = section.map_or(0, |section| section.rows.len() as i32 * ROW_PITCH);
    (content_height - layout.list_client.h).max(0)
}

fn press_matches_hit(press: PressTarget, hit: AdvancedConfigHit) -> bool {
    matches!(
        (press, hit),
        (PressTarget::Close, AdvancedConfigHit::Close)
            | (PressTarget::Save, AdvancedConfigHit::Save)
            | (PressTarget::Cancel, AdvancedConfigHit::Cancel)
    ) || match (press, hit) {
        (PressTarget::Checkbox(left), AdvancedConfigHit::Checkbox(right))
        | (PressTarget::Decrement(left), AdvancedConfigHit::Decrement(right))
        | (PressTarget::Increment(left), AdvancedConfigHit::Increment(right)) => left == right,
        _ => false,
    }
}

fn focus_for_press(press: PressTarget) -> AdvancedConfigFocus {
    match press {
        PressTarget::Close => AdvancedConfigFocus::Close,
        PressTarget::Save => AdvancedConfigFocus::Save,
        PressTarget::Cancel => AdvancedConfigFocus::Cancel,
        PressTarget::Checkbox(index)
        | PressTarget::Decrement(index)
        | PressTarget::Increment(index) => AdvancedConfigFocus::Row(index),
    }
}

fn rect_contains(rect: IntRect, point: GuiPoint) -> bool {
    point.x >= rect.x as f32
        && point.y >= rect.y as f32
        && point.x < (rect.x + rect.w) as f32
        && point.y < (rect.y + rect.h) as f32
}

fn character_position_at(text: &str, rect: IntRect, x: f32) -> usize {
    let character_count = text.chars().count();
    if character_count == 0 {
        return 0;
    }
    let client_width = (rect.w - 4).max(1) as f32;
    let relative = (x - (rect.x + 2) as f32).clamp(0.0, client_width);
    let target = (relative * character_count as f32 / client_width).round() as usize;
    text.char_indices()
        .nth(target)
        .map_or(text.len(), |(index, _)| index)
}

fn intersect_rect(left: IntRect, right: IntRect) -> IntRect {
    let x1 = left.x.max(right.x);
    let y1 = left.y.max(right.y);
    let x2 = (left.x + left.w).min(right.x + right.w);
    let y2 = (left.y + left.h).min(right.y + right.h);
    IntRect::new(x1, y1, (x2 - x1).max(0), (y2 - y1).max(0))
}

fn truncate_utf8(mut value: String, maximum: usize) -> String {
    if value.len() <= maximum {
        return value;
    }
    let mut end = maximum;
    while !value.is_char_boundary(end) {
        end -= 1;
    }
    value.truncate(end);
    value
}

#[cfg(test)]
mod tests {
    use super::*;
    use clonk_graphics::Color;

    fn controller() -> AdvancedConfigController {
        AdvancedConfigController::new(vec![
            AdvancedConfigSection::new(
                "General",
                vec![
                    AdvancedConfigRow::new("Enabled", AdvancedConfigValue::Bool(false)),
                    AdvancedConfigRow::new(
                        "Retries",
                        AdvancedConfigValue::Integer {
                            value: 2,
                            min: -2,
                            max: 5,
                        },
                    ),
                    AdvancedConfigRow::new("PlayerName", AdvancedConfigValue::Text("Clonk".into())),
                    AdvancedConfigRow::new("Version", AdvancedConfigValue::ReadOnly("9.0".into())),
                ],
            ),
            AdvancedConfigSection::new(
                "Network",
                vec![AdvancedConfigRow::new(
                    "Port",
                    AdvancedConfigValue::Integer {
                        value: 11111,
                        min: 1,
                        max: 65535,
                    },
                )],
            ),
        ])
    }

    fn center(rect: IntRect) -> GuiPoint {
        GuiPoint::new((rect.x + rect.w / 2) as f32, (rect.y + rect.h / 2) as f32)
    }

    fn unit_width_font(character: char) -> ClonkFont {
        let mut font = ClonkFont::new(3);
        font.h_space = 0;
        font.add_glyph(
            character,
            clonk_graphics::clonk_font::GlyphCell {
                width: 1,
                pixels: vec![Color::opaque(255, 255, 255); 4],
            },
        );
        font
    }

    #[test]
    fn alt_mnemonics_activate_on_the_translated_letter() {
        // `handle_hotkey` resolves the letter out of the caption rather than
        // from a constant, so a translated caption moves the binding with it.
        // C++ reads the same `&` out of the resource string
        // (7d43b47b src/C4Gui.cpp:47), which is why fifteen shipped keys can
        // nominate a different letter in German than in English.
        let mut state = AdvancedConfigController::new(vec![AdvancedConfigSection::new(
            "General",
            vec![AdvancedConfigRow::new(
                "FPS",
                AdvancedConfigValue::Bool(false),
            )],
        )]);

        state.set_labels(AdvancedConfigLabels {
            caption: "Advanced settings".into(),
            save: "&Save".into(),
            cancel: "&Cancel".into(),
        });
        assert_eq!(state.handle_hotkey('s'), vec![AdvancedConfigAction::Save]);
        assert_eq!(state.handle_hotkey('c'), vec![AdvancedConfigAction::Cancel]);
        assert!(
            state.handle_hotkey('p').is_empty(),
            "an unmarked letter binds nothing"
        );

        // Translated: the marked letters move, and the English ones stop
        // working — which is exactly what a hardcoded letter would fail.
        state.set_labels(AdvancedConfigLabels {
            caption: "Erweiterte Einstellungen".into(),
            save: "S&peichern".into(),
            cancel: "Abbre&chen".into(),
        });
        assert_eq!(state.handle_hotkey('p'), vec![AdvancedConfigAction::Save]);
        assert_eq!(state.handle_hotkey('c'), vec![AdvancedConfigAction::Cancel]);
        assert!(
            state.handle_hotkey('s').is_empty(),
            "the English Save letter must not still activate a German caption"
        );
    }

    #[test]
    fn advanced_config_sections_are_dynamic_and_selectable() {
        let mut state = controller();
        state.resize(800, 600);
        assert_eq!(state.layout().bounds, IntRect::new(100, 75, 600, 450));
        assert_eq!(state.sections().len(), 2);
        assert_eq!(
            state.current_section().map(|section| section.name.as_str()),
            Some("General")
        );
        assert!(state.select_section(1));
        assert_eq!(state.current_section_index(), 1);
        assert_eq!(
            state.current_section().map(|section| section.name.as_str()),
            Some("Network")
        );
        assert!(!state.select_section(99));
    }

    #[test]
    fn advanced_config_set_value_clamps_and_read_only_is_inert() {
        let mut state = controller();
        assert!(state.set_value("General", "Enabled", AdvancedConfigValue::Bool(true)));
        assert!(state.set_value(
            "General",
            "Retries",
            AdvancedConfigValue::Integer {
                value: 99,
                min: i128::MIN,
                max: i128::MAX,
            },
        ));
        assert!(state.set_value(
            "General",
            "PlayerName",
            AdvancedConfigValue::Text("New name".into()),
        ));
        assert!(!state.set_value(
            "General",
            "Version",
            AdvancedConfigValue::ReadOnly("10.0".into()),
        ));
        assert_eq!(
            state.value("General", "Retries"),
            Some(&AdvancedConfigValue::Integer {
                value: 5,
                min: -2,
                max: 5
            })
        );
        assert_eq!(
            state.changes(),
            vec![
                AdvancedConfigChange {
                    section: "General".into(),
                    key: "Enabled".into(),
                    value: "1".into()
                },
                AdvancedConfigChange {
                    section: "General".into(),
                    key: "Retries".into(),
                    value: "5".into()
                },
                AdvancedConfigChange {
                    section: "General".into(),
                    key: "PlayerName".into(),
                    value: "New name".into()
                },
            ]
        );
    }

    #[test]
    fn advanced_config_choice_cycles_by_stable_value_and_serializes_value_not_label() {
        let choices = vec![
            AdvancedConfigChoice {
                value: "".into(),
                label: "System default".into(),
            },
            AdvancedConfigChoice {
                value: "coreaudio:built-in".into(),
                label: "Shared microphone".into(),
            },
            AdvancedConfigChoice {
                value: "coreaudio:usb".into(),
                label: "Shared microphone".into(),
            },
        ];
        let mut state = AdvancedConfigController::new(vec![AdvancedConfigSection::new(
            "Voice",
            vec![AdvancedConfigRow::new(
                "InputDevice",
                AdvancedConfigValue::Choice {
                    value: "coreaudio:built-in".into(),
                    choices,
                },
            )],
        )]);
        state.resize(800, 600);

        let value = state.value("Voice", "InputDevice").expect("choice");
        assert_eq!(value.serialized(), "coreaudio:built-in");
        assert_eq!(value.display_text(), "Shared microphone");

        let edit = state.layout().rows[0].edit.expect("choice display");
        state.handle_pointer_down(center(edit));
        assert!(!state.handle_text_input("not a device id"));
        assert_eq!(
            state
                .value("Voice", "InputDevice")
                .expect("choice")
                .serialized(),
            "coreaudio:built-in",
        );

        let increment = state.layout().rows[0]
            .increment_button
            .expect("choice increment arrow");
        state.handle_pointer_down(center(increment));
        state.handle_pointer_up(center(increment));
        let value = state.value("Voice", "InputDevice").expect("choice");
        assert_eq!(value.serialized(), "coreaudio:usb");
        assert_eq!(value.display_text(), "Shared microphone");
        assert_eq!(
            state.changes(),
            vec![AdvancedConfigChange {
                section: "Voice".into(),
                key: "InputDevice".into(),
                value: "coreaudio:usb".into(),
            }],
            "the stable value, never the duplicate display label, is persisted",
        );

        state.handle_key_down(KeyCode::Up);
        assert_eq!(
            state
                .value("Voice", "InputDevice")
                .expect("choice")
                .serialized(),
            "",
            "Up cycles forward and wraps",
        );
        state.handle_key_down(KeyCode::Down);
        assert_eq!(
            state
                .value("Voice", "InputDevice")
                .expect("choice")
                .serialized(),
            "coreaudio:usb",
            "Down cycles backward and wraps",
        );
    }

    #[test]
    fn advanced_config_choice_preserves_unknown_value_until_it_is_stepped() {
        let mut state = AdvancedConfigController::new(vec![AdvancedConfigSection::new(
            "Voice",
            vec![AdvancedConfigRow::new(
                "InputDevice",
                AdvancedConfigValue::Choice {
                    value: "coreaudio:missing".into(),
                    choices: vec![
                        AdvancedConfigChoice {
                            value: "".into(),
                            label: "System default".into(),
                        },
                        AdvancedConfigChoice {
                            value: "coreaudio:usb".into(),
                            label: "USB microphone".into(),
                        },
                    ],
                },
            )],
        )]);
        state.resize(800, 600);

        let value = state.value("Voice", "InputDevice").expect("choice");
        assert_eq!(value.serialized(), "coreaudio:missing");
        assert_eq!(value.display_text(), "coreaudio:missing");
        assert!(state.changes().is_empty());
        assert!(!state.set_value(
            "Voice",
            "InputDevice",
            AdvancedConfigValue::Choice {
                value: "coreaudio:other-missing".into(),
                choices: Vec::new(),
            },
        ));

        let increment = state.layout().rows[0]
            .increment_button
            .expect("choice increment arrow");
        state.handle_pointer_down(center(increment));
        assert_eq!(
            state
                .value("Voice", "InputDevice")
                .expect("choice")
                .serialized(),
            "",
            "stepping forward from an unknown value selects the first choice",
        );
        assert!(state.set_value(
            "Voice",
            "InputDevice",
            AdvancedConfigValue::Choice {
                value: "coreaudio:usb".into(),
                choices: Vec::new(),
            },
        ));
        let value = state.value("Voice", "InputDevice").expect("choice");
        assert_eq!(value.serialized(), "coreaudio:usb");
        assert_eq!(value.display_text(), "USB microphone");
        assert!(matches!(
            value,
            AdvancedConfigValue::Choice { choices, .. } if choices.len() == 2
        ));
    }

    #[test]
    fn advanced_config_pointer_edits_bool_text_and_integer() {
        let mut state = controller();
        state.resize(800, 600);
        let checkbox = state.layout().rows[0].checkbox.expect("checkbox");
        state.handle_pointer_down(center(checkbox));
        state.handle_pointer_up(center(checkbox));
        assert_eq!(
            state.value("General", "Enabled"),
            Some(&AdvancedConfigValue::Bool(true))
        );

        let integer = state.layout().rows[1].edit.expect("integer edit");
        state.handle_pointer_down(center(integer));
        assert!(state.select_all_edit_text());
        assert!(state.handle_text_input("-99 ignored"));
        state.handle_key_down(KeyCode::Enter);
        assert_eq!(
            state.value("General", "Retries"),
            Some(&AdvancedConfigValue::Integer {
                value: -2,
                min: -2,
                max: 5
            })
        );

        let text = state.layout().rows[2].edit.expect("text edit");
        state.handle_pointer_down(center(text));
        assert!(state.select_all_edit_text());
        assert!(state.handle_text_input("Ada"));
        assert!(state.handle_backspace());
        assert_eq!(
            state.value("General", "PlayerName"),
            Some(&AdvancedConfigValue::Text("Ad".into()))
        );
    }

    #[test]
    fn advanced_config_integer_edit_normalizes_empty_and_overflow_on_finish() {
        let mut state = controller();
        state.resize(800, 600);

        let retries = state.layout().rows[1].edit.expect("Retries edit");
        state.handle_pointer_down(center(retries));
        assert!(state.select_all_edit_text());
        assert!(state.handle_backspace());
        state.handle_key_down(KeyCode::Enter);
        assert_eq!(
            state.value("General", "Retries"),
            Some(&AdvancedConfigValue::Integer {
                value: 0,
                min: -2,
                max: 5,
            })
        );
        state.handle_key_down(KeyCode::Up);
        assert_eq!(state.value("General", "Retries").unwrap().serialized(), "1");
        state.handle_key_down(KeyCode::Down);
        assert_eq!(state.value("General", "Retries").unwrap().serialized(), "0");
        assert!(state.handle_integer_page_step(10));
        assert_eq!(state.value("General", "Retries").unwrap().serialized(), "5");
        assert!(state.handle_integer_page_step(-10));
        assert_eq!(
            state.value("General", "Retries").unwrap().serialized(),
            "-2"
        );

        state.handle_pointer_down(center(retries));
        assert!(state.select_all_edit_text());
        assert!(state.handle_text_input(&"9".repeat(200)));
        state.handle_key_down(KeyCode::Enter);
        assert_eq!(state.value("General", "Retries").unwrap().serialized(), "5");

        state.handle_pointer_down(center(retries));
        assert!(state.select_all_edit_text());
        assert!(state.handle_text_input(&format!("-{}", "9".repeat(200))));
        state.handle_key_down(KeyCode::Enter);
        assert_eq!(
            state.value("General", "Retries").unwrap().serialized(),
            "-2"
        );

        assert!(state.select_section_named("Network"));
        let port = state.layout().rows[0].edit.expect("Port edit");
        state.handle_pointer_down(center(port));
        assert!(state.select_all_edit_text());
        assert!(state.handle_backspace());
        state.handle_key_down(KeyCode::Enter);
        assert_eq!(state.value("Network", "Port").unwrap().serialized(), "1");
    }

    #[test]
    fn advanced_config_enter_never_saves_but_buttons_and_escape_emit_actions() {
        let mut state = controller();
        state.resize(800, 600);
        assert!(state.handle_key_down(KeyCode::Enter).is_empty());
        assert!(state.handle_key_up(KeyCode::Enter).is_empty());

        let save = center(state.layout().save_button);
        state.handle_pointer_down(save);
        assert_eq!(
            state.handle_pointer_up(save),
            vec![AdvancedConfigAction::Save]
        );

        let cancel = center(state.layout().cancel_button);
        state.handle_pointer_down(cancel);
        assert_eq!(
            state.handle_pointer_up(cancel),
            vec![AdvancedConfigAction::Cancel]
        );
        assert_eq!(
            state.handle_key_down(KeyCode::Escape),
            vec![AdvancedConfigAction::Cancel]
        );

        state.set_labels(AdvancedConfigLabels {
            caption: "Advanced".into(),
            save: "&Save".into(),
            cancel: "&Cancel".into(),
        });
        assert_eq!(state.handle_hotkey('s'), vec![AdvancedConfigAction::Save]);
        assert_eq!(state.handle_hotkey('C'), vec![AdvancedConfigAction::Cancel]);
        assert!(state.handle_hotkey('X').is_empty());
    }

    #[test]
    fn advanced_config_checkbox_keyboard_toggle_happens_once_on_key_down() {
        let mut state = controller();
        state.handle_focus_step(false);
        assert_eq!(state.focus(), AdvancedConfigFocus::Row(0));
        assert!(state.handle_key_down(KeyCode::Space).is_empty());
        assert_eq!(
            state.value("General", "Enabled"),
            Some(&AdvancedConfigValue::Bool(true))
        );
        assert_eq!(
            state.take_sound_events(),
            vec![AdvancedConfigSound::ArrowHit]
        );
        assert!(state.handle_key_up(KeyCode::Space).is_empty());
        assert_eq!(
            state.value("General", "Enabled"),
            Some(&AdvancedConfigValue::Bool(true))
        );
        assert!(state.take_sound_events().is_empty());
    }

    #[test]
    fn advanced_config_focus_editing_and_button_enter_follow_native_priority() {
        let mut state = controller();
        state.resize(800, 600);
        state.handle_focus_step(false);
        assert_eq!(state.focus(), AdvancedConfigFocus::Row(0));
        state.handle_focus_step(false);
        assert_eq!(state.focus(), AdvancedConfigFocus::Row(1));
        assert!(state.handle_text_input("4"));
        assert_eq!(state.value("General", "Retries").unwrap().serialized(), "4");
        state.handle_focus_step(false);
        assert_eq!(state.focus(), AdvancedConfigFocus::Row(2));
        state.handle_focus_step(false);
        assert_eq!(
            state.focus(),
            AdvancedConfigFocus::Save,
            "read-only labels are absent from the native tab order"
        );
        assert!(state.handle_key_down(KeyCode::Enter).is_empty());
        assert_eq!(
            state.handle_key_up(KeyCode::Enter),
            vec![AdvancedConfigAction::Save]
        );

        state.handle_focus_step(true);
        assert_eq!(state.focus(), AdvancedConfigFocus::Row(2));

        let edit = state.layout().rows[2].edit.expect("PlayerName edit");
        state.handle_pointer_down(GuiPoint::new(
            (edit.x + edit.w - 3) as f32,
            (edit.y + edit.h / 2) as f32,
        ));
        assert!(state.handle_text_input("!"));
        assert_eq!(
            state.value("General", "PlayerName"),
            Some(&AdvancedConfigValue::Text("Clonk!".into()))
        );
        assert!(state.move_edit_cursor(RenameEditCursorOperation::Home, false, false));
        assert!(state.handle_text_input("A"));
        assert!(state.move_edit_cursor(RenameEditCursorOperation::Right, false, false));
        assert!(state.handle_delete(false, false));
        assert_eq!(
            state.value("General", "PlayerName"),
            Some(&AdvancedConfigValue::Text("AConk!".into()))
        );

        state.handle_key_down(KeyCode::Enter);
        assert!(state.handle_text_input("?"));
        assert_eq!(
            state.value("General", "PlayerName"),
            Some(&AdvancedConfigValue::Text("AC?onk!".into())),
            "Enter commits an edit without taking focus or moving the caret"
        );
    }

    #[test]
    fn advanced_config_selected_signed_integer_accepts_a_replacement_minus() {
        let mut state = controller();
        state.resize(800, 600);
        let retries = state.layout().rows[1].edit.expect("Retries edit");
        state.handle_pointer_down(center(retries));
        assert!(state.select_all_edit_text());
        assert!(state.handle_text_input("-"));
        assert!(state.handle_text_input("5"));
        state.handle_key_down(KeyCode::Enter);
        assert_eq!(
            state.value("General", "Retries").unwrap().serialized(),
            "-2"
        );
    }

    #[test]
    fn advanced_config_invalid_spin_text_still_replaces_the_selection() {
        let mut state = controller();
        state.resize(800, 600);
        let retries = state.layout().rows[1].edit.expect("Retries edit");
        state.handle_pointer_down(center(retries));
        assert!(state.select_all_edit_text());
        assert!(state.handle_text_input("a"));
        state.handle_key_down(KeyCode::Enter);
        assert_eq!(state.value("General", "Retries").unwrap().serialized(), "0");
    }

    #[test]
    fn advanced_config_capture_cancel_keeps_the_focused_edit_alive() {
        let mut state = controller();
        state.resize(800, 600);
        let player_name = state.layout().rows[2].edit.expect("PlayerName edit");
        state.handle_pointer_down(center(player_name));
        assert!(state.select_all_edit_text());
        assert!(state.handle_text_input("A"));
        state.cancel_interaction();
        assert!(state.handle_text_input("B"));
        assert_eq!(
            state.value("General", "PlayerName"),
            Some(&AdvancedConfigValue::Text("AB".into()))
        );
    }

    #[test]
    fn advanced_config_spin_arrows_sound_on_both_mouse_edges_but_not_keys() {
        let mut state = controller();
        state.resize(800, 600);
        assert!(state.set_value(
            "General",
            "Retries",
            AdvancedConfigValue::Integer {
                value: 5,
                min: i128::MIN,
                max: i128::MAX,
            },
        ));
        let increment = state.layout().rows[1]
            .increment_button
            .expect("increment arrow");
        state.handle_pointer_down(center(increment));
        state.handle_pointer_up(center(increment));
        assert_eq!(
            state.take_sound_events(),
            vec![AdvancedConfigSound::ArrowHit, AdvancedConfigSound::ArrowHit],
            "mouse press/release sounds even when the value is already bounded"
        );
        state.handle_key_down(KeyCode::Up);
        assert!(state.take_sound_events().is_empty());
        assert!(state.select_all_edit_text());
        assert!(state.handle_text_input("4"));
        assert_eq!(state.value("General", "Retries").unwrap().serialized(), "4");
    }

    #[test]
    fn advanced_config_wheel_scrolls_long_sections_and_switching_resets_it() {
        let mut rows = vec![AdvancedConfigRow::new(
            "Spin",
            AdvancedConfigValue::Integer {
                value: 2,
                min: 0,
                max: 5,
            },
        )];
        rows.extend((0..40).map(|index| {
            AdvancedConfigRow::new(
                format!("Value{index}"),
                AdvancedConfigValue::Text(index.to_string()),
            )
        }));
        let mut state = AdvancedConfigController::new(vec![
            AdvancedConfigSection::new("Long", rows),
            AdvancedConfigSection::new("Short", Vec::new()),
        ]);
        state.resize(640, 480);

        let spin = center(state.layout().rows[0].edit.expect("integer edit"));
        state.handle_pointer_move(spin);
        assert!(state.handle_wheel(-60));
        assert_eq!(state.value("Long", "Spin").unwrap().serialized(), "2");
        assert_eq!(
            state.scroll_y(),
            60,
            "the containing ScrollWindow owns wheel input over spinboxes"
        );

        let point = center(state.layout().list_client);
        state.handle_pointer_move(point);
        assert!(state.handle_wheel(-7));
        assert_eq!(state.scroll_y(), 67, "pixel-wheel magnitude is preserved");
        assert!(state.handle_wheel(5));
        assert_eq!(state.scroll_y(), 62);
        let first_y = state.layout().rows[0].bounds.y;
        assert!(first_y < state.layout().list_client.y);
        state.select_section(1);
        assert_eq!(state.scroll_y(), 0);
        state.select_section(0);
        assert_eq!(
            state.scroll_y(),
            62,
            "each native ListBox retains its offset"
        );
    }

    #[test]
    fn advanced_config_caption_drag_moves_live_and_release_persists_offset() {
        let mut state = controller();
        state.resize(800, 600);
        let centered = advanced_config_layout(800, 600, state.sections(), 0, 0);
        let caption = state.layout().caption;
        let start = GuiPoint::new((caption.x + 10) as f32, (caption.y + 10) as f32);
        assert_eq!(state.hit_test(start), AdvancedConfigHit::Caption);

        state.handle_pointer_down(start);
        assert!(state.has_positional_pointer_drag());
        let moved = GuiPoint::new(start.x + 37.0, start.y - 14.0);
        state.handle_pointer_move(moved);
        assert_eq!(state.dialog_offset(), (37, -14));
        assert_eq!(state.layout().bounds.x, centered.bounds.x + 37);
        assert_eq!(state.layout().bounds.y, centered.bounds.y - 14);

        let released = GuiPoint::new(moved.x + 5.0, moved.y + 6.0);
        assert!(state.handle_pointer_up(released).is_empty());
        assert_eq!(state.dialog_offset(), (42, -8));
        assert!(!state.has_positional_pointer_drag());
        state.handle_pointer_move(GuiPoint::new(0.0, 0.0));
        assert_eq!(state.dialog_offset(), (42, -8));

        state.resize(1000, 700);
        let resized_center = advanced_config_layout(1000, 700, state.sections(), 0, 0);
        assert_eq!(state.layout().bounds.x, resized_center.bounds.x + 42);
        assert_eq!(state.layout().bounds.y, resized_center.bounds.y - 8);

        let second_caption = state.layout().caption;
        let second_start = GuiPoint::new(
            (second_caption.x + 10) as f32,
            (second_caption.y + 10) as f32,
        );
        state.handle_pointer_down(second_start);
        state.handle_pointer_move(GuiPoint::new(second_start.x + 3.0, second_start.y + 4.0));
        assert_eq!(state.dialog_offset(), (45, -4));
        state.cancel_interaction();
        state.handle_pointer_move(GuiPoint::new(second_start.x + 30.0, second_start.y + 40.0));
        assert_eq!(state.dialog_offset(), (45, -4));
    }

    #[test]
    fn advanced_config_caption_autoscroll_advances_per_frame_and_dwells_at_ends() {
        let mut state = controller();
        state.resize(800, 600);
        let font = unit_width_font('W');
        let layout = state.layout();
        let caption_width =
            (layout.caption.w - TITLE_LEFT_INDENT - TITLE_RIGHT_INDENT + 3) as usize;
        state.set_labels(AdvancedConfigLabels {
            caption: "W".repeat(caption_width),
            ..AdvancedConfigLabels::default()
        });
        let base = Instant::now();
        assert_eq!(state.caption_scroll_offset_at(base, &font), 0);
        assert_eq!(
            state.caption_scroll_offset_at(
                base + TITLE_SCROLL_DELAY - Duration::from_millis(1),
                &font,
            ),
            0
        );
        let outbound = base + TITLE_SCROLL_DELAY;
        assert_eq!(state.caption_scroll_offset_at(outbound, &font), 1);
        assert_eq!(state.caption_scroll_offset_at(outbound, &font), 2);
        assert_eq!(
            state.caption_scroll_offset_at(outbound, &font),
            2,
            "the attempted maximum frame backs off and begins the end dwell"
        );
        assert_eq!(
            state.caption_scroll_offset_at(
                outbound + TITLE_SCROLL_DELAY - Duration::from_millis(1),
                &font,
            ),
            2
        );
        let returning = outbound + TITLE_SCROLL_DELAY;
        assert_eq!(state.caption_scroll_offset_at(returning, &font), 1);
        assert_eq!(state.caption_scroll_offset_at(returning, &font), 0);
        assert_eq!(
            state.caption_scroll_offset_at(returning, &font),
            0,
            "the attempted negative frame backs off and begins the start dwell"
        );
        assert_eq!(
            state.caption_scroll_offset_at(returning + TITLE_SCROLL_DELAY, &font),
            1
        );
    }

    #[test]
    fn advanced_config_caption_and_close_expose_only_native_tooltips() {
        let mut state = controller();
        state.resize(800, 600);
        state.set_labels(AdvancedConfigLabels {
            caption: "Erweiterte Einstellungen".into(),
            ..AdvancedConfigLabels::default()
        });
        let layout = state.layout();
        assert_eq!(
            layout.close_button,
            IntRect::new(
                layout.caption.x + layout.caption.w - 20,
                layout.caption.y + 4,
                16,
                16
            ),
            "Dialog::SetTitle uses a 16px close button inset four pixels"
        );
        let title_point = GuiPoint::new(
            (layout.caption.x + 8) as f32,
            (layout.caption.y + layout.caption.h / 2) as f32,
        );
        let _ = state.handle_pointer_move(title_point);
        assert_eq!(
            state.tooltip_at(title_point),
            Some(StartupTooltip::text("Erweiterte Einstellungen"))
        );
        assert_eq!(
            state.tooltip_at(center(layout.close_button)),
            None,
            "an unrouted overlapping control cannot claim the shared timer"
        );
        let _ = state.handle_pointer_move(center(layout.close_button));
        assert_eq!(
            state.tooltip_at(center(layout.close_button)),
            Some(StartupTooltip::resource("IDS_MNU_CLOSE")),
            "the close control wins its overlap with the wooden caption"
        );
        let _ = state.handle_pointer_move(center(layout.save_button));
        assert_eq!(state.tooltip_at(center(layout.save_button)), None);

        let _ = state.handle_pointer_move(title_point);
        assert_eq!(
            state.tooltip(),
            Some(StartupTooltip::text("Erweiterte Einstellungen"))
        );
    }
}
