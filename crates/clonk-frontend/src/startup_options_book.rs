//! The shared settings browser uses the original options book's artwork and
//! drawing primitives. The compatibility renderer continues to own its layout.
use super::*;
use crate::classic_gui::{draw_engine_box, ClassicButtonState};

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
