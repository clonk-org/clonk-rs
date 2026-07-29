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

/// The `IDS_CTL_*` keys `C4StartupOptionsDlg` resolves for the twelve action
/// labels, in its own order (`C4StartupOptionsDlg.cpp:166-169`). The built-in
/// [`CONTROL_KEY_LABELS`] are the shipped US text and remain the fallback for a
/// key the active language table does not carry.
pub const CONTROL_KEY_LABEL_KEYS: [&str; CONTROL_KEY_COUNT] = [
    "IDS_CTL_SELECTLEFT",
    "IDS_CTL_SELECTTOGGLE",
    "IDS_CTL_SELECTRIGHT",
    "IDS_CTL_THROW",
    "IDS_CTL_UPJUMP",
    "IDS_CTL_DIG",
    "IDS_CTL_LEFT",
    "IDS_CTL_DOWNSTOP",
    "IDS_CTL_RIGHT",
    "IDS_CTL_PLAYERMENU",
    "IDS_CTL_SPECIAL1",
    "IDS_CTL_SPECIAL2",
];

/// The action label for `control`, resolved through the active language table
/// and falling back to the shipped US text.
pub fn control_key_label(control: usize, resources: &dyn Fn(&str) -> Option<String>) -> String {
    CONTROL_KEY_LABEL_KEYS
        .get(control)
        .and_then(|key| resources(key))
        .filter(|label| !label.is_empty())
        .or_else(|| {
            CONTROL_KEY_LABELS
                .get(control)
                .map(|label| (*label).to_owned())
        })
        .unwrap_or_default()
}

/// The four control facets and where they live in their source images
/// (`C4GraphicsResource.cpp:200-203,229`). `fctKeyboard`, `fctCommand` and
/// `fctKey` are sub-rects of one `Control.png`; `fctGamepad` is its own
/// `Gamepad.png` loaded with an 80px phase width.
pub mod control_facets {
    use crate::classic_gui::IntRect;

    /// `fctKeyboard.Set(&sfcControl, 0, 0, 80, 36)` — one phase per control set.
    pub const KEYBOARD: IntRect = IntRect {
        x: 0,
        y: 0,
        w: 80,
        h: 36,
    };
    /// `fctCommand.Set(&sfcControl, 0, 36, 32, 32)` — one phase per command.
    pub const COMMAND: IntRect = IntRect {
        x: 0,
        y: 36,
        w: 32,
        h: 32,
    };
    /// `fctKey.Set(&sfcControl, 0, 100, 64, 64)` — phase 0 idle, 1 pressed.
    pub const KEY: IntRect = IntRect {
        x: 0,
        y: 100,
        w: 64,
        h: 64,
    };
    /// `LoadFile(fctGamepad, "Gamepad", Files, 80)` — phase width, own image.
    pub const GAMEPAD_PHASE_WIDTH: i32 = 80;

    /// The source rect of `phase` within a facet whose cells run left to right.
    pub fn phase_rect(cell: IntRect, phase: usize) -> IntRect {
        IntRect {
            x: cell.x + cell.w * phase as i32,
            ..cell
        }
    }
}

/// Which facet cell a device selector shows. `C4StartupOptionsDlg` draws
/// `fctKeyboard`/`fctGamepad` phases rather than a text button
/// (`C4StartupOptionsDlg.cpp:215-345`).
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct DeviceSelectorFacet {
    /// `fctKeyboard` for a keyboard set, `fctGamepad` for a gamepad set.
    pub device: ControlDevice,
    /// Zero-based control-set index, which is the facet phase.
    pub phase: usize,
}

/// The facets a key binding button composes, in draw order
/// (`C4StartupOptionsDlg::KeySelButton::DrawElement`).
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct KeyButtonFacets {
    /// `fctKey.Draw(cgoDraw, true, fDown)` — the key cap, phase 1 while held.
    pub key_phase: usize,
    /// Where `fctCommand` is drawn, inset from the cap.
    pub command_rect: IntRect,
    /// `fctCommand.Draw(cgoDraw, true, iKeyID, 0)` — the command glyph's phase.
    pub command_phase: usize,
}

/// `KeySelButton::DrawElement`'s inset: the command glyph sits a fifth of the
/// button's width inside the key cap horizontally, three quarters of that
/// above, and a held button nudges it down by half an indent so the glyph
/// follows the cap.
pub fn key_button_facets(bounds: IntRect, key_id: usize, down: bool) -> KeyButtonFacets {
    let indent = bounds.w / 5;
    let mut command = IntRect {
        x: bounds.x + indent,
        y: bounds.y + indent * 3 / 4,
        w: bounds.w - 2 * indent,
        h: bounds.h - 2 * indent,
    };
    if down {
        command.y += indent / 2;
    }
    KeyButtonFacets {
        key_phase: usize::from(down),
        command_rect: command,
        command_phase: key_id,
    }
}

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

    // C4StartupOptionsDlg.cpp:215-345 — the device selectors are facet phases,
    // the key buttons compose fctKey + an inset fctCommand, and the action
    // labels come from the IDS_CTL_* table rather than baked English.
    #[test]
    fn startup_options_control_sheets_render_classic_facets_and_resource_text() {
        // A selector names its device facet and takes the set index as phase.
        for set in 0..CONTROL_SET_COUNT {
            let keyboard = DeviceSelectorFacet {
                device: ControlDevice::Keyboard,
                phase: set,
            };
            assert_eq!(keyboard.phase, set);
            assert_eq!(keyboard.device, ControlDevice::Keyboard);
        }

        // KeySelButton::DrawElement's inset: a fifth of the width either side,
        // three quarters of that above, two indents off the height.
        let bounds = IntRect {
            x: 100,
            y: 40,
            w: 40,
            h: 40,
        };
        let released = key_button_facets(bounds, 7, false);
        assert_eq!(released.key_phase, 0, "an idle cap uses phase 0");
        assert_eq!(released.command_phase, 7, "the command phase is the key id");
        let indent = 40 / 5;
        assert_eq!(
            released.command_rect,
            IntRect {
                x: 100 + indent,
                y: 40 + indent * 3 / 4,
                w: 40 - 2 * indent,
                h: 40 - 2 * indent,
            }
        );

        // A held cap switches phase and nudges the glyph down half an indent,
        // so it follows the pressed key.
        let held = key_button_facets(bounds, 7, true);
        assert_eq!(held.key_phase, 1);
        assert_eq!(held.command_rect.y, released.command_rect.y + indent / 2);
        assert_eq!(held.command_rect.x, released.command_rect.x);
        assert_eq!(held.command_rect.w, released.command_rect.w);

        // A button too narrow to indent degrades without inverting the rect.
        let narrow = key_button_facets(
            IntRect {
                w: 3,
                h: 3,
                ..bounds
            },
            0,
            false,
        );
        assert_eq!(narrow.command_rect.w, 3);
        assert_eq!(narrow.command_rect.h, 3);

        // Labels resolve through the language table in C++'s key order.
        let table = |key: &str| match key {
            "IDS_CTL_SELECTLEFT" => Some("Links wählen".to_owned()),
            "IDS_CTL_THROW" => Some(String::new()),
            _ => None,
        };
        assert_eq!(control_key_label(0, &table), "Links wählen");
        // An empty or missing entry falls back to the shipped US text.
        assert_eq!(control_key_label(3, &table), "Throw");
        assert_eq!(control_key_label(5, &table), "Dig");
        assert_eq!(CONTROL_KEY_LABEL_KEYS.len(), CONTROL_KEY_LABELS.len());
        assert_eq!(CONTROL_KEY_LABEL_KEYS[11], "IDS_CTL_SPECIAL2");

        // The facet source rects come from one Control.png, except the gamepad
        // selector which is its own image (C4GraphicsResource.cpp:200-203,229).
        use control_facets::{phase_rect, COMMAND, GAMEPAD_PHASE_WIDTH, KEY, KEYBOARD};
        assert_eq!(
            (KEYBOARD.x, KEYBOARD.y, KEYBOARD.w, KEYBOARD.h),
            (0, 0, 80, 36)
        );
        assert_eq!(
            (COMMAND.x, COMMAND.y, COMMAND.w, COMMAND.h),
            (0, 36, 32, 32)
        );
        assert_eq!((KEY.x, KEY.y, KEY.w, KEY.h), (0, 100, 64, 64));
        assert_eq!(GAMEPAD_PHASE_WIDTH, 80);
        // Phases run left to right from the cell's own origin.
        assert_eq!(phase_rect(KEY, 0), KEY);
        assert_eq!(phase_rect(KEY, 1).x, KEY.x + KEY.w);
        assert_eq!(phase_rect(COMMAND, 3).x, COMMAND.x + 3 * COMMAND.w);
        assert_eq!(phase_rect(COMMAND, 3).y, COMMAND.y);
    }

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
