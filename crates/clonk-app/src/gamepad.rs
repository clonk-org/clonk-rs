use std::collections::HashMap;

use clonk_engine::ControlButton;
use gilrs::ev::Code;
use gilrs::{Axis, Button, Event, EventType, GamepadId, Gilrs, GilrsBuilder};
use winit::event::ElementState;

use crate::input::{GamepadAxisCalibration, GamepadAxisCalibrations};

/// Normalized equivalent of the strict +/-13337 comparison in
/// `C4GamePadControl::FeedEvent`; gilrs exposes device axes in `[-1, 1]`.
const LEGACY_AXIS_DEAD_ZONE: f32 = 13_337.0 / i16::MAX as f32;
const LEGACY_AXIS_COUNT: usize = 8;
const LEGACY_HAT_X_AXIS: u8 = 6;
const LEGACY_HAT_Y_AXIS: u8 = 7;
const GAMEPAD_SLOT_COUNT: u8 = 4;
const WINDOWS_CALIBRATED_AXIS_COUNT: u8 = 6;
const WINDOWS_AXIS_RAW_MAX: f32 = u16::MAX as f32;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub(crate) struct GamepadSlot(u8);

impl GamepadSlot {
    pub(crate) const fn new(index: u8) -> Self {
        Self(index)
    }

    pub(crate) const fn from_index(index: usize) -> Option<Self> {
        if index < GAMEPAD_SLOT_COUNT as usize {
            Some(Self(index as u8))
        } else {
            None
        }
    }

    fn from_gamepad_id(id: GamepadId) -> Option<Self> {
        u8::try_from(usize::from(id))
            .ok()
            .filter(|index| *index < GAMEPAD_SLOT_COUNT)
            .map(Self)
    }

    pub(crate) const fn control_set(self) -> i32 {
        4 + self.0 as i32
    }

    pub(crate) const fn index(self) -> u8 {
        self.0
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub(crate) struct LegacyGamepadButton(u8);

impl LegacyGamepadButton {
    pub(crate) const fn new(index: u8) -> Self {
        Self(index)
    }

    pub(crate) const fn index(self) -> u8 {
        self.0
    }
}

/// Raw C++ `KEY_JOY_Axis` identity. `high == false` is the minimum extent
/// and `high == true` is the maximum extent.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub(crate) struct LegacyGamepadAxis {
    index: u8,
    high: bool,
}

impl LegacyGamepadAxis {
    pub(crate) const fn new(index: u8, high: bool) -> Self {
        Self { index, high }
    }

    pub(crate) const fn index(self) -> u8 {
        self.index
    }

    pub(crate) const fn high(self) -> bool {
        self.high
    }

    pub(crate) const fn direction(self) -> ControlButton {
        match (self.index.is_multiple_of(2), self.high) {
            (true, false) => ControlButton::Left,
            (true, true) => ControlButton::Right,
            (false, false) => ControlButton::Up,
            (false, true) => ControlButton::Down,
        }
    }
}

pub(crate) struct GamepadManager {
    gilrs: Option<Gilrs>,
    states: HashMap<GamepadSlot, GamepadState>,
    axis_calibrations: GamepadAxisCalibrations,
    axis_calibration_dirty: bool,
    use_windows_axis_calibration: bool,
    /// Logical equivalent of the Options `C4GamePadOpener`. Gilrs owns its
    /// platform handles as one context and cannot physically close one pad,
    /// so this claim controls which device is live for the Options consumer.
    options_open_slot: Option<GamepadSlot>,
    next_cluster: u64,
}

impl GamepadManager {
    pub(crate) fn new(axis_calibrations: GamepadAxisCalibrations) -> Self {
        // C++ owns the only gamepad dead zone. Gilrs' defaults would first
        // apply a device-specific radial dead zone, jitter suppression, and
        // axis-to-D-pad conversion, changing both the threshold and raw hat
        // identity before `FeedEvent` parity can be reproduced here.
        let gilrs = match GilrsBuilder::new().with_default_filters(false).build() {
            Ok(instance) => Some(instance),
            Err(error) => {
                tracing::warn!(error = %error, "failed to initialise gamepad input");
                None
            }
        };
        Self {
            gilrs,
            states: HashMap::new(),
            axis_calibrations,
            axis_calibration_dirty: false,
            use_windows_axis_calibration: cfg!(windows),
            options_open_slot: None,
            next_cluster: 0,
        }
    }

    pub(crate) fn disabled() -> Self {
        Self {
            gilrs: None,
            states: HashMap::new(),
            axis_calibrations: [[GamepadAxisCalibration::default();
                WINDOWS_CALIBRATED_AXIS_COUNT as usize];
                GAMEPAD_SLOT_COUNT as usize],
            axis_calibration_dirty: false,
            use_windows_axis_calibration: false,
            options_open_slot: None,
            next_cluster: 0,
        }
    }

    #[cfg(test)]
    fn disabled_with_windows_axis_calibration(axis_calibrations: GamepadAxisCalibrations) -> Self {
        Self {
            axis_calibrations,
            use_windows_axis_calibration: true,
            ..Self::disabled()
        }
    }

    pub(crate) fn set_axis_calibrations(&mut self, axis_calibrations: GamepadAxisCalibrations) {
        self.axis_calibrations = axis_calibrations;
        self.axis_calibration_dirty = false;
    }

    pub(crate) fn take_axis_calibration_update(&mut self) -> Option<GamepadAxisCalibrations> {
        if !self.axis_calibration_dirty {
            return None;
        }
        self.axis_calibration_dirty = false;
        Some(self.axis_calibrations)
    }

    /// Close the old Options claim before opening the replacement, matching
    /// `C4GamePadOpener::SetGamePad`. Re-selecting the same slot is a no-op.
    pub(crate) fn set_options_open_slot(&mut self, slot: Option<GamepadSlot>) -> bool {
        if self.options_open_slot == slot {
            return false;
        }
        let _ = self.options_open_slot.take();
        self.options_open_slot = slot;
        true
    }

    pub(crate) const fn options_open_slot(&self) -> Option<GamepadSlot> {
        self.options_open_slot
    }

    #[cfg(test)]
    pub(crate) fn is_options_slot_live(&self, slot: GamepadSlot) -> bool {
        self.options_open_slot == Some(slot)
    }

    pub(crate) fn poll(&mut self) -> Vec<SourcedGamepadEvent> {
        let mut output = Vec::new();
        while let Some(event) = self.gilrs.as_mut().and_then(|gilrs| gilrs.next_event()) {
            let gamepad = usize::from(event.id);
            let mut emitted = Vec::new();
            self.process_event(event, &mut emitted);
            append_sourced_events(gamepad, emitted, &mut self.next_cluster, &mut output);
        }
        output
    }

    pub(crate) fn connected_count(&self) -> usize {
        self.states.len()
    }

    fn process_event(&mut self, event: Event, output: &mut Vec<GamepadEvent>) {
        let Some(slot) = GamepadSlot::from_gamepad_id(event.id) else {
            return;
        };
        match event.event {
            EventType::Connected => {
                self.states.entry(slot).or_default();
            }
            EventType::Disconnected => {
                self.states.remove(&slot);
                emit_disconnect_clear(slot, output);
            }
            EventType::ButtonPressed(button, code) => {
                self.handle_gilrs_button_for_slot(
                    slot,
                    button,
                    code,
                    ElementState::Pressed,
                    output,
                );
            }
            EventType::ButtonReleased(button, code) => {
                self.handle_gilrs_button_for_slot(
                    slot,
                    button,
                    code,
                    ElementState::Released,
                    output,
                );
            }
            EventType::AxisChanged(axis, value, _) => {
                self.handle_axis_for_slot(slot, axis, value, output);
            }
            _ => {}
        }
    }

    fn handle_button_for_slot(
        &mut self,
        slot: GamepadSlot,
        button: Button,
        state: ElementState,
        output: &mut Vec<GamepadEvent>,
    ) {
        self.handle_button_for_slot_with_legacy_index(
            slot,
            button,
            legacy_button_from_semantic(button),
            state,
            output,
        );
    }

    fn handle_gilrs_button_for_slot(
        &mut self,
        slot: GamepadSlot,
        button: Button,
        code: Code,
        state: ElementState,
        output: &mut Vec<GamepadEvent>,
    ) {
        self.handle_button_for_slot_with_legacy_index(
            slot,
            button,
            legacy_button_from_gilrs(button, code),
            state,
            output,
        );
    }

    fn handle_button_for_slot_with_legacy_index(
        &mut self,
        slot: GamepadSlot,
        button: Button,
        legacy_button: Option<LegacyGamepadButton>,
        state: ElementState,
        output: &mut Vec<GamepadEvent>,
    ) {
        if let Some(class) = gui_button_class(button) {
            output.push(GamepadEvent::GuiButton { slot, class, state });
        }
        let pressed = state == ElementState::Pressed;
        match button {
            Button::DPadLeft | Button::DPadRight | Button::DPadUp | Button::DPadDown => {
                self.handle_dpad_button_for_slot(slot, button, pressed, output);
            }
            Button::South => {
                output.push(GamepadEvent::Action {
                    slot,
                    action: GamepadActionType::Select,
                    state,
                });
            }
            Button::East => {
                output.push(GamepadEvent::Action {
                    slot,
                    action: GamepadActionType::Cancel,
                    state,
                });
            }
            Button::Select => {
                output.push(GamepadEvent::Action {
                    slot,
                    action: GamepadActionType::MenuToggle,
                    state,
                });
            }
            _ => {}
        }
        if let Some(button) = legacy_button {
            output.push(GamepadEvent::Button {
                slot,
                button,
                state,
            });
        }
    }

    fn handle_dpad_button_for_slot(
        &mut self,
        slot: GamepadSlot,
        button: Button,
        pressed: bool,
        output: &mut Vec<GamepadEvent>,
    ) {
        let (axis, value) = {
            let pad_state = self.states.entry(slot).or_default();
            match button {
                Button::DPadLeft => pad_state.hat.left = pressed,
                Button::DPadRight => pad_state.hat.right = pressed,
                Button::DPadUp => pad_state.hat.up = pressed,
                Button::DPadDown => pad_state.hat.down = pressed,
                _ => return,
            }
            match button {
                Button::DPadLeft | Button::DPadRight => (
                    LEGACY_HAT_X_AXIS,
                    f32::from(pad_state.hat.right as i8 - pad_state.hat.left as i8),
                ),
                Button::DPadUp | Button::DPadDown => (
                    LEGACY_HAT_Y_AXIS,
                    f32::from(pad_state.hat.down as i8 - pad_state.hat.up as i8),
                ),
                _ => unreachable!(),
            }
        };
        self.handle_legacy_axis_for_slot(slot, axis, value, output);
    }

    fn handle_axis_for_slot(
        &mut self,
        slot: GamepadSlot,
        axis: Axis,
        value: f32,
        output: &mut Vec<GamepadEvent>,
    ) {
        // Gilrs normalizes Y axes so positive points up. SDL joystick axes,
        // which the C++ keycodes describe, use negative for up.
        let Some((legacy_axis, value)) = (match axis {
            Axis::LeftStickX => Some((0, value)),
            Axis::LeftStickY => Some((1, -value)),
            Axis::RightStickX => Some((2, value)),
            Axis::RightStickY => Some((3, -value)),
            Axis::LeftZ => Some((4, value)),
            Axis::RightZ => Some((5, value)),
            Axis::DPadX => Some((LEGACY_HAT_X_AXIS, value)),
            Axis::DPadY => Some((LEGACY_HAT_Y_AXIS, -value)),
            Axis::Unknown => None,
        }) else {
            return;
        };
        self.handle_legacy_axis_for_slot(slot, legacy_axis, value, output);
    }

    fn handle_legacy_axis_for_slot(
        &mut self,
        slot: GamepadSlot,
        index: u8,
        value: f32,
        output: &mut Vec<GamepadEvent>,
    ) {
        if usize::from(index) >= LEGACY_AXIS_COUNT {
            return;
        }
        let (desired_min, desired_max) =
            if self.use_windows_axis_calibration && index < WINDOWS_CALIBRATED_AXIS_COUNT {
                let raw = normalized_windows_axis_value(value);
                let calibration =
                    &mut self.axis_calibrations[usize::from(slot.index())][usize::from(index)];
                let before = *calibration;
                let position = calibrated_axis_position(calibration, raw);
                self.axis_calibration_dirty |= *calibration != before;
                (
                    position == LegacyAxisPosition::Low,
                    position == LegacyAxisPosition::High,
                )
            } else {
                (
                    value < -LEGACY_AXIS_DEAD_ZONE,
                    value > LEGACY_AXIS_DEAD_ZONE,
                )
            };
        let pad_state = self.states.entry(slot).or_default();
        for (high, desired) in [(false, desired_min), (true, desired_max)] {
            let axis = LegacyGamepadAxis::new(index, high);
            let direction = axis.direction();
            let direction_was_active = pad_state.direction_active(direction);
            let active = if high {
                &mut pad_state.axes[usize::from(index)].max_active
            } else {
                &mut pad_state.axes[usize::from(index)].min_active
            };
            if *active == desired {
                continue;
            }
            *active = desired;
            let state = if desired {
                ElementState::Pressed
            } else {
                ElementState::Released
            };
            output.push(GamepadEvent::Axis { slot, axis, state });
            if direction_was_active != pad_state.direction_active(direction) {
                output.push(GamepadEvent::Direction {
                    slot,
                    button: direction,
                    state,
                });
            }
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum LegacyAxisPosition {
    Low,
    Mid,
    High,
}

fn normalized_windows_axis_value(value: f32) -> u32 {
    // Gilrs does not expose the WinMM JOYINFOEX values persisted by C++. Use
    // the conventional unsigned 16-bit joystick range as the stable bridge
    // from its normalized Windows values to the legacy calibration domain.
    let value = if value.is_finite() {
        value.clamp(-1.0, 1.0)
    } else {
        0.0
    };
    (((value + 1.0) * 0.5) * WINDOWS_AXIS_RAW_MAX).round() as u32
}

fn calibrated_axis_position(
    calibration: &mut GamepadAxisCalibration,
    position: u32,
) -> LegacyAxisPosition {
    if !calibration.calibrated {
        calibration.min = position;
        calibration.max = position;
        calibration.calibrated = true;
        return LegacyAxisPosition::Mid;
    }

    calibration.min = calibration.min.min(position);
    calibration.max = calibration.max.max(position);
    // Preserve C++ uint32_t arithmetic, including wraparound for handwritten
    // out-of-range calibration extrema.
    let center = calibration.min.wrapping_add(calibration.max) / 2;
    let range = calibration.max.wrapping_sub(center) / 3;
    if position < center.wrapping_sub(range) {
        LegacyAxisPosition::Low
    } else if position > center.wrapping_add(range) {
        LegacyAxisPosition::High
    } else {
        LegacyAxisPosition::Mid
    }
}

fn append_sourced_events(
    gamepad: usize,
    events: impl IntoIterator<Item = GamepadEvent>,
    next_cluster: &mut u64,
    output: &mut Vec<SourcedGamepadEvent>,
) {
    let mut physical_cluster = None;
    let mut previous_was_axis = false;
    for event in events {
        let starts_physical_cluster = matches!(
            event,
            GamepadEvent::GuiButton { .. } | GamepadEvent::Axis { .. }
        );
        let direction = matches!(event, GamepadEvent::Direction { .. });
        let cluster = match (starts_physical_cluster, direction, physical_cluster) {
            (false, false, Some(cluster)) => cluster,
            (false, true, Some(cluster)) if previous_was_axis => cluster,
            _ => {
                let cluster = *next_cluster;
                *next_cluster = (*next_cluster).wrapping_add(1);
                physical_cluster = starts_physical_cluster.then_some(cluster);
                cluster
            }
        };
        previous_was_axis = matches!(event, GamepadEvent::Axis { .. });
        output.push(SourcedGamepadEvent {
            gamepad,
            cluster,
            event,
        });
    }
}

/// Translate gilrs' backend event identity into the zero-based raw button
/// index encoded by C++ `KEY_JOY_Button`. C++ receives SDL joystick indices;
/// gilrs deliberately exposes a normalized semantic `Button` and a
/// platform-specific `Code`, not that SDL index (C4GamePadCon.cpp:424-432;
/// C4KeyboardInput.h:64-80).
///
/// macOS HID button usages retain the physical one-based button number, so
/// use that when available. Other gilrs backends do not expose enough device
/// capability ordering to reconstruct SDL's raw ordinal; the semantic table
/// is an explicit, bounded compatibility translation rather than bit-exact
/// SDL parity. Exact cross-backend config capture remains an SDL-boundary gap.
fn legacy_button_from_gilrs(button: Button, code: Code) -> Option<LegacyGamepadButton> {
    #[cfg(target_os = "macos")]
    {
        const HID_BUTTON_PAGE: u32 = 0x09;
        let raw = code.into_u32();
        let page = raw >> 16;
        let usage = raw & 0xffff;
        if page == HID_BUTTON_PAGE && (1..=32).contains(&usage) {
            return Some(LegacyGamepadButton::new((usage - 1) as u8));
        }
    }
    #[cfg(not(target_os = "macos"))]
    let _ = code;

    legacy_button_from_semantic(button)
}

/// Closest portable fallback to the raw HID/SDL button ordering used by the
/// legacy config. D-pad values are intentionally excluded: C++ turns SDL hats
/// into axis keys rather than button keys (C4GamePadCon.cpp:339-361).
fn legacy_button_from_semantic(button: Button) -> Option<LegacyGamepadButton> {
    let index = match button {
        Button::South => 0,
        Button::East => 1,
        Button::North => 2,
        Button::West => 3,
        Button::LeftTrigger => 4,
        Button::RightTrigger => 5,
        Button::LeftTrigger2 => 6,
        Button::RightTrigger2 => 7,
        Button::Start => 8,
        Button::Select => 9,
        Button::Mode => 10,
        Button::C => 15,
        Button::Z => 16,
        Button::LeftThumb => 17,
        Button::RightThumb => 18,
        Button::DPadUp
        | Button::DPadDown
        | Button::DPadLeft
        | Button::DPadRight
        | Button::Unknown => return None,
    };
    Some(LegacyGamepadButton::new(index))
}

fn gui_button_class(button: Button) -> Option<GuiButtonClass> {
    match button {
        Button::South | Button::East | Button::North | Button::West => Some(GuiButtonClass::Low),
        Button::C
        | Button::Z
        | Button::LeftTrigger
        | Button::LeftTrigger2
        | Button::RightTrigger
        | Button::RightTrigger2
        | Button::Select
        | Button::Start
        | Button::Mode
        | Button::LeftThumb
        | Button::RightThumb
        | Button::Unknown => Some(GuiButtonClass::High),
        Button::DPadUp | Button::DPadDown | Button::DPadLeft | Button::DPadRight => None,
    }
}

/// A disconnected controller cannot deliver releases for controls that were
/// held at removal time. Clear aggregate input so gameplay and modal captures
/// cannot remain latched.
fn emit_disconnect_clear(slot: GamepadSlot, output: &mut Vec<GamepadEvent>) {
    output.push(GamepadEvent::Clear { slot });
}

#[derive(Default)]
struct GamepadState {
    axes: [LegacyAxisState; LEGACY_AXIS_COUNT],
    hat: HatButtonState,
}

impl GamepadState {
    fn direction_active(&self, direction: ControlButton) -> bool {
        self.axes.iter().enumerate().any(|(index, axis)| {
            let even = index % 2 == 0;
            match direction {
                ControlButton::Left => even && axis.min_active,
                ControlButton::Right => even && axis.max_active,
                ControlButton::Up => !even && axis.min_active,
                ControlButton::Down => !even && axis.max_active,
            }
        })
    }
}

#[derive(Default)]
struct LegacyAxisState {
    min_active: bool,
    max_active: bool,
}

#[derive(Default)]
struct HatButtonState {
    left: bool,
    right: bool,
    up: bool,
    down: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum GamepadActionType {
    Select,
    Cancel,
    MenuToggle,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum GuiButtonClass {
    Low,
    High,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum GamepadEvent {
    Axis {
        slot: GamepadSlot,
        axis: LegacyGamepadAxis,
        state: ElementState,
    },
    Direction {
        slot: GamepadSlot,
        button: ControlButton,
        state: ElementState,
    },
    Button {
        slot: GamepadSlot,
        button: LegacyGamepadButton,
        state: ElementState,
    },
    Clear {
        slot: GamepadSlot,
    },
    GuiButton {
        slot: GamepadSlot,
        class: GuiButtonClass,
        state: ElementState,
    },
    Action {
        slot: GamepadSlot,
        action: GamepadActionType,
        state: ElementState,
    },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct SourcedGamepadEvent {
    pub(crate) gamepad: usize,
    pub(crate) cluster: u64,
    pub(crate) event: GamepadEvent,
}

impl GamepadEvent {
    pub(crate) const fn slot(self) -> GamepadSlot {
        match self {
            Self::Axis { slot, .. }
            | Self::Direction { slot, .. }
            | Self::Button { slot, .. }
            | Self::Clear { slot }
            | Self::GuiButton { slot, .. }
            | Self::Action { slot, .. } => slot,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sourced_clusters_keep_gui_aliases_together_and_split_directions_and_clear() {
        let mut next_cluster = 7;
        let mut output = Vec::new();
        append_sourced_events(
            3,
            [
                GamepadEvent::GuiButton {
                    slot: GamepadSlot::new(3),
                    class: GuiButtonClass::High,
                    state: ElementState::Pressed,
                },
                GamepadEvent::Action {
                    slot: GamepadSlot::new(3),
                    action: GamepadActionType::MenuToggle,
                    state: ElementState::Pressed,
                },
                GamepadEvent::Clear {
                    slot: GamepadSlot::new(3),
                },
            ],
            &mut next_cluster,
            &mut output,
        );
        assert_eq!(
            output.iter().map(|event| event.gamepad).collect::<Vec<_>>(),
            [3, 3, 3]
        );
        assert_eq!(
            output.iter().map(|event| event.cluster).collect::<Vec<_>>(),
            [7, 7, 7]
        );

        append_sourced_events(
            3,
            [
                GamepadEvent::Clear {
                    slot: GamepadSlot::new(3),
                },
                GamepadEvent::Direction {
                    slot: GamepadSlot::new(3),
                    button: ControlButton::Left,
                    state: ElementState::Released,
                },
                GamepadEvent::Direction {
                    slot: GamepadSlot::new(3),
                    button: ControlButton::Right,
                    state: ElementState::Pressed,
                },
            ],
            &mut next_cluster,
            &mut output,
        );
        assert_eq!(
            output.iter().map(|event| event.cluster).collect::<Vec<_>>(),
            [7, 7, 7, 8, 9, 10],
            "disconnect Clear and every axis Direction receive fresh dispatch clusters"
        );
    }

    #[test]
    fn l026_axis_uses_cpp_strict_dead_zone_without_hysteresis() {
        let slot = GamepadSlot::new(0);
        let mut manager = GamepadManager::disabled();
        let mut output = Vec::new();

        manager.handle_axis_for_slot(slot, Axis::LeftStickX, -LEGACY_AXIS_DEAD_ZONE, &mut output);
        assert!(output.is_empty(), "the C++ comparison is strict");

        manager.handle_axis_for_slot(
            slot,
            Axis::LeftStickX,
            -(LEGACY_AXIS_DEAD_ZONE + 0.001),
            &mut output,
        );
        assert_eq!(
            output,
            [
                GamepadEvent::Axis {
                    slot,
                    axis: LegacyGamepadAxis::new(0, false),
                    state: ElementState::Pressed,
                },
                GamepadEvent::Direction {
                    slot,
                    button: ControlButton::Left,
                    state: ElementState::Pressed,
                },
            ]
        );

        output.clear();
        manager.handle_axis_for_slot(slot, Axis::LeftStickX, -LEGACY_AXIS_DEAD_ZONE, &mut output);
        assert_eq!(
            output,
            [
                GamepadEvent::Axis {
                    slot,
                    axis: LegacyGamepadAxis::new(0, false),
                    state: ElementState::Released,
                },
                GamepadEvent::Direction {
                    slot,
                    button: ControlButton::Left,
                    state: ElementState::Released,
                },
            ],
            "there is no separate release threshold"
        );
    }

    #[test]
    fn l034_cpp_axis_calibration_uses_strict_integer_boundaries_and_first_sample() {
        let mut calibration = GamepadAxisCalibration::new(0, 600, true);
        assert_eq!(
            calibrated_axis_position(&mut calibration, 199),
            LegacyAxisPosition::Low
        );
        assert_eq!(
            calibrated_axis_position(&mut calibration, 200),
            LegacyAxisPosition::Mid
        );
        assert_eq!(
            calibrated_axis_position(&mut calibration, 400),
            LegacyAxisPosition::Mid
        );
        assert_eq!(
            calibrated_axis_position(&mut calibration, 401),
            LegacyAxisPosition::High
        );

        let mut uncalibrated = GamepadAxisCalibration::default();
        assert_eq!(
            calibrated_axis_position(&mut uncalibrated, 123),
            LegacyAxisPosition::Mid
        );
        assert_eq!(uncalibrated, GamepadAxisCalibration::new(123, 123, true));

        let mut wrapping = GamepadAxisCalibration::new(3_000_000_000, 4_000_000_000, true);
        assert_eq!(
            calibrated_axis_position(&mut wrapping, 3_500_000_000),
            LegacyAxisPosition::High,
            "the C++ uint32_t center calculation wraps on overflow"
        );
    }

    #[test]
    fn l034_loaded_calibration_changes_windows_axis_threshold_but_not_hat_threshold() {
        use clonk_core::std_config::Config;

        use crate::input::GamepadBindings;

        let mut config = Config::new();
        config.set_in(Some("Gamepad0"), "Axis0Min", "0");
        config.set_in(Some("Gamepad0"), "Axis0Max", "65535");
        config.set_in(Some("Gamepad0"), "Axis0Calibrated", "1");
        let bindings = GamepadBindings::from_config(&config);
        let slot = GamepadSlot::new(0);
        let mut manager =
            GamepadManager::disabled_with_windows_axis_calibration(bindings.axis_calibrations());
        let mut output = Vec::new();

        manager.handle_axis_for_slot(slot, Axis::LeftStickX, 0.34, &mut output);
        assert_eq!(
            output,
            [
                GamepadEvent::Axis {
                    slot,
                    axis: LegacyGamepadAxis::new(0, true),
                    state: ElementState::Pressed,
                },
                GamepadEvent::Direction {
                    slot,
                    button: ControlButton::Right,
                    state: ElementState::Pressed,
                },
            ],
            "loaded 0..65535 extrema trigger above one-third while the SDL dead zone would not"
        );
        assert!(manager.take_axis_calibration_update().is_none());

        output.clear();
        manager.handle_axis_for_slot(slot, Axis::DPadX, 0.34, &mut output);
        assert!(output.is_empty(), "POV/hat axes retain the fixed threshold");
    }

    #[test]
    fn l034_runtime_extrema_copy_back_to_cpp_config_keys() {
        use clonk_core::std_config::Config;

        use crate::input::GamepadBindings;

        let mut config = Config::new();
        config.set_in(Some("Gamepad2"), "Axis4Min", "10000");
        config.set_in(Some("Gamepad2"), "Axis4Max", "50000");
        config.set_in(Some("Gamepad2"), "Axis4Calibrated", "true");
        let mut bindings = GamepadBindings::from_config(&config);
        let slot = GamepadSlot::new(2);
        let mut manager =
            GamepadManager::disabled_with_windows_axis_calibration(bindings.axis_calibrations());
        let mut output = Vec::new();

        manager.handle_axis_for_slot(slot, Axis::LeftZ, -1.0, &mut output);
        let calibrations = manager
            .take_axis_calibration_update()
            .expect("new minimum marks calibration dirty");
        bindings.replace_axis_calibrations(calibrations);
        bindings.write_to_config(&mut config);

        assert_eq!(config.get_in(Some("Gamepad2"), "Axis4Min"), Some("0"));
        assert_eq!(config.get_in(Some("Gamepad2"), "Axis4Max"), Some("50000"));
        assert_eq!(
            config.get_in(Some("Gamepad2"), "Axis4Calibrated"),
            Some("true")
        );
    }

    #[test]
    fn l026_gilrs_axes_map_to_cpp_axis_zero_through_hat_zero() {
        let slot = GamepadSlot::new(0);
        let mut manager = GamepadManager::disabled();
        let mut output = Vec::new();
        for (axis, value) in [
            (Axis::LeftStickX, -1.0),
            (Axis::LeftStickY, 1.0),
            (Axis::RightStickX, -1.0),
            (Axis::RightStickY, 1.0),
            (Axis::LeftZ, -1.0),
            (Axis::RightZ, -1.0),
            (Axis::DPadX, -1.0),
            (Axis::DPadY, 1.0),
        ] {
            manager.handle_axis_for_slot(slot, axis, value, &mut output);
        }
        assert_eq!(
            output
                .iter()
                .filter_map(|event| match event {
                    GamepadEvent::Axis { axis, .. } => Some((axis.index(), axis.high())),
                    _ => None,
                })
                .collect::<Vec<_>>(),
            [
                (0, false),
                (1, false),
                (2, false),
                (3, false),
                (4, false),
                (5, false),
                (6, false),
                (7, false),
            ]
        );
    }

    #[test]
    fn l026_dpad_buttons_share_hat_axis_state_with_raw_dpad_axes() {
        let slot = GamepadSlot::new(0);
        let mut manager = GamepadManager::disabled();
        let mut output = Vec::new();

        manager.handle_button_for_slot(slot, Button::DPadLeft, ElementState::Pressed, &mut output);
        assert_eq!(
            output,
            [
                GamepadEvent::Axis {
                    slot,
                    axis: LegacyGamepadAxis::new(LEGACY_HAT_X_AXIS, false),
                    state: ElementState::Pressed,
                },
                GamepadEvent::Direction {
                    slot,
                    button: ControlButton::Left,
                    state: ElementState::Pressed,
                },
            ]
        );

        output.clear();
        manager.handle_axis_for_slot(slot, Axis::DPadX, -1.0, &mut output);
        assert!(
            output.is_empty(),
            "button and axis representations of one hat transition are deduplicated"
        );

        manager.handle_button_for_slot(slot, Button::DPadLeft, ElementState::Released, &mut output);
        assert_eq!(
            output,
            [
                GamepadEvent::Axis {
                    slot,
                    axis: LegacyGamepadAxis::new(LEGACY_HAT_X_AXIS, false),
                    state: ElementState::Released,
                },
                GamepadEvent::Direction {
                    slot,
                    button: ControlButton::Left,
                    state: ElementState::Released,
                },
            ]
        );
        output.clear();
        manager.handle_axis_for_slot(slot, Axis::DPadX, 0.0, &mut output);
        assert!(output.is_empty());

        manager.handle_button_for_slot(slot, Button::DPadUp, ElementState::Pressed, &mut output);
        assert!(matches!(
            output.first(),
            Some(GamepadEvent::Axis {
                axis,
                state: ElementState::Pressed,
                ..
            }) if *axis == LegacyGamepadAxis::new(LEGACY_HAT_Y_AXIS, false)
        ));
    }

    #[test]
    fn l026_stick_and_hat_share_the_semantic_direction_state() {
        let slot = GamepadSlot::new(0);
        let mut manager = GamepadManager::disabled();
        let mut output = Vec::new();

        manager.handle_axis_for_slot(slot, Axis::LeftStickX, -1.0, &mut output);
        assert!(matches!(
            output.as_slice(),
            [
                GamepadEvent::Axis { .. },
                GamepadEvent::Direction {
                    button: ControlButton::Left,
                    state: ElementState::Pressed,
                    ..
                }
            ]
        ));

        output.clear();
        manager.handle_button_for_slot(slot, Button::DPadLeft, ElementState::Pressed, &mut output);
        assert!(matches!(output.as_slice(), [GamepadEvent::Axis { .. }]));

        output.clear();
        manager.handle_axis_for_slot(slot, Axis::LeftStickX, 0.0, &mut output);
        assert!(matches!(output.as_slice(), [GamepadEvent::Axis { .. }]));

        output.clear();
        manager.handle_button_for_slot(slot, Button::DPadLeft, ElementState::Released, &mut output);
        assert!(matches!(
            output.as_slice(),
            [
                GamepadEvent::Axis { .. },
                GamepadEvent::Direction {
                    button: ControlButton::Left,
                    state: ElementState::Released,
                    ..
                }
            ]
        ));
    }

    #[test]
    fn l026_raw_axis_precedes_its_ui_alias_in_one_source_cluster() {
        let slot = GamepadSlot::new(0);
        let events = [
            GamepadEvent::Axis {
                slot,
                axis: LegacyGamepadAxis::new(0, false),
                state: ElementState::Pressed,
            },
            GamepadEvent::Direction {
                slot,
                button: ControlButton::Left,
                state: ElementState::Pressed,
            },
        ];
        let mut next_cluster = 12;
        let mut output = Vec::new();
        append_sourced_events(0, events, &mut next_cluster, &mut output);

        assert!(matches!(output[0].event, GamepadEvent::Axis { .. }));
        assert!(matches!(output[1].event, GamepadEvent::Direction { .. }));
        assert_eq!(output[0].cluster, output[1].cluster);
        assert_eq!(next_cluster, 13);
    }

    #[test]
    fn physical_gamepads_keep_distinct_cpp_slots_on_every_emitted_event() {
        // KEY_Gamepad embeds the physical gamepad id in every input key;
        // C4Game::InitKeyboard registers each pad independently
        // (pristine 9ffa0a5d src/C4KeyboardInput.h:77-95;
        // src/C4Game.cpp:3439-3452).
        let mut manager = GamepadManager::disabled();
        let mut output = Vec::new();
        let mut starts = Vec::new();
        for slot in [GamepadSlot::new(0), GamepadSlot::new(1)] {
            let start = output.len();
            starts.push(start);
            manager.handle_button_for_slot(slot, Button::South, ElementState::Pressed, &mut output);
            assert!(output[start..].iter().all(|event| event.slot() == slot));
        }
        assert_ne!(output[starts[0]].slot(), output[starts[1]].slot());
    }

    #[test]
    fn disconnect_emits_clear_for_latched_input() {
        let slot = GamepadSlot::new(2);
        let mut output = vec![GamepadEvent::Direction {
            slot,
            button: ControlButton::Left,
            state: ElementState::Pressed,
        }];

        emit_disconnect_clear(slot, &mut output);

        assert_eq!(output.last(), Some(&GamepadEvent::Clear { slot }));
    }

    #[test]
    fn gui_button_classes_match_legacy_low_and_high_ranges() {
        for button in [Button::South, Button::East, Button::North, Button::West] {
            assert_eq!(gui_button_class(button), Some(GuiButtonClass::Low));
        }
        for button in [
            Button::C,
            Button::Z,
            Button::LeftTrigger,
            Button::LeftTrigger2,
            Button::RightTrigger,
            Button::RightTrigger2,
            Button::Select,
            Button::Start,
            Button::Mode,
            Button::LeftThumb,
            Button::RightThumb,
            Button::Unknown,
        ] {
            assert_eq!(gui_button_class(button), Some(GuiButtonClass::High));
        }
        for button in [
            Button::DPadUp,
            Button::DPadDown,
            Button::DPadLeft,
            Button::DPadRight,
        ] {
            assert_eq!(gui_button_class(button), None);
        }
    }

    #[test]
    fn semantic_button_fallback_is_an_explicit_gilrs_backend_boundary() {
        assert_eq!(
            legacy_button_from_semantic(Button::South),
            Some(LegacyGamepadButton::new(0))
        );
        assert_eq!(
            legacy_button_from_semantic(Button::Start),
            Some(LegacyGamepadButton::new(8))
        );
        assert_eq!(legacy_button_from_semantic(Button::DPadUp), None);
    }

    #[test]
    fn physical_buttons_emit_candidates_without_hardcoded_gameplay_commands() {
        // SDL button events become physical KEY_Gamepad candidates. Player
        // commands are chosen later by Config.Gamepads; C++ has no semantic
        // Start=>PlayerMenu (pristine 9ffa0a5d src/C4GamePadCon.cpp:424-432;
        // src/C4Game.cpp:3439-3452).
        let mut manager = GamepadManager::disabled();
        for button in [
            Button::South,
            Button::East,
            Button::West,
            Button::North,
            Button::LeftTrigger,
            Button::RightTrigger,
            Button::LeftTrigger2,
            Button::RightTrigger2,
            Button::Start,
        ] {
            let mut output = Vec::new();
            manager.handle_button_for_slot(
                GamepadSlot::new(0),
                button,
                ElementState::Pressed,
                &mut output,
            );
            assert!(output
                .iter()
                .any(|event| matches!(event, GamepadEvent::Button { .. })));
            assert!(!output
                .iter()
                .any(|event| matches!(event, GamepadEvent::Clear { .. })));
        }
    }
}
