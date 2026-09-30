//! The shared settings browser uses the original options book's artwork and
//! drawing primitives. The compatibility renderer continues to own its layout.
use super::*;
use crate::classic_gui::{draw_engine_box, ClassicButtonState};

/// Dark brown ink for the chosen sub-page tab and its focus underline.
const PAGE_TAB_INK: u32 = 0x0061_4a32;
/// Ink for the page's link text: a shade darker than [`PAGE_TAB_INK`] so
/// the book font's thin strokes stay legible.
const PAGE_TAB_INK_RGBA: [u8; 4] = [0x4e, 0x3a, 0x26, 255];
/// The popup window's dark wooden frame and the gilt line inside it.
const WINDOW_FRAME_WOOD: u32 = 0x0030_1c0c;
const WINDOW_FRAME_GILT: u32 = 0x40c0_a060;
/// Medium ink for the other tabs and the rule they stand on, darker than the
/// group-box ink so the strip holds its shape against the parchment.
const PAGE_TAB_RULE_INK: u32 = 0x0080_6a50;

pub(crate) struct OptionsBook<'a> {
    pub assets: &'a OptionsDlgAssets,
    pub gui: &'a ClonkFontSet,
    pub fonts: &'a BookFonts,
}

/// What the options book draws with: its title, Back button, tab strip and
/// paper, in screen coordinates. The sheets' own control layouts stay with
/// the compatibility renderer.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct BookLayout {
    pub title_center: (i32, i32),
    pub back_button: IntRect,
    pub tabular: IntRect,
    pub paper: IntRect,
    pub sheet: IntRect,
    pub tab_clips: [(i32, i32); 7],
    pub tab_icons: [(i32, i32); 7],
    pub tab_captions: [(i32, i32); 7],
    pub focus_highlight: IntRect,
    pub tab_height: i32,
    pub tab_icon_size: i32,
}

impl BookLayout {
    /// The same layout moved by `(dx, dy)`, as when the book opens in a window.
    pub fn offset(self, dx: i32, dy: i32) -> Self {
        let rect = |r: IntRect| IntRect::new(r.x + dx, r.y + dy, r.w, r.h);
        let point = |(x, y): (i32, i32)| (x + dx, y + dy);
        Self {
            title_center: point(self.title_center),
            back_button: rect(self.back_button),
            tabular: rect(self.tabular),
            paper: rect(self.paper),
            sheet: rect(self.sheet),
            tab_clips: self.tab_clips.map(point),
            tab_icons: self.tab_icons.map(point),
            tab_captions: self.tab_captions.map(point),
            focus_highlight: rect(self.focus_highlight),
            ..self
        }
    }
}

impl OptionsBook<'_> {
    /// Where settings opened over another screen sit: the whole options
    /// screen, centred and inset so the screen behind stays in view, never
    /// smaller than the book's 640x480 minimum.
    pub fn window(w: i32, h: i32) -> IntRect {
        let margin_x = ((w - 640) / 2).clamp(0, w / 16);
        let margin_y = ((h - 480) / 2).clamp(0, h / 16);
        IntRect::new(margin_x, margin_y, w - 2 * margin_x, h - 2 * margin_y)
    }

    pub fn layout(w: i32, h: i32, gui: &ClonkFontSet, book: &BookFonts) -> BookLayout {
        let mut state = OptionsDlgState::new(ProgramSheetState::default());
        state.enable_voice_sheet(VoiceOptionsState::default());
        let layout = state.build_layout(w, h, gui, book);
        // Keep the native seven-tab sizing, in the browser's category order.
        let order = voice_sheet::VOICE_SHEETS;
        BookLayout {
            title_center: layout.title_center,
            back_button: layout.back_button,
            tabular: layout.tabular,
            paper: layout.paper,
            sheet: layout.sheet,
            tab_clips: order.map(|sheet| layout.tab_clips[sheet.index()]),
            tab_icons: order.map(|sheet| layout.tab_icons[sheet.index()]),
            tab_captions: order.map(|sheet| layout.tab_captions[sheet.index()]),
            focus_highlight: layout.focus_highlight,
            tab_height: layout.tab_height,
            tab_icon_size: layout.tab_icon_size,
        }
    }

    pub fn chrome(
        &self,
        surface: &mut Surface,
        layout: &BookLayout,
        tabs: &[(&str, usize); 7],
        active: usize,
        tab_focused: bool,
        back: ClassicButtonState,
        window: Option<IntRect>,
        gamma: Option<&GammaRamp>,
    ) {
        let (w, h) = (surface.width() as i32, surface.height() as i32);
        match window {
            None => draw_image_bilinear(
                surface,
                &GuiRect::new(0.0, 0.0, w as f32, h as f32),
                &self.assets.background,
                gamma,
            ),
            // Over another screen: that screen dimmed, and the options screen
            // in a framed window over it.
            Some(frame) => {
                draw_engine_box(surface, 0, 0, w - 1, h - 1, 0x80000000, gamma);
                draw_image_bilinear(surface, &gui_rect(frame), &self.assets.background, gamma);
                let (right, bottom) = (frame.x + frame.w - 1, frame.y + frame.h - 1);
                for inset in 0..2 {
                    draw_frame_dw(
                        surface,
                        frame.x + inset,
                        frame.y + inset,
                        right - inset,
                        bottom - inset,
                        WINDOW_FRAME_WOOD,
                        gamma,
                    );
                }
                draw_frame_dw(
                    surface,
                    frame.x + 2,
                    frame.y + 2,
                    right - 2,
                    bottom - 2,
                    WINDOW_FRAME_GILT,
                    gamma,
                );
            }
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
        layout: &BookLayout,
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

    /// A button inked onto the page: an ink outline around its label. The
    /// default answer (Enter) takes the darker ink and a light wash.
    pub fn ink_button(
        &self,
        surface: &mut Surface,
        rect: IntRect,
        label: &str,
        default: bool,
        gamma: Option<&GammaRamp>,
    ) {
        let (right, bottom) = (rect.x + rect.w - 1, rect.y + rect.h - 1);
        let ink = if default {
            draw_engine_box(surface, rect.x, rect.y, right, bottom, 0xa0ff_f8e8, gamma);
            PAGE_TAB_INK
        } else {
            PAGE_TAB_RULE_INK
        };
        for inset in 0..2 {
            draw_frame_dw(
                surface,
                rect.x + inset,
                rect.y + inset,
                right - inset,
                bottom - inset,
                ink,
                gamma,
            );
        }
        let font = &self.fonts.book_small;
        let width = font.measure(label, true).0;
        font.draw_with_gamma(
            surface,
            rect.x + (rect.w - width) / 2,
            rect.y + (rect.h - font.line_height) / 2,
            label,
            PAGE_TAB_INK_RGBA,
            TextAlign::Left,
            true,
            gamma,
        );
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

fn gui_rect(rect: IntRect) -> GuiRect {
    GuiRect::new(rect.x as f32, rect.y as f32, rect.w as f32, rect.h as f32)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_popup_window_is_centred_and_never_squeezes_the_book_below_its_minimum() {
        for (w, h) in [(800, 600), (1280, 720), (1920, 1080), (1001, 757)] {
            let window = OptionsBook::window(w, h);
            assert_eq!(window.x, w - (window.x + window.w), "{w}x{h} side margins");
            assert_eq!(
                window.y,
                h - (window.y + window.h),
                "{w}x{h} top and bottom"
            );
            assert!(
                window.x > 0 && window.y > 0,
                "{w}x{h} leaves the screen in view"
            );
            assert!(
                window.w >= 640 && window.h >= 480,
                "{w}x{h} keeps the minimum"
            );
        }
        assert_eq!(OptionsBook::window(640, 480), IntRect::new(0, 0, 640, 480));
    }
}
