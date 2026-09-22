//! The port's voice page uses the same paper widgets as the other Options pages.
//! Audio devices and microphone ownership stay in the app; this module is UI only.
use super::*;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum VoiceOptionsControl {
    Enabled,
    Activation,
    PushToTalk,
    Volume,
    Input,
    Output,
    Retry,
    Echo,
    Noise,
    Gain,
    Test,
}

impl VoiceOptionsControl {
    pub const ALL: [Self; 11] = [
        Self::Enabled,
        Self::Activation,
        Self::PushToTalk,
        Self::Volume,
        Self::Input,
        Self::Output,
        Self::Retry,
        Self::Echo,
        Self::Noise,
        Self::Gain,
        Self::Test,
    ];
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum VoiceOptionsAction {
    Activate(VoiceOptionsControl),
    SetVolume(u8),
}

#[derive(Clone, Debug)]
pub struct VoiceOptionsLabels {
    pub volume: String,
    pub devices: String,
    pub input: String,
    pub output: String,
    pub retry: String,
    pub quality: String,
    pub echo: String,
    pub noise: String,
    pub gain: String,
    pub test: String,
    pub record: String,
    pub stop: String,
    pub privacy: String,
}

impl Default for VoiceOptionsLabels {
    fn default() -> Self {
        Self::localized(|_, fallback| fallback.to_owned())
    }
}

impl VoiceOptionsLabels {
    pub fn localized(text: impl Fn(&str, &str) -> String) -> Self {
        Self {
            volume: text("IDS_VOICE_VOLUME", "Volume"),
            devices: text("IDS_VOICE_DEVICES", "Audio devices"),
            input: text("IDS_VOICE_INPUT", "Microphone"),
            output: text("IDS_VOICE_OUTPUT", "Game & voice output"),
            retry: text("IDS_VOICE_REFRESH", "Refresh devices"),
            quality: text("IDS_VOICE_QUALITY", "Voice quality"),
            echo: text("IDS_VOICE_ECHO", "Echo cancellation"),
            noise: text("IDS_VOICE_NOISE", "Noise suppression"),
            gain: text("IDS_VOICE_GAIN", "Automatic mic level"),
            test: text("IDS_VOICE_TEST", "Microphone test"),
            record: text("IDS_VOICE_RECORD", "Record & listen"),
            stop: text("IDS_VOICE_STOP", "Stop test"),
            privacy: text(
                "IDS_VOICE_PRIVACY",
                "Record 3 seconds, then listen. Only you can hear this test.",
            ),
        }
    }
}

#[derive(Clone, Debug)]
pub struct VoiceOptionsState {
    pub labels: VoiceOptionsLabels,
    pub enabled: bool,
    pub activated: bool,
    pub key: String,
    pub volume: u8,
    pub input: String,
    pub output: String,
    pub echo: bool,
    pub noise: bool,
    pub gain: bool,
    pub testing: bool,
    pub status: String,
    pub device_status: String,
    pub level: f32,
}

impl Default for VoiceOptionsState {
    fn default() -> Self {
        Self {
            labels: VoiceOptionsLabels::default(),
            enabled: false,
            activated: false,
            key: "`".into(),
            volume: 100,
            input: "System default".into(),
            output: "System default".into(),
            echo: true,
            noise: true,
            gain: true,
            testing: false,
            status: "Microphone closed.".into(),
            device_status: String::new(),
            level: 0.0,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct VoiceSheetLayout {
    pub controls: [IntRect; 11],
    groups: [IntRect; 4],
    key_label: IntRect,
    volume_label: IntRect,
    input_label: IntRect,
    output_label: IntRect,
    device_status: IntRect,
    status: IntRect,
    privacy: IntRect,
    meter: IntRect,
    compact: bool,
}

impl VoiceSheetLayout {
    pub fn control(&self, control: VoiceOptionsControl) -> IntRect {
        self.controls[control as usize]
    }

    fn new(sheet: IntRect, book: &BookFonts) -> Self {
        let compact = sheet.h < 420 || sheet.w < 600;
        let font = if compact {
            &book.book_small
        } else {
            &book.book
        };
        let line = font.line_height;
        let gap = if compact { 6 } else { 12 };
        let pad = if compact { 8 } else { 16 };
        let row = line + if compact { 4 } else { 8 };
        let x = sheet.x + pad;
        let y = sheet.y + gap;
        let w = sheet.w - 2 * pad;
        let first_h = line + 2 * row + gap;
        let devices_h = line + 3 * row + gap;
        let first = IntRect::new(x, y, w, first_h);
        let devices = IntRect::new(x, y + first_h + gap, w, devices_h);
        let lower_y = devices.y + devices.h + gap;
        let lower_h = sheet.y + sheet.h - gap - lower_y;
        let half = (w - gap) / 2;
        let quality = IntRect::new(x, lower_y, half, lower_h);
        let test = IntRect::new(x + half + gap, lower_y, w - half - gap, lower_h);
        let left = x + pad;
        let right = x + w / 2 + gap;
        let col = w / 2 - pad - gap;
        let mut controls = [IntRect::new(0, 0, 0, 0); 11];
        use VoiceOptionsControl::*;
        controls[Enabled as usize] = IntRect::new(left, y + line + gap / 2, col, line);
        controls[Activation as usize] = IntRect::new(right, y + line + gap / 2, col, line);
        let second_y = y + line + row + gap / 2;
        let key_label = IntRect::new(left, second_y, col / 2, line);
        controls[PushToTalk as usize] =
            IntRect::new(left + col / 2, second_y, col - col / 2, line + 2);
        let volume_label = IntRect::new(right, second_y, col * 3 / 5, line);
        controls[Volume as usize] = IntRect::new(
            right + col * 3 / 5,
            second_y + (line - 16) / 2,
            col - col * 3 / 5,
            16,
        );
        let label_w = w * 2 / 5 - pad;
        let input_label = IntRect::new(left, devices.y + line + gap / 2, label_w, line);
        let output_label = IntRect::new(left, input_label.y + row, label_w, line);
        for (control, label) in [(Input, input_label), (Output, output_label)] {
            controls[control as usize] =
                IntRect::new(left + label_w, label.y, w - 2 * pad - label_w, line + 2);
        }
        let refresh_w = (w / 3).max(130);
        controls[Retry as usize] = IntRect::new(
            x + w - pad - refresh_w,
            output_label.y + row,
            refresh_w,
            line + 2,
        );
        let device_status = IntRect::new(
            left,
            output_label.y + row,
            w - 2 * pad - refresh_w - gap,
            line,
        );
        for (index, control) in [Echo, Noise, Gain].into_iter().enumerate() {
            controls[control as usize] = IntRect::new(
                quality.x + pad,
                lower_y + line + gap / 2 + row * index as i32,
                half - 2 * pad,
                line,
            );
        }
        controls[Test as usize] = IntRect::new(
            test.x + pad,
            lower_y + line + gap / 2,
            test.w - 2 * pad,
            line + 2,
        );
        let meter = IntRect::new(
            test.x + pad,
            controls[Test as usize].y + row + 2,
            test.w - 2 * pad,
            8,
        );
        let status_y = meter.y + meter.h + 4;
        let status_h = (test.y + test.h - pad - status_y - 2 - line * 2).clamp(line, line * 2);
        let status = IntRect::new(test.x + pad, status_y, test.w - 2 * pad, status_h);
        let privacy = IntRect::new(
            test.x + pad,
            status.y + status.h + 2,
            test.w - 2 * pad,
            test.y + test.h - pad - status.y - status.h - 2,
        );
        Self {
            controls,
            groups: [first, devices, quality, test],
            key_label,
            volume_label,
            input_label,
            output_label,
            device_status,
            status,
            privacy,
            meter,
            compact,
        }
    }
}

impl OptionsDlgState {
    pub fn enable_voice_sheet(&mut self, voice: VoiceOptionsState) {
        self.voice = Some(voice);
    }

    pub fn voice(&self) -> Option<&VoiceOptionsState> {
        self.voice.as_ref()
    }

    pub fn set_voice_state(&mut self, voice: VoiceOptionsState) {
        self.voice = Some(voice);
    }

    pub fn voice_control_bounds(&self, control: VoiceOptionsControl) -> Option<IntRect> {
        Some(self.layout.as_ref()?.voice.as_ref()?.control(control))
    }

    pub(super) fn voice_tooltip(&self, point: GuiPoint) -> Option<StartupTooltip> {
        let layout = self.layout.as_ref()?.voice.as_ref()?;
        let state = self.voice.as_ref()?;
        for (rect, text) in [
            (layout.control(VoiceOptionsControl::Input), &state.input),
            (layout.control(VoiceOptionsControl::Output), &state.output),
            (layout.status, &state.status),
            (layout.device_status, &state.device_status),
            (layout.privacy, &state.labels.privacy),
            (
                layout.control(VoiceOptionsControl::Echo),
                &state.labels.echo,
            ),
            (
                layout.control(VoiceOptionsControl::Noise),
                &state.labels.noise,
            ),
            (
                layout.control(VoiceOptionsControl::Gain),
                &state.labels.gain,
            ),
        ] {
            if rect_contains(&rect, point) {
                return Some(StartupTooltip::text(text));
            }
        }
        None
    }

    pub(super) fn visible_sheets(&self) -> &[OptionsSheet] {
        if self.voice.is_some() {
            &VOICE_SHEETS
        } else {
            &OptionsSheet::ALL
        }
    }

    pub(super) fn adjacent_sheet(&self, delta: isize) -> OptionsSheet {
        let sheets = self.visible_sheets();
        let index = sheets
            .iter()
            .position(|s| *s == self.active_sheet)
            .unwrap_or(0);
        sheets[(index as isize + delta).rem_euclid(sheets.len() as isize) as usize]
    }

    pub(super) fn build_layout(
        &self,
        w: i32,
        h: i32,
        gui: &ClonkFontSet,
        book: &BookFonts,
    ) -> OptionsDlgLayout {
        let mut layout = options_dlg_layout_for(
            w,
            h,
            gui,
            book,
            &self.labels,
            self.controls.visible_sets(ControlDevice::Gamepad),
        );
        if self.voice.is_some() {
            layout.sound.voice = None;
            layout.voice = Some(VoiceSheetLayout::new(layout.sheet, book));
            let pitch = ((layout.tabular.h - 20) / 7).min(72);
            layout.tab_height = pitch + 8;
            layout.tab_icon_size = (pitch - book.book_small.line_height - 4).clamp(16, 32);
            for (position, sheet) in VOICE_SHEETS.iter().enumerate() {
                let i = sheet.index();
                let x = layout.tab_clips[0].0;
                let y = layout.tabular.y + 8 + position as i32 * pitch;
                layout.tab_clips[i] = (x, y);
                let icon = layout.tab_icon_size;
                let top = y + (layout.tab_height - icon - 2 - book.book_small.line_height) / 2;
                layout.tab_icons[i] = (x + 95 / 2 - icon / 2, top);
                layout.tab_captions[i] = (x + 95 / 2, top + icon + 2);
            }
            layout.focus_highlight.h = layout.tab_height - 6;
            layout.focus_highlight.y = layout.tab_clips[0].1 + 3;
        }
        layout
    }

    pub(super) fn activate_voice_control(
        &mut self,
        control: VoiceOptionsControl,
    ) -> Vec<OptionsDlgAction> {
        self.focus = OptionsFocus::Voice(control);
        vec![OptionsDlgAction::Voice(VoiceOptionsAction::Activate(
            control,
        ))]
    }

    pub(super) fn voice_volume(&mut self, value: i32) -> Vec<OptionsDlgAction> {
        let value = value.clamp(0, 200) as u8;
        if let Some(voice) = self.voice.as_mut() {
            if voice.volume == value {
                return Vec::new();
            }
            voice.volume = value;
            return vec![OptionsDlgAction::Voice(VoiceOptionsAction::SetVolume(
                value,
            ))];
        }
        Vec::new()
    }

    pub(super) fn drag_voice_volume(&mut self, point: GuiPoint) -> Vec<OptionsDlgAction> {
        let Some(rect) = self.voice_control_bounds(VoiceOptionsControl::Volume) else {
            return Vec::new();
        };
        self.voice_volume((point.x as i32 - rect.x - 24) * 200 / (rect.w - 48).max(1))
    }
}

pub(super) const VOICE_SHEETS: [OptionsSheet; 7] = [
    OptionsSheet::Program,
    OptionsSheet::Graphics,
    OptionsSheet::Sound,
    OptionsSheet::Voice,
    OptionsSheet::Keyboard,
    OptionsSheet::Gamepad,
    OptionsSheet::Network,
];

/// Ellipsize by glyph width, never by bytes. Device names are untrusted plain text.
fn fit_text(font: &ClonkFont, text: &str, width: i32) -> String {
    if font.measure(text, false).0 <= width {
        return text.to_owned();
    }
    let mut result = String::new();
    for ch in text.chars() {
        let mut next = result.clone();
        next.push(ch);
        if font.measure(&format!("{next}..."), false).0 > width {
            break;
        }
        result.push(ch);
    }
    result.push_str("...");
    result
}

impl OptionsDlgScreen {
    pub(super) fn draw_voice_sheet(
        surface: &mut Surface,
        assets: &OptionsDlgAssets,
        book: &BookFonts,
        layout: &VoiceSheetLayout,
        state: &OptionsDlgState,
        gamma: Option<&GammaRamp>,
        draw_focus: bool,
    ) {
        let Some(voice) = state.voice.as_ref() else {
            return;
        };
        let compact_fonts;
        let book = if layout.compact {
            compact_fonts = BookFonts {
                book: book.book_small.clone(),
                book_small: book.book_small.clone(),
                book_caption: book.book_caption.clone(),
                book_title: book.book_title.clone(),
            };
            &compact_fonts
        } else {
            book
        };
        let labels = &voice.labels;
        let highlighted = |control| {
            draw_focus
                && (state.focus == OptionsFocus::Voice(control)
                    || state.hovered == Some(OptionsHit::Voice(control)))
        };
        for (group, title) in layout.groups.iter().zip([
            &state.labels.voice_chat,
            &labels.devices,
            &labels.quality,
            &labels.test,
        ]) {
            Self::draw_group_box(surface, book, group, title, gamma);
        }
        use VoiceOptionsControl::*;
        for (control, label, checked) in [
            (Enabled, &state.labels.voice_enabled, voice.enabled),
            (Activation, &state.labels.voice_activated, voice.activated),
            (Echo, &labels.echo, voice.echo),
            (Noise, &labels.noise, voice.noise),
            (Gain, &labels.gain, voice.gain),
        ] {
            let rect = layout.control(control);
            let text = fit_text(&book.book, label, rect.w - rect.h - 4);
            Self::draw_checkbox(
                surface,
                assets,
                book,
                &rect,
                &text,
                checked,
                highlighted(control),
                gamma,
            );
        }
        let mut text = |rect: IntRect, value: &str| {
            book.book.draw_with_gamma(
                surface,
                rect.x,
                rect.y,
                &fit_text(&book.book, value, rect.w - 4),
                STARTUP_FONT_RGBA,
                TextAlign::Left,
                false,
                gamma,
            );
        };
        text(
            layout.key_label,
            &format!("{}:", state.labels.voice_push_to_talk),
        );
        text(
            layout.volume_label,
            &format!(
                "{}: {}%",
                if layout.compact {
                    &labels.volume
                } else {
                    &state.labels.voice_volume
                },
                voice.volume
            ),
        );
        text(layout.input_label, &format!("{}:", labels.input));
        text(layout.output_label, &format!("{}:", labels.output));
        text(layout.device_status, &voice.device_status);
        for (control, value) in [(Input, &voice.input), (Output, &voice.output)] {
            let rect = layout.control(control);
            Self::draw_combo(
                surface,
                assets,
                book,
                &rect,
                "",
                highlighted(control),
                gamma,
            );
            book.book.draw_with_gamma(
                surface,
                rect.x + 18,
                rect.y + (rect.h - book.book.line_height) / 2,
                &fit_text(&book.book, value, rect.w - 42),
                STARTUP_FONT_RGBA,
                TextAlign::Left,
                false,
                gamma,
            );
        }
        for (control, value) in [
            (PushToTalk, &voice.key),
            (Retry, &labels.retry),
            (
                Test,
                if voice.testing {
                    &labels.stop
                } else {
                    &labels.record
                },
            ),
        ] {
            let rect = layout.control(control);
            Self::draw_small_button(
                surface,
                assets,
                book,
                &rect,
                &fit_text(&book.book, value, rect.w - 12),
                highlighted(control),
                state.pressed_release_target == Some(OptionsHit::Voice(control)),
                gamma,
            );
        }
        let slider = layout.control(Volume);
        Self::draw_book_scrollbar(
            surface,
            assets,
            &slider,
            i32::from(voice.volume) * (slider.w - 48).max(1) / 200,
            false,
            false,
            gamma,
        );
        if draw_focus && state.focus == OptionsFocus::Voice(Volume) {
            draw_frame_dw(
                surface,
                slider.x,
                slider.y - 2,
                slider.x + slider.w,
                slider.y + slider.h + 2,
                EDIT_BORDER_COLOR,
                gamma,
            );
        }
        let meter = layout.meter;
        draw_frame_dw(
            surface,
            meter.x,
            meter.y,
            meter.x + meter.w,
            meter.y + meter.h,
            EDIT_BORDER_COLOR,
            gamma,
        );
        let level = if voice.level.is_finite() {
            voice.level.clamp(0.0, 1.0)
        } else {
            0.0
        };
        let right = meter.x + 2 + ((meter.w - 4) as f32 * level) as i32;
        fill_quad_dw(
            surface,
            &[
                (meter.x + 2, meter.y + 2),
                (right, meter.y + 2),
                (right, meter.y + meter.h - 1),
                (meter.x + 2, meter.y + meter.h - 1),
            ],
            0x004f702d,
            gamma,
        );
        draw_wrapped(
            surface,
            &book.book_small,
            layout.status,
            &voice.status,
            gamma,
        );
        draw_wrapped(
            surface,
            &book.book_small,
            layout.privacy,
            &labels.privacy,
            gamma,
        );
    }
}

fn draw_wrapped(
    surface: &mut Surface,
    font: &ClonkFont,
    rect: IntRect,
    text: &str,
    gamma: Option<&GammaRamp>,
) {
    let max_lines = (rect.h / font.line_height).max(0) as usize;
    let mut lines: Vec<String> = Vec::new();
    let mut line = String::new();
    for word in text.split_whitespace() {
        let next = if line.is_empty() {
            word.to_owned()
        } else {
            format!("{line} {word}")
        };
        if !line.is_empty() && font.measure(&next, false).0 > rect.w {
            lines.push(std::mem::take(&mut line));
        }
        if !line.is_empty() {
            line.push(' ');
        }
        line.push_str(word);
    }
    if !line.is_empty() {
        lines.push(line);
    }
    if lines.len() > max_lines && max_lines > 0 {
        lines[max_lines - 1] = lines[max_lines - 1..].join(" ");
    }
    for (index, line) in lines.iter().take(max_lines).enumerate() {
        font.draw_with_gamma(
            surface,
            rect.x,
            rect.y + index as i32 * font.line_height,
            &fit_text(font, line, rect.w),
            STARTUP_FONT_RGBA,
            TextAlign::Left,
            false,
            gamma,
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_support::{endeavour_font_set, repo_root};

    #[test]
    fn voice_volume_responds_to_gamepad_vertical_keys_without_leaving_options() {
        let mut state = OptionsDlgState::default();
        state.enable_voice_sheet(VoiceOptionsState::default());
        state.restore_sheet(OptionsSheet::Voice);
        for _ in 0..4 {
            state.handle_gamepad_horizontal(false);
        }
        assert_eq!(
            state.handle_key_down(KeyCode::Up),
            [OptionsDlgAction::Voice(VoiceOptionsAction::SetVolume(105))]
        );
        assert_eq!(
            state.handle_key_down(KeyCode::Down),
            [OptionsDlgAction::Voice(VoiceOptionsAction::SetVolume(100))]
        );
        assert_eq!(
            state.handle_key_down(KeyCode::Left),
            [OptionsDlgAction::Voice(VoiceOptionsAction::SetVolume(95))]
        );
        state.handle_gamepad_horizontal(false);
        assert_eq!(state.focus, OptionsFocus::Voice(VoiceOptionsControl::Input));
        state.handle_gamepad_low_down();
        assert_eq!(
            state.handle_gamepad_low_up(),
            [OptionsDlgAction::Voice(VoiceOptionsAction::Activate(
                VoiceOptionsControl::Input
            ))]
        );
    }

    #[test]
    fn voice_page_keeps_controls_and_test_instructions_inside_the_paper() {
        let gui = endeavour_font_set();
        let book = build_book_fonts(
            &std::fs::read(repo_root().join("planet/System.c4g/Endeavour.ttf")).unwrap(),
        )
        .unwrap();
        for (w, h) in [(640, 480), (800, 600), (1280, 720), (1920, 1080)] {
            let mut state = OptionsDlgState::default();
            state.enable_voice_sheet(VoiceOptionsState::default());
            state.resize(w, h, &gui, &book);
            let layout = state.layout.as_ref().unwrap();
            let voice = layout.voice.as_ref().unwrap();
            let contains = |outer: IntRect, inner: IntRect| {
                inner.x >= outer.x
                    && inner.y >= outer.y
                    && inner.x + inner.w <= outer.x + outer.w
                    && inner.y + inner.h <= outer.y + outer.h
            };
            for (index, rect) in voice.controls.iter().enumerate() {
                assert!(
                    rect.w >= 48 && rect.h >= 16 && contains(layout.sheet, *rect),
                    "{w}x{h} control {index}: {rect:?}"
                );
            }
            assert!(
                voice.privacy.h >= 2 * book.book_small.line_height,
                "{w}x{h}: privacy hint must stay readable: {:?}",
                voice.privacy
            );
            assert!(contains(layout.sheet, voice.privacy));
            for sheet in VOICE_SHEETS {
                let (x, y) = layout.tab_clips[sheet.index()];
                let point = GuiPoint::new((x + 40) as f32, (y + layout.tab_height / 2) as f32);
                assert_eq!(
                    options_hit_test(
                        layout,
                        OptionsSheet::Voice,
                        &state.controls,
                        &state.network,
                        point
                    ),
                    Some(OptionsHit::Tab(sheet))
                );
                assert!(y + layout.tab_height <= layout.tabular.y + layout.tabular.h);
            }
            assert!(
                layout.sound.voice.is_none(),
                "Audio must not duplicate the voice page"
            );
        }
    }
}
