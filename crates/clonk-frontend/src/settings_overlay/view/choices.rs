use super::*;
use crate::startup_options_advanced::AdvancedConfigChoice;

/// Height of one entry in an open choice list.
pub(super) const ITEM_HEIGHT: i32 = 26;
/// Entries a choice list shows before it scrolls.
pub(super) const VISIBLE_ITEMS: usize = 8;

pub(crate) struct ChoicePicker {
    pub index: usize,
    pub selected: usize,
    pub scroll: usize,
    pub pressed: Option<usize>,
}

impl SettingsController {
    pub(super) fn open_choices(&mut self, index: usize) {
        if let Some(Setting {
            value: AdvancedConfigValue::Choice { value, choices },
            ..
        }) = self.settings.get(index)
        {
            if choices.is_empty() {
                return;
            }
            let selected = choices
                .iter()
                .position(|choice| &choice.value == value)
                .unwrap_or(0);
            self.view.choice = Some(ChoicePicker {
                index,
                selected,
                scroll: selected.saturating_sub(7),
                pressed: None,
            });
            self.view.pressed = None;
        }
    }

    pub(super) fn choices(&self) -> &[AdvancedConfigChoice] {
        self.view
            .choice
            .as_ref()
            .and_then(|picker| self.settings.get(picker.index))
            .and_then(|setting| match &setting.value {
                AdvancedConfigValue::Choice { choices, .. } => Some(choices.as_slice()),
                _ => None,
            })
            .unwrap_or_default()
    }

    /// The open list: right under its value and aligned to it, as wide as
    /// its longest choice needs, opening upward when the page ends first.
    pub(super) fn choice_rect(&self) -> IntRect {
        let layout = self.layout();
        let choices = self.choices();
        let height = choices.len().min(VISIBLE_ITEMS) as i32 * ITEM_HEIGHT + 6;
        let longest = choices
            .iter()
            .map(|choice| choice.label.chars().count())
            .max()
            .unwrap_or(0) as i32;
        let Some(value) = self
            .view
            .choice
            .as_ref()
            .and_then(|picker| self.row_rect(&layout, picker.index))
            .map(value_rect)
        else {
            return IntRect::new(layout.list.x, layout.list.y, layout.list.w / 3, height);
        };
        let width = (longest * 9 + 28).clamp(value.w, layout.list.w);
        let below = value.y + value.h;
        let y = if below + height <= layout.panel.y + layout.panel.h - 8 {
            below
        } else {
            (value.y - height).max(layout.panel.y + 8)
        };
        IntRect::new(value.x + value.w - width, y, width, height)
    }

    /// The choice under the pointer, if any.
    fn choice_at(&self, point: GuiPoint) -> Option<usize> {
        let rect = self.choice_rect();
        let picker = self.view.choice.as_ref()?;
        (contains(rect, point) && point.y >= (rect.y + 3) as f32)
            .then(|| picker.scroll + ((point.y as i32 - rect.y - 3) / ITEM_HEIGHT) as usize)
            .filter(|index| *index < self.choices().len())
    }

    /// Highlights the choice under the pointer.
    pub(super) fn choice_hover(&mut self, point: GuiPoint) {
        if let Some(index) = self.choice_at(point) {
            if let Some(picker) = self.view.choice.as_mut() {
                picker.selected = index;
            }
        }
    }

    fn accept_choice(&mut self) -> Vec<SettingsAction> {
        let Some(picker) = self.view.choice.take() else {
            return Vec::new();
        };
        let Some(Setting {
            value: AdvancedConfigValue::Choice { choices, .. },
            ..
        }) = self.settings.get(picker.index)
        else {
            return Vec::new();
        };
        choices
            .get(picker.selected)
            .map(|choice| {
                vec![SettingsAction::Change(
                    picker.index,
                    AdvancedConfigValue::Choice {
                        value: choice.value.clone(),
                        choices: choices.clone(),
                    },
                )]
            })
            .unwrap_or_default()
    }

    pub(super) fn choice_key(&mut self, key: KeyCode) -> Vec<SettingsAction> {
        let count = self.choices().len();
        let Some(picker) = self.view.choice.as_mut() else {
            return Vec::new();
        };
        match key {
            KeyCode::Escape => self.view.choice = None,
            KeyCode::Enter | KeyCode::Space => return self.accept_choice(),
            KeyCode::Up | KeyCode::Left | KeyCode::PageUp => {
                picker.selected =
                    picker
                        .selected
                        .saturating_sub(if key == KeyCode::PageUp { 8 } else { 1 })
            }
            KeyCode::Down | KeyCode::Right | KeyCode::PageDown => {
                picker.selected = (picker.selected + if key == KeyCode::PageDown { 8 } else { 1 })
                    .min(count.saturating_sub(1))
            }
            KeyCode::Home => picker.selected = 0,
            KeyCode::End => picker.selected = count.saturating_sub(1),
            _ => {}
        }
        if let Some(picker) = self.view.choice.as_mut() {
            picker.scroll = picker.scroll.min(picker.selected);
            if picker.selected >= picker.scroll + VISIBLE_ITEMS {
                picker.scroll = picker.selected + 1 - VISIBLE_ITEMS;
            }
        }
        Vec::new()
    }

    pub(super) fn choice_type(&mut self, text: &str) {
        let selected = self.choices().iter().position(|choice| {
            choice
                .label
                .to_lowercase()
                .starts_with(&text.to_lowercase())
        });
        if let Some(picker) = self.view.choice.as_mut() {
            if let Some(selected) = selected {
                picker.selected = selected;
                picker.scroll = selected.saturating_sub(7);
            }
        }
    }

    pub(super) fn scroll_choices(&mut self, amount: i32) {
        let count = self.choices().len();
        if let Some(picker) = self.view.choice.as_mut() {
            picker.scroll = picker
                .scroll
                .saturating_add_signed(amount as isize)
                .min(count.saturating_sub(8));
            picker.selected = picker
                .selected
                .max(picker.scroll)
                .min((picker.scroll + 7).min(count.saturating_sub(1)));
        }
    }

    pub(super) fn choice_pointer(&mut self, point: GuiPoint, down: bool) -> Vec<SettingsAction> {
        if !contains(self.choice_rect(), point) {
            if down {
                self.view.choice = None;
            }
            return Vec::new();
        }
        let Some(index) = self.choice_at(point) else {
            return Vec::new();
        };
        let Some(picker) = self.view.choice.as_mut() else {
            return Vec::new();
        };
        if down {
            picker.pressed = Some(index);
            picker.selected = index;
        } else if picker.pressed.take() == Some(index) {
            return self.accept_choice();
        }
        Vec::new()
    }
}
