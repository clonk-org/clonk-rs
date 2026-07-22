use clonk_engine::{CommandKind, ControlButton, ControlCommand, ControlEvent};

/// Maps gameplay input events to menu control commands following LegacyClonk's
/// `C4Menu::ConvertCom` rules. When a menu is active, the original command
/// should not reach the engine; the returned event replaces it.
pub fn map_menu_control_event(event: ControlEvent) -> Option<ControlEvent> {
    match event {
        ControlEvent::Press(button) => map_button_press(button, CommandKind::Press),
        ControlEvent::Release(button) => map_button_press(button, CommandKind::Release),
        ControlEvent::Command { command, kind } => map_command(command, kind),
        raw @ ControlEvent::RawPlayerControl { .. } => Some(raw),
        ControlEvent::ClearPressed => None,
    }
}

/// C4Game::LocalPlayerControl's asynchronous cursor-menu `ConvertCom` pass.
/// Only exact base presses convert; releases and synthesized Single/Double
/// commands must stay raw. Progressive text replaces any recognized press
/// with MenuShowText before the converted control enters the queue.
pub fn map_async_cursor_menu_control_event(
    event: ControlEvent,
    text_progressing: bool,
) -> Option<ControlEvent> {
    let mapped = match event {
        ControlEvent::Press(button) => map_button_press(button, CommandKind::Press),
        ControlEvent::Command {
            command,
            kind: CommandKind::Press,
        } => map_command(command, CommandKind::Press),
        ControlEvent::Release(_)
        | ControlEvent::Command { .. }
        | ControlEvent::RawPlayerControl { .. }
        | ControlEvent::ClearPressed => None,
    }?;
    if text_progressing {
        Some(ControlEvent::Command {
            command: ControlCommand::MenuShowText,
            kind: CommandKind::Press,
        })
    } else {
        Some(mapped)
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

    #[test]
    fn async_cursor_menu_maps_exact_base_presses() {
        for (event, expected) in [
            (
                ControlEvent::Press(ControlButton::Left),
                ControlCommand::MenuLeft,
            ),
            (
                ControlEvent::Press(ControlButton::Right),
                ControlCommand::MenuRight,
            ),
            (
                ControlEvent::Press(ControlButton::Up),
                ControlCommand::MenuUp,
            ),
            (
                ControlEvent::Press(ControlButton::Down),
                ControlCommand::MenuDown,
            ),
            (
                ControlEvent::Command {
                    command: ControlCommand::Throw,
                    kind: CommandKind::Press,
                },
                ControlCommand::MenuEnter,
            ),
            (
                ControlEvent::Command {
                    command: ControlCommand::Dig,
                    kind: CommandKind::Press,
                },
                ControlCommand::MenuClose,
            ),
            (
                ControlEvent::Command {
                    command: ControlCommand::Special2,
                    kind: CommandKind::Press,
                },
                ControlCommand::MenuEnterAll,
            ),
        ] {
            assert_eq!(
                map_async_cursor_menu_control_event(event, false),
                Some(ControlEvent::Command {
                    command: expected,
                    kind: CommandKind::Press,
                })
            );
        }
    }

    #[test]
    fn async_cursor_menu_leaves_non_base_controls_raw() {
        for event in [
            ControlEvent::Release(ControlButton::Left),
            ControlEvent::Command {
                command: ControlCommand::Throw,
                kind: CommandKind::Release,
            },
            ControlEvent::Command {
                command: ControlCommand::Throw,
                kind: CommandKind::Single,
            },
            ControlEvent::Command {
                command: ControlCommand::Throw,
                kind: CommandKind::Double,
            },
            ControlEvent::Command {
                command: ControlCommand::Special,
                kind: CommandKind::Press,
            },
            ControlEvent::Command {
                command: ControlCommand::MenuLeft,
                kind: CommandKind::Press,
            },
            ControlEvent::RawPlayerControl {
                command: 1,
                data: 0,
            },
            ControlEvent::ClearPressed,
        ] {
            assert_eq!(map_async_cursor_menu_control_event(event, false), None);
        }
    }

    #[test]
    fn progressive_text_reveal_only_maps_exact_local_presses() {
        let show_text = ControlEvent::Command {
            command: ControlCommand::MenuShowText,
            kind: CommandKind::Press,
        };
        assert_eq!(
            map_async_cursor_menu_control_event(ControlEvent::Press(ControlButton::Left), true),
            Some(show_text)
        );
        assert_eq!(
            map_async_cursor_menu_control_event(
                ControlEvent::Command {
                    command: ControlCommand::Throw,
                    kind: CommandKind::Press,
                },
                true,
            ),
            Some(show_text)
        );
        for event in [
            ControlEvent::Release(ControlButton::Left),
            ControlEvent::Command {
                command: ControlCommand::Throw,
                kind: CommandKind::Release,
            },
            ControlEvent::Command {
                command: ControlCommand::MenuLeft,
                kind: CommandKind::Press,
            },
            ControlEvent::RawPlayerControl {
                command: 1,
                data: 0,
            },
        ] {
            assert_eq!(map_async_cursor_menu_control_event(event, true), None);
        }
    }
}
