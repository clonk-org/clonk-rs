use crate::{
    control::{ControlButton, ControlEvent},
    CommandDirection,
};

/// Tracks pressed directional buttons for a single player and derives the effective command
/// direction following the classic `Coms2ComDir` mapping from the C++ runtime.
#[derive(Debug, Clone)]
pub struct PlayerInputState {
    left: bool,
    right: bool,
    up: bool,
    down: bool,
    last_direction: CommandDirection,
}

impl Default for PlayerInputState {
    fn default() -> Self {
        Self {
            left: false,
            right: false,
            up: false,
            down: false,
            last_direction: CommandDirection::Stop,
        }
    }
}

impl PlayerInputState {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn press(&mut self, button: ControlButton) -> Option<CommandDirection> {
        self.set_button(button, true)
    }

    pub fn release(&mut self, button: ControlButton) -> Option<CommandDirection> {
        self.set_button(button, false)
    }

    pub fn clear(&mut self) -> Option<CommandDirection> {
        self.left = false;
        self.right = false;
        self.up = false;
        self.down = false;
        self.update_direction(CommandDirection::Stop)
    }

    pub fn handle_event(&mut self, event: ControlEvent) -> Option<CommandDirection> {
        match event {
            ControlEvent::Press(button) => self.press(button),
            ControlEvent::Release(button) => self.release(button),
            ControlEvent::ClearPressed => self.clear(),
            ControlEvent::Command { .. } | ControlEvent::RawPlayerControl { .. } => None,
        }
    }

    pub fn direction(&self) -> CommandDirection {
        self.compute_direction()
    }

    fn set_button(&mut self, button: ControlButton, state: bool) -> Option<CommandDirection> {
        match button {
            ControlButton::Left => self.left = state,
            ControlButton::Right => self.right = state,
            ControlButton::Up => self.up = state,
            ControlButton::Down => self.down = state,
        }
        let direction = self.compute_direction();
        self.update_direction(direction)
    }

    fn compute_direction(&self) -> CommandDirection {
        let horizontal = match (self.left, self.right) {
            (true, false) => -1,
            (false, true) => 1,
            _ => 0,
        };
        let vertical = match (self.up, self.down) {
            (true, false) => -1,
            (false, true) => 1,
            _ => 0,
        };
        match (horizontal, vertical) {
            (-1, -1) => CommandDirection::UpLeft,
            (-1, 0) => CommandDirection::Left,
            (-1, 1) => CommandDirection::DownLeft,
            (0, -1) => CommandDirection::Up,
            (0, 0) => CommandDirection::Stop,
            (0, 1) => CommandDirection::Down,
            (1, -1) => CommandDirection::UpRight,
            (1, 0) => CommandDirection::Right,
            (1, 1) => CommandDirection::DownRight,
            _ => CommandDirection::Stop,
        }
    }

    fn update_direction(&mut self, direction: CommandDirection) -> Option<CommandDirection> {
        if direction != self.last_direction {
            self.last_direction = direction;
            Some(direction)
        } else {
            None
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pressing_direction_changes_command() {
        let mut state = PlayerInputState::new();
        let direction = state
            .press(ControlButton::Right)
            .expect("direction should change on press");
        assert_eq!(direction, CommandDirection::Right);
        assert_eq!(state.direction(), CommandDirection::Right);
    }

    #[test]
    fn conflicting_horizontal_inputs_cancel_out() {
        let mut state = PlayerInputState::new();
        state.press(ControlButton::Right);
        let direction = state
            .press(ControlButton::Left)
            .expect("direction should change when conflict introduced");
        assert_eq!(direction, CommandDirection::Stop);
        assert_eq!(state.direction(), CommandDirection::Stop);
    }

    #[test]
    fn diagonal_inputs_follow_mapping() {
        let mut state = PlayerInputState::new();
        state.press(ControlButton::Right);
        let direction = state
            .press(ControlButton::Up)
            .expect("direction should change to diagonal");
        assert_eq!(direction, CommandDirection::UpRight);
        assert_eq!(state.direction(), CommandDirection::UpRight);
    }

    #[test]
    fn releasing_returns_to_previous_direction() {
        let mut state = PlayerInputState::new();
        state.press(ControlButton::Down);
        state.press(ControlButton::Right);
        let direction = state
            .release(ControlButton::Right)
            .expect("direction should change when releasing");
        assert_eq!(direction, CommandDirection::Down);
        assert_eq!(state.direction(), CommandDirection::Down);
    }

    #[test]
    fn clear_resets_to_stop_once() {
        let mut state = PlayerInputState::new();
        state.press(ControlButton::Left);
        assert_eq!(
            state.clear().expect("clear should report update"),
            CommandDirection::Stop
        );
        assert!(state.clear().is_none(), "second clear is a no-op");
    }
}
