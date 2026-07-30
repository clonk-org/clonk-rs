//! Retained Keyboard/Gamepad page model for `C4StartupOptionsDlg`.

use crate::classic_gui::IntRect;
use crate::startup_options_dlg::Aligner;
use crate::GuiPoint;

pub const CONTROL_SET_COUNT: usize = 4;
pub const CONTROL_KEY_COUNT: usize = 12;

/// `iKeyPosMaxX` / `iKeyPosMaxY` (`C4StartupOptionsDlg.cpp:207`). `iKeyPosis`
/// (`:208-214`) is the identity table, so key `n` sits at row `n / 3`,
/// column `n % 3`.
const KEY_COLUMNS: i32 = 3;
const KEY_ROWS: i32 = 4;

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

/// `C4GUI_ButtonHgt` (`C4Gui.h:119`) — the height of the bottom button strip.
const BUTTON_HEIGHT: i32 = 32;

/// The margins `ControlConfigArea` uses for the control-set selector row
/// (`C4StartupOptionsDlg.cpp:273`). The horizontal one is recomputed once the
/// button width is known.
const SET_MARGIN: i32 = 5;

/// `iKeyMargin` before the key area is scaled to fit
/// (`C4StartupOptionsDlg.cpp:298`).
const KEY_MARGIN: i32 = 20;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ControlSheetLayout {
    pub set_buttons: [IntRect; CONTROL_SET_COUNT],
    /// The `C4GUI::HorizontalLine` under the selector row
    /// (`C4StartupOptionsDlg.cpp:292`).
    pub separator: IntRect,
    /// The `KeySelButton` bounds — square `iKeyWdt` caps. The action and key
    /// labels are drawn beside them, outside these rects
    /// (`IsComponentOutsideClientArea`, `C4StartupOptionsDlg.h:184`).
    pub key_buttons: [IntRect; CONTROL_KEY_COUNT],
    pub reset_button: IntRect,
    pub gamepad_gui_check: IntRect,
}

impl ControlSheetLayout {
    /// `C4StartupOptionsDlg::ControlConfigArea::ControlConfigArea`
    /// (`C4StartupOptionsDlg.cpp:257-352`). `sheet` is the tabular sheet's
    /// client rect in screen coordinates; C++ passes it as the window bounds and
    /// then zeroes the aligner's origin, so the walk runs sheet-relative and the
    /// origin is added back at the end.
    ///
    /// `h_margin`/`v_margin` are the dialog's `caMain.GetWidth()/20` and
    /// `caMain.GetHeight()/40` (`:994`, `:997`). `sets` is `iMaxControlSets`,
    /// `reset_text` is `CaptionFont.GetTextExtent(IDS_BTN_RESETKEYBOARD)`, and
    /// `gamepad_check` carries `CheckBox::GetStandardCheckBoxSize` on the gamepad
    /// tab only — the keyboard tab has no checkbox (`:332`).
    pub fn from_sheet(
        sheet: IntRect,
        h_margin: i32,
        v_margin: i32,
        sets: usize,
        reset_text: (i32, i32),
        gamepad_check: Option<(i32, i32)>,
    ) -> Self {
        let sets = sets.clamp(1, CONTROL_SET_COUNT);
        let mut area = Aligner::new(
            IntRect {
                x: 0,
                y: 0,
                w: sheet.w,
                h: sheet.h,
            },
            h_margin,
            v_margin,
        );

        // Selector row (`:271-289`). The button width is clamped to the facet's
        // own width, and the slack that leaves is redistributed as the row's
        // horizontal margin rather than stretching the buttons.
        let set_row_width = area.width() - h_margin * 2;
        let set_button_width = ((set_row_width - sets as i32 * SET_MARGIN * 2) / sets as i32)
            .clamp(5, control_facets::KEYBOARD.w);
        let set_button_height =
            set_button_width * control_facets::KEYBOARD.h / control_facets::KEYBOARD.w;
        let set_h_margin = (set_row_width - set_button_width * sets as i32) / (sets as i32 * 2);
        let mut set_row = Aligner::new(
            area.get_from_top(2 * SET_MARGIN + set_button_height),
            set_h_margin,
            SET_MARGIN,
        );
        let mut set_buttons = [IntRect::default(); CONTROL_SET_COUNT];
        for button in set_buttons.iter_mut().take(sets) {
            *button = set_row.get_from_left(set_button_width, -1);
        }

        // The separator is bracketed by two `ExpandTop`s, so it costs the area
        // exactly its own two pixels (`:291-293`).
        area.expand_top(v_margin);
        let separator = area.get_from_top(2);
        area.expand_top(v_margin);

        // Key grid (`:294-327`). The natural size is three columns of
        // `iKeyUseWdt` and four rows of `iKeyHgt`, each with `iKeyMargin` on
        // both sides; when that overflows, every dimension is scaled by the
        // tighter of the two ratios and truncated.
        let max_width = area.width() - 2 * h_margin;
        let max_height = area.height() - 2 * v_margin;
        let mut key_margin = KEY_MARGIN;
        let mut key_width = control_facets::KEY.w * 3 / 2;
        let mut key_height = control_facets::KEY.h * 3 / 2;
        let mut key_use_width = key_width + key_height * 3;
        let mut grid_width = (key_use_width + 2 * key_margin) * KEY_COLUMNS;
        let mut grid_height = (key_height + 2 * key_margin) * KEY_ROWS;
        if grid_width > max_width || grid_height > max_height {
            let scale_x = max_width as f32 / grid_width.max(1) as f32;
            let scale_y = max_height as f32 / grid_height.max(1) as f32;
            let scale = if scale_x > scale_y { scale_y } else { scale_x };
            let apply = |value: i32| (scale * value as f32) as i32;
            key_margin = apply(key_margin);
            key_width = apply(key_width);
            key_use_width = apply(key_use_width);
            key_height = apply(key_height);
            grid_width = apply(grid_width);
            grid_height = apply(grid_height);
        }
        let mut grid = Aligner::new(
            area.get_from_top_centered(grid_height, grid_width),
            0,
            key_margin,
        );
        let mut key_buttons = [IntRect::default(); CONTROL_KEY_COUNT];
        for row in 0..KEY_ROWS {
            let mut line = Aligner::new(grid.get_from_top(key_height), key_margin, 0);
            for column in 0..KEY_COLUMNS {
                let rect = line.get_from_left(key_width, -1);
                // The remaining `iKeyUseWdt - iKeyWdt` of the cell is the label
                // space to the button's right (`:321`).
                line.expand_left(key_width - key_use_width);
                key_buttons[(row * KEY_COLUMNS + column) as usize] = rect;
            }
        }

        // Bottom strip (`:329-348`).
        area.expand_bottom(-(key_height / 2));
        let mut bottom = Aligner::new(area.get_from_bottom(BUTTON_HEIGHT), 2, 0);
        let gamepad_gui_check = gamepad_check
            .map(|(w, h)| bottom.get_from_left(w, h))
            .unwrap_or_default();
        let reset_width = (reset_text.0 + reset_text.1 * 4).min(bottom.inner_width());
        let reset_button = bottom.get_from_right(reset_width, -1);

        let on_sheet = |rect: IntRect| IntRect {
            x: rect.x + sheet.x,
            y: rect.y + sheet.y,
            ..rect
        };
        Self {
            set_buttons: set_buttons.map(on_sheet),
            separator: on_sheet(separator),
            key_buttons: key_buttons.map(on_sheet),
            reset_button: on_sheet(reset_button),
            gamepad_gui_check: gamepad_check
                .map(|_| on_sheet(gamepad_gui_check))
                .unwrap_or_default(),
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

        // A selector's phase steps one cell per control set, and the gamepad
        // facet is a separate image whose phase width is 80
        // (C4StartupOptionsDlg.cpp:271; C4GraphicsResource.cpp:229).
        let gamepad_cell = IntRect {
            x: 0,
            y: 0,
            w: control_facets::GAMEPAD_PHASE_WIDTH,
            h: 36,
        };
        assert_eq!(control_facets::phase_rect(gamepad_cell, 2).x, 160);
        assert_eq!(
            control_facets::phase_rect(control_facets::KEYBOARD, 2).x,
            160,
            "both selectors advance by their own phase width"
        );

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

    /// `ControlConfigArea`'s constructor is one `C4GUI::ComponentAligner` walk
    /// (`C4StartupOptionsDlg.cpp:257-352`) over the tabular sheet's client rect,
    /// with the margins the two call sites pass (`:994`, `:997`). At 1280x720
    /// that sheet is `(356, 108, 644, 462)` and the margins are
    /// `caMain.GetWidth()/20 = 61` and `caMain.GetHeight()/40 = 13`.
    #[test]
    fn control_layout_matches_the_cpp_component_aligner_at_1280x720() {
        let sheet = IntRect {
            x: 356,
            y: 108,
            w: 644,
            h: 462,
        };
        let layout = ControlSheetLayout::from_sheet(sheet, 61, 13, 4, (100, 20), None);

        // `iCtrlSetBtnWdt` clamps to `fctKeyboard.Wdt` (80) and the leftover is
        // redistributed as `iCtrlSetHMargin` (`:274-276`), so the selectors keep
        // the facet's own size rather than stretching over the sheet.
        let expected_sets = [442, 572, 702, 832];
        for (set, x) in expected_sets.into_iter().enumerate() {
            assert_eq!(
                layout.set_buttons[set],
                IntRect {
                    x,
                    y: 126,
                    w: 80,
                    h: 36
                },
                "selector {set}"
            );
        }

        // `caArea.ExpandTop(vM); GetFromTop(2); ExpandTop(vM)` (`:291-293`).
        assert_eq!(
            layout.separator,
            IntRect {
                x: 417,
                y: 180,
                w: 522,
                h: 2
            }
        );

        // The key grid is `(iKeyUseWdt + 2*iKeyMargin) * 3` by
        // `(iKeyHgt + 2*iKeyMargin) * 4` scaled down by `min(fScaleX, fScaleY)`
        // (`:294-312`); each button is the *square* `iKeyWdt`, and the label
        // space `iKeyUseWdt - iKeyWdt` sits outside it (`:320-321`).
        for control in 0..CONTROL_KEY_COUNT {
            let expected = IntRect {
                x: [425, 598, 771][control % 3],
                y: [203, 258, 313, 368][control / 3],
                w: 39,
                h: 39,
            };
            assert_eq!(layout.key_buttons[control], expected, "key {control}");
        }

        // `caArea.ExpandBottom(-iKeyHgt/2)`, then a `C4GUI_ButtonHgt` strip whose
        // right end holds `min(txtW + txtH*4, GetInnerWidth())` (`:329-347`).
        assert_eq!(
            layout.reset_button,
            IntRect {
                x: 757,
                y: 506,
                w: 180,
                h: 32
            }
        );
    }

    /// The gamepad tab differs only in its selector count and in the
    /// `IDS_CTL_GAMEPADFORMENU` checkbox that consumes the bottom strip from the
    /// left before the reset button is taken from the right (`:332-347`).
    #[test]
    fn gamepad_control_layout_centres_one_selector_and_seats_the_gui_checkbox() {
        let sheet = IntRect {
            x: 356,
            y: 108,
            w: 644,
            h: 462,
        };
        let layout = ControlSheetLayout::from_sheet(sheet, 61, 13, 1, (100, 20), Some((150, 20)));

        // `iCtrlSetHMargin = (522 - 80) / 2 = 221`, so the lone pad is centred.
        assert_eq!(
            layout.set_buttons[0],
            IntRect {
                x: 638,
                y: 126,
                w: 80,
                h: 36
            }
        );
        // `caKeyBottomBtns.GetFromLeft(iWdt, iHgt)` vertically centres the box.
        assert_eq!(
            layout.gamepad_gui_check,
            IntRect {
                x: 419,
                y: 512,
                w: 150,
                h: 20
            }
        );
        // `GetFromRight` only shrinks `Wdt`, never moves `x` (`C4Gui.cpp:1119`),
        // so the checkbox eats into the strip's *left* and the reset button
        // lands exactly where the keyboard tab puts it.
        assert_eq!(layout.reset_button.x, 757);
        assert_eq!(layout.reset_button.w, 180);
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
            30,
            10,
            4,
            (100, 20),
            None,
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
