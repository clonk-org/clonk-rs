use std::collections::HashMap;

use gilrs::{Axis, Button, Event, EventType, GamepadId, Gilrs};
use lc_engine::{ControlButton, ControlCommand};
use winit::event::ElementState;

const AXIS_PRESS_THRESHOLD: f32 = 0.6;
const AXIS_RELEASE_THRESHOLD: f32 = 0.4;
const GAMEPAD_SLOT_COUNT: u8 = 4;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub(crate) struct GamepadSlot(u8);

impl GamepadSlot {
    pub(crate) const fn new(index: u8) -> Self {
        Self(index)
    }

    fn from_gamepad_id(id: GamepadId) -> Option<Self> {
        u8::try_from(usize::from(id))
            .ok()
            .filter(|index| *index < GAMEPAD_SLOT_COUNT)
            .map(Self)
    }
}

pub(crate) struct GamepadManager {
    gilrs: Option<Gilrs>,
    states: HashMap<GamepadSlot, GamepadState>,
}

impl GamepadManager {
    pub(crate) fn new() -> Self {
        let gilrs = match Gilrs::new() {
            Ok(instance) => Some(instance),
            Err(error) => {
                tracing::warn!(error = %error, "failed to initialise gamepad input");
                None
            }
        };
        Self {
            gilrs,
            states: HashMap::new(),
        }
    }

    pub(crate) fn poll(&mut self) -> Vec<GamepadEvent> {
        let mut output = Vec::new();
        while let Some(event) = self.gilrs.as_mut().and_then(|gilrs| gilrs.next_event()) {
            self.process_event(event, &mut output);
        }
        output
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
            EventType::ButtonPressed(button, _) => {
                self.handle_button_for_slot(slot, button, ElementState::Pressed, output);
            }
            EventType::ButtonReleased(button, _) => {
                self.handle_button_for_slot(slot, button, ElementState::Released, output);
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
        if let Some(class) = gui_button_class(button) {
            output.push(GamepadEvent::GuiButton { slot, class, state });
        }
        let pad_state = self.states.entry(slot).or_default();
        let pressed = state == ElementState::Pressed;
        match button {
            Button::DPadLeft => {
                if let Some(change) = pad_state
                    .direction_mut(ControlButton::Left)
                    .set_dpad(pressed)
                {
                    output.push(GamepadEvent::Direction {
                        slot,
                        button: ControlButton::Left,
                        state: change,
                    });
                }
            }
            Button::DPadRight => {
                if let Some(change) = pad_state
                    .direction_mut(ControlButton::Right)
                    .set_dpad(pressed)
                {
                    output.push(GamepadEvent::Direction {
                        slot,
                        button: ControlButton::Right,
                        state: change,
                    });
                }
            }
            Button::DPadUp => {
                if let Some(change) = pad_state.direction_mut(ControlButton::Up).set_dpad(pressed) {
                    output.push(GamepadEvent::Direction {
                        slot,
                        button: ControlButton::Up,
                        state: change,
                    });
                }
            }
            Button::DPadDown => {
                if let Some(change) = pad_state
                    .direction_mut(ControlButton::Down)
                    .set_dpad(pressed)
                {
                    output.push(GamepadEvent::Direction {
                        slot,
                        button: ControlButton::Down,
                        state: change,
                    });
                }
            }
            Button::South => {
                output.push(GamepadEvent::Action {
                    slot,
                    action: GamepadActionType::Select,
                    state,
                });
                output.push(GamepadEvent::Command {
                    slot,
                    command: ControlCommand::Throw,
                    state,
                });
            }
            Button::East => {
                output.push(GamepadEvent::Action {
                    slot,
                    action: GamepadActionType::Cancel,
                    state,
                });
                output.push(GamepadEvent::Command {
                    slot,
                    command: ControlCommand::Dig,
                    state,
                });
            }
            Button::West => {
                output.push(GamepadEvent::Command {
                    slot,
                    command: ControlCommand::Special,
                    state,
                });
            }
            Button::North => {
                output.push(GamepadEvent::Command {
                    slot,
                    command: ControlCommand::Special2,
                    state,
                });
            }
            Button::LeftTrigger => {
                output.push(GamepadEvent::Command {
                    slot,
                    command: ControlCommand::CursorLeft,
                    state,
                });
            }
            Button::RightTrigger => {
                output.push(GamepadEvent::Command {
                    slot,
                    command: ControlCommand::CursorRight,
                    state,
                });
            }
            Button::LeftTrigger2 => {
                output.push(GamepadEvent::Command {
                    slot,
                    command: ControlCommand::CursorToggle,
                    state,
                });
            }
            Button::RightTrigger2 => {
                if state == ElementState::Pressed {
                    output.push(GamepadEvent::Clear { slot });
                }
            }
            Button::Start => {
                if state == ElementState::Pressed {
                    output.push(GamepadEvent::Command {
                        slot,
                        command: ControlCommand::PlayerMenu,
                        state,
                    });
                }
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
    }

    fn handle_axis_for_slot(
        &mut self,
        slot: GamepadSlot,
        axis: Axis,
        value: f32,
        output: &mut Vec<GamepadEvent>,
    ) {
        let pad_state = self.states.entry(slot).or_default();
        match axis {
            Axis::LeftStickX | Axis::DPadX => {
                if let Some(change) = pad_state
                    .direction_mut(ControlButton::Left)
                    .update_axis((-value).max(0.0))
                {
                    output.push(GamepadEvent::Direction {
                        slot,
                        button: ControlButton::Left,
                        state: change,
                    });
                }
                if let Some(change) = pad_state
                    .direction_mut(ControlButton::Right)
                    .update_axis(value.max(0.0))
                {
                    output.push(GamepadEvent::Direction {
                        slot,
                        button: ControlButton::Right,
                        state: change,
                    });
                }
            }
            Axis::LeftStickY | Axis::DPadY => {
                // Positive values point down on most controllers.
                if let Some(change) = pad_state
                    .direction_mut(ControlButton::Up)
                    .update_axis((-value).max(0.0))
                {
                    output.push(GamepadEvent::Direction {
                        slot,
                        button: ControlButton::Up,
                        state: change,
                    });
                }
                if let Some(change) = pad_state
                    .direction_mut(ControlButton::Down)
                    .update_axis(value.max(0.0))
                {
                    output.push(GamepadEvent::Direction {
                        slot,
                        button: ControlButton::Down,
                        state: change,
                    });
                }
            }
            _ => {}
        }
    }
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
/// held at removal time. Clear aggregate input just like the controller's
/// explicit clear button so gameplay and modal captures cannot remain latched.
fn emit_disconnect_clear(slot: GamepadSlot, output: &mut Vec<GamepadEvent>) {
    output.push(GamepadEvent::Clear { slot });
}

#[derive(Default)]
struct GamepadState {
    left: DirectionState,
    right: DirectionState,
    up: DirectionState,
    down: DirectionState,
}

impl GamepadState {
    fn direction_mut(&mut self, button: ControlButton) -> &mut DirectionState {
        match button {
            ControlButton::Left => &mut self.left,
            ControlButton::Right => &mut self.right,
            ControlButton::Up => &mut self.up,
            ControlButton::Down => &mut self.down,
        }
    }
}

#[derive(Default)]
struct DirectionState {
    dpad_active: bool,
    axis_active: bool,
}

impl DirectionState {
    fn set_dpad(&mut self, pressed: bool) -> Option<ElementState> {
        let previous = self.is_active();
        self.dpad_active = pressed;
        self.diff_state(previous)
    }

    fn update_axis(&mut self, magnitude: f32) -> Option<ElementState> {
        let previous = self.is_active();
        let desired = if self.axis_active {
            magnitude >= AXIS_RELEASE_THRESHOLD
        } else {
            magnitude >= AXIS_PRESS_THRESHOLD
        };
        self.axis_active = desired;
        self.diff_state(previous)
    }

    fn diff_state(&self, previous_overall: bool) -> Option<ElementState> {
        let now = self.is_active();
        if previous_overall == now {
            None
        } else if now {
            Some(ElementState::Pressed)
        } else {
            Some(ElementState::Released)
        }
    }

    fn is_active(&self) -> bool {
        self.dpad_active || self.axis_active
    }
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
    Direction {
        slot: GamepadSlot,
        button: ControlButton,
        state: ElementState,
    },
    Command {
        slot: GamepadSlot,
        command: ControlCommand,
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

impl GamepadEvent {
    pub(crate) const fn slot(self) -> GamepadSlot {
        match self {
            Self::Direction { slot, .. }
            | Self::Command { slot, .. }
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
    fn dpad_changes_direction_state() {
        let mut direction = DirectionState::default();
        assert_eq!(direction.set_dpad(true), Some(ElementState::Pressed));
        assert_eq!(direction.set_dpad(true), None);
        assert_eq!(direction.set_dpad(false), Some(ElementState::Released));
    }

    #[test]
    fn axis_uses_thresholds() {
        let mut direction = DirectionState::default();
        // Below press threshold, no activation.
        assert_eq!(direction.update_axis(0.3), None);
        // Above press threshold, activates.
        assert_eq!(direction.update_axis(0.7), Some(ElementState::Pressed));
        // Small change above release threshold keeps state.
        assert_eq!(direction.update_axis(0.45), None);
        // Drop below release threshold, release.
        assert_eq!(direction.update_axis(0.1), Some(ElementState::Released));
    }

    #[test]
    fn combining_dpad_and_axis_keeps_direction_active() {
        let mut direction = DirectionState::default();
        assert_eq!(direction.set_dpad(true), Some(ElementState::Pressed));
        // Axis activation should not emit a press because direction already active.
        assert_eq!(direction.update_axis(0.8), None);
        // Releasing dpad while axis still active keeps direction active.
        assert_eq!(direction.set_dpad(false), None);
        // Releasing axis now emits release.
        assert_eq!(direction.update_axis(0.0), Some(ElementState::Released));
    }

    #[test]
    fn physical_gamepads_keep_distinct_cpp_slots_on_every_emitted_event() {
        // KEY_Gamepad embeds the physical gamepad id in every input key;
        // C4Game::InitKeyboard registers each pad independently
        // (pristine 9ffa0a5d src/C4KeyboardInput.h:77-95;
        // src/C4Game.cpp:3439-3452).
        let mut manager = GamepadManager {
            gilrs: None,
            states: HashMap::new(),
        };
        let mut output = Vec::new();
        for slot in [GamepadSlot::new(0), GamepadSlot::new(1)] {
            let start = output.len();
            manager.handle_button_for_slot(
                slot,
                Button::South,
                ElementState::Pressed,
                &mut output,
            );
            assert!(output[start..].iter().all(|event| event.slot() == slot));
        }
        assert_ne!(output[0].slot(), output[3].slot());
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
}
