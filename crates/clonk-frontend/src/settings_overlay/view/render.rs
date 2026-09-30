use super::*;
use crate::classic_gui::{draw_clipped_text_with_markup, draw_engine_box, ClassicButtonState};
use crate::startup_options_dlg::OptionsDlgAssets;
use clonk_graphics::clonk_font::{ClonkFont, TextAlign};
use clonk_graphics::{GammaRamp, Surface};

impl SettingsController {
    pub fn render(
        &mut self,
        surface: &mut Surface,
        assets: &OptionsDlgAssets,
        gui: &ClonkFontSet,
        fonts: &BookFonts,
        startup_background: bool,
        gamma: Option<&GammaRamp>,
    ) {
        self.resize_book(surface.width() as i32, surface.height() as i32, gui, fonts);
        let layout = self.layout();
        let native = OptionsBook::layout(self.view.width, self.view.height, gui, fonts);
        let book = OptionsBook { assets, gui, fonts };
        let body_font = &fonts.book;
        let small_font = &fonts.book_small;
        let tabs = SettingsCategory::ALL.map(|category| (category.label(), category.book_icon()));
        let active = SettingsCategory::ALL
            .iter()
            .position(|c| *c == self.category)
            .unwrap_or(0);
        book.chrome(
            surface,
            &native,
            &tabs,
            active,
            self.view.focus == SettingsFocus::Category(self.category),
            ClassicButtonState {
                pressed: self.view.pressed == Some(SettingsFocus::Close),
                highlighted: self.view.focus == SettingsFocus::Close,
            },
            startup_background,
            gamma,
        );
        if !startup_background {
            text(
                surface,
                &gui.caption,
                IntRect::new(
                    layout.back.x + layout.back.w + 24,
                    layout.back.y + 2,
                    self.view.width - layout.back.x - layout.back.w - 48,
                    28,
                ),
                &self.view.context,
                [255, 255, 190, 255],
                gamma,
            );
        }
        text(
            surface,
            body_font,
            IntRect::new(layout.list.x, layout.search.y, 60, 26),
            "Search:",
            [0, 0, 0, 255],
            gamma,
        );
        if self.view.focus == SettingsFocus::Search {
            self.view
                .search_edit
                .render(surface, body_font, layout.search, gamma);
        } else {
            book.field(surface, layout.search, gamma);
            text(
                surface,
                body_font,
                layout.search,
                if self.query.is_empty() {
                    "All settings (Ctrl+F)"
                } else {
                    &self.query
                },
                [55, 45, 32, 255],
                gamma,
            );
        }
        for (focus, rect) in self.targets(&layout) {
            if matches!(
                focus,
                SettingsFocus::Search
                    | SettingsFocus::Row(_)
                    | SettingsFocus::Category(_)
                    | SettingsFocus::Close
            ) {
                continue;
            }
            let highlighted = self.view.focus == focus;
            if matches!(focus, SettingsFocus::Modified | SettingsFocus::Advanced) {
                let (label, checked) = if focus == SettingsFocus::Modified {
                    ("Changed", self.modified_only)
                } else {
                    ("Advanced", self.show_advanced)
                };
                book.checkbox(surface, rect, label, checked, highlighted, gamma);
                continue;
            }
            let label = match focus {
                SettingsFocus::Pin => {
                    if self
                        .view
                        .selected
                        .and_then(|i| self.settings.get(i))
                        .is_some_and(|s| self.pinned.contains(&s.id))
                    {
                        "Unpin"
                    } else {
                        "Pin"
                    }
                }
                SettingsFocus::PinUp => "Up",
                SettingsFocus::PinDown => "Down",
                SettingsFocus::Reset => "Reset value",
                SettingsFocus::ResetCategory => "Reset page",
                SettingsFocus::TestMicrophone => "Record / stop mic test",
                SettingsFocus::RefreshDevices => "Refresh devices",
                _ => "",
            };
            book.button(
                surface,
                rect,
                label,
                ClassicButtonState {
                    pressed: self.view.pressed == Some(focus),
                    highlighted,
                },
                gamma,
            );
        }
        let visible = self.visible_indices();
        if visible.is_empty() {
            text(
                surface,
                body_font,
                layout.list,
                if self.category == SettingsCategory::Quick && self.query.is_empty() {
                    "Pin settings from any page to keep them here."
                } else {
                    "No matches. Try another term or clear the filters."
                },
                [40, 31, 21, 255],
                gamma,
            );
        }
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
            let focused = self.view.focus == SettingsFocus::Row(index);
            let editable = setting.value.is_editable() && setting.details.unavailable.is_none();
            let color = if editable {
                [0, 0, 0, 255]
            } else {
                [102, 90, 74, 255]
            };
            if let AdvancedConfigValue::Bool(checked) = setting.value {
                let checkbox = IntRect::new(rect.x, rect.y + 4, rect.w, 20);
                book.checkbox(surface, checkbox, "", checked, focused, gamma);
                text(
                    surface,
                    body_font,
                    IntRect::new(rect.x + 24, rect.y + 2, rect.w - 24, 28),
                    &setting.label,
                    color,
                    gamma,
                );
                continue;
            }
            text(
                surface,
                body_font,
                IntRect::new(rect.x, rect.y + 2, (rect.w - 182).max(1), 28),
                &setting.label,
                color,
                gamma,
            );
            let value_rect = value_rect(rect);
            if let Some((_, edit)) = self.view.edit.as_mut().filter(|(i, _)| *i == index) {
                edit.render(surface, body_font, value_rect, gamma);
                continue;
            }
            let label =
                setting
                    .details
                    .display_value
                    .clone()
                    .unwrap_or_else(|| match &setting.value {
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
            if let AdvancedConfigValue::Integer { value, min, max } = setting.value {
                if max > min
                    && max - min <= 1000
                    && !setting.details.binding
                    && setting.details.policy != ApplyPolicy::DisplayPreview
                {
                    let track = slider_rect(rect);
                    book.slider(
                        surface,
                        track,
                        (value - min) as f64 / (max - min) as f64,
                        gamma,
                    );
                    let number = IntRect::new(track.x + track.w + 6, value_rect.y, 40, 26);
                    if focused {
                        book.field(surface, number, gamma);
                    }
                    text(surface, body_font, number, &label, color, gamma);
                    continue;
                }
            }
            if matches!(setting.value, AdvancedConfigValue::Choice { .. }) {
                book.combo(surface, value_rect, focused, gamma);
                text(
                    surface,
                    body_font,
                    IntRect::new(
                        value_rect.x + 3,
                        value_rect.y,
                        value_rect.w - 24,
                        value_rect.h,
                    ),
                    &label,
                    color,
                    gamma,
                );
            } else {
                book.field(surface, value_rect, gamma);
                if focused {
                    book.highlight(surface, value_rect, gamma);
                }
                text(surface, body_font, value_rect, &label, color, gamma);
            }
        }
        if let Some((track, thumb)) = self.scrollbar() {
            book.field(surface, track, gamma);
            box_color(surface, thumb, 0x0094846a, gamma);
        }
        let mut description = "Tab: navigate · arrows: adjust · Enter: edit · Esc: back".to_owned();
        let mut detail = self.view.message.clone();
        if let Some(setting) = self.view.selected.and_then(|i| self.settings.get(i)) {
            description = if setting.details.description.is_empty() {
                setting.label.clone()
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
            let meter = IntRect::new(layout.footer.x, layout.footer.y + 69, layout.footer.w, 3);
            box_color(surface, meter, 0x00a4947a, gamma);
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
            IntRect::new(layout.footer.x, layout.footer.y, layout.footer.w, 20),
            &description,
            [42, 33, 22, 255],
            gamma,
        );
        text(
            surface,
            small_font,
            IntRect::new(layout.footer.x, layout.footer.y + 20, layout.footer.w, 20),
            &detail,
            [90, 61, 31, 255],
            gamma,
        );
        if self.view.display_confirmation.is_some() || self.view.reset_confirmation {
            box_color(surface, layout.footer, 0x00c7bca9, gamma);
            let prompt = self
                .view
                .display_confirmation
                .map(|seconds| format!("Keep these display settings? Reverting in {seconds}s."))
                .unwrap_or_else(|| "Reset this page? Display and saved progress are kept.".into());
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
                book.button(
                    surface,
                    IntRect::new(layout.footer.x + offset, layout.footer.y + 70, 150, 28),
                    label,
                    Default::default(),
                    gamma,
                );
            }
        }
        if let Some(picker) = self.view.choice.as_ref() {
            let rect = self.choice_rect();
            box_color(surface, rect, 0x00d5c9b5, gamma);
            book.field(surface, rect, gamma);
            text(
                surface,
                small_font,
                IntRect::new(rect.x + 4, rect.y, rect.w - 8, 28),
                "Enter: select · Esc: cancel",
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
                    book.highlight(surface, row, gamma);
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
