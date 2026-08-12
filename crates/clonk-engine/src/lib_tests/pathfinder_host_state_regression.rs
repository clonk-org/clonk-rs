use super::*;
use crate::landscape::PixelGrid;

fn pixel_landscape(width: u32, height: u32, pixels: Vec<u8>) -> Landscape {
    let mut landscape =
        Landscape::with_default_material(width, vec![height as i32; width as usize], None)
            .expect("pathfinder landscape");
    landscape.set_world_height(height as i32);
    landscape.set_pixel_grid(PixelGrid::new(
        width,
        height,
        pixels,
        vec![0, 100],
        vec![None, Some("Earth".to_owned())],
        vec![None; 2],
    ));
    landscape
}

fn script_get_path(engine: &mut Engine, from: Vector2, to: Vector2) -> Value {
    engine
        .call_engine_global_function(
            "GetPath",
            &[
                Value::Int(from.x),
                Value::Int(from.y),
                Value::Int(to.x),
                Value::Int(to.y),
            ],
        )
        .expect("GetPath host call succeeds")
}

fn unpack_path(value: &Value) -> (i32, Vec<(i32, i32, Option<u64>)>) {
    let Value::Proplist(path) = value else {
        panic!("expected GetPath proplist, got {value:?}");
    };
    let length = match path.get("Length") {
        Some(Value::Int(length)) => *length,
        other => panic!("expected integer Length, got {other:?}"),
    };
    let waypoints = match path.get("Waypoints") {
        Some(Value::Array(waypoints)) => waypoints,
        other => panic!("expected Waypoints array, got {other:?}"),
    };
    let points = waypoints
        .iter()
        .map(|waypoint| {
            let Value::Proplist(waypoint) = waypoint else {
                panic!("expected waypoint proplist, got {waypoint:?}");
            };
            let x = match waypoint.get("X") {
                Some(Value::Int(x)) => *x,
                other => panic!("expected waypoint X, got {other:?}"),
            };
            let y = match waypoint.get("Y") {
                Some(Value::Int(y)) => *y,
                other => panic!("expected waypoint Y, got {other:?}"),
            };
            let transfer_target = match waypoint.get("TransferTarget") {
                Some(Value::Object(id)) => Some(*id),
                Some(other) => panic!("expected object TransferTarget, got {other:?}"),
                None => None,
            };
            (x, y, transfer_target)
        })
        .collect();
    (length, points)
}

fn run_obstructed_move_to(engine: &mut Engine, object_id: ObjectId, target: Vector2) {
    let index = engine
        .find_object_index(object_id)
        .expect("pathfinder object exists");
    engine.objects[index]
        .commands
        .push_front(
            CommandRequest::new(CommandId::MoveTo)
                .with_tx(Some(target.x))
                .with_ty(Some(target.y))
                // C4CMD_MoveTo_NoPosAdjust keeps the test coordinate
                // fixed across the command's evaluation-only Execute.
                .with_data(CommandData::Integer(1)),
        )
        .expect("MoveTo queues");
    engine
        .execute_object_command_now(object_id)
        .expect("MoveTo evaluates");
    engine
        .execute_object_command_now(object_id)
        .expect("MoveTo pathfind executes");
}

#[test]
fn get_path_reuses_last_move_to_pathfinder_level_like_cpp() {
    // Both exits require more than the level-1 MAX_CRAWL=800. Level 5
    // reaches one with only six rays, isolating the persistent Level
    // knob from the fixed MAX_RAY cap (C4PathFinder.cpp:213-217).
    const WIDTH: u32 = 100;
    const HEIGHT: u32 = 2_000;
    let mut pixels = vec![0; WIDTH as usize * HEIGHT as usize];
    for y in 100..=1_900usize {
        for x in 49..=50usize {
            pixels[y * WIDTH as usize + x] = 1;
        }
    }
    let from = Vector2::new(10, 1_000);
    let to = Vector2::new(90, 1_000);
    let mut engine = Engine::with_seed(1);
    engine.set_landscape(pixel_landscape(WIDTH, HEIGHT, pixels));

    let mut definition =
        Definition::from_script("PF05", "Level five mover", "").expect("definition compiles");
    definition.set_pathfinder(5);
    engine
        .register_definition(definition)
        .expect("definition registers");
    let mover = engine
        .spawn_object(SpawnConfig::new("PF05").with_position(from))
        .expect("mover spawns");

    assert!(engine.find_path(from, to, 1, true).is_none());
    assert!(engine.find_path(from, to, 5, true).is_some());
    assert_eq!(
        script_get_path(&mut engine, from, to),
        Value::Nil,
        "a fresh game uses C4PathFinder's default level 1"
    );

    run_obstructed_move_to(&mut engine, mover, to);

    let path = script_get_path(&mut engine, from, to);
    let (length, points) = unpack_path(&path);
    assert!(length > 80, "the route must detour around the tall wall");
    assert_eq!(points.first(), Some(&(from.x, from.y, None)));
    assert_eq!(points.last(), Some(&(to.x, to.y, None)));
    assert!(points.len() > 2);
    assert!(
        points.iter().any(|(_, y, _)| *y < 100 || *y > 1_900),
        "the level-5 route must reach a wall exit"
    );
}

#[test]
fn get_path_reuses_last_move_to_transfer_zone_toggle_like_cpp() {
    const WIDTH: u32 = 100;
    const HEIGHT: u32 = 100;
    let mut pixels = vec![0; WIDTH as usize * HEIGHT as usize];
    for y in 0..HEIGHT as usize {
        pixels[y * WIDTH as usize + 49] = 1;
        pixels[y * WIDTH as usize + 50] = 1;
    }
    let from = Vector2::new(10, 50);
    let to = Vector2::new(90, 50);
    let mut engine = Engine::with_seed(1);
    engine.set_landscape(pixel_landscape(WIDTH, HEIGHT, pixels));

    let mut enabled =
        Definition::from_script("PFTZ", "Zone mover", "").expect("definition compiles");
    enabled.set_pathfinder(1);
    engine
        .register_definition(enabled)
        .expect("enabled definition registers");
    let mut disabled =
        Definition::from_script("PFNZ", "No-zone mover", "").expect("definition compiles");
    disabled.set_pathfinder(1);
    disabled.set_no_transfer_zones(1);
    engine
        .register_definition(disabled)
        .expect("disabled definition registers");
    let zone_owner = engine
        .spawn_object(SpawnConfig::new("PFTZ").with_position(from))
        .expect("zone owner spawns");
    let no_zone_mover = engine
        .spawn_object(SpawnConfig::new("PFNZ").with_position(from))
        .expect("no-zone mover spawns");
    engine
        .set_transfer_zone(
            zone_owner,
            TransferZoneRect {
                x: 45,
                y: 40,
                width: 10,
                height: 20,
            },
        )
        .expect("transfer zone registers");

    assert!(engine.find_path(from, to, 1, false).is_none());
    assert!(engine.find_path(from, to, 1, true).is_some());
    let fresh = script_get_path(&mut engine, from, to);
    let (_, fresh_points) = unpack_path(&fresh);
    assert_eq!(fresh_points.first(), Some(&(from.x, from.y, None)));
    assert_eq!(fresh_points.last(), Some(&(to.x, to.y, None)));
    assert_eq!(
        fresh_points
            .iter()
            .filter(|(_, _, target)| *target == Some(zone_owner.as_u64()))
            .count(),
        1,
        "fresh C4PathFinder state has transfer zones enabled"
    );

    run_obstructed_move_to(&mut engine, no_zone_mover, to);
    assert_eq!(
        script_get_path(&mut engine, from, to),
        Value::Nil,
        "NoTransferZones persists after a failed MoveTo pathfind"
    );

    run_obstructed_move_to(&mut engine, zone_owner, to);
    assert_eq!(
        script_get_path(&mut engine, from, to),
        fresh,
        "the next MoveTo pathfind re-enables zones for GetPath"
    );
}

#[test]
fn execute_command_updates_get_path_settings_within_the_same_script_call() {
    // FnExecuteCommand runs C4Command::MoveTo synchronously. The
    // following GetPath therefore sees the disabled global-zone knob
    // before the outer script call returns and its copied host state is
    // folded back into Engine (C4Script.cpp:922-929,5040).
    const WIDTH: u32 = 100;
    const HEIGHT: u32 = 100;
    let mut pixels = vec![0; WIDTH as usize * HEIGHT as usize];
    for y in 0..HEIGHT as usize {
        pixels[y * WIDTH as usize + 49] = 1;
        pixels[y * WIDTH as usize + 50] = 1;
    }
    let from = Vector2::new(10, 50);
    let to = Vector2::new(90, 50);
    let mut engine = Engine::with_seed(1);
    engine.set_landscape(pixel_landscape(WIDTH, HEIGHT, pixels));

    let mut definition = Definition::from_script(
        "PFSY",
        "Synchronous no-zone mover",
        r#"
                #strict 2
                func Probe()
                {
                    SetCommand(this(), "MoveTo", 0, 90, 50, 0, 1);
                    ExecuteCommand();
                    ExecuteCommand();
                    return GetPath(11, 49, 89, 51);
                }
            "#,
    )
    .expect("definition compiles");
    definition.set_pathfinder(1);
    definition.set_no_transfer_zones(1);
    engine
        .register_definition(definition)
        .expect("definition registers");
    let mover = engine
        .spawn_object(SpawnConfig::new("PFSY").with_position(from))
        .expect("mover spawns");
    engine
        .set_transfer_zone(
            mover,
            TransferZoneRect {
                x: 45,
                y: 40,
                width: 10,
                height: 20,
            },
        )
        .expect("transfer zone registers");
    assert!(engine.find_path(from, to, 1, true).is_some());
    assert!(engine.find_path(from, to, 1, false).is_none());

    let index = engine.find_object_index(mover).expect("mover exists");
    let value = engine
        .call_object_function(index, "Probe", Vec::new())
        .expect("Probe executes");

    assert_eq!(
        value,
        Value::Nil,
        "same-call GetPath observes ExecuteCommand's disabled-zone write"
    );
    let graph = engine.snapshot().pathfinder_debug;
    assert_eq!(graph.rays[0].start, Vector2::new(11, 49));
    assert_eq!(graph.rays[0].target, Vector2::new(89, 51));
    assert_eq!(script_get_path(&mut engine, from, to), Value::Nil);
}

#[test]
fn transfer_direct_callback_runs_on_status_zero_and_keeps_replacement_command() {
    let mut engine = Engine::with_seed(1);
    engine
        .register_definition(
            Definition::from_script(
                "GATE",
                "Transfer gate",
                r#"
                        #strict 2
                        protected func ControlTransfer(object actor, tx, int ty)
                        {
                            SetR(73, actor);
                            SetCommand(actor, "Transfer", this(), 777, 9);
                            return false;
                        }
                    "#,
            )
            .expect("gate definition compiles"),
        )
        .expect("gate definition registers");
    engine
        .register_script_definition("ACTR", "Transfer actor", "")
        .expect("actor definition registers");

    let gate = engine
        .spawn_object(SpawnConfig::new("GATE").with_position(Vector2::new(100, 0)))
        .expect("gate spawns");
    let actor = engine
        .spawn_object(SpawnConfig::new("ACTR").with_position(Vector2::new(95, 0)))
        .expect("actor spawns");
    engine
        .set_transfer_zone(
            gate,
            TransferZoneRect {
                x: 90,
                y: -10,
                width: 20,
                height: 40,
            },
        )
        .expect("transfer zone registers");
    let actor_index = engine.find_object_index(actor).expect("actor exists");
    engine.objects[actor_index]
        .commands
        .push_front(
            CommandRequest::new(CommandId::Transfer)
                .with_target(Some(gate))
                .with_tx_value(Value::C4Id("GOLD".to_string()))
                .with_ty(Some(-5)),
        )
        .expect("Transfer queues");

    let gate_index = engine.find_object_index(gate).expect("gate exists");
    let _ = engine.objects[gate_index].mark_destroyed();
    assert_eq!(
        engine.objects[gate_index].state.status,
        ObjectStatus::Deleted
    );
    assert!(
        engine.transfer_zones.get(gate).is_some(),
        "the synthetic tombstone retains its zone for the direct-call seam"
    );

    engine
        .execute_object_command_now(actor)
        .expect("Transfer executes");

    let actor_index = engine.find_object_index(actor).expect("actor remains");
    assert_eq!(
        engine.objects[actor_index].state.rotation, 73,
        "the cached SFn executes even though C4Object::Call would reject Status zero"
    );
    let commands = engine.objects[actor_index].commands.command_views();
    let [replacement] = commands.as_slice() else {
        panic!("replacement Transfer must remain: {commands:?}");
    };
    assert_eq!(replacement.name, "Transfer");
    assert_eq!(replacement.target, Some(gate));
    assert_eq!(replacement.tx, Some(777));
    assert_eq!(replacement.tx_value, Some(Value::Int(777)));
    assert_eq!(replacement.ty, Some(9));
    assert!(
        !replacement.finished,
        "false resolves only the detached emitting instance"
    );
}

#[test]
fn restored_zero_token_events_pin_transfer_before_callback_replacement() {
    let mut engine = Engine::with_seed(1);
    engine
        .register_definition(
            Definition::from_script(
                "ZTRG",
                "Restored transfer gate",
                r#"
                        #strict 2
                        local answer;
                        protected func ControlTransfer(object actor, tx, int ty)
                        {
                            SetCommand(actor, "Transfer", this(), 777, 9);
                            return answer;
                        }
                    "#,
            )
            .expect("gate definition compiles"),
        )
        .expect("gate definition registers");
    engine
        .register_script_definition("ZTRA", "Restored transfer actor", "")
        .expect("actor definition registers");

    let gate = engine
        .spawn_object(SpawnConfig::new("ZTRG"))
        .expect("gate spawns");
    let actor = engine
        .spawn_object(SpawnConfig::new("ZTRA"))
        .expect("actor spawns");
    let actor_index = engine.find_object_index(actor).expect("actor exists");
    engine.objects[actor_index]
        .commands
        .push_front(CommandRequest::new(CommandId::Transfer).with_target(Some(gate)))
        .expect("original Transfer queues");

    let queued = QueuedCommand::immediate(ObjectUpdate::default()).with_events(vec![
        CommandEvent::ControlTransfer {
            object_id: gate,
            caller: actor,
            tx_value: Value::Nil,
            ty: 0,
            command_instance_id: u64::MAX,
        },
    ]);
    let encoded = serde_json::to_value(queued).expect("queued event serializes");
    let restored: QueuedCommand = serde_json::from_value(encoded).expect("queued event restores");
    let [event] = restored.events.as_slice() else {
        panic!("one restored event expected");
    };
    assert!(matches!(
        event,
        CommandEvent::ControlTransfer {
            command_instance_id: 0,
            ..
        }
    ));

    engine
        .apply_command_event(event.clone())
        .expect("restored direct callback executes");

    let actor_index = engine.find_object_index(actor).expect("actor remains");
    let commands = engine.objects[actor_index].commands.command_views();
    let [replacement] = commands.as_slice() else {
        panic!("callback replacement Transfer must remain: {commands:?}");
    };
    assert_eq!(replacement.target, Some(gate));
    assert_eq!(replacement.tx, Some(777));
    assert_eq!(replacement.ty, Some(9));
    assert!(
        !replacement.finished,
        "the restored event resolves the original instance before its callback"
    );

    let actor_index = engine.find_object_index(actor).expect("actor remains");
    assert_eq!(
        engine.objects[actor_index]
            .commands
            .take_successful_finishes(),
        vec![CommandId::Transfer]
    );
    engine.objects[actor_index].commands.clear();
    engine.objects[actor_index]
        .commands
        .push_front(CommandRequest::new(CommandId::Transfer).with_target(Some(gate)))
        .expect("legacy original Transfer queues");
    let gate_index = engine.find_object_index(gate).expect("gate remains");
    let raw = 1usize.checked_shl(32).unwrap_or(0);
    engine.objects[gate_index]
        .state
        .local_vars
        .insert("answer".to_string(), Value::RawBool(raw));
    let _ = engine.objects[gate_index].mark_destroyed();
    let legacy = QueuedCommand::immediate(ObjectUpdate::default()).with_events(vec![
        CommandEvent::CallObjectFunction {
            object_id: gate,
            function: "ControlTransfer".to_string(),
            caller: actor,
            tx: None,
            tx_value: Some(Value::Nil),
            tx_definition: None,
            ty: Some(0),
            target2: None,
            on_result: Some(CallResultAction::CompleteCommandOnFalse {
                command: CommandId::Transfer,
            }),
        },
    ]);
    let encoded = serde_json::to_value(legacy).expect("legacy event serializes");
    let restored: QueuedCommand = serde_json::from_value(encoded).expect("legacy event restores");
    let [legacy_event] = restored.events.as_slice() else {
        panic!("one restored legacy event expected");
    };
    engine
        .apply_command_event(legacy_event.clone())
        .expect("legacy callback event executes");

    let actor_index = engine.find_object_index(actor).expect("actor remains");
    let commands = engine.objects[actor_index].commands.command_views();
    let [replacement] = commands.as_slice() else {
        panic!("legacy callback replacement Transfer must remain: {commands:?}");
    };
    assert_eq!(replacement.tx, Some(777));
    assert_eq!(replacement.ty, Some(9));
    assert_eq!(Value::RawBool(raw).c4_bool_raw(), Some(0));
    assert_eq!(
        engine.objects[gate_index].state.status,
        ObjectStatus::Deleted,
        "the restored direct callback bypasses the ordinary object-call Status gate"
    );
    assert!(
        !replacement.finished,
        "legacy CallObjectFunction binds its completion target before the callback"
    );
    assert_eq!(
        engine.objects[actor_index]
            .commands
            .take_successful_finishes(),
        vec![CommandId::Transfer],
        "the restored callback consumes native's low bool word"
    );
}

#[test]
fn transfer_direct_callback_uses_the_c4_bool_low_word() {
    let mut engine = Engine::with_seed(1);
    engine
        .register_definition(
            Definition::from_script(
                "GBOL",
                "Raw-bool transfer gate",
                r#"
                        #strict 2
                        local answer;
                        protected func ControlTransfer(object actor, tx, int ty)
                        {
                            return answer;
                        }
                    "#,
            )
            .expect("gate definition compiles"),
        )
        .expect("gate definition registers");
    engine
        .register_script_definition("ABOL", "Raw-bool actor", "")
        .expect("actor definition registers");

    // C4VBool stores a machine word but C4Value::getBool reads its signed
    // 32-bit payload. On a 64-bit host this value is truthy to Rust's
    // generic Value::as_bool while its native C4 bool word is zero.
    let raw = 1usize.checked_shl(32).unwrap_or(0);
    let gate = engine
        .spawn_object(
            SpawnConfig::new("GBOL")
                .with_local_vars(HashMap::from([("answer".to_string(), Value::RawBool(raw))])),
        )
        .expect("gate spawns");
    let actor = engine
        .spawn_object(SpawnConfig::new("ABOL"))
        .expect("actor spawns");
    let gate_index = engine.find_object_index(gate).expect("gate exists");

    assert_eq!(Value::RawBool(raw).c4_bool_raw(), Some(0));
    assert!(
        !engine
            .call_control_transfer(gate_index, actor, Value::Nil, 0)
            .expect("direct callback executes"),
        "ControlTransfer uses C4Value::getBool rather than generic truthiness"
    );
}

#[test]
fn activate_entrance_uses_full_c4_value_truthiness() {
    let mut engine = Engine::with_seed(1);
    engine
        .register_definition(
            Definition::from_script(
                "EBOL",
                "Raw-bool entrance",
                r#"
                        #strict 2
                        local answer;
                        protected func ActivateEntrance(object actor)
                        {
                            return answer;
                        }
                    "#,
            )
            .expect("entrance definition compiles"),
        )
        .expect("entrance definition registers");
    engine
        .register_script_definition("ACBE", "Entrance caller", "")
        .expect("caller definition registers");

    let raw = 1usize.checked_shl(32).unwrap_or(0);
    let entrance = engine
        .spawn_object(
            SpawnConfig::new("EBOL")
                .with_local_vars(HashMap::from([("answer".to_string(), Value::RawBool(raw))])),
        )
        .expect("entrance spawns");
    let caller = engine
        .spawn_object(SpawnConfig::new("ACBE"))
        .expect("caller spawns");
    let entrance_index = engine.find_object_index(entrance).expect("entrance exists");
    engine.objects[entrance_index].state.ocf |= ocf::ENTRANCE;

    assert_eq!(Value::RawBool(raw).c4_bool_raw(), Some(0));
    assert_eq!(
        engine
            .activate_object_entrance(entrance, caller)
            .expect("entrance callback executes"),
        raw != 0,
        "C4Object::ActivateEntrance uses C4Value::operator bool, not getBool"
    );
}

#[test]
fn restored_legacy_entrance_result_does_not_fail_callback_replacement() {
    let mut engine = Engine::with_seed(1);
    engine
        .register_definition(
            Definition::from_script(
                "ZENT",
                "Restored entrance",
                r#"
                        #strict 2
                        protected func ActivateEntrance(object actor)
                        {
                            SetCommand(actor, "Exit");
                            return false;
                        }
                    "#,
            )
            .expect("entrance definition compiles"),
        )
        .expect("entrance definition registers");
    engine
        .register_script_definition("ZENA", "Restored entrance caller", "")
        .expect("caller definition registers");

    let entrance = engine
        .spawn_object(SpawnConfig::new("ZENT"))
        .expect("entrance spawns");
    let caller = engine
        .spawn_object(SpawnConfig::new("ZENA"))
        .expect("caller spawns");
    let entrance_index = engine.find_object_index(entrance).expect("entrance exists");
    engine.objects[entrance_index].state.ocf |= ocf::ENTRANCE;
    let caller_index = engine.find_object_index(caller).expect("caller exists");
    engine.objects[caller_index]
        .commands
        .push_front(CommandRequest::new(CommandId::Exit))
        .expect("original Exit queues");

    let queued = QueuedCommand::immediate(ObjectUpdate::default()).with_events(vec![
        CommandEvent::ActivateEntrance {
            object_id: entrance,
            caller,
            on_result: Some(CallResultAction::FailCommandOnFalse {
                command: CommandId::Exit,
            }),
            command_instance_id: u64::MAX,
        },
    ]);
    let encoded = serde_json::to_value(queued).expect("legacy entrance event serializes");
    let restored: QueuedCommand =
        serde_json::from_value(encoded).expect("legacy entrance event restores");
    let [event] = restored.events.as_slice() else {
        panic!("one restored entrance event expected");
    };
    engine
        .apply_command_event(event.clone())
        .expect("legacy entrance event executes");

    let caller_index = engine.find_object_index(caller).expect("caller remains");
    let commands = engine.objects[caller_index].commands.legacy_save_commands();
    let [replacement] = commands.as_slice() else {
        panic!("callback replacement Exit must remain: {commands:?}");
    };
    assert_eq!(replacement.view.name, "Exit");
    assert_eq!(
        replacement.failures, 0,
        "the false result fails only the detached original Exit"
    );
}

#[test]
fn script_execute_command_runs_direct_transfer_before_returning() {
    let mut engine = Engine::with_seed(1);
    engine
        .register_definition(
            Definition::from_script(
                "GAT2",
                "Synchronous transfer gate",
                r#"
                        #strict 2
                        local seen_tx, seen_ty;
                        protected func ControlTransfer(object actor, tx, int ty)
                        {
                            seen_tx = tx;
                            seen_ty = ty;
                            SetR(73, actor);
                            SetCommand(actor, "Transfer", this(), 777, 9);
                            return false;
                        }
                    "#,
            )
            .expect("gate definition compiles"),
        )
        .expect("gate definition registers");
    engine
        .register_definition(
            Definition::from_script(
                "ACR2",
                "Synchronous transfer actor",
                r#"
                        #strict 2
                        public func Probe(object gate)
                        {
                            SetCommand(this(), "Transfer", gate, GOLD, -5);
                            ExecuteCommand();
                            return [GetR(), GetCommand(), GetCommand(0, 2), GetCommand(0, 3)];
                        }
                    "#,
            )
            .expect("actor definition compiles"),
        )
        .expect("actor definition registers");

    let gate = engine
        .spawn_object(SpawnConfig::new("GAT2").with_position(Vector2::new(100, 0)))
        .expect("gate spawns");
    let actor = engine
        .spawn_object(SpawnConfig::new("ACR2").with_position(Vector2::new(95, 0)))
        .expect("actor spawns");
    engine
        .set_transfer_zone(
            gate,
            TransferZoneRect {
                x: 90,
                y: -10,
                width: 20,
                height: 40,
            },
        )
        .expect("transfer zone registers");
    let gate_index = engine.find_object_index(gate).expect("gate exists");
    let _ = engine.objects[gate_index].mark_destroyed();

    let actor_index = engine.find_object_index(actor).expect("actor exists");
    let result = engine
        .call_object_function(actor_index, "Probe", vec![object_reference_value(gate)])
        .expect("Probe executes");
    assert_eq!(
        result,
        Value::Array(vec![
            Value::Int(73),
            Value::String("Transfer".to_string().into()),
            Value::Int(777),
            Value::Int(9),
        ]),
        "the next VM instruction observes the direct callback and its replacement command"
    );
    let gate_index = engine.find_object_index(gate).expect("gate remains");
    assert_eq!(
        engine.objects[gate_index].state.local_vars.get("seen_tx"),
        Some(&Value::C4Id("GOLD".to_string())),
        "the direct callback receives the exact tagged Tx"
    );
    assert_eq!(
        engine.objects[gate_index].state.local_vars.get("seen_ty"),
        Some(&Value::Int(-5))
    );
    let actor_index = engine.find_object_index(actor).expect("actor remains");
    let commands = engine.objects[actor_index].commands.command_views();
    let [replacement] = commands.as_slice() else {
        panic!("replacement Transfer must remain: {commands:?}");
    };
    assert_eq!(replacement.target, Some(gate));
    assert_eq!(replacement.tx_value, Some(Value::Int(777)));
    assert_eq!(replacement.ty, Some(9));
    assert!(!replacement.finished);
}

#[test]
fn exit_command_runs_live_callbacks_before_finishing_in_both_execution_paths() {
    fn setup() -> (Engine, ObjectId, ObjectId) {
        let mut engine = Engine::with_seed(1);
        let mut container = Definition::from_script(
            "XCTR",
            "Exit callback container",
            r#"
                    #strict 2
                    protected func Ejection(object item)
                    {
                        item->NoteExit(1);
                        return true;
                    }
                "#,
        )
        .expect("container definition compiles");
        container.set_c4_callback_convention(true);
        engine
            .register_definition(container)
            .expect("container definition registers");
        let mut actor = Definition::from_script(
            "XACT",
            "Exit callback actor",
            r#"
                    #strict 2
                    local exit_order, after_exit_order, after_exit_command, after_exit_rotation;
                    public func NoteExit(int step)
                    {
                        exit_order = exit_order * 10 + step;
                        return true;
                    }
                    protected func Departure(object old_container)
                    {
                        NoteExit(2);
                        SetCommand(this(), "Wait", 0, 17);
                        return true;
                    }
                    public func Probe()
                    {
                        ExecuteCommand();
                        after_exit_order = exit_order;
                        after_exit_command = GetCommand();
                        after_exit_rotation = GetR();
                        return true;
                    }
                "#,
        )
        .expect("actor definition compiles");
        actor.set_c4_callback_convention(true);
        engine
            .register_definition(actor)
            .expect("actor definition registers");

        let container = engine
            .spawn_object(SpawnConfig::new("XCTR").with_position(Vector2::new(80, 90)))
            .expect("container spawns");
        let actor = engine
            .spawn_object(SpawnConfig::new("XACT").with_container(container))
            .expect("actor spawns");
        let container_index = engine
            .find_object_index(container)
            .expect("container exists");
        engine.objects[container_index].state.entrance_status = true;
        engine
            .apply_object_update(
                actor,
                ObjectUpdate::new()
                    .with_position(Vector2::new(7, 9))
                    .with_rotation(45)
                    .with_velocity(Vector2::new(6, -2))
                    .with_command_direction(CommandDirection::Right),
            )
            .expect("contained actor is repositioned");
        let actor_index = engine.find_object_index(actor).expect("actor exists");
        engine.objects[actor_index]
            .commands
            .push_front(CommandRequest::new(CommandId::Exit).with_evaluated(true))
            .expect("Exit queues");
        (engine, actor, container)
    }

    fn assert_exit_result(engine: &Engine, actor: ObjectId, container: ObjectId) {
        let actor_index = engine.find_object_index(actor).expect("actor remains");
        let state = &engine.objects[actor_index].state;
        assert_eq!(state.container, None);
        assert_eq!(state.position, Vector2::new(7, 9));
        assert_eq!(state.rotation, 0, "C4Object::Exit uses its default iR=0");
        assert_eq!(engine.objects[actor_index].fixed_velocity, FixedVec2::ZERO);
        assert_eq!(state.command_direction, CommandDirection::Right);
        assert_eq!(state.local_vars.get("exit_order"), Some(&Value::Int(12)));
        assert!(!engine.objects[engine
            .find_object_index(container)
            .expect("container remains")]
        .state
        .contents
        .contains(&actor));
        let commands = engine.objects[actor_index].commands.command_views();
        let [replacement] = commands.as_slice() else {
            panic!("Departure replacement Wait must remain: {commands:?}");
        };
        assert_eq!(replacement.name, "Wait");
        assert_eq!(replacement.tx, Some(17));
        assert!(!replacement.finished);
    }

    let (mut engine, actor, container) = setup();
    engine
        .execute_object_command_now(actor)
        .expect("engine Exit executes");
    assert_exit_result(&engine, actor, container);

    let (mut engine, actor, container) = setup();
    let actor_index = engine.find_object_index(actor).expect("actor exists");
    engine
        .call_object_function(actor_index, "Probe", Vec::new())
        .expect("script ExecuteCommand returns");
    assert_exit_result(&engine, actor, container);
    let actor_index = engine.find_object_index(actor).expect("actor remains");
    let locals = &engine.objects[actor_index].state.local_vars;
    assert_eq!(locals.get("after_exit_order"), Some(&Value::Int(12)));
    assert_eq!(
        locals.get("after_exit_command"),
        Some(&Value::String("Wait".to_string().into()))
    );
    assert_eq!(locals.get("after_exit_rotation"), Some(&Value::Int(0)));
}

#[test]
fn script_removal_clears_transfer_zone_before_same_frame_pathfind_and_transfer() {
    // AssignRemoval reaches Game.ClearPointers synchronously. A GetPath
    // later in the same VM call, then native commands later in the same
    // engine frame, must all observe the zone's immediate deletion
    // (C4Object.cpp:300-313; C4TransferZone.cpp:68-76).
    const WIDTH: u32 = 100;
    const HEIGHT: u32 = 100;
    let mut pixels = vec![0; WIDTH as usize * HEIGHT as usize];
    for y in 0..HEIGHT as usize {
        pixels[y * WIDTH as usize + 49] = 1;
        pixels[y * WIDTH as usize + 50] = 1;
    }
    let from = Vector2::new(10, 50);
    let to = Vector2::new(90, 50);
    let mut engine = Engine::with_seed(1);
    engine.set_landscape(pixel_landscape(WIDTH, HEIGHT, pixels));

    engine
        .register_script_definition("GATE", "Transfer gate", "")
        .expect("gate definition registers");
    let mut mover_definition = Definition::from_script(
        "PFMR",
        "Pathfinder mover",
        r#"
                #strict 2
                func RemoveGate(gate)
                {
                    RemoveObject(gate);
                    return GetPath(10, 50, 90, 50);
                }
            "#,
    )
    .expect("mover definition compiles");
    mover_definition.set_pathfinder(1);
    engine
        .register_definition(mover_definition)
        .expect("mover definition registers");

    let gate = engine
        .spawn_object(SpawnConfig::new("GATE").with_position(Vector2::new(50, 50)))
        .expect("gate spawns");
    let path_actor = engine
        .spawn_object(SpawnConfig::new("PFMR").with_position(from))
        .expect("path actor spawns");
    let transfer_actor = engine
        .spawn_object(SpawnConfig::new("PFMR").with_position(Vector2::new(45, 50)))
        .expect("transfer actor spawns");
    engine
        .set_transfer_zone(
            gate,
            TransferZoneRect {
                x: 45,
                y: 40,
                width: 10,
                height: 20,
            },
        )
        .expect("transfer zone registers");
    assert!(
        engine.find_path(from, to, 1, true).is_some(),
        "the live gate is the only route through the full-height wall"
    );
    run_obstructed_move_to(&mut engine, path_actor, to);
    let path_index = engine
        .find_object_index(path_actor)
        .expect("path actor exists");
    assert!(
        engine.objects[path_index]
            .commands
            .snapshot()
            .command_names()
            .iter()
            .any(|name| name == "Transfer"),
        "the live-zone control must generate a Transfer leg"
    );
    engine.objects[path_index].commands.clear();

    let transfer_index = engine
        .find_object_index(transfer_actor)
        .expect("transfer actor exists");
    engine.objects[transfer_index]
        .commands
        .push_front(
            CommandRequest::new(CommandId::Transfer)
                .with_target(Some(gate))
                .with_tx(Some(to.x))
                .with_ty(Some(to.y)),
        )
        .expect("Transfer queues before removal");

    let path_index = engine
        .find_object_index(path_actor)
        .expect("path actor exists");
    let same_call_path = engine
        .call_object_function(path_index, "RemoveGate", vec![object_reference_value(gate)])
        .expect("script removal executes");
    assert_eq!(
        same_call_path,
        Value::Nil,
        "GetPath later in the removal call must not see the dead gate's zone"
    );
    assert_eq!(engine.frame(), 0, "no end-of-frame sweep has run");
    let gate_index = engine
        .find_object_index(gate)
        .expect("the deleted object remains as a physical tombstone");
    assert!(engine.objects[gate_index].destroyed);
    assert_eq!(
        engine.objects[gate_index].state.status,
        ObjectStatus::Deleted
    );
    assert!(
        engine.transfer_zones.get(gate).is_none(),
        "the authoritative zone table clears before frame cleanup"
    );
    assert!(engine.find_path(from, to, 1, true).is_none());

    run_obstructed_move_to(&mut engine, path_actor, to);
    let path_index = engine
        .find_object_index(path_actor)
        .expect("path actor remains");
    assert!(
        engine.objects[path_index]
            .commands
            .snapshot()
            .command_names()
            .iter()
            .all(|name| name != "Transfer"),
        "same-frame MoveTo must not enqueue a route through the removed zone"
    );

    let transfer_index = engine
        .find_object_index(transfer_actor)
        .expect("transfer actor remains");
    let transfer_view = engine.objects[transfer_index]
        .commands
        .snapshot()
        .command_views()
        .into_iter()
        .next()
        .expect("the pending Transfer remains queued until execution");
    assert_eq!(transfer_view.name, "Transfer");
    assert_eq!(
        transfer_view.target, None,
        "ClearPointers nulls the removed Transfer target synchronously"
    );
    engine
        .execute_object_command_now(transfer_actor)
        .expect("queued Transfer executes after removal");
    let transfer_index = engine
        .find_object_index(transfer_actor)
        .expect("transfer actor remains");
    assert!(
        engine.objects[transfer_index]
            .commands
            .snapshot()
            .is_empty(),
        "Transfer targeting the removed owner fails on its next same-frame execution"
    );
}

#[test]
fn script_removal_materializes_only_objects_that_can_reference_the_target() {
    // C4Game::ClearObjectPtrs visits the live object lists and
    // C4Object::ClearPointers only changes Action targets, Command targets,
    // effect command targets and pLayer (C4Game.cpp:1018-1031;
    // C4Object.cpp:2194-2205). The callback projection may prefilter those
    // scalar fields, but must still clear the one matching command.
    let mut engine = Engine::new();
    engine
        .register_script_definition("FILL", "Unrelated object", "#strict 2\n")
        .expect("filler definition registers");
    engine
        .register_script_definition("TARG", "Removal target", "#strict 2\n")
        .expect("target definition registers");
    engine
        .register_script_definition(
            "RMVR",
            "Removal caller",
            r#"
                #strict 2
                public func RemoveTarget(object target)
                {
                    return RemoveObject(target);
                }
            "#,
        )
        .expect("remover definition registers");

    let target = engine
        .spawn_object(SpawnConfig::new("TARG"))
        .expect("target spawns");
    for _ in 0..128 {
        engine
            .spawn_object(SpawnConfig::new("FILL"))
            .expect("filler spawns");
    }
    let action_referrer = engine
        .spawn_object(SpawnConfig::new("FILL"))
        .expect("action referrer spawns");
    let action_referrer_index = engine
        .find_object_index(action_referrer)
        .expect("action referrer exists");
    engine.objects[action_referrer_index].state.action.target = Some(target);

    let command_referrer = engine
        .spawn_object(SpawnConfig::new("FILL"))
        .expect("command referrer spawns");
    let command_referrer_index = engine
        .find_object_index(command_referrer)
        .expect("command referrer exists");
    engine.objects[command_referrer_index]
        .commands
        .push_front(CommandRequest::new(CommandId::Transfer).with_target(Some(target)))
        .expect("targeting command queues");

    let effect_referrer = engine
        .spawn_object(SpawnConfig::new("FILL"))
        .expect("effect referrer spawns");
    let effect_referrer_index = engine
        .find_object_index(effect_referrer)
        .expect("effect referrer exists");
    engine.objects[effect_referrer_index].state.effects.push(
        EffectState::new("Pointer").with_command_target(Some(
            i32::try_from(target.as_u64()).expect("test object id fits the native effect field"),
        )),
    );

    let layer_referrer = engine
        .spawn_object(SpawnConfig::new("FILL"))
        .expect("layer referrer spawns");
    let layer_referrer_index = engine
        .find_object_index(layer_referrer)
        .expect("layer referrer exists");
    engine.objects[layer_referrer_index].state.layer = Some(target);
    let remover = engine
        .spawn_object(SpawnConfig::new("RMVR"))
        .expect("remover spawns");
    let remover_index = engine.find_object_index(remover).expect("remover exists");

    HOST_WORLD_OBJECT_MATERIALIZATIONS.with(|count| count.set(0));
    engine
        .call_object_function(
            remover_index,
            "RemoveTarget",
            vec![object_reference_value(target)],
        )
        .expect("script removal executes");

    assert_eq!(
        engine.objects[action_referrer_index].state.action.target, None,
        "the matching action pointer must still clear"
    );
    assert_eq!(
        engine.objects[command_referrer_index]
            .commands
            .command_views()[0]
            .target,
        None,
        "the matching command pointer must still clear"
    );
    assert_eq!(
        engine.objects[effect_referrer_index].state.effects[0].command_target, None,
        "the matching effect pointer must still clear"
    );
    assert_eq!(
        engine.objects[layer_referrer_index].state.layer, None,
        "the matching layer pointer must still clear"
    );
    assert_eq!(
        HOST_WORLD_OBJECT_MATERIALIZATIONS.with(Cell::get),
        6,
        "only the callback caller, removal target and four referrers need full snapshots"
    );
}

#[test]
fn effect_batch_threads_and_folds_immediate_transfer_zone_clear() {
    const WIDTH: u32 = 100;
    const HEIGHT: u32 = 100;
    let mut pixels = vec![0; WIDTH as usize * HEIGHT as usize];
    for y in 0..HEIGHT as usize {
        pixels[y * WIDTH as usize + 49] = 1;
        pixels[y * WIDTH as usize + 50] = 1;
    }
    let from = Vector2::new(10, 50);
    let to = Vector2::new(90, 50);
    let mut engine = Engine::with_seed(2);
    engine.set_landscape(pixel_landscape(WIDTH, HEIGHT, pixels));
    engine
        .register_script_definition("GATE", "Transfer gate", "")
        .expect("gate definition registers");
    let mut effect_definition = Definition::from_script(
        "FXRM",
        "Effect remover",
        r#"
                #strict 2
                func Arm(initiator)
                {
                    AddEffect("RemoveGate", this(), 10, 1, this());
                    AddEffect("Observe", this(), 20, 1, this());
                    return true;
                }

                func FxRemoveGateTimer(object target, int number, int time)
                {
                    RemoveObject(FindObject(GATE));
                    return 0;
                }

                func FxObserveTimer(object target, int number, int time)
                {
                    if (GetPath(10, 50, 90, 50)) SetR(17);
                    else SetR(23);
                    return 0;
                }
            "#,
    )
    .expect("effect definition compiles");
    effect_definition.set_c4_callback_convention(true);
    effect_definition.set_pathfinder(1);
    engine
        .register_definition(effect_definition)
        .expect("effect definition registers");

    let gate = engine
        .spawn_object(SpawnConfig::new("GATE").with_position(Vector2::new(50, 50)))
        .expect("gate spawns");
    let actor = engine
        .spawn_object(
            SpawnConfig::new("FXRM")
                .with_position(from)
                .with_rotation(5),
        )
        .expect("effect actor spawns");
    engine
        .set_transfer_zone(
            gate,
            TransferZoneRect {
                x: 45,
                y: 40,
                width: 10,
                height: 20,
            },
        )
        .expect("transfer zone registers");
    assert!(engine.find_path(from, to, 1, true).is_some());
    assert_ne!(
        script_get_path(&mut engine, from, to),
        Value::Nil,
        "the effect callback's host GetPath can use the live zone"
    );

    let actor_index = engine.find_object_index(actor).expect("actor exists");
    engine
        .call_object_function(actor_index, "Arm", vec![object_reference_value(gate)])
        .expect("effects arm");
    let actor_index = engine.find_object_index(actor).expect("actor remains");
    let remove = engine.objects[actor_index]
        .state
        .effects
        .iter()
        .find(|effect| effect.name == "RemoveGate")
        .cloned()
        .expect("removal effect exists");
    let observe = engine.objects[actor_index]
        .state
        .effects
        .iter()
        .find(|effect| effect.name == "Observe")
        .cloned()
        .expect("observer effect exists");
    let definition_id = engine.objects[actor_index].definition_id.clone();

    engine
        .dispatch_object_effect_events(
            actor_index,
            &definition_id,
            vec![EffectEvent::timer(remove), EffectEvent::timer(observe)],
        )
        .expect("effect batch executes");

    assert_eq!(engine.frame(), 0, "no frame cleanup has run");
    assert_eq!(
        engine.objects[actor_index].state.rotation, 23,
        "the next effect callback sees the clear in its threaded host world"
    );
    assert!(
        engine.find_path(from, to, 1, true).is_none(),
        "the effect batch folds the clear into the authoritative table"
    );
}

#[test]
fn effect_batch_threads_callback_final_contents_order() {
    // C++ runs both timers against one live object graph. The first
    // timer moves a StaticBack pistol into BOX and immediately rotates
    // it to Contents.First; the second timer must observe that raw link
    // order before the deferred Rust batch folds authoritatively.
    let mut engine = Engine::with_seed(17);
    for id in ["BOX_", "HOLD", "ROCK", "GOLD", "PSTL"] {
        engine
            .register_script_definition(id, id, "#strict\n")
            .expect("definition registers");
    }
    let mut actor_definition = Definition::from_script(
        "FXCO",
        "Contents-order observer",
        r#"
                #strict 3
                local box;

                func Arm(object target)
                {
                    box = target;
                    AddEffect("Move", this(), 10, 1, this());
                    AddEffect("Observe", this(), 20, 1, this());
                    return(1);
                }

                func FxMoveTimer()
                {
                    Enter(box, FindObject(PSTL));
                    ShiftContents(box, true, PSTL);
                    return(0);
                }

                func FxObserveTimer()
                {
                    if (GetID(Contents(0, box)) == PSTL) SetR(17);
                    else SetR(23);
                    return(0);
                }
            "#,
    )
    .expect("effect actor compiles");
    actor_definition.set_c4_callback_convention(true);
    engine
        .register_definition(actor_definition)
        .expect("effect actor registers");

    let box_id = engine
        .spawn_object(SpawnConfig::new("BOX_").with_category(CATEGORY_OBJECT))
        .expect("box spawns");
    let gold = engine
        .spawn_object(
            SpawnConfig::new("GOLD")
                .with_category(CATEGORY_OBJECT)
                .with_container(box_id),
        )
        .expect("gold spawns");
    let rock = engine
        .spawn_object(
            SpawnConfig::new("ROCK")
                .with_category(CATEGORY_OBJECT)
                .with_container(box_id),
        )
        .expect("rock spawns");
    let holder = engine
        .spawn_object(SpawnConfig::new("HOLD").with_category(CATEGORY_OBJECT))
        .expect("holder spawns");
    let pistol = engine
        .spawn_object(
            SpawnConfig::new("PSTL")
                .with_category(CATEGORY_STATIC_BACK)
                .with_container(holder),
        )
        .expect("retained pistol spawns");
    let actor = engine
        .spawn_object(
            SpawnConfig::new("FXCO")
                .with_category(CATEGORY_OBJECT)
                .with_rotation(5),
        )
        .expect("effect actor spawns");
    let actor_index = engine.find_object_index(actor).expect("actor exists");
    engine
        .call_object_function(actor_index, "Arm", vec![object_reference_value(box_id)])
        .expect("effects arm");
    let actor_index = engine.find_object_index(actor).expect("actor remains");
    let move_effect = engine.objects[actor_index]
        .state
        .effects
        .iter()
        .find(|effect| effect.name == "Move")
        .cloned()
        .expect("move effect exists");
    let observe_effect = engine.objects[actor_index]
        .state
        .effects
        .iter()
        .find(|effect| effect.name == "Observe")
        .cloned()
        .expect("observe effect exists");
    let definition_id = engine.objects[actor_index].definition_id.clone();

    engine
        .dispatch_object_effect_events(
            actor_index,
            &definition_id,
            vec![
                EffectEvent::timer(move_effect),
                EffectEvent::timer(observe_effect),
            ],
        )
        .expect("effect batch executes");

    assert_eq!(
        engine.objects[actor_index].state.rotation, 17,
        "the second timer sees PSTL at Contents.First"
    );
    let box_index = engine.find_object_index(box_id).expect("box remains");
    assert_eq!(
        engine.objects[box_index].state.contents,
        vec![pistol, rock, gold],
        "the same callback-final list folds to the authoritative box"
    );
    assert_eq!(
        engine
            .object_snapshot(pistol)
            .expect("pistol remains")
            .container,
        Some(box_id)
    );
}

#[test]
fn effect_batch_threads_dig_contents_shape_and_layer() {
    let library = clonk_resources::MaterialLibrary::parse(
        r#"
            [Material Earth]
            Name=Earth
            Density=80
            DigFree=1
            Dig2Object=GEM_
            Dig2ObjectRatio=2
            "#,
    )
    .expect("dig material parses");
    let materials = MaterialSet::from_resource_library(&library);
    let mut engine = Engine::with_seed(59);
    engine.set_materials(materials);
    engine.set_landscape(pixel_landscape(2, 1, vec![1, 1]));

    engine
        .register_script_definition("LAYR", "Layer", "")
        .expect("layer definition registers");
    engine
        .register_script_definition("GEM_", "Gem", "")
        .expect("gem definition registers");
    let mut digger = Definition::from_script(
        "FXDG",
        "Effect digger",
        r#"
                #strict 3
                func Arm()
                {
                    AddEffect("First", this(), 200, 1, this());
                    AddEffect("Second", this(), 100, 1, this());
                }

                func FxFirstTimer()
                {
                    DigFreeRect(0, 0, 1, 1);
                    SetPosition(10, 20);
                    SetShape(-1, 3, 4, 7);
                    SetObjectLayer(FindObject(LAYR));
                    return 0;
                }

                func FxSecondTimer()
                {
                    DigFreeRect(1, 0, 1, 1);
                    return 0;
                }
            "#,
    )
    .expect("effect digger compiles");
    digger.set_c4_callback_convention(true);
    digger.set_shape_rect(Some(DefinitionRect::new(0, 0, 1, 2)));
    engine
        .register_definition(digger)
        .expect("effect digger registers");

    let layer = engine
        .spawn_object(SpawnConfig::new("LAYR"))
        .expect("layer spawns");
    let actor = engine
        .spawn_object(SpawnConfig::new("FXDG"))
        .expect("effect digger spawns");
    let actor_index = engine.find_object_index(actor).expect("actor exists");
    engine
        .call_object_function(actor_index, "Arm", Vec::new())
        .expect("effects arm");
    let actor_index = engine.find_object_index(actor).expect("actor remains");
    let first = engine.objects[actor_index]
        .state
        .effects
        .iter()
        .find(|effect| effect.name == "First")
        .cloned()
        .expect("first effect exists");
    let second = engine.objects[actor_index]
        .state
        .effects
        .iter()
        .find(|effect| effect.name == "Second")
        .cloned()
        .expect("second effect exists");
    let definition_id = engine.objects[actor_index].definition_id.clone();

    engine
        .dispatch_object_effect_events(
            actor_index,
            &definition_id,
            vec![EffectEvent::timer(first), EffectEvent::timer(second)],
        )
        .expect("effect batch executes");

    let gems = engine
        .objects
        .iter()
        .filter(|object| object.definition_id == "GEM_" && !object.destroyed)
        .collect::<Vec<_>>();
    assert_eq!(gems.len(), 1, "both callbacks share material credit");
    assert_eq!(
        gems[0].state.position,
        Vector2::new(10, 30),
        "the second callback uses the first callback's shape and position"
    );
    assert_eq!(
        gems[0].state.layer,
        Some(layer),
        "the second callback uses the first callback's layer"
    );
}

#[test]
fn construction_zone_clear_is_visible_to_immediate_initialize() {
    const WIDTH: u32 = 100;
    const HEIGHT: u32 = 100;
    let mut pixels = vec![0; WIDTH as usize * HEIGHT as usize];
    for y in 0..HEIGHT as usize {
        pixels[y * WIDTH as usize + 49] = 1;
        pixels[y * WIDTH as usize + 50] = 1;
    }
    let from = Vector2::new(10, 50);
    let to = Vector2::new(90, 50);
    let mut engine = Engine::with_seed(3);
    engine.set_landscape(pixel_landscape(WIDTH, HEIGHT, pixels));
    engine
        .register_script_definition("GATE", "Transfer gate", "")
        .expect("gate definition registers");
    let mut lifecycle_definition = Definition::from_script(
        "PFLC",
        "Lifecycle remover",
        r#"
                #strict 2
                func Construction()
                {
                    RemoveObject(FindObject(GATE));
                    return true;
                }

                func Initialize()
                {
                    if (GetPath(10, 50, 90, 50)) SetR(17);
                    else SetR(23);
                    return true;
                }
            "#,
    )
    .expect("lifecycle definition compiles");
    lifecycle_definition.set_c4_callback_convention(true);
    lifecycle_definition.set_pathfinder(1);
    engine
        .register_definition(lifecycle_definition)
        .expect("lifecycle definition registers");

    let gate = engine
        .spawn_object(SpawnConfig::new("GATE").with_position(Vector2::new(50, 50)))
        .expect("gate spawns");
    engine
        .set_transfer_zone(
            gate,
            TransferZoneRect {
                x: 45,
                y: 40,
                width: 10,
                height: 20,
            },
        )
        .expect("transfer zone registers");
    assert_ne!(script_get_path(&mut engine, from, to), Value::Nil);

    let actor = engine
        .spawn_object(
            SpawnConfig::new("PFLC")
                .with_position(from)
                .with_rotation(5),
        )
        .expect("lifecycle actor spawns");
    let actor_index = engine.find_object_index(actor).expect("actor remains");

    assert_eq!(engine.frame(), 0, "no frame cleanup has run");
    assert_eq!(
        engine.objects[actor_index].state.rotation, 23,
        "Initialize sees Construction's synchronous zone clear"
    );
    assert!(engine.transfer_zones.get(gate).is_none());
}
