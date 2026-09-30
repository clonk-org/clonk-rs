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
        self.resize_book(
            surface.width() as i32,
            surface.height() as i32,
            gui,
            fonts,
            !startup_background,
        );
        let layout = self.layout();
        let native = self.book_layout(gui, fonts);
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
                pressed: matches!(
                    self.view.pressed,
                    Some(SettingsFocus::Close | SettingsFocus::CloseMicrophoneTest)
                ),
                highlighted: matches!(
                    self.view.focus,
                    SettingsFocus::Close | SettingsFocus::CloseMicrophoneTest
                ),
            },
            self.view.window,
            gamma,
        );
        if let Some(window) = self.view.window {
            text(
                surface,
                &gui.caption,
                IntRect::new(
                    layout.back.x + layout.back.w + 24,
                    layout.back.y + 2,
                    window.x + window.w - layout.back.x - layout.back.w - 48,
                    28,
                ),
                &self.view.context,
                [255, 255, 190, 255],
                gamma,
            );
        }
        if self.view.microphone_test_open {
            self.render_microphone_test(surface, &book, gamma);
            return;
        }
        text(
            surface,
            body_font,
            IntRect::new(layout.list.x, layout.search.y, 60, 26),
            "Search:",
            [0, 0, 0, 255],
            gamma,
        );
        let searching = self.view.focus == SettingsFocus::Search;
        if searching {
            self.view
                .search_edit
                .render(surface, body_font, layout.search, gamma);
        } else {
            book.field(surface, layout.search, gamma);
            text(
                surface,
                body_font,
                layout.search,
                &self.query,
                [55, 45, 32, 255],
                gamma,
            );
        }
        if let Some(hint) = self.search_placeholder() {
            // Beside the caret while focused, as text fields do elsewhere.
            let inset = if searching { 6 } else { 0 };
            text(
                surface,
                body_font,
                IntRect::new(
                    layout.search.x + inset,
                    layout.search.y,
                    layout.search.w - inset,
                    layout.search.h,
                ),
                hint,
                [140, 124, 100, 255],
                gamma,
            );
        }
        self.render_page_tab_rule(surface, &book, &layout, gamma);
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
            if let SettingsFocus::AudioPage(page) = focus {
                let selected = self.audio_page == page && self.query.trim().is_empty();
                let icon = self
                    .page_tab_icons(&layout)
                    .then(|| book.page_icon(page == AudioPage::Voice));
                book.page_tab(
                    surface,
                    rect,
                    page.label(),
                    icon.as_ref(),
                    selected,
                    highlighted,
                    gamma,
                );
                continue;
            }
            if focus == SettingsFocus::Group {
                book.combo(surface, rect, highlighted, gamma);
                text(
                    surface,
                    body_font,
                    IntRect::new(rect.x + 3, rect.y, rect.w - 24, rect.h),
                    &self.current_group().unwrap_or_default(),
                    [0, 0, 0, 255],
                    gamma,
                );
                continue;
            }
            let emphasized = highlighted || self.view.hover == Some(focus);
            let compact = layout.footer.w < 480;
            match focus {
                SettingsFocus::Modified => book.ink_toggle(
                    surface,
                    rect,
                    if compact { "Changed" } else { "Only changed" },
                    self.modified_only,
                    emphasized,
                    gamma,
                ),
                SettingsFocus::Advanced => book.ink_toggle(
                    surface,
                    rect,
                    &format!(
                        "{} ({})",
                        if compact { "Advanced" } else { "Show advanced" },
                        self.advanced_count()
                    ),
                    self.show_advanced,
                    emphasized,
                    gamma,
                ),
                // Drawn on their row, above the row's highlight.
                SettingsFocus::Pin | SettingsFocus::Reset => {}
                _ => {
                    let label = match focus {
                        SettingsFocus::PinUp => "Move up",
                        SettingsFocus::PinDown => "Move down",
                        SettingsFocus::ResetCategory => "Reset page",
                        SettingsFocus::TestMicrophone if compact => "Test mic",
                        SettingsFocus::TestMicrophone => "Test microphone",
                        SettingsFocus::RefreshDevices => "Refresh devices",
                        _ => "",
                    };
                    book.ink_link(surface, rect, label, emphasized, gamma);
                }
            }
        }
        self.render_input_hint(surface, &layout, small_font, gamma);
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
            if let Some(emphasis) = self.row_emphasis(index) {
                draw_row_emphasis(surface, rect, emphasis, gamma);
            }
            let setting = &self.settings[index];
            let focused = self.view.focus == SettingsFocus::Row(index);
            let editable = setting.value.is_editable() && setting.details.unavailable.is_none();
            // Changed settings are written in sienna ink; the footer names
            // the default Reset restores.
            let color = match (editable, setting.is_modified()) {
                (false, _) => [102, 90, 74, 255],
                (true, true) => [128, 45, 12, 255],
                (true, false) => [0, 0, 0, 255],
            };
            if let AdvancedConfigValue::Bool(checked) = setting.value {
                let checkbox = IntRect::new(rect.x, rect.y + 4, rect.w, 20);
                book.checkbox(surface, checkbox, "", checked, focused, gamma);
                let end = self.label_end(index, rect, rect.x + rect.w);
                text(
                    surface,
                    body_font,
                    IntRect::new(rect.x + 24, rect.y + 2, (end - rect.x - 24).max(1), 28),
                    &setting.label,
                    color,
                    gamma,
                );
                continue;
            }
            let end = self.label_end(index, rect, rect.x + rect.w - 182);
            text(
                surface,
                body_font,
                IntRect::new(rect.x, rect.y + 2, (end - rect.x).max(1), 28),
                &setting.label,
                color,
                gamma,
            );
            let value_rect = value_rect(rect);
            if let Some((_, edit)) = self.view.edit.as_mut().filter(|(i, _)| *i == index) {
                edit.render(surface, body_font, value_rect, gamma);
                continue;
            }
            let label = value_label(setting);
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
        self.render_row_actions(surface, &book, &layout, gamma);
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
                detail = detail_text(setting);
            }
        }
        if self.category == SettingsCategory::Audio {
            let help = self
                .view
                .selected
                .and_then(|index| self.settings.get(index))
                .and_then(|setting| setting.details.unavailable.as_deref())
                .unwrap_or(&description);
            wrapped_text(
                surface,
                small_font,
                IntRect::new(layout.footer.x, layout.footer.y, layout.footer.w, 46),
                if self.view.message.is_empty() {
                    help
                } else {
                    &self.view.message
                },
                [42, 33, 22, 255],
                gamma,
            );
        } else {
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
        }
    }

    /// Whether a choice list or a confirmation is open over the page. The
    /// caller draws it with [`Self::render_popup`] after the page, in a layer
    /// of its own where the presenter needs one, so the page's text cannot
    /// show through it.
    pub fn has_popup(&self) -> bool {
        self.view.choice.is_some()
            || self.view.display_confirmation.is_some()
            || self.view.reset_confirmation
    }

    /// Draws the open choice list or confirmation over the page [`Self::render`] drew.
    pub fn render_popup(
        &mut self,
        surface: &mut Surface,
        assets: &OptionsDlgAssets,
        gui: &ClonkFontSet,
        fonts: &BookFonts,
        gamma: Option<&GammaRamp>,
    ) {
        let layout = self.layout();
        let book = OptionsBook { assets, gui, fonts };
        let body_font = &fonts.book;
        let small_font = &fonts.book_small;
        if let (Some(prompt), Some(actions)) = (
            self.confirmation_prompt(),
            self.confirmation_actions(&layout),
        ) {
            // A card over the footer: the question and its two answers.
            let card = layout.footer;
            box_color(surface, card, 0x00d2_c6b0, gamma);
            book.field(surface, card, gamma);
            wrapped_text(
                surface,
                body_font,
                IntRect::new(card.x + 6, card.y + 6, card.w - 12, 48),
                &prompt,
                [40, 31, 21, 255],
                gamma,
            );
            for (index, (label, rect)) in actions.iter().enumerate() {
                book.ink_button(surface, *rect, label, index == 0, gamma);
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

impl SettingsController {
    /// Where a row's label must stop: short of its star and reset link when
    /// it shows them, otherwise at `plain`.
    fn label_end(&self, index: usize, row: IntRect, plain: i32) -> i32 {
        let star = star_rect(row).x;
        let pinned = self
            .settings
            .get(index)
            .is_some_and(|setting| self.pinned.contains(&setting.id));
        match (self.view.selected == Some(index), pinned) {
            (true, _) if self.resettable(index) => star - 58,
            (true, _) | (_, true) => star - 4,
            _ => plain,
        }
    }

    /// Stars on pinned rows, and the selected row's pin toggle and reset link.
    fn render_row_actions(
        &self,
        surface: &mut Surface,
        book: &OptionsBook,
        layout: &SettingsLayout,
        gamma: Option<&GammaRamp>,
    ) {
        let emphasized = |focus| self.view.focus == focus || self.view.hover == Some(focus);
        for index in self.visible_indices() {
            let Some(row) = self.row_rect(layout, index) else {
                continue;
            };
            let pinned = self
                .settings
                .get(index)
                .is_some_and(|setting| self.pinned.contains(&setting.id));
            let selected = self.view.selected == Some(index);
            // Every Quick row is pinned, so only its toggle is drawn there.
            let marked = pinned && self.category != SettingsCategory::Quick;
            if marked || selected {
                let focused = selected && emphasized(SettingsFocus::Pin);
                book.ink_star(surface, star_rect(row), pinned, focused, gamma);
            }
        }
        if let Some((_, rect)) = self
            .row_actions(layout)
            .into_iter()
            .find(|(focus, _)| *focus == SettingsFocus::Reset)
        {
            book.ink_link(
                surface,
                rect,
                "Reset",
                emphasized(SettingsFocus::Reset),
                gamma,
            );
        }
    }

    /// The rule the Audio page's tabs stand on, open under the chosen tab
    /// unless a search has replaced the page.
    fn render_page_tab_rule(
        &self,
        surface: &mut Surface,
        book: &OptionsBook,
        layout: &SettingsLayout,
        gamma: Option<&GammaRamp>,
    ) {
        if self.category != SettingsCategory::Audio {
            return;
        }
        let Some((_, chosen)) = self
            .targets(layout)
            .into_iter()
            .find(|(focus, _)| *focus == SettingsFocus::AudioPage(self.audio_page))
        else {
            return;
        };
        let opening = self
            .query
            .trim()
            .is_empty()
            .then_some((chosen.x + 2, chosen.x + chosen.w - 2));
        book.page_tab_rule(
            surface,
            (layout.list.x, layout.list.x + layout.list.w),
            chosen.y + chosen.h,
            opening,
            gamma,
        );
    }

    /// Right-aligns the focused control's keys on the footer's button row,
    /// in whatever room the buttons leave; omitted where it does not fit.
    fn render_input_hint(
        &self,
        surface: &mut Surface,
        layout: &SettingsLayout,
        font: &ClonkFont,
        gamma: Option<&GammaRamp>,
    ) {
        let links = self.footer_links(layout);
        let Some(row) = links.first().map(|(_, rect)| *rect) else {
            return;
        };
        let left = links
            .iter()
            .filter(|(focus, _)| *focus != SettingsFocus::TestMicrophone)
            .map(|(_, rect)| rect.x + rect.w)
            .max()
            .unwrap_or(row.x)
            + 12;
        let right = links
            .iter()
            .find(|(focus, _)| *focus == SettingsFocus::TestMicrophone)
            .map_or(layout.footer.x + layout.footer.w, |(_, rect)| rect.x - 12);
        let hint = self.input_hint();
        let width = font.measure(&hint, false).0;
        if width <= right - left {
            text(
                surface,
                font,
                IntRect::new(right - width - 6, row.y + 4, width + 6, row.h),
                &hint,
                [96, 80, 60, 255],
                gamma,
            );
        }
    }
}

/// A brown wash over the row (engine colours carry inverted alpha), plus an
/// ink bar at the left edge for the row the footer and keyboard act on.
fn draw_row_emphasis(
    surface: &mut Surface,
    row: IntRect,
    emphasis: RowEmphasis,
    gamma: Option<&GammaRamp>,
) {
    let band = IntRect::new(row.x - 4, row.y - 1, row.w + 8, row.h + 2);
    let (wash, marked) = match emphasis {
        RowEmphasis::Focused => (0xb26b_5030, true),
        RowEmphasis::Selected => (0xcc6b_5030, true),
        RowEmphasis::Hovered => (0xe86b_5030, false),
    };
    box_color(surface, band, wash, gamma);
    if marked {
        box_color(
            surface,
            IntRect::new(band.x, band.y, 3, band.h),
            0x0050_3820,
            gamma,
        );
    }
}

pub(super) fn box_color(
    surface: &mut Surface,
    rect: IntRect,
    color: u32,
    gamma: Option<&GammaRamp>,
) {
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

pub(super) fn text(
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

pub(super) fn wrapped_text(
    surface: &mut Surface,
    font: &ClonkFont,
    rect: IntRect,
    label: &str,
    color: [u8; 4],
    gamma: Option<&GammaRamp>,
) {
    let mut lines = vec![String::new()];
    for word in label.split_whitespace() {
        let line = lines.last_mut().unwrap();
        let candidate = if line.is_empty() {
            word.into()
        } else {
            format!("{line} {word}")
        };
        if !line.is_empty() && font.measure(&candidate, false).0 > rect.w - 6 {
            lines.push(word.into());
        } else {
            *line = candidate;
        }
    }
    let count = ((rect.h - 2) / font.line_height).max(0) as usize;
    if lines.len() > count && count > 0 {
        lines[count - 1].push('…');
    }
    for (index, line) in lines.iter().take(count).enumerate() {
        text(
            surface,
            font,
            IntRect::new(
                rect.x,
                rect.y + index as i32 * font.line_height,
                rect.w,
                font.line_height + 2,
            ),
            line,
            color,
            gamma,
        );
    }
}
