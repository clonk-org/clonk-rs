use std::collections::HashMap;

use gilrs::{Axis, Button, Event, EventType, GamepadId, Gilrs};
use lc_engine::{ControlButton, ControlCommand};
use winit::event::ElementState;

const AXIS_PRESS_THRESHOLD: f32 = 0.6;
const AXIS_RELEASE_THRESHOLD: f32 = 0.4;

pub(crate) struct GamepadManager {
    gilrs: Option<Gilrs>,
    states: HashMap<GamepadId, GamepadState>,
    next_cluster: u64,
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
            next_cluster: 0,
        }
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

    fn process_event(&mut self, event: Event, output: &mut Vec<GamepadEvent>) {
        match event.event {
            EventType::Connected => {
                self.states.entry(event.id).or_default();
            }
            EventType::Disconnected => {
                self.states.remove(&event.id);
                emit_disconnect_clear(output);
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
        if let Some(class) = gui_button_class(button) {
            output.push(GamepadEvent::GuiButton { class, state });
        }
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

fn append_sourced_events(
    gamepad: usize,
    events: impl IntoIterator<Item = GamepadEvent>,
    next_cluster: &mut u64,
    output: &mut Vec<SourcedGamepadEvent>,
) {
    let mut gui_cluster = None;
    for event in events {
        let starts_gui_cluster = matches!(event, GamepadEvent::GuiButton { .. });
        let direction = matches!(event, GamepadEvent::Direction { .. });
        let cluster = match (starts_gui_cluster, direction, gui_cluster) {
            (false, false, Some(cluster)) => cluster,
            _ => {
                let cluster = *next_cluster;
                *next_cluster = (*next_cluster).wrapping_add(1);
                gui_cluster = starts_gui_cluster.then_some(cluster);
                cluster
            }
        };
        output.push(SourcedGamepadEvent {
            gamepad,
            cluster,
            event,
        });
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
fn emit_disconnect_clear(output: &mut Vec<GamepadEvent>) {
    output.push(GamepadEvent::Clear);
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
        button: ControlButton,
        state: ElementState,
    },
    Command {
        command: ControlCommand,
        state: ElementState,
    },
    Clear,
    GuiButton {
        class: GuiButtonClass,
        state: ElementState,
    },
    Action {
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
                    class: GuiButtonClass::High,
                    state: ElementState::Pressed,
                },
                GamepadEvent::Action {
                    action: GamepadActionType::MenuToggle,
                    state: ElementState::Pressed,
                },
                GamepadEvent::Clear,
            ],
            &mut next_cluster,
            &mut output,
        );
        assert_eq!(output.iter().map(|event| event.gamepad).collect::<Vec<_>>(), [3, 3, 3]);
        assert_eq!(output.iter().map(|event| event.cluster).collect::<Vec<_>>(), [7, 7, 7]);

        append_sourced_events(
            3,
            [
                GamepadEvent::Clear,
                GamepadEvent::Direction {
                    button: ControlButton::Left,
                    state: ElementState::Released,
                },
                GamepadEvent::Direction {
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
    fn disconnect_emits_clear_for_latched_input() {
        let mut output = vec![GamepadEvent::Direction {
            button: ControlButton::Left,
            state: ElementState::Pressed,
        }];

        emit_disconnect_clear(&mut output);

        assert_eq!(output.last(), Some(&GamepadEvent::Clear));
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
