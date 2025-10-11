use std::collections::HashMap;

use lc_engine::{
    CommandDirection, ControlEvent, CrewCommandTarget, Engine, EngineError, ObjectUpdate,
    PlayerInputState,
};

/// Centralises player input handling for the Rust frontend. Each player receives their own state
/// machine that mirrors the legacy `Coms2ComDir` mapping, and the latest direction is forwarded to
/// the currently selected crew members.
pub struct InputDispatcher {
    players: HashMap<i32, PlayerInputState>,
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
        let state = self.players.entry(owner).or_default();
        let maybe_direction = state.handle_event(event);
        if let Some(direction) = maybe_direction {
            apply_direction(engine, owner, direction)?;
        }
        Ok(maybe_direction)
    }

    /// Returns the last known command direction for the given player.
    pub fn command_direction(&self, owner: i32) -> CommandDirection {
        self.players
            .get(&owner)
            .map(PlayerInputState::direction)
            .unwrap_or(CommandDirection::Stop)
    }
}

fn apply_direction(
    engine: &mut Engine,
    owner: i32,
    direction: CommandDirection,
) -> Result<(), EngineError> {
    ensure_cursor(engine, owner)?;
    let update = ObjectUpdate::new().with_command_direction(direction);
    match engine.apply_command(owner, CrewCommandTarget::cursor(), update.clone()) {
        Ok(()) => Ok(()),
        Err(EngineError::CrewSelection { .. }) => {
            engine.apply_command(owner, CrewCommandTarget::selection(), update)
        }
        Err(error) => Err(error),
    }
}

fn ensure_cursor(engine: &mut Engine, owner: i32) -> Result<(), EngineError> {
    if engine.crew_cursor(owner).is_some() {
        return Ok(());
    }
    let mut crew = engine.crew_members(owner);
    crew.sort_by_key(|id| id.as_u64());
    if let Some(first) = crew.first().copied() {
        engine.set_crew_cursor(owner, Some(first))?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use lc_engine::{ControlButton, Definition, MovementProfile, SpawnConfig, Vector2, OWNER_NONE};

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
