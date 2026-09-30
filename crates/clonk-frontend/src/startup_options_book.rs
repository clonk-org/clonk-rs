//! The shared settings browser uses the original options book's artwork and
//! drawing primitives. The compatibility renderer continues to own its layout.
use super::*;
use crate::classic_gui::{draw_engine_box, ClassicButtonState};

/// Dark brown ink for the chosen sub-page tab and its focus underline.
const PAGE_TAB_INK: u32 = 0x0061_4a32;
/// Ink for the page's link text: a shade darker than [`PAGE_TAB_INK`] so
/// the book font's thin strokes stay legible.
const PAGE_TAB_INK_RGBA: [u8; 4] = [0x4e, 0x3a, 0x26, 255];
/// Gold of a pinned setting's star.
const STAR_GOLD: u32 = 0x00e0_b030;
/// Medium ink for the other tabs and the rule they stand on, darker than the
/// group-box ink so the strip holds its shape against the parchment.
const PAGE_TAB_RULE_INK: u32 = 0x0080_6a50;

pub(crate) struct OptionsBook<'a> {
    pub assets: &'a OptionsDlgAssets,
    pub gui: &'a ClonkFontSet,
    pub fonts: &'a BookFonts,
}

impl OptionsBook<'_> {
    pub fn layout(w: i32, h: i32, gui: &ClonkFontSet, book: &BookFonts) -> OptionsDlgLayout {
        let mut state = OptionsDlgState::new(ProgramSheetState::default());
        state.enable_voice_sheet(VoiceOptionsState::default());
        let mut layout = state.build_layout(w, h, gui, book);
        // Keep the native seven-tab sizing, in the browser's category order.
        let clips = layout.tab_clips;
        let icons = layout.tab_icons;
        let captions = layout.tab_captions;
        for (position, sheet) in voice_sheet::VOICE_SHEETS.iter().enumerate() {
            layout.tab_clips[position] = clips[sheet.index()];
            layout.tab_icons[position] = icons[sheet.index()];
            layout.tab_captions[position] = captions[sheet.index()];
        }
        layout
    }

    pub fn chrome(
        &self,
        surface: &mut Surface,
        layout: &OptionsDlgLayout,
        tabs: &[(&str, usize); 7],
        active: usize,
        tab_focused: bool,
        back: ClassicButtonState,
        startup_background: bool,
        gamma: Option<&GammaRamp>,
    ) {
        let (w, h) = (surface.width() as i32, surface.height() as i32);
        if startup_background {
            draw_image_bilinear(
                surface,
                &GuiRect::new(0.0, 0.0, w as f32, h as f32),
                &self.assets.background,
                gamma,
            );
        } else {
            draw_engine_box(surface, 0, 0, w - 1, h - 1, 0x80000000, gamma);
            // Over another screen the book sits on a darker panel, so that
            // screen's own buttons cannot be mistaken for the book's.
            let panel = popup_panel(layout);
            draw_engine_box(
                surface,
                panel.x,
                panel.y,
                panel.x + panel.w - 1,
                panel.y + panel.h - 1,
                0x6010_0c08,
                gamma,
            );
            draw_frame_dw(
                surface,
                panel.x,
                panel.y,
                panel.x + panel.w - 1,
                panel.y + panel.h - 1,
                0x70c0_a060,
                gamma,
            );
        }
        self.gui.title.draw_with_gamma(
            surface,
            layout.title_center.0,
            layout.title_center.1,
            "Options",
            YELLOW_FONT_RGBA,
            TextAlign::Center,
            true,
            gamma,
        );
        let b = layout.back_button;
        draw_bar(surface, &gui_rect(b), &self.assets.button, gamma);
        if back.highlighted {
            self.highlight(
                surface,
                IntRect::new(b.x + 5, b.y + 3, b.w - 10, b.h - 6),
                gamma,
            );
        }
        let font = self.gui.button_font(b.h);
        let offset = i32::from(back.pressed);
        font.draw_with_gamma(
            surface,
            (b.x + b.x + b.w - 1) / 2 + offset,
            (b.y + b.y + b.h - 1 - font.line_height) / 2 + offset,
            "Back",
            YELLOW_FONT_RGBA,
            TextAlign::Center,
            true,
            gamma,
        );
        for (index, tab) in tabs.iter().enumerate().filter(|(i, _)| *i != active) {
            self.tab(surface, layout, index, *tab, gamma);
        }
        draw_image_bilinear_white_pad(surface, &gui_rect(layout.paper), &self.assets.paper, gamma);
        self.tab(surface, layout, active, tabs[active], gamma);
        if tab_focused {
            let mut rect = layout.focus_highlight;
            rect.y += layout.tab_clips[active].1 - layout.tab_clips[0].1;
            self.highlight(surface, rect, gamma);
        }
    }

    fn tab(
        &self,
        surface: &mut Surface,
        layout: &OptionsDlgLayout,
        index: usize,
        (label, icon): (&str, usize),
        gamma: Option<&GammaRamp>,
    ) {
        let (x, y) = layout.tab_clips[index];
        draw_image_bilinear(
            surface,
            &GuiRect::new(x as f32, y as f32, 120.0, layout.tab_height as f32),
            &self.assets.tab_clip,
            gamma,
        );
        let cell = self.assets.option_icons.height();
        let image = crop_image(&self.assets.option_icons, cell * icon as u32, 0, cell, cell);
        let (x, y) = layout.tab_icons[index];
        let size = layout.tab_icon_size as f32;
        draw_image_bilinear(
            surface,
            &GuiRect::new(x as f32, y as f32, size, size),
            &image,
            gamma,
        );
        let (x, y) = layout.tab_captions[index];
        self.fonts.book_small.draw_with_gamma(
            surface,
            x,
            y,
            label,
            STARTUP_FONT_RGBA,
            TextAlign::Center,
            true,
            gamma,
        );
    }

    /// A sub-page tab inked onto the page like the book's group boxes: a
    /// folder tab standing on [`Self::page_tab_rule`]. The chosen tab rises
    /// lighter and opens into the page; the others sit lower and shaded.
    /// Keyboard focus underlines the caption.
    #[allow(clippy::too_many_arguments)]
    pub fn page_tab(
        &self,
        surface: &mut Surface,
        rect: IntRect,
        label: &str,
        icon: Option<&ImageData>,
        active: bool,
        focused: bool,
        gamma: Option<&GammaRamp>,
    ) {
        const SLANT: i32 = 6;
        let (left, right, bottom) = (rect.x, rect.x + rect.w, rect.y + rect.h);
        let top = rect.y + if active { 0 } else { 3 };
        let (wash, ink) = if active {
            (0xa0ff_f8e8, PAGE_TAB_INK)
        } else {
            (0xd880_6848, PAGE_TAB_RULE_INK)
        };
        fill_quad_dw(
            surface,
            &[
                (left + SLANT, top),
                (right - SLANT, top),
                (right, bottom),
                (left, bottom),
            ],
            wash,
            gamma,
        );
        for inset in 0..2 {
            draw_line_dw(
                surface,
                left + inset,
                bottom - 1,
                left + SLANT + inset,
                top + inset,
                ink,
                gamma,
            );
            draw_line_dw(
                surface,
                left + SLANT,
                top + inset,
                right - SLANT,
                top + inset,
                ink,
                gamma,
            );
            draw_line_dw(
                surface,
                right - SLANT - inset,
                top + inset,
                right - inset,
                bottom - 1,
                ink,
                gamma,
            );
        }
        let mut x = left + SLANT + 4;
        if let Some(icon) = icon {
            let size = bottom - top - 8;
            crate::startup_plrsel::draw_image_bilinear_modulated(
                surface,
                &GuiRect::new(x as f32, (top + 4) as f32, size as f32, size as f32),
                icon,
                if active { 0x00ff_ffff } else { 0x00b0_a898 },
                gamma,
            );
            x += size + 4;
        }
        let font = &self.fonts.book;
        let width = font.measure(label, true).0;
        let text_x = x + (right - SLANT - 4 - x - width).max(0) / 2;
        let text_y = top + (bottom - top - font.line_height) / 2;
        font.draw_with_gamma(
            surface,
            text_x,
            text_y,
            label,
            if active {
                STARTUP_FONT_RGBA
            } else {
                [78, 64, 46, 255]
            },
            TextAlign::Left,
            true,
            gamma,
        );
        if focused {
            let underline = text_y + font.line_height - 2;
            draw_line_dw(
                surface,
                text_x,
                underline,
                text_x + width,
                underline,
                PAGE_TAB_INK,
                gamma,
            );
        }
    }

    /// The two-pixel rule sub-page tabs stand on, broken under the chosen
    /// tab so it opens into the page.
    pub fn page_tab_rule(
        &self,
        surface: &mut Surface,
        (from, to): (i32, i32),
        y: i32,
        opening: Option<(i32, i32)>,
        gamma: Option<&GammaRamp>,
    ) {
        let spans = match opening {
            Some((start, end)) => vec![(from, start), (end, to)],
            None => vec![(from, to)],
        };
        for (start, end) in spans.into_iter().filter(|(start, end)| start < end) {
            for row in 0..2 {
                draw_line_dw(
                    surface,
                    start,
                    y + row,
                    end,
                    y + row,
                    PAGE_TAB_RULE_INK,
                    gamma,
                );
            }
        }
    }

    /// Book-font text in the page's ink, underlined while it has focus or the
    /// pointer: the page's actions read as annotations, not GUI buttons.
    pub fn ink_link(
        &self,
        surface: &mut Surface,
        rect: IntRect,
        label: &str,
        emphasized: bool,
        gamma: Option<&GammaRamp>,
    ) {
        let font = &self.fonts.book_small;
        let width = font.measure(label, true).0;
        let (x, y) = (rect.x + 2, rect.y + (rect.h - font.line_height) / 2);
        font.draw_with_gamma(
            surface,
            x,
            y,
            label,
            PAGE_TAB_INK_RGBA,
            TextAlign::Left,
            true,
            gamma,
        );
        if emphasized {
            let underline = y + font.line_height - 1;
            draw_line_dw(
                surface,
                x,
                underline,
                x + width,
                underline,
                PAGE_TAB_INK,
                gamma,
            );
        }
    }

    /// A check box inked onto the page, ticked when on, with its label.
    #[allow(clippy::too_many_arguments)]
    pub fn ink_toggle(
        &self,
        surface: &mut Surface,
        rect: IntRect,
        label: &str,
        checked: bool,
        emphasized: bool,
        gamma: Option<&GammaRamp>,
    ) {
        const SIZE: i32 = 12;
        let (x, y) = (rect.x + 2, rect.y + (rect.h - SIZE) / 2);
        for inset in 0..2 {
            draw_frame_dw(
                surface,
                x + inset,
                y + inset,
                x + SIZE - 1 - inset,
                y + SIZE - 1 - inset,
                PAGE_TAB_INK,
                gamma,
            );
        }
        if checked {
            for stroke in 0..2 {
                draw_line_dw(
                    surface,
                    x + 3,
                    y + 5 + stroke,
                    x + 5,
                    y + 8 + stroke,
                    PAGE_TAB_INK,
                    gamma,
                );
                draw_line_dw(
                    surface,
                    x + 5,
                    y + 8 + stroke,
                    x + 9,
                    y + 2 + stroke,
                    PAGE_TAB_INK,
                    gamma,
                );
            }
        }
        self.ink_link(
            surface,
            IntRect::new(rect.x + SIZE + 6, rect.y, rect.w - SIZE - 6, rect.h),
            label,
            emphasized,
            gamma,
        );
    }

    /// A five-pointed star inked into `rect`: gold for a setting pinned to
    /// Quick, an outline for one that is not.
    pub fn ink_star(
        &self,
        surface: &mut Surface,
        rect: IntRect,
        filled: bool,
        emphasized: bool,
        gamma: Option<&GammaRamp>,
    ) {
        let (cx, cy) = (
            rect.x as f32 + rect.w as f32 / 2.0,
            rect.y as f32 + rect.h as f32 / 2.0 + 1.0,
        );
        let outer = rect.w.min(rect.h) as f32 / 2.0;
        let points: Vec<(i32, i32)> = (0..10)
            .map(|point| {
                let radius = if point % 2 == 0 { outer } else { outer * 0.45 };
                let angle =
                    -std::f32::consts::FRAC_PI_2 + point as f32 * std::f32::consts::PI / 5.0;
                (
                    (cx + radius * angle.cos()).round() as i32,
                    (cy + radius * angle.sin()).round() as i32,
                )
            })
            .collect();
        let edges = || (0..10).map(|point| (points[point], points[(point + 1) % 10]));
        // Pinned stars are gold; the others are pale so they read as hollow on
        // a highlighted row.
        let center = (cx.round() as i32, cy.round() as i32);
        let fill = if filled { STAR_GOLD } else { 0x70ff_f8e8 };
        for (from, to) in edges() {
            fill_quad_dw(surface, &[center, from, to, to], fill, gamma);
        }
        for (from, to) in edges() {
            draw_line_dw(surface, from.0, from.1, to.0, to.1, PAGE_TAB_INK, gamma);
            if emphasized {
                draw_line_dw(
                    surface,
                    from.0 + 1,
                    from.1,
                    to.0 + 1,
                    to.1,
                    PAGE_TAB_INK,
                    gamma,
                );
            }
        }
    }

    /// The Sound and Voice chat tab icons: the options book's speaker, and the
    /// chat illustration its voice sheet uses where the icon sheet has it.
    pub fn page_icon(&self, voice: bool) -> ImageData {
        let icons = &self.assets.option_icons;
        let cell = icons.height();
        voice
            .then(|| {
                self.assets
                    .voice_icons
                    .as_ref()
                    .filter(|image| image.width() >= 128 && image.height() >= 320)
                    .map(|image| crop_image(image, 64, 256, 64, 64))
            })
            .flatten()
            .unwrap_or_else(|| crop_image(icons, cell * 2, 0, cell, cell))
    }

    pub fn highlight(&self, surface: &mut Surface, rect: IntRect, gamma: Option<&GammaRamp>) {
        draw_image_bilinear_additive(
            surface,
            &gui_rect(rect),
            &retained_blackened_image(&self.assets.button_highlight),
            gamma,
        );
    }

    pub fn field(&self, surface: &mut Surface, rect: IntRect, gamma: Option<&GammaRamp>) {
        for inset in 0..2 {
            draw_frame_dw(
                surface,
                rect.x + inset,
                rect.y + inset,
                rect.x + rect.w - inset,
                rect.y + rect.h - 1 - inset,
                EDIT_BORDER_COLOR,
                gamma,
            );
        }
    }

    pub fn group(
        &self,
        surface: &mut Surface,
        rect: IntRect,
        title: &str,
        gamma: Option<&GammaRamp>,
    ) {
        OptionsDlgScreen::draw_group_box(surface, self.fonts, &rect, title, gamma);
    }

    pub fn combo(
        &self,
        surface: &mut Surface,
        rect: IntRect,
        focused: bool,
        gamma: Option<&GammaRamp>,
    ) {
        OptionsDlgScreen::draw_combo(surface, self.assets, self.fonts, &rect, "", focused, gamma);
    }

    pub fn checkbox(
        &self,
        surface: &mut Surface,
        rect: IntRect,
        label: &str,
        checked: bool,
        focused: bool,
        gamma: Option<&GammaRamp>,
    ) {
        OptionsDlgScreen::draw_checkbox(
            surface,
            self.assets,
            self.fonts,
            &rect,
            label,
            checked,
            focused,
            gamma,
        );
    }

    pub fn button(
        &self,
        surface: &mut Surface,
        rect: IntRect,
        label: &str,
        state: ClassicButtonState,
        gamma: Option<&GammaRamp>,
    ) {
        OptionsDlgScreen::draw_small_button(
            surface,
            self.assets,
            self.fonts,
            &rect,
            label,
            state.highlighted,
            state.pressed,
            gamma,
        );
    }

    pub fn slider(
        &self,
        surface: &mut Surface,
        rect: IntRect,
        fraction: f64,
        gamma: Option<&GammaRamp>,
    ) {
        // ScrollBar::DrawElement / GetScrollPos (C4GuiContainers.cpp:446-479)
        // reserve 16 pixels for each arrow and another 16 for the Wipf pin.
        let position = (fraction.clamp(0.0, 1.0) * f64::from((rect.w - 48).max(0))).round() as i32;
        OptionsDlgScreen::draw_book_scrollbar(
            surface,
            self.assets,
            &rect,
            position,
            false,
            false,
            gamma,
        );
    }
}

/// The book's footprint when it opens over another screen: tabs, paper,
/// title and Back button, with a margin.
fn popup_panel(layout: &OptionsDlgLayout) -> IntRect {
    let back = layout.back_button;
    let left = layout.tab_clips[0].0.min(back.x) - 16;
    let right = (layout.paper.x + layout.paper.w).max(back.x + back.w) + 16;
    let top = layout.title_center.1 - 8;
    let bottom = back.y + back.h + 12;
    IntRect::new(left, top, right - left, bottom - top)
}

fn gui_rect(rect: IntRect) -> GuiRect {
    GuiRect::new(rect.x as f32, rect.y as f32, rect.w as f32, rect.h as f32)
}
