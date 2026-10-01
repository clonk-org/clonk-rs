use super::*;
use crate::classic_gui::{draw_clipped_text_with_markup, draw_engine_box, ClassicButtonState};
use crate::startup_options_controls::CONTROL_KEY_LABELS;
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
        if self.view.window.is_some() {
            self.render_popup_header(surface, &book, &layout, small_font, gamma);
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
            if let Some((label, icon)) = match focus {
                SettingsFocus::AudioPage(page) => {
                    Some((page.label(), book.page_icon(page == AudioPage::Voice)))
                }
                SettingsFocus::ControlsPage(page) => Some((
                    page.label(),
                    // The book's own pictures: its keys, gamepad and gears.
                    book.option_icon(match page {
                        ControlsPage::Keyboard => 3,
                        ControlsPage::Controller => 4,
                        ControlsPage::General => 0,
                    }),
                )),
                _ => None,
            } {
                let selected =
                    self.current_page_tab() == Some(focus) && self.query.trim().is_empty();
                book.page_tab(
                    surface,
                    rect,
                    label,
                    self.page_tab_icons(&layout).then_some(&icon),
                    selected,
                    highlighted,
                    gamma,
                );
                continue;
            }
            if let SettingsFocus::ControlSet(index) = focus {
                if let Some(device) = self.controls_page.device() {
                    book.control_set_picture(
                        surface,
                        rect,
                        (device, index),
                        self.control_set_page().map(|set| set.index) == Some(index),
                        self.view.hover == Some(focus),
                        highlighted,
                        gamma,
                    );
                }
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
                // Drawn on its row, above the row's highlight.
                SettingsFocus::TestMicrophone => {}
                _ => {
                    let reset = self.page_reset_label();
                    let label = match focus {
                        SettingsFocus::ResetCategory => reset.as_str(),
                        SettingsFocus::RefreshDevices => "Refresh devices",
                        _ => "",
                    };
                    book.ink_link(surface, rect, label, emphasized, gamma);
                }
            }
        }
        self.render_control_set_name(surface, &layout, body_font, gamma);
        self.render_input_hint(surface, &layout, small_font, gamma);
        let visible = self.visible_indices();
        if visible.is_empty() {
            text(
                surface,
                body_font,
                layout.list,
                "No matches. Try another term or clear the filters.",
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
            if self.control_set_page().is_some() {
                self.render_binding_cell(surface, &book, rect, index, gamma);
                continue;
            }
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
                let end = self.label_end(rect, rect.x + rect.w);
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
            let end = self.label_end(rect, rect.x + rect.w - 182);
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
        let detail = self.footer_detail();
        if let Some(setting) = self.view.selected.and_then(|i| self.settings.get(i)) {
            description = if setting.details.description.is_empty() {
                setting.label.clone()
            } else {
                setting.details.description.clone()
            };
        }
        if self.control_set_page().is_some() {
            // The grid names each command, so one line says what the
            // selected key does or what just happened.
            text(
                surface,
                small_font,
                IntRect::new(layout.footer.x, layout.footer.y, layout.footer.w, 20),
                &detail,
                [90, 61, 31, 255],
                gamma,
            );
        } else if self.category == SettingsCategory::Audio {
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
            box_color(surface, rect, 0x00d8_ccb8, gamma);
            book.field(surface, rect, gamma);
            let current = self
                .settings
                .get(picker.index)
                .map(|setting| setting.value.serialized());
            for (index, choice) in self
                .choices()
                .iter()
                .enumerate()
                .skip(picker.scroll)
                .take(choices::VISIBLE_ITEMS)
            {
                let item = IntRect::new(
                    rect.x + 3,
                    rect.y + 3 + (index - picker.scroll) as i32 * choices::ITEM_HEIGHT,
                    rect.w - 6,
                    choices::ITEM_HEIGHT,
                );
                if picker.selected == index {
                    box_color(surface, item, 0xb26b_5030, gamma);
                }
                if current.as_deref() == Some(choice.value.as_str()) {
                    book.ink_tick(surface, item.x + 3, item.y + (item.h - 12) / 2, gamma);
                }
                text(
                    surface,
                    body_font,
                    IntRect::new(item.x + 18, item.y + 1, item.w - 20, item.h),
                    &choice.label,
                    [40, 31, 21, 255],
                    gamma,
                );
            }
        }
    }
}

impl SettingsController {
    /// Where a row's label must stop: short of a link inked into the row,
    /// otherwise at `plain`.
    fn label_end(&self, row: IntRect, plain: i32) -> i32 {
        self.row_actions(&self.layout())
            .into_iter()
            .find(|(_, link)| link.y >= row.y && link.y < row.y + row.h)
            .map_or(plain, |(_, link)| link.x - 4)
    }

    /// The links inked into rows, above the rows' highlights.
    fn render_row_actions(
        &self,
        surface: &mut Surface,
        book: &OptionsBook,
        layout: &SettingsLayout,
        gamma: Option<&GammaRamp>,
    ) {
        for (focus, rect) in self.row_actions(layout) {
            let emphasized = self.view.focus == focus || self.view.hover == Some(focus);
            book.ink_link(surface, rect, "Test", emphasized, gamma);
        }
    }

    /// The rule a page's tabs stand on, open under the chosen tab unless a
    /// search has replaced the page.
    fn render_page_tab_rule(
        &self,
        surface: &mut Surface,
        book: &OptionsBook,
        layout: &SettingsLayout,
        gamma: Option<&GammaRamp>,
    ) {
        let Some((_, chosen)) = self
            .page_tab_rects(layout)
            .into_iter()
            .find(|(focus, _)| Some(*focus) == self.current_page_tab())
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

    /// One binding of a control set: the command's key cap from the classic
    /// control sheets, its name, and the key or button bound to it.
    fn render_binding_cell(
        &self,
        surface: &mut Surface,
        book: &OptionsBook,
        cell: IntRect,
        index: usize,
        gamma: Option<&GammaRamp>,
    ) {
        let setting = &self.settings[index];
        let Some(binding) = setting.details.control else {
            return;
        };
        let emphasis = self.row_emphasis(index);
        if let Some(emphasis) = emphasis {
            draw_row_emphasis(surface, cell, emphasis, gamma);
        }
        if self.view.conflict == Some(index) {
            book.warning_frame(surface, cell, gamma);
        }
        let capturing = self.view.capturing == Some(index);
        let size = (cell.h - 6).min(48);
        let cap = IntRect::new(cell.x + 4, cell.y + (cell.h - size) / 2, size, size);
        book.command_key(
            surface,
            cap,
            binding.command,
            capturing,
            capturing || emphasis.is_some(),
            gamma,
        );
        let font = if cell.h >= 56 {
            &book.fonts.book
        } else {
            &book.fonts.book_small
        };
        let x = cap.x + cap.w + 6;
        let width = cell.x + cell.w - x;
        let middle = cell.y + cell.h / 2;
        let ink = if setting.is_modified() {
            [128, 45, 12, 255]
        } else {
            [0, 0, 0, 255]
        };
        text(
            surface,
            font,
            IntRect::new(x, middle - font.line_height, width, font.line_height),
            CONTROL_KEY_LABELS[binding.command],
            ink,
            gamma,
        );
        let chip = IntRect::new(x + 2, middle + 1, width - 2, font.line_height);
        let key = value_label(setting);
        if capturing {
            let prompt = match binding.set.device {
                ControlDevice::Keyboard => "Press a key…",
                ControlDevice::Gamepad => "Press a button…",
            };
            book.key_chip(
                surface,
                font,
                chip,
                prompt,
                OptionsBook::WARNING_INK_RGBA,
                gamma,
            );
        } else if key == NOT_BOUND {
            text(surface, font, chip, &key, [140, 124, 100, 255], gamma);
        } else {
            book.key_chip(surface, font, chip, &key, ink, gamma);
        }
    }

    /// A popup's header in its paper's margin: what the screen behind is
    /// doing, and the mark that closes settings.
    fn render_popup_header(
        &self,
        surface: &mut Surface,
        book: &OptionsBook,
        layout: &SettingsLayout,
        font: &ClonkFont,
        gamma: Option<&GammaRamp>,
    ) {
        let close = layout.back;
        let closing = [SettingsFocus::Close, SettingsFocus::CloseMicrophoneTest];
        let emphasized = closing.contains(&self.view.focus)
            || self
                .view
                .hover
                .is_some_and(|focus| closing.contains(&focus))
            || self
                .view
                .pressed
                .is_some_and(|focus| closing.contains(&focus));
        book.ink_close(surface, close, emphasized, gamma);
        // Where room is short, the status keeps its first clause.
        let end = close.x - 10;
        let context = self.view.context.as_str();
        let first = context.split(" · ").next().unwrap_or_default();
        if let Some((shown, width)) = [context, first]
            .into_iter()
            .map(|candidate| (candidate, font.measure(candidate, false).0 + 6))
            .find(|(_, width)| *width <= end - layout.list.x)
        {
            text(
                surface,
                font,
                IntRect::new(end - width, close.y, width, close.h),
                shown,
                [96, 80, 60, 255],
                gamma,
            );
        }
    }

    /// Names the chosen control set beside the pictures, where it fits.
    fn render_control_set_name(
        &self,
        surface: &mut Surface,
        layout: &SettingsLayout,
        font: &ClonkFont,
        gamma: Option<&GammaRamp>,
    ) {
        let (Some(set), Some(caption), Some((_, last))) = (
            self.control_set_page(),
            self.control_set_caption(),
            self.control_set_pictures(layout).last().copied(),
        ) else {
            return;
        };
        let x = last.x + last.w + 14;
        let room = layout.list.x + layout.list.w - x;
        // Where room is short, the players' names outlast the set's name,
        // which its glowing picture already shows.
        let users = self
            .set_user_names(set)
            .map(|names| format!("Used by {names}"));
        if let Some(name) = [Some(caption), users, Some(set.label())]
            .into_iter()
            .flatten()
            .find(|name| font.measure(name, false).0 + 6 <= room)
        {
            text(
                surface,
                font,
                IntRect::new(x, last.y + (last.h - 26) / 2, room, 26),
                &name,
                [40, 31, 21, 255],
                gamma,
            );
        }
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
            .map(|(_, rect)| rect.x + rect.w)
            .max()
            .unwrap_or(row.x)
            + 12;
        let right = layout.footer.x + layout.footer.w;
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
    // The line starts 2 px into `rect`, so a tight `rect` would cut off its
    // descenders wherever glyphs fill the line, as they do when scaled.
    let clip = IntRect::new(rect.x, rect.y, rect.w, rect.h.max(font.line_height + 2));
    draw_clipped_text_with_markup(
        surface,
        font,
        rect.x + 3,
        rect.y + 2,
        label,
        color,
        TextAlign::Left,
        gamma,
        clip,
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
