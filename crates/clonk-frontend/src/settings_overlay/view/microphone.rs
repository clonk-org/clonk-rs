use super::render::{box_color, text, wrapped_text};
use super::*;
use crate::classic_gui::ClassicButtonState;
use clonk_graphics::{GammaRamp, Surface};

impl SettingsController {
    pub(super) fn render_microphone_test(
        &self,
        surface: &mut Surface,
        book: &OptionsBook<'_>,
        gamma: Option<&GammaRamp>,
    ) {
        let layout = self.layout();
        let x = layout.list.x;
        let y = layout.search.y;
        let w = layout.list.w;
        let font = &book.fonts.book;
        let small = &book.fonts.book_small;
        let ink = [40, 31, 21, 255];
        text(
            surface,
            font,
            IntRect::new(x, y, w, 28),
            "Microphone test",
            ink,
            gamma,
        );
        wrapped_text(
            surface,
            small,
            IntRect::new(x, y + 34, w, 42),
            "Record 3 seconds, then listen. Only you hear this test.",
            ink,
            gamma,
        );
        book.group(
            surface,
            IntRect::new(x, y + 76, w, 120),
            "Audio devices",
            gamma,
        );
        for (offset, key, label) in [
            (96, "InputDevice", "Microphone:"),
            (124, "OutputDevice", "Output:"),
        ] {
            let value = self
                .settings
                .iter()
                .find(|setting| setting.id.section == "Voice" && setting.id.key == key)
                .map(|setting| match &setting.value {
                    AdvancedConfigValue::Choice { value, choices } => choices
                        .iter()
                        .find(|choice| choice.value == *value)
                        .map(|choice| choice.label.clone())
                        .unwrap_or_else(|| value.clone()),
                    value => value.serialized(),
                })
                .unwrap_or_else(|| "System default".into());
            text(
                surface,
                font,
                IntRect::new(x + 8, y + offset, 112, 26),
                label,
                ink,
                gamma,
            );
            text(
                surface,
                font,
                IntRect::new(x + 124, y + offset, w - 134, 26),
                &value,
                ink,
                gamma,
            );
        }
        wrapped_text(
            surface,
            small,
            IntRect::new(x + 8, y + 154, w - 16, 42),
            &self.view.audio_device_status,
            ink,
            gamma,
        );
        text(
            surface,
            small,
            IntRect::new(x, y + 200, w, 20),
            "Microphone level",
            ink,
            gamma,
        );
        let meter = IntRect::new(x + 3, y + 224, w - 6, 10);
        book.field(surface, meter, gamma);
        let level = self.view.microphone_level.clamp(0.0, 1.0);
        box_color(
            surface,
            IntRect::new(
                meter.x + 2,
                meter.y + 2,
                ((meter.w - 4) as f32 * level) as i32,
                meter.h - 4,
            ),
            0x003a784d,
            gamma,
        );
        wrapped_text(
            surface,
            small,
            IntRect::new(x, y + 244, w, 48),
            &self.view.microphone_status,
            ink,
            gamma,
        );
        for (focus, rect) in self.microphone_targets() {
            let label = match focus {
                SettingsFocus::RecordMicrophone if self.view.microphone_testing => "Stop test",
                SettingsFocus::RecordMicrophone => "Record & listen",
                SettingsFocus::RefreshDevices => "Refresh devices",
                _ => "Back to voice",
            };
            book.button(
                surface,
                rect,
                label,
                ClassicButtonState {
                    pressed: self.view.pressed == Some(focus),
                    highlighted: self.view.focus == focus,
                },
                gamma,
            );
        }
    }

    fn microphone_targets(&self) -> [(SettingsFocus, IntRect); 3] {
        let layout = self.layout();
        let y = layout.footer.y + layout.footer.h - 34;
        [
            (
                SettingsFocus::RecordMicrophone,
                IntRect::new(layout.list.x, y, 146, 28),
            ),
            (
                SettingsFocus::RefreshDevices,
                IntRect::new(layout.list.x + 154, y, 140, 28),
            ),
            (
                SettingsFocus::CloseMicrophoneTest,
                IntRect::new(layout.list.x + layout.list.w - 116, y, 116, 28),
            ),
        ]
    }

    pub(super) fn microphone_key(
        &mut self,
        key: KeyCode,
        shift: bool,
        control: bool,
    ) -> Vec<SettingsAction> {
        match key {
            KeyCode::Escape => return self.activate(SettingsFocus::CloseMicrophoneTest),
            KeyCode::Tab if control => {
                let actions = self.activate(SettingsFocus::CloseMicrophoneTest);
                self.key(key, shift, control);
                return actions;
            }
            KeyCode::Enter | KeyCode::Space => return self.activate(self.view.focus),
            KeyCode::Tab | KeyCode::Left | KeyCode::Right | KeyCode::Up | KeyCode::Down => {
                let targets = self.microphone_targets();
                let index = targets
                    .iter()
                    .position(|(focus, _)| *focus == self.view.focus)
                    .unwrap_or(0);
                let backwards =
                    matches!(key, KeyCode::Left | KeyCode::Up) || (key == KeyCode::Tab && shift);
                self.view.focus = targets[(index + if backwards { 2 } else { 1 }) % 3].0;
            }
            _ => {}
        }
        Vec::new()
    }

    pub(super) fn microphone_pointer(
        &mut self,
        point: GuiPoint,
        down: bool,
    ) -> Vec<SettingsAction> {
        let layout = self.layout();
        let target = self
            .microphone_targets()
            .into_iter()
            .chain([(SettingsFocus::CloseMicrophoneTest, layout.back)])
            .chain(
                SettingsCategory::ALL
                    .into_iter()
                    .zip(layout.tabs)
                    .map(|(category, rect)| (SettingsFocus::Category(category), rect)),
            )
            .find(|(_, rect)| contains(*rect, point))
            .map(|(focus, _)| focus);
        if down {
            self.view.pressed = target;
            if let Some(focus) = target {
                self.view.focus = focus;
            }
        } else if let Some(focus) = self
            .view
            .pressed
            .take()
            .filter(|focus| Some(*focus) == target)
        {
            if let SettingsFocus::Category(category) = focus {
                self.select_category(category);
                return vec![SettingsAction::CancelMicrophoneTest];
            }
            return self.activate(focus);
        }
        Vec::new()
    }
}
