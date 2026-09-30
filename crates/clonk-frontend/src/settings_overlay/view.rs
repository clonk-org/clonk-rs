use super::*;
use crate::classic_gui::IntRect;
use crate::clonk_fonts::ClonkFontSet;
use crate::rename_edit::{RenameEdit, RenameEditCursorOperation};
use crate::startup_options_dlg::{BookFonts, OptionsBook, OptionsDlgLayout};
use crate::{GuiPoint, KeyCode};

mod choices;
mod microphone;
mod render;
const ROW_HEIGHT: i32 = 36;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum SettingsFocus {
    #[default]
    Search,
    Category(SettingsCategory),
    AudioPage(AudioPage),
    Row(usize),
    Modified,
    Advanced,
    Pin,
    PinUp,
    PinDown,
    Reset,
    ResetCategory,
    TestMicrophone,
    RecordMicrophone,
    CloseMicrophoneTest,
    RefreshDevices,
    Close,
}

/// How strongly a list row is marked: the keyboard focus, the row the footer
/// actions and description apply to, or the row under the pointer.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum RowEmphasis {
    Hovered,
    Selected,
    Focused,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum SettingsAction {
    Change(usize, AdvancedConfigValue),
    Close,
    CaptureBinding(usize),
    TestMicrophone,
    CancelMicrophoneTest,
    RefreshDevices,
    ConfirmDisplay(bool),
}

pub struct SettingsViewState {
    pub focus: SettingsFocus,
    pub selected: Option<usize>,
    pub scroll: usize,
    pub message: String,
    pub context: String,
    pub width: i32,
    pub height: i32,
    pub microphone_status: String,
    pub microphone_level: f32,
    pub microphone_test_open: bool,
    pub microphone_testing: bool,
    pub audio_device_status: String,
    pub display_confirmation: Option<u64>,
    pub reset_confirmation: bool,
    pub(crate) search_edit: RenameEdit<()>,
    pub(crate) edit: Option<(usize, RenameEdit<()>)>,
    pub(crate) pressed: Option<SettingsFocus>,
    pub(crate) choice: Option<choices::ChoicePicker>,
    pub(crate) dragging: Option<usize>,
    pub(crate) scroll_drag: Option<i32>,
    pub(crate) hovered: Option<usize>,
    layout: Option<SettingsLayout>,
}

impl Default for SettingsViewState {
    fn default() -> Self {
        Self {
            focus: SettingsFocus::Search,
            selected: None,
            scroll: 0,
            message: String::new(),
            context: String::new(),
            width: 800,
            height: 600,
            microphone_status: String::new(),
            microphone_level: 0.0,
            microphone_test_open: false,
            microphone_testing: false,
            audio_device_status: String::new(),
            display_confirmation: None,
            reset_confirmation: false,
            search_edit: RenameEdit::new("", None),
            edit: None,
            pressed: None,
            choice: None,
            dragging: None,
            scroll_drag: None,
            hovered: None,
            layout: None,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct SettingsLayout {
    pub panel: IntRect,
    pub search: IntRect,
    pub list: IntRect,
    pub footer: IntRect,
    pub tabs: [IntRect; 7],
    pub back: IntRect,
}

impl SettingsLayout {
    pub fn new(width: i32, height: i32) -> Self {
        let w = (width - 16).clamp(1, 1120);
        let h = (height - 16).clamp(1, 760);
        let panel = IntRect::new((width - w) / 2, (height - h) / 2, w, h);
        Self {
            search: IntRect::new(panel.x + 172, panel.y + 52, w - 184, 30),
            list: IntRect::new(panel.x + 172, panel.y + 118, w - 184, (h - 254).max(1)),
            footer: IntRect::new(panel.x + 12, panel.y + h - 132, w - 24, 120),
            tabs: std::array::from_fn(|i| {
                IntRect::new(panel.x + 12, panel.y + 52 + i as i32 * 36, 148, 32)
            }),
            back: IntRect::new(panel.x + w - 142, panel.y + h - 44, 130, 30),
            panel,
        }
    }

    fn from_book(book: &OptionsDlgLayout) -> Self {
        let sheet = book.sheet;
        let margin = if sheet.w < 500 { 8 } else { 32 };
        let x = (sheet.x + margin).max(book.tab_clips[0].0 + 128);
        let w = sheet.x + sheet.w - margin - x;
        let footer = IntRect::new(x, sheet.y + sheet.h - 102, w, 102);
        Self {
            panel: book.tabular,
            search: IntRect::new(x + 60, sheet.y + 2, w - 60, 26),
            list: IntRect::new(
                x,
                sheet.y + 62,
                w,
                (footer.y - sheet.y - 66).max(ROW_HEIGHT),
            ),
            footer,
            tabs: std::array::from_fn(|i| {
                IntRect::new(
                    book.tab_clips[i].0,
                    book.tab_clips[i].1,
                    85,
                    book.tab_height - 8,
                )
            }),
            back: book.back_button,
        }
    }
}

impl SettingsController {
    pub fn resize_book(&mut self, width: i32, height: i32, gui: &ClonkFontSet, book: &BookFonts) {
        self.resize(width, height);
        let layout = SettingsLayout::from_book(&OptionsBook::layout(width, height, gui, book));
        if self.view.layout != Some(layout) {
            self.view.layout = Some(layout);
            self.ensure_visible();
        }
    }

    pub fn layout(&self) -> SettingsLayout {
        let mut layout = self
            .view
            .layout
            .unwrap_or_else(|| SettingsLayout::new(self.view.width, self.view.height));
        if self.category == SettingsCategory::Audio {
            layout.footer.y += 16;
            layout.footer.h -= 16;
            layout.list.h += 16;
        }
        layout
    }

    pub fn resize(&mut self, width: i32, height: i32) {
        if self.view.width == width && self.view.height == height {
            return;
        }
        self.view.width = width;
        self.view.height = height;
        self.view.layout = None;
        self.ensure_visible();
    }

    pub fn select_category(&mut self, category: SettingsCategory) {
        self.view.microphone_test_open = false;
        self.view.choice = None;
        self.cancel_interaction();
        self.category = category;
        self.query.clear();
        self.view.search_edit.set_text("");
        self.view.edit = None;
        self.view.scroll = 0;
        self.view.selected = self.visible_indices().first().copied();
        self.view.focus = SettingsFocus::Category(category);
    }

    pub fn select_audio_page(&mut self, page: AudioPage) {
        self.audio_page = page;
        self.select_category(SettingsCategory::Audio);
        self.view.focus = SettingsFocus::AudioPage(page);
    }

    pub fn voice_page_selected(&self) -> bool {
        self.category == SettingsCategory::Audio && self.audio_page == AudioPage::Voice
    }

    pub fn editing(&self) -> bool {
        !self.view.microphone_test_open
            && (self.view.edit.is_some() || self.view.focus == SettingsFocus::Search)
    }

    pub fn accepts_text(&self) -> bool {
        self.editing() || self.view.choice.is_some()
    }

    pub fn focus_search(&mut self) {
        if self.view.microphone_test_open {
            return;
        }
        self.view.edit = None;
        self.view.choice = None;
        self.cancel_interaction();
        self.view.focus = SettingsFocus::Search;
        self.view.search_edit.set_text(self.query.clone());
        self.view.search_edit.select_all();
    }

    pub fn text(&mut self, text: &str) {
        if self.view.microphone_test_open {
            return;
        }
        if self.view.choice.is_some() {
            self.choice_type(text);
            return;
        }
        if let Some((_, edit)) = self.view.edit.as_mut() {
            edit.insert_text(text);
        } else if self.view.focus == SettingsFocus::Search {
            self.view.search_edit.insert_text(text);
            self.query = self.view.search_edit.text().to_owned();
            self.view.scroll = 0;
            self.view.selected = self.visible_indices().first().copied();
        }
    }

    pub fn edit_command(&mut self, command: &str, control: bool, shift: bool) {
        if self.view.microphone_test_open {
            return;
        }
        let edit = if let Some((_, edit)) = self.view.edit.as_mut() {
            Some(edit)
        } else if self.view.focus == SettingsFocus::Search {
            Some(&mut self.view.search_edit)
        } else {
            None
        };
        if let Some(edit) = edit {
            match command {
                "backspace" => {
                    edit.backspace(control, shift);
                }
                "delete" => {
                    edit.delete(control, shift);
                }
                "all" => edit.select_all(),
                "left" => edit.move_cursor(RenameEditCursorOperation::Left, control, shift),
                "right" => edit.move_cursor(RenameEditCursorOperation::Right, control, shift),
                "home" => edit.move_cursor(RenameEditCursorOperation::Home, control, shift),
                "end" => edit.move_cursor(RenameEditCursorOperation::End, control, shift),
                _ => {}
            }
        }
        if self.view.edit.is_none() && self.view.focus == SettingsFocus::Search {
            self.query = self.view.search_edit.text().to_owned();
            self.view.scroll = 0;
            self.view.selected = self.visible_indices().first().copied();
        }
    }

    pub fn key(&mut self, key: KeyCode, shift: bool, control: bool) -> Vec<SettingsAction> {
        if self.view.microphone_test_open {
            return self.microphone_key(key, shift, control);
        }
        if self.view.choice.is_some() {
            return self.choice_key(key);
        }
        if self.view.display_confirmation.is_some() {
            return match key {
                KeyCode::Enter => vec![SettingsAction::ConfirmDisplay(true)],
                KeyCode::Escape => vec![SettingsAction::ConfirmDisplay(false)],
                _ => Vec::new(),
            };
        }
        if self.view.reset_confirmation {
            match key {
                KeyCode::Escape => self.view.reset_confirmation = false,
                KeyCode::Enter => {
                    self.view.reset_confirmation = false;
                    let visible = self.visible_indices();
                    return self
                        .settings
                        .iter()
                        .enumerate()
                        .filter(|(index, s)| {
                            s.category == self.category
                                && visible.contains(index)
                                && !s.details.exclude_from_category_reset
                                && s.value.is_editable()
                                && s.details.unavailable.is_none()
                                && s.details.policy != ApplyPolicy::DisplayPreview
                        })
                        .map(|(i, s)| SettingsAction::Change(i, s.default.clone()))
                        .collect();
                }
                _ => {}
            }
            return Vec::new();
        }
        if let Some((index, edit)) = self.view.edit.as_ref() {
            match key {
                KeyCode::Escape => self.view.edit = None,
                KeyCode::Enter | KeyCode::Tab => match self.parse_value(*index, edit.text()) {
                    Ok(value) => {
                        let index = *index;
                        self.view.edit = None;
                        let mut actions = vec![SettingsAction::Change(index, value)];
                        if key == KeyCode::Tab {
                            actions.extend(self.key(key, shift, control));
                        }
                        return actions;
                    }
                    Err(error) => self.view.message = error,
                },
                KeyCode::Left => self.edit_command("left", control, shift),
                KeyCode::Right => self.edit_command("right", control, shift),
                KeyCode::Home => self.edit_command("home", control, shift),
                KeyCode::End => self.edit_command("end", control, shift),
                _ => {}
            }
            return Vec::new();
        }
        match key {
            KeyCode::Escape => {
                if !self.query.is_empty() {
                    self.query.clear();
                    self.view.search_edit.set_text("");
                    self.view.scroll = 0;
                } else {
                    return vec![SettingsAction::Close];
                }
            }
            KeyCode::Tab if control => {
                let index = SettingsCategory::ALL
                    .iter()
                    .position(|c| *c == self.category)
                    .unwrap_or(0);
                self.select_category(
                    SettingsCategory::ALL[(index + if shift { 6 } else { 1 }) % 7],
                );
            }
            KeyCode::Tab => {
                let order = self.focus_order();
                let index = order
                    .iter()
                    .position(|f| *f == self.view.focus)
                    .unwrap_or(0);
                self.set_focus(
                    order[(index + if shift { order.len() - 1 } else { 1 }) % order.len()],
                );
            }
            KeyCode::Down | KeyCode::Up | KeyCode::PageDown | KeyCode::PageUp => {
                let backwards = matches!(key, KeyCode::Up | KeyCode::PageUp);
                if matches!(self.view.focus, SettingsFocus::AudioPage(_)) {
                    if backwards {
                        self.set_focus(SettingsFocus::Search);
                    } else if let Some(index) = self.visible_indices().first().copied() {
                        self.set_focus(SettingsFocus::Row(index));
                    }
                    return Vec::new();
                }
                if matches!(
                    self.view.focus,
                    SettingsFocus::Pin
                        | SettingsFocus::PinUp
                        | SettingsFocus::PinDown
                        | SettingsFocus::Reset
                        | SettingsFocus::ResetCategory
                        | SettingsFocus::TestMicrophone
                        | SettingsFocus::RefreshDevices
                        | SettingsFocus::Close
                        | SettingsFocus::Modified
                        | SettingsFocus::Advanced
                ) {
                    let order = self.focus_order();
                    let index = order
                        .iter()
                        .position(|focus| *focus == self.view.focus)
                        .unwrap_or(0);
                    self.set_focus(
                        order[(index + if backwards { order.len() - 1 } else { 1 }) % order.len()],
                    );
                    return Vec::new();
                }
                if let SettingsFocus::Category(category) = self.view.focus {
                    let index = SettingsCategory::ALL
                        .iter()
                        .position(|c| *c == category)
                        .unwrap_or(0);
                    self.select_category(
                        SettingsCategory::ALL[(index + if backwards { 6 } else { 1 }) % 7],
                    );
                } else {
                    let visible = self.visible_indices();
                    if !visible.is_empty() {
                        let old = visible
                            .iter()
                            .position(|i| self.view.focus == SettingsFocus::Row(*i));
                        if key == KeyCode::Up && old == Some(0) {
                            self.set_focus(if self.category == SettingsCategory::Audio {
                                SettingsFocus::AudioPage(self.audio_page)
                            } else {
                                SettingsFocus::Category(self.category)
                            });
                            return Vec::new();
                        }
                        if key == KeyCode::Down && old == Some(visible.len() - 1) {
                            self.set_focus(SettingsFocus::Pin);
                            return Vec::new();
                        }
                        let step = if matches!(key, KeyCode::PageUp | KeyCode::PageDown) {
                            self.page_size()
                        } else {
                            1
                        };
                        let position = old
                            .map(|p| {
                                if backwards {
                                    p.saturating_sub(step)
                                } else {
                                    (p + step).min(visible.len() - 1)
                                }
                            })
                            .unwrap_or(0);
                        self.set_focus(SettingsFocus::Row(visible[position]));
                    }
                }
            }
            KeyCode::Left | KeyCode::Right if self.view.focus == SettingsFocus::Search => {
                self.edit_command(
                    if key == KeyCode::Left {
                        "left"
                    } else {
                        "right"
                    },
                    control,
                    shift,
                );
            }
            KeyCode::Left | KeyCode::Right => {
                if matches!(self.view.focus, SettingsFocus::AudioPage(_)) {
                    self.select_audio_page(if self.audio_page == AudioPage::Sound {
                        AudioPage::Voice
                    } else {
                        AudioPage::Sound
                    });
                    return Vec::new();
                }
                if let SettingsFocus::Row(index) = self.view.focus {
                    return self.adjust(index, if key == KeyCode::Left { -1 } else { 1 });
                }
                if let SettingsFocus::Category(_) = self.view.focus {
                    if self.category == SettingsCategory::Audio {
                        self.set_focus(SettingsFocus::AudioPage(self.audio_page));
                    } else if let Some(index) = self.visible_indices().first().copied() {
                        self.set_focus(SettingsFocus::Row(index));
                    }
                } else {
                    return self.key(KeyCode::Tab, key == KeyCode::Left, false);
                }
            }
            KeyCode::Enter | KeyCode::Space => return self.activate(self.view.focus),
            KeyCode::Home | KeyCode::End if self.view.focus == SettingsFocus::Search => self
                .edit_command(
                    if key == KeyCode::Home { "home" } else { "end" },
                    control,
                    shift,
                ),
            _ => {}
        }
        self.ensure_visible();
        Vec::new()
    }

    fn focus_order(&self) -> Vec<SettingsFocus> {
        let mut order = SettingsCategory::ALL.map(SettingsFocus::Category).to_vec();
        order.push(SettingsFocus::Search);
        if self.category == SettingsCategory::Audio {
            order.push(SettingsFocus::AudioPage(self.audio_page));
        }
        order.extend([SettingsFocus::Modified, SettingsFocus::Advanced]);
        order.extend(self.visible_indices().into_iter().map(SettingsFocus::Row));
        order.push(SettingsFocus::Pin);
        if self.category == SettingsCategory::Quick {
            order.extend([SettingsFocus::PinUp, SettingsFocus::PinDown]);
        }
        order.push(SettingsFocus::Reset);
        if self.category != SettingsCategory::Quick {
            order.push(SettingsFocus::ResetCategory);
        }
        if self.voice_page_selected() {
            order.push(SettingsFocus::TestMicrophone);
        }
        order.push(SettingsFocus::Close);
        order
    }

    pub(crate) fn row_emphasis(&self, index: usize) -> Option<RowEmphasis> {
        [
            (
                self.view.focus == SettingsFocus::Row(index),
                RowEmphasis::Focused,
            ),
            (self.view.selected == Some(index), RowEmphasis::Selected),
            (self.view.hovered == Some(index), RowEmphasis::Hovered),
        ]
        .into_iter()
        .find_map(|(applies, emphasis)| applies.then_some(emphasis))
    }

    pub fn set_focus(&mut self, focus: SettingsFocus) {
        self.view.focus = focus;
        if let SettingsFocus::Row(index) = focus {
            self.view.selected = Some(index);
        }
        self.ensure_visible();
    }

    fn page_size(&self) -> usize {
        (self.layout().list.h / self.row_height()).max(1) as usize
    }

    fn row_height(&self) -> i32 {
        if self.category == SettingsCategory::Audio {
            28
        } else {
            ROW_HEIGHT
        }
    }

    fn ensure_visible(&mut self) {
        let visible = self.visible_indices();
        let count = self.page_size();
        self.view.scroll = self.view.scroll.min(visible.len().saturating_sub(count));
        if let SettingsFocus::Row(index) = self.view.focus {
            if let Some(position) = visible.iter().position(|i| *i == index) {
                if position < self.view.scroll {
                    self.view.scroll = position;
                }
                if position >= self.view.scroll + count {
                    self.view.scroll = position + 1 - count;
                }
            }
        }
    }

    pub fn scroll(&mut self, amount: i32) {
        if self.view.microphone_test_open {
            return;
        }
        if self.view.choice.is_some() {
            self.scroll_choices(amount);
            return;
        }
        self.view.scroll = self.view.scroll.saturating_add_signed(amount as isize).min(
            self.visible_indices()
                .len()
                .saturating_sub(self.page_size()),
        );
    }

    fn editable(&mut self, index: usize) -> bool {
        self.settings.get(index).is_some_and(|setting| {
            if let Some(reason) = &setting.details.unavailable {
                self.view.message = reason.clone();
                false
            } else {
                setting.value.is_editable() && setting.details.policy != ApplyPolicy::ReadOnly
            }
        })
    }

    fn activate(&mut self, focus: SettingsFocus) -> Vec<SettingsAction> {
        match focus {
            SettingsFocus::Row(index) if self.editable(index) => {
                let setting = &self.settings[index];
                if setting.details.binding {
                    return vec![SettingsAction::CaptureBinding(index)];
                }
                match setting.value {
                    AdvancedConfigValue::Bool(_) => return self.adjust(index, 1),
                    AdvancedConfigValue::Choice { .. } => self.open_choices(index),
                    _ => {
                        let mut edit = RenameEdit::new(setting.value.serialized(), None);
                        edit.select_all();
                        self.view.edit = Some((index, edit));
                    }
                }
            }
            SettingsFocus::Category(category) => self.select_category(category),
            SettingsFocus::AudioPage(page) => self.select_audio_page(page),
            SettingsFocus::Modified => {
                self.modified_only = !self.modified_only;
                self.view.scroll = 0;
            }
            SettingsFocus::Advanced => {
                self.show_advanced = !self.show_advanced;
                self.view.scroll = 0;
            }
            SettingsFocus::Pin => {
                if let Some(index) = self.view.selected {
                    self.toggle_pin(index);
                }
            }
            SettingsFocus::PinUp | SettingsFocus::PinDown => {
                if let Some(position) = self
                    .view
                    .selected
                    .and_then(|i| self.settings.get(i))
                    .and_then(|s| self.pinned.iter().position(|id| *id == s.id))
                {
                    let next = if focus == SettingsFocus::PinUp {
                        position.saturating_sub(1)
                    } else {
                        (position + 1).min(self.pinned.len() - 1)
                    };
                    self.pinned.swap(position, next);
                }
            }
            SettingsFocus::Reset => {
                if let Some(index) = self.view.selected.filter(|i| self.editable(*i)) {
                    return vec![SettingsAction::Change(
                        index,
                        self.settings[index].default.clone(),
                    )];
                }
            }
            SettingsFocus::ResetCategory => {
                if !self.query.is_empty() {
                    self.view.message = "Clear search before resetting a category.".into();
                    return Vec::new();
                }
                if self.category != SettingsCategory::Quick {
                    self.view.reset_confirmation = true;
                }
            }
            SettingsFocus::TestMicrophone => {
                self.cancel_interaction();
                self.view.choice = None;
                self.view.edit = None;
                self.view.microphone_test_open = true;
                self.view.focus = SettingsFocus::RecordMicrophone;
            }
            SettingsFocus::RecordMicrophone => return vec![SettingsAction::TestMicrophone],
            SettingsFocus::CloseMicrophoneTest => {
                self.cancel_interaction();
                self.view.microphone_test_open = false;
                self.view.focus = SettingsFocus::TestMicrophone;
                return vec![SettingsAction::CancelMicrophoneTest];
            }
            SettingsFocus::RefreshDevices => return vec![SettingsAction::RefreshDevices],
            SettingsFocus::Close => return vec![SettingsAction::Close],
            _ => {}
        }
        Vec::new()
    }

    fn adjust(&mut self, index: usize, direction: i128) -> Vec<SettingsAction> {
        if !self.editable(index) {
            return Vec::new();
        }
        let setting = &self.settings[index];
        if setting.details.binding {
            return Vec::new();
        }
        let value = match &setting.value {
            AdvancedConfigValue::Bool(value) => AdvancedConfigValue::Bool(!value),
            AdvancedConfigValue::Integer { value, min, max } => AdvancedConfigValue::Integer {
                value: value
                    .saturating_add(direction * setting.details.step.max(1))
                    .clamp(*min, *max),
                min: *min,
                max: *max,
            },
            AdvancedConfigValue::Choice { value, choices } if !choices.is_empty() => {
                let index = choices.iter().position(|c| c.value == *value).unwrap_or(0) as i128;
                let next = (index + direction).rem_euclid(choices.len() as i128) as usize;
                AdvancedConfigValue::Choice {
                    value: choices[next].value.clone(),
                    choices: choices.clone(),
                }
            }
            _ => return Vec::new(),
        };
        vec![SettingsAction::Change(index, value)]
    }

    pub fn cancel_interaction(&mut self) {
        self.view.pressed = None;
        self.view.dragging = None;
        self.view.scroll_drag = None;
        if let Some(choice) = self.view.choice.as_mut() {
            choice.pressed = None;
        }
    }

    pub fn pointer(&mut self, point: GuiPoint, down: bool) -> Vec<SettingsAction> {
        if self.view.microphone_test_open {
            return self.microphone_pointer(point, down);
        }
        if self.view.choice.is_some() {
            return self.choice_pointer(point, down);
        }
        let layout = self.layout();
        if self.view.display_confirmation.is_some() || self.view.reset_confirmation {
            if !down {
                let keep = IntRect::new(
                    layout.footer.x,
                    layout.footer.y + layout.footer.h - 32,
                    150,
                    28,
                );
                let revert = IntRect::new(keep.x + 160, keep.y, 150, 30);
                if contains(keep, point) {
                    return self.key(KeyCode::Enter, false, false);
                }
                if contains(revert, point) {
                    return self.key(KeyCode::Escape, false, false);
                }
            }
            return Vec::new();
        }
        if !down && (self.view.dragging.is_some() || self.view.scroll_drag.is_some()) {
            let actions = self.pointer_move(point);
            self.cancel_interaction();
            return actions;
        }
        if down {
            if let Some((track, thumb)) = self.scrollbar() {
                if contains(track, point) {
                    self.view.scroll_drag = Some(if contains(thumb, point) {
                        point.y as i32 - thumb.y
                    } else {
                        thumb.h / 2
                    });
                    self.view.pressed = None;
                    return self.pointer_move(point);
                }
            }
            for index in self.visible_indices() {
                let Some(row) = self.row_rect(&layout, index) else {
                    continue;
                };
                let track = slider_rect(row);
                let setting = &self.settings[index];
                if contains(track, point)
                    && !setting.details.binding
                    && setting.details.policy != ApplyPolicy::DisplayPreview
                    && matches!(setting.value, AdvancedConfigValue::Integer { min, max, .. } if max > min && max - min <= 1000)
                    && self.editable(index)
                {
                    self.view.edit = None;
                    self.set_focus(SettingsFocus::Row(index));
                    if point.x < (track.x + 16) as f32 {
                        return self.adjust(index, -1);
                    }
                    if point.x >= (track.x + track.w - 16) as f32 {
                        return self.adjust(index, 1);
                    }
                    self.view.dragging = Some(index);
                    return self.pointer_move(point);
                }
            }
        }
        let target = self
            .targets(&layout)
            .into_iter()
            .find(|(_, rect)| contains(*rect, point))
            .map(|(f, _)| f);
        if down {
            if let Some((index, edit)) = self.view.edit.as_ref() {
                if target != Some(SettingsFocus::Row(*index)) {
                    match self.parse_value(*index, edit.text()) {
                        Ok(value) => {
                            let index = *index;
                            self.view.edit = None;
                            self.view.pressed = target;
                            if let Some(focus) = target {
                                self.set_focus(focus);
                            }
                            return vec![SettingsAction::Change(index, value)];
                        }
                        Err(error) => {
                            self.view.message = error;
                            return Vec::new();
                        }
                    }
                }
            }
            self.view.pressed = target;
            if let Some(focus) = target {
                self.set_focus(focus);
            }
        } else if let Some(focus) = self.view.pressed.take().filter(|f| Some(*f) == target) {
            if self
                .view
                .edit
                .as_ref()
                .is_some_and(|(index, _)| focus == SettingsFocus::Row(*index))
            {
                return Vec::new();
            }
            return self.activate(focus);
        }
        Vec::new()
    }

    pub fn pointer_move(&mut self, point: GuiPoint) -> Vec<SettingsAction> {
        let layout = self.layout();
        self.view.hovered = self.visible_indices().into_iter().find(|index| {
            self.row_rect(&layout, *index)
                .is_some_and(|row| contains(row, point))
        });
        if let Some(offset) = self.view.scroll_drag {
            if let Some((track, thumb)) = self.scrollbar() {
                let maximum = self
                    .visible_indices()
                    .len()
                    .saturating_sub(self.page_size());
                let travel = (track.h - thumb.h).max(1);
                self.view.scroll = (((point.y as i32 - track.y - offset).clamp(0, travel) as f64
                    / f64::from(travel))
                    * maximum as f64)
                    .round() as usize;
            }
        }
        let Some(index) = self.view.dragging else {
            return Vec::new();
        };
        let layout = self.layout();
        let Some(row) = self.row_rect(&layout, index) else {
            return Vec::new();
        };
        let setting = &self.settings[index];
        let AdvancedConfigValue::Integer { value, min, max } = setting.value else {
            return Vec::new();
        };
        let track = slider_rect(row);
        let fraction = ((point.x - (track.x + 24) as f32) / (track.w - 48) as f32).clamp(0.0, 1.0);
        let next = min + ((max - min) as f32 * fraction).round() as i128;
        if next == value {
            return Vec::new();
        }
        vec![SettingsAction::Change(
            index,
            AdvancedConfigValue::Integer {
                value: next,
                min,
                max,
            },
        )]
    }

    fn scrollbar(&self) -> Option<(IntRect, IntRect)> {
        let count = self.visible_indices().len();
        let page = self.page_size();
        if count <= page {
            return None;
        }
        let list = self.layout().list;
        let track = IntRect::new(list.x + list.w - 10, list.y, 10, list.h);
        let height = (list.h * page as i32 / count as i32).max(20).min(list.h);
        let top = (list.h - height) * self.view.scroll as i32 / (count - page) as i32;
        Some((track, IntRect::new(track.x, track.y + top, track.w, height)))
    }

    fn row_rect(&self, layout: &SettingsLayout, index: usize) -> Option<IntRect> {
        let position = self.visible_indices().iter().position(|i| *i == index)?;
        (position >= self.view.scroll && position < self.view.scroll + self.page_size()).then(
            || {
                IntRect::new(
                    layout.list.x,
                    layout.list.y + (position - self.view.scroll) as i32 * self.row_height(),
                    layout.list.w - 14,
                    self.row_height() - 3,
                )
            },
        )
    }

    fn targets(&self, layout: &SettingsLayout) -> Vec<(SettingsFocus, IntRect)> {
        let audio = self.category == SettingsCategory::Audio;
        let filters_x = layout.list.x + if audio { 200 } else { 0 };
        let mut targets = vec![
            (SettingsFocus::Search, layout.search),
            (
                SettingsFocus::Modified,
                IntRect::new(
                    filters_x,
                    layout.search.y + 32,
                    if audio { 98 } else { 122 },
                    20,
                ),
            ),
            (
                SettingsFocus::Advanced,
                IntRect::new(
                    filters_x + if audio { 104 } else { 130 },
                    layout.search.y + 32,
                    if audio { layout.list.w - 304 } else { 122 },
                    20,
                ),
            ),
        ];
        if audio {
            targets.extend([
                (
                    SettingsFocus::AudioPage(AudioPage::Sound),
                    IntRect::new(layout.list.x, layout.search.y + 30, 72, 26),
                ),
                (
                    SettingsFocus::AudioPage(AudioPage::Voice),
                    IntRect::new(layout.list.x + 78, layout.search.y + 30, 108, 26),
                ),
            ]);
        }
        targets.extend(
            SettingsCategory::ALL
                .into_iter()
                .enumerate()
                .map(|(i, c)| (SettingsFocus::Category(c), layout.tabs[i])),
        );
        targets.extend(
            self.visible_indices()
                .into_iter()
                .filter_map(|i| self.row_rect(layout, i).map(|r| (SettingsFocus::Row(i), r))),
        );
        let x = layout.footer.x;
        let y = layout.footer.y + if audio { 52 } else { 42 };
        targets.push((SettingsFocus::Pin, IntRect::new(x, y, 58, 26)));
        if self.category == SettingsCategory::Quick {
            targets.extend([
                (SettingsFocus::PinUp, IntRect::new(x + 64, y, 32, 26)),
                (SettingsFocus::PinDown, IntRect::new(x + 102, y, 48, 26)),
                (SettingsFocus::Reset, IntRect::new(x + 156, y, 98, 26)),
            ]);
        } else {
            targets.extend([
                (SettingsFocus::Reset, IntRect::new(x + 64, y, 94, 26)),
                (
                    SettingsFocus::ResetCategory,
                    IntRect::new(x + 164, y, 94, 26),
                ),
            ]);
        }
        if self.voice_page_selected() {
            targets.push((
                SettingsFocus::TestMicrophone,
                IntRect::new(x + layout.footer.w - 148, y, 148, 26),
            ));
        }
        targets.push((SettingsFocus::Close, layout.back));
        targets
    }
}

/// The text a row shows for its value: the application's own label when it
/// provides one (bindings, current-match rows), otherwise the value itself.
pub(crate) fn value_label(setting: &Setting) -> String {
    setting
        .details
        .display_value
        .clone()
        .unwrap_or_else(|| formatted_value(setting, &setting.value))
}

pub(crate) fn formatted_value(setting: &Setting, value: &AdvancedConfigValue) -> String {
    match value {
        AdvancedConfigValue::Choice { value, choices } => choices
            .iter()
            .find(|c| c.value == *value)
            .map(|c| c.label.clone())
            .unwrap_or_else(|| {
                if value.is_empty() {
                    "System default".into()
                } else {
                    value.clone()
                }
            }),
        AdvancedConfigValue::Bool(on) => if *on { "On" } else { "Off" }.into(),
        _ if setting.id.key.to_lowercase().contains("password") => "••••••".into(),
        AdvancedConfigValue::Integer { value, .. } => format!("{value}{}", setting.details.unit),
        _ => value.serialized(),
    }
}

/// The footer's second line: why a setting is unavailable, or where and when
/// a change takes effect.
pub(crate) fn detail_text(setting: &Setting) -> String {
    setting.details.unavailable.clone().unwrap_or_else(|| {
        let active = setting
            .details
            .active_value
            .as_ref()
            .map(|value| format!(" · Active: {value}"))
            .unwrap_or_default();
        let default = if setting.is_modified() {
            let default = setting
                .details
                .default_display
                .clone()
                .unwrap_or_else(|| formatted_value(setting, &setting.default));
            format!(" · Default: {default}")
        } else {
            String::new()
        };
        format!(
            "{} · {}{active}{default}",
            setting.details.scope,
            setting.details.policy.label()
        )
    })
}

fn value_rect(row: IntRect) -> IntRect {
    IntRect::new(row.x + row.w - 174, row.y + 2, 174, 26)
}

fn slider_rect(row: IntRect) -> IntRect {
    IntRect::new(row.x + row.w - 174, row.y + 9, 128, 16)
}

fn contains(rect: IntRect, point: GuiPoint) -> bool {
    point.x >= rect.x as f32
        && point.y >= rect.y as f32
        && point.x < (rect.x + rect.w) as f32
        && point.y < (rect.y + rect.h) as f32
}

#[cfg(test)]
mod tests {
    use super::*;

    fn preference(key: &str, value: AdvancedConfigValue) -> Setting {
        Setting {
            id: SettingId::new("Graphics", key),
            label: key.into(),
            keywords: String::new(),
            category: SettingsCategory::Display,
            advanced: false,
            default: value.clone(),
            value,
            details: Default::default(),
        }
    }

    #[test]
    fn the_row_the_footer_acts_on_stays_highlighted_while_search_has_focus() {
        let mut controller = SettingsController::new(
            ["ShowClock", "ShowStats", "ShowPortraits"]
                .map(|key| preference(key, AdvancedConfigValue::Bool(true)))
                .to_vec(),
        );
        controller.select_category(SettingsCategory::Display);
        controller.focus_search();
        assert_eq!(controller.row_emphasis(0), Some(RowEmphasis::Selected));
        assert_eq!(controller.row_emphasis(1), None);
        controller.set_focus(SettingsFocus::Row(1));
        assert_eq!(controller.row_emphasis(0), None);
        assert_eq!(controller.row_emphasis(1), Some(RowEmphasis::Focused));
    }

    #[test]
    fn numeric_values_and_defaults_carry_their_unit() {
        let seconds = |value| AdvancedConfigValue::Integer {
            value,
            min: 3,
            max: 60,
        };
        let mut duration = preference("Duration", seconds(12));
        duration.details.unit = " s".into();
        assert_eq!(value_label(&duration), "12 s");
        duration.value = seconds(30);
        assert!(detail_text(&duration).ends_with("Default: 12 s"));
    }

    #[test]
    fn a_rebound_key_names_its_default_with_the_applications_label() {
        let key = |value| AdvancedConfigValue::Integer {
            value,
            min: 0,
            max: 255,
        };
        let mut binding = preference("Kbd1Key1", key(81));
        binding.value = key(123);
        binding.details.display_value = Some("F12".into());
        binding.details.default_display = Some("Q".into());
        assert_eq!(value_label(&binding), "F12");
        assert!(detail_text(&binding).ends_with("Default: Q"));
    }

    #[test]
    fn a_changed_setting_is_marked_and_names_the_default_it_would_reset_to() {
        let mut clock = preference("ShowClock", AdvancedConfigValue::Bool(true));
        clock.value = AdvancedConfigValue::Bool(false);
        let mut scale = preference(
            "Scale",
            AdvancedConfigValue::Integer {
                value: 100,
                min: 100,
                max: 400,
            },
        );
        scale.value = AdvancedConfigValue::Integer {
            value: 150,
            min: 100,
            max: 400,
        };
        let stats = preference("ShowStats", AdvancedConfigValue::Bool(false));
        let controller = SettingsController::new(vec![clock, scale, stats]);
        assert!(controller.is_modified(0));
        assert!(controller.is_modified(1));
        assert!(!controller.is_modified(2));
        assert!(detail_text(&controller.settings[0]).ends_with(" · Default: On"));
        assert!(detail_text(&controller.settings[1]).ends_with(" · Default: 100"));
        assert!(!detail_text(&controller.settings[2]).contains("Default"));
    }

    #[test]
    fn the_pointer_marks_the_row_it_is_over_until_it_leaves_the_list() {
        let mut controller = SettingsController::new(
            ["ShowClock", "ShowStats", "ShowPortraits"]
                .map(|key| preference(key, AdvancedConfigValue::Bool(true)))
                .to_vec(),
        );
        controller.select_category(SettingsCategory::Display);
        let layout = controller.layout();
        let row = controller.row_rect(&layout, 2).unwrap();
        controller.pointer_move(GuiPoint::new((row.x + 40) as f32, (row.y + 4) as f32));
        assert_eq!(controller.row_emphasis(2), Some(RowEmphasis::Hovered));
        assert_eq!(controller.row_emphasis(0), Some(RowEmphasis::Selected));
        controller.pointer_move(GuiPoint::new(layout.panel.x as f32, layout.panel.y as f32));
        assert_eq!(controller.row_emphasis(2), None);
    }

    #[test]
    fn audio_subpages_are_directly_clickable_without_changing_filters() {
        let mut controller = SettingsController::new(Vec::new());
        controller.select_category(SettingsCategory::Audio);
        let layout = controller.layout();
        let voice = GuiPoint::new((layout.list.x + 100) as f32, (layout.search.y + 40) as f32);
        controller.pointer(voice, true);
        controller.pointer(voice, false);
        assert_eq!(controller.audio_page, AudioPage::Voice);
        assert!(!controller.modified_only);
    }

    #[test]
    fn opening_microphone_test_waits_for_an_explicit_record_action() {
        let mut controller = SettingsController::new(Vec::new());
        controller.select_category(SettingsCategory::Audio);
        controller.audio_page = AudioPage::Voice;
        controller.set_focus(SettingsFocus::TestMicrophone);
        assert!(
            controller.key(KeyCode::Enter, false, false).is_empty(),
            "opening the test panel must not start recording"
        );
        assert!(
            controller
                .key(KeyCode::Escape, false, false)
                .iter()
                .all(|action| *action != SettingsAction::Close),
            "Escape returns to Voice chat before leaving settings"
        );
        controller.key(KeyCode::Enter, false, false);
        for expected in [
            SettingsFocus::RefreshDevices,
            SettingsFocus::CloseMicrophoneTest,
            SettingsFocus::RecordMicrophone,
        ] {
            controller.key(KeyCode::Tab, false, false);
            assert_eq!(controller.view.focus, expected);
        }
        controller.focus_search();
        assert_eq!(controller.view.focus, SettingsFocus::RecordMicrophone);
        assert_eq!(
            controller.key(KeyCode::Enter, false, false),
            vec![SettingsAction::TestMicrophone]
        );
        assert_eq!(
            controller.key(KeyCode::Escape, false, false),
            vec![SettingsAction::CancelMicrophoneTest]
        );
        assert!(!controller.view.microphone_test_open);
        assert_eq!(controller.view.focus, SettingsFocus::TestMicrophone);
        controller.key(KeyCode::Enter, false, false);
        assert_eq!(
            controller.key(KeyCode::Tab, false, true),
            vec![SettingsAction::CancelMicrophoneTest]
        );
        assert_eq!(controller.category, SettingsCategory::Controls);
    }

    #[test]
    fn focusing_search_finishes_the_previous_edit_interaction() {
        let mut controller = SettingsController::new(vec![Setting {
            id: SettingId::new("Network", "Comment"),
            label: "Comment".into(),
            keywords: String::new(),
            category: SettingsCategory::Game,
            advanced: false,
            value: AdvancedConfigValue::Text("old".into()),
            default: AdvancedConfigValue::Text(String::new()),
            details: Default::default(),
        }]);
        controller.category = SettingsCategory::Game;
        controller.set_focus(SettingsFocus::Row(0));
        controller.key(KeyCode::Enter, false, false);
        controller.text("unfinished");
        controller.focus_search();
        controller.text("volume");
        assert_eq!(controller.query, "volume");
        assert_eq!(controller.settings[0].value.serialized(), "old");
    }

    #[test]
    fn category_reset_does_not_reset_hidden_advanced_preferences() {
        let mut controller = SettingsController::new(
            [false, true]
                .into_iter()
                .map(|advanced| Setting {
                    id: SettingId::new("Sound", if advanced { "Internal" } else { "Music" }),
                    label: "Preference".into(),
                    keywords: String::new(),
                    category: SettingsCategory::Audio,
                    advanced,
                    value: AdvancedConfigValue::Bool(false),
                    default: AdvancedConfigValue::Bool(true),
                    details: Default::default(),
                })
                .collect(),
        );
        controller.category = SettingsCategory::Audio;
        controller.set_focus(SettingsFocus::ResetCategory);
        controller.key(KeyCode::Enter, false, false);
        let actions = controller.key(KeyCode::Enter, false, false);
        assert_eq!(
            actions,
            vec![SettingsAction::Change(0, AdvancedConfigValue::Bool(true))]
        );
    }
    #[test]
    fn directional_navigation_reaches_microphone_tools_and_returns_to_categories() {
        let value = AdvancedConfigValue::Bool(true);
        let mut controller = SettingsController::new(vec![Setting {
            id: SettingId::new("Voice", "Enabled"),
            label: "Voice chat".into(),
            keywords: String::new(),
            category: SettingsCategory::Audio,
            advanced: false,
            value: value.clone(),
            default: value,
            details: Default::default(),
        }]);
        controller.category = SettingsCategory::Audio;
        controller.audio_page = AudioPage::Voice;
        controller.set_focus(SettingsFocus::Row(0));
        controller.key(KeyCode::Down, false, false);
        assert_eq!(controller.view.focus, SettingsFocus::Pin);
        for _ in 0..3 {
            controller.key(KeyCode::Down, false, false);
        }
        assert_eq!(controller.view.focus, SettingsFocus::TestMicrophone);
        controller.set_focus(SettingsFocus::Row(0));
        controller.key(KeyCode::Up, false, false);
        assert_eq!(
            controller.view.focus,
            SettingsFocus::AudioPage(AudioPage::Voice)
        );
    }
    #[test]
    fn clicking_a_volume_slider_emits_the_value_at_the_pointer() {
        let value = AdvancedConfigValue::Integer {
            value: 40,
            min: 0,
            max: 100,
        };
        let mut controller = SettingsController::new(vec![Setting {
            id: SettingId::new("Sound", "MusicVolume"),
            label: "Music volume".into(),
            keywords: String::new(),
            category: SettingsCategory::Audio,
            advanced: false,
            value: value.clone(),
            default: value,
            details: Default::default(),
        }]);
        controller.category = SettingsCategory::Audio;
        let layout = SettingsLayout::new(800, 600);
        let row = controller.row_rect(&layout, 0).unwrap();
        let track = slider_rect(row);
        let actions = controller.pointer(
            GuiPoint::new((track.x + track.w - 24) as f32, (track.y + 8) as f32),
            true,
        );
        assert!(
            matches!(actions.as_slice(), [SettingsAction::Change(0, value)] if value.serialized() == "100")
        );
    }
    #[test]
    fn wheel_scroll_keeps_its_position_when_the_focused_row_leaves_the_view() {
        let mut controller = SettingsController::new(
            (0..20)
                .map(|i| Setting {
                    id: SettingId::new("Sound", i.to_string()),
                    label: format!("Setting {i}"),
                    keywords: String::new(),
                    category: SettingsCategory::Audio,
                    advanced: false,
                    value: AdvancedConfigValue::Bool(true),
                    default: AdvancedConfigValue::Bool(true),
                    details: Default::default(),
                })
                .collect(),
        );
        controller.category = SettingsCategory::Audio;
        controller.set_focus(SettingsFocus::Row(0));
        controller.scroll(5);
        controller.resize(800, 600);
        assert_eq!(controller.view.scroll, 5);
    }
    #[test]
    fn device_choices_can_be_browsed_and_cancelled_before_applying() {
        use crate::startup_options_advanced::AdvancedConfigChoice;
        let value = AdvancedConfigValue::Choice {
            value: "default".into(),
            choices: vec![
                AdvancedConfigChoice {
                    value: "default".into(),
                    label: "System default".into(),
                },
                AdvancedConfigChoice {
                    value: "usb".into(),
                    label: "USB microphone".into(),
                },
            ],
        };
        let mut controller = SettingsController::new(vec![Setting {
            id: SettingId::new("Voice", "InputDevice"),
            label: "Microphone".into(),
            keywords: String::new(),
            category: SettingsCategory::Audio,
            advanced: false,
            value: value.clone(),
            default: value,
            details: Default::default(),
        }]);
        controller.category = SettingsCategory::Audio;
        controller.set_focus(SettingsFocus::Row(0));
        assert!(controller.key(KeyCode::Enter, false, false).is_empty());
        assert!(controller.key(KeyCode::Down, false, false).is_empty());
        assert!(controller.key(KeyCode::Escape, false, false).is_empty());
        assert_eq!(controller.settings[0].value.serialized(), "default");
        controller.key(KeyCode::Enter, false, false);
        controller.key(KeyCode::Down, false, false);
        let actions = controller.key(KeyCode::Enter, false, false);
        assert!(
            matches!(actions.as_slice(), [SettingsAction::Change(0, value)] if value.serialized() == "usb")
        );
    }
    #[test]
    fn leaving_a_setting_editor_commits_valid_input_and_retains_invalid_input() {
        let value = AdvancedConfigValue::Integer {
            value: 40,
            min: 0,
            max: 100,
        };
        let mut controller = SettingsController::new(vec![Setting {
            id: SettingId::new("Sound", "MusicVolume"),
            label: "Music volume".into(),
            keywords: String::new(),
            category: SettingsCategory::Audio,
            advanced: false,
            value: value.clone(),
            default: value,
            details: Default::default(),
        }]);
        controller.category = SettingsCategory::Audio;
        controller.set_focus(SettingsFocus::Row(0));
        controller.key(KeyCode::Enter, false, false);
        controller.text("75");
        assert_eq!(controller.key(KeyCode::Tab, false, false).len(), 1);
        assert_eq!(controller.view.focus, SettingsFocus::Pin);
        controller.set_focus(SettingsFocus::Row(0));
        controller.key(KeyCode::Enter, false, false);
        controller.text("999");
        assert!(controller.key(KeyCode::Tab, false, false).is_empty());
        assert!(controller.editing());
        assert!(controller.view.message.contains("between"));
    }
    #[test]
    fn editing_can_be_cancelled_and_all_footer_actions_are_keyboard_reachable() {
        let value = AdvancedConfigValue::Integer {
            value: 40,
            min: 0,
            max: 100,
        };
        let mut controller = SettingsController::new(vec![Setting {
            id: SettingId::new("Sound", "MusicVolume"),
            label: "Music volume".into(),
            keywords: String::new(),
            category: SettingsCategory::Audio,
            advanced: false,
            value: value.clone(),
            default: value,
            details: Default::default(),
        }]);
        controller.category = SettingsCategory::Audio;
        controller.view.focus = SettingsFocus::Row(0);
        controller.key(KeyCode::Enter, false, false);
        controller.text("75");
        assert!(controller.key(KeyCode::Escape, false, false).is_empty());
        assert_eq!(controller.settings[0].value.serialized(), "40");
        controller.key(KeyCode::Enter, false, false);
        controller.text("75");
        assert_eq!(
            controller.key(KeyCode::Enter, false, false),
            vec![SettingsAction::Change(
                0,
                AdvancedConfigValue::Integer {
                    value: 75,
                    min: 0,
                    max: 100
                }
            )]
        );
        let mut visited = Vec::new();
        for _ in 0..30 {
            controller.key(KeyCode::Tab, false, false);
            visited.push(controller.view.focus);
        }
        for focus in [
            SettingsFocus::Search,
            SettingsFocus::Modified,
            SettingsFocus::Advanced,
            SettingsFocus::Pin,
            SettingsFocus::Reset,
            SettingsFocus::ResetCategory,
            SettingsFocus::Close,
        ] {
            assert!(visited.contains(&focus), "unreachable: {focus:?}");
        }
    }
    #[test]
    fn keyboard_navigation_edits_a_setting_and_returns_without_gameplay_input() {
        let value = AdvancedConfigValue::Bool(true);
        let mut controller = SettingsController::new(vec![Setting {
            id: SettingId::new("Sound", "Music"),
            label: "Music".into(),
            keywords: String::new(),
            category: SettingsCategory::Audio,
            advanced: false,
            value: value.clone(),
            default: value,
            details: Default::default(),
        }]);
        controller.category = SettingsCategory::Audio;
        controller.view.focus = SettingsFocus::Search;
        controller.key(KeyCode::Down, false, false);
        assert_eq!(controller.view.focus, SettingsFocus::Row(0));
        assert_eq!(
            controller.key(KeyCode::Enter, false, false),
            vec![SettingsAction::Change(0, AdvancedConfigValue::Bool(false))]
        );
        assert_eq!(
            controller.key(KeyCode::Escape, false, false),
            vec![SettingsAction::Close]
        );
    }
    #[test]
    fn settings_layout_keeps_search_rows_and_footer_usable_in_compact_windows() {
        for (width, height) in [(640, 480), (800, 600), (1280, 720)] {
            let layout = SettingsLayout::new(width, height);
            assert!(layout.list.w >= 360 && layout.list.h >= 150);
            assert!(layout.search.y + layout.search.h <= layout.list.y);
            assert!(layout.list.y + layout.list.h <= layout.footer.y);
            for rect in [layout.panel, layout.search, layout.list, layout.footer] {
                assert!(
                    rect.x >= 0
                        && rect.y >= 0
                        && rect.x + rect.w <= width
                        && rect.y + rect.h <= height
                );
            }
        }
    }
}
