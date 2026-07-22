//! Retained Keyboard/Gamepad page model for `C4StartupOptionsDlg`.

use crate::classic_gui::IntRect;
use crate::GuiPoint;

pub const CONTROL_SET_COUNT: usize = 4;
pub const CONTROL_KEY_COUNT: usize = 12;

pub const CONTROL_KEY_LABELS: [&str; CONTROL_KEY_COUNT] = [
    "Select left",
    "Select toggle",
    "Select right",
    "Throw",
    "Up / Jump",
    "Dig",
    "Left",
    "Down / Stop",
    "Right",
    "Player menu",
    "Special 1",
    "Special 2",
];

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ControlDevice {
    Keyboard,
    Gamepad,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ControlCaptureTarget {
    pub device: ControlDevice,
    pub set: usize,
    pub control: usize,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ControlSheetState {
    keyboard_labels: [[String; CONTROL_KEY_COUNT]; CONTROL_SET_COUNT],
    gamepad_labels: [[String; CONTROL_KEY_COUNT]; CONTROL_SET_COUNT],
    selected_keyboard_set: usize,
    selected_gamepad_set: usize,
    visible_gamepads: usize,
    gamepad_gui_control: bool,
}

impl Default for ControlSheetState {
    fn default() -> Self {
        Self::new(
            std::array::from_fn(|_| std::array::from_fn(|_| "Undefined".to_string())),
            std::array::from_fn(|_| std::array::from_fn(|_| "Undefined".to_string())),
            1,
            false,
        )
    }
}

impl ControlSheetState {
    pub fn new(
        keyboard_labels: [[String; CONTROL_KEY_COUNT]; CONTROL_SET_COUNT],
        gamepad_labels: [[String; CONTROL_KEY_COUNT]; CONTROL_SET_COUNT],
        connected_gamepads: usize,
        gamepad_gui_control: bool,
    ) -> Self {
        Self {
            keyboard_labels,
            gamepad_labels,
            selected_keyboard_set: 0,
            selected_gamepad_set: 0,
            visible_gamepads: connected_gamepads.clamp(1, CONTROL_SET_COUNT),
            gamepad_gui_control,
        }
    }

    pub const fn selected_set(&self, device: ControlDevice) -> usize {
        match device {
            ControlDevice::Keyboard => self.selected_keyboard_set,
            ControlDevice::Gamepad => self.selected_gamepad_set,
        }
    }

    pub const fn visible_sets(&self, device: ControlDevice) -> usize {
        match device {
            ControlDevice::Keyboard => CONTROL_SET_COUNT,
            ControlDevice::Gamepad => self.visible_gamepads,
        }
    }

    pub fn select_set(&mut self, device: ControlDevice, set: usize) -> bool {
        if set >= self.visible_sets(device) {
            return false;
        }
        let selected = match device {
            ControlDevice::Keyboard => &mut self.selected_keyboard_set,
            ControlDevice::Gamepad => &mut self.selected_gamepad_set,
        };
        let changed = *selected != set;
        *selected = set;
        changed
    }

    pub fn label(&self, target: ControlCaptureTarget) -> Option<&str> {
        if target.set >= CONTROL_SET_COUNT || target.control >= CONTROL_KEY_COUNT {
            return None;
        }
        Some(match target.device {
            ControlDevice::Keyboard => &self.keyboard_labels[target.set][target.control],
            ControlDevice::Gamepad => &self.gamepad_labels[target.set][target.control],
        })
    }

    pub fn visible_label(&self, device: ControlDevice, control: usize) -> Option<&str> {
        self.label(ControlCaptureTarget {
            device,
            set: self.selected_set(device),
            control,
        })
    }

    pub fn set_label(&mut self, target: ControlCaptureTarget, label: String) -> bool {
        if target.set >= CONTROL_SET_COUNT || target.control >= CONTROL_KEY_COUNT {
            return false;
        }
        let slot = match target.device {
            ControlDevice::Keyboard => &mut self.keyboard_labels[target.set][target.control],
            ControlDevice::Gamepad => &mut self.gamepad_labels[target.set][target.control],
        };
        *slot = label;
        true
    }

    pub const fn gamepad_gui_control(&self) -> bool {
        self.gamepad_gui_control
    }

    pub fn set_gamepad_gui_control(&mut self, enabled: bool) {
        self.gamepad_gui_control = enabled;
    }

    pub const fn gamepad_gui_checkbox_visible(&self) -> bool {
        self.selected_gamepad_set == 0
    }

    pub const fn capture_target(
        &self,
        device: ControlDevice,
        control: usize,
    ) -> Option<ControlCaptureTarget> {
        if control >= CONTROL_KEY_COUNT {
            return None;
        }
        Some(ControlCaptureTarget {
            device,
            set: self.selected_set(device),
            control,
        })
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ControlSheetLayout {
    pub set_buttons: [IntRect; CONTROL_SET_COUNT],
    pub key_buttons: [IntRect; CONTROL_KEY_COUNT],
    pub reset_button: IntRect,
    pub gamepad_gui_check: IntRect,
}

impl ControlSheetLayout {
    pub fn from_sheet(sheet: IntRect, line_height: i32) -> Self {
        let mx = (sheet.w / 20).max(8);
        let my = (sheet.h / 40).max(4);
        let inner = IntRect {
            x: sheet.x + mx,
            y: sheet.y + my,
            w: sheet.w - mx * 2,
            h: sheet.h - my * 2,
        };
        let set_gap = 10;
        let set_w = ((inner.w - set_gap * 3) / 4).max(20);
        let set_h = (line_height * 2).max(36);
        let set_buttons = std::array::from_fn(|index| IntRect {
            x: inner.x + index as i32 * (set_w + set_gap),
            y: inner.y,
            w: set_w,
            h: set_h,
        });

        let grid_top = inner.y + set_h + my + 4;
        let footer_h = (line_height * 2).max(32);
        let grid_h = (inner.y + inner.h - footer_h - my - grid_top).max(4);
        let col_gap = 12;
        let row_gap = 8;
        let cell_w = ((inner.w - col_gap * 2) / 3).max(20);
        let cell_h = ((grid_h - row_gap * 3) / 4).max(line_height + 4);
        let key_buttons = std::array::from_fn(|control| {
            let row = control / 3;
            let column = control % 3;
            IntRect {
                x: inner.x + column as i32 * (cell_w + col_gap),
                y: grid_top + row as i32 * (cell_h + row_gap),
                w: cell_w,
                h: cell_h,
            }
        });
        let footer_y = inner.y + inner.h - footer_h;
        let gamepad_gui_check = IntRect {
            x: inner.x,
            y: footer_y + (footer_h - line_height) / 2,
            w: inner.w / 2,
            h: line_height,
        };
        let reset_button = IntRect {
            x: inner.x + inner.w * 3 / 5,
            y: footer_y,
            w: inner.w * 2 / 5,
            h: footer_h,
        };
        Self {
            set_buttons,
            key_buttons,
            reset_button,
            gamepad_gui_check,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ControlSheetHit {
    Set(usize),
    Key(usize),
    Reset,
    GamepadGui,
}

pub fn control_sheet_hit_test(
    layout: &ControlSheetLayout,
    state: &ControlSheetState,
    device: ControlDevice,
    point: GuiPoint,
) -> Option<ControlSheetHit> {
    let contains = |rect: IntRect| {
        let x = point.x.floor() as i32;
        let y = point.y.floor() as i32;
        x >= rect.x && x < rect.x + rect.w && y >= rect.y && y < rect.y + rect.h
    };
    for set in 0..state.visible_sets(device) {
        if contains(layout.set_buttons[set]) {
            return Some(ControlSheetHit::Set(set));
        }
    }
    for control in 0..CONTROL_KEY_COUNT {
        if contains(layout.key_buttons[control]) {
            return Some(ControlSheetHit::Key(control));
        }
    }
    if contains(layout.reset_button) {
        return Some(ControlSheetHit::Reset);
    }
    if device == ControlDevice::Gamepad
        && state.gamepad_gui_checkbox_visible()
        && contains(layout.gamepad_gui_check)
    {
        return Some(ControlSheetHit::GamepadGui);
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    fn labels(prefix: &str) -> [[String; CONTROL_KEY_COUNT]; CONTROL_SET_COUNT] {
        std::array::from_fn(|set| {
            std::array::from_fn(|control| format!("{prefix}-{set}-{control}"))
        })
    }

    #[test]
    fn set_selection_and_capture_keep_keyboard_and_gamepad_independent() {
        let mut state = ControlSheetState::new(labels("K"), labels("G"), 2, true);
        assert!(state.select_set(ControlDevice::Keyboard, 3));
        assert!(state.select_set(ControlDevice::Gamepad, 1));
        assert!(!state.select_set(ControlDevice::Gamepad, 2));
        let keyboard = state.capture_target(ControlDevice::Keyboard, 11).unwrap();
        let gamepad = state.capture_target(ControlDevice::Gamepad, 11).unwrap();
        assert_eq!((keyboard.set, gamepad.set), (3, 1));
        assert_eq!(state.label(keyboard), Some("K-3-11"));
        assert_eq!(state.label(gamepad), Some("G-1-11"));
    }

    #[test]
    fn gamepad_gui_checkbox_exists_only_on_first_pad() {
        let mut state = ControlSheetState::default();
        assert!(state.gamepad_gui_checkbox_visible());
        assert!(
            state.select_set(ControlDevice::Gamepad, 0)
                || state.selected_set(ControlDevice::Gamepad) == 0
        );
        state.visible_gamepads = 4;
        assert!(state.select_set(ControlDevice::Gamepad, 2));
        assert!(!state.gamepad_gui_checkbox_visible());
    }

    #[test]
    fn layout_routes_all_twelve_key_buttons() {
        let state = ControlSheetState::default();
        let layout = ControlSheetLayout::from_sheet(
            IntRect {
                x: 100,
                y: 80,
                w: 600,
                h: 400,
            },
            20,
        );
        for control in 0..CONTROL_KEY_COUNT {
            let rect = layout.key_buttons[control];
            assert_eq!(
                control_sheet_hit_test(
                    &layout,
                    &state,
                    ControlDevice::Keyboard,
                    GuiPoint::new(rect.x as f32, rect.y as f32),
                ),
                Some(ControlSheetHit::Key(control))
            );
        }
    }
}
