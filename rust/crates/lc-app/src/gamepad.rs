use std::collections::HashMap;

use gilrs::{Axis, Button, Event, EventType, GamepadId, Gilrs};
use lc_engine::{ControlButton, ControlCommand};
use winit::event::ElementState;

const AXIS_PRESS_THRESHOLD: f32 = 0.6;
const AXIS_RELEASE_THRESHOLD: f32 = 0.4;

pub(crate) struct GamepadManager {
    gilrs: Option<Gilrs>,
    states: HashMap<GamepadId, GamepadState>,
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
        match event.event {
            EventType::Connected => {
                self.states.entry(event.id).or_default();
            }
            EventType::Disconnected => {
                self.states.remove(&event.id);
            }
            EventType::ButtonPressed(button, _) => {
                self.handle_button(event.id, button, ElementState::Pressed, output);
            }
            EventType::ButtonReleased(button, _) => {
                self.handle_button(event.id, button, ElementState::Released, output);
            }
            EventType::AxisChanged(axis, value, _) => {
                self.handle_axis(event.id, axis, value, output);
            }
            _ => {}
        }
    }

    fn handle_button(
        &mut self,
        id: GamepadId,
        button: Button,
        state: ElementState,
        output: &mut Vec<GamepadEvent>,
    ) {
        let pad_state = self.states.entry(id).or_default();
        let pressed = state == ElementState::Pressed;
        match button {
            Button::DPadLeft => {
                if let Some(change) = pad_state
                    .direction_mut(ControlButton::Left)
                    .set_dpad(pressed)
                {
                    output.push(GamepadEvent::Direction {
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
                        button: ControlButton::Right,
                        state: change,
                    });
                }
            }
            Button::DPadUp => {
                if let Some(change) = pad_state.direction_mut(ControlButton::Up).set_dpad(pressed) {
                    output.push(GamepadEvent::Direction {
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
                        button: ControlButton::Down,
                        state: change,
                    });
                }
            }
            Button::South => {
                output.push(GamepadEvent::Action {
                    action: GamepadActionType::Select,
                    state,
                });
                output.push(GamepadEvent::Command {
                    command: ControlCommand::Throw,
                    state,
                });
            }
            Button::East => {
                output.push(GamepadEvent::Action {
                    action: GamepadActionType::Cancel,
                    state,
                });
                output.push(GamepadEvent::Command {
                    command: ControlCommand::Dig,
                    state,
                });
            }
            Button::West => {
                output.push(GamepadEvent::Command {
                    command: ControlCommand::Special,
                    state,
                });
            }
            Button::North => {
                output.push(GamepadEvent::Command {
                    command: ControlCommand::Special2,
                    state,
                });
            }
            Button::LeftTrigger => {
                output.push(GamepadEvent::Command {
                    command: ControlCommand::CursorLeft,
                    state,
                });
            }
            Button::RightTrigger => {
                output.push(GamepadEvent::Command {
                    command: ControlCommand::CursorRight,
                    state,
                });
            }
            Button::LeftTrigger2 => {
                output.push(GamepadEvent::Command {
                    command: ControlCommand::CursorToggle,
                    state,
                });
            }
            Button::RightTrigger2 => {
                if state == ElementState::Pressed {
                    output.push(GamepadEvent::Clear);
                }
            }
            Button::Start => {
                if state == ElementState::Pressed {
                    output.push(GamepadEvent::Command {
                        command: ControlCommand::PlayerMenu,
                        state,
                    });
                }
            }
            Button::Select => {
                output.push(GamepadEvent::Action {
                    action: GamepadActionType::MenuToggle,
                    state,
                });
            }
            _ => {}
        }
    }

    fn handle_axis(
        &mut self,
        id: GamepadId,
        axis: Axis,
        value: f32,
        output: &mut Vec<GamepadEvent>,
    ) {
        let pad_state = self.states.entry(id).or_default();
        match axis {
            Axis::LeftStickX | Axis::DPadX => {
                if let Some(change) = pad_state
                    .direction_mut(ControlButton::Left)
                    .update_axis((-value).max(0.0))
                {
                    output.push(GamepadEvent::Direction {
                        button: ControlButton::Left,
                        state: change,
                    });
                }
                if let Some(change) = pad_state
                    .direction_mut(ControlButton::Right)
                    .update_axis(value.max(0.0))
                {
                    output.push(GamepadEvent::Direction {
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
                        button: ControlButton::Up,
                        state: change,
                    });
                }
                if let Some(change) = pad_state
                    .direction_mut(ControlButton::Down)
                    .update_axis(value.max(0.0))
                {
                    output.push(GamepadEvent::Direction {
                        button: ControlButton::Down,
                        state: change,
                    });
                }
            }
            _ => {}
        }
    }
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
pub(crate) enum GamepadEvent {
    Direction {
        button: ControlButton,
        state: ElementState,
    },
    Command {
        command: ControlCommand,
        state: ElementState,
    },
    Clear,
    Action {
        action: GamepadActionType,
        state: ElementState,
    },
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
}
