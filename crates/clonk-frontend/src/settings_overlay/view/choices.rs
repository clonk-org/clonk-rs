use super::*;
use crate::startup_options_advanced::AdvancedConfigChoice;

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

    pub(super) fn choice_rect(&self) -> IntRect {
        let layout = SettingsLayout::new(self.view.width, self.view.height);
        let height = self.choices().len().min(8) as i32 * 28 + 32;
        let top = self
            .view
            .choice
            .as_ref()
            .and_then(|picker| self.row_rect(&layout, picker.index))
            .map_or(layout.list.y, |row| row.y + 24);
        IntRect::new(
            layout.list.x,
            top.min(layout.panel.y + layout.panel.h - height - 8)
                .max(layout.panel.y + 32),
            layout.list.w,
            height,
        )
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
            if picker.selected >= picker.scroll + 8 {
                picker.scroll = picker.selected + 1 - 8;
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
        let rect = self.choice_rect();
        let count = self.choices().len();
        if !contains(rect, point) {
            if down {
                self.view.choice = None;
            }
            return Vec::new();
        }
        let Some(picker) = self.view.choice.as_mut() else {
            return Vec::new();
        };
        if point.y >= (rect.y + 28) as f32 {
            let index = picker.scroll + ((point.y as i32 - rect.y - 28) / 28).max(0) as usize;
            if index < count {
                if down {
                    picker.pressed = Some(index);
                    picker.selected = index;
                } else if picker.pressed.take() == Some(index) {
                    return self.accept_choice();
                }
            }
        }
        Vec::new()
    }
}
