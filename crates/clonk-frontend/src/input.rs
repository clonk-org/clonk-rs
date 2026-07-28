use clonk_engine::{
    CommandDirection, CommandKind, ControlButton, ControlCommand, ControlEvent, Engine,
    EngineError, COM_CLEAR_PRESSED_COMS, COM_CURSOR_LEFT, COM_CURSOR_RIGHT, COM_CURSOR_TOGGLE,
    COM_DIG, COM_DOUBLE, COM_DOWN, COM_LEFT, COM_MENU_CLOSE, COM_MENU_DOWN, COM_MENU_ENTER,
    COM_MENU_ENTER_ALL, COM_MENU_LEFT, COM_MENU_RIGHT, COM_MENU_SHOW_TEXT, COM_MENU_UP,
    COM_RELEASE_OFFSET, COM_RIGHT, COM_SINGLE, COM_SPECIAL, COM_SPECIAL2, COM_THROW, COM_UP,
};

/// Player input routing for the Rust frontend. All synchronized coms —
/// object-directed AND cursor coms — run the engine's
/// `C4ControlPlayerControl::Execute` + `C4Player::InCom` port
/// (single/double synthesis, the cursor selection model and the full
/// `C4Object::DirectCom` chain, C4Player.cpp:1490-1554 / 1235-1488 /
/// C4Object.cpp:3327-3557). App-owned menus consume their commands before
/// this layer; synchronized cursor-menu commands continue into the engine.
#[derive(Default)]
pub struct InputDispatcher {}

impl InputDispatcher {
    pub fn new() -> Self {
        Self::default()
    }

    /// Applies a control event for the given player. Returns the direction
    /// change the event caused, if any (informational; movement itself is
    /// applied by the engine's ObjectComMovement fallback).
    pub fn handle_event(
        &mut self,
        engine: &mut Engine,
        owner: i32,
        event: ControlEvent,
    ) -> Result<Option<CommandDirection>, EngineError> {
        match event {
            ControlEvent::Press(button) => {
                engine.execute_player_control(owner, i32::from(button_com(button)), 0)?;
            }
            ControlEvent::Release(button) => {
                engine.execute_player_control(
                    owner,
                    i32::from(button_com(button) + COM_RELEASE_OFFSET),
                    0,
                )?;
            }
            ControlEvent::ClearPressed => {
                engine.execute_player_control(owner, i32::from(COM_CLEAR_PRESSED_COMS), 0)?;
            }
            ControlEvent::Command { command, kind } => {
                let handled = handle_command(engine, owner, command, kind)?;
                if !handled {
                    if let Some(com) = command_com(command, kind) {
                        engine.execute_player_control(owner, i32::from(com), 0)?;
                    }
                }
            }
            ControlEvent::RawPlayerControl { command, data } => {
                engine.execute_player_control(owner, i32::from(command), data)?;
            }
        }
        Ok(None)
    }

    /// Returns the cursor crew's current command direction for the player.
    pub fn command_direction(&self, engine: &Engine, owner: i32) -> CommandDirection {
        engine
            .crew_cursor(owner)
            .and_then(|cursor| engine.object_snapshot(cursor))
            .map(|snapshot| snapshot.command_direction)
            .unwrap_or(CommandDirection::Stop)
    }
}

/// The plain com byte for a directional button (C4Constants.h:178-181).
fn button_com(button: ControlButton) -> u8 {
    match button {
        ControlButton::Left => COM_LEFT,
        ControlButton::Right => COM_RIGHT,
        ControlButton::Up => COM_UP,
        ControlButton::Down => COM_DOWN,
    }
}

/// The com byte for a non-directional control event.
fn command_com(command: ControlCommand, kind: CommandKind) -> Option<u8> {
    let menu_com = match command {
        ControlCommand::MenuEnter => Some(COM_MENU_ENTER),
        ControlCommand::MenuEnterAll => Some(COM_MENU_ENTER_ALL),
        ControlCommand::MenuClose => Some(COM_MENU_CLOSE),
        ControlCommand::MenuShowText => Some(COM_MENU_SHOW_TEXT),
        ControlCommand::MenuLeft => Some(COM_MENU_LEFT),
        ControlCommand::MenuRight => Some(COM_MENU_RIGHT),
        ControlCommand::MenuUp => Some(COM_MENU_UP),
        ControlCommand::MenuDown => Some(COM_MENU_DOWN),
        // MenuSelect needs C4ControlPlayerControl::Data; ControlEvent does
        // not carry it yet, so forwarding it as index zero would be wrong.
        _ => None,
    };
    if let Some(menu_com) = menu_com {
        return matches!(kind, CommandKind::Press).then_some(menu_com);
    }
    let base = match command {
        ControlCommand::Throw => COM_THROW,
        ControlCommand::Dig => COM_DIG,
        ControlCommand::Special => COM_SPECIAL,
        ControlCommand::Special2 => COM_SPECIAL2,
        _ => return None,
    };
    Some(match kind {
        CommandKind::Press => base,
        CommandKind::Release => base + COM_RELEASE_OFFSET,
        CommandKind::Single => base | COM_SINGLE,
        CommandKind::Double => base | COM_DOUBLE,
    })
}

fn handle_command(
    engine: &mut Engine,
    owner: i32,
    command: ControlCommand,
    kind: CommandKind,
) -> Result<bool, EngineError> {
    match command {
        ControlCommand::CursorLeft | ControlCommand::CursorRight | ControlCommand::CursorToggle => {
            // The engine runs the full C4Player cursor model: InCom
            // synthesizes Single/Double from the plain presses
            // (C4Player.cpp:1522-1536) and DirectCom dispatches
            // CursorLeft/CursorRight/CursorToggle/SelectAllCrew
            // (C4Player.cpp:1457-1485).
            let base = match command {
                ControlCommand::CursorLeft => COM_CURSOR_LEFT,
                ControlCommand::CursorRight => COM_CURSOR_RIGHT,
                _ => COM_CURSOR_TOGGLE,
            };
            match kind {
                CommandKind::Press => engine.execute_player_control(owner, i32::from(base), 0)?,
                CommandKind::Release => {
                    engine.execute_player_control(owner, i32::from(base + COM_RELEASE_OFFSET), 0)?
                }
                // Explicit pre-detected Single/Double events are already
                // synchronized packet commands. Plain Press still lets
                // InCom perform its own synthesis.
                CommandKind::Double => {
                    engine.execute_player_control(owner, i32::from(base | COM_DOUBLE), 0)?
                }
                CommandKind::Single => {
                    engine.execute_player_control(owner, i32::from(base | COM_SINGLE), 0)?
                }
            }
            return Ok(true);
        }
        ControlCommand::PlayerMenu => {
            if matches!(kind, CommandKind::Press) {
                // Menu handling occurs in the app layer; nothing to forward to the engine.
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
    use clonk_engine::ocf;
    use clonk_engine::{
        ActionSpec, ActionState, Definition, MovementProfile, ObjectId, ObjectUpdate, PhysicalInfo,
        PlayerConfig, SpawnConfig, Vector2, OWNER_NONE,
    };
    use std::collections::HashMap;

    const WALKER_SCRIPT: &str = r#"
global func Initialize(state, random) { return 0; }
global func Step(state, frame, random) { return 0; }
"#;

    fn walker_actions() -> HashMap<String, ActionSpec> {
        let mut actions = HashMap::new();
        actions.insert("Walk".to_string(), ActionSpec::for_procedure("walk"));
        actions.insert("Jump".to_string(), ActionSpec::for_procedure("flight"));
        actions.insert("Push".to_string(), ActionSpec::for_procedure("push"));
        actions
    }

    fn setup_engine() -> Engine {
        let mut engine = Engine::new();
        let mut definition =
            Definition::from_script("Walker", "Walker", WALKER_SCRIPT).expect("valid script");
        definition.configure_actions(Some("Walk".to_string()), walker_actions());
        definition.set_movement_profile(MovementProfile::default());
        let physical = PhysicalInfo {
            walk: 70_000,
            jump: 40_000,
            ..Default::default()
        };
        definition.set_physical(physical);
        engine
            .register_definition(definition)
            .expect("register definition");
        engine
            .register_player(PlayerConfig::new(1, "Test"))
            .expect("register player");
        engine
    }

    fn spawn_crew_member(engine: &mut Engine, owner: i32, x: i32) -> Result<ObjectId, EngineError> {
        engine.spawn_object(
            SpawnConfig::new("Walker")
                .with_owner(owner)
                .with_crew_member(true)
                .with_action(ActionState::new("Walk"))
                .with_position(Vector2::new(x, 0)),
        )
    }

    #[test]
    fn forwards_direction_to_cursor() -> Result<(), EngineError> {
        // COM_Right in DFA_WALK reaches ObjectComMovement(COMD_Right)
        // through the engine chain (C4Object.cpp:3412).
        let mut engine = setup_engine();
        let crew_id = spawn_crew_member(&mut engine, 1, 0)?;
        let mut dispatcher = InputDispatcher::new();

        dispatcher.handle_event(&mut engine, 1, ControlEvent::Press(ControlButton::Right))?;

        let snapshot = engine.snapshot();
        let crew = snapshot
            .object(crew_id)
            .expect("crew still exists")
            .command_direction;
        assert_eq!(crew, CommandDirection::Right);
        assert_eq!(engine.crew_cursor(1), Some(crew_id));
        let player = engine.player(1).expect("player remains registered");
        assert_eq!(
            (player.control_count(), player.action_count()),
            (1, 1),
            "frontend input must pass through PlayerControl Execute"
        );
        Ok(())
    }

    #[test]
    fn forwards_recorded_menu_navigation_to_the_cursor_menu() -> Result<(), EngineError> {
        // C4Game::LocalPlayerControl queues the already-converted menu com
        // for synchronized execution (C4Game.cpp:3610-3622). Replays and
        // network peers therefore deliver COM_MenuRight directly.
        let script = r#"
        func Initialize() {
            CreateMenu(WIPF, this(), this(), 0, "Choose");
            AddMenuItem("First", "0", WIPF, this());
            AddMenuItem("Second", "0", WIPF, this());
        }
        "#;
        let mut engine = setup_engine();
        let mut definition =
            Definition::from_script("MenuWalker", "Menu Walker", script).expect("valid script");
        definition.configure_actions(Some("Walk".to_string()), walker_actions());
        definition.set_movement_profile(MovementProfile::default());
        engine
            .register_definition(definition)
            .expect("register menu walker");
        let crew = engine.spawn_object(
            SpawnConfig::new("MenuWalker")
                .with_owner(1)
                .with_crew_member(true)
                .with_action(ActionState::new("Walk")),
        )?;
        let mut dispatcher = InputDispatcher::new();

        dispatcher.handle_event(
            &mut engine,
            1,
            ControlEvent::Command {
                command: ControlCommand::MenuRight,
                kind: CommandKind::Press,
            },
        )?;

        assert_eq!(
            engine
                .debug_object_menu(crew.as_u64())
                .expect("crew exists")
                .expect("menu remains open")
                .selection,
            1
        );
        Ok(())
    }

    #[test]
    fn direction_moves_the_cursor_and_makes_selection_follow() -> Result<(), EngineError> {
        // C4Player::ObjectCom applies the com to the CURSOR only
        // (C4Player.cpp:1380-1387); other selected crew receive a Follow
        // command via ObjectComMovement (C4ObjectCom.cpp:224).
        let mut engine = setup_engine();
        let first = spawn_crew_member(&mut engine, 1, 0)?;
        let second = spawn_crew_member(&mut engine, 1, 10)?;
        engine.select_crew(1, vec![first, second])?;
        engine.set_crew_cursor(1, Some(first))?;
        let mut dispatcher = InputDispatcher::new();

        dispatcher.handle_event(&mut engine, 1, ControlEvent::Press(ControlButton::Right))?;

        let snapshot = engine.snapshot();
        assert_eq!(
            snapshot.object(first).expect("cursor").command_direction,
            CommandDirection::Right
        );
        let second_snapshot = snapshot.object(second).expect("selected crew");
        assert_eq!(
            second_snapshot.command_direction,
            CommandDirection::Stop,
            "non-cursor crew get no direct com"
        );
        assert_eq!(
            second_snapshot.command_stack.command_names(),
            vec!["Follow"],
            "selected crew follow the moving cursor (C4ObjectCom.cpp:224)"
        );
        Ok(())
    }

    #[test]
    fn clear_event_resets_pressed_coms_without_stopping() -> Result<(), EngineError> {
        // COM_ClearPressedComs resets the input state only
        // (C4Player::InCom, C4Player.cpp:1496-1501); classic control keeps
        // the last ComDir until a stopping com arrives.
        let mut engine = setup_engine();
        let crew_id = spawn_crew_member(&mut engine, 1, 0)?;
        let mut dispatcher = InputDispatcher::new();

        dispatcher.handle_event(&mut engine, 1, ControlEvent::Press(ControlButton::Up))?;
        dispatcher.handle_event(&mut engine, 1, ControlEvent::ClearPressed)?;

        let snapshot = engine.snapshot();
        let crew = snapshot.object(crew_id).expect("crew present");
        assert_eq!(
            crew.command_direction,
            CommandDirection::Stop,
            "COM_Up in WALK jumps instead of steering (C4Object.cpp:3414)"
        );
        Ok(())
    }

    #[test]
    fn cursor_right_cycles_to_next_crew() -> Result<(), EngineError> {
        // C4Player::CursorRight (C4Player.cpp:1261-1275): without a cursor
        // the scan starts at Crew.First; from a cursor it walks ->Next and
        // wraps by rescanning from the front.
        let mut engine = setup_engine();
        let first = spawn_crew_member(&mut engine, 1, 0)?;
        let second = spawn_crew_member(&mut engine, 1, 10)?;
        let mut dispatcher = InputDispatcher::new();
        let press = ControlEvent::Command {
            command: ControlCommand::CursorRight,
            kind: CommandKind::Press,
        };

        dispatcher.handle_event(&mut engine, 1, press)?;
        assert_eq!(
            engine.crew_cursor(1),
            Some(second),
            "no cursor: the newest-first crew list starts at Crew.First (C4Player.cpp:1268-1270)"
        );

        dispatcher.handle_event(&mut engine, 1, press)?;
        assert_eq!(engine.crew_cursor(1), Some(first));

        dispatcher.handle_event(&mut engine, 1, press)?;
        assert_eq!(
            engine.crew_cursor(1),
            Some(second),
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

        // Pure toggle (no CursorSelection): every crew's Select flips ON
        // (C4Player::CursorToggle, C4Player.cpp:1329-1336).
        let mut selected_once = engine.selected_crew(1);
        selected_once.sort_by_key(|id| id.as_u64());
        assert_eq!(
            selected_once,
            vec![first, second],
            "pure toggle flips Select on the whole crew"
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
        assert_eq!(engine.crew_cursor(1), Some(second));
        Ok(())
    }

    #[test]
    fn cursor_toggle_double_command_selects_all_immediately() -> Result<(), EngineError> {
        let mut engine = setup_engine();
        let first = spawn_crew_member(&mut engine, 1, 0)?;
        let second = spawn_crew_member(&mut engine, 1, 10)?;
        let mut dispatcher = InputDispatcher::new();

        dispatcher.handle_event(
            &mut engine,
            1,
            ControlEvent::Command {
                command: ControlCommand::CursorToggle,
                kind: CommandKind::Double,
            },
        )?;

        let mut selected = engine.selected_crew(1);
        selected.sort_by_key(|id| id.as_u64());
        assert_eq!(selected, vec![first, second]);
        assert_eq!(engine.crew_cursor(1), Some(second));
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
        Ok(())
    }

    #[test]
    fn double_down_grabs_nearby_vehicle_into_push() -> Result<(), EngineError> {
        // COM_Down_D in DFA_WALK → ObjectComDownDouble → C4CMD_Grab on the
        // OCF_Grab object at the clonk (C4Object.cpp:3415,
        // C4ObjectCom.cpp:573-589); grabbing puts the clonk into Push
        // toward the target (ObjectComGrab, C4ObjectCom.cpp:247-259).
        let mut engine = setup_engine();
        let mut cart_definition =
            Definition::from_script("Cart", "Cart", WALKER_SCRIPT).expect("valid script");
        cart_definition.set_ocf_base(ocf::GRAB);
        cart_definition.set_shape_rect(Some(clonk_engine::DefinitionRect::new(-8, -8, 16, 16)));
        engine
            .register_definition(cart_definition)
            .expect("register grabbable definition");
        let crew = spawn_crew_member(&mut engine, 1, 0)?;
        engine.select_crew(1, vec![crew])?;
        engine.set_crew_cursor(1, Some(crew))?;
        let cart =
            engine.spawn_object(SpawnConfig::new("Cart").with_position(Vector2::new(6, 2)))?;
        let mut dispatcher = InputDispatcher::new();

        dispatcher.handle_event(&mut engine, 1, ControlEvent::Press(ControlButton::Down))?;
        dispatcher.handle_event(&mut engine, 1, ControlEvent::Release(ControlButton::Down))?;
        dispatcher.handle_event(&mut engine, 1, ControlEvent::Press(ControlButton::Down))?;

        for _ in 0..10 {
            engine.tick()?;
        }
        let crew_snapshot = engine.object_snapshot(crew).expect("crew snapshot");
        assert_eq!(
            crew_snapshot.action.name, "Push",
            "grabbing enters the Push action (C4ObjectCom.cpp:247-259)"
        );
        assert_eq!(crew_snapshot.action.target, Some(cart));
        Ok(())
    }

    #[test]
    fn down_down_throw_drops_carried_object() -> Result<(), EngineError> {
        // The classic drop: COM_Down_D arms LastComDownDouble and the next
        // throw converts to C4CMD_Drop (PlayerObjectCommand,
        // C4ObjectCom.cpp:1020-1036) which exits the carried object
        // (ObjectComDrop, C4ObjectCom.cpp:640-676).
        let mut engine = setup_engine();
        let mut item_definition =
            Definition::from_script("Gem", "Gem", WALKER_SCRIPT).expect("valid script");
        item_definition.set_ocf_base(ocf::CARRYABLE);
        engine
            .register_definition(item_definition)
            .expect("register item definition");
        let crew = spawn_crew_member(&mut engine, 1, 0)?;
        engine.select_crew(1, vec![crew])?;
        engine.set_crew_cursor(1, Some(crew))?;
        let item = engine.spawn_object(SpawnConfig::new("Gem"))?;
        engine.apply_object_update(item, ObjectUpdate::new().with_container(crew))?;
        assert_eq!(
            engine.object_snapshot(item).expect("item").container,
            Some(crew)
        );
        let mut dispatcher = InputDispatcher::new();

        dispatcher.handle_event(&mut engine, 1, ControlEvent::Press(ControlButton::Down))?;
        dispatcher.handle_event(&mut engine, 1, ControlEvent::Release(ControlButton::Down))?;
        dispatcher.handle_event(&mut engine, 1, ControlEvent::Press(ControlButton::Down))?;
        dispatcher.handle_event(&mut engine, 1, ControlEvent::Release(ControlButton::Down))?;
        dispatcher.handle_event(
            &mut engine,
            1,
            ControlEvent::Command {
                command: ControlCommand::Throw,
                kind: CommandKind::Press,
            },
        )?;

        for _ in 0..10 {
            engine.tick()?;
        }
        let item_snapshot = engine.object_snapshot(item).expect("item snapshot");
        assert!(
            item_snapshot.container.is_none(),
            "down-down-throw drops the carried object"
        );
        Ok(())
    }

    #[test]
    fn press_up_enters_nearby_structure() -> Result<(), EngineError> {
        // COM_Up in DFA_WALK → ObjectComUp → C4CMD_Enter on the entrance at
        // the clonk (C4ObjectCom.cpp:335-345).
        let mut engine = setup_engine();
        let mut structure_definition =
            Definition::from_script("Hut", "Hut", WALKER_SCRIPT).expect("valid script");
        // A real Entrance area: AtObject(x, y, OCF_Entrance) verifies the
        // probe against Def->Entrance (GetOCFForPos, C4Object.cpp:1149-1153),
        // and OCF_Container follows from the entrance (C4Object.cpp:658-660).
        structure_definition
            .set_entrance_rect(Some(clonk_engine::DefinitionRect::new(-16, -16, 32, 32)));
        structure_definition
            .set_shape_rect(Some(clonk_engine::DefinitionRect::new(-16, -16, 32, 32)));
        engine
            .register_definition(structure_definition)
            .expect("register structure definition");
        let crew = spawn_crew_member(&mut engine, 1, 0)?;
        engine.select_crew(1, vec![crew])?;
        engine.set_crew_cursor(1, Some(crew))?;
        let hut = engine.spawn_object(SpawnConfig::new("Hut").with_position(Vector2::new(0, 2)))?;
        // This synthetic hut has no door script; model its already-open state
        // explicitly, as SetEntrance(1) does in C++ (C4Script.cpp:690-695).
        let mut open_entrance = ObjectUpdate::new();
        open_entrance.entrance_status = Some(true);
        engine.apply_object_update(hut, open_entrance)?;
        let mut dispatcher = InputDispatcher::new();

        dispatcher.handle_event(&mut engine, 1, ControlEvent::Press(ControlButton::Up))?;
        for _ in 0..20 {
            engine.tick()?;
        }

        let crew_snapshot = engine.object_snapshot(crew).expect("crew snapshot");
        assert_eq!(crew_snapshot.container, Some(hut));
        Ok(())
    }

    #[test]
    fn press_up_without_entrance_jumps() -> Result<(), EngineError> {
        // COM_Up with no entrance issues C4CMD_Jump (C4ObjectCom.cpp:347-348)
        // which launches the clonk upward (ObjectComJump,
        // C4ObjectCom.cpp:280-308).
        let mut engine = setup_engine();
        let crew = spawn_crew_member(&mut engine, 1, 0)?;
        engine.select_crew(1, vec![crew])?;
        engine.set_crew_cursor(1, Some(crew))?;
        let mut dispatcher = InputDispatcher::new();

        dispatcher.handle_event(&mut engine, 1, ControlEvent::Press(ControlButton::Up))?;
        engine.tick()?;

        let crew_snapshot = engine.object_snapshot(crew).expect("crew snapshot");
        assert_eq!(
            crew_snapshot.action.name, "Jump",
            "the jump command launched the clonk"
        );
        assert!(
            crew_snapshot.velocity.y < 0,
            "jump velocity points upward (ObjectComJump ydir)"
        );
        Ok(())
    }

    #[test]
    fn goldrush_style_control_chain_runs_50_ticks_without_errors() -> Result<(), EngineError> {
        // The GoldRush TRPR/COWB regression, headless: the crew clonk runs
        // the VERBATIM CLNK control chain (Clonk.c4d/Script.c:195-241,
        // 860-875) — int returns, Control2Effect, EffectCall by number —
        // while an armed item answers Fx<Name>Control* on its command
        // target (FnEffectCall, C4Script.cpp:5589-5601). Dig press/release,
        // throw and jump (Up) must never error; C++ plays this content
        // without a single script warning.
        let clonk_script = r#"
#strict
protected func ControlDig()
{
  if (Control2Effect("ControlDig")) return(1);
  return(0);
}
protected func ControlDigReleased()
{
  if (Control2Effect("ControlDigReleased")) return(1);
  return(0);
}
protected func ControlThrow()
{
  if (Control2Effect("ControlThrow")) return(1);
  return(0);
}
private func Control2Effect(string szControl)
{
  var i = GetEffectCount(0, this()), iEffect;
  var res;
  while (i--)
  {
    iEffect = GetEffect("*Control*", this(), i);
    if ( GetEffect(0, this(), iEffect, 1) )
      res += EffectCall(this(), iEffect, szControl);
  }
  return(res);
}
"#;
        let gun_script = r#"
#strict
public func Arm()
{
  AddEffect("GunControl", FindObject(TRPR), 100, 0, this());
  return(1);
}
public func FxGunControlControlDig(pTarget, iNumber)
{
  EffectVar(0, pTarget, iNumber) = EffectVar(0, pTarget, iNumber) + 1;
  return(1);
}
public func FxGunControlControlDigReleased(pTarget, iNumber)
{
  EffectVar(0, pTarget, iNumber) = EffectVar(0, pTarget, iNumber) + 1;
  return(1);
}
public func FxGunControlControlThrow(pTarget, iNumber)
{
  EffectVar(0, pTarget, iNumber) = EffectVar(0, pTarget, iNumber) + 1;
  return(1);
}
"#;
        let mut clonk = Definition::from_script("TRPR", "Trapper", clonk_script)
            .expect("clonk script compiles");
        let mut actions = HashMap::new();
        actions.insert("Idle".to_string(), ActionSpec::for_procedure("walk"));
        clonk.configure_actions(Some("Idle".to_string()), actions);
        clonk.set_movement_profile(MovementProfile::default());
        let gun = Definition::from_script("GUNX", "Gun", gun_script).expect("gun script compiles");

        let mut engine = Engine::new();
        engine.register_definition(clonk)?;
        engine.register_definition(gun)?;
        engine.register_player(PlayerConfig::new(1, "Test"))?;

        let crew = engine.spawn_object(
            SpawnConfig::new("TRPR")
                .with_owner(1)
                .with_crew_member(true)
                .with_action(ActionState::new("Idle")),
        )?;
        let gun_id = engine.spawn_object(SpawnConfig::new("GUNX").with_owner(1))?;
        engine.select_crew(1, vec![crew])?;
        engine.set_crew_cursor(1, Some(crew))?;
        assert!(engine.execute_context_menu(gun_id, "Arm")?);

        let mut dispatcher = InputDispatcher::new();
        let events: &[(u64, ControlEvent)] = &[
            (
                5,
                ControlEvent::Command {
                    command: ControlCommand::Dig,
                    kind: CommandKind::Press,
                },
            ),
            (
                10,
                ControlEvent::Command {
                    command: ControlCommand::Dig,
                    kind: CommandKind::Release,
                },
            ),
            (
                15,
                ControlEvent::Command {
                    command: ControlCommand::Throw,
                    kind: CommandKind::Press,
                },
            ),
            (20, ControlEvent::Press(ControlButton::Up)),
            (25, ControlEvent::Release(ControlButton::Up)),
        ];

        for tick in 0..50u64 {
            for (when, event) in events {
                if *when == tick {
                    dispatcher.handle_event(&mut engine, 1, *event)?;
                }
            }
            engine.tick()?;
        }

        let snapshot = engine.object_snapshot(crew).expect("crew snapshot");
        let effect = snapshot
            .effects
            .iter()
            .find(|effect| effect.name == "GunControl")
            .expect("control effect still installed");
        assert_eq!(
            effect.vars.first(),
            Some(&clonk_engine::EffectVarValue::Int(3)),
            "dig press/release and throw all reached the Fx callbacks"
        );
        Ok(())
    }

    #[test]
    fn fantasy_style_clonk_walks_jumps_digs_and_casts_over_50_ticks() -> Result<(), EngineError> {
        // Fantasy MAGE-style crew (MAGE→SCLK→MCLK→CLNK): the directional
        // Control* overrides return 0 (Clonk.c4d/Script.c:62-105) so the
        // CLASSIC per-procedure fallbacks must move, jump and dig the mage
        // (C4Object.cpp:3406-3424), while ControlSpecial stays a script
        // matter (MagiClonk.c4d/Script.c:89-114). Runs without script
        // errors and with the C++-expected actions firing.
        let mage_script = r#"
#strict
protected func ControlLeft() { return(0); }
protected func ControlRight() { return(0); }
protected func ControlUp() { return(0); }
protected func ControlUpReleased() { return(0); }
protected func ControlDown() { return(0); }
protected func ControlDig() { return(0); }
protected func ControlThrow() { return(0); }
protected func ControlSpecial() { Sound("Magic1"); return(1); }
"#;
        let mut mage =
            Definition::from_script("MAGE", "Mage", mage_script).expect("mage script compiles");
        let mut actions = HashMap::new();
        actions.insert("Walk".to_string(), ActionSpec::for_procedure("walk"));
        actions.insert("Jump".to_string(), ActionSpec::for_procedure("flight"));
        actions.insert("Dig".to_string(), ActionSpec::for_procedure("dig"));
        mage.configure_actions(Some("Walk".to_string()), actions);
        mage.set_movement_profile(MovementProfile::default());
        let physical = PhysicalInfo {
            walk: 70_000,
            jump: 40_000,
            dig: 40_000,
            can_dig: 1,
            ..Default::default()
        };
        mage.set_physical(physical);

        let mut engine = Engine::new();
        engine.register_definition(mage)?;
        engine.register_player(PlayerConfig::new(1, "Test"))?;
        let crew = engine.spawn_object(
            SpawnConfig::new("MAGE")
                .with_owner(1)
                .with_crew_member(true)
                .with_action(ActionState::new("Walk")),
        )?;
        engine.select_crew(1, vec![crew])?;
        engine.set_crew_cursor(1, Some(crew))?;

        let mut dispatcher = InputDispatcher::new();
        let mut saw_walk_right = false;
        let mut saw_jump = false;
        let mut saw_dig = false;

        for tick in 0..50u64 {
            match tick {
                2 => {
                    dispatcher.handle_event(
                        &mut engine,
                        1,
                        ControlEvent::Press(ControlButton::Right),
                    )?;
                }
                6 => {
                    dispatcher.handle_event(
                        &mut engine,
                        1,
                        ControlEvent::Release(ControlButton::Right),
                    )?;
                }
                8 => {
                    // Jump: Up in DFA_WALK (C4Object.cpp:3414).
                    dispatcher.handle_event(
                        &mut engine,
                        1,
                        ControlEvent::Press(ControlButton::Up),
                    )?;
                }
                9 => {
                    dispatcher.handle_event(
                        &mut engine,
                        1,
                        ControlEvent::Release(ControlButton::Up),
                    )?;
                }
                24 => {
                    // Land again, then dig via the COM_Dig_S timeout
                    // (C4Player.cpp:1215-1229 → C4Object.cpp:3416-3421).
                    dispatcher.handle_event(
                        &mut engine,
                        1,
                        ControlEvent::Command {
                            command: ControlCommand::Dig,
                            kind: CommandKind::Press,
                        },
                    )?;
                }
                40 => {
                    // Spell key: the MCLK ControlSpecial override consumes
                    // the com (C4Object.cpp:3385-3389).
                    dispatcher.handle_event(
                        &mut engine,
                        1,
                        ControlEvent::Command {
                            command: ControlCommand::Special,
                            kind: CommandKind::Press,
                        },
                    )?;
                }
                _ => {}
            }
            engine.tick()?;
            let snapshot = engine.object_snapshot(crew).expect("crew snapshot");
            match snapshot.action.name.as_str() {
                "Walk" if snapshot.command_direction == CommandDirection::Right => {
                    saw_walk_right = true;
                }
                "Jump" => saw_jump = true,
                "Dig" => saw_dig = true,
                _ => {}
            }
            if tick == 22 {
                // Reset to walking so the dig fallback applies in DFA_WALK.
                engine.apply_object_update(crew, ObjectUpdate::new().with_action("Walk"))?;
            }
        }

        assert!(saw_walk_right, "COM_Right walked the mage right");
        assert!(saw_jump, "COM_Up jumped via the classic fallback");
        assert!(saw_dig, "COM_Dig dug via the COM_Dig_S timeout");
        Ok(())
    }
}
