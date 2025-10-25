use std::collections::HashMap;

use lc_engine::{
    CommandDirection, CommandKind, ControlCommand, ControlEvent, CrewCommandTarget, Engine,
    EngineError, ObjectUpdate, PlayerInputState,
};

/// Centralises player input handling for the Rust frontend. Each player receives their own state
/// machine that mirrors the legacy `Coms2ComDir` mapping, and the latest direction is forwarded to
/// the currently selected crew members.
pub struct InputDispatcher {
    players: HashMap<i32, PlayerInputContext>,
}

impl InputDispatcher {
    pub fn new() -> Self {
        Self {
            players: HashMap::new(),
        }
    }

    /// Applies a control event for the given player. When the directional state changes, the engine
    /// receives the corresponding `ComDir` update. Returns the new direction when one was issued.
    pub fn handle_event(
        &mut self,
        engine: &mut Engine,
        owner: i32,
        event: ControlEvent,
    ) -> Result<Option<CommandDirection>, EngineError> {
        let frame = engine.frame();
        let context = self
            .players
            .entry(owner)
            .or_insert_with(PlayerInputContext::new);
        let maybe_direction = match event {
            ControlEvent::Press(button) => context.directional.press(button),
            ControlEvent::Release(button) => context.directional.release(button),
            ControlEvent::ClearPressed => context.directional.clear(),
            ControlEvent::Command { command, kind } => {
                let handled = handle_command(engine, owner, context, command, kind, frame)?;
                if !handled {
                    let _ = engine.handle_control_command(owner, command, kind)?;
                }
                None
            }
        };
        if let Some(direction) = maybe_direction {
            apply_direction(engine, owner, direction)?;
        }
        Ok(maybe_direction)
    }

    /// Returns the last known command direction for the given player.
    pub fn command_direction(&self, owner: i32) -> CommandDirection {
        self.players
            .get(&owner)
            .map(|context| context.directional.direction())
            .unwrap_or(CommandDirection::Stop)
    }
}

const DOUBLE_CLICK_WINDOW: u64 = 10;

struct PlayerInputContext {
    directional: PlayerInputState,
    selection: SelectionControlState,
}

impl PlayerInputContext {
    fn new() -> Self {
        Self {
            directional: PlayerInputState::new(),
            selection: SelectionControlState::new(),
        }
    }
}

#[derive(Debug)]
struct SelectionControlState {
    last_toggle_frame: Option<u64>,
}

impl SelectionControlState {
    fn new() -> Self {
        Self {
            last_toggle_frame: None,
        }
    }

    fn register_toggle(&mut self, frame: u64) -> ToggleOutcome {
        match self.last_toggle_frame {
            Some(previous) if frame.saturating_sub(previous) <= DOUBLE_CLICK_WINDOW => {
                self.last_toggle_frame = None;
                ToggleOutcome::Double
            }
            _ => {
                self.last_toggle_frame = Some(frame);
                ToggleOutcome::Single
            }
        }
    }

    fn clear(&mut self) {
        self.last_toggle_frame = None;
    }
}

enum ToggleOutcome {
    Single,
    Double,
}

#[derive(Clone, Copy)]
enum CycleDirection {
    Next,
    Previous,
}

fn apply_direction(
    engine: &mut Engine,
    owner: i32,
    direction: CommandDirection,
) -> Result<(), EngineError> {
    engine.ensure_cursor(owner)?;
    let update = ObjectUpdate::new().with_command_direction(direction);
    match engine.apply_command(owner, CrewCommandTarget::cursor(), update.clone()) {
        Ok(()) => Ok(()),
        Err(EngineError::CrewSelection { .. }) => {
            engine.apply_command(owner, CrewCommandTarget::selection(), update)
        }
        Err(error) => Err(error),
    }
}

fn cycle_cursor(
    engine: &mut Engine,
    owner: i32,
    direction: CycleDirection,
) -> Result<(), EngineError> {
    let mut crew = engine.crew_members(owner);
    if crew.is_empty() {
        return Ok(());
    }
    crew.sort_by_key(|id| id.as_u64());
    let target = match engine.crew_cursor(owner) {
        Some(current) => {
            if let Some((index, _)) = crew.iter().enumerate().find(|(_, id)| **id == current) {
                match direction {
                    CycleDirection::Next => crew.get(index + 1).copied().unwrap_or_else(|| crew[0]),
                    CycleDirection::Previous => {
                        if index == 0 {
                            *crew.last().unwrap()
                        } else {
                            crew[index - 1]
                        }
                    }
                }
            } else {
                crew[0]
            }
        }
        None => crew[0],
    };
    engine.set_crew_cursor(owner, Some(target))?;
    Ok(())
}

fn toggle_cursor_selection(engine: &mut Engine, owner: i32) -> Result<(), EngineError> {
    engine.ensure_cursor(owner)?;
    let Some(cursor) = engine.crew_cursor(owner) else {
        return Ok(());
    };
    let selected = engine.selected_crew(owner);
    if selected.contains(&cursor) {
        engine.deselect_crew(owner, [cursor]);
    } else {
        engine.select_crew(owner, [cursor])?;
    }
    Ok(())
}

fn select_all_crew(engine: &mut Engine, owner: i32) -> Result<(), EngineError> {
    let mut crew = engine.crew_members(owner);
    if crew.is_empty() {
        return Ok(());
    }
    crew.sort_by_key(|id| id.as_u64());
    let cursor = crew[0];
    engine.select_crew(owner, crew.clone())?;
    engine.set_crew_cursor(owner, Some(cursor))?;
    Ok(())
}

fn handle_command(
    engine: &mut Engine,
    owner: i32,
    context: &mut PlayerInputContext,
    command: ControlCommand,
    kind: CommandKind,
    frame: u64,
) -> Result<bool, EngineError> {
    match command {
        ControlCommand::CursorLeft => {
            if matches!(
                kind,
                CommandKind::Press | CommandKind::Single | CommandKind::Double
            ) {
                cycle_cursor(engine, owner, CycleDirection::Previous)?;
                return Ok(true);
            }
        }
        ControlCommand::CursorRight => {
            if matches!(
                kind,
                CommandKind::Press | CommandKind::Single | CommandKind::Double
            ) {
                cycle_cursor(engine, owner, CycleDirection::Next)?;
                return Ok(true);
            }
        }
        ControlCommand::CursorToggle => match kind {
            CommandKind::Release => return Ok(true),
            CommandKind::Double => {
                select_all_crew(engine, owner)?;
                context.selection.clear();
                return Ok(true);
            }
            CommandKind::Press | CommandKind::Single => {
                match context.selection.register_toggle(frame) {
                    ToggleOutcome::Single => toggle_cursor_selection(engine, owner)?,
                    ToggleOutcome::Double => {
                        select_all_crew(engine, owner)?;
                        context.selection.clear();
                    }
                }
                return Ok(true);
            }
        },
        ControlCommand::PlayerMenu => {
            if matches!(kind, CommandKind::Press) {
                // Menu system not yet implemented in Rust frontend.
                return Ok(true);
            }
        }
        _ => {}
    }
    Ok(false)
}

#[cfg(test)]
mod tests {
    use super::*;
    use lc_engine::{
        ControlButton, Definition, MovementProfile, ObjectId, SpawnConfig, Vector2, OWNER_NONE,
    };

    const WALKER_SCRIPT: &str = r#"
global func Initialize(state, random) { return nil; }
global func Step(state, frame, random) { return nil; }
"#;

    fn setup_engine() -> Engine {
        let mut engine = Engine::new();
        let mut definition =
            Definition::from_script("Walker", "Walker", WALKER_SCRIPT).expect("valid script");
        definition.set_movement_profile(MovementProfile::default());
        engine
            .register_definition(definition)
            .expect("register definition");
        engine
    }

    fn spawn_crew_member(engine: &mut Engine, owner: i32, x: i32) -> Result<ObjectId, EngineError> {
        engine.spawn_object(
            SpawnConfig::new("Walker")
                .with_owner(owner)
                .with_crew_member(true)
                .with_position(Vector2::new(x, 0)),
        )
    }

    #[test]
    fn forwards_direction_to_cursor() -> Result<(), EngineError> {
        let mut engine = setup_engine();
        let crew_id = engine
            .spawn_object(
                SpawnConfig::new("Walker")
                    .with_owner(1)
                    .with_crew_member(true)
                    .with_position(Vector2::new(0, 0)),
            )
            .expect("spawn crew");
        let mut dispatcher = InputDispatcher::new();

        dispatcher.handle_event(&mut engine, 1, ControlEvent::Press(ControlButton::Right))?;

        let snapshot = engine.snapshot();
        let crew = snapshot
            .object(crew_id)
            .expect("crew still exists")
            .command_direction;
        assert_eq!(crew, CommandDirection::Right);
        assert_eq!(engine.crew_cursor(1), Some(crew_id));
        Ok(())
    }

    #[test]
    fn clear_event_resets_direction() -> Result<(), EngineError> {
        let mut engine = setup_engine();
        engine
            .spawn_object(
                SpawnConfig::new("Walker")
                    .with_owner(1)
                    .with_crew_member(true)
                    .with_position(Vector2::new(0, 0)),
            )
            .expect("spawn crew");
        let mut dispatcher = InputDispatcher::new();

        dispatcher.handle_event(&mut engine, 1, ControlEvent::Press(ControlButton::Up))?;
        dispatcher.handle_event(&mut engine, 1, ControlEvent::ClearPressed)?;

        assert_eq!(
            dispatcher.command_direction(1),
            CommandDirection::Stop,
            "dispatcher tracked stop state"
        );
        let snapshot = engine.snapshot();
        let crew = snapshot
            .objects
            .iter()
            .find(|object| object.owner == 1)
            .expect("crew present");
        assert_eq!(crew.command_direction, CommandDirection::Stop);
        Ok(())
    }

    #[test]
    fn cursor_right_cycles_to_next_crew() -> Result<(), EngineError> {
        let mut engine = setup_engine();
        let first = spawn_crew_member(&mut engine, 1, 0)?;
        let second = spawn_crew_member(&mut engine, 1, 10)?;
        let mut dispatcher = InputDispatcher::new();

        dispatcher.handle_event(
            &mut engine,
            1,
            ControlEvent::Command {
                command: ControlCommand::CursorRight,
                kind: CommandKind::Press,
            },
        )?;

        assert_eq!(engine.crew_cursor(1), Some(second));

        dispatcher.handle_event(
            &mut engine,
            1,
            ControlEvent::Command {
                command: ControlCommand::CursorRight,
                kind: CommandKind::Press,
            },
        )?;

        assert_eq!(
            engine.crew_cursor(1),
            Some(first),
            "cursor wraps to first crew"
        );
        Ok(())
    }

    #[test]
    fn cursor_toggle_double_selects_all() -> Result<(), EngineError> {
        let mut engine = setup_engine();
        let first = spawn_crew_member(&mut engine, 1, 0)?;
        let second = spawn_crew_member(&mut engine, 1, 10)?;
        let mut dispatcher = InputDispatcher::new();

        dispatcher.handle_event(
            &mut engine,
            1,
            ControlEvent::Command {
                command: ControlCommand::CursorToggle,
                kind: CommandKind::Press,
            },
        )?;

        let selected_once = engine.selected_crew(1);
        assert_eq!(
            selected_once,
            vec![first],
            "first toggle selects cursor crew"
        );

        dispatcher.handle_event(
            &mut engine,
            1,
            ControlEvent::Command {
                command: ControlCommand::CursorToggle,
                kind: CommandKind::Press,
            },
        )?;

        let mut selected_all = engine.selected_crew(1);
        selected_all.sort_by_key(|id| id.as_u64());
        assert_eq!(
            selected_all,
            vec![first, second],
            "double toggle selects all crew"
        );
        assert_eq!(engine.crew_cursor(1), Some(first));
        Ok(())
    }

    #[test]
    fn no_crew_silently_ignores_updates() -> Result<(), EngineError> {
        let mut engine = setup_engine();
        let mut dispatcher = InputDispatcher::new();
        dispatcher.handle_event(
            &mut engine,
            OWNER_NONE,
            ControlEvent::Press(ControlButton::Left),
        )?;
        assert_eq!(
            dispatcher.command_direction(OWNER_NONE),
            CommandDirection::Left
        );
        Ok(())
    }
}
