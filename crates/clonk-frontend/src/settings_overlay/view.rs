use super::*;
use crate::classic_gui::IntRect;
use crate::rename_edit::{RenameEdit, RenameEditCursorOperation};
use crate::{GuiPoint, KeyCode};

mod choices;
mod render;
const ROW_HEIGHT: i32 = 50;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum SettingsFocus {
    #[default]
    Search,
    Category(SettingsCategory),
    Row(usize),
    Modified,
    Advanced,
    Pin,
    PinUp,
    PinDown,
    Reset,
    ResetCategory,
    TestMicrophone,
    RefreshDevices,
    Close,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum SettingsAction {
    Change(usize, AdvancedConfigValue),
    Close,
    CaptureBinding(usize),
    TestMicrophone,
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
    pub display_confirmation: Option<u64>,
    pub reset_confirmation: bool,
    pub(crate) search_edit: RenameEdit<()>,
    pub(crate) edit: Option<(usize, RenameEdit<()>)>,
    pub(crate) pressed: Option<SettingsFocus>,
    pub(crate) choice: Option<choices::ChoicePicker>,
    pub(crate) dragging: Option<usize>,
    pub(crate) scroll_drag: Option<i32>,
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
            display_confirmation: None,
            reset_confirmation: false,
            search_edit: RenameEdit::new("", None),
            edit: None,
            pressed: None,
            choice: None,
            dragging: None,
            scroll_drag: None,
        }
    }
}

pub struct SettingsLayout {
    pub panel: IntRect,
    pub search: IntRect,
    pub list: IntRect,
    pub footer: IntRect,
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
            panel,
        }
    }
}

impl SettingsController {
    pub fn resize(&mut self, width: i32, height: i32) {
        if self.view.width == width && self.view.height == height {
            return;
        }
        self.view.width = width;
        self.view.height = height;
        self.ensure_visible();
    }

    pub fn select_category(&mut self, category: SettingsCategory) {
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

    pub fn editing(&self) -> bool {
        self.view.edit.is_some() || self.view.focus == SettingsFocus::Search
    }

    pub fn accepts_text(&self) -> bool {
        self.editing() || self.view.choice.is_some()
    }

    pub fn focus_search(&mut self) {
        self.view.edit = None;
        self.view.choice = None;
        self.cancel_interaction();
        self.view.focus = SettingsFocus::Search;
        self.view.search_edit.set_text(self.query.clone());
        self.view.search_edit.select_all();
    }

    pub fn text(&mut self, text: &str) {
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
                            self.set_focus(SettingsFocus::Category(self.category));
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
                if let SettingsFocus::Row(index) = self.view.focus {
                    return self.adjust(index, if key == KeyCode::Left { -1 } else { 1 });
                }
                if let SettingsFocus::Category(_) = self.view.focus {
                    if let Some(index) = self.visible_indices().first().copied() {
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
        let mut order = vec![
            SettingsFocus::Search,
            SettingsFocus::Modified,
            SettingsFocus::Advanced,
        ];
        order.extend(SettingsCategory::ALL.map(SettingsFocus::Category));
        order.extend(self.visible_indices().into_iter().map(SettingsFocus::Row));
        order.extend([
            SettingsFocus::Pin,
            SettingsFocus::PinUp,
            SettingsFocus::PinDown,
            SettingsFocus::Reset,
            SettingsFocus::ResetCategory,
        ]);
        if self.category == SettingsCategory::Audio {
            order.extend([SettingsFocus::TestMicrophone, SettingsFocus::RefreshDevices]);
        }
        order.push(SettingsFocus::Close);
        order
    }

    pub fn set_focus(&mut self, focus: SettingsFocus) {
        self.view.focus = focus;
        if let SettingsFocus::Row(index) = focus {
            self.view.selected = Some(index);
        }
        self.ensure_visible();
    }

    fn page_size(&self) -> usize {
        (SettingsLayout::new(self.view.width, self.view.height)
            .list
            .h
            / ROW_HEIGHT)
            .max(1) as usize
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
            SettingsFocus::TestMicrophone => return vec![SettingsAction::TestMicrophone],
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
        if self.view.choice.is_some() {
            return self.choice_pointer(point, down);
        }
        let layout = SettingsLayout::new(self.view.width, self.view.height);
        if self.view.display_confirmation.is_some() || self.view.reset_confirmation {
            if !down {
                let keep = IntRect::new(layout.footer.x, layout.footer.y + 84, 150, 30);
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
                let track = IntRect::new(row.x + row.w - 134, row.y + 33, 111, 14);
                let setting = &self.settings[index];
                if contains(track, point)
                    && !setting.details.binding
                    && setting.details.policy != ApplyPolicy::DisplayPreview
                    && matches!(setting.value, AdvancedConfigValue::Integer { min, max, .. } if max > min && max - min <= 1000)
                    && self.editable(index)
                {
                    self.view.edit = None;
                    self.set_focus(SettingsFocus::Row(index));
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
            if let SettingsFocus::Row(index) = focus {
                let row = self.row_rect(&layout, index);
                if let Some(rect) = row {
                    if point.x >= (rect.x + rect.w - 30) as f32 {
                        return self.adjust(index, 1);
                    }
                    if point.x >= (rect.x + rect.w - 158) as f32
                        && point.x < (rect.x + rect.w - 128) as f32
                    {
                        return self.adjust(index, -1);
                    }
                }
            }
            return self.activate(focus);
        }
        Vec::new()
    }

    pub fn pointer_move(&mut self, point: GuiPoint) -> Vec<SettingsAction> {
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
        let layout = SettingsLayout::new(self.view.width, self.view.height);
        let Some(row) = self.row_rect(&layout, index) else {
            return Vec::new();
        };
        let setting = &self.settings[index];
        let AdvancedConfigValue::Integer { value, min, max } = setting.value else {
            return Vec::new();
        };
        let fraction = ((point.x - (row.x + row.w - 134) as f32) / 110.0).clamp(0.0, 1.0);
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
        let list = SettingsLayout::new(self.view.width, self.view.height).list;
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
                    layout.list.y + (position - self.view.scroll) as i32 * ROW_HEIGHT,
                    layout.list.w - 14,
                    ROW_HEIGHT - 3,
                )
            },
        )
    }

    fn targets(&self, layout: &SettingsLayout) -> Vec<(SettingsFocus, IntRect)> {
        let mut targets = vec![
            (SettingsFocus::Search, layout.search),
            (
                SettingsFocus::Modified,
                IntRect::new(layout.search.x, layout.search.y + 34, 150, 28),
            ),
            (
                SettingsFocus::Advanced,
                IntRect::new(layout.search.x + 156, layout.search.y + 34, 150, 28),
            ),
        ];
        targets.extend(SettingsCategory::ALL.into_iter().enumerate().map(|(i, c)| {
            (
                SettingsFocus::Category(c),
                IntRect::new(
                    layout.panel.x + 12,
                    layout.panel.y + 52 + i as i32 * 36,
                    148,
                    32,
                ),
            )
        }));
        targets.extend(
            self.visible_indices()
                .into_iter()
                .filter_map(|i| self.row_rect(layout, i).map(|r| (SettingsFocus::Row(i), r))),
        );
        let x = layout.footer.x;
        let y = layout.footer.y + 54;
        targets.extend([
            (SettingsFocus::Pin, IntRect::new(x, y, 100, 28)),
            (SettingsFocus::PinUp, IntRect::new(x + 106, y, 42, 28)),
            (SettingsFocus::PinDown, IntRect::new(x + 154, y, 42, 28)),
            (SettingsFocus::Reset, IntRect::new(x + 202, y, 110, 28)),
            (
                SettingsFocus::ResetCategory,
                IntRect::new(x + 318, y, 140, 28),
            ),
            (
                SettingsFocus::Close,
                IntRect::new(x + layout.footer.w - 130, y + 34, 130, 30),
            ),
        ]);
        if self.category == SettingsCategory::Audio {
            targets.extend([
                (
                    SettingsFocus::TestMicrophone,
                    IntRect::new(x, y + 34, 190, 30),
                ),
                (
                    SettingsFocus::RefreshDevices,
                    IntRect::new(x + 196, y + 34, 150, 30),
                ),
            ]);
        }
        targets
    }
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
        controller.set_focus(SettingsFocus::Row(0));
        controller.key(KeyCode::Down, false, false);
        assert_eq!(controller.view.focus, SettingsFocus::Pin);
        for _ in 0..5 {
            controller.key(KeyCode::Down, false, false);
        }
        assert_eq!(controller.view.focus, SettingsFocus::TestMicrophone);
        controller.set_focus(SettingsFocus::Row(0));
        controller.key(KeyCode::Up, false, false);
        assert_eq!(
            controller.view.focus,
            SettingsFocus::Category(SettingsCategory::Audio)
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
        let actions = controller.pointer(
            GuiPoint::new((row.x + row.w - 24) as f32, (row.y + 40) as f32),
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
            SettingsFocus::TestMicrophone,
            SettingsFocus::RefreshDevices,
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
