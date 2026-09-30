use super::*;
use crate::classic_gui::{
    draw_3d_frame, draw_clipped_text_with_markup, draw_engine_box, ClassicButtonState,
};
use crate::message_dialog::MessageDialogResources;
use clonk_graphics::clonk_font::{ClonkFont, TextAlign};
use clonk_graphics::{GammaRamp, Surface};

impl SettingsController {
    pub fn render(
        &mut self,
        surface: &mut Surface,
        resources: MessageDialogResources<'_>,
        book: Option<&crate::startup_options_dlg::BookFonts>,
        gamma: Option<&GammaRamp>,
    ) {
        let body_font = book.map_or(&resources.fonts.text, |fonts| &fonts.book);
        let small_font = book.map_or(&resources.fonts.main_small, |fonts| &fonts.book_small);
        self.resize(surface.width() as i32, surface.height() as i32);
        let layout = SettingsLayout::new(self.view.width, self.view.height);
        let panel = layout.panel;
        draw_engine_box(
            surface,
            0,
            0,
            self.view.width - 1,
            self.view.height - 1,
            0x80000000,
            gamma,
        );
        box_color(surface, panel, 0x00ded0b5, gamma);
        draw_3d_frame(surface, panel, gamma);
        resources.skin.draw_caption(
            surface,
            IntRect::new(panel.x, panel.y, panel.w, 32),
            "Settings",
            &resources.fonts.caption,
            [255, 255, 190, 255],
            TextAlign::Center,
            gamma,
        );
        text(
            surface,
            small_font,
            IntRect::new(panel.x + 12, panel.y + 32, panel.w - 24, 20),
            &self.view.context,
            [55, 45, 32, 255],
            gamma,
        );
        if self.view.focus == SettingsFocus::Search {
            self.view
                .search_edit
                .render(surface, &resources.fonts.text, layout.search, gamma);
        } else {
            box_color(surface, layout.search, 0x002d2923, gamma);
            let label = if self.query.is_empty() {
                "Search all settings..."
            } else {
                &self.query
            };
            text(
                surface,
                body_font,
                layout.search,
                label,
                [245, 240, 223, 255],
                gamma,
            );
        }
        let count = self.visible_indices().len();
        for (focus, rect) in self.targets(&layout) {
            if matches!(focus, SettingsFocus::Search | SettingsFocus::Row(_)) {
                continue;
            }
            let label = match focus {
                SettingsFocus::Category(category) => category.label().to_owned(),
                SettingsFocus::Modified => {
                    format!("{} Changed", if self.modified_only { "[x]" } else { "[ ]" })
                }
                SettingsFocus::Advanced => format!(
                    "{} Advanced",
                    if self.show_advanced { "[x]" } else { "[ ]" }
                ),
                SettingsFocus::Pin => {
                    if self
                        .view
                        .selected
                        .and_then(|i| self.settings.get(i))
                        .is_some_and(|s| self.pinned.contains(&s.id))
                    {
                        "Unpin".into()
                    } else {
                        "Pin".into()
                    }
                }
                SettingsFocus::PinUp => "Up".into(),
                SettingsFocus::PinDown => "Dn".into(),
                SettingsFocus::Reset => "Reset value".into(),
                SettingsFocus::ResetCategory => "Reset category".into(),
                SettingsFocus::TestMicrophone => "Record / stop mic test".into(),
                SettingsFocus::RefreshDevices => "Refresh devices".into(),
                SettingsFocus::Close => "Back / save".into(),
                _ => String::new(),
            };
            resources.skin.draw_button(
                surface,
                rect,
                &label,
                resources.fonts,
                ClassicButtonState {
                    pressed: self.view.pressed == Some(focus),
                    highlighted: self.view.focus == focus
                        || focus == SettingsFocus::Category(self.category),
                },
                gamma,
            );
        }
        if count == 0 {
            let message = if self.category == SettingsCategory::Quick && self.query.is_empty() {
                "Pin settings from any category to keep them here."
            } else {
                "No matches. Try another term or clear the filters."
            };
            text(
                surface,
                body_font,
                IntRect::new(layout.list.x, layout.list.y, layout.list.w, 48),
                message,
                [55, 45, 32, 255],
                gamma,
            );
        }
        let visible = self.visible_indices();
        for index in visible
            .iter()
            .copied()
            .skip(self.view.scroll)
            .take(self.page_size())
        {
            let Some(rect) = self.row_rect(&layout, index) else {
                continue;
            };
            let setting = &self.settings[index];
            let selected = self.view.selected == Some(index);
            box_color(
                surface,
                rect,
                if selected { 0x00c8b58d } else { 0x00e9ddc6 },
                gamma,
            );
            if self.view.focus == SettingsFocus::Row(index) {
                draw_3d_frame(surface, rect, gamma);
            }
            let editable = setting.value.is_editable() && setting.details.unavailable.is_none();
            let color = if editable {
                [40, 31, 21, 255]
            } else {
                [102, 90, 74, 255]
            };
            let marker = if self.pinned.contains(&setting.id) {
                "* "
            } else {
                ""
            };
            text(
                surface,
                body_font,
                IntRect::new(rect.x + 6, rect.y + 3, (rect.w - 170).max(1), 24),
                &format!("{marker}{}", setting.label),
                color,
                gamma,
            );
            let detail = format!(
                "{} · {}",
                setting.category.label(),
                setting.details.policy.label()
            );
            text(
                surface,
                small_font,
                IntRect::new(rect.x + 6, rect.y + 27, (rect.w - 170).max(1), 20),
                &detail,
                [89, 73, 48, 255],
                gamma,
            );
            let value_rect = IntRect::new(rect.x + rect.w - 158, rect.y + 6, 158, 34);
            if let Some((_, edit)) = self.view.edit.as_mut().filter(|(i, _)| *i == index) {
                edit.render(surface, &resources.fonts.text, value_rect, gamma);
                continue;
            }
            let label =
                setting
                    .details
                    .display_value
                    .clone()
                    .unwrap_or_else(|| match &setting.value {
                        AdvancedConfigValue::Bool(value) => {
                            if *value { "On" } else { "Off" }.into()
                        }
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
                        _ if setting.id.key.to_lowercase().contains("password") => "••••••".into(),
                        _ => setting.value.serialized(),
                    });
            let arrows = matches!(
                setting.value,
                AdvancedConfigValue::Integer { .. } | AdvancedConfigValue::Choice { .. }
            ) && !setting.details.binding;
            if arrows && editable {
                text(
                    surface,
                    body_font,
                    IntRect::new(value_rect.x, value_rect.y, 24, 28),
                    "<",
                    color,
                    gamma,
                );
                text(
                    surface,
                    body_font,
                    IntRect::new(value_rect.x + 134, value_rect.y, 24, 28),
                    ">",
                    color,
                    gamma,
                );
            }
            text(
                surface,
                body_font,
                IntRect::new(value_rect.x + 24, value_rect.y, 110, 28),
                &label,
                color,
                gamma,
            );
            if let AdvancedConfigValue::Integer { value, min, max } = setting.value {
                if max > min
                    && max - min <= 1000
                    && !setting.details.binding
                    && setting.details.policy != ApplyPolicy::DisplayPreview
                {
                    let track = IntRect::new(value_rect.x + 24, rect.y + 39, 110, 3);
                    box_color(surface, track, 0x00aa9671, gamma);
                    let filled = ((value - min) * 110 / (max - min)) as i32;
                    if filled > 0 {
                        box_color(
                            surface,
                            IntRect::new(track.x, track.y, filled, 3),
                            0x00665a2d,
                            gamma,
                        );
                    }
                }
            }
        }
        let list_bottom = layout.list.y + layout.list.h;
        if let Some((track, thumb)) = self.scrollbar() {
            box_color(surface, track, 0x00b5a381, gamma);
            box_color(surface, thumb, 0x00705d3b, gamma);
            draw_3d_frame(surface, thumb, gamma);
        }
        text(
            surface,
            small_font,
            IntRect::new(layout.panel.x + 12, list_bottom - 20, 148, 20),
            &format!("{} settings", count),
            [70, 58, 37, 255],
            gamma,
        );
        let mut description = "Tab: navigate · arrows: adjust · Enter: edit · Esc: back".to_owned();
        let mut detail = self.view.message.clone();
        if let Some(setting) = self.view.selected.and_then(|i| self.settings.get(i)) {
            description = if setting.details.description.is_empty() {
                format!("{} — {}", setting.label, setting.details.scope)
            } else {
                setting.details.description.clone()
            };
            if detail.is_empty() {
                detail = setting.details.unavailable.clone().unwrap_or_else(|| {
                    let active = setting
                        .details
                        .active_value
                        .as_ref()
                        .map(|value| format!(" · Active: {value}"))
                        .unwrap_or_default();
                    format!(
                        "{} · {}{active}",
                        setting.details.scope,
                        setting.details.policy.label()
                    )
                });
            }
        }
        if self.category == SettingsCategory::Audio && !self.view.microphone_status.is_empty() {
            if self.view.message.is_empty() {
                detail = self.view.microphone_status.clone();
            }
            let level = self.view.microphone_level.clamp(0.0, 1.0);
            let meter = IntRect::new(layout.footer.x, layout.footer.y + 48, layout.footer.w, 3);
            box_color(surface, meter, 0x00b5a381, gamma);
            if level > 0.0 {
                box_color(
                    surface,
                    IntRect::new(meter.x, meter.y, (meter.w as f32 * level) as i32, 3),
                    0x003a784d,
                    gamma,
                );
            }
        }
        text(
            surface,
            small_font,
            IntRect::new(layout.footer.x, layout.footer.y, layout.footer.w, 24),
            &description,
            [42, 33, 22, 255],
            gamma,
        );
        text(
            surface,
            small_font,
            IntRect::new(layout.footer.x, layout.footer.y + 24, layout.footer.w, 22),
            &detail,
            [90, 61, 31, 255],
            gamma,
        );
        if self.view.display_confirmation.is_some() || self.view.reset_confirmation {
            box_color(surface, layout.footer, 0x00ded0b5, gamma);
            let prompt = self
                .view
                .display_confirmation
                .map(|seconds| format!("Keep these display settings? Reverting in {seconds}s."))
                .unwrap_or_else(|| {
                    "Reset visible category settings? Display and saved progress are kept.".into()
                });
            text(
                surface,
                body_font,
                IntRect::new(layout.footer.x, layout.footer.y + 8, layout.footer.w, 44),
                &prompt,
                [40, 31, 21, 255],
                gamma,
            );
            for (offset, label) in [
                (
                    0,
                    if self.view.reset_confirmation {
                        "Reset"
                    } else {
                        "Keep"
                    },
                ),
                (
                    160,
                    if self.view.reset_confirmation {
                        "Cancel"
                    } else {
                        "Revert"
                    },
                ),
            ] {
                resources.skin.draw_button(
                    surface,
                    IntRect::new(layout.footer.x + offset, layout.footer.y + 84, 150, 30),
                    label,
                    resources.fonts,
                    Default::default(),
                    gamma,
                );
            }
        }
        if let Some(picker) = self.view.choice.as_ref() {
            let rect = self.choice_rect();
            box_color(surface, rect, 0x00eee1c9, gamma);
            draw_3d_frame(surface, rect, gamma);
            text(
                surface,
                small_font,
                IntRect::new(rect.x + 4, rect.y, rect.w - 8, 28),
                "Choose a value · Enter: select · Esc: cancel",
                [55, 45, 32, 255],
                gamma,
            );
            for (index, choice) in self
                .choices()
                .iter()
                .enumerate()
                .skip(picker.scroll)
                .take(8)
            {
                let row = IntRect::new(
                    rect.x + 3,
                    rect.y + 28 + (index - picker.scroll) as i32 * 28,
                    rect.w - 6,
                    28,
                );
                if picker.selected == index {
                    box_color(surface, row, 0x00c8b58d, gamma);
                }
                text(
                    surface,
                    body_font,
                    row,
                    &choice.label,
                    [40, 31, 21, 255],
                    gamma,
                );
            }
        }
    }
}

fn box_color(surface: &mut Surface, rect: IntRect, color: u32, gamma: Option<&GammaRamp>) {
    if rect.w > 0 && rect.h > 0 {
        draw_engine_box(
            surface,
            rect.x,
            rect.y,
            rect.x + rect.w - 1,
            rect.y + rect.h - 1,
            color,
            gamma,
        );
    }
}
fn text(
    surface: &mut Surface,
    font: &ClonkFont,
    rect: IntRect,
    label: &str,
    color: [u8; 4],
    gamma: Option<&GammaRamp>,
) {
    draw_clipped_text_with_markup(
        surface,
        font,
        rect.x + 3,
        rect.y + 2,
        label,
        color,
        TextAlign::Left,
        gamma,
        rect,
        false,
    );
}
