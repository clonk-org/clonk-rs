use lc_engine::{CommandKind, ControlButton, ControlCommand, ControlEvent};

/// Maps gameplay input events to menu control commands following LegacyClonk's
/// `C4Menu::ConvertCom` rules. When a menu is active, the original command
/// should not reach the engine; the returned event replaces it.
pub fn map_menu_control_event(event: ControlEvent) -> Option<ControlEvent> {
    match event {
        ControlEvent::Press(button) => map_button_press(button, CommandKind::Press),
        ControlEvent::Release(button) => map_button_press(button, CommandKind::Release),
        ControlEvent::Command { command, kind } => map_command(command, kind),
        ControlEvent::ClearPressed => None,
    }
}

fn map_button_press(button: ControlButton, kind: CommandKind) -> Option<ControlEvent> {
    let command = match button {
        ControlButton::Left => ControlCommand::MenuLeft,
        ControlButton::Right => ControlCommand::MenuRight,
        ControlButton::Up => ControlCommand::MenuUp,
        ControlButton::Down => ControlCommand::MenuDown,
    };
    Some(ControlEvent::Command { command, kind })
}

fn map_command(command: ControlCommand, kind: CommandKind) -> Option<ControlEvent> {
    let mapped = match command {
        ControlCommand::Throw => ControlCommand::MenuEnter,
        ControlCommand::Dig => ControlCommand::MenuClose,
        ControlCommand::Special2 => ControlCommand::MenuEnterAll,
        _ => return None,
    };
    Some(ControlEvent::Command {
        command: mapped,
        kind,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn converts_directional_to_menu_navigation() {
        let mapped = map_menu_control_event(ControlEvent::Press(ControlButton::Up))
            .expect("press should convert");
        assert_eq!(
            mapped,
            ControlEvent::Command {
                command: ControlCommand::MenuUp,
                kind: CommandKind::Press
            }
        );

        let mapped = map_menu_control_event(ControlEvent::Release(ControlButton::Down))
            .expect("release should convert");
        assert_eq!(
            mapped,
            ControlEvent::Command {
                command: ControlCommand::MenuDown,
                kind: CommandKind::Release
            }
        );
    }

    #[test]
    fn converts_action_commands() {
        let mapped = map_menu_control_event(ControlEvent::Command {
            command: ControlCommand::Throw,
            kind: CommandKind::Press,
        })
        .expect("throw maps to menu enter");
        assert_eq!(
            mapped,
            ControlEvent::Command {
                command: ControlCommand::MenuEnter,
                kind: CommandKind::Press
            }
        );

        let mapped = map_menu_control_event(ControlEvent::Command {
            command: ControlCommand::Special2,
            kind: CommandKind::Double,
        })
        .expect("special2 maps to enter-all");
        assert_eq!(
            mapped,
            ControlEvent::Command {
                command: ControlCommand::MenuEnterAll,
                kind: CommandKind::Double
            }
        );
    }

    #[test]
    fn leaves_unrelated_inputs_untouched() {
        assert!(
            map_menu_control_event(ControlEvent::ClearPressed).is_none(),
            "clear pressed is not remapped"
        );
        assert!(
            map_menu_control_event(ControlEvent::Command {
                command: ControlCommand::PlayerMenu,
                kind: CommandKind::Press
            })
            .is_none(),
            "player menu should pass through unchanged"
        );
    }
}
