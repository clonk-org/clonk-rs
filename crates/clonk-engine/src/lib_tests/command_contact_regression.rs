use super::*;
use crate::landscape::PixelGrid;
use std::collections::HashMap;

trait TestEngineExt {
    fn test_object_index(&self, object: ObjectId) -> usize;
    fn register_test_definition(&mut self, definition: Definition);
    fn register_test_player(&mut self, player: PlayerConfig);
    fn register_test_script_definition(&mut self, id: &str, name: &str, script: &str);
    fn spawn_test_object(&mut self, config: SpawnConfig) -> ObjectId;
}

impl TestEngineExt for Engine {
    #[track_caller]
    fn test_object_index(&self, object: ObjectId) -> usize {
        crate::TestValueExt::test_value(self.find_object_index(object))
    }

    #[track_caller]
    fn register_test_definition(&mut self, definition: Definition) {
        crate::TestValueExt::test_value(self.register_definition(definition));
    }

    #[track_caller]
    fn register_test_player(&mut self, player: PlayerConfig) {
        crate::TestValueExt::test_value(self.register_player(player));
    }

    #[track_caller]
    fn register_test_script_definition(&mut self, id: &str, name: &str, script: &str) {
        crate::TestValueExt::test_value(self.register_script_definition(id, name, script));
    }

    #[track_caller]
    fn spawn_test_object(&mut self, config: SpawnConfig) -> ObjectId {
        crate::TestValueExt::test_value(self.spawn_object(config))
    }
}

#[test]
fn previewing_an_unchanged_effect_list_reuses_the_seeded_object_state() {
    // C4Effect::Execute walks the one live effect list; preparing the next
    // callback does not copy that list when it has not changed
    // (C4Effect.cpp:319-363; C4Object.cpp:1069-1090).
    let mut engine = Engine::with_seed(0);
    engine.register_test_script_definition("FXOB", "Effect preview fixture", "");
    let object = engine.spawn_test_object(SpawnConfig::new("FXOB"));
    let index = engine.test_object_index(object);
    engine.objects[index]
        .state
        .effects
        .push(EffectState::new("Pulse"));

    let mut world = engine.host_world_context_for_object(index);
    let before = crate::TestValueExt::test_value(
        world
            .get_shared(object)
            .and_then(|object| object.full_state().cloned()),
    );
    world.preview_object_effects(object, &before.effects);
    let after = crate::TestValueExt::test_value(
        world
            .get_shared(object)
            .and_then(|object| object.full_state().cloned()),
    );

    assert!(std::rc::Rc::ptr_eq(&before, &after));
}

#[test]
fn pixel_less_landscape_does_not_invent_column_surface_contact() {
    // C4Object::ContactCheck samples the current shape against landscape
    // pixels (C4Movement.cpp:165-181); C4Object::DoMovement consumes that
    // contact result (C4Movement.cpp:231). The C++ oracle has no
    // per-column surface snap for a pixel-less landscape.
    let mut engine = Engine::with_seed(0);
    engine.set_landscape(Landscape::flat(20, 5));

    let mut definition = test_definition("FALL", "Falling fixture", "");
    definition.set_shape_vertices(vec![ObjectVertex::new(0, 0)]);
    engine.register_test_definition(definition);
    let object =
        engine.spawn_test_object(SpawnConfig::new("FALL").with_position(Vector2::new(3, 8)));

    crate::TestValueExt::test_value(
        engine.apply_object_update(
            object,
            ObjectUpdate::new()
                .with_position(Vector2::new(3, 8))
                .with_velocity(Vector2::new(0, 3)),
        ),
    );

    let index = engine.test_object_index(object);
    assert_eq!(engine.objects[index].state.position, Vector2::new(3, 8));
    assert_eq!(engine.objects[index].state.velocity, Vector2::new(0, 3));
}

#[test]
fn synchronize_control_applies_clearance_only_when_requested() {
    // C4ControlSynchronize executes Game.Synchronize first and calls
    // Game.SyncClearance only when SyncClear is set. The latter alone
    // collapses fixed coordinates to integer object state (pristine
    // 9ffa0a5d src/C4Control.cpp:537-550;
    // src/C4Game.cpp:3679-3715; src/C4Object.cpp:3803-3815).
    let mut engine = Engine::with_seed(0);
    engine.register_test_script_definition("SYNC", "Sync", "");
    let object =
        engine.spawn_test_object(SpawnConfig::new("SYNC").with_position(Vector2::new(10, 20)));
    let index = engine.test_object_index(object);
    let fractional = crate::math::C4Fixed::from_raw(itofix(10).val().wrapping_add(1));
    engine.objects[index].fixed_position.x = fractional;

    crate::TestValueExt::test_value(engine.execute_synchronize_control(false, false));
    assert_eq!(engine.objects[index].fixed_position.x, fractional);

    crate::TestValueExt::test_value(engine.execute_synchronize_control(false, true));
    assert_eq!(engine.objects[index].fixed_position.x, itofix(10));
}

#[test]
fn synchronize_control_checkpoints_live_player_and_crew_time_but_not_replays() {
    let mut engine = Engine::new();
    engine.game_time = 10;
    engine.register_test_player(
        PlayerConfig::new(3, "Local")
            .with_player_info_id(17)
            .with_total_playing_time(40),
    );
    engine.crew_rosters.insert(
        3,
        vec![player_file::CrewInfo {
            id: "CLNK".to_string(),
            name: "Crew".to_string(),
            total_playing_time: 7,
            in_action: true,
            was_in_action: true,
            in_action_time: 10,
            ..Default::default()
        }],
    );
    engine.register_test_player(
        PlayerConfig::new(4, "Script")
            .with_player_info_id(18)
            .with_total_playing_time(70),
    );
    crate::TestValueExt::test_value(engine.player_mut(4)).set_script_player(true);
    engine.register_test_player(
        PlayerConfig::new(5, "Eliminated")
            .with_player_info_id(19)
            .with_total_playing_time(80)
            .with_status(PlayerStatus::Eliminated),
    );
    let suppressed_crew = player_file::CrewInfo {
        id: "CLNK".to_string(),
        name: "Suppressed Crew".to_string(),
        total_playing_time: 17,
        in_action: true,
        was_in_action: true,
        in_action_time: 10,
        ..Default::default()
    };
    engine.crew_rosters.insert(4, vec![suppressed_crew.clone()]);
    engine.crew_rosters.insert(5, vec![suppressed_crew]);
    engine.game_time = 25;

    crate::TestValueExt::test_value(engine.execute_synchronize_control(true, false));
    assert_eq!(engine.player(3).unwrap().total_playing_time(), 55);
    assert_eq!(engine.player(3).unwrap().game_join_time(), 25);
    assert_eq!(engine.crew_rosters[&3][0].total_playing_time, 22);
    assert_eq!(engine.crew_rosters[&3][0].in_action_time, 25);
    assert_eq!(engine.player(4).unwrap().total_playing_time(), 70);
    assert_eq!(engine.player(4).unwrap().game_join_time(), 10);
    assert_eq!(engine.player(5).unwrap().total_playing_time(), 80);
    assert_eq!(engine.player(5).unwrap().game_join_time(), 10);
    for player_id in [4, 5] {
        assert_eq!(engine.crew_rosters[&player_id][0].total_playing_time, 17);
        assert_eq!(engine.crew_rosters[&player_id][0].in_action_time, 10);
    }

    engine.set_replay_control(true);
    engine.game_time = 30;
    crate::TestValueExt::test_value(engine.execute_synchronize_control(true, false));
    assert_eq!(engine.player(3).unwrap().total_playing_time(), 55);
    assert_eq!(engine.crew_rosters[&3][0].total_playing_time, 22);
}

#[test]
fn free_stabilize_probe_clears_previous_contact_latch() {
    // C4Object::Stabilize rotates the shape upright and calls
    // ContactCheck; ContactCheck stores Shape.ContactCNAT even when it is
    // zero (C4Movement.cpp:493-516,166-182).
    let mut engine = Engine::with_seed(0);
    engine.set_landscape(Landscape::flat(100, 100));

    let mut definition = test_definition("TILT", "Tilt", "");
    definition.set_rotateable(1);
    definition.set_shape_vertices(vec![ObjectVertex {
        x: 0,
        y: 1,
        cnat: CNAT_BOTTOM,
        friction: 100,
    }]);
    engine.register_test_definition(definition);

    let object_id = engine.spawn_test_object(
        SpawnConfig::new("TILT")
            .with_position(Vector2::new(50, 50))
            .with_rotation(5),
    );
    let index = engine.test_object_index(object_id);
    engine.objects[index].frame_t_contact = CNAT_LEFT;

    crate::TestValueExt::test_value(engine.stabilize_object(index, &[]));

    assert_eq!(engine.objects[index].state.rotation, 0);
    assert_eq!(engine.objects[index].frame_t_contact, CNAT_NONE);
}

#[test]
fn command_snapshot_keeps_definition_command_policies() {
    // Commands read these policies from cObj->Def at execution, so the
    // Rust frame snapshot must preserve the raw engine definition values
    // rather than infer them from crew OCF.
    let mut engine = Engine::with_seed(0);
    let mut definition = test_definition("ROUT", "Router", "");
    definition.set_pathfinder(-4);
    definition.set_no_transfer_zones(-3);
    definition.set_no_push_enter(-2);
    definition.configure_actions(
        Some("Route".to_owned()),
        HashMap::from([(
            "Route".to_owned(),
            ActionSpec::default().with_disabled(true),
        )]),
    );
    engine.register_test_definition(definition);
    let object_id =
        engine.spawn_test_object(SpawnConfig::new("ROUT").with_action(ActionState::new("Route")));
    let index = engine.test_object_index(object_id);

    let snapshot = engine.live_command_snapshot(index, None);

    assert_eq!(snapshot.pathfinder, -4);
    assert_eq!(snapshot.no_transfer_zones, -3);
    assert_eq!(snapshot.no_push_enter, -2);
    assert!(snapshot.action_disabled);
    assert_eq!(snapshot.ocf & ocf::CREW_MEMBER, 0);
}

#[test]
fn idle_objects_do_not_materialize_command_snapshots() {
    // C4Object::ExecuteCommand returns without reading object or world state
    // when Command is null (C4Object.cpp:3997-4009).
    let mut engine = Engine::with_seed(0);
    engine.register_test_script_definition("IDLE", "Idle", "");
    for x in 0..64 {
        engine.spawn_test_object(SpawnConfig::new("IDLE").with_position(Vector2::new(x, 10)));
    }

    COMMAND_SNAPSHOT_MATERIALIZATIONS.with(|count| count.set(0));
    crate::TestValueExt::test_value(engine.tick());

    assert_eq!(
        COMMAND_SNAPSHOT_MATERIALIZATIONS.with(Cell::get),
        0,
        "commandless objects do not need Rust's borrowed-world snapshot table"
    );
}

#[test]
fn commandless_objects_do_not_enter_the_command_queue_executor() {
    // C4Object::ExecuteCommand returns immediately when Command is null
    // (C4Object.cpp:3997-4009); no deferred command work exists to fold.
    let mut engine = Engine::with_seed(0);
    engine.register_test_script_definition("IDLE", "Idle", "");
    for x in 0..64 {
        engine.spawn_test_object(SpawnConfig::new("IDLE").with_position(Vector2::new(x, 10)));
    }

    EMPTY_COMMAND_QUEUE_EXECUTIONS.with(|count| count.set(0));
    crate::TestValueExt::test_value(engine.tick());

    assert_eq!(EMPTY_COMMAND_QUEUE_EXECUTIONS.with(Cell::get), 0);
}

#[test]
fn ordinary_ticks_update_sectors_incrementally() {
    // C++ adds, removes, and updates each object's sector links at those
    // mutations; C4GameObjects::CrossCheck does no full rebuild
    // (C4GameObjects.cpp:60-80,92-115,730-743).
    let mut engine = Engine::with_seed(0);
    engine.set_landscape(Landscape::flat(200, 200));
    engine.register_test_script_definition("IDLE", "Idle", "");
    for x in 0..128 {
        engine.spawn_test_object(SpawnConfig::new("IDLE").with_position(Vector2::new(x, 50)));
    }

    SECTOR_FULL_REBUILDS.with(|count| count.set(0));
    crate::TestValueExt::test_value(engine.tick());

    assert_eq!(SECTOR_FULL_REBUILDS.with(Cell::get), 0);
}

#[test]
fn real_content_without_step_skips_the_synthetic_command_fold() {
    // C4Object::Execute has no per-definition Step callback or returned
    // command batch (C4Object.cpp:1058-1127). That fold exists only for the
    // Rust snapshot-fixture DSL and must stay out of real-content frames.
    let mut engine = Engine::with_seed(0);
    engine.register_test_script_definition("REAL", "Real content", "");
    engine.spawn_test_object(SpawnConfig::new("REAL"));

    SYNTHETIC_COMMAND_FOLDS.with(|count| count.set(0));
    crate::TestValueExt::test_value(engine.tick_without_snapshot());

    assert_eq!(SYNTHETIC_COMMAND_FOLDS.with(Cell::get), 0);
}

#[test]
fn unchanged_actions_skip_the_deferred_callback_drain() {
    // Native SetAction dispatches Start/Abort/End synchronously and has no
    // empty deferred-callback drain in C4Object::Execute (C4Object.cpp:
    // 4160-4185,1058-1127).
    let mut engine = Engine::with_seed(0);
    engine.register_test_script_definition("IDLE", "Idle object", "");
    engine.spawn_test_object(SpawnConfig::new("IDLE"));

    ACTION_CALLBACK_DRAIN_INVOCATIONS.with(|count| count.set(0));
    crate::TestValueExt::test_value(engine.tick_without_snapshot());

    assert_eq!(ACTION_CALLBACK_DRAIN_INVOCATIONS.with(Cell::get), 0);
}

#[test]
fn object_action_lookup_stays_on_the_definition_table() {
    let mut engine = Engine::with_seed(0);
    engine.register_test_script_definition("LOOK", "Lookup object", "");
    let object = engine.spawn_test_object(SpawnConfig::new("LOOK"));
    let index = engine.test_object_index(object);

    DEFINITION_METADATA_TABLE_READS.with(|count| count.set(0));
    let _ = crate::TestValueExt::test_value(engine.object_definition_context(index));

    assert_eq!(DEFINITION_METADATA_TABLE_READS.with(Cell::get), 0);
}

#[test]
fn coordinate_move_to_does_not_snapshot_unrelated_objects() {
    // A targetless C4Command::MoveTo reads cObj plus terrain/pathfinder state;
    // other objects enter only through Action.Target (C4Command.cpp:211-360).
    let mut engine = Engine::with_seed(0);
    engine.register_test_script_definition("FILL", "Filler", "");
    let mut walker = test_definition("WALK", "Walker", "");
    walker.configure_actions(
        Some("Walk".to_owned()),
        HashMap::from([(
            "Walk".to_owned(),
            ActionSpec::default().with_procedure("WALK"),
        )]),
    );
    engine.register_test_definition(walker);
    for x in 0..64 {
        engine.spawn_test_object(SpawnConfig::new("FILL").with_position(Vector2::new(x, 10)));
    }
    let walker = engine.spawn_test_object(
        SpawnConfig::new("WALK")
            .with_position(Vector2::new(10, 10))
            .with_action(ActionState::new("Walk")),
    );
    let walker_index = engine.test_object_index(walker);
    crate::TestValueExt::test_value(
        engine.objects[walker_index].commands.push_front(
            CommandRequest::new(CommandId::MoveTo)
                .with_tx(Some(100))
                .with_ty(Some(10))
                .with_evaluated(true),
        ),
    );

    COMMAND_SNAPSHOT_MATERIALIZATIONS.with(|count| count.set(0));
    crate::TestValueExt::test_value(engine.tick());

    assert!(
        COMMAND_SNAPSHOT_MATERIALIZATIONS.with(Cell::get) <= 3,
        "the actor may refresh around its command, but fillers stay unmaterialized"
    );
    assert_eq!(
        engine.objects[walker_index].state.command_direction,
        CommandDirection::Right
    );
}

#[test]
fn targeted_move_to_snapshots_only_its_explicit_dependencies() {
    // C4Command::MoveTo dereferences Target during InitEvaluation and
    // Action.Target for push/pull, never Game.Objects (C4Command.cpp:211-360).
    let mut engine = Engine::with_seed(0);
    engine.register_test_script_definition("FILL", "Filler", "");
    let mut walker = test_definition("WALK", "Walker", "");
    walker.configure_actions(
        Some("Walk".to_owned()),
        HashMap::from([(
            "Walk".to_owned(),
            ActionSpec::default().with_procedure("WALK"),
        )]),
    );
    engine.register_test_definition(walker);
    let target =
        engine.spawn_test_object(SpawnConfig::new("FILL").with_position(Vector2::new(100, 10)));
    for x in 0..63 {
        engine.spawn_test_object(SpawnConfig::new("FILL").with_position(Vector2::new(x, 20)));
    }
    let walker = engine.spawn_test_object(
        SpawnConfig::new("WALK")
            .with_position(Vector2::new(10, 10))
            .with_action(ActionState::new("Walk")),
    );
    let walker_index = engine.test_object_index(walker);
    crate::TestValueExt::test_value(
        engine.objects[walker_index].commands.push_front(
            CommandRequest::new(CommandId::MoveTo)
                .with_target(Some(target))
                .with_tx(Some(0))
                .with_ty(Some(0)),
        ),
    );

    COMMAND_SNAPSHOT_MATERIALIZATIONS.with(|count| count.set(0));
    crate::TestValueExt::test_value(engine.tick());
    crate::TestValueExt::test_value(engine.tick());

    assert!(
        COMMAND_SNAPSHOT_MATERIALIZATIONS.with(Cell::get) <= 10,
        "the actor and explicit target may refresh, but unrelated fillers stay unmaterialized"
    );
    assert_eq!(
        engine.objects[walker_index].state.command_direction,
        CommandDirection::Right
    );
}

#[test]
/// `C4Object::Call` resolves the function first and returns C4VNull when the
/// definition does not declare it (`C4Object.cpp:2224-2240`), so an absent
/// callback costs a name lookup and nothing else.
///
/// Here the whole calling context — the object's script state, the global
/// effect list, the action library and a host world — was materialised
/// *before* the resolution that finds nothing. Scenario activation is
/// dominated by placement callbacks, and most definitions declare only some of
/// the lifecycle ones, so that was paid hundreds of times per load for calls
/// that never ran.
fn an_absent_callback_materialises_nothing() {
    let mut engine = Engine::new();
    engine.register_test_script_definition(
        "LAZY",
        "Lazy",
        "#strict\nfunc Present() { return(1); }\n",
    );
    let object =
        engine.spawn_test_object(SpawnConfig::new("LAZY").with_position(Vector2::new(50, 50)));
    let index = engine.test_object_index(object);

    HOST_WORLD_OBJECT_MATERIALIZATIONS.with(|count| count.set(0));
    SCRIPT_STATE_SNAPSHOT_MATERIALIZATIONS.with(|count| count.set(0));
    assert_eq!(
        crate::TestValueExt::test_value(engine.call_object_function(index, "Absent", Vec::new())),
        Value::Nil,
        "an undeclared callback is a silent no-op returning nil"
    );
    assert_eq!(
        SCRIPT_STATE_SNAPSHOT_MATERIALIZATIONS.with(Cell::get),
        0,
        "an undeclared callback copies no object state"
    );
    assert_eq!(
        HOST_WORLD_OBJECT_MATERIALIZATIONS.with(Cell::get),
        0,
        "an undeclared callback builds no host world"
    );

    // The declared one still runs, and still pays for exactly one snapshot.
    SCRIPT_STATE_SNAPSHOT_MATERIALIZATIONS.with(|count| count.set(0));
    assert_eq!(
        crate::TestValueExt::test_value(engine.call_object_function(index, "Present", Vec::new())),
        Value::Int(1),
    );
    assert_eq!(
        SCRIPT_STATE_SNAPSHOT_MATERIALIZATIONS.with(Cell::get),
        1,
        "a declared callback is unaffected"
    );
}

#[test]
fn lazy_host_world_call_object_materializes_only_on_world_access() {
    let mut engine = Engine::with_seed(0);
    let mut landscape = crate::TestValueExt::test_value(Landscape::with_default_material(
        100,
        vec![100; 100],
        None,
    ));
    landscape.set_world_height(100);
    let mut pixels = vec![0; 100 * 100];
    pixels[50 * 100 + 52] = 1;
    landscape.set_pixel_grid(PixelGrid::new(
        100,
        100,
        pixels,
        vec![0, 100],
        vec![None, Some("Earth".to_owned())],
        vec![None; 2],
    ));
    engine.set_landscape(landscape);

    engine.register_test_script_definition("FILL", "Filler", "#strict\n");
    let mut caller = test_definition(
        "LAZY",
        "Lazy caller",
        r#"#strict
    local self_calls, world_count, wall_seen;
    protected func SelfOnly()
    {
        self_calls++;
        return(GetX());
    }
    protected func QueryWorld()
    {
        world_count = ObjectCount();
        wall_seen = GBackSolid(2, 0);
        return(world_count);
    }
    "#,
    );
    caller.set_c4_callback_convention(true);
    engine.register_test_definition(caller);
    for x in 0..64 {
        engine.spawn_test_object(SpawnConfig::new("FILL").with_position(Vector2::new(x % 100, 10)));
    }
    let caller =
        engine.spawn_test_object(SpawnConfig::new("LAZY").with_position(Vector2::new(50, 50)));
    let caller_index = engine.test_object_index(caller);

    HOST_WORLD_OBJECT_MATERIALIZATIONS.with(|count| count.set(0));
    HOST_WORLD_LANDSCAPE_MATERIALIZATIONS.with(|count| count.set(0));
    HOST_WORLD_MASTER_ORDER_MATERIALIZATIONS.with(|count| count.set(0));
    SCRIPT_STATE_SNAPSHOT_MATERIALIZATIONS.with(|count| count.set(0));
    assert_eq!(
        engine
            .call_object_function(caller_index, "SelfOnly", Vec::new())
            .expect("self-only callback succeeds"),
        Value::Int(50),
    );
    assert_eq!(
        HOST_WORLD_OBJECT_MATERIALIZATIONS.with(Cell::get),
        1,
        "callback setup copies only the executing object"
    );
    assert_eq!(
        SCRIPT_STATE_SNAPSHOT_MATERIALIZATIONS.with(Cell::get),
        1,
        "the callback and its seeded host-world object share one actor-state snapshot"
    );
    assert_eq!(
        HOST_WORLD_LANDSCAPE_MATERIALIZATIONS.with(Cell::get),
        0,
        "an object-local callback never clones the landscape"
    );
    assert_eq!(
        HOST_WORLD_MASTER_ORDER_MATERIALIZATIONS.with(Cell::get),
        0,
        "an object-local callback never copies the master order"
    );

    let world = engine.host_world_context();
    assert_eq!(HOST_WORLD_MASTER_ORDER_MATERIALIZATIONS.with(Cell::get), 0);
    assert_eq!(world.master_object_ids().len(), engine.objects.len());
    assert_eq!(HOST_WORLD_MASTER_ORDER_MATERIALIZATIONS.with(Cell::get), 1);
    assert_eq!(world.master_object_ids().len(), engine.objects.len());
    assert_eq!(
        HOST_WORLD_MASTER_ORDER_MATERIALIZATIONS.with(Cell::get),
        1,
        "one callback world materializes its master order at most once"
    );
    drop(world);

    HOST_WORLD_OBJECT_MATERIALIZATIONS.with(|count| count.set(0));
    HOST_WORLD_LANDSCAPE_MATERIALIZATIONS.with(|count| count.set(0));
    assert_eq!(
        engine
            .call_object_function(caller_index, "QueryWorld", Vec::new())
            .expect("world-query callback succeeds"),
        Value::Int(engine.objects.len() as i32 - 1),
    );
    assert_eq!(
        HOST_WORLD_OBJECT_MATERIALIZATIONS.with(Cell::get),
        1,
        "scalar ObjectCount enumeration retains only the executing-object snapshot"
    );
    assert_eq!(
        HOST_WORLD_LANDSCAPE_MATERIALIZATIONS.with(Cell::get),
        0,
        "a terrain query borrows the landscape; only a terrain write copies it"
    );
    assert_eq!(
        engine.objects[caller_index]
            .state
            .local_vars
            .get("wall_seen"),
        Some(&Value::Bool(true)),
        "the lazy landscape preserves object-relative GBackSolid behavior"
    );
}

#[test]
fn set_position_without_a_solid_mask_bake_does_not_materialize_the_landscape() {
    // C4Object::ForcePosition only reaches UpdateSolidMask after its changed
    // X/Y gate (oracle-src-pinned src/C4Movement.cpp:552-561). An ordinary
    // object has no C4SolidMask raster to remove, so that callback-local
    // bookkeeping must not deep-clone the landscape.
    let mut engine = Engine::with_seed(0);
    engine.set_landscape(Landscape::flat(100, 100));
    let mover = test_definition(
        "MOVE",
        "Unmasked mover",
        r#"#strict
    func Move() { SetPosition(20, 30); return(0); }
    "#,
    );
    engine.register_test_definition(mover);
    let mover =
        engine.spawn_test_object(SpawnConfig::new("MOVE").with_position(Vector2::new(10, 10)));
    let mover_index = engine.test_object_index(mover);

    HOST_WORLD_LANDSCAPE_MATERIALIZATIONS.with(|count| count.set(0));
    assert_eq!(
        engine
            .call_object_function(mover_index, "Move", Vec::new())
            .expect("move callback succeeds"),
        Value::Nil,
    );
    assert_eq!(
        HOST_WORLD_LANDSCAPE_MATERIALIZATIONS.with(Cell::get),
        0,
        "removing an absent solid-mask bake must not clone the callback landscape"
    );
    assert_eq!(
        engine.objects[mover_index].state.position,
        Vector2::new(20, 30),
        "the callback still applies its position update"
    );
}

#[test]
fn lazy_master_order_ignores_stale_object_index_cache() {
    let mut engine = Engine::new();
    engine.register_test_script_definition("ORDR", "Order", "");
    let ids = (0..3)
        .map(|_| engine.spawn_test_object(SpawnConfig::new("ORDR")))
        .collect::<Vec<_>>();
    for id in &ids {
        assert!(engine.find_object_index(*id).is_some());
    }

    engine.objects.swap(1, 2);
    let inactive = ids[1];
    crate::TestValueExt::test_value(
        engine
            .objects
            .iter_mut()
            .find(|object| object.id == inactive),
    )
    .state
    .status = ObjectStatus::Inactive;
    let expected = engine
        .exec_list
        .iter()
        .rev()
        .copied()
        .filter(|id| {
            engine
                .objects
                .iter()
                .find(|object| object.id == *id)
                .is_some_and(|object| object.state.status != ObjectStatus::Inactive)
        })
        .collect::<Vec<_>>();

    let world = engine.host_world_context();

    assert_eq!(world.master_object_ids(), expected);
}

#[test]
fn lazy_master_order_reads_live_statuses_without_projecting_a_table() {
    let mut engine = Engine::new();
    engine.register_test_script_definition("ORDR", "Order", "");
    for _ in 0..64 {
        engine.spawn_test_object(SpawnConfig::new("ORDR"));
    }
    let expected = engine.exec_list.iter().rev().copied().collect::<Vec<_>>();

    HOST_WORLD_MASTER_ORDER_SOURCE_STATUS_READS.with(|count| count.set(0));
    let world = engine.host_world_context_for_object(0);

    assert_eq!(world.master_object_ids(), expected);
    assert_eq!(
        HOST_WORLD_MASTER_ORDER_SOURCE_STATUS_READS.with(Cell::get),
        expected.len() - 1,
        "master-order lookup must read each unseeded source status exactly once"
    );
}

#[test]
fn lazy_host_world_action_callback_seeds_only_caller() {
    let mut engine = Engine::with_seed(0);
    engine.register_test_script_definition("FILL", "Filler", "#strict\n");
    let mut actor = test_definition(
        "ACTR",
        "Action caller",
        "#strict\nlocal phase_calls; protected func OnPhase() { phase_calls++; return(0); }",
    );
    actor.set_c4_callback_convention(true);
    actor.configure_actions(
        Some("Swim".to_owned()),
        HashMap::from([(
            "Swim".to_owned(),
            ActionSpec::default().with_phase_call("OnPhase"),
        )]),
    );
    engine.register_test_definition(actor);
    for x in 0..64 {
        engine.spawn_test_object(SpawnConfig::new("FILL").with_position(Vector2::new(x, 10)));
    }
    let actor =
        engine.spawn_test_object(SpawnConfig::new("ACTR").with_action(ActionState::new("Swim")));
    let actor_index = engine.test_object_index(actor);
    let action_index = engine.objects[actor_index].state.action.act_map_index;

    HOST_WORLD_OBJECT_MATERIALIZATIONS.with(|count| count.set(0));
    HOST_WORLD_LANDSCAPE_MATERIALIZATIONS.with(|count| count.set(0));
    SCRIPT_STATE_SNAPSHOT_MATERIALIZATIONS.with(|count| count.set(0));
    crate::TestValueExt::test_value(engine.invoke_action_callback(
        actor_index,
        ActionCallbackKind::Phase,
        "Swim",
        action_index,
        None,
        None,
        None,
        None,
    ));
    assert_eq!(
        HOST_WORLD_OBJECT_MATERIALIZATIONS.with(Cell::get),
        1,
        "a self-only PhaseCall copies no unrelated object"
    );
    assert_eq!(
        SCRIPT_STATE_SNAPSHOT_MATERIALIZATIONS.with(Cell::get),
        1,
        "ordinary action callbacks reuse their actor-state snapshot in the host world"
    );
    assert_eq!(HOST_WORLD_LANDSCAPE_MATERIALIZATIONS.with(Cell::get), 0,);
    assert_eq!(
        engine.objects[actor_index]
            .state
            .local_vars
            .get("phase_calls"),
        Some(&Value::Int(1)),
    );
}

#[test]
fn legacy_find_object_rejects_nonmatches_without_full_state_materialization() {
    // C4Game::FindObject walks Game.Objects from First -> Next and tests
    // scalar C4Object fields in place (C4Game.cpp:1337-1424). The first
    // matching object must therefore stay master-order authoritative,
    // without copying every rejected object's full script state.
    let mut engine = Engine::with_seed(0);
    for definition in [
        test_definition("TARG", "Target", "#strict\n"),
        test_definition("FILL", "Filler", "#strict\n"),
    ] {
        engine.register_test_definition(definition);
    }
    let mut searcher = test_definition(
        "SRCH",
        "Searcher",
        "#strict\n\
         protected func FindTarget() { return(FindObject(TARG)); }\n\
         protected func FindOwned() { return(FindObjectOwner(TARG, 3)); }\n",
    );
    searcher.set_c4_callback_convention(true);
    engine.register_test_definition(searcher);
    for player in [2, 3] {
        engine.register_test_player(PlayerConfig::new(player, format!("Player {player}")));
    }

    let older_target = engine.spawn_test_object(SpawnConfig::new("TARG").with_owner(3));
    for x in 0..64 {
        engine.spawn_test_object(SpawnConfig::new("FILL").with_position(Vector2::new(x, 10)));
    }
    let newer_target = engine.spawn_test_object(SpawnConfig::new("TARG").with_owner(2));
    for x in 64..128 {
        engine.spawn_test_object(SpawnConfig::new("FILL").with_position(Vector2::new(x, 10)));
    }
    let searcher = engine.spawn_test_object(SpawnConfig::new("SRCH"));
    let searcher_index = engine.test_object_index(searcher);

    HOST_WORLD_OBJECT_MATERIALIZATIONS.with(|count| count.set(0));
    assert_eq!(
        engine
            .call_object_function(searcher_index, "FindTarget", Vec::new())
            .expect("FindObject succeeds"),
        object_reference_value(newer_target),
        "the newer target is first in C++ forward master-list order"
    );
    assert_ne!(newer_target, older_target);
    assert_eq!(
        HOST_WORLD_OBJECT_MATERIALIZATIONS.with(Cell::get),
        1,
        "only the already-required executing-object snapshot is materialized"
    );

    HOST_WORLD_OBJECT_MATERIALIZATIONS.with(|count| count.set(0));
    assert_eq!(
        engine
            .call_object_function(searcher_index, "FindOwned", Vec::new())
            .expect("FindObjectOwner succeeds"),
        object_reference_value(older_target),
        "the owner predicate skips the newer target without disturbing order"
    );
    assert_eq!(
        HOST_WORLD_OBJECT_MATERIALIZATIONS.with(Cell::get),
        1,
        "FindObjectOwner also rejects candidates through scalar fields"
    );
}

#[test]
fn legacy_object_count_filters_scalars_without_full_state_materialization() {
    // FnObjectCount applies the fixed-parameter C4Game::FindObject predicates
    // to live scalar fields while counting every match
    // (oracle-src-pinned src/C4Script.cpp:2085-2111; src/C4Game.cpp:1337-1424).
    let mut engine = Engine::with_seed(0);
    for definition in [
        test_definition("TARG", "Target", "#strict\n"),
        test_definition("FILL", "Filler", "#strict\n"),
        test_definition(
            "SRCH",
            "Searcher",
            "#strict\nprotected func CountTargets() { return ObjectCount(TARG); }\n",
        ),
    ] {
        engine.register_test_definition(definition);
    }
    for _ in 0..64 {
        engine.spawn_test_object(SpawnConfig::new("FILL"));
    }
    for _ in 0..2 {
        engine.spawn_test_object(SpawnConfig::new("TARG"));
    }
    let searcher = engine.spawn_test_object(SpawnConfig::new("SRCH"));
    let searcher = engine.test_object_index(searcher);

    HOST_WORLD_OBJECT_MATERIALIZATIONS.with(|count| count.set(0));
    assert_eq!(
        engine
            .call_object_function(searcher, "CountTargets", Vec::new())
            .expect("ObjectCount succeeds"),
        Value::Int(2)
    );
    assert_eq!(
        HOST_WORLD_OBJECT_MATERIALIZATIONS.with(Cell::get),
        1,
        "only the executing-object snapshot is materialized"
    );
}

#[test]
fn criterion_object_count_filters_scalars_without_full_state_materialization() {
    // C4FindObject::Check reads ID and Distance directly from each live
    // C4Object; scalar criterion trees do not copy object state while walking
    // candidates (oracle-src-pinned src/C4FindObject.cpp:188-226, 566-611).
    let mut engine = Engine::with_seed(0);
    for definition in [
        test_definition("TARG", "Target", "#strict\n"),
        test_definition("FILL", "Filler", "#strict\n"),
        test_definition("SRCH", "Searcher", "#strict\nprotected func CountTargets() { return ObjectCount2([20, TARG], [14, 0, 10, 50]); }\n"),
    ] {
        engine.register_test_definition(definition);
    }
    for x in 0..64 {
        engine.spawn_test_object(SpawnConfig::new("FILL").with_position(Vector2::new(x, 10)));
    }
    for x in [10, 20] {
        engine.spawn_test_object(SpawnConfig::new("TARG").with_position(Vector2::new(x, 10)));
    }
    let searcher =
        engine.spawn_test_object(SpawnConfig::new("SRCH").with_position(Vector2::new(0, 10)));
    let searcher = engine.test_object_index(searcher);

    HOST_WORLD_OBJECT_MATERIALIZATIONS.with(|count| count.set(0));
    assert_eq!(
        engine
            .call_object_function(searcher, "CountTargets", Vec::new())
            .expect("ObjectCount2 succeeds"),
        Value::Int(2)
    );
    assert_eq!(
        HOST_WORLD_OBJECT_MATERIALIZATIONS.with(Cell::get),
        1,
        "only the executing-object snapshot is materialized"
    );
}

#[test]
fn criterion_find_object_filters_scalars_without_full_state_materialization() {
    // C4FindObject::Find calls scalar Check predicates directly on each live
    // C4Object and returns the first match; no candidate state is copied
    // (oracle-src-pinned src/C4FindObject.cpp:180-199, 566-611).
    let mut engine = Engine::with_seed(0);
    for definition in [
        test_definition("TARG", "Target", "#strict\n"),
        test_definition("FILL", "Filler", "#strict\n"),
        test_definition("SRCH", "Searcher", "#strict\nprotected func FindTarget() { return FindObject2([20, TARG], [14, 0, 10, 50]); }\n"),
    ] {
        engine.register_test_definition(definition);
    }
    for x in 0..64 {
        engine.spawn_test_object(SpawnConfig::new("FILL").with_position(Vector2::new(x, 10)));
    }
    let target =
        engine.spawn_test_object(SpawnConfig::new("TARG").with_position(Vector2::new(20, 10)));
    let searcher =
        engine.spawn_test_object(SpawnConfig::new("SRCH").with_position(Vector2::new(0, 10)));
    let searcher = engine.test_object_index(searcher);

    HOST_WORLD_OBJECT_MATERIALIZATIONS.with(|count| count.set(0));
    assert_eq!(
        engine
            .call_object_function(searcher, "FindTarget", Vec::new())
            .expect("FindObject2 succeeds"),
        Value::Object(target.as_u64())
    );
    assert_eq!(
        HOST_WORLD_OBJECT_MATERIALIZATIONS.with(Cell::get),
        1,
        "only the executing-object snapshot is materialized"
    );
}

#[test]
fn criterion_callback_tree_rejects_scalar_prefix_without_materialization() {
    // C4FindObjectAnd::Check evaluates children in stored order and returns
    // immediately when a scalar owner child fails, before the later Func
    // child can run (oracle-src-pinned src/C4FindObject.cpp:390-410,
    // 653-662). The driver reads that same live object; it need not project
    // callback state for a predicate the scalar prefix already rejected.
    let mut engine = Engine::with_seed(0);
    for definition in [
        test_definition("TARG", "Target", "#strict\nprotected func Match() { return true; }\n"),
        test_definition("SRCH", "Searcher", "#strict\nprotected func FindTarget() { return FindObject2([50, -99], [60, \"Match\"]); }\n"),
    ] {
        engine.register_test_definition(definition);
    }
    for _ in 0..64 {
        engine.spawn_test_object(SpawnConfig::new("TARG").with_owner(0));
    }
    let searcher = engine.spawn_test_object(SpawnConfig::new("SRCH").with_owner(0));
    let searcher = engine.test_object_index(searcher);

    HOST_WORLD_OBJECT_MATERIALIZATIONS.with(|count| count.set(0));
    assert_eq!(
        engine
            .call_object_function(searcher, "FindTarget", Vec::new())
            .expect("FindObject2 callback tree succeeds"),
        Value::Nil
    );
    assert_eq!(
        HOST_WORLD_OBJECT_MATERIALIZATIONS.with(Cell::get),
        1,
        "only the executing object is materialized"
    );
}

#[test]
fn contact_callbacks_reuse_the_definition_action_library() {
    // C4Object::ContactCheck dispatches through the live C4Def and its ActMap
    // pointer; it does not copy the definition table per contacted direction
    // (oracle-src-pinned src/C4Movement.cpp:166-182).
    let mut definition = test_definition(
        "CALL",
        "Contact caller",
        "#strict\nprotected func ContactBottom() { SetVertex(0, 2, 4); return 0; }\n",
    );
    definition.set_contact_function_calls(true);
    let mut engine = Engine::with_seed(0);
    engine.register_test_definition(definition);
    let object = engine.spawn_test_object(SpawnConfig::new("CALL"));
    let index = engine.test_object_index(object);

    CONTACT_ACTION_LIBRARY_DEEP_CLONES.with(|count| count.set(0));
    PARTICLE_DEF_NAME_REBUILDS.with(|count| count.set(0));
    SET_VERTEX_DEFINITION_METADATA_DEEP_CLONES.with(|count| count.set(0));
    crate::TestValueExt::test_value(
        engine.dispatch_contact_callbacks(index, MovementContactDispatch::Direct(CNAT_BOTTOM)),
    );
    assert_eq!(
        CONTACT_ACTION_LIBRARY_DEEP_CLONES.with(Cell::get),
        0,
        "contact dispatch shares immutable definition metadata"
    );
    assert_eq!(
        PARTICLE_DEF_NAME_REBUILDS.with(Cell::get),
        0,
        "contact dispatch shares the immutable particle definition names"
    );
    assert_eq!(
        SET_VERTEX_DEFINITION_METADATA_DEEP_CLONES.with(Cell::get),
        0,
        "SetVertex reads only the definition shape fields it needs"
    );
}

#[test]
fn no_attach_action_reuses_the_definition_action_library() {
    // C4Object::DoMovement retains its live C4Def/ActMap pointer while
    // NoAttachAction runs (oracle-src-pinned src/C4Movement.cpp:463-470).
    let mut definition = test_definition("WALK", "Walker", "#strict\n");
    definition.set_shape_vertices(vec![ObjectVertex::new(0, 2).with_cnat(CNAT_BOTTOM)]);
    definition.configure_actions(
        Some("Walk".to_owned()),
        HashMap::from([
            (
                "Walk".to_owned(),
                ActionSpec::for_procedure("WALK").with_next("Walk"),
            ),
            (
                "Jump".to_owned(),
                ActionSpec::for_procedure("FLIGHT").with_next("Jump"),
            ),
        ]),
    );
    let mut engine = Engine::with_seed(0);
    engine.set_landscape(Landscape::flat(64, 60));
    engine.register_test_definition(definition);
    let object = engine.spawn_test_object(
        SpawnConfig::new("WALK")
            .with_position(Vector2::new(20, 10))
            .with_action(ActionState::new("Walk"))
            .with_mobile(true),
    );
    let index = engine.test_object_index(object);
    engine.objects[index].frame_t_attach = CNAT_BOTTOM;
    let definition_id = engine.objects[index].definition_id.clone();
    let action_library = crate::TestValueExt::test_value(engine.definitions.get(&definition_id))
        .action_library()
        .clone();

    NO_ATTACH_ACTION_LIBRARY_DEEP_CLONES.with(|count| count.set(0));
    CONTAINED_CALL_ACTION_LIBRARY_DEEP_CLONES.with(|count| count.set(0));
    SCRIPT_STATE_SNAPSHOT_MATERIALIZATIONS.with(|count| count.set(0));
    HOST_WORLD_CONTEXT_BASE_MATERIALIZATIONS.with(|count| count.set(0));
    crate::TestValueExt::test_value(engine.exec_object_movement(
        index,
        &action_library,
        &definition_id,
        &[],
    ));
    assert_eq!(
        NO_ATTACH_ACTION_LIBRARY_DEEP_CLONES.with(Cell::get),
        0,
        "NoAttachAction shares immutable definition metadata"
    );
    assert_eq!(
        CONTAINED_CALL_ACTION_LIBRARY_DEEP_CLONES.with(Cell::get),
        0,
        "OnActionJump lookup shares immutable definition metadata"
    );
    assert_eq!(
        SCRIPT_STATE_SNAPSHOT_MATERIALIZATIONS.with(Cell::get),
        0,
        "a missing OnActionJump needs no callback state snapshot"
    );
    assert_eq!(
        HOST_WORLD_CONTEXT_BASE_MATERIALIZATIONS.with(Cell::get),
        0,
        "a missing OnActionJump needs no callback host world"
    );
}

#[test]
fn walking_off_an_attachment_does_not_scan_solid_mask_definitions() {
    // C4Object::NoAttachAction sends DFA_WALK directly to ObjectActionJump;
    // only DFA_SCALE enters ObjectActionCornerScale and its ContactCheck
    // probes (oracle-src-pinned src/C4Object.cpp:4277-4307;
    // src/C4ObjectCom.cpp:167-217).
    let mut engine = Engine::with_seed(0);
    let mut texmap = crate::landscape::RuntimeTexMapState::default();
    texmap.set_default_material_entry("Vehicle", 2);
    let mut landscape = crate::TestValueExt::test_value(Landscape::new(64, vec![8; 64]));
    landscape.set_raster_state(crate::landscape::LandscapeRasterState::new(1, 0, texmap));
    engine.set_landscape(landscape);

    let mut masked = test_definition("MASK", "Masked", "");
    masked.set_solid_mask(Some(DefinitionTargetRect::new(0, 0, 1, 1, 0, 0)));
    engine.register_test_definition(masked);

    let mut walker = test_definition("WALK", "Walker", "");
    walker.set_shape_vertices(vec![ObjectVertex::new(0, 2).with_cnat(CNAT_BOTTOM)]);
    walker.configure_actions(
        Some("Walk".to_owned()),
        HashMap::from([
            (
                "Walk".to_owned(),
                ActionSpec::for_procedure("WALK").with_next("Walk"),
            ),
            (
                "Jump".to_owned(),
                ActionSpec::for_procedure("FLIGHT").with_next("Jump"),
            ),
        ]),
    );
    engine.register_test_definition(walker);
    let object = engine.spawn_test_object(
        SpawnConfig::new("WALK")
            .with_position(Vector2::new(20, 0))
            .with_action(ActionState::new("Walk"))
            .with_mobile(true),
    );
    let index = engine.test_object_index(object);
    engine.objects[index].frame_t_attach = CNAT_BOTTOM;
    let definition_id = engine.objects[index].definition_id.clone();
    let action_library = crate::TestValueExt::test_value(engine.definitions.get(&definition_id))
        .shared_action_library_handle();

    SOLID_MASK_DEFINITION_LOOKUPS.with(|count| count.set(0));
    crate::TestValueExt::test_value(engine.exec_mobile_object_movement(
        index,
        &action_library,
        &definition_id,
        &[],
    ));

    assert_eq!(engine.objects[index].state.action.name, "Jump");
    assert_eq!(
        SOLID_MASK_DEFINITION_LOOKUPS.with(Cell::get),
        0,
        "ordinary walking never enters the corner-scale contact probe"
    );
}

#[test]
fn corner_scale_rechecks_masks_changed_by_an_earlier_contact_callback() {
    // Each C4ObjectCom::CornerScaleOkay invokes a fresh C4Object::ContactCheck.
    // Its synchronous Contact* callback may SetSolidMask before the next
    // candidate is probed, and that later probe sees the changed landscape
    // (oracle-src-pinned src/C4ObjectCom.cpp:167-205;
    // src/C4Movement.cpp:166-182; src/C4Script.cpp:272-277;
    // src/C4Object.cpp:3809-3818).
    let mut engine = Engine::with_seed(0);
    let mut landscape = crate::TestValueExt::test_value(Landscape::new(64, vec![64; 64]));
    landscape.set_world_height(64);
    engine.set_landscape(landscape);
    engine.register_test_script_definition("MASK", "Mask target", "");

    let first_blocker =
        engine.spawn_test_object(SpawnConfig::new("MASK").with_position(Vector2::new(27, 13)));
    let later_blocker =
        engine.spawn_test_object(SpawnConfig::new("MASK").with_position(Vector2::new(27, 14)));
    let first_blocker_index = engine.test_object_index(first_blocker);
    engine.objects[first_blocker_index]
        .state
        .solid_mask_override = Some(DefinitionTargetRect::new(0, 0, 1, 1, 0, 0));
    let later_blocker_index = engine.test_object_index(later_blocker);
    engine.objects[later_blocker_index]
        .state
        .solid_mask_override = Some(DefinitionTargetRect::new(0, 0, 0, 0, 0, 0));

    let mut scaler = test_definition(
        "SCAL",
        "Scaler",
        r#"
        #strict 2
        local blocker, changed;

        protected func ContactTop()
        {
            if (!changed)
            {
                changed = 1;
                SetSolidMask(0, 0, 1, 1, 0, 0, blocker);
            }
            return 0;
        }
    "#,
    );
    scaler.set_c4_callback_convention(true);
    scaler.set_contact_function_calls(true);
    scaler.set_shape_vertices(vec![ObjectVertex::new(0, 0).with_cnat(CNAT_TOP)]);
    scaler.configure_actions(
        Some("Scale".to_owned()),
        HashMap::from([
            (
                "Scale".to_owned(),
                ActionSpec::for_procedure("SCALE").with_next("Scale"),
            ),
            (
                "Walk".to_owned(),
                ActionSpec::for_procedure("WALK").with_next("Walk"),
            ),
        ]),
    );
    engine.register_test_definition(scaler);
    let scaler = engine.spawn_test_object(
        SpawnConfig::new("SCAL")
            .with_position(Vector2::new(20, 20))
            .with_direction(Direction::Right)
            .with_action(ActionState::new("Scale"))
            .with_local_vars(HashMap::from([(
                "blocker".to_owned(),
                Value::Object(later_blocker.as_u64()),
            )])),
    );
    let scaler_index = engine.test_object_index(scaler);
    let definition_id = engine.objects[scaler_index].definition_id.clone();

    assert!(engine
        .object_action_corner_scale(scaler_index, &definition_id, ActionProcedure::Scale)
        .expect("corner scaling succeeds"));

    assert_eq!(
        engine.objects[scaler_index].state.position,
        Vector2::new(27, 15),
        "the newly enabled mask rejects the second corner candidate"
    );
    assert_eq!(
        engine.objects[later_blocker_index]
            .state
            .solid_mask_override,
        Some(DefinitionTargetRect::new(0, 0, 1, 1, 0, 0)),
        "the first probe callback enables the later blocker"
    );
}

#[test]
fn contact_callbacks_borrow_their_cached_world_object() {
    // C4Object::ContactCheck and the dispatched script callback both retain
    // the same live C4Object; entering a callback does not copy object state
    // (oracle-src-pinned src/C4Movement.cpp:166-182).
    let mut definition = test_definition(
        "CALL",
        "Contact caller",
        "#strict\nprotected func ContactBottom() { return 0; }\n",
    );
    definition.set_contact_function_calls(true);
    let mut engine = Engine::with_seed(0);
    engine.register_test_definition(definition);
    let object = engine.spawn_test_object(SpawnConfig::new("CALL"));
    let index = engine.test_object_index(object);

    HOST_WORLD_OBJECT_GET_DEEP_CLONES.with(|count| count.set(0));
    crate::TestValueExt::test_value(
        engine.dispatch_contact_callbacks(index, MovementContactDispatch::Direct(CNAT_BOTTOM)),
    );
    assert_eq!(
        HOST_WORLD_OBJECT_GET_DEEP_CLONES.with(Cell::get),
        0,
        "callback setup borrows the cached world object"
    );
}

#[test]
fn script_callback_snapshots_share_unchanged_local_variables() {
    // C++ callbacks retain one C4Object::Local array; taking a callback-entry
    // view does not copy every local before the VM mutates a cell
    // (oracle-src-pinned src/C4AulExec.cpp:343-352).
    let mut definition = test_definition(
        "CALL",
        "Contact caller",
        "#strict\nlocal value; protected func ContactBottom() { return value; }\n",
    );
    definition.set_contact_function_calls(true);
    let mut engine = Engine::with_seed(0);
    engine.register_test_definition(definition);
    let object = engine.spawn_test_object(
        SpawnConfig::new("CALL")
            .with_local_vars(HashMap::from([("value".to_string(), Value::Int(7))])),
    );
    let index = engine.test_object_index(object);

    SCRIPT_STATE_LOCAL_VAR_DEEP_CLONES.with(|count| count.set(0));
    crate::TestValueExt::test_value(
        engine.dispatch_contact_callbacks(index, MovementContactDispatch::Direct(CNAT_BOTTOM)),
    );
    assert_eq!(
        SCRIPT_STATE_LOCAL_VAR_DEEP_CLONES.with(Cell::get),
        0,
        "callback-entry snapshots share unchanged local storage"
    );
}

#[test]
fn later_foreign_effect_observes_carrier_local_commit_in_same_batch() {
    // C4Effect::Execute walks the one live chain, and each callback resolves
    // LocalN against the then-current C4Object locals. A later foreign
    // command target must therefore see an earlier self-target write
    // (oracle-src-pinned src/C4Effect.cpp:342-345;
    // src/C4AulExec.cpp:418-440).
    let script = r#"#strict
local value, seen;
func FxSetTimer() { value = 7; return 0; }
func FxReadTimer(target) { seen = LocalN("value", target); return 0; }
"#;
    let mut definition = Definition::from_script("CALL", "Effect local visibility fixture", script)
        .expect("definition compiles");
    definition.set_c4_callback_convention(true);
    let mut engine = Engine::with_seed(0);
    engine
        .register_definition(definition)
        .expect("definition registers");
    let foreign = engine
        .spawn_object(SpawnConfig::new("CALL"))
        .expect("foreign command target spawns");
    let carrier = engine
        .spawn_object(
            SpawnConfig::new("CALL")
                .with_local_vars(HashMap::from([("value".to_string(), Value::Int(0))]))
                .add_effect(EffectState::new("Set").with_priority(100).with_interval(1))
                .add_effect(EffectState::new("Read").with_priority(101).with_interval(1)),
        )
        .expect("carrier spawns");
    let carrier_index = engine.find_object_index(carrier).expect("carrier exists");
    engine.objects[carrier_index].state.effects[0].command_target =
        Some(i32::try_from(carrier.as_u64()).expect("carrier id fits C4 int"));
    engine.objects[carrier_index].state.effects[1].command_target =
        Some(i32::try_from(foreign.as_u64()).expect("foreign id fits C4 int"));
    let definition_id = engine.objects[carrier_index].definition_id.clone();
    let events = engine.objects[carrier_index]
        .state
        .effects
        .iter()
        .cloned()
        .map(EffectEvent::timer)
        .collect();

    engine
        .dispatch_object_effect_events(carrier_index, &definition_id, events)
        .expect("effect batch succeeds");

    let foreign_index = engine
        .find_object_index(foreign)
        .expect("foreign target remains");
    assert_eq!(
        engine.objects[foreign_index].state.local_vars.get("seen"),
        Some(&Value::Int(7))
    );
}

#[test]
fn later_self_effect_observes_foreign_write_to_carrier_in_same_batch() {
    // C4Effect::Execute walks one live chain, while LocalN writes through the
    // carrier's one live C4Object before the next callback resolves its locals
    // (oracle-src-pinned src/C4Effect.cpp:342-345;
    // src/C4AulExec.cpp:418-440).
    let script = r#"#strict
local value, seen;
func FxWriteTimer(target) { LocalN("value", target) = 7; return 0; }
func FxReadTimer() { seen = value; return 0; }
"#;
    let mut definition =
        Definition::from_script("CALL", "Effect foreign local write fixture", script)
            .expect("definition compiles");
    definition.set_c4_callback_convention(true);
    let mut engine = Engine::with_seed(0);
    engine
        .register_definition(definition)
        .expect("definition registers");
    let foreign = engine
        .spawn_object(SpawnConfig::new("CALL"))
        .expect("foreign command target spawns");
    let carrier = engine
        .spawn_object(
            SpawnConfig::new("CALL")
                .with_local_vars(HashMap::from([("value".to_string(), Value::Int(0))]))
                .add_effect(
                    EffectState::new("Write")
                        .with_priority(100)
                        .with_interval(1),
                )
                .add_effect(EffectState::new("Read").with_priority(101).with_interval(1)),
        )
        .expect("carrier spawns");
    let carrier_index = engine.find_object_index(carrier).expect("carrier exists");
    engine.objects[carrier_index].state.effects[0].command_target =
        Some(i32::try_from(foreign.as_u64()).expect("foreign id fits C4 int"));
    engine.objects[carrier_index].state.effects[1].command_target =
        Some(i32::try_from(carrier.as_u64()).expect("carrier id fits C4 int"));
    let definition_id = engine.objects[carrier_index].definition_id.clone();
    let events = engine.objects[carrier_index]
        .state
        .effects
        .iter()
        .cloned()
        .map(EffectEvent::timer)
        .collect();

    engine
        .dispatch_object_effect_events(carrier_index, &definition_id, events)
        .expect("effect batch succeeds");

    assert_eq!(
        engine.objects[carrier_index].state.local_vars.get("seen"),
        Some(&Value::Int(7))
    );
}

#[test]
fn action_transitions_reuse_the_definition_action_library() {
    // C4Object::SetActionByName resolves and applies entries through the
    // definition's live ActMap pointer, without copying that table
    // (oracle-src-pinned src/C4Object.cpp:4144-4228).
    let mut definition = test_definition("CALL", "Action caller", "#strict\n");
    definition.configure_actions(
        Some("Walk".to_string()),
        HashMap::from([("Walk".to_string(), ActionSpec::default())]),
    );
    let mut engine = Engine::with_seed(0);
    engine.register_test_definition(definition);
    let object =
        engine.spawn_test_object(SpawnConfig::new("CALL").with_action(ActionState::new("Walk")));
    let index = engine.test_object_index(object);
    let definition_id = engine.objects[index].definition_id.clone();

    ACTION_TRANSITION_ACTION_LIBRARY_DEEP_CLONES.with(|count| count.set(0));
    assert!(engine
        .action_with_calls(index, &definition_id, "Idle")
        .expect("action transition succeeds"));
    assert_eq!(
        ACTION_TRANSITION_ACTION_LIBRARY_DEEP_CLONES.with(Cell::get),
        0,
        "action transition shares immutable definition metadata"
    );
}

#[test]
fn effect_timers_reuse_the_definition_reflection_table() {
    // C4Effect::Execute resolves the live C4Def carried by its object and
    // passes that pointer through the callback; it does not copy DefCore
    // reflection data per timer (oracle-src-pinned src/C4Effect.cpp:342-345).
    let mut definition = test_definition(
        "CALL",
        "Effect caller",
        "#strict\nfunc FxLoadTimer() { return 0; }\n",
    );
    definition.set_c4_callback_convention(true);
    let mut engine = Engine::with_seed(0);
    engine.register_test_definition(definition);
    let object = engine.spawn_test_object(SpawnConfig::new("CALL"));
    let index = engine.test_object_index(object);
    let definition_id = engine.objects[index].definition_id.clone();
    let mut effect = EffectState::new("Load")
        .with_interval(1)
        .with_command_id(Some("CALL"));
    effect.number = 1;
    effect.command_target = Some(object.as_u64() as i32);
    engine.objects[index].state.effects.push(effect.clone());

    EFFECT_DEF_CORE_VALUE_DEEP_CLONES.with(|count| count.set(0));
    crate::TestValueExt::test_value(engine.dispatch_object_effect_events(
        index,
        &definition_id,
        vec![EffectEvent::timer(effect)],
    ));
    assert_eq!(
        EFFECT_DEF_CORE_VALUE_DEEP_CLONES.with(Cell::get),
        0,
        "effect dispatch shares immutable DefCore reflection data"
    );
}

#[test]
fn lazy_host_world_global_effect_without_world_access_copies_nothing() {
    use std::sync::Mutex;

    let calls = Arc::new(Mutex::new(0usize));
    let mut hooks = DebuggerHooks::new();
    {
        let calls = Arc::clone(&calls);
        hooks.set_on_call(move |name, _| {
            if name == "FxLazyTimer" {
                *crate::TestValueExt::test_value(calls.lock()) += 1;
            }
        });
    }
    let mut definition = test_definition(
        "GFXL",
        "Global lazy effect",
        "#strict\nfunc FxLazyTimer(object target, int number, int time) { return(0); }",
    );
    definition.set_c4_callback_convention(true);
    definition.set_debugger_hooks(hooks);
    let mut engine = Engine::with_seed(0);
    engine.register_test_definition(definition);
    engine.register_test_script_definition("FILL", "Filler", "#strict\n");
    for x in 0..64 {
        engine.spawn_test_object(SpawnConfig::new("FILL").with_position(Vector2::new(x, 10)));
    }
    let mut effect = EffectState::new("Lazy")
        .with_interval(1)
        .with_command_id(Some("GFXL"));
    effect.number = 1;
    engine.global_effects.push(effect);

    HOST_WORLD_OBJECT_MATERIALIZATIONS.with(|count| count.set(0));
    HOST_WORLD_LANDSCAPE_MATERIALIZATIONS.with(|count| count.set(0));
    crate::TestValueExt::test_value(engine.tick_global_effects());
    assert_eq!(*calls.lock().expect("call counter"), 1);
    assert_eq!(
        HOST_WORLD_OBJECT_MATERIALIZATIONS.with(Cell::get),
        0,
        "a nil-target global callback copies no object"
    );
    assert_eq!(HOST_WORLD_LANDSCAPE_MATERIALIZATIONS.with(Cell::get), 0,);
}

#[test]
fn lazy_host_world_contact_materialization_is_deferred_until_query() {
    let mut engine = Engine::with_seed(0);
    let mut landscape = crate::TestValueExt::test_value(Landscape::with_default_material(
        100,
        vec![100; 100],
        None,
    ));
    landscape.set_world_height(100);
    landscape.set_pixel_grid(PixelGrid::new(
        100,
        100,
        vec![0; 100 * 100],
        vec![0, 100],
        vec![None, Some("Earth".to_owned())],
        vec![None; 2],
    ));
    engine.set_landscape(landscape);

    engine.register_test_script_definition("FILL", "Filler", "#strict\n");
    let mut swimmer = test_definition(
        "SWIM",
        "Contact swimmer",
        r#"#strict
    local contact_calls, world_count, wall_seen;
    protected func ContactRight()
    {
        contact_calls++;
        world_count = ObjectCount();
        wall_seen = GBackSolid(2, 0);
        return(0);
    }
    "#,
    );
    swimmer.set_c4_callback_convention(true);
    swimmer.set_contact_function_calls(true);
    swimmer.set_shape_rect(Some(DefinitionRect::new(-1, -1, 2, 2)));
    swimmer.set_shape_vertices(vec![ObjectVertex::new(1, 0).with_cnat(CNAT_RIGHT)]);
    engine.register_test_definition(swimmer);

    for x in 0..128 {
        engine.spawn_test_object(SpawnConfig::new("FILL").with_position(Vector2::new(x % 100, 10)));
    }
    let swimmer = engine.spawn_test_object(
        SpawnConfig::new("SWIM")
            .with_position(Vector2::new(50, 50))
            .with_velocity(Vector2::new(1, 0))
            .with_mobile(true),
    );
    let swimmer_index = engine.test_object_index(swimmer);
    let definition_id = engine.objects[swimmer_index].definition_id.clone();
    let action_library = crate::TestValueExt::test_value(engine.definitions.get(&definition_id))
        .action_library()
        .clone();
    let solid_mask_indices = engine.active_solid_mask_indices();

    HOST_WORLD_OBJECT_MATERIALIZATIONS.with(|count| count.set(0));
    HOST_WORLD_LANDSCAPE_MATERIALIZATIONS.with(|count| count.set(0));
    crate::TestValueExt::test_value(engine.exec_object_movement(
        swimmer_index,
        &action_library,
        &definition_id,
        &solid_mask_indices,
    ));
    assert_eq!(
        HOST_WORLD_OBJECT_MATERIALIZATIONS.with(Cell::get),
        0,
        "free movement must not snapshot the world merely because ContactCalls=1"
    );
    assert_eq!(
        HOST_WORLD_LANDSCAPE_MATERIALIZATIONS.with(Cell::get),
        0,
        "free movement must not clone terrain merely because ContactCalls=1"
    );

    crate::TestValueExt::test_value(engine.landscape.as_mut()).grid_write_byte(54, 50, 1);
    // Cross one free pixel first so DoMotion mutably reborrows the
    // non-mover slices before the same movement reaches ContactRight.
    engine.objects[swimmer_index].set_fixed_velocity(FixedVec2::from_ints(2, 0));
    HOST_WORLD_OBJECT_MATERIALIZATIONS.with(|count| count.set(0));
    HOST_WORLD_LANDSCAPE_MATERIALIZATIONS.with(|count| count.set(0));
    crate::TestValueExt::test_value(engine.exec_object_movement(
        swimmer_index,
        &action_library,
        &definition_id,
        &solid_mask_indices,
    ));
    assert_eq!(
        HOST_WORLD_OBJECT_MATERIALIZATIONS.with(Cell::get),
        1,
        "Contact*'s scalar ObjectCount retains only the executing-object snapshot"
    );
    assert_eq!(
        HOST_WORLD_LANDSCAPE_MATERIALIZATIONS.with(Cell::get),
        0,
        "Contact*'s GBackSolid borrows the movement landscape instead of copying it"
    );
    assert_eq!(
        engine.objects[swimmer_index]
            .state
            .local_vars
            .get("contact_calls"),
        Some(&Value::Int(1)),
        "the deferred world preserves Contact* callback execution"
    );
    assert_eq!(
        engine.objects[swimmer_index]
            .state
            .local_vars
            .get("world_count"),
        Some(&Value::Int(engine.objects.len() as i32 - 1)),
        "ObjectCount sees every other live object (C++ excludes the caller)"
    );
    assert_eq!(
        engine.objects[swimmer_index]
            .state
            .local_vars
            .get("wall_seen"),
        Some(&Value::Bool(true)),
        "the deferred callback sees the contact-time landscape"
    );
}

#[test]
fn rejected_step_latches_contact_for_next_move_to_jump() {
    let mut engine = Engine::with_seed(0);
    let mut landscape = crate::TestValueExt::test_value(Landscape::with_default_material(
        200,
        vec![200; 200],
        None,
    ));
    landscape.set_world_height(200);
    let mut pixels = vec![0; 200 * 200];
    // The current left vertex is free at (96,103), but the candidate
    // one-pixel-left step reaches this solid pixel at (95,103).
    pixels[103 * 200 + 95] = 1;
    landscape.set_pixel_grid(PixelGrid::new(
        200,
        200,
        pixels,
        vec![0, 100],
        vec![None, Some("Earth".to_owned())],
        vec![None; 2],
    ));
    engine.set_landscape(landscape);

    let mut walker_definition = test_definition("WALK", "Walker", "");
    let mut actions = HashMap::new();
    actions.insert(
        "Walk".to_owned(),
        ActionSpec::default().with_procedure("Walk"),
    );
    actions.insert(
        "Jump".to_owned(),
        ActionSpec::default().with_procedure("Flight"),
    );
    walker_definition.configure_actions(Some("Walk".to_owned()), actions);
    walker_definition.set_crew_member(true);
    walker_definition.set_shape_rect(Some(DefinitionRect::new(-4, -9, 8, 18)));
    walker_definition.set_shape_vertices(vec![ObjectVertex {
        x: -4,
        y: 3,
        cnat: CNAT_LEFT,
        friction: 100,
    }]);
    engine.register_test_definition(walker_definition);

    let walker = engine.spawn_test_object(
        SpawnConfig::new("WALK")
            .with_position(Vector2::new(100, 100))
            .with_velocity(Vector2::new(-1, 0))
            .with_action(ActionState::new("Walk"))
            .with_category(CATEGORY_OBJECT | CATEGORY_LIVING)
            .with_crew_member(true)
            .with_alive(true)
            .with_mobile(true),
    );
    let walker_index = engine.test_object_index(walker);
    engine.refresh_object_ocf(walker_index);
    assert_ne!(
        engine.objects[walker_index].state.ocf & ocf::CREW_MEMBER,
        0,
        "the fixture participates in C4Command JumpControl"
    );
    let solid_mask_indices = engine.active_solid_mask_indices();
    let definition_id = engine.objects[walker_index].definition_id.clone();
    let action_library = crate::TestValueExt::test_value(engine.definitions.get(&definition_id))
        .action_library()
        .clone();

    crate::TestValueExt::test_value(engine.exec_object_movement(
        walker_index,
        &action_library,
        &definition_id,
        &solid_mask_indices,
    ));

    assert_eq!(
        engine.live_command_snapshot(walker_index, None).contact & CNAT_LEFT,
        CNAT_LEFT,
        "the rejected candidate step latches C4Object::t_contact"
    );

    crate::TestValueExt::test_value(
        engine.objects[walker_index].commands.push_front(
            CommandRequest::new(CommandId::MoveTo)
                .with_tx(Some(60))
                .with_ty(Some(93))
                // C4CMD_MoveTo_NoPosAdjust keeps the low-side target.
                .with_data(CommandData::Integer(1)),
        ),
    );
    crate::TestValueExt::test_value(engine.execute_object_command_now(walker));
    crate::TestValueExt::test_value(engine.execute_object_command_now(walker));

    assert_eq!(
        engine.objects[walker_index]
            .commands
            .snapshot()
            .command_names()
            .first()
            .map(String::as_str),
        Some("Jump"),
        "JumpControl reacts to the rejected step's latched CNAT_Left on the next command frame"
    );
}

#[test]
fn sector_queries_do_not_materialize_the_landscape_shell() {
    // C4LSectors only ever needs the landscape's extent to size its grid
    // (oracle-src-pinned src/C4Sector.cpp:107 reads Game.Landscape.Width/Height
    // and nothing else). Forcing the callback-local landscape copy for two
    // integers costs a full deep clone on the first FindObjects of every
    // script call, which is the hot path while many flames are alive. A
    // bounded C4FindObject walk reads the existing Game.Objects.Sectors lists
    // directly (oracle-src-pinned src/C4FindObject.cpp:315-355), so it must
    // not first clone every object into a second callback-local sector map.
    let mut engine = Engine::with_seed(0);
    engine.register_test_script_definition("SECT", "Sector", "");
    engine.set_landscape(Landscape::flat(100, 100));
    for x in 0..8 {
        engine.spawn_test_object(SpawnConfig::new("SECT").with_position(Vector2::new(x * 8, 10)));
    }

    HOST_WORLD_OBJECT_MATERIALIZATIONS.with(|count| count.set(0));
    HOST_WORLD_LANDSCAPE_MATERIALIZATIONS.with(|count| count.set(0));
    let context = engine.host_world_context();
    let found = crate::TestValueExt::test_value(
        context.object_sector_ids_in_rect(DefinitionRect::new(0, 0, 100, 100)),
    );

    assert_eq!(
        found.len(),
        8,
        "every spawned object is inside the query rect"
    );
    assert_eq!(
        HOST_WORLD_LANDSCAPE_MATERIALIZATIONS.with(Cell::get),
        0,
        "sector sizing must not clone the landscape shell"
    );
    assert_eq!(
        HOST_WORLD_OBJECT_MATERIALIZATIONS.with(Cell::get),
        0,
        "a read-only sector walk must borrow the authoritative candidate lists"
    );
}

#[test]
fn sector_query_rebuilds_after_callback_local_position_update() {
    // C4Object::ForcePosition immediately calls UpdatePos, which reinserts the
    // object in Game.Objects.Sectors before the next script query
    // (oracle-src-pinned src/C4Movement.cpp:536-545;
    // src/C4Object.cpp:346-354). A callback-local preview must therefore stop
    // borrowing the callback-entry sector lists as soon as position changes.
    let mut engine = Engine::with_seed(0);
    engine.register_test_script_definition("SECT", "Sector", "");
    engine.set_landscape(Landscape::flat(400, 100));
    let object =
        engine.spawn_test_object(SpawnConfig::new("SECT").with_position(Vector2::new(10, 10)));

    let mut context = engine.host_world_context();
    assert!(
        context
            .object_sector_ids_in_rect(DefinitionRect::new(0, 0, 50, 100))
            .expect("sector map present")
            .contains(&object),
        "the callback-entry sectors contain the object's old position"
    );

    context.preview_object_update(
        object,
        &ObjectUpdate::new().with_position(Vector2::new(310, 10)),
    );

    assert!(
        !context
            .object_sector_ids_in_rect(DefinitionRect::new(0, 0, 50, 100))
            .expect("sector map present")
            .contains(&object),
        "the rebuilt sectors drop the object's old position"
    );
    assert!(
        context
            .object_sector_ids_in_rect(DefinitionRect::new(300, 0, 50, 100))
            .expect("sector map present")
            .contains(&object),
        "the rebuilt sectors expose the callback-local position"
    );
}

#[test]
fn set_position_updates_bounded_find_sectors_in_the_same_script_call() {
    // C4Object::ForcePosition calls UpdatePos before returning, so the next
    // bounded FindObjects in the same script call walks the new sector links
    // (oracle-src-pinned src/C4Movement.cpp:536-545;
    // src/C4FindObject.cpp:315-355).
    let mut engine = Engine::with_seed(0);
    engine.set_landscape(Landscape::flat(400, 100));
    engine.register_test_definition(test_definition(
        "SECT",
        "Sector mover",
        r#"#strict
        public func MoveAndFind()
        {
            SetPosition(310, 10);
            return GetLength(FindObjects([C4FO_InRect, 300, 0, 50, 100]));
        }
        "#,
    ));
    let object =
        engine.spawn_test_object(SpawnConfig::new("SECT").with_position(Vector2::new(10, 10)));
    let index = engine.test_object_index(object);

    assert_eq!(
        engine
            .call_object_function(index, "MoveAndFind", Vec::new())
            .expect("move-and-find callback succeeds"),
        Value::Int(1),
        "the moved object must be discoverable through its new sector"
    );
}

#[test]
fn set_rotation_updates_bounded_shape_find_sectors_in_the_same_script_call() {
    // C4Object::SetRotation calls UpdateFace(true), whose UpdateShape updates
    // the covered sector area before returning to script
    // (oracle-src-pinned src/C4Object.cpp:322-365,5637-5647;
    // src/C4FindObject.cpp:315-355).
    let mut engine = Engine::with_seed(0);
    engine.set_landscape(Landscape::flat(400, 200));
    let mut definition = test_definition(
        "ROTR",
        "Sector rotator",
        r#"#strict
    public func RotateAndFind()
    {
        SetR(90);
        return GetLength(FindObjects([C4FO_AtRect, 260, 100, 1, 1]));
    }
    "#,
    );
    definition.set_rotateable(1);
    definition.set_shape_rect(Some(DefinitionRect::new(-80, 0, 80, 10)));
    engine.register_test_definition(definition);
    let object =
        engine.spawn_test_object(SpawnConfig::new("ROTR").with_position(Vector2::new(200, 100)));
    let index = engine.test_object_index(object);

    assert_eq!(
        engine
            .call_object_function(index, "RotateAndFind", Vec::new())
            .expect("rotate-and-find callback succeeds"),
        Value::Int(1),
        "the rotated shape must be discoverable through its new sectors"
    );
}

#[test]
fn change_def_updates_bounded_shape_find_sectors_in_the_same_script_call() {
    // C4Object::ChangeDef installs the new definition and calls
    // UpdateFace(true), so its new shape-sector links are visible before the
    // suspended script frame resumes (oracle-src-pinned
    // src/C4Object.cpp:1207-1254,322-365;
    // src/C4FindObject.cpp:315-355).
    let mut engine = Engine::with_seed(0);
    engine.set_landscape(Landscape::flat(400, 200));
    let mut replacement = test_definition("NEW1", "Wide replacement", "#strict\n");
    replacement.set_shape_rect(Some(DefinitionRect::new(-80, 0, 80, 10)));
    engine.register_test_definition(replacement);
    let mut original = test_definition(
        "OLD1",
        "Narrow original",
        r#"#strict
    public func ChangeAndFind()
    {
        ChangeDef(NEW1);
        return GetLength(FindObjects([C4FO_AtRect, 130, 100, 1, 1]));
    }
    "#,
    );
    original.set_shape_rect(Some(DefinitionRect::new(0, 0, 1, 1)));
    engine.register_test_definition(original);
    let object =
        engine.spawn_test_object(SpawnConfig::new("OLD1").with_position(Vector2::new(200, 100)));
    let index = engine.test_object_index(object);

    assert_eq!(
        engine
            .call_object_function(index, "ChangeAndFind", Vec::new())
            .expect("change-and-find callback succeeds"),
        Value::Int(1),
        "the replacement shape must be discoverable through its new sectors"
    );
}

#[test]
fn do_con_updates_bounded_shape_find_sectors_after_keep_bottom_move() {
    // C4Object::DoCon first refreshes the shape through UpdateFace(true),
    // then calls UpdatePos again after moving a straight object's center to
    // keep its old bottom (oracle-src-pinned src/C4Object.cpp:1445-1505;
    // src/C4FindObject.cpp:315-355).
    let mut engine = Engine::with_seed(0);
    engine.set_landscape(Landscape::flat(400, 250));
    let mut definition = test_definition(
        "GROW",
        "Sector grower",
        r#"#strict
    public func GrowAndFind()
    {
        DoCon(50);
        return GetLength(FindObjects([C4FO_AtRect, 200, 30, 1, 1]));
    }

    public func FindAtNewTop()
    {
        return GetLength(FindObjects([C4FO_AtRect, 200, 30, 1, 1]));
    }
    "#,
    );
    definition.set_shape_rect(Some(DefinitionRect::new(0, 0, 10, 80)));
    engine.register_test_definition(definition);
    let object = engine.spawn_test_object(
        SpawnConfig::new("GROW")
            .with_position(Vector2::new(200, 100))
            .with_construction(FULL_CON / 2),
    );
    let index = engine.test_object_index(object);

    assert_eq!(
        engine
            .call_object_function(index, "FindAtNewTop", Vec::new())
            .expect("pre-growth find callback succeeds"),
        Value::Int(0),
        "the incomplete shape must not cover its eventual full-construction top"
    );
    assert_eq!(
        engine
            .call_object_function(index, "GrowAndFind", Vec::new())
            .expect("grow-and-find callback succeeds"),
        Value::Int(1),
        "the grown object must be discoverable at its keep-bottom position"
    );
}

#[test]
fn do_con_refreshes_shape_sectors_before_content_ejection_callback() {
    // UpdateFace(true) performs UpdateShape -> UpdatePos before DoCon ejects
    // incomplete objects' contents, so Ejection observes the intermediate
    // grown shape before the later keep-bottom move
    // (oracle-src-pinned src/C4Object.cpp:1445-1505;
    // src/C4FindObject.cpp:315-355).
    let mut engine = Engine::with_seed(0);
    engine.set_landscape(Landscape::flat(400, 250));
    let mut grower = test_definition(
        "GROW",
        "Callback grower",
        r#"#strict
    local ejection_count;

    protected func Ejection(object child)
    {
        ejection_count = GetLength(FindObjects(
    [C4FO_AtRect, 200, 110, 1, 1],
    [C4FO_ID, GROW]));
    }

    public func GrowAndReadEjection()
    {
        DoCon(25);
        return ejection_count;
    }
    "#,
    );
    grower.set_c4_callback_convention(true);
    grower.set_shape_rect(Some(DefinitionRect::new(0, 0, 10, 80)));
    engine.register_test_definition(grower);
    engine.register_test_script_definition("ITEM", "Contained item", "#strict\n");
    let object = engine.spawn_test_object(
        SpawnConfig::new("GROW")
            .with_position(Vector2::new(200, 100))
            .with_construction(FULL_CON / 4),
    );
    engine.spawn_test_object(SpawnConfig::new("ITEM").with_container(object));
    let index = engine.test_object_index(object);

    assert_eq!(
        engine
            .call_object_function(index, "GrowAndReadEjection", Vec::new())
            .expect("grow-and-read callback succeeds"),
        Value::Int(1),
        "Ejection must see the UpdateFace shape sectors before keep-bottom adjustment"
    );
}

#[test]
fn exit_updates_sectors_before_ejection_callback() {
    // C4Object::Exit installs its new position and runs UpdateFace(true) ->
    // UpdatePos before Ejection/Departure, so the container's Ejection
    // callback finds the exiting object at its outside coordinates
    // (oracle-src-pinned src/C4Object.cpp:1532-1563;
    // src/C4FindObject.cpp:315-355).
    let mut engine = Engine::with_seed(0);
    engine.set_landscape(Landscape::flat(400, 100));
    let mut container = test_definition(
        "CONT",
        "Exit observer",
        r#"#strict
    local ejection_count;

    protected func Ejection(object item)
    {
        ejection_count = GetLength(FindObjects(
    [C4FO_InRect, 300, 0, 50, 100],
    [C4FO_ID, ITEM]));
    }
    "#,
    );
    container.set_c4_callback_convention(true);
    engine.register_test_definition(container);
    engine.register_test_definition(test_definition(
        "ITEM",
        "Exiting item",
        r#"#strict
        public func Leave()
        {
            return Exit(this(), 310, 10);
        }
        "#,
    ));
    let container = engine.spawn_test_object(SpawnConfig::new("CONT"));
    let item = engine.spawn_test_object(SpawnConfig::new("ITEM").with_container(container));
    let item_index = engine.test_object_index(item);

    assert_eq!(
        engine
            .call_object_function(item_index, "Leave", Vec::new())
            .expect("exit callback succeeds"),
        Value::Bool(true)
    );
    assert_eq!(
        engine
            .object_snapshot(container)
            .expect("container remains")
            .local_vars
            .get("ejection_count"),
        Some(&Value::Int(1)),
        "Ejection must see the child's post-Exit sector"
    );
}

#[test]
fn enter_updates_sectors_before_collection2_callback() {
    // C4Object::Enter copies the container motion and runs UpdateFace(true)
    // before Collection2/Entrance, so Collection2 finds the newly contained
    // object at the container's coordinates (oracle-src-pinned
    // src/C4Object.cpp:1566-1636; src/C4FindObject.cpp:315-355).
    let mut engine = Engine::with_seed(0);
    engine.set_landscape(Landscape::flat(400, 100));
    let mut container = test_definition(
        "CONT",
        "Enter observer",
        r#"#strict
    local collection_count;

    protected func Collection2(object item)
    {
        collection_count = GetLength(FindObjects(
    [C4FO_InRect, 0, 0, 50, 100],
    [C4FO_ID, ITEM]));
    }
    "#,
    );
    container.set_c4_callback_convention(true);
    engine.register_test_definition(container);
    engine.register_test_definition(test_definition(
        "ITEM",
        "Entering item",
        r#"#strict
        public func Go(object container)
        {
            return Enter(container);
        }
        "#,
    ));
    let container =
        engine.spawn_test_object(SpawnConfig::new("CONT").with_position(Vector2::new(10, 10)));
    let item =
        engine.spawn_test_object(SpawnConfig::new("ITEM").with_position(Vector2::new(310, 10)));
    let item_index = engine.test_object_index(item);

    assert_eq!(
        engine
            .call_object_function(item_index, "Go", vec![object_reference_value(container)],)
            .expect("enter callback succeeds"),
        Value::Bool(true)
    );
    assert_eq!(
        engine
            .object_snapshot(container)
            .expect("container remains")
            .local_vars
            .get("collection_count"),
        Some(&Value::Int(1)),
        "Collection2 must see the item's post-Enter sector"
    );
}

#[test]
fn set_vertex_permanent_update_refreshes_shape_sectors() {
    // VTX_SetPermanentUpd runs C4Object::UpdateShape(true), which restores
    // the definition rectangle as well as the edited own vertices and calls
    // UpdatePos before SetVertex returns (oracle-src-pinned
    // src/C4Script.cpp:1238-1275; src/C4Object.cpp:322-350;
    // src/C4Shape.cpp:421-450; src/C4FindObject.cpp:315-355).
    let mut engine = Engine::with_seed(0);
    engine.set_landscape(Landscape::flat(400, 100));
    let mut definition = test_definition(
        "VRTX",
        "Permanent vertex editor",
        r#"#strict
    public func RestoreAndFind()
    {
        SetShape(0, 0, 1, 1);
        SetVertex(0, VTX_X, 0, this(), VTX_SetPermanentUpd);
        return [GetLength(FindObjects([C4FO_AtRect, 130, 5, 1, 1])), GetObjWidth()];
    }
    "#,
    );
    definition.set_shape_rect(Some(DefinitionRect::new(-80, 0, 80, 10)));
    definition.set_shape_vertices(vec![ObjectVertex::new(0, 0)]);
    engine.register_test_definition(definition);
    let object =
        engine.spawn_test_object(SpawnConfig::new("VRTX").with_position(Vector2::new(200, 10)));
    let index = engine.test_object_index(object);

    assert_eq!(
        engine
            .call_object_function(index, "RestoreAndFind", Vec::new())
            .expect("restore-and-find callback succeeds"),
        Value::Array(vec![Value::Int(1), Value::Int(80)]),
        "the permanent UpdateShape must restore and relink the definition rectangle"
    );
}

#[test]
fn collect_updates_sectors_after_post_callback_copy_motion() {
    // C4Object::Collect deliberately enters with fCopyMotion=false, runs
    // Collection/Hit at the item's old position, and only then CopyMotion
    // calls UpdatePos when it snaps to the collector (oracle-src-pinned
    // src/C4Object.cpp:5693-5714; src/C4Movement.cpp:518-529;
    // src/C4FindObject.cpp:315-355).
    let mut engine = Engine::with_seed(0);
    engine.set_landscape(Landscape::flat(400, 100));
    let mut collector = test_definition(
        "COLL",
        "Sector collector",
        r#"#strict
    public func TakeAndFind(object item)
    {
        if (!Collect(item)) return -1;
        return GetLength(FindObjects(
    [C4FO_InRect, 0, 0, 50, 100],
    [C4FO_ID, ITEM]));
    }
    "#,
    );
    collector.set_collection_rect(Some(DefinitionRect::new(-5, -5, 10, 10)));
    engine.register_test_definition(collector);
    let mut item = test_definition("ITEM", "Collected item", "#strict\n");
    item.set_collectible(true);
    engine.register_test_definition(item);
    let collector =
        engine.spawn_test_object(SpawnConfig::new("COLL").with_position(Vector2::new(10, 10)));
    let item =
        engine.spawn_test_object(SpawnConfig::new("ITEM").with_position(Vector2::new(310, 10)));
    let collector_index = engine.test_object_index(collector);

    assert_eq!(
        engine
            .call_object_function(
                collector_index,
                "TakeAndFind",
                vec![object_reference_value(item)],
            )
            .expect("collect-and-find callback succeeds"),
        Value::Int(1),
        "the collected item must be discoverable at its post-callback CopyMotion position"
    );
}

#[test]
fn status_activation_refreshes_sectors_after_shape_rebuild() {
    // StatusActivate first adds the inactive object with its current shape,
    // then UpdateFace(true) rebuilds Shape from the definition and UpdatePos
    // relinks that final geometry before UpdateTransferZone
    // (oracle-src-pinned src/C4Object.cpp:5972-5985,322-365;
    // src/C4FindObject.cpp:315-355).
    let mut engine = Engine::with_seed(0);
    engine.set_landscape(Landscape::flat(400, 100));
    let mut definition = test_definition(
        "ACTV",
        "Sector activator",
        r#"#strict
    public func ActivateAndFind()
    {
        SetObjectStatus(C4OS_NORMAL);
        return [GetLength(FindObjects([C4FO_AtRect, 130, 5, 1, 1])), GetObjWidth()];
    }
    "#,
    );
    definition.set_shape_rect(Some(DefinitionRect::new(-80, 0, 80, 10)));
    engine.register_test_definition(definition);
    let object = engine.spawn_test_object(
        SpawnConfig::new("ACTV")
            .with_position(Vector2::new(200, 10))
            .with_shape_rect(DefinitionRect::new(0, 0, 1, 1))
            .with_status(ObjectStatus::Inactive),
    );
    let index = engine.test_object_index(object);

    assert_eq!(
        engine
            .call_object_function(index, "ActivateAndFind", Vec::new())
            .expect("activate-and-find callback succeeds"),
        Value::Array(vec![Value::Int(1), Value::Int(80)]),
        "activation must relink the definition shape after clearing the inactive override"
    );
}

#[test]
fn callback_geometry_update_preserves_unrelated_physical_sector_order() {
    // C4LSectors::Update mutates only the moved object's affected lists; a
    // preceding master-list rank refresh does not reorder unrelated physical
    // sector links (oracle-src-pinned src/C4Sector.cpp:107-147;
    // src/C4GameObjects.cpp:732-736). A bounded FindObjects after SetPosition
    // must therefore retain the callback-entry order of the untouched sector.
    let mut engine = Engine::with_seed(0);
    engine.set_landscape(Landscape::flat(400, 100));
    engine.register_test_script_definition("ORDR", "Ordered candidate", "#strict\n");
    engine.register_test_definition(test_definition(
        "MOVR",
        "Sector mover",
        r#"#strict
        public func MoveAndFindOrdered()
        {
            SetPosition(310, 10);
            return FindObjects([C4FO_InRect, 0, 0, 50, 100]);
        }
        "#,
    ));
    let older =
        engine.spawn_test_object(SpawnConfig::new("ORDR").with_position(Vector2::new(10, 10)));
    let newer =
        engine.spawn_test_object(SpawnConfig::new("ORDR").with_position(Vector2::new(20, 10)));
    for x in 100..116 {
        engine.spawn_test_object(SpawnConfig::new("ORDR").with_position(Vector2::new(x, 10)));
    }
    let mover =
        engine.spawn_test_object(SpawnConfig::new("MOVR").with_position(Vector2::new(210, 10)));

    let older_index = engine.test_object_index(older);
    let newer_index = engine.test_object_index(newer);
    engine.objects[older_index].state.category = CATEGORY_OBJECT;
    engine.objects[newer_index].state.category = CATEGORY_STRUCTURE;
    engine
        .pending_object_order_commands
        .push(ObjectOrderCommand::SortByCategory);
    engine.execute_object_order_commands();

    HOST_WORLD_OBJECT_MATERIALIZATIONS.with(|count| count.set(0));
    let mover_index = engine.test_object_index(mover);
    assert_eq!(
        engine
            .call_object_function(mover_index, "MoveAndFindOrdered", Vec::new())
            .expect("ordered move-and-find callback succeeds"),
        Value::Array(vec![
            object_reference_value(newer),
            object_reference_value(older),
        ]),
        "moving an unrelated object must not rebuild and reorder this sector"
    );
    assert_eq!(
        HOST_WORLD_OBJECT_MATERIALIZATIONS.with(Cell::get),
        1,
        "only the caller materializes; scalar returned candidates stay in the live engine view"
    );
}

#[test]
fn effect_batch_geometry_preview_preserves_callback_entry_sector_order() {
    // Consecutive effect callbacks share one live C4Object graph. A foreign
    // SetPosition in the first timer performs only that object's UpdatePos;
    // the next timer must retain unrelated physical sector-list order even
    // when it differs from stMain after SortByCategory
    // (oracle-src-pinned src/C4Effect.cpp:330-357;
    // src/C4Sector.cpp:107-147; src/C4GameObjects.cpp:732-736).
    let mut engine = Engine::with_seed(0);
    engine.set_landscape(Landscape::flat(400, 100));
    engine.register_test_script_definition("ORDR", "Ordered candidate", "#strict\n");
    engine.register_test_script_definition("MOVR", "Foreign mover", "#strict\n");
    engine.register_test_script_definition("STAT", "Foreign status target", "#strict\n");
    for id in ["CHG1", "CHG2"] {
        engine.register_test_script_definition(id, id, "#strict\n");
    }
    engine.register_test_script_definition("DEAD", "Foreign removal target", "#strict\n");
    let mut observer = test_definition(
        "FXOR",
        "Effect order observer",
        r#"#strict 3
    local mover, status_target, change_target, removal_target;
    local expected_first, expected_second;

    public func Arm(object moved, object status_object, object changed,
            object removed, object first, object second)
    {
        mover = moved;
        status_target = status_object;
        change_target = changed;
        removal_target = removed;
        expected_first = first;
        expected_second = second;
        AddEffect("Move", this(), 10, 1, this());
        AddEffect("Observe", this(), 20, 1, this());
    }

    func FxMoveTimer()
    {
        SetPosition(310, 10, mover);
        SetObjectStatus(C4OS_INACTIVE, status_target);
        ChangeDef(CHG2, change_target);
        RemoveObject(removal_target);
        return 0;
    }

    func FxObserveTimer()
    {
        var found = FindObjects(
    [C4FO_InRect, 0, 0, 50, 100],
    [C4FO_ID, ORDR]);
        if (GetLength(found) == 2 &&
    found[0] == expected_first && found[1] == expected_second)
    SetR(17);
        else
    SetR(23);
        return 0;
    }
    "#,
    );
    observer.set_c4_callback_convention(true);
    engine.register_test_definition(observer);
    let older =
        engine.spawn_test_object(SpawnConfig::new("ORDR").with_position(Vector2::new(10, 10)));
    let newer =
        engine.spawn_test_object(SpawnConfig::new("ORDR").with_position(Vector2::new(20, 10)));
    let mover =
        engine.spawn_test_object(SpawnConfig::new("MOVR").with_position(Vector2::new(210, 10)));
    let status_target =
        engine.spawn_test_object(SpawnConfig::new("STAT").with_position(Vector2::new(260, 10)));
    let change_target =
        engine.spawn_test_object(SpawnConfig::new("CHG1").with_position(Vector2::new(270, 10)));
    let removal_target =
        engine.spawn_test_object(SpawnConfig::new("DEAD").with_position(Vector2::new(280, 10)));
    let observer =
        engine.spawn_test_object(SpawnConfig::new("FXOR").with_position(Vector2::new(210, 20)));

    let older_index = engine.test_object_index(older);
    let newer_index = engine.test_object_index(newer);
    engine.objects[older_index].state.category = CATEGORY_OBJECT;
    engine.objects[newer_index].state.category = CATEGORY_STRUCTURE;
    engine
        .pending_object_order_commands
        .push(ObjectOrderCommand::SortByCategory);
    engine.execute_object_order_commands();

    let observer_index = engine.test_object_index(observer);
    crate::TestValueExt::test_value(engine.call_object_function(
        observer_index,
        "Arm",
        vec![
            object_reference_value(mover),
            object_reference_value(status_target),
            object_reference_value(change_target),
            object_reference_value(removal_target),
            object_reference_value(newer),
            object_reference_value(older),
        ],
    ));
    let observer_index = engine.test_object_index(observer);
    let move_effect = crate::TestValueExt::test_value(
        engine.objects[observer_index]
            .state
            .effects
            .iter()
            .find(|effect| effect.name == "Move")
            .cloned(),
    );
    let observe_effect = crate::TestValueExt::test_value(
        engine.objects[observer_index]
            .state
            .effects
            .iter()
            .find(|effect| effect.name == "Observe")
            .cloned(),
    );
    let definition_id = engine.objects[observer_index].definition_id.clone();

    crate::TestValueExt::test_value(engine.dispatch_object_effect_events(
        observer_index,
        &definition_id,
        vec![
            EffectEvent::timer(move_effect),
            EffectEvent::timer(observe_effect),
        ],
    ));

    assert_eq!(
        engine.objects[observer_index].state.rotation, 17,
        "the second timer must keep the callback-entry physical sector order"
    );
}

#[test]
fn sector_query_ordering_is_frozen_across_rebuild_and_incremental_paths() {
    // FREEZE. `C4LSectors` keeps its own physical per-sector list order and
    // refreshes only a rank oracle on SortByCategory (oracle-src-pinned
    // src/C4Sector.cpp:107-160), so a map rebuilt from the current object set
    // and a map mutated incrementally can legitimately disagree. FindObject
    // ordering is determinism-critical, so pin the exact sequences the
    // callback-local rebuild produces.
    //
    // The frozen sequences are per-sector NEWEST-FIRST, because
    // `C4LSectors::Add` receives the live forward master list and
    // `C4ObjectList::Add(stMain)` links a new object ahead of the first
    // same-category/same-id entry (C4Sector.cpp:88-101;
    // C4ObjectList.cpp:155-163). Verified against the pinned oracle on
    // EkeReloaded's Invasion: for one shared sector C++ reported
    // `721 722 719 720`, and an earlier ascending-order rebuild here reported
    // `721 719 722 720`.
    let mut engine = Engine::with_seed(0);
    for id in ["SCTA", "SCTB"] {
        engine.register_test_definition(test_definition(id, id, ""));
    }
    engine.set_landscape(Landscape::flat(400, 200));

    // Spread objects across sector boundaries and interleave the two
    // definitions so an ordering change cannot hide behind a stable grouping.
    let mut spawned = Vec::new();
    for index in 0..24 {
        let id = if index % 2 == 0 { "SCTA" } else { "SCTB" };
        let object = engine.spawn_test_object(
            SpawnConfig::new(id).with_position(Vector2::new(9 + index * 15, 20 + index % 5)),
        );
        spawned.push(object);
    }

    // The 15px spacing groups these 24 objects into eight sectors; each
    // sector's own list is newest-first, so every group runs backwards.
    // Entries are spawn indices.
    const SECTOR_GROUPS: [&[usize]; 8] = [
        &[2, 1, 0],
        &[6, 5, 4, 3],
        &[9, 8, 7],
        &[12, 11, 10],
        &[16, 15, 14, 13],
        &[19, 18, 17],
        &[22, 21, 20],
        &[23],
    ];
    let expect = |groups: &[&[usize]]| -> Vec<ObjectId> {
        groups
            .iter()
            .flat_map(|group| group.iter().map(|&index| spawned[index]))
            .collect()
    };

    let context = engine.host_world_context();
    let whole = crate::TestValueExt::test_value(
        context.object_sector_ids_in_rect(DefinitionRect::new(0, 0, 400, 200)),
    );
    assert_eq!(
        whole,
        expect(&SECTOR_GROUPS),
        "a full-extent query returns every object in master-list order"
    );

    // A narrow rect resolves to whole overlapping sectors, so the result is a
    // broad-phase superset of the rect: the last object returned sits at
    // x=249, outside the 100..220 query, and comes back because its sector
    // overlaps. That
    // is the C4LSectors contract, and the callers filter afterwards. Freeze the
    // exact span and its order.
    let window = crate::TestValueExt::test_value(
        context.object_sector_ids_in_rect(DefinitionRect::new(100, 0, 120, 200)),
    );
    assert_eq!(
        window,
        expect(&SECTOR_GROUPS[2..5]),
        "a partial rect preserves master-list order within the covered sectors"
    );

    // Per-sector lists are the shape FindObject consumes; freeze the grouping
    // as well as the flattening, since only the latter is order-insensitive.
    let lists = crate::TestValueExt::test_value(
        context.object_sector_id_lists_in_rect(DefinitionRect::new(0, 0, 400, 200)),
    );
    assert_eq!(
        lists.iter().flatten().copied().collect::<Vec<_>>(),
        expect(&SECTOR_GROUPS),
        "per-sector lists flatten back to master-list order"
    );
    assert!(
        lists.iter().any(|list| list.len() > 1),
        "the fixture must actually populate a shared sector, or it freezes nothing"
    );
}

#[test]
fn read_only_terrain_queries_never_clone_the_landscape() {
    // GBackSolid and friends only read `C4Landscape::GetPix`
    // (C4Wrappers.h:66-92). The lazy host world used to answer them from a
    // deep copy of the whole landscape, which is O(map) work per script call
    // on a path real content walks several times per object per frame.
    // Reads borrow the engine's landscape instead; only a host call that
    // *writes* terrain materializes the private copy.
    let mut engine = Engine::with_seed(0);
    let mut landscape = crate::TestValueExt::test_value(Landscape::with_default_material(
        100,
        vec![100; 100],
        None,
    ));
    landscape.set_world_height(100);
    landscape.set_pixel_grid(PixelGrid::new(
        100,
        100,
        vec![0; 100 * 100],
        vec![0, 100],
        vec![None, Some("Earth".to_owned())],
        vec![None; 2],
    ));
    engine.set_landscape(landscape);
    crate::TestValueExt::test_value(engine.landscape.as_mut()).grid_write_byte(10, 20, 1);

    let mut prober = test_definition(
        "PROB",
        "Terrain prober",
        r#"#strict
    local solid, sky;
    public func Probe()
    {
        solid = GBackSolid(10 - GetX(), 20 - GetY());
        sky = GBackSolid(90 - GetX(), 90 - GetY());
        return(0);
    }
    "#,
    );
    prober.set_c4_callback_convention(true);
    engine.register_test_definition(prober);
    let prober =
        engine.spawn_test_object(SpawnConfig::new("PROB").with_position(Vector2::new(50, 50)));
    let prober_index = engine.test_object_index(prober);

    HOST_WORLD_LANDSCAPE_MATERIALIZATIONS.with(|count| count.set(0));
    crate::TestValueExt::test_value(engine.call_object_function(prober_index, "Probe", Vec::new()));
    assert_eq!(
        HOST_WORLD_LANDSCAPE_MATERIALIZATIONS.with(Cell::get),
        0,
        "read-only GBack* queries must borrow terrain rather than copy it"
    );
    // The borrow must still answer exactly what the copy answered.
    assert_eq!(
        engine.objects[prober_index].state.local_vars.get("solid"),
        Some(&Value::Bool(true)),
        "the borrowed landscape reports the written solid pixel"
    );
    assert_eq!(
        engine.objects[prober_index].state.local_vars.get("sky"),
        Some(&Value::Bool(false)),
        "the borrowed landscape reports untouched sky"
    );
}
