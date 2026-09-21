//! Port-only voice setup; no capture, device I/O or networking lives in this UI.
use crate::classic_gui::IntRect;
use clonk_gui::Point;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum VoiceSetupControl {
    Input,
    Output,
    Enabled,
    Activation,
    PushToTalk,
    VolumeDown,
    VolumeUp,
    Echo,
    Noise,
    Gain,
    Test,
    Retry,
    Close,
}

pub const CONTROLS: [VoiceSetupControl; 13] = [
    VoiceSetupControl::Input,
    VoiceSetupControl::Output,
    VoiceSetupControl::Enabled,
    VoiceSetupControl::Activation,
    VoiceSetupControl::PushToTalk,
    VoiceSetupControl::VolumeDown,
    VoiceSetupControl::VolumeUp,
    VoiceSetupControl::Echo,
    VoiceSetupControl::Noise,
    VoiceSetupControl::Gain,
    VoiceSetupControl::Test,
    VoiceSetupControl::Retry,
    VoiceSetupControl::Close,
];

pub fn panel(width: i32, height: i32) -> IntRect {
    IntRect::new((width - 600) / 2, (height - 420) / 2, 600, 420)
}

pub fn control_rect(panel: IntRect, control: VoiceSetupControl) -> IntRect {
    use VoiceSetupControl::*;
    let (x, y, w) = match control {
        Input => (12, 40, 576),
        Output => (12, 76, 576),
        Enabled => (12, 112, 282),
        Activation => (306, 112, 282),
        PushToTalk => (12, 148, 282),
        VolumeDown => (306, 148, 135),
        VolumeUp => (453, 148, 135),
        Echo => (12, 184, 184),
        Noise => (208, 184, 184),
        Gain => (404, 184, 184),
        Test => (12, 220, 380),
        Retry => (404, 220, 184),
        Close => (404, 380, 184),
    };
    IntRect::new(panel.x + x, panel.y + y, w, 28)
}

#[derive(Default)]
pub struct VoiceSetupController {
    focus: usize,
    pressed: Option<VoiceSetupControl>,
}

impl VoiceSetupController {
    pub fn key(
        &mut self,
        key: clonk_gui::KeyCode,
        down: bool,
        shift: bool,
    ) -> Option<VoiceSetupControl> {
        use clonk_gui::KeyCode;
        match key {
            KeyCode::Escape if down => {
                self.cancel_interaction();
                return Some(VoiceSetupControl::Close);
            }
            KeyCode::Tab | KeyCode::Up | KeyCode::Down if down => {
                self.cancel_interaction();
                let backwards = key == KeyCode::Up || (key == KeyCode::Tab && shift);
                self.focus =
                    (self.focus + if backwards { CONTROLS.len() - 1 } else { 1 }) % CONTROLS.len();
            }
            KeyCode::Enter | KeyCode::Space => {
                if down {
                    self.pressed = Some(CONTROLS[self.focus]);
                } else {
                    return self
                        .pressed
                        .take()
                        .filter(|pressed| *pressed == CONTROLS[self.focus]);
                }
            }
            _ => {}
        }
        None
    }

    pub fn cancel_interaction(&mut self) {
        self.pressed = None;
    }
    pub fn pointer(
        &mut self,
        panel: IntRect,
        point: Point,
        down: bool,
    ) -> Option<VoiceSetupControl> {
        let target = CONTROLS.into_iter().find(|&control| {
            let r = control_rect(panel, control);
            point.x >= r.x as f32
                && point.y >= r.y as f32
                && point.x < (r.x + r.w) as f32
                && point.y < (r.y + r.h) as f32
        });
        if down {
            self.pressed = target;
            if let Some(target) = target {
                self.focus = CONTROLS
                    .iter()
                    .position(|control| *control == target)
                    .unwrap_or(0);
            }
            None
        } else {
            self.pressed
                .take()
                .filter(|pressed| Some(*pressed) == target)
        }
    }
}

/// Owned presentation snapshot; callers refresh metadata without native I/O.
pub struct VoiceSetupView {
    pub labels: [String; 13],
    pub status: [String; 4],
    pub level: f32,
}

impl VoiceSetupController {
    pub fn render(
        &self,
        surface: &mut clonk_graphics::Surface,
        resources: crate::message_dialog::MessageDialogResources<'_>,
        view: &VoiceSetupView,
        gamma: Option<&clonk_graphics::GammaRamp>,
    ) {
        use crate::classic_gui::{
            draw_clipped_text_with_markup, draw_engine_box, ClassicButtonState,
        };
        use clonk_graphics::clonk_font::TextAlign;
        let bounds = panel(surface.width() as i32, surface.height() as i32);
        resources.skin.draw_dialog(surface, bounds, gamma);
        resources.skin.draw_caption(
            surface,
            IntRect::new(bounds.x, bounds.y, bounds.w, 30),
            "Voice setup",
            &resources.fonts.caption,
            [255, 255, 160, 255],
            TextAlign::Center,
            gamma,
        );
        for (index, &control) in CONTROLS.iter().enumerate() {
            resources.skin.draw_button(
                surface,
                control_rect(bounds, control),
                &view.labels[index],
                resources.fonts,
                ClassicButtonState {
                    pressed: self.pressed == Some(control),
                    highlighted: self.focus == index,
                },
                gamma,
            );
        }
        for (index, text) in view.status.iter().enumerate() {
            let y = bounds.y + 262 + index as i32 * 20;
            draw_clipped_text_with_markup(
                surface,
                &resources.fonts.main_small,
                bounds.x + 14,
                y,
                text,
                [255; 4],
                TextAlign::Left,
                gamma,
                IntRect::new(bounds.x + 12, y, bounds.w - 24, 20),
                false,
            );
        }
        let x = bounds.x + 12;
        let y = bounds.y + 346;
        draw_engine_box(surface, x, y, x + bounds.w - 25, y + 11, 0x00222222, gamma);
        let level = if view.level.is_finite() {
            view.level.clamp(0.0, 1.0)
        } else {
            0.0
        };
        let width = ((bounds.w - 24) as f32 * level) as i32;
        if width > 0 {
            draw_engine_box(surface, x, y, x + width - 1, y + 11, 0x0044dd66, gamma);
        }
        draw_clipped_text_with_markup(
            surface,
            &resources.fonts.main_small,
            x,
            bounds.y + 363,
            "Record 3 seconds, then listen. Audio stays on this computer.",
            [255; 4],
            TextAlign::Left,
            gamma,
            IntRect::new(x, bounds.y + 363, bounds.w - 24, 17),
            false,
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn voice_setup_controls_fit_small_window_without_overlapping() {
        let bounds = panel(640, 480);
        for (index, &control) in CONTROLS.iter().enumerate() {
            let r = control_rect(bounds, control);
            assert!(r.x >= 0 && r.y >= 0 && r.x + r.w <= 640 && r.y + r.h <= 480);
            assert!(r.w >= 44 && r.h >= 28);
            for &other in &CONTROLS[index + 1..] {
                let s = control_rect(bounds, other);
                assert!(
                    r.x + r.w <= s.x || s.x + s.w <= r.x || r.y + r.h <= s.y || s.y + s.h <= r.y,
                    "{control:?} overlaps {other:?}"
                );
            }
        }
    }

    #[test]
    fn keyboard_can_reach_every_control_and_only_activate_on_release() {
        use clonk_gui::KeyCode;
        let mut controller = VoiceSetupController::default();
        for control in CONTROLS {
            assert_eq!(controller.key(KeyCode::Enter, true, false), None);
            assert_eq!(controller.key(KeyCode::Enter, false, false), Some(control));
            controller.key(KeyCode::Tab, true, false);
        }
        assert_eq!(controller.focus, 0);
        controller.key(KeyCode::Tab, true, true);
        assert_eq!(controller.focus, CONTROLS.len() - 1);
    }

    #[test]
    fn cancelling_test_button_press_never_starts_recording_on_release() {
        let bounds = panel(640, 480);
        let r = control_rect(bounds, VoiceSetupControl::Test);
        let point = Point::new((r.x + 5) as f32, (r.y + 5) as f32);
        let mut controller = VoiceSetupController::default();
        assert_eq!(controller.pointer(bounds, point, true), None);
        controller.cancel_interaction();
        assert_eq!(controller.pointer(bounds, point, false), None);
    }
}
